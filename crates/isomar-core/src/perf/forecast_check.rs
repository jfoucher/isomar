//! How good the model's wind was against the instruments: bias and spread of its speed and direction,
//! by how old the forecast was. Shown to the user, and the measure of σ for the factors of samples that
//! have only a model's wind.

use super::calibrate::Calibration;
use super::factors::Sigma;
use super::filter::{instrument_wind, Tier};
use super::sample::Sample;
use super::stats::{angle_diff, wrap180};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Lead {
    Hindcast,
    Day1,
    Day2,
    Older,
}

impl Lead {
    pub fn of(hindcast: bool, lead_h: Option<f64>) -> Lead {
        match (hindcast, lead_h) {
            (true, _) => Lead::Hindcast,
            (false, Some(l)) if l <= 24.0 => Lead::Day1,
            (false, Some(l)) if l <= 48.0 => Lead::Day2,
            _ => Lead::Older,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LeadStats {
    pub lead: Lead,
    pub hours: f64,
    /// Model minus instruments, knots.
    pub tws_bias: f64,
    pub tws_rms: f64,
    /// The rms over the instruments' mean wind.
    pub tws_rel_rms: f64,
    /// Degrees.
    pub twd_bias: f64,
    pub twd_rms: f64,
}

impl LeadStats {
    /// The errors to assume for this lead, when there is enough to go on.
    pub fn sigma(&self) -> Option<Sigma> {
        (self.hours >= 10.0).then_some(Sigma { angle: self.twd_rms, speed: self.tws_rel_rms })
    }
}

/// Per sample: the model's speed minus the instruments', its direction minus theirs, and their speed.
type Errors = Vec<(f64, f64, f64)>;

pub fn check(samples: &[Sample], cal: Option<&Calibration>) -> Vec<LeadStats> {
    let mut acc: Vec<(Lead, Errors)> = [Lead::Hindcast, Lead::Day1, Lead::Day2, Lead::Older].into_iter().map(|l| (l, Vec::new())).collect();
    for s in samples.iter().filter(|s| s.sailing() && s.sd_heading.is_some_and(|d| d <= 5.0)) {
        let Some((twa, tws, _, Tier::A)) = instrument_wind(s, cal) else { continue };
        let Some(heading) = s.heading else { continue };
        let twd = wrap180(heading + twa);
        for (m, hindcast) in [(s.hindcast, true), (s.forecast, false)] {
            let Some(m) = m else { continue };
            let lead = Lead::of(hindcast, m.lead_h);
            if let Some(a) = acc.iter_mut().find(|a| a.0 == lead) {
                a.1.push((m.tws - tws, angle_diff(m.twd, twd), tws));
            }
        }
    }
    acc.into_iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(lead, v)| {
            let n = v.len() as f64;
            let mean = |f: &dyn Fn(&(f64, f64, f64)) -> f64| v.iter().map(f).sum::<f64>() / n;
            let tws_rms = mean(&|e| e.0 * e.0).sqrt();
            LeadStats {
                lead,
                hours: n * 10.0 / 3600.0,
                tws_bias: mean(&|e| e.0),
                tws_rms,
                tws_rel_rms: tws_rms / mean(&|e| e.2).max(1.0),
                twd_bias: mean(&|e| e.1),
                twd_rms: mean(&|e| e.1 * e.1).sqrt(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::synth::{self, Config};
    use super::*;

    #[test]
    fn measures_the_forecasts_errors() {
        let c = Config { hours: 150.0, forecast_sd: (20.0, 0.25), ..Default::default() };
        let stats = check(&synth::log(&c), None);
        let all: f64 = stats.iter().map(|s| s.hours).sum();
        assert!(all > 80.0, "{all}");
        for s in stats.iter().filter(|s| s.hours > 20.0) {
            assert!((s.twd_rms - 20.0).abs() < 7.0, "{s:?}");
            assert!((s.tws_rel_rms - 0.25).abs() < 0.1, "{s:?}");
            assert!(s.sigma().is_some());
        }
    }
}
