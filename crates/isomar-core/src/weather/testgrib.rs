//! A minimal GRIB2 writer for tests: regular lat/lon grid (template 3.0),
//! analysis/forecast at a point in time (4.0) and simple packing (5.0). It
//! lets the decoder be tested without network access or binary fixtures.

pub struct TestField {
    pub discipline: u8,
    pub category: u8,
    pub number: u8,
    pub surface_type: u8,
    pub surface_value: u32,
    /// Model run: year, month, day, hour.
    pub run: (u16, u8, u8, u8),
    pub forecast_hours: u32,
    pub ni: u32,
    pub nj: u32,
    pub lat_first: f64,
    pub lon_first: f64,
    pub dlat: f64,
    pub dlon: f64,
    /// 0: west to east and north to south. 64 adds "south to north".
    pub scan_mode: u8,
    /// In scan order.
    pub values: Vec<f32>,
}

impl TestField {
    /// A 10 m wind component on a small grid, scanning north to south.
    pub fn wind_u(hours: u32, ni: u32, nj: u32, values: Vec<f32>) -> TestField {
        TestField {
            discipline: 0,
            category: 2,
            number: 2,
            surface_type: 103,
            surface_value: 10,
            run: (2026, 6, 1, 0),
            forecast_hours: hours,
            ni,
            nj,
            lat_first: 48.0,
            lon_first: 350.0,
            dlat: 0.25,
            dlon: 0.25,
            scan_mode: 0,
            values,
        }
    }
}

fn be16(v: u16) -> [u8; 2] {
    v.to_be_bytes()
}

fn be32(v: u32) -> [u8; 4] {
    v.to_be_bytes()
}

/// GRIB's sign-and-magnitude 32-bit integer.
fn sm32(v: i64) -> [u8; 4] {
    let mag = v.unsigned_abs() as u32 & 0x7fff_ffff;
    be32(if v < 0 { mag | 0x8000_0000 } else { mag })
}

fn micro(deg: f64) -> i64 {
    (deg * 1_000_000.0).round() as i64
}

pub fn encode(f: &TestField) -> Vec<u8> {
    assert_eq!(f.values.len(), (f.ni * f.nj) as usize);
    let mut out = Vec::new();

    // Section 1: identification.
    let mut s1 = Vec::new();
    s1.extend(be32(21));
    s1.push(1);
    s1.extend(be16(7)); // centre
    s1.extend(be16(0));
    s1.push(2); // master table version
    s1.push(0);
    s1.push(1); // reference time = start of forecast
    s1.extend(be16(f.run.0));
    s1.extend([f.run.1, f.run.2, f.run.3, 0, 0]);
    s1.push(0); // operational
    s1.push(1); // forecast products

    // Section 3: grid definition, template 3.0.
    let mut s3 = Vec::new();
    s3.extend(be32(72));
    s3.push(3);
    s3.push(0);
    s3.extend(be32(f.ni * f.nj));
    s3.push(0);
    s3.push(0);
    s3.extend(be16(0));
    s3.push(6); // shape of the earth: sphere of 6371229 m
    s3.extend([0u8; 15]); // scale factors and radii, unused for shape 6
    s3.extend(be32(f.ni));
    s3.extend(be32(f.nj));
    s3.extend(be32(0));
    s3.extend(be32(0xffff_ffff));
    let last_lat = f.lat_first + if f.scan_mode & 64 != 0 { 1.0 } else { -1.0 } * f.dlat * (f.nj - 1) as f64;
    let last_lon = f.lon_first + f.dlon * (f.ni - 1) as f64;
    s3.extend(sm32(micro(f.lat_first)));
    s3.extend(sm32(micro(f.lon_first)));
    s3.push(0x30);
    s3.extend(sm32(micro(last_lat)));
    s3.extend(sm32(micro(last_lon)));
    s3.extend(be32(micro(f.dlon) as u32));
    s3.extend(be32(micro(f.dlat) as u32));
    s3.push(f.scan_mode);
    assert_eq!(s3.len(), 72);

    // Section 4: product definition, template 4.0.
    let mut s4 = Vec::new();
    s4.extend(be32(34));
    s4.push(4);
    s4.extend(be16(0));
    s4.extend(be16(0));
    s4.push(f.category);
    s4.push(f.number);
    s4.push(2); // forecast
    s4.push(0);
    s4.push(0);
    s4.extend(be16(0));
    s4.push(0);
    s4.push(1); // hours
    s4.extend(be32(f.forecast_hours));
    s4.push(f.surface_type);
    s4.push(0);
    s4.extend(be32(f.surface_value));
    s4.push(255);
    s4.push(0);
    s4.extend(be32(0));
    assert_eq!(s4.len(), 34);

    // Packing: reference value, decimal scale 2, binary scale 0.
    let decimal = 2i32;
    let scale = 10f64.powi(decimal);
    let min = f.values.iter().cloned().fold(f32::INFINITY, f32::min) as f64;
    let reference = (min * scale).floor() as f32;
    let ints: Vec<u64> = f.values.iter().map(|v| ((*v as f64 * scale) - reference as f64).round().max(0.0) as u64).collect();
    let max = ints.iter().copied().max().unwrap_or(0);
    let bits = (64 - max.leading_zeros()).max(1) as usize;

    let mut s5 = Vec::new();
    s5.extend(be32(21));
    s5.push(5);
    s5.extend(be32(f.ni * f.nj));
    s5.extend(be16(0));
    s5.extend(reference.to_be_bytes());
    s5.extend(be16(0));
    s5.extend(be16(decimal as u16));
    s5.push(bits as u8);
    s5.push(0);

    let s6 = [0, 0, 0, 6, 6, 255];

    let mut packed = Vec::new();
    let (mut acc, mut nacc) = (0u64, 0usize);
    for v in &ints {
        acc = (acc << bits) | v;
        nacc += bits;
        while nacc >= 8 {
            packed.push(((acc >> (nacc - 8)) & 0xff) as u8);
            nacc -= 8;
        }
        acc &= (1u64 << nacc) - 1;
    }
    if nacc > 0 {
        packed.push(((acc << (8 - nacc)) & 0xff) as u8);
    }
    let mut s7 = Vec::new();
    s7.extend(be32(5 + packed.len() as u32));
    s7.push(7);
    s7.extend(packed);

    let body_len = 16 + s1.len() + s3.len() + s4.len() + s5.len() + s6.len() + s7.len() + 4;
    out.extend(b"GRIB");
    out.extend([0, 0]);
    out.push(f.discipline);
    out.push(2);
    out.extend((body_len as u64).to_be_bytes());
    out.extend(s1);
    out.extend(s3);
    out.extend(s4);
    out.extend(s5);
    out.extend(s6);
    out.extend(s7);
    out.extend(b"7777");
    assert_eq!(out.len(), body_len);
    out
}

#[test]
fn the_encoder_makes_messages_with_correct_lengths() {
    let m = encode(&TestField::wind_u(0, 3, 2, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]));
    assert_eq!(&m[..4], b"GRIB");
    assert_eq!(&m[m.len() - 4..], b"7777");
    assert_eq!(u64::from_be_bytes(m[8..16].try_into().unwrap()) as usize, m.len());
}
