//! The results of "Best departure": a bar for each departure tried, the ten best, and what each
//! costs, ready for a shell to draw.

use crate::app::AppState;
use crate::core::routing::stats::sun_elevation_deg;
use crate::flows::departures::{HIGHLIGHTED, LISTED, ending, figures, local, tooltip_of, why};
use crate::i18n::{gettext, gettext_f};
use chrono::{Duration, Local, Utc};
use serde::Serialize;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Bar {
    /// 0..1 across the timeline, and how wide it is.
    pub x: f64,
    pub w: f64,
    /// 0..1 from the baseline; the cost against the dearest passage. Departures that fail are short.
    pub h: f64,
    pub ok: bool,
    /// One of the best (the accent colour).
    pub best: bool,
    pub selected: bool,
    pub tooltip: String,
    /// With several forecast models: each model's cost, as a height and its colour's token name, and the
    /// least and most of them, so a bar reads as a range. A tall range is a gamble; a short low one is
    /// good whichever forecast is right.
    pub dots: Vec<Dot>,
    pub range: Option<[f64; 2]>,
}

/// One forecast model's cost of a departure: its height on the bar's scale and its colour's token name.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Dot {
    pub h: f64,
    pub colour: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Row {
    /// Index into the candidates.
    pub index: usize,
    pub title: String,
    /// How long, how windy, how rough.
    pub figures: Vec<crate::chips::Chip>,
    /// What weighs most on it.
    pub why: Vec<crate::chips::Chip>,
    pub selected: bool,
    /// With several models: what it costs on each.
    pub models: Vec<crate::chips::Chip>,
}

/// A forecast model of the scan, in its colour, for a legend.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Legend {
    pub label: String,
    pub colour: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Day {
    /// Where local midnight is, and where the day's name goes (noon).
    pub x: f64,
    pub label_x: f64,
    pub label: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Departures {
    /// The route these are the departures of.
    pub route_id: i64,
    pub summary: String,
    pub bars: Vec<Bar>,
    pub rows: Vec<Row>,
    pub days: Vec<Day>,
    /// Stretches of the timeline that are dark at the start of the passage, as (from, to) in 0..1.
    pub nights: Vec<(f64, f64)>,
    pub can_compare: bool,
    /// The models scanned, when there were several; empty otherwise.
    pub legend: Vec<Legend>,
    pub cautious: bool,
}

pub fn departures(app: &AppState) -> Option<Departures> {
    let set = app.departures.borrow();
    let d = set.as_ref()?;
    let res = &d.results;
    let n = res.scan.candidates.len();
    let tried = n.to_string();
    let estimated = res.ranked.iter().filter(|&&i| res.scan.candidates[i].passage().is_some_and(|p| !p.arrives())).count();
    let summary = if res.ranked.is_empty() {
        gettext_f("None of the {n} departures tried gets far enough before the forecast ends. Hover over the timeline to see why.", &[("n", &tried)])
    } else if estimated == 0 {
        gettext_f(
            "{k} of the {n} departures tried arrive. The bars show what each costs, lower is better; the shaded bands are night.",
            &[("k", &res.ranked.len().to_string()), ("n", &tried)],
        )
    } else {
        gettext_f(
            "{k} of the {n} departures tried arrive before the forecast ends; {e} more are followed to the point nearest the destination when it ends, and costed as if the rest of the way went as well. The bars show what each costs, lower is better; the shaded bands are night.",
            &[("k", &(res.ranked.len() - estimated).to_string()), ("n", &tried), ("e", &estimated.to_string())],
        )
    };
    let several = res.models.len() > 1;
    let summary = if several {
        let how = if res.cautious { gettext("Each departure counts as its worst forecast.") } else { gettext("Each departure counts as the average of the forecasts; the dots are each forecast's cost.") };
        format!("{summary} {how}")
    } else {
        summary
    };
    let max = res.scans().filter_map(|s| s.cost_range()).map(|(_, hi)| hi).fold(1e-6, f64::max);
    let colour = |k: usize| crate::layers::router::ALT_COLOURS[k % crate::layers::router::ALT_COLOURS.len()].name().to_string();
    let best: Vec<usize> = res.ranked.iter().take(HIGHLIGHTED).copied().collect();
    let bars = (0..n)
        .map(|i| Bar {
            x: res.bar_of(i).0,
            w: res.bar_of(i).1,
            h: res.costs[i].map_or(0.02, |cost| (cost / max).clamp(0.04, 1.0)),
            ok: res.costs[i].is_some(),
            best: res.costs[i].is_some() && best.contains(&i),
            selected: res.selected.get() == Some(i),
            tooltip: tooltip_of(res, i),
            dots: if several { res.model_costs(i).iter().enumerate().filter_map(|(k, (_, c))| c.map(|c| Dot { h: (c / max).clamp(0.04, 1.0), colour: colour(k) })).collect() } else { Vec::new() },
            range: several.then(|| res.model_costs(i).iter().filter_map(|(_, c)| *c).fold(None, |r: Option<(f64, f64)>, c| Some(r.map_or((c, c), |(a, b)| (a.min(c), b.max(c)))))).flatten().map(|(a, b)| [(a / max).clamp(0.04, 1.0), (b / max).clamp(0.04, 1.0)]),
        })
        .collect();
    let rows = res
        .ranked
        .iter()
        .take(LISTED)
        .enumerate()
        .filter_map(|(rank, &i)| {
            let c = &res.scan.candidates[i];
            let p = c.passage()?;
            Some(Row {
                index: i,
                title: gettext_f("{rank}. Leave {dep}, {ending}", &[("rank", &(rank + 1).to_string()), ("dep", &local(c.depart)), ("ending", &ending(p))]),
                figures: figures(p),
                why: why(&p.measures, &res.scan.weights),
                selected: res.selected.get() == Some(i),
                models: if several {
                    res.model_costs(i).iter().map(|(m, c)| crate::chips::Chip::plain(match c { Some(c) => gettext_f("{model} {cost}", &[("model", m.label()), ("cost", &format!("{c:.0}"))]), None => gettext_f("{model} no passage", &[("model", m.label())]) })).collect()
                } else {
                    Vec::new()
                },
            })
        })
        .collect();
    let (a, b) = res.span();
    let total = (b - a).num_seconds().max(1) as f64;
    let mut nights = Vec::new();
    let mut from: Option<f64> = None;
    for k in 0..=200 {
        let f = k as f64 / 200.0;
        let t = a + Duration::seconds((total * f) as i64);
        let dark = sun_elevation_deg(&res.start, t) < -6.0;
        match (dark, from) {
            (true, None) => from = Some(f),
            (false, Some(s)) => {
                nights.push((s, f));
                from = None;
            }
            _ => {}
        }
    }
    if let Some(s) = from {
        nights.push((s, 1.0));
    }
    let mut days = Vec::new();
    let mut day = a.with_timezone(&Local).date_naive();
    while let Some(midnight) = day.and_hms_opt(0, 0, 0).and_then(|m| m.and_local_timezone(Local).earliest()) {
        let t = midnight.with_timezone(&Utc);
        if t > b {
            break;
        }
        if t >= a {
            days.push(Day { x: res.x_of(t, 1.0), label_x: res.x_of(t + Duration::hours(12), 1.0), label: midnight.format("%a %d").to_string() });
        }
        day = day.succ_opt().unwrap_or(day);
    }
    let _ = gettext("x");
    let legend = if several { res.models.iter().enumerate().map(|(k, m)| Legend { label: m.label().to_string(), colour: colour(k) }).collect() } else { Vec::new() };
    Some(Departures { route_id: d.source.id, summary, bars, rows, days, nights, can_compare: !res.ranked.is_empty(), legend, cautious: res.cautious })
}

/// Selects departure `i` (an index into the candidates).
pub fn select(app: &AppState, i: usize) {
    if let Some(d) = app.departures.borrow().as_ref()
        && i < d.results.scan.candidates.len()
    {
        d.results.selected.set(Some(i));
    }
    app.refresh();
}

/// The table comparing the best three and the selected departure.
pub fn compare(app: &AppState) -> Option<(Vec<String>, Vec<crate::vm::stats::Section>)> {
    let set = app.departures.borrow();
    let d = set.as_ref()?;
    let res = &d.results;
    let mut picks: Vec<usize> = res.ranked.iter().take(HIGHLIGHTED).copied().collect();
    if let Some(s) = res.selected.get().filter(|s| !picks.contains(s) && res.scan.candidates[*s].passage().is_some()) {
        picks.push(s);
    }
    let heads: Vec<String> = picks.iter().map(|&i| gettext_f("Leave {dep}", &[("dep", &local(res.scan.candidates[i].depart))])).collect();
    let alts: Vec<&crate::core::routing::alternatives::Alternative> = picks.iter().filter_map(|&i| res.scan.candidates[i].passage().map(|p| &p.alt)).collect();
    if alts.is_empty() {
        return None;
    }
    Some((heads, crate::vm::stats::sections(&alts)))
}

/// Closes the results.
pub fn close(app: &AppState) {
    *app.departures.borrow_mut() = None;
    app.refresh();
}
