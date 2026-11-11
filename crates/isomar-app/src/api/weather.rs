//! The forecast: its page and readout, what is drawn, getting one, and the timeline.

use super::{b, error, f, i, model_of, s};
use crate::app::AppState;
use crate::core::geo::{self, LatLon};
use crate::core::weather::planner::Area;
use crate::flows::{self, weather::Choices};
use crate::i18n::gettext;
use crate::platform::spawn;
use crate::vm;
use chrono::{Local, Utc};
use serde_json::{Value, json};
use std::rc::Rc;

fn choices_of(app: &AppState, args: &Value) -> Choices {
    Choices {
        source: model_of(app, args),
        north: s(args, "north"),
        south: s(args, "south"),
        west: s(args, "west"),
        east: s(args, "east"),
        horizon_index: i(args, "horizon_index") as usize,
        step_index: i(args, "step_index") as usize,
        groups: args
            .get("groups")
            .and_then(|g| g.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str()).filter_map(group_of).collect())
            .unwrap_or_default(),
    }
}

fn group_of(name: &str) -> Option<crate::core::weather::model::Group> {
    flows::weather::GROUPS.iter().map(|g| g.0).find(|g| group_name(*g) == name)
}

fn group_name(g: crate::core::weather::model::Group) -> &'static str {
    use crate::core::weather::model::Group;
    match g {
        Group::Wind => "wind",
        Group::Gust => "gust",
        Group::Pressure => "pressure",
        Group::Waves => "waves",
        Group::Temperature => "temperature",
        Group::Cloud => "cloud",
        #[allow(unreachable_patterns)]
        _ => "other",
    }
}

fn weather_page(app: &AppState) -> Value {
    let w = app.weather.borrow();
    let range = w.store.time_range();
    let fmt = |t: chrono::DateTime<Utc>| t.with_timezone(&Local).format("%a %d %b %H:%M").to_string();
    let area = |r: (LatLon, LatLon)| {
        let a = crate::weather::rect_area(r);
        json!({ "north": geo::format_lat(a.north), "south": geo::format_lat(a.south), "west": geo::format_lon(a.west), "east": geo::format_lon(a.east) })
    };
    json!({
        "model": w.source.id(),
        "model_label": w.source.label(),
        "models": flows::weather::model_choices(app),
        "tracking": flows::perf::tracking(app),
        "loaded": !w.store.is_empty(),
        "fields": w.store.field_count(),
        "run": w.store.latest_run().map(|r| r.format("%d %b %H UTC").to_string()),
        "from": range.map(|r| fmt(r.0)),
        "to": range.map(|r| fmt(r.1)),
        // What colours the sea (one field: "none", "wind" or "waves"), and what is drawn over it.
        "fill": w.fill.name(), "barbs": w.barbs, "pressure": w.pressure, "wave_arrows": w.wave_arrows,
        // Why the wave colours and arrows cannot show: the forecast loaded has none. A weather route
        // downloads wind and pressure only.
        "no_waves": (!w.store.is_empty() && !w.store.has(crate::core::weather::model::Param::WaveHeight))
            .then(|| gettext("This forecast has no waves: get weather with Waves ticked to see them.")),
        "currents": crate::currents::page(app),
        "picking": !matches!(w.pick, crate::weather::Pick::Off),
        "rect": w.rect.map(area),
        "horizons": flows::weather::HORIZONS.iter().map(|(_, l)| gettext(*l)).collect::<Vec<_>>(),
        "steps": [gettext("Every hour"), gettext("Every 3 hours"), gettext("Every 6 hours")],
        "groups": flows::weather::GROUPS.iter().map(|(g, label, on)| json!({ "id": group_name(*g), "label": gettext(*label), "default": on })).collect::<Vec<_>>(),
    })
}

fn weather_readout(app: &AppState) -> Value {
    let w = app.weather.borrow();
    let Some(p) = app.pointer.get().filter(|_| w.has_any()) else { return Value::Null };
    let chips = w.readout_at(p.lat, p.lon);
    if chips.is_empty() { Value::Null } else { json!({ "chips": chips, "time": w.time.with_timezone(&Local).format("%a %d %b %H:%M").to_string() }) }
}

/// Answers `name` if it is one of these calls.
pub(super) fn call(app: &Rc<AppState>, name: &str, args: &Value) -> Option<Value> {
    Some(match name {
        "weather_page" => weather_page(app),
        "weather_readout" => weather_readout(app),

        "weather_toggle" => {
            let mut w = app.weather.borrow_mut();
            match s(args, "name").as_str() {
                "barbs" => w.barbs = !w.barbs,
                "pressure" => w.pressure = !w.pressure,
                "wave_arrows" => w.wave_arrows = !w.wave_arrows,
                _ => {}
            }
            drop(w);
            app.refresh();
            json!({})
        }
        // The field that colours the sea: "none", "wind" or "waves".
        "weather_fill" => {
            if let Some(f) = crate::weather::Fill::from_name(&s(args, "fill")) {
                app.weather.borrow_mut().fill = f;
                app.refresh();
            }
            json!({})
        }
        "weather_clear" => {
            app.weather.borrow_mut().store.clear();
            app.refresh();
            json!({})
        }
        "weather_pick" => {
            crate::weather::start_pick(app);
            json!({})
        }
        "weather_cancel_pick" => {
            {
                let mut w = app.weather.borrow_mut();
                w.pick = crate::weather::Pick::Off;
                w.rect = None;
            }
            app.refresh();
            json!({})
        }
        "weather_defaults" => {
            // The area the sheet starts from: the view, a box round a point, or the box drawn.
            let a = match s(args, "kind").as_str() {
                "here" => Area::around(f(args, "lat"), f(args, "lon"), crate::weather::HERE_HALF_DEG),
                "rect" => app.weather.borrow().rect.map(crate::weather::rect_area).unwrap_or_else(|| Area::from_bounds(&app.canvas.view_info().bounds())),
                _ => Area::from_bounds(&app.canvas.view_info().bounds()),
            };
            json!({ "north": geo::format_lat(a.north), "south": geo::format_lat(a.south), "west": geo::format_lon(a.west), "east": geo::format_lon(a.east) })
        }
        "weather_estimate" => {
            let e = flows::weather::estimate(choices_of(app, args).request().as_ref());
            json!({ "source": e.source, "run": e.run, "size": e.size, "files": e.files, "missing": e.missing, "ok": e.ok })
        }
        "weather_show_model" => {
            flows::weather::show_source(app, model_of(app, args));
            json!({})
        }
        "weather_download" => match choices_of(app, args).request() {
            Some(req) => {
                app.weather.borrow_mut().rect = None;
                let app = app.clone();
                spawn(async move {
                    flows::weather::run_download(&app, req).await;
                });
                json!({})
            }
            None => error(gettext("The area is not valid.")),
        },

        // Timeline.
        "timeline_set" => {
            vm::timeline::set_time(app, i(args, "t"));
            json!({})
        }
        "timeline_step" => {
            vm::timeline::step(app, i(args, "hours"));
            json!({})
        }
        "timeline_play" => {
            vm::timeline::set_playing(app, b(args, "on"));
            json!({})
        }
        "timeline_speed" => {
            vm::timeline::set_speed(app, f(args, "x"));
            json!({})
        }

        _ => return None,
    })
}
