//! The automatic router: the shortest path between the stops of a route that
//! keeps clear of land and of water shallower than the safety depth. It needs
//! no forecast and is not timed. Every stop is kept; waypoints are inserted
//! inside each leg where the direct line is not safe.

use super::navmask::{MaskError, MaskParams, NavMask};
use crate::chart::grid::Grid;
use crate::geo::LatLon;
use crate::vector::{Bounds, Polygon};
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::Arc;
use std::sync::atomic::{self, AtomicBool};

#[derive(Clone, Debug, PartialEq)]
pub struct AutoSettings {
    /// Water shallower than this, in metres, is avoided. Depths are from the
    /// chart datum; the tide is not added.
    pub safety_depth_m: f64,
    /// How far to keep from land, shoals and hazards.
    pub clearance_nm: f64,
    /// How far from a stop that is itself unsafe (a marina, a quay) the path
    /// may begin.
    pub snap_nm: f64,
    /// Most cells of the working grid; coarser cells are used beyond it.
    pub max_cells: u64,
}

impl Default for AutoSettings {
    fn default() -> Self {
        AutoSettings { safety_depth_m: 5.0, clearance_nm: 0.15, snap_nm: 0.5, max_cells: 6_000_000 }
    }
}

/// What the router knows about the sea. All of it is plain, `Send` data.
#[derive(Default)]
pub struct Inputs {
    pub land: Vec<Polygon>,
    pub grids: Vec<Arc<Grid>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RoutedPoint {
    pub pos: LatLon,
    /// Index of the stop this is, or `None` for a waypoint the router added.
    pub source: Option<usize>,
    /// The seamark the path rounds here, when there is one.
    pub mark: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub distance_nm: f64,
    pub direct_nm: f64,
    /// Shallowest depth the path crosses, where any is known.
    pub min_depth_m: Option<f64>,
    /// Length of path through water of unknown depth.
    pub unverified_nm: f64,
    /// Legs that start or end with a short stretch that is not itself safe.
    pub exempt_stubs: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AutoRoute {
    pub points: Vec<RoutedPoint>,
    pub report: Report,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AutoError {
    TooFewStops,
    /// The start of this leg (0-based) is not safe and no safe water is near.
    BlockedStart { leg: usize },
    BlockedEnd { leg: usize },
    NoPath { leg: usize },
    InvalidSettings(String),
    Cancelled,
}

impl std::fmt::Display for AutoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AutoError::TooFewStops => write!(f, "a route needs at least two points"),
            AutoError::BlockedStart { leg } => write!(f, "leg {} starts on land or in water that is too shallow, with no safe water close by", leg + 1),
            AutoError::BlockedEnd { leg } => write!(f, "leg {} ends on land or in water that is too shallow, with no safe water close by", leg + 1),
            AutoError::NoPath { leg } => write!(f, "no safe way through for leg {}", leg + 1),
            AutoError::InvalidSettings(s) => write!(f, "invalid settings: {s}"),
            AutoError::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::error::Error for AutoError {}

/// Routes through `stops` in order. The result starts and ends at the first and
/// last stop and contains every stop in between.
pub fn plan(stops: &[LatLon], inputs: &Inputs, s: &AutoSettings) -> Result<AutoRoute, AutoError> {
    plan_cancellable(stops, inputs, s, &AtomicBool::new(false))
}

/// `plan`, stopping with `AutoError::Cancelled` soon after `cancel` is set: between legs and
/// attempts, and every few thousand cells of a search.
pub fn plan_cancellable(stops: &[LatLon], inputs: &Inputs, s: &AutoSettings, cancel: &AtomicBool) -> Result<AutoRoute, AutoError> {
    if !(s.safety_depth_m.is_finite() && s.safety_depth_m >= 0.0) {
        return Err(AutoError::InvalidSettings("the safety depth must be zero or more".into()));
    }
    if !(s.clearance_nm.is_finite() && (0.0..=5.0).contains(&s.clearance_nm)) {
        return Err(AutoError::InvalidSettings("the clearance must be between 0 and 5 nm".into()));
    }
    if stops.len() < 2 {
        return Err(AutoError::TooFewStops);
    }
    let mut points = vec![RoutedPoint { pos: stops[0], source: Some(0), mark: None }];
    let mut report = Report::default();
    let mut min_depth: Option<f32> = None;
    for (i, w) in stops.windows(2).enumerate() {
        let leg = route_leg(w[0], w[1], i, inputs, s, cancel)?;
        points.extend(leg.inner.into_iter().map(|pos| RoutedPoint { pos, source: None, mark: None }));
        points.push(RoutedPoint { pos: w[1], source: Some(i + 1), mark: None });
        report.direct_nm += w[0].rhumb_to(&w[1]).0;
        report.unverified_nm += leg.unverified_nm;
        report.exempt_stubs += leg.exempt;
        min_depth = match (min_depth, leg.min_depth) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
    }
    report.distance_nm = points.windows(2).map(|w| w[0].pos.rhumb_to(&w[1].pos).0).sum();
    report.min_depth_m = min_depth.map(f64::from);
    Ok(AutoRoute { points, report })
}

struct Leg {
    inner: Vec<LatLon>,
    min_depth: Option<f32>,
    unverified_nm: f64,
    exempt: usize,
}

/// Why one attempt at a leg failed.
enum Fail {
    Start,
    End,
    NoPath,
    Cancelled,
}

fn route_leg(a: LatLon, b: LatLon, leg: usize, inputs: &Inputs, s: &AutoSettings, cancel: &AtomicBool) -> Result<Leg, AutoError> {
    let direct = a.distance_nm(&b);
    if direct < 1e-6 {
        return Ok(Leg { inner: Vec::new(), min_depth: None, unverified_nm: 0.0, exempt: 0 });
    }
    // The first try looks only a little way either side of the direct line; if
    // that is walled in (a peninsula, a bay) the second looks much wider.
    let mut pad = pad_nm(direct, s);
    for attempt in 0..2 {
        if cancel.load(atomic::Ordering::Relaxed) {
            return Err(AutoError::Cancelled);
        }
        match try_leg(a, b, pad, inputs, s, cancel) {
            Ok(l) => return Ok(l),
            Err(Fail::Cancelled) => return Err(AutoError::Cancelled),
            Err(Fail::NoPath) if attempt == 0 => pad *= 4.0,
            Err(Fail::NoPath) => return Err(AutoError::NoPath { leg }),
            Err(Fail::Start) => return Err(AutoError::BlockedStart { leg }),
            Err(Fail::End) => return Err(AutoError::BlockedEnd { leg }),
        }
    }
    Err(AutoError::NoPath { leg })
}

/// Water to search either side of a leg, nautical miles.
pub fn pad_nm(direct_nm: f64, s: &AutoSettings) -> f64 {
    (direct_nm * 0.25).max(3.0).max(s.clearance_nm * 6.0)
}

/// Edge of a mask cell for an area, nautical miles on the ground: a third of
/// the clearance, no finer than the area allows, coarser if it would not fit.
pub fn cell_size_nm(bounds: &Bounds, s: &AutoSettings) -> f64 {
    let mid = ((bounds.min_lat + bounds.max_lat) / 2.0).to_radians().cos().max(0.01);
    let diag = (bounds.height() * 60.0).hypot(bounds.width() * 60.0 * mid);
    let mut cell = (s.clearance_nm / 3.0).max(diag / 1500.0).max(0.01);
    while NavMask::cell_count(bounds, cell) > s.max_cells {
        cell *= 1.25;
    }
    cell
}

fn try_leg(a: LatLon, b: LatLon, pad: f64, inputs: &Inputs, s: &AutoSettings, cancel: &AtomicBool) -> Result<Leg, Fail> {
    let widest = a.lat.abs().max(b.lat.abs()).min(80.0).to_radians().cos();
    let (dlat, dlon) = (pad / 60.0, pad / 60.0 / widest);
    let bounds = Bounds {
        min_lon: a.lon.min(b.lon) - dlon,
        max_lon: a.lon.max(b.lon) + dlon,
        min_lat: (a.lat.min(b.lat) - dlat).max(-85.0),
        max_lat: (a.lat.max(b.lat) + dlat).min(85.0),
    };
    let cell_nm = cell_size_nm(&bounds, s);
    let params = MaskParams { bounds, cell_nm, safety_m: s.safety_depth_m, clearance_nm: s.clearance_nm, band_nm: s.clearance_nm.max(cell_nm * 2.0), max_cells: s.max_cells.max(NavMask::cell_count(&bounds, cell_nm)) };
    let mask = match NavMask::build(&params, &inputs.land, &inputs.grids) {
        Ok(m) => m,
        Err(MaskError::TooLarge { .. }) => return Err(Fail::NoPath),
    };
    if cancel.load(atomic::Ordering::Relaxed) {
        return Err(Fail::Cancelled);
    }
    let (ga, gb) = (mask.to_grid(&a), mask.to_grid(&b));
    let snap_cells = (s.snap_nm / cell_nm).ceil() as usize;
    let (ca, snapped_a) = resolve(&mask, &a, snap_cells).ok_or(Fail::Start)?;
    let (cb, snapped_b) = resolve(&mask, &b, snap_cells).ok_or(Fail::End)?;

    let mut chain: Vec<(f64, f64)> = if !snapped_a && !snapped_b && mask.segment_ok(ga, gb) {
        vec![ga, gb]
    } else {
        let path = astar(&mask, ca, cb, cancel)?;
        let mut chain = smooth(&mask, &path);
        // Use the exact stops where the line to the neighbour is still safe.
        if snapped_a || (chain.len() > 1 && !mask.segment_ok(ga, chain[1])) {
            chain.insert(0, ga);
        } else {
            chain[0] = ga;
        }
        let n = chain.len();
        if !snapped_b && n >= 2 && mask.segment_ok(chain[n - 2], gb) {
            chain[n - 1] = gb;
        } else {
            chain.push(gb);
        }
        chain
    };
    chain.dedup_by(|x, y| (x.0 - y.0).abs() < 1e-9 && (x.1 - y.1).abs() < 1e-9);

    // Measure what the path crosses; the stubs to a snapped stop are exempt.
    let last = chain.len() - 1;
    let (mut min_depth, mut unverified, mut exempt): (Option<f32>, f64, usize) = (None, 0.0, 0);
    for i in 0..last {
        if (i == 0 && snapped_a) || (i + 1 == last && snapped_b) {
            exempt += 1;
            continue;
        }
        mask.walk(chain[i], chain[i + 1], |c, r| {
            match mask.depth_at(c, r) {
                Some(d) => min_depth = Some(min_depth.map_or(d, |m| m.min(d))),
                None => unverified += f64::from(mask.row_nm(r)),
            }
            true
        });
    }
    let inner = chain[1..last].iter().map(|(u, v)| mask.to_latlon(*u, *v)).collect();
    Ok(Leg { inner, min_depth, unverified_nm: unverified, exempt })
}

/// The cell a path should begin in for a stop: its own when that is safe,
/// otherwise the nearest safe one within `max_cells`. The flag says it moved.
fn resolve(mask: &NavMask, p: &LatLon, max_cells: usize) -> Option<((usize, usize), bool)> {
    let (c, r) = mask.cell_at(p)?;
    if mask.navigable(c, r) {
        return Some(((c, r), false));
    }
    let (cols, rows) = (mask.cols() as i64, mask.rows() as i64);
    let mut best: Option<((usize, usize), f64)> = None;
    for d in 1..=max_cells as i64 {
        for dy in -d..=d {
            for dx in -d..=d {
                if dx.abs().max(dy.abs()) != d {
                    continue;
                }
                let (cc, rr) = (c as i64 + dx, r as i64 + dy);
                if cc < 0 || rr < 0 || cc >= cols || rr >= rows || !mask.navigable(cc as usize, rr as usize) {
                    continue;
                }
                let dist = ((dx * dx + dy * dy) as f64).sqrt();
                if best.is_none_or(|(_, b)| dist < b) {
                    best = Some(((cc as usize, rr as usize), dist));
                }
            }
        }
        // Every cell on a later ring is at least d + 1 away.
        if best.is_some_and(|(_, b)| b <= (d + 1) as f64) {
            break;
        }
    }
    best.map(|(cell, _)| (cell, true))
}

#[derive(PartialEq)]
struct Open {
    f: f32,
    g: f32,
    idx: u32,
}

impl Eq for Open {}

impl Ord for Open {
    /// Reversed, so the heap pops the lowest `f`; ties go to the lowest index
    /// so the result does not depend on insertion order.
    fn cmp(&self, o: &Self) -> Ordering {
        o.f.total_cmp(&self.f).then(o.idx.cmp(&self.idx))
    }
}

impl PartialOrd for Open {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

const STEPS: [(i64, i64); 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)];

/// A* over the navigable cells, 8-connected, never cutting a corner.
fn astar(mask: &NavMask, start: (usize, usize), goal: (usize, usize), cancel: &AtomicBool) -> Result<Vec<(usize, usize)>, Fail> {
    let (cols, rows) = (mask.cols(), mask.rows());
    let n = cols * rows;
    let mut g = vec![f32::INFINITY; n];
    let mut came = vec![u32::MAX; n];
    let min_nm = mask.min_row_nm();
    let h = |c: usize, r: usize| (c as f32 - goal.0 as f32).hypot(r as f32 - goal.1 as f32) * min_nm;
    let (si, gi) = (mask.index(start.0, start.1), mask.index(goal.0, goal.1));
    g[si] = 0.0;
    let mut open = BinaryHeap::new();
    open.push(Open { f: h(start.0, start.1), g: 0.0, idx: si as u32 });
    let mut popped = 0u32;
    while let Some(Open { g: gc, idx, .. }) = open.pop() {
        popped = popped.wrapping_add(1);
        if popped.is_multiple_of(4096) && cancel.load(atomic::Ordering::Relaxed) {
            return Err(Fail::Cancelled);
        }
        let idx = idx as usize;
        if gc > g[idx] {
            continue;
        }
        if idx == gi {
            let mut path = vec![goal];
            let mut at = idx;
            while at != si {
                at = came[at] as usize;
                path.push((at % cols, at / cols));
            }
            path.reverse();
            return Ok(path);
        }
        let (c, r) = (idx % cols, idx / cols);
        let here = mask.cost_factor(c, r);
        for (dx, dy) in STEPS {
            let (nc, nr) = (c as i64 + dx, r as i64 + dy);
            if nc < 0 || nr < 0 || nc >= cols as i64 || nr >= rows as i64 {
                continue;
            }
            let (nc, nr) = (nc as usize, nr as usize);
            if !mask.navigable(nc, nr) {
                continue;
            }
            if dx != 0 && dy != 0 && !(mask.navigable(nc, r) && mask.navigable(c, nr)) {
                continue;
            }
            let len = if dx != 0 && dy != 0 { std::f32::consts::SQRT_2 } else { 1.0 };
            let step = len * 0.5 * (mask.row_nm(r) + mask.row_nm(nr)) * 0.5 * (here + mask.cost_factor(nc, nr));
            let ni = mask.index(nc, nr);
            let ng = gc + step;
            if ng < g[ni] {
                g[ni] = ng;
                came[ni] = idx as u32;
                open.push(Open { f: ng + h(nc, nr), g: ng, idx: ni as u32 });
            }
        }
    }
    Err(Fail::NoPath)
}

/// Cuts corners: from each vertex goes as far along the path as a straight
/// line stays on navigable cells. Returns grid coordinates of cell centres.
fn smooth(mask: &NavMask, path: &[(usize, usize)]) -> Vec<(f64, f64)> {
    let pts: Vec<(f64, f64)> = path.iter().map(|(c, r)| NavMask::centre(*c, *r)).collect();
    let mut out = vec![pts[0]];
    let mut i = 0;
    while i + 1 < pts.len() {
        let mut j = i + 1;
        while j + 1 < pts.len() && mask.segment_ok(pts[i], pts[j + 1]) {
            j += 1;
        }
        out.push(pts[j]);
        i = j;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::land::{Land, RasterLand};
    use crate::routing::navmask::testutil::{fill, grid};

    fn island() -> Polygon {
        Polygon { rings: vec![vec![[-4.6, 48.2], [-4.4, 48.2], [-4.4, 48.3], [-4.6, 48.3], [-4.6, 48.2]]] }
    }

    fn west() -> LatLon {
        LatLon::new(48.25, -4.9)
    }

    fn east() -> LatLon {
        LatLon::new(48.25, -4.1)
    }

    fn with_land(land: Vec<Polygon>) -> Inputs {
        Inputs { land, grids: Vec::new() }
    }

    /// Distance in nautical miles from a point to the island's rectangle.
    fn off_island(p: &LatLon) -> f64 {
        let (dlat, dlon) = ((48.2 - p.lat).max(p.lat - 48.3).max(0.0), (-4.6 - p.lon).max(p.lon + 4.4).max(0.0));
        (dlat * 60.0).hypot(dlon * 60.0 * p.lat.to_radians().cos())
    }

    fn samples(points: &[RoutedPoint]) -> Vec<LatLon> {
        points
            .windows(2)
            .flat_map(|w| (0..=40).map(move |i| {
                let t = i as f64 / 40.0;
                LatLon::new(w[0].pos.lat + (w[1].pos.lat - w[0].pos.lat) * t, w[0].pos.lon + (w[1].pos.lon - w[0].pos.lon) * t)
            }))
            .collect()
    }

    #[test]
    fn open_water_needs_no_waypoints() {
        let r = plan(&[west(), east()], &with_land(Vec::new()), &AutoSettings::default()).unwrap();
        assert_eq!(r.points.len(), 2);
        assert_eq!(r.points[0].source, Some(0));
        assert_eq!(r.points[1].source, Some(1));
        assert!((r.report.distance_nm - r.report.direct_nm).abs() < 1e-9);
    }

    #[test]
    fn the_route_goes_round_an_island_and_keeps_its_distance() {
        let s = AutoSettings::default();
        let r = plan(&[west(), east()], &with_land(vec![island()]), &s).unwrap();
        assert!(r.points.len() >= 3, "{:?}", r.points);
        let reference = RasterLand::from_polygons(Bounds { min_lon: -5.2, min_lat: 47.9, max_lon: -3.8, max_lat: 48.6 }, 0.002, &[island()], 0);
        for w in r.points.windows(2) {
            assert!(!reference.blocked(&w[0].pos, &w[1].pos), "{:?} to {:?} crosses the island", w[0].pos, w[1].pos);
        }
        let closest = samples(&r.points).iter().map(off_island).fold(f64::INFINITY, f64::min);
        assert!(closest > s.clearance_nm * 0.7, "passes {closest} nm off");
        // A detour, not a wander.
        assert!(r.report.distance_nm > r.report.direct_nm);
        assert!(r.report.distance_nm < r.report.direct_nm * 1.25, "{} against {}", r.report.distance_nm, r.report.direct_nm);
    }

    #[test]
    fn a_shoal_is_avoided_and_a_gap_in_it_is_used() {
        let area = Bounds { min_lon: -5.2, min_lat: 47.9, max_lon: -3.8, max_lat: 48.6 };
        let mut g = grid(area, 0.005, -30.0);
        // A bar across the whole way, 2 m deep, with a 30 m channel through it.
        fill(&mut g, Bounds { min_lon: -4.55, min_lat: 47.9, max_lon: -4.45, max_lat: 48.6 }, -2.0);
        fill(&mut g, Bounds { min_lon: -4.6, min_lat: 48.30, max_lon: -4.4, max_lat: 48.34 }, -30.0);
        let inputs = Inputs { land: Vec::new(), grids: vec![Arc::new(g)] };
        let s = AutoSettings::default();
        let r = plan(&[west(), east()], &inputs, &s).unwrap();
        assert!(r.points.len() >= 3);
        let through = samples(&r.points).into_iter().find(|p| p.lon > -4.55 && p.lon < -4.45).unwrap();
        assert!(through.lat > 48.29 && through.lat < 48.35, "crossed the bar at {through:?}");
        assert!(r.report.min_depth_m.unwrap() >= 5.0, "{:?}", r.report.min_depth_m);
        assert_eq!(r.report.unverified_nm, 0.0);
        // With a safety depth of 1 m the bar is no obstacle.
        let r = plan(&[west(), east()], &inputs, &AutoSettings { safety_depth_m: 1.0, ..s }).unwrap();
        assert_eq!(r.points.len(), 2);
    }

    #[test]
    fn a_stop_in_the_clearance_zone_begins_from_safe_water() {
        // Four hundredths of a mile off the shore: not land, but inside the clearance.
        let start = LatLon::new(48.3 + 0.04 / 60.0, -4.5);
        let r = plan(&[start, LatLon::new(48.4, -4.1)], &with_land(vec![island()]), &AutoSettings::default()).unwrap();
        assert_eq!(r.report.exempt_stubs, 1);
        assert_eq!(r.points[0].pos, start);
        assert!(r.points.len() >= 3, "the stub gets its own waypoint: {:?}", r.points);
        assert!(off_island(&r.points[1].pos) >= 0.1);
    }

    #[test]
    fn a_stop_far_inland_is_refused() {
        let wide = Polygon { rings: vec![vec![[-4.9, 48.0], [-4.1, 48.0], [-4.1, 48.5], [-4.9, 48.5], [-4.9, 48.0]]] };
        let inland = LatLon::new(48.25, -4.5);
        let out = LatLon::new(48.7, -4.5);
        assert_eq!(plan(&[inland, out], &with_land(vec![wide.clone()]), &AutoSettings::default()), Err(AutoError::BlockedStart { leg: 0 }));
        assert_eq!(plan(&[out, inland], &with_land(vec![wide]), &AutoSettings::default()), Err(AutoError::BlockedEnd { leg: 0 }));
    }

    #[test]
    fn a_wall_with_no_gap_means_no_path() {
        let wall = Polygon { rings: vec![vec![[-30.0, 48.2], [30.0, 48.2], [30.0, 48.3], [-30.0, 48.3], [-30.0, 48.2]]] };
        let r = plan(&[LatLon::new(48.1, -4.5), LatLon::new(48.4, -4.5)], &with_land(vec![wall]), &AutoSettings::default());
        assert_eq!(r, Err(AutoError::NoPath { leg: 0 }));
    }

    #[test]
    fn every_stop_is_kept_and_numbered() {
        let stops = [west(), LatLon::new(48.1, -4.5), east()];
        let r = plan(&stops, &with_land(vec![island()]), &AutoSettings::default()).unwrap();
        let kept: Vec<_> = r.points.iter().filter_map(|p| p.source.map(|s| (s, p.pos))).collect();
        assert_eq!(kept, vec![(0, stops[0]), (1, stops[1]), (2, stops[2])]);
        assert_eq!(r.points[0].pos, stops[0]);
        assert_eq!(r.points.last().unwrap().pos, stops[2]);
    }

    #[test]
    fn unknown_depth_is_reported_not_refused() {
        // A grid over the west half only.
        let half = Bounds { min_lon: -5.2, min_lat: 47.9, max_lon: -4.5, max_lat: 48.6 };
        let inputs = Inputs { land: Vec::new(), grids: vec![Arc::new(grid(half, 0.005, -30.0))] };
        let r = plan(&[west(), east()], &inputs, &AutoSettings::default()).unwrap();
        assert_eq!(r.points.len(), 2);
        let direct = r.report.direct_nm;
        assert!(r.report.unverified_nm > direct * 0.3 && r.report.unverified_nm < direct * 0.7, "{} of {direct}", r.report.unverified_nm);
        assert_eq!(r.report.min_depth_m, Some(30.0));
        // No grid at all: all of it is a guess.
        let r = plan(&[west(), east()], &Inputs::default(), &AutoSettings::default()).unwrap();
        assert!((r.report.unverified_nm - r.report.direct_nm).abs() < 0.5, "{}", r.report.unverified_nm);
        assert_eq!(r.report.min_depth_m, None);
    }

    #[test]
    fn it_is_deterministic() {
        let a = plan(&[west(), east()], &with_land(vec![island()]), &AutoSettings::default()).unwrap();
        let b = plan(&[west(), east()], &with_land(vec![island()]), &AutoSettings::default()).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn bad_input_is_reported() {
        let ok = with_land(Vec::new());
        assert_eq!(plan(&[west()], &ok, &AutoSettings::default()), Err(AutoError::TooFewStops));
        assert!(matches!(plan(&[west(), east()], &ok, &AutoSettings { safety_depth_m: -1.0, ..Default::default() }), Err(AutoError::InvalidSettings(_))));
        assert!(matches!(plan(&[west(), east()], &ok, &AutoSettings { clearance_nm: 50.0, ..Default::default() }), Err(AutoError::InvalidSettings(_))));
        // Two identical stops are a leg of no length.
        assert_eq!(plan(&[west(), west()], &ok, &AutoSettings::default()).unwrap().points.len(), 2);
    }

    #[test]
    fn a_cancelled_search_stops() {
        let cancel = AtomicBool::new(true);
        let r = plan_cancellable(&[west(), east()], &with_land(vec![island()]), &AutoSettings::default(), &cancel);
        assert_eq!(r, Err(AutoError::Cancelled));
    }

    #[test]
    fn a_long_leg_gets_coarser_cells_but_still_routes() {
        let s = AutoSettings { max_cells: 200_000, ..Default::default() };
        let r = plan(&[west(), east()], &with_land(vec![island()]), &s).unwrap();
        assert!(r.points.len() >= 3);
        let bounds = Bounds { min_lon: -5.2, min_lat: 47.9, max_lon: -3.8, max_lat: 48.6 };
        assert!(NavMask::cell_count(&bounds, cell_size_nm(&bounds, &s)) <= 200_000);
        assert!(cell_size_nm(&bounds, &s) > cell_size_nm(&bounds, &AutoSettings::default()));
    }
}
