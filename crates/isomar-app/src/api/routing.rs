//! Weather routes: computing one, choosing among the ways found, its statistics and replay; and
//! the automatic route.

use super::{boat_of, error, i, models_of, parse, parse_departure, s};
use crate::app::AppState;
use crate::flows;
use crate::i18n::gettext;
use crate::platform::spawn;
use crate::vm;
use chrono::{Local, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use std::rc::Rc;

/// What "Automatic route" is asked with: a depth or clearance left out is not taken for zero.
#[derive(Deserialize)]
struct AutoRun {
    id: i64,
    safety_depth: f64,
    clearance_nm: f64,
    #[serde(default)]
    download: bool,
}

/// Answers `name` if it is one of these calls.
pub(super) fn call(app: &Rc<AppState>, name: &str, args: &Value) -> Option<Value> {
    Some(match name {
        "route_defaults" => {
            let id = i(args, "id");
            let s = app.session.borrow();
            let Some(r) = s.routes.iter().find(|r| r.id == id) else { return Some(Value::Null) };
            json!({
                "from": r.points.first().map(|p| p.name.clone()).unwrap_or_default(),
                "to": r.points.last().map(|p| p.name.clone()).unwrap_or_default(),
                "departure": r.departure.unwrap_or_else(Utc::now).with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string(),
                "boat": { "performance_pct": flows::routing::default_performance(app), "use_engine": false, "motor_kn": 5.0, "max_wind_kn": 35.0 },
                "model": app.weather.borrow().source.id(),
                "models": app.settings.borrow().routing_sources.iter().map(|m| m.id()).collect::<Vec<_>>(),
                "currents": flows::routing::currents_choice(app, &r.points.iter().map(|p| p.pos).collect::<Vec<_>>()),
            })
        }
        "route_compute" if flows::routing::is_weather_route(app, i(args, "id")) => error(gettext("This route was made by weather routing: route the route it came from instead.")),
        "route_compute" => match parse_departure(&s(args, "departure")) {
            Ok(Some(depart)) => {
                if let Some(on) = args.get("currents").and_then(|v| v.as_bool()) {
                    flows::routing::set_route_with(app, on);
                }
                let (app, req) = (app.clone(), flows::routing::RouteRequest { route_id: i(args, "id"), depart, boat: boat_of(args), sources: models_of(app, args) });
                spawn(async move {
                    flows::routing::run(&app, req).await;
                });
                json!({})
            }
            _ => error(gettext("The departure is not a date and time (YYYY-MM-DD HH:MM).")),
        },
        "router_select" => {
            vm::router::select(app, i(args, "i") as usize);
            json!({})
        }
        "router_use" => {
            crate::router::use_choice(app, i(args, "i") as usize);
            json!({})
        }
        "router_remove" => {
            crate::router::remove_choice(app, i(args, "i") as usize);
            json!({})
        }
        "router_discard" => {
            vm::router::discard(app);
            json!({})
        }
        // The plan a weather route was made from: its statistics, and playing it back.
        "route_plan_stats" => match vm::plan::stats(app, i(args, "id")) {
            Some((heads, sections)) => json!({ "heads": heads, "sections": sections }),
            None => Value::Null,
        },
        "route_replay" => json!({ "ok": crate::router::replay(app, i(args, "id")) }),
        "route_ensemble" => {
            let (app, id) = (app.clone(), i(args, "id"));
            spawn(async move {
                flows::ensemble::check(&app, id).await;
            });
            json!({})
        }
        "router_focus_model" => {
            vm::router::focus_model(app, i(args, "i") as usize);
            json!({})
        }
        "router_cross" => match vm::router::cross_check(app) {
            Some((heads, sections)) => json!({ "heads": heads, "sections": sections }),
            None => Value::Null,
        },
        "router_stats" => match vm::router::stats(app) {
            Some((heads, sections)) => json!({ "heads": heads, "sections": sections }),
            None => Value::Null,
        },
        "auto_defaults" => {
            let st = app.settings.borrow();
            json!({ "safety_depth": st.safety_depth, "clearance_nm": st.auto_clearance_nm, "offline": crate::debug::offline() })
        }
        "auto_run" => {
            let a: AutoRun = match parse(args) {
                Ok(a) => a,
                Err(e) => return Some(e),
            };
            let Some(source) = app.session.borrow().routes.iter().find(|r| r.id == a.id).cloned() else { return Some(Value::Null) };
            let settings = crate::core::routing::auto::AutoSettings { safety_depth_m: a.safety_depth, clearance_nm: a.clearance_nm, ..Default::default() };
            if (settings.clearance_nm - app.settings.borrow().auto_clearance_nm).abs() > f64::EPSILON {
                app.settings.borrow_mut().auto_clearance_nm = settings.clearance_nm;
                app.apply_settings();
            }
            let (app, download) = (app.clone(), a.download && !crate::debug::offline());
            spawn(async move {
                flows::autoroute::compute(&app, source, settings, download, &|_| {}).await;
            });
            json!({})
        }

        _ => return None,
    })
}
