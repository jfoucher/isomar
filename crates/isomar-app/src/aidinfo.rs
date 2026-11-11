//! What a click on a buoy, beacon, landmark or light tells: the labels are
//! not drawn on the chart, so they are shown here, in a small popover.

use super::app::AppState;
use super::text::position_text;
use super::view::ViewInfo;
use crate::core::chart::aids::{Aid, AidKind, Anchorage, AnchorageKind, Light, light_description};
use crate::core::geo::LatLon;
use crate::core::vector::Bounds;
use crate::i18n::{gettext, gettext_f};

/// Pixels round a click searched for the lights of a mark.
pub const SAME_SPOT_PX: f64 = 6.0;

/// Metres within which a light counts as on the same structure as an aid or another light. On
/// the ground, not on screen: zoomed out, a few pixels hold several marks (the buoys beside the
/// Planier lighthouse are 350 m from it, two pixels at zoom 9).
pub const SAME_SPOT_M: f64 = 20.0;

pub fn same_spot(a: &LatLon, b: &LatLon) -> bool {
    a.distance_nm(b) * 1852.0 <= SAME_SPOT_M
}

/// A mark under the pointer: an aid and the lights on it, a light alone, or an anchorage.
#[derive(Clone, Debug)]
pub struct AidInfo {
    pub pos: LatLon,
    pub aid: Option<Aid>,
    pub lights: Vec<Light>,
    pub anchorage: Option<Anchorage>,
    /// What the seabed is there, NATSUR codes, when a chart cell says.
    pub seabed: Vec<u8>,
    /// Screen distance from the click to the mark, in pixels.
    pub distance: f64,
}

/// The lon/lat box around (`x`, `y`) that is `r` pixels each way, for asking a
/// data set which marks are nearby.
pub fn click_bounds(vi: &ViewInfo, x: f64, y: f64, r: f64) -> Bounds {
    let (a, b) = (vi.position_at(x - r, y - r), vi.position_at(x + r, y + r));
    Bounds { min_lon: a.lon.min(b.lon), min_lat: a.lat.min(b.lat), max_lon: a.lon.max(b.lon), max_lat: a.lat.max(b.lat) }
}

fn px_between(vi: &ViewInfo, a: &LatLon, b: (f64, f64)) -> f64 {
    let (ax, ay) = vi.to_screen(a);
    ((ax - b.0).powi(2) + (ay - b.1).powi(2)).sqrt()
}

/// The mark nearest to (`x`, `y`) within `tol` pixels. An aid wins over a
/// light beside it; the lights on the winner's spot (`same_spot`) come with it.
pub fn pick(vi: &ViewInfo, x: f64, y: f64, tol: f64, aids: Vec<Aid>, lights: Vec<Light>) -> Option<AidInfo> {
    fn by_distance<T>(a: &(f64, T), b: &(f64, T)) -> std::cmp::Ordering {
        a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal)
    }
    let nearest_aid = aids.into_iter().map(|a| (px_between(vi, &a.pos, (x, y)), a)).filter(|(d, _)| *d <= tol).min_by(by_distance);
    let (distance, pos, aid) = match nearest_aid {
        Some((d, a)) => (d, a.pos, Some(a)),
        None => {
            let (d, l) = lights.iter().map(|l| (px_between(vi, &l.pos, (x, y)), l)).filter(|(d, _)| *d <= tol).min_by(by_distance)?;
            (d, l.pos, None)
        }
    };
    // Overlapping cells can each hold the same light.
    let mut on_it: Vec<Light> = Vec::new();
    for l in lights.into_iter().filter(|l| same_spot(&l.pos, &pos)) {
        if !on_it.contains(&l) {
            on_it.push(l);
        }
    }
    let lights = on_it;
    Some(AidInfo { pos, aid, lights, anchorage: None, seabed: Vec::new(), distance })
}

/// The mark under (`x`, `y`) in whichever aids source has one: the aids data
/// or the chart cells; else the anchorage there.
pub fn at(app: &AppState, x: f64, y: f64, tol: f64) -> Option<AidInfo> {
    let vi = app.canvas.view_info();
    let from_data = app.aids.borrow().as_ref().and_then(|l| l.info_at(&vi, x, y, tol));
    let from_cells = app.enc.borrow().as_ref().and_then(|l| l.info_at(&vi, x, y, tol));
    let by_distance = |a: &AidInfo, b: &AidInfo| a.distance.total_cmp(&b.distance);
    if let Some(mark) = [from_data, from_cells].into_iter().flatten().min_by(by_distance) {
        return Some(mark);
    }
    let from_data = app.aids.borrow().as_ref().and_then(|l| l.anchorage_at(&vi, x, y, tol)).map(|(d, a)| (d, a, Vec::new()));
    let from_cells = app.enc.borrow().as_ref().and_then(|l| l.anchorage_at(&vi, x, y, tol));
    let (distance, a, seabed) = [from_data, from_cells].into_iter().flatten().min_by(|a, b| a.0.total_cmp(&b.0))?;
    Some(AidInfo { pos: a.pos, aid: None, lights: Vec::new(), anchorage: Some(a), seabed, distance })
}

fn anchorage_kind_name(a: &Anchorage) -> String {
    match a.kind {
        AnchorageKind::Anchorage if a.rings.is_empty() => gettext("Anchorage"),
        AnchorageKind::Anchorage => gettext("Anchorage area"),
        AnchorageKind::Berth => gettext("Anchor berth"),
        AnchorageKind::Prohibited => gettext("Anchoring prohibited"),
        AnchorageKind::Restricted => gettext("Anchoring restricted"),
    }
}

/// A CATACH code in words.
fn anchorage_category(code: u8) -> Option<String> {
    Some(match code {
        1 => gettext("Unrestricted"),
        2 => gettext("Deep water"),
        3 => gettext("Tankers"),
        4 => gettext("Explosives"),
        5 => gettext("Quarantine"),
        6 => gettext("Seaplanes"),
        7 => gettext("Small craft"),
        8 => gettext("Small craft moorings"),
        9 => gettext("Up to 24 hours"),
        _ => return None,
    })
}

/// A NATSUR code in words.
fn seabed_name(code: u8) -> Option<String> {
    Some(match code {
        1 => gettext("mud"),
        2 => gettext("clay"),
        3 => gettext("silt"),
        4 => gettext("sand"),
        5 => gettext("stones"),
        6 => gettext("gravel"),
        7 => gettext("pebbles"),
        8 => gettext("cobbles"),
        9 => gettext("rock"),
        11 => gettext("lava"),
        14 => gettext("coral"),
        17 => gettext("shells"),
        18 => gettext("boulders"),
        _ => return None,
    })
}

fn kind_name(kind: AidKind) -> String {
    match kind {
        AidKind::BuoyLateral => gettext("Lateral buoy"),
        AidKind::BuoyCardinal => gettext("Cardinal buoy"),
        AidKind::BuoySpecial => gettext("Special-purpose buoy"),
        AidKind::BuoyIsolatedDanger => gettext("Isolated danger buoy"),
        AidKind::BuoySafeWater => gettext("Safe water buoy"),
        AidKind::BuoyInstallation => gettext("Installation buoy"),
        AidKind::BeaconLateral => gettext("Lateral beacon"),
        AidKind::BeaconCardinal => gettext("Cardinal beacon"),
        AidKind::BeaconSpecial => gettext("Special-purpose beacon"),
        AidKind::BeaconIsolatedDanger => gettext("Isolated danger beacon"),
        AidKind::BeaconSafeWater => gettext("Safe water beacon"),
        AidKind::Landmark => gettext("Landmark"),
    }
}

/// CATLAM and CATCAM, the only categories that are named here.
fn category_name(kind: AidKind, code: i32) -> Option<String> {
    Some(match (kind, code) {
        (AidKind::BuoyLateral | AidKind::BeaconLateral, 1) => gettext("Port hand"),
        (AidKind::BuoyLateral | AidKind::BeaconLateral, 2) => gettext("Starboard hand"),
        (AidKind::BuoyLateral | AidKind::BeaconLateral, 3) => gettext("Preferred channel to starboard"),
        (AidKind::BuoyLateral | AidKind::BeaconLateral, 4) => gettext("Preferred channel to port"),
        (AidKind::BuoyCardinal | AidKind::BeaconCardinal, 1) => gettext("North"),
        (AidKind::BuoyCardinal | AidKind::BeaconCardinal, 2) => gettext("East"),
        (AidKind::BuoyCardinal | AidKind::BeaconCardinal, 3) => gettext("South"),
        (AidKind::BuoyCardinal | AidKind::BeaconCardinal, 4) => gettext("West"),
        _ => return None,
    })
}

fn shape_name(kind: AidKind, code: i32) -> Option<String> {
    Some(if kind.is_buoy() {
        match code {
            1 => gettext("Conical"),
            2 => gettext("Can"),
            3 => gettext("Spherical"),
            4 => gettext("Pillar"),
            5 => gettext("Spar"),
            6 => gettext("Barrel"),
            7 => gettext("Super-buoy"),
            8 => gettext("Ice buoy"),
            _ => return None,
        }
    } else if kind.is_beacon() {
        match code {
            1 => gettext("Stake or pole"),
            2 => gettext("Withy"),
            3 => gettext("Tower"),
            4 => gettext("Lattice"),
            5 => gettext("Pile"),
            6 => gettext("Cairn"),
            7 => gettext("Buoyant"),
            _ => return None,
        }
    } else {
        return None;
    })
}

fn colour_name(code: u8) -> Option<String> {
    Some(match code {
        1 => gettext("white"),
        2 => gettext("black"),
        3 => gettext("red"),
        4 => gettext("green"),
        5 => gettext("blue"),
        6 => gettext("yellow"),
        7 => gettext("grey"),
        8 => gettext("brown"),
        9 => gettext("amber"),
        10 => gettext("violet"),
        11 => gettext("orange"),
        12 => gettext("magenta"),
        13 => gettext("pink"),
        _ => return None,
    })
}

fn colours_text(codes: &[u8]) -> Option<String> {
    let names: Vec<String> = codes.iter().filter_map(|c| colour_name(*c)).collect();
    (!names.is_empty()).then(|| names.join(", "))
}

fn trim_float(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 { format!("{}", v.round() as i64) } else { format!("{v:.1}") }
}

/// One line about a light: as printed on a chart, then its sector and height.
fn light_text(l: &Light) -> String {
    let mut parts = Vec::new();
    let d = light_description(l);
    parts.push(if d.is_empty() { gettext("Light") } else { d });
    if let Some((a, b)) = l.sector {
        parts.push(gettext_f("sector {from}° to {to}°", &[("from", &trim_float(a)), ("to", &trim_float(b))]));
    }
    if let Some(h) = l.height_m.filter(|h| *h > 0.0) {
        parts.push(gettext_f("{height} m high", &[("height", &trim_float(h))]));
    }
    parts.join(", ")
}

/// What the popover says: a heading, a line under it, and labelled rows.
#[derive(Clone, Debug, PartialEq)]
pub struct Description {
    pub title: String,
    pub subtitle: Option<String>,
    pub rows: Vec<(String, String)>,
    /// Several features lie under the click: this is number `.0` of `.1`, and the popover offers the others.
    pub choice: Option<(usize, usize)>,
}

pub fn describe(info: &AidInfo) -> Description {
    let mut rows = Vec::new();
    if let Some(a) = &info.anchorage {
        let kind = anchorage_kind_name(a);
        let categories: Vec<String> = a.categories.iter().filter_map(|c| anchorage_category(*c)).collect();
        if !categories.is_empty() {
            rows.push((gettext("For"), categories.join(", ")));
        }
        if let Some(r) = a.radius_m {
            rows.push((gettext("Swinging radius"), gettext_f("{radius} m", &[("radius", &trim_float(r))])));
        }
        let seabed: Vec<String> = info.seabed.iter().filter_map(|c| seabed_name(*c)).collect();
        if !seabed.is_empty() {
            rows.push((gettext("Seabed"), seabed.join(", ")));
        }
        rows.push((gettext("Position"), position_text(&info.pos)));
        let (title, subtitle) = match &a.name {
            Some(n) if a.kind == AnchorageKind::Berth => (gettext_f("Berth {number}", &[("number", n)]), None),
            Some(n) => (n.clone(), Some(kind)),
            None => (kind, None),
        };
        return Description { title, subtitle, rows, choice: None };
    }
    let (title, subtitle) = match &info.aid {
        Some(a) => {
            let kind = if a.kind == AidKind::Landmark && !info.lights.is_empty() { gettext("Lighthouse") } else { kind_name(a.kind) };
            if let Some(c) = a.category.and_then(|c| category_name(a.kind, c)) {
                rows.push((gettext("Mark"), c));
            }
            if let Some(s) = a.shape.and_then(|s| shape_name(a.kind, s)) {
                rows.push((gettext("Shape"), s));
            }
            if let Some(c) = colours_text(&a.colours) {
                rows.push((gettext("Colour"), c));
            }
            match &a.name {
                Some(n) => (n.clone(), Some(kind)),
                None => (kind, None),
            }
        }
        None => (if info.lights.len() > 1 { gettext("Lights") } else { gettext("Light") }, None),
    };
    for l in &info.lights {
        rows.push((gettext("Light"), light_text(l)));
    }
    rows.push((gettext("Position"), position_text(&info.pos)));
    Description { title, subtitle, rows, choice: None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::View;

    fn vi() -> ViewInfo {
        ViewInfo { view: View { center: LatLon::new(48.0, -4.0), zoom: 13.0 }, w: 800.0, h: 600.0 }
    }

    fn aid(pos: LatLon, kind: AidKind) -> Aid {
        Aid { pos, kind, name: None, colours: vec![], shape: None, category: None }
    }

    fn light(pos: LatLon) -> Light {
        Light { pos, colours: vec![1], character: Some(2), group: None, period_s: Some(5.0), range_nm: Some(8.0), sector: None, height_m: Some(12.0) }
    }

    #[test]
    fn the_nearest_aid_in_reach_is_picked_with_the_lights_on_it() {
        let v = vi();
        let at = LatLon::new(48.0, -4.0);
        let (bx, _) = v.to_screen(&at);
        let far = v.position_at(bx + 8.0, 300.0);
        let found = pick(&v, 402.0, 300.0, 12.0, vec![aid(far, AidKind::BuoyCardinal), aid(at, AidKind::BeaconLateral)], vec![light(at), light(v.position_at(500.0, 300.0))]).unwrap();
        assert_eq!(found.aid.unwrap().kind, AidKind::BeaconLateral);
        assert_eq!(found.lights.len(), 1, "the light on the beacon, not the one far off");
    }

    #[test]
    fn a_light_alone_is_picked_when_no_aid_is_near() {
        let v = vi();
        let at = LatLon::new(48.0, -4.0);
        let found = pick(&v, 405.0, 300.0, 12.0, vec![aid(v.position_at(700.0, 100.0), AidKind::Landmark)], vec![light(at)]).unwrap();
        assert!(found.aid.is_none());
        assert_eq!(found.lights.len(), 1);
        assert!(pick(&v, 450.0, 300.0, 12.0, vec![], vec![light(at)]).is_none(), "out of reach");
    }

    #[test]
    fn a_cardinal_buoy_with_a_light_is_described() {
        let at = LatLon::new(48.39, -4.49);
        let mut a = aid(at, AidKind::BuoyCardinal);
        a.name = Some("Basse Royale".into());
        a.category = Some(1);
        a.shape = Some(4);
        a.colours = vec![2, 6];
        let d = describe(&AidInfo { pos: at, aid: Some(a), lights: vec![light(at)], anchorage: None, seabed: vec![], distance: 0.0 });
        assert_eq!(d.title, "Basse Royale");
        assert_eq!(d.subtitle.as_deref(), Some("Cardinal buoy"));
        assert_eq!(
            d.rows,
            vec![
                ("Mark".to_string(), "North".to_string()),
                ("Shape".to_string(), "Pillar".to_string()),
                ("Colour".to_string(), "black, yellow".to_string()),
                ("Light".to_string(), "FlW 5s 8M, 12 m high".to_string()),
                ("Position".to_string(), "48°23.400′N 004°29.400′W".to_string()),
            ]
        );
    }

    #[test]
    fn an_unnamed_mark_is_titled_by_its_kind() {
        let at = LatLon::new(48.0, -4.0);
        let d = describe(&AidInfo { pos: at, aid: Some(aid(at, AidKind::Landmark)), lights: vec![], anchorage: None, seabed: vec![], distance: 0.0 });
        assert_eq!(d.title, "Landmark");
        assert_eq!(d.subtitle, None);
        assert_eq!(d.rows.len(), 1, "only the position");
    }
    #[test]
    fn an_anchorage_is_described_with_its_categories_and_seabed() {
        let at = LatLon::new(47.6, -2.8);
        let a = Anchorage { kind: AnchorageKind::Anchorage, name: Some("Pen er Men".into()), categories: vec![7, 1], pos: at, rings: vec![vec![[-2.81, 47.59], [-2.79, 47.59], [-2.79, 47.61]]], edges: vec![], radius_m: None };
        let d = describe(&AidInfo { pos: at, aid: None, lights: vec![], anchorage: Some(a.clone()), seabed: vec![4, 1], distance: 0.0 });
        assert_eq!((d.title.as_str(), d.subtitle.as_deref()), ("Pen er Men", Some("Anchorage area")));
        assert_eq!(d.rows[0], ("For".to_string(), "Small craft, Unrestricted".to_string()));
        assert_eq!(d.rows[1], ("Seabed".to_string(), "sand, mud".to_string()));
        let berth = Anchorage { kind: AnchorageKind::Berth, name: Some("16C".into()), rings: vec![], radius_m: Some(150.0), categories: vec![], ..a };
        let d = describe(&AidInfo { pos: at, aid: None, lights: vec![], anchorage: Some(berth), seabed: vec![], distance: 0.0 });
        assert_eq!((d.title.as_str(), d.subtitle), ("Berth 16C", None));
        assert_eq!(d.rows[0], ("Swinging radius".to_string(), "150 m".to_string()));
    }
}
