//! The polar library in the app (`routing::library`): which polar is active, switching, saving and
//! removing, and the one-time move of the single polar of earlier versions into the library.
//!
//! The active polar is what weather routing sails with, what the motoring hint measures against, and
//! the only one the performance log learns into: every sample is logged against it, and the analysis
//! reads only its samples, so learning on one boat never changes another's polar.

use crate::app::AppState;
use crate::core::config::Settings;
use crate::core::nav::store::NavStore;
use crate::core::routing::library::{DEFAULT_ID, Entry, Library};
use crate::core::routing::polar::Polar;
use crate::i18n::gettext;
use serde::Serialize;

pub fn library() -> Library {
    Library::user()
}

/// The id of the active polar, as the settings name it (the default when it is gone).
pub fn active_id_in(settings: &Settings) -> String {
    library().resolve(&settings.polar)
}

pub fn active_id(app: &AppState) -> String {
    active_id_in(&app.settings.borrow())
}

/// The active polar's speeds.
pub fn active(app: &AppState) -> Polar {
    load_active(&app.settings.borrow())
}

pub fn load_active(settings: &Settings) -> Polar {
    let lib = library();
    lib.load(&lib.resolve(&settings.polar)).unwrap_or_else(|| crate::core::routing::library::original(DEFAULT_ID).expect("built in"))
}

/// The active polar's name.
pub fn active_name(app: &AppState) -> String {
    library().name(&active_id(app)).unwrap_or_default()
}

/// At the first start with the library: the polar of before becomes "My boat" and stays active, the
/// samples logged until now are given to the active polar, and so is the performance taken from them.
/// Returns whether the settings changed. Called by `host::install`, never by the tests (the files are
/// the user's).
pub fn migrate(settings: &mut Settings, store: &NavStore) -> bool {
    migrate_in(&library(), &crate::core::datadir::legacy_polar_path(), settings, store)
}

fn migrate_in(lib: &Library, legacy: &std::path::Path, settings: &mut Settings, store: &NavStore) -> bool {
    if !settings.polar.is_empty() {
        return false;
    }
    let id = lib.adopt_legacy(legacy, &gettext("My boat")).unwrap_or_else(|| DEFAULT_ID.to_string());
    if let Ok(n) = store.move_perf_samples("", &id)
        && n > 0
    {
        isomar_core::log!("polars: {n} logged samples given to the polar {id}");
    }
    if let Some(pct) = settings.boat_performance_pct.take() {
        settings.polar_performance_pct.insert(id.clone(), pct);
    }
    settings.polar = id;
    true
}

/// The performance weather routing starts from, for the active polar, once the user took one from the log.
pub fn performance_pct(app: &AppState) -> Option<f64> {
    let s = app.settings.borrow();
    s.polar_performance_pct.get(&library().resolve(&s.polar)).copied()
}

pub fn set_performance_pct(app: &AppState, pct: f64) {
    let id = active_id(app);
    app.settings.borrow_mut().polar_performance_pct.insert(id, pct.clamp(30.0, 150.0));
    app.apply_settings();
}

/// The active polar, or its speeds, changed: the log measures against it and its analysis is redone.
fn changed(app: &AppState) {
    {
        let mut s = app.session.borrow_mut();
        s.perf.polar = active(app);
        s.perf.polar_id = active_id(app);
    }
    let mut p = app.perf.borrow_mut();
    p.analysed_count = usize::MAX;
}

/// Makes `id` the active polar.
pub fn activate(app: &AppState, id: &str) -> Result<(), String> {
    if !library().exists(id) {
        return Err(gettext("There is no such polar"));
    }
    if active_id(app) == id {
        return Ok(());
    }
    app.settings.borrow_mut().polar = id.to_string();
    // What was learnt and analysed belongs to the polar before.
    app.perf.borrow_mut().report = None;
    changed(app);
    app.apply_settings();
    Ok(())
}

/// Saves speeds over polar `id` (a built-in one becomes the user's version of it).
pub fn save(app: &AppState, id: &str, p: &Polar) -> Result<(), String> {
    check(p)?;
    library().save(id, p).map_err(|e| e.to_string())?;
    if active_id(app) == id {
        changed(app);
    }
    app.refresh();
    Ok(())
}

/// Saves speeds as a new polar of the user's; returns its id.
pub fn save_as(app: &AppState, name: &str, p: &Polar) -> Result<String, String> {
    check(p)?;
    let id = library().save_as(name, p).map_err(|e| e.to_string())?;
    app.refresh();
    Ok(id)
}

fn check(p: &Polar) -> Result<(), String> {
    if p.twa.is_empty() || p.tws.is_empty() || p.speeds.len() != p.twa.len() || p.speeds.iter().any(|r| r.len() != p.tws.len()) {
        return Err(gettext("The polar table is incomplete"));
    }
    Ok(())
}

/// A polar of the user's is deleted, with the samples logged against it; a built-in one goes back to
/// the published speeds and keeps its samples. Deleting the active polar makes the default active.
pub fn remove(app: &AppState, id: &str) -> Result<(), String> {
    let lib = library();
    let builtin = crate::core::routing::library::original(id).is_some();
    lib.remove(id).map_err(|e| e.to_string())?;
    if !builtin {
        let _ = app.session.borrow().store.clear_perf_of(id);
        app.settings.borrow_mut().polar_performance_pct.remove(id);
        if app.settings.borrow().polar == id {
            app.settings.borrow_mut().polar = DEFAULT_ID.to_string();
            app.perf.borrow_mut().report = None;
        }
        app.apply_settings();
    }
    changed(app);
    app.refresh();
    Ok(())
}

/// The learnt speeds were put in, or taken out again: the polar the log learns into.
pub fn active_changed(app: &AppState) {
    changed(app);
}

/// The library as a shell lists it.
#[derive(Clone, Debug, Serialize)]
pub struct PolarList {
    pub active: String,
    pub polars: Vec<PolarRow>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PolarRow {
    #[serde(flatten)]
    pub entry: Entry,
    pub active: bool,
    /// How many samples the log holds for it.
    pub samples: usize,
}

pub fn list(app: &AppState) -> PolarList {
    let active = active_id(app);
    let store = &app.session.borrow().store;
    let polars = library()
        .list()
        .into_iter()
        .map(|entry| {
            let samples = store.perf_summary(Some(&entry.id)).map(|s| s.count).unwrap_or(0);
            PolarRow { active: entry.id == active, samples, entry }
        })
        .collect();
    PolarList { active, polars }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::perf::Sample;
    use crate::core::session::Session;
    use std::rc::Rc;

    fn app() -> Rc<AppState> {
        let canvas = crate::engine::Engine::new(crate::core::config::ViewState { lat: 48.0, lon: -4.0, zoom: 10.0 }, crate::palette::Mode::Day);
        AppState::new(canvas, Session::new(NavStore::open_memory().unwrap()).unwrap(), Settings::default())
    }

    #[test]
    fn the_first_start_takes_the_old_polar_its_samples_and_its_performance() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path().join("polars"));
        let legacy = dir.path().join("polar.csv");
        std::fs::write(&legacy, Polar::generic_cruiser().to_text()).unwrap();
        let store = NavStore::open_memory().unwrap();
        for _ in 0..3 {
            store.insert_perf_sample(&Sample::default()).unwrap();
        }
        let mut settings = Settings { boat_performance_pct: Some(92.0), ..Settings::default() };
        assert!(migrate_in(&lib, &legacy, &mut settings, &store));
        let id = settings.polar.clone();
        assert!(id.starts_with("custom-"), "{id}");
        assert_eq!(lib.load(&id).unwrap(), Polar::generic_cruiser());
        assert_eq!(store.perf_summary(Some(&id)).unwrap().count, 3);
        assert_eq!((settings.boat_performance_pct, settings.polar_performance_pct.get(&id)), (None, Some(&92.0)));
        // Once only.
        assert!(!migrate_in(&lib, &legacy, &mut settings, &store));

        // Without an old polar the default one is active and gets the samples.
        let mut fresh = Settings::default();
        let store = NavStore::open_memory().unwrap();
        store.insert_perf_sample(&Sample::default()).unwrap();
        assert!(migrate_in(&lib, &dir.path().join("none.csv"), &mut fresh, &store));
        assert_eq!(fresh.polar, DEFAULT_ID);
        assert_eq!(store.perf_summary(Some(DEFAULT_ID)).unwrap().count, 1);
    }

    #[test]
    fn the_active_polar_is_what_the_log_measures_against_and_has_its_own_performance() {
        let app = app();
        assert_eq!(active_id(&app), DEFAULT_ID);
        set_performance_pct(&app, 90.0);
        assert_eq!(performance_pct(&app), Some(90.0));
        activate(&app, "imoca-60").unwrap();
        assert_eq!(active_id(&app), "imoca-60");
        assert_eq!(app.session.borrow().perf.polar_id, "imoca-60");
        assert_eq!(app.session.borrow().perf.polar, library().load("imoca-60").unwrap());
        assert_eq!(performance_pct(&app), None, "the performance measured against another polar does not carry over");
        assert!(activate(&app, "custom-404").is_err());
        assert_eq!(active_id(&app), "imoca-60");
        let l = list(&app);
        assert_eq!(l.active, "imoca-60");
        assert!(l.polars.iter().any(|r| r.active && r.entry.id == "imoca-60"));
    }
}
