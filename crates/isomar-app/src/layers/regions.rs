//! Names of seas, straits, gulfs, island groups, ranges, peninsulas and capes from Natural Earth,
//! lettered as on a paper chart: spread along the feature (`core::labels`), in capitals where a
//! chart sets them so, turned with the curve. The features and their centrelines are fixed (the
//! bundle `cargo xtask naturalearth` makes), so all that is worked out here is, once per zoom, how
//! each name sits and which of two that collide wins; every tile then agrees without asking the others.

use super::names::{self, Measured, PathLabel, Style};
use super::TileArea;
use crate::core::chart::naturalearth::{self, GeoKind, GeoName};
use crate::core::labels::coast::CapeAxis;
use crate::core::labels::{self, FitParams, Spine};
use crate::core::vector::Pt;
use std::cell::{Cell, RefCell};
use std::cmp::Reverse;
use std::collections::{HashMap, VecDeque};
use std::f64::consts::{FRAC_PI_2, PI};
use std::rc::Rc;
use std::sync::Arc;

/// Room kept between names that collide, in pixels.
const GAP_PX: f64 = 4.0;
/// How far past a tile a name is still wanted, in pixels.
const REACH_PX: f64 = 64.0;
/// How far a name may sit from the box of what it names (a cape's name stands off its point), in pixels.
const SLOP_PX: f64 = 80.0;
/// Zoom levels kept.
const LEVELS_KEPT: usize = 3;
/// A name too big for its feature is written smaller, down to this.
const MIN_SIZE: f64 = 10.0;
/// A cape's name begins (or ends) this far from the tip, along the line through it.
const CAPE_OFFSET_PX: f64 = 4.0;
/// How much farther out a cape's name goes, in pixels, and how far it turns off its point, either way, in
/// degrees, when a mark on the tip or the coast is in the way: the nearest place that is clear wins, a degree
/// of turn counting as `CAPE_TURN_COST_PX` farther out. Square to the coast matters less than near the tip.
const CAPE_STEP_OUT_PX: [f64; 7] = [0.0, 2.0, 4.0, 6.0, 8.0, 11.0, 14.0];
const CAPE_TURNS_DEG: [f64; 19] = [0.0, 10.0, -10.0, 20.0, -20.0, 30.0, -30.0, 40.0, -40.0, 50.0, -50.0, 60.0, -60.0, 70.0, -70.0, 80.0, -80.0, 90.0, -90.0];
const CAPE_TURN_COST_PX: f64 = 0.2;
/// How often the water under a cape's name is tested, in pixels, and on which lines, as shares of its height.
const CAPE_WATER_STEP_PX: f64 = 3.0;
const CAPE_WATER_ACROSS: [f64; 3] = [0.0, -0.3, 0.3];

/// How a kind of name is written at a feature `on` pixels long on screen: the style, the tracking
/// limits and how much of the room it fills, the turns allowed, and the zoom past which it is not
/// written (you are inside it).
pub(super) struct Rule {
    pub style: Style,
    pub track: (f64, f64),
    pub fill: f64,
    pub turn: (f64, f64),
    pub max_zoom: f64,
}

fn pick(on: f64, steps: [(f64, f64); 2], last: f64) -> f64 {
    steps.iter().find(|(below, _)| on < *below).map_or(last, |(_, size)| *size)
}

pub(super) fn rule(kind: GeoKind, on: f64) -> Rule {
    use pango::Weight as W;
    let sea = pick(on, [(150.0, 12.5), (400.0, 14.0)], if on < 900.0 { 16.0 } else if on < 1600.0 { 18.0 } else { 20.0 });
    let range = if on < 400.0 { 11.5 } else if on < 1000.0 { 12.5 } else { 13.5 };
    match kind {
        GeoKind::Ocean => Rule { style: Style::water(if on < 900.0 { 18.0 } else if on < 1600.0 { 20.0 } else { 22.0 }).capitals(), track: (0.4, 1.2), fill: 0.80, turn: (12.0, 40.0), max_zoom: 6.0 },
        GeoKind::Sea => Rule { style: Style::water(sea).capitals(), track: (0.3, 2.0), fill: 0.75, turn: (15.0, 50.0), max_zoom: 11.0 },
        GeoKind::Gulf => Rule { style: Style::water(sea - 1.0).capitals(), track: (0.2, 2.0), fill: 0.70, turn: (15.0, 50.0), max_zoom: 12.0 },
        GeoKind::Strait => Rule { style: Style::water(if on < 300.0 { 12.0 } else if on < 800.0 { 13.5 } else { 15.0 }).capitals(), track: (0.1, 1.2), fill: 0.70, turn: (20.0, 70.0), max_zoom: 14.0 },
        GeoKind::IslandGroup => {
            let size = if on < 400.0 { 12.5 } else if on < 1000.0 { 14.0 } else { 15.5 };
            Rule { style: Style::land(size, W::Normal, 0.0).on_sea().capitals(), track: (0.3, 2.5), fill: 0.80, turn: (15.0, 60.0), max_zoom: 10.0 }
        }
        // An island is written the same whichever data names it (the tiles' level names, Natural Earth's along
        // the land): upright, in its own case, sized by how big it is on screen.
        // An island along its long axis, in tracked capitals, the same whichever data names it; sized by the
        // island itself (`island_em`), so that it grows with the island as the view zooms in.
        GeoKind::Island => Rule { style: Style::land(island_size(on), W::Normal, 0.0).capitals(), track: (ISLAND_TRACK_EM, ISLAND_TRACK_EM), fill: 1.0, turn: (20.0, 50.0), max_zoom: 18.0 },
        GeoKind::Range => Rule { style: Style::land(range, W::Normal, 0.0).capitals(), track: (0.2, 1.5), fill: 0.70, turn: (20.0, 60.0), max_zoom: 12.0 },
        GeoKind::Peninsula => Rule { style: Style::land(range, W::Normal, 0.0), track: (0.05, 0.6), fill: 0.60, turn: (20.0, 50.0), max_zoom: 11.0 },
        GeoKind::Bay => Rule { style: Style::water(if on < 300.0 { 11.5 } else { 12.5 }), track: (0.0, 0.3), fill: 0.60, turn: (25.0, 70.0), max_zoom: 14.0 },
        GeoKind::Cape => Rule { style: Style::land(10.5, W::Normal, 0.0).on_sea(), track: (0.0, 0.0), fill: 1.0, turn: (0.0, 0.0), max_zoom: 13.0 },
        // Sized by Natural Earth's rank in `country_style`; past zoom 8 the islands, towns and coasts say where you are.
        GeoKind::Country => Rule { style: country_style(5), track: (COUNTRY_TRACK_EM, COUNTRY_TRACK_EM), fill: 1.0, turn: (0.0, 0.0), max_zoom: 8.0 },
    }
}

/// The zoom before which a kind is not named whatever Natural Earth says: a cape, a range or a peninsula
/// is detail, not what a view of a whole sea needs (Cap Bon came out at zoom 5).
fn min_zoom(kind: GeoKind) -> f64 {
    match kind {
        GeoKind::Cape => 7.0,
        GeoKind::Range | GeoKind::Peninsula => 6.0,
        _ => 0.0,
    }
}

/// An island's name by how long the island is on screen, in pixels.
pub(super) fn island_size(on: f64) -> f64 {
    match on {
        l if l < 40.0 => 10.5,
        l if l < 100.0 => 11.5,
        l if l < 250.0 => 12.5,
        _ => 13.5,
    }
}

/// An island's name along it spreads its capitals this much, and takes this share of the island's length:
/// the name is a drawing on the island, scaled with it from zoom to zoom, between the smallest and biggest
/// sizes (points, before the text size preference).
const ISLAND_TRACK_EM: f64 = 0.25;
const ISLAND_SPAN: f64 = 0.7;
const ISLAND_MIN_PT: f64 = 6.0;
/// And never smaller than this many pixels, whatever the text size preference.
pub(super) const ISLAND_MIN_PX: f64 = 11.0;
pub(super) const ISLAND_MAX_PT: f64 = 18.0;
/// About how wide a capital is, with its tracking, in ems.
const CAPS_EM: f64 = 0.95;
/// Natural Earth's islands are named up to this zoom; past it the tiles' own, drawn from a finer coast, are.
const NE_ISLAND_MAX_ZOOM: f64 = 11.0;

/// The size (points, before the text size preference) at which `name`'s capitals take `ISLAND_SPAN` of a
/// length of `length` pixels, if that is within the bounds.
fn island_em(name: &str, length: f64) -> Option<f64> {
    let pt = island_pt(name, length);
    let scale = super::text_scale() as f64 / 100.0;
    (pt >= ISLAND_MIN_PT && pt * scale >= ISLAND_MIN_PX).then_some(pt.min(ISLAND_MAX_PT))
}

/// The size (points, before the text size preference) at which `name`'s capitals take `ISLAND_SPAN` of an
/// island `length` pixels long, unbounded: an island's level name is sized by it too.
pub(super) fn island_pt(name: &str, length: f64) -> f64 {
    let px = ISLAND_SPAN * length / (name.chars().count().max(1) as f64 * CAPS_EM);
    px / (super::text_scale() as f64 / 100.0)
}

/// How much of an island name's height may stand past the coast: a chart writes a name across an island
/// that only just holds it.
const ISLAND_OVERHANG: f64 = 0.25;

/// How much a country's capitals are spread, and a stacked ocean's, as a share of the size.
const COUNTRY_TRACK_EM: f64 = 0.15;
const OCEAN_STACK_TRACK_EM: f64 = 0.3;
/// A stacked ocean's name may be written smaller than others: the world is small at the zooms it is named.
const STACK_SHRINK: [f64; 4] = [1.0, 0.85, 0.72, 0.6];
/// An ocean's name stacked on two lines is cut when longer than this.
const STACK_TWO: usize = 6;
/// A country's name longer than this is set on two lines.
const COUNTRY_WRAP_CHARS: usize = 12;
/// How much of its box a country's name may take, each way.
const COUNTRY_ROOM: f64 = 0.9;
/// A country this small (its box's longer side, in units: about 3 degrees) is an island or a few, named
/// beside them when they cannot hold the name; once they are this big on screen, and this far off them, in pixels.
const SMALL_COUNTRY_UNITS: f64 = 2.2;
const ISLAND_STATE_MIN_PX: f64 = 8.0;
const ISLAND_STATE_GAP_PX: f64 = 4.0;

/// A country's name, by Natural Earth's label rank (1 the biggest): upright capitals in the muted ink of
/// a chart's land names, so it reads as background to the waters, islands and coasts.
fn country_style(rank: u8) -> Style {
    let size = match rank {
        0..=2 => 13.0,
        3 => 12.0,
        4 => 11.0,
        _ => 10.5,
    };
    Style { fill: crate::tokens::Token::ChartLabel, ..Style::land(size, pango::Weight::Normal, 0.0).capitals() }
}

/// The most an island's name may lean from level along its land, in degrees, before it is written level.
const ISLAND_MAX_TILT_DEG: f64 = 50.0;

/// How far a label's letters lean from level on average, in radians.
fn mean_tilt(l: &PathLabel) -> f64 {
    let letters: Vec<f64> = l.glyphs.iter().filter(|g| !g.0.trim().is_empty()).map(|g| g.1.angle.abs()).collect();
    letters.iter().sum::<f64>() / letters.len().max(1) as f64
}

/// `text` cut into at most two lines at the space that balances them best, when it is long; a name with
/// no space is cut after a hyphen, which stays on the first line (Belle-Île- / en-Mer).
pub(super) fn wrap(text: &str, longer_than: usize) -> Vec<String> {
    let n = text.chars().count();
    if n <= longer_than {
        return vec![text.to_string()];
    }
    let half = (n as i64 - 1) / 2;
    let breaks = |c: char| text.char_indices().filter(move |(_, d)| *d == c).map(|(i, _)| i);
    let balance = |i: &usize| (text[..*i].chars().count() as i64 - half).abs();
    if let Some(best) = breaks(' ').min_by_key(balance) {
        return vec![text[..best].to_string(), text[best + 1..].to_string()];
    }
    match breaks('-').map(|i| i + 1).min_by_key(balance) {
        Some(best) if best < text.len() => vec![text[..best].to_string(), text[best..].to_string()],
        _ => vec![text.to_string()],
    }
}

/// A cape's name, in world pixels at `zoom` (`ppu` per unit), written as a paper chart writes it: on a line
/// through the tip, its baseline on that line (the name's underline, drawn on, would end at the tip), from just
/// off the tip, upright; along the way the point runs, else turned as far as along the coast when a mark or the
/// coast is in the way. `water` says whether a point (world pixels) is on the water, `None` where nothing says.
/// No letter may be on land, where nothing says, or on what `avoid` says no to: then there is no label.
#[allow(clippy::too_many_arguments)]
pub(super) fn cape_label(id: u64, name: &str, m: &Measured, style: Style, axis: &CapeAxis, ppu: f64, zoom: f64, aka: Vec<String>, units: [f64; 4], water: &dyn Fn(Pt) -> Option<bool>, avoid: &dyn Fn(&[f64; 4]) -> bool) -> Option<PathLabel> {
    let advances = m.advances();
    let (w, h) = (advances.iter().sum::<f64>(), m.height);
    // How far the baseline is below the middle of the line.
    let below = m.baseline - h / 2.0;
    let tip = [axis.tip[0] * ppu, axis.tip[1] * ppu];
    let fold = |a: f64| if a > FRAC_PI_2 { a - PI } else if a <= -FRAC_PI_2 { a + PI } else { a };
    // The name with its middle at `c` and turned by `angle`, if every letter is clear and on the water.
    let set = |c: Pt, angle: f64| -> Option<PathLabel> {
        let e = [angle.cos(), angle.sin()];
        let start = [c[0] - e[0] * w / 2.0, c[1] - e[1] * w / 2.0];
        let mut boxes = Vec::with_capacity(advances.len());
        let mut along = 0.0;
        for adv in &advances {
            let mid = along + adv / 2.0;
            boxes.push(labels::box_of(start[0] + e[0] * mid, start[1] + e[1] * mid, angle, *adv, h));
            along += adv;
        }
        if boxes.iter().any(avoid) {
            return None;
        }
        let n = (w / CAPE_WATER_STEP_PX).ceil().max(1.0) as usize;
        for k in CAPE_WATER_ACROSS {
            let o = [-e[1] * k * h, e[0] * k * h];
            for i in 0..=n {
                let t = w * i as f64 / n as f64;
                if water([start[0] + e[0] * t + o[0], start[1] + e[1] * t + o[1]]) != Some(true) {
                    return None;
                }
            }
        }
        let extent = boxes.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, x| [b[0].min(x[0]), b[1].min(x[1]), b[2].max(x[2]), b[3].max(x[3])]);
        Some(PathLabel { id, style, glyphs: vec![(name.to_string(), labels::GlyphAt { x: c[0], y: c[1], angle }, w)], height: h, boxes, extent, aka: aka.clone(), units, zoom })
    };
    // Out along the point, square to the coast at the tip, however steep, from as near the tip as is clear.
    let mut tries: Vec<(f64, f64)> = CAPE_STEP_OUT_PX.iter().flat_map(|&more| CAPE_TURNS_DEG.iter().map(move |&turn| (more, turn))).collect();
    tries.sort_by(|a, b| (a.0 + a.1.abs() * CAPE_TURN_COST_PX).total_cmp(&(b.0 + b.1.abs() * CAPE_TURN_COST_PX)));
    tries.into_iter().find_map(|(more, turn)| {
        let (c, s) = (turn.to_radians().cos(), turn.to_radians().sin());
        let d = [axis.out[0] * c - axis.out[1] * s, axis.out[0] * s + axis.out[1] * c];
        // Read away from the tip, or towards it where the line runs leftwards: either way it stays upright.
        let angle = fold(d[1].atan2(d[0]));
        let r = CAPE_OFFSET_PX + more + w / 2.0;
        // The middle of the line, up from the baseline (square to the text, against its downwards).
        let up = [angle.sin() * below, -angle.cos() * below];
        set([tip[0] + d[0] * r + up[0], tip[1] + d[1] * r + up[1]], angle)
    })
}

/// What a level of fits is for: the zoom, the text size, and the size of the view that decides what is too big.
type LevelKey = (i64, u32, i32, i32);

struct Level {
    key: LevelKey,
    fits: HashMap<u32, Option<Arc<PathLabel>>>,
    kept: HashMap<u32, bool>,
}

/// What one call works from.
struct Ctx<'a> {
    /// What no letter may be on (the marks and lights), in world pixels.
    avoid: &'a dyn Fn(&[f64; 4]) -> bool,
    zoom: f64,
    /// World pixels per unit at `zoom`.
    ppu: f64,
    view: (f64, f64),
    computed: Cell<u32>,
    memo: Cell<u32>,
}

pub struct RegionNames {
    lang: &'static str,
    /// Each name's clusters at a size, for good.
    measured: RefCell<HashMap<(String, u64), Rc<Measured>>>,
    /// The last few zooms, newest first.
    levels: RefCell<VecDeque<Level>>,
    /// The local names of islands, as the tiles call them ("Ilha do Pico", not "Pico Island"): an island is
    /// called the same at every zoom, whichever data names it.
    local: RefCell<HashMap<u32, String>>,
}

impl Default for RegionNames {
    fn default() -> Self {
        Self::new()
    }
}

fn overlap(a: &[f64; 4], b: &[f64; 4]) -> bool {
    a[0] < b[2] && a[2] > b[0] && a[1] < b[3] && a[3] > b[1]
}

fn grown(b: [f64; 4], by: f64) -> [f64; 4] {
    [b[0] - by, b[1] - by, b[2] + by, b[3] + by]
}

impl RegionNames {
    pub fn new() -> RegionNames {
        RegionNames::with_language(crate::core::i18n::language())
    }

    /// Names in `lang` ("fr", "zht"…), English where Natural Earth has none.
    pub fn with_language(lang: &'static str) -> RegionNames {
        RegionNames { lang, measured: RefCell::new(HashMap::new()), levels: RefCell::new(VecDeque::new()), local: RefCell::new(HashMap::new()) }
    }

    /// Takes `name`, which the tiles give an island at `at` (units), as the name of the Natural Earth island
    /// there that has it in some language. Whether that changed a name.
    pub fn learn(&self, at: [f64; 2], name: &str) -> bool {
        let key = name.trim().to_lowercase();
        let found = naturalearth::within([at[0], at[1], at[0], at[1]]).find(|g| g.kind == GeoKind::Island && g.all_names().any(|n| n.trim().to_lowercase() == key));
        let Some(g) = found else { return false };
        if self.local.borrow().get(&g.id).is_some_and(|n| n == name) {
            return false;
        }
        self.local.borrow_mut().insert(g.id, name.to_string());
        self.levels.borrow_mut().clear();
        true
    }

    /// The kept labels whose letters meet the region `area.vi`, grown by 64 px.
    pub fn labels_for(&self, area: &TileArea) -> Vec<Arc<PathLabel>> {
        self.labels_avoiding(area, &|_| false)
    }

    /// Forgets how the names sat: what they keep clear of has changed.
    pub fn forget(&self) {
        self.levels.borrow_mut().clear();
    }

    /// `labels_for`, each name set where none of its letters is on what `avoid` says no to. The labels are
    /// fitted at the area's `names::label_zoom`, in its world pixels, as `avoid` is asked; a shell scales them
    /// to its own zoom (`PathLabel::at_zoom`).
    pub fn labels_avoiding(&self, area: &TileArea, avoid: &dyn Fn(&[f64; 4]) -> bool) -> Vec<Arc<PathLabel>> {
        let started = crate::debug::timing().then(std::time::Instant::now);
        let vi = &area.vi;
        let zoom = names::label_zoom(vi.zoom());
        let ctx = Ctx { avoid, zoom, ppu: 2f64.powf(zoom), view: (area.view.w, area.view.h), computed: Cell::new(0), memo: Cell::new(0) };
        let key: LevelKey = ((zoom * 100.0).round() as i64, super::text_scale(), area.view.w as i32, area.view.h as i32);
        let mut level = {
            let mut levels = self.levels.borrow_mut();
            match levels.iter().position(|l| l.key == key) {
                Some(i) => levels.remove(i).unwrap(),
                None => Level { key, fits: HashMap::new(), kept: HashMap::new() },
            }
        };
        // From the label's world pixels to the region's.
        let k = 2f64.powf(vi.zoom() - zoom);
        let region = naturalearth::units_of(&vi.bounds());
        let wanted = grown(region, (REACH_PX + SLOP_PX) / (ctx.ppu * k));
        let mut out = Vec::new();
        let mut candidates = 0;
        for g in naturalearth::within(wanted) {
            candidates += 1;
            if !self.kept(&mut level, g, &ctx) {
                continue;
            }
            let Some(Some(label)) = level.fits.get(&g.id) else { continue };
            // Its letters, on the screen of the region.
            let (ax, ay) = vi.world_to_screen(label.extent[0] * k, label.extent[1] * k);
            let (bx, by) = (ax + (label.extent[2] - label.extent[0]) * k, ay + (label.extent[3] - label.extent[1]) * k);
            if bx >= -REACH_PX && ax <= vi.w + REACH_PX && by >= -REACH_PX && ay <= vi.h + REACH_PX {
                out.push(label.clone());
            }
        }
        {
            let mut levels = self.levels.borrow_mut();
            levels.push_front(level);
            levels.truncate(LEVELS_KEPT);
        }
        if let Some(t) = started {
            isomar_core::log!("isomar: region names {candidates} candidates, {} fitted, {} from memory, {} shown, {:.2} ms", ctx.computed.get(), ctx.memo.get(), out.len(), t.elapsed().as_secs_f64() * 1000.0);
        }
        out
    }

    /// How long the feature is on screen: its longer side.
    fn on(g: &GeoName, ctx: &Ctx<'_>) -> f64 {
        (g.bounds[2] - g.bounds[0]).max(g.bounds[3] - g.bounds[1]) * ctx.ppu
    }

    /// The order names win a crowded spot in: lower first.
    fn priority(g: &GeoName, ctx: &Ctx<'_>) -> (GeoKind, u8, Reverse<i64>, u32) {
        (g.kind, g.rank, Reverse(Self::on(g, ctx) as i64), g.id)
    }

    pub(super) fn measure(&self, text: &str, st: &Style) -> Rc<Measured> {
        let key = (text.to_string(), (st.size * super::text_scale() as f64).to_bits());
        if let Some(m) = self.measured.borrow().get(&key) {
            return m.clone();
        }
        let m = Rc::new(names::measure(text, st));
        self.measured.borrow_mut().insert(key, m.clone());
        m
    }

    /// Whether `g` is written at this level: it fits, and nothing that wins a crowded spot over it is in its way.
    fn kept(&self, level: &mut Level, g: &GeoName, ctx: &Ctx<'_>) -> bool {
        if let Some(k) = level.kept.get(&g.id) {
            return *k;
        }
        let kept = match self.fitted(level, g, ctx) {
            None => false,
            // An island territory named as its islands are (AZORES, the country and the group) is named once,
            // by the islands.
            Some(_) if g.kind == GeoKind::Country && g.island && self.named_by_islands(level, g, ctx) => false,
            Some(a) => {
                let (mine, reach) = (Self::priority(g, ctx), grown(a.extent.map(|v| v / ctx.ppu), GAP_PX / ctx.ppu));
                let slop = SLOP_PX / ctx.ppu;
                let rivals: Vec<&GeoName> = naturalearth::within(grown(reach, slop)).filter(|b| b.id != g.id && Self::priority(b, ctx) < mine).collect();
                !rivals.into_iter().any(|b| {
                    self.fitted(level, b, ctx).is_some_and(|b| {
                        a.boxes.iter().any(|x| {
                            let x = grown(*x, GAP_PX);
                            b.extent[0] < x[2] && b.extent[2] > x[0] && b.extent[1] < x[3] && b.extent[3] > x[1] && b.boxes.iter().any(|y| overlap(&x, y))
                        })
                    })
                })
            }
        };
        level.kept.insert(g.id, kept);
        kept
    }

    /// Whether an island group or island with one of `g`'s names, where `g` is, is written at this level.
    /// (Its rivals are weighed by whether they fit, not whether they are kept, so this does not come back to `g`.)
    fn named_by_islands(&self, level: &mut Level, g: &GeoName, ctx: &Ctx<'_>) -> bool {
        let names: Vec<String> = g.all_names().map(|n| n.trim().to_lowercase()).collect();
        let near: Vec<&GeoName> = naturalearth::within(g.bounds)
            .filter(|b| matches!(b.kind, GeoKind::IslandGroup | GeoKind::Island) && b.all_names().any(|n| names.contains(&n.trim().to_lowercase())))
            .collect();
        near.into_iter().any(|b| self.kept(level, b, ctx))
    }

    /// How `g` is set at this zoom, if it is shown and fits: memoised for the level.
    fn fitted(&self, level: &mut Level, g: &GeoName, ctx: &Ctx<'_>) -> Option<Arc<PathLabel>> {
        if let Some(f) = level.fits.get(&g.id) {
            ctx.memo.set(ctx.memo.get() + 1);
            return f.clone();
        }
        ctx.computed.set(ctx.computed.get() + 1);
        let f = self.fit_one(g, ctx).map(Arc::new);
        level.fits.insert(g.id, f.clone());
        f
    }

    fn fit_one(&self, g: &GeoName, ctx: &Ctx<'_>) -> Option<PathLabel> {
        let on = Self::on(g, ctx);
        let rule = rule(g.kind, on);
        let max_zoom = match g.kind {
            GeoKind::Country => g.max_zoom.unwrap_or(rule.max_zoom).min(rule.max_zoom),
            GeoKind::Island => NE_ISLAND_MAX_ZOOM,
            _ => rule.max_zoom,
        };
        if ctx.zoom < (g.min_zoom - 0.5).max(min_zoom(g.kind)) || ctx.zoom > max_zoom {
            return None;
        }
        // Too big: the view is inside it, and its name says nothing here.
        if (g.bounds[2] - g.bounds[0]) * ctx.ppu > 1.5 * ctx.view.0 && (g.bounds[3] - g.bounds[1]) * ctx.ppu > 1.5 * ctx.view.1 {
            return None;
        }
        let local = self.local.borrow().get(&g.id).cloned();
        let name = local.as_deref().unwrap_or(g.name(self.lang));
        if name.is_empty() {
            return None;
        }
        let aka: Vec<String> = g.all_names().map(|n| n.trim().to_lowercase()).collect();
        if let Some(cape) = &g.cape {
            // Out from its point as an OSM cape is; Natural Earth has no coast to keep the letters on the water.
            let m = self.measure(name, &rule.style);
            let axis = CapeAxis { tip: cape.at, out: cape.normal };
            return cape_label((g.id as u64) << 8, name, &m, rule.style, &axis, ctx.ppu, ctx.zoom, aka, g.bounds, &|_| Some(true), ctx.avoid);
        }
        if let Some(at) = g.at {
            // A country is named inside its box: a small one is not named until it is big enough to hold its name.
            let label = self.level(g, name, at, country_style(g.rank), COUNTRY_WRAP_CHARS, ctx.ppu, aka.clone());
            let (bw, bh) = ((g.bounds[2] - g.bounds[0]) * ctx.ppu, (g.bounds[3] - g.bounds[1]) * ctx.ppu);
            let (lw, lh) = (label.extent[2] - label.extent[0], label.extent[3] - label.extent[1]);
            if lw <= bw * COUNTRY_ROOM && lh <= bh * COUNTRY_ROOM {
                return Some(label);
            }
            // An island state or territory (Martinique, Barbados) is named beside its island, on the sea, as
            // a chart names an island too small to hold its name; once seen as more than a speck.
            let small = (g.bounds[2] - g.bounds[0]).max(g.bounds[3] - g.bounds[1]) <= SMALL_COUNTRY_UNITS;
            if !g.island || !small || bw.max(bh) < ISLAND_STATE_MIN_PX {
                return None;
            }
            let right = [(g.bounds[2] * ctx.ppu + ISLAND_STATE_GAP_PX + lw / 2.0) / ctx.ppu, at[1]];
            return Some(self.level(g, name, right, country_style(g.rank), COUNTRY_WRAP_CHARS, ctx.ppu, aka).on_sea());
        }
        let spine = g.spine.as_ref()?;
        let along = set_along(g.kind, name, spine, ctx.ppu, g.id as u64, aka.clone(), g.bounds, &mut |text, style| self.measure(text, style), ctx.avoid);
        // An island's name is read across it, not up it: one that would stand on end (IRELAND) is written
        // level at the middle of its land, if the island is that wide.
        if g.kind == GeoKind::Island
            && let Some(l) = &along
            && mean_tilt(l) > ISLAND_MAX_TILT_DEG.to_radians()
        {
            let (u, _) = (0..=20).map(|k| spine.length() * k as f64 / 20.0).map(|u| (u, spine.clear_at(u))).max_by(|a, b| a.1.total_cmp(&b.1))?;
            let level = self.level(g, name, spine.at(u).0, l.style, COUNTRY_WRAP_CHARS, ctx.ppu, aka);
            return ((level.extent[2] - level.extent[0]) <= (g.bounds[2] - g.bounds[0]) * ctx.ppu).then_some(level);
        }
        if along.is_some() || g.kind != GeoKind::Ocean {
            return along;
        }
        // An ocean too long a name for its line is stacked, on two lines or word under word, smaller if need
        // be, at the line's widest open water, where the clearance under the whole block holds it.
        let mut spots: Vec<(f64, f64)> = (0..=40).map(|k| spine.length() * k as f64 / 40.0).map(|u| (u, spine.clear_at(u))).collect();
        spots.sort_by(|a, b| b.1.total_cmp(&a.1));
        for wrap_at in [STACK_TWO, 1] {
            for k in STACK_SHRINK {
                let style = Style { size: (rule.style.size * k).max(MIN_SIZE), ..rule.style };
                for &(u, _) in spots.iter().take(12) {
                    let label = self.level(g, name, spine.at(u).0, style, wrap_at, ctx.ppu, aka.clone());
                    let (half_w, half_h) = ((label.extent[2] - label.extent[0]) / 2.0 / ctx.ppu, ((label.extent[3] - label.extent[1]) / 2.0 + names::NAME_PAD) / ctx.ppu);
                    let held = (0..=8).all(|i| {
                        let v = u - half_w + 2.0 * half_w * i as f64 / 8.0;
                        v >= 0.0 && v <= spine.length() && spine.clear_at(v) >= half_h
                    });
                    if held {
                        return Some(label);
                    }
                }
            }
        }
        None
    }
}

impl RegionNames {
    /// `name` written level and centred on `at` (units), in capitals spread a little, cut into lines when
    /// longer than `wrap_at` characters (1: a word a line).
    #[allow(clippy::too_many_arguments)]
    fn level(&self, g: &GeoName, name: &str, at: [f64; 2], style: Style, wrap_at: usize, ppu: f64, aka: Vec<String>) -> PathLabel {
        let text = if style.caps { name.to_uppercase() } else { name.to_string() };
        let lines = if wrap_at <= 1 { text.split(' ').map(str::to_string).collect() } else { wrap(&text, wrap_at) };
        let measured: Vec<Rc<Measured>> = lines.iter().map(|l| self.measure(l, &style)).collect();
        let height = measured.iter().map(|m| m.height).fold(0.0, f64::max);
        let (cx, cy) = (at[0] * ppu, at[1] * ppu);
        let (mut glyphs, mut boxes) = (Vec::new(), Vec::new());
        for (k, m) in measured.iter().enumerate() {
            let track = if style.halo_is_sea() { OCEAN_STACK_TRACK_EM } else { COUNTRY_TRACK_EM } * m.em;
            let advances = m.advances();
            let total: f64 = advances.iter().sum::<f64>() + track * advances.len().saturating_sub(1) as f64;
            let y = cy + (k as f64 - (measured.len() - 1) as f64 / 2.0) * height * 0.92;
            let mut x = cx - total / 2.0;
            for ((t, adv), _) in m.clusters.iter().zip(&advances) {
                let at = labels::GlyphAt { x: x + adv / 2.0, y, angle: 0.0 };
                if !t.trim().is_empty() {
                    boxes.push(labels::box_of(at.x, at.y, 0.0, *adv, m.height));
                }
                glyphs.push((t.clone(), at, *adv));
                x += adv + track;
            }
        }
        let extent = boxes.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |e, b| [e[0].min(b[0]), e[1].min(b[1]), e[2].max(b[2]), e[3].max(b[3])]);
        PathLabel { id: (g.id as u64) << 8, style, glyphs, height, boxes, extent, aka, units: g.bounds, zoom: ppu.log2() }
    }
}

/// Sets `name` along `spine` (units) at `ppu` pixels to the unit, in the manner of `kind`: in capitals
/// where that kind is set so, spread to fill the spine, slid along the spine to keep its letters off what
/// `avoid` says no to, its words set apart round what is still in the way, and where it does not fit at its
/// size, written smaller or, for a water, on two lines. `measure` cuts a text into clusters (and may
/// remember the answer). `None` when it does not fit even at the smallest size.
#[allow(clippy::too_many_arguments)]
pub(super) fn set_along(kind: GeoKind, name: &str, spine: &Spine, ppu: f64, id: u64, aka: Vec<String>, units: [f64; 4], measure: &mut dyn FnMut(&str, &Style) -> Rc<Measured>, avoid: &dyn Fn(&[f64; 4]) -> bool) -> Option<PathLabel> {
    set_along_as(kind, &|r| r, None, name, spine, ppu, id, aka, units, measure, avoid)
}

/// `set_along`, with the kind's rule changed by `adjust`, and the name's middle as near as it fits to `anchor`
/// (a share of the spine's length) if given.
#[allow(clippy::too_many_arguments)]
pub(super) fn set_along_as(kind: GeoKind, adjust: &dyn Fn(Rule) -> Rule, anchor: Option<f64>, name: &str, spine: &Spine, ppu: f64, id: u64, aka: Vec<String>, units: [f64; 4], measure: &mut dyn FnMut(&str, &Style) -> Rc<Measured>, avoid: &dyn Fn(&[f64; 4]) -> bool) -> Option<PathLabel> {
    let b = spine.pts.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]);
    let on = (b[2] - b[0]).max(b[3] - b[1]) * ppu;
    let rule = adjust(rule(kind, on));
    let text = if rule.style.caps { name.to_uppercase() } else { name.to_string() };
    let params = FitParams { min_track_em: rule.track.0, max_track_em: rule.track.1, fill: rule.fill, max_turn_deg: rule.turn.0, total_turn_deg: rule.turn.1, halo_px: names::NAME_PAD, anchor };
    // An island's name is sized by the island; written smaller only where the land is too thin for it.
    let base = if kind == GeoKind::Island { island_em(&text, spine.length() * ppu)? } else { rule.style.size };
    // An island's name may be written a little smaller to keep clear of a light on it, on one line along the
    // island; a water's a little smaller, then on two lines, then smaller still.
    let lines = wrap(&text, names::WRAP_CHARS);
    let two = matches!(kind, GeoKind::Sea | GeoKind::Gulf | GeoKind::Strait | GeoKind::Bay) && lines.len() == 2;
    let (tries, least): (&[(f64, bool)], f64) = match kind {
        GeoKind::Island => (&[(1.0, false), (0.85, false), (0.72, false), (0.6, false)], ISLAND_MIN_PT),
        // A strait's name runs the length of the strait, smaller before it is stacked.
        GeoKind::Strait if two => (&[(1.0, false), (0.85, false), (0.72, false), (1.0, true), (0.85, true), (0.72, true)], MIN_SIZE),
        _ if two => (&[(1.0, false), (0.85, false), (1.0, true), (0.85, true), (0.72, false), (0.72, true)], MIN_SIZE),
        _ => (&[(1.0, false), (0.85, false), (0.72, false)], MIN_SIZE),
    };
    let mut tried: Vec<(f64, bool)> = Vec::new();
    for (step, &(k, stacked)) in tries.iter().enumerate() {
        // Never up to the least size: a small cove's name may start under it.
        let size = if k == 1.0 { base } else { (base * k).max(least.min(base)) };
        if tried.contains(&(size, stacked)) {
            continue;
        }
        tried.push((size, stacked));
        let style = Style { size, ..rule.style };
        let label = |clusters: Vec<(String, f64)>, height: f64, f: labels::Fitted| {
            let glyphs = clusters.into_iter().zip(f.glyphs).map(|((t, adv), at)| (t, at, adv)).collect();
            PathLabel { id: id << 8 | step as u64, style, glyphs, height, boxes: f.boxes, extent: f.extent, aka: aka.clone(), units, zoom: ppu.log2() }
        };
        if stacked {
            let ms: Vec<Rc<Measured>> = lines.iter().map(|l| measure(l, &style)).collect();
            let (adv, sp): (Vec<Vec<f64>>, Vec<Vec<bool>>) = ms.iter().map(|m| (m.advances(), m.spaces())).unzip();
            let block: Vec<labels::Line> = adv.iter().zip(&sp).map(|(a, s)| (a.as_slice(), s.as_slice())).collect();
            let height = ms.iter().map(|m| m.height).fold(0.0, f64::max);
            if let Some(f) = labels::fit_lines_avoiding(spine, ppu, &block, ms[0].em, height, &params, avoid) {
                return Some(label(ms.iter().flat_map(|m| m.clusters.iter().cloned()).collect(), height, f));
            }
            continue;
        }
        let m = measure(&text, &style);
        let params = if kind == GeoKind::Island { FitParams { halo_px: names::NAME_PAD - ISLAND_OVERHANG * m.height, ..params } } else { params };
        let (adv, sp) = (m.advances(), m.spaces());
        // Its words apart where the whole name, slid along, still meets a light.
        let hyphens: Vec<bool> = m.clusters.iter().map(|c| c.0.ends_with('-')).collect();
        let fitted = labels::fit_avoiding(spine, ppu, &adv, &sp, m.em, m.height, &params, avoid).or_else(|| labels::fit_words(spine, ppu, &adv, &sp, &hyphens, m.em, m.height, &params, avoid));
        if let Some(f) = fitted {
            return Some(label(m.clusters.clone(), m.height, f));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::geo::LatLon;
    use crate::palette::{Mode, Palette};
    use crate::view::{View, ViewInfo};

    fn area(lat: f64, lon: f64, zoom: f64) -> TileArea {
        let vi = ViewInfo { view: View { center: LatLon::new(lat, lon), zoom }, w: 1000.0, h: 700.0 };
        TileArea { view: vi, vi, pal: Palette::new(Mode::Day, false) }
    }

    fn named<'a>(labels: &'a [Arc<PathLabel>], en: &str, regions: &RegionNames) -> Option<&'a Arc<PathLabel>> {
        let g = naturalearth::all().iter().find(|g| g.names.get("en").is_some_and(|n| n.starts_with(en)))?;
        let _ = regions;
        labels.iter().find(|l| (l.id >> 8) as u32 == g.id)
    }

    fn text(l: &PathLabel) -> String {
        l.glyphs.iter().map(|g| g.0.as_str()).collect()
    }

    #[test]
    fn the_strait_of_gibraltar_is_named_along_the_strait_at_zoom_9_and_10() {
        let regions = RegionNames::with_language("en");
        let g = naturalearth::all().iter().find(|g| g.names["en"] == "Strait of Gibraltar").unwrap();
        let s = g.spine.as_ref().unwrap();
        let (a, b) = (s.pts[0], *s.pts.last().unwrap());
        let axis = (b[1] - a[1]).atan2(b[0] - a[0]);
        for zoom in [9.0, 10.0] {
            let labels = regions.labels_for(&area(36.0, -5.5, zoom));
            let l = named(&labels, "Strait of Gibraltar", &regions).unwrap_or_else(|| panic!("not named at zoom {zoom}: {:?}", labels.iter().map(|l| text(l)).collect::<Vec<_>>()));
            assert_eq!(text(l).replace(' ', ""), "STRAITOFGIBRALTAR".to_string());
            for (t, at, _) in &l.glyphs {
                if t.trim().is_empty() {
                    continue;
                }
                let mut d = (at.angle - axis).abs();
                if d > std::f64::consts::PI {
                    d = std::f64::consts::TAU - d;
                }
                assert!(d.to_degrees() < 30.0, "a letter at {} degrees to the strait at zoom {zoom}", d.to_degrees());
            }
        }
    }

    #[test]
    fn the_balearic_sea_is_tracked_wider_than_its_natural_width() {
        let regions = RegionNames::with_language("en");
        let labels = regions.labels_for(&area(39.5, 2.5, 7.0));
        let l = named(&labels, "Balearic Sea", &regions).unwrap_or_else(|| panic!("{:?}", labels.iter().map(|l| text(l)).collect::<Vec<_>>()));
        assert_eq!(text(l), "BALEARIC SEA");
        let natural: f64 = l.glyphs.iter().map(|g| g.2).sum();
        let (first, last) = (&l.glyphs[0].1, &l.glyphs.last().unwrap().1);
        let span = (last.x - first.x).hypot(last.y - first.y) + l.glyphs.last().unwrap().2;
        assert!(span > natural * 1.5, "spread over {span} px from {natural}");
        assert!(l.style.caps && l.style.style == pango::Style::Italic);
    }

    #[test]
    fn a_lower_priority_name_overlapping_a_higher_one_is_dropped_the_same_from_any_area() {
        // What is kept does not depend on the area asked: a name kept from a small area is kept from a
        // big one that holds it, and from another that overlaps it.
        let regions = RegionNames::with_language("en");
        let big = regions.labels_for(&area(38.0, 5.0, 6.0));
        assert!(!big.is_empty());
        let fresh = |lat, lon| RegionNames::with_language("en").labels_for(&area(lat, lon, 6.0));
        let (west, east) = (fresh(39.5, 2.5), fresh(36.0, 12.0));
        for l in west.iter().chain(&east) {
            let (a, b) = (area(38.0, 5.0, 6.0), area(39.5, 2.5, 6.0));
            let g = &naturalearth::all()[(l.id >> 8) as usize];
            let units = |a: &TileArea| naturalearth::units_of(&a.vi.bounds());
            // Both areas reach the names inside the overlap of their regions: they must agree on them.
            let (ua, ub) = (units(&a), units(&b));
            let inside = |u: [f64; 4]| g.bounds[0] >= u[0] && g.bounds[2] <= u[2] && g.bounds[1] >= u[1] && g.bounds[3] <= u[3];
            if inside(ua) && inside(ub) {
                assert!(big.iter().any(|x| x.id == l.id), "{} is kept from one area but not from another", text(l));
            }
        }
        let small = regions.labels_for(&area(39.5, 2.5, 6.0));
        for l in small.iter().filter(|l| {
            let g = &naturalearth::all()[(l.id >> 8) as usize];
            let u = naturalearth::units_of(&area(38.0, 5.0, 6.0).vi.bounds());
            g.bounds[0] >= u[0] && g.bounds[2] <= u[2] && g.bounds[1] >= u[1] && g.bounds[3] <= u[3]
        }) {
            assert!(big.iter().any(|b| b.id == l.id), "{} is kept from the small area but not from the big one", text(l));
        }
        // And what is kept never overlaps another kept name.
        for (i, x) in big.iter().enumerate() {
            for y in &big[i + 1..] {
                assert!(!x.boxes.iter().any(|a| y.boxes.iter().any(|b| overlap(a, b))), "{} and {} overlap", text(x), text(y));
            }
        }
    }

    #[test]
    fn the_azores_are_named_once_by_their_islands_not_also_as_a_territory() {
        for (lat, lon, zoom) in [(38.5, -28.0, 5.0), (38.7, -27.6, 7.0)] {
            let regions = RegionNames::with_language("en");
            let labels = regions.labels_for(&area(lat, lon, zoom));
            let azores: Vec<String> = labels.iter().map(|l| text(l)).filter(|t| t.replace(' ', "").contains("AZORES")).collect();
            assert_eq!(azores.len(), 1, "at zoom {zoom}: {azores:?}");
        }
    }

    #[test]
    fn an_island_takes_the_local_name_the_tiles_give_it_in_capitals_along_it() {
        let regions = RegionNames::with_language("en");
        let g = naturalearth::all().iter().find(|g| g.names["en"] == "Pico Island").unwrap();
        let s = g.spine.as_ref().unwrap();
        let mid = s.pts[s.pts.len() / 2];
        assert!(regions.learn(mid, "Ilha do Pico"));
        assert!(!regions.learn(mid, "Ilha do Pico"), "once");
        assert!(!regions.learn(mid, "Nowhere"), "not one of its names");
        let p = crate::core::geo::unproject(mid[0], mid[1], 0.0);
        let labels = regions.labels_for(&area(p.lat, p.lon, 10.0));
        let l = labels.iter().find(|l| (l.id >> 8) as u32 == g.id).expect("Pico is named at zoom 10");
        assert_eq!(text(l), "ILHA DO PICO");
        assert!(l.style.caps);
    }

    #[test]
    fn an_island_name_grows_with_the_island_as_the_view_zooms_in() {
        // A long straight island, 0.1 units long: its name is the same drawing at two zooms, scaled with it.
        let spine = Spine { pts: vec![[100.0, 100.0], [100.1, 100.0]], clear: vec![0.01, 0.01] };
        let set = |ppu: f64| set_along(GeoKind::Island, "Ile Longue", &spine, ppu, 1, vec![], [0.0; 4], &mut |t, st| Rc::new(names::measure(t, st)), &|_| false).unwrap();
        let ppu = 2f64.powf(10.6);
        let (a, b) = (set(ppu), set(ppu * 1.3));
        assert!((b.style.size / a.style.size - 1.3).abs() < 1e-9, "{} then {}", a.style.size, b.style.size);
        let span = |l: &PathLabel| l.extent[2] - l.extent[0];
        assert!((span(&b) / span(&a) - 1.3).abs() < 0.1, "{} then {}", span(&a), span(&b));
        // Far in it stops growing; far out it is too small to be written along the island.
        assert_eq!(set(2f64.powf(14.0)).style.size, ISLAND_MAX_PT);
        assert!(set_along(GeoKind::Island, "Ile Longue", &spine, 2f64.powf(4.0), 1, vec![], [0.0; 4], &mut |t, st| Rc::new(names::measure(t, st)), &|_| false).is_none());
    }

    #[test]
    fn a_long_name_is_cut_into_two_balanced_lines() {
        assert_eq!(wrap("FRANCE", 12), vec!["FRANCE"]);
        assert_eq!(wrap("UNITED STATES OF AMERICA", 12), vec!["UNITED STATES", "OF AMERICA"]);
        assert_eq!(wrap("Belle-Île-en-Mer", 10), vec!["Belle-", "Île-en-Mer"], "no space: after a hyphen");
        assert_eq!(wrap("Grandcombe", 5), vec!["Grandcombe"], "nowhere to cut");
    }

    #[test]
    fn names_follow_the_language() {
        let fr = RegionNames::with_language("fr").labels_for(&area(39.5, 2.5, 7.0));
        let en = RegionNames::with_language("en").labels_for(&area(39.5, 2.5, 7.0));
        let g = naturalearth::all().iter().find(|g| g.names["en"] == "Balearic Sea").unwrap();
        let find = |v: &[Arc<PathLabel>]| v.iter().find(|l| (l.id >> 8) as u32 == g.id).map(|l| text(l)).unwrap();
        assert_eq!(find(&fr), g.names["fr"].to_uppercase());
        assert_eq!(find(&en), "BALEARIC SEA");
        // A language the data does not have falls back to English.
        let xx = RegionNames::with_language("xx").labels_for(&area(39.5, 2.5, 7.0));
        assert_eq!(find(&xx), "BALEARIC SEA");
    }

    #[test]
    fn a_cape_is_written_on_the_sea_side() {
        let regions = RegionNames::with_language("en");
        let g = naturalearth::all().iter().find(|g| g.names["en"].starts_with("Cape Reinga")).unwrap();
        let cape = g.cape.unwrap();
        let (lat, lon) = {
            let p = crate::core::geo::unproject(cape.at[0], cape.at[1], 0.0);
            (p.lat, p.lon)
        };
        let labels = regions.labels_for(&area(lat, lon, 9.0));
        let l = named(&labels, "Cape Reinga", &regions).expect("Cape Reinga is named at zoom 9");
        let ppu = 2f64.powf(9.0);
        let (at, g0) = ((cape.at[0] * ppu, cape.at[1] * ppu), &l.glyphs[0].1);
        let off = [g0.x - at.0, g0.y - at.1];
        assert!(off[0] * cape.normal[0] + off[1] * cape.normal[1] > 4.0, "off the coast, towards the sea: {off:?}");
        assert!(off_line([at.0, at.1], baseline(l)) < 0.01, "its baseline runs through the point");
        assert!(!l.style.caps);
    }

    fn cape_style() -> Style {
        Style::land(10.5, pango::Weight::Normal, 0.0).on_sea()
    }

    /// `name` as a cape at `tip` (world pixels at zoom 0) pointing `out`, on the water `water` says, clear of
    /// what `avoid` says no to.
    fn cape_avoiding(name: &str, tip: Pt, out: Pt, water: &dyn Fn(Pt) -> bool, avoid: &dyn Fn(&[f64; 4]) -> bool) -> Option<PathLabel> {
        let m = RegionNames::with_language("en").measure(name, &cape_style());
        cape_label(1, name, &m, cape_style(), &CapeAxis { tip, out }, 1.0, 0.0, vec![], [0.0; 4], &|p| Some(water(p)), avoid)
    }

    fn cape(name: &str, tip: Pt, out: Pt, water: &dyn Fn(Pt) -> bool) -> Option<PathLabel> {
        cape_avoiding(name, tip, out, water, &|_| false)
    }

    /// Where the text's baseline begins and ends, from its one glyph.
    fn baseline(l: &PathLabel) -> (Pt, Pt) {
        let (g, w) = (&l.glyphs[0].1, l.glyphs[0].2);
        let m = RegionNames::with_language("en").measure(&l.glyphs[0].0, &l.style);
        let below = m.baseline - m.height / 2.0;
        let (c, s) = (g.angle.cos(), g.angle.sin());
        let base = [g.x - s * below, g.y + c * below];
        ([base[0] - c * w / 2.0, base[1] - s * w / 2.0], [base[0] + c * w / 2.0, base[1] + s * w / 2.0])
    }

    /// How far `p` is from the line through `a` and `b`.
    fn off_line(p: Pt, (a, b): (Pt, Pt)) -> f64 {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx).abs() / dx.hypot(dy)
    }

    #[test]
    fn a_cape_is_named_out_along_its_point_from_just_off_its_tip() {
        // A point running north-east (y grows down the screen): the sea all round its end.
        let out = [0.6, -0.8];
        let land = |p: Pt| p[0] * out[0] + p[1] * out[1] < 0.0;
        let l = cape("Pta. Son Serralta", [0.0, 0.0], out, &|p| !land(p)).expect("written along the point");
        let (a, b) = baseline(&l);
        assert!((l.glyphs[0].1.angle - (-0.8f64).atan2(0.6)).abs() < 1e-9, "along the point, read upwards to the right");
        assert!(off_line([0.0, 0.0], (a, b)) < 0.01, "its baseline runs through the tip: {a:?} {b:?}");
        let near = a[0].hypot(a[1]);
        assert!((near - CAPE_OFFSET_PX).abs() < 0.5 && b[0].hypot(b[1]) > near, "begins just off the tip: {a:?} {b:?}");
        // Pointing west, it is still upright, and ends just off the tip, standing on the line through it.
        let l = cape("Pta. Rotja", [0.0, 0.0], [-1.0, 0.0], &|p| p[0] < 0.0).unwrap();
        let (a, b) = baseline(&l);
        assert!(l.glyphs[0].1.angle.abs() < 1e-9 && (b[0] + CAPE_OFFSET_PX).abs() < 0.5 && a[0] < b[0], "{a:?} {b:?}");
        assert!(a[1].abs() < 0.01 && b[1].abs() < 0.01 && l.glyphs[0].1.y < 0.0, "level with the tip, the letters above it: {a:?} {b:?}");
    }

    #[test]
    fn a_cape_on_a_shore_running_east_west_is_named_square_to_it_and_never_on_land() {
        // Sea to the north (up the screen): the name stands square to the shore, from just off the tip.
        let l = cape("Montauk Point", [0.0, 0.0], [0.0, -1.0], &|p| p[1] < 0.0).expect("along the point");
        let (a, b) = baseline(&l);
        assert!((l.glyphs[0].1.angle.abs() - FRAC_PI_2).abs() < 1e-9, "{:?}", l.glyphs[0].1);
        assert!(off_line([0.0, 0.0], (a, b)) < 0.01, "its baseline runs through the tip: {a:?} {b:?}");
        assert!(a[1].max(b[1]) < 0.0 && (a[1].max(b[1]) + CAPE_OFFSET_PX).abs() < 0.5, "begins just off the tip: {a:?} {b:?}");
        // A mark on the tip: as near as it is clear, not far out, and still on a line through the tip.
        let mark = |b: &[f64; 4]| b[0] < 6.0 && b[2] > -6.0 && b[1] < 2.0 && b[3] > -14.0;
        let l = cape_avoiding("Montauk Point", [0.0, 0.0], [0.0, -1.0], &|p| p[1] < 0.0, &mark).expect("clear of the mark");
        assert!(!l.boxes.iter().any(mark));
        let (a, b) = baseline(&l);
        let near = a[0].hypot(a[1]).min(b[0].hypot(b[1]));
        assert!(near < 24.0 && off_line([0.0, 0.0], (a, b)) < 0.01, "next to the mark: {near}");
        // The water ends a little out from the tip: along the point, the name would run ashore.
        let island = |p: Pt| p[0] > 30.0 && p[0] < 300.0 && p[1].abs() < 100.0;
        if let Some(l) = cape("Pta. Son Serralta", [0.0, 0.0], [1.0, 0.0], &|p| p[0] > 0.0 && !island(p)) {
            assert!(l.glyphs[0].1.angle.abs() > 0.1 && off_line([0.0, 0.0], baseline(&l)) < 0.01);
        }
        assert!(cape("Pta. Son Serralta", [0.0, 0.0], [1.0, 0.0], &|_| false).is_none());
    }

    #[test]
    fn a_cape_whose_point_is_in_the_way_turns_round_its_tip() {
        // A lighthouse's flare out along the point (south-east): the name turns off it, as near as it is clear,
        // its baseline still through the tip, never far out beside it.
        let out = [std::f64::consts::FRAC_1_SQRT_2, std::f64::consts::FRAC_1_SQRT_2];
        let land = |p: Pt| p[0] * out[0] + p[1] * out[1] < 0.0;
        let flare = |b: &[f64; 4]| (0..=12).any(|k| {
            let p = [out[0] * 3.0 * k as f64, out[1] * 3.0 * k as f64];
            b[0] < p[0] + 4.0 && b[2] > p[0] - 4.0 && b[1] < p[1] + 4.0 && b[3] > p[1] - 4.0
        });
        let l = cape_avoiding("C. de San Sebastian", [0.0, 0.0], out, &|p| !land(p), &flare).expect("turned clear of the flare");
        let (a, b) = baseline(&l);
        assert!(off_line([0.0, 0.0], (a, b)) < 0.01, "{a:?} {b:?}");
        assert!(a[0].hypot(a[1]).min(b[0].hypot(b[1])) < CAPE_OFFSET_PX + 15.0, "near the tip: {a:?} {b:?}");
    }

    #[test]
    fn no_name_is_written_where_the_view_is_inside_it() {
        let regions = RegionNames::with_language("en");
        let labels = regions.labels_for(&area(39.5, 2.5, 12.0));
        assert!(named(&labels, "Balearic Sea", &regions).is_none(), "seas are not named past zoom 11");
        // The world is named by its oceans, its biggest seas, gulfs (the Bay of Bengal), islands and island groups, and its countries.
        let world = regions.labels_for(&area(0.0, -30.0, 2.0));
        let kinds: Vec<GeoKind> = world.iter().map(|l| naturalearth::all()[(l.id >> 8) as usize].kind).collect();
        assert!(kinds.iter().all(|k| matches!(k, GeoKind::Ocean | GeoKind::Sea | GeoKind::Gulf | GeoKind::Bay | GeoKind::IslandGroup | GeoKind::Island | GeoKind::Country)), "{kinds:?}");
        assert!(kinds.contains(&GeoKind::Ocean) && kinds.contains(&GeoKind::Country));
    }

    #[test]
    fn the_pacific_is_named_from_either_side_of_the_antimeridian_and_a_small_country_beside_its_island() {
        let regions = RegionNames::with_language("en");
        for lon in [170.0, -150.0] {
            let labels = regions.labels_for(&area(25.0, lon, 3.0));
            assert!(named(&labels, "North Pacific", &regions).is_some(), "at {lon}: {:?}", labels.iter().map(|l| text(l)).collect::<Vec<_>>());
        }
        let labels = regions.labels_for(&area(14.6, -61.0, 7.0));
        let l = named(&labels, "Martinique", &regions).expect("Martinique is named at zoom 7");
        let g = naturalearth::all().iter().find(|g| g.names["en"] == "Martinique" && g.kind == GeoKind::Country).unwrap();
        assert!(l.extent[0] >= g.bounds[2] * 2f64.powf(7.0), "beside the island, east of it");
        assert!(l.style.halo_is_sea());
        // A big country is named inside its box, and a tiny one is not named at all far out.
        let labels = regions.labels_for(&area(46.0, 2.0, 5.0));
        assert!(named(&labels, "France", &regions).is_some());
        assert!(regions.labels_for(&area(14.6, -61.0, 3.0)).iter().all(|l| text(l) != "MARTINIQUE"));
    }
}
