//! Aids to navigation from SHOM's *Balisage maritime* GeoPackage: buoys,
//! beacons, landmarks and lights, with S-57 attribute names and codes.

use crate::geo::LatLon;
use crate::vector::Bounds;
use rusqlite::{Connection, OpenFlags, types::ValueRef};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AidKind {
    BuoyLateral,
    BuoyCardinal,
    BuoySpecial,
    BuoyIsolatedDanger,
    BuoySafeWater,
    BuoyInstallation,
    BeaconLateral,
    BeaconCardinal,
    BeaconSpecial,
    BeaconIsolatedDanger,
    BeaconSafeWater,
    Landmark,
}

impl AidKind {
    pub fn is_buoy(self) -> bool {
        matches!(
            self,
            AidKind::BuoyLateral
                | AidKind::BuoyCardinal
                | AidKind::BuoySpecial
                | AidKind::BuoyIsolatedDanger
                | AidKind::BuoySafeWater
                | AidKind::BuoyInstallation
        )
    }

    pub fn is_beacon(self) -> bool {
        matches!(
            self,
            AidKind::BeaconLateral
                | AidKind::BeaconCardinal
                | AidKind::BeaconSpecial
                | AidKind::BeaconIsolatedDanger
                | AidKind::BeaconSafeWater
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Aid {
    pub pos: LatLon,
    pub kind: AidKind,
    pub name: Option<String>,
    /// S-57 COLOUR codes (1 white, 2 black, 3 red, 4 green, 5 blue, 6 yellow, ...).
    pub colours: Vec<u8>,
    /// BOYSHP or BCNSHP.
    pub shape: Option<i32>,
    /// CATLAM, CATCAM, CATSPM or CATLMK depending on the kind.
    pub category: Option<i32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Light {
    pub pos: LatLon,
    pub colours: Vec<u8>,
    /// LITCHR code.
    pub character: Option<i32>,
    /// SIGGRP, such as `(2)`.
    pub group: Option<String>,
    pub period_s: Option<f64>,
    pub range_nm: Option<f64>,
    /// Sector limits in degrees true, seen from seaward, as in S-57.
    pub sector: Option<(f64, f64)>,
    pub height_m: Option<f64>,
}

/// What an anchorage symbol stands for (INT 1 N10 to N21).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnchorageKind {
    /// An anchorage, at a point or over an area.
    Anchorage,
    /// A berth for one vessel, named by its number.
    Berth,
    /// An area where anchoring is prohibited.
    Prohibited,
    /// An area where anchoring is restricted.
    Restricted,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Anchorage {
    pub kind: AnchorageKind,
    pub name: Option<String>,
    /// S-57 CATACH codes: 1 unrestricted, 2 deep water, 3 tanker, 4 explosives, 5 quarantine,
    /// 6 seaplane, 7 small craft, 8 small craft mooring area, 9 up to 24 hours.
    pub categories: Vec<u8>,
    /// Where its symbol goes: the point, or a point inside the area.
    pub pos: LatLon,
    /// The area, `[lon, lat]` rings (even-odd); empty for a point, and for a piece of an outline.
    pub rings: Vec<Vec<[f64; 2]>>,
    /// The outline to draw: the rings less the stretches a tile's edge cut them along.
    pub edges: Vec<Vec<[f64; 2]>>,
    /// A berth's swinging radius, in metres.
    pub radius_m: Option<f64>,
}

impl Anchorage {
    /// Whether it is for ships rather than yachts: tanker, explosives, quarantine, deep water.
    pub fn is_for_ships(&self) -> bool {
        !self.categories.is_empty() && self.categories.iter().all(|c| matches!(c, 2..=5))
    }

    pub fn contains(&self, p: [f64; 2]) -> bool {
        self.rings.iter().filter(|r| crate::vector::ring_contains(r, p)).count() % 2 == 1
    }

    /// Its box, the symbol's spot for a point.
    pub fn bounds(&self) -> Bounds {
        let mut b = Bounds::of(&[[self.pos.lon, self.pos.lat]]);
        self.rings.iter().chain(&self.edges).flatten().for_each(|p| b.extend(*p));
        b
    }

    /// Whether it is a piece of an outline with nothing to say which side the area is: a closed way a tile cut,
    /// which came as a line. It has no symbol.
    pub fn is_outline_only(&self) -> bool {
        self.rings.is_empty() && !self.edges.is_empty()
    }
}

#[derive(Default)]
pub struct AidsData {
    pub aids: Vec<Aid>,
    pub lights: Vec<Light>,
    pub anchorages: Vec<Anchorage>,
}

const TABLES: [(&str, AidKind); 11] = [
    ("boylat", AidKind::BuoyLateral),
    ("boycar", AidKind::BuoyCardinal),
    ("boyspp", AidKind::BuoySpecial),
    ("boyisd", AidKind::BuoyIsolatedDanger),
    ("boysaw", AidKind::BuoySafeWater),
    ("boyinb", AidKind::BuoyInstallation),
    ("bcnlat", AidKind::BeaconLateral),
    ("bcncar", AidKind::BeaconCardinal),
    ("bcnspp", AidKind::BeaconSpecial),
    ("bcnisd", AidKind::BeaconIsolatedDanger),
    ("lndmrk", AidKind::Landmark),
];

/// Point from a GeoPackage geometry blob (header + WKB). Only 2D points are
/// understood; anything else yields `None`.
pub fn parse_gpkg_point(b: &[u8]) -> Option<[f64; 2]> {
    if b.len() < 8 || &b[0..2] != b"GP" {
        return None;
    }
    let flags = b[3];
    if flags & 0b0010_0000 != 0 || flags & 0b0001_0000 != 0 {
        return None; // extended or empty geometry
    }
    let env = match (flags >> 1) & 0b111 {
        0 => 0,
        1 => 32,
        2 | 3 => 48,
        4 => 64,
        _ => return None,
    };
    let wkb = b.get(8 + env..)?;
    let little = *wkb.first()? == 1;
    let rd_u32 = |o: usize| {
        let s: [u8; 4] = wkb.get(o..o + 4)?.try_into().ok()?;
        Some(if little { u32::from_le_bytes(s) } else { u32::from_be_bytes(s) })
    };
    let rd_f64 = |o: usize| {
        let s: [u8; 8] = wkb.get(o..o + 8)?.try_into().ok()?;
        Some(if little { f64::from_le_bytes(s) } else { f64::from_be_bytes(s) })
    };
    if rd_u32(1)? != 1 {
        return None;
    }
    Some([rd_f64(5)?, rd_f64(13)?])
}

fn text(v: ValueRef) -> Option<String> {
    match v {
        ValueRef::Text(t) => {
            let s = String::from_utf8_lossy(t).trim().to_string();
            (!s.is_empty()).then_some(s)
        }
        ValueRef::Integer(i) => Some(i.to_string()),
        ValueRef::Real(r) => Some(r.to_string()),
        _ => None,
    }
}

fn int(v: ValueRef) -> Option<i32> {
    match v {
        ValueRef::Integer(i) => Some(i as i32),
        ValueRef::Real(r) => Some(r as i32),
        ValueRef::Text(t) => String::from_utf8_lossy(t).trim().parse().ok(),
        _ => None,
    }
}

fn real(v: ValueRef) -> Option<f64> {
    match v {
        ValueRef::Integer(i) => Some(i as f64),
        ValueRef::Real(r) => Some(r),
        ValueRef::Text(t) => String::from_utf8_lossy(t).trim().parse().ok(),
        _ => None,
    }
}

/// `"6,2,6"` to `[6, 2, 6]`.
pub fn parse_colours(s: &str) -> Vec<u8> {
    s.split(',').filter_map(|p| p.trim().parse().ok()).collect()
}

impl AidsData {
    pub fn load(path: &Path) -> Result<AidsData, rusqlite::Error> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let mut data = AidsData::default();
        for (table, kind) in TABLES {
            // A table missing from this GeoPackage is not an error.
            let Ok(mut stmt) = conn.prepare(&format!("SELECT * FROM \"{table}\"")) else { continue };
            let cols: Vec<String> = stmt.column_names().map(|s| s.to_string()).collect();
            let idx = |n: &str| cols.iter().position(|c| c == n);
            let (g, nm, co) = (idx("geom"), idx("objnam"), idx("colour"));
            let shape = idx("boyshp").or(idx("bcnshp"));
            let cat = idx("catlam").or(idx("catcam")).or(idx("catspm")).or(idx("catlmk"));
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                let Some(gi) = g else { break };
                let Some(pos) = (match row.get_ref(gi)? {
                    ValueRef::Blob(b) => parse_gpkg_point(b),
                    _ => None,
                }) else {
                    continue;
                };
                data.aids.push(Aid {
                    pos: LatLon::new(pos[1], pos[0]),
                    kind,
                    name: nm.and_then(|i| text(row.get_ref(i).ok()?)),
                    colours: co.and_then(|i| text(row.get_ref(i).ok()?)).map(|s| parse_colours(&s)).unwrap_or_default(),
                    shape: shape.and_then(|i| int(row.get_ref(i).ok()?)),
                    category: cat.and_then(|i| int(row.get_ref(i).ok()?)),
                });
            }
        }
        if let Ok(mut stmt) = conn.prepare("SELECT * FROM lights") {
            let cols: Vec<String> = stmt.column_names().map(|s| s.to_string()).collect();
            let idx = |n: &str| cols.iter().position(|c| c == n);
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                let Some(gi) = idx("geom") else { break };
                let ValueRef::Blob(b) = row.get_ref(gi)? else { continue };
                let Some(pos) = parse_gpkg_point(b) else { continue };
                let get = |n: &str| idx(n).and_then(|i| row.get_ref(i).ok());
                let s1 = get("sectr1").and_then(real);
                let s2 = get("sectr2").and_then(real);
                data.lights.push(Light {
                    pos: LatLon::new(pos[1], pos[0]),
                    colours: get("colour").and_then(text).map(|s| parse_colours(&s)).unwrap_or_default(),
                    character: get("litchr").and_then(int),
                    group: get("siggrp").and_then(text),
                    period_s: get("sigper").and_then(real),
                    range_nm: get("valnmr").and_then(real),
                    sector: s1.zip(s2),
                    height_m: get("height").and_then(real),
                });
            }
        }
        Ok(data)
    }

    pub fn aids_in(&self, b: &Bounds) -> impl Iterator<Item = &Aid> {
        self.aids.iter().filter(move |a| b.contains([a.pos.lon, a.pos.lat]))
    }

    pub fn lights_in(&self, b: &Bounds) -> impl Iterator<Item = &Light> {
        self.lights.iter().filter(move |l| b.contains([l.pos.lon, l.pos.lat]))
    }

    /// The anchorages whose point or area meets `b`.
    pub fn anchorages_in(&self, b: &Bounds) -> impl Iterator<Item = &Anchorage> {
        self.anchorages.iter().filter(move |a| a.bounds().intersects(b))
    }
}

fn colour_letter(c: u8) -> Option<&'static str> {
    Some(match c {
        1 => "W",
        2 => "B",
        3 => "R",
        4 => "G",
        5 => "Bu",
        6 => "Y",
        7 => "Gy",
        8 => "Br",
        9 => "Am",
        10 => "Vi",
        11 => "Or",
        12 => "Mg",
        13 => "Pk",
        _ => return None,
    })
}

/// Chart abbreviation for a LITCHR code.
pub fn character_abbrev(code: i32) -> Option<&'static str> {
    Some(match code {
        1 => "F",
        2 => "Fl",
        3 => "LFl",
        4 => "Q",
        5 => "VQ",
        6 => "UQ",
        7 => "Iso",
        8 => "Oc",
        9 => "IQ",
        10 => "IVQ",
        11 => "IUQ",
        12 => "Mo",
        13 => "FFl",
        14 => "Fl+LFl",
        15 => "OcFl",
        16 => "FLFl",
        17 => "AlOc",
        18 => "AlLFl",
        19 => "AlFl",
        20 => "Al",
        25 => "Q+LFl",
        26 => "VQ+LFl",
        27 => "UQ+LFl",
        28 => "Al",
        29 => "AlF Fl",
        _ => return None,
    })
}

/// The light as printed on a chart: `Fl(2)WR 6s 12M`.
pub fn light_description(l: &Light) -> String {
    let mut s = String::new();
    if let Some(a) = l.character.and_then(character_abbrev) {
        s.push_str(a);
    }
    if let Some(g) = &l.group {
        // "(1)" on a plain flash is noise on a chart.
        if g != "(1)" {
            s.push_str(g);
        }
    }
    let letters: String = l.colours.iter().filter_map(|c| colour_letter(*c)).collect();
    s.push_str(&letters);
    if let Some(p) = l.period_s.filter(|p| *p > 0.0) {
        s.push(' ');
        s.push_str(&format!("{}s", trim_float(p)));
    }
    if let Some(r) = l.range_nm.filter(|r| *r > 0.0) {
        s.push(' ');
        s.push_str(&format!("{}M", trim_float(r)));
    }
    s.trim().to_string()
}

fn trim_float(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 { format!("{}", v.round() as i64) } else { format!("{v}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point_blob(lon: f64, lat: f64) -> Vec<u8> {
        let mut b = vec![b'G', b'P', 0, 1, 0xe6, 0x10, 0, 0, 1, 1, 0, 0, 0];
        b.extend_from_slice(&lon.to_le_bytes());
        b.extend_from_slice(&lat.to_le_bytes());
        b
    }

    #[test]
    fn parses_a_geopackage_point() {
        assert_eq!(parse_gpkg_point(&point_blob(-4.5, 48.3)), Some([-4.5, 48.3]));
    }

    #[test]
    fn parses_a_point_with_an_envelope() {
        let mut b = vec![b'G', b'P', 0, 0b0000_0011, 0xe6, 0x10, 0, 0];
        b.extend_from_slice(&[0u8; 32]);
        b.extend_from_slice(&[1, 1, 0, 0, 0]);
        b.extend_from_slice(&1.5f64.to_le_bytes());
        b.extend_from_slice(&2.5f64.to_le_bytes());
        assert_eq!(parse_gpkg_point(&b), Some([1.5, 2.5]));
    }

    #[test]
    fn rejects_junk_and_other_geometries() {
        assert_eq!(parse_gpkg_point(b""), None);
        assert_eq!(parse_gpkg_point(b"XX\0\0\0\0\0\0\0"), None);
        let mut b = point_blob(1.0, 2.0);
        b[9] = 2; // linestring
        assert_eq!(parse_gpkg_point(&b), None);
        let short = &point_blob(1.0, 2.0)[..20];
        assert_eq!(parse_gpkg_point(short), None);
    }

    #[test]
    fn colours_parse() {
        assert_eq!(parse_colours("6,2,6"), vec![6, 2, 6]);
        assert_eq!(parse_colours(" 4 "), vec![4]);
        assert!(parse_colours("").is_empty());
    }

    fn light(character: i32, group: Option<&str>, colours: &[u8], period: f64, range: f64) -> Light {
        Light {
            pos: LatLon::new(0.0, 0.0),
            colours: colours.to_vec(),
            character: Some(character),
            group: group.map(String::from),
            period_s: Some(period),
            range_nm: Some(range),
            sector: None,
            height_m: None,
        }
    }

    #[test]
    fn light_descriptions_read_like_a_chart() {
        assert_eq!(light_description(&light(2, Some("(2)"), &[1, 3], 6.0, 12.0)), "Fl(2)WR 6s 12M");
        assert_eq!(light_description(&light(2, Some("(1)"), &[4], 5.0, 7.5)), "FlG 5s 7.5M");
        assert_eq!(light_description(&light(8, None, &[1], 4.0, 0.0)), "OcW 4s");
        assert_eq!(light_description(&light(4, None, &[], 0.0, 0.0)), "Q");
    }

    #[test]
    fn real_balisage_geopackage_loads_when_present() {
        let p = Path::new("testdata/shom-balisage/BALISAGE/GPKG/balisage.gpkg");
        if !p.exists() {
            return;
        }
        let d = AidsData::load(p).unwrap();
        assert!(d.aids.len() > 10_000 && d.lights.len() > 6_000, "{} {}", d.aids.len(), d.lights.len());
        // Brest roadstead has plenty of lights and buoys.
        let brest = Bounds { min_lon: -4.6, min_lat: 48.3, max_lon: -4.4, max_lat: 48.45 };
        assert!(d.lights_in(&brest).count() > 5);
        assert!(d.aids_in(&brest).filter(|a| a.kind.is_buoy()).count() > 5);
        // Lateral buoys carry a category and colours.
        assert!(d.aids.iter().any(|a| a.kind == AidKind::BuoyLateral && a.category.is_some() && !a.colours.is_empty()));
    }
}
