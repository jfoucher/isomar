//! The under-way layout: the navigating card and the docked dashboard. The layout switches on by
//! itself when a route starts and off when navigation stops (`Settings::underway_auto`), and the user
//! can turn it on or off.

use crate::actions;
use crate::app::AppState;
use crate::chips::{Chip, Icon};
use crate::core::geo;
use crate::core::nav::calc::format_duration;
use crate::i18n::{gettext, gettext_f};
use chrono::Local;
use serde::Serialize;
use std::rc::Rc;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Cell {
    pub label: String,
    pub value: String,
    pub unit: String,
}

fn cell(label: &str, value: String, unit: &str) -> Cell {
    Cell { label: label.to_string(), value, unit: unit.to_string() }
}

/// The card with the next waypoint and the figures to steer by.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct NavCard {
    /// The number of the waypoint being steered to, as it is on the chart.
    pub number: usize,
    pub name: String,
    pub cells: Vec<Cell>,
    pub can_skip: bool,
}

/// One big readout of the docked dashboard.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Readout {
    pub id: String,
    pub label: String,
    pub value: String,
    pub unit: String,
    /// A line under it, such as "[satellite] 1 s" or "Safety depth 3.0 m".
    pub note: Vec<Chip>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Underway {
    pub on: bool,
    pub card: Option<NavCard>,
    pub dashboard: Vec<Readout>,
    /// The Sailing / Motoring switch, beside the dashboard under way.
    pub sail: Option<super::performance::Switch>,
}

/// The tiles of the default dashboard, in order.
pub const DEFAULT_DASHBOARD: [&str; 4] = ["depth", "sog", "cog", "tws"];

/// A boat with every instrument: what the store pictures show (`ISOMAR_DASHBOARD=full`).
pub const FULL_DASHBOARD: [&str; 11] = ["depth", "sog", "cog", "heading", "stw", "awa", "aws", "twa", "tws", "twd", "pilot"];

/// Every tile a dashboard can hold, with its name in the settings, in the order they are offered.
pub fn choices() -> Vec<(&'static str, String)> {
    vec![
        ("depth", gettext("Depth")),
        ("sog", gettext("Speed over ground")),
        ("cog", gettext("Course over ground")),
        ("heading", gettext("Heading")),
        ("stw", gettext("Speed through water")),
        ("awa", gettext("Apparent wind angle")),
        ("aws", gettext("Apparent wind speed")),
        ("twa", gettext("True wind angle")),
        ("tws", gettext("True wind speed")),
        ("twd", gettext("True wind direction")),
        ("pilot", gettext("Autopilot")),
        ("temp", gettext("Water temperature")),
        ("set", gettext("Set and drift")),
        ("pos", gettext("Position")),
    ]
}

fn bearing(v: f64) -> String {
    format!("{:03}°", (v.round() as i64).rem_euclid(360))
}

/// A wind angle as a figure and the side it is on: `("38°", "stbd")`.
fn angle(a: f64) -> (String, String) {
    (format!("{:.0}°", a.abs()), if a >= 0.0 { gettext("stbd") } else { gettext("port") })
}

/// Wraps an angle to -180..180.
fn wrap180(a: f64) -> f64 {
    let r = a.rem_euclid(360.0);
    if r > 180.0 { r - 360.0 } else { r }
}

/// "0.04 R": across-track error and the side of the line the boat is on.
fn xte(v: f64) -> String {
    format!("{:.2} {}", v.abs(), if v >= 0.0 { gettext("R") } else { gettext("L") })
}

pub fn card(app: &AppState) -> Option<NavCard> {
    let units = crate::units::Units::of(&app.settings.borrow());
    let s = app.session.borrow();
    let a = s.active.as_ref()?;
    let target = a.target_point();
    let dash = || "—".to_string();
    let nav = s.nav.as_ref();
    let cells = vec![
        cell(&gettext("BTW"), nav.map_or_else(dash, |n| bearing(n.bearing)), ""),
        cell(&gettext("DTW"), nav.map_or_else(dash, |n| format!("{:.1}", units.distance(n.range_nm).0)), units.distance(1.0).1),
        cell(&gettext("XTE"), nav.map_or_else(dash, |n| xte(n.xte_nm)), ""),
        cell(&gettext("VMG"), nav.and_then(|n| n.vmg_kn).map_or_else(dash, |v| format!("{:.1}", units.speed(v).0)), units.speed(1.0).1),
        cell(&gettext("TTG"), nav.and_then(|n| n.ttg).map_or_else(dash, format_duration), ""),
        cell(&gettext("ETA"), nav.and_then(|n| n.eta).map_or_else(dash, |t| t.with_timezone(&Local).format("%H:%M").to_string()), ""),
    ];
    Some(NavCard {
        number: a.target + 1,
        name: if target.name.is_empty() { format!("{}", a.target + 1) } else { target.name.clone() },
        cells,
        can_skip: a.target + 1 < a.route.points.len(),
    })
}

/// The readout of tile `id`, with how old its data is.
pub fn readout(app: &AppState, id: &str) -> Option<Readout> {
    let units = crate::units::Units::of(&app.settings.borrow());
    let s = app.session.borrow();
    let o = &s.own;
    let age = o.pos_updated.map(|t| (chrono::Utc::now() - t).num_seconds().max(0));
    let gps = || match age {
        None => vec![Chip::plain(gettext("No fix"))],
        Some(a) => {
            let a = a.to_string();
            vec![Chip::new(Icon::Satellite, gettext_f("{age} s", &[("age", &a)]), gettext_f("GPS fix {age} s old", &[("age", &a)]))]
        }
    };
    let dash = || "—".to_string();
    let r = |label: &str, value: String, unit: &str, note: Vec<Chip>| Readout { id: id.to_string(), label: label.to_string(), value, unit: unit.to_string(), note };
    let none = Vec::new;
    Some(match id {
        "depth" => {
            let unit = units.depth(1.0).1;
            let safety = units.depth(f64::from(app.settings.borrow().safety_depth)).0;
            r(&gettext("DEPTH"), o.depth_m.map_or_else(dash, |d| format!("{:.1}", units.depth(d).0)), unit, vec![Chip::plain(gettext("Safety depth {d} {u}").replace("{d}", &format!("{safety:.1}")).replace("{u}", unit))])
        }
        "sog" => r(&gettext("SOG"), o.sog_kn.map_or_else(dash, |v| format!("{:.1}", units.speed(v).0)), units.speed(1.0).1, gps()),
        "cog" => r(&gettext("COG"), o.cog.map_or_else(dash, bearing), "T", none()),
        "heading" => r(&gettext("HDG"), o.heading.map_or_else(dash, bearing), "T", none()),
        "tws" => r(&gettext("TWS"), o.wind_true.map_or_else(dash, |(_, v)| format!("{:.0}", units.speed(v).0)), units.speed(1.0).1, none()),
        "aws" => r(&gettext("AWS"), o.wind_apparent.map_or_else(dash, |(_, v)| format!("{:.0}", units.speed(v).0)), units.speed(1.0).1, none()),
        "awa" => {
            let (value, side) = o.wind_apparent.map_or_else(|| (dash(), String::new()), |(a, _)| angle(a));
            r(&gettext("AWA"), value, &side, none())
        }
        "twa" => {
            // The true wind off the bow: its direction less the heading (the course without a compass).
            let twa = o.wind_true.zip(o.heading.or(o.cog)).map(|((d, _), h)| wrap180(d - h));
            let (value, side) = twa.map_or_else(|| (dash(), String::new()), angle);
            r(&gettext("TWA"), value, &side, none())
        }
        "twd" => r(&gettext("TWD"), o.wind_true.map_or_else(dash, |(d, _)| bearing(d)), "T", none()),
        "pilot" => {
            use crate::core::comm::nmea::PilotMode;
            let p = o.autopilot.unwrap_or_default();
            let mode = match p.mode {
                Some(PilotMode::Standby) => gettext("Standby"),
                Some(PilotMode::Heading) => gettext("Auto"),
                Some(PilotMode::Wind) => gettext("Wind"),
                Some(PilotMode::Track) => gettext("Track"),
                None => String::new(),
            };
            let note = if mode.is_empty() { none() } else { vec![Chip::plain(mode.clone())] };
            match (o.autopilot, p.mode) {
                (None, _) => r(&gettext("PILOT"), dash(), "", none()),
                (Some(_), Some(PilotMode::Standby)) => r(&gettext("PILOT"), mode, "", none()),
                (Some(_), Some(PilotMode::Wind)) => {
                    let (value, side) = p.wind_angle.map_or_else(|| (dash(), String::new()), angle);
                    r(&gettext("PILOT"), value, &side, note)
                }
                _ => {
                    let unit = if p.heading.is_some_and(|(_, m)| m) { "M" } else { "T" };
                    r(&gettext("PILOT"), p.heading.map_or_else(dash, |(h, _)| bearing(h)), unit, note)
                }
            }
        }
        "stw" => r(&gettext("STW"), o.stw_kn.map_or_else(dash, |v| format!("{:.1}", units.speed(v).0)), units.speed(1.0).1, none()),
        "temp" => r(&gettext("TEMP"), o.water_temp_c.map_or_else(dash, |t| format!("{t:.1}")), "°C", none()),
        "set" => {
            let sd = crate::currents::set_and_drift(&app.weather.borrow().currents, chrono::Utc::now());
            r(&gettext("SET"), sd.as_ref().map_or_else(dash, |(set, drift)| format!("{set} {drift}")), "kn", none())
        }
        "pos" => r(&gettext("POSITION"), o.pos.map_or_else(dash, |p| format!("{} {}", geo::format_lat(p.lat), geo::format_lon(p.lon))), "", gps()),
        _ => return None,
    })
}

/// The ids of the dashboard's tiles: the user's choice, or the default four.
pub fn dashboard_ids(app: &AppState) -> Vec<String> {
    if let Some(ids) = crate::debug::dashboard() {
        return ids;
    }
    let chosen = app.settings.borrow().dashboard.clone();
    if chosen.is_empty() { DEFAULT_DASHBOARD.iter().map(|s| s.to_string()).collect() } else { chosen }
}

pub fn underway(app: &AppState) -> Underway {
    let on = app.canvas.underway();
    if !on {
        return Underway::default();
    }
    Underway { on, card: card(app), dashboard: dashboard_ids(app).iter().filter_map(|id| readout(app, id)).collect(), sail: Some(super::performance::switch(app)) }
}

/// Turns the under-way layout on or off by the user's choice.
pub fn set(app: &AppState, on: bool) {
    app.canvas.set_underway(on);
    app.refresh();
}

/// Navigation started or stopped: the layout follows, when the settings say so.
pub fn auto(app: &AppState, on: bool) {
    if app.settings.borrow().underway_auto {
        set(app, on);
    }
}

/// Starts following a route (the layout follows).
pub fn start_route(app: &Rc<AppState>, id: i64) {
    actions::activate_route(app, id);
}

/// Stops navigating (the layout follows).
pub fn stop(app: &Rc<AppState>) {
    actions::stop_navigation(app);
}

/// Skips to the next waypoint.
pub fn skip(app: &Rc<AppState>) {
    let ok = app.session.borrow_mut().active.as_mut().is_some_and(|a| a.skip());
    if !ok {
        app.toast(&gettext("That was the last waypoint"));
    }
    app.refresh();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::geo::LatLon;
    use crate::core::nav::model::{Route, RoutePoint};
    use crate::core::nav::store::NavStore;
    use crate::core::session::Session;
    use crate::engine::Engine;
    use crate::palette::Mode;

    fn app() -> Rc<AppState> {
        let canvas = Engine::new(crate::config::ViewState { lat: 48.0, lon: -4.0, zoom: 10.0 }, Mode::Day);
        AppState::new(canvas, Session::new(NavStore::open_memory().unwrap()).unwrap(), crate::config::Settings::default())
    }

    fn route(app: &AppState) -> i64 {
        let mut r = Route::new("east");
        for lon in [-4.0, -3.9, -3.8] {
            r.points.push(RoutePoint::new(format!("P{lon}"), LatLon::new(48.0, lon)));
        }
        app.session.borrow_mut().save_route(&r).unwrap().id
    }

    #[test]
    fn starting_a_route_switches_the_layout_and_stopping_switches_it_back() {
        let app = app();
        let id = route(&app);
        assert!(!underway(&app).on);
        start_route(&app, id);
        assert!(app.canvas.underway(), "starting a route puts it under way");
        let u = underway(&app);
        assert!(u.on && u.card.is_some());
        assert_eq!(u.dashboard.len(), 4, "the default dashboard");
        stop(&app);
        assert!(!app.canvas.underway());
        assert_eq!(underway(&app), Underway::default());
    }

    #[test]
    fn the_layout_stays_off_when_the_setting_says_so() {
        let app = app();
        app.settings.borrow_mut().underway_auto = false;
        let id = route(&app);
        start_route(&app, id);
        assert!(!app.canvas.underway());
        set(&app, true);
        assert!(app.canvas.underway(), "the user can still turn it on");
    }

    #[test]
    fn the_card_names_the_next_waypoint_and_can_skip_until_the_last() {
        let app = app();
        let id = route(&app);
        actions::activate_route(&app, id);
        let c = card(&app).unwrap();
        assert_eq!(c.number, 2, "the first point is where the route starts");
        assert!(c.can_skip);
        assert_eq!(c.cells[0].label, "BTW");
        skip(&app);
        let c = card(&app).unwrap();
        assert_eq!(c.number, 3);
        assert!(!c.can_skip, "that was the last one");
    }

    #[test]
    fn readouts_say_how_old_the_fix_is_and_show_dashes_without_data() {
        let app = app();
        let r = readout(&app, "sog").unwrap();
        assert_eq!((r.value.as_str(), r.note.as_slice()), ("—", [Chip::plain("No fix")].as_slice()));
        {
            let mut s = app.session.borrow_mut();
            s.own.sog_kn = Some(6.04);
            s.own.pos_updated = Some(chrono::Utc::now());
            s.own.depth_m = Some(9.43);
        }
        assert_eq!(readout(&app, "sog").unwrap().value, "6.0");
        assert_eq!(readout(&app, "sog").unwrap().note[0].icon, Some(Icon::Satellite));
        assert_eq!(readout(&app, "depth").unwrap().value, "9.4");
        assert!(readout(&app, "nonsense").is_none());
    }

    #[test]
    fn readouts_use_the_chosen_units() {
        let app = app();
        app.settings.borrow_mut().speed_unit = "km/h".into();
        app.settings.borrow_mut().depth_unit = "ft".into();
        {
            let mut s = app.session.borrow_mut();
            s.own.sog_kn = Some(10.0);
            s.own.depth_m = Some(10.0);
        }
        let sog = readout(&app, "sog").unwrap();
        assert_eq!((sog.value.as_str(), sog.unit.as_str()), ("18.5", "km/h"));
        let depth = readout(&app, "depth").unwrap();
        assert_eq!((depth.value.as_str(), depth.unit.as_str()), ("32.8", "ft"));
    }

    #[test]
    fn wind_angles_and_the_autopilot() {
        use crate::core::comm::nmea::PilotMode;
        use crate::core::comm::state::Autopilot;
        let app = app();
        assert_eq!(readout(&app, "pilot").unwrap().value, "—");
        {
            let mut s = app.session.borrow_mut();
            s.own.heading = Some(130.0);
            s.own.wind_apparent = Some((-38.4, 18.0));
            s.own.wind_true = Some((80.0, 14.0));
            s.own.autopilot = Some(Autopilot { mode: Some(PilotMode::Heading), heading: Some((128.0, true)), wind_angle: None });
        }
        let awa = readout(&app, "awa").unwrap();
        assert_eq!((awa.value.as_str(), awa.unit.as_str()), ("38°", "port"));
        let twa = readout(&app, "twa").unwrap();
        assert_eq!((twa.value.as_str(), twa.unit.as_str()), ("50°", "port"), "from 080° on a heading of 130°");
        assert_eq!(readout(&app, "twd").unwrap().value, "080°");
        let p = readout(&app, "pilot").unwrap();
        assert_eq!((p.value.as_str(), p.unit.as_str(), p.note[0].text.as_str()), ("128°", "M", "Auto"));
        app.session.borrow_mut().own.autopilot = Some(Autopilot { mode: Some(PilotMode::Standby), ..Autopilot::default() });
        assert_eq!(readout(&app, "pilot").unwrap().value, "Standby");
    }

    #[test]
    fn every_offered_tile_has_a_readout() {
        let app = app();
        for (id, _) in choices() {
            assert!(readout(&app, id).is_some(), "{id}");
        }
        assert!(FULL_DASHBOARD.iter().all(|id| choices().iter().any(|(c, _)| c == id)));
    }

    #[test]
    fn cross_track_error_names_its_side() {
        assert_eq!(xte(0.04), "0.04 R");
        assert_eq!(xte(-0.5), "0.50 L");
    }
}
