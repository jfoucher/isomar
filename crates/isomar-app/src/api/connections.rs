//! The instrument connections and the data monitor.

use super::{error, i, s};
use crate::app::AppState;
use crate::core::comm::source::{ConnectionConfig, serial_ports};
use crate::i18n::{gettext, gettext_f};
use chrono::Local;
use serde_json::{Value, json};
use std::rc::Rc;

fn connection_rows(app: &AppState) -> Value {
    let status = app.session.borrow().conn_status.clone();
    let rows: Vec<Value> = app
        .hub
        .borrow()
        .configs()
        .into_iter()
        .map(|c| {
            let st = match status.get(&c.id) {
                Some(crate::core::comm::source::Status::Connected) => gettext("Connected"),
                Some(crate::core::comm::source::Status::Connecting) => gettext("Connecting…"),
                Some(crate::core::comm::source::Status::Disconnected(m)) => gettext_f("Disconnected: {error}", &[("error", m)]),
                Some(crate::core::comm::source::Status::Finished) => gettext("Finished"),
                None => gettext("Not started"),
            };
            json!({ "config": c, "describe": c.describe(), "status": st })
        })
        .collect();
    json!(rows)
}

/// Answers `name` if it is one of these calls.
pub(super) fn call(app: &Rc<AppState>, name: &str, args: &Value) -> Option<Value> {
    Some(match name {
        "connections" => connection_rows(app),
        "connection_save" => match serde_json::from_value::<ConnectionConfig>(args.clone()) {
            Ok(cfg) => {
                let id = cfg.id.clone();
                {
                    let mut hub = app.hub.borrow_mut();
                    hub.remove(&id);
                    hub.add(cfg);
                }
                app.settings.borrow_mut().connections = crate::host::saved_connections(app);
                app.apply_settings();
                json!({})
            }
            Err(e) => error(e.to_string()),
        },
        "connection_remove" => {
            app.hub.borrow_mut().remove(&s(args, "id"));
            app.settings.borrow_mut().connections = crate::host::saved_connections(app);
            app.apply_settings();
            json!({})
        }
        "serial_ports" => json!(serial_ports()),

        // The data monitor.
        "monitor" => {
            let since = i(args, "since");
            let sess = app.session.borrow();
            let lines: Vec<Value> = sess.monitor.iter().skip(since.max(0) as usize).map(|l| json!({ "time": l.time.with_timezone(&Local).format("%H:%M:%S").to_string(), "source": l.source, "text": l.text, "ok": l.ok })).collect();
            json!({ "lines": lines, "total": sess.monitor.len(), "bad": sess.bad_lines })
        }

        _ => return None,
    })
}
