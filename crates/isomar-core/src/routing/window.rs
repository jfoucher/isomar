//! The weather window: when to leave for a passage.
//!
//! The router is run once for each departure in a window (every 3 hours,
//! say), and each passage that arrives gets a cost in hours: its length, plus
//! for every hour of something the crew would rather avoid (beating, the
//! engine, strong wind, gusts, big seas, the dark) that hour again times the
//! weight they give it, plus a cost per tack or gybe and per hour of waiting
//! for the departure. The cheapest passage is the best window. With every
//! weight but time at zero, the shortest passage wins.
//!
//! A passage the forecast ends before is followed as far as it goes, to the
//! position of the last front nearest the destination by water, and costed
//! as if the rest of the way cost what the way made good did, mile for mile
//! (`Passage::estimated`): so a passage longer than the forecast still has
//! a best departure.

use super::alternatives::{self, Alternative};
use super::isochrone::{self, Environment, Node, Progress, RouteError, Settings};
use super::land::Land;
use super::polar::Polar;
use super::stats::Stats;
use crate::geo::LatLon;
use chrono::{DateTime, Duration, Utc};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// The largest weight a slider gives: an hour of it counts as five.
pub const MAX_WEIGHT: f64 = 4.0;

/// Hours a tack or gybe costs at a weight of 1.
const HOURS_PER_MANOEUVRE: f64 = 0.5;

/// A passage the forecast ends before is costed only if it made at least
/// this share of the way good: from less, the estimate says little.
pub const MIN_SHARE_MADE: f64 = 0.05;

/// How much each thing matters, in hours of passage per hour of it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Weights {
    /// Per hour at sea.
    pub time: f64,
    /// Per hour close-hauled.
    pub upwind: f64,
    /// Per hour under engine.
    pub engine: f64,
    /// Per hour with the wind at or above `strong_wind_kn`.
    pub strong_wind: f64,
    pub strong_wind_kn: f64,
    /// Per hour with gusts at or above `gust_kn`.
    pub gusts: f64,
    pub gust_kn: f64,
    /// Per hour with the significant wave height at or above `wave_m`.
    pub waves: f64,
    pub wave_m: f64,
    /// Per hour in the dark.
    pub night: f64,
    /// Per tack or gybe, in half hours.
    pub manoeuvres: f64,
    /// Per hour between the first departure tried and this one.
    pub waiting: f64,
    /// Per hour waiting under way for the stream to turn (off a headland, it is no rest).
    pub holding: f64,
    /// Per hour of wind against a strong tide (`stats::wind_against_tide`): a short, steep sea.
    pub wind_against_tide: f64,
}

impl Default for Weights {
    fn default() -> Self {
        Weights::comfortable()
    }
}

impl Weights {
    /// Only the time at sea counts.
    pub fn fastest() -> Weights {
        Weights { time: 1.0, upwind: 0.0, engine: 0.0, strong_wind: 0.0, gusts: 0.0, waves: 0.0, night: 0.0, manoeuvres: 0.0, waiting: 0.0, holding: 0.0, wind_against_tide: 0.0, ..Weights::comfortable() }
    }

    /// Off the wind, in moderate wind and sea, by day where it can be.
    pub fn comfortable() -> Weights {
        Weights {
            time: 1.0,
            upwind: 1.5,
            engine: 0.5,
            strong_wind: 2.0,
            strong_wind_kn: 20.0,
            gusts: 1.0,
            gust_kn: 30.0,
            waves: 1.5,
            wave_m: 2.5,
            night: 0.5,
            manoeuvres: 0.5,
            waiting: 0.0,
            holding: 0.5,
            wind_against_tide: 1.0,
        }
    }

    /// Under sail as much as possible.
    pub fn sailing() -> Weights {
        Weights { upwind: 0.5, engine: 3.0, strong_wind: 1.0, strong_wind_kn: 25.0, gusts: 0.5, waves: 0.5, night: 0.0, manoeuvres: 0.0, ..Weights::comfortable() }
    }
}

/// What a passage is like, in the units the weights apply to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Measures {
    pub hours: f64,
    pub upwind_hours: f64,
    pub engine_hours: f64,
    pub strong_wind_hours: f64,
    /// `None` when the forecast has no gusts (or waves) along the way.
    pub gust_hours: Option<f64>,
    pub max_gust_kn: Option<f64>,
    pub wave_hours: Option<f64>,
    pub max_wave_m: Option<f64>,
    pub night_hours: f64,
    pub manoeuvres: u32,
    pub waiting_hours: f64,
    pub hold_hours: f64,
    pub wind_against_tide_hours: f64,
}

/// Measures a path. Gusts and waves are read from `env` at the middle of
/// each leg; the wind is the one the router sailed with.
pub fn measure(path: &[Node], stats: &Stats, env: &dyn Environment, w: &Weights, earliest: DateTime<Utc>) -> Measures {
    let mut m = Measures {
        hours: stats.hours,
        upwind_hours: stats.sail_hours[0],
        engine_hours: stats.motor_hours,
        night_hours: stats.dark_hours,
        manoeuvres: stats.tacks + stats.gybes,
        waiting_hours: path.first().map_or(0.0, |n| hours(n.time - earliest).max(0.0)),
        hold_hours: stats.hold_hours,
        wind_against_tide_hours: stats.wind_against_tide_hours,
        ..Measures::default()
    };
    for leg in path.windows(2) {
        let (a, b) = (&leg[0], &leg[1]);
        let h = hours(b.time - a.time);
        if h <= 0.0 {
            continue;
        }
        if b.tws_kn >= w.strong_wind_kn {
            m.strong_wind_hours += h;
        }
        let mid = a.pos.destination(a.pos.bearing_to(&b.pos), a.pos.distance_nm(&b.pos) / 2.0);
        let t = a.time + (b.time - a.time) / 2;
        if let Some(g) = env.gust(&mid, t) {
            *m.gust_hours.get_or_insert(0.0) += if g >= w.gust_kn { h } else { 0.0 };
            m.max_gust_kn = Some(m.max_gust_kn.map_or(g, |x| x.max(g)));
        }
        if let Some(hs) = env.wave_height(&mid, t) {
            *m.wave_hours.get_or_insert(0.0) += if hs >= w.wave_m { h } else { 0.0 };
            m.max_wave_m = Some(m.max_wave_m.map_or(hs, |x| x.max(hs)));
        }
    }
    m
}

fn hours(d: Duration) -> f64 {
    d.num_milliseconds() as f64 / 3_600_000.0
}

/// One part of the cost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Term {
    Time,
    Upwind,
    Engine,
    StrongWind,
    Gusts,
    Waves,
    Night,
    Manoeuvres,
    Waiting,
    Holding,
    WindAgainstTide,
}

/// Each part of the cost: how much of it there is (hours, or a count for
/// manoeuvres) and what it costs, in hours.
pub fn terms(m: &Measures, w: &Weights) -> [(Term, f64, f64); 11] {
    let n = m.manoeuvres as f64;
    let gusts = m.gust_hours.unwrap_or(0.0);
    let waves = m.wave_hours.unwrap_or(0.0);
    [
        (Term::Time, m.hours, w.time * m.hours),
        (Term::Upwind, m.upwind_hours, w.upwind * m.upwind_hours),
        (Term::Engine, m.engine_hours, w.engine * m.engine_hours),
        (Term::StrongWind, m.strong_wind_hours, w.strong_wind * m.strong_wind_hours),
        (Term::Gusts, gusts, w.gusts * gusts),
        (Term::Waves, waves, w.waves * waves),
        (Term::Night, m.night_hours, w.night * m.night_hours),
        (Term::Manoeuvres, n, w.manoeuvres * HOURS_PER_MANOEUVRE * n),
        (Term::Waiting, m.waiting_hours, w.waiting * m.waiting_hours),
        (Term::Holding, m.hold_hours, w.holding * m.hold_hours),
        (Term::WindAgainstTide, m.wind_against_tide_hours, w.wind_against_tide * m.wind_against_tide_hours),
    ]
}

/// The cost of a passage, in hours.
pub fn cost(m: &Measures, w: &Weights) -> f64 {
    terms(m, w).iter().map(|t| t.2).sum()
}

/// The departures to try: from `earliest`, every `step_hours`, for
/// `window_hours`, and none after `latest`.
pub fn departures(earliest: DateTime<Utc>, window_hours: f64, step_hours: f64, latest: Option<DateTime<Utc>>) -> Vec<DateTime<Utc>> {
    if step_hours <= 0.0 || window_hours < 0.0 {
        return Vec::new();
    }
    let step = Duration::milliseconds((step_hours * 3_600_000.0) as i64);
    let end = earliest + Duration::milliseconds((window_hours * 3_600_000.0) as i64);
    let end = latest.map_or(end, |l| end.min(l));
    std::iter::successors(Some(earliest), |t| Some(*t + step)).take_while(|t| *t <= end).collect()
}

/// Why a departure has no passage.
#[derive(Clone, Debug, PartialEq)]
pub enum Failure {
    /// The forecast (or the time allowed) ends before the boat has made
    /// `MIN_SHARE_MADE` of the way good.
    ForecastEnds,
    /// Calm, too much wind or land: no way forward.
    Stalled,
    Route(RouteError),
}

/// A passage that arrived, or as far as it went before the forecast ended.
#[derive(Clone, Debug)]
pub struct Passage {
    /// Ends at the destination (`alt.arrives`), or where the forecast ends.
    pub alt: Alternative,
    /// Of the way sailed.
    pub stats: Stats,
    /// Of the way sailed.
    pub measures: Measures,
    /// The whole passage's cost: for one that does not arrive, the way made
    /// good's cost (waiting aside) scaled to the whole way.
    pub cost: f64,
    /// Set when the forecast ended first: how far it still was, by water.
    pub estimated: Option<Estimate>,
}

/// How much of a passage the forecast covered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Estimate {
    /// From where the forecast ends, by water.
    pub remaining_nm: f64,
    /// Of the way, by water, 0 to 1.
    pub share_made: f64,
}

impl Passage {
    pub fn arrives(&self) -> bool {
        self.estimated.is_none()
    }
}

/// The cost of a whole passage of which `share` was made good for
/// `measures`: the waiting is paid once, the rest for every mile.
pub fn estimated_cost(measures: &Measures, w: &Weights, share: f64) -> f64 {
    let waiting = w.waiting * measures.waiting_hours;
    (cost(measures, w) - waiting) / share.max(MIN_SHARE_MADE) + waiting
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub depart: DateTime<Utc>,
    pub result: Result<Passage, Failure>,
}

impl Candidate {
    pub fn passage(&self) -> Option<&Passage> {
        self.result.as_ref().ok()
    }
}

/// Every departure tried, in order.
#[derive(Clone, Debug)]
pub struct Scan {
    pub candidates: Vec<Candidate>,
    pub weights: Weights,
    /// The searches of the cheapest few departures (`KEPT_SEARCHES`), by index into `candidates`: the
    /// best one is previewed as soon as the results arrive, and searching it again took seconds.
    pub kept: Vec<(usize, Arc<isochrone::Exploration>)>,
}

/// How many of the cheapest departures' searches a scan keeps: an ocean passage's is tens of megabytes.
pub const KEPT_SEARCHES: usize = 3;

/// Where a tidal stream decides the passage, the best departures are tried again closer together: the
/// `REFINE` passes, each round the `top` cheapest so far at ± `hours`. Only when the stream along the way
/// reaches `REFINE_PEAK_KN`.
pub const REFINE: [(usize, f64); 2] = [(3, 1.0), (1, 0.5)];
pub const REFINE_PEAK_KN: f64 = 1.0;

/// The departures to try round the `top` cheapest of `departures` (by `costs`, one each), `hours` either
/// side, that were not tried, between `earliest` and `latest`, in order.
pub fn around_best(departures: &[DateTime<Utc>], costs: &[Option<f64>], top: usize, hours: f64, earliest: DateTime<Utc>, latest: Option<DateTime<Utc>>) -> Vec<DateTime<Utc>> {
    let step = Duration::milliseconds((hours * 3_600_000.0) as i64);
    let mut out: Vec<DateTime<Utc>> = ranked_by(costs)
        .into_iter()
        .take(top)
        .filter_map(|i| departures.get(i))
        .flat_map(|d| [*d - step, *d + step])
        .filter(|t| *t >= earliest && latest.is_none_or(|l| *t <= l))
        .filter(|t| !departures.iter().any(|d| (*d - *t).num_minutes().abs() < 5))
        .collect();
    out.sort();
    out.dedup();
    out
}

impl Scan {
    /// This scan and `other`'s departures together, in time order, with the cheapest searches of both
    /// kept.
    pub fn merge(self, other: Scan) -> Scan {
        let n = self.candidates.len();
        let mut all: Vec<(Candidate, Option<Arc<isochrone::Exploration>>)> = Vec::with_capacity(n + other.candidates.len());
        let kept_of = |kept: &[(usize, Arc<isochrone::Exploration>)], i: usize| kept.iter().find(|(k, _)| *k == i).map(|(_, e)| e.clone());
        for (i, c) in self.candidates.into_iter().enumerate() {
            let k = kept_of(&self.kept, i);
            all.push((c, k));
        }
        for (i, c) in other.candidates.into_iter().enumerate() {
            let k = kept_of(&other.kept, i);
            all.push((c, k));
        }
        all.sort_by_key(|(c, _)| c.depart);
        let mut kept: Vec<(f64, usize, Arc<isochrone::Exploration>)> = all.iter().enumerate().filter_map(|(i, (c, k))| Some((c.passage()?.cost, i, k.clone()?))).collect();
        kept.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        kept.truncate(KEPT_SEARCHES);
        Scan { candidates: all.into_iter().map(|(c, _)| c).collect(), weights: self.weights, kept: kept.into_iter().map(|(_, i, e)| (i, e)).collect() }
    }

    /// The search of departure `i`, when the scan kept it.
    pub fn search_of(&self, i: usize) -> Option<Arc<isochrone::Exploration>> {
        self.kept.iter().find(|(k, _)| *k == i).map(|(_, ex)| ex.clone())
    }
}

impl Scan {
    /// Indices of the departures that have a passage (that arrive, or are
    /// estimated), cheapest first (the earlier of two that cost the same).
    pub fn ranked(&self) -> Vec<usize> {
        let mut out: Vec<usize> = (0..self.candidates.len()).filter(|&i| self.candidates[i].passage().is_some()).collect();
        out.sort_by(|&a, &b| {
            let (ca, cb) = (self.candidates[a].passage().map_or(0.0, |p| p.cost), self.candidates[b].passage().map_or(0.0, |p| p.cost));
            ca.total_cmp(&cb).then(a.cmp(&b))
        });
        out
    }

    /// The lowest and highest cost of the passages.
    pub fn cost_range(&self) -> Option<(f64, f64)> {
        let costs = self.candidates.iter().filter_map(|c| c.passage().map(|p| p.cost));
        costs.fold(None, |r, c| Some(r.map_or((c, c), |(lo, hi): (f64, f64)| (lo.min(c), hi.max(c)))))
    }
}

/// The cost of each departure over several forecast models' scans of the same departures: the
/// average of the models' costs, or with `cautious` the worst of them. A departure one model gives no
/// passage for has none.
pub fn combined_costs(scans: &[&Scan], cautious: bool) -> Vec<Option<f64>> {
    let n = scans.first().map_or(0, |s| s.candidates.len());
    (0..n)
        .map(|i| {
            let costs: Option<Vec<f64>> = scans.iter().map(|s| s.candidates.get(i).and_then(|c| c.passage()).map(|p| p.cost)).collect();
            let costs = costs.filter(|c| !c.is_empty())?;
            Some(if cautious { costs.iter().copied().fold(f64::MIN, f64::max) } else { costs.iter().sum::<f64>() / costs.len() as f64 })
        })
        .collect()
}

/// Indices of the departures that have a cost, cheapest first (the earlier of two that cost the same).
pub fn ranked_by(costs: &[Option<f64>]) -> Vec<usize> {
    let mut out: Vec<usize> = (0..costs.len()).filter(|&i| costs[i].is_some()).collect();
    out.sort_by(|&a, &b| costs[a].unwrap_or(0.0).total_cmp(&costs[b].unwrap_or(0.0)).then(a.cmp(&b)));
    out
}

/// Lets another thread follow and cancel a scan: one router progress per
/// departure.
pub struct ScanProgress {
    runs: Vec<Progress>,
    finished: Vec<AtomicBool>,
}

impl ScanProgress {
    pub fn new(departures: usize) -> ScanProgress {
        ScanProgress { runs: (0..departures).map(|_| Progress::default()).collect(), finished: (0..departures).map(|_| AtomicBool::new(false)).collect() }
    }

    pub fn cancel(&self) {
        for r in &self.runs {
            r.cancel.store(true, Ordering::Relaxed);
        }
    }

    fn cancelled(&self) -> bool {
        self.runs.first().is_some_and(|r| r.cancel.load(Ordering::Relaxed))
    }

    pub fn total(&self) -> usize {
        self.runs.len()
    }

    pub fn done(&self) -> usize {
        self.finished.iter().filter(|f| f.load(Ordering::Relaxed)).count()
    }

    /// How much of the scan is done, 0 to 1, counting the searches under way.
    pub fn fraction(&self) -> f64 {
        if self.runs.is_empty() {
            return 1.0;
        }
        let part = |(r, f): (&Progress, &AtomicBool)| {
            if f.load(Ordering::Relaxed) {
                return 1.0;
            }
            let steps = r.steps.load(Ordering::Relaxed);
            if steps == 0 { 0.0 } else { (r.step.load(Ordering::Relaxed) as f64 / steps as f64).min(1.0) }
        };
        self.runs.iter().zip(&self.finished).map(part).sum::<f64>() / self.runs.len() as f64
    }
}

/// Routes every departure (several at once) and costs the passages that
/// arrive. `progress`, if given, must be for as many departures.
#[allow(clippy::too_many_arguments)]
pub fn scan(start: LatLon, dest: LatLon, departures: &[DateTime<Utc>], polar: &Polar, env: &dyn Environment, land: &dyn Land, s: &Settings, w: &Weights, progress: Option<&ScanProgress>) -> Result<Scan, RouteError> {
    let Some(&earliest) = departures.first() else { return Ok(Scan { candidates: Vec::new(), weights: w.clone(), kept: Vec::new() }) };
    scan_since(earliest, start, dest, departures, polar, env, land, s, w, progress)
}

/// `scan`, with the waiting counted from `earliest` rather than the first of `departures`: for the
/// departures a refinement adds between those tried (`around_best`).
#[allow(clippy::too_many_arguments)]
pub fn scan_since(earliest: DateTime<Utc>, start: LatLon, dest: LatLon, departures: &[DateTime<Utc>], polar: &Polar, env: &dyn Environment, land: &dyn Land, s: &Settings, w: &Weights, progress: Option<&ScanProgress>) -> Result<Scan, RouteError> {
    let own;
    let progress = match progress {
        Some(p) => p,
        None => {
            own = ScanProgress::new(departures.len());
            &own
        }
    };
    assert_eq!(progress.total(), departures.len(), "one progress per departure");
    if departures.is_empty() {
        return Ok(Scan { candidates: Vec::new(), weights: w.clone(), kept: Vec::new() });
    }
    // The cheapest searches so far, by cost; the others are dropped as soon as they are costed.
    let kept: Mutex<Vec<(f64, usize, Arc<isochrone::Exploration>)>> = Mutex::new(Vec::new());
    let keep = |i: usize, cost: f64, ex: isochrone::Exploration| {
        let mut k = kept.lock().unwrap_or_else(|e| e.into_inner());
        if k.len() < KEPT_SEARCHES || k.iter().any(|(c, _, _)| cost < *c) {
            k.push((cost, i, Arc::new(ex)));
            k.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            k.truncate(KEPT_SEARCHES);
        }
    };
    let candidates: Vec<Candidate> = departures
        .par_iter()
        .enumerate()
        .map(|(i, &depart)| {
            let result = if progress.cancelled() {
                Err(Failure::Route(RouteError::Cancelled))
            } else {
                match isochrone::explore(start, dest, depart, polar, env, land, s, Some(&progress.runs[i])) {
                    Ok(ex) => {
                        let passage = passage_of(&ex, env, w, earliest);
                        if let Ok(p) = &passage {
                            keep(i, p.cost, ex);
                        }
                        passage
                    }
                    Err(e) => Err(Failure::Route(e)),
                }
            };
            progress.finished[i].store(true, Ordering::Relaxed);
            Candidate { depart, result }
        })
        .collect();
    if progress.cancelled() {
        return Err(RouteError::Cancelled);
    }
    let kept = kept.into_inner().unwrap_or_else(|e| e.into_inner()).into_iter().map(|(_, i, ex)| (i, ex)).collect();
    Ok(Scan { candidates, weights: w.clone(), kept })
}

/// The passage a search found: the one that arrived, else the way to the
/// last front's position nearest the destination, estimated.
fn passage_of(ex: &isochrone::Exploration, env: &dyn Environment, w: &Weights, earliest: DateTime<Utc>) -> Result<Passage, Failure> {
    let costed = |alt: Alternative| {
        let stats = alt.stats();
        let measures = measure(&alt.path, &stats, env, w, earliest);
        (alt, stats, measures)
    };
    if let Some(alt) = alternatives::arrival(ex) {
        let (alt, stats, measures) = costed(alt);
        return Ok(Passage { cost: cost(&measures, w), alt, stats, measures, estimated: None });
    }
    if ex.stalled {
        return Err(Failure::Stalled);
    }
    let Some(n) = ex.nearest.filter(|n| n.share_made() >= MIN_SHARE_MADE) else { return Err(Failure::ForecastEnds) };
    let (alt, stats, measures) = costed(alternatives::to_node(ex, n.index));
    let share = n.share_made();
    Ok(Passage { cost: estimated_cost(&measures, w, share), alt, stats, measures, estimated: Some(Estimate { remaining_nm: n.remaining_nm, share_made: share }) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::isochrone::ConstantWind;
    use crate::routing::land::NoLand;
    use chrono::TimeZone;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, 6, 0, 0).unwrap()
    }

    fn at(h: f64) -> DateTime<Utc> {
        t0() + Duration::milliseconds((h * 3_600_000.0) as i64)
    }

    /// 6 kn at any angle of 60 degrees or more, 5 kn close-hauled at 45
    /// (the best way to windward), nothing within 40 degrees.
    fn polar() -> Polar {
        Polar::parse("TWA;3;40\n0;0;0\n40;0;0\n45;5;5\n60;6;6\n90;6;6\n180;6;6\n").unwrap()
    }

    fn start() -> LatLon {
        LatLon::new(48.0, -5.0)
    }

    /// 24 nm east: four hours on a beam reach.
    fn dest() -> LatLon {
        start().destination(90.0, 24.0)
    }

    fn only(f: impl FnOnce(&mut Weights)) -> Weights {
        let mut w = Weights { time: 0.0, ..Weights::fastest() };
        f(&mut w);
        w
    }

    fn run(env: &dyn Environment, deps: &[DateTime<Utc>], w: &Weights) -> Scan {
        scan(start(), dest(), deps, &polar(), env, &NoLand, &Settings { step_minutes: 15.0, ..Settings::default() }, w, None).unwrap()
    }

    #[test]
    fn departures_follow_the_step_and_stop_at_the_window_or_the_latest() {
        let d = departures(t0(), 12.0, 3.0, None);
        assert_eq!(d, vec![at(0.0), at(3.0), at(6.0), at(9.0), at(12.0)]);
        assert_eq!(departures(t0(), 12.0, 3.0, Some(at(7.0))).len(), 3);
        assert_eq!(departures(t0(), 0.0, 3.0, None), vec![t0()]);
        assert!(departures(t0(), 12.0, 0.0, None).is_empty());
        assert!(departures(t0(), 12.0, 3.0, Some(at(-1.0))).is_empty());
    }

    #[test]
    fn with_only_time_weighted_the_cost_is_the_passage_hours() {
        let m = Measures { hours: 10.0, upwind_hours: 4.0, engine_hours: 2.0, strong_wind_hours: 1.0, night_hours: 5.0, manoeuvres: 3, waiting_hours: 6.0, ..Measures::default() };
        assert_eq!(cost(&m, &Weights::fastest()), 10.0);
        // Each weight only ever adds.
        let base = cost(&m, &Weights::comfortable());
        for f in [|w: &mut Weights| w.upwind += 1.0, |w: &mut Weights| w.engine += 1.0, |w: &mut Weights| w.night += 1.0, |w: &mut Weights| w.manoeuvres += 1.0, |w: &mut Weights| w.waiting += 1.0] {
            let mut w = Weights::comfortable();
            f(&mut w);
            assert!(cost(&m, &w) > base);
        }
        assert_eq!(cost(&m, &only(|w| w.manoeuvres = 1.0)), 1.5);
        // No gust or wave data costs nothing.
        assert_eq!(cost(&m, &only(|w| { w.gusts = 4.0; w.waves = 4.0 })), 0.0);
    }

    /// From the east (dead ahead) until `turn` hours, then from the north.
    struct Veering {
        turn: f64,
    }

    impl Environment for Veering {
        fn wind(&self, _: &LatLon, t: DateTime<Utc>) -> Option<(f64, f64)> {
            Some((15.0, if t < at(self.turn) { 90.0 } else { 0.0 }))
        }
    }

    #[test]
    fn weighting_upwind_waits_for_the_wind_to_free() {
        let env = Veering { turn: 6.0 };
        let deps = departures(t0(), 6.0, 3.0, None);
        // Counting the wait as well, leaving at once arrives first, beating.
        let soon = run(&env, &deps, &only(|w| { w.time = 1.0; w.waiting = 1.0 }));
        let first = soon.ranked()[0];
        assert_eq!(soon.candidates[first].depart, at(0.0));
        assert!(soon.candidates[first].passage().unwrap().measures.upwind_hours > 2.0);
        // Hating the beat, it is worth waiting for the wind to veer.
        let w = only(|w| { w.time = 1.0; w.waiting = 1.0; w.upwind = 2.0 });
        let free = run(&env, &deps, &w);
        let best = &free.candidates[free.ranked()[0]];
        assert_eq!(best.depart, at(6.0));
        assert!(best.passage().unwrap().measures.upwind_hours < 0.1);
        assert_eq!(free.weights, w);
    }

    /// 15 kn from the north, 30 between `from` and `to` hours.
    struct Gale {
        from: f64,
        to: f64,
    }

    impl Environment for Gale {
        fn wind(&self, _: &LatLon, t: DateTime<Utc>) -> Option<(f64, f64)> {
            Some((if t >= at(self.from) && t < at(self.to) { 30.0 } else { 15.0 }, 0.0))
        }
    }

    #[test]
    fn strong_wind_pushes_the_departures_through_a_gale_down() {
        let deps = departures(t0(), 18.0, 3.0, None);
        let s = run(&Gale { from: 6.0, to: 12.0 }, &deps, &only(|w| { w.time = 1.0; w.strong_wind = 2.0; w.strong_wind_kn = 20.0 }));
        let ranked = s.ranked();
        assert_eq!(ranked.len(), deps.len());
        // Leaving at 0, 12, 15 or 18 misses the gale; 3, 6 and 9 do not.
        for &i in &ranked[..4] {
            assert!(s.candidates[i].passage().unwrap().measures.strong_wind_hours < 0.1, "{:?}", s.candidates[i].depart);
        }
        for &i in &ranked[4..] {
            assert!(s.candidates[i].passage().unwrap().measures.strong_wind_hours > 0.9, "{:?}", s.candidates[i].depart);
        }
        let (lo, hi) = s.cost_range().unwrap();
        assert!(hi > lo + 1.0);
    }

    #[test]
    fn waiting_favours_the_first_departure_when_all_else_is_equal() {
        let deps = departures(t0(), 9.0, 3.0, None);
        let env = ConstantWind { kn: 15.0, from: 0.0 };
        let s = run(&env, &deps, &only(|w| { w.time = 1.0; w.waiting = 0.5 }));
        assert_eq!(s.ranked(), vec![0, 1, 2, 3]);
        let m = &s.candidates[3].passage().unwrap().measures;
        assert!((m.waiting_hours - 9.0).abs() < 1e-6);
        assert!((m.hours - 4.0).abs() < 0.3, "{}", m.hours);
    }

    /// Steady wind with gusts and waves that grow after `rough` hours, and a
    /// forecast that ends at `end`.
    struct Rough {
        rough: f64,
        end: f64,
    }

    impl Environment for Rough {
        fn wind(&self, _: &LatLon, t: DateTime<Utc>) -> Option<(f64, f64)> {
            (t <= at(self.end)).then_some((15.0, 0.0))
        }

        fn gust(&self, _: &LatLon, t: DateTime<Utc>) -> Option<f64> {
            Some(if t >= at(self.rough) { 35.0 } else { 20.0 })
        }

        fn wave_height(&self, _: &LatLon, t: DateTime<Utc>) -> Option<f64> {
            Some(if t >= at(self.rough) { 3.0 } else { 1.0 })
        }

        fn horizon(&self) -> Option<DateTime<Utc>> {
            Some(at(self.end))
        }
    }

    #[test]
    fn gusts_and_waves_are_read_along_the_way() {
        let env = Rough { rough: 2.0, end: 48.0 };
        let s = run(&env, &[t0()], &Weights::comfortable());
        let m = &s.candidates[0].passage().unwrap().measures;
        // Rough for the last two of four hours.
        assert!((m.gust_hours.unwrap() - 2.0).abs() < 0.3, "{m:?}");
        assert!((m.wave_hours.unwrap() - 2.0).abs() < 0.3, "{m:?}");
        assert_eq!(m.max_gust_kn, Some(35.0));
        assert_eq!(m.max_wave_m, Some(3.0));
        // A forecast without them says so rather than calling it calm.
        let calm = run(&ConstantWind { kn: 15.0, from: 0.0 }, &[t0()], &Weights::comfortable());
        let m = &calm.candidates[0].passage().unwrap().measures;
        assert_eq!((m.gust_hours, m.wave_hours), (None, None));
    }

    #[test]
    fn departures_the_forecast_ends_before_are_estimated_from_the_way_made() {
        // Four hours of passage; leaving at 8 the forecast ends half way.
        let env = Rough { rough: 100.0, end: 10.0 };
        let w = Weights::fastest();
        let s = run(&env, &[at(0.0), at(3.0), at(8.0)], &w);
        assert!(s.candidates[..2].iter().all(|c| c.passage().unwrap().arrives()));
        let late = s.candidates[2].passage().expect("estimated");
        let e = late.estimated.expect("the forecast ended first");
        assert!(!late.alt.arrives && late.alt.end <= at(10.5), "{:?}", late.alt.end);
        assert!((0.3..0.7).contains(&e.share_made), "{e:?}");
        assert!((e.remaining_nm - dest().distance_nm(&late.alt.path.last().unwrap().pos)).abs() < 1.0, "{e:?}");
        // Sailed at the same speed, the whole way costs about what a passage that arrives does.
        let whole = s.candidates[0].passage().unwrap().cost;
        assert!((late.cost - whole).abs() < 0.6, "{} against {whole}", late.cost);
        assert!(late.measures.hours < whole * 0.75);
        assert_eq!(s.ranked().len(), 3);
        // A start in no weather at all is the router's error.
        let s = run(&env, &[at(11.0)], &Weights::comfortable());
        assert_eq!(s.candidates[0].result.as_ref().unwrap_err(), &Failure::Route(RouteError::NoWeather));
        assert!(s.ranked().is_empty() && s.cost_range().is_none());
    }

    #[test]
    fn a_passage_the_forecast_covers_too_little_of_is_not_estimated() {
        // Ten hours at 6 kn: 60 nm of 600 is enough to judge, of 2000 not.
        let env = Rough { rough: 100.0, end: 10.0 };
        let far = |nm: f64| scan(start(), start().destination(90.0, nm), &[t0()], &polar(), &env, &NoLand, &Settings { step_minutes: 30.0, ..Settings::default() }, &Weights::fastest(), None).unwrap();
        let p = far(600.0);
        let e = p.candidates[0].passage().expect("estimated").estimated.unwrap();
        assert!((0.08..0.12).contains(&e.share_made), "{e:?}");
        assert_eq!(far(2000.0).candidates[0].result.as_ref().unwrap_err(), &Failure::ForecastEnds);
    }

    #[test]
    fn an_estimate_pays_the_waiting_once() {
        let m = Measures { hours: 10.0, waiting_hours: 6.0, ..Measures::default() };
        let w = only(|w| { w.time = 1.0; w.waiting = 1.0 });
        assert!((estimated_cost(&m, &w, 0.5) - 26.0).abs() < 1e-9);
        assert!((estimated_cost(&m, &w, 1.0) - cost(&m, &w)).abs() < 1e-9);
    }

    #[test]
    fn a_calm_is_a_stall() {
        let s = run(&ConstantWind { kn: 0.0, from: 0.0 }, &[t0()], &Weights::comfortable());
        assert!(matches!(s.candidates[0].result, Err(Failure::Stalled) | Err(Failure::Route(RouteError::NoProgress { .. }))), "{:?}", s.candidates[0].result);
    }

    #[test]
    fn cancelling_stops_the_scan() {
        let deps = departures(t0(), 9.0, 3.0, None);
        let p = ScanProgress::new(deps.len());
        p.cancel();
        let r = scan(start(), dest(), &deps, &polar(), &ConstantWind { kn: 15.0, from: 0.0 }, &NoLand, &Settings::default(), &Weights::default(), Some(&p));
        assert_eq!(r.unwrap_err(), RouteError::Cancelled);
        assert_eq!(p.done(), deps.len());
        assert_eq!(p.fraction(), 1.0);
    }

    #[test]
    fn several_models_rank_by_their_average_or_their_worst() {
        // One model sees a gale between 6 and 12 hours, the other a steady wind.
        let w = only(|w| { w.time = 1.0; w.strong_wind = 2.0; w.strong_wind_kn = 20.0 });
        let deps = departures(t0(), 18.0, 3.0, None);
        let a = run(&ConstantWind { kn: 15.0, from: 0.0 }, &deps, &w);
        let b = run(&Gale { from: 6.0, to: 12.0 }, &deps, &w);
        let typical = combined_costs(&[&a, &b], false);
        let cautious = combined_costs(&[&a, &b], true);
        for i in 0..deps.len() {
            let (ca, cb) = (a.candidates[i].passage().map(|p| p.cost), b.candidates[i].passage().map(|p| p.cost));
            match (ca, cb) {
                (Some(x), Some(y)) => {
                    assert!((typical[i].unwrap() - (x + y) / 2.0).abs() < 1e-9);
                    assert_eq!(cautious[i], Some(x.max(y)));
                }
                _ => assert!(typical[i].is_none() && cautious[i].is_none(), "a departure one model cannot sail has no cost"),
            }
        }
        let r = ranked_by(&cautious);
        assert!(r.windows(2).all(|w| cautious[w[0]] <= cautious[w[1]]));
        assert_eq!(combined_costs(&[&a], true), a.candidates.iter().map(|c| c.passage().map(|p| p.cost)).collect::<Vec<_>>(), "one model is its own cost");
    }

    #[test]
    fn an_empty_window_is_an_empty_scan() {
        let s = run(&ConstantWind { kn: 15.0, from: 0.0 }, &[], &Weights::default());
        assert!(s.candidates.is_empty());
    }
}
