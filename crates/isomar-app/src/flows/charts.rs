//! Chart sources: what is known to exist for a region, how good it is, installing it, and the
//! folders of charts the user added. The sources page and the "Charts for this area" dialog show it.

use crate::app::AppState;
use crate::chips::{Chip, Icon};
use crate::core::datadir;
use crate::core::geo::LatLon;
use crate::core::install::{self, InstallError};
use crate::core::registry::{self, Entry, Kind, Rating, Status};
use crate::i18n::{gettext, gettext_f};
use crate::platform::{bg, spawn};
use serde::Serialize;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The rating's letter, shown as a badge.
pub fn rating_letter(r: Rating) -> &'static str {
    match r {
        Rating::A => "A",
        Rating::B => "B",
        Rating::C => "C",
    }
}

/// What the rating means, beside its letter.
pub fn rating_text(r: Rating) -> String {
    match r {
        Rating::A => gettext("Official"),
        Rating::B => gettext("Open data, planning"),
        Rating::C => gettext("Basic"),
    }
}

pub fn size_text(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= 1024.0 * MB { format!("{:.1} GB", b / (1024.0 * MB)) } else if b >= MB { format!("{:.0} MB", b / MB) } else { format!("{:.0} KB", b / 1024.0) }
}

/// What the row under an entry's title says besides its rating: its size, its licence, and one
/// warning for what matters (not up to date, not for navigation).
pub fn chips(e: &Entry) -> Vec<Chip> {
    let mut out = Vec::new();
    if let Some(s) = e.size_bytes {
        let size = size_text(s);
        out.push(Chip::new(Icon::Size, size.clone(), gettext_f("Download size {size}", &[("size", &size)])));
    }
    if !e.licence.is_empty() {
        out.push(Chip::new(Icon::Licence, e.licence.clone(), gettext_f("Licence: {licence}", &[("licence", &e.licence)])));
    }
    let mut warnings = Vec::new();
    if e.status == Status::Stale {
        warnings.push(gettext("not up to date"));
    }
    if !e.nav_grade && e.kind != Kind::Paid {
        warnings.push(gettext("not for navigation"));
    }
    if !warnings.is_empty() {
        let mut w = warnings.join(", ");
        if let Some(first) = w.get(..1) {
            w = first.to_uppercase() + &w[1..];
        }
        out.push(Chip::with(Icon::Warning, w));
    }
    out
}

pub fn explain(e: &InstallError) -> String {
    match e {
        InstallError::Cancelled => gettext("Cancelled."),
        other => gettext_f("The installation failed: {error}", &[("error", &other.to_string())]),
    }
}

/// A source for the page, with what its button does.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SourceRow {
    pub id: String,
    pub name: String,
    /// The rating's letter, for a badge, and what it means: "A", "Official".
    pub rating: String,
    pub rating_text: String,
    pub chips: Vec<Chip>,
    /// Lines under the chips: the entry's note, where it is installed.
    pub note: String,
    /// `install`, `installing`, `installed`, `update`, `on`, `turn_on` or `website`.
    pub action: String,
    pub url: String,
    /// It is installed and can be removed (`charts_uninstall`).
    pub installed: bool,
}

/// What is known of an installed source this session.
#[derive(Clone, Copy, Default)]
struct Local {
    /// The server offers something other than the copy here.
    newer: bool,
    /// Bytes on disk, once counted.
    size: Option<u64>,
}

thread_local! {
    /// Sources being downloaded, so a second click does not start a second download.
    static INSTALLING: RefCell<HashSet<String>> = RefCell::default();
    /// Installed sources checked against their server, once a session (or after an install).
    static LOCAL: RefCell<HashMap<String, Local>> = RefCell::default();
}

/// Where an entry is (or would be) installed.
pub fn install_folder(e: &Entry) -> Option<PathBuf> {
    e.install.as_ref().map(|i| datadir::user_data_dir().join(&i.dir))
}

fn is_installed(e: &Entry) -> bool {
    e.installable() && install_folder(e).is_some_and(|f| f.exists())
}

fn service_layers() -> [&'static str; 2] {
    [crate::tiles::LAND_LAYER, crate::tiles::MARKS_LAYER]
}

pub fn service_is_on(app: &AppState) -> bool {
    service_layers().iter().all(|id| app.canvas.layer_visible(id))
}

pub fn rows_for(app: &AppState, entries: &[Entry]) -> Vec<SourceRow> {
    entries
        .iter()
        .map(|e| {
            let installed = is_installed(e);
            let local = LOCAL.with(|l| l.borrow().get(&e.id).copied());
            let mut note = e.note.clone();
            let action = if INSTALLING.with(|i| i.borrow().contains(&e.id)) {
                "installing"
            } else if installed {
                if let Some(folder) = install_folder(e) {
                    let at = folder.display().to_string();
                    let line = match local.and_then(|l| l.size) {
                        Some(n) => gettext_f("Installed in {folder} ({size})", &[("folder", &at), ("size", &size_text(n))]),
                        None => gettext_f("Installed in {folder}", &[("folder", &at)]),
                    };
                    if !note.is_empty() {
                        note.push('\n');
                    }
                    note.push_str(&line);
                }
                if local.is_some_and(|l| l.newer) { "update" } else { "installed" }
            } else if e.installable() {
                "install"
            } else if e.kind == Kind::Service && e.status == Status::Live {
                if service_is_on(app) { "on" } else { "turn_on" }
            } else {
                "website"
            };
            SourceRow { id: e.id.clone(), name: e.name.clone(), rating: rating_letter(e.rating).into(), rating_text: rating_text(e.rating), chips: chips(e), note, action: action.into(), url: e.url.clone(), installed: installed && action != "installing" }
        })
        .collect()
}

/// Asks the servers of the installed `entries` whether they offer something newer, and counts
/// what each takes on disk; once a session per source. The page refreshes with the answers.
pub fn check_installed(app: &Rc<AppState>, entries: &[Entry]) {
    for e in entries.iter().filter(|e| is_installed(e)) {
        let fresh = LOCAL.with(|l| l.borrow_mut().insert(e.id.clone(), Local::default()).is_none());
        if !fresh || INSTALLING.with(|i| i.borrow().contains(&e.id)) {
            continue;
        }
        let (Some(folder), id, url, app) = (install_folder(e), e.id.clone(), e.url.clone(), app.clone()) else { continue };
        spawn(async move {
            let local = bg(move || {
                // Offline, or the server says nothing useful: keep what is here.
                let newer = install::check_remote(&url).is_ok_and(|r| install::is_newer(&folder, &url, &r));
                Local { newer, size: Some(install::folder_size(&folder)) }
            })
            .await;
            if LOCAL.with(|l| l.borrow().contains_key(&id)) {
                LOCAL.with(|l| l.borrow_mut().insert(id, local));
                app.refresh();
            }
        });
    }
}

/// What is known to exist for a position, best first.
pub fn entries_here(pos: LatLon) -> Vec<Entry> {
    registry::builtin().for_position(pos.lat, pos.lon).into_iter().cloned().collect()
}

pub fn entry(id: &str) -> Option<Entry> {
    registry::builtin().entries.into_iter().find(|e| e.id == id)
}

/// Turns the downloaded coastline and seamarks on: the service is read by the app as you zoom.
pub fn turn_service_on(app: &AppState) {
    for id in service_layers() {
        app.canvas.set_layer_visible(id, true);
    }
}

/// Downloads and installs a source as a job, then loads it. Returns whether it worked.
pub async fn install_entry(app: &Rc<AppState>, e: Entry) -> bool {
    if !INSTALLING.with(|i| i.borrow_mut().insert(e.id.clone())) {
        return false;
    }
    app.refresh();
    let ok = run_install(app, &e).await;
    INSTALLING.with(|i| i.borrow_mut().remove(&e.id));
    // Checked again (and counted) the next time the page shows it.
    LOCAL.with(|l| l.borrow_mut().remove(&e.id));
    app.refresh();
    ok
}

async fn run_install(app: &Rc<AppState>, e: &Entry) -> bool {
    let e = e.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let job = {
        let cancel = cancel.clone();
        app.jobs.start(&format!("chart:{}", e.id), &gettext_f("Installing {name}", &[("name", &e.name)]), Some(Rc::new(move || cancel.store(true, Ordering::Relaxed))))
    };
    // Dropped half-way, the install stops too (it cleans up behind itself).
    let _stop = {
        let cancel = cancel.clone();
        crate::jobs::on_drop(move || cancel.store(true, Ordering::Relaxed))
    };
    let done = Arc::new(AtomicU64::new(0));
    let total = Arc::new(AtomicU64::new(0));
    let (entry, c2, dir) = (e.clone(), cancel.clone(), datadir::user_data_dir());
    let (d2, t2) = (done.clone(), total.clone());
    let work = bg(move || {
        std::fs::create_dir_all(&dir).map_err(|e| InstallError::Io(e.to_string()))?;
        install::install(&entry, &dir, &c2, &|d, t| {
            d2.store(d, Ordering::Relaxed);
            t2.store(t.unwrap_or(0), Ordering::Relaxed);
        })
    });
    // Progress is read off the counters while the download runs on its worker.
    let watching = Arc::new(AtomicBool::new(true));
    let job = Rc::new(job);
    {
        let (done, total, watching, job) = (done.clone(), total.clone(), watching.clone(), job.clone());
        spawn(async move {
            while watching.load(Ordering::Relaxed) {
                let (d, t) = (done.load(Ordering::Relaxed), total.load(Ordering::Relaxed));
                if d > 0 {
                    let of = if t > 0 { format!(" / {}", size_text(t)) } else { String::new() };
                    job.progress(if t > 0 { d as f64 / t as f64 } else { 0.0 }, &format!("{}{of}", size_text(d)));
                }
                sleep(std::time::Duration::from_millis(400)).await;
            }
        });
    }
    // A cancel returns at once; the worker stops at its next check and removes what it left.
    let result = job.unless_cancelled(work).await;
    watching.store(false, Ordering::Relaxed);
    drop(job);
    let Some(result) = result else {
        app.toast(&gettext("Install cancelled"));
        return false;
    };
    match result {
        Ok(folder) => {
            load_installed(app, &e, folder);
            true
        }
        Err(err) => {
            app.toast(&explain(&err));
            false
        }
    }
}

/// Waits `d` on the main loop.
async fn sleep(d: std::time::Duration) {
    let (tx, rx) = async_channel::bounded::<()>(1);
    crate::platform::timeout_once(d, move || {
        let _ = tx.try_send(());
    });
    let _ = rx.recv().await;
}

/// Makes freshly installed charts appear without a restart where we can. What an earlier copy
/// of the folder held is dropped first: an update replaces it.
pub fn load_installed(app: &Rc<AppState>, e: &Entry, folder: std::path::PathBuf) {
    let toast = app.clone();
    let status: crate::data::Status = Rc::new(move |m: &str| toast.toast(m));
    match e.kind {
        Kind::Enc => {
            if let (Some(enc), Some(aids)) = (app.enc.borrow().clone(), app.aids.borrow().clone()) {
                enc.remove_under(&folder);
                let (app, name) = (app.clone(), e.name.clone());
                spawn(async move {
                    let cells = crate::data::find_enc(vec![folder]).await;
                    app.toast(&gettext_f("Installed {name}: {n} charts", &[("name", &name), ("n", &cells.len().to_string())]));
                    crate::data::load_enc(enc, aids, cells, status);
                });
                return;
            }
        }
        Kind::Raster => {
            if let Some(bsb) = app.bsb.borrow().clone() {
                bsb.remove_under(&folder);
                let files = datadir::kap_files(std::slice::from_ref(&folder));
                app.toast(&gettext_f("Installed {name}: {n} charts", &[("name", &e.name), ("n", &files.len().to_string())]));
                crate::data::load_bsb(bsb, files, status);
                return;
            }
        }
        Kind::Coastline => {
            if let Some(land) = app.land.borrow().clone() {
                land.remove_stores();
                app.toast(&gettext_f("Installed {name}", &[("name", &e.name)]));
                crate::data::load_coast(land, status);
                return;
            }
        }
        _ => {}
    }
    app.toast(&gettext_f("Installed {name}. Restart to show it.", &[("name", &e.name)]));
}

/// Deletes an installed source, after asking, and takes its charts off the chart.
pub async fn uninstall_entry(app: &Rc<AppState>, e: Entry) {
    let Some(folder) = install_folder(&e) else { return };
    if !folder.exists() || INSTALLING.with(|i| i.borrow().contains(&e.id)) {
        return;
    }
    if let Some(shell) = app.shell() {
        let size = LOCAL.with(|l| l.borrow().get(&e.id).and_then(|l| l.size));
        let at = folder.display().to_string();
        let body = match size {
            Some(n) => gettext_f("This deletes {size} of charts from {folder}. You can install them again later.", &[("size", &size_text(n)), ("folder", &at)]),
            None => gettext_f("This deletes the charts in {folder}. You can install them again later.", &[("folder", &at)]),
        };
        if !shell.confirm(&gettext_f("Remove {name}?", &[("name", &e.name)]), &body, &gettext("Remove"), true).await {
            return;
        }
    }
    if let Some(enc) = app.enc.borrow().clone() {
        enc.remove_under(&folder);
    }
    if let Some(bsb) = app.bsb.borrow().clone() {
        bsb.remove_under(&folder);
    }
    if e.kind == Kind::Coastline
        && let Some(land) = app.land.borrow().clone()
    {
        land.remove_stores();
    }
    let (entry, dir) = (e.clone(), datadir::user_data_dir());
    match bg(move || install::uninstall(&entry, &dir)).await {
        Ok(()) => app.toast(&gettext_f("Removed {name}", &[("name", &e.name)])),
        Err(err) => app.toast(&gettext_f("Could not remove {name}: {error}", &[("name", &e.name), ("error", &err.to_string())])),
    }
    LOCAL.with(|l| l.borrow_mut().remove(&e.id));
    app.refresh();
}

/// Adds a folder of charts the user has licensed: it is read from where it is.
pub fn add_folder(app: &Rc<AppState>, path: &str) {
    {
        let mut s = app.settings.borrow_mut();
        if s.chart_folders.iter().any(|f| f == path) {
            return;
        }
        s.chart_folders.push(path.to_string());
    }
    app.apply_settings();
    let (Some(enc), Some(aids), Some(bsb)) = (app.enc.borrow().clone(), app.aids.borrow().clone(), app.bsb.borrow().clone()) else { return };
    let folder = std::path::PathBuf::from(path);
    let app = app.clone();
    spawn(async move {
        let cells = crate::data::find_enc(vec![folder.clone()]).await;
        let rasters = datadir::kap_files(std::slice::from_ref(&folder));
        let n = cells.len() + rasters.len();
        let toast = app.clone();
        let status: crate::data::Status = Rc::new(move |m: &str| toast.toast(m));
        crate::data::load_enc(enc, aids, cells, status.clone());
        crate::data::load_bsb(bsb, rasters, status);
        app.toast(&gettext_f("Added a folder with {n} charts", &[("n", &n.to_string())]));
    });
}

/// Forgets a folder; its charts leave the chart at the next start.
pub fn remove_folder(app: &Rc<AppState>, path: &str) {
    app.settings.borrow_mut().chart_folders.retain(|f| f != path);
    app.apply_settings();
    app.toast(&gettext("The folder is forgotten. Its charts leave the chart when the app restarts."));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(size_text(2048), "2 KB");
        assert_eq!(size_text(3_610_746), "3 MB");
        assert_eq!(size_text(822_000_000), "784 MB");
        assert_eq!(size_text(26_000_000_000), "24.2 GB");
    }

    #[test]
    fn subtitles_warn_about_what_matters() {
        let r = registry::builtin();
        let get = |id: &str| r.entries.iter().find(|e| e.id == id).unwrap().clone();
        let warning = |id: &str| chips(&get(id)).into_iter().find(|c| c.icon == Some(Icon::Warning)).map(|c| c.text);
        assert_eq!(warning("nz-kap-archive").as_deref(), Some("Not up to date, not for navigation"));
        let noaa = get("noaa-enc");
        assert_eq!((rating_letter(noaa.rating), rating_text(noaa.rating).as_str(), warning("noaa-enc")), ("A", "Official", None));
        assert_eq!(warning("shom-enc-paid"), None);
    }

    #[test]
    fn failures_have_a_message() {
        assert_eq!(explain(&InstallError::Cancelled), "Cancelled.");
        assert!(explain(&InstallError::Network("x".into())).contains("network error"));
    }
}
