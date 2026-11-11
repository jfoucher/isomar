//! Buttons that start a job (`isomar_app::jobs`). While their job runs they look off, with a
//! spinner in place of their icon, and a press opens the activity indicator's list of running jobs
//! (`win.show-activity`) instead of starting the work again: the spinner is where the work was asked
//! for, and the detail and Cancel where they always are. Each such button is registered here with
//! the action its jobs name; the window's 4 Hz update calls `update`.

use super::app::AppState;
use adw::prelude::*;
use gtk::glib;
use isomar_app::vm::window::JobRow;
use std::cell::RefCell;

struct Tracked {
    widget: glib::WeakRef<gtk::Widget>,
    spinner: glib::WeakRef<adw::Spinner>,
    /// What the spinner stands in for (the button's icon), hidden while it spins.
    icon: Option<glib::WeakRef<gtk::Widget>>,
    action: String,
}

thread_local! {
    static TRACKED: RefCell<Vec<Tracked>> = const { RefCell::new(Vec::new()) };
}

fn show(t: &Tracked, busy: bool) {
    let (Some(w), Some(s)) = (t.widget.upgrade(), t.spinner.upgrade()) else { return };
    s.set_visible(busy);
    if let Some(i) = t.icon.as_ref().and_then(|i| i.upgrade()) {
        i.set_visible(!busy);
    }
    if busy {
        w.add_css_class("job-busy");
    } else {
        w.remove_css_class("job-busy");
    }
}

/// Registers `widget`, which starts `action`'s jobs: `spinner` shows while one runs, in place of
/// `icon` if there is one.
pub fn track(app: &AppState, widget: &impl IsA<gtk::Widget>, spinner: &adw::Spinner, icon: Option<&gtk::Widget>, action: &str) {
    let t = Tracked { widget: widget.upcast_ref::<gtk::Widget>().downgrade(), spinner: spinner.downgrade(), icon: icon.map(|i| i.downgrade()), action: action.to_string() };
    show(&t, app.jobs.busy(action));
    TRACKED.with(|v| {
        let mut v = v.borrow_mut();
        v.retain(|t| t.widget.upgrade().is_some());
        v.push(t);
    });
}

/// A button labelled `label` for `action`, with room for its spinner; the label is returned to be
/// changed.
pub fn button(app: &AppState, action: &str, label: &str) -> (gtk::Button, gtk::Label) {
    let spinner = adw::Spinner::builder().visible(false).build();
    let text = gtk::Label::new(Some(label));
    let content = gtk::Box::builder().spacing(8).halign(gtk::Align::Center).build();
    content.append(&spinner);
    content.append(&text);
    let b = gtk::Button::builder().child(&content).build();
    track(app, &b, &spinner, None, action);
    (b, text)
}

/// A button with an icon and a label for `action`: the spinner takes the icon's place.
pub fn icon_button(app: &AppState, action: &str, icon: &str, label: &str) -> gtk::Button {
    let spinner = adw::Spinner::builder().visible(false).build();
    let image = gtk::Image::from_icon_name(icon);
    let content = gtk::Box::builder().spacing(6).halign(gtk::Align::Center).build();
    content.append(&image);
    content.append(&spinner);
    content.append(&gtk::Label::builder().label(label).use_underline(true).build());
    let b = gtk::Button::builder().child(&content).build();
    track(app, &b, &spinner, Some(image.upcast_ref()), action);
    b
}

/// An icon-only button for `action`: the spinner takes the icon's place.
pub fn icon(app: &AppState, action: &str, icon: &str, tooltip: &str) -> gtk::Button {
    let spinner = adw::Spinner::builder().visible(false).build();
    let image = gtk::Image::from_icon_name(icon);
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    content.append(&image);
    content.append(&spinner);
    let b = gtk::Button::builder().child(&content).tooltip_text(tooltip).build();
    track(app, &b, &spinner, Some(image.upcast_ref()), action);
    b
}

/// An activatable row for `action`: the spinner goes at its end.
pub fn row(app: &AppState, row: &adw::ActionRow, action: &str) {
    let spinner = adw::Spinner::builder().visible(false).build();
    row.add_suffix(&spinner);
    track(app, row, &spinner, None, action);
}

/// What a press of `widget` does first: while `action`'s job runs, it shows the running jobs and
/// says so (the caller then does nothing else).
pub fn pressed(app: &AppState, widget: &impl IsA<gtk::Widget>, action: &str) -> bool {
    if !app.jobs.busy(action) {
        return false;
    }
    let _ = widget.activate_action("win.show-activity", None);
    true
}

/// Shows each registered button busy or not, from the jobs running.
pub fn update(jobs: &[JobRow]) {
    TRACKED.with(|v| {
        let mut v = v.borrow_mut();
        v.retain(|t| t.widget.upgrade().is_some());
        for t in v.iter() {
            show(t, jobs.iter().any(|j| !j.action.is_empty() && j.action == t.action));
        }
    });
}
