//! Importing and exporting GPX files (OpenCPN-compatible; the format is `core::nav::gpx`).

use crate::core::nav::gpx;
use crate::i18n::{gettext, gettext_f};
use crate::ui::app::AppState;
use crate::ui::{show_error, spawn};
use adw::prelude::*;
use gtk::gio;
use std::rc::Rc;

fn gpx_filter() -> gio::ListStore {
    let f = gtk::FileFilter::new();
    f.set_name(Some(&gettext("GPX files")));
    f.add_pattern("*.gpx");
    f.add_pattern("*.xml");
    let store = gio::ListStore::new::<gtk::FileFilter>();
    store.append(&f);
    store
}

fn window_of(_app: &AppState) -> Option<gtk::Window> {
    crate::ui::chart_widget().root().and_then(|r| r.downcast::<gtk::Window>().ok())
}

pub fn import_gpx(app: &Rc<AppState>) {
    let app = app.clone();
    spawn(async move {
        let dialog = gtk::FileDialog::builder().title(gettext("Import GPX")).filters(&gpx_filter()).modal(false).build();
        let Ok(file) = dialog.open_future(window_of(&app).as_ref()).await else { return };
        let Some(path) = file.path() else { return };
        let text = match crate::ui::bg(move || std::fs::read_to_string(path)).await {
            Ok(t) => t,
            Err(e) => {
                show_error(&gettext("Could not read the file"), &e.to_string());
                return;
            }
        };
        match gpx::parse(&text) {
            Ok(doc) => {
                let res = {
                    let mut s = app.session.borrow_mut();
                    let r = s.store.import(&doc);
                    if r.is_ok() {
                        s.waypoints = s.store.waypoints().unwrap_or_default();
                        s.routes = s.store.routes().unwrap_or_default();
                        s.tracks = s.store.tracks().unwrap_or_default();
                    }
                    r
                };
                match res {
                    Ok((w, r, t)) => {
                        let mut msg = gettext_f("Imported {w} waypoints, {r} routes, {t} tracks", &[("w", &w.to_string()), ("r", &r.to_string()), ("t", &t.to_string())]);
                        if !doc.warnings.is_empty() {
                            msg.push_str(&format!(" ({})", gettext_f("{n} items skipped", &[("n", &doc.warnings.len().to_string())])));
                        }
                        app.toast(&msg);
                        app.refresh();
                    }
                    Err(e) => show_error(&gettext("Could not store the data"), &e.to_string()),
                }
            }
            Err(e) => show_error(&gettext("This is not a usable GPX file"), &e),
        }
    });
}

pub fn export_gpx(app: &Rc<AppState>) {
    let doc = {
        let s = app.session.borrow();
        gpx::GpxDoc { waypoints: s.waypoints.clone(), routes: s.routes.clone(), tracks: s.tracks.clone(), warnings: vec![] }
    };
    save_gpx(app, doc, "isomar.gpx".into());
}

/// One route on its own, for apps that import a GPX file (Navionics, phones).
pub fn export_route_gpx(app: &Rc<AppState>, id: i64) {
    let Some(r) = app.session.borrow().routes.iter().find(|r| r.id == id).cloned() else { return };
    let name = format!("{}.gpx", file_stem(&r.name));
    save_gpx(app, gpx::GpxDoc { waypoints: vec![], routes: vec![r], tracks: vec![], warnings: vec![] }, name);
}

/// A route name as a file name: no path separators or characters some file systems refuse.
fn file_stem(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '_' } else { c }).collect();
    let s = s.trim().trim_start_matches('.');
    if s.is_empty() { "route".into() } else { s.into() }
}

fn save_gpx(app: &Rc<AppState>, doc: gpx::GpxDoc, initial_name: String) {
    let app = app.clone();
    spawn(async move {
        let dialog = gtk::FileDialog::builder().title(gettext("Export GPX")).initial_name(initial_name).filters(&gpx_filter()).modal(false).build();
        let Ok(file) = dialog.save_future(window_of(&app).as_ref()).await else { return };
        let Some(path) = file.path() else { return };
        let xml = gpx::write(&doc);
        match crate::ui::bg(move || std::fs::write(path, xml)).await {
            Ok(()) => app.toast(&gettext("Exported")),
            Err(e) => show_error(&gettext("Could not write the file"), &e.to_string()),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn route_names_make_safe_file_names() {
        assert_eq!(file_stem("Brest → Penmarc'h"), "Brest → Penmarc'h");
        assert_eq!(file_stem("a/b:c"), "a_b_c");
        assert_eq!(file_stem("  ../x "), "_x");
        assert_eq!(file_stem("..."), "route");
        assert_eq!(file_stem(""), "route");
    }
}
