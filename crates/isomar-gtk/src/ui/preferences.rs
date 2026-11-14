//! Preferences (`design/screens/dialogs-gnome-preferences.png`): General, Units, Chart, Alarms,
//! Boat and Data, each a preferences page under an inline switcher, in a window of its own beside
//! the chart. Every change applies at once, through the shared `settings_patch`.

use super::app::AppState;
use crate::i18n::gettext;
use adw::prelude::*;
use serde_json::{Value, json};
use std::rc::Rc;

fn patch(app: &Rc<AppState>, v: Value) {
    let a = isomar_app::api::call(app, "settings_patch", &v);
    if let Some(e) = a.get("error").and_then(|e| e.as_str()) {
        super::show_error(&gettext("Could not save the preferences"), e);
    }
}

/// A switch row for the boolean setting `key`.
fn switch(app: &Rc<AppState>, title: &str, subtitle: &str, key: &'static str, on: bool) -> adw::SwitchRow {
    let r = adw::SwitchRow::builder().title(title).active(on).build();
    if !subtitle.is_empty() {
        r.set_subtitle(subtitle);
    }
    let app = Rc::downgrade(app);
    r.connect_active_notify(move |r| {
        if let Some(app) = app.upgrade() {
            patch(&app, json!({ key: r.is_active() }));
        }
    });
    r
}

/// A number with its unit as a suffix, for the setting `key`.
fn number(app: &Rc<AppState>, title: &str, unit: &str, key: &'static str, (min, max, step): (f64, f64, f64), digits: u32, value: f64) -> adw::SpinRow {
    let r = adw::SpinRow::with_range(min, max, step);
    r.set_title(title);
    r.set_digits(digits);
    r.set_value(value.clamp(min, max));
    if !unit.is_empty() {
        r.add_suffix(&gtk::Label::builder().label(unit).css_classes(["dim-label"]).build());
    }
    let app = Rc::downgrade(app);
    r.connect_value_notify(move |r| {
        if let Some(app) = app.upgrade() {
            // Integer settings stay integers in the JSON.
            let v = if digits == 0 { json!(r.value().round() as i64) } else { json!(r.value()) };
            patch(&app, json!({ key: v }));
        }
    });
    r
}

/// A choice among `options` (value, label) for the string setting `key`.
fn choice(app: &Rc<AppState>, title: &str, key: &'static str, options: &[(&'static str, String)], current: &str) -> adw::ComboRow {
    let labels: Vec<&str> = options.iter().map(|(_, l)| l.as_str()).collect();
    let r = adw::ComboRow::builder().title(title).model(&gtk::StringList::new(&labels)).build();
    r.set_selected(options.iter().position(|(v, _)| *v == current).unwrap_or(0) as u32);
    let values: Vec<&'static str> = options.iter().map(|(v, _)| *v).collect();
    let app = Rc::downgrade(app);
    r.connect_selected_notify(move |r| {
        if let (Some(app), Some(v)) = (app.upgrade(), values.get(r.selected() as usize)) {
            patch(&app, json!({ key: v }));
        }
    });
    r
}

fn group(page: &adw::PreferencesPage, title: &str, description: &str) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::builder().title(title).build();
    if !description.is_empty() {
        g.set_description(Some(description));
    }
    page.add(&g);
    g
}

pub fn open(app: &Rc<AppState>) {
    let s = app.settings.borrow().clone();
    let stack = adw::ViewStack::new();
    let add_page = |name: &str, title: &str, icon: &str| {
        let p = adw::PreferencesPage::new();
        stack.add_titled_with_icon(&p, Some(name), title, icon);
        p
    };

    // General.
    let general = add_page("general", &gettext("General"), "preferences-system-symbolic");
    let g = group(&general, &gettext("Appearance"), "");
    let palette = adw::ComboRow::builder().title(gettext("Palette")).subtitle(gettext("Night is dim blue-grey and keeps your night vision")).model(&gtk::StringList::new(&[&gettext("Follow system"), &gettext("Day"), &gettext("Dusk"), &gettext("Night")])).build();
    let modes = ["auto", "day", "dusk", "night"];
    palette.set_selected(modes.iter().position(|m| *m == app.canvas.mode().as_str()).unwrap_or(0) as u32);
    palette.connect_selected_notify(move |r| {
        let _ = super::chart_widget().activate_action("win.mode", Some(&modes[r.selected() as usize].to_variant()));
    });
    g.add(&palette);
    g.add(&number(app, &gettext("Text size on the chart"), "%", "text_scale", (50.0, 300.0, 10.0), 0, f64::from(s.text_scale)));
    let g = group(&general, &gettext("Under way"), "");
    g.add(&switch(app, &gettext("Switch to the under-way layout when a route starts"), &gettext("And back when navigation stops"), "underway_auto", s.underway_auto));
    g.add(&switch(app, &gettext("Show the dashboard on the chart while planning"), "", "dashboard_on_chart", s.dashboard_on_chart));

    // Units.
    let units = add_page("units", &gettext("Units"), "isomar-measure-symbolic");
    let g = group(&units, &gettext("Units"), "");
    g.add(&choice(app, &gettext("Depth"), "depth_unit", &[("m", gettext("Metres")), ("ft", gettext("Feet")), ("fa", gettext("Fathoms"))], &s.depth_unit));
    g.add(&choice(app, &gettext("Distance"), "distance_unit", &[("nm", gettext("Nautical miles")), ("km", gettext("Kilometres")), ("mi", gettext("Miles"))], &s.distance_unit));
    g.add(&choice(app, &gettext("Speed"), "speed_unit", &[("kn", gettext("Knots")), ("km/h", gettext("km/h")), ("m/s", gettext("m/s"))], &s.speed_unit));
    let bearings = adw::ComboRow::builder().title(gettext("Bearings")).model(&gtk::StringList::new(&[&gettext("True"), &gettext("Magnetic")])).selected(u32::from(s.bearing_magnetic)).build();
    {
        let app = Rc::downgrade(app);
        bearings.connect_selected_notify(move |r| {
            if let Some(app) = app.upgrade() {
                patch(&app, json!({ "bearing_magnetic": r.selected() == 1 }));
            }
        });
    }
    g.add(&bearings);

    // Chart.
    let chart = add_page("chart", &gettext("Chart"), "isomar-charts-symbolic");
    let g = group(&chart, &gettext("Chart"), "");
    g.add(&choice(app, &gettext("Chart style"), "chart_style", &[("paper", gettext("Paper")), ("ecdis", gettext("ECDIS"))], &s.chart_style));
    let scale_filter = adw::ComboRow::builder()
        .title(gettext("Maximum detail (S-57)"))
        .subtitle(gettext("Limits S-57 charts to this scale band and coarser"))
        .model(&gtk::StringList::new(&[
            &gettext("Auto (All scales)"),
            &gettext("Up to Coastal (Band 3)"),
            &gettext("Up to Approach (Band 4)"),
            &gettext("Up to Harbour (Band 5)"),
            &gettext("All details (Band 6)"),
        ]))
        .build();
    let current_idx = match s.enc_max_band {
        Some(3) => 1,
        Some(4) => 2,
        Some(5) => 3,
        Some(6) => 4,
        _ => 0,
    };
    scale_filter.set_selected(current_idx);
    {
        let app = Rc::downgrade(app);
        scale_filter.connect_selected_notify(move |r| {
            if let Some(app) = app.upgrade() {
                let band = match r.selected() {
                    1 => Some(3),
                    2 => Some(4),
                    3 => Some(5),
                    4 => Some(6),
                    _ => None,
                };
                let v = match band {
                    Some(b) => serde_json::json!(b),
                    None => serde_json::Value::Null,
                };
                patch(&app, json!({ "enc_max_band": v }));
            }
        });
    }
    g.add(&scale_filter);
    g.add(&switch(app, &gettext("Light sectors at their full range"), &gettext("Otherwise drawn at a fixed size"), "full_sectors", s.full_sectors));
    g.add(&number(app, &gettext("Safety contour"), &gettext("m"), "safety_depth", (1.0, 50.0, 1.0), 0, f64::from(s.safety_depth)));
    g.add(&number(app, &gettext("Course vector"), &gettext("minutes"), "vector_minutes", (1.0, 30.0, 1.0), 0, s.vector_minutes));

    // Alarms.
    let alarms = add_page("alarms", &gettext("Alarms"), "isomar-alarm-symbolic");
    let g = group(&alarms, &gettext("Collision risk"), &gettext("An alarm when another vessel will pass closer than this, within this time."));
    g.add(&number(app, &gettext("Closest approach under"), &gettext("nm"), "ais_cpa_nm", (0.05, 5.0, 0.05), 2, s.ais_cpa_nm));
    g.add(&number(app, &gettext("Within"), &gettext("minutes"), "ais_tcpa_min", (1.0, 120.0, 1.0), 0, s.ais_tcpa_min));
    let g = group(&alarms, &gettext("At anchor and under way"), "");
    g.add(&number(app, &gettext("Anchor circle"), &gettext("nm"), "anchor_radius_nm", (0.01, 1.0, 0.01), 2, s.anchor_radius_nm));
    g.add(&number(app, &gettext("Off course beyond"), &gettext("nm"), "xte_limit_nm", (0.02, 2.0, 0.02), 2, s.xte_limit_nm));
    let shallow = number(app, &gettext("Shallow water under"), &gettext("m"), "shallow_alarm_m", (0.0, 30.0, 0.5), 1, s.shallow_alarm_m);
    shallow.set_subtitle(&gettext("0 turns it off"));
    g.add(&shallow);
    let wind = number(app, &gettext("Wind warning above"), &gettext("kn"), "wind_limit_kn", (0.0, 60.0, 1.0), 0, s.wind_limit_kn);
    wind.set_subtitle(&gettext("0 turns it off"));
    g.add(&wind);

    // Boat.
    let boat = add_page("boat", &gettext("Boat"), "isomar-polar-symbolic");
    let g = group(&boat, &gettext("Size"), &gettext("Draws the boat to scale when zoomed in."));
    g.add(&number(app, &gettext("Length"), &gettext("m"), "boat_length_m", (3.0, 60.0, 0.5), 1, s.boat_length_m));
    g.add(&number(app, &gettext("Beam"), &gettext("m"), "boat_beam_m", (1.0, 15.0, 0.1), 1, s.boat_beam_m));
    let g = group(&boat, &gettext("Performance"), &gettext("The polar says how fast the boat sails at each wind angle and strength. Weather routing uses it."));
    let polar = adw::ButtonRow::builder().title(gettext("Edit the polar…")).start_icon_name("isomar-polar-symbolic").action_name("win.polar").build();
    g.add(&polar);
    g.add(&switch(app, &gettext("Record performance while under way"), &gettext("Learns what the boat really does: the instruments every 10 seconds, with the forecast's wind and waves."), "perf_recording", s.perf_recording));
    g.add(&adw::ButtonRow::builder().title(gettext("Performance and polar…")).start_icon_name("isomar-instruments-symbolic").action_name("win.performance").build());

    // Data.
    let data = add_page("data", &gettext("Data"), "drive-harddisk-symbolic");
    let g = group(&data, &gettext("Routing"), "");
    g.add(&number(app, &gettext("Auto route keeps clear by"), &gettext("nm"), "auto_clearance_nm", (0.05, 2.0, 0.05), 2, s.auto_clearance_nm));
    let tolerance = number(app, &gettext("Saved weather route stays within"), &gettext("nm"), "route_tolerance_nm", (0.01, 2.0, 0.01), 2, s.route_tolerance_nm);
    tolerance.set_subtitle(&gettext("Of the calculated path. More gives fewer waypoints; tacks and gybes are always kept, and legs never cross land the search avoided."));
    g.add(&tolerance);
    let g = group(&data, &gettext("Connections"), &gettext("Where the boat's data comes from: NMEA 0183 over the network or a serial port, gpsd, Signal K."));
    g.add(&adw::ButtonRow::builder().title(gettext("Connections…")).start_icon_name("network-wired-symbolic").action_name("win.connections").build());
    g.add(&adw::ButtonRow::builder().title(gettext("Data monitor…")).start_icon_name("utilities-terminal-symbolic").action_name("win.monitor").build());
    let g = group(&data, &gettext("Kept on this computer"), &gettext("Charts you add are read where they are. Tiles, depths and forecasts are kept so they work offline."));
    // What each download keeps, as values after its name once the folders are read.
    let downloads: Vec<(String, super::chips::ChipLine)> = isomar_app::vm::layers::groups()
        .into_iter()
        .flat_map(|g| g.layers)
        .filter(|(id, _)| [isomar_app::tiles::LAND_LAYER, isomar_app::dem::LAYER, isomar_app::emodnet::LAYER, isomar_app::places::LAYER].contains(&id.as_str()))
        .map(|(id, label)| {
            let r = adw::ActionRow::builder().title(label).build();
            let line = super::chips::ChipLine::single(&["dim-label"], 14);
            line.set(&[isomar_app::chips::Chip::plain("…")]);
            r.add_suffix(&line.widget);
            g.add(&r);
            (id, line)
        })
        .collect();
    super::spawn(async move {
        let rows = super::bg(isomar_app::vm::layers::downloads).await;
        for (id, line) in &downloads {
            if let Some(d) = rows.iter().find(|d| &d.id == id) {
                line.set(&d.detail);
            }
        }
    });
    let folder = adw::ButtonRow::builder().title(gettext("Open the data folder")).start_icon_name("folder-open-symbolic").build();
    folder.connect_activated(|_| {
        let uri = gtk::gio::File::for_path(isomar_app::core::datadir::user_data_dir()).uri();
        gtk::UriLauncher::new(&uri).launch(super::chart_widget().root().and_downcast_ref::<gtk::Window>(), gtk::gio::Cancellable::NONE, |_| {});
    });
    g.add(&folder);

    let switcher = adw::InlineViewSwitcher::builder().stack(&stack).display_mode(adw::InlineViewSwitcherDisplayMode::Labels).homogeneous(true).margin_start(12).margin_end(12).margin_bottom(6).build();
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&adw::HeaderBar::new());
    tv.add_top_bar(&switcher);
    tv.set_content(Some(&stack));
    let win = super::float::window(&gettext("Preferences"), 640, 680);
    win.set_content(Some(&tv));
    win.present();
}
