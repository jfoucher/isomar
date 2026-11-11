//! BSB/KAP raster charts. Only headers are read at start; an image is decoded
//! in the background when the view first needs it, and the least recently
//! used images are dropped to keep memory in check.

use super::land::LandLayer;
use super::{Changes, DrawCtx, Layer, Redraw, TileArea, TilePaint};
use crate::core::chart::bsb::{Georef, Header, Image, decode_image, parse_header};
use crate::core::geo::LatLon;
use crate::core::vector::{Bounds, ring_contains};
use crate::palette::{Palette, Tone};
use crate::view::ViewInfo;
use crate::platform::{bg, spawn};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;
use std::sync::Arc;

/// Decoded images kept in memory at once, besides any the view is using.
const MAX_LOADED: usize = 5;
/// Points across the view that decide which charts are worth drawing.
const PLAN_COLUMNS: usize = 16;
const PLAN_ROWS: usize = 10;

/// What the header says about a chart: plain data, readable on any thread.
pub struct ChartInfo {
    pub path: PathBuf,
    pub header: Header,
    pub georef: Georef,
    pub bounds: Bounds,
}

/// A chart together with its (lazily decoded) image.
pub struct Chart {
    info: Arc<ChartInfo>,
    image: RefCell<Option<Arc<Image>>>,
    /// The image shrunk, in the colours of the tone it was made for.
    mips: RefCell<Option<Arc<Mips>>>,
    loading: Cell<bool>,
    /// Decoding failed; do not try again on every redraw.
    failed: Cell<bool>,
    last_used: Cell<u64>,
}

impl std::ops::Deref for Chart {
    type Target = ChartInfo;
    fn deref(&self) -> &ChartInfo {
        &self.info
    }
}

impl Chart {
    pub fn new(info: ChartInfo) -> Chart {
        Chart { info: Arc::new(info), image: RefCell::new(None), mips: RefCell::new(None), loading: Cell::new(false), failed: Cell::new(false), last_used: Cell::new(0) }
    }

    #[cfg(test)]
    pub fn open(path: PathBuf) -> Result<Chart, String> {
        ChartInfo::open(path).map(Chart::new)
    }
}

impl ChartInfo {
    /// Opens a chart's header. Reads only as much of the file as the header needs.
    pub fn open(path: PathBuf) -> Result<ChartInfo, String> {
        use std::io::Read;
        let mut f = std::fs::File::open(&path).map_err(|e| e.to_string())?;
        let mut buf = Vec::new();
        let mut chunk = vec![0u8; 64 * 1024];
        // The header ends at the byte pair 1A 00; read until it appears.
        loop {
            let n = f.read(&mut chunk).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if crate::core::chart::bsb::header_end(&buf).is_some() || buf.len() > 4 * 1024 * 1024 {
                break;
            }
        }
        let header = parse_header(&buf).map_err(|e| e.to_string())?;
        let georef = Georef::new(&header).map_err(|e| e.to_string())?;
        let bounds = header.bounds().ok_or("the chart has no extent")?;
        Ok(ChartInfo { path, header, georef, bounds })
    }
}

/// A raster chart is shown from a little coarser than its scale down to far
/// past it (it simply gets blocky when overzoomed).
pub fn scale_suits(chart_scale: u32, view_denominator: f64) -> bool {
    chart_scale == 0 || (view_denominator <= chart_scale as f64 * 6.0 && view_denominator >= chart_scale as f64 / 12.0)
}

/// With a vector base map under it, a raster chart is shown only up to this
/// far past its scale: beyond that the base is the better map.
const MAX_OVERZOOM_OVER_BASE: f64 = 2.5;

/// Charts at or finer than this have text and detail the base map does not,
/// so they stay up further past their scale. A scan is one pixel per 0.1 mm
/// of paper, which is one screen pixel at about 0.38 times its scale: 1 / 6
/// keeps it until its pixels are blown up a little over twice.
const DETAILED_SCALE: u32 = 100_000;
const MAX_OVERZOOM_DETAILED_OVER_BASE: f64 = 6.0;

/// Whether a raster chart beats the vector base map at this view scale: near
/// its own scale, not blown up and not far finer than the view needs.
pub fn scale_suits_over_base(chart_scale: u32, view_denominator: f64) -> bool {
    let overzoom = if chart_scale <= DETAILED_SCALE { MAX_OVERZOOM_DETAILED_OVER_BASE } else { MAX_OVERZOOM_OVER_BASE };
    chart_scale == 0 || (view_denominator <= chart_scale as f64 * 6.0 && view_denominator >= chart_scale as f64 / overzoom)
}

/// How long the base map must stop covering the view before the charts drawn over it change.
const UNCOVERED_WAIT: std::time::Duration = std::time::Duration::from_millis(1500);

pub struct BsbLayer {
    /// The downloaded land, when it covers the view, makes a chart that does
    /// not suit give way to it.
    base: RefCell<Option<Rc<LandLayer>>>,
    charts: RefCell<Vec<Rc<Chart>>>,
    clock: Cell<u64>,
    /// Charts added and removed, and images arriving, each where its chart is.
    changes: Rc<Changes>,
    /// Whether the base map covered the area last prepared: when that changes, so do the charts drawn.
    over_base: Cell<Option<bool>>,
    /// Since when the base map has not covered the area, while `over_base` still says it does.
    uncovered_since: Cell<Option<Instant>>,
    /// Where the charts are, made again after charts come or go.
    index: RefCell<Option<Rc<Index>>>,
    redraw: Redraw,
}

impl BsbLayer {
    pub fn new(redraw: Redraw) -> BsbLayer {
        BsbLayer { base: RefCell::new(None), charts: RefCell::new(Vec::new()), clock: Cell::new(0), changes: Rc::default(), over_base: Cell::new(None), uncovered_since: Cell::new(None), index: RefCell::new(None), redraw }
    }

    pub fn set_base(&self, land: Rc<LandLayer>) {
        *self.base.borrow_mut() = Some(land);
    }

    pub fn add_chart(&self, info: ChartInfo) {
        let mut charts = self.charts.borrow_mut();
        self.changes.bump(Some(info.bounds));
        self.index.take();
        charts.push(Rc::new(Chart::new(info)));
        // Coarse scales first so finer charts cover them.
        charts.sort_by_key(|c| std::cmp::Reverse(c.header.scale));
        drop(charts);
        (self.redraw)();
    }

    /// Drops the charts read from files under `dir`, as when that folder is deleted or replaced.
    pub fn remove_under(&self, dir: &std::path::Path) {
        let mut charts = self.charts.borrow_mut();
        let before = charts.len();
        charts.retain(|c| !c.path.starts_with(dir));
        let changed = charts.len() != before;
        self.index.take();
        drop(charts);
        if changed {
            self.changes.bump(None);
            (self.redraw)();
        }
    }

    /// The chart drawn at `p` in the view `vi`, as `prepare` picks it.
    pub fn in_use_at(&self, vi: &ViewInfo, p: LatLon) -> Option<Rc<Chart>> {
        let denom = super::enc::view_scale(vi.view.center.lat, vi.zoom());
        let base = self.base.borrow().as_ref().is_some_and(|l| l.covers_view(vi));
        let charts = self.charts.borrow();
        plan_in(&charts, &self.index(), &[p], denom, base).pop()
    }

    fn index(&self) -> Rc<Index> {
        self.index.borrow_mut().get_or_insert_with(|| Rc::new(Index::new(&self.charts.borrow()))).clone()
    }

    pub fn charts(&self) -> Vec<Rc<Chart>> {
        self.charts.borrow().clone()
    }

    /// Decodes the chart's image if it is not, and shrinks it in the colours of `pal`.
    fn request(&self, chart: &Rc<Chart>, pal: Palette) {
        if chart.failed.get() || chart.loading.replace(true) {
            return;
        }
        let (chart, redraw, changes) = (chart.clone(), self.redraw.clone(), self.changes.clone());
        spawn(async move {
            let (info, img) = (chart.info.clone(), chart.image.borrow().clone());
            let done = bg(move || {
                let img = match img {
                    Some(img) => img,
                    None => Arc::new(std::fs::read(&info.path).ok().and_then(|b| decode_image(&b, &info.header).ok())?),
                };
                let mips = Mips::new(&img, &colours(&info, &pal), pal.tone);
                Some((img, mips))
            })
            .await;
            chart.failed.set(done.is_none());
            if let Some((img, mips)) = done {
                *chart.image.borrow_mut() = Some(img);
                *chart.mips.borrow_mut() = Some(Arc::new(mips));
            }
            chart.loading.set(false);
            if crate::debug::timing() {
                isomar_core::log!("isomar: bsb image of {} ready ({:?})", chart.info.path.display(), chart.bounds);
            }
            changes.bump(Some(chart.bounds));
            redraw();
        });
    }

    /// Drops the images used longest ago beyond the limit, never one the
    /// current drawing (`clock`) used: dropping those would only make the
    /// next redraw decode them again.
    fn evict(&self, clock: u64) {
        let charts = self.charts.borrow();
        let mut loaded: Vec<&Rc<Chart>> = charts.iter().filter(|c| c.image.borrow().is_some()).collect();
        let over = loaded.len().saturating_sub(MAX_LOADED);
        loaded.retain(|c| c.last_used.get() != clock);
        loaded.sort_by_key(|c| c.last_used.get());
        for c in loaded.into_iter().take(over) {
            *c.image.borrow_mut() = None;
            *c.mips.borrow_mut() = None;
        }
    }
}

impl Layer for BsbLayer {
    fn id(&self) -> &'static str {
        "bsb"
    }

    fn revision(&self) -> u64 {
        self.changes.get()
    }

    fn changes_since(&self, revision: u64) -> Option<Vec<super::Change>> {
        self.changes.since(revision)
    }

    fn prepare(&self, area: &TileArea) -> Option<Arc<dyn TilePaint>> {
        let vi = &area.vi;
        let denom = super::enc::view_scale(vi.view.center.lat, vi.zoom());
        self.clock.set(self.clock.get() + 1);
        let clock = self.clock.get();
        let charts = self.charts.borrow().clone();
        let covered = self.base.borrow().as_ref().is_some_and(|l| l.covers_view(&area.view));
        // The base map stops covering the view for a moment whenever a pan or a zoom shows land tiles not
        // read yet: changing the charts then drew every tile under a small-scale chart again (the whole
        // Atlantic for a pilot chart), twice. It must stay uncovered a while before the charts change.
        let base = if covered || self.over_base.get() != Some(true) {
            self.uncovered_since.set(None);
            covered
        } else {
            let since = self.uncovered_since.get().unwrap_or_else(Instant::now);
            self.uncovered_since.set(Some(since));
            since.elapsed() < UNCOVERED_WAIT
        };
        if self.over_base.replace(Some(base)).is_some_and(|b| b != base) {
            if crate::debug::timing() {
                isomar_core::log!("isomar: bsb base map now covers the view: {base}");
            }
            // Tiles of this zoom drawn before chose their charts the other way; they are drawn again.
            self.changes.bump_change(Some(super::Change { area: vi.bounds(), zooms: Some((vi.zoom(), vi.zoom())) }));
        }
        let mut ready = Vec::new();
        for chart in plan_in(&charts, &self.index(), &sample_points(vi), denom, base) {
            chart.last_used.set(clock);
            let img = chart.image.borrow().clone();
            let mips = chart.mips.borrow().clone().filter(|m| m.tone == area.pal.tone);
            if img.is_none() || mips.is_none() {
                self.request(&chart, area.pal);
            }
            let Some(img) = img else {
                continue;
            };
            ready.push((chart.info.clone(), img, mips));
        }
        self.evict(clock);
        (!ready.is_empty()).then(|| Arc::new(BsbPaint { charts: ready }) as Arc<dyn TilePaint>)
    }
}

/// The charts to draw, coarse first, with their decoded images and, once made, their shrunk ones.
struct BsbPaint {
    charts: Vec<(Arc<ChartInfo>, Arc<Image>, Option<Arc<Mips>>)>,
}

impl TilePaint for BsbPaint {
    fn paint(&self, ctx: &DrawCtx) {
        let cr = ctx.cr;
        // Only what the target shows: a tile's margin is there for marks and labels that straddle its
        // edge, and working out a raster's pixels there was more than half the work.
        let (x0, y0, x1, y1) = cr.clip_extents().unwrap_or((0.0, 0.0, ctx.vi.w, ctx.vi.h));
        let area = [x0.max(0.0).floor(), y0.max(0.0).floor(), x1.min(ctx.vi.w).ceil(), y1.min(ctx.vi.h).ceil()];
        if area[2] <= area[0] || area[3] <= area[1] {
            return;
        }
        let scale = cr.target().device_scale().0.max(1.0);
        let mut covered = Covered::new(area, scale);
        // The finest first: a pixel a finer chart fills is not worked out again for the coarser ones under it.
        let mut drawn: Vec<Drawn> = self.charts.iter().rev().filter_map(|(chart, img, mips)| render(ctx, chart, img, mips.as_deref(), &mut covered)).collect();
        // Drawn coarse first, so the finer cover them where blocks meet.
        drawn.reverse();
        for d in drawn {
            let _ = cr.save();
            if cr.set_source_surface(&d.surface, d.x, d.y).is_ok() {
                cr.source().set_filter(if d.smooth { cairo::Filter::Bilinear } else { cairo::Filter::Nearest });
                let _ = cr.paint();
            }
            let _ = cr.restore();
        }
    }
}

/// Which device pixels of the area a finer chart has filled.
struct Covered {
    /// The area, in points: x0, y0, x1, y1.
    area: [f64; 4],
    scale: f64,
    w: usize,
    h: usize,
    px: Vec<bool>,
}

impl Covered {
    fn new(area: [f64; 4], scale: f64) -> Covered {
        let (w, h) = (((area[2] - area[0]) * scale).ceil() as usize, ((area[3] - area[1]) * scale).ceil() as usize);
        Covered { area, scale, w, h, px: vec![false; w * h] }
    }

    /// The device pixels of the block of `n` by `n` with its top-left corner at (`x`, `y`) points that are in
    /// the area, as rows: (first index, how many).
    fn rows(&self, x: f64, y: f64, n: usize) -> impl Iterator<Item = (usize, usize)> + use<> {
        let (i, j) = (((x - self.area[0]) * self.scale).round() as isize, ((y - self.area[1]) * self.scale).round() as isize);
        let (a0, a1) = (i.max(0) as usize, ((i + n as isize).max(0) as usize).min(self.w));
        let (b0, b1) = (j.max(0) as usize, ((j + n as isize).max(0) as usize).min(self.h));
        let w = self.w;
        (b0..b1).filter(move |_| a1 > a0).map(move |b| (b * w + a0, a1 - a0))
    }

    /// Whether the block is in the area and filled already.
    fn all(&self, x: f64, y: f64, n: usize) -> bool {
        let mut rows = self.rows(x, y, n).peekable();
        rows.peek().is_some() && rows.all(|(k, len)| self.px[k..k + len].iter().all(|p| *p))
    }

    fn fill(&mut self, x: f64, y: f64, n: usize) {
        for (k, len) in self.rows(x, y, n) {
            self.px[k..k + len].fill(true);
        }
    }
}

/// A chart's part of a tile, ready to paint.
struct Drawn {
    surface: cairo::ImageSurface,
    x: f64,
    y: f64,
    smooth: bool,
}

/// An even grid of positions over the view.
fn sample_points(vi: &ViewInfo) -> Vec<LatLon> {
    let mut out = Vec::with_capacity(PLAN_COLUMNS * PLAN_ROWS);
    for j in 0..PLAN_ROWS {
        for i in 0..PLAN_COLUMNS {
            out.push(vi.position_at(vi.w * (i as f64 + 0.5) / PLAN_COLUMNS as f64, vi.h * (j as f64 + 0.5) / PLAN_ROWS as f64));
        }
    }
    out
}

/// Whether the chart's coverage includes a position, whichever side of the
/// antimeridian its longitude is written.
fn covers(c: &ChartInfo, p: &LatLon) -> bool {
    let lon = p.lon + 360.0 * (((c.bounds.min_lon + c.bounds.max_lon) / 2.0 - p.lon) / 360.0).round();
    if !c.bounds.contains([lon, p.lat]) {
        return false;
    }
    c.header.polygon.len() < 3 || ring_contains(&c.header.polygon, [lon, p.lat])
}

/// How a chart's scale sits against the view's, worst first.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Fit {
    /// Far more detailed than the view needs.
    TooFine,
    /// Blown up far past its scale.
    TooCoarse,
    Suits,
}

/// Higher is drawn later, so on top, and wins the position it covers.
fn rank(scale: u32, denom: f64) -> (Fit, u64) {
    let finer_is_better = u64::MAX - if scale == 0 { u64::MAX / 2 } else { scale as u64 };
    if scale_suits(scale, denom) {
        (Fit::Suits, finer_is_better)
    } else if denom > scale as f64 {
        (Fit::TooFine, scale as u64)
    } else {
        (Fit::TooCoarse, finer_is_better)
    }
}

/// The charts to draw, in drawing order. Every position gets the finest chart
/// whose scale suits the view; where none does, the one nearest in scale, so
/// zooming in or out never leaves a hole where a chart exists. A chart that
/// wins no position is left out, so it is not even decoded. Where `base` (the
/// vector map) covers the view, only charts that suit it are drawn at all.
#[cfg(test)]
fn plan(charts: &[Rc<Chart>], samples: &[LatLon], denom: f64, base: bool) -> Vec<Rc<Chart>> {
    plan_in(charts, &Index::new(charts), samples, denom, base)
}

/// Which charts reach into each degree of latitude and longitude, so that a position is tested against
/// those, not against every chart of a folder of thousands.
struct Index {
    cells: HashMap<(i32, i32), Vec<usize>>,
    /// Charts over so many degrees that they are tested everywhere.
    wide: Vec<usize>,
}

impl Index {
    fn new(charts: &[Rc<Chart>]) -> Index {
        let mut index = Index { cells: HashMap::new(), wide: Vec::new() };
        for (i, c) in charts.iter().enumerate() {
            let b = c.bounds;
            let (x0, x1, y0, y1) = (b.min_lon.floor() as i32, b.max_lon.floor() as i32, b.min_lat.floor() as i32, b.max_lat.floor() as i32);
            if (x1 - x0 + 1) as i64 * (y1 - y0 + 1) as i64 > 256 {
                index.wide.push(i);
                continue;
            }
            for y in y0..=y1 {
                for x in x0..=x1 {
                    // Written past 180 across the antimeridian: filed where a position's longitude falls.
                    index.cells.entry(((x + 180).rem_euclid(360) - 180, y)).or_default().push(i);
                }
            }
        }
        index
    }

    /// The charts that may cover `p`, in their order.
    fn at(&self, p: &LatLon) -> Vec<usize> {
        let key = (p.lon.floor() as i32, p.lat.floor() as i32);
        let mut out: Vec<usize> = self.cells.get(&key).map_or_else(Vec::new, |v| v.clone());
        out.extend(&self.wide);
        out.sort_unstable();
        out
    }
}

fn plan_in(charts: &[Rc<Chart>], index: &Index, samples: &[LatLon], denom: f64, base: bool) -> Vec<Rc<Chart>> {
    let mut chosen: Vec<(usize, (Fit, u64))> = Vec::new();
    for p in samples {
        let best = index
            .at(p)
            .into_iter()
            .map(|i| (i, &charts[i]))
            .filter(|(_, c)| covers(c, p) && (!base || scale_suits_over_base(c.header.scale, denom)))
            .map(|(i, c)| (i, rank(c.header.scale, denom)))
            .max_by_key(|(_, r)| *r);
        if let Some(b) = best
            && !chosen.iter().any(|(i, _)| *i == b.0)
        {
            chosen.push(b);
        }
    }
    chosen.sort_by_key(|(_, r)| *r);
    chosen.into_iter().map(|(i, _)| charts[i].clone()).collect()
}

/// What `sample` reads: a chart's own image through its colour table, or a level of its `Mips`.
trait Pixels {
    fn size(&self) -> (usize, usize);
    /// The colour at (`x`, `y`), which are within the size, with its weight (`inked`) in the top byte.
    fn at(&self, x: usize, y: usize) -> Option<u32>;
}

/// A chart's image and the colours of its palette indices, as `colours` makes them.
struct Indexed<'a> {
    img: &'a Image,
    lut: &'a [u32],
}

impl Pixels for Indexed<'_> {
    fn size(&self) -> (usize, usize) {
        (self.img.width, self.img.height)
    }

    fn at(&self, x: usize, y: usize) -> Option<u32> {
        self.lut.get(self.img.at(x, y) as usize).copied()
    }
}

/// The colour table of a chart in `pal`'s tone: its own palette for that tone if it has one, else
/// its day colours dimmed.
fn colours(chart: &ChartInfo, pal: &Palette) -> Vec<u32> {
    let key = match pal.tone {
        Tone::Day => "DAY",
        Tone::Dusk => "DSK",
        Tone::Night => "NGT",
    };
    let own = chart.header.palettes.contains_key(key);
    chart
        .header
        .palette(if own { key } else { "DAY" })
        .iter()
        .map(|c| {
            let (r, g, b) = (c[0] as f64 / 255.0, c[1] as f64 / 255.0, c[2] as f64 / 255.0);
            let (r, g, b) = if own || pal.tone == Tone::Day { (r, g, b) } else { pal.c((r, g, b)) };
            inked((r * 255.0) as u32, (g * 255.0) as u32, (b * 255.0) as u32)
        })
        .collect()
}

/// A colour with how much it counts when averaged in its top byte, from 0 for white (a weight of
/// one) to 255 for black (one plus `INK_WEIGHT`).
fn inked(r: u32, g: u32, b: u32) -> u32 {
    let dark = 1.0 - (0.3 * r as f64 + 0.59 * g as f64 + 0.11 * b as f64) / 255.0;
    (((dark * dark * 255.0).round() as u32) << 24) | (r << 16) | (g << 8) | b
}

fn weight(c: u32) -> f64 {
    1.0 + INK_WEIGHT * (c >> 24) as f64 / 255.0
}

const OPAQUE: u32 = 0xFF00_0000;

/// Colour of the image at (`px`, `py`), where one screen pixel covers `span`
/// source pixels. Zoomed in, the nearest four are blended, with the blend
/// squeezed towards the nearest so a chart enlarged past its resolution is
/// smooth without going soft. Zoomed out, a block of them is averaged
/// (`average`).
fn sample(src: &impl Pixels, px: f64, py: f64, span: f64) -> Option<u32> {
    let (w, h) = src.size();
    let (w, h) = (w as f64, h as f64);
    let at = |x: f64, y: f64| src.at(x.clamp(0.0, w - 1.0) as usize, y.clamp(0.0, h - 1.0) as usize);
    if span < 0.9 {
        let (fx, fy) = (px - 0.5, py - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        // The more it is enlarged, the narrower the blend, but never narrower than about a pixel and a half.
        let sharpen = (1.0 / span / 1.5).clamp(1.0, 3.0);
        let squeeze = |t: f64| ((t - 0.5) * sharpen + 0.5).clamp(0.0, 1.0);
        let (tx, ty) = (squeeze(fx - x0), squeeze(fy - y0));
        let taps = [(at(x0, y0)?, (1.0 - tx) * (1.0 - ty)), (at(x0 + 1.0, y0)?, tx * (1.0 - ty)), (at(x0, y0 + 1.0)?, (1.0 - tx) * ty), (at(x0 + 1.0, y0 + 1.0)?, tx * ty)];
        let ch = |shift: u32| taps.iter().map(|(c, wt)| ((c >> shift) & 0xff) as f64 * wt).sum::<f64>().round() as u32;
        return Some(OPAQUE | (ch(16) << 16) | (ch(8) << 8) | ch(0));
    }
    if span <= 1.5 {
        return at(px, py).map(|c| OPAQUE | c);
    }
    average(src, px, py, span)
}

/// The colour of the `span` by `span` square of the image round (`px`, `py`), from a few points
/// across it, the dark ones counting for more: taking one pixel would keep a few stray text and
/// line pixels and lose the rest, and a plain average turns black text on a pale ground into grey
/// smudges.
fn average(src: &impl Pixels, px: f64, py: f64, span: f64) -> Option<u32> {
    let (w, h) = src.size();
    let (w, h) = (w as f64, h as f64);
    let n = (span.ceil() as usize).clamp(2, 6);
    let (mut sum, mut weights) = ([0.0f64; 3], 0.0f64);
    for j in 0..n {
        for i in 0..n {
            let qx = px + ((i as f64 + 0.5) / n as f64 - 0.5) * span;
            let qy = py + ((j as f64 + 0.5) / n as f64 - 0.5) * span;
            if qx < 0.0 || qy < 0.0 || qx >= w || qy >= h {
                continue;
            }
            if let Some(c) = src.at(qx as usize, qy as usize) {
                let weight = weight(c);
                sum[0] += ((c >> 16) & 0xff) as f64 * weight;
                sum[1] += ((c >> 8) & 0xff) as f64 * weight;
                sum[2] += (c & 0xff) as f64 * weight;
                weights += weight;
            }
        }
    }
    let channel = |v: f64| (v / weights).round() as u32;
    (weights > 0.0).then(|| OPAQUE | (channel(sum[0]) << 16) | (channel(sum[1]) << 8) | channel(sum[2]))
}

/// How much more black counts than white when pixels are averaged: a pixel of
/// pure black weighs this plus one.
const INK_WEIGHT: f64 = 2.0;

/// A chart's image halved and halved again, from a quarter of its size down, in the colours of one
/// tone, each pixel the ink-weighted average of the image's pixels under it. A chart shrunk to a
/// tenth is read from the level a few of its pixels to a screen pixel: averaging a hundred of the
/// image's own for every screen pixel was too slow, so blocks of screen pixels were worked out and
/// smoothed together, and fine charts went soft (a quarter of the image's memory).
struct Mips {
    tone: Tone,
    levels: Vec<Level>,
}

/// A level of `Mips`: colours with their mean weight in the top byte, as `inked` puts it.
struct Level {
    width: usize,
    height: usize,
    px: Vec<u32>,
}

impl Pixels for Level {
    fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    fn at(&self, x: usize, y: usize) -> Option<u32> {
        Some(self.px[y * self.width + x])
    }
}

/// How small the last level of `Mips` gets.
const MIPS_SMALLEST: usize = 64;

impl Mips {
    fn new(img: &Image, lut: &[u32], tone: Tone) -> Mips {
        let mut levels = vec![Level::shrink(&Indexed { img, lut }, 4)];
        while let Some(last) = levels.last().filter(|l| l.width.max(l.height) > MIPS_SMALLEST) {
            let next = Level::shrink(last, 2);
            levels.push(next);
        }
        Mips { tone, levels }
    }

    /// The level to read when one screen pixel covers `span` of the image's, with how many of the
    /// image's pixels its own are across; none under four, where the image itself is read.
    fn level(&self, span: f64) -> Option<(&Level, f64)> {
        if span < 4.0 {
            return None;
        }
        let i = (span.log2().floor() as usize - 2).min(self.levels.len().checked_sub(1)?);
        Some((&self.levels[i], (4usize << i) as f64))
    }

    #[cfg(test)]
    fn bytes(&self) -> usize {
        self.levels.iter().map(|l| l.px.len() * 4).sum()
    }
}

impl Level {
    /// `src` shrunk `factor` times: each pixel averages the `factor` by `factor` under it, each
    /// counting for its weight, so a level made from a level weighs the image's pixels once.
    fn shrink(src: &(impl Pixels + Sync), factor: usize) -> Level {
        use rayon::prelude::*;
        let (w, h) = src.size();
        let (width, height) = (w.div_ceil(factor), h.div_ceil(factor));
        let mut px = vec![0u32; width * height];
        px.par_chunks_mut(width).enumerate().for_each(|(y, row)| {
            for (x, out) in row.iter_mut().enumerate() {
                let (mut sum, mut weights, mut n) = ([0.0f64; 3], 0.0f64, 0usize);
                for sy in y * factor..((y + 1) * factor).min(h) {
                    for sx in x * factor..((x + 1) * factor).min(w) {
                        let Some(c) = src.at(sx, sy) else { continue };
                        let weight = weight(c);
                        sum[0] += ((c >> 16) & 0xff) as f64 * weight;
                        sum[1] += ((c >> 8) & 0xff) as f64 * weight;
                        sum[2] += (c & 0xff) as f64 * weight;
                        weights += weight;
                        n += 1;
                    }
                }
                *out = if n == 0 {
                    0x00FF_FFFF
                } else {
                    let channel = |v: f64| (v / weights).round() as u32;
                    let ink = ((weights / n as f64 - 1.0) / INK_WEIGHT * 255.0).round().clamp(0.0, 255.0) as u32;
                    (ink << 24) | (channel(sum[0]) << 16) | (channel(sum[1]) << 8) | channel(sum[2])
                };
            }
        });
        Level { width, height, px }
    }
}

/// Longitudes at which the closed ring crosses latitude `lat`, ascending. A
/// point is inside when an odd number of them lie east of it.
fn crossings(ring: &[[f64; 2]], lat: f64, out: &mut Vec<f64>) {
    out.clear();
    let mut j = ring.len().wrapping_sub(1);
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[j]);
        if (a[1] > lat) != (b[1] > lat) {
            out.push((b[0] - a[0]) * (lat - a[1]) / (b[1] - a[1]) + a[0]);
        }
        j = i;
    }
    out.sort_by(f64::total_cmp);
}

fn render(ctx: &DrawCtx, chart: &ChartInfo, img: &Image, mips: Option<&Mips>, covered: &mut Covered) -> Option<Drawn> {
    let (vi, pal) = (&ctx.vi, &ctx.pal);
    // Only the part of the screen the chart can cover.
    let b = chart.bounds;
    let corners = [(b.min_lat, b.min_lon), (b.min_lat, b.max_lon), (b.max_lat, b.min_lon), (b.max_lat, b.max_lon)];
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for (lat, lon) in corners {
        let (x, y) = vi.to_screen(&LatLon::new(lat, lon));
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    let within = |pad: f64| {
        let a = &covered.area;
        ((x0.max(a[0] - pad)).floor(), (y0.max(a[1] - pad)).floor(), (x1.min(a[2] + pad)).ceil(), (y1.min(a[3] + pad)).ceil())
    };
    let (mut x0, mut y0, mut x1, mut y1) = within(0.0);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    // Work in device pixels: on a 2x screen a block per logical pixel would be
    // a visible 2x2 square.
    let scale = covered.scale;
    let g = &chart.georef;
    // Source pixels under one device pixel, from a step at the middle.
    let span = {
        let (mx, my, d) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0, 1.0 / scale);
        let u = |x: f64| g.u_of_lon(vi.position_at(x, 0.0).lon);
        let v = |y: f64| g.v_of_lat(vi.position_at(0.0, y).lat);
        let o = g.uv_to_pixel(0.0, 0.0);
        let (a, c) = (g.uv_to_pixel(u(mx + d) - u(mx), 0.0), g.uv_to_pixel(0.0, v(my + d) - v(my)));
        (a.0 - o.0).hypot(a.1 - o.1).max((c.0 - o.0).hypot(c.1 - o.1))
    };
    // Every device pixel is worked out, or text turns to mush. Shrunk to under a quarter, that is
    // from the mips; until they are made, fewer, larger blocks, smoothed.
    let level = mips.and_then(|m| m.level(span));
    let block = if span <= 4.0 || level.is_some() { 1.0 } else if span <= 8.0 { 2.0 } else { 3.0 };
    if block > 1.0 {
        // Smoothed blocks blend with their neighbours: a block or two past the edge, so a tile's edge does
        // not fade to nothing.
        (x0, y0, x1, y1) = within(2.0 * block / scale);
    }
    let (sw, sh) = (((x1 - x0) * scale / block).ceil() as i32, ((y1 - y0) * scale / block).ceil() as i32);
    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, sw, sh).ok()?;
    surface.set_device_scale(scale / block, scale / block);
    let lut = colours(chart, pal);
    let src = Indexed { img, lut: &lut };
    // Longitude depends only on the column and latitude on the row, so the
    // map projection and the chart's own are worked out once for each.
    let lon_mid = (chart.bounds.min_lon + chart.bounds.max_lon) / 2.0;
    let pos = |i: i32| i as f64 * block / scale + block / scale / 2.0;
    let lons: Vec<f64> = (0..sw).map(|bx| vi.position_at(x0 + pos(bx), 0.0).lon).collect();
    // The polygon of a chart across the antimeridian runs past 180.
    let ring_lons: Vec<f64> = lons.iter().map(|l| l + 360.0 * ((lon_mid - l) / 360.0).round()).collect();
    let us: Vec<f64> = lons.iter().map(|l| g.u_of_lon(*l)).collect();
    let poly = &chart.header.polygon;
    let mut cross = Vec::new();
    let mut painted = false;
    {
        let stride = surface.stride() as usize;
        let mut data = surface.data().ok()?;
        data.fill(0);
        for by in 0..sh {
            let lat = vi.position_at(0.0, y0 + pos(by)).lat;
            let v = g.v_of_lat(lat);
            if poly.len() >= 3 {
                crossings(poly, lat, &mut cross);
                if cross.is_empty() {
                    continue;
                }
            }
            let y = y0 + by as f64 * block / scale;
            for bx in 0..sw as usize {
                let x = x0 + bx as f64 * block / scale;
                // Filled by a finer chart already (a block, all of it: it is smoothed into its neighbours).
                if covered.all(x, y, block as usize) {
                    continue;
                }
                if poly.len() >= 3 {
                    let east = cross.len() - cross.partition_point(|c| *c <= ring_lons[bx]);
                    if east % 2 == 0 {
                        continue;
                    }
                }
                let (px, py) = g.uv_to_pixel(us[bx], v);
                if px < 0.0 || py < 0.0 || px >= img.width as f64 || py >= img.height as f64 {
                    continue;
                }
                let c = match level {
                    Some((level, f)) => average(level, px / f, py / f, span / f),
                    None => sample(&src, px, py, span * block),
                };
                if let Some(c) = c {
                    let o = by as usize * stride + bx * 4;
                    data[o..o + 4].copy_from_slice(&c.to_ne_bytes());
                    painted = true;
                    // Opaque, so whatever is under it is hidden.
                    covered.fill(x, y, block as usize);
                }
            }
        }
    }
    painted.then_some(Drawn { surface, x: x0, y: y0, smooth: block > 1.0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vector_base_takes_over_from_a_chart_blown_up_or_far_too_fine() {
        assert!(scale_suits_over_base(10_000_000, 10_000_000.0));
        assert!(scale_suits_over_base(10_000_000, 4_500_000.0));
        assert!(!scale_suits_over_base(10_000_000, 2_300_000.0), "the Galapagos at zoom 8");
        assert!(!scale_suits_over_base(25_000, 5_000_000.0), "a harbour chart over a whole sea");
        assert!(scale_suits_over_base(0, 1.0e9));
    }

    #[test]
    fn a_detailed_chart_outlasts_its_own_pixels_over_a_base_map() {
        // A 1:10 000 scan has one pixel per screen pixel at about 3 800: it stays to 1 700, not 4 000.
        assert!(scale_suits_over_base(10_000, 3_800.0));
        assert!(scale_suits_over_base(10_000, 2_000.0));
        assert!(!scale_suits_over_base(10_000, 1_500.0));
        // A small-scale chart blown up that far is just a blur.
        assert!(!scale_suits_over_base(500_000, 100_000.0));
    }

    #[test]
    fn charts_show_around_their_own_scale() {
        assert!(scale_suits(25_000, 25_000.0));
        assert!(scale_suits(25_000, 120_000.0));
        assert!(!scale_suits(25_000, 400_000.0), "too far out");
        assert!(scale_suits(25_000, 3_000.0), "overzoomed but readable");
        assert!(!scale_suits(25_000, 1_000.0), "absurdly overzoomed");
        assert!(scale_suits(0, 1.0e9));
    }

    #[test]
    fn a_real_chart_opens_from_its_header_alone() {
        let p = std::path::PathBuf::from("testdata/nz-kap-archive/BSB_ROOT/NZ5224/NZ522401.KAP");
        if !p.exists() {
            return;
        }
        let c = Chart::open(p).unwrap();
        assert_eq!(c.header.scale, 25_000);
        assert!(c.bounds.min_lat < -36.0 && c.bounds.max_lat > -36.5, "{:?}", c.bounds);
        assert!(c.image.borrow().is_none(), "the image is not decoded yet");
    }

    /// Every chart in the NZ archive should at least open. Run with
    /// `cargo test --release every_nz -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn every_nz_chart_opens() {
        let files = crate::core::datadir::kap_files(&[PathBuf::from("testdata/nz-kap-archive")]);
        if files.is_empty() {
            return;
        }
        let mut failed = Vec::new();
        for f in &files {
            if let Err(e) = ChartInfo::open(f.clone()) {
                failed.push(format!("{}: {e}", f.display()));
            }
        }
        eprintln!("{} charts, {} failed", files.len(), failed.len());
        for f in failed.iter().take(10) {
            eprintln!("  {f}");
        }
        assert!(failed.len() * 50 < files.len(), "more than 2% failed");
    }

    #[test]
    fn opening_junk_is_an_error() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.kap");
        std::fs::write(&p, b"no header here").unwrap();
        assert!(Chart::open(p).is_err());
        assert!(Chart::open(d.path().join("missing.kap")).is_err());
    }

    fn chart_of_scale(dir: &std::path::Path, scale: u32, refs: &[(f64, f64, f64, f64)]) -> Rc<Chart> {
        use crate::core::chart::bsb::testutil::kap;
        let bytes = kap(100, 60, &|_, _| 1, "MERCATOR", refs);
        let text = String::from_utf8_lossy(&bytes[..300]).replace("SC=25000", &format!("SC={scale:08}"));
        let mut out = text.into_bytes();
        out.extend_from_slice(&bytes[300..]);
        let path = dir.join(format!("{scale}.kap"));
        std::fs::write(&path, out).unwrap();
        Rc::new(Chart::open(path).unwrap())
    }

    fn planned(scales: &[u32], denom: f64) -> Vec<u32> {
        planned_over(scales, denom, false)
    }

    fn planned_over(scales: &[u32], denom: f64, base: bool) -> Vec<u32> {
        let d = tempfile::tempdir().unwrap();
        let (lat_top, w) = (-36.0f64, 100.0f64);
        let refs: Vec<_> = [(0.0, 0.0), (100.0, 0.0), (100.0, 60.0), (0.0, 60.0)]
            .iter()
            .map(|&(x, y)| (x, y, lat_top - y * 0.1 / w, 175.0 + x * 0.1 / w))
            .collect();
        let charts: Vec<_> = scales.iter().map(|s| chart_of_scale(d.path(), *s, &refs)).collect();
        let inside = [LatLon::new(-36.12, 175.38)];
        plan(&charts, &inside, denom, base).iter().map(|c| c.header.scale).collect()
    }

    #[test]
    fn the_index_finds_what_trying_every_chart_does() {
        let d = tempfile::tempdir().unwrap();
        // Charts of a degree and a bit, in a row, one across the antimeridian, and one over half the world.
        let chart = |lon: f64, lat: f64, w: f64, scale: u32| {
            use crate::core::chart::bsb::testutil::kap;
            let h = w.min(40.0);
            let refs = [(0.0, 0.0, lat, lon), (100.0, 0.0, lat, lon + w), (100.0, 60.0, lat - h, lon + w), (0.0, 60.0, lat - h, lon)];
            let bytes = kap(100, 60, &|_, _| 1, "MERCATOR", &refs);
            // Without its polygon (the helper's is at one place), a chart covers what its corners do.
            let end = bytes.windows(2).position(|w| w == [0x1a, 0]).unwrap();
            let head: String = String::from_utf8_lossy(&bytes[..end]).split("\r\n").filter(|l| !l.starts_with("PLY/")).collect::<Vec<_>>().join("\r\n");
            let head = head.replace("SC=25000", &format!("SC={scale:08}"));
            let mut out = head.into_bytes();
            out.extend_from_slice(&bytes[end..]);
            let path = d.path().join(format!("{lon}_{lat}.kap"));
            std::fs::write(&path, out).unwrap();
            Rc::new(Chart::open(path).unwrap())
        };
        let charts: Vec<Rc<Chart>> = vec![chart(-170.0, 30.0, 150.0, 10_000_000), chart(-4.6, 48.5, 1.2, 50_000), chart(-3.5, 48.5, 1.2, 25_000), chart(179.5, -16.0, 1.0, 50_000)];
        let index = Index::new(&charts);
        assert_eq!(index.wide.len(), 1);
        for (lat, lon) in [(48.0, -4.0), (48.0, -3.0), (47.9, -2.4), (-16.5, 179.9), (-16.5, -179.9), (0.0, 0.0), (60.0, 60.0)] {
            let p = LatLon::new(lat, lon);
            let every: Vec<usize> = (0..charts.len()).filter(|i| covers(&charts[*i], &p)).collect();
            let found: Vec<usize> = index.at(&p).into_iter().filter(|i| covers(&charts[*i], &p)).collect();
            assert_eq!(found, every, "at {lat}, {lon}");
        }
    }

    #[test]
    fn the_finest_suitable_chart_wins_and_hidden_ones_are_not_drawn() {
        assert_eq!(planned(&[25_000, 1_500_000], 100_000.0), [25_000]);
        assert_eq!(planned(&[25_000, 1_500_000], 10_000.0), [25_000]);
        assert_eq!(planned(&[1_500_000, 200_000, 25_000], 300_000.0), [200_000]);
    }

    #[test]
    fn over_a_base_map_only_charts_that_suit_are_drawn() {
        // Without a base the nearest chart fills the hole; with one, it gives way.
        assert_eq!(planned_over(&[10_000_000], 2_300_000.0, false), [10_000_000]);
        assert!(planned_over(&[10_000_000], 2_300_000.0, true).is_empty());
        assert_eq!(planned_over(&[10_000_000, 25_000], 100_000.0, true), [25_000]);
        assert_eq!(planned_over(&[10_000_000], 8_000_000.0, true), [10_000_000]);
    }

    #[test]
    fn zooming_far_in_or_out_never_leaves_a_hole() {
        // Past every chart's useful range the nearest scale still shows.
        assert_eq!(planned(&[25_000, 1_500_000], 500.0), [25_000]);
        assert_eq!(planned(&[25_000, 1_500_000], 200_000_000.0), [1_500_000]);
        // Nothing covers a position outside the charts.
        let d = tempfile::tempdir().unwrap();
        let refs = [(0.0, 0.0, -36.0, 175.0), (100.0, 0.0, -36.0, 175.1), (100.0, 60.0, -36.06, 175.1), (0.0, 60.0, -36.06, 175.0)];
        let c = chart_of_scale(d.path(), 25_000, &refs);
        assert!(plan(&[c], &[LatLon::new(10.0, 10.0)], 50_000.0, false).is_empty());
    }

    fn image_of(w: usize, h: usize, f: &dyn Fn(usize, usize) -> u8) -> Image {
        Image { width: w, height: h, index: (0..w * h).map(|i| f(i % w, i / w)).collect() }
    }

    const BLACK: u32 = 0xFF00_0000;
    const WHITE: u32 = 0xFFFF_FFFF;

    fn sample(img: &Image, lut: &[u32], px: f64, py: f64, span: f64) -> Option<u32> {
        super::sample(&Indexed { img, lut }, px, py, span)
    }

    #[test]
    fn zoomed_in_pixels_blend_and_zoomed_out_pixels_average() {
        let lut = [inked(0, 0, 0), inked(255, 255, 255)];
        // Black left half, white right half.
        let img = image_of(10, 4, &|x, _| (x >= 5) as u8);
        // Magnified: the edge between two source pixels is a smooth ramp.
        assert_eq!(sample(&img, &lut, 2.5, 2.0, 0.5), Some(BLACK));
        let mid = sample(&img, &lut, 5.0, 2.0, 0.5).unwrap() & 0xff;
        assert!((120..136).contains(&mid), "halfway between the two: {mid}");
        // About one to one: the source pixel itself, no blending.
        assert_eq!(sample(&img, &lut, 7.5, 2.0, 1.0), Some(WHITE));
        // Minified over a block that is half black, half white: grey, leaning to the black.
        let g = sample(&img, &lut, 5.0, 2.0, 4.0).unwrap() & 0xff;
        assert!((50..80).contains(&g), "weighted average of the block: {g}");
    }

    #[test]
    fn a_thin_line_survives_zooming_out_as_a_tint() {
        let lut = [inked(255, 255, 255), inked(0, 0, 0)];
        // A one pixel black line every 4 pixels, looked at 4 to 1.
        let img = image_of(64, 64, &|x, _| (x % 4 == 0) as u8);
        let v = |x: f64| sample(&img, &lut, x, 32.0, 4.0).unwrap() & 0xff;
        let tints: Vec<u32> = (0..6).map(|i| v(8.0 + i as f64 * 4.0 + 0.3 * i as f64)).collect();
        // Nearest sampling would give pure black or pure white; averaging gives
        // the same grey wherever the window falls, darker than a quarter black
        // because the line counts for more than the ground.
        assert!(tints.iter().all(|t| (115..140).contains(t)), "{tints:?}");
    }

    #[test]
    fn mips_keep_thin_lines_as_a_tint_and_are_picked_by_how_far_the_chart_is_shrunk() {
        let lut = [inked(255, 255, 255), inked(0, 0, 0)];
        let img = image_of(512, 256, &|x, _| (x % 4 == 0) as u8);
        let mips = Mips::new(&img, &lut, Tone::Day);
        let sizes: Vec<usize> = mips.levels.iter().map(|l| l.width).collect();
        assert_eq!(sizes, [128, 64]);
        assert!(mips.level(3.9).is_none(), "read from the image itself");
        assert_eq!(mips.level(4.0).map(|(l, f)| (l.width, f)), Some((128, 4.0)));
        assert_eq!(mips.level(9.0).map(|(l, f)| (l.width, f)), Some((64, 8.0)));
        assert_eq!(mips.level(100.0).map(|(l, f)| (l.width, f)), Some((64, 8.0)), "the smallest there is");
        // Every level is the same grey, a black pixel weighing three to three white ones: the line weighed
        // once, not again at each level.
        for l in &mips.levels {
            let g = l.px[l.width * 2 + 2] & 0xff;
            assert!(g.abs_diff(128) <= 1, "{g}");
        }
    }

    #[test]
    fn scanline_crossings_follow_the_polygon() {
        let ring = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let mut c = Vec::new();
        crossings(&ring, 5.0, &mut c);
        assert_eq!(c, [0.0, 10.0]);
        crossings(&ring, 15.0, &mut c);
        assert!(c.is_empty());
        // A triangle: its width shrinks with latitude.
        let tri = [[0.0, 0.0], [10.0, 0.0], [5.0, 10.0]];
        crossings(&tri, 5.0, &mut c);
        assert_eq!(c, [2.5, 7.5]);
    }

    /// Renders one real chart at device scales 1 and 2 into PPMs named by
    /// `ISOMAR_BSB_PNG` (a prefix), and checks the 2x one has finer detail.
    /// `ISOMAR_BSB_PNG=/tmp/x cargo test --release render_at_two_scales -- --ignored --nocapture`
    /// (`ISOMAR_BSB_KAP`, `ISOMAR_BSB_CENTER=<lat>,<lon>` and `ISOMAR_BSB_ZOOM` for another chart).
    #[test]
    #[ignore]
    fn render_at_two_scales() {
        use crate::layers::overlay;
        use crate::palette::{Mode, Palette};
        use crate::view::View;
        let path = std::path::PathBuf::from(std::env::var("ISOMAR_BSB_KAP").unwrap_or_else(|_| "../../testdata/nz-kap-archive/BSB_ROOT/NZ5224/NZ522401.KAP".into()));
        if !path.exists() {
            return;
        }
        let prefix = std::env::var("ISOMAR_BSB_PNG").unwrap_or_else(|_| "/tmp/bsb".into());
        let zoom: f64 = std::env::var("ISOMAR_BSB_ZOOM").ok().and_then(|z| z.parse().ok()).unwrap_or(14.0);
        let chart = Chart::open(path.clone()).unwrap();
        let img = Arc::new(decode_image(&std::fs::read(&path).unwrap(), &chart.header).unwrap());
        let pal = Palette::new(Mode::Day, false);
        let t = std::time::Instant::now();
        let mips = Mips::new(&img, &colours(&chart.info, &pal), pal.tone);
        eprintln!("mips of {}x{} in {:?}, {} MB", img.width, img.height, t.elapsed(), mips.bytes() >> 20);
        let paint = BsbPaint { charts: vec![(chart.info.clone(), img, Some(Arc::new(mips)))] };
        let (w, h) = (700.0, 450.0);
        let center = std::env::var("ISOMAR_BSB_CENTER").ok().and_then(|c| c.split_once(',').and_then(|(a, b)| Some(LatLon::new(a.parse().ok()?, b.parse().ok()?))));
        let vi = ViewInfo { view: View { center: center.unwrap_or(LatLon::new(-36.29, 175.40)), zoom }, w, h };
        let mut ink = Vec::new();
        for scale in [1, 2] {
            let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, (w as i32) * scale, (h as i32) * scale).unwrap();
            surface.set_device_scale(scale as f64, scale as f64);
            let cr = cairo::Context::new(&surface).unwrap();
            cr.set_source_rgb(0.86, 0.91, 0.95);
            let _ = cr.paint();
            let t = std::time::Instant::now();
            paint.paint(&DrawCtx { cr: &cr, vi, pal: Palette::new(Mode::Day, false), overlay: &overlay::Overlay::default(), reduce_motion: false, top_inset: 0.0 });
            eprintln!("scale {scale}: painted in {:?}", t.elapsed());
            drop(cr);
            let mut surface = surface;
            let stride = surface.stride() as usize;
            let data = surface.data().unwrap();
            // A binary PPM, since cairo is built here without PNG support.
            let (dw, dh) = ((w as usize) * scale as usize, (h as usize) * scale as usize);
            let mut ppm = format!("P6\n{dw} {dh}\n255\n").into_bytes();
            for y in 0..dh {
                for x in 0..dw {
                    let o = y * stride + x * 4;
                    ppm.extend([data[o + 2], data[o + 1], data[o]]);
                }
            }
            std::fs::write(format!("{prefix}-{scale}x.ppm"), ppm).unwrap();
            // Distinct colours per device pixel is a rough measure of detail.
            let dark = data.chunks(4).filter(|p| p[0] < 90 && p[1] < 90 && p[2] < 90).count();
            ink.push(dark as f64 / (data.len() / 4) as f64);
        }
        eprintln!("dark pixel fraction at 1x {:.4}, at 2x {:.4}", ink[0], ink[1]);
    }
}
