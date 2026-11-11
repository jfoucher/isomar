//! The first-run welcome (`design/screens/flows-gnome-firstrun.png`): where to start (charts, the
//! boat's data, the forecast) and what Isomar is not. Shown once, until "Get started".

use super::app::AppState;
use crate::i18n::gettext;
use adw::prelude::*;
use serde_json::json;
use std::rc::Rc;

/// Opens the welcome when the settings say it has not been seen (never during a screenshot, unless
/// `ISOMAR_DIALOG=welcome` asks for it).
pub fn open_if_first_run(app: &Rc<AppState>) {
    let asked = std::env::var("ISOMAR_DIALOG").is_ok_and(|d| d == "welcome");
    if asked || (!app.settings.borrow().first_run_done && std::env::var_os("ISOMAR_SCREENSHOT").is_none()) {
        open(app);
    }
}

pub fn open(app: &Rc<AppState>) {
    let win = super::float::window(&gettext("Welcome"), 520, -1);
    let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(14).margin_top(6).margin_bottom(18).margin_start(18).margin_end(18).build();
    content.append(&gtk::Label::builder().label(gettext("Welcome to Isomar")).xalign(0.0).css_classes(["title-1"]).build());
    content.append(&gtk::Label::builder().label(gettext("A chart plotter for sailors. Start with what you have:")).xalign(0.0).wrap(true).css_classes(["dim-label"]).build());
    let steps = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::None).build();
    for (icon, title, subtitle, action, target) in [
        ("isomar-charts-symbolic", gettext("Charts"), gettext("Add a folder of charts you own, or install a free source for your area."), "win.page", Some("charts")),
        ("network-wired-symbolic", gettext("A GPS or instruments"), gettext("Connect over the network, a serial port or a file."), "win.connections", None),
        ("weather-few-clouds-symbolic", gettext("The forecast"), gettext("Download the wind and waves for the area you sail."), "win.page", Some("weather")),
    ] {
        let row = adw::ActionRow::builder().title(title).subtitle(subtitle).activatable(true).action_name(action).build();
        if let Some(t) = target {
            row.set_action_target_value(Some(&t.to_variant()));
        }
        row.add_prefix(&gtk::Image::from_icon_name(icon));
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        steps.append(&row);
    }
    content.append(&steps);
    let note = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let warn = gtk::Image::from_icon_name("dialog-warning-symbolic");
    warn.add_css_class("warning");
    warn.set_valign(gtk::Align::Start);
    note.append(&warn);
    note.append(&gtk::Label::builder().label(gettext("Isomar helps you plan and follow a passage. It does not replace official charts, a lookout or your judgement. The open-data map is not for navigation.")).xalign(0.0).wrap(true).build());
    content.append(&note);
    let start = gtk::Button::builder().label(gettext("Get started")).css_classes(["suggested-action", "pill"]).halign(gtk::Align::End).build();
    content.append(&start);
    {
        let (app, win) = (Rc::downgrade(app), win.clone());
        start.connect_clicked(move |_| {
            if let Some(app) = app.upgrade() {
                isomar_app::api::call(&app, "settings_patch", &json!({ "first_run_done": true }));
            }
            win.close();
        });
    }
    // Following a step leaves the welcome for next time.
    {
        let win = win.clone();
        steps.connect_row_activated(move |_, _| win.close());
    }
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&adw::HeaderBar::builder().show_title(false).build());
    tv.set_content(Some(&content));
    win.set_content(Some(&tv));
    win.set_default_widget(Some(&start));
    win.present();
}
