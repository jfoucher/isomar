//! The Weather page: what forecast is loaded and how old it is, getting one for the view or for an
//! area drawn on the chart, what to show, and the tides nearby. The time control is the timeline
//! docked over the chart.

use super::scrolled;
use crate::i18n::gettext;
use crate::ui::app::AppState;
use adw::prelude::*;
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub fn build(app: &Rc<AppState>) -> gtk::Widget {
    let page = super::column();

    let forecast = adw::PreferencesGroup::builder().title(gettext("Forecast")).build();
    let summary = adw::ActionRow::builder().title(gettext("No forecast yet")).subtitle(gettext("Get weather for what is on screen, or draw an area on the chart.")).subtitle_lines(3).build();
    summary.add_prefix(&gtk::Image::from_icon_name("weather-few-clouds-symbolic"));
    // The model shown, and routed on: each keeps its own forecast.
    let models = gtk::StringList::new(&[]);
    let model = adw::ComboRow::builder().title(gettext("Model")).model(&models).build();
    forecast.add(&model);
    forecast.add(&summary);
    page.append(&forecast);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    // While a download runs it is off, with a spinner, and pressing it shows the running jobs.
    let get = crate::ui::busy::icon_button(app, "weather", "folder-download-symbolic", &gettext("Get weather…"));
    get.add_css_class("suggested-action");
    {
        let app = Rc::downgrade(app);
        get.connect_clicked(move |b| {
            if app.upgrade().is_some_and(|app| crate::ui::busy::pressed(&app, b, "weather")) {
                return;
            }
            let _ = b.activate_action("win.get-weather", None);
        });
    }
    let draw = gtk::Button::builder().child(&adw::ButtonContent::builder().icon_name("selection-mode-symbolic").label(gettext("Draw an area")).build()).tooltip_text(gettext("Drag a box on the chart")).build();
    buttons.append(&get);
    buttons.append(&draw);
    page.append(&buttons);
    {
        let app = app.clone();
        draw.connect_clicked(move |_| isomar_app::weather::start_pick(&app));
    }

    // One field colours the sea, chosen like a radio; barbs, wave arrows and isobars go over it. These
    // were four switches of which two turned each other off.
    let show = adw::PreferencesGroup::builder().title(gettext("Show")).build();
    let syncing = Rc::new(Cell::new(false));
    let fill = adw::ToggleGroup::builder().homogeneous(false).build();
    for (name, label) in [("none", gettext("None")), ("wind", gettext("Wind")), ("waves", gettext("Waves")), ("current", gettext("Current"))] {
        fill.add(adw::Toggle::builder().name(name).label(label).build());
    }
    // The toggles under the title, full width: beside it the sidebar is too narrow for their labels.
    let fill_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).margin_top(10).margin_bottom(12).margin_start(12).margin_end(12).build();
    fill_box.append(&gtk::Label::builder().label(gettext("Colour")).xalign(0.0).build());
    let fill_note = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["caption", "dim-label"]).build();
    fill_box.append(&fill_note);
    fill.set_margin_top(4);
    fill_box.append(&fill);
    show.add(&gtk::ListBoxRow::builder().child(&fill_box).activatable(false).selectable(false).build());
    {
        let (app, syncing) = (app.clone(), syncing.clone());
        fill.connect_active_name_notify(move |t| {
            if let (false, Some(name)) = (syncing.get(), t.active_name()) {
                isomar_app::api::call(&app, "weather_fill", &json!({ "fill": name.as_str() }));
            }
        });
    }
    let mut switches = Vec::new();
    for (key, label) in [("barbs", gettext("Wind barbs")), ("wave_arrows", gettext("Wave arrows")), ("pressure", gettext("Isobars"))] {
        let row = adw::SwitchRow::builder().title(label).build();
        let (app2, syncing) = (app.clone(), syncing.clone());
        row.connect_active_notify(move |_| {
            if !syncing.get() {
                isomar_app::api::call(&app2, "weather_toggle", &json!({ "name": key }));
            }
        });
        show.add(&row);
        switches.push((key, row));
    }
    page.append(&show);

    // Currents: their own source, shared by every model.
    let currents = adw::PreferencesGroup::builder().title(gettext("Currents")).build();
    let currents_summary = adw::ActionRow::builder().title(gettext("No current loaded")).subtitle_lines(4).build();
    currents_summary.add_prefix(&gtk::Image::from_icon_name("isomar-current-symbolic"));
    currents.add(&currents_summary);
    let current_arrows = adw::SwitchRow::builder().title(gettext("Current arrows")).build();
    let route_with = adw::SwitchRow::builder().title(gettext("Sail in the current")).subtitle(gettext("Weather routes and best departures")).build();
    {
        let (app, syncing) = (app.clone(), syncing.clone());
        current_arrows.connect_active_notify(move |r| {
            if !syncing.get() {
                isomar_app::api::call(&app, "currents_arrows", &json!({ "on": r.is_active() }));
            }
        });
    }
    {
        let (app, syncing) = (app.clone(), syncing.clone());
        route_with.connect_active_notify(move |r| {
            if !syncing.get() {
                isomar_app::api::call(&app, "currents_route_with", &json!({ "on": r.is_active() }));
            }
        });
    }
    let get_currents = adw::ButtonRow::builder().title(gettext("Get currents for this view")).start_icon_name("folder-download-symbolic").build();
    {
        let app = app.clone();
        get_currents.connect_activated(move |_| {
            isomar_app::api::call(&app, "currents_fetch", &Value::Null);
        });
    }
    currents.add(&get_currents);
    currents.add(&current_arrows);
    currents.add(&route_with);
    let atlas_rows: RefCell<Vec<adw::ActionRow>> = RefCell::default();
    page.append(&currents);

    let manage = adw::PreferencesGroup::builder().build();
    let clear = adw::ButtonRow::builder().title(gettext("Clear the forecast")).start_icon_name("user-trash-symbolic").css_classes(["destructive-action"]).build();
    {
        let app = app.clone();
        clear.connect_activated(move |_| {
            isomar_app::api::call(&app, "weather_clear", &Value::Null);
        });
    }
    manage.add(&clear);
    page.append(&manage);

    // Under way, each loaded model against the wind the instruments measure.
    let tracking = adw::PreferencesGroup::builder()
        .title(gettext("Forecasts against your wind"))
        .description(gettext("Each loaded model against the true wind your instruments measured, over the last six hours."))
        .visible(false)
        .build();
    page.append(&tracking);
    let tracking_rows: RefCell<Vec<adw::ActionRow>> = RefCell::default();

    let tides = adw::PreferencesGroup::builder().title(gettext("Tides")).build();
    let tides_row = adw::ActionRow::builder().title(gettext("Tides nearby")).subtitle(gettext("High and low waters at the station nearest the centre of the chart")).activatable(true).action_name("win.tides-here").build();
    tides_row.add_prefix(&gtk::Image::from_icon_name("isomar-tide-symbolic"));
    tides_row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    tides.add(&tides_row);
    page.append(&tides);

    let ids: Rc<RefCell<Vec<String>>> = Rc::default();
    {
        let (app, syncing, ids) = (app.clone(), syncing.clone(), ids.clone());
        model.connect_selected_notify(move |c| {
            if syncing.get() {
                return;
            }
            if let Some(id) = ids.borrow().get(c.selected() as usize) {
                isomar_app::api::call(&app, "weather_show_model", &json!({ "model": id }));
            }
        });
    }
    let shown: RefCell<Value> = RefCell::new(Value::Null);
    let refresh = {
        let app = Rc::downgrade(app);
        move || {
            let Some(app) = app.upgrade() else { return };
            let w = isomar_app::api::call(&app, "weather_page", &Value::Null);
            if *shown.borrow() == w {
                return;
            }
            let loaded = w.get("loaded").and_then(|v| v.as_bool()).unwrap_or(false);
            let s = |k: &str| w.get(k).and_then(|v| v.as_str()).map(str::to_string);
            syncing.set(true);
            let choices = w.get("models").and_then(|m| m.as_array()).cloned().unwrap_or_default();
            let label = |c: &Value| {
                let l = c.get("label").and_then(|v| v.as_str()).unwrap_or_default();
                if c.get("loaded").and_then(|v| v.as_bool()).unwrap_or(false) { crate::i18n::gettext_f("{model} (loaded)", &[("model", l)]) } else { l.to_string() }
            };
            let labels: Vec<String> = choices.iter().map(label).collect();
            models.splice(0, models.n_items(), &labels.iter().map(String::as_str).collect::<Vec<_>>());
            *ids.borrow_mut() = choices.iter().filter_map(|c| c.get("id").and_then(|v| v.as_str()).map(str::to_string)).collect();
            if let Some(i) = choices.iter().position(|c| c.get("shown").and_then(|v| v.as_bool()).unwrap_or(false)) {
                model.set_selected(i as u32);
                forecast.set_description(choices[i].get("about").and_then(|v| v.as_str()));
            }
            syncing.set(false);
            if loaded {
                summary.set_title(&crate::i18n::gettext_f("{model} loaded", &[("model", &s("model_label").unwrap_or_default())]));
                let mut lines = Vec::new();
                if let Some(r) = s("run") {
                    lines.push(crate::i18n::gettext_f("Run {run}", &[("run", &r)]));
                }
                if let (Some(a), Some(b)) = (s("from"), s("to")) {
                    lines.push(format!("{a} – {b}"));
                }
                summary.set_subtitle(&lines.join("\n"));
            } else {
                summary.set_title(&gettext("No forecast yet"));
                summary.set_subtitle(&gettext("Get weather for what is on screen, or draw an area on the chart."));
            }
            manage.set_visible(loaded);
            let tracked = w.get("tracking").and_then(|t| t.as_array()).cloned().unwrap_or_default();
            for r in tracking_rows.borrow_mut().drain(..) {
                tracking.remove(&r);
            }
            for t in &tracked {
                let label = t.get("label").and_then(|v| v.as_str()).unwrap_or_default();
                let best = t.get("best").and_then(|v| v.as_bool()).unwrap_or(false);
                let title = if best { crate::i18n::gettext_f("{model} · closest", &[("model", label)]) } else { label.to_string() };
                let row = adw::ActionRow::builder().title(title).subtitle(t.get("text").and_then(|v| v.as_str()).unwrap_or_default()).subtitle_lines(3).build();
                tracking.add(&row);
                tracking_rows.borrow_mut().push(row);
            }
            tracking.set_visible(!tracked.is_empty());
            let c = w.get("currents").cloned().unwrap_or(Value::Null);
            let cs = |k: &str| c.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
            let sources = c.get("sources").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            // Under way, each source against the stream measured.
            let tracked: Vec<String> = c.get("tracking").and_then(|v| v.as_array()).into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect();
            currents.set_description(Some(&std::iter::once(cs("note")).chain(tracked).collect::<Vec<_>>().join("\n")));
            if sources.is_empty() {
                currents_summary.set_title(&gettext("No current loaded"));
                currents_summary.set_subtitle("");
            } else {
                let line = |v: &Value| format!("{} · {}", v.get("label").and_then(|x| x.as_str()).unwrap_or_default(), v.get("detail").and_then(|x| x.as_str()).unwrap_or_default());
                currents_summary.set_title(&gettext("Currents loaded"));
                currents_summary.set_subtitle(&sources.iter().map(line).collect::<Vec<_>>().join("\n"));
            }
            for r in atlas_rows.borrow_mut().drain(..) {
                currents.remove(&r);
            }
            for a in c.get("atlases").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
                let get = |k: &str| a.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
                let installed = a.get("installed").and_then(|v| v.as_bool()).unwrap_or(false);
                let row = adw::ActionRow::builder().title(crate::i18n::gettext_f("Shom atlas: {zone}", &[("zone", &get("name"))])).subtitle(get("detail")).build();
                let (verb, icon, tip) = if installed { ("atlas_remove", "user-trash-symbolic", gettext("Remove")) } else { ("atlas_install", "folder-download-symbolic", gettext("Download")) };
                let b = gtk::Button::builder().icon_name(icon).tooltip_text(tip).valign(gtk::Align::Center).css_classes(["flat"]).build();
                {
                    let (app, id) = (app.clone(), get("id"));
                    b.connect_clicked(move |_| {
                        isomar_app::api::call(&app, verb, &json!({ "id": id }));
                    });
                }
                row.add_suffix(&b);
                currents.add(&row);
                atlas_rows.borrow_mut().push(row);
            }
            syncing.set(true);
            current_arrows.set_active(c.get("arrows").and_then(|v| v.as_bool()).unwrap_or(true));
            route_with.set_active(c.get("route_with").and_then(|v| v.as_bool()).unwrap_or(true));
            if let Some(f) = s("fill") {
                fill.set_active_name(Some(&f));
            }
            // A forecast without waves: say why the wave colours and arrows show nothing.
            fill_note.set_text(&s("no_waves").filter(|_| s("fill").as_deref() == Some("waves")).unwrap_or_else(|| gettext("What colours the sea, under the arrows and lines")));
            for (key, row) in &switches {
                row.set_active(w.get(*key).and_then(|v| v.as_bool()).unwrap_or(false));
                if *key == "wave_arrows" {
                    row.set_subtitle(&s("no_waves").unwrap_or_default());
                }
            }
            syncing.set(false);
            *shown.borrow_mut() = w;
        }
    };
    refresh();
    app.on_refresh(refresh);
    scrolled(&page).upcast()
}
