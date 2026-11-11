//! The sidebar pages, and the details they push.

pub mod charts;
pub mod details;
pub mod instruments;
pub mod passage;
pub mod performance;
pub mod routing;
pub mod traffic;
pub mod weather;

use adw::prelude::*;

/// A scrolled page with a padded vertical box.
pub fn scrolled(content: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    let clamp = adw::Clamp::builder().maximum_size(520).child(content).build();
    gtk::ScrolledWindow::builder().child(&clamp).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build()
}

/// A page's column: the groups, spaced as the boxed lists of a preferences page.
pub fn column() -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 18);
    b.set_margin_top(12);
    b.set_margin_bottom(18);
    b.set_margin_start(12);
    b.set_margin_end(12);
    b
}

/// An empty list's line, in place of rows.
pub fn placeholder(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).wrap(true).xalign(0.0).css_classes(["caption", "dim-label"]).margin_start(6).build()
}

/// A flat icon button for a row's end, with its tooltip as its name.
pub fn row_button(icon: &str, tip: &str) -> gtk::Button {
    let b = gtk::Button::builder().icon_name(icon).tooltip_text(tip).valign(gtk::Align::Center).css_classes(["flat"]).build();
    b.update_property(&[gtk::accessible::Property::Label(tip)]);
    b
}

/// A "⋯" menu for a row: entries that each call an action of the window with the row's id.
pub fn more_menu(items: &[(String, &str)], id: i64) -> gtk::MenuButton {
    let menu = gtk::gio::Menu::new();
    for (label, action) in items {
        let item = gtk::gio::MenuItem::new(Some(label), None);
        item.set_action_and_target_value(Some(action), Some(&id.to_variant()));
        menu.append_item(&item);
    }
    gtk::MenuButton::builder().icon_name("view-more-symbolic").menu_model(&menu).valign(gtk::Align::Center).css_classes(["flat"]).tooltip_text(crate::i18n::gettext("More")).build()
}

/// Removes every child of `b`.
pub fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

/// Removes every row of `list`.
pub fn clear_list(list: &gtk::ListBox) {
    list.remove_all();
}
