//! Vector tiles from the Open Waters "seamap" tileset: OpenStreetMap land
//! polygons and seamarks, served as plain Mapbox vector tiles
//! (`/{z}/{x}/{y}.pbf`, zoom 0 to 14).
//!
//! One tile carries both the coastline (`land` layer) and the marks
//! (`seamark` layer), so the app fetches a tile once and uses what it needs.
//! Tiles are cached on disk as the raw answer. The data is
//! © Open Waters, © OpenStreetMap contributors (ODbL), volunteer made:
//! planning only.

use crate::geo::LatLon;
use crate::vector::{Bounds, Polygon};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

const URL: &str = "https://tiles.openwaters.io/seamap";
/// The credit the licences ask for whenever tile data is on screen.
pub const CREDIT: &str = "© Open Waters, © OpenStreetMap contributors";
pub const MAX_ZOOM: u8 = 14;
/// How long a cached tile is used before it is fetched again.
pub const CACHE_TTL: Duration = Duration::from_secs(30 * 24 * 3600);
/// A tile is a few tens of kilobytes; anything far bigger is not one.
pub(crate) const MAX_BYTES: u64 = 8 * 1024 * 1024;
/// Tiles reach this far past their edge, in tile units, so fills overlap.
pub const BUFFER: i32 = 64;

#[derive(Debug, PartialEq)]
pub enum TileError {
    Network(String),
    Format(String),
    /// Not on disk, and downloading is not allowed.
    Offline,
    /// No route to the server at all: no network, or no name lookup. Not
    /// worth telling the user about each time; the download is tried again.
    NoConnection,
}

impl TileError {
    /// What a failed request means: no connection, or a real network error.
    pub fn from_net(e: ureq::Error) -> TileError {
        use std::io::ErrorKind as K;
        use ureq::Timeout as T;
        match &e {
            ureq::Error::HostNotFound | ureq::Error::ConnectionFailed | ureq::Error::Timeout(T::Resolve | T::Connect) => TileError::NoConnection,
            ureq::Error::Io(io) if matches!(io.kind(), K::NetworkUnreachable | K::HostUnreachable | K::NetworkDown | K::AddrNotAvailable) => {
                TileError::NoConnection
            }
            _ => TileError::Network(e.to_string()),
        }
    }

    /// Whether this failure is worth a message: not when downloads are off or
    /// there is no network.
    pub fn worth_telling(&self) -> bool {
        !matches!(self, TileError::Offline | TileError::NoConnection)
    }
}

impl std::fmt::Display for TileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TileError::Network(m) | TileError::Format(m) => write!(f, "{m}"),
            TileError::Offline => write!(f, "not downloaded yet"),
            TileError::NoConnection => write!(f, "no internet connection"),
        }
    }
}

// ---- tile addresses --------------------------------------------------------

/// A Web Mercator tile, as in every slippy map.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileId {
    pub z: u8,
    pub x: u32,
    pub y: u32,
}

impl TileId {
    /// The tile zoom that suits a view at `view_zoom`: tiles are then drawn
    /// between 1 and 2 times their size.
    pub fn zoom_for(view_zoom: f64) -> u8 {
        view_zoom.floor().clamp(0.0, MAX_ZOOM as f64) as u8
    }

    fn count(&self) -> f64 {
        (1u64 << self.z) as f64
    }

    /// The tile holding a position.
    pub fn at(z: u8, lat: f64, lon: f64) -> TileId {
        let n = (1u64 << z) as f64;
        let lat = lat.clamp(-85.05112878, 85.05112878).to_radians();
        let fx = (lon + 180.0) / 360.0;
        let fy = (1.0 - lat.tan().asinh() / std::f64::consts::PI) / 2.0;
        let clamp = |f: f64| ((f * n).floor().max(0.0) as u32).min(n as u32 - 1);
        TileId { z, x: clamp(fx), y: clamp(fy) }
    }

    pub fn parent(&self) -> Option<TileId> {
        (self.z > 0).then(|| TileId { z: self.z - 1, x: self.x / 2, y: self.y / 2 })
    }

    fn lon_at(&self, f: f64) -> f64 {
        (self.x as f64 + f) / self.count() * 360.0 - 180.0
    }

    fn lat_at(&self, f: f64) -> f64 {
        (std::f64::consts::PI * (1.0 - 2.0 * (self.y as f64 + f) / self.count())).sinh().atan().to_degrees()
    }

    pub fn bounds(&self) -> Bounds {
        Bounds { min_lon: self.lon_at(0.0), min_lat: self.lat_at(1.0), max_lon: self.lon_at(1.0), max_lat: self.lat_at(0.0) }
    }

    /// The position of a point in tile units (`0..extent`).
    pub fn point(&self, px: f64, py: f64, extent: f64) -> LatLon {
        LatLon::new(self.lat_at(py / extent), self.lon_at(px / extent))
    }

    fn file(&self, dir: &Path) -> PathBuf {
        dir.join(self.z.to_string()).join(self.x.to_string()).join(format!("{}.pbf", self.y))
    }
}

/// The tiles of zoom `z` touching `b`, nearest to its centre first, at most `max`.
pub fn tiles_for(b: &Bounds, z: u8, max: usize) -> Vec<TileId> {
    // A view ending exactly on a tile edge does not need the tile beyond it.
    const EDGE: f64 = 1e-9;
    let nw = TileId::at(z, b.max_lat - EDGE, b.min_lon + EDGE);
    let se = TileId::at(z, b.min_lat + EDGE, b.max_lon - EDGE);
    let (x1, y1) = (se.x.max(nw.x), se.y.max(nw.y));
    let centre = [(b.min_lon + b.max_lon) / 2.0, (b.min_lat + b.max_lat) / 2.0];
    let mut v: Vec<TileId> = (nw.y..=y1).flat_map(|y| (nw.x..=x1).map(move |x| TileId { z, x, y })).collect();
    let dist = |t: &TileId| {
        let tb = t.bounds();
        ((tb.min_lon + tb.max_lon) / 2.0 - centre[0]).powi(2) + ((tb.min_lat + tb.max_lat) / 2.0 - centre[1]).powi(2)
    };
    v.sort_by(|a, b| dist(a).total_cmp(&dist(b)));
    v.truncate(max);
    v
}

/// For each wanted tile, the loaded tile to draw in its place: the tile itself,
/// else its nearest loaded ancestor. Returns each such tile once, with the
/// wanted tiles it stands in for.
pub fn resolve(wanted: &[TileId], has: impl Fn(&TileId) -> bool) -> Vec<(TileId, Vec<TileId>)> {
    let mut out: Vec<(TileId, Vec<TileId>)> = Vec::new();
    for w in wanted {
        let mut t = Some(*w);
        while let Some(c) = t {
            if has(&c) {
                break;
            }
            t = c.parent();
        }
        let Some(src) = t else { continue };
        match out.iter_mut().find(|(s, _)| *s == src) {
            Some((_, kids)) => kids.push(*w),
            None => out.push((src, vec![*w])),
        }
    }
    out
}

/// Loaded tiles, the least recently used dropped when there are too many.
pub struct TileCache<T> {
    map: HashMap<TileId, (T, u64)>,
    stamp: u64,
    cap: usize,
}

impl<T: Clone> TileCache<T> {
    pub fn new(cap: usize) -> Self {
        TileCache { map: HashMap::new(), stamp: 0, cap }
    }

    pub fn insert(&mut self, id: TileId, v: T) {
        self.stamp += 1;
        self.map.insert(id, (v, self.stamp));
        if self.map.len() > self.cap
            && let Some(oldest) = self.map.iter().min_by_key(|(_, (_, s))| *s).map(|(k, _)| *k)
        {
            self.map.remove(&oldest);
        }
    }

    pub fn contains(&self, id: &TileId) -> bool {
        self.map.contains_key(id)
    }

    /// Every tile held, without counting it as used.
    pub fn iter(&self) -> impl Iterator<Item = (&TileId, &T)> {
        self.map.iter().map(|(k, (v, _))| (k, v))
    }

    /// The tile, counted as used just now.
    pub fn get(&mut self, id: &TileId) -> Option<T> {
        self.stamp += 1;
        let stamp = self.stamp;
        self.map.get_mut(id).map(|(v, s)| {
            *s = stamp;
            v.clone()
        })
    }
}

// ---- disk cache and download -----------------------------------------------

/// The cached bytes of a tile and whether they are still fresh. An empty tile
/// is cached as an empty file.
pub fn read_cache(dir: &Path, id: &TileId) -> Option<(Vec<u8>, bool)> {
    let path = id.file(dir);
    let bytes = std::fs::read(&path).ok()?;
    let age = std::fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok());
    Some((bytes, age.is_some_and(|a| a < CACHE_TTL)))
}

pub fn write_cache(dir: &Path, id: &TileId, bytes: &[u8]) -> std::io::Result<()> {
    let path = id.file(dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("part");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(tmp, path)
}

pub fn forget(dir: &Path, id: &TileId) {
    let _ = std::fs::remove_file(id.file(dir));
}

pub(crate) fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .user_agent("isomar/0.1 (open-source chart plotter)")
            .http_status_as_error(false)
            .build()
            .into()
    })
}

/// Downloads a tile and caches it. Blocking; call from a worker thread.
pub fn fetch(dir: &Path, id: &TileId) -> Result<Vec<u8>, TileError> {
    let net = TileError::from_net;
    let mut res = agent().get(format!("{URL}/{}/{}/{}.pbf", id.z, id.x, id.y)).call().map_err(net)?;
    let bytes = match res.status().as_u16() {
        200 => res.body_mut().with_config().limit(MAX_BYTES).read_to_vec().map_err(net)?,
        // Nothing mapped here.
        204 | 404 => Vec::new(),
        s => return Err(TileError::Network(format!("tile server answered {s}"))),
    };
    // A cache that can't be written still serves this session.
    let _ = write_cache(dir, id, &bytes);
    Ok(bytes)
}

/// Deletes cached tiles not written for `age`, and the folders left empty.
pub fn prune(dir: &Path, age: Duration) {
    let old = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok()).is_some_and(|a| a > age);
    let list = |p: &Path| std::fs::read_dir(p).map(|d| d.flatten().map(|e| e.path()).collect::<Vec<_>>()).unwrap_or_default();
    for zdir in list(dir) {
        for xdir in list(&zdir) {
            for f in list(&xdir) {
                if old(&f) {
                    let _ = std::fs::remove_file(f);
                }
            }
            let _ = std::fs::remove_dir(&xdir);
        }
        let _ = std::fs::remove_dir(&zdir);
    }
}

// ---- decoding --------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeomKind {
    Point,
    Line,
    Polygon,
}

#[derive(Debug)]
pub struct Feature {
    pub kind: GeomKind,
    /// Points (one per path), line strings, or polygon rings (not closed
    /// explicitly), in tile units.
    pub paths: Vec<Vec<[i32; 2]>>,
    pub tags: HashMap<String, String>,
}

#[derive(Debug)]
pub struct Layer {
    pub name: String,
    pub extent: u32,
    pub features: Vec<Feature>,
}

#[derive(Debug, Default)]
pub struct VectorTile {
    pub layers: Vec<Layer>,
}

impl VectorTile {
    pub fn layer(&self, name: &str) -> Option<&Layer> {
        self.layers.iter().find(|l| l.name == name)
    }
}

/// Malformed input; the caller turns it into a [`TileError`].
struct Bad;

enum Wire<'a> {
    Varint(u64),
    Fixed64(u64),
    Fixed32(u32),
    Bytes(&'a [u8]),
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn varint(&mut self) -> Result<u64, Bad> {
        let mut v = 0u64;
        for shift in (0..70).step_by(7) {
            let byte = *self.b.get(self.i).ok_or(Bad)?;
            self.i += 1;
            v |= ((byte & 0x7f) as u64).checked_shl(shift).ok_or(Bad)?;
            if byte & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(Bad)
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], Bad> {
        let s = self.b.get(self.i..self.i.checked_add(n).ok_or(Bad)?).ok_or(Bad)?;
        self.i += n;
        Ok(s)
    }

    /// The next field number and value, or `None` at the end.
    fn next(&mut self) -> Result<Option<(u32, Wire<'a>)>, Bad> {
        if self.i >= self.b.len() {
            return Ok(None);
        }
        let key = self.varint()?;
        let wire = match key & 7 {
            0 => Wire::Varint(self.varint()?),
            1 => Wire::Fixed64(u64::from_le_bytes(self.take(8)?.try_into().map_err(|_| Bad)?)),
            2 => {
                let n = self.varint()? as usize;
                Wire::Bytes(self.take(n)?)
            }
            5 => Wire::Fixed32(u32::from_le_bytes(self.take(4)?.try_into().map_err(|_| Bad)?)),
            _ => return Err(Bad),
        };
        Ok(Some(((key >> 3) as u32, wire)))
    }
}

fn varints(b: &[u8]) -> Result<Vec<u32>, Bad> {
    let mut r = Reader { b, i: 0 };
    let mut out = Vec::new();
    while r.i < b.len() {
        out.push(r.varint()? as u32);
    }
    Ok(out)
}

fn zigzag(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

fn value_text(b: &[u8]) -> Result<String, Bad> {
    let mut r = Reader { b, i: 0 };
    let mut out = String::new();
    while let Some((f, w)) = r.next()? {
        match (f, w) {
            (1, Wire::Bytes(s)) => out = String::from_utf8_lossy(s).into_owned(),
            (2, Wire::Fixed32(v)) => out = f32::from_bits(v).to_string(),
            (3, Wire::Fixed64(v)) => out = f64::from_bits(v).to_string(),
            (4, Wire::Varint(v)) => out = (v as i64).to_string(),
            (5, Wire::Varint(v)) => out = v.to_string(),
            (6, Wire::Varint(v)) => out = zigzag(v).to_string(),
            (7, Wire::Varint(v)) => out = (v != 0).to_string(),
            _ => {}
        }
    }
    Ok(out)
}

fn paths_of(commands: &[u32]) -> Result<Vec<Vec<[i32; 2]>>, Bad> {
    let mut paths: Vec<Vec<[i32; 2]>> = Vec::new();
    let (mut i, mut x, mut y) = (0, 0i32, 0i32);
    while i < commands.len() {
        let (id, count) = (commands[i] & 7, (commands[i] >> 3) as usize);
        i += 1;
        match id {
            1 | 2 => {
                for _ in 0..count {
                    let (dx, dy) = (*commands.get(i).ok_or(Bad)?, *commands.get(i + 1).ok_or(Bad)?);
                    i += 2;
                    x = x.wrapping_add(zigzag(dx as u64) as i32);
                    y = y.wrapping_add(zigzag(dy as u64) as i32);
                    match (id, paths.last_mut()) {
                        (2, Some(p)) => p.push([x, y]),
                        (2, None) => return Err(Bad),
                        _ => paths.push(vec![[x, y]]),
                    }
                }
            }
            7 => {}
            _ => return Err(Bad),
        }
    }
    Ok(paths)
}

struct RawFeature {
    tags: Vec<u32>,
    kind: u64,
    geometry: Vec<u32>,
}

fn decode_layer(b: &[u8]) -> Result<Layer, Bad> {
    let mut r = Reader { b, i: 0 };
    let (mut name, mut extent) = (String::new(), 4096);
    let (mut keys, mut values, mut raw) = (Vec::new(), Vec::new(), Vec::new());
    while let Some((f, w)) = r.next()? {
        match (f, w) {
            (1, Wire::Bytes(s)) => name = String::from_utf8_lossy(s).into_owned(),
            (2, Wire::Bytes(s)) => {
                let mut fr = Reader { b: s, i: 0 };
                let mut feature = RawFeature { tags: Vec::new(), kind: 0, geometry: Vec::new() };
                while let Some((f, w)) = fr.next()? {
                    match (f, w) {
                        (2, Wire::Bytes(t)) => feature.tags = varints(t)?,
                        (3, Wire::Varint(k)) => feature.kind = k,
                        (4, Wire::Bytes(g)) => feature.geometry = varints(g)?,
                        _ => {}
                    }
                }
                raw.push(feature);
            }
            (3, Wire::Bytes(s)) => keys.push(String::from_utf8_lossy(s).into_owned()),
            (4, Wire::Bytes(s)) => values.push(value_text(s)?),
            (5, Wire::Varint(v)) => extent = v.clamp(1, 1 << 20) as u32,
            _ => {}
        }
    }
    let mut features = Vec::with_capacity(raw.len());
    for f in raw {
        let kind = match f.kind {
            1 => GeomKind::Point,
            2 => GeomKind::Line,
            3 => GeomKind::Polygon,
            _ => continue,
        };
        let tags = f
            .tags
            .as_chunks::<2>()
            .0
            .iter()
            .filter_map(|kv| Some((keys.get(kv[0] as usize)?.clone(), values.get(kv[1] as usize)?.clone())))
            .collect();
        features.push(Feature { kind, paths: paths_of(&f.geometry)?, tags });
    }
    Ok(Layer { name, extent, features })
}

/// Decodes a tile. An empty answer is an empty tile.
pub fn decode(bytes: &[u8]) -> Result<VectorTile, TileError> {
    let bad = || TileError::Format("not a vector tile".into());
    let mut r = Reader { b: bytes, i: 0 };
    let mut tile = VectorTile::default();
    while let Some((f, w)) = r.next().map_err(|_| bad())? {
        if let (3, Wire::Bytes(s)) = (f, w) {
            tile.layers.push(decode_layer(s).map_err(|_| bad())?);
        }
    }
    Ok(tile)
}

// ---- land ------------------------------------------------------------------

/// The land of one tile: polygon rings in tile units, filled even-odd.
#[derive(Debug, Default, PartialEq)]
pub struct LandTile {
    pub extent: i32,
    pub rings: Vec<Vec<[i16; 2]>>,
}

impl LandTile {
    pub fn from_tile(t: &VectorTile) -> LandTile {
        let Some(layer) = t.layer("land") else { return LandTile::default() };
        let clamp = |v: i32| v.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        let rings = layer
            .features
            .iter()
            .filter(|f| f.kind == GeomKind::Polygon)
            .flat_map(|f| f.paths.iter())
            .filter(|p| p.len() >= 3)
            .map(|p| p.iter().map(|q| [clamp(q[0]), clamp(q[1])]).collect())
            .collect();
        LandTile { extent: layer.extent as i32, rings }
    }

    pub fn is_empty(&self) -> bool {
        self.rings.is_empty()
    }

    /// Whether the segment lies on the artificial edge of the tile's buffer
    /// and so is not coast.
    pub fn on_edge(&self, a: [i16; 2], b: [i16; 2]) -> bool {
        let out = |v: i16| (v as i32) < 0 || (v as i32) > self.extent;
        (a[0] == b[0] && out(a[0])) || (a[1] == b[1] && out(a[1]))
    }

    /// The land as one lon/lat polygon, for the router.
    pub fn polygon(&self, id: &TileId) -> Polygon {
        let e = self.extent as f64;
        let rings = self
            .rings
            .iter()
            .map(|r| {
                r.iter()
                    .map(|p| {
                        let ll = id.point(p[0] as f64, p[1] as f64, e);
                        [ll.lon, ll.lat]
                    })
                    .collect()
            })
            .collect();
        Polygon { rings }
    }
}

#[cfg(any(test, feature = "testutil"))]
pub mod testutil {
    //! A small encoder, so tests need no downloaded data.
    fn varint(out: &mut Vec<u8>, mut v: u64) {
        while v >= 0x80 {
            out.push((v as u8 & 0x7f) | 0x80);
            v >>= 7;
        }
        out.push(v as u8);
    }

    fn bytes(out: &mut Vec<u8>, field: u32, b: &[u8]) {
        varint(out, ((field as u64) << 3) | 2);
        varint(out, b.len() as u64);
        out.extend_from_slice(b);
    }

    fn zig(v: i32) -> u64 {
        ((v << 1) ^ (v >> 31)) as u32 as u64
    }

    /// Geometry commands for paths: a point per path, or a ring/line.
    pub fn geometry(paths: &[Vec<[i32; 2]>], close: bool) -> Vec<u8> {
        let mut out = Vec::new();
        let (mut x, mut y) = (0, 0);
        for p in paths {
            varint(&mut out, 1 | (1 << 3));
            varint(&mut out, zig(p[0][0] - x));
            varint(&mut out, zig(p[0][1] - y));
            (x, y) = (p[0][0], p[0][1]);
            if p.len() > 1 {
                varint(&mut out, 2 | (((p.len() - 1) as u64) << 3));
                for q in &p[1..] {
                    varint(&mut out, zig(q[0] - x));
                    varint(&mut out, zig(q[1] - y));
                    (x, y) = (q[0], q[1]);
                }
            }
            if close {
                varint(&mut out, 7 | (1 << 3));
            }
        }
        out
    }

    pub enum V<'a> {
        S(&'a str),
        Double(f64),
        Int(i64),
        Bool(bool),
    }

    pub struct F<'a> {
        pub kind: u64,
        pub paths: Vec<Vec<[i32; 2]>>,
        pub tags: Vec<(&'a str, V<'a>)>,
    }

    /// One layer with its features, as a tile of that layer alone.
    pub fn layer(name: &str, extent: u32, features: &[F]) -> Vec<u8> {
        let (mut keys, mut vals): (Vec<&str>, Vec<Vec<u8>>) = (Vec::new(), Vec::new());
        let mut body = Vec::new();
        bytes(&mut body, 1, name.as_bytes());
        for f in features {
            let mut fb = Vec::new();
            let mut tags = Vec::new();
            for (k, v) in &f.tags {
                let ki = keys.iter().position(|x| x == k).unwrap_or_else(|| {
                    keys.push(k);
                    keys.len() - 1
                });
                let mut vb = Vec::new();
                match v {
                    V::S(s) => bytes(&mut vb, 1, s.as_bytes()),
                    V::Double(d) => {
                        varint(&mut vb, (3 << 3) | 1);
                        vb.extend_from_slice(&d.to_le_bytes());
                    }
                    V::Int(i) => {
                        varint(&mut vb, 4 << 3);
                        varint(&mut vb, *i as u64);
                    }
                    V::Bool(b) => {
                        varint(&mut vb, 7 << 3);
                        varint(&mut vb, *b as u64);
                    }
                }
                let vi = vals.iter().position(|x| *x == vb).unwrap_or_else(|| {
                    vals.push(vb);
                    vals.len() - 1
                });
                varint(&mut tags, ki as u64);
                varint(&mut tags, vi as u64);
            }
            bytes(&mut fb, 2, &tags);
            varint(&mut fb, 3 << 3);
            varint(&mut fb, f.kind);
            bytes(&mut fb, 4, &geometry(&f.paths, f.kind == 3));
            bytes(&mut body, 2, &fb);
        }
        for k in keys {
            bytes(&mut body, 3, k.as_bytes());
        }
        for v in vals {
            bytes(&mut body, 4, &v);
        }
        varint(&mut body, 5 << 3);
        varint(&mut body, extent as u64);
        let mut tile = Vec::new();
        bytes(&mut tile, 3, &body);
        tile
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;

    #[test]
    fn no_network_is_told_apart_from_a_server_error() {
        use std::io::{Error, ErrorKind};
        let unreachable = TileError::from_net(ureq::Error::Io(Error::from(ErrorKind::NetworkUnreachable)));
        assert_eq!(unreachable, TileError::NoConnection);
        assert!(!unreachable.worth_telling());
        assert_eq!(TileError::from_net(ureq::Error::HostNotFound), TileError::NoConnection);
        let reset = TileError::from_net(ureq::Error::Io(Error::from(ErrorKind::ConnectionReset)));
        assert!(matches!(reset, TileError::Network(_)) && reset.worth_telling());
    }

    #[test]
    fn tile_addresses() {
        // Brest at zoom 10, as the tile server numbers it.
        assert_eq!(TileId::at(10, 48.38, -4.49), TileId { z: 10, x: 499, y: 354 });
        assert_eq!(TileId::at(0, 80.0, 179.9), TileId { z: 0, x: 0, y: 0 });
        assert_eq!(TileId::at(3, -89.0, 180.0), TileId { z: 3, x: 7, y: 7 }, "clamped to the world");
        let t = TileId { z: 10, x: 499, y: 354 };
        let b = t.bounds();
        assert!(b.contains([-4.49, 48.38]), "{b:?}");
        assert!((b.max_lon - b.min_lon - 360.0 / 1024.0).abs() < 1e-9);
        assert_eq!(t.parent(), Some(TileId { z: 9, x: 249, y: 177 }));
        assert_eq!(TileId { z: 0, x: 0, y: 0 }.parent(), None);
        // The corners of the tile in tile units.
        let nw = t.point(0.0, 0.0, 4096.0);
        assert!((nw.lat - b.max_lat).abs() < 1e-9 && (nw.lon - b.min_lon).abs() < 1e-9);
    }

    #[test]
    fn zoom_follows_the_view_and_stops_at_the_data() {
        assert_eq!(TileId::zoom_for(-1.0), 0);
        assert_eq!(TileId::zoom_for(9.99), 9);
        assert_eq!(TileId::zoom_for(10.0), 10);
        assert_eq!(TileId::zoom_for(17.3), MAX_ZOOM);
    }

    #[test]
    fn tiles_cover_the_view_nearest_first() {
        // Exactly one tile at zoom 10, edge to edge.
        let t = TileId { z: 10, x: 499, y: 354 }.bounds();
        assert_eq!(tiles_for(&t, 10, 99), vec![TileId { z: 10, x: 499, y: 354 }]);
        // A view across two tiles, the nearer first.
        let b = Bounds { min_lon: t.max_lon - 0.01, max_lon: t.max_lon + 0.03, min_lat: t.min_lat + 0.01, max_lat: t.min_lat + 0.02 };
        let v = tiles_for(&b, 10, 99);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].x, 500, "the view's centre is in the east tile");
        assert_eq!(tiles_for(&b, 10, 1).len(), 1);
        // The whole world at zoom 1.
        let world = Bounds { min_lon: -180.0, min_lat: -85.0, max_lon: 180.0, max_lat: 85.0 };
        assert_eq!(tiles_for(&world, 1, 99).len(), 4);
    }

    #[test]
    fn a_missing_tile_is_replaced_by_the_nearest_ancestor() {
        let a = TileId { z: 5, x: 10, y: 10 };
        let kids = [TileId { z: 6, x: 20, y: 20 }, TileId { z: 6, x: 21, y: 20 }, TileId { z: 6, x: 22, y: 20 }];
        let r = resolve(&kids, |t| *t == a || *t == kids[2]);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0], (a, vec![kids[0], kids[1]]));
        assert_eq!(r[1], (kids[2], vec![kids[2]]));
        assert!(resolve(&kids, |_| false).is_empty());
    }

    #[test]
    fn the_cache_forgets_the_least_recently_used() {
        let id = |x| TileId { z: 3, x, y: 0 };
        let mut c = TileCache::new(2);
        c.insert(id(0), 'a');
        c.insert(id(1), 'b');
        assert_eq!(c.get(&id(0)), Some('a'));
        c.insert(id(2), 'c');
        assert!(c.contains(&id(0)) && c.contains(&id(2)) && !c.contains(&id(1)));
    }

    #[test]
    fn decodes_points_with_every_kind_of_value() {
        let bytes = layer(
            "seamark",
            4096,
            &[F {
                kind: 1,
                paths: vec![vec![[100, 200]], vec![[-5, 4100]]],
                tags: vec![("seamark:type", V::S("buoy_lateral")), ("range", V::Double(9.0)), ("rank", V::Int(2)), ("near", V::Bool(true))],
            }],
        );
        let t = decode(&bytes).unwrap();
        let l = t.layer("seamark").unwrap();
        assert_eq!(l.extent, 4096);
        let f = &l.features[0];
        assert_eq!((f.kind, f.paths.clone()), (GeomKind::Point, vec![vec![[100, 200]], vec![[-5, 4100]]]));
        assert_eq!(f.tags["seamark:type"], "buoy_lateral");
        assert_eq!((f.tags["range"].as_str(), f.tags["rank"].as_str(), f.tags["near"].as_str()), ("9", "2", "true"));
    }

    #[test]
    fn decodes_polygon_rings_and_lines() {
        let outer = vec![[-64, -64], [4160, -64], [4160, 4160], [-64, 4160]];
        let hole = vec![[1000, 1000], [1000, 2000], [2000, 2000]];
        let bytes = layer("land", 4096, &[F { kind: 3, paths: vec![outer.clone(), hole.clone()], tags: vec![] }]);
        let t = decode(&bytes).unwrap();
        assert_eq!(t.layer("land").unwrap().features[0].paths, vec![outer, hole]);
        let bytes = layer("waterway", 4096, &[F { kind: 2, paths: vec![vec![[0, 0], [10, 0], [10, 10]]], tags: vec![] }]);
        assert_eq!(decode(&bytes).unwrap().layer("waterway").unwrap().features[0].kind, GeomKind::Line);
    }

    #[test]
    fn malformed_input_is_an_error_not_a_panic() {
        assert!(decode(b"").unwrap().layers.is_empty(), "an empty tile");
        assert!(decode(b"<html>busy</html>").is_err());
        let good = layer("land", 4096, &[F { kind: 3, paths: vec![vec![[0, 0], [10, 0], [10, 10]]], tags: vec![] }]);
        for cut in 0..good.len() {
            // Never panics; most truncations are errors.
            let _ = decode(&good[..cut]);
        }
        // A varint that never ends.
        assert!(decode(&[0xff; 20]).is_err());
    }

    #[test]
    fn land_keeps_the_rings_and_knows_the_buffer_edge() {
        let outer = vec![[-64, -64], [4160, -64], [4160, 4160], [-64, 4160]];
        let bytes = layer("land", 4096, &[F { kind: 3, paths: vec![outer, vec![[1, 1], [2, 2]]], tags: vec![] }]);
        let land = LandTile::from_tile(&decode(&bytes).unwrap());
        assert_eq!(land.rings.len(), 1, "a ring needs three points");
        assert!(land.on_edge([-64, -64], [4160, -64]));
        assert!(land.on_edge([4160, 4160], [4160, -64]));
        assert!(!land.on_edge([100, 100], [100, 900]), "a coast inside the tile");
        assert!(LandTile::from_tile(&VectorTile::default()).is_empty());
    }

    #[test]
    fn land_becomes_a_lon_lat_polygon_for_routing() {
        let id = TileId { z: 4, x: 7, y: 5 };
        let bytes = layer("land", 4096, &[F { kind: 3, paths: vec![vec![[0, 0], [4096, 0], [4096, 4096], [0, 4096]]], tags: vec![] }]);
        let p = LandTile::from_tile(&decode(&bytes).unwrap()).polygon(&id);
        let b = p.bounds();
        let t = id.bounds();
        assert!((b.min_lon - t.min_lon).abs() < 1e-9 && (b.max_lat - t.max_lat).abs() < 1e-9);
        assert!(p.contains([(t.min_lon + t.max_lon) / 2.0, (t.min_lat + t.max_lat) / 2.0]));
    }

    #[test]
    fn the_disk_cache_returns_what_was_written_and_prunes() {
        let d = tempfile::tempdir().unwrap();
        let id = TileId { z: 8, x: 124, y: 88 };
        assert!(read_cache(d.path(), &id).is_none());
        write_cache(d.path(), &id, b"abc").unwrap();
        write_cache(d.path(), &TileId { z: 8, x: 124, y: 89 }, b"").unwrap();
        let (bytes, fresh) = read_cache(d.path(), &id).unwrap();
        assert_eq!((bytes.as_slice(), fresh), (&b"abc"[..], true));
        assert_eq!(read_cache(d.path(), &TileId { z: 8, x: 124, y: 89 }).unwrap().0.len(), 0, "an empty tile is remembered");
        prune(d.path(), Duration::from_secs(3600));
        assert!(read_cache(d.path(), &id).is_some(), "recent tiles stay");
        prune(d.path(), Duration::ZERO);
        assert!(read_cache(d.path(), &id).is_none());
        assert!(!d.path().join("8").exists(), "empty folders go too");
    }

    /// Live check. Run with `cargo test --release real_tile -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_tile_for_brest_has_land_and_marks() {
        let d = tempfile::tempdir().unwrap();
        let id = TileId::at(10, 48.38, -4.49);
        let started = std::time::Instant::now();
        let bytes = fetch(d.path(), &id).unwrap();
        let t = decode(&bytes).unwrap();
        let land = LandTile::from_tile(&t);
        let marks = t.layer("seamark").map(|l| l.features.len()).unwrap_or(0);
        eprintln!("{} bytes, {} land rings, {} seamark features in {:?}", bytes.len(), land.rings.len(), marks, started.elapsed());
        assert!(!land.is_empty() && marks > 0);
        assert_eq!(read_cache(d.path(), &id).unwrap().0, bytes);
        // The open sea far from anything is an empty tile.
        let sea = TileId::at(12, 46.0, -20.0);
        assert!(decode(&fetch(d.path(), &sea).unwrap()).unwrap().layers.is_empty());
    }
}
