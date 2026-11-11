//! GTK user interface.

#![allow(clippy::type_complexity, clippy::too_many_arguments)]

pub mod alerts;
pub mod autoroute;
pub mod busy;
pub mod chips;
pub mod canvas;
pub mod datetime;
pub mod debug;
pub mod dialogs;
pub mod float;
pub mod form;
pub mod gpx;
pub mod header;
pub mod overlays;
pub mod pages;
pub mod polar;
pub mod popovers;
pub mod preferences;
pub mod routing_stats;
pub mod shell;
pub mod sidebar;
pub mod theme;
pub mod weather;
pub mod welcome;
pub use isomar_app::{actions, app, palette, view};
pub mod window;

use crate::i18n::gettext;
use adw::prelude::*;
use gtk::{gdk, gio, glib};

pub use isomar_app::platform::{bg, spawn};

thread_local! {
    static CHART: std::cell::RefCell<Option<gtk::Widget>> = const { std::cell::RefCell::new(None) };
}

/// Remembers the chart widget, the parent of dialogs and menus.
pub fn set_chart_widget(w: &gtk::Widget) {
    CHART.with(|c| *c.borrow_mut() = Some(w.clone()));
}

/// The chart widget; only after the main window exists.
pub fn chart_widget() -> gtk::Widget {
    CHART.with(|c| c.borrow().clone()).expect("the main window is not built yet")
}

/// The GTK main loop's side of `isomar_app::platform`; call once, on the main thread.
pub fn install_platform() {
    use isomar_app::platform::{self, Hooks};
    platform::install(Hooks {
        spawn_local: Box::new(|f| {
            glib::spawn_future_local(f);
        }),
        timeout_once: Box::new(|d, f| {
            glib::timeout_add_local_once(d, f);
        }),
        set_clipboard: Box::new(|text| {
            if let Some(d) = gdk::Display::default() {
                d.clipboard().set_text(text);
            }
        }),
    });
}

fn mono_text(text: &str) -> gtk::ScrolledWindow {
    let tv = gtk::TextView::builder()
        .editable(false)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::WordChar)
        .left_margin(8)
        .right_margin(8)
        .top_margin(8)
        .bottom_margin(8)
        .build();
    tv.buffer().set_text(text);
    let sw = gtk::ScrolledWindow::builder()
        .child(&tv)
        .min_content_height(80)
        .max_content_height(320)
        .propagate_natural_height(true)
        .build();
    sw.add_css_class("card");
    sw
}

/// Shows an error dialog with (monospace) details.
pub fn show_error(heading: &str, details: &str) {
    let d = float::Alert::new(heading, None);
    if !details.trim().is_empty() {
        d.extra(&mono_text(details.trim()));
    }
    let close = d.response("close", &gettext("Close"), adw::ResponseAppearance::Default);
    d.win.set_default_widget(Some(&close));
    d.close_response("close");
    d.present();
}

/// Asks for confirmation; returns true when the user accepts.
pub async fn confirm(heading: &str, body: &str, ok_label: &str, destructive: bool) -> bool {
    if std::env::var_os("ISOMAR_AUTO_ACCEPT").is_some() {
        return true;
    }
    let d = float::Alert::new(heading, Some(body));
    let cancel = d.response("cancel", &gettext("Cancel"), adw::ResponseAppearance::Default);
    d.response("ok", ok_label, if destructive { adw::ResponseAppearance::Destructive } else { adw::ResponseAppearance::Suggested });
    d.win.set_default_widget(Some(&cancel));
    d.close_response("cancel");
    glib::idle_add_local_once(move || {
        cancel.grab_focus();
    });
    d.choose().await == "ok"
}

/// Asks for a single line of text.
pub async fn ask_text(heading: &str, body: &str, initial: &str, ok_label: &str) -> Option<String> {
    let d = float::Alert::new(heading, (!body.is_empty()).then_some(body));
    let entry = gtk::Entry::builder().text(initial).activates_default(true).build();
    d.extra(&entry);
    d.response("cancel", &gettext("Cancel"), adw::ResponseAppearance::Default);
    let ok = d.response("ok", ok_label, adw::ResponseAppearance::Suggested);
    d.win.set_default_widget(Some(&ok));
    d.close_response("cancel");
    let e2 = entry.clone();
    glib::idle_add_local_once(move || {
        e2.grab_focus();
    });
    let r = d.choose().await;
    let t = entry.text().trim().to_string();
    (r == "ok" && !t.is_empty()).then_some(t)
}

/// Pops up a context menu at (x, y) relative to `widget`.
pub fn popup_menu(widget: &impl IsA<gtk::Widget>, x: f64, y: f64, menu: &gio::Menu) {
    let pop = gtk::PopoverMenu::from_model(Some(menu));
    pop.set_parent(widget);
    pop.set_has_arrow(false);
    pop.set_halign(gtk::Align::Start);
    pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    pop.connect_closed(|p| {
        let p = p.clone();
        // Unparent after the activated action has been dispatched.
        glib::idle_add_local_once(move || p.unparent());
    });
    pop.popup();
}

