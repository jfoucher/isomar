//! Land relief: hillshade and contour lines from downloaded elevation tiles.
//!
//! Not a layer of its own. The land layer draws it over its fill, in a group
//! with the `Atop` operator, so the relief covers the land and nothing else
//! and follows whichever coast is drawn (SHOM's or OpenStreetMap's); the
//! charts' land layers call it too, clipped to their land. Heights come from
//! `core::chart::dem`; the controller in `dem.rs` downloads the tiles.
//!
//! Heights are read on a lattice anchored to the world, `STEP` points apart
//! (as the depth shading is), so that neighbouring tiles sample the same
//! places and their seams do not show.

use super::credit::Credit;
use super::{DrawCtx, Redraw, TileArea};
use crate::core::chart::dem::{self, ElevTile, Mosaic};
use crate::core::chart::grid::marching_squares;
use crate::core::chart::vtile::{TileCache, TileId, resolve, tiles_for};
use crate::core::geo::{unproject, world_size};
use crate::core::vector::Bounds;
use crate::tokens::Token;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

/// Points between the lattice's samples.
const STEP: f64 = 2.0;
/// Below this zoom the relief is not drawn.
pub const MIN_ZOOM: f64 = 7.0;
/// Elevation tiles kept in memory (256 KB each).
const TILE_CAP: usize = 160;
/// Most tiles one area asks for.
pub(crate) const MAX_TILES: usize = 96;
/// The most opaque the shaded and the lit side of a slope get.
const SHADOW_ALPHA: f32 = 0.5;
const LIGHT_ALPHA: f32 = 0.4;
/// Metres round the equator, for the size of a lattice step on the ground.
const EARTH_M: f64 = 40_075_016.686;
/// The sun's height above the horizon, and where the light comes from (degrees clockwise from north):
/// four directions round the north-west, so no slope is lost to a single light's shadow.
const ALTITUDE_DEG: f32 = 45.0;
const AZIMUTHS_DEG: [f32; 4] = [225.0, 270.0, 315.0, 360.0];

/// The elevation tiles, shared with the controller that downloads them.
pub struct ReliefLayer {
    tiles: RefCell<TileCache<Arc<ElevTile>>>,
    shown: Cell<bool>,
    credit: Rc<Credit>,
    /// The layers that draw the relief, told where it changed (`None`: anywhere) so that
    /// the tiles there are drawn again.
    observers: RefCell<Vec<Box<dyn Fn(Option<Bounds>)>>>,
    redraw: Redraw,
    /// Counts what changes the heights, to know when the last snapshot is still good.
    generation: Cell<u64>,
    /// The last snapshot, for the several layers that ask for the same area in one pass.
    last: RefCell<Option<(([u64; 5], u64), Option<Arc<ReliefPaint>>)>>,
}

impl ReliefLayer {
    pub fn new(redraw: Redraw, credit: Rc<Credit>) -> ReliefLayer {
        ReliefLayer { tiles: RefCell::new(TileCache::new(TILE_CAP)), shown: Cell::new(false), credit, observers: RefCell::new(Vec::new()), redraw, generation: Cell::new(0), last: RefCell::new(None) }
    }

    /// Calls `f` with the area that changed whenever the relief does.
    pub fn follow(&self, f: impl Fn(Option<Bounds>) + 'static) {
        self.observers.borrow_mut().push(Box::new(f));
    }

    fn changed(&self, area: Option<Bounds>) {
        self.generation.set(self.generation.get() + 1);
        for f in self.observers.borrow().iter() {
            f(area);
        }
        (self.redraw)();
    }

    pub fn shown(&self) -> bool {
        self.shown.get()
    }

    pub fn set_shown(&self, on: bool) {
        if self.shown.replace(on) != on {
            self.changed(None);
        }
    }

    pub fn has_tile(&self, id: &TileId) -> bool {
        self.tiles.borrow().contains(id)
    }

    /// Adds a tile, replacing any earlier copy and the coarser tile that stood in for it.
    pub fn add_tile(&self, id: TileId, tile: Arc<ElevTile>) {
        self.tiles.borrow_mut().insert(id, tile);
        // The tile's neighbours read across its edge.
        self.changed(Some(id.bounds().expanded(0.01 * 360.0 / (1u64 << id.z) as f64)));
    }

    /// Snapshots the heights of the tiles round `area`, a coarser tile standing in for one that is
    /// missing, and sets the credit. `None` when there is nothing to draw.
    pub fn prepare(&self, area: &TileArea) -> Option<Arc<ReliefPaint>> {
        let b = area.vi.bounds();
        let key = ([b.min_lon, b.min_lat, b.max_lon, b.max_lat, area.vi.zoom()].map(f64::to_bits), self.generation.get());
        if let Some((k, paint)) = self.last.borrow().as_ref()
            && *k == key
        {
            self.credit.relief.set(paint.is_some());
            return paint.clone();
        }
        let paint = self.make(area);
        *self.last.borrow_mut() = Some((key, paint.clone()));
        paint
    }

    fn make(&self, area: &TileArea) -> Option<Arc<ReliefPaint>> {
        let vi = &area.vi;
        let on = self.shown.get() && vi.zoom() >= MIN_ZOOM;
        if !on {
            self.credit.relief.set(false);
            return None;
        }
        let z = dem::zoom_for(vi.zoom());
        // A ring of one tile round the area, for the seams.
        let ring = 360.0 / (1u64 << z) as f64;
        let wanted = tiles_for(&vi.bounds().expanded(ring), z, MAX_TILES);
        let mut tiles = self.tiles.borrow_mut();
        let mut mosaic = Mosaic::new(z);
        for (src, kids) in resolve(&wanted, |t| tiles.contains(t)) {
            let Some(tile) = tiles.get(&src) else { continue };
            for kid in kids {
                mosaic.insert(kid, if src == kid { tile.clone() } else { Arc::new(tile.stand_in(src, kid)) });
            }
        }
        self.credit.relief.set(!mosaic.is_empty());
        (!mosaic.is_empty()).then(|| Arc::new(ReliefPaint { mosaic, kept: Mutex::new(Vec::new()) }))
    }
}

/// A snapshot of the heights round an area.
pub struct ReliefPaint {
    mosaic: Mosaic,
    /// The tiles worked out last (see `paint`), shared by the tile workers.
    kept: Mutex<Vec<(WorkedKey, Option<Arc<Worked>>)>>,
}

/// The interval between contours at a zoom, in metres; none where they would be a mess.
pub fn contour_interval(zoom: f64) -> Option<f32> {
    Some(match zoom {
        z if z < 9.0 => return None,
        z if z < 10.0 => 200.0,
        z if z < 11.0 => 100.0,
        z if z < 12.0 => 50.0,
        z if z < 13.0 => 20.0,
        z if z < 15.0 => 10.0,
        _ => 5.0,
    })
}

/// How much to exaggerate heights over distances: a big map flattens the land.
fn exaggeration(zoom: f64) -> f32 {
    1.0 + 3.0 * ((13.0 - zoom) / 5.0).clamp(0.0, 1.0) as f32
}

/// How much lighter (positive) or darker (negative) than flat ground a slope looks, between -1 and 1,
/// for the gradients east and north in metres per metre.
fn relief_shade(dzdx: f32, dzdy: f32, exaggeration: f32) -> f32 {
    let (nx, ny, nz) = (-exaggeration * dzdx, -exaggeration * dzdy, 1.0);
    let norm = (nx * nx + ny * ny + nz * nz).sqrt();
    let alt = ALTITUDE_DEG.to_radians();
    let lit: f32 = AZIMUTHS_DEG
        .iter()
        .map(|az| {
            let az = az.to_radians();
            (nx * az.sin() * alt.cos() + ny * az.cos() * alt.cos() + nz * alt.sin()) / norm
        })
        .sum::<f32>()
        / AZIMUTHS_DEG.len() as f32;
    ((lit - alt.sin()) / 0.4).clamp(-1.0, 1.0)
}

/// The lattice smoothed by a 3 by 3 mean, which hides the whole metres the sources are rounded to.
/// A sample with no data stays so; one missing neighbour is left out of the mean.
fn smooth(h: &[f32], nx: usize, ny: usize) -> Vec<f32> {
    let mut out = h.to_vec();
    for y in 0..ny {
        for x in 0..nx {
            if h[y * nx + x].is_nan() {
                continue;
            }
            let (mut sum, mut n) = (0.0, 0.0);
            for yy in y.saturating_sub(1)..=(y + 1).min(ny - 1) {
                for xx in x.saturating_sub(1)..=(x + 1).min(nx - 1) {
                    let v = h[yy * nx + xx];
                    if !v.is_nan() {
                        sum += v;
                        n += 1.0;
                    }
                }
            }
            out[y * nx + x] = sum / n;
        }
    }
    out
}

/// The hillshade of a lattice of heights as premultiplied ARGB pixels, one per sample (the edge ring
/// stays clear). `cell_m[y]` is the ground size of a step along row `y`. Where the height is 0 or
/// less (the sea, or no data) nothing is drawn.
fn shade_pixels(h: &[f32], nx: usize, ny: usize, cell_m: &[f64], exaggeration: f32, shadow: [f32; 3], light: [f32; 3]) -> Vec<u32> {
    let mut px = vec![0u32; nx * ny];
    // The sea is level for the slopes: a coast is not a cliff of the model's depth.
    let g = |x: usize, y: usize| h[y * nx + x].max(0.0);
    for y in 1..ny.saturating_sub(1) {
        let cs = cell_m[y] as f32;
        for x in 1..nx.saturating_sub(1) {
            let e = h[y * nx + x];
            if e.is_nan() || e <= 0.0 {
                continue;
            }
            if [(-1i32, -1i32), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)].iter().any(|(dx, dy)| h[(y as i32 + dy) as usize * nx + (x as i32 + dx) as usize].is_nan()) {
                continue;
            }
            let (a, b, c) = (g(x - 1, y - 1), g(x, y - 1), g(x + 1, y - 1));
            let (d, f) = (g(x - 1, y), g(x + 1, y));
            let (gg, hh, i) = (g(x - 1, y + 1), g(x, y + 1), g(x + 1, y + 1));
            let dzdx = ((c + 2.0 * f + i) - (a + 2.0 * d + gg)) / (8.0 * cs);
            // Rows run south, so north is the other way.
            let dzdy = ((a + 2.0 * b + c) - (gg + 2.0 * hh + i)) / (8.0 * cs);
            let s = relief_shade(dzdx, dzdy, exaggeration);
            let (colour, alpha) = if s < 0.0 { (shadow, -s * SHADOW_ALPHA) } else { (light, s * LIGHT_ALPHA) };
            let byte = |v: f32| (v * alpha * 255.0 + 0.5) as u32;
            px[y * nx + x] = ((alpha * 255.0 + 0.5) as u32) << 24 | byte(colour[0]) << 16 | byte(colour[1]) << 8 | byte(colour[2]);
        }
    }
    px
}

/// The contour levels (above the sea) a lattice's heights span.
fn levels(h: &[f32], interval: f32) -> Vec<f32> {
    let (lo, hi) = h.iter().filter(|v| !v.is_nan()).fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
    if lo > hi {
        return Vec::new();
    }
    let first = (lo.max(0.0) / interval).floor() as i64 + 1;
    (first..).map(|k| k as f32 * interval).take_while(|l| *l <= hi).collect()
}

impl ReliefPaint {
    /// Draws the relief of the tile `ctx` shows: shading, and lines when `contours` and the zoom suit.
    /// Whatever operator the context has set decides where it lands: `Atop` keeps it on land.
    ///
    /// The land, the towns and every chart cell of a tile paint it, each under its own clip, so what it
    /// is made of is worked out once per tile and kept (`Worked`): at Boston, where NOAA cells overlap
    /// several deep, working it out for each cell was more than half of drawing a tile.
    pub fn paint(&self, ctx: &DrawCtx, contours: bool) {
        let Some(w) = self.worked(ctx, contours) else { return };
        let Some(img) = &w.image else { return };
        let cr = ctx.cr;
        // The picture is made at the target's own resolution, so it is laid on pixel for pixel.
        let Ok(mut surface) = cairo::ImageSurface::create(cairo::Format::ARgb32, img.width, img.height) else { return };
        {
            let stride = surface.stride() as usize;
            let row = img.width as usize * 4;
            let Ok(mut data) = surface.data() else { return };
            for y in 0..img.height as usize {
                data[y * stride..y * stride + row].copy_from_slice(&img.pixels[y * row..(y + 1) * row]);
            }
        }
        surface.set_device_scale(img.scale, img.scale);
        let _ = cr.save();
        if cr.set_source_surface(&surface, 0.0, 0.0).is_ok() {
            let _ = cr.paint();
        }
        let _ = cr.restore();
    }

    /// The shading and lines of the tile `ctx` shows, from the kept ones when another layer of the same tile
    /// asked first.
    fn worked(&self, ctx: &DrawCtx, contours: bool) -> Option<Arc<Worked>> {
        let (vi, pal) = (&ctx.vi, &ctx.pal);
        let (cx, cy) = vi.view.center_px();
        let colours = [pal.rgba(Token::ChartReliefShadow), pal.rgba(Token::ChartReliefLight)].map(|(r, g, b, _)| [r, g, b].map(|v| (v as f32).to_bits()));
        let scale = ctx.cr.target().device_scale().0;
        let key = WorkedKey { at: [cx, cy, vi.w, vi.h, vi.zoom(), scale].map(f64::to_bits), colours, contours };
        let found = {
            let kept = self.kept.lock().unwrap_or_else(|e| e.into_inner());
            kept.iter().find(|(k, _)| *k == key).map(|(_, w)| w.clone())
        };
        if let Some(w) = found {
            return w;
        }
        let w = self.work_out(ctx, contours).map(Arc::new);
        let mut kept = self.kept.lock().unwrap_or_else(|e| e.into_inner());
        if kept.len() >= KEPT_TILES {
            kept.remove(0);
        }
        kept.push((key, w.clone()));
        w
    }

    fn work_out(&self, ctx: &DrawCtx, contours: bool) -> Option<Worked> {
        let (vi, pal) = (&ctx.vi, &ctx.pal);
        let zoom = vi.zoom();
        let size = world_size(zoom);
        let (cx, cy) = vi.view.center_px();
        let (left, top) = (cx - vi.w / 2.0, cy - vi.h / 2.0);
        // A ring of samples round the tile, which only its neighbours' slopes and lines need.
        let (j0, i0) = ((left / STEP).floor() as i64 - 1, (top / STEP).floor() as i64 - 1);
        let (nx, ny) = (((left + vi.w) / STEP).floor() as i64 + 2 - j0 + 1, ((top + vi.h) / STEP).floor() as i64 + 2 - i0 + 1);
        if nx < 3 || ny < 3 {
            return None;
        }
        let (nx, ny) = (nx as usize, ny as usize);
        let lats: Vec<f64> = (0..ny).map(|y| unproject(0.0, (i0 + y as i64) as f64 * STEP, zoom).lat).collect();
        let lons: Vec<f64> = (0..nx).map(|x| (j0 + x as i64) as f64 * STEP / size * 360.0 - 180.0).collect();
        let heights = self.mosaic.heights(&lats, &lons);
        if heights.iter().all(|h| h.is_nan() || *h <= 0.0) {
            return None;
        }
        let heights = smooth(&heights, nx, ny);

        // Hillshade.
        let cell_m: Vec<f64> = lats.iter().map(|lat| STEP * EARTH_M * lat.to_radians().cos() / size).collect();
        let rgb = |t: Token| {
            let (r, g, b, _) = pal.rgba(t);
            [r as f32, g as f32, b as f32]
        };
        let px = shade_pixels(&heights, nx, ny, &cell_m, exaggeration(zoom), rgb(Token::ChartReliefShadow), rgb(Token::ChartReliefLight));
        let shade = px.iter().any(|p| *p != 0).then_some(px);

        // Contour lines.
        let (mut thin, mut index) = (Vec::new(), Vec::new());
        if let Some(interval) = contours.then(|| contour_interval(zoom)).flatten() {
            // The sea is level here too: a contour above it, not along the model's cliff to the seabed.
            let land: Vec<f32> = heights.iter().map(|h| h.max(0.0)).collect();
            let place = |c: f64, r: f64| [(j0 as f64 + c) * STEP - left, (i0 as f64 + r) * STEP - top];
            for level in levels(&land, interval) {
                let segs = marching_squares(nx, ny, |c, r| land[r * nx + c], level, place);
                let lines = if (level / interval).round() as i64 % 5 == 0 { &mut index } else { &mut thin };
                lines.extend(segs);
            }
        }
        let origin = [(j0 as f64 - 0.5) * STEP - left, (i0 as f64 - 0.5) * STEP - top];
        Some(Worked { image: picture(ctx, nx, ny, origin, shade.as_deref(), &thin, &index) })
    }
}

/// How many tiles' relief a snapshot keeps: a few per tile worker drawing at once.
const KEPT_TILES: usize = 24;

/// What a tile's relief is drawn from, kept for the other layers of the same tile.
#[derive(Clone, Copy, PartialEq)]
struct WorkedKey {
    /// The tile's centre in world pixels, its size, its zoom and its target's scale (bits, so equal only when
    /// the same tile).
    at: [u64; 6],
    /// The palette's shadow and light, which the shading is made in.
    colours: [[u32; 3]; 2],
    contours: bool,
}

struct Worked {
    /// The shading with the lines over it, the size of the tile; `None` where there is nothing to draw.
    image: Option<Picture>,
}

/// A tile's relief as pixels (premultiplied ARGB, rows without padding), made once and painted under each
/// caller's clip: one clipped paint costs far less than stroking every contour under a cell's coast again.
struct Picture {
    width: i32,
    height: i32,
    scale: f64,
    pixels: Vec<u8>,
}

/// Draws the shading (`nx` by `ny` samples from `origin`) and the lines into a picture of the tile `ctx`
/// shows, at its target's resolution. Painting it gives what drawing them in turn gives, under `Over` and
/// under `Atop` alike.
fn picture(ctx: &DrawCtx, nx: usize, ny: usize, origin: [f64; 2], shade: Option<&[u32]>, thin: &[[f32; 4]], index: &[[f32; 4]]) -> Option<Picture> {
    if shade.is_none() && thin.is_empty() && index.is_empty() {
        return None;
    }
    let (vi, pal) = (&ctx.vi, &ctx.pal);
    let scale = ctx.cr.target().device_scale().0.max(1.0);
    let (width, height) = ((vi.w * scale).ceil() as i32, (vi.h * scale).ceil() as i32);
    let target = cairo::ImageSurface::create(cairo::Format::ARgb32, width, height).ok()?;
    target.set_device_scale(scale, scale);
    {
        let cr = cairo::Context::new(&target).ok()?;
        if let Some(px) = shade
            && let Ok(mut surface) = cairo::ImageSurface::create(cairo::Format::ARgb32, nx as i32, ny as i32)
        {
            {
                let stride = surface.stride() as usize;
                if let Ok(mut data) = surface.data() {
                    for y in 0..ny {
                        for x in 0..nx {
                            let o = y * stride + x * 4;
                            data[o..o + 4].copy_from_slice(&px[y * nx + x].to_ne_bytes());
                        }
                    }
                }
            }
            let _ = cr.save();
            // A sample is at the centre of its pixel.
            cr.translate(origin[0], origin[1]);
            cr.scale(STEP, STEP);
            if cr.set_source_surface(&surface, 0.0, 0.0).is_ok() {
                cr.source().set_filter(cairo::Filter::Bilinear);
                let _ = cr.paint();
            }
            let _ = cr.restore();
        }
        cr.set_line_cap(cairo::LineCap::Round);
        for (segs, token, width) in [(thin, Token::ChartLandContour, 0.6), (index, Token::ChartLandContourIndex, 1.1)] {
            if segs.is_empty() {
                continue;
            }
            cr.new_path();
            for s in segs.iter() {
                cr.move_to(s[0] as f64, s[1] as f64);
                cr.line_to(s[2] as f64, s[3] as f64);
            }
            pal.set(&cr, token);
            cr.set_line_width(width);
            let _ = cr.stroke();
        }
    }
    target.flush();
    let stride = target.stride() as usize;
    let row = width as usize * 4;
    let mut target = target;
    let data = target.data().ok()?;
    let mut pixels = Vec::with_capacity(row * height as usize);
    for y in 0..height as usize {
        pixels.extend_from_slice(&data[y * stride..y * stride + row]);
    }
    Some(Picture { width, height, scale, pixels })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_ground_is_neither_lit_nor_shaded() {
        assert!(relief_shade(0.0, 0.0, 1.0).abs() < 1e-5);
    }

    #[test]
    fn slopes_facing_the_north_west_are_lit_and_those_facing_away_are_dark() {
        // Rising to the east means facing west; rising to the south, facing north.
        assert!(relief_shade(0.5, 0.0, 1.0) > 0.0, "facing west");
        assert!(relief_shade(0.0, -0.5, 1.0) > 0.0, "facing north");
        assert!(relief_shade(-0.5, 0.0, 1.0) < 0.0, "facing east");
        assert!(relief_shade(0.0, 0.5, 1.0) < 0.0, "facing south");
        // Steeper is stronger, and exaggeration steepens.
        assert!(relief_shade(-1.0, 0.0, 1.0) < relief_shade(-0.2, 0.0, 1.0));
        assert!(relief_shade(-0.2, 0.0, 3.0) < relief_shade(-0.2, 0.0, 1.0));
        assert!((-1.0..=1.0).contains(&relief_shade(-100.0, 100.0, 4.0)));
    }

    #[test]
    fn contours_are_wider_apart_as_the_map_shrinks() {
        assert_eq!(contour_interval(8.0), None);
        let steps: Vec<f32> = [9.5, 10.5, 11.5, 12.5, 13.5, 16.0].iter().map(|z| contour_interval(*z).unwrap()).collect();
        assert_eq!(steps, [200.0, 100.0, 50.0, 20.0, 10.0, 5.0]);
        assert!(exaggeration(8.0) > exaggeration(10.0) && exaggeration(10.0) > exaggeration(13.0));
        assert_eq!(exaggeration(15.0), 1.0);
    }

    #[test]
    fn levels_start_above_the_sea() {
        assert_eq!(levels(&[-50.0, 0.0, 35.0], 10.0), [10.0, 20.0, 30.0]);
        assert_eq!(levels(&[12.0, f32::NAN, 41.0], 10.0), [20.0, 30.0, 40.0]);
        assert!(levels(&[-5.0, -1.0], 10.0).is_empty());
        assert!(levels(&[f32::NAN], 10.0).is_empty());
    }

    #[test]
    fn a_hill_is_lit_on_one_side_and_dark_on_the_other_and_the_sea_is_clear() {
        // A cone of 200 m on a 41 by 41 lattice with 20 m steps, 20 samples of sea along the west edge.
        let n = 41;
        let h: Vec<f32> = (0..n * n)
            .map(|i| {
                let (x, y) = ((i % n) as f32 - 20.0, (i / n) as f32 - 20.0);
                if i % n < 3 { -30.0 } else { (200.0 - 10.0 * (x * x + y * y).sqrt()).max(1.0) }
            })
            .collect();
        let px = shade_pixels(&h, n, n, &vec![20.0; n], 1.0, [0.0; 3], [1.0; 3]);
        let alpha = |x: usize, y: usize| px[y * n + x] >> 24;
        // The north-west flank (facing north-west) is lit, the south-east one is shaded: the lit one is white, the shaded black.
        let (nw, se) = (px[15 * n + 15], px[25 * n + 25]);
        assert!(nw >> 24 > 0 && (nw & 0xff) > 0, "lit side is white: {nw:08x}");
        assert!(se >> 24 > 0 && (se & 0xff) == 0, "shaded side is black: {se:08x}");
        // The sea is clear, and so is the ring at the edge.
        assert_eq!(alpha(1, 20), 0);
        assert_eq!(alpha(0, 0), 0);
        assert_eq!(alpha(n - 1, n - 1), 0);
    }

    #[test]
    fn smoothing_flattens_steps_and_keeps_no_data() {
        let h = [0.0, 0.0, 0.0, 0.0, 9.0, 0.0, 0.0, f32::NAN, 0.0];
        let s = smooth(&h, 3, 3);
        // Nine cells less the one with no data.
        assert!((s[4] - 9.0 / 8.0).abs() < 1e-5, "{}", s[4]);
        assert!(s[7].is_nan());
    }
}
