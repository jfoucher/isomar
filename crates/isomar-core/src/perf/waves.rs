//! How much waves slow the boat: speed over the polar by significant wave height, upwind and downwind.
//! Shown only; the router does not take waves into account yet.

use super::filter::Point;
use super::stats::median;
use crate::routing::polar::Polar;
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WaveBand {
    /// The band of significant wave height, metres (`hi` is infinite for the last).
    pub lo: f64,
    pub hi: f64,
    pub upwind: bool,
    /// Median of speed over the polar's.
    pub ratio: f64,
    pub hours: f64,
}

const BANDS: [(f64, f64); 4] = [(0.0, 1.0), (1.0, 2.0), (2.0, 3.0), (3.0, f64::INFINITY)];

pub fn bands(points: &[Point], polar: &Polar) -> Vec<WaveBand> {
    let mut out = Vec::new();
    for upwind in [true, false] {
        for (lo, hi) in BANDS {
            let ratios: Vec<f64> = points
                .iter()
                .filter(|p| (p.twa.abs() < 90.0) == upwind && p.hs.is_some_and(|h| (lo..hi).contains(&h)))
                .filter_map(|p| {
                    let r = polar.speed(p.twa, p.tws);
                    (r > 0.5).then(|| p.speed / r)
                })
                .collect();
            let hours = ratios.len() as f64 * 10.0 / 3600.0;
            if hours >= 0.5
                && let Some(ratio) = median(&ratios)
            {
                out.push(WaveBand { lo, hi: if hi.is_finite() { hi } else { f64::MAX }, upwind, ratio, hours });
            }
        }
    }
    out
}
