//! One window of the performance log.

use super::stats::wrap180;
use crate::comm::state::WindRef;
use crate::geo::LatLon;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Whether the boat is sailing, as the switch says.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SailMode {
    /// Nobody said: the log keeps it, and the analysis uses it unless it looks motored.
    #[default]
    Unset,
    Sailing,
    Motoring,
}

impl SailMode {
    pub fn code(self) -> i64 {
        match self {
            SailMode::Unset => 0,
            SailMode::Sailing => 1,
            SailMode::Motoring => 2,
        }
    }

    pub fn from_code(c: i64) -> SailMode {
        match c {
            1 => SailMode::Sailing,
            2 => SailMode::Motoring,
            _ => SailMode::Unset,
        }
    }

    /// As the settings and the api spell it.
    pub fn as_str(self) -> &'static str {
        match self {
            SailMode::Unset => "",
            SailMode::Sailing => "sailing",
            SailMode::Motoring => "motoring",
        }
    }

    pub fn parse(s: &str) -> SailMode {
        match s {
            "sailing" => SailMode::Sailing,
            "motoring" => SailMode::Motoring,
            _ => SailMode::Unset,
        }
    }
}

/// Who set the mode: the user's switch, or an engine reporting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeSource {
    #[default]
    User,
    Engine,
}

/// What the data suggests when no engine reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Hint {
    #[default]
    None,
    LooksMotored,
}

/// A weather model's wind and waves at a sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelWind {
    pub tws: f64,
    /// Where the wind blows from, 0..360.
    pub twd: f64,
    pub gust: Option<f64>,
    /// Significant wave height, metres; where the waves come from; their period, seconds.
    pub hs: Option<f64>,
    pub wave_dir: Option<f64>,
    pub wave_period: Option<f64>,
    /// The model run, and how long after it the sample is.
    pub run: Option<DateTime<Utc>>,
    pub lead_h: Option<f64>,
    /// The forecast current there and then, (east, north) knots: only from a source that resolves tidal
    /// races (`CurrentSource::resolves_races`), for tier C's correction.
    pub current: Option<(f64, f64)>,
}

/// The instruments averaged over a window of about 10 s.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    /// Its row in the database (0 before it is stored).
    pub id: i64,
    /// The middle of the window.
    pub t: DateTime<Utc>,
    pub pos: LatLon,
    /// How long the window was, seconds.
    pub secs: f64,
    pub sog: f64,
    pub cog: f64,
    pub heading: Option<f64>,
    pub stw: Option<f64>,
    /// Apparent wind, angle off the bow (-180..180, starboard positive) and speed, as read.
    pub awa: Option<f64>,
    pub aws: Option<f64>,
    /// True wind as the instruments or `OwnShip` gave it: where from, how strong, against what.
    pub twd: Option<f64>,
    pub tws: Option<f64>,
    pub wind_ref: Option<WindRef>,
    pub heel: Option<f64>,
    pub rpm: Option<f64>,
    /// How steady the window was: standard deviations of the heading (else the course), the speed
    /// through the water and the apparent wind angle.
    pub sd_heading: Option<f64>,
    pub sd_stw: Option<f64>,
    pub sd_awa: Option<f64>,
    /// How many readings were averaged.
    pub n: u32,
    pub mode: SailMode,
    pub mode_from: ModeSource,
    pub hint: Hint,
    /// The loaded forecast at the sample when it was written.
    pub forecast: Option<ModelWind>,
    /// A later run's analysis hours, fetched afterwards.
    pub hindcast: Option<ModelWind>,
    /// The polar that was active when it was logged (`routing::library`): only that one learns from it.
    pub polar: String,
}

impl Sample {
    /// The true wind angle off the bow from the instruments' true wind, -180..180.
    pub fn twa(&self) -> Option<f64> {
        Some(wrap180(self.twd? - self.heading?))
    }

    /// The stream the instruments measure: the way over the ground less the way through the water (the
    /// log along the compass heading), (east, north) knots. Leeway counts in it too, so a window says
    /// little alone; averaged over minutes it is the set and drift.
    pub fn stream(&self) -> Option<(f64, f64)> {
        let (h, stw) = (self.heading?, self.stw?);
        let (g, w) = (self.cog.to_radians(), h.to_radians());
        Some((self.sog * g.sin() - stw * w.sin(), self.sog * g.cos() - stw * w.cos()))
    }

    /// The boat's speed: through the water when there is a speedo, else over the ground.
    pub fn speed(&self) -> f64 {
        self.stw.unwrap_or(self.sog)
    }

    /// Where the bow points, or failing a compass, the course over ground.
    pub fn bow(&self) -> f64 {
        self.heading.unwrap_or(self.cog)
    }

    /// Whether the window counts as sailing.
    pub fn sailing(&self) -> bool {
        match self.mode {
            SailMode::Sailing => true,
            SailMode::Motoring => false,
            SailMode::Unset => self.hint == Hint::None,
        }
    }

    /// The model wind the analysis uses: the hindcast, else a forecast made at most `max_lead_h` before.
    pub fn model(&self, max_lead_h: f64) -> Option<(ModelWind, bool)> {
        if let Some(h) = self.hindcast {
            return Some((h, true));
        }
        self.forecast.filter(|f| f.lead_h.is_none_or(|l| l <= max_lead_h)).map(|f| (f, false))
    }
}

impl Default for Sample {
    fn default() -> Self {
        Sample {
            id: 0,
            t: DateTime::<Utc>::UNIX_EPOCH,
            pos: LatLon::new(0.0, 0.0),
            secs: 10.0,
            sog: 0.0,
            cog: 0.0,
            heading: None,
            stw: None,
            awa: None,
            aws: None,
            twd: None,
            tws: None,
            wind_ref: None,
            heel: None,
            rpm: None,
            sd_heading: None,
            sd_stw: None,
            sd_awa: None,
            n: 0,
            mode: SailMode::Unset,
            mode_from: ModeSource::User,
            hint: Hint::None,
            forecast: None,
            hindcast: None,
            polar: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_round_trip_and_sailing_follows_the_hint() {
        for m in [SailMode::Unset, SailMode::Sailing, SailMode::Motoring] {
            assert_eq!(SailMode::from_code(m.code()), m);
            assert_eq!(SailMode::parse(m.as_str()), m);
        }
        let mut s = Sample { heading: Some(350.0), twd: Some(30.0), ..Default::default() };
        assert_eq!(s.twa(), Some(40.0));
        assert!(s.sailing());
        s.hint = Hint::LooksMotored;
        assert!(!s.sailing());
        s.mode = SailMode::Sailing;
        assert!(s.sailing());
    }
}
