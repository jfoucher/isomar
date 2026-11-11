//! The header bar (`design/ia.md`): the sidebar switch, the title with the connection light, New
//! route, Weather, Layers, the activity indicator and the primary menu. Under way it shows Stop
//! navigation and Layers.

use super::app::AppState;
use crate::i18n::gettext;
use adw::prelude::*;
use isomar_app::vm::window::{Link, Window};
use gtk::{gio, glib};
use std::cell::RefCell;
use std::rc::Rc;

pub struct Header {
    pub bar: adw::HeaderBar,
    pub sidebar: gtk::ToggleButton,
    dot: gtk::Box,
    subtitle: super::chips::ChipLine,
    new_route: gtk::Button,
    weather: gtk::Button,
    stop: gtk::Button,
    activity: gtk::MenuButton,
    spinner: adw::Spinner,
    count: gtk::Label,
    jobs: gtk::Box,
    /// One row per running job, kept while it runs and changed in place.
    rows: RefCell<Vec<JobRow>>,
    offline_note: gtk::Label,
    shown: RefCell<Option<Window>>,
    compact: std::cell::Cell<bool>,
}

impl Header {
    pub fn new(app: &Rc<AppState>) -> Rc<Header> {
        let bar = adw::HeaderBar::new();
        let sidebar = gtk::ToggleButton::builder().icon_name("sidebar-show-symbolic").tooltip_text(gettext("Show sidebar")).build();
        sidebar.update_property(&[gtk::accessible::Property::Label(&gettext("Sidebar"))]);
        bar.pack_start(&sidebar);

        // The title, with the connection's light and words; it opens Connections.
        let title_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        title_box.append(&gtk::Label::builder().label(gettext("Isomar")).css_classes(["title"]).build());
        let sub = gtk::Box::builder().spacing(5).halign(gtk::Align::Center).build();
        let dot = gtk::Box::builder().css_classes(["status-dot"]).valign(gtk::Align::Center).build();
        let subtitle = super::chips::ChipLine::single(&["subtitle", "numeric"], 12);
        sub.append(&dot);
        sub.append(&subtitle.widget);
        title_box.append(&sub);
        let title = gtk::Button::builder().child(&title_box).css_classes(["flat", "title-button"]).action_name("win.connections").tooltip_text(gettext("Connections")).build();
        bar.set_title_widget(Some(&title));

        let menu = gtk::MenuButton::builder().icon_name("open-menu-symbolic").tooltip_text(gettext("Main menu")).primary(true).build();
        let pop = gtk::PopoverMenu::from_model(Some(&primary_menu()));
        pop.add_child(&palette_row(), "palette");
        menu.set_popover(Some(&pop));
        bar.pack_end(&menu);

        let (activity, spinner, count, jobs, offline_note) = activity_button(app);
        bar.pack_end(&activity);

        let layers = gtk::MenuButton::builder().icon_name("isomar-layers-symbolic").tooltip_text(gettext("Layers")).build();
        {
            let app = app.clone();
            layers.set_create_popup_func(move |m| m.set_popover(Some(&layers_popover(&app))));
        }
        bar.pack_end(&layers);

        let weather = gtk::Button::builder().icon_name("weather-few-clouds-symbolic").tooltip_text(gettext("Weather")).action_name("win.page").action_target(&"weather".to_variant()).build();
        bar.pack_end(&weather);
        let new_route = gtk::Button::builder().icon_name("isomar-route-new-symbolic").tooltip_text(gettext("New route (Ctrl+N)")).action_name("win.new-route").build();
        bar.pack_end(&new_route);
        let stop = gtk::Button::builder()
            .child(&adw::ButtonContent::builder().icon_name("media-playback-stop-symbolic").label(gettext("Stop navigation")).build())
            .css_classes(["destructive-action"])
            .action_name("win.stop-navigation")
            .visible(false)
            .build();
        bar.pack_end(&stop);
        for b in [&weather, &new_route] {
            if let Some(t) = b.tooltip_text() {
                b.update_property(&[gtk::accessible::Property::Label(&t)]);
            }
        }
        Rc::new(Header { bar, sidebar, dot, subtitle, new_route, weather, stop, activity, spinner, count, jobs, rows: RefCell::new(Vec::new()), offline_note, shown: RefCell::new(None), compact: std::cell::Cell::new(false) })
    }

    /// At phone width the header keeps the sidebar switch, the title, Layers and the menu.
    pub fn set_compact(&self, on: bool) {
        self.compact.set(on);
        self.shown.borrow_mut().take();
        let underway = self.stop.is_visible();
        self.new_route.set_visible(!underway && !on);
        self.weather.set_visible(!underway && !on);
    }

    /// Opens the list of running jobs: what a busy job button does when pressed (`busy`).
    pub fn show_activity(&self) {
        self.activity.set_visible(true);
        self.activity.popup();
    }

    pub fn update(&self, w: &Window) {
        if self.shown.borrow().as_ref() == Some(w) {
            return;
        }
        self.subtitle.set(&w.subtitle);
        for c in ["ok", "stale"] {
            self.dot.remove_css_class(c);
        }
        match w.link {
            Link::Ok => self.dot.add_css_class("ok"),
            Link::Stale => self.dot.add_css_class("stale"),
            Link::None => {}
        }
        self.new_route.set_visible(!w.underway && !self.compact.get());
        self.weather.set_visible(!w.underway && !self.compact.get());
        self.stop.set_visible(w.underway);
        self.sidebar.set_visible(!w.underway);

        // Activity: a spinner and a count while jobs run; "Offline" when downloads are off.
        let busy = !w.jobs.is_empty();
        self.activity.set_visible(busy || w.offline);
        self.spinner.set_visible(busy);
        self.count.set_visible(w.jobs.len() > 1 || (!busy && w.offline));
        self.count.set_text(&if busy { w.jobs.len().to_string() } else { gettext("Offline") });
        self.activity.set_tooltip_text(Some(&if busy { gettext("Running tasks") } else { gettext("Offline: nothing is downloaded") }));
        self.offline_note.set_visible(!busy && w.offline);
        // Rows are changed in place, never built again while their job runs: progress moves a few
        // times a second, and a Cancel button replaced between press and release lost the click.
        let mut rows = self.rows.borrow_mut();
        rows.retain(|r| {
            let running = w.jobs.iter().any(|j| j.id == r.id);
            if !running {
                self.jobs.remove(&r.widget);
            }
            running
        });
        for j in &w.jobs {
            let i = match rows.iter().position(|r| r.id == j.id) {
                Some(i) => i,
                None => {
                    let row = JobRow::new(j, &self.activity);
                    self.jobs.append(&row.widget);
                    rows.push(row);
                    rows.len() - 1
                }
            };
            rows[i].show(j);
        }
        if !busy && !w.offline {
            self.activity.popdown();
        }
        drop(rows);
        *self.shown.borrow_mut() = Some(w.clone());
    }
}

/// A running job in the activity popover: its title, a Cancel button if it has one, its progress.
struct JobRow {
    id: u64,
    widget: gtk::Box,
    title: gtk::Label,
    bar: gtk::ProgressBar,
    detail: gtk::Label,
}

impl JobRow {
    fn new(j: &isomar_app::vm::window::JobRow, activity: &gtk::MenuButton) -> JobRow {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let title = gtk::Label::builder().xalign(0.0).hexpand(true).css_classes(["heading"]).wrap(true).build();
        head.append(&title);
        if j.cancellable {
            let cancel = gtk::Button::builder().label(gettext("Cancel")).action_name("win.cancel-job").action_target(&j.id.to_variant()).valign(gtk::Align::Center).build();
            // The job is gone the moment Cancel is pressed: its row too, and the popover with the last.
            let (row, activity) = (widget.downgrade(), activity.downgrade());
            cancel.connect_clicked(move |_| {
                let Some(row) = row.upgrade() else { return };
                row.set_visible(false);
                let mut other = row.parent().and_then(|p| p.first_child());
                let mut any = false;
                while let Some(c) = other {
                    any |= c.is_visible();
                    other = c.next_sibling();
                }
                if !any && let Some(a) = activity.upgrade() {
                    a.popdown();
                }
            });
            head.append(&cancel);
        }
        widget.append(&head);
        let bar = gtk::ProgressBar::new();
        widget.append(&bar);
        let detail = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["caption", "dim-label"]).build();
        widget.append(&detail);
        JobRow { id: j.id, widget, title, bar, detail }
    }

    fn show(&self, j: &isomar_app::vm::window::JobRow) {
        self.title.set_label(&j.title);
        match j.fraction {
            Some(f) => self.bar.set_fraction(f),
            None => self.bar.pulse(),
        }
        self.detail.set_label(&j.detail);
        self.detail.set_visible(!j.detail.is_empty());
    }
}

fn activity_button(_app: &Rc<AppState>) -> (gtk::MenuButton, adw::Spinner, gtk::Label, gtk::Box, gtk::Label) {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let spinner = adw::Spinner::new();
    let count = gtk::Label::builder().css_classes(["caption", "numeric"]).build();
    content.append(&spinner);
    content.append(&count);
    let jobs = gtk::Box::new(gtk::Orientation::Vertical, 12);
    jobs.set_margin_top(6);
    jobs.set_margin_bottom(6);
    jobs.set_margin_start(6);
    jobs.set_margin_end(6);
    jobs.set_width_request(280);
    let offline_note = gtk::Label::builder().label(gettext("Offline: downloads are off. Charts, tiles and forecasts already saved still show.")).wrap(true).max_width_chars(32).xalign(0.0).visible(false).build();
    jobs.append(&offline_note);
    let pop = gtk::Popover::builder().child(&jobs).build();
    let button = gtk::MenuButton::builder().child(&content).popover(&pop).visible(false).build();
    button.update_property(&[gtk::accessible::Property::Label(&gettext("Running tasks"))]);
    (button, spinner, count, jobs, offline_note)
}

fn primary_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let palette = gio::Menu::new();
    let item = gio::MenuItem::new(None, None);
    item.set_attribute_value("custom", Some(&"palette".to_variant()));
    palette.append_item(&item);
    menu.append_section(None, &palette);

    let view = gio::Menu::new();
    view.append(Some(&gettext("Under way layout")), Some("win.underway"));
    menu.append_section(None, &view);

    let files = gio::Menu::new();
    files.append(Some(&gettext("Import GPX…")), Some("win.import"));
    files.append(Some(&gettext("Export GPX…")), Some("win.export"));
    menu.append_section(None, &files);

    let boat = gio::Menu::new();
    boat.append(Some(&gettext("Connections")), Some("win.connections"));
    boat.append(Some(&gettext("Data monitor")), Some("win.monitor"));
    menu.append_section(None, &boat);

    let app = gio::Menu::new();
    app.append(Some(&gettext("Preferences")), Some("win.preferences"));
    app.append(Some(&gettext("Keyboard Shortcuts")), Some("win.show-help-overlay"));
    app.append(Some(&gettext("About Isomar")), Some("win.about"));
    menu.append_section(None, &app);
    menu
}

/// The palette switch at the top of the primary menu: Follow system, Day, Dusk, Night.
fn palette_row() -> gtk::Box {
    let b = gtk::Box::builder().css_classes(["linked"]).halign(gtk::Align::Center).margin_top(6).margin_bottom(6).homogeneous(true).build();
    let mut first: Option<gtk::ToggleButton> = None;
    for (name, label) in [("auto", gettext("System")), ("day", gettext("Day")), ("dusk", gettext("Dusk")), ("night", gettext("Night"))] {
        let t = gtk::ToggleButton::builder().label(&label).action_name("win.mode").action_target(&name.to_variant()).build();
        t.set_tooltip_text(Some(&match name {
            "auto" => gettext("Follow the system: Day when light, Dusk when dark"),
            "day" => gettext("Day: bright, readable in sunlight"),
            "dusk" => gettext("Dusk: dark"),
            _ => gettext("Night: dim blue-grey, keeps your night vision"),
        }));
        if let Some(f) = &first {
            t.set_group(Some(f));
        } else {
            first = Some(t.clone());
        }
        b.append(&t);
    }
    b
}

/// The Layers popover: check buttons on the `win.layer-<id>` actions, grouped, with what is kept
/// of each download.
pub fn layers_popover(_app: &Rc<AppState>) -> gtk::Popover {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
    b.set_margin_top(6);
    b.set_margin_bottom(6);
    b.set_margin_start(6);
    b.set_margin_end(6);
    b.set_width_request(300);
    let details: Rc<RefCell<Vec<(String, super::chips::ChipLine)>>> = Rc::new(RefCell::new(Vec::new()));
    let groups = isomar_app::vm::layers::groups();
    let last = groups.len() - 1;
    for (gi, g) in groups.into_iter().enumerate() {
        b.append(&gtk::Label::builder().label(&g.title).xalign(0.0).css_classes(["caption-heading", "dim-label"]).margin_start(6).margin_top(6).build());
        for (id, label) in g.layers {
            let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let check = gtk::CheckButton::builder().label(&label).action_name(format!("win.layer-{id}")).build();
            row.append(&check);
            if gi == last || id == isomar_app::places::LAYER {
                let d = super::chips::ChipLine::single(&["caption", "dim-label"], 12);
                d.widget.set_margin_start(30);
                row.append(&d.widget);
                details.borrow_mut().push((id.clone(), d));
            }
            b.append(&row);
        }
        if gi == last {
            b.append(&gtk::Label::builder().label(gettext("Saved on this computer, so they work offline.")).xalign(0.0).wrap(true).css_classes(["caption", "dim-label"]).margin_start(6).margin_top(4).build());
        } else {
            b.append(&gtk::Separator::builder().margin_top(4).build());
        }
    }
    let pop = gtk::Popover::builder().child(&b).build();
    pop.connect_show(move |_| {
        let details = details.clone();
        super::spawn(async move {
            let rows = super::bg(isomar_app::vm::layers::downloads).await;
            for (id, line) in details.borrow().iter() {
                if let Some(r) = rows.iter().find(|r| &r.id == id) {
                    line.set(&r.detail);
                }
            }
        });
    });
    pop
}

/// The jobs' Cancel buttons call `win.cancel-job`.
pub fn install_actions(group: &gio::SimpleActionGroup, app: &Rc<AppState>) {
    let a = gio::SimpleAction::new("cancel-job", Some(glib::VariantTy::UINT64));
    let app = Rc::downgrade(app);
    a.connect_activate(move |_, p| {
        if let (Some(app), Some(id)) = (app.upgrade(), p.and_then(|p| p.get::<u64>())) {
            app.jobs.cancel(id);
        }
    });
    group.add_action(&a);
}
