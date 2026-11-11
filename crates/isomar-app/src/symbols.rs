//! Chart symbols drawn with Cairo to INT 1 (`design/design-system/chart-symbols.md`): buoys that
//! lean 15 degrees with their bands, stripes and topmarks, upright beacons, light flares, and the
//! arcs of sector lights. Drawn from the specification, never copied from a published chart.

use super::palette::{Paint, Palette, Rgb, rgb};
use super::tokens::{self, MM_PX, Token};
use crate::core::chart::aids::{Aid, AidKind};
use cairo::Context;
use std::f64::consts::PI;
use std::sync::atomic::{AtomicBool, Ordering};

static ECDIS: AtomicBool = AtomicBool::new(false);
static FULL_SECTORS: AtomicBool = AtomicBool::new(false);

/// The height of buoys and beacons and the width of landmarks, in pixels at 100% text size,
/// whatever the zoom: far out, smaller marks are hidden rather than drawn smaller.
pub const MARK_PX: f64 = 19.6;

/// The width of a lighthouse's star at 100% text size: the marks were made 40% bigger, the star
/// was already the right size.
pub const LIGHTHOUSE_PX: f64 = 14.0;

/// How much bigger marks and flares are drawn: the text size preference, so that they grow with
/// the chart text.
pub fn mark_scale() -> f64 {
    crate::layers::text_scale() as f64 / 100.0
}

/// Sector arcs at the lights' nominal range, not a fixed size on screen.
pub fn set_full_sectors(on: bool) {
    FULL_SECTORS.store(on, Ordering::Relaxed);
}

pub fn full_sectors() -> bool {
    FULL_SECTORS.load(Ordering::Relaxed)
}

/// Which colours marks and lights are drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// The INT 1 paper colours of the design tokens.
    Paper,
    /// The S-52 style colours the app used before the paper style.
    Ecdis,
}

pub fn set_style(style: Style) {
    ECDIS.store(style == Style::Ecdis, Ordering::Relaxed);
}

pub fn style() -> Style {
    if ECDIS.load(Ordering::Relaxed) { Style::Ecdis } else { Style::Paper }
}

impl std::str::FromStr for Style {
    type Err = ();
    fn from_str(s: &str) -> Result<Style, ()> {
        match s {
            "paper" => Ok(Style::Paper),
            "ecdis" => Ok(Style::Ecdis),
            _ => Err(()),
        }
    }
}

/// S-57 COLOUR code to a display colour.
pub fn s57_colour(code: u8) -> Paint {
    let legacy = |c: Rgb| Paint::Legacy(c);
    if style() == Style::Ecdis {
        return legacy(match code {
            1 => rgb(250, 250, 250),
            2 => rgb(30, 30, 30),
            3 => rgb(214, 40, 40),
            4 => rgb(30, 160, 70),
            5 => rgb(40, 90, 200),
            6 => rgb(245, 205, 20),
            7 => rgb(140, 140, 140),
            8 => rgb(120, 80, 40),
            9 => rgb(245, 160, 20),
            10 => rgb(140, 70, 170),
            11 => rgb(245, 130, 30),
            12 => rgb(200, 40, 140),
            13 => rgb(250, 160, 180),
            _ => rgb(120, 120, 120),
        });
    }
    match code {
        1 => Token::MarkWhite.into(),
        2 => Token::MarkBlack.into(),
        3 => Token::MarkRed.into(),
        4 => Token::MarkGreen.into(),
        5 => Token::MarkBlue.into(),
        6 => Token::MarkYellow.into(),
        7 => legacy(rgb(140, 140, 140)),
        8 => legacy(rgb(120, 80, 40)),
        9 => legacy(rgb(245, 160, 20)),
        10 => legacy(rgb(140, 70, 170)),
        11 => legacy(rgb(245, 130, 30)),
        12 => Token::ChartMagenta.into(),
        13 => legacy(rgb(250, 160, 180)),
        _ => legacy(rgb(120, 120, 120)),
    }
}

/// The colour of a light for an S-57 colour code.
pub fn light_colour_of(code: u8) -> Paint {
    if style() == Style::Ecdis {
        return Paint::Legacy(match code {
            1 | 6 | 11 => rgb(245, 215, 60),
            3 => rgb(220, 40, 40),
            4 => rgb(30, 170, 70),
            5 => rgb(60, 110, 220),
            _ => rgb(190, 40, 150),
        });
    }
    match code {
        1 | 6 | 11 => Token::LightWhite,
        3 => Token::LightRed,
        4 => Token::LightGreen,
        _ => Token::LightOther,
    }
    .into()
}

/// Colour of a light's arc or flare: the first of its colours decides.
pub fn light_colour(codes: &[u8]) -> Paint {
    light_colour_of(codes.first().copied().unwrap_or(0))
}

/// What a mark is, which fixes its colours' layout and its topmark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Port,
    Starboard,
    /// 1 north, 2 east, 3 south, 4 west.
    Cardinal(u8),
    IsolatedDanger,
    SafeWater,
    Special,
    /// Emergency wreck marking: blue and yellow stripes, a yellow upright cross.
    Wreck,
    Plain,
}

impl Mark {
    /// From CATLAM (1 port hand, 2 starboard hand).
    pub fn lateral(category: Option<i32>) -> Mark {
        match category {
            Some(1) => Mark::Port,
            Some(2) => Mark::Starboard,
            _ => Mark::Plain,
        }
    }

    /// From CATCAM (1 to 4: north, east, south, west).
    pub fn cardinal(category: Option<i32>) -> Mark {
        match category {
            Some(c @ 1..=4) => Mark::Cardinal(c as u8),
            _ => Mark::Plain,
        }
    }

    /// Colours in vertical stripes, not horizontal bands.
    fn striped(self) -> bool {
        matches!(self, Mark::SafeWater | Mark::Wreck)
    }
}

/// How much of a buoy's height a topmark adds.
const TOPMARK: f64 = 0.42;
/// A buoy leans this far, to the right.
const LEAN_DEG: f64 = 15.0;

fn ink(cr: &Context, pal: &Palette) {
    pal.set(cr, Token::ChartInk);
    cr.set_line_width(0.8);
    cr.set_line_join(cairo::LineJoin::Round);
}

/// Fills the current path with the mark's colours, as horizontal bands (listed top to bottom) or
/// vertical stripes, then outlines it in `chart-ink`. The box is the symbol's extent.
fn paint_colours(cr: &Context, pal: &Palette, colours: &[u8], striped: bool, bbox: (f64, f64, f64, f64)) {
    // The bands are filled on a clean path: added to the symbol's own, they would be a hole in it under
    // the even-odd rule the ENC layer draws with, and the fill would leave nothing to outline.
    let path = cr.copy_path().ok();
    let _ = cr.save();
    cr.clip();
    let cols: Vec<Paint> = if colours.is_empty() { vec![Paint::Legacy(rgb(200, 200, 200))] } else { colours.iter().map(|c| s57_colour(*c)).collect() };
    let (x0, y0, x1, y1) = bbox;
    let n = cols.len() as f64;
    for (i, c) in cols.iter().enumerate() {
        pal.set(cr, *c);
        if striped {
            let w = (x1 - x0) / n;
            cr.rectangle(x0 + w * i as f64, y0 - 1.0, w + 0.5, y1 - y0 + 2.0);
        } else {
            let h = (y1 - y0) / n;
            cr.rectangle(x0 - 1.0, y0 + h * i as f64, x1 - x0 + 2.0, h + 0.5);
        }
        let _ = cr.fill();
    }
    let _ = cr.restore();
    if let Some(path) = &path {
        cr.append_path(path);
    }
    ink(cr, pal);
    let _ = cr.stroke();
}

fn fill_ink(cr: &Context, pal: &Palette, fill: impl Into<Paint>) {
    pal.set(cr, fill);
    let _ = cr.fill_preserve();
    ink(cr, pal);
    let _ = cr.stroke();
}

/// A cone of base width `w` and height `h`, base on `y`, pointing up or down.
fn cone(cr: &Context, pal: &Palette, cx: f64, y: f64, w: f64, h: f64, up: bool, fill: Token) {
    cr.new_path();
    if up {
        cr.move_to(cx, y - h);
        cr.line_to(cx + w / 2.0, y);
        cr.line_to(cx - w / 2.0, y);
    } else {
        cr.move_to(cx, y);
        cr.line_to(cx + w / 2.0, y - h);
        cr.line_to(cx - w / 2.0, y - h);
    }
    cr.close_path();
    fill_ink(cr, pal, fill);
}

fn sphere(cr: &Context, pal: &Palette, cx: f64, cy: f64, r: f64, fill: Token) {
    cr.new_path();
    cr.arc(cx, cy, r, 0.0, 2.0 * PI);
    fill_ink(cr, pal, fill);
}

/// The topmark standing on the point (`0`, `y`), `h` tall at most.
fn topmark(cr: &Context, pal: &Palette, mark: Mark, y: f64, h: f64) {
    let e = h / 2.0;
    let w = h * 0.8;
    // A sliver of space, wider than the 0.8 px outline, which eats into it.
    let gap = (h * 0.2).max(1.6);
    match mark {
        Mark::Port => {
            cr.new_path();
            cr.rectangle(-h * 0.3, y - h, h * 0.6, h);
            fill_ink(cr, pal, Token::MarkRed);
        }
        Mark::Starboard => cone(cr, pal, 0.0, y, w, h, true, Token::MarkGreen),
        // Both cones raised a sliver, so the lower one's base does not merge into the mark's top.
        Mark::Cardinal(1) => {
            cone(cr, pal, 0.0, y - gap, w, e, true, Token::MarkBlack);
            cone(cr, pal, 0.0, y - e - gap, w, e, true, Token::MarkBlack);
        }
        // Base to base: the upper cone points up, the lower one down, the upper raised a sliver so
        // the pair does not read as a lozenge.
        Mark::Cardinal(2) => {
            cone(cr, pal, 0.0, y, w, e, false, Token::MarkBlack);
            cone(cr, pal, 0.0, y - e - gap, w, e, true, Token::MarkBlack);
        }
        Mark::Cardinal(3) => {
            cone(cr, pal, 0.0, y, w, e, false, Token::MarkBlack);
            cone(cr, pal, 0.0, y - e, w, e, false, Token::MarkBlack);
        }
        // Point to point: the upper cone points down, the lower one up.
        Mark::Cardinal(_) => {
            cone(cr, pal, 0.0, y, w, e, true, Token::MarkBlack);
            cone(cr, pal, 0.0, y - e, w, e, false, Token::MarkBlack);
        }
        Mark::IsolatedDanger => {
            sphere(cr, pal, 0.0, y - e * 0.5, e * 0.5, Token::MarkBlack);
            sphere(cr, pal, 0.0, y - e * 1.5, e * 0.5, Token::MarkBlack);
        }
        Mark::SafeWater => sphere(cr, pal, 0.0, y - h * 0.4, h * 0.4, Token::MarkRed),
        Mark::Special | Mark::Wreck => {
            let _ = cr.save();
            cr.new_path();
            cr.translate(0.0, y - h / 2.0);
            if mark == Mark::Special {
                cr.rotate(PI / 4.0);
            }
            let r = h * 0.5;
            cr.move_to(-r, 0.0);
            cr.line_to(r, 0.0);
            cr.move_to(0.0, -r);
            cr.line_to(0.0, r);
            // The cross is yellow with an ink edge: ink first, then the colour on top.
            ink(cr, pal);
            cr.set_line_width(2.6);
            let _ = cr.stroke_preserve();
            pal.set(cr, Token::MarkYellow);
            cr.set_line_width(1.4);
            let _ = cr.stroke();
            let _ = cr.restore();
        }
        Mark::Plain => {}
    }
}

/// The small open circle that marks where an object is.
fn position_ring(cr: &Context, pal: &Palette) {
    cr.new_path();
    cr.arc(0.0, 0.0, 1.8, 0.0, 2.0 * PI);
    ink(cr, pal);
    let _ = cr.stroke();
}

/// What kind of mark an aid is, for its colours' layout and its topmark.
pub fn mark_of(a: &Aid) -> Mark {
    match a.kind {
        AidKind::BuoyLateral | AidKind::BeaconLateral => Mark::lateral(a.category),
        AidKind::BuoyCardinal | AidKind::BeaconCardinal => Mark::cardinal(a.category),
        AidKind::BuoyIsolatedDanger | AidKind::BeaconIsolatedDanger => Mark::IsolatedDanger,
        AidKind::BuoySafeWater | AidKind::BeaconSafeWater => Mark::SafeWater,
        AidKind::BuoySpecial | AidKind::BeaconSpecial => {
            // Emergency wreck marking is a special mark whose colours are blue and yellow.
            if a.colours.contains(&5) && a.colours.contains(&6) { Mark::Wreck } else { Mark::Special }
        }
        _ => Mark::Plain,
    }
}

/// An aid to navigation, `s` pixels tall, with its foot on (x, y).
pub fn aid(cr: &Context, pal: &Palette, x: f64, y: f64, s: f64, a: &Aid) {
    let mark = mark_of(a);
    match a.kind {
        k if k.is_buoy() => buoy(cr, pal, x, y, s, a.shape, &a.colours, mark),
        k if k.is_beacon() => beacon(cr, pal, x, y, s, a.shape, &a.colours, mark),
        _ => landmark(cr, pal, x, y, s),
    }
}

/// A buoy of BOYSHP `shape`, `s` pixels tall, leaning 15 degrees, with its foot on (x, y).
pub fn buoy(cr: &Context, pal: &Palette, x: f64, y: f64, s: f64, shape: Option<i32>, colours: &[u8], mark: Mark) {
    let s = s.min(tokens::BUOY_HEIGHT * MM_PX) * mark_scale();
    // A previous label leaves a current point that arcs would connect to.
    cr.new_path();
    let _ = cr.save();
    cr.translate(x, y);
    cr.rotate(LEAN_DEG.to_radians());
    // The shape falls back to what the kind of mark is: cans for port, cones for starboard.
    let shape = shape.or(match mark {
        Mark::Port => Some(2),
        Mark::Starboard => Some(1),
        Mark::SafeWater => Some(3),
        _ => None,
    });
    let (body, w) = match shape {
        // Conical: point up.
        Some(1) => {
            cr.move_to(0.0, -s);
            cr.line_to(s * 0.3, 0.0);
            cr.line_to(-s * 0.3, 0.0);
            cr.close_path();
            (s, s * 0.6)
        }
        // Can: a cylinder.
        Some(2) => {
            cr.rectangle(-s * 0.28, -s * 0.8, s * 0.56, s * 0.8);
            (s * 0.8, s * 0.56)
        }
        // Spherical, barrel.
        Some(3) | Some(6) => {
            cr.arc(0.0, -s * 0.3, s * 0.3, 0.0, 2.0 * PI);
            (s * 0.6, s * 0.6)
        }
        // Spar: a thin stick.
        Some(5) => {
            cr.rectangle(-s * 0.06, -s, s * 0.12, s);
            (s, s * 0.12)
        }
        // Pillar, super-buoy and anything else: a tall body that narrows to the foot.
        _ => {
            cr.move_to(-s * 0.16, -s);
            cr.line_to(s * 0.16, -s);
            cr.line_to(s * 0.2, -s * 0.1);
            cr.line_to(-s * 0.2, -s * 0.1);
            cr.close_path();
            (s, s * 0.4)
        }
    };
    paint_colours(cr, pal, colours, mark.striped(), (-w / 2.0, -body, w / 2.0, 0.0));
    topmark(cr, pal, mark, -body, s * TOPMARK);
    position_ring(cr, pal);
    let _ = cr.restore();
}

/// A fixed beacon of BCNSHP `shape`, `s` pixels tall, upright, with its foot on (x, y).
pub fn beacon(cr: &Context, pal: &Palette, x: f64, y: f64, s: f64, shape: Option<i32>, colours: &[u8], mark: Mark) {
    let s = s.min(tokens::BUOY_HEIGHT * MM_PX) * mark_scale();
    cr.new_path();
    let _ = cr.save();
    cr.translate(x, y);
    let (body, w) = match shape {
        // Stake, pole, perch, withy, pile: a single stroke.
        Some(1) | Some(2) | Some(5) => {
            cr.rectangle(-s * 0.05, -s, s * 0.1, s);
            (s, s * 0.1)
        }
        // Cairn: a low heap.
        Some(6) => {
            cr.move_to(-s * 0.3, 0.0);
            cr.line_to(-s * 0.12, -s * 0.7);
            cr.line_to(s * 0.12, -s * 0.7);
            cr.line_to(s * 0.3, 0.0);
            cr.close_path();
            (s * 0.7, s * 0.6)
        }
        // Tower, lattice and anything else: a narrow trapezoid.
        _ => {
            cr.move_to(-s * 0.18, 0.0);
            cr.line_to(s * 0.18, 0.0);
            cr.line_to(s * 0.1, -s);
            cr.line_to(-s * 0.1, -s);
            cr.close_path();
            (s, s * 0.36)
        }
    };
    paint_colours(cr, pal, colours, mark.striped(), (-w / 2.0, -body, w / 2.0, 0.0));
    topmark(cr, pal, mark, -body, s * TOPMARK);
    position_ring(cr, pal);
    let _ = cr.restore();
}

/// Landmark: a small circle with a centre dot.
pub fn landmark(cr: &Context, pal: &Palette, x: f64, y: f64, s: f64) {
    let s = s * mark_scale();
    // A previous label leaves a current point that arcs would connect to.
    cr.new_path();
    pal.set(cr, Token::ChartInk);
    cr.set_line_width(1.0);
    cr.arc(x, y, s * 0.35, 0.0, 2.0 * PI);
    let _ = cr.stroke();
    cr.arc(x, y, 1.0, 0.0, 2.0 * PI);
    let _ = cr.fill();
}

/// A lighthouse (a landmark with a light on it): a five-pointed star in ink, centred on (x, y),
/// that the light's flare leaves from.
pub fn lighthouse(cr: &Context, pal: &Palette, x: f64, y: f64, s: f64) {
    let outer = s * mark_scale() * 0.5;
    let inner = outer * 0.42;
    cr.new_path();
    for k in 0..10 {
        let r = if k % 2 == 0 { outer } else { inner };
        let a = (k as f64 * 36.0 - 90.0).to_radians();
        cr.line_to(x + r * a.cos(), y + r * a.sin());
    }
    cr.close_path();
    pal.set(cr, Token::ChartInk);
    let _ = cr.fill();
}

/// The flare of a light (the "tear"): a teardrop with its point on (x, y), `len` long, pointing along
/// the compass bearing `bearing`, filled with the light's colour and edged in ink.
pub fn flare(cr: &Context, pal: &Palette, x: f64, y: f64, len: f64, bearing: f64, fill: impl Into<Paint>) {
    cr.new_path();
    let _ = cr.save();
    cr.translate(x, y);
    // The drop is drawn pointing along +y; this turns +y to the bearing.
    cr.rotate((bearing + 180.0).to_radians());
    let r = len * 0.21;
    let d = len - r;
    let phi = (r / d).asin();
    cr.move_to(0.0, 0.0);
    cr.line_to(r * phi.cos(), d - r * phi.sin());
    cr.arc(0.0, d, r, -phi, PI + phi);
    cr.close_path();
    pal.set(cr, fill);
    let _ = cr.fill_preserve();
    pal.set(cr, Token::ChartInk);
    cr.set_line_width(0.5);
    cr.set_line_join(cairo::LineJoin::Round);
    let _ = cr.stroke();
    let _ = cr.restore();
}

/// How long a light's flare is, the same at every zoom: the token's length up to 5 nm of nominal
/// range (or when it is unknown), growing to 1.5 times it at 25 nm, so the lights that reach far
/// stand out whether or not they have sectors.
pub fn flare_length(range_nm: Option<f64>) -> f64 {
    let reach = range_nm.map_or(0.0, |nm| ((nm - 5.0) / 20.0).clamp(0.0, 1.0));
    tokens::FLARE_LENGTH * MM_PX * mark_scale() * (1.0 + 0.5 * reach)
}

/// The dot in ink on a light's exact position, alone: for a sector light, which the arcs show.
pub fn light_dot(cr: &Context, pal: &Palette, x: f64, y: f64) {
    cr.new_path();
    cr.arc(x, y, (tokens::FLARE_LENGTH * MM_PX * 0.08).max(1.6) * mark_scale(), 0.0, 2.0 * PI);
    pal.set(cr, Token::ChartInk);
    let _ = cr.fill();
}

/// An all-round or fixed light as the paper chart has it: a dot in ink at the position and one
/// flare per colour, fanned 15 degrees apart clockwise from south-east. `fan` is how many flares
/// other lights on the same structure already have, so that theirs do not overlap.
pub fn light(cr: &Context, pal: &Palette, x: f64, y: f64, colours: &[u8], fan: usize, range_nm: Option<f64>) {
    let len = flare_length(range_nm);
    let colours: &[u8] = if colours.is_empty() { &[0] } else { colours };
    for (k, c) in colours.iter().enumerate() {
        flare(cr, pal, x, y, len, 135.0 + 15.0 * (fan + k) as f64, light_colour_of(*c));
    }
    cr.new_path();
    cr.arc(x, y, (tokens::FLARE_LENGTH * MM_PX * 0.08).max(1.6) * mark_scale(), 0.0, 2.0 * PI);
    pal.set(cr, Token::ChartInk);
    let _ = cr.fill();
}

/// The compass-bearing angle on the screen of a bearing seen from seaward, as cairo's arcs want it.
fn arc_angle(bearing: f64) -> f64 {
    (bearing + 180.0).to_radians() - PI / 2.0
}

/// The nominal range drawn at the token's sector radius; other ranges scale from it.
const SECTOR_REFERENCE_NM: f64 = 10.0;

/// The radius of a light's sector arcs on screen: the token's size scaled by the light's nominal
/// range (10 nm at the token size, within 0.4 to 1.5 times it; the token size when the range is
/// unknown), or the nominal range itself with "full-length sectors".
pub fn sector_radius(range_nm: Option<f64>, full: bool, metres_per_px: f64) -> f64 {
    match range_nm {
        Some(nm) if full => (nm * crate::core::geo::NM_M / metres_per_px).clamp(20.0, 3000.0),
        Some(nm) => tokens::SECTOR_RADIUS * MM_PX * (nm / SECTOR_REFERENCE_NM).clamp(0.4, 1.5),
        None => tokens::SECTOR_RADIUS * MM_PX,
    }
}

/// Sector arc of a light, a band in the sector's colour with an ink edge each side, and the limits
/// as dashed ink lines to the arc. `from` and `to` are bearings in degrees true seen from seaward,
/// as in S-57; the arc is drawn on the light's own side.
pub fn light_sector(cr: &Context, pal: &Palette, x: f64, y: f64, radius: f64, from: f64, to: f64, colours: &[u8]) {
    // A previous label leaves a current point that arcs would connect to.
    cr.new_path();
    let start = arc_angle(from);
    let mut end = arc_angle(to);
    if end <= start {
        end += 2.0 * PI;
    }
    cr.set_line_cap(cairo::LineCap::Butt);
    cr.arc(x, y, radius, start, end);
    pal.set(cr, Token::ChartInk);
    cr.set_line_width(tokens::STROKE_SECTOR_ARC + 1.0);
    let _ = cr.stroke_preserve();
    pal.set(cr, light_colour(colours));
    cr.set_line_width(tokens::STROKE_SECTOR_ARC);
    let _ = cr.stroke();
    // The limits, dashed, from the light to the arc.
    cr.set_dash(&[5.0, 3.0], 0.0);
    pal.set(cr, Token::ChartInk);
    cr.set_line_width(0.8);
    for a in [start, end] {
        cr.move_to(x, y);
        cr.line_to(x + radius * a.cos(), y + radius * a.sin());
    }
    let _ = cr.stroke();
    cr.set_dash(&[], 0.0);
}

/// The full ring of an all-round major light at the sector radius.
pub fn light_ring(cr: &Context, pal: &Palette, x: f64, y: f64, radius: f64, colours: &[u8]) {
    cr.new_path();
    cr.arc(x, y, radius, 0.0, 2.0 * PI);
    pal.set(cr, Token::ChartInk);
    cr.set_line_width(tokens::STROKE_SECTOR_ARC + 1.0);
    let _ = cr.stroke_preserve();
    pal.set(cr, light_colour(colours));
    cr.set_line_width(tokens::STROKE_SECTOR_ARC);
    let _ = cr.stroke();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::Mode;

    fn canvas() -> (cairo::ImageSurface, Context) {
        let s = cairo::ImageSurface::create(cairo::Format::ARgb32, 120, 120).unwrap();
        let cr = Context::new(&s).unwrap();
        (s, cr)
    }

    /// The bounding box of what was drawn (alpha above zero), if anything.
    fn extent(mut s: cairo::ImageSurface) -> Option<(i32, i32, i32, i32)> {
        let stride = s.stride() as usize;
        let data = s.data().unwrap();
        let mut b: Option<(i32, i32, i32, i32)> = None;
        for y in 0..120usize {
            for x in 0..120usize {
                if data[y * stride + x * 4 + 3] > 0 {
                    let (x, y) = (x as i32, y as i32);
                    b = Some(b.map_or((x, y, x, y), |(a, c, d, e)| (a.min(x), c.min(y), d.max(x), e.max(y))));
                }
            }
        }
        b
    }

    fn pal() -> Palette {
        Palette::new(Mode::Day, false)
    }

    #[test]
    fn known_colour_codes_are_distinct() {
        let all: Vec<Paint> = (1..=13).map(s57_colour).collect();
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b);
            }
        }
        assert_eq!(s57_colour(99), Paint::Legacy(rgb(120, 120, 120)));
    }

    #[test]
    fn light_colours() {
        assert_eq!(light_colour(&[3]), Paint::Token(Token::LightRed));
        assert_eq!(light_colour(&[1, 3]), Paint::Token(Token::LightWhite), "the first colour decides");
        assert_eq!(light_colour(&[]), Paint::Token(Token::LightOther));
    }

    #[test]
    fn the_ecdis_style_keeps_the_old_colours() {
        set_style(Style::Ecdis);
        let red = s57_colour(3);
        let light = light_colour(&[4]);
        set_style(Style::Paper);
        assert_eq!(red, Paint::Legacy(rgb(214, 40, 40)));
        assert_eq!(light, Paint::Legacy(rgb(30, 170, 70)));
        assert_eq!(s57_colour(3), Paint::Token(Token::MarkRed));
        assert_eq!("ecdis".parse::<Style>(), Ok(Style::Ecdis));
    }

    #[test]
    fn buoys_lean_to_the_right_and_beacons_stand_upright() {
        let (s, cr) = canvas();
        buoy(&cr, &pal(), 60.0, 100.0, 40.0, Some(5), &[2], Mark::Plain);
        drop(cr);
        let (x0, _, x1, y1) = extent(s).unwrap();
        assert!(x1 - 60 > 60 - x0, "the top leans right: {x0}..{x1}");
        assert!(y1 <= 104, "the foot is on the position");

        let (s, cr) = canvas();
        beacon(&cr, &pal(), 60.0, 100.0, 40.0, Some(1), &[2], Mark::Plain);
        drop(cr);
        let (x0, _, x1, _) = extent(s).unwrap();
        assert!((x1 - 60 - (60 - x0)).abs() <= 3, "symmetric about the position: {x0}..{x1}");
    }

    #[test]
    fn topmarks_make_marks_taller() {
        let height = |mark| {
            let (s, cr) = canvas();
            buoy(&cr, &pal(), 60.0, 100.0, 30.0, Some(5), &[2, 6], mark);
            drop(cr);
            let (_, y0, _, y1) = extent(s).unwrap();
            y1 - y0
        };
        assert!(height(Mark::Cardinal(1)) > height(Mark::Plain) + 5);
        assert!(height(Mark::IsolatedDanger) > height(Mark::Plain) + 5);
    }

    #[test]
    fn a_flare_points_to_the_bearing_with_its_tip_on_the_light() {
        let (s, cr) = canvas();
        // South-east: the drop lies below and to the right of the light.
        flare(&cr, &pal(), 30.0, 30.0, 40.0, 135.0, Token::LightRed);
        drop(cr);
        let (x0, y0, x1, y1) = extent(s).unwrap();
        assert!(x0 >= 29 && y0 >= 29, "the tip is at the light: {x0},{y0}");
        assert!(x1 > 55 && y1 > 55, "and the body away from it: {x1},{y1}");
    }

    #[test]
    fn a_multicolour_light_fans_one_flare_per_colour() {
        let one = {
            let (s, cr) = canvas();
            light(&cr, &pal(), 40.0, 30.0, &[3], 0, None);
            drop(cr);
            extent(s).unwrap()
        };
        let two = {
            let (s, cr) = canvas();
            light(&cr, &pal(), 40.0, 30.0, &[3, 4], 0, None);
            drop(cr);
            extent(s).unwrap()
        };
        assert!(two.2 - two.0 > one.2 - one.0 || two.3 - two.1 > one.3 - one.1, "{one:?} {two:?}");
    }

    #[test]
    fn sector_arcs_scale_with_range_unless_full_length() {
        let token = tokens::SECTOR_RADIUS * MM_PX;
        assert_eq!(sector_radius(Some(10.0), false, 10.0), token);
        assert_eq!(sector_radius(Some(15.0), false, 10.0), token * 1.5);
        assert_eq!(sector_radius(Some(6.0), false, 10.0), token * 0.6);
        assert_eq!(sector_radius(Some(1.0), false, 10.0), token * 0.4);
        assert_eq!(sector_radius(Some(40.0), false, 10.0), token * 1.5);
        assert_eq!(sector_radius(None, false, 10.0), token);
    }

    #[test]
    fn far_reaching_lights_have_longer_flares() {
        let base = flare_length(None);
        assert_eq!(flare_length(Some(3.0)), base);
        assert_eq!(flare_length(Some(5.0)), base);
        assert!((flare_length(Some(15.0)) - base * 1.25).abs() < 1e-9);
        assert!((flare_length(Some(25.0)) - base * 1.5).abs() < 1e-9);
        assert!((flare_length(Some(40.0)) - base * 1.5).abs() < 1e-9);
        let full = sector_radius(Some(10.0), true, 10.0);
        assert!((full - 10.0 * 1852.0 / 10.0).abs() < 1e-6 || full == 3000.0);
        assert_eq!(sector_radius(None, true, 10.0), tokens::SECTOR_RADIUS * MM_PX);
    }
}

/// What an anchor stands for: an anchorage, or where anchoring is prohibited (INT 1 N20, crossed) or
/// restricted (N21, crossed with dashes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    Plain,
    Prohibited,
    Restricted,
}

/// An anchor (INT 1 N10) in chart magenta, `s` tall, centred on (x, y): a ring, the shank with its
/// stock, and the arms curving up to the flukes.
pub fn anchor(cr: &Context, pal: &Palette, x: f64, y: f64, s: f64, kind: Anchor) {
    let s = s * mark_scale();
    let _ = cr.save();
    cr.new_path();
    cr.set_line_width((s * 0.09).max(1.1));
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_dash(&[], 0.0);
    pal.set(cr, Token::ChartMagenta);
    let (top, bottom) = (y - s * 0.5, y + s * 0.42);
    let ring = s * 0.09;
    cr.arc(x, top + ring, ring, 0.0, 2.0 * PI);
    cr.move_to(x, top + 2.0 * ring);
    cr.line_to(x, bottom);
    cr.move_to(x - s * 0.24, y - s * 0.2);
    cr.line_to(x + s * 0.24, y - s * 0.2);
    // The arms: a half circle up from the crown to the flukes.
    let r = s * 0.36;
    cr.new_sub_path();
    cr.arc_negative(x, bottom - r, r, PI * 0.85, PI * 0.15);
    let _ = cr.stroke();
    for side in [-1.0, 1.0] {
        let (fx, fy) = (x + side * r * (PI * 0.15).cos(), bottom - r + r * (PI * 0.15).sin());
        cr.move_to(fx, fy);
        cr.line_to(fx + side * s * 0.03, fy - s * 0.14);
        cr.move_to(fx, fy);
        cr.line_to(fx - side * s * 0.13, fy - s * 0.02);
    }
    let _ = cr.stroke();
    if kind != Anchor::Plain {
        let d = s * 0.5;
        if kind == Anchor::Restricted {
            cr.set_dash(&[s * 0.14, s * 0.1], 0.0);
        }
        cr.move_to(x - d, y - d);
        cr.line_to(x + d, y + d);
        cr.move_to(x + d, y - d);
        cr.line_to(x - d, y + d);
        let _ = cr.stroke();
    }
    let _ = cr.restore();
}

/// An anchor berth (INT 1 N11): its number in a magenta circle of radius `r`, the number written by the caller.
pub fn berth(cr: &Context, pal: &Palette, x: f64, y: f64, r: f64) {
    cr.new_path();
    cr.arc(x, y, r, 0.0, 2.0 * PI);
    pal.set_alpha(cr, Token::ChartSea, 0.85);
    let _ = cr.fill_preserve();
    pal.set(cr, Token::ChartMagenta);
    cr.set_line_width(1.2);
    let _ = cr.stroke();
}
