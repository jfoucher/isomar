//! Which way a coast runs, for writing a cape's name beside it, and which way a cape points, for writing
//! its name out from its tip as a paper chart does.

use crate::vector::{Pt, ring_contains};

/// The coast's direction near `at` (radians, folded into (-π/2, π/2]) and the unit normal pointing to the sea, from land rings in units.
pub fn coast_at(at: Pt, land: &[Vec<Pt>], radius: f64) -> Option<(f64, Pt)> {
    let (mut sc, mut ss, mut total) = (0.0, 0.0, 0.0);
    for ring in land {
        let n = ring.len();
        for i in 0..n {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len = dx.hypot(dy);
            if len == 0.0 {
                continue;
            }
            let t = (((at[0] - a[0]) * dx + (at[1] - a[1]) * dy) / (len * len)).clamp(0.0, 1.0);
            if (at[0] - (a[0] + t * dx)).hypot(at[1] - (a[1] + t * dy)) > radius {
                continue;
            }
            // Directions are lines, not arrows: average the doubled angle.
            let theta = dy.atan2(dx);
            sc += len * (2.0 * theta).cos();
            ss += len * (2.0 * theta).sin();
            total += len;
        }
    }
    if total == 0.0 {
        return None;
    }
    // Folded into (-π/2, π/2].
    let theta = ss.atan2(sc) / 2.0;
    let mut normal = [-theta.sin(), theta.cos()];
    let probe = |n: Pt| [at[0] + n[0] * radius / 2.0, at[1] + n[1] * radius / 2.0];
    let on_land = |p: Pt| land.iter().fold(false, |inside, r| inside ^ ring_contains(r, p));
    if on_land(probe(normal)) {
        normal = [-normal[0], -normal[1]];
    }
    Some((theta, normal))
}

/// A cape's tip, on the coast, and the unit direction its point runs out to sea, in units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CapeAxis {
    pub tip: Pt,
    pub out: Pt,
}

/// Samples across the disc a cape's axis is measured over.
const AXIS_SAMPLES: usize = 11;
/// Samples across the smaller disc its tip is looked for in.
const TIP_SAMPLES: usize = 9;

/// The middles of the land and of the sea within `radius` of `at`, from a lattice of `n` by `n`
/// samples over the disc; `None` unless there is some of both.
fn middles(at: Pt, radius: f64, n: usize, is_sea: &dyn Fn(Pt) -> bool) -> Option<(Pt, Pt)> {
    let (mut land, mut sea) = (([0.0, 0.0], 0usize), ([0.0, 0.0], 0usize));
    for i in 0..n {
        for j in 0..n {
            let (u, v) = (2.0 * i as f64 / (n - 1) as f64 - 1.0, 2.0 * j as f64 / (n - 1) as f64 - 1.0);
            if u * u + v * v > 1.0 {
                continue;
            }
            let p = [at[0] + u * radius, at[1] + v * radius];
            let m = if is_sea(p) { &mut sea } else { &mut land };
            m.0 = [m.0[0] + p[0], m.0[1] + p[1]];
            m.1 += 1;
        }
    }
    (land.1 > 0 && sea.1 > 0).then(|| ([land.0[0] / land.1 as f64, land.0[1] / land.1 as f64], [sea.0[0] / sea.1 as f64, sea.0[1] / sea.1 as f64]))
}

/// The unit direction from the middle of the land round `at` to the middle of the sea: along a point,
/// out of its end; square to a straight shore. It holds on a sharp tip, where the two sides' directions
/// (`coast_at`) cancel out.
fn seaward(at: Pt, radius: f64, n: usize, is_sea: &dyn Fn(Pt) -> bool) -> Option<Pt> {
    let (land, sea) = middles(at, radius, n, is_sea)?;
    let (dx, dy) = (sea[0] - land[0], sea[1] - land[1]);
    let len = dx.hypot(dy);
    (len > radius * 1e-3).then(|| [dx / len, dy / len])
}

/// Samples round each circle `square_out` looks along.
const ARC_SAMPLES: usize = 96;
/// The circles round a tip that `square_out` looks along, as shares of the axis's radius.
const ARC_RADII: [f64; 3] = [0.2, 0.3, 0.4];

/// The unit direction square to the coast at `tip` (on the shore), out to sea: the middle of the widest stretch
/// of sea on a few small circles round it. That is square to a straight shore and along a narrow point, and,
/// unlike the middles of a wide disc, a bay or a headland a little way along the coast does not turn it.
fn square_out(tip: Pt, radius: f64, is_sea: &dyn Fn(Pt) -> bool) -> Option<Pt> {
    let mut sum = [0.0, 0.0];
    for share in ARC_RADII {
        let rho = radius * share;
        let at = |k: usize| {
            let a = std::f64::consts::TAU * k as f64 / ARC_SAMPLES as f64;
            (a, is_sea([tip[0] + rho * a.cos(), tip[1] + rho * a.sin()]))
        };
        let wet: Vec<bool> = (0..ARC_SAMPLES).map(|k| at(k).1).collect();
        // The longest run of sea round the circle, from a sample of land so that no run is cut in two.
        let Some(dry) = wet.iter().position(|w| !w) else { continue };
        let (mut best, mut run) = ((0, 0), (0, 0));
        for i in 1..=ARC_SAMPLES {
            let k = (dry + i) % ARC_SAMPLES;
            if wet[k] {
                run = if run.1 == 0 { (k, 1) } else { (run.0, run.1 + 1) };
                if run.1 > best.1 {
                    best = run;
                }
            } else {
                run = (0, 0);
            }
        }
        if best.1 == 0 {
            continue;
        }
        let mid = at(best.0).0 + std::f64::consts::TAU * (best.1 - 1) as f64 / 2.0 / ARC_SAMPLES as f64;
        sum = [sum[0] + mid.cos(), sum[1] + mid.sin()];
    }
    let len = sum[0].hypot(sum[1]);
    (len > 1e-6).then(|| [sum[0] / len, sum[1] / len])
}

/// Where the land ends going from `from` (on land) along `dir`, within `reach`, by halving.
fn shore_along(from: Pt, dir: Pt, reach: f64, is_sea: &dyn Fn(Pt) -> bool) -> Pt {
    let at = |t: f64| [from[0] + dir[0] * t, from[1] + dir[1] * t];
    let (mut lo, mut hi) = (0.0, reach);
    if !is_sea(at(hi)) {
        return at(hi);
    }
    for _ in 0..16 {
        let mid = (lo + hi) / 2.0;
        if is_sea(at(mid)) { hi = mid } else { lo = mid }
    }
    at(lo)
}

/// A cape's tip and the way it points, from where it is said to be (`at`: an OSM node at the tip, an ENC
/// point inland of it, a rounded position off it), with `is_sea` telling sea from land, looked for within
/// `radius` (units). The tip is the land within `reach` of `at` (units, at most half the radius) that reaches
/// farthest out along the cape, so a point a little inland or off the end is moved to the end: an exact
/// position wants a small reach, which only puts it on the coast `is_sea` draws. `None` inland, on open
/// water, or where the land and the sea are balanced all round (a lake shore seen from its middle).
pub fn cape_axis(at: Pt, is_sea: &dyn Fn(Pt) -> bool, radius: f64, reach: f64) -> Option<CapeAxis> {
    let out = seaward(at, radius, AXIS_SAMPLES, is_sea)?;
    // The land within reach that reaches farthest along the cape.
    let r = reach.min(radius / 2.0);
    let mut best: Option<(f64, Pt)> = None;
    for i in 0..TIP_SAMPLES {
        for j in 0..TIP_SAMPLES {
            let (u, v) = (2.0 * i as f64 / (TIP_SAMPLES - 1) as f64 - 1.0, 2.0 * j as f64 / (TIP_SAMPLES - 1) as f64 - 1.0);
            if u * u + v * v > 1.0 {
                continue;
            }
            let p = [at[0] + u * r, at[1] + v * r];
            let d = (p[0] - at[0]) * out[0] + (p[1] - at[1]) * out[1];
            if !is_sea(p) && best.is_none_or(|(b, _)| d > b) {
                best = Some((d, p));
            }
        }
    }
    // A point narrower than the lattice: back along the cape from a position off its end.
    if is_sea(at) {
        let k = (1..=32).map(|k| 2.0 * r * k as f64 / 32.0).find(|t| !is_sea([at[0] - out[0] * t, at[1] - out[1] * t]));
        if let Some(t) = k
            && best.is_none_or(|(b, _)| -t > b)
        {
            best = Some((-t, [at[0] - out[0] * t, at[1] - out[1] * t]));
        }
    }
    let (_, land) = best?;
    let step = r * 2.0 / (TIP_SAMPLES - 1) as f64;
    let tip = shore_along(land, out, step * 1.5, is_sea);
    // Square to the coast at the tip, which the first look, over the whole disc, may have been well inland of
    // or turned off by a bay along the coast.
    let out = square_out(tip, radius, is_sea).unwrap_or(out);
    Some(CapeAxis { tip: shore_along(tip, out, step, is_sea), out })
}

/// Steps a cove is walked out along, over its radius.
const COVE_STEPS: usize = 32;

/// How far the water goes from `from` along `dir`, up to `most` in steps of `step`, and whether land ends it.
fn water_run(from: Pt, dir: Pt, most: f64, step: f64, is_sea: &dyn Fn(Pt) -> bool) -> (f64, bool) {
    let mut t = 0.0;
    while t + step <= most {
        if !is_sea([from[0] + dir[0] * (t + step), from[1] + dir[1] * (t + step)]) {
            return (t, true);
        }
        t += step;
    }
    (t, false)
}

/// A cove's mouth, on the water, and the unit direction out of it to sea, in units: a cape turned inside out,
/// whose name a paper chart writes the same way, from just off the mouth out to sea ("Cala Figuereta"). From
/// where it is said to be (`at`: an OSM node in the cove, the middle of its water, a point inland of a cove too
/// small for the coast `is_sea` draws), looked at within `radius` (units). `None` on open water or an open
/// shore: the water must be held by land on both sides across the way out, within `radius`.
pub fn cove_axis(at: Pt, is_sea: &dyn Fn(Pt) -> bool, radius: f64) -> Option<CapeAxis> {
    let step = radius / COVE_STEPS as f64;
    // On the water: the sample nearest `at` within a quarter of the radius.
    let start = if is_sea(at) {
        at
    } else {
        let r = radius / 4.0;
        let n = TIP_SAMPLES * 2 - 1;
        (0..n * n)
            .map(|k| {
                let (u, v) = (2.0 * (k % n) as f64 / (n - 1) as f64 - 1.0, 2.0 * (k / n) as f64 / (n - 1) as f64 - 1.0);
                [at[0] + u * r, at[1] + v * r]
            })
            .filter(|p| (p[0] - at[0]).hypot(p[1] - at[1]) <= r && is_sea(*p))
            .min_by(|a, b| (a[0] - at[0]).hypot(a[1] - at[1]).total_cmp(&(b[0] - at[0]).hypot(b[1] - at[1])))?
    };
    let out = seaward(start, radius, AXIS_SAMPLES, is_sea)?;
    let across = [-out[1], out[0]];
    // Whether land ends the water within `most` on both sides of `p`, across the way out, and how far it is.
    let held = |p: Pt, most: f64| {
        let ((a, la), (b, lb)) = (water_run(p, [-across[0], -across[1]], most, step, is_sea), water_run(p, across, most, step, is_sea));
        (la && lb).then_some(a + b + step)
    };
    let width = held(start, radius)?;
    // Out along the cove until it has opened to more than twice its width (no land within its width on a side),
    // or until land is ahead (a cove that bends).
    let mut mouth = start;
    for k in 1..=COVE_STEPS {
        let p = [start[0] + out[0] * step * k as f64, start[1] + out[1] * step * k as f64];
        if !is_sea(p) {
            break;
        }
        mouth = p;
        if held(p, width).is_none() {
            break;
        }
    }
    Some(CapeAxis { tip: mouth, out })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_coast_normal_points_to_the_sea() {
        let island: Vec<Pt> = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let (angle, n) = coast_at([10.0, 5.0], std::slice::from_ref(&island), 1.0).unwrap();
        assert!(n[0] > 0.99 && n[1].abs() < 0.1, "{n:?}");
        assert!((angle.abs() - std::f64::consts::FRAC_PI_2).abs() < 0.2, "the east coast runs north-south: {angle}");
        let (_, n) = coast_at([5.0, 0.0], std::slice::from_ref(&island), 1.0).unwrap();
        assert!(n[1] < -0.99, "the north shore (y grows southwards) faces up the screen: {n:?}");
        assert!(coast_at([50.0, 50.0], &[island], 1.0).is_none());
    }
    /// A square island with a point 1 wide running east from its east side to a round end at x 15.5.
    fn with_point(x: f64, y: f64) -> bool {
        let island = (0.0..=10.0).contains(&x) && (0.0..=10.0).contains(&y);
        let point = (10.0..=15.0).contains(&x) && (4.5..=5.5).contains(&y) || (x - 15.0).hypot(y - 5.0) <= 0.5;
        island || point
    }

    #[test]
    fn a_cape_points_out_along_its_point_from_its_tip() {
        let is_sea = |p: Pt| !with_point(p[0], p[1]);
        let a = cape_axis([15.4, 5.0], &is_sea, 3.0, 1.5).unwrap();
        assert!(a.out[0] > 0.95, "east, along the point: {:?}", a.out);
        assert!((a.tip[0] - 15.5).abs() < 0.15 && (a.tip[1] - 5.0).abs() < 0.3, "at its end: {:?}", a.tip);
    }

    #[test]
    fn a_cape_said_to_be_inland_or_off_its_end_is_moved_to_the_end() {
        let is_sea = |p: Pt| !with_point(p[0], p[1]);
        for at in [[14.0, 5.0], [16.1, 5.2]] {
            let a = cape_axis(at, &is_sea, 3.0, 1.5).unwrap();
            assert!((a.tip[0] - 15.5).abs() < 0.2, "{at:?} gave {:?}", a.tip);
        }
    }

    #[test]
    fn a_bay_beside_the_point_does_not_turn_it() {
        // A shore along y = 0, land south (y > 0), with a point 1 wide out to y = -3 at x = 0, and a bay 4
        // deep to its west only: the middles of the land and the sea lean towards the bay.
        let land = |p: Pt| {
            let shore = if p[0] < -1.0 && p[0] > -6.0 { 4.0 } else { 0.0 };
            p[1] > shore || (p[0].abs() <= 0.5 && p[1] > -3.0)
        };
        let a = cape_axis([0.0, -3.0], &|p| !land(p), 4.0, 2.0).unwrap();
        assert!(a.out[1] < -0.97, "up along the point: {:?}", a.out);
        // A corner of the coast: north-facing west of x = 0, east-facing south of y = 0. Square to it is the
        // bisector, north-east.
        let corner = |p: Pt| p[0] < 0.0 && p[1] > 0.0;
        let a = cape_axis([0.0, 0.0], &|p| !corner(p), 4.0, 2.0).unwrap();
        let ne = std::f64::consts::FRAC_1_SQRT_2;
        assert!((a.out[0] - ne).abs() < 0.08 && (a.out[1] + ne).abs() < 0.08, "{:?}", a.out);
    }

    #[test]
    fn a_straight_shore_points_square_to_it_and_inland_or_open_water_not_at_all() {
        // Land west of x = 0.
        let is_sea = |p: Pt| p[0] > 0.0;
        let a = cape_axis([0.0, 3.0], &is_sea, 2.0, 1.0).unwrap();
        assert!(a.out[0] > 0.99 && a.tip[0].abs() < 0.05, "{a:?}");
        assert!(cape_axis([-10.0, 3.0], &is_sea, 2.0, 1.0).is_none());
        assert!(cape_axis([10.0, 3.0], &is_sea, 2.0, 1.0).is_none());
    }

    /// Land south of y = 0 (y grows southwards), with a cove 2 wide and 4 deep cut into it at x = 0.
    fn with_cove(p: Pt) -> bool {
        p[1] > 0.0 && !(p[0].abs() < 1.0 && p[1] < 4.0)
    }

    #[test]
    fn a_cove_points_out_of_its_mouth() {
        let is_sea = |p: Pt| !with_cove(p);
        for at in [[0.0, 3.0], [0.3, 1.5]] {
            let a = cove_axis(at, &is_sea, 4.0).unwrap();
            assert!(a.out[1] < -0.95, "out, up the screen: {:?}", a.out);
            assert!(a.tip[1].abs() < 0.4 && a.tip[0].abs() < 0.5, "at its mouth: {a:?}");
        }
    }

    #[test]
    fn a_cove_said_to_be_ashore_is_found_on_its_water() {
        // A cove 0.6 wide, and a point just inside its east side.
        let land = |p: Pt| p[1] > 0.0 && !(p[0].abs() < 0.3 && p[1] < 4.0);
        let a = cove_axis([0.45, 2.5], &|p| !land(p), 4.0).unwrap();
        assert!(a.out[1] < -0.95 && a.tip[1].abs() < 0.4, "{a:?}");
    }

    #[test]
    fn open_water_a_straight_shore_or_a_wide_bay_is_no_cove() {
        let is_sea = |p: Pt| !with_cove(p);
        assert!(cove_axis([0.0, -6.0], &is_sea, 4.0).is_none(), "open water");
        assert!(cove_axis([10.0, -0.2], &is_sea, 4.0).is_none(), "a straight shore");
        // A bay 20 wide, looked at within 4.
        let bay = |p: Pt| !(p[1] > 0.0 && !(p[0].abs() < 10.0 && p[1] < 8.0));
        assert!(cove_axis([0.0, 5.0], &bay, 4.0).is_none(), "wider than it is looked at");
        assert!(cove_axis([0.0, 10.0], &is_sea, 4.0).is_none(), "inland");
    }
}
