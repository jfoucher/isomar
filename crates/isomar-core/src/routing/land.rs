//! A raster land mask for fast "does this leg cross land" tests.

use super::seaway::{self, SeaField, floor_i64};
use crate::geo::LatLon;
use crate::vector::{Bounds, Polygon};
use std::sync::{Arc, Mutex, OnceLock};

/// What the router asks about land.
pub trait Land: Sync {
    /// True when the straight leg from `a` to `b` touches land.
    fn blocked(&self, a: &LatLon, b: &LatLon) -> bool;

    /// The way from `p` out to water with about `clear_nm` of it all round,
    /// where legs of the router's length have room: `p` first, the open
    /// water last. `p` may be on land (a quay, or a river the mask fills) if
    /// open water is within `snap_nm`; `None` when it is not.
    fn approach(&self, p: LatLon, _clear_nm: f64, snap_nm: f64) -> Option<Vec<LatLon>> {
        nearest_open(self, p, snap_nm).map(|q| if q == p { vec![p] } else { vec![p, q] })
    }

    /// The distance by water to `goal` across this mask, where it can tell.
    fn seaway(&self, _goal: LatLon) -> Option<Arc<SeaField>> {
        None
    }
}

/// `p` when it is open water, otherwise the nearest point within `max_nm`
/// that is: rings 20 m apart, with a point every 20 m or so along each.
fn nearest_open<L: Land + ?Sized>(land: &L, p: LatLon, max_nm: f64) -> Option<LatLon> {
    if !land.blocked(&p, &p) {
        return Some(p);
    }
    let step = 0.01;
    (1..=(max_nm / step).ceil() as usize).find_map(|i| {
        let r = i as f64 * step;
        let n = ((2.0 * std::f64::consts::PI * r / step).ceil() as usize).clamp(8, 720);
        (0..n).map(|k| p.destination(k as f64 * 360.0 / n as f64, r)).find(|q| !land.blocked(q, q))
    })
}

pub struct RasterLand {
    bounds: Bounds,
    res: f64,
    cols: usize,
    rows: usize,
    /// Row-major from the south; one byte per cell for simplicity.
    cells: Vec<bool>,
    /// Per block of `BLOCK` by `BLOCK` cells, whether any of them is land: a leg whose box lies in blocks
    /// with none is clear without walking it (most legs of an ocean passage).
    blocks: Vec<bool>,
    block_cols: usize,
}

/// The side of a block of `RasterLand::blocks`, in cells.
const BLOCK: usize = 8;

impl RasterLand {
    /// Rasterises `polygons` (even-odd) over `bounds` at `res` degrees per cell
    /// and then grows the land by `margin_cells` in every direction, which is
    /// the router's safety margin. A cell is land when any of the polygon
    /// touches it, so a spit or an islet narrower than a cell is still in the
    /// mask, and a coast is never put a cell inland of where it is.
    pub fn from_polygons(bounds: Bounds, res: f64, polygons: &[Polygon], margin_cells: usize) -> RasterLand {
        let cols = ((bounds.width() / res).ceil() as usize).max(1);
        let rows = ((bounds.height() / res).ceil() as usize).max(1);
        let mut cells = vec![false; cols * rows];
        let col_of = |lon: f64| ((lon - bounds.min_lon) / res).floor();
        let row_of = |lat: f64| ((lat - bounds.min_lat) / res).floor();
        for poly in polygons {
            let pb = poly.bounds();
            if !pb.intersects(&bounds) {
                continue;
            }
            // The inside: where the middle of each row crosses the polygon's
            // edges. That fills every cell that lies wholly inside; the cells
            // the outline passes through are marked below.
            let r0 = row_of(pb.min_lat).max(0.0) as usize;
            let r1 = (row_of(pb.max_lat) + 1.0).clamp(0.0, rows as f64) as usize;
            let mut crossings: Vec<Vec<f64>> = vec![Vec::new(); r1.saturating_sub(r0)];
            for ring in &poly.rings {
                let n = ring.len();
                for i in 0..n {
                    let (a, b) = (ring[i], ring[(i + 1) % n]);
                    // The outline, in steps of half a cell so no cell on it is skipped.
                    let steps = (((b[0] - a[0]).abs().max((b[1] - a[1]).abs()) / (res * 0.5)).ceil() as usize).max(1);
                    for k in 0..=steps {
                        let t = k as f64 / steps as f64;
                        let (c, r) = (col_of(a[0] + (b[0] - a[0]) * t), row_of(a[1] + (b[1] - a[1]) * t));
                        if c >= 0.0 && r >= 0.0 && c < cols as f64 && r < rows as f64 {
                            cells[r as usize * cols + c as usize] = true;
                        }
                    }
                    if a[1] == b[1] {
                        continue;
                    }
                    // The rows whose middle lies between the two ends.
                    let (lo, hi) = (a[1].min(b[1]), a[1].max(b[1]));
                    let first = ((lo - bounds.min_lat) / res - 0.5).ceil().max(r0 as f64);
                    let last = (((hi - bounds.min_lat) / res - 0.5).ceil() - 1.0).min(r1 as f64 - 1.0);
                    let mut r = first;
                    while r <= last {
                        let lat = bounds.min_lat + (r + 0.5) * res;
                        crossings[r as usize - r0].push(a[0] + (lat - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
                        r += 1.0;
                    }
                }
            }
            for (k, xs) in crossings.iter_mut().enumerate() {
                let r = r0 + k;
                xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                for pair in xs.chunks(2) {
                    if let [x0, x1] = pair {
                        let c0 = col_of(*x0).max(0.0) as usize;
                        let c1 = col_of(*x1);
                        if c1 < 0.0 {
                            continue;
                        }
                        for c in c0..=(c1 as usize).min(cols - 1) {
                            cells[r * cols + c] = true;
                        }
                    }
                }
            }
        }
        let mut land = RasterLand { bounds, res, cols, rows, cells, blocks: Vec::new(), block_cols: 0 };
        if margin_cells > 0 {
            land.grow(margin_cells);
        }
        land.summarise();
        land
    }

    fn summarise(&mut self) {
        let (bc, br) = (self.cols.div_ceil(BLOCK), self.rows.div_ceil(BLOCK));
        let mut blocks = vec![false; bc * br];
        for r in 0..self.rows {
            let row = &self.cells[r * self.cols..(r + 1) * self.cols];
            for (c, _) in row.iter().enumerate().filter(|(_, l)| **l) {
                blocks[(r / BLOCK) * bc + c / BLOCK] = true;
            }
        }
        self.blocks = blocks;
        self.block_cols = bc;
    }

    /// Whether the cells from (`c0`, `r0`) to (`c1`, `r1`), inclusive and inside the mask, hold no land, by the
    /// blocks they are in (`false` means "maybe").
    fn clear_by_blocks(&self, c0: usize, r0: usize, c1: usize, r1: usize) -> bool {
        (r0 / BLOCK..=r1 / BLOCK).all(|br| (c0 / BLOCK..=c1 / BLOCK).all(|bc| !self.blocks[br * self.block_cols + bc]))
    }

    fn grow(&mut self, n: usize) {
        for _ in 0..n {
            let src = self.cells.clone();
            for r in 0..self.rows {
                for c in 0..self.cols {
                    if src[r * self.cols + c] {
                        continue;
                    }
                    let near = (r.saturating_sub(1)..=(r + 1).min(self.rows - 1)).any(|rr| (c.saturating_sub(1)..=(c + 1).min(self.cols - 1)).any(|cc| src[rr * self.cols + cc]));
                    if near {
                        self.cells[r * self.cols + c] = true;
                    }
                }
            }
        }
    }

    /// The column and row of `p`, which may be outside the mask.
    fn col_row(&self, p: &LatLon) -> (i64, i64) {
        (floor_i64((p.lon - self.bounds.min_lon) / self.res), floor_i64((p.lat - self.bounds.min_lat) / self.res))
    }

    fn cell_index(&self, p: &LatLon) -> Option<usize> {
        let (c, r) = self.col_row(p);
        if c < 0 || r < 0 || c >= self.cols as i64 || r >= self.rows as i64 {
            return None;
        }
        Some(r as usize * self.cols + c as usize)
    }

    fn cell(&self, p: &LatLon) -> Option<bool> {
        self.cell_index(p).map(|i| self.cells[i])
    }

    /// Outside the bounds counts as land: the mask knows nothing there, and
    /// open water would let a route cross a coast nobody loaded.
    pub fn is_land(&self, p: &LatLon) -> bool {
        self.cell(p).unwrap_or(true)
    }

    fn centre(&self, i: usize) -> LatLon {
        let (r, c) = (i / self.cols, i % self.cols);
        LatLon::new(self.bounds.min_lat + (r as f64 + 0.5) * self.res, self.bounds.min_lon + (c as f64 + 0.5) * self.res)
    }

    /// The open cells joined (side to side) to an open cell on the edge of
    /// the mask that `open_beyond` also takes for water: water that leads out
    /// of the mask, not a pond or a reach of river that the margin cut off.
    fn open_from_edge(&self, open_beyond: impl Fn(&LatLon) -> bool) -> Vec<bool> {
        let (cols, rows) = (self.cols, self.rows);
        let mut reached = vec![false; self.cells.len()];
        let edge = (0..cols).flat_map(|c| [c, (rows - 1) * cols + c]).chain((0..rows).flat_map(|r| [r * cols, r * cols + cols - 1]));
        let mut stack: Vec<usize> = edge.filter(|&i| !self.cells[i] && open_beyond(&self.centre(i))).collect();
        while let Some(i) = stack.pop() {
            if self.cells[i] || reached[i] {
                continue;
            }
            reached[i] = true;
            let (r, c) = (i / cols, i % cols);
            if c > 0 { stack.push(i - 1) }
            if c + 1 < cols { stack.push(i + 1) }
            if r > 0 { stack.push(i - cols) }
            if r + 1 < rows { stack.push(i + cols) }
        }
        reached
    }

    pub fn bounds(&self) -> Bounds {
        self.bounds
    }

    pub fn res(&self) -> f64 {
        self.res
    }

    /// Columns, rows, and the cells (true for land), row-major from the south.
    pub fn grid(&self) -> (usize, usize, &[bool]) {
        (self.cols, self.rows, &self.cells)
    }

    pub fn land_fraction(&self) -> f64 {
        self.cells.iter().filter(|c| **c).count() as f64 / self.cells.len() as f64
    }
}

impl Land for RasterLand {
    fn seaway(&self, goal: LatLon) -> Option<Arc<SeaField>> {
        SeaField::new(self, goal, seaway::MAX_CELLS).map(Arc::new)
    }

    fn blocked(&self, a: &LatLon, b: &LatLon) -> bool {
        // A leg whose box (and a cell round it) lies in blocks with no land is clear.
        let ((ca, ra), (cb, rb)) = (self.col_row(a), self.col_row(b));
        let (c0, c1, r0, r1) = (ca.min(cb) - 1, ca.max(cb) + 1, ra.min(rb) - 1, ra.max(rb) + 1);
        if c0 >= 0 && r0 >= 0 && c1 < self.cols as i64 && r1 < self.rows as i64 && self.clear_by_blocks(c0 as usize, r0 as usize, c1 as usize, r1 as usize) {
            return false;
        }
        // Walk the leg in steps of half a cell so no cell can be skipped.
        let (dx, dy) = (b.lon - a.lon, b.lat - a.lat);
        let len = dx.abs().max(dy.abs());
        let n = ((len / (self.res * 0.5)).ceil() as usize).max(1);
        (0..=n).any(|i| {
            let t = i as f64 / n as f64;
            self.is_land(&LatLon::new(a.lat + dy * t, a.lon + dx * t))
        })
    }
}

/// A coarse mask with finer ones over small areas (the ends of a long
/// passage, where a coarse cell and its margin fill a river or a harbour).
/// Inside a fine mask's bounds, it alone decides.
pub struct DetailedLand {
    pub base: RasterLand,
    pub detail: Vec<RasterLand>,
    /// The fields `seaway` worked out, by goal and the number of fine masks then: every departure of a
    /// weather window asks for the same two, and each is a search over millions of cells.
    fields: Mutex<Vec<FieldEntry>>,
}

/// A goal's bits, the number of fine masks, and its field once worked out.
type FieldEntry = ((u64, u64, usize), Arc<OnceLock<Option<Arc<SeaField>>>>);

/// The most fields `DetailedLand` keeps.
const FIELDS_KEPT: usize = 6;

impl DetailedLand {
    pub fn new(base: RasterLand, detail: Vec<RasterLand>) -> DetailedLand {
        DetailedLand { base, detail, fields: Mutex::new(Vec::new()) }
    }

    /// The field over the coarse mask towards `goal`, with each of its cells that holds water in a fine
    /// mask counted as water.
    fn make_seaway(&self, goal: LatLon) -> Option<Arc<SeaField>> {
        let b = &self.base;
        let mut cells = b.cells.clone();
        for d in &self.detail {
            for (i, _) in d.cells.iter().enumerate().filter(|(_, land)| !**land) {
                if let Some(j) = b.cell_index(&d.centre(i)) {
                    cells[j] = false;
                }
            }
        }
        SeaField::from_cells(b.bounds, b.res, b.cols, b.rows, &cells, goal, seaway::MAX_CELLS).map(Arc::new)
    }

    pub fn is_land(&self, p: &LatLon) -> bool {
        match self.detail.iter().find(|d| d.bounds.contains([p.lon, p.lat])) {
            Some(d) => d.is_land(p),
            None => self.base.is_land(p),
        }
    }
}

impl DetailedLand {
    /// Where `path`, the way to its last point found on a coarser mask
    /// without a margin, is cut off by this one (a strait the margin
    /// closes): the area round the cut that wants a fine mask. `None` when
    /// the path holds, or the cut is inside a fine mask already.
    pub fn cut_area(&self, path: &[LatLon]) -> Option<Bounds> {
        let field = self.seaway(*path.last()?)?;
        let (good, bad) = field.cut(path)?;
        let m = CUT_MARGIN_DEG;
        let b = Bounds { min_lon: good.lon.min(bad.lon) - m, max_lon: good.lon.max(bad.lon) + m, min_lat: good.lat.min(bad.lat) - m, max_lat: good.lat.max(bad.lat) + m };
        let covered = |p: &LatLon| self.detail.iter().any(|d| d.bounds.contains([p.lon, p.lat]));
        (!(covered(&good) && covered(&bad))).then_some(b)
    }
}

/// How far round a cut in the way by water its fine mask reaches.
const CUT_MARGIN_DEG: f64 = 0.15;

impl Land for DetailedLand {
    /// Over the coarse mask, with each of its cells that holds water in a
    /// fine mask counted as water: a strait the coarse mask closes is open.
    fn seaway(&self, goal: LatLon) -> Option<Arc<SeaField>> {
        let key = (goal.lat.to_bits(), goal.lon.to_bits(), self.detail.len());
        // The lock is held only to find the slot; the search runs outside it, once, whoever asks first.
        let slot = {
            let mut fields = self.fields.lock().unwrap_or_else(|e| e.into_inner());
            match fields.iter().find(|(k, _)| *k == key) {
                Some((_, slot)) => slot.clone(),
                None => {
                    if fields.len() == FIELDS_KEPT {
                        fields.remove(0);
                    }
                    let slot = Arc::new(OnceLock::new());
                    fields.push((key, slot.clone()));
                    slot
                }
            }
        };
        slot.get_or_init(|| self.make_seaway(goal)).clone()
    }

    /// Inside a fine mask: from the nearest water that leads out of the
    /// mask (not a pocket of river the margin cut off), the shortest way
    /// through the fine mask's water to a place with `clear_nm` all round.
    fn approach(&self, p: LatLon, clear_nm: f64, snap_nm: f64) -> Option<Vec<LatLon>> {
        let Some(d) = self.detail.iter().find(|d| d.bounds.contains([p.lon, p.lat])) else { return nearest_open(self, p, snap_nm).map(|q| if q == p { vec![p] } else { vec![p, q] }) };
        let reached = d.open_from_edge(|q| !self.base.is_land(q));
        let first = match d.cell_index(&p).filter(|&i| reached[i]) {
            Some(i) => i,
            None => (0..reached.len()).filter(|&i| reached[i]).map(|i| (d.centre(i).distance_nm(&p), i)).filter(|(nm, _)| *nm <= snap_nm).min_by(|a, b| a.0.total_cmp(&b.0))?.1,
        };
        let roomy = |q: &LatLon| (0..16).all(|k| [clear_nm, clear_nm / 2.0].iter().all(|r| !self.is_land(&q.destination(k as f64 * 22.5, *r))));
        // Breadth first, side to side, until there is room.
        let (cols, rows) = (d.cols, d.rows);
        let mut parent = vec![usize::MAX; reached.len()];
        parent[first] = first;
        let mut queue = std::collections::VecDeque::from([first]);
        let (mut end, mut edge) = (None, None);
        while let Some(i) = queue.pop_front() {
            if roomy(&d.centre(i)) {
                end = Some(i);
                break;
            }
            let (r, c) = (i / cols, i % cols);
            if edge.is_none() && (r == 0 || c == 0 || r + 1 == rows || c + 1 == cols) {
                edge = Some(i);
            }
            let next = [(c > 0).then(|| i - 1), (c + 1 < cols).then(|| i + 1), (r > 0).then(|| i - cols), (r + 1 < rows).then(|| i + cols)];
            for j in next.into_iter().flatten() {
                if reached[j] && parent[j] == usize::MAX {
                    parent[j] = i;
                    queue.push_back(j);
                }
            }
        }
        // No room anywhere in the fine mask: the nearest way out of it will do.
        let end = end.or(edge)?;
        let mut cells = vec![end];
        while parent[cells[cells.len() - 1]] != cells[cells.len() - 1] {
            cells.push(parent[cells[cells.len() - 1]]);
        }
        cells.reverse();
        let mut steps: Vec<LatLon> = cells.iter().map(|&i| d.centre(i)).collect();
        if d.cell_index(&p) == Some(first) {
            steps[0] = p;
        }
        // Straighten the staircase: from each kept point, the farthest point
        // in a run that it sees over open water.
        let mut out = vec![p];
        if steps[0] != p {
            out.push(steps[0]);
        }
        let mut i = 0;
        while i + 1 < steps.len() {
            let mut j = i + 1;
            while j + 1 < steps.len() && !d.blocked(&steps[i], &steps[j + 1]) {
                j += 1;
            }
            out.push(steps[j]);
            i = j;
        }
        out.dedup();
        Some(out)
    }

    fn blocked(&self, a: &LatLon, b: &LatLon) -> bool {
        // Steps of half the finest cell the leg may cross.
        let leg = Bounds::of(&[[a.lon, a.lat], [b.lon, b.lat]]);
        let mut fine = self.detail.iter().filter(|d| d.bounds.intersects(&leg)).peekable();
        // Away from the fine masks the coarse one alone decides.
        if fine.peek().is_none() {
            return self.base.blocked(a, b);
        }
        let res = fine.map(|d| d.res).fold(self.base.res, f64::min);
        let (dx, dy) = (b.lon - a.lon, b.lat - a.lat);
        let n = (((dx.abs().max(dy.abs())) / (res * 0.5)).ceil() as usize).max(1);
        (0..=n).any(|i| {
            let t = i as f64 / n as f64;
            self.is_land(&LatLon::new(a.lat + dy * t, a.lon + dx * t))
        })
    }
}

/// A mask with no land, for open-water tests and when no chart data is loaded.
pub struct NoLand;

impl Land for NoLand {
    fn blocked(&self, _: &LatLon, _: &LatLon) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn island() -> Polygon {
        // A square island from 1..2 degrees east and north.
        Polygon { rings: vec![vec![[1.0, 1.0], [2.0, 1.0], [2.0, 2.0], [1.0, 2.0], [1.0, 1.0]]] }
    }

    fn bounds() -> Bounds {
        Bounds { min_lon: 0.0, min_lat: 0.0, max_lon: 3.0, max_lat: 3.0 }
    }

    #[test]
    fn the_island_is_rasterised() {
        let l = RasterLand::from_polygons(bounds(), 0.01, &[island()], 0);
        assert!(l.is_land(&LatLon::new(1.5, 1.5)));
        assert!(!l.is_land(&LatLon::new(0.5, 0.5)));
        assert!(!l.is_land(&LatLon::new(2.5, 1.5)));
        // One ninth of the box is land.
        assert!((l.land_fraction() - 1.0 / 9.0).abs() < 0.01, "{}", l.land_fraction());
    }

    #[test]
    fn a_cell_the_land_only_partly_covers_is_land_on_either_side() {
        // 0.03 to 0.17 east: the cells 0 to 0.1 and 0.1 to 0.2 are both touched.
        let b = Bounds { min_lon: 0.0, min_lat: 0.0, max_lon: 0.5, max_lat: 0.5 };
        let p = Polygon { rings: vec![vec![[0.03, 0.03], [0.17, 0.03], [0.17, 0.17], [0.03, 0.17], [0.03, 0.03]]] };
        let l = RasterLand::from_polygons(b, 0.1, &[p], 0);
        assert!(l.is_land(&LatLon::new(0.05, 0.05)) && l.is_land(&LatLon::new(0.15, 0.15)));
        assert!(!l.is_land(&LatLon::new(0.25, 0.15)) && !l.is_land(&LatLon::new(0.15, 0.25)));
    }

    #[test]
    fn land_thinner_than_a_cell_is_not_lost() {
        let b = Bounds { min_lon: 0.0, min_lat: 0.0, max_lon: 1.0, max_lat: 1.0 };
        // A spit 0.002 wide and 0.5 long, and a sliver 0.5 wide and 0.002 tall, in 0.1 cells.
        let spit = Polygon { rings: vec![vec![[0.151, 0.2], [0.153, 0.2], [0.153, 0.7], [0.151, 0.7], [0.151, 0.2]]] };
        let sliver = Polygon { rings: vec![vec![[0.2, 0.851], [0.7, 0.851], [0.7, 0.853], [0.2, 0.853], [0.2, 0.851]]] };
        let l = RasterLand::from_polygons(b, 0.1, &[spit, sliver], 0);
        assert!(l.blocked(&LatLon::new(0.45, 0.05), &LatLon::new(0.45, 0.35)), "across the spit");
        assert!(l.blocked(&LatLon::new(0.75, 0.45), &LatLon::new(0.95, 0.45)), "across the sliver");
    }

    #[test]
    fn holes_stay_water() {
        let mut p = island();
        p.rings.push(vec![[1.4, 1.4], [1.6, 1.4], [1.6, 1.6], [1.4, 1.6], [1.4, 1.4]]);
        let l = RasterLand::from_polygons(bounds(), 0.01, &[p], 0);
        assert!(!l.is_land(&LatLon::new(1.5, 1.5)), "a lake");
        assert!(l.is_land(&LatLon::new(1.2, 1.2)));
    }

    #[test]
    fn legs_across_the_island_are_blocked_and_legs_beside_it_are_not() {
        let l = RasterLand::from_polygons(bounds(), 0.01, &[island()], 0);
        assert!(l.blocked(&LatLon::new(1.5, 0.5), &LatLon::new(1.5, 2.5)));
        assert!(!l.blocked(&LatLon::new(0.5, 0.2), &LatLon::new(0.5, 2.8)));
        // A leg that ends on the island is blocked; one that stops short is not.
        assert!(l.blocked(&LatLon::new(1.5, 0.5), &LatLon::new(1.5, 1.2)));
        assert!(!l.blocked(&LatLon::new(1.5, 0.5), &LatLon::new(1.5, 0.9)));
        // A very short leg inside land is blocked.
        assert!(l.blocked(&LatLon::new(1.5, 1.5), &LatLon::new(1.5001, 1.5001)));
    }

    #[test]
    fn a_detail_mask_decides_inside_its_bounds() {
        // A channel 0.02 wide through the island, lost in 0.1 cells.
        let west = Polygon { rings: vec![vec![[1.0, 1.0], [1.49, 1.0], [1.49, 2.0], [1.0, 2.0], [1.0, 1.0]]] };
        let east = Polygon { rings: vec![vec![[1.51, 1.0], [2.0, 1.0], [2.0, 2.0], [1.51, 2.0], [1.51, 1.0]]] };
        let polys = [west, east];
        let in_channel = LatLon::new(1.5, 1.5);
        let base = RasterLand::from_polygons(bounds(), 0.1, &polys, 0);
        assert!(base.is_land(&in_channel));
        let fine = RasterLand::from_polygons(Bounds { min_lon: 1.3, min_lat: 1.3, max_lon: 1.7, max_lat: 1.7 }, 0.005, &polys, 0);
        let l = DetailedLand::new(base, vec![fine]);
        assert!(!l.is_land(&in_channel));
        assert!(!l.blocked(&LatLon::new(1.4, 1.5), &LatLon::new(1.6, 1.5)), "up the channel");
        assert!(l.blocked(&in_channel, &LatLon::new(1.5, 1.4)), "across the fine mask's land");
        assert!(l.blocked(&LatLon::new(1.8, 1.5), &LatLon::new(1.65, 1.5)), "from the coarse land in");
    }

    #[test]
    fn the_way_out_of_a_harbour_follows_its_channel_to_open_water() {
        // Land north of 1.5 with a channel 0.02 wide up to a harbour at 1.8 N, 1.5 E.
        let west = Polygon { rings: vec![vec![[0.0, 1.5], [1.49, 1.5], [1.49, 3.0], [0.0, 3.0], [0.0, 1.5]]] };
        let east = Polygon { rings: vec![vec![[1.51, 1.5], [3.0, 1.5], [3.0, 3.0], [1.51, 3.0], [1.51, 1.5]]] };
        let polys = [west, east];
        let harbour = LatLon::new(1.8, 1.5);
        let base = RasterLand::from_polygons(bounds(), 0.05, &polys, 1);
        assert!(base.is_land(&harbour));
        let fine = RasterLand::from_polygons(Bounds { min_lon: 1.0, min_lat: 1.0, max_lon: 2.0, max_lat: 2.0 }, 0.002, &polys, 1);
        let l = DetailedLand::new(base, vec![fine]);
        let way = l.approach(harbour, 2.0, 1.0).expect("a way out");
        assert_eq!(way[0], harbour);
        let end = way[way.len() - 1];
        assert!(end.lat < 1.49, "out of the channel at {end:?}");
        for w in way.windows(2) {
            assert!(!l.blocked(&w[0], &w[1]), "the way out crosses land at {:?}", w[0]);
        }
        // Too far from water to snap to.
        assert_eq!(l.approach(LatLon::new(1.8, 1.2), 2.0, 1.0), None);
    }

    #[test]
    fn the_safety_margin_widens_the_land() {
        let plain = RasterLand::from_polygons(bounds(), 0.01, &[island()], 0);
        let margin = RasterLand::from_polygons(bounds(), 0.01, &[island()], 5);
        let near = LatLon::new(1.5, 0.97);
        assert!(!plain.is_land(&near));
        assert!(margin.is_land(&near), "3 hundredths from the shore is inside a 5-cell margin");
        assert!(!margin.is_land(&LatLon::new(1.5, 0.9)));
    }

    #[test]
    fn outside_the_mask_is_land_and_no_land_never_blocks() {
        let l = RasterLand::from_polygons(bounds(), 0.01, &[island()], 0);
        assert!(l.is_land(&LatLon::new(50.0, 50.0)));
        assert!(l.blocked(&LatLon::new(0.5, 0.5), &LatLon::new(0.5, 50.0)), "a leg out of the mask");
        assert!(!NoLand.blocked(&LatLon::new(0.0, 0.0), &LatLon::new(1.0, 1.0)));
        // Polygons outside the box are ignored.
        let far = Polygon { rings: vec![vec![[10.0, 10.0], [11.0, 10.0], [11.0, 11.0], [10.0, 10.0]]] };
        assert_eq!(RasterLand::from_polygons(bounds(), 0.1, &[far], 0).land_fraction(), 0.0);
    }
}
