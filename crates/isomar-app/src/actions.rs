//! What the user can do to the chart: the operations behind clicks, menus and
//! keys. Dialogs live in `dialogs`; this file changes state.

use super::app::{AppState, DragItem, EditRoute, Measure, Selection};
use super::context;
use super::engine::{Click, DragPhase};
use super::key::Key;
use super::hit::{Hit, hit_test};
use crate::core::geo::LatLon;
use crate::core::nav::model::{Route, RoutePoint};
use crate::core::session::Notice;
use crate::i18n::{gettext, gettext_f};
use std::rc::Rc;


pub use super::text::position_text;

fn db_error(app: &AppState, e: rusqlite::Error) {
    app.toast(&gettext_f("Could not save: {error}", &[("error", &e.to_string())]));
}

pub fn drop_waypoint(app: &Rc<AppState>, pos: LatLon) {
    let res = app.session.borrow_mut().add_waypoint(pos, None);
    match res {
        Ok(w) => {
            app.select(Selection::Waypoint(w.id));
            app.toast(&gettext_f("Waypoint {name} added", &[("name", &w.name)]));
        }
        Err(e) => db_error(app, e),
    }
}

pub fn goto(app: &Rc<AppState>, pos: LatLon, name: &str) {
    let ok = app.session.borrow_mut().goto(pos, name);
    if ok {
        app.session.borrow_mut().update(chrono::Utc::now());
        super::vm::underway::auto(app, true);
        app.refresh();
    } else {
        app.toast(&gettext("No position fix, so there is nothing to navigate from"));
    }
}

pub fn start_route(app: &Rc<AppState>, first: Option<LatLon>) {
    let n = app.session.borrow().routes.len() + 1;
    let mut r = Route::new(gettext_f("Route {n}", &[("n", &n.to_string())]));
    if let Some(p) = first {
        r.points.push(RoutePoint::new("", p));
    }
    *app.edit.borrow_mut() = Some(EditRoute { route: r });
    *app.measure.borrow_mut() = None;
    app.toast(&if app.canvas.is_touch() {
        gettext("Tap to add points, double-tap to finish.")
    } else {
        gettext("Click to add points. Double-click or press Enter to finish, Esc to cancel.")
    });
    app.refresh();
}

pub fn edit_route(app: &Rc<AppState>, id: i64) {
    let r = app.session.borrow().routes.iter().find(|r| r.id == id).cloned();
    if let Some(route) = r {
        *app.edit.borrow_mut() = Some(EditRoute { route });
        app.refresh();
    }
}

pub fn add_edit_point(app: &Rc<AppState>, pos: LatLon) {
    if let Some(e) = app.edit.borrow_mut().as_mut() {
        e.route.points.push(RoutePoint::new("", pos));
    }
    app.refresh();
}

pub fn remove_last_edit_point(app: &Rc<AppState>) {
    if let Some(e) = app.edit.borrow_mut().as_mut() {
        e.route.points.pop();
    }
    app.refresh();
}

pub fn finish_edit(app: &Rc<AppState>) {
    let Some(e) = app.edit.borrow_mut().take() else { return };
    if e.route.points.len() < 2 {
        app.toast(&gettext("A route needs at least two points"));
        app.refresh();
        return;
    }
    let res = app.session.borrow_mut().save_route(&e.route);
    match res {
        Ok(r) => {
            app.toast(&gettext_f("Route {name} saved", &[("name", &r.name)]));
            app.selected.set(Selection::Route(r.id));
        }
        Err(err) => db_error(app, err),
    }
    app.refresh();
}

pub fn cancel_edit(app: &Rc<AppState>) {
    if app.edit.borrow_mut().take().is_some() {
        app.refresh();
    }
}

pub fn begin_measure(app: &Rc<AppState>, from: LatLon) {
    *app.edit.borrow_mut() = None;
    *app.measure.borrow_mut() = Some(Measure { from, to: None });
    app.refresh();
}

pub fn set_anchor(app: &Rc<AppState>, pos: LatLon, radius_nm: f64) {
    app.session.borrow_mut().set_anchor(pos, radius_nm);
    app.toast(&gettext("Anchor watch on"));
    app.refresh();
}

pub fn toggle_recording(app: &Rc<AppState>) {
    let now = chrono::Utc::now();
    let recording = app.session.borrow().is_recording();
    if recording {
        app.session.borrow_mut().stop_recording();
        app.toast(&gettext("Track recording stopped"));
    } else {
        let name = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
        match app.session.borrow_mut().start_recording(&name, now) {
            Ok(()) => app.toast(&gettext("Recording track")),
            Err(e) => db_error(app, e),
        }
    }
    app.refresh();
}

pub fn delete_waypoint(app: &Rc<AppState>, id: i64) {
    let r = app.session.borrow_mut().delete_waypoint(id);
    if let Err(e) = r {
        db_error(app, e);
    }
    if app.selected.get() == Selection::Waypoint(id) {
        app.selected.set(Selection::None);
    }
    app.refresh();
}

pub fn delete_route(app: &Rc<AppState>, id: i64) {
    let r = app.session.borrow_mut().delete_route(id);
    if let Err(e) = r {
        db_error(app, e);
    }
    app.refresh();
}

pub fn delete_track(app: &Rc<AppState>, id: i64) {
    let r = app.session.borrow_mut().delete_track(id);
    if let Err(e) = r {
        db_error(app, e);
    }
    app.refresh();
}

pub fn reverse_route(app: &Rc<AppState>, id: i64) {
    let r = app.session.borrow().routes.iter().find(|r| r.id == id).cloned();
    if let Some(mut r) = r {
        r.reverse();
        let res = app.session.borrow_mut().save_route(&r);
        if let Err(e) = res {
            db_error(app, e);
        }
        app.refresh();
    }
}

pub fn activate_route(app: &Rc<AppState>, id: i64) {
    let ok = app.session.borrow_mut().activate_route(id);
    if ok {
        app.session.borrow_mut().update(chrono::Utc::now());
        super::vm::underway::auto(app, true);
        app.refresh();
    } else {
        app.toast(&gettext("That route cannot be activated"));
    }
}

pub fn stop_navigation(app: &Rc<AppState>) {
    app.session.borrow_mut().stop_navigation();
    super::vm::underway::auto(app, false);
    app.refresh();
}

/// Inserts a point into the stored route's nearest segment.
pub fn insert_route_point(app: &Rc<AppState>, route_id: i64, pos: LatLon) {
    let r = app.session.borrow().routes.iter().find(|r| r.id == route_id).cloned();
    let Some(mut r) = r else { return };
    let idx = nearest_segment(&r, &pos);
    r.points.insert(idx + 1, RoutePoint::new("", pos));
    for (i, p) in r.points.iter_mut().enumerate() {
        if i == idx + 1 {
            p.name = String::new();
        }
    }
    let res = app.session.borrow_mut().save_route(&r);
    if let Err(e) = res {
        db_error(app, e);
    }
    app.refresh();
}

/// Index of the leg start nearest to `pos`.
pub fn nearest_segment(r: &Route, pos: &LatLon) -> usize {
    r.points
        .windows(2)
        .enumerate()
        .map(|(i, w)| (i, crate::core::nav::calc::distance_to_segment_nm(&w[0].pos, &w[1].pos, pos)))
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i)
        .unwrap_or(0)
}

pub fn remove_route_point(app: &Rc<AppState>, route_id: i64, idx: usize) {
    let r = app.session.borrow().routes.iter().find(|r| r.id == route_id).cloned();
    let Some(mut r) = r else { return };
    if r.points.len() <= 2 {
        app.toast(&gettext("A route needs at least two points"));
        return;
    }
    r.points.remove(idx);
    let res = app.session.borrow_mut().save_route(&r);
    if let Err(e) = res {
        db_error(app, e);
    }
    app.refresh();
}

/// Turns what the session reports into messages. Alarms are the banner's business (`vm::banner`);
/// here arrivals get a soft tick and a message, and the end of an alarm a note.
pub fn announce(app: &Rc<AppState>, notices: &[Notice]) {
    for n in notices {
        let text = match n {
            Notice::Arrived { waypoint } => {
                app.sound(super::shell::Sound::Tick);
                gettext_f("Arrived at {waypoint}", &[("waypoint", waypoint)])
            }
            Notice::RouteFinished { route } => gettext_f("Route {route} completed", &[("route", route)]),
            Notice::AnchorCleared => gettext("Back within the anchor circle"),
            Notice::SailMode { mode: crate::core::perf::SailMode::Motoring } => gettext("Engine running: motoring"),
            Notice::SailMode { .. } => gettext("Engine stopped: sailing"),
            Notice::AnchorAlarm { .. } | Notice::AisAlarm { .. } | Notice::AisCleared { .. } => continue,
        };
        app.toast(&text);
    }
}

/// Connects clicks, drags and keys on the canvas to the actions above.
pub fn install_interaction(app: &Rc<AppState>) {
    let canvas = &app.canvas;
    {
        let app = Rc::downgrade(app);
        canvas.connect_pointer_moved(move |p| {
            if let Some(app) = app.upgrade() {
                app.pointer.set(p);
                let at = p.and_then(|p| app.canvas.screen_of(&p));
                super::probe::pointer_moved(&app, at);
                // The rubber band and measuring line follow the pointer.
                if app.edit.borrow().is_some() || app.measure.borrow().as_ref().is_some_and(|m| m.to.is_none()) || app.router.borrow().is_some() {
                    app.canvas.queue_draw();
                }
            }
        });
    }
    {
        let app = Rc::downgrade(app);
        canvas.connect_view_changed(move |_| {
            if let Some(app) = app.upgrade() {
                super::probe::view_changed(&app);
            }
        });
    }
    {
        let app = Rc::downgrade(app);
        canvas.connect_click(move |c| {
            if let Some(app) = app.upgrade() {
                on_click(&app, c);
            }
        });
    }
    {
        let app = Rc::downgrade(app);
        canvas.connect_drag(move |phase, pos| match app.upgrade() {
            Some(app) => on_drag(&app, phase, pos),
            None => false,
        });
    }
    {
        let app = Rc::downgrade(app);
        canvas.connect_key(move |key| match app.upgrade() {
            Some(app) => on_key(&app, key),
            None => false,
        });
    }
}

/// Pixels within which a click counts as on an object: half the smallest hit size.
pub fn hit_tolerance(app: &AppState) -> f64 {
    app.canvas.hit_min() / 2.0
}

pub fn hit_at(app: &AppState, x: f64, y: f64) -> Hit {
    let s = app.session.borrow();
    let e = app.edit.borrow();
    hit_test(&s, e.as_ref().map(|e| &e.route), &app.canvas.view_info(), x, y, hit_tolerance(app))
}

fn on_click(app: &Rc<AppState>, c: Click) {
    match c.button {
        3 => context::show(app, c),
        1 => primary_click(app, c),
        _ => {}
    }
}

fn primary_click(app: &Rc<AppState>, c: Click) {
    // A click on the front of a weather-route preview adds an alternative.
    if c.n_press == 1 && super::router::click_front(app, c.x, c.y) {
        return;
    }
    if app.edit.borrow().is_some() {
        if c.n_press >= 2 {
            finish_edit(app);
        } else if !matches!(hit_at(app, c.x, c.y), Hit::EditPoint(_)) {
            add_edit_point(app, c.pos);
        }
        return;
    }
    {
        let mut m = app.measure.borrow_mut();
        if let Some(meas) = m.as_mut() {
            // With a pointer the line follows it to the click that ends it, and the next click clears it. A
            // finger has no pointer: each tap moves the end, until the shell's ruler button ends it.
            if meas.to.is_none() || app.canvas.is_touch() {
                meas.to = Some(c.pos);
            } else {
                *m = None;
            }
            drop(m);
            app.refresh();
            return;
        }
    }
    let hit = hit_at(app, c.x, c.y);
    let sel = match hit {
        Hit::Waypoint(id) => Selection::Waypoint(id),
        Hit::RoutePoint { route, idx } => Selection::RoutePoint { route, idx },
        Hit::Route(id) => Selection::Route(id),
        Hit::Target(m) => Selection::Target(m),
        Hit::EditPoint(_) | Hit::None => Selection::None,
    };
    if hit == Hit::None && c.n_press >= 2 {
        let v = app.canvas.view_state();
        app.canvas.zoom_to(v.zoom + 1.0, c.x, c.y);
        return;
    }
    // Buoys and lights carry no labels on the chart, so a click on one tells what it is; then the chart's other
    // features: dangers, cables, restricted areas. The pointer's outline showed which one this opens.
    if hit == Hit::None && c.n_press == 1 {
        let found = super::probe::find(app, c.x, c.y, hit_tolerance(app));
        if !found.is_empty() {
            app.select(sel);
            super::probe::open(app, found, c.x, c.y);
            return;
        }
    }
    app.select(sel);
}

fn on_drag(app: &Rc<AppState>, phase: DragPhase, pos: LatLon) -> bool {
    match phase {
        DragPhase::Begin => {
            let Some((x, y)) = app.canvas.screen_of(&pos) else { return false };
            let item = match hit_at(app, x, y) {
                Hit::EditPoint(i) => Some(DragItem::EditPoint(i)),
                Hit::Waypoint(id) => Some(DragItem::Waypoint(id)),
                Hit::RoutePoint { route, idx } => Some(DragItem::RoutePoint { route, idx }),
                _ => None,
            };
            app.drag.set(item);
            item.is_some()
        }
        DragPhase::Update | DragPhase::End => {
            let Some(item) = app.drag.get() else { return false };
            move_item(app, item, pos);
            if phase == DragPhase::End {
                app.drag.set(None);
                persist_item(app, item);
                app.refresh();
            } else {
                app.canvas.queue_draw();
            }
            true
        }
    }
}

fn move_item(app: &AppState, item: DragItem, pos: LatLon) {
    match item {
        DragItem::EditPoint(i) => {
            if let Some(p) = app.edit.borrow_mut().as_mut().and_then(|e| e.route.points.get_mut(i)) {
                p.pos = pos;
            }
        }
        DragItem::Waypoint(id) => {
            if let Some(w) = app.session.borrow_mut().waypoints.iter_mut().find(|w| w.id == id) {
                w.pos = pos;
            }
        }
        DragItem::RoutePoint { route, idx } => {
            if let Some(p) = app.session.borrow_mut().routes.iter_mut().find(|r| r.id == route).and_then(|r| r.points.get_mut(idx)) {
                p.pos = pos;
            }
        }
    }
}

fn persist_item(app: &AppState, item: DragItem) {
    let mut s = app.session.borrow_mut();
    let res = match item {
        DragItem::EditPoint(_) => Ok(()),
        DragItem::Waypoint(id) => match s.waypoints.iter().find(|w| w.id == id).cloned() {
            Some(w) => s.update_waypoint(&w),
            None => Ok(()),
        },
        DragItem::RoutePoint { route, .. } => match s.routes.iter().find(|r| r.id == route).cloned() {
            Some(r) => s.save_route(&r).map(|_| ()),
            None => Ok(()),
        },
    };
    drop(s);
    if let Err(e) = res {
        db_error(app, e);
    }
}

fn on_key(app: &Rc<AppState>, key: Key) -> bool {
    match key {
        Key::Escape => {
            let had = app.edit.borrow().is_some() || app.measure.borrow().is_some() || app.selected.get() != Selection::None;
            *app.measure.borrow_mut() = None;
            if app.edit.borrow().is_some() {
                cancel_edit(app);
            }
            app.select(Selection::None);
            had
        }
        Key::Enter if app.edit.borrow().is_some() => {
            finish_edit(app);
            true
        }
        Key::Backspace if app.edit.borrow().is_some() => {
            remove_last_edit_point(app);
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(points: &[(f64, f64)]) -> Route {
        let mut r = Route::new("r");
        for (lat, lon) in points {
            r.points.push(RoutePoint::new("", LatLon::new(*lat, *lon)));
        }
        r
    }

    #[test]
    fn the_nearest_segment_is_found_for_insertion() {
        let r = route(&[(48.0, -4.0), (48.0, -3.0), (49.0, -3.0)]);
        assert_eq!(nearest_segment(&r, &LatLon::new(48.01, -3.5)), 0);
        assert_eq!(nearest_segment(&r, &LatLon::new(48.5, -2.99)), 1);
        assert_eq!(nearest_segment(&route(&[]), &LatLon::new(0.0, 0.0)), 0);
    }
}
