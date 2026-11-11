//! The anchor alarm.

use crate::geo::LatLon;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnchorStatus {
    pub distance_nm: f64,
    pub alarm: bool,
}

/// A circle around the anchor position. The alarm sounds when the boat drifts
/// outside it, and clears only once the boat is back inside by a margin, so a
/// boat swinging on the edge does not flap.
#[derive(Clone, Debug)]
pub struct AnchorWatch {
    pub center: LatLon,
    pub radius_nm: f64,
    alarming: bool,
}

/// Fraction of the radius the boat must return within to clear the alarm.
const CLEAR_FRACTION: f64 = 0.9;

impl AnchorWatch {
    pub fn new(center: LatLon, radius_nm: f64) -> AnchorWatch {
        AnchorWatch { center, radius_nm, alarming: false }
    }

    pub fn check(&mut self, pos: &LatLon) -> AnchorStatus {
        let d = self.center.distance_nm(pos);
        if d > self.radius_nm {
            self.alarming = true;
        } else if d <= self.radius_nm * CLEAR_FRACTION {
            self.alarming = false;
        }
        AnchorStatus { distance_nm: d, alarm: self.alarming }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alarm_trips_outside_the_circle_and_has_hysteresis() {
        let c = LatLon::new(48.0, -4.0);
        let mut w = AnchorWatch::new(c, 0.1);
        assert!(!w.check(&c).alarm);
        // 0.12 nm north.
        assert!(w.check(&c.destination(0.0, 0.12)).alarm);
        // Back inside, but not by the margin: still ringing.
        assert!(w.check(&c.destination(0.0, 0.095)).alarm);
        // Well inside: cleared.
        assert!(!w.check(&c.destination(0.0, 0.05)).alarm);
        // And stays cleared at 0.095, which is inside the radius.
        assert!(!w.check(&c.destination(0.0, 0.095)).alarm);
    }

    #[test]
    fn distance_is_reported() {
        let c = LatLon::new(0.0, 0.0);
        let mut w = AnchorWatch::new(c, 1.0);
        let s = w.check(&LatLon::new(0.0, 1.0 / 60.0));
        assert!((s.distance_nm - 1.0).abs() < 0.01);
    }
}
