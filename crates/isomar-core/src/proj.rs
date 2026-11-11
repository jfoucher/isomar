//! Map projections needed to read French source data.
//!
//! Only Lambert-93 (EPSG:2154) for now. RGF93 and WGS84 differ by well under a
//! metre, so no datum shift is applied.

use std::f64::consts::FRAC_PI_2;

const A: f64 = 6_378_137.0;
const F: f64 = 1.0 / 298.257_222_101;

/// Lambert conformal conic, two standard parallels.
pub struct Lcc {
    e: f64,
    n: f64,
    big_f: f64,
    rho0: f64,
    lon0: f64,
    x0: f64,
    y0: f64,
}

fn m(e: f64, phi: f64) -> f64 {
    phi.cos() / (1.0 - e * e * phi.sin().powi(2)).sqrt()
}

fn t(e: f64, phi: f64) -> f64 {
    let es = e * phi.sin();
    (FRAC_PI_2 / 2.0 - phi / 2.0).tan() / ((1.0 - es) / (1.0 + es)).powf(e / 2.0)
}

impl Lcc {
    pub fn new(lat0: f64, lon0: f64, lat1: f64, lat2: f64, x0: f64, y0: f64) -> Lcc {
        let e = (2.0 * F - F * F).sqrt();
        let (p0, p1, p2) = (lat0.to_radians(), lat1.to_radians(), lat2.to_radians());
        let (m1, m2) = (m(e, p1), m(e, p2));
        let (t0, t1, t2) = (t(e, p0), t(e, p1), t(e, p2));
        let n = (m1.ln() - m2.ln()) / (t1.ln() - t2.ln());
        let big_f = m1 / (n * t1.powf(n));
        Lcc { e, n, big_f, rho0: A * big_f * t0.powf(n), lon0: lon0.to_radians(), x0, y0 }
    }

    /// Lambert-93: RGF93 / Lambert-93, EPSG:2154.
    pub fn lambert93() -> Lcc {
        Lcc::new(46.5, 3.0, 49.0, 44.0, 700_000.0, 6_600_000.0)
    }

    /// Easting/northing in metres to `[lon, lat]` degrees.
    pub fn inverse(&self, x: f64, y: f64) -> [f64; 2] {
        let (dx, dy) = (x - self.x0, self.rho0 - (y - self.y0));
        let rho = self.n.signum() * (dx * dx + dy * dy).sqrt();
        let theta = (self.n.signum() * dx).atan2(self.n.signum() * dy);
        let tt = (rho / (A * self.big_f)).powf(1.0 / self.n);
        let mut phi = FRAC_PI_2 - 2.0 * tt.atan();
        for _ in 0..8 {
            let es = self.e * phi.sin();
            phi = FRAC_PI_2 - 2.0 * (tt * ((1.0 - es) / (1.0 + es)).powf(self.e / 2.0)).atan();
        }
        [(theta / self.n + self.lon0).to_degrees(), phi.to_degrees()]
    }

    /// `[lon, lat]` degrees to easting/northing in metres.
    pub fn forward(&self, lon: f64, lat: f64) -> [f64; 2] {
        let rho = A * self.big_f * t(self.e, lat.to_radians()).powf(self.n);
        let theta = self.n * (lon.to_radians() - self.lon0);
        [self.x0 + rho * theta.sin(), self.y0 + self.rho0 - rho * theta.cos()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_maps_to_false_easting_northing() {
        let l = Lcc::lambert93();
        let [x, y] = l.forward(3.0, 46.5);
        assert!((x - 700_000.0).abs() < 1e-6 && (y - 6_600_000.0).abs() < 1e-6);
        let [lon, lat] = l.inverse(700_000.0, 6_600_000.0);
        assert!((lon - 3.0).abs() < 1e-9 && (lat - 46.5).abs() < 1e-9);
    }

    #[test]
    fn roundtrip_across_france() {
        let l = Lcc::lambert93();
        for (lon, lat) in [(-4.49, 48.38), (2.35, 48.86), (9.2, 41.4), (7.7, 43.7), (-1.5, 43.4)] {
            let [x, y] = l.forward(lon, lat);
            let [lo, la] = l.inverse(x, y);
            assert!((lo - lon).abs() < 1e-9 && (la - lat).abs() < 1e-9, "{lon} {lat}");
        }
    }

    #[test]
    fn paris_is_where_it_should_be() {
        // Notre-Dame de Paris is at about 652 000 E, 6 862 000 N in Lambert-93.
        let [x, y] = Lcc::lambert93().forward(2.3499, 48.8530);
        assert!((x - 652_000.0).abs() < 2_000.0, "{x}");
        assert!((y - 6_862_000.0).abs() < 2_000.0, "{y}");
    }
}
