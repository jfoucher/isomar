//! The catalogue of free chart sources: what exists for a region, how good it
//! is, under what licence, and how to install it.
//!
//! Ratings: **A** official, navigation-grade charts; **B** open data with
//! real aids to navigation, for planning; **C** only OpenStreetMap seamarks
//! and basic bathymetry.

use serde::Deserialize;

const BUILTIN: &str = include_str!("../../../data/registry.json");

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
pub enum Rating {
    A,
    B,
    C,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Enc,
    Raster,
    Aids,
    Coastline,
    Bathymetry,
    /// A web service the app reads by itself, with nothing to install.
    Service,
    /// Sold by the producer; shown with a link.
    Paid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Live,
    Stale,
    Dead,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Install {
    /// `zip`, `7z` or `file`.
    pub archive: String,
    /// Folder under the data directory.
    pub dir: String,
    #[serde(default)]
    pub max_unpacked_bytes: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Entry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub region: String,
    /// `[west, south, east, north]`
    pub bbox: [f64; 4],
    /// Where the source really has data, as boxes like `bbox`, when the bounding box is far
    /// too wide (NOAA's reaches from Alaska to the Caribbean, Haiti included).
    #[serde(default)]
    pub areas: Vec<[f64; 4]>,
    pub rating: Rating,
    pub kind: Kind,
    #[serde(default)]
    pub format: String,
    pub url: String,
    #[serde(default)]
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub licence: String,
    #[serde(default)]
    pub attribution: String,
    #[serde(default)]
    pub nav_grade: bool,
    #[serde(default = "none")]
    pub encryption: String,
    #[serde(default)]
    pub redistribute: bool,
    pub status: Status,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub install: Option<Install>,
}

fn none() -> String {
    "none".into()
}

impl Entry {
    pub fn covers(&self, lat: f64, lon: f64) -> bool {
        let inside = |b: &[f64; 4]| lat >= b[1] && lat <= b[3] && lon >= b[0] && lon <= b[2];
        inside(&self.bbox) && (self.areas.is_empty() || self.areas.iter().any(inside))
    }

    /// Whether the app can fetch and use it by itself.
    pub fn installable(&self) -> bool {
        self.install.is_some() && self.status != Status::Dead && self.encryption == "none"
    }
}

#[derive(Debug)]
pub struct Registry {
    pub entries: Vec<Entry>,
}

#[derive(Deserialize)]
struct File {
    version: u32,
    entries: Vec<Entry>,
}

pub fn parse(json: &str) -> Result<Registry, String> {
    let f: File = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if f.version != 1 {
        return Err(format!("unsupported registry version {}", f.version));
    }
    let mut seen = std::collections::HashSet::new();
    for e in &f.entries {
        if !seen.insert(e.id.clone()) {
            return Err(format!("duplicate entry {}", e.id));
        }
        for b in std::iter::once(&e.bbox).chain(&e.areas) {
            let [w, s, ea, n] = *b;
            if !(-90.0..=90.0).contains(&s) || !(-90.0..=90.0).contains(&n) || s >= n || !(-180.0..=180.0).contains(&w) || !(-180.0..=180.0).contains(&ea) || w >= ea {
                return Err(format!("{}: bad bounding box {:?}", e.id, b));
            }
        }
        if !e.url.starts_with("https://") {
            return Err(format!("{}: the address must be https", e.id));
        }
        if let Some(i) = &e.install {
            if !matches!(i.archive.as_str(), "zip" | "7z" | "file") {
                return Err(format!("{}: unknown archive type {}", e.id, i.archive));
            }
            if i.dir.is_empty() || i.dir.contains("..") || i.dir.starts_with('/') {
                return Err(format!("{}: unsafe install folder {:?}", e.id, i.dir));
            }
        }
    }
    Ok(Registry { entries: f.entries })
}

/// The catalogue that ships with the app.
pub fn builtin() -> Registry {
    parse(BUILTIN).expect("the built-in registry is valid")
}

impl Registry {
    /// Sources covering a position, best rating first, then smallest download.
    pub fn for_position(&self, lat: f64, lon: f64) -> Vec<&Entry> {
        let mut v: Vec<&Entry> = self.entries.iter().filter(|e| e.covers(lat, lon)).collect();
        v.sort_by_key(|e| (e.rating, e.size_bytes.unwrap_or(u64::MAX), e.id.clone()));
        v
    }

    /// The best rating available for a position.
    pub fn best_rating(&self, lat: f64, lon: f64) -> Option<Rating> {
        self.for_position(lat, lon).iter().map(|e| e.rating).min()
    }

    /// The best rating among sources the app can use today.
    pub fn best_usable_rating(&self, lat: f64, lon: f64) -> Option<Rating> {
        self.for_position(lat, lon).iter().filter(|e| e.installable() || e.kind == Kind::Paid || (e.kind == Kind::Service && e.status == Status::Live)).map(|e| e.rating).min()
    }
}

/// What to tell the user about the chart quality of a region.
pub fn describe(r: Option<Rating>) -> &'static str {
    match r {
        Some(Rating::A) => "Official charts are available here.",
        Some(Rating::B) => "Open data with real aids to navigation: good for planning.",
        Some(Rating::C) => "Only OpenStreetMap seamarks and coarse depth: basic, not for navigation.",
        None => "No chart source is known for this region.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_registry_is_valid() {
        let r = builtin();
        assert!(r.entries.len() >= 10);
        for e in &r.entries {
            assert!(!e.name.is_empty() && !e.licence.is_empty(), "{}", e.id);
            // Anything the app installs itself must be unencrypted and carry a size to show.
            if e.installable() {
                assert!(e.size_bytes.is_some(), "{} needs a size", e.id);
                assert_eq!(e.encryption, "none");
            }
            // Charts we do not redistribute must never claim to be redistributable data.
            if e.kind == Kind::Paid {
                assert!(!e.redistribute && e.install.is_none(), "{}", e.id);
            }
        }
        // Encrypted sources are never offered for installation.
        assert!(r.entries.iter().filter(|e| e.encryption != "none").all(|e| !e.installable()));
    }

    #[test]
    fn regions_get_the_right_sources_best_first() {
        let r = builtin();
        let brest: Vec<&str> = r.for_position(48.38, -4.5).iter().map(|e| e.id.as_str()).collect();
        assert!(brest.contains(&"shom-balisage") && brest.contains(&"shom-limite-terre-mer") && brest.contains(&"shom-enc-paid"));
        assert!(brest.contains(&"openwaters-seamap"), "worldwide coverage applies everywhere");
        assert!(!brest.contains(&"noaa-enc"));
        let ratings: Vec<Rating> = r.for_position(48.38, -4.5).iter().map(|e| e.rating).collect();
        assert!(ratings.windows(2).all(|w| w[0] <= w[1]), "sorted by rating: {ratings:?}");
        // Saint-Malo has the 5 m grid, Brest does not.
        assert!(r.for_position(48.65, -2.0).iter().any(|e| e.id == "shom-dtm-saint-malo-5m"));
        assert!(!brest.contains(&"shom-dtm-saint-malo-5m"));
    }

    #[test]
    fn ratings_by_region_match_the_research() {
        let r = builtin();
        assert_eq!(r.best_rating(42.36, -71.05), Some(Rating::A), "Boston: NOAA");
        let noaa = |lat: f64, lon: f64| r.for_position(lat, lon).iter().any(|e| e.id == "noaa-enc");
        for (lat, lon, place) in [
            (42.36, -71.05, "Boston"),
            (25.77, -80.13, "Miami"),
            (24.55, -81.8, "Key West"),
            (29.3, -94.8, "Galveston"),
            (37.8, -122.5, "San Francisco"),
            (47.6, -122.35, "Seattle"),
            (61.2, -149.9, "Anchorage"),
            (52.0, 175.0, "the western Aleutians"),
            (21.3, -157.9, "Honolulu"),
            (18.47, -66.1, "San Juan"),
            (18.34, -64.93, "Saint Thomas"),
            (13.45, 144.65, "Guam"),
            (-14.27, -170.7, "Pago Pago"),
            (41.9, -87.6, "Chicago"),
        ] {
            assert!(noaa(lat, lon), "NOAA covers {place}");
        }
        for (lat, lon, place) in [(18.55, -72.35, "Port-au-Prince"), (19.75, -72.2, "Cap-Haïtien"), (23.1, -82.4, "Havana"), (25.06, -77.35, "Nassau"), (19.4, -99.1, "Mexico City"), (9.0, -79.5, "Panama"), (14.6, -61.07, "Martinique")] {
            assert!(!noaa(lat, lon), "NOAA has nothing at {place}");
        }
        assert_eq!(r.best_rating(48.38, -4.5), Some(Rating::A), "France: SHOM sells official ENC");
        // What is usable today in France without paying: planning data.
        let free_fr = r.for_position(48.38, -4.5).into_iter().filter(|e| e.installable()).map(|e| e.rating).min();
        assert_eq!(free_fr, Some(Rating::B));
        assert_eq!(r.best_rating(-17.7, -149.4), Some(Rating::C), "Tahiti: only OSM");
        assert_eq!(r.best_rating(-36.2, 175.4), Some(Rating::A), "Great Barrier Island");
        // Nothing is installed in Tahiti; the app uses the Seamap tiles by itself.
        assert!(r.for_position(-17.7, -149.4).into_iter().all(|e| !e.installable()));
        assert_eq!(r.best_usable_rating(-17.7, -149.4), Some(Rating::C), "the worldwide tile service counts");
        // The worldwide layer stops at 85 degrees; only the depth model goes on to the north pole.
        assert_eq!(r.best_rating(89.0, 0.0), Some(Rating::C), "Arctic: the depth model only");
        assert_eq!(r.best_rating(-89.0, 0.0), None, "nothing at the south pole");
    }

    #[test]
    fn installable_means_unencrypted_with_install_instructions() {
        let r = builtin();
        let linz = r.entries.iter().find(|e| e.id == "linz-enc").unwrap();
        assert!(!linz.installable(), "S-63 is not supported");
        let nz = r.entries.iter().find(|e| e.id == "nz-kap-archive").unwrap();
        assert!(nz.installable());
        assert_eq!(nz.status, Status::Stale);
        assert!(!nz.nav_grade);
    }

    #[test]
    fn bad_registries_are_rejected() {
        let base = |extra: &str| format!(r#"{{"version":1,"entries":[{{"id":"a","name":"A","bbox":[0,0,1,1],"rating":"B","kind":"enc","url":"https://x/y","status":"live"{extra}}}]}}"#);
        assert!(parse(&base("")).is_ok());
        assert!(parse(&base(r#","install":{"archive":"zip","dir":"../up"}"#)).is_err());
        assert!(parse(&base(r#","install":{"archive":"rar","dir":"ok"}"#)).is_err());
        assert!(parse(&base(r#","install":{"archive":"zip","dir":"/abs"}"#)).is_err());
        assert!(parse(&base("").replace("https://x/y", "http://x/y")).is_err());
        assert!(parse(&base("").replace("[0,0,1,1]", "[5,0,1,1]")).is_err(), "west is east of east");
        assert!(parse(&base("").replace("[0,0,1,1]", "[0,0,1,99]")).is_err());
        assert!(parse(&base(r#","areas":[[0,0,1,1]]"#)).is_ok());
        assert!(parse(&base(r#","areas":[[1,0,0,1]]"#)).is_err(), "areas are checked like the box");
        assert!(parse(&base("").replace("\"B\"", "\"Z\"")).is_err());
        assert!(parse(&base("").replace("\"version\":1", "\"version\":9")).is_err());
        let dup = base("").replace("]}", &format!(",{}]}}", base("").split_once("[{").unwrap().1.trim_end_matches("]}")));
        assert!(parse(&dup).is_err(), "duplicate id");
        assert!(parse("nonsense").is_err());
    }

    #[test]
    fn descriptions_cover_every_rating() {
        for r in [Some(Rating::A), Some(Rating::B), Some(Rating::C), None] {
            assert!(!describe(r).is_empty());
        }
        assert_ne!(describe(Some(Rating::A)), describe(Some(Rating::C)));
    }
}
