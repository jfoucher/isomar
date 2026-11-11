//! Currents: arrows on the chart, routes in the stream, what is loaded.

use super::{s, to};
use crate::app::AppState;
use crate::flows;
use crate::platform::spawn;
use serde_json::{Value, json};
use std::rc::Rc;

/// Answers `name` if it is one of these calls.
pub(super) fn call(app: &Rc<AppState>, name: &str, args: &Value) -> Option<Value> {
    Some(match name {
        "currents_get" => to(&crate::currents::page(app)),
        "currents_arrows" => {
            {
                let mut w = app.weather.borrow_mut();
                w.currents.arrows = args.get("on").and_then(|v| v.as_bool()).unwrap_or(!w.currents.arrows);
            }
            app.refresh();
            json!({})
        }
        "currents_route_with" => {
            let on = args.get("on").and_then(|v| v.as_bool()).unwrap_or(!app.weather.borrow().currents.route_with);
            flows::routing::set_route_with(app, on);
            app.refresh();
            json!({})
        }
        "currents_fetch" => {
            let app = app.clone();
            spawn(async move {
                flows::currents::fetch_view(&app).await;
            });
            json!({})
        }
        "atlas_install" => {
            let (app, id) = (app.clone(), s(args, "id"));
            spawn(async move {
                crate::currents::atlases::install(&app, &id).await;
            });
            json!({})
        }
        "atlas_remove" => {
            crate::currents::atlases::remove(app, &s(args, "id"));
            json!({})
        }
        "currents_clear" => {
            app.weather.borrow_mut().currents.clear();
            app.refresh();
            json!({})
        }

        _ => return None,
    })
}
