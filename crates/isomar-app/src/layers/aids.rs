//! Buoys, beacons, landmarks and lights.

use super::credit::Credit;
use super::{Changes, DrawCtx, Layer, Redraw, TileArea, TilePaint};
use super::anchorages;
use crate::core::chart::aids::{Aid, AidKind, AidsData, Anchorage, Light};
use crate::core::chart::vtile::{TileCache, TileId, resolve, tiles_for};
use crate::core::geo::LatLon;
use crate::core::vector::Bounds;
use crate::aidinfo::{self, AidInfo, SAME_SPOT_PX};
use crate::symbols;
use crate::view::ViewInfo;
use crate::core::chart::aids::light_description;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

pub struct AidsLayer {
    data: RefCell<Option<Arc<AidsData>>>,
    /// Community data (OpenStreetMap seamarks) by tile. It is drawn only
    /// where the main data has nothing.
    supplement: RefCell<TileCache<Arc<AidsData>>>,
    online: Cell<bool>,
    credit: Rc<Credit>,
    /// Cells of `CELL_DEG` that hold main data, and their neighbours.
    covered: RefCell<Arc<HashSet<(i32, i32)>>>,
    /// Areas covered by chart cells that draw their own aids, with the cells'
    /// compilation scales: a cell too fine for the view draws nothing.
    exclude: RefCell<Vec<(crate::core::vector::Bounds, u32)>>,
    /// Whether the chart cells are shown at all: hidden, they leave their areas to these aids.
    enc_shown: Cell<bool>,
    changes: Changes,
    redraw: Redraw,
    /// The snapshot of the last area, which the names ask for to keep clear of the marks.
    last: RefCell<Option<(AreaKey, Option<Arc<AidsPaint>>)>>,
}

/// An area and the layer's revision, bit for bit.
type AreaKey = ([u64; 5], u64);

/// Tiles of community marks kept in memory.
const TILE_CAP: usize = 400;

/// Zoom thresholds below which each kind of symbol is hidden. Between `MIN_ZOOM_LIGHTS` and
/// `MIN_ZOOM_ALL_LIGHTS` a light shows when its range is at least `LIGHT_RANGE_PX` long on screen,
/// and so does the mark or lighthouse it stands on; unlit marks show from `MIN_ZOOM_AIDS`. A
/// light's sectors show `SECTORS_AFTER` zoom levels after the light, and not before `MIN_ZOOM_SECTORS`.
pub const MIN_ZOOM_LIGHTS: f64 = 7.0;
pub const SECTORS_AFTER: f64 = 1.0;
/// No sectors further out than this, so the arcs do not drown the all-round lights at small scale.
pub const MIN_ZOOM_SECTORS: f64 = 11.0;
/// Whole zooms, as `Declutter` decides at whole zooms (`names::mark_zoom`).
pub const MIN_ZOOM_AIDS: f64 = 12.0;
pub const MIN_ZOOM_ALL_LIGHTS: f64 = 13.0;
pub const MIN_ZOOM_LANDMARK: f64 = 13.0;
pub const LIGHT_RANGE_PX: f64 = 100.0;
/// The range taken for a light whose range is not known.
const DEFAULT_RANGE_NM: f64 = 2.0;
/// The full ring of an all-round major light, and what makes a light major, in nautical miles.
pub const MIN_ZOOM_RING: f64 = 13.0;
pub const MAJOR_RANGE_NM: f64 = 10.0;
/// The light's characteristic is written beside it from here.
pub const MIN_ZOOM_CHARACTER: f64 = 14.0;

/// Places on the ground, matched within `aidinfo::SAME_SPOT_M`: a light and the structure it
/// stands on are separate objects, a few metres apart in some data.
#[derive(Default)]
struct Spots(HashMap<(i32, i32), Vec<LatLon>>);

impl Spots {
    /// Buckets of a thousandth of a degree, more than `SAME_SPOT_M` across.
    fn bucket(p: &LatLon) -> (i32, i32) {
        ((p.lon * 1000.0).floor() as i32, (p.lat * 1000.0).floor() as i32)
    }

    fn add(&mut self, p: LatLon) {
        self.0.entry(Self::bucket(&p)).or_default().push(p);
    }

    fn has(&self, p: &LatLon) -> bool {
        let (x, y) = Self::bucket(p);
        (-1..=1).any(|dx| (-1..=1).any(|dy| self.0.get(&(x + dx, y + dy)).is_some_and(|v| v.iter().any(|q| aidinfo::same_spot(p, q)))))
    }
}

/// How a mark is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Glyph {
    Mark,
    Lighthouse,
    Landmark,
}

/// Which lights and marks a view shows: further out only the lights that reach far, and what
/// they stand on, at the same size as close in. Where they would still crowd each other, the
/// lights that reach farthest are kept and the others within `LIGHT_SPACING` flare lengths of them
/// on screen are left out, and a light's sectors show only where they would not lie on the arcs
/// of a light that reaches farther.
pub struct Declutter {
    zoom: f64,
    px_per_nm: f64,
    /// Where any light is, to tell a lighthouse (a landmark with a light on it) from a landmark.
    lit: Spots,
    /// Where a light that is shown is.
    shown: Spots,
    /// Where a light that shows its sectors is.
    sectored: Spots,
}

/// The least distance between two lights shown, in flare lengths.
pub const LIGHT_SPACING: f64 = 1.6;

/// Points on screen within reach of each other, bucketed by `size` pixels.
struct Near {
    size: f64,
    cells: HashMap<(i32, i32), Vec<(f64, f64, LatLon, f64)>>,
}

impl Near {
    fn new(size: f64) -> Near {
        Near { size: size.max(1.0), cells: HashMap::new() }
    }

    fn key(&self, x: f64, y: f64) -> (i32, i32) {
        ((x / self.size).floor() as i32, (y / self.size).floor() as i32)
    }

    fn add(&mut self, x: f64, y: f64, pos: LatLon, r: f64) {
        let k = self.key(x, y);
        self.cells.entry(k).or_default().push((x, y, pos, r));
    }

    /// Whether a point already added, not on `pos`'s spot, is closer to (x, y) than `gap` gives
    /// for its radius and `r`. `gap` must not exceed `size`.
    fn crowded(&self, x: f64, y: f64, pos: &LatLon, r: f64, gap: impl Fn(f64, f64) -> f64) -> bool {
        let (kx, ky) = self.key(x, y);
        (-1..=1).any(|dx| {
            (-1..=1).any(|dy| {
                self.cells.get(&(kx + dx, ky + dy)).is_some_and(|v| {
                    v.iter().any(|(qx, qy, q, qr)| (qx - x).hypot(qy - y) < gap(r, *qr) && !aidinfo::same_spot(pos, q))
                })
            })
        })
    }
}

impl Declutter {
    /// For `vi`, with the lights near enough to it to stand on a mark it draws. Decided as at the view's
    /// `names::mark_zoom`, so that what shows changes only at whole zooms, where the names are placed again.
    pub fn new<'a>(vi: &ViewInfo, lights: impl IntoIterator<Item = &'a Light>) -> Declutter {
        let (vi, _) = super::names::view_at(vi, super::names::mark_zoom(vi.zoom()));
        let vi = &vi;
        let mut d = Declutter {
            zoom: vi.zoom(),
            px_per_nm: 1852.0 / vi.metres_per_px(),
            lit: Spots::default(),
            shown: Spots::default(),
            sectored: Spots::default(),
        };
        let mut candidates: Vec<&Light> = Vec::new();
        for l in lights {
            d.lit.add(l.pos);
            if d.shows(l, 0.0) {
                candidates.push(l);
            }
        }
        // The lights that reach farthest first; the order of the rest does not move with the view.
        let reach = |l: &Light| l.range_nm.unwrap_or(DEFAULT_RANGE_NM);
        candidates.sort_by(|a, b| {
            reach(b).total_cmp(&reach(a)).then(b.sector.is_some().cmp(&a.sector.is_some())).then(a.pos.lat.total_cmp(&b.pos.lat)).then(a.pos.lon.total_cmp(&b.pos.lon))
        });
        let spacing = LIGHT_SPACING * symbols::flare_length(None);
        let mut kept = Near::new(spacing);
        for l in candidates.iter().copied() {
            let (x, y) = vi.to_screen(&l.pos);
            if d.shown.has(&l.pos) || !kept.crowded(x, y, &l.pos, 0.0, |_, _| spacing) {
                kept.add(x, y, l.pos, 0.0);
                d.shown.add(l.pos);
            }
        }
        // Sectors, each the mean of the two radii away from the arcs of a light that reaches farther,
        // unless they are drawn to their full range, which the user asked for.
        let full = symbols::full_sectors();
        let radius = |l: &Light| symbols::sector_radius(l.range_nm, full, vi.metres_per_px());
        let widest = symbols::sector_radius(Some(f64::MAX), false, 1.0);
        let mut arcs = Near::new(widest);
        let sectored: Vec<&Light> =
            candidates.into_iter().filter(|l| l.sector.is_some() && d.shown.has(&l.pos) && d.zoom >= MIN_ZOOM_SECTORS && d.shows(l, SECTORS_AFTER)).collect();
        for l in sectored {
            let (x, y) = vi.to_screen(&l.pos);
            let r = radius(l);
            if full || d.sectored.has(&l.pos) || !arcs.crowded(x, y, &l.pos, r, |a, b| (a + b) / 2.0) {
                arcs.add(x, y, l.pos, r);
                d.sectored.add(l.pos);
            }
        }
        d
    }

    pub fn light(&self, l: &Light) -> bool {
        self.shows(l, 0.0) && self.shown.has(&l.pos)
    }

    /// Whether a sector light shows its sectors rather than a flare.
    pub fn sectors(&self, l: &Light) -> bool {
        l.sector.is_some() && self.sectored.has(&l.pos)
    }

    /// Whether `l` shows at `later` zoom levels past where lights do.
    fn shows(&self, l: &Light, later: f64) -> bool {
        let zoom = self.zoom - later;
        let px_per_nm = self.px_per_nm / 2f64.powf(later);
        zoom >= MIN_ZOOM_ALL_LIGHTS || (zoom >= MIN_ZOOM_LIGHTS && l.range_nm.unwrap_or(DEFAULT_RANGE_NM) * px_per_nm >= LIGHT_RANGE_PX)
    }

    /// How a mark of `kind` at `pos` is drawn, if it is.
    pub fn glyph(&self, kind: AidKind, pos: &LatLon) -> Option<Glyph> {
        if kind != AidKind::Landmark {
            (self.zoom >= MIN_ZOOM_AIDS || self.shown.has(pos)).then_some(Glyph::Mark)
        } else if self.lit.has(pos) {
            self.shown.has(pos).then_some(Glyph::Lighthouse)
        } else {
            (self.zoom >= MIN_ZOOM_LANDMARK).then_some(Glyph::Landmark)
        }
    }
}

/// The flares already drawn on each spot, so that lights on one structure fan out clockwise
/// instead of lying on each other, and the sectors of one light show each colour once.
#[derive(Default)]
pub struct Fans(HashMap<(i32, i32), Vec<u8>>);

impl Fans {
    /// Draws the flares of a light's colours that its spot does not have yet, as long as its range
    /// makes them.
    pub fn light(&mut self, cr: &cairo::Context, pal: &crate::palette::Palette, x: f64, y: f64, colours: &[u8], range_nm: Option<f64>) {
        let drawn = self.0.entry((x.round() as i32, y.round() as i32)).or_default();
        let colours: &[u8] = if colours.is_empty() { &[0] } else { colours };
        let new: Vec<u8> = colours.iter().copied().filter(|c| !drawn.contains(c)).collect();
        if !new.is_empty() {
            symbols::light(cr, pal, x, y, &new, drawn.len(), range_nm);
            drawn.extend(new);
        }
    }
}

pub fn draw_aid(cr: &cairo::Context, pal: &crate::palette::Palette, x: f64, y: f64, a: &Aid, glyph: Glyph) {
    match glyph {
        Glyph::Mark => symbols::aid(cr, pal, x, y, symbols::MARK_PX, a),
        Glyph::Lighthouse => symbols::lighthouse(cr, pal, x, y, symbols::LIGHTHOUSE_PX),
        Glyph::Landmark => symbols::landmark(cr, pal, x, y, symbols::MARK_PX),
    }
}

/// Which marks are left out: those a chart cell draws, and community marks beside official ones.
struct Hidden {
    exclude: Vec<Bounds>,
    covered: Arc<HashSet<(i32, i32)>>,
}

impl Hidden {
    fn at(&self, lon: f64, lat: f64, community: bool) -> bool {
        self.exclude.iter().any(|b| b.contains([lon, lat])) || (community && self.covered.contains(&cell(lon, lat)))
    }
}

impl AidsLayer {
    pub fn new(redraw: Redraw, credit: Rc<Credit>) -> AidsLayer {
        AidsLayer {
            data: RefCell::new(None),
            supplement: RefCell::new(TileCache::new(TILE_CAP)),
            online: Cell::new(false),
            credit,
            covered: RefCell::new(Arc::new(HashSet::new())),
            exclude: RefCell::new(Vec::new()),
            enc_shown: Cell::new(true),
            changes: Changes::default(),
            redraw,
            last: RefCell::new(None),
        }
    }

    pub fn set_data(&self, data: Arc<AidsData>) {
        *self.covered.borrow_mut() = Arc::new(covered_cells(&data));
        *self.data.borrow_mut() = Some(data);
        self.changes.bump(None);
        (self.redraw)();
    }

    /// Whether the downloaded community marks are drawn.
    pub fn set_online(&self, on: bool) {
        if self.online.replace(on) != on {
            self.changes.bump(None);
            (self.redraw)();
        }
    }

    /// Adds the community aids of one tile, replacing any earlier copy.
    pub fn add_tile(&self, tile: TileId, data: Arc<AidsData>) {
        self.supplement.borrow_mut().insert(tile, data);
        self.changes.bump(Some(super::reach_of(&tile, 0.0)));
        (self.redraw)();
    }

    pub fn has_tile(&self, tile: &TileId) -> bool {
        self.supplement.borrow().contains(tile)
    }

    /// The downloaded tiles to draw for `view`: each wanted tile, or the
    /// nearest coarser one loaded in its place.
    fn community_in(&self, view: &crate::core::vector::Bounds, zoom: f64) -> Vec<Arc<AidsData>> {
        if !self.online.get() {
            return Vec::new();
        }
        let wanted = tiles_for(view, TileId::zoom_for(zoom), 128);
        let resolved = {
            let tiles = self.supplement.borrow();
            resolve(&wanted, |t| tiles.contains(t))
        };
        let mut tiles = self.supplement.borrow_mut();
        resolved.into_iter().filter_map(|(src, _)| tiles.get(&src)).collect()
    }

    /// The main data and the community tiles to draw for `view`; the flag is
    /// true for community marks.
    fn sources_in(&self, view: &Bounds, zoom: f64) -> Vec<(Arc<AidsData>, bool)> {
        let mut sources: Vec<(Arc<AidsData>, bool)> = self.data.borrow().iter().map(|d| (d.clone(), false)).collect();
        sources.extend(self.community_in(view, zoom).into_iter().map(|d| (d, true)));
        sources
    }

    /// Tells whether a mark at (lon, lat) is left to a chart cell, or is a
    /// community mark beside an official one.
    fn hidden_fn(&self, vi: ViewInfo) -> Hidden {
        let denom = super::enc::view_scale(vi.view.center.lat, vi.zoom());
        let exclude: Vec<Bounds> = if self.enc_shown.get() {
            self.exclude.borrow().iter().filter(|(_, cscl)| super::enc::cell_visible(*cscl, denom)).map(|(b, _)| *b).collect()
        } else {
            Vec::new()
        };
        Hidden { exclude, covered: self.covered.borrow().clone() }
    }

    /// The mark within `tol` pixels of (`x`, `y`), if what it is drawn as is
    /// on screen at this zoom.
    pub fn info_at(&self, vi: &ViewInfo, x: f64, y: f64, tol: f64) -> Option<AidInfo> {
        let zoom = vi.zoom();
        if zoom < MIN_ZOOM_LIGHTS {
            return None;
        }
        let near = aidinfo::click_bounds(vi, x, y, tol + SAME_SPOT_PX);
        let view = vi.bounds().expanded(60.0 * vi.deg_per_px());
        let sources = self.sources_in(&view, zoom);
        let hidden = self.hidden_fn(*vi);
        let (mut aids, mut lights) = (Vec::new(), Vec::new());
        for (data, community) in &sources {
            lights.extend(data.lights_in(&view).filter(|l| !hidden.at(l.pos.lon, l.pos.lat, *community)).cloned());
        }
        // Only what is drawn, which depends on the lights round it.
        let declutter = Declutter::new(vi, &lights);
        lights.retain(|l| near.contains([l.pos.lon, l.pos.lat]));
        for (data, community) in &sources {
            aids.extend(
                data.aids_in(&near)
                    .filter(|a| {
                        !hidden.at(a.pos.lon, a.pos.lat, *community) && declutter.glyph(a.kind, &a.pos).is_some()
                    })
                    .cloned(),
            );
        }
        lights.retain(|l| declutter.light(l));
        aidinfo::pick(vi, x, y, tol, aids, lights)
    }

    /// The anchorage under (`x`, `y`), with how far its anchor is, if one is drawn there (`anchorages::hit`).
    pub fn anchorage_at(&self, vi: &ViewInfo, x: f64, y: f64, tol: f64) -> Option<(f64, Anchorage)> {
        let zoom = vi.zoom();
        let near = aidinfo::click_bounds(vi, x, y, tol);
        let hidden = self.hidden_fn(*vi);
        self.sources_in(&near, zoom)
            .iter()
            .flat_map(|(data, _)| data.anchorages_in(&near).filter(|a| !hidden.at(a.pos.lon, a.pos.lat, false) && anchorages::shown(a, zoom)).cloned().collect::<Vec<_>>())
            .filter_map(|a| Some((anchorages::hit(vi, &a, x, y, tol)?, a)))
            .min_by(|a, b| a.0.total_cmp(&b.0))
    }

    /// Follows the chart cells' layer: while it is hidden, the aids are drawn
    /// inside its cells too, as they are where there is none.
    pub fn set_enc_shown(&self, shown: bool) {
        if self.enc_shown.replace(shown) == shown {
            return;
        }
        let mut touched: Option<Bounds> = None;
        for (b, _) in self.exclude.borrow().iter() {
            touched.get_or_insert(*b).union(b);
        }
        if touched.is_some() {
            self.changes.bump(touched);
            (self.redraw)();
        }
    }

    /// Stops drawing the open-data aids inside these areas (each with the
    /// scale of its chart) while that chart is on screen.
    pub fn set_exclusions(&self, areas: Vec<(crate::core::vector::Bounds, u32)>) {
        // Called again as each batch of cells is read: only the areas that came or went change anything.
        let key = |(b, s): &(crate::core::vector::Bounds, u32)| ([b.min_lon, b.min_lat, b.max_lon, b.max_lat].map(f64::to_bits), *s);
        let mut exclude = self.exclude.borrow_mut();
        let (before, after): (HashSet<_>, HashSet<_>) = (exclude.iter().map(key).collect(), areas.iter().map(key).collect());
        let mut touched: Option<crate::core::vector::Bounds> = None;
        for a in exclude.iter().chain(&areas).filter(|a| before.contains(&key(a)) != after.contains(&key(a))) {
            touched.get_or_insert(a.0).union(&a.0);
        }
        *exclude = areas;
        drop(exclude);
        if touched.is_some() {
            self.changes.bump(touched);
            (self.redraw)();
        }
    }
}

impl Layer for AidsLayer {
    fn id(&self) -> &'static str {
        "aids"
    }

    fn revision(&self) -> u64 {
        self.changes.get()
    }

    fn changes_since(&self, revision: u64) -> Option<Vec<super::Change>> {
        self.changes.since(revision)
    }

    fn prepare(&self, area: &TileArea) -> Option<Arc<dyn TilePaint>> {
        self.paint_for(area).map(|p| p as Arc<dyn TilePaint>)
    }
}

impl AidsLayer {
    /// Where the marks changed since `revision` (`None`: anywhere), and the revision now.
    pub fn changes_after(&self, revision: u64) -> (Option<Vec<super::Change>>, u64) {
        (self.changes.since(revision), self.changes.get())
    }

    /// What the tiles of `area` are drawn from, made once for the area: the names ask for it too, to
    /// keep clear of the marks and lights.
    pub fn paint_for(&self, area: &TileArea) -> Option<Arc<AidsPaint>> {
        let b = area.vi.bounds();
        let key = ([b.min_lon, b.min_lat, b.max_lon, b.max_lat, area.vi.zoom()].map(f64::to_bits), self.changes.get());
        if let Some((k, paint)) = self.last.borrow().as_ref()
            && *k == key
        {
            return paint.clone();
        }
        let paint = self.make_paint(area);
        *self.last.borrow_mut() = Some((key, paint.clone()));
        paint
    }

    fn make_paint(&self, area: &TileArea) -> Option<Arc<AidsPaint>> {
        let vi = &area.vi;
        let zoom = vi.zoom();
        self.credit.marks.set(false);
        if zoom < MIN_ZOOM_LIGHTS {
            return None;
        }
        let view = vi.bounds().expanded(60.0 * vi.deg_per_px());
        let sources = self.sources_in(&view, zoom);
        if sources.is_empty() {
            return None;
        }
        let hidden = self.hidden_fn(*vi);
        // Which lights and marks show is decided once for the area, so that every tile agrees.
        let all_lights = || sources.iter().flat_map(|(data, community)| data.lights_in(&view).filter(|l| !hidden.at(l.pos.lon, l.pos.lat, *community)).map(move |l| (l, *community)));
        let declutter = Declutter::new(vi, all_lights().map(|(l, _)| l));
        // The licence asks for a credit whenever community marks are on screen.
        let community_shown = all_lights().any(|(l, community)| community && declutter.light(l))
            || sources.iter().any(|(data, community)| {
                *community
                    && (data.aids_in(&view).any(|a| !hidden.at(a.pos.lon, a.pos.lat, true) && declutter.glyph(a.kind, &a.pos).is_some())
                        || data.anchorages_in(&view).any(|a| !hidden.at(a.pos.lon, a.pos.lat, false) && anchorages::shown(a, zoom)))
            });
        self.credit.marks.set(community_shown);
        let mut paint = AidsPaint { sources, hidden, declutter, metres_per_px: vi.metres_per_px(), anchors: HashSet::new() };
        // Which anchors are drawn is decided once for the area, past it by a tile, so every tile agrees.
        let reach = view.expanded(crate::render::TILE as f64 * vi.deg_per_px());
        let all: Vec<&Anchorage> = paint.anchorages_in(&reach, zoom).collect();
        let anchors = anchorages::thin(vi, &all).into_iter().zip(&all).filter(|(kept, _)| *kept).map(|(_, a)| anchor_key(a)).collect();
        paint.anchors = anchors;
        Some(Arc::new(paint))
    }
}

pub struct AidsPaint {
    /// The main data first, then each community tile, which hides where the main data is.
    sources: Vec<(Arc<AidsData>, bool)>,
    hidden: Hidden,
    declutter: Declutter,
    /// At the middle of the area: the radii of sectors follow it.
    metres_per_px: f64,
    /// The anchorages whose anchor is drawn (`anchorages::thin`), by `anchor_key`.
    anchors: HashSet<[u64; 3]>,
}

/// An anchorage's symbol, told apart from others: its spot and its kind.
fn anchor_key(a: &Anchorage) -> [u64; 3] {
    [a.pos.lon.to_bits(), a.pos.lat.to_bits(), a.kind as u64]
}

impl AidsPaint {
    /// The anchorages in `view` shown at `zoom`. The main data has none, so a community one is left out only
    /// where a chart cell draws its own.
    fn anchorages_in<'a>(&'a self, view: &'a Bounds, zoom: f64) -> impl Iterator<Item = &'a Anchorage> + 'a {
        self.sources
            .iter()
            .flat_map(move |(data, _)| data.anchorages_in(view))
            .filter(move |a| !self.hidden.at(a.pos.lon, a.pos.lat, false) && anchorages::shown(a, zoom))
    }

    /// The boxes on the screen of `vi` that the marks, the lights' flares and their characteristics take,
    /// within `margin` pixels of it: names are not written under them. Once names are settled
    /// (`names::settled`), every mark and light, as if all were shown: those crowding leaves out show as the
    /// view zooms in, and the names are not placed again.
    pub fn boxes(&self, vi: &ViewInfo, margin: f64) -> Vec<[f64; 4]> {
        let zoom = vi.zoom();
        let all = super::names::settled(zoom);
        let view = vi.bounds().expanded((60.0 + margin) * vi.deg_per_px());
        let (sources, hidden, declutter) = (&self.sources, &self.hidden, &self.declutter);
        let mark = symbols::MARK_PX * symbols::mark_scale();
        let star = symbols::LIGHTHOUSE_PX * symbols::mark_scale();
        let mut out = Vec::new();
        for a in self.anchorages_in(&view, zoom) {
            anchorages::boxes(vi, a, self.anchors.contains(&anchor_key(a)), &mut out);
        }
        for (data, community) in sources {
            for a in data.aids_in(&view).filter(|a| !hidden.at(a.pos.lon, a.pos.lat, *community)) {
                let glyph = declutter.glyph(a.kind, &a.pos);
                if all || glyph.is_some() {
                    let (x, y) = vi.to_screen(&a.pos);
                    match glyph {
                        // A lighthouse's star and a landmark's circle are centred on the spot.
                        Some(Glyph::Lighthouse) => out.push([x - star * 0.5, y - star * 0.5, x + star * 0.5, y + star * 0.5]),
                        Some(Glyph::Landmark) => out.push([x - mark * 0.5, y - mark * 0.5, x + mark * 0.5, y + mark * 0.5]),
                        // A mark stands on its spot, with its topmark above it.
                        _ => out.push([x - mark * 0.6, y - mark * 1.3, x + mark * 0.6, y + mark * 0.4]),
                    }
                }
            }
            for l in data.lights_in(&view).filter(|l| !hidden.at(l.pos.lon, l.pos.lat, *community) && (all || declutter.light(l))) {
                let (x, y) = vi.to_screen(&l.pos);
                // Settled, a sector light's flare is kept clear of too: crowding may leave its sectors out.
                if !all && declutter.sectors(l) {
                    out.push([x - 3.0, y - 3.0, x + 3.0, y + 3.0]);
                    continue;
                }
                // The flares point south-east, fanned clockwise when a spot has several: a few small boxes along
                // the drop, not one box round it, which would take the empty corner beside it too.
                let len = symbols::flare_length(l.range_nm);
                out.push([x - 3.0, y - 3.0, x + 3.0, y + 3.0]);
                for (t, r) in [(0.3, 0.12), (0.55, 0.17), (0.79, 0.23)] {
                    let (cx, cy) = (x + t * len * 0.75, y + t * len * 0.8);
                    out.push([cx - r * len, cy - r * len, cx + r * len, cy + r * len]);
                }
                if zoom >= MIN_ZOOM_CHARACTER {
                    let (tx, ty) = (x + len * 0.75 + 4.0, y + len * 0.75 + 2.0);
                    let em = 10.0 * super::text_scale() as f64 / 100.0;
                    out.push([tx, ty - em, tx + light_description(l).chars().count() as f64 * em * 0.6, ty + em * 0.4]);
                }
            }
        }
        out
    }

    /// Where the major lights in `view` stand: those that reach `MAJOR_RANGE_NM` or more, and of those this area
    /// shows, those on a lighthouse too (Beachy Head's reaches 8 miles), with the names of what stands there
    /// ("Start Point Lighthouse"), the lighthouse's first. A cape beside a light shown is named from the zoom its
    /// light shows at; one beside any is named as its light is.
    pub fn major_lights(&self, view: &Bounds) -> Vec<MajorLight> {
        let mut out: Vec<MajorLight> = Vec::new();
        for (data, community) in &self.sources {
            for l in data.lights_in(view).filter(|l| !self.hidden.at(l.pos.lon, l.pos.lat, *community)) {
                let shown = self.declutter.light(l);
                let far = l.range_nm.is_some_and(|r| r >= MAJOR_RANGE_NM);
                if !shown && !far {
                    continue;
                }
                // A lighthouse carries several lights, one entry for them all.
                if let Some(m) = out.iter_mut().find(|m| aidinfo::same_spot(&m.at, &l.pos)) {
                    m.shown |= shown;
                    continue;
                }
                let spot = Bounds { min_lon: l.pos.lon - 0.001, min_lat: l.pos.lat - 0.001, max_lon: l.pos.lon + 0.001, max_lat: l.pos.lat + 0.001 };
                let on: Vec<&Aid> = data.aids_in(&spot).filter(|a| aidinfo::same_spot(&a.pos, &l.pos)).collect();
                let lighthouse = on.iter().any(|a| a.kind == AidKind::Landmark);
                if !lighthouse && !far {
                    continue;
                }
                // Every name there, the lighthouse's first: one may be English ("Cape Ortegal"), another local.
                let mut on = on;
                on.sort_by_key(|a| a.kind != AidKind::Landmark);
                let mut names: Vec<String> = Vec::new();
                for n in on.iter().filter_map(|a| a.name.clone()) {
                    if !names.contains(&n) {
                        names.push(n);
                    }
                }
                out.push(MajorLight { at: l.pos, names, shown });
            }
        }
        out
    }
}

/// A major light, for the names of the capes (`AidsPaint::major_lights`).
#[derive(Clone, Debug)]
pub struct MajorLight {
    pub at: LatLon,
    /// The names of what stands there, the lighthouse's first.
    pub names: Vec<String>,
    /// Whether the area shows it (`Declutter`).
    pub shown: bool,
}

impl TilePaint for AidsPaint {
    fn paint(&self, ctx: &DrawCtx) {
        let (cr, vi, pal) = (ctx.cr, &ctx.vi, &ctx.pal);
        let zoom = vi.zoom();
        let view = vi.bounds().expanded(60.0 * vi.deg_per_px());
        let (sources, hidden, declutter) = (&self.sources, &self.hidden, &self.declutter);
        let lights = || {
            sources
                .iter()
                .flat_map(|(data, community)| data.lights_in(&view).filter(move |l| !hidden.at(l.pos.lon, l.pos.lat, *community)).map(move |l| (l, *community)))
                .filter(|(l, _)| declutter.light(l))
        };
        // Anchorages under the marks that may stand in them.
        for a in self.anchorages_in(&view, zoom) {
            anchorages::draw(cr, pal, vi, a, self.anchors.contains(&anchor_key(a)));
        }
        // The sectors are drawn under the names, by `AidSectors`.
        for (data, community) in sources {
            for a in data.aids_in(&view).filter(|a| !hidden.at(a.pos.lon, a.pos.lat, *community)) {
                if let Some(glyph) = declutter.glyph(a.kind, &a.pos) {
                    let (x, y) = vi.to_screen(&a.pos);
                    draw_aid(cr, pal, x, y, a, glyph);
                }
            }
        }
        let mut fans = Fans::default();
        for (l, _) in lights() {
            let (x, y) = vi.to_screen(&l.pos);
            if declutter.sectors(l) {
                // Sector lights are shown by their arcs and a dot.
                symbols::light_dot(cr, pal, x, y);
                continue;
            }
            fans.light(cr, pal, x, y, &l.colours, l.range_nm);
            // An all-round major light gets its full ring at large scale.
            if l.sector.is_none() && l.range_nm.is_some_and(|r| r >= MAJOR_RANGE_NM) && zoom >= MIN_ZOOM_RING {
                symbols::light_ring(cr, pal, x, y, symbols::sector_radius(l.range_nm, symbols::full_sectors(), self.metres_per_px), &l.colours);
            }
            // The characteristic is written beside the light only where there is room for it.
            if zoom >= MIN_ZOOM_CHARACTER {
                let r = symbols::flare_length(l.range_nm);
                // 10 pt was cairo's default, which the text used before it followed the text size preference.
                super::font_size(cr, 10.0);
                super::halo_text(cr, pal, x + r * 0.75 + 4.0, y + r * 0.75 + 2.0, &light_description(l), crate::palette::TEXT);
            }
        }
    }
}

/// The arcs of the sector lights, drawn under the names of places, capes and waters (`towns`), which are
/// written over them as on a paper chart; the marks and lights themselves stay over the names (`AidsLayer`).
/// It shares the marks' id, so hiding the marks hides their sectors.
pub struct AidSectors(pub Rc<AidsLayer>);

impl Layer for AidSectors {
    fn id(&self) -> &'static str {
        "aids"
    }

    fn revision(&self) -> u64 {
        self.0.revision()
    }

    fn changes_since(&self, revision: u64) -> Option<Vec<super::Change>> {
        self.0.changes_since(revision)
    }

    fn prepare(&self, area: &TileArea) -> Option<Arc<dyn TilePaint>> {
        self.0.paint_for(area).map(|p| Arc::new(SectorPaint(p)) as Arc<dyn TilePaint>)
    }
}

struct SectorPaint(Arc<AidsPaint>);

impl TilePaint for SectorPaint {
    fn paint(&self, ctx: &DrawCtx) {
        let (cr, vi, pal) = (ctx.cr, &ctx.vi, &ctx.pal);
        let p = &self.0;
        let view = vi.bounds().expanded(60.0 * vi.deg_per_px());
        for (data, community) in &p.sources {
            for l in data.lights_in(&view).filter(|l| !p.hidden.at(l.pos.lon, l.pos.lat, *community) && p.declutter.light(l)) {
                if let Some((a, b)) = l.sector.filter(|_| p.declutter.sectors(l)) {
                    let (x, y) = vi.to_screen(&l.pos);
                    symbols::light_sector(cr, pal, x, y, symbols::sector_radius(l.range_nm, symbols::full_sectors(), p.metres_per_px), a, b, &l.colours);
                }
            }
        }
    }
}

/// Size of the cells used to tell where the main data has coverage, in degrees.
const CELL_DEG: f64 = 0.1;

fn cell(lon: f64, lat: f64) -> (i32, i32) {
    ((lon / CELL_DEG).floor() as i32, (lat / CELL_DEG).floor() as i32)
}

/// Every cell holding an aid or light, and the eight around it, so that
/// community marks right beside official ones are not drawn twice.
fn covered_cells(d: &AidsData) -> HashSet<(i32, i32)> {
    let mut out = HashSet::new();
    let points = d.aids.iter().map(|a| a.pos).chain(d.lights.iter().map(|l| l.pos));
    for p in points {
        let (x, y) = cell(p.lon, p.lat);
        for dx in -1..=1 {
            for dy in -1..=1 {
                out.insert((x + dx, y + dy));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_data_covers_its_cell_and_the_neighbours() {
        use crate::core::chart::aids::Aid;
        use crate::core::geo::LatLon;
        let d = AidsData {
            aids: vec![Aid { pos: LatLon::new(48.35, -4.45), kind: AidKind::BuoyLateral, name: None, colours: vec![], shape: None, category: None }],
            lights: vec![],
            anchorages: vec![],
        };
        let c = covered_cells(&d);
        assert!(c.contains(&cell(-4.45, 48.35)));
        assert!(c.contains(&cell(-4.45 + 0.1, 48.35 - 0.1)), "a neighbouring cell");
        assert!(!c.contains(&cell(-4.45 + 0.3, 48.35)), "three cells away is open");
        assert_eq!(c.len(), 9);
    }

    #[test]
    fn far_out_only_long_range_lights_and_what_they_stand_on_show() {
        let light = |pos: LatLon, range: f64| Light { pos, colours: vec![1], character: Some(2), group: None, period_s: None, range_nm: Some(range), sector: None, height_m: None };
        let at = |zoom: f64| ViewInfo { view: crate::view::View { center: LatLon::new(48.0, -4.0), zoom }, w: 800.0, h: 600.0 };
        let (major, minor, unlit) = (LatLon::new(48.0, -4.0), LatLon::new(48.0, -3.99), LatLon::new(48.0, -4.01));
        let lights = [light(major, 20.0), light(minor, 2.0)];
        for (zoom, minor_shown, unlit_shown) in [(9.0, false, false), (11.0, false, false), (12.0, true, true)] {
            let vi = at(zoom);
            let d = Declutter::new(&vi, &lights);
            assert!(d.light(&lights[0]), "a 20 nm light shows at zoom {zoom}");
            assert_eq!(d.light(&lights[1]), minor_shown, "a 2 nm light at zoom {zoom}");
            assert_eq!(d.glyph(AidKind::Landmark, &major), Some(Glyph::Lighthouse));
            assert_eq!(d.glyph(AidKind::BuoyLateral, &minor).is_some(), minor_shown);
            assert_eq!(d.glyph(AidKind::BuoyLateral, &unlit).is_some(), unlit_shown);
            assert_eq!(d.glyph(AidKind::Landmark, &unlit), None, "a plain landmark only from {MIN_ZOOM_LANDMARK}");
        }
    }

    #[test]
    fn sectors_show_a_zoom_level_after_their_light() {
        let light = |range: f64| Light { pos: LatLon::new(48.0, -4.0), colours: vec![1], character: Some(2), group: None, period_s: None, range_nm: Some(range), sector: Some((30.0, 90.0)), height_m: None };
        let at = |zoom: f64, l: &Light| Declutter::new(&ViewInfo { view: crate::view::View { center: LatLon::new(48.0, -4.0), zoom }, w: 800.0, h: 600.0 }, [l]);
        // Where a light first shows, by bisection.
        let first = |l: &Light, f: &dyn Fn(&Declutter, &Light) -> bool| {
            let (mut lo, mut hi) = (0.0, 20.0);
            for _ in 0..40 {
                let mid = (lo + hi) / 2.0;
                if f(&at(mid, l), l) { hi = mid } else { lo = mid }
            }
            hi
        };
        for range in [1.0, 2.0, 4.0] {
            let l = light(range);
            let (shown, sectors) = (first(&l, &|d, l| d.light(l)), first(&l, &|d, l| d.sectors(l)));
            assert!((sectors - shown - SECTORS_AFTER).abs() < 1e-6, "{range} nm: light at {shown}, sectors at {sectors}");
        }
        assert!(first(&light(4.0), &|d, l| d.sectors(l)) < first(&light(2.0), &|d, l| d.sectors(l)));
        // A far-reaching light shows early, but its sectors wait for MIN_ZOOM_SECTORS.
        let far = light(25.0);
        assert!(first(&far, &|d, l| d.light(l)) + SECTORS_AFTER < MIN_ZOOM_SECTORS);
        assert!((first(&far, &|d, l| d.sectors(l)) - MIN_ZOOM_SECTORS).abs() < 1e-6);
        let round = Light { sector: None, ..light(10.0) };
        assert!(!at(16.0, &round).sectors(&round), "an all-round light has none");
    }

    #[test]
    fn crowded_lights_leave_room_for_the_one_that_reaches_farthest() {
        let vi = ViewInfo { view: crate::view::View { center: LatLon::new(48.0, -4.0), zoom: 12.0 }, w: 800.0, h: 600.0 };
        let light = |lon: f64, range: f64, sector: Option<(f64, f64)>| Light {
            pos: LatLon::new(48.0, lon),
            colours: vec![1],
            character: Some(2),
            group: None,
            period_s: None,
            range_nm: Some(range),
            sector,
            height_m: None,
        };
        let px = vi.metres_per_px() / (1852.0 * 60.0 * 48f64.to_radians().cos());
        let gap = LIGHT_SPACING * symbols::flare_length(None);
        let (major, close, far) = (light(-4.0, 10.0, None), light(-4.0 + 0.5 * gap * px, 3.0, None), light(-4.0 + 1.5 * gap * px, 3.0, None));
        // Another light on the same structure stays, to fan out beside it.
        let same = light(-4.0 + 0.00001, 3.0, None);
        let d = Declutter::new(&vi, [&close, &far, &major, &same]);
        assert!(d.light(&major) && d.light(&far) && d.light(&same));
        assert!(!d.light(&close), "a lesser light within the spacing is left out");

        // Sectors that would lie on the arcs of a light that reaches farther show as a flare.
        let at16 = ViewInfo { view: crate::view::View { zoom: 16.0, ..vi.view }, ..vi };
        let px = at16.metres_per_px() / (1852.0 * 60.0 * 48f64.to_radians().cos());
        let r = symbols::sector_radius(Some(10.0), false, at16.metres_per_px());
        let (a, b) = (light(-4.0, 10.0, Some((0.0, 90.0))), light(-4.0 + 0.5 * r * px, 10.0, Some((90.0, 180.0))));
        let c = light(-4.0 + 1.2 * r * px, 10.0, Some((90.0, 180.0)));
        let d = Declutter::new(&at16, [&a, &b, &c]);
        assert!(d.light(&b) && !d.sectors(&b));
        assert!(d.sectors(&a) && d.sectors(&c));
    }

    /// Planier: unlit special marks 350 m from the lighthouse are two pixels from it at zoom 9.2,
    /// but they do not stand on its light.
    #[test]
    fn a_mark_beside_a_lighthouse_is_not_lit_by_it() {
        let lighthouse = LatLon::new(43.1988335, 5.2306066);
        let buoy = LatLon::new(43.1971131, 5.2266159);
        let light = Light { pos: lighthouse, colours: vec![1], character: Some(2), group: None, period_s: None, range_nm: Some(23.0), sector: None, height_m: None };
        let vi = ViewInfo { view: crate::view::View { center: lighthouse, zoom: 9.2 }, w: 800.0, h: 600.0 };
        let d = Declutter::new(&vi, [&light]);
        assert_eq!(d.glyph(AidKind::Landmark, &lighthouse), Some(Glyph::Lighthouse));
        assert_eq!(d.glyph(AidKind::BuoySpecial, &buoy), None);
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn zoom_thresholds_are_ordered_sensibly() {
        assert!(MIN_ZOOM_LIGHTS < MIN_ZOOM_SECTORS);
        assert!(MIN_ZOOM_SECTORS < MIN_ZOOM_AIDS);
        assert!(MIN_ZOOM_AIDS < MIN_ZOOM_ALL_LIGHTS);
        assert!(MIN_ZOOM_ALL_LIGHTS <= MIN_ZOOM_LANDMARK);
    }
}

