//! Small statistics shared by the analysis: angles, means, medians and quantiles.

use crate::geo::normalize_bearing;

/// An angle wrapped to -180..180.
pub fn wrap180(a: f64) -> f64 {
    let r = normalize_bearing(a);
    if r > 180.0 { r - 360.0 } else { r }
}

/// The signed difference `a - b` of two directions, -180..180.
pub fn angle_diff(a: f64, b: f64) -> f64 {
    wrap180(a - b)
}

/// Sums of unit vectors, for averaging directions.
#[derive(Clone, Copy, Debug, Default)]
pub struct Circular {
    sin: f64,
    cos: f64,
    n: f64,
}

impl Circular {
    pub fn add(&mut self, deg: f64) {
        self.add_weighted(deg, 1.0);
    }

    pub fn add_weighted(&mut self, deg: f64, w: f64) {
        let r = deg.to_radians();
        self.sin += w * r.sin();
        self.cos += w * r.cos();
        self.n += w;
    }

    pub fn count(&self) -> f64 {
        self.n
    }

    /// The mean direction, 0..360, if there is one.
    pub fn mean(&self) -> Option<f64> {
        (self.n > 0.0 && self.sin.hypot(self.cos) > 1e-9 * self.n).then(|| normalize_bearing(self.sin.atan2(self.cos).to_degrees()))
    }

    /// The circular standard deviation in degrees.
    pub fn sd(&self) -> Option<f64> {
        if self.n <= 0.0 {
            return None;
        }
        let r = (self.sin.hypot(self.cos) / self.n).clamp(1e-12, 1.0);
        Some((-2.0 * r.ln()).sqrt().to_degrees())
    }
}

/// Running sums for a mean and standard deviation.
#[derive(Clone, Copy, Debug, Default)]
pub struct Linear {
    sum: f64,
    sq: f64,
    n: f64,
}

impl Linear {
    pub fn add(&mut self, x: f64) {
        self.sum += x;
        self.sq += x * x;
        self.n += 1.0;
    }

    pub fn count(&self) -> f64 {
        self.n
    }

    pub fn mean(&self) -> Option<f64> {
        (self.n > 0.0).then(|| self.sum / self.n)
    }

    pub fn sd(&self) -> Option<f64> {
        let m = self.mean()?;
        Some((self.sq / self.n - m * m).max(0.0).sqrt())
    }
}

/// The `q` quantile (0..1) of values, linearly interpolated; `None` when empty.
pub fn quantile(values: &[f64], q: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let pos = q.clamp(0.0, 1.0) * (v.len() - 1) as f64;
    let (i, f) = (pos.floor() as usize, pos.fract());
    Some(if i + 1 < v.len() { v[i] + (v[i + 1] - v[i]) * f } else { v[i] })
}

pub fn median(values: &[f64]) -> Option<f64> {
    quantile(values, 0.5)
}

/// The median of values each with a weight: the value where half the weight is on either side.
pub fn weighted_median(pairs: &[(f64, f64)]) -> Option<f64> {
    weighted_quantile(pairs, 0.5)
}

/// The `q` quantile of `(value, weight)` pairs: the value below which that share of the weight lies.
pub fn weighted_quantile(pairs: &[(f64, f64)], q: f64) -> Option<f64> {
    let mut v: Vec<(f64, f64)> = pairs.iter().copied().filter(|(x, w)| x.is_finite() && *w > 0.0).collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    let target = v.iter().map(|p| p.1).sum::<f64>() * q.clamp(0.0, 1.0);
    let mut acc = 0.0;
    for (x, w) in &v {
        acc += w;
        if acc >= target {
            return Some(*x);
        }
    }
    v.last().map(|p| p.0)
}

/// An AR(1) process: noise with a memory of `tau_s` seconds and a standard deviation of `sd`, for errors
/// that persist (a forecast wrong for hours, not for one sample).
pub struct Drift {
    x: f64,
    sd: f64,
    tau_s: f64,
}

impl Drift {
    pub fn new(sd: f64, tau_s: f64, rng: &mut super::rng::Rng) -> Drift {
        Drift { x: sd * rng.normal(), sd, tau_s }
    }

    pub fn step(&mut self, dt_s: f64, rng: &mut super::rng::Rng) -> f64 {
        let phi = (-dt_s / self.tau_s).exp();
        self.x = phi * self.x + (1.0 - phi * phi).sqrt() * self.sd * rng.normal();
        self.x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directions_average_across_north() {
        let mut c = Circular::default();
        c.add(350.0);
        c.add(10.0);
        assert!(c.mean().unwrap() < 1e-9 || c.mean().unwrap() > 359.999);
        assert!((c.sd().unwrap() - 10.0).abs() < 0.2);
        assert_eq!(angle_diff(5.0, 355.0), 10.0);
        assert_eq!(wrap180(270.0), -90.0);
    }

    #[test]
    fn quantiles_and_medians() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(quantile(&[0.0, 10.0], 0.85), Some(8.5));
        assert_eq!(quantile(&[], 0.5), None);
        assert_eq!(weighted_median(&[(1.0, 1.0), (2.0, 1.0), (10.0, 5.0)]), Some(10.0));
        let mut l = Linear::default();
        for x in [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0] {
            l.add(x);
        }
        assert_eq!((l.mean(), l.sd()), (Some(5.0), Some(2.0)));
    }
}
