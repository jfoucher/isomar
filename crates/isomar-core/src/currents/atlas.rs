//! Shom tidal-stream atlases ("Atlas de courants de marée 2D", Licence Ouverte 2.0, credit "Shom", DOI
//! 10.17183/ATLASCOURANTS2D_NETCDF), converted by `scripts/gen-tidal-atlas.py` from Shom's NetCDF into
//! `.isoatlas` files and hosted on the website, a zone at a time.
//!
//! An atlas holds the tidal stream on a regular grid at mean neaps (coefficient 45) and mean springs (95),
//! every half hour (or quarter) from six hours before to six hours after high water at its reference port
//! (low water for some). It is used as the paper atlases are (`coefficient`): the hour from the port's
//! high water nearest the moment, and the day's coefficient from Brest, interpolated between the steps
//! and between neaps and springs. Shom: "A n'utiliser qu'en complément des cartes et ouvrages nautiques
//! officiels."
//!
//! The file: `ISOATLAS`, a little-endian `u32` version and header length, the JSON header, then the
//! deflated `i16` values in cm/s (−32768 where there is no water): U then V, each by coefficient, step,
//! row (north first) and column.

use super::coefficient;
use super::field::Tidal;
use crate::geo::LatLon;
use crate::tides::Extreme;
use crate::vector::Bounds;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

pub const MAGIC: &[u8; 8] = b"ISOATLAS";
pub const VERSION: u32 = 1;
const MISSING: i16 = i16::MIN;
const CM_S_TO_KN: f64 = 0.01 / 0.514_444;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Header {
    /// The grid's name, as the zone's list shows it ("Iroise 250 m").
    pub name: String,
    /// The zone it belongs to ("finistere").
    pub zone: String,
    /// The reference port, where it is, and whether the hours count from its low water.
    pub port: String,
    pub port_lat: f64,
    pub port_lon: f64,
    #[serde(default)]
    pub low_water: bool,
    /// The coefficients of the two tides drawn (45, 95).
    pub coeffs: Vec<f64>,
    /// The steps, minutes from the reference port's high (or low) water.
    pub minutes: Vec<f64>,
    pub ncols: usize,
    pub nrows: usize,
    /// The lower-left corner of the lower-left cell, and the cell size, degrees.
    pub xll: f64,
    pub yll: f64,
    pub dx: f64,
    pub dy: f64,
    /// Shom's release date of the source.
    #[serde(default)]
    pub release: String,
}

pub struct Atlas {
    pub header: Header,
    u: Vec<i16>,
    v: Vec<i16>,
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]))
}

impl Atlas {
    /// Reads an `.isoatlas` file's bytes.
    pub fn read(bytes: &[u8]) -> Result<Atlas, String> {
        if bytes.get(..8) != Some(MAGIC.as_slice()) {
            return Err("not an atlas".into());
        }
        if u32_at(bytes, 8) != Some(VERSION) {
            return Err("an atlas of another version".into());
        }
        let len = u32_at(bytes, 12).ok_or("truncated")? as usize;
        let json = bytes.get(16..16 + len).ok_or("truncated header")?;
        let header: Header = serde_json::from_slice(json).map_err(|e| e.to_string())?;
        let n = header.coeffs.len() * header.minutes.len() * header.nrows * header.ncols;
        if header.coeffs.len() != 2 || header.minutes.len() < 2 || n == 0 || header.dx <= 0.0 || header.dy <= 0.0 {
            return Err("an atlas with nothing in it".into());
        }
        let mut raw = Vec::with_capacity(4 * n);
        flate2::read::ZlibDecoder::new(&bytes[16 + len..]).take(4 * n as u64 + 1).read_to_end(&mut raw).map_err(|e| e.to_string())?;
        if raw.len() != 4 * n {
            return Err(format!("{} bytes of values, not {}", raw.len(), 4 * n));
        }
        let vals: Vec<i16> = raw.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
        let (u, v) = vals.split_at(n);
        Ok(Atlas { header, u: u.to_vec(), v: v.to_vec() })
    }

    /// Writes one (for the tests; the generator writes the real ones).
    pub fn write(header: &Header, u: &[i16], v: &[i16]) -> Vec<u8> {
        use std::io::Write;
        let json = serde_json::to_vec(header).unwrap_or_default();
        let mut out = MAGIC.to_vec();
        out.extend(VERSION.to_le_bytes());
        out.extend((json.len() as u32).to_le_bytes());
        out.extend(&json);
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        for x in u.iter().chain(v) {
            let _ = z.write_all(&x.to_le_bytes());
        }
        out.extend(z.finish().unwrap_or_default());
        out
    }

    pub fn open(path: &Path) -> Result<Atlas, String> {
        Atlas::read(&std::fs::read(path).map_err(|e| e.to_string())?)
    }

    pub fn bounds(&self) -> Bounds {
        let h = &self.header;
        Bounds { min_lon: h.xll, min_lat: h.yll, max_lon: h.xll + h.ncols as f64 * h.dx, max_lat: h.yll + h.nrows as f64 * h.dy }
    }

    fn at_cell(&self, col: usize, row: usize, step: (usize, usize, f64), w: f64) -> Option<(f64, f64)> {
        let h = &self.header;
        let nt = h.minutes.len();
        let i = |c: usize, k: usize| ((c * nt + k) * h.nrows + row) * h.ncols + col;
        let one = |a: &[i16]| -> Option<f64> {
            let (k0, k1, f) = step;
            let mut by_coeff = [0.0; 2];
            for (c, slot) in by_coeff.iter_mut().enumerate() {
                let (x, y) = (a[i(c, k0)], a[i(c, k1)]);
                if x == MISSING || y == MISSING {
                    return None;
                }
                *slot = x as f64 + (y as f64 - x as f64) * f;
            }
            Some(by_coeff[0] + (by_coeff[1] - by_coeff[0]) * w)
        };
        Some((one(&self.u)? * CM_S_TO_KN, one(&self.v)? * CM_S_TO_KN))
    }

    /// The stream at `p`, `phase_h` hours from the reference port's high (or low) water, with `w` the
    /// weight of springs against neaps (`coefficient::springs_weight`): (east, north) knots, bilinear
    /// over the cells that have water.
    pub fn at(&self, p: &LatLon, phase_h: f64, w: f64) -> Option<(f64, f64)> {
        let h = &self.header;
        let m = (phase_h * 60.0).clamp(h.minutes[0], h.minutes[h.minutes.len() - 1]);
        let k = h.minutes.partition_point(|x| *x <= m).clamp(1, h.minutes.len() - 1);
        let span = h.minutes[k] - h.minutes[k - 1];
        let step = (k - 1, k, if span > 0.0 { (m - h.minutes[k - 1]) / span } else { 0.0 });
        let fc = (p.lon - h.xll) / h.dx - 0.5;
        let fr = h.nrows as f64 - (p.lat - h.yll) / h.dy - 0.5;
        if fc < -0.5 || fr < -0.5 || fc > h.ncols as f64 - 0.5 || fr > h.nrows as f64 - 0.5 {
            return None;
        }
        let (fc, fr) = (fc.clamp(0.0, (h.ncols - 1) as f64), fr.clamp(0.0, (h.nrows - 1) as f64));
        let (c0, r0) = (fc.floor() as usize, fr.floor() as usize);
        let (c1, r1) = ((c0 + 1).min(h.ncols - 1), (r0 + 1).min(h.nrows - 1));
        let (tx, ty) = (fc - c0 as f64, fr - r0 as f64);
        let (mut e, mut n, mut wsum) = (0.0, 0.0, 0.0);
        for (c, r, wt) in [(c0, r0, (1.0 - tx) * (1.0 - ty)), (c1, r0, tx * (1.0 - ty)), (c0, r1, (1.0 - tx) * ty), (c1, r1, tx * ty)] {
            if wt <= 0.0 {
                continue;
            }
            if let Some((a, b)) = self.at_cell(c, r, step, w) {
                e += a * wt;
                n += b * wt;
                wsum += wt;
            }
        }
        (wsum > 1e-9).then(|| (e / wsum, n / wsum))
    }
}

/// An atlas with what it takes to use it at any moment: the tide at its reference port and at Brest.
pub struct Phased {
    pub atlas: Arc<Atlas>,
    pub port: Arc<Vec<Extreme>>,
    pub brest: Arc<Vec<Extreme>>,
}

impl Tidal for Phased {
    fn bounds(&self) -> Bounds {
        self.atlas.bounds()
    }

    fn step_deg(&self) -> f64 {
        self.atlas.header.dy
    }

    fn tide(&self, p: &LatLon, t: DateTime<Utc>) -> Option<(f64, f64)> {
        let phase = coefficient::phase_hours(&self.port, self.atlas.header.low_water, t)?;
        let coeff = coefficient::coefficient(&self.brest, t)?;
        self.atlas.at(p, phase, coefficient::springs_weight(coeff))
    }

    fn name(&self) -> String {
        self.atlas.header.name.clone()
    }
}

/// Every atlas in `dir` and the folders under it.
pub fn load_dir(dir: &Path) -> Vec<Arc<Atlas>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(load_dir(&p));
        } else if p.extension().is_some_and(|x| x == "isoatlas") {
            match Atlas::open(&p) {
                Ok(a) => out.push(Arc::new(a)),
                Err(err) => crate::log!("atlas: {} cannot be read: {err}", p.display()),
            }
        }
    }
    out
}

/// The atlases phased with the tides of their reference ports and Brest, from `tides_dir` or the
/// network (`tides::lookup`, which keeps a month). An atlas whose port's tide cannot be had is left out.
/// Blocking.
pub fn phase(atlases: &[Arc<Atlas>], tides_dir: &Path, from: DateTime<Utc>, hours: i64) -> Vec<Phased> {
    let brest = match crate::tides::lookup(tides_dir, coefficient::BREST.0, coefficient::BREST.1, from, hours) {
        Ok((f, _)) => Arc::new(f.extremes),
        Err(e) => {
            crate::log!("atlas: no tide at Brest for the coefficients: {e}");
            return Vec::new();
        }
    };
    let mut ports: std::collections::BTreeMap<String, Option<Arc<Vec<Extreme>>>> = Default::default();
    let mut out = Vec::new();
    for a in atlases {
        let h = &a.header;
        let port = ports
            .entry(h.port.clone())
            .or_insert_with(|| match crate::tides::lookup(tides_dir, h.port_lat, h.port_lon, from, hours) {
                Ok((f, _)) => Some(Arc::new(f.extremes)),
                Err(e) => {
                    crate::log!("atlas: no tide at {} for {}: {e}", h.port, h.name);
                    None
                }
            })
            .clone();
        if let Some(port) = port {
            out.push(Phased { atlas: a.clone(), port, brest: brest.clone() });
        }
    }
    out
}

/// One zone of the hosted atlases (`atlases/index.json` on the website).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Zone {
    pub id: String,
    pub name: String,
    /// West, south, east, north.
    pub bounds: [f64; 4],
    pub files: Vec<ZoneFile>,
    pub bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ZoneFile {
    /// Relative to the index ("finistere/iroise-250m.isoatlas").
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Index {
    pub version: u32,
    pub zones: Vec<Zone>,
}

impl Zone {
    pub fn area(&self) -> Bounds {
        Bounds { min_lon: self.bounds[0], min_lat: self.bounds[1], max_lon: self.bounds[2], max_lat: self.bounds[3] }
    }
}

/// Where the atlases are hosted; `ISOMAR_ATLAS_URL` points elsewhere (a local folder served for a test).
pub fn base_url() -> String {
    std::env::var("ISOMAR_ATLAS_URL").unwrap_or_else(|_| "https://isomar.marinetechs.com/atlases".into())
}

/// Where installed zones live: reference data, not a cache.
pub fn install_dir() -> std::path::PathBuf {
    crate::datadir::user_data_dir().join("atlases")
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(std::time::Duration::from_secs(10)))
        .timeout_global(Some(std::time::Duration::from_secs(600)))
        .user_agent("isomar/0.1 (open-source chart plotter)")
        .build()
        .into()
}

fn fetch(url: &str) -> Result<Vec<u8>, String> {
    if let Some(path) = url.strip_prefix("file://") {
        return std::fs::read(path).map_err(|e| e.to_string());
    }
    let mut resp = agent().get(url).call().map_err(|e| e.to_string())?;
    resp.body_mut().with_config().limit(512 * 1024 * 1024).read_to_vec().map_err(|e| e.to_string())
}

/// The zones the website offers. Blocking.
pub fn index() -> Result<Index, String> {
    serde_json::from_slice(&fetch(&format!("{}/index.json", base_url()))?).map_err(|e| e.to_string())
}

fn sha256_hex(b: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

/// Downloads a zone's files into `dir/<zone id>/`, each checked against its hash before it is kept.
/// Blocking.
pub fn install(zone: &Zone, dir: &Path, cancel: &std::sync::atomic::AtomicBool) -> Result<(), String> {
    install_from(&base_url(), zone, dir, cancel)
}

/// `install` from the atlases at `base` (a URL, or `file://` and a folder).
pub fn install_from(base: &str, zone: &Zone, dir: &Path, cancel: &std::sync::atomic::AtomicBool) -> Result<(), String> {
    if zone.id.is_empty() || zone.id.contains(['/', '\\', '.']) {
        return Err("a zone without a proper name".into());
    }
    let target = dir.join(&zone.id);
    let part = dir.join(format!("{}.part", zone.id));
    let _ = std::fs::remove_dir_all(&part);
    std::fs::create_dir_all(&part).map_err(|e| e.to_string())?;
    for f in &zone.files {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            let _ = std::fs::remove_dir_all(&part);
            return Err("cancelled".into());
        }
        let bytes = fetch(&format!("{base}/{}", f.name))?;
        if sha256_hex(&bytes) != f.sha256 {
            let _ = std::fs::remove_dir_all(&part);
            return Err(format!("{} is not the file the index describes", f.name));
        }
        Atlas::read(&bytes)?;
        let name = Path::new(&f.name).file_name().ok_or("a file without a name")?;
        std::fs::write(part.join(name), &bytes).map_err(|e| e.to_string())?;
    }
    let _ = std::fs::remove_dir_all(&target);
    std::fs::rename(&part, &target).map_err(|e| e.to_string())
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    /// A 3 by 3 atlas over (0..3 E, 0..3 N): a stream setting east, neaps `n` cm/s and springs `s` cm/s,
    /// growing by 10 cm/s a step from HW−1 h to HW+1 h; the north-west cell is land.
    pub fn synthetic(n: i16, s: i16) -> Atlas {
        let h = Header {
            name: "Test 1°".into(),
            zone: "test".into(),
            port: "TEST".into(),
            port_lat: 1.0,
            port_lon: 1.0,
            low_water: false,
            coeffs: vec![45.0, 95.0],
            minutes: vec![-60.0, 0.0, 60.0],
            ncols: 3,
            nrows: 3,
            xll: 0.0,
            yll: 0.0,
            dx: 1.0,
            dy: 1.0,
            release: String::new(),
        };
        let mut u = Vec::new();
        for base in [n, s] {
            for k in 0..3i16 {
                for cell in 0..9 {
                    u.push(if cell == 0 { MISSING } else { base + 10 * k });
                }
            }
        }
        let v: Vec<i16> = u.iter().map(|x| if *x == MISSING { MISSING } else { 0 }).collect();
        Atlas::read(&Atlas::write(&h, &u, &v)).unwrap()
    }

    #[test]
    fn steps_and_coefficients_interpolate_as_the_paper_atlas_is_read() {
        let a = synthetic(100, 200);
        let kn = |cm: f64| cm * CM_S_TO_KN;
        let e = |phase: f64, w: f64| a.at(&LatLon::new(1.5, 1.5), phase, w).unwrap().0;
        assert!((e(0.0, 0.0) - kn(110.0)).abs() < 1e-9, "at high water, neaps");
        assert!((e(0.0, 1.0) - kn(210.0)).abs() < 1e-9, "springs");
        assert!((e(0.0, 0.5) - kn(160.0)).abs() < 1e-9, "coefficient 70, halfway");
        assert!((e(0.5, 0.0) - kn(115.0)).abs() < 1e-9, "halfway between steps");
        assert!((e(3.0, 0.0) - kn(120.0)).abs() < 1e-9, "past the last step, held there");
        assert!((e(0.0, 1.5) - kn(260.0)).abs() < 1e-9, "a big tide, stretched past springs");
        // Next to the land cell, the water cells alone.
        assert!((a.at(&LatLon::new(2.4, 0.6), 0.0, 0.0).unwrap().0 - kn(110.0)).abs() < 1e-9);
        assert!(a.at(&LatLon::new(2.9, 0.1), 0.0, 0.0).is_none(), "on land");
        assert!(a.at(&LatLon::new(5.0, 5.0), 0.0, 0.0).is_none(), "outside");
        assert!(Atlas::read(b"ISOATLAS\x02\0\0\0").is_err());
    }

    #[test]
    fn phased_by_its_port_and_brest() {
        let t0 = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        let tide = |range: f64| -> Vec<Extreme> { (0..8).map(|k| Extreme { time: t0 + Duration::minutes(k * 745 / 2), level_m: if k % 2 == 0 { 4.0 + range / 2.0 } else { 4.0 - range / 2.0 }, high: k % 2 == 0 }).collect() };
        let p = Phased { atlas: Arc::new(synthetic(100, 200)), port: Arc::new(tide(5.0)), brest: Arc::new(tide(2.745)) };
        let hw = t0 + Duration::minutes(745);
        let (e, _) = p.tide(&LatLon::new(1.5, 1.5), hw).unwrap();
        assert!((e - 110.0 * CM_S_TO_KN).abs() < 1e-6, "neaps at Brest, high water at the port: {e}");
        let (later, _) = p.tide(&LatLon::new(1.5, 1.5), hw + Duration::minutes(31)).unwrap();
        assert!(later > e, "the stream grows after high water");
        assert_eq!(p.step_deg(), 1.0);
    }

    #[test]
    fn a_zone_installs_only_what_its_index_vouches_for() {
        let site = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let bytes = Atlas::write(&synthetic(100, 200).header, &[0; 54], &[0; 54]);
        std::fs::create_dir_all(site.path().join("test")).unwrap();
        std::fs::write(site.path().join("test/a.isoatlas"), &bytes).unwrap();
        let base = format!("file://{}", site.path().display());
        let mut zone = Zone { id: "test".into(), name: "Test".into(), bounds: [0.0, 0.0, 3.0, 3.0], files: vec![ZoneFile { name: "test/a.isoatlas".into(), bytes: bytes.len() as u64, sha256: sha256_hex(&bytes) }], bytes: bytes.len() as u64 };
        let no = std::sync::atomic::AtomicBool::new(false);
        install_from(&base, &zone, home.path(), &no).unwrap();
        assert_eq!(load_dir(home.path()).len(), 1);
        zone.files[0].sha256 = "00".into();
        assert!(install_from(&base, &zone, home.path(), &no).is_err(), "a file that is not the one described");
        assert_eq!(load_dir(home.path()).len(), 1, "the installed copy is left alone");
        zone.id = "../evil".into();
        assert!(install_from(&base, &zone, home.path(), &no).is_err());
    }

    #[test]
    fn a_zone_index_reads() {
        let i: Index = serde_json::from_str(r#"{"version":1,"zones":[{"id":"finistere","name":"Finistère","bounds":[-5.5,47.5,-3.9,48.8],"bytes":42,"files":[{"name":"finistere/iroise-250m.isoatlas","bytes":42,"sha256":"ab"}]}]}"#).unwrap();
        assert!(i.zones[0].area().contains([-4.8, 48.0]));
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    /// The converted Iroise grid (`scripts/gen-tidal-atlas.py` into `website/atlases`): the Raz de Sein and
    /// the Fromveur at mean springs, against the rates the charts give (about 6.5 and 8 kn), and the
    /// atlas phased by today's tide at Brest.
    /// `cargo test --release -p isomar-core live_iroise -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_iroise_atlas() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../website/atlases/finistere/iroise-250m.isoatlas");
        let Ok(a) = Atlas::open(&path) else {
            eprintln!("no {}: run scripts/gen-tidal-atlas.py first", path.display());
            return;
        };
        let peak = |lat0: f64, lat1: f64, lon0: f64, lon1: f64| -> (f64, f64) {
            let mut best = (0.0f64, 0.0);
            let mut lat = lat0;
            while lat <= lat1 {
                let mut lon = lon0;
                while lon <= lon1 {
                    for m in (-360..=360).step_by(15) {
                        if let Some((e, n)) = a.at(&LatLon::new(lat, lon), m as f64 / 60.0, 1.0)
                            && e.hypot(n) > best.0
                        {
                            best = (e.hypot(n), m as f64 / 60.0);
                        }
                    }
                    lon += 0.003;
                }
                lat += 0.003;
            }
            best
        };
        let raz = peak(48.02, 48.06, -4.80, -4.70);
        let fromveur = peak(48.43, 48.47, -5.05, -4.97);
        eprintln!("Raz de Sein at springs: {:.1} kn at HW{:+.1}; Fromveur {:.1} kn at HW{:+.1}", raz.0, raz.1, fromveur.0, fromveur.1);
        assert!(raz.0 > 5.0 && raz.0 < 9.0, "{raz:?}");
        assert!(fromveur.0 > 6.0 && fromveur.0 < 11.0, "{fromveur:?}");
        let dir = tempfile::tempdir().unwrap();
        let now = Utc::now();
        let phased = phase(&[Arc::new(a)], dir.path(), now, 48);
        assert_eq!(phased.len(), 1, "Brest's tide came");
        for h in (0..13).step_by(2) {
            let t = now + chrono::Duration::hours(h);
            let c = phased[0].tide(&LatLon::new(48.04, -4.75), t).map(|(e, n)| (e.hypot(n), crate::geo::set_of(e, n)));
            eprintln!("  +{h:2} h: {c:?}");
        }
    }
}
