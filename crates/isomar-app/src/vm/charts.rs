//! The Charts page: the charts loaded for the area on screen, what could be installed for it, and
//! the folders of charts the user added.

use crate::app::AppState;
use crate::chips::{Chip, Icon};
use crate::core::registry;
use crate::flows::charts::{SourceRow, check_installed, entries_here, install_folder, rows_for};
use crate::i18n::{gettext, gettext_f};
use crate::layers::enc::view_scale;
use serde::Serialize;
use std::path::Path;
use std::rc::Rc;

/// A chart drawn at the centre of the view.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct InUse {
    pub name: String,
    /// "Vector chart (S-57)".
    pub kind: String,
    /// Its scale, date and edition, whether it is shown enlarged, where it comes from.
    pub chips: Vec<Chip>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Page {
    /// Sources for the area at the centre of the view, best first. Where a loaded chart covers
    /// the view, only the installed ones (to update or remove): nothing needs downloading.
    pub here: Vec<SourceRow>,
    /// The charts drawn at the centre of the view at this zoom, the one on top first.
    pub in_use: Vec<InUse>,
    /// How many vector (S-57) and raster (BSB/KAP) charts are loaded.
    pub vector: usize,
    pub rasters: usize,
    pub folders: Vec<String>,
    /// No chart the user installed or added covers the view.
    pub planning_only: bool,
    /// Maximum S-57 navigational purpose / usage band to display.
    pub enc_max_band: Option<u8>,
}

/// Also starts checking the installed sources for updates; the page refreshes with the answers.
pub fn page(app: &Rc<AppState>) -> Page {
    check_installed(app, &entries_here(app.canvas.view_state().center()));
    listed(app)
}

/// The page as things stand, without asking any server.
fn listed(app: &AppState) -> Page {
    let planning_only = crate::vm::footer::planning_only(app);
    let mut here = rows_for(app, &entries_here(app.canvas.view_state().center()));
    if !planning_only {
        here.retain(|r| r.installed || r.action == "installing");
    }
    Page {
        here,
        in_use: in_use(app),
        vector: app.enc.borrow().as_ref().map_or(0, |e| e.len()),
        rasters: app.bsb.borrow().as_ref().map_or(0, |b| b.charts().len()),
        folders: app.settings.borrow().chart_folders.clone(),
        planning_only,
        enc_max_band: app.settings.borrow().enc_max_band,
    }
}

/// `1:25 000`, with thin spaces between thousands as on a printed chart.
fn scale_text(scale: u32) -> String {
    let digits = scale.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('\u{202f}');
        }
        out.push(c);
    }
    format!("1:{out}")
}

/// An S-57 issue date (`20240904`) as `2024-09-04`.
fn scale_chip(scale: u32) -> Chip {
    let s = scale_text(scale);
    Chip::new(Icon::Distance, s.clone(), gettext_f("scale {scale}", &[("scale", &s)]))
}

fn enlarged_chip() -> Chip {
    Chip::with(Icon::Zoom, gettext("Shown enlarged"))
}

fn source_chip(src: &str) -> Chip {
    Chip::with(Icon::Source, gettext_f("From {source}", &[("source", src)]))
}

fn date_text(d: &str) -> String {
    if d.len() == 8 && d.bytes().all(|b| b.is_ascii_digit()) { format!("{}-{}-{}", &d[..4], &d[4..6], &d[6..]) } else { d.to_string() }
}

/// Where a chart file comes from: the source it was installed from, or the user's folder.
fn source_of(app: &AppState, path: &Path) -> Option<String> {
    if let Some(e) = registry::builtin().entries.iter().find(|e| install_folder(e).is_some_and(|f| path.starts_with(f))) {
        return Some(e.name.clone());
    }
    app.settings.borrow().chart_folders.iter().find(|f| path.starts_with(f)).cloned()
}

fn in_use(app: &AppState) -> Vec<InUse> {
    let vi = app.canvas.view_info();
    let centre = vi.position_at(vi.w * 0.5, vi.h * 0.5);
    let denom = view_scale(centre.lat, vi.zoom());
    let enlarged = |scale: u32| scale > 0 && denom < scale as f64 / 2.0;
    let mut out = Vec::new();
    // Rasters are drawn over vector charts.
    if app.canvas.layer_visible("bsb")
        && let Some(c) = app.bsb.borrow().as_ref().and_then(|b| b.in_use_at(&vi, centre))
    {
        let file = c.path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let name = if c.header.name.trim().is_empty() { file } else { c.header.name.trim().to_string() };
        let mut chips = Vec::new();
        if c.header.scale > 0 {
            chips.push(scale_chip(c.header.scale));
        }
        if enlarged(c.header.scale) {
            chips.push(enlarged_chip());
        }
        if let Some(src) = source_of(app, &c.path) {
            chips.push(source_chip(&src));
        }
        out.push(InUse { name, kind: gettext("Raster chart (BSB/KAP)"), chips });
    }
    if app.canvas.layer_visible("enc")
        && let Some(enc) = app.enc.borrow().as_ref()
    {
        for (c, path) in enc.all_in_use_at(centre.lon, centre.lat, denom) {
            let mut chips = Vec::new();
            let band = c.usage_band();
            let band_title = c.usage_band_name();
            chips.push(Chip::new(Icon::Zoom, format!("{band_title} (Band {band})"), gettext_f("{title} scale (S-57 Band {band})", &[("title", band_title), ("band", &band.to_string())])));
            if c.cscl > 0 {
                chips.push(scale_chip(c.cscl));
            }
            let edition = gettext_f("edition {n}", &[("n", &format!("{}.{}", c.edition, c.update))]);
            chips.push(if c.issue_date.is_empty() {
                Chip::with(Icon::Date, edition)
            } else {
                let date = date_text(&c.issue_date);
                Chip::new(Icon::Date, format!("{date}, {edition}"), gettext_f("issued {date}, {edition}", &[("date", &date), ("edition", &edition)]))
            });
            if enlarged(c.cscl) {
                chips.push(enlarged_chip());
            }
            if let Some(src) = path.as_deref().and_then(|p| source_of(app, p)) {
                chips.push(source_chip(&src));
            }
            out.push(InUse { name: c.name.trim_end_matches(".000").to_string(), kind: gettext("Vector chart (S-57)"), chips });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::nav::store::NavStore;
    use crate::core::session::Session;
    use crate::engine::Engine;
    use crate::palette::Mode;

    #[test]
    fn the_page_lists_what_exists_for_the_area_on_screen() {
        // Brest: the registry knows French sources.
        let canvas = Engine::new(crate::config::ViewState { lat: 48.38, lon: -4.49, zoom: 10.0 }, Mode::Day);
        let app = AppState::new(canvas, Session::new(NavStore::open_memory().unwrap()).unwrap(), crate::config::Settings::default());
        let p = listed(&app);
        assert!(!p.here.is_empty(), "France has sources");
        assert!(p.here.iter().all(|r| ["install", "installed", "update", "on", "turn_on", "website"].contains(&r.action.as_str())));
        assert!(p.in_use.is_empty() && p.vector == 0 && p.planning_only);
        assert_eq!(p.folders.len(), 0);
    }

    #[test]
    fn scales_read_like_a_printed_chart() {
        assert_eq!(scale_text(25_000), "1:25\u{202f}000");
        assert_eq!(scale_text(1_200_000), "1:1\u{202f}200\u{202f}000");
        assert_eq!(scale_text(500), "1:500");
        assert_eq!(date_text("20240904"), "2024-09-04");
        assert_eq!(date_text("2024"), "2024");
    }
}
