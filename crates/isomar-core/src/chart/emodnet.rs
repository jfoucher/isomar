//! EMODnet Bathymetry, fetched in half-degree tiles from its WCS and kept on
//! disk for good.
//!
//! The service serves one depth model for European waters and the North
//! Atlantic as a GeoTIFF of metres (negative below the datum) at about 115 m.
//! A tile is 0.5 degrees square, 480 by 480 samples plus a few more on each
//! side so neighbours overlap, about 0.7 MB compressed.
//! A tile the service has nothing for is kept as an empty file so it is not
//! asked for again. The data is for planning, not for navigation.

use super::grid::Grid;
use super::vtile::TileError;
use crate::vector::Bounds;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

const WCS: &str = "https://ows.emodnet-bathymetry.eu/wcs";

/// Where the model has data, from its coverage description.
pub const COVERAGE: Bounds = Bounds { min_lon: -70.5, min_lat: 11.0, max_lon: 43.0, max_lat: 90.0 };

/// Tiles per degree, in each direction.
const PER_DEGREE: f64 = 2.0;

/// Asked for past each edge, in degrees (about three samples), so neighbouring
/// tiles overlap and no strip of water is left unshaded at the seam.
const MARGIN: f64 = 0.003;

/// Largest answer taken: a tile is under 1 MB, so this is generous.
const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// One half-degree tile, by the whole number of half degrees to its
/// south-west corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    pub iy: i32,
    pub ix: i32,
}

impl TileKey {
    pub fn bounds(&self) -> Bounds {
        let (lat, lon) = (self.iy as f64 / PER_DEGREE, self.ix as f64 / PER_DEGREE);
        Bounds { min_lon: lon, min_lat: lat, max_lon: lon + 1.0 / PER_DEGREE, max_lat: lat + 1.0 / PER_DEGREE }
    }

    pub fn file(&self, dir: &Path) -> PathBuf {
        dir.join(format!("{}_{}.tif", self.iy, self.ix))
    }

    fn url(&self) -> String {
        let b = self.bounds().expanded(MARGIN);
        format!(
            "{WCS}?service=WCS&version=2.0.1&request=GetCoverage&coverageId=emodnet__mean&format=image/tiff\
             &geotiff:compression=Deflate&subset=Lat({:.4},{:.4})&subset=Long({:.4},{:.4})",
            b.min_lat, b.max_lat, b.min_lon, b.max_lon
        )
    }
}

/// The tiles under `view` that the service covers, nearest the middle of the
/// view first, at most `max` of them.
pub fn tiles_for(view: &Bounds, max: usize) -> Vec<TileKey> {
    let (lo_x, hi_x) = (view.min_lon.max(COVERAGE.min_lon), view.max_lon.min(COVERAGE.max_lon));
    let (lo_y, hi_y) = (view.min_lat.max(COVERAGE.min_lat), view.max_lat.min(COVERAGE.max_lat));
    if lo_x >= hi_x || lo_y >= hi_y {
        return Vec::new();
    }
    let mut keys = Vec::new();
    // A tile owns its south-west edge, so a view ending on an edge does not need the next tile.
    let range = |lo: f64, hi: f64| {
        let first = (lo * PER_DEGREE).floor() as i32;
        first..=(((hi * PER_DEGREE).ceil() as i32) - 1).max(first)
    };
    for iy in range(lo_y, hi_y) {
        for ix in range(lo_x, hi_x) {
            keys.push(TileKey { iy, ix });
        }
    }
    let (cx, cy) = ((view.min_lon + view.max_lon) / 2.0, (view.min_lat + view.max_lat) / 2.0);
    let dist = |k: &TileKey| {
        let b = k.bounds();
        ((b.min_lon + b.max_lon) / 2.0 - cx).powi(2) + ((b.min_lat + b.max_lat) / 2.0 - cy).powi(2)
    };
    keys.sort_by(|a, b| dist(a).total_cmp(&dist(b)));
    keys.truncate(max);
    keys
}

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            .user_agent("isomar/0.1 (open-source chart plotter)")
            .http_status_as_error(false)
            .build()
            .into()
    })
}

/// Whether `bytes` start like a TIFF, so an XML error page is not kept as data.
fn is_tiff(bytes: &[u8]) -> bool {
    bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*")
}

/// Keeps `bytes` as the tile's file, atomically.
fn store(dir: &Path, key: &TileKey, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = key.file(dir);
    let tmp = path.with_extension("part");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(tmp, path)
}

/// Downloads a tile and keeps it in `dir`. Blocking. `Ok(false)` when the
/// service has no data there (also kept, as an empty file).
fn fetch(dir: &Path, key: &TileKey) -> Result<bool, TileError> {
    let net = TileError::from_net;
    let mut res = agent().get(key.url()).call().map_err(net)?;
    match res.status().as_u16() {
        200 => {
            let bytes = res.body_mut().with_config().limit(MAX_BYTES).read_to_vec().map_err(net)?;
            if !is_tiff(&bytes) {
                return Err(TileError::Format("the service did not answer with a GeoTIFF".into()));
            }
            store(dir, key, &bytes).map_err(|e| TileError::Format(e.to_string()))?;
            Ok(true)
        }
        404 => {
            store(dir, key, b"").map_err(|e| TileError::Format(e.to_string()))?;
            Ok(false)
        }
        s => Err(TileError::Network(format!("the depth service answered {s}"))),
    }
}

/// Reads a tile from `dir`, downloading it first when it is missing and
/// `download` allows. `Ok(None)` is a tile with no data. A damaged file is
/// deleted so the next try fetches it again.
pub fn load(dir: &Path, key: &TileKey, download: bool) -> Result<Option<Grid>, TileError> {
    let path = key.file(dir);
    let kept = match std::fs::metadata(&path) {
        Ok(m) => Some(m.len()),
        Err(_) => None,
    };
    match kept {
        Some(0) => return Ok(None),
        Some(_) => {}
        None if download => {
            if !fetch(dir, key)? {
                return Ok(None);
            }
        }
        None => return Err(TileError::Offline),
    }
    Grid::from_geotiff(&path).map(Some).map_err(|e| {
        let _ = std::fs::remove_file(&path);
        TileError::Format(e.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(min_lon: f64, min_lat: f64, max_lon: f64, max_lat: f64) -> Bounds {
        Bounds { min_lon, min_lat, max_lon, max_lat }
    }

    #[test]
    fn a_tile_covers_half_a_degree() {
        let k = TileKey { iy: 95, ix: -6 };
        assert_eq!(k.bounds(), view(-3.0, 47.5, -2.5, 48.0));
        assert!(k.url().contains("subset=Lat(47.4970,48.0030)&subset=Long(-3.0030,-2.4970)"), "{}", k.url());
        assert!(k.file(Path::new("/d")).ends_with("95_-6.tif"));
    }

    #[test]
    fn the_view_is_covered_nearest_the_middle_first() {
        let keys = tiles_for(&view(-3.2, 47.4, -2.2, 47.9), 100);
        // Longitudes -3.5..-2.0 in three tiles, latitudes 47.0..47.5 and 47.5..48.0 in two.
        assert_eq!(keys.len(), 3 * 2);
        for k in &keys {
            assert!(k.bounds().intersects(&view(-3.2, 47.4, -2.2, 47.9)));
        }
        let first = keys[0].bounds();
        assert!(first.contains([-2.7, 47.7]), "{first:?}");
        assert_eq!(tiles_for(&view(-3.2, 47.4, -2.2, 47.9), 3).len(), 3);
    }

    #[test]
    fn tiles_outside_the_coverage_are_not_asked_for() {
        // Ecuador, and the far side of the world.
        assert!(tiles_for(&view(-81.1, -2.3, -80.9, -2.1), 10).is_empty());
        assert!(tiles_for(&view(170.0, 10.0, 171.0, 11.0), 10).is_empty());
        // A view straddling the edge keeps only the covered part.
        let keys = tiles_for(&view(42.0, 50.0, 45.0, 50.5), 100);
        assert!(!keys.is_empty() && keys.iter().all(|k| k.bounds().min_lon < 43.0));
        // A view that ends exactly on a tile edge does not need the tile beyond it.
        assert_eq!(tiles_for(&view(-3.0, 47.0, -2.5, 47.5), 10), vec![TileKey { iy: 94, ix: -6 }]);
    }

    #[test]
    fn only_tiffs_are_kept() {
        assert!(is_tiff(b"MM\0*rest") && is_tiff(b"II*\0rest"));
        assert!(!is_tiff(b"<?xml version"));
        assert!(!is_tiff(b""));
    }

    #[test]
    fn a_kept_tile_loads_offline_and_an_empty_one_means_no_data() {
        let d = tempfile::tempdir().unwrap();
        let none = TileKey { iy: 1, ix: 1 };
        assert_eq!(load(d.path(), &none, false).err(), Some(TileError::Offline));
        store(d.path(), &none, b"").unwrap();
        assert!(matches!(load(d.path(), &none, false), Ok(None)));
    }

    #[test]
    fn a_damaged_tile_is_forgotten_so_it_is_fetched_again() {
        let d = tempfile::tempdir().unwrap();
        let k = TileKey { iy: 95, ix: -6 };
        store(d.path(), &k, b"<html>not a tiff</html>").unwrap();
        assert!(matches!(load(d.path(), &k, false), Err(TileError::Format(_))));
        assert!(!k.file(d.path()).exists());
        assert_eq!(load(d.path(), &k, false).err(), Some(TileError::Offline));
    }

    /// The live service: one tile of the Gulf of Morbihan, downloaded, kept and read back.
    #[test]
    #[ignore]
    fn live_a_real_tile_downloads_and_reads() {
        let d = tempfile::tempdir().unwrap();
        let k = TileKey { iy: 95, ix: -6 };
        let g = load(d.path(), &k, true).unwrap().expect("data at Quiberon");
        assert!((480..490).contains(&g.ncols) && (480..490).contains(&g.nrows), "{}x{}", g.ncols, g.nrows);
        assert!(g.data.iter().any(|v| *v < -10.0), "deep water in the tile");
        // The second read comes from disk.
        assert!(load(d.path(), &k, false).unwrap().is_some());
        // Open ocean off the covered area is just not asked for, and a covered but empty tile is kept as such.
        assert!(tiles_for(&view(-100.0, 0.0, -99.0, 1.0), 4).is_empty());
    }
}
