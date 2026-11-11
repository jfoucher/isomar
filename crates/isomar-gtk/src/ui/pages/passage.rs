//! The Passage page: what the boat is following, the routes, marks and tracks. A route or a mark
//! opens its detail inside the page; the rest is on each row's menu. Built from `vm::passage`.

use super::{clear_list, more_menu, placeholder, row_button, scrolled};
use crate::i18n::gettext;
use crate::ui::chips;
use crate::ui::app::AppState;
use adw::prelude::*;
use isomar_app::vm::passage::Passage;
use std::cell::RefCell;
use std::rc::Rc;

/// A titled list: the heading with an optional button at its end, then a boxed list.
pub fn section(title: &str, suffix: Option<&gtk::Widget>) -> (gtk::Box, gtk::ListBox) {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    head.append(&gtk::Label::builder().label(title).xalign(0.0).hexpand(true).css_classes(["heading"]).margin_start(6).accessible_role(gtk::AccessibleRole::Heading).build());
    if let Some(s) = suffix {
        head.append(s);
    }
    b.append(&head);
    let list = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::None).build();
    b.append(&list);
    (b, list)
}

fn icon(name: &str) -> gtk::Image {
    gtk::Image::from_icon_name(name)
}

pub fn build(app: &Rc<AppState>, _nav: &adw::NavigationView) -> gtk::Widget {
    let page = super::column();

    let (following_box, following) = section(&gettext("Following"), None);
    page.append(&following_box);

    let new = gtk::Button::builder().child(&adw::ButtonContent::builder().icon_name("isomar-route-new-symbolic").label(gettext("New")).build()).action_name("win.new-route").css_classes(["flat"]).build();
    let (routes_box, routes) = section(&gettext("Routes"), Some(new.upcast_ref()));
    page.append(&routes_box);
    let (marks_box, marks) = section(&gettext("Marks"), None);
    page.append(&marks_box);
    let (tracks_box, tracks) = section(&gettext("Tracks"), None);
    page.append(&tracks_box);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let record = gtk::Button::builder().action_name("win.record-track").build();
    let record_content = adw::ButtonContent::builder().icon_name("isomar-track-symbolic").label(gettext("Record track")).build();
    record.set_child(Some(&record_content));
    let import = gtk::Button::builder().child(&adw::ButtonContent::builder().icon_name("document-open-symbolic").label(gettext("Import GPX…")).build()).action_name("win.import").build();
    buttons.append(&record);
    buttons.append(&import);
    page.append(&buttons);

    let shown: RefCell<Option<Passage>> = RefCell::new(None);
    let refresh = {
        let app = Rc::downgrade(app);
        move || {
            let Some(app) = app.upgrade() else { return };
            let p = isomar_app::vm::passage::passage(&app);
            if shown.borrow().as_ref() == Some(&p) {
                return;
            }

            following_box.set_visible(p.following.is_some());
            clear_list(&following);
            if let Some(f) = &p.following {
                let row = chips::row(Some("isomar-follow-on-symbolic"), &f.name, &f.detail);
                let stop = row_button("media-playback-stop-symbolic", &gettext("Stop navigating"));
                stop.set_action_name(Some("win.stop-navigation"));
                row.add_suffix(&stop);
                following.append(&row);
            }

            clear_list(&routes);
            if p.routes.is_empty() {
                routes.set_placeholder(Some(&placeholder(&gettext("No routes. Use New, then click on the chart."))));
            }
            for r in &p.routes {
                let lead = gtk::Image::from_icon_name("isomar-route-symbolic").upcast::<gtk::Widget>();
                let lines: Vec<&[_]> = [&r.detail[..], &r.planned[..]].into_iter().filter(|l| !l.is_empty()).collect();
                let row = chips::row_lines(Some(&lead), &r.name, &lines, false);
                row.set_activatable(true);
                if !r.visible {
                    row.add_css_class("dim-label");
                }
                let play = row_button("media-playback-start-symbolic", &gettext("Follow this route"));
                play.set_action_name(Some("win.activate-route"));
                play.set_action_target_value(Some(&r.id.to_variant()));
                row.add_suffix(&play);
                row.add_suffix(&more_menu(
                    &[
                        (gettext("Edit on the chart"), "win.edit-route"),
                        (gettext("Reverse"), "win.reverse-route"),
                        (if r.visible { gettext("Hide") } else { gettext("Show") }, "win.toggle-route"),
                        (gettext("Export GPX…"), "win.export-route"),
                        (gettext("Delete…"), "win.delete-route"),
                    ],
                    r.id,
                ));
                row.add_suffix(&icon("go-next-symbolic"));
                row.set_action_name(Some("win.route-detail"));
                row.set_action_target_value(Some(&r.id.to_variant()));
                if r.active {
                    row.add_css_class("active-route");
                }
                routes.append(&row);
            }

            clear_list(&marks);
            if p.waypoints.is_empty() {
                marks.set_placeholder(Some(&placeholder(&gettext("No marks. Right-click the chart and choose Drop waypoint here."))));
            }
            for w in &p.waypoints {
                let row = adw::ActionRow::builder().title(&w.name).subtitle(&w.detail).activatable(true).build();
                row.add_prefix(&icon("mark-location-symbolic"));
                row.add_suffix(&more_menu(
                    &[(gettext("Navigate to"), "win.goto-waypoint"), (gettext("Properties"), "win.waypoint-detail"), (gettext("Delete…"), "win.delete-waypoint")],
                    w.id,
                ));
                row.set_action_name(Some("win.zoom-waypoint"));
                row.set_action_target_value(Some(&w.id.to_variant()));
                marks.append(&row);
            }

            clear_list(&tracks);
            if p.tracks.is_empty() {
                tracks.set_placeholder(Some(&placeholder(&gettext("Recorded tracks appear here."))));
            }
            for t in &p.tracks {
                let row = chips::row(Some("isomar-track-symbolic"), &t.name, &t.detail);
                row.set_activatable(true);
                if !t.visible {
                    row.add_css_class("dim-label");
                }
                row.add_suffix(&more_menu(&[(if t.visible { gettext("Hide") } else { gettext("Show") }, "win.toggle-track"), (gettext("Delete…"), "win.delete-track")], t.id));
                row.set_action_name(Some("win.zoom-track"));
                row.set_action_target_value(Some(&t.id.to_variant()));
                tracks.append(&row);
            }

            record_content.set_label(&if p.recording { gettext("Stop recording") } else { gettext("Record track") });
            record_content.set_icon_name(if p.recording { "media-record-symbolic" } else { "isomar-track-symbolic" });
            *shown.borrow_mut() = Some(p);
        }
    };
    refresh();
    app.on_refresh(refresh);
    scrolled(&page).upcast()
}
