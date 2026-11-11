//! The task dialog template (`design/screens/dialogs-gnome-task.png`): Cancel and one primary
//! action in the header, groups of libadwaita rows, units as suffixes, Advanced folded in an
//! expander, and the planning-only line at the foot. In a window of its own (`float`), so the
//! chart under it still pans and zooms.

use crate::i18n::gettext;
use adw::prelude::*;
use gtk::glib;
use std::cell::RefCell;
use std::rc::Rc;

/// Groups of libadwaita rows in a column, built one after the other: the body of a [`Form`], or of
/// a sidebar page laid out the same way.
pub struct Rows {
    pub page: gtk::Box,
    group: RefCell<adw::PreferencesGroup>,
    /// While set, rows go into this expander (Advanced) instead of the group.
    expander: RefCell<Option<adw::ExpanderRow>>,
}

impl Rows {
    /// Rows appended to `page`, starting with an untitled group.
    pub fn new(page: &gtk::Box) -> Self {
        let group = adw::PreferencesGroup::new();
        page.append(&group);
        Self { page: page.clone(), group: RefCell::new(group), expander: RefCell::new(None) }
    }

    /// Starts a new titled group of rows.
    pub fn group(&self, title: &str) -> adw::PreferencesGroup {
        let g = adw::PreferencesGroup::new();
        if !title.is_empty() {
            g.set_title(title);
        }
        self.page.append(&g);
        *self.group.borrow_mut() = g.clone();
        *self.expander.borrow_mut() = None;
        g
    }

    /// Starts the folded Advanced group: the rows added after it go inside, with `summary` saying
    /// what they are set to while it is closed.
    pub fn advanced(&self, summary: &str) -> adw::ExpanderRow {
        let g = adw::PreferencesGroup::new();
        self.page.append(&g);
        let e = adw::ExpanderRow::builder().title(gettext("Advanced")).subtitle(summary).build();
        g.add(&e);
        *self.group.borrow_mut() = g;
        *self.expander.borrow_mut() = Some(e.clone());
        e
    }

    /// The quiet line at the foot: "Planning only…".
    pub fn footnote(&self, text: &str) {
        let l = gtk::Label::builder().label(text).xalign(0.0).wrap(true).css_classes(["caption", "dim-label"]).margin_top(6).build();
        self.page.append(&l);
    }

    /// The planning-only line, for dialogs whose result is a route.
    pub fn planning_only(&self) {
        self.footnote(&gettext("Planning only. Check the result against official charts before you sail."));
    }

    /// A number with its unit as a suffix: `3.0 m`.
    pub fn number(&self, title: &str, unit: &str, (min, max, step): (f64, f64, f64), digits: u32, value: f64) -> adw::SpinRow {
        let s = adw::SpinRow::with_range(min, max, step);
        s.set_title(title);
        s.set_digits(digits);
        s.set_value(value.clamp(min, max));
        if !unit.is_empty() {
            let u = gtk::Label::builder().label(unit).css_classes(["dim-label"]).build();
            s.add_suffix(&u);
        }
        self.add(&s);
        s
    }

    /// A date and time, chosen from a calendar.
    pub fn datetime(&self, title: &str, value: Option<chrono::DateTime<chrono::Utc>>, optional: bool) -> Rc<super::datetime::DateTimeRow> {
        let r = super::datetime::DateTimeRow::new(title, value, optional);
        self.add(&r.row);
        r
    }

    /// A row that only tells: a title, what it is, an icon before and maybe a widget after.
    pub fn line(&self, icon: &str, title: &str, subtitle: &str) -> adw::ActionRow {
        let r = adw::ActionRow::builder().title(title).subtitle(subtitle).build();
        if !icon.is_empty() {
            r.add_prefix(&gtk::Image::from_icon_name(icon));
        }
        self.add(&r);
        r
    }

    /// [`Rows::line`] with a line of values, each after its icon, where the subtitle would be.
    pub fn chip_line(&self, icon: &str, title: &str, chips: &[isomar_app::chips::Chip]) -> adw::ActionRow {
        let r = super::chips::row((!icon.is_empty()).then_some(icon), title, chips);
        self.add(&r);
        r
    }

    pub fn description(&self, text: &str) {
        self.group.borrow().set_description(Some(text));
    }

    pub fn add(&self, w: &impl IsA<gtk::Widget>) {
        match self.expander.borrow().as_ref() {
            Some(e) => e.add_row(w),
            None => self.group.borrow().add(w),
        }
    }

    pub fn entry(&self, title: &str, initial: &str) -> adw::EntryRow {
        let e = adw::EntryRow::builder()
            .title(title)
            .text(initial)
            .activates_default(true)
            .build();
        self.add(&e);
        e
    }

    pub fn combo(&self, title: &str, items: &[String], selected: Option<&str>) -> adw::ComboRow {
        let model = gtk::StringList::new(&items.iter().map(|s| s.as_str()).collect::<Vec<_>>());
        let c = adw::ComboRow::builder().title(title).model(&model).build();
        let searchable = items.len() > 12;
        let long = items.iter().any(|s| s.chars().count() > 24);
        if searchable || long {
            c.set_expression(Some(gtk::PropertyExpression::new(
                gtk::StringObject::static_type(),
                None::<&gtk::Expression>,
                "string",
            )));
        }
        c.set_enable_search(searchable);
        if long {
            // Long values would be ellipsized beside the title: show the
            // selection as a wrapping subtitle, and wrap the entries of the
            // popup too.
            c.set_use_subtitle(true);
            c.set_use_markup(false);
            c.set_list_factory(Some(&wrapping_factory()));
            c.connect_selected_notify(|c| c.set_subtitle(&combo_value(c)));
        }
        if let Some(sel) = selected
            && let Some(i) = items.iter().position(|s| s == sel) {
                c.set_selected(i as u32);
            }
        if long {
            c.set_subtitle(&combo_value(&c));
        }
        self.add(&c);
        c
    }

    pub fn switch(&self, title: &str, subtitle: &str, active: bool) -> adw::SwitchRow {
        let s = adw::SwitchRow::builder().title(title).active(active).build();
        if !subtitle.is_empty() {
            s.set_subtitle(subtitle);
        }
        self.add(&s);
        s
    }

    /// Folds the rows added after it into an expander row of the current group.
    pub fn fold(&self, title: &str, summary: &str) -> adw::ExpanderRow {
        let e = adw::ExpanderRow::builder().title(title).subtitle(summary).build();
        self.group.borrow().add(&e);
        *self.expander.borrow_mut() = Some(e.clone());
        e
    }
}

pub struct Form {
    pub dialog: super::float::Panel,
    rows: Rows,
    pub ok: gtk::Button,
    focus: RefCell<Option<gtk::Widget>>,
    tx: async_channel::Sender<bool>,
    rx: async_channel::Receiver<bool>,
    validators: RefCell<Vec<Rc<dyn Fn() -> bool>>>,
}

impl std::ops::Deref for Form {
    type Target = Rows;

    fn deref(&self) -> &Rows {
        &self.rows
    }
}

impl Form {
    pub fn new(title: &str, ok_label: &str) -> Rc<Self> {
        let dialog = super::float::window(title, 520, -1);
        let header = adw::HeaderBar::builder()
            .show_start_title_buttons(false)
            .show_end_title_buttons(false)
            .build();
        let cancel = gtk::Button::with_label(&gettext("Cancel"));
        let ok = gtk::Button::with_label(ok_label);
        ok.add_css_class("suggested-action");
        header.pack_start(&cancel);
        header.pack_end(&ok);

        let page = gtk::Box::new(gtk::Orientation::Vertical, 18);
        page.set_margin_top(12);
        page.set_margin_bottom(18);
        page.set_margin_start(18);
        page.set_margin_end(18);
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&page)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(640)
            .build();
        let tv = adw::ToolbarView::new();
        tv.add_top_bar(&header);
        tv.set_content(Some(&scrolled));
        dialog.set_content(Some(&tv));
        dialog.set_default_widget(Some(&ok));


        let (tx, rx) = async_channel::bounded(1);
        let form = Rc::new(Self {
            dialog,
            rows: Rows::new(&page),
            ok: ok.clone(),
            focus: RefCell::new(None),
            tx,
            rx,
            validators: RefCell::new(Vec::new()),
        });

        let f = Rc::downgrade(&form);
        ok.connect_clicked(move |_| {
            if let Some(f) = f.upgrade() {
                let _ = f.tx.try_send(true);
                f.dialog.close();
            }
        });
        let d = form.dialog.clone();
        cancel.connect_clicked(move |_| {
            d.close();
        });
        let tx = form.tx.clone();
        form.dialog.connect_close_request(move |_| {
            let _ = tx.try_send(false);
            glib::Propagation::Proceed
        });
        form
    }

    /// Adds a condition that must hold for OK to be enabled. Call
    /// [`Form::revalidate`] from change handlers.
    pub fn validate(self: &Rc<Self>, f: impl Fn() -> bool + 'static) {
        self.validators.borrow_mut().push(Rc::new(f));
        self.revalidate();
    }

    pub fn revalidate(&self) {
        let ok = self.validators.borrow().iter().all(|v| v());
        self.ok.set_sensitive(ok);
    }

    /// Revalidate whenever this editable changes.
    pub fn watch(self: &Rc<Self>, e: &impl IsA<gtk::Editable>) {
        let f = Rc::downgrade(self);
        e.connect_changed(move |_| {
            if let Some(f) = f.upgrade() {
                f.revalidate();
            }
        });
    }

    pub fn watch_combo(self: &Rc<Self>, c: &adw::ComboRow) {
        let f = Rc::downgrade(self);
        c.connect_selected_notify(move |_| {
            if let Some(f) = f.upgrade() {
                f.revalidate();
            }
        });
    }

    /// Presents the dialog and resolves to true when OK was pressed.
    pub async fn run(&self) -> bool {
        self.dialog.present();
        if std::env::var_os("ISOMAR_AUTO_ACCEPT").is_some() {
            // Development aid: accept with default values.
            let ok = self.ok.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
                if ok.is_sensitive() {
                    ok.emit_clicked();
                }
            });
        }
        let target = self.focus.borrow().clone();
        let first = self.rows.page.first_child();
        glib::idle_add_local_once(move || {
            if let Some(w) = target {
                w.grab_focus();
            } else if let Some(w) = first {
                w.child_focus(gtk::DirectionType::TabForward);
            }
        });
        self.rx.recv().await.unwrap_or(false)
    }
}

/// Popup entries for combo rows with long strings: wrapped instead of
/// ellipsized, with the usual checkmark on the selected one.
fn wrapping_factory() -> gtk::SignalListItemFactory {
    let f = gtk::SignalListItemFactory::new();
    f.connect_setup(|_, obj| {
        let item = obj.downcast_ref::<gtk::ListItem>().unwrap();
        let label = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .max_width_chars(48)
            .hexpand(true)
            .build();
        let check = gtk::Image::from_icon_name("object-select-symbolic");
        item.bind_property("selected", &check, "opacity")
            .transform_to(|_, selected: bool| Some(if selected { 1.0 } else { 0.0 }))
            .sync_create()
            .build();
        let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        hbox.append(&label);
        hbox.append(&check);
        item.set_child(Some(&hbox));
    });
    f.connect_bind(|_, obj| {
        let item = obj.downcast_ref::<gtk::ListItem>().unwrap();
        let text = item
            .item()
            .and_downcast::<gtk::StringObject>()
            .map(|s| s.string().to_string())
            .unwrap_or_default();
        if let Some(label) = item.child().and_then(|b| b.first_child()).and_downcast::<gtk::Label>() {
            label.set_label(&text);
        }
    });
    f
}

/// Selected string of a combo row built with [`Form::combo`].
pub fn combo_value(c: &adw::ComboRow) -> String {
    c.selected_item()
        .and_downcast::<gtk::StringObject>()
        .map(|s| s.string().to_string())
        .unwrap_or_default()
}
