//! A name set along a spine: letters spread to fill it as a paper chart's lettering does, each turned
//! to follow the curve, or the whole name straight where the curve is too tight.

use super::spine::{Path, Spine};

#[derive(Clone, Copy, Debug)]
pub struct FitParams {
    /// The tracking, as a fraction of the font size added after each letter, at least and at most.
    pub min_track_em: f64,
    pub max_track_em: f64,
    /// How much of the room along the spine the name fills when it can be spread that far.
    pub fill: f64,
    /// The most one letter may turn from the next, and the most the whole name turns, in degrees.
    pub max_turn_deg: f64,
    pub total_turn_deg: f64,
    /// The outline round the letters, in pixels, which the spine must leave room for.
    pub halo_px: f64,
    /// Where along the spine the name's middle goes, as a share of its length from its start, as near as the
    /// room allows: a cove's name starts on the cove and runs on out over the open water as far as it needs.
    /// `None` puts it in the middle of the longest room.
    pub anchor: Option<f64>,
}

impl FitParams {
    /// The same, for the spine taken the other way round.
    fn reversed(&self) -> FitParams {
        FitParams { anchor: self.anchor.map(|a| 1.0 - a), ..*self }
    }
}

/// One glyph (cluster) of a name placed along a spine, in world pixels at the zoom; `angle` in radians, screen orientation.
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphAt {
    pub x: f64,
    pub y: f64,
    pub angle: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fitted {
    /// One per cluster, spaces included.
    pub glyphs: Vec<GlyphAt>,
    /// The box of each letter (not the spaces), padded by a pixel.
    pub boxes: Vec<[f64; 4]>,
    pub extent: [f64; 4],
}

/// How far from its anchor a name set from it may be, in line heights: a cove's head has no room for letters.
const ANCHOR_SLACK: f64 = 2.0;

/// Sampling step along the spine, in pixels.
const STEP: f64 = 2.0;

/// Where along `path` there is room for letters of `height` with the outline: the longest run, or with an
/// `anchor` (arc length) the run nearest it.
fn longest_run(path: &Path, r: f64, anchor: Option<f64>) -> Option<(f64, f64)> {
    let len = path.length();
    let steps = (len / STEP).floor() as usize;
    let mut runs: Vec<(f64, f64)> = Vec::new();
    let mut run: Option<(f64, f64)> = None;
    for k in 0..=steps + 1 {
        let u = (k as f64 * STEP).min(len);
        if path.clear_at(u) >= r {
            run = Some(run.map_or((u, u), |(a, _)| (a, u)));
        } else if let Some(r) = run.take() {
            runs.push(r);
        }
        if u >= len {
            break;
        }
    }
    runs.extend(run);
    let runs = runs.into_iter().filter(|(a, b)| b > a);
    match anchor {
        Some(at) => runs.min_by(|x, y| far(*x, at).total_cmp(&far(*y, at))),
        None => runs.max_by(|x, y| (x.1 - x.0).total_cmp(&(y.1 - y.0))),
    }
}

/// How far `at` is from the run (`a`, `b`): none inside it.
fn far((a, b): (f64, f64), at: f64) -> f64 {
    (a - at).max(at - b).max(0.0)
}

/// Where a block `total` long starts in the room (`u0`, `u1`): its middle on the anchor if there is one, moved by
/// `shift` of its own length and kept in the room; else centred in the room, moved by `shift` of the room to spare.
fn start_in((u0, u1): (f64, f64), total: f64, anchor: Option<f64>, shift: f64) -> f64 {
    match anchor {
        Some(at) => (at - total / 2.0 + shift * total).clamp(u0, (u1 - total).max(u0)),
        None => u0 + (u1 - u0 - total) / 2.0 * (1.0 + shift.clamp(-1.0, 1.0)),
    }
}

/// The axis-aligned box of a `w` by `h` rectangle centred on (`x`, `y`) and turned by `angle`, padded by a pixel.
pub fn box_of(x: f64, y: f64, angle: f64, w: f64, h: f64) -> [f64; 4] {
    let (c, s) = (angle.cos().abs(), angle.sin().abs());
    let (hw, hh) = ((w * c + h * s) / 2.0 + 1.0, (w * s + h * c) / 2.0 + 1.0);
    [x - hw, y - hh, x + hw, y + hh]
}

/// The lines of a name set on two lines are this far apart, as a share of the line height.
pub const LINE_SPACING: f64 = 0.92;

/// One line of a name: its clusters' widths in px, and which of them are spaces.
pub type Line<'a> = (&'a [f64], &'a [bool]);

/// The gap after each cluster but the last: one track, two after a space (the word gap).
fn gap_weights(spaces: &[bool]) -> Vec<f64> {
    let n = spaces.len();
    (0..n).map(|i| if i + 1 == n { 0.0 } else if spaces[i] { 2.0 } else { 1.0 }).collect()
}

/// The angle a cluster `adv` wide centred at arc length `u` takes: the chord under it.
fn angle_under(path: &Path, u: f64, adv: f64) -> ([f64; 2], f64) {
    let (c, tangent) = path.at(u);
    if adv < 1.0 {
        return (c, tangent);
    }
    let (a, b) = (path.at(u - adv / 2.0).0, path.at(u + adv / 2.0).0);
    (c, (b[1] - a[1]).atan2(b[0] - a[0]))
}

fn extent_of(boxes: &[[f64; 4]]) -> [f64; 4] {
    boxes.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |e, b| [e[0].min(b[0]), e[1].min(b[1]), e[2].max(b[2]), e[3].max(b[3])])
}

/// How the lines `lines` sit on `path`, one above the other and centred on it, or `None` when they do
/// not fit. `None` is also what the readable side of a spine that turns back on itself gives.
/// `shift` moves them from the middle of the room towards either end, as a share of the room to spare (-1 to 1).
fn block_at(path: &Path, lines: &[Line], em: f64, height: f64, p: &FitParams, shift: f64) -> Option<Fitted> {
    let spacing = height * LINE_SPACING;
    // Half the block's height: the room the spine must leave either side, with the outline.
    let half = (lines.len() - 1) as f64 * spacing / 2.0 + height / 2.0;
    let anchor = p.anchor.map(|a| a * path.length());
    let (u0, u1) = longest_run(path, half + p.halo_px, anchor)?;
    let l = u1 - u0;
    let weights: Vec<Vec<f64>> = lines.iter().map(|(_, s)| gap_weights(s)).collect();
    let natural: Vec<f64> = lines.iter().map(|(a, _)| a.iter().sum()).collect();
    // The longest line fills the room; the others are tracked the same and centred on it.
    let longest = (0..lines.len()).max_by(|&a, &b| natural[a].total_cmp(&natural[b]))?;
    let weight_sum: f64 = weights[longest].iter().sum();
    let track = if lines[longest].0.len() > 1 && weight_sum > 0.0 { ((p.fill * l - natural[longest]) / weight_sum).clamp(p.min_track_em * em, p.max_track_em * em) } else { 0.0 };
    let totals: Vec<f64> = (0..lines.len()).map(|k| natural[k] + weights[k].iter().sum::<f64>() * track).collect();
    let total = totals.iter().copied().fold(0.0, f64::max);
    if total > l {
        return None;
    }
    let s0 = start_in((u0, u1), total, anchor, shift);
    // A name set from its anchor stays on it: it may only start a little past it, where there is no room right there.
    if anchor.is_some_and(|at| s0 - at > ANCHOR_SLACK * height || at - (s0 + total) > ANCHOR_SLACK * height) {
        return None;
    }
    // Each cluster: its line, whether it is a space, how far along the block its middle is, and its width.
    let mut along: Vec<(usize, bool, f64, f64)> = Vec::new();
    for (k, (adv, spaces)) in lines.iter().enumerate() {
        let mut run = (total - totals[k]) / 2.0;
        for (i, a) in adv.iter().enumerate() {
            along.push((k, spaces[i], run + a / 2.0, *a));
            run += a + weights[k][i] * track;
        }
    }
    let mut glyphs: Vec<GlyphAt> = along
        .iter()
        .map(|&(_, _, off, adv)| {
            let (c, angle) = angle_under(path, s0 + off, adv);
            GlyphAt { x: c[0], y: c[1], angle }
        })
        .collect();
    // Readable: not upside down. A spine that runs right to left is tried the other way round.
    let mean_cos = glyphs.iter().map(|g| g.angle.cos()).sum::<f64>() / glyphs.len() as f64;
    if mean_cos < 0.0 {
        return None;
    }
    let letters: Vec<usize> = (0..along.len()).filter(|&j| !along[j].1).collect();
    let turns = (1..along.len()).filter(|&j| along[j].0 == along[j - 1].0).map(|j| angle_diff(glyphs[j].angle, glyphs[j - 1].angle).abs()).fold(0.0, f64::max).to_degrees();
    let (lo, hi) = letters.iter().map(|&i| glyphs[i].angle).fold((f64::MAX, f64::MIN), |(lo, hi), a| (lo.min(a), hi.max(a)));
    if turns > p.max_turn_deg || (hi - lo).to_degrees() > p.total_turn_deg {
        // Too tight a curve for letters to follow it: the whole name straight, where it stays in the channel.
        let (a, b) = (path.at(s0).0, path.at(s0 + total).0);
        let angle = (b[1] - a[1]).atan2(b[0] - a[0]);
        if angle.cos() < 0.0 {
            return None;
        }
        let (c, s) = (angle.cos(), angle.sin());
        let mut k = 0.0;
        while k <= total + 1e-9 {
            let u = s0 + k;
            let (q, _) = path.at(u);
            let on_chord = [a[0] + c * k, a[1] + s * k];
            if (on_chord[0] - q[0]).hypot(on_chord[1] - q[1]) + half > path.clear_at(u) {
                return None;
            }
            k += height / 2.0;
        }
        for (g, &(_, _, off, _)) in glyphs.iter_mut().zip(&along) {
            *g = GlyphAt { x: a[0] + c * off, y: a[1] + s * off, angle };
        }
    }
    // The first line above the spine, the last below it.
    for (g, &(k, ..)) in glyphs.iter_mut().zip(&along) {
        let o = (k as f64 - (lines.len() - 1) as f64 / 2.0) * spacing;
        g.x -= g.angle.sin() * o;
        g.y += g.angle.cos() * o;
    }
    let boxes: Vec<[f64; 4]> = letters.iter().map(|&j| box_of(glyphs[j].x, glyphs[j].y, glyphs[j].angle, along[j].3, height)).collect();
    let extent = extent_of(&boxes);
    Some(Fitted { glyphs, boxes, extent })
}

/// `advances` are the clusters' widths in px (a space is a cluster, marked in `spaces`), `em` the font
/// size and `height` the line height in px. The spine is in units, `px_per_unit` takes it to pixels.
pub fn fit(spine: &Spine, px_per_unit: f64, advances: &[f64], spaces: &[bool], em: f64, height: f64, p: &FitParams) -> Option<Fitted> {
    fit_avoiding(spine, px_per_unit, advances, spaces, em, height, p, &|_| false)
}

/// Where along the room a name is tried when it must keep clear of something: the middle, then towards
/// either end, as shares of the room to spare (with an anchor, of the name's length either side of it).
const SHIFTS: [f64; 9] = [0.0, -0.25, 0.25, -0.5, 0.5, -0.75, 0.75, -1.0, 1.0];

/// `fit`, keeping every letter's box clear of what `avoid` says no to (the marks and lights): the name
/// slides along the room it has until it is clear, or does not fit.
#[allow(clippy::too_many_arguments)]
pub fn fit_avoiding(spine: &Spine, px_per_unit: f64, advances: &[f64], spaces: &[bool], em: f64, height: f64, p: &FitParams, avoid: &dyn Fn(&[f64; 4]) -> bool) -> Option<Fitted> {
    fit_lines_avoiding(spine, px_per_unit, &[(advances, spaces)], em, height, p, avoid)
}

/// `fit_avoiding` for a name on several lines, one above the other along the spine, each centred on it.
#[allow(clippy::too_many_arguments)]
pub fn fit_lines_avoiding(spine: &Spine, px_per_unit: f64, lines: &[Line], em: f64, height: f64, p: &FitParams, avoid: &dyn Fn(&[f64; 4]) -> bool) -> Option<Fitted> {
    if lines.is_empty() || lines.iter().any(|(a, s)| a.is_empty() || a.len() != s.len()) || spine.pts.len() < 2 {
        return None;
    }
    let forward = spine.path().scaled(px_per_unit);
    let half = (lines.len() - 1) as f64 * height * LINE_SPACING / 2.0 + height / 2.0;
    for path in beside(&forward, half + p.halo_px, height, p.anchor) {
        let backward = path.reversed();
        for shift in SHIFTS {
            if let Some(f) = [(&path, *p), (&backward, p.reversed())].into_iter().find_map(|(path, p)| block_at(path, lines, em, height, &p, shift))
                && !f.boxes.iter().any(avoid)
            {
                return Some(f);
            }
        }
    }
    None
}

/// The paths a name tries in turn: `path`, then moved across it by half a line `height` at a time either
/// way, nearest first, as far as the room either side holds letters `r` from their middle: a name kept off
/// the middle of a water by a light there goes beside it. A name set from an `anchor` moves as far as the room
/// there: the line runs on out over the open sea, where the room is not the cove's.
fn beside(path: &Path, r: f64, height: f64, anchor: Option<f64>) -> Vec<Path> {
    let most = match anchor {
        Some(a) => path.clear_at(a * path.length()),
        None => path.clear.iter().copied().fold(0.0, f64::max),
    };
    let room = most - r;
    let step = height / 2.0;
    let n = if step > 0.0 && room > 0.0 { (room / step).floor() as usize } else { 0 };
    std::iter::once(path.clone()).chain((1..=n).flat_map(|k| [path.offset(-(k as f64) * step), path.offset(k as f64 * step)])).collect()
}

/// The farthest a word of a name set apart is from the word before it, in ems: farther, and the words
/// no longer read as one name.
const MAX_WORD_GAP_EM: f64 = 3.0;

/// `fit_avoiding` for a name cut into its words (at its spaces, and after its hyphens where `hyphens`
/// says a cluster ends with one), each of which finds its own place along the room, in order and close
/// to the one before: a light between two words then no longer keeps the name out. `None` for a name of
/// one word, or where the words do not fit.
#[allow(clippy::too_many_arguments)]
pub fn fit_words(spine: &Spine, px_per_unit: f64, advances: &[f64], spaces: &[bool], hyphens: &[bool], em: f64, height: f64, p: &FitParams, avoid: &dyn Fn(&[f64; 4]) -> bool) -> Option<Fitted> {
    let n = advances.len();
    if n == 0 || spaces.len() != n || hyphens.len() != n || spine.pts.len() < 2 {
        return None;
    }
    // The words, as ranges of clusters without the spaces between them.
    let mut words: Vec<(usize, usize)> = Vec::new();
    let mut start = 0;
    for i in 0..n {
        if spaces[i] {
            if i > start {
                words.push((start, i));
            }
            start = i + 1;
        } else if hyphens[i] && i + 1 < n && !spaces[i + 1] {
            words.push((start, i + 1));
            start = i + 1;
        }
    }
    if start < n {
        words.push((start, n));
    }
    if words.len() < 2 {
        return None;
    }
    let forward = spine.path().scaled(px_per_unit);
    beside(&forward, height / 2.0 + p.halo_px, height, p.anchor).into_iter().find_map(|path| {
        let backward = path.reversed();
        [(&path, *p), (&backward, p.reversed())].into_iter().find_map(|(path, p)| words_along(path, advances, spaces, &words, em, height, &p, avoid))
    })
}

#[allow(clippy::too_many_arguments)]
fn words_along(path: &Path, advances: &[f64], spaces: &[bool], words: &[(usize, usize)], em: f64, height: f64, p: &FitParams, avoid: &dyn Fn(&[f64; 4]) -> bool) -> Option<Fitted> {
    let anchor = p.anchor.map(|a| a * path.length());
    let (u0, u1) = longest_run(path, height / 2.0 + p.halo_px, anchor)?;
    let track = p.min_track_em * em;
    let width = |&(a, b): &(usize, usize)| advances[a..b].iter().sum::<f64>() + track * (b - a - 1) as f64;
    // Between two words, the space that was there (and a track either side), or a track after a hyphen.
    let gap_before = |k: usize| {
        let (a, _) = words[k];
        if a > 0 && spaces[a - 1] { advances[a - 1] + 2.0 * track } else { track }
    };
    let least: f64 = words.iter().map(width).sum::<f64>() + (1..words.len()).map(gap_before).sum::<f64>();
    if least > u1 - u0 {
        return None;
    }
    let step = (em / 2.0).max(STEP);
    // A word set at arc length `u`: its letters, if they are readable, follow the curve and are clear.
    let set = |&(a, b): &(usize, usize), u: f64| -> Option<Vec<GlyphAt>> {
        let mut run = 0.0;
        let mut glyphs = Vec::with_capacity(b - a);
        for adv in &advances[a..b] {
            let (c, angle) = angle_under(path, u + run + adv / 2.0, *adv);
            glyphs.push(GlyphAt { x: c[0], y: c[1], angle });
            run += adv + track;
        }
        let turns = glyphs.windows(2).map(|w| angle_diff(w[1].angle, w[0].angle).abs()).fold(0.0, f64::max).to_degrees();
        let readable = glyphs.iter().map(|g| g.angle.cos()).sum::<f64>() >= 0.0;
        let clear = !glyphs.iter().zip(&advances[a..b]).any(|(g, adv)| avoid(&box_of(g.x, g.y, g.angle, *adv, height)));
        (readable && turns <= p.max_turn_deg && clear).then_some(glyphs)
    };
    // Where the first word starts: as near as can be to where the whole name, centred, would.
    let centred = start_in((u0, u1), least, anchor, 0.0);
    if anchor.is_some_and(|at| centred - at > ANCHOR_SLACK * height || at - (centred + least) > ANCHOR_SLACK * height) {
        return None;
    }
    let mut starts: Vec<f64> = (0..).map(|k| u0 + k as f64 * step).take_while(|s| s + least <= u1 + 1e-9).collect();
    starts.sort_by(|a, b| (a - centred).abs().total_cmp(&(b - centred).abs()));
    'starts: for start in starts {
        let mut placed: Vec<(usize, Vec<GlyphAt>)> = Vec::new();
        let mut u = start;
        for (k, w) in words.iter().enumerate() {
            if k > 0 {
                u += gap_before(k);
            }
            let earliest = u;
            // The rest of the words must still fit after this one.
            let rest: f64 = words[k + 1..].iter().map(width).sum::<f64>() + (k + 1..words.len()).map(gap_before).sum::<f64>();
            let glyphs = loop {
                if u + width(w) + rest > u1 + 1e-9 || u - earliest > MAX_WORD_GAP_EM * em {
                    continue 'starts;
                }
                if let Some(g) = set(w, u) {
                    break g;
                }
                // The first word stays where it starts: the next start moves it.
                if k == 0 {
                    continue 'starts;
                }
                u += step;
            };
            placed.push((k, glyphs));
            u += width(w);
        }
        // The words read as one line: none turns from the one before more than a letter may, nor the whole more than the name may.
        let mean = |g: &[GlyphAt]| g.iter().map(|g| g.angle.sin()).sum::<f64>().atan2(g.iter().map(|g| g.angle.cos()).sum::<f64>());
        let turns = placed.windows(2).map(|w| angle_diff(mean(&w[1].1), mean(&w[0].1)).abs()).fold(0.0, f64::max).to_degrees();
        let (lo, hi) = placed.iter().flat_map(|(_, g)| g.iter().map(|g| g.angle)).fold((f64::MAX, f64::MIN), |(lo, hi), a| (lo.min(a), hi.max(a)));
        if turns > p.max_turn_deg || (hi - lo).to_degrees() > p.total_turn_deg {
            continue 'starts;
        }
        let mut glyphs: Vec<GlyphAt> = Vec::with_capacity(advances.len());
        let mut boxes = Vec::new();
        for (k, word) in &placed {
            let (a, b) = words[*k];
            // The spaces before this word, between it and the last.
            while glyphs.len() < a {
                let at = glyphs.last().cloned().unwrap_or_else(|| word[0].clone());
                glyphs.push(GlyphAt { x: (at.x + word[0].x) / 2.0, y: (at.y + word[0].y) / 2.0, angle: word[0].angle });
            }
            for (g, adv) in word.iter().zip(&advances[a..b]) {
                boxes.push(box_of(g.x, g.y, g.angle, *adv, height));
                glyphs.push(g.clone());
            }
        }
        let extent = extent_of(&boxes);
        return Some(Fitted { glyphs, boxes, extent });
    }
    None
}

/// `a - b` folded into (-π, π].
fn angle_diff(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(std::f64::consts::TAU);
    if d > std::f64::consts::PI { d - std::f64::consts::TAU } else { d }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> FitParams {
        FitParams { min_track_em: 0.1, max_track_em: 2.0, fill: 0.8, max_turn_deg: 20.0, total_turn_deg: 70.0, halo_px: 2.5, anchor: None }
    }

    /// A straight spine `len` px long at one pixel to the unit, 20 px of room on either side.
    fn bar(len: f64) -> Spine {
        Spine { pts: vec![[0.0, 50.0], [len, 50.0]], clear: vec![20.0, 20.0] }
    }

    fn word(n: usize) -> (Vec<f64>, Vec<bool>) {
        (vec![8.0; n], vec![false; n])
    }

    #[test]
    fn tracking_spreads_a_short_name_over_a_long_spine_up_to_its_cap() {
        let (adv, sp) = word(6);
        let f = fit(&bar(200.0), 1.0, &adv, &sp, 12.0, 14.0, &params()).unwrap();
        let span = f.glyphs[5].x - f.glyphs[0].x + 8.0;
        assert!((span - 0.8 * 200.0).abs() < 1.0, "fills 80%: {span}");
        let f = fit(&bar(2000.0), 1.0, &adv, &sp, 12.0, 14.0, &params()).unwrap();
        let gap = (f.glyphs[1].x - f.glyphs[0].x) - 8.0;
        assert!((gap - 24.0).abs() < 1e-6, "tracked at most 2 em: {gap}");
        // And centred.
        assert!(((f.glyphs[0].x + f.glyphs[5].x) / 2.0 - 1000.0).abs() < 1e-6);
    }

    #[test]
    fn a_name_slides_along_its_room_to_keep_clear_of_a_mark() {
        let (adv, sp) = word(6);
        let p = FitParams { fill: 0.4, ..params() };
        let free = fit(&bar(400.0), 1.0, &adv, &sp, 12.0, 14.0, &p).unwrap();
        // A light in the middle of the bar.
        let light = [185.0, 40.0, 215.0, 60.0];
        let hits = |b: &[f64; 4]| b[0] < light[2] && b[2] > light[0] && b[1] < light[3] && b[3] > light[1];
        assert!(free.boxes.iter().any(hits), "centred, it is on the light");
        let moved = fit_avoiding(&bar(400.0), 1.0, &adv, &sp, 12.0, 14.0, &p, &hits).expect("it moves along");
        assert!(!moved.boxes.iter().any(hits));
        // With no room to move, it does not fit.
        let full = FitParams { fill: 0.95, min_track_em: 0.6, ..params() };
        assert!(fit_avoiding(&bar(140.0), 1.0, &adv, &sp, 12.0, 14.0, &full, &|b: &[f64; 4]| b[0] < 95.0 && b[2] > 45.0).is_none());
    }

    #[test]
    fn a_name_set_from_an_anchor_is_centred_on_it_and_runs_on_where_the_room_ends() {
        let (adv, sp) = word(6);
        let p = FitParams { min_track_em: 0.0, max_track_em: 0.0, ..params() };
        let middle = |f: &Fitted| (f.glyphs[0].x + f.glyphs[5].x) / 2.0;
        let at = fit(&bar(1000.0), 1.0, &adv, &sp, 12.0, 14.0, &FitParams { anchor: Some(0.3), ..p }).unwrap();
        assert!((middle(&at) - 300.0).abs() < 1e-6, "on its anchor: {}", middle(&at));
        // At the very end of the room, it starts there and runs back along it.
        let end = fit(&bar(1000.0), 1.0, &adv, &sp, 12.0, 14.0, &FitParams { anchor: Some(0.0), ..p }).unwrap();
        assert!((end.glyphs[0].x - 4.0).abs() < 1e-6, "from the end: {}", end.glyphs[0].x);
        // No room by the anchor (a cove's head): it starts a little past it, or not at all if that is far.
        let head = |room_from: f64| Spine { pts: vec![[0.0, 50.0], [room_from, 50.0], [room_from + 1.0, 50.0], [1000.0, 50.0]], clear: vec![2.0, 2.0, 20.0, 20.0] };
        let near = fit(&head(20.0), 1.0, &adv, &sp, 12.0, 14.0, &FitParams { anchor: Some(0.0), ..p }).expect("starts past the head");
        assert!(near.glyphs[0].x > 20.0 && near.glyphs[0].x < 40.0, "{}", near.glyphs[0].x);
        assert!(fit(&head(200.0), 1.0, &adv, &sp, 12.0, 14.0, &FitParams { anchor: Some(0.0), ..p }).is_none(), "not far out on the water");
    }

    #[test]
    fn a_word_gap_is_wider_than_a_letter_gap() {
        let adv = vec![8.0, 8.0, 5.0, 8.0, 8.0];
        let sp = vec![false, false, true, false, false];
        let f = fit(&bar(300.0), 1.0, &adv, &sp, 12.0, 14.0, &params()).unwrap();
        let gap = |i: usize| f.glyphs[i + 1].x - f.glyphs[i].x - (adv[i] + adv[i + 1]) / 2.0;
        assert!(gap(2) > gap(0) * 1.9, "{} against {}", gap(2), gap(0));
        assert_eq!(f.boxes.len(), 4, "no box for the space");
    }

    #[test]
    fn a_name_too_long_for_the_spine_does_not_fit() {
        let (adv, sp) = word(20);
        assert!(fit(&bar(100.0), 1.0, &adv, &sp, 12.0, 14.0, &params()).is_none());
        // Nor where the channel is narrower than the letters are tall.
        let thin = Spine { pts: vec![[0.0, 0.0], [500.0, 0.0]], clear: vec![4.0, 4.0] };
        let (adv, sp) = word(3);
        assert!(fit(&thin, 1.0, &adv, &sp, 12.0, 14.0, &params()).is_none());
    }

    #[test]
    fn the_longest_stretch_with_room_is_used() {
        // Room on the left 40 px, a pinch, then room for the rest.
        let s = Spine { pts: vec![[0.0, 0.0], [40.0, 0.0], [50.0, 0.0], [300.0, 0.0]], clear: vec![20.0, 20.0, 2.0, 20.0] };
        let (adv, sp) = word(5);
        let f = fit(&s, 1.0, &adv, &sp, 12.0, 14.0, &params()).unwrap();
        assert!(f.glyphs.iter().all(|g| g.x > 50.0), "{:?}", f.glyphs);
    }

    #[test]
    fn glyphs_are_never_upside_down() {
        // A spine drawn right to left is read the other way.
        let s = Spine { pts: vec![[200.0, 50.0], [0.0, 50.0]], clear: vec![20.0, 20.0] };
        let (adv, sp) = word(5);
        let f = fit(&s, 1.0, &adv, &sp, 12.0, 14.0, &params()).unwrap();
        assert!(f.glyphs.iter().all(|g| g.angle.cos() > 0.0), "{:?}", f.glyphs);
        assert!(f.glyphs[0].x < f.glyphs[4].x);
    }

    #[test]
    fn a_name_follows_a_gentle_curve() {
        let pts: Vec<[f64; 2]> = (0..=40).map(|i| {
            let a = -0.4 + 0.8 * i as f64 / 40.0;
            [300.0 * a.sin() / 0.4, 300.0 * (1.0 - a.cos()) / 0.4 * 0.5]
        }).collect();
        let s = Spine { clear: vec![20.0; pts.len()], pts };
        let (adv, sp) = word(8);
        let f = fit(&s, 1.0, &adv, &sp, 12.0, 14.0, &params()).unwrap();
        assert!(f.glyphs[0].angle != f.glyphs[7].angle, "the letters turn with the spine");
        assert!(f.glyphs.windows(2).all(|w| (w[1].angle - w[0].angle).abs() < 0.35));
    }

    #[test]
    fn a_tight_curve_falls_back_to_straight_or_nothing() {
        // A hairpin 60 px across: the letters cannot follow it.
        let pts = vec![[0.0, 0.0], [80.0, 0.0], [80.0, 30.0], [0.0, 30.0]];
        let s = Spine { clear: vec![25.0; 4], pts };
        let (adv, sp) = word(14);
        match fit(&s, 1.0, &adv, &sp, 12.0, 14.0, &params()) {
            None => {}
            Some(f) => {
                let a = f.glyphs[0].angle;
                assert!(f.glyphs.iter().all(|g| g.angle == a), "straight when kept: one angle");
            }
        }
        // A spine that bends sharply in a wide-open sea: straight across the bend.
        let open = Spine { pts: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0]], clear: vec![200.0; 3] };
        let (adv, sp) = word(10);
        let f = fit(&open, 1.0, &adv, &sp, 12.0, 14.0, &params()).unwrap();
        assert!(f.glyphs.iter().all(|g| g.angle == f.glyphs[0].angle), "{:?}", f.glyphs);
    }

    #[test]
    fn the_same_spine_fits_the_same_at_the_same_zoom() {
        let (adv, sp) = word(9);
        let s = Spine { pts: vec![[0.0, 0.0], [60.0, 10.0], [120.0, 40.0]], clear: vec![10.0, 12.0, 10.0] };
        let a = fit(&s, 2.0, &adv, &sp, 12.0, 14.0, &params());
        let b = fit(&s, 2.0, &adv, &sp, 12.0, 14.0, &params());
        assert_eq!(a, b);
        assert!(a.is_some());
        let (a, b) = (a.unwrap(), b.unwrap());
        assert!(a.glyphs.iter().zip(&b.glyphs).all(|(p, q)| p.x.to_bits() == q.x.to_bits() && p.angle.to_bits() == q.angle.to_bits()));
    }

    #[test]
    fn two_lines_sit_one_above_the_other_centred_on_the_spine() {
        let (a, sa) = word(5);
        let (b, sb) = word(3);
        let f = fit_lines_avoiding(&bar(300.0), 1.0, &[(&a, &sa), (&b, &sb)], 12.0, 14.0, &params(), &|_| false).unwrap();
        assert_eq!(f.glyphs.len(), 8);
        let (top, under) = (&f.glyphs[..5], &f.glyphs[5..]);
        assert!(top.iter().all(|g| (g.y - (50.0 - 14.0 * LINE_SPACING / 2.0)).abs() < 1e-9), "{top:?}");
        assert!(under.iter().all(|g| (g.y - (50.0 + 14.0 * LINE_SPACING / 2.0)).abs() < 1e-9), "{under:?}");
        let mid = |g: &[GlyphAt]| (g[0].x + g[g.len() - 1].x) / 2.0;
        assert!((mid(top) - mid(under)).abs() < 1e-9 && (mid(top) - 150.0).abs() < 1e-9, "both centred");
        // A channel that holds one line does not hold two.
        let one = Spine { pts: vec![[0.0, 0.0], [300.0, 0.0]], clear: vec![12.0, 12.0] };
        assert!(fit(&one, 1.0, &a, &sa, 12.0, 14.0, &params()).is_some());
        assert!(fit_lines_avoiding(&one, 1.0, &[(&a, &sa), (&b, &sb)], 12.0, 14.0, &params(), &|_| false).is_none());
    }

    #[test]
    fn words_find_their_own_place_round_a_light_between_them() {
        // ÉTANG DE THAU, with lights where the whole name would go however it slides.
        let adv = vec![8.0; 13];
        let mut sp = vec![false; 13];
        sp[5] = true;
        sp[8] = true;
        let p = FitParams { fill: 0.5, min_track_em: 0.0, ..params() };
        let lights = [[60.0, 40.0, 70.0, 60.0], [130.0, 40.0, 140.0, 60.0]];
        let hits = |b: &[f64; 4]| lights.iter().any(|l| b[0] < l[2] && b[2] > l[0] && b[1] < l[3] && b[3] > l[1]);
        assert!(fit_avoiding(&bar(200.0), 1.0, &adv, &sp, 12.0, 14.0, &p, &hits).is_none());
        let f = fit_words(&bar(200.0), 1.0, &adv, &sp, &[false; 13], 12.0, 14.0, &p, &hits).expect("the words go between the lights");
        assert_eq!(f.glyphs.len(), 13);
        assert!(!f.boxes.iter().any(hits));
        assert!(f.glyphs.windows(2).all(|w| w[1].x > w[0].x), "in order");
        // One word cannot be cut, unless at a hyphen.
        let (w, ws) = word(6);
        assert!(fit_words(&bar(200.0), 1.0, &w, &ws, &[false; 6], 12.0, 14.0, &p, &|_| false).is_none());
        let hy = [false, false, true, false, false, false];
        assert!(fit_words(&bar(200.0), 1.0, &w, &ws, &hy, 12.0, 14.0, &p, &|_| false).is_some());
    }

    #[test]
    fn boxes_hold_their_letters_and_the_extent_holds_the_boxes() {
        let (adv, sp) = word(5);
        let f = fit(&bar(200.0), 1.0, &adv, &sp, 12.0, 14.0, &params()).unwrap();
        for (g, b) in f.glyphs.iter().zip(&f.boxes) {
            assert!(b[0] < g.x && g.x < b[2] && b[1] < g.y && g.y < b[3]);
            assert!((b[2] - b[0] - 10.0).abs() < 1e-9 && (b[3] - b[1] - 16.0).abs() < 1e-9, "8 by 14 and a pixel each side: {b:?}");
        }
        assert!(f.boxes.iter().all(|b| b[0] >= f.extent[0] && b[2] <= f.extent[2]));
    }
}
