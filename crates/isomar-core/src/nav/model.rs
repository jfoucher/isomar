//! Plain data for waypoints, routes and tracks.

use crate::geo::LatLon;
use chrono::{DateTime, Utc};

/// Default radius at which a waypoint counts as reached, in nautical miles.
pub const DEFAULT_ARRIVAL_RADIUS_NM: f64 = 0.05;
pub const DEFAULT_SPEED_KN: f64 = 5.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Waypoint {
    /// Database id; 0 until stored.
    pub id: i64,
    pub name: String,
    pub pos: LatLon,
    /// Symbol name, such as "anchorage" or "mark".
    pub icon: String,
    pub description: String,
    pub arrival_radius_nm: f64,
    pub visible: bool,
    pub time: Option<DateTime<Utc>>,
}

impl Waypoint {
    pub fn new(name: impl Into<String>, pos: LatLon) -> Waypoint {
        Waypoint {
            id: 0,
            name: name.into(),
            pos,
            icon: "mark".into(),
            description: String::new(),
            arrival_radius_nm: DEFAULT_ARRIVAL_RADIUS_NM,
            visible: true,
            time: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RoutePoint {
    pub name: String,
    pub pos: LatLon,
    pub arrival_radius_nm: f64,
}

impl RoutePoint {
    pub fn new(name: impl Into<String>, pos: LatLon) -> RoutePoint {
        RoutePoint { name: name.into(), pos, arrival_radius_nm: DEFAULT_ARRIVAL_RADIUS_NM }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub id: i64,
    pub name: String,
    pub points: Vec<RoutePoint>,
    pub speed_kn: f64,
    pub departure: Option<DateTime<Utc>>,
    pub visible: bool,
}

impl Route {
    pub fn new(name: impl Into<String>) -> Route {
        Route {
            id: 0,
            name: name.into(),
            points: Vec::new(),
            speed_kn: DEFAULT_SPEED_KN,
            departure: None,
            visible: true,
        }
    }

    pub fn reverse(&mut self) {
        self.points.reverse();
    }

    /// Numbers unnamed points `001`, `002`, ... in order.
    pub fn name_points(&mut self) {
        for (i, p) in self.points.iter_mut().enumerate() {
            if p.name.trim().is_empty() {
                p.name = format!("{:03}", i + 1);
            }
        }
    }

    /// Splits at point `i` into two routes sharing that point.
    pub fn split_at(&self, i: usize) -> Option<(Route, Route)> {
        if i == 0 || i + 1 >= self.points.len() {
            return None;
        }
        let mut a = self.clone();
        let mut b = self.clone();
        a.id = 0;
        b.id = 0;
        a.points.truncate(i + 1);
        b.points.drain(..i);
        a.name = format!("{} (1)", self.name);
        b.name = format!("{} (2)", self.name);
        Some((a, b))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TrackPoint {
    pub pos: LatLon,
    pub time: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub id: i64,
    pub name: String,
    pub points: Vec<TrackPoint>,
    pub visible: bool,
}

impl Track {
    pub fn new(name: impl Into<String>) -> Track {
        Track { id: 0, name: name.into(), points: Vec::new(), visible: true }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route3() -> Route {
        let mut r = Route::new("r");
        for (i, lat) in [48.0, 48.1, 48.2].iter().enumerate() {
            r.points.push(RoutePoint::new(format!("p{i}"), LatLon::new(*lat, -4.0)));
        }
        r
    }

    #[test]
    fn reverse_flips_the_order() {
        let mut r = route3();
        r.reverse();
        assert_eq!(r.points[0].name, "p2");
    }

    #[test]
    fn unnamed_points_get_numbers() {
        let mut r = Route::new("r");
        r.points.push(RoutePoint::new("", LatLon::new(0.0, 0.0)));
        r.points.push(RoutePoint::new("keep", LatLon::new(0.0, 1.0)));
        r.points.push(RoutePoint::new(" ", LatLon::new(0.0, 2.0)));
        r.name_points();
        let names: Vec<_> = r.points.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["001", "keep", "003"]);
    }

    #[test]
    fn split_shares_the_split_point() {
        let r = route3();
        let (a, b) = r.split_at(1).unwrap();
        assert_eq!(a.points.len(), 2);
        assert_eq!(b.points.len(), 2);
        assert_eq!(a.points[1], b.points[0]);
        assert!(r.split_at(0).is_none());
        assert!(r.split_at(2).is_none());
    }
}
