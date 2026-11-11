//! NMEA 0183 sentences: framing, checksums, and the decoders for the
//! sentences a chart plotter cares about.

use crate::geo::LatLon;
use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};

const MS_TO_KN: f64 = 1.0 / 0.514_444;
const KMH_TO_KN: f64 = 1.0 / 1.852;

#[derive(Debug, PartialEq, Eq)]
pub enum NmeaError {
    /// Not a `$...` or `!...` sentence.
    NotASentence,
    BadChecksum { expected: u8, found: u8 },
    /// Address field too short to hold talker and type.
    BadAddress,
}

impl std::fmt::Display for NmeaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NmeaError::NotASentence => write!(f, "not an NMEA sentence"),
            NmeaError::BadChecksum { expected, found } => {
                write!(f, "checksum mismatch: computed {expected:02X}, sentence says {found:02X}")
            }
            NmeaError::BadAddress => write!(f, "bad address field"),
        }
    }
}

impl std::error::Error for NmeaError {}

/// XOR of every byte between `$`/`!` and `*`.
pub fn checksum(body: &str) -> u8 {
    body.bytes().fold(0, |a, b| a ^ b)
}

/// Wraps `body` (without delimiters) as `$body*CS`.
pub fn frame(body: &str, prefix: char) -> String {
    format!("{prefix}{body}*{:02X}", checksum(body))
}

/// How the autopilot steers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PilotMode {
    /// Not steering: the helm is by hand.
    Standby,
    /// Holding a compass heading.
    Heading,
    /// Holding an angle to the apparent wind.
    Wind,
    /// Following the route's track.
    Track,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sentence {
    /// `$` for normal sentences, `!` for encapsulated ones such as AIS.
    pub prefix: char,
    /// Two-letter talker id, e.g. `GP`, `II`, `AI`.
    pub talker: String,
    /// Three-letter sentence type, e.g. `RMC`.
    pub kind: String,
    pub fields: Vec<String>,
}

impl Sentence {
    pub fn field(&self, i: usize) -> &str {
        self.fields.get(i).map(|s| s.as_str()).unwrap_or("")
    }

    fn num(&self, i: usize) -> Option<f64> {
        self.field(i).parse().ok()
    }
}

/// Parses one line. A sentence without a checksum is accepted (many simple
/// instruments omit it), one with a wrong checksum is not.
pub fn parse_sentence(line: &str) -> Result<Sentence, NmeaError> {
    let line = line.trim();
    let prefix = line.chars().next().ok_or(NmeaError::NotASentence)?;
    if prefix != '$' && prefix != '!' {
        return Err(NmeaError::NotASentence);
    }
    let rest = &line[1..];
    let (body, cs) = match rest.split_once('*') {
        Some((b, c)) => (b, Some(c)),
        None => (rest, None),
    };
    if let Some(cs) = cs {
        let found = u8::from_str_radix(cs.trim().get(..2).unwrap_or(cs.trim()), 16).map_err(|_| NmeaError::NotASentence)?;
        let expected = checksum(body);
        if found != expected {
            return Err(NmeaError::BadChecksum { expected, found });
        }
    }
    let mut parts = body.split(',');
    let addr = parts.next().unwrap_or("");
    if addr.len() < 5 || !addr.is_ascii() {
        return Err(NmeaError::BadAddress);
    }
    // Proprietary sentences start with P and have a 3-letter maker code.
    let (talker, kind) = match addr.strip_prefix('P') {
        Some(rest) => ("P".to_string(), rest.to_string()),
        None => (addr[..2].to_string(), addr[2..5].to_string()),
    };
    Ok(Sentence { prefix, talker, kind, fields: parts.map(|s| s.to_string()).collect() })
}

/// `ddmm.mmmm` and a hemisphere letter to signed degrees.
pub fn parse_coord(value: &str, hemi: &str, deg_digits: usize) -> Option<f64> {
    if value.len() < deg_digits + 2 || !value.is_ascii() {
        return None;
    }
    let deg: f64 = value[..deg_digits].parse().ok()?;
    let min: f64 = value[deg_digits..].parse().ok()?;
    if min >= 60.0 {
        return None;
    }
    let v = deg + min / 60.0;
    match hemi {
        "N" | "E" => Some(v),
        "S" | "W" => Some(-v),
        _ => None,
    }
}

fn parse_pos(s: &Sentence, i: usize) -> Option<LatLon> {
    let lat = parse_coord(s.field(i), s.field(i + 1), 2)?;
    let lon = parse_coord(s.field(i + 2), s.field(i + 3), 3)?;
    ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)).then(|| LatLon::new(lat, lon))
}

fn parse_time(t: &str) -> Option<NaiveTime> {
    if t.len() < 6 || !t.is_ascii() {
        return None;
    }
    let (h, m): (u32, u32) = (t[0..2].parse().ok()?, t[2..4].parse().ok()?);
    let sec: f64 = t[4..].parse().ok()?;
    NaiveTime::from_hms_milli_opt(h, m, sec.trunc() as u32, ((sec.fract()) * 1000.0).round() as u32)
}

fn parse_date(d: &str) -> Option<NaiveDate> {
    if d.len() != 6 || !d.is_ascii() {
        return None;
    }
    let (day, month, yy): (u32, u32, i32) = (d[0..2].parse().ok()?, d[2..4].parse().ok()?, d[4..6].parse().ok()?);
    NaiveDate::from_ymd_opt(if yy >= 80 { 1900 + yy } else { 2000 + yy }, month, day)
}

fn signed(value: Option<f64>, hemi: &str) -> Option<f64> {
    value.map(|v| if hemi == "W" || hemi == "S" { -v } else { v })
}

/// What a decoded sentence tells us.
#[derive(Clone, Debug, PartialEq)]
pub enum Nmea {
    Rmc { time: Option<DateTime<Utc>>, valid: bool, pos: Option<LatLon>, sog_kn: Option<f64>, cog: Option<f64>, variation: Option<f64> },
    Gga { pos: Option<LatLon>, quality: u8, satellites: Option<u32>, hdop: Option<f64> },
    Gll { pos: Option<LatLon>, valid: bool },
    Vtg { cog_true: Option<f64>, sog_kn: Option<f64> },
    /// Satellites in view of one constellation (the talker: GP, GL, GA, GB…).
    Gsv { talker: String, in_view: u32 },
    /// The receiver's own estimate of its horizontal error: the radius in metres it is within 95% of the time
    /// (GST, gpsd's `eph`).
    Accuracy { metres: f64 },
    /// Horizontal dilution of precision alone (Signal K's `navigation.gnss.horizontalDilution`).
    Hdop { hdop: f64 },
    Hdt { heading: f64 },
    Hdm { heading: f64 },
    Hdg { heading: Option<f64>, deviation: Option<f64>, variation: Option<f64> },
    /// Wind angle in degrees (0..360, from the bow clockwise for apparent) and speed in knots.
    Mwv { angle: f64, true_wind: bool, speed_kn: f64 },
    /// True wind direction (from) and speed.
    Mwd { direction: f64, speed_kn: f64 },
    Depth { metres: f64 },
    Vhw { stw_kn: Option<f64>, heading: Option<f64> },
    WaterTemp { celsius: f64 },
    Rmb { xte_nm: Option<f64>, dest: String, dest_pos: Option<LatLon>, bearing: Option<f64>, range_nm: Option<f64> },
    /// An engine running or not (`$--RPM` with its revolutions, Signal K `propulsion.<id>.revolutions` or
    /// `.state`). `id` tells the engines apart (`E1`, `S2`, `port`); `rpm` is unsigned (astern is negative in RPM).
    Engine { id: String, running: bool, rpm: Option<f64> },
    /// Heel (positive to starboard) and pitch (positive bow up), degrees: `$--XDR` angular transducers, Signal K
    /// `navigation.attitude`.
    Attitude { heel: Option<f64>, pitch: Option<f64> },
    /// What the autopilot is doing: `$--HTD` (its mode and the heading or track it steers), or one of
    /// Signal K's `steering.autopilot.*` values (each comes alone, so every field is optional).
    Pilot { mode: Option<PilotMode>, heading: Option<f64>, magnetic: bool, wind_angle: Option<f64> },
    /// An AIVDM/AIVDO sentence, left for the AIS decoder.
    Ais(Sentence),
}

pub fn decode(s: &Sentence) -> Option<Nmea> {
    match s.kind.as_str() {
        "RMC" => {
            let time = parse_date(s.field(8)).zip(parse_time(s.field(0))).map(|(d, t)| Utc.from_utc_datetime(&d.and_time(t)));
            Some(Nmea::Rmc {
                time,
                valid: s.field(1) == "A",
                pos: parse_pos(s, 2),
                sog_kn: s.num(6),
                cog: s.num(7),
                variation: signed(s.num(9), s.field(10)),
            })
        }
        "GGA" => Some(Nmea::Gga {
            pos: parse_pos(s, 1),
            quality: s.field(5).parse().unwrap_or(0),
            satellites: s.field(6).parse().ok(),
            hdop: s.num(7),
        }),
        "GLL" => Some(Nmea::Gll { pos: parse_pos(s, 0), valid: s.field(5) == "A" || s.field(5).is_empty() }),
        "VTG" => Some(Nmea::Vtg { cog_true: s.num(0), sog_kn: s.num(4).or_else(|| s.num(6).map(|k| k * KMH_TO_KN)) }),
        "GSV" => s.field(2).parse().ok().map(|n| Nmea::Gsv { talker: s.talker.clone(), in_view: n }),
        // Standard deviations of the latitude and longitude errors, in metres: 2DRMS is the 95% radius.
        "GST" => s.num(5).zip(s.num(6)).map(|(lat, lon)| Nmea::Accuracy { metres: 2.0 * lat.hypot(lon) }),
        "HDT" => s.num(0).map(|h| Nmea::Hdt { heading: h }),
        "HDM" => s.num(0).map(|h| Nmea::Hdm { heading: h }),
        "HDG" => Some(Nmea::Hdg { heading: s.num(0), deviation: signed(s.num(1), s.field(2)), variation: signed(s.num(3), s.field(4)) }),
        "MWV" => {
            if s.field(4) == "V" {
                return None; // data invalid
            }
            let speed = s.num(2)?;
            let kn = match s.field(3) {
                "K" => speed * KMH_TO_KN,
                "M" => speed * MS_TO_KN,
                _ => speed,
            };
            Some(Nmea::Mwv { angle: s.num(0)?, true_wind: s.field(1) == "T", speed_kn: kn })
        }
        "MWD" => Some(Nmea::Mwd {
            direction: s.num(0)?,
            speed_kn: s.num(4).or_else(|| s.num(6).map(|m| m * MS_TO_KN))?,
        }),
        "DBT" => s.num(2).or_else(|| s.num(0).map(|f| f * 0.3048)).map(|m| Nmea::Depth { metres: m }),
        "DPT" => s.num(0).map(|m| Nmea::Depth { metres: m + s.num(1).unwrap_or(0.0).max(0.0) }),
        "VHW" => Some(Nmea::Vhw { stw_kn: s.num(4), heading: s.num(2).or_else(|| s.num(0)) }),
        "MTW" => s.num(0).map(|c| Nmea::WaterTemp { celsius: c }),
        "RMB" => Some(Nmea::Rmb {
            xte_nm: s.num(1).map(|x| if s.field(2) == "L" { -x } else { x }),
            dest: s.field(4).to_string(),
            dest_pos: parse_pos(s, 5),
            range_nm: s.num(9),
            bearing: s.num(10),
        }),
        "RPM" => {
            // Source (S shaft, E engine), its number, revolutions per minute, pitch, status.
            if s.field(4) == "V" {
                return None;
            }
            let rpm = s.num(2)?.abs();
            Some(Nmea::Engine { id: format!("{}{}", s.field(0), s.field(1)), running: rpm > 0.0, rpm: Some(rpm) })
        }
        "XDR" => decode_xdr(s),
        // Heading/track control data (IEC 61162-1): the steering mode in field 3 (M manual, S stand-alone
        // heading, H heading control, T track, R rudder), the heading to steer in 9, the track in 11, T or M
        // in 12.
        "HTD" => {
            let mode = match s.field(3) {
                "S" | "H" => PilotMode::Heading,
                "T" => PilotMode::Track,
                "M" | "R" => PilotMode::Standby,
                _ => return None,
            };
            let heading = if mode == PilotMode::Track { s.num(11).or(s.num(9)) } else { s.num(9) };
            Some(Nmea::Pilot { mode: Some(mode), heading, magnetic: s.field(12) == "M", wind_angle: None })
        }
        "VDM" | "VDO" if s.prefix == '!' => Some(Nmea::Ais(s.clone())),
        _ => None,
    }
}

/// Heel and pitch from a transducer sentence: quadruplets of type, value, unit and name, of which the
/// angular ones (`A`, in degrees) named for roll or pitch are read. Names vary by maker (`ROLL`, `Heel`,
/// `PTCH`, `PITCH`).
fn decode_xdr(s: &Sentence) -> Option<Nmea> {
    let (mut heel, mut pitch) = (None, None);
    for q in (0..s.fields.len()).step_by(4) {
        if s.field(q) != "A" || !matches!(s.field(q + 2), "D" | "") {
            continue;
        }
        let Some(v) = s.num(q + 1) else { continue };
        let name = s.field(q + 3).to_ascii_uppercase();
        if name.contains("ROLL") || name.contains("HEEL") {
            heel = Some(v);
        } else if name.contains("PTCH") || name.contains("PITCH") {
            pitch = Some(v);
        }
    }
    (heel.is_some() || pitch.is_some()).then_some(Nmea::Attitude { heel, pitch })
}

/// Convenience: parse a line and decode it.
pub fn parse_line(line: &str) -> Result<Option<Nmea>, NmeaError> {
    parse_sentence(line).map(|s| decode(&s))
}

/// Builds `$GPRMC` as a boat simulator or a test would emit it.
pub fn make_rmc(time: DateTime<Utc>, pos: &LatLon, sog_kn: f64, cog: f64) -> String {
    let (lat_d, lat_m) = split_deg(pos.lat.abs());
    let (lon_d, lon_m) = split_deg(pos.lon.abs());
    let body = format!(
        "GPRMC,{},A,{:02}{:07.4},{},{:03}{:07.4},{},{:.1},{:.1},{},,,A",
        time.format("%H%M%S"),
        lat_d,
        lat_m,
        if pos.lat >= 0.0 { 'N' } else { 'S' },
        lon_d,
        lon_m,
        if pos.lon >= 0.0 { 'E' } else { 'W' },
        sog_kn,
        cog,
        time.format("%d%m%y"),
    );
    frame(&body, '$')
}

fn split_deg(abs: f64) -> (u32, f64) {
    let mut d = abs.floor();
    let mut m = (abs - d) * 60.0;
    if m >= 59.99995 {
        d += 1.0;
        m = 0.0;
    }
    (d as u32, m)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RMC: &str = "$GPRMC,123519,A,4807.038,N,01131.000,E,022.4,084.4,230394,003.1,W*6A";

    #[test]
    fn checksum_of_a_known_sentence() {
        assert_eq!(checksum("GPRMC,123519,A,4807.038,N,01131.000,E,022.4,084.4,230394,003.1,W"), 0x6A);
    }

    #[test]
    fn parses_the_textbook_rmc() {
        let n = parse_line(RMC).unwrap().unwrap();
        let Nmea::Rmc { time, valid, pos, sog_kn, cog, variation } = n else { panic!("{n:?}") };
        assert!(valid);
        let p = pos.unwrap();
        assert!((p.lat - 48.1173).abs() < 1e-4 && (p.lon - 11.5167).abs() < 1e-4);
        assert_eq!((sog_kn, cog), (Some(22.4), Some(84.4)));
        assert_eq!(variation, Some(-3.1));
        assert_eq!(time.unwrap().to_rfc3339(), "1994-03-23T12:35:19+00:00");
    }

    #[test]
    fn rejects_bad_checksums_but_accepts_missing_ones() {
        let bad = RMC.replace("*6A", "*6B");
        assert!(matches!(parse_sentence(&bad), Err(NmeaError::BadChecksum { expected: 0x6A, found: 0x6B })));
        let none = RMC.split('*').next().unwrap();
        assert!(parse_sentence(none).is_ok());
    }

    #[test]
    fn rejects_junk() {
        assert_eq!(parse_sentence(""), Err(NmeaError::NotASentence));
        assert_eq!(parse_sentence("hello"), Err(NmeaError::NotASentence));
        assert_eq!(parse_sentence("$GP*17"), Err(NmeaError::BadAddress));
        assert!(matches!(parse_sentence("$GP*00"), Err(NmeaError::BadChecksum { .. })));
        assert!(parse_sentence("$GPRMC,1*zz").is_err());
        assert!(parse_sentence("$é,é,é").is_err());
    }

    #[test]
    fn coordinates() {
        assert!((parse_coord("4807.038", "N", 2).unwrap() - 48.1173).abs() < 1e-4);
        assert!((parse_coord("01131.000", "W", 3).unwrap() + 11.5167).abs() < 1e-4);
        assert_eq!(parse_coord("4860.000", "N", 2), None);
        assert_eq!(parse_coord("48", "N", 2), None);
        assert_eq!(parse_coord("4807.0", "X", 2), None);
        assert_eq!(parse_coord("", "N", 2), None);
    }

    #[test]
    fn void_fix_is_flagged_and_empty_fields_are_none() {
        let s = frame("GPRMC,123519,V,,,,,,,230394,,", '$');
        let Some(Nmea::Rmc { valid, pos, sog_kn, .. }) = parse_line(&s).unwrap() else { panic!() };
        assert!(!valid && pos.is_none() && sog_kn.is_none());
    }

    #[test]
    fn gga_vtg_hdt() {
        let gga = frame("GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,", '$');
        let Some(Nmea::Gga { pos, quality, satellites, hdop }) = parse_line(&gga).unwrap() else { panic!() };
        assert!(pos.is_some());
        assert_eq!((quality, satellites, hdop), (1, Some(8), Some(0.9)));
        let vtg = frame("GPVTG,054.7,T,034.4,M,005.5,N,010.2,K,A", '$');
        assert_eq!(parse_line(&vtg).unwrap(), Some(Nmea::Vtg { cog_true: Some(54.7), sog_kn: Some(5.5) }));
        let hdt = frame("HEHDT,274.07,T", '$');
        assert_eq!(parse_line(&hdt).unwrap(), Some(Nmea::Hdt { heading: 274.07 }));
    }

    #[test]
    fn engine_revolutions() {
        let rpm = frame("ERRPM,E,1,1850.0,,A", '$');
        assert_eq!(parse_line(&rpm).unwrap(), Some(Nmea::Engine { id: "E1".into(), running: true, rpm: Some(1850.0) }));
        // Astern is negative; stopped is zero; void data says nothing.
        let Some(Nmea::Engine { rpm, .. }) = parse_line(&frame("ERRPM,S,2,-900,,A", '$')).unwrap() else { panic!() };
        assert_eq!(rpm, Some(900.0));
        let Some(Nmea::Engine { running, .. }) = parse_line(&frame("ERRPM,E,1,0,,A", '$')).unwrap() else { panic!() };
        assert!(!running);
        assert_eq!(parse_line(&frame("ERRPM,E,1,1850,,V", '$')).unwrap(), None);
    }

    #[test]
    fn transducers_give_heel_and_pitch() {
        let xdr = frame("IIXDR,A,-12.5,D,ROLL,A,2.0,D,PTCH,C,18.0,C,AIRTEMP", '$');
        assert_eq!(parse_line(&xdr).unwrap(), Some(Nmea::Attitude { heel: Some(-12.5), pitch: Some(2.0) }));
        let xdr = frame("YXXDR,A,8,D,Heel Angle", '$');
        assert_eq!(parse_line(&xdr).unwrap(), Some(Nmea::Attitude { heel: Some(8.0), pitch: None }));
        // Other transducers alone are not an attitude.
        assert_eq!(parse_line(&frame("IIXDR,P,1.013,B,BARO", '$')).unwrap(), None);
    }

    #[test]
    fn htd_gives_the_autopilot_mode_and_heading() {
        let htd = frame("APHTD,V,,,S,,,,,,130.0,,,M,,,,128.5", '$');
        assert_eq!(parse_line(&htd).unwrap(), Some(Nmea::Pilot { mode: Some(PilotMode::Heading), heading: Some(130.0), magnetic: true, wind_angle: None }));
        // Track control steers the commanded track (field 11).
        let htd = frame("APHTD,V,,,T,,,,,,130.0,,95.0,T,,,,128.5", '$');
        assert_eq!(parse_line(&htd).unwrap(), Some(Nmea::Pilot { mode: Some(PilotMode::Track), heading: Some(95.0), magnetic: false, wind_angle: None }));
        let htd = frame("APHTD,V,,,M,,,,,,,,,T,,,,128.5", '$');
        assert!(matches!(parse_line(&htd).unwrap(), Some(Nmea::Pilot { mode: Some(PilotMode::Standby), heading: None, .. })));
    }

    #[test]
    fn gst_gives_the_95_percent_radius() {
        // Latitude error 1.8 m, longitude 2.4 m, one standard deviation each: 2DRMS is 6 m.
        let gst = frame("GPGST,172814.0,0.006,0.023,0.020,273.6,1.8,2.4,0.04", '$');
        let Some(Nmea::Accuracy { metres }) = parse_line(&gst).unwrap() else { panic!() };
        assert!((metres - 6.0).abs() < 1e-9);
        assert_eq!(parse_line(&frame("GPGST,172814.0,,,,,,,", '$')).unwrap(), None);
    }

    #[test]
    fn vtg_falls_back_to_kmh() {
        let vtg = frame("GPVTG,054.7,T,,M,,N,18.52,K,A", '$');
        let Some(Nmea::Vtg { sog_kn, .. }) = parse_line(&vtg).unwrap() else { panic!() };
        assert!((sog_kn.unwrap() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn wind_units_are_converted_to_knots() {
        let kn = frame("IIMWV,045,R,12.0,N,A", '$');
        assert_eq!(parse_line(&kn).unwrap(), Some(Nmea::Mwv { angle: 45.0, true_wind: false, speed_kn: 12.0 }));
        let ms = frame("IIMWV,270,T,5.14444,M,A", '$');
        let Some(Nmea::Mwv { speed_kn, true_wind, .. }) = parse_line(&ms).unwrap() else { panic!() };
        assert!(true_wind && (speed_kn - 10.0).abs() < 0.01);
        let kmh = frame("IIMWV,270,R,18.52,K,A", '$');
        let Some(Nmea::Mwv { speed_kn, .. }) = parse_line(&kmh).unwrap() else { panic!() };
        assert!((speed_kn - 10.0).abs() < 1e-9);
        // Invalid data is ignored.
        assert_eq!(parse_line(&frame("IIMWV,270,R,18.52,K,V", '$')).unwrap(), None);
    }

    #[test]
    fn depth_sentences() {
        assert_eq!(parse_line(&frame("SDDBT,12.3,f,3.7,M,2.0,F", '$')).unwrap(), Some(Nmea::Depth { metres: 3.7 }));
        let Some(Nmea::Depth { metres }) = parse_line(&frame("SDDBT,10.0,f,,M,,F", '$')).unwrap() else { panic!() };
        assert!((metres - 3.048).abs() < 1e-9, "feet fallback");
        // DPT adds the transducer offset.
        assert_eq!(parse_line(&frame("SDDPT,3.7,0.5", '$')).unwrap(), Some(Nmea::Depth { metres: 4.2 }));
        // A negative offset (keel) is not added.
        assert_eq!(parse_line(&frame("SDDPT,3.7,-0.5", '$')).unwrap(), Some(Nmea::Depth { metres: 3.7 }));
    }

    #[test]
    fn rmb_cross_track_sign() {
        let s = frame("GPRMB,A,0.66,L,001,002,4917.24,N,12309.57,W,001.3,052.5,000.5,V", '$');
        let Some(Nmea::Rmb { xte_nm, dest, dest_pos, bearing, range_nm }) = parse_line(&s).unwrap() else { panic!() };
        assert_eq!(xte_nm, Some(-0.66));
        assert_eq!(dest, "002");
        assert!(dest_pos.is_some());
        assert_eq!((bearing, range_nm), (Some(52.5), Some(1.3)));
    }

    #[test]
    fn proprietary_and_unknown_sentences_are_not_decoded() {
        let s = parse_sentence("$PGRMZ,93,f,3*21").unwrap();
        assert_eq!((s.talker.as_str(), s.kind.as_str()), ("P", "GRMZ"));
        assert_eq!(decode(&s), None);
        assert_eq!(parse_line(&frame("GPXXX,1,2", '$')).unwrap(), None);
    }

    #[test]
    fn ais_sentences_are_passed_on() {
        let s = parse_sentence("!AIVDM,1,1,,B,177KQJ5000G?tO`K>RA1wUbN0TKH,0*5C").unwrap();
        assert!(matches!(decode(&s), Some(Nmea::Ais(_))));
        // With the $ prefix it's not AIS.
        let s2 = Sentence { prefix: '$', ..s };
        assert_eq!(decode(&s2), None);
    }

    #[test]
    fn make_rmc_roundtrips() {
        let t = Utc.with_ymd_and_hms(2026, 6, 1, 12, 30, 15).unwrap();
        let p = LatLon::new(-33.8567, 151.2153);
        let line = make_rmc(t, &p, 6.5, 123.4);
        let Some(Nmea::Rmc { time, pos, sog_kn, cog, valid, .. }) = parse_line(&line).unwrap() else { panic!("{line}") };
        assert!(valid);
        let q = pos.unwrap();
        assert!((q.lat - p.lat).abs() < 1e-5 && (q.lon - p.lon).abs() < 1e-5);
        assert_eq!((sog_kn, cog, time), (Some(6.5), Some(123.4), Some(t)));
    }

    #[test]
    fn minutes_that_round_up_carry_into_degrees() {
        let t = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        let line = make_rmc(t, &LatLon::new(48.999_999_9, -4.0), 0.0, 0.0);
        let Some(Nmea::Rmc { pos, .. }) = parse_line(&line).unwrap() else { panic!() };
        assert!((pos.unwrap().lat - 49.0).abs() < 1e-5, "{line}");
    }
}
