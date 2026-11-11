//! Land polygons in an indexed, multi-resolution SQLite store.
//!
//! The SHOM *limite terre-mer* shapefile has 10 million vertices in Lambert-93.
//! `CoastStore::build` converts it once to lon/lat, simplifies each polygon at
//! several tolerances, and indexes the results with an R-tree. Drawing then
//! reads only the polygons in view, at a detail that suits the zoom.

use crate::chart::shp::{Shape, ShpError, ShpReader};
use crate::proj::Lcc;
use crate::vector::{Bounds, Polygon, Ring, simplify_ring};
use rayon::prelude::*;
use rusqlite::{Connection, OpenFlags, params};
use std::path::Path;

/// Simplification tolerance per level of detail, in degrees; coarsest first.
/// The last level is the original geometry, only for zoom 14 and closer: the
/// SHOM coast has polygons of 850,000 points there.
pub const LOD_TOLERANCE_DEG: [f64; 6] = [0.02, 0.005, 0.0012, 0.0003, 0.00008, 0.0];

const SCHEMA_VERSION: &str = "2";

#[derive(Debug)]
pub enum CoastError {
    Shp(ShpError),
    Db(rusqlite::Error),
    Other(String),
}

impl From<ShpError> for CoastError {
    fn from(e: ShpError) -> Self {
        CoastError::Shp(e)
    }
}
impl From<rusqlite::Error> for CoastError {
    fn from(e: rusqlite::Error) -> Self {
        CoastError::Db(e)
    }
}
impl From<std::io::Error> for CoastError {
    fn from(e: std::io::Error) -> Self {
        CoastError::Other(e.to_string())
    }
}
impl std::fmt::Display for CoastError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoastError::Shp(e) => write!(f, "{e}"),
            CoastError::Db(e) => write!(f, "{e}"),
            CoastError::Other(s) => write!(f, "{s}"),
        }
    }
}
impl std::error::Error for CoastError {}

/// Which level of detail to read at `zoom` and `lat`: the coarsest whose
/// tolerance is at most about one pixel.
pub fn lod_for_view(zoom: f64, lat: f64) -> usize {
    let deg_per_px = 360.0 / (256.0 * 2f64.powf(zoom)) * lat.to_radians().cos().max(0.05);
    LOD_TOLERANCE_DEG
        .iter()
        .position(|t| *t <= deg_per_px * 1.2)
        .unwrap_or(LOD_TOLERANCE_DEG.len() - 1)
}

fn encode(rings: &[Ring]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(rings.len() as u32).to_le_bytes());
    for r in rings {
        out.extend_from_slice(&(r.len() as u32).to_le_bytes());
        for p in r {
            out.extend_from_slice(&(p[0] as f32).to_le_bytes());
            out.extend_from_slice(&(p[1] as f32).to_le_bytes());
        }
    }
    out
}

fn decode(b: &[u8]) -> Option<Polygon> {
    let u32_at = |o: usize| b.get(o..o + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap()));
    let f32_at = |o: usize| b.get(o..o + 4).map(|s| f32::from_le_bytes(s.try_into().unwrap()) as f64);
    let n = u32_at(0)? as usize;
    let mut o = 4;
    let mut rings = Vec::with_capacity(n);
    for _ in 0..n {
        let m = u32_at(o)? as usize;
        o += 4;
        let mut ring = Vec::with_capacity(m);
        for _ in 0..m {
            ring.push([f32_at(o)?, f32_at(o + 4)?]);
            o += 8;
        }
        rings.push(ring);
    }
    Some(Polygon { rings })
}

/// One polygon's rows for every level of detail: (lod, bounds, blob).
fn lods_of(poly: &Polygon) -> Vec<(usize, Bounds, Vec<u8>)> {
    let mut rows = Vec::new();
    for (lod, tol) in LOD_TOLERANCE_DEG.iter().enumerate() {
        let rings: Vec<Ring> = if *tol == 0.0 {
            poly.rings.clone()
        } else {
            poly.rings.iter().filter_map(|r| simplify_ring(r, *tol)).collect()
        };
        if rings.is_empty() {
            continue;
        }
        let mut b = Bounds::EMPTY;
        for r in &rings {
            b.union(&Bounds::of(r));
        }
        rows.push((lod, b, encode(&rings)));
    }
    rows
}

pub struct CoastStore {
    conn: Connection,
    bounds: Bounds,
}

impl CoastStore {
    pub fn open(path: &Path) -> Result<Self, CoastError> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let v: String = conn
            .query_row("SELECT value FROM meta WHERE key='schema'", [], |r| r.get(0))
            .map_err(|_| CoastError::Other("not a coast store".into()))?;
        if v != SCHEMA_VERSION {
            return Err(CoastError::Other(format!("unsupported coast store version {v}")));
        }
        // Stores built before bounds were recorded get them from the index.
        let recorded: Option<String> =
            conn.query_row("SELECT value FROM meta WHERE key='bounds'", [], |r| r.get(0)).ok();
        let parsed = recorded.and_then(|v| {
            let n: Vec<f64> = v.split(',').filter_map(|x| x.parse().ok()).collect();
            (n.len() == 4).then(|| Bounds { min_lon: n[0], min_lat: n[1], max_lon: n[2], max_lat: n[3] })
        });
        let bounds = match parsed {
            Some(b) => b,
            None => conn.query_row("SELECT min(minx), min(miny), max(maxx), max(maxy) FROM idx", [], |r| {
                Ok(Bounds { min_lon: r.get(0)?, min_lat: r.get(1)?, max_lon: r.get(2)?, max_lat: r.get(3)? })
            })?,
        };
        Ok(CoastStore { conn, bounds })
    }

    /// The area the store has land data for.
    pub fn bounds(&self) -> &Bounds {
        &self.bounds
    }

    /// Converts a Lambert-93 polygon shapefile into a store at `db`.
    /// `progress` receives 0..1. The store appears atomically when finished.
    pub fn build_lambert93(shp: &Path, db: &Path, progress: impl FnMut(f64)) -> Result<(), CoastError> {
        let lcc = Lcc::lambert93();
        Self::build_with(shp, db, move |x, y| lcc.inverse(x, y), progress)
    }

    /// Builds from a shapefile already in lon/lat.
    pub fn build_lonlat(shp: &Path, db: &Path, progress: impl FnMut(f64)) -> Result<(), CoastError> {
        Self::build_with(shp, db, |x, y| [x, y], progress)
    }

    fn build_with(
        shp: &Path,
        db: &Path,
        project: impl Fn(f64, f64) -> [f64; 2] + Sync,
        mut progress: impl FnMut(f64),
    ) -> Result<(), CoastError> {
        if let Some(dir) = db.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = db.with_extension("tmp");
        let _ = std::fs::remove_file(&tmp);
        let mut conn = Connection::open(&tmp)?;
        conn.execute_batch(
            "PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF;
             CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT);
             CREATE TABLE features(id INTEGER PRIMARY KEY, lod INTEGER NOT NULL, geom BLOB NOT NULL);
             CREATE VIRTUAL TABLE idx USING rtree(id, minx, maxx, miny, maxy);",
        )?;

        let mut reader = ShpReader::open(shp)?;
        let mut extent = Bounds::EMPTY;
        let mut batch: Vec<Polygon> = Vec::new();
        let mut done = false;
        while !done {
            batch.clear();
            while batch.len() < 4000 {
                match reader.next_shape()? {
                    Some(Shape::Polygon(parts)) => {
                        let rings = parts
                            .into_iter()
                            .map(|r| r.into_iter().map(|p| project(p[0], p[1])).collect())
                            .collect();
                        batch.push(Polygon { rings });
                    }
                    Some(_) => {}
                    None => {
                        done = true;
                        break;
                    }
                }
            }
            for poly in &batch {
                extent.union(&poly.bounds());
            }
            let rows: Vec<Vec<(usize, Bounds, Vec<u8>)>> = batch.par_iter().map(lods_of).collect();
            let tx = conn.transaction()?;
            {
                let mut ins_f = tx.prepare("INSERT INTO features(lod, geom) VALUES (?1, ?2)")?;
                let mut ins_i = tx.prepare("INSERT INTO idx VALUES (?1, ?2, ?3, ?4, ?5)")?;
                for polygon_rows in rows {
                    for (lod, b, blob) in polygon_rows {
                        ins_f.execute(params![lod as i64, blob])?;
                        let id = tx.last_insert_rowid();
                        ins_i.execute(params![id, b.min_lon, b.max_lon, b.min_lat, b.max_lat])?;
                    }
                }
            }
            tx.commit()?;
            progress(reader.progress());
        }
        conn.execute("INSERT INTO meta VALUES ('schema', ?1)", params![SCHEMA_VERSION])?;
        if !extent.is_empty() {
            let b = format!("{},{},{},{}", extent.min_lon, extent.min_lat, extent.max_lon, extent.max_lat);
            conn.execute("INSERT INTO meta VALUES ('bounds', ?1)", params![b])?;
        }
        conn.execute_batch("VACUUM")?;
        drop(conn);
        std::fs::rename(&tmp, db)?;
        Ok(())
    }

    /// The polygons at level `lod` that touch `bounds`.
    pub fn query(&self, bounds: &Bounds, lod: usize) -> Result<Vec<Polygon>, CoastError> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT f.geom FROM idx JOIN features f ON f.id = idx.id
             WHERE f.lod = ?1 AND idx.maxx >= ?2 AND idx.minx <= ?3 AND idx.maxy >= ?4 AND idx.miny <= ?5",
        )?;
        let rows = stmt.query_map(
            params![lod as i64, bounds.min_lon, bounds.max_lon, bounds.min_lat, bounds.max_lat],
            |r| r.get::<_, Vec<u8>>(0),
        )?;
        let mut out = Vec::new();
        for blob in rows {
            if let Some(p) = decode(&blob?) {
                out.push(p);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::shp::testutil::{file, polygon_body};

    fn square(x: f64, y: f64, s: f64) -> Ring {
        vec![[x, y], [x, y + s], [x + s, y + s], [x + s, y], [x, y]]
    }

    fn write_shp(dir: &Path, polys: &[Vec<Ring>]) -> std::path::PathBuf {
        let p = dir.join("land.shp");
        std::fs::write(&p, file(5, polys.iter().map(|r| polygon_body(r, 5)).collect())).unwrap();
        p
    }

    #[test]
    fn lod_gets_finer_when_zooming_in() {
        assert_eq!(lod_for_view(3.0, 48.0), 0);
        assert!(lod_for_view(9.0, 48.0) < LOD_TOLERANCE_DEG.len() - 1);
        assert!(lod_for_view(12.0, 48.0) < LOD_TOLERANCE_DEG.len() - 1, "zoom 12 is not full detail");
        assert_eq!(lod_for_view(17.0, 48.0), LOD_TOLERANCE_DEG.len() - 1);
        let mut last = 0;
        for z in 2..=18 {
            let l = lod_for_view(z as f64, 48.0);
            assert!(l >= last);
            last = l;
        }
    }

    #[test]
    fn a_store_knows_its_bounds() {
        let dir = tempfile::tempdir().unwrap();
        let shp = write_shp(dir.path(), &[vec![square(-5.0, 48.0, 2.0)], vec![square(10.0, 40.0, 1.0)]]);
        let db = dir.path().join("coast.sqlite");
        CoastStore::build_lonlat(&shp, &db, |_| {}).unwrap();
        let b = *CoastStore::open(&db).unwrap().bounds();
        assert_eq!((b.min_lon, b.min_lat, b.max_lon, b.max_lat), (-5.0, 40.0, 11.0, 50.0));
    }

    #[test]
    fn build_and_query_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let big = square(-5.0, 48.0, 1.0);
        let hole = square(-4.6, 48.4, 0.2);
        let islet = square(-3.0, 49.0, 0.0001);
        let shp = write_shp(dir.path(), &[vec![big.clone(), hole.clone()], vec![islet.clone()]]);
        let db = dir.path().join("coast.sqlite");
        let mut last = 0.0;
        CoastStore::build_lonlat(&shp, &db, |p| last = p).unwrap();
        assert!((last - 1.0).abs() < 1e-9);

        let store = CoastStore::open(&db).unwrap();
        let view = Bounds { min_lon: -6.0, min_lat: 47.0, max_lon: -2.0, max_lat: 50.0 };

        // Full detail has both polygons, with the hole and exact vertices.
        let full = store.query(&view, LOD_TOLERANCE_DEG.len() - 1).unwrap();
        assert_eq!(full.len(), 2);
        let p = full.iter().find(|p| p.rings.len() == 2).unwrap();
        assert!(p.contains([-4.9, 48.1]) && !p.contains([-4.5, 48.5]));

        // The islet is far below the coarsest tolerance and vanishes there.
        let coarse = store.query(&view, 0).unwrap();
        assert_eq!(coarse.len(), 1);

        // Only polygons in view come back.
        let far = Bounds { min_lon: 10.0, min_lat: 10.0, max_lon: 11.0, max_lat: 11.0 };
        assert!(store.query(&far, 4).unwrap().is_empty());
    }

    #[test]
    fn lambert_input_lands_in_france() {
        let dir = tempfile::tempdir().unwrap();
        // A 10 km square near Brest, in Lambert-93 metres.
        let [x, y] = Lcc::lambert93().forward(-4.5, 48.4);
        let ring = vec![[x, y], [x, y + 10_000.0], [x + 10_000.0, y + 10_000.0], [x + 10_000.0, y], [x, y]];
        let shp = write_shp(dir.path(), &[vec![ring]]);
        let db = dir.path().join("c.sqlite");
        CoastStore::build_lambert93(&shp, &db, |_| {}).unwrap();
        let store = CoastStore::open(&db).unwrap();
        let polys = store.query(&Bounds { min_lon: -5.0, min_lat: 48.0, max_lon: -4.0, max_lat: 49.0 }, 4).unwrap();
        assert_eq!(polys.len(), 1);
        let b = polys[0].bounds();
        // The Lambert grid is rotated by the meridian convergence (about 5
        // degrees here), so the lon/lat box is a little larger than the corner.
        assert!((b.min_lon + 4.5).abs() < 0.03 && (b.min_lat - 48.4).abs() < 0.03, "{b:?}");
        assert!(b.width() > 0.1 && b.height() > 0.08);
    }

    #[test]
    fn opening_garbage_fails_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.sqlite");
        std::fs::write(&p, b"not a database").unwrap();
        assert!(CoastStore::open(&p).is_err());
    }

    /// Run with `cargo test --release real_shom -- --ignored` after
    /// `scripts/fetch-testdata.sh shom-limite-terre-mer`.
    #[test]
    #[ignore]
    fn real_shom_coastline_builds() {
        let shp = Path::new("testdata/shom-limite-terre-mer/Limite_terre_mer_France_metropolitaine/SHAPE/Limite_terre-mer_France_metropolitaine_polygones.shp");
        if !shp.exists() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("coast.sqlite");
        let t = std::time::Instant::now();
        CoastStore::build_lambert93(shp, &db, |_| {}).unwrap();
        eprintln!("built in {:?}, {} bytes", t.elapsed(), std::fs::metadata(&db).unwrap().len());
        let store = CoastStore::open(&db).unwrap();
        // Brest area at full detail.
        let polys = store.query(&Bounds { min_lon: -4.7, min_lat: 48.3, max_lon: -4.3, max_lat: 48.5 }, 4).unwrap();
        assert!(!polys.is_empty());
        // Brest town itself is land, the middle of the roadstead is not.
        assert!(polys.iter().any(|p| p.contains([-4.486, 48.39])));
        assert!(!polys.iter().any(|p| p.contains([-4.45, 48.34])));
    }
}
