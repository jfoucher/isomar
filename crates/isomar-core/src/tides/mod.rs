//! Tide predictions for the nearest station, from the Open Waters tides
//! service (harmonic constants from TICON-4 and national sources).
//!
//! The service does the harmonic analysis; this module builds the request,
//! reads the answer, and interpolates heights between high and low water.

use crate::geo::LatLon;
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use std::f64::consts::PI;
use std::path::{Path, PathBuf};

const API: &str = "https://api.openwaters.io/tides/extremes";

#[derive(Debug, PartialEq)]
pub enum TideError {
    /// The request failed.
    Network(String),
    /// The answer was not what we expect.
    Format(String),
    /// No station near the position.
    NoStation,
}

impl std::fmt::Display for TideError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TideError::Network(s) => write!(f, "network error: {s}"),
            TideError::Format(s) => write!(f, "unexpected answer from the tide service: {s}"),
            TideError::NoStation => write!(f, "there is no tide station near here"),
        }
    }
}

impl std::error::Error for TideError {}

#[derive(Clone, Debug, PartialEq)]
pub struct Station {
    pub id: String,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    /// Distance from the requested position, nautical miles.
    pub distance_nm: f64,
    pub datum: String,
    pub attribution: String,
    pub licence_url: String,
    pub commercial_use: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extreme {
    pub time: DateTime<Utc>,
    pub level_m: f64,
    pub high: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Forecast {
    pub station: Station,
    pub extremes: Vec<Extreme>,
}

/// A UTC date and time, for callers that do not use chrono themselves.
pub type DateTimeUtc = DateTime<Utc>;

/// The current time.
pub fn now_utc() -> DateTime<Utc> {
    Utc::now()
}

/// The request for a position and time span.
pub fn url(lat: f64, lon: f64, start: DateTime<Utc>, end: DateTime<Utc>) -> String {
    let t = |d: DateTime<Utc>| d.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    format!("{API}?latitude={lat:.4}&longitude={lon:.4}&start={}&end={}", t(start), t(end))
}

fn s(v: &Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

pub fn parse(text: &str) -> Result<Forecast, TideError> {
    let v: Value = serde_json::from_str(text).map_err(|e| TideError::Format(e.to_string()))?;
    let st = v.get("station").ok_or(TideError::NoStation)?;
    if st.is_null() {
        return Err(TideError::NoStation);
    }
    let station = Station {
        id: s(st, "id"),
        name: s(st, "name"),
        lat: st.get("latitude").and_then(|x| x.as_f64()).ok_or_else(|| TideError::Format("station without latitude".into()))?,
        lon: st.get("longitude").and_then(|x| x.as_f64()).ok_or_else(|| TideError::Format("station without longitude".into()))?,
        // The service reports kilometres.
        distance_nm: v.get("distance").and_then(|x| x.as_f64()).unwrap_or(0.0) / 1.852,
        datum: st.get("chart_datum").and_then(|x| x.as_str()).or_else(|| v.get("datum").and_then(|x| x.as_str())).unwrap_or("").to_string(),
        attribution: s(st, "attribution"),
        licence_url: st.get("license").map(|l| s(l, "url")).unwrap_or_default(),
        commercial_use: st.get("license").and_then(|l| l.get("commercial_use")).and_then(|x| x.as_bool()).unwrap_or(false),
    };
    let mut extremes = Vec::new();
    for e in v.get("extremes").and_then(|x| x.as_array()).ok_or_else(|| TideError::Format("no extremes".into()))? {
        let time = e.get("time").and_then(|x| x.as_str()).and_then(|t| DateTime::parse_from_rfc3339(t).ok()).map(|t| t.with_timezone(&Utc));
        let level = e.get("level").and_then(|x| x.as_f64());
        if let (Some(time), Some(level_m)) = (time, level) {
            extremes.push(Extreme { time, level_m, high: e.get("high").and_then(|x| x.as_bool()).unwrap_or(false) });
        }
    }
    extremes.sort_by_key(|e| e.time);
    Ok(Forecast { station, extremes })
}

/// Downloads the raw answer for a time span. Blocking.
fn fetch_body(lat: f64, lon: f64, start: DateTime<Utc>, end: DateTime<Utc>) -> Result<String, TideError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(20)))
        .user_agent("isomar/0.1 (open-source chart plotter)")
        .build()
        .into();
    let mut resp = agent.get(url(lat, lon, start, end)).call().map_err(|e| match e {
        ureq::Error::StatusCode(404) => TideError::NoStation,
        other => TideError::Network(other.to_string()),
    })?;
    resp.body_mut().read_to_string().map_err(|e| TideError::Network(e.to_string()))
}

/// Fetches and parses the forecast, without the cache `lookup` keeps: for the live check.
#[cfg(test)]
pub fn fetch(lat: f64, lon: f64, from: DateTime<Utc>, hours: i64) -> Result<Forecast, TideError> {
    // Start a day early so the extreme before `from` is known too.
    parse(&fetch_body(lat, lon, from - Duration::hours(12), from + Duration::hours(hours))?)
}

// ---- keeping forecasts for use without a network ---------------------------

/// How far ahead one download predicts. The answer is tiny (about 20 KB a month).
const WINDOW_DAYS: i64 = 30;
/// A stored station this close is used without asking the network.
const NEAR_NM: f64 = 5.0;
/// With no network, a stored station this close is better than nothing.
const OFFLINE_NM: f64 = 100.0;
/// Stored stations kept; the oldest go first.
const KEEP_FILES: usize = 200;

/// Where predictions are kept between runs.
pub fn cache_dir() -> PathBuf {
    crate::datadir::cache_dir().join("tides")
}

/// Where a forecast came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Network,
    /// Kept from an earlier lookup near here.
    Stored,
    /// The network failed, so this is the nearest station kept earlier.
    StoredFar,
}

fn file_for(dir: &Path, id: &str) -> PathBuf {
    let safe: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    dir.join(format!("{safe}.json"))
}

fn store(dir: &Path, id: &str, body: &str) {
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let path = file_for(dir, id);
    let tmp = path.with_extension("part");
    if std::fs::write(&tmp, body).is_ok() {
        let _ = std::fs::rename(tmp, path);
    }
    // Forget the oldest beyond the limit.
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    if files.len() > KEEP_FILES {
        files.sort();
        for (_, p) in files.iter().take(files.len() - KEEP_FILES) {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// The nearest kept station within `max_nm` whose predictions run from before
/// `from` until `until`. Its distance is measured from the given position.
fn stored(dir: &Path, lat: f64, lon: f64, from: DateTime<Utc>, until: DateTime<Utc>, max_nm: f64) -> Option<Forecast> {
    let here = LatLon::new(lat, lon);
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| parse(&std::fs::read_to_string(e.path()).ok()?).ok())
        .filter(|f| f.extremes.first().is_some_and(|e| e.time <= from) && f.extremes.last().is_some_and(|e| e.time >= until))
        .map(|mut f| {
            f.station.distance_nm = here.distance_nm(&LatLon::new(f.station.lat, f.station.lon));
            f
        })
        .filter(|f| f.station.distance_nm <= max_nm)
        .min_by(|a, b| a.station.distance_nm.total_cmp(&b.station.distance_nm))
}

/// Predictions for the next `hours` at the nearest station: from what was
/// kept earlier if that covers the span, else from the network (which keeps
/// a month, so later lookups here work offline), else from a farther station
/// that was kept. Blocking; call from a worker thread.
pub fn lookup(dir: &Path, lat: f64, lon: f64, from: DateTime<Utc>, hours: i64) -> Result<(Forecast, Origin), TideError> {
    // One extreme before `from` is needed to place the tide now.
    let start = from - Duration::hours(12);
    let until = from + Duration::hours(hours);
    if let Some(f) = stored(dir, lat, lon, start, until, NEAR_NM) {
        return Ok((f, Origin::Stored));
    }
    match fetch_body(lat, lon, start, from + Duration::days(WINDOW_DAYS)).and_then(|body| parse(&body).map(|f| (body, f))) {
        Ok((body, f)) => {
            store(dir, &f.station.id, &body);
            Ok((f, Origin::Network))
        }
        Err(TideError::Network(m)) => {
            stored(dir, lat, lon, start, from + Duration::hours(6), OFFLINE_NM).map(|f| (f, Origin::StoredFar)).ok_or(TideError::Network(m))
        }
        Err(e) => Err(e),
    }
}

impl Forecast {
    /// The extremes at or after `t`.
    pub fn upcoming(&self, t: DateTime<Utc>) -> impl Iterator<Item = &Extreme> {
        self.extremes.iter().filter(move |e| e.time >= t)
    }

    /// Height at `t` by the half-cosine rule between the surrounding extremes,
    /// which is accurate to a few centimetres for semi-diurnal tides.
    pub fn height_at(&self, t: DateTime<Utc>) -> Option<f64> {
        let i = self.extremes.partition_point(|e| e.time <= t);
        let (a, b) = (self.extremes.get(i.checked_sub(1)?)?, self.extremes.get(i)?);
        let span = (b.time - a.time).num_seconds() as f64;
        if span <= 0.0 {
            return None;
        }
        let f = (t - a.time).num_seconds() as f64 / span;
        Some(a.level_m + (b.level_m - a.level_m) * (1.0 - (PI * f).cos()) / 2.0)
    }

    /// Whether the tide is rising at `t`.
    pub fn rising_at(&self, t: DateTime<Utc>) -> Option<bool> {
        let i = self.extremes.partition_point(|e| e.time <= t);
        self.extremes.get(i).map(|next| next.high)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 26, h, m, 0).unwrap()
    }

    const SAMPLE: &str = r#"{"datum":"LAT","units":"meters","distance":0.81,
      "station":{"id":"ticon/brest","name":"Brest","latitude":48.383,"longitude":-4.5,"chart_datum":"LAT",
        "attribution":"Source: TICON-4, CC BY 4.0","license":{"type":"cc-by-4.0","commercial_use":true,"url":"https://creativecommons.org/licenses/by/4.0/"}},
      "extremes":[
        {"time":"2026-10-26T09:53:03.293Z","level":0.85,"high":false,"low":true,"label":"Low"},
        {"time":"2026-10-26T03:37:51.251Z","level":6.77,"high":true,"low":false,"label":"High"},
        {"time":"2026-10-26T15:54:50.262Z","level":6.88,"high":true,"low":false,"label":"High"}]}"#;

    #[test]
    fn the_service_answer_is_read_and_sorted() {
        let f = parse(SAMPLE).unwrap();
        assert_eq!(f.station.name, "Brest");
        assert_eq!(f.station.datum, "LAT");
        assert!((f.station.distance_nm - 0.81 / 1.852).abs() < 1e-9, "kilometres become miles");
        assert!(f.station.commercial_use && f.station.attribution.contains("TICON-4"));
        assert_eq!(f.extremes.len(), 3);
        assert!(f.extremes.windows(2).all(|w| w[0].time < w[1].time));
        assert!(f.extremes[0].high && !f.extremes[1].high);
    }

    #[test]
    fn heights_follow_a_half_cosine_between_extremes() {
        let f = parse(SAMPLE).unwrap();
        // Right at an extreme.
        assert!((f.height_at(f.extremes[0].time).unwrap() - 6.77).abs() < 1e-9);
        // Half way between high (6.77) and low (0.85) is the mean.
        let mid = f.extremes[0].time + (f.extremes[1].time - f.extremes[0].time) / 2;
        assert!((f.height_at(mid).unwrap() - (6.77 + 0.85) / 2.0).abs() < 0.01);
        // Slowest change near the extremes, fastest at mid-tide.
        let near = f.height_at(f.extremes[0].time + Duration::minutes(30)).unwrap();
        assert!(6.77 - near < 0.25, "{near}");
        // Outside the known span there is no answer.
        assert!(f.height_at(t(1, 0)).is_none() && f.height_at(t(20, 0)).is_none());
    }

    #[test]
    fn rising_or_falling_and_what_comes_next() {
        let f = parse(SAMPLE).unwrap();
        assert_eq!(f.rising_at(t(6, 0)), Some(false), "heading for low water");
        assert_eq!(f.rising_at(t(12, 0)), Some(true));
        assert_eq!(f.rising_at(t(20, 0)), None);
        let next: Vec<_> = f.upcoming(t(8, 0)).map(|e| e.high).collect();
        assert_eq!(next, vec![false, true]);
    }

    #[test]
    fn odd_answers_are_errors() {
        assert!(matches!(parse("nonsense"), Err(TideError::Format(_))));
        assert_eq!(parse(r#"{"station":null,"extremes":[]}"#).unwrap_err(), TideError::NoStation);
        assert_eq!(parse("{}").unwrap_err(), TideError::NoStation);
        assert!(matches!(parse(r#"{"station":{"latitude":1,"longitude":2}}"#), Err(TideError::Format(_))));
        assert!(matches!(parse(r#"{"station":{"latitude":1}, "extremes":[]}"#), Err(TideError::Format(_))));
        // Entries without a usable time are dropped rather than failing the lot.
        let f = parse(r#"{"station":{"latitude":1,"longitude":2},"extremes":[{"time":"bad","level":1},{"time":"2026-01-01T00:00:00Z","level":2,"high":true}]}"#).unwrap();
        assert_eq!(f.extremes.len(), 1);
    }

    #[test]
    fn the_request_names_the_position_and_span() {
        let u = url(48.38, -4.49, t(0, 0), t(12, 0));
        assert!(u.starts_with("https://api.openwaters.io/tides/extremes?latitude=48.3800&longitude=-4.4900"));
        assert!(u.contains("start=2026-10-26T00:00:00Z") && u.contains("end=2026-10-26T12:00:00Z"));
    }

    /// Live check. Run with `cargo test --release real_tides -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_tides_for_brest() {
        let f = fetch(48.38, -4.49, Utc::now(), 36).unwrap();
        eprintln!("{} at {:.1} nm, {} extremes, next {:?}", f.station.name, f.station.distance_nm, f.extremes.len(), f.upcoming(Utc::now()).next());
        assert_eq!(f.station.name, "Brest");
        assert!(f.extremes.len() >= 6);
        // Brest has a big tide: more than 4 m between high and low.
        let (hi, lo) = f.extremes.iter().fold((f64::MIN, f64::MAX), |(h, l), e| (h.max(e.level_m), l.min(e.level_m)));
        assert!(hi - lo > 3.0, "{hi} {lo}");
        assert!(f.height_at(Utc::now()).is_some());
    }

    /// An answer like the service's, for a station at `lat`, `lon` covering 25 October to 25 November.
    fn answer(id: &str, lat: f64, lon: f64) -> String {
        format!(
            r#"{{"datum":"LAT","distance":0.5,"station":{{"id":"{id}","name":"{id}","latitude":{lat},"longitude":{lon}}},
            "extremes":[{{"time":"2026-10-25T00:00:00Z","level":1.0,"high":false}},
                        {{"time":"2026-11-25T00:00:00Z","level":6.0,"high":true}}]}}"#
        )
    }

    #[test]
    fn kept_stations_are_found_by_distance_and_span() {
        let d = tempfile::tempdir().unwrap();
        store(d.path(), "ticon/brest", &answer("ticon/brest", 48.383, -4.5));
        store(d.path(), "ticon/roscoff", &answer("ticon/roscoff", 48.72, -3.97));
        assert!(!d.path().join("ticon").exists(), "ids with slashes stay in one folder");
        let (from, until) = (t(0, 0), t(0, 0) + Duration::hours(40));
        // Near Brest: Brest, with the distance measured from here, not from the old request.
        let f = stored(d.path(), 48.38, -4.45, from, until, 5.0).unwrap();
        assert_eq!(f.station.name, "ticon/brest");
        assert!((f.station.distance_nm - 2.0).abs() < 0.5, "{}", f.station.distance_nm);
        // Between the two, the nearer wins.
        assert_eq!(stored(d.path(), 48.65, -4.1, from, until, 50.0).unwrap().station.name, "ticon/roscoff");
        // Too far for a close match, fine for a loose one.
        assert!(stored(d.path(), 48.0, -4.5, from, until, 5.0).is_none());
        assert!(stored(d.path(), 48.0, -4.5, from, until, 100.0).is_some());
        // A span the predictions don't reach is no match.
        assert!(stored(d.path(), 48.38, -4.5, from, t(0, 0) + Duration::days(40), 5.0).is_none());
        assert!(stored(d.path(), 48.38, -4.5, t(0, 0) - Duration::days(5), until, 5.0).is_none());
        assert!(stored(&d.path().join("nothing"), 48.38, -4.5, from, until, 5.0).is_none());
    }

    #[test]
    fn lookup_uses_the_kept_station_without_the_network() {
        let d = tempfile::tempdir().unwrap();
        store(d.path(), "ticon/brest", &answer("ticon/brest", 48.383, -4.5));
        let (f, origin) = lookup(d.path(), 48.38, -4.49, t(6, 0), 40).unwrap();
        assert_eq!((f.station.name.as_str(), origin), ("ticon/brest", Origin::Stored));
    }

    #[test]
    fn only_the_newest_stations_are_kept() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..KEEP_FILES + 5 {
            store(d.path(), &format!("s{i}"), &answer(&format!("s{i}"), 1.0, 2.0));
        }
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), KEEP_FILES);
    }

    #[test]
    fn open_ocean_gives_no_station_error_shape() {
        // The service answers with an error status when nothing is near; fetch maps 404 to NoStation.
        assert_eq!(TideError::NoStation.to_string(), "there is no tide station near here");
    }
}
