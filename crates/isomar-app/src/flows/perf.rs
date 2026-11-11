//! The performance log in the app: the forecast's wind and waves at each new window, the switch kept in
//! the settings, the analysis off the main thread, the hindcast download, and applying what was learnt
//! (always at the user's request: the learnt polar into the editor's file, the factor into routing).

use crate::app::AppState;
use crate::core::datadir;
use crate::core::geo::LatLon;
use crate::core::nav::store::NavStore;
use crate::core::perf::analyse::{Report, analyse};
use crate::core::perf::passage::Passage;
use crate::core::perf::{ModelWind, SailMode};
use crate::core::routing::polar::{EDITOR_TWS, Polar};
use crate::core::weather::download::{Progress, fetch_plan};
use crate::core::weather::grib::read_fields;
use crate::core::weather::model::Param;
use crate::core::weather::model::units::MS_TO_KN;
use crate::core::weather::planner::{Area, analysis_plan, cycle_of};
use crate::core::weather::store::WeatherStore;
use crate::core::weather::cache::Cache;
use crate::i18n::{gettext, gettext_f};
use chrono::{DateTime, Duration, Utc};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// How far back NOMADS keeps runs, with a margin.
pub const HINDCAST_DAYS: i64 = 9;
/// A run's analysis hours are asked for once this long has passed since it (publication and margin).
const HINDCAST_AFTER: Duration = Duration::hours(7);

#[derive(Default)]
pub struct PerfState {
    pub report: Option<Rc<Report>>,
    pub passages: Vec<Passage>,
    pub analysing: bool,
    pub backfilling: bool,
    /// How many samples the report was made from, and when.
    pub analysed_count: usize,
    pub analysed_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

/// The loaded forecast's wind and waves at a place and time.
pub fn model_at(store: &WeatherStore, pos: LatLon, t: DateTime<Utc>) -> Option<ModelWind> {
    let w = store.wind(pos.lat, pos.lon, t)?;
    let kn = |p: Param| store.sample(p, pos.lat, pos.lon, t).map(|v| v as f64);
    let run = store.run_at(Param::WindU, pos.lat, pos.lon, t);
    Some(ModelWind {
        tws: (w.speed_ms() * MS_TO_KN) as f64,
        twd: w.blows_from() as f64,
        gust: kn(Param::Gust).map(|g| g * MS_TO_KN as f64),
        hs: kn(Param::WaveHeight),
        wave_dir: kn(Param::WaveDir),
        wave_period: kn(Param::WavePeriod),
        run,
        lead_h: run.map(|r| (t - r).num_seconds() as f64 / 3600.0),
        current: None,
    })
}

/// The 4 Hz poll's part: the forecast at each window written since, and the switch into the settings
/// when an engine moved it.
pub fn after_poll(app: &AppState) {
    let written = std::mem::take(&mut app.session.borrow_mut().perf_written);
    if !written.is_empty() {
        // Each loaded model's wind where the instruments measured one, to see which is tracking it.
        let mut compared = Vec::new();
        {
            let w = app.weather.borrow();
            let s = app.session.borrow();
            for wr in &written {
                if let Some(mut m) = model_at(&w.store, wr.pos, wr.t) {
                    m.current = crate::currents::race_current(&w.currents.field(), wr.pos, wr.t);
                    let _ = s.store.set_perf_forecast(wr.id, &m);
                }
                let Some(measured) = wr.wind else { continue };
                for model in w.loaded() {
                    if let Some(m) = w.store_of(model).and_then(|st| model_at(st, wr.pos, wr.t)) {
                        compared.push((wr.t, model, (m.tws, m.twd), measured));
                    }
                }
            }
        }
        if !compared.is_empty() {
            let mut w = app.weather.borrow_mut();
            for (t, model, m, measured) in compared {
                w.tracker.add(t, model, m, measured);
            }
        }
        // The stream measured, and each current source's there and then.
        {
            let mut w = app.weather.borrow_mut();
            let field = w.currents.field();
            for wr in written.iter().filter(|wr| wr.stream.is_some()) {
                let each = field.each_at(&wr.pos, wr.t);
                w.currents.tracker.add(wr.t, wr.stream.unwrap_or_default(), each);
            }
        }
        forecast_drift(app);
        current_drift(app);
    }
    let mode = app.session.borrow().perf.mode.as_str();
    if app.settings.borrow().sail_mode != mode {
        app.settings.borrow_mut().sail_mode = mode.to_string();
        app.apply_settings();
    }
}

/// Tells the user, once, when another model has been clearly closer to the measured wind than the one
/// in use, and clears that when it no longer is.
fn forecast_drift(app: &AppState) {
    use crate::core::alerts::{Level, Source};
    let now = Utc::now();
    let better = {
        let w = app.weather.borrow();
        w.tracker.better_than(w.source, now).map(|b| (b.source, w.source))
    };
    let mut s = app.session.borrow_mut();
    match better {
        Some((best, used)) => {
            let text = gettext_f("{best} is closer to your wind than {used}: consider routing on it", &[("best", best.label()), ("used", used.label())]);
            s.alerts.raise(now, Level::Info, Source::ForecastDrift, text, None);
        }
        None => s.alerts.clear(&Source::ForecastDrift),
    }
}

/// Tells the user when the current source in use has been far off the stream the boat measures for
/// half an hour, and clears that when it no longer is.
fn current_drift(app: &AppState) {
    use crate::core::alerts::{Level, Source};
    let now = Utc::now();
    let far = app.weather.borrow().currents.tracker.far_off(now);
    let mut s = app.session.borrow_mut();
    match far {
        Some((source, measured, forecast)) => {
            let text = gettext_f(
                "The stream measured is {m}, {source} says {f}: the current used for routing is off here",
                &[("m", &crate::currents::current_text(measured.0, measured.1)), ("source", source.label()), ("f", &crate::currents::current_text(forecast.0, forecast.1))],
            );
            s.alerts.raise(now, Level::Info, Source::CurrentDrift, text, None);
        }
        None => s.alerts.clear(&Source::CurrentDrift),
    }
}

/// One loaded model against the measured wind over the last hours, in words, for the weather page.
#[derive(serde::Serialize, Clone, Debug, PartialEq)]
pub struct Tracking {
    pub label: String,
    pub text: String,
    /// The closest of them.
    pub best: bool,
}

pub fn tracking(app: &AppState) -> Vec<Tracking> {
    let w = app.weather.borrow();
    let scores = w.tracker.scores(Utc::now());
    scores
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let speed = if s.tws_bias.abs() < 1.0 {
                gettext("speed right")
            } else if s.tws_bias > 0.0 {
                gettext_f("{kn} kn too strong", &[("kn", &format!("{:.0}", s.tws_bias))])
            } else {
                gettext_f("{kn} kn too light", &[("kn", &format!("{:.0}", -s.tws_bias))])
            };
            let dir = if s.twd_bias.abs() < 5.0 {
                gettext("direction right")
            } else if s.twd_bias > 0.0 {
                gettext_f("veered {deg}°", &[("deg", &format!("{:.0}", s.twd_bias))])
            } else {
                gettext_f("backed {deg}°", &[("deg", &format!("{:.0}", -s.twd_bias))])
            };
            let text = gettext_f(
                "{speed}, {dir}; off by {kn} kn and {deg}° on average over {h} measured",
                &[("speed", &speed), ("dir", &dir), ("kn", &format!("{:.1}", s.tws_rms)), ("deg", &format!("{:.0}", s.twd_rms)), ("h", &crate::core::nav::calc::format_duration(chrono::Duration::minutes((s.hours * 60.0) as i64)))],
            );
            Tracking { label: s.source.label().to_string(), text, best: i == 0 && scores.len() > 1 }
        })
        .collect()
}

/// The user flips the switch. False while an engine decides.
pub fn set_mode(app: &AppState, mode: SailMode) -> bool {
    let ok = app.session.borrow_mut().set_sail_mode(mode);
    if ok {
        app.settings.borrow_mut().sail_mode = mode.as_str().to_string();
        app.apply_settings();
    }
    ok
}

/// Whether the report is missing or older than the active polar's log.
pub fn stale(app: &AppState) -> bool {
    let id = crate::polars::active_id(app);
    let n = app.session.borrow().store.perf_summary(Some(&id)).map(|s| s.count).unwrap_or(0);
    let p = app.perf.borrow();
    !p.analysing && (p.report.is_none() || p.analysed_count != n)
}

/// Analyses the log again, as a job, off the main thread (on it for a database in memory: the tests).
pub fn analyse_now(app: &Rc<AppState>) {
    if app.perf.borrow().analysing {
        return;
    }
    app.perf.borrow_mut().analysing = true;
    let app = app.clone();
    crate::platform::spawn(async move {
        let job = app.jobs.start("perf_analyse", &gettext("Analysing the sailing log"), None);
        // The active polar learns only from what was logged against it.
        let polar = crate::polars::active(&app);
        let polar_id = crate::polars::active_id(&app);
        let cal = app.settings.borrow().perf_calibration.clone();
        let path = app.session.borrow().store.path();
        let now = Utc::now();
        let result: Result<(Report, usize), String> = match path {
            Some(path) => {
                crate::platform::bg(move || {
                    let store = NavStore::open_readonly(&path).map_err(|e| e.to_string())?;
                    let samples = store.perf_samples_of(&polar_id, None, None).map_err(|e| e.to_string())?;
                    Ok((analyse(&samples, &polar, cal.as_ref(), now), samples.len()))
                })
                .await
            }
            None => app.session.borrow().store.perf_samples_of(&polar_id, None, None).map_err(|e| e.to_string()).map(|samples| (analyse(&samples, &polar, cal.as_ref(), now), samples.len())),
        };
        drop(job);
        let passages = app.session.borrow().store.passages().unwrap_or_default();
        {
            let mut p = app.perf.borrow_mut();
            p.analysing = false;
            p.passages = passages;
            match result {
                Ok((r, n)) => {
                    p.report = Some(Rc::new(r));
                    p.analysed_count = n;
                    p.analysed_at = Some(now);
                    p.error = None;
                }
                Err(e) => p.error = Some(e),
            }
        }
        app.refresh();
    });
}

/// The samples of the last days without a hindcast, grouped by the GFS cycle they fall in.
fn hindcast_wanted(app: &AppState, now: DateTime<Utc>) -> BTreeMap<DateTime<Utc>, Vec<(i64, DateTime<Utc>, LatLon)>> {
    let rows = app.session.borrow().store.perf_without_hindcast(now - Duration::days(HINDCAST_DAYS)).unwrap_or_default();
    let mut by: BTreeMap<DateTime<Utc>, Vec<_>> = BTreeMap::new();
    for r in rows {
        let c = cycle_of(r.1);
        if now - c >= HINDCAST_AFTER {
            by.entry(c).or_default().push(r);
        }
    }
    by
}

/// How many samples a hindcast could be fetched for now: those of the last days whose cycle is old enough.
pub fn hindcast_pending(app: &AppState) -> usize {
    let now = Utc::now();
    let before = cycle_of(now - HINDCAST_AFTER) + Duration::hours(6);
    app.session.borrow().store.perf_count_without_hindcast(now - Duration::days(HINDCAST_DAYS), before).unwrap_or(0)
}

/// Fetches the analysis hours of the runs the log's last days fell in, and keeps the wind they give at
/// each sample: closer to what the wind was than the forecast the boat had.
pub fn backfill(app: &Rc<AppState>) {
    if app.perf.borrow().backfilling || std::env::var_os("ISOMAR_OFFLINE").is_some() {
        return;
    }
    let wanted = hindcast_wanted(app, Utc::now());
    if wanted.is_empty() {
        return;
    }
    app.perf.borrow_mut().backfilling = true;
    let app = app.clone();
    crate::platform::spawn(async move {
        let cancel = Arc::new(AtomicBool::new(false));
        let job = {
            let c = cancel.clone();
            Rc::new(app.jobs.start("perf_backfill", &gettext("Fetching the wind as it was"), Some(Rc::new(move || c.store(true, Ordering::Relaxed)))))
        };
        let total = wanted.len();
        let (tx, rx) = async_channel::unbounded::<usize>();
        let progress = {
            let job = job.clone();
            async move {
                while let Ok(done) = rx.recv().await {
                    job.progress(done as f64 / total as f64, &gettext_f("{done} of {total} forecast runs", &[("done", &done.to_string()), ("total", &total.to_string())]));
                }
            }
        };
        crate::platform::spawn(progress);
        let c = cancel.clone();
        let work = crate::platform::bg(move || -> Result<Vec<(i64, ModelWind)>, String> {
            let cache = Cache::new(&datadir::cache_dir());
            let mut out = Vec::new();
            for (done, (run, rows)) in wanted.into_iter().enumerate() {
                if c.load(Ordering::Relaxed) {
                    return Err(gettext("Cancelled"));
                }
                let (mut lat, mut lon) = ((f64::MAX, f64::MIN), (f64::MAX, f64::MIN));
                for (_, _, p) in &rows {
                    lat = (lat.0.min(p.lat), lat.1.max(p.lat));
                    lon = (lon.0.min(p.lon), lon.1.max(p.lon));
                }
                let plan = analysis_plan(run, &Area::new(lat.0 - 0.5, lon.0 - 0.5, lat.1 + 0.5, lon.1 + 0.5));
                let got = match fetch_plan(&plan, &cache, &c, &|_: Progress| {}) {
                    Ok(g) => g,
                    Err(e) => {
                        isomar_core::log!("perf: no hindcast for the {run} run: {e}");
                        let _ = tx.send_blocking(done + 1);
                        continue;
                    }
                };
                let mut store = WeatherStore::new();
                for path in &got.paths {
                    if let Ok(data) = std::fs::read(path) {
                        store.extend(read_fields(&data).unwrap_or_default());
                    }
                }
                for (id, t, pos) in rows {
                    if let Some(w) = store.wind(pos.lat, pos.lon, t) {
                        out.push((id, ModelWind { tws: (w.speed_ms() * MS_TO_KN) as f64, twd: w.blows_from() as f64, gust: None, hs: None, wave_dir: None, wave_period: None, run: Some(run), lead_h: Some((t - run).num_seconds() as f64 / 3600.0), current: None }));
                    }
                }
                let _ = tx.send_blocking(done + 1);
            }
            Ok(out)
        });
        // A cancel returns at once; the worker stops at its next check (the hook set `cancel`).
        let result = job.unless_cancelled(work).await.unwrap_or_else(|| Err(gettext("Cancelled")));
        drop(job);
        app.perf.borrow_mut().backfilling = false;
        match result {
            Ok(rows) => {
                let n = rows.len();
                {
                    let s = app.session.borrow();
                    for (id, m) in &rows {
                        let _ = s.store.set_perf_hindcast(*id, m);
                    }
                }
                app.perf.borrow_mut().analysed_count = usize::MAX;
                if n > 0 {
                    app.toast(&gettext_f("The analysed wind was added to {n} samples", &[("n", &n.to_string())]));
                }
            }
            Err(e) => app.toast(&e),
        }
        app.refresh();
    });
}

/// Puts the learnt values into the active polar where their confidence reaches `min_confidence`, keeping
/// the previous one for `undo_polar`. Returns how many cells changed.
pub fn apply_polar(app: &AppState, min_confidence: f64) -> Result<usize, String> {
    let report = app.perf.borrow().report.clone().ok_or_else(|| gettext("Analyse the log first"))?;
    let learned = report.learned.as_ref().ok_or_else(|| gettext("The log has no instrument data to learn from"))?;
    let mut polar = crate::polars::active(app).resampled(&EDITOR_TWS);
    let changed = merge_learned(&mut polar, learned, min_confidence);
    if changed == 0 {
        return Ok(0);
    }
    let (lib, id) = (crate::polars::library(), crate::polars::active_id(app));
    lib.keep_previous(&id).map_err(|e| e.to_string())?;
    lib.save(&id, &polar).map_err(|e| e.to_string())?;
    crate::polars::active_changed(app);
    Ok(changed)
}

/// Copies the learnt speeds into `polar`, at its own angles and wind speeds, where they are confident
/// enough. Returns how many cells changed.
pub fn merge_learned(polar: &mut Polar, learned: &crate::core::perf::learn::Learned, min_confidence: f64) -> usize {
    let mut changed = 0;
    for (row, a) in polar.twa.clone().iter().enumerate() {
        if *a <= 0.0 {
            continue;
        }
        for (col, w) in polar.tws.clone().iter().enumerate() {
            if learned.confidence_at(*a, *w) >= min_confidence {
                let v = (learned.polar.speed(*a, *w) * 100.0).round() / 100.0;
                if (polar.speeds[row][col] - v).abs() >= 0.05 {
                    polar.set_speed(row, col, v);
                    changed += 1;
                }
            }
        }
    }
    changed
}

/// Puts back the active polar as it was before the learnt values were applied.
pub fn undo_polar(app: &AppState) -> Result<(), String> {
    crate::polars::library().restore_previous(&crate::polars::active_id(app)).map_err(|_| gettext("There is no earlier polar to go back to"))?;
    crate::polars::active_changed(app);
    Ok(())
}

/// Weather routing starts from this performance of the active polar from now on.
pub fn apply_factor(app: &AppState, pct: f64) {
    crate::polars::set_performance_pct(app, pct);
}

/// The log as a CSV file, one row per sample, of every polar. Returns the number of rows.
pub fn export_csv(app: &AppState, path: &std::path::Path) -> Result<usize, String> {
    let samples = app.session.borrow().store.perf_samples(None, None).map_err(|e| e.to_string())?;
    let o = |v: Option<f64>| v.map(|x| format!("{x:.2}")).unwrap_or_default();
    let mut out = String::from("time,lat,lon,sog,cog,heading,stw,awa,aws,twd,tws,heel,rpm,mode,hint,forecast_tws,forecast_twd,forecast_hs,forecast_lead_h,hindcast_tws,hindcast_twd,polar\n");
    for s in &samples {
        let f = s.forecast;
        let h = s.hindcast;
        out.push_str(&format!(
            "{},{:.5},{:.5},{:.2},{:.1},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            s.t.to_rfc3339(),
            s.pos.lat,
            s.pos.lon,
            s.sog,
            s.cog,
            o(s.heading),
            o(s.stw),
            o(s.awa),
            o(s.aws),
            o(s.twd),
            o(s.tws),
            o(s.heel),
            o(s.rpm),
            s.mode.as_str(),
            if s.hint == crate::core::perf::Hint::LooksMotored { "motored" } else { "" },
            o(f.map(|m| m.tws)),
            o(f.map(|m| m.twd)),
            o(f.and_then(|m| m.hs)),
            o(f.and_then(|m| m.lead_h)),
            o(h.map(|m| m.tws)),
            o(h.map(|m| m.twd)),
            s.polar,
        ));
    }
    std::fs::write(path, out).map_err(|e| e.to_string())?;
    Ok(samples.len())
}

/// Forgets what was logged against the active polar, and the passages.
pub fn clear(app: &AppState) {
    let id = crate::polars::active_id(app);
    {
        let s = app.session.borrow();
        let _ = s.store.clear_perf_of(&id);
        let _ = s.store.clear_passages();
    }
    let mut p = app.perf.borrow_mut();
    p.report = None;
    p.passages.clear();
    p.analysed_count = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live: the analysis hours of a run of yesterday, off Brittany, give a wind at a sample's place.
    #[test]
    #[ignore]
    fn hindcast_from_nomads() {
        let run = cycle_of(Utc::now() - Duration::hours(30));
        let plan = analysis_plan(run, &Area::new(47.0, -4.5, 48.0, -3.0));
        assert_eq!(plan.jobs.len(), 7);
        let dir = tempfile::tempdir().unwrap();
        let got = fetch_plan(&plan, &Cache::new(dir.path()), &AtomicBool::new(false), &|_: Progress| {}).expect("NOMADS answers");
        let mut store = WeatherStore::new();
        for p in &got.paths {
            store.extend(read_fields(&std::fs::read(p).unwrap()).unwrap());
        }
        let w = store.wind(47.5, -3.7, run + Duration::hours(3)).expect("wind at the sample");
        let kn = w.speed_ms() * MS_TO_KN;
        println!("{run}: {kn:.1} kn from {:.0}", w.blows_from());
        assert!((0.0..80.0).contains(&kn));
        assert_eq!(store.run_at(Param::WindU, 47.5, -3.7, run + Duration::hours(3)), Some(run));
    }

    /// A west wind of `ms` m/s over Brittany, from an hour ago for the next two.
    fn west(ms: f32) -> WeatherStore {
        use crate::core::chart::grid::Grid;
        use crate::core::weather::model::Field;
        let mut s = WeatherStore::new();
        let t0 = Utc::now() - Duration::hours(1);
        for h in 0..=3 {
            for (p, v) in [(Param::WindU, ms), (Param::WindV, 0.0)] {
                s.add(Field { param: p, time: t0 + Duration::hours(h), run: Some(t0), grid: Grid { ncols: 10, nrows: 10, xll: -8.0, yll: 44.0, dx: 1.0, dy: 1.0, data: vec![v; 100] } });
            }
        }
        s
    }

    #[test]
    fn the_model_nearest_the_measured_wind_is_named_and_suggested() {
        use crate::core::alerts::Source as Alert;
        use crate::core::weather::source::Source;
        let canvas = crate::engine::Engine::new(crate::config::ViewState { lat: 48.0, lon: -4.0, zoom: 8.0 }, crate::palette::Mode::Day);
        let app = AppState::new(canvas, crate::core::session::Session::new(crate::core::nav::store::NavStore::open_memory().unwrap()).unwrap(), crate::config::Settings::default());
        {
            let mut w = app.weather.borrow_mut();
            // GFS, shown and routed on, says 10 kn; IFS 19 kn; the boat measures 20 kn from the west.
            *w.store_of_mut(Source::Gfs) = west(10.0 / MS_TO_KN);
            *w.store_of_mut(Source::Ifs) = west(19.0 / MS_TO_KN);
        }
        let now = Utc::now();
        let pos = crate::core::geo::LatLon::new(47.5, -4.0);
        app.session.borrow_mut().perf_written = (0..240).map(|i| crate::core::session::Written { id: 0, t: now - Duration::seconds(10 * (240 - i)), pos, wind: Some((20.0, 270.0)), stream: None }).collect();
        after_poll(&app);
        let rows = tracking(&app);
        assert_eq!(rows.iter().map(|r| (r.label.as_str(), r.best)).collect::<Vec<_>>(), vec![("ECMWF IFS", true), ("GFS", false)]);
        assert!(rows[1].text.contains("10 kn too light"), "{}", rows[1].text);
        let text = app.session.borrow().alerts.by_source(&Alert::ForecastDrift).map(|a| a.message.clone());
        assert!(text.as_deref().is_some_and(|t| t.contains("ECMWF IFS is closer")), "{text:?}");
        // Routing on IFS, nothing to suggest.
        app.weather.borrow_mut().show(Source::Ifs);
        app.session.borrow_mut().perf_written = vec![crate::core::session::Written { id: 0, t: now, pos, wind: Some((20.0, 270.0)), stream: None }];
        after_poll(&app);
        assert!(app.session.borrow().alerts.by_source(&Alert::ForecastDrift).is_none());
    }

    #[test]
    fn the_stream_measured_is_the_set_and_drift_and_a_source_far_off_it_is_said() {
        use crate::core::alerts::Source as Alert;
        use crate::core::currents::CurrentSource;
        let canvas = crate::engine::Engine::new(crate::config::ViewState { lat: 48.0, lon: -4.8, zoom: 8.0 }, crate::palette::Mode::Day);
        let app = AppState::new(canvas, crate::core::session::Session::new(crate::core::nav::store::NavStore::open_memory().unwrap()).unwrap(), crate::config::Settings::default());
        let now = Utc::now();
        // The demo's current in the Raz; the boat measures 3 kn setting west, which it is not.
        app.weather.borrow_mut().currents.add(CurrentSource::Demo, crate::currents::demo_fields(now));
        let pos = crate::core::geo::LatLon::new(48.035, -4.79);
        app.session.borrow_mut().perf_written = (0..240).map(|i| crate::core::session::Written { id: 0, t: now - Duration::seconds(10 * (240 - i)), pos, wind: None, stream: Some((-3.0, 0.0)) }).collect();
        after_poll(&app);
        let tile = crate::vm::instruments::tiles(&app).pop().unwrap();
        assert_eq!((tile.value.as_str(), tile.unit.as_str()), ("270° 3.0", "kn"));
        let text = app.session.borrow().alerts.by_source(&Alert::CurrentDrift).map(|a| a.message.clone());
        assert!(text.as_deref().is_some_and(|t| t.contains("Demo says")), "{text:?}");
        assert!(crate::currents::page(&app).tracking[0].starts_with("Demo:"));
    }
}
