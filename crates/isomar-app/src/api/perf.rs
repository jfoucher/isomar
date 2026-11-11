//! The performance log: its page, the sailing mode, calibration, and what it teaches the polar.

use super::polars::{polar_arg, polar_json};
use super::{b, error, f, s, to};
use crate::app::AppState;
use crate::flows;
use crate::i18n::gettext;
use crate::vm;
use serde_json::{Value, json};
use std::rc::Rc;

/// Answers `name` if it is one of these calls.
pub(super) fn call(app: &Rc<AppState>, name: &str, args: &Value) -> Option<Value> {
    Some(match name {
        "perf_get" => {
            if flows::perf::stale(app) {
                flows::perf::analyse_now(app);
            }
            to(&vm::performance::performance(app))
        }
        // The page's record as it is, without starting an analysis (for a shell refreshing a page on show).
        "perf_page" => to(&vm::performance::performance(app)),
        "perf_switch" => to(&vm::performance::switch(app)),
        // The polar an editor holds, with the learnt speeds put in where the log is confident.
        "polar_with_learnt" => {
            let mut p = match polar_arg(args) {
                Ok(p) => p,
                Err(e) => return Some(e),
            };
            let report = app.perf.borrow().report.clone();
            match report.as_ref().and_then(|r| r.learned.as_ref()) {
                Some(l) if !p.twa.is_empty() && !p.tws.is_empty() => {
                    let n = flows::perf::merge_learned(&mut p, l, vm::performance::APPLY_CONFIDENCE);
                    let mut v = polar_json(&p);
                    v["changed"] = json!(n);
                    v
                }
                _ => error(gettext("Nothing learnt yet")),
            }
        }
        "perf_set_mode" => {
            if flows::perf::set_mode(app, crate::core::perf::SailMode::parse(&s(args, "mode"))) {
                json!({})
            } else {
                error(gettext("An engine is reporting: it sets the mode."))
            }
        }
        "perf_set_recording" => {
            app.settings.borrow_mut().perf_recording = b(args, "on");
            app.apply_settings();
            json!({})
        }
        "perf_analyse" => {
            flows::perf::analyse_now(app);
            json!({})
        }
        "perf_calibration_save" => {
            let est = app.perf.borrow().report.as_ref().map(|r| r.estimate.clone()).filter(|c| !c.is_empty());
            let Some(est) = est else { return Some(error(gettext("The log says nothing about the instruments yet."))) };
            app.settings.borrow_mut().perf_calibration = Some(est);
            app.apply_settings();
            app.perf.borrow_mut().analysed_count = usize::MAX;
            flows::perf::analyse_now(app);
            json!({})
        }
        "perf_calibration_clear" => {
            app.settings.borrow_mut().perf_calibration = None;
            app.apply_settings();
            app.perf.borrow_mut().analysed_count = usize::MAX;
            flows::perf::analyse_now(app);
            json!({})
        }
        "perf_apply_polar" => {
            let min = args.get("min_confidence").and_then(|v| v.as_f64()).unwrap_or(vm::performance::APPLY_CONFIDENCE);
            match flows::perf::apply_polar(app, min) {
                Ok(n) => {
                    app.refresh();
                    json!({ "changed": n })
                }
                Err(e) => error(e),
            }
        }
        "perf_undo_polar" => match flows::perf::undo_polar(app) {
            Ok(()) => {
                app.refresh();
                json!({})
            }
            Err(e) => error(e),
        },
        "perf_apply_factor" => {
            flows::perf::apply_factor(app, f(args, "pct"));
            json!({})
        }
        "perf_backfill" => {
            flows::perf::backfill(app);
            json!({})
        }
        "perf_export_csv" => match flows::perf::export_csv(app, std::path::Path::new(&s(args, "path"))) {
            Ok(n) => json!({ "rows": n }),
            Err(e) => error(e),
        },
        "perf_clear" => {
            flows::perf::clear(app);
            app.refresh();
            json!({})
        }
        "polar_observed" => {
            if flows::perf::stale(app) {
                flows::perf::analyse_now(app);
            }
            vm::performance::observed(app).map(|o| to(&o)).unwrap_or(Value::Null)
        }

        _ => return None,
    })
}
