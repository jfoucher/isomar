//! Own-ship state assembled from instrument data.

use super::nmea::{Nmea, PilotMode};
use crate::geo::{LatLon, normalize_bearing};
use crate::nav::calc::Fix;
use chrono::{DateTime, Duration, Utc};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub struct OwnShip {
    pub pos: Option<LatLon>,
    pub sog_kn: Option<f64>,
    /// Course over ground, true.
    pub cog: Option<f64>,
    /// Heading, true.
    pub heading: Option<f64>,
    pub stw_kn: Option<f64>,
    pub depth_m: Option<f64>,
    pub water_temp_c: Option<f64>,
    /// Magnetic variation, east positive.
    pub variation: Option<f64>,
    /// Apparent wind: angle off the bow (-180..180, positive to starboard) and speed.
    pub wind_apparent: Option<(f64, f64)>,
    /// True wind as direction it comes from (0..360) and speed.
    pub wind_true: Option<(f64, f64)>,
    pub satellites: Option<u32>,
    pub hdop: Option<f64>,
    /// When the HDOP was last given, by our own clock.
    pub hdop_at: Option<DateTime<Utc>>,
    /// GGA's fix quality: 1 GPS, 2 DGPS or SBAS, 4 RTK fixed, 5 RTK float, 6 dead reckoning.
    pub fix_quality: Option<u8>,
    /// The receiver's own 95% radius of its horizontal error in metres (GST, gpsd), and when it came.
    pub error_m: Option<(f64, DateTime<Utc>)>,
    pub fix_time: Option<DateTime<Utc>>,
    /// When position was last updated, by our own clock.
    pub pos_updated: Option<DateTime<Utc>>,
    /// When a receiver last said it has no fix (RMC or GLL void, GGA quality 0), by our own clock.
    pub no_fix_at: Option<DateTime<Utc>>,
    /// Satellites in view per constellation (GSV talker), fix or not.
    pub in_view: BTreeMap<String, u32>,
    /// When each of these last arrived, by our own clock: a speedo or a wind instrument that stops
    /// talking leaves its last value behind, and the performance log must not take it for a reading.
    pub sog_at: Option<DateTime<Utc>>,
    pub cog_at: Option<DateTime<Utc>>,
    pub heading_at: Option<DateTime<Utc>>,
    pub stw_at: Option<DateTime<Utc>>,
    pub wind_apparent_at: Option<DateTime<Utc>>,
    pub wind_true_at: Option<DateTime<Utc>>,
    /// What `wind_true` is relative to.
    pub wind_true_ref: Option<WindRef>,
    /// Each engine that reported, by its id.
    pub engines: BTreeMap<String, EngineReading>,
    /// Heel in degrees, positive to starboard, and when it came.
    pub heel: Option<(f64, DateTime<Utc>)>,
    /// What the autopilot said last, and when.
    pub autopilot: Option<Autopilot>,
    pub autopilot_at: Option<DateTime<Utc>>,
}

/// The autopilot's mode and what it steers to. Signal K sends the mode and the target apart, so either can
/// be missing for a moment.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Autopilot {
    pub mode: Option<PilotMode>,
    /// The heading or track to steer, and whether it is magnetic.
    pub heading: Option<(f64, bool)>,
    /// The apparent wind angle held in wind mode, -180..180, positive to starboard.
    pub wind_angle: Option<f64>,
}

/// What a true wind was worked out against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindRef {
    /// From the apparent wind and the speed through the water: the wind the sails feel, as a polar has it.
    Water,
    /// From the apparent wind and the speed over ground (no speedo): a current is in it.
    Ground,
    /// As an instrument gave it (MWV T, MWD, Signal K's true wind): whatever its processor did.
    Instrument,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EngineReading {
    pub running: bool,
    pub rpm: Option<f64>,
    pub at: DateTime<Utc>,
}

/// How long a speed through the water stays good for working out the true wind.
const STW_FRESH: Duration = Duration::seconds(10);

/// Wraps a wind angle to -180..180.
fn wrap180(a: f64) -> f64 {
    let r = normalize_bearing(a);
    if r > 180.0 { r - 360.0 } else { r }
}

/// True wind from apparent wind and boat speed through the water.
/// Returns the true wind angle off the bow (-180..180) and speed.
pub fn true_wind(app_angle: f64, app_speed: f64, stw: f64) -> (f64, f64) {
    let a = app_angle.to_radians();
    // Wind vector relative to the boat, in boat axes (x starboard, y forward),
    // pointing the way the wind blows.
    let (ax, ay) = (-app_speed * a.sin(), -app_speed * a.cos());
    // Add the boat's own motion.
    let (tx, ty) = (ax, ay + stw);
    let speed = (tx * tx + ty * ty).sqrt();
    // Angle the wind comes from.
    let from = (-tx).atan2(-ty).to_degrees();
    (wrap180(from), speed)
}

impl OwnShip {
    pub fn apply(&mut self, n: &Nmea, now: DateTime<Utc>) {
        match n {
            Nmea::Rmc { time, valid, pos, sog_kn, cog, variation } => {
                if *valid {
                    if let Some(p) = pos {
                        self.pos = Some(*p);
                        self.pos_updated = Some(now);
                    }
                    self.set_motion(*sog_kn, *cog, now);
                    self.fix_time = time.or(self.fix_time);
                } else {
                    self.no_fix_at = Some(now);
                }
                self.variation = variation.or(self.variation);
            }
            Nmea::Gga { pos, quality, satellites, hdop } => {
                if *quality > 0
                    && let Some(p) = pos
                {
                    self.pos = Some(*p);
                    self.pos_updated = Some(now);
                } else if *quality == 0 {
                    self.no_fix_at = Some(now);
                }
                self.satellites = satellites.or(self.satellites);
                self.fix_quality = Some(*quality);
                if let Some(h) = hdop {
                    self.hdop = Some(*h);
                    self.hdop_at = Some(now);
                }
            }
            Nmea::Hdop { hdop } => {
                self.hdop = Some(*hdop);
                self.hdop_at = Some(now);
            }
            Nmea::Accuracy { metres } => self.error_m = Some((*metres, now)),
            Nmea::Gll { pos, valid } => {
                if *valid && let Some(p) = pos {
                    self.pos = Some(*p);
                    self.pos_updated = Some(now);
                } else if !valid {
                    self.no_fix_at = Some(now);
                }
            }
            Nmea::Gsv { talker, in_view } => {
                self.in_view.insert(talker.clone(), *in_view);
            }
            Nmea::Vtg { cog_true, sog_kn } => self.set_motion(*sog_kn, *cog_true, now),
            Nmea::Hdt { heading } => self.set_heading(*heading, now),
            Nmea::Hdm { heading } => {
                // Only usable once the variation is known.
                if let Some(v) = self.variation {
                    self.set_heading(heading + v, now);
                }
            }
            Nmea::Hdg { heading, deviation, variation } => {
                if let Some(v) = variation {
                    self.variation = Some(*v);
                }
                if let (Some(h), Some(v)) = (heading, self.variation) {
                    self.set_heading(h + deviation.unwrap_or(0.0) + v, now);
                }
            }
            Nmea::Mwv { angle, true_wind: is_true, speed_kn } => {
                if *is_true {
                    if let Some(h) = self.heading {
                        self.set_true_wind(h + angle, *speed_kn, WindRef::Instrument, now);
                    }
                } else {
                    self.wind_apparent = Some((wrap180(*angle), *speed_kn));
                    self.wind_apparent_at = Some(now);
                    // Through the water when the speedo is talking, else over the ground.
                    let stw = self.stw_kn.filter(|_| self.stw_at.is_some_and(|t| now - t <= STW_FRESH));
                    let speed = stw.map(|s| (s, WindRef::Water)).or(self.sog_kn.map(|s| (s, WindRef::Ground)));
                    if let (Some((boat, r)), Some(h)) = (speed, self.heading) {
                        let (ang, spd) = true_wind(*angle, *speed_kn, boat);
                        self.set_true_wind(h + ang, spd, r, now);
                    }
                }
            }
            Nmea::Mwd { direction, speed_kn } => self.set_true_wind(*direction, *speed_kn, WindRef::Instrument, now),
            Nmea::Depth { metres } => self.depth_m = Some(*metres),
            Nmea::Vhw { stw_kn, heading } => {
                if let Some(s) = stw_kn {
                    self.stw_kn = Some(*s);
                    self.stw_at = Some(now);
                }
                if self.heading.is_none() && let (Some(h), Some(v)) = (heading, self.variation) {
                    self.set_heading(h + v, now);
                }
            }
            Nmea::WaterTemp { celsius } => self.water_temp_c = Some(*celsius),
            Nmea::Engine { id, running, rpm } => {
                self.engines.insert(id.clone(), EngineReading { running: *running, rpm: *rpm, at: now });
            }
            Nmea::Attitude { heel, .. } => {
                if let Some(h) = heel {
                    self.heel = Some((*h, now));
                }
            }
            Nmea::Pilot { mode, heading, magnetic, wind_angle } => {
                let p = self.autopilot.get_or_insert_with(Autopilot::default);
                if mode.is_some() {
                    p.mode = *mode;
                }
                if let Some(h) = heading {
                    p.heading = Some((normalize_bearing(*h), *magnetic));
                }
                if let Some(a) = wind_angle {
                    p.wind_angle = Some(wrap180(*a));
                }
                self.autopilot_at = Some(now);
            }
            Nmea::Rmb { .. } | Nmea::Ais(_) => {}
        }
    }

    fn set_motion(&mut self, sog_kn: Option<f64>, cog: Option<f64>, now: DateTime<Utc>) {
        if let Some(s) = sog_kn {
            self.sog_kn = Some(s);
            self.sog_at = Some(now);
        }
        if let Some(c) = cog {
            self.cog = Some(c);
            self.cog_at = Some(now);
        }
    }

    fn set_heading(&mut self, heading: f64, now: DateTime<Utc>) {
        self.heading = Some(normalize_bearing(heading));
        self.heading_at = Some(now);
    }

    fn set_true_wind(&mut self, from: f64, kn: f64, r: WindRef, now: DateTime<Utc>) {
        self.wind_true = Some((normalize_bearing(from), kn));
        self.wind_true_ref = Some(r);
        self.wind_true_at = Some(now);
    }

    /// Whether an engine is running, from the engines that reported within `fresh`: `Some(true)` if any
    /// is, `Some(false)` if all that reported are stopped, `None` when no engine data is coming.
    pub fn engine_running(&self, now: DateTime<Utc>, fresh: Duration) -> Option<bool> {
        let recent: Vec<_> = self.engines.values().filter(|e| now - e.at <= fresh).collect();
        (!recent.is_empty()).then(|| recent.iter().any(|e| e.running))
    }

    /// The position fix for navigation, if we have one.
    pub fn fix(&self, now: DateTime<Utc>) -> Option<Fix> {
        Some(Fix { pos: self.pos?, sog_kn: self.sog_kn, cog: self.cog, time: self.fix_time.unwrap_or(now) })
    }

    /// Satellites in view, all constellations, if a receiver says.
    pub fn satellites_in_view(&self) -> Option<u32> {
        (!self.in_view.is_empty()).then(|| self.in_view.values().sum())
    }

    /// How far off the position may be: the radius in metres it is within 95% of the time, if a receiver
    /// said within `timeout`. Its own estimate when it gives one, else twice the HDOP times the error of a
    /// satellite's range for a fix of its kind (a rule of thumb: no use for dead reckoning or a fix it calls void).
    pub fn error_radius_m(&self, now: DateTime<Utc>, timeout: Duration) -> Option<f64> {
        let fresh = |t: DateTime<Utc>| now - t <= timeout;
        if let Some((m, t)) = self.error_m
            && fresh(t)
        {
            return Some(m);
        }
        let range_error_m = match self.fix_quality {
            None | Some(1) | Some(3) => 3.0,
            Some(2) | Some(9) => 1.0,
            Some(5) => 0.3,
            Some(4) => 0.02,
            _ => return None,
        };
        let hdop = self.hdop.filter(|_| self.hdop_at.is_some_and(fresh)).filter(|h| *h > 0.0 && *h < 99.0)?;
        Some(2.0 * hdop * range_error_m)
    }

    /// A receiver is talking but has no fix: it said so within `timeout`, and no position came since.
    pub fn searching(&self, now: DateTime<Utc>, timeout: Duration) -> bool {
        self.no_fix_at.is_some_and(|t| now - t < timeout && self.pos_updated.is_none_or(|p| p < t))
    }

    /// No position update for `timeout`: the plotter should say so.
    pub fn position_is_stale(&self, now: DateTime<Utc>, timeout: Duration) -> bool {
        self.pos_updated.is_none_or(|t| now - t > timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comm::nmea::{frame, parse_line};
    use chrono::TimeZone;

    fn t(s: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_780_000_000 + s, 0).unwrap()
    }

    fn feed(o: &mut OwnShip, body: &str) {
        if let Some(n) = parse_line(&frame(body, '$')).unwrap() {
            o.apply(&n, t(0));
        }
    }

    #[test]
    fn the_autopilot_keeps_a_mode_and_a_target_that_come_apart() {
        let mut o = OwnShip::default();
        o.apply(&Nmea::Pilot { mode: Some(PilotMode::Wind), heading: None, magnetic: false, wind_angle: None }, t(0));
        o.apply(&Nmea::Pilot { mode: None, heading: None, magnetic: false, wind_angle: Some(320.0) }, t(1));
        let p = o.autopilot.unwrap();
        assert_eq!((p.mode, p.wind_angle), (Some(PilotMode::Wind), Some(-40.0)));
        feed(&mut o, "APHTD,V,,,S,,,,,,365.0,,,M,,,,128.5");
        let p = o.autopilot.unwrap();
        assert_eq!((p.mode, p.heading), (Some(PilotMode::Heading), Some((5.0, true))));
    }

    #[test]
    fn true_wind_cases() {
        // Stationary boat: true equals apparent.
        let (a, s) = true_wind(40.0, 10.0, 0.0);
        assert!((a - 40.0).abs() < 1e-9 && (s - 10.0).abs() < 1e-9);
        // Wind from dead ahead at 10 kn while making 5 kn: true wind is 5 kn from ahead.
        let (a, s) = true_wind(0.0, 10.0, 5.0);
        assert!(a.abs() < 1e-9 && (s - 5.0).abs() < 1e-9);
        // Beam reach (apparent 90 deg, 10 kn) at 5 kn: true wind is further aft.
        let (a, s) = true_wind(90.0, 10.0, 5.0);
        assert!(a > 90.0 && a < 130.0, "{a}");
        assert!((s - (125.0f64).sqrt()).abs() < 1e-9);
        // Port side is negative.
        let (a, _) = true_wind(-90.0, 10.0, 5.0);
        assert!(a < -90.0);
        // Apparent wind from astern at 2 kn on a boat doing 5 kn: the air moves forward
        // at 2 kn relative to the boat, so over the ground it moves at 7 kn from astern.
        let (a, s) = true_wind(180.0, 2.0, 5.0);
        assert!((a.abs() - 180.0).abs() < 1e-9 && (s - 7.0).abs() < 1e-9, "{a} {s}");
    }

    #[test]
    fn rmc_sets_position_but_only_when_valid() {
        let mut o = OwnShip::default();
        feed(&mut o, "GPRMC,123519,V,4807.038,N,01131.000,E,022.4,084.4,230394,003.1,W");
        assert!(o.pos.is_none());
        feed(&mut o, "GPRMC,123519,A,4807.038,N,01131.000,E,022.4,084.4,230394,003.1,W");
        assert!(o.pos.is_some());
        assert_eq!((o.sog_kn, o.cog, o.variation), (Some(22.4), Some(84.4), Some(-3.1)));
        assert!(o.fix_time.is_some());
    }

    #[test]
    fn gga_needs_a_fix_quality() {
        let mut o = OwnShip::default();
        feed(&mut o, "GPGGA,123519,4807.038,N,01131.000,E,0,00,,,M,,M,,");
        assert!(o.pos.is_none());
        feed(&mut o, "GPGGA,123519,4807.038,N,01131.000,E,1,07,1.1,,M,,M,,");
        assert!(o.pos.is_some());
        assert_eq!((o.satellites, o.hdop), (Some(7), Some(1.1)));
    }

    #[test]
    fn a_receiver_without_a_fix_is_searching_with_its_satellites() {
        // What a u-blox 7 indoors sends: void RMC, quality 0, satellites heard but none used.
        let mut o = OwnShip::default();
        assert!(!o.searching(t(0), Duration::seconds(30)) && o.satellites_in_view().is_none());
        feed(&mut o, "GPRMC,101844.00,V,,,,,,,311026,,,N");
        feed(&mut o, "GPGGA,101844.00,,,,,0,00,99.99,,,,,,");
        feed(&mut o, "GPGSV,2,1,08,03,,,22,07,,,23,18,,,23,22,,,27");
        feed(&mut o, "GLGSV,1,1,03,65,,,20,66,,,21,72,,,19");
        assert!(o.pos.is_none() && o.searching(t(1), Duration::seconds(30)));
        assert_eq!(o.satellites_in_view(), Some(11));
        assert!(!o.searching(t(40), Duration::seconds(30)));
        // A fix after that ends the search.
        o.apply(&parse_line(&frame("GPGGA,123519,4807.038,N,01131.000,E,1,07,1.1,,M,,M,,", '$')).unwrap().unwrap(), t(2));
        assert!(!o.searching(t(3), Duration::seconds(30)));
    }

    #[test]
    fn the_error_radius_is_the_receivers_own_else_from_hdop() {
        let timeout = Duration::seconds(30);
        let mut o = OwnShip::default();
        assert_eq!(o.error_radius_m(t(0), timeout), None);
        // A GPS fix at HDOP 1.5: 2 × 1.5 × 3 m.
        feed(&mut o, "GPGGA,123519,4807.038,N,01131.000,E,1,08,1.5,545.4,M,46.9,M,,");
        assert_eq!(o.error_radius_m(t(1), timeout), Some(9.0));
        // SBAS corrections make each range better.
        feed(&mut o, "GPGGA,123519,4807.038,N,01131.000,E,2,08,1.5,545.4,M,46.9,M,,");
        assert_eq!(o.error_radius_m(t(1), timeout), Some(3.0));
        // The receiver's own estimate wins while it keeps coming.
        feed(&mut o, "GPGST,172814.0,0.006,0.023,0.020,273.6,1.8,2.4,0.04");
        assert_eq!(o.error_radius_m(t(1), timeout), Some(6.0));
        assert_eq!(o.error_radius_m(t(40), timeout), None, "both too old");
        // Dead reckoning has no such error.
        feed(&mut o, "GPGGA,123519,4807.038,N,01131.000,E,6,00,1.5,545.4,M,46.9,M,,");
        o.error_m = None;
        assert_eq!(o.error_radius_m(t(1), timeout), None);
    }

    #[test]
    fn magnetic_heading_needs_variation() {
        let mut o = OwnShip::default();
        feed(&mut o, "HEHDM,100.0,M");
        assert!(o.heading.is_none());
        o.variation = Some(-3.0); // 3 degrees west
        feed(&mut o, "HEHDM,100.0,M");
        assert_eq!(o.heading, Some(97.0));
        feed(&mut o, "HEHDT,355.5,T");
        assert_eq!(o.heading, Some(355.5));
        // Wraps correctly.
        o.variation = Some(10.0);
        feed(&mut o, "HEHDM,355.0,M");
        assert!((o.heading.unwrap() - 5.0).abs() < 1e-9);
    }

    #[test]
    fn hdg_uses_deviation_and_variation() {
        let mut o = OwnShip::default();
        feed(&mut o, "HCHDG,98.3,0.6,E,0.6,W");
        // 98.3 + 0.6 (E dev) - 0.6 (W var)
        assert!((o.heading.unwrap() - 98.3).abs() < 1e-9);
    }

    #[test]
    fn apparent_wind_yields_true_wind_when_heading_is_known() {
        let mut o = OwnShip { heading: Some(90.0), sog_kn: Some(0.0), ..Default::default() };
        feed(&mut o, "IIMWV,045,R,10.0,N,A");
        assert_eq!(o.wind_apparent, Some((45.0, 10.0)));
        let (dir, spd) = o.wind_true.unwrap();
        assert!((dir - 135.0).abs() < 1e-6 && (spd - 10.0).abs() < 1e-6, "{dir} {spd}");
        // Port-side apparent wind is reported as a negative angle.
        feed(&mut o, "IIMWV,315,R,10.0,N,A");
        assert_eq!(o.wind_apparent.unwrap().0, -45.0);
    }

    #[test]
    fn stw_is_preferred_over_sog_for_true_wind() {
        let mut o = OwnShip { heading: Some(0.0), sog_kn: Some(0.0), stw_kn: Some(5.0), stw_at: Some(t(0)), ..Default::default() };
        feed(&mut o, "IIMWV,000,R,10.0,N,A");
        assert!((o.wind_true.unwrap().1 - 5.0).abs() < 1e-9);
        assert_eq!(o.wind_true_ref, Some(WindRef::Water));
        // A speedo that stopped talking is not used: the speed over ground is, and the wind is said to be over the ground.
        o.apply(&parse_line(&frame("IIMWV,000,R,10.0,N,A", '$')).unwrap().unwrap(), t(60));
        assert!((o.wind_true.unwrap().1 - 10.0).abs() < 1e-9);
        assert_eq!((o.wind_true_ref, o.wind_apparent_at, o.wind_true_at), (Some(WindRef::Ground), Some(t(60)), Some(t(60))));
        feed(&mut o, "IIMWV,045,T,12.0,N,A");
        assert_eq!(o.wind_true_ref, Some(WindRef::Instrument));
    }

    #[test]
    fn readings_carry_their_time() {
        let mut o = OwnShip::default();
        o.apply(&parse_line(&frame("GPVTG,054.7,T,034.4,M,005.5,N,010.2,K,A", '$')).unwrap().unwrap(), t(3));
        o.apply(&parse_line(&frame("IIVHW,,T,,M,6.1,N,,K", '$')).unwrap().unwrap(), t(4));
        o.apply(&parse_line(&frame("HEHDT,10.0,T", '$')).unwrap().unwrap(), t(5));
        assert_eq!((o.sog_at, o.cog_at, o.stw_at, o.heading_at), (Some(t(3)), Some(t(3)), Some(t(4)), Some(t(5))));
        assert_eq!(o.stw_kn, Some(6.1));
    }

    #[test]
    fn engines_say_whether_one_is_running() {
        let mut o = OwnShip::default();
        let fresh = Duration::seconds(30);
        assert_eq!(o.engine_running(t(0), fresh), None);
        o.apply(&parse_line(&frame("ERRPM,E,1,0,,A", '$')).unwrap().unwrap(), t(0));
        o.apply(&parse_line(&frame("ERRPM,E,2,1500,,A", '$')).unwrap().unwrap(), t(0));
        assert_eq!(o.engine_running(t(1), fresh), Some(true));
        o.apply(&parse_line(&frame("ERRPM,E,2,0,,A", '$')).unwrap().unwrap(), t(2));
        assert_eq!(o.engine_running(t(3), fresh), Some(false));
        // Gone quiet: no engine data.
        assert_eq!(o.engine_running(t(100), fresh), None);
        o.apply(&parse_line(&frame("IIXDR,A,-12.5,D,ROLL", '$')).unwrap().unwrap(), t(4));
        assert_eq!(o.heel, Some((-12.5, t(4))));
    }

    #[test]
    fn staleness() {
        let mut o = OwnShip::default();
        assert!(o.position_is_stale(t(0), Duration::seconds(10)));
        o.apply(&parse_line(&frame("GPGLL,4807.038,N,01131.000,E,123519,A", '$')).unwrap().unwrap(), t(0));
        assert!(!o.position_is_stale(t(5), Duration::seconds(10)));
        assert!(o.position_is_stale(t(30), Duration::seconds(10)));
    }

    #[test]
    fn fix_requires_a_position_and_falls_back_to_our_clock() {
        let mut o = OwnShip::default();
        assert!(o.fix(t(0)).is_none());
        o.pos = Some(LatLon::new(1.0, 2.0));
        o.sog_kn = Some(3.0);
        let f = o.fix(t(7)).unwrap();
        assert_eq!((f.time, f.sog_kn), (t(7), Some(3.0)));
    }

    #[test]
    fn depth_and_temperature() {
        let mut o = OwnShip::default();
        feed(&mut o, "SDDBT,,,4.2,M,,");
        feed(&mut o, "YXMTW,17.5,C");
        assert_eq!((o.depth_m, o.water_temp_c), (Some(4.2), Some(17.5)));
    }
}
