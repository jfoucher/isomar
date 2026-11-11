//! State shared by the chart, the sidebar pages and the dialogs.

use super::engine::Engine;
use super::shell::Shell;
use crate::config::Settings;
use crate::core::comm::hub::Hub;
use crate::core::geo::LatLon;
use crate::core::nav::model::Route;
use crate::core::session::Session;
use super::layers::aids::AidsLayer;
use super::layers::enc::EncLayer;
use super::layers::land::LandLayer;
use super::weather::WeatherState;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Selection {
    #[default]
    None,
    Waypoint(i64),
    RoutePoint { route: i64, idx: usize },
    Route(i64),
    Target(u32),
}

/// A route being drawn or edited. `route.id` is 0 for a new one.
pub struct EditRoute {
    pub route: Route,
}

pub struct Measure {
    pub from: LatLon,
    pub to: Option<LatLon>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragItem {
    Waypoint(i64),
    RoutePoint { route: i64, idx: usize },
    EditPoint(usize),
}

/// An entry other modules add to the chart's context menu, such as
/// "Get weather here".
#[derive(Clone)]
pub struct ContextItem {
    pub label: String,
    pub run: Rc<dyn Fn(&Rc<AppState>, LatLon)>,
}

pub struct AppState {
    pub session: RefCell<Session>,
    pub hub: RefCell<Hub>,
    pub canvas: Engine,
    pub settings: RefCell<Settings>,
    pub weather: RefCell<WeatherState>,
    pub land: RefCell<Option<Rc<LandLayer>>>,
    pub enc: RefCell<Option<Rc<EncLayer>>>,
    pub bsb: RefCell<Option<Rc<super::layers::bsb::BsbLayer>>>,
    pub aids: RefCell<Option<Rc<AidsLayer>>>,
    pub depth: RefCell<Option<Rc<super::layers::bathy::DepthLayer>>>,
    pub credit: RefCell<Option<Rc<super::layers::credit::Credit>>>,
    /// The ensemble checks of routes, by route id (`flows::ensemble`).
    pub ensembles: RefCell<std::collections::HashMap<i64, super::flows::ensemble::Check>>,
    /// Long tasks in progress, for the activity indicator.
    pub jobs: Rc<super::jobs::Jobs>,
    pub edit: RefCell<Option<EditRoute>>,
    pub measure: RefCell<Option<Measure>>,
    /// A weather-route search being previewed and played back.
    pub router: RefCell<Option<super::router::RouterPreview>>,
    /// The search being computed, whose fronts the chart shows as they grow.
    pub router_live: RefCell<Option<std::sync::Arc<crate::core::routing::isochrone::Progress>>>,
    /// How many weather-route calculations are in progress (forecast, search or preview); the
    /// shells keep their "Compute" button off while it is not zero.
    routing_busy: Cell<u32>,
    /// The results of "Best departure", while they are being looked at.
    pub departures: RefCell<Option<super::flows::departures::DepartureSet>>,
    /// The performance log's last analysis and what is running for it.
    pub perf: RefCell<super::flows::perf::PerfState>,
    pub selected: Cell<Selection>,
    /// How many times something was selected: a touch shell opens what was tapped each time, even the
    /// same mark again (`vm::window::Picked`).
    pub selections: Cell<u64>,
    pub drag: Cell<Option<DragItem>>,
    pub pointer: Cell<Option<LatLon>>,
    /// What the pointer is over, and what the open popover is about, outlined on the chart.
    pub probe: super::probe::Probe,
    drawn_revision: Cell<u64>,
    ticks_since_draw: Cell<u32>,
    refreshers: RefCell<Vec<Box<dyn Fn()>>>,
    tickers: RefCell<Vec<Box<dyn Fn()>>>,
    setting_appliers: RefCell<Vec<Box<dyn Fn(&Settings)>>>,
    pub context_items: RefCell<Vec<ContextItem>>,
    shell: RefCell<Option<Rc<dyn Shell>>>,
}

/// A weather-route calculation in progress; see `AppState::routing_guard`.
pub struct RoutingGuard(Rc<AppState>);

impl Drop for RoutingGuard {
    fn drop(&mut self) {
        self.0.routing_busy.set(self.0.routing_busy.get() - 1);
    }
}

/// The session's alarm limits from the settings.
fn set_alarm_limits(sess: &mut Session, s: &Settings) {
    sess.alarm_cfg.cpa_nm = s.ais_cpa_nm;
    sess.alarm_cfg.tcpa_min = s.ais_tcpa_min;
    sess.alarm_cfg.xte_limit_nm = s.xte_limit_nm;
    sess.alarm_cfg.shallow_m = s.shallow_alarm_m;
    sess.alarm_cfg.wind_limit_kn = s.wind_limit_kn;
    sess.perf.recording = s.perf_recording;
}

impl AppState {
    pub fn new(canvas: Engine, mut session: Session, settings: Settings) -> Rc<AppState> {
        // The alarms start with the user's limits, not the defaults (they were applied only
        // once a setting changed, so a CPA of 0.2 nm still alarmed at 0.5 until then).
        set_alarm_limits(&mut session, &settings);
        // The switch as it was left, and the polar the motoring hint measures against.
        session.perf.mode = crate::core::perf::SailMode::parse(&settings.sail_mode);
        session.perf.polar = crate::polars::load_active(&settings);
        session.perf.polar_id = crate::polars::active_id_in(&settings);
        Rc::new(AppState {
            session: RefCell::new(session),
            hub: RefCell::new(Hub::new()),
            canvas,
            weather: RefCell::new({
                let mut w = WeatherState { source: settings.weather_source, ..WeatherState::default() };
                w.currents.route_with = settings.currents.route_with;
                w
            }),
            settings: RefCell::new(settings),
            land: RefCell::new(None),
            enc: RefCell::new(None),
            bsb: RefCell::new(None),
            aids: RefCell::new(None),
            depth: RefCell::new(None),
            credit: RefCell::new(None),
            ensembles: RefCell::default(),
            jobs: Rc::new(super::jobs::Jobs::default()),
            edit: RefCell::new(None),
            measure: RefCell::new(None),
            router: RefCell::new(None),
            router_live: RefCell::new(None),
            routing_busy: Cell::new(0),
            departures: RefCell::new(None),
            perf: RefCell::new(Default::default()),
            selected: Cell::new(Selection::None),
            selections: Cell::new(0),
            drag: Cell::new(None),
            pointer: Cell::new(None),
            probe: super::probe::Probe::default(),
            drawn_revision: Cell::new(u64::MAX),
            ticks_since_draw: Cell::new(0),
            refreshers: RefCell::new(Vec::new()),
            tickers: RefCell::new(Vec::new()),
            setting_appliers: RefCell::new(Vec::new()),
            context_items: RefCell::new(Vec::new()),
            shell: RefCell::new(None),
        })
    }

    /// Registers something to call whenever the data changed.
    pub fn on_refresh(&self, f: impl Fn() + 'static) {
        self.refreshers.borrow_mut().push(Box::new(f));
    }

    /// Registers something to call on every clock tick (live readouts).
    pub fn on_tick(&self, f: impl Fn() + 'static) {
        self.tickers.borrow_mut().push(Box::new(f));
    }

    /// Redraws the chart and updates live readouts, without rebuilding lists.
    pub fn tick(&self) {
        // Repaint when data arrived, and now and then anyway so that time-based
        // things (a position going stale) show up.
        let rev = self.session.borrow().revision;
        let idle = self.ticks_since_draw.get() + 1;
        if rev != self.drawn_revision.get() || idle >= 8 {
            self.drawn_revision.set(rev);
            self.ticks_since_draw.set(0);
            self.canvas.queue_draw();
        } else {
            self.ticks_since_draw.set(idle);
        }
        super::vm::timeline::advance(self, 0.25);
        // Something that blinks needs repainting at the blink rate, between data.
        if self.canvas.animating() {
            self.canvas.queue_draw();
        }
        for f in self.tickers.borrow().iter() {
            f();
        }
    }

    pub fn add_context_item(&self, label: &str, run: impl Fn(&Rc<AppState>, LatLon) + 'static) {
        self.context_items.borrow_mut().push(ContextItem { label: label.to_string(), run: Rc::new(run) });
    }

    /// Marks a weather-route calculation as running until the guard is dropped.
    pub fn routing_guard(self: &Rc<Self>) -> RoutingGuard {
        self.routing_busy.set(self.routing_busy.get() + 1);
        RoutingGuard(self.clone())
    }

    /// Whether a weather-route calculation is running.
    pub fn routing_busy(&self) -> bool {
        self.routing_busy.get() > 0
    }

    /// Redraws the chart and tells every view to update itself. Never call
    /// this while holding a borrow of `session`.
    pub fn refresh(&self) {
        self.canvas.queue_draw();
        for f in self.refreshers.borrow().iter() {
            f();
        }
    }

    /// Registers something that applies settings (called now and whenever
    /// they change).
    pub fn on_settings(&self, f: impl Fn(&Settings) + 'static) {
        f(&self.settings.borrow());
        self.setting_appliers.borrow_mut().push(Box::new(f));
    }

    /// Applies the current settings everywhere and saves them.
    pub fn apply_settings(&self) {
        let s = self.settings.borrow().clone();
        set_alarm_limits(&mut self.session.borrow_mut(), &s);
        for f in self.setting_appliers.borrow().iter() {
            f(&s);
        }
        // Never from a test: they ran with the defaults and wrote them over the user's settings.
        if std::env::var_os("ISOMAR_NO_SAVE").is_none() && !cfg!(test) {
            s.save();
        }
        self.refresh();
    }

    pub fn set_shell(&self, shell: Rc<dyn Shell>) {
        *self.shell.borrow_mut() = Some(shell);
    }

    /// The shell, once the window has installed it.
    pub fn shell(&self) -> Option<Rc<dyn Shell>> {
        self.shell.borrow().clone()
    }

    pub fn toast(&self, text: &str) {
        if let Some(s) = self.shell() {
            s.toast(text);
        }
    }

    pub fn show_error(&self, heading: &str, details: &str) {
        if let Some(s) = self.shell() {
            s.show_error(heading, details);
        }
    }

    pub fn sound(&self, sound: super::shell::Sound) {
        if let Some(s) = self.shell() {
            s.sound(sound);
        }
    }

    /// A long calculation the user may have stopped watching has ended (a weather route, a
    /// best-departure scan, an ensemble check), well or not: the soft sound, and a notification
    /// when the app is out of sight. Not for one the user cancelled.
    pub fn finished(&self, title: &str, body: &str) {
        if let Some(s) = self.shell() {
            s.sound(super::shell::Sound::Tick);
            s.notify(title, body);
        }
    }

    pub fn select(&self, s: Selection) {
        self.selected.set(s);
        self.selections.set(self.selections.get() + 1);
        self.refresh();
    }
}
