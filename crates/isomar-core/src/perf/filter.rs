//! Which samples are steady sailing, and what each one's wind is worth.
//!
//! - **Tier A**: wind instruments and a speedo. The true wind is worked out again from the raw apparent
//!   wind and speed through the water with the saved calibration (or taken as the instruments gave it
//!   through the water when the raw readings are missing).
//! - **Tier C**: wind instruments but no speedo: the true wind over the ground, and the speed over ground,
//!   so a current is in both. Weighted 0.3. Where a forecast current that resolves the tidal races was
//!   loaded when the window was logged (`ModelWind::current`), it is taken out of both: the boat's speed
//!   through the water is the ground's less the stream, and its wind the wind over the water.
//! - **Tier D**: no wind instrument: a model's wind (a hindcast, else a forecast made at most
//!   [`MAX_LEAD_H`] before), the speed over ground and the course. Only ever used for factors.

use super::calibrate::Calibration;
use super::sample::Sample;
use super::stats::wrap180;
use crate::comm::state::{WindRef, true_wind};
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

/// A forecast made longer ago than this is too far off to use.
pub const MAX_LEAD_H: f64 = 48.0;
/// How long after a tack or gybe the boat is not up to speed.
const SETTLE: Duration = Duration::seconds(60);
/// The weight of a tier C sample against a tier A one.
pub const TIER_C_WEIGHT: f64 = 0.3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Tier {
    A,
    C,
    D,
}

#[derive(Clone, Copy, Debug)]
pub struct Point {
    pub t: DateTime<Utc>,
    pub tier: Tier,
    /// True wind angle, -180..180, and speed.
    pub twa: f64,
    pub tws: f64,
    /// Through the water for tier A, over the ground otherwise.
    pub speed: f64,
    pub weight: f64,
    /// For model winds: a hindcast, or the forecast's lead in hours.
    pub hindcast: bool,
    pub lead_h: Option<f64>,
    /// The model's significant wave height there.
    pub hs: Option<f64>,
    /// Which sample it came from.
    pub index: usize,
}

/// A tier C wind and speed (angle off the bow, speed, both over the ground) with the forecast current
/// taken out, when the sample has one.
fn through_water(s: &Sample, heading: f64, twa: f64, tws: f64) -> (f64, f64, f64) {
    let Some(c) = s.forecast.and_then(|f| f.current) else { return (twa, tws, s.sog) };
    let (ge, gn) = (s.sog * s.cog.to_radians().sin(), s.sog * s.cog.to_radians().cos());
    let stw = (ge - c.0).hypot(gn - c.1);
    let (tws_w, twd_w) = crate::geo::wind_over_water(tws, heading + twa, c);
    (wrap180(twd_w - heading), tws_w, stw)
}

/// The sample's wind from its instruments, and the tier that makes it.
pub fn instrument_wind(s: &Sample, cal: Option<&Calibration>) -> Option<(f64, f64, f64, Tier)> {
    let none = Calibration::default();
    let cal = cal.unwrap_or(&none);
    let heading = s.heading?;
    match (s.awa, s.aws, s.stw) {
        (Some(awa), Some(aws), Some(stw)) => {
            let (twa, tws) = cal.true_wind(awa, aws, stw);
            Some((twa, tws, cal.correct_stw(stw), Tier::A))
        }
        (Some(awa), Some(aws), None) => {
            let a = cal.correct_awa(awa, aws, s.sog);
            let (twa, tws) = true_wind(a, aws, s.sog);
            let (twa, tws, speed) = through_water(s, heading, twa, tws);
            Some((twa, tws, speed, Tier::C))
        }
        _ => {
            let (twd, tws) = (s.twd?, s.tws?);
            let twa = wrap180(twd - heading);
            match s.wind_ref {
                Some(WindRef::Ground) => {
                    let (twa, tws, speed) = through_water(s, heading, twa, tws);
                    Some((twa, tws, speed, Tier::C))
                }
                _ => Some((twa, tws, s.stw.map(|v| cal.correct_stw(v)).unwrap_or(s.sog), if s.stw.is_some() { Tier::A } else { Tier::C })),
            }
        }
    }
}

/// The sample's wind from the model: angle off the bow (or the course), speed, and the boat's speed.
pub fn model_wind(s: &Sample, cal: Option<&Calibration>) -> Option<(f64, f64, f64, bool, Option<f64>)> {
    let (m, hindcast) = s.model(MAX_LEAD_H)?;
    let speed = match (s.stw, cal) {
        (Some(v), Some(c)) => c.correct_stw(v),
        (Some(v), None) => v,
        (None, _) => s.sog,
    };
    Some((wrap180(m.twd - s.bow()), m.tws, speed, hindcast, if hindcast { None } else { m.lead_h }))
}

/// Indices of the samples that are steady sailing: not motored, on a steady heading and speed, under
/// way, and not within a minute of a tack or gybe. `side` gives the tack each sample is on.
fn steady(samples: &[Sample], side: impl Fn(&Sample) -> Option<f64>) -> Vec<usize> {
    let mut out = Vec::new();
    let mut last_turn: Option<DateTime<Utc>> = None;
    let mut prev_side: Option<(DateTime<Utc>, f64)> = None;
    for (i, s) in samples.iter().enumerate() {
        let side = side(s);
        let turning = s.sd_heading.is_none_or(|d| d > 5.0);
        if turning || prev_side.is_some_and(|(t, p)| side.is_some_and(|q| q != p) && s.t - t <= SETTLE * 3) {
            last_turn = Some(s.t);
        }
        if let Some(q) = side {
            prev_side = Some((s.t, q));
        }
        let settled = last_turn.is_none_or(|t| s.t - t >= SETTLE);
        if s.sailing() && settled && !turning && s.sog > 1.0 && s.sd_stw.is_none_or(|d| d <= 0.3) {
            out.push(i);
        }
    }
    out
}

/// Tier A and C points: the instruments' wind.
pub fn instrument_points(samples: &[Sample], cal: Option<&Calibration>) -> Vec<Point> {
    steady(samples, |s| instrument_wind(s, cal).map(|w| w.0.signum()))
        .into_iter()
        .filter_map(|i| {
            let s = &samples[i];
            let (twa, tws, speed, tier) = instrument_wind(s, cal)?;
            (3.0..=45.0).contains(&tws).then(|| Point {
                t: s.t,
                tier,
                twa,
                tws,
                speed,
                weight: if tier == Tier::C { TIER_C_WEIGHT } else { 1.0 },
                hindcast: false,
                lead_h: None,
                hs: s.model(f64::MAX).and_then(|(m, _)| m.hs),
                index: i,
            })
        })
        .collect()
}

/// Model points: every steady sailing sample with a usable model wind, instruments or not. Tier D for
/// the samples without wind instruments; for the others the same winds let the forecast's factor be
/// compared with the instruments'.
pub fn model_points(samples: &[Sample], cal: Option<&Calibration>) -> Vec<Point> {
    steady(samples, |s| model_wind(s, cal).map(|w| w.0.signum()))
        .into_iter()
        .filter_map(|i| {
            let s = &samples[i];
            let (twa, tws, speed, hindcast, lead_h) = model_wind(s, cal)?;
            (3.0..=45.0).contains(&tws).then(|| Point {
                t: s.t,
                tier: Tier::D,
                twa,
                tws,
                speed,
                weight: 1.0,
                hindcast,
                lead_h,
                hs: s.model(f64::MAX).and_then(|(m, _)| m.hs),
                index: i,
            })
        })
        .collect()
}

/// Whether a sample has wind instruments.
pub fn has_instruments(s: &Sample) -> bool {
    s.heading.is_some() && (s.awa.is_some() || s.tws.is_some())
}

#[cfg(test)]
mod tests {
    use super::super::synth::{self, Config};
    use super::*;

    #[test]
    fn tier_c_takes_the_forecast_stream_out() {
        // 5 kn through the water heading east in a 2 kn stream setting east: 7 kn over the ground. The
        // wind over the water is 12 kn from the north; over the ground the stream adds to it.
        let (tws_g, twd_g) = crate::geo::wind_from_uv(2.0, -12.0);
        let mut s = Sample { heading: Some(90.0), cog: 90.0, sog: 7.0, twd: Some(twd_g), tws: Some(tws_g), wind_ref: Some(WindRef::Ground), ..Default::default() };
        let (twa, tws, speed, tier) = instrument_wind(&s, None).unwrap();
        assert_eq!(tier, Tier::C);
        assert!((speed - 7.0).abs() < 1e-9 && (tws - tws_g).abs() < 1e-9 && (twa + 90.0).abs() > 5.0, "no current known: over the ground");
        s.forecast = Some(crate::perf::ModelWind { tws: 12.0, twd: 0.0, gust: None, hs: None, wave_dir: None, wave_period: None, run: None, lead_h: None, current: Some((2.0, 0.0)) });
        let (twa, tws, speed, _) = instrument_wind(&s, None).unwrap();
        assert!((speed - 5.0).abs() < 1e-9, "{speed}");
        assert!((tws - 12.0).abs() < 1e-9 && (twa + 90.0).abs() < 1e-6, "{tws} {twa}");
    }

    #[test]
    fn steady_sailing_only() {
        let c = Config { hours: 30.0, motoring_share: 0.3, ..Default::default() };
        let log = synth::log(&c);
        let pts = instrument_points(&log, None);
        assert!(pts.len() > 3000, "{}", pts.len());
        assert!(pts.iter().all(|p| p.tier == Tier::A && log[p.index].mode.code() == 1));
        // No point within a minute after a turn.
        for p in &pts {
            let turned = log.iter().any(|s| s.sd_heading.unwrap() > 5.0 && s.t <= p.t && p.t - s.t < SETTLE);
            assert!(!turned, "{}", p.t);
        }
        // Without instruments: no instrument points, model points for the same sailing.
        let gps = synth::log(&Config { instruments: false, ..c });
        assert!(instrument_points(&gps, None).is_empty());
        let m = model_points(&gps, None);
        assert!(m.len() > 3000 && m.iter().all(|p| p.tier == Tier::D && p.lead_h.is_some_and(|l| l <= MAX_LEAD_H)));
    }
}
