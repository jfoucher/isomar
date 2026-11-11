//! Plain vector geometry in lon/lat degrees, with line simplification.

/// A longitude/latitude pair, as `[lon, lat]`.
pub type Pt = [f64; 2];

/// A closed or open sequence of points.
pub type Ring = Vec<Pt>;

/// Axis-aligned bounds, `[min_lon, min_lat, max_lon, max_lat]`.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Bounds {
    pub min_lon: f64,
    pub min_lat: f64,
    pub max_lon: f64,
    pub max_lat: f64,
}

impl Bounds {
    pub const EMPTY: Bounds = Bounds {
        min_lon: f64::INFINITY,
        min_lat: f64::INFINITY,
        max_lon: f64::NEG_INFINITY,
        max_lat: f64::NEG_INFINITY,
    };

    pub fn of(points: &[Pt]) -> Bounds {
        let mut b = Bounds::EMPTY;
        for p in points {
            b.extend(*p);
        }
        b
    }

    pub fn extend(&mut self, p: Pt) {
        self.min_lon = self.min_lon.min(p[0]);
        self.max_lon = self.max_lon.max(p[0]);
        self.min_lat = self.min_lat.min(p[1]);
        self.max_lat = self.max_lat.max(p[1]);
    }

    pub fn union(&mut self, o: &Bounds) {
        self.min_lon = self.min_lon.min(o.min_lon);
        self.max_lon = self.max_lon.max(o.max_lon);
        self.min_lat = self.min_lat.min(o.min_lat);
        self.max_lat = self.max_lat.max(o.max_lat);
    }

    pub fn is_empty(&self) -> bool {
        self.min_lon > self.max_lon || self.min_lat > self.max_lat
    }

    pub fn intersects(&self, o: &Bounds) -> bool {
        self.min_lon <= o.max_lon
            && self.max_lon >= o.min_lon
            && self.min_lat <= o.max_lat
            && self.max_lat >= o.min_lat
    }

    pub fn contains(&self, p: Pt) -> bool {
        p[0] >= self.min_lon && p[0] <= self.max_lon && p[1] >= self.min_lat && p[1] <= self.max_lat
    }

    pub fn width(&self) -> f64 {
        self.max_lon - self.min_lon
    }

    pub fn height(&self) -> f64 {
        self.max_lat - self.min_lat
    }

    pub fn expanded(&self, by: f64) -> Bounds {
        Bounds {
            min_lon: self.min_lon - by,
            min_lat: self.min_lat - by,
            max_lon: self.max_lon + by,
            max_lat: self.max_lat + by,
        }
    }
}

/// A polygon with holes. Rings are filled with the even-odd rule, so ring
/// orientation does not matter.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Polygon {
    pub rings: Vec<Ring>,
}

impl Polygon {
    pub fn bounds(&self) -> Bounds {
        let mut b = Bounds::EMPTY;
        for r in &self.rings {
            b.union(&Bounds::of(r));
        }
        b
    }

    /// Even-odd point-in-polygon test over all rings.
    pub fn contains(&self, p: Pt) -> bool {
        let mut inside = false;
        for ring in &self.rings {
            if ring_contains(ring, p) {
                inside = !inside;
            }
        }
        inside
    }
}

/// Ray casting test for one ring.
pub fn ring_contains(ring: &[Pt], p: Pt) -> bool {
    let mut inside = false;
    let n = ring.len();
    if n < 3 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (ring[i], ring[j]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// A point inside an area (its rings, holes included, by the even-odd rule) to
/// write its name at: the middle of the widest run inside along one of a few
/// parallels, the central one preferred. `None` for an area with no inside.
pub fn label_point(rings: &[Ring]) -> Option<Pt> {
    let b = rings.iter().fold(Bounds::EMPTY, |mut b, r| {
        r.iter().for_each(|p| b.extend(*p));
        b
    });
    if b.is_empty() {
        return None;
    }
    let mut best: Option<(f64, Pt)> = None;
    for f in [0.5, 0.4, 0.6, 0.3, 0.7, 0.2, 0.8] {
        let lat = b.min_lat + b.height() * f;
        let mut xs: Vec<f64> = Vec::new();
        for r in rings {
            let n = r.len();
            for i in 0..n {
                let (a, c) = (r[i], r[(i + 1) % n]);
                if (a[1] > lat) != (c[1] > lat) {
                    xs.push(a[0] + (lat - a[1]) * (c[0] - a[0]) / (c[1] - a[1]));
                }
            }
        }
        xs.sort_by(f64::total_cmp);
        // Off the middle a run must be clearly wider to win.
        let weight = 1.0 - (f - 0.5f64).abs();
        for &[a, c] in xs.as_chunks::<2>().0 {
            let w = (c - a) * weight;
            if best.is_none_or(|(bw, _)| w > bw) {
                best = Some((w, [(a + c) / 2.0, lat]));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// Signed area of a ring in square degrees; positive when counter-clockwise.
pub fn signed_area(ring: &[Pt]) -> f64 {
    let mut s = 0.0;
    let n = ring.len();
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        s += a[0] * b[1] - b[0] * a[1];
    }
    s / 2.0
}

fn perp_dist_sq(p: Pt, a: Pt, b: Pt) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        let (ex, ey) = (p[0] - a[0], p[1] - a[1]);
        return ex * ex + ey * ey;
    }
    let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0);
    let (cx, cy) = (a[0] + t * dx, a[1] + t * dy);
    let (ex, ey) = (p[0] - cx, p[1] - cy);
    ex * ex + ey * ey
}

/// Douglas-Peucker simplification keeping the end points. `tol` is in the same
/// unit as the coordinates. Iterative, so very long lines are fine.
pub fn simplify(points: &[Pt], tol: f64) -> Vec<Pt> {
    let n = points.len();
    if n <= 2 || tol <= 0.0 {
        return points.to_vec();
    }
    let tol2 = tol * tol;
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0usize, n - 1)];
    while let Some((lo, hi)) = stack.pop() {
        if hi <= lo + 1 {
            continue;
        }
        let (mut far, mut far_d) = (lo, 0.0);
        for i in lo + 1..hi {
            let d = perp_dist_sq(points[i], points[lo], points[hi]);
            if d > far_d {
                far_d = d;
                far = i;
            }
        }
        if far_d > tol2 {
            keep[far] = true;
            stack.push((lo, far));
            stack.push((far, hi));
        }
    }
    points.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

/// Simplifies a closed ring. Returns `None` when it collapses to fewer than
/// four points (a closed triangle), so tiny islands vanish at coarse zoom.
pub fn simplify_ring(ring: &[Pt], tol: f64) -> Option<Ring> {
    let b = Bounds::of(ring);
    if b.width().max(b.height()) < tol {
        return None;
    }
    let s = simplify(ring, tol);
    (s.len() >= 4).then_some(s)
}

/// Clips a ring to `b` (Sutherland-Hodgman), for filling with the even-odd
/// rule. The points the clip adds lie exactly on `b`'s edges, so the cut can
/// be told from the coast (`on_clip_edge`). None when nothing is left.
pub fn clip_ring(ring: &[Pt], b: &Bounds) -> Option<Ring> {
    let rb = Bounds::of(ring);
    if !rb.intersects(b) {
        return None;
    }
    if b.contains([rb.min_lon, rb.min_lat]) && b.contains([rb.max_lon, rb.max_lat]) {
        return Some(ring.to_vec());
    }
    let mut pts = ring.to_vec();
    // West, east, south, north: (axis, limit, keep points above it).
    for (axis, limit, above) in [(0, b.min_lon, true), (0, b.max_lon, false), (1, b.min_lat, true), (1, b.max_lat, false)] {
        let inside = |p: &Pt| if above { p[axis] >= limit } else { p[axis] <= limit };
        let cross = |a: &Pt, c: &Pt| {
            let t = (limit - a[axis]) / (c[axis] - a[axis]);
            let mut p = [a[0] + t * (c[0] - a[0]), a[1] + t * (c[1] - a[1])];
            p[axis] = limit;
            p
        };
        let mut out = Vec::with_capacity(pts.len() + 4);
        for i in 0..pts.len() {
            let (prev, cur) = (&pts[(i + pts.len() - 1) % pts.len()], &pts[i]);
            match (inside(prev), inside(cur)) {
                (true, true) => out.push(*cur),
                (true, false) => out.push(cross(prev, cur)),
                (false, true) => {
                    out.push(cross(prev, cur));
                    out.push(*cur);
                }
                (false, false) => {}
            }
        }
        pts = out;
        if pts.len() < 3 {
            return None;
        }
    }
    Some(pts)
}

/// Whether the segment `a`-`c` runs along an edge of `b`: a cut made by
/// `clip_ring`, not coast.
pub fn on_clip_edge(a: Pt, c: Pt, b: &Bounds) -> bool {
    (a[0] == c[0] && (a[0] == b.min_lon || a[0] == b.max_lon)) || (a[1] == c[1] && (a[1] == b.min_lat || a[1] == b.max_lat))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_goes_inside_its_area_even_a_c_shaped_one() {
        // A C open to the east: its centre is outside.
        let c: Ring = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 2.0], [2.0, 2.0], [2.0, 8.0], [10.0, 8.0], [10.0, 10.0], [0.0, 10.0], [0.0, 0.0]];
        let p = label_point(std::slice::from_ref(&c)).unwrap();
        assert!(ring_contains(&c, p), "{p:?}");
        // A square with a hole in the middle: the name goes beside the hole.
        let outer: Ring = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0], [0.0, 0.0]];
        let hole: Ring = vec![[2.0, 2.0], [8.0, 2.0], [8.0, 8.0], [2.0, 8.0], [2.0, 2.0]];
        let p = label_point(&[outer.clone(), hole.clone()]).unwrap();
        assert!(ring_contains(&outer, p) && !ring_contains(&hole, p), "{p:?}");
        assert_eq!(label_point(&[outer]), Some([5.0, 5.0]));
        assert_eq!(label_point(&[]), None);
    }

    fn area(ring: &[Pt]) -> f64 {
        signed_area(ring).abs()
    }

    #[test]
    fn a_ring_is_clipped_to_the_box() {
        let b = Bounds { min_lon: 0.0, min_lat: 0.0, max_lon: 10.0, max_lat: 10.0 };
        // Inside: untouched. Outside: gone.
        let inner = vec![[1.0, 1.0], [2.0, 1.0], [2.0, 2.0], [1.0, 1.0]];
        assert_eq!(clip_ring(&inner, &b), Some(inner.clone()));
        assert_eq!(clip_ring(&[[20.0, 20.0], [21.0, 20.0], [21.0, 21.0]], &b), None);
        // A square across the east edge keeps the part inside.
        let across = vec![[5.0, 2.0], [15.0, 2.0], [15.0, 8.0], [5.0, 8.0], [5.0, 2.0]];
        let c = clip_ring(&across, &b).unwrap();
        assert!((area(&c) - 30.0).abs() < 1e-9);
        assert!(c.iter().all(|p| p[0] <= 10.0));
        // The cut is along the edge and is told from the rest.
        let cuts = (0..c.len()).filter(|&i| on_clip_edge(c[i], c[(i + 1) % c.len()], &b)).count();
        assert_eq!(cuts, 1);
        // A concave ring (a U open to the north) keeps both arms when the box cuts its base.
        let u = vec![[2.0, -5.0], [8.0, -5.0], [8.0, 5.0], [6.0, 5.0], [6.0, -2.0], [4.0, -2.0], [4.0, 5.0], [2.0, 5.0]];
        let c = clip_ring(&u, &b).unwrap();
        assert!((area(&c) - 20.0).abs() < 1e-9, "two 2 x 5 arms, got {}", area(&c));
    }

    #[test]
    fn simplify_drops_collinear_points_and_keeps_corners() {
        let line = vec![[0.0, 0.0], [1.0, 0.0001], [2.0, 0.0], [2.0, 2.0]];
        let s = simplify(&line, 0.01);
        assert_eq!(s, vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0]]);
    }

    #[test]
    fn simplify_keeps_everything_with_zero_tolerance() {
        let line = vec![[0.0, 0.0], [1.0, 0.5], [2.0, 0.0]];
        assert_eq!(simplify(&line, 0.0), line);
    }

    #[test]
    fn tiny_rings_disappear_when_coarse() {
        let ring = vec![[0.0, 0.0], [0.001, 0.0], [0.001, 0.001], [0.0, 0.001], [0.0, 0.0]];
        assert!(simplify_ring(&ring, 0.01).is_none());
        assert!(simplify_ring(&ring, 0.0001).is_some());
    }

    #[test]
    fn polygon_with_hole_uses_even_odd() {
        let outer = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0], [0.0, 0.0]];
        let hole = vec![[4.0, 4.0], [6.0, 4.0], [6.0, 6.0], [4.0, 6.0], [4.0, 4.0]];
        let poly = Polygon { rings: vec![outer, hole] };
        assert!(poly.contains([1.0, 1.0]));
        assert!(!poly.contains([5.0, 5.0]));
        assert!(!poly.contains([11.0, 5.0]));
    }

    #[test]
    fn orientation_sign() {
        let ccw = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        assert!(signed_area(&ccw) > 0.0);
        let cw: Vec<Pt> = ccw.iter().rev().copied().collect();
        assert!(signed_area(&cw) < 0.0);
    }

    #[test]
    fn bounds_operations() {
        let mut b = Bounds::of(&[[1.0, 2.0], [3.0, 5.0]]);
        assert_eq!((b.width(), b.height()), (2.0, 3.0));
        assert!(b.contains([2.0, 3.0]));
        b.union(&Bounds::of(&[[-1.0, 0.0]]));
        assert_eq!(b.min_lon, -1.0);
        assert!(Bounds::EMPTY.is_empty());
        assert!(!b.intersects(&Bounds::of(&[[10.0, 10.0], [11.0, 11.0]])));
    }
}
