//! Turning "this area, these parameters, this long" into the smallest set of
//! requests that delivers it, with an honest size estimate.
//!
//! Each forecast model (`Source`) is planned its own way. GFS (atmosphere and waves) comes through
//! the NOMADS grib filter, which crops on the server: one small file a step. ECMWF's IFS and AIFS
//! publish one whole-globe file a step with an index of where each field starts; a job names the
//! fields it wants, they are read by byte range and cropped to the area before they are cached.

use super::model::{Group, Param};
pub use super::source::Source;
use super::store::WeatherStore;
use crate::geo::LatLon;
use crate::vector::Bounds;
use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc};

/// Grid spacing of the GFS 0.25 degree products.
pub const GFS_RES: f64 = 0.25;
/// Longest forecast we ask for.
pub const GFS_MAX_HOURS: u32 = 384;

const NOMADS: &str = "https://nomads.ncep.noaa.gov/cgi-bin";

/// A rectangle of the Earth, in degrees. West may be greater than east when it
/// crosses the antimeridian.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub south: f64,
    pub west: f64,
    pub north: f64,
    pub east: f64,
}

impl Area {
    /// A box around a point, `half_deg` to each side.
    pub fn around(lat: f64, lon: f64, half_deg: f64) -> Area {
        Area::new(lat - half_deg, lon - half_deg, lat + half_deg, lon + half_deg)
    }

    pub fn new(south: f64, west: f64, north: f64, east: f64) -> Area {
        let (s, n) = (south.min(north).clamp(-90.0, 90.0), south.max(north).clamp(-90.0, 90.0));
        Area { south: s, west: wrap(west), north: n, east: wrap(east) }
    }

    pub fn from_bounds(b: &Bounds) -> Area {
        Area::new(b.min_lat, b.min_lon, b.max_lat, b.max_lon)
    }

    /// The smallest box on a lattice of `step` degrees that holds this one, so
    /// that requests for nearly the same place come out the same.
    pub fn outward(&self, step: f64) -> Area {
        Area {
            south: ((self.south / step).floor() * step).max(-90.0),
            north: ((self.north / step).ceil() * step).min(90.0),
            west: ((self.west / step).floor() * step).max(-180.0),
            east: ((self.east / step).ceil() * step).min(180.0),
        }
    }

    pub fn width_deg(&self) -> f64 {
        let w = (self.east - self.west).rem_euclid(360.0);
        // A full-width box wraps to zero; treat it as the whole globe.
        if w == 0.0 && self.east != self.west { 360.0 } else { w }
    }

    pub fn height_deg(&self) -> f64 {
        self.north - self.south
    }

    /// Widened outwards to the model grid, so nothing at the edge is lost.
    pub fn snapped(&self, res: f64) -> Area {
        let w = (self.west / res).floor() * res;
        let e = (self.east / res).ceil() * res;
        Area {
            south: ((self.south / res).floor() * res).max(-90.0),
            north: ((self.north / res).ceil() * res).min(90.0),
            west: wrap(w),
            east: wrap(e),
        }
    }

    /// Grid points of a model with spacing `res` inside the area.
    pub fn grid_points(&self, res: f64) -> (u32, u32) {
        let a = self.snapped(res);
        let ni = (a.width_deg() / res).round() as u32 + 1;
        let nj = (a.height_deg() / res).round() as u32 + 1;
        (ni, nj)
    }

    pub fn is_valid(&self) -> bool {
        self.height_deg() > 0.0 && self.width_deg() > 0.0 && self.west.is_finite()
    }
}

fn wrap(lon: f64) -> f64 {
    let l = (lon + 180.0).rem_euclid(360.0) - 180.0;
    if l == -180.0 && lon > 0.0 { 180.0 } else { l }
}

/// One product of a forecast model: its atmosphere, or its waves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Model {
    Gfs,
    GfsWave,
    Ifs,
    IfsWave,
    Aifs,
    AifsWave,
    IconEu,
    /// NOAA's ensemble, 31 runs of GFS from slightly different starts: not a forecast to show, but the
    /// spread of what may happen (`routing::ensemble`).
    Gefs,
}

impl Model {
    pub const ALL: [Model; 8] = [Model::Gfs, Model::GfsWave, Model::Ifs, Model::IfsWave, Model::Aifs, Model::AifsWave, Model::IconEu, Model::Gefs];

    /// Also the name of its folder in the cache.
    pub fn id(self) -> &'static str {
        match self {
            Model::Gfs => "gfs",
            Model::GfsWave => "gfswave",
            Model::Ifs => "ifs",
            Model::IfsWave => "ifswave",
            Model::Aifs => "aifs",
            Model::AifsWave => "aifswave",
            Model::IconEu => "iconeu",
            Model::Gefs => "gefs",
        }
    }

    pub fn from_id(s: &str) -> Option<Model> {
        Model::ALL.into_iter().find(|m| m.id() == s)
    }

    pub fn label(self) -> &'static str {
        match self {
            Model::Gfs => "NOAA GFS 0.25°",
            Model::GfsWave => "NOAA GFS-Wave 0.25°",
            Model::Ifs => "ECMWF IFS 0.25°",
            Model::IfsWave => "ECMWF IFS waves 0.25°",
            Model::Aifs => "ECMWF AIFS 0.25°",
            Model::AifsWave => "ECMWF AIFS waves 0.25°",
            Model::IconEu => "DWD ICON-EU 0.0625°",
            Model::Gefs => "NOAA GEFS 0.5° ensemble",
        }
    }

    /// The forecast model it is part of (the ensemble counts as NOAA's, but is never loaded as GFS).
    pub fn source(self) -> Source {
        match self {
            Model::Gfs | Model::GfsWave | Model::Gefs => Source::Gfs,
            Model::Ifs | Model::IfsWave => Source::Ifs,
            Model::Aifs | Model::AifsWave => Source::Aifs,
            Model::IconEu => Source::IconEu,
        }
    }

    /// Whether the model covers `area` (all of them are global).
    pub fn covers(self, _area: &Area) -> bool {
        true
    }
}

/// How a job's file is had.
#[derive(Clone, Debug, PartialEq)]
pub enum Fetch {
    /// The job's URL answers with the file, already cut to the area (the NOMADS grib filter).
    Whole,
    /// The job's URL is a whole-globe file with an index at `index` (ECMWF's JSON lines, one a
    /// field, with `_offset` and `_length`). The surface fields named in `params` are read by byte
    /// range and cropped to `crop` before they are cached.
    Ranges { index: String, params: Vec<&'static str>, crop: Area },
    /// Whole files, one a field (bz2-compressed when the name says so), cropped to `crop` before they
    /// are cached together as the job's file: DWD's open data.
    Files { urls: Vec<String>, crop: Area },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    /// The forecast model to ask.
    pub source: Source,
    pub area: Area,
    pub groups: Vec<Group>,
    /// How far ahead of now to cover, in hours.
    pub horizon_hours: u32,
    /// Interval between time steps, in hours.
    pub step_hours: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub model: Model,
    pub run: DateTime<Utc>,
    pub fhour: u32,
    pub url: String,
    /// File name in the cache, unique for this exact query.
    pub cache_name: String,
    pub est_bytes: u64,
    pub params: Vec<Param>,
    pub valid: DateTime<Utc>,
    pub fetch: Fetch,
    /// The ensemble member the file is of: 0 the control run, then the perturbed ones.
    pub member: Option<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub run: DateTime<Utc>,
    pub jobs: Vec<Job>,
    pub est_bytes: u64,
    pub description: String,
    /// Groups no available source can provide for this request.
    pub unavailable: Vec<Group>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    pub fn models(&self) -> Vec<Model> {
        let mut m: Vec<Model> = Vec::new();
        for j in &self.jobs {
            if !m.contains(&j.model) {
                m.push(j.model);
            }
        }
        m
    }
}

/// GFS runs to try, newest first. A run is offered once enough of it should
/// have been published, which is about 4.5 hours after its nominal time.
pub fn candidate_runs(now: DateTime<Utc>) -> Vec<DateTime<Utc>> {
    Source::Gfs.candidate_runs(now)
}

/// GFS forecast hours to fetch for a run so that `from` to `from + horizon` is
/// covered at about `step` hours, using only hours GFS actually publishes.
pub fn forecast_hours(run: DateTime<Utc>, from: DateTime<Utc>, horizon_hours: u32, step_hours: u32) -> Vec<u32> {
    hours_of(Source::Gfs, run, from, horizon_hours, step_hours)
}

/// The forecast hours of `source`'s run to fetch so that `from` to `from + horizon` is covered at
/// about `step` hours, from the hours the run publishes: the first at or before `from` (so the first
/// frame can be interpolated), then the first published at or after each step, to one step past the
/// end to bracket it.
pub fn hours_of(source: Source, run: DateTime<Utc>, from: DateTime<Utc>, horizon_hours: u32, step_hours: u32) -> Vec<u32> {
    let published = source.published_hours(run);
    let step = step_hours.max(1);
    let start = (from - run).num_hours().max(0) as u32;
    let start = start - start % step;
    let end = start + horizon_hours + step;
    let mut v: Vec<u32> = Vec::new();
    let Some(&first) = published.iter().rev().find(|h| **h <= start).or(published.first()) else { return v };
    v.push(first);
    let mut h = start + step;
    while h <= end {
        let Some(&next) = published.iter().find(|p| **p >= h) else { break };
        if v.last() != Some(&next) {
            v.push(next);
        }
        h += step;
    }
    v
}

fn fnv(s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// Per-field, per-step cost model: simple packing is about 1.6 bytes a point,
/// plus a message header.
pub fn estimate_bytes(points: u32, fields: usize, steps: usize) -> u64 {
    ((points as f64 * 1.6 + 150.0) * fields as f64 * steps as f64) as u64
}

fn atmos_query(groups: &[Group]) -> (Vec<&'static str>, Vec<&'static str>, Vec<Param>) {
    let (mut vars, mut levels, mut params) = (Vec::new(), Vec::new(), Vec::new());
    let mut add = |v: &[&'static str], l: &'static str, p: &[Param]| {
        vars.extend_from_slice(v);
        if !levels.contains(&l) {
            levels.push(l);
        }
        params.extend_from_slice(p);
    };
    for g in groups {
        match g {
            Group::Wind => add(&["UGRD", "VGRD"], "lev_10_m_above_ground", &[Param::WindU, Param::WindV]),
            Group::Gust => add(&["GUST"], "lev_surface", &[Param::Gust]),
            Group::Pressure => add(&["PRMSL"], "lev_mean_sea_level", &[Param::Pressure]),
            Group::Temperature => add(&["TMP"], "lev_2_m_above_ground", &[Param::Temperature]),
            Group::Cloud => add(&["TCDC"], "lev_entire_atmosphere", &[Param::CloudCover]),
            Group::Waves | Group::Current => {}
        }
    }
    (vars, levels, params)
}

fn area_query(a: &Area) -> String {
    // The filter wants a left/right range; a box crossing the antimeridian is
    // expressed with right > 180.
    let (left, mut right) = (a.west, a.east);
    if right < left {
        right += 360.0;
    }
    format!("subregion=&leftlon={left}&rightlon={right}&toplat={}&bottomlat={}", a.north, a.south)
}

fn gfs_job(run: DateTime<Utc>, fhour: u32, groups: &[Group], area: &Area) -> Option<Job> {
    let (vars, levels, params) = atmos_query(groups);
    if vars.is_empty() {
        return None;
    }
    let date = run.format("%Y%m%d");
    let hh = run.format("%H");
    let mut q = format!("dir=%2Fgfs.{date}%2F{hh}%2Fatmos&file=gfs.t{hh}z.pgrb2.0p25.f{fhour:03}");
    for v in &vars {
        q.push_str(&format!("&var_{v}=on"));
    }
    for l in &levels {
        q.push_str(&format!("&{l}=on"));
    }
    q.push('&');
    q.push_str(&area_query(area));
    let (ni, nj) = area.grid_points(GFS_RES);
    Some(Job {
        model: Model::Gfs,
        run,
        fhour,
        cache_name: format!("gfs_{}_f{fhour:03}_{:08x}.grib2", run.format("%Y%m%d%H"), fnv(&q)),
        est_bytes: estimate_bytes(ni * nj, params.len(), 1),
        url: format!("{NOMADS}/filter_gfs_0p25.pl?{q}"),
        params,
        valid: run + Duration::hours(fhour as i64),
        fetch: Fetch::Whole,
        member: None,
    })
}

/// The size of one whole-globe field at 0.25°, in bytes, as published on 2026-11-05 (ECMWF packs
/// each parameter at its own precision).
fn ecmwf_field_bytes(source: Source, param: &str) -> u64 {
    let ifs = source == Source::Ifs;
    match param {
        "10u" | "10v" => if ifs { 737_000 } else { 597_000 },
        "10fg" => 648_000,
        "msl" => if ifs { 521_000 } else { 485_000 },
        "2t" => if ifs { 321_000 } else { 249_000 },
        "swh" => if ifs { 393_000 } else { 368_000 },
        "mwd" => if ifs { 489_000 } else { 449_000 },
        "mwp" => if ifs { 435_000 } else { 403_000 },
        _ => 600_000,
    }
}

/// The index of one step of the IFS: about 40 KB.
const ECMWF_INDEX_BYTES: u64 = 40_000;

const ECMWF: &str = "https://data.ecmwf.int/forecasts";

/// The ECMWF fields (short names) and our parameters for the groups asked for, of the atmosphere
/// or of the waves.
fn ecmwf_fields(source: Source, groups: &[Group], waves: bool) -> (Vec<&'static str>, Vec<Param>) {
    let (mut names, mut params) = (Vec::new(), Vec::new());
    for g in groups.iter().filter(|g| source.has(**g)) {
        let (n, p): (&[&'static str], &[Param]) = match (g, waves) {
            (Group::Wind, false) => (&["10u", "10v"], &[Param::WindU, Param::WindV]),
            (Group::Gust, false) => (&["10fg"], &[Param::Gust]),
            (Group::Pressure, false) => (&["msl"], &[Param::Pressure]),
            (Group::Temperature, false) => (&["2t"], &[Param::Temperature]),
            (Group::Waves, true) => (&["swh", "mwp", "mwd"], &[Param::WaveHeight, Param::WavePeriod, Param::WaveDir]),
            _ => continue,
        };
        names.extend_from_slice(n);
        params.extend_from_slice(p);
    }
    (names, params)
}

/// One step of an ECMWF model: its atmosphere, or its waves.
fn ecmwf_job(source: Source, run: DateTime<Utc>, fhour: u32, groups: &[Group], area: &Area, waves: bool) -> Option<Job> {
    let (names, params) = ecmwf_fields(source, groups, waves);
    if names.is_empty() {
        return None;
    }
    let (model, dir) = match (source, waves) {
        (Source::Ifs, false) => (Model::Ifs, "ifs"),
        (Source::Ifs, true) => (Model::IfsWave, "ifs"),
        (Source::Aifs, false) => (Model::Aifs, "aifs-single"),
        (Source::Aifs, true) => (Model::AifsWave, "aifs-single"),
        (Source::Gfs | Source::IconEu, _) => return None,
    };
    let stream = if waves { "wave" } else { "oper" };
    let base = format!("{ECMWF}/{}/{}z/{dir}/0p25/{stream}/{}0000-{fhour}h-{stream}-fc", run.format("%Y%m%d"), run.format("%H"), run.format("%Y%m%d%H"));
    let key = format!("{} {} {} {} {}", names.join(","), area.south, area.west, area.north, area.east);
    Some(Job {
        model,
        run,
        fhour,
        cache_name: format!("{}_{}_f{fhour:03}_{:08x}.grib2", model.id(), run.format("%Y%m%d%H"), fnv(&key)),
        est_bytes: ECMWF_INDEX_BYTES + names.iter().map(|n| ecmwf_field_bytes(source, n)).sum::<u64>(),
        url: format!("{base}.grib2"),
        params,
        valid: run + Duration::hours(fhour as i64),
        fetch: Fetch::Ranges { index: format!("{base}.index"), params: names, crop: *area },
        member: None,
    })
}

fn wave_job(run: DateTime<Utc>, fhour: u32, area: &Area) -> Job {
    let date = run.format("%Y%m%d");
    let hh = run.format("%H");
    let mut q = format!("dir=%2Fgfs.{date}%2F{hh}%2Fwave%2Fgridded&file=gfswave.t{hh}z.global.0p25.f{fhour:03}.grib2");
    for v in ["HTSGW", "PERPW", "DIRPW"] {
        q.push_str(&format!("&var_{v}=on"));
    }
    q.push_str("&lev_surface=on&");
    q.push_str(&area_query(area));
    let (ni, nj) = area.grid_points(GFS_RES);
    Job {
        model: Model::GfsWave,
        run,
        fhour,
        cache_name: format!("gfswave_{}_f{fhour:03}_{:08x}.grib2", run.format("%Y%m%d%H"), fnv(&q)),
        est_bytes: estimate_bytes(ni * nj, 3, 1),
        url: format!("{NOMADS}/filter_gfswave.pl?{q}"),
        params: vec![Param::WaveHeight, Param::WavePeriod, Param::WaveDir],
        valid: run + Duration::hours(fhour as i64),
        fetch: Fetch::Whole,
        member: None,
    }
}

/// The forecast a weather route through `points` needs from `source`: wind (and pressure, for the
/// overlay) over the points' bounding box plus room to manoeuvre, for as long as the model goes. A
/// box that crosses the antimeridian is not handled: it comes out as the long way round.
pub fn route_request(source: Source, points: &[LatLon]) -> Option<Request> {
    let (first, rest) = points.split_first()?;
    let mut b = Bounds::EMPTY;
    for p in std::iter::once(first).chain(rest) {
        b.extend([p.lon, p.lat]);
    }
    // The router tries headings up to 95 degrees either side of the bearing, so
    // the fleet spreads well beyond the direct line.
    let margin = (0.3 * b.width().max(b.height())).max(3.0);
    let area = Area::new(b.min_lat - margin, b.min_lon - margin, b.max_lat + margin, b.max_lon + margin);
    area.is_valid().then_some(Request { source, area, groups: vec![Group::Wind, Group::Pressure], horizon_hours: source.max_hours(), step_hours: 3 })
}

/// Whether the store already holds what `req` would download: wind over the
/// whole area, up to the end of the newest run (or the one before it), and
/// gusts and waves over it for as long if it asks for them. Pressure is only for the
/// overlay and is not checked.
pub fn covers(store: &WeatherStore, req: &Request, now: DateTime<Utc>) -> bool {
    let a = req.area;
    if a.west > a.east {
        return false;
    }
    let want = Bounds { min_lon: a.west, max_lon: a.east, min_lat: a.south, max_lat: a.north };
    let run = req.source.candidate_runs(now)[0];
    let last = hours_of(req.source, run, now, req.horizon_hours, req.step_hours).last().copied().unwrap_or(0);
    // A run one cycle older ends six hours earlier and is as good.
    let enough = run + Duration::hours(last as i64 - 6);
    // One area holds all of it, not several that add up to it: a seam between
    // two downloads has no data to sample across.
    let wind = [Param::WindU, Param::WindV].iter().all(|p| store.reaches(*p, &want, enough));
    // A group the model does not have cannot be waited for.
    wind && req.groups.iter().filter(|g| matches!(g, Group::Gust | Group::Waves) && req.source.has(**g)).all(|g| store.reaches(g.params()[0], &want, enough))
}

/// Plans a request against one run of its model.
pub fn plan(req: &Request, run: DateTime<Utc>, now: DateTime<Utc>) -> Plan {
    match req.source {
        Source::Gfs => plan_gfs(req, run, now),
        Source::Ifs | Source::Aifs => plan_ecmwf(req, run, now),
        Source::IconEu => plan_dwd(req, run, now),
    }
}

const DWD: &str = "https://opendata.dwd.de/weather/nwp/icon-eu/grib";

/// ICON-EU's fields for a group: the folder, the name in the file name, our parameter and the size of
/// one step of it, as published on 2026-11-06.
fn dwd_fields(g: Group) -> &'static [(&'static str, &'static str, Param, u64)] {
    match g {
        Group::Wind => &[("u_10m", "U_10M", Param::WindU, 1_227_000), ("v_10m", "V_10M", Param::WindV, 1_202_000)],
        Group::Gust => &[("vmax_10m", "VMAX_10M", Param::Gust, 1_209_000)],
        Group::Pressure => &[("pmsl", "PMSL", Param::Pressure, 839_000)],
        Group::Temperature => &[("t_2m", "T_2M", Param::Temperature, 1_056_000)],
        Group::Cloud => &[("clct", "CLCT", Param::CloudCover, 1_161_000)],
        Group::Waves | Group::Current => &[],
    }
}

/// The request's area cut to the model's domain, when it is regional; `None` when they do not meet.
pub fn within_domain(source: Source, a: &Area) -> Option<Area> {
    let Some((s, w, n, e)) = source.domain() else { return Some(*a) };
    if !source.covers(a.south, a.west, a.north, a.east) {
        return None;
    }
    Some(Area { south: a.south.max(s), west: a.west.max(w), north: a.north.min(n), east: a.east.min(e) })
}

fn plan_dwd(req: &Request, run: DateTime<Utc>, now: DateTime<Utc>) -> Plan {
    let source = req.source;
    let unavailable: Vec<Group> = req.groups.iter().copied().filter(|g| !source.has(*g)).collect();
    let Some(area) = within_domain(source, &req.area) else {
        return Plan { run, jobs: Vec::new(), est_bytes: 0, description: String::new(), unavailable: req.groups.clone() };
    };
    let area = area.snapped(source.res());
    let fields: Vec<&(&str, &str, Param, u64)> = req.groups.iter().flat_map(|g| dwd_fields(*g)).collect();
    let (date, hh) = (run.format("%Y%m%d%H").to_string(), run.format("%H").to_string());
    let key = format!("{} {} {} {} {}", fields.iter().map(|f| f.1).collect::<Vec<_>>().join(","), area.south, area.west, area.north, area.east);
    let jobs: Vec<Job> = if fields.is_empty() {
        Vec::new()
    } else {
        hours_of(source, run, now, req.horizon_hours, req.step_hours)
            .into_iter()
            .map(|h| {
                let urls: Vec<String> = fields.iter().map(|(dir, name, _, _)| format!("{DWD}/{hh}/{dir}/icon-eu_europe_regular-lat-lon_single-level_{date}_{h:03}_{name}.grib2.bz2")).collect();
                Job {
                    model: Model::IconEu,
                    run,
                    fhour: h,
                    url: urls[0].clone(),
                    cache_name: format!("iconeu_{date}_f{h:03}_{:08x}.grib2", fnv(&key)),
                    est_bytes: fields.iter().map(|f| f.3).sum(),
                    params: fields.iter().map(|f| f.2).collect(),
                    valid: run + Duration::hours(h as i64),
                    fetch: Fetch::Files { urls, crop: area },
                    member: None,
                }
            })
            .collect()
    };
    let est_bytes = jobs.iter().map(|j| j.est_bytes).sum();
    let last = jobs.last().map_or(0, |j| j.fhour);
    let description = if jobs.is_empty() { String::new() } else { format!("{}, run {}Z, to +{last} h", Model::IconEu.label(), run.format("%Y-%m-%d %H")) };
    Plan { run, jobs, est_bytes, description, unavailable }
}

fn plan_ecmwf(req: &Request, run: DateTime<Utc>, now: DateTime<Utc>) -> Plan {
    let source = req.source;
    let area = req.area.snapped(source.res());
    let hours = hours_of(source, run, now, req.horizon_hours, req.step_hours);
    let unavailable: Vec<Group> = req.groups.iter().copied().filter(|g| !source.has(*g)).collect();
    let mut jobs = Vec::new();
    let waves = req.groups.contains(&Group::Waves) && source.has(Group::Waves);
    for h in &hours {
        jobs.extend(ecmwf_job(source, run, *h, &req.groups, &area, false));
        if waves {
            jobs.extend(ecmwf_job(source, run, *h, &req.groups, &area, true));
        }
    }
    let est_bytes = jobs.iter().map(|j| j.est_bytes).sum();
    let last = jobs.last().map_or(0, |j| j.fhour);
    let mut labels: Vec<&str> = Vec::new();
    for j in &jobs {
        if !labels.contains(&j.model.label()) {
            labels.push(j.model.label());
        }
    }
    let description = if jobs.is_empty() { String::new() } else { format!("{}, run {}Z, to +{last} h", labels.join(" + "), run.format("%Y-%m-%d %H")) };
    Plan { run, jobs, est_bytes, description, unavailable }
}

fn plan_gfs(req: &Request, run: DateTime<Utc>, now: DateTime<Utc>) -> Plan {
    let area = req.area.snapped(GFS_RES);
    let hours = forecast_hours(run, now, req.horizon_hours, req.step_hours);
    let mut jobs = Vec::new();
    let mut unavailable = Vec::new();
    for g in &req.groups {
        if *g == Group::Current {
            unavailable.push(*g);
        }
    }
    let atmos: Vec<Group> = req.groups.iter().copied().filter(|g| !matches!(g, Group::Waves | Group::Current)).collect();
    let waves = req.groups.contains(&Group::Waves);
    for h in &hours {
        if let Some(j) = gfs_job(run, *h, &atmos, &area) {
            jobs.push(j);
        }
        if waves {
            jobs.push(wave_job(run, *h, &area));
        }
    }
    let est_bytes = jobs.iter().map(|j| j.est_bytes).sum();
    let last = hours.last().copied().unwrap_or(0);
    let mut models: Vec<&str> = Vec::new();
    if !atmos.is_empty() {
        models.push(Model::Gfs.label());
    }
    if waves {
        models.push(Model::GfsWave.label());
    }
    let description = if jobs.is_empty() {
        String::new()
    } else {
        format!("{}, run {}Z, to +{last} h", models.join(" + "), run.format("%Y-%m-%d %H"))
    };
    Plan { run, jobs, est_bytes, description, unavailable }
}

/// How many members GEFS has: the control run and 30 perturbed ones.
pub const GEFS_MEMBERS: u8 = 31;

/// GEFS runs to try, newest first: published about five and a half hours after the run.
pub fn gefs_runs(now: DateTime<Utc>) -> Vec<DateTime<Utc>> {
    Source::Gfs.candidate_runs(now - Duration::minutes(60))
}

/// The 10 m wind of every GEFS member over `area`, from `from` for `horizon_hours`, every 6 hours, through
/// the NOMADS filter (0.5°, cropped on the server: about a kilobyte a file). One request a member a step,
/// so a three-day passage is some 400 requests: minutes under the NOMADS throttle.
pub fn ensemble_plan(run: DateTime<Utc>, area: &Area, from: DateTime<Utc>, horizon_hours: u32) -> Plan {
    let area = area.snapped(0.5);
    let start = ((from - run).num_hours().max(0) as u32 / 6) * 6;
    let hours: Vec<u32> = (start..=(start + horizon_hours + 6).min(384)).step_by(6).collect();
    let (date, hh) = (run.format("%Y%m%d"), run.format("%H"));
    let mut jobs = Vec::new();
    for m in 0..GEFS_MEMBERS {
        let name = if m == 0 { "gec00".to_string() } else { format!("gep{m:02}") };
        for &h in &hours {
            let q = format!(
                "dir=%2Fgefs.{date}%2F{hh}%2Fatmos%2Fpgrb2ap5&file={name}.t{hh}z.pgrb2a.0p50.f{h:03}&var_UGRD=on&var_VGRD=on&lev_10_m_above_ground=on&{}",
                area_query(&area)
            );
            jobs.push(Job {
                model: Model::Gefs,
                run,
                fhour: h,
                cache_name: format!("gefs_{}_{name}_f{h:03}_{:08x}.grib2", run.format("%Y%m%d%H"), fnv(&q)),
                est_bytes: estimate_bytes(area.grid_points(0.5).0 * area.grid_points(0.5).1, 2, 1),
                url: format!("{NOMADS}/filter_gefs_atmos_0p50a.pl?{q}"),
                params: vec![Param::WindU, Param::WindV],
                valid: run + Duration::hours(h as i64),
                fetch: Fetch::Whole,
                member: Some(m),
            });
        }
    }
    let est_bytes = jobs.iter().map(|j| j.est_bytes).sum();
    let last = hours.last().copied().unwrap_or(0);
    Plan { run, jobs, est_bytes, description: format!("{}, run {}Z, to +{last} h", Model::Gefs.label(), run.format("%Y-%m-%d %H")), unavailable: Vec::new() }
}

/// The wind of a past run's first six hours over `area`: the model's analysis of what the wind was,
/// for the performance log (a hindcast). NOMADS keeps about ten days of runs.
pub fn analysis_plan(run: DateTime<Utc>, area: &Area) -> Plan {
    let area = area.snapped(GFS_RES);
    let jobs: Vec<Job> = (0..=6).filter_map(|h| gfs_job(run, h, &[Group::Wind], &area)).collect();
    let est_bytes = jobs.iter().map(|j| j.est_bytes).sum();
    Plan { run, jobs, est_bytes, description: format!("{}, run {}Z, analysis hours", Model::Gfs.label(), run.format("%Y-%m-%d %H")), unavailable: Vec::new() }
}

/// The GFS cycle a time falls in (00, 06, 12 or 18 UTC).
pub fn cycle_of(t: DateTime<Utc>) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(t.year(), t.month(), t.day(), (t.hour() / 6) * 6, 0, 0).unwrap()
}

/// Human-readable size: `850 KB`, `1.2 MB`.
pub fn format_bytes(b: u64) -> String {
    if b < 1000 {
        format!("{b} B")
    } else if b < 1_000_000 {
        format!("{:.0} KB", b as f64 / 1000.0)
    } else {
        format!("{:.1} MB", b as f64 / 1_000_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(d: u32, h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, d, h, m, 0).unwrap()
    }

    #[test]
    fn a_hindcast_asks_for_the_first_hours_of_one_run() {
        let run = cycle_of(t(20, 14, 37));
        assert_eq!(run, t(20, 12, 0));
        let p = analysis_plan(run, &Area::around(47.3, -3.1, 0.5));
        assert_eq!(p.jobs.iter().map(|j| j.fhour).collect::<Vec<_>>(), (0..=6).collect::<Vec<_>>());
        assert!(p.jobs.iter().all(|j| j.url.contains("gfs.20261020%2F12") && j.url.contains("var_UGRD") && !j.url.contains("PRMSL")));
    }

    #[test]
    fn areas_snap_outwards_to_the_grid() {
        let a = Area::new(43.1, -12.1, 51.9, -0.1).snapped(0.25);
        assert_eq!((a.south, a.north), (43.0, 52.0));
        assert_eq!((a.west, a.east), (-12.25, 0.0));
        // The Bay of Biscay and Channel box used in the research: 9 x 12 degrees.
        let (ni, nj) = Area::new(43.0, -12.0, 52.0, 0.0).grid_points(0.25);
        assert_eq!((ni, nj), (49, 37));
    }

    #[test]
    fn antimeridian_boxes_have_the_right_width() {
        let a = Area::new(-10.0, 170.0, 10.0, -170.0);
        assert!((a.width_deg() - 20.0).abs() < 1e-9);
        assert!(a.is_valid());
        let q = area_query(&a);
        assert!(q.contains("leftlon=170") && q.contains("rightlon=190"), "{q}");
        assert!(!Area::new(10.0, 0.0, 10.0, 5.0).is_valid());
    }

    #[test]
    fn longitudes_wrap_and_latitudes_clamp() {
        let a = Area::new(-95.0, 190.0, 95.0, 200.0);
        assert_eq!((a.south, a.north), (-90.0, 90.0));
        assert!((a.west + 170.0).abs() < 1e-9 && (a.east + 160.0).abs() < 1e-9);
        // Swapped north and south are put right.
        let b = Area::new(50.0, 0.0, 40.0, 5.0);
        assert_eq!((b.south, b.north), (40.0, 50.0));
    }

    #[test]
    fn candidate_runs_lag_the_clock_and_go_back_six_hours_at_a_time() {
        // 10:00 UTC: 00Z has been out for 10 h, 06Z only for 4 h (not yet offered).
        let r = candidate_runs(t(26, 10, 0));
        assert_eq!(r[0], t(26, 0, 0));
        assert_eq!(r[1], t(25, 18, 0));
        assert_eq!(r.len(), 4);
        // 11:00: 06Z is now 5 h old, so it is offered.
        assert_eq!(candidate_runs(t(26, 11, 0))[0], t(26, 6, 0));
        // Across midnight.
        assert_eq!(candidate_runs(t(26, 2, 0))[0], t(25, 18, 0));
    }

    #[test]
    fn forecast_hours_cover_now_to_the_horizon() {
        // Run 00Z, now 10:20, five days at 3 h: start at 9 (the step at or before 10), to 120+.
        let h = forecast_hours(t(26, 0, 0), t(26, 10, 20), 120, 3);
        assert_eq!(h[0], 9);
        assert_eq!(*h.last().unwrap(), 132); // 9 + 120 + 3
        assert!(h.windows(2).all(|w| w[1] > w[0]));
        // Hourly stays hourly inside the hourly range.
        let h = forecast_hours(t(26, 0, 0), t(26, 10, 20), 6, 1);
        assert_eq!(h, vec![10, 11, 12, 13, 14, 15, 16, 17]);
        // A run in the future is treated as starting now.
        assert_eq!(forecast_hours(t(26, 12, 0), t(26, 10, 0), 6, 3)[0], 0);
    }

    #[test]
    fn beyond_five_days_only_three_hourly_hours_are_requested() {
        let h = forecast_hours(t(20, 0, 0), t(26, 0, 0), 24, 1); // starts at f144
        assert!(h.iter().all(|x| x % 3 == 0), "{h:?}");
        // Never past the end of the forecast.
        let h = forecast_hours(t(1, 0, 0), t(16, 0, 0), 100, 3);
        assert!(h.iter().all(|x| *x <= GFS_MAX_HOURS), "{h:?}");
    }

    fn store_of(area: &Area, until: DateTime<Utc>) -> WeatherStore {
        use super::super::model::Field;
        use crate::chart::grid::Grid;
        let (ni, nj) = area.grid_points(GFS_RES);
        let mut s = WeatherStore::new();
        for p in [Param::WindU, Param::WindV] {
            for at in [until - Duration::hours(300), until] {
                let a = area.snapped(GFS_RES);
                s.add(Field { param: p, time: at, run: None, grid: Grid { ncols: ni as usize, nrows: nj as usize, xll: a.west, yll: a.south, dx: GFS_RES, dy: GFS_RES, data: vec![1.0; (ni * nj) as usize] } });
            }
        }
        s
    }

    #[test]
    fn a_route_asks_for_its_box_with_room_and_the_longest_forecast() {
        let r = route_request(Source::Gfs, &[LatLon::new(48.4, -4.5), LatLon::new(43.3, -8.4)]).unwrap();
        assert_eq!(r.horizon_hours, GFS_MAX_HOURS);
        assert!(r.groups.contains(&Group::Wind));
        // The route is 5 degrees across: 30% of that is under the 3 degree minimum.
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(close(r.area.south, 43.3 - 3.0) && close(r.area.north, 48.4 + 3.0), "{:?}", r.area);
        assert!(close(r.area.west, -8.4 - 3.0) && close(r.area.east, -4.5 + 3.0), "{:?}", r.area);
        // A long passage gets a proportionally wider margin.
        let long = route_request(Source::Gfs, &[LatLon::new(48.0, -5.0), LatLon::new(14.0, -61.0)]).unwrap();
        assert!((long.area.west - (-61.0 - 0.3 * 56.0)).abs() < 1e-9, "{:?}", long.area);
        assert!(route_request(Source::Gfs, &[]).is_none());
    }

    #[test]
    fn the_store_covers_a_request_only_if_it_has_the_area_and_the_length() {
        let now = t(26, 10, 0);
        let req = route_request(Source::Gfs, &[LatLon::new(48.4, -4.5), LatLon::new(43.3, -8.4)]).unwrap();
        let end = candidate_runs(now)[0] + Duration::hours(384);
        assert!(!covers(&WeatherStore::new(), &req, now));
        assert!(covers(&store_of(&req.area, end), &req, now));
        assert!(covers(&store_of(&req.area, end - Duration::hours(6)), &req, now), "a run older by one cycle is as good");
        assert!(!covers(&store_of(&req.area, end - Duration::hours(24)), &req, now), "a shorter forecast");
        let small = Area::new(46.0, -6.0, 47.0, -5.0);
        assert!(!covers(&store_of(&small, end), &req, now), "a smaller area");
        // Two downloads either side of the route: the union of their boxes covers it, the fields do not.
        let mut two = store_of(&Area::new(40.0, -12.0, 46.0, -2.0), end);
        two.merge(store_of(&Area::new(46.0, -12.0, 52.0, -2.0), end - Duration::hours(3)));
        assert!(!covers(&two, &req, now), "separate areas");
        // Asking for gusts too needs them in the store.
        let gusty = Request { groups: vec![Group::Wind, Group::Gust], ..req.clone() };
        let mut store = store_of(&req.area, end);
        assert!(!covers(&store, &gusty, now));
        let wind = store.nearest(Param::WindU, end).unwrap().clone();
        store.add(super::super::model::Field { param: Param::Gust, ..wind });
        assert!(covers(&store, &gusty, now));
    }

    #[test]
    fn a_forecast_kept_for_another_passage_does_not_cover_this_one_but_stays() {
        let now = t(26, 10, 0);
        let end = candidate_runs(now)[0] + Duration::hours(384);
        let here = route_request(Source::Gfs, &[LatLon::new(48.4, -4.5), LatLon::new(43.3, -8.4)]).unwrap();
        let there = route_request(Source::Gfs, &[LatLon::new(-36.0, 175.0), LatLon::new(-41.0, 174.0)]).unwrap();
        let mut store = store_of(&here.area, end);
        store.merge(store_of(&there.area, end));
        assert!(covers(&store, &here, now) && covers(&store, &there, now), "both are there");
        assert!(store.wind(46.0, -6.0, end).is_some() && store.wind(-38.0, 174.5, end).is_some());
    }

    #[test]
    fn requests_rounded_outwards_are_the_same_for_a_nudged_route() {
        let a = route_request(Source::Gfs, &[LatLon::new(48.4, -4.5), LatLon::new(43.3, -8.4)]).unwrap().area.outward(2.0);
        let b = route_request(Source::Gfs, &[LatLon::new(48.5, -4.4), LatLon::new(43.3, -8.4)]).unwrap().area.outward(2.0);
        assert_eq!(a, b);
        let raw = route_request(Source::Gfs, &[LatLon::new(48.4, -4.5), LatLon::new(43.3, -8.4)]).unwrap().area;
        assert!(a.south <= raw.south && a.north >= raw.north && a.west <= raw.west && a.east >= raw.east);
        assert!(a.north - raw.north < 2.0 && raw.south - a.south < 2.0);
        assert_eq!(Area::new(-89.0, 179.0, 89.0, 181.0).outward(2.0).north, 90.0);
    }

    fn biscay() -> Request {
        Request { source: Source::Gfs, area: Area::new(43.0, -12.0, 52.0, 0.0), groups: vec![Group::Wind, Group::Gust, Group::Pressure, Group::Waves], horizon_hours: 120, step_hours: 3 }
    }

    #[test]
    fn a_plan_has_one_atmosphere_and_one_wave_job_per_step() {
        let p = plan(&biscay(), t(26, 0, 0), t(26, 10, 0));
        let atmos = p.jobs.iter().filter(|j| j.model == Model::Gfs).count();
        let waves = p.jobs.iter().filter(|j| j.model == Model::GfsWave).count();
        assert_eq!(atmos, waves);
        assert!(atmos >= 40, "{atmos}");
        assert_eq!(p.models(), vec![Model::Gfs, Model::GfsWave].into_iter().filter(|m| p.jobs.iter().any(|j| j.model == *m)).collect::<Vec<_>>());
        assert!(p.description.contains("GFS") && p.description.contains("2026-10-26 00Z"));
        assert!(p.unavailable.is_empty());
    }

    #[test]
    fn the_estimate_matches_what_the_nomads_filter_returned() {
        // Measured: 4 fields x 1 step over 49 x 37 points = 11.4 KB, 7 wave variables 13.6 KB.
        let est = estimate_bytes(49 * 37, 4, 1);
        assert!((9_000..14_000).contains(&est), "{est}");
        let p = plan(&biscay(), t(26, 0, 0), t(26, 10, 0));
        // The whole five-day request is about a megabyte.
        assert!((600_000..1_800_000).contains(&p.est_bytes), "{}", p.est_bytes);
    }

    #[test]
    fn urls_use_the_nomads_filter_with_the_right_variables_and_levels() {
        let p = plan(&biscay(), t(26, 0, 0), t(26, 10, 0));
        let j = p.jobs.iter().find(|j| j.model == Model::Gfs).unwrap();
        assert!(j.url.starts_with("https://nomads.ncep.noaa.gov/cgi-bin/filter_gfs_0p25.pl?"));
        for needle in ["dir=%2Fgfs.20261026%2F00%2Fatmos", "gfs.t00z.pgrb2.0p25.f009", "var_UGRD=on", "var_VGRD=on", "var_GUST=on", "var_PRMSL=on",
            "lev_10_m_above_ground=on", "lev_surface=on", "lev_mean_sea_level=on", "leftlon=-12", "rightlon=0", "toplat=52", "bottomlat=43"] {
            assert!(j.url.contains(needle), "{needle} missing from {}", j.url);
        }
        assert!(!j.url.contains("var_TMP"), "temperature was not requested");
        let w = p.jobs.iter().find(|j| j.model == Model::GfsWave).unwrap();
        assert!(w.url.contains("filter_gfswave.pl") && w.url.contains("gfswave.t00z.global.0p25.f009.grib2") && w.url.contains("var_HTSGW=on"));
        assert_eq!(j.valid, t(26, 9, 0));
    }

    #[test]
    fn cache_names_differ_by_query_and_hour() {
        let a = plan(&biscay(), t(26, 0, 0), t(26, 10, 0));
        let mut other = biscay();
        other.area = Area::new(40.0, -12.0, 52.0, 0.0);
        let b = plan(&other, t(26, 0, 0), t(26, 10, 0));
        assert_ne!(a.jobs[0].cache_name, b.jobs[0].cache_name);
        let names: std::collections::HashSet<_> = a.jobs.iter().map(|j| j.cache_name.clone()).collect();
        assert_eq!(names.len(), a.jobs.len(), "every job has its own file");
        assert!(a.jobs[0].cache_name.starts_with("gfs_2026102600_f009_"));
    }

    #[test]
    fn currents_are_reported_as_unavailable_and_empty_requests_plan_nothing() {
        let mut r = biscay();
        r.groups = vec![Group::Current];
        let p = plan(&r, t(26, 0, 0), t(26, 10, 0));
        assert!(p.is_empty() && p.unavailable == vec![Group::Current] && p.description.is_empty());
        r.groups = vec![];
        assert!(plan(&r, t(26, 0, 0), t(26, 10, 0)).is_empty());
    }

    #[test]
    fn only_waves_means_only_wave_jobs() {
        let mut r = biscay();
        r.groups = vec![Group::Waves];
        let p = plan(&r, t(26, 0, 0), t(26, 10, 0));
        assert!(!p.is_empty() && p.jobs.iter().all(|j| j.model == Model::GfsWave));
    }

    #[test]
    fn icon_eu_asks_for_a_file_a_field_and_only_over_its_domain() {
        let r = Request { source: Source::IconEu, groups: vec![Group::Wind, Group::Gust, Group::Waves], ..biscay() };
        let p = plan(&r, t(26, 6, 0), t(26, 10, 0));
        assert!(p.unavailable == vec![Group::Waves] && !p.is_empty());
        let j = &p.jobs[0];
        let Fetch::Files { urls, crop } = &j.fetch else { panic!("{:?}", j.fetch) };
        assert_eq!(urls.len(), 3);
        assert!(urls[0].ends_with("/06/u_10m/icon-eu_europe_regular-lat-lon_single-level_2026102606_003_U_10M.grib2.bz2"), "{}", urls[0]);
        assert!(urls[2].contains("/vmax_10m/") && urls[2].contains("_VMAX_10M.grib2.bz2"));
        assert_eq!((crop.south, crop.north), (43.0, 52.0));
        // Hourly where it is, every three hours past 78.
        assert!(p.jobs.iter().all(|j| j.fhour <= 120));
        // Cut to the domain; nothing at all over New Zealand.
        assert_eq!(within_domain(Source::IconEu, &Area::new(20.0, -30.0, 40.0, 0.0)).unwrap().south, 29.5);
        let nz = Request { area: Area::new(-45.0, 165.0, -30.0, 179.0), ..r };
        assert!(plan(&nz, t(26, 6, 0), t(26, 10, 0)).is_empty());
    }

    #[test]
    fn the_ensemble_asks_each_member_for_each_six_hours() {
        let p = ensemble_plan(t(26, 0, 0), &Area::new(44.0, -10.0, 50.0, 0.0), t(26, 10, 0), 48);
        assert_eq!(p.jobs.len(), 31 * 10, "members times 6 to 60 h");
        assert_eq!(p.jobs.iter().filter(|j| j.member == Some(0)).count(), 10);
        let j = &p.jobs[11];
        assert_eq!((j.member, j.fhour), (Some(1), 12));
        assert!(j.url.contains("filter_gefs_atmos_0p50a.pl") && j.url.contains("gep01.t00z.pgrb2a.0p50.f012") && j.url.contains("var_UGRD=on"), "{}", j.url);
        assert!(p.jobs[0].url.contains("gec00.t00z"));
        assert!(p.est_bytes < 1_000_000);
    }

    #[test]
    fn sizes_are_formatted() {
        assert_eq!(format_bytes(850), "850 B");
        assert_eq!(format_bytes(11_400), "11 KB");
        assert_eq!(format_bytes(1_234_567), "1.2 MB");
    }
}
