//! Copernicus Marine currents, read anonymously from the ARCO Zarr stores (`design/currents.md` §2):
//! the global SMOC (circulation plus a FES2014 tide) and the regional models (North-West Shelf, IBI,
//! Mediterranean, Baltic).
//!
//! There is no server-side subsetting: the stores are read chunk by chunk. The geo-chunked store keeps
//! every hour of a small area in one chunk, compressed in blocks of a run of hours, so the forecast is
//! fetched as a byte range of each chunk (its header and block offsets first, then the blocks holding the
//! hours wanted). What is fetched is cropped and cached as GRIB2, keyed by the store's last update.
//! Dataset ids carry a version that changes, so they are found through the STAC catalogue, not written
//! here.

use super::CurrentSource;
use super::blosc;
use super::zarr::{self, ArrayMeta};
use crate::chart::grid::Grid;
use crate::vector::Bounds;
use crate::weather::download::DlError;
use crate::weather::model::{Field, Param};
use chrono::{DateTime, Duration, Utc};
use std::collections::BTreeMap;
use std::io::Read;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

const STAC: &str = "https://stac.marine.copernicus.eu/metadata";
/// Connections at once: the object store has no known limit, but be polite.
const PARALLEL: usize = 8;
/// A discovered store is trusted this long before the catalogue is read again.
const STAC_HOURS: i64 = 24;
/// Cached subsets are kept this long.
const KEEP_DAYS: u64 = 3;
/// The first bytes of a chunk asked for: its header and the offsets of up to 252 blocks.
const HEAD_BYTES: usize = 1024;

/// A Copernicus Marine dataset of surface currents.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dataset {
    pub source: CurrentSource,
    pub product: &'static str,
    /// The dataset id without its version suffix.
    pub prefix: &'static str,
    /// The tide's own variables, besides `uo` and `vo` (SMOC only).
    pub tide: Option<(&'static str, &'static str)>,
    pub domain: Bounds,
    /// A geo-chunk's size, degrees of longitude and latitude, and its typical size per variable with
    /// byte ranges (measured 2026-11-09), for estimates.
    pub chunk_deg: (f64, f64),
    pub chunk_kb: u64,
}

const fn b(min_lon: f64, min_lat: f64, max_lon: f64, max_lat: f64) -> Bounds {
    Bounds { min_lon, min_lat, max_lon, max_lat }
}

/// Finest first, as `CurrentField` prefers them.
pub const DATASETS: [Dataset; 5] = [
    Dataset { source: CurrentSource::NwShelf, product: "NWSHELF_ANALYSISFORECAST_PHY_004_013", prefix: "cmems_mod_nws_phy-cur_anfc_1.5km-2D_PT1H-i_", tide: None, domain: b(-16.0, 46.0, 13.0, 62.74), chunk_deg: (0.485, 0.216), chunk_kb: 150 },
    Dataset { source: CurrentSource::Ibi, product: "IBI_ANALYSISFORECAST_PHY_005_001", prefix: "cmems_mod_ibi_phy_anfc_0.027deg-2D_PT1H-m_", tide: None, domain: b(-19.08, 26.17, 5.08, 56.08), chunk_deg: (0.444, 0.444), chunk_kb: 150 },
    Dataset { source: CurrentSource::Med, product: "MEDSEA_ANALYSISFORECAST_PHY_006_013", prefix: "cmems_mod_med_phy-cur_anfc_4.2km-2D_PT1H-m_", tide: None, domain: b(-17.29, 30.19, 36.29, 45.98), chunk_deg: (0.667, 0.667), chunk_kb: 300 },
    Dataset { source: CurrentSource::Baltic, product: "BALTICSEA_ANALYSISFORECAST_PHY_003_006", prefix: "cmems_mod_bal_phy_anfc_PT1H-i_", tide: None, domain: b(9.04, 53.0, 30.21, 65.89), chunk_deg: (0.222, 0.267), chunk_kb: 150 },
    Dataset { source: CurrentSource::Smoc, product: "GLOBAL_ANALYSISFORECAST_PHY_001_024", prefix: "cmems_mod_glo_phy_anfc_merged-uv_PT1H-i_", tide: Some(("utide", "vtide")), domain: b(-180.0, -80.0, 180.0, 90.0), chunk_deg: (0.667, 1.333), chunk_kb: 110 },
];

pub fn dataset(s: CurrentSource) -> Option<&'static Dataset> {
    DATASETS.iter().find(|d| d.source == s)
}

fn contains(outer: &Bounds, inner: &Bounds) -> bool {
    outer.min_lon <= inner.min_lon && outer.max_lon >= inner.max_lon && outer.min_lat <= inner.min_lat && outer.max_lat >= inner.max_lat
}

/// What to fetch for `area`, finest first: each dataset reaching into it, and whether its tide (SMOC)
/// is wanted. A dataset is left out where a finer one holds the whole area. SMOC then comes only for
/// its residual, and only when `residual` (an atlas there wants it).
pub fn choose(area: &Bounds, residual: bool) -> Vec<(&'static Dataset, Vec<&'static str>)> {
    let mut out: Vec<(&'static Dataset, Vec<&'static str>)> = Vec::new();
    for d in &DATASETS {
        if !d.domain.intersects(area) {
            continue;
        }
        let covered = out.iter().any(|(o, _)| contains(&o.domain, area));
        match d.tide {
            Some(_) if covered => {
                if residual {
                    out.push((d, vec!["uo", "vo"]));
                }
            }
            Some((ut, vt)) => out.push((d, vec!["uo", "vo", ut, vt])),
            None if covered => {}
            None => out.push((d, vec!["uo", "vo"])),
        }
    }
    out
}

/// About how many bytes fetching `area` costs, for the panels to say before it is fetched.
pub fn estimate_bytes(area: &Bounds, residual: bool) -> u64 {
    choose(area, residual)
        .iter()
        .map(|(d, vars)| {
            let i = d.domain;
            let w = (area.max_lon.min(i.max_lon) - area.min_lon.max(i.min_lon)).max(0.0);
            let h = (area.max_lat.min(i.max_lat) - area.min_lat.max(i.min_lat)).max(0.0);
            let chunks = ((w / d.chunk_deg.0).ceil() + 1.0) * ((h / d.chunk_deg.1).ceil() + 1.0);
            chunks as u64 * vars.len() as u64 * d.chunk_kb * 1024
        })
        .sum()
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(std::time::Duration::from_secs(10)))
        .timeout_global(Some(std::time::Duration::from_secs(120)))
        .http_status_as_error(false)
        .user_agent("isomar/0.1 (open-source chart plotter)")
        .build()
        .into()
}

struct Reply {
    status: u16,
    body: Vec<u8>,
    last_modified: Option<String>,
}

/// A GET, the bytes `range` only when given. 403 and 404 (a chunk the store does not have, or a
/// dataset gone) are `NotAvailable`.
fn get(agent: &ureq::Agent, url: &str, range: Option<Range<usize>>, cancel: &AtomicBool, bytes: &AtomicU64) -> Result<Reply, DlError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(DlError::Cancelled);
    }
    let mut req = agent.get(url);
    if let Some(r) = &range {
        req = req.header("Range", format!("bytes={}-{}", r.start, r.end.saturating_sub(1)));
    }
    let mut resp = req.call().map_err(|e| DlError::Http(e.to_string()))?;
    let status = resp.status().as_u16();
    if status == 403 || status == 404 {
        return Err(DlError::NotAvailable(format!("HTTP {status}")));
    }
    if !resp.status().is_success() {
        return Err(DlError::Http(format!("HTTP {status} for {url}")));
    }
    let last_modified = resp.headers().get("last-modified").map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned());
    let mut reader = resp.body_mut().as_reader();
    let mut buf = [0u8; 16 * 1024];
    let mut body = Vec::new();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(DlError::Cancelled);
        }
        let n = reader.read(&mut buf).map_err(|e| DlError::Http(e.to_string()))?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&buf[..n]);
        bytes.fetch_add(n as u64, Ordering::Relaxed);
    }
    Ok(Reply { status, body, last_modified })
}

fn currents_dir(cache: &Path) -> PathBuf {
    cache.join("currents")
}

/// The geo-chunked store of `d`'s current version: from the catalogue, or what it said within the day.
pub fn discover(agent: &ureq::Agent, d: &Dataset, cache: &Path, cancel: &AtomicBool) -> Result<String, DlError> {
    let saved = currents_dir(cache).join("stac").join(format!("{}.json", d.source.id()));
    if let Ok(text) = std::fs::read_to_string(&saved)
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(&text)
        && let (Some(url), Some(at)) = (v.get("url").and_then(|u| u.as_str()), v.get("at").and_then(|a| a.as_i64()))
        && Utc::now().timestamp() - at < STAC_HOURS * 3600
    {
        return Ok(url.to_string());
    }
    let bytes = AtomicU64::new(0);
    let product = get(agent, &format!("{STAC}/{}/product.stac.json", d.product), None, cancel, &bytes)?;
    let v: serde_json::Value = serde_json::from_slice(&product.body).map_err(|e| DlError::Http(format!("catalogue: {e}")))?;
    let mut items: Vec<String> = v
        .get("links")
        .and_then(|l| l.as_array())
        .into_iter()
        .flatten()
        .filter(|l| l.get("rel").and_then(|r| r.as_str()) == Some("item"))
        .filter_map(|l| l.get("href").and_then(|h| h.as_str()))
        .filter(|h| h.starts_with(d.prefix))
        .map(str::to_string)
        .collect();
    // The newest version first; one not published yet has no stores and is passed over.
    items.sort();
    items.reverse();
    for item in items {
        let r = get(agent, &format!("{STAC}/{}/{item}", d.product), None, cancel, &bytes)?;
        let Ok(ds) = serde_json::from_slice::<serde_json::Value>(&r.body) else { continue };
        let url = ds.pointer("/assets/geoChunked/href").and_then(|h| h.as_str());
        let end = ds.pointer("/properties/end_datetime").and_then(|e| e.as_str()).and_then(|e| DateTime::parse_from_rfc3339(e).ok());
        if let (Some(url), Some(end)) = (url, end)
            && end.with_timezone(&Utc) > Utc::now() - Duration::days(2)
        {
            if let Some(dir) = saved.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&saved, serde_json::json!({ "url": url, "at": Utc::now().timestamp() }).to_string());
            crate::log!("currents: {} is {url}", d.source.label());
            return Ok(url.to_string());
        }
    }
    Err(DlError::NotAvailable(format!("no current version of {} in the catalogue", d.prefix)))
}

/// What a fetch did, for the job's progress and the logs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub chunks_done: usize,
    pub chunks_total: usize,
    pub bytes: u64,
}

/// The indices of `coord` (ascending or descending) between `lo` and `hi`, with one more each side.
fn index_range(coord: &[f64], lo: f64, hi: f64) -> Option<Range<usize>> {
    let inside: Vec<usize> = coord.iter().enumerate().filter(|(_, c)| **c >= lo && **c <= hi).map(|(i, _)| i).collect();
    let (first, last) = (*inside.first()?, *inside.last()?);
    Some(first.saturating_sub(1)..(last + 2).min(coord.len()))
}

/// A whole coordinate array, every chunk of it.
fn coordinate(agent: &ureq::Agent, url: &str, name: &str, m: &ArrayMeta, cancel: &AtomicBool, bytes: &AtomicU64) -> Result<Vec<f64>, DlError> {
    let mut out = Vec::with_capacity(m.shape[0]);
    for c in 0..m.n_chunks(0) {
        let r = get(agent, &format!("{url}/{}", m.key(name, &[c])), None, cancel, bytes)?;
        let raw = if m.blosc { blosc::decode(&r.body).map_err(|e| DlError::Http(e.to_string()))? } else { r.body };
        out.extend(m.values64(&raw));
    }
    out.truncate(m.shape[0]);
    Ok(out)
}

/// The time steps of the axis from `from` on: the last chunk of it, and the one before when the last
/// starts after `from`. Returns the absolute indices and their times.
fn time_tail(agent: &ureq::Agent, url: &str, m: &ArrayMeta, from: DateTime<Utc>, cancel: &AtomicBool, bytes: &AtomicU64) -> Result<Vec<(usize, DateTime<Utc>)>, DlError> {
    let n = m.shape[0];
    let mut c = (n - 1) / m.chunks[0];
    let mut out: Vec<(usize, DateTime<Utc>)> = Vec::new();
    loop {
        let r = get(agent, &format!("{url}/{}", m.key("time", &[c])), None, cancel, bytes)?;
        let raw = if m.blosc { blosc::decode(&r.body).map_err(|e| DlError::Http(e.to_string()))? } else { r.body };
        let start = c * m.chunks[0];
        let vals: Vec<f64> = m.values64(&raw).into_iter().take(n - start).collect();
        let ts = zarr::times(&m.units, &vals).ok_or_else(|| DlError::Http(format!("time units {}", m.units)))?;
        let mut part: Vec<(usize, DateTime<Utc>)> = ts.into_iter().enumerate().map(|(i, t)| (start + i, t)).collect();
        part.append(&mut out);
        out = part;
        if c == 0 || out.first().is_some_and(|(_, t)| *t <= from) {
            break;
        }
        c -= 1;
    }
    Ok(out.into_iter().filter(|(_, t)| *t >= from - Duration::hours(1)).collect())
}

/// One chunk to read: a variable, its chunk indices, and the steps of its first axis wanted.
struct Job {
    var: usize,
    idx: Vec<usize>,
    steps: Range<usize>,
}

/// The values of `steps` (the chunk's own first-axis indices) of a chunk, every other axis whole. A chunk
/// the store does not have is all fill.
fn read_chunk(agent: &ureq::Agent, url: &str, m: &ArrayMeta, key: &str, steps: Range<usize>, cancel: &AtomicBool, bytes: &AtomicU64) -> Result<Vec<f32>, DlError> {
    let wanted = m.leading_bytes(steps.clone());
    let whole = |body: &[u8]| -> Result<Vec<f32>, DlError> {
        let raw = if m.blosc { blosc::decode(body).map_err(|e| DlError::Http(e.to_string()))? } else { body.to_vec() };
        Ok(m.values(raw.get(wanted.clone()).ok_or_else(|| DlError::Http("short chunk".into()))?))
    };
    let missing = || vec![f32::NAN; wanted.len() / m.dtype.size()];
    let chunk_url = format!("{url}/{key}");
    if !m.blosc {
        return match get(agent, &chunk_url, Some(wanted.clone()), cancel, bytes) {
            Ok(r) if r.status == 206 => Ok(m.values(&r.body)),
            Ok(r) => whole(&r.body),
            Err(DlError::NotAvailable(_)) => Ok(missing()),
            Err(e) => Err(e),
        };
    }
    let head = match get(agent, &chunk_url, Some(0..HEAD_BYTES), cancel, bytes) {
        Ok(r) => r,
        Err(DlError::NotAvailable(_)) => return Ok(missing()),
        Err(e) => return Err(e),
    };
    if head.status != 206 {
        return whole(&head.body);
    }
    let h = blosc::Header::parse(&head.body).map_err(|e| DlError::Http(e.to_string()))?;
    if h.memcpyed() || h.nblocks() <= 1 || h.cbytes <= head.body.len() {
        return if h.cbytes <= head.body.len() { whole(&head.body) } else { whole(&get(agent, &chunk_url, None, cancel, bytes)?.body) };
    }
    let index = if h.index_len() <= head.body.len() { head.body } else { get(agent, &chunk_url, Some(0..h.index_len()), cancel, bytes)?.body };
    let offsets = blosc::block_offsets(&index, &h).map_err(|e| DlError::Http(e.to_string()))?;
    let blocks = h.blocks_for(wanted.clone());
    let span = blosc::span(&h, &offsets, blocks.clone());
    let part = get(agent, &chunk_url, Some(span.clone()), cancel, bytes)?;
    let raw = if part.status == 206 {
        blosc::decode_blocks(&h, &offsets, &part.body, span.start, blocks.clone())
    } else {
        blosc::decode_blocks(&h, &offsets, &part.body, 0, blocks.clone())
    };
    match raw {
        Ok(raw) => {
            let first = h.block_range(blocks.start).start;
            Ok(m.values(raw.get(wanted.start - first..wanted.end - first).ok_or_else(|| DlError::Http("short blocks".into()))?))
        }
        // Whatever went wrong with the parts, the whole chunk still decodes.
        Err(e) => {
            crate::log!("currents: {key} by parts failed ({e}); fetching it whole");
            whole(&get(agent, &chunk_url, None, cancel, bytes)?.body)
        }
    }
}

/// The variable's axes: time first, then an optional level, latitude, longitude.
struct Axes {
    level: Option<usize>,
    lat: usize,
    lon: usize,
}

fn axes(m: &ArrayMeta) -> Option<Axes> {
    if m.dims.first().map(String::as_str) != Some("time") {
        return None;
    }
    let pos = |n: &str| m.dims.iter().position(|d| d == n);
    let level = pos("elevation").or_else(|| pos("depth"));
    Some(Axes { level, lat: pos("latitude")?, lon: pos("longitude")? })
}

fn area_key(a: &Bounds) -> String {
    format!("{:.2}_{:.2}_{:.2}_{:.2}", a.min_lat, a.min_lon, a.max_lat, a.max_lon)
}

/// Rounded outward to a quarter degree: a passage nudged a little finds the subset already cached.
pub fn rounded(a: &Bounds) -> Bounds {
    let (down, up) = (|v: f64| (v * 4.0).floor() / 4.0, |v: f64| (v * 4.0).ceil() / 4.0);
    Bounds { min_lon: down(a.min_lon), min_lat: down(a.min_lat), max_lon: up(a.max_lon), max_lat: up(a.max_lat) }
}

/// `d`'s currents over `area` from `from` to the end of its forecast, as fields: `CurrentU/V` the total
/// current (`uo + utide` for SMOC), and for SMOC `ResidualU/V` its circulation alone. `vars` as
/// `choose` gave them. From the cache when the store has not changed since.
pub fn fetch(d: &Dataset, vars: &[&str], area: &Bounds, from: DateTime<Utc>, cache: &Path, cancel: &AtomicBool, progress: &(dyn Fn(Progress) + Sync)) -> Result<Vec<Field>, DlError> {
    let agent = agent();
    let bytes = AtomicU64::new(0);
    let area = rounded(area);
    let url = discover(&agent, d, cache, cancel)?;
    let zm = get(&agent, &format!("{url}/.zmetadata"), None, cancel, &bytes)?;
    let metas = zarr::parse_metadata(&zm.body).map_err(|e| DlError::Http(e.to_string()))?;
    let stamp = zm.last_modified.as_deref().and_then(|s| DateTime::parse_from_rfc2822(s).ok()).map(|t| t.with_timezone(&Utc).format("%Y%m%d%H%M").to_string()).unwrap_or_else(|| Utc::now().format("%Y%m%d").to_string());
    let dir = currents_dir(cache).join(d.source.id());
    let tide = vars.len() > 2;
    let file = dir.join(format!("{stamp}_{}_{}.grib2", area_key(&area), if tide { "tide" } else { "uv" }));
    if let Ok(data) = std::fs::read(&file)
        && let Ok(fields) = crate::weather::grib::read_fields(&data)
        && !fields.is_empty()
    {
        crate::log!("currents: {} for {area:?} from the cache", d.source.label());
        return Ok(fields);
    }
    let m = |n: &str| metas.get(n).ok_or_else(|| DlError::Http(format!("{} has no {n}", d.prefix)));
    let lat = coordinate(&agent, &url, "latitude", m("latitude")?, cancel, &bytes)?;
    let lon = coordinate(&agent, &url, "longitude", m("longitude")?, cancel, &bytes)?;
    let times = time_tail(&agent, &url, m("time")?, from, cancel, &bytes)?;
    let (Some(ys), Some(xs)) = (index_range(&lat, area.min_lat, area.max_lat), index_range(&lon, area.min_lon, area.max_lon)) else {
        return Ok(Vec::new());
    };
    let (Some(t0), Some(t1)) = (times.first().map(|t| t.0), times.last().map(|t| t.0 + 1)) else { return Ok(Vec::new()) };
    let metas: Vec<(&str, &ArrayMeta, Axes)> = vars.iter().map(|v| Ok((*v, m(v)?, axes(m(v)?).ok_or_else(|| DlError::Http(format!("{v}: unexpected axes")))?))).collect::<Result<_, DlError>>()?;
    // The surface: the level nearest zero.
    let surface = match metas[0].2.level {
        Some(a) => {
            let name = metas[0].1.dims[a].clone();
            let levels = coordinate(&agent, &url, &name, m(&name)?, cancel, &bytes)?;
            levels.iter().enumerate().min_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).map_or(0, |(i, _)| i)
        }
        None => 0,
    };
    // Every chunk of every variable over the area and the hours.
    let mut jobs = Vec::new();
    for (vi, (_, am, ax)) in metas.iter().enumerate() {
        let (ct, cy, cx) = (am.chunks[0], am.chunks[ax.lat], am.chunks[ax.lon]);
        for tc in t0 / ct..=(t1 - 1) / ct {
            let steps = t0.max(tc * ct) - tc * ct..t1.min((tc + 1) * ct) - tc * ct;
            for yc in ys.start / cy..=(ys.end - 1) / cy {
                for xc in xs.start / cx..=(xs.end - 1) / cx {
                    let mut idx = vec![0; am.dims.len()];
                    idx[0] = tc;
                    idx[ax.lat] = yc;
                    idx[ax.lon] = xc;
                    if let Some(l) = ax.level {
                        idx[l] = surface / am.chunks[l];
                    }
                    jobs.push(Job { var: vi, idx, steps: steps.clone() });
                }
            }
        }
    }
    let total = jobs.len();
    crate::log!("currents: {} over {area:?}, {} hours, {total} chunks of {}", d.source.label(), t1 - t0, vars.join(", "));
    let (nt, ny, nx) = (t1 - t0, ys.len(), xs.len());
    let out: Vec<Mutex<Vec<f32>>> = metas.iter().map(|_| Mutex::new(vec![f32::NAN; nt * ny * nx])).collect();
    let (next, done) = (AtomicUsize::new(0), AtomicUsize::new(0));
    let first_error: Mutex<Option<DlError>> = Mutex::new(None);
    let stop = AtomicBool::new(false);
    let halt = || cancel.load(Ordering::Relaxed) || stop.load(Ordering::Relaxed);
    let report = || progress(Progress { chunks_done: done.load(Ordering::Relaxed), chunks_total: total, bytes: bytes.load(Ordering::Relaxed) });
    report();
    std::thread::scope(|s| {
        for _ in 0..PARALLEL.min(total.max(1)) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= total || halt() {
                        return;
                    }
                    let job = &jobs[i];
                    let (name, am, ax) = &metas[job.var];
                    let key = am.key(name, &job.idx);
                    let mut got = Err(DlError::Http("no attempt".into()));
                    for attempt in 0..3 {
                        got = read_chunk(&agent, &url, am, &key, job.steps.clone(), cancel, &bytes);
                        match &got {
                            Err(DlError::Http(e)) if attempt < 2 => {
                                crate::log!("currents: {key}: {e}; again");
                                std::thread::sleep(std::time::Duration::from_millis(500 << attempt));
                            }
                            _ => break,
                        }
                    }
                    let vals = match got {
                        Ok(v) => v,
                        Err(e) => {
                            stop.store(true, Ordering::Relaxed);
                            first_error.lock().unwrap().get_or_insert(e);
                            return;
                        }
                    };
                    // Into the output: the chunk is [steps][level?][y][x], C order.
                    let (cy, cx) = (am.chunks[ax.lat], am.chunks[ax.lon]);
                    let levels = ax.level.map_or(1, |l| am.chunks[l]);
                    let local_level = ax.level.map_or(0, |l| surface % am.chunks[l]);
                    let (y0, x0) = (job.idx[ax.lat] * cy, job.idx[ax.lon] * cx);
                    let t_base = job.idx[0] * am.chunks[0] + job.steps.start;
                    let mut o = out[job.var].lock().unwrap();
                    for (k, _) in job.steps.clone().enumerate() {
                        let t = t_base + k - t0;
                        for yy in 0..cy {
                            let y = y0 + yy;
                            if y < ys.start || y >= ys.end {
                                continue;
                            }
                            for xx in 0..cx {
                                let x = x0 + xx;
                                if x < xs.start || x >= xs.end {
                                    continue;
                                }
                                let src = ((k * levels + local_level) * cy + yy) * cx + xx;
                                if let Some(v) = vals.get(src) {
                                    o[(t * ny + (y - ys.start)) * nx + (x - xs.start)] = *v;
                                }
                            }
                        }
                    }
                    drop(o);
                    done.fetch_add(1, Ordering::Relaxed);
                    report();
                }
            });
        }
    });
    if cancel.load(Ordering::Relaxed) {
        return Err(DlError::Cancelled);
    }
    if let Some(e) = first_error.into_inner().unwrap() {
        return Err(e);
    }
    crate::log!("currents: {} fetched, {} bytes", d.source.label(), bytes.load(Ordering::Relaxed));
    let out: Vec<Vec<f32>> = out.into_iter().map(|m| m.into_inner().unwrap()).collect();
    let fields = assemble(&out, vars, d.tide.is_some() && vars.len() == 2, &lat[ys.clone()], &lon[xs.clone()], &times);
    // Cached for the day: an older update of the same area goes.
    let _ = std::fs::create_dir_all(&dir);
    let key = area_key(&area);
    for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let p = e.path();
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|a| a.as_secs() > KEEP_DAYS * 86400);
        if old || p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.contains(&key)) {
            let _ = std::fs::remove_file(p);
        }
    }
    let mut bytes_out = Vec::new();
    for f in &fields {
        bytes_out.extend(crate::weather::gribwrite::encode(f));
    }
    let part = file.with_extension("part");
    if std::fs::write(&part, &bytes_out).is_ok() {
        let _ = std::fs::rename(&part, &file);
    }
    Ok(fields)
}

/// The fetched values as fields, rows north to south: `uo + utide` as `CurrentU`, `uo` as `ResidualU`
/// when there is a tide; `uo` as `ResidualU` alone when SMOC came only for its residual
/// (`residual_only`); else `uo` as `CurrentU`.
fn assemble(vals: &[Vec<f32>], vars: &[&str], residual_only: bool, lat: &[f64], lon: &[f64], times: &[(usize, DateTime<Utc>)]) -> Vec<Field> {
    let (ny, nx) = (lat.len(), lon.len());
    if ny < 2 || nx < 2 {
        return Vec::new();
    }
    let dy = (lat[ny - 1] - lat[0]).abs() / (ny - 1) as f64;
    let dx = (lon[nx - 1] - lon[0]) / (nx - 1) as f64;
    let ascending = lat[ny - 1] > lat[0];
    let south = lat[0].min(lat[ny - 1]);
    let run = times.first().map(|t| t.1);
    let var = |n: &str| vars.iter().position(|v| *v == n);
    let mut fields = Vec::new();
    for (k, (_, t)) in times.iter().enumerate() {
        let grid = |f: &dyn Fn(usize) -> f32| {
            let mut data = Vec::with_capacity(ny * nx);
            for r in 0..ny {
                let y = if ascending { ny - 1 - r } else { r };
                for x in 0..nx {
                    data.push(f((k * ny + y) * nx + x));
                }
            }
            Grid { ncols: nx, nrows: ny, xll: lon[0] - dx / 2.0, yll: south - dy / 2.0, dx, dy, data }
        };
        let (Some(u), Some(v)) = (var("uo"), var("vo")) else { continue };
        match (var("utide"), var("vtide")) {
            (Some(ut), Some(vt)) => {
                for (p, a, b) in [(Param::CurrentU, u, ut), (Param::CurrentV, v, vt)] {
                    fields.push(Field { param: p, time: *t, run, grid: grid(&|i| vals[a][i] + vals[b][i]) });
                }
                for (p, a) in [(Param::ResidualU, u), (Param::ResidualV, v)] {
                    fields.push(Field { param: p, time: *t, run, grid: grid(&|i| vals[a][i]) });
                }
            }
            _ => {
                // SMOC fetched for its residual alone gives no total.
                let params = if residual_only { [Param::ResidualU, Param::ResidualV] } else { [Param::CurrentU, Param::CurrentV] };
                for (p, a) in params.into_iter().zip([u, v]) {
                    fields.push(Field { param: p, time: *t, run, grid: grid(&|i| vals[a][i]) });
                }
            }
        }
    }
    fields
}

/// Fetches every dataset `choose` gives for `area`, finest first. A dataset that fails is logged and
/// left out (the coarser ones still cover); the error of the last failure is returned only when
/// nothing came.
pub fn fetch_all(area: &Bounds, residual: bool, from: DateTime<Utc>, cache: &Path, cancel: &AtomicBool, progress: &(dyn Fn(CurrentSource, Progress) + Sync)) -> Result<BTreeMap<CurrentSource, Vec<Field>>, DlError> {
    let mut out = BTreeMap::new();
    let mut last = None;
    for (d, vars) in choose(area, residual) {
        match fetch(d, &vars, area, from, cache, cancel, &|p| progress(d.source, p)) {
            Ok(f) if !f.is_empty() => {
                out.insert(d.source, f);
            }
            Ok(_) => {}
            Err(DlError::Cancelled) => return Err(DlError::Cancelled),
            Err(e) => {
                crate::log!("currents: {} failed: {e}", d.source.label());
                last = Some(e);
            }
        }
    }
    match (out.is_empty(), last) {
        (true, Some(e)) => Err(e),
        _ => Ok(out),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_finest_dataset_that_holds_the_area_is_enough() {
        let channel = Bounds { min_lon: -3.0, min_lat: 49.0, max_lon: -1.0, max_lat: 50.5 };
        let ids = |v: Vec<(&Dataset, Vec<&str>)>| v.into_iter().map(|(d, vars)| (d.source, vars.len())).collect::<Vec<_>>();
        assert_eq!(ids(choose(&channel, false)), vec![(CurrentSource::NwShelf, 2)], "the shelf holds it: no IBI, no SMOC");
        assert_eq!(ids(choose(&channel, true)), vec![(CurrentSource::NwShelf, 2), (CurrentSource::Smoc, 2)], "SMOC's residual for an atlas");
        let biscay = Bounds { min_lon: -6.0, min_lat: 44.0, max_lon: -2.0, max_lat: 47.0 };
        assert_eq!(ids(choose(&biscay, false)), vec![(CurrentSource::NwShelf, 2), (CurrentSource::Ibi, 2)], "the shelf ends at 46 N; IBI holds it all");
        let atlantic = Bounds { min_lon: -40.0, min_lat: 30.0, max_lon: -30.0, max_lat: 40.0 };
        assert_eq!(ids(choose(&atlantic, false)), vec![(CurrentSource::Smoc, 4)], "SMOC with its tide");
        assert!(estimate_bytes(&channel, false) > 1_000_000 && estimate_bytes(&channel, false) < 100_000_000);
    }

    #[test]
    fn coordinate_ranges_take_a_cell_more_each_side() {
        let up: Vec<f64> = (0..10).map(|i| i as f64).collect();
        assert_eq!(index_range(&up, 2.5, 5.5), Some(2..7));
        assert_eq!(index_range(&up, -5.0, 0.5), Some(0..2));
        let down: Vec<f64> = up.iter().rev().copied().collect();
        assert_eq!(index_range(&down, 2.5, 5.5), Some(3..8));
        assert_eq!(index_range(&up, 20.0, 30.0), None);
    }

    #[test]
    fn fetched_values_become_fields_north_to_south() {
        let t = Utc::now();
        let lat = [48.0, 48.5, 49.0];
        let lon = [-5.0, -4.0];
        // uo: the cell index; utide: 10 everywhere.
        let uo: Vec<f32> = (0..6).map(|i| i as f32).collect();
        let vals = vec![uo.clone(), uo.clone(), vec![10.0; 6], vec![10.0; 6]];
        let f = assemble(&vals, &["uo", "vo", "utide", "vtide"], false, &lat, &lon, &[(0, t)]);
        assert_eq!(f.iter().map(|f| f.param).collect::<Vec<_>>(), vec![Param::CurrentU, Param::CurrentV, Param::ResidualU, Param::ResidualV]);
        let g = &f[0].grid;
        // The northernmost row first: latitude 49 is cells 4 and 5 of the ascending input.
        assert_eq!(g.data[..2], [14.0, 15.0]);
        assert!((g.yll - 47.75).abs() < 1e-9 && (g.xll + 5.5).abs() < 1e-9);
        assert_eq!(g.sample(-5.0, 49.0), Some(14.0));
        assert_eq!(f[2].grid.sample(-5.0, 49.0), Some(4.0), "the residual has no tide");
    }

    /// `cargo test --release -p isomar-core copernicus -- --ignored --nocapture`: SMOC and the North-West
    /// Shelf over the Alderney Race for the next two days, the bytes each costs, and the race's peak.
    #[test]
    #[ignore]
    fn live_alderney_race() {
        let dir = tempfile::tempdir().unwrap();
        let area = Bounds { min_lon: -2.4, min_lat: 49.55, max_lon: -1.8, max_lat: 49.85 };
        let cancel = AtomicBool::new(false);
        let from = Utc::now();
        let peak = |fields: &[Field], p: (Param, Param)| -> f64 {
            let mut best = 0.0f64;
            for u in fields.iter().filter(|f| f.param == p.0) {
                let v = fields.iter().find(|f| f.param == p.1 && f.time == u.time).unwrap();
                for (a, b) in u.grid.data.iter().zip(&v.grid.data) {
                    if a.is_finite() && b.is_finite() {
                        best = best.max((a.hypot(*b) * crate::weather::model::units::MS_TO_KN) as f64);
                    }
                }
            }
            best
        };
        for (src, vars) in [(CurrentSource::NwShelf, vec!["uo", "vo"]), (CurrentSource::Smoc, vec!["uo", "vo", "utide", "vtide"])] {
            let d = dataset(src).unwrap();
            let last = Mutex::new(Progress::default());
            let started = std::time::Instant::now();
            let fields = fetch(d, &vars, &area, from, dir.path(), &cancel, &|p| *last.lock().unwrap() = p).unwrap();
            let p = last.lock().unwrap().clone();
            let hours = fields.iter().filter(|f| f.param == Param::CurrentU).count();
            let kn = peak(&fields, (Param::CurrentU, Param::CurrentV));
            eprintln!("{}: {hours} hours, {} chunks, {} KB in {:.1} s; peak {kn:.1} kn", src.label(), p.chunks_total, p.bytes / 1024, started.elapsed().as_secs_f64());
            assert!(hours >= 48, "two days at least");
            assert!(kn > 2.0, "the race runs");
            let again = std::time::Instant::now();
            let cached = fetch(d, &vars, &area, from, dir.path(), &cancel, &|_| {}).unwrap();
            assert_eq!(cached.len(), fields.len(), "from the cache");
            eprintln!("  again from the cache in {:.2} s", again.elapsed().as_secs_f64());
        }
    }

    /// The router on the North-West Shelf's current through the Alderney Race (no land: the physics
    /// only), every two hours over a tide: the passage time swings with the stream by hours. Measured
    /// 2026-11-09, near springs: 363 min with no current, 194 to 356 with it.
    /// `cargo test --release -p isomar-core live_route -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_route_through_the_alderney_race() {
        use crate::geo::LatLon;
        use crate::routing::isochrone::{ConstantWind, explore};
        use crate::routing::land::NoLand;
        use crate::routing::polar::Polar;
        let dir = tempfile::tempdir().unwrap();
        let area = Bounds { min_lon: -2.6, min_lat: 49.5, max_lon: -1.6, max_lat: 49.9 };
        let cancel = AtomicBool::new(false);
        let now = Utc::now();
        let fields = fetch(dataset(CurrentSource::NwShelf).unwrap(), &["uo", "vo"], &area, now, dir.path(), &cancel, &|_| {}).unwrap();
        let mut field = super::super::CurrentField::new();
        let mut store = crate::weather::store::WeatherStore::new();
        store.extend(fields);
        field.add_store(CurrentSource::NwShelf, store);
        let polar = Polar::parse("TWA;6;12;20\n0;0;0;0\n40;0;0;0\n45;4;5.5;6\n60;5;6.5;7\n90;5.5;7;7.5\n135;5;6.5;7.5\n180;4;5.5;6.5\n").unwrap();
        let wind = ConstantWind { kn: 12.0, from: 270.0 };
        let (start, dest) = (LatLon::new(49.78, -1.75), LatLon::new(49.62, -2.35));
        let mut s = crate::routing::passage::settings_for_distance(start.distance_nm(&dest));
        crate::routing::passage::for_currents(&mut s, 25.0, 3.0);
        let calm = explore(start, dest, now, &polar, &wind, &NoLand, &s, None).unwrap().arrived.unwrap().duration().num_minutes();
        let env = super::super::WithCurrents { wind: ConstantWind { kn: 12.0, from: 270.0 }, currents: &field };
        let mut mins = Vec::new();
        for h in (0..=12).step_by(2) {
            let t = now + Duration::hours(h);
            let started = std::time::Instant::now();
            let m = explore(start, dest, t, &polar, &env, &NoLand, &s, None).ok().and_then(|e| e.arrived).map(|a| a.duration().num_minutes());
            eprintln!("leave +{h:2} h: {:?} min ({} ms)", m, started.elapsed().as_millis());
            mins.extend(m);
        }
        let (lo, hi) = (*mins.iter().min().unwrap(), *mins.iter().max().unwrap());
        eprintln!("no current: {calm} min; with it {lo} to {hi} min");
        // Beating into the wind, even a foul stream gives more wind over the water, and over six hours
        // the stream turns: the times swing with the tide rather than straddle the calm one.
        assert!(lo < calm, "the fair stream shortens it");
        assert!(hi - lo > 60, "an hour at least between the best and the worst departure");
    }
}
