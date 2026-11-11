//! A polar learnt from the instruments (tiers A and C).
//!
//! Each point is turned into a ratio to the boat's own polar at its wind, so the polar's shape inside a
//! bin is kept. Per bin (5 degrees of true wind angle by a column of the editor's wind speeds), the 85th
//! percentile of the ratios is taken: a boat sailed badly is slower, never faster, so the top of what it
//! did is what it can do. That is blended with the polar by the evidence (counted in 2-minute blocks, as
//! windows next to each other are not independent), smoothed lightly, and bounded unless the evidence is
//! strong. Where the log has nothing the boat's polar stays as it is.

use super::filter::{Point, Tier};
use super::stats::{quantile, weighted_quantile};
use crate::routing::polar::{EDITOR_TWS, Polar};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

/// Width of an angle bin.
pub const BIN_DEG: f64 = 5.0;
const BINS: usize = 36;
/// How many 2-minute blocks of evidence weigh as much as the boat's polar.
pub const PRIOR_BLOCKS: f64 = 8.0;
/// Seconds in a block of evidence.
const BLOCK_S: i64 = 120;
/// Evidence past which the learnt ratio is no longer bounded.
const STRONG_BLOCKS: f64 = 30.0;
/// The share of what the boat did that it can do.
pub const TOP_QUANTILE: f64 = 0.85;

/// What one bin holds.
#[derive(Clone, Debug, Serialize)]
pub struct Cell {
    /// The middle of the angle bin, and the column's wind speed.
    pub twa: f64,
    pub tws: f64,
    pub points: usize,
    /// Distinct 2-minute blocks, tier C counted at its weight.
    pub blocks: f64,
    /// The 85th percentile and the median of speed over the polar's.
    pub top: f64,
    pub median: f64,
    /// The top ratio as a speed at the middle of the bin.
    pub speed: f64,
}

#[derive(Clone, Debug)]
pub struct Learned {
    /// On the editor's wind speeds and the boat polar's angles.
    pub polar: Polar,
    /// 0..1 per cell of `polar`: how much is the log's rather than the boat polar's.
    pub confidence: Vec<Vec<f64>>,
    pub cells: Vec<Cell>,
    /// Per column, the best angle and VMG to windward and downwind, where the log covers them.
    pub best_up: Vec<Option<(f64, f64)>>,
    pub best_down: Vec<Option<(f64, f64)>>,
    pub hours: f64,
}

/// The column a wind speed belongs to: halfway to its neighbours.
pub fn column(tws: f64) -> usize {
    let c = &EDITOR_TWS;
    (0..c.len()).find(|&j| j + 1 == c.len() || tws < (c[j] + c[j + 1]) / 2.0).unwrap_or(c.len() - 1)
}

fn bin(twa: f64) -> usize {
    ((twa.abs() / BIN_DEG) as usize).min(BINS - 1)
}

fn centre(b: usize) -> f64 {
    (b as f64 + 0.5) * BIN_DEG
}

/// What a bin gathered: (ratio, weight) of each point, and the blocks seen with their best weight.
#[derive(Clone, Default)]
struct Bin(Vec<(f64, f64)>, HashMap<i64, f64>);

pub fn learn(points: &[Point], reference: &Polar) -> Option<Learned> {
    let cols = EDITOR_TWS.len();
    let mut data: Vec<Vec<Bin>> = vec![vec![Bin::default(); BINS]; cols];
    let mut hours = 0.0;
    for p in points.iter().filter(|p| p.tier != Tier::D) {
        let r = reference.speed(p.twa, p.tws);
        if r < 0.5 {
            continue;
        }
        let cell = &mut data[column(p.tws)][bin(p.twa)];
        cell.0.push((p.speed / r, p.weight));
        let block = cell.1.entry(p.t.timestamp().div_euclid(BLOCK_S)).or_insert(0.0);
        *block = block.max(p.weight);
        hours += p.weight * 10.0 / 3600.0;
    }
    if hours < 0.5 {
        return None;
    }
    let blocks: Vec<Vec<f64>> = data.iter().map(|col| col.iter().map(|c| c.1.values().sum()).collect()).collect();
    let tops: Vec<Vec<Option<f64>>> = data.iter().map(|col| col.iter().map(|c| weighted_quantile(&c.0, TOP_QUANTILE)).collect()).collect();
    // Spread each bin's evidence to its neighbours (half weight, an angle bin and a column away), then
    // blend with the polar (a ratio of 1): neighbours without data add nothing rather than pulling to 1.
    let (mut ev, mut dev) = (vec![vec![0.0; BINS]; cols], vec![vec![0.0; BINS]; cols]);
    for j in 0..cols {
        for i in 0..BINS {
            let Some(top) = tops[j][i] else { continue };
            for (dj, fj) in [(-1i64, 0.5), (0, 1.0), (1, 0.5)] {
                for (di, fi) in [(-1i64, 0.5), (0, 1.0), (1, 0.5)] {
                    let (k, m) = (j as i64 + dj, i as i64 + di);
                    if k < 0 || m < 0 || k >= cols as i64 || m >= BINS as i64 {
                        continue;
                    }
                    let w = fj * fi * blocks[j][i];
                    ev[k as usize][m as usize] += w;
                    dev[k as usize][m as usize] += w * (top - 1.0);
                }
            }
        }
    }
    let ratio: Vec<Vec<f64>> = (0..cols)
        .map(|j| {
            (0..BINS)
                .map(|i| {
                    let r = 1.0 + dev[j][i] / (ev[j][i] + PRIOR_BLOCKS);
                    if ev[j][i] >= STRONG_BLOCKS { r } else { r.clamp(0.6, 1.25) }
                })
                .collect()
        })
        .collect();
    // Read off at the polar's angles, between bin middles.
    let at = |grid: &Vec<Vec<f64>>, j: usize, a: f64| -> f64 {
        let x = (a / BIN_DEG - 0.5).clamp(0.0, (BINS - 1) as f64);
        let (i0, f) = (x.floor() as usize, x.fract());
        let i1 = (i0 + 1).min(BINS - 1);
        grid[j][i0] * (1.0 - f) + grid[j][i1] * f
    };
    let twa = reference.twa.clone();
    let speeds: Vec<Vec<f64>> = twa.iter().map(|a| (0..cols).map(|j| (at(&ratio, j, *a) * reference.speed(*a, EDITOR_TWS[j]) * 100.0).round() / 100.0).collect()).collect();
    let conf_grid: Vec<Vec<f64>> = ev.iter().map(|col| col.iter().map(|b| b / (b + PRIOR_BLOCKS)).collect()).collect();
    let confidence: Vec<Vec<f64>> = twa.iter().map(|a| (0..cols).map(|j| if *a == 0.0 { 0.0 } else { at(&conf_grid, j, *a) }).collect()).collect();
    let polar = Polar { tws: EDITOR_TWS.to_vec(), twa, speeds };
    let mut cells = Vec::new();
    for (j, col) in data.iter().enumerate() {
        for (i, c) in col.iter().enumerate() {
            if c.0.is_empty() {
                continue;
            }
            let rs: Vec<f64> = c.0.iter().map(|p| p.0).collect();
            let top = tops[j][i].unwrap_or(1.0);
            cells.push(Cell {
                twa: centre(i),
                tws: EDITOR_TWS[j],
                points: c.0.len(),
                blocks: blocks[j][i],
                top,
                median: quantile(&rs, 0.5).unwrap_or(1.0),
                speed: top * reference.speed(centre(i), EDITOR_TWS[j]),
            });
        }
    }
    // The best angles, where the log covers them.
    let covered = |j: usize, a: f64| blocks[j][bin(a)] / (blocks[j][bin(a)] + PRIOR_BLOCKS) >= 0.5;
    let best_up = (0..cols).map(|j| Some(polar.best_vmg(EDITOR_TWS[j], true)).filter(|(a, _)| covered(j, *a))).collect();
    let best_down = (0..cols).map(|j| Some(polar.best_vmg(EDITOR_TWS[j], false)).filter(|(a, _)| covered(j, *a))).collect();
    Some(Learned { polar, confidence, cells, best_up, best_down, hours })
}

impl Learned {
    /// The confidence at any angle and wind speed, read between the cells of `polar`.
    pub fn confidence_at(&self, twa: f64, tws: f64) -> f64 {
        let (i0, i1, fa) = bracket(&self.polar.twa, twa.abs().min(180.0));
        let (j0, j1, fw) = bracket(&self.polar.tws, tws);
        let c = &self.confidence;
        let row = |i: usize| c[i][j0] * (1.0 - fw) + c[i][j1] * fw;
        row(i0) * (1.0 - fa) + row(i1) * fa
    }
}

/// The two indices of `axis` around `x` and the weight of the second (the ends held).
fn bracket(axis: &[f64], x: f64) -> (usize, usize, f64) {
    let last = axis.len().saturating_sub(1);
    if axis.is_empty() || x <= axis[0] {
        return (0, 0, 0.0);
    }
    if x >= axis[last] {
        return (last, last, 0.0);
    }
    let i = axis.partition_point(|v| *v <= x);
    (i - 1, i, (x - axis[i - 1]) / (axis[i] - axis[i - 1]))
}

/// The polar binned straight from points, without ratios, blending or bounds: what not to do with a
/// model's wind, kept to show why.
pub fn naive(points: &[Point]) -> Polar {
    let mut bins: BTreeMap<(usize, usize), Vec<f64>> = BTreeMap::new();
    for p in points {
        bins.entry((column(p.tws), bin(p.twa))).or_default().push(p.speed);
    }
    let twa: Vec<f64> = std::iter::once(0.0).chain((0..BINS).map(centre)).collect();
    let speeds = twa
        .iter()
        .map(|a| (0..EDITOR_TWS.len()).map(|j| if *a == 0.0 { 0.0 } else { bins.get(&(j, bin(*a))).and_then(|v| quantile(v, TOP_QUANTILE)).unwrap_or(0.0) }).collect())
        .collect();
    Polar { tws: EDITOR_TWS.to_vec(), twa, speeds }
}

#[cfg(test)]
mod tests {
    use super::super::filter::{instrument_points, model_points};
    use super::super::synth::{self, Config, scaled};
    use super::*;

    #[test]
    fn learns_the_boat_where_it_sailed_and_keeps_the_polar_elsewhere() {
        let reference = Polar::generic_cruiser();
        // The real boat is slower upwind and quicker downwind than its polar says.
        let truth = scaled(&reference, 0.85, 1.08);
        let c = Config { hours: 300.0, truth: truth.clone(), speed_noise: 0.02, ..Default::default() };
        let pts = instrument_points(&synth::log(&c), None);
        let l = learn(&pts, &reference).unwrap();
        let mut checked = 0;
        for (row, a) in l.polar.twa.iter().enumerate() {
            for (col, w) in l.polar.tws.iter().enumerate() {
                let (got, want, conf) = (l.polar.speeds[row][col], truth.speed(*a, *w), l.confidence[row][col]);
                if conf >= 0.8 {
                    checked += 1;
                    assert!((got / want - 1.0).abs() < 0.05, "{a}° {w} kn: learnt {got:.2}, real {want:.2} (confidence {conf:.2})");
                }
            }
        }
        assert!(checked >= 8, "{checked}");
        // Where it beat, the best angle to windward is known.
        assert!(l.best_up.iter().any(Option::is_some));
    }

    #[test]
    fn nothing_far_from_the_data_changes() {
        use chrono::{Duration, TimeZone, Utc};
        let reference = Polar::generic_cruiser();
        let t0 = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        // An hour on a beam reach in 15 kn, 10% faster than the polar.
        let pts: Vec<Point> = (0..360)
            .map(|i| Point { t: t0 + Duration::seconds(i * 10), tier: Tier::A, twa: 92.0, tws: 15.0, speed: 1.1 * reference.speed(92.0, 15.0), weight: 1.0, hindcast: false, lead_h: None, hs: None, index: 0 })
            .collect();
        let l = learn(&pts, &reference).unwrap();
        let near = l.polar.speed(90.0, 15.0) / reference.speed(90.0, 15.0);
        assert!(near > 1.05 && near < 1.11, "{near}");
        for (a, w) in [(150.0, 30.0), (52.0, 5.0), (180.0, 40.0)] {
            assert!((l.polar.speed(a, w) - reference.speed(a, w)).abs() < 0.02, "{a}° {w} kn");
        }
        let cell = l.cells.iter().find(|c| c.twa == 92.5 && c.tws == 15.0).unwrap();
        assert_eq!((cell.points, cell.blocks), (360, 30.0));
    }

    #[test]
    fn a_model_wind_binned_naively_points_too_high() {
        // Why a forecast's wind gives factors and never a polar: its angle errors smear the polar, and the
        // binned polar sails fast at angles the boat never could.
        let c = Config { hours: 300.0, instruments: false, ..Default::default() };
        let pts = model_points(&synth::log(&c), None);
        let naive = naive(&pts);
        let w = 15.0;
        let (real_up, _) = c.truth.best_vmg(w, true);
        let (naive_up, _) = naive.best_vmg(w, true);
        assert!(real_up > 38.0, "{real_up}");
        assert!(naive_up < 35.0, "the naive polar beats at {naive_up}° in {w} kn, the boat at {real_up}°");
    }
}
