//! What a weather route keeps of the plan it was made from: a summary for its page, its
//! statistics, and the playback (`router::replay`).

use crate::app::AppState;
use crate::core::routing::alternatives::Alternative;
use crate::i18n::{gettext, gettext_f};
use crate::router::{local, summary};
use serde::Serialize;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PlanInfo {
    /// "[distance] 312 nm  [arrival] Tue 28 14:00  [wind] up to 24 kn".
    pub summary: Vec<crate::chips::Chip>,
    /// "Left Mon 27 06:00 on the forecast of the Mon 27 Oct 00 UTC run".
    pub forecast: String,
    /// The route's points were changed after the plan was made.
    pub edited: bool,
    /// When it was routed on several forecasts: the spread of its arrival over them, or empty.
    pub bracket: String,
}

/// The plan of route `id`, if it keeps one.
pub fn info(app: &AppState, id: i64) -> Option<PlanInfo> {
    let s = app.session.borrow();
    let plan = s.plan(id)?;
    let route = s.routes.iter().find(|r| r.id == id)?;
    let alt = Alternative::new(plan.nodes.clone(), plan.dest, plan.arrives);
    let forecast = match (plan.run(), plan.source) {
        (Some(run), Some(m)) => gettext_f("Left {from} on the {model} forecast of the {run} run", &[("from", &local(plan.depart)), ("model", m.label()), ("run", &run.format("%a %d %b %H UTC").to_string())]),
        (Some(run), None) => gettext_f("Left {from} on the forecast of the {run} run", &[("from", &local(plan.depart)), ("run", &run.format("%a %d %b %H UTC").to_string())]),
        (None, _) => gettext_f("Left {from}; no forecast was kept", &[("from", &local(plan.depart))]),
    };
    let bracket = crate::flows::routing::bracket_text(&plan.checks).unwrap_or_default();
    Some(PlanInfo { summary: summary(&alt), forecast, edited: !plan.matches(&route.points.iter().map(|p| p.pos).collect::<Vec<_>>()), bracket })
}

/// The statistics of the passage as planned, in the shape of the router's: column headings and sections.
pub fn stats(app: &AppState, id: i64) -> Option<(Vec<String>, Vec<crate::vm::stats::Section>)> {
    let plan = app.session.borrow().plan(id)?;
    let alt = Alternative::new(plan.nodes, plan.dest, plan.arrives);
    let mut sections = crate::vm::stats::sections(&[&alt]);
    // Routed on several forecasts: the saved route sailed through each.
    if !plan.checks.is_empty() {
        let hours = |h: f64| crate::core::nav::calc::format_duration(chrono::Duration::minutes((h * 60.0).round() as i64));
        let rows = plan
            .checks
            .iter()
            .map(|c| {
                let mut text = if c.arrives {
                    gettext_f("arrives {eta} after {h}", &[("eta", &local(c.end)), ("h", &hours(c.hours))])
                } else {
                    gettext_f("{nm} nm to go when its forecast ends {end}", &[("nm", &format!("{:.0}", c.remaining_nm)), ("end", &local(c.end))])
                };
                text.push_str(&gettext_f(", wind up to {kn} kn", &[("kn", &format!("{:.0}", c.max_tws_kn))]));
                if c.motor_hours > 0.1 {
                    text.push_str(&gettext_f(", {h} under engine", &[("h", &hours(c.motor_hours))]));
                }
                (c.source.label().to_string(), vec![text])
            })
            .collect();
        sections.insert(0, crate::vm::stats::Section { title: gettext("The saved route on each forecast"), rows });
    }
    Some((vec![gettext("Planned")], sections))
}
