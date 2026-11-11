//! The map view: where the canvas is looking, and the maths between screen
//! pixels and positions.

use crate::config::ViewState;
use crate::core::geo::{self, LatLon};
use crate::core::vector::Bounds;

pub const MIN_ZOOM: f64 = 2.0;
pub const MAX_ZOOM: f64 = 19.0;

/// Zooms are kept to hundredths, so that the chart's tiles (one set per zoom, `render`) line up with the view.
pub fn quantize_zoom(zoom: f64) -> f64 {
    (zoom * 100.0).round() / 100.0
}

#[derive(Clone, Copy, Debug)]
pub struct View {
    pub center: LatLon,
    pub zoom: f64,
}

impl View {
    pub fn from_state(s: ViewState) -> View {
        View { center: s.center(), zoom: quantize_zoom(s.zoom.clamp(MIN_ZOOM, MAX_ZOOM)) }
    }

    pub fn state(&self) -> ViewState {
        ViewState { lat: self.center.lat, lon: self.center.lon, zoom: self.zoom }
    }

    pub fn center_px(&self) -> (f64, f64) {
        geo::project(&self.center, self.zoom)
    }

    /// The position under widget coordinates (`sx`, `sy`).
    pub fn at(&self, sx: f64, sy: f64, w: f64, h: f64) -> LatLon {
        let (cx, cy) = self.center_px();
        geo::unproject(cx + sx - w / 2.0, cy + sy - h / 2.0, self.zoom)
    }

    /// Zooms so the position under (`sx`, `sy`) stays where it is.
    pub fn zoom_at(&mut self, zoom: f64, sx: f64, sy: f64, w: f64, h: f64) {
        let anchor = self.at(sx, sy, w, h);
        self.place(anchor, zoom, sx, sy, w, h);
    }

    /// Sets the zoom and puts `p` at widget coordinates (`sx`, `sy`).
    pub fn place(&mut self, p: LatLon, zoom: f64, sx: f64, sy: f64, w: f64, h: f64) {
        self.zoom = quantize_zoom(zoom.clamp(MIN_ZOOM, MAX_ZOOM));
        let (ax, ay) = geo::project(&p, self.zoom);
        self.set_center_px(ax - (sx - w / 2.0), ay - (sy - h / 2.0));
    }

    pub fn set_center_px(&mut self, x: f64, y: f64) {
        let size = geo::world_size(self.zoom);
        self.center = geo::unproject(x, y.clamp(0.0, size), self.zoom);
    }

    /// Frames `b` inside a `w` x `h` widget with some margin.
    pub fn fit(&mut self, b: &Bounds, w: f64, h: f64) {
        self.center = LatLon::new((b.min_lat + b.max_lat) / 2.0, (b.min_lon + b.max_lon) / 2.0);
        let mut z = MAX_ZOOM;
        while z > MIN_ZOOM {
            let (x0, y0) = geo::project(&LatLon::new(b.max_lat, b.min_lon), z);
            let (x1, y1) = geo::project(&LatLon::new(b.min_lat, b.max_lon), z);
            if (x1 - x0).abs() <= w * 0.85 && (y1 - y0).abs() <= h * 0.85 {
                break;
            }
            z -= 0.25;
        }
        self.zoom = z;
    }
}

/// `ViewInfo::to_screen` with what does not depend on the point worked out once.
#[derive(Clone, Copy, Debug)]
pub struct Projector {
    cx: f64,
    cy: f64,
    size: f64,
    half_w: f64,
    half_h: f64,
}

impl Projector {
    pub fn to_screen(&self, p: &LatLon) -> (f64, f64) {
        let lat = p.lat.clamp(-geo::MAX_LAT, geo::MAX_LAT).to_radians();
        let wx = (p.lon + 180.0) / 360.0 * self.size;
        let wy = (1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / std::f64::consts::PI) / 2.0 * self.size;
        let mut dx = wx - self.cx;
        if dx > self.size / 2.0 {
            dx -= self.size;
        } else if dx < -self.size / 2.0 {
            dx += self.size;
        }
        (self.half_w + dx, self.half_h + (wy - self.cy))
    }
}

/// A view together with the widget size: everything a layer needs to place
/// things on screen.
#[derive(Clone, Copy, Debug)]
pub struct ViewInfo {
    pub view: View,
    pub w: f64,
    pub h: f64,
}

impl ViewInfo {
    pub fn zoom(&self) -> f64 {
        self.view.zoom
    }

    /// Screen position of a point, taking the shortest way round the world
    /// horizontally so features near the antimeridian stay together.
    pub fn to_screen(self, p: &LatLon) -> (f64, f64) {
        let (px, py) = geo::project(p, self.view.zoom);
        self.world_to_screen(px, py)
    }

    /// Screen position of the world pixel (`wx`, `wy`) at the view's zoom, taking the shortest way
    /// round the world horizontally like `to_screen`.
    pub fn world_to_screen(&self, wx: f64, wy: f64) -> (f64, f64) {
        let (cx, cy) = self.view.center_px();
        let size = geo::world_size(self.view.zoom);
        let mut dx = wx - cx;
        if dx > size / 2.0 {
            dx -= size;
        } else if dx < -size / 2.0 {
            dx += size;
        }
        (self.w / 2.0 + dx, self.h / 2.0 + (wy - cy))
    }

    /// `to_screen` for many points: the view's centre and the world's size are worked out once, not again
    /// for every point (which was two of the three projections a point cost). The same arithmetic, so the
    /// same positions.
    pub fn projector(&self) -> Projector {
        let (cx, cy) = self.view.center_px();
        Projector { cx, cy, size: geo::world_size(self.view.zoom), half_w: self.w / 2.0, half_h: self.h / 2.0 }
    }

    /// World pixels per unit (Web Mercator at zoom 0, 256 across) at the view's zoom.
    pub fn ppu(&self) -> f64 {
        geo::world_size(self.view.zoom) / 256.0
    }

    /// A point in units (Web Mercator at zoom 0, 256 across) as a world pixel at the view's zoom.
    pub fn world_px(&self, units: [f64; 2]) -> (f64, f64) {
        let k = geo::world_size(self.view.zoom) / 256.0;
        (units[0] * k, units[1] * k)
    }

    pub fn position_at(&self, x: f64, y: f64) -> LatLon {
        self.view.at(x, y, self.w, self.h)
    }

    /// Visible lon/lat bounds, clamped to the world.
    pub fn bounds(&self) -> Bounds {
        let (cx, cy) = self.view.center_px();
        let size = geo::world_size(self.view.zoom);
        let top = (cy - self.h / 2.0).clamp(0.0, size);
        let bot = (cy + self.h / 2.0).clamp(0.0, size);
        let lon = |x: f64| (x / size * 360.0 - 180.0).clamp(-180.0, 180.0);
        Bounds {
            min_lon: lon(cx - self.w / 2.0),
            max_lon: lon(cx + self.w / 2.0),
            min_lat: geo::unproject(0.0, bot, self.view.zoom).lat,
            max_lat: geo::unproject(0.0, top, self.view.zoom).lat,
        }
    }

    /// Degrees of longitude per pixel.
    pub fn deg_per_px(&self) -> f64 {
        360.0 / geo::world_size(self.view.zoom)
    }

    pub fn metres_per_px(&self) -> f64 {
        geo::metres_per_pixel(self.view.center.lat, self.view.zoom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_projector_places_points_where_to_screen_does() {
        for (lat, lon, zoom) in [(48.38, -4.5, 12.0), (42.33, -71.0, 13.37), (-38.9, 179.9, 6.5), (0.0, 0.0, 0.0), (85.0, -179.9, 18.0)] {
            let vi = ViewInfo { view: View { center: LatLon::new(lat, lon), zoom }, w: 384.0, h: 384.0 };
            let proj = vi.projector();
            for (dlat, dlon) in [(0.0, 0.0), (0.01, -0.02), (-0.3, 0.4), (1.0, 181.0), (-2.0, -359.0)] {
                let p = LatLon::new((lat + dlat).clamp(-89.0, 89.0), lon + dlon);
                assert_eq!(proj.to_screen(&p), vi.to_screen(&p), "{p:?} at {zoom}");
            }
        }
    }

    fn info(lat: f64, lon: f64, zoom: f64) -> ViewInfo {
        ViewInfo { view: View { center: LatLon::new(lat, lon), zoom }, w: 1000.0, h: 600.0 }
    }

    #[test]
    fn centre_maps_to_the_middle_of_the_widget() {
        let v = info(48.0, -4.0, 9.0);
        let (x, y) = v.to_screen(&LatLon::new(48.0, -4.0));
        assert!((x - 500.0).abs() < 1e-6 && (y - 300.0).abs() < 1e-6);
    }

    #[test]
    fn world_to_screen_matches_to_screen() {
        let v = info(48.0, -4.0, 11.0);
        for p in [LatLon::new(48.01, -4.02), LatLon::new(47.0, -5.5), LatLon::new(49.3, -2.0)] {
            let (wx, wy) = geo::project(&p, 11.0);
            let (a, b) = (v.to_screen(&p), v.world_to_screen(wx, wy));
            assert!((a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9);
            // The same from units.
            let (ux, uy) = geo::project(&p, 0.0);
            let c = v.world_to_screen(v.world_px([ux, uy]).0, v.world_px([ux, uy]).1);
            assert!((a.0 - c.0).abs() < 1e-6 && (a.1 - c.1).abs() < 1e-6);
        }
        // Round the antimeridian too.
        let v = info(0.0, 179.9, 8.0);
        let (wx, wy) = geo::project(&LatLon::new(0.0, -179.95), 8.0);
        assert_eq!(v.world_to_screen(wx, wy), v.to_screen(&LatLon::new(0.0, -179.95)));
    }

    #[test]
    fn screen_roundtrip() {
        let v = info(48.0, -4.0, 11.0);
        let p = LatLon::new(48.01, -4.02);
        let (x, y) = v.to_screen(&p);
        let q = v.position_at(x, y);
        assert!((p.lat - q.lat).abs() < 1e-9 && (p.lon - q.lon).abs() < 1e-9);
    }

    #[test]
    fn features_near_the_antimeridian_stay_together() {
        let v = info(0.0, 179.9, 8.0);
        let (x1, _) = v.to_screen(&LatLon::new(0.0, 179.95));
        let (x2, _) = v.to_screen(&LatLon::new(0.0, -179.95));
        assert!(x2 > x1 && x2 - x1 < 200.0, "{x1} {x2}");
    }

    #[test]
    fn bounds_contain_the_centre_and_scale_with_zoom() {
        let a = info(48.0, -4.0, 9.0).bounds();
        let b = info(48.0, -4.0, 11.0).bounds();
        assert!(a.contains([-4.0, 48.0]) && b.contains([-4.0, 48.0]));
        assert!(a.width() > b.width() * 3.0);
    }

    #[test]
    fn fit_frames_the_requested_area() {
        let mut v = View { center: LatLon::new(0.0, 0.0), zoom: 3.0 };
        let target = Bounds { min_lon: -5.0, min_lat: 48.0, max_lon: -4.0, max_lat: 48.5 };
        v.fit(&target, 1000.0, 600.0);
        let vis = ViewInfo { view: v, w: 1000.0, h: 600.0 }.bounds();
        assert!(vis.min_lon < target.min_lon && vis.max_lon > target.max_lon);
        assert!(vis.min_lat < target.min_lat && vis.max_lat > target.max_lat);
        assert!(vis.width() < 3.0 * target.width());
    }

    #[test]
    fn zoom_is_clamped() {
        let mut v = View { center: LatLon::new(0.0, 0.0), zoom: 5.0 };
        v.zoom_at(99.0, 10.0, 10.0, 100.0, 100.0);
        assert_eq!(v.zoom, MAX_ZOOM);
        v.zoom_at(-5.0, 10.0, 10.0, 100.0, 100.0);
        assert_eq!(v.zoom, MIN_ZOOM);
    }
}
