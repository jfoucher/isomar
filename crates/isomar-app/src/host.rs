//! Start-up and the clock of the application, shared by the shells: which
//! layers there are and in what order, which start hidden, the settings that
//! reach them, the connections, and the 4 Hz tick.

use crate::app::AppState;
use crate::chips::{Chip, Icon};
use crate::config::Settings;
use crate::core::comm::source::{ConnectionConfig, Kind};
use crate::core::session::Notice;
use crate::data::{self, ChartLayers, Status};
use crate::engine::Engine;
use crate::i18n::{gettext, gettext_f};
use crate::layers::boat::BoatLayer;
use crate::layers::nav::NavLayer;
use crate::layers::hover::HoverLayer;
use crate::layers::router::RouterLayer;
use crate::weather::layer::{WeatherFrame, WeatherLayer};
use crate::{dem, emodnet, places, tiles};
use chrono::Utc;
use std::rc::Rc;

/// Layers in the menu that start off, until the user ticks them.
pub const OFF_BY_DEFAULT: [&str; 1] = [crate::emodnet::LAYER];

/// Whether the menu entry `id` is ticked under `settings`.
pub fn ticked(settings: &Settings, id: &str) -> bool {
    if OFF_BY_DEFAULT.contains(&id) { settings.optional_layers.iter().any(|l| l == id) } else { !settings.hidden_layers.iter().any(|h| h == id) }
}

pub fn toggleable_layers() -> [(&'static str, String); 15] {
    [
        ("depth", gettext("Depth shading")),
        ("land", gettext("Land")),
        ("enc", gettext("Charts (S-57 ENC)")),
        ("soundings", gettext("Soundings (from depth grid)")),
        ("bsb", gettext("Raster charts (KAP)")),
        ("graticule", gettext("Lat/lon grid")),
        ("aids", gettext("Buoys and lights")),
        (crate::tiles::LAND_LAYER, gettext("Download coastline (OpenStreetMap)")),
        (crate::tiles::MARKS_LAYER, gettext("Download seamarks (OpenStreetMap)")),
        (crate::places::LAYER, gettext("Place names and town outlines (OpenStreetMap)")),
        (crate::emodnet::LAYER, gettext("Download depths (EMODnet, saved locally)")),
        (crate::dem::LAYER, gettext("Relief: hillshade and contours (terrain tiles)")),
        ("nav", gettext("Routes, waypoints and tracks")),
        ("weather", gettext("Weather")),
        (crate::currents::layer::LAYER, gettext("Currents")),
    ]
}


/// Creates the chart layers and the downloads that feed them, adds the
/// navigation layers on top, and hides what the settings and the
/// `ISOMAR_HIDE` / `ISOMAR_SHOW` variables say.
pub fn install(app: &Rc<AppState>, canvas: &Engine, status: Status, settings: &Settings) -> ChartLayers {
    // The single polar of earlier versions, and the log's samples, into the polar library (once).
    let moved = crate::polars::migrate(&mut app.settings.borrow_mut(), &app.session.borrow().store);
    if moved {
        crate::polars::active_changed(app);
        app.apply_settings();
    }
    let layers = data::install(canvas, status.clone(), &settings.chart_folders);
    tiles::install(app, layers.land.clone(), layers.aids.clone(), status.clone());
    places::install(app, layers.towns.clone(), status.clone());
    emodnet::install(app, layers.depth.clone(), status.clone());
    dem::install(app, layers.relief.clone(), status);
    crate::weather::install(app);
    if !cfg!(test) {
        crate::currents::atlases::start(app);
    }
    app.add_context_item(&gettext("Tides here…"), |app, pos| {
        if let Some(s) = app.shell() {
            s.open(crate::shell::Dialog::Tides(pos));
        }
    });
    app.add_context_item(&gettext("Charts for this area…"), |app, pos| {
        if let Some(s) = app.shell() {
            s.open(crate::shell::Dialog::ChartsHere(pos));
        }
    });
    canvas.add_layer(Rc::new(crate::currents::layer::CurrentLayer::new(app)));
    canvas.add_layer(Rc::new(WeatherLayer::new(app)));
    canvas.add_layer(Rc::new(WeatherFrame::new(app)));
    *app.land.borrow_mut() = Some(layers.land.clone());
    *app.enc.borrow_mut() = Some(layers.enc.clone());
    *app.bsb.borrow_mut() = Some(layers.bsb.clone());
    *app.aids.borrow_mut() = Some(layers.aids.clone());
    *app.depth.borrow_mut() = Some(layers.depth.clone());
    *app.credit.borrow_mut() = Some(layers.credit.clone());
    canvas.add_layer(Rc::new(HoverLayer::new(app)));
    canvas.add_layer(Rc::new(RouterLayer::new(app)));
    canvas.add_layer(Rc::new(NavLayer::new(app)));
    canvas.add_layer(Rc::new(BoatLayer::new(app)));
    for (id, _) in toggleable_layers() {
        if !ticked(settings, id) {
            canvas.set_layer_visible(id, false);
        }
    }
    // Layers outside the menu can be hidden too.
    for id in &settings.hidden_layers {
        canvas.set_layer_visible(id, false);
    }
    // Development aid: ISOMAR_HIDE=weather,aids hides those layers.
    if let Ok(list) = std::env::var("ISOMAR_HIDE") {
        list.split(',').for_each(|id| canvas.set_layer_visible(id.trim(), false));
    }
    // Development aid: ISOMAR_SHOW=online-depth ticks layers that start off.
    if let Ok(list) = std::env::var("ISOMAR_SHOW") {
        list.split(',').for_each(|id| canvas.set_layer_visible(id.trim(), true));
    }
    crate::layers::set_text_scale(settings.text_scale);
    crate::symbols::set_style(settings.chart_style.parse().unwrap_or(crate::symbols::Style::Paper));
    crate::symbols::set_full_sectors(settings.full_sectors);
    {
        let canvas = canvas.clone();
        app.on_settings(move |s| {
            crate::layers::set_text_scale(s.text_scale);
            crate::symbols::set_style(s.chart_style.parse().unwrap_or(crate::symbols::Style::Paper));
            crate::symbols::set_full_sectors(s.full_sectors);
            canvas.queue_draw();
        });
    }
    {
        let (depth, enc) = (layers.depth.clone(), layers.enc.clone());
        enc.set_max_band(settings.enc_max_band);
        app.on_settings(move |s| {
            depth.set_safety_depth(s.safety_depth);
            enc.set_safety_depth(s.safety_depth as f64);
            enc.set_max_band(s.enc_max_band);
        });
    }
    if let Some(m) = crate::debug::palette() {
        canvas.set_mode(m);
    }
    canvas.set_underway(crate::debug::underway());
    layers
}

/// The id of the simulated boat `ISOMAR_DEMO` adds for one run. It is never saved: a phone saves the
/// connections each time the app goes to the background, and the demo boat, with its traffic and their
/// collision alarms, came back at every launch. (A demo boat the user adds has an id of its own and stays.)
const DEMO_CONNECTION: &str = "demo";

/// The connections to keep in the settings: all of them but the `ISOMAR_DEMO` boat.
pub fn saved_connections(app: &AppState) -> Vec<ConnectionConfig> {
    without_demo(app.hub.borrow().configs())
}

fn without_demo(mut cfgs: Vec<ConnectionConfig>) -> Vec<ConnectionConfig> {
    cfgs.retain(|c| c.id != DEMO_CONNECTION);
    cfgs
}

/// Starts the saved connections, and the simulated boat under `ISOMAR_DEMO`.
/// Returns whether there is no data source at all.
pub fn start_connections(app: &AppState, settings: &Settings) -> bool {
    let demo = std::env::var_os("ISOMAR_DEMO").is_some();
    // Settings saved before the demo boat was left out may still hold it.
    let mut cfgs = without_demo(settings.connections.clone());
    // Only in a demo area, moored or under way (`debug::demo_boat`), never wherever the view is.
    if demo && let Some(b) = crate::debug::demo_boat(app.canvas.view_state().center()) {
        cfgs.push(ConnectionConfig::new(DEMO_CONNECTION, &gettext("Demo boat"), Kind::Simulator { lat: b.lat, lon: b.lon, cog: b.cog, sog: b.sog }));
    }
    let none = cfgs.is_empty();
    for c in cfgs {
        app.hub.borrow_mut().add(c);
    }
    none
}

thread_local! {
    /// How many vector and raster charts were loaded at the last poll, and at the last refresh.
    static CHART_COUNTS: std::cell::Cell<[(usize, usize); 2]> = const { std::cell::Cell::new([(0, 0); 2]) };
}

/// Charts load in the background, at start, from an install or a folder just added: once the
/// count stops moving, the pages that list or depend on them (the Charts page, "planning only")
/// are refreshed. Not on every batch: a folder of thousands of cells arrives in many.
fn refresh_when_charts_settle(app: &AppState) {
    let now = (app.enc.borrow().as_ref().map_or(0, |e| e.len()), app.bsb.borrow().as_ref().map_or(0, |b| b.charts().len()));
    let [seen, shown] = CHART_COUNTS.get();
    let settled = now == seen && now != shown;
    CHART_COUNTS.set([now, if settled { now } else { shown }]);
    if settled {
        app.refresh();
    }
}

/// What the 4 Hz clock does before the shell looks at the result: reads the
/// connections and updates the session. Returns what the user should hear of.
pub fn poll(app: &AppState) -> Vec<Notice> {
    use crate::shell::Sound;
    crate::debug::fake_alarm(app);
    // Said here, not at start-up: the shell that shows it is installed after the settings are read.
    if app.shell().is_some() && Settings::take_load_problem() {
        app.toast(&gettext("Settings could not be read and were reset; the old file was kept as settings.json.bad"));
    }
    let events = app.hub.borrow().poll(4000);
    let now = Utc::now();
    let notices = app.session.borrow_mut().process(&events, now);
    crate::flows::perf::after_poll(app);
    // A forecast that is old is said so, as a warning that clears when a new one arrives.
    {
        use crate::core::alerts::{Level, Source};
        let w = app.weather.borrow();
        let stale = crate::weather::layer::forecast_age(&w.store).filter(|a| *a > chrono::Duration::hours(crate::weather::layer::STALE_AFTER_HOURS));
        let mut s = app.session.borrow_mut();
        match (stale, w.store.latest_run()) {
            (Some(age), Some(run)) => {
                let text = gettext_f("{model} run {run}, {h} h old", &[("model", w.source.label()), ("run", &run.format("%H UTC").to_string()), ("h", &age.num_hours().to_string())]);
                s.alerts.raise(now, Level::Warning, Source::StaleForecast, text, None);
            }
            _ => s.alerts.clear(&Source::StaleForecast),
        }
    }
    refresh_when_charts_settle(app);
    let (alarm, chime) = {
        let mut s = app.session.borrow_mut();
        (s.alerts.alarm_sound_due(now), s.alerts.take_new_warnings() > 0)
    };
    // The alarm repeats until it is dealt with; a warning chimes once.
    if alarm {
        app.sound(Sound::Alarm);
    } else if chime {
        app.sound(Sound::Chime);
    }
    notices
}

/// The start of the window's subtitle: speed over the ground, when known, or how old the fix is.
pub fn subtitle(app: &AppState) -> Option<Chip> {
    let s = app.session.borrow();
    if let Some(age) = crate::layers::boat::fix_age_text(s.own.pos_updated.filter(|_| s.own.pos.is_some()), Utc::now()) {
        return Some(Chip::plain(age));
    }
    let sog = format!("{:.1}", s.own.sog_kn.filter(|_| s.own.pos.is_some())?);
    Some(Chip::new(Icon::Speed, gettext_f("{sog} kn", &[("sog", &sog)]), gettext_f("{sog} kn over the ground", &[("sog", &sog)])))
}

/// What the footer says of the view when the pointer is not on the chart: the zoom and the scale.
pub fn view_chips(canvas: &Engine, zoom: f64) -> Vec<Chip> {
    let scale = format!("{:.2}", canvas.metres_per_pixel() * 100.0 / crate::core::geo::NM_M);
    let zoom = format!("{zoom:.1}");
    vec![
        Chip::new(Icon::Zoom, zoom.clone(), gettext_f("Zoom {zoom}", &[("zoom", &zoom)])),
        Chip::with(Icon::Distance, gettext_f("{scale} nm / 100 px", &[("scale", &scale)])),
    ]
}

/// [`view_chips`] as text.
pub fn view_text(canvas: &Engine, zoom: f64) -> String {
    crate::chips::plain(&view_chips(canvas, zoom))
}

/// Saves what should outlive the session and stops the connections (see `save`).
pub fn shutdown(app: &AppState) {
    save(app);
    app.hub.borrow_mut().stop_all();
}

/// Saves what should outlive the session (the view, the display mode, the hidden layers, the
/// connections), and goes on: a phone's shell calls it when the app goes to the background, since the
/// system may end a suspended app without a word. A view forced by `ISOMAR_DEBUG_VIEW` is not saved,
/// and nothing is under `ISOMAR_NO_SAVE`.
pub fn save(app: &AppState) {
    if crate::debug::forced_view().is_none() {
        {
            let mut s = app.settings.borrow_mut();
            s.view = app.canvas.view_state();
            s.mode = app.canvas.mode().as_str().to_string();
            let menu = toggleable_layers();
            s.hidden_layers = menu.iter().filter(|(id, _)| !OFF_BY_DEFAULT.contains(id) && !app.canvas.layer_visible(id)).map(|(id, _)| id.to_string()).collect();
            s.optional_layers = menu.iter().filter(|(id, _)| OFF_BY_DEFAULT.contains(id) && app.canvas.layer_visible(id)).map(|(id, _)| id.to_string()).collect();
            s.connections = saved_connections(app);
            s.weather_source = app.weather.borrow().source;
        }
        if std::env::var_os("ISOMAR_NO_SAVE").is_none() && !cfg!(test) {
            app.settings.borrow().save();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_demo_boat_is_never_saved() {
        let sim = Kind::Simulator { lat: 48.0, lon: -4.5, cog: 90.0, sog: 6.0 };
        let cfgs = vec![
            ConnectionConfig::new(DEMO_CONNECTION, "Demo boat", sim.clone()),
            ConnectionConfig::new("c1", "Demo boat", sim),
            ConnectionConfig::new("c2", "Chartplotter", Kind::Tcp { host: "192.168.1.5".into(), port: 10110 }),
        ];
        let ids: Vec<String> = without_demo(cfgs).into_iter().map(|c| c.id).collect();
        assert_eq!(ids, ["c1", "c2"]);
    }
}
