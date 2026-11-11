//! The whole analysis of a log, as one report.

use super::calibrate::{self, Calibration};
use super::factors::{self, Factors, Sigma, default_sigma};
use super::filter::{self, Point, Tier};
use super::forecast_check::{self, Lead, LeadStats};
use super::learn::{self, Learned};
use super::sample::{Hint, SailMode, Sample};
use super::waves::{self, WaveBand};
use crate::routing::polar::Polar;
use chrono::{DateTime, Utc};
use serde::Serialize;

/// Hours of the log, by what they were.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Hours {
    pub total: f64,
    pub sailing: f64,
    pub motoring: f64,
    /// Neither set: counted as sailing unless it looked motored.
    pub unset: f64,
    pub looked_motored: f64,
    /// Steady sailing usable for each tier.
    pub tier_a: f64,
    pub tier_c: f64,
    /// Steady sailing with a model's wind and no wind instruments.
    pub tier_d: f64,
    /// Steady sailing with a model's wind, instruments or not.
    pub model: f64,
}

/// What to suggest for weather routing's performance.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Suggestion {
    pub pct: f64,
    /// "instruments" or "forecast".
    pub from: &'static str,
}

/// Hours of evidence each kind of suggestion needs.
const SUGGEST_INSTRUMENT_H: f64 = 3.0;
const SUGGEST_MODEL_H: f64 = 10.0;

#[derive(Clone, Debug)]
pub struct Report {
    pub samples: usize,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub hours: Hours,
    /// What the log says about the instruments, from the raw readings.
    pub estimate: Calibration,
    /// The calibration the analysis applied (the saved one).
    pub applied: Option<Calibration>,
    pub learned: Option<Learned>,
    /// The polar compared as it is with the instruments' wind, and blurred with the model's.
    pub instruments: Factors,
    pub forecast: Factors,
    pub forecast_check: Vec<LeadStats>,
    pub waves: Vec<WaveBand>,
    pub suggestion: Option<Suggestion>,
}

fn hours_of(n: usize) -> f64 {
    n as f64 * 10.0 / 3600.0
}

/// Analyses `samples` (in time order) against the boat's `polar`, applying the saved calibration `cal`.
pub fn analyse(samples: &[Sample], polar: &Polar, cal: Option<&Calibration>, now: DateTime<Utc>) -> Report {
    let mut hours = Hours::default();
    for s in samples {
        let h = s.secs / 3600.0;
        hours.total += h;
        match s.mode {
            SailMode::Sailing => hours.sailing += h,
            SailMode::Motoring => hours.motoring += h,
            SailMode::Unset => hours.unset += h,
        }
        if s.mode != SailMode::Motoring && s.hint == Hint::LooksMotored {
            hours.looked_motored += h;
        }
    }
    let cal = cal.filter(|c| !c.is_empty());
    let inst = filter::instrument_points(samples, cal);
    let model = filter::model_points(samples, cal);
    hours.tier_a = hours_of(inst.iter().filter(|p| p.tier == Tier::A).count());
    hours.tier_c = hours_of(inst.iter().filter(|p| p.tier == Tier::C).count());
    hours.tier_d = hours_of(model.iter().filter(|p| !filter::has_instruments(&samples[p.index])).count());
    hours.model = hours_of(model.len());

    let check = forecast_check::check(samples, cal);
    let measured = |lead: Lead| check.iter().find(|s| s.lead == lead).and_then(LeadStats::sigma);
    let sigma = |p: &Point| -> Sigma { measured(Lead::of(p.hindcast, p.lead_h)).unwrap_or_else(|| default_sigma(p.hindcast, p.lead_h)) };
    let learned = learn::learn(&inst, polar);
    let instruments = factors::fit(&inst, polar, |_| Sigma::NONE);
    let forecast = factors::fit(&model, polar, sigma);
    let suggestion = match (instruments.overall, forecast.overall) {
        (Some(f), _) if f.hours >= SUGGEST_INSTRUMENT_H => Some(Suggestion { pct: (f.k * 100.0).round(), from: "instruments" }),
        (_, Some(f)) if f.hours >= SUGGEST_MODEL_H => Some(Suggestion { pct: (f.k * 100.0).round(), from: "forecast" }),
        _ => None,
    };
    Report {
        samples: samples.len(),
        from: samples.first().map(|s| s.t),
        to: samples.last().map(|s| s.t),
        hours,
        estimate: calibrate::estimate(samples, now),
        applied: cal.cloned(),
        waves: waves::bands(&inst, learned.as_ref().map_or(polar, |l| &l.polar)),
        learned,
        instruments,
        forecast,
        forecast_check: check,
        suggestion,
    }
}

#[cfg(test)]
mod tests {
    use super::super::synth::{self, Config, scaled};
    use super::*;

    #[test]
    fn a_season_with_instruments() {
        let reference = Polar::generic_cruiser();
        let c = Config { hours: 150.0, truth: scaled(&reference, 0.9, 0.95), awa_offset: 2.0, speedo_factor: 0.95, motoring_share: 0.2, ..Default::default() };
        let log = synth::log(&c);
        let r = analyse(&log, &reference, None, c.start);
        assert!((r.hours.total - 150.0).abs() < 0.01);
        assert!(r.hours.motoring > 10.0 && r.hours.tier_a > 50.0 && r.hours.tier_d == 0.0, "{:?}", r.hours);
        assert!(r.learned.is_some() && r.estimate.awa_offset.is_some());
        let s = r.suggestion.unwrap();
        assert_eq!(s.from, "instruments");
        // Uncalibrated, the speedo reads 5% low: the boat looks slower than it is.
        assert!((80.0..95.0).contains(&s.pct), "{s:?}");
        // Applying the calibration the log suggests brings it back.
        let r2 = analyse(&log, &reference, Some(&r.estimate), c.start);
        let s2 = r2.suggestion.unwrap();
        assert!(s2.pct > s.pct, "{s:?} {s2:?}");
        assert!(!r.forecast_check.is_empty());
    }

    #[test]
    fn a_gps_alone_gives_factors_and_no_polar() {
        let reference = Polar::generic_cruiser();
        let c = Config { hours: 150.0, instruments: false, truth: scaled(&reference, 0.85, 0.85), ..Default::default() };
        let r = analyse(&synth::log(&c), &reference, None, c.start);
        assert!(r.learned.is_none() && r.estimate.is_empty());
        assert!(r.hours.tier_d > 50.0);
        let s = r.suggestion.unwrap();
        assert_eq!(s.from, "forecast");
        assert!((78.0..92.0).contains(&s.pct), "{s:?}");
        assert!(r.forecast_check.is_empty());
    }
}
