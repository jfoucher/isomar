//! The details pushed inside a sidebar page: a route or a mark (Passage) and a vessel (Traffic).
//! They read and save through `isomar_app::api` (`route_get`, `route_save`, …), the same records the
//! macOS shell uses.

use super::scrolled;
use crate::i18n::gettext;
use crate::ui::app::AppState;
use crate::ui::datetime::DateTimeRow;
use adw::prelude::*;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;

fn call(app: &Rc<AppState>, name: &str, args: Value) -> Value {
    isomar_app::api::call(app, name, &args)
}

/// Shows the error an `api` answer carries, if any; true when it went through.
fn ok(answer: &Value, heading: &str) -> bool {
    match answer.get("error").and_then(|e| e.as_str()) {
        Some(e) => {
            crate::ui::show_error(heading, e);
            false
        }
        None => true,
    }
}

fn str_of(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string()
}

fn page(title: &str, content: &gtk::Box) -> adw::NavigationPage {
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&adw::HeaderBar::builder().show_end_title_buttons(false).show_start_title_buttons(false).build());
    tv.set_content(Some(&scrolled(content)));
    adw::NavigationPage::builder().title(title).child(&tv).build()
}

fn button(icon: &str, label: &str) -> gtk::Button {
    gtk::Button::builder().child(&adw::ButtonContent::builder().icon_name(icon).label(label).build()).build()
}

/// The detail of route `id`: follow it, plan a passage on it, its properties and legs.
pub fn route(app: &Rc<AppState>, id: i64) -> Option<adw::NavigationPage> {
    let r = call(app, "route_get", json!({ "id": id }));
    if r.is_null() {
        return None;
    }
    let content = super::column();
    content.append(&crate::ui::chips::line(&isomar_app::chips::from_json(&r["chips"]), &["dim-label"]));

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let follow = button("media-playback-start-symbolic", &gettext("Follow"));
    follow.add_css_class("suggested-action");
    follow.set_action_name(Some("win.activate-route"));
    follow.set_action_target_value(Some(&id.to_variant()));
    let edit = button("document-edit-symbolic", &gettext("Edit on the chart"));
    edit.set_action_name(Some("win.edit-route"));
    edit.set_action_target_value(Some(&id.to_variant()));
    actions.append(&follow);
    actions.append(&edit);
    content.append(&actions);

    let plan = adw::PreferencesGroup::builder().title(gettext("Plan a passage")).build();
    for (icon, title, subtitle, action) in [
        ("isomar-isochrone-symbolic", gettext("Weather routing"), gettext("The fastest way with the forecast and the boat's polar, and when to leave"), "win.weather-routing"),
        ("isomar-route-symbolic", gettext("Auto route…"), gettext("Adds waypoints to keep the route in safe water"), "win.auto-route"),
    ] {
        let row = adw::ActionRow::builder().title(title).subtitle(subtitle).activatable(true).build();
        row.add_prefix(&gtk::Image::from_icon_name(icon));
        if action == "win.auto-route" {
            // Its search, while it runs (`win.auto-route` then shows the running jobs).
            crate::ui::busy::row(app, &row, &format!("autoroute:{id}"));
        }
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        row.set_action_name(Some(action));
        row.set_action_target_value(Some(&id.to_variant()));
        plan.add(&row);
    }
    // The route sailed through NOAA's 31-member ensemble: how sure its arrival is.
    let ensemble = adw::ActionRow::builder()
        .title(gettext("Ensemble check"))
        .subtitle(gettext("Sails the route through 31 GEFS forecasts for the spread of its arrival and wind; a few minutes' download"))
        .activatable(true)
        .build();
    ensemble.add_prefix(&gtk::Image::from_icon_name("weather-few-clouds-symbolic"));
    let key = format!("ensemble:{id}");
    crate::ui::busy::row(app, &ensemble, &key);
    {
        let app = app.clone();
        ensemble.connect_activated(move |row| {
            if crate::ui::busy::pressed(&app, row, &key) {
                return;
            }
            call(&app, "route_ensemble", json!({ "id": id }));
        });
    }
    plan.add(&ensemble);
    content.append(&plan);
    let checked: Vec<String> = r.get("ensemble").and_then(|e| e.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
    if !checked.is_empty() {
        let g = adw::PreferencesGroup::builder().title(gettext("Ensemble")).build();
        let row = adw::ActionRow::builder().title(checked.join("\n")).title_lines(8).css_classes(["property"]).build();
        g.add(&row);
        content.append(&g);
    }

    if let Some(plan) = r.get("plan").filter(|p| p.is_object()) {
        content.append(&planned(app, id, plan));
    }

    let props = adw::PreferencesGroup::builder().title(gettext("Route")).build();
    let name = adw::EntryRow::builder().title(gettext("Name")).text(str_of(&r, "name")).build();
    let speed = adw::SpinRow::with_range(0.5, 40.0, 0.5);
    speed.set_title(&gettext("Planned speed"));
    speed.set_subtitle(&gettext("knots"));
    speed.set_digits(1);
    speed.set_value(r.get("speed_kn").and_then(|v| v.as_f64()).unwrap_or(5.0));
    let departure = DateTimeRow::new(&gettext("Departure"), isomar_app::api::parse_departure(&str_of(&r, "departure")).ok().flatten(), true);
    let visible = adw::SwitchRow::builder().title(gettext("Show on the chart")).active(r.get("visible").and_then(|v| v.as_bool()).unwrap_or(true)).build();
    props.add(&name);
    props.add(&speed);
    props.add(&departure.row);
    props.add(&visible);
    // Save is offered only once something differs from what is saved.
    let save = gtk::Button::builder().label(gettext("Save")).css_classes(["suggested-action"]).halign(gtk::Align::End).margin_top(6).sensitive(false).build();
    // Weak: the departure row's own change handler holds this.
    let form = {
        let (name, speed, departure, visible) = (name.clone(), speed.clone(), Rc::downgrade(&departure), visible.clone());
        Rc::new(move || {
            let departure = departure.upgrade().map(|d| d.form_text()).unwrap_or_default();
            json!({ "name": name.text().to_string(), "speed_kn": speed.value(), "departure": departure, "visible": visible.is_active() })
        })
    };
    let saved = Rc::new(RefCell::new(form()));
    let dirty = {
        let (save, form, saved) = (save.clone(), form.clone(), saved.clone());
        Rc::new(move || save.set_sensitive(form() != *saved.borrow()))
    };
    {
        let d = dirty.clone();
        name.connect_changed(move |_| d());
        let d = dirty.clone();
        speed.connect_value_notify(move |_| d());
        let d = dirty.clone();
        visible.connect_active_notify(move |_| d());
        let d = dirty.clone();
        departure.connect_changed(move || d());
    }
    {
        let app = app.clone();
        save.connect_clicked(move |_| {
            let mut args = form();
            args["id"] = json!(id);
            let a = call(&app, "route_save", args);
            if ok(&a, &gettext("Could not save the route")) {
                *saved.borrow_mut() = form();
                dirty();
                app.toast(&gettext("Route saved"));
            }
        });
    }
    content.append(&props);
    content.append(&save);

    let legs = adw::PreferencesGroup::builder().title(gettext("Legs")).build();
    for l in r.get("legs").and_then(|l| l.as_array()).cloned().unwrap_or_default() {
        let row = adw::ActionRow::builder().title(format!("{} → {}", str_of(&l, "from"), str_of(&l, "to"))).build();
        row.add_suffix(&gtk::Label::builder().label(str_of(&l, "text")).css_classes(["numeric", "dim-label"]).build());
        legs.add(&row);
    }
    content.append(&legs);

    let more = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    for (icon, label, action) in [
        ("object-flip-horizontal-symbolic", gettext("Reverse"), "win.reverse-route"),
        ("document-save-symbolic", gettext("Export GPX…"), "win.export-route"),
        ("user-trash-symbolic", gettext("Delete…"), "win.delete-route"),
    ] {
        let b = button(icon, &label);
        b.set_action_name(Some(action));
        b.set_action_target_value(Some(&id.to_variant()));
        if action == "win.delete-route" {
            b.add_css_class("destructive-action");
        }
        more.append(&b);
    }
    content.append(&more);
    Some(page(&str_of(&r, "name"), &content))
}

/// What a weather route was planned with: how the passage was expected to go, its statistics, and
/// playing it back on the forecast it was routed on.
fn planned(app: &Rc<AppState>, id: i64, plan: &Value) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title(gettext("Weather routing")).build();
    let edited = plan.get("edited").and_then(|e| e.as_bool()).unwrap_or(false);
    let mut forecast = str_of(plan, "forecast");
    if edited {
        forecast = format!("{forecast}. {}", gettext("The route has been edited since: this is the passage as it was planned."));
    }
    // Routed on several forecasts: the spread of its arrival over them.
    let bracket = str_of(plan, "bracket");
    if !bracket.is_empty() {
        forecast = format!("{forecast}. {bracket}.");
    }
    // The passage in chips where the title would be, what it was planned on under it.
    let row = adw::ActionRow::new();
    let text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(3).hexpand(true).valign(gtk::Align::Center).margin_top(6).margin_bottom(6).build();
    let summary = isomar_app::chips::from_json(&plan["summary"]);
    text.append(&crate::ui::chips::line(&summary, &[]));
    text.append(&gtk::Label::builder().label(&forecast).xalign(0.0).wrap(true).css_classes(["chips-subtitle"]).build());
    row.add_prefix(&text);
    row.upcast_ref::<gtk::Widget>().update_property(&[gtk::accessible::Property::Label(&format!("{}. {forecast}", isomar_app::chips::plain(&summary)))]);
    let stats = gtk::Button::builder().label(gettext("Statistics")).valign(gtk::Align::Center).build();
    let replay = gtk::Button::builder().label(gettext("Play")).valign(gtk::Align::Center).css_classes(["suggested-action"]).build();
    {
        let app = app.clone();
        stats.connect_clicked(move |_| {
            if let Some((heads, sections)) = isomar_app::vm::plan::stats(&app, id) {
                let heads: Vec<String> = heads.iter().map(|h| format!("<b>{}</b>", gtk::glib::markup_escape_text(h))).collect();
                crate::ui::routing_stats::show_sections(&gettext("Route statistics"), &heads, &sections);
            }
        });
    }
    {
        let app = app.clone();
        replay.connect_clicked(move |_| {
            if call(&app, "route_replay", json!({ "id": id })).get("ok").and_then(|o| o.as_bool()) != Some(true) {
                app.toast(&gettext("This route has no plan to play back."));
            }
        });
    }
    row.add_suffix(&stats);
    row.add_suffix(&replay);
    group.add(&row);
    group
}

/// The detail of mark `id`: navigate to it, its name, position, arrival circle and notes.
pub fn waypoint(app: &Rc<AppState>, id: i64) -> Option<adw::NavigationPage> {
    let w = call(app, "waypoint_get", json!({ "id": id }));
    if w.is_null() {
        return None;
    }
    let content = super::column();
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let go = button("isomar-follow-on-symbolic", &gettext("Navigate to"));
    go.add_css_class("suggested-action");
    go.set_action_name(Some("win.goto-waypoint"));
    go.set_action_target_value(Some(&id.to_variant()));
    let del = button("user-trash-symbolic", &gettext("Delete…"));
    del.add_css_class("destructive-action");
    del.set_action_name(Some("win.delete-waypoint"));
    del.set_action_target_value(Some(&id.to_variant()));
    actions.append(&go);
    actions.append(&del);
    content.append(&actions);

    let g = adw::PreferencesGroup::builder().title(gettext("Mark")).build();
    let name = adw::EntryRow::builder().title(gettext("Name")).text(str_of(&w, "name")).build();
    let lat = adw::EntryRow::builder().title(gettext("Latitude")).text(str_of(&w, "latitude")).build();
    let lon = adw::EntryRow::builder().title(gettext("Longitude")).text(str_of(&w, "longitude")).build();
    let radius = adw::SpinRow::with_range(0.0, 5.0, 0.01);
    radius.set_title(&gettext("Arrival circle"));
    radius.set_subtitle(&gettext("nautical miles"));
    radius.set_digits(2);
    radius.set_value(w.get("arrival_radius_nm").and_then(|v| v.as_f64()).unwrap_or(0.05));
    let notes = adw::EntryRow::builder().title(gettext("Notes")).text(str_of(&w, "description")).build();
    let visible = adw::SwitchRow::builder().title(gettext("Show on the chart")).active(w.get("visible").and_then(|v| v.as_bool()).unwrap_or(true)).build();
    for r in [name.upcast_ref::<gtk::Widget>(), lat.upcast_ref(), lon.upcast_ref(), radius.upcast_ref(), notes.upcast_ref(), visible.upcast_ref()] {
        g.add(r);
    }
    content.append(&g);
    let save = gtk::Button::builder().label(gettext("Save")).css_classes(["suggested-action"]).halign(gtk::Align::End).build();
    {
        let app = app.clone();
        save.connect_clicked(move |_| {
            let a = call(
                &app,
                "waypoint_save",
                json!({ "id": id, "name": name.text().to_string(), "latitude": lat.text().to_string(), "longitude": lon.text().to_string(),
                        "description": notes.text().to_string(), "arrival_radius_nm": radius.value(), "visible": visible.is_active() }),
            );
            if ok(&a, &gettext("Could not save the mark")) {
                app.toast(&gettext("Mark saved"));
            }
        });
    }
    content.append(&save);
    Some(page(&str_of(&w, "name"), &content))
}

/// Fills `content` with what is known of vessel `mmsi`; false once it is gone.
fn fill_vessel(app: &Rc<AppState>, mmsi: u32, content: &gtk::Box) -> bool {
    let v = call(app, "vessel_get", json!({ "id": mmsi }));
    if v.is_null() {
        return false;
    }
    super::clear(content);
    for g in v.get("groups").and_then(|g| g.as_array()).cloned().unwrap_or_default() {
        let title = str_of(&g, "title");
        let group = adw::PreferencesGroup::builder().title(if title.is_empty() { gettext("Vessel") } else { title }).build();
        for r in g.get("rows").and_then(|r| r.as_array()).cloned().unwrap_or_default() {
            let label = r.get(0).and_then(|x| x.as_str()).unwrap_or_default();
            let value = r.get(1).and_then(|x| x.as_str()).unwrap_or_default();
            let row = adw::ActionRow::builder().title(label).subtitle(value).subtitle_selectable(true).css_classes(["property"]).build();
            group.add(&row);
        }
        content.append(&group);
    }
    let center = button("find-location-symbolic", &gettext("Show on the chart"));
    {
        let app = app.clone();
        center.connect_clicked(move |_| isomar_app::vm::traffic::show_vessel(&app, mmsi));
    }
    content.append(&center);
    true
}

/// The detail of vessel `mmsi`, kept up to date every two seconds while it is shown.
pub fn vessel(app: &Rc<AppState>, mmsi: u32) -> Option<adw::NavigationPage> {
    let title = call(app, "vessel_get", json!({ "id": mmsi }));
    if title.is_null() {
        return None;
    }
    let content = super::column();
    fill_vessel(app, mmsi, &content);
    let p = page(&str_of(&title, "title"), &content);
    let (app, weak) = (Rc::downgrade(app), p.downgrade());
    gtk::glib::timeout_add_local(std::time::Duration::from_secs(2), move || {
        let (Some(app), Some(p)) = (app.upgrade(), weak.upgrade()) else { return gtk::glib::ControlFlow::Break };
        if p.parent().is_none() || !fill_vessel(&app, mmsi, &content) {
            return gtk::glib::ControlFlow::Break;
        }
        gtk::glib::ControlFlow::Continue
    });
    Some(p)
}
