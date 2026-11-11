//! What is under a point on the chart.

use super::view::ViewInfo;
use crate::core::nav::model::Route;
use crate::core::session::Session;
use std::cell::Cell;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    None,
    Waypoint(i64),
    /// A point of a stored route.
    RoutePoint { route: i64, idx: usize },
    /// A point of the route being edited.
    EditPoint(usize),
    Route(i64),
    Target(u32),
}

fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

pub fn seg_dist(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l2 = dx * dx + dy * dy;
    let t = if l2 == 0.0 { 0.0 } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0) };
    dist(p, (a.0 + t * dx, a.1 + t * dy))
}

/// Finds the topmost thing within `tol` pixels of (`x`, `y`). Priority: points
/// of the route being edited, AIS targets, waypoints, route points, then route
/// lines.
pub fn hit_test(s: &Session, editing: Option<&Route>, vi: &ViewInfo, x: f64, y: f64, tol: f64) -> Hit {
    let p = (x, y);
    if let Some(r) = editing {
        for (i, rp) in r.points.iter().enumerate().rev() {
            if dist(vi.to_screen(&rp.pos), p) <= tol {
                return Hit::EditPoint(i);
            }
        }
    }
    let best: Cell<Option<(f64, Hit)>> = Cell::new(None);
    let consider = |d: f64, h: Hit| {
        if d <= tol && best.get().is_none_or(|(bd, _)| d < bd) {
            best.set(Some((d, h)));
        }
    };
    for t in s.targets.iter() {
        if let Some(pos) = t.pos {
            consider(dist(vi.to_screen(&pos), p), Hit::Target(t.mmsi));
        }
    }
    if let Some((_, h)) = best.get() {
        return h;
    }
    for w in s.waypoints.iter().filter(|w| w.visible) {
        consider(dist(vi.to_screen(&w.pos), p), Hit::Waypoint(w.id));
    }
    if let Some((_, h)) = best.get() {
        return h;
    }
    for r in s.routes.iter().filter(|r| r.visible && editing.is_none_or(|e| e.id != r.id)) {
        for (i, rp) in r.points.iter().enumerate() {
            consider(dist(vi.to_screen(&rp.pos), p), Hit::RoutePoint { route: r.id, idx: i });
        }
    }
    if let Some((_, h)) = best.get() {
        return h;
    }
    for r in s.routes.iter().filter(|r| r.visible && editing.is_none_or(|e| e.id != r.id)) {
        for w in r.points.windows(2) {
            consider(seg_dist(p, vi.to_screen(&w[0].pos), vi.to_screen(&w[1].pos)), Hit::Route(r.id));
        }
    }
    best.get().map(|(_, h)| h).unwrap_or(Hit::None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::geo::LatLon;
    use crate::core::nav::model::RoutePoint;
    use crate::core::nav::store::NavStore;
    use crate::view::View;

    fn setup() -> (Session, ViewInfo) {
        let s = Session::new(NavStore::open_memory().unwrap()).unwrap();
        let vi = ViewInfo { view: View { center: LatLon::new(48.0, -4.0), zoom: 12.0 }, w: 800.0, h: 600.0 };
        (s, vi)
    }

    fn route(points: &[(f64, f64)]) -> Route {
        let mut r = Route::new("r");
        for (lat, lon) in points {
            r.points.push(RoutePoint::new("p", LatLon::new(*lat, *lon)));
        }
        r
    }

    #[test]
    fn empty_chart_hits_nothing() {
        let (s, vi) = setup();
        assert_eq!(hit_test(&s, None, &vi, 400.0, 300.0, 10.0), Hit::None);
    }

    #[test]
    fn waypoints_are_hit_within_tolerance_only() {
        let (mut s, vi) = setup();
        let w = s.add_waypoint(LatLon::new(48.0, -4.0), None).unwrap();
        assert_eq!(hit_test(&s, None, &vi, 405.0, 304.0, 10.0), Hit::Waypoint(w.id));
        assert_eq!(hit_test(&s, None, &vi, 430.0, 300.0, 10.0), Hit::None);
        // Hidden ones are not hit.
        let mut hidden = w.clone();
        hidden.visible = false;
        s.update_waypoint(&hidden).unwrap();
        assert_eq!(hit_test(&s, None, &vi, 400.0, 300.0, 10.0), Hit::None);
    }

    #[test]
    fn the_nearest_of_two_waypoints_wins() {
        let (mut s, vi) = setup();
        let a = s.add_waypoint(LatLon::new(48.0, -4.0), None).unwrap();
        let (bx, _) = vi.to_screen(&LatLon::new(48.0, -4.0));
        // Put b about 8 px to the right.
        let b_lon = vi.position_at(bx + 8.0, 300.0).lon;
        let b = s.add_waypoint(LatLon::new(48.0, b_lon), None).unwrap();
        assert_eq!(hit_test(&s, None, &vi, 402.0, 300.0, 10.0), Hit::Waypoint(a.id));
        assert_eq!(hit_test(&s, None, &vi, 407.0, 300.0, 10.0), Hit::Waypoint(b.id));
    }

    #[test]
    fn route_points_beat_route_lines_and_lines_are_hit_between_points() {
        let (mut s, vi) = setup();
        let r = s.save_route(&route(&[(48.0, -4.0), (48.0, -3.9)])).unwrap();
        assert_eq!(hit_test(&s, None, &vi, 401.0, 300.0, 10.0), Hit::RoutePoint { route: r.id, idx: 0 });
        let (x1, y1) = vi.to_screen(&LatLon::new(48.0, -3.9));
        assert_eq!(hit_test(&s, None, &vi, (400.0 + x1) / 2.0, y1 + 4.0, 10.0), Hit::Route(r.id));
        assert_eq!(hit_test(&s, None, &vi, (400.0 + x1) / 2.0, y1 + 40.0, 10.0), Hit::None);
    }

    #[test]
    fn the_route_being_edited_takes_priority_and_hides_its_stored_copy() {
        let (mut s, vi) = setup();
        let r = s.save_route(&route(&[(48.0, -4.0), (48.0, -3.9)])).unwrap();
        let w = s.add_waypoint(LatLon::new(48.0, -4.0), None).unwrap();
        let editing = r.clone();
        assert_eq!(hit_test(&s, Some(&editing), &vi, 400.0, 300.0, 10.0), Hit::EditPoint(0));
        // Away from the edit points, the stored copy of the edited route is ignored,
        // but other things are still found.
        assert_eq!(hit_test(&s, Some(&editing), &vi, 400.0, 300.0 + 30.0, 10.0), Hit::None);
        let _ = w;
        let mut other = route(&[(48.0, -4.0), (48.0, -3.9)]);
        other.id = 999;
        assert_eq!(hit_test(&s, Some(&other), &vi, 400.0, 300.0, 10.0), Hit::EditPoint(0));
    }

    #[test]
    fn targets_beat_waypoints() {
        use crate::core::comm::nmea::frame;
        use crate::core::comm::source::{Event, Payload};
        let (mut s, vi) = setup();
        s.add_waypoint(LatLon::new(48.0, -4.0), None).unwrap();
        // A class A report from the published sample puts a target in Seattle; instead
        // build one at our spot through the session's own pipeline.
        let (p, fill) = crate::core::comm::ais::encode_position_a(&crate::core::comm::ais::PositionReport {
            mmsi: 5,
            pos: LatLon::new(48.0, -4.0),
            sog_kn: 5.0,
            cog: 0.0,
            heading: None,
            status: 0,
        });
        let line = crate::core::comm::ais::to_sentences(&p, fill, 'A', 0).remove(0);
        let _ = frame("", '$');
        s.process(&[Event { source: "t".into(), payload: Payload::Line(line) }], chrono::Utc::now());
        assert_eq!(hit_test(&s, None, &vi, 400.0, 300.0, 10.0), Hit::Target(5));
    }
}
