//! What floats over the chart (`design/ia.md`, window model): the map control cluster, the
//! navigating card and the docked dashboard under way, the router preview card, the forecast
//! readout, the timeline, and the footer strip under the chart. Each is drawn from a `vm` record and
//! rebuilt only when the record changes.

use super::app::AppState;
use crate::i18n::{gettext, gettext_f};
use adw::prelude::*;
use isomar_app::tokens::{self, Token};
use isomar_app::vm::footer::Footer;
use isomar_app::vm::router::Card;
use isomar_app::vm::timeline::Timeline;
use isomar_app::vm::performance::Switch;
use isomar_app::vm::underway::{NavCard, Readout};
use gtk::gdk;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

/// A card over the chart: opaque `card-bg`, never blurred.
pub fn card(orientation: gtk::Orientation, spacing: i32) -> gtk::Box {
    let b = gtk::Box::new(orientation, spacing);
    b.add_css_class("isomar-card");
    b
}

fn label(text: &str, classes: &[&str]) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).css_classes(classes.to_vec()).build()
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

/// Zooms the chart about its centre by `by` levels.
pub fn zoom_by(app: &AppState, by: f64) {
    let w = super::chart_widget();
    app.canvas.zoom_to(app.canvas.view_state().zoom + by, w.width() as f64 / 2.0, w.height() as f64 / 2.0);
}

// Map controls.

/// The cluster at the bottom right: follow the boat, layers, measure, zoom.
pub struct MapCluster {
    pub widget: gtk::Box,
    follow: gtk::ToggleButton,
    /// Set while the app (not the user) switches following, which needs no message.
    quiet: Rc<Cell<bool>>,
}

impl MapCluster {
    pub fn new(app: &Rc<AppState>, following: Rc<Cell<bool>>) -> Rc<MapCluster> {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
        b.add_css_class("map-cluster");
        b.set_halign(gtk::Align::End);
        b.set_valign(gtk::Align::End);
        b.set_margin_end(16);
        b.set_margin_bottom(16);
        let button = |icon: &str, tip: &str| {
            let btn = gtk::Button::builder().icon_name(icon).tooltip_text(tip).css_classes(["flat"]).build();
            btn.update_property(&[gtk::accessible::Property::Label(tip)]);
            btn
        };
        let follow = gtk::ToggleButton::builder().icon_name("isomar-follow-symbolic").tooltip_text(gettext("Keep the boat centred")).css_classes(["flat"]).build();
        follow.update_property(&[gtk::accessible::Property::Label(&gettext("Follow the boat"))]);
        let quiet = Rc::new(Cell::new(false));
        {
            let (app, following, quiet) = (Rc::downgrade(app), following.clone(), quiet.clone());
            follow.connect_toggled(move |f| {
                following.set(f.is_active());
                f.set_icon_name(if f.is_active() { "isomar-follow-on-symbolic" } else { "isomar-follow-symbolic" });
                f.set_tooltip_text(Some(&if f.is_active() { gettext("Stop following the boat") } else { gettext("Keep the boat centred") }));
                if f.is_active()
                    && let Some(app) = app.upgrade()
                {
                    let pos = app.session.borrow().own.pos;
                    match pos {
                        Some(p) => app.canvas.center_on(p),
                        None if !quiet.get() => app.toast(&gettext("No position fix yet")),
                        None => {}
                    }
                }
            });
        }
        let layers_btn = gtk::MenuButton::builder().icon_name("isomar-layers-symbolic").tooltip_text(gettext("Layers")).css_classes(["flat"]).build();
        layers_btn.set_direction(gtk::ArrowType::Left);
        {
            // The header's Layers popover is shared: the cluster opens its own copy of the list.
            let app = app.clone();
            layers_btn.set_create_popup_func(move |m| m.set_popover(Some(&super::header::layers_popover(&app))));
        }
        let measure = button("isomar-measure-symbolic", &gettext("Measure a distance and bearing (M)"));
        measure.set_action_name(Some("win.measure"));
        let zin = button("zoom-in-symbolic", &gettext("Zoom in"));
        let zout = button("zoom-out-symbolic", &gettext("Zoom out"));
        {
            let a = Rc::downgrade(app);
            zin.connect_clicked(move |_| {
                if let Some(a) = a.upgrade() {
                    zoom_by(&a, 1.0);
                }
            });
            let a = Rc::downgrade(app);
            zout.connect_clicked(move |_| {
                if let Some(a) = a.upgrade() {
                    zoom_by(&a, -1.0);
                }
            });
        }
        b.append(&follow);
        b.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        b.append(&layers_btn);
        b.append(&measure);
        b.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        b.append(&zin);
        b.append(&zout);
        Rc::new(MapCluster { widget: b, follow, quiet })
    }

    /// Follows the boat or stops, as the under-way layout switches.
    pub fn set_following(&self, on: bool) {
        if self.follow.is_active() != on {
            self.quiet.set(true);
            self.follow.set_active(on);
            self.quiet.set(false);
        }
    }

    /// Follows or stops at the user's word (a key).
    pub fn toggle_following(&self) {
        self.follow.set_active(!self.follow.is_active());
    }
}

// Under way.

pub struct NavigatingCard {
    pub widget: gtk::Box,
    number: gtk::Label,
    name: gtk::Label,
    grid: gtk::Grid,
    skip: gtk::Button,
    shown: RefCell<Option<NavCard>>,
}

impl NavigatingCard {
    pub fn new(app: &Rc<AppState>) -> Rc<NavigatingCard> {
        let widget = card(gtk::Orientation::Vertical, 10);
        widget.add_css_class("navigating-card");
        widget.set_halign(gtk::Align::Start);
        widget.set_visible(false);
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let number = gtk::Label::builder().css_classes(["waypoint-number", "numeric"]).valign(gtk::Align::Center).build();
        let names = gtk::Box::new(gtk::Orientation::Vertical, 0);
        names.append(&label(&gettext("Next waypoint"), &["caption-heading", "dim-label"]));
        let name = gtk::Label::builder().xalign(0.0).css_classes(["title-3"]).ellipsize(gtk::pango::EllipsizeMode::End).build();
        names.append(&name);
        head.append(&number);
        head.append(&names);
        widget.append(&head);
        let grid = gtk::Grid::builder().column_spacing(24).row_spacing(6).build();
        widget.append(&grid);
        let buttons = gtk::Box::builder().spacing(8).homogeneous(true).build();
        let skip = gtk::Button::builder().child(&adw::ButtonContent::builder().icon_name("media-skip-forward-symbolic").label(gettext("Skip")).build()).build();
        let stop = gtk::Button::builder().child(&adw::ButtonContent::builder().icon_name("media-playback-stop-symbolic").label(gettext("Stop")).build()).css_classes(["destructive-action"]).build();
        let a = Rc::downgrade(app);
        skip.connect_clicked(move |_| {
            if let Some(a) = a.upgrade() {
                isomar_app::vm::underway::skip(&a);
            }
        });
        let a = Rc::downgrade(app);
        stop.connect_clicked(move |_| {
            if let Some(a) = a.upgrade() {
                isomar_app::vm::underway::stop(&a);
            }
        });
        buttons.append(&skip);
        buttons.append(&stop);
        widget.append(&buttons);
        Rc::new(NavigatingCard { widget, number, name, grid, skip, shown: RefCell::new(None) })
    }

    pub fn update(&self, c: Option<&NavCard>) {
        if self.shown.borrow().as_ref() == c {
            return;
        }
        *self.shown.borrow_mut() = c.cloned();
        self.widget.set_visible(c.is_some());
        let Some(c) = c else { return };
        self.number.set_text(&c.number.to_string());
        self.name.set_text(&c.name);
        self.skip.set_sensitive(c.can_skip);
        while let Some(w) = self.grid.first_child() {
            self.grid.remove(&w);
        }
        for (i, cell) in c.cells.iter().enumerate() {
            let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
            b.append(&label(&cell.label, &["caption-heading", "dim-label"]));
            let v = gtk::Box::new(gtk::Orientation::Horizontal, 3);
            v.append(&label(&cell.value, &["readout-m"]));
            if !cell.unit.is_empty() {
                let u = label(&cell.unit, &["readout-unit"]);
                u.set_valign(gtk::Align::Baseline);
                v.append(&u);
            }
            b.append(&v);
            self.grid.attach(&b, (i % 3) as i32, (i / 3) as i32, 1, 1);
        }
    }
}

/// The dashboard pinned over the chart while planning (`Settings::dashboard_on_chart`). Under way the
/// instruments have a column of their own (`InstrumentColumn`).
pub struct Dashboard {
    pub widget: gtk::Box,
    shown: RefCell<Option<Vec<Readout>>>,
}

impl Dashboard {
    pub fn new() -> Rc<Dashboard> {
        let widget = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        widget.add_css_class("dashboard");
        widget.set_halign(gtk::Align::Start);
        widget.set_valign(gtk::Align::End);
        widget.set_visible(false);
        Rc::new(Dashboard { widget, shown: RefCell::new(None) })
    }

    pub fn update(&self, readouts: &[Readout]) {
        if self.shown.borrow().as_deref() == Some(readouts) {
            return;
        }
        clear(&self.widget);
        self.widget.set_visible(!readouts.is_empty());
        for r in readouts {
            let tile = card(gtk::Orientation::Vertical, 2);
            tile.add_css_class("instrument-tile");
            tile.append(&label(&r.label, &["caption-heading", "dim-label"]));
            let v = gtk::Box::new(gtk::Orientation::Horizontal, 3);
            v.append(&label(&r.value, &["readout-l"]));
            let u = label(&r.unit, &["readout-unit"]);
            u.set_valign(gtk::Align::Baseline);
            v.append(&u);
            tile.append(&v);
            if !r.note.is_empty() {
                tile.append(&super::chips::line(&r.note, &["caption", "dim-label"]));
            }
            tile.update_property(&[gtk::accessible::Property::Label(&format!("{} {} {}", r.label, r.value, r.unit))]);
            self.widget.append(&tile);
        }
        *self.shown.borrow_mut() = Some(readouts.to_vec());
    }
}

/// The columns, the rows and the figures' size, in pixels, for `n` readouts in `w` x `h` with `gap`
/// between them: the number of columns that makes the figures largest.
pub fn fit(n: usize, w: f64, h: f64, gap: f64) -> (usize, usize, f64) {
    let mut best = (1, n, 0.0);
    for c in 1..=n {
        let r = n.div_ceil(c);
        let tw = (w - gap * (c - 1) as f64) / c as f64;
        let th = (h - gap * (r - 1) as f64) / r as f64;
        // The figure, its label and its note in the height (GTK's lines are taller than the font); four or
        // five characters in the width.
        let f = (th * 0.38).min(tw * 0.27);
        if f > best.2 {
            best = (c, r, f);
        }
    }
    best
}

/// One readout of the under-way column, its labels sized by `InstrumentColumn::layout`.
struct Tile {
    widget: gtk::Box,
    label: gtk::Label,
    value: gtk::Label,
    unit: gtk::Label,
    /// One line, never wrapped: a wrapping line in a narrow tile asks for the height of many.
    note: super::chips::ChipLine,
}

/// Under way, beside the chart where the sidebar was: the navigating card, the instruments filling the
/// height (the fewer, the bigger: `fit`), and the Sailing / Motoring switch.
pub struct InstrumentColumn {
    pub widget: gtk::Box,
    pub nav: Rc<NavigatingCard>,
    rows: gtk::Box,
    tiles: RefCell<Vec<Tile>>,
    shown: RefCell<Vec<Readout>>,
    size: Cell<(i32, i32)>,
    laid: Cell<(i32, i32, usize)>,
    sail: gtk::Box,
    sail_shown: RefCell<Option<Switch>>,
    app: Weak<AppState>,
}

const GAP: i32 = 8;

impl InstrumentColumn {
    pub fn new(app: &Rc<AppState>) -> Rc<InstrumentColumn> {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, GAP);
        widget.add_css_class("underway-column");
        widget.set_width_request((tokens::SIDEBAR_WIDTH * 0.9) as i32);
        widget.set_visible(false);
        let nav = NavigatingCard::new(app);
        nav.widget.set_halign(gtk::Align::Fill);
        widget.append(&nav.widget);
        // The space the readouts have, measured by an empty area under them: the rows are an overlay and
        // never ask for more.
        let area = gtk::DrawingArea::builder().hexpand(true).vexpand(true).build();
        let rows = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(GAP).homogeneous(true).build();
        let over = gtk::Overlay::new();
        over.set_child(Some(&area));
        over.add_overlay(&rows);
        over.set_measure_overlay(&rows, false);
        over.set_overflow(gtk::Overflow::Hidden);
        widget.append(&over);
        let sail = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.append(&sail);
        let col = Rc::new(InstrumentColumn {
            widget,
            nav,
            rows,
            tiles: RefCell::new(Vec::new()),
            shown: RefCell::new(Vec::new()),
            size: Cell::new((0, 0)),
            laid: Cell::new((0, 0, 0)),
            sail,
            sail_shown: RefCell::new(None),
            app: Rc::downgrade(app),
        });
        let c = Rc::downgrade(&col);
        area.connect_resize(move |_, w, h| {
            if let Some(c) = c.upgrade() {
                c.size.set((w, h));
                c.layout();
            }
        });
        col
    }

    pub fn update(&self, on: bool, card: Option<&NavCard>, readouts: &[Readout], sail: Option<&Switch>) {
        self.widget.set_visible(on);
        if !on {
            return;
        }
        self.nav.update(card);
        if *self.sail_shown.borrow() != sail.cloned() {
            clear(&self.sail);
            if let Some(s) = sail {
                self.sail.append(&sail_tile(&self.app, s));
            }
            *self.sail_shown.borrow_mut() = sail.cloned();
        }
        if *self.shown.borrow() == readouts {
            return;
        }
        let same_tiles = self.shown.borrow().iter().map(|r| &r.id).eq(readouts.iter().map(|r| &r.id));
        if !same_tiles {
            *self.tiles.borrow_mut() = readouts.iter().map(|_| Self::tile()).collect();
            self.laid.set((0, 0, 0));
        }
        for (t, r) in self.tiles.borrow().iter().zip(readouts) {
            t.label.set_text(&r.label.to_uppercase());
            t.value.set_text(&r.value);
            t.unit.set_text(&r.unit);
            t.unit.set_visible(!r.unit.is_empty());
            t.note.set(&r.note);
            t.note.widget.set_visible(!r.note.is_empty());
            t.widget.update_property(&[gtk::accessible::Property::Label(&format!("{} {} {}", r.label, r.value, r.unit))]);
        }
        *self.shown.borrow_mut() = readouts.to_vec();
        self.layout();
    }

    fn tile() -> Tile {
        let widget = card(gtk::Orientation::Vertical, 0);
        widget.add_css_class("instrument-tile");
        widget.set_hexpand(true);
        let lab = |classes: &[&str]| gtk::Label::builder().xalign(0.0).css_classes(classes.to_vec()).ellipsize(gtk::pango::EllipsizeMode::End).build();
        let label = lab(&["instrument-label", "dim-label"]);
        let value = lab(&["instrument-figure"]);
        let unit = lab(&["readout-unit"]);
        unit.set_valign(gtk::Align::Baseline);
        let line = gtk::Box::builder().spacing(4).vexpand(true).valign(gtk::Align::Center).build();
        value.set_valign(gtk::Align::Baseline);
        line.append(&value);
        line.append(&unit);
        let note = super::chips::ChipLine::single(&["dim-label"], 14);
        widget.append(&label);
        widget.append(&line);
        widget.append(&note.widget);
        Tile { widget, label, value, unit, note }
    }

    /// Puts the tiles in rows and sizes their text for the space there is.
    fn layout(&self) {
        let (w, h) = self.size.get();
        let tiles = self.tiles.borrow();
        let n = tiles.len();
        if w <= 0 || h <= 0 || n == 0 {
            return;
        }
        let shown = self.shown.borrow();
        let (cols, _, figure) = fit(n, w as f64, h as f64, GAP as f64);
        let figure = figure.min(110.0);
        if self.laid.get() != (w, h, n) {
            self.laid.set((w, h, n));
            clear(&self.rows);
            for chunk in tiles.chunks(cols) {
                let row = gtk::Box::builder().spacing(GAP).homogeneous(true).build();
                for t in chunk {
                    if let Some(p) = t.widget.parent() {
                        p.downcast::<gtk::Box>().expect("a row").remove(&t.widget);
                    }
                    row.append(&t.widget);
                }
                self.rows.append(&row);
            }
        }
        let size = |label: &gtk::Label, px: f64| {
            let attrs = gtk::pango::AttrList::new();
            attrs.insert(gtk::pango::AttrSize::new_size_absolute((px * gtk::pango::SCALE as f64) as i32));
            label.set_attributes(Some(&attrs));
        };
        let small = (figure * 0.24).max(11.0);
        for (t, r) in tiles.iter().zip(shown.iter()) {
            // A word ("Standby") is wider than a figure: it is set smaller to fit.
            let chars = r.value.chars().count().max(4) as f64;
            size(&t.label, small);
            size(&t.value, figure * (5.0 / chars).min(1.0));
            size(&t.unit, (figure * 0.3).max(11.0));
            let pad = (figure * 0.2).max(8.0) as i32;
            t.widget.set_margin_top(0);
            t.widget.set_spacing(0);
            t.label.set_margin_top(pad / 2);
            t.label.set_margin_start(pad);
            t.value.set_margin_start(pad);
            t.note.widget.set_margin_start(pad);
            t.note.widget.set_margin_end(pad);
            t.note.widget.set_margin_bottom(pad / 2);
        }
    }
}

/// The Sailing / Motoring switch: what the performance log learns from.
fn sail_tile(app: &Weak<AppState>, s: &Switch) -> gtk::Box {
    let tile = card(gtk::Orientation::Horizontal, 8);
    tile.add_css_class("instrument-tile");
    let toggles = adw::ToggleGroup::builder().homogeneous(true).hexpand(true).build();
    toggles.add(adw::Toggle::builder().name("sailing").label(gettext("Sailing")).build());
    toggles.add(adw::Toggle::builder().name("motoring").label(gettext("Motoring")).build());
    if s.mode.is_empty() {
        toggles.set_active(gtk::INVALID_LIST_POSITION);
    } else {
        toggles.set_active_name(Some(&s.mode));
    }
    toggles.set_sensitive(!s.by_engine);
    if let Some(n) = s.note.as_ref().or(s.hint.as_ref()) {
        toggles.set_tooltip_text(Some(n));
    }
    let app = app.clone();
    toggles.connect_active_name_notify(move |t| {
        let (Some(app), Some(name)) = (app.upgrade(), t.active_name()) else { return };
        isomar_app::api::call(&app, "perf_set_mode", &serde_json::json!({ "mode": name.as_str() }));
    });
    tile.append(&toggles);
    tile
}

// The timeline and the router card.

/// One time control for the forecast and for the router's clock, docked over the bottom of the
/// chart while a forecast is loaded or a route preview is open.
pub struct TimelineBar {
    pub widget: gtk::Box,
    play: gtk::Button,
    scale: gtk::Scale,
    time: gtk::Label,
    age: gtk::Label,
    speed: gtk::MenuButton,
    syncing: Cell<bool>,
    shown: RefCell<Option<Timeline>>,
}

impl TimelineBar {
    pub fn new(app: &Rc<AppState>) -> Rc<TimelineBar> {
        let widget = card(gtk::Orientation::Vertical, 2);
        widget.add_css_class("timeline");
        widget.set_valign(gtk::Align::End);
        widget.set_hexpand(true);
        widget.set_visible(false);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let play = gtk::Button::builder().icon_name("media-playback-start-symbolic").tooltip_text(gettext("Play")).css_classes(["flat"]).valign(gtk::Align::Center).build();
        let back = gtk::Button::builder().icon_name("media-seek-backward-symbolic").tooltip_text(gettext("Back one hour")).css_classes(["flat"]).valign(gtk::Align::Center).build();
        let fwd = gtk::Button::builder().icon_name("media-seek-forward-symbolic").tooltip_text(gettext("Forward one hour")).css_classes(["flat"]).valign(gtk::Align::Center).build();
        let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 3600.0);
        scale.set_draw_value(false);
        scale.set_hexpand(true);
        scale.set_width_request(160);
        scale.update_property(&[gtk::accessible::Property::Label(&gettext("Forecast time"))]);
        let time = gtk::Label::builder().css_classes(["numeric"]).width_chars(16).xalign(1.0).build();
        let speed = gtk::MenuButton::builder().label("1×").tooltip_text(gettext("Playback speed")).css_classes(["flat"]).valign(gtk::Align::Center).build();
        let menu = gtk::gio::Menu::new();
        for x in ["0.5", "1", "2", "4"] {
            let item = gtk::gio::MenuItem::new(Some(&format!("{x}×")), None);
            item.set_action_and_target_value(Some("win.timeline-speed"), Some(&x.to_variant()));
            menu.append_item(&item);
        }
        speed.set_menu_model(Some(&menu));
        row.append(&play);
        row.append(&back);
        row.append(&scale);
        row.append(&fwd);
        row.append(&time);
        row.append(&speed);
        widget.append(&row);
        let age = gtk::Label::builder().xalign(0.0).css_classes(["caption", "dim-label"]).margin_start(6).build();
        widget.append(&age);
        let bar = Rc::new(TimelineBar { widget, play, scale, time, age, speed, syncing: Cell::new(false), shown: RefCell::new(None) });
        {
            let a = Rc::downgrade(app);
            let b = Rc::downgrade(&bar);
            bar.play.connect_clicked(move |_| {
                if let (Some(a), Some(b)) = (a.upgrade(), b.upgrade()) {
                    let playing = b.shown.borrow().as_ref().is_some_and(|t| t.playing);
                    isomar_app::vm::timeline::set_playing(&a, !playing);
                    b.refresh(&a);
                }
            });
        }
        for (btn, steps) in [(&back, -1), (&fwd, 1)] {
            let a = Rc::downgrade(app);
            let b = Rc::downgrade(&bar);
            btn.connect_clicked(move |_| {
                if let (Some(a), Some(b)) = (a.upgrade(), b.upgrade()) {
                    isomar_app::vm::timeline::step(&a, steps);
                    b.refresh(&a);
                }
            });
        }
        {
            let a = Rc::downgrade(app);
            let b = Rc::downgrade(&bar);
            bar.scale.connect_value_changed(move |s| {
                if let (Some(a), Some(b)) = (a.upgrade(), b.upgrade())
                    && !b.syncing.get()
                {
                    isomar_app::vm::timeline::set_time(&a, s.value() as i64);
                    b.refresh(&a);
                }
            });
        }
        bar
    }

    pub fn refresh(&self, app: &AppState) {
        self.update(isomar_app::vm::timeline::timeline(app).as_ref());
    }

    pub fn update(&self, t: Option<&Timeline>) {
        if self.shown.borrow().as_ref() == t {
            return;
        }
        *self.shown.borrow_mut() = t.cloned();
        self.widget.set_visible(t.is_some());
        let Some(t) = t else { return };
        self.syncing.set(true);
        self.scale.set_range(t.start as f64, t.end.max(t.start + 1) as f64);
        self.scale.set_increments(t.step as f64, t.step as f64 * 6.0);
        self.scale.set_value(t.now as f64);
        self.syncing.set(false);
        self.time.set_text(&t.label);
        self.play.set_icon_name(if t.playing { "media-playback-pause-symbolic" } else { "media-playback-start-symbolic" });
        self.play.set_tooltip_text(Some(&if t.playing { gettext("Pause") } else { gettext("Play") }));
        self.speed.set_label(&format!("{}×", t.speed));
        self.age.set_visible(t.age.is_some());
        self.age.set_text(t.age.as_deref().unwrap_or(""));
        if t.stale {
            self.age.add_css_class("stale");
        } else {
            self.age.remove_css_class("stale");
        }
    }
}

/// A short line in an alternative's colour and dash pattern, so the list matches the chart.
fn dash_swatch(colour: &str, dash: &[f64]) -> gtk::DrawingArea {
    let token = Token::ALL.iter().copied().find(|t| t.name() == colour).unwrap_or(Token::Route);
    let dash = dash.to_vec();
    let area = gtk::DrawingArea::builder().content_width(30).content_height(10).valign(gtk::Align::Center).build();
    area.set_draw_func(move |_, cr, w, h| {
        let (r, g, b, a) = tokens::color(token, super::theme::token_theme());
        cr.set_source_rgba(r, g, b, a);
        cr.set_line_width(3.0);
        cr.set_line_cap(gtk::cairo::LineCap::Round);
        cr.set_dash(&dash, 0.0);
        cr.move_to(2.0, h as f64 / 2.0);
        cr.line_to(w as f64 - 2.0, h as f64 / 2.0);
        let _ = cr.stroke();
    });
    area
}

/// The card of a weather-route preview: what the search found, the alternatives, Use and Discard.
pub struct RouterCard {
    pub widget: gtk::Box,
    app: Weak<AppState>,
    shown: RefCell<Option<Card>>,
}

impl RouterCard {
    pub fn new(app: &Rc<AppState>) -> Rc<RouterCard> {
        let widget = card(gtk::Orientation::Vertical, 8);
        widget.add_css_class("router-card");
        widget.set_halign(gtk::Align::Start);
        widget.set_width_request(340);
        widget.set_visible(false);
        Rc::new(RouterCard { widget, app: Rc::downgrade(app), shown: RefCell::new(None) })
    }

    pub fn update(&self, c: Option<&Card>) {
        if self.shown.borrow().as_ref() == c {
            return;
        }
        *self.shown.borrow_mut() = c.cloned();
        clear(&self.widget);
        self.widget.set_visible(c.is_some());
        let (Some(c), Some(app)) = (c, self.app.upgrade()) else { return };
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        if c.failure {
            let w = gtk::Image::from_icon_name("dialog-warning-symbolic");
            w.add_css_class("warning");
            head.append(&w);
        }
        let title = label(&c.title, &["heading"]);
        title.set_wrap(true);
        head.append(&title);
        self.widget.append(&head);
        // Several models have their own rows and verdict: the instructions would make the card too tall.
        if c.models.is_empty() {
            let note = label(&c.note, &["caption", "dim-label"]);
            note.set_wrap(true);
            note.set_max_width_chars(44);
            self.widget.append(&note);
        }
        // Routed on several forecasts: a row per model (click one to show its search and wind), and what
        // they say together.
        if !c.models.is_empty() {
            let list = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::Single).build();
            for m in &c.models {
                let title = if m.robust { gettext_f("{model} · holds up best", &[("model", &m.label)]) } else { m.label.clone() };
                let row = super::chips::row_with(Some(dash_swatch(&m.colour, &[]).upcast_ref()), &title, &m.summary);
                row.set_activatable(true);
                list.append(&row);
                if m.focused {
                    list.select_row(Some(&row));
                }
            }
            let app2 = Rc::downgrade(&app);
            list.connect_row_activated(move |_, row| {
                if let Some(a) = app2.upgrade() {
                    isomar_app::vm::router::focus_model(&a, row.index() as usize);
                }
            });
            self.widget.append(&list);
            let l = label(&c.verdict.join(" "), &["caption"]);
            l.set_wrap(true);
            l.set_max_width_chars(48);
            self.widget.append(&l);
            let cross = gtk::Button::builder().label(gettext("Cross-check the routes…")).tooltip_text(gettext("Each model's route sailed through every model's forecast")).css_classes(["flat"]).halign(gtk::Align::Start).build();
            let a = Rc::downgrade(&app);
            cross.connect_clicked(move |_| {
                if let Some(a) = a.upgrade()
                    && let Some((heads, sections)) = isomar_app::vm::router::cross_check(&a)
                {
                    let heads: Vec<String> = heads.iter().map(|h| format!("<b>{}</b>", gtk::glib::markup_escape_text(h))).collect();
                    super::routing_stats::show_sections(&gettext("Cross-check"), &heads, &sections);
                }
            });
            self.widget.append(&cross);
        }
        // With several models the shown model's best way is its row above; the others picked on its front
        // are listed.
        if !c.alternatives.is_empty() && (c.models.is_empty() || c.alternatives.len() > 1) {
            let list = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::Single).build();
            for a in &c.alternatives {
                let row = super::chips::row_with(Some(dash_swatch(&a.colour, &a.dash).upcast_ref()), &a.title, &a.summary);
                row.set_activatable(true);
                let remove = gtk::Button::builder().icon_name("user-trash-symbolic").tooltip_text(gettext("Remove this alternative")).css_classes(["flat"]).valign(gtk::Align::Center).build();
                let (app2, i) = (Rc::downgrade(&app), a.index);
                remove.connect_clicked(move |_| {
                    if let Some(a) = app2.upgrade() {
                        isomar_app::router::remove_choice(&a, i);
                    }
                });
                if !c.replay {
                    row.add_suffix(&remove);
                }
                list.append(&row);
                if a.selected {
                    list.select_row(Some(&row));
                }
            }
            let app2 = Rc::downgrade(&app);
            list.connect_row_activated(move |_, row| {
                if let Some(a) = app2.upgrade() {
                    isomar_app::vm::router::select(&a, row.index() as usize);
                }
            });
            self.widget.append(&list);
        }
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let stats = gtk::Button::with_label(&gettext("Statistics"));
        stats.set_sensitive(!c.alternatives.is_empty());
        let discard = gtk::Button::with_label(&if c.replay { gettext("Close") } else { gettext("Discard") });
        discard.set_hexpand(true);
        discard.set_halign(gtk::Align::End);
        let use_btn = gtk::Button::builder().label(gettext("Use this route")).css_classes(["suggested-action"]).sensitive(!c.alternatives.is_empty()).build();
        let selected = c.alternatives.iter().find(|a| a.selected).or(c.alternatives.first()).map(|a| a.index);
        let a = Rc::downgrade(&app);
        let heads: Vec<String> = c.alternatives.iter().map(|x| super::routing_stats::head(&x.colour, x.index + 1)).collect();
        stats.connect_clicked(move |_| {
            if let Some(a) = a.upgrade()
                && let Some((_, sections)) = isomar_app::vm::router::stats(&a)
            {
                super::routing_stats::show_sections(&gettext("Route statistics"), &heads, &sections);
            }
        });
        let a = Rc::downgrade(&app);
        discard.connect_clicked(move |_| {
            if let Some(a) = a.upgrade() {
                isomar_app::vm::router::discard(&a);
            }
        });
        let a = Rc::downgrade(&app);
        use_btn.connect_clicked(move |_| {
            if let (Some(a), Some(i)) = (a.upgrade(), selected) {
                isomar_app::router::use_choice(&a, i);
            }
        });
        buttons.append(&stats);
        buttons.append(&discard);
        if !c.replay {
            buttons.append(&use_btn);
        }
        self.widget.append(&buttons);
    }
}

/// What the forecast says under the pointer, top right of the chart.
pub struct WeatherReadout {
    pub widget: gtk::Box,
    lines: Vec<super::chips::ChipLine>,
    time: gtk::Label,
}

impl WeatherReadout {
    pub fn new() -> WeatherReadout {
        let widget = card(gtk::Orientation::Vertical, 2);
        widget.set_halign(gtk::Align::End);
        widget.set_valign(gtk::Align::Start);
        widget.set_margin_top(16);
        widget.set_margin_end(16);
        widget.set_can_target(false);
        widget.set_visible(false);
        // One line a value: wind, gusts, pressure, waves, air temperature, current.
        let lines: Vec<_> = (0..6).map(|_| super::chips::ChipLine::single(&["numeric"], 14)).collect();
        for l in &lines {
            widget.append(&l.widget);
        }
        let time = label("", &["caption", "dim-label"]);
        widget.append(&time);
        WeatherReadout { widget, lines, time }
    }

    pub fn update(&self, app: &AppState) {
        let w = app.weather.borrow();
        let chips = app.pointer.get().filter(|_| w.has_any()).map(|p| w.readout_at(p.lat, p.lon)).unwrap_or_default();
        self.widget.set_visible(!chips.is_empty());
        if !chips.is_empty() {
            for (i, l) in self.lines.iter().enumerate() {
                l.set(chips.get(i).map(std::slice::from_ref).unwrap_or_default());
                l.widget.set_visible(i < chips.len());
            }
            self.time.set_text(&w.time.with_timezone(&chrono::Local).format("%a %d %b %H:%M").to_string());
        }
    }
}

// The footer.

/// A scale bar: a bracket `px` long and its label.
fn scale_line() -> (gtk::Box, gtk::DrawingArea, gtk::Label) {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let area = gtk::DrawingArea::builder().content_height(6).valign(gtk::Align::Center).build();
    area.set_draw_func(|_, cr, w, h| {
        let (r, g, b, _) = tokens::color(Token::OwnShip, super::theme::token_theme());
        cr.set_source_rgb(r, g, b);
        cr.set_line_width(1.5);
        let (w, h) = (w as f64 - 1.0, h as f64 - 1.0);
        cr.move_to(0.75, 0.0);
        cr.line_to(0.75, h);
        cr.line_to(w, h);
        cr.line_to(w, 0.0);
        let _ = cr.stroke();
    });
    let l = gtk::Label::builder().css_classes(["caption"]).build();
    b.append(&area);
    b.append(&l);
    (b, area, l)
}

/// The strip under the chart: scale bars, credits (they give way first), the Planning only pill and
/// the position under the pointer (click to copy).
pub struct FooterStrip {
    pub widget: gtk::Box,
    nm: (gtk::DrawingArea, gtk::Label),
    metric: (gtk::DrawingArea, gtk::Label),
    credits: gtk::Label,
    pill: gtk::MenuButton,
    pill_text: gtk::Label,
    position: gtk::Button,
    position_chips: super::chips::ChipLine,
    shown: RefCell<Option<Footer>>,
    metric_box: gtk::Widget,
}

impl FooterStrip {
    pub fn new(app: &Rc<AppState>) -> Rc<FooterStrip> {
        let widget = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        widget.add_css_class("isomar-footer");
        let scales = gtk::Box::new(gtk::Orientation::Vertical, 0);
        scales.set_valign(gtk::Align::Center);
        let (nb, na, nl) = scale_line();
        let (mb, ma, ml) = scale_line();
        scales.append(&nb);
        scales.append(&mb);
        widget.append(&scales);
        let credits = gtk::Label::builder().xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).width_chars(4).css_classes(["caption", "dim-label"]).build();
        widget.append(&credits);

        // The pill explains itself and leads to the charts for the area.
        let pop = gtk::Popover::new();
        let pb = gtk::Box::new(gtk::Orientation::Vertical, 8);
        pb.set_margin_top(8);
        pb.set_margin_bottom(8);
        pb.set_margin_start(8);
        pb.set_margin_end(8);
        pb.append(&label(&gettext("Planning only"), &["heading"]));
        let pill_text = label("", &[]);
        pill_text.set_wrap(true);
        pill_text.set_max_width_chars(36);
        pb.append(&pill_text);
        let charts = gtk::Button::builder().label(gettext("Charts for this area…")).action_name("win.page").action_target(&"charts".to_variant()).css_classes(["suggested-action"]).build();
        {
            let pop = pop.clone();
            charts.connect_clicked(move |_| pop.popdown());
        }
        pb.append(&charts);
        pop.set_child(Some(&pb));
        let pill_content = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        pill_content.append(&gtk::Image::from_icon_name("dialog-warning-symbolic"));
        pill_content.append(&gtk::Label::new(Some(&gettext("Planning only"))));
        let pill = gtk::MenuButton::builder().child(&pill_content).popover(&pop).css_classes(["flat", "planning-pill"]).valign(gtk::Align::Center).build();
        pill.set_tooltip_text(Some(&gettext("No official chart covers this area. Not for navigation.")));
        widget.append(&pill);

        let position_chips = super::chips::ChipLine::single(&["numeric"], 14);
        let position = gtk::Button::builder().child(&position_chips.widget).css_classes(["flat"]).tooltip_text(gettext("Click to copy the position")).build();
        {
            let a = Rc::downgrade(app);
            position.connect_clicked(move |_| {
                let Some(a) = a.upgrade() else { return };
                if let Some(p) = a.pointer.get() {
                    if let Some(d) = gdk::Display::default() {
                        d.clipboard().set_text(&isomar_app::text::position_text(&p));
                    }
                    a.toast(&gettext("Position copied"));
                }
            });
        }
        widget.append(&position);
        let f = Rc::new(FooterStrip { widget, nm: (na, nl), metric: (ma, ml), credits, pill, pill_text, position, position_chips, shown: RefCell::new(None), metric_box: mb.upcast() });
        {
            // The pointer moves often: only the position label follows it, at once.
            let fw = Rc::downgrade(&f);
            let a = Rc::downgrade(app);
            app.canvas.connect_pointer_moved(move |_| {
                if let (Some(f), Some(a)) = (fw.upgrade(), a.upgrade()) {
                    f.update(&isomar_app::vm::footer::footer(&a));
                }
            });
            let fw = Rc::downgrade(&f);
            let a = Rc::downgrade(app);
            app.canvas.connect_view_changed(move |_| {
                if let (Some(f), Some(a)) = (fw.upgrade(), a.upgrade()) {
                    f.update(&isomar_app::vm::footer::footer(&a));
                }
            });
        }
        f
    }

    /// At phone width the footer keeps one scale bar, short credits and the pill.
    pub fn set_compact(&self, on: bool) {
        self.metric_box.set_visible(!on);
        self.position.set_visible(!on);
    }

    pub fn update(&self, f: &Footer) {
        if self.shown.borrow().as_ref() == Some(f) {
            return;
        }
        for ((area, l), bar) in [(&self.nm, &f.scale), (&self.metric, &f.metric)] {
            area.set_content_width(bar.px.round().max(2.0) as i32);
            area.queue_draw();
            l.set_text(&bar.label);
        }
        self.credits.set_text(&match f.credits.as_slice() {
            [] => String::new(),
            [one] => one.clone(),
            [first, rest @ ..] => gettext_f("{first} and {n} more", &[("first", first), ("n", &rest.len().to_string())]),
        });
        self.credits.set_tooltip_text(Some(&f.credits.join("\n")));
        self.pill.set_visible(f.planning_only);
        self.pill_text.set_text(&f.planning_text);
        self.position_chips.set(f.pointer.as_ref().map_or(&f.view, |p| &p.chips));
        *self.shown.borrow_mut() = Some(f.clone());
    }
}
