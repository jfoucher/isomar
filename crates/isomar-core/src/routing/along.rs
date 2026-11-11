//! Sailing a route already chosen through a forecast: no search, one boat following the line, beating
//! or running in tacks where the line goes too close to the wind or too deep downwind, motoring by the
//! router's rule. What several forecasts make of the same route: when it would arrive, how hard it
//! would blow, how long under engine. The weather router finds the best way for one forecast; this
//! tells how that way fares if another forecast is the right one.

use super::isochrone::{Environment, Node, Settings};
use super::polar::Polar;
use crate::geo::{LatLon, normalize_bearing};
use chrono::{DateTime, Duration, Utc};

/// The time step of the sail, minutes: fine enough for a coastal leg, cheap for an ocean one.
const STEP_MIN: f64 = 10.0;

#[derive(Clone, Debug)]
pub struct Sailed {
    /// The boat every step, from the start to where it ended.
    pub path: Vec<Node>,
    /// It reached the end of the route.
    pub arrives: bool,
    pub end: DateTime<Utc>,
    pub distance_nm: f64,
    /// Along the route, from where it ended to the last point; 0 when it arrives.
    pub remaining_nm: f64,
    pub motor_hours: f64,
    pub max_tws_kn: f64,
    /// Hours in more wind than `Settings::max_wind_kn`: the route goes through it, the boat cannot avoid it.
    pub over_limit_hours: f64,
    /// Hours with a current across the line stronger than the boat: it cannot hold the line there, and the
    /// time is a guess.
    pub swept_hours: f64,
}

impl Sailed {
    pub fn hours(&self) -> f64 {
        self.path.first().map_or(0.0, |s| (self.end - s.time).num_seconds() as f64 / 3600.0)
    }
}

/// The smaller angle between two directions, 0..180.
fn angle_between(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(360.0);
    if d > 180.0 { 360.0 - d } else { d }
}

/// Speed made along a course `twa` degrees off the wind: the polar's where it can be sailed straight,
/// else the best angle's, tacking (or gybing) either side of the course.
pub fn course_speed(polar: &Polar, twa: f64, tws: f64) -> f64 {
    let (up_angle, up_vmg) = polar.best_vmg(tws, true);
    let (down_angle, down_vmg) = polar.best_vmg(tws, false);
    if up_vmg > 0.0 && twa < up_angle {
        // Both tacks at the best angle: the speed made to windward, along a course `twa` off it.
        up_vmg / twa.to_radians().cos().max(0.05)
    } else if down_vmg > 0.0 && twa > down_angle {
        down_vmg / (180.0 - twa).to_radians().cos().max(0.05)
    } else {
        polar.speed(twa, tws)
    }
}

/// Sails `points` from `depart` through `env`. It stops where the forecast ends, at `s.max_hours`, or
/// at the last point.
pub fn sail(points: &[LatLon], depart: DateTime<Utc>, polar: &Polar, env: &dyn Environment, s: &Settings) -> Sailed {
    let total: f64 = points.windows(2).map(|w| w[0].distance_nm(&w[1])).sum();
    let mut pos = points.first().copied().unwrap_or(LatLon::new(0.0, 0.0));
    let start = Node { pos, time: depart, cog: 0.0, sog_kn: 0.0, heading: 0.0, tws_kn: 0.0, twd: 0.0, motoring: false, current: (0.0, 0.0) };
    let mut out = Sailed { path: vec![start], arrives: points.len() < 2, end: depart, distance_nm: 0.0, remaining_nm: total, motor_hours: 0.0, max_tws_kn: 0.0, over_limit_hours: 0.0, swept_hours: 0.0 };
    let (mut leg, mut t) = (0usize, depart);
    let dt_h = STEP_MIN / 60.0;
    let limit = depart + Duration::minutes((s.max_hours * 60.0) as i64);
    let currents = env.has_currents();
    while leg + 1 < points.len() {
        if t >= limit {
            break;
        }
        let Some((tws, twd)) = env.wind(&pos, t) else { break };
        let course = pos.bearing_to(&points[leg + 1]);
        let current = if currents { env.current(&pos, t).unwrap_or((0.0, 0.0)) } else { (0.0, 0.0) };
        // The boat sails in the wind over the water.
        let (tws, twd) = crate::geo::wind_over_water(tws, twd, current);
        let speed_at = |heading: f64| {
            let sail = course_speed(polar, angle_between(twd, heading), tws) * s.performance;
            match s.motor_speed_kn {
                Some(m) if sail < s.motor_below_kn => (m.max(sail), true),
                _ => (sail, false),
            }
        };
        let (mut speed, mut motoring) = speed_at(course);
        let (sc, cc) = course.to_radians().sin_cos();
        let (ce, cn) = current;
        // The current along the course helps or hinders; across it the boat heads up into it (crabs) to
        // hold the line, which costs some of its speed and changes its angle to the wind.
        let along = ce * sc + cn * cc;
        let across = ce * cc - cn * sc;
        let mut heading = normalize_bearing(course);
        let mut swept = false;
        let sog = if across == 0.0 {
            (speed + along).max(0.0)
        } else {
            for _ in 0..2 {
                heading = normalize_bearing(course - (across / speed.max(0.1)).clamp(-1.0, 1.0).asin().to_degrees());
                (speed, motoring) = speed_at(heading);
            }
            if across.abs() >= speed {
                swept = true;
                along.max(0.0)
            } else {
                ((speed * speed - across * across).sqrt() + along).max(0.0)
            }
        };
        // How much of this step the route still needs: the last one ends at the last point.
        let mut go = sog * dt_h;
        let mut used = dt_h;
        let before = pos;
        loop {
            let left = pos.distance_nm(&points[leg + 1]);
            if go < left {
                pos = pos.destination(pos.bearing_to(&points[leg + 1]), go);
                break;
            }
            go -= left;
            pos = points[leg + 1];
            leg += 1;
            if leg + 1 == points.len() {
                if sog > 0.0 {
                    used -= go / sog;
                }
                break;
            }
        }
        let moved = before.distance_nm(&pos).max(if leg + 1 == points.len() { sog * used } else { 0.0 });
        out.distance_nm += moved;
        t += Duration::milliseconds((used * 3_600_000.0) as i64);
        if motoring {
            out.motor_hours += used;
        }
        if tws > s.max_wind_kn {
            out.over_limit_hours += used;
        }
        if swept {
            out.swept_hours += used;
        }
        out.max_tws_kn = out.max_tws_kn.max(tws);
        out.path.push(Node { pos, time: t, cog: normalize_bearing(course), sog_kn: sog, heading, tws_kn: tws, twd, motoring, current });
    }
    out.arrives = leg + 1 >= points.len();
    out.end = t;
    out.distance_nm = out.distance_nm.min(total);
    out.remaining_nm = if out.arrives { 0.0 } else { (pos.distance_nm(&points[leg + 1]) + points[leg + 1..].windows(2).map(|w| w[0].distance_nm(&w[1])).sum::<f64>()).max(0.0) };
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap()
    }

    /// The same wind everywhere until `until`.
    struct Steady {
        kn: f64,
        from: f64,
        until: Option<DateTime<Utc>>,
    }

    impl Environment for Steady {
        fn wind(&self, _: &LatLon, t: DateTime<Utc>) -> Option<(f64, f64)> {
            self.until.is_none_or(|u| t <= u).then_some((self.kn, self.from))
        }
    }

    fn line(nm: f64) -> Vec<LatLon> {
        let a = LatLon::new(47.0, -5.0);
        vec![a, a.destination(0.0, nm / 2.0), a.destination(0.0, nm)]
    }

    #[test]
    fn a_beam_reach_takes_the_distance_at_the_polar_speed() {
        let p = Polar::generic_cruiser();
        let env = Steady { kn: 12.0, from: 90.0, until: None };
        let r = sail(&line(30.0), t0(), &p, &env, &Settings::default());
        let want = 30.0 / p.speed(90.0, 12.0);
        assert!(r.arrives && (r.hours() - want).abs() < 0.05, "{} vs {want}", r.hours());
        assert!((r.distance_nm - 30.0).abs() < 0.1 && r.remaining_nm == 0.0);
        assert_eq!(r.motor_hours, 0.0);
    }

    #[test]
    fn dead_upwind_it_beats_at_the_best_vmg() {
        let p = Polar::generic_cruiser();
        let env = Steady { kn: 12.0, from: 0.0, until: None };
        let r = sail(&line(20.0), t0(), &p, &env, &Settings::default());
        let vmg = p.best_vmg(12.0, true).1;
        assert!((r.hours() - 20.0 / vmg).abs() < 0.1, "{} vs {}", r.hours(), 20.0 / vmg);
        // Slower than a reach would be.
        assert!(r.hours() > 20.0 / p.speed(90.0, 12.0));
    }

    #[test]
    fn where_the_forecast_ends_it_stops_with_what_is_left() {
        let p = Polar::generic_cruiser();
        let env = Steady { kn: 12.0, from: 90.0, until: Some(t0() + Duration::hours(2)) };
        let r = sail(&line(60.0), t0(), &p, &env, &Settings::default());
        assert!(!r.arrives);
        assert!((r.end - t0()).num_minutes() <= 130 && r.remaining_nm > 40.0, "{} {}", r.end, r.remaining_nm);
        assert!((r.distance_nm + r.remaining_nm - 60.0).abs() < 0.5);
    }

    #[test]
    fn in_a_calm_it_motors_when_allowed_and_counts_the_wind_over_the_limit() {
        let p = Polar::generic_cruiser();
        let calm = Steady { kn: 1.0, from: 90.0, until: None };
        let s = Settings { motor_speed_kn: Some(6.0), ..Settings::default() };
        let r = sail(&line(12.0), t0(), &p, &calm, &s);
        assert!(r.arrives && (r.hours() - 2.0).abs() < 0.05 && (r.motor_hours - r.hours()).abs() < 0.01, "{} {}", r.hours(), r.motor_hours);
        let gale = Steady { kn: 40.0, from: 90.0, until: None };
        let r = sail(&line(12.0), t0(), &p, &gale, &Settings::default());
        assert!(r.arrives && r.over_limit_hours > 0.0 && r.max_tws_kn == 40.0);
    }

    /// A wind, and a current everywhere.
    struct Stream {
        wind: (f64, f64),
        current: (f64, f64),
    }

    impl Environment for Stream {
        fn wind(&self, _: &LatLon, _: DateTime<Utc>) -> Option<(f64, f64)> {
            Some(self.wind)
        }
        fn current(&self, _: &LatLon, _: DateTime<Utc>) -> Option<(f64, f64)> {
            Some(self.current)
        }
        fn has_currents(&self) -> bool {
            true
        }
    }

    #[test]
    fn across_a_current_the_boat_heads_up_into_it_to_hold_the_line() {
        let p = Polar::generic_cruiser();
        let s = Settings::default();
        // North with a beam wind, and a 2 kn stream setting east across the line. Carried east, the boat
        // feels 14 kn from the east over the water: the same as 14 kn over the ground and no stream, but
        // for the stream across the line.
        let still = sail(&line(20.0), t0(), &p, &Stream { wind: (14.0, 90.0), current: (0.0, 0.0) }, &s);
        let across = sail(&line(20.0), t0(), &p, &Stream { wind: (12.0, 90.0), current: (2.0, 0.0) }, &s);
        assert!(across.arrives && across.hours() > still.hours(), "{} vs {}", across.hours(), still.hours());
        // It points west of north, into the stream, and stays on the line.
        let n = &across.path[3];
        assert!(n.heading > 180.0 && n.heading < 359.0 && (n.cog - 0.0).abs() < 1.0, "heading {} cog {}", n.heading, n.cog);
        assert_eq!(across.swept_hours, 0.0);
        // A stream across stronger than the boat sweeps it off: the hours are counted.
        let swept = sail(&line(5.0), t0(), &p, &Stream { wind: (12.0, 90.0), current: (12.0, 2.0) }, &Settings { max_hours: 4.0, ..s.clone() });
        assert!(swept.swept_hours > 0.0);
    }

    #[test]
    fn a_stream_along_the_line_helps_or_hinders() {
        let p = Polar::generic_cruiser();
        let s = Settings::default();
        // A beam wind of 12 kn, so a stream along the line barely changes the wind over the water.
        let still = sail(&line(20.0), t0(), &p, &Stream { wind: (12.0, 90.0), current: (0.0, 0.0) }, &s);
        let fair = sail(&line(20.0), t0(), &p, &Stream { wind: (12.0, 90.0), current: (0.0, 1.5) }, &s);
        let foul = sail(&line(20.0), t0(), &p, &Stream { wind: (12.0, 90.0), current: (0.0, -1.5) }, &s);
        assert!(fair.hours() < still.hours() && foul.hours() > still.hours());
    }
}
