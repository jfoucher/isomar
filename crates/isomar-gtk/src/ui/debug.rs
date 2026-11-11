//! Development aids driven by environment variables, used by
//! `scripts/screenshot.sh` to check the UI headlessly.
//!
//! - `ISOMAR_SCREENSHOT=<png>`: render the window to a PNG and quit.
//! - `ISOMAR_SCREENSHOT_DELAY=<ms>`: wait before capturing (default 2000).
//! - `ISOMAR_DEBUG_VIEW=<lat>,<lon>,<zoom>`: start with this view.
//! - `ISOMAR_POINTER=<x>,<y>|centre`: put the pointer there (window pixels in the chart) just before the screenshot, so the
//!   outline of what it is over shows; `ISOMAR_CLICK=<x>,<y>` clicks there instead, which opens the popover and
//!   pins the outline.
//! - `ISOMAR_KEEP_OPEN`: don't quit after the screenshot.
//! - `ISOMAR_DEBUG_TIMING`: print each frame's time, and log each tile drawn with
//!   the layers that took over 2 ms, and each preparation of a batch of tiles.
//! - `ISOMAR_TILE_THREADS=<n>`: draw the chart's tiles on `n` threads (0: on the main thread).
//! - `ISOMAR_BENCH=pan|zoom`: once the screenshot delay is over, pan (or zoom in
//!   and out) 60 times at 60 Hz (`ISOMAR_BENCH_STEPS` times), log the frame times
//!   and the CPU time of each kind of thread, then take the screenshot. `slowzoom`
//!   zooms in and out by small steps at uneven intervals, with pauses, as a hand on a
//!   touchpad does; with `ISOMAR_DEBUG_TIMING` the log says when the chart shows
//!   tiles of another zoom (`plane 0 shows <level> at <view's level>`).
//! - `ISOMAR_DEBUG_COVERAGE`: outline where each ENC cell has data (red), its
//!   holes (orange) and its bounding box (purple).
//! - `ISOMAR_HIDE=<id>,<id>`: hide layers (`towns` hides the town names and outlines).
//! - `ISOMAR_SHOW=<id>,<id>`: show layers that start off, such as `online-depth`.
//! - `ISOMAR_DEMO`: a simulated boat, a route and a few waypoints, only in a demo area near the view
//!   (`isomar_app::debug::DEMO_AREAS`: Quiberon, the Iroise), never round the view centre. Its traffic
//!   passes clear: `ISOMAR_DEMO_COLLISION` puts the ferry on a crossing course, for screenshots of the alert only.
//! - `ISOMAR_DEMO_BOAT=harbour|sea|off`: the demo boat moored in the area's harbour (no traffic), under way
//!   on its route, or none; by default under way with `ISOMAR_UNDERWAY` and moored otherwise. Screenshots
//!   that are not under way use `off`.
//! - `ISOMAR_DEMO_WEATHER`: download a day of weather around the view.
//! - `ISOMAR_DEMO_CURRENTS`: a made-up current round the Iroise (the Raz de Sein, the Fromveur) for two days.
//! - `ISOMAR_SIDEBAR=<page>`: open the sidebar on a page.
//! - `ISOMAR_OFFLINE`: never download map tiles; cached ones are still used.
//! - `ISOMAR_DEMO_PERF=<hours>`: fills an empty performance log with that many hours of a synthetic boat.
//! - `ISOMAR_DIALOG=charts|tides|polar|performance|preferences|connections|monitor|get-weather|about|auto-route|route|waypoint`:
//!   open that dialog (or page) at the view centre, or for the first route or mark.
//!   `weather-route` saves a 450 nm route towards the south-west, leaving in 72 hours, and routes it
//!   once the demo weather has arrived (use with `ISOMAR_DEMO_WEATHER` and
//!   `ISOMAR_AUTO_ACCEPT`). `weather-window` saves a 120 nm route the same way and opens "Best
//!   departure…" on it. `ISOMAR_ROUTE=<lat>,<lon>,<lat>,<lon>` gives either its ends
//!   instead, and `ISOMAR_ROUTE_LEAVE_H=<hours>` when it leaves.
//! - `ISOMAR_ROUTER_USE=1|replay`: the preview saves the route that arrived at once; `replay` then plays it back.
//! - `ISOMAR_ROUTER_T=<0..1>`: where the weather-route preview's clock starts.
//! - `ISOMAR_ROUTER_PICKS=<n>`: the preview starts with `n` alternatives picked, evenly along the front.

pub use isomar_app::debug::*;

use super::app::AppState;
use crate::core::nav::model::{Route, RoutePoint};
use std::rc::Rc;
use adw::prelude::*;
use gtk::glib;
use std::time::Duration;

/// Renders `win` to the PNG named by `ISOMAR_SCREENSHOT`, then quits. With
/// `ISOMAR_BENCH`, the benchmark runs first.
pub fn screenshot_if_requested(app: &Rc<AppState>, win: &adw::ApplicationWindow) {
    let Ok(path) = std::env::var("ISOMAR_SCREENSHOT") else { return };
    let delay = std::env::var("ISOMAR_SCREENSHOT_DELAY")
        .ok()
        .and_then(|d| d.parse().ok())
        .unwrap_or(2000);
    let (app, win) = (app.clone(), win.clone());
    glib::timeout_add_local_once(Duration::from_millis(delay), move || {
        let canvas = app.canvas.clone();
        let started = std::time::Instant::now();
        // The chart's tiles are drawn off the main thread: wait for the ones in view.
        glib::timeout_add_local(Duration::from_millis(50), move || {
            if !settled(&canvas) && started.elapsed() < Duration::from_secs(60) {
                return glib::ControlFlow::Continue;
            }
            let (app, win, path) = (app.clone(), win.clone(), path.clone());
            match std::env::var("ISOMAR_BENCH") {
                Ok(kind) => bench(&app, &kind, move || capture(win, path)),
                Err(_) => {
                    let wait = aim(&app);
                    glib::timeout_add_local_once(wait, move || capture(win, path));
                }
            }
            glib::ControlFlow::Break
        });
    });
}

/// Puts the pointer where `ISOMAR_POINTER` says, or clicks where `ISOMAR_CLICK` says, and returns how long to wait
/// for the outline to be drawn.
fn aim(app: &Rc<AppState>) -> Duration {
    let at = |name: &str| {
        let v = std::env::var(name).ok()?;
        if v == "centre" {
            let (w, h) = app.canvas.size();
            return Some((w / 2.0, h / 2.0));
        }
        let (x, y) = v.split_once(',')?;
        Some((x.trim().parse::<f64>().ok()?, y.trim().parse::<f64>().ok()?))
    };
    if let Some((x, y)) = at("ISOMAR_CLICK") {
        app.canvas.click(1, 1, x, y);
    } else if let Some((x, y)) = at("ISOMAR_POINTER") {
        app.canvas.pointer_moved(Some((x, y)));
    } else {
        return Duration::ZERO;
    }
    Duration::from_millis(400)
}

/// Whether every tile in view is drawn and none is queued.
/// CPU time each thread of this process has used, in seconds, by thread name with its digits left out
/// (`isomar-tile-` for all the tile workers). Linux only; elsewhere empty.
fn cpu_by_thread() -> std::collections::BTreeMap<String, f64> {
    let mut out = std::collections::BTreeMap::new();
    let Ok(tasks) = std::fs::read_dir("/proc/self/task") else { return out };
    for task in tasks.flatten() {
        let Ok(stat) = std::fs::read_to_string(task.path().join("stat")) else { continue };
        // "pid (comm) state ... utime stime ...": comm may hold spaces, so split after its ')'.
        let (Some(open), Some(close)) = (stat.find('('), stat.rfind(')')) else { continue };
        let name: String = stat[open + 1..close].chars().filter(|c| !c.is_ascii_digit()).collect();
        let fields: Vec<&str> = stat[close + 2..].split(' ').collect();
        let ticks = |i: usize| fields.get(i).and_then(|f| f.parse::<f64>().ok()).unwrap_or(0.0);
        // utime and stime are the 14th and 15th fields; `fields` starts at the 3rd.
        *out.entry(name).or_insert(0.0) += (ticks(11) + ticks(12)) / 100.0;
    }
    out
}

/// What `cpu_by_thread` grew by since `before`, the biggest first, as "name 1.23 s".
fn cpu_since(before: &std::collections::BTreeMap<String, f64>) -> String {
    let now = cpu_by_thread();
    let mut used: Vec<(String, f64)> = now.iter().map(|(k, v)| (k.clone(), v - before.get(k).copied().unwrap_or(0.0))).filter(|(_, d)| *d > 0.0).collect();
    used.sort_by(|a, b| b.1.total_cmp(&a.1));
    let total: f64 = used.iter().map(|(_, d)| d).sum();
    let parts: Vec<String> = used.iter().map(|(k, d)| format!("{k} {d:.2}")).collect();
    format!("cpu {total:.2} s [{}]", parts.join(", "))
}

fn settled(canvas: &isomar_app::engine::Engine) -> bool {
    let (ready, shown) = canvas.tiles_ready();
    canvas.tiles_pending() == 0 && ready == shown
}

/// Zooms in slowly by small steps at uneven intervals, now and then pausing long enough for the view's
/// own zoom to be drawn, as a hand on a touchpad does, then out the same way: with `ISOMAR_DEBUG_TIMING`,
/// the log says each time the chart shows tiles of another zoom.
fn slow_zoom(app: &Rc<AppState>, then: impl FnOnce() + 'static) {
    // (zoom step, milliseconds to the next), in for 2.4 zooms, then out again.
    let mut plan: Vec<(f64, u64)> = Vec::new();
    for dir in [1.0, -1.0] {
        for i in 0..120 {
            let wait = if i % 23 == 22 { 400 } else if i % 7 == 6 { 140 } else { 30 + (i * 37 % 60) as u64 };
            plan.push((dir * 0.02, wait));
        }
    }
    fn step(canvas: isomar_app::engine::Engine, mut plan: std::vec::IntoIter<(f64, u64)>, then: Box<dyn FnOnce()>) {
        let Some((dz, wait)) = plan.next() else {
            glib::timeout_add_local_once(Duration::from_secs(2), then);
            return;
        };
        let mut v = crate::ui::view::View::from_state(canvas.view_state());
        let vi = canvas.view_info();
        v.zoom_at(v.zoom + dz, vi.w / 2.0, vi.h / 2.0, vi.w, vi.h);
        canvas.set_view(v.state());
        glib::timeout_add_local_once(Duration::from_millis(wait), move || step(canvas, plan, then));
    }
    step(app.canvas.clone(), plan.into_iter(), Box::new(then));
}

/// Moves the view 60 times, a frame apart, and logs how long frames took, and how long the chart then took
/// to settle: every tile in view drawn.
fn bench(app: &Rc<AppState>, kind: &str, then: impl FnOnce() + 'static) {
    let steps: u32 = std::env::var("ISOMAR_BENCH_STEPS").ok().and_then(|s| s.parse().ok()).unwrap_or(60);
    let zoom = match kind {
        "pan" => false,
        "zoom" => true,
        "slowzoom" => return slow_zoom(app, then),
        other => {
            eprintln!("isomar: unknown ISOMAR_BENCH {other}, use pan, zoom or slowzoom");
            return then();
        }
    };
    let canvas = app.canvas.clone();
    let _ = canvas.record_frames(true);
    let cpu_before = cpu_by_thread();
    let started = std::time::Instant::now();
    let step = std::cell::Cell::new(0u32);
    let then = std::cell::RefCell::new(Some(then));
    let settle_from: std::cell::Cell<Option<std::time::Instant>> = std::cell::Cell::new(None);
    glib::timeout_add_local(Duration::from_millis(16), move || {
        let n = step.get();
        step.set(n + 1);
        if n < steps {
            let mut v = crate::ui::view::View::from_state(canvas.view_state());
            if zoom {
                // In for half the steps, then out again.
                let dz = if n < steps / 2 { 0.05 } else { -0.05 };
                let vi = canvas.view_info();
                v.zoom_at(v.zoom + dz, vi.w / 2.0, vi.h / 2.0, vi.w, vi.h);
            } else {
                let (x, y) = v.center_px();
                v.set_center_px(x + 20.0, y + 5.0);
            }
            canvas.set_view(v.state());
            return glib::ControlFlow::Continue;
        }
        // The tiles that follow the gesture.
        let settle_from = settle_from.get().unwrap_or_else(|| {
            settle_from.set(Some(std::time::Instant::now()));
            std::time::Instant::now()
        });
        if !(settled(&canvas) && settle_from.elapsed() > Duration::from_millis(50)) && settle_from.elapsed() < Duration::from_secs(60) {
            return glib::ControlFlow::Continue;
        }
        let settle_ms = settle_from.elapsed().as_secs_f64() * 1000.0 - 50.0;
        let frames = canvas.record_frames(false);
        let mut ms: Vec<f64> = frames.iter().map(|(t, _)| t.as_secs_f64() * 1000.0).collect();
        ms.sort_by(f64::total_cmp);
        let rebuilds = frames.iter().filter(|(_, r)| *r).count();
        let at = |q: f64| ms.get(((ms.len() as f64 - 1.0) * q).round() as usize).copied().unwrap_or(0.0);
        isomar_core::log!(
            "isomar: bench {} over {:.1} s: {} frames ({} planned tiles), min {:.1} ms, median {:.1} ms, p90 {:.1} ms, max {:.1} ms, settled {:.0} ms after the last move, {}",
            if zoom { "zoom" } else { "pan" },
            started.elapsed().as_secs_f64(),
            ms.len(),
            rebuilds,
            at(0.0),
            at(0.5),
            at(0.9),
            at(1.0),
            settle_ms.max(0.0),
            cpu_since(&cpu_before),
        );
        if let Some(f) = then.borrow_mut().take() {
            f();
        }
        glib::ControlFlow::Break
    });
}

/// Renders `win` (or a window opened over it) to `path`, then quits.
fn capture(win: adw::ApplicationWindow, path: String) {
    let tries = std::cell::Cell::new(0);
    glib::timeout_add_local(Duration::from_millis(100), move || {
        // A window opened on top of the main one (the polar editor) is what is wanted.
        let shown: gtk::Window = gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Window>().ok())
            .rfind(|w| w.is_visible() && w.upcast_ref::<gtk::Window>() != win.upcast_ref::<gtk::Window>() && w.is::<adw::Window>())
            .unwrap_or_else(|| win.clone().upcast());
        let paintable = gtk::WidgetPaintable::new(Some(&shown));
        let (w, h) = (shown.width() as f64, shown.height() as f64);
        let snap = gtk::Snapshot::new();
        paintable.snapshot(&snap, w, h);
        match (snap.to_node(), shown.renderer()) {
            (Some(node), Some(renderer)) => {
                let tex = renderer.render_texture(&node, None);
                if let Err(e) = tex.save_to_png(&path) {
                    eprintln!("isomar: screenshot failed: {e}");
                }
            }
            _ if tries.get() < 20 => {
                tries.set(tries.get() + 1);
                win.queue_draw();
                return glib::ControlFlow::Continue;
            }
            _ => eprintln!("isomar: nothing to render"),
        }
        if std::env::var_os("ISOMAR_KEEP_OPEN").is_none()
            && let Some(app) = win.application()
        {
            app.quit();
        }
        glib::ControlFlow::Break
    });
}


/// With `ISOMAR_DEMO_WEATHER` set, downloads a day of weather around the view
/// through the normal download path.
pub fn seed_weather(app: &Rc<AppState>) {
    use crate::core::weather::model::Group;
    use crate::core::weather::planner::{Area, Request};
    if std::env::var_os("ISOMAR_DEMO_WEATHER").is_none() {
        return;
    }
    let c = app.canvas.view_state().center();
    // `ISOMAR_DEMO_WEATHER=ifs` (or `aifs`) asks that model instead of the one chosen last.
    let source = std::env::var("ISOMAR_DEMO_WEATHER").ok().and_then(|m| crate::core::weather::source::Source::from_id(&m)).unwrap_or_else(|| app.weather.borrow().source);
    let req = Request {
        source,
        area: Area::around(c.lat, c.lon, 9.0),
        groups: vec![Group::Wind, Group::Gust, Group::Pressure, Group::Waves],
        horizon_hours: 24,
        step_hours: 3,
    };
    let app = app.clone();
    super::spawn(async move {
        isomar_app::flows::weather::run_download(&app, req).await;
    });
}

/// With `ISOMAR_DIALOG` set, opens that dialog at the view centre.
pub fn open_dialog(app: &Rc<AppState>) {
    let Ok(which) = std::env::var("ISOMAR_DIALOG") else { return };
    let c = app.canvas.view_state().center();
    match which.as_str() {
        "sources" | "charts" => {
            if let Some(s) = app.shell() {
                s.open(isomar_app::shell::Dialog::ChartsHere(c));
            }
        }
        "tides" => {
            if let Some(s) = app.shell() {
                s.open(isomar_app::shell::Dialog::Tides(c));
            }
        }
        "preferences" | "connections" | "monitor" | "get-weather" | "about" | "show-help-overlay" => {
            let _ = super::chart_widget().activate_action(&format!("win.{which}"), None);
        }
        "auto-route" | "route" | "waypoint" => {
            // The first route (or mark), as a dialog or its detail in the sidebar.
            let (r, w) = { let s = app.session.borrow(); (s.routes.first().map(|r| r.id), s.waypoints.first().map(|w| w.id)) };
            let action = match which.as_str() { "auto-route" => "win.auto-route", "route" => "win.route-detail", _ => "win.waypoint-detail" };
            if let Some(id) = if which == "waypoint" { w } else { r } {
                let _ = super::chart_widget().activate_action(action, Some(&id.to_variant()));
            }
        }
        "polar" => super::polar::open(app),
        "performance" => {
            let _ = super::chart_widget().activate_action("win.performance", None);
        }
        "weather-route" => {
            let ends = route_ends();
            let (from, to) = ends.unwrap_or((c, c.destination(235.0, 450.0)));
            let mut r = Route::new(if ends.is_some() { "Passage demo" } else { "Long demo" });
            r.points.push(RoutePoint::new("Start", from));
            r.points.push(RoutePoint::new("Far", to));
            // Late in the forecast by default, so that it ends before the passage does.
            let leave_h = std::env::var("ISOMAR_ROUTE_LEAVE_H").ok().and_then(|h| h.parse().ok()).unwrap_or(72);
            r.departure = Some(chrono::Utc::now() + chrono::Duration::hours(leave_h));
            let id = app.session.borrow_mut().save_route(&r).map(|r| r.id).unwrap_or(0);
            gtk::glib::timeout_add_local_once(std::time::Duration::from_secs(10), move || {
                let _ = super::chart_widget().activate_action("win.weather-route", Some(&id.to_variant()));
            });
        }
        "weather-window" => {
            let ends = route_ends();
            let (from, to) = ends.unwrap_or((c, c.destination(235.0, 120.0)));
            let mut r = Route::new(if ends.is_some() { "Passage demo" } else { "Window demo" });
            r.points.push(RoutePoint::new("Start", from));
            r.points.push(RoutePoint::new("Away", to));
            let id = app.session.borrow_mut().save_route(&r).map(|r| r.id).unwrap_or(0);
            gtk::glib::timeout_add_local_once(std::time::Duration::from_secs(10), move || {
                let _ = super::chart_widget().activate_action("win.best-departure", Some(&id.to_variant()));
            });
        }
        other => eprintln!("isomar: unknown ISOMAR_DIALOG {other}"),
    }
}

/// `ISOMAR_ROUTE=<lat>,<lon>,<lat>,<lon>`: the ends of the demo route of `weather-route` and `weather-window`.
fn route_ends() -> Option<(crate::core::geo::LatLon, crate::core::geo::LatLon)> {
    let v = std::env::var("ISOMAR_ROUTE").ok()?;
    let n: Vec<f64> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    (n.len() == 4).then(|| (crate::core::geo::LatLon { lat: n[0], lon: n[1] }, crate::core::geo::LatLon { lat: n[2], lon: n[3] }))
}
