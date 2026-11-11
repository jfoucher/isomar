//! A route sailed through each member of an ensemble forecast (`along::sail`): not one arrival time but
//! their spread, and how many of the members take the boat into more wind than its limit. A few models
//! side by side say whether they agree; an ensemble says how sure that is.

use super::along::Sailed;
use chrono::{DateTime, Duration, Utc};

#[derive(Clone, Debug, PartialEq)]
pub struct Spread {
    pub members: usize,
    /// How many get there before their forecast ends.
    pub arrived: usize,
    /// The arrival one member in ten beats, half do, nine in ten do; of the members that arrive.
    pub p10: Option<DateTime<Utc>>,
    pub p50: Option<DateTime<Utc>>,
    pub p90: Option<DateTime<Utc>>,
    /// How many members meet more wind than the limit somewhere on the route.
    pub over_limit: usize,
    /// The strongest wind on the route: half the members see no more than this, and nine in ten.
    pub max_wind_p50: f64,
    pub max_wind_p90: f64,
    /// The most hours under engine of half the members.
    pub motor_p50: f64,
}

/// The value at `q` (0..1) of the sorted `v`, by nearest rank.
fn quantile<T: Copy>(v: &[T], q: f64) -> Option<T> {
    if v.is_empty() {
        return None;
    }
    let i = ((q * v.len() as f64).ceil() as usize).clamp(1, v.len()) - 1;
    Some(v[i])
}

pub fn spread(sails: &[Sailed]) -> Spread {
    let mut arrivals: Vec<DateTime<Utc>> = sails.iter().filter(|s| s.arrives).map(|s| s.end).collect();
    arrivals.sort();
    let mut winds: Vec<f64> = sails.iter().map(|s| s.max_tws_kn).collect();
    winds.sort_by(f64::total_cmp);
    let mut motor: Vec<f64> = sails.iter().map(|s| s.motor_hours).collect();
    motor.sort_by(f64::total_cmp);
    Spread {
        members: sails.len(),
        arrived: arrivals.len(),
        p10: quantile(&arrivals, 0.1),
        p50: quantile(&arrivals, 0.5),
        p90: quantile(&arrivals, 0.9),
        over_limit: sails.iter().filter(|s| s.over_limit_hours > 0.0).count(),
        max_wind_p50: quantile(&winds, 0.5).unwrap_or(0.0),
        max_wind_p90: quantile(&winds, 0.9).unwrap_or(0.0),
        motor_p50: quantile(&motor, 0.5).unwrap_or(0.0),
    }
}

impl Spread {
    /// From the arrival one member in ten beats to the one nine in ten do.
    pub fn width(&self) -> Option<Duration> {
        Some(self.p90? - self.p10?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::LatLon;
    use crate::routing::isochrone::Node;
    use chrono::TimeZone;

    fn sailed(h: i64, arrives: bool, wind: f64, over: f64) -> Sailed {
        let t0 = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        let n = Node { pos: LatLon::new(47.0, -5.0), time: t0, cog: 0.0, sog_kn: 5.0, heading: 0.0, tws_kn: wind, twd: 0.0, motoring: false, current: (0.0, 0.0) };
        Sailed { path: vec![n], arrives, end: t0 + Duration::hours(h), distance_nm: 10.0, remaining_nm: 0.0, motor_hours: h as f64 / 10.0, max_tws_kn: wind, over_limit_hours: over, swept_hours: 0.0 }
    }

    #[test]
    fn the_spread_of_ten_members() {
        let sails: Vec<Sailed> = (0..10).map(|i| sailed(20 + i, i != 9, 15.0 + i as f64, if i >= 8 { 2.0 } else { 0.0 })).collect();
        let s = spread(&sails);
        assert_eq!((s.members, s.arrived, s.over_limit), (10, 9, 2));
        let t0 = sails[0].path[0].time;
        // Nine arrive, after 20 to 28 hours.
        assert_eq!(s.p10, Some(t0 + Duration::hours(20)));
        assert_eq!(s.p50, Some(t0 + Duration::hours(24)));
        assert_eq!(s.p90, Some(t0 + Duration::hours(28)));
        assert_eq!(s.width(), Some(Duration::hours(8)));
        assert_eq!((s.max_wind_p50, s.max_wind_p90), (19.0, 23.0));
        assert!(spread(&[]).p50.is_none());
    }
}
