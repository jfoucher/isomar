//! Getting the currents for a passage (inside a route's or a scan's job) or for the view (the weather
//! page's button): Copernicus Marine, finest first, cached for the day. A failure is told and the route
//! goes on without them, never an error.

use crate::app::AppState;
use crate::core::currents::copernicus::{self, Progress};
use crate::core::currents::{CurrentSource, WithCurrents};
use crate::core::geo::LatLon;
use crate::core::routing::isochrone::{ConstantWind, Settings};
use crate::core::routing::passage;
use crate::core::vector::Bounds;
use crate::core::weather::model::Param;
use crate::i18n::{gettext, gettext_f};
use crate::jobs::JobHandle;
use chrono::{DateTime, Duration, Utc};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A passage's currents are fetched over its whole area up to this estimate; past it, round its ends
/// only, where the tidal streams that matter are.
pub const MAX_BYTES: u64 = 40 * 1024 * 1024;
/// Half the side of the box fetched round each end of a long passage, degrees.
pub const END_HALF_DEG: f64 = 1.0;
/// How far ahead a loaded current must reach to count as covering a passage (the shelf models end
/// about six days on).
const AHEAD_HOURS: i64 = 96;

fn mb(bytes: u64) -> String {
    format!("{:.0}", (bytes as f64 / 1_048_576.0).max(1.0))
}

/// Whether what is loaded already covers `area` until `until`, from every source `choose` would fetch.
fn covered(app: &AppState, area: &Bounds, residual: bool, until: DateTime<Utc>) -> bool {
    let w = app.weather.borrow();
    let until = until.min(Utc::now() + Duration::hours(AHEAD_HOURS));
    let rounded = copernicus::rounded(area);
    copernicus::choose(area, residual).iter().all(|(d, vars)| {
        let p = if d.tide.is_some() && vars.len() == 2 { Param::ResidualU } else { Param::CurrentU };
        w.currents.store(d.source).is_some_and(|st| st.reaches(p, &rounded, until) || st.reaches(p, area, until))
    })
}

/// Whether an installed atlas reaches into `area`: then SMOC's residual is wanted to go with it.
fn atlas_in(app: &AppState, area: &Bounds) -> bool {
    app.weather.borrow().currents.tides().iter().any(|t| t.bounds().intersects(area))
}

/// The boxes to fetch for a passage over `area` from `start` to `dest`: the area, or round the ends
/// when the area would cost more than `MAX_BYTES`.
pub fn boxes(area: &Bounds, start: LatLon, dest: LatLon, residual: bool) -> Vec<Bounds> {
    if copernicus::estimate_bytes(area, residual) <= MAX_BYTES {
        return vec![*area];
    }
    let around = |p: LatLon| Bounds { min_lon: p.lon - END_HALF_DEG, max_lon: p.lon + END_HALF_DEG, min_lat: p.lat - END_HALF_DEG, max_lat: p.lat + END_HALF_DEG };
    if start.distance_nm(&dest) < 120.0 {
        let mut b = around(start);
        b.union(&around(dest));
        return vec![b];
    }
    vec![around(start), around(dest)]
}

/// Fetches the currents of `boxes` on a worker, the job's progress following it. True when something
/// came; a failure is a toast (`quiet` leaves it out).
async fn fetch(app: &Rc<AppState>, job: &JobHandle, boxes: Vec<Bounds>, residual: bool, quiet: bool) -> bool {
    let cancel = Arc::new(AtomicBool::new(false));
    let _stop = {
        let c = cancel.clone();
        crate::jobs::on_drop(move || c.store(true, Ordering::Relaxed))
    };
    let (tx, rx) = async_channel::unbounded::<(CurrentSource, Progress)>();
    let cache = crate::core::datadir::cache_dir();
    let c2 = cancel.clone();
    let work = crate::platform::bg(move || {
        let mut all: Vec<(CurrentSource, Vec<crate::core::weather::model::Field>)> = Vec::new();
        let mut last_err = None;
        for b in boxes {
            match copernicus::fetch_all(&b, residual, Utc::now() - Duration::hours(1), &cache, &c2, &|s, p| {
                let _ = tx.try_send((s, p));
            }) {
                Ok(m) => all.extend(m),
                Err(e) => last_err = Some(e),
            }
        }
        if all.is_empty() { Err(last_err.unwrap_or(crate::core::weather::download::DlError::NotAvailable("nothing covers it".into()))) } else { Ok(all) }
    });
    let watch = async {
        while let Ok((s, p)) = rx.recv().await {
            if p.chunks_total > 0 {
                job.progress(
                    p.chunks_done as f64 / p.chunks_total as f64,
                    &gettext_f("{source}: {mb} MB", &[("source", s.label()), ("mb", &format!("{:.1}", p.bytes as f64 / 1_048_576.0))]),
                );
            }
        }
    };
    let got = futures_lite_select(work, watch).await;
    match got {
        Ok(all) => {
            {
                let mut w = app.weather.borrow_mut();
                for (s, fields) in all {
                    w.currents.add(s, fields);
                }
                if w.store.is_empty() && let Some((lo, hi)) = w.currents.field().time_range() {
                    w.time = Utc::now().clamp(lo, hi);
                }
            }
            app.refresh();
            true
        }
        Err(crate::core::weather::download::DlError::Cancelled) => false,
        Err(e) => {
            isomar_core::log!("currents: {e}");
            if !quiet {
                app.toast(&gettext_f("The currents could not be fetched ({e}); the route goes on without them.", &[("e", &crate::flows::weather::explain(&e))]));
            }
            false
        }
    }
}

/// Runs `work` while `watch` follows it, and returns what `work` gave.
async fn futures_lite_select<T>(work: impl std::future::Future<Output = T>, watch: impl std::future::Future<Output = ()>) -> T {
    let mut work = Box::pin(work);
    let mut watch = Box::pin(watch);
    std::future::poll_fn(move |cx| {
        let _ = watch.as_mut().poll(cx);
        work.as_mut().poll(cx)
    })
    .await
}

/// The currents of a passage over `area` until `until`, fetched in `job` unless loaded already, when
/// routes use currents and Copernicus is switched on.
pub async fn ensure(app: &Rc<AppState>, job: &JobHandle, area: Bounds, start: LatLon, dest: LatLon, until: DateTime<Utc>) {
    let (on, fetch_on) = (app.weather.borrow().currents.route_with, app.settings.borrow().currents.fetch_copernicus);
    if !on || !fetch_on || app.weather.borrow().currents.store(CurrentSource::Demo).is_some() {
        return;
    }
    let residual = atlas_in(app, &area);
    let boxes = boxes(&area, start, dest, residual);
    if boxes.iter().all(|b| covered(app, b, residual, until)) {
        isomar_core::log!("currents: loaded already for {area:?}");
        return;
    }
    if boxes.len() > 1 || boxes[0] != area {
        app.toast(&gettext_f("The currents of the whole passage would be about {mb} MB: fetched round its ends only.", &[("mb", &mb(copernicus::estimate_bytes(&area, residual)))]));
    }
    job.detail(&gettext("Getting the currents"));
    fetch(app, job, boxes, residual, false).await;
}

/// Where the passage has a current: the boat may wait for the stream to turn, and the legs are
/// shortened where the stream along `path` runs strong (`passage::for_currents`), from `depart` for
/// `hours`. Returns the strongest stream along the way, knots (0 without currents).
pub fn tide_settings(app: &AppState, settings: &mut Settings, nm: f64, area: &Bounds, path: &[LatLon], depart: DateTime<Utc>, hours: f64) -> f64 {
    let Some(field) = app.weather.borrow().currents.for_route(area) else { return 0.0 };
    let env = WithCurrents { wind: ConstantWind { kn: 0.0, from: 0.0 }, currents: field.as_ref() };
    let peak = passage::peak_current_kn(&env, path, depart, hours);
    let before = settings.step_minutes;
    passage::for_currents(settings, nm, peak);
    settings.allow_hold = true;
    isomar_core::log!("currents: peak {peak:.1} kn along the way; legs of {} min (were {before})", settings.step_minutes);
    peak
}

/// "Get currents here": the view's currents, as a job of its own.
pub async fn fetch_view(app: &Rc<AppState>) {
    let b = app.canvas.view_info().bounds();
    let area = Bounds { min_lon: b.min_lon.max(-180.0), max_lon: b.max_lon.min(180.0), min_lat: b.min_lat.max(-80.0), max_lat: b.max_lat.min(90.0) };
    let residual = atlas_in(app, &area);
    let est = copernicus::estimate_bytes(&area, residual);
    if est > MAX_BYTES * 2 {
        app.show_error(&gettext("Zoom in to get currents"), &gettext_f("The currents of this view would be about {mb} MB.", &[("mb", &mb(est))]));
        return;
    }
    let job = Rc::new(app.jobs.start_cancellable("currents", &gettext("Getting the currents")));
    let done = job.unless_cancelled(fetch(app, &job, vec![area], residual, false)).await;
    if done == Some(true) {
        let sources = app.weather.borrow().currents.sources_text(&area).unwrap_or_default();
        app.toast(&gettext_f("Currents loaded: {sources}", &[("sources", &sources)]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_passage_gets_its_currents_round_its_ends() {
        let channel = Bounds { min_lon: -3.0, min_lat: 49.0, max_lon: -1.0, max_lat: 50.5 };
        let (a, b) = (LatLon::new(49.6, -1.6), LatLon::new(50.4, -2.0));
        assert_eq!(boxes(&channel, a, b, false), vec![channel], "a Channel crossing whole");
        let ocean = Bounds { min_lon: -40.0, min_lat: 30.0, max_lon: -5.0, max_lat: 50.0 };
        let (c, d) = (LatLon::new(48.0, -5.0), LatLon::new(37.0, -25.0));
        let got = boxes(&ocean, c, d, false);
        assert_eq!(got.len(), 2, "Brest to the Azores: round each end");
        assert!(got[0].contains([c.lon, c.lat]) && got[1].contains([d.lon, d.lat]));
    }
}
