//! JSON feeds: Signal K (REST document or delta messages) and gpsd.
//!
//! Both are turned into the same [`Nmea`] values the NMEA 0183 decoder
//! produces, so the rest of the app never needs to know where data came from.

use super::nmea::{Nmea, PilotMode};
use crate::geo::{LatLon, normalize_bearing};
use serde_json::Value;

const MS_TO_KN: f64 = 1.0 / 0.514_444;

/// Walks a Signal K vessel document and collects `(path, value)` for every
/// leaf that has a `value`.
pub fn flatten_document(v: &Value) -> Vec<(String, Value)> {
    fn walk(prefix: &str, v: &Value, out: &mut Vec<(String, Value)>) {
        let Some(obj) = v.as_object() else { return };
        if let Some(val) = obj.get("value") {
            out.push((prefix.to_string(), val.clone()));
            return;
        }
        for (k, child) in obj {
            if matches!(k.as_str(), "meta" | "timestamp" | "$source" | "source" | "sources") {
                continue;
            }
            let p = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
            walk(&p, child, out);
        }
    }
    let mut out = Vec::new();
    walk("", v, &mut out);
    out
}

/// `(path, value)` pairs from a delta message such as
/// `{"updates":[{"values":[{"path":"navigation.speedOverGround","value":3.0}]}]}`.
pub fn parse_delta(text: &str) -> Vec<(String, Value)> {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return Vec::new() };
    let mut out = Vec::new();
    for u in v.get("updates").and_then(|u| u.as_array()).into_iter().flatten() {
        for val in u.get("values").and_then(|u| u.as_array()).into_iter().flatten() {
            if let (Some(p), Some(x)) = (val.get("path").and_then(|p| p.as_str()), val.get("value")) {
                out.push((p.to_string(), x.clone()));
            }
        }
    }
    out
}

/// Converts one Signal K value (SI units, angles in radians) into an update.
pub fn to_nmea(path: &str, v: &Value) -> Option<Nmea> {
    let num = || v.as_f64();
    match path {
        "navigation.position" => {
            let (lat, lon) = (v.get("latitude")?.as_f64()?, v.get("longitude")?.as_f64()?);
            ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)).then(|| Nmea::Gll { pos: Some(LatLon::new(lat, lon)), valid: true })
        }
        "navigation.speedOverGround" => Some(Nmea::Vtg { cog_true: None, sog_kn: Some(num()? * MS_TO_KN) }),
        "navigation.courseOverGroundTrue" => Some(Nmea::Vtg { cog_true: Some(normalize_bearing(num()?.to_degrees())), sog_kn: None }),
        "navigation.headingTrue" => Some(Nmea::Hdt { heading: normalize_bearing(num()?.to_degrees()) }),
        "navigation.speedThroughWater" => Some(Nmea::Vhw { stw_kn: Some(num()? * MS_TO_KN), heading: None }),
        "environment.depth.belowTransducer" | "environment.depth.belowSurface" => Some(Nmea::Depth { metres: num()? }),
        "environment.water.temperature" => Some(Nmea::WaterTemp { celsius: num()? - 273.15 }),
        "navigation.gnss.horizontalDilution" => num().filter(|h| *h > 0.0).map(|hdop| Nmea::Hdop { hdop }),
        // Radians, roll positive to starboard, pitch positive bow up.
        "navigation.attitude" => {
            let (roll, pitch) = (v.get("roll").and_then(|x| x.as_f64()), v.get("pitch").and_then(|x| x.as_f64()));
            (roll.is_some() || pitch.is_some()).then(|| Nmea::Attitude { heel: roll.map(f64::to_degrees), pitch: pitch.map(f64::to_degrees) })
        }
        "steering.autopilot.state" => {
            let mode = match v.as_str()? {
                "standby" => PilotMode::Standby,
                "auto" => PilotMode::Heading,
                "wind" => PilotMode::Wind,
                "route" | "track" => PilotMode::Track,
                _ => return None,
            };
            Some(Nmea::Pilot { mode: Some(mode), heading: None, magnetic: false, wind_angle: None })
        }
        "steering.autopilot.target.headingTrue" | "steering.autopilot.target.headingMagnetic" => {
            Some(Nmea::Pilot { mode: None, heading: Some(normalize_bearing(num()?.to_degrees())), magnetic: path.ends_with("Magnetic"), wind_angle: None })
        }
        "steering.autopilot.target.windAngleApparent" => Some(Nmea::Pilot { mode: None, heading: None, magnetic: false, wind_angle: Some(num()?.to_degrees()) }),
        _ => engine(path, v),
    }
}

/// `propulsion.<id>.revolutions` (Hz) or `propulsion.<id>.state` (`started`, `stopped`).
fn engine(path: &str, v: &Value) -> Option<Nmea> {
    let rest = path.strip_prefix("propulsion.")?;
    if let Some(id) = rest.strip_suffix(".revolutions") {
        let rpm = v.as_f64()?.abs() * 60.0;
        return Some(Nmea::Engine { id: id.to_string(), running: rpm > 0.0, rpm: Some(rpm) });
    }
    let id = rest.strip_suffix(".state")?;
    let running = match v.as_str()? {
        "started" | "running" => true,
        "stopped" => false,
        _ => return None,
    };
    Some(Nmea::Engine { id: id.to_string(), running, rpm: None })
}

/// Apparent wind needs both angle and speed, so it is assembled from the pairs.
pub fn wind_from_pairs(pairs: &[(String, Value)]) -> Option<Nmea> {
    let get = |p: &str| pairs.iter().find(|(k, _)| k == p).and_then(|(_, v)| v.as_f64());
    let angle = get("environment.wind.angleApparent")?;
    let speed = get("environment.wind.speedApparent")?;
    Some(Nmea::Mwv { angle: normalize_bearing(angle.to_degrees()), true_wind: false, speed_kn: speed * MS_TO_KN })
}

/// The true wind as the server worked it out: through the water (an angle off the bow), else over the
/// ground (a direction). Applied after the apparent wind, so the server's own wins over ours.
fn true_wind_from_pairs(pairs: &[(String, Value)]) -> Option<Nmea> {
    let get = |p: &str| pairs.iter().find(|(k, _)| k == p).and_then(|(_, v)| v.as_f64());
    if let (Some(a), Some(s)) = (get("environment.wind.angleTrueWater"), get("environment.wind.speedTrue")) {
        return Some(Nmea::Mwv { angle: normalize_bearing(a.to_degrees()), true_wind: true, speed_kn: s * MS_TO_KN });
    }
    let (d, s) = (get("environment.wind.directionTrue")?, get("environment.wind.speedOverGround")?);
    Some(Nmea::Mwd { direction: normalize_bearing(d.to_degrees()), speed_kn: s * MS_TO_KN })
}

/// All updates in a set of Signal K pairs.
pub fn updates_from_pairs(pairs: &[(String, Value)]) -> Vec<Nmea> {
    let mut out: Vec<Nmea> = pairs.iter().filter_map(|(p, v)| to_nmea(p, v)).collect();
    out.extend(wind_from_pairs(pairs));
    out.extend(true_wind_from_pairs(pairs));
    out
}

/// A gpsd JSON line to updates. Only `TPV` reports with a 2D or 3D fix count.
pub fn gpsd_to_nmea(line: &str) -> Vec<Nmea> {
    let Ok(v) = serde_json::from_str::<Value>(line) else { return Vec::new() };
    if v.get("class").and_then(|c| c.as_str()) != Some("TPV") {
        return Vec::new();
    }
    if v.get("mode").and_then(|m| m.as_i64()).unwrap_or(0) < 2 {
        return Vec::new();
    }
    let mut out = Vec::new();
    if let (Some(lat), Some(lon)) = (v.get("lat").and_then(|x| x.as_f64()), v.get("lon").and_then(|x| x.as_f64()))
        && (-90.0..=90.0).contains(&lat)
        && (-180.0..=180.0).contains(&lon)
    {
        out.push(Nmea::Gll { pos: Some(LatLon::new(lat, lon)), valid: true });
    }
    // The horizontal error at 95%: `eph` in gpsd 3.20 and later, else the two axes' (each at 95%).
    let num = |k: &str| v.get(k).and_then(|x| x.as_f64()).filter(|x| x.is_finite() && *x > 0.0);
    if let Some(metres) = num("eph").or_else(|| num("epx").zip(num("epy")).map(|(x, y)| x.hypot(y))) {
        out.push(Nmea::Accuracy { metres });
    }
    let sog = v.get("speed").and_then(|x| x.as_f64()).map(|s| s * MS_TO_KN);
    let cog = v.get("track").and_then(|x| x.as_f64()).map(normalize_bearing);
    if sog.is_some() || cog.is_some() {
        out.push(Nmea::Vtg { cog_true: cog, sog_kn: sog });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn flattens_a_vessel_document() {
        let doc = json!({
            "navigation": {
                "position": {"value": {"latitude": 48.0, "longitude": -4.5}, "timestamp": "2026-01-01T00:00:00Z"},
                "speedOverGround": {"value": 2.5, "$source": "x", "meta": {"units": "m/s"}},
            },
            "environment": {"depth": {"belowTransducer": {"value": 7.2}}},
            "uuid": "urn:mrn:signalk:uuid:abc"
        });
        let mut pairs = flatten_document(&doc);
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        let paths: Vec<_> = pairs.iter().map(|p| p.0.as_str()).collect();
        assert_eq!(paths, ["environment.depth.belowTransducer", "navigation.position", "navigation.speedOverGround"]);
    }

    #[test]
    fn engines_attitude_and_true_wind() {
        let pairs = parse_delta(
            r#"{"updates":[{"values":[
            {"path":"propulsion.port.revolutions","value":30.0},
            {"path":"propulsion.starboard.state","value":"stopped"},
            {"path":"navigation.attitude","value":{"roll":0.1745,"pitch":-0.0175,"yaw":1.0}},
            {"path":"environment.wind.angleTrueWater","value":-0.7854},
            {"path":"environment.wind.speedTrue","value":5.14444}]}]}"#,
        );
        let ups = updates_from_pairs(&pairs);
        assert!(ups.contains(&Nmea::Engine { id: "port".into(), running: true, rpm: Some(1800.0) }));
        assert!(ups.contains(&Nmea::Engine { id: "starboard".into(), running: false, rpm: None }));
        let Some(Nmea::Attitude { heel: Some(h), pitch: Some(p) }) = ups.iter().find(|n| matches!(n, Nmea::Attitude { .. })).cloned() else { panic!() };
        assert!((h - 10.0).abs() < 0.01 && (p + 1.0).abs() < 0.01);
        let Some(Nmea::Mwv { angle, true_wind: true, speed_kn }) = ups.iter().find(|n| matches!(n, Nmea::Mwv { .. })).cloned() else { panic!() };
        assert!((angle - 315.0).abs() < 0.01 && (speed_kn - 10.0).abs() < 1e-3);
        // A stopped engine reports no revolutions.
        assert_eq!(to_nmea("propulsion.main.revolutions", &json!(0.0)), Some(Nmea::Engine { id: "main".into(), running: false, rpm: Some(0.0) }));
        assert_eq!(to_nmea("propulsion.main.temperature", &json!(350.0)), None);
    }

    #[test]
    fn converts_units() {
        let n = to_nmea("navigation.speedOverGround", &json!(5.14444)).unwrap();
        let Nmea::Vtg { sog_kn, .. } = n else { panic!() };
        assert!((sog_kn.unwrap() - 10.0).abs() < 1e-3);
        let n = to_nmea("navigation.courseOverGroundTrue", &json!(std::f64::consts::PI)).unwrap();
        let Nmea::Vtg { cog_true, .. } = n else { panic!() };
        assert!((cog_true.unwrap() - 180.0).abs() < 1e-9);
        // Negative angles wrap.
        let Nmea::Hdt { heading } = to_nmea("navigation.headingTrue", &json!(-std::f64::consts::FRAC_PI_2)).unwrap() else { panic!() };
        assert!((heading - 270.0).abs() < 1e-9);
        let Nmea::WaterTemp { celsius } = to_nmea("environment.water.temperature", &json!(290.15)).unwrap() else { panic!() };
        assert!((celsius - 17.0).abs() < 1e-9);
    }

    #[test]
    fn autopilot_state_and_targets() {
        assert_eq!(to_nmea("steering.autopilot.state", &json!("wind")), Some(Nmea::Pilot { mode: Some(PilotMode::Wind), heading: None, magnetic: false, wind_angle: None }));
        assert_eq!(to_nmea("steering.autopilot.state", &json!("dancing")), None);
        let Some(Nmea::Pilot { heading: Some(h), magnetic: true, .. }) = to_nmea("steering.autopilot.target.headingMagnetic", &json!(std::f64::consts::FRAC_PI_2)) else { panic!() };
        assert!((h - 90.0).abs() < 1e-9);
        let Some(Nmea::Pilot { wind_angle: Some(a), .. }) = to_nmea("steering.autopilot.target.windAngleApparent", &json!(-std::f64::consts::FRAC_PI_4)) else { panic!() };
        assert!((a + 45.0).abs() < 1e-9);
    }

    #[test]
    fn position_validation() {
        assert!(to_nmea("navigation.position", &json!({"latitude": 48.0, "longitude": -4.0})).is_some());
        assert!(to_nmea("navigation.position", &json!({"latitude": 98.0, "longitude": -4.0})).is_none());
        assert!(to_nmea("navigation.position", &json!({"latitude": 48.0})).is_none());
        assert!(to_nmea("navigation.position", &json!("nope")).is_none());
        assert!(to_nmea("navigation.unknown", &json!(1)).is_none());
        assert!(to_nmea("navigation.speedOverGround", &json!("fast")).is_none());
    }

    #[test]
    fn deltas_are_parsed_and_junk_is_ignored() {
        let text = r#"{"context":"vessels.self","updates":[{"values":[
            {"path":"navigation.speedOverGround","value":3.0},
            {"path":"environment.wind.angleApparent","value":0.5},
            {"path":"environment.wind.speedApparent","value":5.14444}]}]}"#;
        let pairs = parse_delta(text);
        assert_eq!(pairs.len(), 3);
        let ups = updates_from_pairs(&pairs);
        assert_eq!(ups.len(), 2);
        let Some(Nmea::Mwv { angle, speed_kn, true_wind }) = ups.into_iter().find(|u| matches!(u, Nmea::Mwv { .. })) else { panic!() };
        assert!((angle - 28.6479).abs() < 1e-3 && (speed_kn - 10.0).abs() < 1e-3 && !true_wind);
        assert!(parse_delta("not json").is_empty());
        assert!(parse_delta("{}").is_empty());
        assert!(parse_delta(r#"{"updates":[{"values":[{"path":"x"}]}]}"#).is_empty());
    }

    #[test]
    fn gpsd_tpv() {
        let line = r#"{"class":"TPV","device":"/dev/ttyUSB0","mode":3,"lat":48.38,"lon":-4.49,"speed":2.572,"track":45.0}"#;
        let u = gpsd_to_nmea(line);
        assert_eq!(u.len(), 2);
        assert!(matches!(u[0], Nmea::Gll { pos: Some(_), valid: true }));
        let Nmea::Vtg { sog_kn, cog_true } = &u[1] else { panic!() };
        assert!((sog_kn.unwrap() - 5.0).abs() < 1e-2 && *cog_true == Some(45.0));
    }

    #[test]
    fn gpsd_and_signal_k_give_the_fix_error() {
        let acc = |line: &str| gpsd_to_nmea(line).into_iter().find_map(|n| if let Nmea::Accuracy { metres } = n { Some(metres) } else { None });
        assert_eq!(acc(r#"{"class":"TPV","mode":3,"lat":48.38,"lon":-4.49,"eph":7.5,"epx":3.0,"epy":4.0}"#), Some(7.5));
        assert_eq!(acc(r#"{"class":"TPV","mode":3,"lat":48.38,"lon":-4.49,"epx":3.0,"epy":4.0}"#), Some(5.0));
        assert_eq!(acc(r#"{"class":"TPV","mode":3,"lat":48.38,"lon":-4.49}"#), None);
        assert_eq!(to_nmea("navigation.gnss.horizontalDilution", &json!(1.2)), Some(Nmea::Hdop { hdop: 1.2 }));
    }

    #[test]
    fn gpsd_ignores_other_classes_and_no_fix() {
        assert!(gpsd_to_nmea(r#"{"class":"SKY","satellites":[]}"#).is_empty());
        assert!(gpsd_to_nmea(r#"{"class":"TPV","mode":1}"#).is_empty());
        assert!(gpsd_to_nmea(r#"{"class":"TPV","mode":2,"lat":99.0,"lon":0.0}"#).is_empty());
        assert!(gpsd_to_nmea("garbage").is_empty());
        assert!(gpsd_to_nmea(r#"{"class":"VERSION"}"#).is_empty());
    }
}
