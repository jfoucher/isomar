//! Every named cape in OpenStreetMap (`natural=cape`, and the localities and points named as capes), and every
//! named bay and cove (`natural=bay`, `water=cove`), from `cargo xtask capes`: the base maps' tiles carry no capes,
//! and their coves only at zoom 14, too small to name. Bundled like the Natural Earth names, so the chart names
//! points and coves offline and in every shell. The files are databases derived from OpenStreetMap and are under
//! the ODbL (`data/labels/capes.LICENSE`).

use std::io::Read;
use std::sync::OnceLock;

const BUNDLE: &[u8] = include_bytes!("../../../../data/labels/capes.json.gz");
const COVES_BUNDLE: &[u8] = include_bytes!("../../../../data/labels/coves.json.gz");

pub const CREDIT: &str = "© OpenStreetMap contributors";

/// Positions are stored as whole multiples of this, in units: about a metre at the equator.
pub const SCALE: f64 = 131_072.0;

/// A cape (or a cove), with where OSM puts it (at its tip, mostly) in units.
#[derive(Clone, Debug, PartialEq)]
pub struct Cape {
    pub id: u32,
    pub name: String,
    pub at: [f64; 2],
    /// 1 for the better known (they have a Wikidata entry), 2 for the rest, 3 for a place whose name says it is a
    /// cape that is not mapped as one (a locality, a point).
    pub rank: u8,
}

/// The bundle as `cargo xtask capes` writes it: positions as differences from the one before, which
/// gzip packs far better, in the order of their cells.
#[derive(serde::Deserialize)]
struct Bundle {
    names: Vec<String>,
    dx: Vec<i64>,
    dy: Vec<i64>,
    rank: Vec<u8>,
}

/// The capes, sorted by the unit square they are in (`cell`), with where each square's run starts.
pub struct Capes {
    all: Vec<Cape>,
    /// `(cell, first index)`, sorted by cell.
    starts: Vec<(u32, u32)>,
}

/// Words that make a name a cape's, in the languages of the coasts with charts (whole words).
const CAPE_WORDS: [&str; 24] = [
    "point", "pt", "cape", "head", "ness", "bill", "mull", "pointe", "pte", "punta", "pta", "ponta", "cap", "cabo", "capo", "capu", "kap", "kaap", "hoek",
    "odde", "nes", "akra", "rt", "ras",
];

/// Whether a name is a cape's: "Montauk Point", "Pointe de Corsen", "Cabo de Palos", "Beachy Head".
pub fn is_cape_name(text: &str) -> bool {
    text.split(|c: char| !c.is_alphanumeric()).any(|w| CAPE_WORDS.iter().any(|c| w.eq_ignore_ascii_case(c)))
}

/// The cape word a chart writes short, written out.
const CAPE_SHORT: [(&str, &str); 3] = [("pt", "point"), ("pte", "pointe"), ("pta", "punta")];

/// A cape's generic word, lower case and written out: "Cabo Fisterra" is "cabo", "Pta. de la Estaca" "punta",
/// "Cape Ortegal" "cape"; `None` without one.
pub fn cape_word(text: &str) -> Option<String> {
    let words: Vec<String> = text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect();
    let w = words.iter().find(|w| CAPE_WORDS.contains(&w.as_str()))?;
    Some(CAPE_SHORT.iter().find(|(s, _)| s == w).map_or(w.clone(), |(_, long)| long.to_string()))
}

/// Words that join a name's parts, in the languages of the coasts with charts, and the words of a lighthouse's
/// name round its place.
const JOINING_WORDS: [&str; 34] = [
    "de", "da", "do", "das", "dos", "del", "della", "di", "du", "des", "la", "le", "les", "el", "lo", "los", "las", "l", "d", "of", "the", "s", "sa", "ses",
    "es", "en", "na", "o", "a", "faro", "farol", "phare", "lighthouse", "light",
];

/// The words that tell a cape from others, without accents and in lower case, leaving out its generic word and
/// the words that join: "Cabo Ortegal", "Cape Ortegal" and "Faro do cabo Ortegal" are all `ortegal`, "Punta da
/// Estaca de Bares" and "Punta de la Estaca de Bares" `estaca bares`. Names in two languages that differ in
/// more than that (Fisterra, Finisterre) do not meet.
pub fn name_stem(text: &str) -> String {
    let folded: String = text.chars().flat_map(char::to_lowercase).map(unaccented).collect();
    folded
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !CAPE_WORDS.contains(w) && !JOINING_WORDS.contains(w))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A Latin letter without its accent.
fn unaccented(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'ç' => 'c',
        'è' | 'é' | 'ê' | 'ë' => 'e',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'ñ' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => 'o',
        'ù' | 'ú' | 'û' | 'ü' => 'u',
        'ý' | 'ÿ' => 'y',
        '’' => '\'',
        c => c,
    }
}

/// Words that make a name a cove's or a small bay's (whole words).
const COVE_WORDS: [&str; 21] = [
    "cove", "creek", "inlet", "bight", "bay", "harbor", "harbour", "anse", "crique", "calanque", "baie", "cala", "caló", "calo", "cale", "caleta", "enseada",
    "ensenada", "bucht", "vik", "vig",
];

/// Whether a name is a cove's: "Fort Pond Bay", "Anse des Blancs Sablons", "Cala Figuereta".
pub fn is_cove_name(text: &str) -> bool {
    text.split(|c: char| !c.is_alphanumeric()).any(|w| COVE_WORDS.iter().any(|c| w.to_lowercase() == *c))
}

/// The unit square of a point: 256 by 256 of them over the world.
fn cell(x: f64, y: f64) -> u32 {
    let (cx, cy) = (x.floor().clamp(0.0, 255.0) as u32, y.floor().clamp(0.0, 255.0) as u32);
    cy * 256 + cx
}

impl Capes {
    fn from_bundle(b: Bundle) -> Capes {
        let (mut x, mut y) = (0i64, 0i64);
        let mut all: Vec<Cape> = Vec::with_capacity(b.names.len());
        for (i, ((name, (dx, dy)), rank)) in b.names.into_iter().zip(b.dx.into_iter().zip(b.dy)).zip(b.rank).enumerate() {
            x += dx;
            y += dy;
            all.push(Cape { id: i as u32, name, at: [x as f64 / SCALE, y as f64 / SCALE], rank });
        }
        Capes::new(all)
    }

    /// From capes in any order: sorted into their cells, ids kept.
    pub fn new(mut all: Vec<Cape>) -> Capes {
        all.sort_by_key(|c| (cell(c.at[0], c.at[1]), c.id));
        let mut starts: Vec<(u32, u32)> = Vec::new();
        for (i, c) in all.iter().enumerate() {
            let k = cell(c.at[0], c.at[1]);
            if starts.last().is_none_or(|s| s.0 != k) {
                starts.push((k, i as u32));
            }
        }
        Capes { all, starts }
    }

    pub fn len(&self) -> usize {
        self.all.len()
    }

    pub fn is_empty(&self) -> bool {
        self.all.is_empty()
    }

    /// The capes in the box `b` (`[x0, y0, x1, y1]`, units).
    pub fn within(&self, b: [f64; 4]) -> impl Iterator<Item = &Cape> {
        let (x0, x1) = (b[0].floor().clamp(0.0, 255.0) as u32, b[2].floor().clamp(0.0, 255.0) as u32);
        let (y0, y1) = (b[1].floor().clamp(0.0, 255.0) as u32, b[3].floor().clamp(0.0, 255.0) as u32);
        (y0..=y1)
            .flat_map(move |cy| (x0..=x1).map(move |cx| cy * 256 + cx))
            .filter_map(|k| {
                let i = self.starts.binary_search_by_key(&k, |s| s.0).ok()?;
                let end = self.starts.get(i + 1).map_or(self.all.len(), |s| s.1 as usize);
                Some(&self.all[self.starts[i].1 as usize..end])
            })
            .flatten()
            .filter(move |c| c.at[0] >= b[0] && c.at[0] <= b[2] && c.at[1] >= b[1] && c.at[1] <= b[3])
    }
}

fn load(bytes: &[u8]) -> Capes {
    let mut json = Vec::new();
    if flate2::read::GzDecoder::new(bytes).read_to_end(&mut json).is_err() {
        return Capes::new(Vec::new());
    }
    serde_json::from_slice::<Bundle>(&json).map_or_else(|_| Capes::new(Vec::new()), Capes::from_bundle)
}

/// Every cape in the bundle; unpacked on first use.
pub fn all() -> &'static Capes {
    static ALL: OnceLock<Capes> = OnceLock::new();
    ALL.get_or_init(|| load(BUNDLE))
}

/// Every bay and cove in the bundle, by the same means; unpacked on first use.
pub fn coves() -> &'static Capes {
    static ALL: OnceLock<Capes> = OnceLock::new();
    ALL.get_or_init(|| load(COVES_BUNDLE))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(lat: f64, lon: f64) -> [f64; 2] {
        let (x, y) = crate::geo::project(&crate::geo::LatLon::new(lat, lon), 0.0);
        [x, y]
    }

    #[test]
    fn the_bundle_has_the_points_of_mallorca_and_montauk() {
        let capes = all();
        assert!(capes.len() > 60_000, "{}", capes.len());
        let (a, b) = (unit(39.85, 2.3), unit(39.55, 2.7));
        let names: Vec<&str> = capes.within([a[0], a[1], b[0], b[1]]).map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"Punta Roja") && names.contains(&"punta de Son Serralta"), "{names:?}");
        let roja = capes.within([a[0], a[1], b[0], b[1]]).find(|c| c.name == "Punta Roja").unwrap();
        let at = unit(39.6646061, 2.4763358);
        assert!((roja.at[0] - at[0]).abs() < 1e-4 && (roja.at[1] - at[1]).abs() < 1e-4, "{:?} {at:?}", roja.at);
        let (a, b) = (unit(41.2, -72.2), unit(40.9, -71.8));
        assert!(capes.within([a[0], a[1], b[0], b[1]]).any(|c| c.name == "Montauk Point"));
    }

    #[test]
    fn the_coves_of_mallorca_are_bundled() {
        let (a, b) = (unit(39.36, 3.05), unit(39.26, 3.2));
        let names: Vec<&str> = coves().within([a[0], a[1], b[0], b[1]]).map(|c| c.name.as_str()).collect();
        for n in ["Cala Figuereta", "Olla des Bastons", "Cala Llombards", "Caló des Màrmols"] {
            assert!(names.contains(&n), "{n} in {names:?}");
        }
        assert!(is_cove_name("Fort Pond Bay") && is_cove_name("Caló des Moro") && is_cove_name("Anse de Test"));
        assert!(!is_cove_name("Montauk Point") && !is_cove_name("Calanques"));
    }

    #[test]
    fn the_points_of_mallorca_named_as_capes_are_bundled_last() {
        let (a, b) = (unit(39.36, 3.05), unit(39.26, 3.2));
        let near: Vec<&Cape> = all().within([a[0], a[1], b[0], b[1]]).collect();
        for n in ["Punta de sa Cresta", "punta des Corb Marí", "Punta des Blanquer"] {
            let cape = near.iter().find(|c| c.name == n).unwrap_or_else(|| panic!("{n}"));
            assert_eq!(cape.rank, 3, "{n}");
        }
    }

    #[test]
    fn names_of_one_cape_in_two_languages_meet_at_their_stem() {
        assert_eq!(name_stem("Cabo Ortegal"), "ortegal");
        assert_eq!(name_stem("Cape Ortegal"), name_stem("Faro do cabo Ortegal"));
        assert_eq!(name_stem("Punta da Estaca de Bares"), name_stem("Punta de la Estaca de Bares"));
        assert_eq!(name_stem("Cap de Sant Sebastià"), "sant sebastia");
        assert_ne!(name_stem("Cabo Fisterra"), name_stem("Cabo Finisterre"));
        assert_eq!(cape_word("Cabo Fisterra").as_deref(), Some("cabo"));
        assert_eq!(cape_word("Pta. de la Estaca").as_deref(), Some("punta"));
        assert_eq!(cape_word("Cape Ortegal").as_deref(), Some("cape"));
        assert_eq!(cape_word("Sant Sebastià"), None);
    }

    #[test]
    fn capes_are_found_across_cells() {
        let c = |id, x, y| Cape { id, name: format!("{id}"), at: [x, y], rank: 2 };
        let capes = Capes::new(vec![c(0, 10.5, 20.5), c(1, 11.2, 20.1), c(2, 30.0, 30.0), c(3, 10.9, 21.5)]);
        let mut ids: Vec<u32> = capes.within([10.4, 20.0, 11.5, 21.6]).map(|c| c.id).collect();
        ids.sort();
        assert_eq!(ids, vec![0, 1, 3]);
        assert_eq!(capes.within([10.6, 20.0, 11.0, 21.0]).count(), 0);
    }
}
