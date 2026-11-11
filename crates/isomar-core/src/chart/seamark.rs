//! OpenStreetMap seamarks (`seamark:*` tags, the OpenSeaMap scheme) as aids to
//! navigation.
//!
//! The marks come from the `seamark` layer of the Open Waters vector tiles
//! (see `vtile`), which keeps the OSM tags. Parsing maps them onto the same
//! `Aid`/`Light` structs the SHOM GeoPackage fills, using S-57 codes.
//!
//! OSM data is © OpenStreetMap contributors under the ODbL. It is volunteer
//! made, so it has gaps and stale positions: planning only.

use crate::chart::aids::{Aid, AidKind, AidsData, Anchorage, AnchorageKind, Light};
use crate::chart::vtile::{GeomKind, TileId, VectorTile};
use crate::geo::LatLon;
use crate::vector::{Bounds, clip_ring, label_point, on_clip_edge};
use std::collections::HashMap;

// ---- tags to S-57 --------------------------------------------------------

fn colour_code(name: &str) -> Option<u8> {
    Some(match name.trim() {
        "white" => 1,
        "black" => 2,
        "red" => 3,
        "green" => 4,
        "blue" => 5,
        "yellow" => 6,
        "grey" | "gray" => 7,
        "brown" => 8,
        "amber" => 9,
        "violet" => 10,
        "orange" => 11,
        "magenta" => 12,
        "pink" => 13,
        _ => return None,
    })
}

/// `"red;white;red"` to S-57 COLOUR codes.
fn colours(v: Option<&str>) -> Vec<u8> {
    v.map(|s| s.split(';').filter_map(colour_code).collect()).unwrap_or_default()
}

fn buoy_shape(s: &str) -> Option<i32> {
    Some(match s.trim() {
        "conical" => 1,
        "can" | "cylindrical" => 2,
        "spherical" => 3,
        "pillar" => 4,
        "spar" => 5,
        "barrel" => 6,
        "super-buoy" => 7,
        "ice-buoy" => 8,
        _ => return None,
    })
}

fn beacon_shape(s: &str) -> Option<i32> {
    Some(match s.trim() {
        "stake" | "pole" | "perch" => 1,
        "whisp" | "withy" => 2,
        "tower" => 3,
        "lattice" => 4,
        "pile" => 5,
        "cairn" => 6,
        "buoyant" => 7,
        _ => return None,
    })
}

/// CATLAM or CATCAM from the OSM category word; other kinds have none here.
fn category(kind: AidKind, word: &str) -> Option<i32> {
    match kind {
        AidKind::BuoyLateral | AidKind::BeaconLateral => match word.trim() {
            "port" => Some(1),
            "starboard" => Some(2),
            "preferred_channel_starboard" => Some(3),
            "preferred_channel_port" => Some(4),
            _ => None,
        },
        AidKind::BuoyCardinal | AidKind::BeaconCardinal => match word.trim() {
            "north" => Some(1),
            "east" => Some(2),
            "south" => Some(3),
            "west" => Some(4),
            _ => None,
        },
        _ => None,
    }
}

/// The leading number of `"12"`, `"2.5"` or `"12 s"`.
fn number(s: &str) -> Option<f64> {
    let t = s.trim().replace(',', ".");
    let end = t.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).unwrap_or(t.len());
    t[..end].parse().ok()
}

/// LITCHR code and any group written into the character, as in `Fl(2)`.
fn character(s: &str) -> (Option<i32>, Option<String>) {
    let clean: String = s.chars().filter(|c| !c.is_whitespace() && *c != '.').collect();
    let (base, group) = match clean.find('(') {
        Some(i) if clean.ends_with(')') => (&clean[..i], Some(clean[i..].to_string())),
        _ => (clean.as_str(), None),
    };
    let code = match base {
        "F" => 1,
        "Fl" => 2,
        "LFl" => 3,
        "Q" => 4,
        "VQ" => 5,
        "UQ" => 6,
        "Iso" => 7,
        "Oc" => 8,
        "IQ" => 9,
        "IVQ" => 10,
        "IUQ" => 11,
        "Mo" => 12,
        "FFl" => 13,
        "Fl+LFl" => 14,
        "OcFl" => 15,
        "FLFl" => 16,
        "AlOc" => 17,
        "AlLFl" => 18,
        "AlFl" => 19,
        "Al" => 20,
        "Q+LFl" => 25,
        "VQ+LFl" => 26,
        "UQ+LFl" => 27,
        _ => return (None, group),
    };
    (Some(code), group)
}

/// SIGGRP as printed on charts: `3` becomes `(3)`.
fn group(s: &str) -> Option<String> {
    let t = s.trim();
    match t {
        "" => None,
        _ if t.starts_with('(') => Some(t.to_string()),
        _ => Some(format!("({t})")),
    }
}

type Tags = HashMap<String, String>;

fn tag<'a>(tags: &'a Tags, key: &str) -> Option<&'a str> {
    tags.get(key).map(String::as_str)
}

/// Every light on an element: the plain `seamark:light:*` tags, and the
/// numbered `seamark:light:1:*`, `:2:*`, ... of a sector light.
fn lights_of(tags: &Tags, pos: LatLon) -> Vec<Light> {
    let prefixes = std::iter::once("seamark:light:".to_string()).chain((1..=9).map(|n| format!("seamark:light:{n}:")));
    let mut out = Vec::new();
    for p in prefixes {
        let get = |k: &str| tag(tags, &format!("{p}{k}"));
        let (code, embedded) = get("character").map(character).unwrap_or((None, None));
        let colours = colours(get("colour"));
        let sector = get("sector_start").and_then(number).zip(get("sector_end").and_then(number));
        if code.is_none() && colours.is_empty() && sector.is_none() {
            continue;
        }
        out.push(Light {
            pos,
            colours,
            character: code,
            group: get("group").and_then(group).or(embedded),
            period_s: get("period").and_then(number),
            range_nm: get("range").and_then(number),
            sector,
            height_m: get("height").and_then(number),
        });
    }
    out
}

fn aid_kind(seamark_type: &str) -> Option<AidKind> {
    Some(match seamark_type {
        "buoy_lateral" => AidKind::BuoyLateral,
        "buoy_cardinal" => AidKind::BuoyCardinal,
        "buoy_isolated_danger" => AidKind::BuoyIsolatedDanger,
        "buoy_safe_water" => AidKind::BuoySafeWater,
        "buoy_installation" => AidKind::BuoyInstallation,
        "beacon_lateral" => AidKind::BeaconLateral,
        "beacon_cardinal" => AidKind::BeaconCardinal,
        "beacon_isolated_danger" => AidKind::BeaconIsolatedDanger,
        "beacon_safe_water" => AidKind::BeaconSafeWater,
        "landmark" => AidKind::Landmark,
        t if t.starts_with("buoy_") => AidKind::BuoySpecial,
        t if t.starts_with("beacon_") => AidKind::BeaconSpecial,
        _ => return None,
    })
}

/// Aids and lights from tagged positions.
fn aids_from_elements<'a>(elements: impl Iterator<Item = (LatLon, &'a Tags)>) -> AidsData {
    let mut data = AidsData::default();
    for (pos, tags) in elements {
        data.lights.extend(lights_of(tags, pos));
        // The tiles name the type twice; lighthouses may carry only the short one.
        let st = tag(tags, "seamark:type").or_else(|| tag(tags, "type"));
        // A lighthouse is a landmark with its light on it, whether or not it is tagged as a seamark.
        if st == Some("light_major") || tag(tags, "man_made") == Some("lighthouse") {
            data.aids.push(Aid {
                pos,
                kind: AidKind::Landmark,
                name: tag(tags, "seamark:name").or_else(|| tag(tags, "name")).map(str::to_string),
                colours: Vec::new(),
                shape: None,
                category: None,
            });
            continue;
        }
        let Some(st) = st else { continue };
        let Some(kind) = aid_kind(st) else { continue };
        let get = |k: &str| tag(tags, &format!("seamark:{st}:{k}"));
        data.aids.push(Aid {
            pos,
            kind,
            name: tag(tags, "seamark:name").or_else(|| tag(tags, "name")).map(str::to_string),
            colours: colours(get("colour")),
            shape: get("shape").and_then(if kind.is_buoy() { buoy_shape } else { beacon_shape }),
            category: get("category").and_then(|c| category(kind, c)),
        });
    }
    data
}

/// The aids, lights and anchorages in a tile's `seamark` layer.
pub fn aids_from_tile(id: &TileId, tile: &VectorTile) -> AidsData {
    let Some(layer) = tile.layer("seamark") else { return AidsData::default() };
    let extent = layer.extent as i32;
    let at = layer.features.iter().filter(|f| f.kind == GeomKind::Point).flat_map(|f| {
        f.paths
            .iter()
            .filter_map(|p| p.first())
            // Points in the tile's buffer are also in its neighbour.
            .filter(move |p| (0..extent).contains(&p[0]) && (0..extent).contains(&p[1]))
            .map(move |p| (id.point(p[0] as f64, p[1] as f64, extent as f64), &f.tags))
    });
    let mut data = aids_from_elements(at);
    data.anchorages = anchorages_of(id, layer);
    data
}

/// What an element says of anchoring, and its anchorage categories as CATACH codes.
fn anchorage_kind(tags: &Tags) -> Option<(AnchorageKind, Vec<u8>)> {
    let st = tag(tags, "seamark:type").or_else(|| tag(tags, "type"))?;
    let words = |key: &str| tag(tags, key).into_iter().flat_map(|v| v.split([';', ','])).map(str::trim).collect::<Vec<_>>();
    match st {
        "anchorage" | "anchor_berth" | "anchor berth" => {
            let mut cats: Vec<u8> = words("seamark:anchorage:category").into_iter().chain(words("seamark:anchor_berth:category")).filter_map(catach).collect();
            cats.dedup();
            Some((if st == "anchorage" { AnchorageKind::Anchorage } else { AnchorageKind::Berth }, cats))
        }
        "restricted_area" => {
            let r = words("seamark:restricted_area:restriction");
            if r.contains(&"no_anchoring") {
                Some((AnchorageKind::Prohibited, Vec::new()))
            } else if r.contains(&"restricted_anchoring") {
                Some((AnchorageKind::Restricted, Vec::new()))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// The pieces of a line within the tile's own square, 0 to `e`, in tile units.
fn clip_line(pts: &[[f64; 2]], e: f64) -> Vec<Vec<[f64; 2]>> {
    let mut out: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut run: Vec<[f64; 2]> = Vec::new();
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let d = [b[0] - a[0], b[1] - a[1]];
        // Liang-Barsky: the part of the segment inside, as t0..t1.
        let (mut t0, mut t1) = (0.0f64, 1.0f64);
        let mut inside = true;
        for (p, q) in [(-d[0], a[0]), (d[0], e - a[0]), (-d[1], a[1]), (d[1], e - a[1])] {
            if p == 0.0 {
                inside &= q >= 0.0;
            } else if p < 0.0 {
                t0 = t0.max(q / p);
            } else {
                t1 = t1.min(q / p);
            }
        }
        if !inside || t0 > t1 {
            if run.len() >= 2 {
                out.push(std::mem::take(&mut run));
            }
            run.clear();
            continue;
        }
        let at = |t: f64| [a[0] + d[0] * t, a[1] + d[1] * t];
        if run.is_empty() || t0 > 0.0 {
            if run.len() >= 2 {
                out.push(std::mem::take(&mut run));
            }
            run = vec![at(t0)];
        }
        run.push(at(t1));
        if t1 < 1.0 {
            out.push(std::mem::take(&mut run));
        }
    }
    if run.len() >= 2 {
        out.push(run);
    }
    out
}

/// An OpenSeaMap anchorage category to its CATACH code.
fn catach(word: &str) -> Option<u8> {
    Some(match word {
        "unrestricted" => 1,
        "deep_water" => 2,
        "tanker" => 3,
        "explosives" => 4,
        "quarantine" => 5,
        "seaplane" | "sea-plane" => 6,
        "small_craft" | "small_boat_anchorage" => 7,
        "small_craft_mooring" => 8,
        "24_hour" => 9,
        _ => return None,
    })
}

/// The anchorages of a tile's `seamark` layer: its points, and its areas cut to the tile, so that each
/// tile draws its own piece. An area's symbol goes inside its piece, its outline leaves out the cut.
fn anchorages_of(id: &TileId, layer: &crate::chart::vtile::Layer) -> Vec<Anchorage> {
    let e = layer.extent as f64;
    let tile = Bounds { min_lon: 0.0, min_lat: 0.0, max_lon: e, max_lat: e };
    let to_ll = |p: [f64; 2]| {
        let ll = id.point(p[0], p[1], e);
        [ll.lon, ll.lat]
    };
    let mut out = Vec::new();
    for f in &layer.features {
        let Some((kind, categories)) = anchorage_kind(&f.tags) else { continue };
        let name = tag(&f.tags, "seamark:name").or_else(|| tag(&f.tags, "name")).map(str::to_string);
        let radius_m = tag(&f.tags, "seamark:anchor_berth:radius").and_then(number);
        match f.kind {
            GeomKind::Point => {
                for p in f.paths.iter().filter_map(|p| p.first()).filter(|p| (0..e as i32).contains(&p[0]) && (0..e as i32).contains(&p[1])) {
                    let pos = id.point(p[0] as f64, p[1] as f64, e);
                    out.push(Anchorage { kind, name: name.clone(), categories: categories.clone(), pos, rings: Vec::new(), edges: Vec::new(), radius_m });
                }
            }
            // A closed way can come as a line: those that close are areas.
            GeomKind::Line if f.paths.iter().any(|p| p.len() < 4 || p.first() != p.last()) => {
                let edges: Vec<Vec<[f64; 2]>> = f.paths.iter().flat_map(|p| clip_line(&p.iter().map(|q| [q[0] as f64, q[1] as f64]).collect::<Vec<_>>(), e)).map(|l| l.into_iter().map(to_ll).collect()).collect();
                if !edges.is_empty() {
                    // Nothing says which side the area is on: its outline alone.
                    let pos = LatLon::new(edges[0][0][1], edges[0][0][0]);
                    out.push(Anchorage { kind, name, categories, pos, rings: Vec::new(), edges, radius_m });
                }
            }
            GeomKind::Polygon | GeomKind::Line => {
                // In tile units, where the cut is exact.
                let rings: Vec<Vec<[f64; 2]>> = f
                    .paths
                    .iter()
                    // A closed line repeats its first point; a polygon's ring does not.
                    .map(|r| if r.len() > 1 && r.first() == r.last() { &r[..r.len() - 1] } else { &r[..] })
                    .filter(|r| r.len() >= 3)
                    .filter_map(|r| clip_ring(&r.iter().map(|p| [p[0] as f64, p[1] as f64]).collect::<Vec<_>>(), &tile))
                    .collect();
                let Some(spot) = label_point(&rings) else { continue };
                let mut edges = Vec::new();
                for r in &rings {
                    let mut run: Vec<[f64; 2]> = Vec::new();
                    // Round the ring, from a point that is not on the cut if there is one.
                    let start = (0..r.len()).find(|&i| !on_clip_edge(r[(i + r.len() - 1) % r.len()], r[i], &tile)).unwrap_or(0);
                    for k in 0..r.len() {
                        let (a, b) = (r[(start + k + r.len() - 1) % r.len()], r[(start + k) % r.len()]);
                        if on_clip_edge(a, b, &tile) {
                            if run.len() >= 2 {
                                edges.push(std::mem::take(&mut run));
                            }
                            run.clear();
                            continue;
                        }
                        if run.is_empty() {
                            run.push(to_ll(a));
                        }
                        run.push(to_ll(b));
                    }
                    if run.len() >= 2 {
                        edges.push(run);
                    }
                }
                let rings = rings.into_iter().map(|r| r.into_iter().map(to_ll).collect()).collect();
                out.push(Anchorage { kind, name, categories, pos: id.point(spot[0], spot[1], e), rings, edges, radius_m });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::aids::light_description;

    /// Real elements from the Overpass answer for the Brest approaches.
    const BREST: &str = r#"{"version":0.6,"elements":[
 {"type":"node","id":745763552,"lat":48.3869667,"lon":-4.4292833,"tags":{"seamark:buoy_lateral:category":"starboard","seamark:buoy_lateral:colour":"green","seamark:buoy_lateral:shape":"conical","seamark:buoy_lateral:system":"iala-a","seamark:light:character":"Fl","seamark:light:colour":"green","seamark:light:group":"3","seamark:light:period":"12","seamark:name":"MB1","seamark:type":"buoy_lateral"}},
 {"type":"node","id":1004030788,"lat":48.3293069,"lon":-4.5942145,"tags":{"seamark:buoy_cardinal:category":"west","seamark:buoy_cardinal:colour":"yellow;black;yellow","seamark:buoy_cardinal:shape":"pillar","seamark:light:character":"VQ","seamark:light:colour":"white","seamark:light:group":"9","seamark:light:period":"10","seamark:name":"Plateau des fillettes","seamark:type":"buoy_cardinal"}},
 {"type":"node","id":745779306,"lat":48.3913833,"lon":-4.4293667,"tags":{"seamark:light:character":"Fl","seamark:light:colour":"red","seamark:light:height":"2","seamark:light:period":"2.5","seamark:light:range":"1","seamark:name":"MB4","seamark:type":"light_minor"}},
 {"type":"node","id":4563733253,"lat":48.3905,"lon":-4.4285,"tags":{"seamark:beacon_lateral:category":"starboard","seamark:beacon_lateral:shape":"perch","seamark:light:character":"Fl","seamark:light:colour":"green","seamark:light:period":"2.5","seamark:light:range":"2","seamark:name":"MB3","seamark:type":"beacon_lateral"}},
 {"type":"node","id":1370801418,"lat":48.358281,"lon":-4.5340038,"tags":{"man_made":"lighthouse","name":"Phare du Portzic","seamark:light:1:character":"Oc","seamark:light:1:colour":"red","seamark:light:1:group":"2","seamark:light:1:height":"56","seamark:light:1:period":"12","seamark:light:1:range":"15","seamark:light:1:sector_end":"259","seamark:light:1:sector_start":"219","seamark:light:2:character":"Oc","seamark:light:2:colour":"white","seamark:light:2:group":"2","seamark:light:2:period":"12","seamark:light:2:range":"19","seamark:light:2:sector_end":"338","seamark:light:2:sector_start":"259"}},
 {"type":"node","id":6904563331,"lat":48.3074182,"lon":-4.4927336,"tags":{"seamark:buoy_special_purpose:category":"no_entry","seamark:buoy_special_purpose:colour":"yellow","seamark:buoy_special_purpose:shape":"spherical","seamark:type":"buoy_special_purpose"}},
 {"type":"node","id":5472635087,"lat":48.3806627,"lon":-4.4897497,"tags":{"seamark:landmark:category":"flagstaff","seamark:type":"landmark"}},
 {"type":"node","id":1,"lat":48.34,"lon":-4.4,"tags":{"seamark:type":"small_craft_facility"}},
 {"type":"way","id":2,"center":{"lat":48.35,"lon":-4.45},"tags":{"seamark:type":"beacon_safe_water","seamark:beacon_safe_water:colour":"red;white"}},
 {"type":"node","id":3,"lat":48.0,"lon":-4.0}
]}"#;


    /// Parses Overpass-style elements, the shape the tags were first recorded in.
    fn parse_overpass(body: &str) -> Result<AidsData, String> {
        let v: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
        let mut owned: Vec<(LatLon, Tags)> = Vec::new();
        for e in v["elements"].as_array().ok_or("no elements")? {
            let p = if e["type"] == "node" { e } else { &e["center"] };
            let (Some(lat), Some(lon), Some(tags)) = (p["lat"].as_f64(), p["lon"].as_f64(), e["tags"].as_object()) else { continue };
            let tags = tags.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect();
            owned.push((LatLon::new(lat, lon), tags));
        }
        Ok(aids_from_elements(owned.iter().map(|(p, t)| (*p, t))))
    }
    #[test]
    fn a_lateral_buoy_with_a_light() {
        let d = parse_overpass(BREST).unwrap();
        let a = d.aids.iter().find(|a| a.name.as_deref() == Some("MB1")).unwrap();
        assert_eq!(a.kind, AidKind::BuoyLateral);
        assert_eq!((a.colours.clone(), a.shape, a.category), (vec![4], Some(1), Some(2)));
        let l = d.lights.iter().find(|l| l.pos == a.pos).unwrap();
        assert_eq!(l.character, Some(2));
        assert_eq!(light_description(l), "Fl(3)G 12s");
    }

    #[test]
    fn cardinal_marks_keep_their_colour_bands_and_quarter() {
        let d = parse_overpass(BREST).unwrap();
        let a = d.aids.iter().find(|a| a.kind == AidKind::BuoyCardinal).unwrap();
        assert_eq!((a.colours.clone(), a.shape, a.category), (vec![6, 2, 6], Some(4), Some(4)));
        let l = d.lights.iter().find(|l| l.pos == a.pos).unwrap();
        assert_eq!(light_description(l), "VQ(9)W 10s");
    }

    #[test]
    fn a_light_without_a_mark_is_only_a_light() {
        let d = parse_overpass(BREST).unwrap();
        let l = d.lights.iter().find(|l| l.height_m == Some(2.0)).unwrap();
        assert_eq!((l.period_s, l.range_nm, l.colours.clone()), (Some(2.5), Some(1.0), vec![3]));
        assert!(!d.aids.iter().any(|a| a.pos == l.pos), "light_minor is not an aid");
    }

    #[test]
    fn a_lighthouse_with_no_seamark_type_gives_its_sectors() {
        let d = parse_overpass(BREST).unwrap();
        let mut sectors: Vec<_> = d.lights.iter().filter(|l| (l.pos.lat - 48.358281).abs() < 1e-9).collect();
        sectors.sort_by(|a, b| a.sector.unwrap().0.total_cmp(&b.sector.unwrap().0));
        assert_eq!(sectors.len(), 2);
        assert_eq!(sectors[0].sector, Some((219.0, 259.0)));
        assert_eq!(sectors[1].sector, Some((259.0, 338.0)));
        assert_eq!(light_description(sectors[0]), "Oc(2)R 12s 15M");
        let lighthouse = d.aids.iter().find(|a| a.pos == sectors[0].pos).unwrap();
        assert_eq!((lighthouse.kind, lighthouse.name.as_deref()), (AidKind::Landmark, Some("Phare du Portzic")));
    }

    #[test]
    fn other_marks_and_shapes() {
        let d = parse_overpass(BREST).unwrap();
        let special = d.aids.iter().find(|a| a.kind == AidKind::BuoySpecial).unwrap();
        assert_eq!((special.colours.clone(), special.shape, special.category), (vec![6], Some(3), None));
        let bcn = d.aids.iter().find(|a| a.name.as_deref() == Some("MB3")).unwrap();
        assert_eq!((bcn.kind, bcn.shape, bcn.category), (AidKind::BeaconLateral, Some(1), Some(2)));
        assert!(d.aids.iter().any(|a| a.kind == AidKind::Landmark));
        // Areas are placed at their centre; unrelated seamarks and tagless nodes are dropped.
        let safe = d.aids.iter().find(|a| a.kind == AidKind::BeaconSafeWater).unwrap();
        assert_eq!((safe.pos.lat, safe.pos.lon, safe.colours.clone()), (48.35, -4.45, vec![3, 1]));
        assert_eq!(d.aids.len(), 7);
    }

    #[test]
    fn characters_in_their_common_spellings() {
        assert_eq!(character("Fl"), (Some(2), None));
        assert_eq!(character("Fl(2)"), (Some(2), Some("(2)".into())));
        assert_eq!(character("Al.Oc"), (Some(17), None));
        assert_eq!(character("Q + LFl").0, Some(25), "spaces are dropped");
        assert_eq!(character("Zz"), (None, None));
        assert_eq!(group("2+1"), Some("(2+1)".into()));
        assert_eq!(group("(A)"), Some("(A)".into()));
        assert_eq!(group(" "), None);
    }

    #[test]
    fn numbers_tolerate_units_and_commas() {
        assert_eq!(number("12"), Some(12.0));
        assert_eq!(number("2,5"), Some(2.5));
        assert_eq!(number("12 s"), Some(12.0));
        assert_eq!(number("ten"), None);
    }

    #[test]
    fn a_tile_gives_its_marks_and_lights_at_their_positions() {
        use crate::chart::vtile::{decode, testutil::*};
        let id = TileId { z: 12, x: 1996, y: 1417 };
        let bytes = layer(
            "seamark",
            4096,
            &[
                F {
                    kind: 1,
                    paths: vec![vec![[2048, 2048]]],
                    tags: vec![
                        ("seamark:type", V::S("buoy_lateral")),
                        ("seamark:buoy_lateral:category", V::S("port")),
                        ("seamark:buoy_lateral:colour", V::S("red")),
                        ("seamark:light:character", V::S("Fl")),
                        ("seamark:light:colour", V::S("red")),
                        ("seamark:light:period", V::S("4")),
                        ("light_range", V::Double(5.0)),
                    ],
                },
                // Lighthouses may have only the short type.
                F { kind: 1, paths: vec![vec![[100, 100]]], tags: vec![("type", V::S("buoy_cardinal")), ("seamark:buoy_cardinal:category", V::S("north"))] },
                // In the buffer: the neighbour has it.
                F { kind: 1, paths: vec![vec![[-30, 100]]], tags: vec![("seamark:type", V::S("buoy_lateral"))] },
                F { kind: 1, paths: vec![vec![[4096, 100]]], tags: vec![("seamark:type", V::S("buoy_lateral"))] },
                // Not a point.
                F { kind: 3, paths: vec![vec![[0, 0], [10, 0], [10, 10]]], tags: vec![("seamark:type", V::S("buoy_lateral"))] },
            ],
        );
        let d = aids_from_tile(&id, &decode(&bytes).unwrap());
        assert_eq!(d.aids.len(), 2);
        let mid = id.point(2048.0, 2048.0, 4096.0);
        let a = d.aids.iter().find(|a| a.kind == AidKind::BuoyLateral).unwrap();
        assert_eq!((a.pos, a.colours.clone(), a.category), (mid, vec![3], Some(1)));
        let b = id.bounds();
        assert!(b.contains([a.pos.lon, a.pos.lat]));
        assert_eq!(d.aids.iter().find(|a| a.kind == AidKind::BuoyCardinal).map(|a| a.category), Some(Some(1)));
        assert_eq!(d.lights.len(), 1);
        assert_eq!(light_description(&d.lights[0]), "FlR 4s");
        // A tile without the layer is empty.
        assert!(aids_from_tile(&id, &decode(&layer("land", 4096, &[])).unwrap()).aids.is_empty());
    }

    #[test]
    fn a_tile_gives_its_anchorages_and_where_anchoring_is_prohibited() {
        use crate::chart::vtile::{decode, testutil::*};
        let id = TileId { z: 13, x: 4186, y: 3103 };
        let bytes = layer(
            "seamark",
            4096,
            &[
                // A cove's anchorage, a point.
                F { kind: 1, paths: vec![vec![[1000, 1000]]], tags: vec![("seamark:type", V::S("anchorage")), ("name", V::S("Cala Galdana"))] },
                // An area that runs off the tile to the east.
                F {
                    kind: 3,
                    paths: vec![vec![[3000, 2000], [3000, 3000], [4160, 3000], [4160, 2000]]],
                    tags: vec![("seamark:type", V::S("anchorage")), ("seamark:anchorage:category", V::S("small_craft;unrestricted"))],
                },
                F {
                    kind: 3,
                    paths: vec![vec![[100, 3000], [100, 3500], [600, 3500], [600, 3000]]],
                    tags: vec![("seamark:type", V::S("restricted_area")), ("seamark:restricted_area:restriction", V::S("no_anchoring;no_fishing"))],
                },
                // Restricted, but not for anchoring.
                F { kind: 3, paths: vec![vec![[100, 100], [100, 500], [600, 500]]], tags: vec![("seamark:type", V::S("restricted_area")), ("seamark:restricted_area:restriction", V::S("no_entry"))] },
                F { kind: 1, paths: vec![vec![[2000, 500]]], tags: vec![("seamark:type", V::S("anchor_berth")), ("seamark:name", V::S("16C")), ("seamark:anchor_berth:radius", V::S("150"))] },
                // A tanker anchorage.
                F { kind: 1, paths: vec![vec![[2500, 500]]], tags: vec![("seamark:type", V::S("anchorage")), ("seamark:anchorage:category", V::S("tanker"))] },
                // In the buffer: the neighbour has it.
                F { kind: 1, paths: vec![vec![[-20, 500]]], tags: vec![("seamark:type", V::S("anchorage"))] },
                // A reserve mapped as a closed way, and a piece of one the tile cut.
                F {
                    kind: 2,
                    paths: vec![vec![[1000, 3000], [1500, 3000], [1500, 3500], [1000, 3000]]],
                    tags: vec![("seamark:type", V::S("restricted_area")), ("seamark:restricted_area:restriction", V::S("no_anchoring"))],
                },
                F {
                    kind: 2,
                    paths: vec![vec![[3800, -50], [3800, 200], [4150, 200]]],
                    tags: vec![("seamark:type", V::S("restricted_area")), ("seamark:restricted_area:restriction", V::S("no_anchoring"))],
                },
            ],
        );
        let d = aids_from_tile(&id, &decode(&bytes).unwrap());
        assert!(d.aids.is_empty());
        assert_eq!(d.anchorages.len(), 7, "{:?}", d.anchorages);
        let a = &d.anchorages;
        assert_eq!((a[0].kind, a[0].name.as_deref(), a[0].rings.len()), (AnchorageKind::Anchorage, Some("Cala Galdana"), 0));
        assert!(!a[0].is_for_ships());

        let area = a.iter().find(|a| !a.rings.is_empty() && a.kind == AnchorageKind::Anchorage).unwrap();
        assert_eq!(area.categories, vec![7, 1]);
        let b = id.bounds();
        assert!(area.rings[0].iter().all(|p| p[0] <= b.max_lon + 1e-9), "cut to the tile");
        assert!(area.contains([area.pos.lon, area.pos.lat]), "its symbol is inside it");
        // Three sides drawn, the cut along the tile's edge not.
        assert_eq!(area.edges.len(), 1);
        assert_eq!(area.edges[0].len(), 4);
        assert!(area.edges[0].iter().any(|p| (p[0] - b.max_lon).abs() < 1e-9), "the outline runs up to the cut");

        let no: Vec<&Anchorage> = a.iter().filter(|a| a.kind == AnchorageKind::Prohibited).collect();
        assert_eq!(no.len(), 3);
        assert_eq!((no[0].edges.len(), no[0].edges[0].len()), (1, 5), "a closed ring, all of it drawn");
        assert_eq!((no[1].rings.len(), no[1].edges[0].len()), (1, 4), "a closed line is an area");
        // A piece of a line: only what is in the tile, an outline with no inside.
        assert!(no[2].rings.is_empty());
        let piece = &no[2].edges[0];
        assert_eq!(piece.len(), 3);
        assert!((piece[0][1] - b.max_lat).abs() < 1e-9 && (piece[2][0] - b.max_lon).abs() < 1e-9, "{piece:?}");

        let berth = a.iter().find(|a| a.kind == AnchorageKind::Berth).unwrap();
        assert_eq!((berth.name.as_deref(), berth.radius_m), (Some("16C"), Some(150.0)));
        assert!(a.iter().any(|a| a.is_for_ships()), "the tanker anchorage");
    }
}
