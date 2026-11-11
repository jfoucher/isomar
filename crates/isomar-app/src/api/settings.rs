//! The settings: read whole, changed by a patch of the fields that differ.

use super::{error, to};
use crate::app::AppState;
use serde_json::{Value, json};
use std::rc::Rc;

/// Answers `name` if it is one of these calls.
pub(super) fn call(app: &Rc<AppState>, name: &str, args: &Value) -> Option<Value> {
    Some(match name {
        "settings_get" => to(&*app.settings.borrow()),
        // The app goes to the background: what should outlive it is saved, and it goes on.
        "save_state" => {
            crate::host::save(app);
            Value::Null
        }
        "settings_patch" => {
            let mut v = to(&*app.settings.borrow());
            if let (Some(o), Some(p)) = (v.as_object_mut(), args.as_object()) {
                for (k, x) in p {
                    o.insert(k.clone(), x.clone());
                }
            }
            match serde_json::from_value::<crate::config::Settings>(v) {
                Ok(new) => {
                    *app.settings.borrow_mut() = new;
                    app.apply_settings();
                    json!({})
                }
                Err(e) => error(e.to_string()),
            }
        }

        _ => return None,
    })
}
