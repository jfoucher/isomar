//! GPX 1.0/1.1 import and export. The extensions OpenCPN uses
//! (`opencpn:arrival_radius`, `opencpn:planned_speed`) are understood and
//! written, so routes and marks can be exchanged with it.

use super::model::*;
use crate::geo::LatLon;
use chrono::{DateTime, SecondsFormat, Utc};
use roxmltree::{Document, Node};

#[derive(Default, Debug)]
pub struct GpxDoc {
    pub waypoints: Vec<Waypoint>,
    pub routes: Vec<Route>,
    pub tracks: Vec<Track>,
    /// Things that were skipped or looked wrong.
    pub warnings: Vec<String>,
}

fn child<'a, 'b>(n: Node<'a, 'b>, name: &str) -> Option<Node<'a, 'b>> {
    n.children().find(|c| c.is_element() && c.tag_name().name() == name)
}

fn child_text(n: Node, name: &str) -> Option<String> {
    child(n, name).and_then(|c| c.text()).map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

/// Text of an element nested in `<extensions>`, whatever its namespace.
fn ext_text(n: Node, name: &str) -> Option<String> {
    child(n, "extensions").and_then(|e| child_text(e, name))
}

fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc))
}

fn latlon(n: Node) -> Option<LatLon> {
    let lat: f64 = n.attribute("lat")?.trim().parse().ok()?;
    let lon: f64 = n.attribute("lon")?.trim().parse().ok()?;
    ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)).then(|| LatLon::new(lat, lon))
}

pub fn parse(xml: &str) -> Result<GpxDoc, String> {
    let doc = Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions { allow_dtd: false, ..Default::default() },
    )
    .map_err(|e| format!("not valid XML: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "gpx" {
        return Err("not a GPX file".into());
    }
    let mut out = GpxDoc::default();
    for n in root.children().filter(|c| c.is_element()) {
        match n.tag_name().name() {
            "wpt" => match latlon(n) {
                Some(pos) => out.waypoints.push(waypoint(n, pos)),
                None => out.warnings.push("skipped a waypoint with bad coordinates".into()),
            },
            "rte" => out.routes.push(route(n, &mut out.warnings)),
            "trk" => out.tracks.push(track(n, &mut out.warnings)),
            _ => {}
        }
    }
    Ok(out)
}

fn waypoint(n: Node, pos: LatLon) -> Waypoint {
    let mut w = Waypoint::new(child_text(n, "name").unwrap_or_default(), pos);
    if let Some(s) = child_text(n, "sym") {
        w.icon = s;
    }
    w.description = child_text(n, "desc").or_else(|| child_text(n, "cmt")).unwrap_or_default();
    w.time = child_text(n, "time").and_then(|t| parse_time(&t));
    if let Some(r) = ext_text(n, "arrival_radius").and_then(|s| s.parse::<f64>().ok()) {
        w.arrival_radius_nm = r;
    }
    if let Some(v) = ext_text(n, "viz") {
        w.visible = v != "0";
    }
    w
}

fn route(n: Node, warnings: &mut Vec<String>) -> Route {
    let mut r = Route::new(child_text(n, "name").unwrap_or_else(|| "Route".into()));
    if let Some(v) = ext_text(n, "planned_speed").and_then(|s| s.parse::<f64>().ok()) {
        r.speed_kn = v;
    }
    if let Some(t) = ext_text(n, "planned_departure").and_then(|s| parse_time(&s)) {
        r.departure = Some(t);
    }
    for p in n.children().filter(|c| c.is_element() && c.tag_name().name() == "rtept") {
        match latlon(p) {
            Some(pos) => {
                let mut rp = RoutePoint::new(child_text(p, "name").unwrap_or_default(), pos);
                if let Some(rad) = ext_text(p, "arrival_radius").and_then(|s| s.parse::<f64>().ok()) {
                    rp.arrival_radius_nm = rad;
                }
                r.points.push(rp);
            }
            None => warnings.push(format!("skipped a bad point in route {}", r.name)),
        }
    }
    r
}

fn track(n: Node, warnings: &mut Vec<String>) -> Track {
    let mut t = Track::new(child_text(n, "name").unwrap_or_else(|| "Track".into()));
    for seg in n.children().filter(|c| c.is_element() && c.tag_name().name() == "trkseg") {
        for p in seg.children().filter(|c| c.is_element() && c.tag_name().name() == "trkpt") {
            match latlon(p) {
                Some(pos) => t.points.push(TrackPoint { pos, time: child_text(p, "time").and_then(|s| parse_time(&s)) }),
                None => warnings.push(format!("skipped a bad point in track {}", t.name)),
            }
        }
    }
    t
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn time(t: &DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn write(doc: &GpxDoc) -> String {
    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    s.push_str(
        "<gpx version=\"1.1\" creator=\"Isomar\" xmlns=\"http://www.topografix.com/GPX/1/1\" \
         xmlns:opencpn=\"http://www.opencpn.org\">\n",
    );
    for w in &doc.waypoints {
        s.push_str(&format!("  <wpt lat=\"{:.7}\" lon=\"{:.7}\">\n", w.pos.lat, w.pos.lon));
        if let Some(t) = &w.time {
            s.push_str(&format!("    <time>{}</time>\n", time(t)));
        }
        s.push_str(&format!("    <name>{}</name>\n", esc(&w.name)));
        if !w.description.is_empty() {
            s.push_str(&format!("    <desc>{}</desc>\n", esc(&w.description)));
        }
        s.push_str(&format!("    <sym>{}</sym>\n", esc(&w.icon)));
        s.push_str("    <extensions>\n");
        s.push_str(&format!("      <opencpn:arrival_radius>{}</opencpn:arrival_radius>\n", w.arrival_radius_nm));
        s.push_str(&format!("      <opencpn:viz>{}</opencpn:viz>\n", if w.visible { 1 } else { 0 }));
        s.push_str("    </extensions>\n  </wpt>\n");
    }
    for r in &doc.routes {
        s.push_str("  <rte>\n");
        s.push_str(&format!("    <name>{}</name>\n", esc(&r.name)));
        s.push_str("    <extensions>\n");
        s.push_str(&format!("      <opencpn:planned_speed>{}</opencpn:planned_speed>\n", r.speed_kn));
        if let Some(d) = &r.departure {
            s.push_str(&format!("      <opencpn:planned_departure>{}</opencpn:planned_departure>\n", time(d)));
        }
        s.push_str("    </extensions>\n");
        for p in &r.points {
            s.push_str(&format!("    <rtept lat=\"{:.7}\" lon=\"{:.7}\">\n", p.pos.lat, p.pos.lon));
            s.push_str(&format!("      <name>{}</name>\n", esc(&p.name)));
            s.push_str(&format!(
                "      <extensions><opencpn:arrival_radius>{}</opencpn:arrival_radius></extensions>\n",
                p.arrival_radius_nm
            ));
            s.push_str("    </rtept>\n");
        }
        s.push_str("  </rte>\n");
    }
    for t in &doc.tracks {
        s.push_str(&format!("  <trk>\n    <name>{}</name>\n    <trkseg>\n", esc(&t.name)));
        for p in &t.points {
            s.push_str(&format!("      <trkpt lat=\"{:.7}\" lon=\"{:.7}\">", p.pos.lat, p.pos.lon));
            if let Some(tm) = &p.time {
                s.push_str(&format!("<time>{}</time>", time(tm)));
            }
            s.push_str("</trkpt>\n");
        }
        s.push_str("    </trkseg>\n  </trk>\n");
    }
    s.push_str("</gpx>\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn sample() -> GpxDoc {
        let mut w = Waypoint::new("Mouillage <A&B>", LatLon::new(48.3812345, -4.5123456));
        w.icon = "anchorage".into();
        w.description = "quiet \"spot\"".into();
        w.arrival_radius_nm = 0.12;
        w.visible = false;
        w.time = Some(Utc.with_ymd_and_hms(2026, 5, 1, 10, 30, 0).unwrap());
        let mut r = Route::new("Brest - Camaret");
        r.speed_kn = 6.5;
        r.departure = Some(Utc.with_ymd_and_hms(2026, 6, 2, 7, 0, 0).unwrap());
        r.points.push(RoutePoint::new("start", LatLon::new(48.38, -4.49)));
        let mut p2 = RoutePoint::new("end", LatLon::new(48.28, -4.59));
        p2.arrival_radius_nm = 0.2;
        r.points.push(p2);
        let mut t = Track::new("morning");
        t.points.push(TrackPoint { pos: LatLon::new(48.1, -4.1), time: Some(Utc.with_ymd_and_hms(2026, 6, 2, 7, 0, 5).unwrap()) });
        t.points.push(TrackPoint { pos: LatLon::new(48.2, -4.2), time: None });
        GpxDoc { waypoints: vec![w], routes: vec![r], tracks: vec![t], warnings: vec![] }
    }

    #[test]
    fn write_then_parse_roundtrips() {
        let doc = sample();
        let back = parse(&write(&doc)).unwrap();
        assert!(back.warnings.is_empty());
        let (a, b) = (&doc.waypoints[0], &back.waypoints[0]);
        assert_eq!((a.name.as_str(), a.icon.as_str(), a.description.as_str()), (b.name.as_str(), b.icon.as_str(), b.description.as_str()));
        assert!((a.pos.lat - b.pos.lat).abs() < 1e-6 && (a.pos.lon - b.pos.lon).abs() < 1e-6);
        assert_eq!((a.arrival_radius_nm, a.visible, a.time), (b.arrival_radius_nm, b.visible, b.time));
        let (ra, rb) = (&doc.routes[0], &back.routes[0]);
        assert_eq!((&ra.name, ra.speed_kn, ra.departure), (&rb.name, rb.speed_kn, rb.departure));
        assert_eq!(ra.points.len(), rb.points.len());
        assert_eq!(rb.points[1].arrival_radius_nm, 0.2);
        assert_eq!(back.tracks[0].points.len(), 2);
        assert_eq!(back.tracks[0].points[0].time, doc.tracks[0].points[0].time);
        assert_eq!(back.tracks[0].points[1].time, None);
    }

    #[test]
    fn reads_an_opencpn_style_file() {
        let xml = r#"<?xml version="1.0"?>
<gpx version="1.1" creator="OpenCPN" xmlns="http://www.topografix.com/GPX/1/1" xmlns:opencpn="http://www.opencpn.org">
 <wpt lat="48.0" lon="-4.0"><name>M1</name><sym>diamond</sym>
  <extensions><opencpn:arrival_radius>0.25</opencpn:arrival_radius></extensions></wpt>
 <rte><name>R</name>
  <extensions><opencpn:planned_speed>7.0</opencpn:planned_speed></extensions>
  <rtept lat="48.0" lon="-4.0"><name>a</name></rtept>
  <rtept lat="48.1" lon="-4.1"><name>b</name></rtept></rte>
</gpx>"#;
        let d = parse(xml).unwrap();
        assert_eq!(d.waypoints[0].arrival_radius_nm, 0.25);
        assert_eq!(d.waypoints[0].icon, "diamond");
        assert_eq!(d.routes[0].speed_kn, 7.0);
        assert_eq!(d.routes[0].points.len(), 2);
    }

    #[test]
    fn plain_gpx_10_without_namespaces_works() {
        let d = parse(r#"<gpx version="1.0"><wpt lat="1" lon="2"><name>x</name></wpt></gpx>"#).unwrap();
        assert_eq!(d.waypoints.len(), 1);
    }

    #[test]
    fn bad_points_are_skipped_with_a_warning() {
        let d = parse(r#"<gpx><wpt lat="91" lon="0"/><wpt lat="x" lon="0"/><wpt lon="3"/><wpt lat="1" lon="1"/>
            <rte><rtept lat="1" lon="500"/><rtept lat="1" lon="1"/></rte></gpx>"#)
        .unwrap();
        assert_eq!(d.waypoints.len(), 1);
        assert_eq!(d.routes[0].points.len(), 1);
        assert_eq!(d.warnings.len(), 4);
    }

    #[test]
    fn rejects_non_gpx_and_garbage() {
        assert!(parse("<kml/>").is_err());
        assert!(parse("hello").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn dtds_are_refused() {
        let evil = "<?xml version=\"1.0\"?><!DOCTYPE gpx [<!ENTITY a \"aaaa\">]><gpx><wpt lat=\"1\" lon=\"1\"><name>&a;</name></wpt></gpx>";
        assert!(parse(evil).is_err());
    }

    #[test]
    fn escaping_keeps_the_xml_valid() {
        let out = write(&sample());
        assert!(Document::parse(&out).is_ok());
        assert!(out.contains("Mouillage &lt;A&amp;B&gt;"));
    }

    #[test]
    fn empty_document_roundtrips() {
        let back = parse(&write(&GpxDoc::default())).unwrap();
        assert!(back.waypoints.is_empty() && back.routes.is_empty() && back.tracks.is_empty());
    }
}
