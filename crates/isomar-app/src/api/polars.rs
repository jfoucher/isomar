//! The polar library: the built-in polars and the user's, one of them active.

use super::{b, error, parse, s, to};
use crate::app::AppState;
use crate::core::routing::polar::Polar;
use crate::i18n::gettext;
use serde::Deserialize;
use serde_json::{Value, json};
use std::rc::Rc;

pub(super) fn polar_json(p: &Polar) -> Value {
    json!({ "tws": p.tws, "twa": p.twa, "speeds": p.speeds })
}

/// The table of a call's `tws`, `twa` and `speeds`: all three are needed, and one left out or
/// misspelt is an error rather than an empty table.
pub(super) fn polar_arg(args: &Value) -> Result<Polar, Value> {
    #[derive(Deserialize)]
    struct Table {
        tws: Vec<f64>,
        twa: Vec<f64>,
        speeds: Vec<Vec<f64>>,
    }
    parse::<Table>(args).map(|t| Polar { tws: t.tws, twa: t.twa, speeds: t.speeds })
}

/// A polar of the library with what a list says of it.
fn library_polar_json(app: &AppState, id: &str) -> Value {
    let lib = crate::polars::library();
    let Some(p) = lib.load(id) else { return error(gettext("There is no such polar")) };
    let row = crate::polars::list(app).polars.into_iter().find(|r| r.entry.id == id);
    let mut v = polar_json(&p);
    v["id"] = json!(id);
    if let Some(r) = row {
        v["name"] = json!(r.entry.name);
        v["builtin"] = json!(r.entry.builtin);
        v["modified"] = json!(r.entry.modified);
        v["active"] = json!(r.active);
        v["samples"] = json!(r.samples);
    }
    v
}

/// Answers `name` if it is one of these calls.
pub(super) fn call(app: &Rc<AppState>, name: &str, args: &Value) -> Option<Value> {
    Some(match name {
        "polar_list" => to(&crate::polars::list(app)),
        // A polar (`id`, else the active one) with its name and whether it is active.
        "polar_get" => {
            let id = args.get("id").and_then(|v| v.as_str()).map(str::to_string).unwrap_or_else(|| crate::polars::active_id(app));
            library_polar_json(app, &id)
        }
        // A built-in polar as published (`id`, else the default one).
        "polar_default" | "polar_original" => {
            let id = args.get("id").and_then(|v| v.as_str()).unwrap_or(crate::core::routing::library::DEFAULT_ID);
            crate::core::routing::library::original(id).map(|p| polar_json(&p)).unwrap_or_else(|| error(gettext("There is no such polar")))
        }
        // Saves the speeds over polar `id`, else the active one.
        "polar_save" => {
            let id = args.get("id").and_then(|v| v.as_str()).map(str::to_string).unwrap_or_else(|| crate::polars::active_id(app));
            let table = match polar_arg(args) {
                Ok(p) => p,
                Err(e) => return Some(e),
            };
            match crate::polars::save(app, &id, &table) {
                Ok(()) => json!({ "id": id }),
                Err(e) => error(e),
            }
        }
        // Saves the speeds as a new polar named `name`; `activate` makes it the active one.
        "polar_save_as" => {
            let table = match polar_arg(args) {
                Ok(p) => p,
                Err(e) => return Some(e),
            };
            match crate::polars::save_as(app, &s(args, "name"), &table) {
                Ok(id) => {
                    if b(args, "activate")
                        && let Err(e) = crate::polars::activate(app, &id)
                    {
                        return Some(error(e));
                    }
                    json!({ "id": id })
                }
                Err(e) => error(e),
            }
        }
        "polar_activate" => match crate::polars::activate(app, &s(args, "id")) {
            Ok(()) => json!({}),
            Err(e) => error(e),
        },
        // Deletes a polar of the user's (and its samples), or takes a built-in one back to the original.
        "polar_remove" => match crate::polars::remove(app, &s(args, "id")) {
            Ok(()) => json!({ "active": crate::polars::active_id(app) }),
            Err(e) => error(e),
        },
        "polar_import" => match std::fs::read_to_string(s(args, "path")).map_err(|e| e.to_string()).and_then(|t| Polar::parse(&t).map_err(|e| e.to_string())) {
            Ok(p) => polar_json(&p),
            Err(e) => error(e),
        },

        _ => return None,
    })
}
