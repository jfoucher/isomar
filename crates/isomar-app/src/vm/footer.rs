//! The strip along the bottom of the chart: scale bars, the data credits and the
//! "Planning only" pill. The Mac draws it natively from this; GTK keeps its own scale bar and credit layer.

use crate::app::AppState;
use crate::chips::{Chip, Icon};
use crate::core::geo::{self, LatLon};
use crate::engine::scale_bar_length;
use crate::i18n::{gettext, gettext_f};
use serde::Serialize;

/// A scale bar: how long it is on screen, and what it says.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Bar {
    pub px: f64,
    pub label: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Pointer {
    /// The position alone, to copy.
    pub position: String,
    /// The position, the depth there and the magnetic variation (from a chart cell in use or the
    /// World Magnetic Model).
    pub chips: Vec<Chip>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Footer {
    /// Nautical miles (or metres, zoomed in far) and, on a second scale, metres or kilometres.
    pub scale: Bar,
    pub metric: Bar,
    pub credits: Vec<String>,
    /// No licensed official chart covers the view: it is a planning chart, not for navigation.
    pub planning_only: bool,
    pub planning_text: String,
    pub pointer: Option<Pointer>,
    /// What the footer says when the pointer is off the chart: the zoom and the scale.
    pub view: Vec<Chip>,
}

/// A round metric length for the second scale: metres, then kilometres.
pub fn metric_bar_length(metres_per_px: f64, max_px: f64) -> (f64, String) {
    const STEPS: [f64; 16] = [5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10_000.0, 20_000.0, 50_000.0, 100_000.0, 200_000.0, 500_000.0];
    let max_m = metres_per_px * max_px;
    let m = STEPS.iter().copied().rfind(|s| *s <= max_m).unwrap_or(STEPS[0]);
    let label = if m >= 1000.0 { format!("{} km", m / 1000.0) } else { format!("{m} m") };
    (m / metres_per_px, label)
}

/// Whether a chart the user installed or added (an S-57 cell or a raster chart) is drawn over the
/// view: at its centre and at least two of four points inset from the corners. Drawn, as "In use
/// here" says, not merely loaded: an overview cell or a chart whose box spans the antimeridian
/// must not hide the warning where nothing shows.
pub fn planning_only(app: &AppState) -> bool {
    use crate::layers::Layer;
    let vi = app.canvas.view_info();
    // Asked for on every move of the pointer and of the view: the same view over the same charts has the
    // same answer, and working it out tries every raster chart.
    let rev = |l: Option<&dyn Layer>| l.map_or(u64::MAX, |l| l.revision());
    let key = (
        [vi.view.center.lat, vi.view.center.lon, vi.zoom(), vi.w, vi.h].map(f64::to_bits),
        [rev(app.enc.borrow().as_deref().map(|l| l as &dyn Layer)), rev(app.bsb.borrow().as_deref().map(|l| l as &dyn Layer)), rev(app.land.borrow().as_deref().map(|l| l as &dyn Layer))],
        (app.canvas.layer_visible("enc"), app.canvas.layer_visible("bsb")),
    );
    thread_local! {
        static LAST: std::cell::RefCell<Option<(PlanningKey, bool)>> = const { std::cell::RefCell::new(None) };
    }
    if let Some(answer) = LAST.with(|l| l.borrow().as_ref().filter(|(k, _)| *k == key).map(|(_, a)| *a)) {
        return answer;
    }
    let answer = work_out_planning_only(app, &vi);
    LAST.with(|l| *l.borrow_mut() = Some((key, answer)));
    answer
}

type PlanningKey = ([u64; 5], [u64; 3], (bool, bool));

fn work_out_planning_only(app: &AppState, vi: &crate::view::ViewInfo) -> bool {
    let (w, h) = (vi.w, vi.h);
    let points = [(0.5, 0.5), (0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)];
    let enc = app.enc.borrow().clone().filter(|_| app.canvas.layer_visible("enc"));
    let bsb = app.bsb.borrow().clone().filter(|_| app.canvas.layer_visible("bsb"));
    let covered = |p: LatLon| {
        let denom = crate::layers::enc::view_scale(p.lat, vi.zoom());
        enc.as_ref().is_some_and(|e| e.in_use_at(p.lon, p.lat, denom).is_some()) || bsb.as_ref().is_some_and(|b| b.in_use_at(vi, p).is_some())
    };
    let hits = points.iter().filter(|(fx, fy)| covered(vi.position_at(w * fx, h * fy))).count();
    let centre = covered(vi.position_at(w * 0.5, h * 0.5));
    !(centre && hits >= 3)
}

pub fn footer(app: &AppState) -> Footer {
    let vi = app.canvas.view_info();
    let mpp = vi.metres_per_px();
    let (px, label) = scale_bar_length(mpp, 150.0);
    let (mpx, mlabel) = metric_bar_length(mpp, 150.0);
    let credits = app.credit.borrow().as_ref().map(|c| c.shown.borrow().iter().map(|s| s.to_string()).collect()).unwrap_or_default();
    let planning_only = planning_only(app);
    let pointer = app.pointer.get().map(|p| pointer(app, &p));
    let _ = geo::NM_M;
    Footer {
        scale: Bar { px, label },
        metric: Bar { px: mpx, label: mlabel },
        credits,
        planning_only,
        planning_text: gettext("Planning only: no official chart covers this area. Not for navigation."),
        pointer,
        view: crate::host::view_chips(&app.canvas, vi.zoom()),
    }
}

/// What the footer says of the point under the pointer.
pub fn pointer(app: &AppState, p: &LatLon) -> Pointer {
    let position = crate::text::position_text(p);
    let mut chips = vec![Chip::with(Icon::Position, position.clone())];
    if let Some(d) = app.depth.borrow().as_ref().and_then(|d| d.depth_at(p.lon, p.lat)) {
        let d = format!("{d:.1}");
        chips.push(Chip::new(Icon::Depth, format!("{d} m"), gettext_f("Depth {d} m", &[("d", &d)])));
    }
    if let Some(v) = crate::variation::at(app, p) {
        chips.push(Chip::new(Icon::Compass, crate::variation::value_text(&v), crate::variation::text(&v)));
    }
    Pointer { position, chips }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_metric_bar_is_a_round_length_that_fits() {
        let (px, label) = metric_bar_length(10.0, 150.0);
        assert_eq!(label, "1 km");
        assert!(px <= 150.0 && px > 50.0);
        let (_, label) = metric_bar_length(0.5, 150.0);
        assert_eq!(label, "50 m");
        let (_, label) = metric_bar_length(500.0, 150.0);
        assert_eq!(label, "50 km");
    }

    #[test]
    fn with_no_charts_the_view_is_planning_only() {
        use crate::core::nav::store::NavStore;
        use crate::core::session::Session;
        use crate::engine::Engine;
        use crate::palette::Mode;
        let canvas = Engine::new(crate::config::ViewState { lat: 48.0, lon: -4.0, zoom: 10.0 }, Mode::Day);
        let app = AppState::new(canvas, Session::new(NavStore::open_memory().unwrap()).unwrap(), crate::config::Settings::default());
        let f = footer(&app);
        assert!(f.planning_only);
        assert!(f.credits.is_empty() && f.pointer.is_none());
        assert!(f.scale.px > 0.0 && f.metric.px > 0.0);
    }
}
