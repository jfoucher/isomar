//! The main window (`design/ia.md`, window model): a header bar, the sidebar at the start
//! (floating over the chart below 1000 px and under way), and the chart with what floats over it:
//! alert strips, the hint, navigating and router cards, the forecast readout, the dashboard, the
//! timeline, the map controls and the footer strip.

use super::actions;
use super::alerts::{self, HintCard, Notifier, Strips};
use super::app::AppState;
use super::canvas::Canvas;
use super::debug;
use super::dialogs;
use super::header::{self, Header};
use super::overlays::{self, Dashboard, FooterStrip, InstrumentColumn, MapCluster, NavigatingCard, RouterCard, TimelineBar, WeatherReadout};
use super::palette::Mode;
use super::sidebar::{self, Sidebar};
use crate::config::Settings;
use crate::core::datadir;
use crate::core::nav::store::NavStore;
use crate::core::session::Session;
use crate::i18n::gettext;
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

pub use isomar_app::host::{ticked, toggleable_layers};

/// Under this width the sidebar floats over the chart instead of pushing it aside.
const NARROW_BELOW: f64 = 1000.0;
/// Under this width the header and footer drop what can wait.
const COMPACT_BELOW: f64 = 600.0;

pub struct MainWindow {
    pub window: adw::ApplicationWindow,
    footer: Rc<FooterStrip>,
    pub app: Rc<AppState>,
    sidebar: Rc<Sidebar>,
    split: adw::OverlaySplitView,
    header: Rc<Header>,
    cluster: Rc<MapCluster>,
    following: Rc<Cell<bool>>,
    narrow: Cell<bool>,
    underway: Cell<bool>,
    sidebar_before: Cell<bool>,
}

impl MainWindow {
    pub fn get_or_create(app: &adw::Application) -> adw::ApplicationWindow {
        if let Some(w) = app.active_window().and_then(|w| w.downcast::<adw::ApplicationWindow>().ok()) {
            return w;
        }
        let mw = Self::new(app);
        let win = mw.window.clone();
        // The window owns the state for as long as it lives.
        let keep = std::cell::RefCell::new(Some(mw));
        win.connect_destroy(move |_| {
            keep.borrow_mut().take();
        });
        win
    }

    fn new(application: &adw::Application) -> Rc<Self> {
        let settings = Settings::load();
        let mode = debug::palette().or_else(|| settings.mode.parse::<Mode>().ok()).unwrap_or_default();
        super::theme::apply(mode);
        let chart = Canvas::new(debug::forced_view().unwrap_or(settings.view), mode);
        super::set_chart_widget(&chart.widget);
        let (widget, canvas) = (chart.widget, chart.engine);

        let store = NavStore::open(&datadir::nav_db_path()).or_else(|_| NavStore::open_memory()).expect("in-memory database");
        let session = Session::new(store).expect("navigation data");
        let app = AppState::new(canvas.clone(), session, settings.clone());

        let toasts = adw::ToastOverlay::new();
        // What the data loaders report: work in progress ("Loading map tiles…") is a job in the
        // activity indicator until an empty status ends it; anything else is a message.
        let status: isomar_app::data::Status = {
            let (t, a) = (toasts.clone(), Rc::downgrade(&app));
            let job: RefCell<Option<isomar_app::jobs::JobHandle>> = RefCell::new(None);
            Rc::new(move |s: &str| {
                job.borrow_mut().take();
                if s.ends_with('…') {
                    if let Some(a) = a.upgrade() {
                        *job.borrow_mut() = Some(a.jobs.start("", s.trim_end_matches('…'), None));
                    }
                } else if !s.is_empty() {
                    t.add_toast(adw::Toast::new(s));
                }
            })
        };
        isomar_app::host::install(&app, &canvas, status, &settings);
        actions::install_interaction(&app);
        // The footer strip is GTK's: the engine leaves out its scale bar and credits.
        canvas.set_footer_native(true);
        if let Some(c) = app.credit.borrow().as_ref() {
            c.hidden.set(true);
        }
        if let Some(s) = gtk::Settings::default() {
            canvas.set_reduce_motion(!s.is_gtk_enable_animations());
        }

        // Sidebar.
        let sidebar = Sidebar::new(&app);
        let split = adw::OverlaySplitView::builder().sidebar_position(gtk::PackType::Start).min_sidebar_width(300.0).max_sidebar_width(isomar_app::tokens::SIDEBAR_WIDTH).build();
        split.set_sidebar(Some(&sidebar.widget));
        if let Ok(p) = std::env::var("ISOMAR_SIDEBAR") {
            sidebar.show(sidebar::page_name(&p));
        }

        // The chart and what floats over it.
        let following = Rc::new(Cell::new(false));
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&widget));
        let (frame, frame_on) = alerts::alarm_frame();
        overlay.add_overlay(&frame);
        let hint = HintCard::new(&app);
        let nav_card = NavigatingCard::new(&app);
        let router_card = RouterCard::new(&app);
        let tasks = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).halign(gtk::Align::Start).valign(gtk::Align::Start).margin_top(16).margin_start(16).build();
        tasks.append(&nav_card.widget);
        tasks.append(&hint.widget);
        tasks.append(&router_card.widget);
        super::weather::dialog::install(&tasks);
        overlay.add_overlay(&tasks);
        let readout = WeatherReadout::new();
        overlay.add_overlay(&readout.widget);
        let dashboard = Dashboard::new();
        let timeline = TimelineBar::new(&app);
        let bottom = gtk::Box::builder().spacing(10).valign(gtk::Align::End).margin_start(16).margin_bottom(16).margin_end(16 + 56 + 16).build();
        bottom.append(&dashboard.widget);
        bottom.append(&timeline.widget);
        overlay.add_overlay(&bottom);
        let cluster = MapCluster::new(&app, following.clone());
        overlay.add_overlay(&cluster.widget);
        for w in [tasks.upcast_ref::<gtk::Widget>(), bottom.upcast_ref(), frame.upcast_ref()] {
            overlay.set_measure_overlay(w, false);
        }
        bottom.set_can_target(true);

        let strips = Strips::new(&app);
        let footer = FooterStrip::new(&app);
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&strips.widget);
        column.append(&overlay);
        column.append(&footer.widget);
        column.set_hexpand(true);
        // Under way the instruments take the sidebar's place, the whole height beside the chart.
        let instruments = InstrumentColumn::new(&app);
        let with_instruments = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        with_instruments.append(&instruments.widget);
        with_instruments.append(&column);
        toasts.set_child(Some(&with_instruments));
        split.set_content(Some(&toasts));

        let shell = super::shell::GtkShell::new(&toasts);
        shell.attach(&app, &sidebar);
        app.set_shell(shell);

        let header = Header::new(&app);
        split.bind_property("show-sidebar", &header.sidebar, "active").bidirectional().sync_create().build();
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header.bar);
        // Dialogs float over the sidebar and the chart, inside the window (`float`).
        let panels = gtk::Overlay::new();
        panels.set_child(Some(&split));
        super::float::install(&panels);
        toolbar.set_content(Some(&panels));

        let window = adw::ApplicationWindow::builder()
            .application(application)
            .title(gettext("Isomar"))
            .default_width(debug::window_size().map_or(1280, |s| s.0))
            .default_height(debug::window_size().map_or(800, |s| s.1))
            .width_request(360)
            .height_request(400)
            .content(&toolbar)
            .build();
        // GTK's labels share cairo's faces of Adwaita Sans with the tile threads: keep theirs alive too
        // (`isomar_app::fonts`), with the options of the screen the window is on.
        let hold_ui_fonts = |w: &adw::ApplicationWindow| {
            let layout = w.create_pango_layout(None);
            layout.set_markup("Aa <b>Aa</b> <i>Aa</i> <b><i>Aa</i></b>");
            isomar_app::fonts::hold_layout(&layout);
        };
        window.connect_map(hold_ui_fonts);
        window.connect_scale_factor_notify(hold_ui_fonts);

        let mw = Rc::new(MainWindow {
            window: window.clone(),
            footer: footer.clone(),
            app: app.clone(),
            sidebar,
            split,
            header,
            cluster,
            following,
            narrow: Cell::new(false),
            underway: Cell::new(false),
            sidebar_before: Cell::new(true),
        });

        // Below 1000 px the sidebar floats over the chart.
        let bp = adw::Breakpoint::new(adw::BreakpointCondition::new_length(adw::BreakpointConditionLengthType::MaxWidth, NARROW_BELOW, adw::LengthUnit::Sp));
        // A setter, so that it holds before the window is measured; the signals keep `narrow`.
        bp.add_setter(&mw.split, "collapsed", Some(&true.to_value()));
        {
            let m = Rc::downgrade(&mw);
            bp.connect_apply(move |_| {
                if let Some(m) = m.upgrade() {
                    m.narrow.set(true);
                    m.layout();
                }
            });
            let m = Rc::downgrade(&mw);
            bp.connect_unapply(move |_| {
                if let Some(m) = m.upgrade() {
                    m.narrow.set(false);
                    m.layout();
                }
            });
        }
        window.add_breakpoint(bp);
        // At phone width the header and footer keep only what matters.
        // Only one breakpoint holds at a time, so this one is narrow too.
        let compact = adw::Breakpoint::new(adw::BreakpointCondition::new_length(adw::BreakpointConditionLengthType::MaxWidth, COMPACT_BELOW, adw::LengthUnit::Sp));
        compact.add_setter(&mw.split, "collapsed", Some(&true.to_value()));
        for on in [true, false] {
            let m = Rc::downgrade(&mw);
            let f = move |_: &adw::Breakpoint| {
                if let Some(m) = m.upgrade() {
                    m.header.set_compact(on);
                    m.footer.set_compact(on);
                    m.narrow.set(on);
                    m.layout();
                }
            };
            if on {
                compact.connect_apply(f);
            } else {
                compact.connect_unapply(f);
            }
        }
        window.add_breakpoint(compact);
        mw.split.set_show_sidebar(debug::window_size().is_none_or(|s| f64::from(s.0) >= NARROW_BELOW));
        if std::env::var_os("ISOMAR_SIDEBAR").is_some() {
            // After the first layout: collapsing at a narrow start hides the sidebar.
            let split = mw.split.clone();
            glib::timeout_add_local_once(Duration::from_millis(300), move || split.set_show_sidebar(true));
        }

        mw.install_actions(application, &settings);
        alerts::install_action(application, &app);
        mw.install_keys(&widget);
        if isomar_app::host::start_connections(&app, &settings) {
            app.toast(&gettext("No data source: open the menu and choose Connections"));
        }

        // The 4 Hz clock: connections, navigation and alarms, and what the chrome shows of them.
        let notifier = Notifier::new();
        let stats_shown = Cell::new(false);
        let update = {
            let m = Rc::downgrade(&mw);
            move || {
                let Some(m) = m.upgrade() else { return };
                let app = &m.app;
                let w = isomar_app::vm::window::window(app);
                if w.underway != m.underway.get() {
                    m.set_underway(w.underway);
                }
                m.header.update(&w);
                super::busy::update(&w.jobs);
                let a = isomar_app::vm::banner::alerts(app);
                strips.update(&a);
                hint.update(a.hint.as_ref());
                frame_on.set(a.alarm.as_ref().is_some_and(|s| !s.dim));
                notifier.update(&m.window, &a);
                let u = isomar_app::vm::underway::underway(app);
                // The card floats over the chart while navigating in the planning layout; under way it heads
                // the instruments' column.
                nav_card.update(if u.on { None } else { u.card.as_ref() });
                instruments.update(u.on, u.card.as_ref(), &u.dashboard, u.sail.as_ref());
                if !u.on && app.settings.borrow().dashboard_on_chart {
                    let r: Vec<_> = isomar_app::vm::underway::dashboard_ids(app).iter().filter_map(|id| isomar_app::vm::underway::readout(app, id)).collect();
                    dashboard.update(&r);
                } else {
                    dashboard.update(&[]);
                }
                let card = isomar_app::vm::router::card(app);
                if card.is_some()
                    && debug::router_stats()
                    && !stats_shown.replace(true)
                    && let Some((heads, sections)) = isomar_app::vm::router::stats(app)
                {
                    super::routing_stats::show_sections(&gettext("Route statistics"), &heads, &sections);
                }
                router_card.update(card.as_ref());
                // Best departure's results, on the weather routing page.
                super::pages::routing::update(app);
                timeline.refresh(app);
                readout.update(app);
                footer.update(&isomar_app::vm::footer::footer(app));
            }
        };
        update();
        let update = Rc::new(update);
        {
            let u = update.clone();
            app.on_refresh(move || u());
        }
        {
            let (weak, following) = (Rc::downgrade(&app), mw.following.clone());
            glib::timeout_add_local(Duration::from_millis(250), move || {
                let Some(app) = weak.upgrade() else { return glib::ControlFlow::Break };
                let notices = isomar_app::host::poll(&app);
                actions::announce(&app, &notices);
                if following.get()
                    && let Some(p) = app.session.borrow().own.pos
                {
                    app.canvas.center_on(p);
                }
                app.tick();
                update();
                glib::ControlFlow::Continue
            });
        }
        {
            let app = app.clone();
            window.connect_close_request(move |_| {
                isomar_app::host::shutdown(&app);
                glib::Propagation::Proceed
            });
        }
        super::chart_widget().grab_focus();
        isomar_app::debug::seed_demo(&app);
        isomar_app::debug::seed_perf(&app);
        if debug::underway() {
            isomar_app::vm::underway::set(&app, true);
        }
        {
            // `ISOMAR_ALARM` needs a position: wait for the demo boat's first fix.
            let app = Rc::downgrade(&app);
            glib::timeout_add_local_once(Duration::from_millis(1500), move || {
                if let Some(app) = app.upgrade() {
                    isomar_app::debug::fake_alarm(&app);
                }
            });
        }
        app.refresh();
        debug::seed_weather(&app);
        debug::open_dialog(&app);
        super::welcome::open_if_first_run(&app);
        debug::screenshot_if_requested(&app, &window);
        mw
    }

    /// The sidebar floats over the chart when the window is narrow (the breakpoint's setter) or the
    /// boat is under way, and pushes the chart aside otherwise.
    fn layout(&self) {
        let collapsed = self.narrow.get() || self.underway.get();
        if self.split.is_collapsed() != collapsed {
            self.split.set_collapsed(collapsed);
        }
    }

    fn set_underway(&self, on: bool) {
        self.underway.set(on);
        if on {
            self.sidebar_before.set(self.split.shows_sidebar());
            self.split.set_show_sidebar(false);
            self.window.add_css_class("underway");
            self.cluster.set_following(true);
        } else {
            self.window.remove_css_class("underway");
            self.split.set_show_sidebar(self.sidebar_before.get() && !self.narrow.get());
        }
        self.layout();
        self.window.upcast_ref::<gtk::Widget>().announce(&if on { gettext("Under way layout") } else { gettext("Planning layout") }, gtk::AccessibleAnnouncementPriority::Medium);
        if let Some(a) = self.window.lookup_action("underway").and_downcast::<gio::SimpleAction>() {
            a.set_state(&on.to_variant());
        }
    }

    fn show_page(&self, name: &str) {
        self.sidebar.show(name);
        self.split.set_show_sidebar(true);
    }

    fn push(&self, name: &str, page: Option<adw::NavigationPage>) {
        if let Some(p) = page {
            self.sidebar.push(name, &p);
            self.split.set_show_sidebar(true);
        }
    }

    /// Single letters reach the chart only while it has focus: M measures, F follows the boat.
    fn install_keys(self: &Rc<Self>, chart: &gtk::Widget) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Bubble);
        let m = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, mods| {
            let Some(m) = m.upgrade() else { return glib::Propagation::Proceed };
            if mods.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK | gdk::ModifierType::SUPER_MASK) {
                return glib::Propagation::Proceed;
            }
            match key.to_lower() {
                gdk::Key::m => {
                    let _ = WidgetExt::activate_action(&m.window, "win.measure", None);
                    glib::Propagation::Stop
                }
                gdk::Key::f => {
                    m.cluster.toggle_following();
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        chart.add_controller(keys);
    }

    fn install_actions(self: &Rc<Self>, application: &adw::Application, settings: &Settings) {
        let group = gio::SimpleActionGroup::new();
        let canvas = self.app.canvas.clone();

        // The palette: a radio action.
        let mode = gio::SimpleAction::new_stateful("mode", Some(glib::VariantTy::STRING), &canvas.mode().as_str().to_variant());
        {
            let canvas = canvas.clone();
            mode.connect_activate(move |a, p| {
                let Some(s) = p.and_then(|p| p.get::<String>()) else { return };
                if let Ok(m) = s.parse::<Mode>() {
                    canvas.set_mode(m);
                    super::theme::apply(m);
                    a.set_state(&s.to_variant());
                }
            });
        }
        group.add_action(&mode);

        // One boolean action per layer.
        for (id, _) in toggleable_layers() {
            let visible = ticked(settings, id);
            let a = gio::SimpleAction::new_stateful(&format!("layer-{id}"), None, &visible.to_variant());
            let canvas = canvas.clone();
            a.connect_activate(move |a, _| {
                let on = !a.state().and_then(|s| s.get::<bool>()).unwrap_or(true);
                a.set_state(&on.to_variant());
                canvas.set_layer_visible(id, on);
            });
            group.add_action(&a);
        }

        // The under-way layout, by hand.
        let underway = gio::SimpleAction::new_stateful("underway", None, &false.to_variant());
        {
            let app = Rc::downgrade(&self.app);
            underway.connect_activate(move |a, _| {
                let on = !a.state().and_then(|s| s.get::<bool>()).unwrap_or(false);
                if let Some(app) = app.upgrade() {
                    isomar_app::vm::underway::set(&app, on);
                }
            });
        }
        group.add_action(&underway);

        let simple = |name: &str, f: Box<dyn Fn(&Rc<MainWindow>)>| {
            let a = gio::SimpleAction::new(name, None);
            let m = Rc::downgrade(self);
            a.connect_activate(move |_, _| {
                if let Some(m) = m.upgrade() {
                    f(&m);
                }
            });
            group.add_action(&a);
        };
        simple("connections", Box::new(|m| dialogs::connections(&m.app)));
        simple("monitor", Box::new(|m| dialogs::monitor(&m.app)));
        simple("polar", Box::new(|m| super::polar::open(&m.app)));
        simple("performance", Box::new(|m| m.push("instruments", Some(super::pages::performance::page(&m.app)))));
        simple("preferences", Box::new(|m| super::preferences::open(&m.app)));
        simple("import", Box::new(|m| super::gpx::import_gpx(&m.app)));
        simple("export", Box::new(|m| super::gpx::export_gpx(&m.app)));
        simple("about", Box::new(|_| about(&super::chart_widget())));
        simple("show-help-overlay", Box::new(|m| shortcuts(&m.window)));
        simple("toggle-sidebar", Box::new(|m| m.split.set_show_sidebar(!m.split.shows_sidebar())));
        simple("new-route", Box::new(|m| actions::start_route(&m.app, None)));
        simple("stop-navigation", Box::new(|m| isomar_app::vm::underway::stop(&m.app)));
        simple("record-track", Box::new(|m| actions::toggle_recording(&m.app)));
        simple("zoom-in", Box::new(|m| overlays::zoom_by(&m.app, 1.0)));
        simple("zoom-out", Box::new(|m| overlays::zoom_by(&m.app, -1.0)));
        simple("follow", Box::new(|m| m.cluster.toggle_following()));
        simple("measure", Box::new(|m| {
            // A second press ends the measurement.
            if m.app.measure.borrow().is_some() {
                *m.app.measure.borrow_mut() = None;
                m.app.refresh();
            } else {
                let at = m.app.pointer.get().unwrap_or_else(|| m.app.canvas.view_state().center());
                actions::begin_measure(&m.app, at);
            }
        }));
        simple("show-activity", Box::new(|m| m.header.show_activity()));
        simple("get-weather", Box::new(|m| {
            let a = crate::core::weather::planner::Area::from_bounds(&m.app.canvas.view_info().bounds());
            super::weather::dialog::open(&m.app, a);
        }));
        simple("tides-here", Box::new(|m| super::popovers::show_tides(&m.app, m.app.canvas.view_state().center())));
        simple("anchor-here", Box::new(|m| {
            let own = m.app.session.borrow().own.pos;
            match own {
                Some(p) => {
                    let r = m.app.settings.borrow().anchor_radius_nm;
                    actions::set_anchor(&m.app, p, r);
                }
                None => m.app.toast(&gettext("No position fix, so there is nothing to watch")),
            }
        }));
        simple("anchor-clear", Box::new(|m| {
            m.app.session.borrow_mut().clear_anchor();
            m.app.refresh();
        }));

        // Pages of the sidebar.
        let page = gio::SimpleAction::new("page", Some(glib::VariantTy::STRING));
        {
            let m = Rc::downgrade(self);
            page.connect_activate(move |_, p| {
                if let (Some(m), Some(name)) = (m.upgrade(), p.and_then(|p| p.get::<String>())) {
                    m.show_page(sidebar::page_name(&name));
                }
            });
        }
        group.add_action(&page);
        let speed = gio::SimpleAction::new("timeline-speed", Some(glib::VariantTy::STRING));
        {
            let app = Rc::downgrade(&self.app);
            speed.connect_activate(move |_, p| {
                if let (Some(app), Some(x)) = (app.upgrade(), p.and_then(|p| p.get::<String>()).and_then(|s| s.parse::<f64>().ok())) {
                    isomar_app::vm::timeline::set_speed(&app, x);
                }
            });
        }
        group.add_action(&speed);
        header::install_actions(&group, &self.app);

        // What a row, a detail or a popover asks for, by the id of what it is about.
        let by_id = |name: &str, f: Box<dyn Fn(&Rc<MainWindow>, i64)>| {
            let a = gio::SimpleAction::new(name, Some(glib::VariantTy::INT64));
            let m = Rc::downgrade(self);
            a.connect_activate(move |_, p| {
                if let (Some(m), Some(id)) = (m.upgrade(), p.and_then(|p| p.get::<i64>())) {
                    f(&m, id);
                }
            });
            group.add_action(&a);
        };
        by_id("activate-route", Box::new(|m, id| isomar_app::vm::underway::start_route(&m.app, id)));
        by_id("edit-route", Box::new(|m, id| actions::edit_route(&m.app, id)));
        by_id("reverse-route", Box::new(|m, id| actions::reverse_route(&m.app, id)));
        by_id("toggle-route", Box::new(|m, id| isomar_app::vm::passage::toggle_route_visible(&m.app, id)));
        by_id("export-route", Box::new(|m, id| super::gpx::export_route_gpx(&m.app, id)));
        by_id("delete-route", Box::new(|m, id| confirm_then(m, &gettext("Delete this route?"), move |a| actions::delete_route(a, id))));
        by_id("route-detail", Box::new(|m, id| {
            isomar_app::vm::passage::zoom_to_route(&m.app, id);
            m.push("passage", super::pages::details::route(&m.app, id));
        }));
        by_id("zoom-waypoint", Box::new(|m, id| isomar_app::vm::passage::zoom_to_waypoint(&m.app, id)));
        by_id("goto-waypoint", Box::new(|m, id| {
            let w = m.app.session.borrow().waypoints.iter().find(|w| w.id == id).map(|w| (w.pos, w.name.clone()));
            if let Some((p, n)) = w {
                actions::goto(&m.app, p, &n);
            }
        }));
        by_id("waypoint-detail", Box::new(|m, id| m.push("passage", super::pages::details::waypoint(&m.app, id))));
        by_id("delete-waypoint", Box::new(|m, id| confirm_then(m, &gettext("Delete this mark?"), move |a| actions::delete_waypoint(a, id))));
        by_id("zoom-track", Box::new(|m, id| isomar_app::vm::passage::zoom_to_track(&m.app, id)));
        by_id("toggle-track", Box::new(|m, id| isomar_app::vm::passage::toggle_track_visible(&m.app, id)));
        by_id("delete-track", Box::new(|m, id| confirm_then(m, &gettext("Delete this track?"), move |a| actions::delete_track(a, id))));
        by_id("show-vessel", Box::new(|m, id| isomar_app::vm::traffic::show_vessel(&m.app, id as u32)));
        by_id("vessel-detail", Box::new(|m, id| m.push("traffic", super::pages::details::vessel(&m.app, id as u32))));
        by_id("acknowledge-vessel", Box::new(|m, id| {
            m.app.session.borrow_mut().acknowledge_ais(id as u32);
            m.app.refresh();
        }));
        by_id("weather-routing", Box::new(|m, id| m.push("passage", super::pages::routing::page(&m.app, id, None))));
        by_id("weather-route", Box::new(|m, id| m.push("passage", super::pages::routing::page(&m.app, id, Some(super::pages::routing::Mode::Fixed)))));
        by_id("best-departure", Box::new(|m, id| m.push("passage", super::pages::routing::page(&m.app, id, Some(super::pages::routing::Mode::Window)))));
        // While its search runs, the auto route of a route shows the running jobs instead.
        by_id("auto-route", Box::new(|m, id| if m.app.jobs.busy(&format!("autoroute:{id}")) { m.header.show_activity() } else { super::autoroute::open(&m.app, id) }));

        self.window.insert_action_group("win", Some(&group));
        for (action, keys) in [
            ("win.new-route", &["<Ctrl>n"][..]),
            ("win.page::passage", &["<Ctrl>1"]),
            ("win.page::weather", &["<Ctrl>2"]),
            ("win.page::traffic", &["<Ctrl>3"]),
            ("win.page::charts", &["<Ctrl>4"]),
            ("win.page::instruments", &["<Ctrl>5"]),
            ("win.underway", &["<Ctrl>u"]),
            ("win.get-weather", &["<Ctrl><Shift>w"]),
            ("win.toggle-sidebar", &["F9"]),
            ("win.preferences", &["<Ctrl>comma"]),
            ("win.connections", &["<Ctrl>k"]),
            ("win.zoom-in", &["<Ctrl>plus", "<Ctrl>equal"]),
            ("win.zoom-out", &["<Ctrl>minus"]),
            ("win.import", &["<Ctrl>o"]),
            ("win.export", &["<Ctrl>e"]),
            ("win.show-help-overlay", &["<Ctrl>question"]),
        ] {
            application.set_accels_for_action(action, keys);
        }
    }
}

/// Asks before deleting, then does it.
fn confirm_then(m: &MainWindow, heading: &str, then: impl FnOnce(&Rc<AppState>) + 'static) {
    let (app, heading) = (m.app.clone(), heading.to_string());
    super::spawn(async move {
        if super::confirm(&heading, "", &gettext("Delete"), true).await {
            then(&app);
        }
    });
}

/// The keyboard shortcuts window. Deprecated in GTK 4.18 for libadwaita 1.8's dialog, which 1.7
/// does not have.
#[allow(deprecated)]
fn shortcuts(parent: &adw::ApplicationWindow) {
    let section = gtk::ShortcutsSection::builder().section_name("main").build();
    let groups: [(String, Vec<(&str, String)>); 3] = [
        (
            gettext("Isomar"),
            vec![
                ("<Ctrl>plus", gettext("Zoom in")),
                ("<Ctrl>minus", gettext("Zoom out")),
                ("m", gettext("Measure (with the chart focused)")),
                ("f", gettext("Follow the boat (with the chart focused)")),
                ("Escape", gettext("Cancel drawing or measuring")),
            ],
        ),
        (
            gettext("Passage"),
            vec![("<Ctrl>n", gettext("New route")), ("<Ctrl>u", gettext("Under way layout")), ("<Ctrl><Shift>w", gettext("Get weather")), ("<Ctrl>o", gettext("Import GPX")), ("<Ctrl>e", gettext("Export GPX"))],
        ),
        (
            gettext("Window"),
            vec![
                ("F9", gettext("Show or hide the sidebar")),
                ("<Ctrl>1", gettext("Passage")),
                ("<Ctrl>2", gettext("Weather")),
                ("<Ctrl>3", gettext("Traffic")),
                ("<Ctrl>4", gettext("Charts")),
                ("<Ctrl>5", gettext("Instruments")),
                ("<Ctrl>comma", gettext("Preferences")),
                ("<Ctrl>k", gettext("Connections")),
                ("<Ctrl>question", gettext("Keyboard shortcuts")),
            ],
        ),
    ];
    for (title, items) in groups {
        let g = gtk::ShortcutsGroup::builder().title(title).build();
        for (accel, label) in items {
            g.add_shortcut(&gtk::ShortcutsShortcut::builder().accelerator(accel).title(label).build());
        }
        section.add_group(&g);
    }
    let w = gtk::ShortcutsWindow::builder().transient_for(parent).modal(false).build();
    w.add_section(&section);
    w.present();
}

fn about(parent: &impl IsA<gtk::Widget>) {
    let d = adw::AboutDialog::builder()
        .application_name(gettext("Isomar"))
        .application_icon("eu._6px.isomar")
        .developer_name("Isomar contributors")
        .version(env!("CARGO_PKG_VERSION"))
        .license_type(gtk::License::Gpl30)
        .comments(gettext("A chart plotter for sailors.\n\nNot for navigation: charts built from open data are for planning only. Use official charts for navigation."))
        .build();
    d.present(Some(parent));
}

#[cfg(test)]
mod tests {
    use super::*;
    use isomar_app::emodnet;

    #[test]
    fn download_layers_start_off_and_the_rest_start_on() {
        let mut s = Settings::default();
        assert!(!ticked(&s, emodnet::LAYER), "no download until asked for");
        assert!(ticked(&s, "land"));
        assert!(ticked(&s, isomar_app::dem::LAYER), "relief is on until switched off");
        s.optional_layers.push(emodnet::LAYER.to_string());
        s.hidden_layers.push("land".to_string());
        assert!(ticked(&s, emodnet::LAYER));
        assert!(!ticked(&s, "land"));
    }
}
