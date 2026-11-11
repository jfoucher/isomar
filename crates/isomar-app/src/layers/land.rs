//! Land: the local coastline stores (SHOM) and, under them, the OpenStreetMap
//! land of the downloaded vector tiles.

use super::credit::Credit;
use super::relief::{ReliefLayer, ReliefPaint};
use super::{Change, Changes, DrawCtx, Layer, Redraw, TileArea, TilePaint};
use crate::core::chart::coast::{CoastStore, lod_for_view};
use crate::core::chart::vtile::{LandTile, TileCache, TileId, resolve, tiles_for};
use crate::core::geo::{self, LatLon};
use crate::core::vector::{Bounds, Polygon, clip_ring};
use crate::palette::{LAND, LAND_EDGE};
use crate::view::ViewInfo;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

/// Tiles kept in memory.
const TILE_CAP: usize = 400;
/// Most tiles a view can need.
const MAX_TILES: usize = 128;

/// Polygons with the bounds of each, so that a tile can leave out those far from it, and their points
/// projected once (Web Mercator pixels at zoom 0), so that a tile only scales and shifts them.
struct Polys {
    bounds: Vec<Bounds>,
    /// Each polygon's rings, projected, with the box of each ring in the same units.
    world: Vec<Vec<(Vec<(f64, f64)>, [f64; 4])>>,
}

impl Polys {
    fn new(list: Vec<Polygon>) -> Polys {
        let bounds = list.iter().map(|p| p.bounds()).collect();
        let world = list
            .iter()
            .map(|p| {
                p.rings
                    .iter()
                    .map(|r| {
                        let pts: Vec<(f64, f64)> = r.iter().map(world).collect();
                        let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
                        for &(x, y) in &pts {
                            b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
                        }
                        (pts, b)
                    })
                    .collect()
            })
            .collect();
        Polys { bounds, world }
    }

    /// The polygons reaching into `area`, by index.
    fn near<'a>(&'a self, area: &'a Bounds) -> impl Iterator<Item = usize> + 'a {
        self.bounds.iter().enumerate().filter(move |(_, b)| b.intersects(area)).map(|(i, _)| i)
    }
}

/// A position in Web Mercator pixels at zoom 0.
fn world(p: &[f64; 2]) -> (f64, f64) {
    geo::project(&LatLon::new(p[1], p[0]), 0.0)
}

/// Turns projected points into a view's screen points: (scale, x offset, y offset), with a shift of a
/// whole world for a polygon on the other side of the antimeridian.
#[derive(Clone, Copy)]
struct ToScreen {
    k: f64,
    ox: f64,
    oy: f64,
}

impl ToScreen {
    fn new(vi: &ViewInfo) -> ToScreen {
        let k = 2f64.powf(vi.zoom());
        let (cx, cy) = vi.view.center_px();
        ToScreen { k, ox: vi.w / 2.0 - cx, oy: vi.h / 2.0 - cy }
    }

    /// The same, shifted by whole worlds to be nearest the view for something at world x `wx`.
    fn near(self, vi: &ViewInfo, wx: f64) -> ToScreen {
        let size = geo::world_size(vi.zoom());
        let dx = wx * self.k + self.ox - vi.w / 2.0;
        ToScreen { ox: self.ox - (dx / size).round() * size, ..self }
    }

    fn at(&self, (x, y): (f64, f64)) -> (f64, f64) {
        (x * self.k + self.ox, y * self.k + self.oy)
    }
}

/// Cuts a closed ring of screen points to the rectangle `r` (x0, y0, x1, y1), Sutherland-Hodgman: the
/// part inside, with the cut running along the rectangle's edge. Cairo then has only what the tile
/// shows to fill, not the whole coast of a country.
fn clip_to_rect(ring: &mut Vec<(f64, f64)>, r: [f64; 4], buf: &mut Vec<(f64, f64)>) {
    for side in 0..4 {
        if ring.is_empty() {
            return;
        }
        buf.clear();
        let inside = |p: (f64, f64)| match side {
            0 => p.0 >= r[0],
            1 => p.0 <= r[2],
            2 => p.1 >= r[1],
            _ => p.1 <= r[3],
        };
        let cross = |a: (f64, f64), b: (f64, f64)| match side {
            0 | 1 => {
                let x = if side == 0 { r[0] } else { r[2] };
                (x, a.1 + (x - a.0) / (b.0 - a.0) * (b.1 - a.1))
            }
            _ => {
                let y = if side == 2 { r[1] } else { r[3] };
                (a.0 + (y - a.1) / (b.1 - a.1) * (b.0 - a.0), y)
            }
        };
        let mut prev = ring[ring.len() - 1];
        for &p in ring.iter() {
            match (inside(prev), inside(p)) {
                (true, true) => buf.push(p),
                (true, false) => buf.push(cross(prev, p)),
                (false, true) => {
                    buf.push(cross(prev, p));
                    buf.push(p);
                }
                (false, false) => {}
            }
            prev = p;
        }
        std::mem::swap(ring, buf);
    }
}

/// Whether the segment `a`-`b` lies wholly on one side outside the rectangle `r`.
fn outside(a: (f64, f64), b: (f64, f64), r: [f64; 4]) -> bool {
    (a.0 < r[0] && b.0 < r[0]) || (a.0 > r[2] && b.0 > r[2]) || (a.1 < r[1] && b.1 < r[1]) || (a.1 > r[3] && b.1 > r[3])
}

/// A source's polygons around the view, clipped to `bounds`.
struct Cache {
    lod: usize,
    bounds: Bounds,
    polys: Arc<Polys>,
}

/// One local coastline store. A higher `detail` is a finer, more trusted source.
struct Source {
    /// Shared with the worker thread that reads it.
    store: Arc<Mutex<CoastStore>>,
    bounds: Bounds,
    detail: u32,
    /// Polygons read at a few levels of detail, the latest used first.
    cache: RefCell<Vec<Cache>>,
    /// A read is under way off the main thread.
    reading: Cell<bool>,
}

pub struct LandLayer {
    /// Sorted by `detail`, coarsest first, which is also the drawing order.
    sources: RefCell<Vec<Rc<Source>>>,
    /// Downloaded OpenStreetMap land, drawn under the local stores.
    tiles: RefCell<TileCache<Arc<LandTile>>>,
    online: Cell<bool>,
    credit: Rc<Credit>,
    changes: Rc<Changes>,
    redraw: Redraw,
    /// Notes a change anywhere and asks for a redraw.
    changed: Redraw,
    /// Hillshade and contour lines over the land, once installed.
    relief: RefCell<Option<Rc<ReliefLayer>>>,
}

impl LandLayer {
    pub fn new(redraw: Redraw, credit: Rc<Credit>) -> LandLayer {
        let changes = Rc::new(Changes::default());
        let changed: Redraw = {
            let (changes, redraw) = (changes.clone(), redraw.clone());
            Rc::new(move || {
                changes.bump(None);
                redraw();
            })
        };
        LandLayer { sources: RefCell::new(Vec::new()), tiles: RefCell::new(TileCache::new(TILE_CAP)), online: Cell::new(false), credit, changes, redraw, changed, relief: RefCell::new(None) }
    }

    /// Draws this relief over the land, and draws the land tiles it touches again when it changes.
    pub fn set_relief(&self, relief: Rc<ReliefLayer>) {
        let changes = self.changes.clone();
        relief.follow(move |area| changes.bump(area));
        *self.relief.borrow_mut() = Some(relief);
    }

    fn changed(&self) {
        (self.changed)();
    }

    /// Adds a coastline. Where a source with a higher `detail` has land data
    /// under the centre of the view, the coarser ones are not drawn.
    pub fn add_store(&self, store: CoastStore, detail: u32) {
        let mut sources = self.sources.borrow_mut();
        let bounds = *store.bounds();
        sources.push(Rc::new(Source { store: Arc::new(Mutex::new(store)), bounds, detail, cache: RefCell::new(Vec::new()), reading: Cell::new(false) }));
        sources.sort_by_key(|s| s.detail);
        drop(sources);
        self.changed();
    }

    /// Drops the local coastlines, before an update loads the new one or after they are removed.
    pub fn remove_stores(&self) {
        self.sources.borrow_mut().clear();
        self.changed();
    }

    /// Whether the downloaded land is drawn.
    pub fn set_online(&self, on: bool) {
        if self.online.replace(on) != on {
            self.changed();
        }
    }

    /// Adds the downloaded land of one tile, replacing any earlier copy.
    pub fn add_tile(&self, id: TileId, land: Arc<LandTile>) {
        self.tiles.borrow_mut().insert(id, land);
        // The tile replaces the coarser one that stood in for it; its coast reaches into the disputed band round it.
        let b = id.bounds();
        let lat = b.min_lat.abs().max(b.max_lat.abs());
        self.changes.bump(Some(b.expanded(2.0 * DISPUTED_M / 111_000.0 / lat.to_radians().cos().max(0.1))));
        (self.redraw)();
    }

    /// Whether the downloaded land is on and has something to draw for every
    /// tile of the view, a coarser tile standing in for a missing one.
    pub fn covers_view(&self, vi: &ViewInfo) -> bool {
        if !self.online.get() {
            return false;
        }
        let wanted = tiles_for(&vi.bounds(), TileId::zoom_for(vi.zoom()), MAX_TILES);
        let tiles = self.tiles.borrow();
        let found: usize = resolve(&wanted, |t| tiles.contains(t)).iter().map(|(_, kids)| kids.len()).sum();
        found == wanted.len()
    }

    pub fn has_tile(&self, id: &TileId) -> bool {
        self.tiles.borrow().contains(id)
    }
}

impl Source {
    /// The polygons last read, clipped to the box they were read for. When
    /// they do not suit `view` (wider by `reach` degrees, which coast beyond
    /// the view affects) and `lod`, a new read starts off the main thread and
    /// `changed` is called when it lands; until then the old ones are drawn.
    /// A store has polygons of 850,000 points, so they are cut to a box a
    /// little bigger than the view, or every frame would trace all of them.
    fn polygons(self: &Rc<Self>, view: &Bounds, reach: f64, lod: usize, changes: &Rc<Changes>, redraw: &Redraw) -> Option<(Arc<Polys>, Bounds)> {
        let needed = view.expanded(reach);
        let fits = |c: &Cache| c.bounds.contains([needed.min_lon, needed.min_lat]) && c.bounds.contains([needed.max_lon, needed.max_lat]);
        let mut caches = self.cache.borrow_mut();
        if let Some(i) = caches.iter().position(|c| c.lod == lod) {
            let c = caches.remove(i);
            caches.insert(0, c);
            if fits(&caches[0]) {
                return Some((caches[0].polys.clone(), caches[0].bounds));
            }
        }
        // Until the read lands: this detail read round somewhere else, or the nearest detail read here.
        let stand_in = caches
            .iter()
            .find(|c| c.lod == lod)
            .or_else(|| caches.iter().filter(|c| fits(c)).min_by_key(|c| c.lod.abs_diff(lod)))
            .or(caches.first())
            .map(|c| (c.polys.clone(), c.bounds));
        if !self.reading.replace(true) {
            let q = needed.expanded(view.width().max(view.height()) * 0.5);
            // Where the polygons were read at this detail before, the new ones draw the same.
            let before = caches.iter().find(|c| c.lod == lod).map(|c| c.bounds);
            let (src, store, changes, redraw) = (self.clone(), self.store.clone(), changes.clone(), redraw.clone());
            crate::platform::spawn(async move {
                let polys = crate::platform::bg(move || {
                    let found = store.lock().map(|s| s.query(&q, lod).unwrap_or_default()).unwrap_or_default();
                    Polys::new(
                        found
                            .into_iter()
                            .filter_map(|p| {
                                let rings: Vec<_> = p.rings.iter().filter_map(|r| clip_ring(r, &q)).collect();
                                (!rings.is_empty()).then_some(Polygon { rings })
                            })
                            .collect::<Vec<_>>(),
                    )
                })
                .await;
                let mut caches = src.cache.borrow_mut();
                caches.retain(|c| c.lod != lod);
                caches.insert(0, Cache { lod, bounds: q, polys: Arc::new(polys) });
                caches.truncate(LODS_KEPT);
                drop(caches);
                src.reading.set(false);
                // Only the tiles of the zooms that read this detail draw it.
                let zooms = lod_zooms(lod, &q);
                match before {
                    Some(b) => {
                        // The coast there also moves the disputed band as far as it reaches.
                        for area in outside_of(&q, &b) {
                            changes.bump_change(Some(Change { area: area.expanded(reach), zooms }));
                        }
                    }
                    None => changes.bump_change(Some(Change { area: q.expanded(reach), zooms })),
                }
                redraw();
            });
        }
        stand_in
    }
}

/// Levels of detail of a coastline kept read at once, so that zooming in and out again finds them.
const LODS_KEPT: usize = 3;

/// The zooms at which `lod_for_view` picks `lod` somewhere in `area`.
fn lod_zooms(lod: usize, area: &Bounds) -> Option<(f64, f64)> {
    let tol = &crate::core::chart::coast::LOD_TOLERANCE_DEG;
    // A tolerance `t` suits while 1.2 degrees a pixel (shrunk by the latitude) is at least `t`.
    let zoom_of = |t: f64, cos: f64| (360.0 * 1.2 * cos.max(0.05) / (256.0 * t)).log2();
    let lat_far = area.min_lat.abs().max(area.max_lat.abs()).min(89.0);
    let lat_near = if area.min_lat <= 0.0 && area.max_lat >= 0.0 { 0.0 } else { area.min_lat.abs().min(area.max_lat.abs()) };
    let (cos_lo, cos_hi) = (lat_far.to_radians().cos(), lat_near.to_radians().cos());
    let lo = if lod == 0 { f64::NEG_INFINITY } else { zoom_of(tol[lod - 1], cos_lo) };
    let hi = if tol.get(lod).is_none_or(|t| *t == 0.0) { f64::INFINITY } else { zoom_of(tol[lod], cos_hi) };
    Some((lo, hi))
}

/// The parts of `a` outside `b`, as up to four boxes. A box that only shrank leaves nothing: what it
/// still covers was read the same.
fn outside_of(a: &Bounds, b: &Bounds) -> Vec<Bounds> {
    let mut out = Vec::new();
    let (lo, hi) = (a.min_lat.max(b.min_lat), a.max_lat.min(b.max_lat));
    if a.min_lon < b.min_lon {
        out.push(Bounds { max_lon: b.min_lon.min(a.max_lon), ..*a });
    }
    if a.max_lon > b.max_lon {
        out.push(Bounds { min_lon: b.max_lon.max(a.min_lon), ..*a });
    }
    let (w, e) = (a.min_lon.max(b.min_lon), a.max_lon.min(b.max_lon));
    if w < e {
        if a.min_lat < lo {
            out.push(Bounds { min_lon: w, max_lon: e, min_lat: a.min_lat, max_lat: lo.min(a.max_lat) });
        }
        if a.max_lat > hi {
            out.push(Bounds { min_lon: w, max_lon: e, min_lat: hi.max(a.min_lat), max_lat: a.max_lat });
        }
    }
    out
}

/// A source's land traced for one tile: the rings to fill, and the coast to
/// stroke (the rings without the cuts of the clip).
struct Traced {
    fill: cairo::Path,
    edge: cairo::Path,
    /// The coast as points, a NaN before the first point of each line, for `disputed_mask`.
    coast: Vec<(f64, f64)>,
}

impl Traced {
    /// Traces the polygons that reach into `near`, the part of the world the tile can be affected by, as
    /// far as they reach into `r`, the same on screen.
    fn new(cr: &cairo::Context, vi: &ViewInfo, polys: &Polys, clip: &Bounds, near: &Bounds, r: [f64; 4]) -> Option<Traced> {
        let base = ToScreen::new(vi);
        let (q0, q1) = (world(&[clip.min_lon, clip.max_lat]), world(&[clip.max_lon, clip.min_lat]));
        let (mut ring, mut buf) = (Vec::new(), Vec::new());
        let mut edges: Vec<(f64, f64)> = Vec::new();
        cr.new_path();
        for i in polys.near(near) {
            let rings = &polys.world[i];
            let Some(first) = rings.first() else { continue };
            let to = base.near(vi, (first.1[0] + first.1[2]) / 2.0);
            for (pts, b) in rings {
                let (lo, hi) = (to.at((b[0], b[1])), to.at((b[2], b[3])));
                if hi.0 < r[0] || lo.0 > r[2] || hi.1 < r[1] || lo.1 > r[3] {
                    continue;
                }
                ring.clear();
                ring.extend(pts.iter().map(|p| to.at(*p)));
                clip_to_rect(&mut ring, r, &mut buf);
                if let Some(&(x, y)) = ring.first() {
                    cr.move_to(x, y);
                    for &(x, y) in &ring[1..] {
                        cr.line_to(x, y);
                    }
                    cr.close_path();
                }
                // The coast: the ring without the cuts along the box the polygons were read for.
                let n = pts.len();
                let mut pen_down = false;
                for j in 0..n {
                    let (a, c) = (pts[j], pts[(j + 1) % n]);
                    let on_cut = (a.0 == c.0 && (a.0 == q0.0 || a.0 == q1.0)) || (a.1 == c.1 && (a.1 == q0.1 || a.1 == q1.1));
                    let (sa, sc) = (to.at(a), to.at(c));
                    if on_cut || outside(sa, sc, r) {
                        pen_down = false;
                        continue;
                    }
                    if !pen_down {
                        edges.push((f64::NAN, f64::NAN));
                        edges.push(sa);
                    }
                    edges.push(sc);
                    pen_down = true;
                }
            }
        }
        let fill = cr.copy_path().ok()?;
        cr.new_path();
        trace_lines(cr, &edges, 0.0);
        let edge = cr.copy_path().ok()?;
        cr.new_path();
        Some(Traced { fill, edge, coast: edges })
    }
}

/// Adds lines given as points, a NaN before the first point of each, to the path, leaving out points
/// within `tolerance` of the last one kept (but never a line's last).
fn trace_lines(cr: &cairo::Context, pts: &[(f64, f64)], tolerance: f64) {
    let mut last = (f64::NAN, f64::NAN);
    let mut i = 0;
    while i < pts.len() {
        let p = pts[i];
        if p.0.is_nan() {
            if let Some(&(x, y)) = pts.get(i + 1) {
                cr.move_to(x, y);
                last = (x, y);
            }
            i += 2;
            continue;
        }
        let end = pts.get(i + 1).is_none_or(|n| n.0.is_nan());
        if end || (p.0 - last.0).hypot(p.1 - last.1) >= tolerance {
            cr.line_to(p.0, p.1);
            last = p;
        }
        i += 1;
    }
}

/// The indices of the sources to draw for `view`: every source that touches
/// it, except those hidden by a finer source covering the view's centre.
fn visible_sources(bounds: &[(u32, Bounds)], view: &Bounds) -> Vec<usize> {
    let centre = [(view.min_lon + view.max_lon) / 2.0, (view.min_lat + view.max_lat) / 2.0];
    (0..bounds.len())
        .filter(|&i| bounds[i].1.intersects(view))
        .filter(|&i| !bounds.iter().any(|(d, b)| *d > bounds[i].0 && b.contains(centre)))
        .collect()
}

impl LandLayer {
    /// The land polygons touching `b` at full detail from the local stores,
    /// for routing. Only the finest source that has land there is used, so
    /// shorelines don't double up. Empty away from them.
    pub fn polygons_in(&self, b: &Bounds) -> Vec<Polygon> {
        let last = crate::core::chart::coast::LOD_TOLERANCE_DEG.len() - 1;
        let sources = self.sources.borrow();
        let bounds: Vec<(u32, Bounds)> = sources.iter().map(|s| (s.detail, s.bounds)).collect();
        let Some(&i) = visible_sources(&bounds, b).last() else { return Vec::new() };
        sources[i].store.lock().ok().and_then(|s| s.query(b, last).ok()).unwrap_or_default()
    }
}

/// The downloaded land tiles that suit an area: each wanted tile, or the nearest coarser one loaded in
/// its place, with the wanted tiles it stands for.
type Online = Vec<(TileId, Vec<TileId>, Arc<LandTile>)>;

impl LandLayer {
    fn online_for(&self, wanted: &[TileId]) -> Online {
        let resolved = {
            let tiles = self.tiles.borrow();
            resolve(wanted, |t| tiles.contains(t))
        };
        resolved.into_iter().filter_map(|(src, areas)| self.tiles.borrow_mut().get(&src).map(|land| (src, areas, land))).collect()
    }
}

/// What the land tiles are painted from: the local stores' polygons and the downloaded tiles around the
/// area, as they were when the tiles were prepared.
struct LandPaint {
    /// Each visible source's polygons and the box they were cut to.
    local: Vec<(Arc<Polys>, Bounds)>,
    /// The downloaded land to draw, if it is switched on: for the area, and for a band round it.
    online: Option<(Online, Online)>,
    /// The heights to shade the land with.
    relief: Option<Arc<ReliefPaint>>,
}

impl LandPaint {
    /// Draws the downloaded land. A tile still missing is stood in for by the
    /// nearest coarser one, clipped to the area it replaces.
    fn draw_online(&self, ctx: &DrawCtx, part: Part) {
        let (cr, vi, pal) = (ctx.cr, &ctx.vi, &ctx.pal);
        let Some((online, _)) = &self.online else { return };
        for (src, areas, land) in online {
            if areas.is_empty() || land.is_empty() || !shown(vi, src, 0.0) {
                continue;
            }
            let _ = cr.save();
            if areas.as_slice() != [*src] {
                cr.new_path();
                for t in areas {
                    let (x, y, size) = tile_origin(vi, t);
                    cr.rectangle(x, y, size, size);
                }
                cr.clip();
            }
            draw_land(cr, vi, pal, src, land, part);
            let _ = cr.restore();
        }
    }

    /// The area within `DISPUTED_M` of both the local stores' coast and the
    /// downloaded one, opaque in an alpha mask. None if it would be empty.
    /// The band is wide and soft, so the mask is drawn at a fraction of the
    /// screen's resolution: wide strokes cost cairo in proportion to the
    /// pixels they cover, and at full size this took seconds a frame.
    fn disputed_mask(&self, ctx: &DrawCtx, local: &[Traced]) -> Option<cairo::Pattern> {
        let vi = &ctx.vi;
        let width = 2.0 * (DISPUTED_M / metres_per_px(vi)).min(vi.w.max(vi.h));
        if width < 2.0 {
            return None;
        }
        let f = (width / 16.0).clamp(1.0, 8.0);
        let surface = cairo::ImageSurface::create(cairo::Format::A8, (vi.w / f).ceil() as i32 + 1, (vi.h / f).ceil() as i32 + 1).ok()?;
        let cr = cairo::Context::new(&surface).ok()?;
        cr.scale(1.0 / f, 1.0 / f);
        cr.set_source_rgba(0.0, 0.0, 0.0, 1.0);
        cr.set_line_join(cairo::LineJoin::Round);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_line_width(width);
        cr.new_path();
        // At the mask's resolution, under a band this wide, detail finer than a pixel of it changes
        // nothing; the stroker's work is by the segment, and the coasts round a tile have thousands.
        for t in local {
            trace_lines(&cr, &t.coast, f);
        }
        let _ = cr.stroke();
        cr.set_operator(cairo::Operator::DestIn);
        cr.new_path();
        // Coast beyond the tile counts too, as far as the band reaches.
        if let Some((_, around)) = &self.online {
            for (src, _, land) in around {
                if shown(vi, src, width / 2.0) {
                    let pad = width / 2.0 + 4.0;
                    trace_coast(&cr, vi, src, land, [-pad, -pad, vi.w + pad, vi.h + pad], f);
                }
            }
        }
        let _ = cr.stroke();
        // Next to a land border the foreign shore is near both coasts too.
        cr.set_operator(cairo::Operator::Clear);
        cr.new_path();
        for b in NEIGHBOURS {
            let (x0, y0) = vi.to_screen(&LatLon::new(b.max_lat, b.min_lon));
            let (x1, y1) = vi.to_screen(&LatLon::new(b.min_lat, b.max_lon));
            cr.rectangle(x0, y0, x1 - x0, y1 - y0);
        }
        let _ = cr.fill();
        drop(cr);
        let mask = cairo::SurfacePattern::create(&surface);
        mask.set_matrix(cairo::Matrix::new(1.0 / f, 0.0, 0.0, 1.0 / f, 0.0, 0.0));
        mask.set_filter(cairo::Filter::Bilinear);
        mask.set_extend(cairo::Extend::Pad);
        Some((*mask).clone())
    }
}

/// Whether the tile `id` reaches within `margin` pixels of the view.
fn shown(vi: &ViewInfo, id: &TileId, margin: f64) -> bool {
    let (x, y, size) = tile_origin(vi, id);
    x < vi.w + margin && x + size > -margin && y < vi.h + margin && y + size > -margin
}

/// How far beyond the view, in degrees, coast affects the disputed band.
fn disputed_reach(vi: &ViewInfo) -> f64 {
    DISPUTED_M / 111_000.0 / vi.view.center.lat.to_radians().cos().max(0.1)
}

/// How far from both coasts the downloaded land gives way to the local stores.
const DISPUTED_M: f64 = 1500.0;

/// Where France's land borders reach the sea: here the downloaded land is
/// only cut by the local stores' land, so the neighbour's shore stays.
const NEIGHBOURS: [Bounds; 5] = [
    // Hendaye and Hondarribia, up the Bidasoa.
    Bounds { min_lon: -1.86, min_lat: 43.30, max_lon: -1.72, max_lat: 43.40 },
    // Cerbère and Portbou.
    Bounds { min_lon: 3.12, min_lat: 42.40, max_lon: 3.22, max_lat: 42.47 },
    // Monaco.
    Bounds { min_lon: 7.39, min_lat: 43.71, max_lon: 7.46, max_lat: 43.76 },
    // Menton and Ventimiglia.
    Bounds { min_lon: 7.49, min_lat: 43.76, max_lon: 7.60, max_lat: 43.83 },
    // Bray-Dunes and De Panne.
    Bounds { min_lon: 2.48, min_lat: 51.05, max_lon: 2.64, max_lat: 51.13 },
];

fn metres_per_px(vi: &ViewInfo) -> f64 {
    156_543.03 * vi.view.center.lat.to_radians().cos() / 2f64.powf(vi.zoom())
}

/// Screen position and size of a tile.
pub(super) fn tile_origin(vi: &ViewInfo, id: &TileId) -> (f64, f64, f64) {
    let (cx, cy) = vi.view.center_px();
    let size = 256.0 * 2f64.powf(vi.zoom() - id.z as f64);
    (vi.w / 2.0 + id.x as f64 * size - cx, vi.h / 2.0 + id.y as f64 * size - cy, size)
}

/// What of a tile's land is drawn: the fill, the outline, or both.
#[derive(Clone, Copy, PartialEq)]
enum Part {
    Fill,
    Edge,
    Both,
}

fn draw_land(cr: &cairo::Context, vi: &ViewInfo, pal: &crate::palette::Palette, id: &TileId, land: &LandTile, part: Part) {
    let (ox, oy, size) = tile_origin(vi, id);
    let k = size / land.extent as f64;
    let at = |p: [i16; 2]| (ox + p[0] as f64 * k, oy + p[1] as f64 * k);
    // A downloaded tile can be many of the chart's tiles across (zoomed past 14): only what this one shows.
    let r = [-4.0, -4.0, vi.w + 4.0, vi.h + 4.0];
    cr.set_fill_rule(cairo::FillRule::EvenOdd);
    cr.set_line_join(cairo::LineJoin::Round);
    cr.set_line_width(1.0);
    if part != Part::Edge {
        cr.new_path();
        let (mut pts, mut buf) = (Vec::new(), Vec::new());
        for ring in &land.rings {
            pts.clear();
            pts.extend(ring.iter().map(|p| at(*p)));
            clip_to_rect(&mut pts, r, &mut buf);
            if let Some(&(x, y)) = pts.first() {
                cr.move_to(x, y);
                for &(x, y) in &pts[1..] {
                    cr.line_to(x, y);
                }
                cr.close_path();
            }
        }
        pal.set(cr, LAND);
        let _ = cr.fill();
    }
    if part == Part::Fill {
        return;
    }
    cr.new_path();
    trace_coast(cr, vi, id, land, r, 0.0);
    pal.set(cr, LAND_EDGE);
    let _ = cr.stroke();
}

/// Adds a tile's coastline to the current path: its outline, without the
/// straight cuts along the edge of the tile, as far as it reaches into `r` on screen.
fn trace_coast(cr: &cairo::Context, vi: &ViewInfo, id: &TileId, land: &LandTile, r: [f64; 4], tolerance: f64) {
    let (ox, oy, size) = tile_origin(vi, id);
    let k = size / land.extent as f64;
    let at = |p: [i16; 2]| (ox + p[0] as f64 * k, oy + p[1] as f64 * k);
    let mut pts = Vec::new();
    for ring in &land.rings {
        let n = ring.len();
        let mut pen_down = false;
        for i in 0..n {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            let (sa, sb) = (at(a), at(b));
            if land.on_edge(a, b) || outside(sa, sb, r) {
                pen_down = false;
                continue;
            }
            if !pen_down {
                pts.push((f64::NAN, f64::NAN));
                pts.push(sa);
            }
            pts.push(sb);
            pen_down = true;
        }
    }
    trace_lines(cr, &pts, tolerance);
}

impl Layer for LandLayer {
    fn id(&self) -> &'static str {
        "land"
    }

    fn revision(&self) -> u64 {
        self.changes.get()
    }

    fn changes_since(&self, revision: u64) -> Option<Vec<super::Change>> {
        self.changes.since(revision)
    }

    fn prepare(&self, area: &TileArea) -> Option<Arc<dyn TilePaint>> {
        let vi = &area.vi;
        let sources = self.sources.borrow();
        let lod = lod_for_view(vi.zoom(), vi.view.center.lat);
        let view = vi.bounds();
        let bounds: Vec<(u32, Bounds)> = sources.iter().map(|s| (s.detail, s.bounds)).collect();
        let reach = disputed_reach(vi);
        let local: Vec<(Arc<Polys>, Bounds)> = visible_sources(&bounds, &view).into_iter().filter_map(|i| sources[i].polygons(&view, reach, lod, &self.changes, &self.redraw)).collect();
        let online = self.online.get().then(|| {
            let zoom = TileId::zoom_for(vi.zoom());
            let wanted = tiles_for(&view, zoom, MAX_TILES);
            // The coast within the disputed band, beyond the area, matters to the tiles at its edge.
            let around = if local.is_empty() { Vec::new() } else { self.online_for(&tiles_for(&view.expanded(reach), zoom, 2 * MAX_TILES)) };
            (self.online_for(&wanted), around)
        });
        self.credit.land.set(online.as_ref().is_some_and(|(o, _)| o.iter().any(|(_, areas, land)| !areas.is_empty() && !land.is_empty())));
        let relief = self.relief.borrow().as_ref().and_then(|r| r.prepare(area));
        (!local.is_empty() || online.is_some()).then(|| Arc::new(LandPaint { local, online, relief }) as Arc<dyn TilePaint>)
    }
}

impl TilePaint for LandPaint {
    fn paint(&self, ctx: &DrawCtx) {
        let (cr, vi, pal) = (ctx.cr, &ctx.vi, &ctx.pal);
        // The land is drawn in a group when there is relief to put on it: laid `Atop`, the relief
        // reaches only the land's own pixels, whichever coast drew them.
        if self.relief.is_some() {
            cr.push_group();
        }
        // What can affect this tile: the band round a coast reaches this far, in degrees.
        let near = vi.bounds().expanded(disputed_reach(vi).max(coast_band_px(vi) * vi.deg_per_px()));
        // The same on screen: what is drawn here, and what the disputed band and the coast band round it reach.
        let pad = (DISPUTED_M / metres_per_px(vi)).min(vi.w.max(vi.h)).max(coast_band_px(vi)) + 4.0;
        let r = [-pad, -pad, vi.w + pad, vi.h + pad];
        let local: Vec<Traced> = self.local.iter().filter_map(|(polys, clip)| Traced::new(cr, vi, polys, clip, &near, r)).collect();
        if self.online.is_some() {
            if local.is_empty() {
                self.draw_online(ctx, Part::Both);
            } else {
                // The downloaded land goes wherever the local stores have none:
                // their land is cut out of it, so their coast is not drawn twice.
                // The outline is also cut out of a band along their coast, where
                // the two shorelines disagree, so it never shows beside theirs.
                // The band takes only the outline: clearing the fill would show
                // whatever is under the land (the depth grids shade land too).
                // Their coast is trusted over it further out too: what lies near
                // both coasts is where the two disagree (OSM often puts drying
                // flats ashore), so it is cleared as well. Land near their coast
                // but far from the sea (across a border) is kept.
                let disputed = self.disputed_mask(ctx, &local);
                        for (part, band) in [(Part::Fill, false), (Part::Edge, true)] {
                    cr.push_group();
                    self.draw_online(ctx, part);
                    let _ = cr.save();
                    cr.set_operator(cairo::Operator::Clear);
                    cr.set_fill_rule(cairo::FillRule::EvenOdd);
                    cr.set_line_join(cairo::LineJoin::Round);
                    cr.set_line_width(2.0 * coast_band_px(vi));
                    cr.new_path();
                    for t in &local {
                        cr.append_path(&t.fill);
                    }
                    let _ = cr.fill();
                    if band {
                        for t in &local {
                            cr.append_path(&t.edge);
                        }
                        let _ = cr.stroke();
                    }
                    if let Some(mask) = &disputed {
                        // Painted as the source: `cr.mask` with DestOut left the outline.
                        cr.set_operator(cairo::Operator::DestOut);
                        let _ = cr.set_source(mask);
                        let _ = cr.paint();
                    }
                    let _ = cr.restore();
                    let _ = cr.pop_group_to_source();
                    let _ = cr.paint();
                }
            }
        }
        cr.set_fill_rule(cairo::FillRule::EvenOdd);
        cr.set_line_join(cairo::LineJoin::Round);
        cr.set_line_width(1.0);
        for t in &local {
            cr.new_path();
            cr.append_path(&t.fill);
            pal.set(cr, LAND);
            let _ = cr.fill();
            cr.append_path(&t.edge);
            pal.set(cr, LAND_EDGE);
            let _ = cr.stroke();
        }
        if let Some(relief) = &self.relief {
            let _ = cr.save();
            cr.set_operator(cairo::Operator::Atop);
            relief.paint(ctx, true);
            let _ = cr.restore();
            let _ = cr.pop_group_to_source();
            let _ = cr.paint();
        }
    }
}

/// How far (in pixels) from a local coast the downloaded land's outline is left
/// out: about 80 m on the ground, but at least 3 px and at most 30 px.
fn coast_band_px(vi: &ViewInfo) -> f64 {
    (80.0 / metres_per_px(vi)).clamp(3.0, 30.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(w: f64, s: f64, e: f64, n: f64) -> Bounds {
        Bounds { min_lon: w, min_lat: s, max_lon: e, max_lat: n }
    }

    fn area(ring: &[(f64, f64)]) -> f64 {
        let n = ring.len();
        (0..n).map(|i| ring[i].0 * ring[(i + 1) % n].1 - ring[(i + 1) % n].0 * ring[i].1).sum::<f64>().abs() / 2.0
    }

    #[test]
    fn a_ring_is_cut_to_the_rectangle() {
        let (mut ring, mut buf) = (vec![(-50.0, -50.0), (150.0, -50.0), (150.0, 50.0), (-50.0, 50.0)], Vec::new());
        clip_to_rect(&mut ring, [0.0, 0.0, 100.0, 100.0], &mut buf);
        assert!((area(&ring) - 100.0 * 50.0).abs() < 1e-9, "{ring:?}");
        assert!(ring.iter().all(|p| (0.0..=100.0).contains(&p.0) && (0.0..=100.0).contains(&p.1)));
        // Wholly outside: nothing; round the rectangle: the rectangle.
        let mut out = vec![(200.0, 200.0), (300.0, 200.0), (300.0, 300.0)];
        clip_to_rect(&mut out, [0.0, 0.0, 100.0, 100.0], &mut buf);
        assert!(out.is_empty());
        let mut round = vec![(-10.0, -10.0), (110.0, -10.0), (110.0, 110.0), (-10.0, 110.0)];
        clip_to_rect(&mut round, [0.0, 0.0, 100.0, 100.0], &mut buf);
        assert!((area(&round) - 10_000.0).abs() < 1e-9);
    }

    #[test]
    fn a_box_moved_or_grown_leaves_strips_and_one_that_shrank_none() {
        let old = b(0.0, 0.0, 10.0, 10.0);
        assert!(outside_of(&b(2.0, 2.0, 8.0, 8.0), &old).is_empty());
        let strips = outside_of(&b(5.0, -2.0, 15.0, 8.0), &old);
        let total: f64 = strips.iter().map(|s| s.width() * s.height()).sum();
        // The new box is 100 square degrees, 40 of them inside the old one.
        assert!((total - 60.0).abs() < 1e-9, "{strips:?}");
        assert!(strips.iter().all(|s| s.width() > 0.0 && s.height() > 0.0));
    }

    #[test]
    fn each_level_of_detail_is_read_at_the_zooms_it_says() {
        let area = b(-5.0, 47.0, -4.0, 48.0);
        for zoom in [6.0, 9.5, 12.0, 14.3, 17.0] {
            let lod = lod_for_view(zoom, 47.5);
            let (lo, hi) = lod_zooms(lod, &area).unwrap();
            assert!(lo < zoom && zoom <= hi, "zoom {zoom}, lod {lod}: {lo}..{hi}");
        }
    }

    #[test]
    fn finer_sources_hide_coarser_ones_where_they_have_land() {
        let world = (0, b(-180.0, -85.0, 180.0, 85.0));
        let france = (30, b(-6.0, 41.0, 10.0, 51.0));
        let all = [world, france];
        // Over Brittany only the French source is drawn.
        assert_eq!(visible_sources(&all, &b(-5.0, 48.0, -4.0, 49.0)), vec![1]);
        // Over Tahiti the French source does not intersect the view.
        assert_eq!(visible_sources(&all, &b(-150.0, -18.0, -149.0, -17.0)), vec![0]);
        // A view centred outside France that spills into it draws both.
        assert_eq!(visible_sources(&all, &b(8.0, 45.0, 14.0, 47.0)), vec![0, 1]);
    }

    #[test]
    fn tiles_are_placed_where_the_view_puts_them() {
        use crate::view::View;
        // Centred on the corner shared by four tiles of zoom 3, at view zoom 3.
        let vi = ViewInfo { view: View { center: LatLon::new(0.0, 0.0), zoom: 3.0 }, w: 800.0, h: 600.0 };
        let (x, y, size) = tile_origin(&vi, &TileId { z: 3, x: 4, y: 4 });
        assert_eq!((x, y, size), (400.0, 300.0, 256.0), "the south-east tile starts at the centre");
        let (x, y, _) = tile_origin(&vi, &TileId { z: 3, x: 3, y: 3 });
        assert_eq!((x, y), (400.0 - 256.0, 300.0 - 256.0));
        // Zoomed in one level, the same tile is twice as big.
        let vi = ViewInfo { view: View { center: LatLon::new(0.0, 0.0), zoom: 4.0 }, ..vi };
        assert_eq!(tile_origin(&vi, &TileId { z: 3, x: 4, y: 4 }).2, 512.0);
    }
}
