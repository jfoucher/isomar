//! Navigation calculations: legs, cross-track error, VMG, ETAs, and the
//! active-route state machine.

use super::model::{Route, RoutePoint};
use crate::geo::{self, LatLon, normalize_bearing};
use chrono::{DateTime, Duration, Utc};

const R_NM: f64 = geo::EARTH_RADIUS_M / geo::NM_M;

/// Distance (nm) and bearing (degrees true) of a leg: a rhumb line, the
/// straight line the chart draws.
pub fn leg(from: &LatLon, to: &LatLon) -> (f64, f64) {
    from.rhumb_to(to)
}

/// Signed cross-track distance of `pos` from the great-circle line `from`
/// to `to`, in nautical miles. Positive means right of the track.
pub fn cross_track_nm(from: &LatLon, to: &LatLon, pos: &LatLon) -> f64 {
    let d13 = from.distance_nm(pos) / R_NM;
    if d13 == 0.0 {
        return 0.0;
    }
    let t13 = from.bearing_to(pos).to_radians();
    let t12 = from.bearing_to(to).to_radians();
    (d13.sin() * (t13 - t12).sin()).asin() * R_NM
}

/// Distance along the leg from `from` to the point nearest `pos`, in nautical
/// miles. Negative when `pos` is behind the start.
pub fn along_track_nm(from: &LatLon, to: &LatLon, pos: &LatLon) -> f64 {
    let d13 = from.distance_nm(pos) / R_NM;
    let xt = cross_track_nm(from, to, pos) / R_NM;
    let t13 = from.bearing_to(pos).to_radians();
    let t12 = from.bearing_to(to).to_radians();
    let along = (d13.cos() / xt.cos().max(1e-12)).clamp(-1.0, 1.0).acos() * R_NM;
    if (t13 - t12).cos() < 0.0 { -along } else { along }
}

/// Distance in nm from `pos` to the nearest point of the segment `a`-`b`
/// (planar approximation scaled to nautical miles; fine for chart-scale hit
/// testing).
pub fn distance_to_segment_nm(a: &LatLon, b: &LatLon, pos: &LatLon) -> f64 {
    let k = pos.lat.to_radians().cos();
    let to_xy = |p: &LatLon| ((p.lon - pos.lon) * k * 60.0, (p.lat - pos.lat) * 60.0);
    let (ax, ay) = to_xy(a);
    let (bx, by) = to_xy(b);
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { ((-ax) * dx + (-ay) * dy) / len2 }.clamp(0.0, 1.0);
    let (cx, cy) = (ax + t * dx, ay + t * dy);
    (cx * cx + cy * cy).sqrt()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Leg {
    pub from: usize,
    pub to: usize,
    pub distance_nm: f64,
    pub bearing: f64,
    /// Time of arrival at `to`, when the route has a departure time.
    pub eta: Option<DateTime<Utc>>,
}

/// All legs of a route with distances and, if a departure is set, ETAs at the
/// planned speed.
pub fn legs(route: &Route) -> Vec<Leg> {
    let mut out = Vec::new();
    let mut t = route.departure;
    for (i, w) in route.points.windows(2).enumerate() {
        let (d, b) = leg(&w[0].pos, &w[1].pos);
        if let (Some(cur), true) = (t, route.speed_kn > 0.0) {
            t = Some(cur + hours(d / route.speed_kn));
        } else {
            t = None;
        }
        out.push(Leg { from: i, to: i + 1, distance_nm: d, bearing: b, eta: t });
    }
    out
}

pub fn total_distance_nm(route: &Route) -> f64 {
    legs(route).iter().map(|l| l.distance_nm).sum()
}

fn hours(h: f64) -> Duration {
    Duration::milliseconds((h * 3_600_000.0) as i64)
}

/// The position fix navigation works from.
#[derive(Clone, Copy, Debug)]
pub struct Fix {
    pub pos: LatLon,
    pub sog_kn: Option<f64>,
    pub cog: Option<f64>,
    pub time: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NavData {
    /// Index of the point being steered to.
    pub target: usize,
    pub bearing: f64,
    pub range_nm: f64,
    /// Cross-track error; positive is right of the line.
    pub xte_nm: f64,
    /// Velocity made good towards the waypoint.
    pub vmg_kn: Option<f64>,
    pub ttg: Option<Duration>,
    pub eta: Option<DateTime<Utc>>,
    /// A waypoint was reached by this update.
    pub arrived: bool,
    /// The last waypoint was reached.
    pub finished: bool,
}

/// A route being followed, or a GoTo.
#[derive(Clone, Debug)]
pub struct ActiveRoute {
    pub route: Route,
    /// Index of the point being steered to (at least 1).
    pub target: usize,
    /// Where the current leg starts: the previous point, or for the first
    /// leg the position at activation.
    pub leg_start: LatLon,
    pub finished: bool,
}

impl ActiveRoute {
    /// Follows `route` from `start`, towards the first point (when `start` is
    /// given and is not itself the first point) or the second.
    pub fn new(route: Route, start: Option<LatLon>) -> Option<ActiveRoute> {
        if route.points.len() < 2 {
            return None;
        }
        let (target, leg_start) = match start {
            Some(p) => (0, p),
            None => (1, route.points[0].pos),
        };
        Some(ActiveRoute { route, target, leg_start, finished: false })
    }

    /// A direct leg from the boat to `to`.
    pub fn goto(from: LatLon, to: RoutePoint) -> ActiveRoute {
        let mut r = Route::new(to.name.clone());
        r.points = vec![RoutePoint::new("start", from), to];
        ActiveRoute { route: r, target: 1, leg_start: from, finished: false }
    }

    pub fn target_point(&self) -> &RoutePoint {
        &self.route.points[self.target]
    }

    /// Recomputes steering data for a new fix, advancing to the next leg when
    /// the current waypoint is reached.
    pub fn update(&mut self, fix: &Fix) -> NavData {
        let target = self.target_point().clone();
        let (range, bearing) = leg(&fix.pos, &target.pos);
        let xte = cross_track_nm(&self.leg_start, &target.pos, &fix.pos);
        let vmg = match (fix.sog_kn, fix.cog) {
            (Some(s), Some(c)) => Some(s * (c - bearing).to_radians().cos()),
            _ => None,
        };
        let ttg = vmg.filter(|v| *v > 0.1).map(|v| hours(range / v));
        let eta = ttg.map(|t| fix.time + t);
        let arrived = range <= target.arrival_radius_nm;
        let mut data = NavData {
            target: self.target,
            bearing,
            range_nm: range,
            xte_nm: xte,
            vmg_kn: vmg,
            ttg,
            eta,
            arrived,
            finished: false,
        };
        if arrived && !self.finished {
            if self.target + 1 < self.route.points.len() {
                self.leg_start = target.pos;
                self.target += 1;
            } else {
                self.finished = true;
                data.finished = true;
            }
        }
        data
    }

    /// Skips to the following waypoint without arriving at this one.
    pub fn skip(&mut self) -> bool {
        if self.target + 1 < self.route.points.len() {
            self.leg_start = self.target_point().pos;
            self.target += 1;
            true
        } else {
            false
        }
    }
}

/// Rhumb range and bearing, formatted for a status line: `12.3 nm 245°`.
pub fn format_range_bearing(range_nm: f64, bearing: f64) -> String {
    let r = if range_nm < 0.1 {
        format!("{:.0} m", range_nm * geo::NM_M)
    } else if range_nm < 10.0 {
        format!("{range_nm:.2} nm")
    } else {
        format!("{range_nm:.1} nm")
    };
    format!("{r} {:03}°", (normalize_bearing(bearing).round() as i64) % 360)
}

/// `1 h 05 min`, `42 min`, `3 d 4 h`.
pub fn format_duration(d: Duration) -> String {
    let mins = d.num_minutes().max(0);
    let (days, hours, minutes) = (mins / 1440, (mins / 60) % 24, mins % 60);
    if days > 0 {
        format!("{days} d {hours} h")
    } else if hours > 0 {
        format!("{hours} h {minutes:02} min")
    } else {
        format!("{minutes} min")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, h, m, 0).unwrap()
    }

    #[test]
    fn xte_sign_and_size() {
        // Track due north along lon 0; a point 0.1 deg east is to starboard.
        let (a, b) = (LatLon::new(0.0, 0.0), LatLon::new(1.0, 0.0));
        let east = cross_track_nm(&a, &b, &LatLon::new(0.5, 0.1));
        let west = cross_track_nm(&a, &b, &LatLon::new(0.5, -0.1));
        assert!((east - 6.0).abs() < 0.05, "{east}");
        assert!((west + 6.0).abs() < 0.05, "{west}");
        assert!(cross_track_nm(&a, &b, &LatLon::new(0.5, 0.0)).abs() < 1e-6);
        assert_eq!(cross_track_nm(&a, &b, &a), 0.0);
    }

    #[test]
    fn along_track() {
        let (a, b) = (LatLon::new(0.0, 0.0), LatLon::new(1.0, 0.0));
        let mid = along_track_nm(&a, &b, &LatLon::new(0.5, 0.05));
        assert!((mid - 30.0).abs() < 0.1, "{mid}");
        assert!(along_track_nm(&a, &b, &LatLon::new(-0.2, 0.0)) < 0.0);
    }

    #[test]
    fn segment_distance_clamps_to_the_ends() {
        let (a, b) = (LatLon::new(0.0, 0.0), LatLon::new(0.0, 1.0));
        assert!((distance_to_segment_nm(&a, &b, &LatLon::new(0.1, 0.5)) - 6.0).abs() < 0.05);
        // Beyond the end, the nearest point is the end itself.
        assert!((distance_to_segment_nm(&a, &b, &LatLon::new(0.0, 1.1)) - 6.0).abs() < 0.05);
        // Degenerate segment.
        assert!((distance_to_segment_nm(&a, &a, &LatLon::new(0.1, 0.0)) - 6.0).abs() < 0.05);
    }

    #[test]
    fn route_legs_accumulate_etas() {
        let mut r = Route::new("r");
        r.speed_kn = 6.0;
        r.departure = Some(at(8, 0));
        for lat in [48.0, 48.1, 48.2] {
            r.points.push(RoutePoint::new("", LatLon::new(lat, -4.0)));
        }
        let l = legs(&r);
        assert_eq!(l.len(), 2);
        // 6 nm legs at 6 kn: one hour each (a degree is 60.04 nm on the sphere).
        let close = |a: Option<DateTime<Utc>>, b: DateTime<Utc>| (a.unwrap() - b).num_seconds().abs() < 10;
        assert!(close(l[0].eta, at(9, 0)));
        assert!(close(l[1].eta, at(10, 0)));
        assert!((total_distance_nm(&r) - 12.0).abs() < 0.05);
        r.departure = None;
        assert!(legs(&r).iter().all(|l| l.eta.is_none()));
        r.departure = Some(at(8, 0));
        r.speed_kn = 0.0;
        assert!(legs(&r).iter().all(|l| l.eta.is_none()));
    }

    fn fix(lat: f64, lon: f64, sog: f64, cog: f64) -> Fix {
        Fix { pos: LatLon::new(lat, lon), sog_kn: Some(sog), cog: Some(cog), time: at(12, 0) }
    }

    fn route_north() -> Route {
        let mut r = Route::new("n");
        for lat in [48.0, 48.1, 48.2] {
            r.points.push(RoutePoint::new(format!("{lat}"), LatLon::new(lat, -4.0)));
        }
        r
    }

    #[test]
    fn active_route_steers_and_advances() {
        let mut ar = ActiveRoute::new(route_north(), None).unwrap();
        assert_eq!(ar.target, 1);
        let d = ar.update(&fix(48.0, -4.0, 6.0, 0.0));
        assert!((d.range_nm - 6.0).abs() < 0.05 && d.bearing < 0.5 || d.bearing > 359.5);
        assert!((d.vmg_kn.unwrap() - 6.0).abs() < 0.01);
        assert!((d.ttg.unwrap().num_seconds() - 3600).abs() < 10);
        assert!((d.eta.unwrap() - at(13, 0)).num_seconds().abs() < 10);
        assert!(!d.arrived);
        // Within the arrival radius: advance to the last point.
        let d = ar.update(&fix(48.0999, -4.0, 6.0, 0.0));
        assert!(d.arrived && !d.finished);
        assert_eq!(ar.target, 2);
        assert!((ar.leg_start.lat - 48.1).abs() < 1e-9);
        // And finish.
        let d = ar.update(&fix(48.2, -4.0, 6.0, 0.0));
        assert!(d.finished && ar.finished);
    }

    #[test]
    fn vmg_is_negative_when_heading_away_and_ttg_absent() {
        let mut ar = ActiveRoute::new(route_north(), None).unwrap();
        let d = ar.update(&fix(48.0, -4.0, 5.0, 180.0));
        assert!(d.vmg_kn.unwrap() < -4.9);
        assert!(d.ttg.is_none() && d.eta.is_none());
        let mut f = fix(48.0, -4.0, 5.0, 0.0);
        f.sog_kn = None;
        assert!(ar.update(&f).vmg_kn.is_none());
    }

    #[test]
    fn xte_is_measured_from_the_leg_start() {
        let mut ar = ActiveRoute::new(route_north(), None).unwrap();
        let d = ar.update(&fix(48.05, -3.99, 5.0, 0.0));
        assert!(d.xte_nm > 0.3 && d.xte_nm < 0.5, "{}", d.xte_nm);
    }

    #[test]
    fn activating_from_a_position_steers_to_the_first_point() {
        let ar = ActiveRoute::new(route_north(), Some(LatLon::new(47.9, -4.0))).unwrap();
        assert_eq!(ar.target, 0);
        assert!(ActiveRoute::new(Route::new("empty"), None).is_none());
    }

    #[test]
    fn goto_is_a_two_point_route() {
        let mut ar = ActiveRoute::goto(LatLon::new(48.0, -4.0), RoutePoint::new("dest", LatLon::new(48.1, -4.0)));
        assert_eq!(ar.route.points.len(), 2);
        assert!(!ar.skip(), "nothing to skip to");
        let d = ar.update(&fix(48.0, -4.0, 5.0, 0.0));
        assert!((d.range_nm - 6.0).abs() < 0.05);
    }

    #[test]
    fn skip_moves_to_the_next_point() {
        let mut ar = ActiveRoute::new(route_north(), None).unwrap();
        assert!(ar.skip());
        assert_eq!(ar.target, 2);
        assert!((ar.leg_start.lat - 48.1).abs() < 1e-9);
    }

    #[test]
    fn formatting() {
        assert_eq!(format_range_bearing(0.05, 90.0), "93 m 090°");
        assert_eq!(format_range_bearing(2.345, 5.0), "2.35 nm 005°");
        assert_eq!(format_range_bearing(123.45, 359.6), "123.5 nm 000°");
        assert_eq!(format_duration(Duration::minutes(42)), "42 min");
        assert_eq!(format_duration(Duration::minutes(65)), "1 h 05 min");
        assert_eq!(format_duration(Duration::hours(52)), "2 d 4 h");
        assert_eq!(format_duration(Duration::minutes(-5)), "0 min");
    }
}
