//! What the window's chrome says about the app: the title's subtitle and connection light, the jobs
//! in the activity indicator, and the mode (planning or under way).

use crate::app::AppState;
use crate::chips::{Chip, Icon};
use crate::core::comm::source::Status;
use crate::i18n::{gettext, ngettext_f};
use serde::Serialize;

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Link {
    /// Data is arriving.
    Ok,
    /// Connected, but nothing recent, or a connection dropped.
    Stale,
    /// No data source.
    None,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct JobRow {
    pub id: u64,
    /// The action that started it (`jobs`): its button shows it running.
    pub action: String,
    pub title: String,
    pub fraction: Option<f64>,
    pub detail: String,
    pub cancellable: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Window {
    pub title: String,
    /// "[speed] 6.0 kn  NMEA connected".
    pub subtitle: Vec<Chip>,
    pub link: Link,
    pub jobs: Vec<JobRow>,
    /// The under-way layout is on.
    pub underway: bool,
    pub offline: bool,
    pub recording: bool,
    /// The machine should not sleep: data is arriving or an anchor watch is set.
    pub keep_awake: bool,
    /// Something must go on with the app out of sight (a phone's background): an anchor watch, a route
    /// being followed or a track being recorded.
    pub watching: bool,
    /// The view keeps the boat centred (the touch shells' follow button; panning ends it).
    pub following: bool,
    /// A weather-route calculation is running: its "Compute" button is off.
    pub routing: bool,
    /// What was last selected on the chart, which a touch shell opens in its sheet (a desktop shows the
    /// selection on the chart and in the sidebar's lists).
    pub picked: Option<Picked>,
}

/// A mark, a route (or one of its points) or a vessel selected on the chart. `n` counts selections, so the
/// same mark tapped again is a new pick.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Picked {
    /// "waypoint", "route" or "vessel".
    pub kind: &'static str,
    pub id: i64,
    pub n: u64,
}

fn picked(app: &AppState) -> Option<Picked> {
    use crate::app::Selection;
    let (kind, id) = match app.selected.get() {
        Selection::None => return None,
        Selection::Waypoint(id) => ("waypoint", id),
        Selection::Route(id) | Selection::RoutePoint { route: id, .. } => ("route", id),
        Selection::Target(mmsi) => ("vessel", mmsi as i64),
    };
    Some(Picked { kind, id, n: app.selections.get() })
}

/// The connection light and its words: the worst of the configured connections.
pub fn link(app: &AppState) -> (Link, Vec<Chip>) {
    let s = app.session.borrow();
    let said = |t: String| vec![Chip::plain(t)];
    let now = chrono::Utc::now();
    let fresh = s.own.pos_updated.is_some_and(|t| now - t < chrono::Duration::seconds(30));
    if s.conn_status.is_empty() {
        // The phone's or tablet's own GPS (source "device") is no connection, but it is data arriving:
        // the light is green and the speed says enough.
        if fresh {
            return (Link::Ok, Vec::new());
        }
        return (Link::None, said(gettext("No data source")));
    }
    let mut connected = 0;
    let mut lost = 0;
    for st in s.conn_status.values() {
        match st {
            Status::Connected => connected += 1,
            Status::Disconnected(_) => lost += 1,
            Status::Connecting | Status::Finished => {}
        }
    }
    match (connected, lost) {
        (c, 0) if c > 0 && fresh => (Link::Ok, said(gettext("NMEA connected"))),
        // Data is arriving, but the receiver is still looking for satellites (indoors, a cold start).
        (c, _) if c > 0 && s.own.searching(now, chrono::Duration::seconds(30)) => (Link::Stale, no_fix_text(s.own.satellites_in_view())),
        (c, _) if c > 0 => (Link::Stale, said(gettext("No recent data"))),
        (_, l) if l > 0 => (Link::Stale, said(gettext("Connection lost"))),
        _ => (Link::Stale, said(gettext("Connecting"))),
    }
}

/// "No GPS fix  [satellite] 8": what a receiver searching for a fix hears.
fn no_fix_text(in_view: Option<u32>) -> Vec<Chip> {
    let mut out = vec![Chip::plain(gettext("No GPS fix"))];
    match in_view {
        None => {}
        Some(0) => out.push(Chip::new(Icon::Satellite, "0", gettext("no satellites in view"))),
        Some(n) => out.push(Chip::new(Icon::Satellite, n.to_string(), ngettext_f("{n} satellite in view", "{n} satellites in view", n, &[("n", &n.to_string())]))),
    }
    out
}

pub fn window(app: &AppState) -> Window {
    let (link, words) = link(app);
    let speed = crate::host::subtitle(app);
    let subtitle = speed.into_iter().chain(words).collect();
    Window {
        title: gettext("Isomar"),
        subtitle,
        link,
        jobs: app.jobs.snapshot(),
        underway: app.canvas.underway(),
        offline: crate::debug::offline(),
        recording: app.session.borrow().is_recording(),
        keep_awake: link != Link::None || app.session.borrow().anchor.is_some(),
        following: app.canvas.following(),
        watching: {
            let s = app.session.borrow();
            s.anchor.is_some() || s.active.is_some() || s.is_recording()
        },
        routing: app.routing_busy(),
        picked: picked(app),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chips::plain;
    use crate::core::nav::store::NavStore;
    use crate::core::session::Session;
    use crate::engine::Engine;
    use crate::palette::Mode;

    fn app() -> std::rc::Rc<AppState> {
        let canvas = Engine::new(crate::config::ViewState { lat: 48.0, lon: -4.0, zoom: 10.0 }, Mode::Day);
        AppState::new(canvas, Session::new(NavStore::open_memory().unwrap()).unwrap(), crate::config::Settings::default())
    }

    #[test]
    fn no_connection_is_no_data_source_and_jobs_are_listed() {
        let app = app();
        let w = window(&app);
        assert_eq!((w.link, plain(&w.subtitle).as_str()), (Link::None, "No data source"));
        let _job = app.jobs.start("weather", "Downloading weather", None);
        assert_eq!(window(&app).jobs.len(), 1);
    }

    #[test]
    fn a_connected_source_with_fresh_data_is_ok() {
        let app = app();
        {
            let mut s = app.session.borrow_mut();
            s.conn_status.insert("gps".into(), Status::Connected);
            s.own.pos = Some(crate::core::geo::LatLon::new(48.0, -4.0));
            s.own.pos_updated = Some(chrono::Utc::now());
            s.own.sog_kn = Some(6.0);
        }
        let w = window(&app);
        assert_eq!(w.link, Link::Ok);
        assert_eq!(w.subtitle, vec![Chip::new(Icon::Speed, "6.0 kn", "6.0 kn over the ground"), Chip::plain("NMEA connected")]);
        app.session.borrow_mut().conn_status.insert("gps".into(), Status::Disconnected("x".into()));
        assert_eq!(window(&app).link, Link::Stale);
    }

    #[test]
    fn the_devices_own_gps_without_a_connection_is_green_and_says_only_the_speed() {
        let app = app();
        {
            let mut s = app.session.borrow_mut();
            s.own.pos = Some(crate::core::geo::LatLon::new(48.0, -4.0));
            s.own.pos_updated = Some(chrono::Utc::now());
            s.own.sog_kn = Some(2.2);
        }
        let w = window(&app);
        assert_eq!(w.link, Link::Ok);
        assert_eq!(w.subtitle, vec![Chip::new(Icon::Speed, "2.2 kn", "2.2 kn over the ground")]);
    }

    #[test]
    fn each_selection_is_a_new_pick_even_of_the_same_mark() {
        use crate::app::Selection;
        let app = app();
        assert_eq!(window(&app).picked, None);
        app.select(Selection::Waypoint(3));
        let first = window(&app).picked.unwrap();
        assert_eq!((first.kind, first.id), ("waypoint", 3));
        app.select(Selection::Waypoint(3));
        assert_ne!(window(&app).picked.unwrap(), first, "tapped again, it opens again");
        app.select(Selection::RoutePoint { route: 7, idx: 2 });
        let p = window(&app).picked.unwrap();
        assert_eq!((p.kind, p.id), ("route", 7));
    }

    #[test]
    fn a_receiver_without_a_fix_says_so_with_its_satellites() {
        let app = app();
        {
            let mut s = app.session.borrow_mut();
            s.conn_status.insert("gps".into(), Status::Connected);
            s.own.no_fix_at = Some(chrono::Utc::now());
        }
        assert_eq!(window(&app).subtitle, vec![Chip::plain("No GPS fix")]);
        app.session.borrow_mut().own.in_view.insert("GP".into(), 8);
        let w = window(&app);
        assert_eq!((w.link, w.subtitle[1].clone()), (Link::Stale, Chip::new(Icon::Satellite, "8", "8 satellites in view")));
    }
}
