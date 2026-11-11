//! Best departure: the scan, and choosing among the departures it ranks.

use super::{boat_of, error, f, i, models_of, parse_departure, s, to};
use crate::app::AppState;
use crate::core::routing::window::Weights;
use crate::flows;
use crate::i18n::gettext;
use crate::platform::spawn;
use crate::vm;
use serde_json::{Value, json};
use std::rc::Rc;

fn weights_json(w: &Weights) -> Value {
    to(w)
}

/// Answers `name` if it is one of these calls.
pub(super) fn call(app: &Rc<AppState>, name: &str, args: &Value) -> Option<Value> {
    Some(match name {
        "window_defaults" => {
            let id = i(args, "id");
            let prefs = app.settings.borrow().weather_window.clone();
            let s = app.session.borrow();
            let Some(r) = s.routes.iter().find(|r| r.id == id) else { return Some(Value::Null) };
            json!({
                "from": r.points.first().map(|p| p.name.clone()).unwrap_or_default(),
                "to": r.points.last().map(|p| p.name.clone()).unwrap_or_default(),
                "earliest": flows::departures::next_slot(prefs.step_hours).format("%Y-%m-%d %H:%M").to_string(),
                "days": prefs.days, "step_hours": prefs.step_hours, "steps": flows::departures::STEPS_H,
                "weights": weights_json(&prefs.weights),
                "presets": flows::departures::presets().iter().map(|(n, w)| json!({ "name": n, "weights": weights_json(w) })).collect::<Vec<_>>(),
                "boat": { "performance_pct": flows::routing::default_performance(app), "use_engine": false, "motor_kn": 5.0, "max_wind_kn": 35.0 },
                "max_weight": crate::core::routing::window::MAX_WEIGHT,
                "cautious": prefs.cautious,
                "model": app.weather.borrow().source.id(),
                "models": app.settings.borrow().routing_sources.iter().map(|m| m.id()).collect::<Vec<_>>(),
                "currents": flows::routing::currents_choice(app, &r.points.iter().map(|p| p.pos).collect::<Vec<_>>()),
            })
        }
        "window_run" => match (parse_departure(&s(args, "earliest")), serde_json::from_value::<Weights>(args.get("weights").cloned().unwrap_or(Value::Null))) {
            (Ok(Some(earliest)), Ok(weights)) => {
                if let Some(on) = args.get("currents").and_then(|v| v.as_bool()) {
                    flows::routing::set_route_with(app, on);
                }
                let prefs = crate::config::WindowPrefs { weights, days: f(args, "days").max(1.0), step_hours: f(args, "step_hours").max(1.0), cautious: args.get("cautious").and_then(|v| v.as_bool()).unwrap_or_else(|| app.settings.borrow().weather_window.cautious) };
                let (app, req) = (app.clone(), flows::departures::WindowRequest { route_id: i(args, "id"), earliest, prefs, boat: boat_of(args), sources: models_of(app, args) });
                spawn(async move {
                    flows::departures::run(&app, req).await;
                });
                json!({})
            }
            _ => error(gettext("The earliest departure is not a date and time (YYYY-MM-DD HH:MM).")),
        },
        "departure_select" => {
            vm::departures::select(app, i(args, "i") as usize);
            json!({})
        }
        "departure_preview" => {
            let app = app.clone();
            spawn(async move {
                flows::departures::preview(&app).await;
            });
            json!({})
        }
        "departure_compare" => match vm::departures::compare(app) {
            Some((heads, sections)) => json!({ "heads": heads, "sections": sections }),
            None => Value::Null,
        },
        "departure_close" => {
            vm::departures::close(app);
            json!({})
        }

        _ => return None,
    })
}
