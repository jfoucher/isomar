//! What a weather route was planned with, kept with the route it made: the
//! boat's path (so its statistics can be worked out again and the passage
//! played back) and the forecast it was routed on, cut to the area it sails.

use super::isochrone::Node;
use super::land::Land;
use super::stats::signed_angle;
use crate::nav::calc::distance_to_segment_nm;
use crate::chart::grid::Grid;
use crate::geo::LatLon;
use crate::vector::Bounds;
use crate::weather::model::{Field, Param};
use crate::weather::store::WeatherStore;
use chrono::{DateTime, Duration, TimeZone, Utc};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

/// Degrees of forecast kept round the path.
const AREA_MARGIN_DEG: f64 = 2.0;
/// Hours of forecast kept before the start and after the end of the path, so
/// the fields either side of every moment of it are there.
const TIME_MARGIN_H: i64 = 6;
const VERSION: u32 = 1;

/// How far the waypoints of a saved route may stray from the path it was
/// calculated as, in nautical miles, unless the user chose otherwise
/// (`Settings::route_tolerance_nm`): well under a pixel at any zoom the passage is
/// seen at, while a straight stretch is still one leg.
pub const ROUTE_TOLERANCE_NM: f64 = 0.1;

/// The points of `nodes` to save as a route: every tack and gybe, and enough of the
/// bends between them that no part of the path is more than `tol_nm` from the route. A
/// leg that would touch `land` is split where the path strays furthest from it, so the
/// route crosses land only where the path did (which is nowhere).
pub fn route_points(nodes: &[Node], land: Option<&dyn Land>, tol_nm: f64) -> Vec<LatLon> {
    let n = nodes.len();
    if n < 3 {
        return nodes.iter().map(|x| x.pos).collect();
    }
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    // Leg `i` runs from node `i - 1`; where the wind changes side the vertex is at its start.
    let mut side: Option<f64> = None;
    for i in 1..n {
        let b = &nodes[i];
        if b.motoring || b.time <= nodes[i - 1].time {
            continue;
        }
        let angle = signed_angle(b.twd, b.heading);
        if angle.abs() > 5.0 && angle.abs() < 175.0 {
            if side.is_some_and(|before| before.signum() != angle.signum()) {
                keep[i - 1] = true;
            }
            side = Some(angle);
        }
    }
    let fixed: Vec<usize> = (0..n).filter(|&i| keep[i]).collect();
    let mut spans: Vec<(usize, usize)> = fixed.windows(2).map(|w| (w[0], w[1])).collect();
    while let Some((a, b)) = spans.pop() {
        if b - a < 2 {
            continue;
        }
        let (pa, pb) = (nodes[a].pos, nodes[b].pos);
        let (far, dev) = (a + 1..b).map(|i| (i, distance_to_segment_nm(&pa, &pb, &nodes[i].pos))).max_by(|x, y| x.1.total_cmp(&y.1)).expect("a span has a point inside");
        if dev <= tol_nm && !land.is_some_and(|l| l.blocked(&pa, &pb)) {
            continue;
        }
        keep[far] = true;
        spans.push((a, far));
        spans.push((far, b));
    }
    (0..n).filter(|&i| keep[i]).map(|i| nodes[i].pos).collect()
}

/// When a plan leaves and ends, kept beside its blob so a list of routes need not read the plan.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanTimes {
    pub depart: DateTime<Utc>,
    /// When the path ends: the arrival, or where the forecast ended when it does not arrive.
    pub end: DateTime<Utc>,
    pub arrives: bool,
}

#[derive(Clone, Debug)]
pub struct RoutePlan {
    pub depart: DateTime<Utc>,
    pub dest: LatLon,
    /// The path reaches the destination; if not it ends where the forecast did.
    pub arrives: bool,
    pub nodes: Vec<Node>,
    /// The route's points when it was saved, to tell whether it was edited since.
    pub points: Vec<LatLon>,
    pub forecast: WeatherStore,
    /// The model the forecast is from, when known (plans made before there was a choice have none).
    pub source: Option<crate::weather::source::Source>,
    /// The saved route sailed through each forecast it was routed on (`routing::along`), when there
    /// were several: the spread of its arrival.
    pub checks: Vec<PlanCheck>,
}

/// What the saved route makes of one forecast.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanCheck {
    pub source: crate::weather::source::Source,
    pub end: DateTime<Utc>,
    pub arrives: bool,
    pub hours: f64,
    pub remaining_nm: f64,
    pub max_tws_kn: f64,
    pub motor_hours: f64,
}

impl PlanCheck {
    pub fn of(source: crate::weather::source::Source, s: &super::along::Sailed) -> PlanCheck {
        PlanCheck { source, end: s.end, arrives: s.arrives, hours: s.hours(), remaining_nm: s.remaining_nm, max_tws_kn: s.max_tws_kn, motor_hours: s.motor_hours }
    }
}

#[derive(Serialize, Deserialize)]
struct WireCheck {
    model: String,
    end: i64,
    arrives: bool,
    hours: f64,
    remaining_nm: f64,
    max_tws_kn: f64,
    motor_hours: f64,
}

#[derive(Serialize, Deserialize)]
struct Wire {
    version: u32,
    depart: i64,
    dest: [f64; 2],
    arrives: bool,
    /// lat, lon, time in ms, cog, sog, heading, tws, twd, motoring (0 or 1), and where the leg met a
    /// current its east and north knots (absent otherwise, so plans without currents read as before).
    nodes: Vec<Vec<f64>>,
    points: Vec<[f64; 2]>,
    fields: Vec<WireField>,
    /// `Source::id` of the forecast; absent in plans saved before there was a choice of model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    checks: Vec<WireCheck>,
}

#[derive(Serialize, Deserialize)]
struct WireField {
    param: String,
    time: i64,
    run: Option<i64>,
    ncols: usize,
    nrows: usize,
    xll: f64,
    yll: f64,
    dx: f64,
    dy: f64,
}

fn ms(t: DateTime<Utc>) -> f64 {
    t.timestamp_millis() as f64
}

fn from_ms(v: f64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(v as i64).single().unwrap_or_default()
}

impl RoutePlan {
    /// The plan for a path, with `forecast` cut to the area and the time it covers.
    pub fn new(depart: DateTime<Utc>, dest: LatLon, arrives: bool, nodes: Vec<Node>, points: Vec<LatLon>, forecast: &WeatherStore) -> RoutePlan {
        let mut b = Bounds::EMPTY;
        for n in &nodes {
            b.extend([n.pos.lon, n.pos.lat]);
        }
        let b = b.expanded(AREA_MARGIN_DEG);
        let (from, to) = match (nodes.first(), nodes.last()) {
            (Some(a), Some(z)) => (a.time - Duration::hours(TIME_MARGIN_H), z.time + Duration::hours(TIME_MARGIN_H)),
            _ => (depart, depart),
        };
        RoutePlan { depart, dest, arrives, nodes, points, forecast: forecast.clipped(&b, from, to), source: None, checks: Vec::new() }
    }

    /// When the path ends.
    pub fn end(&self) -> DateTime<Utc> {
        self.nodes.last().map_or(self.depart, |n| n.time)
    }

    pub fn times(&self) -> PlanTimes {
        PlanTimes { depart: self.depart, end: self.end(), arrives: self.arrives }
    }

    /// The model run the forecast came from.
    pub fn run(&self) -> Option<DateTime<Utc>> {
        self.forecast.latest_run()
    }

    /// Whether `points` are still the ones the plan was made for.
    pub fn matches(&self, points: &[LatLon]) -> bool {
        self.points.len() == points.len() && self.points.iter().zip(points).all(|(a, b)| (a.lat - b.lat).abs() < 1e-9 && (a.lon - b.lon).abs() < 1e-9)
    }

    /// Compressed bytes for the database: a JSON header, then the forecast's
    /// values as little-endian f32.
    pub fn encode(&self) -> Vec<u8> {
        let fields: Vec<&Field> = self.forecast.fields().collect();
        let wire = Wire {
            version: VERSION,
            depart: self.depart.timestamp(),
            dest: [self.dest.lat, self.dest.lon],
            arrives: self.arrives,
            nodes: self
                .nodes
                .iter()
                .map(|n| {
                    let mut v = vec![n.pos.lat, n.pos.lon, ms(n.time), n.cog, n.sog_kn, n.heading, n.tws_kn, n.twd, n.motoring as u8 as f64];
                    if n.current != (0.0, 0.0) {
                        v.extend([n.current.0, n.current.1]);
                    }
                    v
                })
                .collect(),
            points: self.points.iter().map(|p| [p.lat, p.lon]).collect(),
            fields: fields
                .iter()
                .map(|f| WireField {
                    param: f.param.id().to_string(),
                    time: f.time.timestamp(),
                    run: f.run.map(|r| r.timestamp()),
                    ncols: f.grid.ncols,
                    nrows: f.grid.nrows,
                    xll: f.grid.xll,
                    yll: f.grid.yll,
                    dx: f.grid.dx,
                    dy: f.grid.dy,
                })
                .collect(),
            model: self.source.map(|s| s.id().to_string()),
            checks: self
                .checks
                .iter()
                .map(|c| WireCheck { model: c.source.id().to_string(), end: c.end.timestamp(), arrives: c.arrives, hours: c.hours, remaining_nm: c.remaining_nm, max_tws_kn: c.max_tws_kn, motor_hours: c.motor_hours })
                .collect(),
        };
        let head = serde_json::to_vec(&wire).unwrap_or_default();
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        let _ = gz.write_all(&(head.len() as u32).to_le_bytes());
        let _ = gz.write_all(&head);
        for f in fields {
            for v in &f.grid.data {
                let _ = gz.write_all(&v.to_le_bytes());
            }
        }
        gz.finish().unwrap_or_default()
    }

    pub fn decode(bytes: &[u8]) -> Option<RoutePlan> {
        let mut raw = Vec::new();
        GzDecoder::new(bytes).read_to_end(&mut raw).ok()?;
        let len = u32::from_le_bytes(raw.get(..4)?.try_into().ok()?) as usize;
        let wire: Wire = serde_json::from_slice(raw.get(4..4 + len)?).ok()?;
        if wire.version != VERSION {
            return None;
        }
        let mut at = 4 + len;
        let mut forecast = WeatherStore::new();
        for f in &wire.fields {
            let n = f.ncols.checked_mul(f.nrows)?;
            let chunk = raw.get(at..at.checked_add(n.checked_mul(4)?)?)?;
            at += n * 4;
            let data = chunk.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect();
            forecast.add(Field {
                param: Param::from_id(&f.param)?,
                time: Utc.timestamp_opt(f.time, 0).single()?,
                run: f.run.and_then(|r| Utc.timestamp_opt(r, 0).single()),
                grid: Grid { ncols: f.ncols, nrows: f.nrows, xll: f.xll, yll: f.yll, dx: f.dx, dy: f.dy, data },
            });
        }
        let nodes: Vec<Node> = wire
            .nodes
            .iter()
            .filter(|n| n.len() >= 9)
            .map(|n| Node { pos: LatLon::new(n[0], n[1]), time: from_ms(n[2]), cog: n[3], sog_kn: n[4], heading: n[5], tws_kn: n[6], twd: n[7], motoring: n[8] != 0.0, current: if n.len() >= 11 { (n[9], n[10]) } else { (0.0, 0.0) } })
            .collect();
        if nodes.is_empty() {
            return None;
        }
        Some(RoutePlan {
            depart: Utc.timestamp_opt(wire.depart, 0).single()?,
            dest: LatLon::new(wire.dest[0], wire.dest[1]),
            arrives: wire.arrives,
            nodes,
            points: wire.points.iter().map(|p| LatLon::new(p[0], p[1])).collect(),
            forecast,
            source: wire.model.as_deref().and_then(crate::weather::source::Source::from_id),
            checks: wire
                .checks
                .iter()
                .filter_map(|c| {
                    Some(PlanCheck {
                        source: crate::weather::source::Source::from_id(&c.model)?,
                        end: Utc.timestamp_opt(c.end, 0).single()?,
                        arrives: c.arrives,
                        hours: c.hours,
                        remaining_nm: c.remaining_nm,
                        max_tws_kn: c.max_tws_kn,
                        motor_hours: c.motor_hours,
                    })
                })
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, 6, 0, 0).unwrap()
    }

    fn node(lat: f64, lon: f64, h: i64) -> Node {
        Node { pos: LatLon::new(lat, lon), time: t0() + Duration::hours(h), cog: 10.0, sog_kn: 5.5, heading: 12.0, tws_kn: 14.0, twd: 300.0, motoring: h == 2, current: if h == 1 { (0.8, -1.2) } else { (0.0, 0.0) } }
    }

    /// A wind field every three hours over a 10 x 10 degree area, from a run at `t0`.
    fn forecast() -> WeatherStore {
        let mut s = WeatherStore::new();
        for h in (0..=48).step_by(3) {
            for (p, v) in [(Param::WindU, 3.0), (Param::WindV, -4.0)] {
                let data = (0..100).map(|i| v + i as f32 * 0.01).collect();
                s.add(Field { param: p, time: t0() + Duration::hours(h), run: Some(t0()), grid: Grid { ncols: 10, nrows: 10, xll: -10.0, yll: 40.0, dx: 1.0, dy: 1.0, data } });
            }
        }
        s
    }

    fn plan() -> RoutePlan {
        let nodes = vec![node(45.0, -5.0, 0), node(45.2, -5.1, 1), node(45.4, -5.1, 2), node(45.6, -5.0, 3)];
        RoutePlan::new(t0(), LatLon::new(45.6, -5.0), true, nodes, vec![LatLon::new(45.0, -5.0), LatLon::new(45.6, -5.0)], &forecast())
    }

    #[test]
    fn a_plan_saved_before_currents_reads_with_none() {
        // Nine values a node, as plans were written before the router knew of currents.
        let head = br#"{"version":1,"depart":0,"dest":[45.6,-5.0],"arrives":true,"nodes":[[45.0,-5.0,0,10,5.5,12,14,300,0],[45.6,-5.0,3600000,10,5.5,12,14,300,1]],"points":[[45.0,-5.0],[45.6,-5.0]],"fields":[]}"#;
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(&(head.len() as u32).to_le_bytes()).unwrap();
        gz.write_all(head).unwrap();
        let q = RoutePlan::decode(&gz.finish().unwrap()).expect("decodes");
        assert_eq!(q.nodes.len(), 2);
        assert!(q.nodes.iter().all(|n| n.current == (0.0, 0.0)) && q.nodes[1].motoring);
    }

    #[test]
    fn a_plan_comes_back_as_it_went_in() {
        let p = plan();
        let q = RoutePlan::decode(&p.encode()).expect("decodes");
        assert_eq!((q.depart, q.dest, q.arrives), (p.depart, p.dest, p.arrives));
        assert_eq!(q.nodes.len(), 4);
        for (a, b) in p.nodes.iter().zip(&q.nodes) {
            assert_eq!((a.pos, a.time, a.cog, a.sog_kn, a.heading, a.tws_kn, a.twd, a.motoring, a.current), (b.pos, b.time, b.cog, b.sog_kn, b.heading, b.tws_kn, b.twd, b.motoring, b.current));
        }
        assert_eq!(q.nodes[1].current, (0.8, -1.2), "a leg's current is kept");
        assert_eq!(q.points, p.points);
        assert_eq!(q.forecast.field_count(), p.forecast.field_count());
        assert_eq!(q.run(), Some(t0()));
        // The same statistics: they are read off the nodes.
        let (a, b) = (super::super::stats::Stats::of(&p.nodes), super::super::stats::Stats::of(&q.nodes));
        assert_eq!((a.hours, a.distance_nm, a.motor_hours), (b.hours, b.distance_nm, b.motor_hours));
        assert_eq!(q.source, None, "a plan of no known model");
        let mut p = p;
        p.source = Some(crate::weather::source::Source::Aifs);
        p.checks = vec![PlanCheck { source: crate::weather::source::Source::Gfs, end: t0() + Duration::hours(30), arrives: true, hours: 30.0, remaining_nm: 0.0, max_tws_kn: 22.0, motor_hours: 1.5 }];
        let q = RoutePlan::decode(&p.encode()).unwrap();
        assert_eq!((q.source, &q.checks), (p.source, &p.checks));
    }

    #[test]
    fn the_forecast_is_cut_to_the_path_and_its_time() {
        let p = plan();
        let (lo, hi) = p.forecast.time_range().unwrap();
        assert_eq!((lo, hi), (t0(), t0() + Duration::hours(9)), "three hours of path, six either side");
        let g = &p.forecast.nearest(Param::WindU, t0()).unwrap().grid;
        assert!(g.ncols < 10 && g.nrows < 10, "{}x{}", g.ncols, g.nrows);
    }

    #[test]
    fn a_cut_forecast_samples_as_the_whole_one_did() {
        let whole = forecast();
        let p = plan();
        for (lat, lon, h) in [(45.0, -5.0, 0), (45.3, -5.05, 2), (45.6, -4.9, 3)] {
            let t = t0() + Duration::hours(h);
            assert_eq!(p.forecast.wind(lat, lon, t), whole.wind(lat, lon, t), "{lat} {lon} {h}");
        }
    }

    #[test]
    fn a_plan_knows_when_its_route_was_edited() {
        let p = plan();
        assert!(p.matches(&p.points));
        assert!(!p.matches(&p.points[..1]));
        assert!(!p.matches(&[LatLon::new(45.0, -5.0), LatLon::new(45.7, -5.0)]));
    }

    #[test]
    fn garbage_is_not_a_plan() {
        assert!(RoutePlan::decode(b"nonsense").is_none());
        let mut bytes = plan().encode();
        bytes.truncate(bytes.len() / 2);
        assert!(RoutePlan::decode(&bytes).is_none());
    }

    /// A path of `legs` legs that bears `a` degrees off north on the first
    /// half of each pair and `b` on the second, to tack on.
    fn zigzag(legs: usize, a: f64, b: f64) -> Vec<Node> {
        let mut out = vec![node(45.0, -5.0, 0)];
        for i in 0..legs {
            let last = out.last().unwrap().clone();
            let cog = if i % 2 == 0 { a } else { b };
            let mut n = last.clone();
            n.pos = last.pos.destination(cog, 4.0);
            n.time = last.time + Duration::hours(1);
            n.cog = cog;
            n.heading = cog;
            n.twd = 0.0;
            n.motoring = false;
            out.push(n);
        }
        out
    }

    /// The farthest any node of `path` is from the polyline through `points`.
    fn stray(path: &[Node], points: &[LatLon]) -> f64 {
        path.iter().map(|n| points.windows(2).map(|w| distance_to_segment_nm(&w[0], &w[1], &n.pos)).fold(f64::INFINITY, f64::min)).fold(0.0, f64::max)
    }

    #[test]
    fn a_straight_path_is_one_leg_and_a_zigzag_keeps_every_turn() {
        let straight = zigzag(20, 30.0, 30.0);
        assert_eq!(route_points(&straight, None, ROUTE_TOLERANCE_NM).len(), 2);
        // The wind on the other bow every hour: a tack at each node.
        let tacking = zigzag(20, 40.0, 320.0);
        let pts = route_points(&tacking, None, ROUTE_TOLERANCE_NM);
        assert_eq!(pts.len(), tacking.len(), "every vertex of a path that turns that much stays");
        assert!(stray(&tacking, &pts) < 1e-6);
    }

    #[test]
    fn a_gentle_curve_stays_within_the_tolerance_of_the_path() {
        // 1.5 degrees more each leg: no turn is sharp, but the sum is.
        let mut path = vec![node(45.0, -5.0, 0)];
        for i in 0..60 {
            let last = path.last().unwrap().clone();
            let cog = 1.5 * i as f64;
            let mut n = last.clone();
            n.pos = last.pos.destination(cog, 4.0);
            n.time = last.time + Duration::hours(1);
            n.cog = cog;
            n.heading = cog;
            path.push(n);
        }
        let pts = route_points(&path, None, ROUTE_TOLERANCE_NM);
        assert!(stray(&path, &pts) <= ROUTE_TOLERANCE_NM + 1e-9, "{}", stray(&path, &pts));
        assert!(pts.len() > 8 && pts.len() < path.len(), "{} of {}", pts.len(), path.len());
        // What the old rule gave: turns of 20 degrees or more, so none of this curve.
        assert!(super::super::isochrone::waypoints_of(&path, 20.0, 6.0).len() < pts.len());
    }

    #[test]
    fn a_gybe_is_kept_even_when_it_barely_changes_the_course() {
        // Running with the wind from astern, 170 degrees off the bow one way, then the other.
        let mut path = zigzag(10, 0.0, 0.0);
        for (i, n) in path.iter_mut().enumerate().skip(1) {
            n.twd = if i <= 5 { 170.0 } else { 190.0 };
            n.heading = 0.0;
        }
        // Same bearing all along, so only the wind side tells where the gybe was.
        let pts = route_points(&path, None, 5.0);
        assert!(pts.contains(&path[5].pos), "the gybe vertex is a waypoint");
    }

    /// Land that blocks any leg whose straight line passes within `r` nm of `at`.
    struct Rock {
        at: LatLon,
        r: f64,
    }

    impl Land for Rock {
        fn blocked(&self, a: &LatLon, b: &LatLon) -> bool {
            distance_to_segment_nm(a, b, &self.at) < self.r
        }
    }

    #[test]
    fn a_leg_that_would_cross_land_is_split_even_within_the_tolerance() {
        // A motored path (no tacks to keep) that bends a little round a rock: the chord cuts the rock, the path does not.
        let mut path = vec![node(45.0, -5.0, 0)];
        for (i, brg) in [10.0, 10.0, 10.0, 10.0, 350.0, 350.0, 350.0, 350.0].iter().enumerate() {
            let last = path.last().unwrap().clone();
            let mut n = last.clone();
            n.pos = last.pos.destination(*brg, 5.0);
            n.time = last.time + Duration::hours(1);
            n.cog = *brg;
            n.heading = *brg;
            n.motoring = true;
            path.push(n);
            let _ = i;
        }
        let land = Rock { at: path[0].pos.destination(0.0, path[0].pos.distance_nm(&path[8].pos) / 2.0), r: 0.3 };
        assert!(!path.windows(2).any(|w| land.blocked(&w[0].pos, &w[1].pos)), "the path itself clears it");
        let chord = [path[0].pos, path[8].pos];
        assert!(land.blocked(&chord[0], &chord[1]), "the straight way does not");
        let without = route_points(&path, None, 5.0);
        let with = route_points(&path, Some(&land), 5.0);
        assert_eq!(without.len(), 2, "a wide tolerance takes the shortcut");
        assert!(with.len() > 2);
        assert!(!with.windows(2).any(|w| land.blocked(&w[0], &w[1])), "no leg of the route touches the rock");
    }
}
