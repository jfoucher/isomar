//! How fast the boat sails against its polar, as factors: overall and upwind, reaching and running.
//!
//! With the instruments' wind the polar is compared as it is. With a model's wind it is first blurred
//! by the model's errors (the expected speed over angle errors of σa degrees and speed errors of σs, by
//! Gauss–Hermite quadrature), because the observed speeds are those of the real wind seen through the
//! model's: compared with the sharp polar, the factor would come out wrong where the polar curves. The
//! factor is the weighted median of observed over expected, with an interval from a bootstrap over
//! 6-hour blocks (errors persist for hours, so samples are not independent).

use super::filter::Point;
use super::rng::Rng;
use super::stats::{quantile, weighted_median};
use crate::routing::polar::Polar;
use serde::Serialize;
use std::collections::BTreeMap;

/// A model's wind errors.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Sigma {
    /// Degrees.
    pub angle: f64,
    /// A fraction of the wind speed.
    pub speed: f64,
}

impl Sigma {
    pub const NONE: Sigma = Sigma { angle: 0.0, speed: 0.0 };
}

/// What is assumed of a model's errors when the log cannot tell: a hindcast, a forecast up to a day old,
/// up to two.
pub fn default_sigma(hindcast: bool, lead_h: Option<f64>) -> Sigma {
    match (hindcast, lead_h) {
        (true, _) => Sigma { angle: 18.0, speed: 0.20 },
        (false, Some(l)) if l <= 24.0 => Sigma { angle: 25.0, speed: 0.30 },
        _ => Sigma { angle: 30.0, speed: 0.35 },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Factor {
    pub k: f64,
    /// The 5th and 95th percentiles of the bootstrap.
    pub lo: f64,
    pub hi: f64,
    pub hours: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Factors {
    pub overall: Option<Factor>,
    pub upwind: Option<Factor>,
    pub reach: Option<Factor>,
    pub run: Option<Factor>,
}

/// Probabilists' Gauss–Hermite nodes and weights for five points (a standard normal).
const GH: [(f64, f64); 5] = [(-2.856_970_013_872_8, 0.011_257_411_327_7), (-1.355_626_179_974_3, 0.222_075_922_005_6), (0.0, 0.533_333_333_333_3), (1.355_626_179_974_3, 0.222_075_922_005_6), (2.856_970_013_872_8, 0.011_257_411_327_7)];

/// The speed the polar gives on average when the wind is only known to within `sigma`.
pub fn expected(polar: &Polar, twa: f64, tws: f64, sigma: Sigma) -> f64 {
    if sigma.angle == 0.0 && sigma.speed == 0.0 {
        return polar.speed(twa, tws);
    }
    let mut s = 0.0;
    for (xa, wa) in GH {
        for (xs, ws) in GH {
            s += wa * ws * polar.speed(twa + sigma.angle * xa, (tws * (1.0 + sigma.speed * xs)).max(0.0));
        }
    }
    s
}

/// Hours of evidence a factor needs.
const MIN_HOURS: f64 = 1.0;
const BOOT: usize = 200;
const BLOCK_S: i64 = 6 * 3600;

fn one(pts: &[(&Point, f64)], rng: &mut Rng) -> Option<Factor> {
    let hours = pts.iter().map(|(p, _)| p.weight * 10.0 / 3600.0).sum::<f64>();
    if hours < MIN_HOURS {
        return None;
    }
    let pairs = |set: &mut dyn Iterator<Item = &(&Point, f64)>| -> Vec<(f64, f64)> { set.filter(|(_, e)| *e > 0.3).map(|(p, e)| (p.speed / e, e * p.weight)).collect() };
    let k = weighted_median(&pairs(&mut pts.iter()))?;
    let mut blocks: BTreeMap<i64, Vec<(&Point, f64)>> = BTreeMap::new();
    for (p, e) in pts {
        blocks.entry(p.t.timestamp().div_euclid(BLOCK_S)).or_default().push((p, *e));
    }
    let blocks: Vec<_> = blocks.into_values().collect();
    let mut ks = Vec::with_capacity(BOOT);
    for _ in 0..BOOT {
        let mut draw: Vec<(f64, f64)> = Vec::new();
        for _ in 0..blocks.len() {
            draw.extend(pairs(&mut blocks[rng.below(blocks.len())].iter()));
        }
        if let Some(k) = weighted_median(&draw) {
            ks.push(k);
        }
    }
    Some(Factor { k, lo: quantile(&ks, 0.05).unwrap_or(k), hi: quantile(&ks, 0.95).unwrap_or(k), hours })
}

/// Factors of `points` against `polar`, each point's expected speed blurred by `sigma` of it.
pub fn fit(points: &[Point], polar: &Polar, sigma: impl Fn(&Point) -> Sigma) -> Factors {
    let pts: Vec<(&Point, f64)> = points.iter().map(|p| (p, expected(polar, p.twa, p.tws, sigma(p)))).collect();
    let mut rng = Rng::new(0x5EED);
    let band = |lo: f64, hi: f64| -> Vec<(&Point, f64)> { pts.iter().filter(|(p, _)| (lo..hi).contains(&p.twa.abs())).copied().collect() };
    Factors {
        overall: one(&pts, &mut rng),
        upwind: one(&band(0.0, 70.0), &mut rng),
        reach: one(&band(70.0, 120.0), &mut rng),
        run: one(&band(120.0, 180.1), &mut rng),
    }
}

#[cfg(test)]
mod tests {
    use super::super::filter::{instrument_points, model_points};
    use super::super::synth::{self, Config, scaled};
    use super::*;

    #[test]
    fn blurring_flattens_the_polar() {
        let p = Polar::generic_cruiser();
        assert_eq!(expected(&p, 90.0, 12.0, Sigma::NONE), p.speed(90.0, 12.0));
        // Close to the wind the blurred polar is faster (some of the time the boat is really freer),
        // on a reach slower.
        assert!(expected(&p, 20.0, 12.0, Sigma { angle: 25.0, speed: 0.3 }) > p.speed(20.0, 12.0));
        assert!(expected(&p, 100.0, 12.0, Sigma { angle: 25.0, speed: 0.3 }) < p.speed(100.0, 12.0));
    }

    #[test]
    fn a_forecast_finds_how_fast_the_boat_is() {
        let reference = Polar::generic_cruiser();
        let c = Config { hours: 300.0, instruments: false, truth: scaled(&reference, 0.85, 0.85), forecast_sd: (25.0, 0.3), ..Default::default() };
        let pts = model_points(&synth::log(&c), None);
        let f = fit(&pts, &reference, |_| Sigma { angle: 25.0, speed: 0.3 });
        let k = f.overall.unwrap();
        assert!((k.k - 0.85).abs() < 0.07, "{k:?}");
        assert!(k.lo <= k.k && k.k <= k.hi && k.hi - k.lo < 0.2, "{k:?}");
    }

    #[test]
    fn the_instruments_find_each_band() {
        let reference = Polar::generic_cruiser();
        let c = Config { hours: 200.0, truth: scaled(&reference, 0.8, 1.05), speed_noise: 0.02, ..Default::default() };
        let f = fit(&instrument_points(&synth::log(&c), None), &reference, |_| Sigma::NONE);
        let (up, run) = (f.upwind.unwrap(), f.run.unwrap());
        assert!((up.k - 0.8).abs() < 0.04, "{up:?}");
        assert!((run.k - 1.05).abs() < 0.04, "{run:?}");
    }
}
