//! IHO S-57 electronic navigational charts: a base cell (`.000`) plus its
//! numbered updates (`.001`, `.002`, ...), assembled into features with
//! geometry in lon/lat.

use super::iso8211::{Ddf, IsoError, Record, Value, read_file};
use crate::vector::{Bounds, Pt};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// Object class codes used by the renderer (from the S-57 object catalogue).
pub mod obj {
    /// Anchor berth.
    pub const ACHBRT: u16 = 3;
    pub const ACHARE: u16 = 4;
    pub const BCNCAR: u16 = 5;
    pub const BCNISD: u16 = 6;
    pub const BCNLAT: u16 = 7;
    pub const BCNSAW: u16 = 8;
    pub const BCNSPP: u16 = 9;
    pub const BUAARE: u16 = 13;
    pub const BOYCAR: u16 = 14;
    pub const BOYINB: u16 = 15;
    pub const BOYISD: u16 = 16;
    pub const BOYLAT: u16 = 17;
    pub const BOYSAW: u16 = 18;
    pub const BOYSPP: u16 = 19;
    pub const COALNE: u16 = 30;
    pub const DEPARE: u16 = 42;
    pub const DEPCNT: u16 = 43;
    pub const DRGARE: u16 = 46;
    pub const FAIRWY: u16 = 51;
    pub const LIGHTS: u16 = 75;
    pub const LNDARE: u16 = 71;
    /// Land elevation: contour lines and spot heights (`ELEVAT`).
    pub const LNDELV: u16 = 72;
    pub const LNDMRK: u16 = 74;
    pub const LNDRGN: u16 = 73;
    pub const OBSTRN: u16 = 86;
    pub const RESARE: u16 = 112;
    pub const SBDARE: u16 = 121;
    pub const SEAARE: u16 = 119;
    pub const SLCONS: u16 = 122;
    /// Slope top line.
    pub const SLOTOP: u16 = 126;
    pub const SOUNDG: u16 = 129;
    pub const UWTROC: u16 = 153;
    pub const WRECKS: u16 = 159;
    pub const M_COVR: u16 = 302;
    pub const BRIDGE: u16 = 11;
    pub const CBLARE: u16 = 20;
    pub const CBLOHD: u16 = 21;
    pub const CBLSUB: u16 = 22;
    pub const CTNARE: u16 = 27;
    pub const DMPGRD: u16 = 48;
    pub const FSHZNE: u16 = 54;
    pub const ISTZNE: u16 = 68;
    pub const MAGVAR: u16 = 81;
    pub const MARCUL: u16 = 82;
    pub const MIPARE: u16 = 83;
    pub const NAVLNE: u16 = 85;
    pub const OSPARE: u16 = 88;
    pub const PIPARE: u16 = 92;
    pub const PIPSOL: u16 = 94;
    pub const RECTRC: u16 = 109;
    pub const SPLARE: u16 = 120;
    pub const TESARE: u16 = 135;
    pub const TS_PAD: u16 = 138;
    /// Two-way route part: S-52 draws it (a dashed edge and an arrow), the chart here does not.
    pub const TWRTPT: u16 = 152;
    pub const TS_FEB: u16 = 160;
    pub const UNSARE: u16 = 154;
    pub const WATTUR: u16 = 156;
    pub const M_QUAL: u16 = 308;
}

/// Attribute codes used by the renderer.
pub mod attr {
    pub const BCNSHP: u16 = 2;
    pub const BOYSHP: u16 = 4;
    /// Category of anchorage (a list).
    pub const CATACH: u16 = 8;
    pub const CATCOV: u16 = 18;
    pub const CATLAM: u16 = 36;
    pub const CATCAM: u16 = 13;
    /// Category of sea area: 5 is a bay.
    pub const CATSEA: u16 = 59;
    pub const COLOUR: u16 = 75;
    pub const COLPAT: u16 = 76;
    pub const DRVAL1: u16 = 87;
    pub const DRVAL2: u16 = 88;
    /// Elevation of a land contour or spot height, in metres.
    pub const ELEVAT: u16 = 90;
    pub const HEIGHT: u16 = 95;
    pub const LITCHR: u16 = 107;
    /// Nature of the seabed (a list: 1 mud, 4 sand, 9 rock…).
    pub const NATSUR: u16 = 113;
    pub const OBJNAM: u16 = 116;
    pub const ORIENT: u16 = 117;
    /// A berth's swinging radius, in metres.
    pub const RADIUS: u16 = 127;
    /// Restriction (a list: 1 anchoring prohibited, 2 anchoring restricted…).
    pub const RESTRN: u16 = 131;
    pub const SCAMIN: u16 = 133;
    pub const SECTR1: u16 = 136;
    pub const SECTR2: u16 = 137;
    pub const SIGGRP: u16 = 141;
    pub const SIGPER: u16 = 142;
    /// Value of a depth contour, in metres.
    pub const VALDCO: u16 = 174;
    pub const VALNMR: u16 = 178;
    pub const VALSOU: u16 = 179;
    pub const WATLEV: u16 = 187;
    /// Category of shoreline construction.
    pub const CATSLC: u16 = 60;
    /// Category of wreck: 2 is a dangerous wreck.
    pub const CATWRK: u16 = 71;
    /// Category of obstruction: 6 foul area, 7 foul ground.
    pub const CATOBS: u16 = 42;
    pub const CATNAV: u16 = 41;
    /// Category of recommended track: 1 based on a system of fixed marks.
    pub const CATTRK: u16 = 54;
    /// Category of water turbulence: 1 breakers, 2 eddies, 3 overfalls, 4 tide rips.
    pub const CATWAT: u16 = 69;
    /// Category of a restricted area (a list).
    pub const CATREA: u16 = 56;
    /// Category of zone of confidence: 1 is A1, 5 is D, 6 not assessed.
    pub const CATZOC: u16 = 72;
    /// Category of tidal stream: 1 flood, 2 ebb.
    pub const CAT_TS: u16 = 188;
    pub const CURVEL: u16 = 84;
    pub const EXPSOU: u16 = 93;
    pub const INFORM: u16 = 102;
    pub const NINFOM: u16 = 300;
    pub const QUASOU: u16 = 125;
    pub const RYRMGV: u16 = 130;
    pub const STATUS: u16 = 149;
    pub const TECSOU: u16 = 156;
    pub const TRAFIC: u16 = 172;
    pub const VALACM: u16 = 173;
    pub const VALMAG: u16 = 176;
    pub const VERCLR: u16 = 181;
}

#[derive(Debug)]
pub enum S57Error {
    Io(std::io::Error),
    Iso(IsoError),
    /// The file has no dataset parameter record.
    NotAnEnc,
}

impl From<std::io::Error> for S57Error {
    fn from(e: std::io::Error) -> Self {
        S57Error::Io(e)
    }
}

impl From<IsoError> for S57Error {
    fn from(e: IsoError) -> Self {
        S57Error::Iso(e)
    }
}

impl std::fmt::Display for S57Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            S57Error::Io(e) => write!(f, "{e}"),
            S57Error::Iso(e) => write!(f, "{e}"),
            S57Error::NotAnEnc => write!(f, "this is not an S-57 cell"),
        }
    }
}

impl std::error::Error for S57Error {}

#[derive(Clone, Debug, PartialEq)]
pub enum Geom {
    None,
    Point(Pt),
    Line(Vec<Pt>),
    /// Rings, exterior first; fill with the even-odd rule.
    Area(Vec<Vec<Pt>>),
    /// `[lon, lat, depth in metres]`.
    Soundings(Vec<[f64; 3]>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Feature {
    pub rcid: u32,
    pub objl: u16,
    pub attrs: BTreeMap<u16, String>,
    pub geom: Geom,
}

impl Feature {
    pub fn attr(&self, code: u16) -> Option<&str> {
        self.attrs.get(&code).map(|s| s.as_str()).filter(|s| !s.is_empty())
    }

    pub fn num(&self, code: u16) -> Option<f64> {
        self.attr(code).and_then(|s| s.trim().parse().ok())
    }

    /// An attribute holding a list, such as COLOUR `3,1`.
    pub fn list(&self, code: u16) -> Vec<u8> {
        self.attr(code).map(|s| s.split(',').filter_map(|p| p.trim().parse().ok()).collect()).unwrap_or_default()
    }
}

#[derive(Clone, Debug)]
pub struct Cell {
    pub name: String,
    pub edition: u32,
    pub update: u32,
    pub issue_date: String,
    /// Compilation scale denominator.
    pub cscl: u32,
    pub features: Vec<Feature>,
    pub bounds: Bounds,
    pub warnings: Vec<String>,
    /// Update files that were not applied because the cell already includes them.
    pub skipped_updates: Vec<u32>,
}

impl Cell {
    pub fn count(&self, objl: u16) -> usize {
        self.features.iter().filter(|f| f.objl == objl).count()
    }

    /// Where the cell has data, from its coverage objects (`M_COVR`).
    pub fn coverage(&self) -> Coverage {
        let mut c = Coverage::default();
        for f in self.features.iter().filter(|f| f.objl == obj::M_COVR) {
            let Geom::Area(rings) = &f.geom else { continue };
            // CATCOV 1 is "coverage available", 2 "no coverage"; a missing value means coverage.
            let into = if f.num(attr::CATCOV) == Some(2.0) { &mut c.holes } else { &mut c.covered };
            into.extend(rings.iter().cloned());
        }
        c
    }

    /// What the cell is, without its features: enough to place it on the
    /// chart and list it, so the cell itself can be read only when needed.
    pub fn summary(&self) -> CellSummary {
        CellSummary {
            name: self.name.clone(),
            edition: self.edition,
            update: self.update,
            issue_date: self.issue_date.clone(),
            cscl: self.cscl,
            bounds: self.bounds,
            features: self.features.len(),
            warnings: self.warnings.len(),
            cover: self.coverage(),
        }
    }

    pub fn usage_band(&self) -> u8 {
        usage_band(&self.name, self.cscl)
    }

    pub fn usage_band_name(&self) -> &'static str {
        usage_band_name(self.usage_band())
    }
}

/// A cell with its features left out; see [`Cell::summary`].
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CellSummary {
    pub name: String,
    pub edition: u32,
    pub update: u32,
    pub issue_date: String,
    pub cscl: u32,
    pub bounds: Bounds,
    /// How many features the cell holds.
    pub features: usize,
    /// How many warnings reading it raised.
    pub warnings: usize,
    pub cover: Coverage,
}

impl CellSummary {
    pub fn usage_band(&self) -> u8 {
        usage_band(&self.name, self.cscl)
    }

    pub fn usage_band_name(&self) -> &'static str {
        usage_band_name(self.usage_band())
    }
}

/// Navigational purpose (usage band 1 to 6) as indicated by the 3rd character
/// of the standard 8-character S-57 cell name (e.g. 'FR5...', 'US4...'), or inferred
/// from compilation scale `cscl`.
pub fn usage_band(name: &str, cscl: u32) -> u8 {
    let stem = name.split('.').next().unwrap_or(name);
    if stem.len() >= 3 {
        let b = stem.as_bytes()[2];
        if (b'1'..=b'6').contains(&b) {
            return b - b'0';
        }
    }
    match cscl {
        cs if cs > 1_499_999 => 1,
        cs if cs >= 350_000 => 2,
        cs if cs >= 90_000 => 3,
        cs if cs >= 22_000 => 4,
        cs if cs >= 4_000 => 5,
        cs if cs > 0 => 6,
        _ => 5,
    }
}

pub fn usage_band_name(band: u8) -> &'static str {
    match band {
        1 => "Overview",
        2 => "General",
        3 => "Coastal",
        4 => "Approach",
        5 => "Harbour",
        6 => "Berthing",
        _ => "Chart",
    }
}

/// The area a cell has data for: the polygons of its coverage objects, less
/// the parts it says it has none for. A cell's outline is not its bounding
/// box, and a chart viewer draws a cell only where it is the data.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Coverage {
    /// Rings of the areas with coverage; holes among them follow the even-odd rule.
    pub covered: Vec<Vec<Pt>>,
    /// Rings of the areas inside those that have no data.
    pub holes: Vec<Vec<Pt>>,
}

impl Coverage {
    /// Whether the cell has data at `p`. A cell with no coverage object is
    /// taken to have data everywhere, so callers fall back to its bounds.
    pub fn contains(&self, p: Pt) -> bool {
        let inside = |rings: &[Vec<Pt>]| rings.iter().filter(|r| crate::vector::ring_contains(r, p)).count() % 2 == 1;
        inside(&self.covered) && !inside(&self.holes)
    }
}

type Key = (u8, u32);

#[derive(Clone, Debug, PartialEq)]
struct Vpt {
    key: Key,
    topi: u8,
}

#[derive(Clone, Debug, PartialEq)]
struct Spt {
    key: Key,
    ornt: u8,
}

#[derive(Default, Clone)]
struct Spatial {
    pts: Vec<[i32; 2]>,
    soundings: Vec<[i32; 3]>,
    vrpt: Vec<Vpt>,
}

#[derive(Clone)]
struct Feat {
    prim: u8,
    objl: u16,
    attrs: BTreeMap<u16, String>,
    fspt: Vec<Spt>,
}

/// Applies an S-57 pointer or coordinate update to a list: `ui` 1 inserts
/// `new` before the 1-based index `ix`, 2 deletes `n` entries from it, 3
/// replaces them. Out-of-range indexes are clamped rather than rejected.
pub fn edit_list<T: Clone>(list: &mut Vec<T>, ui: i64, ix: i64, n: i64, new: &[T]) {
    let at = (ix - 1).clamp(0, list.len() as i64) as usize;
    match ui {
        1 => {
            for (k, v) in new.iter().enumerate() {
                list.insert((at + k).min(list.len()), v.clone());
            }
        }
        2 => {
            let end = (at + n.max(0) as usize).min(list.len());
            list.drain(at..end);
        }
        3 => {
            for (k, v) in new.iter().enumerate() {
                if at + k < list.len() {
                    list[at + k] = v.clone();
                } else {
                    list.push(v.clone());
                }
            }
        }
        _ => {}
    }
}

fn sub<'a>(ddf: &Ddf, tag: &str, row: &'a [Value], label: &str) -> Option<&'a Value> {
    let i = ddf.defs.get(tag)?.labels.iter().position(|l| l == label)?;
    row.get(i)
}

fn rows(ddf: &Ddf, rec: &Record, tag: &str) -> Vec<Vec<Value>> {
    rec.get(tag).map(|d| ddf.decode(tag, d)).unwrap_or_default()
}

fn name_key(b: &[u8]) -> Option<Key> {
    (b.len() >= 5).then(|| (b[0], u32::from_le_bytes([b[1], b[2], b[3], b[4]])))
}

#[derive(Default)]
struct Builder {
    comf: f64,
    somf: f64,
    cscl: u32,
    name: String,
    edition: u32,
    update: u32,
    issue: String,
    spatial: HashMap<Key, Spatial>,
    feats: BTreeMap<u32, Feat>,
    warnings: Vec<String>,
    skipped: Vec<u32>,
}

impl Builder {
    fn apply_file(&mut self, bytes: &[u8], base: bool) -> Result<(), S57Error> {
        let (ddf, records) = read_file(bytes)?;
        for rec in &records {
            if rec.has("DSID") {
                if let Some(row) = rows(&ddf, rec, "DSID").first() {
                    let get = |l: &str| sub(&ddf, "DSID", row, l);
                    self.name = get("DSNM").map(|v| v.text()).unwrap_or_default();
                    self.edition = get("EDTN").and_then(|v| v.int()).unwrap_or(0) as u32;
                    self.update = get("UPDN").and_then(|v| v.int()).unwrap_or(0) as u32;
                    self.issue = get("ISDT").map(|v| v.text()).unwrap_or_default();
                }
            } else if rec.has("DSPM") {
                if let Some(row) = rows(&ddf, rec, "DSPM").first()
                    && base
                {
                    let get = |l: &str| sub(&ddf, "DSPM", row, l).and_then(|v| v.int());
                    self.comf = get("COMF").unwrap_or(10_000_000) as f64;
                    self.somf = get("SOMF").unwrap_or(10) as f64;
                    self.cscl = get("CSCL").unwrap_or(0) as u32;
                }
            } else if rec.has("VRID") {
                self.apply_spatial(&ddf, rec);
            } else if rec.has("FRID") {
                self.apply_feature(&ddf, rec);
            }
        }
        if self.comf == 0.0 {
            return Err(S57Error::NotAnEnc);
        }
        Ok(())
    }

    fn apply_spatial(&mut self, ddf: &Ddf, rec: &Record) {
        let Some(row) = rows(ddf, rec, "VRID").into_iter().next() else { return };
        let g = |l: &str| sub(ddf, "VRID", &row, l).and_then(|v| v.int()).unwrap_or(0);
        let key = (g("RCNM") as u8, g("RCID") as u32);
        let ruin = g("RUIN");

        let pts: Vec<[i32; 2]> = rows(ddf, rec, "SG2D")
            .iter()
            .filter_map(|r| Some([sub(ddf, "SG2D", r, "YCOO")?.int()? as i32, sub(ddf, "SG2D", r, "XCOO")?.int()? as i32]))
            .collect();
        let soundings: Vec<[i32; 3]> = rows(ddf, rec, "SG3D")
            .iter()
            .filter_map(|r| Some([sub(ddf, "SG3D", r, "YCOO")?.int()? as i32, sub(ddf, "SG3D", r, "XCOO")?.int()? as i32, sub(ddf, "SG3D", r, "VE3D")?.int()? as i32]))
            .collect();
        let vrpt: Vec<Vpt> = rows(ddf, rec, "VRPT")
            .iter()
            .filter_map(|r| Some(Vpt { key: name_key(sub(ddf, "VRPT", r, "NAME")?.bytes())?, topi: sub(ddf, "VRPT", r, "TOPI")?.int()? as u8 }))
            .collect();

        match ruin {
            2 => {
                self.spatial.remove(&key);
            }
            3 => {
                let Some(s) = self.spatial.get_mut(&key) else {
                    self.warnings.push(format!("update for a missing spatial record {key:?}"));
                    return;
                };
                if let Some(c) = rows(ddf, rec, "VRPC").first() {
                    let n = |l: &str| sub(ddf, "VRPC", c, l).and_then(|v| v.int()).unwrap_or(0);
                    edit_list(&mut s.vrpt, n("VPUI"), n("VPIX"), n("NVPT"), &vrpt);
                }
                if let Some(c) = rows(ddf, rec, "SGCC").first() {
                    let n = |l: &str| sub(ddf, "SGCC", c, l).and_then(|v| v.int()).unwrap_or(0);
                    if !soundings.is_empty() {
                        edit_list(&mut s.soundings, n("CCUI"), n("CCIX"), n("CCNC"), &soundings);
                    } else {
                        edit_list(&mut s.pts, n("CCUI"), n("CCIX"), n("CCNC"), &pts);
                    }
                }
            }
            _ => {
                self.spatial.insert(key, Spatial { pts, soundings, vrpt });
            }
        }
    }

    fn attributes(ddf: &Ddf, rec: &Record) -> Vec<(u16, String)> {
        let mut out = Vec::new();
        for tag in ["ATTF", "NATF"] {
            for r in rows(ddf, rec, tag) {
                if let (Some(c), Some(v)) = (sub(ddf, tag, &r, "ATTL").and_then(|v| v.int()), sub(ddf, tag, &r, "ATVL")) {
                    out.push((c as u16, v.text()));
                }
            }
        }
        out
    }

    fn apply_feature(&mut self, ddf: &Ddf, rec: &Record) {
        let Some(row) = rows(ddf, rec, "FRID").into_iter().next() else { return };
        let g = |l: &str| sub(ddf, "FRID", &row, l).and_then(|v| v.int()).unwrap_or(0);
        let (rcid, ruin) = (g("RCID") as u32, g("RUIN"));
        let fspt: Vec<Spt> = rows(ddf, rec, "FSPT")
            .iter()
            .filter_map(|r| Some(Spt { key: name_key(sub(ddf, "FSPT", r, "NAME")?.bytes())?, ornt: sub(ddf, "FSPT", r, "ORNT")?.int()? as u8 }))
            .collect();
        let attrs = Self::attributes(ddf, rec);
        match ruin {
            2 => {
                self.feats.remove(&rcid);
            }
            3 => {
                let Some(f) = self.feats.get_mut(&rcid) else {
                    self.warnings.push(format!("update for a missing feature {rcid}"));
                    return;
                };
                for (code, v) in attrs {
                    // An empty value cancels the attribute.
                    if v.is_empty() || v == "\u{18}" {
                        f.attrs.remove(&code);
                    } else {
                        f.attrs.insert(code, v);
                    }
                }
                if let Some(c) = rows(ddf, rec, "FSPC").first() {
                    let n = |l: &str| sub(ddf, "FSPC", c, l).and_then(|v| v.int()).unwrap_or(0);
                    edit_list(&mut f.fspt, n("FSUI"), n("FSIX"), n("NSPT"), &fspt);
                }
            }
            _ => {
                self.feats.insert(rcid, Feat { prim: g("PRIM") as u8, objl: g("OBJL") as u16, attrs: attrs.into_iter().collect(), fspt });
            }
        }
    }

    fn pt(&self, p: [i32; 2]) -> Pt {
        [p[1] as f64 / self.comf, p[0] as f64 / self.comf]
    }

    fn node_pt(&self, key: Key) -> Option<Pt> {
        self.spatial.get(&key).and_then(|s| s.pts.first()).map(|p| self.pt(*p))
    }

    /// An edge as (begin node, end node, points from begin to end), turned
    /// round when `reverse`.
    fn edge(&self, key: Key, reverse: bool) -> Option<(Option<Key>, Option<Key>, Vec<Pt>)> {
        let e = self.spatial.get(&key)?;
        let begin = e.vrpt.iter().find(|v| v.topi == 1).map(|v| v.key);
        let end = e.vrpt.iter().find(|v| v.topi == 2).map(|v| v.key);
        let mut pts: Vec<Pt> = Vec::new();
        if let Some(p) = begin.and_then(|k| self.node_pt(k)) {
            pts.push(p);
        }
        pts.extend(e.pts.iter().map(|p| self.pt(*p)));
        if let Some(p) = end.and_then(|k| self.node_pt(k)) {
            pts.push(p);
        }
        Some(if reverse {
            pts.reverse();
            (end, begin, pts)
        } else {
            (begin, end, pts)
        })
    }

    fn geometry(&self, f: &Feat) -> Geom {
        match f.prim {
            1 => {
                let Some(first) = f.fspt.first() else { return Geom::None };
                match self.spatial.get(&first.key) {
                    Some(s) if !s.soundings.is_empty() => Geom::Soundings(
                        s.soundings.iter().map(|p| [p[1] as f64 / self.comf, p[0] as f64 / self.comf, p[2] as f64 / self.somf]).collect(),
                    ),
                    Some(s) => s.pts.first().map(|p| Geom::Point(self.pt(*p))).unwrap_or(Geom::None),
                    None => Geom::None,
                }
            }
            2 => {
                let mut line: Vec<Pt> = Vec::new();
                for sp in &f.fspt {
                    if let Some((_, _, pts)) = self.edge(sp.key, sp.ornt == 2) {
                        for p in pts {
                            if line.last() != Some(&p) {
                                line.push(p);
                            }
                        }
                    }
                }
                if line.len() >= 2 { Geom::Line(line) } else { Geom::None }
            }
            3 => {
                let mut rings: Vec<Vec<Pt>> = Vec::new();
                let mut cur: Vec<Pt> = Vec::new();
                let mut start: Option<Key> = None;
                for sp in &f.fspt {
                    let Some((b, e, pts)) = self.edge(sp.key, sp.ornt == 2) else { continue };
                    if cur.is_empty() {
                        start = b;
                        cur.extend(pts);
                    } else {
                        cur.extend(pts.into_iter().skip(1));
                    }
                    let closed = match (start, e) {
                        (Some(s), Some(e)) => s == e,
                        _ => cur.len() > 2 && cur.first() == cur.last(),
                    };
                    if closed {
                        if cur.first() != cur.last() {
                            let first = cur[0];
                            cur.push(first);
                        }
                        rings.push(std::mem::take(&mut cur));
                        start = None;
                    }
                }
                rings.retain(|r| r.len() >= 4);
                if rings.is_empty() { Geom::None } else { Geom::Area(rings) }
            }
            _ => Geom::None,
        }
    }

    fn finish(self) -> Cell {
        let mut features = Vec::new();
        let mut bounds = Bounds::EMPTY;
        for (rcid, f) in &self.feats {
            let geom = self.geometry(f);
            match &geom {
                Geom::Point(p) => bounds.extend(*p),
                Geom::Line(l) => l.iter().for_each(|p| bounds.extend(*p)),
                Geom::Area(r) => r.iter().flatten().for_each(|p| bounds.extend(*p)),
                Geom::Soundings(s) => s.iter().for_each(|p| bounds.extend([p[0], p[1]])),
                Geom::None => {}
            }
            features.push(Feature { rcid: *rcid, objl: f.objl, attrs: f.attrs.clone(), geom });
        }
        Cell { name: self.name, edition: self.edition, update: self.update, issue_date: self.issue, cscl: self.cscl, features, bounds, warnings: self.warnings, skipped_updates: self.skipped }
    }
}

/// The update number a file declares in its dataset identification record.
fn declared_update(bytes: &[u8]) -> Option<u32> {
    let (ddf, records) = read_file(bytes).ok()?;
    let rec = records.iter().find(|r| r.has("DSID"))?;
    let row = rows(&ddf, rec, "DSID").into_iter().next()?;
    sub(&ddf, "DSID", &row, "UPDN")?.int().map(|v| v as u32)
}

/// Reads a base cell and applies `updates` in the order given. An update is
/// applied only if its number is exactly one more than the cell's current
/// one, as S-57 requires: older ones are already part of the cell and are
/// skipped; a gap stops the sequence with a warning.
pub fn load_cell_with(base: &Path, updates: &[std::path::PathBuf]) -> Result<Cell, S57Error> {
    let mut b = Builder::default();
    b.apply_file(&std::fs::read(base)?, true)?;
    for p in updates {
        let bytes = std::fs::read(p)?;
        let Some(n) = declared_update(&bytes) else {
            b.warnings.push(format!("{}: not an update file", p.display()));
            continue;
        };
        if n <= b.update {
            b.skipped.push(n);
        } else if n == b.update + 1 {
            b.apply_file(&bytes, false)?;
        } else {
            b.warnings.push(format!("update {n} cannot follow update {}: the sequence is broken", b.update));
            break;
        }
    }
    Ok(b.finish())
}

/// The numbered update files (`.001`, `.002`, ...) beside a base cell.
pub fn update_files(base: &Path) -> Vec<std::path::PathBuf> {
    (1..=999).map(|n| base.with_extension(format!("{n:03}"))).take_while(|p| p.exists()).collect()
}

/// Reads a base cell and every update beside it that is newer than it.
pub fn load_cell(base: &Path) -> Result<Cell, S57Error> {
    load_cell_with(base, &update_files(base))
}

/// Reads a cell and keeps only its summary, so a whole folder of cells can be
/// indexed without holding them all in memory.
pub fn summarize(base: &Path) -> Result<CellSummary, S57Error> {
    load_cell(base).map(|c| c.summary())
}

/// Loads only the base cell, ignoring updates (for comparison and tests).
pub fn load_base_only(base: &Path) -> Result<Cell, S57Error> {
    let mut b = Builder::default();
    b.apply_file(&std::fs::read(base)?, true)?;
    Ok(b.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_edits_follow_the_s57_rules() {
        let mut v = vec![1, 2, 3, 4];
        edit_list(&mut v, 1, 3, 2, &[10, 11]);
        assert_eq!(v, vec![1, 2, 10, 11, 3, 4], "insert before index 3");
        edit_list(&mut v, 2, 2, 2, &[]);
        assert_eq!(v, vec![1, 11, 3, 4], "delete two from index 2");
        edit_list(&mut v, 3, 2, 2, &[20, 30]);
        assert_eq!(v, vec![1, 20, 30, 4], "replace two from index 2");
        edit_list(&mut v, 1, 99, 1, &[5]);
        assert_eq!(v.last(), Some(&5), "insert past the end appends");
        edit_list(&mut v, 2, 99, 5, &[]);
        assert_eq!(v.len(), 5, "delete past the end does nothing");
        edit_list(&mut v, 9, 1, 1, &[7]);
        assert_eq!(v.len(), 5, "unknown instruction ignored");
        let mut e: Vec<i32> = Vec::new();
        edit_list(&mut e, 3, 1, 1, &[1]);
        assert_eq!(e, vec![1]);
    }

    #[test]
    fn names_are_five_bytes() {
        assert_eq!(name_key(&[130, 1, 0, 0, 0]), Some((130, 1)));
        assert_eq!(name_key(&[110, 0x34, 0x12, 0, 0]), Some((110, 0x1234)));
        assert_eq!(name_key(&[1, 2]), None);
    }

    fn builder_with_square() -> Builder {
        // Four nodes of a unit square, four edges, and an area bounded by them.
        let mut b = Builder { comf: 1000.0, somf: 10.0, ..Builder::default() };
        let node = |lat: i32, lon: i32| Spatial { pts: vec![[lat, lon]], ..Spatial::default() };
        b.spatial.insert((120, 1), node(0, 0));
        b.spatial.insert((120, 2), node(0, 1000));
        b.spatial.insert((120, 3), node(1000, 1000));
        b.spatial.insert((120, 4), node(1000, 0));
        let edge = |a: u32, z: u32, mid: Vec<[i32; 2]>| Spatial { pts: mid, soundings: vec![], vrpt: vec![Vpt { key: (120, a), topi: 1 }, Vpt { key: (120, z), topi: 2 }] };
        b.spatial.insert((130, 1), edge(1, 2, vec![[0, 500]]));
        b.spatial.insert((130, 2), edge(2, 3, vec![]));
        b.spatial.insert((130, 3), edge(3, 4, vec![]));
        b.spatial.insert((130, 4), edge(4, 1, vec![]));
        b
    }

    #[test]
    fn an_area_closes_into_one_ring_from_its_edges() {
        let mut b = builder_with_square();
        let f = Feat { prim: 3, objl: obj::DEPARE, attrs: BTreeMap::new(), fspt: (1..=4).map(|i| Spt { key: (130, i), ornt: 1 }).collect() };
        b.feats.insert(1, f);
        let cell = b.finish();
        let Geom::Area(rings) = &cell.features[0].geom else { panic!("{:?}", cell.features[0].geom) };
        assert_eq!(rings.len(), 1);
        let r = &rings[0];
        assert_eq!(r.first(), r.last());
        assert_eq!(r.len(), 6, "four corners, one mid point, and the closing point: {r:?}");
        assert!(r.contains(&[0.5, 0.0]), "the intermediate point of edge 1 is kept");
        assert_eq!(cell.bounds, Bounds { min_lon: 0.0, min_lat: 0.0, max_lon: 1.0, max_lat: 1.0 });
    }

    #[test]
    fn reversed_edges_are_turned_round() {
        let mut b = builder_with_square();
        // Walk the square the other way: edges 4,3,2,1 each reversed.
        let f = Feat { prim: 3, objl: obj::LNDARE, attrs: BTreeMap::new(), fspt: (1..=4).rev().map(|i| Spt { key: (130, i), ornt: 2 }).collect() };
        b.feats.insert(1, f);
        let cell = b.finish();
        let Geom::Area(rings) = &cell.features[0].geom else { panic!() };
        assert_eq!(rings[0].len(), 6);
        assert_eq!(rings[0].first(), rings[0].last());
        // Starting at the end of edge 4 (node 1), going to node 4 first.
        assert_eq!(rings[0][0], [0.0, 0.0]);
        assert_eq!(rings[0][1], [0.0, 1.0]);
    }

    #[test]
    fn an_island_in_an_area_makes_a_second_ring() {
        let mut b = builder_with_square();
        // A closed edge: begins and ends at one node.
        b.spatial.insert((120, 9), Spatial { pts: vec![[400, 400]], ..Spatial::default() });
        b.spatial.insert((130, 9), Spatial { pts: vec![[400, 600], [600, 600], [600, 400]], soundings: vec![], vrpt: vec![Vpt { key: (120, 9), topi: 1 }, Vpt { key: (120, 9), topi: 2 }] });
        let mut fspt: Vec<Spt> = (1..=4).map(|i| Spt { key: (130, i), ornt: 1 }).collect();
        fspt.push(Spt { key: (130, 9), ornt: 1 });
        b.feats.insert(1, Feat { prim: 3, objl: obj::DEPARE, attrs: BTreeMap::new(), fspt });
        let cell = b.finish();
        let Geom::Area(rings) = &cell.features[0].geom else { panic!() };
        assert_eq!(rings.len(), 2);
        assert_eq!(rings[1].first(), rings[1].last());
    }

    #[test]
    fn lines_chain_their_edges_without_repeating_joints() {
        let mut b = builder_with_square();
        b.feats.insert(1, Feat { prim: 2, objl: obj::COALNE, attrs: BTreeMap::new(), fspt: vec![Spt { key: (130, 1), ornt: 1 }, Spt { key: (130, 2), ornt: 1 }] });
        let cell = b.finish();
        assert_eq!(cell.features[0].geom, Geom::Line(vec![[0.0, 0.0], [0.5, 0.0], [1.0, 0.0], [1.0, 1.0]]));
    }

    #[test]
    fn soundings_are_scaled_by_somf() {
        let mut b = builder_with_square();
        b.spatial.insert((110, 5), Spatial { soundings: vec![[500, 250, 123], [750, 750, 45]], ..Spatial::default() });
        b.feats.insert(1, Feat { prim: 1, objl: obj::SOUNDG, attrs: BTreeMap::new(), fspt: vec![Spt { key: (110, 5), ornt: 1 }] });
        b.feats.insert(2, Feat { prim: 1, objl: obj::LIGHTS, attrs: BTreeMap::from([(attr::VALNMR, "12".to_string())]), fspt: vec![Spt { key: (120, 3), ornt: 1 }] });
        let cell = b.finish();
        assert_eq!(cell.features[0].geom, Geom::Soundings(vec![[0.25, 0.5, 12.3], [0.75, 0.75, 4.5]]));
        assert_eq!(cell.features[1].geom, Geom::Point([1.0, 1.0]));
        assert_eq!(cell.features[1].num(attr::VALNMR), Some(12.0));
        assert_eq!(cell.count(obj::LIGHTS), 1);
    }

    #[test]
    fn broken_references_give_no_geometry_instead_of_panicking() {
        let mut b = builder_with_square();
        b.feats.insert(1, Feat { prim: 3, objl: obj::DEPARE, attrs: BTreeMap::new(), fspt: vec![Spt { key: (130, 99), ornt: 1 }] });
        b.feats.insert(2, Feat { prim: 1, objl: obj::LIGHTS, attrs: BTreeMap::new(), fspt: vec![] });
        b.feats.insert(3, Feat { prim: 2, objl: obj::COALNE, attrs: BTreeMap::new(), fspt: vec![Spt { key: (130, 1), ornt: 1 }] });
        b.feats.insert(4, Feat { prim: 9, objl: obj::LIGHTS, attrs: BTreeMap::new(), fspt: vec![] });
        let cell = b.finish();
        assert_eq!(cell.features[0].geom, Geom::None);
        assert_eq!(cell.features[1].geom, Geom::None);
        assert!(matches!(cell.features[2].geom, Geom::Line(_)));
        assert_eq!(cell.features[3].geom, Geom::None);
    }

    #[test]
    fn attribute_helpers_parse_lists_and_numbers() {
        let f = Feature {
            rcid: 1,
            objl: obj::BOYLAT,
            attrs: BTreeMap::from([(attr::COLOUR, "3,1".to_string()), (attr::DRVAL1, "2.5".to_string()), (attr::OBJNAM, "".to_string())]),
            geom: Geom::None,
        };
        assert_eq!(f.list(attr::COLOUR), vec![3, 1]);
        assert_eq!(f.num(attr::DRVAL1), Some(2.5));
        assert_eq!(f.attr(attr::OBJNAM), None, "empty is absent");
        assert!(f.list(attr::BOYSHP).is_empty());
    }

    const IROISE: &str = "testdata/shom-iroise-s57/S57_OPENDATA_IROISEE/FR571220/FR571220.000";

    fn assert_sane(cell: &Cell) {
        for (class, name) in [(obj::DEPARE, "DEPARE"), (obj::SOUNDG, "SOUNDG"), (obj::LIGHTS, "LIGHTS"), (obj::COALNE, "COALNE"), (obj::LNDARE, "LNDARE")] {
            assert!(cell.count(class) > 0, "no {name}");
        }
        // Molène and Ouessant, in the Iroise sea.
        let b = cell.bounds;
        assert!((-5.4..-4.4).contains(&b.min_lon) && (-5.4..-4.4).contains(&b.max_lon), "{b:?}");
        assert!((48.0..48.8).contains(&b.min_lat) && (48.0..48.8).contains(&b.max_lat), "{b:?}");
        let mut areas = 0;
        for f in cell.features.iter().filter(|f| f.objl == obj::DEPARE) {
            if let Geom::Area(rings) = &f.geom {
                areas += 1;
                for r in rings {
                    assert_eq!(r.first(), r.last());
                    assert!(r.len() >= 4);
                }
                assert!(f.num(attr::DRVAL1).is_some_and(|d| (-20.0..500.0).contains(&d)), "{:?}", f.attrs);
            }
        }
        assert!(areas > 20, "{areas} depth areas");
        let soundings = cell.features.iter().find_map(|f| if let Geom::Soundings(s) = &f.geom { Some(s) } else { None }).unwrap();
        assert!(soundings.iter().all(|s| (-5.4..-4.4).contains(&s[0]) && (48.0..48.8).contains(&s[1])));
        // Names are Latin-1 (Windows-1252) and come out whole.
        let names: Vec<&str> = cell.features.iter().filter_map(|f| f.attr(attr::OBJNAM)).collect();
        assert!(names.iter().all(|n| !n.contains('\u{fffd}')), "{:?}", names.iter().filter(|n| n.contains('\u{fffd}')).collect::<Vec<_>>());
    }

    #[test]
    fn real_cells_carry_valdco_and_elevat() {
        // The tests run in the crate's folder; the data is at the workspace root.
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(IROISE);
        if !p.exists() {
            return;
        }
        let cell = load_cell(&p).unwrap();
        let contours: Vec<&Feature> = cell.features.iter().filter(|f| f.objl == obj::DEPCNT).collect();
        assert!(contours.len() > 100, "{} depth contours", contours.len());
        assert!(contours.iter().all(|f| f.num(attr::VALDCO).is_some()), "a depth contour with no VALDCO");
        // Land elevation: lines and spot heights, each with its height.
        let (mut lines, mut points) = (0, 0);
        for f in cell.features.iter().filter(|f| f.objl == obj::LNDELV) {
            assert!(f.num(attr::ELEVAT).is_some_and(|e| (0.0..500.0).contains(&e)), "{:?}", f.attrs);
            match f.geom {
                Geom::Line(_) => lines += 1,
                Geom::Point(_) => points += 1,
                _ => {}
            }
        }
        assert!(lines > 50 && points > 20, "{lines} lines, {points} points");
    }

    #[test]
    fn a_consolidated_reissue_skips_updates_it_already_contains() {
        let p = Path::new(IROISE);
        if !p.exists() {
            return;
        }
        let cell = load_cell(p).unwrap();
        assert_eq!(cell.name.split('.').next().unwrap(), "FR571220");
        assert_eq!(cell.update, 22, "the .000 file is already at update 22");
        assert_eq!(cell.skipped_updates.len(), 22, "so every numbered file is old news");
        assert_eq!(cell.cscl, 25_036);
        assert!(cell.features.len() > 1000, "{}", cell.features.len());
        assert!(cell.warnings.is_empty(), "{:?}", cell.warnings);
        assert_sane(&cell);
        assert_eq!(cell.features.len(), load_base_only(p).unwrap().features.len());
    }

    #[test]
    fn an_old_edition_plus_its_updates_reaches_the_current_state() {
        let dir = Path::new("testdata/shom-iroise-s57/S57_OPENDATA_IROISEE/FR571220");
        let old = dir.join("FR571220_20071120.000");
        if !old.exists() {
            return;
        }
        let ups = update_files(&dir.join("FR571220.000"));
        let base = load_base_only(&old).unwrap();
        let updated = load_cell_with(&old, &ups).unwrap();
        eprintln!("old edition: update {}, {} features; after {} updates: update {}, {} features, skipped {:?}, warnings {:?}", base.update, base.features.len(), ups.len(), updated.update, updated.features.len(), updated.skipped_updates, updated.warnings);
        assert!(updated.update > base.update, "some updates were applied");
        assert!(updated.warnings.is_empty(), "{:?}", updated.warnings);
        assert_sane(&updated);
        let changed = updated.features.len() != base.features.len()
            || updated.features.iter().map(|f| f.attrs.len()).sum::<usize>() != base.features.iter().map(|f| f.attrs.len()).sum::<usize>();
        assert!(changed, "the updates changed something");
        // Edition plus updates reproduces the consolidated cell, class by class.
        let current = load_base_only(&dir.join("FR571220.000")).unwrap();
        let mut a: BTreeMap<u16, usize> = BTreeMap::new();
        let mut b: BTreeMap<u16, usize> = BTreeMap::new();
        updated.features.iter().for_each(|f| *a.entry(f.objl).or_default() += 1);
        current.features.iter().for_each(|f| *b.entry(f.objl).or_default() += 1);
        eprintln!("updated {} vs consolidated {} features", updated.features.len(), current.features.len());
        let diff: Vec<_> = a.keys().chain(b.keys()).collect::<std::collections::BTreeSet<_>>().into_iter().filter(|k| a.get(k) != b.get(k)).map(|k| (*k, a.get(k).copied().unwrap_or(0), b.get(k).copied().unwrap_or(0))).collect();
        eprintln!("classes that differ (objl, updated, consolidated): {diff:?}");
        assert_eq!(updated.features.len(), current.features.len(), "{diff:?}");
    }

    #[test]
    fn the_saint_malo_cells_load_too() {
        let p = Path::new("testdata/shom-s101-pack/diffusionS101_May_June/ENC_S-57/FR571300.000");
        if !p.exists() {
            return;
        }
        let c = load_cell(p).unwrap();
        assert!(c.features.len() > 500);
        assert!(c.bounds.min_lon < -1.5 && c.bounds.max_lat > 48.0, "{:?}", c.bounds);
        assert!(c.count(obj::DEPARE) > 0 && c.count(obj::SOUNDG) > 0);
    }

    #[test]
    fn not_an_enc_is_an_error() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.000");
        std::fs::write(&p, b"definitely not an iso 8211 file at all, no way").unwrap();
        assert!(load_cell(&p).is_err());
        assert!(load_cell(&d.path().join("missing.000")).is_err());
    }

    #[test]
    fn coverage_is_the_covered_polygons_less_the_holes() {
        let square = |x0: f64, y0: f64, x1: f64, y1: f64| Geom::Area(vec![vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]]]);
        let covr = |catcov: Option<&str>, geom| Feature {
            rcid: 1,
            objl: obj::M_COVR,
            attrs: catcov.map(|c| BTreeMap::from([(attr::CATCOV, c.to_string())])).unwrap_or_default(),
            geom,
        };
        let cell = Cell {
            name: "X".into(),
            edition: 1,
            update: 0,
            issue_date: String::new(),
            cscl: 1,
            features: vec![covr(Some("1"), square(0.0, 0.0, 10.0, 10.0)), covr(Some("2"), square(4.0, 4.0, 6.0, 6.0)), covr(None, square(20.0, 0.0, 30.0, 10.0))],
            bounds: Bounds::EMPTY,
            warnings: vec![],
            skipped_updates: vec![],
        };
        let c = cell.coverage();
        assert_eq!((c.covered.len(), c.holes.len()), (2, 1), "a missing CATCOV means coverage");
        assert!(c.contains([1.0, 1.0]) && c.contains([25.0, 5.0]));
        assert!(!c.contains([5.0, 5.0]), "inside the no-coverage hole");
        assert!(!c.contains([15.0, 5.0]) && !c.contains([-1.0, 5.0]), "outside every polygon");
        assert!(Coverage::default().covered.is_empty() && !Coverage::default().contains([0.0, 0.0]));
    }

    #[test]
    fn the_iroise_cells_have_outlines_that_are_not_boxes() {
        let (a, b) = (Path::new(IROISE), Path::new("testdata/shom-iroise-s57/S57_OPENDATA_IROISEE/FR571230/FR571230.000"));
        if !a.exists() || !b.exists() {
            return;
        }
        for p in [a, b] {
            let cell = load_cell(p).unwrap();
            let c = cell.coverage();
            assert!(!c.covered.is_empty(), "{} has a coverage polygon", cell.name);
            assert!(c.covered.iter().flatten().all(|pt| cell.bounds.expanded(1e-6).contains(*pt)));
            // The polygon is a real outline: a corner of the bounding box is outside it.
            let corners = [[cell.bounds.min_lon, cell.bounds.min_lat], [cell.bounds.max_lon, cell.bounds.max_lat], [cell.bounds.min_lon, cell.bounds.max_lat], [cell.bounds.max_lon, cell.bounds.min_lat]];
            assert!(corners.iter().any(|k| !c.contains(*k)), "{}", cell.name);
            // And the middle of the data is inside.
            let inside = c.covered[0].iter().fold([0.0, 0.0], |m, p| [m[0] + p[0] / c.covered[0].len() as f64, m[1] + p[1] / c.covered[0].len() as f64]);
            eprintln!("{} coverage: {} polygon(s), {} hole(s), mean point inside: {}", cell.name, c.covered.len(), c.holes.len(), c.contains(inside));
        }
    }

    #[test]
    fn usage_bands_are_derived_from_name_or_scale() {
        assert_eq!(usage_band("FR571220.000", 10_000), 5);
        assert_eq!(usage_band("US1WC01M", 2_000_000), 1);
        assert_eq!(usage_band("GB200001", 500_000), 2);
        assert_eq!(usage_band("DE300001", 150_000), 3);
        assert_eq!(usage_band("NO400001", 45_000), 4);
        assert_eq!(usage_band("CA600001", 2_000), 6);
        // Fallback to compilation scale when name has no standard band character
        assert_eq!(usage_band("CUSTOM_CHART", 2_000_000), 1);
        assert_eq!(usage_band("CUSTOM_CHART", 500_000), 2);
        assert_eq!(usage_band("CUSTOM_CHART", 150_000), 3);
        assert_eq!(usage_band("CUSTOM_CHART", 45_000), 4);
        assert_eq!(usage_band("CUSTOM_CHART", 10_000), 5);
        assert_eq!(usage_band("CUSTOM_CHART", 2_000), 6);
        assert_eq!(usage_band_name(5), "Harbour");
        assert_eq!(usage_band_name(3), "Coastal");
    }
}

