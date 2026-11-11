//! The Instruments page: every live readout, the dashboard that floats over the chart (which
//! figures, and whether it stays pinned in planning), and the anchor watch.

use super::scrolled;
use crate::i18n::gettext;
use crate::ui::app::AppState;
use adw::prelude::*;
use serde_json::json;
use std::cell::Cell;
use std::rc::Rc;

/// The figures the dashboard can show, by `vm::underway::readout` id.
fn dashboard_choices() -> Vec<(&'static str, String)> {
    isomar_app::vm::underway::choices()
}

pub fn build(app: &Rc<AppState>) -> gtk::Widget {
    let page = super::column();
    page.append(&sail_switch(app));
    let grid = gtk::Grid::builder().column_spacing(8).row_spacing(8).column_homogeneous(true).build();
    page.append(&grid);
    let titles = isomar_app::vm::instruments::titles();
    let mut tiles = Vec::new();
    for (i, t) in titles.iter().enumerate() {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
        b.add_css_class("card");
        b.add_css_class("instrument-tile");
        let title = gtk::Label::builder().label(t).xalign(0.0).wrap(true).css_classes(["caption-heading", "dim-label"]).build();
        let value = gtk::Label::builder().label("—").xalign(0.0).wrap(true).css_classes(["readout-m"]).build();
        let unit = gtk::Label::builder().xalign(0.0).css_classes(["caption", "dim-label"]).build();
        b.append(&title);
        b.append(&value);
        b.append(&unit);
        // The position spans the page.
        let (col, row, span) = if i == 8 { (0, 4, 2) } else if i > 8 { (((i - 9) % 2) as i32, 5 + ((i - 9) / 2) as i32, 1) } else { ((i % 2) as i32, (i / 2) as i32, 1) };
        grid.attach(&b, col, row, span, 1);
        tiles.push((value, unit));
    }
    {
        let app2 = Rc::downgrade(app);
        let update = move || {
            let Some(app) = app2.upgrade() else { return };
            for ((v, u), t) in tiles.iter().zip(isomar_app::vm::instruments::tiles(&app)) {
                v.set_text(&t.value);
                u.set_text(&t.unit);
            }
        };
        update();
        app.on_tick(update);
    }

    // The dashboard.
    let dash = adw::PreferencesGroup::builder().title(gettext("Dashboard")).description(gettext("Big figures over the chart, docked bottom left. Under way it always shows.")).build();
    let pin = adw::SwitchRow::builder().title(gettext("Show over the chart while planning")).active(app.settings.borrow().dashboard_on_chart).build();
    {
        let app = app.clone();
        pin.connect_active_notify(move |r| {
            isomar_app::api::call(&app, "settings_patch", &json!({ "dashboard_on_chart": r.is_active() }));
        });
    }
    dash.add(&pin);
    let figures = adw::ExpanderRow::builder().title(gettext("Figures")).subtitle(gettext("Up to four read best")).build();
    let chosen = isomar_app::vm::underway::dashboard_ids(app);
    let checks: Rc<Vec<(&'static str, gtk::CheckButton)>> = Rc::new(
        dashboard_choices()
            .into_iter()
            .map(|(id, label)| {
                let check = gtk::CheckButton::builder().active(chosen.iter().any(|c| c == id)).valign(gtk::Align::Center).build();
                let row = adw::ActionRow::builder().title(label).activatable_widget(&check).build();
                row.add_prefix(&check);
                figures.add_row(&row);
                (id, check)
            })
            .collect(),
    );
    let syncing = Rc::new(Cell::new(false));
    for (_, c) in checks.iter() {
        let (app, checks, syncing) = (Rc::downgrade(app), Rc::downgrade(&checks), syncing.clone());
        c.connect_toggled(move |_| {
            let (Some(app), Some(checks)) = (app.upgrade(), checks.upgrade()) else { return };
            if syncing.get() {
                return;
            }
            let ids: Vec<&str> = checks.iter().filter(|(_, c)| c.is_active()).map(|(id, _)| *id).collect();
            isomar_app::api::call(&app, "settings_patch", &json!({ "dashboard": ids }));
        });
    }
    dash.add(&figures);
    page.append(&dash);

    // The performance log.
    let perf = adw::PreferencesGroup::builder().title(gettext("Performance")).build();
    let row = adw::ActionRow::builder()
        .title(gettext("Performance and polar"))
        .subtitle(gettext("What the boat does in each wind, learnt from the log"))
        .activatable(true)
        .action_name("win.performance")
        .build();
    row.add_prefix(&gtk::Image::from_icon_name("isomar-polar-symbolic"));
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    perf.add(&row);
    page.append(&perf);

    // Anchor watch.
    let anchor = adw::PreferencesGroup::builder().title(gettext("Anchor watch")).description(gettext("An alarm if the boat leaves the circle. The radius is in Preferences, Alarms.")).build();
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.append(&gtk::Button::builder().child(&adw::ButtonContent::builder().icon_name("isomar-anchor-symbolic").label(gettext("Set here")).build()).action_name("win.anchor-here").build());
    buttons.append(&gtk::Button::builder().label(gettext("Clear")).action_name("win.anchor-clear").build());
    anchor.add(&buttons);
    page.append(&anchor);
    scrolled(&page).upcast()
}

/// Sailing or motoring: the user's switch, or the engine's word while one reports. What the
/// performance log learns from, so the time under engine does not count as sailing.
pub fn sail_switch(app: &Rc<AppState>) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let toggles = adw::ToggleGroup::builder().homogeneous(true).build();
    toggles.add(adw::Toggle::builder().name("sailing").label(gettext("Sailing")).build());
    toggles.add(adw::Toggle::builder().name("motoring").label(gettext("Motoring")).build());
    let note = gtk::Label::builder().xalign(0.5).wrap(true).css_classes(["caption", "dim-label"]).build();
    b.append(&toggles);
    b.append(&note);
    let syncing = Rc::new(Cell::new(false));
    let sync = {
        let (app, toggles, note, syncing) = (Rc::downgrade(app), toggles.downgrade(), note.downgrade(), syncing.clone());
        move || {
            let (Some(app), Some(toggles), Some(note)) = (app.upgrade(), toggles.upgrade(), note.upgrade()) else { return };
            let s = isomar_app::vm::performance::switch(&app);
            syncing.set(true);
            if s.mode.is_empty() {
                toggles.set_active(gtk::INVALID_LIST_POSITION);
            } else if toggles.active_name().as_deref() != Some(s.mode.as_str()) {
                toggles.set_active_name(Some(&s.mode));
            }
            syncing.set(false);
            toggles.set_sensitive(!s.by_engine);
            let text = s.note.or(s.hint).unwrap_or_default();
            note.set_text(&text);
            note.set_visible(!text.is_empty());
        }
    };
    sync();
    app.on_tick(sync);
    {
        let app = Rc::downgrade(app);
        toggles.connect_active_name_notify(move |t| {
            if syncing.get() {
                return;
            }
            let (Some(app), Some(name)) = (app.upgrade(), t.active_name()) else { return };
            let r = isomar_app::api::call(&app, "perf_set_mode", &json!({ "mode": name.as_str() }));
            if let Some(e) = r.get("error").and_then(|e| e.as_str()) {
                app.toast(e);
            }
        });
    }
    b.upcast()
}
