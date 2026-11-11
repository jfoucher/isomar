//! Live instrument readouts.

use crate::app::AppState;
use crate::core::geo;
use crate::core::nav::calc::format_duration;
use crate::core::session::Session;
use crate::i18n::gettext;
use chrono::Local;
use serde::Serialize;

fn dash(v: Option<String>) -> String {
    v.unwrap_or_else(|| "—".to_string())
}

fn bearing(v: Option<f64>) -> Option<String> {
    v.map(|b| format!("{:03}°", (b.round() as i64).rem_euclid(360)))
}

fn kn(v: Option<f64>) -> Option<String> {
    v.map(|s| format!("{s:.1}"))
}

/// Wind angle with the side it is on: `45° stbd`.
pub fn wind_angle(a: f64) -> String {
    let side = if a >= 0.0 { gettext("stbd") } else { gettext("port") };
    format!("{:.0}° {side}", a.abs())
}

/// Readout strings `(value, unit)` for each tile, in display order.
pub fn readouts(s: &Session) -> Vec<(String, String)> {
    let o = &s.own;
    let nav = s.nav.as_ref();
    let pos = o.pos.map(|p| format!("{}\n{}", geo::format_lat(p.lat), geo::format_lon(p.lon)));
    vec![
        (dash(kn(o.sog_kn)), "kn".into()),
        (dash(bearing(o.cog)), "".into()),
        (dash(bearing(o.heading)), "".into()),
        (dash(o.depth_m.map(|d| format!("{d:.1}"))), "m".into()),
        (dash(kn(o.stw_kn)), "kn".into()),
        (dash(o.wind_apparent.map(|(a, _)| wind_angle(a))), o.wind_apparent.map(|(_, s)| format!("{s:.1} kn")).unwrap_or_default()),
        (dash(o.wind_true.and_then(|(d, _)| bearing(Some(d)))), o.wind_true.map(|(_, s)| format!("{s:.1} kn")).unwrap_or_default()),
        (dash(o.water_temp_c.map(|t| format!("{t:.1}"))), "°C".into()),
        (dash(pos), "".into()),
        (dash(nav.map(|n| bearing(Some(n.bearing)).unwrap_or_default())), "".into()),
        (dash(nav.map(|n| format!("{:.2}", n.range_nm))), "nm".into()),
        (
            dash(nav.map(|n| format!("{:.2}", n.xte_nm.abs()))),
            nav.map(|n| if n.xte_nm > 0.0 { gettext("nm, steer left") } else { gettext("nm, steer right") }).unwrap_or_default(),
        ),
        (dash(nav.and_then(|n| kn(n.vmg_kn))), "kn".into()),
        (dash(nav.and_then(|n| n.ttg).map(format_duration)), "".into()),
        (dash(nav.and_then(|n| n.eta).map(|t| t.with_timezone(&Local).format("%H:%M").to_string())), "".into()),
    ]
}

/// The title of each tile, in the order `readouts` gives them.
pub fn titles() -> Vec<String> {
    vec![
        gettext("Speed over ground"),
        gettext("Course over ground"),
        gettext("Heading"),
        gettext("Depth"),
        gettext("Speed through water"),
        gettext("Apparent wind"),
        gettext("True wind"),
        gettext("Water temperature"),
        gettext("Position"),
        gettext("Bearing to waypoint"),
        gettext("Distance to waypoint"),
        gettext("Cross-track error"),
        gettext("Velocity made good"),
        gettext("Time to go"),
        gettext("Arrival"),
    ]
}

/// A tile as the Instruments page shows it.
#[derive(Serialize, Debug, PartialEq)]
pub struct Tile {
    pub title: String,
    pub value: String,
    pub unit: String,
}

pub fn tiles(app: &AppState) -> Vec<Tile> {
    let mut v: Vec<Tile> = titles().into_iter().zip(readouts(&app.session.borrow())).map(|(title, (value, unit))| Tile { title, value, unit }).collect();
    // The stream the log, the compass and the GPS measure together, averaged over minutes.
    let (value, unit) = match crate::currents::set_and_drift(&app.weather.borrow().currents, chrono::Utc::now()) {
        Some((set, drift)) => (format!("{set} {drift}"), "kn".to_string()),
        None => ("—".to_string(), String::new()),
    };
    v.push(Tile { title: gettext("Set and drift"), value, unit });
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::comm::nmea::{frame, make_rmc};
    use crate::core::comm::source::{Event, Payload};
    use crate::core::nav::store::NavStore;
    use chrono::Utc;

    #[test]
    fn empty_session_shows_dashes() {
        let s = Session::new(NavStore::open_memory().unwrap()).unwrap();
        let r = readouts(&s);
        assert_eq!(r.len(), 15);
        assert!(r.iter().all(|(v, _)| v == "—"));
    }

    #[test]
    fn readouts_reflect_the_instruments() {
        let mut s = Session::new(NavStore::open_memory().unwrap()).unwrap();
        let now = Utc::now();
        let lines = [
            make_rmc(now, &geo::LatLon::new(48.39, -4.49), 6.25, 91.4),
            frame("HEHDT,92.0,T", '$'),
            frame("SDDPT,12.34,0.0", '$'),
            frame("IIMWV,045,R,12.0,N,A", '$'),
            frame("YXMTW,15.5,C", '$'),
        ];
        let ev: Vec<Event> = lines.into_iter().map(|l| Event { source: "t".into(), payload: Payload::Line(l) }).collect();
        s.process(&ev, now);
        let r = readouts(&s);
        // 6.25 may print either way depending on float representation.
        assert!(r[0].0 == "6.2" || r[0].0 == "6.3", "{}", r[0].0);
        assert_eq!(r[0].1, "kn");
        assert_eq!(r[1].0, "091°");
        assert_eq!(r[2].0, "092°");
        assert_eq!(r[3].0, "12.3");
        assert!(r[5].0.contains("45°") && r[5].0.contains(&gettext("stbd")) && r[5].1 == "12.0 kn");
        assert_eq!(r[7].0, "15.5");
        assert!(r[8].0.contains("48°23.400′N"));
        assert_eq!(r[9].0, "—", "no active route, no navigation data");
    }

    #[test]
    fn wind_angles_name_their_side() {
        assert!(wind_angle(45.0).contains("45°"));
        assert_ne!(wind_angle(45.0), wind_angle(-45.0));
    }
}
