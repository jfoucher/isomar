use super::polars::polar_arg;
use super::*;
use crate::core::comm::source::ConnectionConfig;
use crate::core::geo::LatLon;
use crate::core::nav::store::NavStore;
use crate::core::session::Session;
use crate::engine::Engine;
use crate::palette::Mode;

fn app() -> Rc<AppState> {
    let canvas = Engine::new(crate::config::ViewState { lat: 48.0, lon: -4.0, zoom: 10.0 }, Mode::Day);
    AppState::new(canvas, Session::new(NavStore::open_memory().unwrap()).unwrap(), crate::config::Settings::default())
}

#[test]
fn the_alarm_limits_are_the_users_from_the_start() {
    let canvas = Engine::new(crate::config::ViewState { lat: 48.0, lon: -4.0, zoom: 10.0 }, Mode::Day);
    let settings = crate::config::Settings { ais_cpa_nm: 0.2, ais_tcpa_min: 9.0, xte_limit_nm: 0.3, ..Default::default() };
    let app = AppState::new(canvas, Session::new(NavStore::open_memory().unwrap()).unwrap(), settings);
    let s = app.session.borrow();
    assert_eq!((s.alarm_cfg.cpa_nm, s.alarm_cfg.tcpa_min, s.alarm_cfg.xte_limit_nm), (0.2, 9.0, 0.3));
}

#[test]
fn unknown_calls_say_so_and_pages_answer_json() {
    let app = app();
    assert!(call(&app, "nonsense", &Value::Null)["error"].as_str().unwrap().contains("nonsense"));
    assert!(call(&app, "passage", &Value::Null)["routes"].as_array().unwrap().is_empty());
    assert_eq!(call(&app, "window", &Value::Null)["link"], "none");
    assert!(call(&app, "timeline", &Value::Null).is_null());
    assert_eq!(call(&app, "footer", &Value::Null)["planning_only"], true);
}

#[test]
fn settings_are_read_and_patched() {
    let app = app();
    assert_eq!(call(&app, "settings_get", &Value::Null)["chart_style"], "paper");
    let r = call(&app, "settings_patch", &json!({ "chart_style": "ecdis", "safety_depth": 7.5 }));
    assert!(r.get("error").is_none(), "{r}");
    assert_eq!(app.settings.borrow().chart_style, "ecdis");
    assert_eq!(app.settings.borrow().safety_depth, 7.5);
    let bad = call(&app, "settings_patch", &json!({ "safety_depth": "deep" }));
    assert!(bad.get("error").is_some());
    assert_eq!(app.settings.borrow().safety_depth, 7.5, "a bad patch changes nothing");
}

#[test]
fn a_waypoint_is_read_edited_and_saved() {
    let app = app();
    let id = app.session.borrow_mut().add_waypoint(LatLon::new(48.0, -4.0), Some("Mark".into())).unwrap().id;
    let mut w = call(&app, "waypoint_get", &json!({ "id": id }));
    assert_eq!(w["name"], "Mark");
    w["name"] = json!("Anchorage");
    w["latitude"] = json!("48 10.000 N");
    let r = call(&app, "waypoint_save", &w);
    assert!(r.get("error").is_none(), "{r}");
    let saved = call(&app, "waypoint_get", &json!({ "id": id }));
    assert_eq!(saved["name"], "Anchorage");
    assert!(saved["latitude"].as_str().unwrap().starts_with("48°10"));
    w["longitude"] = json!("somewhere");
    assert!(call(&app, "waypoint_save", &w).get("error").is_some());
}

#[test]
fn departures_parse_in_local_time_and_bad_ones_are_refused() {
    assert!(parse_departure("2026-10-27 15:30").unwrap().is_some());
    assert_eq!(parse_departure("  "), Ok(None));
    assert!(parse_departure("tomorrow").is_err());
    let app = app();
    let r = call(&app, "route_compute", &json!({ "id": 1, "departure": "tomorrow" }));
    assert!(r["error"].as_str().unwrap().contains("YYYY-MM-DD"));
}

#[test]
fn the_weather_estimate_needs_a_valid_area() {
    let app = app();
    let ok = call(&app, "weather_estimate", &json!({ "north": "48", "south": "47", "west": "-5", "east": "-4", "horizon_index": 1, "step_index": 1, "groups": ["wind"] }));
    assert_eq!(ok["ok"], true, "{ok}");
    let bad = call(&app, "weather_estimate", &json!({ "north": "x" }));
    assert_eq!(bad["ok"], false);
    let page = call(&app, "weather_page", &Value::Null);
    assert_eq!(page["loaded"], false);
    assert_eq!(page["groups"].as_array().unwrap().len(), 6);
}

#[test]
fn one_field_colours_the_sea_and_wave_arrows_go_over_either() {
    let app = app();
    let page = || call(&app, "weather_page", &Value::Null);
    assert_eq!(page()["fill"], "wind");
    assert_eq!(page()["wave_arrows"], true);
    call(&app, "weather_fill", &json!({ "fill": "waves" }));
    assert_eq!(page()["fill"], "waves");
    // Arrows and barbs are independent of the field, and of each other.
    call(&app, "weather_toggle", &json!({ "name": "wave_arrows" }));
    assert_eq!(page()["wave_arrows"], false);
    assert_eq!(page()["barbs"], true);
    call(&app, "weather_fill", &json!({ "fill": "none" }));
    assert_eq!(page()["fill"], "none");
    call(&app, "weather_fill", &json!({ "fill": "rain" }));
    assert_eq!(page()["fill"], "none", "an unknown field changes nothing");
    call(&app, "weather_fill", &json!({ "fill": "current" }));
    assert_eq!(page()["fill"], "current");
}

#[test]
fn currents_switch_their_arrows_and_routes() {
    let app = app();
    let page = || call(&app, "currents_get", &Value::Null);
    assert_eq!(page()["loaded"], false);
    assert_eq!((page()["arrows"].as_bool(), page()["route_with"].as_bool()), (Some(true), Some(true)), "on by default");
    app.weather.borrow_mut().currents.add(crate::core::currents::CurrentSource::Demo, crate::currents::demo_fields(Utc::now()));
    assert_eq!(page()["loaded"], true);
    assert_eq!(page()["sources"][0]["label"], "Demo");
    call(&app, "currents_arrows", &json!({ "on": false }));
    call(&app, "currents_route_with", &json!({ "on": false }));
    assert_eq!((page()["arrows"].as_bool(), page()["route_with"].as_bool()), (Some(false), Some(false)));
    assert!(!app.settings.borrow().currents.route_with, "kept for next time");
    call(&app, "currents_clear", &Value::Null);
    assert_eq!(page()["loaded"], false);
}

#[test]
fn route_names_make_safe_file_names() {
    assert_eq!(file_stem("Brest → Penmarc'h"), "Brest → Penmarc'h");
    assert_eq!(file_stem("a/b:c"), "a_b_c");
    assert_eq!(file_stem("  ../x "), "_x");
    assert_eq!(file_stem("..."), "route");
}

#[test]
fn a_polar_survives_a_json_round_trip() {
    let p = call(&app(), "polar_default", &Value::Null);
    assert!(p["tws"].as_array().unwrap().len() > 3);
    assert_eq!(polar_arg(&p).unwrap(), crate::core::routing::library::original("medium-cruiser").unwrap());
    let imoca = call(&app(), "polar_original", &json!({ "id": "imoca-60" }));
    assert_eq!(polar_arg(&imoca).unwrap(), crate::core::routing::library::original("imoca-60").unwrap());
    assert!(call(&app(), "polar_original", &json!({ "id": "nothing" }))["error"].is_string());
}

#[test]
fn the_library_lists_every_built_in_polar_and_one_is_active() {
    let app = app();
    let l = call(&app, "polar_list", &Value::Null);
    let rows = l["polars"].as_array().unwrap();
    assert!(rows.len() >= 6);
    assert_eq!(rows.iter().filter(|r| r["active"] == true).count(), 1);
    assert_eq!(l["active"], "medium-cruiser");
    assert!(rows.iter().any(|r| r["id"] == "ultim" && r["builtin"] == true && r["name"] == "Ultim trimaran"));
    let got = call(&app, "polar_get", &Value::Null);
    assert_eq!((got["id"].as_str(), got["active"].as_bool()), (Some("medium-cruiser"), Some(true)));
    assert!(call(&app, "polar_activate", &json!({ "id": "../x" }))["error"].is_string());
    assert!(call(&app, "polar_save", &json!({ "id": "ultim", "tws": [], "twa": [], "speeds": [] }))["error"].is_string(), "an empty table is refused");
}

#[test]
fn connections_are_listed_saved_and_removed() {
    let app = app();
    let cfg = ConnectionConfig::new("gps", "GPS", crate::core::comm::source::Kind::Simulator { lat: 48.0, lon: -4.0, cog: 90.0, sog: 5.0 });
    let r = call(&app, "connection_save", &serde_json::to_value(&cfg).unwrap());
    assert!(r.get("error").is_none(), "{r}");
    let list = call(&app, "connections", &Value::Null);
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(app.settings.borrow().connections.len(), 1);
    call(&app, "connection_remove", &json!({ "id": "gps" }));
    assert!(call(&app, "connections", &Value::Null).as_array().unwrap().is_empty());
    app.hub.borrow_mut().stop_all();
}

#[test]
fn departures_parse_or_are_refused() {
    assert_eq!(parse_departure(""), Ok(None));
    assert_eq!(parse_departure("   "), Ok(None));
    let t = parse_departure("2026-06-01 14:30").unwrap().unwrap();
    // Back in local time it reads as typed.
    assert_eq!(t.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string(), "2026-06-01 14:30");
    assert!(parse_departure("tomorrow").is_err());
    assert!(parse_departure("2026-13-01 10:00").is_err());
    assert!(parse_departure("2026-06-01").is_err());
}

#[test]
fn a_form_saved_without_one_of_its_fields_is_refused_not_filled_with_nothing() {
    let app = app();
    let id = app.session.borrow_mut().add_waypoint(LatLon::new(48.0, -4.0), Some("Mark".into())).unwrap().id;
    let whole = call(&app, "waypoint_get", &json!({ "id": id }));
    for field in ["name", "latitude", "longitude", "description", "arrival_radius_nm", "visible"] {
        let mut w = whole.clone();
        w.as_object_mut().unwrap().remove(field);
        let r = call(&app, "waypoint_save", &w);
        assert!(r["error"].as_str().is_some_and(|e| e.contains(field)), "{field}: {r}");
    }
    // Nothing was written: the mark is as it was, still shown.
    assert_eq!(call(&app, "waypoint_get", &json!({ "id": id })), whole);
    assert_eq!(whole["visible"], true);
    // A number where text is wanted is refused too.
    let mut w = whole.clone();
    w["visible"] = json!("yes");
    assert!(call(&app, "waypoint_save", &w)["error"].is_string());

    assert!(call(&app, "route_save", &json!({ "id": 1, "name": "Out", "departure": "", "visible": true }))["error"].as_str().unwrap().contains("speed_kn"));
    assert!(call(&app, "auto_run", &json!({ "id": 1, "clearance_nm": 0.15 }))["error"].as_str().unwrap().contains("safety_depth"));
    assert!(call(&app, "polar_save", &json!({ "id": "ultim", "tws": [10.0], "twa": [90.0] }))["error"].as_str().unwrap().contains("speeds"));
}

/// The call names in `text`: the string that opens the arguments of one of the `wrappers`
/// (`send("x"`, `call<Rows>("x"`).
fn names_in(text: &str, wrappers: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    for word in wrappers {
        let mut rest = text;
        while let Some(at) = rest.find(word) {
            // `send(`, not `resend(`.
            let whole_word = !rest[..at].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_');
            rest = &rest[at + word.len()..];
            // A type between the name and its arguments, in Kotlin.
            let mut args = rest;
            if args.starts_with('<')
                && let Some(end) = args.find('>')
            {
                args = &args[end + 1..];
            }
            let Some(inside) = args.strip_prefix("(\"") else { continue };
            let name: String = inside.chars().take_while(|c| c.is_ascii_lowercase() || *c == '_').collect();
            if whole_word && !name.is_empty() && inside[name.len()..].starts_with('"') {
                found.push(name);
            }
        }
    }
    found
}

fn sources(dir: &std::path::Path, ext: &str, into: &mut String) {
    for e in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())).flatten() {
        let p = e.path();
        if p.is_dir() {
            sources(&p, ext, into);
        } else if p.extension().is_some_and(|x| x == ext) {
            into.push_str(&std::fs::read_to_string(&p).unwrap());
            into.push('\n');
        }
    }
}

#[test]
fn every_call_the_other_shells_make_exists() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    // The calls answered: the arms of each group's match, `"name" =>` or `"a" | "b" =>`.
    let mut rust = String::new();
    sources(&root.join("crates/isomar-app/src/api"), "rs", &mut rust);
    let known: std::collections::BTreeSet<String> = rust
        .lines()
        .filter(|l| l.starts_with("        \"") && l.contains("\" =>"))
        .flat_map(|l| l[..l.find("=>").unwrap()].split('|').map(|n| n.trim().trim_matches('"').to_string()).collect::<Vec<_>>())
        .collect();
    assert!(known.len() > 100 && known.contains("passage") && known.contains("polar_original"), "{} calls found", known.len());

    // The wrappers of `isomar_call` in Model.swift, and of `Native.call` in Model.kt.
    for (dir, ext, wrappers) in [("macos", "swift", &["send", "dict", "list", "call", "callText"][..]), ("android/app/src/main/java", "kt", &["send", "dict", "call", "callText"][..])] {
        let mut text = String::new();
        sources(&root.join(dir), ext, &mut text);
        // "nonsense" is the Swift test of what an unknown call answers.
        let used: std::collections::BTreeSet<String> = names_in(&text, wrappers).into_iter().filter(|n| n != "nonsense").collect();
        assert!(used.len() > 40, "{dir}: only {} calls found, the scan is broken", used.len());
        let unknown: Vec<&String> = used.difference(&known).collect();
        assert!(unknown.is_empty(), "{dir} calls what the engine does not answer: {unknown:?}");
    }
}
