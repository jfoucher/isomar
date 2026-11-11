//! "Auto route": finds waypoints between the points of a route that keep clear of land and of
//! water shallower than the safety depth, and saves the result as a new route.

use crate::app::{AppState, Selection};
use crate::core::chart::emodnet::{self, TileKey};
use crate::core::chart::vtile::TileError;
use crate::core::geo::LatLon;
use crate::core::nav::model::{Route, RoutePoint};
use crate::core::routing::auto::{self, AutoError, AutoRoute, AutoSettings, Inputs, Report};
use crate::core::vector::Bounds;
use crate::i18n::{gettext, gettext_f};
use crate::jobs::JobHandle;
use crate::platform::bg;
use rayon::prelude::*;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Most depth tiles fetched without asking: about 0.45 MB each.
const ASK_ABOVE_TILES: usize = 40;

/// Area whose land and depth the router is given: the stops, and twice the
/// search width of its longest leg beyond them (the second try looks wider).
pub fn data_bounds(stops: &[LatLon], s: &AutoSettings) -> Bounds {
    let longest = stops.windows(2).map(|w| w[0].distance_nm(&w[1])).fold(0.0, f64::max);
    let pad = auto::pad_nm(longest, s) * 2.0;
    let widest = stops.iter().map(|p| p.lat.abs()).fold(0.0, f64::max).min(80.0).to_radians().cos();
    let mut b = Bounds { min_lon: f64::MAX, min_lat: f64::MAX, max_lon: f64::MIN, max_lat: f64::MIN };
    for p in stops {
        b.min_lon = b.min_lon.min(p.lon);
        b.max_lon = b.max_lon.max(p.lon);
        b.min_lat = b.min_lat.min(p.lat);
        b.max_lat = b.max_lat.max(p.lat);
    }
    Bounds { min_lon: b.min_lon - pad / 60.0 / widest, max_lon: b.max_lon + pad / 60.0 / widest, min_lat: (b.min_lat - pad / 60.0).max(-85.0), max_lat: (b.max_lat + pad / 60.0).min(85.0) }
}

pub fn explain(e: &AutoError) -> String {
    match e {
        AutoError::BlockedStart { leg } | AutoError::BlockedEnd { leg } => {
            let which = if matches!(e, AutoError::BlockedStart { .. }) { gettext("starts") } else { gettext("ends") };
            gettext_f(
                "Leg {n} {which} on land or in water shallower than the safety depth, with no safe water within half a mile. Move that point into the water, or lower the safety depth.",
                &[("n", &(leg + 1).to_string()), ("which", &which)],
            )
        }
        AutoError::NoPath { leg } => gettext_f(
            "There is no way through for leg {n} that keeps to the safety depth and clearance. Try a lower safety depth or clearance, or add depth data for the area.",
            &[("n", &(leg + 1).to_string())],
        ),
        other => other.to_string(),
    }
}

/// The line shown when the route is done.
pub fn summary(r: &Report, failed_tiles: usize) -> String {
    let mut parts = vec![gettext_f("{nm} nm (direct {direct})", &[("nm", &format!("{:.1}", r.distance_nm)), ("direct", &format!("{:.1}", r.direct_nm))])];
    if let Some(d) = r.min_depth_m {
        parts.push(gettext_f("shallowest {m} m", &[("m", &format!("{d:.0}"))]));
    }
    if r.unverified_nm >= 0.05 {
        parts.push(gettext_f("{nm} nm of unknown depth", &[("nm", &format!("{:.1}", r.unverified_nm))]));
    }
    if r.exempt_stubs > 0 {
        parts.push(gettext("starts or ends outside safe water"));
    }
    if failed_tiles > 0 {
        parts.push(gettext_f("{n} depth tiles could not be loaded", &[("n", &failed_tiles.to_string())]));
    }
    parts.join(", ")
}

/// Runs the search as a job with Cancel; `busy` is told when the work starts and ends, for a shell
/// that wants a spinner.
pub async fn compute(app: &Rc<AppState>, source: Route, settings: AutoSettings, download: bool, busy: &dyn Fn(bool)) {
    let stops: Vec<LatLon> = source.points.iter().map(|p| p.pos).collect();
    let b = data_bounds(&stops, &settings);

    // Depth: what the chart already holds, then the EMODnet tiles it does not. Asked about before
    // anything is fetched.
    let dir = crate::emodnet::dir();
    let wanted: Vec<TileKey> = {
        let depth = app.depth.borrow();
        emodnet::tiles_for(&b, 1000).into_iter().filter(|k| !depth.as_ref().is_some_and(|d| d.has_tile(k))).collect()
    };
    let missing = wanted.iter().filter(|k| !k.file(&dir).exists()).count();
    let download = download && missing > 0;
    if download && missing > ASK_ABOVE_TILES {
        let body = gettext_f("{n} depth tiles are missing for this route, roughly {mb} MB.", &[("n", &missing.to_string()), ("mb", &format!("{:.0}", missing as f64 * 0.45))]);
        let Some(shell) = app.shell() else { return };
        if !shell.confirm(&gettext("Download depth data?"), &body, &gettext("Download"), false).await {
            return;
        }
    }

    // One job from the land tiles to the search: a Cancel stops it at whatever stage it is.
    let job = app.jobs.start_cancellable(&format!("autoroute:{}", source.id), &gettext("Finding a safe route"));
    busy(true);
    let found = job.unless_cancelled(search(app, &job, stops, b, wanted, download, settings)).await;
    busy(false);
    drop(job);
    let Some((result, failed)) = found else {
        app.toast(&gettext("Safe route cancelled"));
        return;
    };

    let routed = match result {
        Ok(r) => r,
        Err(AutoError::Cancelled) => {
            app.toast(&gettext("Safe route cancelled"));
            return;
        }
        Err(e) => {
            app.show_error(&gettext("No route found"), &explain(&e));
            return;
        }
    };
    if routed.points.iter().all(|p| p.source.is_some()) {
        app.toast(&format!("{}: {}.", gettext("The route is already clear of land and shallows"), summary(&routed.report, failed)));
        return;
    }
    let mut route = Route::new(gettext_f("Auto: {name}", &[("name", &source.name)]));
    route.speed_kn = source.speed_kn;
    let mut added = 0;
    for p in &routed.points {
        match p.source {
            Some(i) => route.points.push(source.points[i].clone()),
            None => {
                added += 1;
                let name = p.mark.clone().unwrap_or_else(|| format!("A{added:02}"));
                route.points.push(RoutePoint::new(name, p.pos));
            }
        }
    }
    let saved = app.session.borrow_mut().save_route(&route);
    match saved {
        Ok(r) => {
            app.toast(&format!("{}: {}", r.name, summary(&routed.report, failed)));
            app.select(Selection::Route(r.id));
            app.refresh();
        }
        Err(e) => app.show_error(&gettext("Could not save the route"), &e.to_string()),
    }
}

/// The land, the depths and the search, off the main thread; stops when dropped. The result,
/// and how many depth tiles could not be loaded.
async fn search(app: &Rc<AppState>, job: &JobHandle, stops: Vec<LatLon>, b: Bounds, wanted: Vec<TileKey>, download: bool, settings: AutoSettings) -> (Result<AutoRoute, AutoError>, usize) {
    job.detail(&gettext("Getting the land"));
    // The finest local coast plus the downloaded land, as the weather route does.
    let mut land = app.land.borrow().as_ref().map(|l| l.polygons_in(&b)).unwrap_or_default();
    land.extend(crate::tiles::land_polygons(app, b).await.0);
    let mut grids: Vec<Arc<_>> = app.depth.borrow().as_ref().map(|d| d.grids_in(&b)).unwrap_or_default();
    job.detail(&if download { gettext("Downloading depths, then looking for the shortest safe way.") } else { gettext("Looking for the shortest safe way.") });

    let cancel = Arc::new(AtomicBool::new(false));
    let _stop = {
        let cancel = cancel.clone();
        crate::jobs::on_drop(move || cancel.store(true, Ordering::Relaxed))
    };
    let dir = crate::emodnet::dir();
    bg(move || {
        let loaded: Vec<Result<Option<_>, TileError>> = wanted.par_iter().map(|k| if cancel.load(Ordering::Relaxed) { Ok(None) } else { emodnet::load(&dir, k, download) }).collect();
        let mut failed = 0;
        for l in loaded {
            match l {
                Ok(Some(g)) => grids.push(Arc::new(g)),
                Ok(None) | Err(TileError::Offline) => {}
                Err(_) => failed += 1,
            }
        }
        (auto::plan_cancellable(&stops, &Inputs { land, grids }, &settings, &cancel), failed)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_covers_the_stops_with_room_to_go_round() {
        let s = AutoSettings::default();
        let stops = [LatLon::new(48.0, -5.0), LatLon::new(48.0, -4.0)];
        let b = data_bounds(&stops, &s);
        assert!(b.min_lon < -5.0 && b.max_lon > -4.0 && b.min_lat < 48.0 && b.max_lat > 48.0);
        // A leg of about 40 nm is searched 10 nm either side, and the box is twice that.
        let leg = stops[0].distance_nm(&stops[1]);
        let pad_lat = (48.0 - b.min_lat) * 60.0;
        assert!((pad_lat - auto::pad_nm(leg, &s) * 2.0).abs() < 0.01, "{pad_lat}");
        // A longer route has more room.
        let long = data_bounds(&[LatLon::new(48.0, -8.0), LatLon::new(48.0, -4.0)], &s);
        assert!(48.0 - long.min_lat > 48.0 - b.min_lat);
    }

    #[test]
    fn errors_explain_themselves() {
        assert!(explain(&AutoError::BlockedStart { leg: 0 }).contains("Leg 1 starts"));
        assert!(explain(&AutoError::BlockedEnd { leg: 2 }).contains("Leg 3 ends"));
        assert!(explain(&AutoError::NoPath { leg: 1 }).contains("leg 2"));
        assert!(explain(&AutoError::TooFewStops).contains("two points"));
    }

    #[test]
    fn the_summary_says_what_is_uncertain() {
        let r = Report { distance_nm: 12.34, direct_nm: 10.0, min_depth_m: Some(7.2), unverified_nm: 0.0, exempt_stubs: 0 };
        assert_eq!(summary(&r, 0), "12.3 nm (direct 10.0), shallowest 7 m");
        let r = Report { unverified_nm: 2.5, exempt_stubs: 1, min_depth_m: None, ..r };
        let text = summary(&r, 3);
        assert!(text.contains("2.5 nm of unknown depth") && text.contains("outside safe water") && text.contains("3 depth tiles"), "{text}");
        assert!(!text.contains("shallowest"));
    }
}
