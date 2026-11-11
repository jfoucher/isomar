//! Getting the weather: what to ask for, what it will cost, and the download itself, in the
//! order NOMADS publishes it. The download runs on a worker thread; `run_download` makes it a job
//! the shells show with a progress bar and a Cancel button.

use crate::app::AppState;
use crate::core::datadir;
use crate::core::geo;
use crate::core::weather::cache::Cache;
use crate::core::weather::download::{DlError, Progress, fetch_plan};
use crate::core::weather::grib::read_fields;
use crate::core::weather::model::{Field, Group};
use crate::core::weather::planner::{Area, Model, Request, Source, format_bytes, plan};
use crate::i18n::{gettext, gettext_f};
use crate::platform::bg;
use chrono::{DateTime, Duration, Utc};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub const HORIZONS: [(u32, &str); 4] = [(24, "1 day"), (72, "3 days"), (120, "5 days"), (168, "7 days")];
pub const STEPS: [u32; 3] = [1, 3, 6];
pub const GROUPS: [(Group, &str, bool); 6] = [
    (Group::Wind, "Wind", true),
    (Group::Gust, "Gusts", true),
    (Group::Pressure, "Pressure", true),
    (Group::Waves, "Waves", true),
    (Group::Temperature, "Air temperature", false),
    (Group::Cloud, "Cloud cover", false),
];

/// The request described by the dialog's controls, if they make sense.
pub struct Choices {
    pub north: String,
    pub south: String,
    pub west: String,
    pub east: String,
    pub horizon_index: usize,
    pub step_index: usize,
    pub groups: Vec<Group>,
    pub source: Source,
}

impl Choices {
    pub fn request(&self) -> Option<Request> {
        let n = geo::parse_coord(&self.north, true)?;
        let s = geo::parse_coord(&self.south, true)?;
        let w = geo::parse_coord(&self.west, false)?;
        let e = geo::parse_coord(&self.east, false)?;
        let area = Area::new(s, w, n, e);
        if !area.is_valid() || self.groups.is_empty() {
            return None;
        }
        Some(Request { source: self.source, area, groups: self.groups.clone(), horizon_hours: HORIZONS.get(self.horizon_index)?.0, step_hours: *STEPS.get(self.step_index)? })
    }
}


/// What a request will cost, as the dialog's "What will be downloaded" section says.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Estimate {
    pub source: String,
    pub run: String,
    pub size: String,
    pub files: String,
    /// What the model does not have of what was asked ("ECMWF AIFS has no gusts"), or empty.
    pub missing: String,
    /// There is something to download.
    pub ok: bool,
}

pub fn estimate(req: Option<&Request>) -> Estimate {
    let dash = || "—".to_string();
    let Some(req) = req else { return Estimate { source: dash(), run: dash(), size: dash(), files: dash(), missing: String::new(), ok: false } };
    let now = Utc::now();
    let p = plan(req, req.source.candidate_runs(now)[0], now);
    let missing: Vec<String> = p.unavailable.iter().filter_map(|g| GROUPS.iter().find(|x| x.0 == *g)).map(|x| gettext(x.1).to_lowercase()).collect();
    let a = req.area;
    if !req.source.covers(a.south, a.west, a.north, a.east) {
        return Estimate { missing: gettext_f("{model} does not cover this area", &[("model", req.source.label())]), ..estimate(None) };
    }
    Estimate {
        source: if p.description.is_empty() { dash() } else { p.description.split(", run").next().unwrap_or("").to_string() },
        run: format!("{}Z", p.run.format("%Y-%m-%d %H")),
        size: if p.is_empty() { dash() } else { format!("≈ {}", format_bytes(p.est_bytes)) },
        files: p.jobs.len().to_string(),
        missing: if missing.is_empty() { String::new() } else { gettext_f("{model} has no {what}", &[("model", req.source.label()), ("what", &missing.join(", "))]) },
        ok: !p.is_empty(),
    }
}

pub fn explain(e: &DlError) -> String {
    match e {
        DlError::NotAvailable(_) => gettext("No recent forecast could be found on the server. It may not be published yet; try again in a while."),
        DlError::Throttled(_) => gettext("The weather server is turning requests away for a while, because too many came at once. Try again in a few minutes; what has arrived is kept."),
        DlError::Http(m) => gettext_f("The network failed: {error}", &[("error", m)]),
        DlError::Io(m) => gettext_f("The disk failed: {error}", &[("error", m)]),
        DlError::Cancelled => gettext("Cancelled"),
    }
}

/// The blocking part: asks the server for `req`, run by run, and reads the fields. The newest run is
/// published an hour at a time, so the hours it does not have yet come from the run before (and so
/// on), not the whole forecast again. `progress` hears of each step.
pub fn fetch_fields(req: &Request, cancel: &Arc<AtomicBool>, progress: &(dyn Fn(Progress) + Sync)) -> Result<Vec<Field>, DlError> {
        let cache = Cache::new(&datadir::cache_dir());
        let now = Utc::now();
        let mut fields = Vec::new();
        // The steps still to find, after the first run tried: those it has not published yet, and
        // those past its end when it stops short (ECMWF's 06 and 18 UTC runs end at 144 h).
        let mut wanted: Option<Vec<(Model, DateTime<Utc>)>> = None;
        let mut beyond: Option<DateTime<Utc>> = None;
        let until = now + Duration::hours(req.horizon_hours as i64);
        for run in req.source.candidate_runs(now) {
            let mut p = plan(req, run, now);
            if wanted.is_none() && beyond.is_none() {
                if p.is_empty() {
                    return Err(DlError::NotAvailable("nothing to download".into()));
                }
            } else {
                p.jobs.retain(|j| wanted.as_ref().is_some_and(|w| w.contains(&(j.model, j.valid))) || beyond.is_some_and(|b| j.valid > b));
                p.est_bytes = p.jobs.iter().map(|j| j.est_bytes).sum();
            }
            // An older run ends earlier still, so it cannot help either.
            if p.is_empty() {
                break;
            }
            isomar_core::log!("weather: trying the {run} run for {} files", p.jobs.len());
            let got = fetch_plan(&p, &cache, cancel, progress).inspect_err(|e| isomar_core::log!("weather: download failed: {e}"))?;
            for path in &got.paths {
                let data = std::fs::read(path).map_err(|e| DlError::Io(e.to_string()))?;
                fields.extend(read_fields(&data).map_err(|e| DlError::Io(e.to_string()))?);
            }
            let w: Vec<_> = got.missing.iter().map(|&i| (p.jobs[i].model, p.jobs[i].valid)).collect();
            let end = req.source.published_hours(run).last().map(|h| run + Duration::hours(*h as i64));
            // A day short at least: every run ends a little before now plus its length.
            let short = end.filter(|e| *e + Duration::hours(24) < until && beyond.is_none_or(|b| *e > b));
            if let Some(e) = short {
                isomar_core::log!("weather: the {run} run ends {e}, before the {until} asked for; the run before goes on");
                beyond = Some(e);
            } else if beyond.is_some() {
                beyond = None;
            }
            if w.is_empty() && beyond.is_none() {
                wanted = None;
                break;
            }
            if !w.is_empty() {
                isomar_core::log!("weather: {} files are not in the {run} run, looking in the one before", w.len());
            }
            wanted = Some(w);
        }
        if fields.is_empty() {
            return Err(DlError::NotAvailable("no recent run".into()));
        }
        if let Some(w) = wanted.filter(|w| !w.is_empty()) {
            isomar_core::log!("weather: {} steps were in no run; the forecast has gaps or ends early", w.len());
        }
        cache.prune(4);
        Ok(fields)
}

/// Puts downloaded fields of `source` in the weather state, shows that model, and moves the shown
/// time into the forecast.
pub fn apply_fields(app: &Rc<AppState>, source: Source, fields: Vec<Field>) -> usize {
    let n = fields.len();
    show_source(app, source);
    {
        let mut w = app.weather.borrow_mut();
        w.store_of_mut(source).extend(fields);
        let now = Utc::now();
        if let Some((lo, hi)) = w.store.time_range() {
            w.time = now.clamp(lo, hi.max(lo + Duration::minutes(1)));
        }
    }
    app.refresh();
    n
}

/// Shows `source`'s forecast on the chart and in the pages, and makes it the one weather routes use
/// unless another is chosen. A saved route being played back is closed first: it shows the forecast it
/// was planned on, in place of the one loaded.
pub fn show_source(app: &Rc<AppState>, source: Source) {
    if app.weather.borrow().source == source {
        return;
    }
    if app.router.borrow().as_ref().is_some_and(|p| p.replay.is_some()) {
        crate::router::close(app);
    }
    {
        let mut w = app.weather.borrow_mut();
        w.show(source);
        if let Some((lo, hi)) = w.store.time_range() {
            w.time = w.time.clamp(lo, hi.max(lo + Duration::minutes(1)));
        }
    }
    app.settings.borrow_mut().weather_source = source;
    app.refresh();
}

/// The models a user can pick from, with what is loaded of each: for the pickers of every shell.
#[derive(serde::Serialize, Clone, Debug, PartialEq)]
pub struct ModelChoice {
    pub id: String,
    pub label: String,
    /// What it is, in a sentence.
    pub about: String,
    /// What is loaded of it, then what it is.
    pub detail: String,
    pub loaded: bool,
    pub shown: bool,
}

pub fn model_choices(app: &AppState) -> Vec<ModelChoice> {
    let w = app.weather.borrow();
    Source::ALL
        .into_iter()
        .map(|s| {
            let about = match s {
                Source::Gfs => gettext("NOAA, 0.25°, to 16 days. Small downloads: the server cuts out the area."),
                Source::Ifs => gettext("ECMWF, 0.25°, to 15 days. Usually the most accurate, but the whole globe is downloaded for each step: tens of megabytes."),
                Source::Aifs => gettext("ECMWF's AI model, 0.25°, every 6 hours, to 15 days. No gusts. Whole-globe downloads, a little smaller."),
                Source::IconEu => gettext("DWD's regional model, about 7 km, Europe and the Mediterranean only, to 5 days: the coast's effect on the wind. Routes on it go on with GFS outside its area and after it ends; downloads cover all of Europe."),
            };
            let store = w.store_of(s);
            let detail = match store.and_then(|st| st.latest_run().zip(st.wind_range())) {
                Some((run, (_, end))) => gettext_f(
                    "Run {run}, until {end}. {about}",
                    &[("run", &run.format("%d %b %H UTC").to_string()), ("end", &end.with_timezone(&chrono::Local).format("%a %d %b %H:%M").to_string()), ("about", &about)],
                ),
                None => about.clone(),
            };
            ModelChoice { id: s.id().to_string(), label: s.label().to_string(), about, detail, loaded: store.is_some(), shown: s == w.source }
        })
        .collect()
}

/// Downloads the weather for `req` as a job (with a progress bar and Cancel) and loads it. Returns
/// whether it arrived; failures are reported to the shell.
pub async fn run_download(app: &Rc<AppState>, req: Request) -> bool {
    let cancel = Arc::new(AtomicBool::new(false));
    let job = {
        let cancel = cancel.clone();
        app.jobs.start("weather", &gettext("Downloading weather"), Some(Rc::new(move || cancel.store(true, Ordering::Relaxed))))
    };
    // Dropped half-way (this job or one waiting on it cancelled), the download stops too.
    let _stop = {
        let cancel = cancel.clone();
        crate::jobs::on_drop(move || cancel.store(true, Ordering::Relaxed))
    };
    let (tx, rx) = async_channel::unbounded::<Progress>();
    let c2 = cancel.clone();
    let source = req.source;
    let worker = bg(move || fetch_fields(&req, &c2, &|p| {
        let _ = tx.try_send(p);
    }));
    // Progress arrives while the download runs.
    let watcher = {
        let job = job_proxy(&job);
        async move {
            while let Ok(p) = rx.recv().await {
                job(p);
            }
        }
    };
    // A cancel returns at once; the workers stop between chunks and clean up behind.
    let result = job.unless_cancelled(join2(worker, watcher)).await.map_or(Err(DlError::Cancelled), |(r, ())| r);
    drop(job);
    match result {
        Ok(fields) => {
            let n = apply_fields(app, source, fields);
            app.toast(&gettext_f("Weather updated: {n} fields of {model}", &[("n", &n.to_string()), ("model", source.label())]));
            true
        }
        Err(DlError::Cancelled) => {
            app.toast(&gettext("Download cancelled"));
            false
        }
        Err(e) => {
            app.show_error(&gettext("Could not get the weather"), &explain(&e));
            false
        }
    }
}

fn job_proxy(job: &crate::jobs::JobHandle) -> impl Fn(Progress) + '_ {
    move |p| {
        if p.files_total > 0 {
            job.progress(p.files_done as f64 / p.files_total as f64, &gettext_f("{done} of {total} files, {size}", &[("done", &p.files_done.to_string()), ("total", &p.files_total.to_string()), ("size", &format_bytes(p.bytes_done))]));
        }
    }
}

/// Polls two futures together until both are done.
async fn join2<A: std::future::Future, B: std::future::Future>(a: A, b: B) -> (A::Output, B::Output) {
    let (mut a, mut b) = (Box::pin(a), Box::pin(b));
    let (mut ra, mut rb) = (None, None);
    std::future::poll_fn(move |cx| {
        if ra.is_none()
            && let std::task::Poll::Ready(v) = a.as_mut().poll(cx)
        {
            ra = Some(v);
        }
        if rb.is_none()
            && let std::task::Poll::Ready(v) = b.as_mut().poll(cx)
        {
            rb = Some(v);
        }
        if ra.is_some() && rb.is_some() {
            std::task::Poll::Ready((ra.take().unwrap(), rb.take().unwrap()))
        } else {
            std::task::Poll::Pending
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choices() -> Choices {
        Choices {
            north: "52°00.000′N".into(),
            south: "43°00.000′N".into(),
            west: "012°00.000′W".into(),
            east: "000°00.000′E".into(),
            horizon_index: 2,
            step_index: 1,
            groups: vec![Group::Wind, Group::Pressure],
            source: Source::Gfs,
        }
    }

    #[test]
    fn valid_choices_make_a_request() {
        let r = choices().request().unwrap();
        assert_eq!((r.horizon_hours, r.step_hours), (120, 3));
        assert_eq!((r.area.south, r.area.north, r.area.west, r.area.east), (43.0, 52.0, -12.0, 0.0));
        assert_eq!(r.groups, vec![Group::Wind, Group::Pressure]);
    }

    #[test]
    fn nonsense_is_refused() {
        let mut c = choices();
        c.north = "banana".into();
        assert!(c.request().is_none());
        let mut c = choices();
        c.groups.clear();
        assert!(c.request().is_none(), "nothing selected");
        let mut c = choices();
        c.south = c.north.clone();
        assert!(c.request().is_none(), "no height");
        let mut c = choices();
        c.horizon_index = 9;
        assert!(c.request().is_none());
    }

    #[test]
    fn typed_decimal_degrees_work_too() {
        let mut c = choices();
        c.north = "52".into();
        c.south = "43".into();
        c.west = "-12".into();
        c.east = "0".into();
        assert!(c.request().is_some());
    }

    #[test]
    fn failures_are_explained_in_plain_words() {
        assert!(explain(&DlError::NotAvailable("x".into())).contains("published"));
        assert!(explain(&DlError::Http("timeout".into())).contains("timeout"));
        assert!(explain(&DlError::Throttled("x".into())).contains("few minutes"));
    }
}
