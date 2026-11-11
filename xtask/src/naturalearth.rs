//! `cargo xtask naturalearth <dir> [--out <file>]`: the names of seas, straits, island groups, ranges and
//! capes from Natural Earth (public domain), with each one's centreline worked out here, once, so the app
//! does no geometry for them. `<dir>` is `testdata/naturalearth` as `scripts/fetch-naturalearth.sh` fills it.

use isomar_core::geo::{LatLon, project};
use isomar_core::labels::coast::coast_at;
use isomar_core::labels::{Mask, spine, spine_in};
use isomar_core::vector::{Bounds, Pt, simplify_ring};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

/// In the order the app ranks them.
/// How much a strait's box is grown on each side, as a share of its size, for the water round it: its name runs
/// from one sea to the other, and its mouths are not where its water ends.
const STRAIT_ROOM: f64 = 0.5;

/// A strait's centreline from the sea on one side to the sea on the other (`labels::spine_in`): over a box grown
/// by `STRAIT_ROOM`, the room measured from the land alone, from the middle of one mouth to the other's. A strait
/// in pieces is the water in their hull (the Strait of Gibraltar is five); its biggest piece alone is a bay off
/// its side. `None` where the outline does not part two seas.
fn strait_spine(f: &Feature, b: [f64; 4], land: &Land, ice: &Land) -> Option<isomar_core::labels::Spine> {
    let (gx, gy) = ((b[2] - b[0]) * STRAIT_ROOM, (b[3] - b[1]) * STRAIT_ROOM);
    let grid = [b[0] - gx, b[1] - gy, b[2] + gx, b[3] + gy];
    let mut dry = Mask::over(grid, 256);
    dry.fill(&land.meeting(grid), true);
    dry.fill(&ice.meeting(grid), true);
    let mut room = dry.empty_like();
    room.fill(&[vec![[grid[0], grid[1]], [grid[2], grid[1]], [grid[2], grid[3]], [grid[0], grid[3]]]], true);
    room.minus(&dry);
    room.fill_small_holes(room.w * room.h / ISLET_SHARE);
    let mut strait = dry.empty_like();
    if f.rings.len() > 1 {
        strait.fill(&[convex_hull(f.rings.iter().flatten().copied().collect())], true);
    } else {
        strait.fill(&f.rings, true);
    }
    strait.minus(&dry);
    strait.fill_small_holes(strait.w * strait.h / ISLET_SHARE);
    spine_in(&strait.largest_component(), &room)
}

const KINDS: [&str; 11] = ["ocean", "country", "sea", "gulf", "strait", "island_group", "island", "range", "peninsula", "bay", "cape"];
const MAX_BYTES: usize = 1_500_000;
const LANGUAGES: [&str; 25] = [
    "fr", "es", "de", "it", "pt", "nl", "pl", "ru", "sv", "tr", "el", "ja", "zh", "zht", "ko", "ar", "he", "uk", "hu", "id", "vi", "fa", "hi", "bn", "ur",
];

struct Feature {
    class: String,
    props: Map<String, Value>,
    /// The rings of every polygon, in units.
    rings: Vec<Vec<Pt>>,
    /// A point feature's place, in units.
    point: Option<Pt>,
    /// Whether it goes all round the world.
    wraps: bool,
}

fn unit(lon: f64, lat: f64) -> Pt {
    let (x, y) = project(&LatLon::new(lat, lon), 0.0);
    [x, y]
}

/// Features of a GeoJSON file: property names lowercased, geometry in units. A feature that crosses the
/// antimeridian has its western longitudes moved on by 360 degrees, so it is in one piece (its units then
/// reach past 256); one that goes all round the world (the Arctic, the Southern Ocean) is left as it is.
fn read(path: &Path) -> Vec<Feature> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e} (run scripts/fetch-naturalearth.sh)", path.display()));
    let json: Value = serde_json::from_str(&text).expect("GeoJSON");
    let mut out = Vec::new();
    for f in json["features"].as_array().expect("features") {
        let props: Map<String, Value> = f["properties"].as_object().expect("properties").iter().map(|(k, v)| (k.to_lowercase(), v.clone())).collect();
        let class = props.get("featurecla").and_then(Value::as_str).unwrap_or("").trim().to_lowercase();
        let g = &f["geometry"];
        let ring = |coords: &Value| -> Vec<[f64; 2]> { coords.as_array().unwrap().iter().map(|c| [c[0].as_f64().unwrap(), c[1].as_f64().unwrap()]).collect() };
        let mut lonlat: Vec<Vec<[f64; 2]>> = Vec::new();
        let mut point = None;
        match g["type"].as_str() {
            Some("Polygon") => g["coordinates"].as_array().unwrap().iter().for_each(|r| lonlat.push(ring(r))),
            Some("MultiPolygon") => g["coordinates"].as_array().unwrap().iter().for_each(|p| p.as_array().unwrap().iter().for_each(|r| lonlat.push(ring(r)))),
            Some("Point") => point = Some(unit(g["coordinates"][0].as_f64().unwrap(), g["coordinates"][1].as_f64().unwrap())),
            _ => continue,
        }
        let span = |rings: &[Vec<[f64; 2]>]| rings.iter().flatten().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p[0]), hi.max(p[0])));
        let (lo, hi) = span(&lonlat);
        let crosses = lonlat.iter().any(|r| r.windows(2).any(|w| (w[1][0] - w[0][0]).abs() > 180.0));
        let mut wraps = false;
        if crosses || hi - lo > 180.0 {
            let moved: Vec<Vec<[f64; 2]>> = lonlat.iter().map(|r| r.iter().map(|p| [if p[0] < 0.0 { p[0] + 360.0 } else { p[0] }, p[1]]).collect()).collect();
            let (lo2, hi2) = span(&moved);
            if hi2 - lo2 <= 180.0 && !moved.iter().any(|r| r.windows(2).any(|w| (w[1][0] - w[0][0]).abs() > 180.0)) {
                lonlat = moved;
            } else {
                wraps = true;
            }
        }
        // Lon 180 projects to the right edge; past it, units go on beyond 256.
        let rings = lonlat.iter().map(|r| r.iter().map(|p| unit_beyond(p[0], p[1])).collect()).collect();
        out.push(Feature { class, props, rings, point, wraps });
    }
    out
}

/// `unit`, for longitudes past 180 too.
fn unit_beyond(lon: f64, lat: f64) -> Pt {
    if lon > 180.0 {
        let p = unit(lon - 360.0, lat);
        [p[0] + 256.0, p[1]]
    } else {
        unit(lon, lat)
    }
}

fn text(props: &Map<String, Value>, key: &str) -> Option<String> {
    let s = props.get(key)?.as_str()?;
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    (!s.is_empty()).then_some(s)
}

fn number(props: &Map<String, Value>, key: &str) -> Option<f64> {
    props.get(key)?.as_f64()
}

fn bounds_of(rings: &[Vec<Pt>]) -> [f64; 4] {
    let b = rings.iter().fold(Bounds::EMPTY, |mut b, r| {
        r.iter().for_each(|p| b.extend(*p));
        b
    });
    [b.min_lon, b.min_lat, b.max_lon, b.max_lat]
}

fn meets(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] <= b[2] && a[2] >= b[0] && a[1] <= b[3] && a[3] >= b[1]
}

fn grown(b: [f64; 4], by: f64) -> [f64; 4] {
    [b[0] - by, b[1] - by, b[2] + by, b[3] + by]
}

fn r5(x: f64) -> f64 {
    (x * 1e5).round() / 1e5
}

struct Land {
    rings: Vec<(Vec<Pt>, [f64; 4])>,
}

impl Land {
    /// The rings whose box meets `b`, also those a world to the east, moved there: a feature that crosses
    /// the antimeridian reaches past 256 units.
    fn meeting(&self, b: [f64; 4]) -> Vec<Vec<Pt>> {
        let mut out: Vec<Vec<Pt>> = self.rings.iter().filter(|(_, rb)| meets(*rb, b)).map(|(r, _)| r.clone()).collect();
        if b[2] > 256.0 {
            let east = [b[0] - 256.0, b[1], b[2] - 256.0, b[3]];
            out.extend(self.rings.iter().filter(|(_, rb)| meets(*rb, east)).map(|(r, _)| r.iter().map(|p| [p[0] + 256.0, p[1]]).collect()));
        }
        out
    }

    fn read(path: &Path, tol: f64) -> Land {
        Land {
            rings: read(path)
                .into_iter()
                .flat_map(|f| f.rings)
                .filter_map(|r| simplify_ring(&r, tol))
                .map(|r| {
                    let b = bounds_of(std::slice::from_ref(&r));
                    (r, b)
                })
                .collect(),
        }
    }
}

/// Islands smaller than this share of a water's grid are written over.
const ISLET_SHARE: usize = 400;
/// A territory whose label is this far from its country's is named on its own (Martinique, not France), in degrees.
const OVERSEAS_DEG: f64 = 9.0;

/// "SOUTHERN OCEAN" as "Southern Ocean".
fn title_case(s: &str) -> String {
    s.split(' ')
        .map(|w| {
            let mut c = w.chars();
            c.next().map_or(String::new(), |f| f.to_uppercase().chain(c.flat_map(|c| c.to_lowercase())).collect())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// One degree in units.
const DEGREE: f64 = 256.0 / 360.0;

pub fn run(args: &[String]) {
    let dir = args.first().expect("usage: cargo xtask naturalearth <dir> [--out file]");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut out = root.join("data/labels/naturalearth.json.gz");
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                out = args.get(i + 1).expect("--out needs a path").into();
                i += 2;
            }
            other => panic!("unknown argument {other}"),
        }
    }
    let dir = Path::new(dir);
    let tol = 0.002 * DEGREE;
    let land = Land::read(&dir.join("ne_10m_land.geojson"), tol);
    // The ice shelves are coast to whoever sails there: Natural Earth's land stops at the grounding line,
    // so without them the Weddell Sea's name ran over the Filchner-Ronne shelf.
    let ice = Land::read(&dir.join("ne_10m_antarctic_ice_shelves_polys.geojson"), tol);
    println!("land: {} rings, ice shelves: {}", land.rings.len(), ice.rings.len());

    let mut seen: BTreeMap<(String, String), (usize, usize)> = BTreeMap::new();
    let mut features: Vec<(usize, u8, String, Value)> = Vec::new();
    for (file, source) in [("ne_10m_geography_marine_polys", "marine"), ("ne_10m_geography_regions_polys", "region"), ("ne_10m_geography_regions_points", "point")] {
        for f in read(&dir.join(format!("{file}.geojson"))) {
            let kind = match (source, f.class.as_str()) {
                ("marine", "ocean") => Some("ocean"),
                ("marine", "sea") => Some("sea"),
                ("marine", "gulf") => Some("gulf"),
                ("marine", "bay" | "lagoon" | "fjord" | "inlet") => Some("bay"),
                ("marine", "strait" | "channel" | "sound" | "passage") => Some("strait"),
                ("region", "island group") => Some("island_group"),
                ("region", "island") => Some("island"),
                ("region", "range/mtn") => Some("range"),
                ("region", "peninsula" | "pen/cape") => Some("peninsula"),
                ("point", "cape") => Some("cape"),
                _ => None,
            };
            let entry = seen.entry((source.to_string(), f.class.clone())).or_insert((0, 0));
            entry.0 += 1;
            let Some(kind) = kind else { continue };
            let Some(name_en) = text(&f.props, "name_en").or_else(|| text(&f.props, "name")) else { continue };
            // Only the oceans may go all round the world (the Arctic, the Southern Ocean).
            if f.wraps && kind != "ocean" {
                continue;
            }
            // Natural Earth calls the oceans' halves by the ocean ("Atlantic Ocean"); its own name says which.
            let name_en = if kind == "ocean" { text(&f.props, "name").map(|n| title_case(&n)).unwrap_or(name_en) } else { name_en };
            let rank = number(&f.props, "scalerank").unwrap_or(5.0).clamp(0.0, 255.0);
            let min_zoom = number(&f.props, "min_zoom").or_else(|| number(&f.props, "min_label")).unwrap_or(rank * 0.8 + 1.0);
            let mut names = Map::new();
            names.insert("en".into(), json!(name_en));
            for lang in LANGUAGES {
                if let Some(n) = text(&f.props, &format!("name_{lang}"))
                    && n != name_en
                {
                    names.insert(lang.into(), json!(n));
                }
            }
            let mut feature = json!({"kind": kind, "rank": rank as u8, "min_zoom": r5(min_zoom), "names": names});
            if let Some(max) = number(&f.props, "max_label") {
                feature["max_zoom"] = json!(r5(max));
            }
            if kind == "cape" {
                let Some(at) = f.point else { continue };
                let near = land.meeting(grown([at[0], at[1], at[0], at[1]], 4.0 * 0.05 * DEGREE));
                let Some((angle, normal)) = coast_at(at, &near, 0.05 * DEGREE) else { continue };
                feature["bounds"] = json!([r5(at[0]), r5(at[1]), r5(at[0]), r5(at[1])]);
                feature["cape"] = json!({"at": [r5(at[0]), r5(at[1])], "angle": r5(angle), "normal": [r5(normal[0]), r5(normal[1])]});
            } else {
                let b = bounds_of(&f.rings);
                let mut mask = Mask::over(b, 256);
                mask.fill(&f.rings, true);
                // An ocean's polygon has holes where Natural Earth names a sea in it (the Sargasso Sea cuts the
                // North Atlantic in two at 30 N): for its name, all the water in its box is the ocean.
                if kind == "ocean" && !f.wraps {
                    mask.fill(&[vec![[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]]], true);
                }
                let near = land.meeting(b);
                match kind {
                    "ocean" | "sea" | "gulf" | "bay" | "strait" => {
                        let mut sea = mask.empty_like();
                        sea.fill(&near, true);
                        sea.fill(&ice.meeting(b), true);
                        mask.minus(&sea);
                        // A water's name may run over its islets, as a paper chart's does.
                        mask.fill_small_holes(mask.w * mask.h / ISLET_SHARE);
                    }
                    "island" | "range" | "peninsula" => {
                        let mut ground = mask.empty_like();
                        ground.fill(&near, true);
                        mask.and(&ground);
                        // A range or a peninsula is named only if it is on a coast.
                        if kind != "island" && water_share(&land, b) < 0.02 {
                            continue;
                        }
                    }
                    _ => {}
                }
                // An ocean is named level across its widest open stretch, as on a paper chart, not along a
                // centreline that can run north to south down the middle of the Atlantic.
                let pieces = f.rings.len() > 1;
                let mut found = if kind == "ocean" { level_line(&mask, &f) } else { None }.or_else(|| spine(&mask.largest_component()));
                // Some features are a scatter of fragments (the Strait of Gibraltar is five): the water inside their hull is the feature.
                if found.is_none() && matches!(kind, "sea" | "gulf" | "strait" | "bay") && pieces {
                    let all: Vec<Pt> = f.rings.iter().flatten().copied().collect();
                    let mut hull = mask.empty_like();
                    hull.fill(&[convex_hull(all)], true);
                    let mut sea = mask.empty_like();
                    sea.fill(&near, true);
                    hull.minus(&sea);
                    found = spine(&hull.largest_component());
                }
                // A strait's name runs from one sea to the other, where its outline says which two.
                if kind == "strait"
                    && let Some(open) = strait_spine(&f, b, &land, &ice)
                    && found.as_ref().is_none_or(|s| open.length() > s.length())
                {
                    found = Some(open);
                }
                let Some(s) = found else { continue };
                feature["bounds"] = json!(b.map(r5));
                feature["spine"] = json!({"pts": s.pts.iter().map(|p| [r5(p[0]), r5(p[1])]).collect::<Vec<_>>(), "clear": s.clear.iter().map(|c| r5(*c)).collect::<Vec<_>>()});
            }
            entry.1 += 1;
            let order = KINDS.iter().position(|k| *k == kind).unwrap();
            features.push((order, rank as u8, name_en, feature));
        }
    }
    countries(dir, &land, &mut features);
    println!("\nclass table (source, featurecla: seen, kept):");
    for ((source, class), (n, kept)) in &seen {
        println!("  {source:7} {class:14} {n:4} seen {kept:4} kept");
    }
    features.sort_by(|a, b| (a.0, a.1, &a.2).cmp(&(b.0, b.1, &b.2)));
    let mut counts: BTreeMap<usize, usize> = BTreeMap::new();
    for f in &features {
        *counts.entry(f.0).or_default() += 1;
    }
    println!("\nkept:");
    for (k, n) in &counts {
        println!("  {:13} {n}", KINDS[*k]);
    }
    let bundle = json!({"version": 1, "features": features.iter().map(|f| f.3.clone()).collect::<Vec<_>>()});
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    gz.write_all(serde_json::to_string(&bundle).unwrap().as_bytes()).unwrap();
    let bytes = gz.finish().unwrap();
    println!("\n{}: {} features, {} bytes gzipped", out.display(), features.len(), bytes.len());
    assert!(bytes.len() <= MAX_BYTES, "the bundle is over {MAX_BYTES} bytes");
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(&out, bytes).unwrap();
    println!("\nsamples:");
    for f in &features {
        let en = f.2.as_str();
        if ["Gibraltar", "Balear", "Mallorca", "Reinga", "Hauraki", "Pacific", "Weddell", "Martinique", "Ocean"].iter().any(|s| en.contains(s)) {
            println!("  {:13} rank {} {en} ({})", KINDS[f.0], f.1, if f.3.get("spine").is_some() { "spine" } else { "cape" });
        }
    }
}

/// The countries, named at Natural Earth's label point between its zooms, and the territories far from
/// their country (Martinique, Guadeloupe, Réunion): at a small scale they are what an island is called.
fn countries(dir: &Path, land: &Land, features: &mut Vec<(usize, u8, String, Value)>) {
    let order = KINDS.iter().position(|k| *k == "country").unwrap();
    let all = read(&dir.join("ne_10m_admin_0_countries.geojson"));
    let label = |f: &Feature| Some((number(&f.props, "label_x")?, number(&f.props, "label_y")?));
    let mut at_of: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    for f in &all {
        if let (Some(a), Some(l)) = (text(&f.props, "admin"), label(f)) {
            at_of.insert(a, l);
        }
    }
    let units = read(&dir.join("ne_10m_admin_0_map_units.geojson"));
    let overseas = units.into_iter().filter(|u| {
        let (Some(admin), Some(geounit), Some(l)) = (text(&u.props, "admin"), text(&u.props, "geounit"), label(u)) else { return false };
        admin != geounit && at_of.get(&admin).is_some_and(|c| (c.0 - l.0).hypot(c.1 - l.1) > OVERSEAS_DEG)
    });
    let (mut n, mut far) = (0, 0);
    for (f, is_far) in all.into_iter().map(|f| (f, false)).chain(overseas.map(|f| (f, true))) {
        // `name` is the map's short form ("China", "Dem. Rep. Congo"); `name_en` the long one.
        let (Some(name_en), Some((lx, ly)), Some(min)) = (text(&f.props, "name").or_else(|| text(&f.props, "name_en")), label(&f), number(&f.props, "min_label")) else { continue };
        let rank = number(&f.props, "labelrank").unwrap_or(5.0).clamp(0.0, 255.0);
        let mut names = Map::new();
        names.insert("en".into(), json!(name_en));
        for lang in LANGUAGES {
            if let Some(n) = text(&f.props, &format!("name_{lang}"))
                && n != name_en
            {
                names.insert(lang.into(), json!(n));
            }
        }
        let at = unit_beyond(if f.rings.iter().flatten().any(|p| p[0] > 256.0) && lx < 0.0 { lx + 360.0 } else { lx }, ly);
        // The box of the part the name is in (metropolitan France, not France with Guiana and Réunion), else of the biggest.
        let part = f.rings.iter().find(|r| isomar_core::vector::ring_contains(r, at)).or_else(|| f.rings.iter().max_by_key(|r| r.len()));
        let b = match part {
            Some(r) if !f.wraps => bounds_of(std::slice::from_ref(r)),
            _ => [at[0], at[1], at[0], at[1]],
        };
        let mut feature = json!({"kind": "country", "rank": rank as u8, "min_zoom": r5(min), "names": names, "bounds": b.map(r5), "at": [r5(at[0]), r5(at[1])]});
        // An island state or territory: the land its name is on is no bigger than the country's part
        // (Martinique, not Andorra, whose land is Eurasia). Its name may go beside it, on the sea.
        let ground = land.meeting([at[0], at[1], at[0], at[1]]).into_iter().filter(|r| isomar_core::vector::ring_contains(r, at)).map(|r| bounds_of(std::slice::from_ref(&r))).next();
        if ground.is_some_and(|g| (g[2] - g[0]) <= (b[2] - b[0]) * 1.5 + 0.05 && (g[3] - g[1]) <= (b[3] - b[1]) * 1.5 + 0.05) {
            feature["island"] = json!(true);
        }
        if let Some(max) = number(&f.props, "max_label") {
            feature["max_zoom"] = json!(r5(max));
        }
        features.push((order, rank as u8, name_en, feature));
        n += 1;
        far += usize::from(is_far);
    }
    println!("countries: {n}, of them {far} territories far from their country");
}

/// The latitudes an ocean's name is written between, unless the ocean is all beyond them.
const OCEAN_LATS: (f64, f64) = (-45.0, 60.0);

/// A level line across the longest run of open water in `mask`, between `OCEAN_LATS`, keeping a fortieth
/// of the grid clear of land above and below.
fn level_line(mask: &Mask, f: &Feature) -> Option<isomar_core::labels::Spine> {
    if f.wraps {
        return None;
    }
    let d = mask.distance();
    let need = (mask.w.max(mask.h) as f32 / 40.0).max(2.0);
    let (top, bottom) = (unit(0.0, OCEAN_LATS.1)[1], unit(0.0, OCEAN_LATS.0)[1]);
    let mut best: Option<(usize, usize, usize, f64)> = None;
    for j in 0..mask.h {
        let y = mask.centre(0, j)[1];
        if y < top || y > bottom {
            continue;
        }
        let mut run: Option<usize> = None;
        for i in 0..=mask.w {
            let open = i < mask.w && d[mask.index(i, j)] >= need;
            match (open, run) {
                (true, None) => run = Some(i),
                (false, Some(a)) => {
                    // Longer and roomier is better (the North Pacific's longest run hugs the equator, its edge),
                    // but Mercator stretches the far south: a run across the bottom of the South Atlantic, below
                    // the continents, is not where its name belongs. Weighted by cos³ of the latitude.
                    let lat = isomar_core::geo::unproject(0.0, y, 0.0).lat.to_radians();
                    let mut room: Vec<f32> = (a..i).map(|k| d[mask.index(k, j)]).collect();
                    room.sort_by(f32::total_cmp);
                    let score = (i - a) as f64 * room[room.len() / 2] as f64 * lat.cos().powi(3);
                    if best.is_none_or(|b| score > b.3) {
                        best = Some((j, a, i - 1, score));
                    }
                    run = None;
                }
                _ => {}
            }
        }
    }
    let (j, a, b, score) = best?;
    if std::env::var_os("NE_DEBUG").is_some() {
        let lat = isomar_core::geo::unproject(0.0, mask.centre(0, j)[1], 0.0).lat;
        println!("level line {}: lat {lat:.1}, {} cells, score {score:.0}", text(&f.props, "name").unwrap_or_default(), b - a);
    }
    let mut cols: Vec<usize> = (a..=b).step_by(((b - a) / 16).max(1)).chain(std::iter::once(b)).collect();
    cols.dedup();
    let pts: Vec<Pt> = cols.iter().map(|&i| mask.centre(i, j)).collect();
    let clear = cols.iter().map(|&i| d[mask.index(i, j)] as f64 * mask.cell).collect();
    (pts.len() >= 2).then_some(isomar_core::labels::Spine { pts, clear })
}

/// The share of the area within 0.3 degrees of the box `b` that is water.
fn water_share(land: &Land, b: [f64; 4]) -> f64 {
    let wide = grown(b, 0.3 * DEGREE);
    let mut ground = Mask::over(wide, 64);
    ground.fill(&land.meeting(wide), true);
    let (mut cells, mut water) = (0usize, 0usize);
    for j in 2..ground.h - 2 {
        for i in 2..ground.w - 2 {
            cells += 1;
            water += usize::from(!ground.inside[ground.index(i, j)]);
        }
    }
    water as f64 / cells.max(1) as f64
}

/// The convex hull of `pts` (Andrew's monotone chain).
fn convex_hull(mut pts: Vec<Pt>) -> Vec<Pt> {
    pts.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: Pt, a: Pt, b: Pt| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut hull: Vec<Pt> = Vec::new();
    for pass in 0..2 {
        let start = hull.len();
        let it: Box<dyn Iterator<Item = &Pt>> = if pass == 0 { Box::new(pts.iter()) } else { Box::new(pts.iter().rev()) };
        for p in it {
            while hull.len() >= start + 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], *p) <= 0.0 {
                hull.pop();
            }
            hull.push(*p);
        }
        hull.pop();
    }
    hull
}
