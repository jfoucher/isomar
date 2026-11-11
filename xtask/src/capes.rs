//! `cargo xtask capes <dir> [--coves] [--out <file>]`: every named cape in OpenStreetMap, as
//! `scripts/fetch-osm-capes.sh` leaves it in `<dir>`, packed for the app (`isomar_core::chart::capes`): the local
//! name, where it is in units to about a metre, and its rank. The capes and headlands of `capes.*.json` (rank 1
//! with a Wikidata entry, else 2), and the localities, points and cliffs of `near.*.json` whose name is a cape's:
//! OSM maps some of the great capes so (Beachy Head is a cliff, Cap de Sant Sebastià a locality), and the points
//! of a coast that nobody mapped as capes ("Punta de sa Cresta"), rank 3 without a Wikidata entry; a cliff needs
//! one. With `--coves`, the bays and coves of `coves.*.json` instead, into `coves.json.gz`. The output is ODbL,
//! like its source.

use isomar_core::chart::capes::{SCALE, is_cape_name};
use isomar_core::geo::{LatLon, project};
use serde_json::{Value, json};
use std::io::Write;
use std::path::Path;

const MAX_BYTES: usize = 1_500_000;
/// The same name this close (units, about 100 m) is the same cape mapped twice, as a node and a way.
const SAME_CAPE: f64 = 100.0 / 40_075_016.0 * 256.0;
/// A cove's node and its water's centre can be farther apart (about 400 m).
const SAME_COVE: f64 = 400.0 / 40_075_016.0 * 256.0;

/// The first of the names a cape carries in several languages (`A / B`, `A;B`): the local one.
fn local_name(name: &str) -> &str {
    name.split(" / ").next().unwrap_or(name).split(';').next().unwrap_or(name).trim()
}

/// Where Overpass puts an element: a node's own position, the centre of a way or relation.
fn position(e: &Value) -> Option<LatLon> {
    let at = if e["lat"].is_number() { e } else { &e["center"] };
    Some(LatLon::new(at["lat"].as_f64()?, at["lon"].as_f64()?))
}

pub fn run(args: &[String]) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let dir = Path::new(args.first().expect("usage: cargo xtask capes <dir> [--coves] [--out <file>]"));
    let coves = args.iter().any(|a| a == "--coves");
    let out = match args.iter().position(|a| a == "--out") {
        Some(i) => Path::new(args.get(i + 1).expect("--out needs a path")).to_path_buf(),
        None => root.join(if coves { "data/labels/coves.json.gz" } else { "data/labels/capes.json.gz" }),
    };
    // Every band of a kind (`capes.0.json` to `capes.7.json`), or one file for the world (`capes.json`).
    let elements = |kind: &str| -> Vec<Value> {
        let mut paths: Vec<_> = std::fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&format!("{kind}.")) && n.ends_with(".json")))
            .collect();
        paths.sort();
        if paths.is_empty() {
            println!("warning: no {kind}*.json in {} (run scripts/fetch-osm-capes.sh {kind})", dir.display());
        }
        let mut all = Vec::new();
        for path in paths {
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let mut json: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            all.append(json["elements"].as_array_mut().expect("elements"));
        }
        all
    };
    let kind = if coves { "coves" } else { "capes" };
    let capes_read = elements(kind);
    assert!(!capes_read.is_empty(), "no {kind} in {}", dir.display());
    // A cliff named as a cape is one only when it is known; a locality or a point is a cape its name says it is.
    let near_kept: Vec<Value> = if coves {
        Vec::new()
    } else {
        let near = elements("near");
        let near_count = near.len();
        let kept: Vec<Value> = near
            .into_iter()
            .filter(|e| e["tags"]["name"].as_str().is_some_and(|n| is_cape_name(local_name(n))))
            .filter(|e| e["tags"]["natural"] != "cliff" || e["tags"]["wikidata"].is_string())
            .collect();
        println!("{} cliffs, localities and points named as capes, {} kept", near_count, kept.len());
        kept
    };
    // (cell, x, y, name, rank)
    let mut capes: Vec<(u32, i64, i64, String, u8)> = Vec::new();
    for (e, near) in capes_read.iter().map(|e| (e, false)).chain(near_kept.iter().map(|e| (e, true))) {
        let tags = &e["tags"];
        let Some(name) = tags["name"].as_str().map(local_name).filter(|n| !n.is_empty()) else { continue };
        let Some(LatLon { lat, lon }) = position(e) else { continue };
        if lat.abs() > 85.0 {
            continue;
        }
        let (x, y) = project(&LatLon::new(lat, lon), 0.0);
        let cell = y.floor().clamp(0.0, 255.0) as u32 * 256 + x.floor().clamp(0.0, 255.0) as u32;
        let rank = match (tags["wikidata"].is_string(), near) {
            (true, _) => 1,
            (false, false) => 2,
            (false, true) => 3,
        };
        capes.push((cell, (x * SCALE).round() as i64, (y * SCALE).round() as i64, name.to_string(), rank));
    }
    let read = capes.len();
    capes.sort_by_key(|c| (c.0, c.1, c.2));
    // The same cape as a node and as a way: keep the first, with the better rank.
    let near = if coves { SAME_COVE } else { SAME_CAPE } * SCALE;
    let mut kept: Vec<(u32, i64, i64, String, u8)> = Vec::with_capacity(capes.len());
    for c in capes {
        let twin = kept.iter_mut().rev().take(64).find(|k| k.0 == c.0 && k.3 == c.3 && ((k.1 - c.1) as f64).hypot((k.2 - c.2) as f64) < near);
        match twin {
            Some(k) => k.4 = k.4.min(c.4),
            None => kept.push(c),
        }
    }
    let (mut px, mut py) = (0i64, 0i64);
    let (mut dx, mut dy) = (Vec::new(), Vec::new());
    for c in &kept {
        dx.push(c.1 - px);
        dy.push(c.2 - py);
        (px, py) = (c.1, c.2);
    }
    let bundle = json!({
        "names": kept.iter().map(|c| c.3.clone()).collect::<Vec<_>>(),
        "dx": dx,
        "dy": dy,
        "rank": kept.iter().map(|c| c.4).collect::<Vec<_>>(),
    });
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    gz.write_all(serde_json::to_string(&bundle).unwrap().as_bytes()).unwrap();
    let bytes = gz.finish().unwrap();
    let well_known = kept.iter().filter(|c| c.4 == 1).count();
    println!("{} {kind} ({} read, {} with Wikidata), {} bytes", kept.len(), read, well_known, bytes.len());
    assert!(bytes.len() <= MAX_BYTES, "the bundle is {} bytes, over {MAX_BYTES}", bytes.len());
    std::fs::write(&out, bytes).unwrap();
    println!("wrote {}", out.display());
}
