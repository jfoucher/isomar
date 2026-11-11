//! Names of capes and points, and of coves, from the OpenStreetMap capes and coves bundled with the app
//! (`core::chart::capes`), written as a paper chart writes them: along the point, out on the sea from just off its
//! tip ("Punta Son Serralta"), and a cove's the same way from its mouth ("Cala Figuereta"). A cape with a
//! lighthouse is named as its light is, which is how charts name it.
//! The tiles of the towns layer say where the sea is, so a cape is named only where they are loaded; which
//! way each point runs is worked out once per tile zoom, and how its name sits once per label zoom.

use super::TileArea;
use super::names::{self, Measured, PathLabel, Style};
use super::aids::MajorLight;
use super::regions::cape_label;
use super::waters::tile_box;
use crate::core::chart::capes::{self, Cape};
use crate::core::chart::naturalearth::units_of;
use crate::core::chart::places::PlacesTile;
use crate::core::chart::vtile::TileId;
use crate::core::geo::{metres_per_pixel, project};
use crate::core::labels::coast::{CapeAxis, cape_axis, cove_axis};
use crate::core::vector::Pt;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

/// The label zoom from which the better known capes (those with a Wikidata entry) are named, and the rest.
const MIN_ZOOM_KNOWN: f64 = 10.0;
const MIN_ZOOM: f64 = 12.0;
/// The label zoom from which coves are named, all of them: they are small.
const MIN_ZOOM_COVE: f64 = 12.0;
/// A cape with a major light on it is named from the zoom its light shows at (`aids::Declutter`), as far out as
/// lights show at all: Cabo de São Vicente with its lighthouse. The light is this close to the cape's point
/// (metres): the lighthouse of Cap de la Hague is on a rock off it, Tarifa's on the island off the point.
const MIN_ZOOM_LIT: f64 = super::aids::MIN_ZOOM_LIGHTS;
const LIT_NEAR_M: f64 = 2500.0;
/// A light named after a cape this close to a cape (metres) is on it, whatever either is called: the light "Cabo
/// Finisterre" and OSM's Cabo Fisterra.
const LIGHT_ON_CAPE_M: f64 = 1000.0;
/// How far round a cape its point is looked at, in pixels.
const AXIS_RADIUS_PX: f64 = 40.0;
/// How far from its OSM node a cape's tip may be, in pixels: the node is on the point to a metre, so only as
/// far as the tiles' coast may be from it. Farther, Cap du Lazaret's tip went miles up the lido at zoom 8.
const TIP_REACH_PX: f64 = 6.0;
/// How far from its node the tip of a place named as a cape may be (rank 3: a locality, a point), in metres, up to
/// half the radius: OSM puts these on the land of the point, a little inland of the tip.
const LOCALITY_REACH_M: f64 = 150.0;
/// How far past the area a cape's name may reach into it from, in pixels.
const REACH_PX: f64 = 300.0;
/// How far from a cape another name can be in its way, in pixels: past its farthest step out and its longest name.
const NEAR_PX: f64 = 400.0;
/// Room kept between names, in pixels.
const GAP_PX: f64 = 4.0;
/// A Natural Earth name this close to a cape (units, about 5 km) and with its name is the same cape.
const SAME_CAPE_UNITS: f64 = 0.03;
/// A Natural Earth cape this close to one of the bundle (metres) with one of its names, or the same name with other
/// generic and joining words, is the same cape: Natural Earth's points are coarse (Cape Finisterre's is 5 km
/// north of the tip).
const NE_SAME_CAPE_M: f64 = 8000.0;
/// Points remembered, at most, before they are all forgotten.
const AXES_KEPT: usize = 20_000;
/// Labels have ids of their own, past those of the other names.
const ID_BASE: u64 = 1 << 52;

/// The style of a cape's name: land's ink, outlined in the sea's colour, as it is written on the water.
fn style() -> Style {
    Style::land(10.5, pango::Weight::Normal, 0.0).on_sea()
}

/// The style of a cove's name: a water's sloping blue, as big as a cape's.
fn cove_style() -> Style {
    Style::water(10.5)
}

/// How a chart writes a cape's name: in full, with a capital ("Punta de Son Serralta", "Cabo Finisterre"), without
/// the place OSM adds in brackets to tell it from another (`Cabo Carvoeiro (Lagoa)`).
pub fn chart_name(name: &str) -> String {
    let name = name.trim();
    let name = match name.rfind(" (") {
        Some(i) if i > 0 && name.ends_with(')') => name[..i].trim_end(),
        _ => name,
    };
    let mut c = name.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

/// The same with the generic word of the Romance languages cut short, as the charts of those coasts do where room
/// is short ("Pta. de Son Serralta", "C. Carvoeiro"): written only where the name in full does not fit.
pub fn short_name(name: &str) -> String {
    const SHORT: [(&str, &str); 4] = [("punta", "Pta."), ("ponta", "Pta."), ("pointe", "Pte"), ("cabo", "C.")];
    let name = chart_name(name);
    let (first, rest) = name.split_once(' ').unwrap_or((&name, ""));
    let lower = first.to_lowercase();
    match SHORT.iter().find(|(w, _)| *w == lower) {
        Some((_, short)) if !rest.is_empty() => format!("{short} {rest}"),
        _ => name,
    }
}

/// Words a light's name has round the place it is named after: "Faro de Cabo de Gata", "Start Point Lighthouse".
/// The longest of those that start alike first.
const LIGHT_PREFIXES: [&str; 28] = [
    "phare de la ", "phare de l'", "phare des ", "phare du ", "phare de ", "phare ", "faro de la ", "faro del ", "faro de ", "faro di ", "faro dos ",
    "faro das ", "faro do ", "faro da ", "faro ", "far de la ", "far del ", "far de ", "far ", "farol dos ", "farol das ", "farol da ", "farol do ",
    "farol de ", "farol ", "fanale di ", "leuchtturm ", "fyr ",
];
const LIGHT_SUFFIXES: [&str; 9] = [" lighthouse", " light", " lt", " phare", " faro", " leuchtturm", " fyr", " vuurtoren", " fyrtårn"];

/// The place a light is named after, lower case: "Faro de Cabo de Gata" is "cabo de gata", "Sant Sebastià"
/// "sant sebastià".
fn light_place(name: &str) -> Option<String> {
    let mut s = name.trim().to_lowercase().replace('’', "'");
    // "Cabo de Gata (faro)"
    if let Some(i) = s.find(" (") {
        s.truncate(i);
    }
    if let Some(p) = LIGHT_PREFIXES.iter().find(|p| s.starts_with(*p)) {
        s.drain(..p.len());
    }
    if let Some(x) = LIGHT_SUFFIXES.iter().find(|x| s.ends_with(*x)) {
        s.truncate(s.len() - x.len());
    }
    let s = s.trim();
    // A light named only for what it is names no place.
    let bare = LIGHT_PREFIXES.iter().any(|p| p.trim_end() == s) || LIGHT_SUFFIXES.iter().any(|x| x.trim_start() == s);
    (!bare && s.chars().filter(|c| c.is_alphabetic()).count() >= 3).then(|| s.to_string())
}

/// Whether `part` is in `whole` as whole words.
fn has_words(whole: &str, part: &str) -> bool {
    whole.match_indices(part).any(|(i, _)| {
        let before = whole[..i].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        let after = whole[i + part.len()..].chars().next().is_none_or(|c| !c.is_alphanumeric());
        before && after
    })
}

/// Ids of the capes written for a light that no cape in the bundle stands for, past every bundle id.
const LIGHT_CAPE_ID: u32 = 1 << 31;

/// What the lights say of the capes they stand on.
#[derive(Debug, Default)]
pub struct LitCapes {
    /// The capes the lights stand on.
    pub lit: HashSet<u32>,
    /// Capes to write for a light that names one the bundle does not have.
    pub extra: Vec<Cape>,
    /// The name a cape is written with where its light gives one: the light's name is the chart's.
    pub names: HashMap<u32, String>,
    /// The places the lights on a cape are named after, lower case: names it has in other languages, to tell
    /// its twin in Natural Earth by ("cabo finisterre" for OSM's "Cabo Fisterra").
    pub aka: HashMap<u32, Vec<String>>,
}

/// The capes that the lights at `lights` stand on (each with the names of what stands there), those to write
/// for a light that names a cape the bundle does not have, and the names the lights give them. For each light,
/// within `LIT_NEAR_M` of it: the nearest cape whose name holds the place one of the light's names is named
/// after, or names it with other generic and joining words (the Phare de Pertusato is Capo Pertusato's, Sant
/// Sebastià's light the Cap de Sant Sebastià's, the light "Cape Ortegal" Cabo Ortegal's); else, when that place
/// is a cape's name, the nearest cape within `LIGHT_ON_CAPE_M` (the light "Cabo Finisterre" is on Cabo
/// Fisterra), else a cape of that name at the light ("Start Point Lighthouse"); else the nearest cape, a better
/// known one (rank 1) counted as half as far, so that the light names one cape and not every point round it.
/// That nearest cape counts only when it is known or its name says it is one: the cove-side rock "la Figuera" by
/// Sant Sebastià's light is not what the light marks. A light named after a cape gives it its name, as charts
/// write it, where the generic word is the cape's own: "Cabo Finisterre" for Cabo Fisterra, but not "Cape
/// Ortegal" for Cabo Ortegal. A light the area does not show only names the cape it is on.
pub fn lit_capes(capes: &capes::Capes, lights: &[MajorLight]) -> LitCapes {
    let mut out = LitCapes::default();
    for MajorLight { at: l, names, shown } in lights {
        let (x, y) = project(l, 0.0);
        let mpp = metres_per_pixel(l.lat, 0.0);
        let r = LIT_NEAR_M / mpp;
        let near: Vec<(&Cape, f64)> =
            capes.within([x - r, y - r, x + r, y + r]).map(|c| (c, (c.at[0] - x).hypot(c.at[1] - y))).filter(|(_, d)| *d <= r).collect();
        fn nearest<'a>(v: &mut dyn Iterator<Item = (&'a Cape, f64)>) -> Option<&'a Cape> {
            v.min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.id.cmp(&b.0.id))).map(|(c, _)| c)
        }
        let eligible = |reach: f64| {
            near.iter().filter(move |(c, d)| *d <= reach && (c.rank == 1 || capes::is_cape_name(&c.name))).map(|(c, d)| (*c, if c.rank == 1 { d / 2.0 } else { *d }))
        };
        // Each name's place, lower case, and as the name writes it, less the lighthouse's words.
        let places: Vec<(String, String)> = names
            .iter()
            .filter_map(|full| {
                let place = light_place(full)?;
                let at = full.to_lowercase().find(place.as_str()).filter(|&i| full.is_char_boundary(i) && full.is_char_boundary(i + place.len()));
                let written = at.map_or_else(|| place.clone(), |i| full[i..i + place.len()].to_string());
                Some((place, written))
            })
            .collect();
        let of_cape: Vec<&(String, String)> = places.iter().filter(|(p, _)| capes::is_cape_name(p)).collect();
        let by_name = places.iter().find_map(|(place, _)| {
            let stem = capes::name_stem(place);
            nearest(&mut near.iter().copied().filter(|(c, _)| has_words(&c.name.to_lowercase().replace('’', "'"), place) || !stem.is_empty() && capes::name_stem(&c.name) == stem))
        });
        // The light's name for the cape, where its generic word is the cape's own.
        let light_name = |cape: &Cape| {
            let word = capes::cape_word(&cape.name)?;
            of_cape.iter().find(|(p, _)| capes::cape_word(p).as_ref() == Some(&word)).map(|(_, written)| chart_name(written))
        };
        let cape = by_name.or_else(|| if of_cape.is_empty() { None } else { nearest(&mut eligible(LIGHT_ON_CAPE_M / mpp)) });
        let id = match cape {
            Some(cape) if !shown => {
                if let Some(name) = light_name(cape) {
                    out.names.insert(cape.id, name);
                }
                out.aka.entry(cape.id).or_default().extend(of_cape.iter().map(|(p, _)| p.clone()));
                continue;
            }
            _ if !shown => continue,
            Some(cape) => {
                if let Some(name) = light_name(cape) {
                    out.names.insert(cape.id, name);
                }
                cape.id
            }
            None => match of_cape.first() {
                Some((_, written)) => {
                    let id = LIGHT_CAPE_ID | ((x * 4096.0) as u32 ^ ((y * 4096.0) as u32).rotate_left(16)) & !LIGHT_CAPE_ID;
                    out.extra.push(Cape { id, name: written.clone(), at: [x, y], rank: 1 });
                    id
                }
                None => {
                    if let Some(cape) = nearest(&mut eligible(r)) {
                        out.lit.insert(cape.id);
                    }
                    continue;
                }
            },
        };
        out.lit.insert(id);
        out.aka.entry(id).or_default().extend(of_cape.iter().map(|(p, _)| p.clone()));
    }
    out
}

/// Whether `p` (units) is on water, the sea or (with `lakes`) a lake, in the finest of `tiles` that holds it, and
/// that tile's zoom.
fn water_at(tiles: &[(TileId, Arc<PlacesTile>)], p: Pt, lakes: bool) -> Option<(bool, u8)> {
    let (id, tile) = tiles
        .iter()
        .filter(|(id, _)| {
            let b = tile_box(id);
            p[0] >= b[0] && p[0] < b[2] && p[1] >= b[1] && p[1] < b[3]
        })
        .max_by_key(|(id, _)| id.z)?;
    let (e, n) = (tile.extent.max(1) as f64, 2f64.powi(id.z as i32));
    let (x, y) = ((p[0] / 256.0 * n - id.x as f64) * e, (p[1] / 256.0 * n - id.y as f64) * e);
    Some((tile.on_sea(x, y) || lakes && tile.on_lake(x, y), id.z))
}

/// What a point's name names: a cape, written out from its tip, or a cove, written the same way from its mouth,
/// as a paper chart writes both ("Pta. de Sa Creta", "Cala Figuereta"). Capes first where they crowd.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Kind {
    Cape,
    Cove,
}

/// The key of a cape or a cove: coves' ids are apart from the capes'.
const COVE_KEY: u64 = 1 << 40;
/// The key of a name fitted again, round the names that won where it was first set.
const REFIT_KEY: u64 = 1 << 41;

fn key_of(kind: Kind, c: &Cape) -> u64 {
    c.id as u64 | if kind == Kind::Cove { COVE_KEY } else { 0 }
}

pub struct CapeNames {
    /// Each point's tip (a cove's mouth) and the way it points on the tiles of a zoom, `None` where it is not on
    /// their coast.
    axes: RefCell<HashMap<(u64, u8), Option<CapeAxis>>>,
    /// How each name sits at a label zoom and text size, once the tiles under it were all there, with a key of
    /// the other names near it that it went round.
    fits: RefCell<Option<((i64, u32), HashMap<u64, (u64, Option<Arc<PathLabel>>)>)>>,
}

impl Default for CapeNames {
    fn default() -> Self {
        Self::new()
    }
}

/// What `labels_for` writes: the capes' and coves' names, at the area's label zoom, and the Natural Earth names
/// they stand for (a cape Natural Earth names too), to leave out.
pub struct Written {
    pub labels: Vec<Arc<PathLabel>>,
    pub replaced: HashSet<u64>,
}

/// A hash of the names near a point that it went round, of its name and of its size.
fn near_key<'a>(names: impl Iterator<Item = &'a Arc<PathLabel>>, name: &str, size: f64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut hasher);
    names.fold(0u64, |h, l| h.rotate_left(7) ^ l.id ^ l.extent[0].to_bits().rotate_left(3) ^ l.extent[1].to_bits()) ^ size.to_bits() ^ hasher.finish()
}

impl CapeNames {
    pub fn new() -> CapeNames {
        CapeNames { axes: RefCell::new(HashMap::new()), fits: RefCell::new(None) }
    }

    /// Forgets how the names sat: what they keep clear of has changed.
    pub fn forget(&self) {
        *self.fits.borrow_mut() = None;
    }

    /// The names of the capes and coves that meet the area, set at its `names::label_zoom` in its world pixels (as
    /// `ne`, `taken` and `avoid` are), on the water of `tiles`: out of the way of the names in `ne` and `taken` (each
    /// is set where it is clear of them, as of what `avoid` says no to), and clear of each other, those with a major
    /// light first (it shows at `lights`, `lit_capes`), then by rank, a cape before a cove of the same rank. A name
    /// that a better one is in the way of is set again round the better ones that were kept, if it can be. A
    /// Natural Earth name of the same cape gives way to it; a cove `taken` already names along its water is left
    /// to it. A name is never bigger than the name of the island it is on, which `island_pt` gives (points, at the
    /// label zoom) for a point in units.
    #[allow(clippy::too_many_arguments)]
    pub fn labels_for(&self, area: &TileArea, tiles: &[(TileId, Arc<PlacesTile>)], lights: &[MajorLight], ne: &[Arc<PathLabel>], taken: &[Arc<PathLabel>], island_pt: &dyn Fn(Pt) -> Option<f64>, avoid: &dyn Fn(&[f64; 4]) -> bool, measure: &dyn Fn(&str, &Style) -> Rc<Measured>) -> Written {
        let mut written = Written { labels: Vec::new(), replaced: HashSet::new() };
        let vi = &area.vi;
        let zoom = names::label_zoom(vi.zoom());
        let ne_stems: Vec<Option<Vec<String>>> = ne
            .iter()
            .map(|l| {
                let is_cape = crate::core::chart::naturalearth::all().get((l.id >> 8) as usize).is_some_and(|g| g.kind == crate::core::chart::naturalearth::GeoKind::Cape);
                is_cape.then(|| l.aka.iter().map(|a| capes::name_stem(a)).filter(|s| !s.is_empty()).collect())
            })
            .collect();
        // Nothing is named before the lights show, or Natural Earth names a cape.
        let soonest = if lights.iter().any(|l| l.shown) || ne_stems.iter().any(Option::is_some) { MIN_ZOOM_LIT } else { MIN_ZOOM_KNOWN };
        if tiles.is_empty() || zoom < soonest {
            return written;
        }
        let LitCapes { lit, extra, names: lit_names, aka: lit_aka } = lit_capes(capes::all(), lights);
        // A cape is written as its light names it, where it does.
        fn name_in<'a>(names: &'a HashMap<u32, String>, c: &'a Cape) -> &'a str {
            names.get(&c.id).map_or(c.name.as_str(), String::as_str)
        }
        let name_of = |c| name_in(&lit_names, c);
        let started = crate::debug::timing().then(std::time::Instant::now);
        let ppu = 2f64.powf(zoom);
        let tile_z = TileId::zoom_for(vi.zoom());
        let region = units_of(&vi.bounds());
        let by = REACH_PX / vi.ppu();
        let wanted = [region[0] - by, region[1] - by, region[2] + by, region[3] + by];
        let level = ((zoom * 100.0).round() as i64, super::text_scale());
        let mut fits = self.fits.borrow_mut();
        if fits.as_ref().is_none_or(|(l, _)| *l != level) {
            *fits = Some((level, HashMap::new()));
        }
        let memo = &mut fits.as_mut().unwrap().1;
        if self.axes.borrow().len() > AXES_KEPT {
            self.axes.borrow_mut().clear();
        }
        let is_lit = |kind: Kind, c: &Cape| kind == Kind::Cape && lit.contains(&c.id);
        // A cove big enough to be named along its water already is.
        let named_along = |cape: &Cape| {
            let key = cape.name.trim().to_lowercase();
            let u = SAME_CAPE_UNITS;
            taken.iter().any(|l| l.aka.contains(&key) && cape.at[0] >= l.units[0] - u && cape.at[0] <= l.units[2] + u && cape.at[1] >= l.units[1] - u && cape.at[1] <= l.units[3] + u)
        };
        let meets = |x: &[f64; 4], b: &PathLabel| {
            let x = [x[0] - GAP_PX, x[1] - GAP_PX, x[2] + GAP_PX, x[3] + GAP_PX];
            b.boxes.iter().any(|y| x[0] < y[2] && x[2] > y[0] && x[1] < y[3] && x[3] > y[1])
        };
        let hits = |a: &PathLabel, b: &PathLabel| a.boxes.iter().any(|x| meets(x, b));
        // As small as an island's name may be, but no smaller.
        let least = super::regions::ISLAND_MIN_PX / (super::text_scale() as f64 / 100.0);
        let style_of = |kind: Kind, cape: &Cape| {
            let st = match kind {
                Kind::Cape => style(),
                Kind::Cove => cove_style(),
            };
            Style { size: island_pt(cape.at).map_or(st.size, |pt| st.size.min(pt.max(least))), ..st }
        };
        let min_zoom = |kind: Kind, c: &Cape| match kind {
            Kind::Cove => MIN_ZOOM_COVE,
            _ if lit.contains(&c.id) => MIN_ZOOM_LIT,
            _ if c.rank == 1 => MIN_ZOOM_KNOWN,
            _ => MIN_ZOOM,
        };
        // The Natural Earth names of the same cape, which give way to it; the others are in the way. Natural
        // Earth's capes are told within `NE_SAME_CAPE_M` by any of the cape's names, its lights' too, or by its
        // stem ("Cape Ortegal" is Cabo Ortegal); its other names within `SAME_CAPE_UNITS` by the same name.
        let names_of = |cape: &Cape| -> Vec<String> {
            std::iter::once(cape.name.as_str())
                .chain(lit_names.get(&cape.id).map(String::as_str))
                .chain(lit_aka.get(&cape.id).into_iter().flatten().map(String::as_str))
                .map(|n| n.trim().to_lowercase())
                .collect()
        };
        let apart = |l: &PathLabel, cape: &Cape| (l.units[0] - cape.at[0]).abs().max((l.units[1] - cape.at[1]).abs());
        let far = |cape: &Cape| NE_SAME_CAPE_M / metres_per_pixel(crate::core::geo::unproject(cape.at[0], cape.at[1], 0.0).lat, 0.0);
        let twins_of = |cape: &Cape, ne_capes_only: bool| -> Vec<u64> {
            let names = names_of(cape);
            let stems: Vec<String> = names.iter().map(|n| capes::name_stem(n)).filter(|s| !s.is_empty()).collect();
            let far = far(cape);
            ne.iter()
                .zip(&ne_stems)
                .filter(|(l, ne_stems)| {
                    let d = apart(l, cape);
                    let named = names.iter().any(|n| l.aka.contains(n));
                    !ne_capes_only && named && d < SAME_CAPE_UNITS || ne_stems.as_ref().is_some_and(|s| d < far && (named || stems.iter().any(|x| s.contains(x))))
                })
                .map(|(l, _)| l.id)
                .collect()
        };
        // A cape Natural Earth names is written from the zoom Natural Earth writes it, in its place: the bundle's
        // point is on the tip, Natural Earth's coarse. The best known of its twins, then the nearest.
        let mut twins: HashMap<u64, Vec<u64>> = HashMap::new();
        let mut stand_ins: HashMap<u64, (&Cape, (u8, f64))> = HashMap::new();
        for cape in capes::all().within(wanted).chain(&extra) {
            if zoom >= min_zoom(Kind::Cape, cape) {
                twins.insert(key_of(Kind::Cape, cape), twins_of(cape, false));
            } else if ne.iter().zip(&ne_stems).any(|(l, s)| s.is_some() && apart(l, cape) < far(cape)) {
                for id in twins_of(cape, true) {
                    let d = ne.iter().find(|l| l.id == id).map_or(f64::MAX, |l| apart(l, cape));
                    let better = |(c, (rank, d0)): &(&Cape, (u8, f64))| (cape.rank, d) < (*rank, *d0) || cape.rank == *rank && d == *d0 && cape.id < c.id;
                    if stand_ins.get(&id).is_none_or(better) {
                        stand_ins.insert(id, (cape, (cape.rank, d)));
                    }
                }
            }
        }
        let due: HashSet<u64> = twins.values().flatten().copied().collect();
        let mut points: Vec<(Kind, &Cape)> = Vec::new();
        for (id, (cape, _)) in stand_ins.into_iter().filter(|(id, _)| !due.contains(id)) {
            let key = key_of(Kind::Cape, cape);
            if !twins.contains_key(&key) {
                points.push((Kind::Cape, cape));
            }
            twins.entry(key).or_default().push(id);
        }
        points.extend(capes::all().within(wanted).chain(&extra).filter(|c| zoom >= min_zoom(Kind::Cape, c)).map(|c| (Kind::Cape, c)));
        points.extend(capes::coves().within(wanted).filter(|c| zoom >= MIN_ZOOM_COVE && !named_along(c)).map(|c| (Kind::Cove, c)));
        // In the same order whatever the map's: the collisions are settled by priority, but the memo of fits is not.
        points.sort_by_key(|(kind, c)| key_of(*kind, c));
        let twin = |kind: Kind, cape: &Cape, l: &PathLabel| twins.get(&key_of(kind, cape)).is_some_and(|ids| ids.contains(&l.id));
        // Natural Earth's name gives way to the cape's wherever that is due, written or not: its point is coarse.
        written.replaced.extend(twins.values().flatten());
        let near_to = |cape: &Cape, l: &PathLabel| {
            let (cx, cy) = (cape.at[0] * ppu, cape.at[1] * ppu);
            l.extent[0] < cx + NEAR_PX && l.extent[2] > cx - NEAR_PX && l.extent[1] < cy + NEAR_PX && l.extent[3] > cy - NEAR_PX
        };
        let mut fitted: Vec<(Kind, &Cape, Arc<PathLabel>)> = Vec::new();
        for &(kind, cape) in &points {
            // The names near enough to be in its way: where those move, it is set again.
            let style = style_of(kind, cape);
            let key = near_key(ne.iter().filter(|n| !twin(kind, cape, n)).chain(taken).filter(|l| near_to(cape, l)), name_of(cape), style.size);
            let label = match memo.get(&key_of(kind, cape)) {
                Some((k, l)) if *k == key => l.clone(),
                _ => {
                    let clear = |b: &[f64; 4]| avoid(b) || ne.iter().any(|n| !twin(kind, cape, n) && meets(b, n)) || taken.iter().any(|t| meets(b, t));
                    let (label, settled) = self.fit(kind, cape, name_of(cape), tiles, tile_z, ppu, zoom, &style, &clear, measure);
                    if settled {
                        memo.insert(key_of(kind, cape), (key, label.clone()));
                    }
                    label
                }
            };
            if let Some(l) = label {
                fitted.push((kind, cape, l));
            }
        }
        let blocked = |kind: Kind, cape: &Cape, l: &PathLabel| ne.iter().any(|n| !twin(kind, cape, n) && hits(l, n)) || taken.iter().any(|t| hits(l, t));
        let priority = |kind: Kind, c: &Cape| (!is_lit(kind, c), c.rank, kind, c.id);
        let free: Vec<bool> = fitted.iter().map(|(k, c, l)| !blocked(*k, c, l)).collect();
        let won: Vec<bool> = fitted
            .iter()
            .zip(&free)
            .map(|((k, c, l), ok)| *ok && !fitted.iter().zip(&free).any(|((k2, c2, l2), f2)| *f2 && priority(*k2, c2) < priority(*k, c) && hits(l, l2)))
            .collect();
        // Those a better name was in the way of, set again round the names that won, as near the point as is clear;
        // of those, a better one wins again.
        let winners: Vec<Arc<PathLabel>> = fitted.iter().zip(&won).filter(|(_, w)| **w).map(|((_, _, l), _)| l.clone()).collect();
        let mut again: Vec<(Kind, &Cape, Arc<PathLabel>)> = Vec::new();
        for ((kind, cape, _), _) in fitted.iter().zip(&won).zip(&free).filter(|((_, w), f)| !**w && **f).map(|(x, _)| x) {
            let style = style_of(*kind, cape);
            let near: Vec<&Arc<PathLabel>> = ne.iter().filter(|n| !twin(*kind, cape, n)).chain(taken).chain(&winners).filter(|l| near_to(cape, l)).collect();
            let key = near_key(near.iter().copied(), name_of(cape), style.size);
            let label = match memo.get(&(key_of(*kind, cape) | REFIT_KEY)) {
                Some((k, l)) if *k == key => l.clone(),
                _ => {
                    let clear = |b: &[f64; 4]| avoid(b) || near.iter().any(|n| meets(b, n));
                    let (label, settled) = self.fit(*kind, cape, name_of(cape), tiles, tile_z, ppu, zoom, &style, &clear, measure);
                    if settled {
                        memo.insert(key_of(*kind, cape) | REFIT_KEY, (key, label.clone()));
                    }
                    label
                }
            };
            if let Some(l) = label {
                again.push((*kind, cape, l));
            }
        }
        let kept = fitted.iter().zip(&won).filter(|(_, w)| **w).map(|(x, _)| x);
        let kept_again = again.iter().filter(|(k, c, l)| !again.iter().any(|(k2, c2, l2)| priority(*k2, c2) < priority(*k, c) && hits(l, l2)));
        written.labels.extend(kept.chain(kept_again).map(|(_, _, l)| l.clone()));
        if let Some(t) = started {
            isomar_core::log!("isomar: cape and cove names {} fitted, {} set again, {} shown, {:.2} ms", fitted.len(), again.len(), written.labels.len(), t.elapsed().as_secs_f64() * 1000.0);
        }
        written
    }

    /// How a cape's or a cove's name sits, and whether that is settled: every bit of water it looked at was in a
    /// tile of the view's zoom, so it will not change until the zoom does.
    #[allow(clippy::too_many_arguments)]
    fn fit(&self, kind: Kind, cape: &Cape, name: &str, tiles: &[(TileId, Arc<PlacesTile>)], tile_z: u8, ppu: f64, zoom: f64, style: &Style, avoid: &dyn Fn(&[f64; 4]) -> bool, measure: &dyn Fn(&str, &Style) -> Rc<Measured>) -> (Option<Arc<PathLabel>>, bool) {
        let settled = Cell::new(true);
        // A cove is on the sea: a bay of a lake is not named as one.
        let wet = |p: Pt| -> Option<bool> {
            let got = water_at(tiles, p, kind == Kind::Cape);
            if got.is_none_or(|(_, z)| z < tile_z) {
                settled.set(false);
            }
            got.map(|(w, _)| w)
        };
        let key = (key_of(kind, cape), tile_z);
        let known = self.axes.borrow().get(&key).copied();
        let axis = match known {
            Some(a) => a,
            None => {
                let radius = AXIS_RADIUS_PX / ppu;
                let a = match kind {
                    Kind::Cove => cove_axis(cape.at, &|p| wet(p) != Some(false), radius),
                    Kind::Cape => {
                        // A place named as a cape is not on its tip as a cape's node is: a little way inland, mostly.
                        let reach = if cape.rank == 3 {
                            let lat = crate::core::geo::unproject(cape.at[0], cape.at[1], 0.0).lat;
                            (LOCALITY_REACH_M / metres_per_pixel(lat, 0.0)).clamp(TIP_REACH_PX / ppu, radius / 2.0)
                        } else {
                            TIP_REACH_PX / ppu
                        };
                        cape_axis(cape.at, &|p| wet(p) != Some(false), radius, reach)
                    }
                };
                if settled.get() {
                    self.axes.borrow_mut().insert(key, a);
                }
                a
            }
        };
        let Some(axis) = axis else { return (None, settled.get()) };
        // In full, else with its generic word cut short where that fits.
        let (full, short) = (chart_name(name), short_name(name));
        let aka = vec![cape.name.trim().to_lowercase(), name.trim().to_lowercase(), full.to_lowercase(), short.to_lowercase()];
        let units = [cape.at[0], cape.at[1], cape.at[0], cape.at[1]];
        let id = ID_BASE | key_of(kind, cape);
        let texts = if short == full { vec![full] } else { vec![full, short] };
        let label = texts.iter().find_map(|text| {
            let m = measure(text, style);
            cape_label(id, text, &m, *style, &axis, ppu, zoom, aka.clone(), units, &|p| wet([p[0] / ppu, p[1] / ppu]), avoid)
        });
        (label.map(Arc::new), settled.get())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::geo::LatLon;

    fn shown(at: LatLon, names: Vec<String>) -> MajorLight {
        MajorLight { at, names, shown: true }
    }

    #[test]
    fn a_cape_is_named_as_the_chart_of_its_coast_names_it() {
        assert_eq!(chart_name("punta de Son Serralta"), "Punta de Son Serralta");
        assert_eq!(chart_name("na Galinda"), "Na Galinda");
        assert_eq!(chart_name("Cabo Carvoeiro (Lagoa)"), "Cabo Carvoeiro");
        // Cut short only where the name in full does not fit.
        assert_eq!(short_name("punta de Son Serralta"), "Pta. de Son Serralta");
        assert_eq!(short_name("Pointe du Raz"), "Pte du Raz");
        assert_eq!(short_name("Montauk Point"), "Montauk Point");
        assert_eq!(short_name("Punta"), "Punta");
        assert_eq!(short_name("Cap Gros"), "Cap Gros");
        assert_eq!(short_name("Cabo Carvoeiro (Lagoa)"), "C. Carvoeiro");
    }

    #[test]
    fn a_light_names_the_cape_it_stands_on_and_no_other() {
        let unit = |lat: f64, lon: f64| {
            let (x, y) = project(&LatLon::new(lat, lon), 0.0);
            [x, y]
        };
        let cape = |id, name: &str, lat, lon, rank| Cape { id, name: name.into(), at: unit(lat, lon), rank };
        let capes = capes::Capes::new(vec![
            // About 300 m, 1.2 km and 1.6 km from the light, and one 4 km off.
            cape(0, "la Figuera", 41.8975, 3.2045, 2),
            cape(1, "Punta d'en Blanc", 41.8940, 3.1890, 2),
            cape(2, "Cap de Sant Sebastià", 41.8960, 3.1830, 1),
            cape(3, "Cap Roig", 41.8700, 3.1700, 2),
        ]);
        let light = LatLon::new(41.8960, 3.2025);
        let names = |name: Option<&str>| name.map(|n| vec![n.to_string()]).unwrap_or_default();
        let ids = |capes: &capes::Capes, name: Option<&str>| lit_capes(capes, &[shown(light, names(name))]).lit;
        // The known cape wins over a nearer point; the rock with no cape word never counts.
        assert_eq!(ids(&capes, None), HashSet::from([2]));
        let unknown = capes::Capes::new(capes.within([0.0, 0.0, 256.0, 256.0]).filter(|c| c.id != 2).cloned().collect());
        assert_eq!(ids(&unknown, None), HashSet::from([1]));
        assert!(lit_capes(&capes, &[shown(LatLon::new(41.80, 3.10), vec![])]).lit.is_empty(), "no cape within reach");
        // The light's name says which: the place it is named after, in a cape's name.
        assert_eq!(ids(&unknown, Some("Far de Punta d’en Blanc")), HashSet::from([1]));
        assert_eq!(ids(&capes, Some("Sant Sebastià")), HashSet::from([2]));
        // A cape the bundle does not have is written at the light, as the light names it.
        let got = lit_capes(&unknown, &[shown(light, names(Some("Far del Cap de Sant Sebastià")))]);
        assert_eq!(got.extra.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Cap de Sant Sebastià"]);
        assert_eq!(got.lit, HashSet::from([got.extra[0].id]));
        assert!(got.extra[0].id >= LIGHT_CAPE_ID && got.extra[0].rank == 1);
        // A light named after no cape falls back to the nearest.
        assert_eq!(ids(&unknown, Some("Goury")), HashSet::from([1]));
    }

    #[test]
    fn a_light_with_another_name_for_its_cape_names_it_once_as_the_chart_does() {
        let unit = |lat: f64, lon: f64| {
            let (x, y) = project(&LatLon::new(lat, lon), 0.0);
            [x, y]
        };
        let cape = |id, name: &str, lat, lon, rank| Cape { id, name: name.into(), at: unit(lat, lon), rank };
        let capes = capes::Capes::new(vec![
            cape(0, "Cabo Ortegal", 43.7716, -7.8700, 1),
            cape(1, "Cabo Fisterra", 42.8800, -9.2722, 1),
            cape(2, "Punta da Estaca de Bares", 43.7900, -7.6870, 1),
            cape(3, "Cabo da Nave", 42.9125, -9.2860, 1),
        ]);
        let ortegal = LatLon::new(43.7708, -7.8694);
        let fisterra = LatLon::new(42.8822, -9.2717);
        let bares = LatLon::new(43.7862, -7.6848);
        let got = lit_capes(
            &capes,
            &[
                shown(ortegal, vec!["Cape Ortegal".into(), "Faro do cabo Ortegal".into()]),
                shown(fisterra, vec!["Cape Finisterre Lighthouse".into(), "Cabo Finisterre".into()]),
                shown(bares, vec!["Punta de la Estaca de Bares".into()]),
            ],
        );
        // Each light is on the bundle's cape: none is written a second time at the light.
        assert!(got.extra.is_empty(), "{:?}", got.extra);
        assert_eq!(got.lit, HashSet::from([0, 1, 2]));
        // Written as the light names it, where the generic word is the cape's own.
        assert_eq!(got.names.get(&0).map(String::as_str), Some("Cabo Ortegal"));
        assert_eq!(got.names.get(&1).map(String::as_str), Some("Cabo Finisterre"));
        assert_eq!(got.names.get(&2).map(String::as_str), Some("Punta de la Estaca de Bares"));
        // The light's names in other languages are the cape's too, for Natural Earth's "Cape Finisterre".
        assert!(got.aka[&1].contains(&"cape finisterre".to_string()) && got.aka[&1].contains(&"cabo finisterre".to_string()));
        // An English name alone is not written for a Galician cape.
        let english = lit_capes(&capes, &[shown(ortegal, vec!["Cape Ortegal".into()])]);
        assert!(english.extra.is_empty() && english.lit == HashSet::from([0]) && english.names.is_empty());
        // A light the area does not show names its cape still, but does not light it nor stand for one.
        let hidden = MajorLight { at: fisterra, names: vec!["Cabo Finisterre".into()], shown: false };
        let got = lit_capes(&capes, std::slice::from_ref(&hidden));
        assert!(got.lit.is_empty() && got.extra.is_empty());
        assert_eq!(got.names.get(&1).map(String::as_str), Some("Cabo Finisterre"));
        let nowhere = capes::Capes::new(vec![]);
        let got = lit_capes(&nowhere, &[hidden]);
        assert!(got.lit.is_empty() && got.extra.is_empty() && got.names.is_empty());
    }

    #[test]
    fn a_light_is_named_after_its_place() {
        assert_eq!(light_place("Faro de Cabo de Gata").as_deref(), Some("cabo de gata"));
        assert_eq!(light_place("Start Point Lighthouse").as_deref(), Some("start point"));
        assert_eq!(light_place("Phare de Pertusato").as_deref(), Some("pertusato"));
        assert_eq!(light_place("Beachy Head Lighthouse").as_deref(), Some("beachy head"));
        assert_eq!(light_place("Sant Sebastià").as_deref(), Some("sant sebastià"));
        assert_eq!(light_place("Phare de l’Île Vierge").as_deref(), Some("île vierge"));
        assert_eq!(light_place("Fyr"), None);
        assert_eq!(light_place("Faro do cabo Ortegal").as_deref(), Some("cabo ortegal"));
        assert!(has_words("capo pertusato", "pertusato") && !has_words("capo pertusatone", "pertusato"));
    }
}
