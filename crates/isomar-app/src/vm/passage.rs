//! The Passage page: routes, waypoints and tracks, and what can be done to them.

use crate::app::AppState;
use crate::core::geo::LatLon;
use crate::core::nav::calc::{format_duration, format_range_bearing, total_distance_nm};
use crate::core::nav::model::{Route, Track};
use crate::core::vector::Bounds;
use crate::chips::{Chip, Icon};
use crate::i18n::{gettext_f, ngettext_f};
use serde::Serialize;

pub fn track_distance_nm(t: &Track) -> f64 {
    t.points.windows(2).map(|w| w[0].pos.distance_nm(&w[1].pos)).sum()
}

pub fn route_bounds(r: &Route) -> Option<Bounds> {
    let b = Bounds::of(&r.points.iter().map(|p| [p.pos.lon, p.pos.lat]).collect::<Vec<_>>());
    (!b.is_empty()).then_some(b)
}

pub fn track_bounds(t: &Track) -> Option<Bounds> {
    let b = Bounds::of(&t.points.iter().map(|p| [p.pos.lon, p.pos.lat]).collect::<Vec<_>>());
    (!b.is_empty()).then_some(b)
}

#[derive(Serialize, Debug, PartialEq)]
pub struct RouteRow {
    pub id: i64,
    pub name: String,
    pub detail: Vec<Chip>,
    /// A weather route's plan: when it leaves and when it arrives (or the forecast ends); empty
    /// for a route with no plan.
    pub planned: Vec<Chip>,
    /// Made by weather routing: it is not routed again.
    pub weather: bool,
    pub visible: bool,
    pub active: bool,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct WaypointRow {
    pub id: i64,
    pub name: String,
    pub detail: String,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct TrackRow {
    pub id: i64,
    pub name: String,
    pub detail: Vec<Chip>,
    pub visible: bool,
}

/// What the boat is following, when a route is active.
#[derive(Serialize, Debug, PartialEq)]
pub struct Following {
    pub name: String,
    pub detail: Vec<Chip>,
}

#[derive(Serialize, Debug, PartialEq, Default)]
pub struct Passage {
    pub routes: Vec<RouteRow>,
    pub waypoints: Vec<WaypointRow>,
    pub tracks: Vec<TrackRow>,
    pub recording: bool,
    pub following: Option<Following>,
}

/// How many points a route or track has and how long it is: "[points] 12  [distance] 68.2 nm".
pub fn points_and_distance(n: usize, nm: f64) -> Vec<Chip> {
    let d = format!("{nm:.1}");
    vec![
        Chip::new(Icon::Points, n.to_string(), ngettext_f("{n} point", "{n} points", n as u32, &[("n", &n.to_string())])),
        Chip::with(Icon::Distance, gettext_f("{d} nm", &[("d", &d)])),
    ]
}

/// When a weather route's plan leaves and arrives: "[date] Mon 03 Nov 06:00  [arrival] Tue 04 Nov 14:00".
pub fn plan_times(t: &crate::core::routing::plan::PlanTimes) -> Vec<Chip> {
    let when = |t: chrono::DateTime<chrono::Utc>| t.with_timezone(&chrono::Local).format("%a %d %b %H:%M").to_string();
    let (from, to) = (when(t.depart), when(t.end));
    let end = if t.arrives {
        Chip::new(Icon::Arrival, to.clone(), gettext_f("arrives {time}", &[("time", &to)]))
    } else {
        Chip::new(Icon::Time, to.clone(), gettext_f("forecast ends {time}", &[("time", &to)]))
    };
    vec![Chip::new(Icon::Date, from.clone(), gettext_f("leaves {time}", &[("time", &from)])), end]
}

pub fn passage(app: &AppState) -> Passage {
    let s = app.session.borrow();
    let active_id = s.active.as_ref().map(|a| a.route.id);
    let following = s.active.as_ref().map(|a| {
        let t = a.target_point();
        let mut detail = vec![Chip::plain(gettext_f("To {point}", &[("point", &if t.name.is_empty() { "—".to_string() } else { t.name.clone() })]))];
        if let Some(n) = s.nav.as_ref() {
            let rb = format_range_bearing(n.range_nm, n.bearing);
            detail.push(Chip::new(Icon::Distance, rb.clone(), gettext_f("{range} to go", &[("range", &rb)])));
            if let Some(ttg) = n.ttg {
                let t = format_duration(ttg);
                detail.push(Chip::new(Icon::Time, t.clone(), gettext_f("{time} to go", &[("time", &t)])));
            }
        }
        Following { name: a.route.name.clone(), detail }
    });
    Passage {
        routes: s
            .routes
            .iter()
            .map(|r| RouteRow {
                id: r.id,
                name: r.name.clone(),
                detail: points_and_distance(r.points.len(), total_distance_nm(r)),
                planned: s.plans.get(&r.id).map(plan_times).unwrap_or_default(),
                weather: s.plans.contains_key(&r.id),
                visible: r.visible,
                active: Some(r.id) == active_id,
            })
            .collect(),
        waypoints: s.waypoints.iter().map(|w| WaypointRow { id: w.id, name: w.name.clone(), detail: crate::text::position_text(&w.pos) }).collect(),
        tracks: s.tracks.iter().map(|t| TrackRow { id: t.id, name: t.name.clone(), detail: points_and_distance(t.points.len(), track_distance_nm(t)), visible: t.visible }).collect(),
        recording: s.is_recording(),
        following,
    }
}

pub fn zoom_to_route(app: &AppState, id: i64) {
    let b = app.session.borrow().routes.iter().find(|r| r.id == id).and_then(route_bounds);
    if let Some(b) = b {
        app.canvas.fit_bounds(&b);
    }
}

pub fn zoom_to_track(app: &AppState, id: i64) {
    let b = app.session.borrow().tracks.iter().find(|t| t.id == id).and_then(track_bounds);
    if let Some(b) = b {
        app.canvas.fit_bounds(&b);
    }
}

pub fn zoom_to_waypoint(app: &AppState, id: i64) {
    let p: Option<LatLon> = app.session.borrow().waypoints.iter().find(|w| w.id == id).map(|w| w.pos);
    if let Some(p) = p {
        app.canvas.center_on(p);
        app.select(crate::app::Selection::Waypoint(id));
    }
}

pub fn toggle_route_visible(app: &AppState, id: i64) {
    let r = app.session.borrow().routes.iter().find(|r| r.id == id).cloned();
    if let Some(mut r) = r {
        r.visible = !r.visible;
        let _ = app.session.borrow_mut().save_route(&r);
        app.refresh();
    }
}

pub fn toggle_track_visible(app: &AppState, id: i64) {
    let mut s = app.session.borrow_mut();
    if let Some(t) = s.tracks.iter_mut().find(|t| t.id == id) {
        t.visible = !t.visible;
        let t = t.clone();
        let _ = s.store.update_track_meta(&t);
    }
    drop(s);
    app.refresh();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::nav::model::{RoutePoint, TrackPoint};
    use crate::core::nav::store::NavStore;
    use crate::core::session::Session;
    use crate::engine::Engine;
    use crate::palette::Mode;

    fn app() -> std::rc::Rc<AppState> {
        let canvas = Engine::new(crate::config::ViewState { lat: 48.0, lon: -4.0, zoom: 10.0 }, Mode::Day);
        AppState::new(canvas, Session::new(NavStore::open_memory().unwrap()).unwrap(), crate::config::Settings::default())
    }

    #[test]
    fn bounds_and_distance_of_routes_and_tracks() {
        let mut r = Route::new("r");
        r.points.push(RoutePoint::new("a", LatLon::new(48.0, -4.0)));
        r.points.push(RoutePoint::new("b", LatLon::new(48.5, -3.0)));
        let b = route_bounds(&r).unwrap();
        assert_eq!((b.min_lat, b.max_lon), (48.0, -3.0));
        assert!(route_bounds(&Route::new("empty")).is_none());
        let mut t = Track::new("t");
        t.points.push(TrackPoint { pos: LatLon::new(0.0, 0.0), time: None });
        t.points.push(TrackPoint { pos: LatLon::new(1.0 / 60.0, 0.0), time: None });
        assert!((track_distance_nm(&t) - 1.0).abs() < 0.01);
        assert!(track_bounds(&Track::new("e")).is_none());
    }

    #[test]
    fn the_page_lists_what_is_stored() {
        let app = app();
        assert_eq!(passage(&app), Passage::default());
        let mut r = Route::new("Brest to Camaret");
        r.points.push(RoutePoint::new("a", LatLon::new(48.38, -4.49)));
        r.points.push(RoutePoint::new("b", LatLon::new(48.28, -4.59)));
        let saved = app.session.borrow_mut().save_route(&r).unwrap();
        app.session.borrow_mut().add_waypoint(LatLon::new(48.0, -4.0), Some("Mark".into())).unwrap();
        let p = passage(&app);
        assert_eq!(p.routes.len(), 1);
        assert_eq!(p.routes[0].id, saved.id);
        assert_eq!(p.routes[0].detail[0].label, "2 points");
        assert_eq!(p.waypoints[0].name, "Mark");
        assert!(p.following.is_none() && !p.recording);
        toggle_route_visible(&app, saved.id);
        assert!(!passage(&app).routes[0].visible);
    }
}
