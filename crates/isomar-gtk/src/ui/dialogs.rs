//! Dialogs: connections, the data monitor and preferences.

use super::app::AppState;
use super::form::{Form, combo_value};
use super::spawn;
use crate::core::comm::source::{ConnectionConfig, Kind, Status, list_serial_ports};
use crate::i18n::{gettext, gettext_f};
use adw::prelude::*;
use chrono::{Local, Utc};
use gtk::glib;
use std::cell::RefCell;
use std::rc::Rc;

fn spin_row(title: &str, min: f64, max: f64, step: f64, digits: u32, value: f64) -> adw::SpinRow {
    let r = adw::SpinRow::with_range(min, max, step);
    r.set_digits(digits);
    r.set_title(title);
    r.set_value(value);
    r
}

// Connections

fn persist_connections(app: &AppState) {
    let cfgs = isomar_app::host::saved_connections(app);
    app.settings.borrow_mut().connections = cfgs;
    app.apply_settings();
}

fn kind_labels() -> Vec<String> {
    vec![
        gettext("NMEA 0183 over TCP"),
        gettext("NMEA 0183 over UDP"),
        gettext("Serial port"),
        gettext("gpsd"),
        gettext("Signal K"),
        gettext("Replay a log file"),
        gettext("Demo boat (simulator)"),
    ]
}

fn kind_index(k: &Kind) -> u32 {
    match k {
        Kind::Tcp { .. } => 0,
        Kind::Udp { .. } => 1,
        Kind::Serial { .. } => 2,
        Kind::Gpsd { .. } => 3,
        Kind::SignalK { .. } => 4,
        Kind::File { .. } => 5,
        Kind::Simulator { .. } => 6,
    }
}

async fn edit_connection(app: &Rc<AppState>, existing: Option<ConnectionConfig>, preset: Option<u32>) -> Option<ConnectionConfig> {
    let base = existing.clone().unwrap_or_else(|| ConnectionConfig::new("", "", Kind::Tcp { host: "192.168.1.1".into(), port: 10110 }));
    let idx = preset.unwrap_or_else(|| kind_index(&base.kind));
    let form = Form::new(&gettext("Connection"), &gettext("Save"));
    form.group("");
    let name = form.entry(&gettext("Name"), &base.name);
    let labels = kind_labels();
    let kind = form.combo(&gettext("Type"), &labels, Some(&labels[idx as usize]));
    let (host0, port0) = match &base.kind {
        Kind::Tcp { host, port } | Kind::Gpsd { host, port } => (host.clone(), *port),
        Kind::Udp { port } => ("".into(), *port),
        _ => ("localhost".into(), 10110),
    };
    let host = form.entry(&gettext("Host"), &host0);
    let port = spin_row(&gettext("Port"), 1.0, 65535.0, 1.0, 0, port0 as f64);
    form.add(&port);
    let ports = list_serial_ports();
    let serial_path0 = if let Kind::Serial { path, .. } = &base.kind { path.clone() } else { ports.first().cloned().unwrap_or_else(|| "/dev/ttyUSB0".into()) };
    let serial = form.entry(&gettext("Device"), &serial_path0);
    let bauds = ["4800", "9600", "19200", "38400", "57600", "115200"].map(String::from).to_vec();
    let baud0 = if let Kind::Serial { baud, .. } = &base.kind { baud.to_string() } else { "4800".into() };
    let baud = form.combo(&gettext("Baud rate"), &bauds, Some(&baud0));
    let url0 = if let Kind::SignalK { url } = &base.kind { url.clone() } else { "http://localhost:3000".into() };
    let url = form.entry(&gettext("Address"), &url0);
    let file0 = if let Kind::File { path, .. } = &base.kind { path.clone() } else { String::new() };
    let file = form.entry(&gettext("File"), &file0);
    form.group(&gettext("Filter"));
    let modes = [gettext("Pass everything"), gettext("Only these sentences"), gettext("Everything except these")];
    let mode_i = match base.filter_mode.as_str() {
        "allow" => 1,
        "deny" => 2,
        _ => 0,
    };
    let mode = form.combo(&gettext("Sentences"), &modes, Some(&modes[mode_i]));
    let items = form.entry(&gettext("List (e.g. RMC, GGA)"), &base.filter_items);

    // Show only the fields that make sense for the chosen type.
    let update = {
        let (kind, host, port, serial, baud, url, file, mode, items) =
            (kind.clone(), host.clone(), port.clone(), serial.clone(), baud.clone(), url.clone(), file.clone(), mode.clone(), items.clone());
        move || {
            let k = kind.selected();
            host.set_visible(matches!(k, 0 | 3));
            port.set_visible(matches!(k, 0 | 1 | 3));
            serial.set_visible(k == 2);
            baud.set_visible(k == 2);
            url.set_visible(k == 4);
            file.set_visible(k == 5);
            let filterable = matches!(k, 0 | 1 | 2 | 5);
            mode.set_visible(filterable);
            items.set_visible(filterable && mode.selected() != 0);
        }
    };
    update();
    {
        let u = update.clone();
        kind.connect_selected_notify(move |_| u());
        let u = update.clone();
        mode.connect_selected_notify(move |_| u());
    }
    {
        let (kind, host, serial, url, file) = (kind.clone(), host.clone(), serial.clone(), url.clone(), file.clone());
        form.validate(move || match kind.selected() {
            0 | 3 => !host.text().trim().is_empty(),
            2 => !serial.text().trim().is_empty(),
            4 => !url.text().trim().is_empty(),
            5 => !file.text().trim().is_empty(),
            _ => true,
        });
    }
    form.watch(&host);
    form.watch(&serial);
    form.watch(&url);
    form.watch(&file);
    form.watch_combo(&kind);
    if !form.run().await {
        return None;
    }
    let kind_v = match kind.selected() {
        0 => Kind::Tcp { host: host.text().trim().into(), port: port.value() as u16 },
        1 => Kind::Udp { port: port.value() as u16 },
        2 => Kind::Serial { path: serial.text().trim().into(), baud: combo_value(&baud).parse().unwrap_or(4800) },
        3 => Kind::Gpsd { host: host.text().trim().into(), port: port.value() as u16 },
        4 => Kind::SignalK { url: url.text().trim().into() },
        5 => Kind::File { path: file.text().trim().into(), rate_hz: 0.0 },
        _ => {
            let v = app.canvas.view_state();
            Kind::Simulator { lat: v.lat, lon: v.lon, cog: 90.0, sog: 6.0 }
        }
    };
    let mut c = existing.unwrap_or_else(|| ConnectionConfig::new(&format!("c{}", Utc::now().timestamp_millis()), "", kind_v.clone()));
    c.kind = kind_v;
    c.name = if name.text().trim().is_empty() { c.describe() } else { name.text().trim().to_string() };
    c.filter_mode = ["off", "allow", "deny"][mode.selected().min(2) as usize].to_string();
    c.filter_items = items.text().trim().to_string();
    Some(c)
}

fn status_text(app: &AppState, c: &ConnectionConfig) -> String {
    if !c.enabled {
        return gettext("Disabled");
    }
    match app.session.borrow().conn_status.get(&c.id) {
        Some(Status::Connected) => gettext("Connected"),
        Some(Status::Connecting) | None => gettext("Connecting…"),
        Some(Status::Disconnected(e)) => gettext_f("Not connected: {error}", &[("error", e)]),
        Some(Status::Finished) => gettext("Finished"),
    }
}

/// The icon of a connection's state.
fn status_icon(app: &AppState, c: &ConnectionConfig) -> &'static str {
    if !c.enabled {
        return "network-offline-symbolic";
    }
    match app.session.borrow().conn_status.get(&c.id) {
        Some(Status::Connected) => "network-transmit-receive-symbolic",
        Some(Status::Connecting) | None => "network-idle-symbolic",
        Some(Status::Disconnected(_)) => "network-error-symbolic",
        Some(Status::Finished) => "network-offline-symbolic",
    }
}

pub fn connections(app: &Rc<AppState>) {
    let dialog = super::float::window(&gettext("Connections"), 560, 520);
    let header = adw::HeaderBar::new();
    let add = gtk::MenuButton::builder().icon_name("list-add-symbolic").tooltip_text(gettext("Add a connection")).build();
    let menu = gtk::gio::Menu::new();
    for (i, l) in kind_labels().iter().enumerate() {
        let item = gtk::gio::MenuItem::new(Some(l), None);
        item.set_action_and_target_value(Some("conn.add"), Some(&(i as u32).to_variant()));
        menu.append_item(&item);
    }
    add.set_menu_model(Some(&menu));
    header.pack_start(&add);
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    group.set_description(Some(&gettext("Where the chart gets its position, depth, wind and AIS traffic from.")));
    page.add(&group);
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&header);
    tv.set_content(Some(&page));
    dialog.set_content(Some(&tv));

    let rows: Rc<RefCell<Vec<gtk::Widget>>> = Rc::new(RefCell::new(Vec::new()));
    let rebuild: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let f: Rc<dyn Fn()> = {
        let (app, group, rows, rebuild) = (app.clone(), group.clone(), rows.clone(), rebuild.clone());
        Rc::new(move || {
            for r in rows.borrow_mut().drain(..) {
                group.remove(&r);
            }
            let cfgs = app.hub.borrow().configs();
            if cfgs.is_empty() {
                let row = adw::ActionRow::builder().title(gettext("No connections")).subtitle(gettext("Add one with the + button, or try the demo boat.")).build();
                group.add(&row);
                rows.borrow_mut().push(row.upcast());
            }
            for c in cfgs {
                // The state as an icon before the name (its words in the tooltip), the source under it,
                // and why it is not connected, when it is not.
                let status = status_text(&app, &c);
                let icon = gtk::Image::from_icon_name(status_icon(&app, &c));
                icon.set_tooltip_text(Some(&status));
                icon.update_property(&[gtk::accessible::Property::Label(&status)]);
                let error = match app.session.borrow().conn_status.get(&c.id) {
                    Some(Status::Disconnected(_)) if c.enabled => status.clone(),
                    _ => String::new(),
                };
                let row = super::chips::row_full(Some(icon.upcast_ref()), &c.name, &[&[isomar_app::chips::Chip::plain(c.describe())]], &error, false);
                let sw = gtk::Switch::builder().active(c.enabled).valign(gtk::Align::Center).build();
                let edit = gtk::Button::builder().icon_name("document-edit-symbolic").valign(gtk::Align::Center).tooltip_text(gettext("Edit")).build();
                let del = gtk::Button::builder().icon_name("user-trash-symbolic").valign(gtk::Align::Center).tooltip_text(gettext("Remove")).build();
                edit.add_css_class("flat");
                del.add_css_class("flat");
                row.add_suffix(&sw);
                row.add_suffix(&edit);
                row.add_suffix(&del);
                {
                    let (app, c, rebuild) = (app.clone(), c.clone(), rebuild.clone());
                    sw.connect_active_notify(move |s| {
                        let mut c = c.clone();
                        c.enabled = s.is_active();
                        app.hub.borrow_mut().add(c);
                        persist_connections(&app);
                        if let Some(r) = rebuild.borrow().as_ref() {
                            r();
                        }
                    });
                }
                {
                    let (app, c, rebuild) = (app.clone(), c.clone(), rebuild.clone());
                    edit.connect_clicked(move |_| {
                        let (app, c, rebuild) = (app.clone(), c.clone(), rebuild.clone());
                        spawn(async move {
                            if let Some(new) = edit_connection(&app, Some(c), None).await {
                                app.hub.borrow_mut().add(new);
                                persist_connections(&app);
                                if let Some(r) = rebuild.borrow().as_ref() {
                                    r();
                                }
                            }
                        });
                    });
                }
                {
                    let (app, id, rebuild) = (app.clone(), c.id.clone(), rebuild.clone());
                    del.connect_clicked(move |_| {
                        app.hub.borrow_mut().remove(&id);
                        persist_connections(&app);
                        if let Some(r) = rebuild.borrow().as_ref() {
                            r();
                        }
                    });
                }
                group.add(&row);
                rows.borrow_mut().push(row.upcast());
            }
        })
    };
    *rebuild.borrow_mut() = Some(f.clone());
    f();

    // "Add" menu entries.
    let actions = gtk::gio::SimpleActionGroup::new();
    let add_action = gtk::gio::SimpleAction::new("add", Some(glib::VariantTy::UINT32));
    {
        let (app, rebuild) = (app.clone(), rebuild.clone());
        add_action.connect_activate(move |_, p| {
            let Some(i) = p.and_then(|p| p.get::<u32>()) else { return };
            let (app, rebuild) = (app.clone(), rebuild.clone());
            spawn(async move {
                if let Some(c) = edit_connection(&app, None, Some(i)).await {
                    app.hub.borrow_mut().add(c);
                    persist_connections(&app);
                    if let Some(r) = rebuild.borrow().as_ref() {
                        r();
                    }
                }
            });
        });
    }
    actions.add_action(&add_action);
    dialog.insert_action_group("conn", Some(&actions));

    // Keep the status text current while the dialog is open.
    {
        let weak = dialog.downgrade();
        glib::timeout_add_seconds_local(1, move || {
            let Some(d) = weak.upgrade() else { return glib::ControlFlow::Break };
            if d.is_mapped()
                && let Some(r) = rebuild.borrow().as_ref() {
                    r();
                }
            glib::ControlFlow::Continue
        });
    }
    dialog.present();
}

// Data monitor

pub fn monitor(app: &Rc<AppState>) {
    let dialog = super::float::window(&gettext("Data monitor"), 760, 520);
    let header = adw::HeaderBar::new();
    let pause = gtk::ToggleButton::builder().icon_name("media-playback-pause-symbolic").tooltip_text(gettext("Pause")).build();
    let clear = gtk::Button::builder().icon_name("edit-clear-all-symbolic").tooltip_text(gettext("Clear")).build();
    header.pack_start(&pause);
    header.pack_start(&clear);
    let filter = gtk::SearchEntry::builder().placeholder_text(gettext("Filter (for example RMC or AIVDM)")).hexpand(true).build();
    let tv = gtk::TextView::builder().editable(false).monospace(true).cursor_visible(false).left_margin(8).right_margin(8).top_margin(6).build();
    let bad = gtk::TextTag::builder().name("bad").foreground("#c01c28").build();
    tv.buffer().tag_table().add(&bad);
    let sw = gtk::ScrolledWindow::builder().child(&tv).vexpand(true).build();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
    body.set_margin_start(6);
    body.set_margin_end(6);
    body.append(&filter);
    body.append(&sw);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&body));
    dialog.set_content(Some(&toolbar));

    let cleared_at = Rc::new(RefCell::new(0usize));
    let render = {
        let (app, tv, filter, pause, cleared_at, sw) = (app.clone(), tv.clone(), filter.clone(), pause.clone(), cleared_at.clone(), sw.clone());
        move || {
            if pause.is_active() {
                return;
            }
            let needle = filter.text().to_lowercase();
            let s = app.session.borrow();
            let buf = tv.buffer();
            buf.set_text("");
            let skip = *cleared_at.borrow();
            let mut end = buf.end_iter();
            for l in s.monitor.iter().skip(skip.min(s.monitor.len())) {
                if !needle.is_empty() && !l.text.to_lowercase().contains(&needle) {
                    continue;
                }
                let line = format!("{}  {:<8} {}\n", l.time.with_timezone(&Local).format("%H:%M:%S"), l.source, l.text);
                if l.ok {
                    buf.insert(&mut end, &line);
                } else {
                    buf.insert_with_tags_by_name(&mut end, &line, &["bad"]);
                }
            }
            let adj = sw.vadjustment();
            adj.set_value(adj.upper());
        }
    };
    {
        let (app, cleared_at, render) = (app.clone(), cleared_at.clone(), render.clone());
        clear.connect_clicked(move |_| {
            *cleared_at.borrow_mut() = app.session.borrow().monitor.len();
            render();
        });
    }
    {
        let render = render.clone();
        filter.connect_search_changed(move |_| render());
    }
    render();
    {
        let weak = dialog.downgrade();
        glib::timeout_add_local(std::time::Duration::from_millis(700), move || {
            let Some(d) = weak.upgrade() else { return glib::ControlFlow::Break };
            if d.is_mapped() {
                render();
            }
            glib::ControlFlow::Continue
        });
    }
    dialog.present();
}

