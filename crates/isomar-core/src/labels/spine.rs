//! A feature's centreline: the ridge of a distance map, from one end of the feature to the other.

use super::mask::Mask;
use crate::vector::{Pt, simplify};
use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

/// A feature's centreline in units, read left to right, with the room on either side of it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Spine {
    pub pts: Vec<Pt>,
    /// How far each point is from the edge of the feature, in units.
    pub clear: Vec<f64>,
}

/// A polyline with its cumulative arc length, which `at` and `clear_at` search.
#[derive(Clone, Debug)]
pub struct Path {
    pub pts: Vec<Pt>,
    pub clear: Vec<f64>,
    pub cum: Vec<f64>,
}

impl Path {
    pub fn new(pts: Vec<Pt>, clear: Vec<f64>) -> Path {
        let mut cum = Vec::with_capacity(pts.len());
        let mut total = 0.0;
        for (i, p) in pts.iter().enumerate() {
            if i > 0 {
                total += (p[0] - pts[i - 1][0]).hypot(p[1] - pts[i - 1][1]);
            }
            cum.push(total);
        }
        Path { pts, clear, cum }
    }

    pub fn length(&self) -> f64 {
        self.cum.last().copied().unwrap_or(0.0)
    }

    /// The segment holding arc length `u` (clamped), and how far along it.
    fn locate(&self, u: f64) -> (usize, f64) {
        let n = self.pts.len();
        if n < 2 {
            return (0, 0.0);
        }
        let u = u.clamp(0.0, self.length());
        let i = self.cum.partition_point(|c| *c <= u).clamp(1, n - 1) - 1;
        let len = self.cum[i + 1] - self.cum[i];
        (i, if len > 0.0 { ((u - self.cum[i]) / len).clamp(0.0, 1.0) } else { 0.0 })
    }

    /// The point and the tangent's angle at arc length `u` (clamped).
    pub fn at(&self, u: f64) -> (Pt, f64) {
        let n = self.pts.len();
        if n < 2 {
            return (self.pts.first().copied().unwrap_or([0.0; 2]), 0.0);
        }
        let (i, t) = self.locate(u);
        let (a, b) = (self.pts[i], self.pts[i + 1]);
        ([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t], (b[1] - a[1]).atan2(b[0] - a[0]))
    }

    pub fn clear_at(&self, u: f64) -> f64 {
        if self.clear.len() < 2 {
            return self.clear.first().copied().unwrap_or(0.0);
        }
        let (i, t) = self.locate(u);
        self.clear[i] + (self.clear[i + 1] - self.clear[i]) * t
    }

    /// The same path read the other way.
    pub fn reversed(&self) -> Path {
        Path::new(self.pts.iter().rev().copied().collect(), self.clear.iter().rev().copied().collect())
    }

    /// The path moved `d` to its left (negative: to its right) as it runs, with the room left either side.
    pub fn offset(&self, d: f64) -> Path {
        let n = self.pts.len();
        if n < 2 || d == 0.0 {
            return self.clone();
        }
        let pts = (0..n)
            .map(|i| {
                let (a, b) = (self.pts[i.saturating_sub(1)], self.pts[(i + 1).min(n - 1)]);
                let len = (b[0] - a[0]).hypot(b[1] - a[1]).max(1e-12);
                // Left of the way it runs, on a screen whose y grows downwards.
                let normal = [(b[1] - a[1]) / len, -(b[0] - a[0]) / len];
                [self.pts[i][0] + normal[0] * d, self.pts[i][1] + normal[1] * d]
            })
            .collect();
        Path::new(pts, self.clear.iter().map(|c| (c - d.abs()).max(0.0)).collect())
    }

    /// Every point and clearance times `k`.
    pub fn scaled(&self, k: f64) -> Path {
        Path::new(self.pts.iter().map(|p| [p[0] * k, p[1] * k]).collect(), self.clear.iter().map(|c| c * k).collect())
    }
}

impl Spine {
    pub fn path(&self) -> Path {
        Path::new(self.pts.clone(), self.clear.clone())
    }

    pub fn length(&self) -> f64 {
        self.pts.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum()
    }

    /// The point and the tangent's angle at arc length `u` (clamped).
    pub fn at(&self, u: f64) -> (Pt, f64) {
        self.path().at(u)
    }

    pub fn clear_at(&self, u: f64) -> f64 {
        self.path().clear_at(u)
    }

    /// The part of the spine within `half` of arc length `u` on either side.
    pub fn cut_around(&self, u: f64, half: f64) -> Option<Spine> {
        let path = self.path();
        let (from, to) = ((u - half).max(0.0), (u + half).min(path.length()));
        if to - from < 1e-9 {
            return None;
        }
        let mut pts = vec![path.at(from).0];
        let mut clear = vec![path.clear_at(from)];
        for (i, c) in path.cum.iter().enumerate() {
            if *c > from && *c < to {
                pts.push(path.pts[i]);
                clear.push(path.clear[i]);
            }
        }
        pts.push(path.at(to).0);
        clear.push(path.clear_at(to));
        Some(Spine { pts, clear })
    }

    /// The arc length of the point of the spine nearest `p`.
    pub fn nearest(&self, p: Pt) -> f64 {
        let mut best = (f64::MAX, 0.0);
        let mut run = 0.0;
        for w in self.pts.windows(2) {
            let (dx, dy) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
            let len = dx.hypot(dy);
            let t = if len > 0.0 { (((p[0] - w[0][0]) * dx + (p[1] - w[0][1]) * dy) / (len * len)).clamp(0.0, 1.0) } else { 0.0 };
            let d = (p[0] - (w[0][0] + t * dx)).hypot(p[1] - (w[0][1] + t * dy));
            if d < best.0 {
                best = (d, run + t * len);
            }
            run += len;
        }
        best.1
    }
}

#[derive(PartialEq)]
struct Cost(f64);
impl Eq for Cost {}
impl PartialOrd for Cost {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Cost {
    fn cmp(&self, o: &Self) -> Ordering {
        self.0.total_cmp(&o.0)
    }
}

/// Dijkstra over the inside cells from `from`, 8 neighbours; `weight(cell)` multiplies a step's length.
/// Returns the distance to every cell (infinite where not reached) and the cell each was reached from.
fn dijkstra(m: &Mask, from: usize, weight: impl Fn(usize) -> f64) -> (Vec<f64>, Vec<usize>) {
    let mut dist = vec![f64::INFINITY; m.inside.len()];
    let mut prev = vec![usize::MAX; m.inside.len()];
    let mut heap = BinaryHeap::new();
    dist[from] = 0.0;
    heap.push(Reverse((Cost(0.0), from)));
    while let Some(Reverse((Cost(d), c))) = heap.pop() {
        if d > dist[c] {
            continue;
        }
        let (ci, cj) = ((c % m.w) as isize, (c / m.w) as isize);
        for dj in -1..=1isize {
            for di in -1..=1isize {
                let (i, j) = (ci + di, cj + dj);
                if (di == 0 && dj == 0) || i < 0 || j < 0 || i >= m.w as isize || j >= m.h as isize {
                    continue;
                }
                let n = j as usize * m.w + i as usize;
                if !m.inside[n] {
                    continue;
                }
                let step = if di != 0 && dj != 0 { std::f64::consts::SQRT_2 } else { 1.0 };
                let nd = d + step * weight(n);
                if nd < dist[n] {
                    dist[n] = nd;
                    prev[n] = c;
                    heap.push(Reverse((Cost(nd), n)));
                }
            }
        }
    }
    (dist, prev)
}

fn farthest(dist: &[f64]) -> usize {
    farthest_of(dist, &|_| true)
}

/// `farthest` among the cells `keep` holds.
fn farthest_of(dist: &[f64], keep: &dyn Fn(usize) -> bool) -> usize {
    (0..dist.len()).filter(|&c| dist[c].is_finite() && keep(c)).max_by(|a, b| dist[*a].total_cmp(&dist[*b])).unwrap_or(0)
}

/// For each cell of `mask` beside water of `room` outside it, which piece of that water: a strait's mouths on the
/// seas it joins. Only seas: a
/// pocket of water past the feature's edge (the head of a bay its outline cuts across) is not one, and only the
/// pieces of the water outside that are at least `MOUTH_SHARE` of the biggest count.
fn mouths(mask: &Mask, room: &Mask) -> Vec<Option<usize>> {
    let n = mask.inside.len();
    let outside = |c: usize| room.inside[c] && !mask.inside[c];
    let neighbours = |c: usize| {
        let (i, j) = ((c % mask.w) as isize, (c / mask.w) as isize);
        (-1..=1isize).flat_map(move |dj| (-1..=1isize).map(move |di| (i + di, j + dj))).filter(move |&(x, y)| (x, y) != (i, j) && x >= 0 && y >= 0 && (x as usize) < mask.w && (y as usize) < mask.h).map(|(x, y)| y as usize * mask.w + x as usize)
    };
    // The pieces of the water outside, and their sizes.
    let mut piece = vec![usize::MAX; n];
    let mut sizes: Vec<usize> = Vec::new();
    for c in 0..n {
        if !outside(c) || piece[c] != usize::MAX {
            continue;
        }
        let k = sizes.len();
        let (mut stack, mut size) = (vec![c], 0);
        piece[c] = k;
        while let Some(x) = stack.pop() {
            size += 1;
            for y in neighbours(x) {
                if outside(y) && piece[y] == usize::MAX {
                    piece[y] = k;
                    stack.push(y);
                }
            }
        }
        sizes.push(size);
    }
    let biggest = sizes.iter().copied().max().unwrap_or(0);
    let sea = |c: usize| (piece[c] != usize::MAX && sizes[piece[c]] as f64 >= MOUTH_SHARE * biggest as f64).then_some(piece[c]);
    (0..n).map(|c| if mask.inside[c] { neighbours(c).find_map(sea) } else { None }).collect()
}

/// For each sea a strait opens on (`open`, from `mouths`), the middle of its mouth: of the longest run of the
/// strait's cells beside that sea, the one with the most room `d`. A shorter run beside the same sea (the head of
/// a bay the outline cuts across, joined to the sea round a spit the grid has lost) is not its mouth.
fn mouth_middles(mask: &Mask, open: &[Option<usize>], d: &[f32]) -> Vec<usize> {
    let n = open.len();
    let mut seen = vec![false; n];
    // Per sea: the longest run's size and its roomiest cell.
    let mut best: std::collections::HashMap<usize, (usize, usize)> = std::collections::HashMap::new();
    for c in 0..n {
        let Some(sea) = open[c] else { continue };
        if seen[c] {
            continue;
        }
        let (mut stack, mut run) = (vec![c], Vec::new());
        seen[c] = true;
        while let Some(x) = stack.pop() {
            run.push(x);
            let (i, j) = ((x % mask.w) as isize, (x / mask.w) as isize);
            for (di, dj) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                let (u, v) = (i + di, j + dj);
                if u < 0 || v < 0 || u as usize >= mask.w || v as usize >= mask.h {
                    continue;
                }
                let y = v as usize * mask.w + u as usize;
                if !seen[y] && open[y] == Some(sea) {
                    seen[y] = true;
                    stack.push(y);
                }
            }
        }
        let Some(&middle) = run.iter().max_by(|a, b| d[**a].total_cmp(&d[**b])) else { continue };
        if best.get(&sea).is_none_or(|(size, _)| run.len() > *size) {
            best.insert(sea, (run.len(), middle));
        }
    }
    let mut ends: Vec<usize> = best.into_values().map(|(_, c)| c).collect();
    ends.sort();
    ends
}

/// The least share of the biggest piece of water outside a strait that another must be to be a sea it opens on.
const MOUTH_SHARE: f64 = 0.05;

/// Path cells nearer the edge than this share of the roomiest cell are cut off the ends: where a feature narrows to a point the ridge curls into its corners.
const TRIM: f32 = 0.5;

/// The centreline of the piece `mask` holds (give it one connected piece: see `Mask::largest_component`).
pub fn spine(mask: &Mask) -> Option<Spine> {
    spine_in(mask, mask)
}

/// `spine`, with the room round the piece measured in `room` (the same grid): a strait's from the land alone,
/// over all the water round it, so that its mouths on the open sea are not taken for its narrowing ends and the
/// centreline runs from one sea to the other. `None` unless it opens on two seas (pieces of `room`'s water
/// outside it): a sound has one.
pub fn spine_in(mask: &Mask, room: &Mask) -> Option<Spine> {
    if (room.w, room.h, room.x0.to_bits(), room.y0.to_bits(), room.cell.to_bits()) != (mask.w, mask.h, mask.x0.to_bits(), mask.y0.to_bits(), mask.cell.to_bits()) {
        return None;
    }
    // The room in the piece alone: what lies round it is room, not more of it.
    let d: Vec<f32> = room.distance().into_iter().zip(&mask.inside).map(|(v, inside)| if *inside { v } else { 0.0 }).collect();
    let dmax = d.iter().copied().fold(0.0f32, f32::max);
    if dmax < 1.5 {
        return None;
    }
    let widest = d.iter().position(|v| *v == dmax)?;
    // The two ends: the cell farthest from the widest point, then the one farthest from that; in a room of its
    // own, among its mouths, so that a strait runs from one sea to the other and not into a bay off its side.
    // In a room of its own, the ends are the middles of its mouths on the two seas farthest apart.
    let ends = if std::ptr::eq(mask, room) { Vec::new() } else { mouth_middles(mask, &mouths(mask, room), &d) };
    if !std::ptr::eq(mask, room) && ends.len() < 2 {
        return None;
    }
    let (from_widest, _) = dijkstra(mask, widest, |_| 1.0);
    let (a, b) = if ends.len() >= 2 {
        let a = *ends.iter().max_by(|x, y| from_widest[**x].total_cmp(&from_widest[**y]))?;
        let (from_a, _) = dijkstra(mask, a, |_| 1.0);
        (a, *ends.iter().filter(|e| **e != a).max_by(|x, y| from_a[**x].total_cmp(&from_a[**y]))?)
    } else {
        let a = farthest(&from_widest);
        let (from_a, _) = dijkstra(mask, a, |_| 1.0);
        (a, farthest(&from_a))
    };
    if a == b {
        return None;
    }
    // The path between them keeps to the ridge: a step costs more the nearer the edge.
    let dm = dmax as f64;
    let (_, prev) = dijkstra(mask, a, |c| dm / (d[c] as f64).max(0.5));
    let mut path = vec![b];
    while *path.last()? != a {
        let p = prev[*path.last()?];
        if p == usize::MAX {
            return None;
        }
        path.push(p);
    }
    path.reverse();
    // Trim the ends, where the feature narrows to a point. Measured in a room of its own, it opens onto the sea
    // at its ends and narrows between: only the corners where the path runs into the land are cut.
    let peak = if std::ptr::eq(mask, room) {
        path.iter().map(|&c| d[c]).fold(0.0f32, f32::max)
    } else {
        let n = path.len();
        path[n / 10..(n - n / 10).max(n / 10 + 1)].iter().map(|&c| d[c]).fold(f32::MAX, f32::min)
    };
    let keep = |c: &usize| d[*c] >= TRIM * peak;
    let first = path.iter().position(keep)?;
    let last = path.iter().rposition(keep)?;
    if last <= first {
        return None;
    }
    let path = &path[first..=last];
    let pts: Vec<Pt> = path.iter().map(|&c| mask.centre(c % mask.w, c / mask.w)).collect();
    // Smooth the staircase of the grid, keeping both ends.
    let n = pts.len();
    let k = (n / 20).max(2);
    let smooth: Vec<Pt> = (0..n)
        .map(|i| {
            let kk = k.min(i).min(n - 1 - i);
            let window = &pts[i - kk..=i + kk];
            let s = window.iter().fold([0.0, 0.0], |s, p| [s[0] + p[0], s[1] + p[1]]);
            [s[0] / window.len() as f64, s[1] / window.len() as f64]
        })
        .collect();
    let mut pts = simplify(&smooth, 0.5 * mask.cell);
    let clear = |p: &Pt| mask.cell_at(*p).map_or(0.0, |(i, j)| d[mask.index(i, j)] as f64 * mask.cell);
    let mut clearance: Vec<f64> = pts.iter().map(clear).collect();
    if pts.last()?[0] < pts[0][0] {
        pts.reverse();
        clearance.reverse();
    }
    let s = Spine { pts, clear: clearance };
    (s.length() >= 2.0 * mask.cell).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector::ring_contains;

    /// An L: a band 10 units wide along the top and down the right side.
    fn l_shape() -> Vec<Pt> {
        vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [90.0, 100.0], [90.0, 10.0], [0.0, 10.0]]
    }

    fn bend_mask() -> Mask {
        let mut m = Mask::over([0.0, 0.0, 100.0, 100.0], 128);
        m.fill(&[l_shape()], true);
        m.largest_component()
    }

    #[test]
    fn a_strait_runs_from_one_sea_to_the_other() {
        // Land above and below a channel 20 wide from x = 30 to 70; open sea either side of it. The strait is the
        // channel alone: measured in itself it narrows at both mouths, measured from the land it does not.
        let land = vec![vec![[30.0, 0.0], [70.0, 0.0], [70.0, 40.0], [30.0, 40.0]], vec![[30.0, 60.0], [70.0, 60.0], [70.0, 100.0], [30.0, 100.0]]];
        let mut strait = Mask::over([0.0, 0.0, 100.0, 100.0], 100);
        strait.fill(&[vec![[30.0, 40.0], [70.0, 40.0], [70.0, 60.0], [30.0, 60.0]]], true);
        let mut room = strait.empty_like();
        room.fill(&[vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]]], true);
        let mut ashore = strait.empty_like();
        ashore.fill(&land, true);
        room.minus(&ashore);
        let alone = spine(&strait).unwrap();
        let open = spine_in(&strait, &room).unwrap();
        let span = |s: &Spine| s.pts.last().unwrap()[0] - s.pts[0][0];
        assert!(span(&open) > 37.0, "mouth to mouth: {:?}", open.pts);
        assert!(span(&open) > span(&alone) + 5.0, "{} against {}", span(&open), span(&alone));
    }

    #[test]
    fn a_bent_channel_has_a_spine_inside_it_that_follows_the_bend() {
        let s = spine(&bend_mask()).expect("a spine");
        let ring = l_shape();
        assert!(s.pts.iter().all(|p| ring_contains(&ring, *p)), "every spine point is in the channel: {:?}", s.pts);
        assert!(s.pts[0][0] <= s.pts.last().unwrap()[0], "reads left to right");
        assert!(s.pts[0][0] < 20.0 && s.pts.last().unwrap()[1] > 80.0, "from the west end of the top band down the right band: {:?}", (s.pts[0], s.pts.last()));
        // The first and last stretches are about 90 degrees apart.
        let (_, a0) = s.at(5.0);
        let (_, a1) = s.at(s.length() - 5.0);
        let mut turn = (a1 - a0).abs();
        if turn > std::f64::consts::PI {
            turn = std::f64::consts::TAU - turn;
        }
        assert!((turn - std::f64::consts::FRAC_PI_2).abs() < 0.3, "turns by {} degrees", turn.to_degrees());
        // 10 units wide: about 5 either side, a bit less at the cell level.
        assert!(s.clear.iter().all(|c| *c > 2.0 && *c < 8.0), "{:?}", s.clear);
        assert_eq!(s.pts.len(), s.clear.len());
    }

    #[test]
    fn a_bar_read_from_either_end_gives_the_same_spine_left_to_right() {
        let mut m = Mask::over([0.0, 0.0, 60.0, 8.0], 120);
        m.fill(&[vec![[0.0, 0.0], [60.0, 0.0], [60.0, 8.0], [0.0, 8.0]]], true);
        let s = spine(&m).unwrap();
        assert!(s.pts[0][0] < s.pts.last().unwrap()[0]);
        assert!((s.length() - 60.0).abs() < 12.0, "{}", s.length());
        assert!(s.pts.iter().all(|p| (p[1] - 4.0).abs() < 2.5), "the ends may curl a little into the corners: {:?}", s.pts);
    }

    #[test]
    fn a_speck_has_no_spine_and_arc_length_queries_clamp() {
        let mut m = Mask::over([0.0, 0.0, 1.0, 1.0], 128);
        m.fill(&[vec![[0.0, 0.0], [0.01, 0.0], [0.01, 0.01], [0.0, 0.01]]], true);
        assert!(spine(&m).is_none());
        let s = Spine { pts: vec![[0.0, 0.0], [10.0, 0.0]], clear: vec![1.0, 3.0] };
        assert_eq!(s.at(-5.0).0, [0.0, 0.0]);
        assert_eq!(s.at(99.0).0, [10.0, 0.0]);
        assert_eq!(s.clear_at(5.0), 2.0);
        assert_eq!(s.nearest([4.0, 7.0]), 4.0);
        let cut = s.cut_around(5.0, 2.0).unwrap();
        assert_eq!(cut.pts.first().unwrap(), &[3.0, 0.0]);
        assert_eq!(cut.pts.last().unwrap(), &[7.0, 0.0]);
    }
}
