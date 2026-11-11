//! Land elevation from the AWS Open Data "Terrain Tiles" (Mapzen): Terrarium
//! PNG tiles in Web Mercator, 256 px, addressed `/{z}/{x}/{y}.png`. The heights
//! come from SRTM (30 m, public domain) and national models such as USGS 3DEP.
//! They feed the hillshade and the land contours of the planning chart.
//!
//! A pixel holds the height in metres as `R * 256 + G + B / 256 - 32768`.
//! Tiles are cached on disk as the raw answer; terrain does not change, so a
//! cached tile is used for a year. Not for navigation.

use super::vtile::{self, TileCache, TileError, TileId};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

const URL: &str = "https://s3.amazonaws.com/elevation-tiles-prod/terrarium";
/// The credit the data's sources ask for whenever it is on screen.
pub const CREDIT: &str = "Terrain: SRTM, USGS 3DEP and national models (Mapzen, AWS Open Data)";
/// Pixels along a tile's side.
pub const SIZE: usize = 256;
/// The finest zoom fetched: the data is about 30 m a pixel, so finer tiles
/// only repeat it. The service has tiles to zoom 15.
pub const MAX_ZOOM: u8 = 13;
/// The coarsest zoom fetched; below it the relief is not drawn.
pub const MIN_ZOOM: u8 = 6;
/// How long a cached tile is used before it is fetched again.
pub const CACHE_TTL: Duration = Duration::from_secs(365 * 24 * 3600);
/// A tile is some tens of kilobytes; anything far bigger is not one.
const MAX_BYTES: u64 = 4 * 1024 * 1024;

/// The tile zoom that suits a view at `view_zoom`.
pub fn zoom_for(view_zoom: f64) -> u8 {
    view_zoom.floor().clamp(MIN_ZOOM as f64, MAX_ZOOM as f64) as u8
}

// ---- a tile ------------------------------------------------------------------

/// The heights of one tile, in metres, row-major from the north-west corner.
/// NaN is a pixel with no data.
#[derive(Debug, PartialEq)]
pub struct ElevTile {
    pub heights: Vec<f32>,
}

impl ElevTile {
    pub fn from_png(bytes: &[u8]) -> Result<ElevTile, TileError> {
        let bad = |m: &str| TileError::Format(format!("elevation tile: {m}"));
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().map_err(|e| bad(&e.to_string()))?;
        let info = reader.info();
        if (info.width as usize, info.height as usize) != (SIZE, SIZE) {
            return Err(bad("not 256 by 256"));
        }
        let (colour, depth) = reader.output_color_type();
        let channels = match (colour, depth) {
            (png::ColorType::Rgb, png::BitDepth::Eight) => 3,
            (png::ColorType::Rgba, png::BitDepth::Eight) => 4,
            _ => return Err(bad("not 8-bit RGB")),
        };
        let mut buf = vec![0; reader.output_buffer_size().ok_or_else(|| bad("too big"))?];
        let frame = reader.next_frame(&mut buf).map_err(|e| bad(&e.to_string()))?;
        let heights = buf[..frame.buffer_size()].chunks_exact(channels).map(|p| decode(p[0], p[1], p[2])).collect::<Vec<_>>();
        if heights.len() != SIZE * SIZE {
            return Err(bad("short image"));
        }
        Ok(ElevTile { heights })
    }

    pub fn at(&self, px: usize, py: usize) -> f32 {
        self.heights[py * SIZE + px]
    }

    /// This tile, an ancestor, as a tile of `to`, one of its descendants:
    /// the part of it that `to` covers, spread over 256 pixels. Bilinear, so a
    /// tile still loading shows its parent's heights, smoothly.
    pub fn stand_in(&self, from: TileId, to: TileId) -> ElevTile {
        debug_assert!(to.z >= from.z);
        let shift = (to.z - from.z) as u32;
        let scale = (1u64 << shift) as f64;
        let (ox, oy) = (to.x as f64 - (from.x as f64) * scale, to.y as f64 - (from.y as f64) * scale);
        let mut heights = Vec::with_capacity(SIZE * SIZE);
        for j in 0..SIZE {
            let sy = (oy + (j as f64 + 0.5) / SIZE as f64) / scale * SIZE as f64 - 0.5;
            for i in 0..SIZE {
                let sx = (ox + (i as f64 + 0.5) / SIZE as f64) / scale * SIZE as f64 - 0.5;
                heights.push(bilinear(sx, sy, |x, y| Some(self.at(x, y))).unwrap_or(f32::NAN));
            }
        }
        ElevTile { heights }
    }
}

/// A Terrarium pixel's height in metres.
pub fn decode(r: u8, g: u8, b: u8) -> f32 {
    (r as f32) * 256.0 + g as f32 + (b as f32) / 256.0 - 32768.0
}

/// Bilinear sample at fractional pixel position `(x, y)` (pixel centres on
/// whole numbers), given the pixel at a whole position. `None` when a corner
/// has no data. Positions outside the pixels `0..SIZE` use the edge pixel.
fn bilinear(x: f64, y: f64, get: impl Fn(usize, usize) -> Option<f32>) -> Option<f32> {
    let last = (SIZE - 1) as f64;
    let (x, y) = (x.clamp(0.0, last), y.clamp(0.0, last));
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = ((x - x0) as f32, (y - y0) as f32);
    let (x0, y0) = (x0 as usize, y0 as usize);
    let (x1, y1) = ((x0 + 1).min(SIZE - 1), (y0 + 1).min(SIZE - 1));
    let (a, b, c, d) = (get(x0, y0)?, get(x1, y0)?, get(x0, y1)?, get(x1, y1)?);
    if [a, b, c, d].iter().any(|v| v.is_nan()) {
        return None;
    }
    Some((a * (1.0 - fx) + b * fx) * (1.0 - fy) + (c * (1.0 - fx) + d * fx) * fy)
}

// ---- heights across tiles ---------------------------------------------------------

/// Tiles of one zoom side by side, to read heights across their seams.
#[derive(Clone, Default)]
pub struct Mosaic {
    pub z: u8,
    tiles: std::collections::HashMap<TileId, Arc<ElevTile>>,
}

impl Mosaic {
    pub fn new(z: u8) -> Mosaic {
        Mosaic { z, tiles: Default::default() }
    }

    pub fn insert(&mut self, id: TileId, tile: Arc<ElevTile>) {
        debug_assert_eq!(id.z, self.z);
        self.tiles.insert(id, tile);
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// The pixel at a position of the whole world's pixel grid at this zoom.
    fn pixel(&self, gx: i64, gy: i64) -> Option<f32> {
        let n = (SIZE as i64) << self.z;
        if gy < 0 || gy >= n {
            return None;
        }
        let gx = gx.rem_euclid(n);
        let s = SIZE as i64;
        let id = TileId { z: self.z, x: (gx / s) as u32, y: (gy / s) as u32 };
        self.tiles.get(&id).map(|t| t.at((gx % s) as usize, (gy % s) as usize))
    }

    /// The height at a position, in metres, bilinear across tile edges. `None`
    /// where a tile is missing or has no data.
    pub fn height(&self, lat: f64, lon: f64) -> Option<f32> {
        let ((x0, fx), (y0, fy)) = (self.split(self.grid_x(lon)), self.split(self.grid_y(lat)));
        let (a, b, c, d) = (self.pixel(x0, y0)?, self.pixel(x0 + 1, y0)?, self.pixel(x0, y0 + 1)?, self.pixel(x0 + 1, y0 + 1)?);
        mix4(a, b, c, d, fx, fy)
    }

    /// `height` at every crossing of `lats` (rows) and `lons` (columns), row by row, NaN where it is
    /// `None`. Each row's and column's place in the grid is worked out once, and the tile looked up only
    /// when a sample leaves the one before's, not four times a sample.
    pub fn heights(&self, lats: &[f64], lons: &[f64]) -> Vec<f32> {
        let cols: Vec<(i64, f32)> = lons.iter().map(|lon| self.split(self.grid_x(*lon))).collect();
        let mut out = Vec::with_capacity(lats.len() * lons.len());
        let mut tiles = TileReader { mosaic: self, last: [None, None] };
        for lat in lats {
            let (y0, fy) = self.split(self.grid_y(*lat));
            out.extend(cols.iter().map(|&(x0, fx)| {
                let h = (|| mix4(tiles.pixel(x0, y0)?, tiles.pixel(x0 + 1, y0)?, tiles.pixel(x0, y0 + 1)?, tiles.pixel(x0 + 1, y0 + 1)?, fx, fy))();
                h.unwrap_or(f32::NAN)
            }));
        }
        out
    }

    fn grid_x(&self, lon: f64) -> f64 {
        let n = (SIZE as f64) * (1u64 << self.z) as f64;
        (lon + 180.0) / 360.0 * n - 0.5
    }

    fn grid_y(&self, lat: f64) -> f64 {
        let n = (SIZE as f64) * (1u64 << self.z) as f64;
        let lat = lat.clamp(-85.05112878, 85.05112878).to_radians();
        (1.0 - lat.tan().asinh() / std::f64::consts::PI) / 2.0 * n - 0.5
    }

    /// A grid coordinate as its whole pixel and the fraction past it.
    fn split(&self, g: f64) -> (i64, f32) {
        let g0 = g.floor();
        (g0 as i64, (g - g0) as f32)
    }
}

fn mix4(a: f32, b: f32, c: f32, d: f32, fx: f32, fy: f32) -> Option<f32> {
    if [a, b, c, d].iter().any(|v| v.is_nan()) {
        return None;
    }
    Some((a * (1.0 - fx) + b * fx) * (1.0 - fy) + (c * (1.0 - fx) + d * fx) * fy)
}

/// Reads a mosaic's pixels, remembering the last two tiles it found (a sample's four pixels are nearly
/// always in one, at most four at a corner).
struct TileReader<'a> {
    mosaic: &'a Mosaic,
    last: [Option<(TileId, Option<&'a ElevTile>)>; 2],
}

impl<'a> TileReader<'a> {
    fn pixel(&mut self, gx: i64, gy: i64) -> Option<f32> {
        let z = self.mosaic.z;
        let n = (SIZE as i64) << z;
        if gy < 0 || gy >= n {
            return None;
        }
        let gx = gx.rem_euclid(n);
        let s = SIZE as i64;
        let id = TileId { z, x: (gx / s) as u32, y: (gy / s) as u32 };
        let tile = match self.last.iter().flatten().find(|(t, _)| *t == id) {
            Some((_, tile)) => *tile,
            None => {
                let tile = self.mosaic.tiles.get(&id).map(|t| t.as_ref());
                self.last = [Some((id, tile)), self.last[0]];
                tile
            }
        };
        tile.map(|t| t.at((gx % s) as usize, (gy % s) as usize))
    }
}

// ---- disk cache and download -----------------------------------------------------

fn file(dir: &Path, id: &TileId) -> PathBuf {
    dir.join(id.z.to_string()).join(id.x.to_string()).join(format!("{}.png", id.y))
}

fn age(path: &Path) -> Option<Duration> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok())
}

/// The cached bytes of a tile and whether they are still fresh. A tile the
/// service does not have is cached as an empty file.
pub fn read_cache(dir: &Path, id: &TileId) -> Option<(Vec<u8>, bool)> {
    let path = file(dir, id);
    let bytes = std::fs::read(&path).ok()?;
    Some((bytes, age(&path).is_some_and(|a| a < CACHE_TTL)))
}

fn write_cache(dir: &Path, id: &TileId, bytes: &[u8]) -> std::io::Result<()> {
    let path = file(dir, id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("part");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(tmp, path)
}

fn forget(dir: &Path, id: &TileId) {
    let _ = std::fs::remove_file(file(dir, id));
}

fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x89, b'P', b'N', b'G'])
}

/// Downloads a tile and caches it. Blocking; call from a worker thread.
fn fetch(dir: &Path, id: &TileId) -> Result<Vec<u8>, TileError> {
    let net = TileError::from_net;
    let mut res = vtile::agent().get(format!("{URL}/{}/{}/{}.png", id.z, id.x, id.y)).call().map_err(net)?;
    let bytes = match res.status().as_u16() {
        200 => res.body_mut().with_config().limit(MAX_BYTES).read_to_vec().map_err(net)?,
        404 => Vec::new(),
        s => return Err(TileError::Network(format!("terrain server answered {s}"))),
    };
    if !bytes.is_empty() && !is_png(&bytes) {
        return Err(TileError::Format("terrain server did not answer with an image".into()));
    }
    // A cache that can't be written still serves this session.
    let _ = write_cache(dir, id, &bytes);
    Ok(bytes)
}

/// A tile from the cache, else from the service when `download` is on. `None`
/// is a tile the service does not have. A fresh cache hit is used as it is, a
/// stale one when the download fails.
pub fn load(dir: &Path, id: &TileId, download: bool) -> Result<Option<Arc<ElevTile>>, TileError> {
    let cached = read_cache(dir, id);
    let bytes = match cached {
        Some((b, true)) => b,
        Some((b, false)) if !download => b,
        _ if !download => return Err(TileError::Offline),
        stale => match fetch(dir, id) {
            Ok(b) => b,
            Err(e) => match stale {
                Some((b, _)) => b,
                None => return Err(e),
            },
        },
    };
    if bytes.is_empty() {
        return Ok(None);
    }
    match ElevTile::from_png(&bytes) {
        Ok(t) => Ok(Some(Arc::new(t))),
        Err(e) => {
            // A damaged file is fetched again next time.
            forget(dir, id);
            Err(e)
        }
    }
}

/// Deletes cached tiles not written for `age`, and the folders left empty.
pub fn prune(dir: &Path, age: Duration) {
    vtile::prune(dir, age);
}

/// Tiles of the cache the layer holds, for the app's size and age listing.
pub type Cache = TileCache<Arc<ElevTile>>;

#[cfg(any(test, feature = "testutil"))]
pub mod testutil {
    use super::*;

    /// A Terrarium PNG whose pixel `(x, y)` has the height `f(x, y)`.
    pub fn png(f: impl Fn(usize, usize) -> f32) -> Vec<u8> {
        let mut raw = Vec::with_capacity(SIZE * SIZE * 3);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let v = ((f(x, y) + 32768.0) * 256.0).round() as u32;
                raw.extend([(v >> 16) as u8, (v >> 8) as u8, v as u8]);
            }
        }
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, SIZE as u32, SIZE as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&raw).unwrap();
        out
    }

    pub fn tile(f: impl Fn(usize, usize) -> f32) -> Arc<ElevTile> {
        Arc::new(ElevTile::from_png(&png(f)).unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;

    #[test]
    fn terrarium_formula() {
        assert_eq!(decode(128, 0, 0), 0.0);
        assert_eq!(decode(128, 100, 0), 100.0);
        assert_eq!(decode(127, 255, 128), -0.5);
        assert!((decode(130, 1, 64) - 513.25).abs() < 1e-4);
    }

    #[test]
    fn png_round_trip() {
        let t = ElevTile::from_png(&png(|x, y| x as f32 + 2.0 * y as f32 - 40.0)).unwrap();
        assert_eq!(t.at(0, 0), -40.0);
        assert_eq!(t.at(10, 20), 10.0);
        assert_eq!(t.at(255, 255), 725.0);
    }

    #[test]
    fn rejects_other_images() {
        assert!(ElevTile::from_png(b"not a png").is_err());
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, 4, 4);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&[0; 48]).unwrap();
        assert!(ElevTile::from_png(&out).is_err());
    }

    #[test]
    fn mosaic_reads_across_a_seam() {
        // One slope over two tiles side by side: height = global x.
        let z = 8;
        let mut m = Mosaic::new(z);
        m.insert(TileId { z, x: 100, y: 100 }, tile(|x, _| x as f32));
        m.insert(TileId { z, x: 101, y: 100 }, tile(|x, _| (x + SIZE) as f32));
        let n = (SIZE as f64) * (1u64 << z) as f64;
        // The seam is at global pixel x = 101 * 256; pixel centres sit half a pixel in.
        let lon_of = |gx: f64| (gx + 0.5) / n * 360.0 - 180.0;
        let lat = TileId { z, x: 100, y: 100 }.bounds();
        let lat = (lat.min_lat + lat.max_lat) / 2.0;
        let h = |gx: f64| m.height(lat, lon_of(gx)).unwrap();
        assert!((h(100.0 * 256.0 + 255.0) - 255.0).abs() < 1e-3);
        assert!((h(100.0 * 256.0 + 255.5) - 255.5).abs() < 1e-3);
        assert!((h(101.0 * 256.0) - 256.0).abs() < 1e-3);
        // Beyond the two tiles there is nothing.
        assert!(m.height(lat, lon_of(102.0 * 256.0 + 5.0)).is_none());
    }

    #[test]
    fn a_lattice_reads_what_each_point_reads() {
        // Four tiles round a corner, and a lattice over the corner and past the tiles.
        let z = 8;
        let mut m = Mosaic::new(z);
        for (dx, dy) in [(0, 0), (1, 0), (0, 1)] {
            m.insert(TileId { z, x: 100 + dx, y: 100 + dy }, tile(move |x, y| (x * 3 + y * 7 + (dx * 1000 + dy * 2000) as usize) as f32));
        }
        let corner = TileId { z, x: 101, y: 101 }.bounds();
        let lats: Vec<f64> = (0..40).map(|i| corner.max_lat + 0.4 - i as f64 * 0.02).collect();
        let lons: Vec<f64> = (0..50).map(|i| corner.min_lon - 0.5 + i as f64 * 0.021).collect();
        let all = m.heights(&lats, &lons);
        for (r, lat) in lats.iter().enumerate() {
            for (c, lon) in lons.iter().enumerate() {
                let (one, many) = (m.height(*lat, *lon), all[r * lons.len() + c]);
                assert!(one.map_or(many.is_nan(), |h| h == many), "{lat} {lon}: {one:?} {many}");
            }
        }
        assert!(all.iter().any(|h| h.is_nan()) && all.iter().any(|h| !h.is_nan()));
    }

    #[test]
    fn only_a_missing_tile_has_no_height() {
        let mut m = Mosaic::new(5);
        m.insert(TileId { z: 5, x: 3, y: 3 }, tile(|x, _| if x < 100 { 50.0 } else { -32768.0 }));
        // -32768 decodes to a real (very deep) height, not NaN: only a missing tile has no height.
        let b = TileId { z: 5, x: 3, y: 3 }.bounds();
        assert!(m.height((b.min_lat + b.max_lat) / 2.0, (b.min_lon + b.max_lon) / 2.0).is_some());
        assert!(m.height(0.0, 0.0).is_none());
    }

    #[test]
    fn a_child_stands_in_for_nothing_but_its_parent_area() {
        let parent = TileId { z: 7, x: 10, y: 20 };
        let t = ElevTile::from_png(&png(|x, _| x as f32)).unwrap();
        // The west child covers the parent's west half: its heights run 0 to ~128.
        let west = t.stand_in(parent, TileId { z: 8, x: 20, y: 40 });
        let east = t.stand_in(parent, TileId { z: 8, x: 21, y: 40 });
        assert!(west.at(0, 0) < 1.0 && west.at(255, 0) > 126.0 && west.at(255, 0) < 129.0);
        assert!(east.at(0, 0) > 127.0 && east.at(0, 0) < 130.0 && east.at(255, 0) > 254.0);
        // The same tile stands in for itself unchanged.
        let same = t.stand_in(parent, parent);
        assert!((same.at(77, 5) - 77.0).abs() < 1e-3);
    }

    #[test]
    fn cache_serves_offline_and_remembers_missing_tiles() {
        let dir = tempfile::tempdir().unwrap();
        let id = TileId { z: 9, x: 1, y: 2 };
        assert_eq!(load(dir.path(), &id, false), Err(TileError::Offline));
        write_cache(dir.path(), &id, &png(|_, _| 12.0)).unwrap();
        let t = load(dir.path(), &id, false).unwrap().unwrap();
        assert_eq!(t.at(3, 3), 12.0);
        let none = TileId { z: 9, x: 1, y: 3 };
        write_cache(dir.path(), &none, &[]).unwrap();
        assert_eq!(load(dir.path(), &none, false), Ok(None));
        // A damaged file is deleted so it is fetched again.
        let bad = TileId { z: 9, x: 1, y: 4 };
        write_cache(dir.path(), &bad, b"junk").unwrap();
        assert!(load(dir.path(), &bad, false).is_err());
        assert!(read_cache(dir.path(), &bad).is_none());
    }

    #[test]
    fn zoom_is_clamped() {
        assert_eq!(zoom_for(3.2), MIN_ZOOM);
        assert_eq!(zoom_for(9.99), 9);
        assert_eq!(zoom_for(17.0), MAX_ZOOM);
    }

    #[test]
    #[ignore = "needs the network"]
    fn live_tile_decodes() {
        let dir = tempfile::tempdir().unwrap();
        // Capo Malfatano, Sardinia.
        let id = TileId { z: 12, x: 2148, y: 1566 };
        let t = load(dir.path(), &id, true).unwrap().unwrap();
        let max = t.heights.iter().cloned().fold(f32::MIN, f32::max);
        assert!(max > 100.0, "no hills in {max}");
    }
}
