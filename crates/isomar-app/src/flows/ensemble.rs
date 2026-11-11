//! The ensemble check of a route: the NOAA GEFS members' wind over the passage, cropped on the server,
//! the route as it is sailed through each (`routing::along`), and the spread of what they make of it
//! (`routing::ensemble`). One job, minutes long: 31 members a step under the NOMADS throttle.

use crate::app::AppState;
use crate::core::geo::LatLon;
use crate::core::routing::ensemble::{self, Spread};
use crate::core::routing::isochrone::{Bounded, StoreEnv};
use crate::core::weather::cache::Cache;
use crate::core::weather::download::{DlError, Progress, fetch_plan};
use crate::core::weather::grib::read_fields;
use crate::core::weather::planner::{self, Area, Plan};
use crate::core::weather::store::WeatherStore;
use crate::i18n::{gettext, gettext_f};
use chrono::{DateTime, Local, Utc};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// What the ensemble made of a route, for its page.
#[derive(Clone, Debug)]
pub struct Check {
    /// The route's points and departure it was made for: an edited route is checked again.
    pub points: Vec<LatLon>,
    pub depart: DateTime<Utc>,
    pub run: DateTime<Utc>,
    pub spread: Spread,
    /// The wind limit it was counted against.
    pub limit_kn: f64,
}

/// The check in words: when it arrives, how sure that is, and the wind.
pub fn describe(c: &Check) -> Vec<String> {
    let t = |d: DateTime<Utc>| d.with_timezone(&Local).format("%a %d %H:%M").to_string();
    let s = &c.spread;
    let mut out = Vec::new();
    match (s.p10, s.p50, s.p90) {
        (Some(a), Some(m), Some(z)) => out.push(gettext_f(
            "Leaving {dep}, {k} of {n} ensemble forecasts arrive; half by {mid}, nine in ten between {from} and {to}.",
            &[("dep", &t(c.depart)), ("k", &s.arrived.to_string()), ("n", &s.members.to_string()), ("mid", &t(m)), ("from", &t(a)), ("to", &t(z))],
        )),
        _ => out.push(gettext_f("Leaving {dep}, none of the {n} ensemble forecasts reach the destination before they end.", &[("dep", &t(c.depart)), ("n", &s.members.to_string())])),
    }
    out.push(gettext_f(
        "Strongest wind on the way: up to {p50} kn in half of them, {p90} kn in nine in ten; {k} of {n} meet more than your {limit} kn.",
        &[("p50", &format!("{:.0}", s.max_wind_p50)), ("p90", &format!("{:.0}", s.max_wind_p90)), ("k", &s.over_limit.to_string()), ("n", &s.members.to_string()), ("limit", &format!("{:.0}", c.limit_kn))],
    ));
    out.push(gettext_f("NOAA GEFS, run {run}.", &[("run", &c.run.format("%d %b %H UTC").to_string())]));
    out
}

/// The members' wind, each in a store of its own, for the newest run that has them all.
fn fetch_members(area: Area, from: DateTime<Utc>, hours: u32, cancel: &Arc<AtomicBool>, progress: &(dyn Fn(Progress) + Sync)) -> Result<(DateTime<Utc>, Vec<WeatherStore>), DlError> {
    let cache = Cache::new(&crate::core::datadir::cache_dir());
    for run in planner::gefs_runs(Utc::now()) {
        let plan: Plan = planner::ensemble_plan(run, &area, from, hours);
        let got = fetch_plan(&plan, &cache, cancel, progress)?;
        if !got.missing.is_empty() {
            crate::core::log!("ensemble: the {run} run lacks {} files, trying the one before", got.missing.len());
            continue;
        }
        let mut members = vec![WeatherStore::new(); planner::GEFS_MEMBERS as usize];
        for (job, path) in plan.jobs.iter().zip(&got.paths) {
            let fields = read_fields(&std::fs::read(path).map_err(|e| DlError::Io(e.to_string()))?).map_err(|e| DlError::Io(e.to_string()))?;
            members[job.member.unwrap_or(0) as usize].extend(fields);
        }
        return Ok((run, members));
    }
    Err(DlError::NotAvailable("no complete ensemble run".into()))
}

/// Checks route `route_id` leaving at its departure (or now) against the ensemble, as a job, and keeps
/// what it found for the route's page.
pub async fn check(app: &Rc<AppState>, route_id: i64) {
    let Some(r) = app.session.borrow().routes.iter().find(|r| r.id == route_id).cloned() else { return };
    if r.points.len() < 2 {
        return;
    }
    let now = Utc::now();
    let depart = r.departure.filter(|d| *d > now).unwrap_or(now);
    let points: Vec<LatLon> = r.points.iter().map(|p| p.pos).collect();
    let nm = crate::core::nav::calc::total_distance_nm(&r);
    let hours = (super::routing::passage_hours(nm) + (depart - now).num_hours() as f64).ceil() as u32;
    let mut b = crate::core::vector::Bounds::EMPTY;
    for p in &points {
        b.extend([p.lon, p.lat]);
    }
    let b = b.expanded(1.0);
    let area = Area::from_bounds(&b).outward(1.0);
    let cancel = Arc::new(AtomicBool::new(false));
    let job = {
        let cancel = cancel.clone();
        Rc::new(app.jobs.start(&format!("ensemble:{route_id}"), &gettext("Ensemble check"), Some(Rc::new(move || cancel.store(true, Ordering::Relaxed)))))
    };
    let _stop = {
        let cancel = cancel.clone();
        crate::jobs::on_drop(move || cancel.store(true, Ordering::Relaxed))
    };
    job.detail(&gettext("Getting the 31 GEFS forecasts"));
    let (tx, rx) = async_channel::unbounded::<Progress>();
    let c2 = cancel.clone();
    let polar = crate::polars::active(app);
    let mut settings = super::routing::settings_for_distance(nm);
    super::routing::BoatParams { performance_pct: super::routing::default_performance(app), ..Default::default() }.apply(&mut settings);
    let limit_kn = settings.max_wind_kn;
    let pts = points.clone();
    let work = crate::platform::bg(move || {
        let (run, members) = fetch_members(area, now, hours, &c2, &|p| {
            let _ = tx.try_send(p);
        })?;
        let sails: Vec<_> = members
            .iter()
            .map(|m| crate::core::routing::along::sail(&pts, depart, &polar, &Bounded { env: &StoreEnv(m), bounds: b }, &settings))
            .collect();
        Ok::<_, DlError>((run, ensemble::spread(&sails)))
    });
    let watch = {
        let job = job.clone();
        async move {
            while let Ok(p) = rx.recv().await {
                if p.files_total > 0 {
                    job.progress(p.files_done as f64 / p.files_total as f64, &gettext_f("{done} of {total} files", &[("done", &p.files_done.to_string()), ("total", &p.files_total.to_string())]));
                }
            }
        }
    };
    crate::platform::spawn(watch);
    let result = job.unless_cancelled(work).await;
    drop(job);
    match result {
        Some(Ok((run, spread))) => {
            let c = Check { points, depart, run, spread, limit_kn };
            app.toast(&describe(&c)[0]);
            app.finished(&gettext("Ensemble check done"), &describe(&c)[0]);
            app.ensembles.borrow_mut().insert(route_id, c);
            app.refresh();
            // The route's page again, with what the ensemble said.
            if let Some(shell) = app.shell() {
                shell.open(crate::shell::Dialog::RouteProperties(route_id));
            }
        }
        Some(Err(DlError::Cancelled)) | None => app.toast(&gettext("Ensemble check cancelled")),
        Some(Err(e)) => {
            let (title, why) = (gettext("No ensemble check"), crate::flows::weather::explain(&e));
            app.show_error(&title, &why);
            app.finished(&title, &why);
        }
    }
}

/// What the check of route `id` says, if it was made for the route as it is.
pub fn lines(app: &AppState, id: i64) -> Vec<String> {
    let s = app.session.borrow();
    let Some(r) = s.routes.iter().find(|r| r.id == id) else { return Vec::new() };
    let e = app.ensembles.borrow();
    let Some(c) = e.get(&id) else { return Vec::new() };
    let same = c.points.len() == r.points.len() && c.points.iter().zip(&r.points).all(|(a, b)| a.distance_nm(&b.pos) < 0.01);
    if same { describe(c) } else { vec![gettext("The route has changed since the ensemble check: check it again.")] }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live: the GEFS members over a short passage off Brittany, and the spread of its arrival. Run with
    /// `cargo test --release ensemble_live -- --ignored --nocapture` (about a minute under the throttle).
    #[test]
    #[ignore]
    fn ensemble_live() {
        let a = LatLon::new(47.6, -4.2);
        let b = a.destination(250.0, 30.0);
        let area = Area::new(46.0, -6.0, 49.0, -3.0);
        let now = Utc::now();
        let (run, members) = fetch_members(area, now, 12, &Arc::new(AtomicBool::new(false)), &|_| {}).expect("an ensemble run");
        assert_eq!(members.len(), 31);
        assert!(members.iter().all(|m| m.wind(a.lat, a.lon, now).is_some()), "every member has wind at the start");
        let bounds = crate::core::vector::Bounds { min_lon: -6.0, max_lon: -3.0, min_lat: 46.0, max_lat: 49.0 };
        let polar = crate::core::routing::polar::Polar::generic_cruiser();
        let s = crate::core::routing::isochrone::Settings::default();
        let sails: Vec<_> = members.iter().map(|m| crate::core::routing::along::sail(&[a, b], now, &polar, &Bounded { env: &StoreEnv(m), bounds }, &s)).collect();
        let spread = ensemble::spread(&sails);
        let c = Check { points: vec![a, b], depart: now, run, spread, limit_kn: 35.0 };
        for l in describe(&c) {
            println!("{l}");
        }
        assert_eq!(c.spread.members, 31);
        // Members differ: not every one the same speed.
        let winds: std::collections::BTreeSet<i64> = sails.iter().map(|s| (s.max_tws_kn * 10.0) as i64).collect();
        assert!(winds.len() > 3, "{winds:?}");
    }
}
