//! Regular lon/lat elevation grids (bathymetry): ESRI ASCII grids as published
//! by SHOM and GeoTIFFs as served by EMODnet. Values are metres, negative below
//! the datum; missing data is NaN.

use crate::vector::Bounds;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug)]
pub struct GridError(pub String);

impl std::fmt::Display for GridError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for GridError {}

impl From<std::io::Error> for GridError {
    fn from(e: std::io::Error) -> Self {
        GridError(e.to_string())
    }
}

#[derive(Clone, Debug)]
pub struct Grid {
    pub ncols: usize,
    pub nrows: usize,
    /// Lower-left corner of the lower-left cell, degrees.
    pub xll: f64,
    pub yll: f64,
    pub dx: f64,
    pub dy: f64,
    /// Row-major, first row is the northernmost.
    pub data: Vec<f32>,
}

/// Anything at or above this is treated as "no data" (SHOM uses 1.7e38).
const NODATA_LIMIT: f32 = 1.0e30;

/// A spot depth picked from a grid, in metres below the datum.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub lon: f64,
    pub lat: f64,
    pub depth: f32,
}

/// Samples scanned along each edge of a lattice cell at most.
const SPOT_SCAN: i64 = 12;

impl Grid {
    pub fn bounds(&self) -> Bounds {
        Bounds {
            min_lon: self.xll,
            min_lat: self.yll,
            max_lon: self.xll + self.ncols as f64 * self.dx,
            max_lat: self.yll + self.nrows as f64 * self.dy,
        }
    }

    /// The part of the grid that covers `b`, with a cell to spare each side so
    /// sampling inside `b` gives what the whole grid would. `b`'s longitudes
    /// are taken in the grid's own convention (-180..180 or 0..360). `None`
    /// when `b` is outside the grid.
    pub fn clipped(&self, b: &Bounds) -> Option<Grid> {
        // Shifted by whole turns to where it overlaps the grid most: a box starting west of the grid
        // must not be taken a turn east, past its other edge.
        let (x0, x1) = (self.xll, self.xll + (self.ncols as f64 - 1.0) * self.dx);
        let overlap = |lo: f64| (b.max_lon - b.min_lon + lo).min(x1) - lo.max(x0);
        let base = b.min_lon + 360.0 * ((x0 - b.min_lon) / 360.0).round();
        let lo = [base - 360.0, base, base + 360.0].into_iter().max_by(|a, c| overlap(*a).total_cmp(&overlap(*c))).unwrap_or(base);
        let hi = lo + (b.max_lon - b.min_lon);
        let n = self.nrows as f64;
        let c0 = (((lo - self.xll) / self.dx).floor() - 1.0).max(0.0);
        let c1 = (((hi - self.xll) / self.dx).ceil() + 1.0).min(self.ncols as f64 - 1.0);
        let r0 = ((n - (b.max_lat - self.yll) / self.dy).floor() - 1.0).max(0.0);
        let r1 = ((n - (b.min_lat - self.yll) / self.dy).ceil() + 1.0).min(n - 1.0);
        if c0 > c1 || r0 > r1 {
            return None;
        }
        let (c0, c1, r0, r1) = (c0 as usize, c1 as usize, r0 as usize, r1 as usize);
        let mut data = Vec::with_capacity((c1 - c0 + 1) * (r1 - r0 + 1));
        for r in r0..=r1 {
            data.extend_from_slice(&self.data[r * self.ncols + c0..=r * self.ncols + c1]);
        }
        Some(Grid {
            ncols: c1 - c0 + 1,
            nrows: r1 - r0 + 1,
            xll: self.xll + c0 as f64 * self.dx,
            yll: self.yll + (self.nrows - 1 - r1) as f64 * self.dy,
            dx: self.dx,
            dy: self.dy,
            data,
        })
    }

    pub fn lon_at(&self, col: f64) -> f64 {
        self.xll + (col + 0.5) * self.dx
    }

    pub fn lat_at(&self, row: f64) -> f64 {
        self.yll + (self.nrows as f64 - row - 0.5) * self.dy
    }

    pub fn at(&self, col: usize, row: usize) -> f32 {
        self.data[row * self.ncols + col]
    }

    /// Bilinear sample; `None` outside the grid or next to missing data.
    pub fn sample(&self, lon: f64, lat: f64) -> Option<f32> {
        let fc = (lon - self.xll) / self.dx - 0.5;
        let fr = self.nrows as f64 - (lat - self.yll) / self.dy - 0.5;
        if fc < 0.0 || fr < 0.0 || fc > (self.ncols - 1) as f64 || fr > (self.nrows - 1) as f64 {
            return None;
        }
        let (c0, r0) = (fc.floor() as usize, fr.floor() as usize);
        let (c1, r1) = ((c0 + 1).min(self.ncols - 1), (r0 + 1).min(self.nrows - 1));
        let (tx, ty) = ((fc - c0 as f64) as f32, (fr - r0 as f64) as f32);
        let (a, b, c, d) = (self.at(c0, r0), self.at(c1, r0), self.at(c0, r1), self.at(c1, r1));
        if a.is_nan() || b.is_nan() || c.is_nan() || d.is_nan() {
            return None;
        }
        let top = a + (b - a) * tx;
        let bot = c + (d - c) * tx;
        Some(top + (bot - top) * ty)
    }

    /// Bilinear sample over the corners that have data, their weights renormalised: `None` outside the
    /// grid or when all four corners are missing. For currents, which are missing over land: `sample`
    /// would leave the whole last cell before the coast without a current, where it matters most.
    pub fn sample_valid(&self, lon: f64, lat: f64) -> Option<f32> {
        let fc = (lon - self.xll) / self.dx - 0.5;
        let fr = self.nrows as f64 - (lat - self.yll) / self.dy - 0.5;
        if fc < -0.5 || fr < -0.5 || fc > self.ncols as f64 - 0.5 || fr > self.nrows as f64 - 0.5 {
            return None;
        }
        let (fc, fr) = (fc.clamp(0.0, (self.ncols - 1) as f64), fr.clamp(0.0, (self.nrows - 1) as f64));
        let (c0, r0) = (fc.floor() as usize, fr.floor() as usize);
        let (c1, r1) = ((c0 + 1).min(self.ncols - 1), (r0 + 1).min(self.nrows - 1));
        let (tx, ty) = (fc - c0 as f64, fr - r0 as f64);
        let (mut sum, mut weight) = (0.0f64, 0.0f64);
        for (c, r, w) in [(c0, r0, (1.0 - tx) * (1.0 - ty)), (c1, r0, tx * (1.0 - ty)), (c0, r1, (1.0 - tx) * ty), (c1, r1, tx * ty)] {
            let v = self.at(c, r);
            if !v.is_nan() && v.abs() < NODATA_LIMIT && w > 0.0 {
                sum += v as f64 * w;
                weight += w;
            }
        }
        (weight > 1e-9).then(|| (sum / weight) as f32)
    }

    /// Spot soundings for a chart view: the shallowest sample below the datum
    /// in each cell of a lattice of `cell_lon` by `cell_lat` degrees (anchored
    /// at 0, 0 so the picks stay put while panning), the way a chart maker
    /// picks soundings. Samples shoaler than `min_depth` metres are left out:
    /// a grid cannot tell drying ground from land, and the shore would fill
    /// with zeros. Cells are scanned with a stride so a wide view stays cheap.
    pub fn spot_depths(&self, view: &Bounds, cell_lon: f64, cell_lat: f64, min_depth: f32) -> Vec<Spot> {
        let b = self.bounds();
        if cell_lon <= 0.0 || cell_lat <= 0.0 || !b.intersects(view) {
            return Vec::new();
        }
        let (lo_x, hi_x) = (view.min_lon.max(b.min_lon), view.max_lon.min(b.max_lon));
        let (lo_y, hi_y) = (view.min_lat.max(b.min_lat), view.max_lat.min(b.max_lat));
        let mut out = Vec::new();
        for iy in (lo_y / cell_lat).floor() as i64..=(hi_y / cell_lat).floor() as i64 {
            let (ly0, ly1) = (iy as f64 * cell_lat, (iy + 1) as f64 * cell_lat);
            // Rows whose centre latitude is in [ly0, ly1).
            let r0 = ((self.nrows as f64 - 0.5 - (ly1 - self.yll) / self.dy).floor() as i64 + 1).max(0);
            let r1 = ((self.nrows as f64 - 0.5 - (ly0 - self.yll) / self.dy).floor() as i64).min(self.nrows as i64 - 1);
            let rstep = ((r1 - r0 + 1) / SPOT_SCAN).max(1) as usize;
            for ix in (lo_x / cell_lon).floor() as i64..=(hi_x / cell_lon).floor() as i64 {
                let (lx0, lx1) = (ix as f64 * cell_lon, (ix + 1) as f64 * cell_lon);
                // Columns whose centre longitude is in [lx0, lx1).
                let c0 = (((lx0 - self.xll) / self.dx - 0.5).ceil() as i64).max(0);
                let c1 = (((lx1 - self.xll) / self.dx - 0.5).ceil() as i64).min(self.ncols as i64);
                let cstep = ((c1 - c0) / SPOT_SCAN).max(1) as usize;
                let mut best: Option<(usize, usize, f32)> = None;
                for r in (r0.max(0)..=r1).step_by(rstep) {
                    for c in (c0..c1).step_by(cstep) {
                        let e = self.at(c as usize, r as usize);
                        // NaN fails the comparison; elevation above -min_depth is too shoal or land.
                        if e <= -min_depth && best.is_none_or(|(_, _, b)| e > b) {
                            best = Some((c as usize, r as usize, e));
                        }
                    }
                }
                if let Some((c, r, e)) = best {
                    let (lon, lat) = (self.lon_at(c as f64), self.lat_at(r as f64));
                    if view.contains([lon, lat]) {
                        out.push(Spot { lon, lat, depth: -e });
                    }
                }
            }
        }
        out
    }

    pub fn from_asc_file(path: &Path) -> Result<Grid, GridError> {
        Grid::parse_asc(&std::fs::read_to_string(path)?)
    }

    pub fn parse_asc(text: &str) -> Result<Grid, GridError> {
        let mut hdr: HashMap<String, f64> = HashMap::new();
        let mut rest = text;
        // Header lines are `KEY value`; stop at the first line that starts with a number.
        loop {
            let (line, tail) = rest.split_once('\n').unwrap_or((rest, ""));
            let mut it = line.split_whitespace();
            let (Some(k), Some(v)) = (it.next(), it.next()) else { break };
            if k.parse::<f64>().is_ok() {
                break;
            }
            hdr.insert(
                k.to_ascii_lowercase(),
                v.parse().map_err(|_| GridError(format!("bad header value for {k}")))?,
            );
            rest = tail;
        }
        let get = |k: &str| hdr.get(k).copied();
        let ncols = get("ncols").ok_or_else(|| GridError("missing NCOLS".into()))? as usize;
        let nrows = get("nrows").ok_or_else(|| GridError("missing NROWS".into()))? as usize;
        let cell = get("cellsize").ok_or_else(|| GridError("missing CELLSIZE".into()))?;
        let (xll, yll) = match (get("xllcorner"), get("yllcorner"), get("xllcenter"), get("yllcenter")) {
            (Some(x), Some(y), _, _) => (x, y),
            (_, _, Some(x), Some(y)) => (x - cell / 2.0, y - cell / 2.0),
            _ => return Err(GridError("missing lower-left corner".into())),
        };
        let nodata = get("nodata_value");
        if ncols == 0 || nrows == 0 || ncols.checked_mul(nrows).is_none_or(|n| n > 1 << 30) {
            return Err(GridError("implausible grid size".into()));
        }
        let mut data = Vec::with_capacity(ncols * nrows);
        for tok in rest.split_ascii_whitespace() {
            let v: f64 = tok.parse().map_err(|_| GridError(format!("bad value {tok:?}")))?;
            let missing = nodata.is_some_and(|n| v == n) || v.abs() >= NODATA_LIMIT as f64;
            data.push(if missing { f32::NAN } else { v as f32 });
        }
        if data.len() != ncols * nrows {
            return Err(GridError(format!("expected {} values, found {}", ncols * nrows, data.len())));
        }
        Ok(Grid { ncols, nrows, xll, yll, dx: cell, dy: cell, data })
    }

    /// Reads a single-band float GeoTIFF with ModelPixelScale/ModelTiepoint tags.
    pub fn from_geotiff(path: &Path) -> Result<Grid, GridError> {
        use tiff::decoder::{Decoder, DecodingResult};
        use tiff::tags::Tag;
        let f = std::fs::File::open(path)?;
        let mut d = Decoder::new(std::io::BufReader::new(f)).map_err(|e| GridError(e.to_string()))?;
        let (w, h) = d.dimensions().map_err(|e| GridError(e.to_string()))?;
        // Georeferencing is either ModelPixelScale + ModelTiepoint, or a
        // ModelTransformation matrix (as served by EMODnet's WCS).
        let (x_left, y_top, dx, dy) = if let Ok(m) = d.get_tag_f64_vec(Tag::Unknown(34264)) {
            if m.len() < 16 || m[1] != 0.0 || m[4] != 0.0 {
                return Err(GridError("rotated GeoTIFFs are not supported".into()));
            }
            (m[3], m[7], m[0], -m[5])
        } else {
            let scale = d
                .get_tag_f64_vec(Tag::Unknown(33550))
                .map_err(|_| GridError("not a GeoTIFF: no georeferencing tags".into()))?;
            let tie = d
                .get_tag_f64_vec(Tag::Unknown(33922))
                .map_err(|_| GridError("not a GeoTIFF: no ModelTiepoint".into()))?;
            if scale.len() < 2 || tie.len() < 6 {
                return Err(GridError("unsupported GeoTIFF georeferencing".into()));
            }
            // Tiepoint (i, j, k, x, y, z) pins pixel (i, j) to (x, y); assume pixel-is-area.
            (tie[3] - tie[0] * scale[0], tie[4] + tie[1] * scale[1], scale[0], scale[1])
        };
        let nodata = d
            .get_tag_ascii_string(Tag::Unknown(42113))
            .ok()
            .and_then(|s| s.trim_end_matches('\0').trim().parse::<f64>().ok());
        let raw = d.read_image().map_err(|e| GridError(e.to_string()))?;
        let mut data: Vec<f32> = match raw {
            DecodingResult::F32(v) => v,
            DecodingResult::F64(v) => v.into_iter().map(|x| x as f32).collect(),
            DecodingResult::I16(v) => v.into_iter().map(|x| x as f32).collect(),
            DecodingResult::I32(v) => v.into_iter().map(|x| x as f32).collect(),
            _ => return Err(GridError("unsupported GeoTIFF sample type".into())),
        };
        for v in data.iter_mut() {
            if nodata.is_some_and(|n| *v as f64 == n) || v.abs() >= NODATA_LIMIT {
                *v = f32::NAN;
            }
        }
        if data.len() != (w * h) as usize {
            return Err(GridError("multi-band GeoTIFFs are not supported".into()));
        }
        Ok(Grid {
            ncols: w as usize,
            nrows: h as usize,
            xll: x_left,
            yll: y_top - h as f64 * dy,
            dx,
            dy,
            data,
        })
    }

    /// Contour segments `[lon1, lat1, lon2, lat2]` at `level`, by marching squares.
    pub fn contour(&self, level: f32) -> Vec<[f32; 4]> {
        marching_squares(self.ncols, self.nrows, |c, r| self.at(c, r), level, |c, r| [self.lon_at(c), self.lat_at(r)])
    }
}

/// Contour segments `[x1, y1, x2, y2]` at `level` over a lattice of `ncols` by
/// `nrows` samples, by marching squares. `at(col, row)` reads a sample (NaN is
/// none: a cell with a corner of it is skipped) and `place(col, row)` maps a
/// position of the lattice, in fractional columns and rows, to the
/// coordinates of the segments.
pub fn marching_squares(ncols: usize, nrows: usize, at: impl Fn(usize, usize) -> f32, level: f32, place: impl Fn(f64, f64) -> [f64; 2]) -> Vec<[f32; 4]> {
    let mut out = Vec::new();
    for r in 0..nrows.saturating_sub(1) {
        for c in 0..ncols.saturating_sub(1) {
            let (tl, tr, br, bl) = (at(c, r), at(c + 1, r), at(c + 1, r + 1), at(c, r + 1));
            if tl.is_nan() || tr.is_nan() || br.is_nan() || bl.is_nan() {
                continue;
            }
            let case = (tl >= level) as u8 | ((tr >= level) as u8) << 1 | ((br >= level) as u8) << 2 | ((bl >= level) as u8) << 3;
            if case == 0 || case == 15 {
                continue;
            }
            let (cf, rf) = (c as f64, r as f64);
            let lerp = |a: f32, b: f32| ((level - a) / (b - a)) as f64;
            // Crossing points on the four edges.
            let top = || place(cf + lerp(tl, tr), rf);
            let right = || place(cf + 1.0, rf + lerp(tr, br));
            let bottom = || place(cf + lerp(bl, br), rf + 1.0);
            let left = || place(cf, rf + lerp(tl, bl));
            let mut seg = |a: [f64; 2], b: [f64; 2]| out.push([a[0] as f32, a[1] as f32, b[0] as f32, b[1] as f32]);
            match case {
                1 | 14 => seg(left(), top()),
                2 | 13 => seg(top(), right()),
                3 | 12 => seg(left(), right()),
                4 | 11 => seg(right(), bottom()),
                6 | 9 => seg(top(), bottom()),
                7 | 8 => seg(left(), bottom()),
                5 | 10 => {
                    // Saddle: decide by the cell centre.
                    let centre_high = (tl + tr + br + bl) / 4.0 >= level;
                    if (case == 5) == centre_high {
                        seg(left(), bottom());
                        seg(top(), right());
                    } else {
                        seg(left(), top());
                        seg(right(), bottom());
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// Joins unordered contour segments into polylines, each as `[lon, lat]`
/// points. Closed contours come back with their first point repeated.
pub fn join_segments(segs: &[[f32; 4]]) -> Vec<Vec<[f32; 2]>> {
    use std::collections::HashMap;
    // Neighbouring cells compute the same crossing point separately and may
    // differ in the last bit, so matches are found by distance, probing the
    // surrounding buckets.
    const Q: f64 = 1e4;
    const EPS2: f64 = 1e-5 * 1e-5;
    let cell = |x: f32, y: f32| ((x as f64 * Q).floor() as i64, (y as f64 * Q).floor() as i64);
    let mut ends: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, s) in segs.iter().enumerate() {
        ends.entry(cell(s[0], s[1])).or_default().push(i);
        ends.entry(cell(s[2], s[3])).or_default().push(i);
    }
    let near = |a: [f32; 2], x: f32, y: f32| {
        let (dx, dy) = ((a[0] - x) as f64, (a[1] - y) as f64);
        dx * dx + dy * dy <= EPS2
    };
    // A grid point exactly on the level yields a zero-length segment; ignore it.
    let mut used: Vec<bool> = segs.iter().map(|s| near([s[0], s[1]], s[2], s[3])).collect();
    let mut out = Vec::new();
    // Grows a chain through unused segments touching its last point.
    let extend = |chain: &mut Vec<[f32; 2]>, used: &mut Vec<bool>| loop {
        let last = *chain.last().unwrap();
        let (cx, cy) = cell(last[0], last[1]);
        let mut found = None;
        'probe: for dx in -1..=1 {
            for dy in -1..=1 {
                for &i in ends.get(&(cx + dx, cy + dy)).into_iter().flatten() {
                    if used[i] {
                        continue;
                    }
                    let s = segs[i];
                    if near(last, s[0], s[1]) {
                        found = Some((i, [s[2], s[3]]));
                        break 'probe;
                    }
                    if near(last, s[2], s[3]) {
                        found = Some((i, [s[0], s[1]]));
                        break 'probe;
                    }
                }
            }
        }
        let Some((i, next)) = found else { break };
        used[i] = true;
        chain.push(next);
    };
    for i in 0..segs.len() {
        if used[i] {
            continue;
        }
        used[i] = true;
        let s = segs[i];
        let mut chain = vec![[s[0], s[1]], [s[2], s[3]]];
        extend(&mut chain, &mut used);
        chain.reverse();
        extend(&mut chain, &mut used);
        out.push(chain);
    }
    out
}

/// The contour at one level, joined into lines, kept at several levels of
/// detail and cut into short pieces bucketed into tiles, so a view only
/// touches the pieces it can see, with no more points than it can show.
pub struct ContourSet {
    pub level: f32,
    xll: f64,
    yll: f64,
    tile_dx: f64,
    tile_dy: f64,
    ntx: usize,
    nty: usize,
    /// Finest first: the lines as the grid gives them, then simplified to
    /// `dx * 2^(i - 3)` degrees: a quarter of a cell, half, one, two, and so on.
    details: Vec<Detail>,
    /// Segments the grid gave.
    pub count: usize,
}

struct Detail {
    /// Tolerance of the simplification, in degrees.
    tol: f64,
    tiles: Vec<Vec<Piece>>,
}

/// A stretch of one contour line, `[lon, lat]`.
pub struct Piece {
    pub points: Vec<[f32; 2]>,
    pub bounds: Bounds,
}

/// Cells per tile edge.
const TILE_CELLS: f64 = 64.0;
/// Points per piece at most, so a piece stays about a tile long.
const PIECE_POINTS: usize = 64;
/// Levels of detail past the full one.
const COARSER_DETAILS: i32 = 8;

impl ContourSet {
    pub fn build(grid: &Grid, level: f32) -> ContourSet {
        let (tile_dx, tile_dy) = (grid.dx * TILE_CELLS, grid.dy * TILE_CELLS);
        let ntx = (grid.ncols as f64 / TILE_CELLS).ceil() as usize + 1;
        let nty = (grid.nrows as f64 / TILE_CELLS).ceil() as usize + 1;
        let segs = grid.contour(level);
        let count = segs.len();
        let lines: Vec<Vec<[f64; 2]>> = join_segments(&segs).into_iter().map(|l| l.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
        let mut set = ContourSet { level, xll: grid.xll, yll: grid.yll, tile_dx, tile_dy, ntx, nty, details: Vec::new(), count };
        for i in 0..=COARSER_DETAILS {
            let tol = if i == 0 { 0.0 } else { grid.dx.min(grid.dy) * 2f64.powi(i - 3) };
            let mut tiles: Vec<Vec<Piece>> = (0..ntx * nty).map(|_| Vec::new()).collect();
            for line in &lines {
                let kept = if i == 0 { line.clone() } else { crate::vector::simplify(line, tol) };
                // Neighbouring pieces share their end point, so the line stays whole.
                let mut start = 0;
                while start + 1 < kept.len() {
                    let end = (start + PIECE_POINTS).min(kept.len() - 1);
                    let run = &kept[start..=end];
                    let piece = Piece { points: run.iter().map(|p| [p[0] as f32, p[1] as f32]).collect(), bounds: Bounds::of(run) };
                    let t = set.tile_of(run[0][0], run[0][1]);
                    tiles[t].push(piece);
                    start = end;
                }
            }
            set.details.push(Detail { tol, tiles });
        }
        set
    }

    fn tile_of(&self, lon: f64, lat: f64) -> usize {
        let tx = (((lon - self.xll) / self.tile_dx).floor().max(0.0) as usize).min(self.ntx - 1);
        let ty = (((lat - self.yll) / self.tile_dy).floor().max(0.0) as usize).min(self.nty - 1);
        ty * self.ntx + tx
    }

    /// The pieces that reach into `b`, from the coarsest detail whose
    /// simplification stays within `tol` degrees.
    pub fn pieces_in<'a>(&'a self, b: &Bounds, tol: f64) -> impl Iterator<Item = &'a Piece> + 'a {
        let detail = self.details.iter().rev().find(|d| d.tol <= tol).unwrap_or(&self.details[0]);
        // A piece is filed under the tile of its first point and is at most
        // about a tile long, so the tiles one round `b` are looked at too.
        let lo_x = (((b.min_lon - self.xll) / self.tile_dx).floor() - 1.0).max(0.0) as usize;
        let hi_x = ((((b.max_lon - self.xll) / self.tile_dx).floor() + 2.0).max(0.0) as usize).min(self.ntx);
        let lo_y = (((b.min_lat - self.yll) / self.tile_dy).floor() - 1.0).max(0.0) as usize;
        let hi_y = ((((b.max_lat - self.yll) / self.tile_dy).floor() + 2.0).max(0.0) as usize).min(self.nty);
        let (ntx, b) = (self.ntx, *b);
        (lo_y..hi_y).flat_map(move |ty| (lo_x..hi_x).flat_map(move |tx| detail.tiles[ty * ntx + tx].iter())).filter(move |p| p.bounds.intersects(&b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_coast_keeps_the_current_of_the_sea_beside_it() {
        // Two by two cells: the west column is land.
        let g = Grid { ncols: 2, nrows: 2, xll: 0.0, yll: 0.0, dx: 1.0, dy: 1.0, data: vec![f32::NAN, 2.0, f32::NAN, 4.0] };
        assert_eq!(g.sample(1.0, 1.0), None, "the plain sample gives up next to land");
        assert_eq!(g.sample_valid(1.0, 1.0), Some(3.0), "the sea corners only");
        assert_eq!(g.sample_valid(1.5, 1.5), Some(2.0), "at the north-east cell's centre");
        assert_eq!(g.sample_valid(1.9, 1.5), Some(2.0), "in the half-cell past the last centre");
        assert_eq!(g.sample_valid(0.6, 1.5), Some(2.0), "the land corner weighs nothing");
        assert_eq!(g.sample_valid(0.4, 1.5), None, "in the outer half of a land cell");
        assert_eq!(g.sample_valid(2.6, 1.0), None, "outside");
        let land = Grid { data: vec![f32::NAN; 4], ..g.clone() };
        assert_eq!(land.sample_valid(1.0, 1.0), None);
        let sea = Grid { data: vec![1.0, 2.0, 3.0, 4.0], ..g };
        assert_eq!(sea.sample_valid(1.0, 1.0), sea.sample(1.0, 1.0), "the same as the plain sample where all is known");
    }

    #[test]
    fn a_box_reaching_past_the_west_of_a_grid_still_cuts_it() {
        // Quarter degrees from 13 W to 5 E, 38 N to 56 N.
        let g = Grid { ncols: 73, nrows: 73, xll: -13.0, yll: 38.0, dx: 0.25, dy: 0.25, data: vec![1.0; 73 * 73] };
        let b = |w: f64, e: f64| Bounds { min_lon: w, min_lat: 40.0, max_lon: e, max_lat: 50.0 };
        let c = g.clipped(&b(-15.0, 0.0)).expect("a box over the western edge");
        assert_eq!(c.xll, -13.0);
        assert!((c.xll + (c.ncols - 1) as f64 * c.dx) >= 0.0);
        assert!(g.clipped(&b(-4.0, 10.0)).is_some(), "over the eastern edge");
        assert!(g.clipped(&b(20.0, 30.0)).is_none(), "beside it");
        // A grid in 0..360 cut by a box written west of Greenwich.
        let h = Grid { ncols: 1440, nrows: 73, xll: 0.0, yll: 38.0, dx: 0.25, dy: 0.25, data: vec![1.0; 1440 * 73] };
        let c = h.clipped(&b(-10.0, -5.0)).unwrap();
        assert!((c.xll - 349.75).abs() < 1e-9, "{}", c.xll);
    }

    const ASC: &str = "NCOLS 3\nNROWS 2\nXLLCORNER 10.0\nYLLCORNER 40.0\nCELLSIZE 0.5\nNODATA_VALUE -9999\n\
        -1 -2 -3\n-4 -9999 -6\n";

    #[test]
    fn parses_asc_and_places_cells() {
        let g = Grid::parse_asc(ASC).unwrap();
        assert_eq!((g.ncols, g.nrows), (3, 2));
        assert_eq!(g.at(0, 0), -1.0);
        assert!(g.at(1, 1).is_nan());
        // Row 0 is the north row: its centre is 0.25 below the top edge at 41.0.
        assert!((g.lat_at(0.0) - 40.75).abs() < 1e-9);
        assert!((g.lon_at(0.0) - 10.25).abs() < 1e-9);
        let b = g.bounds();
        assert_eq!((b.min_lon, b.max_lat), (10.0, 41.0));
    }

    #[test]
    fn huge_nodata_values_become_nan() {
        let g = Grid::parse_asc("ncols 2\nnrows 1\nxllcorner 0\nyllcorner 0\ncellsize 1\n1.0 170141000918782798866653488190622531584.00\n").unwrap();
        assert!(g.at(1, 0).is_nan());
    }

    #[test]
    fn rejects_bad_files() {
        assert!(Grid::parse_asc("hello").is_err());
        assert!(Grid::parse_asc("ncols 2\nnrows 2\nxllcorner 0\nyllcorner 0\ncellsize 1\n1 2 3\n").is_err());
        assert!(Grid::parse_asc("ncols 2\nnrows 1\nxllcorner 0\nyllcorner 0\ncellsize 1\n1 x\n").is_err());
        assert!(Grid::parse_asc("ncols 0\nnrows 1\nxllcorner 0\nyllcorner 0\ncellsize 1\n").is_err());
    }

    #[test]
    fn centre_registration_is_shifted_by_half_a_cell() {
        let g = Grid::parse_asc("ncols 1\nnrows 1\nxllcenter 5\nyllcenter 5\ncellsize 2\n7\n").unwrap();
        assert_eq!((g.xll, g.yll), (4.0, 4.0));
    }

    #[test]
    fn bilinear_sampling() {
        let g = Grid::parse_asc("ncols 2\nnrows 2\nxllcorner 0\nyllcorner 0\ncellsize 1\n0 10\n20 30\n").unwrap();
        // Cell centres are at (0.5, 1.5) top-left ... (1.5, 0.5) bottom-right.
        assert_eq!(g.sample(0.5, 1.5), Some(0.0));
        assert_eq!(g.sample(1.5, 0.5), Some(30.0));
        let mid = g.sample(1.0, 1.0).unwrap();
        assert!((mid - 15.0).abs() < 1e-5);
        assert_eq!(g.sample(5.0, 5.0), None);
        let holey = Grid::parse_asc("ncols 2\nnrows 1\nxllcorner 0\nyllcorner 0\ncellsize 1\n1 -9999\nnodata_value -9999\n");
        // NODATA_VALUE after the data is not a header, so this must fail cleanly.
        assert!(holey.is_err());
    }

    #[test]
    fn contour_of_a_cone_is_a_circle() {
        // A bowl: depth = -(distance from centre), 101x101 cells of 0.01 degrees.
        let n = 101;
        let mut data = Vec::new();
        let mut g = Grid { ncols: n, nrows: n, xll: 0.0, yll: 0.0, dx: 0.01, dy: 0.01, data: vec![] };
        for r in 0..n {
            for c in 0..n {
                let (x, y) = (g.lon_at(c as f64) - 0.505, g.lat_at(r as f64) - 0.505);
                data.push(-((x * x + y * y).sqrt() as f32) * 100.0);
            }
        }
        g.data = data;
        let segs = g.contour(-20.0); // radius 0.2 degrees
        assert!(segs.len() > 100);
        for s in &segs {
            for (x, y) in [(s[0], s[1]), (s[2], s[3])] {
                let r = (((x as f64) - 0.505).powi(2) + ((y as f64) - 0.505).powi(2)).sqrt();
                assert!((r - 0.2).abs() < 0.002, "{r}");
            }
        }
        // Levels outside the data range give nothing.
        assert!(g.contour(5.0).is_empty());
        assert!(g.contour(-500.0).is_empty());
    }

    #[test]
    fn contour_set_returns_only_nearby_pieces() {
        // A 300x300 grid sloping west to east: one long vertical contour.
        let n = 300;
        let mut data = Vec::new();
        for _ in 0..n {
            for c in 0..n {
                data.push(c as f32);
            }
        }
        let g = Grid { ncols: n, nrows: n, xll: 0.0, yll: 0.0, dx: 0.01, dy: 0.01, data };
        let set = ContourSet::build(&g, 100.5);
        assert_eq!(set.count, n - 1);
        let points = |b: &Bounds, tol: f64| set.pieces_in(b, tol).map(|p| p.points.len() - 1).sum::<usize>();
        let all = Bounds { min_lon: -1.0, min_lat: -1.0, max_lon: 5.0, max_lat: 5.0 };
        assert_eq!(points(&all, 0.0), set.count, "every segment, once");
        // The contour is at lon ~ 1.005; a window far to the east sees nothing.
        let east = Bounds { min_lon: 2.5, min_lat: 0.0, max_lon: 2.9, max_lat: 3.0 };
        assert_eq!(points(&east, 0.0), 0);
        // A window round the line, covering the lower third of the grid, sees part of it.
        let part = Bounds { min_lon: 0.9, min_lat: 0.0, max_lon: 1.1, max_lat: 1.0 };
        let n_part = points(&part, 0.0);
        assert!(n_part >= 100 && n_part < set.count, "{n_part}");
        // Windows outside the grid are fine too.
        let outside = Bounds { min_lon: -9.0, min_lat: -9.0, max_lon: -8.0, max_lat: -8.0 };
        assert_eq!(points(&outside, 0.0), 0);
        // Far out, the straight line needs only its ends in each piece.
        assert!(points(&all, 1.0) < 10, "{}", points(&all, 1.0));
    }

    #[test]
    fn coarser_details_keep_to_their_tolerance() {
        // A bowl: the contour is a circle of 0.2 degrees round the centre.
        let n = 101;
        let mut g = Grid { ncols: n, nrows: n, xll: 0.0, yll: 0.0, dx: 0.01, dy: 0.01, data: vec![] };
        for r in 0..n {
            for c in 0..n {
                let (x, y) = (g.lon_at(c as f64) - 0.505, g.lat_at(r as f64) - 0.505);
                g.data.push(-((x * x + y * y).sqrt() as f32) * 100.0);
            }
        }
        let set = ContourSet::build(&g, -20.0);
        let all = g.bounds();
        let count = |tol: f64| set.pieces_in(&all, tol).map(|p| p.points.len()).sum::<usize>();
        assert!(count(0.04) < count(0.01) && count(0.01) < count(0.0), "{} {} {}", count(0.04), count(0.01), count(0.0));
        for p in set.pieces_in(&all, 0.04).flat_map(|p| &p.points) {
            let r = (((p[0] as f64) - 0.505).powi(2) + ((p[1] as f64) - 0.505).powi(2)).sqrt();
            assert!((r - 0.2).abs() < 0.002, "a kept point is on the line: {r}");
        }
    }

    #[test]
    fn segments_join_into_one_closed_ring_for_a_circle() {
        let n = 61;
        let mut g = Grid { ncols: n, nrows: n, xll: 0.0, yll: 0.0, dx: 0.01, dy: 0.01, data: vec![] };
        for r in 0..n {
            for c in 0..n {
                let (x, y) = (g.lon_at(c as f64) - 0.305, g.lat_at(r as f64) - 0.305);
                g.data.push(-((x * x + y * y).sqrt() as f32) * 100.0);
            }
        }
        let segs = g.contour(-15.0);
        let lines = join_segments(&segs);
        assert_eq!(lines.len(), 1, "one connected contour");
        let l = &lines[0];
        assert!(l.len() > 100);
        let (f, e) = (l[0], l[l.len() - 1]);
        assert!(((f[0] - e[0]).powi(2) + (f[1] - e[1]).powi(2)).sqrt() < 1e-4, "closed: {f:?} {e:?}");
    }

    #[test]
    fn open_contours_and_separate_pieces_stay_separate() {
        // Two disjoint segments, and a chain of three given out of order and reversed.
        let segs = [[0.0, 0.0, 1.0, 0.0], [5.0, 5.0, 6.0, 5.0], [2.0, 0.0, 3.0, 0.0], [2.0, 0.0, 1.0, 0.0]];
        let mut lines = join_segments(&segs);
        lines.sort_by_key(|l| l.len());
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].len(), 2);
        assert_eq!(lines[1].len(), 4);
        let xs: Vec<f32> = lines[1].iter().map(|p| p[0]).collect();
        assert!(xs == vec![0.0, 1.0, 2.0, 3.0] || xs == vec![3.0, 2.0, 1.0, 0.0], "{xs:?}");
        assert!(join_segments(&[]).is_empty());
    }

    #[test]
    fn contour_skips_cells_with_missing_data() {
        let g = Grid { ncols: 2, nrows: 2, xll: 0.0, yll: 0.0, dx: 1.0, dy: 1.0, data: vec![0.0, f32::NAN, 10.0, 10.0] };
        assert!(g.contour(5.0).is_empty());
    }

    #[test]
    fn spot_depths_pick_the_shallowest_sample_per_cell() {
        // 4x2 grid of 1 degree cells, first row northernmost; land and holes are left out.
        let g = Grid { ncols: 4, nrows: 2, xll: 0.0, yll: 0.0, dx: 1.0, dy: 1.0, data: vec![-9.0, -3.0, -7.0, 12.0, -20.0, f32::NAN, -0.2, -5.0] };
        let view = g.bounds();
        // One lattice cell per pair of columns, covering both rows.
        let mut spots = g.spot_depths(&view, 2.0, 2.0, 0.5);
        spots.sort_by(|a, b| a.lon.total_cmp(&b.lon));
        assert_eq!(spots, vec![Spot { lon: 1.5, lat: 1.5, depth: 3.0 }, Spot { lon: 3.5, lat: 0.5, depth: 5.0 }]);
        // A finer lattice gives every sample its own cell, minus the missing, the land and the too shoal.
        let all = g.spot_depths(&view, 1.0, 1.0, 0.5);
        assert_eq!(all.len(), 5);
        assert!(all.iter().all(|s| s.depth >= 0.5));
    }

    #[test]
    fn spot_depths_stay_inside_the_view_and_the_lattice_is_fixed() {
        let g = Grid { ncols: 10, nrows: 10, xll: 0.0, yll: 0.0, dx: 1.0, dy: 1.0, data: (0..100).map(|i| -(1.0 + (i % 7) as f32)).collect() };
        let view = Bounds { min_lon: 2.0, min_lat: 2.0, max_lon: 6.0, max_lat: 6.0 };
        let spots = g.spot_depths(&view, 2.0, 2.0, 0.5);
        assert!(!spots.is_empty() && spots.iter().all(|s| view.contains([s.lon, s.lat])));
        // Panning the view by a whole lattice cell reuses the same picks.
        let moved = Bounds { min_lon: 4.0, min_lat: 2.0, max_lon: 8.0, max_lat: 6.0 };
        let again = g.spot_depths(&moved, 2.0, 2.0, 0.5);
        assert!(again.iter().filter(|s| s.lon >= 4.0 && s.lon <= 6.0).all(|s| spots.contains(s)));
        assert!(g.spot_depths(&Bounds { min_lon: 50.0, min_lat: 50.0, max_lon: 60.0, max_lat: 60.0 }, 2.0, 2.0, 0.5).is_empty());
        assert!(g.spot_depths(&view, 0.0, 2.0, 0.5).is_empty());
    }

    #[test]
    fn saddle_cells_produce_two_segments() {
        let g = Grid { ncols: 2, nrows: 2, xll: 0.0, yll: 0.0, dx: 1.0, dy: 1.0, data: vec![10.0, 0.0, 0.0, 10.0] };
        assert_eq!(g.contour(5.0).len(), 2);
    }

    #[test]
    fn reads_the_real_emodnet_geotiff_when_present() {
        let p = Path::new("testdata/emodnet-brest/brest.tif");
        if !p.exists() {
            return;
        }
        let g = Grid::from_geotiff(p).unwrap();
        let b = g.bounds();
        assert!((b.min_lon + 5.0).abs() < 0.01 && (b.max_lon + 4.3).abs() < 0.01, "{b:?}");
        assert!((b.min_lat - 48.25).abs() < 0.01 && (b.max_lat - 48.5).abs() < 0.01, "{b:?}");
        // Out in the Iroise sea it is deep water, and EMODnet depths are negative.
        let d = g.sample(-4.95, 48.3).unwrap();
        assert!(d < -20.0, "{d}");
    }

    #[test]
    fn reads_the_real_shom_grid_when_present() {
        let p = Path::new("testdata/shom-dtm-saint-malo-5m/MNT_COTIER_PORT_SAINT-MALO_PAPI_PBMA/DONNEES/MNT_COTIER_PORT_SM_PAPI_SM_5m_WGS84_PBMA_ZNEG.asc");
        if !p.exists() {
            return;
        }
        let g = Grid::from_asc_file(p).unwrap();
        assert_eq!((g.ncols, g.nrows), (2601, 1601));
        let b = g.bounds();
        assert!((b.min_lon + 2.11).abs() < 0.001 && (b.min_lat - 48.61).abs() < 0.001);
        assert!(g.data.iter().any(|v| *v < -10.0));
    }
}
