//! Weather routing, a page pushed in Passage from a route: one place for "leave at this time" and
//! "find the best time", which share the passage, the forecast and the boat. A fixed departure
//! runs `isomar_app::flows::routing::run` and the preview card on the chart follows. A window of
//! departures runs `isomar_app::flows::departures::run`; its results (`vm::departures`) fill the
//! foot of the page: the cost of each departure on a timeline and the best ten. Picking one
//! previews it on the chart (`flows::departures::preview`), the best one at once.

use super::column;
use crate::config::WindowPrefs;
use crate::core::routing::window::{MAX_WEIGHT, Weights};
use crate::i18n::{gettext, gettext_f};
use crate::ui::app::AppState;
use crate::ui::form::Rows;
use crate::ui::spawn;
use adw::prelude::*;
use isomar_app::chips::{Chip, Icon};
use isomar_app::flows::departures::{STEPS_H, WindowRequest, next_slot, presets};
use isomar_app::flows::routing::{BoatParams, RouteRequest};
use isomar_app::tokens::{self, Token};
use isomar_app::vm::departures::Departures;
use isomar_app::core::weather::source::Source;
use chrono::{Local, Utc};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

/// Whether the page routes one departure or looks for the best one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Fixed,
    Window,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Mode::Fixed => "fixed",
            Mode::Window => "window",
        }
    }
}

thread_local! {
    /// The page shown, which the window's update feeds with the results.
    static SHOWN: RefCell<Weak<RoutingPage>> = const { RefCell::new(Weak::new()) };
    /// The mode chosen last, for the next page.
    static LAST_MODE: Cell<Mode> = const { Cell::new(Mode::Fixed) };
}

/// The forecast rows: a row per model, ticked to route on it, with what is loaded of it (how far it
/// goes and how old it is) or that it will be downloaded. Several ticked: each is routed on and they
/// are compared. Returns the models ticked and what brings the rows up to date.
fn forecast_rows(app: &Rc<AppState>, rows: &Rows) -> (Rc<dyn Fn() -> Vec<Source>>, Rc<dyn Fn()>) {
    rows.group(&gettext("Forecasts"));
    let chosen = {
        let s = app.settings.borrow();
        if s.routing_sources.is_empty() { vec![app.weather.borrow().source] } else { s.routing_sources.clone() }
    };
    let mut checks = Vec::new();
    for src in Source::ALL {
        let row = rows.line("", src.label(), "");
        row.set_subtitle_lines(3);
        let check = gtk::CheckButton::builder().active(chosen.contains(&src)).valign(gtk::Align::Center).build();
        row.add_prefix(&check);
        row.set_activatable_widget(Some(&check));
        let badge = gtk::Label::builder().valign(gtk::Align::Center).css_classes(["stale-badge"]).build();
        row.add_suffix(&badge);
        checks.push((src, check, row, badge));
    }
    let checks = Rc::new(checks);
    let describe: Rc<dyn Fn()> = {
        let (app, checks) = (Rc::downgrade(app), checks.clone());
        Rc::new(move || {
            let Some(app) = app.upgrade() else { return };
            let w = app.weather.borrow();
            for (src, _, row, badge) in checks.iter() {
                match w.store_of(*src).and_then(|st| st.latest_run().zip(st.wind_range())) {
                    Some((run, (_, end))) => {
                        let end = end.with_timezone(&Local).format("%a %d %b %H:%M").to_string();
                        row.set_subtitle(&gettext_f("Run {run}, until {end}", &[("run", &run.format("%H UTC").to_string()), ("end", &end)]));
                        badge.set_label(&gettext_f("{h} h old", &[("h", &(Utc::now() - run).num_hours().max(0).to_string())]));
                        let stale = w.store_of(*src).and_then(isomar_app::weather::layer::forecast_age).is_some_and(|a| a > chrono::Duration::hours(isomar_app::weather::layer::STALE_AFTER_HOURS));
                        if stale { badge.remove_css_class("fresh") } else { badge.add_css_class("fresh") }
                        badge.set_visible(true);
                    }
                    None => {
                        row.set_subtitle(&if src.crops_on_server() {
                            gettext("None loaded: downloaded first, a small download.")
                        } else {
                            gettext("None loaded: downloaded first, the whole globe for each step, so tens of megabytes for a long passage.")
                        });
                        badge.set_visible(false);
                    }
                }
            }
        })
    };
    describe();
    let selected: Rc<dyn Fn() -> Vec<Source>> = {
        let checks = checks.clone();
        Rc::new(move || checks.iter().filter(|(_, c, _, _)| c.is_active()).map(|(s, _, _, _)| *s).collect())
    };
    (selected, describe)
}

/// Keeps the models ticked for next time; none ticked is the one shown.
fn remember_models(app: &AppState, models: Vec<Source>) -> Vec<Source> {
    app.settings.borrow_mut().routing_sources = models.clone();
    app.apply_settings();
    models
}

/// The boat's rows: the polar, the engine and the wind limit, with the performance folded under
/// Advanced.
struct BoatRows {
    polar: adw::ActionRow,
    perf: adw::SpinRow,
    engine: adw::SwitchRow,
    motor: adw::SpinRow,
    max_wind: adw::SpinRow,
}

fn boat_rows(rows: &Rows, polar_name: &str, performance_pct: f64) -> BoatRows {
    rows.group(&gettext("Boat"));
    let polar = rows.line("isomar-polar-symbolic", &gettext("Polar"), polar_name);
    let edit = gtk::Button::builder().label(gettext("Edit…")).valign(gtk::Align::Center).action_name("win.polar").build();
    polar.add_suffix(&edit);
    let engine = rows.switch(&gettext("Use the engine in light wind"), "", false);
    let motor = rows.number(&gettext("Engine speed"), &gettext("kn"), (2.0, 12.0, 0.5), 1, 5.0);
    engine.bind_property("active", &motor, "sensitive").sync_create().build();
    let max_wind = rows.number(&gettext("Avoid wind above"), &gettext("kn"), (15.0, 60.0, 1.0), 0, 35.0);
    let summary = |pct: f64| gettext_f("Performance {pct} % of the polar", &[("pct", &format!("{pct:.0}"))]);
    let advanced = rows.advanced(&summary(performance_pct));
    let perf = rows.number(&gettext("Performance"), "%", (30.0, 150.0, 1.0), 0, performance_pct);
    perf.set_subtitle(&gettext("Of the polar's speeds: lower for a foul bottom, higher for a racing crew. The performance log can measure it."));
    perf.connect_value_notify(move |p| advanced.set_subtitle(&summary(p.value())));
    BoatRows { polar, perf, engine, motor, max_wind }
}

impl BoatRows {
    fn params(&self) -> BoatParams {
        BoatParams { performance_pct: self.perf.value(), use_engine: self.engine.is_active(), motor_kn: self.motor.value(), max_wind_kn: self.max_wind.value() }
    }
}

/// The weights' sliders and thresholds.
struct WeightRows {
    time: gtk::Scale,
    upwind: gtk::Scale,
    engine: gtk::Scale,
    strong_wind: gtk::Scale,
    strong_wind_kn: adw::SpinRow,
    gusts: gtk::Scale,
    gust_kn: adw::SpinRow,
    waves: gtk::Scale,
    wave_m: adw::SpinRow,
    night: gtk::Scale,
    manoeuvres: gtk::Scale,
    waiting: gtk::Scale,
    holding: gtk::Scale,
    wind_against_tide: gtk::Scale,
}

impl WeightRows {
    fn scales(&self) -> [&gtk::Scale; 11] {
        [&self.time, &self.upwind, &self.engine, &self.strong_wind, &self.gusts, &self.waves, &self.night, &self.manoeuvres, &self.waiting, &self.holding, &self.wind_against_tide]
    }

    fn set(&self, w: &Weights) {
        for (s, v) in self.scales().into_iter().zip([w.time, w.upwind, w.engine, w.strong_wind, w.gusts, w.waves, w.night, w.manoeuvres, w.waiting, w.holding, w.wind_against_tide]) {
            s.set_value(v);
        }
        self.strong_wind_kn.set_value(w.strong_wind_kn);
        self.gust_kn.set_value(w.gust_kn);
        self.wave_m.set_value(w.wave_m);
    }

    fn get(&self) -> Weights {
        Weights {
            time: self.time.value(),
            upwind: self.upwind.value(),
            engine: self.engine.value(),
            strong_wind: self.strong_wind.value(),
            strong_wind_kn: self.strong_wind_kn.value(),
            gusts: self.gusts.value(),
            gust_kn: self.gust_kn.value(),
            waves: self.waves.value(),
            wave_m: self.wave_m.value(),
            night: self.night.value(),
            manoeuvres: self.manoeuvres.value(),
            waiting: self.waiting.value(),
            holding: self.holding.value(),
            wind_against_tide: self.wind_against_tide.value(),
        }
    }
}

/// A row with its title over a slider from "doesn't matter" to "matters a lot", so that it fits
/// the sidebar's width.
fn slider(rows: &Rows, title: &str, subtitle: &str) -> gtk::Scale {
    let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).margin_top(8).margin_bottom(4).margin_start(12).margin_end(12).build();
    b.append(&gtk::Label::builder().label(title).xalign(0.0).build());
    b.append(&gtk::Label::builder().label(subtitle).xalign(0.0).wrap(true).css_classes(["caption", "dim-label"]).build());
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, MAX_WEIGHT, 0.25);
    scale.set_draw_value(false);
    scale.set_hexpand(true);
    scale.add_mark(0.0, gtk::PositionType::Bottom, Some(&gettext("No")));
    scale.add_mark(MAX_WEIGHT, gtk::PositionType::Bottom, Some(&gettext("A lot")));
    scale.update_property(&[gtk::accessible::Property::Label(title)]);
    b.append(&scale);
    rows.add(&adw::PreferencesRow::builder().child(&b).activatable(false).build());
    scale
}

/// "What matters": a preset, and the sliders folded under it.
fn weight_rows(rows: &Rows, initial: &Weights) -> (adw::PreferencesGroup, Rc<WeightRows>) {
    let group = rows.group(&gettext("What matters"));
    let names: Vec<String> = presets().into_iter().map(|p| p.0).chain([gettext("Custom")]).collect();
    let preset_of = |w: &Weights| presets().iter().position(|p| p.1 == *w).unwrap_or(presets().len());
    let combo = rows.combo(&gettext("Preset"), &names, None);
    combo.set_selected(preset_of(initial) as u32);
    let fold = rows.fold(&gettext("Adjust"), &gettext("How much each thing counts"));
    fold.add_row(&gtk::Label::builder().label(gettext("A passage costs its hours at sea, plus each hour of what you would rather avoid counted again as many times as its slider says (up to 4 more).")).wrap(true).xalign(0.0).css_classes(["caption", "dim-label"]).margin_top(8).margin_bottom(8).margin_start(12).margin_end(12).build());
    let w = Rc::new(WeightRows {
        time: slider(rows, &gettext("Time at sea"), &gettext("Shorter passages")),
        upwind: slider(rows, &gettext("Upwind"), &gettext("Hours close-hauled")),
        engine: slider(rows, &gettext("Engine"), &gettext("Hours motoring")),
        strong_wind: slider(rows, &gettext("Strong wind"), &gettext("Hours in wind at or above the limit below")),
        strong_wind_kn: rows.number(&gettext("Strong wind from"), &gettext("kn"), (10.0, 50.0, 1.0), 0, initial.strong_wind_kn),
        gusts: slider(rows, &gettext("Gusts"), &gettext("Hours with gusts at or above the limit below")),
        gust_kn: rows.number(&gettext("Strong gusts from"), &gettext("kn"), (15.0, 70.0, 1.0), 0, initial.gust_kn),
        waves: slider(rows, &gettext("Rough sea"), &gettext("Hours with waves at or above the height below")),
        wave_m: rows.number(&gettext("Rough from"), &gettext("m of waves"), (0.5, 8.0, 0.5), 1, initial.wave_m),
        night: slider(rows, &gettext("Night"), &gettext("Hours in the dark")),
        manoeuvres: slider(rows, &gettext("Tacks and gybes"), &gettext("Each counts as half an hour at the first mark")),
        waiting: slider(rows, &gettext("Leaving soon"), &gettext("Hours waiting for the departure; later forecasts are less sure")),
        holding: slider(rows, &gettext("Waiting for the stream"), &gettext("Hours stemming a foul stream off a headland until it turns")),
        wind_against_tide: slider(rows, &gettext("Wind against tide"), &gettext("Hours of fresh wind against a strong stream: a short, steep sea")),
    });
    w.set(initial);
    // A preset sets the sliders; moving one makes it "Custom".
    let setting = Rc::new(Cell::new(false));
    {
        let (w, setting) = (Rc::downgrade(&w), setting.clone());
        combo.connect_selected_notify(move |c| {
            if let (Some((_, p)), Some(w)) = (presets().get(c.selected() as usize), w.upgrade()) {
                setting.set(true);
                w.set(p);
                setting.set(false);
            }
        });
    }
    let changed = {
        let (w, combo) = (Rc::downgrade(&w), combo.clone());
        move || {
            if !setting.get()
                && let Some(w) = w.upgrade()
            {
                combo.set_selected(preset_of(&w.get()) as u32);
            }
        }
    };
    for s in w.scales() {
        let changed = changed.clone();
        s.connect_value_changed(move |_| changed());
    }
    for s in [&w.strong_wind_kn, &w.gust_kn, &w.wave_m] {
        let changed = changed.clone();
        s.connect_value_notify(move |_| changed());
    }
    (group, w)
}

const AXIS_H: f64 = 26.0;

fn set(cr: &gtk::cairo::Context, token: Token) {
    let (r, g, b, a) = tokens::color(token, crate::ui::theme::token_theme());
    cr.set_source_rgba(r, g, b, a);
}

/// A bar with its top corners rounded, from the baseline `y0` up to `y1`.
fn bar(cr: &gtk::cairo::Context, x: f64, w: f64, y0: f64, y1: f64) {
    let r = 4.0f64.min(w / 2.0).min((y0 - y1) / 2.0).max(0.0);
    cr.new_sub_path();
    cr.move_to(x, y0);
    cr.line_to(x, y1 + r);
    cr.arc(x + r, y1 + r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
    cr.arc(x + w - r, y1 + r, r, 1.5 * std::f64::consts::PI, 0.0);
    cr.line_to(x + w, y0);
    cr.close_path();
}

/// The timeline of departures: night bands, days, and a bar per departure as tall as its cost,
/// in the `cost-*` ramp (the best ones `cost-best`).
fn draw_timeline(cr: &gtk::cairo::Context, width: f64, height: f64, d: &Departures) {
    let plot_h = height - AXIS_H;
    set(cr, Token::CostNightBand);
    for (a, b) in &d.nights {
        cr.rectangle(a * width, 0.0, (b - a) * width, plot_h);
    }
    let _ = cr.fill();
    let layout = pangocairo::functions::create_layout(cr);
    layout.set_font_description(Some(&gtk::pango::FontDescription::from_string("Sans 9")));
    for day in &d.days {
        set(cr, Token::Border);
        cr.set_line_width(1.0);
        let x = (day.x * width).round() + 0.5;
        cr.move_to(x, 0.0);
        cr.line_to(x, height);
        let _ = cr.stroke();
        layout.set_text(&day.label);
        let (tw, _) = layout.pixel_size();
        let noon = day.label_x * width;
        if noon - tw as f64 / 2.0 > 0.0 && noon + (tw as f64) / 2.0 < width {
            set(cr, Token::TextMuted);
            cr.move_to(noon - tw as f64 / 2.0, plot_h + 4.0);
            pangocairo::functions::show_layout(cr, &layout);
        }
    }
    for b in &d.bars {
        let slot = b.w * width;
        let gap = if slot > 6.0 { 2.0 } else { 0.0 };
        let (x, w) = (b.x * width + gap / 2.0, slot - gap);
        let y1 = plot_h - b.h * (plot_h - 6.0);
        bar(cr, x, w, plot_h, y1);
        set(cr, if !b.ok { Token::CostWorst } else if b.best { Token::CostBest } else if b.h < 0.6 { Token::CostMid } else { Token::CostWorst });
        let _ = cr.fill();
        if b.selected {
            set(cr, Token::Text);
            cr.set_line_width(2.0);
            bar(cr, x + 1.0, (w - 2.0).max(1.0), plot_h, y1.min(plot_h - 4.0) + 1.0);
            let _ = cr.stroke();
        }
        // Several forecasts: the spread of their costs as a line, and a dot each in its colour.
        let y = |h: f64| plot_h - h * (plot_h - 6.0);
        let cx = x + w / 2.0;
        if let Some([lo, hi]) = b.range {
            set(cr, Token::Text);
            cr.set_line_width(1.5);
            cr.move_to(cx, y(lo));
            cr.line_to(cx, y(hi));
            let _ = cr.stroke();
        }
        for dot in &b.dots {
            let token = Token::ALL.iter().copied().find(|t| t.name() == dot.colour).unwrap_or(Token::Route);
            cr.arc(cx, y(dot.h), (w / 3.0).clamp(1.5, 3.5), 0.0, std::f64::consts::TAU);
            set(cr, token);
            let _ = cr.fill_preserve();
            set(cr, Token::ViewBg);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
        }
    }
    set(cr, Token::BorderControl);
    cr.set_line_width(1.0);
    cr.move_to(0.0, plot_h + 0.5);
    cr.line_to(width, plot_h + 0.5);
    let _ = cr.stroke();
}

/// The page while it is shown.
pub struct RoutingPage {
    app: Weak<AppState>,
    route_id: i64,
    results: adw::PreferencesGroup,
    scroller: gtk::ScrolledWindow,
    summary: gtk::Label,
    area: gtk::DrawingArea,
    list: gtk::ListBox,
    compare: gtk::Button,
    /// The boat's polar row and performance, which follow the active polar when it changes (the
    /// editor is opened from this page), and the polar they show (`Settings::polar`).
    polar: adw::ActionRow,
    perf: adw::SpinRow,
    polar_id: RefCell<String>,
    shown: Rc<RefCell<Option<Departures>>>,
    /// While the list is set from the results, so that its selection is not taken as the user's.
    syncing: Cell<bool>,
    /// A preview is computing; another pick while it does runs once it ends.
    previewing: Cell<bool>,
    again: Cell<bool>,
    /// Brings the forecast row up to date.
    forecast_status: Rc<dyn Fn()>,
}

impl RoutingPage {
    /// Shows the active polar, and its performance, once another one is made active.
    fn follow_polar(&self, app: &AppState) {
        if *self.polar_id.borrow() == app.settings.borrow().polar {
            return;
        }
        *self.polar_id.borrow_mut() = app.settings.borrow().polar.clone();
        self.polar.set_subtitle(&isomar_app::polars::active_name(app));
        self.perf.set_value(isomar_app::flows::routing::default_performance(app));
    }

    /// Picks departure `i` (an index into the candidates) and previews it on the chart.
    fn pick(self: &Rc<Self>, i: usize) {
        let Some(app) = self.app.upgrade() else { return };
        isomar_app::vm::departures::select(&app, i);
        self.preview();
    }

    fn preview(self: &Rc<Self>) {
        if self.previewing.replace(true) {
            self.again.set(true);
            return;
        }
        let (me, app) = (self.clone(), self.app.clone());
        spawn(async move {
            while let Some(app) = app.upgrade() {
                isomar_app::flows::departures::preview(&app).await;
                if !me.again.replace(false) {
                    break;
                }
            }
            me.previewing.set(false);
        });
    }

    /// Shows `d`, the results of a scan of this route, or hides the results.
    fn update(self: &Rc<Self>, d: Option<&Departures>) {
        let Some(d) = d else {
            self.results.set_visible(false);
            *self.shown.borrow_mut() = None;
            return;
        };
        if self.shown.borrow().as_ref() == Some(d) {
            return;
        }
        let fresh = self.shown.borrow().is_none();
        self.results.set_visible(true);
        // Several forecasts: which dot is which.
        let legend: Vec<String> = d
            .legend
            .iter()
            .map(|l| {
                let token = Token::ALL.iter().copied().find(|t| t.name() == l.colour).unwrap_or(Token::Route);
                let (r, g, b, _) = tokens::color(token, crate::ui::theme::token_theme());
                format!("<span foreground=\"#{:02x}{:02x}{:02x}\">●</span> {}", (r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, gtk::glib::markup_escape_text(&l.label))
            })
            .collect();
        let text = gtk::glib::markup_escape_text(&d.summary).to_string();
        self.summary.set_markup(&if legend.is_empty() { text } else { format!("{text}\n{}", legend.join("   ")) });
        self.compare.set_sensitive(d.can_compare);
        let rows_changed = self.shown.borrow().as_ref().is_none_or(|old| old.rows.iter().map(|r| (&r.title, &r.figures, &r.why, &r.models)).ne(d.rows.iter().map(|r| (&r.title, &r.figures, &r.why, &r.models))));
        self.syncing.set(true);
        if rows_changed {
            self.list.remove_all();
            for r in &d.rows {
                let lines: Vec<&[isomar_app::chips::Chip]> = [r.figures.as_slice(), r.why.as_slice(), r.models.as_slice()].into_iter().filter(|l| !l.is_empty()).collect();
                let row = crate::ui::chips::row_lines(None, &r.title, &lines, true);
                self.list.append(&row);
            }
        }
        match d.rows.iter().position(|r| r.selected) {
            Some(k) => self.list.select_row(self.list.row_at_index(k as i32).as_ref()),
            None => self.list.unselect_all(),
        }
        self.syncing.set(false);
        *self.shown.borrow_mut() = Some(d.clone());
        self.area.queue_draw();
        if fresh {
            // New results: scrolled into view, and the best departure goes on the chart at once.
            let (scroller, results) = (self.scroller.clone(), self.results.clone());
            // After the next layout, once the group shown has its place.
            gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
                if let Some(p) = results.compute_point(&scroller.child().unwrap(), &gtk::graphene::Point::new(0.0, 0.0)) {
                    scroller.vadjustment().set_value(p.y() as f64 - 12.0);
                }
            });
            if d.rows.iter().any(|r| r.selected) {
                self.preview();
            }
        }
    }
}

/// Feeds the page shown, if any, with the best-departure results of its route.
pub fn update(app: &AppState) {
    let Some(page) = SHOWN.with(|s| s.borrow().upgrade()) else { return };
    page.follow_polar(app);
    (page.forecast_status)();
    let ours = app.departures.borrow().as_ref().is_some_and(|d| d.source.id == page.route_id);
    page.update(if ours { isomar_app::vm::departures::departures(app) } else { None }.as_ref());
}

/// The page for route `route_id`, starting in `mode` (or the mode used last, or the best-departure
/// mode while there are results for this route).
pub fn page(app: &Rc<AppState>, route_id: i64, mode: Option<Mode>) -> Option<adw::NavigationPage> {
    let r = app.session.borrow().routes.iter().find(|r| r.id == route_id).cloned()?;
    if r.points.len() < 2 {
        return None;
    }
    let has_results = app.departures.borrow().as_ref().is_some_and(|d| d.source.id == route_id);
    let mode = mode.unwrap_or_else(|| if has_results { Mode::Window } else { LAST_MODE.with(|m| m.get()) });
    let prefs = app.settings.borrow().weather_window.clone();
    let content = column();
    let rows = Rows::new(&content);

    let nm = isomar_app::core::nav::calc::total_distance_nm(&r);
    let ends = gettext_f("{from} to {to}", &[("from", &r.points[0].name), ("to", &r.points[r.points.len() - 1].name)]);
    rows.chip_line("isomar-route-symbolic", &r.name, &[Chip::plain(ends), Chip::with(Icon::Distance, gettext_f("{d} nm", &[("d", &format!("{nm:.1}"))]))]);

    // When: one departure, or the best of a window of them.
    let toggles = adw::ToggleGroup::builder().homogeneous(true).build();
    toggles.add(adw::Toggle::builder().name(Mode::Fixed.name()).label(gettext("At a time")).build());
    toggles.add(adw::Toggle::builder().name(Mode::Window.name()).label(gettext("Best time")).build());
    toggles.set_active_name(Some(mode.name()));
    content.append(&toggles);
    rows.group("");
    let leave = rows.datetime(&gettext("Leave at"), Some(r.departure.unwrap_or_else(Utc::now)), false);
    let earliest = rows.datetime(&gettext("Earliest"), Some(next_slot(prefs.step_hours).with_timezone(&Utc)), false);
    let days = rows.number(&gettext("Over"), &gettext("days"), (1.0, 14.0, 1.0), 0, prefs.days);
    let steps: Vec<String> = STEPS_H.iter().map(|h| gettext_f("Every {h} h", &[("h", &h.to_string())])).collect();
    let step_now = steps[STEPS_H.iter().position(|h| *h == prefs.step_hours).unwrap_or(1)].clone();
    let step = rows.combo(&gettext("Try a departure"), &steps, Some(&step_now));
    let (models, forecast_status) = forecast_rows(app, &rows);
    // Currents: one switch for every model, with what covers the passage.
    rows.group(&gettext("Currents"));
    let points: Vec<_> = r.points.iter().map(|p| p.pos).collect();
    let choice = isomar_app::flows::routing::currents_choice(app, &points);
    let currents = rows.switch(&gettext("Sail in the current"), &choice.detail, choice.on);
    {
        let app = Rc::downgrade(app);
        currents.connect_active_notify(move |row| {
            let Some(app) = app.upgrade() else { return };
            isomar_app::flows::routing::set_route_with(&app, row.is_active());
            row.set_subtitle(&isomar_app::flows::routing::currents_choice(&app, &points).detail);
        });
    }
    rows.group("");
    let ranks = [gettext("The average of the forecasts"), gettext("The worst forecast")];
    let rank = rows.combo(&gettext("Rank departures by"), &ranks, Some(&ranks[prefs.cautious as usize]));
    let (matters, weights) = weight_rows(&rows, &prefs.weights);
    let boat = boat_rows(&rows, &isomar_app::polars::active_name(app), isomar_app::flows::routing::default_performance(app));

    // Off, with a spinner, while a route is computed; pressed then, it shows the running jobs.
    let (compute, compute_label) = crate::ui::busy::button(app, "route", "");
    compute.add_css_class("suggested-action");
    compute.add_css_class("pill");
    compute.set_halign(gtk::Align::Center);
    content.append(&compute);
    rows.planning_only();

    // The results of a best-departure search.
    let results = adw::PreferencesGroup::builder().title(gettext("Best departures")).visible(false).build();
    let compare = gtk::Button::builder().label(gettext("Compare")).tooltip_text(gettext("Compare the statistics of the best three and the selected departure")).css_classes(["flat"]).valign(gtk::Align::Center).build();
    results.set_header_suffix(Some(&compare));
    let summary = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["dim-label"]).margin_bottom(6).build();
    results.add(&summary);
    let area = gtk::DrawingArea::builder().content_height(120).hexpand(true).has_tooltip(true).margin_bottom(12).build();
    results.add(&area);
    let list = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::Single).build();
    results.add(&list);
    content.append(&results);

    let scroller = super::scrolled(&content);
    let page = Rc::new(RoutingPage {
        app: Rc::downgrade(app),
        route_id,
        results,
        scroller: scroller.clone(),
        summary,
        area: area.clone(),
        list: list.clone(),
        compare: compare.clone(),
        polar: boat.polar.clone(),
        perf: boat.perf.clone(),
        polar_id: RefCell::new(app.settings.borrow().polar.clone()),
        shown: Rc::default(),
        syncing: Cell::new(false),
        previewing: Cell::new(false),
        again: Cell::new(false),
        forecast_status,
    });
    {
        let shown = page.shown.clone();
        area.set_draw_func(move |_, cr, w, h| {
            if let Some(d) = shown.borrow().as_ref() {
                draw_timeline(cr, w as f64, h as f64, d);
            }
        });
    }
    {
        let shown = page.shown.clone();
        area.connect_query_tooltip(move |a, x, y, _, tip| {
            if (y as f64) > a.height() as f64 - AXIS_H {
                return false;
            }
            let f = x as f64 / a.width().max(1) as f64;
            let s = shown.borrow();
            let Some(b) = s.as_ref().and_then(|d| d.bars.iter().find(|b| f >= b.x && f < b.x + b.w)) else { return false };
            tip.set_text(Some(&b.tooltip));
            true
        });
    }
    {
        // A click on a bar picks that departure.
        let click = gtk::GestureClick::new();
        let p = Rc::downgrade(&page);
        click.connect_released(move |g, _, x, _| {
            let Some(p) = p.upgrade() else { return };
            let f = x / g.widget().map_or(1, |w| w.width().max(1)) as f64;
            let i = p.shown.borrow().as_ref().and_then(|d| d.bars.iter().position(|b| f >= b.x && f < b.x + b.w));
            if let Some(i) = i {
                p.pick(i);
            }
        });
        area.add_controller(click);
    }
    {
        let p = Rc::downgrade(&page);
        list.connect_row_selected(move |_, row| {
            let Some(p) = p.upgrade() else { return };
            if p.syncing.get() {
                return;
            }
            let i = row.and_then(|r| p.shown.borrow().as_ref().and_then(|d| d.rows.get(r.index() as usize).map(|x| x.index)));
            if let Some(i) = i {
                p.pick(i);
            }
        });
    }
    {
        let app = Rc::downgrade(app);
        compare.connect_clicked(move |_| {
            if let Some((heads, sections)) = app.upgrade().and_then(|a| isomar_app::vm::departures::compare(&a)) {
                let heads: Vec<String> = heads.iter().map(|h| format!("<b>{}</b>", gtk::glib::markup_escape_text(h))).collect();
                crate::ui::routing_stats::show_sections(&gettext("Route statistics"), &heads, &sections);
            }
        });
    }

    // The rows of the other mode are hidden.
    let window_only: Vec<gtk::Widget> = vec![earliest.row.clone().upcast(), days.clone().upcast(), step.clone().upcast(), matters.upcast(), rank.clone().upcast()];
    let show_mode = {
        let (leave, compute, page) = (leave.row.clone(), compute_label.clone(), Rc::downgrade(&page));
        move |m: Mode| {
            leave.set_visible(m == Mode::Fixed);
            for w in &window_only {
                w.set_visible(m == Mode::Window);
            }
            compute.set_label(&if m == Mode::Fixed { gettext("Compute the route") } else { gettext("Find the best departure") });
            if let Some(p) = page.upgrade() {
                p.results.set_visible(m == Mode::Window && p.shown.borrow().is_some());
            }
        }
    };
    show_mode(mode);
    let mode_of = |t: &adw::ToggleGroup| if t.active_name().as_deref() == Some(Mode::Window.name()) { Mode::Window } else { Mode::Fixed };
    {
        let show_mode = show_mode.clone();
        toggles.connect_active_name_notify(move |t| {
            let m = mode_of(t);
            LAST_MODE.with(|l| l.set(m));
            show_mode(m);
        });
    }

    {
        let (app, toggles) = (app.clone(), toggles.clone());
        compute.connect_clicked(move |b| {
            let boat = boat.params();
            let app = app.clone();
            if crate::ui::busy::pressed(&app, b, "route") || app.routing_busy() {
                return;
            }
            // Held from the click, so a second one cannot slip in before the flow starts.
            let busy = app.routing_guard();
            match mode_of(&toggles) {
                Mode::Fixed => {
                    let Some(depart) = leave.get() else { return };
                    let sources = remember_models(&app, models());
                    spawn(async move {
                        isomar_app::flows::routing::run(&app, RouteRequest { route_id, depart, boat, sources }).await;
                        drop(busy);
                    });
                }
                Mode::Window => {
                    let Some(earliest) = earliest.get() else { return };
                    let prefs = WindowPrefs { weights: weights.get(), days: days.value(), step_hours: STEPS_H[(step.selected() as usize).min(STEPS_H.len() - 1)], cautious: rank.selected() == 1 };
                    let sources = remember_models(&app, models());
                    spawn(async move {
                        isomar_app::flows::departures::run(&app, WindowRequest { route_id, earliest, prefs, boat, sources }).await;
                        drop(busy);
                    });
                }
            }
        });
    }
    if std::env::var_os("ISOMAR_AUTO_ACCEPT").is_some() {
        // Development aid: compute with the values as they are.
        let compute = compute.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || compute.emit_clicked());
    }

    SHOWN.with(|s| *s.borrow_mut() = Rc::downgrade(&page));
    update(app);
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&adw::HeaderBar::builder().show_end_title_buttons(false).show_start_title_buttons(false).build());
    tv.set_content(Some(&scroller));
    let nav = adw::NavigationPage::builder().title(gettext("Weather routing")).tag("weather-routing").child(&tv).build();
    // The page lives as long as its widgets: the handler holds it.
    nav.connect_hidden(move |_| {
        let _ = &page;
    });
    Some(nav)
}
