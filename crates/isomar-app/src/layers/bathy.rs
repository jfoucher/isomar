//! Depth shading and contours from bathymetry grids.

use super::credit::Credit;
use super::towns::{TownLayer, TownPaint};
use super::enc::{SOUNDING_INK, THIN_MAX_RADIUS, draw_sounding_placed_at, sounding_cell, thin_soundings};
use super::{Changes, DrawCtx, LabelPlacer, Layer, Redraw, TileArea, TilePaint, names};
use crate::core::chart::emodnet::TileKey;
use crate::core::chart::grid::{ContourSet, Grid};
use crate::core::vector::Bounds;
use crate::palette::rgb;
use crate::tokens::Token;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

/// Contour levels in metres (negative is below the datum).
pub const CONTOUR_LEVELS: [f32; 8] = [-2.0, -5.0, -10.0, -20.0, -50.0, -100.0, -200.0, -500.0];

/// Pixels per shaded sample; higher is faster and blurrier.
const SHADE_STEP: f64 = 3.0;

pub struct DepthGrid {
    /// Shared so the automatic router can take it to a worker thread.
    pub grid: Arc<Grid>,
    pub contours: Vec<ContourSet>,
    /// Set for a downloaded EMODnet tile, which the layer can hide and drop.
    pub tile: Option<TileKey>,
}

impl DepthGrid {
    pub fn new(grid: Grid) -> DepthGrid {
        let contours = CONTOUR_LEVELS.iter().map(|l| ContourSet::build(&grid, *l)).collect();
        DepthGrid { grid: Arc::new(grid), contours, tile: None }
    }

    pub fn for_tile(grid: Grid, key: TileKey) -> DepthGrid {
        DepthGrid { tile: Some(key), ..DepthGrid::new(grid) }
    }

    /// The grids of `grids` after this one (finer, as they are kept coarsest
    /// first) that are strictly finer and reach into `area`: where they have
    /// data they are drawn instead of this one.
    fn finer_bounds(&self, later: &[Arc<DepthGrid>], area: &Bounds) -> Vec<Bounds> {
        later.iter().filter(|f| f.grid.dx < self.grid.dx).map(|f| f.grid.bounds()).filter(|b| b.intersects(area)).collect()
    }
}

/// The depth tint of an elevation in metres, as a paper chart has it: drying ground, then water down
/// to 5 m, down to the safety depth, and deeper.
pub fn depth_color(elev: f32, safety: f32) -> Token {
    if elev > 0.0 {
        return Token::ChartDrying;
    }
    let d = -elev;
    if d < 5.0 {
        Token::ChartShallow
    } else if d < safety {
        Token::ChartMedium
    } else {
        Token::ChartDeep
    }
}

/// Smallest zoom at which a contour is worth drawing.
pub fn contour_min_zoom(level: f32) -> f64 {
    match level.abs() {
        l if l <= 5.0 => 10.0,
        l if l <= 20.0 => 9.0,
        _ => 7.5,
    }
}

pub struct DepthLayer {
    /// Coarsest first, so finer grids paint over them.
    grids: RefCell<Vec<Arc<DepthGrid>>>,
    safety: Cell<f32>,
    /// Whether downloaded tiles are shown.
    tiles_shown: Cell<bool>,
    credit: RefCell<Option<Rc<Credit>>>,
    changes: Changes,
    redraw: Redraw,
}

impl DepthLayer {
    pub fn new(redraw: Redraw) -> DepthLayer {
        DepthLayer { grids: RefCell::new(Vec::new()), safety: Cell::new(-5.0), tiles_shown: Cell::new(true), credit: RefCell::new(None), changes: Changes::default(), redraw }
    }

    /// Has the layer credit downloaded data while it draws it.
    pub fn set_credit(&self, credit: Rc<Credit>) {
        *self.credit.borrow_mut() = Some(credit);
    }

    fn bump(&self) {
        self.bump_in(None);
    }

    /// A change within `area`, or anywhere. Soundings near a grid's edge are thinned against those
    /// just past it, so the area takes in a margin.
    fn bump_in(&self, area: Option<Bounds>) {
        self.changes.bump(area.map(|b| b.expanded(0.1 * b.width().max(b.height()) + 0.02)));
        (self.redraw)();
    }

    /// The grids in use that reach into `view`, coarsest first.
    fn live(&self, view: &Bounds) -> Vec<Arc<DepthGrid>> {
        let shown = self.tiles_shown.get();
        self.grids.borrow().iter().filter(|g| (shown || g.tile.is_none()) && g.grid.bounds().intersects(view)).cloned().collect()
    }

    pub fn set_tiles_shown(&self, shown: bool) {
        if self.tiles_shown.replace(shown) != shown {
            self.bump();
        }
    }

    pub fn has_tile(&self, key: &TileKey) -> bool {
        self.grids.borrow().iter().any(|g| g.tile.as_ref() == Some(key))
    }

    /// Number of downloaded tiles held.
    pub fn tile_count(&self) -> usize {
        self.grids.borrow().iter().filter(|g| g.tile.is_some()).count()
    }

    /// Forgets the downloaded tiles that do not reach into `keep`, returning
    /// their keys; they are read again from disk if the view comes back.
    pub fn drop_tiles_outside(&self, keep: &Bounds) -> Vec<TileKey> {
        let mut dropped = Vec::new();
        let mut area: Option<Bounds> = None;
        self.grids.borrow_mut().retain(|g| match g.tile {
            Some(k) if !g.grid.bounds().intersects(keep) => {
                dropped.push(k);
                let b = g.grid.bounds();
                area.get_or_insert(b).union(&b);
                false
            }
            _ => true,
        });
        if !dropped.is_empty() {
            self.bump_in(area);
        }
        dropped
    }

    pub fn add_grid(&self, g: DepthGrid) {
        let area = g.grid.bounds();
        let mut grids = self.grids.borrow_mut();
        grids.push(Arc::new(g));
        grids.sort_by(|a, b| b.grid.dx.partial_cmp(&a.grid.dx).unwrap_or(std::cmp::Ordering::Equal));
        drop(grids);
        self.bump_in(Some(area));
    }

    /// Depth in metres (positive) at a position, from the finest grid that has data.
    pub fn depth_at(&self, lon: f64, lat: f64) -> Option<f32> {
        let shown = self.tiles_shown.get();
        self.grids.borrow().iter().rev().filter(|g| shown || g.tile.is_none()).find_map(|g| g.grid.sample(lon, lat)).map(|e| -e)
    }

    /// The grids in memory that reach into `area`, for work off the main thread.
    pub fn grids_in(&self, area: &Bounds) -> Vec<Arc<Grid>> {
        self.grids.borrow().iter().filter(|g| g.grid.bounds().intersects(area)).map(|g| g.grid.clone()).collect()
    }

    pub fn set_safety_depth(&self, metres: f32) {
        self.safety.set(-metres.abs());
        self.bump();
    }
}

impl Layer for DepthLayer {
    fn id(&self) -> &'static str {
        "depth"
    }

    fn revision(&self) -> u64 {
        self.changes.get()
    }

    fn changes_since(&self, revision: u64) -> Option<Vec<super::Change>> {
        self.changes.since(revision)
    }

    fn prepare(&self, area: &TileArea) -> Option<Arc<dyn TilePaint>> {
        let grids = self.live(&area.vi.bounds());
        let tiles = grids.iter().any(|g| g.tile.is_some());
        if let Some(credit) = self.credit.borrow().as_ref() {
            credit.depth.set(tiles);
        }
        (!grids.is_empty()).then(|| Arc::new(DepthPaint { grids, safety: self.safety.get() }) as Arc<dyn TilePaint>)
    }
}

/// The depth grids as they were when the tiles were prepared.
struct DepthPaint {
    grids: Vec<Arc<DepthGrid>>,
    safety: f32,
}

impl DepthPaint {
    /// Colours the grids' samples on a lattice anchored to the world, `SHADE_STEP` pixels apart, so that
    /// neighbouring tiles sample the same places, and paints it smoothed.
    fn shade(&self, ctx: &DrawCtx, grids: &[Arc<DepthGrid>]) {
        let (cr, vi, pal) = (ctx.cr, &ctx.vi, &ctx.pal);
        let size = crate::core::geo::world_size(vi.zoom());
        let (cx, cy) = vi.view.center_px();
        let (left, top) = (cx - vi.w / 2.0, cy - vi.h / 2.0);
        let (j0, i0) = ((left / SHADE_STEP).floor() as i64, (top / SHADE_STEP).floor() as i64);
        let (sw, sh) = (((left + vi.w) / SHADE_STEP).floor() as i64 - j0 + 1, ((top + vi.h) / SHADE_STEP).floor() as i64 - i0 + 1);
        if sw <= 0 || sh <= 0 {
            return;
        }
        let (sw, sh) = (sw as i32, sh as i32);
        let Ok(mut surface) = cairo::ImageSurface::create(cairo::Format::ARgb32, sw, sh) else { return };
        let mut painted = false;
        {
            let stride = surface.stride() as usize;
            let Ok(mut data) = surface.data() else { return };
            data.fill(0);
            // Latitude depends only on the row, longitude only on the column.
            let lats: Vec<f64> = (0..sh).map(|y| crate::core::geo::unproject(0.0, (i0 + y as i64) as f64 * SHADE_STEP + SHADE_STEP / 2.0, vi.zoom()).lat).collect();
            let lons: Vec<f64> = (0..sw).map(|x| ((j0 + x as i64) as f64 * SHADE_STEP + SHADE_STEP / 2.0) / size * 360.0 - 180.0).collect();
            for dg in grids {
                let b = dg.grid.bounds();
                let x0 = lons.partition_point(|l| *l < b.min_lon);
                let x1 = lons.partition_point(|l| *l <= b.max_lon);
                for (y, lat) in lats.iter().enumerate() {
                    if *lat < b.min_lat || *lat > b.max_lat {
                        continue;
                    }
                    for (x, lon) in lons.iter().enumerate().take(x1).skip(x0) {
                        if let Some(e) = dg.grid.sample(*lon, *lat) {
                            let (r, g, bl, _) = pal.rgba(depth_color(e, self.safety));
                            let px = (255u32 << 24)
                                | ((r * 255.0) as u32) << 16
                                | ((g * 255.0) as u32) << 8
                                | (bl * 255.0) as u32;
                            let o = y * stride + x * 4;
                            data[o..o + 4].copy_from_slice(&px.to_ne_bytes());
                            painted = true;
                        }
                    }
                }
            }
        }
        if !painted {
            return;
        }
        let _ = cr.save();
        cr.translate(j0 as f64 * SHADE_STEP - left, i0 as f64 * SHADE_STEP - top);
        cr.scale(SHADE_STEP, SHADE_STEP);
        if cr.set_source_surface(&surface, 0.0, 0.0).is_ok() {
            cr.source().set_filter(cairo::Filter::Bilinear);
            let _ = cr.paint();
        }
        let _ = cr.restore();
    }

    fn contours(&self, ctx: &DrawCtx, grids: &[Arc<DepthGrid>], view: &Bounds) {
        let (cr, vi, pal) = (ctx.cr, &ctx.vi, &ctx.pal);
        let safety = self.safety;
        let mut budget = 200_000usize;
        // Lines simplified to half a pixel look the same; latitude is the
        // shorter way in degrees, by the cosine of it.
        let tol = 0.5 * vi.deg_per_px() * vi.view.center.lat.to_radians().cos();
        for (i, dg) in grids.iter().enumerate() {
            // Where a finer grid reaches, only it draws contours.
            let finer = dg.finer_bounds(&grids[i + 1..], view);
            let _ = cr.save();
            if !finer.is_empty() {
                cr.new_path();
                cr.rectangle(-1.0, -1.0, vi.w + 2.0, vi.h + 2.0);
                for b in &finer {
                    let (x1, y1) = vi.to_screen(&crate::core::geo::LatLon::new(b.max_lat, b.min_lon));
                    let (x2, y2) = vi.to_screen(&crate::core::geo::LatLon::new(b.min_lat, b.max_lon));
                    cr.rectangle(x1, y1, x2 - x1, y2 - y1);
                }
                cr.set_fill_rule(cairo::FillRule::EvenOdd);
                cr.clip();
            }
            for set in &dg.contours {
                if vi.zoom() < contour_min_zoom(set.level) {
                    continue;
                }
                let is_safety = (set.level - safety).abs() < 1e-3;
                pal.set_alpha(cr, if is_safety { rgb(30, 70, 140) } else { rgb(90, 130, 175) }, if is_safety { 1.0 } else { 0.8 });
                cr.set_line_width(if is_safety { 1.6 } else { 0.8 });
                for piece in set.pieces_in(view, tol) {
                    if budget < piece.points.len() {
                        break;
                    }
                    budget -= piece.points.len();
                    for (i, p) in piece.points.iter().enumerate() {
                        let (x, y) = vi.to_screen(&crate::core::geo::LatLon::new(p[1] as f64, p[0] as f64));
                        if i == 0 {
                            cr.move_to(x, y);
                        } else {
                            cr.line_to(x, y);
                        }
                    }
                }
                let _ = cr.stroke();
            }
            let _ = cr.restore();
        }
    }
}

impl TilePaint for DepthPaint {
    fn paint(&self, ctx: &DrawCtx) {
        let view = ctx.vi.bounds();
        let grids: Vec<Arc<DepthGrid>> = self.grids.iter().filter(|g| g.grid.bounds().intersects(&view)).cloned().collect();
        if grids.is_empty() {
            return;
        }
        self.shade(ctx, &grids);
        self.contours(ctx, &grids, &view);
    }
}

/// Spot depths from the finest bathymetry grid, printed like the soundings of
/// a chart. They are picked on a lattice sized to the text, shallowest in
/// each cell, so the numbers neither overlap nor crowd a far view. Drawn just
/// above the depth shading and below land, so land hides the numbers the grid
/// puts ashore, and below S-57 and raster charts, which print soundings of
/// their own.
pub struct SoundingLayer {
    depth: Rc<DepthLayer>,
    /// The place names, which soundings keep clear of.
    names: Option<Rc<TownLayer>>,
}

/// Smallest zoom at which grid soundings appear.
const SOUNDINGS_MIN_ZOOM: f64 = 10.0;

/// Shallower than this the grid is more likely shore than water.
const SOUNDINGS_MIN_DEPTH: f32 = 1.0;

impl SoundingLayer {
    pub fn new(depth: Rc<DepthLayer>) -> SoundingLayer {
        SoundingLayer { depth, names: None }
    }

    /// Keeps the soundings clear of the names `towns` writes.
    pub fn with_names(self, towns: Rc<TownLayer>) -> SoundingLayer {
        SoundingLayer { names: Some(towns), ..self }
    }
}

/// The two revisions a sounding tile depends on, in one: the grids' above, the names' below.
const NAMES_BITS: u32 = 32;

impl SoundingLayer {
    fn names_revision(&self) -> u64 {
        self.names.as_ref().map_or(0, |t| t.revision())
    }
}

impl Layer for SoundingLayer {
    fn id(&self) -> &'static str {
        "soundings"
    }

    fn revision(&self) -> u64 {
        self.depth.changes.get() << NAMES_BITS | (self.names_revision() & ((1 << NAMES_BITS) - 1))
    }

    fn changes_since(&self, revision: u64) -> Option<Vec<super::Change>> {
        let mut changes = self.depth.changes.since(revision >> NAMES_BITS)?;
        if let Some(towns) = &self.names
            && towns.revision() != revision & ((1 << NAMES_BITS) - 1)
        {
            changes.extend(towns.changes_since(revision & ((1 << NAMES_BITS) - 1))?);
        }
        Some(changes)
    }

    fn prepare(&self, area: &TileArea) -> Option<Arc<dyn TilePaint>> {
        let vi = &area.vi;
        if vi.zoom() < SOUNDINGS_MIN_ZOOM {
            return None;
        }
        let grids = self.depth.live(&vi.bounds());
        if grids.is_empty() {
            return None;
        }
        // A lattice cell holds one number. Its size is the same for every tile, and for every area at
        // the zoom up to the next whole degree of latitude, so that numbers do not move as tiles are made;
        // and it is the size of the whole zoom below (`SoundingPaint`).
        let (cell_w, cell_h) = sounding_cell();
        let k = names::view_at(vi, names::mark_zoom(vi.zoom())).1;
        let deg_x = vi.deg_per_px() * k * cell_w;
        let deg_y = vi.deg_per_px() * k * vi.view.center.lat.round().to_radians().cos() * cell_h;
        let names = self.names.as_ref().and_then(|t| t.paint_for(area));
        Some(Arc::new(SoundingPaint { grids, deg_x, deg_y, names }))
    }
}

struct SoundingPaint {
    grids: Vec<Arc<DepthGrid>>,
    /// The lattice cell, in degrees.
    deg_x: f64,
    deg_y: f64,
    names: Option<Arc<TownPaint>>,
}

impl TilePaint for SoundingPaint {
    fn paint(&self, ctx: &DrawCtx) {
        let (cr, vi, pal) = (ctx.cr, &ctx.vi, &ctx.pal);
        let (cell_w, cell_h) = sounding_cell();
        // The soundings are picked, thinned and kept clear of the names at the whole zoom below the tile's,
        // `k` times smaller, the same ones in the same places at every zoom up to the next, spreading out
        // as the chart grows; as marks and lights are (`names::mark_zoom`). Picked at the tile's own zoom,
        // a zoom drawn again at each pause changed every number on the chart.
        let k = names::view_at(vi, names::mark_zoom(vi.zoom())).1;
        // Thinning keeps a sounding only if no kept one nearby silences it, so the soundings up to the
        // widest radius past the tile's edge are weighed too, and neighbouring tiles agree on which stay.
        let reach = (cell_w.max(cell_h) * THIN_MAX_RADIUS).ceil();
        let view = vi.bounds().expanded(reach * k * vi.deg_per_px());
        let grids: Vec<Arc<DepthGrid>> = self.grids.iter().filter(|g| g.grid.bounds().intersects(&view)).cloned().collect();
        let mut spots = Vec::new();
        for (i, dg) in grids.iter().enumerate() {
            // Where a finer grid reaches, its soundings are used instead.
            let finer = dg.finer_bounds(&grids[i + 1..], &view);
            spots.extend(dg.grid.spot_depths(&view, self.deg_x, self.deg_y, SOUNDINGS_MIN_DEPTH).into_iter().filter(|s| !finer.iter().any(|b| b.contains([s.lon, s.lat]))));
        }
        let pts = spots
            .iter()
            .map(|s| {
                let (x, y) = vi.to_screen(&crate::core::geo::LatLon::new(s.lat, s.lon));
                (s.depth as f64, x / k, y / k)
            })
            .collect();
        // No sounding is written over a name.
        let boxes = self.names.as_ref().map(|n| n.name_boxes(cr, vi, pal)).unwrap_or_default();
        let mut placer = LabelPlacer::with(boxes.into_iter().map(|b| b.map(|v| v / k)));
        cr.select_font_face(crate::layers::FONT_FAMILY, cairo::FontSlant::Normal, cairo::FontWeight::Normal);
        pal.set(cr, SOUNDING_INK);
        for (d, x, y) in thin_soundings(pts, (cell_w, cell_h)) {
            if (-40.0..vi.w + 40.0).contains(&(x * k)) && (-40.0..vi.h + 40.0).contains(&(y * k)) {
                draw_sounding_placed_at(cr, &mut placer, x, y, d, 10.0, k);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_is_tinted_in_bands() {
        assert_eq!(depth_color(1.5, 5.0), Token::ChartDrying);
        assert_eq!(depth_color(-2.0, 5.0), Token::ChartShallow);
        assert_eq!(depth_color(-4.99, 10.0), Token::ChartShallow);
        assert_eq!(depth_color(-7.0, 10.0), Token::ChartMedium, "between 5 m and the safety depth");
        assert_eq!(depth_color(-10.0, 10.0), Token::ChartDeep, "the safety depth itself is the deep side");
        assert_eq!(depth_color(-500.0, 10.0), Token::ChartDeep);
        assert_eq!(depth_color(-7.0, 3.0), Token::ChartDeep, "a safety depth under 5 m leaves no medium band");
    }

    #[test]
    fn shallow_contours_need_more_zoom() {
        assert!(contour_min_zoom(-2.0) > contour_min_zoom(-20.0));
        assert!(contour_min_zoom(-20.0) > contour_min_zoom(-200.0));
    }

    #[test]
    fn depth_lookup_prefers_the_finest_grid() {
        let layer = DepthLayer::new(Rc::new(|| {}));
        let coarse = Grid { ncols: 2, nrows: 2, xll: 0.0, yll: 0.0, dx: 1.0, dy: 1.0, data: vec![-100.0; 4] };
        let fine = Grid { ncols: 2, nrows: 2, xll: 0.5, yll: 0.5, dx: 0.5, dy: 0.5, data: vec![-7.0; 4] };
        layer.add_grid(DepthGrid::new(fine));
        layer.add_grid(DepthGrid::new(coarse));
        assert_eq!(layer.depth_at(1.0, 1.0), Some(7.0));
        // Outside the fine grid only the coarse one answers.
        assert_eq!(layer.depth_at(0.55, 0.55), Some(100.0));
        assert_eq!(layer.depth_at(9.0, 9.0), None);
    }

    fn flat(xll: f64, yll: f64, n: usize, dx: f64, depth: f32) -> Grid {
        Grid { ncols: n, nrows: n, xll, yll, dx, dy: dx, data: vec![-depth; n * n] }
    }

    #[test]
    fn downloaded_tiles_can_be_hidden_and_dropped() {
        let layer = DepthLayer::new(Rc::new(|| {}));
        let (a, b) = (TileKey { iy: 0, ix: 0 }, TileKey { iy: 0, ix: 40 });
        layer.add_grid(DepthGrid::for_tile(flat(0.0, 0.0, 4, 0.1, 30.0), a));
        layer.add_grid(DepthGrid::for_tile(flat(20.0, 0.0, 4, 0.1, 40.0), b));
        layer.add_grid(DepthGrid::new(flat(-2.0, -2.0, 8, 1.0, 7.0)));
        assert!(layer.has_tile(&a) && layer.has_tile(&b) && layer.tile_count() == 2);
        assert_eq!(layer.depth_at(0.2, 0.2), Some(30.0), "the finer tile answers");
        // Hidden, only the local grid answers.
        layer.set_tiles_shown(false);
        assert_eq!(layer.depth_at(0.2, 0.2), Some(7.0));
        assert_eq!(layer.depth_at(20.2, 0.2), None);
        layer.set_tiles_shown(true);
        assert_eq!(layer.depth_at(20.2, 0.2), Some(40.0));
        // Dropping keeps what is near the view and says what went.
        let rev = layer.changes.get();
        let dropped = layer.drop_tiles_outside(&Bounds { min_lon: -1.0, min_lat: -1.0, max_lon: 1.0, max_lat: 1.0 });
        assert_eq!(dropped, vec![b]);
        assert!(layer.has_tile(&a) && !layer.has_tile(&b));
        assert!(layer.changes.get() > rev, "the cached image is invalidated");
        assert!(layer.drop_tiles_outside(&Bounds { min_lon: -1.0, min_lat: -1.0, max_lon: 1.0, max_lat: 1.0 }).is_empty());
    }

    #[test]
    fn a_finer_grid_takes_over_inside_its_bounds() {
        let coarse = DepthGrid::new(flat(0.0, 0.0, 10, 1.0, 50.0));
        let fine = Arc::new(DepthGrid::new(flat(2.0, 2.0, 4, 0.1, 5.0)));
        let view = Bounds { min_lon: 0.0, min_lat: 0.0, max_lon: 10.0, max_lat: 10.0 };
        assert_eq!(coarse.finer_bounds(std::slice::from_ref(&fine), &view), vec![fine.grid.bounds()]);
        // Not itself, not a coarser one, and not one outside the area asked about.
        assert!(fine.finer_bounds(&[Arc::new(DepthGrid::new(flat(0.0, 0.0, 10, 1.0, 50.0)))], &view).is_empty());
        let away = Bounds { min_lon: 8.0, min_lat: 8.0, max_lon: 9.0, max_lat: 9.0 };
        assert!(coarse.finer_bounds(&[fine], &away).is_empty());
    }
}
