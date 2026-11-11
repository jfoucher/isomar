//! Latitude/longitude grid with degree and minute labels.

use super::{DrawCtx, Layer, TileArea, TilePaint, font_size, halo_text, text_scale};
use crate::core::geo::{self, LatLon};
use crate::palette::{GRID, TEXT};

const MIN_GRID_PX: f64 = 90.0;
/// Width of a graticule line, in points.
const GRID_WIDTH: f64 = 1.0;
/// Graticule steps in degrees, largest first.
const GRID_STEPS: [f64; 14] = [
    60.0,
    30.0,
    10.0,
    5.0,
    2.0,
    1.0,
    0.5,
    1.0 / 6.0,
    1.0 / 12.0,
    1.0 / 30.0,
    1.0 / 60.0,
    1.0 / 120.0,
    1.0 / 300.0,
    1.0 / 600.0,
];

/// The labels along one edge of the view, drawn every frame: the meridians' along the top, the parallels'
/// down the left.
pub struct GraticuleLabels(pub Edge);

#[derive(Clone, Copy, PartialEq)]
pub enum Edge {
    Top,
    Left,
}

/// The lines, drawn into the chart's tiles; shown and hidden with the labels (the same id).
pub struct GraticuleLines;

impl Layer for GraticuleLines {
    fn id(&self) -> &'static str {
        "graticule"
    }

    fn prepare(&self, area: &TileArea) -> Option<std::sync::Arc<dyn TilePaint>> {
        Some(std::sync::Arc::new(LinesPaint { step: grid_step(area.vi.zoom()) }))
    }
}

struct LinesPaint {
    step: f64,
}

impl TilePaint for LinesPaint {
    fn paint(&self, ctx: &DrawCtx) {
        let (cr, vi, pal) = (ctx.cr, &ctx.vi, &ctx.pal);
        let (size, step) = (geo::world_size(vi.zoom()), self.step);
        let (cx, cy) = vi.view.center_px();
        let (left, top) = (cx - vi.w / 2.0, cy - vi.h / 2.0);
        cr.set_line_width(GRID_WIDTH);
        pal.set_alpha(cr, GRID, 0.7);
        for lon in meridians(left, vi.w, size, step) {
            let x = ((lon + 180.0) / 360.0 * size - left).round() + 0.5;
            cr.move_to(x, 0.0);
            cr.line_to(x, vi.h);
        }
        for lat in parallels(top, vi.h, size, vi.zoom(), step) {
            let y = (geo::project(&LatLon::new(lat, 0.0), vi.zoom()).1 - top).round() + 0.5;
            cr.move_to(0.0, y);
            cr.line_to(vi.w, y);
        }
        let _ = cr.stroke();
    }
}

/// The meridians of `step` degrees across the world pixels `left`..`left + w`: their longitudes.
fn meridians(left: f64, w: f64, size: f64, step: f64) -> impl Iterator<Item = f64> {
    let lon0 = left / size * 360.0 - 180.0;
    let lon1 = (left + w) / size * 360.0 - 180.0;
    let first = (lon0 / step).ceil() as i64;
    (first..).map(move |k| k as f64 * step).take_while(move |lon| *lon <= lon1)
}

/// The parallels of `step` degrees down the world pixels `top`..`top + h`: their latitudes.
fn parallels(top: f64, h: f64, size: f64, zoom: f64, step: f64) -> impl Iterator<Item = f64> {
    let lat_top = geo::unproject(0.0, top.clamp(0.0, size), zoom).lat;
    let lat_bot = geo::unproject(0.0, (top + h).clamp(0.0, size), zoom).lat;
    let first = (lat_bot / step).ceil() as i64;
    (first..).map(move |k| k as f64 * step).take_while(move |lat| *lat <= lat_top)
}

/// Lines are kept further apart when the labels are larger.
pub fn grid_step(zoom: f64) -> f64 {
    step_for(zoom, MIN_GRID_PX * text_scale() as f64 / 100.0)
}

/// The finest step at least `min_px` wide at `zoom`. Zoomed out so far that
/// none is, the coarsest: falling back to the finest drew 216,000 lines a frame.
fn step_for(zoom: f64, min_px: f64) -> f64 {
    let px_per_deg = geo::world_size(zoom) / 360.0;
    GRID_STEPS.into_iter().rfind(|s| s * px_per_deg >= min_px).unwrap_or(GRID_STEPS[0])
}

/// Label for a graticule line: whole degrees for coarse grids, degrees and
/// minutes otherwise.
pub fn grid_label(value: f64, step: f64, is_lat: bool) -> String {
    if step >= 1.0 {
        let value = if is_lat { value } else { geo::normalize_lon(value) };
        let hemi = match (is_lat, value >= 0.0) {
            (true, true) => 'N',
            (true, false) => 'S',
            (false, true) => 'E',
            (false, false) => 'W',
        };
        format!("{}°{}", value.abs().round() as i64, hemi)
    } else if step >= 1.0 / 60.0 - 1e-9 {
        whole_minutes(if is_lat { value } else { geo::normalize_lon(value) }, is_lat)
    } else if is_lat {
        geo::format_lat(value)
    } else {
        geo::format_lon(geo::normalize_lon(value))
    }
}

/// Degrees and whole minutes: `48°10′N`, `004°30′W`.
fn whole_minutes(value: f64, is_lat: bool) -> String {
    let hemi = match (is_lat, value >= 0.0) {
        (true, true) => 'N',
        (true, false) => 'S',
        (false, true) => 'E',
        (false, false) => 'W',
    };
    let total = (value.abs() * 60.0).round() as i64;
    let (deg, min) = (total / 60, total % 60);
    if is_lat { format!("{deg:02}°{min:02}′{hemi}") } else { format!("{deg:03}°{min:02}′{hemi}") }
}

impl Layer for GraticuleLabels {
    fn id(&self) -> &'static str {
        "graticule"
    }

    /// Its labels sit at the edges of the screen, not of a tile.
    fn is_dynamic(&self) -> bool {
        true
    }

    fn draw(&self, ctx: &DrawCtx) {
        let (cr, vi, pal) = (ctx.cr, &ctx.vi, &ctx.pal);
        let view = &vi.view;
        let size = geo::world_size(view.zoom);
        let (cx, cy) = view.center_px();
        let (left, top) = (cx - vi.w / 2.0, cy - vi.h / 2.0);
        let step = grid_step(view.zoom);

        cr.select_font_face(crate::layers::FONT_FAMILY, cairo::FontSlant::Normal, cairo::FontWeight::Normal);
        font_size(cr, 11.0);
        if self.0 == Edge::Top {
            for lon in meridians(left, vi.w, size, step) {
                let x = ((lon + 180.0) / 360.0 * size - left).round() + 0.5;
                halo_text(cr, pal, x + 4.0, 14.0 * text_scale() as f64 / 100.0, &grid_label(lon, step, false), TEXT);
            }
        } else {
            for lat in parallels(top, vi.h, size, view.zoom, step) {
                let y = (geo::project(&LatLon::new(lat, 0.0), view.zoom).1 - top).round() + 0.5;
                halo_text(cr, pal, 6.0, y - 4.0, &grid_label(lat, step, true), TEXT);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_gets_finer_as_we_zoom_in() {
        assert!(grid_step(3.0) > grid_step(9.0));
        assert!(grid_step(9.0) > grid_step(15.0));
    }

    #[test]
    fn grid_lines_stay_far_enough_apart() {
        for z in [2.0, 6.5, 10.0, 14.0] {
            let px = grid_step(z) * geo::world_size(z) / 360.0;
            assert!(px >= MIN_GRID_PX, "zoom {z}: {px}");
        }
    }

    #[test]
    fn large_labels_zoomed_out_keep_the_coarsest_step() {
        // At zoom 2 with text at 200%, even 60 degrees is narrower than the minimum.
        assert_eq!(step_for(2.0, MIN_GRID_PX * 2.0), GRID_STEPS[0]);
        assert_eq!(step_for(0.0, MIN_GRID_PX), GRID_STEPS[0]);
    }

    #[test]
    fn labels() {
        assert_eq!(grid_label(48.0, 1.0, true), "48°N");
        assert_eq!(grid_label(-5.0, 5.0, false), "5°W");
        assert_eq!(grid_label(48.5, 0.5, true), "48°30′N");
        assert_eq!(grid_label(-4.5, 0.5, false), "004°30′W");
        assert_eq!(grid_label(48.0 + 10.0 / 60.0, 1.0 / 6.0, true), "48°10′N", "whole minutes");
        assert_eq!(grid_label(48.0 + 1.0 / 120.0, 1.0 / 120.0, true), "48°00.500′N", "finer than a minute keeps decimals");
        // Unwrapped longitudes are normalised for display.
        assert_eq!(grid_label(185.0, 5.0, false), "175°W");
    }
}
