//! A row for a date and time: a calendar and hour and minute spinners in a popover, in local
//! time, instead of typing "YYYY-MM-DD HH:MM".

use crate::i18n::gettext;
use adw::prelude::*;
use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone, Timelike, Utc};
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub struct DateTimeRow {
    pub row: adw::ActionRow,
    value: Rc<RefCell<Option<DateTime<Utc>>>>,
    label: gtk::Label,
    optional: bool,
    changed: RefCell<Option<Box<dyn Fn()>>>,
}

fn text(v: Option<DateTime<Utc>>, optional: bool) -> String {
    match v {
        // The year only when it is not this one, so that the row fits the sidebar.
        Some(t) => {
            let t = t.with_timezone(&Local);
            t.format(if t.year() == Local::now().year() { "%a %d %b, %H:%M" } else { "%a %d %b %Y, %H:%M" }).to_string()
        }
        None if optional => gettext("Not set"),
        None => String::new(),
    }
}

impl DateTimeRow {
    /// A row titled `title`. An optional one can be cleared (no time, such as "leave now").
    pub fn new(title: &str, value: Option<DateTime<Utc>>, optional: bool) -> Rc<DateTimeRow> {
        let row = adw::ActionRow::builder().title(title).activatable(true).build();
        let label = gtk::Label::builder().css_classes(["numeric"]).build();
        let value = Rc::new(RefCell::new(value));
        let calendar = gtk::Calendar::new();
        let hour = gtk::SpinButton::with_range(0.0, 23.0, 1.0);
        let minute = gtk::SpinButton::with_range(0.0, 59.0, 5.0);
        for s in [&hour, &minute] {
            s.set_orientation(gtk::Orientation::Vertical);
            s.set_wrap(true);
            s.set_numeric(true);
        }
        hour.update_property(&[gtk::accessible::Property::Label(&gettext("Hour"))]);
        minute.update_property(&[gtk::accessible::Property::Label(&gettext("Minute"))]);
        // Minutes print with two digits.
        minute.connect_output(|s| {
            s.set_text(&format!("{:02}", s.value() as i64));
            glib::Propagation::Stop
        });
        let time = gtk::Box::builder().spacing(4).halign(gtk::Align::Center).build();
        time.append(&hour);
        time.append(&gtk::Label::new(Some(":")));
        time.append(&minute);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.append(&calendar);
        content.append(&time);
        let clear = gtk::Button::builder().label(gettext("Clear")).visible(optional).build();
        content.append(&clear);
        let pop = gtk::Popover::builder().child(&content).build();
        let button = gtk::MenuButton::builder().popover(&pop).icon_name("x-office-calendar-symbolic").valign(gtk::Align::Center).css_classes(["flat"]).tooltip_text(gettext("Choose a date and time")).build();
        row.add_suffix(&label);
        row.add_suffix(&button);
        row.set_activatable_widget(Some(&button));

        let me = Rc::new(DateTimeRow { row, value, label, optional, changed: RefCell::new(None) });
        me.label.set_text(&text(*me.value.borrow(), optional));

        // Opening shows the value (or now); every change applies at once.
        let syncing = Rc::new(Cell::new(false));
        {
            let (me2, calendar, hour, minute, syncing) = (Rc::downgrade(&me), calendar.clone(), hour.clone(), minute.clone(), syncing.clone());
            pop.connect_show(move |_| {
                let Some(me) = me2.upgrade() else { return };
                // Unset, the next whole hour.
                let next_hour = || {
                    let t = Local::now() + chrono::Duration::hours(1);
                    t.with_minute(0).unwrap_or(t)
                };
                let t = me.value.borrow().map(|v| v.with_timezone(&Local)).unwrap_or_else(next_hour);
                syncing.set(true);
                if let Ok(d) = glib::DateTime::from_local(t.year(), t.month() as i32, t.day() as i32, 0, 0, 0.0) {
                    calendar.select_day(&d);
                }
                hour.set_value(t.hour() as f64);
                minute.set_value(t.minute() as f64);
                syncing.set(false);
            });
        }
        let apply = {
            let (me2, calendar, hour, minute, syncing) = (Rc::downgrade(&me), calendar.clone(), hour.clone(), minute.clone(), syncing.clone());
            Rc::new(move || {
                if syncing.get() {
                    return;
                }
                let Some(me) = me2.upgrade() else { return };
                let d = calendar.date();
                let Some(day) = NaiveDate::from_ymd_opt(d.year(), d.month() as u32, d.day_of_month() as u32) else { return };
                let Some(naive) = day.and_hms_opt(hour.value() as u32, minute.value() as u32, 0) else { return };
                if let Some(t) = Local.from_local_datetime(&naive).earliest() {
                    me.set(Some(t.with_timezone(&Utc)));
                }
            })
        };
        {
            let a = apply.clone();
            calendar.connect_day_selected(move |_| a());
        }
        {
            let a = apply.clone();
            hour.connect_value_changed(move |_| a());
        }
        minute.connect_value_changed(move |_| apply());
        {
            let (me2, pop) = (Rc::downgrade(&me), pop.clone());
            clear.connect_clicked(move |_| {
                if let Some(me) = me2.upgrade() {
                    me.set(None);
                }
                pop.popdown();
            });
        }
        me
    }

    pub fn get(&self) -> Option<DateTime<Utc>> {
        *self.value.borrow()
    }

    pub fn set(&self, v: Option<DateTime<Utc>>) {
        *self.value.borrow_mut() = v;
        self.label.set_text(&text(v, self.optional));
        if let Some(f) = &*self.changed.borrow() {
            f();
        }
    }

    /// `f` runs whenever the value changes.
    pub fn connect_changed(&self, f: impl Fn() + 'static) {
        *self.changed.borrow_mut() = Some(Box::new(f));
    }

    /// The value as the shared code's forms take it: "2026-10-27 15:30" in local time, or empty.
    pub fn form_text(&self) -> String {
        self.get().map(|t| t.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default()
    }
}
