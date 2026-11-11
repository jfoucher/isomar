//! The distance by water to a goal from anywhere in a land mask, and the way
//! there: a Dijkstra search out from the goal over the mask's water cells.
//!
//! The isochrone router judges progress by it instead of the distance as the
//! crow flies, which points through land whenever the passage goes round it
//! (Napoli → Split goes south round Calabria, away from Split, for two days).

use super::land::RasterLand;
use crate::geo::LatLon;
use crate::vector::{Bounds, Polygon};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Cells beyond this are merged, a block at a time, before the search.
pub const MAX_CELLS: usize = 2_000_000;

/// A point on another mask's path is cut off here when the way from it is
/// this much longer, plus a few cells for the two grids' staircases.
const CUT_RATIO: f64 = 1.25;
const CUT_SLACK_CELLS: f64 = 6.0;

/// Moves to the 16 neighbours: sides, corners and knight's moves, so open
/// water costs at most 3% more than the great circle.
const MOVES: [(i32, i32); 16] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1), (2, 1), (2, -1), (-2, 1), (-2, -1), (1, 2), (1, -2), (-1, 2), (-1, -2)];

pub struct SeaField {
    bounds: Bounds,
    /// Degrees per cell.
    res: f64,
    cols: usize,
    rows: usize,
    /// Nautical miles by water to the goal; infinite where it cannot be reached.
    dist: Vec<f32>,
    /// The next cell on the way to the goal; the goal's own cell points to itself.
    next: Vec<u32>,
    goal: LatLon,
    /// The cosine of each row's latitude, for distances within a few cells.
    cos_row: Vec<f64>,
}

/// `x.floor()` as an integer, without the libm call `f64::floor` is on x86-64 without SSE4.1.
#[inline]
pub(crate) fn floor_i64(x: f64) -> i64 {
    let i = x as i64;
    if (i as f64) > x { i - 1 } else { i }
}

impl SeaField {
    /// The field over `mask` towards `goal`. A cell is water if any of the
    /// mask's cells merged into it is, so a strait narrower than a merged
    /// cell stays open. `None` when no water is near the goal.
    pub fn new(mask: &RasterLand, goal: LatLon, max_cells: usize) -> Option<SeaField> {
        let (cols, rows, cells) = mask.grid();
        SeaField::from_cells(mask.bounds(), mask.res(), cols, rows, cells, goal, max_cells)
    }

    /// The same over a grid of `mcols` by `mcells` cells of `mres` degrees
    /// from the south-west corner of `bounds`, row-major from the south, true
    /// for land.
    pub fn from_cells(bounds: Bounds, mres: f64, mcols: usize, mrows: usize, mcells: &[bool], goal: LatLon, max_cells: usize) -> Option<SeaField> {
        let k = ((mcols * mrows) as f64 / max_cells.max(1) as f64).sqrt().ceil().max(1.0) as usize;
        let (cols, rows) = (mcols.div_ceil(k), mrows.div_ceil(k));
        let mut water = vec![false; cols * rows];
        for r in 0..mrows {
            for c in 0..mcols {
                if !mcells[r * mcols + c] {
                    water[(r / k) * cols + c / k] = true;
                }
            }
        }
        let res = mres * k as f64;
        let cos_row = (0..rows).map(|r| (bounds.min_lat + (r as f64 + 0.5) * res).to_radians().cos()).collect();
        let mut field = SeaField { bounds, res, cols, rows, dist: vec![f32::INFINITY; cols * rows], next: vec![u32::MAX; cols * rows], goal, cos_row };
        // The goal itself may be in a cell the mask's margin fills.
        let seed = field.nearest(&goal, 10, |i| water[i])?;
        field.dist[seed] = field.centre(seed).distance_nm(&goal) as f32;
        field.next[seed] = seed as u32;
        let dy = res * 60.0;
        let dx: Vec<f64> = (0..rows).map(|r| dy * (bounds.min_lat + (r as f64 + 0.5) * res).to_radians().cos()).collect();
        let at = |c: i32, r: i32| (c >= 0 && r >= 0 && (c as usize) < cols && (r as usize) < rows).then(|| r as usize * cols + c as usize);
        let open = |c: i32, r: i32| at(c, r).is_some_and(|i| water[i]);
        let mut heap = BinaryHeap::from([Reverse((field.dist[seed].to_bits(), seed))]);
        while let Some(Reverse((bits, i))) = heap.pop() {
            let d = f32::from_bits(bits);
            if d > field.dist[i] {
                continue;
            }
            let (c, r) = ((i % cols) as i32, (i / cols) as i32);
            for (mc, mr) in MOVES {
                let (nc, nr) = (c + mc, r + mr);
                let Some(j) = at(nc, nr).filter(|&j| water[j]) else { continue };
                // A corner may be cut past one land cell, not two; a knight's
                // move needs both cells it crosses.
                let passable = match (mc.abs(), mr.abs()) {
                    (1, 1) => open(nc, r) || open(c, nr),
                    (2, 1) => open(c + mc / 2, r) && open(c + mc / 2, nr),
                    (1, 2) => open(c, r + mr / 2) && open(nc, r + mr / 2),
                    _ => true,
                };
                if !passable {
                    continue;
                }
                let mid = ((r + nr) as usize) / 2;
                let nd = d + (mc as f64 * dx[mid]).hypot(mr as f64 * dy) as f32;
                if nd < field.dist[j] {
                    field.dist[j] = nd;
                    field.next[j] = i as u32;
                    heap.push(Reverse((nd.to_bits(), j)));
                }
            }
        }
        Some(field)
    }

    /// The size of a cell north to south, in nautical miles.
    pub fn cell_nm(&self) -> f64 {
        self.res * 60.0
    }

    /// The distance by water from `p` to the goal, or `None` where the
    /// field does not reach (outside it, or water cut off from the goal).
    pub fn distance_nm(&self, p: &LatLon) -> Option<f64> {
        let i = self.reached_near(p)?;
        Some(self.dist[i] as f64 + self.near_nm(p, i))
    }

    /// How far `p` is from the centre of cell `i`, a few cells away at most: flat, with the
    /// row's own scale for longitude (no trigonometry, which was most of a search's time).
    fn near_nm(&self, p: &LatLon, i: usize) -> f64 {
        let (r, c) = (i / self.cols, i % self.cols);
        let dy = (p.lat - (self.bounds.min_lat + (r as f64 + 0.5) * self.res)) * 60.0;
        let dx = (p.lon - (self.bounds.min_lon + (c as f64 + 0.5) * self.res)) * 60.0 * self.cos_row[r];
        dx.hypot(dy)
    }

    /// The point `ahead_nm` along the way by water from `p` to the goal (the
    /// goal itself when nearer).
    pub fn toward(&self, p: &LatLon, ahead_nm: f64) -> Option<LatLon> {
        let mut i = self.reached_near(p)?;
        let until = self.dist[i] as f64 - ahead_nm;
        while self.dist[i] as f64 > until {
            let n = self.next[i] as usize;
            if n == i {
                return Some(self.goal);
            }
            i = n;
        }
        Some(self.centre(i))
    }

    /// Where `path`, a way to the goal found on another (coarser) mask, is
    /// cut off on this one: walking it back from the goal, the last point
    /// whose distance here still matches the rest of the path, and the
    /// first that does not (unreachable, or only by a long detour). `None`
    /// when the whole path holds.
    pub fn cut(&self, path: &[LatLon]) -> Option<(LatLon, LatLon)> {
        let mut rest = 0.0;
        let mut good = *path.last()?;
        for w in path.windows(2).rev() {
            rest += w[0].distance_nm(&w[1]);
            let p = w[0];
            match self.distance_nm(&p) {
                Some(d) if d <= rest * CUT_RATIO + CUT_SLACK_CELLS * self.cell_nm() => good = p,
                _ => return Some((good, p)),
            }
        }
        None
    }

    /// The box round every cell on a way from `from`'s goal to this field's
    /// goal at most `ratio` times as long as the shortest: where the
    /// passage can go, not just where it goes if the wind lets it take the
    /// shortest way. Both fields must be over the same mask. `None` when
    /// this field does not reach `from`'s goal.
    pub fn corridor(&self, from: &SeaField, ratio: f64) -> Option<Bounds> {
        assert!(self.cols == from.cols && self.rows == from.rows, "fields over different masks");
        let limit = self.distance_nm(&from.goal)? * ratio;
        let mut b = Bounds::EMPTY;
        for (i, (a, f)) in self.dist.iter().zip(&from.dist).enumerate() {
            if (*a as f64 + *f as f64) <= limit {
                let c = self.centre(i);
                b.extend([c.lon - self.res / 2.0, c.lat - self.res / 2.0]);
                b.extend([c.lon + self.res / 2.0, c.lat + self.res / 2.0]);
            }
        }
        (b.min_lon <= b.max_lon).then_some(b)
    }

    /// The way by water from `p` to the goal, one point a cell, `p` first
    /// and the goal last.
    pub fn path(&self, p: &LatLon) -> Option<Vec<LatLon>> {
        let mut i = self.reached_near(p)?;
        let mut out = vec![*p];
        while self.next[i] as usize != i {
            out.push(self.centre(i));
            i = self.next[i] as usize;
        }
        out.push(self.goal);
        Some(out)
    }

    /// The cell of `p` if the search reached it, otherwise the reached cell
    /// nearest it within two cells (`p` is in the mask's margin).
    fn reached_near(&self, p: &LatLon) -> Option<usize> {
        self.nearest(p, 2, |i| self.dist[i].is_finite())
    }

    /// The cell within `radius` cells of `p` that passes `ok`, nearest `p`.
    fn nearest(&self, p: &LatLon, radius: i32, ok: impl Fn(usize) -> bool) -> Option<usize> {
        let c = floor_i64((p.lon - self.bounds.min_lon) / self.res);
        let r = floor_i64((p.lat - self.bounds.min_lat) / self.res);
        let inside = |rr: i64, cc: i64| rr >= 0 && cc >= 0 && (rr as usize) < self.rows && (cc as usize) < self.cols;
        // Its own cell first: nearly always the answer.
        if inside(r, c) && ok(r as usize * self.cols + c as usize) {
            return Some(r as usize * self.cols + c as usize);
        }
        let radius = radius as i64;
        let mut best: Option<(f64, usize)> = None;
        for rr in r - radius..=r + radius {
            for cc in c - radius..=c + radius {
                if !inside(rr, cc) {
                    continue;
                }
                let i = rr as usize * self.cols + cc as usize;
                if ok(i) {
                    let d = self.near_nm(p, i);
                    if best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, i));
                    }
                }
            }
        }
        best.map(|(_, i)| i)
    }

    fn centre(&self, i: usize) -> LatLon {
        let (r, c) = (i / self.cols, i % self.cols);
        LatLon::new(self.bounds.min_lat + (r as f64 + 0.5) * self.res, self.bounds.min_lon + (c as f64 + 0.5) * self.res)
    }
}

/// The way by water from `start` to `dest` over `polygons` inside `bounds`,
/// on a coarse mask of about `max_cells`: what tells the router which area
/// a passage needs. `None` when the land inside `bounds` separates them.
pub fn sea_path(bounds: Bounds, polygons: &[Polygon], start: LatLon, dest: LatLon, max_cells: usize) -> Option<Vec<LatLon>> {
    let res = (bounds.width() * bounds.height() / max_cells.max(1) as f64).sqrt().max(0.005);
    let mask = RasterLand::from_polygons(bounds, res, polygons, 0);
    SeaField::new(&mask, dest, max_cells)?.path(&start)
}

/// A way by water this much longer than the shortest is still one a
/// passage may take, when the wind favours it.
pub const ALTERNATIVE_RATIO: f64 = 1.25;

/// The shortest way by water and the box round all the ways up to
/// `ALTERNATIVE_RATIO` times as long, so a passage round land that can go
/// either way (Liverpool → Hull, round Land's End or round Scotland) is
/// given room for both. Found on a coarse mask like `sea_path`, so the ends
/// must be in open water, not up a river the mask fills.
pub struct Ways {
    pub path: Vec<LatLon>,
    pub corridor: Bounds,
}

pub fn sea_ways(bounds: Bounds, polygons: &[Polygon], start: LatLon, dest: LatLon, max_cells: usize) -> Option<Ways> {
    let res = (bounds.width() * bounds.height() / max_cells.max(1) as f64).sqrt().max(0.005);
    let mask = RasterLand::from_polygons(bounds, res, polygons, 0);
    let to_dest = SeaField::new(&mask, dest, max_cells)?;
    let path = to_dest.path(&start)?;
    let corridor = SeaField::new(&mask, start, max_cells).and_then(|from| to_dest.corridor(&from, ALTERNATIVE_RATIO));
    let mut all = Bounds::EMPTY;
    for p in &path {
        all.extend([p.lon, p.lat]);
    }
    if let Some(c) = corridor {
        all.extend([c.min_lon, c.min_lat]);
        all.extend([c.max_lon, c.max_lat]);
    }
    Some(Ways { path, corridor: all })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wall along 1 degree east from the south edge to 1.8 north, with
    /// water round its northern end.
    fn wall() -> RasterLand {
        let b = Bounds { min_lon: 0.0, max_lon: 2.0, min_lat: 0.0, max_lat: 2.0 };
        let wall = Polygon { rings: vec![vec![[0.95, -0.1], [1.05, -0.1], [1.05, 1.8], [0.95, 1.8], [0.95, -0.1]]] };
        RasterLand::from_polygons(b, 0.01, &[wall], 1)
    }

    #[test]
    fn open_water_costs_about_the_great_circle() {
        let land = RasterLand::from_polygons(Bounds { min_lon: 0.0, max_lon: 2.0, min_lat: 40.0, max_lat: 42.0 }, 0.01, &[], 0);
        let goal = LatLon::new(41.8, 1.8);
        let f = SeaField::new(&land, goal, MAX_CELLS).unwrap();
        for p in [LatLon::new(40.1, 0.1), LatLon::new(41.8, 0.1), LatLon::new(40.1, 1.7), LatLon::new(41.0, 0.6)] {
            let (sea, gc) = (f.distance_nm(&p).unwrap(), p.distance_nm(&goal));
            assert!(sea >= gc - 1.0 && sea <= gc * 1.035 + 1.0, "{p:?}: {sea} vs {gc}");
        }
    }

    #[test]
    fn the_way_round_a_wall_is_measured_and_followed() {
        let land = wall();
        let (a, goal) = (LatLon::new(0.5, 0.5), LatLon::new(0.5, 1.5));
        let f = SeaField::new(&land, goal, MAX_CELLS).unwrap();
        let sea = f.distance_nm(&a).unwrap();
        // Up 1.35 degrees, across, and down: about 2 x 83 nm.
        assert!(sea > 150.0 && sea < 185.0, "{sea}");
        // The way starts north, along the wall, not east through it.
        let ahead = f.toward(&a, 20.0).unwrap();
        let b = a.bearing_to(&ahead);
        assert!(!(45.0..=315.0).contains(&b), "{b}");
        let path = f.path(&a).unwrap();
        assert_eq!((path[0], *path.last().unwrap()), (a, goal));
        assert!(path.iter().any(|p| p.lat > 1.8));
        // Merged cells measure much the same.
        let coarse = SeaField::new(&land, goal, 5_000).unwrap();
        assert!(coarse.cell_nm() > f.cell_nm());
        assert!((coarse.distance_nm(&a).unwrap() - sea).abs() < 10.0);
    }

    #[test]
    fn a_strait_narrower_than_a_merged_cell_stays_open() {
        let b = Bounds { min_lon: 0.0, max_lon: 2.0, min_lat: 0.0, max_lat: 2.0 };
        // A wall right across, with a gap of 0.03 degree at 1 north.
        let south = Polygon { rings: vec![vec![[0.95, -0.1], [1.05, -0.1], [1.05, 0.985], [0.95, 0.985], [0.95, -0.1]]] };
        let north = Polygon { rings: vec![vec![[0.95, 1.015], [1.05, 1.015], [1.05, 2.1], [0.95, 2.1], [0.95, 1.015]]] };
        let land = RasterLand::from_polygons(b, 0.005, &[south, north], 0);
        let f = SeaField::new(&land, LatLon::new(1.0, 1.5), 10_000).unwrap();
        let d = f.distance_nm(&LatLon::new(1.0, 0.5)).unwrap();
        assert!(d < 70.0, "{d}");
    }

    #[test]
    fn a_path_through_a_strait_this_mask_closes_is_cut_there() {
        let b = Bounds { min_lon: 0.0, max_lon: 2.0, min_lat: 0.0, max_lat: 2.0 };
        // A wall across with a 0.03 degree gap at 1 north; open at the far north.
        let south = Polygon { rings: vec![vec![[0.95, -0.1], [1.05, -0.1], [1.05, 0.985], [0.95, 0.985], [0.95, -0.1]]] };
        let north = Polygon { rings: vec![vec![[0.95, 1.015], [1.05, 1.015], [1.05, 1.9], [0.95, 1.9], [0.95, 1.015]]] };
        let (a, goal) = (LatLon::new(1.0, 0.5), LatLon::new(1.0, 1.5));
        let fine = RasterLand::from_polygons(b, 0.002, &[south.clone(), north.clone()], 0);
        let path = SeaField::new(&fine, goal, MAX_CELLS).unwrap().path(&a).unwrap();
        assert!(path.iter().all(|p| p.lat < 1.1), "through the gap");
        // With a margin the gap is shut, and the way is round the north end.
        let coarse = RasterLand::from_polygons(b, 0.01, &[south, north], 1);
        let field = SeaField::new(&coarse, goal, MAX_CELLS).unwrap();
        let (good, bad) = field.cut(&path).expect("cut at the gap");
        for p in [good, bad] {
            assert!((p.lat - 1.0).abs() < 0.1 && (0.85..1.15).contains(&p.lon), "{good:?} {bad:?}");
        }
        assert!(SeaField::new(&fine, goal, MAX_CELLS).unwrap().cut(&path).is_none());
    }

    #[test]
    fn the_corridor_holds_the_other_way_round_land() {
        // A wall from 0.5 to 1.8 north across the line between the ends: the
        // short way is round its southern end (about 92 nm), the other
        // round its northern end (about 122).
        let b = Bounds { min_lon: 0.0, max_lon: 2.0, min_lat: 0.0, max_lat: 2.0 };
        let wall = Polygon { rings: vec![vec![[0.95, 0.5], [1.05, 0.5], [1.05, 1.8], [0.95, 1.8], [0.95, 0.5]]] };
        let land = RasterLand::from_polygons(b, 0.01, &[wall], 0);
        let (a, goal) = (LatLon::new(1.0, 0.5), LatLon::new(1.0, 1.5));
        let (to, from) = (SeaField::new(&land, goal, MAX_CELLS).unwrap(), SeaField::new(&land, a, MAX_CELLS).unwrap());
        let path = to.path(&a).unwrap();
        assert!(path.iter().all(|p| p.lat < 1.0 + 0.1), "{path:?}");
        let near = to.corridor(&from, 1.05).unwrap();
        assert!(near.max_lat < 1.5, "{near:?}");
        let wide = to.corridor(&from, 1.45).unwrap();
        assert!(wide.max_lat > 1.8, "{wide:?}");
    }

    #[test]
    fn water_cut_off_from_the_goal_is_not_reached() {
        let b = Bounds { min_lon: 0.0, max_lon: 2.0, min_lat: 0.0, max_lat: 2.0 };
        let wall = Polygon { rings: vec![vec![[0.95, -0.1], [1.05, -0.1], [1.05, 2.1], [0.95, 2.1], [0.95, -0.1]]] };
        let land = RasterLand::from_polygons(b, 0.01, &[wall], 1);
        let f = SeaField::new(&land, LatLon::new(1.0, 1.5), MAX_CELLS).unwrap();
        assert!(f.distance_nm(&LatLon::new(1.0, 0.5)).is_none());
        assert!(f.distance_nm(&LatLon::new(1.0, 1.2)).is_some());
        assert!(f.distance_nm(&LatLon::new(5.0, 5.0)).is_none());
    }
}
