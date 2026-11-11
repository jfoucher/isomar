//! Soundings: how a depth is written, drawn and thinned.

use super::*;

/// How a sounding is printed: whole metres, the tenth when shallow (drawn
/// small and lowered), and whether it is a drying height (negative depth,
/// printed without its sign and underlined).
pub fn sounding_parts(depth: f64) -> (String, Option<char>, bool) {
    let drying = depth < 0.0;
    let m = depth.abs();
    if m < 31.0 {
        let s = format!("{m:.1}");
        let (whole, tenth) = s.split_once('.').unwrap_or((&s, "0"));
        (whole.to_string(), tenth.chars().next(), drying)
    } else {
        (format!("{:.0}", m.floor()), None, drying)
    }
}

/// Ink of printed soundings, a single colour whatever the depth.
pub const SOUNDING_INK: Token = Token::ChartSounding;

/// Size of the tenth relative to the whole metres.
pub(super) const TENTH_SCALE: f64 = 0.7;

/// Draws a sounding centred on (`x`, `y`) in the current colour, as printed
/// charts do. Skipped, returning false, when it would overlap one already
/// placed in `placer`.
pub fn draw_sounding(cr: &cairo::Context, placer: &mut LabelPlacer, x: f64, y: f64, depth: f64, size: f64) -> bool {
    draw_sounding_placed_at(cr, placer, x, y, depth, size, 1.0)
}

/// Like `draw_sounding`, but placed in `placer` at (`x`, `y`) in a view `k` times smaller than the one it
/// is drawn in, at (`k x`, `k y`) and its usual size: what is placed at a whole zoom stays the same at
/// every zoom up to the next.
pub fn draw_sounding_placed_at(cr: &cairo::Context, placer: &mut LabelPlacer, x: f64, y: f64, depth: f64, size: f64, k: f64) -> bool {
    let (whole, tenth, drying) = sounding_parts(depth);
    // Boxes and offsets follow the user's text size, like the glyphs do.
    let size = size * text_scale() as f64 / 100.0;
    cr.set_font_size(size);
    let Ok(we) = cr.text_extents(&whole) else { return false };
    let mut width = we.x_advance();
    let tenth = tenth.map(|t| t.to_string());
    if let Some(t) = &tenth {
        cr.set_font_size(size * TENTH_SCALE);
        let Ok(te) = cr.text_extents(t) else { return false };
        width += te.x_advance();
    }
    let (x0, base) = (x - width / 2.0, y + size * 0.35);
    // A little air around each number keeps a far view from reading as a smear.
    let pad = size * 0.3;
    if !placer.try_place(x0 - pad, base - size * 0.75 - pad, width + 2.0 * pad, size + 3.0 + 2.0 * pad) {
        return false;
    }
    let (x0, base) = (k * x - width / 2.0, k * y + size * 0.35);
    cr.new_path();
    cr.set_font_size(size);
    cr.move_to(x0, base);
    let _ = cr.show_text(&whole);
    if let Some(t) = &tenth {
        cr.set_font_size(size * TENTH_SCALE);
        cr.move_to(x0 + we.x_advance(), base + size * 0.25);
        let _ = cr.show_text(t);
    }
    if drying {
        cr.new_path();
        cr.move_to(x0, base + size * 0.4);
        cr.line_to(x0 + width, base + size * 0.4);
        cr.set_line_width(0.9);
        let _ = cr.stroke();
    }
    cr.new_path();
    true
}

/// Room one printed sounding is given, in pixels at 100% text: about three
/// digits wide and two lines high. Grid soundings are picked on a lattice of
/// this size and `thin_soundings` measures its distances in it.
pub fn sounding_cell() -> (f64, f64) {
    let scale = text_scale() as f64 / 100.0;
    (56.0 * scale, 30.0 * scale)
}

/// How far, in sounding cells, a kept sounding silences others of about its
/// depth: more in deep water, where a few numbers tell enough.
pub(super) fn thin_radius(depth: f64) -> f64 {
    (2.0 + depth.abs() / 25.0).min(THIN_MAX_RADIUS)
}

pub const THIN_MAX_RADIUS: f64 = 4.0;

/// How much deeper than a kept sounding a neighbour must be to be printed too.
pub(super) fn thin_tolerance(depth: f64) -> f64 {
    0.5 + 0.15 * depth.abs()
}

/// The soundings worth printing, chosen the way a chart maker thins them:
/// shallowest first, and one near a sounding already kept is left out unless
/// it is notably deeper. Where the bottom is flat or deep few numbers remain;
/// where it shoals or drops off they stay dense, and no shoal is ever hidden
/// by a deeper number. `pts` are (depth, x, y) on screen, drying heights
/// negative.
pub fn thin_soundings(mut pts: Vec<(f64, f64, f64)>, cell: (f64, f64)) -> Vec<(f64, f64, f64)> {
    pts.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.total_cmp(&b.2)));
    // Buckets as large as the widest radius, so only the 3x3 round a sounding can silence it.
    let (bw, bh) = (cell.0 * THIN_MAX_RADIUS, cell.1 * THIN_MAX_RADIUS);
    let bucket = |x: f64, y: f64| ((x / bw).floor() as i64, (y / bh).floor() as i64);
    let mut kept: std::collections::HashMap<(i64, i64), Vec<(f64, f64, f64)>> = std::collections::HashMap::new();
    let mut out = Vec::new();
    for p in pts {
        let (bx, by) = bucket(p.1, p.2);
        let silenced = (bx - 1..=bx + 1).flat_map(|i| (by - 1..=by + 1).map(move |j| (i, j))).filter_map(|k| kept.get(&k)).flatten().any(|k| {
            let r = thin_radius(k.0);
            let (u, v) = ((p.1 - k.1) / (cell.0 * r), (p.2 - k.2) / (cell.1 * r));
            u * u + v * v <= 1.0 && p.0 - k.0 <= thin_tolerance(k.0)
        });
        if !silenced {
            kept.entry((bx, by)).or_default().push(p);
            out.push(p);
        }
    }
    out
}

/// Soundings are drawn wherever their cell is, like the soundings printed on
/// a paper chart; they are de-cluttered, so a coarse view shows only the
/// shallowest. Cells without a scale wait for a harbour-size view.
pub fn soundings_visible(cscl: u32, view_denominator: f64, zoom: f64) -> bool {
    if cscl == 0 { zoom >= 11.0 } else { cell_visible(cscl, view_denominator) }
}

/// How far past a tile, in pixels, the soundings are weighed.
pub(super) fn sounding_reach() -> f64 {
    let (w, h) = sounding_cell();
    (w.max(h) * THIN_MAX_RADIUS).ceil()
}
