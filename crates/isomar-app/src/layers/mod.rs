//! Things drawn on the chart, back to front.

pub mod aids;
pub mod anchorages;
pub mod bathy;
pub mod boat;
pub mod bsb;
pub mod capes;
pub mod chartinfo;
pub mod credit;
pub mod enc;
pub mod graticule;
pub mod hazard;
pub mod hover;
pub mod land;
pub mod names;
pub mod nav;
pub mod overlay;
pub mod regions;
pub mod relief;
pub mod router;
pub mod towns;
pub mod waters;

use super::palette::Palette;
use super::view::ViewInfo;
use crate::core::vector::Bounds;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

/// The plane of tiles over the chart's that the weather is drawn into (`Layer::plane`).
pub const WEATHER_PLANE: u8 = 1;
/// How long its tiles wait for a forecast that keeps changing (playing through time) before they are
/// drawn again: about ten frames a second.
pub const WEATHER_STALE_WAIT: std::time::Duration = std::time::Duration::from_millis(100);

/// Changes remembered by `Changes`; a tile drawn before the oldest is drawn again.
const CHANGES_KEPT: usize = 256;

/// Where a layer's data changed: an area, and, if only the tiles of some zooms draw it differently (a
/// coastline read at another level of detail), those zooms.
#[derive(Clone, Copy, Debug)]
pub struct Change {
    pub area: Bounds,
    pub zooms: Option<(f64, f64)>,
}

impl Change {
    pub fn within(area: Bounds) -> Change {
        Change { area, zooms: None }
    }

    /// Whether a tile showing `area` at `zoom` is drawn differently after it.
    pub fn touches(&self, area: &Bounds, zoom: f64) -> bool {
        self.area.intersects(area) && self.zooms.is_none_or(|(lo, hi)| (lo - 1e-9..=hi + 1e-9).contains(&zoom))
    }
}

/// A layer's revision, with where each change was, for `Layer::changes_since`: data arriving for one
/// corner of the view then draws only the tiles there again.
#[derive(Default)]
pub struct Changes {
    revision: Cell<u64>,
    /// (the revision a change made, where it was, or `None` for anywhere), oldest first.
    log: RefCell<VecDeque<(u64, Option<Change>)>>,
}

impl Changes {
    pub fn get(&self) -> u64 {
        self.revision.get()
    }

    /// A change within `area` (in degrees, west to east), or anywhere with `None`.
    pub fn bump(&self, area: Option<Bounds>) {
        self.bump_change(area.map(Change::within));
    }

    pub fn bump_change(&self, change: Option<Change>) {
        let r = self.revision.get() + 1;
        self.revision.set(r);
        let mut log = self.log.borrow_mut();
        if log.len() == CHANGES_KEPT {
            log.pop_front();
        }
        log.push_back((r, change));
    }

    pub fn since(&self, revision: u64) -> Option<Vec<Change>> {
        let log = self.log.borrow();
        let newer = log.iter().filter(|(r, _)| *r > revision);
        // Forgotten changes may have been anywhere.
        if log.front().is_none_or(|(r, _)| *r > revision + 1) && revision < self.revision.get() {
            return None;
        }
        newer.map(|(_, a)| *a).collect()
    }
}

pub struct DrawCtx<'a> {
    pub cr: &'a cairo::Context,
    pub vi: ViewInfo,
    pub pal: Palette,
    /// Labels and symbol areas of this frame's overlay layers, placed together after they draw.
    pub overlay: &'a overlay::Overlay,
    /// Motion (blinking, pulsing) is off.
    pub reduce_motion: bool,
    /// How much of the top of the screen the shell covers, in points (a phone's status bar and floating
    /// bar; `Engine::set_insets`): what a layer pins to the top of the screen goes below it. 0 in tiles.
    pub top_inset: f64,
}

/// The region the chart tiles about to be drawn belong to: the tiles in view and the ring round them.
pub struct TileArea {
    /// The view itself, at the zoom of the tiles.
    pub view: ViewInfo,
    /// The whole region as one view, at the zoom of the tiles. Layers decide here what depends on
    /// where the view is (which source, which scale), so that every tile of the region agrees.
    pub vi: ViewInfo,
    pub pal: Palette,
}

/// What a static layer draws a tile from: a snapshot of its data, taken on the main thread by
/// `Layer::prepare`, and painted on a worker thread.
pub trait TilePaint: Send + Sync {
    /// Draws the part of the layer that `ctx.vi` (a tile and a margin) shows.
    fn paint(&self, ctx: &DrawCtx);
}

pub trait Layer {
    /// Stable identifier, used for the layers menu and settings.
    fn id(&self) -> &'static str;

    /// Draws a dynamic layer, on the main thread, every frame.
    fn draw(&self, _ctx: &DrawCtx) {}

    /// For the layers that are not dynamic, on the main thread, before tiles are drawn: asks for
    /// the data the area lacks (cells, tiles, coast reads), sets the credits, and returns what to
    /// draw the tiles from. `None` means nothing to draw.
    fn prepare(&self, _area: &TileArea) -> Option<Arc<dyn TilePaint>> {
        None
    }

    /// True for layers that change on their own (the boat, routes being
    /// edited) or follow the screen rather than the map (the graticule's
    /// labels); they are drawn every frame. The others are drawn into tiles
    /// that are kept until the palette or the layer's `revision` changes.
    fn is_dynamic(&self) -> bool {
        false
    }

    /// Which plane of tiles a static layer is drawn into: the chart's (0), or one over it that is drawn
    /// and kept on its own (`WEATHER_PLANE`), so that a change there does not draw the chart again.
    fn plane(&self) -> u8 {
        0
    }

    /// Changes whenever the layer's data does.
    fn revision(&self) -> u64 {
        0
    }

    /// Where the changes since `revision` were, or `None` when they may have been anywhere (or the layer
    /// does not say). Tiles no change touches keep what they drew.
    fn changes_since(&self, _revision: u64) -> Option<Vec<Change>> {
        None
    }

    /// True while something it draws moves on its own (a blink): the engine then repaints at the blink rate.
    fn animating(&self) -> bool {
        false
    }

    /// True while it is still reading data the view asked for, which will change the tiles again when it
    /// arrives (chart cells read one by one): stale tiles then wait longer before they are drawn again.
    fn loading(&self) -> bool {
        false
    }
}

/// What data arriving for a downloaded tile can change: the tile, the names and symbols of its features
/// that run past it (up to most of a tile at the zoom it is drawn for), and `metres` round it.
pub fn reach_of(id: &crate::core::chart::vtile::TileId, metres: f64) -> Bounds {
    let b = id.bounds();
    let lat = b.min_lat.abs().max(b.max_lat.abs()).min(85.0);
    b.expanded(0.6 * b.width() + metres / 111_000.0 / lat.to_radians().cos())
}

/// The family of all chart text. Adwaita Sans ships with the Mac app and is installed with GNOME; a system
/// without it falls back to its default sans.
pub const FONT_FAMILY: &str = "Adwaita Sans";

/// User's chart text size, in percent (see `Settings::text_scale`).
static TEXT_SCALE: AtomicU32 = AtomicU32::new(100);

pub fn set_text_scale(percent: u32) {
    TEXT_SCALE.store(percent.clamp(50, 300), Ordering::Relaxed);
}

pub fn text_scale() -> u32 {
    TEXT_SCALE.load(Ordering::Relaxed)
}

/// Sets the font size for chart text, scaled by the text size preference.
/// Layers use this instead of `Context::set_font_size`.
pub fn font_size(cr: &cairo::Context, size: f64) {
    cr.set_font_size(size * text_scale() as f64 / 100.0);
}

/// Asks the canvas to repaint; layers call it when background work finishes.
pub type Redraw = Rc<dyn Fn()>;

/// Text with a pale outline so it stays legible over anything. Drawn once into a small image, kept, and
/// then copied: the outline and fill of the glyphs cost about a tenth of a millisecond a label, and the
/// graticule's labels are drawn every frame. Off a plain translation (scaled or rotated), drawn directly.
pub fn halo_text(cr: &cairo::Context, pal: &Palette, x: f64, y: f64, text: &str, colour: impl Into<super::palette::Paint>) {
    let colour = pal.rgba(colour);
    let (hr, hg, hb, ha) = pal.rgba(super::tokens::Token::OverlayHalo);
    let halo = (hr, hg, hb, ha * 0.85);
    let m = cr.matrix();
    let face = cr.font_face();
    let size = cr.font_matrix().yy();
    let (sx, sy) = cr.target().device_scale();
    let plain = m.xx() == 1.0 && m.yy() == 1.0 && m.xy() == 0.0 && m.yx() == 0.0 && sx == sy && face.type_() == cairo::FontType::FontTypeToy;
    if !plain {
        return halo_text_path(cr, x, y, text, colour, halo);
    }
    let key = LabelKey {
        text: text.to_string(),
        family: face.toy_get_family().map(|f| f.to_string()).unwrap_or_default(),
        weight: matches!(face.toy_get_weight(), cairo::FontWeight::Bold),
        slant: matches!(face.toy_get_slant(), cairo::FontSlant::Normal),
        bits: [size, sx, colour.0, colour.1, colour.2, colour.3, halo.0, halo.1, halo.2, halo.3].map(f64::to_bits),
    };
    LABELS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if !cache.contains_key(&key) {
            if cache.len() >= LABELS_KEPT {
                cache.clear();
            }
            let Some(label) = render_label(cr, text, colour, halo, sx) else { return halo_text_path(cr, x, y, text, colour, halo) };
            cache.insert(key.clone(), label);
        }
        let Some((surface, ox, oy)) = cache.get(&key) else { return };
        // On a whole device pixel, so it is copied, not resampled.
        let (dx, dy) = (((m.x0() + x + ox) * sx).round() / sx - m.x0(), ((m.y0() + y + oy) * sy).round() / sy - m.y0());
        let _ = cr.save();
        if cr.set_source_surface(surface, dx, dy).is_ok() {
            cr.source().set_filter(cairo::Filter::Nearest);
            cr.rectangle(dx, dy, surface.width() as f64 / sx, surface.height() as f64 / sy);
            let _ = cr.fill();
        }
        let _ = cr.restore();
        cr.new_path();
    });
}

/// The labels kept by `halo_text`, per thread; all are dropped when there are this many.
const LABELS_KEPT: usize = 2048;

#[derive(Clone, PartialEq, Eq, Hash)]
struct LabelKey {
    text: String,
    family: String,
    weight: bool,
    slant: bool,
    /// The size, the device scale, and the colours of the text and of its outline.
    bits: [u64; 10],
}

thread_local! {
    /// Each label as an image, with where its top-left corner is from the text's origin, in points.
    static LABELS: RefCell<std::collections::HashMap<LabelKey, (cairo::ImageSurface, f64, f64)>> = RefCell::new(std::collections::HashMap::new());
}

/// Width of the outline round halo text, in points.
const HALO_WIDTH: f64 = 3.0;

fn halo_text_path(cr: &cairo::Context, x: f64, y: f64, text: &str, colour: crate::tokens::Rgba, halo: crate::tokens::Rgba) {
    cr.new_path();
    cr.move_to(x, y);
    cr.text_path(text);
    cr.set_source_rgba(halo.0, halo.1, halo.2, halo.3);
    cr.set_line_width(HALO_WIDTH);
    cr.set_line_join(cairo::LineJoin::Round);
    let _ = cr.stroke_preserve();
    cr.set_source_rgba(colour.0, colour.1, colour.2, colour.3);
    let _ = cr.fill();
    cr.new_path();
}

/// `text` with its outline, in the font `cr` has, at device scale `scale`.
fn render_label(cr: &cairo::Context, text: &str, colour: crate::tokens::Rgba, halo: crate::tokens::Rgba, scale: f64) -> Option<(cairo::ImageSurface, f64, f64)> {
    let e = cr.text_extents(text).ok()?;
    let pad = HALO_WIDTH / 2.0 + 1.0;
    let (w, h) = (e.width() + 2.0 * pad, e.height() + 2.0 * pad);
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, (w * scale).ceil().max(1.0) as i32, (h * scale).ceil().max(1.0) as i32).ok()?;
    surface.set_device_scale(scale, scale);
    let c = cairo::Context::new(&surface).ok()?;
    c.set_font_face(&cr.font_face());
    c.set_font_matrix(cr.font_matrix());
    c.set_font_options(&cr.font_options().ok()?);
    let (ox, oy) = (e.x_bearing() - pad, e.y_bearing() - pad);
    halo_text_path(&c, -ox, -oy, text, colour, halo);
    drop(c);
    Some((surface, ox, oy))
}

/// Keeps labels from piling up: each accepted label reserves its rectangle.
#[derive(Default)]
pub struct LabelPlacer {
    rects: Vec<[f64; 4]>,
}

impl LabelPlacer {
    /// A placer with the boxes `[x0, y0, x1, y1]` already taken, by names a sounding must not cover.
    pub fn with(rects: impl IntoIterator<Item = [f64; 4]>) -> LabelPlacer {
        LabelPlacer { rects: rects.into_iter().collect() }
    }

    /// Reserves the box with its top-left at (`x`, `y`) unless it overlaps an
    /// earlier one. Returns whether the label should be drawn.
    pub fn try_place(&mut self, x: f64, y: f64, w: f64, h: f64) -> bool {
        let r = [x, y, x + w, y + h];
        if self.rects.iter().any(|o| r[0] < o[2] && r[2] > o[0] && r[1] < o[3] && r[3] > o[1]) {
            return false;
        }
        self.rects.push(r);
        true
    }

    /// Reserves all the boxes `[x0, y0, x1, y1]` unless any overlaps an earlier one: all or none.
    /// Returns whether they were reserved.
    pub fn try_place_all(&mut self, rects: &[[f64; 4]]) -> bool {
        let overlaps = |r: &[f64; 4], o: &[f64; 4]| r[0] < o[2] && r[2] > o[0] && r[1] < o[3] && r[3] > o[1];
        if rects.iter().any(|r| self.rects.iter().any(|o| overlaps(r, o))) {
            return false;
        }
        self.rects.extend_from_slice(rects);
        true
    }

    /// Takes the boxes whether or not they overlap: what a name already written covers.
    pub fn reserve(&mut self, rects: impl IntoIterator<Item = [f64; 4]>) {
        self.rects.extend(rects);
    }

    /// Draws `text` with its baseline-left at (`x`, `y`) if there is room.
    pub fn text(&mut self, cr: &cairo::Context, x: f64, y: f64, text: &str) {
        let Ok(e) = cr.text_extents(text) else { return };
        if self.try_place(x, y - e.height(), e.width() + 2.0, e.height() + 2.0) {
            cr.new_path();
            cr.move_to(x, y);
            let _ = cr.show_text(text);
            cr.new_path();
        }
    }
}

/// How far past the edges of the view lines are drawn before they are cut.
const CLIP_MARGIN: f64 = 1000.0;

/// The part of the screen segment `a`–`b` within the view and a margin, and
/// how far along the segment it starts; `None` when none of it is. Cairo
/// keeps coordinates in 24.8 fixed point (about 8 million pixels) and clamps
/// beyond: at zoom 18 the far end of an ocean leg is further away than that,
/// and the leg came out at the wrong angle.
pub fn clip_segment(vi: &ViewInfo, a: (f64, f64), b: (f64, f64)) -> Option<((f64, f64), (f64, f64), f64)> {
    clip_segment_to(vi, a, b, CLIP_MARGIN)
}

/// `clip_segment` with a margin of `margin` pixels round the view (negative
/// keeps that far inside it).
pub fn clip_segment_to(vi: &ViewInfo, a: (f64, f64), b: (f64, f64), margin: f64) -> Option<((f64, f64), (f64, f64), f64)> {
    // Liang-Barsky.
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, q) in [(-dx, a.0 + margin), (dx, vi.w + margin - a.0), (-dy, a.1 + margin), (dy, vi.h + margin - a.1)] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else if p < 0.0 {
            t0 = t0.max(q / p);
        } else {
            t1 = t1.min(q / p);
        }
    }
    if t0 > t1 {
        return None;
    }
    let at = |t: f64| (a.0 + dx * t, a.1 + dy * t);
    Some((at(t0), at(t1), t0 * dx.hypot(dy)))
}

/// Adds the line through the screen points `pts` to the path, cut to the
/// view (see `clip_segment`).
pub fn polyline(cr: &cairo::Context, vi: &ViewInfo, pts: impl IntoIterator<Item = (f64, f64)>) {
    let (mut prev, mut pen) = (None, None);
    for p in pts {
        if let Some((s, e, _)) = prev.and_then(|a| clip_segment(vi, a, p)) {
            if pen != Some(s) {
                cr.move_to(s.0, s.1);
            }
            cr.line_to(e.0, e.1);
            pen = Some(e);
        }
        prev = Some(p);
    }
}

/// Adds the segment `a`–`b` to the path, cut to the view, and returns how
/// far along it the part drawn starts: the dash offset that keeps dashes
/// still on the chart.
pub fn segment(cr: &cairo::Context, vi: &ViewInfo, a: (f64, f64), b: (f64, f64)) -> f64 {
    match clip_segment(vi, a, b) {
        Some((s, e, from)) => {
            cr.move_to(s.0, s.1);
            cr.line_to(e.0, e.1);
            from
        }
        None => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> ViewInfo {
        ViewInfo { view: crate::view::View { center: crate::core::geo::LatLon::new(47.6637, -2.9781), zoom: 18.0 }, w: 800.0, h: 600.0 }
    }

    #[test]
    fn a_segment_reaching_far_off_screen_is_cut_at_its_own_angle() {
        let vi = view();
        // An ocean leg at zoom 18: one end on screen, the other 20 million pixels away.
        let (a, b) = ((400.0, 300.0), (-2.0e7, 1.0e7));
        let (s, e, from) = clip_segment(&vi, a, b).unwrap();
        assert_eq!(s, a);
        assert_eq!(from, 0.0);
        assert!((e.0 + CLIP_MARGIN).abs() < 1e-6, "{e:?}");
        let slope = |p: (f64, f64), q: (f64, f64)| (q.1 - p.1) / (q.0 - p.0);
        assert!((slope(s, e) - slope(a, b)).abs() < 1e-9);
        // Seen from the far end, the part drawn starts where it enters.
        let (_, _, from) = clip_segment(&vi, b, a).unwrap();
        assert!((from - (b.0 - e.0).hypot(b.1 - e.1)).abs() < 1e-3);
        assert_eq!(clip_segment(&vi, (-5000.0, -5000.0), (-5000.0, 5000.0)), None);
    }

    #[test]
    fn a_transatlantic_leg_at_zoom_18_is_beyond_what_cairo_can_hold() {
        use crate::core::geo::LatLon;
        let vi = view();
        let (a, b) = (vi.to_screen(&LatLon::new(47.6637, -2.9781)), vi.to_screen(&LatLon::new(16.194, -61.546)));
        assert!(b.0.abs() > 8.4e6 || b.1.abs() > 8.4e6, "{b:?}");
        let (s, e, _) = clip_segment(&vi, a, b).unwrap();
        assert!(s.0.abs().max(s.1.abs()).max(e.0.abs()).max(e.1.abs()) <= 800.0 + CLIP_MARGIN);
        assert!(((e.1 - s.1).atan2(e.0 - s.0) - (b.1 - a.1).atan2(b.0 - a.0)).abs() < 1e-9);
    }

    #[test]
    fn try_place_all_is_all_or_nothing() {
        let mut p = LabelPlacer::default();
        assert!(p.try_place(0.0, 0.0, 50.0, 10.0));
        assert!(!p.try_place_all(&[[60.0, 0.0, 80.0, 10.0], [40.0, 0.0, 70.0, 10.0]]), "the second overlaps the first");
        assert!(p.try_place(60.0, 0.0, 20.0, 10.0), "the first of them was not kept");
        assert!(p.try_place_all(&[[0.0, 20.0, 10.0, 30.0], [10.0, 20.0, 20.0, 30.0]]), "touching edges are fine");
        assert!(!p.try_place(5.0, 22.0, 3.0, 3.0), "and both are kept");
        p.reserve([[100.0, 100.0, 200.0, 200.0]]);
        assert!(!p.try_place(150.0, 150.0, 5.0, 5.0));
    }

    #[test]
    fn overlapping_labels_are_refused() {
        let mut p = LabelPlacer::default();
        assert!(p.try_place(0.0, 0.0, 50.0, 10.0));
        assert!(!p.try_place(40.0, 5.0, 50.0, 10.0));
        assert!(p.try_place(60.0, 0.0, 50.0, 10.0), "touching edges is fine");
        assert!(p.try_place(0.0, 20.0, 50.0, 10.0));
    }
}
