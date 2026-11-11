//! One time control for the forecast and for the router's clock. With a route preview open it
//! moves the preview's clock (and the weather overlay follows it); otherwise it moves the weather
//! overlay's time through the downloaded forecast.

use crate::app::AppState;
use crate::i18n::gettext_f;
use chrono::{DateTime, Duration, Local, Utc};
use serde::Serialize;

/// At 1x the clock runs this many forecast hours a second.
pub const HOURS_PER_SECOND: f64 = 3.0;

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Weather,
    Router,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Timeline {
    pub mode: Mode,
    /// Seconds since the epoch.
    pub start: i64,
    pub end: i64,
    pub now: i64,
    /// One step of the arrow keys, in seconds.
    pub step: i64,
    pub playing: bool,
    pub speed: f64,
    /// "Tue 28 Oct 15:00".
    pub label: String,
    /// "GFS run 06 UTC, 31 h old", when there is a forecast.
    pub age: Option<String>,
    pub stale: bool,
    /// The spans the forecast has data for, as seconds since the epoch.
    pub coverage: Vec<(i64, i64)>,
}

fn local(t: DateTime<Utc>) -> String {
    t.with_timezone(&Local).format("%a %d %b %H:%M").to_string()
}

pub fn timeline(app: &AppState) -> Option<Timeline> {
    let w = app.weather.borrow();
    let speed = w.speed_x;
    let age = w.store.latest_run().map(|r| {
        let h = (Utc::now() - r).num_hours().max(0);
        gettext_f("{model} run {run}, {h} h old", &[("model", w.source.label()), ("run", &r.format("%H UTC").to_string()), ("h", &h.to_string())])
    });
    let stale = crate::weather::layer::forecast_age(&w.store).is_some_and(|a| a > Duration::hours(crate::weather::layer::STALE_AFTER_HOURS));
    // The forecast's span and the currents', so the timeline plays with either loaded alone.
    let span = match (w.store.time_range(), w.currents.field().time_range()) {
        (Some((a, b)), Some((c, d))) => Some((a.min(c), b.max(d))),
        (a, c) => a.or(c),
    };
    let coverage: Vec<(i64, i64)> = span.map(|(a, b)| vec![(a.timestamp(), b.timestamp())]).unwrap_or_default();
    if let Some(p) = app.router.borrow().as_ref() {
        return Some(Timeline {
            mode: Mode::Router,
            start: p.ex.depart.timestamp(),
            end: p.end.timestamp(),
            now: p.t.timestamp(),
            step: 3600,
            playing: w.playing,
            speed,
            label: local(p.t),
            age,
            stale,
            coverage,
        });
    }
    let (lo, hi) = span?;
    Some(Timeline { mode: Mode::Weather, start: lo.timestamp(), end: hi.timestamp(), now: w.time.timestamp(), step: 3600, playing: w.playing, speed, label: local(w.time), age, stale, coverage })
}

/// Moves the clock to `t` (seconds since the epoch), within the span.
pub fn set_time(app: &AppState, t: i64) {
    let Some(tl) = timeline(app) else { return };
    let t = t.clamp(tl.start, tl.end.max(tl.start));
    let Some(when) = DateTime::from_timestamp(t, 0) else { return };
    match tl.mode {
        Mode::Router => {
            if let Some(p) = app.router.borrow_mut().as_mut() {
                p.t = when;
            }
            crate::router::follow_weather(app, when);
        }
        Mode::Weather => app.weather.borrow_mut().time = when,
    }
    app.canvas.queue_draw();
}

/// Moves the clock by `steps` hours.
pub fn step(app: &AppState, steps: i64) {
    if let Some(tl) = timeline(app) {
        set_time(app, tl.now + steps * tl.step);
    }
}

pub fn set_playing(app: &AppState, on: bool) {
    app.weather.borrow_mut().playing = on;
}

pub fn set_speed(app: &AppState, x: f64) {
    app.weather.borrow_mut().speed_x = x.clamp(0.25, 8.0);
}

/// Runs the clock for `dt` seconds of real time while it is playing; stops at the end.
pub fn advance(app: &AppState, dt: f64) {
    let playing = app.weather.borrow().playing;
    let Some(tl) = timeline(app).filter(|_| playing) else { return };
    let next = tl.now as f64 + dt * tl.speed * HOURS_PER_SECOND * 3600.0;
    if next >= tl.end as f64 {
        set_time(app, tl.end);
        set_playing(app, false);
    } else {
        set_time(app, next as i64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::nav::store::NavStore;
    use crate::core::session::Session;
    use crate::engine::Engine;
    use crate::palette::Mode as PaletteMode;
    use std::rc::Rc;

    fn app() -> Rc<AppState> {
        let canvas = Engine::new(crate::config::ViewState { lat: 48.0, lon: -4.0, zoom: 8.0 }, PaletteMode::Day);
        AppState::new(canvas, Session::new(NavStore::open_memory().unwrap()).unwrap(), crate::config::Settings::default())
    }

    /// A 3x3 wind field every three hours for twelve hours, from a run three hours ago.
    fn with_forecast(app: &AppState) -> DateTime<Utc> {
        use crate::core::chart::grid::Grid;
        use crate::core::weather::model::{Field, Param};
        let run = DateTime::from_timestamp((Utc::now() - Duration::hours(3)).timestamp(), 0).unwrap();
        let mut w = app.weather.borrow_mut();
        for h in [0, 3, 6, 9, 12] {
            for p in [Param::WindU, Param::WindV] {
                w.store.add(Field {
                    param: p,
                    time: run + Duration::hours(h),
                    run: Some(run),
                    grid: Grid { ncols: 3, nrows: 3, xll: -5.0, yll: 47.0, dx: 1.0, dy: 1.0, data: vec![5.0; 9] },
                });
            }
        }
        w.time = run;
        run
    }

    #[test]
    fn without_a_forecast_there_is_no_timeline() {
        assert!(timeline(&app()).is_none());
    }

    #[test]
    fn the_timeline_spans_the_forecast_and_names_its_run() {
        let app = app();
        let run = with_forecast(&app);
        let t = timeline(&app).unwrap();
        assert_eq!(t.mode, Mode::Weather);
        assert_eq!((t.start, t.end), (run.timestamp(), (run + Duration::hours(12)).timestamp()));
        assert!(t.age.as_deref().unwrap().contains("3 h old"), "{:?}", t.age);
        assert!(!t.stale);
        assert_eq!(t.coverage, vec![(t.start, t.end)]);
    }

    #[test]
    fn the_clock_stays_inside_the_forecast_and_steps_by_hours() {
        let app = app();
        let run = with_forecast(&app);
        step(&app, 2);
        assert_eq!(app.weather.borrow().time, run + Duration::hours(2));
        set_time(&app, run.timestamp() - 100_000);
        assert_eq!(app.weather.borrow().time, run, "not before the start");
        set_time(&app, run.timestamp() + 1_000_000);
        assert_eq!(app.weather.borrow().time, run + Duration::hours(12), "not after the end");
    }

    #[test]
    fn playing_runs_the_clock_and_stops_at_the_end() {
        let app = app();
        let run = with_forecast(&app);
        advance(&app, 1.0);
        assert_eq!(app.weather.borrow().time, run, "not playing: still");
        set_playing(&app, true);
        advance(&app, 1.0);
        assert_eq!(app.weather.borrow().time, run + Duration::hours(3), "3 forecast hours a second at 1x");
        set_speed(&app, 2.0);
        advance(&app, 1.0);
        assert_eq!(app.weather.borrow().time, run + Duration::hours(9));
        advance(&app, 10.0);
        assert_eq!(app.weather.borrow().time, run + Duration::hours(12));
        assert!(!app.weather.borrow().playing, "it stops at the end");
    }
}
