//! A JSON-in, JSON-out door to everything a shell's forms and pages need, so that a shell written
//! in another language (the macOS app) needs no binding for each call. `call(app, name, args)`
//! returns what the page shows or does what the form asked and answers `{}`; long work (a
//! download, a route search) is started and shows as a job.

//!
//! The calls are grouped by what they are about, one file each; `call` asks each group in turn.
//! A call that reads takes its arguments leniently (`s`, `f`, `i`, `b`: nothing for a field left
//! out). One that saves a form reads them into a record with `parse`, so a field that is missing
//! or of the wrong type is an error the shell is told, never a zero written over the user's data.

mod charts;
mod connections;
mod currents;
mod departures;
mod nav;
mod perf;
mod polars;
mod routing;
mod settings;
mod weather;

use crate::app::AppState;
use crate::flows::{self, routing::BoatParams};
use crate::vm;
use chrono::{Local, NaiveDateTime, TimeZone, Utc};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::rc::Rc;

fn to<T: Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

fn s(args: &Value, key: &str) -> String {
    args.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn f(args: &Value, key: &str) -> f64 {
    args.get(key).and_then(|v| v.as_f64()).unwrap_or(0.0)
}

fn i(args: &Value, key: &str) -> i64 {
    args.get(key).and_then(|v| v.as_i64()).unwrap_or(0)
}

fn b(args: &Value, key: &str) -> bool {
    args.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

/// A call's arguments as the record it needs, or the error to answer with.
fn parse<T: DeserializeOwned>(args: &Value) -> Result<T, Value> {
    serde_json::from_value(args.clone()).map_err(|e| error(format!("bad arguments: {e}")))
}

/// A route name as a file name: no path separators or characters some file systems refuse.
pub fn file_stem(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '_' } else { c }).collect();
    let s = s.trim().trim_start_matches('.');
    if s.is_empty() { "route".into() } else { s.into() }
}

fn error(text: impl Into<String>) -> Value {
    json!({ "error": text.into() })
}

/// "2026-10-27 15:30" in local time, as the departure fields have it.
#[allow(clippy::result_unit_err)]
pub fn parse_departure(text: &str) -> Result<Option<chrono::DateTime<Utc>>, ()> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    // A `T` for the space is taken too (the form of ISO 8601).
    let naive = NaiveDateTime::parse_from_str(&text.replacen('T', " ", 1), "%Y-%m-%d %H:%M").map_err(|_| ())?;
    match Local.from_local_datetime(&naive) {
        chrono::LocalResult::Single(t) | chrono::LocalResult::Ambiguous(t, _) => Ok(Some(t.with_timezone(&Utc))),
        chrono::LocalResult::None => Err(()),
    }
}

fn boat_of(args: &Value) -> BoatParams {
    let d = BoatParams::default();
    let v = args.get("boat").cloned().unwrap_or(Value::Null);
    BoatParams {
        performance_pct: v.get("performance_pct").and_then(|x| x.as_f64()).unwrap_or(d.performance_pct),
        use_engine: v.get("use_engine").and_then(|x| x.as_bool()).unwrap_or(d.use_engine),
        motor_kn: v.get("motor_kn").and_then(|x| x.as_f64()).unwrap_or(d.motor_kn),
        max_wind_kn: v.get("max_wind_kn").and_then(|x| x.as_f64()).unwrap_or(d.max_wind_kn),
    }
}

/// The forecast model an action names (`"model": "ifs"`), else the one shown.
fn model_of(app: &AppState, args: &Value) -> crate::core::weather::source::Source {
    args.get("model").and_then(|m| m.as_str()).and_then(crate::core::weather::source::Source::from_id).unwrap_or_else(|| app.weather.borrow().source)
}

/// The forecast models an action names (`"models": ["gfs", "ifs"]`), else the one it names as
/// `"model"`, else the one shown.
fn models_of(app: &AppState, args: &Value) -> Vec<crate::core::weather::source::Source> {
    let named: Vec<_> = args.get("models").and_then(|m| m.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).filter_map(crate::core::weather::source::Source::from_id).collect()).unwrap_or_default();
    if named.is_empty() {
        return vec![model_of(app, args)];
    }
    // Remembered for the next weather route or best departure.
    let models = flows::routing::models_of(app, &named);
    app.settings.borrow_mut().routing_sources = models.clone();
    models
}

/// The pages and pieces of chrome, as the records of `vm`.
fn pages(app: &Rc<AppState>, name: &str) -> Option<Value> {
    Some(match name {
        "passage" => to(&vm::passage::passage(app)),
        "traffic" => to(&vm::traffic::vessels(app, 60)),
        "instruments" => to(&vm::instruments::tiles(app)),
        "alerts" => to(&vm::banner::alerts(app)),
        "footer" => to(&vm::footer::footer(app)),
        "window" => to(&vm::window::window(app)),
        "underway" => to(&vm::underway::underway(app)),
        "timeline" => to(&vm::timeline::timeline(app)),
        "charts" => to(&vm::charts::page(app)),
        "router_card" => to(&vm::router::card(app)),
        "departures" => to(&vm::departures::departures(app)),
        "view" => {
            let v = app.canvas.view_state();
            json!({ "lat": v.lat, "lon": v.lon, "zoom": v.zoom })
        }

        _ => return None,
    })
}

/// A shell asks for `name` with `args`.
pub fn call(app: &Rc<AppState>, name: &str, args: &Value) -> Value {
    pages(app, name)
        .or_else(|| weather::call(app, name, args))
        .or_else(|| currents::call(app, name, args))
        .or_else(|| charts::call(app, name, args))
        .or_else(|| routing::call(app, name, args))
        .or_else(|| departures::call(app, name, args))
        .or_else(|| nav::call(app, name, args))
        .or_else(|| settings::call(app, name, args))
        .or_else(|| connections::call(app, name, args))
        .or_else(|| polars::call(app, name, args))
        .or_else(|| perf::call(app, name, args))
        .unwrap_or_else(|| error(format!("unknown call {name}")))
}

#[cfg(test)]
mod tests;
