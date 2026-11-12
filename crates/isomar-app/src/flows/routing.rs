//! Weather routing: the parameters a passage gets.

use crate::app::AppState;
use crate::core::geo::LatLon;
use crate::core::nav::calc::format_duration;
use crate::core::nav::model::{Route, RoutePoint};
use crate::core::routing::alternatives::Alternative;
use crate::core::routing::isochrone::{self, Bounded, Progress, RouteError, Settings, StoreEnv};
use crate::core::routing::land::{DetailedLand, Land, RasterLand};
use crate::core::routing::plan::{self, RoutePlan};
use crate::core::routing::polar::Polar;
use crate::core::routing::seaway;
pub use crate::core::routing::passage::{DETAIL_RES, OFFSHORE_NM, SEARCH_MARGINS_DEG, area_margin_deg, around, detail_bounds, is_detour, land_resolution, offshore, settings_for_distance, wider_margin};
use crate::core::weather::model::Group;
use crate::core::weather::planner::{self, Source};
use crate::core::vector::Bounds;
use crate::core::nav::model::Route as NavRoute;
use crate::jobs::JobHandle;
use crate::platform::bg;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use crate::i18n::{gettext, gettext_f};
use chrono::{Local, Utc};
use std::rc::Rc;

/// A model's forecast as the router reads it, inside an area: its own, and for a regional model its
/// parent's around it and after it (`routing::nested`); and the current every model shares, when routes
/// use currents and one covers the area.
#[derive(Default)]
pub struct ModelWeather {
    pub store: crate::core::weather::store::WeatherStore,
    pub parent: Option<crate::core::weather::store::WeatherStore>,
    pub currents: Option<Arc<crate::core::currents::CurrentField>>,
}

impl ModelWeather {
    pub fn of(w: &crate::weather::WeatherState, model: Source, b: &Bounds) -> Option<ModelWeather> {
        let store = w.store_of(model)?.within(b);
        let parent = model.parent().and_then(|p| w.store_of(p)).map(|st| st.within(b));
        Some(ModelWeather { store, parent, currents: w.currents.for_route(b) })
    }

    pub fn env(&self) -> Box<dyn isochrone::Environment + '_> {
        let wind: Box<dyn isochrone::Environment + '_> = match &self.parent {
            Some(p) => Box::new(crate::core::routing::nested::NestedEnv::new(&self.store, p)),
            None => Box::new(StoreEnv(&self.store)),
        };
        match &self.currents {
            Some(c) => Box::new(crate::core::currents::WithCurrents { wind, currents: c.as_ref() }),
            None => wind,
        }
    }
}

/// Whether route `id` was made by weather routing (it keeps a plan). Such a route is not routed
/// again: neither a weather route nor a best departure is offered on it.
pub fn is_weather_route(app: &AppState, id: i64) -> bool {
    app.session.borrow().plans.contains_key(&id)
}

/// The "Currents" switch of the routing panels: whether routes sail in the current, and what covers a
/// passage between `points` (or will, once fetched).
#[derive(serde::Serialize, Clone, Debug, PartialEq)]
pub struct CurrentsChoice {
    pub on: bool,
    /// What covers the passage, or why nothing does.
    pub detail: String,
}

pub fn currents_choice(app: &AppState, points: &[LatLon]) -> CurrentsChoice {
    let on = app.weather.borrow().currents.route_with;
    let b = around(points, 0.5);
    let covering = app.weather.borrow().currents.sources_text(&b);
    let fetch = app.settings.borrow().currents.fetch_copernicus;
    let mut detail = match (on, covering) {
        (false, _) => gettext("Off: the router ignores the stream."),
        (true, Some(s)) => gettext_f("From {sources}", &[("sources", &s)]),
        (true, None) if fetch => gettext("Copernicus Marine currents are fetched for the passage when it is routed."),
        (true, None) => gettext("No current is loaded for this passage."),
    };
    // A Shom atlas would resolve the races better than any forecast grid: say so, never fetch it unasked.
    if on && let Some(z) = crate::currents::atlases::missing_over(app, &b).first() {
        detail.push(' ');
        detail.push_str(&gettext_f("The Shom atlas for {zone} would resolve its tidal races ({mb} MB, on the Weather page).", &[("zone", &z.name), ("mb", &format!("{:.0}", (z.bytes as f64 / 1_048_576.0).max(1.0)))]));
    }
    CurrentsChoice { on, detail }
}

/// The current a search over `area` sails in, in words, when routes use currents and one covers it.
fn currents_text(app: &AppState, area: &Bounds) -> Option<String> {
    let w = app.weather.borrow();
    if !w.currents.route_with { None } else { w.currents.sources_text(area) }
}

/// Switches currents in weather routes on or off, for this and later routes.
pub fn set_route_with(app: &AppState, on: bool) {
    app.weather.borrow_mut().currents.route_with = on;
    let changed = app.settings.borrow().currents.route_with != on;
    if changed {
        app.settings.borrow_mut().currents.route_with = on;
        app.apply_settings();
    }
}

/// What a route saved from a passage routed on several models is sailed through once saved: each
/// model's forecast, for the spread of its arrival (`RoutePlan::checks`).
pub struct CheckWith {
    pub models: Vec<Source>,
    pub polar: Arc<Polar>,
    pub settings: Settings,
    pub area: Bounds,
}

/// The saved route `points` sailed through each of `c.models`' forecasts, leaving at `depart`.
pub fn check_route(app: &AppState, points: &[LatLon], depart: chrono::DateTime<Utc>, c: &CheckWith) -> Vec<crate::core::routing::plan::PlanCheck> {
    let w = app.weather.borrow();
    c.models
        .iter()
        .filter_map(|m| {
            let mw = ModelWeather::of(&w, *m, &c.area)?;
            let env = mw.env();
            let env = Bounded { env: env.as_ref(), bounds: c.area };
            Some(crate::core::routing::plan::PlanCheck::of(*m, &crate::core::routing::along::sail(points, depart, &c.polar, &env, &c.settings)))
        })
        .collect()
}

/// "between Sat 14:20 (GFS) and Sat 19:05 (ECMWF AIFS)": the spread of the arrivals a route was
/// checked against, when at least two forecasts get it there.
pub fn bracket_text(checks: &[crate::core::routing::plan::PlanCheck]) -> Option<String> {
    let arriving: Vec<_> = checks.iter().filter(|c| c.arrives).collect();
    let first = arriving.iter().min_by_key(|c| c.end)?;
    let last = arriving.iter().max_by_key(|c| c.end)?;
    if arriving.len() < 2 {
        return None;
    }
    let t = |c: &crate::core::routing::plan::PlanCheck| c.end.with_timezone(&Local).format("%a %H:%M").to_string();
    let mut text = gettext_f(
        "Over {n} forecasts it arrives between {from} ({a}) and {to} ({b}), {spread} apart",
        &[("n", &checks.len().to_string()), ("from", &t(first)), ("a", first.source.label()), ("to", &t(last)), ("b", last.source.label()), ("spread", &format_duration(last.end - first.end))],
    );
    let short: Vec<&str> = checks.iter().filter(|c| !c.arrives).map(|c| c.source.label()).collect();
    if !short.is_empty() {
        text.push_str("; ");
        text.push_str(&gettext_f("{models} end before the destination", &[("models", &short.join(", "))]));
    }
    Some(text)
}

/// Saves the chosen alternative as a new route. One that stops where the
/// forecast ends carries on to the destination in a straight line. With `check`, the saved route is
/// sailed through each of its models' forecasts, for the spread of its arrival.
pub fn use_alternative(app: &Rc<AppState>, source: &Route, depart: chrono::DateTime<Utc>, alt: &Alternative, dest: LatLon, land: Option<&dyn Land>, check: Option<CheckWith>) {
    let mut route = Route::new(gettext_f("Weather route: {name}", &[("name", &source.name)]));
    route.departure = Some(depart);
    let dur = alt.end - depart;
    let hours = dur.num_seconds() as f64 / 3600.0;
    route.speed_kn = if hours > 0.0 { (alt.distance_nm / hours).clamp(0.5, 40.0) } else { source.speed_kn };
    // Every tack and gybe, and the bends between them to within the user's tolerance of the path, so
    // the route lies on the one that was calculated, and off the land it kept clear of.
    let tolerance = app.settings.borrow().route_tolerance_nm.clamp(0.005, 5.0);
    let wps = plan::route_points(&alt.path, land, tolerance);
    isomar_core::log!("route: {} waypoints for {} nodes of the path", wps.len(), alt.path.len());
    let n = wps.len();
    for (i, p) in wps.iter().enumerate() {
        route.points.push(RoutePoint::new(format!("W{:02}", i + 1), *p));
    }
    if !alt.arrives {
        let name = source.points.last().map(|p| p.name.clone()).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("W{:02}", n + 1));
        route.points.push(RoutePoint::new(name, dest));
    }
    let saved = app.session.borrow_mut().save_route(&route);
    match saved {
        Ok(r) => {
            // The path and the forecast it was routed on stay with the route, for its
            // statistics and for playing it back.
            let model = app.router.borrow().as_ref().map_or_else(|| app.weather.borrow().source, |p| p.model);
            let mut plan = RoutePlan::new(depart, dest, alt.arrives, alt.path.clone(), r.points.iter().map(|p| p.pos).collect(), app.weather.borrow().store_of(model).unwrap_or(&Default::default()));
            plan.source = Some(model);
            if let Some(c) = &check {
                plan.checks = check_route(app, &r.points.iter().map(|p| p.pos).collect::<Vec<_>>(), depart, c);
            }
            if let Err(e) = app.session.borrow_mut().save_plan(r.id, &plan) {
                app.toast(&gettext_f("The route is saved, but not its statistics and forecast: {e}", &[("e", &e.to_string())]));
            }
            let mut msg = passage_text(&r.name, alt, depart);
            if alt.motor_hours > 0.1 {
                msg.push_str(&format!(", {}", gettext_f("{h} h under engine", &[("h", &format!("{:.0}", alt.motor_hours))])));
            }
            if let Some(b) = bracket_text(&plan.checks) {
                msg.push_str(". ");
                msg.push_str(&b);
            }
            app.toast(&msg);
            app.select(crate::app::Selection::Route(r.id));
            app.refresh();
            if let Some(shell) = app.shell() {
                shell.open(crate::shell::Dialog::RouteProperties(r.id));
            }
        }
        Err(e) => app.show_error(&gettext("Could not save the route"), &e.to_string()),
    }
}

/// How a way found goes, in a line: "{name}: 120 nm in 1 d 2 h, arriving Tue 14:00".
fn passage_text(name: &str, alt: &Alternative, depart: chrono::DateTime<Utc>) -> String {
    let nm = format!("{:.0}", alt.distance_nm);
    let time = alt.end.with_timezone(&Local).format("%a %H:%M").to_string();
    if alt.arrives {
        gettext_f("{name}: {nm} nm in {dur}, arriving {eta}", &[("name", name), ("nm", &nm), ("dur", &format_duration(alt.end - depart)), ("eta", &time)])
    } else {
        gettext_f("{name}: {nm} nm to where the forecast ends ({time}), then {left} nm direct", &[("name", name), ("nm", &nm), ("time", &time), ("left", &format!("{:.0}", alt.remaining_nm))])
    }
}

/// Tells the user the search for `preview` has ended, in case they looked away while it ran.
fn signal_found(app: &AppState, preview: &crate::router::RouterPreview) {
    let body = preview.alts.first().map(|c| passage_text(&preview.source.name, &c.alt, preview.ex.depart)).unwrap_or_default();
    app.finished(&gettext("Weather route ready"), &body);
}

/// A search that ended without a route: said as an error, and signalled as the end of the search.
pub fn failed(app: &AppState, e: &RouteError) {
    let (title, why) = (gettext("No route found"), explain(e));
    app.show_error(&title, &why);
    app.finished(&title, &why);
}

pub fn explain(e: &RouteError) -> String {
    match e {
        RouteError::NoWeather => gettext("There is no forecast at the start. The downloaded area may not reach it; get the weather for a larger area."),
        RouteError::NoProgress { hours } => gettext_f(
            "No way forward after {h} hours. It may be calm, too windy, blocked by land, or the forecast may end before the passage does. Try motoring, or load a longer forecast.",
            &[("h", &format!("{hours:.0}"))],
        ),
        other => other.to_string(),
    }
}

/// Why `depart` cannot be routed, given the span of the loaded wind forecast.
pub fn departure_problem(range: Option<(chrono::DateTime<Utc>, chrono::DateTime<Utc>)>, depart: chrono::DateTime<Utc>) -> Option<String> {
    let show = |t: chrono::DateTime<Utc>| t.with_timezone(&Local).format("%a %d %b %H:%M").to_string();
    match range {
        None => Some(gettext("There is no wind forecast. Check the network and try again.")),
        Some((lo, hi)) if depart < lo || depart > hi => Some(gettext_f(
            "The departure is outside the forecast, which runs from {from} to {to}. Choose a departure inside it.",
            &[("from", &show(lo)), ("to", &show(hi))],
        )),
        Some(_) => None,
    }
}


/// The forecast of a route is asked for in boxes on this lattice.
const FORECAST_LATTICE_DEG: f64 = 2.0;

/// How long a passage of `sea_nm` might take at worst, for how much forecast to fetch: three knots
/// made good, and a day besides.
pub fn passage_hours(sea_nm: f64) -> f64 {
    sea_nm / 3.0 + 24.0
}

/// Fetches `source`'s forecast for the box round `points` and `area` (and `extra` groups besides wind
/// and pressure), unless what is loaded already covers it, and shows it. The box is rounded outwards
/// to whole `FORECAST_LATTICE_DEG`, so a route nudged a little asks for the same forecast and finds it
/// loaded (or in the cache). Forecasts of other areas stay loaded beside the new one. GFS is fetched
/// to its end, as its downloads are small; a model downloaded whole-globe only as far as `until`.
/// False when the download was cancelled or failed.
pub async fn ensure_forecast(app: &Rc<AppState>, source: Source, points: &[LatLon], area: Bounds, extra: &[Group], until: chrono::DateTime<Utc>) -> bool {
    crate::flows::weather::show_source(app, source);
    let mut points = points.to_vec();
    points.extend([LatLon::new(area.min_lat, area.min_lon), LatLon::new(area.max_lat, area.max_lon)]);
    let Some(mut req) = planner::route_request(source, &points) else { return true };
    req.area = req.area.outward(FORECAST_LATTICE_DEG);
    // A regional model: its parent's forecast first, for around it and after it, then its own over the
    // part of the passage it covers.
    if let Some(parent) = source.parent() {
        if !Box::pin(ensure_forecast(app, parent, &points, area, extra, until)).await {
            return false;
        }
        crate::flows::weather::show_source(app, source);
        let Some(inside) = planner::within_domain(source, &req.area) else {
            app.toast(&gettext_f("{model} does not cover this passage", &[("model", source.label())]));
            return false;
        };
        req.area = inside;
    }
    req.groups.extend(extra.iter().filter(|g| !req.groups.contains(g)).copied().collect::<Vec<_>>());
    if !source.crops_on_server() {
        let hours = (until - Utc::now()).num_hours() + 12;
        req.horizon_hours = hours.clamp(24, source.max_hours() as i64) as u32;
        // Pressure is only for the isobars, and from a whole-globe or whole-Europe model it would add a
        // quarter to the download.
        req.groups.retain(|g| *g != Group::Pressure);
    }
    // Not inside the `if`: the borrow must end before the download takes its own.
    let covered = app.weather.borrow().store_of(source).is_some_and(|st| planner::covers(st, &req, Utc::now()));
    isomar_core::log!("route: the {} forecast over {:?} for {} h is {}", source.label(), req.area, req.horizon_hours, if covered { "loaded" } else { "to download" });
    covered || crate::flows::weather::run_download(app, req).await
}

/// What a search needs besides the departure, so the weather window can run
/// many with one land mask.
pub struct Search {
    /// The forecast model the search sails on.
    pub source: Source,
    pub start: LatLon,
    pub dest: LatLon,
    pub polar: Arc<Polar>,
    pub settings: Settings,
    pub area: Bounds,
    pub land: Arc<DetailedLand>,
}

/// The polygons of the local coast and the downloaded land under `b`.
async fn land_in(app: &Rc<AppState>, b: Bounds) -> Vec<crate::core::vector::Polygon> {
    let mut polygons = app.land.borrow().as_ref().map(|l| l.polygons_in(&b)).unwrap_or_default();
    polygons.extend(crate::tiles::land_polygons(app, b).await.0);
    polygons
}

/// The area the router searches, and the length of the passage by water:
/// round the way by water on a coarse mask, found in ever wider areas round
/// the ends. A box round the ends alone cuts off a passage that leaves it to
/// go round land (Napoli → Split goes south round Calabria, past the box's
/// southern edge). Where the way is a detour, the area holds every way up to
/// a quarter longer too, or the router could not take the other: Liverpool →
/// Hull round Scotland, with the wind behind it, was walled in by a box round
/// the way round Land's End. Also the shortest way itself. When no way is
/// found, the box round the ends, the direct distance and no way.
pub async fn passage_area(app: &Rc<AppState>, start: LatLon, dest: LatLon) -> (Bounds, f64, Vec<LatLon>) {
    let direct = start.distance_nm(&dest);
    for (i, margin) in SEARCH_MARGINS_DEG.into_iter().enumerate() {
        let wide = around(&[start, dest], margin);
        let polygons = land_in(app, wide).await;
        let path = bg(move || seaway::sea_path(wide, &polygons, start, dest, 1_000_000)).await;
        let Some(path) = path else { continue };
        let nm: f64 = path.windows(2).map(|w| w[0].distance_nm(&w[1])).sum();
        isomar_core::log!("route: way by water {nm:.0} nm ({direct:.0} direct), found within {margin} degrees of the ends");
        // What the area must hold: the way, and the corners of the box round the other ways too.
        let mut reach = path.clone();
        if is_detour(nm, direct)
            && let Some((from, to)) = offshore(&path)
        {
            let wider = around(&[start, dest], wider_margin(i));
            let polygons = land_in(app, wider).await;
            if let Some(w) = bg(move || seaway::sea_ways(wider, &polygons, from, to, 1_000_000)).await {
                isomar_core::log!("route: ways up to {:.0}% longer lie within {:?}", (seaway::ALTERNATIVE_RATIO - 1.0) * 100.0, w.corridor);
                reach.extend([LatLon::new(w.corridor.min_lat, w.corridor.min_lon), LatLon::new(w.corridor.max_lat, w.corridor.max_lon)]);
            }
        }
        let nm = nm.max(direct);
        return (around(&reach, area_margin_deg(nm)), nm, path);
    }
    isomar_core::log!("route: no way by water found within {} degrees of the ends", SEARCH_MARGINS_DEG[SEARCH_MARGINS_DEG.len() - 1]);
    (around(&[start, dest], area_margin_deg(direct)), direct, Vec::new())
}

/// Fine masks added where the router's mask cuts the way by water (a
/// strait its margin closes, like Messina at 0.01 degree), at most this many.
const MAX_CUTS: usize = 6;

/// The router's land mask for the passage, and how many land tiles could
/// not be had.
pub async fn prepare_land(app: &Rc<AppState>, start: LatLon, dest: LatLon, b: Bounds, sea_nm: f64, path: &[LatLon]) -> (DetailedLand, usize) {
    // The finest local coast, plus the downloaded land for what it does not
    // reach (a foreign shore inside a local store's bounding box). Both are
    // rasterised as a union, so overlapping them does no harm.
    let mut polygons = app.land.borrow().as_ref().map(|l| l.polygons_in(&b)).unwrap_or_default();
    let (tile_land, mut missing_tiles) = crate::tiles::land_polygons(app, b).await;
    polygons.extend(tile_land);
    let res = land_resolution(sea_nm);
    // The ends get a finer mask, from finer tiles.
    let mut detail = Vec::new();
    for p in [start, dest] {
        let db = detail_bounds(p);
        let mut polys = app.land.borrow().as_ref().map(|l| l.polygons_in(&db)).unwrap_or_default();
        let (tile_land, missing) = crate::tiles::detailed_land_polygons(app, db).await;
        polys.extend(tile_land);
        missing_tiles += missing;
        detail.push((db, polys));
    }
    let mut land = bg(move || {
        DetailedLand::new(RasterLand::from_polygons(b, res, &polygons, 1), detail.iter().map(|(db, polys)| RasterLand::from_polygons(*db, DETAIL_RES, polys, 1)).collect())
    })
    .await;
    // Where the way by water is cut off in the mask, a fine mask opens it.
    for _ in 0..MAX_CUTS {
        if path.len() < 2 {
            break;
        }
        let (l, p) = (land, path.to_vec());
        let (l, cut) = bg(move || {
            let cut = l.cut_area(&p);
            (l, cut)
        })
        .await;
        land = l;
        let Some(cb) = cut else { break };
        let mut polys = app.land.borrow().as_ref().map(|l| l.polygons_in(&cb)).unwrap_or_default();
        let (tile_land, missing) = crate::tiles::detailed_land_polygons(app, cb).await;
        polys.extend(tile_land);
        missing_tiles += missing;
        isomar_core::log!("route: a fine mask where the way by water is cut, {cb:?}");
        land = bg(move || {
            land.detail.push(RasterLand::from_polygons(cb, DETAIL_RES, &polys, 1));
            land
        })
        .await;
    }

    (land, missing_tiles)
}

static NEXT_PREVIEW: AtomicU64 = AtomicU64::new(1);

/// A number no earlier preview had, for the panel to tell previews apart.
pub fn next_preview_id() -> u64 {
    NEXT_PREVIEW.fetch_add(1, Ordering::Relaxed)
}

/// Searches from `search.start` leaving at `depart`, with `progress` for a shell to watch and cancel,
/// and previews what it found. Returns whether the statistics should open at once (`ISOMAR_ROUTER_STATS`).
pub async fn compute_with(app: &Rc<AppState>, source: NavRoute, search: &Search, depart: chrono::DateTime<Utc>, missing_tiles: usize, progress: Arc<Progress>) -> bool {
    let (start, dest, b) = (search.start, search.dest, search.area);
    let store = ModelWeather::of(&app.weather.borrow(), search.source, &b).unwrap_or_default();
    let (polar, land, settings) = (search.polar.clone(), search.land.clone(), search.settings.clone());
    let p2 = progress.clone();
    // The fronts show on the chart as the search grows them, over the whole passage.
    fit_passage(app, start, dest);
    *app.router_live.borrow_mut() = Some(progress.clone());
    let live = Arc::new(std::sync::atomic::AtomicBool::new(true));
    // However the search ends, its live fronts go; dropped half-way (a cancel), it stops too.
    let end = {
        let (app, live, progress) = (app.clone(), live.clone(), progress.clone());
        crate::jobs::on_drop(move || {
            progress.cancel.store(true, Ordering::Relaxed);
            live.store(false, Ordering::Relaxed);
            *app.router_live.borrow_mut() = None;
            app.canvas.queue_draw();
        })
    };
    {
        let (app, live) = (app.clone(), live.clone());
        crate::platform::spawn(async move {
            while live.load(Ordering::Relaxed) {
                app.canvas.queue_draw();
                let (tx, rx) = async_channel::bounded::<()>(1);
                crate::platform::timeout_once(std::time::Duration::from_millis(250), move || {
                    let _ = tx.try_send(());
                });
                let _ = rx.recv().await;
            }
        });
    }
    let result = bg(move || {
        // Past the land mask's bounds the search knows no land, so it has no
        // weather there either and stops at the edge of the box.
        let env = store.env();
        let env = Bounded { env: env.as_ref(), bounds: b };
        isochrone::explore(start, dest, depart, &polar, &env, land.as_ref(), &settings, Some(&p2))
    })
    .await;
    drop(end);
    match &result {
        Ok(ex) => isomar_core::log!("route: {} fronts, arrived {}, stalled {}, horizon {:?}", ex.fronts.len(), ex.arrived.is_some(), ex.stalled, ex.horizon),
        Err(e) => isomar_core::log!("route: search failed: {e}"),
    }
    match result {
        Ok(ex) => show_found(app, source, search, ex, missing_tiles, true),
        Err(RouteError::Cancelled) => {
            app.toast(&gettext("Route cancelled"));
            false
        }
        Err(e) => {
            failed(app, &e);
            false
        }
    }
}

/// Shows the whole passage on the chart.
fn fit_passage(app: &Rc<AppState>, start: LatLon, dest: LatLon) {
    let pad = |a: f64, b: f64| (a - b).abs() * 0.15 + 0.5;
    let (dlon, dlat) = (pad(start.lon, dest.lon), pad(start.lat, dest.lat));
    app.canvas.fit_bounds(&Bounds { min_lon: start.lon.min(dest.lon) - dlon, max_lon: start.lon.max(dest.lon) + dlon, min_lat: start.lat.min(dest.lat) - dlat, max_lat: start.lat.max(dest.lat) + dlat });
}

/// Previews what a search found, with the end-of-search signal when it was searched just now
/// (`signal`). Returns whether the statistics should open at once.
fn show_found(app: &Rc<AppState>, source: NavRoute, search: &Search, ex: isochrone::Exploration, missing_tiles: usize, signal: bool) -> bool {
    let id = next_preview_id();
    // The wind shown is the wind it sailed on.
    crate::flows::weather::show_source(app, search.source);
    let weather_time = app.weather.borrow().time;
    let mut preview = crate::router::RouterPreview::new(id, source, ex, weather_time);
    preview.land = Some(search.land.clone());
    preview.model = search.source;
    preview.currents = currents_text(app, &search.area);
    if signal {
        signal_found(app, &preview);
    }
    let stats = crate::router::show(app, preview);
    if missing_tiles > 0 {
        app.toast(&crate::tiles::missing_land_message(missing_tiles));
    }
    stats
}

/// Previews a search done before (one the weather window kept), as `compute_with` does once it has searched.
pub fn show_search(app: &Rc<AppState>, source: NavRoute, search: &Search, ex: isochrone::Exploration, missing_tiles: usize) -> bool {
    fit_passage(app, search.start, search.dest);
    show_found(app, source, search, ex, missing_tiles, false)
}

/// What the user chose for a weather route besides the route itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoatParams {
    /// Percent of the polar the boat achieves.
    pub performance_pct: f64,
    pub use_engine: bool,
    pub motor_kn: f64,
    /// The boat does not sail in more wind than this.
    pub max_wind_kn: f64,
}

impl Default for BoatParams {
    fn default() -> Self {
        BoatParams { performance_pct: 100.0, use_engine: false, motor_kn: 5.0, max_wind_kn: 35.0 }
    }
}

/// The performance weather routing starts from: what the user took from the performance log, else 100%.
pub fn default_performance(app: &AppState) -> f64 {
    crate::polars::performance_pct(app).unwrap_or(100.0)
}

impl BoatParams {
    pub fn apply(&self, s: &mut Settings) {
        s.performance = self.performance_pct / 100.0;
        s.max_wind_kn = self.max_wind_kn;
        s.motor_speed_kn = self.use_engine.then_some(self.motor_kn);
    }
}

/// Everything a weather route needs from the user.
#[derive(Clone, Debug)]
pub struct RouteRequest {
    pub route_id: i64,
    pub depart: chrono::DateTime<Utc>,
    pub boat: BoatParams,
    /// The forecast models to route on: one search each, compared when there are several.
    pub sources: Vec<Source>,
}

/// `sources` in `Source::ALL`'s order without repeats, or the model shown when there are none.
pub fn models_of(app: &AppState, sources: &[Source]) -> Vec<Source> {
    let v: Vec<Source> = Source::ALL.into_iter().filter(|s| sources.contains(s)).collect();
    if v.is_empty() { vec![app.weather.borrow().source] } else { v }
}

/// Fetches each model's forecast for the passage, and keeps the models whose forecast came and
/// covers `depart`. When none does, the reason is told and the list is empty.
pub async fn forecasts_for(app: &Rc<AppState>, job: &JobHandle, sources: &[Source], points: &[LatLon], area: Bounds, extra: &[Group], until: chrono::DateTime<Utc>, depart: chrono::DateTime<Utc>, title: &str) -> Vec<Source> {
    let mut ok = Vec::new();
    let mut problem = None;
    for (i, src) in sources.iter().enumerate() {
        let detail = if sources.len() > 1 {
            gettext_f("Getting the {model} forecast ({i} of {n})", &[("model", src.label()), ("i", &(i + 1).to_string()), ("n", &sources.len().to_string())])
        } else {
            gettext_f("Getting the {model} forecast", &[("model", src.label())])
        };
        job.detail(&detail);
        let extra: Vec<Group> = extra.iter().copied().filter(|g| src.has(*g)).collect();
        if !ensure_forecast(app, *src, points, area, &extra, until).await {
            continue;
        }
        let range = app.weather.borrow().store_of(*src).and_then(|st| st.wind_range_in(Some(&area)));
        match departure_problem(range, depart) {
            Some(p) => {
                isomar_core::log!("route: {} cannot be used: {p}", src.label());
                problem.get_or_insert(p);
            }
            None => ok.push(*src),
        }
    }
    if ok.is_empty() {
        if let Some(p) = problem {
            app.show_error(title, &p);
        }
    } else if ok.len() < sources.len() {
        let left: Vec<&str> = sources.iter().filter(|s| !ok.contains(s)).map(|s| s.label()).collect();
        app.toast(&gettext_f("Left out: {models}, whose forecast does not cover the departure", &[("models", &left.join(", "))]));
    }
    ok
}

/// The whole flow after the user has chosen: find the area, get the forecast, build the land, run the
/// search, and preview; all of it one job, whose Cancel stops it at whatever stage it is. Failures
/// are told to the shell.
pub async fn run(app: &Rc<AppState>, req: RouteRequest) {
    let job = Rc::new(app.jobs.start_cancellable("route", &gettext("Computing the route")));
    if job.unless_cancelled(route(app, req, &job)).await.is_none() {
        app.toast(&gettext("Route cancelled"));
    }
}

/// How often a job's progress is read off its worker.
pub const PROGRESS_EVERY: std::time::Duration = std::time::Duration::from_millis(200);

async fn route(app: &Rc<AppState>, req: RouteRequest, job: &Rc<JobHandle>) {
    let _busy = app.routing_guard();
    let Some(r) = app.session.borrow().routes.iter().find(|r| r.id == req.route_id).cloned() else { return };
    if r.points.len() < 2 {
        return;
    }
    let (start, dest) = (r.points[0].pos, r.points[r.points.len() - 1].pos);
    job.detail(&gettext("Finding the way by water"));
    let (area, sea_nm, path) = passage_area(app, start, dest).await;
    let points: Vec<LatLon> = r.points.iter().map(|p| p.pos).collect();
    let until = req.depart + chrono::Duration::minutes((passage_hours(sea_nm) * 60.0) as i64);
    let models = forecasts_for(app, job, &models_of(app, &req.sources), &points, area, &[], until, req.depart, &gettext("No route found")).await;
    if models.is_empty() {
        return;
    }
    crate::flows::currents::ensure(app, job, area, start, dest, until).await;
    // The active polar of the library.
    let polar = Arc::new(crate::polars::active(app));
    let mut settings = settings_for_distance(sea_nm);
    crate::flows::currents::tide_settings(app, &mut settings, sea_nm, &area, &path, req.depart, 13.0);
    req.boat.apply(&mut settings);
    job.detail(&gettext("Preparing the land"));
    let (land, missing_tiles) = prepare_land(app, start, dest, area, sea_nm, &path).await;
    let search = Search { source: models[0], start, dest, polar, settings, area, land: Arc::new(land) };

    let progress = Arc::new(Progress::default());
    follow_search(job, progress.clone());
    if models.len() == 1 {
        compute_with(app, r, &search, req.depart, missing_tiles, progress).await;
    } else {
        job.detail(&gettext_f("Routing on {n} forecasts", &[("n", &models.len().to_string())]));
        compute_models(app, r, &search, &models, req.depart, missing_tiles, progress).await;
    }
}

/// Searches the passage once for each of `models`, sails each one's best way through every model's
/// forecast (`compare::CrossCheck`), and previews them together, the first model's shown. Returns
/// whether the statistics should open at once.
pub async fn compute_models(app: &Rc<AppState>, route: NavRoute, search: &Search, models: &[Source], depart: chrono::DateTime<Utc>, missing_tiles: usize, progress: Arc<Progress>) -> bool {
    use crate::core::routing::compare::{self, CrossCheck};
    let (start, dest, b) = (search.start, search.dest, search.area);
    let stores: Vec<(Source, ModelWeather)> = {
        let w = app.weather.borrow();
        models.iter().filter_map(|m| Some((*m, ModelWeather::of(&w, *m, &b)?))).collect()
    };
    let (polar, land, settings) = (search.polar.clone(), search.land.clone(), search.settings.clone());
    fit_passage(app, start, dest);
    *app.router_live.borrow_mut() = Some(progress.clone());
    let end = {
        let (app, progress) = (app.clone(), progress.clone());
        crate::jobs::on_drop(move || {
            progress.cancel.store(true, Ordering::Relaxed);
            *app.router_live.borrow_mut() = None;
            app.canvas.queue_draw();
        })
    };
    let (p2, polar2, settings2) = (progress.clone(), polar.clone(), settings.clone());
    let found = bg(move || {
        let mut runs = Vec::new();
        for (m, store) in &stores {
            let env = store.env();
            let env = Bounded { env: env.as_ref(), bounds: b };
            match isochrone::explore(start, dest, depart, &polar2, &env, land.as_ref(), &settings2, Some(&p2)) {
                Ok(ex) => runs.push((*m, ex)),
                Err(RouteError::Cancelled) => return Err(RouteError::Cancelled),
                Err(e) => {
                    isomar_core::log!("route: {} found no route: {e}", m.label());
                    if stores.len() == 1 {
                        return Err(e);
                    }
                }
            }
        }
        if runs.is_empty() {
            return Err(RouteError::NoWeather);
        }
        let bests: Vec<Alternative> = runs.iter().map(|(_, ex)| crate::core::routing::alternatives::best(ex).into_iter().next().unwrap_or_else(|| Alternative::new(vec![], dest, false))).collect();
        let ways: Vec<Vec<LatLon>> = bests.iter().map(|a| a.path.iter().map(|n| n.pos).chain((!a.arrives).then_some(dest)).collect()).collect();
        let owned: Vec<Box<dyn isochrone::Environment + '_>> = runs.iter().filter_map(|(m, _)| stores.iter().find(|(s, _)| s == m)).map(|(_, st)| st.env()).collect();
        let bounded: Vec<Bounded<dyn isochrone::Environment>> = owned.iter().map(|e| Bounded { env: e.as_ref(), bounds: b }).collect();
        let refs: Vec<&dyn isochrone::Environment> = bounded.iter().map(|e| e as &dyn isochrone::Environment).collect();
        let cross = CrossCheck::run(&ways, &refs, depart, &polar2, &settings2);
        let agree = compare::agree(&ways, start.distance_nm(&dest), 3.0);
        Ok((runs, bests, cross, agree))
    })
    .await;
    drop(end);
    match found {
        Ok((runs, bests, cross, agree)) => {
            isomar_core::log!("route: {} models, agree {agree}, most robust {:?}", runs.len(), cross.most_robust());
            let id = next_preview_id();
            let mut runs = runs.into_iter().zip(bests);
            let ((first, ex), _) = runs.next().expect("one model at least");
            crate::flows::weather::show_source(app, first);
            let weather_time = app.weather.borrow().time;
            let mut preview = crate::router::RouterPreview::new(id, route, ex, weather_time);
            preview.land = Some(search.land.clone());
            preview.model = first;
            preview.currents = currents_text(app, &search.area);
            let first_best = preview.alts.first().map(|c| c.alt.clone()).unwrap_or_else(|| Alternative::new(vec![], dest, false));
            preview.runs.push(crate::router::ModelRun { model: first, ex: None, best: first_best });
            for ((m, ex), best) in runs {
                preview.runs.push(crate::router::ModelRun { model: m, ex: Some(ex), best });
            }
            // Played to the last of them.
            preview.end = preview.runs.iter().map(|r| r.best.end).fold(preview.end, chrono::DateTime::max);
            preview.t = preview.end;
            preview.comparison = Some(crate::router::Comparison { polar, settings, area: b, cross, agree });
            signal_found(app, &preview);
            let stats = crate::router::show(app, preview);
            if missing_tiles > 0 {
                app.toast(&crate::tiles::missing_land_message(missing_tiles));
            }
            stats
        }
        Err(RouteError::Cancelled) => {
            app.toast(&gettext("Route cancelled"));
            false
        }
        Err(e) => {
            failed(app, &e);
            false
        }
    }
}

/// Shows a search's steps as the job's progress.
pub fn follow_search(job: &Rc<JobHandle>, progress: Arc<Progress>) {
    job.every(PROGRESS_EVERY, move |job| {
        let (step, steps) = (progress.step.load(Ordering::Relaxed), progress.steps.load(Ordering::Relaxed));
        if steps > 0 {
            job.progress((step as f64 / steps as f64).min(1.0), &format!("{step} / {steps}"));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    #[test]
    fn departures_outside_the_forecast_are_explained() {
        let lo = Utc.with_ymd_and_hms(2026, 10, 26, 12, 0, 0).unwrap();
        let hi = lo + Duration::hours(384);
        let r = Some((lo, hi));
        assert!(departure_problem(r, lo + Duration::hours(5)).is_none());
        assert!(departure_problem(r, lo).is_none() && departure_problem(r, hi).is_none());
        assert!(departure_problem(r, lo - Duration::hours(1)).unwrap().contains("outside the forecast"));
        assert!(departure_problem(r, hi + Duration::hours(1)).unwrap().contains("outside the forecast"));
        assert!(departure_problem(None, lo).unwrap().contains("no wind forecast"));
    }

    #[test]
    fn errors_explain_themselves() {
        assert!(explain(&RouteError::NoWeather).contains("forecast"));
        assert!(explain(&RouteError::NoProgress { hours: 12.0 }).contains("12"));
        assert!(explain(&RouteError::BlockedStart).contains("land"));
    }
}
