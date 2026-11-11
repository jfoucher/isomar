//! The isochrone weather router.
//!
//! From the start, every position reachable after one time step is computed
//! by trying a fan of headings at the boat speed the polar gives for the wind
//! there and then, from every point of the last front. The sea is cut into
//! small cells: the best point in each cell is kept, and none in a cell an
//! earlier front reached first; the edge of what is reached is the next
//! "isochrone". This repeats until the destination is reached, and the route
//! is read back through the parents of the arriving point.

use super::land::Land;
use super::polar::Polar;
use crate::geo::{LatLon, normalize_bearing};
use crate::nav::calc::distance_to_segment_nm;
use crate::vector::Bounds;
use crate::weather::store::WeatherStore;
use chrono::{DateTime, Duration, Utc};
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

/// What the router needs to know about the weather.
pub trait Environment: Sync {
    /// True wind speed in knots and the direction it blows *from*, degrees true.
    fn wind(&self, p: &LatLon, t: DateTime<Utc>) -> Option<(f64, f64)>;
    /// Whether the wind is known there and then: what `wind` would say, without working it out.
    fn has_wind(&self, p: &LatLon, t: DateTime<Utc>) -> bool {
        self.wind(p, t).is_some()
    }
    /// Surface current as (east, north) knots.
    fn current(&self, _p: &LatLon, _t: DateTime<Utc>) -> Option<(f64, f64)> {
        None
    }
    /// Whether `current` can ever say anything: without currents the router skips sampling them
    /// mid-leg, and its arithmetic is what it was before it knew of them.
    fn has_currents(&self) -> bool {
        false
    }
    fn wave_height(&self, _p: &LatLon, _t: DateTime<Utc>) -> Option<f64> {
        None
    }
    /// Gusts in knots. The router does not use them; the weather window
    /// counts the hours they are strong.
    fn gust(&self, _p: &LatLon, _t: DateTime<Utc>) -> Option<f64> {
        None
    }
    /// The last moment the wind is known, if it is ever unknown. Past it the
    /// router stops and reports where it got to instead of failing.
    fn horizon(&self) -> Option<DateTime<Utc>> {
        None
    }
}

/// An environment behind a box, so a composed one can own what it wraps.
impl<T: Environment + ?Sized> Environment for Box<T> {
    fn wind(&self, p: &LatLon, t: DateTime<Utc>) -> Option<(f64, f64)> {
        (**self).wind(p, t)
    }
    fn has_wind(&self, p: &LatLon, t: DateTime<Utc>) -> bool {
        (**self).has_wind(p, t)
    }
    fn current(&self, p: &LatLon, t: DateTime<Utc>) -> Option<(f64, f64)> {
        (**self).current(p, t)
    }
    fn has_currents(&self) -> bool {
        (**self).has_currents()
    }
    fn wave_height(&self, p: &LatLon, t: DateTime<Utc>) -> Option<f64> {
        (**self).wave_height(p, t)
    }
    fn gust(&self, p: &LatLon, t: DateTime<Utc>) -> Option<f64> {
        (**self).gust(p, t)
    }
    fn horizon(&self) -> Option<DateTime<Utc>> {
        (**self).horizon()
    }
}

/// Weather read from the app's store.
pub struct StoreEnv<'a>(pub &'a WeatherStore);

impl Environment for StoreEnv<'_> {
    fn horizon(&self) -> Option<DateTime<Utc>> {
        self.0.wind_range().map(|(_, hi)| hi)
    }

    fn wind(&self, p: &LatLon, t: DateTime<Utc>) -> Option<(f64, f64)> {
        self.0.wind(p.lat, p.lon, t).map(|w| (crate::weather::model::units::ms_to_kn(w.speed_ms()) as f64, w.blows_from() as f64))
    }

    fn has_wind(&self, p: &LatLon, t: DateTime<Utc>) -> bool {
        self.0.has_wind(p.lat, p.lon, t)
    }

    fn current(&self, p: &LatLon, t: DateTime<Utc>) -> Option<(f64, f64)> {
        self.0.current(p.lat, p.lon, t).map(|c| (crate::weather::model::units::ms_to_kn(c.u) as f64, crate::weather::model::units::ms_to_kn(c.v) as f64))
    }

    fn has_currents(&self) -> bool {
        self.0.has(crate::weather::model::Param::CurrentU)
    }

    fn wave_height(&self, p: &LatLon, t: DateTime<Utc>) -> Option<f64> {
        self.0.sample(crate::weather::model::Param::WaveHeight, p.lat, p.lon, t).map(|h| h as f64)
    }

    fn gust(&self, p: &LatLon, t: DateTime<Utc>) -> Option<f64> {
        self.0.sample(crate::weather::model::Param::Gust, p.lat, p.lon, t).map(|g| crate::weather::model::units::ms_to_kn(g) as f64)
    }
}

/// Weather only inside `bounds`: the area the search is meant to cover. Past
/// it there is no wind, so the search does not go there, where it would know
/// nothing of the land either.
pub struct Bounded<'a, E: Environment + ?Sized> {
    pub env: &'a E,
    pub bounds: Bounds,
}

impl<E: Environment + ?Sized> Bounded<'_, E> {
    fn inside(&self, p: &LatLon) -> bool {
        self.bounds.contains([p.lon, p.lat])
    }
}

impl<E: Environment + ?Sized> Environment for Bounded<'_, E> {
    fn horizon(&self) -> Option<DateTime<Utc>> {
        self.env.horizon()
    }

    fn wind(&self, p: &LatLon, t: DateTime<Utc>) -> Option<(f64, f64)> {
        self.inside(p).then(|| self.env.wind(p, t)).flatten()
    }

    fn has_wind(&self, p: &LatLon, t: DateTime<Utc>) -> bool {
        self.inside(p) && self.env.has_wind(p, t)
    }

    fn current(&self, p: &LatLon, t: DateTime<Utc>) -> Option<(f64, f64)> {
        self.inside(p).then(|| self.env.current(p, t)).flatten()
    }

    fn has_currents(&self) -> bool {
        self.env.has_currents()
    }

    fn wave_height(&self, p: &LatLon, t: DateTime<Utc>) -> Option<f64> {
        self.inside(p).then(|| self.env.wave_height(p, t)).flatten()
    }

    fn gust(&self, p: &LatLon, t: DateTime<Utc>) -> Option<f64> {
        self.inside(p).then(|| self.env.gust(p, t)).flatten()
    }
}

/// The same wind everywhere, always.
pub struct ConstantWind {
    pub kn: f64,
    pub from: f64,
}

impl Environment for ConstantWind {
    fn wind(&self, _: &LatLon, _: DateTime<Utc>) -> Option<(f64, f64)> {
        Some((self.kn, self.from))
    }
}

/// Which point survives in each cell of sea.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prune {
    /// The one farthest from the start (by water round land).
    FarthestFromStart,
    /// The one closest to the destination (by water round land).
    ClosestToDestination,
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub step_minutes: f64,
    pub heading_step_deg: f64,
    /// How far either side of the way to the destination headings are
    /// tried. All round by default: beating round a cape can mean sailing
    /// away from it first, and legs that go back land in cells already
    /// reached and are dropped.
    pub max_deviation_deg: f64,
    /// Scales the polar: 0.9 for a loaded boat, say.
    pub performance: f64,
    pub motor_speed_kn: Option<f64>,
    /// Motor when sailing would be slower than this.
    pub motor_below_kn: f64,
    /// Do not sail in more wind than this.
    pub max_wind_kn: f64,
    pub max_wave_m: Option<f64>,
    /// The destination counts as reached within this distance.
    pub arrival_nm: f64,
    pub max_hours: f64,
    /// Width of the square cells of sea used for pruning: one point is kept
    /// in each, and none in a cell an earlier front reached.
    pub cell_nm: f64,
    pub prune: Prune,
    /// A point is dropped as behind when it is farther from the destination
    /// than the nearest one by more than `BEHIND_HOURS` of the fastest leg, or
    /// by this share of the way the nearest has come, whichever is more. A
    /// fixed distance narrows the front to nothing on a long passage; a share
    /// keeps the strategies that diverge early (the side of the ITCZ to
    /// cross on, which way round a high).
    pub behind_share: f64,
    /// A point may wait where it is for the stream to turn (`HOLD_*`): on when the passage has
    /// currents. Waiting at the start is what the departure scan is for, so not in the first hour.
    pub allow_hold: bool,
}

/// A hold is tried only where the stream runs at least this fast...
pub const HOLD_MIN_KN: f64 = 0.5;
/// ...and the boat can stem it with this to spare.
pub const HOLD_MARGIN_KN: f64 = 0.5;
/// No hold within this many seconds of leaving.
pub const HOLD_AFTER_S: f64 = 3600.0;

impl Default for Settings {
    fn default() -> Self {
        Settings {
            step_minutes: 30.0,
            heading_step_deg: 5.0,
            max_deviation_deg: 180.0,
            performance: 1.0,
            motor_speed_kn: None,
            motor_below_kn: 2.0,
            max_wind_kn: 35.0,
            max_wave_m: None,
            arrival_nm: 0.3,
            // Longer than GFS's 384 h, so the forecast is what ends a long passage.
            max_hours: 24.0 * 17.0,
            cell_nm: 1.0,
            prune: Prune::FarthestFromStart,
            behind_share: 0.35,
            allow_hold: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RouteError {
    /// No weather at the start, so nothing can be computed.
    NoWeather,
    BlockedStart,
    BlockedDestination,
    /// Every heading is blocked by land, wind, waves or calm.
    NoProgress { hours: f64 },
    /// Not reached within the allowed time.
    TooLong,
    InvalidSettings(String),
    /// The forecast ran out before the destination was reached. `explore`
    /// reports this as a result; only `route` turns it into an error.
    ForecastEnds { hours: f64 },
    Cancelled,
}

impl std::fmt::Display for RouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RouteError::NoWeather => write!(f, "there is no weather data at the start"),
            RouteError::BlockedStart => write!(f, "the start is on land, with no open water within {SNAP_NM} nm"),
            RouteError::BlockedDestination => write!(f, "the destination is on land, with no open water within {SNAP_NM} nm"),
            RouteError::NoProgress { hours } => write!(f, "no way forward after {hours:.0} hours (calm, too much wind or waves, or land in the way)"),
            RouteError::TooLong => write!(f, "the destination is not reachable within the time allowed"),
            RouteError::InvalidSettings(s) => write!(f, "invalid settings: {s}"),
            RouteError::ForecastEnds { hours } => write!(f, "the forecast ends {hours:.0} hours after departure, before the destination is reached"),
            RouteError::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::error::Error for RouteError {}

#[derive(Clone, Debug)]
pub struct Node {
    pub pos: LatLon,
    pub time: DateTime<Utc>,
    /// Course over ground and speed over ground of the leg that ended here.
    pub cog: f64,
    pub sog_kn: f64,
    /// Where the bow pointed on that leg; differs from `cog` in a current.
    pub heading: f64,
    /// The wind the boat sailed in on that leg: over the water, so the wind over the ground less the
    /// current.
    pub tws_kn: f64,
    pub twd: f64,
    pub motoring: bool,
    /// The current met on that leg (at its middle), (east, north) knots.
    pub current: (f64, f64),
}

impl Node {
    /// Whether the leg that ended here was spent waiting for the stream to turn, heading into it
    /// (`Settings::allow_hold`): the only legs that make no way. (The start, which ends no leg, has no
    /// speed either.)
    pub fn holding(&self) -> bool {
        self.sog_kn == 0.0
    }
}

#[derive(Clone, Debug)]
pub struct RouteResult {
    pub nodes: Vec<Node>,
    pub arrival: DateTime<Utc>,
    pub distance_nm: f64,
    pub direct_nm: f64,
    /// The pruned fronts, for drawing.
    pub isochrones: Vec<Vec<LatLon>>,
    pub motor_hours: f64,
}

impl RouteResult {
    pub fn duration(&self) -> Duration {
        self.arrival - self.nodes[0].time
    }

    /// Turning points: where the course changes by at least `min_turn_deg`,
    /// plus a point at least every `max_leg_hours`, with the ends.
    pub fn waypoints(&self, min_turn_deg: f64, max_leg_hours: f64) -> Vec<LatLon> {
        waypoints_of(&self.nodes, min_turn_deg, max_leg_hours)
    }
}

/// `RouteResult::waypoints` for any run of nodes (at least two).
pub fn waypoints_of(n: &[Node], min_turn_deg: f64, max_leg_hours: f64) -> Vec<LatLon> {
    let mut out = vec![n[0].pos];
    let mut since = n[0].time;
    for i in 1..n.len() - 1 {
        let turn = {
            let d = (n[i + 1].cog - n[i].cog).rem_euclid(360.0);
            if d > 180.0 { 360.0 - d } else { d }
        };
        let long = (n[i].time - since).num_seconds() as f64 / 3600.0 >= max_leg_hours;
        if turn >= min_turn_deg || long {
            out.push(n[i].pos);
            since = n[i].time;
        }
    }
    out.push(n[n.len() - 1].pos);
    out
}

/// Within this many of the longest legs of the destination, the front also
/// keeps the nearest point in each of `APPROACH_SLOTS` directions from it.
const APPROACH_LEGS: f64 = 5.0;
const APPROACH_SLOTS: usize = 36;
/// A candidate farther from the destination (by water) than the front's nearest one by
/// more than this many hours of the fastest leg has fallen behind for good:
/// it is sailing away, or sideways along the far side of the start, and only
/// widens the front. Generous, because the way round a cape or out of a bay
/// can lead away from the destination for a while. On a long passage
/// `Settings::behind_share` of the way made allows more.
const BEHIND_HOURS: f64 = 24.0;
/// The way by water is a detour when it is this much longer than the direct
/// line; headings then centre on it instead of on the destination.
const DETOUR_RATIO: f64 = 1.05;
/// Front points one parallel task expands at least: fewer, and handing them out costs more than it saves.
const MIN_POINTS_A_TASK: usize = 16;
/// Candidates one parallel task looks up in the sea fields at least.
const MIN_LOOKUPS_A_TASK: usize = 512;
/// More current than any forecast gives, for the bound on how far a leg can go.
const MAX_CURRENT_KN: f64 = 10.0;
/// A cell of `Grid`: rows from the equator, columns from the meridian.
type Cell = (i32, i32);

/// A map keyed by cells, with a hash for two small integers (the default one, made to resist attacks,
/// was a tenth of a search's time).
type CellMap<V> = HashMap<Cell, V, std::hash::BuildHasherDefault<CellHasher>>;

#[derive(Default)]
struct CellHasher(u64);

impl std::hash::Hasher for CellHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.write_u64(*b as u64);
        }
    }

    fn write_i32(&mut self, i: i32) {
        self.write_u64(i as u32 as u64);
    }

    fn write_u64(&mut self, i: u64) {
        self.0 = (self.0.rotate_left(5) ^ i).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}

/// Square cells of sea `cell_nm` wide, columns scaled for one latitude.
struct Grid {
    deg: f64,
    cos: f64,
}

impl Grid {
    fn new(cell_nm: f64, lat: f64) -> Grid {
        Grid { deg: cell_nm / 60.0, cos: lat.to_radians().cos().max(0.05) }
    }

    fn cell(&self, p: &LatLon) -> Cell {
        ((p.lat / self.deg).floor() as i32, (p.lon * self.cos / self.deg).floor() as i32)
    }

    fn centre(&self, (r, c): Cell) -> LatLon {
        LatLon::new((r as f64 + 0.5) * self.deg, (c as f64 + 0.5) * self.deg / self.cos)
    }
}

fn neighbours((r, c): Cell) -> impl Iterator<Item = Cell> {
    [(-1, -1), (-1, 0), (-1, 1), (0, -1), (0, 1), (1, -1), (1, 0), (1, 1)].into_iter().map(move |(dr, dc)| (r + dr, c + dc))
}

/// `cells` as chains of neighbours, to draw as lines. Each chain starts
/// where it has the fewest neighbours (an end, if it has one) and goes on
/// to the neighbour with the fewest left, side neighbours before corner
/// ones, so it follows the edge rather than cutting across it.
fn chains(cells: &BTreeSet<Cell>) -> Vec<Vec<Cell>> {
    let mut left = cells.clone();
    let degree = |c: Cell, left: &BTreeSet<Cell>| neighbours(c).filter(|n| left.contains(n)).count();
    let mut out = Vec::new();
    while let Some(&first) = left.iter().min_by_key(|&&c| (degree(c, &left), c)) {
        let mut chain = vec![first];
        left.remove(&first);
        let mut at = first;
        while let Some(next) = neighbours(at).filter(|n| left.contains(n)).min_by_key(|&n| (degree(n, &left), n.0 != at.0 && n.1 != at.1, n)) {
            left.remove(&next);
            chain.push(next);
            at = next;
        }
        out.push(chain);
    }
    out
}

#[derive(Clone)]
struct Cand {
    pos: LatLon,
    parent: usize,
    secs: f64,
    cog: f64,
    sog: f64,
    heading: f64,
    tws: f64,
    twd: f64,
    motor: bool,
    cur: (f64, f64),
}

impl Cand {
    /// The sine and cosine of the course over ground.
    fn east_unit(&self) -> f64 {
        self.cog.to_radians().sin()
    }

    fn north_unit(&self) -> f64 {
        self.cog.to_radians().cos()
    }
}

fn angle_between(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(360.0);
    if d > 180.0 { 360.0 - d } else { d }
}

/// A position the search reached. Index 0 of `Exploration::tree` is the start.
#[derive(Clone, Debug)]
pub struct TreeNode {
    pub node: Node,
    pub parent: Option<usize>,
}

/// Everything the search found, so the fronts can be drawn and any position
/// on the last one read back to the start.
#[derive(Clone, Debug)]
pub struct Exploration {
    pub start: LatLon,
    pub dest: LatLon,
    pub depart: DateTime<Utc>,
    /// The time from the start out to where there was room to search from.
    pub lead: Duration,
    pub step: Duration,
    pub tree: Vec<TreeNode>,
    /// Indices into `tree`; `fronts[k]` is the set of positions at
    /// `depart + lead + (k + 1) * step`: the edge of what was reached, not
    /// every point searched from. The front that arrived is not included.
    pub fronts: Vec<Vec<usize>>,
    /// For each front, whether position `j` is drawn joined to `j + 1`
    /// (`joins[k].len() == fronts[k].len() - 1`). A front is the edge of the
    /// water reached by then, as chains of neighbouring cells, each joined
    /// along its length and not to the next.
    pub joins: Vec<Vec<bool>>,
    /// Set when the destination was reached.
    pub arrived: Option<RouteResult>,
    /// Set when the search stopped short of the destination: the time of the
    /// last front. That is where the forecast (or `max_hours`) ended, or, if
    /// `stalled`, where the boat could get no further.
    pub horizon: Option<DateTime<Utc>>,
    /// The search stopped because no front was getting any closer (calm,
    /// too much wind, land in the way), not because the forecast ended.
    pub stalled: bool,
    /// When the search stopped short: the position of the last front
    /// nearest the destination, by water where the land mask knows the way.
    pub nearest: Option<Nearest>,
}

/// The position a search that stopped short got nearest the destination.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Nearest {
    /// Index into `Exploration::tree`.
    pub index: usize,
    /// How far it still is from the destination's open water, by water
    /// (as the crow flies where the water there does not reach it).
    pub remaining_nm: f64,
    /// How far the start's open water was, measured the same way.
    pub start_nm: f64,
}

impl Nearest {
    /// How much of the way was made good, 0 to 1.
    pub fn share_made(&self) -> f64 {
        if self.start_nm > 0.0 { ((self.start_nm - self.remaining_nm) / self.start_nm).clamp(0.0, 1.0) } else { 0.0 }
    }
}

impl Exploration {
    /// Front `k` as runs of positions to draw as one line each, split where
    /// the front is not joined up. A run may be a single position.
    pub fn front_runs(&self, k: usize) -> Vec<&[usize]> {
        split_runs(&self.fronts[k], &self.joins[k])
    }

    /// The positions from the start to `tree[i]`, oldest first.
    pub fn path_to(&self, mut i: usize) -> Vec<Node> {
        let mut out = Vec::new();
        loop {
            out.push(self.tree[i].node.clone());
            match self.tree[i].parent {
                Some(p) => i = p,
                None => break,
            }
        }
        out.reverse();
        // The start has no leg of its own; it takes the direction of the first.
        if out.len() > 1 {
            out[0].cog = out[1].cog;
        }
        out
    }
}

/// Lets another thread follow and cancel a search.
#[derive(Default)]
pub struct Progress {
    pub cancel: AtomicBool,
    /// Steps done and the steps expected (to the forecast end if there is
    /// one, otherwise the time limit).
    pub step: AtomicUsize,
    pub steps: AtomicUsize,
    /// The fronts so far, as runs of positions to draw as lines (see
    /// `Exploration::front_runs`), for a shell to show the search as it grows.
    pub fronts: Mutex<Vec<Vec<Vec<LatLon>>>>,
}

/// `front` cut into runs where `joins[j]` says position `j` is not joined to `j + 1`.
fn split_runs<'a, T>(front: &'a [T], joins: &[bool]) -> Vec<&'a [T]> {
    let mut runs = Vec::new();
    let mut from = 0;
    for (j, joined) in joins.iter().enumerate() {
        if !joined {
            runs.push(&front[from..=j]);
            from = j + 1;
        }
    }
    if !front.is_empty() {
        runs.push(&front[from..]);
    }
    runs
}

/// Computes a route. `depart` is when the boat leaves `start`. A forecast that
/// ends before the destination is reached is an error here; `explore` reports
/// how far the boat could get instead.
#[allow(clippy::too_many_arguments)]
pub fn route(start: LatLon, dest: LatLon, depart: DateTime<Utc>, polar: &Polar, env: &dyn Environment, land: &dyn Land, s: &Settings) -> Result<RouteResult, RouteError> {
    let ex = explore(start, dest, depart, polar, env, land, s, None)?;
    match (ex.arrived, ex.horizon) {
        (Some(r), _) => Ok(r),
        (None, Some(h)) if ex.stalled => Err(RouteError::NoProgress { hours: (h - depart).num_seconds() as f64 / 3600.0 }),
        (None, Some(h)) if env.horizon().is_none_or(|l| h < l) => Err(RouteError::TooLong),
        (None, Some(h)) => Err(RouteError::ForecastEnds { hours: (h - depart).num_seconds() as f64 / 3600.0 }),
        (None, None) => Err(RouteError::TooLong),
    }
}

/// How far an end of the passage may be from open water in the land mask.
pub const SNAP_NM: f64 = 3.0;

/// The speed along the way between an end of the passage and open water
/// when no engine speed is set.
const APPROACH_KN: f64 = 5.0;

/// Searches from `start` towards `dest` for as long as the weather is known.
#[allow(clippy::too_many_arguments)]
pub fn explore(start: LatLon, dest: LatLon, depart: DateTime<Utc>, polar: &Polar, env: &dyn Environment, land: &dyn Land, s: &Settings, progress: Option<&Progress>) -> Result<Exploration, RouteError> {
    if s.step_minutes <= 0.0 || s.heading_step_deg <= 0.0 || s.cell_nm <= 0.0 || s.max_hours <= 0.0 {
        return Err(RouteError::InvalidSettings("steps and limits must be positive".into()));
    }
    let dt_s = s.step_minutes * 60.0;
    // A harbour, a river or a narrow sound is no place for legs an hour
    // long, and may even be inside the mask (a cell and its margin are wider
    // than the water): the search starts and ends where there is room, and
    // the ends are joined to it by the way through the narrow water.
    let clear_nm = (0.25 * 8.0 * dt_s / 3600.0).clamp(0.2, 2.0);
    let out = land.approach(start, clear_nm, SNAP_NM).ok_or(RouteError::BlockedStart)?;
    let mut into = land.approach(dest, clear_nm, SNAP_NM).ok_or(RouteError::BlockedDestination)?;
    into.reverse();
    let (from, goal) = (out[out.len() - 1], into[0]);
    if env.wind(&from, depart).is_none() {
        return Err(RouteError::NoWeather);
    }
    let direct_nm = start.distance_nm(&dest);
    // Progress is measured by water: as the crow flies, the way round a
    // peninsula looks like going backwards, and is pruned or called a stall.
    let field = land.seaway(goal);
    // Water the field does not reach is cut off from the goal: as the crow
    // flies it could look nearest of all, and the rest of the front would
    // be pruned as behind it.
    let to_goal = |p: &LatLon| match &field {
        Some(f) => f.distance_nm(p).unwrap_or(f64::INFINITY),
        None => p.distance_nm(&goal),
    };
    // How far out from the start a point is: as the crow flies where that is
    // the way, by water round land. Past Messina from Split, the boats making
    // for Napoli come back towards Split; as the crow flies the ones running
    // west along Sicily looked farther out and were kept instead.
    let outward = land.seaway(from);
    let reach = |p: &LatLon| {
        let crow = from.distance_nm(p);
        match outward.as_ref().and_then(|f| f.distance_nm(p).map(|d| (d, f.cell_nm()))) {
            Some((sea, cell)) if sea > crow * DETOUR_RATIO + 2.0 * cell => sea,
            _ => crow,
        }
    };
    // Headings fan out round the way by water where it bends away from the
    // direct bearing, so a boat can sail away from the goal to round a cape.
    let ahead_nm = 3.0 * 8.0 * dt_s / 3600.0;
    let aim = |p: &LatLon| {
        if let Some(f) = &field
            && let Some(sea) = f.distance_nm(p)
            && sea > p.distance_nm(&goal) * DETOUR_RATIO + 2.0 * f.cell_nm()
            && let Some(q) = f.toward(p, ahead_nm.max(4.0 * f.cell_nm()))
        {
            return p.bearing_to(&q);
        }
        p.bearing_to(&goal)
    };
    let approach = Approach { kn: s.motor_speed_kn.unwrap_or(APPROACH_KN), motor: s.motor_speed_kn.is_some() };

    // Nodes accepted so far; the first is the start, then the way out.
    let mut nodes: Vec<Cand> = vec![Cand { pos: start, parent: usize::MAX, secs: 0.0, cog: start.bearing_to(&dest), sog: 0.0, heading: start.bearing_to(&dest), tws: 0.0, twd: 0.0, motor: false, cur: (0.0, 0.0) }];
    for p in &out[1..] {
        let last = &nodes[nodes.len() - 1];
        let cog = last.pos.bearing_to(p);
        let secs = last.secs + last.pos.distance_nm(p) / approach.kn * 3600.0;
        nodes.push(Cand { pos: *p, parent: nodes.len() - 1, secs, cog, sog: approach.kn, heading: cog, tws: 0.0, twd: 0.0, motor: approach.motor, cur: (0.0, 0.0) });
    }
    let lead_s = nodes[nodes.len() - 1].secs;
    let mut front: Vec<usize> = vec![nodes.len() - 1];
    let mut isochrones = Vec::new();
    let mut fronts: Vec<Vec<usize>> = Vec::new();
    let mut front_joins: Vec<Vec<bool>> = Vec::new();
    let mut arrived = None;
    let mut horizon = None;
    let mut stalled = false;
    let mut max_steps = (s.max_hours * 3600.0 / dt_s).ceil() as usize;
    let limit = env.horizon();
    if let Some(h) = limit {
        let until = (((h - depart).num_milliseconds() as f64 / 1000.0 - lead_s).max(0.0) / dt_s).ceil() as usize + 1;
        max_steps = max_steps.min(until);
    }
    if let Some(p) = progress {
        p.steps.store(max_steps, Ordering::Relaxed);
    }
    let start_dist = to_goal(&from);
    let mut best_dist = start_dist;
    let (mut dropped_behind, mut widest) = (0usize, 0usize);
    let mut stagnant = 0usize;
    let grid = Grid::new(s.cell_nm, (from.lat + goal.lat) / 2.0);
    // The step at which each cell was first reached.
    let mut visited: CellMap<usize> = CellMap::default();
    let mut edge: BTreeSet<Cell> = BTreeSet::new();
    // Whether the middle of a cell is land, as the edge asks.
    let mut sea_land: CellMap<bool> = CellMap::default();
    // No leg is faster than this (the polar's best, the engine, and a generous current).
    let fastest_kn = polar.max_speed() * s.performance.max(1.0) + s.motor_speed_kn.unwrap_or(0.0) + MAX_CURRENT_KN;
    let half = (s.max_deviation_deg / s.heading_step_deg).floor() as i32;
    // All round, the heading straight back would be tried twice.
    let last = if 2.0 * half as f64 * s.heading_step_deg >= 360.0 { half - 1 } else { half };
    // The sine and cosine of each turn of the fan from the aim.
    let turns: Vec<(f64, f64)> = (-half..=last).map(|k| (k as f64 * s.heading_step_deg).to_radians().sin_cos()).collect();
    let currents = env.has_currents();
    let hold = currents && s.allow_hold;

    for step in 0..max_steps {
        if let Some(p) = progress {
            if p.cancel.load(Ordering::Relaxed) {
                return Err(RouteError::Cancelled);
            }
            p.step.store(step, Ordering::Relaxed);
        }
        // The front being expanded is at this time; if the weather is not
        // known then, stop here and report the front.
        let front_time = depart + Duration::milliseconds(((lead_s + step as f64 * dt_s) * 1000.0) as i64);
        if limit.is_some_and(|h| front_time > h) {
            horizon = Some(front_time);
            break;
        }
        // A leg ending in a cell an earlier front reached is dropped by the pruning below, unless it stays in
        // its own edge cell: such legs (most of an all-round fan) are left out here, before the land and the
        // weather are asked about them. Those near the goal stay, for the arrival and the approach slots.
        let reject_near_nm = (APPROACH_LEGS + 1.0) * fastest_kn * dt_s / 3600.0;
        // The fastest leg of the whole fan, those left out included, as if none were.
        let fastest_leg = AtomicU64::new(0);
        let behind = |node_pos: &LatLon, own: Cell, pos: &LatLon| {
            let cell = grid.cell(pos);
            visited.contains_key(&cell) && !(cell == own && edge.contains(&own)) && ((pos.lat - goal.lat).abs() * 60.0 > reject_near_nm || pos.distance_nm(&goal) > reject_near_nm) && distance_to_segment_nm(node_pos, pos, &goal) > s.arrival_nm
        };
        let candidates: Vec<Cand> = front
            .par_iter()
            .with_min_len(MIN_POINTS_A_TASK)
            .flat_map_iter(|&pi| {
                let node = &nodes[pi];
                let own = grid.cell(&node.pos);
                let t = depart + Duration::milliseconds((node.secs * 1000.0) as i64);
                let mut out = Vec::new();
                let Some((tws, twd)) = env.wind(&node.pos, t) else { return out.into_iter() };
                let c0 = if currents { env.current(&node.pos, t).unwrap_or((0.0, 0.0)) } else { (0.0, 0.0) };
                // The boat sails in the wind over the water: the wind over the ground less the current.
                let (tws, twd) = crate::geo::wind_over_water(tws, twd, c0);
                if tws > s.max_wind_kn {
                    return out.into_iter();
                }
                if let (Some(max), Some(h)) = (s.max_wave_m, env.wave_height(&node.pos, t))
                    && h > max
                {
                    return out.into_iter();
                }
                let (ce, cn) = c0;
                // The leg again with the current met halfway along it, which differs from the start's in a
                // race a mile or two wide or where the stream turns within the leg.
                let lat_sc = node.pos.lat.to_radians().sin_cos();
                let t_mid = t + Duration::milliseconds((dt_s * 500.0) as i64);
                let mid_leg = |c: Cand, (we, wn): (f64, f64)| -> Cand {
                    if !currents {
                        return c;
                    }
                    let half = c.sog * dt_s / 7200.0;
                    let mid = node.pos.destination_by(lat_sc, c.east_unit(), c.north_unit(), half);
                    let (me, mn) = env.current(&mid, t_mid).unwrap_or((0.0, 0.0));
                    let (east, north) = (we + me, wn + mn);
                    let sog = east.hypot(north);
                    if sog < 0.1 {
                        return Cand { sog: 0.0, ..c };
                    }
                    let pos = node.pos.destination_by(lat_sc, east / sog, north / sog, sog * dt_s / 3600.0);
                    Cand { pos, cog: normalize_bearing(east.atan2(north).to_degrees()), sog, cur: (me, mn), ..c }
                };
                let bearing = aim(&node.pos);
                let at_wind = polar.at_wind(tws);
                let (sb, cb) = bearing.to_radians().sin_cos();
                // Whether a leg may be sailed: the weather known where it ends, and no land on the way.
                let open = |pos: &LatLon| {
                    // A leg that reaches the goal stops there: what lies beyond does not matter.
                    let end = if distance_to_segment_nm(&node.pos, pos, &goal) <= s.arrival_nm { goal } else { *pos };
                    // Nowhere to go where the weather is not known.
                    (end == goal || env.has_wind(pos, t)) && !land.blocked(&node.pos, &end)
                };
                // The legs left out, with how far each ends from the goal: the nearest that may be sailed
                // still counts for the leader and the nearest of all, as it did before they were left out.
                let mut left_out: Vec<(f64, Cand, (f64, f64))> = Vec::new();
                for (k, &(sk, ck)) in (-half..=last).zip(&turns) {
                    let heading = normalize_bearing(bearing + k as f64 * s.heading_step_deg);
                    let twa = angle_between(twd, heading);
                    let sail = at_wind.speed(twa) * s.performance;
                    let (speed, motor) = match s.motor_speed_kn {
                        Some(m) if sail < s.motor_below_kn => (m.max(sail), true),
                        _ => (sail, false),
                    };
                    if speed < 0.2 {
                        continue;
                    }
                    // The heading's sine and cosine, turned from the aim's by angle addition.
                    let (sh, ch) = (sb * ck + cb * sk, cb * ck - sb * sk);
                    let water = (speed * sh, speed * ch);
                    let (east, north) = (water.0 + ce, water.1 + cn);
                    let sog = east.hypot(north);
                    if sog < 0.1 {
                        continue;
                    }
                    let cog = normalize_bearing(east.atan2(north).to_degrees());
                    let pos = node.pos.destination_by(lat_sc, east / sog, north / sog, sog * dt_s / 3600.0);
                    // Non-negative floats order as their bits do.
                    fastest_leg.fetch_max(sog.to_bits(), Ordering::Relaxed);
                    let cand = Cand { pos, parent: pi, secs: node.secs + dt_s, cog, sog, heading, tws, twd, motor, cur: c0 };
                    if behind(&node.pos, own, &pos) {
                        left_out.push((to_goal(&pos), cand, water));
                        continue;
                    }
                    let cand = mid_leg(cand, water);
                    if cand.sog >= 0.1 && open(&cand.pos) {
                        out.push(cand);
                    }
                }
                left_out.sort_by(|a, b| a.0.total_cmp(&b.0));
                if let Some(c) = left_out.into_iter().map(|(_, c, water)| mid_leg(c, water)).find(|c| c.sog >= 0.1 && open(&c.pos)) {
                    out.push(c);
                }
                // Waiting for the stream: the point stays put for a leg, the boat heading into the stream
                // and stemming it, where it can.
                if hold && node.secs >= lead_s + HOLD_AFTER_S {
                    let c_kn = ce.hypot(cn);
                    if c_kn >= HOLD_MIN_KN {
                        let sail = (0..=36).map(|k| at_wind.speed(k as f64 * 5.0)).fold(0.0, f64::max) * s.performance;
                        let best = s.motor_speed_kn.map_or(sail, |m| sail.max(m));
                        if best >= c_kn + HOLD_MARGIN_KN {
                            let into = normalize_bearing(crate::geo::set_of(ce, cn) + 180.0);
                            out.push(Cand { pos: node.pos, parent: pi, secs: node.secs + dt_s, cog: into, sog: 0.0, heading: into, tws, twd, motor: false, cur: c0 });
                        }
                    }
                }
                out.into_iter()
            })
            .collect();
        if candidates.is_empty() {
            // Nothing can move any more. With earlier fronts there is still
            // something to choose from; with none there is nothing to show.
            if step == 0 {
                return Err(RouteError::NoProgress { hours: 0.0 });
            }
            horizon = Some(front_time);
            stalled = true;
            break;
        }

        // Did any leg pass the destination? Take the earliest arrival.
        let mut arrival: Option<(f64, usize)> = None;
        for (ci, c) in candidates.iter().enumerate() {
            let p = &nodes[c.parent];
            if distance_to_segment_nm(&p.pos, &c.pos, &goal) <= s.arrival_nm {
                let leg = p.pos.distance_nm(&c.pos).max(1e-9);
                let frac = (p.pos.distance_nm(&goal) / leg).clamp(0.0, 1.0);
                let t = p.secs + frac * dt_s;
                if arrival.is_none_or(|(bt, _)| t < bt) {
                    arrival = Some((t, ci));
                }
            }
        }
        if let Some((t, ci)) = arrival {
            let c = candidates[ci].clone();
            arrived = Some(build_result(&nodes, c, t, &into, approach, depart, direct_nm, std::mem::take(&mut isochrones)));
            break;
        }

        // Points that have fallen far behind the nearest one are dropped.
        let longest_leg = f64::from_bits(fastest_leg.load(Ordering::Relaxed)) * dt_s / 3600.0;
        // Lookups in the field are cheap: in small tasks they cost more in handing out than they save.
        let remaining: Vec<f64> = candidates.par_iter().with_min_len(MIN_LOOKUPS_A_TASK).map(|c| to_goal(&c.pos)).collect();
        let leader = remaining.iter().copied().fold(f64::INFINITY, f64::min);
        let made = if start_dist.is_finite() && leader.is_finite() { (start_dist - leader).max(0.0) } else { 0.0 };
        let behind = leader + (BEHIND_HOURS * 3600.0 / dt_s * longest_leg).max(s.behind_share * made);
        let before = candidates.len();
        let (candidates, remaining): (Vec<Cand>, Vec<f64>) = candidates.into_iter().zip(remaining).filter(|(_, d)| *d <= behind).unzip();
        dropped_behind += before - candidates.len();
        let reached: Vec<f64> = candidates.par_iter().with_min_len(MIN_LOOKUPS_A_TASK).map(|c| reach(&c.pos)).collect();
        // One point per cell of the sea, the best, and none in a cell an
        // earlier front reached: the boat was there sooner, so whatever goes
        // on from there is behind. Unlike directions from the start, this
        // holds round any coast: past a cape, the front along the shore is
        // no longer on the same bearing as water much farther out.
        let mut best: BTreeMap<Cell, (f64, usize)> = BTreeMap::new();
        for (ci, c) in candidates.iter().enumerate() {
            let cell = grid.cell(&c.pos);
            // A point on the edge may stay in its cell: in light wind a leg
            // can be shorter than a cell, and the front must still creep on.
            if visited.contains_key(&cell) && !(edge.contains(&cell) && cell == grid.cell(&nodes[c.parent].pos)) {
                continue;
            }
            // Smaller scores win.
            let score = match s.prune {
                Prune::ClosestToDestination => remaining[ci],
                Prune::FarthestFromStart => -reached[ci],
            };
            let slot = best.entry(cell).or_insert((score, ci));
            if score < slot.0 {
                *slot = (score, ci);
            }
        }
        for &cell in best.keys() {
            visited.entry(cell).or_insert(step);
        }
        // Cells that no point happened to land in but that are surrounded
        // by reached ones are behind too.
        let holes: BTreeSet<Cell> = best.keys().flat_map(|&c| neighbours(c)).filter(|n| !visited.contains_key(n) && neighbours(*n).filter(|m| visited.contains_key(m)).count() >= 6).collect();
        for h in holes {
            visited.insert(h, step);
        }
        // The edge: kept cells next to open water no front has reached.
        let mut is_open = |c: Cell| !visited.contains_key(&c) && !*sea_land.entry(c).or_insert_with(|| land.blocked(&grid.centre(c), &grid.centre(c)));
        edge = best.keys().copied().filter(|&c| neighbours(c).any(&mut is_open)).collect();

        // Near the destination, the nearest point in each direction from it
        // is kept as well, and the nearest of all wherever it is, whatever
        // cell they are in: a front can pass the destination with no leg
        // close enough to it, and then the cells round it are all behind.
        let near_goal = APPROACH_LEGS * longest_leg;
        let mut around: Vec<Option<(f64, usize)>> = vec![None; APPROACH_SLOTS];
        let mut nearest: Option<(f64, usize)> = None;
        for (ci, c) in candidates.iter().enumerate() {
            if nearest.is_none_or(|(bd, _)| remaining[ci] < bd) {
                nearest = Some((remaining[ci], ci));
            }
            let d = c.pos.distance_nm(&goal);
            if d <= near_goal {
                let slot = ((goal.bearing_to(&c.pos) / 360.0 * APPROACH_SLOTS as f64) as usize).min(APPROACH_SLOTS - 1);
                if around[slot].is_none_or(|(bd, _)| d < bd) {
                    around[slot] = Some((d, ci));
                }
            }
        }
        let mut approach: Vec<usize> = around.into_iter().flatten().map(|(_, ci)| ci).collect();
        if let Some((_, ci)) = nearest
            && !approach.contains(&ci)
        {
            approach.push(ci);
        }
        let kept_ci: BTreeSet<usize> = best.values().map(|&(_, ci)| ci).collect();
        approach.retain(|ci| !kept_ci.contains(ci));

        // Every kept point is searched on from; the edge, in chains of
        // neighbouring cells, is the front that is drawn.
        front.clear();
        let mut node_of: BTreeMap<Cell, usize> = BTreeMap::new();
        let mut closest = f64::INFINITY;
        for (&cell, &(_, ci)) in &best {
            closest = closest.min(remaining[ci]);
            nodes.push(candidates[ci].clone());
            front.push(nodes.len() - 1);
            node_of.insert(cell, nodes.len() - 1);
        }
        let mut drawn = Vec::new();
        let mut joined = Vec::new();
        for chain in chains(&edge) {
            joined.extend(std::iter::repeat_n(true, chain.len() - 1));
            joined.push(false);
            drawn.extend(chain.iter().map(|c| node_of[c]));
        }
        for &ci in &approach {
            closest = closest.min(remaining[ci]);
            nodes.push(candidates[ci].clone());
            front.push(nodes.len() - 1);
        }
        // Each chain's last flag is the gap to what follows; the last of all has nothing to follow.
        joined.pop();
        widest = widest.max(drawn.len());
        let drawn_pos: Vec<LatLon> = drawn.iter().map(|&i| nodes[i].pos).collect();
        if let Some(p) = progress {
            p.fronts.lock().unwrap().push(split_runs(&drawn_pos, &joined).into_iter().map(<[LatLon]>::to_vec).collect());
        }
        isochrones.push(drawn_pos);
        front_joins.push(joined);
        fronts.push(drawn);

        if closest < best_dist - 0.05 {
            best_dist = closest;
            stagnant = 0;
        } else {
            stagnant += 1;
            // A day without getting closer: boxed in by land or a persistent
            // calm. The fronts so far are kept, for the user to choose from.
            if stagnant as f64 * s.step_minutes / 60.0 > 24.0 {
                horizon = Some(front_time + Duration::milliseconds((dt_s * 1000.0) as i64));
                stalled = true;
                break;
            }
        }
    }
    // Ran out of steps, at the forecast end or at `max_hours`: the fronts so
    // far are kept, for the user to choose from.
    if arrived.is_none() && horizon.is_none() {
        horizon = Some(depart + Duration::milliseconds(((lead_s + max_steps as f64 * dt_s) * 1000.0) as i64));
    }
    // Where the search stopped short, the last front's position nearest the
    // goal is its best guess at the way: by water, else (cut off from the
    // goal in the mask) as the crow flies.
    let nearest = if arrived.is_some() || fronts.is_empty() {
        None
    } else {
        let by_water = front.iter().map(|&i| (to_goal(&nodes[i].pos), i)).filter(|(d, _)| d.is_finite()).min_by(|a, b| a.0.total_cmp(&b.0));
        let (remaining_nm, index, start_nm) = match by_water {
            Some((d, i)) => (d, i, start_dist),
            None => front.iter().map(|&i| (nodes[i].pos.distance_nm(&goal), i, from.distance_nm(&goal))).min_by(|a, b| a.0.total_cmp(&b.0)).expect("a front has positions"),
        };
        Some(Nearest { index, remaining_nm, start_nm: if start_nm.is_finite() { start_nm } else { from.distance_nm(&goal) } })
    };
    if std::env::var_os("ISOMAR_DEBUG_ROUTER").is_some() {
        crate::log!("router: {} fronts, widest {widest} points, {dropped_behind} candidates dropped as behind, {} points searched", fronts.len(), nodes.len());
    }
    let at = |secs: f64| depart + Duration::milliseconds((secs * 1000.0) as i64);
    let tree = nodes
        .iter()
        .map(|c| TreeNode {
            node: Node { pos: c.pos, time: at(c.secs), cog: c.cog, sog_kn: c.sog, heading: c.heading, tws_kn: c.tws, twd: c.twd, motoring: c.motor, current: c.cur },
            parent: (c.parent != usize::MAX).then_some(c.parent),
        })
        .collect();
    let lead = Duration::milliseconds((lead_s * 1000.0) as i64);
    Ok(Exploration { start, dest, depart, lead, step: Duration::milliseconds((dt_s * 1000.0) as i64), tree, fronts, joins: front_joins, arrived, horizon, stalled, nearest })
}

/// How the ends of the passage are joined to the open water.
#[derive(Clone, Copy)]
struct Approach {
    kn: f64,
    motor: bool,
}

/// The route that arrived at `into[0]`, the open water the search was
/// heading for, carried on along `into` to the destination.
#[allow(clippy::too_many_arguments)]
fn build_result(nodes: &[Cand], last: Cand, arrive_secs: f64, into: &[LatLon], approach: Approach, depart: DateTime<Utc>, direct_nm: f64, isochrones: Vec<Vec<LatLon>>) -> RouteResult {
    let at = |secs: f64| depart + Duration::milliseconds((secs * 1000.0) as i64);
    // Walk back from the arriving leg.
    let mut chain: Vec<&Cand> = Vec::new();
    let mut i = last.parent;
    while i != usize::MAX {
        chain.push(&nodes[i]);
        i = nodes[i].parent;
    }
    chain.reverse();
    let mut out: Vec<Node> = chain
        .iter()
        .map(|c| Node { pos: c.pos, time: at(c.secs), cog: c.cog, sog_kn: c.sog, heading: c.heading, tws_kn: c.tws, twd: c.twd, motoring: c.motor, current: c.cur })
        .collect();
    out.push(Node { pos: into[0], time: at(arrive_secs), cog: last.cog, sog_kn: last.sog, heading: last.heading, tws_kn: last.tws, twd: last.twd, motoring: last.motor, current: last.cur });
    let mut arrive_secs = arrive_secs;
    for w in into.windows(2) {
        arrive_secs += w[0].distance_nm(&w[1]) / approach.kn * 3600.0;
        let cog = w[0].bearing_to(&w[1]);
        out.push(Node { pos: w[1], time: at(arrive_secs), cog, sog_kn: approach.kn, heading: cog, tws_kn: last.tws, twd: last.twd, motoring: approach.motor, current: (0.0, 0.0) });
    }
    // The first node's cog is the direction of the first leg.
    if out.len() > 1 {
        out[0].cog = out[1].cog;
    }
    let distance_nm = out.windows(2).map(|w| w[0].pos.distance_nm(&w[1].pos)).sum();
    let motor_hours = out.windows(2).filter(|w| w[1].motoring).map(|w| (w[1].time - w[0].time).num_seconds() as f64 / 3600.0).sum();
    RouteResult { arrival: at(arrive_secs), nodes: out, distance_nm, direct_nm, isochrones, motor_hours }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::land::{NoLand, RasterLand};
    use crate::vector::{Bounds, Polygon};
    use chrono::TimeZone;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, 6, 0, 0).unwrap()
    }

    /// 6 kn at any angle of 60 degrees or more, nothing within 40 degrees,
    /// ramping up between, whatever the wind (as long as there is some).
    fn simple_polar() -> Polar {
        Polar::parse("TWA;3;40\n0;0;0\n40;0;0\n60;6;6\n90;6;6\n180;6;6\n").unwrap()
    }

    fn east_of(p: LatLon, nm: f64) -> LatLon {
        p.destination(90.0, nm)
    }

    fn start() -> LatLon {
        LatLon::new(48.0, -5.0)
    }

    #[test]
    fn points_that_fall_far_behind_the_leader_are_dropped() {
        let d = east_of(start(), 500.0);
        let s = Settings { step_minutes: 60.0, ..Settings::default() };
        let ex = explore(start(), d, t0(), &simple_polar(), &ConstantWind { kn: 15.0, from: 0.0 }, &NoLand, &s, None).unwrap();
        let farthest = |k: usize| ex.fronts[k].iter().map(|&i| ex.tree[i].node.pos.distance_nm(&d)).fold(0.0, f64::max);
        // Early on, sailing north or south of the start is still in the running...
        assert!(farthest(5) > 500.0, "{}", farthest(5));
        // ...but a day later than the leader it is not: only the way east is left.
        assert!(farthest(40) < 500.0, "{}", farthest(40));
        assert!(ex.arrived.is_some());
    }

    /// How far from the line to the destination the front reaches at its
    /// widest, over the second half of a long passage.
    fn spread(share: f64) -> f64 {
        let d = east_of(start(), 2500.0);
        let s = Settings { step_minutes: 60.0, cell_nm: 4.0, behind_share: share, ..Settings::default() };
        let ex = explore(start(), d, t0(), &simple_polar(), &ConstantWind { kn: 15.0, from: 0.0 }, &NoLand, &s, None).unwrap();
        let k = ex.fronts.len() * 3 / 4;
        ex.fronts[k].iter().map(|&i| (ex.tree[i].node.pos.lat - start().lat).abs() * 60.0).fold(0.0, f64::max)
    }

    #[test]
    fn a_long_passage_keeps_a_wider_front_than_a_fixed_distance_would() {
        let (fixed, share) = (spread(0.0), spread(0.25));
        assert!(share > fixed * 1.3, "fixed {fixed:.0} nm, share {share:.0} nm");
    }

    #[test]
    fn a_shell_can_follow_the_fronts_as_they_grow() {
        let d = east_of(start(), 120.0);
        let progress = Progress::default();
        let ex = explore(start(), d, t0(), &simple_polar(), &ConstantWind { kn: 15.0, from: 0.0 }, &NoLand, &Settings::default(), Some(&progress)).unwrap();
        let live = progress.fronts.lock().unwrap();
        assert_eq!(live.len(), ex.fronts.len());
        for k in 0..live.len() {
            let runs: Vec<Vec<LatLon>> = ex.front_runs(k).iter().map(|r| r.iter().map(|&i| ex.tree[i].node.pos).collect()).collect();
            assert_eq!(live[k], runs, "front {k}");
        }
    }

    #[test]
    fn the_search_stays_where_the_weather_is_known() {
        let d = east_of(start(), 60.0);
        let bounds = Bounds { min_lon: -5.1, max_lon: d.lon + 0.1, min_lat: 47.9, max_lat: 48.1 };
        let env = Bounded { env: &ConstantWind { kn: 15.0, from: 0.0 }, bounds };
        assert!(env.wind(&LatLon::new(48.2, -4.5), t0()).is_none());
        let ex = explore(start(), d, t0(), &simple_polar(), &env, &NoLand, &Settings::default(), None).unwrap();
        assert!(ex.fronts.iter().flatten().all(|&i| bounds.contains([ex.tree[i].node.pos.lon, ex.tree[i].node.pos.lat])));
        assert!(ex.arrived.is_some());
    }

    #[test]
    fn a_beam_reach_takes_distance_over_speed() {
        let d = east_of(start(), 20.0);
        let r = route(start(), d, t0(), &simple_polar(), &ConstantWind { kn: 15.0, from: 0.0 }, &NoLand, &Settings::default()).unwrap();
        let hours = r.duration().num_minutes() as f64 / 60.0;
        assert!((hours - 20.0 / 6.0).abs() < 0.2, "{hours} h");
        assert!((r.distance_nm - 20.0).abs() < 1.5, "{}", r.distance_nm);
        assert_eq!(r.nodes.first().unwrap().pos, start());
        assert_eq!(r.nodes.last().unwrap().pos, d);
        assert_eq!(r.arrival, r.nodes.last().unwrap().time);
        assert!(r.motor_hours == 0.0 && !r.isochrones.is_empty());
    }

    #[test]
    fn going_to_windward_means_tacking() {
        let d = start().destination(0.0, 12.0);
        let r = route(start(), d, t0(), &simple_polar(), &ConstantWind { kn: 15.0, from: 0.0 }, &NoLand, &Settings::default()).unwrap();
        let hours = r.duration().num_minutes() as f64 / 60.0;
        // About 60 degrees off the wind at 6 kn makes 3 kn good: 4 hours, give or take.
        assert!((3.5..6.5).contains(&hours), "{hours} h");
        assert!(r.distance_nm > 12.0 * 1.4, "sailed {} nm", r.distance_nm);
        // It never sails closer than the polar allows.
        for n in &r.nodes[1..] {
            assert!(angle_between(n.cog, 0.0) >= 38.0, "heading {}", n.cog);
        }
        // And it does tack: courses on both sides of the wind.
        let port = r.nodes.iter().any(|n| (n.cog - 0.0).rem_euclid(360.0) > 180.0);
        let stbd = r.nodes.iter().any(|n| (n.cog - 0.0).rem_euclid(360.0) < 180.0);
        assert!(port && stbd);
    }

    #[test]
    fn a_favourable_current_shortens_the_passage() {
        struct Stream;
        impl Environment for Stream {
            fn wind(&self, _: &LatLon, _: DateTime<Utc>) -> Option<(f64, f64)> {
                Some((15.0, 0.0))
            }
            fn current(&self, _: &LatLon, _: DateTime<Utc>) -> Option<(f64, f64)> {
                Some((2.0, 0.0))
            }
            fn has_currents(&self) -> bool {
                true
            }
        }
        let d = east_of(start(), 20.0);
        let r = route(start(), d, t0(), &simple_polar(), &Stream, &NoLand, &Settings::default()).unwrap();
        let hours = r.duration().num_minutes() as f64 / 60.0;
        assert!((hours - 2.5).abs() < 0.2, "8 knots over the ground: {hours} h");
        assert!(r.nodes[1].sog_kn > 7.5);
    }

    /// 15 kn from the north over the ground, and a current: everywhere, or only in a band east of the
    /// start (from `band.0` to `band.1` nm).
    struct Tide {
        current: (f64, f64),
        band: Option<(f64, f64)>,
    }

    impl Environment for Tide {
        fn wind(&self, _: &LatLon, _: DateTime<Utc>) -> Option<(f64, f64)> {
            Some((15.0, 0.0))
        }
        fn current(&self, p: &LatLon, _: DateTime<Utc>) -> Option<(f64, f64)> {
            let east_nm = (p.lon - start().lon) * 60.0 * start().lat.to_radians().cos();
            match self.band {
                Some((a, b)) if east_nm < a || east_nm > b => Some((0.0, 0.0)),
                _ => Some(self.current),
            }
        }
        fn has_currents(&self) -> bool {
            true
        }
    }

    #[test]
    fn the_boat_sails_in_the_wind_over_the_water() {
        // 15 kn from the north over the ground, a 3 kn stream setting north: 18 kn over the water.
        let d = east_of(start(), 20.0);
        let r = route(start(), d, t0(), &Polar::generic_cruiser(), &Tide { current: (0.0, 3.0), band: None }, &NoLand, &Settings::default()).unwrap();
        let n = &r.nodes[2];
        assert!((n.tws_kn - 18.0).abs() < 0.01 && (n.twd - 0.0).abs() < 0.01, "{} kn from {}", n.tws_kn, n.twd);
        assert_eq!(n.current, (0.0, 3.0));
        // A stream across the wind veers it: carried east under a north wind, the boat feels the wind from
        // east of north.
        let r = route(start(), d, t0(), &Polar::generic_cruiser(), &Tide { current: (3.0, 0.0), band: None }, &NoLand, &Settings::default()).unwrap();
        let n = &r.nodes[2];
        assert!((n.tws_kn - 15.0_f64.hypot(3.0)).abs() < 0.01 && (n.twd - 3.0_f64.atan2(15.0).to_degrees()).abs() < 0.01, "{} kn from {}", n.tws_kn, n.twd);
    }

    #[test]
    fn a_leg_meets_the_current_halfway_along_it() {
        // A foul stream of 4 kn in a band from 0.8 to 2.2 nm east of the start, and the destination 2.9 nm
        // east: one leg of 30 minutes at 6 kn would reach it, starting where there is no stream. Read at
        // the start of the leg, the band would not be felt at all; read at its middle, it is.
        let d = east_of(start(), 2.9);
        let s = Settings { step_minutes: 30.0, ..Settings::default() };
        let calm = route(start(), d, t0(), &simple_polar(), &Tide { current: (0.0, 0.0), band: None }, &NoLand, &s).unwrap();
        let band = route(start(), d, t0(), &simple_polar(), &Tide { current: (-4.0, 0.0), band: Some((0.8, 2.2)) }, &NoLand, &s).unwrap();
        assert!(calm.duration() <= Duration::minutes(31), "{}", calm.duration());
        assert!(band.nodes.iter().any(|n| n.current == (-4.0, 0.0)), "a leg records the band's stream");
        assert!(band.duration() > Duration::minutes(40), "{}", band.duration());
    }

    fn island_between() -> (RasterLand, LatLon, LatLon) {
        // Boat at 48N, 5W; destination about 40 nm east; an island blocks the direct line.
        let s = start();
        let d = east_of(s, 40.0);
        let mid = east_of(s, 20.0);
        let (dlat, dlon) = (6.0 / 60.0, 6.0 / 60.0 / 48f64.to_radians().cos());
        let poly = Polygon {
            rings: vec![vec![
                [mid.lon - dlon, mid.lat - dlat],
                [mid.lon + dlon, mid.lat - dlat],
                [mid.lon + dlon, mid.lat + dlat],
                [mid.lon - dlon, mid.lat + dlat],
                [mid.lon - dlon, mid.lat - dlat],
            ]],
        };
        let b = Bounds { min_lon: s.lon - 0.5, min_lat: s.lat - 1.0, max_lon: d.lon + 0.5, max_lat: s.lat + 1.0 };
        (RasterLand::from_polygons(b, 0.005, &[poly], 2), s, d)
    }

    #[test]
    fn the_route_goes_round_an_island() {
        let (land, s, d) = island_between();
        assert!(land.blocked(&s, &d), "the direct line crosses the island");
        let r = route(s, d, t0(), &simple_polar(), &ConstantWind { kn: 15.0, from: 0.0 }, &land, &Settings::default()).unwrap();
        for w in r.nodes.windows(2) {
            assert!(!land.blocked(&w[0].pos, &w[1].pos), "leg through land at {:?}", w[0].pos);
        }
        assert!(r.distance_nm > 40.5, "had to detour: {}", r.distance_nm);
        assert!(r.distance_nm < 55.0, "but not absurdly: {}", r.distance_nm);
        let ys: Vec<f64> = r.nodes.iter().map(|n| n.pos.lat).collect();
        let spread = ys.iter().cloned().fold(f64::MIN, f64::max) - ys.iter().cloned().fold(f64::MAX, f64::min);
        assert!(spread > 0.05, "the path bends: {spread}");
    }

    /// Round Iberia from Donostia, the fronts along the coast past each cape
    /// were missing: the water farther out on the same bearing from the start
    /// won every direction bin. Here a peninsula hangs south between the start
    /// and the goal, and the route must be on a drawn front all the way round.
    #[test]
    fn the_fronts_follow_the_route_round_a_peninsula() {
        let s = start();
        let d = east_of(s, 20.0);
        let mid = east_of(s, 10.0);
        let dlon = 2.0 / 60.0 / 48f64.to_radians().cos();
        let poly = Polygon {
            rings: vec![vec![
                [mid.lon - dlon, mid.lat - 0.8],
                [mid.lon + dlon, mid.lat - 0.8],
                [mid.lon + dlon, mid.lat + 1.5],
                [mid.lon - dlon, mid.lat + 1.5],
                [mid.lon - dlon, mid.lat - 0.8],
            ]],
        };
        let b = Bounds { min_lon: s.lon - 1.0, min_lat: s.lat - 1.5, max_lon: d.lon + 1.0, max_lat: s.lat + 1.0 };
        let land = RasterLand::from_polygons(b, 0.005, &[poly], 2);
        let set = Settings::default();
        // From the east, the boat beats round the tip: south-east first, away from the goal.
        for from in [0.0, 90.0, 180.0, 270.0] {
            let ex = explore(s, d, t0(), &simple_polar(), &ConstantWind { kn: 15.0, from }, &land, &set, None).unwrap();
            let r = ex.arrived.as_ref().unwrap_or_else(|| panic!("wind from {from}: stalled {} after {} fronts", ex.stalled, ex.fronts.len()));
            assert!(r.nodes.iter().any(|n| n.pos.lat < mid.lat - 0.8), "the route rounds the tip");
            for k in 0..ex.fronts.len() {
                let t = ex.tree[ex.fronts[k][0]].node.time;
                let Some(n) = r.nodes.iter().find(|n| n.time == t) else { continue };
                let near = ex.fronts[k].iter().map(|&i| ex.tree[i].node.pos.distance_nm(&n.pos)).fold(f64::INFINITY, f64::min);
                assert!(near <= 2.0 * set.cell_nm, "wind from {from}: front {k} is {near:.1} nm from the route");
            }
        }
    }

    /// Mahón to Cagliari once stalled 6 to 12 nm past the goal: getting
    /// nearer it meant coming back towards the start, and the direction bins
    /// (the pruning then) kept the points farther out. Here the goal is in the lee of a block of land, a
    /// few legs from its corners.
    #[test]
    fn a_goal_in_the_lee_of_land_is_reached_from_beyond_it() {
        let s = start();
        let goal = east_of(s, 31.0);
        let (lat_nm, lon_nm) = (1.0 / 60.0, 1.0 / 60.0 / 48f64.to_radians().cos());
        let at = |x: f64, y: f64| [s.lon + x * lon_nm, s.lat + y * lat_nm];
        let block = Polygon { rings: vec![vec![at(20.0, -10.0), at(30.0, -10.0), at(30.0, 10.0), at(20.0, 10.0), at(20.0, -10.0)]] };
        let b = Bounds { min_lon: s.lon - 0.5, min_lat: s.lat - 1.0, max_lon: goal.lon + 1.0, max_lat: s.lat + 1.0 };
        let land = RasterLand::from_polygons(b, 0.005, &[block], 1);
        let set = Settings { step_minutes: 30.0, ..Settings::default() };
        let ex = explore(s, goal, t0(), &simple_polar(), &ConstantWind { kn: 12.0, from: 0.0 }, &land, &set, None).unwrap();
        assert!(ex.arrived.is_some(), "stalled {} after {} fronts", ex.stalled, ex.fronts.len());
        // The last front closes in on the goal as a line, not a lone point.
        let k = ex.fronts.len() - 1;
        let longest = ex.front_runs(k).into_iter().map(|r| r.iter().filter(|&&i| ex.tree[i].node.pos.distance_nm(&goal) < 6.0).count()).max().unwrap_or(0);
        assert!(longest >= 3, "{longest} joined points near the goal");
    }

    #[test]
    fn fronts_are_not_joined_far_through_land_or_back_to_the_start() {
        let (land, s, d) = island_between();
        let ex = explore(s, d, t0(), &simple_polar(), &ConstantWind { kn: 15.0, from: 0.0 }, &land, &Settings::default(), None).unwrap();
        let mut broken = 0;
        for k in 0..ex.fronts.len() {
            assert_eq!(ex.joins[k].len(), ex.fronts[k].len().saturating_sub(1));
            let runs = ex.front_runs(k);
            assert_eq!(runs.iter().map(|r| r.len()).sum::<usize>(), ex.fronts[k].len(), "every position is in a run");
            broken += runs.len() - 1;
            for run in runs {
                for w in run.windows(2) {
                    let (a, b) = (ex.tree[w[0]].node.pos, ex.tree[w[1]].node.pos);
                    assert!(!land.blocked(&a, &b) || a.distance_nm(&b) <= 3.0, "front {k} joined through land at {a:?}");
                    // Joined neighbours are in neighbouring cells, not rays back to the start.
                    assert!(a.distance_nm(&b) <= 3.0 * Settings::default().cell_nm, "front {k} jumps {:.1} nm", a.distance_nm(&b));
                }
            }
        }
        assert!(broken > 0, "the island splits the fronts that meet it");
    }

    #[test]
    fn starting_or_ending_on_land_is_refused() {
        let (land, s, d) = island_between();
        let on_island = east_of(s, 20.0);
        let e = ConstantWind { kn: 10.0, from: 0.0 };
        assert_eq!(route(on_island, d, t0(), &simple_polar(), &e, &land, &Settings::default()).unwrap_err(), RouteError::BlockedStart);
        assert_eq!(route(s, on_island, t0(), &simple_polar(), &e, &land, &Settings::default()).unwrap_err(), RouteError::BlockedDestination);
    }

    #[test]
    fn an_end_just_inside_the_land_mask_is_joined_to_the_nearest_open_water() {
        // Half a mile inside the island's shores (a mile from open water with the
        // margin), as a river mouth is in a coarse mask.
        let (land, s, d) = island_between();
        let ashore = east_of(s, 14.5);
        let e = ConstantWind { kn: 10.0, from: 0.0 };
        let r = route(s, ashore, t0(), &simple_polar(), &e, &land, &Settings::default()).expect("routed to the open water nearby");
        let n = r.nodes.len();
        assert_eq!(r.nodes[n - 1].pos, ashore);
        assert!(r.nodes[n - 2].pos.distance_nm(&ashore) < 1.5, "arrived {} nm off", r.nodes[n - 2].pos.distance_nm(&ashore));
        assert!(r.arrival > r.nodes[n - 2].time);
        let ashore = east_of(s, 25.5);
        let back = route(ashore, d, t0(), &simple_polar(), &e, &land, &Settings::default()).expect("left from the open water nearby");
        assert_eq!(back.nodes[0].pos, ashore);
        for w in back.nodes[1..].windows(2) {
            assert!(!land.blocked(&w[0].pos, &w[1].pos), "leg through land at {:?}", w[0].pos);
        }
    }

    #[test]
    fn a_calm_stops_a_sailboat_but_not_one_with_an_engine() {
        let d = east_of(start(), 20.0);
        let calm = ConstantWind { kn: 0.0, from: 0.0 };
        let e = route(start(), d, t0(), &simple_polar(), &calm, &NoLand, &Settings::default()).unwrap_err();
        assert!(matches!(e, RouteError::NoProgress { .. }), "{e:?}");
        let s = Settings { motor_speed_kn: Some(5.0), ..Settings::default() };
        let r = route(start(), d, t0(), &simple_polar(), &calm, &NoLand, &s).unwrap();
        let hours = r.duration().num_minutes() as f64 / 60.0;
        assert!((hours - 4.0).abs() < 0.3, "{hours}");
        assert!((r.motor_hours - hours).abs() < 0.6, "all of it under engine: {} of {hours}", r.motor_hours);
        assert!(r.nodes[1].motoring);
    }

    #[test]
    fn the_engine_is_used_only_when_sailing_is_too_slow() {
        let d = east_of(start(), 20.0);
        let s = Settings { motor_speed_kn: Some(5.0), ..Settings::default() };
        let r = route(start(), d, t0(), &simple_polar(), &ConstantWind { kn: 15.0, from: 0.0 }, &NoLand, &s).unwrap();
        assert_eq!(r.motor_hours, 0.0, "a beam reach at 6 kn beats 5 under engine");
    }

    #[test]
    fn a_boat_that_gets_stuck_midway_keeps_its_fronts() {
        // Wind from the destination's side only in the first hours, then a gale.
        struct GaleAfter(DateTime<Utc>);
        impl Environment for GaleAfter {
            fn wind(&self, _: &LatLon, t: DateTime<Utc>) -> Option<(f64, f64)> {
                Some((if t < self.0 { 15.0 } else { 50.0 }, 0.0))
            }
        }
        let d = east_of(start(), 100.0);
        let env = GaleAfter(t0() + Duration::hours(3));
        let ex = explore(start(), d, t0(), &simple_polar(), &env, &NoLand, &Settings::default(), None).unwrap();
        assert!(ex.arrived.is_none() && ex.stalled);
        assert!(!ex.fronts.is_empty());
        let h = ex.horizon.unwrap();
        assert_eq!(ex.tree[*ex.fronts.last().unwrap().first().unwrap()].node.time, h);
        assert!(matches!(route(start(), d, t0(), &simple_polar(), &env, &NoLand, &Settings::default()), Err(RouteError::NoProgress { .. })));
    }

    #[test]
    fn too_much_wind_or_the_time_limit_stops_the_route() {
        let d = east_of(start(), 20.0);
        let storm = ConstantWind { kn: 50.0, from: 0.0 };
        assert!(matches!(route(start(), d, t0(), &simple_polar(), &storm, &NoLand, &Settings::default()), Err(RouteError::NoProgress { .. })));
        let s = Settings { max_hours: 1.0, ..Settings::default() };
        assert_eq!(route(start(), d, t0(), &simple_polar(), &ConstantWind { kn: 15.0, from: 0.0 }, &NoLand, &s).unwrap_err(), RouteError::TooLong);
        // Exploring keeps the fronts so far instead, as when the forecast ends.
        let ex = explore(start(), d, t0(), &simple_polar(), &ConstantWind { kn: 15.0, from: 0.0 }, &NoLand, &s, None).unwrap();
        assert!(ex.arrived.is_none() && !ex.stalled && !ex.fronts.is_empty());
        assert_eq!(ex.horizon, Some(t0() + Duration::hours(1)));
    }

    #[test]
    fn missing_weather_and_bad_settings_are_reported() {
        struct Nothing;
        impl Environment for Nothing {
            fn wind(&self, _: &LatLon, _: DateTime<Utc>) -> Option<(f64, f64)> {
                None
            }
        }
        let d = east_of(start(), 20.0);
        assert_eq!(route(start(), d, t0(), &simple_polar(), &Nothing, &NoLand, &Settings::default()).unwrap_err(), RouteError::NoWeather);
        let bad = Settings { step_minutes: 0.0, ..Settings::default() };
        assert!(matches!(route(start(), d, t0(), &simple_polar(), &ConstantWind { kn: 10.0, from: 0.0 }, &NoLand, &bad), Err(RouteError::InvalidSettings(_))));
    }

    #[test]
    fn performance_scales_the_passage_time() {
        let d = east_of(start(), 20.0);
        let env = ConstantWind { kn: 15.0, from: 0.0 };
        let slow = Settings { performance: 0.5, ..Settings::default() };
        let r = route(start(), d, t0(), &simple_polar(), &env, &NoLand, &slow).unwrap();
        let hours = r.duration().num_minutes() as f64 / 60.0;
        assert!((hours - 20.0 / 3.0).abs() < 0.4, "{hours}");
    }

    #[test]
    fn waypoints_pick_out_the_tacks() {
        let d = start().destination(0.0, 12.0);
        let r = route(start(), d, t0(), &simple_polar(), &ConstantWind { kn: 15.0, from: 0.0 }, &NoLand, &Settings::default()).unwrap();
        let w = r.waypoints(30.0, 100.0);
        assert!(w.len() >= 3 && w.len() < r.nodes.len(), "{} of {}", w.len(), r.nodes.len());
        assert_eq!(w[0], start());
        assert_eq!(*w.last().unwrap(), d);
        // A straight run collapses to its ends, unless legs are capped in time.
        let straight = route(start(), east_of(start(), 20.0), t0(), &simple_polar(), &ConstantWind { kn: 15.0, from: 0.0 }, &NoLand, &Settings::default()).unwrap();
        assert_eq!(straight.waypoints(30.0, 100.0).len(), 2);
        assert!(straight.waypoints(30.0, 1.0).len() >= 4);
    }

    #[test]
    fn it_is_deterministic() {
        let d = start().destination(0.0, 12.0);
        let env = ConstantWind { kn: 15.0, from: 0.0 };
        let a = route(start(), d, t0(), &simple_polar(), &env, &NoLand, &Settings::default()).unwrap();
        let b = route(start(), d, t0(), &simple_polar(), &env, &NoLand, &Settings::default()).unwrap();
        assert_eq!(a.arrival, b.arrival);
        assert_eq!(a.nodes.len(), b.nodes.len());
        assert!((a.distance_nm - b.distance_nm).abs() < 1e-9);
    }

    /// Uses the forecast and coastline left in the cache by the app. Run with
    /// `cargo test --release real_forecast -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_forecast_route_brest_to_penmarch() {
        use crate::chart::coast::{CoastStore, LOD_TOLERANCE_DEG};
        use crate::datadir;
        use crate::weather::cache::Cache;
        let now = Utc::now();
        let (store, n) = Cache::new(&datadir::cache_dir()).load(now, Duration::hours(72));
        // The cache can hold stores of an older schema too; take one that opens.
        let Some(coast) = std::fs::read_dir(datadir::cache_dir())
            .ok()
            .and_then(|d| d.flatten().map(|e| e.path()).filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("coast-"))).find_map(|p| CoastStore::open(&p).ok()))
        else {
            return;
        };
        if n == 0 {
            return;
        }
        let (a, b) = (LatLon::new(48.33, -4.50), LatLon::new(47.78, -4.38));
        let bounds = Bounds { min_lon: -6.0, min_lat: 47.0, max_lon: -3.0, max_lat: 49.0 };
        let polys = coast.query(&bounds, LOD_TOLERANCE_DEG.len() - 1).unwrap();
        let land = RasterLand::from_polygons(bounds, 0.002, &polys, 1);
        let (lo, hi) = store.time_range().unwrap();
        let depart = now.clamp(lo, hi);
        let started = std::time::Instant::now();
        for prune in [Prune::FarthestFromStart, Prune::ClosestToDestination] {
        let r = route(a, b, depart, &Polar::generic_cruiser(), &StoreEnv(&store), &land, &Settings { step_minutes: 15.0, prune, ..Settings::default() });
        eprintln!("{prune:?}: {:?}", r.as_ref().map(|r| (r.distance_nm as i64, r.duration().num_minutes())).map_err(|e| e.to_string()));
        }
        let r = route(a, b, depart, &Polar::generic_cruiser(), &StoreEnv(&store), &land, &Settings { step_minutes: 15.0, ..Settings::default() });
        eprintln!("computed in {:?}: {:?}", started.elapsed(), r.as_ref().map(|r| (r.distance_nm, r.duration().num_minutes(), r.nodes.len())).map_err(|e| e.to_string()));
        let r = r.expect("a route");
        for w in r.nodes.windows(2) {
            assert!(!land.blocked(&w[0].pos, &w[1].pos));
        }
        assert!(r.distance_nm >= r.direct_nm - 0.5 && r.distance_nm < r.direct_nm * 3.0);
    }

    #[test]
    fn the_store_environment_reads_wind_in_knots_from_components() {
        use crate::chart::grid::Grid;
        use crate::weather::model::{Field, Param};
        let mut store = WeatherStore::new();
        for (p, v) in [(Param::WindU, 0.0), (Param::WindV, -7.71666)] {
            store.add(Field { param: p, time: t0(), run: None, grid: Grid { ncols: 4, nrows: 4, xll: -10.0, yll: 45.0, dx: 2.0, dy: 2.0, data: vec![v; 16] } });
        }
        let (kn, from) = StoreEnv(&store).wind(&LatLon::new(48.0, -5.0), t0()).unwrap();
        // v = -7.7 m/s blows towards the south, so it is a 15 kn north wind.
        assert!((kn - 15.0).abs() < 0.01 && from.abs() < 0.01 || (from - 360.0).abs() < 0.01, "{kn} {from}");
        assert!(StoreEnv(&store).wind(&LatLon::new(10.0, 0.0), t0()).is_none());
    }
}
