//! Geodesy on a sphere, plus the Web Mercator projection used by the canvas.
//!
//! Distances are in nautical miles and bearings in degrees true, as on a chart.

use std::f64::consts::PI;

/// Mean Earth radius in metres (IUGG).
pub const EARTH_RADIUS_M: f64 = 6_371_008.8;
/// Metres per nautical mile.
pub const NM_M: f64 = 1852.0;
/// Latitude limit of Web Mercator.
pub const MAX_LAT: f64 = 85.051_128_779_806_59;
/// Edge length of the zoom-0 world, in pixels.
pub const TILE_SIZE: f64 = 256.0;

const R_NM: f64 = EARTH_RADIUS_M / NM_M;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LatLon {
    pub lat: f64,
    pub lon: f64,
}

impl LatLon {
    pub const fn new(lat: f64, lon: f64) -> Self {
        Self { lat, lon }
    }

    /// Great-circle distance in nautical miles (haversine).
    pub fn distance_nm(&self, to: &LatLon) -> f64 {
        let (p1, p2) = (self.lat.to_radians(), to.lat.to_radians());
        let dp = p2 - p1;
        let dl = (to.lon - self.lon).to_radians();
        let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
        2.0 * R_NM * a.sqrt().asin()
    }

    /// Initial great-circle bearing to `to`, 0..360.
    pub fn bearing_to(&self, to: &LatLon) -> f64 {
        let (p1, p2) = (self.lat.to_radians(), to.lat.to_radians());
        let dl = (to.lon - self.lon).to_radians();
        let y = dl.sin() * p2.cos();
        let x = p1.cos() * p2.sin() - p1.sin() * p2.cos() * dl.cos();
        normalize_bearing(y.atan2(x).to_degrees())
    }

    /// Point reached from here along a great circle.
    pub fn destination(&self, bearing_deg: f64, distance_nm: f64) -> LatLon {
        let d = distance_nm / R_NM;
        let b = bearing_deg.to_radians();
        let p1 = self.lat.to_radians();
        let p2 = (p1.sin() * d.cos() + p1.cos() * d.sin() * b.cos()).asin();
        let dl = (b.sin() * d.sin() * p1.cos()).atan2(d.cos() - p1.sin() * p2.sin());
        LatLon::new(p2.to_degrees(), normalize_lon(self.lon + dl.to_degrees()))
    }

    /// `destination` for many legs from one point: `sin_cos_lat` is the sine and cosine of this point's
    /// latitude, and the bearing is given by its sine and cosine (the eastward and northward shares of the
    /// leg). The same formula with the same values, so the same result to rounding, with one `sin_cos`,
    /// one `asin` and one `atan2` instead of a dozen calls.
    pub fn destination_by(&self, (sin_lat, cos_lat): (f64, f64), sin_b: f64, cos_b: f64, distance_nm: f64) -> LatLon {
        let (sd, cd) = (distance_nm / R_NM).sin_cos();
        let sin_p2 = sin_lat * cd + cos_lat * sd * cos_b;
        let p2 = sin_p2.clamp(-1.0, 1.0).asin();
        let dl = (sin_b * sd * cos_lat).atan2(cd - sin_lat * sin_p2);
        LatLon::new(p2.to_degrees(), normalize_lon(self.lon + dl.to_degrees()))
    }

    /// Rhumb-line (constant bearing) distance and bearing to `to`.
    pub fn rhumb_to(&self, to: &LatLon) -> (f64, f64) {
        let (p1, p2) = (self.lat.to_radians(), to.lat.to_radians());
        let dp = p2 - p1;
        let mut dl = (to.lon - self.lon).to_radians();
        // Take the shorter way round the globe.
        if dl > PI {
            dl -= 2.0 * PI;
        } else if dl < -PI {
            dl += 2.0 * PI;
        }
        let dpsi = ((PI / 4.0 + p2 / 2.0).tan() / (PI / 4.0 + p1 / 2.0).tan()).ln();
        // On an east-west line dpsi vanishes; fall back to cos(lat).
        let q = if dpsi.abs() > 1e-12 { dp / dpsi } else { p1.cos() };
        let dist = (dp * dp + q * q * dl * dl).sqrt() * R_NM;
        let brg = normalize_bearing(dl.atan2(dpsi).to_degrees());
        (dist, brg)
    }
}

/// Wraps a bearing into 0..360.
pub fn normalize_bearing(deg: f64) -> f64 {
    deg.rem_euclid(360.0)
}

/// Wraps a longitude into -180..180.
pub fn normalize_lon(deg: f64) -> f64 {
    let l = (deg + 180.0).rem_euclid(360.0) - 180.0;
    if l == -180.0 && deg > 0.0 { 180.0 } else { l }
}

/// A wind of `kn` blowing from `from` degrees true, as the east and north components of the air's motion.
pub fn wind_uv(kn: f64, from: f64) -> (f64, f64) {
    let r = from.to_radians();
    (-kn * r.sin(), -kn * r.cos())
}

/// The speed of a wind given as east and north components, and the direction it blows from.
pub fn wind_from_uv(u: f64, v: f64) -> (f64, f64) {
    (u.hypot(v), (-u).atan2(-v).to_degrees().rem_euclid(360.0))
}

/// The wind a boat in a current feels: the wind over the ground (`kn` from `from`) less the current's
/// (east, north) knots, as speed and the direction it blows from. Unchanged, to the bit, without a current.
pub fn wind_over_water(kn: f64, from: f64, (ce, cn): (f64, f64)) -> (f64, f64) {
    if ce == 0.0 && cn == 0.0 {
        return (kn, from);
    }
    let (u, v) = wind_uv(kn, from);
    wind_from_uv(u - ce, v - cn)
}

/// The direction a current of (east, north) knots sets towards, degrees true.
pub fn set_of(ce: f64, cn: f64) -> f64 {
    ce.atan2(cn).to_degrees().rem_euclid(360.0)
}

/// A lat/lon rectangle. `west > east` means it crosses the antimeridian.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BBox {
    pub south: f64,
    pub west: f64,
    pub north: f64,
    pub east: f64,
}

impl BBox {
    pub fn contains(&self, p: &LatLon) -> bool {
        let in_lat = p.lat >= self.south && p.lat <= self.north;
        let in_lon = if self.west <= self.east {
            p.lon >= self.west && p.lon <= self.east
        } else {
            p.lon >= self.west || p.lon <= self.east
        };
        in_lat && in_lon
    }

    pub fn width_deg(&self) -> f64 {
        (self.east - self.west).rem_euclid(360.0)
    }

    pub fn height_deg(&self) -> f64 {
        self.north - self.south
    }
}

/// Web Mercator: a point at `zoom` as world pixels, with (0, 0) at the
/// north-west corner of the world.
pub fn project(p: &LatLon, zoom: f64) -> (f64, f64) {
    let size = world_size(zoom);
    let lat = p.lat.clamp(-MAX_LAT, MAX_LAT).to_radians();
    let x = (p.lon + 180.0) / 360.0 * size;
    let y = (1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / PI) / 2.0 * size;
    (x, y)
}

/// Inverse of [`project`].
pub fn unproject(x: f64, y: f64, zoom: f64) -> LatLon {
    let size = world_size(zoom);
    let lon = x / size * 360.0 - 180.0;
    let lat = (PI * (1.0 - 2.0 * y / size)).sinh().atan().to_degrees();
    LatLon::new(lat.clamp(-MAX_LAT, MAX_LAT), normalize_lon(lon))
}

/// Edge length of the whole world in pixels at `zoom`.
pub fn world_size(zoom: f64) -> f64 {
    TILE_SIZE * 2f64.powf(zoom)
}

/// Metres on the ground per pixel at `lat` and `zoom`.
pub fn metres_per_pixel(lat: f64, zoom: f64) -> f64 {
    2.0 * PI * EARTH_RADIUS_M * lat.to_radians().cos() / world_size(zoom)
}

/// Formats a latitude as `48°23.400′N`, the way chart plotters show it.
pub fn format_lat(lat: f64) -> String {
    format_dm(lat.abs(), 2, if lat >= 0.0 { 'N' } else { 'S' })
}

/// Formats a longitude as `004°29.400′W`.
pub fn format_lon(lon: f64) -> String {
    format_dm(lon.abs(), 3, if lon >= 0.0 { 'E' } else { 'W' })
}

fn format_dm(abs: f64, deg_width: usize, hemi: char) -> String {
    let mut deg = abs.floor();
    let mut min = ((abs - deg) * 60.0 * 1000.0).round() / 1000.0;
    if min >= 60.0 {
        deg += 1.0;
        min = 0.0;
    }
    format!("{:0w$}°{:06.3}′{}", deg as u32, min, hemi, w = deg_width)
}


/// Parses a latitude or longitude as people type it: `48.39`, `-4.49`,
/// `48 23.4 N`, `48°23.400′N`, `N48°23'24"`, `4°29.4'W`, `48,39`.
/// Returns signed degrees, or `None` when it is not a valid coordinate.
pub fn parse_coord(input: &str, is_lat: bool) -> Option<f64> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    let upper = s.to_uppercase();
    let letters: Vec<char> = upper.chars().filter(|c| matches!(c, 'N' | 'S' | 'E' | 'W')).collect();
    if letters.len() > 1 {
        return None;
    }
    let hemi = letters.first().copied();
    // The hemisphere must suit the axis.
    match (hemi, is_lat) {
        (Some('E') | Some('W'), true) | (Some('N') | Some('S'), false) => return None,
        _ => {}
    }
    let cleaned: String = upper
        .chars()
        .map(|c| match c {
            'N' | 'S' | 'E' | 'W' | '°' | '′' | '″' | '\'' | '"' | '\u{2019}' | '\u{201d}' => ' ',
            c => c,
        })
        .collect();
    // A comma is a decimal separator only when it is the sole separator.
    let cleaned = if cleaned.matches(',').count() == 1 && !cleaned.contains('.') { cleaned.replace(',', ".") } else { cleaned.replace(',', " ") };
    let nums: Vec<f64> = cleaned.split_whitespace().map(|p| p.parse::<f64>().ok()).collect::<Option<_>>()?;
    if nums.is_empty() || nums.len() > 3 {
        return None;
    }
    let negative_sign = cleaned.trim_start().starts_with('-');
    let deg = nums[0].abs();
    let min = nums.get(1).copied().unwrap_or(0.0);
    let sec = nums.get(2).copied().unwrap_or(0.0);
    if min < 0.0 || sec < 0.0 || min >= 60.0 || sec >= 60.0 || !deg.is_finite() {
        return None;
    }
    // Only the last component may be fractional when several are given.
    if nums.len() > 1 && nums[0].fract() != 0.0 {
        return None;
    }
    let v = deg + min / 60.0 + sec / 3600.0;
    let limit = if is_lat { 90.0 } else { 180.0 };
    if v > limit {
        return None;
    }
    let negative = negative_sign || matches!(hemi, Some('S') | Some('W'));
    Some(if negative { -v } else { v })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PARIS: LatLon = LatLon::new(48.8566, 2.3522);
    const LONDON: LatLon = LatLon::new(51.5074, -0.1278);

    #[test]
    fn paris_london_distance() {
        // About 343.5 km, so about 185.5 nm.
        let d = PARIS.distance_nm(&LONDON);
        assert!((d - 185.5).abs() < 1.0, "{d}");
    }

    #[test]
    fn paris_london_bearing_points_north_west() {
        let b = PARIS.bearing_to(&LONDON);
        assert!((325.0..335.0).contains(&b), "{b}");
    }

    #[test]
    fn destination_inverts_distance_and_bearing() {
        let start = LatLon::new(48.38, -4.49);
        let end = start.destination(247.0, 123.0);
        assert!((start.distance_nm(&end) - 123.0).abs() < 1e-6);
        assert!((start.bearing_to(&end) - 247.0).abs() < 0.5);
    }

    #[test]
    fn one_degree_of_latitude_is_sixty_miles() {
        let d = LatLon::new(10.0, 0.0).distance_nm(&LatLon::new(11.0, 0.0));
        assert!((d - 60.0).abs() < 0.2, "{d}");
    }

    #[test]
    fn rhumb_along_a_parallel_matches_cos_lat() {
        let a = LatLon::new(60.0, 0.0);
        let b = LatLon::new(60.0, 2.0);
        let (dist, brg) = a.rhumb_to(&b);
        assert!((dist - 120.0 * 60f64.to_radians().cos() * 1.0).abs() < 0.3, "{dist}");
        assert!((brg - 90.0).abs() < 1e-6);
    }

    #[test]
    fn rhumb_is_never_shorter_than_great_circle() {
        let (rhumb, _) = PARIS.rhumb_to(&LONDON);
        assert!(rhumb >= PARIS.distance_nm(&LONDON) - 1e-9);
    }

    #[test]
    fn rhumb_takes_the_short_way_across_the_antimeridian() {
        let (dist, brg) = LatLon::new(0.0, 179.0).rhumb_to(&LatLon::new(0.0, -179.0));
        assert!((dist - 120.0).abs() < 0.5, "{dist}");
        assert!((brg - 90.0).abs() < 1e-6);
    }

    #[test]
    fn longitude_wraps() {
        assert_eq!(normalize_lon(190.0), -170.0);
        assert_eq!(normalize_lon(-190.0), 170.0);
        assert_eq!(normalize_lon(180.0), 180.0);
    }

    #[test]
    fn mercator_origin_and_roundtrip() {
        let (x, y) = project(&LatLon::new(0.0, 0.0), 0.0);
        assert!((x - 128.0).abs() < 1e-9 && (y - 128.0).abs() < 1e-9);
        for z in [0.0, 5.5, 12.0] {
            let p = LatLon::new(48.38, -4.49);
            let (x, y) = project(&p, z);
            let q = unproject(x, y, z);
            assert!((p.lat - q.lat).abs() < 1e-9 && (p.lon - q.lon).abs() < 1e-9);
        }
    }

    #[test]
    fn scale_at_equator_zoom_zero() {
        // Mean-sphere circumference over 256 px (the WGS84 equatorial figure,
        // 156 543 m, is 0.1% larger).
        let m = metres_per_pixel(0.0, 0.0);
        assert!((m - 156_368.08).abs() < 0.5, "{m}");
    }

    #[test]
    fn bbox_crossing_antimeridian() {
        let b = BBox { south: -10.0, west: 170.0, north: 10.0, east: -170.0 };
        assert!(b.contains(&LatLon::new(0.0, 179.0)));
        assert!(b.contains(&LatLon::new(0.0, -179.0)));
        assert!(!b.contains(&LatLon::new(0.0, 0.0)));
        assert!((b.width_deg() - 20.0).abs() < 1e-9);
    }

    #[test]
    fn typed_coordinates_parse() {
        let p = |s: &str, lat: bool| parse_coord(s, lat);
        assert_eq!(p("48.39", true), Some(48.39));
        assert_eq!(p("-4.49", false), Some(-4.49));
        assert!((p("48 23.4 N", true).unwrap() - 48.39).abs() < 1e-9);
        assert!((p("48°23.400′N", true).unwrap() - 48.39).abs() < 1e-9);
        assert!((p("N48°23'24\"", true).unwrap() - (48.0 + 23.0 / 60.0 + 24.0 / 3600.0)).abs() < 1e-9);
        assert!((p("4°29.4'W", false).unwrap() + 4.49).abs() < 1e-9);
        assert_eq!(p("004°29.400′W", false), p("-4 29.4", false));
        assert_eq!(p("48,39", true), Some(48.39));
        assert_eq!(p("  12.5s ", true), Some(-12.5));
        assert_eq!(p("0", true), Some(0.0));
    }

    #[test]
    fn bad_coordinates_are_refused() {
        for (s, lat) in [("", true), ("abc", true), ("91", true), ("181", false), ("48 60 N", true), ("48 23 61", true),
                         ("48 23 24 12", true), ("48N 23N", true), ("4W", true), ("48N", false), ("48.5 23.4", true), ("nan", true), ("inf", true)] {
            assert_eq!(parse_coord(s, lat), None, "{s:?}");
        }
    }

    #[test]
    fn formatted_coordinates_parse_back() {
        for v in [48.39f64, -4.49, 0.0166, -33.8567, 179.9999] {
            let lat_s = format_lat(v.clamp(-90.0, 90.0));
            assert!((parse_coord(&lat_s, true).unwrap() - v.clamp(-90.0, 90.0)).abs() < 1e-3, "{lat_s}");
            let lon_s = format_lon(v);
            assert!((parse_coord(&lon_s, false).unwrap() - v).abs() < 1e-3, "{lon_s}");
        }
    }

    #[test]
    fn coordinates_are_formatted_for_a_chart() {
        assert_eq!(format_lat(48.39), "48°23.400′N");
        assert_eq!(format_lon(-4.49), "004°29.400′W");
        assert_eq!(format_lat(-0.0166667), "00°01.000′S");
    }
}
