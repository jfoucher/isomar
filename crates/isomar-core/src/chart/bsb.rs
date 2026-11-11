//! BSB/KAP raster nautical charts: header, georeferencing and the run-length
//! coded image. Charts are decoded to one palette index per pixel.

use crate::vector::{Bounds, Pt};
use std::collections::HashMap;
use std::f64::consts::PI;

#[derive(Debug, PartialEq, Eq)]
pub enum BsbError {
    NotBsb,
    MissingField(&'static str),
    /// Fewer than three reference points, so no georeferencing is possible.
    NotEnoughReferences,
    /// The reference points do not define a usable transform.
    DegenerateReferences,
    Truncated,
}

impl std::fmt::Display for BsbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BsbError::NotBsb => write!(f, "this is not a BSB chart"),
            BsbError::MissingField(s) => write!(f, "the chart header has no {s}"),
            BsbError::NotEnoughReferences => write!(f, "the chart has fewer than three reference points"),
            BsbError::DegenerateReferences => write!(f, "the chart's reference points do not fix its position"),
            BsbError::Truncated => write!(f, "the chart image is cut short"),
        }
    }
}

impl std::error::Error for BsbError {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RefPoint {
    pub x: f64,
    pub y: f64,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Clone, Debug)]
pub struct Header {
    pub name: String,
    pub number: String,
    pub width: usize,
    pub height: usize,
    pub scale: u32,
    pub projection: String,
    pub datum: String,
    pub refs: Vec<RefPoint>,
    /// Coverage polygon as `[lon, lat]`.
    pub polygon: Vec<Pt>,
    /// Palette name (`RGB`, `DAY`, `DSK`, `NGT`) to colours; index 0 is unused.
    pub palettes: HashMap<String, Vec<[u8; 3]>>,
    pub depth_bits: u8,
    /// Datum shift in degrees, `(lon, lat)`, to add to chart positions.
    pub datum_shift: (f64, f64),
    /// Where the image data starts in the file.
    pub data_offset: usize,
}

/// Maps between pixels and positions with an affine fit of the reference
/// points, in Mercator coordinates for Mercator charts and in a local
/// equirectangular plane otherwise (accurate enough across one chart).
#[derive(Clone, Debug)]
pub struct Georef {
    mercator: bool,
    cos_lat: f64,
    /// u = a0 + a1 x + a2 y ; v = b0 + b1 x + b2 y
    fwd: [f64; 6],
    inv: [f64; 6],
    shift: (f64, f64),
    /// Middle of the chart's (unwrapped) longitudes, to bring any query next to it.
    lon_centre: f64,
}

/// A longitude brought into -180..180.
pub fn wrap_lon(lon: f64) -> f64 {
    lon - 360.0 * ((lon + 180.0) / 360.0).floor()
}

/// Charts that cross the antimeridian list longitudes such as 176 then -154.
/// Adding 360 to the western ones makes them continuous (176 then 206), so
/// fitting, the coverage polygon and the bounds all work. Returns whether
/// the chart crosses.
fn unwrap_lons(refs: &mut [RefPoint], polygon: &mut [Pt]) -> bool {
    let lons = || refs.iter().map(|r| r.lon).chain(polygon.iter().map(|p| p[0]));
    let (lo, hi) = lons().fold((f64::MAX, f64::MIN), |a, l| (a.0.min(l), a.1.max(l)));
    if hi - lo <= 180.0 {
        return false;
    }
    for r in refs.iter_mut().filter(|r| r.lon < 0.0) {
        r.lon += 360.0;
    }
    for p in polygon.iter_mut().filter(|p| p[0] < 0.0) {
        p[0] += 360.0;
    }
    true
}

fn merc_y(lat: f64) -> f64 {
    (PI / 4.0 + lat.to_radians() / 2.0).tan().ln().to_degrees()
}

fn inv_merc_y(v: f64) -> f64 {
    (2.0 * (v.to_radians().exp()).atan() - PI / 2.0).to_degrees()
}

/// Solves the 3x3 normal equations for `target = c0 + c1 x + c2 y`.
fn fit(pts: &[(f64, f64, f64)]) -> Option<[f64; 3]> {
    let (mut sx, mut sy, mut sxx, mut sxy, mut syy, mut st, mut sxt, mut syt) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    let n = pts.len() as f64;
    for &(x, y, t) in pts {
        sx += x;
        sy += y;
        sxx += x * x;
        sxy += x * y;
        syy += y * y;
        st += t;
        sxt += x * t;
        syt += y * t;
    }
    let m = [[n, sx, sy], [sx, sxx, sxy], [sy, sxy, syy]];
    let rhs = [st, sxt, syt];
    let det = |m: [[f64; 3]; 3]| m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let d = det(m);
    if d.abs() < 1e-12 {
        return None;
    }
    let mut out = [0.0; 3];
    for (k, o) in out.iter_mut().enumerate() {
        let mut mk = m;
        for r in 0..3 {
            mk[r][k] = rhs[r];
        }
        *o = det(mk) / d;
    }
    Some(out)
}

impl Georef {
    pub fn new(h: &Header) -> Result<Georef, BsbError> {
        if h.refs.len() < 3 {
            return Err(BsbError::NotEnoughReferences);
        }
        let mercator = h.projection.to_ascii_uppercase().contains("MERCATOR") && !h.projection.to_ascii_uppercase().contains("TRANSVERSE");
        let mean_lat = h.refs.iter().map(|r| r.lat).sum::<f64>() / h.refs.len() as f64;
        let cos_lat = mean_lat.to_radians().cos();
        let uv = |lat: f64, lon: f64| if mercator { (lon, merc_y(lat)) } else { (lon * cos_lat, lat) };
        let us: Vec<_> = h.refs.iter().map(|r| (r.x, r.y, uv(r.lat, r.lon).0)).collect();
        let vs: Vec<_> = h.refs.iter().map(|r| (r.x, r.y, uv(r.lat, r.lon).1)).collect();
        let (a, b) = (fit(&us).ok_or(BsbError::DegenerateReferences)?, fit(&vs).ok_or(BsbError::DegenerateReferences)?);
        let det = a[1] * b[2] - a[2] * b[1];
        if det.abs() < 1e-18 {
            return Err(BsbError::DegenerateReferences);
        }
        // Invert [u-a0, v-b0] = M [x, y].
        let (i11, i12, i21, i22) = (b[2] / det, -a[2] / det, -b[1] / det, a[1] / det);
        let inv = [-(i11 * a[0] + i12 * b[0]), i11, i12, -(i21 * a[0] + i22 * b[0]), i21, i22];
        let lon_centre = h.refs.iter().map(|r| r.lon).sum::<f64>() / h.refs.len() as f64;
        Ok(Georef { mercator, cos_lat, fwd: [a[0], a[1], a[2], b[0], b[1], b[2]], inv, shift: h.datum_shift, lon_centre })
    }

    /// Position `(lat, lon)` of a pixel, with the longitude in -180..180.
    pub fn pixel_to_ll(&self, x: f64, y: f64) -> (f64, f64) {
        let u = self.fwd[0] + self.fwd[1] * x + self.fwd[2] * y;
        let v = self.fwd[3] + self.fwd[4] * x + self.fwd[5] * y;
        let (lat, lon) = if self.mercator { (inv_merc_y(v), u) } else { (v, u / self.cos_lat) };
        (lat + self.shift.1, wrap_lon(lon + self.shift.0))
    }

    /// Pixel of a position (fractional, possibly outside the image). The
    /// longitude may be on either side of the antimeridian.
    pub fn ll_to_pixel(&self, lat: f64, lon: f64) -> (f64, f64) {
        self.uv_to_pixel(self.u_of_lon(lon), self.v_of_lat(lat))
    }

    /// The chart plane's horizontal coordinate of a longitude. Depends on
    /// the longitude alone, so a drawing can work it out once per column.
    pub fn u_of_lon(&self, lon: f64) -> f64 {
        let lon = lon - self.shift.0;
        let lon = lon + 360.0 * ((self.lon_centre - lon) / 360.0).round();
        if self.mercator { lon } else { lon * self.cos_lat }
    }

    /// The vertical coordinate of a latitude, once per row.
    pub fn v_of_lat(&self, lat: f64) -> f64 {
        let lat = lat - self.shift.1;
        if self.mercator { merc_y(lat) } else { lat }
    }

    /// Pixel at a point of the chart plane.
    pub fn uv_to_pixel(&self, u: f64, v: f64) -> (f64, f64) {
        (self.inv[0] + self.inv[1] * u + self.inv[2] * v, self.inv[3] + self.inv[4] * u + self.inv[5] * v)
    }

    /// Whether the transform is simple enough to use per column and per row
    /// (no rotation), which makes drawing cheaper.
    pub fn is_north_up(&self) -> bool {
        self.fwd[2].abs() < 1e-9 * self.fwd[1].abs() && self.fwd[4].abs() < 1e-9 * self.fwd[5].abs()
    }
}

fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let i = line.find(&format!("{key}="))?;
    let rest = &line[i + key.len() + 1..];
    let end = rest.find(',').unwrap_or(rest.len());
    Some(rest[..end].trim())
}

/// The header ends at the byte pair `1A 00`; a lone 0x1A can appear inside
/// the text of some charts.
pub fn header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(2).position(|w| w == [0x1A, 0x00])
}

/// Reads the header and finds where the image starts.
pub fn parse_header(bytes: &[u8]) -> Result<Header, BsbError> {
    let end = header_end(bytes).ok_or(BsbError::NotBsb)?;
    if bytes.len() < end + 3 {
        return Err(BsbError::NotBsb);
    }
    let text = String::from_utf8_lossy(&bytes[..end]).into_owned();
    // Continuation lines start with whitespace and belong to the record above.
    let mut records: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(last) = records.last_mut() {
                last.push(',');
                last.push_str(line.trim());
            }
        } else if !line.is_empty() {
            records.push(line.to_string());
        }
    }
    if !records.iter().any(|r| r.starts_with("BSB/")) {
        return Err(BsbError::NotBsb);
    }
    let (mut name, mut number, mut width, mut height) = (String::new(), String::new(), 0usize, 0usize);
    let (mut scale, mut projection, mut datum) = (0u32, String::new(), String::new());
    let (mut refs, mut polygon) = (Vec::new(), Vec::new());
    let mut palettes: HashMap<String, Vec<[u8; 3]>> = HashMap::new();
    let (mut depth_bits, mut datum_shift) = (0u8, (0.0, 0.0));
    for r in &records {
        let Some((tag, rest)) = r.split_once('/') else { continue };
        match tag {
            "BSB" => {
                // The name may itself contain commas, so cut at the next known key.
                if let Some(i) = rest.find("NA=") {
                    let after = &rest[i + 3..];
                    let cut = after.find(",NU=").or_else(|| after.find(",RA=")).unwrap_or(after.len());
                    name = after[..cut].trim().to_string();
                }
                number = field(rest, "NU").unwrap_or("").to_string();
                if let Some(ra) = field(rest, "RA") {
                    // RA=w,h: the height follows the comma.
                    let i = rest.find("RA=").unwrap() + 3;
                    let mut parts = rest[i..].split(',');
                    width = parts.next().and_then(|v| v.trim().parse().ok()).unwrap_or(0);
                    height = parts.next().and_then(|v| v.trim().parse().ok()).unwrap_or(0);
                    let _ = ra;
                }
            }
            "KNP" => {
                scale = field(rest, "SC").and_then(|v| v.parse().ok()).unwrap_or(0);
                projection = field(rest, "PR").unwrap_or("").to_string();
                datum = field(rest, "GD").unwrap_or("").to_string();
            }
            "REF" => {
                let p: Vec<f64> = rest.split(',').skip(1).filter_map(|v| v.trim().parse().ok()).collect();
                if let [x, y, lat, lon] = p[..] {
                    refs.push(RefPoint { x, y, lat, lon });
                }
            }
            "PLY" => {
                let p: Vec<f64> = rest.split(',').skip(1).filter_map(|v| v.trim().parse().ok()).collect();
                if let [lat, lon] = p[..] {
                    polygon.push([lon, lat]);
                }
            }
            "RGB" | "DAY" | "DSK" | "NGT" => {
                let p: Vec<i64> = rest.split(',').filter_map(|v| v.trim().parse().ok()).collect();
                if let [i, rr, g, b] = p[..]
                    && (0..=255).contains(&i)
                {
                    let pal = palettes.entry(tag.to_string()).or_default();
                    if pal.len() <= i as usize {
                        pal.resize(i as usize + 1, [0, 0, 0]);
                    }
                    pal[i as usize] = [rr.clamp(0, 255) as u8, g.clamp(0, 255) as u8, b.clamp(0, 255) as u8];
                }
            }
            "DTM" => {
                let p: Vec<f64> = rest.split(',').filter_map(|v| v.trim().parse().ok()).collect();
                if let [lon, lat] = p[..] {
                    // Given in seconds of arc.
                    datum_shift = (lon / 3600.0, lat / 3600.0);
                }
            }
            "IFM" => depth_bits = rest.trim().parse().unwrap_or(0),
            _ => {}
        }
    }
    unwrap_lons(&mut refs, &mut polygon);
    if width == 0 || height == 0 {
        return Err(BsbError::MissingField("image size (RA)"));
    }
    if palettes.is_empty() {
        return Err(BsbError::MissingField("palette (RGB)"));
    }
    // The byte after the header terminator gives the bits per pixel.
    let bits = bytes[end + 2];
    if (1..=7).contains(&bits) {
        depth_bits = bits;
    }
    if !(1..=7).contains(&depth_bits) {
        return Err(BsbError::MissingField("colour depth"));
    }
    Ok(Header { name, number, width, height, scale, projection, datum, refs, polygon, palettes, depth_bits, datum_shift, data_offset: end + 3 })
}

impl Header {
    /// The chart's footprint: its coverage polygon, or failing that the
    /// image corners, as lon/lat bounds. A chart that crosses the
    /// antimeridian has `max_lon` above 180 (and so does `polygon`); callers
    /// test it shifted by 360 degrees as well.
    pub fn bounds(&self) -> Option<Bounds> {
        let b = if self.polygon.len() >= 3 {
            Bounds::of(&self.polygon)
        } else {
            let g = Georef::new(self).ok()?;
            let corners: Vec<Pt> = [(0.0, 0.0), (self.width as f64, 0.0), (self.width as f64, self.height as f64), (0.0, self.height as f64)]
                .iter()
                .map(|(x, y)| {
                    let (lat, lon) = g.pixel_to_ll(*x, *y);
                    [lon, lat]
                })
                .collect();
            let mut b = Bounds::of(&corners);
            if b.width() > 180.0 {
                // Corners either side of the antimeridian.
                let east: Vec<Pt> = corners.iter().map(|c| [if c[0] < 0.0 { c[0] + 360.0 } else { c[0] }, c[1]]).collect();
                b = Bounds::of(&east);
            }
            b
        };
        (!b.is_empty()).then_some(b)
    }

    /// Palette for a tone: day colours by default; dusk and night when the
    /// chart supplies them.
    pub fn palette(&self, key: &str) -> &[[u8; 3]] {
        self.palettes.get(key).or_else(|| self.palettes.get("DAY")).or_else(|| self.palettes.get("RGB")).map(|v| v.as_slice()).unwrap_or(&[])
    }
}

#[derive(Debug, Clone)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    /// One palette index per pixel, row-major from the top.
    pub index: Vec<u8>,
}

impl Image {
    pub fn at(&self, x: usize, y: usize) -> u8 {
        self.index[y * self.width + x]
    }
}

/// Decodes the run-length coded image that follows the header.
pub fn decode_image(bytes: &[u8], h: &Header) -> Result<Image, BsbError> {
    let n = h.depth_bits as u32;
    let value_mask = (((1u32 << n) - 1) << (7 - n)) as u8;
    let value_shift = 7 - n;
    let count_mask = ((1u32 << (7 - n)) - 1) as u8;
    let mut index = vec![0u8; h.width * h.height];
    let mut pos = h.data_offset;
    let next = |pos: &mut usize| -> Result<u8, BsbError> {
        let b = *bytes.get(*pos).ok_or(BsbError::Truncated)?;
        *pos += 1;
        Ok(b)
    };
    let mut rows_done = 0;
    // Row numbers start at 1, except in some older charts (SHOM's) where they start at 0.
    let mut first_row = 1usize;
    while rows_done < h.height {
        // The row number, seven bits at a time with a continuation flag.
        let mut row = 0usize;
        loop {
            let b = match next(&mut pos) {
                Ok(b) => b,
                // A file may end after the last full row.
                Err(_) if rows_done > 0 => return Ok(Image { width: h.width, height: h.height, index }),
                Err(e) => return Err(e),
            };
            row = row * 128 + (b & 0x7f) as usize;
            if b & 0x80 == 0 {
                break;
            }
        }
        if rows_done == 0 && row == 0 {
            first_row = 0;
        }
        // The end-of-file marker is a row number past the image.
        if row < first_row || row - first_row >= h.height {
            if rows_done == 0 {
                return Err(BsbError::Truncated);
            }
            break;
        }
        let y = row - first_row;
        let mut x = 0usize;
        loop {
            let b = next(&mut pos)?;
            if b == 0 {
                break;
            }
            let value = (b & value_mask) >> value_shift;
            let mut run = (b & count_mask) as usize;
            let mut cur = b;
            while cur & 0x80 != 0 {
                cur = next(&mut pos)?;
                run = run * 128 + (cur & 0x7f) as usize;
            }
            run += 1;
            let end = (x + run).min(h.width);
            if x < end {
                index[y * h.width + x..y * h.width + end].fill(value);
            }
            x += run;
        }
        rows_done += 1;
    }
    Ok(Image { width: h.width, height: h.height, index })
}

#[cfg(any(test, feature = "testutil"))]
pub mod testutil {
    //! Builds small KAP files.

    /// Encodes one row of palette indexes as runs.
    pub fn encode_row(row_number: usize, pixels: &[u8], depth: u32) -> Vec<u8> {
        let mut out = Vec::new();
        // Row number.
        let mut digits = vec![(row_number % 128) as u8];
        let mut r = row_number / 128;
        while r > 0 {
            digits.push((r % 128) as u8 | 0x80);
            r /= 128;
        }
        digits.reverse();
        out.extend(digits);
        let count_bits = 7 - depth;
        let mut i = 0;
        while i < pixels.len() {
            let v = pixels[i];
            let mut run = 1;
            while i + run < pixels.len() && pixels[i + run] == v {
                run += 1;
            }
            i += run;
            // run - 1 split into the low count bits and continuation bytes.
            let mut n = run - 1;
            let mut cont = Vec::new();
            let first_max = (1usize << count_bits) - 1;
            let first = n % (first_max + 1);
            n /= first_max + 1;
            while n > 0 {
                cont.push((n % 128) as u8);
                n /= 128;
            }
            let mut b = (v << count_bits) | first as u8;
            if !cont.is_empty() {
                b |= 0x80;
            }
            out.push(b);
            cont.reverse();
            for (k, c) in cont.iter().enumerate() {
                out.push(if k + 1 < cont.len() { c | 0x80 } else { *c });
            }
        }
        out.push(0);
        out
    }

    pub fn kap(width: usize, height: usize, pixels: &dyn Fn(usize, usize) -> u8, projection: &str, refs: &[(f64, f64, f64, f64)]) -> Vec<u8> {
        let depth = 4u32;
        let mut text = String::new();
        text.push_str("!\r\nVER/3.0\r\n");
        text.push_str(&format!("BSB/NA=Test Chart, with comma,NU=T1\r\n    RA={width},{height},DU=254\r\n"));
        text.push_str(&format!("KNP/SC=25000,GD=WGS84,PR={projection},PP=175.39\r\n    SK=0.0,TA=90.0\r\n"));
        for (i, (x, y, lat, lon)) in refs.iter().enumerate() {
            text.push_str(&format!("REF/{},{x},{y},{lat},{lon}\r\n", i + 1));
        }
        text.push_str("PLY/1,-36.10,175.30\r\nPLY/2,-36.10,175.40\r\nPLY/3,-36.20,175.40\r\n");
        text.push_str("RGB/1,0,0,0\r\nRGB/2,255,255,255\r\nRGB/3,200,0,0\r\nDAY/3,210,10,10\r\nNGT/3,50,0,0\r\nIFM/4\r\n");
        let mut out = text.into_bytes();
        out.extend([0x1A, 0x00, depth as u8]);
        for y in 0..height {
            let row: Vec<u8> = (0..width).map(|x| pixels(x, y)).collect();
            out.extend(encode_row(y + 1, &row, depth));
        }
        out.extend([0x00, 0x00]);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;

    fn refs_mercator() -> Vec<(f64, f64, f64, f64)> {
        // 100 x 60 pixel chart, 0.1 degrees wide, north up, in Mercator.
        let (w, h) = (100.0, 60.0);
        let lat_top = -36.0;
        let lon_left = 175.0;
        let lat_at = |y: f64| inv_merc_y(merc_y(lat_top) - y * (0.1 / w));
        vec![(0.0, 0.0, lat_top, lon_left), (w, 0.0, lat_top, lon_left + 0.1), (w, h, lat_at(h), lon_left + 0.1), (0.0, h, lat_at(h), lon_left)]
    }

    #[test]
    fn header_fields_are_read_including_names_with_commas() {
        let bytes = kap(100, 60, &|x, _| (x % 3) as u8 + 1, "MERCATOR", &refs_mercator());
        let h = parse_header(&bytes).unwrap();
        assert_eq!(h.name, "Test Chart, with comma");
        assert_eq!((h.number.as_str(), h.width, h.height, h.scale), ("T1", 100, 60, 25_000));
        assert_eq!((h.projection.as_str(), h.datum.as_str(), h.depth_bits), ("MERCATOR", "WGS84", 4));
        assert_eq!(h.refs.len(), 4);
        assert_eq!(h.polygon.len(), 3);
        assert_eq!(h.palette("RGB")[3], [200, 0, 0]);
        assert_eq!(h.palette("DAY")[3], [210, 10, 10]);
        assert_eq!(h.palette("NGT")[3], [50, 0, 0]);
        assert_eq!(h.palette("DSK")[3], [210, 10, 10], "falls back to day colours");
        assert_eq!(h.data_offset, bytes.iter().position(|b| *b == 0x1A).unwrap() + 3);
    }

    #[test]
    fn a_stray_1a_inside_the_text_does_not_end_the_header() {
        let mut bytes = kap(100, 60, &|_, _| 1, "MERCATOR", &refs_mercator());
        // Plant a lone 0x1A in the first comment line.
        let at = bytes.iter().position(|b| *b == b'V').unwrap();
        bytes[at] = 0x1A;
        bytes[at + 1] = b' ';
        let h = parse_header(&bytes).unwrap();
        assert_eq!(h.width, 100);
        assert_eq!(h.data_offset, header_end(&bytes).unwrap() + 3);
    }

    #[test]
    fn the_image_decodes_exactly() {
        let f = |x: usize, y: usize| ((x / 7 + y) % 5) as u8 + 1;
        let bytes = kap(100, 60, &f, "MERCATOR", &refs_mercator());
        let h = parse_header(&bytes).unwrap();
        let img = decode_image(&bytes, &h).unwrap();
        for y in 0..60 {
            for x in 0..100 {
                assert_eq!(img.at(x, y), f(x, y), "pixel {x},{y}");
            }
        }
    }

    #[test]
    fn rows_numbered_from_zero_decode_too() {
        // SHOM's older charts number the rows 0 to height - 1 and have no end marker.
        let f = |x: usize, y: usize| ((x + y) % 3) as u8 + 1;
        let bytes = kap(100, 60, &f, "MERCATOR", &refs_mercator());
        let h = parse_header(&bytes).unwrap();
        let mut zero_based = bytes[..h.data_offset].to_vec();
        for y in 0..60 {
            let row: Vec<u8> = (0..100).map(|x| f(x, y)).collect();
            zero_based.extend(encode_row(y, &row, 4));
        }
        let img = decode_image(&zero_based, &h).unwrap();
        for y in 0..60 {
            for x in 0..100 {
                assert_eq!(img.at(x, y), f(x, y), "pixel {x},{y}");
            }
        }
    }

    #[test]
    fn long_runs_use_continuation_bytes() {
        // A whole row of one colour is a run of 3000, far past the 3 count bits.
        let bytes = kap(3000, 4, &|_, y| y as u8 + 1, "MERCATOR", &refs_mercator());
        let h = parse_header(&bytes).unwrap();
        let img = decode_image(&bytes, &h).unwrap();
        assert!(img.index[..3000].iter().all(|v| *v == 1));
        assert!(img.index[9000..].iter().all(|v| *v == 4));
    }

    #[test]
    fn georeferencing_roundtrips_and_hits_the_reference_points() {
        let bytes = kap(100, 60, &|_, _| 1, "MERCATOR", &refs_mercator());
        let h = parse_header(&bytes).unwrap();
        let g = Georef::new(&h).unwrap();
        for r in &h.refs {
            let (lat, lon) = g.pixel_to_ll(r.x, r.y);
            assert!((lat - r.lat).abs() < 1e-6 && (lon - r.lon).abs() < 1e-6, "{r:?} -> {lat} {lon}");
            let (x, y) = g.ll_to_pixel(r.lat, r.lon);
            assert!((x - r.x).abs() < 1e-3 && (y - r.y).abs() < 1e-3);
        }
        let (lat, lon) = g.pixel_to_ll(50.0, 30.0);
        assert!((lon - 175.05).abs() < 1e-6 && lat < -36.0 && lat > -36.1);
        assert!(g.is_north_up());
        let b = h.bounds().unwrap();
        assert!((b.min_lon - 175.30).abs() < 1e-9, "the coverage polygon wins: {b:?}");
    }

    #[test]
    fn other_projections_use_a_local_plane() {
        let refs = [(0.0, 0.0, -36.0, 175.0), (100.0, 0.0, -36.0, 175.12), (100.0, 60.0, -36.1, 175.12), (0.0, 60.0, -36.1, 175.0)];
        let bytes = kap(100, 60, &|_, _| 1, "Transverse Mercator", &refs);
        let h = parse_header(&bytes).unwrap();
        let g = Georef::new(&h).unwrap();
        let (lat, lon) = g.pixel_to_ll(100.0, 60.0);
        assert!((lat + 36.1).abs() < 1e-6 && (lon - 175.12).abs() < 1e-6);
        let (x, y) = g.ll_to_pixel(-36.05, 175.06);
        assert!((x - 50.0).abs() < 0.01 && (y - 30.0).abs() < 0.01);
    }

    #[test]
    fn rotated_charts_are_handled() {
        // Rotate the chart 90 degrees: x runs north, y runs east.
        let refs = [(0.0, 0.0, -36.0, 175.0), (100.0, 0.0, -35.9, 175.0), (100.0, 60.0, -35.9, 175.06), (0.0, 60.0, -36.0, 175.06)];
        let bytes = kap(100, 60, &|_, _| 1, "Transverse Mercator", &refs);
        let g = Georef::new(&parse_header(&bytes).unwrap()).unwrap();
        assert!(!g.is_north_up());
        let (lat, lon) = g.pixel_to_ll(50.0, 30.0);
        assert!((lat + 35.95).abs() < 1e-6 && (lon - 175.03).abs() < 1e-6);
    }

    #[test]
    fn bad_charts_are_rejected_with_a_reason() {
        assert_eq!(parse_header(b"hello").unwrap_err(), BsbError::NotBsb);
        assert_eq!(parse_header(b"!\r\nNOT/1\r\n\x1a\x00\x04").unwrap_err(), BsbError::NotBsb);
        let two_refs = kap(100, 60, &|_, _| 1, "MERCATOR", &refs_mercator()[..2]);
        let h = parse_header(&two_refs).unwrap();
        assert_eq!(Georef::new(&h).unwrap_err(), BsbError::NotEnoughReferences);
        let collinear = kap(100, 60, &|_, _| 1, "MERCATOR", &[(0.0, 0.0, -36.0, 175.0), (50.0, 0.0, -36.0, 175.05), (100.0, 0.0, -36.0, 175.1)]);
        assert_eq!(Georef::new(&parse_header(&collinear).unwrap()).unwrap_err(), BsbError::DegenerateReferences);
    }

    #[test]
    fn truncated_images_fail_or_stop_cleanly() {
        let bytes = kap(100, 60, &|x, y| ((x + y) % 4) as u8 + 1, "MERCATOR", &refs_mercator());
        let h = parse_header(&bytes).unwrap();
        for cut in [h.data_offset, h.data_offset + 3, h.data_offset + 40, bytes.len() / 2] {
            let r = decode_image(&bytes[..cut], &h);
            assert!(r.is_err() || r.is_ok(), "no panic at {cut}");
        }
        assert_eq!(decode_image(&bytes[..h.data_offset], &h).unwrap_err(), BsbError::Truncated);
    }

    #[test]
    fn a_real_new_zealand_chart() {
        let p = std::path::Path::new("testdata/nz-kap-archive/BSB_ROOT/NZ5224/NZ522401.KAP");
        if !p.exists() {
            return;
        }
        let bytes = std::fs::read(p).unwrap();
        let h = parse_header(&bytes).unwrap();
        assert_eq!((h.width, h.height, h.scale), (10_343, 6_852, 25_000));
        assert!(h.name.starts_with("Great Barrier Island"), "{}", h.name);
        assert!(h.refs.len() >= 3);
        let g = Georef::new(&h).unwrap();
        // Southern Aotea / Great Barrier Island.
        let (lat, lon) = g.pixel_to_ll(h.width as f64 / 2.0, h.height as f64 / 2.0);
        assert!((-36.4..-36.0).contains(&lat) && (175.2..175.6).contains(&lon), "{lat} {lon}");
        for r in &h.refs {
            let (lt, ln) = g.pixel_to_ll(r.x, r.y);
            // The fit across a transverse Mercator chart is good to a few pixels.
            let px = ((lt - r.lat) * 111_320.0).hypot((ln - r.lon) * 111_320.0 * 0.8) / 2.5;
            assert!(px < 12.0, "reference point misplaced by {px} px");
        }
        let img = decode_image(&bytes, &h).unwrap();
        assert_eq!(img.index.len(), h.width * h.height);
        let pal = h.palette("DAY").len();
        assert!(img.index.iter().all(|v| (*v as usize) < pal), "indexes stay inside the palette");
        let distinct: std::collections::HashSet<_> = img.index.iter().step_by(997).collect();
        assert!(distinct.len() >= 4, "a real chart has several colours");
        // Most of the chart is sea or land, not the unpainted zero.
        assert!(img.index.iter().step_by(101).filter(|v| **v == 0).count() < img.index.len() / 101 / 4);
    }

    #[test]
    fn a_chart_across_the_antimeridian_is_placed_where_it_says() {
        // 0.1 degrees wide, from 179.95 east to -179.95 (listed wrapped, as
        // the New Zealand ocean charts do).
        let w = |lon: f64| if lon > 180.0 { lon - 360.0 } else { lon };
        let refs: Vec<_> = [(0.0, 0.0, -36.0), (100.0, 0.0, -36.0), (100.0, 60.0, -36.06), (0.0, 60.0, -36.06)]
            .iter()
            .map(|&(x, y, lat)| (x, y, lat, w(179.95 + x * 0.001)))
            .collect();
        let bytes = kap(100, 60, &|_, _| 1, "MERCATOR", &refs);
        let h = parse_header(&bytes).unwrap();
        let g = Georef::new(&h).unwrap();
        let (_, lon) = g.pixel_to_ll(50.0, 30.0);
        assert!((lon.abs() - 180.0).abs() < 1e-6, "the antimeridian itself: {lon}");
        let (_, lon) = g.pixel_to_ll(75.0, 30.0);
        assert!((lon + 179.975).abs() < 1e-6, "reported in -180..180: {lon}");
        // Either side of the antimeridian lands on the right pixel, and so
        // does the same longitude written past 180.
        let (x, _) = g.ll_to_pixel(-36.03, -179.975);
        assert!((x - 75.0).abs() < 0.01, "{x}");
        let (x, _) = g.ll_to_pixel(-36.03, 180.025);
        assert!((x - 75.0).abs() < 0.01, "{x}");
        let (x, _) = g.ll_to_pixel(-36.03, 179.975);
        assert!((x - 25.0).abs() < 0.01, "{x}");
    }

    #[test]
    fn bounds_of_a_chart_across_the_antimeridian_run_past_180() {
        let mut refs = refs_mercator();
        for r in refs.iter_mut() {
            r.3 = r.3 - 175.0 + 179.95;
            if r.3 > 180.0 {
                r.3 -= 360.0;
            }
        }
        let h = parse_header(&kap(100, 60, &|_, _| 1, "MERCATOR", &refs)).unwrap();
        assert!(h.refs.iter().all(|r| r.lon > 179.0), "{:?}", h.refs);
        let b = h.bounds().unwrap();
        assert!(b.width() < 1.0, "not the width of the world: {b:?}");
    }
}
