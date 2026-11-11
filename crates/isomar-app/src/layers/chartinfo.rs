//! What an S-57 cell says besides depths, land, marks and dangers: submarine cables and pipelines, leading
//! lines and recommended tracks, overfalls and tidal streams, unsurveyed ground, the zones of confidence of the
//! survey, and the areas a chart warns of (restricted, caution, military, dumping, cables, fish farms), drawn
//! in magenta as S-52 draws the information a mariner should not miss.

use super::enc::trace;
use super::{LabelPlacer, font_size, halo_text};
use crate::core::chart::s57::{Feature, Geom, attr, obj};
use crate::core::geo::LatLon;
use crate::core::vector::{self as core_vector, Bounds};
use crate::i18n::gettext_f;
use crate::palette::{MAGENTA, Palette};
use crate::tokens::Token;
use crate::view::ViewInfo;
use std::f64::consts::TAU;

/// How far past the view a stroke runs before it is cut short, as `enc::trace` does.
const MARGIN: f64 = 16.0;

/// Where a polyline (in screen pixels) is inside the box `[lo, hi_x] x [lo, hi_y]`: the part of the segment from
/// `a` to `b`, as the fractions `(t0, t1)` of it, or none.
fn clip_segment(a: (f64, f64), b: (f64, f64), hi_x: f64, hi_y: f64) -> Option<(f64, f64)> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, q) in [(-dx, a.0 + MARGIN), (dx, hi_x - a.0), (-dy, a.1 + MARGIN), (dy, hi_y - a.1)] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
        }
        if t0 > t1 {
            return None;
        }
    }
    Some((t0, t1))
}

/// Strokes a wave along `pts`, `amp` pixels each side and `wavelength` long. The phase counts from the start of
/// the line, so tiles that cut it in different places draw the same wave.
fn wavy(vi: &ViewInfo, cr: &cairo::Context, pts: &[[f64; 2]], amp: f64, wavelength: f64) {
    let screen: Vec<(f64, f64)> = pts.iter().map(|p| vi.to_screen(&LatLon::new(p[1], p[0]))).collect();
    let (hi_x, hi_y) = (vi.w + MARGIN, vi.h + MARGIN);
    let mut along = 0.0;
    let mut joined = false;
    for w in screen.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = (b.0 - a.0).hypot(b.1 - a.1);
        if len == 0.0 {
            continue;
        }
        match clip_segment(a, b, hi_x, hi_y) {
            Some((t0, t1)) => {
                let (nx, ny) = (-(b.1 - a.1) / len, (b.0 - a.0) / len);
                let steps = (((t1 - t0) * len / 2.0).ceil() as usize).max(1);
                for i in 0..=steps {
                    let t = t0 + (t1 - t0) * i as f64 / steps as f64;
                    let off = amp * ((along + t * len) / wavelength * TAU).sin();
                    let (x, y) = (a.0 + t * (b.0 - a.0) + nx * off, a.1 + t * (b.1 - a.1) + ny * off);
                    if i == 0 && !(joined && t0 == 0.0) {
                        cr.move_to(x, y);
                    } else {
                        cr.line_to(x, y);
                    }
                }
                joined = t1 >= 1.0;
            }
            None => joined = false,
        }
        along += len;
    }
}

/// Where along a line (in screen pixels) to put a label: the middle of the stretch inside the view, if it is long
/// enough to carry one, and the direction the line runs there.
fn middle_in_view(vi: &ViewInfo, pts: &[[f64; 2]], at_least: f64) -> Option<((f64, f64), f64)> {
    let screen: Vec<(f64, f64)> = pts.iter().map(|p| vi.to_screen(&LatLon::new(p[1], p[0]))).collect();
    let inside = |p: &(f64, f64)| p.0 > 20.0 && p.0 < vi.w - 20.0 && p.1 > 12.0 && p.1 < vi.h - 12.0;
    let total: f64 = screen.windows(2).map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1)).sum();
    if total < at_least {
        return None;
    }
    let mut left = total / 2.0;
    for w in screen.windows(2) {
        let len = (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
        if len >= left && len > 0.0 {
            let t = left / len;
            let p = (w[0].0 + t * (w[1].0 - w[0].0), w[0].1 + t * (w[1].1 - w[0].1));
            let bearing = (w[1].0 - w[0].0).atan2(-(w[1].1 - w[0].1)).to_degrees();
            return inside(&p).then_some((p, bearing));
        }
        left -= len;
    }
    None
}

/// An arrow from (`x`, `y`) `len` pixels along the compass `bearing` (clockwise from north, which is up).
fn arrow(cr: &cairo::Context, x: f64, y: f64, bearing: f64, len: f64, both_ends: bool) {
    let b = bearing.to_radians();
    let (ux, uy) = (b.sin(), -b.cos());
    let (tx, ty) = (x + ux * len / 2.0, y + uy * len / 2.0);
    let (hx, hy) = (x - ux * len / 2.0, y - uy * len / 2.0);
    cr.move_to(hx, hy);
    cr.line_to(tx, ty);
    // The two strokes of a head at (`px`, `py`) on an arrow that points along `base`.
    let head = |cr: &cairo::Context, px: f64, py: f64, base: f64| {
        for side in [-1.0, 1.0] {
            let a = base + std::f64::consts::PI + 0.45 * side;
            cr.move_to(px, py);
            cr.line_to(px + 5.5 * a.sin(), py - 5.5 * a.cos());
        }
    };
    head(cr, tx, ty, b);
    if both_ends {
        head(cr, hx, hy, b + std::f64::consts::PI);
    }
}

fn lines_of(f: &Feature) -> Option<&[[f64; 2]]> {
    if let Geom::Line(l) = &f.geom { Some(l) } else { None }
}

fn rings_of(f: &Feature) -> Option<&[Vec<[f64; 2]>]> {
    if let Geom::Area(r) = &f.geom { Some(r) } else { None }
}

/// The size on screen, in pixels, of the box round a feature's area.
fn extent_px(vi: &ViewInfo, rings: &[Vec<[f64; 2]>]) -> (f64, f64) {
    let mut b = Bounds::EMPTY;
    for r in rings {
        b.union(&Bounds::of(r));
    }
    let (x0, y0) = vi.to_screen(&LatLon::new(b.min_lat, b.min_lon));
    let (x1, y1) = vi.to_screen(&LatLon::new(b.max_lat, b.max_lon));
    ((x1 - x0).abs(), (y1 - y0).abs())
}

/// Unsurveyed ground, the grey of a chart with no data.
pub fn draw_unsurveyed(vi: &ViewInfo, cr: &cairo::Context, pal: &Palette, feats: &[&Feature]) {
    for f in feats.iter().filter(|f| f.objl == obj::UNSARE) {
        let Some(rings) = rings_of(f) else { continue };
        cr.new_path();
        for r in rings {
            trace(vi, cr, r, true);
        }
        pal.set_alpha(cr, Token::ChartVoid, 0.75);
        let _ = cr.fill();
    }
}

/// Cables, pipelines, leading lines and recommended tracks.
pub fn draw_lines(vi: &ViewInfo, cr: &cairo::Context, pal: &Palette, feats: &[&Feature], labels: &mut LabelPlacer) {
    let zoom = vi.zoom();
    let _ = cr.save();
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    for f in feats {
        let Some(line) = lines_of(f) else { continue };
        match f.objl {
            // A submarine cable is a wave, as INT 1 draws it.
            obj::CBLSUB => {
                cr.new_path();
                wavy(vi, cr, line, 1.7, 9.0);
                pal.set_alpha(cr, MAGENTA, 0.9);
                cr.set_line_width(1.0);
                let _ = cr.stroke();
            }
            // A pipeline is dashes and dots.
            obj::PIPSOL => {
                cr.new_path();
                trace(vi, cr, line, false);
                pal.set_alpha(cr, MAGENTA, 0.9);
                cr.set_line_width(1.1);
                cr.set_dash(&[8.0, 3.0, 1.0, 3.0], 0.0);
                let _ = cr.stroke();
                cr.set_dash(&[], 0.0);
            }
            // An overhead cable, with the clearance under it where the cell has it.
            obj::CBLOHD => {
                cr.new_path();
                trace(vi, cr, line, false);
                pal.set_alpha(cr, MAGENTA, 0.9);
                cr.set_line_width(1.2);
                let _ = cr.stroke();
                if zoom >= 12.0
                    && let Some(h) = f.num(attr::VERCLR)
                    && let Some(((x, y), _)) = middle_in_view(vi, line, 60.0)
                {
                    font_size(cr, 9.5);
                    let text = format!("{h:.0} m");
                    if labels.try_place(x - 14.0, y - 16.0, 28.0, 12.0) {
                        halo_text(cr, pal, x - 12.0, y - 6.0, &text, MAGENTA);
                    }
                }
            }
            // A leading line or clearing line: dashed, with the bearing it is to be sailed on.
            obj::NAVLNE => {
                cr.new_path();
                trace(vi, cr, line, false);
                pal.set_alpha(cr, MAGENTA, 0.9);
                cr.set_line_width(1.0);
                cr.set_dash(&[9.0, 4.0], 0.0);
                let _ = cr.stroke();
                cr.set_dash(&[], 0.0);
                bearing_label(vi, cr, pal, line, f.num(attr::ORIENT), labels, false);
            }
            // A recommended track: solid between fixed marks, dashed without; arrows for the traffic flow.
            obj::RECTRC => {
                cr.new_path();
                trace(vi, cr, line, false);
                pal.set_alpha(cr, MAGENTA, 0.9);
                cr.set_line_width(1.4);
                if f.num(attr::CATTRK).is_some_and(|c| c as i32 == 2) {
                    cr.set_dash(&[10.0, 5.0], 0.0);
                }
                let _ = cr.stroke();
                cr.set_dash(&[], 0.0);
                bearing_label(vi, cr, pal, line, f.num(attr::ORIENT), labels, true);
                // 1 inbound, 2 outbound and 3 one way go one way along the track; 4 is two-way.
                if let Some(flow) = f.num(attr::TRAFIC).map(|t| t as i32).filter(|t| (1..=4).contains(t))
                    && let (Some(((x, y), _)), Some(orient)) = (middle_in_view(vi, line, 50.0), f.num(attr::ORIENT))
                {
                    cr.new_path();
                    cr.set_line_width(1.4);
                    arrow(cr, x, y, orient, 26.0, flow == 4);
                    let _ = cr.stroke();
                }
            }
            _ => {}
        }
    }
    let _ = cr.restore();
}

/// A bearing written beside a line, as "123°", where its middle is in view.
fn bearing_label(vi: &ViewInfo, cr: &cairo::Context, pal: &Palette, line: &[[f64; 2]], orient: Option<f64>, labels: &mut LabelPlacer, below: bool) {
    let Some(orient) = orient else { return };
    if vi.zoom() < 11.0 {
        return;
    }
    let Some(((x, y), _)) = middle_in_view(vi, line, 90.0) else { return };
    font_size(cr, 9.5);
    let text = format!("{:03.0}°", orient.rem_euclid(360.0));
    let w = cr.text_extents(&text).map_or(26.0, |e| e.width());
    let top = if below { y + 5.0 } else { y - 17.0 };
    if labels.try_place(x - w / 2.0 - 2.0, top, w + 4.0, 12.0) {
        halo_text(cr, pal, x - w / 2.0, top + 10.0, &text, MAGENTA);
    }
}

/// S-57 `RESTRN` values a chart forbids outright: the rest limit (2, 4, 6…) or are not a prohibition (13, 14).
fn prohibited(restn: &[u8]) -> bool {
    restn.iter().any(|r| matches!(r, 1 | 3 | 5 | 7 | 9 | 11 | 15 | 16 | 18 | 20 | 22..=26))
}

/// The little symbol in the middle of an area, or at a point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
    /// A magenta ring with a stroke through it: something is forbidden.
    Prohibited,
    /// A magenta ring with an exclamation mark: take care, or entry is limited.
    Restricted,
    /// A magenta triangle with an exclamation mark: a caution.
    Caution,
}

/// How an area is outlined and what marks it, if the class is one that says something to a mariner.
struct AreaStyle {
    marker: Option<Marker>,
    width: f64,
    dash: [f64; 2],
    /// A band is shaded inside the edge, as S-52 hatches a restricted area's edge.
    band: bool,
}

/// Whether a caution area only says that its own cell leaves out the detail there: NOAA's rescheme puts one in a
/// small-scale cell over every place a larger-scale cell covers ("Most features, including bathymetry, are omitted
/// in this area… mariners should use a more appropriate navigational purpose"), 75% of the Gulf of Mexico's
/// overview cell, about 3,300 in 989 cells. The chart draws the larger-scale cell there by itself as soon as the
/// zoom suits it, so the area is neither drawn nor reported: it covered the overview, and the pointer was always
/// over it.
pub fn omits_detail(f: &Feature) -> bool {
    f.objl == obj::CTNARE
        && f.attr(attr::INFORM).is_some_and(|t| {
            let t = t.to_lowercase();
            t.contains("omitted") && (t.contains("minimal depiction") || t.contains("most features") || t.contains("large scale"))
        })
}

fn area_style(f: &Feature) -> Option<AreaStyle> {
    if omits_detail(f) {
        return None;
    }
    let restn = f.list(attr::RESTRN);
    let (marker, width, dash, band) = match f.objl {
        obj::RESARE => (Some(if prohibited(&restn) { Marker::Prohibited } else { Marker::Restricted }), 1.6, [10.0, 5.0], true),
        obj::CTNARE => (Some(Marker::Caution), 1.3, [5.0, 3.0], false),
        obj::MIPARE | obj::DMPGRD | obj::OSPARE | obj::SPLARE | obj::MARCUL | obj::PIPARE | obj::CBLARE => (Some(if prohibited(&restn) { Marker::Prohibited } else { Marker::Restricted }), 1.2, [6.0, 4.0], false),
        // A fishing zone or the limits of a state say where rules change, not what to do.
        obj::FSHZNE | obj::TESARE | obj::ISTZNE => (None, 1.0, [3.0, 4.0], false),
        _ => return None,
    };
    Some(AreaStyle { marker, width, dash, band })
}

/// The marker an area of this feature is charted with, where its class has one.
pub fn area_marker(f: &Feature) -> Option<Marker> {
    area_style(f)?.marker
}

pub fn marker(cr: &cairo::Context, pal: &Palette, x: f64, y: f64, m: Marker) {
    let _ = cr.save();
    pal.set(cr, MAGENTA);
    cr.set_line_width(1.4);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.new_path();
    match m {
        Marker::Prohibited => {
            cr.arc(x, y, 7.0, 0.0, TAU);
            cr.move_to(x - 5.0, y + 5.0);
            cr.line_to(x + 5.0, y - 5.0);
            let _ = cr.stroke();
        }
        Marker::Restricted => {
            cr.arc(x, y, 7.0, 0.0, TAU);
            let _ = cr.stroke();
            exclamation(cr, x, y);
        }
        Marker::Caution => {
            cr.move_to(x, y - 8.0);
            cr.line_to(x + 8.0, y + 6.0);
            cr.line_to(x - 8.0, y + 6.0);
            cr.close_path();
            let _ = cr.stroke();
            exclamation(cr, x, y + 1.0);
        }
    }
    let _ = cr.restore();
}

fn exclamation(cr: &cairo::Context, x: f64, y: f64) {
    cr.new_path();
    cr.move_to(x, y - 3.5);
    cr.line_to(x, y + 0.8);
    let _ = cr.stroke();
    cr.new_path();
    cr.arc(x, y + 3.2, 0.9, 0.0, TAU);
    let _ = cr.fill();
}

/// How wide an area must be on screen for its marker, in pixels.
const MARKER_MIN_PX: f64 = 36.0;

/// Restricted, caution and other areas outlined in magenta, and the zones of confidence in grey. Areas that
/// are anchorages are drawn by the anchorage code.
pub fn draw_areas(vi: &ViewInfo, cr: &cairo::Context, pal: &Palette, feats: &[&Feature], is_anchorage: impl Fn(&Feature) -> bool, labels: &mut LabelPlacer) {
    let zoom = vi.zoom();
    let _ = cr.save();
    cr.set_line_join(cairo::LineJoin::Round);
    for f in feats {
        if f.objl == obj::M_QUAL {
            if let Some(rings) = rings_of(f) {
                draw_zone_of_confidence(vi, cr, pal, f, rings, labels);
            }
            continue;
        }
        let Some(style) = area_style(f) else { continue };
        if is_anchorage(f) {
            continue;
        }
        match &f.geom {
            Geom::Area(rings) => {
                if style.band {
                    let _ = cr.save();
                    cr.new_path();
                    for r in rings {
                        trace(vi, cr, r, true);
                    }
                    cr.clip_preserve();
                    pal.set_alpha(cr, MAGENTA, 0.16);
                    cr.set_line_width(9.0);
                    let _ = cr.stroke();
                    let _ = cr.restore();
                }
                cr.new_path();
                for r in rings {
                    trace(vi, cr, r, true);
                }
                pal.set_alpha(cr, MAGENTA, 0.9);
                cr.set_line_width(style.width);
                cr.set_dash(&style.dash, 0.0);
                let _ = cr.stroke();
                cr.set_dash(&[], 0.0);
                if let (Some(m), Some(c)) = (style.marker, core_vector::label_point(rings))
                    && zoom >= 9.0
                {
                    let (w, h) = extent_px(vi, rings);
                    let (x, y) = vi.to_screen(&LatLon::new(c[1], c[0]));
                    if w.min(h) >= MARKER_MIN_PX && (0.0..vi.w).contains(&x) && (0.0..vi.h).contains(&y) && labels.try_place(x - 9.0, y - 9.0, 18.0, 18.0) {
                        marker(cr, pal, x, y, m);
                    }
                }
            }
            Geom::Point(p) => {
                if let Some(m) = style.marker {
                    let (x, y) = vi.to_screen(&LatLon::new(p[1], p[0]));
                    if (0.0..vi.w).contains(&x) && (0.0..vi.h).contains(&y) && labels.try_place(x - 9.0, y - 9.0, 18.0, 18.0) {
                        marker(cr, pal, x, y, m);
                    }
                }
            }
            _ => {}
        }
    }
    let _ = cr.restore();
}

/// A zone of confidence's name, "A1" to "D" or "U", as S-57's `CATZOC` has it.
pub fn zone_of_confidence(code: u8) -> Option<&'static str> {
    Some(match code {
        1 => "A1",
        2 => "A2",
        3 => "B",
        4 => "C",
        5 => "D",
        6 => "U",
        _ => return None,
    })
}

/// The edge of a zone of confidence, dotted and quiet, with its category written once inside it.
fn draw_zone_of_confidence(vi: &ViewInfo, cr: &cairo::Context, pal: &Palette, f: &Feature, rings: &[Vec<[f64; 2]>], labels: &mut LabelPlacer) {
    let zone = f.num(attr::CATZOC).and_then(|c| zone_of_confidence(c as u8));
    cr.new_path();
    for r in rings {
        trace(vi, cr, r, true);
    }
    pal.set_alpha(cr, Token::ChartLabel, 0.55);
    cr.set_line_width(0.9);
    cr.set_dash(&[1.5, 3.5], 0.0);
    let _ = cr.stroke();
    cr.set_dash(&[], 0.0);
    let (Some(zone), Some(c)) = (zone, core_vector::label_point(rings)) else { return };
    if vi.zoom() < 9.0 {
        return;
    }
    let (w, h) = extent_px(vi, rings);
    let (x, y) = vi.to_screen(&LatLon::new(c[1], c[0]));
    if w.min(h) < 80.0 || !(0.0..vi.w).contains(&x) || !(0.0..vi.h).contains(&y) {
        return;
    }
    font_size(cr, 9.5);
    let text = gettext_f("ZOC {zone}", &[("zone", zone)]);
    let tw = cr.text_extents(&text).map_or(40.0, |e| e.width());
    if labels.try_place(x - tw / 2.0 - 2.0, y - 8.0, tw + 4.0, 12.0) {
        halo_text(cr, pal, x - tw / 2.0, y + 2.0, &text, Token::ChartLabel);
    }
}

/// Overfalls, eddies and breakers, and the tidal streams a cell gives with their rates.
pub fn draw_symbols(vi: &ViewInfo, cr: &cairo::Context, pal: &Palette, feats: &[&Feature], labels: &mut LabelPlacer) {
    let _ = cr.save();
    cr.set_line_cap(cairo::LineCap::Round);
    for f in feats {
        match (f.objl, &f.geom) {
            (obj::WATTUR, Geom::Point(p)) => {
                let (x, y) = vi.to_screen(&LatLon::new(p[1], p[0]));
                if !(-10.0..vi.w + 10.0).contains(&x) || !(-10.0..vi.h + 10.0).contains(&y) {
                    continue;
                }
                turbulence(cr, pal, x, y);
                labels.try_place(x - 9.0, y - 8.0, 18.0, 16.0);
            }
            (obj::WATTUR, Geom::Area(rings)) => {
                // The outline, dotted, and the symbol once in the middle.
                cr.new_path();
                for r in rings {
                    trace(vi, cr, r, true);
                }
                pal.set_alpha(cr, Token::ChartInk, 0.6);
                cr.set_line_width(1.0);
                cr.set_dash(&[1.2, 3.0], 0.0);
                let _ = cr.stroke();
                cr.set_dash(&[], 0.0);
                if let Some(c) = core_vector::label_point(rings) {
                    let (x, y) = vi.to_screen(&LatLon::new(c[1], c[0]));
                    if extent_px(vi, rings).0 >= 24.0 && (0.0..vi.w).contains(&x) && (0.0..vi.h).contains(&y) {
                        turbulence(cr, pal, x, y);
                    }
                }
            }
            (obj::TS_FEB, Geom::Point(p)) => {
                let Some(orient) = f.num(attr::ORIENT) else { continue };
                let (x, y) = vi.to_screen(&LatLon::new(p[1], p[0]));
                if !(0.0..vi.w).contains(&x) || !(0.0..vi.h).contains(&y) || vi.zoom() < 10.0 {
                    continue;
                }
                // Flood streams are drawn solid, ebb streams dashed.
                pal.set(cr, Token::ChartInk);
                cr.set_line_width(1.3);
                if f.num(attr::CAT_TS).is_some_and(|c| c as i32 == 2) {
                    cr.set_dash(&[4.0, 3.0], 0.0);
                }
                cr.new_path();
                arrow(cr, x, y, orient, 30.0, false);
                let _ = cr.stroke();
                cr.set_dash(&[], 0.0);
                if let Some(v) = f.num(attr::CURVEL) {
                    font_size(cr, 9.5);
                    let b = orient.to_radians();
                    let (lx, ly) = (x + 22.0 * b.sin(), y - 22.0 * b.cos());
                    let text = format!("{v:.1}");
                    let w = cr.text_extents(&text).map_or(16.0, |e| e.width());
                    if labels.try_place(lx - w / 2.0 - 1.0, ly - 7.0, w + 2.0, 12.0) {
                        halo_text(cr, pal, lx - w / 2.0, ly + 3.0, &text, Token::ChartInk);
                    }
                }
            }
            _ => {}
        }
    }
    let _ = cr.restore();
}

/// Three short waves one above another, as INT 1 marks overfalls and tide rips.
fn turbulence(cr: &cairo::Context, pal: &Palette, x: f64, y: f64) {
    pal.set(cr, Token::ChartInk);
    cr.set_line_width(1.1);
    cr.new_path();
    for k in -1..=1 {
        let yy = y + 3.6 * k as f64;
        cr.move_to(x - 6.0, yy);
        cr.curve_to(x - 3.5, yy - 3.0, x - 1.5, yy - 3.0, x, yy);
        cr.curve_to(x + 1.5, yy + 3.0, x + 3.5, yy + 3.0, x + 6.0, yy);
    }
    let _ = cr.stroke();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_segment_is_cut_to_the_view() {
        let (t0, t1) = clip_segment((-100.0, 50.0), (100.0, 50.0), 300.0, 300.0).unwrap();
        assert!((t0 - (100.0 - MARGIN) / 200.0).abs() < 1e-9 && (t1 - 1.0).abs() < 1e-9, "{t0} {t1}");
        assert!(clip_segment((-100.0, -100.0), (-50.0, -90.0), 300.0, 300.0).is_none(), "wholly outside");
        assert_eq!(clip_segment((10.0, 10.0), (20.0, 20.0), 300.0, 300.0), Some((0.0, 1.0)));
    }

    #[test]
    fn prohibitions_are_told_from_restrictions() {
        assert!(prohibited(&[7]));
        assert!(prohibited(&[2, 3]));
        assert!(!prohibited(&[2, 8, 27]));
        assert!(!prohibited(&[14]));
    }

    #[test]
    fn the_zones_of_confidence_have_their_names() {
        assert_eq!(zone_of_confidence(1), Some("A1"));
        assert_eq!(zone_of_confidence(3), Some("B"));
        assert_eq!(zone_of_confidence(6), Some("U"));
        assert_eq!(zone_of_confidence(0), None);
    }
}
