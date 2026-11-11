//! Settings stored as JSON in `$XDG_CONFIG_HOME/isomar/settings.json`.

use crate::comm::source::ConnectionConfig;
use crate::geo::LatLon;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Where the chart is looking, restored on the next start.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct ViewState {
    pub lat: f64,
    pub lon: f64,
    pub zoom: f64,
}

impl Default for ViewState {
    /// The Rade de Brest, a convenient place to test French charts.
    fn default() -> Self {
        Self { lat: 48.35, lon: -4.55, zoom: 9.0 }
    }
}

impl ViewState {
    pub fn center(&self) -> LatLon {
        LatLon::new(self.lat, self.lon)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub view: ViewState,
    /// "auto", "day", "dusk" or "night".
    pub mode: String,
    /// Layers the user has switched off, by id.
    pub hidden_layers: Vec<String>,
    /// Layers that start off and that the user has switched on, by id.
    pub optional_layers: Vec<String>,
    /// Depth drawn as the safety contour, in metres.
    pub safety_depth: f32,
    pub connections: Vec<ConnectionConfig>,
    /// AIS collision alert limits.
    pub ais_cpa_nm: f64,
    pub ais_tcpa_min: f64,
    /// Radius of a new anchor watch, in nautical miles.
    pub anchor_radius_nm: f64,
    /// Folders searched for S-57 chart cells, besides the data folders.
    pub chart_folders: Vec<String>,
    /// Size of the text drawn on the chart, in percent.
    pub text_scale: u32,
    /// How far the automatic router keeps from land and hazards, in nautical miles.
    pub auto_clearance_nm: f64,
    /// How far the waypoints of a saved weather route may stray from the path it was calculated as,
    /// in nautical miles: a larger value gives fewer waypoints (`routing::plan::route_points`).
    pub route_tolerance_nm: f64,
    /// The last choices of the "Best departure" dialog.
    pub weather_window: WindowPrefs,
    /// The forecast model chosen last: the one shown, downloaded and routed on unless another is picked.
    pub weather_source: crate::weather::source::Source,
    /// The models weather routing and best departure were last asked to use; empty for the one shown.
    pub routing_sources: Vec<crate::weather::source::Source>,
    /// "paper" (INT 1) or "ecdis" (S-52 colours) for charts and marks.
    pub chart_style: String,
    /// Draw light sectors at their full nominal range, not a fixed size on screen.
    pub full_sectors: bool,
    /// Own ship's length and beam in metres; the hull is drawn to scale once it is bigger than the symbol.
    pub boat_length_m: f64,
    pub boat_beam_m: f64,
    /// How far ahead the COG vector reaches, in minutes.
    pub vector_minutes: f64,
    /// The corridor drawn along the active leg and the off-course warning, in nautical miles either side.
    pub xte_limit_nm: f64,
    /// Shallow-water alarm depth in metres, and the true wind in knots above which to warn; 0 is off.
    pub shallow_alarm_m: f64,
    pub wind_limit_kn: f64,
    /// Instrument tiles on the dashboard, by id, and whether they also show on the chart.
    pub dashboard: Vec<String>,
    pub dashboard_on_chart: bool,
    /// Bearings are magnetic, not true.
    pub bearing_magnetic: bool,
    /// "kn", "km/h" or "m/s"; "nm", "km" or "mi"; "m", "ft" or "fa".
    pub speed_unit: String,
    pub distance_unit: String,
    pub depth_unit: String,
    /// The under-way layout switches on by itself when a route starts.
    pub underway_auto: bool,
    /// The welcome screen has been seen.
    pub first_run_done: bool,
    /// Whether the performance log keeps 10 s windows while under way.
    pub perf_recording: bool,
    /// The Sailing / Motoring switch ("sailing", "motoring", or "" for not set), kept across restarts.
    pub sail_mode: String,
    /// The instruments' calibration the user saved from the performance log.
    pub perf_calibration: Option<crate::perf::calibrate::Calibration>,
    /// The active polar (`routing::library`); empty until the app first starts with the library.
    pub polar: String,
    /// The performance weather routing starts from, in percent, per polar, once the user took it from
    /// the log measured against that polar.
    pub polar_performance_pct: std::collections::BTreeMap<String, f64>,
    /// The single performance of the versions before the polar library: moved into
    /// `polar_performance_pct` for the polar it was measured against at the first start.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boat_performance_pct: Option<f64>,
    /// Currents: whether routes use them, and the tidal atlases installed.
    pub currents: CurrentPrefs,
}

/// What the user chose about currents.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CurrentPrefs {
    /// Weather routes and best departures sail in the current wherever a source covers the passage.
    pub route_with: bool,
    /// Copernicus currents are fetched for a passage when it is routed.
    pub fetch_copernicus: bool,
    /// The Shom atlas zones installed, by id (`currents::atlas`), loaded at start.
    pub atlases: Vec<String>,
}

impl Default for CurrentPrefs {
    fn default() -> Self {
        CurrentPrefs { route_with: true, fetch_copernicus: true, atlases: Vec::new() }
    }
}

/// What the "Best departure" dialog remembers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowPrefs {
    pub weights: crate::routing::window::Weights,
    pub days: f64,
    pub step_hours: f64,
    /// Ranked over several forecast models by the worst of them, not their average.
    pub cautious: bool,
}

impl Default for WindowPrefs {
    fn default() -> Self {
        WindowPrefs { weights: Default::default(), days: 3.0, step_hours: 3.0, cautious: false }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            view: ViewState::default(),
            mode: "auto".into(),
            hidden_layers: Vec::new(),
            optional_layers: Vec::new(),
            safety_depth: 5.0,
            connections: Vec::new(),
            ais_cpa_nm: 0.5,
            ais_tcpa_min: 20.0,
            anchor_radius_nm: 0.05,
            chart_folders: Vec::new(),
            text_scale: 100,
            auto_clearance_nm: 0.15,
            route_tolerance_nm: crate::routing::plan::ROUTE_TOLERANCE_NM,
            weather_window: WindowPrefs::default(),
            weather_source: Default::default(),
            routing_sources: Vec::new(),
            chart_style: "paper".into(),
            full_sectors: false,
            boat_length_m: 10.0,
            boat_beam_m: 3.4,
            vector_minutes: 6.0,
            xte_limit_nm: 0.1,
            shallow_alarm_m: 0.0,
            wind_limit_kn: 0.0,
            dashboard: Vec::new(),
            dashboard_on_chart: false,
            bearing_magnetic: false,
            speed_unit: "kn".into(),
            distance_unit: "nm".into(),
            depth_unit: "m".into(),
            underway_auto: true,
            first_run_done: false,
            perf_recording: true,
            sail_mode: String::new(),
            perf_calibration: None,
            polar: String::new(),
            polar_performance_pct: Default::default(),
            boat_performance_pct: None,
            currents: CurrentPrefs::default(),
        }
    }
}

fn path() -> PathBuf {
    if let Some(d) = std::env::var_os("ISOMAR_CONFIG_DIR") {
        return PathBuf::from(d).join("settings.json");
    }
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("isomar").join("settings.json")
}

/// Set when the settings file was there but could not be read; see `Settings::take_load_problem`.
static LOAD_PROBLEM: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The file a settings file that could not be read is kept as.
fn bad_path(p: &Path) -> PathBuf {
    p.with_extension("json.bad")
}

impl Settings {
    /// Reads the settings. No file yields the defaults; a file that cannot be read or understood
    /// yields them too, but is kept beside as `settings.json.bad` first, since the next save
    /// replaces it (`take_load_problem` then says so once).
    pub fn load() -> Self {
        let (s, problem) = Self::load_from(&path());
        if problem {
            LOAD_PROBLEM.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        s
    }

    /// Whether a settings file was found unreadable since this was last asked: the shell tells
    /// the user once.
    pub fn take_load_problem() -> bool {
        LOAD_PROBLEM.swap(false, std::sync::atomic::Ordering::Relaxed)
    }

    /// The settings in `p`, and whether a file was there that could not be used.
    fn load_from(p: &Path) -> (Self, bool) {
        let why = match std::fs::read_to_string(p) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(s) => return (s, false),
                Err(e) => e.to_string(),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Self::default(), false),
            Err(e) => e.to_string(),
        };
        let bad = bad_path(p);
        let said = match std::fs::copy(p, &bad) {
            Ok(_) => format!("settings: {} cannot be read ({why}); kept as {} and reset", p.display(), bad.display()),
            Err(e) => format!("settings: {} cannot be read ({why}) and could not be kept ({e}); reset", p.display()),
        };
        // The log is the user's: a test does not write there.
        if !cfg!(test) {
            crate::log::write(&said);
        }
        (Self::default(), true)
    }

    pub fn save(&self) {
        let p = path();
        if let Err(e) = self.save_to(&p) {
            crate::log::write(&format!("settings: cannot save {}: {e}", p.display()));
        }
    }

    /// Writes the settings whole or not at all: to a file beside, then renamed over `p`, so a
    /// power cut mid-write leaves the old settings rather than half a file.
    fn save_to(&self, p: &Path) -> std::io::Result<()> {
        use std::io::Write;
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(self)?;
        let tmp = p.with_extension("json.tmp");
        let written = (|| {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(text.as_bytes())?;
            f.sync_all()?;
            std::fs::rename(&tmp, p)
        })();
        if written.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        written
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.view, ViewState::default());
        assert_eq!(s.mode, "auto");
        assert_eq!(s.safety_depth, 5.0);
        assert!(s.hidden_layers.is_empty());
        assert!(s.optional_layers.is_empty(), "download layers start off");
        assert_eq!(s.text_scale, 100);
        assert_eq!(s.auto_clearance_nm, 0.15);
        assert_eq!(s.route_tolerance_nm, 0.1);
        assert_eq!(s.weather_window, WindowPrefs::default());
        assert!(s.currents.route_with, "currents are on unless switched off");
        let w: WindowPrefs = serde_json::from_str(r#"{"weights": {"upwind": 3.0}}"#).unwrap();
        assert_eq!((w.weights.upwind, w.weights.engine, w.days), (3.0, crate::routing::window::Weights::default().engine, 3.0));
    }

    #[test]
    fn roundtrip() {
        let s = Settings { mode: "night".into(), hidden_layers: vec!["aids".into()], optional_layers: vec!["online-depth".into()], ..Settings::default() };
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.mode, "night");
        assert_eq!(back.hidden_layers, vec!["aids".to_string()]);
        assert_eq!(back.optional_layers, vec!["online-depth".to_string()]);
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let s: Settings = serde_json::from_str(r#"{"mode":"day","future":1}"#).unwrap();
        assert_eq!(s.mode, "day");
    }

    #[test]
    fn a_missing_file_gives_the_defaults_and_keeps_nothing() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("settings.json");
        let (s, problem) = Settings::load_from(&p);
        assert!(!problem);
        assert_eq!(s.mode, "auto");
        assert!(!bad_path(&p).exists());
    }

    #[test]
    fn a_file_that_cannot_be_read_is_kept_before_it_is_reset() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("settings.json");
        // Cut short, as a power cut mid-write would leave it.
        let whole = serde_json::to_string_pretty(&Settings { mode: "night".into(), ..Settings::default() }).unwrap();
        let half = &whole[..whole.len() / 2];
        std::fs::write(&p, half).unwrap();
        let (s, problem) = Settings::load_from(&p);
        assert!(problem);
        assert_eq!(s.mode, "auto", "the defaults");
        assert_eq!(std::fs::read_to_string(bad_path(&p)).unwrap(), half);
        // A field of the wrong type is a problem too, not a silent reset.
        std::fs::write(&p, r#"{"mode": 3}"#).unwrap();
        assert!(Settings::load_from(&p).1);
        assert_eq!(std::fs::read_to_string(bad_path(&p)).unwrap(), r#"{"mode": 3}"#);
    }

    #[test]
    fn saving_replaces_the_file_whole() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("isomar").join("settings.json");
        Settings::default().save_to(&p).unwrap();
        Settings { mode: "night".into(), ..Settings::default() }.save_to(&p).unwrap();
        let (s, problem) = Settings::load_from(&p);
        assert_eq!((s.mode.as_str(), problem), ("night", false));
        let left: Vec<_> = std::fs::read_dir(p.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, vec![std::ffi::OsString::from("settings.json")], "no temporary file is left");
    }
}
