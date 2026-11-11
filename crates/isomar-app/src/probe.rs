//! What the pointer is over: the chart feature, mark or anchorage a click would open a popover for, and its
//! outline. The same lookup serves both, so what lights up under the pointer is what a click opens (the
//! pointer's outline, the cursor), and what stays lit while the popover is open (the pinned outline). Several
//! features can lie under one spot (a caution area inside a restricted area); the popover lets the user step
//! through them, and the outline follows.

use super::actions;
use super::aidinfo::{self, AidInfo, Description};
use super::app::AppState;
use super::hit::Hit;
use super::layers::chartinfo::{self, Marker};
use super::platform;
use crate::core::chart::s57::{Feature, Geom};
use crate::core::geo::LatLon;
use crate::core::vector as core_vector;
use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;
use std::time::Duration;

pub type Ring = Vec<[f64; 2]>;

/// The most features offered for one spot.
const MAX_CHOICES: usize = 8;

/// How long the pointer must rest before the lookup runs: it follows a moving pointer at most this often.
const SETTLE: Duration = Duration::from_millis(40);

/// How often the lookup is tried again while the chart is being dragged.
const DRAG_RETRY: Duration = Duration::from_millis(120);

/// The shape of what is under the pointer, as `[lon, lat]`.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Point([f64; 2]),
    Line(Ring),
    Area(Vec<Ring>),
}

/// What to draw to show a feature: its shape, and for an area whether a click anywhere inside opens it (a
/// fairway's inside says nothing: only its edge does) and the marker it is charted with.
#[derive(Clone, Debug)]
pub struct Outline {
    pub shape: Shape,
    pub inside: bool,
    pub marker: Option<Marker>,
    centre: OnceCell<Option<[f64; 2]>>,
}

impl PartialEq for Outline {
    fn eq(&self, other: &Outline) -> bool {
        self.shape == other.shape && self.inside == other.inside && self.marker == other.marker
    }
}

impl Outline {
    fn new(shape: Shape, inside: bool, marker: Option<Marker>) -> Outline {
        Outline { shape, inside, marker, centre: OnceCell::new() }
    }

    pub fn of_feature(f: &Feature) -> Option<Outline> {
        Some(match &f.geom {
            Geom::Point(p) => Outline::new(Shape::Point(*p), false, None),
            Geom::Line(l) => Outline::new(Shape::Line(l.clone()), false, None),
            Geom::Area(rings) => Outline::new(Shape::Area(rings.clone()), super::featureinfo::clickable_inside(f.objl), chartinfo::area_marker(f)),
            _ => return None,
        })
    }

    pub fn of_mark(info: &AidInfo) -> Outline {
        match &info.anchorage {
            Some(a) if !a.rings.is_empty() => Outline::new(Shape::Area(a.rings.clone()), true, None),
            _ => Outline::new(Shape::Point([info.pos.lon, info.pos.lat]), false, None),
        }
    }

    /// Where an area's marking goes: a point well inside it, found once.
    pub fn centre(&self) -> Option<[f64; 2]> {
        *self.centre.get_or_init(|| match &self.shape {
            Shape::Area(rings) => core_vector::label_point(rings),
            _ => None,
        })
    }
}

/// One thing under the pointer, told in words and drawn as an outline.
#[derive(Clone, Debug)]
pub struct FeatureHit {
    pub description: Description,
    pub outline: Outline,
}

impl FeatureHit {
    pub fn of_feature(f: &Feature, description: Description) -> Option<FeatureHit> {
        Some(FeatureHit { description, outline: Outline::of_feature(f)? })
    }
}

/// The marks, anchorages and chart features at (`x`, `y`), the one a click opens first leading: a mark beats an
/// anchorage, which beats the chart's other features.
pub fn find(app: &AppState, x: f64, y: f64, tol: f64) -> Vec<FeatureHit> {
    if let Some(info) = aidinfo::at(app, x, y, tol) {
        return vec![FeatureHit { description: aidinfo::describe(&info), outline: Outline::of_mark(&info) }];
    }
    let vi = app.canvas.view_info();
    let mut hits = app.enc.borrow().as_ref().map(|l| l.features_at(&vi, x, y, tol)).unwrap_or_default();
    hits.truncate(MAX_CHOICES);
    hits
}

/// What a click at (`x`, `y`) would open, or nothing where a click does something else: while a route is
/// edited, a distance measured or a weather route previewed, or over a route, waypoint or vessel.
pub fn under_pointer(app: &AppState, x: f64, y: f64) -> Vec<FeatureHit> {
    if app.edit.borrow().is_some() || app.measure.borrow().is_some() || app.router.borrow().is_some() || actions::hit_at(app, x, y) != Hit::None {
        return Vec::new();
    }
    find(app, x, y, actions::hit_tolerance(app))
}

/// The features a popover is offering, and which one it shows.
struct Choices {
    items: Vec<FeatureHit>,
    index: usize,
    x: f64,
    y: f64,
}

/// The state of the pointer's outline: under the pointer, and kept while a popover is open.
#[derive(Default)]
pub struct Probe {
    hover: RefCell<Option<Outline>>,
    pinned: RefCell<Option<Outline>>,
    choices: RefCell<Option<Choices>>,
    /// The pointer on the chart, in pixels.
    at: Cell<Option<(f64, f64)>>,
    timer: Cell<bool>,
    pointing: Cell<bool>,
}

impl Probe {
    /// The outline of what a click would open at the pointer.
    pub fn hover(&self) -> Option<Outline> {
        self.hover.borrow().clone()
    }

    /// The outline of what the open popover is about.
    pub fn pinned(&self) -> Option<Outline> {
        self.pinned.borrow().clone()
    }
}

/// The pointer moved to `at` (pixels), or left the chart. The lookup runs once it has rested for a moment.
pub fn pointer_moved(app: &Rc<AppState>, at: Option<(f64, f64)>) {
    app.probe.at.set(at);
    if at.is_none() {
        set_hover(app, None);
        return;
    }
    schedule(app, SETTLE);
}

/// The view moved under the pointer (a zoom, a key): what is there may be something else now.
pub fn view_changed(app: &Rc<AppState>) {
    if app.probe.at.get().is_some() {
        schedule(app, SETTLE);
    }
}

fn schedule(app: &Rc<AppState>, delay: Duration) {
    if !platform::installed() {
        refresh(app);
        return;
    }
    if app.probe.timer.replace(true) {
        return;
    }
    let app = Rc::downgrade(app);
    platform::timeout_once(delay, move || {
        if let Some(app) = app.upgrade() {
            app.probe.timer.set(false);
            refresh(&app);
        }
    });
}

/// Looks again at what is under the pointer.
pub fn refresh(app: &Rc<AppState>) {
    let Some((x, y)) = app.probe.at.get() else {
        set_hover(app, None);
        return;
    };
    // While the chart is dragged what is under the pointer is changing every frame: wait for it to stop.
    if app.canvas.dragging() {
        set_hover(app, None);
        schedule(app, DRAG_RETRY);
        return;
    }
    let outline = under_pointer(app, x, y).into_iter().next().map(|h| h.outline);
    set_hover(app, outline);
}

fn set_hover(app: &Rc<AppState>, outline: Option<Outline>) {
    let probe = &app.probe;
    let pointing = outline.is_some();
    if probe.pointing.replace(pointing) != pointing
        && let Some(shell) = app.shell()
    {
        shell.set_cursor(pointing);
    }
    if *probe.hover.borrow() != outline {
        *probe.hover.borrow_mut() = outline;
        app.canvas.queue_draw();
    }
}

/// A click at (`x`, `y`) opens `hits`: the first is shown, and the outline of the one shown stays on the chart
/// until the popover is closed.
pub fn open(app: &Rc<AppState>, hits: Vec<FeatureHit>, x: f64, y: f64) {
    if hits.is_empty() {
        return;
    }
    *app.probe.choices.borrow_mut() = Some(Choices { items: hits, index: 0, x, y });
    present(app);
}

/// Shows the popover for the chosen feature and pins its outline.
fn present(app: &Rc<AppState>) {
    let (description, outline, x, y) = {
        let choices = app.probe.choices.borrow();
        let Some(c) = choices.as_ref() else { return };
        let hit = &c.items[c.index];
        let mut description = hit.description.clone();
        if c.items.len() > 1 {
            description.choice = Some((c.index + 1, c.items.len()));
        }
        (description, hit.outline.clone(), c.x, c.y)
    };
    *app.probe.pinned.borrow_mut() = Some(outline);
    set_hover(app, None);
    app.canvas.queue_draw();
    if let Some(shell) = app.shell() {
        shell.object_info(&description, x, y);
    }
}

/// Shows the next (`delta` 1) or the previous (-1) of the features under the click.
pub fn cycle(app: &Rc<AppState>, delta: i32) {
    {
        let mut choices = app.probe.choices.borrow_mut();
        let Some(c) = choices.as_mut() else { return };
        let n = c.items.len() as i32;
        c.index = (c.index as i32 + delta).rem_euclid(n) as usize;
    }
    present(app);
}

/// The popover was closed.
pub fn info_closed(app: &Rc<AppState>) {
    *app.probe.choices.borrow_mut() = None;
    if app.probe.pinned.borrow_mut().take().is_some() {
        app.canvas.queue_draw();
    }
    if app.probe.at.get().is_some() {
        schedule(app, SETTLE);
    }
}

/// The point on the chart where an area's outline is marked, for a test or a shell that wants it.
pub fn centre_of(outline: &Outline) -> Option<LatLon> {
    outline.centre().map(|c| LatLon::new(c[1], c[0]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::chart::s57::{Cell as S57Cell, attr, obj};
    use crate::core::nav::store::NavStore;
    use crate::core::session::Session;
    use crate::core::vector::Bounds;
    use crate::engine::Engine;
    use crate::layers::enc::EncLayer;
    use crate::palette::Mode;
    use crate::shell::{Dialog, Menu, Shell};
    use std::collections::BTreeMap;
    use std::future::Future;
    use std::pin::Pin;

    /// A shell that remembers what the engine asked of it.
    #[derive(Default)]
    struct Spy {
        shown: RefCell<Vec<Description>>,
        cursor: Cell<bool>,
    }

    impl Shell for Spy {
        fn toast(&self, _: &str) {}
        fn show_error(&self, _: &str, _: &str) {}
        fn sound(&self, _: crate::shell::Sound) {}
        fn confirm(&self, _: &str, _: &str, _: &str, _: bool) -> Pin<Box<dyn Future<Output = bool>>> {
            Box::pin(async { false })
        }
        fn context_menu(&self, _: Menu, _: f64, _: f64) {}
        fn object_info(&self, info: &Description, _: f64, _: f64) {
            self.shown.borrow_mut().push(info.clone());
        }
        fn set_cursor(&self, pointing: bool) {
            self.cursor.set(pointing);
        }
        fn open(&self, _: Dialog) {}
    }

    fn square(c: [f64; 2], d: f64) -> Vec<Ring> {
        vec![vec![[c[0] - d, c[1] - d], [c[0] + d, c[1] - d], [c[0] + d, c[1] + d], [c[0] - d, c[1] + d], [c[0] - d, c[1] - d]]]
    }

    fn feature(rcid: u32, objl: u16, attrs: &[(u16, &str)], geom: Geom) -> Feature {
        Feature { rcid, objl, attrs: attrs.iter().map(|(k, v)| (*k, v.to_string())).collect::<BTreeMap<_, _>>(), geom }
    }

    /// An app on a cell with a caution area inside a big restricted area and a fairway round both, and the
    /// spy shell.
    fn app() -> (Rc<AppState>, Rc<Spy>) {
        let canvas = Engine::new(crate::config::ViewState { lat: 48.0, lon: -4.0, zoom: 14.0 }, Mode::Day);
        canvas.resize(800.0, 600.0);
        let app = AppState::new(canvas, Session::new(NavStore::open_memory().unwrap()).unwrap(), crate::config::Settings::default());
        let c = [-4.0, 48.0];
        let features = vec![
            feature(1, obj::RESARE, &[(attr::RESTRN, "3")], Geom::Area(square(c, 0.02))),
            feature(2, obj::CTNARE, &[], Geom::Area(square(c, 0.004))),
            feature(3, obj::FAIRWY, &[], Geom::Area(square(c, 0.03))),
        ];
        let layer = EncLayer::new(Rc::new(|| {}));
        let mut bounds = Bounds::EMPTY;
        bounds.extend([-4.05, 47.95]);
        bounds.extend([-3.95, 48.05]);
        layer.add_cell(S57Cell {
            name: "TEST".into(),
            edition: 1,
            update: 0,
            issue_date: String::new(),
            cscl: 25_000,
            features,
            bounds,
            warnings: Vec::new(),
            skipped_updates: Vec::new(),
        });
        *app.enc.borrow_mut() = Some(layer);
        let spy = Rc::new(Spy::default());
        app.set_shell(spy.clone());
        (app, spy)
    }

    #[test]
    fn the_outline_under_the_pointer_is_the_area_a_click_opens_and_it_is_cleared_off_it() {
        let (app, spy) = app();
        pointer_moved(&app, Some((400.0, 300.0)));
        let h = app.probe.hover().expect("the caution area under the pointer");
        assert!(matches!(h.shape, Shape::Area(_)) && h.inside);
        assert_eq!(h.marker, Some(Marker::Caution), "its own marking is what is drawn at its centre");
        let c = centre_of(&h).unwrap();
        assert!((c.lat - 48.0).abs() < 0.001 && (c.lon + 4.0).abs() < 0.001, "{c:?}");
        assert!(spy.cursor.get(), "the pointer is a hand over it");
        // Inside the restricted area, outside the caution area in it.
        let (x, y) = app.canvas.view_info().to_screen(&LatLon::new(48.0, -3.99));
        pointer_moved(&app, Some((x, y)));
        let h = app.probe.hover().expect("the restricted area");
        assert_eq!(h.marker, Some(Marker::Prohibited));
        pointer_moved(&app, Some((5.0, 5.0)));
        assert!(app.probe.hover().is_none() && !spy.cursor.get());
        pointer_moved(&app, None);
        assert!(app.probe.hover().is_none());
    }

    #[test]
    fn a_fairway_lights_up_only_at_its_edge() {
        let (app, _) = app();
        // The restricted area reaches 0.02 degrees from the middle and the fairway 0.03.
        let (x, y) = app.canvas.view_info().to_screen(&LatLon::new(48.0, -3.99));
        pointer_moved(&app, Some((x, y)));
        assert!(app.probe.hover().is_some_and(|h| h.marker == Some(Marker::Prohibited)), "inside the restricted area");
        let (x, y) = app.canvas.view_info().to_screen(&LatLon::new(48.0, -3.975));
        pointer_moved(&app, Some((x, y)));
        assert!(app.probe.hover().is_none(), "inside the fairway, outside the rest");
        let (x, y) = app.canvas.view_info().to_screen(&LatLon::new(48.0, -3.97));
        pointer_moved(&app, Some((x, y)));
        let h = app.probe.hover().expect("the fairway's edge");
        assert!(!h.inside);
    }

    #[test]
    fn a_click_pins_the_outline_and_steps_through_what_lies_under_it() {
        let (app, spy) = app();
        let hits = find(&app, 400.0, 300.0, 6.0);
        assert_eq!(hits.len(), 2, "the caution area inside the restricted area");
        open(&app, hits, 400.0, 300.0);
        let first = spy.shown.borrow().last().cloned().unwrap();
        assert_eq!(first.subtitle, None);
        assert_eq!(first.choice, Some((1, 2)));
        let pinned = app.probe.pinned().unwrap();
        assert_eq!(pinned.marker, Some(Marker::Caution), "the smallest first");
        cycle(&app, 1);
        let second = spy.shown.borrow().last().cloned().unwrap();
        assert_eq!(second.choice, Some((2, 2)));
        assert_ne!(second.title, first.title);
        assert_eq!(app.probe.pinned().unwrap().marker, Some(Marker::Prohibited));
        cycle(&app, 1);
        assert_eq!(spy.shown.borrow().last().unwrap().choice, Some((1, 2)), "round again");
        cycle(&app, -1);
        assert_eq!(spy.shown.borrow().last().unwrap().choice, Some((2, 2)));
        // While it is open the pointer's own outline gives way to the pinned one.
        assert!(app.probe.hover().is_none());
        info_closed(&app);
        assert!(app.probe.pinned().is_none());
        cycle(&app, 1);
        assert_eq!(spy.shown.borrow().len(), 4, "nothing is offered once it is closed");
    }

    #[test]
    fn a_single_feature_offers_no_choice() {
        let (app, spy) = app();
        let (x, y) = app.canvas.view_info().to_screen(&LatLon::new(48.0, -3.99));
        let hits = find(&app, x, y, 6.0);
        assert_eq!(hits.len(), 1);
        open(&app, hits, x, y);
        assert_eq!(spy.shown.borrow()[0].choice, None);
    }
}
