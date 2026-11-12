//! Marks, routes and vessels: their properties, the selection, GPX files, and navigating a route.

use super::{b, error, f, file_stem, i, parse, parse_departure, s};
use crate::app::{AppState, Selection};
use crate::core::comm::ais::{nav_status_name, ship_type_name};
use crate::core::comm::cpa::{Motion, cpa};
use crate::core::geo::{self, LatLon};
use crate::core::nav::calc::format_range_bearing;
use crate::flows;
use crate::i18n::{gettext, gettext_f};
use crate::vm;
use chrono::{Local, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use std::rc::Rc;

/// A mark's form as it is saved. Every field is needed: one left out is an error, where it used
/// to be read as nothing (a mark saved without `visible` was hidden).
#[derive(Deserialize)]
struct WaypointForm {
    id: i64,
    name: String,
    latitude: String,
    longitude: String,
    description: String,
    arrival_radius_nm: f64,
    visible: bool,
}

/// A route's form as it is saved; every field is needed, as for a mark.
#[derive(Deserialize)]
struct RouteForm {
    id: i64,
    name: String,
    speed_kn: f64,
    departure: String,
    visible: bool,
}

fn waypoint_json(app: &AppState, id: i64) -> Value {
    match app.session.borrow().waypoints.iter().find(|w| w.id == id) {
        Some(w) => json!({
            "id": w.id, "name": w.name, "latitude": geo::format_lat(w.pos.lat), "longitude": geo::format_lon(w.pos.lon),
            "description": w.description, "icon": w.icon, "arrival_radius_nm": w.arrival_radius_nm, "visible": w.visible,
        }),
        None => Value::Null,
    }
}

fn route_json(app: &AppState, id: i64) -> Value {
    let s = app.session.borrow();
    let Some(r) = s.routes.iter().find(|r| r.id == id) else { return Value::Null };
    let legs: Vec<Value> = r
        .points
        .windows(2)
        .map(|w| {
            let (d, brg) = crate::core::nav::calc::leg(&w[0].pos, &w[1].pos);
            json!({ "from": w[0].name, "to": w[1].name, "text": format_range_bearing(d, brg) })
        })
        .collect();
    json!({
        "id": r.id, "name": r.name, "speed_kn": r.speed_kn,
        "plan": vm::plan::info(app, id),
        // Made by weather routing: it is not offered weather routing or a best departure again.
        "weather_route": s.plans.contains_key(&id),
        "ensemble": flows::ensemble::lines(app, id),
        "departure": r.departure.map(|d| d.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default(),
        "visible": r.visible,
        "points": r.points.iter().map(|p| json!({ "name": p.name, "position": crate::text::position_text(&p.pos) })).collect::<Vec<_>>(),
        "legs": legs, "distance_nm": crate::core::nav::calc::total_distance_nm(r),
        "chips": vm::passage::points_and_distance(r.points.len(), crate::core::nav::calc::total_distance_nm(r)),
    })
}

fn vessel_json(app: &AppState, mmsi: u32) -> Value {
    let s = app.session.borrow();
    let Some(t) = s.targets.get(mmsi).cloned() else { return Value::Null };
    let own = s.own.pos.zip(s.own.sog_kn).zip(s.own.cog).map(|((pos, sog_kn), cog)| Motion { pos, sog_kn, cog });
    let mut groups: Vec<Value> = Vec::new();
    let mut about: Vec<(String, String)> = vec![("MMSI".into(), mmsi.to_string())];
    if let Some(c) = &t.callsign {
        about.push((gettext("Call sign"), c.clone()));
    }
    if let Some(ty) = t.ship_type {
        about.push((gettext("Type"), gettext(ship_type_name(ty))));
    }
    if let Some(d) = t.dims {
        about.push((gettext("Size"), format!("{} × {} m", d.length(), d.beam())));
    }
    if let Some(d) = &t.destination {
        about.push((gettext("Destination"), d.clone()));
    }
    if let Some(st) = t.status {
        about.push((gettext("Status"), gettext(nav_status_name(st))));
    }
    groups.push(json!({ "title": "", "rows": about }));
    let mut mv: Vec<(String, String)> = Vec::new();
    if let Some(p) = t.pos {
        mv.push((gettext("Position"), crate::text::position_text(&p)));
    }
    mv.push((
        gettext("Speed and course"),
        match (t.sog_kn, t.cog) {
            (Some(s), Some(c)) => format!("{s:.1} kn  {c:03.0}°"),
            (Some(s), None) => format!("{s:.1} kn"),
            _ => "—".to_string(),
        },
    ));
    if let (Some(o), Some(pos), Some(sog), Some(cog)) = (own.as_ref(), t.pos, t.sog_kn, t.cog) {
        let c = cpa(o, &Motion { pos, sog_kn: sog, cog });
        mv.push((gettext("Range and bearing"), format_range_bearing(c.range_nm, c.bearing)));
        mv.push((
            gettext("Closest approach"),
            if c.tcpa_min >= 0.0 { format!("{:.2} nm in {:.0} min", c.cpa_nm, c.tcpa_min) } else { gettext("Moving apart") },
        ));
    }
    let secs = (Utc::now() - t.last_seen).num_seconds().max(0);
    mv.push((gettext("Last heard"), gettext_f("{n} s ago", &[("n", &secs.to_string())])));
    groups.push(json!({ "title": gettext("Movement"), "rows": mv }));
    json!({ "title": t.name.clone().unwrap_or_else(|| mmsi.to_string()), "groups": groups })
}

/// Answers `name` if it is one of these calls.
pub(super) fn call(app: &Rc<AppState>, name: &str, args: &Value) -> Option<Value> {
    Some(match name {
        "waypoint_get" => waypoint_json(app, i(args, "id")),
        "waypoint_save" => {
            let form: WaypointForm = match parse(args) {
                Ok(form) => form,
                Err(e) => return Some(e),
            };
            let (Some(lat), Some(lon)) = (geo::parse_coord(&form.latitude, true), geo::parse_coord(&form.longitude, false)) else {
                return Some(error(gettext("The position is not a latitude and longitude.")));
            };
            let w = app.session.borrow().waypoints.iter().find(|w| w.id == form.id).cloned();
            let Some(mut w) = w else { return Some(Value::Null) };
            w.name = form.name;
            w.pos = LatLon::new(lat, lon);
            w.description = form.description;
            w.arrival_radius_nm = form.arrival_radius_nm.max(0.0);
            w.visible = form.visible;
            let r = app.session.borrow_mut().update_waypoint(&w);
            app.refresh();
            match r {
                Ok(()) => json!({}),
                Err(e) => error(e.to_string()),
            }
        }
        "route_get" => route_json(app, i(args, "id")),
        "route_save" => {
            let form: RouteForm = match parse(args) {
                Ok(form) => form,
                Err(e) => return Some(e),
            };
            let r = app.session.borrow().routes.iter().find(|r| r.id == form.id).cloned();
            let Some(mut r) = r else { return Some(Value::Null) };
            r.name = form.name;
            r.speed_kn = form.speed_kn.clamp(0.5, 40.0);
            r.departure = match parse_departure(&form.departure) {
                Ok(d) => d,
                Err(()) => return Some(error(gettext("The departure is not a date and time (YYYY-MM-DD HH:MM)."))),
            };
            r.visible = form.visible;
            let res = app.session.borrow_mut().save_route(&r);
            app.refresh();
            match res {
                Ok(_) => json!({}),
                Err(e) => error(e.to_string()),
            }
        }
        // The name alone, from the title of the route's page; the rest of its form may hold edits not saved.
        "route_rename" => {
            let (id, name) = (i(args, "id"), s(args, "name").trim().to_string());
            let r = app.session.borrow().routes.iter().find(|r| r.id == id).cloned();
            let Some(mut r) = r.filter(|_| !name.is_empty()) else { return Some(Value::Null) };
            r.name = name;
            let res = app.session.borrow_mut().save_route(&r);
            app.refresh();
            match res {
                Ok(_) => json!({}),
                Err(e) => error(e.to_string()),
            }
        }
        "vessel_get" => vessel_json(app, i(args, "id") as u32),
        // Where a position is on the chart, in points from its top left (scripted checks tap there).
        "screen_of" => match app.canvas.screen_of(&LatLon::new(f(args, "lat"), f(args, "lon"))) {
            Some((x, y)) => json!({ "x": x, "y": y }),
            None => Value::Null,
        },
        "select" => {
            let sel = match s(args, "kind").as_str() {
                "route" => Selection::Route(i(args, "id")),
                "waypoint" => Selection::Waypoint(i(args, "id")),
                _ => Selection::None,
            };
            app.select(sel);
            json!({})
        }

        // GPX files.
        "gpx_import" => match std::fs::read_to_string(s(args, "path")) {
            Err(e) => error(gettext_f("Could not read the file: {error}", &[("error", &e.to_string())])),
            Ok(text) => match crate::core::nav::gpx::parse(&text) {
                Err(e) => error(gettext_f("This is not a usable GPX file: {error}", &[("error", &e)])),
                Ok(doc) => {
                    let res = {
                        let mut sess = app.session.borrow_mut();
                        let r = sess.store.import(&doc);
                        if r.is_ok() {
                            sess.waypoints = sess.store.waypoints().unwrap_or_default();
                            sess.routes = sess.store.routes().unwrap_or_default();
                            sess.tracks = sess.store.tracks().unwrap_or_default();
                        }
                        r
                    };
                    match res {
                        Ok((w, r, t)) => {
                            let mut msg = gettext_f("Imported {w} waypoints, {r} routes, {t} tracks", &[("w", &w.to_string()), ("r", &r.to_string()), ("t", &t.to_string())]);
                            if !doc.warnings.is_empty() {
                                msg.push_str(&format!(" ({})", gettext_f("{n} items skipped", &[("n", &doc.warnings.len().to_string())])));
                            }
                            app.toast(&msg);
                            app.refresh();
                            json!({})
                        }
                        Err(e) => error(gettext_f("Could not store the data: {error}", &[("error", &e.to_string())])),
                    }
                }
            },
        },
        "gpx_export" => {
            let id = i(args, "id");
            let doc = {
                let sess = app.session.borrow();
                if id == 0 {
                    crate::core::nav::gpx::GpxDoc { waypoints: sess.waypoints.clone(), routes: sess.routes.clone(), tracks: sess.tracks.clone(), warnings: vec![] }
                } else {
                    let routes: Vec<_> = sess.routes.iter().filter(|r| r.id == id).cloned().collect();
                    crate::core::nav::gpx::GpxDoc { waypoints: vec![], routes, tracks: vec![], warnings: vec![] }
                }
            };
            match std::fs::write(s(args, "path"), crate::core::nav::gpx::write(&doc)) {
                Ok(()) => {
                    app.toast(&gettext("Exported"));
                    json!({})
                }
                Err(e) => error(gettext_f("Could not write the file: {error}", &[("error", &e.to_string())])),
            }
        }
        "route_file_name" => json!(file_stem(&s(args, "name"))),

        // Under way.
        "underway_set" => {
            vm::underway::set(app, b(args, "on"));
            json!({})
        }
        "underway_skip" => {
            vm::underway::skip(app);
            json!({})
        }
        "underway_stop" => {
            vm::underway::stop(app);
            json!({})
        }
        "start_route" => {
            vm::underway::start_route(app, i(args, "id"));
            json!({})
        }

        _ => return None,
    })
}
