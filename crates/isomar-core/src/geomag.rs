//! Magnetic variation anywhere from the World Magnetic Model (NOAA/NCEI and the British Geological Survey,
//! public domain), for the parts of the map no chart cell gives it for. The coefficients are in
//! `data/geomag/WMM2025.COF`, embedded; the model is valid for five years from its epoch.

use std::sync::OnceLock;

const COEFFICIENTS: &str = include_str!("../../../data/geomag/WMM2025.COF");

/// Degree and order of the model.
const N_MAX: usize = 12;
/// The reference radius of the model's spherical harmonics, in km.
const RE_KM: f64 = 6371.2;
/// WGS84 semi-major axis in km and flattening.
const A_KM: f64 = 6378.137;
const FLAT: f64 = 1.0 / 298.257_223_563;

struct Model {
    epoch: f64,
    /// `g`, `h`, and their yearly change, at `[n][m]`.
    g: [[f64; N_MAX + 1]; N_MAX + 1],
    h: [[f64; N_MAX + 1]; N_MAX + 1],
    gt: [[f64; N_MAX + 1]; N_MAX + 1],
    ht: [[f64; N_MAX + 1]; N_MAX + 1],
}

fn model() -> &'static Model {
    static MODEL: OnceLock<Model> = OnceLock::new();
    MODEL.get_or_init(|| {
        let mut m = Model { epoch: 0.0, g: Default::default(), h: Default::default(), gt: Default::default(), ht: Default::default() };
        for (i, line) in COEFFICIENTS.lines().enumerate() {
            let f: Vec<&str> = line.split_whitespace().collect();
            if i == 0 {
                m.epoch = f.first().and_then(|e| e.parse().ok()).unwrap_or(2025.0);
            } else if f.len() == 6
                && let (Ok(n), Ok(mm)) = (f[0].parse::<usize>(), f[1].parse::<usize>())
                && n <= N_MAX
                && mm <= n
            {
                let v: Vec<f64> = f[2..].iter().map(|s| s.parse().unwrap_or(0.0)).collect();
                (m.g[n][mm], m.h[n][mm], m.gt[n][mm], m.ht[n][mm]) = (v[0], v[1], v[2], v[3]);
            }
        }
        m
    })
}

/// The first year the bundled model is valid for, and the last.
pub fn validity() -> (f64, f64) {
    let e = model().epoch;
    (e, e + 5.0)
}

/// A date as a decimal year.
pub fn decimal_year(date: chrono::NaiveDate) -> f64 {
    use chrono::Datelike;
    let days = if date.leap_year() { 366.0 } else { 365.0 };
    date.year() as f64 + (date.ordinal0() as f64 + 0.5) / days
}

/// Today as a decimal year.
pub fn today() -> f64 {
    decimal_year(chrono::Utc::now().date_naive())
}

/// The magnetic field at a place and time, in nanotesla: north, east and down.
fn field(lat_deg: f64, lon_deg: f64, year: f64) -> (f64, f64, f64) {
    let m = model();
    let dt = year - m.epoch;
    let (lat, lon) = (lat_deg.to_radians(), lon_deg.to_radians());
    // Geodetic to geocentric spherical coordinates, at the ellipsoid's surface.
    let e2 = FLAT * (2.0 - FLAT);
    let rc = A_KM / (1.0 - e2 * lat.sin().powi(2)).sqrt();
    let (p, z) = (rc * lat.cos(), rc * (1.0 - e2) * lat.sin());
    let r = p.hypot(z);
    let lat_c = (z / r).asin();
    let (c, s) = (lat_c.sin(), lat_c.cos()); // cos and sin of the colatitude
    // Associated Legendre functions, Schmidt semi-normalised, and their derivatives by colatitude.
    let mut pn = [[0.0f64; N_MAX + 1]; N_MAX + 1];
    let mut dp = [[0.0f64; N_MAX + 1]; N_MAX + 1];
    pn[0][0] = 1.0;
    for n in 1..=N_MAX {
        for mm in 0..=n {
            if n == mm {
                pn[n][n] = s * pn[n - 1][n - 1];
                dp[n][n] = s * dp[n - 1][n - 1] + c * pn[n - 1][n - 1];
            } else if n == 1 {
                pn[1][0] = c * pn[0][0];
                dp[1][0] = c * dp[0][0] - s * pn[0][0];
            } else if mm + 2 > n {
                pn[n][mm] = c * pn[n - 1][mm];
                dp[n][mm] = c * dp[n - 1][mm] - s * pn[n - 1][mm];
            } else {
                let k = (((n - 1) * (n - 1) - mm * mm) as f64) / ((2 * n - 1) * (2 * n - 3)) as f64;
                pn[n][mm] = c * pn[n - 1][mm] - k * pn[n - 2][mm];
                dp[n][mm] = c * dp[n - 1][mm] - s * pn[n - 1][mm] - k * dp[n - 2][mm];
            }
        }
    }
    // Schmidt semi-normalisation.
    let mut schmidt = [[1.0f64; N_MAX + 1]; N_MAX + 1];
    for n in 1..=N_MAX {
        schmidt[n][0] = schmidt[n - 1][0] * (2 * n - 1) as f64 / n as f64;
        for mm in 1..=n {
            schmidt[n][mm] = schmidt[n][mm - 1] * (((n - mm + 1) * if mm == 1 { 2 } else { 1 }) as f64 / (n + mm) as f64).sqrt();
        }
    }
    let (mut br, mut bt, mut bp) = (0.0, 0.0, 0.0);
    for n in 1..=N_MAX {
        let scale = (RE_KM / r).powi(n as i32 + 2);
        for mm in 0..=n {
            let g = m.g[n][mm] + dt * m.gt[n][mm];
            let h = m.h[n][mm] + dt * m.ht[n][mm];
            let (sm, cm) = (mm as f64 * lon).sin_cos();
            let (pv, dv) = (pn[n][mm] * schmidt[n][mm], dp[n][mm] * schmidt[n][mm]);
            br += scale * (n + 1) as f64 * (g * cm + h * sm) * pv;
            bt -= scale * (g * cm + h * sm) * dv;
            bp += scale * mm as f64 * (g * sm - h * cm) * pv / s;
        }
    }
    // North, east and down at the geocentric latitude, turned to the geodetic one.
    let (x, y, zz) = (-bt, bp, -br);
    let psi = lat_c - lat;
    (x * psi.cos() - zz * psi.sin(), y, x * psi.sin() + zz * psi.cos())
}

/// Magnetic variation in degrees at a place and decimal year, east positive, or `None` where the model has none:
/// outside the years it is valid for (a little over), and near the poles where a compass is no use.
pub fn variation(lat: f64, lon: f64, year: f64) -> Option<f64> {
    let (from, to) = validity();
    if !(from - 0.5..=to + 1.0).contains(&year) || lat.abs() > 89.0 {
        return None;
    }
    let (x, y, _) = field(lat, lon, year);
    Some(y.atan2(x).to_degrees())
}

/// How much the variation changes in a year, in degrees, east positive.
pub fn annual_change(lat: f64, lon: f64, year: f64) -> Option<f64> {
    let (a, b) = (variation(lat, lon, year)?, variation(lat, lon, year + 0.5)?);
    let d = b - a;
    Some((if d > 180.0 { d - 360.0 } else if d < -180.0 { d + 360.0 } else { d }) * 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// NOAA's published test values (WMM2025_TEST_VALUES.txt): date, latitude, longitude, then
    /// X, Y, Z (nT) and the declination (degrees), at sea level.
    const TEST_VALUES: &[(f64, f64, f64, f64, f64, f64, f64)] = &[
        (2025.0, 80.0, 0.0, 6521.6, 145.9, 54791.5, 1.28),
        (2025.0, 0.0, 120.0, 39677.8, -109.6, -10580.2, -0.16),
        (2025.0, -80.0, 240.0, 6117.5, 15751.9, -52022.5, 68.78),
        (2027.5, 80.0, 0.0, 6500.8, 294.5, 54869.4, 2.59),
        (2027.5, 0.0, 120.0, 39701.6, -167.4, -10381.8, -0.24),
        (2027.5, -80.0, 240.0, 6200.7, 15730.3, -51783.7, 68.49),
    ];

    #[test]
    fn the_model_gives_the_published_test_values() {
        for &(year, lat, lon, x, y, z, d) in TEST_VALUES {
            let (fx, fy, fz) = field(lat, lon, year);
            assert!((fx - x).abs() < 1.0 && (fy - y).abs() < 1.0 && (fz - z).abs() < 1.5, "at {lat} {lon}: got {fx:.1} {fy:.1} {fz:.1}, want {x} {y} {z}");
            let got = variation(lat, lon, year).unwrap();
            assert!((got - d).abs() < 0.02, "declination at {lat} {lon}: {got:.3}, want {d}");
        }
    }

    #[test]
    fn variation_in_the_places_sailors_know() {
        // Brittany's variation has been through zero (it was 4 west in 2000); the Bahamas are about 8 west.
        let brest = variation(48.39, -4.49, 2026.8).unwrap();
        assert!((-1.0..1.0).contains(&brest), "{brest}");
        let nassau = variation(25.0, -77.3, 2026.8).unwrap();
        assert!((-10.0..-5.0).contains(&nassau), "{nassau}");
        assert!(annual_change(48.39, -4.49, 2026.8).unwrap().abs() < 0.5);
    }

    #[test]
    fn no_value_outside_the_model_or_at_the_poles() {
        assert!(variation(48.0, -4.0, 2010.0).is_none());
        assert!(variation(48.0, -4.0, 2040.0).is_none());
        assert!(variation(89.9, 0.0, 2026.0).is_none());
        assert_eq!(validity().0, 2025.0);
    }

    #[test]
    fn a_date_is_a_decimal_year() {
        let d = decimal_year(chrono::NaiveDate::from_ymd_opt(2026, 7, 2).unwrap());
        assert!((d - 2026.5).abs() < 0.01, "{d}");
    }
}
