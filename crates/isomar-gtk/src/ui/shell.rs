//! The GTK implementation of `isomar_app::shell::Shell`: toasts, dialogs and
//! the context menu.

use super::popovers;
use super::app::AppState;
use adw::prelude::*;
use isomar_app::aidinfo::Description;
use isomar_app::shell::{Dialog, Menu, Shell};
use gtk::{gdk, gio};
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::{Rc, Weak};

pub struct GtkShell {
    toasts: adw::ToastOverlay,
    app: RefCell<Weak<AppState>>,
    sidebar: RefCell<Weak<super::sidebar::Sidebar>>,
}

impl GtkShell {
    pub fn new(toasts: &adw::ToastOverlay) -> Rc<GtkShell> {
        Rc::new(GtkShell { toasts: toasts.clone(), app: RefCell::new(Weak::new()), sidebar: RefCell::new(Weak::new()) })
    }

    /// Tells the shell which state its dialogs work on, and where details open.
    pub fn attach(&self, app: &Rc<AppState>, sidebar: &Rc<super::sidebar::Sidebar>) {
        *self.app.borrow_mut() = Rc::downgrade(app);
        *self.sidebar.borrow_mut() = Rc::downgrade(sidebar);
    }
}

impl Shell for GtkShell {
    fn toast(&self, text: &str) {
        let t = adw::Toast::new(text);
        t.set_timeout((isomar_app::tokens::TOAST_DURATION / 1000.0) as u32);
        self.toasts.add_toast(t);
    }

    fn show_error(&self, heading: &str, details: &str) {
        super::show_error(heading, details);
    }

    fn sound(&self, sound: isomar_app::shell::Sound) {
        play(sound);
    }

    fn confirm(&self, heading: &str, body: &str, ok_label: &str, destructive: bool) -> Pin<Box<dyn Future<Output = bool>>> {
        let (heading, body, ok) = (heading.to_string(), body.to_string(), ok_label.to_string());
        Box::pin(async move { super::confirm(&heading, &body, &ok, destructive).await })
    }

    fn context_menu(&self, menu: Menu, x: f64, y: f64) {
        let model = gio::Menu::new();
        let group = gio::SimpleActionGroup::new();
        let mut n = 0;
        for section in &menu.sections {
            let gsection = gio::Menu::new();
            for item in section {
                let name = format!("a{n}");
                n += 1;
                let action = gio::SimpleAction::new(&name, None);
                let run = item.run.clone();
                action.connect_activate(move |_, _| run());
                group.add_action(&action);
                gsection.append(Some(&item.label), Some(&format!("ctx.{name}")));
            }
            model.append_section(None, &gsection);
        }
        super::chart_widget().insert_action_group("ctx", Some(&group));
        super::popup_menu(&super::chart_widget(), x, y, &model);
    }

    fn object_info(&self, info: &Description, x: f64, y: f64) {
        let app = self.app.borrow().upgrade();
        popovers::show_description(app.as_ref(), info, x, y);
    }

    fn notify(&self, title: &str, body: &str) {
        // In front, the result is on screen already.
        let window = super::chart_widget().root().and_downcast::<gtk::Window>();
        if window.is_none_or(|w| w.is_active()) {
            return;
        }
        let Some(app) = gio::Application::default() else { return };
        let n = gio::Notification::new(title);
        n.set_body(Some(body));
        app.send_notification(Some("finished"), &n);
    }

    fn set_cursor(&self, pointing: bool) {
        super::chart_widget().set_cursor_from_name(pointing.then_some("pointer"));
    }

    fn open(&self, dialog: Dialog) {
        let Some(app) = self.app.borrow().upgrade() else { return };
        let page = |name: &str, page: Option<adw::NavigationPage>| {
            if let (Some(p), Some(s)) = (page, self.sidebar.borrow().upgrade()) {
                s.push(name, &p);
            }
        };
        match dialog {
            Dialog::WaypointProperties(id) => page("passage", super::pages::details::waypoint(&app, id)),
            Dialog::RouteProperties(id) => page("passage", super::pages::details::route(&app, id)),
            Dialog::AisInfo(mmsi) => popovers::show_vessel(&app, mmsi),
            Dialog::WeatherRoute(id) => page("passage", super::pages::routing::page(&app, id, Some(super::pages::routing::Mode::Fixed))),
            Dialog::BestDeparture(id) => page("passage", super::pages::routing::page(&app, id, Some(super::pages::routing::Mode::Window))),
            Dialog::AutoRoute(id) => super::autoroute::open(&app, id),
            Dialog::GetWeather { north, south, west, east } => super::weather::dialog::open(&app, isomar_app::core::weather::planner::Area::new(south, west, north, east)),
            Dialog::Tides(pos) => popovers::show_tides(&app, pos),
            Dialog::ChartsHere(pos) => {
                app.canvas.center_on(pos);
                if let Some(s) = self.sidebar.borrow().upgrade() {
                    s.show("charts");
                    if let Some(split) = s.widget.ancestor(adw::OverlaySplitView::static_type()).and_downcast::<adw::OverlaySplitView>() {
                        split.set_show_sidebar(true);
                    }
                }
            }
        }
    }
}

thread_local! {
    static SOUNDS: RefCell<Vec<(isomar_app::shell::Sound, gtk::MediaFile)>> = const { RefCell::new(Vec::new()) };
}

/// Plays one of the bundled sounds (`data/sounds`), from the start; a beep if it cannot.
fn play(sound: isomar_app::shell::Sound) {
    use isomar_app::shell::Sound;
    let media = SOUNDS.with(|s| {
        let mut s = s.borrow_mut();
        if let Some((_, m)) = s.iter().find(|(k, _)| *k == sound) {
            return m.clone();
        }
        let name = match sound {
            Sound::Alarm => "alarm",
            Sound::Chime => "chime",
            Sound::Tick => "tick",
        };
        let m = gtk::MediaFile::for_resource(&format!("/eu/_6px/isomar/sounds/{name}.wav"));
        s.push((sound, m.clone()));
        m
    });
    if media.error().is_some() {
        if let Some(d) = gdk::Display::default() {
            d.beep();
        }
        return;
    }
    media.seek(0);
    media.play();
}
