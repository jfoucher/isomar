//! Town and city names, their built-up outlines and, close in, their
//! buildings, and the names of seas, bays and straits: the downloaded
//! OpenFreeMap tiles. Drawn above the land and
//! below the chart layers, so a chart that covers the area hides them.

use super::aids::{AidsLayer, AidsPaint};
use super::credit::Credit;
use super::land::tile_origin;
use super::names::{self, PathLabel, Placed, Style};
use super::regions::RegionNames;
use super::relief::{ReliefLayer, ReliefPaint};
use super::capes::CapeNames;
use super::waters::{Candidate, Island, MIN_POINT_WATER_PX, WaterNames, has_axis, point_em, unit_of};
use super::{Changes, DrawCtx, LabelPlacer, Layer, Redraw, TilePaint, TileArea};
use crate::core::chart::places::{Place, PlaceKind, PlacesTile};
use crate::core::chart::vtile::{TileCache, TileId, resolve, tiles_for};
use crate::palette::{BUILDING, BUILDING_EDGE, Paint, Palette, TOWN, TOWN_EDGE};
use crate::view::ViewInfo;
use std::cell::{Cell, RefCell};
use crate::core::vector::{Bounds, Pt};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::rc::Rc;
use std::sync::Arc;

/// Tiles kept in memory.
const TILE_CAP: usize = 400;
/// Most tiles a view can need.
const MAX_TILES: usize = 128;
/// View zoom from which the built-up outlines and the buildings are drawn.
const MIN_ZOOM_URBAN: f64 = 7.0;
const MIN_ZOOM_BUILDINGS: f64 = 13.0;
/// Below this zoom only what is on the coast is drawn; closer in, everything in view is.
const COASTAL_BELOW_ZOOM: f64 = 12.0;
/// How far from the sea counts as the coast, in metres, and the least it is on screen.
const COAST_M: f64 = 5000.0;
const COAST_MIN_PX: f64 = 6.0;
/// Built-up areas smaller than this on screen are specks, not towns.
const MIN_URBAN_PX: f64 = 5.0;

pub struct TownLayer {
    tiles: RefCell<TileCache<Arc<PlacesTile>>>,
    online: Cell<bool>,
    credit: Rc<Credit>,
    changes: Rc<Changes>,
    redraw: Redraw,
    /// The names of seas, straits and capes from Natural Earth, set along their features.
    regions: RegionNames,
    /// The names of waters and islands from the tiles, set along their own water where the tiles show it.
    waters: WaterNames,
    /// The names of capes and points, out along each from its tip.
    capes: CapeNames,
    /// The snapshot of the last area, which the soundings ask for before this layer draws.
    last: RefCell<Option<(AreaKey, Option<Arc<TownPaint>>)>>,
    /// Every island named in a tile loaded so far, at any zoom: the tiles leave out an island now and
    /// then (Île de Ré is only in the zoom 9 tile), so what a coarser tile named is kept.
    seen: RefCell<HashMap<String, Vec<Seen>>>,
    /// The tiles whose names are in `seen`.
    named: RefCell<HashSet<TileId>>,
    /// The marks and lights, which names keep clear of: they matter more to a sailor than a name.
    aids: RefCell<Option<Rc<AidsLayer>>>,
    /// The relief shown through the built-up areas, which would otherwise hide it.
    relief: RefCell<Option<Rc<ReliefLayer>>>,
    /// How many of `changes` were the relief's: they redraw tiles but leave the snapshot as it is.
    relief_bumps: Rc<Cell<u64>>,
    /// The marks' revision the names were last placed round.
    aids_seen: Cell<u64>,
    /// Told when the shapes of the names want tiles the view does not hold.
    on_wanted: RefCell<Option<Rc<dyn Fn()>>>,
}

/// An island a tile named: where (units), how big (the tiles' rank) and the zoom of the finest tile that named it.
#[derive(Clone, Copy, Debug)]
struct Seen {
    at: Pt,
    rank: u32,
    z: u8,
}

/// How far past a tile, in pixels, the names it places are weighed.
const PLAN_REACH_PX: f64 = 400.0;

/// Two names of an island this close (units, about 3 km) are the same island.
const SAME_ISLAND_UNITS: f64 = 0.02;
/// Islands remembered, at most.
const SEEN_KEPT: usize = 20_000;

/// An area and the layer's revision, bit for bit.
type AreaKey = ([u64; 5], u64, bool);

impl TownLayer {
    /// Shows this relief through the built-up areas, and draws the tiles it touches again when it changes.
    pub fn set_relief(&self, relief: Rc<ReliefLayer>) {
        let (changes, bumps) = (self.changes.clone(), self.relief_bumps.clone());
        relief.follow(move |area| {
            bumps.set(bumps.get() + 1);
            changes.bump(area);
        });
        *self.relief.borrow_mut() = Some(relief);
    }

    pub fn new(redraw: Redraw, credit: Rc<Credit>) -> TownLayer {
        TownLayer {
            tiles: RefCell::new(TileCache::new(TILE_CAP)),
            online: Cell::new(false),
            credit,
            changes: Rc::new(Changes::default()),
            redraw,
            regions: RegionNames::new(),
            waters: WaterNames::new(),
            capes: CapeNames::new(),
            last: RefCell::new(None),
            seen: RefCell::new(HashMap::new()),
            named: RefCell::new(HashSet::new()),
            aids: RefCell::new(None),
            relief: RefCell::new(None),
            relief_bumps: Rc::new(Cell::new(0)),
            aids_seen: Cell::new(0),
            on_wanted: RefCell::new(None),
        }
    }

    /// The tiles past the view that the shapes of its waters need (a lagoon reaching out of it).
    pub fn shape_tiles(&self) -> Vec<TileId> {
        self.waters.wanted()
    }

    /// Calls `f` when there are new `shape_tiles` to fetch.
    pub fn connect_wanted(&self, f: impl Fn() + 'static) {
        *self.on_wanted.borrow_mut() = Some(Rc::new(f));
    }

    fn changed(&self) {
        self.bump_everywhere("shown or hidden");
        (self.redraw)();
    }

    /// Draws every tile again: logged under `ISOMAR_DEBUG_TIMING`, since it is costly.
    fn bump_everywhere(&self, why: &str) {
        if crate::debug::timing() {
            isomar_core::log!("isomar: towns drawn again everywhere: {why}");
        }
        self.changes.bump(None);
    }

    /// Whether the downloaded towns are drawn.
    pub fn set_online(&self, on: bool) {
        if self.online.replace(on) != on {
            self.changed();
        }
    }

    /// Where the marks moved, the names are placed again.
    fn follow_aids(&self) {
        let Some(aids) = self.aids.borrow().clone() else { return };
        let (changes, now) = aids.changes_after(self.aids_seen.get());
        if now == self.aids_seen.get() {
            return;
        }
        self.aids_seen.set(now);
        // The names along paths were set clear of the old marks.
        self.regions.forget();
        self.waters.forget();
        self.capes.forget();
        match changes {
            Some(all) => all.into_iter().for_each(|c| self.changes.bump_change(Some(c))),
            None => self.bump_everywhere("the marks changed somewhere"),
        }
    }

    /// The layer of marks and lights the names keep clear of.
    pub fn set_aids(&self, aids: Rc<AidsLayer>) {
        *self.aids.borrow_mut() = Some(aids);
    }

    pub fn add_tile(&self, id: TileId, tile: Arc<PlacesTile>) {
        self.remember(&id, &tile);
        self.tiles.borrow_mut().insert(id, tile);
        // Its names run past it, and its sea decides which names near the shore show. One fetched for the shape of a
        // water may be out of the view, which then has nothing to draw again and would never ask for the shape: the
        // tiles under the waters that want it are drawn again too.
        let mut area = super::reach_of(&id, COAST_M);
        if let Some(waters) = self.waters.wanted_by(&id) {
            area.union(&waters);
        }
        self.changes.bump(Some(area));
        (self.redraw)();
    }

    pub fn has_tile(&self, id: &TileId) -> bool {
        self.tiles.borrow().contains(id)
    }

    /// Takes only the island names of a coarser tile than the view's, which the view's own tiles may
    /// leave out.
    pub fn add_names(&self, id: TileId, tile: &PlacesTile) {
        if self.remember(&id, tile) {
            self.changes.bump(Some(super::reach_of(&id, COAST_M)));
            (self.redraw)();
        }
    }

    /// Whether the names of the tile `id` are known.
    pub fn has_names(&self, id: &TileId) -> bool {
        self.named.borrow().contains(id)
    }

    /// Notes the islands of a tile; whether there was one not known before.
    fn remember(&self, id: &TileId, tile: &PlacesTile) -> bool {
        if !self.named.borrow_mut().insert(*id) {
            return false;
        }
        let mut seen = self.seen.borrow_mut();
        if seen.len() > SEEN_KEPT {
            seen.clear();
        }
        let (mut new, mut renamed) = (false, false);
        let e = tile.extent.max(1) as f64;
        for p in tile.places.iter().filter(|p| p.kind == PlaceKind::Island) {
            let at = unit_of(id, e, [p.at[0] as f64, p.at[1] as f64]);
            let all = seen.entry(p.name.clone()).or_default();
            match all.iter_mut().find(|s| (s.at[0] - at[0]).hypot(s.at[1] - at[1]) < SAME_ISLAND_UNITS) {
                Some(s) => {
                    s.rank = s.rank.min(p.rank);
                    if id.z > s.z {
                        *s = Seen { at, z: id.z, ..*s };
                    }
                }
                None => {
                    all.push(Seen { at, rank: p.rank, z: id.z });
                    new = true;
                }
            }
            // Natural Earth's island here is called by the tiles' name from now on.
            renamed |= self.regions.learn(at, &p.name);
        }
        // A Natural Earth name set anywhere may have changed.
        if renamed {
            self.bump_everywhere("an island's local name");
        }
        new || renamed
    }
}

/// The view zoom from which a place is named. At small scales a chart names the waters, the islands
/// and the countries (`regions`), not the towns; closer in the bigger places (a lower rank) come first.
/// A water named along a line (`line`) is a big one; at a point, a small one.
pub(super) fn min_zoom(kind: PlaceKind, rank: u32, line: bool) -> f64 {
    match kind {
        PlaceKind::City => match rank {
            0..=6 => 7.0,
            7..=10 => 8.0,
            _ => 9.0,
        },
        PlaceKind::Town => match rank {
            0..=7 => 8.0,
            8..=11 => 9.0,
            12 => 9.5,
            13 => 10.0,
            _ => 10.5,
        },
        PlaceKind::Island => island_zoom(rank),
        PlaceKind::Sea => 7.0,
        PlaceKind::Strait | PlaceKind::Bay if line => 9.0,
        PlaceKind::Strait => 11.0,
        PlaceKind::Bay => 11.5,
        k => k.min_zoom(),
    }
}

/// The view zoom from which an island of the tiles' rank (by its area: Oléron and Ré 2, Houat and
/// Aix 5, Fort Boyard 6) is named, where its land is not known.
fn island_zoom(rank: u32) -> f64 {
    match rank {
        0..=1 => 7.0,
        2 => 8.0,
        3 => 9.0,
        4 => 10.0,
        5 => 10.5,
        6 => 12.0,
        _ => 13.0,
    }
}

/// The order places win a crowded spot in, lower first: an island's name before a town's (the island is
/// what a sailor sees), the big islands before the cities, a big water (named along a line) before a
/// small island, and a pass between islands after them.
fn priority(kind: PlaceKind, rank: u32, line: bool) -> u8 {
    match kind {
        PlaceKind::Sea => 0,
        PlaceKind::Island if rank <= 4 => 1,
        PlaceKind::City => 2,
        PlaceKind::Strait | PlaceKind::Bay if line => 3,
        PlaceKind::Island => 4,
        PlaceKind::Strait | PlaceKind::Bay => 5,
        PlaceKind::Town => 6,
        PlaceKind::Village => 7,
        PlaceKind::Lagoon => 8,
        PlaceKind::Hamlet => 9,
        PlaceKind::Suburb => 10,
    }
}

/// An island's name by how long its land is on screen, or by the tiles' rank where that is not known.
fn island_size(name: &str, on_screen: Option<f64>, rank: u32) -> f64 {
    match on_screen {
        // As along the island: a share of its length, so that the name keeps its size on the island from one
        // zoom to the next, and the step to setting it along the island is not seen.
        Some(l) => {
            let least = (super::regions::ISLAND_MIN_PX / (super::text_scale() as f64 / 100.0)).max(ISLAND_LEVEL_MIN_PT);
            super::regions::island_pt(name, l).clamp(least, super::regions::ISLAND_MAX_PT)
        }
        None => match rank {
            0..=2 => 12.5,
            3..=4 => 11.5,
            _ => 10.5,
        },
    }
}

/// A cape this close to an island's box (pixels at the label zoom) is on the island: the cape's node is on
/// the coast, which the tiles draw a little off it.
const CAPE_ON_ISLAND_PX: f64 = 4.0;
/// Islands of this rank or bigger are named without asking whether their point is near the shore.
const BIG_ISLAND_RANK: u32 = 3;
/// The smallest an island's level name is written, in points, and in pixels as along an island.
const ISLAND_LEVEL_MIN_PT: f64 = 6.0;
/// An island smaller than this on screen, its longer side in pixels, is not named.
const ISLAND_MIN_PX: f64 = 16.0;
/// Where along an island's centreline its name is tried, as shares of its length from the middle.
const RIDGE_AT: [f64; 5] = [0.0, -0.15, 0.15, -0.3, 0.3];
/// How much of a name written on an island must be on its land.
const ON_ISLAND_SHARE: f64 = 0.5;
/// How far a small island's name stands off its land, in pixels.
const ISLAND_GAP_PX: f64 = 3.0;

/// How a kind of place is written: the bigger the place, the bigger its
/// name; waters in sloping blue, land in upright black.
fn style(kind: PlaceKind) -> Style {
    use pango::Weight as W;
    match kind {
        PlaceKind::Sea => Style::water(16.0),
        PlaceKind::City => Style::land(15.0, W::Bold, 3.0),
        PlaceKind::Town => Style::land(13.5, W::Semibold, 2.5),
        PlaceKind::Strait | PlaceKind::Bay => Style::water(12.5),
        PlaceKind::Island => Style::land(12.5, W::Normal, 0.0).capitals(),
        PlaceKind::Village => Style::land(12.0, W::Normal, 2.0),
        PlaceKind::Lagoon => Style::water(11.5),
        PlaceKind::Hamlet => Style::land(11.0, W::Normal, 0.0),
        PlaceKind::Suburb => Style::land(10.5, W::Normal, 0.0),
    }
}

/// View zoom past which the names of seas are not written: you are in one.
const MAX_ZOOM_SEA: f64 = 11.0;

/// A water's line reaches into several tiles and each names it at its own
/// piece, and a town often bears its island's name: the same name this close
/// to one already written is left out.
const SAME_NAME_PX: f64 = 400.0;

/// The shore as segments in screen pixels, bucketed so that asking whether
/// the sea is within a radius looks at a handful of them.
struct Shore {
    reach: f64,
    cell: f64,
    buckets: HashMap<(i32, i32), Vec<[f64; 4]>>,
    /// How many times bigger the view asked about is than the one the shore was made for.
    scale: f64,
}

fn distance_to_segment(p: (f64, f64), s: &[f64; 4]) -> f64 {
    let (dx, dy) = (s[2] - s[0], s[3] - s[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (((p.0 - s[0]) * dx + (p.1 - s[1]) * dy) / len2).clamp(0.0, 1.0) };
    (p.0 - (s[0] + t * dx)).hypot(p.1 - (s[1] + t * dy))
}

impl Shore {
    /// A shore that answers for points within `reach` pixels of it.
    fn new(reach: f64) -> Shore {
        Shore { reach, cell: reach.max(16.0), buckets: HashMap::new(), scale: 1.0 }
    }

    fn add(&mut self, a: (f64, f64), b: (f64, f64)) {
        // File the segment under every cell it passes through; a point within
        // `reach` of it is then in one of those or next to one.
        let steps = ((b.0 - a.0).hypot(b.1 - a.1) / (self.cell / 2.0)).ceil().max(1.0);
        if steps > 20_000.0 {
            return;
        }
        let mut last = None;
        for i in 0..=steps as u32 {
            let t = i as f64 / steps;
            let c = (((a.0 + (b.0 - a.0) * t) / self.cell).floor() as i32, ((a.1 + (b.1 - a.1) * t) / self.cell).floor() as i32);
            if last != Some(c) {
                self.buckets.entry(c).or_default().push([a.0, a.1, b.0, b.1]);
                last = Some(c);
            }
        }
    }

    fn near(&self, p: (f64, f64)) -> bool {
        let p = (p.0 / self.scale, p.1 / self.scale);
        let (cx, cy) = ((p.0 / self.cell).floor() as i32, (p.1 / self.cell).floor() as i32);
        (cx - 1..=cx + 1).any(|x| (cy - 1..=cy + 1).any(|y| self.buckets.get(&(x, y)).is_some_and(|v| v.iter().any(|s| distance_to_segment(p, s) <= self.reach))))
    }
}

/// How many pixels `COAST_M` is at the view.
fn coast_reach(vi: &ViewInfo) -> f64 {
    let metres_per_px = 40_075_016.7 * vi.view.center.lat.to_radians().cos().max(0.05) / crate::core::geo::world_size(vi.zoom());
    (COAST_M / metres_per_px).max(COAST_MIN_PX)
}

/// The rings of `rings` worth drawing: not specks, and (with a `shore`) near the sea.
fn keep_rings<'a>(rings: &'a [Vec<[i16; 2]>], at: &dyn Fn([i16; 2]) -> (f64, f64), view: [f64; 4], min_px: f64, shore: Option<&Shore>) -> Vec<&'a Vec<[i16; 2]>> {
    rings
        .iter()
        .filter(|ring| {
            let pts = || ring.iter().map(|p| at(*p));
            let (mut lo, mut hi) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
            for (x, y) in pts() {
                lo = (lo.0.min(x), lo.1.min(y));
                hi = (hi.0.max(x), hi.1.max(y));
            }
            // Zoomed in past the downloads' zoom, a downloaded tile is many of the chart's.
            let shown = hi.0 >= view[0] && lo.0 <= view[2] && hi.1 >= view[1] && lo.1 <= view[3];
            shown && (hi.0 - lo.0).max(hi.1 - lo.1) >= min_px && shore.is_none_or(|s| pts().any(|p| s.near(p)))
        })
        .collect()
}

/// Traces the rings of a tile, each from tile units to the screen.
fn trace(cr: &cairo::Context, rings: &[&Vec<[i16; 2]>], at: &dyn Fn([i16; 2]) -> (f64, f64)) {
    for ring in rings {
        for (n, p) in ring.iter().enumerate() {
            let (x, y) = at(*p);
            if n == 0 {
                cr.move_to(x, y);
            } else {
                cr.line_to(x, y);
            }
        }
        cr.close_path();
    }
}

/// Traces the outlines of a tile without the cuts along the edge of its buffer.
fn trace_outline(cr: &cairo::Context, tile: &PlacesTile, rings: &[&Vec<[i16; 2]>], at: &dyn Fn([i16; 2]) -> (f64, f64)) {
    for ring in rings {
        let n = ring.len();
        let mut pen_down = false;
        for i in 0..n {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            if tile.on_edge(a, b) {
                pen_down = false;
                continue;
            }
            if !pen_down {
                let (x, y) = at(a);
                cr.move_to(x, y);
            }
            let (x, y) = at(b);
            cr.line_to(x, y);
            pen_down = true;
        }
    }
}

/// Fills `rings` (non-zero rule: the tiles wind holes the other way) and
/// draws their outline.
fn draw_areas(cr: &cairo::Context, pal: &Palette, tile: &PlacesTile, rings: &[&Vec<[i16; 2]>], at: &dyn Fn([i16; 2]) -> (f64, f64), colours: (Paint, Paint), width: f64) {
    if rings.is_empty() {
        return;
    }
    cr.set_fill_rule(cairo::FillRule::Winding);
    cr.set_line_join(cairo::LineJoin::Round);
    cr.new_path();
    trace(cr, rings, at);
    pal.set(cr, colours.0);
    let _ = cr.fill();
    cr.new_path();
    trace_outline(cr, tile, rings, at);
    pal.set(cr, colours.1);
    cr.set_line_width(width);
    let _ = cr.stroke();
}

/// A name ready to place: where it is on the screen and what it is.
struct Label<'a> {
    place: &'a Place,
    x: f64,
    y: f64,
    /// An island's land on screen, `[x0, y0, x1, y1]`, when the tiles show the sea all round it.
    land: Option<[f64; 4]>,
    /// Points along the island's centreline on screen, its middle first.
    ridge: Vec<(f64, f64)>,
    /// How big what it names is on screen, for a water: the bigger is placed first.
    big: f64,
    /// For a water named along a line, where else its level name is tried, in turn, when it does not fit at (`x`, `y`).
    spots: Vec<(f64, f64)>,
    /// For a water named at a point, the size of its name, by how wide it is (`waters::point_em`).
    em: Option<f64>,
}

fn inside(vi: &ViewInfo, areas: &[TileId], x: f64, y: f64) -> bool {
    areas.iter().any(|t| {
        let (ox, oy, size) = tile_origin(vi, t);
        x >= ox && x < ox + size && y >= oy && y < oy + size
    })
}

impl Layer for TownLayer {
    fn id(&self) -> &'static str {
        "towns"
    }

    fn revision(&self) -> u64 {
        self.follow_aids();
        self.changes.get()
    }

    fn changes_since(&self, revision: u64) -> Option<Vec<super::Change>> {
        self.changes.since(revision)
    }

    fn prepare(&self, area: &TileArea) -> Option<Arc<dyn TilePaint>> {
        let paint = self.paint_for(area)?;
        let relief = self.relief.borrow().as_ref().and_then(|r| r.prepare(area));
        Some(match relief {
            Some(relief) => Arc::new(WithRelief { paint, relief }),
            None => paint,
        })
    }
}

impl TownLayer {
    /// What the tiles of `area` are drawn from, made once for the area: the
    /// soundings ask for it too, to keep clear of the names.
    pub fn paint_for(&self, area: &TileArea) -> Option<Arc<TownPaint>> {
        self.follow_aids();
        let b = area.vi.bounds();
        let key = ([b.min_lon, b.min_lat, b.max_lon, b.max_lat, area.vi.zoom()].map(f64::to_bits), self.changes.get() - self.relief_bumps.get(), self.online.get());
        if let Some((k, paint)) = self.last.borrow().as_ref()
            && *k == key
        {
            return paint.clone();
        }
        let paint = self.make_paint(area);
        self.credit.towns.set(paint.as_ref().is_some_and(|p| !p.shown.is_empty()));
        self.credit.natural_earth.set(paint.as_ref().is_some_and(|p| !p.paths.is_empty()));
        *self.last.borrow_mut() = Some((key, paint.clone()));
        paint
    }

    fn make_paint(&self, area: &TileArea) -> Option<Arc<TownPaint>> {
        if !self.online.get() {
            return None;
        }
        let wanted = tiles_for(&area.vi.bounds(), TileId::zoom_for(area.vi.zoom()), MAX_TILES);
        let resolved = {
            let tiles = self.tiles.borrow();
            resolve(&wanted, |t| tiles.contains(t))
        };
        let mut shown: Vec<Shown> = Vec::new();
        for (src, areas) in resolved {
            if let Some(tile) = self.tiles.borrow_mut().get(&src) {
                shown.push((src, areas, tile));
            }
        }
        // Without tiles the names from Natural Earth are still written.
        let aids = self.aids.borrow().as_ref().and_then(|a| a.paint_for(area));
        // No name is written under a mark or a light: one along a path that would be is left out (a level
        // name, which can go round them, may name the place instead).
        // The names along paths are placed as at the step's zoom (`names::label_zoom`), keeping clear of the marks
        // as they are there, where they are biggest against the names, and scaled with the chart up to the next
        // step: what is clear at the step stays clear, and nothing is placed again in between. The marks are
        // worked out at the step's zoom itself, so that every zoom of the step sees them to the pixel the same.
        let zoom = area.vi.view.zoom;
        let band = names::label_zoom(zoom);
        let band_marks: Vec<[f64; 4]> = aids.as_ref().map_or_else(Vec::new, |a| {
            let (at_step, _) = names::at_label_zoom(&area.vi);
            let (cx, cy) = at_step.view.center_px();
            let (ox, oy) = (cx - at_step.w / 2.0, cy - at_step.h / 2.0);
            a.boxes(&at_step, PLAN_REACH_PX).into_iter().map(|b| [b[0] + ox, b[1] + oy, b[2] + ox, b[3] + oy]).collect()
        });
        let on_band_mark = |b: &[f64; 4]| band_marks.iter().any(|m| crosses(b, m, band));
        let tiles: Vec<(TileId, Arc<PlacesTile>)> = shown.iter().map(|(src, _, tile)| (*src, tile.clone())).collect();
        // Each as fitted, and as drawn here.
        let ne: Vec<(Arc<PathLabel>, Arc<PathLabel>)> = self.regions.labels_avoiding(area, &on_band_mark).into_iter().map(|l| (Arc::new(l.at_zoom(zoom)), l)).collect();
        let ne_fitted: Vec<Arc<PathLabel>> = ne.iter().map(|(_, l)| l.clone()).collect();
        let mut paths: Vec<Arc<PathLabel>> = ne.into_iter().map(|(here, _)| here).collect();
        if shown.is_empty() && paths.is_empty() {
            return None;
        }
        let mut dedupe: HashMap<String, Vec<[f64; 4]>> = HashMap::new();
        for p in &paths {
            for n in &p.aka {
                dedupe.entry(n.clone()).or_default().push(p.units);
            }
        }
        // A water named along a line that reaches past the view is one the view is inside: its name says nothing here.
        let mut reach: HashMap<&str, Bounds> = HashMap::new();
        for (src, _, tile) in &shown {
            for p in &tile.places {
                if let Some([x0, y0, x1, y1]) = p.span {
                    let b = reach.entry(&p.name).or_insert(Bounds::EMPTY);
                    for (x, y) in [(x0, y0), (x1, y1)] {
                        let at = src.point(x as f64, y as f64, tile.extent as f64);
                        b.extend([at.lon, at.lat]);
                    }
                }
            }
        }
        // A window of the view's size at the step's zoom, as the other names judge it, so that what is named does
        // not change between steps.
        let view = ViewInfo { view: crate::view::View { center: area.view.view.center, zoom: names::label_zoom(area.view.zoom()) }, ..area.view }.bounds();
        let too_big = reach.iter().filter(|(_, b)| b.width() > view.width() || b.height() > view.height()).map(|(n, _)| n.to_string()).collect();
        let extents = reach.iter().map(|(n, b)| (n.to_string(), b.width().max(b.height()))).collect();
        let extras = self.extras(area, &shown);
        // Waters and islands whose spine the tiles allow are set along it, after the Natural Earth names, which win.
        let z = TileId::zoom_for(area.vi.zoom());
        let extra_cands = extras.iter().map(|(p, at)| Candidate { name: p.name.clone(), kind: p.kind, rank: p.rank, lines: vec![], points: vec![*at], z }).collect();
        // Every tile loaded, for the shapes of waters that reach past the view; the tiles they still need are fetched.
        let pool: Vec<(TileId, Arc<PlacesTile>)> = self.tiles.borrow().iter().map(|(id, t)| (*id, t.clone())).collect();
        let before = self.waters.wanted();
        let written = self.waters.labels_for(area, &tiles, &pool, extra_cands, &too_big, &ne_fitted, &on_band_mark, &self.changes, &self.redraw);
        let wanted = self.waters.wanted();
        if wanted.iter().any(|t| !before.contains(t))
            && let Some(f) = self.on_wanted.borrow().as_ref()
        {
            f();
        }
        // Capes after the waters, out of the way of their names (a strait's runs the length of the strait); one
        // Natural Earth names too is written as a cape. A cape with a major light on it is named from where its
        // light shows.
        let lights = aids.as_ref().map_or_else(Vec::new, |a| a.major_lights(&area.vi.bounds().expanded(60.0 * area.vi.deg_per_px())));
        // The size of the name of the island a point is on, at the label zoom: the tiles' island whose land holds
        // it (level from its box, or along its centreline), and Natural Earth's as fitted; the smallest.
        let ppu = 2f64.powf(band);
        let island_pt = |p: Pt| -> Option<f64> {
            let by = CAPE_ON_ISLAND_PX / ppu;
            let holds = |b: &[f64; 4]| p[0] >= b[0] - by && p[0] <= b[2] + by && p[1] >= b[1] - by && p[1] <= b[3] + by;
            let tiles = written.islands.iter().filter_map(|(name, _, land, spine)| {
                let b = land.as_ref().filter(|b| holds(b))?;
                let level = island_size(name, Some((b[2] - b[0]).max(b[3] - b[1]) * ppu), 0);
                Some(spine.as_ref().map_or(level, |s| level.min(super::regions::island_pt(name, s.length() * ppu))))
            });
            let ne = ne_fitted.iter().filter(|l| holds(&l.units) && crate::core::chart::naturalearth::all().get((l.id >> 8) as usize).is_some_and(|g| g.kind == crate::core::chart::naturalearth::GeoKind::Island)).map(|l| l.style.size);
            tiles.chain(ne).min_by(f64::total_cmp)
        };
        let capes = self.capes.labels_for(area, &tiles, &lights, &ne_fitted, &written.labels, &island_pt, &on_band_mark, &|t, st| self.regions.measure(t, st));
        paths.retain(|p| !capes.replaced.contains(&p.id));
        paths.extend(capes.labels.iter().map(|l| Arc::new(l.at_zoom(zoom))));
        let along_kept: Vec<Arc<PathLabel>> = written.labels.into_iter().map(|l| Arc::new(l.at_zoom(zoom))).collect();
        let along = written.names.into_iter().filter(|n| along_kept.iter().any(|l| l.aka.contains(&n.trim().to_lowercase()))).collect();
        paths.extend(along_kept);
        Some(Arc::new(TownPaint { id: NEXT_PAINT.fetch_add(1, Ordering::Relaxed), shown, too_big, extents, paths, dedupe, along, unsettled: written.unsettled, extras, islands: written.islands, spots: written.spots, sizes: written.sizes, aids }))
    }

    /// The islands named in coarser tiles near the area that the shown tiles do not name, as places with
    /// where they are in units. Each is placed as a tile's own would be.
    fn extras(&self, area: &TileArea, shown: &[Shown]) -> Vec<(Place, Pt)> {
        let b = crate::core::chart::naturalearth::units_of(&area.vi.bounds().expanded(area.vi.deg_per_px() * 200.0));
        let mut own: HashMap<&str, Vec<Pt>> = HashMap::new();
        for (src, _, tile) in shown {
            for p in tile.places.iter().filter(|p| p.kind == PlaceKind::Island) {
                own.entry(&p.name).or_default().push(unit_of(src, tile.extent.max(1) as f64, [p.at[0] as f64, p.at[1] as f64]));
            }
        }
        let mut out = Vec::new();
        for (name, all) in self.seen.borrow().iter() {
            for s in all {
                let inside = s.at[0] >= b[0] && s.at[0] <= b[2] && s.at[1] >= b[1] && s.at[1] <= b[3];
                let known = own.get(name.as_str()).is_some_and(|v| v.iter().any(|q| (q[0] - s.at[0]).hypot(q[1] - s.at[1]) < 4.0 * SAME_ISLAND_UNITS));
                if inside && !known {
                    out.push((Place { name: name.clone(), kind: PlaceKind::Island, rank: s.rank, at: [0, 0], span: None, line: None }, s.at));
                }
            }
        }
        out.sort_by(|a, b| (a.0.rank, &a.0.name).cmp(&(b.0.rank, &b.0.name)));
        out
    }
}

/// A tile that is drawn, the wanted tiles it stands for, and its data.
type Shown = (TileId, Vec<TileId>, Arc<PlacesTile>);

pub struct TownPaint {
    /// Tells this snapshot's plans from another's.
    id: u64,
    shown: Vec<Shown>,
    /// Waters too big for the view to name.
    too_big: HashSet<String>,
    /// How far each water named along a line reaches, in degrees.
    extents: HashMap<String, f64>,
    /// The names set along their features (Natural Earth), in world pixels at the zoom of the area.
    paths: Vec<Arc<PathLabel>>,
    /// What those names are of, by every name they have: an OSM place of the same name inside the same
    /// feature is the same thing, written once. The features' boxes, in units.
    dedupe: HashMap<String, Vec<[f64; 4]>>,
    /// The names of waters and islands set along their own water: not written level as well.
    along: HashSet<String>,
    /// The waters and islands whose shape is still being worked out: not written level meanwhile.
    unsettled: HashSet<String>,
    /// Islands coarser tiles named that the shown ones do not, with where they are (units).
    extras: Vec<(Place, Pt)>,
    /// The islands whose land the tiles show: name, where it is named, the land's box (units) or `None`
    /// where the sea does not go round it.
    islands: Vec<Island>,
    /// Where the waters named along a line are written level: name, the line's middle and the spot (units).
    spots: Vec<(String, Pt, Vec<Pt>)>,
    /// How wide the waters named at a point are: name, where it is named and the width (units).
    sizes: Vec<(String, Pt, f64, Option<f64>)>,
    /// The marks and lights of the area, which names keep clear of.
    aids: Option<Arc<AidsPaint>>,
}

static NEXT_PAINT: AtomicU64 = AtomicU64::new(0);

/// What a tile shows of the towns layer: the shore, and the names placed.
struct Plan {
    shore: Option<Shore>,
    placed: Vec<Placed>,
    /// The boxes of the letters of the names set along paths that reach the tile.
    path_boxes: Vec<[f64; 4]>,
}

thread_local! {
    /// The last tile's plan on this thread: the soundings make it, then the names draw it.
    static PLAN: RefCell<Option<((u64, [u64; 6]), Rc<Plan>)>> = const { RefCell::new(None) };
}

impl TownPaint {
    /// The tiles within `margin` pixels of `vi`.
    fn near<'a>(&'a self, vi: &ViewInfo, margin: f64) -> Vec<&'a Shown> {
        self.shown.iter().filter(|(src, _, _)| near_view(vi, src, margin)).collect()
    }

    /// The boxes of the names in the tile `vi`, which soundings keep clear of.
    pub fn name_boxes(&self, cr: &cairo::Context, vi: &ViewInfo, pal: &Palette) -> Vec<[f64; 4]> {
        let plan = self.plan(cr, vi, pal);
        plan.placed.iter().map(Placed::rect).chain(plan.path_boxes.iter().copied()).collect()
    }

    /// The names of the tile `vi`: placed as at its `names::label_zoom`, the same at every zoom up to the next
    /// step, and grown with the chart to the tile's zoom.
    fn plan(&self, cr: &cairo::Context, vi: &ViewInfo, pal: &Palette) -> Rc<Plan> {
        let b = vi.bounds();
        let key = (self.id, [b.min_lon, b.min_lat, b.max_lon, b.max_lat, vi.zoom(), cr.target().device_scale().0].map(f64::to_bits));
        if let Some(plan) = PLAN.with(|p| p.borrow().as_ref().filter(|(k, _)| *k == key).map(|(_, plan)| plan.clone())) {
            return plan;
        }
        let (at_step, k) = names::at_label_zoom(vi);
        let step = self.make_plan(cr, &at_step, pal);
        let shore = step.shore.map(|s| Shore { scale: k, ..s });
        let placed = step.placed.iter().filter_map(|p| p.scaled(cr, pal, k)).collect();
        let path_boxes = step.path_boxes.iter().map(|b| b.map(|v| v * k)).collect();
        let plan = Rc::new(Plan { shore, placed, path_boxes });
        PLAN.with(|p| *p.borrow_mut() = Some((key, plan.clone())));
        plan
    }

    /// Whether the place `name` at the screen point (`x`, `y`) is what a name set along a path already names.
    fn is_named_along_a_path(&self, vi: &ViewInfo, name: &str, x: f64, y: f64) -> bool {
        let Some(boxes) = self.dedupe.get(&name.trim().to_lowercase()) else { return false };
        let at = vi.position_at(x, y);
        let (ux, uy) = crate::core::geo::project(&at, 0.0);
        // A cape's feature is a point: stand off it by the pixels its name does.
        let slop = 40.0 / 2f64.powf(vi.zoom());
        boxes.iter().any(|b| ux >= b[0] - slop && ux <= b[2] + slop && uy >= b[1] - slop && uy <= b[3] + slop)
    }

    /// The boxes of the path names' letters that reach the tile `vi`, on its screen, at its zoom.
    fn path_boxes(&self, vi: &ViewInfo) -> Vec<[f64; 4]> {
        let view = [-PLAN_REACH_PX - 100.0, -PLAN_REACH_PX - 100.0, vi.w + PLAN_REACH_PX + 100.0, vi.h + PLAN_REACH_PX + 100.0];
        self.paths
            .iter()
            .flat_map(|p| {
                let k = 2f64.powf(vi.zoom() - p.zoom);
                p.boxes.iter().map(move |b| b.map(|v| v * k))
            })
            .map(|b| {
                let (x, y) = vi.world_to_screen(b[0], b[1]);
                [x, y, x + b[2] - b[0], y + b[3] - b[1]]
            })
            .filter(|b| b[0] < view[2] && b[2] > view[0] && b[1] < view[3] && b[3] > view[1])
            .collect()
    }

    /// What the tiles say of the island `name` named at the screen point (`x`, `y`): `None` when not known
    /// yet, else the box of its land (units, `None` for land the sea does not go round) and its centreline.
    fn island_at(&self, vi: &ViewInfo, name: &str, x: f64, y: f64) -> Option<(Option<[f64; 4]>, Option<Arc<crate::core::labels::Spine>>)> {
        let (ux, uy) = crate::core::geo::project(&vi.position_at(x, y), 0.0);
        self.islands
            .iter()
            .filter(|(n, at, _, _)| n == name && (at[0] - ux).hypot(at[1] - uy) < 4.0 * SAME_ISLAND_UNITS)
            .min_by(|a, b| (a.1[0] - ux).hypot(a.1[1] - uy).total_cmp(&(b.1[0] - ux).hypot(b.1[1] - uy)))
            .map(|(_, _, land, spine)| (*land, spine.clone()))
    }

    /// How wide on screen, at the zoom names are fitted at, the water `name` named at the screen point (`x`, `y`)
    /// is, if it was measured.
    fn width_at(&self, vi: &ViewInfo, name: &str, x: f64, y: f64) -> Option<(f64, Option<f64>)> {
        let (ux, uy) = crate::core::geo::project(&vi.position_at(x, y), 0.0);
        let ppu = 2f64.powf(names::label_zoom(vi.zoom()));
        self.sizes
            .iter()
            .filter(|(n, at, ..)| n == name && (at[0] - ux).hypot(at[1] - uy) < 4.0 * SAME_ISLAND_UNITS)
            .min_by(|a, b| (a.1[0] - ux).hypot(a.1[1] - uy).total_cmp(&(b.1[0] - ux).hypot(b.1[1] - uy)))
            .map(|(_, _, size, width)| (size * ppu, width.map(|w| w * ppu)))
    }

    /// Where the water `name` named along a line is tried level, in turn, if the tiles told (units).
    fn spots_of(&self, vi: &ViewInfo, name: &str) -> Vec<Pt> {
        let (cx, cy) = crate::core::geo::project(&vi.view.center, 0.0);
        self.spots.iter().filter(|(n, _, _)| n == name).min_by(|a, b| (a.1[0] - cx).hypot(a.1[1] - cy).total_cmp(&(b.1[0] - cx).hypot(b.1[1] - cy))).map_or_else(Vec::new, |(_, _, s)| s.clone())
    }

    fn make_plan<'a>(&'a self, cr: &cairo::Context, vi: &ViewInfo, pal: &Palette) -> Plan {
        let zoom = vi.zoom();
        let reach = coast_reach(vi);
        // Only the tiles that reach into the view, and for the shore, within the coast's reach of it.
        let shown = self.near(vi, PLAN_REACH_PX.max(reach));

        // Far out only the coast is wanted: the shore of every tile in view tells which is.
        let shore = (zoom < COASTAL_BELOW_ZOOM).then(|| {
            let mut shore = Shore::new(reach);
            for (src, _, tile) in &shown {
                let (ox, oy, size) = tile_origin(vi, src);
                let k = size / tile.extent as f64;
                let at = |p: [i16; 2]| (ox + p[0] as f64 * k, oy + p[1] as f64 * k);
                for ring in &tile.sea {
                    let n = ring.len();
                    for i in 0..n {
                        let (a, b) = (ring[i], ring[(i + 1) % n]);
                        if !tile.on_edge(a, b) {
                            shore.add(at(a), at(b));
                        }
                    }
                }
            }
            shore
        });

        // Names, the biggest places first so they win a crowded spot.
        let mut labels: Vec<Label> = Vec::new();
        // Names well past the tile are weighed too, so that a tile and its neighbour, each placing the names
        // that reach them, agree on the ones on their edge.
        let in_view = |x: f64, y: f64| x > -PLAN_REACH_PX && x < vi.w + PLAN_REACH_PX && y > -PLAN_REACH_PX / 2.0 && y < vi.h + PLAN_REACH_PX / 2.0;
        let mut consider = |place: &'a Place, x: f64, y: f64| {
            // A bay, cove, pass or lagoon named at a point is named by its size, once it is measured: the tiles name
            // every cove.
            let point = place.line.is_none() && has_axis(place.kind);
            let measured = if point { self.width_at(vi, &place.name, x, y) } else { None };
            let (width, em) = (measured.map(|m| m.0), measured.and_then(|m| m.1).map(point_em));
            if point && width.is_none_or(|w| w < MIN_POINT_WATER_PX) {
                return;
            }
            if !point && zoom < min_zoom(place.kind, place.rank, place.line.is_some()) - if place.kind == PlaceKind::Island { 1.0 } else { 0.0 } {
                return;
            }
            if (place.kind == PlaceKind::Sea && zoom > MAX_ZOOM_SEA) || (place.span.is_some() && self.too_big.contains(&place.name)) {
                return;
            }
            if place.kind.is_water_or_island() && (self.along.contains(&place.name) || self.unsettled.contains(&place.name)) {
                return;
            }
            if !in_view(x, y) || self.is_named_along_a_path(vi, &place.name, x, y) {
                return;
            }
            let near_shore = || shore.as_ref().is_none_or(|s| s.near((x, y)));
            let mut land = None;
            let mut ridge = Vec::new();
            let shows = match place.kind {
                // An island is named when it is big enough on screen, and below `COASTAL_BELOW_ZOOM` only one
                // the sea goes round (not a river's islet). Where its land is not known yet, by its rank.
                PlaceKind::Island => match self.island_at(vi, &place.name, x, y) {
                    Some((Some(b), spine)) => {
                        let (ax, ay) = vi.world_to_screen(b[0] * vi.ppu(), b[1] * vi.ppu());
                        let r = [ax, ay, ax + (b[2] - b[0]) * vi.ppu(), ay + (b[3] - b[1]) * vi.ppu()];
                        land = Some(r);
                        // Along its centreline, the middle first: where a level name is on the island, the same at every zoom.
                        if let Some(s) = spine {
                            ridge = RIDGE_AT.iter().map(|k| s.at(s.length() * (0.5 + k)).0).map(|p| vi.world_to_screen(p[0] * vi.ppu(), p[1] * vi.ppu())).collect();
                        }
                        (r[2] - r[0]).max(r[3] - r[1]) >= ISLAND_MIN_PX
                    }
                    Some((None, _)) => zoom >= COASTAL_BELOW_ZOOM,
                    // A big island's point is far from its shore (Mallorca's), and no river has one.
                    None => zoom >= island_zoom(place.rank) && (place.rank <= BIG_ISLAND_RANK || near_shore()),
                },
                // The sea's own names need no shore; a lake does, to be a lagoon.
                PlaceKind::Sea | PlaceKind::Strait | PlaceKind::Bay => true,
                _ => near_shore(),
            };
            // A big water's level name goes to its roomiest water rather than the middle of its line, which
            // is often by a shore; every tile's piece of the line then names it at the same spot.
            let mut spots: Vec<(f64, f64)> = if place.line.is_some() { self.spots_of(vi, &place.name).iter().map(|p| vi.world_to_screen(p[0] * vi.ppu(), p[1] * vi.ppu())).collect() } else { Vec::new() };
            let (x, y) = if spots.is_empty() { (x, y) } else { spots.remove(0) };
            if shows {
                let big = width.or_else(|| place.span.and_then(|_| self.extents.get(&place.name)).map(|e| e / vi.deg_per_px())).unwrap_or(0.0);
                labels.push(Label { place, x, y, land, ridge, big, spots, em });
            }
        };
        for (src, areas, tile) in &shown {
            let (ox, oy, size) = tile_origin(vi, src);
            let k = size / tile.extent as f64;
            for place in &tile.places {
                let (x, y) = (ox + place.at[0] as f64 * k, oy + place.at[1] as f64 * k);
                if inside(vi, areas, x, y) {
                    consider(place, x, y);
                }
            }
        }
        for (place, at) in &self.extras {
            let (x, y) = vi.world_to_screen(at[0] * vi.ppu(), at[1] * vi.ppu());
            consider(place, x, y);
        }
        let key = |l: &Label| (priority(l.place.kind, l.place.rank, l.place.line.is_some()), std::cmp::Reverse(l.big as i64), l.place.rank, l.place.name.clone());
        labels.sort_by_cached_key(key);
        // The letters of the names along paths come first: the points go round them.
        let path_boxes = self.path_boxes(vi);
        let mut placer = LabelPlacer::default();
        placer.reserve(path_boxes.iter().copied());
        if let Some(aids) = &self.aids {
            placer.reserve(aids.boxes(vi, PLAN_REACH_PX));
        }
        let mut written: Vec<(&str, f64, f64)> = Vec::new();
        let mut placed = Vec::new();
        let scale = cr.target().device_scale().0;
        // Every tile of the area, not only those near this one: a name's ends may reach past them, and a
        // neighbour that knew more would place it otherwise.
        let all: Vec<&Shown> = self.shown.iter().collect();
        let sea = |x: f64, y: f64| sea_at(&all, vi, x, y);
        let lake = |x: f64, y: f64| lake_at(&all, vi, x, y);
        for l in &labels {
            let kind = l.place.kind;
            // A water's line names it in every tile it crosses, and an island and its town often share a name.
            if written.iter().any(|&(n, x, y)| n == l.place.name && (x - l.x).hypot(y - l.y) < SAME_NAME_PX) {
                continue;
            }
            let mut st = style(kind);
            if let Some(em) = l.em {
                st.size = em;
            }
            if kind == PlaceKind::Island {
                st.size = island_size(&l.place.name, l.land.map(|r| (r[2] - r[0]).max(r[3] - r[1])), l.place.rank);
            }
            // A water named along a line is written as big as it is.
            if let Some(e) = l.place.span.and_then(|_| self.extents.get(&l.place.name)) {
                st.size = st.size.max(names::water_size(e / vi.deg_per_px()));
            }
            // An island is named in capitals, level or along it.
            let text = if st.caps { l.place.name.to_uppercase() } else { l.place.name.clone() };
            let fitted = match l.land {
                Some(r) => beside(cr, pal, &mut placer, &text, st, r, &l.ridge, &sea),
                None => {
                    // A sea's name is on the sea, over its islets at most: moved a little or written smaller if it
                    // would cross the coast, else left out. The tiles call many coves lakes: a lake whose point is on the sea is one.
                    let on_sea = matches!(kind, PlaceKind::Sea | PlaceKind::Strait | PlaceKind::Bay) || (kind == PlaceKind::Lagoon && sea(l.x, l.y) == Some(true));
                    // A lagoon on a lake is named on its lake.
                    let on_lake = kind == PlaceKind::Lagoon && !on_sea && lake(l.x, l.y) == Some(true);
                    // A bay, cove, pass or lagoon named at a point is small: its level name is wholly on its water.
                    let dry = if l.place.line.is_some() || l.place.span.is_some() {
                        DRY_SHARE_BIG
                    } else if has_axis(kind) {
                        0.0
                    } else {
                        DRY_SHARE
                    };
                    // A town's name is on its land, or not written: towns matter least to a sailor.
                    let town = st.dot > 0.0;
                    let ok = |x: f64, top: f64, w: f64, h: f64| {
                        if town {
                            let (n, wet) = names::samples(x, top, w, h).fold((0, 0), |(n, d), (sx, sy)| (n + 1, d + usize::from(sea(sx, sy) == Some(true))));
                            return wet as f64 <= TOWN_WET_SHARE * n as f64;
                        }
                        if on_lake {
                            return on_water(x, top, w, h, dry, &lake);
                        }
                        !on_sea || on_water(x, top, w, h, dry, &sea)
                    };
                    // An island whose land is not known yet may move off its point a little, like a water's name.
                    let nudge = on_sea || on_lake || kind == PlaceKind::Island;
                    std::iter::once((l.x, l.y)).chain(l.spots.iter().copied()).find_map(|(x, y)| names::fit(cr, pal, &mut placer, &text, st, x, y, nudge, ok))
                }
            };
            let Some((mut name, fitted, x, top)) = fitted else { continue };
            st = fitted;
            let (w, h) = (name.w, name.h);
            written.push((&l.place.name, l.x, l.y));
            // A land name out on the water (an islet's) is outlined in the sea's colour.
            if !st.halo_is_sea() && names::mostly(x, top, w, h, |sx, sy| sea(sx, sy) == Some(true)) {
                st = st.on_sea();
                name = names::name(cr, &name.text, &st, pal, scale).unwrap_or(name);
            }
            let dot = (st.dot > 0.0 && l.land.is_none()).then_some((l.x, l.y, st.dot, st.fill));
            placed.push(Placed { name, style: st, x, top, dot });
        }
        Plan { shore, placed, path_boxes }
    }
}

/// Whether two boxes in world pixels overlap, either as they are or a world apart.
fn crosses(a: &[f64; 4], b: &[f64; 4], zoom: f64) -> bool {
    let world = crate::core::geo::world_size(zoom);
    [0.0, world, -world].iter().any(|dx| a[0] + dx < b[2] && a[2] + dx > b[0] && a[1] < b[3] && a[3] > b[1])
}

/// Whether a water's name, at (`x`, `top`) and `w` by `h`, is on the water: its middle on the sea and at
/// most `dry` of it over land, so that it may run over islets (a big water's over the islands of a gulf).
fn on_water(x: f64, top: f64, w: f64, h: f64, dry: f64, sea: &dyn Fn(f64, f64) -> Option<bool>) -> bool {
    if sea(x + w / 2.0, top + h / 2.0) == Some(false) {
        return false;
    }
    // None of it: tested finely enough to meet a spit or a breakwater a few pixels wide.
    if dry == 0.0 {
        let (nx, ny) = ((w / DRY_STEP_PX).ceil().max(1.0) as usize, (h / DRY_STEP_PX).ceil().max(1.0) as usize);
        return !(0..=nx).any(|i| (0..=ny).any(|j| sea(x + w * i as f64 / nx as f64, top + h * j as f64 / ny as f64) == Some(false)));
    }
    let (n, on_land) = names::samples(x, top, w, h).fold((0, 0), |(n, d), (sx, sy)| (n + 1, d + usize::from(sea(sx, sy) == Some(false))));
    on_land as f64 <= dry * n as f64
}

/// How far apart a name that may not touch land is tested, in pixels.
const DRY_STEP_PX: f64 = 4.0;

/// How much of a name may be over land: an islet's or a small water's, a little; a water named along a
/// line (a gulf, a big bay), more, over its islands.
const DRY_SHARE: f64 = 0.2;
/// How much of a town's name may be over the sea: none.
const TOWN_WET_SHARE: f64 = 0.0;
const DRY_SHARE_BIG: f64 = 0.35;

/// Fits an island's name to its land `r` (screen): on the island, on its centreline `ridge` (the middle
/// first), when the name is mostly on land there; else beside it on the sea, as a paper chart names an
/// islet. On two lines when that is what fits.
#[allow(clippy::too_many_arguments)]
fn beside(cr: &cairo::Context, pal: &Palette, placer: &mut LabelPlacer, text: &str, st: Style, r: [f64; 4], ridge: &[(f64, f64)], sea: &dyn Fn(f64, f64) -> Option<bool>) -> Option<(Rc<names::Name>, Style, f64, f64)> {
    let scale = cr.target().device_scale().0;
    let (cx, cy) = ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0);
    let mut texts = vec![text.to_string()];
    let wrapped = super::regions::wrap(text, names::WRAP_CHARS);
    if wrapped.len() > 1 {
        texts.push(wrapped.join("\n"));
    }
    // At its size, else a step smaller, on one line, else two.
    let sizes = [st, Style { size: (st.size * 0.85).max(10.0), ..st }];
    let names: Vec<(Rc<names::Name>, Style)> = sizes.iter().flat_map(|st| texts.iter().filter_map(move |t| Some((names::name(cr, t, st, pal, scale)?, *st)))).collect();
    let on_land = |x: f64, top: f64, w: f64, h: f64| {
        let (n, dry) = names::samples(x, top, w, h).fold((0, 0), |(n, d), (sx, sy)| (n + 1, d + usize::from(sea(sx, sy) != Some(true))));
        dry as f64 >= ON_ISLAND_SHARE * n as f64
    };
    // On the island.
    for (name, st) in &names {
        let (w, h) = (name.w, name.h);
        let centres: Vec<(f64, f64)> = if ridge.is_empty() { vec![(cx, cy)] } else { ridge.to_vec() };
        let at: Vec<(f64, f64)> = centres.iter().map(|(x, y)| (x - w / 2.0, y - h / 2.0)).collect();
        if let Some((x, top)) = names::try_spots(placer, name, &at, |x, top| on_land(x, top, w, h)) {
            return Some((name.clone(), *st, x, top));
        }
    }
    // Beside it: right, left, below, above, then at its corners.
    for (name, st) in &names {
        let (w, h) = (name.w, name.h);
        let g = ISLAND_GAP_PX;
        let at = [
            (r[2] + g, cy - h / 2.0),
            (r[0] - g - w, cy - h / 2.0),
            (cx - w / 2.0, r[3] + g / 2.0),
            (cx - w / 2.0, r[1] - g / 2.0 - h),
            (r[2] + g, r[1] - h),
            (r[2] + g, r[3]),
            (r[0] - g - w, r[1] - h),
            (r[0] - g - w, r[3]),
        ];
        if let Some((x, top)) = names::try_spots(placer, name, &at, |x, top| on_water(x, top, w, h, DRY_SHARE, sea)) {
            return Some((name.clone(), *st, x, top));
        }
    }
    None
}

/// Whether the screen point (`x`, `y`) is on the sea, from the tile that
/// stands for it; `None` where no tile knows (nothing loaded, or no sea in it).
fn sea_at(shown: &[&Shown], vi: &ViewInfo, x: f64, y: f64) -> Option<bool> {
    let (src, _, tile) = shown.iter().find(|(_, areas, _)| inside(vi, areas, x, y))?;
    if tile.sea.is_empty() {
        return None;
    }
    let (ox, oy, size) = tile_origin(vi, src);
    let k = tile.extent as f64 / size;
    Some(tile.on_sea((x - ox) * k, (y - oy) * k))
}

/// Whether the screen point (`x`, `y`) is on a lake, from the tile that stands for it.
fn lake_at(shown: &[&Shown], vi: &ViewInfo, x: f64, y: f64) -> Option<bool> {
    let (src, _, tile) = shown.iter().find(|(_, areas, _)| inside(vi, areas, x, y))?;
    let (ox, oy, size) = tile_origin(vi, src);
    let k = tile.extent as f64 / size;
    Some(tile.on_lake((x - ox) * k, (y - oy) * k))
}

/// The towns, with the relief shown through their built-up areas.
struct WithRelief {
    paint: Arc<TownPaint>,
    relief: Arc<ReliefPaint>,
}

impl TilePaint for WithRelief {
    fn paint(&self, ctx: &DrawCtx) {
        self.paint.paint_with(ctx, Some(&self.relief));
    }
}

impl TilePaint for TownPaint {
    fn paint(&self, ctx: &DrawCtx) {
        self.paint_with(ctx, None);
    }
}

impl TownPaint {
    fn paint_with(&self, ctx: &DrawCtx, relief: Option<&ReliefPaint>) {
        let (cr, vi, pal) = (ctx.cr, &ctx.vi, &ctx.pal);
        let zoom = vi.zoom();
        let shown = self.near(vi, 200.0f64.max(coast_reach(vi)));
        if shown.is_empty() && self.paths.is_empty() {
            return;
        }
        let plan = self.plan(cr, vi, pal);
        let shore = &plan.shore;

        // The areas are drawn in a group when there is relief: laid `Atop`, it reaches them and nothing else.
        let relief = relief.filter(|_| zoom >= MIN_ZOOM_URBAN);
        if relief.is_some() {
            cr.push_group();
        }
        let mut drew = false;
        for (urban, colours, width, min_zoom, min_px) in
            [(true, (TOWN.into(), TOWN_EDGE.into()), 0.8, MIN_ZOOM_URBAN, MIN_URBAN_PX), (false, (BUILDING.into(), BUILDING_EDGE.into()), 0.5, MIN_ZOOM_BUILDINGS, 0.0)]
        {
            if zoom < min_zoom {
                continue;
            }
            for (src, areas, tile) in &shown {
                if !near_view(vi, src, 8.0) {
                    continue;
                }
                let (ox, oy, size) = tile_origin(vi, src);
                let k = size / tile.extent as f64;
                let at = |p: [i16; 2]| (ox + p[0] as f64 * k, oy + p[1] as f64 * k);
                let _ = cr.save();
                // A coarser tile stands in only for the area it replaces.
                if areas.as_slice() != [*src] {
                    cr.new_path();
                    for t in areas {
                        let (x, y, s) = tile_origin(vi, t);
                        cr.rectangle(x, y, s, s);
                    }
                    cr.clip();
                }
                // Buildings are only drawn close in, where everything in view is wanted.
                let view = [-width, -width, vi.w + width, vi.h + width];
                let rings = keep_rings(if urban { &tile.urban } else { &tile.buildings }, &at, view, min_px, shore.as_ref().filter(|_| urban));
                drew |= !rings.is_empty();
                draw_areas(cr, pal, tile, &rings, &at, colours, width);
                let _ = cr.restore();
            }
        }
        if let Some(relief) = relief {
            if drew {
                let _ = cr.save();
                cr.set_operator(cairo::Operator::Atop);
                relief.paint(ctx, true);
                let _ = cr.restore();
            }
            let _ = cr.pop_group_to_source();
            let _ = cr.paint();
        }

        names::draw_paths(cr, pal, vi, &self.paths);
        names::draw(cr, pal, &plan.placed);
    }
}

/// Whether the tile `id` reaches within `margin` pixels of the view.
fn near_view(vi: &ViewInfo, id: &TileId, margin: f64) -> bool {
    let (x, y, size) = tile_origin(vi, id);
    x < vi.w + margin && x + size > -margin && y < vi.h + margin && y + size > -margin
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A view taking in everything.
    const ALL: [f64; 4] = [f64::MIN, f64::MIN, f64::MAX, f64::MAX];

    #[test]
    fn far_out_the_towns_wait_and_islands_win_over_them() {
        // No town or city is named at the zooms of oceans and countries.
        for kind in [PlaceKind::City, PlaceKind::Town, PlaceKind::Village] {
            assert!(min_zoom(kind, 0, false) >= 7.0, "{kind:?}");
        }
        assert!(min_zoom(PlaceKind::Town, 14, false) > min_zoom(PlaceKind::Town, 11, false), "small towns later");
        assert!(island_zoom(2) < island_zoom(5) && island_zoom(5) < island_zoom(6), "Ré before Aix before Fort Boyard");
        // An island before a town, a big island before a city, a pass between islands after the islands.
        assert!(priority(PlaceKind::Island, 6, false) < priority(PlaceKind::Town, 5, false));
        assert!(priority(PlaceKind::Island, 2, false) < priority(PlaceKind::City, 1, false));
        assert!(priority(PlaceKind::Strait, 1, false) > priority(PlaceKind::Island, 5, false));
        assert!(priority(PlaceKind::Bay, 0, true) < priority(PlaceKind::Island, 5, false), "a gulf before its islands");
        // An island's name is sized by the island: half as long again on screen, half as big again, between bounds.
        let (a, b) = (island_size("ILE D'HOUAT", Some(170.0), 5), island_size("ILE D'HOUAT", Some(255.0), 5));
        assert!((b / a - 1.5).abs() < 1e-9, "{a} then {b}");
        assert_eq!(island_size("HOUAT", Some(5.0), 5), super::super::regions::ISLAND_MIN_PX.max(ISLAND_LEVEL_MIN_PT));
        assert!(island_size("X", None, 5) < island_size("X", None, 2));
    }

    #[test]
    fn a_water_name_may_run_over_islets_but_not_be_on_land() {
        // Land is the band x in 40..60.
        let sea = |x: f64, _y: f64| Some(!(40.0..60.0).contains(&x));
        assert!(on_water(0.0, 0.0, 30.0, 10.0, DRY_SHARE, &sea), "all on the sea");
        assert!(on_water(-100.0, 0.0, 200.0, 10.0, DRY_SHARE, &sea), "an islet a tenth of it, off its middle");
        assert!(!on_water(30.0, 0.0, 30.0, 10.0, DRY_SHARE_BIG, &sea), "its middle on land");
        assert!(!on_water(0.0, 0.0, 70.0, 10.0, DRY_SHARE, &sea) && on_water(0.0, 0.0, 70.0, 10.0, DRY_SHARE_BIG, &sea), "a big water may cross more");
    }

    #[test]
    fn bigger_places_are_written_bigger() {
        assert!(style(PlaceKind::City).size > style(PlaceKind::Town).size);
        assert!(style(PlaceKind::Town).size > style(PlaceKind::Village).size);
        assert!(style(PlaceKind::Hamlet).dot == 0.0, "small names go without a dot");
    }

    fn segment_shore(reach: f64) -> Shore {
        let mut s = Shore::new(reach);
        s.add((0.0, 100.0), (500.0, 100.0));
        s
    }

    #[test]
    fn the_shore_answers_for_points_within_its_reach_only() {
        let s = segment_shore(30.0);
        assert!(s.near((250.0, 120.0)));
        assert!(s.near((250.0, 70.0)));
        assert!(!s.near((250.0, 60.0)));
        assert!(!s.near((250.0, 200.0)));
        assert!(s.near((520.0, 100.0)), "past the end of a segment, by the distance to its end");
        assert!(!s.near((540.0, 100.0)));
        assert!(!Shore::new(30.0).near((0.0, 0.0)), "no sea in view, no coast");
    }

    #[test]
    fn a_segment_longer_than_a_cell_is_found_along_its_whole_length() {
        let mut s = Shore::new(20.0);
        s.add((-5000.0, -5000.0), (5000.0, 5000.0));
        assert!(s.near((1234.0, 1240.0)));
        assert!(!s.near((1234.0, 1300.0)));
    }

    #[test]
    fn specks_and_inland_areas_are_not_kept() {
        let at = |p: [i16; 2]| (p[0] as f64, p[1] as f64);
        let square = |x: i16, y: i16, s: i16| vec![[x, y], [x + s, y], [x + s, y + s], [x, y + s]];
        let rings = vec![square(0, 80, 40), square(10, 90, 3), square(0, 400, 40)];
        assert_eq!(keep_rings(&rings, &at, ALL, 0.0, None).len(), 3);
        assert_eq!(keep_rings(&rings, &at, ALL, 5.0, None).len(), 2, "the speck goes");
        let shore = segment_shore(30.0);
        let kept = keep_rings(&rings, &at, ALL, 5.0, Some(&shore));
        assert_eq!(kept.len(), 1, "only the one by the sea");
        assert_eq!(kept[0][0], [0, 80]);
    }

    #[test]
    fn the_coast_is_five_kilometres_but_never_less_than_a_few_pixels() {
        let info = |lat: f64, zoom: f64| crate::view::ViewInfo { view: crate::view::View { center: crate::core::geo::LatLon::new(lat, -4.0), zoom }, w: 800.0, h: 600.0 };
        let near = coast_reach(&info(48.0, 10.0));
        assert!((40.0..60.0).contains(&near), "about 49 px at zoom 10, got {near}");
        assert!(coast_reach(&info(48.0, 11.0)) > near * 1.9);
        assert_eq!(coast_reach(&info(48.0, 3.0)), COAST_MIN_PX);
    }

    #[test]
    fn a_label_is_kept_only_inside_the_tiles_it_stands_for() {
        let vi = crate::view::ViewInfo { view: crate::view::View { center: crate::core::geo::LatLon::new(48.0, -4.0), zoom: 9.0 }, w: 800.0, h: 600.0 };
        let id = TileId::at(9, 48.0, -4.0);
        let (ox, oy, size) = tile_origin(&vi, &id);
        assert!(inside(&vi, &[id], ox + size / 2.0, oy + size / 2.0));
        assert!(!inside(&vi, &[id], ox - 1.0, oy + size / 2.0));
        assert!(!inside(&vi, &[], ox + 1.0, oy + 1.0));
    }
}
