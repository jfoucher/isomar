//! What the instruments get wrong, found from the log itself.
//!
//! - **Masthead offset** (the unit turned on its mount): every apparent angle is off by the same amount to
//!   one side, so the boat seems to point higher on one tack than the other. Close-hauled, a boat is
//!   sailed at the same true angle on either tack, so half the difference of the apparent angles on the
//!   two tacks, at like wind speeds, is the offset.
//! - **Angle error** (upwash, leeway): the angle reads too wide (or narrow) on both tacks alike. The true
//!   wind does not turn because the boat tacked, yet the computed direction jumps by twice the error at
//!   every tack (and gybe). The error is the one that removes the jumps, upwind and downwind apart.
//! - **Speedo factor**: over an hour with several headings, the speed over ground is the speed through the
//!   water times the factor plus a current; least squares gives both, and the median over the hours the
//!   factor.
//!
//! The result is shown to the user and, once saved, applied by the analysis to the raw readings; it never
//! changes the live instruments.

use super::sample::Sample;
use super::stats::{Circular, angle_diff, median, wrap180};
use crate::comm::state::true_wind;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Calibration {
    /// Degrees the masthead unit adds to every apparent angle (positive: to starboard).
    pub awa_offset: Option<f64>,
    /// Degrees the apparent angle reads too wide, upwind and downwind.
    pub angle_error: [Option<f64>; 2],
    /// What the speedo's reading must be multiplied by.
    pub speed_factor: Option<f64>,
    /// How much evidence each rests on.
    pub tacks: usize,
    pub gybes: usize,
    pub speed_hours: usize,
    pub upwind_hours: f64,
    /// When, in seconds since 1970 (chrono's serde is not built in).
    pub computed_at: Option<i64>,
}

/// A sample with what the calibration needs.
#[derive(Clone, Copy)]
struct Raw {
    t: DateTime<Utc>,
    heading: f64,
    awa: f64,
    aws: f64,
    stw: f64,
    steady: bool,
}

impl Calibration {
    pub fn is_empty(&self) -> bool {
        self.awa_offset.is_none() && self.angle_error.iter().all(Option::is_none) && self.speed_factor.is_none()
    }

    /// The apparent angle with the offset and the angle error taken out.
    pub fn correct_awa(&self, awa: f64, aws: f64, stw: f64) -> f64 {
        correct(awa, aws, stw, self.awa_offset.unwrap_or(0.0), [self.angle_error[0].unwrap_or(0.0), self.angle_error[1].unwrap_or(0.0)])
    }

    pub fn correct_stw(&self, stw: f64) -> f64 {
        stw * self.speed_factor.unwrap_or(1.0)
    }

    /// The true wind angle and speed from raw readings, corrected.
    pub fn true_wind(&self, awa: f64, aws: f64, stw: f64) -> (f64, f64) {
        let stw = self.correct_stw(stw);
        true_wind(self.correct_awa(awa, aws, stw), aws, stw)
    }
}

/// Takes `offset` out, then `error` (upwind or downwind by the true angle it gives) from the size of the angle.
fn correct(awa: f64, aws: f64, stw: f64, offset: f64, error: [f64; 2]) -> f64 {
    let a = wrap180(awa - offset);
    let (twa, _) = true_wind(a, aws, stw);
    let e = error[if twa.abs() < 90.0 { 0 } else { 1 }];
    let size = (a.abs() - e).clamp(0.0, 180.0);
    if a < 0.0 { -size } else { size }
}

/// The samples with raw readings, the speedo corrected by `speed_factor` (it changes the true wind, so it
/// is found first).
fn raws(samples: &[Sample], speed_factor: f64) -> Vec<Raw> {
    let mut v: Vec<Raw> = samples
        .iter()
        .filter(|s| s.sailing())
        .filter_map(|s| {
            Some(Raw {
                t: s.t,
                heading: s.heading?,
                awa: s.awa?,
                aws: s.aws?,
                stw: s.stw? * speed_factor,
                steady: s.sd_heading.is_some_and(|d| d <= 5.0) && s.sd_stw.is_none_or(|d| d <= 0.3) && s.sog > 1.0,
            })
        })
        .collect();
    v.sort_by_key(|r| r.t);
    v
}

/// The masthead offset: half the difference of the close-hauled apparent angles on the two tacks, compared
/// within bands of wind speed and weighted by the evidence in each.
fn offset(raws: &[Raw]) -> (Option<f64>, f64) {
    let mut bands: std::collections::BTreeMap<i64, (Vec<f64>, Vec<f64>)> = Default::default();
    let mut hours = 0.0;
    for r in raws.iter().filter(|r| r.steady) {
        let (twa, tws) = true_wind(r.awa, r.aws, r.stw);
        if twa.abs() >= 70.0 || tws < 4.0 {
            continue;
        }
        hours += 10.0 / 3600.0;
        let b = bands.entry((tws / 4.0).floor() as i64).or_default();
        if r.awa > 0.0 { b.0.push(r.awa) } else { b.1.push(-r.awa) }
    }
    let (mut sum, mut weight) = (0.0, 0.0);
    for (stbd, port) in bands.values() {
        if stbd.len() < 30 || port.len() < 30 {
            continue;
        }
        let w = stbd.len().min(port.len()) as f64;
        sum += w * (median(stbd).unwrap_or(0.0) - median(port).unwrap_or(0.0)) / 2.0;
        weight += w;
    }
    ((weight > 0.0).then(|| sum / weight), hours)
}

/// Steady sailing before and after each tack or gybe, with like winds.
struct Manoeuvre {
    before: Vec<Raw>,
    after: Vec<Raw>,
    upwind: bool,
}

const SIDE: Duration = Duration::minutes(15);
const TURN: Duration = Duration::minutes(3);

fn manoeuvres(raws: &[Raw], offset: f64) -> Vec<Manoeuvre> {
    let steady: Vec<&Raw> = raws.iter().filter(|r| r.steady).collect();
    let side = |r: &Raw| wrap180(r.awa - offset).signum();
    let mut out = Vec::new();
    for w in 1..steady.len() {
        let (a, b) = (steady[w - 1], steady[w]);
        if side(a) == side(b) || b.t - a.t > TURN {
            continue;
        }
        let before: Vec<Raw> = steady[..w].iter().rev().take_while(|r| a.t - r.t <= SIDE && side(r) == side(a)).map(|r| **r).collect();
        let after: Vec<Raw> = steady[w..].iter().take_while(|r| r.t - b.t <= SIDE && side(r) == side(b)).map(|r| **r).collect();
        if before.len() < 6 || after.len() < 6 {
            continue;
        }
        let mean_tw = |set: &[Raw]| {
            let n = set.len() as f64;
            set.iter().map(|r| true_wind(r.awa, r.aws, r.stw)).fold((0.0, 0.0), |(a, s), (twa, tws)| (a + twa.abs() / n, s + tws / n))
        };
        let ((a1, s1), (a2, s2)) = (mean_tw(&before), mean_tw(&after));
        if (s1 - s2).abs() > 0.25 * (s1 + s2) / 2.0 || s1 < 4.0 {
            continue;
        }
        let upwind = match (a1 < 90.0, a2 < 90.0) {
            (true, true) => true,
            (false, false) => false,
            _ => continue,
        };
        out.push(Manoeuvre { before, after, upwind });
    }
    out
}

/// The angle error that best removes the jumps in the true wind direction across the manoeuvres.
fn angle_error(ms: &[&Manoeuvre], offset: f64) -> Option<f64> {
    if ms.len() < 3 {
        return None;
    }
    let cost = |e: f64| -> f64 {
        let twd = |set: &[Raw]| {
            let mut c = Circular::default();
            for r in set {
                let a = correct(r.awa, r.aws, r.stw, offset, [e, e]);
                let (twa, _) = true_wind(a, r.aws, r.stw);
                c.add(r.heading + twa);
            }
            c.mean().unwrap_or(0.0)
        };
        ms.iter().map(|m| angle_diff(twd(&m.before), twd(&m.after)).powi(2)).sum()
    };
    let mut best = (0.0, f64::MAX);
    let mut e = -15.0;
    while e <= 15.0 {
        let c = cost(e);
        if c < best.1 {
            best = (e, c);
        }
        e += 0.5;
    }
    let (lo, hi) = (best.0 - 0.5, best.0 + 0.5);
    let mut e = lo;
    while e <= hi {
        let c = cost(e);
        if c < best.1 {
            best = (e, c);
        }
        e += 0.05;
    }
    Some(best.0)
}

/// The speedo factor: per hour with headings spread over at least 60 degrees, the least squares fit of
/// `sog⃗ = f · stw · ĥ + current⃗`; the median over the hours.
fn speed_factor(samples: &[Sample]) -> (Option<f64>, usize) {
    let mut hours: std::collections::BTreeMap<i64, Vec<&Sample>> = Default::default();
    for s in samples {
        if s.heading.is_some() && s.stw.is_some_and(|v| v > 1.0) && s.sd_heading.is_some_and(|d| d <= 5.0) && s.sd_stw.is_none_or(|d| d <= 0.3) {
            hours.entry(s.t.timestamp().div_euclid(3600)).or_default().push(s);
        }
    }
    let mut fs = Vec::new();
    for set in hours.values().filter(|v| v.len() >= 60) {
        let mut spread = Circular::default();
        for s in set {
            spread.add(s.heading.unwrap_or(0.0));
        }
        // Headings 60 degrees apart, evenly held, leave a mean resultant of cos 30°.
        if spread.sd().is_none_or(|sd| sd < 25.0) {
            continue;
        }
        // Normal equations of [stw·sin h, 1, 0; stw·cos h, 0, 1] · [f, cu, cv] = [sog·sin cog; sog·cos cog].
        let mut m = [[0.0f64; 3]; 3];
        let mut b = [0.0f64; 3];
        for s in set {
            let (h, c) = (s.heading.unwrap_or(0.0).to_radians(), s.cog.to_radians());
            let stw = s.stw.unwrap_or(0.0);
            for (row, y) in [([stw * h.sin(), 1.0, 0.0], s.sog * c.sin()), ([stw * h.cos(), 0.0, 1.0], s.sog * c.cos())] {
                for i in 0..3 {
                    for j in 0..3 {
                        m[i][j] += row[i] * row[j];
                    }
                    b[i] += row[i] * y;
                }
            }
        }
        if let Some(x) = solve3(m, b)
            && (0.5..1.5).contains(&x[0])
        {
            fs.push(x[0]);
        }
    }
    let n = fs.len();
    ((n >= 3).then(|| median(&fs)).flatten(), n)
}

/// Solves a 3×3 system by Cramer's rule.
fn solve3(m: [[f64; 3]; 3], b: [f64; 3]) -> Option<[f64; 3]> {
    let det = |a: [[f64; 3]; 3]| a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1]) - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0]) + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0]);
    let d = det(m);
    if d.abs() < 1e-9 {
        return None;
    }
    let mut x = [0.0; 3];
    for (k, xk) in x.iter_mut().enumerate() {
        let mut a = m;
        for i in 0..3 {
            a[i][k] = b[i];
        }
        *xk = det(a) / d;
    }
    Some(x)
}

/// Everything the log says about the instruments.
pub fn estimate(samples: &[Sample], now: DateTime<Utc>) -> Calibration {
    let (speed_factor, speed_hours) = speed_factor(samples);
    let raws = raws(samples, speed_factor.unwrap_or(1.0));
    let (awa_offset, upwind_hours) = offset(&raws);
    let o = awa_offset.unwrap_or(0.0);
    let ms = manoeuvres(&raws, o);
    let up: Vec<&Manoeuvre> = ms.iter().filter(|m| m.upwind).collect();
    let down: Vec<&Manoeuvre> = ms.iter().filter(|m| !m.upwind).collect();
    Calibration {
        awa_offset,
        angle_error: [angle_error(&up, o), angle_error(&down, o)],
        speed_factor,
        tacks: up.len(),
        gybes: down.len(),
        speed_hours,
        upwind_hours,
        computed_at: Some(now.timestamp()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::synth::{self, Config};
    use super::*;

    #[test]
    fn finds_the_faults_put_in() {
        let c = Config { hours: 200.0, awa_offset: 3.0, angle_error: [4.0, -3.0], speedo_factor: 0.93, current_kn: 1.5, ..Default::default() };
        let log = synth::log(&c);
        let cal = estimate(&log, c.start);
        let o = cal.awa_offset.unwrap();
        assert!((o - 3.0).abs() < 1.0, "offset {o}");
        let up = cal.angle_error[0].unwrap();
        assert!((up - 4.0).abs() < 1.5, "upwind error {up} over {} tacks", cal.tacks);
        let down = cal.angle_error[1].unwrap();
        assert!((down + 3.0).abs() < 1.5, "downwind error {down} over {} gybes", cal.gybes);
        // The speedo reads 93% of the truth: its reading must be multiplied by 1/0.93.
        let f = cal.speed_factor.unwrap();
        assert!((f * 0.93 - 1.0).abs() < 0.02, "speed factor {f} over {} hours", cal.speed_hours);
        // Corrected, a reading gives the true wind the boat sailed in.
        let clean = synth::log(&Config { awa_offset: 0.0, angle_error: [0.0, 0.0], speedo_factor: 1.0, ..c.clone() });
        let mut worst: f64 = 0.0;
        for (s, truth) in log.iter().zip(&clean).filter(|(s, _)| s.sd_heading.unwrap() < 5.0 && s.awa.is_some()).take(2000) {
            let (twa, tws) = cal.true_wind(s.awa.unwrap(), s.aws.unwrap(), s.stw.unwrap());
            worst = worst.max(angle_diff(twa, truth.twa().unwrap()).abs());
            assert!((tws - truth.tws.unwrap()).abs() < 1.0, "{tws} {:?}", truth.tws);
        }
        assert!(worst < 3.0, "{worst}");
    }

    #[test]
    fn a_slow_speedo_does_not_pass_for_an_angle_error() {
        // A speedo reading low makes the true wind angle come out wrong too: found first, it is taken out
        // before the angles are measured.
        let c = Config { hours: 200.0, awa_offset: 3.0, angle_error: [3.5, -2.0], speedo_factor: 0.95, current_kn: 0.8, motoring_share: 0.2, ..Default::default() };
        let cal = estimate(&synth::log(&c), c.start);
        let (up, down) = (cal.angle_error[0].unwrap(), cal.angle_error[1].unwrap());
        assert!((up - 3.5).abs() < 1.0 && (down + 2.0).abs() < 1.5, "{up} {down}");
        assert!((cal.awa_offset.unwrap() - 3.0).abs() < 1.0);
    }

    #[test]
    fn good_instruments_need_nothing_and_no_data_says_nothing() {
        let c = Config { hours: 120.0, ..Default::default() };
        let cal = estimate(&synth::log(&c), c.start);
        assert!(cal.awa_offset.unwrap().abs() < 1.0, "{:?}", cal.awa_offset);
        assert!(cal.angle_error[0].unwrap().abs() < 1.5, "{:?}", cal.angle_error);
        let cal = estimate(&synth::log(&Config { hours: 50.0, instruments: false, ..Default::default() }), c.start);
        assert!(cal.is_empty());
    }
}
