//! What the session runs every tick: the engines' verdict on the switch, the 10 s windows, the hint.

use super::aggregator::Aggregator;
use super::engine::EngineMode;
use super::hint::Hinter;
use super::sample::{Hint, ModeSource, SailMode, Sample};
use crate::comm::state::OwnShip;
use crate::routing::polar::Polar;
use chrono::{DateTime, Utc};

pub struct PerfLog {
    /// Whether windows are kept.
    pub recording: bool,
    /// The Sailing / Motoring switch: what the user set, or the engines while they report.
    pub mode: SailMode,
    pub mode_from: ModeSource,
    /// What the data suggests when no engine reports.
    pub hint: Hint,
    /// The boat's polar, for the hint.
    pub polar: Polar,
    /// Its id in the library: every window is logged against it.
    pub polar_id: String,
    agg: Aggregator,
    engine: EngineMode,
    hinter: Hinter,
}

/// What a tick produced.
#[derive(Debug, Default, PartialEq)]
pub struct Tick {
    /// A window to keep.
    pub sample: Option<Sample>,
    /// The engines changed the switch to this.
    pub engine_set: Option<SailMode>,
    /// The hint has just turned to motoring: the windows since this time look motored too.
    pub hint_since: Option<DateTime<Utc>>,
}

impl Default for PerfLog {
    fn default() -> Self {
        PerfLog {
            recording: true,
            mode: SailMode::Unset,
            mode_from: ModeSource::User,
            hint: Hint::None,
            polar: Polar::generic_cruiser(),
            polar_id: String::new(),
            agg: Aggregator::default(),
            engine: EngineMode::default(),
            hinter: Hinter::default(),
        }
    }
}

impl PerfLog {
    /// The user flips the switch. Ignored while an engine reports: it decides.
    pub fn set_mode(&mut self, m: SailMode) -> bool {
        if self.mode_from == ModeSource::Engine {
            return false;
        }
        self.mode = m;
        self.hint = Hint::None;
        self.hinter.reset();
        true
    }

    pub fn tick(&mut self, own: &OwnShip, anchored: bool, now: DateTime<Utc>) -> Tick {
        let mut out = Tick::default();
        match self.engine.update(own, now) {
            Some(m) => {
                if self.mode != m || self.mode_from != ModeSource::Engine {
                    out.engine_set = (self.mode != m).then_some(m);
                    self.hinter.reset();
                    self.hint = Hint::None;
                }
                self.mode = m;
                self.mode_from = ModeSource::Engine;
            }
            None => self.mode_from = ModeSource::User,
        }
        let Some(mut s) = self.agg.push(own, now) else { return out };
        if anchored {
            return out;
        }
        s.mode = self.mode;
        s.mode_from = self.mode_from;
        s.polar = self.polar_id.clone();
        if self.mode_from == ModeSource::User && self.mode != SailMode::Motoring {
            let was = self.hint;
            self.hint = self.hinter.push(&s, &self.polar);
            if was == Hint::None && self.hint == Hint::LooksMotored {
                out.hint_since = Some(s.t - super::hint::HOLD);
            }
            s.hint = self.hint;
        }
        if self.recording {
            out.sample = Some(s);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comm::nmea::{frame, parse_line};
    use chrono::{Duration, TimeZone};

    fn t(ms: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_780_000_000, 0).unwrap() + Duration::milliseconds(ms)
    }

    fn feed(o: &mut OwnShip, body: &str, at: DateTime<Utc>) {
        o.apply(&parse_line(&frame(body, '$')).unwrap().unwrap(), at);
    }

    fn run(log: &mut PerfLog, o: &mut OwnShip, from_s: i64, secs: i64, rpm: Option<f64>, wind_angle: f64) -> Vec<Tick> {
        (0..secs * 4)
            .map(|i| {
                let now = t(from_s * 1000 + i * 250);
                feed(o, "GPGLL,4807.038,N,00431.000,W,123519,A", now);
                feed(o, "GPVTG,000.0,T,,M,6.0,N,,K,A", now);
                feed(o, "HEHDT,000.0,T", now);
                feed(o, "IIVHW,,T,,M,6.0,N,,K", now);
                feed(o, &format!("IIMWV,{wind_angle:03},R,16.0,N,A"), now);
                if let Some(r) = rpm {
                    feed(o, &format!("ERRPM,E,1,{r},,A"), now);
                }
                log.tick(o, false, now)
            })
            .collect()
    }

    #[test]
    fn the_engine_flips_the_switch_and_the_user_cannot_override_it() {
        let (mut log, mut o) = (PerfLog::default(), OwnShip::default());
        log.set_mode(SailMode::Sailing);
        let ticks = run(&mut log, &mut o, 0, 30, Some(1500.0), 60.0);
        assert_eq!(ticks.iter().filter_map(|t| t.engine_set).collect::<Vec<_>>(), vec![SailMode::Motoring]);
        assert!(ticks.iter().filter_map(|t| t.sample.as_ref()).all(|s| s.mode == SailMode::Motoring && s.mode_from == ModeSource::Engine));
        assert!(!log.set_mode(SailMode::Sailing));
        assert_eq!(log.mode, SailMode::Motoring);
        // Engine off: sailing after a few seconds.
        let ticks = run(&mut log, &mut o, 30, 20, Some(0.0), 60.0);
        assert_eq!(ticks.iter().filter_map(|t| t.engine_set).collect::<Vec<_>>(), vec![SailMode::Sailing]);
        // No engine data any more: the switch is the user's again, and keeps the engine's last word.
        run(&mut log, &mut o, 50, 40, None, 60.0);
        assert_eq!((log.mode, log.mode_from), (SailMode::Sailing, ModeSource::User));
        assert!(log.set_mode(SailMode::Motoring));
    }

    #[test]
    fn motoring_head_to_wind_without_engine_data_is_hinted() {
        let (mut log, mut o) = (PerfLog::default(), OwnShip::default());
        log.set_mode(SailMode::Sailing);
        // Apparent wind 10 degrees off the bow at 6 kn: true wind about 16 degrees off, 10 kn.
        let ticks = run(&mut log, &mut o, 0, 400, None, 10.0);
        let since: Vec<_> = ticks.iter().filter_map(|t| t.hint_since).collect();
        assert_eq!(since.len(), 1);
        assert_eq!(log.hint, Hint::LooksMotored);
        assert!(ticks.last().unwrap().sample.is_none() || ticks.iter().rev().find_map(|t| t.sample.as_ref()).unwrap().hint == Hint::LooksMotored);
        // Anchored or not recording: nothing kept.
        log.recording = false;
        assert!(run(&mut log, &mut o, 400, 30, None, 60.0).iter().all(|t| t.sample.is_none()));
    }
}
