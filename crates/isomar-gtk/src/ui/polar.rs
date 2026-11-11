//! The boat polar editor: a polar diagram of boat speed against true wind
//! angle for one wind speed at a time, whose points can be dragged. Each
//! point sits on a fixed angle of the table, so dragging changes its speed.
//! The polar is one of the library (`isomar_app::polars`): a built-in one, averaged from published
//! polars, or one of the user's. Saved polars are kept in the data folder; the one in use is what the
//! weather router sails with and the only one the performance log learns into.

use super::app::AppState;
use super::show_error;
use crate::core::routing::polar::{EDITOR_TWS, Polar};
use crate::i18n::{gettext, gettext_f};
use adw::prelude::*;
use isomar_app::polars::{PolarList, PolarRow};
use isomar_app::vm::performance::Observed;
use std::cell::RefCell;
use std::f64::consts::PI;
use std::rc::Rc;

/// Pixels around the diagram kept for the angle labels.
const MARGIN: f64 = 34.0;
/// How close, in pixels, the pointer must be to grab a point.
const GRAB: f64 = 14.0;
/// Speeds are kept to a tenth of a knot.
const STEP: f64 = 0.1;

/// Where the diagram sits in the drawing area.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Layout {
    cx: f64,
    cy: f64,
    /// Pixels from the centre to the outer ring.
    r: f64,
}

impl Layout {
    fn new(w: f64, h: f64) -> Layout {
        Layout { cx: w / 2.0, cy: h / 2.0, r: (w.min(h) / 2.0 - MARGIN).max(20.0) }
    }

    /// The point for a speed at a wind angle. `side` is 1 for starboard
    /// (right) and -1 for port; the wind comes from the top.
    fn point(&self, scale: f64, twa: f64, kn: f64, side: f64) -> (f64, f64) {
        let rr = kn / scale * self.r;
        let a = twa.to_radians();
        (self.cx + side * rr * a.sin(), self.cy - rr * a.cos())
    }

    /// The speed along the ray of `twa` that is closest to a pixel, clamped
    /// to the diagram and rounded to a tenth.
    fn speed_at(&self, scale: f64, twa: f64, side: f64, x: f64, y: f64) -> f64 {
        let a = twa.to_radians();
        let along = (x - self.cx) * side * a.sin() + (self.cy - y) * a.cos();
        let kn = along / self.r * scale;
        ((kn / STEP).round() * STEP).clamp(0.0, scale)
    }
}

/// Outer ring and spacing of the rings for a polar, with room above its
/// fastest point.
fn scale_for(p: &Polar) -> (f64, f64) {
    let max = p.speeds.iter().flatten().fold(0.0f64, |m, v| m.max(*v));
    let step = if max < 11.0 { 2.0 } else { 5.0 };
    ((((max + 0.5) / step).ceil() * step).max(3.0 * step), step)
}

/// The nearest point of the column within reach of a pixel: its row and
/// the side it was grabbed on.
fn hit_test(l: &Layout, scale: f64, p: &Polar, col: usize, x: f64, y: f64) -> Option<(usize, f64)> {
    let mut best: Option<(usize, f64, f64)> = None;
    for (row, a) in p.twa.iter().enumerate() {
        for side in [1.0, -1.0] {
            let (px, py) = l.point(scale, *a, p.speeds[row][col], side);
            let d = (px - x).hypot(py - y);
            if d <= GRAB && best.is_none_or(|b| d < b.2) {
                best = Some((row, side, d));
            }
        }
    }
    best.map(|(row, side, _)| (row, side))
}

struct Drag {
    row: usize,
    side: f64,
    /// The scale when the drag began, so the diagram does not rescale under
    /// the pointer.
    scale: f64,
}

struct State {
    polar: Polar,
    col: usize,
    hover: Option<usize>,
    drag: Option<Drag>,
    /// What the performance log learnt, drawn under the polar (only for the polar in use: it learns alone).
    observed: Option<Observed>,
    /// The polar shown, of the library.
    id: String,
    list: PolarList,
    /// Speeds changed since it was shown or saved.
    dirty: bool,
}

/// The confidence from which a learnt speed is offered.
const LEARNT_CONFIDENCE: f64 = isomar_app::vm::performance::APPLY_CONFIDENCE;

/// The learnt speeds `p` would take: (row, col, speed) where the log is confident and differs.
fn learnt_changes(p: &Polar, o: &Observed) -> Vec<(usize, usize, f64)> {
    let mut out = Vec::new();
    for (row, a) in p.twa.iter().enumerate() {
        let Some(lr) = o.twa.iter().position(|x| (x - a).abs() < 1e-6) else { continue };
        for (col, w) in p.tws.iter().enumerate() {
            let Some(lc) = o.tws.iter().position(|x| (x - w).abs() < 1e-6) else { continue };
            let v = o.speeds[lr][lc];
            if o.confidence[lr][lc] >= LEARNT_CONFIDENCE && (p.speeds[row][col] - v).abs() >= 0.05 {
                out.push((row, col, v));
            }
        }
    }
    out
}

/// What the picker's list says under a polar's name.
fn picker_note(r: &PolarRow) -> Option<String> {
    match (r.active, r.entry.modified) {
        (true, true) => Some(gettext("In use, changed from the published speeds")),
        (true, false) => Some(gettext("In use")),
        (false, true) => Some(gettext("Changed from the published speeds")),
        (false, false) => None,
    }
}

/// The picker's list: each polar's name, and under it whether it is in use or changed.
fn picker_list_factory(state: &Rc<RefCell<State>>) -> gtk::SignalListItemFactory {
    let f = gtk::SignalListItemFactory::new();
    f.connect_setup(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else { return };
        let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
        b.append(&gtk::Label::builder().xalign(0.0).build());
        b.append(&gtk::Label::builder().xalign(0.0).css_classes(["caption", "dim-label"]).build());
        item.set_child(Some(&b));
    });
    let state = Rc::downgrade(state);
    f.connect_bind(move |_, item| {
        let (Some(item), Some(state)) = (item.downcast_ref::<gtk::ListItem>(), state.upgrade()) else { return };
        let Some(b) = item.child().and_downcast::<gtk::Box>() else { return };
        let (Some(name), Some(note)) = (b.first_child().and_downcast::<gtk::Label>(), b.last_child().and_downcast::<gtk::Label>()) else { return };
        let s = state.borrow();
        let Some(r) = s.list.polars.get(item.position() as usize) else { return };
        name.set_label(&r.entry.name);
        let n = picker_note(r);
        note.set_visible(n.is_some());
        note.set_label(&n.unwrap_or_default());
    });
    f
}

fn selected_row(s: &State) -> Option<&PolarRow> {
    s.list.polars.iter().find(|r| r.entry.id == s.id)
}

/// The editor shows polar `id`: its speeds, and what the log learnt when it is the active one.
fn show_polar(app: &AppState, s: &mut State, id: &str) {
    s.id = id.to_string();
    s.polar = isomar_app::polars::library().load(id).unwrap_or_else(Polar::generic_cruiser).resampled(&EDITOR_TWS);
    s.observed = if s.list.active == id { isomar_app::vm::performance::observed(app) } else { None };
    s.dirty = false;
    s.hover = None;
}

pub fn open(app: &Rc<AppState>) {
    // What the performance log learnt, once its analysis is done (started now if the log is newer).
    if isomar_app::flows::perf::stale(app) {
        isomar_app::flows::perf::analyse_now(app);
    }
    let list = isomar_app::polars::list(app);
    let mut first = State { polar: Polar::generic_cruiser(), col: 1, hover: None, drag: None, observed: None, id: String::new(), list, dirty: false };
    let active = first.list.active.clone();
    show_polar(app, &mut first, &active);
    let state = Rc::new(RefCell::new(first));

    let win = super::float::window(&gettext("Boat polar"), 780, 780);
    let header = adw::HeaderBar::builder().show_start_title_buttons(false).show_end_title_buttons(false).build();
    let cancel = gtk::Button::with_label(&gettext("Cancel"));
    let save = gtk::Button::with_label(&gettext("Save"));
    save.add_css_class("suggested-action");
    let menu = gtk::gio::Menu::new();
    let more = gtk::MenuButton::builder().icon_name("view-more-symbolic").tooltip_text(gettext("More")).menu_model(&menu).build();
    header.pack_start(&cancel);
    header.pack_end(&save);
    header.pack_end(&more);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.set_margin_top(12);
    body.set_margin_bottom(18);
    body.set_margin_start(18);
    body.set_margin_end(18);

    // Which polar: the built-in ones and the user's, the one in use marked.
    let names = gtk::StringList::new(&[]);
    let picker = adw::ComboRow::builder().title(gettext("Polar")).model(&names).build();
    let use_it = gtk::Button::builder().label(gettext("Use this polar")).valign(gtk::Align::Center).css_classes(["suggested-action"]).build();
    picker.set_list_factory(Some(&picker_list_factory(&state)));
    picker.add_suffix(&use_it);
    let boxed = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
    boxed.append(&picker);
    body.append(&boxed);

    let hint = gtk::Label::builder()
        .label(gettext("Boat speed at each true wind angle, for one wind speed at a time. The wind comes from the top. Drag a point to change the speed at that angle."))
        .wrap(true)
        .xalign(0.0)
        .build();
    hint.add_css_class("dim-label");
    body.append(&hint);

    let toggles = adw::ToggleGroup::new();
    toggles.set_homogeneous(true);
    for (i, w) in EDITOR_TWS.iter().enumerate() {
        toggles.add(adw::Toggle::builder().name(i.to_string()).label(gettext_f("{n} kn", &[("n", &format!("{w:.0}"))])).build());
    }
    toggles.set_active(state.borrow().col as u32);
    toggles.set_halign(gtk::Align::Center);
    body.append(&toggles);

    let area = gtk::DrawingArea::builder().content_width(360).content_height(320).hexpand(true).vexpand(true).build();
    let readout = gtk::Label::builder().xalign(0.5).build();
    readout.add_css_class("heading");
    let vmg = gtk::Label::builder().xalign(0.5).wrap(true).justify(gtk::Justification::Center).build();
    vmg.add_css_class("dim-label");
    body.append(&area);
    body.append(&readout);
    body.append(&vmg);

    let refresh = {
        let (state, readout, vmg) = (state.clone(), readout.clone(), vmg.clone());
        Rc::new(move || {
            let s = state.borrow();
            let tws = s.polar.tws[s.col];
            let row = s.drag.as_ref().map(|d| d.row).or(s.hover);
            readout.set_text(&match row {
                Some(r) => gettext_f(
                    "{a}° from the wind: {v} kn in {w} kn of wind",
                    &[("a", &format!("{:.0}", s.polar.twa[r])), ("v", &format!("{:.1}", s.polar.speeds[r][s.col])), ("w", &format!("{tws:.0}"))],
                ),
                None => gettext_f("{w} kn of wind", &[("w", &format!("{tws:.0}"))]),
            });
            let (ua, uv) = s.polar.best_vmg(tws, true);
            let (da, dv) = s.polar.best_vmg(tws, false);
            vmg.set_text(&gettext_f(
                "Best upwind: {ua}°, {uv} kn made good. Best downwind: {da}°, {dv} kn made good.",
                &[("ua", &format!("{ua:.0}")), ("uv", &format!("{uv:.1}")), ("da", &format!("{da:.0}")), ("dv", &format!("{dv:.1}"))],
            ));
        })
    };
    refresh();

    {
        let state = state.clone();
        area.set_draw_func(move |a, cr, w, h| draw(a, cr, w as f64, h as f64, &state.borrow()));
    }
    {
        let (state, area, refresh) = (state.clone(), area.downgrade(), refresh.clone());
        toggles.connect_active_notify(move |t| {
            {
                let mut s = state.borrow_mut();
                s.col = (t.active() as usize).min(EDITOR_TWS.len() - 1);
                s.hover = None;
            }
            refresh();
            if let Some(a) = area.upgrade() {
                a.queue_draw();
            }
        });
    }

    let hover = gtk::EventControllerMotion::new();
    {
        let (state, area, refresh) = (state.clone(), area.downgrade(), refresh.clone());
        hover.connect_motion(move |_, x, y| {
            let Some(area) = area.upgrade() else { return };
            let mut s = state.borrow_mut();
            if s.drag.is_some() {
                return;
            }
            let l = Layout::new(area.width() as f64, area.height() as f64);
            let (scale, _) = scale_for(&s.polar);
            let hit = hit_test(&l, scale, &s.polar, s.col, x, y);
            let row = hit.map(|h| h.0);
            if row != s.hover {
                s.hover = row;
                area.set_cursor_from_name(Some(if row.is_some() { "grab" } else { "default" }));
                drop(s);
                refresh();
                area.queue_draw();
            }
        });
    }
    {
        let (state, area, refresh) = (state.clone(), area.downgrade(), refresh.clone());
        hover.connect_leave(move |_| {
            if state.borrow().drag.is_some() {
                return;
            }
            state.borrow_mut().hover = None;
            refresh();
            if let Some(a) = area.upgrade() {
                a.queue_draw();
            }
        });
    }
    area.add_controller(hover);

    let drag = gtk::GestureDrag::new();
    {
        let (state, area, refresh) = (state.clone(), area.downgrade(), refresh.clone());
        drag.connect_drag_begin(move |g, x, y| {
            let Some(area) = area.upgrade() else { return };
            let l = Layout::new(area.width() as f64, area.height() as f64);
            let grabbed = {
                let mut s = state.borrow_mut();
                let (scale, _) = scale_for(&s.polar);
                let hit = hit_test(&l, scale, &s.polar, s.col, x, y);
                if let Some((row, side)) = hit {
                    s.drag = Some(Drag { row, side, scale });
                    s.hover = Some(row);
                }
                hit.is_some()
            };
            if grabbed {
                area.set_cursor_from_name(Some("grabbing"));
            } else {
                // Denying ends the gesture at once, and drag-end borrows the
                // state, so it must not be held here.
                g.set_state(gtk::EventSequenceState::Denied);
            }
            refresh();
            area.queue_draw();
        });
    }
    {
        let (state, area, refresh) = (state.clone(), area.downgrade(), refresh.clone());
        drag.connect_drag_update(move |g, dx, dy| {
            let Some(area) = area.upgrade() else { return };
            let (sx, sy) = g.start_point().unwrap_or((0.0, 0.0));
            let mut s = state.borrow_mut();
            let Some((row, side, scale)) = s.drag.as_ref().map(|d| (d.row, d.side, d.scale)) else { return };
            let l = Layout::new(area.width() as f64, area.height() as f64);
            let kn = l.speed_at(scale, s.polar.twa[row], side, sx + dx, sy + dy);
            let col = s.col;
            s.polar.set_speed(row, col, kn);
            s.dirty = true;
            drop(s);
            refresh();
            area.queue_draw();
        });
    }
    {
        let (state, area, refresh) = (state.clone(), area.downgrade(), refresh.clone());
        drag.connect_drag_end(move |_, _, _| {
            state.borrow_mut().drag = None;
            refresh();
            if let Some(a) = area.upgrade() {
                a.set_cursor_from_name(Some("grab"));
                a.queue_draw();
            }
        });
    }
    area.add_controller(drag);

    // What the performance log learnt: a legend and a button, shown once there is something.
    let learnt = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let legend = gtk::Label::builder()
        .label(gettext("Dots: what the boat did, from the performance log. Dashed: the polar learnt from it."))
        .wrap(true)
        .xalign(0.5)
        .justify(gtk::Justification::Center)
        .css_classes(["caption", "dim-label"])
        .build();
    let use_learnt = gtk::Button::builder().halign(gtk::Align::Center).build();
    learnt.append(&legend);
    learnt.append(&use_learnt);
    body.append(&learnt);
    let sync_learnt = {
        let (state, learnt, use_learnt) = (state.clone(), learnt.downgrade(), use_learnt.downgrade());
        Rc::new(move || {
            let (Some(learnt), Some(b)) = (learnt.upgrade(), use_learnt.upgrade()) else { return };
            let s = state.borrow();
            learnt.set_visible(s.observed.is_some());
            let n = s.observed.as_ref().map_or(0, |o| learnt_changes(&s.polar, o).len());
            b.set_label(&gettext_f("Use the learnt speeds ({n})", &[("n", &n.to_string())]));
            b.set_visible(n > 0);
        })
    };
    sync_learnt();
    {
        let (state, area, refresh, sync) = (state.clone(), area.downgrade(), refresh.clone(), sync_learnt.clone());
        use_learnt.connect_clicked(move |_| {
            {
                let mut s = state.borrow_mut();
                let changes = s.observed.as_ref().map(|o| learnt_changes(&s.polar, o)).unwrap_or_default();
                for (row, col, v) in changes {
                    s.polar.set_speed(row, col, (v / STEP).round() * STEP);
                }
                s.dirty = true;
            }
            refresh();
            sync();
            if let Some(a) = area.upgrade() {
                a.queue_draw();
            }
        });
    }
    // The analysis may still be running: pick its result up when it lands.
    let watch_analysis = {
        let (app, state, area, sync) = (Rc::downgrade(app), Rc::downgrade(&state), area.downgrade(), sync_learnt.clone());
        Rc::new(move || {
            let (app, state, area, sync) = (app.clone(), state.clone(), area.clone(), sync.clone());
            gtk::glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
                let (Some(app), Some(state), Some(area)) = (app.upgrade(), state.upgrade(), area.upgrade()) else { return gtk::glib::ControlFlow::Break };
                if app.perf.borrow().analysing {
                    return gtk::glib::ControlFlow::Continue;
                }
                {
                    let mut s = state.borrow_mut();
                    if s.list.active == s.id {
                        s.observed = isomar_app::vm::performance::observed(&app);
                    }
                }
                sync();
                area.queue_draw();
                gtk::glib::ControlFlow::Break
            });
        })
    };
    if state.borrow().observed.is_none() && app.perf.borrow().analysing {
        watch_analysis();
    }

    // The picker and the menu follow the library; `syncing` keeps a rebuilt list from loading a polar.
    let syncing = Rc::new(std::cell::Cell::new(false));
    let actions = gtk::gio::SimpleActionGroup::new();
    let sync_picker = {
        let (state, picker, names, use_it, actions, syncing) = (state.clone(), picker.downgrade(), names.clone(), use_it.downgrade(), actions.clone(), syncing.clone());
        Rc::new(move || {
            let (Some(picker), Some(use_it)) = (picker.upgrade(), use_it.upgrade()) else { return };
            let s = state.borrow();
            syncing.set(true);
            let refs: Vec<&str> = s.list.polars.iter().map(|r| r.entry.name.as_str()).collect();
            names.splice(0, names.n_items(), &refs);
            if let Some(i) = s.list.polars.iter().position(|r| r.entry.id == s.id) {
                picker.set_selected(i as u32);
            }
            syncing.set(false);
            let row = selected_row(&s);
            let (active, builtin, modified, samples) = row.map_or((false, true, false, 0), |r| (r.active, r.entry.builtin, r.entry.modified, r.samples));
            use_it.set_visible(!active);
            let mut sub = if active { gettext("In use: weather routing sails with it, and the performance log learns into it alone.") } else { gettext("Not in use.") };
            if samples > 0 {
                sub.push(' ');
                sub.push_str(&gettext_f("{n} h logged against it.", &[("n", &format!("{:.1}", samples as f64 * 10.0 / 3600.0))]));
            }
            if builtin && modified {
                sub.push(' ');
                sub.push_str(&gettext("Changed from the published speeds."));
            }
            picker.set_subtitle(&sub);
            let enable = |name: &str, on: bool| {
                if let Some(a) = actions.lookup_action(name).and_downcast::<gtk::gio::SimpleAction>() {
                    a.set_enabled(on);
                }
            };
            enable("restore", builtin && (modified || s.dirty));
            enable("delete", !builtin);
        })
    };

    // Picking another polar shows it (after asking, when the shown one has unsaved changes).
    {
        let (app, state, area, refresh, sync, sync_picker, syncing) = (app.clone(), state.clone(), area.downgrade(), refresh.clone(), sync_learnt.clone(), sync_picker.clone(), syncing.clone());
        picker.connect_selected_notify(move |p| {
            if syncing.get() {
                return;
            }
            let Some(id) = state.borrow().list.polars.get(p.selected() as usize).map(|r| r.entry.id.clone()) else { return };
            if id == state.borrow().id {
                return;
            }
            let go = {
                let (app, state, area, refresh, sync, sync_picker) = (app.clone(), state.clone(), area.clone(), refresh.clone(), sync.clone(), sync_picker.clone());
                move || {
                    show_polar(&app, &mut state.borrow_mut(), &id);
                    refresh();
                    sync();
                    sync_picker();
                    if let Some(a) = area.upgrade() {
                        a.queue_draw();
                    }
                }
            };
            if !state.borrow().dirty {
                go();
                return;
            }
            let name = selected_row(&state.borrow()).map(|r| r.entry.name.clone()).unwrap_or_default();
            let sync_picker = sync_picker.clone();
            gtk::glib::spawn_future_local(async move {
                if super::confirm(&gettext_f("Discard the changes to {name}?", &[("name", &name)]), &gettext("The speeds you changed are not saved."), &gettext("Discard"), true).await {
                    go();
                } else {
                    // Back to the polar being edited.
                    sync_picker();
                }
            });
        });
    }
    {
        let (app, state, sync_picker, sync, area, watch) = (app.clone(), state.clone(), sync_picker.clone(), sync_learnt.clone(), area.downgrade(), watch_analysis.clone());
        use_it.connect_clicked(move |_| {
            let id = state.borrow().id.clone();
            match isomar_app::polars::activate(&app, &id) {
                Ok(()) => {
                    let mut s = state.borrow_mut();
                    s.list = isomar_app::polars::list(&app);
                    s.observed = None;
                    let name = selected_row(&s).map(|r| r.entry.name.clone()).unwrap_or_default();
                    drop(s);
                    app.toast(&gettext_f("{name} is in use: weather routing sails with it and the log learns into it.", &[("name", &name)]));
                    // Its own log, analysed again.
                    if isomar_app::flows::perf::stale(&app) {
                        isomar_app::flows::perf::analyse_now(&app);
                    }
                    if app.perf.borrow().analysing {
                        watch();
                    } else {
                        state.borrow_mut().observed = isomar_app::vm::performance::observed(&app);
                    }
                }
                Err(e) => show_error(&gettext("Could not use the polar"), &e),
            }
            sync_picker();
            sync();
            if let Some(a) = area.upgrade() {
                a.queue_draw();
            }
        });
    }

    // The menu: save as a new polar, the speeds of a published polar, back to the published speeds or delete.
    let set_from = gtk::gio::Menu::new();
    for b in &isomar_app::core::routing::library::BUILTINS {
        let item = gtk::gio::MenuItem::new(Some(&gettext(b.name)), None);
        item.set_action_and_target_value(Some("polar.set-from"), Some(&b.id.to_variant()));
        set_from.append_item(&item);
    }
    let first_section = gtk::gio::Menu::new();
    first_section.append(Some(&gettext("Save as a New Polar…")), Some("polar.save-as"));
    first_section.append_submenu(Some(&gettext("Set the Speeds From")), &set_from);
    menu.append_section(None, &first_section);
    let last_section = gtk::gio::Menu::new();
    last_section.append(Some(&gettext("Back to the Published Speeds")), Some("polar.restore"));
    last_section.append(Some(&gettext("Delete This Polar…")), Some("polar.delete"));
    menu.append_section(None, &last_section);

    let redraw = {
        let (refresh, sync, sync_picker, area) = (refresh.clone(), sync_learnt.clone(), sync_picker.clone(), area.downgrade());
        Rc::new(move || {
            refresh();
            sync();
            sync_picker();
            if let Some(a) = area.upgrade() {
                a.queue_draw();
            }
        })
    };
    {
        let action = gtk::gio::SimpleAction::new("set-from", Some(gtk::glib::VariantTy::STRING));
        let (state, redraw) = (state.clone(), redraw.clone());
        action.connect_activate(move |_, v| {
            let Some(id) = v.and_then(|v| v.get::<String>()) else { return };
            let Some(p) = isomar_app::core::routing::library::original(&id) else { return };
            {
                let mut s = state.borrow_mut();
                s.polar = p.resampled(&EDITOR_TWS);
                s.dirty = true;
            }
            redraw();
        });
        actions.add_action(&action);
    }
    {
        let action = gtk::gio::SimpleAction::new("save-as", None);
        let (app, state, redraw) = (app.clone(), state.clone(), redraw.clone());
        action.connect_activate(move |_, _| {
            let (app, state, redraw) = (app.clone(), state.clone(), redraw.clone());
            gtk::glib::spawn_future_local(async move {
                let Some(name) = super::ask_text(&gettext("Save as a new polar"), &gettext("Your polars are listed after the built-in ones. Make it the one in use with “Use this polar”."), "", &gettext("Save")).await else { return };
                let p = state.borrow().polar.clone();
                match isomar_app::polars::save_as(&app, &name, &p) {
                    Ok(id) => {
                        {
                            let mut s = state.borrow_mut();
                            s.list = isomar_app::polars::list(&app);
                            show_polar(&app, &mut s, &id);
                        }
                        app.toast(&gettext_f("Polar {name} saved", &[("name", &name)]));
                    }
                    Err(e) => show_error(&gettext("Could not save the polar"), &e),
                }
                redraw();
            });
        });
        actions.add_action(&action);
    }
    {
        let action = gtk::gio::SimpleAction::new("restore", None);
        let (app, state, redraw) = (app.clone(), state.clone(), redraw.clone());
        action.connect_activate(move |_, _| {
            let id = state.borrow().id.clone();
            if let Err(e) = isomar_app::polars::remove(&app, &id) {
                show_error(&gettext("Could not restore the polar"), &e);
            }
            {
                let mut s = state.borrow_mut();
                s.list = isomar_app::polars::list(&app);
                show_polar(&app, &mut s, &id);
            }
            redraw();
        });
        actions.add_action(&action);
    }
    {
        let action = gtk::gio::SimpleAction::new("delete", None);
        let (app, state, redraw) = (app.clone(), state.clone(), redraw.clone());
        action.connect_activate(move |_, _| {
            let (app, state, redraw) = (app.clone(), state.clone(), redraw.clone());
            gtk::glib::spawn_future_local(async move {
                let Some(row) = selected_row(&state.borrow()).cloned() else { return };
                let mut body = gettext("The polar is deleted for good.");
                if row.samples > 0 {
                    body.push(' ');
                    body.push_str(&gettext_f("So are the {h} h of the performance log measured against it.", &[("h", &format!("{:.1}", row.samples as f64 * 10.0 / 3600.0))]));
                }
                if row.active {
                    body.push(' ');
                    body.push_str(&gettext("The medium cruiser is used instead."));
                }
                if !super::confirm(&gettext_f("Delete {name}?", &[("name", &row.entry.name)]), &body, &gettext("Delete"), true).await {
                    return;
                }
                if let Err(e) = isomar_app::polars::remove(&app, &row.entry.id) {
                    show_error(&gettext("Could not delete the polar"), &e);
                }
                {
                    let mut s = state.borrow_mut();
                    s.list = isomar_app::polars::list(&app);
                    let active = s.list.active.clone();
                    show_polar(&app, &mut s, &active);
                }
                redraw();
            });
        });
        actions.add_action(&action);
    }
    win.insert_action_group("polar", Some(&actions));
    sync_picker();

    {
        let win = win.downgrade();
        cancel.connect_clicked(move |_| {
            if let Some(w) = win.upgrade() {
                w.close();
            }
        });
    }
    {
        let (win, app, state) = (win.downgrade(), app.clone(), state.clone());
        save.connect_clicked(move |_| {
            let (id, p, active) = {
                let s = state.borrow();
                (s.id.clone(), s.polar.clone(), s.list.active == s.id)
            };
            let name = selected_row(&state.borrow()).map(|r| r.entry.name.clone()).unwrap_or_default();
            match isomar_app::polars::save(&app, &id, &p) {
                Ok(()) if active => app.toast(&gettext_f("Polar {name} saved. The weather router will use it.", &[("name", &name)])),
                Ok(()) => app.toast(&gettext_f("Polar {name} saved. It is not the one in use.", &[("name", &name)])),
                Err(e) => show_error(&gettext("Could not save the polar"), &e),
            }
            if let Some(w) = win.upgrade() {
                w.close();
            }
        });
    }

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&body));
    win.set_content(Some(&toolbar));
    win.present();
}

fn draw(area: &gtk::DrawingArea, cr: &cairo::Context, w: f64, h: f64, s: &State) {
    let fg = area.color();
    let accent = adw::StyleManager::default().accent_color_rgba();
    let ink = |a: f64| cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, a);
    let tint = |a: f64| cr.set_source_rgba(accent.red() as f64, accent.green() as f64, accent.blue() as f64, a);
    let l = Layout::new(w, h);
    let scale = s.drag.as_ref().map(|d| d.scale).unwrap_or_else(|| scale_for(&s.polar).0);
    let step = scale_for(&s.polar).1;
    let p = &s.polar;

    cr.set_font_size((l.r / 20.0).clamp(11.0, 18.0));
    let text_at = |x: f64, y: f64, t: &str| {
        if let Ok(e) = cr.text_extents(t) {
            cr.move_to(x - e.width() / 2.0 - e.x_bearing(), y - e.height() / 2.0 - e.y_bearing());
            let _ = cr.show_text(t);
        }
    };

    // Rings, labelled with the speed along the top axis.
    cr.set_line_width(1.0);
    let rings = (scale / step).round() as i32;
    for k in 1..=rings {
        let kn = k as f64 * step;
        let rr = kn / scale * l.r;
        ink(0.15);
        cr.arc(l.cx, l.cy, rr, 0.0, 2.0 * PI);
        let _ = cr.stroke();
        ink(0.55);
        text_at(l.cx + 12.0, l.cy - rr + 7.0, &format!("{kn:.0}"));
    }
    // Spokes every 30°, with the angle outside the outer ring.
    for i in 0..12 {
        let a = (i as f64 * 30.0).to_radians();
        ink(0.10);
        cr.move_to(l.cx, l.cy);
        cr.line_to(l.cx + l.r * a.sin(), l.cy - l.r * a.cos());
        let _ = cr.stroke();
        let deg = if i <= 6 { i * 30 } else { (12 - i) * 30 };
        ink(0.7);
        text_at(l.cx + (l.r + 16.0) * a.sin(), l.cy - (l.r + 16.0) * a.cos(), &format!("{deg}°"));
    }

    let outline = |cr: &cairo::Context, col: usize| {
        let n = p.twa.len();
        for (k, row) in (0..n).chain((0..n).rev()).enumerate() {
            let side = if k < n { 1.0 } else { -1.0 };
            let (x, y) = l.point(scale, p.twa[row], p.speeds[row][col], side);
            if k == 0 {
                cr.move_to(x, y);
            } else {
                cr.line_to(x, y);
            }
        }
        cr.close_path();
    };

    // The other wind speeds, faint, so the shape can be compared.
    cr.set_line_width(1.0);
    for col in (0..p.tws.len()).filter(|c| *c != s.col) {
        ink(0.22);
        outline(cr, col);
        let _ = cr.stroke();
    }

    // What the performance log learnt in this wind: its dots, and the learnt polar dashed.
    if let Some(o) = &s.observed
        && let Some(lc) = o.tws.iter().position(|w| (w - p.tws[s.col]).abs() < 1e-6)
    {
        for (twa, kn, conf) in &o.cells[lc] {
            for side in [1.0, -1.0] {
                let (x, y) = l.point(scale, *twa, *kn, side);
                cr.arc(x, y, 2.5, 0.0, 2.0 * PI);
                ink(0.15 + 0.55 * conf);
                let _ = cr.fill();
            }
        }
        let n = o.twa.len();
        for (k, row) in (0..n).chain((0..n).rev()).enumerate() {
            let side = if k < n { 1.0 } else { -1.0 };
            let (x, y) = l.point(scale, o.twa[row], o.speeds[row][lc], side);
            if k == 0 {
                cr.move_to(x, y);
            } else {
                cr.line_to(x, y);
            }
        }
        cr.close_path();
        ink(0.7);
        cr.set_line_width(1.5);
        cr.set_dash(&[5.0, 4.0], 0.0);
        let _ = cr.stroke();
        cr.set_dash(&[], 0.0);
    }

    // The wind speed being edited.
    outline(cr, s.col);
    tint(0.14);
    let _ = cr.fill_preserve();
    tint(1.0);
    cr.set_line_width(2.5);
    cr.set_line_join(cairo::LineJoin::Round);
    let _ = cr.stroke();

    let active = s.drag.as_ref().map(|d| d.row).or(s.hover);
    for (row, a) in p.twa.iter().enumerate() {
        for side in [1.0, -1.0] {
            let (x, y) = l.point(scale, *a, p.speeds[row][s.col], side);
            let on = active == Some(row);
            cr.arc(x, y, if on { 7.0 } else { 4.5 }, 0.0, 2.0 * PI);
            tint(1.0);
            let _ = cr.fill_preserve();
            cr.set_line_width(1.5);
            cr.set_source_rgba(1.0, 1.0, 1.0, if on { 0.95 } else { 0.7 });
            let _ = cr.stroke();
        }
    }

    // The speed beside the point being moved.
    if let Some(d) = &s.drag {
        let kn = p.speeds[d.row][s.col];
        let (x, y) = l.point(scale, p.twa[d.row], kn, d.side);
        ink(1.0);
        text_at(x + d.side * 24.0, y - 14.0, &format!("{kn:.1}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout {
        Layout::new(500.0, 440.0)
    }

    #[test]
    fn points_follow_the_wind_angle_and_mirror() {
        let l = layout();
        let (x, y) = l.point(10.0, 0.0, 10.0, 1.0);
        assert!((x - l.cx).abs() < 1e-9 && (y - (l.cy - l.r)).abs() < 1e-9, "into the wind is straight up");
        let (x, y) = l.point(10.0, 90.0, 5.0, 1.0);
        assert!((x - (l.cx + l.r / 2.0)).abs() < 1e-9 && (y - l.cy).abs() < 1e-9);
        let (xp, yp) = l.point(10.0, 90.0, 5.0, -1.0);
        assert!((xp - (l.cx - l.r / 2.0)).abs() < 1e-9 && (yp - y).abs() < 1e-9);
        let (_, y) = l.point(10.0, 180.0, 5.0, 1.0);
        assert!(y > l.cy, "running is below the centre");
    }

    #[test]
    fn dragging_gives_back_the_speed_on_either_side() {
        let l = layout();
        for twa in [0.0, 40.0, 90.0, 135.0, 180.0] {
            for side in [1.0, -1.0] {
                let (x, y) = l.point(12.0, twa, 7.3, side);
                assert!((l.speed_at(12.0, twa, side, x, y) - 7.3).abs() < 1e-9, "{twa} {side}");
            }
        }
    }

    #[test]
    fn a_drag_slides_along_the_ray_and_stays_in_the_diagram() {
        let l = layout();
        // Pulling a 90° point straight down changes nothing: the ray is horizontal.
        let (x, y) = l.point(10.0, 90.0, 4.0, 1.0);
        assert!((l.speed_at(10.0, 90.0, 1.0, x, y + 80.0) - 4.0).abs() < 1e-9);
        // Past the centre or beyond the outer ring clamps.
        assert_eq!(l.speed_at(10.0, 90.0, 1.0, l.cx - 50.0, l.cy), 0.0);
        assert_eq!(l.speed_at(10.0, 90.0, 1.0, l.cx + 5000.0, l.cy), 10.0);
    }

    #[test]
    fn the_scale_leaves_room_above_the_fastest_point() {
        let p = Polar::generic_cruiser().resampled(&EDITOR_TWS);
        let (scale, step) = scale_for(&p);
        let max = p.speeds.iter().flatten().fold(0.0f64, |m, v| m.max(*v));
        assert!(scale > max && scale - max <= step + 0.5, "{scale} {max}");
        assert_eq!(scale % step, 0.0);
        let slow = Polar::parse("TWA;5\n0;0\n90;2").unwrap();
        assert_eq!(scale_for(&slow).0, 6.0);
    }

    #[test]
    fn the_nearest_point_within_reach_is_grabbed() {
        let l = layout();
        let p = Polar::generic_cruiser().resampled(&EDITOR_TWS);
        let (scale, _) = scale_for(&p);
        let row = p.twa.iter().position(|a| *a == 90.0).unwrap();
        let (x, y) = l.point(scale, 90.0, p.speeds[row][1], 1.0);
        assert_eq!(hit_test(&l, scale, &p, 1, x + 3.0, y - 3.0), Some((row, 1.0)));
        let (x, y) = l.point(scale, 90.0, p.speeds[row][1], -1.0);
        assert_eq!(hit_test(&l, scale, &p, 1, x, y), Some((row, -1.0)));
        assert_eq!(hit_test(&l, scale, &p, 1, l.cx + 3.0, l.cy - 200.0), None);
    }
}
