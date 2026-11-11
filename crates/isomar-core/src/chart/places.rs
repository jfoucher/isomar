//! Town and water names and built-up outlines from the OpenFreeMap vector
//! tiles (OpenMapTiles schema, zoom 0 to 14, no key).
//!
//! The Open Waters tiles carry no places, so this is a second, independent
//! download: from a tile the app keeps the `place` points and the
//! `water_name` points and lines (seas, straits, bays), the built-up `landuse` polygons (a town's outline) and, from zoom 13, the
//! `building` footprints. Tiles are cached on disk as the raw answer, like
//! the Open Waters ones. The data is © OpenFreeMap, © OpenMapTiles and
//! © OpenStreetMap contributors (ODbL): planning only.

use super::vtile::{GeomKind, MAX_BYTES, TileError, TileId, VectorTile, agent, write_cache};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The credit the licences ask for whenever this data is on screen.
pub const CREDIT: &str = "© OpenFreeMap, © OpenMapTiles, © OpenStreetMap contributors";

const TILEJSON: &str = "https://tiles.openfreemap.org/planet";
const HOST: &str = "https://tiles.openfreemap.org/";
const PATH: &str = "/{z}/{x}/{y}.pbf";
/// How long the tile address (it carries the planet build's date) is reused.
const BASE_TTL: Duration = Duration::from_secs(6 * 3600);

/// The kinds of place worth a name on a chart, most important first: towns
/// from the `place` layer, waters from `water_name`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PlaceKind {
    /// A sea or an ocean.
    Sea,
    City,
    Town,
    /// A strait, sound, pass or channel.
    Strait,
    /// A bay, gulf, cove or harbour.
    Bay,
    Island,
    Village,
    /// A lake; only lagoons by the sea are shown far out.
    Lagoon,
    Hamlet,
    Suburb,
}

impl PlaceKind {
    fn from_class(class: &str) -> Option<PlaceKind> {
        Some(match class {
            "city" => PlaceKind::City,
            "town" => PlaceKind::Town,
            "island" | "islet" => PlaceKind::Island,
            "village" => PlaceKind::Village,
            "hamlet" => PlaceKind::Hamlet,
            "suburb" | "quarter" | "neighbourhood" => PlaceKind::Suburb,
            _ => return None,
        })
    }

    fn from_water_class(class: &str) -> Option<PlaceKind> {
        Some(match class {
            "ocean" | "sea" => PlaceKind::Sea,
            "strait" => PlaceKind::Strait,
            "bay" => PlaceKind::Bay,
            "lake" => PlaceKind::Lagoon,
            _ => return None,
        })
    }

    /// The view zoom from which the name is shown.
    pub fn min_zoom(self) -> f64 {
        match self {
            PlaceKind::Sea => 4.0,
            PlaceKind::City => 5.0,
            PlaceKind::Town => 8.5,
            // From zoom 7 a strait is named if it can be set along its water (`layers::waters`); level, from zoom 9.
            PlaceKind::Strait => 7.0,
            PlaceKind::Bay => 9.0,
            PlaceKind::Island => 10.0,
            PlaceKind::Lagoon => 11.0,
            PlaceKind::Village => 12.0,
            PlaceKind::Hamlet => 14.0,
            PlaceKind::Suburb => 14.5,
        }
    }

    /// A water or an island: what a name can be set along.
    pub fn is_water_or_island(self) -> bool {
        self.is_water() || self == PlaceKind::Island
    }

    /// A water, whose name is written on the sea rather than beside a dot.
    pub fn is_water(self) -> bool {
        matches!(self, PlaceKind::Sea | PlaceKind::Strait | PlaceKind::Bay | PlaceKind::Lagoon)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub name: String,
    pub kind: PlaceKind,
    /// The tile's own importance order within a kind; lower is bigger.
    pub rank: u32,
    pub at: [i16; 2],
    /// For a water named along a line, the box of the line's piece in this
    /// tile, `[x0, y0, x1, y1]`: the pieces in all tiles say how big it is.
    pub span: Option<[i16; 4]>,
    /// For a water named along a line, its longest path in this tile, cut to the tile (`0..extent`): the
    /// ends of the pieces in neighbouring tiles meet on the tile's edge, so they can be joined.
    pub line: Option<Vec<[i16; 2]>>,
}

/// What one tile holds of towns and named waters, in tile units.
#[derive(Debug, Default, PartialEq)]
pub struct PlacesTile {
    pub extent: i32,
    /// Most important first.
    pub places: Vec<Place>,
    /// Rings of built-up areas, to fill with the non-zero rule: the tiles wind
    /// outer rings and holes in opposite directions.
    pub urban: Vec<Vec<[i16; 2]>>,
    pub buildings: Vec<Vec<[i16; 2]>>,
    /// Rings of the sea (the `water` layer's ocean), to tell which places are
    /// on the coast. Their cuts along the edge of the buffer are not shore.
    pub sea: Vec<Vec<[i16; 2]>>,
    /// The box of each ring of `sea`, `[x0, y0, x1, y1]`.
    pub sea_boxes: Vec<[i16; 4]>,
    /// Rings of the lakes (the `water` layer's lake, which holds the lagoons by the sea: the Étang de
    /// Thau), so that a lagoon's name can be set along its own water.
    pub lakes: Vec<Vec<[i16; 2]>>,
}

/// `landuse` classes that make up a town's built-up area.
const URBAN: [&str; 4] = ["residential", "commercial", "industrial", "retail"];

/// The first of the names a place carries in several languages
/// (`Mar Ligure / Mer Ligure`, `Maghnia;ⵎⵖⵏⵉⵢⴰ;مغنية`): the local one, and a
/// chart has room for one.
fn local_name(name: &str) -> &str {
    name.split(" / ").next().unwrap_or(name).split(';').next().unwrap_or(name).trim()
}

fn clamp(v: i32) -> i16 {
    v.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

/// The longest stretch of the path `p` inside the tile (`0..=extent` both ways): the segments that cross
/// the edge are cut exactly on it.
fn clip_to_tile(p: &[[i32; 2]], extent: i32) -> Vec<[i16; 2]> {
    let e = extent as f64;
    let mut runs: Vec<Vec<[f64; 2]>> = vec![Vec::new()];
    for w in p.windows(2) {
        let (a, b) = (w[0].map(f64::from), w[1].map(f64::from));
        // Liang-Barsky against the tile.
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let (mut t0, mut t1) = (0.0f64, 1.0f64);
        let mut outside = false;
        for (p, q) in [(-dx, a[0]), (dx, e - a[0]), (-dy, a[1]), (dy, e - a[1])] {
            if p == 0.0 {
                outside |= q < 0.0;
            } else if p < 0.0 {
                t0 = t0.max(q / p);
            } else {
                t1 = t1.min(q / p);
            }
        }
        if outside || t0 > t1 {
            runs.push(Vec::new());
            continue;
        }
        let (s, t) = ([a[0] + dx * t0, a[1] + dy * t0], [a[0] + dx * t1, a[1] + dy * t1]);
        let run = runs.last_mut().unwrap();
        // A segment that starts where the last ended carries on; one that does not starts a new run.
        if run.last().is_some_and(|l| (l[0] - s[0]).abs() > 0.5 || (l[1] - s[1]).abs() > 0.5) {
            runs.push(vec![s]);
        } else if run.is_empty() {
            run.push(s);
        }
        runs.last_mut().unwrap().push(t);
    }
    let best = runs.into_iter().max_by_key(Vec::len).unwrap_or_default();
    best.into_iter().map(|q| [clamp(q[0].round() as i32), clamp(q[1].round() as i32)]).collect()
}

impl PlacesTile {
    pub fn from_tile(t: &VectorTile) -> PlacesTile {
        let extent = t.layers.iter().find(|l| matches!(l.name.as_str(), "place" | "landuse" | "building" | "water" | "water_name")).map_or(4096, |l| l.extent as i32);
        let rings = |name: &str, keep: &dyn Fn(&std::collections::HashMap<String, String>) -> bool| -> Vec<Vec<[i16; 2]>> {
            let Some(layer) = t.layer(name) else { return Vec::new() };
            layer
                .features
                .iter()
                .filter(|f| f.kind == GeomKind::Polygon && keep(&f.tags))
                .flat_map(|f| f.paths.iter())
                .filter(|p| p.len() >= 3)
                .map(|p| p.iter().map(|q| [clamp(q[0]), clamp(q[1])]).collect())
                .collect()
        };
        let mut places = Vec::new();
        // A point in the buffer belongs to the neighbouring tile, which has it too.
        let inside = |[x, y]: [i32; 2]| (0..extent).contains(&x) && (0..extent).contains(&y);
        let name_of = |f: &super::vtile::Feature| ["name", "name_int", "name_en"].iter().filter_map(|k| f.tags.get(*k)).map(|n| local_name(n)).find(|n| !n.is_empty()).map(str::to_string);
        if let Some(layer) = t.layer("place") {
            for f in layer.features.iter().filter(|f| f.kind == GeomKind::Point) {
                let Some(kind) = f.tags.get("class").and_then(|c| PlaceKind::from_class(c)) else { continue };
                let Some(name) = name_of(f) else { continue };
                let rank = f.tags.get("rank").and_then(|r| r.parse().ok()).unwrap_or(99);
                for p in &f.paths {
                    let Some(&at) = p.first() else { continue };
                    if inside(at) {
                        places.push(Place { name: name.clone(), kind, rank, at: [clamp(at[0]), clamp(at[1])], span: None, line: None });
                    }
                }
            }
        }
        // Big waters come as a line across them, small ones as a point. The
        // name is written level at the middle of the line's piece in this
        // tile, before the points.
        if let Some(layer) = t.layer("water_name") {
            for f in layer.features.iter().filter(|f| matches!(f.kind, GeomKind::Point | GeomKind::Line)) {
                let Some(kind) = f.tags.get("class").and_then(|c| PlaceKind::from_water_class(c)) else { continue };
                let Some(name) = name_of(f) else { continue };
                let line = f.kind == GeomKind::Line;
                let at = if line { f.paths.iter().max_by_key(|p| p.len()).and_then(|p| p.get(p.len() / 2)) } else { f.paths.first().and_then(|p| p.first()) };
                let span = line.then(|| {
                    let pts = f.paths.iter().flatten();
                    let (xs, ys) = (pts.clone().map(|p| p[0]), pts.map(|p| p[1]));
                    [xs.clone().min(), ys.clone().min(), xs.max(), ys.max()].map(|v| clamp(v.unwrap_or(0)))
                });
                let path = line.then(|| f.paths.iter().map(|p| clip_to_tile(p, extent)).max_by_key(Vec::len)).flatten().filter(|p| p.len() >= 2);
                if let Some(&at) = at.filter(|a| inside(**a)) {
                    places.push(Place { name, kind, rank: if line { 0 } else { 1 }, at: [clamp(at[0]), clamp(at[1])], span, line: path });
                }
            }
        }
        places.sort_by(|a, b| (a.kind, a.rank, &a.name).cmp(&(b.kind, b.rank, &b.name)));
        let sea = rings("water", &|t| t.get("class").is_some_and(|c| c == "ocean"));
        let sea_boxes = sea
            .iter()
            .map(|r| {
                let (xs, ys) = (r.iter().map(|p| p[0]), r.iter().map(|p| p[1]));
                [xs.clone().min(), ys.clone().min(), xs.max(), ys.max()].map(|v| v.unwrap_or(0))
            })
            .collect();
        PlacesTile {
            extent,
            places,
            urban: rings("landuse", &|t| t.get("class").is_some_and(|c| URBAN.contains(&c.as_str()))),
            buildings: rings("building", &|_| true),
            sea,
            sea_boxes,
            lakes: rings("water", &|t| t.get("class").is_some_and(|c| c == "lake")),
        }
    }

    /// Whether (`x`, `y`), in tile units, is on the sea: inside an odd number of its rings.
    pub fn on_sea(&self, x: f64, y: f64) -> bool {
        let mut inside = false;
        for (ring, b) in self.sea.iter().zip(&self.sea_boxes) {
            if x < b[0] as f64 || x > b[2] as f64 || y < b[1] as f64 || y > b[3] as f64 {
                continue;
            }
            let n = ring.len();
            let mut j = n - 1;
            for i in 0..n {
                let (a, c) = (ring[i].map(f64::from), ring[j].map(f64::from));
                if (a[1] > y) != (c[1] > y) && x < (c[0] - a[0]) * (y - a[1]) / (c[1] - a[1]) + a[0] {
                    inside = !inside;
                }
                j = i;
            }
        }
        inside
    }

    /// Whether (`x`, `y`), in tile units, is on a lake: inside an odd number of its rings.
    pub fn on_lake(&self, x: f64, y: f64) -> bool {
        let mut inside = false;
        for ring in &self.lakes {
            let n = ring.len();
            let mut j = n - 1;
            for i in 0..n {
                let (a, c) = (ring[i].map(f64::from), ring[j].map(f64::from));
                if (a[1] > y) != (c[1] > y) && x < (c[0] - a[0]) * (y - a[1]) / (c[1] - a[1]) + a[0] {
                    inside = !inside;
                }
                j = i;
            }
        }
        inside
    }

    /// Whether the segment lies on the artificial edge of the tile's buffer
    /// and so is not an outline.
    pub fn on_edge(&self, a: [i16; 2], b: [i16; 2]) -> bool {
        let out = |v: i16| (v as i32) < 0 || (v as i32) > self.extent;
        (a[0] == b[0] && out(a[0])) || (a[1] == b[1] && out(a[1]))
    }
}

// ---- download --------------------------------------------------------------

/// `(tile address template, when it was read)`.
static BASE: Mutex<Option<(String, Instant)>> = Mutex::new(None);

/// The tile address from OpenFreeMap's TileJSON, with the placeholders still
/// in it. Only an address on OpenFreeMap's own host is accepted.
pub fn base_from_tilejson(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let url = v.get("tiles")?.as_array()?.first()?.as_str()?;
    (url.starts_with(HOST) && url.ends_with(PATH) && url.len() > HOST.len() + PATH.len()).then(|| url.to_string())
}

fn base(refresh: bool) -> Result<String, TileError> {
    let net = TileError::from_net;
    let mut cached = BASE.lock().unwrap_or_else(|e| e.into_inner());
    if !refresh && let Some((url, at)) = cached.as_ref() && at.elapsed() < BASE_TTL {
        return Ok(url.clone());
    }
    let mut res = agent().get(TILEJSON).call().map_err(net)?;
    if res.status().as_u16() != 200 {
        return Err(TileError::Network(format!("tile server answered {}", res.status().as_u16())));
    }
    let text = res.body_mut().with_config().limit(1 << 20).read_to_string().map_err(net)?;
    let url = base_from_tilejson(&text).ok_or_else(|| TileError::Format("unexpected tile server description".into()))?;
    *cached = Some((url.clone(), Instant::now()));
    Ok(url)
}

fn tile_url(base: &str, id: &TileId) -> String {
    base.replace("{z}", &id.z.to_string()).replace("{x}", &id.x.to_string()).replace("{y}", &id.y.to_string())
}

/// Downloads a tile and caches it. Blocking; call from a worker thread.
pub fn fetch(dir: &Path, id: &TileId) -> Result<Vec<u8>, TileError> {
    let net = TileError::from_net;
    let mut refreshed = false;
    loop {
        let mut res = agent().get(tile_url(&base(refreshed)?, id)).call().map_err(net)?;
        let bytes = match res.status().as_u16() {
            200 => res.body_mut().with_config().limit(MAX_BYTES).read_to_vec().map_err(net)?,
            204 => Vec::new(),
            // The planet build in the address may have been replaced: ask again once.
            404 if !refreshed => {
                refreshed = true;
                continue;
            }
            s => return Err(TileError::Network(format!("tile server answered {s}"))),
        };
        // A cache that can't be written still serves this session.
        let _ = write_cache(dir, id, &bytes);
        return Ok(bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::super::vtile::decode;
    use super::super::vtile::testutil::*;
    use super::*;

    fn point<'a>(x: i32, y: i32, class: &'a str, name: &'a str, rank: i64) -> F<'a> {
        F { kind: 1, paths: vec![vec![[x, y]]], tags: vec![("class", V::S(class)), ("name", V::S(name)), ("rank", V::Int(rank))] }
    }

    fn square(x: i32, y: i32, s: i32, class: Option<&'static str>) -> F<'static> {
        F { kind: 3, paths: vec![vec![[x, y], [x + s, y], [x + s, y + s], [x, y + s]]], tags: class.map(|c| vec![("class", V::S(c))]).unwrap_or_default() }
    }

    #[test]
    fn places_are_kept_by_kind_and_ranked_biggest_first() {
        let bytes = layer(
            "place",
            4096,
            &[
                point(10, 10, "village", "Kerlouan", 9),
                point(20, 20, "city", "Brest", 3),
                point(30, 30, "town", "Landerneau", 5),
                point(40, 40, "country", "France", 1),
                point(50, 50, "isolated_dwelling", "Ferme", 9),
                point(60, 60, "islet", "Ile Verte", 9),
            ],
        );
        let t = PlacesTile::from_tile(&decode(&bytes).unwrap());
        let names: Vec<_> = t.places.iter().map(|p| (p.name.as_str(), p.kind)).collect();
        assert_eq!(
            names,
            vec![("Brest", PlaceKind::City), ("Landerneau", PlaceKind::Town), ("Ile Verte", PlaceKind::Island), ("Kerlouan", PlaceKind::Village)]
        );
    }

    #[test]
    fn a_place_in_the_buffer_is_left_to_the_neighbouring_tile() {
        let bytes = layer("place", 4096, &[point(-30, 100, "town", "West", 1), point(4100, 100, "town", "East", 1), point(4095, 0, "town", "Inside", 1)]);
        let t = PlacesTile::from_tile(&decode(&bytes).unwrap());
        assert_eq!(t.places.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), vec!["Inside"]);
    }

    #[test]
    fn a_place_without_a_name_is_skipped_and_the_international_name_stands_in() {
        let unnamed = F { kind: 1, paths: vec![vec![[5, 5]]], tags: vec![("class", V::S("town"))] };
        let intl = F { kind: 1, paths: vec![vec![[6, 6]]], tags: vec![("class", V::S("town")), ("name", V::S(" ")), ("name_int", V::S("Athens"))] };
        let t = PlacesTile::from_tile(&decode(&layer("place", 4096, &[unnamed, intl])).unwrap());
        assert_eq!(t.places.len(), 1);
        assert_eq!(t.places[0].name, "Athens");
    }

    #[test]
    fn named_waters_are_kept_at_a_point_or_the_middle_of_their_line() {
        let water = |kind: u64, paths: Vec<Vec<[i32; 2]>>, class: &'static str, name: &'static str| F { kind, paths, tags: vec![("class", V::S(class)), ("name", V::S(name))] };
        let bytes = layer(
            "water_name",
            4096,
            &[
                water(2, vec![vec![[100, 100], [200, 150], [300, 200]]], "bay", "Rade de Brest"),
                water(1, vec![vec![[400, 400]]], "strait", "Goulet de Brest"),
                water(1, vec![vec![[500, 500]]], "sea", "Mer d'Iroise"),
                water(1, vec![vec![[600, 600]]], "river", "L'Élorn"),
                water(1, vec![vec![[-20, 600]]], "bay", "Next door"),
            ],
        );
        let t = PlacesTile::from_tile(&decode(&bytes).unwrap());
        let got: Vec<_> = t.places.iter().map(|p| (p.name.as_str(), p.kind, p.at)).collect();
        assert_eq!(
            got,
            vec![
                ("Mer d'Iroise", PlaceKind::Sea, [500, 500]),
                ("Goulet de Brest", PlaceKind::Strait, [400, 400]),
                ("Rade de Brest", PlaceKind::Bay, [200, 150])
            ]
        );
        assert!(t.places.iter().all(|p| p.kind.is_water()));
        assert_eq!(t.places[2].span, Some([100, 100, 300, 200]), "a line says how far it reaches");
        assert_eq!(t.places[0].span, None);
        assert_eq!(t.places[2].line, Some(vec![[100, 100], [200, 150], [300, 200]]), "and keeps its path");
        assert_eq!(t.places[0].line, None);
    }

    #[test]
    fn a_line_is_cut_to_the_tile_on_its_edge() {
        // Leaves through the east edge at y 2000 and comes back through it at y 3000.
        let p = [[100, 2000], [5000, 2000], [5000, 3000], [100, 3000]];
        let cut = clip_to_tile(&p, 4096);
        assert!(cut.first() == Some(&[100, 2000]) || cut.first() == Some(&[4096, 3000]), "{cut:?}");
        assert!(cut.iter().all(|q| (0..=4096).contains(&(q[0] as i32)) && (0..=4096).contains(&(q[1] as i32))), "{cut:?}");
        assert_eq!(cut.len(), 2, "the longest of the two stretches inside: {cut:?}");
        assert!(cut.contains(&[4096, 2000]) || cut.contains(&[4096, 3000]), "cut exactly on the edge: {cut:?}");
        assert_eq!(clip_to_tile(&[[-50, 100], [-10, 900]], 4096), Vec::<[i16; 2]>::new());
        // Running along the tile and out the end.
        let cut = clip_to_tile(&[[-100, 500], [200, 500], [900, 500]], 4096);
        assert_eq!(cut, vec![[0, 500], [200, 500], [900, 500]]);
    }

    #[test]
    fn a_name_in_several_languages_is_written_in_the_first() {
        assert_eq!(local_name("Golfe de Gascogne / Golfo de Vizcaya / Bay of Biscay/Bizkaiko golkoa"), "Golfe de Gascogne");
        assert_eq!(local_name("Maghnia;ⵎⵖⵏⵉⵢⴰ;مغنية"), "Maghnia");
        assert_eq!(local_name(" Brest "), "Brest");
    }

    #[test]
    fn the_sea_is_inside_the_ocean_rings_less_the_islands() {
        let ocean = square(0, 0, 100, Some("ocean"));
        // An island: a hole wound the other way.
        let island = F { kind: 3, paths: vec![vec![[0, 0], [100, 0], [100, 100], [0, 100]], vec![[40, 40], [40, 60], [60, 60], [60, 40]]], tags: vec![("class", V::S("ocean"))] };
        let t = PlacesTile::from_tile(&decode(&layer("water", 4096, &[ocean])).unwrap());
        assert!(t.on_sea(50.0, 50.0) && !t.on_sea(150.0, 50.0));
        let t = PlacesTile::from_tile(&decode(&layer("water", 4096, &[island])).unwrap());
        assert!(t.on_sea(20.0, 20.0) && !t.on_sea(50.0, 50.0));
    }

    #[test]
    fn only_built_up_land_use_is_an_outline() {
        let mut bytes = layer(
            "landuse",
            4096,
            &[square(0, 0, 100, Some("residential")), square(200, 0, 100, Some("railway")), square(400, 0, 100, Some("industrial")), square(600, 0, 100, Some("cemetery"))],
        );
        bytes.extend(layer("building", 4096, &[square(10, 10, 5, None), square(30, 30, 5, None)]));
        let t = PlacesTile::from_tile(&decode(&bytes).unwrap());
        assert_eq!(t.urban.len(), 2, "residential and industrial");
        assert_eq!(t.buildings.len(), 2);
        assert!(t.places.is_empty() && t.sea.is_empty());
    }

    #[test]
    fn only_the_ocean_is_the_sea() {
        let bytes = layer("water", 4096, &[square(0, 0, 100, Some("ocean")), square(200, 0, 100, Some("lake")), square(400, 0, 100, Some("river"))]);
        let t = PlacesTile::from_tile(&decode(&bytes).unwrap());
        assert_eq!(t.sea.len(), 1);
        assert_eq!(t.lakes.len(), 1, "the lake is kept apart, for the lagoons' names");
    }

    #[test]
    fn the_buffer_edge_is_not_an_outline() {
        let t = PlacesTile { extent: 4096, ..Default::default() };
        assert!(t.on_edge([-30, 0], [-30, 500]));
        assert!(t.on_edge([0, 4126], [500, 4126]));
        assert!(!t.on_edge([0, 0], [0, 500]), "the tile's own edge is inside the extent");
        assert!(!t.on_edge([-30, 0], [10, 20]));
    }

    #[test]
    fn the_tile_address_comes_from_the_tilejson_and_only_on_openfreemaps_host() {
        let good = r#"{"tilejson":"3.0.0","tiles":["https://tiles.openfreemap.org/planet/20261018_080001_pt/{z}/{x}/{y}.pbf"]}"#;
        let base = base_from_tilejson(good).unwrap();
        assert_eq!(tile_url(&base, &TileId { z: 11, x: 998, y: 708 }), "https://tiles.openfreemap.org/planet/20261018_080001_pt/11/998/708.pbf");
        assert_eq!(base_from_tilejson(r#"{"tiles":["https://evil.example/planet/{z}/{x}/{y}.pbf"]}"#), None);
        assert_eq!(base_from_tilejson(r#"{"tiles":["https://tiles.openfreemap.org/{z}/{x}/{y}.pbf"]}"#), None);
        assert_eq!(base_from_tilejson(r#"{"tiles":[]}"#), None);
        assert_eq!(base_from_tilejson("not json"), None);
    }

    #[test]
    fn kinds_appear_in_order_of_size() {
        assert!(PlaceKind::City.min_zoom() < PlaceKind::Town.min_zoom());
        assert!(PlaceKind::Town.min_zoom() < PlaceKind::Village.min_zoom());
        assert!(PlaceKind::Village.min_zoom() < PlaceKind::Hamlet.min_zoom());
    }

    #[test]
    #[ignore = "downloads from OpenFreeMap"]
    fn real_tile_for_brest_has_the_city_and_its_outline() {
        let d = tempfile::tempdir().unwrap();
        let id = TileId::at(11, 48.39, -4.49);
        let t = PlacesTile::from_tile(&decode(&fetch(d.path(), &id).unwrap()).unwrap());
        assert!(t.places.iter().any(|p| p.name == "Brest"), "{:?}", t.places.iter().take(5).collect::<Vec<_>>());
        assert!(!t.urban.is_empty());
    }
}
