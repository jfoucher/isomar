//! Reading GRIB2 files into [`Field`]s.

use super::model::{Field, Param};
use crate::chart::grid::Grid;
use chrono::{DateTime, Utc};
use grib::{Grib2SubmessageDecoder, LatLons};

#[derive(Debug, PartialEq, Eq)]
pub enum GribError {
    /// The data is not GRIB2 or is damaged.
    Unreadable(String),
}

impl std::fmt::Display for GribError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GribError::Unreadable(s) => write!(f, "cannot read GRIB data: {s}"),
        }
    }
}

impl std::error::Error for GribError {}

/// Checks the framing of a GRIB2 file: it must start with `GRIB` and end with
/// `7777`. Used to reject truncated downloads before they reach the cache.
pub fn looks_complete(bytes: &[u8]) -> bool {
    bytes.len() >= 20 && &bytes[..4] == b"GRIB" && &bytes[bytes.len() - 4..] == b"7777"
}

fn norm360(l: f64) -> f64 {
    l.rem_euclid(360.0)
}

/// Decodes every message we understand. Messages for parameters we do not use
/// are skipped; a message that fails to decode is skipped too, so one bad
/// field does not lose the rest of a file. Returns an error only when nothing
/// could be read at all.
pub fn read_fields(bytes: &[u8]) -> Result<Vec<Field>, GribError> {
    let g = grib::from_bytes(bytes.to_vec()).map_err(|e| GribError::Unreadable(e.to_string()))?;
    let mut out = Vec::new();
    let mut seen = 0usize;
    for (_, sub) in g.iter() {
        seen += 1;
        let pd = sub.prod_def();
        let (Some(cat), Some(num)) = (pd.parameter_category(), pd.parameter_number()) else { continue };
        let surface = pd.fixed_surfaces().map(|(a, _)| (a.surface_type, a.value()));
        let Some(param) = Param::from_grib(sub.indicator().discipline, cat, num, surface) else { continue };
        let Ok((ni, nj)) = sub.grid_shape() else { continue };
        if ni < 2 || nj < 2 {
            continue;
        }
        let t = sub.temporal_info();
        let Some(valid) = period_end(&sub).or(t.forecast_time_target) else { continue };
        let Ok(latlons) = sub.latlons() else { continue };
        let pts: Vec<(f64, f64)> = latlons.collect();
        if pts.len() != ni * nj {
            continue;
        }
        let Ok(dec) = Grib2SubmessageDecoder::from(sub) else { continue };
        let Ok(vals) = dec.dispatch() else { continue };
        let values: Vec<f32> = vals.collect();
        if values.len() != ni * nj {
            continue;
        }
        if let Some(f) = build_field(param, valid, t.ref_time, ni, nj, &pts, values) {
            out.push(f);
        }
    }
    if out.is_empty() && seen == 0 {
        return Err(GribError::Unreadable("no messages found".into()));
    }
    Ok(out)
}

/// When a field is a statistic over a period (template 4.8: ECMWF's gusts are the strongest of the
/// past hour or three), the end of the period: the time it is the forecast for. The start, which is
/// the forecast time in the template, would put it an hour or more early.
fn period_end<R>(sub: &grib::SubMessage<'_, R>) -> Option<DateTime<Utc>> {
    use chrono::TimeZone;
    let grib::def::grib2::ProductDefinitionTemplate::_4_8(t) = sub.section4().ok()?.payload.template else { return None };
    let e = t.time_interval.end_of_overall_time_interval;
    Utc.with_ymd_and_hms(e.year as i32, e.month as u32, e.day as u32, e.hour as u32, e.minute as u32, e.second as u32).single()
}

/// Normalises a decoded grid to rows running north to south and columns west
/// to east, and wraps it in a [`Grid`].
fn build_field(param: Param, valid: DateTime<Utc>, run: Option<DateTime<Utc>>, ni: usize, nj: usize, pts: &[(f64, f64)], mut values: Vec<f32>) -> Option<Field> {
    let (lat0, lon0) = pts[0];
    let lat_next_row = pts[ni].0;
    let lon_next_col = pts[1].1;
    let dlat_signed = lat_next_row - lat0;
    let dlon_signed = {
        let d = norm360(lon_next_col - lon0);
        if d > 180.0 { d - 360.0 } else { d }
    };
    if dlat_signed == 0.0 || dlon_signed == 0.0 {
        return None;
    }
    let (dlat, dlon) = (dlat_signed.abs(), dlon_signed.abs());

    // Flip so that row 0 is the northernmost and column 0 the westernmost.
    if dlat_signed > 0.0 {
        for r in 0..nj / 2 {
            for c in 0..ni {
                values.swap(r * ni + c, (nj - 1 - r) * ni + c);
            }
        }
    }
    if dlon_signed < 0.0 {
        for r in 0..nj {
            values[r * ni..(r + 1) * ni].reverse();
        }
    }
    let lat_top = if dlat_signed > 0.0 { lat0 + dlat * (nj - 1) as f64 } else { lat0 };
    let lon_left = if dlon_signed < 0.0 { lon0 - dlon * (ni - 1) as f64 } else { lon0 };
    // Missing values come out as NaN already; GRIB "no data" sentinels are rare
    // in the sources we use, but guard against absurd magnitudes.
    for v in values.iter_mut() {
        if !v.is_finite() || v.abs() > 1.0e15 {
            *v = f32::NAN;
        }
    }
    let lat_bottom = lat_top - dlat * (nj - 1) as f64;
    Some(Field {
        param,
        time: valid,
        run,
        grid: Grid {
            ncols: ni,
            nrows: nj,
            // Grid cells are centred on the data points.
            xll: lon_left - dlon / 2.0,
            yll: lat_bottom - dlat / 2.0,
            dx: dlon,
            dy: dlat,
            data: values,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::weather::testgrib::{TestField, encode};
    use chrono::TimeZone;

    fn ramp(n: usize) -> Vec<f32> {
        (0..n).map(|i| i as f32 * 0.5 - 3.0).collect()
    }

    #[test]
    fn reads_a_wind_field_with_its_time_and_grid() {
        let vals = ramp(12); // 4 columns, 3 rows
        let bytes = encode(&TestField::wind_u(6, 4, 3, vals.clone()));
        assert!(looks_complete(&bytes));
        let fields = read_fields(&bytes).unwrap();
        assert_eq!(fields.len(), 1);
        let f = &fields[0];
        assert_eq!(f.param, Param::WindU);
        // Run 2026-06-01 00Z plus 6 hours.
        assert_eq!(f.time, Utc.with_ymd_and_hms(2026, 6, 1, 6, 0, 0).unwrap());
        assert_eq!(f.run, Some(Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap()));
        let g = &f.grid;
        assert_eq!((g.ncols, g.nrows), (4, 3));
        assert!((g.dx - 0.25).abs() < 1e-6 && (g.dy - 0.25).abs() < 1e-6);
        // First point is 48N 350E (10W, in either convention); data points sit at cell centres.
        assert!((norm360(g.lon_at(0.0)) - 350.0).abs() < 1e-4, "{}", g.lon_at(0.0));
        assert!((g.lat_at(0.0) - 48.0).abs() < 1e-4);
        // Values survive packing at 0.01 resolution.
        for (a, b) in g.data.iter().zip(&vals) {
            assert!((a - b).abs() < 0.011, "{a} vs {b}");
        }
    }

    #[test]
    fn south_to_north_scanning_is_flipped_to_north_first() {
        let mut t = TestField::wind_u(0, 2, 3, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        t.lat_first = 47.5; // southernmost row first
        t.scan_mode = 64;
        let f = &read_fields(&encode(&t)).unwrap()[0];
        // Rows were south..north = [1,2],[3,4],[5,6]; north first means [5,6] on top.
        assert!((f.grid.data[0] - 5.0).abs() < 0.011 && (f.grid.data[5] - 2.0).abs() < 0.011, "{:?}", f.grid.data);
        assert!((f.grid.lat_at(0.0) - 48.0).abs() < 1e-4);
        assert!((f.grid.lat_at(2.0) - 47.5).abs() < 1e-4);
    }

    #[test]
    fn several_messages_and_unknown_parameters() {
        let mut data = encode(&TestField::wind_u(0, 3, 3, ramp(9)));
        data.extend(encode(&TestField::wind_u(3, 3, 3, ramp(9))));
        // Parameter we do not use: 0,1,1 relative humidity.
        let mut other = TestField::wind_u(0, 3, 3, ramp(9));
        other.category = 1;
        other.number = 1;
        data.extend(encode(&other));
        // A MSLP field.
        let mut p = TestField::wind_u(0, 3, 3, vec![1013.0; 9]);
        p.category = 3;
        p.number = 1;
        p.surface_type = 101;
        p.surface_value = 0;
        data.extend(encode(&p));
        let fields = read_fields(&data).unwrap();
        assert_eq!(fields.len(), 3);
        let params: Vec<_> = fields.iter().map(|f| f.param).collect();
        assert_eq!(params, vec![Param::WindU, Param::WindU, Param::Pressure]);
        assert!((fields[2].grid.data[0] - 1013.0).abs() < 0.011);
    }

    #[test]
    fn garbage_and_truncated_files_are_errors_not_panics() {
        assert!(read_fields(b"hello world, definitely not grib").is_err());
        assert!(read_fields(&[]).is_err());
        let good = encode(&TestField::wind_u(0, 3, 3, ramp(9)));
        let cut = &good[..good.len() - 30];
        assert!(!looks_complete(cut));
        // A truncated message must not crash; whether it errors or yields nothing is fine.
        let r = read_fields(cut);
        assert!(r.is_err() || r.unwrap().is_empty());
    }

    #[test]
    fn completeness_check() {
        assert!(!looks_complete(b""));
        assert!(!looks_complete(b"GRIB1234567890123456"));
        assert!(!looks_complete(b"XXXX1234567890123456789 7777"));
        let mut ok = b"GRIB".to_vec();
        ok.extend([0u8; 20]);
        ok.extend(b"7777");
        assert!(looks_complete(&ok));
    }

    #[test]
    fn constant_fields_pack_and_unpack() {
        let f = &read_fields(&encode(&TestField::wind_u(0, 3, 3, vec![7.25; 9]))).unwrap()[0];
        assert!(f.grid.data.iter().all(|v| (v - 7.25).abs() < 0.011));
    }
}
