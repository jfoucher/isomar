//! AIS (ITU-R M.1371) message decoding from `!AIVDM`/`!AIVDO` sentences, and
//! just enough encoding for the simulator and tests.

use super::nmea::{Sentence, frame};
use crate::geo::LatLon;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AisClass {
    A,
    B,
    Sar,
    Base,
    Aton,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Dims {
    pub bow: u16,
    pub stern: u16,
    pub port: u8,
    pub starboard: u8,
}

impl Dims {
    pub fn length(&self) -> u16 {
        self.bow + self.stern
    }

    pub fn beam(&self) -> u16 {
        self.port as u16 + self.starboard as u16
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum AisBody {
    Position {
        class: AisClass,
        status: Option<u8>,
        sog_kn: Option<f64>,
        cog: Option<f64>,
        heading: Option<u16>,
        pos: Option<LatLon>,
    },
    Static {
        name: Option<String>,
        callsign: Option<String>,
        ship_type: Option<u8>,
        dims: Option<Dims>,
        destination: Option<String>,
        draught_m: Option<f64>,
        imo: Option<u32>,
    },
    Aton { name: Option<String>, aton_type: u8, pos: Option<LatLon> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct AisMessage {
    pub msg_type: u8,
    pub mmsi: u32,
    pub body: AisBody,
}

/// Payload characters to a bit vector (one bit per element), dropping `fill` pad bits.
pub fn bits_from_payload(payload: &str, fill: usize) -> Option<Vec<u8>> {
    let mut bits = Vec::with_capacity(payload.len() * 6);
    for c in payload.bytes() {
        let mut v = c.checked_sub(48)?;
        if v > 40 {
            v = v.checked_sub(8)?;
        }
        if v > 63 {
            return None;
        }
        for i in (0..6).rev() {
            bits.push((v >> i) & 1);
        }
    }
    let keep = bits.len().checked_sub(fill)?;
    bits.truncate(keep);
    Some(bits)
}

fn uval(b: &[u8], start: usize, len: usize) -> Option<u64> {
    let slice = b.get(start..start + len)?;
    Some(slice.iter().fold(0u64, |a, bit| (a << 1) | *bit as u64))
}

fn sval(b: &[u8], start: usize, len: usize) -> Option<i64> {
    let u = uval(b, start, len)?;
    let sign = 1u64 << (len - 1);
    Some(if u & sign != 0 { u as i64 - (1i64 << len) } else { u as i64 })
}

fn text(b: &[u8], start: usize, len_bits: usize) -> Option<String> {
    let mut s = String::new();
    for i in 0..len_bits / 6 {
        let v = uval(b, start + i * 6, 6)? as u8;
        s.push((if v < 32 { v + 64 } else { v }) as char);
    }
    let t = s.trim_end_matches(['@', ' ']).trim().to_string();
    (!t.is_empty()).then_some(t)
}

fn lonlat(lon_raw: Option<i64>, lat_raw: Option<i64>, divisor: f64) -> Option<LatLon> {
    let (lon, lat) = (lon_raw? as f64 / divisor, lat_raw? as f64 / divisor);
    // 181 and 91 mean "not available".
    ((-180.0..=180.0).contains(&lon) && (-90.0..=90.0).contains(&lat)).then(|| LatLon::new(lat, lon))
}

fn dims(b: &[u8], at: usize) -> Option<Dims> {
    let d = Dims {
        bow: uval(b, at, 9)? as u16,
        stern: uval(b, at + 9, 9)? as u16,
        port: uval(b, at + 18, 6)? as u8,
        starboard: uval(b, at + 24, 6)? as u8,
    };
    (d.length() > 0 || d.beam() > 0).then_some(d)
}

/// Decodes a complete payload. `None` for unsupported or truncated messages.
pub fn decode_bits(b: &[u8]) -> Option<AisMessage> {
    let msg_type = uval(b, 0, 6)? as u8;
    let mmsi = uval(b, 8, 30)? as u32;
    let body = match msg_type {
        1..=3 => AisBody::Position {
            class: AisClass::A,
            status: Some(uval(b, 38, 4)? as u8).filter(|s| *s != 15),
            sog_kn: Some(uval(b, 50, 10)?).filter(|s| *s != 1023).map(|s| s as f64 / 10.0),
            cog: Some(uval(b, 116, 12)?).filter(|c| *c < 3600).map(|c| c as f64 / 10.0),
            heading: Some(uval(b, 128, 9)? as u16).filter(|h| *h < 360),
            pos: lonlat(sval(b, 61, 28), sval(b, 89, 27), 600_000.0),
        },
        4 => AisBody::Position {
            class: AisClass::Base,
            status: None,
            sog_kn: None,
            cog: None,
            heading: None,
            pos: lonlat(sval(b, 79, 28), sval(b, 107, 27), 600_000.0),
        },
        9 => AisBody::Position {
            class: AisClass::Sar,
            status: None,
            sog_kn: Some(uval(b, 50, 10)?).filter(|s| *s != 1023).map(|s| s as f64),
            cog: Some(uval(b, 116, 12)?).filter(|c| *c < 3600).map(|c| c as f64 / 10.0),
            heading: None,
            pos: lonlat(sval(b, 61, 28), sval(b, 89, 27), 600_000.0),
        },
        18 | 19 => AisBody::Position {
            class: AisClass::B,
            status: None,
            sog_kn: Some(uval(b, 46, 10)?).filter(|s| *s != 1023).map(|s| s as f64 / 10.0),
            cog: Some(uval(b, 112, 12)?).filter(|c| *c < 3600).map(|c| c as f64 / 10.0),
            heading: Some(uval(b, 124, 9)? as u16).filter(|h| *h < 360),
            pos: lonlat(sval(b, 57, 28), sval(b, 85, 27), 600_000.0),
        },
        27 => AisBody::Position {
            class: AisClass::A,
            status: Some(uval(b, 40, 4)? as u8).filter(|s| *s != 15),
            sog_kn: Some(uval(b, 79, 6)?).filter(|s| *s != 63).map(|s| s as f64),
            cog: Some(uval(b, 85, 9)?).filter(|c| *c < 360).map(|c| c as f64),
            heading: None,
            pos: lonlat(sval(b, 44, 18), sval(b, 62, 17), 600.0),
        },
        5 => AisBody::Static {
            imo: Some(uval(b, 40, 30)? as u32).filter(|i| *i != 0),
            callsign: text(b, 70, 42),
            name: text(b, 112, 120),
            ship_type: Some(uval(b, 232, 8)? as u8).filter(|t| *t != 0),
            dims: dims(b, 240),
            draught_m: Some(uval(b, 294, 8)?).filter(|d| *d != 0).map(|d| d as f64 / 10.0),
            destination: text(b, 302, 120),
        },
        21 => AisBody::Aton {
            aton_type: uval(b, 38, 5)? as u8,
            name: text(b, 43, 120),
            pos: lonlat(sval(b, 164, 28), sval(b, 192, 27), 600_000.0),
        },
        24 => match uval(b, 38, 2)? {
            0 => AisBody::Static { name: text(b, 40, 120), callsign: None, ship_type: None, dims: None, destination: None, draught_m: None, imo: None },
            1 => AisBody::Static {
                name: None,
                ship_type: Some(uval(b, 40, 8)? as u8).filter(|t| *t != 0),
                callsign: text(b, 90, 42),
                dims: dims(b, 132),
                destination: None,
                draught_m: None,
                imo: None,
            },
            _ => return None,
        },
        _ => return None,
    };
    // Class B "extended" (type 19) also carries the name.
    Some(AisMessage { msg_type, mmsi, body })
}

/// Fragments received so far for one message: how many it has, and each part.
type Pending = (usize, Vec<Option<String>>);

/// Collects the fragments of multi-sentence messages.
#[derive(Default)]
pub struct Assembler {
    pending: HashMap<(String, String), Pending>,
}

impl Assembler {
    /// Feeds one `!AIVDM` sentence; returns a message when it is complete.
    pub fn push(&mut self, s: &Sentence) -> Option<AisMessage> {
        let total: usize = s.field(0).parse().ok()?;
        let num: usize = s.field(1).parse().ok()?;
        let fill: usize = s.field(5).parse().unwrap_or(0);
        let payload = s.field(4);
        if total == 0 || num == 0 || num > total {
            return None;
        }
        if total == 1 {
            return decode_bits(&bits_from_payload(payload, fill)?);
        }
        let key = (s.field(2).to_string(), s.field(3).to_string());
        if self.pending.len() > 32 {
            self.pending.clear();
        }
        let entry = self.pending.entry(key.clone()).or_insert_with(|| (total, vec![None; total]));
        if entry.0 != total {
            *entry = (total, vec![None; total]);
        }
        entry.1[num - 1] = Some(payload.to_string());
        if entry.1.iter().all(|p| p.is_some()) {
            let (_, parts) = self.pending.remove(&key)?;
            let joined: String = parts.into_iter().flatten().collect();
            // The fill bits belong to the last fragment.
            return decode_bits(&bits_from_payload(&joined, fill)?);
        }
        None
    }
}


/// Plain-language ship type for an AIS ship-type code.
pub fn ship_type_name(code: u8) -> &'static str {
    match code {
        20..=29 => "Wing in ground",
        30 => "Fishing",
        31 | 32 => "Towing",
        33 => "Dredging",
        34 => "Diving operations",
        35 => "Military",
        36 => "Sailing",
        37 => "Pleasure craft",
        40..=49 => "High-speed craft",
        50 => "Pilot vessel",
        51 => "Search and rescue",
        52 => "Tug",
        53 => "Port tender",
        54 => "Anti-pollution",
        55 => "Law enforcement",
        58 => "Medical transport",
        60..=69 => "Passenger",
        70..=79 => "Cargo",
        80..=89 => "Tanker",
        90..=99 => "Other",
        _ => "Unknown",
    }
}

/// Plain-language navigational status.
pub fn nav_status_name(code: u8) -> &'static str {
    match code {
        0 => "Under way using engine",
        1 => "At anchor",
        2 => "Not under command",
        3 => "Restricted manoeuvrability",
        4 => "Constrained by draught",
        5 => "Moored",
        6 => "Aground",
        7 => "Engaged in fishing",
        8 => "Under way sailing",
        _ => "Not defined",
    }
}

// Encoding

struct BitWriter(Vec<u8>);

impl BitWriter {
    fn new() -> Self {
        BitWriter(Vec::new())
    }

    fn u(&mut self, v: u64, len: usize) {
        for i in (0..len).rev() {
            self.0.push(((v >> i) & 1) as u8);
        }
    }

    fn i(&mut self, v: i64, len: usize) {
        self.u((v as u64) & ((1u64 << len) - 1), len);
    }

    fn text(&mut self, s: &str, chars: usize) {
        let up = s.to_ascii_uppercase();
        let mut it = up.bytes();
        for _ in 0..chars {
            let c = it.next().unwrap_or(b'@');
            let v = if (64..96).contains(&c) { c - 64 } else if (32..64).contains(&c) { c } else { 0 };
            self.u(v as u64, 6);
        }
    }

    /// 6-bit payload characters and the number of pad bits.
    fn payload(mut self) -> (String, usize) {
        let fill = (6 - self.0.len() % 6) % 6;
        self.0.extend(std::iter::repeat_n(0, fill));
        let s = self
            .0
            .chunks(6)
            .map(|c| {
                let v = c.iter().fold(0u8, |a, b| (a << 1) | b);
                (if v < 40 { v + 48 } else { v + 56 }) as char
            })
            .collect();
        (s, fill)
    }
}

pub struct PositionReport {
    pub mmsi: u32,
    pub pos: LatLon,
    pub sog_kn: f64,
    pub cog: f64,
    pub heading: Option<u16>,
    pub status: u8,
}

/// Class A position report (type 1).
pub fn encode_position_a(r: &PositionReport) -> (String, usize) {
    let mut w = BitWriter::new();
    w.u(1, 6);
    w.u(0, 2);
    w.u(r.mmsi as u64, 30);
    w.u(r.status as u64, 4);
    w.i(-128, 8);
    w.u((r.sog_kn * 10.0).round() as u64, 10);
    w.u(1, 1);
    w.i((r.pos.lon * 600_000.0).round() as i64, 28);
    w.i((r.pos.lat * 600_000.0).round() as i64, 27);
    w.u((r.cog * 10.0).round() as u64, 12);
    w.u(r.heading.map(|h| h as u64).unwrap_or(511), 9);
    w.u(60, 6);
    w.u(0, 2);
    w.u(0, 3);
    w.u(0, 1);
    w.u(0, 19);
    w.payload()
}

/// Class B position report (type 18).
pub fn encode_position_b(r: &PositionReport) -> (String, usize) {
    let mut w = BitWriter::new();
    w.u(18, 6);
    w.u(0, 2);
    w.u(r.mmsi as u64, 30);
    w.u(0, 8);
    w.u((r.sog_kn * 10.0).round() as u64, 10);
    w.u(1, 1);
    w.i((r.pos.lon * 600_000.0).round() as i64, 28);
    w.i((r.pos.lat * 600_000.0).round() as i64, 27);
    w.u((r.cog * 10.0).round() as u64, 12);
    w.u(r.heading.map(|h| h as u64).unwrap_or(511), 9);
    w.u(60, 6);
    w.u(0, 8);
    w.u(0, 20);
    w.payload()
}

/// Static and voyage data (type 5).
pub fn encode_static(mmsi: u32, name: &str, callsign: &str, ship_type: u8, dims: Dims, destination: &str) -> (String, usize) {
    let mut w = BitWriter::new();
    w.u(5, 6);
    w.u(0, 2);
    w.u(mmsi as u64, 30);
    w.u(0, 2);
    w.u(0, 30);
    w.text(callsign, 7);
    w.text(name, 20);
    w.u(ship_type as u64, 8);
    w.u(dims.bow as u64, 9);
    w.u(dims.stern as u64, 9);
    w.u(dims.port as u64, 6);
    w.u(dims.starboard as u64, 6);
    w.u(1, 4);
    w.u(0, 4);
    w.u(0, 5);
    w.u(24, 5);
    w.u(60, 6);
    w.u(0, 8);
    w.text(destination, 20);
    w.u(0, 1);
    w.u(0, 1);
    w.payload()
}

/// Splits a payload into `!AIVDM` sentences of at most 60 characters.
pub fn to_sentences(payload: &str, fill: usize, channel: char, seq: u8) -> Vec<String> {
    let chunks: Vec<&str> = payload.as_bytes().chunks(60).map(|c| std::str::from_utf8(c).unwrap_or("")).collect();
    let n = chunks.len();
    chunks
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let seq_field = if n > 1 { seq.to_string() } else { String::new() };
            let f = if i + 1 == n { fill } else { 0 };
            frame(&format!("AIVDM,{n},{},{seq_field},{channel},{c},{f}", i + 1), '!')
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comm::nmea::parse_sentence;

    fn decode_line(a: &mut Assembler, line: &str) -> Option<AisMessage> {
        a.push(&parse_sentence(line).unwrap())
    }

    #[test]
    fn decodes_the_published_class_a_example() {
        // The sample from the gpsd AIVDM documentation: MMSI 477553000 moored off Seattle.
        let mut a = Assembler::default();
        let m = decode_line(&mut a, "!AIVDM,1,1,,B,177KQJ5000G?tO`K>RA1wUbN0TKH,0*5C").unwrap();
        assert_eq!((m.msg_type, m.mmsi), (1, 477_553_000));
        let AisBody::Position { class, status, sog_kn, cog, heading, pos } = m.body else { panic!() };
        assert_eq!(class, AisClass::A);
        assert_eq!(status, Some(5), "moored");
        assert_eq!(sog_kn, Some(0.0));
        assert_eq!(cog, Some(51.0));
        assert_eq!(heading, Some(181));
        let p = pos.unwrap();
        assert!((p.lon + 122.345_832).abs() < 1e-5 && (p.lat - 47.582_833).abs() < 1e-5, "{p:?}");
    }

    #[test]
    fn decodes_the_published_two_part_static_example() {
        // Type 5 for EVER DIADEM from the same documentation.
        let mut a = Assembler::default();
        assert!(decode_line(&mut a, "!AIVDM,2,1,1,A,55?MbV02;H;s<HtKR20EHE:0@T4@Dn2222222216L961O5Gf0NSQEp6ClRp8,0*1C").is_none());
        let m = decode_line(&mut a, "!AIVDM,2,2,1,A,88888888880,2*25").unwrap();
        assert_eq!((m.msg_type, m.mmsi), (5, 351_759_000));
        let AisBody::Static { name, callsign, ship_type, destination, dims, .. } = m.body else { panic!() };
        assert_eq!(name.as_deref(), Some("EVER DIADEM"));
        assert_eq!(callsign.as_deref(), Some("3FOF8"));
        assert_eq!(ship_type, Some(70));
        assert_eq!(destination.as_deref(), Some("NEW YORK"));
        assert_eq!(dims.unwrap().length(), 295);
    }

    #[test]
    fn fragments_may_arrive_out_of_order_and_interleave() {
        let (p, fill) = encode_static(123_456_789, "TEST BOAT", "FX1234", 36, Dims { bow: 8, stern: 4, port: 2, starboard: 2 }, "BREST");
        let s = to_sentences(&p, fill, 'A', 3);
        assert_eq!(s.len(), 2);
        let (p2, f2) = encode_static(987_654_321, "OTHER", "FX9", 60, Dims::default(), "");
        let s2 = to_sentences(&p2, f2, 'B', 3);
        let mut a = Assembler::default();
        // Different channels with the same sequence id don't mix.
        assert!(decode_line(&mut a, &s[1]).is_none());
        assert!(decode_line(&mut a, &s2[0]).is_none());
        let m = decode_line(&mut a, &s[0]).unwrap();
        assert_eq!(m.mmsi, 123_456_789);
        let m2 = decode_line(&mut a, &s2[1]).unwrap();
        assert_eq!(m2.mmsi, 987_654_321);
    }

    #[test]
    fn encode_decode_class_a_roundtrip_including_negatives() {
        let r = PositionReport { mmsi: 227_000_123, pos: LatLon::new(-33.8567, -70.2153), sog_kn: 12.3, cog: 359.9, heading: Some(0), status: 0 };
        let (p, fill) = encode_position_a(&r);
        let line = to_sentences(&p, fill, 'A', 0).remove(0);
        let m = decode_line(&mut Assembler::default(), &line).unwrap();
        let AisBody::Position { pos, sog_kn, cog, heading, status, .. } = m.body else { panic!() };
        let q = pos.unwrap();
        assert!((q.lat + 33.8567).abs() < 1e-5 && (q.lon + 70.2153).abs() < 1e-5);
        assert_eq!((sog_kn, cog, heading, status), (Some(12.3), Some(359.9), Some(0), Some(0)));
    }

    #[test]
    fn class_b_roundtrip_and_unavailable_values() {
        let r = PositionReport { mmsi: 235_000_001, pos: LatLon::new(50.7, -1.3), sog_kn: 5.0, cog: 90.0, heading: None, status: 0 };
        let (p, fill) = encode_position_b(&r);
        let line = to_sentences(&p, fill, 'B', 0).remove(0);
        let m = decode_line(&mut Assembler::default(), &line).unwrap();
        let AisBody::Position { class, heading, sog_kn, .. } = m.body else { panic!() };
        assert_eq!(class, AisClass::B);
        assert_eq!(heading, None, "511 means not available");
        assert_eq!(sog_kn, Some(5.0));
    }

    #[test]
    fn unavailable_position_is_none() {
        let mut w = BitWriter::new();
        w.u(1, 6);
        w.u(0, 2);
        w.u(1, 30);
        w.u(0, 4);
        w.i(0, 8);
        w.u(1023, 10);
        w.u(0, 1);
        w.i(181 * 600_000, 28);
        w.i(91 * 600_000, 27);
        w.u(3600, 12);
        w.u(511, 9);
        w.u(0, 25);
        let (p, fill) = w.payload();
        let m = decode_bits(&bits_from_payload(&p, fill).unwrap()).unwrap();
        let AisBody::Position { pos, sog_kn, cog, .. } = m.body else { panic!() };
        assert!(pos.is_none() && sog_kn.is_none() && cog.is_none());
    }

    #[test]
    fn garbage_is_rejected_not_panicked_on() {
        assert!(bits_from_payload("~~~", 0).is_none());
        assert!(bits_from_payload("ab", 99).is_none());
        let mut a = Assembler::default();
        let s = Sentence { prefix: '!', talker: "AI".into(), kind: "VDM".into(), fields: vec!["1".into(), "1".into(), "".into(), "A".into(), "1".into(), "0".into()] };
        assert!(a.push(&s).is_none(), "truncated payload");
        let bad = Sentence { fields: vec!["0".into(), "0".into()], ..s.clone() };
        assert!(a.push(&bad).is_none());
        let bad = Sentence { fields: vec!["2".into(), "3".into(), "".into(), "A".into(), "xx".into(), "0".into()], ..s };
        assert!(a.push(&bad).is_none());
    }

    #[test]
    fn ship_type_and_status_names() {
        assert_eq!(ship_type_name(36), "Sailing");
        assert_eq!(ship_type_name(70), "Cargo");
        assert_eq!(ship_type_name(79), "Cargo");
        assert_eq!(ship_type_name(85), "Tanker");
        assert_eq!(ship_type_name(0), "Unknown");
        assert_eq!(ship_type_name(10), "Unknown");
        assert_eq!(nav_status_name(5), "Moored");
        assert_eq!(nav_status_name(14), "Not defined");
    }

    #[test]
    fn six_bit_text_trims_padding() {
        let mut w = BitWriter::new();
        w.text("ab c", 8);
        let b = w.0;
        assert_eq!(text(&b, 0, 48).as_deref(), Some("AB C"));
        let mut w = BitWriter::new();
        w.text("", 4);
        assert_eq!(text(&w.0, 0, 24), None);
    }

    #[test]
    fn signed_fields_extend_correctly() {
        let mut w = BitWriter::new();
        w.i(-5, 8);
        w.i(5, 8);
        assert_eq!(sval(&w.0, 0, 8), Some(-5));
        assert_eq!(sval(&w.0, 8, 8), Some(5));
        assert_eq!(uval(&w.0, 0, 100), None);
    }

    #[test]
    fn long_payloads_split_in_two_sentences_with_fill_on_the_last() {
        let (p, fill) = encode_static(1, "A", "B", 0, Dims::default(), "");
        let s = to_sentences(&p, fill, 'A', 7);
        assert!(s[0].contains(",2,1,7,A,"));
        assert!(s[1].contains(&",2,2,7,A,".to_string()) && s[1].contains(&format!(",{fill}*")));
    }
}
