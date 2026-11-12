//! The right-click menu on the chart, as data for the shell to show. What it
//! offers depends on what is under the pointer. Other modules add entries to
//! the position section through `AppState::add_context_item` (this is how
//! "Get weather" appears).

use super::actions::{self, position_text};
use super::app::{AppState, Selection};
use super::engine::Click;
use super::hit::Hit;
use super::platform::{self, spawn};
use super::router;
use super::shell::{Dialog, Menu, MenuItem};
use crate::i18n::{gettext, gettext_f};
use std::rc::{Rc, Weak};

/// Collects the sections of a menu; each entry runs only while the app state exists.
struct Builder {
    menu: Menu,
    current: Vec<MenuItem>,
    app: Weak<AppState>,
}

impl Builder {
    fn new(app: &Rc<AppState>) -> Builder {
        Builder { menu: Menu::default(), current: Vec::new(), app: Rc::downgrade(app) }
    }

    fn item(&mut self, label: &str, f: impl Fn(&Rc<AppState>) + 'static) {
        let weak = self.app.clone();
        let run: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(app) = weak.upgrade() {
                f(&app);
            }
        });
        self.current.push(MenuItem { label: label.to_string(), run });
    }

    /// Ends the section being built.
    fn commit(&mut self) {
        let section = std::mem::take(&mut self.current);
        if !section.is_empty() {
            self.menu.sections.push(section);
        }
    }

    fn finish(mut self, app: &AppState, x: f64, y: f64) {
        self.commit();
        if let Some(shell) = app.shell() {
            shell.context_menu(self.menu, x, y);
        }
    }
}

/// Deletes after the user confirms.
fn confirm_delete(app: &Rc<AppState>, name: String, delete: impl FnOnce(&Rc<AppState>) + 'static) {
    let app = app.clone();
    spawn(async move {
        let Some(shell) = app.shell() else { return };
        if shell.confirm(&gettext_f("Delete {name}?", &[("name", &name)]), "", &gettext("Delete"), true).await {
            delete(&app);
        }
    });
}

pub fn show(app: &Rc<AppState>, c: Click) {
    let hit = actions::hit_at(app, c.x, c.y);
    let pos = c.pos;
    let mut b = Builder::new(app);
    b.menu.title = position_text(&pos);

    // On an alternative of a weather-route preview, the menu is about it.
    let alts = router::alternatives_at(app, c.x, c.y);
    if !alts.is_empty() {
        for (i, title) in &alts {
            let i = *i;
            let label = if alts.len() == 1 { gettext("Use this route") } else { gettext_f("Use {route}", &[("route", title)]) };
            b.item(&label, move |a| router::use_choice(a, i));
        }
        b.commit();
        for (i, title) in &alts {
            let i = *i;
            let label = if alts.len() == 1 { gettext("Remove this alternative") } else { gettext_f("Remove {route}", &[("route", title)]) };
            b.item(&label, move |a| router::remove_choice(a, i));
        }
        return b.finish(app, c.x, c.y);
    }

    // While drawing a route, the menu is about the route.
    if app.edit.borrow().is_some() {
        b.item(&gettext("Finish route"), actions::finish_edit);
        b.item(&gettext("Remove last point"), actions::remove_last_edit_point);
        b.item(&gettext("Cancel"), actions::cancel_edit);
        return b.finish(app, c.x, c.y);
    }

    match hit {
        Hit::Waypoint(id) => {
            let name = app.session.borrow().waypoints.iter().find(|w| w.id == id).map(|w| w.name.clone()).unwrap_or_default();
            app.select(Selection::Waypoint(id));
            b.item(&gettext("Properties…"), move |a| open(a, Dialog::WaypointProperties(id)));
            let wpos = app.session.borrow().waypoints.iter().find(|w| w.id == id).map(|w| w.pos);
            if let Some(wp) = wpos {
                let n = name.clone();
                b.item(&gettext("Navigate to this waypoint"), move |a| actions::goto(a, wp, &n));
                b.item(&gettext("Start a route here"), move |a| actions::start_route(a, Some(wp)));
            }
            b.item(&gettext("Delete waypoint"), move |a| confirm_delete(a, name.clone(), move |a| actions::delete_waypoint(a, id)));
        }
        Hit::RoutePoint { route, idx } => {
            app.select(Selection::RoutePoint { route, idx });
            b.item(&gettext("Remove this point"), move |a| actions::remove_route_point(a, route, idx));
            route_items(app, &mut b, route);
        }
        Hit::Route(id) => {
            app.select(Selection::Route(id));
            b.item(&gettext("Add a point here"), move |a| actions::insert_route_point(a, id, pos));
            route_items(app, &mut b, id);
        }
        Hit::Target(mmsi) => {
            app.select(Selection::Target(mmsi));
            b.item(&gettext("Vessel information…"), move |a| open(a, Dialog::AisInfo(mmsi)));
            b.item(&gettext("Acknowledge collision alert"), move |a| {
                a.session.borrow_mut().acknowledge_ais(mmsi);
                a.refresh();
            });
        }
        Hit::EditPoint(_) | Hit::None => {}
    }
    b.commit();

    let text = position_text(&pos);
    b.item(&gettext_f("Copy {position}", &[("position", &text)]), move |_| platform::set_clipboard(&text));
    if app.session.borrow().own.pos.is_some() {
        b.item(&gettext("Navigate here"), move |a| actions::goto(a, pos, &gettext("Here")));
    }
    b.item(&gettext("Drop waypoint here"), move |a| actions::drop_waypoint(a, pos));
    b.item(&gettext("New route from here"), move |a| actions::start_route(a, Some(pos)));
    b.item(&gettext("Measure from here"), move |a| actions::begin_measure(a, pos));
    if app.session.borrow().anchor.is_some() {
        b.item(&gettext("Clear anchor watch"), |a| {
            a.session.borrow_mut().clear_anchor();
            a.refresh();
        });
    } else {
        b.item(&gettext("Set anchor watch here"), move |a| {
            let r = a.settings.borrow().anchor_radius_nm;
            actions::set_anchor(a, pos, r);
        });
    }
    b.commit();

    let extras = app.context_items.borrow().clone();
    for item in extras {
        let run = item.run.clone();
        b.item(&item.label, move |a| run(a, pos));
    }
    b.finish(app, c.x, c.y);
}

fn open(app: &AppState, d: Dialog) {
    if let Some(shell) = app.shell() {
        shell.open(d);
    }
}

fn route_items(app: &Rc<AppState>, b: &mut Builder, id: i64) {
    let name = app.session.borrow().routes.iter().find(|r| r.id == id).map(|r| r.name.clone()).unwrap_or_default();
    b.item(&gettext("Activate route"), move |a| actions::activate_route(a, id));
    b.item(&gettext("Edit route"), move |a| actions::edit_route(a, id));
    b.item(&gettext("Reverse route"), move |a| actions::reverse_route(a, id));
    b.item(&gettext("Route properties…"), move |a| open(a, Dialog::RouteProperties(id)));
    // A route made by weather routing is not routed again.
    if !crate::flows::routing::is_weather_route(app, id) {
        b.item(&gettext("Weather route…"), move |a| open(a, Dialog::WeatherRoute(id)));
        b.item(&gettext("Best departure…"), move |a| open(a, Dialog::BestDeparture(id)));
    }
    b.item(&gettext("Auto route…"), move |a| open(a, Dialog::AutoRoute(id)));
    b.item(&gettext("Delete route"), move |a| confirm_delete(a, name.clone(), move |a| actions::delete_route(a, id)));
}
