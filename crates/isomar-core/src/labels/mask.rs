//! A grid of cells over a feature: which are inside it, how far from its edge, which piece is biggest.

use crate::vector::Pt;

/// A grid of cells over a box in units: what is inside a feature.
#[derive(Clone, Debug, PartialEq)]
pub struct Mask {
    pub x0: f64,
    pub y0: f64,
    pub cell: f64,
    pub w: usize,
    pub h: usize,
    pub inside: Vec<bool>,
}

impl Mask {
    /// A grid over `b` = [x0, y0, x1, y1] with `across` cells along its longer side and 2 empty cells of padding all round.
    pub fn over(b: [f64; 4], across: usize) -> Mask {
        let longer = (b[2] - b[0]).max(b[3] - b[1]).max(1e-9);
        let cell = longer / across.max(1) as f64;
        let (x0, y0) = (b[0] - 2.0 * cell, b[1] - 2.0 * cell);
        let w = ((b[2] - b[0]) / cell).ceil() as usize + 4;
        let h = ((b[3] - b[1]) / cell).ceil() as usize + 4;
        Mask { x0, y0, cell, w, h, inside: vec![false; w * h] }
    }

    /// Sets the cells whose centres are inside `rings` (even-odd over all the rings together) to `value`.
    /// A ring may be open or closed: the edge from its last point to its first counts.
    pub fn fill(&mut self, rings: &[Vec<Pt>], value: bool) {
        let mut rows: Vec<Vec<f64>> = vec![Vec::new(); self.h];
        let row_y = |j: usize| self.y0 + (j as f64 + 0.5) * self.cell;
        for ring in rings {
            let n = ring.len();
            if n < 2 {
                continue;
            }
            for i in 0..n {
                let (a, b) = (ring[i], ring[(i + 1) % n]);
                if a[1] == b[1] {
                    continue;
                }
                let (lo, hi) = (a[1].min(b[1]), a[1].max(b[1]));
                // The rows whose centre is in [lo, hi), widened by one against rounding and then tested for real.
                let first = (((lo - self.y0) / self.cell - 0.5).ceil() - 1.0).max(0.0);
                let last = (((hi - self.y0) / self.cell - 0.5).ceil() + 1.0).min(self.h as f64);
                if first >= last {
                    continue;
                }
                for (j, row) in rows.iter_mut().enumerate().take(last as usize).skip(first as usize) {
                    let y = row_y(j);
                    if (a[1] > y) != (b[1] > y) {
                        row.push(a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1]));
                    }
                }
            }
        }
        for (j, xs) in rows.iter_mut().enumerate() {
            xs.sort_by(f64::total_cmp);
            for pair in xs.as_chunks::<2>().0 {
                // Cells whose centre x is in [pair[0], pair[1]).
                let from = ((pair[0] - self.x0) / self.cell - 0.5).ceil().max(0.0) as usize;
                let to = (((pair[1] - self.x0) / self.cell - 0.5).ceil().max(0.0) as usize).min(self.w);
                for i in from..to {
                    self.inside[j * self.w + i] = value;
                }
            }
        }
    }

    /// Keeps a cell only where it is also inside `other` (same grid).
    pub fn and(&mut self, other: &Mask) {
        debug_assert_eq!((self.w, self.h), (other.w, other.h));
        for (a, b) in self.inside.iter_mut().zip(&other.inside) {
            *a &= *b;
        }
    }

    /// Keeps a cell only where it is not inside `other` (same grid).
    pub fn minus(&mut self, other: &Mask) {
        debug_assert_eq!((self.w, self.h), (other.w, other.h));
        for (a, b) in self.inside.iter_mut().zip(&other.inside) {
            *a &= !*b;
        }
    }

    /// The same grid with nothing inside.
    pub fn empty_like(&self) -> Mask {
        Mask { inside: vec![false; self.w * self.h], ..self.clone() }
    }

    pub fn count(&self) -> usize {
        self.inside.iter().filter(|c| **c).count()
    }

    /// The cell's index in `inside`.
    pub fn index(&self, i: usize, j: usize) -> usize {
        j * self.w + i
    }

    /// The cell holding `p` (units), if on the grid.
    pub fn cell_at(&self, p: Pt) -> Option<(usize, usize)> {
        let (fx, fy) = ((p[0] - self.x0) / self.cell, (p[1] - self.y0) / self.cell);
        (fx >= 0.0 && fy >= 0.0 && (fx as usize) < self.w && (fy as usize) < self.h).then_some((fx as usize, fy as usize))
    }

    /// The largest 8-connected piece.
    pub fn largest_component(&self) -> Mask {
        let mut seen = vec![false; self.inside.len()];
        let mut best: Vec<usize> = Vec::new();
        let mut stack = Vec::new();
        for start in 0..self.inside.len() {
            if !self.inside[start] || seen[start] {
                continue;
            }
            let mut piece = Vec::new();
            seen[start] = true;
            stack.push(start);
            while let Some(c) = stack.pop() {
                piece.push(c);
                let (ci, cj) = ((c % self.w) as isize, (c / self.w) as isize);
                for dj in -1..=1isize {
                    for di in -1..=1isize {
                        let (i, j) = (ci + di, cj + dj);
                        if (di == 0 && dj == 0) || i < 0 || j < 0 || i >= self.w as isize || j >= self.h as isize {
                            continue;
                        }
                        let n = j as usize * self.w + i as usize;
                        if self.inside[n] && !seen[n] {
                            seen[n] = true;
                            stack.push(n);
                        }
                    }
                }
            }
            if piece.len() > best.len() {
                best = piece;
            }
        }
        let mut out = self.empty_like();
        for c in best {
            out.inside[c] = true;
        }
        out
    }

    /// The largest 8-connected piece that holds the cell `at` (units), or `None` when that cell is outside.
    pub fn component_at(&self, at: Pt) -> Option<Mask> {
        let (i, j) = self.cell_at(at)?;
        let start = self.index(i, j);
        if !self.inside[start] {
            return None;
        }
        let mut out = self.empty_like();
        let mut stack = vec![start];
        out.inside[start] = true;
        while let Some(c) = stack.pop() {
            let (ci, cj) = ((c % self.w) as isize, (c / self.w) as isize);
            for dj in -1..=1isize {
                for di in -1..=1isize {
                    let (i, j) = (ci + di, cj + dj);
                    if i < 0 || j < 0 || i >= self.w as isize || j >= self.h as isize {
                        continue;
                    }
                    let n = j as usize * self.w + i as usize;
                    if self.inside[n] && !out.inside[n] {
                        out.inside[n] = true;
                        stack.push(n);
                    }
                }
            }
        }
        Some(out)
    }

    /// Sets inside every piece of outside cells (8-connected) of at most `max_cells` that does not reach
    /// the grid's edge: islets in a gulf, which a water's name may be written over.
    pub fn fill_small_holes(&mut self, max_cells: usize) {
        let mut seen = vec![false; self.inside.len()];
        let mut stack = Vec::new();
        for start in 0..self.inside.len() {
            if self.inside[start] || seen[start] {
                continue;
            }
            let (mut piece, mut edge) = (Vec::new(), false);
            seen[start] = true;
            stack.push(start);
            while let Some(c) = stack.pop() {
                piece.push(c);
                let (ci, cj) = ((c % self.w) as isize, (c / self.w) as isize);
                edge |= ci == 0 || cj == 0 || ci == self.w as isize - 1 || cj == self.h as isize - 1;
                for dj in -1..=1isize {
                    for di in -1..=1isize {
                        let (i, j) = (ci + di, cj + dj);
                        if i < 0 || j < 0 || i >= self.w as isize || j >= self.h as isize {
                            continue;
                        }
                        let n = j as usize * self.w + i as usize;
                        if !self.inside[n] && !seen[n] {
                            seen[n] = true;
                            stack.push(n);
                        }
                    }
                }
            }
            if !edge && piece.len() <= max_cells {
                for c in piece {
                    self.inside[c] = true;
                }
            }
        }
    }

    /// Whether any inside cell is on the grid's outermost ring of cells.
    pub fn touches_edge(&self) -> bool {
        (0..self.w).any(|i| self.inside[i] || self.inside[(self.h - 1) * self.w + i]) || (0..self.h).any(|j| self.inside[j * self.w] || self.inside[j * self.w + self.w - 1])
    }

    /// Chamfer (3-4) distance from every inside cell to the nearest outside cell or grid edge, in cells; 0 outside.
    pub fn distance(&self) -> Vec<f32> {
        const BIG: u32 = u32::MAX / 4;
        let (w, h) = (self.w as isize, self.h as isize);
        let mut d: Vec<u32> = self.inside.iter().map(|&c| if c { BIG } else { 0 }).collect();
        // Beyond the grid counts as outside.
        let at = |d: &[u32], i: isize, j: isize| if i < 0 || j < 0 || i >= w || j >= h { 0 } else { d[(j * w + i) as usize] };
        for j in 0..h {
            for i in 0..w {
                let c = (j * w + i) as usize;
                if d[c] == 0 {
                    continue;
                }
                let v = [(-1, 0, 3), (-1, -1, 4), (0, -1, 3), (1, -1, 4)].iter().map(|&(di, dj, k)| at(&d, i + di, j + dj) + k).min().unwrap();
                d[c] = d[c].min(v);
            }
        }
        for j in (0..h).rev() {
            for i in (0..w).rev() {
                let c = (j * w + i) as usize;
                if d[c] == 0 {
                    continue;
                }
                let v = [(1, 0, 3), (1, 1, 4), (0, 1, 3), (-1, 1, 4)].iter().map(|&(di, dj, k)| at(&d, i + di, j + dj) + k).min().unwrap();
                d[c] = d[c].min(v);
            }
        }
        d.into_iter().map(|v| v as f32 / 3.0).collect()
    }

    /// A cell's centre in units.
    pub fn centre(&self, i: usize, j: usize) -> Pt {
        [self.x0 + (i as f64 + 0.5) * self.cell, self.y0 + (j as f64 + 0.5) * self.cell]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector::ring_contains;

    /// A small deterministic generator, so the tests need no crate for it.
    fn next(state: &mut u64) -> f64 {
        *state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (*state >> 11) as f64 / (1u64 << 53) as f64
    }

    #[test]
    fn fill_matches_ring_contains() {
        // A C open to the east.
        let c: Vec<Pt> = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 2.0], [2.0, 2.0], [2.0, 8.0], [10.0, 8.0], [10.0, 10.0], [0.0, 10.0]];
        let mut m = Mask::over([0.0, 0.0, 10.0, 10.0], 50);
        m.fill(std::slice::from_ref(&c), true);
        let mut s = 7;
        let mut checked = 0;
        for _ in 0..2000 {
            let p = [next(&mut s) * 12.0 - 1.0, next(&mut s) * 12.0 - 1.0];
            let Some((i, j)) = m.cell_at(p) else { continue };
            // Compare at the cell's own centre, which is what the mask tests.
            let centre = m.centre(i, j);
            assert_eq!(m.inside[m.index(i, j)], ring_contains(&c, centre), "{centre:?}");
            checked += 1;
        }
        assert!(checked > 1500);
        assert!(m.count() > 0);
    }

    #[test]
    fn fill_with_false_cuts_a_hole_and_holes_in_rings_are_even_odd() {
        let outer: Vec<Pt> = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let hole: Vec<Pt> = vec![[4.0, 4.0], [6.0, 4.0], [6.0, 6.0], [4.0, 6.0]];
        let mut m = Mask::over([0.0, 0.0, 10.0, 10.0], 20);
        m.fill(&[outer.clone(), hole.clone()], true);
        let at = |p: Pt| m.cell_at(p).map(|(i, j)| m.inside[m.index(i, j)]);
        assert_eq!(at([2.0, 2.0]), Some(true));
        assert_eq!(at([5.0, 5.0]), Some(false));
        let mut n = Mask::over([0.0, 0.0, 10.0, 10.0], 20);
        n.fill(&[outer], true);
        n.fill(&[hole], false);
        assert_eq!(n.inside, m.inside);
    }

    #[test]
    fn distance_is_largest_in_the_middle_of_a_bar() {
        let bar: Vec<Pt> = vec![[0.0, 0.0], [20.0, 0.0], [20.0, 6.0], [0.0, 6.0]];
        let mut m = Mask::over([0.0, 0.0, 20.0, 6.0], 40);
        m.fill(&[bar], true);
        let d = m.distance();
        let best = (0..d.len()).max_by(|a, b| d[*a].total_cmp(&d[*b])).unwrap();
        let (i, j) = (best % m.w, best / m.w);
        let c = m.centre(i, j);
        assert!((c[1] - 3.0).abs() <= m.cell, "the ridge is the bar's middle line, got {c:?}");
        assert!(d[best] > 5.0, "3 units of half-width at 0.5 per cell is about 6 cells: {}", d[best]);
        assert!(d.iter().zip(&m.inside).all(|(d, inside)| *inside || *d == 0.0));
        // The edge of the bar is next to outside: 1 cell.
        let (i, j) = m.cell_at([10.0, 0.1]).unwrap();
        assert!(d[m.index(i, j)] <= 1.01);
    }

    #[test]
    fn the_largest_piece_is_kept_and_found_from_a_point() {
        let a: Vec<Pt> = vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]];
        let b: Vec<Pt> = vec![[10.0, 0.0], [20.0, 0.0], [20.0, 10.0], [10.0, 10.0]];
        let mut m = Mask::over([0.0, 0.0, 20.0, 10.0], 40);
        m.fill(&[a, b], true);
        let big = m.largest_component();
        assert!(big.cell_at([15.0, 5.0]).is_some_and(|(i, j)| big.inside[big.index(i, j)]));
        assert!(big.cell_at([2.0, 2.0]).is_some_and(|(i, j)| !big.inside[big.index(i, j)]));
        let small = m.component_at([2.0, 2.0]).unwrap();
        assert!(small.count() < big.count() && small.count() > 0);
        assert!(m.component_at([8.0, 8.0]).is_none());
        assert!(!big.touches_edge(), "two cells of padding");
    }

    #[test]
    fn small_islands_are_filled_and_big_ones_and_the_shore_are_not() {
        let sea: Vec<Pt> = vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]];
        let islet: Vec<Pt> = vec![[3.0, 3.0], [4.0, 3.0], [4.0, 4.0], [3.0, 4.0]];
        let island: Vec<Pt> = vec![[8.0, 2.0], [16.0, 2.0], [16.0, 8.0], [8.0, 8.0]];
        let mut m = Mask::over([0.0, 0.0, 20.0, 10.0], 40);
        m.fill(&[sea, islet, island], true);
        m.fill_small_holes(20);
        let at = |p: Pt| m.cell_at(p).map(|(i, j)| m.inside[m.index(i, j)]);
        assert_eq!(at([3.5, 3.5]), Some(true), "the islet is water now");
        assert_eq!(at([12.0, 5.0]), Some(false), "the island stays");
        assert_eq!(at([-0.9, 5.0]), Some(false), "the padding round the grid reaches its edge");
    }
}
