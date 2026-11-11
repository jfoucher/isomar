//! What a click on any other feature of an S-57 cell tells: its class and name, and every attribute the cell gives
//! in words (a wreck's category, a restricted area's restrictions, the text a chart's author added). Marks, lights
//! and anchorages have their own report (`aidinfo`); this one is for the rest of what the chart is drawn from.

use super::aidinfo::Description;
use super::text::position_text;
use super::view::ViewInfo;
use crate::core::chart::s57::{Feature, Geom, attr, obj};
use crate::core::chart::s57names::{attr_name, attr_type, class_name, enum_meaning};
use crate::core::geo::LatLon;
use crate::core::vector as core_vector;
use crate::i18n::gettext;

/// The classes a click reports (and the pointer outlines): what the chart draws for itself, other than the marks,
/// lights and anchorages that `aidinfo` reports and the sea, land, depths and coastline that say nothing when
/// clicked. A class the chart does not draw is not here, whatever the cell holds: outlining a two-way route part
/// that is nowhere on the chart only puzzles, and the zones of confidence (`M_QUAL`), which S-52 puts in the
/// display category Other and which are drawn only as a quiet dotted edge with their category written in them,
/// are no more to click than the edge of the chart's grid. Add a class here when a layer draws it.
pub fn reportable(objl: u16) -> bool {
    matches!(
        objl,
        // Areas outlined by `chartinfo::draw_areas`, `enc::draw_dredged_and_fairways`, and the grey of unsurveyed ground.
        obj::RESARE
            | obj::CTNARE
            | obj::MIPARE
            | obj::DMPGRD
            | obj::MARCUL
            | obj::OSPARE
            | obj::SPLARE
            | obj::CBLARE
            | obj::PIPARE
            | obj::FSHZNE
            | obj::TESARE
            | obj::ISTZNE
            | obj::FAIRWY
            | obj::DRGARE
            | obj::UNSARE
            // Lines and what the layers draw along them.
            | obj::CBLOHD
            | obj::CBLSUB
            | obj::PIPSOL
            | obj::NAVLNE
            | obj::RECTRC
            | obj::SLCONS
            | obj::LNDELV
            | obj::TS_FEB
            // Dangers.
            | obj::WATTUR
            | obj::UWTROC
            | obj::WRECKS
            | obj::OBSTRN
    )
}

/// Whether a click inside an area of this class reports it. A warning is, as is a danger or a turbulent patch;
/// a fairway, a survey zone or an administrative area covers much of the chart, and only its edge is clickable.
pub fn clickable_inside(objl: u16) -> bool {
    matches!(
        objl,
        obj::RESARE
            | obj::CTNARE
            | obj::MIPARE
            | obj::DMPGRD
            | obj::MARCUL
            | obj::OSPARE
            | obj::SPLARE
            | obj::CBLARE
            | obj::PIPARE
            | obj::FSHZNE
            | obj::TESARE
            | obj::ISTZNE
            | obj::WATTUR
            | obj::UWTROC
            | obj::WRECKS
            | obj::OBSTRN
            | obj::SLCONS
    )
}

fn px_distance(vi: &ViewInfo, p: &[f64; 2], x: f64, y: f64) -> f64 {
    let (px, py) = vi.to_screen(&LatLon::new(p[1], p[0]));
    (px - x).hypot(py - y)
}

/// How far (`x`, `y`) is, in pixels, from the polyline `pts`.
fn polyline_distance(vi: &ViewInfo, pts: &[[f64; 2]], x: f64, y: f64) -> f64 {
    let screen: Vec<(f64, f64)> = pts.iter().map(|p| vi.to_screen(&LatLon::new(p[1], p[0]))).collect();
    if let [only] = screen.as_slice() {
        return (only.0 - x).hypot(only.1 - y);
    }
    screen
        .windows(2)
        .map(|w| {
            let (dx, dy) = (w[1].0 - w[0].0, w[1].1 - w[0].1);
            let len2 = dx * dx + dy * dy;
            let t = if len2 == 0.0 { 0.0 } else { (((x - w[0].0) * dx + (y - w[0].1) * dy) / len2).clamp(0.0, 1.0) };
            (w[0].0 + t * dx - x).hypot(w[0].1 + t * dy - y)
        })
        .fold(f64::MAX, f64::min)
}

/// How near a click is to a feature, for choosing between those under it: points and lines by their distance in
/// pixels, an area it is inside by 1000 pixels plus its size, so the smallest area wins and anything nearer
/// than that beats it. `None` when the click is not on it.
fn reach(vi: &ViewInfo, f: &Feature, x: f64, y: f64, tol: f64) -> Option<f64> {
    match &f.geom {
        Geom::Point(p) => Some(px_distance(vi, p, x, y)).filter(|d| *d <= tol),
        Geom::Line(l) => Some(polyline_distance(vi, l, x, y)).filter(|d| *d <= tol),
        Geom::Area(rings) => {
            let at = vi.position_at(x, y);
            if clickable_inside(f.objl) && rings.iter().filter(|r| core_vector::ring_contains(r, [at.lon, at.lat])).count() % 2 == 1 {
                let b = rings.iter().fold(core_vector::Bounds::EMPTY, |mut b, r| {
                    b.union(&core_vector::Bounds::of(r));
                    b
                });
                return Some(1000.0 + b.width() * b.height() * 1e6);
            }
            rings.iter().map(|r| polyline_distance(vi, r, x, y)).fold(f64::MAX, f64::min).le(&tol).then_some(500.0)
        }
        _ => None,
    }
}

/// Every feature under (`x`, `y`) among `features`, the one a click opens first: points and lines within `tol`
/// pixels nearest first, then the areas it is on, the smallest first.
pub fn pick_all<'a>(vi: &ViewInfo, features: impl Iterator<Item = &'a Feature>, x: f64, y: f64, tol: f64) -> Vec<&'a Feature> {
    let mut found: Vec<(f64, &Feature)> = features.filter(|f| reportable(f.objl) && !crate::layers::chartinfo::omits_detail(f)).filter_map(|f| Some((reach(vi, f, x, y, tol)?, f))).collect();
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found.into_iter().map(|(_, f)| f).collect()
}

/// The feature under (`x`, `y`) that a click opens first.
#[cfg(test)]
pub fn pick<'a>(vi: &ViewInfo, features: impl Iterator<Item = &'a Feature>, x: f64, y: f64, tol: f64) -> Option<&'a Feature> {
    pick_all(vi, features, x, y, tol).into_iter().next()
}

/// The attributes that say nothing a mariner needs: when and by whom a feature was surveyed, its scale limits and
/// the update bookkeeping.
fn is_bookkeeping(code: u16) -> bool {
    matches!(attr_acronym(code), "SORDAT" | "SORIND" | "SCAMIN" | "SCAMAX" | "RECDAT" | "RECIND" | "NOBJNM" | "OBJNAM" | "INFORM" | "NINFOM" | "TXTDSC" | "PICREP" | "PEREND" | "PERSTA" | "DATEND" | "DATSTA" | "NATION")
}

fn attr_acronym(code: u16) -> &'static str {
    crate::core::chart::s57names::ATTRIBUTES.binary_search_by_key(&code, |a| a.0).ok().map_or("", |i| crate::core::chart::s57names::ATTRIBUTES[i].1)
}

/// The unit an attribute's number is in, where the standard has one.
fn unit(code: u16) -> &'static str {
    match attr_acronym(code) {
        "VALSOU" | "DRVAL1" | "DRVAL2" | "HEIGHT" | "ELEVAT" | "VERCLR" | "VERCCL" | "VERCOP" | "VERCSA" | "VERLEN" | "HORLEN" | "HORWID" | "HORCLR" | "SOUACC" | "VALDCO" | "RADIUS" | "DEPTH" | "VERACC" | "HORACC" => " m",
        "VALNMR" => " M",
        "ORIENT" | "VALMAG" | "SECTR1" | "SECTR2" | "BEARNG" => "°",
        "CURVEL" => " kn",
        "VALACM" => "′/yr",
        _ => "",
    }
}

fn trim_float(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 { format!("{}", v.round() as i64) } else { format!("{v:.1}") }
}

/// An attribute's value in words: each enumerated value by what it means, numbers with their unit, text as it is.
fn value_text(code: u16, raw: &str) -> String {
    let raw = raw.trim();
    match attr_type(code) {
        Some('E' | 'L') => {
            let words: Vec<String> = raw
                .split(',')
                .filter(|p| !p.trim().is_empty())
                .map(|p| p.trim().parse::<u16>().ok().and_then(|v| enum_meaning(code, v)).map_or_else(|| p.trim().to_string(), gettext))
                .collect();
            words.join(", ")
        }
        Some('F' | 'I') => match raw.parse::<f64>() {
            Ok(v) => format!("{}{}", trim_float(v), unit(code)),
            Err(_) => raw.to_string(),
        },
        _ => raw.to_string(),
    }
}

/// A feature in words: its name (or its class's) as the heading, then its attributes.
pub fn describe(f: &Feature, pos: &LatLon) -> Description {
    let class = class_name(f.objl).map_or_else(|| gettext("Chart feature"), gettext);
    let (title, subtitle) = match f.attr(attr::OBJNAM) {
        Some(n) => (n.trim().to_string(), Some(class)),
        None => (class, None),
    };
    let mut codes: Vec<u16> = f.attrs.iter().filter(|(c, v)| !v.trim().is_empty() && !is_bookkeeping(**c)).map(|(c, _)| *c).collect();
    // Categories first: they say what the thing is.
    codes.sort_by_key(|c| (!attr_acronym(*c).starts_with("CAT"), *c));
    let mut rows: Vec<(String, String)> = codes
        .into_iter()
        .filter_map(|c| {
            let text = value_text(c, &f.attrs[&c]);
            (!text.is_empty()).then(|| (attr_name(c).map_or_else(|| format!("{c}"), gettext), text))
        })
        .collect();
    for code in [attr::INFORM, attr::NINFOM] {
        if let Some(t) = f.attr(code) {
            rows.push((gettext("Note"), t.trim().to_string()));
        }
    }
    rows.push((gettext("Position"), position_text(pos)));
    Description { title, subtitle, rows, choice: None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::View;
    use std::collections::BTreeMap;

    fn vi() -> ViewInfo {
        ViewInfo { view: View { center: LatLon::new(48.0, -4.0), zoom: 13.0 }, w: 800.0, h: 600.0 }
    }

    fn feature(objl: u16, attrs: &[(u16, &str)], geom: Geom) -> Feature {
        Feature { rcid: 1, objl, attrs: attrs.iter().map(|(k, v)| (*k, v.to_string())).collect::<BTreeMap<_, _>>(), geom }
    }

    #[test]
    fn a_wreck_is_described_in_words() {
        let f = feature(obj::WRECKS, &[(attr::CATWRK, "2"), (attr::WATLEV, "3"), (attr::VALSOU, "12.4"), (attr::SCAMIN, "44999"), (attr::INFORM, "Wreck of the Maria")], Geom::Point([-4.0, 48.0]));
        let d = describe(&f, &LatLon::new(48.0, -4.0));
        assert_eq!((d.title.as_str(), d.subtitle), ("Wreck", None));
        assert_eq!(d.rows[0], ("Category of wreck".to_string(), "dangerous wreck".to_string()), "categories first");
        assert!(d.rows.contains(&("Water level effect".to_string(), "always under water/submerged".to_string())));
        assert!(d.rows.contains(&("Value of sounding".to_string(), "12.4 m".to_string())), "{:?}", d.rows);
        assert!(d.rows.contains(&("Note".to_string(), "Wreck of the Maria".to_string())));
        assert!(!d.rows.iter().any(|r| r.0.contains("scale") || r.0.contains("Scale")), "no scale limits: {:?}", d.rows);
        assert_eq!(d.rows.last().unwrap().0, "Position");
    }

    #[test]
    fn a_named_feature_has_its_name_for_a_title_and_its_class_under_it() {
        let f = feature(obj::RESARE, &[(attr::OBJNAM, "Zone de protection"), (attr::RESTRN, "3,7")], Geom::None);
        let d = describe(&f, &LatLon::new(0.0, 0.0));
        assert_eq!(d.title, "Zone de protection");
        assert_eq!(d.subtitle.as_deref(), Some("Restricted area"));
        assert_eq!(d.rows[0].1, "fishing prohibited, entry prohibited");
    }

    #[test]
    fn the_nearest_line_within_reach_is_picked_and_a_small_area_beats_a_big_one() {
        let v = vi();
        let c = v.position_at(400.0, 300.0);
        let (a, b) = (v.position_at(300.0, 303.0), v.position_at(500.0, 303.0));
        let cable = feature(obj::CBLSUB, &[], Geom::Line(vec![[a.lon, a.lat], [b.lon, b.lat]]));
        let far = feature(obj::CBLSUB, &[], Geom::Line(vec![[a.lon, a.lat - 0.01], [b.lon, b.lat - 0.01]]));
        assert_eq!(pick(&v, [&far, &cable].into_iter(), 400.0, 300.0, 8.0).map(|f| f.rcid), Some(1));
        assert!(pick(&v, [&far].into_iter(), 400.0, 300.0, 8.0).is_none());
        let ring = |d: f64| vec![vec![[c.lon - d, c.lat - d], [c.lon + d, c.lat - d], [c.lon + d, c.lat + d], [c.lon - d, c.lat + d], [c.lon - d, c.lat - d]]];
        let mut big = feature(obj::RESARE, &[], Geom::Area(ring(0.05)));
        big.rcid = 10;
        let mut small = feature(obj::CTNARE, &[], Geom::Area(ring(0.005)));
        small.rcid = 11;
        assert_eq!(pick(&v, [&big, &small].into_iter(), 400.0, 300.0, 8.0).map(|f| f.rcid), Some(11));
        // A fairway reports at its edge only.
        let mut fairway = feature(obj::FAIRWY, &[], Geom::Area(ring(0.05)));
        fairway.rcid = 12;
        assert!(pick(&v, [&fairway].into_iter(), 400.0, 300.0, 8.0).is_none(), "the inside of a fairway says nothing");
        // The sea and the land are not reported at all.
        let sea = feature(obj::DEPARE, &[], Geom::Area(ring(0.05)));
        assert!(pick(&v, [&sea].into_iter(), 400.0, 300.0, 8.0).is_none());
        // Nor what the chart does not draw: a two-way route part, a zone of confidence.
        for objl in [obj::TWRTPT, obj::M_QUAL] {
            let undrawn = feature(objl, &[], Geom::Area(ring(0.05)));
            assert!(pick(&v, [&undrawn].into_iter(), 400.0, 300.0, 8.0).is_none(), "class {objl}");
            let edge = feature(objl, &[], Geom::Area(ring(0.0005)));
            assert!(pick(&v, [&edge].into_iter(), 400.0, 300.0, 8.0).is_none(), "class {objl}");
        }
        // Nor a NOAA caution area that only says its cell leaves the detail to a larger-scale one.
        let note = "Most features, including bathymetry, are omitted in this area.  The minimal depiction of detail in this area does not support safe navigation.";
        let omitted = feature(obj::CTNARE, &[(attr::INFORM, note)], Geom::Area(ring(0.05)));
        assert!(pick(&v, [&omitted].into_iter(), 400.0, 300.0, 8.0).is_none());
        let debris = feature(obj::CTNARE, &[(attr::INFORM, "Uncharted post-hurricane debris has been reported in 2005.")], Geom::Area(ring(0.05)));
        assert!(pick(&v, [&debris].into_iter(), 400.0, 300.0, 8.0).is_some(), "other cautions still are");
    }
}
