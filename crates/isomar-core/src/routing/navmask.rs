//! A navigability mask for the automatic router: land, water shallower than
//! the safety depth and chart hazards, each grown by a clearance.
//!
//! The grid is laid out in Mercator coordinates, so a straight line on it is a
//! rhumb line (the default leg kind) and cells are square on the ground. Rows
//! run from the south.

use super::land::Land;
use crate::chart::grid::Grid;
use crate::geo::{EARTH_RADIUS_M, LatLon, NM_M};
use crate::vector::{Bounds, Polygon};
use std::f64::consts::PI;
use std::sync::Arc;

const R_NM: f64 = EARTH_RADIUS_M / NM_M;

/// Mercator x and y in nautical miles at the equator.
pub fn merc_x(lon: f64) -> f64 {
    lon.to_radians() * R_NM
}

pub fn merc_y(lat: f64) -> f64 {
    (PI / 4.0 + lat.to_radians() / 2.0).tan().ln() * R_NM
}

fn inv_x(x: f64) -> f64 {
    (x / R_NM).to_degrees()
}

fn inv_y(y: f64) -> f64 {
    (y / R_NM).sinh().atan().to_degrees()
}

/// Never closer than this to a hazard, in cells: it keeps a leg from slipping
/// between two diagonal hazard cells.
const MIN_CLEARANCE_CELLS: f64 = 1.5;

/// Cost of passing right at the clearance limit, relative to open water.
const BAND_PENALTY: f32 = 2.0;

#[derive(Debug, PartialEq)]
pub enum MaskError {
    TooLarge { cells: u64 },
}

impl std::fmt::Display for MaskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MaskError::TooLarge { cells } => write!(f, "the area needs {cells} cells"),
        }
    }
}

pub struct MaskParams {
    pub bounds: Bounds,
    /// Cell edge in nautical miles on the ground at the middle of `bounds`.
    pub cell_nm: f64,
    /// Water shallower than this (metres, positive) is blocked.
    pub safety_m: f64,
    pub clearance_nm: f64,
    /// Width of the preferred-distance band beyond the clearance.
    pub band_nm: f64,
    pub max_cells: u64,
}

pub struct NavMask {
    x0: f64,
    y0: f64,
    /// Cell edge in Mercator nautical miles.
    cell: f64,
    cols: usize,
    rows: usize,
    /// Land, shoal or hazard, before any clearance.
    hazard: Vec<bool>,
    /// Metres below the datum; NaN where no source has data.
    depth: Vec<f32>,
    /// Distance in cells to the nearest hazard cell.
    clear: Vec<f32>,
    min_clear: f32,
    band: f32,
    /// Ground nautical miles per cell edge in each row.
    row_nm: Vec<f32>,
    bounds: Bounds,
}

impl NavMask {
    /// Cells a mask for `bounds` needs at `cell_nm`.
    pub fn cell_count(bounds: &Bounds, cell_nm: f64) -> u64 {
        let k = ((bounds.min_lat + bounds.max_lat) / 2.0).to_radians().cos().max(0.01);
        let cell = cell_nm / k;
        let cols = ((merc_x(bounds.max_lon) - merc_x(bounds.min_lon)) / cell).ceil().max(1.0) as u64;
        let rows = ((merc_y(bounds.max_lat) - merc_y(bounds.min_lat)) / cell).ceil().max(1.0) as u64;
        cols * rows
    }

    /// Rasterises `land` (even-odd), then marks every cell where the finest of
    /// `grids` has less than the safety depth. A grid cannot tell drying ground
    /// from land, and either way the cell is blocked.
    pub fn build(p: &MaskParams, land: &[Polygon], grids: &[Arc<Grid>]) -> Result<NavMask, MaskError> {
        let cells = NavMask::cell_count(&p.bounds, p.cell_nm);
        if cells > p.max_cells {
            return Err(MaskError::TooLarge { cells });
        }
        let b = p.bounds;
        let k = ((b.min_lat + b.max_lat) / 2.0).to_radians().cos().max(0.01);
        let cell = p.cell_nm / k;
        let (x0, y0) = (merc_x(b.min_lon), merc_y(b.min_lat));
        let cols = ((merc_x(b.max_lon) - x0) / cell).ceil().max(1.0) as usize;
        let rows = ((merc_y(b.max_lat) - y0) / cell).ceil().max(1.0) as usize;
        let n = cols * rows;
        let mut hazard = vec![false; n];
        let mut depth = vec![f32::NAN; n];

        for poly in land {
            let pb = poly.bounds();
            if !pb.intersects(&b) {
                continue;
            }
            let r0 = (((merc_y(pb.min_lat.max(b.min_lat)) - y0) / cell).floor().max(0.0)) as usize;
            let r1 = (((merc_y(pb.max_lat.min(b.max_lat)) - y0) / cell).ceil().max(0.0) as usize).min(rows);
            for r in r0..r1 {
                let lat = inv_y(y0 + (r as f64 + 0.5) * cell);
                let mut xs: Vec<f64> = Vec::new();
                for ring in &poly.rings {
                    let m = ring.len();
                    for i in 0..m {
                        let (a, c) = (ring[i], ring[(i + 1) % m]);
                        if (a[1] > lat) != (c[1] > lat) {
                            xs.push(a[0] + (lat - a[1]) / (c[1] - a[1]) * (c[0] - a[0]));
                        }
                    }
                }
                xs.sort_by(|a, b| a.total_cmp(b));
                for pair in xs.chunks(2) {
                    if let [xa, xb] = pair {
                        // Cells whose centre lies between the two crossings.
                        let c0 = ((merc_x(*xa) - x0) / cell - 0.5).ceil().max(0.0) as usize;
                        let c1 = (((merc_x(*xb) - x0) / cell - 0.5).floor() + 1.0).max(0.0) as usize;
                        for c in c0..c1.min(cols) {
                            hazard[r * cols + c] = true;
                        }
                    }
                }
            }
        }

        // Finest grid first, so it wins where grids overlap.
        let mut sorted: Vec<&Arc<Grid>> = grids.iter().filter(|g| g.bounds().intersects(&b)).collect();
        sorted.sort_by(|a, b| a.dx.total_cmp(&b.dx));
        if !sorted.is_empty() {
            let lons: Vec<f64> = (0..cols).map(|c| inv_x(x0 + (c as f64 + 0.5) * cell)).collect();
            for r in 0..rows {
                let lat = inv_y(y0 + (r as f64 + 0.5) * cell);
                for (c, lon) in lons.iter().enumerate() {
                    if let Some(e) = sorted.iter().find_map(|g| g.sample(*lon, lat)) {
                        let i = r * cols + c;
                        depth[i] = -e;
                        if (-e as f64) < p.safety_m {
                            hazard[i] = true;
                        }
                    }
                }
            }
        }

        let row_nm: Vec<f32> = (0..rows).map(|r| (cell * inv_y(y0 + (r as f64 + 0.5) * cell).to_radians().cos()) as f32).collect();
        let clear = distance_to_hazard(&hazard, cols, rows);
        let ground_cell = p.cell_nm.max(1e-9);
        Ok(NavMask {
            x0,
            y0,
            cell,
            cols,
            rows,
            hazard,
            depth,
            clear,
            min_clear: (p.clearance_nm / ground_cell).max(MIN_CLEARANCE_CELLS) as f32,
            band: (p.band_nm / ground_cell).max(1.0) as f32,
            row_nm,
            bounds: b,
        })
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn bounds(&self) -> Bounds {
        self.bounds
    }

    /// Smallest ground length of a cell edge, nautical miles.
    pub fn min_row_nm(&self) -> f32 {
        self.row_nm.iter().copied().fold(f32::INFINITY, f32::min)
    }

    pub fn row_nm(&self, row: usize) -> f32 {
        self.row_nm[row]
    }

    /// Fractional grid coordinates of a position; a cell is `floor` of them.
    pub fn to_grid(&self, p: &LatLon) -> (f64, f64) {
        ((merc_x(p.lon) - self.x0) / self.cell, (merc_y(p.lat) - self.y0) / self.cell)
    }

    pub fn to_latlon(&self, u: f64, v: f64) -> LatLon {
        LatLon::new(inv_y(self.y0 + v * self.cell), inv_x(self.x0 + u * self.cell))
    }

    pub fn cell_at(&self, p: &LatLon) -> Option<(usize, usize)> {
        let (u, v) = self.to_grid(p);
        if u < 0.0 || v < 0.0 || u >= self.cols as f64 || v >= self.rows as f64 {
            return None;
        }
        Some((u as usize, v as usize))
    }

    /// Centre of a cell in grid coordinates.
    pub fn centre(c: usize, r: usize) -> (f64, f64) {
        (c as f64 + 0.5, r as f64 + 0.5)
    }

    pub fn index(&self, c: usize, r: usize) -> usize {
        r * self.cols + c
    }

    /// True when a vessel may be in the cell: not a hazard and clear of one.
    pub fn navigable(&self, c: usize, r: usize) -> bool {
        let i = r * self.cols + c;
        !self.hazard[i] && self.clear[i] >= self.min_clear
    }

    pub fn is_hazard(&self, c: usize, r: usize) -> bool {
        self.hazard[r * self.cols + c]
    }

    /// Depth in metres, `None` when no source covered the cell.
    pub fn depth_at(&self, c: usize, r: usize) -> Option<f32> {
        let d = self.depth[r * self.cols + c];
        (!d.is_nan()).then_some(d)
    }

    /// Cost of a step through the cell, relative to open water of known depth:
    /// dearer close to a hazard and where the depth is a guess.
    pub fn cost_factor(&self, c: usize, r: usize) -> f32 {
        let i = r * self.cols + c;
        let mut f = 1.0;
        let over = self.clear[i] - self.min_clear;
        if over < self.band {
            f += BAND_PENALTY * (1.0 - (over / self.band).clamp(0.0, 1.0));
        }
        if self.depth[i].is_nan() {
            f += 1.0;
        }
        f
    }

    /// Whether a straight line between two points in grid coordinates stays on
    /// navigable cells. Anything outside the mask counts as blocked.
    pub fn segment_ok(&self, a: (f64, f64), b: (f64, f64)) -> bool {
        let mut ok = true;
        self.walk(a, b, |c, r| {
            if !self.navigable(c, r) {
                ok = false;
            }
            ok
        }) && ok
    }

    /// Calls `f` for each cell along the line, about every half cell, until `f`
    /// returns false. Returns false when the line leaves the mask.
    pub fn walk(&self, a: (f64, f64), b: (f64, f64), mut f: impl FnMut(usize, usize) -> bool) -> bool {
        let len = (b.0 - a.0).hypot(b.1 - a.1);
        let n = ((len * 2.0).ceil() as usize).max(1);
        let mut last = (usize::MAX, usize::MAX);
        for i in 0..=n {
            let t = i as f64 / n as f64;
            let (u, v) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            if u < 0.0 || v < 0.0 || u >= self.cols as f64 || v >= self.rows as f64 {
                return false;
            }
            let cell = (u as usize, v as usize);
            if cell != last {
                last = cell;
                if !f(cell.0, cell.1) {
                    return true;
                }
            }
        }
        true
    }
}

impl Land for NavMask {
    /// True when the leg touches a cell that is not navigable. Parts outside
    /// the mask are not judged.
    fn blocked(&self, a: &LatLon, b: &LatLon) -> bool {
        let (pa, pb) = (self.to_grid(a), self.to_grid(b));
        let mut blocked = false;
        let len = (pb.0 - pa.0).hypot(pb.1 - pa.1);
        let n = ((len * 2.0).ceil() as usize).max(1);
        for i in 0..=n {
            let t = i as f64 / n as f64;
            let (u, v) = (pa.0 + (pb.0 - pa.0) * t, pa.1 + (pb.1 - pa.1) * t);
            if u >= 0.0 && v >= 0.0 && u < self.cols as f64 && v < self.rows as f64 && !self.navigable(u as usize, v as usize) {
                blocked = true;
                break;
            }
        }
        blocked
    }
}

/// Exact Euclidean distance, in cells, from every cell to the nearest hazard
/// cell (Felzenszwalb and Huttenlocher). Very large where there is none.
fn distance_to_hazard(hazard: &[bool], cols: usize, rows: usize) -> Vec<f32> {
    const INF: f64 = 1e18;
    let mut g = vec![0.0f64; cols * rows];
    let longest = cols.max(rows);
    let (mut f, mut d) = (vec![0.0f64; longest], vec![0.0f64; longest]);
    let (mut v, mut z) = (vec![0usize; longest], vec![0.0f64; longest + 1]);
    for c in 0..cols {
        for r in 0..rows {
            f[r] = if hazard[r * cols + c] { 0.0 } else { INF };
        }
        squared_distance_1d(&f[..rows], &mut d[..rows], &mut v, &mut z);
        for r in 0..rows {
            g[r * cols + c] = d[r];
        }
    }
    for r in 0..rows {
        f[..cols].copy_from_slice(&g[r * cols..(r + 1) * cols]);
        squared_distance_1d(&f[..cols], &mut d[..cols], &mut v, &mut z);
        g[r * cols..(r + 1) * cols].copy_from_slice(&d[..cols]);
    }
    g.into_iter().map(|d| d.sqrt() as f32).collect()
}

/// Lower envelope of the parabolas `(q - i)^2 + f[i]`.
fn squared_distance_1d(f: &[f64], d: &mut [f64], v: &mut [usize], z: &mut [f64]) {
    let n = f.len();
    let mut k = 0usize;
    v[0] = 0;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    let cross = |q: usize, p: usize| ((f[q] + (q * q) as f64) - (f[p] + (p * p) as f64)) / (2.0 * (q as f64 - p as f64));
    for q in 1..n {
        let mut s = cross(q, v[k]);
        while s <= z[k] {
            k -= 1;
            s = cross(q, v[k]);
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f64::INFINITY;
    }
    k = 0;
    for (q, out) in d.iter_mut().enumerate() {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let dq = q as f64 - v[k] as f64;
        *out = dq * dq + f[v[k]];
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    use super::*;

    /// A grid of constant elevation (negative below the datum) with a block of
    /// another value, for building synthetic seabeds.
    pub fn grid(bounds: Bounds, res: f64, elev: f32) -> Grid {
        let ncols = (bounds.width() / res).round() as usize;
        let nrows = (bounds.height() / res).round() as usize;
        Grid { ncols, nrows, xll: bounds.min_lon, yll: bounds.min_lat, dx: res, dy: res, data: vec![elev; ncols * nrows] }
    }

    /// Sets the elevation of every sample inside `area`.
    pub fn fill(g: &mut Grid, area: Bounds, elev: f32) {
        for row in 0..g.nrows {
            for col in 0..g.ncols {
                let (lon, lat) = (g.lon_at(col as f64), g.lat_at(row as f64));
                if lon >= area.min_lon && lon <= area.max_lon && lat >= area.min_lat && lat <= area.max_lat {
                    g.data[row * g.ncols + col] = elev;
                }
            }
        }
    }

    pub fn params(bounds: Bounds, cell_nm: f64, safety_m: f64, clearance_nm: f64) -> MaskParams {
        MaskParams { bounds, cell_nm, safety_m, clearance_nm, band_nm: 0.0, max_cells: 10_000_000 }
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;

    fn area() -> Bounds {
        Bounds { min_lon: -5.0, min_lat: 48.0, max_lon: -4.0, max_lat: 48.5 }
    }

    fn island() -> Polygon {
        Polygon { rings: vec![vec![[-4.6, 48.2], [-4.4, 48.2], [-4.4, 48.3], [-4.6, 48.3], [-4.6, 48.2]]] }
    }

    #[test]
    fn land_is_rasterised_and_the_clearance_grows_it() {
        let m = NavMask::build(&params(area(), 0.1, 5.0, 0.5), &[island()], &[]).unwrap();
        let on = m.cell_at(&LatLon::new(48.25, -4.5)).unwrap();
        assert!(m.is_hazard(on.0, on.1));
        // 0.1 degree of longitude at 48 N is about 4 nm: far outside the clearance.
        let far = m.cell_at(&LatLon::new(48.25, -4.9)).unwrap();
        assert!(m.navigable(far.0, far.1));
        // Half a nautical mile off the shore is inside the 0.5 nm clearance.
        let shore_lat = 48.3 + 0.2 / 60.0;
        let near = m.cell_at(&LatLon::new(shore_lat, -4.5)).unwrap();
        assert!(!m.is_hazard(near.0, near.1));
        assert!(!m.navigable(near.0, near.1));
    }

    #[test]
    fn the_distance_transform_is_euclidean() {
        let (cols, rows) = (9, 9);
        let mut h = vec![false; cols * rows];
        h[4 * cols + 4] = true;
        let d = distance_to_hazard(&h, cols, rows);
        assert_eq!(d[4 * cols + 4], 0.0);
        assert!((d[4 * cols + 7] - 3.0).abs() < 1e-6);
        assert!((d[7 * cols + 7] - 18f32.sqrt()).abs() < 1e-5, "{}", d[7 * cols + 7]);
        // With no hazard the distance is huge.
        assert!(distance_to_hazard(&[false; 9], 3, 3)[4] > 1e6);
    }

    #[test]
    fn water_shallower_than_the_safety_depth_is_blocked() {
        let mut g = testutil::grid(area(), 0.01, -30.0);
        fill(&mut g, Bounds { min_lon: -4.6, min_lat: 48.2, max_lon: -4.4, max_lat: 48.3 }, -3.0);
        let grids = [Arc::new(g)];
        let m = NavMask::build(&params(area(), 0.1, 5.0, 0.0), &[], &grids).unwrap();
        let shoal = m.cell_at(&LatLon::new(48.25, -4.5)).unwrap();
        assert!(m.is_hazard(shoal.0, shoal.1));
        assert_eq!(m.depth_at(shoal.0, shoal.1), Some(3.0));
        let deep = m.cell_at(&LatLon::new(48.4, -4.9)).unwrap();
        assert!(!m.is_hazard(deep.0, deep.1));
        // A 3 m safety depth lets the same shoal through.
        let m = NavMask::build(&params(area(), 0.1, 3.0, 0.0), &[], &grids).unwrap();
        let shoal = m.cell_at(&LatLon::new(48.25, -4.5)).unwrap();
        assert!(!m.is_hazard(shoal.0, shoal.1));
    }

    #[test]
    fn the_finest_grid_wins_and_gaps_stay_unknown() {
        let coarse = testutil::grid(area(), 0.05, -2.0);
        let patch = Bounds { min_lon: -4.6, min_lat: 48.2, max_lon: -4.4, max_lat: 48.3 };
        let fine = testutil::grid(patch, 0.005, -40.0);
        let m = NavMask::build(&params(area(), 0.05, 5.0, 0.0), &[], &[Arc::new(coarse), Arc::new(fine)]).unwrap();
        let inside = m.cell_at(&LatLon::new(48.25, -4.5)).unwrap();
        assert!(!m.is_hazard(inside.0, inside.1), "40 m from the fine grid beats 2 m from the coarse one");
        let outside = m.cell_at(&LatLon::new(48.4, -4.9)).unwrap();
        assert!(m.is_hazard(outside.0, outside.1));
        // Without any grid the depth is unknown, not blocked.
        let none = NavMask::build(&params(area(), 0.05, 5.0, 0.0), &[], &[]).unwrap();
        let c = none.cell_at(&LatLon::new(48.4, -4.9)).unwrap();
        assert!(none.depth_at(c.0, c.1).is_none() && none.navigable(c.0, c.1));
        assert!(none.cost_factor(c.0, c.1) > 1.5, "a guess costs more");
    }

    #[test]
    fn legs_are_judged_cell_by_cell() {
        let m = NavMask::build(&params(area(), 0.05, 5.0, 0.2), &[island()], &[]).unwrap();
        // Straight across the island.
        assert!(m.blocked(&LatLon::new(48.25, -4.9), &LatLon::new(48.25, -4.1)));
        // Well to the south of it.
        assert!(!m.blocked(&LatLon::new(48.05, -4.9), &LatLon::new(48.05, -4.1)));
        let (a, b) = (m.to_grid(&LatLon::new(48.05, -4.9)), m.to_grid(&LatLon::new(48.05, -4.1)));
        assert!(m.segment_ok(a, b));
        // Leaving the mask is not ok for the router, and not judged by `Land`.
        assert!(!m.segment_ok(a, (-1.0, -1.0)));
        assert!(!m.blocked(&LatLon::new(48.05, -6.0), &LatLon::new(48.05, -5.5)));
    }

    #[test]
    fn grid_coordinates_round_trip() {
        let m = NavMask::build(&params(area(), 0.1, 5.0, 0.2), &[], &[]).unwrap();
        let p = LatLon::new(48.123, -4.456);
        let (u, v) = m.to_grid(&p);
        let q = m.to_latlon(u, v);
        assert!((p.lat - q.lat).abs() < 1e-9 && (p.lon - q.lon).abs() < 1e-9);
    }

    #[test]
    fn cells_are_the_same_size_on_the_ground() {
        let m = NavMask::build(&params(area(), 0.1, 5.0, 0.2), &[], &[]).unwrap();
        // Ground length of a cell edge: 0.1 nm in the middle of the box, a little
        // more to the south and less to the north.
        assert!((m.row_nm(m.rows() / 2) - 0.1).abs() < 0.002);
        assert!(m.row_nm(0) > m.row_nm(m.rows() - 1));
    }

    #[test]
    fn a_huge_area_is_refused() {
        let mut p = params(area(), 0.001, 5.0, 0.2);
        p.max_cells = 1_000_000;
        assert!(matches!(NavMask::build(&p, &[], &[]), Err(MaskError::TooLarge { .. })));
    }
}
