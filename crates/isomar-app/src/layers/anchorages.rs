//! Anchorages as INT 1 draws them, from OpenStreetMap seamarks (`AidsLayer`) and ENC cells (`EncLayer`) alike:
//! a magenta anchor at the point or inside the area (N10), the area's edge dashed (N12), with ticks inward and
//! the anchor crossed where anchoring is prohibited or restricted (N20, N21), and a berth's number in a circle
//! (N11). Anchorages for ships (tankers, explosives, quarantine, deep water) wait for a closer zoom, so a
//! harbour's approaches are not covered in them.

use super::names::{self, Placed, Style};
use crate::core::chart::aids::{Anchorage, AnchorageKind};
use crate::palette::{Palette, SEA};
use crate::symbols::{self, Anchor};
use crate::tokens::Token;
use crate::view::ViewInfo;

/// Areas show from this zoom, points and berths from the next; anchorages for ships from `MIN_ZOOM_SHIPS`.
pub const MIN_ZOOM_AREA: f64 = 10.0;
pub const MIN_ZOOM_POINT: f64 = 11.0;
pub const MIN_ZOOM_SHIPS: f64 = 12.0;
/// An area's name shows from here once it is `NAME_MIN_PX` wide. A point's is not written: it is the cove's
/// or the bay's, which the chart names already, and the popover tells it.
pub const MIN_ZOOM_AREA_NAME: f64 = 12.0;
const NAME_MIN_PX: f64 = 40.0;
/// No anchor within so many of its heights of another: mooring fields are mapped as rows of small areas.
const ANCHOR_SPACING: f64 = 3.0;
/// The anchor's height at 100% text size (the marks' size before they were made 40% bigger).
const ANCHOR_PX: f64 = 14.0;
/// A berth's circle, at 100% text size.
const BERTH_R: f64 = 7.5;
/// The ticks along the edge of an area where anchoring is prohibited: every so many pixels, so long.
const TICK_EVERY: f64 = 14.0;
const TICK_LEN: f64 = 4.0;

const NAME: Style = Style { size: 9.5, weight: pango::Weight::Normal, style: pango::Style::Italic, dot: 0.0, fill: Token::ChartMagenta, halo: SEA, caps: false };
const NUMBER: Style = Style { size: 8.0, ..NAME };

/// Whether `a` shows at `zoom`.
pub fn shown(a: &Anchorage, zoom: f64) -> bool {
    let from = if a.rings.is_empty() && a.edges.is_empty() { MIN_ZOOM_POINT } else { MIN_ZOOM_AREA };
    zoom >= if a.is_for_ships() { from.max(MIN_ZOOM_SHIPS) } else { from }
}

fn anchor_of(kind: AnchorageKind) -> Anchor {
    match kind {
        AnchorageKind::Prohibited => Anchor::Prohibited,
        AnchorageKind::Restricted => Anchor::Restricted,
        _ => Anchor::Plain,
    }
}

/// Its area's box on the screen, `None` for a point.
fn screen_box(vi: &ViewInfo, a: &Anchorage) -> Option<[f64; 4]> {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for p in a.rings.iter().flatten() {
        let (x, y) = vi.to_screen(&crate::core::geo::LatLon::new(p[1], p[0]));
        b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
    }
    (b[0] <= b[2]).then_some(b)
}

/// What is drawn of it at its spot: the anchor, or nothing for a sliver of an area a tile cut off (the
/// tile beside it has the rest, with its own anchor); and its name, if it has room.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Marks {
    symbol: bool,
    name: bool,
}

fn marks(vi: &ViewInfo, a: &Anchorage) -> Marks {
    if a.is_outline_only() {
        return Marks { symbol: false, name: false };
    }
    let s = ANCHOR_PX * symbols::mark_scale();
    let zoom = vi.zoom();
    match screen_box(vi, a) {
        None => Marks { symbol: true, name: false },
        Some(b) => {
            let size = (b[2] - b[0]).max(b[3] - b[1]);
            // A piece of an area is cut where its outline is not closed.
            let cut = a.edges.iter().any(|e| e.first() != e.last()) || a.edges.is_empty();
            Marks { symbol: size >= 1.5 * s || !cut, name: zoom >= MIN_ZOOM_AREA_NAME && size >= NAME_MIN_PX }
        }
    }
}

/// Which of `all` (on the screen of `vi`) get their anchor or berth drawn, by index: the biggest areas first,
/// none within `ANCHOR_SPACING` of one kept. Worked out once for an area, so every tile agrees.
pub fn thin(vi: &ViewInfo, all: &[&Anchorage]) -> Vec<bool> {
    let s = ANCHOR_PX * symbols::mark_scale();
    let size = |a: &Anchorage| screen_box(vi, a).map_or(0.0, |b| (b[2] - b[0]).max(b[3] - b[1]));
    let mut order: Vec<usize> = (0..all.len()).filter(|&i| marks(vi, all[i]).symbol).collect();
    order.sort_by(|&i, &j| size(all[j]).total_cmp(&size(all[i])).then(i.cmp(&j)));
    let mut kept: Vec<(f64, f64, AnchorageKind)> = Vec::new();
    let mut out = vec![false; all.len()];
    for i in order {
        let (x, y) = vi.to_screen(&all[i].pos);
        if kept.iter().any(|(kx, ky, kind)| *kind == all[i].kind && (kx - x).hypot(ky - y) < ANCHOR_SPACING * s) {
            continue;
        }
        kept.push((x, y, all[i].kind));
        out[i] = true;
    }
    out
}

/// The berth's circle and number, or the anchor, and the name beside or under it, as boxes on the screen
/// of `vi`: names of places keep clear of them. `symbol` is whether its anchor is drawn (`thin`).
pub fn boxes(vi: &ViewInfo, a: &Anchorage, symbol: bool, out: &mut Vec<[f64; 4]>) {
    let m = marks(vi, a);
    if !m.symbol || !symbol {
        return;
    }
    let (x, y) = vi.to_screen(&a.pos);
    let k = symbols::mark_scale();
    let r = if a.kind == AnchorageKind::Berth { BERTH_R * k } else { ANCHOR_PX * k * 0.55 };
    out.push([x - r, y - r, x + r, y + r]);
    if m.name && a.kind != AnchorageKind::Berth
        && let Some(n) = &a.name
    {
        // Measured as the name will be written, roughly: an italic's average letter.
        let em = NAME.size * k;
        let w = n.chars().count() as f64 * em * 0.5;
        out.push(name_box(a, x, y, w, em * 1.2));
    }
}

/// Where its name of `w` by `h` goes: beside a point's anchor, under an area's.
fn name_box(a: &Anchorage, x: f64, y: f64, w: f64, h: f64) -> [f64; 4] {
    let s = ANCHOR_PX * symbols::mark_scale();
    if a.rings.is_empty() {
        [x + s * 0.6, y - h / 2.0, x + s * 0.6 + w, y + h / 2.0]
    } else {
        [x - w / 2.0, y + s * 0.6, x + w / 2.0, y + s * 0.6 + h]
    }
}

/// Draws its edge, and its anchor or berth and its name when `symbol` (`thin`).
pub fn draw(cr: &cairo::Context, pal: &Palette, vi: &ViewInfo, a: &Anchorage, symbol: bool) {
    edge(cr, pal, vi, a);
    let m = marks(vi, a);
    if !m.symbol || !symbol {
        return;
    }
    let (x, y) = vi.to_screen(&a.pos);
    let k = symbols::mark_scale();
    let scale = cr.target().device_scale().0;
    if a.kind == AnchorageKind::Berth {
        if let Some(r) = a.radius_m.map(|m| m / vi.metres_per_px()).filter(|r| *r > BERTH_R * k * 1.5) {
            // Its swinging circle, once it is bigger than the symbol.
            cr.new_path();
            cr.arc(x, y, r, 0.0, std::f64::consts::TAU);
            pal.set_alpha(cr, Token::ChartMagenta, 0.7);
            cr.set_line_width(0.8);
            cr.set_dash(&[3.0, 3.0], 0.0);
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);
        }
        symbols::berth(cr, pal, x, y, BERTH_R * k);
        if let Some(n) = a.name.as_deref().and_then(|t| names::name(cr, t, &NUMBER, pal, scale)) {
            names::draw(cr, pal, &[Placed { x: x - n.w / 2.0, top: y - n.h / 2.0, name: n, style: NUMBER, dot: None }]);
        }
        return;
    }
    symbols::anchor(cr, pal, x, y, ANCHOR_PX, anchor_of(a.kind));
    if m.name
        && let Some(n) = a.name.as_deref().and_then(|t| names::name(cr, t, &NAME, pal, scale))
    {
        let b = name_box(a, x, y, n.w, n.h);
        names::draw(cr, pal, &[Placed { x: b[0], top: b[1], name: n, style: NAME, dot: None }]);
    }
}

/// The area's outline, dashed, with ticks inward where anchoring is prohibited or restricted.
fn edge(cr: &cairo::Context, pal: &Palette, vi: &ViewInfo, a: &Anchorage) {
    if a.edges.is_empty() {
        return;
    }
    let ticks = matches!(a.kind, AnchorageKind::Prohibited | AnchorageKind::Restricted) && !a.rings.is_empty();
    let _ = cr.save();
    pal.set_alpha(cr, Token::ChartMagenta, 0.9);
    cr.set_line_width(1.2);
    cr.set_line_join(cairo::LineJoin::Round);
    cr.set_dash(&[7.0, 4.0], 0.0);
    cr.new_path();
    let lines: Vec<Vec<(f64, f64)>> = a.edges.iter().map(|e| e.iter().map(|p| vi.to_screen(&crate::core::geo::LatLon::new(p[1], p[0]))).collect()).collect();
    for l in &lines {
        for (i, p) in l.iter().enumerate() {
            if i == 0 { cr.move_to(p.0, p.1) } else { cr.line_to(p.0, p.1) }
        }
    }
    let _ = cr.stroke();
    if ticks {
        cr.set_dash(&[], 0.0);
        for l in &lines {
            let side = inward(vi, a, l);
            // From half a step in, so short edges get one.
            let mut next = TICK_EVERY / 2.0;
            let mut along = 0.0;
            for w in l.windows(2) {
                let (dx, dy) = (w[1].0 - w[0].0, w[1].1 - w[0].1);
                let len = dx.hypot(dy);
                if len == 0.0 {
                    continue;
                }
                let (nx, ny) = (-dy / len * side, dx / len * side);
                while next <= along + len {
                    let t = (next - along) / len;
                    let (x, y) = (w[0].0 + dx * t, w[0].1 + dy * t);
                    cr.move_to(x, y);
                    cr.line_to(x + nx * TICK_LEN, y + ny * TICK_LEN);
                    next += TICK_EVERY;
                }
                along += len;
            }
        }
        let _ = cr.stroke();
    }
    let _ = cr.restore();
}

/// Which way the area lies from the line `l` on the screen: 1 to the left of `(-dy, dx)`, else -1.
fn inward(vi: &ViewInfo, a: &Anchorage, l: &[(f64, f64)]) -> f64 {
    for w in l.windows(2) {
        let (dx, dy) = (w[1].0 - w[0].0, w[1].1 - w[0].1);
        let len = dx.hypot(dy);
        if len < 2.0 {
            continue;
        }
        let (mx, my) = ((w[0].0 + w[1].0) / 2.0, (w[0].1 + w[1].1) / 2.0);
        let p = vi.position_at(mx - dy / len * 1.5, my + dx / len * 1.5);
        return if a.contains([p.lon, p.lat]) { 1.0 } else { -1.0 };
    }
    1.0
}

/// How far (`x`, `y`) is from `a` on the screen of `vi`, if within `tol`: its symbol's distance, or `tol`
/// itself inside its area, so that a mark in the area is picked before it.
pub fn hit(vi: &ViewInfo, a: &Anchorage, x: f64, y: f64, tol: f64) -> Option<f64> {
    if a.is_outline_only() {
        return None;
    }
    let (ax, ay) = vi.to_screen(&a.pos);
    let d = (ax - x).hypot(ay - y);
    if d <= tol.max(ANCHOR_PX * symbols::mark_scale() * 0.6) {
        return Some(d.min(tol));
    }
    let p = vi.position_at(x, y);
    (!a.rings.is_empty() && a.contains([p.lon, p.lat])).then_some(tol)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::geo::LatLon;
    use crate::view::View;

    fn vi(zoom: f64) -> ViewInfo {
        ViewInfo { view: View { center: LatLon::new(39.93, 3.96), zoom }, w: 800.0, h: 600.0 }
    }

    fn point(categories: Vec<u8>) -> Anchorage {
        Anchorage { kind: AnchorageKind::Anchorage, name: Some("Cala Galdana".into()), categories, pos: LatLon::new(39.93, 3.96), rings: vec![], edges: vec![], radius_m: None }
    }

    fn square(kind: AnchorageKind, half: f64) -> Anchorage {
        let (lon, lat) = (3.96, 39.93);
        let ring = vec![[lon - half, lat - half], [lon + half, lat - half], [lon + half, lat + half], [lon - half, lat + half]];
        let mut edge = ring.clone();
        edge.push(ring[0]);
        Anchorage { kind, name: Some("Pen er Men".into()), categories: vec![], pos: LatLon::new(lat, lon), rings: vec![ring], edges: vec![edge], radius_m: None }
    }

    #[test]
    fn anchorages_for_ships_wait_for_a_closer_zoom() {
        let (yachts, tankers) = (point(vec![7]), point(vec![3]));
        assert!(!shown(&yachts, 10.5));
        assert!(shown(&yachts, 11.0));
        assert!(!shown(&tankers, 11.5));
        assert!(shown(&tankers, 12.0));
        assert!(shown(&square(AnchorageKind::Anchorage, 0.01), 10.0), "an area from 10");
    }

    #[test]
    fn an_area_is_named_once_it_has_room_and_a_cut_sliver_has_no_anchor() {
        let a = square(AnchorageKind::Anchorage, 0.01);
        assert_eq!(marks(&vi(11.0), &a), Marks { symbol: true, name: false }, "too far out to name");
        assert_eq!(marks(&vi(14.0), &a), Marks { symbol: true, name: true });
        // Tiny but whole: its anchor stands for it.
        let tiny = square(AnchorageKind::Anchorage, 0.0001);
        assert!(marks(&vi(12.0), &tiny).symbol);
        // Tiny and cut by a tile: the neighbour draws it.
        let mut sliver = tiny.clone();
        sliver.edges[0].pop();
        assert!(!marks(&vi(12.0), &sliver).symbol);
    }

    #[test]
    fn a_row_of_mooring_areas_gets_an_anchor_every_few_heights() {
        let v = vi(13.0);
        let step = v.deg_per_px() * 15.0;
        let row: Vec<Anchorage> = (0..6)
            .map(|k| {
                let mut a = square(AnchorageKind::Anchorage, step * 0.6);
                a.pos.lon += k as f64 * step;
                a.rings[0].iter_mut().chain(a.edges[0].iter_mut()).for_each(|p| p[0] += k as f64 * step);
                a
            })
            .collect();
        let kept = thin(&v, &row.iter().collect::<Vec<_>>());
        // 15 px apart, 42 px of spacing at 100%: every third.
        assert_eq!(kept, vec![true, false, false, true, false, false]);
        // Another kind is not thinned by them.
        let mut no = row[1].clone();
        no.kind = AnchorageKind::Prohibited;
        assert_eq!(thin(&v, &[&row[0], &no]), vec![true, true]);
    }

    #[test]
    fn a_click_finds_the_anchor_or_the_inside_of_the_area() {
        let v = vi(14.0);
        let a = square(AnchorageKind::Prohibited, 0.01);
        let (x, y) = v.to_screen(&a.pos);
        assert_eq!(hit(&v, &a, x + 2.0, y, 10.0), Some(2.0));
        assert_eq!(hit(&v, &a, x + 60.0, y, 10.0), Some(10.0), "inside, behind any mark");
        assert_eq!(hit(&v, &a, x + 2000.0, y, 10.0), None);
    }

    #[test]
    fn ticks_point_into_the_area() {
        let v = vi(14.0);
        let a = square(AnchorageKind::Prohibited, 0.01);
        let lines: Vec<(f64, f64)> = a.edges[0].iter().map(|p| v.to_screen(&LatLon::new(p[1], p[0]))).collect();
        let side = inward(&v, &a, &lines);
        // The first side runs east along the south edge: the area is north of it, up the screen.
        let (dx, dy) = (lines[1].0 - lines[0].0, lines[1].1 - lines[0].1);
        let len = dx.hypot(dy);
        let ny = dx / len * side;
        assert!(ny < 0.0, "the tick points up the screen, into the area: {ny}");
    }
}
