//! The alert chrome (`design/design-system/alert-hierarchy.md`), drawn from `vm::banner`: the
//! alarm strip pinned over the chart with warnings below it, the frame that pulses round the chart
//! while an alarm sounds, the hint card, and a notification when the window is in the background.

use super::app::AppState;
use crate::core::alerts::Action;
use crate::i18n::gettext;
use adw::prelude::*;
use isomar_app::tokens;
use isomar_app::vm::banner::{self, Alerts, Hint, Strip};
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const NOTIFICATION_ID: &str = "alarm";

/// The strips over the chart: one alarm and the warnings under it.
pub struct Strips {
    pub widget: gtk::Box,
    shown: RefCell<(Option<Strip>, Vec<Strip>)>,
    app: std::rc::Weak<AppState>,
}

impl Strips {
    pub fn new(app: &Rc<AppState>) -> Rc<Strips> {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.set_valign(gtk::Align::Start);
        Rc::new(Strips { widget, shown: RefCell::new((None, Vec::new())), app: Rc::downgrade(app) })
    }

    pub fn update(&self, a: &Alerts) {
        let now = (a.alarm.clone(), a.warnings.clone());
        if *self.shown.borrow() == now {
            return;
        }
        while let Some(c) = self.widget.first_child() {
            self.widget.remove(&c);
        }
        if let Some(s) = &a.alarm {
            self.widget.append(&self.strip(s));
        }
        for w in &a.warnings {
            self.widget.append(&self.strip(w));
        }
        *self.shown.borrow_mut() = now;
    }

    fn strip(&self, s: &Strip) -> gtk::Box {
        let alarm = s.level == "alarm";
        let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        b.add_css_class("alert-strip");
        b.add_css_class(if alarm { "alarm" } else { "warning" });
        if s.dim {
            b.add_css_class("dim");
        }
        let icon = gtk::Image::from_icon_name(if alarm { "isomar-alarm-symbolic" } else { "dialog-warning-symbolic" });
        icon.set_valign(gtk::Align::Start);
        icon.set_margin_top(8);
        b.append(&icon);
        // The buttons go under the words when the window is narrow.
        let wrap = adw::WrapBox::builder().child_spacing(10).line_spacing(4).hexpand(true).build();
        let title = gtk::Label::builder().label(&s.title).xalign(0.0).wrap(true).css_classes(["heading"]).valign(gtk::Align::Center).build();
        wrap.append(&title);
        if let Some(l) = &s.count_label {
            let count = gtk::Box::builder().spacing(4).valign(gtk::Align::Center).css_classes(["alarm-count"]).tooltip_text(l).build();
            count.append(&gtk::Image::from_icon_name(super::chips::icon_name(isomar_app::chips::Icon::Alarm)));
            count.append(&gtk::Label::builder().label(s.count.to_string()).css_classes(["heading", "numeric"]).build());
            wrap.append(&count);
        }
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let button = |label: &str, action: Action| {
            let btn = gtk::Button::with_label(label);
            btn.set_valign(gtk::Align::Center);
            let (app, id) = (self.app.clone(), s.id);
            btn.connect_clicked(move |_| {
                if let Some(app) = app.upgrade() {
                    banner::press(&app, id, action);
                    if let Some(g) = gio::Application::default() {
                        g.withdraw_notification(NOTIFICATION_ID);
                    }
                }
            });
            btn
        };
        if let Some(l) = &s.snooze {
            buttons.append(&button(l, Action::Snooze));
        }
        if let Some(l) = &s.acknowledge {
            let ack = button(l, Action::Acknowledge);
            ack.add_css_class("strong");
            buttons.append(&ack);
        }
        if let Some(l) = &s.action {
            let btn = gtk::Button::with_label(l);
            btn.set_valign(gtk::Align::Center);
            btn.add_css_class("strong");
            let (app, id) = (self.app.clone(), s.id);
            btn.connect_clicked(move |_| {
                if let Some(app) = app.upgrade() {
                    banner::press_action(&app, id);
                }
            });
            buttons.append(&btn);
        }
        if let Some(l) = &s.dismiss {
            buttons.append(&button(l, Action::Dismiss));
        }
        wrap.append(&buttons);
        b.append(&wrap);
        b.update_property(&[gtk::accessible::Property::Label(&with_count(s))]);
        b.set_accessible_role(gtk::AccessibleRole::Alert);
        b
    }
}

/// The frame round the chart while an alarm sounds: `stroke-alarm-frame` pulsing between 4 and 8
/// px at `alarm-pulse`, steady at 8 px when animations are off.
pub fn alarm_frame() -> (gtk::DrawingArea, Rc<Cell<bool>>) {
    let area = gtk::DrawingArea::builder().can_target(false).can_focus(false).hexpand(true).vexpand(true).build();
    let on = Rc::new(Cell::new(false));
    let start = std::time::Instant::now();
    area.set_draw_func(move |_, cr, w, h| {
        let animate = gtk::Settings::default().is_some_and(|s| s.is_gtk_enable_animations());
        let width = if animate {
            let phase = (start.elapsed().as_secs_f64() * 1000.0 / tokens::ALARM_PULSE) * std::f64::consts::TAU;
            tokens::STROKE_ALARM_FRAME + 2.0 * phase.sin()
        } else {
            8.0
        };
        let (r, g, b, _) = tokens::color(tokens::Token::Alarm, super::theme::token_theme());
        cr.set_source_rgb(r, g, b);
        cr.set_line_width(width);
        cr.rectangle(width / 2.0, width / 2.0, w as f64 - width, h as f64 - width);
        let _ = cr.stroke();
    });
    {
        let on = on.clone();
        area.add_tick_callback(move |a, _| {
            a.set_visible(on.get());
            if on.get() {
                a.queue_draw();
            }
            glib::ControlFlow::Continue
        });
    }
    (area, on)
}

/// The hint card: what the mode the user is in wants, with its buttons. Never a banner.
pub struct HintCard {
    pub widget: gtk::Box,
    title: gtk::Label,
    finish: gtk::Button,
    undo: gtk::Button,
    cancel: gtk::Button,
    shown: RefCell<Option<Hint>>,
}

impl HintCard {
    pub fn new(app: &Rc<AppState>) -> Rc<HintCard> {
        let widget = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        widget.add_css_class("isomar-card");
        widget.add_css_class("hint-card");
        widget.set_halign(gtk::Align::Start);
        widget.set_visible(false);
        widget.append(&gtk::Image::from_icon_name("document-edit-symbolic"));
        let title = gtk::Label::builder().xalign(0.0).wrap(true).max_width_chars(40).css_classes(["dim-label"]).build();
        widget.append(&title);
        let undo = gtk::Button::builder().icon_name("edit-undo-symbolic").tooltip_text(gettext("Undo")).css_classes(["flat"]).valign(gtk::Align::Center).build();
        let finish = gtk::Button::builder().label(gettext("Finish")).css_classes(["suggested-action"]).valign(gtk::Align::Center).build();
        let cancel = gtk::Button::builder().label(gettext("Cancel")).valign(gtk::Align::Center).build();
        widget.append(&undo);
        widget.append(&cancel);
        widget.append(&finish);
        for (b, which) in [(&undo, "undo"), (&finish, "finish"), (&cancel, "cancel")] {
            let app = Rc::downgrade(app);
            b.connect_clicked(move |_| {
                if let Some(app) = app.upgrade() {
                    banner::press_hint(&app, which);
                }
            });
        }
        Rc::new(HintCard { widget, title, finish, undo, cancel, shown: RefCell::new(None) })
    }

    pub fn update(&self, h: Option<&Hint>) {
        if self.shown.borrow().as_ref() == h {
            return;
        }
        *self.shown.borrow_mut() = h.cloned();
        self.widget.set_visible(h.is_some());
        let Some(h) = h else { return };
        self.title.set_text(&h.title);
        self.finish.set_visible(h.finish.is_some());
        self.undo.set_visible(h.undo.is_some());
        self.cancel.set_visible(h.cancel.is_some());
        if let Some(l) = &h.finish {
            self.finish.set_label(l);
        }
        if let Some(l) = &h.cancel {
            self.cancel.set_label(l);
        }
    }
}

/// While the window is in the background, a new alarm also goes out as an urgent notification with
/// an Acknowledge button (`app.acknowledge-alarm`); it is withdrawn when the alarm ends.
pub struct Notifier {
    sent: Cell<Option<u64>>,
}

impl Notifier {
    pub fn new() -> Notifier {
        Notifier { sent: Cell::new(None) }
    }

    pub fn update(&self, window: &impl IsA<gtk::Window>, a: &Alerts) {
        let Some(app) = gio::Application::default() else { return };
        let live = a.alarm.as_ref().filter(|s| !s.dim);
        match live {
            Some(s) if !window.is_active() && self.sent.get() != Some(s.id) => {
                let n = gio::Notification::new(&gettext("Alarm"));
                n.set_body(Some(&with_count(s)));
                n.set_priority(gio::NotificationPriority::Urgent);
                n.add_button_with_target_value(&gettext("Acknowledge"), "app.acknowledge-alarm", Some(&s.id.to_variant()));
                app.send_notification(Some(NOTIFICATION_ID), &n);
                self.sent.set(Some(s.id));
            }
            None if self.sent.get().is_some() => {
                app.withdraw_notification(NOTIFICATION_ID);
                self.sent.set(None);
            }
            _ => {}
        }
    }
}

/// The application action a notification's Acknowledge button calls.
pub fn install_action(application: &adw::Application, app: &Rc<AppState>) {
    let a = gio::SimpleAction::new("acknowledge-alarm", Some(glib::VariantTy::UINT64));
    let app = Rc::downgrade(app);
    a.connect_activate(move |_, p| {
        if let (Some(app), Some(id)) = (app.upgrade(), p.and_then(|p| p.get::<u64>())) {
            banner::press(&app, id, Action::Acknowledge);
        }
    });
    application.add_action(&a);
}

/// The alert's words, with how many alarms there are when there are several: "Anchor alarm, 2 alarms".
fn with_count(s: &Strip) -> String {
    match &s.count_label {
        Some(l) => format!("{}, {l}", s.title),
        None => s.title.clone(),
    }
}
