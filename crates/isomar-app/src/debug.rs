//! Development aids driven by environment variables that do not need a
//! toolkit; the rest (screenshots, the benchmark, dialogs) are the shell's.
//! The variables are listed in the shell's `debug.rs`.

use super::app::AppState;
use crate::config::ViewState;
use crate::core::geo::LatLon;
use crate::core::nav::model::{Route, RoutePoint};
use std::rc::Rc;

/// The view requested through `ISOMAR_DEBUG_VIEW`, if any.
pub fn forced_view() -> Option<ViewState> {
    let v = std::env::var("ISOMAR_DEBUG_VIEW").ok()?;
    let mut it = v.split(',').map(|s| s.trim().parse::<f64>());
    match (it.next(), it.next(), it.next()) {
        (Some(Ok(lat)), Some(Ok(lon)), Some(Ok(zoom))) => Some(ViewState { lat, lon, zoom }),
        _ => None,
    }
}

/// Whether `ISOMAR_DEBUG_TIMING` asks for frame and layer times.
pub fn timing() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("ISOMAR_DEBUG_TIMING").is_some())
}

/// Whether `ISOMAR_DEBUG_COVERAGE` asks for the outline of each ENC cell's data.
pub fn coverage() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("ISOMAR_DEBUG_COVERAGE").is_some())
}

/// Whether `ISOMAR_OFFLINE` forbids downloading map tiles; cached ones are still used.
pub fn offline() -> bool {
    std::env::var_os("ISOMAR_OFFLINE").is_some()
}

/// The fraction of the weather-route preview to start at (`ISOMAR_ROUTER_T`).
pub fn router_clock() -> Option<f64> {
    std::env::var("ISOMAR_ROUTER_T").ok()?.parse::<f64>().ok().map(|f| f.clamp(0.0, 1.0))
}

/// How many alternatives the preview starts with (`ISOMAR_ROUTER_PICKS`).
pub fn router_picks() -> usize {
    std::env::var("ISOMAR_ROUTER_PICKS").ok().and_then(|v| v.parse().ok()).unwrap_or(0)
}

/// Whether the preview opens its statistics at once (`ISOMAR_ROUTER_STATS`).
pub fn router_stats() -> bool {
    std::env::var_os("ISOMAR_ROUTER_STATS").is_some()
}

/// What the preview does at once (`ISOMAR_ROUTER_USE`): `1` saves the route that arrived, `replay`
/// then plays it back from the route's saved plan.
pub fn router_use() -> Option<String> {
    std::env::var("ISOMAR_ROUTER_USE").ok()
}

/// Where the `ISOMAR_DEMO` boat is: its position, course and speed (0 is moored, with no traffic).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DemoBoat {
    pub lat: f64,
    pub lon: f64,
    pub cog: f64,
    pub sog: f64,
}

impl DemoBoat {
    pub fn pos(&self) -> LatLon {
        LatLon::new(self.lat, self.lon)
    }
}

/// A sea area the demo knows: a harbour to lie in, a place under way, and a route and marks, every one
/// of them chosen by hand on the chart and checked against the coastline
/// (`tiles::live::the_demo_areas_are_at_sea`). The demo puts the boat, the route and the marks only in
/// one of these, never round the view centre: a view centre is as likely to be a hill, a peninsula or the
/// middle of an ocean, and screenshots showed the boat aground and a route over an island.
pub struct DemoArea {
    pub name: &'static str,
    /// Moored in the marina.
    pub harbour: DemoBoat,
    /// Under way, on a leg of the route, with the demo traffic round it.
    pub at_sea: DemoBoat,
    pub route: &'static [(&'static str, f64, f64)],
    pub marks: &'static [(&'static str, f64, f64)],
}

pub const DEMO_AREAS: &[DemoArea] = &[
    DemoArea {
        name: "Quiberon",
        // Port Haliguen, on the east side of the Quiberon peninsula.
        harbour: DemoBoat { lat: 47.4882, lon: -3.1020, cog: 30.0, sog: 0.0 },
        at_sea: DemoBoat { lat: 47.4450, lon: -2.9880, cog: 130.0, sog: 6.0 },
        route: &[("Haliguen", 47.4950, -3.0550), ("Bay", 47.4750, -3.0300), ("Houat", 47.4250, -2.9550), ("Hoëdic", 47.3750, -2.8600)],
        marks: &[("Mark A", 47.4950, -3.0000), ("Anchorage", 47.3920, -2.9050)],
    },
    DemoArea {
        name: "Iroise",
        // Port Vauban, Camaret-sur-Mer.
        harbour: DemoBoat { lat: 48.27875, lon: -4.59325, cog: 90.0, sog: 0.0 },
        at_sea: DemoBoat { lat: 48.1700, lon: -4.7750, cog: 190.0, sog: 6.0 },
        route: &[("Bertheaume", 48.3100, -4.6900), ("Iroise", 48.2000, -4.7600), ("Raz approach", 48.1000, -4.7900)],
        marks: &[("Mark A", 48.2600, -4.8100), ("Anchorage", 48.2950, -4.6600)],
    },
];

/// How near the view centre must be to an area for the demo to use it.
const DEMO_AREA_NM: f64 = 40.0;

/// The demo area round `centre`, if any.
pub fn demo_area(centre: LatLon) -> Option<&'static DemoArea> {
    DEMO_AREAS
        .iter()
        .map(|a| (a, a.harbour.pos().distance_nm(&centre).min(a.at_sea.pos().distance_nm(&centre))))
        .filter(|(_, d)| *d <= DEMO_AREA_NM)
        .min_by(|x, y| x.1.total_cmp(&y.1))
        .map(|(a, _)| a)
}

/// The `ISOMAR_DEMO` boat for a view centred on `centre`: `ISOMAR_DEMO_BOAT=harbour|sea|off`, by
/// default at sea when `ISOMAR_UNDERWAY` is set and in the harbour otherwise. None outside every
/// demo area: there is nowhere known to be safe to put it.
pub fn demo_boat(centre: LatLon) -> Option<DemoBoat> {
    let area = demo_area(centre)?;
    match std::env::var("ISOMAR_DEMO_BOAT").as_deref() {
        Ok("off") => None,
        Ok("sea") => Some(area.at_sea),
        Ok("harbour") => Some(area.harbour),
        _ if underway() => Some(area.at_sea),
        _ => Some(area.harbour),
    }
}

/// With `ISOMAR_DEMO` set and an empty database, adds the route and marks of the demo area in view
/// (`DEMO_AREAS`), or the passage of `ISOMAR_ROUTE` alone, so there is something to look at.
pub fn seed_demo(app: &Rc<AppState>) {
    seed_demo_currents(app);
    if std::env::var_os("ISOMAR_DEMO").is_none() {
        return;
    }
    let c = app.canvas.view_state().center();
    let mut s = app.session.borrow_mut();
    if !s.routes.is_empty() || !s.waypoints.is_empty() {
        return;
    }
    if let Some((from, to)) = demo_route_ends() {
        // A passage to route, for screenshots of the shells that have no `ISOMAR_DIALOG`.
        let mut r = Route::new("Passage demo");
        r.points.push(RoutePoint::new("Start", from));
        r.points.push(RoutePoint::new("Far", to));
        let _ = s.save_route(&r);
        return;
    }
    let Some(area) = demo_area(c) else { return };
    let mut r = Route::new("Demo route");
    for (name, lat, lon) in area.route {
        r.points.push(RoutePoint::new(*name, LatLon::new(*lat, *lon)));
    }
    let _ = s.save_route(&r);
    for (name, lat, lon) in area.marks {
        let _ = s.add_waypoint(LatLon::new(*lat, *lon), Some((*name).into()));
    }
}

/// `ISOMAR_ROUTE=<lat>,<lon>,<lat>,<lon>`: the ends of the demo route, instead of the short one round the view.
fn demo_route_ends() -> Option<(LatLon, LatLon)> {
    let v = std::env::var("ISOMAR_ROUTE").ok()?;
    let n: Vec<f64> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    (n.len() == 4).then(|| (LatLon::new(n[0], n[1]), LatLon::new(n[2], n[3])))
}

/// With `ISOMAR_DEMO_CURRENTS` set, loads a made-up current round the Iroise (the Raz de Sein and the
/// Fromveur running at up to 6 kn) for two days: for screenshots and the simulators. No real data.
pub fn seed_demo_currents(app: &Rc<AppState>) {
    if std::env::var_os("ISOMAR_DEMO_CURRENTS").is_none() {
        return;
    }
    let now = chrono::Utc::now();
    {
        let mut w = app.weather.borrow_mut();
        w.currents.add(crate::core::currents::CurrentSource::Demo, crate::currents::demo_fields(now));
        if w.store.is_empty() {
            w.time = now;
        }
    }
    app.refresh();
}

/// With `ISOMAR_DEMO_PERF=<hours>` and an empty performance log, fills it with that many hours of a
/// synthetic boat round the view centre (wind instruments with a vane turned 3 degrees, a speedo reading
/// 5% low, a tidal stream, a fifth of the time under engine, the forecast's errors), ending now, and two
/// passages, so the Performance page and the polar editor have something to show.
pub fn seed_perf(app: &Rc<AppState>) {
    use crate::core::perf::passage::Passage;
    use crate::core::perf::synth::{self, Config, scaled};
    let Some(hours) = std::env::var("ISOMAR_DEMO_PERF").ok().and_then(|h| h.parse::<f64>().ok()) else { return };
    let s = app.session.borrow();
    if s.store.perf_summary(None).map(|x| x.count).unwrap_or(0) > 0 {
        return;
    }
    let reference = crate::polars::active(app);
    let polar_id = crate::polars::active_id(app);
    let now = chrono::Utc::now();
    let start = now - chrono::Duration::seconds((hours * 3600.0) as i64);
    let c = Config {
        hours,
        truth: scaled(&reference, 0.88, 1.02),
        awa_offset: 3.0,
        angle_error: [3.5, -2.0],
        speedo_factor: 0.95,
        current_kn: 0.8,
        motoring_share: 0.2,
        start,
        pos: app.canvas.view_state().center(),
        ..Default::default()
    };
    for mut x in synth::log(&c) {
        x.polar = polar_id.clone();
        let _ = s.store.insert_perf_sample(&x);
    }
    for (i, (name, planned_h, sailed_h)) in [("Groix to Belle-Île", 4.5, 5.2), ("Belle-Île to Houat", 2.0, 1.9)].into_iter().enumerate() {
        let finished = now - chrono::Duration::days(2 * i as i64 + 1);
        let _ = s.store.insert_passage(&Passage {
            route_id: 0,
            name: name.into(),
            started: (finished - chrono::Duration::minutes((sailed_h * 60.0) as i64)).timestamp(),
            finished: finished.timestamp(),
            planned_s: planned_h * 3600.0,
            sailed_s: sailed_h * 3600.0,
            planned_sog: 5.5,
            sailed_sog: 5.5 * planned_h / sailed_h,
            distance_nm: 5.5 * planned_h,
            planned_motoring: 0.0,
            motored: 0.1,
        });
    }
}

/// The palette forced by `ISOMAR_PALETTE=day|dusk|night`, if any.
pub fn palette() -> Option<crate::palette::Mode> {
    std::env::var("ISOMAR_PALETTE").ok()?.parse().ok()
}

/// The window size asked for by `ISOMAR_SIZE=WxH`, for the narrow and wide boards.
pub fn window_size() -> Option<(i32, i32)> {
    let v = std::env::var("ISOMAR_SIZE").ok()?;
    let (w, h) = v.split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

/// Whether `ISOMAR_UNDERWAY` starts with the under-way layout on.
pub fn underway() -> bool {
    std::env::var_os("ISOMAR_UNDERWAY").is_some()
}

/// The under-way dashboard asked for by `ISOMAR_DASHBOARD=depth,sog,...` (tile ids, as `vm::underway::readout`
/// knows them), or `full` for a boat with every instrument (`vm::underway::FULL_DASHBOARD`). Overrides the
/// settings without saving.
pub fn dashboard() -> Option<Vec<String>> {
    let v = std::env::var("ISOMAR_DASHBOARD").ok()?;
    let ids: Vec<&str> = if v.trim() == "full" { crate::vm::underway::FULL_DASHBOARD.to_vec() } else { v.split(',').map(str::trim).filter(|s| !s.is_empty()).collect() };
    Some(ids.into_iter().map(String::from).collect())
}

/// The popover asked for by `ISOMAR_POPOVER=aid|vessel|tide`.
pub fn popover() -> Option<String> {
    std::env::var("ISOMAR_POPOVER").ok()
}

/// With `ISOMAR_ALARM=anchor|lostfix` set, fakes that situation once the boat has a position, for
/// screenshots (for a collision, `ISOMAR_DEMO_COLLISION` puts the demo ferry on a crossing course).
/// Called on every clock tick.
pub fn fake_alarm(app: &AppState) {
    let Ok(which) = std::env::var("ISOMAR_ALARM") else { return };
    let mut s = app.session.borrow_mut();
    let Some(pos) = s.own.pos else { return };
    match which.as_str() {
        "anchor" if s.anchor.is_none() => s.set_anchor(pos.destination(0.0, 0.3), 0.05),
        "lostfix" => s.own.pos_updated = Some(chrono::Utc::now() - chrono::Duration::minutes(3)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_demo_is_only_in_its_areas_and_the_moored_boat_is_still() {
        // The Bay of Biscay (the weather-route view), Boston and Mallorca have no demo boat, route or marks.
        for c in [LatLon::new(45.6, -5.8), LatLon::new(42.33, -70.95), LatLon::new(39.6, 2.9)] {
            assert!(demo_area(c).is_none(), "{c:?}");
        }
        assert_eq!(demo_area(LatLon::new(47.42, -2.99)).map(|a| a.name), Some("Quiberon"));
        assert_eq!(demo_area(LatLon::new(48.35, -4.75)).map(|a| a.name), Some("Iroise"));
        for a in DEMO_AREAS {
            assert_eq!(a.harbour.sog, 0.0, "{}: moored, so no traffic is placed round it", a.name);
            assert!(a.at_sea.sog > 0.0, "{}", a.name);
        }
    }
}
