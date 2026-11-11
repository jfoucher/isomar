//! Whether the boat looks motored, for when no engine reports: a suggestion to the user, never a decision.
//!
//! A window looks motored when the boat goes where sails cannot take it: closer than 28 degrees to the true
//! wind at more than 2.5 kn, much faster than the polar in light air, or at 3 kn in next to no wind. It
//! takes [`HOLD`] of such windows in a row (gaps of a minute allowed) to suggest motoring.

use super::sample::{Hint, Sample};
use crate::routing::polar::Polar;
use chrono::{DateTime, Duration, Utc};

/// How long the boat must look motored before the hint is given.
pub const HOLD: Duration = Duration::minutes(5);
/// Windows further apart than this do not make a run.
const BREAK: Duration = Duration::seconds(60);

#[derive(Default)]
pub struct Hinter {
    since: Option<DateTime<Utc>>,
    last: Option<DateTime<Utc>>,
}

/// Whether one window looks motored.
pub fn looks_motored(s: &Sample, polar: &Polar) -> bool {
    let (Some(twa), Some(tws)) = (s.twa(), s.tws) else { return false };
    let speed = s.speed();
    let twa = twa.abs();
    (twa < 28.0 && speed > 2.5) || (tws < 8.0 && speed > 2.0 && speed > 1.4 * polar.speed(twa, tws)) || (tws < 3.0 && speed > 3.0)
}

impl Hinter {
    /// Takes the next window and says what it suggests.
    pub fn push(&mut self, s: &Sample, polar: &Polar) -> Hint {
        if self.last.is_some_and(|l| s.t - l > BREAK) {
            self.since = None;
        }
        self.last = Some(s.t);
        if looks_motored(s, polar) {
            let since = *self.since.get_or_insert(s.t);
            if s.t - since >= HOLD {
                return Hint::LooksMotored;
            }
        } else {
            self.since = None;
        }
        Hint::None
    }

    pub fn reset(&mut self) {
        *self = Hinter::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(s: i64, twa: f64, tws: f64, stw: f64) -> Sample {
        Sample { t: Utc.timestamp_opt(1_780_000_000 + s, 0).unwrap(), heading: Some(0.0), twd: Some(twa), tws: Some(tws), stw: Some(stw), sog: stw, ..Default::default() }
    }

    #[test]
    fn motoring_into_the_wind_is_suggested_after_five_minutes() {
        let p = Polar::generic_cruiser();
        let mut h = Hinter::default();
        let hints: Vec<Hint> = (0..40).map(|i| h.push(&at(i * 10, 10.0, 12.0, 6.0), &p)).collect();
        assert_eq!(hints[29], Hint::None);
        assert_eq!(hints[30], Hint::LooksMotored);
        // Sailing close-hauled is not motoring.
        let mut h = Hinter::default();
        assert!((0..60).all(|i| h.push(&at(i * 10, 42.0, 14.0, 5.6), &p) == Hint::None));
        // Six knots in four knots of wind on a reach is.
        assert!(looks_motored(&at(0, 90.0, 4.0, 6.0), &p));
        assert!(!looks_motored(&at(0, 90.0, 4.0, 3.5), &p));
        // A break in the log starts the count again.
        let mut h = Hinter::default();
        for i in 0..20 {
            h.push(&at(i * 10, 10.0, 12.0, 6.0), &p);
        }
        assert_eq!(h.push(&at(1000, 10.0, 12.0, 6.0), &p), Hint::None);
    }
}
