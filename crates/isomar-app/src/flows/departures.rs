//! "Best departure": routes a passage for every departure in a window, a few hours apart, and
//! ranks them by what the crew cares about. The scan is a job; the results are kept in the app
//! state for the shell to show (`vm::departures`) and to preview one of them.

use crate::app::AppState;
use crate::chips::{Chip, Icon, plain};
use crate::config::WindowPrefs;
use crate::core::geo::LatLon;
use crate::core::nav::calc::format_duration;
use crate::core::nav::model::Route;
use crate::core::routing::isochrone::{Bounded, RouteError};
use crate::core::routing::window::{self, Failure, Scan, ScanProgress, Term, Weights};
use crate::core::weather::model::Group;
use crate::core::weather::source::Source;
use crate::flows::routing::{self, BoatParams, Search};
use crate::i18n::{gettext, gettext_f};
use crate::jobs::JobHandle;
use crate::platform::bg;
use chrono::{DateTime, Duration, Local, Timelike, Utc};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

/// How many of the best departures the list shows.
pub const LISTED: usize = 10;
/// How many of the best are drawn in the accent colour on the timeline.
pub const HIGHLIGHTED: usize = 3;
/// The speed a passage is assumed to need at least, to stop trying
/// departures too close to the end of the forecast.
pub const FAST_KN: f64 = 8.0;
/// A departure is tried only this long before the forecast ends, at least
/// (or the time the passage needs at `FAST_KN`, if less): a passage the
/// forecast ends before is judged on the way it made until then.
pub const MIN_FORECAST_H: f64 = 24.0;
pub const STEPS_H: [f64; 3] = [1.0, 3.0, 6.0];


/// The presets of the "What matters" group, in the combo's order; the last
/// entry of the combo is "Custom".
pub fn presets() -> [(String, Weights); 3] {
    [(gettext("Comfortable"), Weights::comfortable()), (gettext("Fastest"), Weights::fastest()), (gettext("Sail, don't motor"), Weights::sailing())]
}


pub fn next_slot(step_h: f64) -> DateTime<Local> {
    let now = Local::now();
    let step = (step_h * 60.0).max(1.0) as i64;
    let minutes = now.hour() as i64 * 60 + now.minute() as i64;
    let next = (minutes / step + 1) * step;
    let midnight = now - Duration::minutes(minutes) - Duration::seconds(now.second() as i64);
    midnight + Duration::minutes(next)
}


pub fn local(t: DateTime<Utc>) -> String {
    t.with_timezone(&Local).format("%a %d %H:%M").to_string()
}

pub fn hours(h: f64) -> String {
    format_duration(Duration::seconds((h * 3600.0).round() as i64))
}

/// Why a departure has no passage.
pub fn failure_text(f: &Failure) -> String {
    match f {
        Failure::ForecastEnds => gettext("The forecast ends before the boat has made enough of the way to judge it"),
        Failure::Stalled => gettext("No way forward (calm, too much wind, or land in the way)"),
        Failure::Route(e) => routing::explain(e),
    }
}

/// One part of a passage's cost: "[upwind] 6 h", "6 h close-hauled" in words.
pub fn term_chip(t: Term, amount: f64, w: &Weights) -> Chip {
    let h = hours(amount);
    let hv = [("h", h.as_str())];
    match t {
        Term::Time => Chip::new(Icon::Time, h.clone(), gettext_f("{h} at sea", &hv)),
        Term::Upwind => Chip::new(Icon::Upwind, h.clone(), gettext_f("{h} close-hauled", &hv)),
        Term::Engine => Chip::new(Icon::Engine, h.clone(), gettext_f("{h} under engine", &hv)),
        Term::StrongWind => {
            let a = [("h", h.as_str()), ("kn", &format!("{:.0}", w.strong_wind_kn))];
            Chip::new(Icon::Wind, gettext_f("{h} ≥ {kn} kn", &a), gettext_f("{h} in {kn} kn of wind or more", &a))
        }
        Term::Gusts => {
            let a = [("h", h.as_str()), ("kn", &format!("{:.0}", w.gust_kn))];
            Chip::new(Icon::Gusts, gettext_f("{h} ≥ {kn} kn", &a), gettext_f("{h} in gusts of {kn} kn or more", &a))
        }
        Term::Waves => {
            let a = [("h", h.as_str()), ("m", &format!("{:.1}", w.wave_m))];
            Chip::new(Icon::Waves, gettext_f("{h} ≥ {m} m", &a), gettext_f("{h} in waves of {m} m or more", &a))
        }
        Term::Night => Chip::new(Icon::Night, h.clone(), gettext_f("{h} in the dark", &hv)),
        Term::Manoeuvres => {
            let n = format!("{amount:.0}");
            Chip::new(Icon::Tack, n.clone(), gettext_f("{n} tacks and gybes", &[("n", &n)]))
        }
        Term::Waiting => Chip::new(Icon::Waiting, h.clone(), gettext_f("{h} of waiting", &hv)),
        Term::Holding => Chip::new(Icon::Waiting, gettext_f("{h} for the stream", &hv), gettext_f("{h} waiting under way for the stream to turn", &hv)),
        Term::WindAgainstTide => Chip::new(Icon::Current, h.clone(), gettext_f("{h} of wind against tide", &hv)),
    }
}

/// What weighs most on a passage, besides its length: the two largest costs.
pub fn why(m: &window::Measures, w: &Weights) -> Vec<Chip> {
    let mut terms: Vec<_> = window::terms(m, w).into_iter().filter(|t| t.0 != Term::Time && t.2 > 0.05).collect();
    terms.sort_by(|a, b| b.2.total_cmp(&a.2));
    if terms.is_empty() {
        return vec![Chip::plain(gettext("Nothing to avoid"))];
    }
    let mut chips: Vec<Chip> = terms.iter().take(2).map(|t| term_chip(t.0, t.1, w)).collect();
    // Wind against tide is always said: the sea it raises is worse than its hours tell.
    if m.wind_against_tide_hours > 0.05 && !terms.iter().take(2).any(|t| t.0 == Term::WindAgainstTide) {
        chips.push(term_chip(Term::WindAgainstTide, m.wind_against_tide_hours, w));
    }
    chips
}

/// The figures of a passage, for its row.
pub fn figures(p: &window::Passage) -> Vec<Chip> {
    let at_sea = hours(p.measures.hours);
    let kn = format!("{:.0}", p.stats.max_tws_kn);
    let mut out = vec![
        Chip::new(Icon::Time, at_sea.clone(), gettext_f("{h} at sea", &[("h", &at_sea)])),
        Chip::new(Icon::Wind, gettext_f("up to {kn} kn", &[("kn", &kn)]), gettext_f("wind up to {kn} kn", &[("kn", &kn)])),
    ];
    if let Some(g) = p.measures.max_gust_kn {
        let kn = format!("{g:.0}");
        out.push(Chip::new(Icon::Gusts, gettext_f("{kn} kn", &[("kn", &kn)]), gettext_f("gusts {kn} kn", &[("kn", &kn)])));
    }
    if let Some(m) = p.measures.max_wave_m {
        let m = format!("{m:.1}");
        out.push(Chip::new(Icon::Waves, gettext_f("{m} m", &[("m", &m)]), gettext_f("waves {m} m", &[("m", &m)])));
    }
    if p.measures.engine_hours > 0.05 {
        let h = hours(p.measures.engine_hours);
        out.push(Chip::new(Icon::Engine, h.clone(), gettext_f("{h} under engine", &[("h", &h)])));
    }
    out
}


/// The results window's state.
pub struct Results {
    /// The first model's scan; with several models, `others` are theirs, of the same departures.
    pub scan: Scan,
    pub ranked: Vec<usize>,
    /// Index into `scan.candidates`.
    pub selected: Cell<Option<usize>>,
    pub start: LatLon,
    /// The models scanned, `scan`'s first.
    pub models: Vec<Source>,
    pub others: Vec<Scan>,
    /// Ranked by the worst model, not the average.
    pub cautious: bool,
    /// Each departure's cost over the models (`window::combined_costs`).
    pub costs: Vec<Option<f64>>,
}

impl Results {
    pub fn new(models: Vec<Source>, scans: Vec<Scan>, cautious: bool, start: LatLon) -> Results {
        let costs = window::combined_costs(&scans.iter().collect::<Vec<_>>(), cautious);
        let ranked = window::ranked_by(&costs);
        let mut scans = scans.into_iter();
        let scan = scans.next().expect("a scan at least");
        Results { scan, ranked: ranked.clone(), selected: Cell::new(ranked.first().copied()), start, models, others: scans.collect(), cautious, costs }
    }

    /// Every model's scan, the first model's first.
    pub fn scans(&self) -> impl Iterator<Item = &Scan> {
        std::iter::once(&self.scan).chain(&self.others)
    }

    /// Departure `i`'s cost under each model, where it has a passage.
    pub fn model_costs(&self, i: usize) -> Vec<(Source, Option<f64>)> {
        self.models.iter().copied().zip(self.scans().map(|s| s.candidates.get(i).and_then(|c| c.passage()).map(|p| p.cost))).collect()
    }

    /// The span the timeline shows: the first departure to a step after the last.
    /// From the first departure to a step past the last, the step being the scan's own (the widest gap:
    /// the refined departures fall between the scan's).
    pub fn span(&self) -> (DateTime<Utc>, DateTime<Utc>) {
        let c = &self.scan.candidates;
        let first = c[0].depart;
        let step = c.windows(2).map(|w| w[1].depart - w[0].depart).max().unwrap_or(Duration::hours(3));
        (first, c[c.len() - 1].depart + step)
    }

    /// Departure `i`'s bar, as a share of the width: from it to the next one.
    pub fn bar_of(&self, i: usize) -> (f64, f64) {
        let c = &self.scan.candidates;
        let x0 = self.x_of(c[i].depart, 1.0);
        let x1 = c.get(i + 1).map_or(1.0, |n| self.x_of(n.depart, 1.0));
        (x0, (x1 - x0).max(0.0))
    }

    pub fn x_of(&self, t: DateTime<Utc>, width: f64) -> f64 {
        let (a, b) = self.span();
        (t - a).num_seconds() as f64 / (b - a).num_seconds().max(1) as f64 * width
    }

    pub fn index_at(&self, x: f64, width: f64) -> Option<usize> {
        let f = x / width;
        (0..self.scan.candidates.len()).find(|&i| {
            let (x0, w) = self.bar_of(i);
            f >= x0 && f < x0 + w
        })
    }
}

/// "arrive Tue 04 18:00", or where the forecast ends: "312 nm to go on Tue 04 18:00".
pub fn ending(p: &window::Passage) -> String {
    match p.estimated {
        None => gettext_f("arrive {arr}", &[("arr", &local(p.alt.end))]),
        Some(e) => gettext_f("{nm} nm to go on {end}", &[("nm", &format!("{:.0}", e.remaining_nm)), ("end", &local(p.alt.end))]),
    }
}

pub fn tooltip_of(res: &Results, i: usize) -> String {
    let c = &res.scan.candidates[i];
    let text = match &c.result {
        Ok(p) if p.estimated.is_some() => {
            let e = p.estimated.expect("checked");
            let rank = res.ranked.iter().position(|&r| r == i).map_or(0, |r| r + 1);
            gettext_f(
                "Leave {dep}, {nm} nm to go when the forecast ends on {end}\n{figures}\n{why}\nCounts as about {cost}, from the {pct}% of the way the forecast covers (number {rank} of {n})",
                &[
                    ("dep", &local(c.depart)),
                    ("nm", &format!("{:.0}", e.remaining_nm)),
                    ("end", &local(p.alt.end)),
                    ("figures", &plain(&figures(p))),
                    ("why", &plain(&why(&p.measures, &res.scan.weights))),
                    ("cost", &hours(p.cost)),
                    ("pct", &format!("{:.0}", e.share_made * 100.0)),
                    ("rank", &rank.to_string()),
                    ("n", &res.ranked.len().to_string()),
                ],
            )
        }
        Ok(p) => {
            let rank = res.ranked.iter().position(|&r| r == i).map_or(0, |r| r + 1);
            gettext_f(
                "Leave {dep}, arrive {arr}\n{figures}\n{why}\nCounts as {cost} (number {rank} of {n})",
                &[
                    ("dep", &local(c.depart)),
                    ("arr", &local(p.alt.end)),
                    ("figures", &plain(&figures(p))),
                    ("why", &plain(&why(&p.measures, &res.scan.weights))),
                    ("cost", &hours(p.cost)),
                    ("rank", &rank.to_string()),
                    ("n", &res.ranked.len().to_string()),
                ],
            )
        }
        Err(f) => gettext_f("Leave {dep}: {why}", &[("dep", &local(c.depart)), ("why", &failure_text(f))]),
    };
    if res.models.len() < 2 {
        return text;
    }
    // Then what it costs on each forecast, and how it counts over them.
    let mut lines = vec![text];
    for (m, cost) in res.model_costs(i) {
        lines.push(match cost {
            Some(c) => gettext_f("{model}: counts as {cost}", &[("model", m.label()), ("cost", &hours(c))]),
            None => gettext_f("{model}: no passage", &[("model", m.label())]),
        });
    }
    if let Some(c) = res.costs.get(i).copied().flatten() {
        lines.push(if res.cautious { gettext_f("At worst {cost}", &[("cost", &hours(c))]) } else { gettext_f("On average {cost}", &[("cost", &hours(c))]) });
    }
    lines.join("\n")
}

/// Runs the scan on a worker, reading its progress from `progress`; `None` if it was cancelled or
/// failed (the shell is told).
pub async fn scan_with(app: &Rc<AppState>, search: &Search, deps: Vec<DateTime<Utc>>, w: Weights, progress: Arc<ScanProgress>) -> Option<Scan> {
    let earliest = *deps.first()?;
    scan_on(app, search, search.source, earliest, deps, w, progress).await
}

/// `scan_with` on `model`'s forecast, the waiting counted from `earliest`.
#[allow(clippy::too_many_arguments)]
pub async fn scan_on(app: &Rc<AppState>, search: &Search, model: Source, earliest: DateTime<Utc>, deps: Vec<DateTime<Utc>>, w: Weights, progress: Arc<ScanProgress>) -> Option<Scan> {
    let (start, dest, b) = (search.start, search.dest, search.area);
    let store = routing::ModelWeather::of(&app.weather.borrow(), model, &b).unwrap_or_default();
    let (polar, land, settings, p2) = (search.polar.clone(), search.land.clone(), search.settings.clone(), progress);
    let started = std::time::Instant::now();
    let n = deps.len();
    // Dropped half-way (a cancel), the scan stops too.
    let _stop = {
        let p = p2.clone();
        crate::jobs::on_drop(move || p.cancel())
    };
    let result = bg(move || {
        let env = store.env();
        let env = Bounded { env: env.as_ref(), bounds: b };
        window::scan_since(earliest, start, dest, &deps, &polar, &env, land.as_ref(), &settings, &w, Some(&p2))
    })
    .await;
    isomar_core::log!("weather window: {n} departures in {:.1} s", started.elapsed().as_secs_f64());
    match result {
        Ok(scan) => Some(scan),
        Err(RouteError::Cancelled) => {
            app.toast(&gettext("Search cancelled"));
            None
        }
        Err(e) => {
            routing::failed(app, &e);
            None
        }
    }
}

/// Everything the best-departure results need, kept while the user looks at them.
pub struct DepartureSet {
    pub source: Route,
    pub search: Rc<Search>,
    pub results: Rc<Results>,
    pub missing_tiles: usize,
}

/// What the user chose for the scan.
#[derive(Clone, Debug)]
pub struct WindowRequest {
    pub route_id: i64,
    pub earliest: DateTime<Utc>,
    pub prefs: WindowPrefs,
    pub boat: BoatParams,
    /// The forecast models to scan on: each departure is costed on each, and ranked over them.
    pub sources: Vec<Source>,
}

/// The whole flow after the user has chosen: the area, the forecast (with gusts and waves when they
/// are weighted), the land, the scan; all of it one job, whose Cancel stops it at whatever stage it
/// is. The results are left in `app.departures`.
pub async fn run(app: &Rc<AppState>, req: WindowRequest) {
    let job = Rc::new(app.jobs.start_cancellable("route", &gettext("Finding the best departure")));
    if job.unless_cancelled(find(app, req, &job)).await.is_none() {
        app.toast(&gettext("Search cancelled"));
    }
}

async fn find(app: &Rc<AppState>, req: WindowRequest, job: &Rc<JobHandle>) {
    let _busy = app.routing_guard();
    let Some(r) = app.session.borrow().routes.iter().find(|r| r.id == req.route_id).cloned() else { return };
    if r.points.len() < 2 {
        return;
    }
    if req.prefs != app.settings.borrow().weather_window {
        app.settings.borrow_mut().weather_window = req.prefs.clone();
        app.apply_settings();
    }
    let (start, dest) = (r.points[0].pos, r.points[r.points.len() - 1].pos);
    let w = req.prefs.weights.clone();
    job.detail(&gettext("Finding the way by water"));
    let (area, sea_nm, path) = routing::passage_area(app, start, dest).await;
    let points: Vec<LatLon> = r.points.iter().map(|p| p.pos).collect();
    let extra: Vec<Group> = [(w.gusts > 0.0, Group::Gust), (w.waves > 0.0, Group::Waves)].into_iter().filter(|e| e.0).map(|e| e.1).collect();
    let until = req.earliest + Duration::minutes(((req.prefs.days * 24.0 + routing::passage_hours(sea_nm)) * 60.0) as i64);
    let models = routing::forecasts_for(app, job, &routing::models_of(app, &req.sources), &points, area, &extra, until, req.earliest, &gettext("No departure to try")).await;
    if models.is_empty() {
        return;
    }
    // The departures every model's forecast covers.
    let range = {
        let w = app.weather.borrow();
        models.iter().filter_map(|m| w.store_of(*m).and_then(|st| st.wind_range_in(Some(&area)))).reduce(|a, b| (a.0.max(b.0), a.1.min(b.1)))
    };
    let latest = range.map(|(_, hi)| hi - Duration::minutes(((sea_nm / FAST_KN).min(MIN_FORECAST_H) * 60.0) as i64));
    let deps = window::departures(req.earliest, req.prefs.days * 24.0, req.prefs.step_hours, latest);
    if deps.is_empty() {
        app.show_error(&gettext("No departure to try"), &gettext("The forecast ends less than a day after the earliest departure, too soon to judge the passage. Choose an earlier one."));
        return;
    }
    crate::flows::currents::ensure(app, job, area, start, dest, until).await;
    let polar = Arc::new(crate::polars::active(app));
    let mut settings = routing::settings_for_distance(sea_nm);
    let peak_kn = crate::flows::currents::tide_settings(app, &mut settings, sea_nm, &area, &path, req.earliest, 13.0);
    req.boat.apply(&mut settings);
    job.detail(&gettext("Preparing the land"));
    let (land, missing_tiles) = routing::prepare_land(app, start, dest, area, sea_nm, &path).await;
    let search = Rc::new(Search { source: models[0], start, dest, polar, settings, area, land: Arc::new(land) });

    // One scan a model, of the same departures, one after the other (each uses every core).
    let progress: Vec<Arc<ScanProgress>> = models.iter().map(|_| Arc::new(ScanProgress::new(deps.len()))).collect();
    {
        let progress = progress.clone();
        job.every(routing::PROGRESS_EVERY, move |job| {
            let n = progress.len();
            let fraction = progress.iter().map(|p| p.fraction()).sum::<f64>() / n as f64;
            let done: usize = progress.iter().map(|p| p.done()).sum();
            let total: usize = progress.iter().map(|p| p.total()).sum();
            let text = if n > 1 {
                gettext_f("{done} of {total} departures, on {n} forecasts", &[("done", &done.to_string()), ("total", &total.to_string()), ("n", &n.to_string())])
            } else {
                gettext_f("{done} of {total} departures", &[("done", &done.to_string()), ("total", &total.to_string())])
            };
            job.progress(fraction, &text);
        });
    }
    let mut scans = Vec::new();
    for (m, p) in models.iter().zip(&progress) {
        if models.len() > 1 {
            job.detail(&gettext_f("Trying the departures on {model}", &[("model", m.label())]));
        }
        let Some(scan) = scan_on(app, &search, *m, req.earliest, deps.clone(), w.clone(), p.clone()).await else { return };
        scans.push(scan);
    }
    // Where a tidal stream decides the passage, the best departures again, closer together: the same
    // ones on every model, so the scans stay side by side.
    if peak_kn >= window::REFINE_PEAK_KN {
        for (top, hours) in window::REFINE {
            let refs: Vec<&Scan> = scans.iter().collect();
            let costs = window::combined_costs(&refs, req.prefs.cautious);
            let tried: Vec<DateTime<Utc>> = scans[0].candidates.iter().map(|c| c.depart).collect();
            let more = window::around_best(&tried, &costs, top, hours, req.earliest, latest);
            if more.is_empty() {
                continue;
            }
            job.detail(&gettext_f("Trying {n} departures closer to the best, for the tide", &[("n", &more.len().to_string())]));
            for k in 0..scans.len() {
                let p = Arc::new(ScanProgress::new(more.len()));
                let Some(extra) = scan_on(app, &search, models[k], req.earliest, more.clone(), w.clone(), p).await else { return };
                let done = std::mem::replace(&mut scans[k], Scan { candidates: Vec::new(), weights: w.clone(), kept: Vec::new() });
                scans[k] = done.merge(extra);
            }
        }
    }
    let results = Rc::new(Results::new(models, scans, req.prefs.cautious, start));
    let best = results.ranked.first().map(|&i| results.scan.candidates[i].depart);
    let body = match best {
        Some(t) => gettext_f("{name}: leave {when}", &[("name", &r.name), ("when", &local(t))]),
        None => gettext("No departure reaches the destination"),
    };
    *app.departures.borrow_mut() = Some(DepartureSet { source: r, search, results, missing_tiles });
    app.refresh();
    app.finished(&gettext("Best departure found"), &body);
}

/// Routes the selected departure on the chart, to play it back and use it.
pub async fn preview(app: &Rc<AppState>) {
    let _busy = app.routing_guard();
    let chosen = app.departures.borrow().as_ref().and_then(|d| {
        let i = d.results.selected.get()?;
        Some((d.source.clone(), d.search.clone(), d.results.scan.candidates[i].depart, d.missing_tiles, d.results.scan.search_of(i).filter(|_| d.results.models.len() == 1), d.results.models.clone()))
    });
    let Some((source, search, depart, missing, kept, models)) = chosen else { return };
    // The scan kept the cheapest departures' searches: those need no search again.
    if let Some(ex) = kept {
        routing::show_search(app, source, &search, Arc::unwrap_or_clone(ex), missing);
        return;
    }
    let progress = Arc::new(crate::core::routing::isochrone::Progress::default());
    let job = Rc::new(app.jobs.start_cancellable("route", &gettext("Computing the route")));
    routing::follow_search(&job, progress.clone());
    // Scanned on several models, the departure is previewed on each of them, compared.
    let compute = async {
        if models.len() > 1 { routing::compute_models(app, source, &search, &models, depart, missing, progress).await } else { routing::compute_with(app, source, &search, depart, missing, progress).await }
    };
    if job.unless_cancelled(compute).await.is_none() {
        app.toast(&gettext("Route cancelled"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::routing::window::Measures;

    #[test]
    fn why_names_the_two_heaviest_costs_besides_time() {
        let w = Weights::comfortable();
        let m = Measures { hours: 20.0, upwind_hours: 6.0, engine_hours: 1.0, night_hours: 2.0, ..Measures::default() };
        // 6 h x 1.5 close-hauled, then 2 h x 0.5 dark and 1 h x 0.5 engine.
        let s = why(&m, &w);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0], Chip::new(Icon::Upwind, "6 h 00 min", "6 h 00 min close-hauled"));
        assert_eq!(why(&Measures { hours: 5.0, ..Measures::default() }, &w), vec![Chip::plain("Nothing to avoid")]);
    }

    #[test]
    fn the_timeline_maps_departures_to_slots() {
        let t0 = Utc::now();
        let candidates = (0..4).map(|i| window::Candidate { depart: t0 + Duration::hours(3 * i), result: Err(Failure::Stalled) }).collect();
        let res = Results::new(vec![Source::Gfs], vec![Scan { candidates, weights: Weights::default(), kept: Vec::new() }], false, LatLon::new(0.0, 0.0));
        assert!(res.ranked.is_empty() && res.selected.get().is_none());
        assert_eq!(res.span().1, t0 + Duration::hours(12));
        assert_eq!(res.index_at(0.0, 400.0), Some(0));
        assert_eq!(res.index_at(399.0, 400.0), Some(3));
        assert_eq!(res.index_at(400.0, 400.0), None);
        assert!((res.x_of(t0 + Duration::hours(6), 400.0) - 200.0).abs() < 1e-6);
    }
}
