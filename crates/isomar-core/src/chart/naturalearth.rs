//! Names from Natural Earth (public domain): seas, straits, gulfs, island groups, ranges and capes, with the
//! centreline of each worked out ahead of time by `cargo xtask naturalearth`, so that writing one along its
//! feature takes no geometry at run time. Like the registry, the bundle is compiled in, so every shell has it.

use crate::labels::Spine;
use crate::vector::Bounds;
use std::collections::HashMap;
use std::io::Read;
use std::sync::OnceLock;

const BUNDLE: &[u8] = include_bytes!("../../../../data/labels/naturalearth.json.gz");

pub const CREDIT: &str = "Natural Earth";

/// What a name is of. The order is the priority: the biggest things win a crowded spot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeoKind {
    Ocean,
    Country,
    Sea,
    Gulf,
    Strait,
    IslandGroup,
    Island,
    Range,
    Peninsula,
    Bay,
    Cape,
}

/// A cape's name goes beside it, on the sea side: where it is, which way the coast runs and the unit
/// normal pointing to the sea. In units.
#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize)]
pub struct Cape {
    pub at: [f64; 2],
    pub angle: f64,
    pub normal: [f64; 2],
}

#[derive(Clone, Debug, serde::Deserialize)]
struct Raw {
    kind: GeoKind,
    rank: u8,
    min_zoom: f64,
    bounds: [f64; 4],
    names: HashMap<String, String>,
    spine: Option<Spine>,
    cape: Option<Cape>,
    #[serde(default)]
    max_zoom: Option<f64>,
    #[serde(default)]
    at: Option<[f64; 2]>,
    #[serde(default)]
    island: bool,
}

#[derive(serde::Deserialize)]
struct Bundle {
    features: Vec<Raw>,
}

#[derive(Clone, Debug)]
pub struct GeoName {
    pub id: u32,
    pub kind: GeoKind,
    pub rank: u8,
    pub min_zoom: f64,
    /// The zoom past which Natural Earth stops naming it, if it says.
    pub max_zoom: Option<f64>,
    /// `[x0, y0, x1, y1]` in units (Web Mercator at zoom 0, 256 across). A feature that crosses the
    /// antimeridian reaches past 256: it is one world to the east of its western part.
    pub bounds: [f64; 4],
    pub names: HashMap<String, String>,
    pub spine: Option<Spine>,
    pub cape: Option<Cape>,
    /// Where a country's name goes, in units.
    pub at: Option<[f64; 2]>,
    /// A country that is an island or a few (Martinique, Barbados): its name may go beside it, on the sea.
    pub island: bool,
}

impl GeoName {
    /// The name in `lang` ("fr", "pt", "zht"…), English where Natural Earth has none.
    pub fn name(&self, lang: &str) -> &str {
        self.names.get(lang).or_else(|| self.names.get("en")).map_or("", String::as_str)
    }

    /// Every name it has, in every language.
    pub fn all_names(&self) -> impl Iterator<Item = &str> {
        self.names.values().map(String::as_str)
    }
}

fn load(bytes: &[u8]) -> Vec<GeoName> {
    let mut json = Vec::new();
    if flate2::read::GzDecoder::new(bytes).read_to_end(&mut json).is_err() {
        return Vec::new();
    }
    let Ok(bundle) = serde_json::from_slice::<Bundle>(&json) else { return Vec::new() };
    bundle
        .features
        .into_iter()
        .enumerate()
        .map(|(i, r)| GeoName { id: i as u32, kind: r.kind, rank: r.rank, min_zoom: r.min_zoom, max_zoom: r.max_zoom, bounds: r.bounds, names: r.names, spine: r.spine, cape: r.cape, at: r.at, island: r.island })
        .collect()
}

/// Every name in the bundle; unpacked on first use.
pub fn all() -> &'static [GeoName] {
    static ALL: OnceLock<Vec<GeoName>> = OnceLock::new();
    ALL.get_or_init(|| load(BUNDLE))
}

/// The names whose box meets `b` (`[x0, y0, x1, y1]` in units).
pub fn within(b: [f64; 4]) -> impl Iterator<Item = &'static GeoName> {
    all().iter().filter(move |g| meets_wrapped(&g.bounds, &b))
}

/// Whether the boxes `a` and `b` (units) meet, either as they are or with one a world to the east of
/// the other: a feature across the antimeridian is past 256, the view of its western end is not.
pub fn meets_wrapped(a: &[f64; 4], b: &[f64; 4]) -> bool {
    let meets = |dx: f64| a[0] + dx <= b[2] && a[2] + dx >= b[0] && a[1] <= b[3] && a[3] >= b[1];
    meets(0.0) || meets(-256.0) || meets(256.0)
}

/// The box `b` of a view in degrees as units.
pub fn units_of(b: &Bounds) -> [f64; 4] {
    let (x0, y1) = crate::geo::project(&crate::geo::LatLon::new(b.min_lat, b.min_lon), 0.0);
    let (x1, y0) = crate::geo::project(&crate::geo::LatLon::new(b.max_lat, b.max_lon), 0.0);
    [x0, y0, x1, y1]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn by_en(en: &str) -> &'static GeoName {
        all().iter().find(|g| g.names.get("en").is_some_and(|n| n.starts_with(en))).unwrap_or_else(|| panic!("{en} is not in the bundle"))
    }

    #[test]
    fn the_bundle_has_the_strait_of_gibraltar_the_balearic_sea_and_cape_reinga() {
        let g = by_en("Strait of Gibraltar");
        assert_eq!(g.kind, GeoKind::Strait);
        assert!(g.spine.as_ref().is_some_and(|s| s.pts.len() >= 2));
        // The strait lies at about 36 N, 5.5 W.
        let (x, y) = crate::geo::project(&crate::geo::LatLon::new(36.0, -5.5), 0.0);
        assert!(g.bounds[0] <= x && x <= g.bounds[2] && g.bounds[1] <= y && y <= g.bounds[3]);
        // Natural Earth has the Balearic Sea, not the Canal de Mallorca.
        let g = by_en("Balearic Sea");
        assert_eq!(g.kind, GeoKind::Sea);
        assert!(g.spine.is_some());
        let g = by_en("Cape Reinga");
        assert_eq!(g.kind, GeoKind::Cape);
        assert!(g.cape.is_some() && g.spine.is_none());
        assert_eq!(by_en("Balearic Islands").kind, GeoKind::IslandGroup);
    }

    #[test]
    fn the_oceans_across_the_antimeridian_and_the_countries_are_there() {
        let pacific = by_en("North Pacific Ocean");
        assert_eq!(pacific.kind, GeoKind::Ocean);
        assert!(pacific.bounds[2] > 256.0, "one piece, reaching past the antimeridian: {:?}", pacific.bounds);
        assert!(pacific.spine.as_ref().is_some_and(|s| s.pts.len() >= 2));
        // Seen from just west of Hawaii (-160) and from Japan's side (150).
        let hawaii = crate::geo::project(&crate::geo::LatLon::new(20.0, -160.0), 0.0);
        assert!(within([hawaii.0, hawaii.1, hawaii.0, hawaii.1]).any(|g| g.id == pacific.id));
        for en in ["South Pacific Ocean", "North Atlantic Ocean", "Southern Ocean", "Arctic Ocean", "Indian Ocean"] {
            assert_eq!(by_en(en).kind, GeoKind::Ocean, "{en}");
        }
        let martinique = by_en("Martinique");
        assert_eq!(martinique.kind, GeoKind::Country, "a territory far from France is named on its own");
        assert!(martinique.at.is_some() && martinique.max_zoom.is_some() && martinique.island);
        assert!(!all().iter().any(|g| g.names["en"] == "Andorra" && g.island), "a country on the continent is no island");
        assert!(all().iter().any(|g| g.kind == GeoKind::Country && g.names["en"] == "France"));
        assert!(!all().iter().any(|g| g.kind == GeoKind::Country && g.names["en"].starts_with("Flemish")), "a region next to its country is not");
    }

    #[test]
    fn every_spine_reads_left_to_right_and_has_room() {
        let mut n = 0;
        for g in all() {
            let Some(s) = &g.spine else { continue };
            n += 1;
            assert!(s.pts.len() >= 2 && s.pts.len() == s.clear.len(), "{}", g.name("en"));
            assert!(s.pts[0][0] <= s.pts.last().unwrap()[0], "{} reads right to left", g.name("en"));
            assert!(s.clear.iter().all(|c| *c >= 0.0) && s.clear.iter().any(|c| *c > 0.0), "{}", g.name("en"));
        }
        assert!(n > 500, "{n} spines");
        assert!(all().iter().all(|g| g.bounds[0] <= g.bounds[2] && g.bounds[1] <= g.bounds[3]));
    }

    #[test]
    fn a_name_falls_back_to_english() {
        let g = by_en("Strait of Gibraltar");
        assert_eq!(g.name("en"), "Strait of Gibraltar");
        assert_eq!(g.name("fr"), g.names["fr"], "French has one");
        assert_eq!(g.name("xx"), "Strait of Gibraltar");
        let reinga = by_en("Cape Reinga");
        assert!(within(reinga.bounds).any(|g| g.id == reinga.id));
    }
}
