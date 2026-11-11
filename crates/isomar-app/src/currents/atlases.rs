//! The Shom tidal atlases: those installed are loaded at start and phased by the tides of their
//! reference ports; the zones the website offers are listed on the weather page, to install or remove.
//! A zone is never downloaded unasked: a route over one that is missing only says it would help.

use crate::app::AppState;
use crate::core::currents::atlas::{self, Zone};
use crate::core::currents::field::Tidal;
use crate::core::vector::Bounds;
use crate::i18n::{gettext, gettext_f};
use crate::platform::bg;
use chrono::{Duration, Utc};
use std::rc::Rc;
use std::sync::Arc;

/// How far ahead the atlases are phased: the tide service keeps a month, a route looks ten days ahead.
const PHASE_DAYS: i64 = 14;

/// The zone ids installed, from the folders in the install dir.
pub fn installed() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(atlas::install_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| !n.ends_with(".part"))
        .collect();
    v.sort();
    v
}

/// Loads and phases the installed atlases on a worker, and puts them in the current field.
pub async fn reload(app: &Rc<AppState>) {
    let dir = atlas::install_dir();
    let tides = crate::core::tides::cache_dir();
    let phased = bg(move || {
        let atlases = atlas::load_dir(&dir);
        atlas::phase(&atlases, &tides, Utc::now() - Duration::hours(12), PHASE_DAYS * 24)
    })
    .await;
    isomar_core::log!("atlas: {} grids phased", phased.len());
    let ids = installed();
    {
        let mut w = app.weather.borrow_mut();
        w.currents.set_tides(phased.into_iter().map(|p| Arc::new(p) as Arc<dyn Tidal>).collect());
        w.currents.installed = ids.clone();
    }
    if app.settings.borrow().currents.atlases != ids {
        app.settings.borrow_mut().currents.atlases = ids;
        app.apply_settings();
    }
    app.refresh();
}

/// At start: the installed atlases, and the list of zones the website offers.
pub fn start(app: &Rc<AppState>) {
    app.weather.borrow_mut().currents.installed = installed();
    let a = app.clone();
    crate::platform::spawn(async move {
        if !installed().is_empty() {
            reload(&a).await;
        }
        match bg(atlas::index).await {
            Ok(i) => {
                a.weather.borrow_mut().currents.zones = i.zones;
                a.refresh();
            }
            Err(e) => isomar_core::log!("atlas: no list of zones: {e}"),
        }
    });
}

/// Downloads zone `id`, as a job, and loads it.
pub async fn install(app: &Rc<AppState>, id: &str) {
    let Some(zone) = app.weather.borrow().currents.zones.iter().find(|z| z.id == id).cloned() else { return };
    let job = Rc::new(app.jobs.start_cancellable("atlas", &gettext_f("Shom atlas: {zone}", &[("zone", &zone.name)])));
    job.detail(&gettext_f("{mb} MB", &[("mb", &format!("{:.1}", zone.bytes as f64 / 1_048_576.0))]));
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let _stop = {
        let c = cancel.clone();
        crate::jobs::on_drop(move || c.store(true, std::sync::atomic::Ordering::Relaxed))
    };
    let z = zone.clone();
    let got = job.unless_cancelled(bg(move || atlas::install(&z, &atlas::install_dir(), &cancel))).await;
    match got {
        Some(Ok(())) => {
            job.detail(&gettext("Reading the tides of its ports"));
            reload(app).await;
            app.toast(&gettext_f("The Shom atlas for {zone} is installed", &[("zone", &zone.name)]));
        }
        Some(Err(e)) => app.show_error(&gettext("The atlas could not be installed"), &e),
        None => {}
    }
}

/// Removes zone `id` and what it gave the current field.
pub fn remove(app: &Rc<AppState>, id: &str) {
    if id.is_empty() || id.contains(['/', '\\', '.']) {
        return;
    }
    let _ = std::fs::remove_dir_all(atlas::install_dir().join(id));
    let a = app.clone();
    crate::platform::spawn(async move { reload(&a).await });
}

/// The zones that reach into `b` and are not installed: what a route there could use.
pub fn missing_over(app: &AppState, b: &Bounds) -> Vec<Zone> {
    let w = app.weather.borrow();
    w.currents.zones.iter().filter(|z| z.area().intersects(b) && !w.currents.installed.contains(&z.id)).cloned().collect()
}
