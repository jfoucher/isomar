//! The weather overlay.

use crate::core::chart::grid::join_segments;
use crate::core::geo::LatLon;
use crate::core::weather::model::{Param, units};
use crate::app::AppState;
use super::Fill;
use crate::layers::overlay::{LabelRequest, Place, priority, stroke_haloed};
use crate::layers::{DrawCtx, Layer};
use crate::palette::Palette;
use crate::tokens::{self, Rgba, Token};
use crate::core::vector::Bounds;
use crate::layers::{TileArea, TilePaint};
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::Arc;

/// The forecast: colour field, isobars, barbs and wave arrows, drawn into tiles of their own plane over
/// the chart's (`layers::WEATHER_PLANE`), so that panning moves them like the chart and playing through
/// time draws only them again. It was drawn every frame, sampling the wind some 40 000 times and
/// tracing every isobar again, and took most of a core while panning.
pub struct WeatherLayer {
    app: Weak<AppState>,
    /// The last snapshot, with the revision and the area it was made for.
    last: RefCell<Option<(u64, Bounds, Arc<WeatherPaint>)>>,
}

impl WeatherLayer {
    pub fn new(app: &Rc<AppState>) -> WeatherLayer {
        WeatherLayer { app: Rc::downgrade(app), last: RefCell::new(None) }
    }
}

/// What follows the screen: the box being chosen for a download, and the time shown, top left.
pub struct WeatherFrame {
    app: Weak<AppState>,
}

impl WeatherFrame {
    pub fn new(app: &Rc<AppState>) -> WeatherFrame {
        WeatherFrame { app: Rc::downgrade(app) }
    }
}

/// The wind-speed ramp: speed in knots and the token of its colour.
const WIND_STOPS: [(f32, Token); 9] = [
    (0.0, Token::Wind00),
    (6.0, Token::Wind06),
    (12.0, Token::Wind12),
    (17.0, Token::Wind17),
    (22.0, Token::Wind22),
    (28.0, Token::Wind28),
    (34.0, Token::Wind34),
    (41.0, Token::Wind41),
    (48.0, Token::Wind48),
];

/// The wave-height ramp: height in metres and the token of its colour.
const WAVE_STOPS: [(f32, Token); 7] = [
    (0.0, Token::Wave0),
    (0.5, Token::Wave05),
    (1.0, Token::Wave1),
    (2.0, Token::Wave2),
    (3.0, Token::Wave3),
    (5.0, Token::Wave5),
    (8.0, Token::Wave8),
];

/// Colour for a wind speed in knots, in the palette's theme.
pub fn wind_colour(pal: &Palette, kn: f32) -> Rgba {
    ramp(pal, &WIND_STOPS, kn)
}

/// Colour for a significant wave height in metres.
pub fn wave_colour(pal: &Palette, m: f32) -> Rgba {
    ramp(pal, &WAVE_STOPS, m)
}

pub(crate) fn ramp(pal: &Palette, stops: &[(f32, Token)], v: f32) -> Rgba {
    if v <= stops[0].0 {
        return pal.color(stops[0].1);
    }
    for w in stops.windows(2) {
        let ((v0, c0), (v1, c1)) = (w[0], w[1]);
        if v <= v1 {
            let t = ((v - v0) / (v1 - v0)) as f64;
            let (a, b) = (pal.color(c0), pal.color(c1));
            let mix = |x: f64, y: f64| x + (y - x) * t;
            return (mix(a.0, b.0), mix(a.1, b.1), mix(a.2, b.2), mix(a.3, b.3));
        }
    }
    pal.color(stops[stops.len() - 1].1)
}

/// A forecast older than this is drawn hatched.
pub const STALE_AFTER_HOURS: i64 = 18;

/// Pennants (50 kn), full barbs (10 kn) and half barbs (5 kn) for a wind
/// speed, rounded to the nearest 5 knots.
pub fn barb_elements(kn: f32) -> (u32, u32, u32) {
    let five = (kn / 5.0).round() as u32;
    (five / 10, (five % 10) / 2, five % 2)
}

fn barb(cr: &cairo::Context, x: f64, y: f64, from_deg: f64, kn: f32) {
    let a = from_deg.to_radians();
    let (dx, dy) = (a.sin(), -a.cos());
    cr.new_path();
    if kn < 2.5 {
        cr.arc(x, y, 3.5, 0.0, std::f64::consts::TAU);
        return;
    }
    let len = 28.0;
    let (ex, ey) = (x + dx * len, y + dy * len);
    cr.move_to(x, y);
    cr.line_to(ex, ey);
    // Barbs lean away from the shaft, to the left of the direction it points.
    let (px, py) = (dy, -dx);
    let (flags, fulls, halves) = barb_elements(kn);
    let mut pos = 0.0;
    let at = |p: f64| (ex - dx * p, ey - dy * p);
    for _ in 0..flags {
        let (bx, by) = at(pos);
        let (cx, cy) = at(pos + 5.0);
        cr.move_to(bx, by);
        cr.line_to(bx + px * 10.0 - dx * 2.0, by + py * 10.0 - dy * 2.0);
        cr.line_to(cx, cy);
        pos += 6.0;
    }
    if flags > 0 {
        pos += 1.0;
    }
    for _ in 0..fulls {
        let (bx, by) = at(pos);
        cr.move_to(bx, by);
        cr.line_to(bx + px * 10.0 - dx * 3.0, by + py * 10.0 - dy * 3.0);
        pos += 4.0;
    }
    if halves > 0 {
        // A lone half barb sits one step in from the end so it is not a stub.
        if flags + fulls == 0 {
            pos += 4.0;
        }
        let (bx, by) = at(pos);
        cr.move_to(bx, by);
        cr.line_to(bx + px * 5.0 - dx * 1.5, by + py * 5.0 - dy * 1.5);
    }
}

fn arrow(cr: &cairo::Context, x: f64, y: f64, towards_deg: f64, len: f64) {
    let a = towards_deg.to_radians();
    let (dx, dy) = (a.sin(), -a.cos());
    let (ex, ey) = (x + dx * len, y + dy * len);
    cr.new_path();
    cr.move_to(x, y);
    cr.line_to(ex, ey);
    for s in [-1.0, 1.0] {
        let b = a + std::f64::consts::PI + s * 0.45;
        cr.move_to(ex, ey);
        cr.line_to(ex + b.sin() * 6.0, ey - b.cos() * 6.0);
    }
}

impl Layer for WeatherFrame {
    fn id(&self) -> &'static str {
        "weather"
    }

    fn is_dynamic(&self) -> bool {
        true
    }

    fn draw(&self, ctx: &DrawCtx) {
        let Some(app) = self.app.upgrade() else { return };
        let w = app.weather.borrow();
        let (cr, vi, pal) = (ctx.cr, &ctx.vi, &ctx.pal);
        if let Some((a, b)) = w.rect {
            let (p, q) = (vi.to_screen(&a), vi.to_screen(&b));
            cr.new_path();
            cr.rectangle(p.0.min(q.0), p.1.min(q.1), (p.0 - q.0).abs(), (p.1 - q.1).abs());
            pal.set_alpha(cr, Token::Accent, 0.12);
            let _ = cr.fill_preserve();
            cr.set_dash(&[8.0, 5.0], 0.0);
            stroke_haloed(cr, pal, Token::Accent, 2.0);
            cr.set_dash(&[], 0.0);
            cr.new_path();
        }
        if w.store.is_empty() {
            return;
        }
        // What time we are looking at, and how old the forecast is.
        let t = w.time;
        let local = t.with_timezone(&chrono::Local).format("%a %d %b %H:%M").to_string();
        let run = w.store.latest_run().map(|r| format!(" · {} run {}Z", w.source.label(), r.format("%d %H"))).unwrap_or_default();
        // Below what covers the top of a phone's screen (the status bar, the floating bar), not under it.
        let y = 40.0f64.max(ctx.top_inset + 24.0);
        ctx.overlay.request(LabelRequest::new(priority::WEATHER, format!("{local}{run}"), Place::At(vec![(12.0, y)]), Token::Text).strong());
    }
}

impl Layer for WeatherLayer {
    fn id(&self) -> &'static str {
        "weather"
    }

    fn plane(&self) -> u8 {
        crate::layers::WEATHER_PLANE
    }

    /// A fingerprint of what is shown: the time, the switches, the forecast's fields and whether it is stale.
    fn revision(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let Some(app) = self.app.upgrade() else { return 0 };
        let w = app.weather.borrow();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (w.time.timestamp_millis(), w.barbs, w.fill, w.pressure, w.wave_arrows, is_stale(&w.store)).hash(&mut h);
        for f in w.store.fields() {
            (f.param, f.time.timestamp(), f.run.map(|r| r.timestamp()), f.grid.ncols, f.grid.nrows, f.grid.xll.to_bits(), f.grid.yll.to_bits()).hash(&mut h);
        }
        h.finish()
    }

    fn prepare(&self, area: &TileArea) -> Option<Arc<dyn TilePaint>> {
        let app = self.app.upgrade()?;
        let w = app.weather.borrow();
        let drawn = !w.store.is_empty() && (w.barbs || w.fill != Fill::None || w.pressure || w.wave_arrows);
        if let Some(c) = app.credit.borrow().as_ref() {
            c.weather.set(drawn.then(|| w.source.credit()));
        }
        if !drawn {
            return None;
        }
        let rev = self.revision();
        let view = area.vi.bounds();
        if let Some((r, b, paint)) = self.last.borrow().as_ref()
            && *r == rev
            && b.contains([view.min_lon, view.min_lat])
            && b.contains([view.max_lon, view.max_lat])
        {
            return Some(paint.clone() as Arc<dyn TilePaint>);
        }
        // The forecast round the area, with room for the pressure centres' search, and the hours either
        // side of the time shown: a small copy for the workers.
        let b = view.expanded((view.width().max(view.height()) * 0.25).max(2.0));
        let hours = chrono::Duration::hours(12);
        let store = w.store.clipped(&b, w.time - hours, w.time + hours);
        let paint = Arc::new(WeatherPaint::new(store, &w));
        *self.last.borrow_mut() = Some((rev, b, paint.clone()));
        Some(paint)
    }
}

/// A forecast drawn from a forecast run older than `STALE_AFTER_HOURS` is hatched.
fn is_stale(store: &crate::core::weather::store::WeatherStore) -> bool {
    forecast_age(store).is_some_and(|a| a > chrono::Duration::hours(STALE_AFTER_HOURS))
}

/// What the weather's tiles are drawn from.
struct WeatherPaint {
    store: crate::core::weather::store::WeatherStore,
    time: chrono::DateTime<chrono::Utc>,
    stale: bool,
    barbs: bool,
    fill: Fill,
    wave_arrows: bool,
    /// Isobars, as longitude and latitude, traced once for every tile.
    isobars: Vec<Vec<[f64; 2]>>,
    /// Their values on the longest of each level, and the pressure centres: where, what, and whether strong.
    labels: Vec<(LatLon, String, bool)>,
}

/// Samples of the colour field, every this many points.
const SHADE_STEP: f64 = 8.0;
/// Wind barbs and wave arrows, every this many points (on the world's grid, so neighbouring tiles agree).
const BARB_STEP: f64 = 48.0;
const WAVE_STEP: f64 = 96.0;

impl WeatherPaint {
    fn new(store: crate::core::weather::store::WeatherStore, w: &super::WeatherState) -> WeatherPaint {
        let (mut isobars, mut labels) = (Vec::new(), Vec::new());
        if w.pressure {
            for g in store.grids_at(Param::Pressure, w.time) {
                isobars_of(&g, &mut isobars, &mut labels);
            }
        }
        WeatherPaint { stale: is_stale(&store), store, time: w.time, barbs: w.barbs, fill: w.fill, wave_arrows: w.wave_arrows, isobars, labels }
    }

    /// The points of the world grid of `step` points that fall within `area` (screen x0, y0, x1, y1) and
    /// `reach` round it, on screen.
    fn grid(vi: &crate::view::ViewInfo, step: f64, offset: f64, area: [f64; 4], reach: f64) -> Vec<(f64, f64)> {
        let (cx, cy) = vi.view.center_px();
        let (left, top) = (cx - vi.w / 2.0, cy - vi.h / 2.0);
        let mut out = Vec::new();
        let first = |o: f64, a: f64| ((o + a - reach - offset) / step).floor() as i64;
        let (i0, i1) = (first(left, area[0]), first(left, area[2] + 2.0 * reach) + 1);
        let (j0, j1) = (first(top, area[1]), first(top, area[3] + 2.0 * reach) + 1);
        for j in j0..=j1 {
            for i in i0..=i1 {
                out.push((i as f64 * step + offset - left, j as f64 * step + offset - top));
            }
        }
        out
    }
}

impl TilePaint for WeatherPaint {
    fn paint(&self, ctx: &DrawCtx) {
        let (cr, vi, pal) = (ctx.cr, &ctx.vi, &ctx.pal);
        let (x0, y0, x1, y1) = cr.clip_extents().unwrap_or((0.0, 0.0, vi.w, vi.h));
        let area = [x0, y0, x1, y1];
        let (w, t) = (&self.store, self.time);

        // One colour field at a time (`Fill`): two would mix into colours neither ramp has.
        match self.fill {
            Fill::Waves if w.has(Param::WaveHeight) => shade(ctx, area, self.stale, |lat, lon| w.sample(Param::WaveHeight, lat, lon, t).map(|m| wave_colour(pal, m))),
            Fill::Wind if w.has(Param::WindU) => shade(ctx, area, self.stale, |lat, lon| w.wind(lat, lon, t).map(|v| wind_colour(pal, units::ms_to_kn(v.speed_ms())))),
            _ => {}
        }

        // Isobars that reach into the tile.
        cr.set_line_join(cairo::LineJoin::Round);
        for line in &self.isobars {
            let pts: Vec<(f64, f64)> = line.iter().map(|p| vi.to_screen(&LatLon::new(p[1], p[0]))).collect();
            let (mut lo, mut hi) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
            for &(x, y) in &pts {
                lo = (lo.0.min(x), lo.1.min(y));
                hi = (hi.0.max(x), hi.1.max(y));
            }
            if hi.0 < x0 - 4.0 || lo.0 > x1 + 4.0 || hi.1 < y0 - 4.0 || lo.1 > y1 + 4.0 {
                continue;
            }
            cr.new_path();
            crate::layers::polyline(cr, vi, pts);
            stroke_haloed(cr, pal, Token::Isobar, 1.0);
            cr.new_path();
        }
        for (at, text, strong) in &self.labels {
            let (x, y) = vi.to_screen(at);
            if x < x0 - 80.0 || x > x1 + 80.0 || y < y0 - 20.0 || y > y1 + 20.0 {
                continue;
            }
            cr.select_font_face(crate::layers::FONT_FAMILY, cairo::FontSlant::Normal, if *strong { cairo::FontWeight::Bold } else { cairo::FontWeight::Normal });
            crate::layers::font_size(cr, 10.5);
            let Ok(e) = cr.text_extents(text) else { continue };
            crate::layers::halo_text(cr, pal, x - e.width() / 2.0 - e.x_bearing(), y - e.height() / 2.0 - e.y_bearing(), text, Token::Isobar);
        }

        if self.barbs && w.has(Param::WindU) {
            cr.set_line_width(1.5);
            cr.set_line_cap(cairo::LineCap::Round);
            for (x, y) in WeatherPaint::grid(vi, BARB_STEP, BARB_STEP / 2.0, area, 40.0) {
                let p = vi.position_at(x, y);
                if let Some(v) = w.wind(p.lat, p.lon, t) {
                    let kn = units::ms_to_kn(v.speed_ms());
                    barb(cr, x, y, v.blows_from() as f64, kn);
                    if self.fill == Fill::Wind {
                        stroke_haloed(cr, pal, Token::WindBarb, 1.5);
                    } else {
                        // No colour field under them: the barb wears the colour the field would have here.
                        stroke_haloed_in(cr, pal, wind_colour(pal, kn), 1.5);
                    }
                    cr.new_path();
                }
            }
        }
        if self.wave_arrows && w.has(Param::WaveHeight) && vi.zoom() >= 4.0 {
            cr.select_font_face(crate::layers::FONT_FAMILY, cairo::FontSlant::Normal, cairo::FontWeight::Normal);
            crate::layers::font_size(cr, 10.5);
            for (x, y) in WeatherPaint::grid(vi, WAVE_STEP, WAVE_STEP / 2.0 + 32.0, area, 60.0) {
                let p = vi.position_at(x, y);
                let hs = w.sample(Param::WaveHeight, p.lat, p.lon, t);
                let dir = w.sample(Param::WaveDir, p.lat, p.lon, t);
                if let (Some(hs), Some(dir)) = (hs, dir) {
                    arrow(cr, x, y, dir as f64 + 180.0, 10.0 + (hs as f64 * 5.0).min(24.0));
                    stroke_haloed(cr, pal, Token::WindBarb, 1.5);
                    cr.new_path();
                    crate::layers::halo_text(cr, pal, x + 6.0, y + 26.0, &format!("{hs:.1} m"), Token::WindBarb);
                }
            }
        }
    }
}

/// The points of the world grid of `step` points (offset by `offset`) within `area` and `reach` round
/// it, on screen: for the current arrows, on the same lattice rule as the barbs.
pub(crate) fn world_grid(vi: &crate::view::ViewInfo, step: f64, offset: f64, area: [f64; 4], reach: f64) -> Vec<(f64, f64)> {
    WeatherPaint::grid(vi, step, offset, area, reach)
}

/// `stroke_haloed` with a colour worked out rather than a token.
fn stroke_haloed_in(cr: &cairo::Context, pal: &Palette, (r, g, b, a): Rgba, width: f64) {
    cr.set_line_width(width + 2.0 * tokens::STROKE_HALO);
    pal.set_alpha(cr, Token::OverlayHalo, 0.75);
    let _ = cr.stroke_preserve();
    cr.set_line_width(width);
    cr.set_source_rgba(r, g, b, a);
    let _ = cr.stroke_preserve();
}

/// How old the newest forecast run is.
pub fn forecast_age(store: &crate::core::weather::store::WeatherStore) -> Option<chrono::Duration> {
    store.latest_run().map(|r| chrono::Utc::now() - r)
}

/// Paints a translucent colour field over `area` (screen x0, y0, x1, y1) by sampling every few points, on
/// the world's grid so that neighbouring tiles agree, and a sample past the area all round so that the
/// smoothing does not fade at its edge.
pub(crate) fn shade(ctx: &DrawCtx, area: [f64; 4], stale: bool, colour: impl Fn(f64, f64) -> Option<Rgba>) {
    const STEP: f64 = SHADE_STEP;
    let (cr, vi, pal) = (ctx.cr, &ctx.vi, &ctx.pal);
    let (cx, cy) = vi.view.center_px();
    let (left, top) = (cx - vi.w / 2.0, cy - vi.h / 2.0);
    let (i0, j0) = (((left + area[0]) / STEP).floor() - 1.0, ((top + area[1]) / STEP).floor() - 1.0);
    let (i1, j1) = (((left + area[2]) / STEP).ceil() + 1.0, ((top + area[3]) / STEP).ceil() + 1.0);
    let (sw, sh) = ((i1 - i0) as i32, (j1 - j0) as i32);
    let Ok(mut surface) = cairo::ImageSurface::create(cairo::Format::ARgb32, sw.max(1), sh.max(1)) else { return };
    let mut any = false;
    {
        let stride = surface.stride() as usize;
        let Ok(mut data) = surface.data() else { return };
        data.fill(0);
        for y in 0..sh {
            for x in 0..sw {
                let p = vi.position_at((i0 + x as f64 + 0.5) * STEP - left, (j0 + y as f64 + 0.5) * STEP - top);
                if let Some((r, g, b, _)) = colour(p.lat, p.lon) {
                    // Premultiplied, at 55% opacity.
                    let a = 140u32;
                    let px = (a << 24) | (((r * a as f64) as u32) << 16) | (((g * a as f64) as u32) << 8) | (b * a as f64) as u32;
                    let o = y as usize * stride + x as usize * 4;
                    data[o..o + 4].copy_from_slice(&px.to_ne_bytes());
                    any = true;
                }
            }
        }
    }
    if !any {
        return;
    }
    let _ = cr.save();
    cr.translate(i0 * STEP - left, j0 * STEP - top);
    cr.scale(STEP, STEP);
    if cr.set_source_surface(&surface, 0.0, 0.0).is_ok() {
        cr.source().set_filter(cairo::Filter::Bilinear);
        let _ = cr.paint();
        if stale {
            // Hatched with 45 degree lines in the stale colour, where the field is.
            hatch(cr, pal, &surface);
        }
    }
    let _ = cr.restore();
}

/// Draws 45 degree `stale` lines through the field's alpha. Called with the field's own scaling in place.
fn hatch(cr: &cairo::Context, pal: &Palette, field: &cairo::ImageSurface) {
    let Ok(tile) = cairo::ImageSurface::create(cairo::Format::ARgb32, 8, 8) else { return };
    if let Ok(t) = cairo::Context::new(&tile) {
        let (r, g, b, a) = pal.color(Token::Stale);
        t.set_source_rgba(r, g, b, a.max(0.9));
        t.set_line_width(1.6);
        t.move_to(-1.0, 9.0);
        t.line_to(9.0, -1.0);
        let _ = t.stroke();
    }
    let pattern = cairo::SurfacePattern::create(&tile);
    pattern.set_extend(cairo::Extend::Repeat);
    // The pattern is in device pixels, the mask in the field's scaled units.
    let _ = cr.save();
    let (sx, sy) = (cr.matrix().xx(), cr.matrix().yy());
    cr.identity_matrix();
    let _ = cr.set_source(&pattern);
    let _ = cr.restore();
    let _ = cr.save();
    let _ = (sx, sy);
    let _ = cr.mask_surface(field, 0.0, 0.0);
    let _ = cr.restore();
}

/// Isobars every 4 hPa, as lines of longitude and latitude, with a label on the longest line of each
/// level, near its middle, and the pressure centres.
fn isobars_of(g: &crate::core::chart::grid::Grid, lines_out: &mut Vec<Vec<[f64; 2]>>, labels: &mut Vec<(LatLon, String, bool)>) {
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for v in g.data.iter().filter(|v| v.is_finite()) {
        lo = lo.min(*v);
        hi = hi.max(*v);
    }
    if !lo.is_finite() {
        return;
    }
    let shift = if g.xll >= 180.0 { -360.0 } else { 0.0 };
    let first = (lo / 400.0).ceil() as i32;
    let last = (hi / 400.0).floor() as i32;
    for k in first..=last {
        let level = k as f32 * 400.0;
        let lines = join_segments(&g.contour(level));
        if let Some(l) = lines.iter().max_by_key(|l| l.len()).filter(|l| l.len() > 6) {
            let m = l[l.len() / 2];
            labels.push((LatLon::new(m[1] as f64, m[0] as f64 + shift), format!("{}", (level / 100.0).round() as i32), false));
        }
        lines_out.extend(lines.into_iter().map(|l| l.into_iter().map(|p| [p[0] as f64 + shift, p[1] as f64]).collect()));
    }
    pressure_centres(g, shift, labels);
}

/// The pressure centres: `H` on a local maximum and `L` on a local minimum of the grid, with their values.
fn pressure_centres(g: &crate::core::chart::grid::Grid, shift: f64, labels: &mut Vec<(LatLon, String, bool)>) {
    let (ncols, nrows) = (g.ncols as isize, g.nrows as isize);
    let at = |c: isize, r: isize| g.data.get((r * ncols + c) as usize).copied().filter(|v| v.is_finite());
    // A cell is a centre when it is the extreme of the cells within five steps.
    let radius = 5;
    for r in radius..nrows - radius {
        for c in radius..ncols - radius {
            let Some(v) = at(c, r) else { continue };
            let mut high = true;
            let mut low = true;
            'scan: for dr in -radius..=radius {
                for dc in -radius..=radius {
                    if (dr, dc) == (0, 0) {
                        continue;
                    }
                    match at(c + dc, r + dr) {
                        Some(o) => {
                            high &= o < v;
                            low &= o > v;
                        }
                        None => {
                            high = false;
                            low = false;
                        }
                    }
                    if !high && !low {
                        break 'scan;
                    }
                }
            }
            if !(high || low) {
                continue;
            }
            let (lon, lat) = (g.lon_at(c as f64), g.lat_at(r as f64));
            labels.push((LatLon::new(lat, lon + shift), format!("{} {}", if high { "H" } else { "L" }, (v / 100.0).round() as i32), true));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::Mode;

    fn day() -> Palette {
        Palette::new(Mode::Day, false)
    }

    #[test]
    fn wind_colours_follow_the_token_ramp() {
        let p = day();
        assert_eq!(wind_colour(&p, 0.0), p.color(Token::Wind00));
        assert_eq!(wind_colour(&p, 12.0), p.color(Token::Wind12));
        assert_eq!(wind_colour(&p, 90.0), p.color(Token::Wind48));
        // Halfway between two stops is halfway between their colours.
        let (a, b, m) = (p.color(Token::Wind12), p.color(Token::Wind17), wind_colour(&p, 14.5));
        assert!((m.0 - (a.0 + b.0) / 2.0).abs() < 1e-9);
        // Continuous across a stop.
        let (x, y) = (wind_colour(&p, 11.99), wind_colour(&p, 12.01));
        assert!((x.0 - y.0).abs() < 0.01);
    }

    #[test]
    fn the_ramp_changes_with_the_theme() {
        let night = Palette::new(Mode::Night, false);
        assert_ne!(wind_colour(&day(), 20.0), wind_colour(&night, 20.0));
    }

    #[test]
    fn wave_colours_clamp_at_the_ends() {
        let p = day();
        assert_eq!(wave_colour(&p, -1.0), wave_colour(&p, 0.0));
        assert_eq!(wave_colour(&p, 30.0), p.color(Token::Wave8));
    }

    #[test]
    fn barbs_count_pennants_full_and_half() {
        assert_eq!(barb_elements(0.0), (0, 0, 0));
        assert_eq!(barb_elements(5.0), (0, 0, 1));
        assert_eq!(barb_elements(10.0), (0, 1, 0));
        assert_eq!(barb_elements(15.0), (0, 1, 1));
        assert_eq!(barb_elements(25.0), (0, 2, 1));
        assert_eq!(barb_elements(50.0), (1, 0, 0));
        assert_eq!(barb_elements(65.0), (1, 1, 1));
        // Rounds to the nearest five.
        assert_eq!(barb_elements(12.4), (0, 1, 0));
        assert_eq!(barb_elements(13.0), (0, 1, 1));
    }
}
