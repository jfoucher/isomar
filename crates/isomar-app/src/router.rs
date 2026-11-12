//! The model of a weather-route preview: the search, the alternatives picked
//! from it, the playback clock, and what a click on the chart means.

use super::app::AppState;
use crate::chips::{Chip, Icon};
use super::hit::seg_dist;
use super::layers::router::ALT_COLOURS;
use crate::core::geo::LatLon;
use crate::core::nav::model::Route;
use crate::core::routing::alternatives::{self, Alternative};
use crate::core::routing::land::DetailedLand;
use crate::core::weather::store::WeatherStore;
use std::sync::Arc;
use crate::core::routing::isochrone::Exploration;
use crate::core::vector::Bounds;
use crate::i18n::{gettext, gettext_f};
use chrono::{DateTime, Duration, Local, Utc};
use std::rc::Rc;

/// How long, at 1x, the whole search takes to play, in 40 ms ticks.
pub const PLAY_TICKS: f64 = 375.0;
/// How close, in pixels, a click must be to a front or a track to mean it.
pub const PICK_TOLERANCE: f64 = 14.0;

/// An alternative the user picked, and the colour it keeps while it exists.
pub struct Choice {
    pub alt: Alternative,
    /// Index into `ALT_COLOURS`.
    pub colour: usize,
}

/// What `RouterPreview::add` did.
#[derive(Debug, PartialEq, Eq)]
pub enum Added {
    /// A new alternative, at this index.
    New(usize),
    /// That position was already an alternative.
    Existing(usize),
    /// Every colour is taken.
    Full,
}

/// A saved route's plan being played back, not a search: the forecast it was
/// routed on stands in for the one loaded until the preview closes.
pub struct Replay {
    pub route_id: i64,
    /// The forecast that was loaded, to put back.
    pub store: WeatherStore,
    /// The route's points are not the ones the plan was made for.
    pub edited: bool,
    /// The model run the plan's forecast came from.
    pub run: Option<DateTime<Utc>>,
    /// The model it came from, for plans that know.
    pub model: Option<crate::core::weather::source::Source>,
}

/// The search of one forecast model, when the passage was routed on several.
pub struct ModelRun {
    pub model: crate::core::weather::source::Source,
    /// Its search; `None` while it is the one shown (`RouterPreview::ex`).
    pub ex: Option<Exploration>,
    /// Its best way: to the destination, or as near as its forecast reached.
    pub best: Alternative,
}

/// What a passage routed on several models needs besides the searches: to sail the routes through
/// the other forecasts (the cross-check, and the arrival times of a route once it is saved).
pub struct Comparison {
    pub polar: Arc<crate::core::routing::polar::Polar>,
    pub settings: crate::core::routing::isochrone::Settings,
    /// The area the searches covered; their forecasts are read inside it.
    pub area: Bounds,
    /// Each model's best way sailed through each model's forecast, in `runs` order both ways.
    pub cross: crate::core::routing::compare::CrossCheck,
    /// The best ways go the same way (`compare::agree`).
    pub agree: bool,
}

/// A search waiting for the user to pick from it.
pub struct RouterPreview {
    /// Tells the panel when a different search arrived.
    pub id: u64,
    /// Changes when alternatives are added or removed.
    pub version: u64,
    pub source: Route,
    pub ex: Exploration,
    pub alts: Vec<Choice>,
    pub selected: usize,
    /// The playback clock.
    pub t: DateTime<Utc>,
    /// The last moment to play: the arrival, or where the forecast ends.
    pub end: DateTime<Utc>,
    /// The weather overlay's time to put back when the preview closes.
    pub weather_time: DateTime<Utc>,
    pub replay: Option<Replay>,
    /// The land the search kept clear of, so a route saved from it can too.
    pub land: Option<Arc<DetailedLand>>,
    /// The forecast model it sailed on (or, played back, was planned on).
    pub model: crate::core::weather::source::Source,
    /// Every model's search, in the order asked for, when there were several; empty otherwise.
    pub runs: Vec<ModelRun>,
    pub comparison: Option<Comparison>,
    /// The current it sailed in, in words ("Copernicus NW Shelf"), when it had one.
    pub currents: Option<String>,
}

impl RouterPreview {
    /// The whole search is shown at first, with the route that arrived if
    /// there was one, else the one that got nearest the destination; play
    /// starts it over.
    pub fn new(id: u64, source: Route, ex: Exploration, weather_time: DateTime<Utc>) -> RouterPreview {
        let end = ex.arrived.as_ref().map(|r| r.arrival).or(ex.horizon).unwrap_or(ex.depart);
        let alts = alternatives::best(&ex).into_iter().map(|alt| Choice { alt, colour: 0 }).collect();
        RouterPreview { id, version: 0, source, ex, alts, selected: 0, t: end, end, weather_time, replay: None, land: None, model: Default::default(), runs: Vec::new(), comparison: None, currents: None }
    }

    /// The index in `runs` of the model shown.
    pub fn focused(&self) -> Option<usize> {
        self.runs.iter().position(|r| r.ex.is_none())
    }

    /// Shows model `i`'s search in place of the one shown: its fronts, its alternatives (its best way,
    /// in its colour), its wind. Alternatives picked on the other are dropped.
    pub fn focus(&mut self, i: usize) -> bool {
        let Some(cur) = self.focused() else { return false };
        if i == cur || i >= self.runs.len() {
            return false;
        }
        let Some(ex) = self.runs[i].ex.take() else { return false };
        let old = std::mem::replace(&mut self.ex, ex);
        self.runs[cur].ex = Some(old);
        self.model = self.runs[i].model;
        self.alts = alternatives::best(&self.ex).into_iter().map(|alt| Choice { alt, colour: i % ALT_COLOURS.len() }).collect();
        self.selected = 0;
        self.version += 1;
        true
    }

    /// How many fronts the clock has reached.
    pub fn fronts_shown(&self) -> usize {
        let step_ms = self.ex.step.num_milliseconds().max(1);
        (((self.t - self.ex.depart - self.ex.lead).num_milliseconds() / step_ms).max(0) as usize).min(self.ex.fronts.len())
    }

    /// The front at the clock: the one alternatives are picked from.
    pub fn current_front(&self) -> Option<&[usize]> {
        self.ex.fronts.get(self.fronts_shown().checked_sub(1)?).map(|f| f.as_slice())
    }

    fn free_colour(&self) -> Option<usize> {
        // The other models' colours stay theirs.
        let theirs = |c: usize| self.runs.iter().enumerate().any(|(k, r)| r.ex.is_some() && k % ALT_COLOURS.len() == c);
        (0..ALT_COLOURS.len()).find(|c| !theirs(*c) && self.alts.iter().all(|a| a.colour != *c))
    }

    /// Adds the alternative that ends at position `node` of the tree, and selects it.
    pub fn add(&mut self, node: usize) -> Added {
        let pos = self.ex.tree[node].node.pos;
        let time = self.ex.tree[node].node.time;
        if let Some(i) = self.alts.iter().position(|c| c.alt.end == time && c.alt.path.last().is_some_and(|n| n.pos == pos)) {
            self.selected = i;
            self.version += 1;
            return Added::Existing(i);
        }
        let Some(colour) = self.free_colour() else { return Added::Full };
        self.alts.push(Choice { alt: alternatives::to_node(&self.ex, node), colour });
        self.selected = self.alts.len() - 1;
        self.version += 1;
        Added::New(self.selected)
    }

    pub fn remove(&mut self, i: usize) {
        if i < self.alts.len() {
            self.alts.remove(i);
            self.selected = self.selected.min(self.alts.len().saturating_sub(1));
            self.version += 1;
        }
    }

    /// The position on the current front nearest to (`x`, `y`) on screen, if
    /// within `tol` pixels of the front's line, as an index into the tree.
    pub fn front_hit(&self, project: &impl Fn(&LatLon) -> (f64, f64), x: f64, y: f64, tol: f64) -> Option<usize> {
        let k = self.fronts_shown().checked_sub(1)?;
        let front = self.current_front()?;
        let pt = |i: usize| project(&self.ex.tree[i].node.pos);
        let p = (x, y);
        let near = |i: usize| {
            let q = pt(i);
            ((q.0 - x).powi(2) + (q.1 - y).powi(2)).sqrt()
        };
        // Only the drawn line counts, and a position stands alone when nothing joins it.
        let line = front.windows(2).zip(&self.ex.joins[k]).filter(|(_, joined)| **joined).map(|(w, _)| seg_dist(p, pt(w[0]), pt(w[1])));
        let d = line.chain(front.iter().map(|&i| near(i))).fold(f64::INFINITY, f64::min);
        if d > tol {
            return None;
        }
        front.iter().copied().min_by(|&a, &b| near(a).total_cmp(&near(b)))
    }

    /// The alternatives whose track (as far as the clock has played it) or boat
    /// is within `tol` pixels of (`x`, `y`), nearest first.
    pub fn alts_hit(&self, project: &impl Fn(&LatLon) -> (f64, f64), x: f64, y: f64, tol: f64) -> Vec<usize> {
        let p = (x, y);
        let mut hits: Vec<(f64, usize)> = self
            .alts
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                let track: Vec<(f64, f64)> = c.alt.track_until(self.t).iter().map(project).collect();
                let boat = project(&c.alt.position_at(self.t));
                let d = track.windows(2).map(|w| seg_dist(p, w[0], w[1])).fold(seg_dist(p, boat, boat), f64::min);
                (d <= tol).then_some((d, i))
            })
            .collect();
        hits.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        hits.into_iter().map(|(_, i)| i).collect()
    }
}

pub fn local(t: DateTime<Utc>) -> String {
    t.with_timezone(&Local).format("%a %d %H:%M").to_string()
}

/// What an alternative is worth: how far, when it arrives or how far is left, the wind, the engine.
pub fn summary(a: &Alternative) -> Vec<Chip> {
    let nm = format!("{:.0}", a.distance_nm);
    let mut out = if a.arrives {
        let eta = local(a.end);
        vec![Chip::with(Icon::Distance, gettext_f("{nm} nm", &[("nm", &nm)])), Chip::new(Icon::Arrival, eta.clone(), gettext_f("arrives {eta}", &[("eta", &eta)]))]
    } else {
        let time = local(a.end);
        vec![
            Chip::with(Icon::Distance, gettext_f("{nm} nm sailed, {left} nm to go", &[("left", &format!("{:.0}", a.remaining_nm)), ("nm", &nm)])),
            Chip::new(Icon::Time, time.clone(), gettext_f("forecast ends {time}", &[("time", &time)])),
        ]
    };
    if a.max_tws_kn > 0.0 {
        let kn = format!("{:.0}", a.max_tws_kn);
        out.push(Chip::new(Icon::Wind, gettext_f("up to {kn} kn", &[("kn", &kn)]), gettext_f("wind up to {kn} kn", &[("kn", &kn)])));
    }
    if a.motor_hours > 0.1 {
        let h = format!("{:.0}", a.motor_hours);
        out.push(Chip::new(Icon::Engine, gettext_f("{h} h", &[("h", &h)]), gettext_f("{h} h under engine", &[("h", &h)])));
    }
    out
}

/// The name an alternative goes by in lists and menus.
pub fn alt_title(i: usize, a: &Alternative) -> String {
    if a.arrives {
        gettext_f("{n}. Route to the destination", &[("n", &(i + 1).to_string())])
    } else {
        gettext_f("{n}. Route to {position}", &[("n", &(i + 1).to_string()), ("position", &a.path.last().map_or_else(String::new, |n| super::text::position_text(&n.pos)))])
    }
}

/// Puts the weather overlay at the clock, as far as the forecast reaches.
pub fn follow_weather(app: &AppState, t: DateTime<Utc>) {
    let mut w = app.weather.borrow_mut();
    if let Some((lo, hi)) = w.store.time_range() {
        w.time = t.clamp(lo, hi);
    }
}

/// What to bring into view: the start, where the search got to (the
/// destination too if it was reached), with room below for the card.
pub fn preview_bounds(ex: &Exploration) -> Bounds {
    let mut b = Bounds::EMPTY;
    b.extend([ex.start.lon, ex.start.lat]);
    if ex.arrived.is_some() {
        b.extend([ex.dest.lon, ex.dest.lat]);
    }
    for &i in ex.fronts.last().into_iter().flatten() {
        let p = ex.tree[i].node.pos;
        b.extend([p.lon, p.lat]);
    }
    let pad = 0.05 * b.width().max(b.height()).max(0.2);
    let mut b = b.expanded(pad);
    // The card covers the lower part of the chart, so extend the area south
    // and the search sits above it.
    b.min_lat -= 0.7 * b.height();
    b
}

/// Ends the preview and puts the weather overlay back.
pub fn close(app: &Rc<AppState>) {
    let p = app.router.borrow_mut().take();
    if let Some(p) = p {
        let mut w = app.weather.borrow_mut();
        w.time = p.weather_time;
        if let Some(r) = p.replay {
            w.store = r.store;
        }
    }
    app.refresh();
}

/// A click at (`x`, `y`) while previewing: on the front at the clock it adds
/// an alternative ending there. Returns whether the click was used.
pub fn click_front(app: &Rc<AppState>, x: f64, y: f64) -> bool {
    let vi = app.canvas.view_info();
    let project = |p: &LatLon| vi.to_screen(p);
    let added = {
        let mut r = app.router.borrow_mut();
        let Some(p) = r.as_mut() else { return false };
        let Some(node) = p.front_hit(&project, x, y, PICK_TOLERANCE) else { return false };
        p.add(node)
    };
    match added {
        Added::New(_) | Added::Existing(_) => app.refresh(),
        Added::Full => app.toast(&gettext("Every colour is in use. Remove an alternative to add another.")),
    }
    true
}

/// The alternatives under (`x`, `y`), nearest first, with their titles.
pub fn alternatives_at(app: &AppState, x: f64, y: f64) -> Vec<(usize, String)> {
    let vi = app.canvas.view_info();
    let r = app.router.borrow();
    let Some(p) = r.as_ref() else { return Vec::new() };
    p.alts_hit(&|q: &LatLon| vi.to_screen(q), x, y, PICK_TOLERANCE).into_iter().map(|i| (i, alt_title(i, &p.alts[i].alt))).collect()
}

/// Saves alternative `i` as a route and ends the preview.
pub fn use_choice(app: &Rc<AppState>, i: usize) {
    let chosen = app.router.borrow().as_ref().filter(|p| p.replay.is_none()).and_then(|p| {
        // Routed on several models: the saved route is sailed through each of their forecasts.
        let check = p.comparison.as_ref().map(|c| super::flows::routing::CheckWith { models: p.runs.iter().map(|r| r.model).collect(), polar: c.polar.clone(), settings: c.settings.clone(), area: c.area });
        p.alts.get(i).map(|c| (p.source.clone(), p.ex.depart, c.alt.clone(), p.ex.dest, p.land.clone(), check, p.model))
    });
    if let Some((source, depart, alt, dest, land, check, model)) = chosen {
        close(app);
        // The plan keeps the forecast of the model the route was found on.
        super::flows::weather::show_source(app, model);
        super::flows::routing::use_alternative(app, &source, depart, &alt, dest, land.as_deref().map(|l| l as &dyn crate::core::routing::land::Land), check);
    }
}

pub fn remove_choice(app: &Rc<AppState>, i: usize) {
    if let Some(p) = app.router.borrow_mut().as_mut() {
        p.remove(i);
    }
    app.refresh();
}

/// Plays back the plan route `route_id` was made from, on the forecast it
/// was routed on. False when the route keeps no plan, or one that cannot be read.
pub fn replay(app: &Rc<AppState>, route_id: i64) -> bool {
    let (route, plan) = {
        let s = app.session.borrow();
        (s.routes.iter().find(|r| r.id == route_id).cloned(), s.plan(route_id))
    };
    let (Some(route), Some(plan)) = (route, plan) else { return false };
    let alt = Alternative::new(plan.nodes.clone(), plan.dest, plan.arrives);
    let (end, start) = (alt.end, plan.nodes[0].pos);
    let ex = Exploration {
        start,
        dest: plan.dest,
        depart: plan.nodes[0].time,
        lead: Duration::zero(),
        step: Duration::hours(1),
        tree: Vec::new(),
        fronts: Vec::new(),
        joins: Vec::new(),
        arrived: None,
        horizon: (!plan.arrives).then_some(end),
        stalled: false,
        nearest: None,
    };
    let mut bounds = Bounds::EMPTY;
    for n in &plan.nodes {
        bounds.extend([n.pos.lon, n.pos.lat]);
    }
    bounds.extend([plan.dest.lon, plan.dest.lat]);
    let pad = 0.05 * bounds.width().max(bounds.height()).max(0.2);
    let mut bounds = bounds.expanded(pad);
    // As in `preview_bounds`: the card covers the lower part of the chart.
    bounds.min_lat -= 0.7 * bounds.height();

    // Whatever was previewing is closed first, so its forecast is put back before this one replaces it.
    close(app);
    let weather_time = app.weather.borrow().time;
    let mut preview = RouterPreview::new(super::flows::routing::next_preview_id(), route.clone(), ex, weather_time);
    preview.alts = vec![Choice { alt, colour: 0 }];
    preview.end = end;
    // The clock starts at the departure, with the scrubber at the start of the timeline.
    preview.t = preview.ex.depart;
    if let Some(f) = crate::debug::router_clock() {
        preview.t = preview.ex.depart + Duration::milliseconds(((end - preview.ex.depart).num_milliseconds() as f64 * f) as i64);
    }
    let run = plan.run();
    let kept = !plan.forecast.is_empty();
    let edited = !plan.matches(&route.points.iter().map(|p| p.pos).collect::<Vec<_>>());
    let model = plan.source;
    preview.model = model.unwrap_or(app.weather.borrow().source);
    let store = if kept { std::mem::replace(&mut app.weather.borrow_mut().store, plan.forecast) } else { app.weather.borrow().store.clone() };
    preview.replay = Some(Replay { route_id, store, edited, run, model });
    follow_weather(app, preview.t);
    app.canvas.fit_bounds(&bounds);
    *app.router.borrow_mut() = Some(preview);
    app.refresh();
    true
}

/// Starts previewing a search. Returns whether the statistics should open
/// at once (`ISOMAR_ROUTER_STATS`).
pub fn show(app: &Rc<AppState>, mut preview: RouterPreview) -> bool {
    if let Some(f) = crate::debug::router_clock() {
        preview.t = preview.ex.depart + Duration::milliseconds(((preview.end - preview.ex.depart).num_milliseconds() as f64 * f) as i64);
    }
    let picks = crate::debug::router_picks();
    if let Some(front) = preview.current_front().map(|f| f.to_vec()).filter(|_| picks > 0) {
        for k in 0..picks {
            preview.add(front[(2 * k + 1) * front.len() / (2 * picks)]);
        }
    }
    follow_weather(app, preview.t);
    app.canvas.fit_bounds(&preview_bounds(&preview.ex));
    *app.router.borrow_mut() = Some(preview);
    app.refresh();
    if let Some(mode) = crate::debug::router_use() {
        use_choice(app, 0);
        let saved = app.session.borrow().routes.last().map(|r| r.id);
        if let Some(id) = saved.filter(|_| mode == "replay") {
            replay(app, id);
        }
        return false;
    }
    crate::debug::router_stats()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chips::plain;
    use crate::core::geo::LatLon;
    use crate::core::routing::isochrone::{Environment, Node, Settings, explore};
    use crate::core::routing::land::NoLand;
    use crate::core::routing::polar::Polar;
    use chrono::TimeZone;

    fn alt(arrives: bool, motor_hours: f64) -> Alternative {
        let t = Utc.with_ymd_and_hms(2026, 6, 1, 6, 0, 0).unwrap();
        let node = |lat: f64, h: i64| Node { pos: LatLon::new(lat, -5.0), time: t + Duration::hours(h), cog: 0.0, sog_kn: 5.0, heading: 0.0, tws_kn: 12.0, twd: 0.0, motoring: false, current: (0.0, 0.0) };
        Alternative {
            path: vec![node(48.0, 0), node(48.5, 6)],
            distance_nm: 30.0,
            remaining_nm: if arrives { 0.0 } else { 70.0 },
            motor_hours,
            max_tws_kn: 12.0,
            end: t + Duration::hours(6),
            arrives,
        }
    }

    struct Until(DateTime<Utc>);
    impl Environment for Until {
        fn wind(&self, _: &LatLon, t: DateTime<Utc>) -> Option<(f64, f64)> {
            (t <= self.0).then_some((15.0, 0.0))
        }
        fn horizon(&self) -> Option<DateTime<Utc>> {
            Some(self.0)
        }
    }

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, 6, 0, 0).unwrap()
    }

    /// A search into a north wind that the forecast cuts short, as it opens.
    fn opened() -> RouterPreview {
        let start = LatLon::new(48.0, -5.0);
        let dest = start.destination(0.0, 100.0);
        let polar = Polar::parse("TWA;3;40\n0;0;0\n40;0;0\n60;6;6\n90;6;6\n180;6;6\n").unwrap();
        let ex = explore(start, dest, t0(), &polar, &Until(t0() + Duration::hours(10)), &NoLand, &Settings::default(), None).unwrap();
        RouterPreview::new(1, Route::new("r"), ex, t0())
    }

    /// The same with the route it starts with removed, for the user's picks.
    fn search() -> RouterPreview {
        let mut p = opened();
        p.alts.clear();
        p
    }

    /// Flat metres-ish projection, good enough to test distances on screen.
    fn flat(p: &LatLon) -> (f64, f64) {
        (p.lon * 1000.0, -p.lat * 1000.0)
    }

    #[test]
    fn the_summary_says_how_far_is_left_and_how_much_engine() {
        let s = plain(&summary(&alt(false, 0.0)));
        assert!(s.contains("70") && s.contains("30") && !s.contains("engine"), "{s}");
        assert!(plain(&summary(&alt(false, 3.0))).contains("3 h under engine"));
        assert!(summary(&alt(true, 0.0)).iter().any(|c| c.icon == Some(Icon::Arrival)));
    }

    #[test]
    fn the_view_fits_the_search_and_leaves_room_for_the_card() {
        let p = search();
        let b = preview_bounds(&p.ex);
        let start = p.ex.start;
        assert!(b.contains([start.lon, start.lat]));
        for &i in p.ex.fronts.last().unwrap() {
            let q = p.ex.tree[i].node.pos;
            assert!(b.contains([q.lon, q.lat]), "{q:?} outside {b:?}");
        }
        // Everything is in the northern part; the south is left empty.
        let mid = b.min_lat + 0.4 * b.height();
        assert!(start.lat > mid, "start {} below {mid}", start.lat);
        assert!(p.ex.fronts.last().unwrap().iter().all(|&i| p.ex.tree[i].node.pos.lat > mid));
    }

    #[test]
    fn a_search_that_does_not_arrive_starts_with_the_way_that_got_nearest() {
        let p = opened();
        assert_eq!(p.t, p.end, "the whole search is shown");
        assert_eq!((p.alts.len(), p.selected), (1, 0));
        let a = &p.alts[0].alt;
        assert!(!a.arrives);
        assert_eq!(a.end, p.end, "it ends on the last front");
        let last = p.ex.fronts.last().unwrap();
        let nearest = last.iter().map(|&i| p.ex.tree[i].node.pos.distance_nm(&p.ex.dest)).fold(f64::INFINITY, f64::min);
        assert!(a.remaining_nm <= nearest + 1e-9, "{} > {nearest}", a.remaining_nm);
    }

    #[test]
    fn clicking_the_front_picks_the_position_nearest_to_the_click() {
        let p = search();
        let front = p.current_front().expect("the clock is at the end");
        assert_eq!(front, p.ex.fronts.last().unwrap().as_slice());
        for &i in [front[0], front[front.len() / 2], front[front.len() - 1]].iter() {
            let (x, y) = flat(&p.ex.tree[i].node.pos);
            // Fronts are dense, so only a click on a position finds that one;
            // far from the line finds nothing.
            assert_eq!(p.front_hit(&flat, x, y, 14.0), Some(i));
            assert_eq!(p.front_hit(&flat, x + 500.0, y + 500.0, 14.0), None);
        }
    }

    #[test]
    fn only_the_front_at_the_clock_can_be_clicked() {
        let mut p = search();
        let late = p.ex.fronts.last().unwrap()[0];
        let (x, y) = flat(&p.ex.tree[late].node.pos);
        p.t = p.ex.depart;
        assert!(p.current_front().is_none() && p.front_hit(&flat, x, y, 14.0).is_none());
        p.t = p.ex.depart + p.ex.step * 4;
        let front = p.current_front().unwrap();
        assert_eq!(front, p.ex.fronts[3].as_slice());
        let (x, y) = flat(&p.ex.tree[front[0]].node.pos);
        assert_eq!(p.front_hit(&flat, x, y, 14.0), Some(front[0]));
    }

    #[test]
    fn each_pick_is_an_alternative_with_its_own_colour() {
        let mut p = search();
        let front = p.ex.fronts.last().unwrap().clone();
        let picks = [front[0], front[front.len() / 2], front[front.len() - 1]];
        for (n, &i) in picks.iter().enumerate() {
            assert_eq!(p.add(i), Added::New(n));
            assert_eq!(p.selected, n, "the new one is selected");
        }
        let colours: std::collections::HashSet<_> = p.alts.iter().map(|c| c.colour).collect();
        assert_eq!(colours.len(), 3);
        // The same place twice is the same alternative.
        let v = p.version;
        assert_eq!(p.add(picks[0]), Added::Existing(0));
        assert_eq!((p.alts.len(), p.selected), (3, 0));
        assert!(p.version > v);
    }

    #[test]
    fn a_removed_colour_is_given_to_the_next_pick_and_the_others_keep_theirs() {
        let mut p = search();
        let front = p.ex.fronts.last().unwrap().clone();
        for &i in &[front[0], front[1], front[2]] {
            p.add(i);
        }
        let kept = (p.alts[0].colour, p.alts[2].colour);
        let freed = p.alts[1].colour;
        p.remove(1);
        assert_eq!((p.alts[0].colour, p.alts[1].colour), kept);
        assert_eq!(p.add(front[3]), Added::New(2));
        assert_eq!(p.alts[2].colour, freed);
    }

    #[test]
    fn there_are_only_as_many_alternatives_as_colours() {
        let mut p = search();
        let front = p.ex.fronts.last().unwrap().clone();
        assert!(front.len() > ALT_COLOURS.len());
        for &i in front.iter().take(ALT_COLOURS.len()) {
            assert!(matches!(p.add(i), Added::New(_)));
        }
        assert_eq!(p.add(front[ALT_COLOURS.len()]), Added::Full);
        assert_eq!(p.alts.len(), ALT_COLOURS.len());
    }

    #[test]
    fn removing_keeps_the_selection_in_range() {
        let mut p = search();
        let front = p.ex.fronts.last().unwrap().clone();
        p.add(front[0]);
        p.add(front[1]);
        assert_eq!(p.selected, 1);
        p.remove(1);
        assert_eq!(p.selected, 0);
        p.remove(0);
        assert!(p.alts.is_empty() && p.selected == 0);
        p.remove(0);
    }

    #[test]
    fn right_clicking_finds_the_alternatives_under_the_pointer_nearest_first() {
        let mut p = search();
        let front = p.ex.fronts.last().unwrap().clone();
        p.add(front[0]);
        p.add(front[front.len() - 1]);
        // Their ends are far apart; each is found at its own end.
        for (n, &i) in [front[0], front[front.len() - 1]].iter().enumerate() {
            let (x, y) = flat(&p.ex.tree[i].node.pos);
            assert_eq!(p.alts_hit(&flat, x + 2.0, y, 14.0).first(), Some(&n));
        }
        // Nowhere near either.
        assert!(p.alts_hit(&flat, 1e6, 1e6, 14.0).is_empty());
        // Before the clock has reached them there is nothing to hit at the far end.
        p.t = p.ex.depart;
        let (x, y) = flat(&p.ex.tree[front[0]].node.pos);
        assert!(p.alts_hit(&flat, x, y, 14.0).is_empty());
    }

    #[test]
    fn titles_say_which_route_it_is() {
        assert!(alt_title(0, &alt(true, 0.0)).starts_with("1. Route to the destination"));
        assert!(alt_title(2, &alt(false, 0.0)).starts_with("3. Route to 48°"));
    }

    mod replay {
        use super::*;
        use crate::core::chart::grid::Grid;
        use crate::core::nav::model::RoutePoint;
        use crate::core::nav::store::NavStore;
        use crate::core::routing::plan::RoutePlan;
        use crate::core::session::Session;
        use crate::core::weather::model::{Field, Param};
        use crate::engine::Engine;
        use crate::palette::Mode;

        fn app() -> Rc<AppState> {
            let canvas = Engine::new(crate::config::ViewState { lat: 48.0, lon: -4.0, zoom: 8.0 }, Mode::Day);
            AppState::new(canvas, Session::new(NavStore::open_memory().unwrap()).unwrap(), crate::config::Settings::default())
        }

        fn forecast(value: f32) -> WeatherStore {
            let mut s = WeatherStore::new();
            for h in (0..=12).step_by(3) {
                for p in [Param::WindU, Param::WindV] {
                    s.add(Field { param: p, time: t0() + Duration::hours(h), run: Some(t0()), grid: Grid { ncols: 10, nrows: 10, xll: -8.0, yll: 44.0, dx: 1.0, dy: 1.0, data: vec![value; 100] } });
                }
            }
            s
        }

        /// A route with a plan sailed on a 5 m/s forecast, while 9 m/s is loaded.
        fn with_plan(arrives: bool) -> (Rc<AppState>, i64) {
            let app = app();
            let mut route = Route::new("Weather route: x");
            route.points = vec![RoutePoint::new("a", LatLon::new(48.0, -5.0)), RoutePoint::new("b", LatLon::new(48.5, -5.0))];
            let r = app.session.borrow_mut().save_route(&route).unwrap();
            let node = |lat: f64, h: i64| Node { pos: LatLon::new(lat, -5.0), time: t0() + Duration::hours(h), cog: 0.0, sog_kn: 6.0, heading: 0.0, tws_kn: 15.0, twd: 0.0, motoring: false, current: (0.0, 0.0) };
            let plan = RoutePlan::new(t0(), LatLon::new(48.5, -5.0), arrives, vec![node(48.0, 0), node(48.25, 3), node(48.5, 6)], r.points.iter().map(|p| p.pos).collect(), &forecast(5.0));
            app.session.borrow_mut().save_plan(r.id, &plan).unwrap();
            app.weather.borrow_mut().store = forecast(9.0);
            (app, r.id)
        }

        fn wind_now(app: &AppState) -> f32 {
            app.weather.borrow().store.wind(48.2, -5.0, t0() + Duration::hours(1)).unwrap().u
        }

        #[test]
        fn a_weather_route_is_not_routed_again() {
            use serde_json::json;
            let (app, id) = with_plan(true);
            let plain = app.session.borrow_mut().save_route(&Route::new("plain")).unwrap().id;
            let get = |id: i64| crate::api::call(&app, "route_get", &json!({ "id": id }));
            assert_eq!((get(id)["weather_route"].clone(), get(plain)["weather_route"].clone()), (json!(true), json!(false)));
            let rows = crate::api::call(&app, "passage", &serde_json::Value::Null)["routes"].clone();
            let weather = |id: i64| rows.as_array().unwrap().iter().find(|r| r["id"] == id).unwrap()["weather"].clone();
            assert_eq!((weather(id), weather(plain)), (json!(true), json!(false)));
            for (name, arg) in [("route_compute", "departure"), ("window_run", "earliest")] {
                let r = crate::api::call(&app, name, &json!({ "id": id, arg: "2026-11-20 08:00" }));
                assert!(r["error"].as_str().is_some_and(|e| e.contains("weather routing")), "{name}: {r}");
            }
        }

        #[test]
        fn a_route_without_a_plan_cannot_be_replayed() {
            let app = app();
            let r = app.session.borrow_mut().save_route(&Route::new("plain")).unwrap();
            assert!(!replay(&app, r.id) && app.router.borrow().is_none());
        }

        #[test]
        fn the_replay_shows_the_passage_on_its_own_forecast_and_puts_the_loaded_one_back() {
            let (app, id) = with_plan(true);
            assert_eq!(wind_now(&app), 9.0);
            assert!(replay(&app, id));
            {
                let r = app.router.borrow();
                let p = r.as_ref().unwrap();
                assert_eq!((p.alts.len(), p.t, p.end), (1, t0(), t0() + Duration::hours(6)), "one passage, the clock at the departure");
                assert!(p.alts[0].alt.arrives && p.fronts_shown() == 0);
            }
            assert_eq!(wind_now(&app), 5.0, "the forecast it was planned on");
            let card = crate::vm::router::card(&app).unwrap();
            assert!(card.replay && !card.can_add && card.title.contains("as planned"), "{card:?}");
            assert!(crate::vm::router::stats(&app).is_some(), "the statistics are there");
            close(&app);
            assert!(app.router.borrow().is_none());
            assert_eq!(wind_now(&app), 9.0, "the loaded forecast is back");
        }

        #[test]
        fn a_replay_cannot_be_used_as_a_new_route() {
            let (app, id) = with_plan(true);
            assert!(replay(&app, id));
            let before = app.session.borrow().routes.len();
            use_choice(&app, 0);
            assert_eq!(app.session.borrow().routes.len(), before);
            assert!(app.router.borrow().is_some(), "still previewing");
            close(&app);
        }

        #[test]
        fn replaying_twice_does_not_lose_the_loaded_forecast() {
            let (app, id) = with_plan(false);
            assert!(replay(&app, id));
            assert!(replay(&app, id));
            close(&app);
            assert_eq!(wind_now(&app), 9.0);
        }

        #[test]
        fn the_plan_page_says_what_was_planned_and_whether_the_route_changed() {
            let (app, id) = with_plan(true);
            let info = crate::vm::plan::info(&app, id).unwrap();
            assert!(crate::chips::plain(&info.summary).contains("arrives") && info.forecast.contains("Left") && !info.edited, "{info:?}");
            let mut r = app.session.borrow().routes.iter().find(|r| r.id == id).cloned().unwrap();
            r.points.pop();
            app.session.borrow_mut().save_route(&r).unwrap();
            assert!(crate::vm::plan::info(&app, id).unwrap().edited);
            let (heads, sections) = crate::vm::plan::stats(&app, id).unwrap();
            assert_eq!(heads.len(), 1);
            assert!(sections.iter().all(|s| s.rows.iter().all(|(_, cells)| cells.len() == 1)));
        }

        /// A passage routed on two models: GFS with a 5 m/s west wind, IFS with 9 m/s.
        fn two_models(app: &Rc<AppState>) {
            use crate::core::routing::compare::{self, CrossCheck};
            use crate::core::routing::isochrone::{Bounded, StoreEnv};
            use crate::core::weather::source::Source;
            {
                let mut w = app.weather.borrow_mut();
                *w.store_of_mut(Source::Gfs) = forecast(5.0);
                *w.store_of_mut(Source::Ifs) = forecast(9.0);
            }
            let start = LatLon::new(48.0, -5.0);
            let dest = start.destination(0.0, 30.0);
            let polar = std::sync::Arc::new(Polar::generic_cruiser());
            let area = crate::core::vector::Bounds { min_lon: -8.0, max_lon: 2.0, min_lat: 44.0, max_lat: 54.0 };
            let s = Settings::default();
            let w = app.weather.borrow();
            let envs: Vec<StoreEnv> = [Source::Gfs, Source::Ifs].iter().map(|m| StoreEnv(w.store_of(*m).unwrap())).collect();
            let bounded: Vec<Bounded<StoreEnv>> = envs.iter().map(|e| Bounded { env: e, bounds: area }).collect();
            let exs: Vec<Exploration> = bounded.iter().map(|e| explore(start, dest, t0(), &polar, e, &NoLand, &s, None).unwrap()).collect();
            let bests: Vec<Alternative> = exs.iter().map(|ex| alternatives::best(ex).unwrap()).collect();
            let ways: Vec<Vec<LatLon>> = bests.iter().map(|a| a.path.iter().map(|n| n.pos).collect()).collect();
            let refs: Vec<&dyn Environment> = bounded.iter().map(|e| e as &dyn Environment).collect();
            let cross = CrossCheck::run(&ways, &refs, t0(), &polar, &s);
            let agree = compare::agree(&ways, 30.0, 3.0);
            drop(w);
            let mut exs = exs.into_iter();
            let mut p = RouterPreview::new(1, Route::new("Passage"), exs.next().unwrap(), t0());
            p.model = Source::Gfs;
            p.runs = vec![ModelRun { model: Source::Gfs, ex: None, best: bests[0].clone() }, ModelRun { model: Source::Ifs, ex: exs.next(), best: bests[1].clone() }];
            p.comparison = Some(Comparison { polar, settings: s, area, cross, agree });
            *app.router.borrow_mut() = Some(p);
        }

        #[test]
        fn several_models_are_shown_one_at_a_time_compared_and_checked_once_saved() {
            use crate::core::weather::source::Source;
            let app = app();
            two_models(&app);
            let card = crate::vm::router::card(&app).unwrap();
            assert_eq!(card.models.iter().map(|m| (m.label.as_str(), m.focused)).collect::<Vec<_>>(), vec![("GFS", true), ("ECMWF IFS", false)]);
            assert_eq!(card.models.iter().filter(|m| m.robust).count(), 1, "one route holds up best");
            assert!(card.verdict.iter().any(|v| v.contains("agree")), "{:?}", card.verdict);
            assert!(card.verdict.iter().any(|v| v.contains("arrive between")), "{:?}", card.verdict);
            // The stronger wind is faster: IFS arrives first.
            let (heads, sections) = crate::vm::router::cross_check(&app).unwrap();
            assert_eq!(heads.len(), 3);
            assert!(sections.iter().all(|s| s.rows.len() == 2 && s.rows.iter().all(|r| r.1.len() == 3)));
            // Showing IFS: its search, its colour, its wind; GFS keeps its own.
            crate::vm::router::focus_model(&app, 1);
            {
                let r = app.router.borrow();
                let p = r.as_ref().unwrap();
                assert_eq!((p.model, p.focused(), p.alts[0].colour), (Source::Ifs, Some(1), 1));
                assert!(p.runs[0].ex.is_some());
            }
            assert_eq!(app.weather.borrow().source, Source::Ifs);
            // A new alternative takes neither model's colour.
            let node = app.router.borrow().as_ref().unwrap().current_front().unwrap()[0];
            app.router.borrow_mut().as_mut().unwrap().add(node);
            assert!(app.router.borrow().as_ref().unwrap().alts.iter().skip(1).all(|c| c.colour > 1));
            // Saved, the route keeps its arrival on each forecast.
            use_choice(&app, 0);
            let id = app.session.borrow().routes.last().unwrap().id;
            let plan = app.session.borrow().plan(id).unwrap();
            assert_eq!(plan.source, Some(Source::Ifs));
            assert_eq!(plan.checks.iter().map(|c| c.source).collect::<Vec<_>>(), vec![Source::Gfs, Source::Ifs]);
            assert!(plan.checks.iter().all(|c| c.arrives));
            assert!(plan.checks[1].end < plan.checks[0].end, "more wind, sooner");
            let info = crate::vm::plan::info(&app, id).unwrap();
            assert!(info.bracket.contains("between"), "{}", info.bracket);
        }

        #[test]
        fn the_saved_route_follows_the_tolerance_the_user_set() {
            let app = app();
            // A bend that a wide tolerance cuts off and a narrow one follows.
            let mut path = vec![Node { pos: LatLon::new(48.0, -5.0), time: t0(), cog: 0.0, sog_kn: 6.0, heading: 0.0, tws_kn: 15.0, twd: 0.0, motoring: true, current: (0.0, 0.0) }];
            for (i, brg) in [10.0, 10.0, 10.0, 10.0, 350.0, 350.0, 350.0, 350.0].iter().enumerate() {
                let last = path.last().unwrap().clone();
                path.push(Node { pos: last.pos.destination(*brg, 5.0), time: t0() + Duration::hours(i as i64 + 1), cog: *brg, heading: *brg, ..last });
            }
            let dest = path.last().unwrap().pos;
            let alt = Alternative::new(path, dest, true);
            let saved = |tolerance: f64| {
                app.settings.borrow_mut().route_tolerance_nm = tolerance;
                crate::flows::routing::use_alternative(&app, &Route::new("x"), t0(), &alt, dest, None, None);
                app.session.borrow().routes.last().unwrap().points.len()
            };
            let (narrow, wide) = (saved(0.1), saved(20.0));
            assert!(narrow > wide && wide == 2, "{narrow} then {wide}");
        }
    }
}
