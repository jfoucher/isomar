//! Zarr v2 as the Copernicus Marine ARCO stores lay it out: the consolidated metadata (`.zmetadata`),
//! chunk keys, decoding a chunk's values (Blosc, the dtype, the fill value, CF scale and offset), and the
//! CF time axis.

use super::blosc;
use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, Utc};
use std::collections::BTreeMap;
use std::ops::Range;

#[derive(Debug, Clone, PartialEq)]
pub struct ZarrError(pub String);

impl std::fmt::Display for ZarrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "zarr: {}", self.0)
    }
}

impl std::error::Error for ZarrError {}

impl From<blosc::BloscError> for ZarrError {
    fn from(e: blosc::BloscError) -> Self {
        ZarrError(e.to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dtype {
    F4,
    F8,
    I2,
    I4,
    I8,
}

impl Dtype {
    pub fn parse(s: &str) -> Option<Dtype> {
        Some(match s {
            "<f4" => Dtype::F4,
            "<f8" => Dtype::F8,
            "<i2" => Dtype::I2,
            "<i4" => Dtype::I4,
            "<i8" => Dtype::I8,
            _ => return None,
        })
    }

    pub fn size(self) -> usize {
        match self {
            Dtype::I2 => 2,
            Dtype::F4 | Dtype::I4 => 4,
            Dtype::F8 | Dtype::I8 => 8,
        }
    }

    fn value(self, b: &[u8]) -> f64 {
        match self {
            Dtype::F4 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
            Dtype::F8 => f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]),
            Dtype::I2 => i16::from_le_bytes([b[0], b[1]]) as f64,
            Dtype::I4 => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
            Dtype::I8 => i64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as f64,
        }
    }
}

/// One array of a store.
#[derive(Clone, Debug, PartialEq)]
pub struct ArrayMeta {
    pub shape: Vec<usize>,
    pub chunks: Vec<usize>,
    pub dtype: Dtype,
    /// The raw value that stands for "no data".
    pub fill: Option<f64>,
    /// CF packing: value = raw × scale + offset.
    pub scale: f64,
    pub offset: f64,
    /// The dimension names (`_ARRAY_DIMENSIONS`).
    pub dims: Vec<String>,
    /// CF units ("m s-1", "hours since 1950-01-01").
    pub units: String,
    pub blosc: bool,
    pub separator: String,
}

fn num(v: &serde_json::Value) -> Option<f64> {
    match v {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) if s == "NaN" => Some(f64::NAN),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// The arrays of a consolidated `.zmetadata`, by name.
pub fn parse_metadata(bytes: &[u8]) -> Result<BTreeMap<String, ArrayMeta>, ZarrError> {
    let v: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| ZarrError(e.to_string()))?;
    let meta = v.get("metadata").and_then(|m| m.as_object()).ok_or_else(|| ZarrError("no consolidated metadata".into()))?;
    let mut out = BTreeMap::new();
    for (key, a) in meta {
        let Some(name) = key.strip_suffix("/.zarray") else { continue };
        let attrs = meta.get(&format!("{name}/.zattrs"));
        let list = |k: &str| a.get(k).and_then(|x| x.as_array()).map(|xs| xs.iter().filter_map(|x| x.as_u64().map(|n| n as usize)).collect::<Vec<_>>()).unwrap_or_default();
        let Some(dtype) = a.get("dtype").and_then(|d| d.as_str()).and_then(Dtype::parse) else { continue };
        if a.get("order").and_then(|o| o.as_str()).unwrap_or("C") != "C" || a.get("filters").is_some_and(|f| !f.is_null()) {
            continue;
        }
        let blosc = match a.get("compressor") {
            Some(c) if !c.is_null() => {
                if c.get("id").and_then(|i| i.as_str()) != Some("blosc") {
                    continue;
                }
                true
            }
            _ => false,
        };
        let attr = |k: &str| attrs.and_then(|x| x.get(k));
        let fill = a.get("fill_value").and_then(num).or_else(|| attr("_FillValue").and_then(num));
        let m = ArrayMeta {
            shape: list("shape"),
            chunks: list("chunks"),
            dtype,
            fill,
            scale: attr("scale_factor").and_then(num).unwrap_or(1.0),
            offset: attr("add_offset").and_then(num).unwrap_or(0.0),
            dims: attr("_ARRAY_DIMENSIONS").and_then(|d| d.as_array()).map(|d| d.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default(),
            units: attr("units").and_then(|u| u.as_str()).unwrap_or_default().to_string(),
            blosc,
            separator: a.get("dimension_separator").and_then(|s| s.as_str()).unwrap_or(".").to_string(),
        };
        if m.shape.len() != m.chunks.len() || m.shape.is_empty() || m.chunks.contains(&0) {
            continue;
        }
        out.insert(name.to_string(), m);
    }
    Ok(out)
}

impl ArrayMeta {
    /// How many chunks along `axis`.
    pub fn n_chunks(&self, axis: usize) -> usize {
        self.shape[axis].div_ceil(self.chunks[axis])
    }

    /// The key of the chunk at chunk indices `idx`: "uo/12.0.96.263".
    pub fn key(&self, name: &str, idx: &[usize]) -> String {
        let parts: Vec<String> = idx.iter().map(|i| i.to_string()).collect();
        format!("{name}/{}", parts.join(&self.separator))
    }

    /// Elements in a whole chunk.
    pub fn chunk_len(&self) -> usize {
        self.chunks.iter().product()
    }

    /// The uncompressed bytes of a C-order chunk that hold the steps `r` of its first axis, every other
    /// axis whole: in a time-major chunk, those hours.
    pub fn leading_bytes(&self, r: Range<usize>) -> Range<usize> {
        let per = self.chunks[1..].iter().product::<usize>() * self.dtype.size();
        r.start * per..r.end * per
    }

    /// Raw bytes as values: the fill value as NaN, then CF scale and offset.
    pub fn values(&self, raw: &[u8]) -> Vec<f32> {
        self.values64(raw).into_iter().map(|v| v as f32).collect()
    }

    /// `values` at full precision: for coordinates, such as seconds since 1970.
    pub fn values64(&self, raw: &[u8]) -> Vec<f64> {
        let size = self.dtype.size();
        raw.chunks_exact(size)
            .map(|b| {
                let v = self.dtype.value(b);
                if v.is_nan() || self.fill.is_some_and(|f| v == f || (f.abs() > 1e30 && (v - f).abs() <= f.abs() * 1e-6)) {
                    f64::NAN
                } else {
                    v * self.scale + self.offset
                }
            })
            .collect()
    }

    /// A whole chunk decoded to values.
    pub fn decode(&self, chunk: &[u8]) -> Result<Vec<f32>, ZarrError> {
        let raw = if self.blosc { blosc::decode(chunk)? } else { chunk.to_vec() };
        Ok(self.values(&raw))
    }
}

/// The times of a CF time axis: "hours since 1950-01-01", "seconds since 1970-01-01T00:00:00+00:00",
/// "minutes since 1900-01-01", "days since 1900-01-01 00:00:00".
pub fn times(units: &str, values: &[f64]) -> Option<Vec<DateTime<Utc>>> {
    let (unit, since) = units.split_once(" since ")?;
    let seconds = match unit.trim() {
        "seconds" | "second" | "s" => 1.0,
        "minutes" | "minute" => 60.0,
        "hours" | "hour" | "h" => 3600.0,
        "days" | "day" | "d" => 86400.0,
        _ => return None,
    };
    let since = since.trim().trim_end_matches('Z').trim_end_matches("+00:00").trim_end_matches(" UTC").replace('T', " ");
    let epoch = NaiveDateTime::parse_from_str(&since, "%Y-%m-%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(&since, "%Y-%m-%d %H:%M"))
        .or_else(|_| NaiveDate::parse_from_str(&since, "%Y-%m-%d").map(|d| d.and_hms_opt(0, 0, 0).unwrap_or_default()))
        .ok()?
        .and_utc();
    Some(values.iter().map(|v| epoch + Duration::milliseconds((v * seconds * 1000.0).round() as i64)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    const META: &str = r#"{"metadata": {
        ".zattrs": {"title": "test"},
        "uo/.zarray": {"shape": [53016, 1, 2041, 4320], "chunks": [4272, 1, 16, 8], "dtype": "<f4", "order": "C", "filters": null,
            "fill_value": 9.969209968386869e+36, "compressor": {"id": "blosc", "cname": "lz4", "clevel": 5, "shuffle": 1, "blocksize": 0}, "zarr_format": 2},
        "uo/.zattrs": {"_ARRAY_DIMENSIONS": ["time", "elevation", "latitude", "longitude"], "units": "m s-1"},
        "vo/.zarray": {"shape": [27456, 1240, 958], "chunks": [3720, 16, 16], "dtype": "<i2", "order": "C", "filters": null,
            "fill_value": -32768, "compressor": {"id": "blosc", "cname": "lz4", "clevel": 5, "shuffle": 1, "blocksize": 0}, "zarr_format": 2},
        "vo/.zattrs": {"_ARRAY_DIMENSIONS": ["time", "latitude", "longitude"], "scale_factor": 0.001, "add_offset": 0.0, "units": "m s-1"},
        "time/.zarray": {"shape": [53016], "chunks": [21312], "dtype": "<f4", "order": "C", "filters": null, "fill_value": "NaN", "compressor": null, "zarr_format": 2},
        "time/.zattrs": {"_ARRAY_DIMENSIONS": ["time"], "units": "hours since 1950-01-01"},
        "odd/.zarray": {"shape": [3], "chunks": [3], "dtype": "|u1", "order": "C", "filters": null, "fill_value": 0, "compressor": null, "zarr_format": 2}
    }}"#;

    #[test]
    fn the_metadata_gives_shapes_keys_and_packing() {
        let m = parse_metadata(META.as_bytes()).unwrap();
        assert!(!m.contains_key("odd"), "a dtype it cannot read is left out");
        let uo = &m["uo"];
        assert_eq!((uo.n_chunks(0), uo.n_chunks(2), uo.n_chunks(3)), (13, 128, 540));
        assert_eq!(uo.key("uo", &[12, 0, 96, 263]), "uo/12.0.96.263");
        assert_eq!(uo.dims[1], "elevation");
        // Hours 1536 to 1752 of a chunk: each hour is 16 × 8 cells of four bytes.
        assert_eq!(uo.leading_bytes(1536..1752), 1536 * 512..1752 * 512);
        let vo = &m["vo"];
        assert_eq!(vo.values(&[0x10, 0x27, 0x00, 0x80, 0xf0, 0xd8]).iter().map(|v| if v.is_nan() { None } else { Some((v * 1000.0).round() as i32) }).collect::<Vec<_>>(), vec![Some(10_000), None, Some(-10_000)], "10 000 mm/s, the fill value, -10 000");
        let fill = 9.969_21e36_f32.to_le_bytes();
        assert!(uo.values(&fill)[0].is_nan(), "the float fill value");
        assert!(m["time"].fill.unwrap().is_nan() && !m["time"].blosc);
    }

    #[test]
    fn cf_time_axes_in_every_unit_the_stores_use() {
        let at = |y, mo, d, h| Utc.with_ymd_and_hms(y, mo, d, h, 0, 0).unwrap();
        assert_eq!(times("hours since 1950-01-01", &[0.0, 24.0]).unwrap(), vec![at(1950, 1, 1, 0), at(1950, 1, 2, 0)]);
        assert_eq!(times("seconds since 1970-01-01T00:00:00+00:00", &[3600.0]).unwrap(), vec![at(1970, 1, 1, 1)]);
        assert_eq!(times("minutes since 1900-01-01 00:00:00", &[60.0]).unwrap(), vec![at(1900, 1, 1, 1)]);
        assert_eq!(times("days since 1900-01-01", &[1.5]).unwrap(), vec![at(1900, 1, 2, 12)]);
        assert!(times("fortnights since 1900-01-01", &[1.0]).is_none());
        assert!(times("hours", &[1.0]).is_none());
    }
}
