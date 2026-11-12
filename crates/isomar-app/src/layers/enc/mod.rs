//! S-57 chart cells drawn with a simplified S-52 presentation: depth areas
//! shaded around the safety contour, land, contours, soundings, hazards,
//! restricted areas, and buoys, beacons and lights.

use super::bathy::depth_color;
use super::{chartinfo, hazard};
use super::anchorages;
use super::names::{self, PathLabel, Placed};
use super::regions::{cape_label, set_along};
use super::relief::{ReliefLayer, ReliefPaint};
use super::{Changes, DrawCtx, LabelPlacer, Layer, Redraw, TileArea, TilePaint, font_size, halo_text, text_scale};
use crate::platform::{bg, spawn};
use crate::core::chart::aids::{Aid, AidKind, Anchorage, AnchorageKind, Light};
use crate::core::chart::s57::{Cell, CellSummary, Feature, Geom, attr, load_cell, obj};
use crate::core::chart::naturalearth::GeoKind;
use crate::core::chart::capes::{is_cape_name, is_cove_name};
use crate::core::geo::{LatLon, metres_per_pixel};
use crate::core::labels::coast::{CapeAxis, cape_axis, cove_axis};
use crate::core::labels::{Mask, Spine, spine};
use crate::core::vector::{self as core_vector, Bounds};
use crate::aidinfo::{self, AidInfo, SAME_SPOT_PX};
use super::aids::{Declutter, Fans, MIN_ZOOM_LIGHTS, draw_aid};
use crate::palette::{LAND, LAND_EDGE, MAGENTA, Paint, Palette, rgb};
use crate::tokens::Token;
use crate::symbols;
use crate::view::ViewInfo;
use std::cell::{Cell as StdCell, RefCell};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::{Duration, Instant};

mod draw;
mod info;
mod naming;
mod soundings;

use draw::*;
use naming::*;
pub use soundings::*;

/// Pixel size on a typical screen, to turn metres per pixel into a chart scale.
const METRES_PER_PIXEL_AT_96_DPI: f64 = 0.000_264_583;

/// The scale denominator the view is shown at (1:N).
pub fn view_scale(lat: f64, zoom: f64) -> f64 {
    metres_per_pixel(lat, zoom) / METRES_PER_PIXEL_AT_96_DPI
}

/// A cell is drawn from this much coarser than its compilation scale down to
/// any zoom: harbour charts do not clutter a passage view.
pub fn cell_visible(cscl: u32, view_denominator: f64) -> bool {
    cscl == 0 || view_denominator <= cscl as f64 * 3.0
}

/// A cell is drawn if it meets both compilation scale rules and navigational purpose / usage band limits.
pub fn cell_summary_visible(meta: &CellSummary, view_denominator: f64, max_band: Option<u8>) -> bool {
    let band = meta.usage_band();
    if let Some(limit) = max_band
        && band > limit
    {
        return false;
    }
    // Scale cutoffs per band so harbour and berthing charts don't clutter coastal views:
    let max_denom_for_band = match band {
        6 => 15_000.0,
        5 => 55_000.0,
        4 => 220_000.0,
        3 => 850_000.0,
        2 => 3_500_000.0,
        _ => f64::INFINITY,
    };
    if view_denominator > max_denom_for_band {
        return false;
    }
    cell_visible(meta.cscl, view_denominator)
}

/// Fill colour of a depth area: the tint the depth grids shade the middle of its range with, so that
/// a cell's edge does not show against grid shading or a coarser cell; the safety contour is the
/// heavy line. Drying ground is green.
pub fn depth_area_colour(drval1: f64, drval2: Option<f64>, safety: f64) -> Token {
    if drval1 < 0.0 {
        return Token::ChartDrying;
    }
    let mid = drval2.filter(|d2| *d2 > drval1).map_or(drval1, |d2| (drval1 + d2) / 2.0);
    depth_color(-(mid as f32), safety as f32)
}

/// How many cells are read from disk at once while drawing.
const MAX_READS: usize = 4;

/// The most cells drawn at once. Real charts overlap a few deep; past this
/// the coarsest are left out rather than let one draw take minutes.
const MAX_DRAWN: usize = 40;

/// While cells are still arriving, how seldom at least the layer is redrawn to show them.
const PROGRESS_EVERY: Duration = Duration::from_millis(150);

/// Cells kept in memory once they are off screen; the least recently drawn
/// go first. Cells in the last draw are never dropped.
const KEEP_LOADED: usize = 48;

/// A cell and the area it has data for. A folder of charts can hold
/// thousands of cells, far more than fit in memory, so a cell found in a
/// folder is known by its summary and read when it is on screen.
struct Entry {
    meta: Arc<CellSummary>,
    /// Where to read the cell again; `None` for one that was added already read.
    path: Option<PathBuf>,
    cell: RefCell<Option<Arc<Loaded>>>,
    /// The draw that last had this cell on screen.
    used: StdCell<u64>,
    loading: StdCell<bool>,
    failed: StdCell<bool>,
}

/// A cell in memory, with the bounds of each of its features, worked out
/// once when it is read rather than at every draw.
struct Loaded {
    cell: Cell,
    bounds: Vec<Bounds>,
    /// Its lights, which are weighed against each other to see which are shown.
    lights: Vec<Light>,
    names: Vec<CellName>,
    /// Its land areas, by index into the features.
    land: Vec<usize>,
    /// Its land contours.
    elev: Elev,
}

/// What a cell has of land elevation: whether it draws contour lines of its own (else the downloaded
/// relief supplies them), and the spacing of those, to tell the index contours.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Elev {
    lines: bool,
    interval: f64,
}

impl Elev {
    fn of(features: &[Feature]) -> Elev {
        let mut heights: Vec<f64> = features.iter().filter(|f| f.objl == obj::LNDELV && matches!(f.geom, Geom::Line(_))).filter_map(|f| f.num(attr::ELEVAT)).collect();
        if heights.is_empty() {
            return Elev::default();
        }
        heights.sort_by(f64::total_cmp);
        heights.dedup();
        let interval = heights.windows(2).map(|w| w[1] - w[0]).filter(|d| *d > 0.4).fold(f64::MAX, f64::min);
        Elev { lines: true, interval: if interval == f64::MAX { 0.0 } else { interval } }
    }

    /// Whether a contour at `metres` is an index contour: every fifth of the cell's spacing.
    fn is_index(&self, metres: f64) -> bool {
        self.interval > 0.0 && ((metres / (5.0 * self.interval)).round() * 5.0 * self.interval - metres).abs() < 1e-6
    }
}

impl Loaded {
    fn new(cell: Cell) -> Loaded {
        let bounds = cell.features.iter().map(feature_bounds).collect();
        let lights = cell
            .features
            .iter()
            .filter(|f| f.objl == obj::LIGHTS)
            .filter_map(|f| if let Geom::Point(p) = &f.geom { Some(light_of(f, *p)) } else { None })
            .collect();
        let names = names_of(&cell);
        let land = cell.features.iter().enumerate().filter(|(_, f)| f.objl == obj::LNDARE && matches!(f.geom, Geom::Area(_))).map(|(i, _)| i).collect();
        let elev = Elev::of(&cell.features);
        Loaded { cell, bounds, lights, names, land, elev }
    }

    /// Whether `p` is on one of its land areas.
    fn land_at(&self, p: [f64; 2]) -> bool {
        self.land.iter().any(|&i| {
            self.bounds[i].contains(p)
                && matches!(&self.cell.features[i].geom, Geom::Area(rings) if rings.iter().filter(|r| core_vector::ring_contains(r, p)).count() % 2 == 1)
        })
    }

    /// The features drawn, less coverage, that reach into `view`.
    fn features_in(&self, view: &Bounds) -> Vec<&Feature> {
        self.cell.features.iter().zip(&self.bounds).filter(|(f, b)| f.objl != obj::M_COVR && b.intersects(view)).map(|(f, _)| f).collect()
    }
}

/// The magnetic variation a chart cell states: degrees (east positive) in the year it was measured, how it
/// changes by the year in minutes, and that year.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChartVariation {
    pub value: f64,
    pub annual_change: Option<f64>,
    pub year: Option<f64>,
}

pub struct EncLayer {
    cells: RefCell<Vec<Entry>>,
    safety: StdCell<f64>,
    max_band: StdCell<Option<u8>>,
    changes: Changes,
    /// Where the changes the next `bump` shows are: `Some(None)` anywhere.
    touched: RefCell<Option<Option<Bounds>>>,
    redraw: Redraw,
    me: Weak<EncLayer>,
    draws: StdCell<u64>,
    reads: StdCell<usize>,
    /// While a folder of cells is being indexed (`data::load_enc`): cells keep arriving.
    indexing: StdCell<bool>,
    /// Cells the last draw needed and has not got, in drawing order.
    wanted: RefCell<Vec<PathBuf>>,
    last_bump: StdCell<Instant>,
    /// The view and scale of the last draw, to tell whether new cells matter to it.
    last_view: StdCell<Option<(Bounds, f64)>>,
    /// Cells were added that the view shows, but the redraw was held back.
    dirty: StdCell<bool>,
    /// How long the last draw took.
    last_cost: StdCell<Duration>,
    /// Hillshade, and contours where a cell has none of its own, over the cells' land.
    relief: RefCell<Option<Rc<ReliefLayer>>>,
}

impl EncLayer {
    pub fn new(redraw: Redraw) -> Rc<EncLayer> {
        Rc::new_cyclic(|me| EncLayer {
            cells: RefCell::new(Vec::new()),
            safety: StdCell::new(5.0),
            max_band: StdCell::new(None),
            changes: Changes::default(),
            touched: RefCell::new(None),
            redraw,
            me: me.clone(),
            draws: StdCell::new(0),
            reads: StdCell::new(0),
            indexing: StdCell::new(false),
            wanted: RefCell::new(Vec::new()),
            last_bump: StdCell::new(Instant::now()),
            last_view: StdCell::new(None),
            dirty: StdCell::new(false),
            last_cost: StdCell::new(Duration::ZERO),
            relief: RefCell::new(None),
        })
    }

    fn entry(meta: CellSummary, path: Option<PathBuf>, cell: Option<Cell>) -> Entry {
        Entry { meta: Arc::new(meta), path, cell: RefCell::new(cell.map(|c| Arc::new(Loaded::new(c)))), used: StdCell::new(0), loading: StdCell::new(false), failed: StdCell::new(false) }
    }

    /// Adds a cell already read; coarser scales are drawn first so finer ones cover them.
    #[cfg(test)]
    pub fn add_cell(&self, c: Cell) {
        let mut cells = self.cells.borrow_mut();
        let at = cells.partition_point(|e| e.meta.cscl >= c.cscl);
        self.touch(Some(c.summary().bounds));
        cells.insert(at, Self::entry(c.summary(), None, Some(c)));
        drop(cells);
        self.bump();
    }

    /// Adds cells known by their summary, to be read from `path` when they
    /// are drawn. A cell already there under the same name and update is skipped.
    pub fn add_indexed(&self, found: Vec<(PathBuf, CellSummary)>) {
        let mut cells = self.cells.borrow_mut();
        let mut seen: HashSet<(String, u32)> = cells.iter().map(|e| (e.meta.name.clone(), e.meta.update)).collect();
        let mut matters = false;
        let before = cells.len();
        for (path, meta) in found {
            if seen.insert((meta.name.clone(), meta.update)) {
                let shown = self.last_view.get().is_none_or(|(view, denom)| self.entry_visible(&meta, denom) && meta.bounds.intersects(&view));
                if shown {
                    self.touch(Some(meta.bounds));
                }
                matters |= shown;
                cells.push(Self::entry(meta, Some(path), None));
            }
        }
        if cells.len() == before {
            return;
        }
        cells.sort_by_key(|e| std::cmp::Reverse(e.meta.cscl));
        drop(cells);
        // A folder of thousands arrives in many batches; most are elsewhere, and
        // those that are not are shown together now and then.
        if !matters {
            return;
        }
        if self.may_redraw() {
            self.bump();
        } else {
            self.dirty.set(true);
        }
    }

    /// Shows what `add_indexed` held back; call when the cells have all been added.
    /// Notes that a folder is being indexed and its cells will arrive in batches.
    pub fn begin_indexing(&self) {
        self.indexing.set(true);
    }

    /// The index is complete: shows what arrived since the last batch shown.
    pub fn settle(&self) {
        self.indexing.set(false);
        if self.dirty.get() {
            self.bump();
        }
    }

    /// Drops the cells read from files under `dir`, as when that folder is deleted or replaced.
    pub fn remove_under(&self, dir: &Path) {
        let mut cells = self.cells.borrow_mut();
        let before = cells.len();
        cells.retain(|e| e.path.as_ref().is_none_or(|p| !p.starts_with(dir)));
        let changed = cells.len() != before;
        drop(cells);
        self.wanted.borrow_mut().retain(|p| !p.starts_with(dir));
        if changed {
            self.touch(None);
            self.bump();
        }
    }

    /// Shades the cells' land with this relief, and draws again where it changes.
    pub fn set_relief(&self, relief: Rc<ReliefLayer>) {
        let me = self.me.clone();
        relief.follow(move |area| {
            if let Some(enc) = me.upgrade() {
                enc.touch(area);
                enc.bump();
            }
        });
        *self.relief.borrow_mut() = Some(relief);
    }

    pub fn set_safety_depth(&self, metres: f64) {
        self.safety.set(metres.max(0.5));
        self.touch(None);
        self.bump();
    }

    /// Sets the maximum S-57 navigational purpose / usage band to display (1 to 6).
    /// `None` displays all bands up to their scale cutoffs.
    pub fn set_max_band(&self, max_band: Option<u8>) {
        let max_band = max_band.filter(|&b| (1..=6).contains(&b));
        if self.max_band.get() != max_band {
            self.max_band.set(max_band);
            self.touch(None);
            self.bump();
        }
    }

    pub fn max_band(&self) -> Option<u8> {
        self.max_band.get()
    }

    /// Whether an entry is visible at `denom` given its scale and usage band constraints.
    pub fn entry_visible(&self, meta: &CellSummary, denom: f64) -> bool {
        cell_summary_visible(meta, denom, self.max_band.get())
    }

    /// The magnetic variation a chart cell in memory gives at (`lon`, `lat`), from the finest cell that has any
    /// there: the `MAGVAR` area that holds the place, else the nearest `MAGVAR` point in the cell.
    pub fn variation_at(&self, lon: f64, lat: f64) -> Option<ChartVariation> {
        let cells = self.cells.borrow();
        for e in cells.iter().rev().filter(|e| e.meta.bounds.contains([lon, lat])) {
            let Some(cell) = e.cell.borrow().clone() else { continue };
            let mut best: Option<(f64, &Feature)> = None;
            for f in cell.cell.features.iter().filter(|f| f.objl == obj::MAGVAR && f.num(attr::VALMAG).is_some()) {
                let d = match &f.geom {
                    Geom::Area(rings) if rings.iter().filter(|r| core_vector::ring_contains(r, [lon, lat])).count() % 2 == 1 => 0.0,
                    Geom::Point(p) => (p[0] - lon).hypot(p[1] - lat) + 1e-6,
                    _ => continue,
                };
                if best.is_none_or(|(b, _)| d < b) {
                    best = Some((d, f));
                }
            }
            if let Some((_, f)) = best {
                return Some(ChartVariation { value: f.num(attr::VALMAG)?, annual_change: f.num(attr::VALACM), year: f.num(attr::RYRMGV) });
            }
        }
        None
    }

    /// Every cell known, coarse first, whether or not it is in memory.
    pub fn summaries(&self) -> Vec<CellSummary> {
        self.cells.borrow().iter().map(|e| (*e.meta).clone()).collect()
    }

    pub fn len(&self) -> usize {
        self.cells.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.borrow().is_empty()
    }

    /// Whether a cell has data at (`lon`, `lat`): inside its coverage, or its bounds when it has none.
    pub fn covers(&self, lon: f64, lat: f64) -> bool {
        self.cells.borrow().iter().any(|e| {
            e.meta.bounds.contains([lon, lat]) && (e.meta.cover.covered.is_empty() || e.meta.cover.contains([lon, lat]))
        })
    }

    /// The cell drawn on top at (`lon`, `lat`) at the view scale 1:`denom`, with the file it is read from.
    pub fn in_use_at(&self, lon: f64, lat: f64, denom: f64) -> Option<(CellSummary, Option<PathBuf>)> {
        let cells = self.cells.borrow();
        // Kept coarse first, so the last that shows here is drawn over the others.
        let e = cells.iter().rev().find(|e| {
            self.entry_visible(&e.meta, denom) && e.meta.bounds.contains([lon, lat]) && (e.meta.cover.covered.is_empty() || e.meta.cover.contains([lon, lat]))
        })?;
        Some(((*e.meta).clone(), e.path.clone()))
    }

    /// All cells covering (`lon`, `lat`) that are visible at `denom`, fine first.
    pub fn all_in_use_at(&self, lon: f64, lat: f64, denom: f64) -> Vec<(CellSummary, Option<PathBuf>)> {
        let cells = self.cells.borrow();
        cells
            .iter()
            .rev()
            .filter(|e| self.entry_visible(&e.meta, denom) && e.meta.bounds.contains([lon, lat]) && (e.meta.cover.covered.is_empty() || e.meta.cover.contains([lon, lat])))
            .map(|e| ((*e.meta).clone(), e.path.clone()))
            .collect()
    }

    /// Where each non-empty cell has data, with its scale.
    pub fn exclusions(&self) -> Vec<(Bounds, u32)> {
        self.cells.borrow().iter().filter(|e| !e.meta.bounds.is_empty()).map(|e| (e.meta.bounds, e.meta.cscl)).collect()
    }

    /// The cell in memory for `e`; if it is not, it is queued to be read.
    fn cell_of(&self, e: &Entry, wanted: &mut Vec<PathBuf>) -> Option<Arc<Loaded>> {
        if let Some(c) = e.cell.borrow().clone() {
            return Some(c);
        }
        if let Some(path) = &e.path
            && !e.loading.get()
            && !e.failed.get()
        {
            wanted.push(path.clone());
        }
        None
    }

    /// Starts reading queued cells, a few at a time.
    fn pump(&self) {
        while self.reads.get() < MAX_READS {
            let Some(path) = self.wanted.borrow_mut().pop() else { return };
            let cells = self.cells.borrow();
            let Some(e) = cells.iter().find(|e| e.path.as_deref() == Some(path.as_path())) else { continue };
            if e.loading.get() || e.failed.get() || e.cell.borrow().is_some() {
                continue;
            }
            e.loading.set(true);
            self.reads.set(self.reads.get() + 1);
            let me = self.me.clone();
            spawn(async move {
                let p = path.clone();
                let res = bg(move || load_cell(&p).map(Loaded::new).map_err(|e| e.to_string())).await;
                if let Some(layer) = me.upgrade() {
                    layer.arrived(&path, res);
                }
            });
        }
    }

    fn arrived(&self, path: &Path, res: Result<Loaded, String>) {
        self.reads.set(self.reads.get().saturating_sub(1));
        if let Some(e) = self.cells.borrow().iter().find(|e| e.path.as_deref() == Some(path)) {
            self.touch(Some(e.meta.bounds));
            e.loading.set(false);
            match res {
                Ok(c) => *e.cell.borrow_mut() = Some(Arc::new(c)),
                Err(_) => e.failed.set(true),
            }
        }
        self.evict();
        self.pump();
        // Drawing costs more with every cell in view, so many arriving one by
        // one are shown together, with a redraw now and then to show progress.
        if (self.reads.get() == 0 && self.wanted.borrow().is_empty()) || self.may_redraw() {
            self.bump();
        }
    }

    /// Drops the cells read from disk that were drawn longest ago, past `KEEP_LOADED`.
    fn evict(&self) {
        let cells = self.cells.borrow();
        let mut held: Vec<&Entry> = cells.iter().filter(|e| e.path.is_some() && e.cell.borrow().is_some()).collect();
        if held.len() <= KEEP_LOADED {
            return;
        }
        held.sort_by_key(|e| e.used.get());
        let surplus = held.len() - KEEP_LOADED;
        for e in held.into_iter().take(surplus).filter(|e| e.used.get() < self.draws.get()) {
            *e.cell.borrow_mut() = None;
        }
    }

    /// Whether enough time has passed since the last redraw to ask for another:
    /// never more than a quarter of the time is spent drawing.
    fn may_redraw(&self) -> bool {
        self.last_bump.get().elapsed() >= PROGRESS_EVERY.max(self.last_cost.get() * 4)
    }

    /// Notes a change within `area` (anywhere with `None`) for the next `bump`.
    fn touch(&self, area: Option<Bounds>) {
        let mut t = self.touched.borrow_mut();
        *t = match (t.take(), area) {
            (None, a) => Some(a),
            (Some(Some(mut b)), Some(a)) => {
                b.union(&a);
                Some(Some(b))
            }
            _ => Some(None),
        };
    }

    fn bump(&self) {
        self.dirty.set(false);
        self.last_bump.set(Instant::now());
        // A cell's labels and symbols run a little past it.
        let area = self.touched.take().flatten().map(|b| b.expanded(0.05 * b.width().max(b.height()) + 0.005));
        self.changes.bump(area);
        (self.redraw)();
    }
}

/// How far outside the view a traced line may run before it is cut short:
/// more than the widest stroke, so nothing drawn there can show.
const TRACE_MARGIN: f64 = 16.0;

/// Adds `pts` (lon, lat) to the path in screen coordinates. Cells hold their
/// lines at full detail and a depth area can reach far beyond the view, so
/// vertices less than half a pixel from the last one are skipped, and a run
/// of vertices that stays beyond one edge of the view becomes a straight line
/// between its ends, which stays beyond that edge too: fills and strokes look
/// the same on screen, though a dash pattern may start elsewhere.
pub(super) fn trace(vi: &ViewInfo, cr: &cairo::Context, pts: &[[f64; 2]], close: bool) {
    let (lo, hi_x, hi_y) = (-TRACE_MARGIN, vi.w + TRACE_MARGIN, vi.h + TRACE_MARGIN);
    // Which edges of the view a point is beyond, a bit each.
    let outside = |x: f64, y: f64| (x < lo) as u8 | ((x > hi_x) as u8) << 1 | ((y < lo) as u8) << 2 | ((y > hi_y) as u8) << 3;
    let Some((first, rest)) = pts.split_first() else { return };
    let proj = vi.projector();
    let (mut lx, mut ly) = proj.to_screen(&LatLon::new(first[1], first[0]));
    cr.move_to(lx, ly);
    // The edges every point since the last one drawn is beyond, and the latest of them.
    let mut run = outside(lx, ly);
    let mut skipped: Option<(f64, f64)> = None;
    let last = rest.len().saturating_sub(1);
    for (i, p) in rest.iter().enumerate() {
        let (x, y) = proj.to_screen(&LatLon::new(p[1], p[0]));
        let code = outside(x, y);
        if code & run != 0 {
            run &= code;
            skipped = Some((x, y));
            continue;
        }
        if skipped.is_none() && code == 0 && i != last && (x - lx).abs() < 0.5 && (y - ly).abs() < 0.5 {
            continue;
        }
        if let Some((sx, sy)) = skipped.take() {
            cr.line_to(sx, sy);
        }
        cr.line_to(x, y);
        (lx, ly, run) = (x, y, code);
    }
    if let Some((sx, sy)) = skipped {
        cr.line_to(sx, sy);
    }
    if close {
        cr.close_path();
    }
}

fn feature_bounds(f: &Feature) -> Bounds {
    match &f.geom {
        Geom::Point(p) => Bounds::of(&[*p]),
        Geom::Line(l) => Bounds::of(l),
        Geom::Area(r) => {
            let mut b = Bounds::EMPTY;
            for ring in r {
                b.union(&Bounds::of(ring));
            }
            b
        }
        Geom::Soundings(s) => Bounds::of(&s.iter().map(|p| [p[0], p[1]]).collect::<Vec<_>>()),
        Geom::None => Bounds::EMPTY,
    }
}

fn light_of(f: &Feature, pos: [f64; 2]) -> Light {
    let sector = f.num(attr::SECTR1).zip(f.num(attr::SECTR2));
    Light {
        pos: LatLon::new(pos[1], pos[0]),
        colours: f.list(attr::COLOUR),
        character: f.num(attr::LITCHR).map(|v| v as i32),
        group: f.attr(attr::SIGGRP).map(|s| s.to_string()),
        period_s: f.num(attr::SIGPER),
        range_nm: f.num(attr::VALNMR),
        sector,
        height_m: f.num(attr::HEIGHT),
    }
}

/// The anchorage, anchor berth or area where anchoring is prohibited or restricted a feature is, if it is one.
fn anchorage_of(f: &Feature) -> Option<Anchorage> {
    let kind = match f.objl {
        obj::ACHARE => AnchorageKind::Anchorage,
        obj::ACHBRT => AnchorageKind::Berth,
        obj::RESARE if f.list(attr::RESTRN).contains(&1) => AnchorageKind::Prohibited,
        obj::RESARE if f.list(attr::RESTRN).contains(&2) => AnchorageKind::Restricted,
        _ => return None,
    };
    let (pos, rings) = match &f.geom {
        Geom::Point(p) => (*p, Vec::new()),
        Geom::Area(rings) => (core_vector::label_point(rings)?, rings.clone()),
        _ => return None,
    };
    let edges = rings
        .iter()
        .map(|r| {
            let mut e = r.clone();
            if e.first() != e.last()
                && let Some(p) = e.first().copied()
            {
                e.push(p);
            }
            e
        })
        .collect();
    Some(Anchorage {
        kind,
        name: f.attr(attr::OBJNAM).map(|n| n.trim().to_string()),
        categories: if kind == AnchorageKind::Prohibited || kind == AnchorageKind::Restricted { Vec::new() } else { f.list(attr::CATACH) },
        pos: LatLon::new(pos[1], pos[0]),
        rings,
        edges,
        radius_m: f.num(attr::RADIUS),
    })
}

/// Whether the anchorage `a`, made from `f`, shows in a view at `zoom` of scale 1:`denom`: as the cell says
/// (SCAMIN), as S-52 has it, else from the zooms the OSM anchorages show at (`anchorages::shown`).
fn anchorage_shown(f: &Feature, a: &Anchorage, zoom: f64, denom: f64) -> bool {
    match f.num(attr::SCAMIN).filter(|s| *s > 0.0) {
        Some(s) => denom <= s,
        None => anchorages::shown(a, zoom),
    }
}

/// The buoy, beacon or landmark a feature is, if it is one.
fn aid_of(f: &Feature, pos: [f64; 2]) -> Option<Aid> {
    let kind = match f.objl {
        obj::BOYLAT => AidKind::BuoyLateral,
        obj::BOYCAR => AidKind::BuoyCardinal,
        obj::BOYISD => AidKind::BuoyIsolatedDanger,
        obj::BOYSAW => AidKind::BuoySafeWater,
        obj::BOYSPP => AidKind::BuoySpecial,
        obj::BOYINB => AidKind::BuoyInstallation,
        obj::BCNLAT => AidKind::BeaconLateral,
        obj::BCNCAR => AidKind::BeaconCardinal,
        obj::BCNISD => AidKind::BeaconIsolatedDanger,
        obj::BCNSAW => AidKind::BeaconSafeWater,
        obj::BCNSPP => AidKind::BeaconSpecial,
        obj::LNDMRK => AidKind::Landmark,
        _ => return None,
    };
    let category = match kind {
        AidKind::BuoyLateral | AidKind::BeaconLateral => f.num(attr::CATLAM),
        AidKind::BuoyCardinal | AidKind::BeaconCardinal => f.num(attr::CATCAM),
        _ => None,
    };
    Some(Aid {
        pos: LatLon::new(pos[1], pos[0]),
        kind,
        name: f.attr(attr::OBJNAM).map(str::to_string),
        colours: f.list(attr::COLOUR),
        shape: f.num(attr::BOYSHP).or_else(|| f.num(attr::BCNSHP)).map(|v| v as i32),
        category: category.map(|v| v as i32),
    })
}

impl Layer for EncLayer {
    fn id(&self) -> &'static str {
        "enc"
    }

    fn revision(&self) -> u64 {
        self.changes.get()
    }

    fn changes_since(&self, revision: u64) -> Option<Vec<super::Change>> {
        self.changes.since(revision)
    }

    fn loading(&self) -> bool {
        self.indexing.get() || self.reads.get() > 0 || !self.wanted.borrow().is_empty()
    }

    fn prepare(&self, area: &TileArea) -> Option<Arc<dyn TilePaint>> {
        let started = Instant::now();
        let paint = self.prepare_cells(area);
        self.last_cost.set(started.elapsed());
        paint
    }
}

/// A cell ready to draw: what it covers, its features, and which of its lights are shown.
struct Ready {
    meta: Arc<CellSummary>,
    cell: Arc<Loaded>,
    declutter: Declutter,
}

/// The chart cells the tiles of an area are drawn from, coarse first.
struct EncPaint {
    cells: Vec<Ready>,
    safety: f64,
    /// The scale denominator of the view, which says which cells show their soundings.
    denom: f64,
    /// The view's larger side, in pixels: an area wider than that is not named.
    view_px: f64,
    /// The heights to shade the land with.
    relief: Option<Arc<ReliefPaint>>,
}

impl EncLayer {
    fn prepare_cells(&self, area: &TileArea) -> Option<Arc<dyn TilePaint>> {
        let cells = self.cells.borrow();
        if cells.is_empty() {
            return None;
        }
        let vi = &area.vi;
        let view = vi.bounds();
        let denom = view_scale(vi.view.center.lat, vi.zoom());
        let safety = self.safety.get();
        self.last_view.set(Some((view, denom)));
        let mut shown: Vec<&Entry> = cells.iter().filter(|e| self.entry_visible(&e.meta, denom) && e.meta.bounds.intersects(&view)).collect();
        if shown.is_empty() {
            return None;
        }
        // Cells are kept coarse first, so the extra ones are at the front.
        shown.drain(..shown.len().saturating_sub(MAX_DRAWN));
        // Cells not yet read are asked for and left out; each one that
        // arrives redraws the layer. Until then a coarser cell stays visible.
        let draw_no = self.draws.get() + 1;
        self.draws.set(draw_no);
        let mut wanted = Vec::new();
        let ready: Vec<Ready> = shown
            .iter()
            .filter_map(|e| {
                e.used.set(draw_no);
                self.cell_of(e, &mut wanted).map(|cell| {
                    // Each cell decides which of its lights show once for the area, so every tile agrees.
                    let declutter = Declutter::new(vi, cell.lights.iter().filter(|l| view.contains([l.pos.lon, l.pos.lat])));
                    Ready { meta: e.meta.clone(), cell, declutter }
                })
            })
            .collect();
        // The coarsest is popped last, so the finest, which show most, arrive first.
        *self.wanted.borrow_mut() = wanted;
        self.pump();
        let relief = self.relief.borrow().as_ref().and_then(|r| r.prepare(area));
        Some(Arc::new(EncPaint { cells: ready, safety, denom, view_px: area.view.w.max(area.view.h), relief }))
    }
}

impl TilePaint for EncPaint {
    fn paint(&self, ctx: &DrawCtx) {
        let (cr, vi) = (ctx.cr, &ctx.vi);
        let view = vi.bounds();
        // Soundings are thinned by the ones near them, so those up to the widest radius past the edge count.
        let wide = view.expanded(sounding_reach() * vi.deg_per_px());
        // Names are placed first, so that soundings keep clear of them.
        // Placed as at the tile's `names::label_zoom`, the same up to the next step, and grown with the chart.
        let (at_step, k) = names::at_label_zoom(vi);
        let step = plan_names(&DrawCtx { vi: at_step, ..*ctx }, &self.cells, self.denom * k, self.view_px);
        let plan = NamePlan {
            placed: step.placed.iter().filter_map(|p| p.scaled(cr, &ctx.pal, k)).collect(),
            paths: step.paths.iter().map(|l| Arc::new(l.at_zoom(vi.zoom()))).collect(),
            boxes: step.boxes.iter().map(|b| b.map(|v| v * k)).collect(),
        };
        let boxes: Vec<[f64; 4]> = plan.placed.iter().map(Placed::rect).chain(plan.boxes.iter().copied()).collect();
        // Cells are drawn coarse to fine, each only where it has data and
        // after clearing what coarser cells drew there, as a chart viewer
        // does. The group keeps the clearing from touching the layers below.
        let _ = cr.save();
        cr.push_group();
        for rc in &self.cells {
            if !rc.meta.bounds.intersects(&view) {
                continue;
            }
            let vis = rc.cell.features_in(&view);
            let mask = coverage_mask(ctx, &rc.meta);
            if let Some(mask) = &mask {
                let _ = cr.save();
                cr.set_operator(cairo::Operator::DestOut);
                let _ = cr.mask(mask);
                let _ = cr.restore();
            }
            cr.push_group();
            let show_soundings = soundings_visible(rc.meta.cscl, self.denom, vi.zoom());
            let around = rc.cell.features_in(&wide);
            draw_cell(ctx, &rc.meta, &rc.declutter, &vis, &around, self.safety, show_soundings, &boxes, self.denom, self.relief.as_deref(), &rc.cell.elev);
            if let Ok(drawn) = cr.pop_group() {
                let _ = cr.set_source(&drawn);
                match &mask {
                    Some(mask) => {
                        let _ = cr.mask(mask);
                    }
                    None => {
                        let _ = cr.paint();
                    }
                }
            }
        }
        if let Ok(all) = cr.pop_group() {
            let _ = cr.set_source(&all);
            let _ = cr.paint();
        }
        // After the cells, so a finer cell's clearing does not cut a coarser cell's names.
        names::draw_paths(cr, &ctx.pal, vi, &plan.paths);
        names::draw(cr, &ctx.pal, &plan.placed);
        if crate::debug::coverage() {
            draw_coverage(vi, cr, &self.cells);
        }
        let _ = cr.restore();
    }
}

#[cfg(test)]
mod tests;
