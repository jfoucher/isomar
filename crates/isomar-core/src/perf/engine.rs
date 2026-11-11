//! The engine sets Sailing or Motoring: revolutions reported are a fact, not a guess.
//!
//! While any engine reports that it runs (Signal K `propulsion.*`, NMEA `RPM`), the boat is motoring.
//! Once every engine that reports has been stopped for [`STOPPED_FOR`], it is sailing. When no engine
//! data has come for [`QUIET`] (a link that dropped, a boat without engine data), the engines say nothing
//! and the user's switch rules again.

use super::sample::SailMode;
use crate::comm::state::OwnShip;
use chrono::{DateTime, Duration, Utc};

/// How long every engine must have been stopped before the boat is sailing again.
pub const STOPPED_FOR: Duration = Duration::seconds(10);
/// Engine readings older than this are no data.
pub const QUIET: Duration = Duration::seconds(30);

#[derive(Default)]
pub struct EngineMode {
    stopped_since: Option<DateTime<Utc>>,
    verdict: Option<SailMode>,
}

impl EngineMode {
    /// What the engines say the mode is now, or `None` when no engine data is coming.
    pub fn update(&mut self, own: &OwnShip, now: DateTime<Utc>) -> Option<SailMode> {
        match own.engine_running(now, QUIET) {
            None => {
                *self = EngineMode::default();
            }
            Some(true) => {
                self.stopped_since = None;
                self.verdict = Some(SailMode::Motoring);
            }
            Some(false) => {
                let since = *self.stopped_since.get_or_insert(now);
                if now - since >= STOPPED_FOR {
                    self.verdict = Some(SailMode::Sailing);
                }
            }
        }
        self.verdict
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comm::nmea::{frame, parse_line};
    use chrono::TimeZone;

    fn t(s: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_780_000_000 + s, 0).unwrap()
    }

    fn rpm(o: &mut OwnShip, r: f64, at: DateTime<Utc>) {
        o.apply(&parse_line(&frame(&format!("ERRPM,E,1,{r},,A"), '$')).unwrap().unwrap(), at);
    }

    #[test]
    fn revolutions_decide_and_silence_hands_back() {
        let (mut e, mut o) = (EngineMode::default(), OwnShip::default());
        assert_eq!(e.update(&o, t(0)), None);
        rpm(&mut o, 1800.0, t(1));
        assert_eq!(e.update(&o, t(1)), Some(SailMode::Motoring));
        // Stopped: still motoring for a few seconds (a stall, a shift through neutral), then sailing.
        for s in 2..11 {
            rpm(&mut o, 0.0, t(s));
            assert_eq!(e.update(&o, t(s)), Some(SailMode::Motoring), "{s}");
        }
        rpm(&mut o, 0.0, t(12));
        assert_eq!(e.update(&o, t(12)), Some(SailMode::Sailing));
        // Started again: motoring at once.
        rpm(&mut o, 900.0, t(13));
        assert_eq!(e.update(&o, t(13)), Some(SailMode::Motoring));
        // The link drops: after a while the engines say nothing.
        assert_eq!(e.update(&o, t(30)), Some(SailMode::Motoring));
        assert_eq!(e.update(&o, t(50)), None);
    }

    #[test]
    fn an_engine_stopped_from_the_start_means_sailing_after_a_while() {
        let (mut e, mut o) = (EngineMode::default(), OwnShip::default());
        rpm(&mut o, 0.0, t(0));
        assert_eq!(e.update(&o, t(0)), None);
        rpm(&mut o, 0.0, t(11));
        assert_eq!(e.update(&o, t(11)), Some(SailMode::Sailing));
    }
}
