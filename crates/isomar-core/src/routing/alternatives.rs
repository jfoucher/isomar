//! Ways to get as far as the forecast reaches.
//!
//! The user chooses them: each position clicked on a front is read back
//! through the tree to the start and becomes an alternative. When the
//! destination was reached there is also the route that got there.

use super::isochrone::{Exploration, Node};
use super::stats::Stats;
use crate::geo::LatLon;
use chrono::{DateTime, Utc};

#[derive(Clone, Debug)]
pub struct Alternative {
    /// From the start to where the boat is at the chosen front (or the
    /// destination, when it arrives).
    pub path: Vec<Node>,
    pub distance_nm: f64,
    /// Great-circle distance from the end of the path to the destination.
    pub remaining_nm: f64,
    pub motor_hours: f64,
    pub max_tws_kn: f64,
    pub end: DateTime<Utc>,
    pub arrives: bool,
}

impl Alternative {
    /// The alternative that follows `path` (at least the start) towards `dest`.
    pub fn new(path: Vec<Node>, dest: LatLon, arrives: bool) -> Alternative {
        let distance_nm = path.windows(2).map(|w| w[0].pos.distance_nm(&w[1].pos)).sum();
        let motor_hours = path.windows(2).filter(|w| w[1].motoring).map(|w| (w[1].time - w[0].time).num_seconds() as f64 / 3600.0).sum();
        let max_tws_kn = path.iter().skip(1).map(|n| n.tws_kn).fold(0.0, f64::max);
        let last = path.last().expect("a path has the start");
        Alternative { distance_nm, remaining_nm: last.pos.distance_nm(&dest), motor_hours, max_tws_kn, end: last.time, arrives, path }
    }

    pub fn stats(&self) -> Stats {
        Stats::of(&self.path)
    }

    /// Where the boat is at `t`, along the path, for playing it back. Before
    /// the start it waits there; after the end it stays there.
    pub fn position_at(&self, t: DateTime<Utc>) -> LatLon {
        let p = &self.path;
        let i = p.partition_point(|n| n.time <= t);
        if i == 0 {
            return p[0].pos;
        }
        if i == p.len() {
            return p[p.len() - 1].pos;
        }
        let (a, b) = (&p[i - 1], &p[i]);
        let span = (b.time - a.time).num_milliseconds().max(1) as f64;
        let f = ((t - a.time).num_milliseconds() as f64 / span).clamp(0.0, 1.0);
        let d = a.pos.distance_nm(&b.pos);
        a.pos.destination(a.pos.bearing_to(&b.pos), d * f)
    }

    /// The path up to `t`, ending at the boat's position then.
    pub fn track_until(&self, t: DateTime<Utc>) -> Vec<LatLon> {
        let mut out: Vec<LatLon> = self.path.iter().take_while(|n| n.time <= t).map(|n| n.pos).collect();
        if t < self.end && !out.is_empty() {
            out.push(self.position_at(t));
        }
        out
    }

    /// The same track as `track_until`, in runs of legs under sail (`false`) and under engine (`true`),
    /// each starting where the one before ends.
    pub fn track_legs_until(&self, t: DateTime<Utc>) -> Vec<(bool, Vec<LatLon>)> {
        let mut out: Vec<(bool, Vec<LatLon>)> = Vec::new();
        for w in self.path.windows(2) {
            let (a, b) = (&w[0], &w[1]);
            if a.time >= t {
                break;
            }
            let end = if b.time <= t { b.pos } else { self.position_at(t) };
            match out.last_mut() {
                Some((motoring, pts)) if *motoring == b.motoring => pts.push(end),
                _ => out.push((b.motoring, vec![a.pos, end])),
            }
        }
        out
    }
}

/// The route that reached the destination, if the search did.
pub fn arrival(ex: &Exploration) -> Option<Alternative> {
    ex.arrived.as_ref().map(|r| Alternative::new(r.nodes.clone(), ex.dest, true))
}

/// The route that arrived, else the way to the position of the last front
/// nearest the destination, if the search got anywhere.
pub fn best(ex: &Exploration) -> Option<Alternative> {
    arrival(ex).or_else(|| ex.nearest.map(|n| to_node(ex, n.index)))
}

/// The way to the position `i` of the tree, which the boat reaches at the
/// time of its front.
pub fn to_node(ex: &Exploration, i: usize) -> Alternative {
    Alternative::new(ex.path_to(i), ex.dest, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::isochrone::{Environment, Settings, explore, route};
    use crate::routing::land::NoLand;
    use crate::routing::polar::Polar;
    use chrono::{Duration, TimeZone};

    /// A steady wind that is known only until `until`.
    struct Until {
        until: DateTime<Utc>,
        from: f64,
    }

    impl Environment for Until {
        fn wind(&self, _: &LatLon, t: DateTime<Utc>) -> Option<(f64, f64)> {
            (t <= self.until).then_some((15.0, self.from))
        }
        fn horizon(&self) -> Option<DateTime<Utc>> {
            Some(self.until)
        }
    }

    fn polar() -> Polar {
        Polar::parse("TWA;3;40\n0;0;0\n40;0;0\n60;6;6\n90;6;6\n180;6;6\n").unwrap()
    }

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, 6, 0, 0).unwrap()
    }

    fn start() -> LatLon {
        LatLon::new(48.0, -5.0)
    }

    /// 100 nm to windward at 6 kn needs far more than the 10 hours known.
    fn too_far() -> (LatLon, Until) {
        (start().destination(0.0, 100.0), Until { until: t0() + Duration::hours(10), from: 0.0 })
    }

    #[test]
    fn a_forecast_that_ends_gives_the_fronts_so_far_not_an_error() {
        let (dest, env) = too_far();
        let ex = explore(start(), dest, t0(), &polar(), &env, &NoLand, &Settings::default(), None).unwrap();
        assert!(ex.arrived.is_none());
        let h = ex.horizon.expect("stopped by the forecast");
        assert!(h > env.until && h <= env.until + Duration::minutes(30), "{h}");
        assert_eq!(ex.fronts.len(), 21, "one front per half hour up to the horizon");
        assert_eq!(ex.tree[*ex.fronts.last().unwrap().first().unwrap()].node.time, h);
        // `route` has no result to give, and says why.
        let err = route(start(), dest, t0(), &polar(), &env, &NoLand, &Settings::default()).unwrap_err();
        assert!(matches!(err, crate::routing::isochrone::RouteError::ForecastEnds { hours } if (10.0..11.0).contains(&hours)), "{err:?}");
    }

    #[test]
    fn a_passage_inside_the_forecast_arrives() {
        let dest = start().destination(90.0, 20.0);
        let env = Until { until: t0() + Duration::hours(48), from: 0.0 };
        let ex = explore(start(), dest, t0(), &polar(), &env, &NoLand, &Settings::default(), None).unwrap();
        assert!(ex.horizon.is_none() && ex.arrived.is_some());
        let a = arrival(&ex).expect("it arrived");
        assert!(a.arrives && a.remaining_nm == 0.0);
        assert_eq!(a.path.last().unwrap().pos, dest);
    }

    #[test]
    fn there_is_no_arrival_when_the_forecast_ends_first() {
        let (dest, env) = too_far();
        let ex = explore(start(), dest, t0(), &polar(), &env, &NoLand, &Settings::default(), None).unwrap();
        assert!(arrival(&ex).is_none());
    }

    #[test]
    fn a_position_on_a_front_is_read_back_to_the_start() {
        let (dest, env) = too_far();
        let ex = explore(start(), dest, t0(), &polar(), &env, &NoLand, &Settings::default(), None).unwrap();
        let front = ex.fronts.last().unwrap();
        for &i in [front[0], front[front.len() / 2], front[front.len() - 1]].iter() {
            let a = to_node(&ex, i);
            assert_eq!(a.path[0].pos, start());
            assert!(a.path.windows(2).all(|w| w[0].time < w[1].time), "time runs forward");
            assert_eq!(a.path.last().unwrap().pos, ex.tree[i].node.pos);
            assert_eq!(a.end, ex.horizon.unwrap());
            assert!(!a.arrives);
            assert!((a.remaining_nm - ex.tree[i].node.pos.distance_nm(&dest)).abs() < 1e-9);
        }
        // An earlier front gives a shorter alternative.
        let early = to_node(&ex, ex.fronts[3][0]);
        assert_eq!(early.end, t0() + Duration::minutes(30 * 4));
    }

    #[test]
    fn the_boat_is_interpolated_along_its_path_and_waits_at_the_ends() {
        let (dest, env) = too_far();
        let ex = explore(start(), dest, t0(), &polar(), &env, &NoLand, &Settings::default(), None).unwrap();
        let a = &to_node(&ex, ex.fronts.last().unwrap()[0]);
        assert_eq!(a.position_at(t0() - Duration::hours(1)), start());
        assert_eq!(a.position_at(a.end + Duration::hours(5)), a.path.last().unwrap().pos);
        let mid = t0() + Duration::minutes(45);
        let (p, q) = (&a.path[1], &a.path[2]);
        let pos = a.position_at(mid);
        let (d1, d2) = (p.pos.distance_nm(&pos), pos.distance_nm(&q.pos));
        assert!(d1 > 0.0 && d2 > 0.0 && (d1 + d2 - p.pos.distance_nm(&q.pos)).abs() < 0.01, "{d1} {d2}");
        let track = a.track_until(mid);
        assert_eq!(track.len(), 3, "start, the first step, and the boat now");
        assert_eq!(*track.last().unwrap(), pos);
        assert_eq!(a.track_until(a.end).len(), a.path.len());
    }

    #[test]
    fn the_track_is_cut_into_runs_under_sail_and_under_engine() {
        let node = |h: i64, motoring: bool| Node {
            pos: LatLon::new(48.0 + h as f64 * 0.1, -5.0),
            time: t0() + Duration::hours(h),
            cog: 0.0,
            sog_kn: 6.0,
            heading: 0.0,
            tws_kn: 10.0,
            twd: 0.0,
            motoring,
            current: (0.0, 0.0),
        };
        // Sail, sail, engine, engine, sail.
        let path = vec![node(0, false), node(1, false), node(2, false), node(3, true), node(4, true), node(5, false)];
        let a = Alternative::new(path.clone(), LatLon::new(49.0, -5.0), false);
        let runs = a.track_legs_until(a.end);
        let kinds: Vec<bool> = runs.iter().map(|r| r.0).collect();
        assert_eq!(kinds, [false, true, false]);
        assert_eq!(runs[0].1, vec![path[0].pos, path[1].pos, path[2].pos]);
        assert_eq!(runs[1].1, vec![path[2].pos, path[3].pos, path[4].pos], "each run starts where the last ends");
        assert_eq!(runs[2].1, vec![path[4].pos, path[5].pos]);
        // Part way through the engine run, the track ends at the boat.
        let t = t0() + Duration::minutes(210);
        let runs = a.track_legs_until(t);
        assert_eq!(runs.len(), 2);
        assert_eq!(*runs[1].1.last().unwrap(), a.position_at(t));
        let all: Vec<LatLon> = runs.iter().enumerate().flat_map(|(i, r)| r.1.iter().skip((i > 0) as usize).copied()).collect();
        assert_eq!(all, a.track_until(t));
        // Before the start there is nothing to draw.
        assert!(a.track_legs_until(t0()).is_empty());
    }

    #[test]
    fn a_start_with_no_weather_is_still_an_error() {
        let (dest, env) = too_far();
        let late = env.until + Duration::hours(5);
        let err = explore(start(), dest, late, &polar(), &env, &NoLand, &Settings::default(), None).unwrap_err();
        assert_eq!(err, crate::routing::isochrone::RouteError::NoWeather);
    }

    #[test]
    fn cancelling_stops_the_search() {
        use crate::routing::isochrone::{Progress, RouteError};
        use std::sync::atomic::Ordering;
        let (dest, env) = too_far();
        let p = Progress::default();
        p.cancel.store(true, Ordering::Relaxed);
        let err = explore(start(), dest, t0(), &polar(), &env, &NoLand, &Settings::default(), Some(&p)).unwrap_err();
        assert_eq!(err, RouteError::Cancelled);
        let p = Progress::default();
        explore(start(), dest, t0(), &polar(), &env, &NoLand, &Settings::default(), Some(&p)).unwrap();
        assert_eq!(p.steps.load(Ordering::Relaxed), 21,"steps expected to the forecast end");
    }
}
