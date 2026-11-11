//! Names written on the chart (towns, sea areas, capes…): laid out by Pango,
//! outlined once into a small image and copied wherever they are drawn.
//!
//! As on a paper chart, waters are named in sloping blue on a sea-coloured
//! outline, land in upright black on a land-coloured one, and the size says how
//! much the name stands for. A layer places its names for a tile before it
//! draws anything (`Placed`), so the soundings under them can be left out.

use super::{LabelPlacer, text_scale};
use pango::glib::translate::IntoGlib;
use crate::core::labels::GlyphAt;
use crate::palette::{LAND, Palette, SEA};
use crate::tokens::Token;
use crate::view::ViewInfo;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

/// How a kind of name is written.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub size: f64,
    pub weight: pango::Weight,
    pub style: pango::Style,
    /// The radius of the dot that marks a town, or 0 for a name centred on its place.
    pub dot: f64,
    pub fill: Token,
    /// The outline that keeps it legible: the colour of what it is written on.
    pub halo: Token,
    /// Written in capitals: whoever builds the text upper-cases it first (`str::to_uppercase`).
    pub caps: bool,
}

impl Style {
    /// Upright black on land, with a dot of radius `dot` beside it when not 0.
    pub const fn land(size: f64, weight: pango::Weight, dot: f64) -> Style {
        Style { size, weight, style: pango::Style::Normal, dot, fill: Token::ChartInk, halo: LAND, caps: false }
    }

    /// Sloping blue on the sea.
    pub const fn water(size: f64) -> Style {
        Style { size, weight: pango::Weight::Normal, style: pango::Style::Italic, dot: 0.0, fill: Token::ChartNameWater, halo: SEA, caps: false }
    }

    /// Whether it is outlined in the sea's colour.
    pub fn halo_is_sea(&self) -> bool {
        self.halo == SEA
    }

    /// The same, outlined in the sea's colour, for a land name written on the water.
    pub fn on_sea(self) -> Style {
        Style { halo: SEA, ..self }
    }

    /// The same, written in capitals.
    pub const fn capitals(self) -> Style {
        Style { caps: true, ..self }
    }

    /// Sloping black on the sea: rocks, shoals and wrecks.
    pub const fn danger(size: f64) -> Style {
        Style { size, weight: pango::Weight::Normal, style: pango::Style::Italic, dot: 0.0, fill: Token::ChartInk, halo: SEA, caps: false }
    }
}

/// Room kept around a name, so names do not crowd each other.
pub const LABEL_PAD: (f64, f64) = (10.0, 4.0);

/// A name laid out by Pango, which falls back to a font that has each
/// character (Arabic, Greek, Cyrillic, CJK…); Cairo's own text drawing does not.
fn layout(cr: &cairo::Context, text: &str, st: &Style) -> pango::Layout {
    let layout = pangocairo::functions::create_layout(cr);
    let mut font = pango::FontDescription::new();
    font.set_family(crate::layers::FONT_FAMILY);
    font.set_weight(st.weight);
    font.set_style(st.style);
    font.set_absolute_size(st.size * text_scale() as f64 / 100.0 * pango::SCALE as f64);
    layout.set_font_description(Some(&font));
    // A name on two lines is centred line by line.
    layout.set_alignment(pango::Alignment::Center);
    layout.set_text(text);
    crate::fonts::hold_layout(&layout);
    layout
}

/// A name with its outline, drawn once: laying it out and outlining its glyphs was most of what a
/// tile of towns cost, and every tile round it drew the same names again.
pub struct Name {
    surface: cairo::ImageSurface,
    /// What it says, lines and all.
    pub text: String,
    /// The size of the text, in points, without the outline.
    pub w: f64,
    pub h: f64,
}

/// Room round a name's text for its outline, in points.
pub const NAME_PAD: f64 = 2.5;
/// Names kept per thread; all are dropped when there are this many.
const NAMES_KEPT: usize = 4096;

type Key = (String, [u64; 8], i32, bool);

thread_local! {
    static NAMES: RefCell<HashMap<Key, Rc<Name>>> = RefCell::new(HashMap::new());
}

/// `text` in the style `st`, outlined, at device scale `scale`, from this thread's cache.
pub fn name(cr: &cairo::Context, text: &str, st: &Style, pal: &Palette, scale: f64) -> Option<Rc<Name>> {
    let (fill, halo) = (pal.rgba(st.fill), pal.rgba(st.halo));
    let bits = [st.size * text_scale() as f64, scale, fill.0, fill.1, fill.2, halo.0, halo.1, halo.2].map(f64::to_bits);
    let key = (text.to_string(), bits, st.weight.into_glib(), st.style == pango::Style::Italic);
    if let Some(n) = NAMES.with(|c| c.borrow().get(&key).cloned()) {
        return Some(n);
    }
    let text_layout = layout(cr, text, st);
    let (w, h) = text_layout.pixel_size();
    let (w, h) = (w as f64, h as f64);
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, ((w + 2.0 * NAME_PAD) * scale).ceil() as i32, ((h + 2.0 * NAME_PAD) * scale).ceil() as i32).ok()?;
    surface.set_device_scale(scale, scale);
    {
        let c = cairo::Context::new(&surface).ok()?;
        if let Ok(o) = cr.font_options() {
            c.set_font_options(&o);
        }
        let text_layout = layout(&c, text, st);
        c.move_to(NAME_PAD, NAME_PAD);
        pangocairo::functions::layout_path(&c, &text_layout);
        pal.set_alpha(&c, st.halo, 0.85);
        c.set_line_width(2.5);
        c.set_line_join(cairo::LineJoin::Round);
        let _ = c.stroke_preserve();
        pal.set(&c, st.fill);
        let _ = c.fill();
    }
    let n = Rc::new(Name { surface, text: text.to_string(), w, h });
    NAMES.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() >= NAMES_KEPT {
            c.clear();
        }
        c.insert(key, n.clone());
    });
    Some(n)
}

/// A name given its place in a tile.
pub struct Placed {
    pub name: Rc<Name>,
    /// How it is written.
    pub style: Style,
    /// The top-left of its text.
    pub x: f64,
    pub top: f64,
    /// The town's dot, `(x, y, radius)`, with its colour.
    pub dot: Option<(f64, f64, f64, Token)>,
}

impl Placed {
    /// The same placed on a view `k` times bigger round the same centre (`at_label_zoom`): where the chart
    /// takes it, written as much bigger.
    pub fn scaled(&self, cr: &cairo::Context, pal: &Palette, k: f64) -> Option<Placed> {
        if k == 1.0 {
            return Some(Placed { name: self.name.clone(), style: self.style, x: self.x, top: self.top, dot: self.dot });
        }
        let style = Style { size: self.style.size * k, dot: self.style.dot * k, ..self.style };
        let name = name(cr, &self.name.text, &style, pal, cr.target().device_scale().0)?;
        // Kept on the same middle: the text at the bigger size is not exactly k times as wide.
        let (cx, cy) = ((self.x + self.name.w / 2.0) * k, (self.top + self.name.h / 2.0) * k);
        let (x, top) = match self.dot {
            // A town's name stays beside its dot, on the same side.
            Some((dx, _, _, _)) if self.x >= dx => (self.x * k, cy - name.h / 2.0),
            Some(_) => ((self.x + self.name.w) * k - name.w, cy - name.h / 2.0),
            None => (cx - name.w / 2.0, cy - name.h / 2.0),
        };
        let dot = self.dot.map(|(x, y, r, c)| (x * k, y * k, r * k, c));
        Some(Placed { name, style, x, top, dot })
    }

    /// The box it keeps clear of soundings and other names, `[x0, y0, x1, y1]`.
    pub fn rect(&self) -> [f64; 4] {
        let text = [self.x - NAME_PAD, self.top - NAME_PAD, self.x + self.name.w + NAME_PAD, self.top + self.name.h + NAME_PAD];
        match self.dot {
            // The dot is on either side of the text.
            Some((x, y, r, _)) => [text[0].min(x - r), text[1].min(y - r), text[2].max(x + r), text[3].max(y + r)],
            None => text,
        }
    }
}

/// The farthest a water's name is moved from its point to keep it off the
/// land, in pixels: a name is never moved onto another water.
pub const MAX_NUDGE_PX: (f64, f64) = (30.0, 18.0);

/// Where to try a name centred on (`x`, `y`) of `w` by `h`: there first, then,
/// for a water's name that would cross the coast, moved by a quarter of its
/// width and a line at most, never more than `MAX_NUDGE_PX`, nearest first.
pub fn nudges(x: f64, y: f64, w: f64, h: f64, nudge: bool) -> Vec<(f64, f64)> {
    let (x0, top) = (x - w / 2.0, y - h / 2.0);
    if !nudge {
        return vec![(x0, top)];
    }
    let (sx, sy) = ((w / 4.0).min(MAX_NUDGE_PX.0), h.min(MAX_NUDGE_PX.1));
    let mut at: Vec<(f64, f64)> = Vec::new();
    for dx in [0.0, -sx, sx] {
        for dy in [0.0, -sy, sy] {
            at.push((dx, dy));
        }
    }
    at.sort_by(|a, b| a.0.hypot(a.1).total_cmp(&b.0.hypot(b.1)));
    at.into_iter().map(|(dx, dy)| (x0 + dx, top + dy)).collect()
}

/// How much smaller a water's name may be written where it does not fit at
/// its size, and the smallest a name gets.
const SHRINK: [f64; 3] = [1.0, 0.85, 0.72];
const MIN_SIZE: f64 = 10.0;
const TOWN_SHRINK: [f64; 4] = [1.0, 0.85, 0.72, 0.6];

/// A name longer than this, in characters, may be set on two lines where it does not fit on one.
pub const WRAP_CHARS: usize = 10;

/// Fits the name `text`, centred on (`x`, `y`): at its size, or for a water's
/// name (`water`) moved a little or written smaller, wherever `ok` (given the
/// text's top-left, width and height) holds and `placer` has room; a long
/// name on two lines where one does not fit. `None` when it does not fit: it
/// is left out rather than moved away from what it names.
#[allow(clippy::too_many_arguments)]
pub fn fit(cr: &cairo::Context, pal: &Palette, placer: &mut LabelPlacer, text: &str, st: Style, x: f64, y: f64, water: bool, ok: impl Fn(f64, f64, f64, f64) -> bool) -> Option<(Rc<Name>, Style, f64, f64)> {
    if let Some(found) = fit_lines(cr, pal, placer, text, st, x, y, water, &ok) {
        return Some(found);
    }
    let lines = super::regions::wrap(text, WRAP_CHARS);
    // A town is named on one line beside its dot, or not at all.
    if lines.len() < 2 || st.dot > 0.0 {
        return None;
    }
    fit_lines(cr, pal, placer, &lines.join("\n"), st, x, y, water, &ok)
}

#[allow(clippy::too_many_arguments)]
fn fit_lines(cr: &cairo::Context, pal: &Palette, placer: &mut LabelPlacer, text: &str, st: Style, x: f64, y: f64, water: bool, ok: &impl Fn(f64, f64, f64, f64) -> bool) -> Option<(Rc<Name>, Style, f64, f64)> {
    let scale = cr.target().device_scale().0;
    // A town may be written down to 60% of its size to stay on its land.
    let (sizes, least) = if st.dot > 0.0 { (&TOWN_SHRINK[..], 0.0) } else if water { (&SHRINK[..], MIN_SIZE) } else { (&SHRINK[..1], MIN_SIZE) };
    let mut tried = 0.0;
    for k in sizes {
        // Never up to the least size: a small cove's name may start under it.
        let st = Style { size: if *k == 1.0 { st.size } else { (st.size * k).max(least.min(st.size)) }, ..st };
        if st.size == tried {
            break;
        }
        tried = st.size;
        let name = name(cr, text, &st, pal, scale)?;
        let (w, h) = (name.w, name.h);
        // A town's name goes to the right of its dot, else to the left.
        let at = if st.dot > 0.0 { vec![(x + st.dot + 3.0, y - h / 2.0), (x - st.dot - 3.0 - w, y - h / 2.0)] } else { nudges(x, y, w, h, water) };
        if let Some((x, top)) = try_spots(placer, &name, &at, |x, top| ok(x, top, w, h)) {
            return Some((name, st, x, top));
        }
    }
    None
}

/// Points over a name's box, about every 10 px, to test what it is written on.
pub fn samples(x: f64, top: f64, w: f64, h: f64) -> impl Iterator<Item = (f64, f64)> {
    let nx = (w / 10.0).ceil().max(1.0) as usize;
    (0..=nx).flat_map(move |i| [0.0, 0.5, 1.0].map(move |f| (x + w * i as f64 / nx as f64, top + h * f)))
}

/// Whether most of a name's box, at (`x`, `top`) and `w` by `h`, is where `on` holds.
pub fn mostly(x: f64, top: f64, w: f64, h: f64, on: impl Fn(f64, f64) -> bool) -> bool {
    let (n, yes) = samples(x, top, w, h).fold((0, 0), |(n, yes), (sx, sy)| (n + 1, yes + on(sx, sy) as usize));
    2 * yes > n
}

/// The size of a water's name for an area `on_screen` pixels across: the bigger the water, the bigger its name.
pub fn water_size(on_screen: f64) -> f64 {
    match on_screen {
        e if e < 150.0 => 12.5,
        e if e < 400.0 => 14.0,
        e if e < 900.0 => 16.0,
        _ => 18.0,
    }
}

/// Takes the first spot of `at` where `ok` holds and `placer` has room for a name of `name`'s size.
pub fn try_spots(placer: &mut LabelPlacer, name: &Name, at: &[(f64, f64)], ok: impl Fn(f64, f64) -> bool) -> Option<(f64, f64)> {
    at.iter().copied().find(|&(x, top)| ok(x, top) && placer.try_place(x - LABEL_PAD.0 / 2.0, top - LABEL_PAD.1 / 2.0, name.w + LABEL_PAD.0, name.h + LABEL_PAD.1))
}

/// Draws the names placed, where the tile `cr` draws shows them.
pub fn draw(cr: &cairo::Context, pal: &Palette, placed: &[Placed]) {
    let scale = cr.target().device_scale().0;
    let (x0, y0, x1, y1) = cr.clip_extents().unwrap_or((f64::MIN, f64::MIN, f64::MAX, f64::MAX));
    for p in placed {
        let r = p.rect();
        if r[2] < x0 || r[0] > x1 || r[3] < y0 || r[1] > y1 {
            continue;
        }
        if let Some((x, y, radius, colour)) = p.dot {
            cr.new_path();
            cr.arc(x, y, radius, 0.0, std::f64::consts::TAU);
            pal.set(cr, colour);
            let _ = cr.fill();
        }
        blit(cr, &p.name, p.x, p.top, scale);
    }
}

/// Copies `name` with its text's top-left at (`x`, `top`), on a whole device
/// pixel so it is copied, not resampled.
fn blit(cr: &cairo::Context, name: &Name, x: f64, top: f64, scale: f64) {
    blit_surface(cr, &name.surface, x - NAME_PAD, top - NAME_PAD, scale);
}

/// Copies `surface` with its top-left at (`left`, `top`), on a whole device pixel.
fn blit_surface(cr: &cairo::Context, surface: &cairo::ImageSurface, left: f64, top: f64, scale: f64) {
    let m = cr.matrix();
    let at = |v: f64, o: f64| ((o + v) * scale).round() / scale - o;
    let (dx, dy) = (at(left, m.x0()), at(top, m.y0()));
    let _ = cr.save();
    if cr.set_source_surface(surface, dx, dy).is_ok() {
        cr.source().set_filter(cairo::Filter::Nearest);
        cr.rectangle(dx, dy, surface.width() as f64 / scale, surface.height() as f64 / scale);
        let _ = cr.fill();
    }
    let _ = cr.restore();
}

/// The font options of everything written along a path: unhinted, so that the advances measured when a
/// name is fitted are the ones its letters are drawn with, at any device scale.
pub fn path_font_options() -> cairo::FontOptions {
    let mut o = cairo::FontOptions::new().expect("font options");
    o.set_hint_metrics(cairo::HintMetrics::Off);
    o.set_hint_style(cairo::HintStyle::None);
    o.set_antialias(cairo::Antialias::Gray);
    o
}

/// A name cut into clusters (what a reader takes for one letter), each with its width.
#[derive(Clone, Debug, PartialEq)]
pub struct Measured {
    pub clusters: Vec<(String, f64)>,
    /// The font size in pixels, the height of a line, and how far down the line its baseline is.
    pub em: f64,
    pub height: f64,
    pub baseline: f64,
}

impl Measured {
    pub fn advances(&self) -> Vec<f64> {
        self.clusters.iter().map(|c| c.1).collect()
    }

    pub fn spaces(&self) -> Vec<bool> {
        self.clusters.iter().map(|c| c.0.chars().all(char::is_whitespace)).collect()
    }
}

/// Scripts whose letters join or reorder, and so cannot be spread apart or turned one by one.
fn is_complex(c: char) -> bool {
    matches!(c as u32, 0x0590..=0x08FF | 0x0900..=0x0DFF | 0x0E00..=0x0E7F | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF)
}

thread_local! {
    static MEASURING: cairo::Context = {
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 1, 1).expect("1x1 surface");
        let cr = cairo::Context::new(&surface).expect("context");
        cr.set_font_options(&path_font_options());
        cr
    };
}

/// `text` in the style `st`, cut into clusters with their widths: letters of the Latin, Greek and Cyrillic
/// kind one by one, a script that joins (Arabic, Hebrew, Indic, Thai) as one piece, set straight.
pub fn measure(text: &str, st: &Style) -> Measured {
    MEASURING.with(|cr| {
        let layout = layout(cr, text, st);
        let em = st.size * text_scale() as f64 / 100.0;
        let height = layout.pixel_size().1 as f64;
        let baseline = layout.baseline() as f64 / pango::SCALE as f64;
        if text.chars().any(is_complex) {
            return Measured { clusters: vec![(text.to_string(), layout.pixel_size().0 as f64)], em, height, baseline };
        }
        let mut clusters = Vec::new();
        let mut iter = layout.iter();
        loop {
            let start = iter.index() as usize;
            let width = iter.cluster_extents().1.width() as f64 / pango::SCALE as f64;
            let more = iter.next_cluster();
            let end = if more { iter.index() as usize } else { text.len() };
            if let Some(t) = text.get(start..end) {
                clusters.push((t.to_string(), width));
            }
            if !more {
                break;
            }
        }
        Measured { clusters, em, height, baseline }
    })
}

/// Glyph images kept per thread; all are dropped when there are this many.
const GLYPHS_KEPT: usize = 8192;

type GlyphKey = (String, [u64; 8], i32, bool, i32);

thread_local! {
    static GLYPHS: RefCell<HashMap<GlyphKey, Rc<Name>>> = RefCell::new(HashMap::new());
}

/// One cluster of a name, turned to `angle_deg` and outlined, centred on its image: from this thread's
/// cache, which is by whole degrees of turn.
fn glyph(text: &str, adv: f64, height: f64, st: &Style, angle_deg: i32, pal: &Palette, scale: f64) -> Option<Rc<Name>> {
    let (fill, halo) = (pal.rgba(st.fill), pal.rgba(st.halo));
    let bits = [st.size * text_scale() as f64, scale, fill.0, fill.1, fill.2, halo.0, halo.1, halo.2].map(f64::to_bits);
    let key = (text.to_string(), bits, st.weight.into_glib(), st.style == pango::Style::Italic, angle_deg);
    if let Some(n) = GLYPHS.with(|c| c.borrow().get(&key).cloned()) {
        return Some(n);
    }
    let side = ((adv.hypot(height) + 2.0 * NAME_PAD) * scale).ceil().max(1.0);
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, side as i32, side as i32).ok()?;
    surface.set_device_scale(scale, scale);
    {
        let c = cairo::Context::new(&surface).ok()?;
        c.set_font_options(&path_font_options());
        let text_layout = layout(&c, text, st);
        c.translate(side / 2.0 / scale, side / 2.0 / scale);
        c.rotate((angle_deg as f64).to_radians());
        c.move_to(-adv / 2.0, -height / 2.0);
        pangocairo::functions::layout_path(&c, &text_layout);
        pal.set_alpha(&c, st.halo, 0.85);
        c.set_line_width(2.5);
        c.set_line_join(cairo::LineJoin::Round);
        let _ = c.stroke_preserve();
        pal.set(&c, st.fill);
        let _ = c.fill();
    }
    let n = Rc::new(Name { surface, text: text.to_string(), w: side / scale, h: side / scale });
    GLYPHS.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() >= GLYPHS_KEPT {
            c.clear();
        }
        c.insert(key, n.clone());
    });
    Some(n)
}

/// A name set along a path, its letters placed in world pixels at the zoom of the view it was fitted for.
#[derive(Clone, Debug)]
pub struct PathLabel {
    /// Tells it from another label, or the same one at another size.
    pub id: u64,
    pub style: Style,
    /// Each cluster of the text, where it goes and how wide it is.
    pub glyphs: Vec<(String, GlyphAt, f64)>,
    pub height: f64,
    /// The box of each letter, `[x0, y0, x1, y1]` in world pixels, and of all of them.
    pub boxes: Vec<[f64; 4]>,
    pub extent: [f64; 4],
    /// Every name of what it names, lower case: an OSM name of the same thing is not written again.
    pub aka: Vec<String>,
    /// The box of what it names, in units.
    pub units: [f64; 4],
    /// The zoom its world pixels are at: the one it was fitted at (`label_zoom`).
    pub zoom: f64,
}

/// Names are placed only at the zooms where the marks and lights they keep clear of show or hide: every
/// whole zoom (`aids::Declutter` decides at these zooms too, `mark_zoom`). Between two, they grow with the
/// chart, as if written on it, up to twice their size, and their shapes and places do not change: placing
/// them again at every step of a zoom moved them, and changed which were written. A wheel notch is half a
/// zoom, so every other notch grows the names rather than placing them again.
pub const LABEL_STEP: f64 = 1.0;

/// From this zoom names keep clear of every mark and light, as if all were shown (crowding still leaves a
/// few lights out here, none by 16 or 17 in the SHOM marks), so the marks that show as the view zooms in
/// never come under a name, and names are placed only every `SETTLED_STEP` zooms: they grow with the chart
/// up to four times their size. Every kind of name shows by here (suburbs from 14.5).
pub const SETTLED_ZOOM: f64 = 15.0;
const SETTLED_STEP: f64 = 2.0;

/// The zoom names are placed at for a view at `zoom`: the last step at or below it. The marks are then at
/// their biggest against the names, so what is clear of them there stays clear up to the next step.
pub fn label_zoom(zoom: f64) -> f64 {
    // A zoom quantized to hundredths a hair under a step is on it.
    let z = zoom + 1e-6;
    if z >= SETTLED_ZOOM {
        return SETTLED_ZOOM + ((z - SETTLED_ZOOM) / SETTLED_STEP).floor() * SETTLED_STEP;
    }
    (z / LABEL_STEP).floor() * LABEL_STEP
}

/// The zoom which marks and lights are shown as at for a view at `zoom` (`aids::Declutter`): the last whole zoom.
pub fn mark_zoom(zoom: f64) -> f64 {
    (zoom + 1e-6).floor()
}

/// Whether names placed at the step `zoom` keep clear of every mark and light, shown or not (`SETTLED_ZOOM`).
pub fn settled(zoom: f64) -> bool {
    zoom + 1e-6 >= SETTLED_ZOOM
}

/// `vi` as it is at `zoom`: the same ground round the same centre, and how many times bigger `vi` is than that.
pub fn view_at(vi: &ViewInfo, zoom: f64) -> (ViewInfo, f64) {
    let k = 2f64.powf(vi.zoom() - zoom);
    (ViewInfo { view: crate::view::View { center: vi.view.center, zoom }, w: vi.w / k, h: vi.h / k }, k)
}

/// `vi` as it is at its `label_zoom`, and how many times bigger the view is than that (1 to 2, or 4 once
/// settled).
pub fn at_label_zoom(vi: &ViewInfo) -> (ViewInfo, f64) {
    view_at(vi, label_zoom(vi.zoom()))
}

impl PathLabel {
    /// The same, outlined in the sea's colour: a land name written on the water.
    pub fn on_sea(self) -> PathLabel {
        PathLabel { style: self.style.on_sea(), ..self }
    }

    /// The same at `zoom`: its letters where the chart takes them, and as much bigger or smaller.
    pub fn at_zoom(&self, zoom: f64) -> PathLabel {
        let k = 2f64.powf(zoom - self.zoom);
        let scale = |b: &[f64; 4]| b.map(|v| v * k);
        PathLabel {
            id: self.id,
            style: Style { size: self.style.size * k, ..self.style },
            glyphs: self.glyphs.iter().map(|(t, g, adv)| (t.clone(), GlyphAt { x: g.x * k, y: g.y * k, angle: g.angle }, adv * k)).collect(),
            height: self.height * k,
            boxes: self.boxes.iter().map(scale).collect(),
            extent: scale(&self.extent),
            aka: self.aka.clone(),
            units: self.units,
            zoom,
        }
    }
}

/// Draws the names set along paths, where the tile `cr` draws shows them.
pub fn draw_paths(cr: &cairo::Context, pal: &Palette, vi: &ViewInfo, labels: &[Arc<PathLabel>]) {
    let started = crate::debug::timing().then(std::time::Instant::now);
    let scale = cr.target().device_scale().0;
    let (x0, y0, x1, y1) = cr.clip_extents().unwrap_or((f64::MIN, f64::MIN, f64::MAX, f64::MAX));
    let mut drawn = 0;
    for l in labels {
        let (ax, ay) = vi.world_to_screen(l.extent[0], l.extent[1]);
        let (bx, by) = (ax + l.extent[2] - l.extent[0], ay + l.extent[3] - l.extent[1]);
        let pad = l.height + 2.0 * NAME_PAD;
        if bx + pad < x0 || ax - pad > x1 || by + pad < y0 || ay - pad > y1 {
            continue;
        }
        for (text, g, adv) in &l.glyphs {
            if text.chars().all(char::is_whitespace) {
                continue;
            }
            let (sx, sy) = vi.world_to_screen(g.x, g.y);
            let reach = (adv.hypot(l.height) + 2.0 * NAME_PAD) / 2.0;
            if sx + reach < x0 || sx - reach > x1 || sy + reach < y0 || sy - reach > y1 {
                continue;
            }
            if let Some(img) = glyph(text, *adv, l.height, &l.style, g.angle.to_degrees().round() as i32, pal, scale) {
                blit_surface(cr, &img.surface, sx - img.w / 2.0, sy - img.h / 2.0, scale);
                drawn += 1;
            }
        }
    }
    if let Some(t) = started.filter(|t| t.elapsed() > std::time::Duration::from_millis(1)) {
        isomar_core::log!("isomar: draw_paths {drawn} glyphs of {} names {:.1} ms", labels.len(), t.elapsed().as_secs_f64() * 1000.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_tried_where_it_is_first_then_nudged_only_when_asked() {
        assert_eq!(nudges(100.0, 50.0, 40.0, 10.0, false), vec![(80.0, 45.0)]);
        let at = nudges(100.0, 50.0, 40.0, 10.0, true);
        assert_eq!(at[0], (80.0, 45.0));
        assert_eq!(at.len(), 9);
        assert!(at.windows(2).all(|p| (p[0].0 - 80.0).hypot(p[0].1 - 45.0) <= (p[1].0 - 80.0).hypot(p[1].1 - 45.0)), "nearest first");
        // A long name at a small scale is moved by pixels, not by its width: never to another sea.
        let far = nudges(500.0, 300.0, 400.0, 30.0, true);
        assert!(far.iter().all(|&(x, top)| (x + 200.0 - 500.0).abs() <= MAX_NUDGE_PX.0 && (top + 15.0 - 300.0).abs() <= MAX_NUDGE_PX.1), "{far:?}");
    }

    #[test]
    fn measure_splits_into_clusters_whose_advances_sum_to_the_width() {
        let st = Style::water(14.0);
        let m = measure("Balearic Sea", &st);
        assert_eq!(m.clusters.iter().map(|c| c.0.as_str()).collect::<String>(), "Balearic Sea");
        assert_eq!(m.clusters.len(), 12);
        assert!(m.clusters.iter().all(|c| c.1 > 0.0));
        assert_eq!(m.spaces().iter().filter(|s| **s).count(), 1);
        let total: f64 = m.advances().iter().sum();
        let whole = MEASURING.with(|cr| layout(cr, "Balearic Sea", &st).pixel_size().0 as f64);
        assert!((total - whole).abs() <= 1.5, "{total} against {whole}");
        assert!(m.height >= m.em, "a line is at least as tall as the font is big");
        // Accented letters are one cluster.
        let m = measure("Île d\u{2019}Yeu", &st);
        assert_eq!(m.clusters.len(), 9);
    }

    #[test]
    fn arabic_is_one_cluster() {
        let m = measure("البحر المتوسط", &Style::water(14.0));
        assert_eq!(m.clusters.len(), 1);
        assert_eq!(m.clusters[0].0, "البحر المتوسط");
        assert!(m.clusters[0].1 > 10.0);
    }

    #[test]
    fn names_along_paths_are_fitted_once_a_band_and_scaled_with_the_chart_through_it() {
        assert_eq!(label_zoom(12.0), 12.0);
        assert_eq!(label_zoom(11.99), 11.0);
        assert_eq!(label_zoom(12.5), 12.0, "a wheel notch past a step grows the names");
        assert_eq!(label_zoom(12.9999999), 13.0, "a hair under is on it");
        assert_eq!(mark_zoom(12.5), 12.0);
        // Once every mark is kept clear of, names are placed every two zooms.
        assert_eq!(label_zoom(14.99), 14.0);
        assert_eq!(label_zoom(16.99), 15.0);
        assert_eq!(label_zoom(17.0), 17.0);
        assert_eq!(mark_zoom(16.5), 16.0);
        let st = Style::water(12.0);
        let l = PathLabel { id: 1, style: st, glyphs: vec![("A".into(), GlyphAt { x: 100.0, y: 50.0, angle: 0.3 }, 8.0)], height: 14.0, boxes: vec![[96.0, 43.0, 104.0, 57.0]], extent: [96.0, 43.0, 104.0, 57.0], aka: vec![], units: [0.0; 4], zoom: 12.0 };
        let up = l.at_zoom(12.5);
        let k = 2f64.powf(0.5);
        assert!((up.glyphs[0].1.x - 100.0 * k).abs() < 1e-9 && (up.glyphs[0].1.y - 50.0 * k).abs() < 1e-9 && up.glyphs[0].1.angle == 0.3);
        assert!((up.style.size - 12.0 * k).abs() < 1e-9 && (up.height - 14.0 * k).abs() < 1e-9 && (up.glyphs[0].2 - 8.0 * k).abs() < 1e-9);
        assert!((up.boxes[0][2] - 104.0 * k).abs() < 1e-9 && up.zoom == 12.5);
        let back = up.at_zoom(12.0);
        assert!((back.glyphs[0].1.x - 100.0).abs() < 1e-9 && (back.style.size - 12.0).abs() < 1e-9);
    }

    #[test]
    fn samples_cover_the_box() {
        let s: Vec<_> = samples(0.0, 0.0, 30.0, 10.0).collect();
        assert!(s.contains(&(0.0, 0.0)) && s.contains(&(30.0, 10.0)) && s.contains(&(10.0, 5.0)));
    }
}
