//! Writing a [`Field`] as a GRIB2 message: what a model that only publishes whole-globe fields
//! (ECMWF) is cached as once it is cropped to the area asked for, so the cache keeps a few
//! hundred kilobytes where the download was tens of megabytes, and reads back like any other file.
//!
//! Regular lat/lon grid (template 3.0) scanned north to south, a forecast at a point in time (4.0;
//! a maximum over a period, such as ECMWF's gusts, is written at the end of its period), simple
//! packing (5.0) at the parameter's `decimals`, and a bitmap (section 6) where values are missing,
//! as the waves are over land.

use super::model::Field;

fn be16(v: u16) -> [u8; 2] {
    v.to_be_bytes()
}

fn be32(v: u32) -> [u8; 4] {
    v.to_be_bytes()
}

/// GRIB's sign-and-magnitude integers.
fn sm32(v: i64) -> [u8; 4] {
    let mag = v.unsigned_abs() as u32 & 0x7fff_ffff;
    be32(if v < 0 { mag | 0x8000_0000 } else { mag })
}

fn sm16(v: i16) -> [u8; 2] {
    let mag = v.unsigned_abs() & 0x7fff;
    be16(if v < 0 { mag | 0x8000 } else { mag })
}

fn micro(deg: f64) -> i64 {
    (deg * 1_000_000.0).round() as i64
}

/// Packs `bits`-wide integers most significant bit first.
fn pack(ints: impl Iterator<Item = u64>, bits: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut acc, mut n) = (0u64, 0usize);
    for v in ints {
        acc = (acc << bits) | v;
        n += bits;
        while n >= 8 {
            out.push(((acc >> (n - 8)) & 0xff) as u8);
            n -= 8;
        }
        acc &= (1u64 << n) - 1;
    }
    if n > 0 {
        out.push(((acc << (8 - n)) & 0xff) as u8);
    }
    out
}

/// One GRIB2 message holding `f`.
pub fn encode(f: &Field) -> Vec<u8> {
    let g = &f.grid;
    let (ni, nj) = (g.ncols as u32, g.nrows as u32);
    let run = f.run.unwrap_or(f.time);
    let lead = (f.time - run).num_minutes().max(0);
    // Whole hours where they are, minutes otherwise.
    let (unit, lead) = if lead % 60 == 0 { (1u8, lead / 60) } else { (0u8, lead) };
    let (discipline, category, number, surface_type, surface_value) = f.param.grib_codes();

    // Section 1: identification. Centre 255 (missing): the values are someone else's, cropped.
    let mut s1 = Vec::new();
    s1.extend(be32(21));
    s1.push(1);
    s1.extend(be16(255));
    s1.extend(be16(0));
    s1.push(2);
    s1.push(0);
    s1.push(1); // the reference time is the start of the forecast
    s1.extend(be16(run.format("%Y").to_string().parse().unwrap_or(1970)));
    for part in ["%m", "%d", "%H", "%M", "%S"] {
        s1.push(run.format(part).to_string().parse().unwrap_or(0));
    }
    s1.push(0);
    s1.push(1);

    // Section 3: template 3.0, the first point the north-west one.
    let lat_first = g.yll + g.dy / 2.0 + g.dy * (nj as f64 - 1.0);
    let lon_first = (g.xll + g.dx / 2.0).rem_euclid(360.0);
    let lat_last = g.yll + g.dy / 2.0;
    let lon_last = (lon_first + g.dx * (ni as f64 - 1.0)).rem_euclid(360.0);
    let mut s3 = Vec::new();
    s3.extend(be32(72));
    s3.push(3);
    s3.push(0);
    s3.extend(be32(ni * nj));
    s3.push(0);
    s3.push(0);
    s3.extend(be16(0));
    s3.push(6); // a sphere of 6371229 m
    s3.extend([0u8; 15]);
    s3.extend(be32(ni));
    s3.extend(be32(nj));
    s3.extend(be32(0));
    s3.extend(be32(0xffff_ffff));
    s3.extend(sm32(micro(lat_first)));
    s3.extend(sm32(micro(lon_first)));
    s3.push(0x30);
    s3.extend(sm32(micro(lat_last)));
    s3.extend(sm32(micro(lon_last)));
    s3.extend(be32(micro(g.dx) as u32));
    s3.extend(be32(micro(g.dy) as u32));
    s3.push(0); // west to east, north to south
    debug_assert_eq!(s3.len(), 72);

    // Section 4: template 4.0.
    let mut s4 = Vec::new();
    s4.extend(be32(34));
    s4.push(4);
    s4.extend(be16(0));
    s4.extend(be16(0));
    s4.push(category);
    s4.push(number);
    s4.push(2);
    s4.push(0);
    s4.push(0);
    s4.extend(be16(0));
    s4.push(0);
    s4.push(unit);
    s4.extend(be32(lead as u32));
    s4.push(surface_type);
    s4.push(0);
    s4.extend(be32(surface_value));
    s4.push(255);
    s4.push(0);
    s4.extend(be32(0));
    debug_assert_eq!(s4.len(), 34);

    // Section 5: simple packing of the values present.
    let decimal = f.param.decimals();
    let scale = 10f64.powi(decimal as i32);
    let present: Vec<f64> = g.data.iter().filter(|v| v.is_finite()).map(|v| *v as f64 * scale).collect();
    let reference = present.iter().copied().fold(f64::INFINITY, f64::min);
    let reference = if reference.is_finite() { reference.floor() } else { 0.0 };
    let ints: Vec<u64> = present.iter().map(|v| (v - reference).round().max(0.0) as u64).collect();
    let max = ints.iter().copied().max().unwrap_or(0);
    let bits = (64 - max.leading_zeros()) as usize;
    let mut s5 = Vec::new();
    s5.extend(be32(21));
    s5.push(5);
    s5.extend(be32(ints.len() as u32));
    s5.extend(be16(0));
    s5.extend((reference as f32).to_be_bytes());
    s5.extend(sm16(0));
    s5.extend(sm16(decimal));
    s5.push(bits as u8);
    s5.push(0);

    // Section 6: no bitmap when every value is there.
    let s6 = if ints.len() == g.data.len() {
        vec![0, 0, 0, 6, 6, 255]
    } else {
        let map = pack(g.data.iter().map(|v| v.is_finite() as u64), 1);
        let mut s = Vec::new();
        s.extend(be32(6 + map.len() as u32));
        s.push(6);
        s.push(0);
        s.extend(map);
        s
    };

    // Section 7. With every value the same there are no bits: the reference is the value.
    let packed = if bits == 0 { Vec::new() } else { pack(ints.into_iter(), bits) };
    let mut s7 = Vec::new();
    s7.extend(be32(5 + packed.len() as u32));
    s7.push(7);
    s7.extend(packed);

    let len = 16 + s1.len() + s3.len() + s4.len() + s5.len() + s6.len() + s7.len() + 4;
    let mut out = Vec::with_capacity(len);
    out.extend(b"GRIB");
    out.extend([0, 0]);
    out.push(discipline);
    out.push(2);
    out.extend((len as u64).to_be_bytes());
    for s in [s1, s3, s4, s5, s6, s7] {
        out.extend(s);
    }
    out.extend(b"7777");
    debug_assert_eq!(out.len(), len);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::grid::Grid;
    use crate::weather::grib::read_fields;
    use crate::weather::model::Param;
    use chrono::{Duration, TimeZone, Utc};

    fn field(param: Param, xll: f64, data: Vec<f32>, hours: i64) -> Field {
        let run = Utc.with_ymd_and_hms(2026, 11, 5, 12, 0, 0).unwrap();
        Field { param, time: run + Duration::hours(hours), run: Some(run), grid: Grid { ncols: 4, nrows: 3, xll, yll: 45.875, dx: 0.25, dy: 0.25, data } }
    }

    fn roundtrip(f: &Field) -> Field {
        let bytes = encode(f);
        assert!(crate::weather::grib::looks_complete(&bytes));
        let mut back = read_fields(&bytes).unwrap();
        assert_eq!(back.len(), 1);
        back.remove(0)
    }

    #[test]
    fn a_field_reads_back_with_its_grid_times_and_values() {
        let data: Vec<f32> = (0..12).map(|i| i as f32 * 1.37 - 4.0).collect();
        let f = field(Param::WindU, -5.125, data.clone(), 27);
        let b = roundtrip(&f);
        assert_eq!((b.param, b.time, b.run), (f.param, f.time, f.run));
        assert_eq!((b.grid.ncols, b.grid.nrows), (4, 3));
        // The same place, whichever way round the longitudes come back.
        assert!((b.grid.xll - f.grid.xll).rem_euclid(360.0) < 1e-6, "{}", b.grid.xll);
        assert!((b.grid.yll - f.grid.yll).abs() < 1e-6 && (b.grid.dx - 0.25).abs() < 1e-9);
        for (a, z) in data.iter().zip(&b.grid.data) {
            assert!((a - z).abs() < 0.006, "{a} {z}");
        }
    }

    #[test]
    fn missing_values_stay_missing() {
        let mut data = vec![1.5f32; 12];
        data[0] = f32::NAN;
        data[7] = f32::NAN;
        let b = roundtrip(&field(Param::WaveHeight, 10.0, data, 6));
        assert!(b.grid.data[0].is_nan() && b.grid.data[7].is_nan());
        assert_eq!(b.grid.data.iter().filter(|v| v.is_finite()).count(), 10);
        assert!(b.grid.data.iter().filter(|v| v.is_finite()).all(|v| (v - 1.5).abs() < 1e-3));
    }

    #[test]
    fn pressure_keeps_its_pascals_and_a_constant_field_its_value() {
        let data: Vec<f32> = (0..12).map(|i| 101_325.0 + i as f32 * 37.0).collect();
        let b = roundtrip(&field(Param::Pressure, 0.0, data.clone(), 0));
        assert!(data.iter().zip(&b.grid.data).all(|(a, z)| (a - z).abs() <= 1.0));
        let b = roundtrip(&field(Param::WindV, 0.0, vec![3.25; 12], 0));
        assert!(b.grid.data.iter().all(|v| (v - 3.25).abs() < 1e-4), "{:?}", b.grid.data);
    }
}
