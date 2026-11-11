//! What a route is like to sail: how windy, on which points of sail, how much
//! under engine, how much in the dark.
//!
//! Everything is read off the nodes of a route. Leg `i` runs from node
//! `i - 1` to node `i`, and node `i` carries the wind and heading the leg was
//! sailed with.

use super::isochrone::Node;
use crate::geo::LatLon;
use chrono::{DateTime, Utc};

/// Where the wind is relative to the bow, from the true wind angle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointOfSail {
    CloseHauled,
    CloseReach,
    BeamReach,
    BroadReach,
    Run,
}

impl PointOfSail {
    pub const ALL: [PointOfSail; 5] = [PointOfSail::CloseHauled, PointOfSail::CloseReach, PointOfSail::BeamReach, PointOfSail::BroadReach, PointOfSail::Run];

    /// The point of sail at a true wind angle of `twa` degrees (0 to 180).
    pub fn of(twa: f64) -> PointOfSail {
        match twa {
            t if t < 55.0 => PointOfSail::CloseHauled,
            t if t < 85.0 => PointOfSail::CloseReach,
            t if t < 115.0 => PointOfSail::BeamReach,
            t if t < 150.0 => PointOfSail::BroadReach,
            _ => PointOfSail::Run,
        }
    }
}

/// Upper limits, in knots, of the wind bands in `Stats::wind_hours`; the
/// last band is everything above.
pub const WIND_BANDS_KN: [f64; 3] = [10.0, 20.0, 30.0];

/// The sun is this far below the horizon (degrees) when it counts as dark:
/// the end of civil twilight.
const DARK_BELOW_DEG: f64 = -6.0;

/// A current along the track of at least this many knots, with or against the boat, counts as fair or foul.
pub const FAIR_FOUL_KN: f64 = 0.5;
/// Wind against tide: at least this much wind...
pub const WIND_AGAINST_TIDE_KN: f64 = 15.0;
/// ...blowing within `AGAINST_DEG` of straight against a current of at least this many knots.
pub const TIDE_AGAINST_WIND_KN: f64 = 1.5;
const AGAINST_DEG: f64 = 60.0;

/// Whether a wind of `tws` knots from `twd` blows against the current `(ce, cn)` hard enough to raise a
/// short steep sea: the wind comes from where the stream sets to.
pub fn wind_against_tide(tws: f64, twd: f64, (ce, cn): (f64, f64)) -> bool {
    tws >= WIND_AGAINST_TIDE_KN && ce.hypot(cn) >= TIDE_AGAINST_WIND_KN && {
        let d = (twd - crate::geo::set_of(ce, cn)).rem_euclid(360.0);
        d.min(360.0 - d) <= AGAINST_DEG
    }
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub hours: f64,
    pub distance_nm: f64,
    /// Great-circle distance from the start to the end.
    pub direct_nm: f64,
    pub avg_sog_kn: f64,
    pub max_sog_kn: f64,
    pub mean_tws_kn: f64,
    pub max_tws_kn: f64,
    pub max_tws_at: Option<DateTime<Utc>>,
    /// Hours of wind below each limit of `WIND_BANDS_KN`, and above the last.
    pub wind_hours: [f64; 4],
    /// Hours under sail on each point of `PointOfSail::ALL`.
    pub sail_hours: [f64; 5],
    pub motor_hours: f64,
    pub dark_hours: f64,
    pub tacks: u32,
    pub gybes: u32,
    /// Some leg met a current.
    pub has_current: bool,
    /// Hours with the current along the track at least `FAIR_FOUL_KN` with the boat, and against it.
    pub fair_hours: f64,
    pub foul_hours: f64,
    /// What the current gave (or, negative, took) along the track, nautical miles.
    pub current_gain_nm: f64,
    pub max_current_kn: f64,
    pub max_current_at: Option<DateTime<Utc>>,
    /// Hours of wind against tide (`wind_against_tide`).
    pub wind_against_tide_hours: f64,
    /// Hours spent waiting for the stream to turn (`Node::holding`).
    pub hold_hours: f64,
}

impl Stats {
    pub fn of(path: &[Node]) -> Stats {
        let mut s = Stats::default();
        let (Some(first), Some(last)) = (path.first(), path.last()) else { return s };
        s.direct_nm = first.pos.distance_nm(&last.pos);
        let mut wind_sum = 0.0;
        // Which side the wind was on at the last leg whose side was clear.
        let mut side: Option<f64> = None;
        for w in path.windows(2) {
            let (a, b) = (&w[0], &w[1]);
            let h = (b.time - a.time).num_milliseconds() as f64 / 3_600_000.0;
            if h <= 0.0 {
                continue;
            }
            let mid = a.time + (b.time - a.time) / 2;
            s.hours += h;
            s.distance_nm += a.pos.distance_nm(&b.pos);
            s.max_sog_kn = s.max_sog_kn.max(b.sog_kn);
            wind_sum += b.tws_kn * h;
            if b.tws_kn > s.max_tws_kn {
                s.max_tws_kn = b.tws_kn;
                s.max_tws_at = Some(mid);
            }
            s.wind_hours[WIND_BANDS_KN.iter().position(|&limit| b.tws_kn < limit).unwrap_or(WIND_BANDS_KN.len())] += h;
            let (ce, cn) = b.current;
            if ce != 0.0 || cn != 0.0 {
                s.has_current = true;
                let (sc, cc) = b.cog.to_radians().sin_cos();
                let along = ce * sc + cn * cc;
                s.current_gain_nm += along * h;
                if along >= FAIR_FOUL_KN {
                    s.fair_hours += h;
                } else if along <= -FAIR_FOUL_KN {
                    s.foul_hours += h;
                }
                let kn = ce.hypot(cn);
                if kn > s.max_current_kn {
                    s.max_current_kn = kn;
                    s.max_current_at = Some(mid);
                }
                if wind_against_tide(b.tws_kn, b.twd, b.current) {
                    s.wind_against_tide_hours += h;
                }
            }
            if sun_elevation_deg(&a.pos.destination(a.pos.bearing_to(&b.pos), a.pos.distance_nm(&b.pos) / 2.0), mid) < DARK_BELOW_DEG {
                s.dark_hours += h;
            }
            // Waiting is no point of sail, and no tack.
            if b.holding() {
                s.hold_hours += h;
                continue;
            }
            let angle = signed_angle(b.twd, b.heading);
            if b.motoring {
                s.motor_hours += h;
                continue;
            }
            s.sail_hours[PointOfSail::ALL.iter().position(|p| *p == PointOfSail::of(angle.abs())).unwrap_or(0)] += h;
            // Straight into or away from the wind there is no telling which side.
            if angle.abs() > 5.0 && angle.abs() < 175.0 {
                if let Some(before) = side.filter(|before| before.signum() != angle.signum()) {
                    if (before.abs() + angle.abs()) / 2.0 < 90.0 {
                        s.tacks += 1;
                    } else {
                        s.gybes += 1;
                    }
                }
                side = Some(angle);
            }
        }
        if s.hours > 0.0 {
            s.avg_sog_kn = s.distance_nm / s.hours;
            s.mean_tws_kn = wind_sum / s.hours;
        }
        s
    }

    pub fn sail_total(&self) -> f64 {
        self.sail_hours.iter().sum()
    }

    /// Hours with the wind at or above `kn`, to the nearest band limit.
    pub fn hours_over_kn(&self, kn: f64) -> f64 {
        let first = WIND_BANDS_KN.iter().position(|&l| l > kn).unwrap_or(WIND_BANDS_KN.len());
        self.wind_hours[first..].iter().sum()
    }

    /// Distance sailed over the straight line, as a percentage extra.
    pub fn extra_percent(&self) -> f64 {
        if self.direct_nm > 0.0 { (self.distance_nm / self.direct_nm - 1.0) * 100.0 } else { 0.0 }
    }
}

/// Degrees from the bow to the wind, -180 to 180: positive with the wind
/// from one side, negative from the other.
pub(crate) fn signed_angle(twd: f64, heading: f64) -> f64 {
    (twd - heading + 540.0).rem_euclid(360.0) - 180.0
}

/// How high the sun is, in degrees, at `pos` and `t` (a low-precision
/// almanac good to about a quarter of a degree).
pub fn sun_elevation_deg(pos: &LatLon, t: DateTime<Utc>) -> f64 {
    let n = t.timestamp_millis() as f64 / 86_400_000.0 + 2_440_587.5 - 2_451_545.0;
    let l = (280.460 + 0.985_647_4 * n).rem_euclid(360.0);
    let g = (357.528 + 0.985_600_3 * n).rem_euclid(360.0).to_radians();
    let lambda = (l + 1.915 * g.sin() + 0.020 * (2.0 * g).sin()).to_radians();
    let eps = (23.439 - 0.000_000_4 * n).to_radians();
    let ra = (eps.cos() * lambda.sin()).atan2(lambda.cos());
    let dec = (eps.sin() * lambda.sin()).asin();
    let gmst = (280.460_618_37 + 360.985_647_366_29 * n).rem_euclid(360.0);
    let ha = (gmst + pos.lon).to_radians() - ra;
    let lat = pos.lat.to_radians();
    (lat.sin() * dec.sin() + lat.cos() * dec.cos() * ha.cos()).asin().to_degrees()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn t0() -> DateTime<Utc> {
        // Noon at longitude 0 on the June solstice.
        Utc.with_ymd_and_hms(2026, 6, 21, 12, 0, 0).unwrap()
    }

    /// A leg of one hour per node, heading east along the equator at 5 kn.
    fn leg(i: i64, heading: f64, twd: f64, tws: f64, motoring: bool) -> Node {
        let pos = LatLon::new(0.0, 0.0).destination(90.0, 5.0 * i as f64);
        Node { pos, time: t0() + Duration::hours(i), cog: 90.0, sog_kn: 5.0, heading, tws_kn: tws, twd, motoring, current: (0.0, 0.0) }
    }

    fn path(legs: &[(f64, f64, f64, bool)]) -> Vec<Node> {
        std::iter::once(leg(0, 90.0, 0.0, 0.0, false)).chain(legs.iter().enumerate().map(|(i, l)| leg(i as i64 + 1, l.0, l.1, l.2, l.3))).collect()
    }

    #[test]
    fn the_sun_is_up_at_noon_and_down_at_midnight() {
        let here = LatLon::new(0.0, 0.0);
        let noon = sun_elevation_deg(&here, t0());
        // On the solstice the sun is 23.4 degrees from overhead on the equator.
        assert!((noon - 66.6).abs() < 1.0, "{noon}");
        assert!(sun_elevation_deg(&here, t0() + Duration::hours(12)) < -60.0);
        // Midnight sun far enough north.
        assert!(sun_elevation_deg(&LatLon::new(75.0, 0.0), t0() + Duration::hours(12)) > 0.0);
    }

    #[test]
    fn points_of_sail_split_the_time_and_engine_hours_are_apart() {
        // Wind from 90 degrees off the bow, then from dead astern, then under engine.
        let p = path(&[(90.0, 180.0, 12.0, false), (90.0, 180.0, 12.0, false), (90.0, 270.0, 8.0, false), (90.0, 90.0, 4.0, true)]);
        let s = Stats::of(&p);
        assert!((s.hours - 4.0).abs() < 1e-9);
        assert!((s.sail_hours[2] - 2.0).abs() < 1e-9, "beam reach {:?}", s.sail_hours);
        assert!((s.sail_hours[4] - 1.0).abs() < 1e-9, "run {:?}", s.sail_hours);
        assert!((s.motor_hours - 1.0).abs() < 1e-9);
        assert!((s.sail_total() + s.motor_hours - s.hours).abs() < 1e-9);
    }

    #[test]
    fn wind_figures_are_by_time_not_by_leg() {
        let p = path(&[(90.0, 0.0, 8.0, false), (90.0, 0.0, 24.0, false), (90.0, 0.0, 32.0, false)]);
        let s = Stats::of(&p);
        assert_eq!(s.max_tws_kn, 32.0);
        assert_eq!(s.max_tws_at, Some(t0() + Duration::minutes(150)));
        assert!((s.mean_tws_kn - 64.0 / 3.0).abs() < 1e-9);
        assert_eq!(s.wind_hours, [1.0, 0.0, 1.0, 1.0]);
        assert_eq!(s.hours_over_kn(20.0), 2.0);
        assert_eq!(s.hours_over_kn(30.0), 1.0);
        assert_eq!(s.hours_over_kn(0.0), 3.0);
    }

    #[test]
    fn a_change_of_side_is_a_tack_upwind_and_a_gybe_downwind() {
        // Wind from the north-east, then north-west, close to the wind both times.
        let upwind = path(&[(0.0, 40.0, 12.0, false), (0.0, 40.0, 12.0, false), (80.0, 40.0, 12.0, false)]);
        let s = Stats::of(&upwind);
        assert_eq!((s.tacks, s.gybes), (1, 0), "{s:?}");
        // Running with the wind first on one quarter, then the other.
        let downwind = path(&[(90.0, 250.0, 12.0, false), (90.0, 250.0, 12.0, false), (90.0, 290.0, 12.0, false)]);
        let s = Stats::of(&downwind);
        assert_eq!((s.tacks, s.gybes), (0, 1), "{s:?}");
    }

    #[test]
    fn dead_ahead_and_dead_astern_do_not_count_as_a_side() {
        let p = path(&[(0.0, 45.0, 10.0, false), (0.0, 0.0, 10.0, false), (0.0, 45.0, 10.0, false)]);
        assert_eq!(Stats::of(&p).tacks, 0);
    }

    #[test]
    fn dark_hours_follow_the_sun() {
        // Eight hours from 20:00 to 04:00 at the equator is all dark except the ends.
        let mut p = path(&[(90.0, 0.0, 10.0, false); 8]);
        for (i, n) in p.iter_mut().enumerate() {
            n.time = Utc.with_ymd_and_hms(2026, 3, 20, 20, 0, 0).unwrap() + Duration::hours(i as i64);
        }
        let s = Stats::of(&p);
        assert!(s.dark_hours > 6.5 && s.dark_hours <= 8.0, "{}", s.dark_hours);
        assert_eq!(Stats::of(&path(&[(90.0, 0.0, 10.0, false)])).dark_hours, 0.0);
    }

    #[test]
    fn the_extra_distance_compares_with_the_straight_line() {
        let mut p = path(&[(90.0, 0.0, 10.0, false), (90.0, 0.0, 10.0, false)]);
        // Detour north in the middle.
        p[1].pos = LatLon::new(0.0, 0.0).destination(45.0, 7.0);
        let s = Stats::of(&p);
        assert!(s.extra_percent() > 0.0, "{}", s.extra_percent());
        assert_eq!(Stats::of(&[]).hours, 0.0);
    }

    #[test]
    fn a_current_along_the_track_is_fair_or_foul_and_counts_what_it_gave() {
        // Three hourly legs east: 2 kn with the boat, 2 kn against, then 0.2 kn across.
        let mut p = path(&[(90.0, 0.0, 10.0, false); 3]);
        p[1].current = (2.0, 0.0);
        p[2].current = (-2.0, 0.0);
        p[3].current = (0.0, 0.2);
        let s = Stats::of(&p);
        assert!(s.has_current);
        assert!((s.fair_hours - 1.0).abs() < 1e-9 && (s.foul_hours - 1.0).abs() < 1e-9, "{} {}", s.fair_hours, s.foul_hours);
        assert!(s.current_gain_nm.abs() < 1e-9 && (s.max_current_kn - 2.0).abs() < 1e-9);
        assert!(!Stats::of(&path(&[(90.0, 0.0, 10.0, false)])).has_current);
    }

    #[test]
    fn a_hold_is_time_at_sea_but_no_point_of_sail() {
        let mut p = path(&[(90.0, 0.0, 10.0, false); 3]);
        p[2].sog_kn = 0.0;
        p[2].pos = p[1].pos;
        p[2].current = (-2.0, 0.0);
        let s = Stats::of(&p);
        assert!((s.hold_hours - 1.0).abs() < 1e-9 && (s.hours - 3.0).abs() < 1e-9);
        assert!((s.sail_hours.iter().sum::<f64>() - 2.0).abs() < 1e-9, "the two legs sailed");
        assert_eq!(Stats::of(&path(&[(90.0, 0.0, 10.0, false)])).hold_hours, 0.0);
    }

    #[test]
    fn wind_against_tide_is_wind_from_where_a_strong_stream_sets() {
        // A stream setting west at 3 kn: a westerly against it, an easterly with it.
        assert!(wind_against_tide(20.0, 270.0, (-3.0, 0.0)));
        assert!(wind_against_tide(20.0, 230.0, (-3.0, 0.0)));
        assert!(!wind_against_tide(20.0, 90.0, (-3.0, 0.0)));
        assert!(!wind_against_tide(10.0, 270.0, (-3.0, 0.0)), "too little wind");
        assert!(!wind_against_tide(20.0, 270.0, (-1.0, 0.0)), "too little stream");
        let mut p = path(&[(0.0, 270.0, 20.0, false); 2]);
        p[2].current = (-3.0, 0.0);
        assert!((Stats::of(&p).wind_against_tide_hours - 1.0).abs() < 1e-9);
    }
}
