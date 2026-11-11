//! Charts to install and the folders they are read from, and the tide at a place.

use super::{error, f, s, to};
use crate::app::AppState;
use crate::core::geo::LatLon;
use crate::flows;
use crate::platform::spawn;
use chrono::Utc;
use serde_json::{Value, json};
use std::rc::Rc;

/// Answers `name` if it is one of these calls.
pub(super) fn call(app: &Rc<AppState>, name: &str, args: &Value) -> Option<Value> {
    Some(match name {
        "charts_install" => match flows::charts::entry(&s(args, "id")) {
            Some(e) => {
                let app = app.clone();
                spawn(async move {
                    flows::charts::install_entry(&app, e).await;
                });
                json!({})
            }
            None => error("unknown source"),
        },
        "charts_uninstall" => match flows::charts::entry(&s(args, "id")) {
            Some(e) => {
                let app = app.clone();
                spawn(async move {
                    flows::charts::uninstall_entry(&app, e).await;
                });
                json!({})
            }
            None => error("unknown source"),
        },
        "charts_service_on" => {
            flows::charts::turn_service_on(app);
            app.refresh();
            json!({})
        }
        "charts_add_folder" => {
            flows::charts::add_folder(app, &s(args, "path"));
            json!({})
        }
        "charts_remove_folder" => {
            flows::charts::remove_folder(app, &s(args, "path"));
            json!({})
        }
        "charts_here" => to(&flows::charts::rows_for(app, &flows::charts::entries_here(LatLon::new(f(args, "lat"), f(args, "lon"))))),

        // Tides.
        "tides" => {
            let (app, pos) = (app.clone(), LatLon::new(f(args, "lat"), f(args, "lon")));
            spawn(async move {
                if let Some(info) = flows::tides::info(&app, pos).await
                    && let Some(shell) = app.shell()
                {
                    shell.object_info(&crate::aidinfo::Description { title: info.title.clone(), subtitle: None, rows: info.groups.iter().flat_map(|g| g.1.clone()).collect(), choice: None }, 0.0, 0.0);
                }
            });
            json!({})
        }
        "tide_info" => {
            // The tide for a position, as groups and a curve, in the answer itself when it is kept.
            let pos = LatLon::new(f(args, "lat"), f(args, "lon"));
            let now = Utc::now();
            match crate::core::tides::lookup(&crate::core::tides::cache_dir(), pos.lat, pos.lon, now, 40) {
                Ok((fc, origin)) => to(&flows::tides::describe(&fc, origin, now)),
                Err(e) => error(flows::tides::explain(&e)),
            }
        }

        _ => return None,
    })
}
