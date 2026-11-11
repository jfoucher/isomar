//! The card over the chart that previews a weather-route search: what it found, the alternatives
//! the user picked, and what each button does. The clock is the timeline's.

use crate::app::AppState;
use crate::core::geo::LatLon;
use crate::i18n::{gettext, gettext_f};
use crate::layers::router::{ALT_COLOURS, ALT_DASHES};
use crate::router::{alt_title, local, summary};
use serde::Serialize;
use std::rc::Rc;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct AltRow {
    pub index: usize,
    pub title: String,
    pub summary: Vec<crate::chips::Chip>,
    /// The colour's token name (`alt-1` …) and its dash pattern, so a swatch looks like the line.
    pub colour: String,
    pub dash: Vec<f64>,
    pub selected: bool,
    pub arrives: bool,
}

/// One forecast model's best way, when the passage was routed on several.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ModelRow {
    pub index: usize,
    pub label: String,
    pub summary: Vec<crate::chips::Chip>,
    /// Its colour's token name (`alt-1` …), as its track on the chart.
    pub colour: String,
    /// It is the one shown: its fronts, its wind, the alternatives below.
    pub focused: bool,
    pub arrives: bool,
    /// Its route holds up best whichever forecast is right.
    pub robust: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Card {
    pub title: String,
    pub note: String,
    /// The search failed to reach the destination: the card shows a warning, and the track to the
    /// end of the forecast is drawn in the stale colour.
    pub failure: bool,
    pub alternatives: Vec<AltRow>,
    /// A colour is free for another alternative.
    pub can_add: bool,
    /// A saved route's plan played back: nothing to pick or use, only to close.
    pub replay: bool,
    /// One row per forecast model when the passage was routed on several; empty otherwise.
    pub models: Vec<ModelRow>,
    /// What the models say together: whether their ways agree, the spread of their arrivals, the route
    /// that holds up best.
    pub verdict: Vec<String>,
}

fn hours_text(h: f64) -> String {
    crate::core::nav::calc::format_duration(chrono::Duration::minutes((h * 60.0).round() as i64))
}

/// The card's rows for the models, and what they say together.
fn models_and_verdict(p: &crate::router::RouterPreview) -> (Vec<ModelRow>, Vec<String>) {
    let Some(c) = p.comparison.as_ref() else { return Default::default() };
    let robust = c.cross.most_robust();
    let models = p
        .runs
        .iter()
        .enumerate()
        .map(|(i, r)| ModelRow {
            index: i,
            label: r.model.label().to_string(),
            summary: summary(&r.best),
            colour: ALT_COLOURS[i % ALT_COLOURS.len()].name().to_string(),
            focused: r.ex.is_none(),
            arrives: r.best.arrives,
            robust: robust == Some(i),
        })
        .collect();
    let mut verdict = vec![if c.agree {
        gettext("The forecasts agree on the way.")
    } else {
        gettext("The forecasts disagree on the way: their tracks part on the chart.")
    }];
    let arriving: Vec<&crate::router::ModelRun> = p.runs.iter().filter(|r| r.best.arrives).collect();
    if let (Some(a), Some(z)) = (arriving.iter().min_by_key(|r| r.best.end), arriving.iter().max_by_key(|r| r.best.end))
        && arriving.len() >= 2
    {
        verdict.push(gettext_f(
            "They arrive between {from} ({a}) and {to} ({z}).",
            &[("from", &local(a.best.end)), ("a", a.model.label()), ("to", &local(z.best.end)), ("z", z.model.label())],
        ));
    }
    let short: Vec<&str> = p.runs.iter().filter(|r| !r.best.arrives).map(|r| r.model.label()).collect();
    if !short.is_empty() {
        verdict.push(gettext_f("The forecast of {models} ends before the destination.", &[("models", &short.join(", "))]));
    }
    if robust.is_some_and(|i| c.cross.over_limit(i)) {
        verdict.push(gettext("Every route meets more wind than your limit on some forecast."));
    }
    if let Some(i) = robust {
        let worst = c.cross.worst(i);
        let loss = c.cross.max_regret(i);
        verdict.push(if loss < 0.25 {
            gettext_f("The {model} route is best whichever forecast is right (at worst {worst}).", &[("model", p.runs[i].model.label()), ("worst", &hours_text(worst))])
        } else {
            gettext_f(
                "The {model} route holds up best: at worst {worst}, never more than {loss} behind.",
                &[("model", p.runs[i].model.label()), ("worst", &hours_text(worst)), ("loss", &hours_text(loss))],
            )
        });
    }
    (models, verdict)
}

/// Shows model `i`'s search, its alternatives and its wind.
pub fn focus_model(app: &Rc<AppState>, i: usize) {
    let model = {
        let mut r = app.router.borrow_mut();
        let Some(p) = r.as_mut() else { return };
        if !p.focus(i) {
            return;
        }
        p.model
    };
    crate::flows::weather::show_source(app, model);
    let t = app.router.borrow().as_ref().map(|p| p.t);
    if let Some(t) = t {
        crate::router::follow_weather(app, t);
    }
    app.canvas.queue_draw();
    app.refresh();
}

/// The cross-check as a table: a column per forecast, a row per model's route, in sections for the
/// time, the strongest wind and the hours under engine; and how much each loses at worst.
pub fn cross_check(app: &AppState) -> Option<(Vec<String>, Vec<crate::vm::stats::Section>)> {
    let r = app.router.borrow();
    let p = r.as_ref()?;
    let c = p.comparison.as_ref()?;
    let cells = &c.cross.cells;
    let mut heads: Vec<String> = p.runs.iter().map(|r| gettext_f("If {model} is right", &[("model", r.model.label())])).collect();
    heads.push(gettext("At worst"));
    let route = |i: usize| gettext_f("{model} route", &[("model", p.runs[i].model.label())]);
    let section = |title: String, cell: &dyn Fn(&crate::core::routing::compare::Cell) -> String, worst: &dyn Fn(usize) -> String| crate::vm::stats::Section {
        title,
        rows: (0..cells.len()).map(|i| (route(i), cells[i].iter().map(cell).chain([worst(i)]).collect())).collect(),
    };
    let time = |x: &crate::core::routing::compare::Cell| {
        if x.arrives { hours_text(x.hours) } else { gettext_f("{h} + {nm} nm", &[("h", &hours_text(x.hours)), ("nm", &format!("{:.0}", x.remaining_nm))]) }
    };
    Some((
        heads,
        vec![
            section(gettext("Time to the destination"), &time, &|i| hours_text(c.cross.worst(i))),
            section(gettext("Behind the best route for that forecast"), &|x| x.cost_hours().to_string(), &|i| hours_text(c.cross.max_regret(i)))
                .with_cells(|i, f| hours_text(c.cross.regret(i, f))),
            section(gettext("Strongest wind"), &|x| gettext_f("{kn} kn", &[("kn", &format!("{:.0}", x.max_tws_kn))]), &|i| gettext_f("{kn} kn", &[("kn", &format!("{:.0}", cells[i].iter().map(|x| x.max_tws_kn).fold(0.0, f64::max)))])),
            section(gettext("Under engine"), &|x| hours_text(x.motor_hours), &|i| hours_text(cells[i].iter().map(|x| x.motor_hours).fold(0.0, f64::max))),
        ],
    ))
}

/// What the card says over a saved route's plan: when it was made for and with which forecast.
fn replay_text(p: &crate::router::RouterPreview) -> (String, String, bool) {
    let Some(r) = p.replay.as_ref() else { return Default::default() };
    let a = &p.alts[0].alt;
    let title = gettext("Weather route as planned");
    let forecast = match (r.run, r.model) {
        (Some(run), Some(m)) => gettext_f("the {model} forecast of the {run} run", &[("model", m.label()), ("run", &run.format("%a %d %b %H UTC").to_string())]),
        (Some(run), None) => gettext_f("the forecast of the {run} run", &[("run", &run.format("%a %d %b %H UTC").to_string())]),
        (None, _) => gettext("no forecast was kept, so the weather shown is the one loaded now"),
    };
    let mut note = if a.arrives {
        gettext_f("Left {from}, arrived {eta}, on {forecast}. Play or drag the time to watch the passage.", &[("from", &local(p.ex.depart)), ("eta", &local(a.end)), ("forecast", &forecast)])
    } else {
        gettext_f("Left {from}; the forecast ended {time}, and the rest of the way is direct. On {forecast}. Play or drag the time to watch the passage.", &[("from", &local(p.ex.depart)), ("time", &local(a.end)), ("forecast", &forecast)])
    };
    note.push(' ');
    note.push_str(&gettext("The saved route is the blue band beneath it, with its waypoints, to compare."));
    if r.edited {
        note.push(' ');
        note.push_str(&gettext("The route has been edited since: this is the passage as it was planned."));
    }
    // Closing the card ends the playback: the boat leaves the route, and the weather goes back to the
    // forecast loaded before, over its whole area and time.
    note.push(' ');
    note.push_str(&gettext("Close it to go back to the forecast loaded before."));
    (title, note, false)
}

pub fn card(app: &AppState) -> Option<Card> {
    let r = app.router.borrow();
    let p = r.as_ref()?;
    let touch = app.canvas.is_touch();
    let time = || p.ex.horizon.map(local).unwrap_or_default();
    let (title, mut note, failure) = match &p.ex.arrived {
        _ if p.replay.is_some() => replay_text(p),
        Some(r) => (
            gettext("Weather route"),
            if touch {
                gettext_f("Arrives {eta}. Tap the front line to add other ways to go. Play or drag the time to watch them all; touch and hold one to use it.", &[("eta", &local(r.arrival))])
            } else {
                gettext_f("Arrives {eta}. Click the front line to add other ways to go. Play or drag the time to watch them all; right-click one to use it.", &[("eta", &local(r.arrival))])
            },
            false,
        ),
        None if p.ex.stalled => (
            gettext("No way forward"),
            if touch {
                gettext_f(
                    "The boat could get no closer after {time}: it may be calm, too windy, or blocked by land. The way that got nearest is shown. Tap the front line, or scrub back to an earlier one, to add a coloured alternative ending there. Touch and hold one to save it as a route to that point, then the rest of the way direct.",
                    &[("time", &time())],
                )
            } else {
                gettext_f(
                    "The boat could get no closer after {time}: it may be calm, too windy, or blocked by land. The way that got nearest is shown. Click the front line, or scrub back to an earlier one, to add a coloured alternative ending there. Right-click one to save it as a route to that point, then the rest of the way direct.",
                    &[("time", &time())],
                )
            },
            true,
        ),
        None => (
            gettext("The forecast ends before the destination"),
            if touch {
                gettext_f(
                    "The way that gets nearest the destination by then is shown. Tap the front line, as many times as you like, to add a coloured alternative ending there. Play or drag the time to watch them all; touch and hold one to save it as a route to {time}, then the rest of the way direct.",
                    &[("time", &time())],
                )
            } else {
                gettext_f(
                    "The way that gets nearest the destination by then is shown. Click the front line, as many times as you like, to add a coloured alternative ending there. Play or drag the time to watch them all; right-click one to save it as a route to {time}, then the rest of the way direct.",
                    &[("time", &time())],
                )
            },
            true,
        ),
    };
    if let Some(c) = p.currents.as_ref().filter(|_| p.replay.is_none()) {
        note.push(' ');
        note.push_str(&gettext_f("Sailed in the current of {sources}.", &[("sources", c)]));
    }
    let alternatives = p
        .alts
        .iter()
        .enumerate()
        .map(|(i, c)| AltRow {
            index: i,
            title: alt_title(i, &c.alt),
            summary: summary(&c.alt),
            colour: ALT_COLOURS[c.colour % ALT_COLOURS.len()].name().to_string(),
            dash: ALT_DASHES[c.colour % ALT_DASHES.len()].to_vec(),
            selected: i == p.selected,
            arrives: c.alt.arrives,
        })
        .collect();
    let (models, verdict) = models_and_verdict(p);
    Some(Card { title, note, failure, alternatives, can_add: p.replay.is_none() && p.alts.len() < ALT_COLOURS.len(), replay: p.replay.is_some(), models, verdict })
}

/// Adds an alternative ending at the point of the front nearest to (`lat`, `lon`).
pub fn pick_near(app: &Rc<AppState>, lat: f64, lon: f64) -> bool {
    let (x, y) = app.canvas.view_info().to_screen(&LatLon::new(lat, lon));
    crate::router::click_front(app, x, y)
}

pub fn select(app: &AppState, i: usize) {
    if let Some(p) = app.router.borrow_mut().as_mut()
        && i < p.alts.len()
    {
        p.selected = i;
    }
    app.canvas.queue_draw();
    app.refresh();
}

/// Ends the preview without saving anything.
pub fn discard(app: &Rc<AppState>) {
    crate::router::close(app);
}

/// The statistics of the alternatives in the preview, as the table's sections and column headings.
pub fn stats(app: &AppState) -> Option<(Vec<String>, Vec<crate::vm::stats::Section>)> {
    let r = app.router.borrow();
    let p = r.as_ref()?;
    if p.alts.is_empty() {
        return None;
    }
    let alts: Vec<&crate::core::routing::alternatives::Alternative> = p.alts.iter().map(|c| &c.alt).collect();
    let heads = p.alts.iter().enumerate().map(|(i, _)| (i + 1).to_string()).collect();
    Some((heads, crate::vm::stats::sections(&alts)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::nav::store::NavStore;
    use crate::core::session::Session;
    use crate::engine::Engine;
    use crate::palette::Mode;

    #[test]
    fn without_a_preview_there_is_no_card() {
        let canvas = Engine::new(crate::config::ViewState { lat: 48.0, lon: -4.0, zoom: 8.0 }, Mode::Day);
        let app = AppState::new(canvas, Session::new(NavStore::open_memory().unwrap()).unwrap(), crate::config::Settings::default());
        assert!(card(&app).is_none());
        assert!(stats(&app).is_none());
        let _ = gettext("x");
    }
}
