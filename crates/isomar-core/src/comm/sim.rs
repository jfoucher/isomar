//! A simulated boat with a few AIS targets. It produces the same sentences a
//! real instrument network would, so the whole pipeline can be exercised (and
//! demonstrated) without hardware.

use super::ais::{Dims, PositionReport, encode_position_a, encode_position_b, encode_static, to_sentences};
use super::nmea::{frame, make_rmc};
use crate::geo::{LatLon, normalize_bearing};
use chrono::{DateTime, Duration, Utc};

struct SimTarget {
    mmsi: u32,
    name: &'static str,
    callsign: &'static str,
    ship_type: u8,
    pos: LatLon,
    sog: f64,
    cog: f64,
    class_b: bool,
}

pub struct Simulator {
    pub pos: LatLon,
    pub sog_kn: f64,
    pub cog: f64,
    pub time: DateTime<Utc>,
    /// Direction the true wind blows from, and its speed.
    pub wind_from: f64,
    pub wind_kn: f64,
    targets: Vec<SimTarget>,
    ticks: u64,
}

/// Apparent wind (angle off the bow, speed) felt on a boat heading `heading`
/// at `stw`, given the true wind direction and speed.
pub fn apparent_wind(wind_from: f64, wind_kn: f64, heading: f64, stw: f64) -> (f64, f64) {
    // True wind relative to the bow, then subtract the boat's motion.
    let rel = (wind_from - heading).to_radians();
    let (wx, wy) = (-wind_kn * rel.sin(), -wind_kn * rel.cos()); // blowing towards
    let (ax, ay) = (wx, wy - stw);
    let speed = (ax * ax + ay * ay).sqrt();
    let from = (-ax).atan2(-ay).to_degrees();
    (normalize_bearing(from), speed)
}

impl Simulator {
    pub fn new(start: LatLon, cog: f64, sog_kn: f64, now: DateTime<Utc>) -> Simulator {
        Simulator { pos: start, sog_kn, cog, time: now, wind_from: 250.0, wind_kn: 14.0, targets: Vec::new(), ticks: 0 }
    }

    /// A boat with a ferry on a crossing course (it reaches our track about 12
    /// minutes ahead, passing roughly 0.15 nm in front, so the collision alert
    /// can be seen), a sailboat overtaking, and a moored fishing boat.
    pub fn demo(start: LatLon, cog: f64, sog_kn: f64, now: DateTime<Utc>) -> Simulator {
        let mut s = Simulator::new(start, cog, sog_kn, now);
        // Where we will be in 12 minutes, nudged ahead so the ferry passes in front.
        let meet = start.destination(cog, sog_kn * 0.2 + 0.15);
        let ferry_start = meet.destination(cog + 90.0, 18.0 * 0.2);
        s.targets = vec![
            SimTarget { mmsi: 227_100_001, name: "ARMORIQUE", callsign: "FNAC", ship_type: 60, pos: ferry_start, sog: 18.0, cog: normalize_bearing(cog + 270.0), class_b: false },
            SimTarget { mmsi: 227_200_002, name: "MOUETTE", callsign: "FX2002", ship_type: 36, pos: start.destination(cog + 180.0, 0.8).destination(cog + 90.0, 0.3), sog: sog_kn + 1.5, cog, class_b: true },
            SimTarget { mmsi: 227_300_003, name: "BIGOUDEN", callsign: "FX3003", ship_type: 30, pos: start.destination(cog + 60.0, 1.5), sog: 0.0, cog: 0.0, class_b: false },
        ];
        s
    }

    /// The same three boats, all passing well clear, so no alert is raised: what `ISOMAR_DEMO` shows
    /// unless `ISOMAR_DEMO_COLLISION` asks for the ferry's crossing. Screenshots show the alert only
    /// when that is their subject. The ferry meets us in 12 minutes on the reciprocal course 1.5 nm to
    /// port; the sailboat overtakes 1 nm to starboard.
    pub fn demo_clear(start: LatLon, cog: f64, sog_kn: f64, now: DateTime<Utc>) -> Simulator {
        let mut s = Simulator::demo(start, cog, sog_kn, now);
        let ferry = &mut s.targets[0];
        ferry.pos = start.destination(cog, (sog_kn + ferry.sog) * 0.2).destination(cog - 90.0, 1.5);
        ferry.cog = normalize_bearing(cog + 180.0);
        s.targets[1].pos = start.destination(cog + 180.0, 0.8).destination(cog + 90.0, 1.0);
        s
    }

    /// Where each target is, by name: for checking that a demo's traffic is all at sea.
    pub fn targets(&self) -> impl Iterator<Item = (&'static str, LatLon)> + '_ {
        self.targets.iter().map(|t| (t.name, t.pos))
    }

    /// Heads for `dest` at the current speed.
    pub fn steer_to(&mut self, dest: &LatLon) {
        self.cog = self.pos.bearing_to(dest);
    }

    /// Advances `dt_s` seconds and returns the sentences emitted.
    pub fn step(&mut self, dt_s: f64) -> Vec<String> {
        self.time += Duration::milliseconds((dt_s * 1000.0) as i64);
        self.pos = self.pos.destination(self.cog, self.sog_kn * dt_s / 3600.0);
        for t in &mut self.targets {
            t.pos = t.pos.destination(t.cog, t.sog * dt_s / 3600.0);
        }
        self.ticks += 1;

        let mut out = vec![
            make_rmc(self.time, &self.pos, self.sog_kn, self.cog),
            frame(&format!("HEHDT,{:.1},T", self.cog), '$'),
            frame(&format!("GPVTG,{:.1},T,,M,{:.1},N,{:.1},K,A", self.cog, self.sog_kn, self.sog_kn * 1.852), '$'),
        ];
        // A speedo that agrees with the GPS (no current), so the performance log has a full set.
        out.push(frame(&format!("VWVHW,,T,,M,{:.1},N,,K", self.sog_kn), '$'));
        let (ang, spd) = apparent_wind(self.wind_from, self.wind_kn, self.cog, self.sog_kn);
        out.push(frame(&format!("IIMWV,{ang:.0},R,{spd:.1},N,A"), '$'));
        // An autopilot holding the course, as a boat with a full set of instruments has.
        out.push(frame(&format!("APHTD,V,,,S,,,,,,{:.1},,,T,,,,{:.1}", self.cog, self.cog), '$'));
        // A gently varying depth, so the depth instrument has something to show.
        let depth = 12.0 + 4.0 * ((self.ticks as f64) / 40.0).sin();
        out.push(frame(&format!("SDDPT,{depth:.1},0.5"), '$'));
        // The receiver's error estimate, wandering as the satellites move.
        let sd = 3.0 + 1.5 * ((self.ticks as f64) / 60.0).sin();
        out.push(frame(&format!("GPGST,{},,,,,{sd:.1},{:.1},", self.time.format("%H%M%S"), sd * 1.2), '$'));

        for (i, t) in self.targets.iter().enumerate() {
            let r = PositionReport { mmsi: t.mmsi, pos: t.pos, sog_kn: t.sog, cog: t.cog, heading: Some(t.cog.round() as u16 % 360), status: if t.sog < 0.5 { 5 } else { 0 } };
            let (p, fill) = if t.class_b { encode_position_b(&r) } else { encode_position_a(&r) };
            out.extend(to_sentences(&p, fill, 'A', 0));
            // Static data every 30 ticks, staggered.
            if (self.ticks + i as u64 * 7) % 30 == 1 {
                let (p, fill) = encode_static(t.mmsi, t.name, t.callsign, t.ship_type, Dims { bow: 20, stern: 15, port: 4, starboard: 4 }, "BREST");
                out.extend(to_sentences(&p, fill, 'B', (self.ticks % 9) as u8));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comm::ais::{AisBody, Assembler};
    use crate::comm::state::true_wind;
    use crate::comm::cpa::AisTargets;
    use crate::comm::nmea::{Nmea, decode, parse_sentence};
    use crate::comm::state::OwnShip;
    use chrono::TimeZone;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap()
    }

    #[test]
    fn apparent_wind_inverts_true_wind() {
        for (from, kn, hdg, stw) in [(250.0, 14.0, 90.0, 6.0), (10.0, 5.0, 350.0, 8.0), (180.0, 20.0, 0.0, 0.0)] {
            let (aa, asp) = apparent_wind(from, kn, hdg, stw);
            let aa180 = if aa > 180.0 { aa - 360.0 } else { aa };
            let (ta, ts) = true_wind(aa180, asp, stw);
            let true_dir = normalize_bearing(hdg + ta);
            assert!((true_dir - from).abs() < 1e-6 || (true_dir - from).abs() > 359.999, "{true_dir} vs {from}");
            assert!((ts - kn).abs() < 1e-6);
        }
    }

    #[test]
    fn the_boat_moves_at_the_right_speed() {
        let start = LatLon::new(48.0, -4.0);
        let mut s = Simulator::new(start, 0.0, 6.0, t0());
        for _ in 0..600 {
            s.step(6.0); // an hour in 6 s steps
        }
        assert!((start.distance_nm(&s.pos) - 6.0).abs() < 0.01);
        assert!((s.time - t0()).num_minutes() == 60);
    }

    #[test]
    fn every_emitted_sentence_parses_and_feeds_the_whole_pipeline() {
        let mut s = Simulator::demo(LatLon::new(48.35, -4.55), 90.0, 6.0, t0());
        let mut own = OwnShip { stw_kn: Some(6.0), ..OwnShip::default() };
        let mut targets = AisTargets::default();
        let mut asm = Assembler::default();
        let mut lines = 0;
        for tick in 0..40 {
            for l in s.step(1.0) {
                lines += 1;
                let sent = parse_sentence(&l).unwrap_or_else(|e| panic!("{l}: {e}"));
                match decode(&sent) {
                    Some(Nmea::Ais(a)) => {
                        if let Some(m) = asm.push(&a) {
                            targets.update(m, t0() + Duration::seconds(tick));
                        }
                    }
                    Some(n) => own.apply(&n, t0()),
                    None => {}
                }
            }
        }
        assert!(lines > 300);
        let p = own.pos.unwrap();
        assert!(p.lon > -4.55, "moved east");
        assert_eq!(own.cog, Some(90.0));
        assert!(own.depth_m.unwrap() > 8.0 && own.depth_m.unwrap() < 17.0);
        assert!(own.wind_apparent.is_some() && own.wind_true.is_some());
        let pilot = own.autopilot.unwrap();
        assert_eq!((pilot.mode, pilot.heading), (Some(crate::comm::nmea::PilotMode::Heading), Some((90.0, false))));
        assert_eq!(targets.len(), 3);
        let named = targets.iter().filter(|t| t.name.is_some()).count();
        assert!(named >= 2, "static data arrives over time ({named})");
        let ferry = targets.get(227_100_001).unwrap();
        assert!(ferry.sog_kn.unwrap() > 17.0);
    }

    #[test]
    fn the_moored_boat_stays_put() {
        let mut s = Simulator::demo(LatLon::new(48.0, -4.0), 0.0, 5.0, t0());
        let mut asm = Assembler::default();
        let mut seen = Vec::new();
        for _ in 0..3 {
            for l in s.step(1.0) {
                let sent = parse_sentence(&l).unwrap();
                if let Some(Nmea::Ais(a)) = decode(&sent)
                    && let Some(m) = asm.push(&a)
                    && m.mmsi == 227_300_003
                    && let AisBody::Position { pos: Some(p), .. } = m.body
                {
                    seen.push(p);
                }
            }
        }
        assert_eq!(seen.len(), 3);
        assert!(seen[0].distance_nm(&seen[2]) < 0.001);
    }

    #[test]
    fn steer_to_points_at_the_destination() {
        let mut s = Simulator::new(LatLon::new(48.0, -4.0), 0.0, 5.0, t0());
        s.steer_to(&LatLon::new(48.0, -3.0));
        assert!((s.cog - 90.0).abs() < 0.5);
    }
}
