//! Fetching a plan's files: in parallel, with progress and cancellation,
//! verifying each file before it enters the cache.

use super::cache::Cache;
use super::grib::looks_complete;
use super::planner::{Area, Fetch, Job, Plan};
use crate::chart::grid::Grid;
use crate::vector::Bounds;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const PARALLEL: usize = 4;
const ATTEMPTS: u32 = 3;
/// The gap between two requests, across every download in the process. NOMADS
/// blocks a client for a few minutes over about 120 requests a minute; this
/// is 100.
const INTERVAL: Duration = if cfg!(test) { Duration::from_millis(1) } else { Duration::from_millis(600) };
/// How long everything waits after the server turned a request away, times
/// the attempt number.
const BLOCK_PAUSE: Duration = if cfg!(test) { Duration::from_millis(20) } else { Duration::from_secs(30) };

static THROTTLE: Throttle = Throttle { next: Mutex::new(None) };

/// Hands out request slots `interval` apart, to every thread that asks.
struct Throttle {
    next: Mutex<Option<Instant>>,
}

impl Throttle {
    /// Waits for the next free slot. False if `cancelled` said so meanwhile.
    fn wait(&self, interval: Duration, cancelled: &dyn Fn() -> bool) -> bool {
        let slot = {
            let mut next = self.next.lock().unwrap();
            let now = Instant::now();
            let slot = next.map_or(now, |t| t.max(now));
            *next = Some(slot + interval);
            slot
        };
        loop {
            let now = Instant::now();
            if now >= slot {
                return true;
            }
            if cancelled() {
                return false;
            }
            std::thread::sleep((slot - now).min(Duration::from_millis(100)));
        }
    }

    /// Gives out no slot for `d` from now.
    fn pause(&self, d: Duration) {
        let mut next = self.next.lock().unwrap();
        let until = Instant::now() + d;
        *next = Some(next.map_or(until, |t| t.max(until)));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DlError {
    Cancelled,
    /// The server does not have the file (yet): wrong or unpublished run.
    NotAvailable(String),
    /// The server turned the request away for coming too fast: NOMADS's CDN
    /// answers a 302 with no Location (or a 429) while it blocks a client.
    Throttled(String),
    Http(String),
    Io(String),
}

impl std::fmt::Display for DlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DlError::Cancelled => write!(f, "cancelled"),
            DlError::NotAvailable(s) => write!(f, "not available: {s}"),
            DlError::Throttled(s) => write!(f, "the server is turning requests away for a while: {s}"),
            DlError::Http(s) => write!(f, "network error: {s}"),
            DlError::Io(s) => write!(f, "disk error: {s}"),
        }
    }
}

impl std::error::Error for DlError {}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub bytes_done: u64,
    pub bytes_estimated: u64,
    pub files_done: usize,
    pub files_total: usize,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(Duration::from_secs(180)))
        // Redirects are followed by `get`: a 3xx without a Location header
        // (NOMADS's block page) is an error in ureq's own handling, and here
        // it is a reason to wait.
        .max_redirects(0)
        .http_status_as_error(false)
        .user_agent("isomar/0.1 (open-source chart plotter)")
        .build()
        .into()
}

fn fetch_one(agent: &ureq::Agent, job: &Job, cache: &Cache, cancel: &AtomicBool, stop: &AtomicBool, bytes: &AtomicU64) -> Result<PathBuf, DlError> {
    let path = cache.job_path(job);
    if let Ok(existing) = std::fs::read(&path)
        && looks_complete(&existing)
    {
        bytes.fetch_add(existing.len() as u64, Ordering::Relaxed);
        return Ok(path);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| DlError::Io(e.to_string()))?;
    }
    let part = path.with_extension("part");
    let mut last = DlError::Http("no attempt made".into());
    for attempt in 0..ATTEMPTS {
        if cancel.load(Ordering::Relaxed) || stop.load(Ordering::Relaxed) {
            return Err(DlError::Cancelled);
        }
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(400 * 2u64.pow(attempt)));
        }
        let got = match &job.fetch {
            Fetch::Whole => attempt_download(agent, &job.url, &part, cancel, stop, bytes),
            Fetch::Ranges { index, params, crop } => attempt_ranges(agent, &job.url, index, params, crop, &part, cancel, stop, bytes),
            Fetch::Files { urls, crop } => attempt_files(agent, urls, crop, &part, cancel, stop, bytes),
        };
        match got {
            Ok(()) => {
                std::fs::rename(&part, &path).map_err(|e| DlError::Io(e.to_string()))?;
                return Ok(path);
            }
            Err(e @ (DlError::Cancelled | DlError::NotAvailable(_) | DlError::Io(_))) => {
                let _ = std::fs::remove_file(&part);
                return Err(e);
            }
            Err(e) => {
                crate::log!("weather: attempt {}/{ATTEMPTS} for f{:03} of {} failed: {e}", attempt + 1, job.fhour, job.run);
                if matches!(e, DlError::Throttled(_)) && attempt + 1 < ATTEMPTS {
                    // Every worker waits: the block is on us, not on this file.
                    let d = BLOCK_PAUSE * (attempt + 1);
                    crate::log!("weather: pausing all requests for {d:?}");
                    THROTTLE.pause(d);
                }
                last = e;
            }
        }
    }
    let _ = std::fs::remove_file(&part);
    crate::log!("weather: giving up on f{:03} of {}: {last}", job.fhour, job.run);
    Err(last)
}

const MAX_REDIRECTS: usize = 5;

/// Whether requests to `url` wait their turn behind the NOMADS throttle. ECMWF's open-data server
/// takes hundreds of connections at once, and a step of it is several requests.
fn throttled(url: &str) -> bool {
    !url.starts_with("https://data.ecmwf.int/") && !url.starts_with("https://opendata.dwd.de/")
}

/// A GET that follows redirects itself; anything but a 2xx is an error. With `range` (first and
/// last byte), only those bytes: a server that sends the whole file instead is an error too.
fn get(agent: &ureq::Agent, url: &str, range: Option<(u64, u64)>, cancelled: &dyn Fn() -> bool) -> Result<ureq::http::Response<ureq::Body>, DlError> {
    let mut url = url.to_string();
    for _ in 0..=MAX_REDIRECTS {
        if throttled(&url) && !THROTTLE.wait(INTERVAL, cancelled) {
            return Err(DlError::Cancelled);
        }
        if cancelled() {
            return Err(DlError::Cancelled);
        }
        let started = std::time::Instant::now();
        let mut req = agent.get(&url);
        if let Some((first, last)) = range {
            req = req.header("Range", format!("bytes={first}-{last}"));
        }
        let resp = match req.call() {
            Ok(r) => r,
            Err(e) => {
                crate::log!("weather: GET {url} failed after {:?}: {e}", started.elapsed());
                return Err(DlError::Http(e.to_string()));
            }
        };
        let code = resp.status().as_u16();
        if resp.status().is_success() {
            if range.is_some() && code != 206 {
                return Err(DlError::Http(format!("HTTP {code}: the server sent the whole file, not the part asked for")));
            }
            if range.is_none() {
                crate::log!("weather: GET {url} -> {code} in {:?}", started.elapsed());
            }
            return Ok(resp);
        }
        let headers: Vec<String> = resp.headers().iter().map(|(k, v)| format!("{k}: {}", String::from_utf8_lossy(v.as_bytes()))).collect();
        crate::log!("weather: GET {url} -> {code} in {:?}; headers: [{}]", started.elapsed(), headers.join("; "));
        if resp.status().is_redirection() {
            let location = resp.headers().get("location").map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned());
            let target = location.as_deref().and_then(|l| resolve(&url, l));
            match target {
                Some(t) => {
                    url = t;
                    continue;
                }
                // NOMADS's CDN answers this way while it blocks us.
                None => return Err(DlError::Throttled(format!("HTTP {code} without a usable Location header (Location: {location:?})"))),
            }
        }
        return Err(match code {
            404 | 403 | 410 => DlError::NotAvailable(format!("HTTP {code}")),
            429 => DlError::Throttled(format!("HTTP {code}")),
            _ => DlError::Http(format!("HTTP {code}")),
        });
    }
    Err(DlError::Http("too many redirects".into()))
}

/// `location` as an absolute URL, given the URL it was a reply to.
fn resolve(base: &str, location: &str) -> Option<String> {
    let location = location.trim();
    if location.starts_with("http://") || location.starts_with("https://") {
        return Some(location.to_string());
    }
    let origin_end = base.find("://")? + 3;
    let origin_end = base[origin_end..].find('/').map_or(base.len(), |i| origin_end + i);
    let path = location.strip_prefix('/')?;
    Some(format!("{}/{}", &base[..origin_end], path))
}

fn attempt_download(agent: &ureq::Agent, url: &str, part: &std::path::Path, cancel: &AtomicBool, stop: &AtomicBool, bytes: &AtomicU64) -> Result<(), DlError> {
    let mut resp = get(agent, url, None, &|| cancel.load(Ordering::Relaxed) || stop.load(Ordering::Relaxed))?;
    let mut file = std::fs::File::create(part).map_err(|e| DlError::Io(e.to_string()))?;
    let mut reader = resp.body_mut().as_reader();
    let mut buf = [0u8; 16 * 1024];
    let mut head = Vec::new();
    let mut tail = [0u8; 4];
    let mut total = 0usize;
    loop {
        if cancel.load(Ordering::Relaxed) || stop.load(Ordering::Relaxed) {
            return Err(DlError::Cancelled);
        }
        let n = reader.read(&mut buf).map_err(|e| DlError::Http(e.to_string()))?;
        if n == 0 {
            break;
        }
        if head.len() < 4 {
            head.extend_from_slice(&buf[..n.min(4 - head.len())]);
        }
        file.write_all(&buf[..n]).map_err(|e| DlError::Io(e.to_string()))?;
        // Remember the last four bytes seen.
        let chunk = &buf[..n];
        if n >= 4 {
            tail.copy_from_slice(&chunk[n - 4..]);
        } else {
            tail.rotate_left(n);
            tail[4 - n..].copy_from_slice(chunk);
        }
        total += n;
        bytes.fetch_add(n as u64, Ordering::Relaxed);
    }
    drop(file);
    // The NOMADS filter answers a missing file with an HTML page, and a cut
    // connection leaves a short file; neither is GRIB.
    if total < 20 || head != b"GRIB" || &tail != b"7777" {
        let body = std::fs::read(part).unwrap_or_default();
        let msg = String::from_utf8_lossy(&body[..body.len().min(80)]).replace('\n', " ");
        return Err(if head == b"GRIB" { DlError::Http("download was cut short".into()) } else { DlError::NotAvailable(msg) });
    }
    Ok(())
}

/// Reads a response's body, counting the bytes and stopping when told to.
fn read_body(resp: &mut ureq::http::Response<ureq::Body>, cancel: &AtomicBool, stop: &AtomicBool, bytes: &AtomicU64) -> Result<Vec<u8>, DlError> {
    let mut reader = resp.body_mut().as_reader();
    let mut buf = [0u8; 16 * 1024];
    let mut out = Vec::new();
    loop {
        if cancel.load(Ordering::Relaxed) || stop.load(Ordering::Relaxed) {
            return Err(DlError::Cancelled);
        }
        let n = reader.read(&mut buf).map_err(|e| DlError::Http(e.to_string()))?;
        if n == 0 {
            return Ok(out);
        }
        out.extend_from_slice(&buf[..n]);
        bytes.fetch_add(n as u64, Ordering::Relaxed);
    }
}

/// Where each of `params` lies in the file an ECMWF index describes: (offset, length) of the
/// surface field of that name, in the order asked for. A parameter the index lacks is left out.
pub fn index_ranges(index: &str, params: &[&str]) -> Vec<(u64, u64)> {
    let entries: Vec<serde_json::Value> = index.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    params
        .iter()
        .filter_map(|p| {
            let e = entries.iter().find(|e| {
                e.get("param").and_then(|v| v.as_str()) == Some(*p)
                    && e.get("levtype").and_then(|v| v.as_str()).is_none_or(|l| l == "sfc")
                    && e.get("number").is_none()
            })?;
            Some((e.get("_offset")?.as_u64()?, e.get("_length")?.as_u64()?))
        })
        .collect()
}

/// Cuts `g` to `area`. A whole-globe grid is first turned to start west of the area, so an area
/// across the antimeridian (or the grid's own seam) comes out in one piece.
pub fn crop(g: &Grid, area: &Area) -> Option<Grid> {
    let width = area.width_deg();
    let b = |west: f64| Bounds { min_lon: west, max_lon: west + width, min_lat: area.south, max_lat: area.north };
    let n = g.ncols;
    let global = ((n as f64 * g.dx) - 360.0).abs() < g.dx / 2.0;
    if !global || n == 0 {
        return g.clipped(&b(area.west));
    }
    let first = g.xll + g.dx / 2.0;
    let k = (((area.west - 1.0 - first) / g.dx).floor() as i64).rem_euclid(n as i64) as usize;
    let mut data = Vec::with_capacity(g.data.len());
    for row in g.data.chunks(n) {
        data.extend_from_slice(&row[k..]);
        data.extend_from_slice(&row[..k]);
    }
    let turned = Grid { xll: g.xll + k as f64 * g.dx, data, ..g.clone() };
    turned.clipped(&b(area.west))
}

/// Reads the fields `params` of the whole-globe file at `url`, by byte range from its index, and
/// writes them cropped to `area` as GRIB2 to `part`.
#[allow(clippy::too_many_arguments)]
fn attempt_ranges(agent: &ureq::Agent, url: &str, index: &str, params: &[&str], area: &Area, part: &std::path::Path, cancel: &AtomicBool, stop: &AtomicBool, bytes: &AtomicU64) -> Result<(), DlError> {
    let cancelled = || cancel.load(Ordering::Relaxed) || stop.load(Ordering::Relaxed);
    let mut resp = get(agent, index, None, &cancelled)?;
    let text = String::from_utf8(read_body(&mut resp, cancel, stop, bytes)?).map_err(|e| DlError::Http(e.to_string()))?;
    let ranges = index_ranges(&text, params);
    if ranges.is_empty() {
        return Err(DlError::NotAvailable(format!("none of {params:?} in {index}")));
    }
    let started = std::time::Instant::now();
    let mut out = Vec::new();
    let mut fetched = 0u64;
    for (offset, len) in ranges {
        let mut resp = get(agent, url, Some((offset, offset + len - 1)), &cancelled)?;
        let body = read_body(&mut resp, cancel, stop, bytes)?;
        if body.len() as u64 != len || !looks_complete(&body) {
            return Err(DlError::Http("a field was cut short".into()));
        }
        fetched += len;
        for f in super::grib::read_fields(&body).map_err(|e| DlError::Http(e.to_string()))? {
            if let Some(grid) = crop(&f.grid, area) {
                out.extend(super::gribwrite::encode(&super::model::Field { grid, ..f }));
            }
        }
    }
    crate::log!("weather: {url}: {} fields, {fetched} bytes in {:?}, {} cached", params.len(), started.elapsed(), out.len());
    if out.is_empty() {
        return Err(DlError::NotAvailable(format!("nothing of {params:?} over the area")));
    }
    std::fs::write(part, &out).map_err(|e| DlError::Io(e.to_string()))
}

/// Reads whole files, one a field (bz2 when the name ends so), and writes their fields cropped to
/// `area` as GRIB2 to `part`.
fn attempt_files(agent: &ureq::Agent, urls: &[String], area: &Area, part: &std::path::Path, cancel: &AtomicBool, stop: &AtomicBool, bytes: &AtomicU64) -> Result<(), DlError> {
    let cancelled = || cancel.load(Ordering::Relaxed) || stop.load(Ordering::Relaxed);
    let mut out = Vec::new();
    for url in urls {
        let mut resp = get(agent, url, None, &cancelled)?;
        let body = read_body(&mut resp, cancel, stop, bytes)?;
        let grib = if url.ends_with(".bz2") {
            let mut raw = Vec::new();
            bzip2::read::BzDecoder::new(body.as_slice()).read_to_end(&mut raw).map_err(|e| DlError::Http(format!("{url}: {e}")))?;
            raw
        } else {
            body
        };
        if !looks_complete(&grib) {
            return Err(DlError::Http(format!("{url}: not a whole GRIB file")));
        }
        for f in super::grib::read_fields(&grib).map_err(|e| DlError::Http(e.to_string()))? {
            if let Some(grid) = crop(&f.grid, area) {
                out.extend(super::gribwrite::encode(&super::model::Field { grid, ..f }));
            }
        }
    }
    if out.is_empty() {
        return Err(DlError::NotAvailable("nothing over the area".into()));
    }
    std::fs::write(part, &out).map_err(|e| DlError::Io(e.to_string()))
}

/// What a plan's download brought.
#[derive(Debug, Default)]
pub struct Fetched {
    /// The files that arrived, in the plan's order.
    pub paths: Vec<PathBuf>,
    /// The indices in `plan.jobs` of the ones the run does not have (yet).
    pub missing: Vec<usize>,
}

/// Downloads every job of `plan` into `cache`. `on_progress` is called from
/// worker threads. A run is published an hour at a time, so a file the server
/// does not have means the later hours are not there either: they are not
/// asked for, and come back in `missing`. Any other failure stops the others
/// and fails the whole plan.
pub fn fetch_plan(plan: &Plan, cache: &Cache, cancel: &AtomicBool, on_progress: &(dyn Fn(Progress) + Sync)) -> Result<Fetched, DlError> {
    let agent = agent();
    crate::log!("weather: fetching {} files of the {} run, about {} bytes, {PARALLEL} at a time", plan.jobs.len(), plan.jobs.first().map(|j| j.run.to_string()).unwrap_or_default(), plan.est_bytes);
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let bytes = AtomicU64::new(0);
    let failed = AtomicBool::new(false);
    let first_error: Mutex<Option<DlError>> = Mutex::new(None);
    let results: Mutex<Vec<(usize, PathBuf)>> = Mutex::new(Vec::new());
    let total = plan.jobs.len();
    let stop = AtomicBool::new(false);
    // The first forecast hour the server said it does not have.
    let cutoff = AtomicU32::new(u32::MAX);
    let report = || {
        on_progress(Progress { bytes_done: bytes.load(Ordering::Relaxed), bytes_estimated: plan.est_bytes, files_done: done.load(Ordering::Relaxed), files_total: total });
    };
    report();
    std::thread::scope(|s| {
        for _ in 0..PARALLEL.min(total.max(1)) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= total || failed.load(Ordering::Relaxed) {
                        return;
                    }
                    let job = &plan.jobs[i];
                    if job.fhour >= cutoff.load(Ordering::Relaxed) {
                        continue;
                    }
                    // The user's cancel stops us; so does a sibling's failure.
                    match fetch_one(&agent, job, cache, cancel, &stop, &bytes) {
                        Ok(p) => {
                            results.lock().unwrap().push((i, p));
                            done.fetch_add(1, Ordering::Relaxed);
                            report();
                        }
                        Err(DlError::NotAvailable(m)) => {
                            crate::log!("weather: f{:03} of {} is not published ({m}); not asking for later hours", job.fhour, job.run);
                            cutoff.fetch_min(job.fhour, Ordering::Relaxed);
                        }
                        Err(e) => {
                            failed.store(true, Ordering::Relaxed);
                            stop.store(true, Ordering::Relaxed);
                            let mut fe = first_error.lock().unwrap();
                            // Keep the most informative error: not a consequence of stopping.
                            if fe.is_none() || (*fe == Some(DlError::Cancelled) && e != DlError::Cancelled) {
                                *fe = Some(e);
                            }
                            return;
                        }
                    }
                }
            });
        }
    });
    if let Some(e) = first_error.into_inner().unwrap() {
        crate::log!("weather: fetch stopped: {e}");
        return Err(if cancel.load(Ordering::Relaxed) { DlError::Cancelled } else { e });
    }
    let mut r = results.into_inner().unwrap();
    r.sort();
    let missing = (0..total).filter(|i| r.binary_search_by_key(i, |(j, _)| *j).is_err()).collect();
    Ok(Fetched { paths: r.into_iter().map(|(_, p)| p).collect(), missing })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::weather::model::Param;
    use crate::weather::planner::Model;
    use crate::weather::testgrib::{TestField, encode};
    use chrono::{TimeZone, Utc};
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::Arc;

    enum Reply {
        Body(Vec<u8>),
        Status(u16),
        /// A 302 to another path of the same server.
        Redirect(&'static str),
        /// Claims a longer body than it sends, then hangs up.
        Truncated(Vec<u8>),
        /// Sends data slowly so a cancel can land.
        Slow(Vec<u8>),
        /// Answers a `Range` request with those bytes (206), as ECMWF's server does.
        Ranged(Vec<u8>),
    }

    struct Server {
        port: u16,
        hits: Arc<Mutex<HashMap<String, usize>>>,
        stop: Arc<AtomicBool>,
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            let _ = std::net::TcpStream::connect(("127.0.0.1", self.port));
        }
    }

    fn serve(routes: Vec<(&'static str, Reply)>) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let hits: Arc<Mutex<HashMap<String, usize>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let routes = Arc::new(routes);
        let (h2, s2) = (hits.clone(), stop.clone());
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                if s2.load(Ordering::Relaxed) {
                    return;
                }
                let Ok(mut stream) = conn else { continue };
                let (routes, hits) = (routes.clone(), h2.clone());
                std::thread::spawn(move || {
                    let mut r = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    if r.read_line(&mut line).is_err() {
                        return;
                    }
                    let path = line.split(' ').nth(1).unwrap_or("/").to_string();
                    let mut range: Option<(usize, usize)> = None;
                    loop {
                        let mut l = String::new();
                        if r.read_line(&mut l).unwrap_or(0) <= 2 {
                            break;
                        }
                        if let Some(v) = l.to_ascii_lowercase().strip_prefix("range: bytes=") {
                            let (a, b) = v.trim().split_once('-').unwrap();
                            range = Some((a.parse().unwrap(), b.parse().unwrap()));
                        }
                    }
                    *hits.lock().unwrap().entry(path.clone()).or_default() += 1;
                    let reply = routes.iter().find(|(p, _)| path.starts_with(p)).map(|(_, r)| r);
                    let _ = match reply {
                        Some(Reply::Body(b)) => {
                            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", b.len());
                            stream.write_all(b)
                        }
                        Some(Reply::Truncated(b)) => {
                            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", b.len() + 500);
                            stream.write_all(b)
                        }
                        Some(Reply::Slow(b)) => {
                            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", b.len());
                            for chunk in b.chunks(8) {
                                if stream.write_all(chunk).is_err() {
                                    break;
                                }
                                std::thread::sleep(Duration::from_millis(30));
                            }
                            Ok(())
                        }
                        Some(Reply::Ranged(b)) => {
                            let (a, z) = range.unwrap_or((0, b.len() - 1));
                            let part = &b[a..=z.min(b.len() - 1)];
                            let _ = write!(stream, "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", part.len());
                            stream.write_all(part)
                        }
                        Some(Reply::Redirect(to)) => write!(stream, "HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
                        Some(Reply::Status(c)) => write!(stream, "HTTP/1.1 {c} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
                        None => write!(stream, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
                    };
                });
            }
        });
        Server { port, hits, stop }
    }

    fn job(port: u16, path: &str, n: u32) -> Job {
        let run = Utc.with_ymd_and_hms(2026, 10, 26, 0, 0, 0).unwrap();
        Job {
            model: Model::Gfs,
            run,
            fhour: n,
            url: format!("http://127.0.0.1:{port}{path}"),
            cache_name: format!("test_{n}.grib2"),
            est_bytes: 1000,
            params: vec![Param::WindU],
            valid: run,
            fetch: Fetch::Whole,
            member: None,
        }
    }

    /// A whole-globe field every 30 degrees: columns from 180 W, rows from 60 N to 60 S. Its value is
    /// its longitude, plus `add`.
    fn globe(number: u8, add: f32) -> Vec<u8> {
        let mut f = TestField::wind_u(6, 12, 5, (0..60).map(|i| -180.0 + 30.0 * (i % 12) as f32 + add).collect());
        f.number = number;
        (f.lat_first, f.lon_first, f.dlat, f.dlon) = (60.0, 180.0, 30.0, 30.0);
        encode(&f)
    }

    #[test]
    fn the_index_gives_the_surface_fields_asked_for() {
        let index = [
            r#"{"param": "10u", "levtype": "pl", "levelist": "1000", "_offset": 0, "_length": 10}"#,
            r#"{"param": "msl", "levtype": "sfc", "_offset": 10, "_length": 20}"#,
            r#"{"param": "10u", "levtype": "sfc", "_offset": 30, "_length": 40}"#,
            r#"{"param": "10u", "levtype": "sfc", "number": "3", "_offset": 70, "_length": 40}"#,
            "not json",
        ]
        .join("\n");
        assert_eq!(index_ranges(&index, &["10u", "msl", "10fg"]), vec![(30, 40), (10, 20)]);
    }

    #[test]
    fn a_whole_globe_field_is_cropped_across_the_antimeridian_in_one_piece() {
        let f = &crate::weather::grib::read_fields(&globe(2, 0.0)).unwrap()[0];
        let g = crop(&f.grid, &Area::new(-20.0, 140.0, 20.0, -140.0)).unwrap();
        // From 120 E to 90 W, the columns either side of the area kept as a margin, in one piece.
        assert_eq!(g.ncols, 6, "{g:?}");
        assert_eq!(&g.data[..6], &[120.0, 150.0, -180.0, -150.0, -120.0, -90.0]);
        assert!((crate::weather::store::sample_wrapped(&g, 180.0, 0.0).unwrap() + 180.0).abs() < 1e-3);
        // An area away from the seam is a plain cut.
        let g = crop(&f.grid, &Area::new(-10.0, -10.0, 10.0, 10.0)).unwrap();
        assert!(g.ncols < 6 && g.data[..g.ncols].windows(2).all(|w| w[1] - w[0] == 30.0), "{g:?}");
        assert!(crate::weather::store::sample_wrapped(&g, 0.0, 0.0).unwrap().abs() < 1e-3);
    }

    #[test]
    fn a_ranged_job_reads_its_fields_from_the_index_and_caches_them_cropped() {
        let (u, junk, v) = (globe(2, 0.0), globe(22, 0.0), globe(3, 0.5));
        let file = [u.clone(), junk.clone(), v.clone()].concat();
        let index = format!(
            "{{\"param\": \"10u\", \"levtype\": \"sfc\", \"_offset\": 0, \"_length\": {}}}\n{{\"param\": \"10fg\", \"levtype\": \"sfc\", \"_offset\": {}, \"_length\": {}}}\n{{\"param\": \"10v\", \"levtype\": \"sfc\", \"_offset\": {}, \"_length\": {}}}\n",
            u.len(), u.len(), junk.len(), u.len() + junk.len(), v.len()
        );
        let srv = serve(vec![("/step.index", Reply::Body(index.into_bytes())), ("/step.grib2", Reply::Ranged(file))]);
        let mut j = job(srv.port, "/step.grib2", 6);
        j.fetch = Fetch::Ranges { index: format!("http://127.0.0.1:{}/step.index", srv.port), params: vec!["10u", "10v"], crop: Area::new(-20.0, -40.0, 20.0, 40.0) };
        let dir = tempfile::tempdir().unwrap();
        let got = fetch_plan(&plan_of(vec![j]), &Cache::new(dir.path()), &AtomicBool::new(false), &quiet).unwrap();
        let fields = crate::weather::grib::read_fields(&std::fs::read(&got.paths[0]).unwrap()).unwrap();
        assert_eq!(fields.iter().map(|f| f.param).collect::<Vec<_>>(), vec![Param::WindU, Param::WindV], "the gusts were not asked for");
        assert!(fields.iter().all(|f| f.grid.ncols < 12), "cropped");
        let at = |f: &crate::weather::model::Field| crate::weather::store::sample_wrapped(&f.grid, 30.0, 0.0).unwrap();
        assert!((at(&fields[0]) - 30.0).abs() < 0.01 && (at(&fields[1]) - 30.5).abs() < 0.01);
        assert_eq!(srv.hits.lock().unwrap().get("/step.grib2"), Some(&2), "one request a field");
    }

    #[test]
    fn a_step_the_index_is_not_published_for_is_missing() {
        let srv = serve(vec![]);
        let mut j = job(srv.port, "/step.grib2", 6);
        j.fetch = Fetch::Ranges { index: format!("http://127.0.0.1:{}/step.index", srv.port), params: vec!["10u"], crop: Area::new(-20.0, -40.0, 20.0, 40.0) };
        let dir = tempfile::tempdir().unwrap();
        let got = fetch_plan(&plan_of(vec![j]), &Cache::new(dir.path()), &AtomicBool::new(false), &quiet).unwrap();
        assert_eq!(got.missing, vec![0]);
    }

    fn plan_of(jobs: Vec<Job>) -> Plan {
        Plan { run: jobs[0].run, est_bytes: jobs.iter().map(|j| j.est_bytes).sum(), jobs, description: String::new(), unavailable: vec![] }
    }

    fn grib() -> Vec<u8> {
        encode(&TestField::wind_u(0, 3, 3, (0..9).map(|i| i as f32).collect()))
    }

    fn quiet(_: Progress) {}

    #[test]
    fn downloads_verifies_and_reports_progress() {
        let g = grib();
        let srv = serve(vec![("/a", Reply::Body(g.clone())), ("/b", Reply::Body(g.clone()))]);
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let plan = plan_of(vec![job(srv.port, "/a", 0), job(srv.port, "/b", 3)]);
        let seen: Mutex<Vec<Progress>> = Mutex::new(Vec::new());
        let paths = fetch_plan(&plan, &cache, &AtomicBool::new(false), &|p| seen.lock().unwrap().push(p)).unwrap().paths;
        assert_eq!(paths.len(), 2);
        for p in &paths {
            assert_eq!(std::fs::read(p).unwrap(), g);
        }
        let last = seen.lock().unwrap().iter().max_by_key(|p| p.files_done).cloned().unwrap();
        assert_eq!((last.files_done, last.files_total), (2, 2));
        assert_eq!(last.bytes_done, 2 * g.len() as u64);
        assert!(!paths[0].with_extension("part").exists());
    }

    #[test]
    fn cached_files_are_not_fetched_again() {
        let srv = serve(vec![("/a", Reply::Body(grib()))]);
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let plan = plan_of(vec![job(srv.port, "/a", 0)]);
        fetch_plan(&plan, &cache, &AtomicBool::new(false), &quiet).unwrap();
        fetch_plan(&plan, &cache, &AtomicBool::new(false), &quiet).unwrap();
        assert_eq!(srv.hits.lock().unwrap()["/a"], 1);
    }

    #[test]
    fn a_missing_file_is_reported_as_missing() {
        let srv = serve(vec![("/gone", Reply::Status(404))]);
        let dir = tempfile::tempdir().unwrap();
        let plan = plan_of(vec![job(srv.port, "/gone", 0)]);
        let got = fetch_plan(&plan, &Cache::new(dir.path()), &AtomicBool::new(false), &quiet).unwrap();
        assert!(got.paths.is_empty());
        assert_eq!(got.missing, vec![0]);
        assert_eq!(srv.hits.lock().unwrap()["/gone"], 1, "a 404 is not retried");
    }

    #[test]
    fn a_redirect_is_followed() {
        let g = grib();
        let srv = serve(vec![("/old", Reply::Redirect("/new")), ("/new", Reply::Body(g.clone()))]);
        let dir = tempfile::tempdir().unwrap();
        let plan = plan_of(vec![job(srv.port, "/old", 0)]);
        let paths = fetch_plan(&plan, &Cache::new(dir.path()), &AtomicBool::new(false), &quiet).unwrap().paths;
        assert_eq!(std::fs::read(&paths[0]).unwrap(), g);
    }

    #[test]
    fn a_redirect_without_a_location_is_retried_then_reported() {
        let srv = serve(vec![("/odd", Reply::Status(302))]);
        let dir = tempfile::tempdir().unwrap();
        let plan = plan_of(vec![job(srv.port, "/odd", 0)]);
        let e = fetch_plan(&plan, &Cache::new(dir.path()), &AtomicBool::new(false), &quiet).unwrap_err();
        assert!(matches!(e, DlError::Throttled(_)), "{e}");
        assert_eq!(srv.hits.lock().unwrap().get("/odd").copied(), Some(ATTEMPTS as usize));
    }

    #[test]
    fn relative_locations_resolve_against_the_origin() {
        assert_eq!(resolve("https://a.b/x/y?q=1", "/z?r=2").as_deref(), Some("https://a.b/z?r=2"));
        assert_eq!(resolve("https://a.b", "/z").as_deref(), Some("https://a.b/z"));
        assert_eq!(resolve("https://a.b/x", "http://c.d/e").as_deref(), Some("http://c.d/e"));
        assert_eq!(resolve("https://a.b/x", "relative"), None);
    }

    #[test]
    fn an_html_error_page_is_not_mistaken_for_data() {
        let html = b"<html><body>Data file is not present, sorry</body></html>".to_vec();
        let srv = serve(vec![("/h", Reply::Body(html))]);
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let plan = plan_of(vec![job(srv.port, "/h", 0)]);
        let got = fetch_plan(&plan, &cache, &AtomicBool::new(false), &quiet).unwrap();
        assert_eq!(got.missing, vec![0]);
        assert!(cache.files().is_empty(), "nothing is cached");
    }

    #[test]
    fn a_cut_download_is_retried_then_fails_without_leaving_a_file() {
        let g = grib();
        let srv = serve(vec![("/t", Reply::Truncated(g[..g.len() - 10].to_vec()))]);
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let plan = plan_of(vec![job(srv.port, "/t", 0)]);
        let e = fetch_plan(&plan, &cache, &AtomicBool::new(false), &quiet).unwrap_err();
        assert!(matches!(e, DlError::Http(_)), "{e}");
        assert_eq!(srv.hits.lock().unwrap()["/t"], ATTEMPTS as usize);
        assert!(cache.files().is_empty());
        assert!(!cache.job_path(&plan.jobs[0]).with_extension("part").exists());
    }

    #[test]
    fn cancelling_stops_promptly_and_cleans_up() {
        let mut big = grib();
        // Pad the middle so the transfer takes a while; framing stays valid.
        let tail = big.split_off(big.len() - 4);
        big.extend(std::iter::repeat_n(0u8, 4000));
        big.extend(tail);
        let srv = serve(vec![("/s", Reply::Slow(big))]);
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let plan = plan_of(vec![job(srv.port, "/s", 0)]);
        let cancel = Arc::new(AtomicBool::new(false));
        let c2 = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            c2.store(true, Ordering::Relaxed);
        });
        let started = std::time::Instant::now();
        let e = fetch_plan(&plan, &cache, &cancel, &quiet).unwrap_err();
        assert_eq!(e, DlError::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(cache.files().is_empty());
        assert!(!cache.job_path(&plan.jobs[0]).with_extension("part").exists());
    }

    #[test]
    fn many_jobs_download_in_parallel_and_come_back_in_order() {
        let g = grib();
        let srv = serve(vec![("/p", Reply::Body(g))]);
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let jobs: Vec<Job> = (0..12).map(|i| job(srv.port, &format!("/p{i}"), i)).collect();
        let plan = plan_of(jobs);
        let paths = fetch_plan(&plan, &cache, &AtomicBool::new(false), &quiet).unwrap().paths;
        assert_eq!(paths.len(), 12);
        for (p, j) in paths.iter().zip(&plan.jobs) {
            assert_eq!(p, &cache.job_path(j));
        }
    }

    #[test]
    fn one_failure_fails_the_whole_plan() {
        let srv = serve(vec![("/ok", Reply::Body(grib())), ("/bad", Reply::Status(500))]);
        let dir = tempfile::tempdir().unwrap();
        let plan = plan_of(vec![job(srv.port, "/ok", 0), job(srv.port, "/bad", 1), job(srv.port, "/ok", 2)]);
        let e = fetch_plan(&plan, &Cache::new(dir.path()), &AtomicBool::new(false), &quiet).unwrap_err();
        assert!(matches!(e, DlError::Http(_)), "{e}");
    }

    #[test]
    fn hours_after_a_missing_one_are_not_asked_for() {
        let srv = serve(vec![("/ok", Reply::Body(grib())), ("/gone", Reply::Status(404))]);
        let dir = tempfile::tempdir().unwrap();
        // One at a time would be the plain case; with four workers the hours
        // just after the missing one may already be on their way.
        let mut jobs: Vec<Job> = (0..4).map(|i| job(srv.port, &format!("/ok{i}"), i)).collect();
        jobs.push(job(srv.port, "/gone", 4));
        jobs.extend((5..20).map(|i| job(srv.port, &format!("/ok{i}"), i)));
        let plan = plan_of(jobs);
        let got = fetch_plan(&plan, &Cache::new(dir.path()), &AtomicBool::new(false), &quiet).unwrap();
        assert!(got.paths.len() >= 4 && got.paths.len() <= 4 + PARALLEL, "{}", got.paths.len());
        assert!(got.missing.contains(&4) && got.missing.contains(&19));
        assert_eq!(got.paths.len() + got.missing.len(), 20);
        let hits = srv.hits.lock().unwrap();
        assert!(hits.keys().filter(|k| k.starts_with("/ok")).count() <= 4 + PARALLEL);
    }

    #[test]
    fn the_throttle_spaces_requests_out_and_pauses() {
        let t = Throttle { next: Mutex::new(None) };
        let gap = Duration::from_millis(40);
        let started = Instant::now();
        for _ in 0..4 {
            assert!(t.wait(gap, &|| false));
        }
        assert!(started.elapsed() >= gap * 3, "{:?}", started.elapsed());
        t.pause(Duration::from_millis(150));
        let paused = Instant::now();
        assert!(t.wait(gap, &|| false));
        assert!(paused.elapsed() >= Duration::from_millis(140), "{:?}", paused.elapsed());
        t.pause(Duration::from_secs(60));
        let cancelled = Instant::now();
        assert!(!t.wait(gap, &|| true), "a cancel ends the wait");
        assert!(cancelled.elapsed() < Duration::from_secs(1));
    }

    /// Live check against NOAA. Run with
    /// `cargo test --release real_nomads -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_nomads_download_decodes() {
        use crate::weather::grib::read_fields;
        use crate::weather::model::Group;
        use crate::weather::planner::{Area, Request, candidate_runs, plan};
        let now = Utc::now();
        let req = Request { source: crate::weather::source::Source::Gfs, area: Area::new(43.0, -12.0, 52.0, 0.0), groups: vec![Group::Wind, Group::Gust, Group::Pressure, Group::Waves], horizon_hours: 6, step_hours: 3 };
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let mut ok = None;
        for run in candidate_runs(now) {
            let p = plan(&req, run, now);
            eprintln!("trying run {run}: {} files, ~{} bytes", p.jobs.len(), p.est_bytes);
            match fetch_plan(&p, &cache, &AtomicBool::new(false), &quiet) {
                Ok(got) if got.missing.is_empty() => {
                    ok = Some((p, got.paths));
                    break;
                }
                Ok(got) => eprintln!("  {} files not published", got.missing.len()),
                Err(e) => eprintln!("  failed: {e}"),
            }
        }
        let (p, paths) = ok.expect("no model run could be downloaded");
        let mut params = std::collections::HashSet::new();
        let mut bytes = 0;
        for path in &paths {
            let data = std::fs::read(path).unwrap();
            bytes += data.len();
            for f in read_fields(&data).unwrap() {
                eprintln!("  {:?} at {} {}x{}", f.param, f.time, f.grid.ncols, f.grid.nrows);
                params.insert(f.param);
            }
        }
        eprintln!("downloaded {bytes} bytes (estimated {})", p.est_bytes);
        for want in [Param::WindU, Param::WindV, Param::Gust, Param::Pressure, Param::WaveHeight] {
            assert!(params.contains(&want), "{want:?} missing from {params:?}");
        }
        let ratio = bytes as f64 / p.est_bytes as f64;
        assert!((0.3..3.0).contains(&ratio), "estimate off by {ratio}");
    }

    /// Live check against DWD's open data, ICON-EU. Run with
    /// `cargo test --release real_icon -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_icon_eu_download_decodes_and_crops() {
        use crate::weather::grib::read_fields;
        use crate::weather::model::Group;
        use crate::weather::planner::{Area, Request, plan};
        use crate::weather::source::Source;
        let now = Utc::now();
        let req = Request { source: Source::IconEu, area: Area::new(43.0, -12.0, 52.0, 0.0), groups: vec![Group::Wind, Group::Gust, Group::Pressure], horizon_hours: 6, step_hours: 3 };
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let mut ok = None;
        for run in Source::IconEu.candidate_runs(now) {
            let p = plan(&req, run, now);
            eprintln!("trying run {run}: {} files, ~{} bytes", p.jobs.len(), p.est_bytes);
            match fetch_plan(&p, &cache, &AtomicBool::new(false), &quiet) {
                Ok(got) if got.missing.is_empty() => {
                    ok = Some((p, got.paths));
                    break;
                }
                Ok(got) => eprintln!("  {} files not published", got.missing.len()),
                Err(e) => eprintln!("  failed: {e}"),
            }
        }
        let (p, paths) = ok.expect("no run could be downloaded");
        let mut params = std::collections::HashSet::new();
        for path in &paths {
            for f in read_fields(&std::fs::read(path).unwrap()).unwrap() {
                eprintln!("  {:?} at {} {}x{} res {}", f.param, f.time, f.grid.ncols, f.grid.nrows, f.grid.dx);
                assert!((f.grid.dx - 0.0625).abs() < 1e-6 && f.grid.ncols < 220, "cropped at its own resolution");
                assert_eq!(f.run, Some(p.run));
                params.insert(f.param);
            }
        }
        for want in [Param::WindU, Param::WindV, Param::Gust, Param::Pressure] {
            assert!(params.contains(&want), "{want:?} missing from {params:?}");
        }
    }

    /// Live check against ECMWF's open data, IFS and AIFS. Run with
    /// `cargo test --release real_ecmwf -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_ecmwf_download_decodes_and_crops() {
        use crate::weather::grib::read_fields;
        use crate::weather::model::Group;
        use crate::weather::planner::{Area, Request, plan};
        use crate::weather::source::Source;
        let now = Utc::now();
        for source in [Source::Ifs, Source::Aifs] {
            let req = Request { source, area: Area::new(43.0, -12.0, 52.0, 0.0), groups: vec![Group::Wind, Group::Gust, Group::Pressure, Group::Waves], horizon_hours: 12, step_hours: 6 };
            let dir = tempfile::tempdir().unwrap();
            let cache = Cache::new(dir.path());
            let mut ok = None;
            for run in source.candidate_runs(now) {
                let p = plan(&req, run, now);
                eprintln!("{source:?}: trying run {run}: {} files, ~{} bytes, unavailable {:?}", p.jobs.len(), p.est_bytes, p.unavailable);
                let seen = Mutex::new(0u64);
                match fetch_plan(&p, &cache, &AtomicBool::new(false), &|pr| *seen.lock().unwrap() = pr.bytes_done) {
                    Ok(got) if got.missing.is_empty() => {
                        ok = Some((p, got.paths, *seen.lock().unwrap()));
                        break;
                    }
                    Ok(got) => eprintln!("  {} files not published", got.missing.len()),
                    Err(e) => eprintln!("  failed: {e}"),
                }
            }
            let (p, paths, downloaded) = ok.expect("no run could be downloaded");
            let mut params = std::collections::HashSet::new();
            let mut cached = 0;
            for path in &paths {
                let data = std::fs::read(path).unwrap();
                cached += data.len();
                for f in read_fields(&data).unwrap() {
                    assert!(f.grid.ncols < 60 && f.grid.nrows < 50, "cropped: {}x{}", f.grid.ncols, f.grid.nrows);
                    assert_eq!(f.run, Some(p.run));
                    params.insert(f.param);
                }
            }
            eprintln!("{source:?}: downloaded {downloaded} bytes (estimated {}), cached {cached}", p.est_bytes);
            let mut want = vec![Param::WindU, Param::WindV, Param::Pressure, Param::WaveHeight, Param::WavePeriod, Param::WaveDir];
            if source == Source::Ifs {
                want.push(Param::Gust);
            }
            for w in want {
                assert!(params.contains(&w), "{w:?} missing from {params:?}");
            }
            let ratio = downloaded as f64 / p.est_bytes as f64;
            assert!((0.7..1.3).contains(&ratio), "estimate off by {ratio}");
        }
    }

    #[test]
    fn connection_refused_is_a_network_error() {
        let port = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let dir = tempfile::tempdir().unwrap();
        let plan = plan_of(vec![job(port, "/x", 0)]);
        let e = fetch_plan(&plan, &Cache::new(dir.path()), &AtomicBool::new(false), &quiet).unwrap_err();
        assert!(matches!(e, DlError::Http(_)), "{e}");
    }
}
