//! AIS target tracking and collision alerts (CPA/TCPA).

use super::ais::{AisBody, AisClass, AisMessage, Dims};
use crate::geo::LatLon;
use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct AisTarget {
    pub mmsi: u32,
    pub class: AisClass,
    pub name: Option<String>,
    pub callsign: Option<String>,
    pub ship_type: Option<u8>,
    pub dims: Option<Dims>,
    pub destination: Option<String>,
    pub pos: Option<LatLon>,
    pub sog_kn: Option<f64>,
    pub cog: Option<f64>,
    pub heading: Option<u16>,
    pub status: Option<u8>,
    pub last_seen: DateTime<Utc>,
    /// Recent positions, oldest first.
    pub trail: Vec<LatLon>,
}

const MAX_TRAIL: usize = 60;

#[derive(Default)]
pub struct AisTargets {
    map: HashMap<u32, AisTarget>,
}

impl AisTargets {
    pub fn update(&mut self, m: AisMessage, now: DateTime<Utc>) {
        let class_hint = match &m.body {
            AisBody::Position { class, .. } => *class,
            AisBody::Aton { .. } => AisClass::Aton,
            AisBody::Static { .. } => AisClass::A,
        };
        let t = self.map.entry(m.mmsi).or_insert_with(|| AisTarget {
            mmsi: m.mmsi,
            class: class_hint,
            name: None,
            callsign: None,
            ship_type: None,
            dims: None,
            destination: None,
            pos: None,
            sog_kn: None,
            cog: None,
            heading: None,
            status: None,
            last_seen: now,
            trail: Vec::new(),
        });
        t.last_seen = now;
        match m.body {
            AisBody::Position { class, status, sog_kn, cog, heading, pos } => {
                t.class = class;
                t.status = status.or(t.status);
                t.sog_kn = sog_kn;
                t.cog = cog;
                t.heading = heading;
                if let Some(p) = pos {
                    // Only keep trail points that moved.
                    if t.trail.last().is_none_or(|l| l.distance_nm(&p) > 0.01) {
                        t.trail.push(p);
                        if t.trail.len() > MAX_TRAIL {
                            t.trail.remove(0);
                        }
                    }
                    t.pos = Some(p);
                }
            }
            AisBody::Static { name, callsign, ship_type, dims, destination, .. } => {
                t.name = name.or(t.name.take());
                t.callsign = callsign.or(t.callsign.take());
                t.ship_type = ship_type.or(t.ship_type);
                t.dims = dims.or(t.dims);
                t.destination = destination.or(t.destination.take());
            }
            AisBody::Aton { name, pos, .. } => {
                t.class = AisClass::Aton;
                t.name = name.or(t.name.take());
                t.pos = pos.or(t.pos);
            }
        }
    }

    /// Drops targets not heard from for `timeout`; returns their MMSIs.
    pub fn expire(&mut self, now: DateTime<Utc>, timeout: Duration) -> Vec<u32> {
        let gone: Vec<u32> = self.map.values().filter(|t| now - t.last_seen > timeout).map(|t| t.mmsi).collect();
        for m in &gone {
            self.map.remove(m);
        }
        gone
    }

    pub fn get(&self, mmsi: u32) -> Option<&AisTarget> {
        self.map.get(&mmsi)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &AisTarget> {
        self.map.values()
    }

    /// Targets with a position, nearest to `from` first.
    pub fn by_distance(&self, from: &LatLon) -> Vec<(&AisTarget, f64)> {
        let mut v: Vec<_> = self.map.values().filter_map(|t| t.pos.map(|p| (t, from.distance_nm(&p)))).collect();
        v.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        v
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Motion {
    pub pos: LatLon,
    pub sog_kn: f64,
    pub cog: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cpa {
    pub cpa_nm: f64,
    /// Minutes until closest approach; negative when the target is moving away.
    pub tcpa_min: f64,
    pub range_nm: f64,
    pub bearing: f64,
}

/// Closest point of approach between two vessels moving in straight lines.
pub fn cpa(own: &Motion, tgt: &Motion) -> Cpa {
    let k = own.pos.lat.to_radians().cos();
    // Target relative to own ship, in nm on a local flat plane.
    let (rx, ry) = ((tgt.pos.lon - own.pos.lon) * k * 60.0, (tgt.pos.lat - own.pos.lat) * 60.0);
    let vel = |m: &Motion| (m.sog_kn * m.cog.to_radians().sin(), m.sog_kn * m.cog.to_radians().cos());
    let ((ox, oy), (tx, ty)) = (vel(own), vel(tgt));
    let (vx, vy) = (tx - ox, ty - oy);
    let v2 = vx * vx + vy * vy;
    let range = (rx * rx + ry * ry).sqrt();
    let bearing = own.pos.bearing_to(&tgt.pos);
    if v2 < 1e-9 {
        // No relative motion: the distance never changes.
        return Cpa { cpa_nm: range, tcpa_min: 0.0, range_nm: range, bearing };
    }
    let t = -(rx * vx + ry * vy) / v2; // hours
    let (cx, cy) = (rx + vx * t, ry + vy * t);
    Cpa { cpa_nm: (cx * cx + cy * cy).sqrt(), tcpa_min: t * 60.0, range_nm: range, bearing }
}

#[derive(Clone, Copy, Debug)]
pub struct AlarmConfig {
    pub cpa_nm: f64,
    pub tcpa_min: f64,
    /// How long a target must stay safe before its alert may fire again.
    pub hold: Duration,
    /// Targets slower than this are treated as moored and ignored.
    pub min_sog_kn: f64,
    /// Shallow-water alarm: under this depth in metres; 0 is off.
    pub shallow_m: f64,
    /// Off-course warning: across-track error beyond this many nautical miles; 0 is off.
    pub xte_limit_nm: f64,
    /// Wind warning: true wind above this many knots; 0 is off.
    pub wind_limit_kn: f64,
}

impl Default for AlarmConfig {
    fn default() -> Self {
        AlarmConfig { cpa_nm: 0.5, tcpa_min: 20.0, hold: Duration::seconds(60), min_sog_kn: 0.3, shallow_m: 0.0, xte_limit_nm: 0.1, wind_limit_kn: 0.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AlarmEvent {
    /// A target became dangerous and the user has not seen it yet.
    New { mmsi: u32, cpa: Cpa },
    /// A target has been safe for the hold time.
    Cleared { mmsi: u32 },
}

struct Tracked {
    acked: bool,
    last_danger: DateTime<Utc>,
}

#[derive(Default)]
pub struct AisAlarms {
    active: HashMap<u32, Tracked>,
}

impl AisAlarms {
    /// Evaluates every target against the alarm limits. An alert fires once per
    /// encounter: acknowledging it silences the target until it has been safe
    /// for the hold time, so a boat hovering at the limit does not retrigger.
    pub fn evaluate(&mut self, own: &Motion, targets: &AisTargets, cfg: &AlarmConfig, now: DateTime<Utc>) -> Vec<AlarmEvent> {
        let mut events = Vec::new();
        for t in targets.iter() {
            if matches!(t.class, AisClass::Aton | AisClass::Base) {
                continue;
            }
            let (Some(pos), Some(sog), Some(cog)) = (t.pos, t.sog_kn, t.cog) else { continue };
            if sog < cfg.min_sog_kn {
                continue;
            }
            let c = cpa(own, &Motion { pos, sog_kn: sog, cog });
            let dangerous = c.tcpa_min >= 0.0 && c.tcpa_min <= cfg.tcpa_min && c.cpa_nm <= cfg.cpa_nm;
            if dangerous {
                match self.active.get_mut(&t.mmsi) {
                    Some(tr) => tr.last_danger = now,
                    None => {
                        self.active.insert(t.mmsi, Tracked { acked: false, last_danger: now });
                        events.push(AlarmEvent::New { mmsi: t.mmsi, cpa: c });
                    }
                }
            }
        }
        // Targets that vanished from the table are safe by definition.
        let alive: Vec<u32> = targets.iter().map(|t| t.mmsi).collect();
        let cleared: Vec<u32> = self
            .active
            .iter()
            .filter(|(m, tr)| now - tr.last_danger >= cfg.hold || !alive.contains(m))
            .map(|(m, _)| *m)
            .collect();
        for m in cleared {
            self.active.remove(&m);
            events.push(AlarmEvent::Cleared { mmsi: m });
        }
        events
    }

    pub fn acknowledge(&mut self, mmsi: u32) {
        if let Some(t) = self.active.get_mut(&mmsi) {
            t.acked = true;
        }
    }

    /// MMSIs currently dangerous, with whether the user has acknowledged them.
    pub fn active(&self) -> Vec<(u32, bool)> {
        let mut v: Vec<_> = self.active.iter().map(|(m, t)| (*m, t.acked)).collect();
        v.sort();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap()
    }

    fn motion(lat: f64, lon: f64, sog: f64, cog: f64) -> Motion {
        Motion { pos: LatLon::new(lat, lon), sog_kn: sog, cog }
    }

    #[test]
    fn head_on_collision_course() {
        // Own ship heading north at 6 kn, target 6 nm ahead heading south at 6 kn.
        let own = motion(48.0, -4.0, 6.0, 0.0);
        let tgt = motion(48.1, -4.0, 6.0, 180.0);
        let c = cpa(&own, &tgt);
        assert!(c.cpa_nm < 0.01, "{c:?}");
        assert!((c.tcpa_min - 30.0).abs() < 0.5, "{c:?}");
        assert!((c.range_nm - 6.0).abs() < 0.05);
        assert!(c.bearing < 0.5 || c.bearing > 359.5);
    }

    #[test]
    fn crossing_target_passes_ahead_with_a_known_miss_distance() {
        // Own north at 10 kn. Target 5 nm east and 10 nm ahead, moving west at 10 kn.
        // Relative to us it moves (-10, -10) kn from (5, 10) nm, so it is closest after
        // 0.75 h at (-2.5, 2.5) nm, a miss of 2.5 * sqrt(2).
        let own = motion(0.0, 0.0, 10.0, 0.0);
        let tgt = motion(10.0 / 60.0, 5.0 / 60.0, 10.0, 270.0);
        let c = cpa(&own, &tgt);
        assert!((c.cpa_nm - 2.5 * 2f64.sqrt()).abs() < 0.05, "{c:?}");
        assert!((c.tcpa_min - 45.0).abs() < 0.5, "{c:?}");
    }

    #[test]
    fn diverging_targets_have_negative_tcpa() {
        let own = motion(48.0, -4.0, 5.0, 0.0);
        let tgt = motion(48.1, -4.0, 8.0, 0.0);
        assert!(cpa(&own, &tgt).tcpa_min < 0.0);
    }

    #[test]
    fn parallel_courses_keep_their_distance() {
        let own = motion(48.0, -4.0, 5.0, 90.0);
        let tgt = motion(48.05, -4.0, 5.0, 90.0);
        let c = cpa(&own, &tgt);
        assert!((c.cpa_nm - 3.0).abs() < 0.05 && c.tcpa_min == 0.0);
    }

    fn msg(mmsi: u32, lat: f64, lon: f64, sog: f64, cog: f64) -> AisMessage {
        AisMessage {
            msg_type: 1,
            mmsi,
            body: AisBody::Position { class: AisClass::A, status: Some(0), sog_kn: Some(sog), cog: Some(cog), heading: None, pos: Some(LatLon::new(lat, lon)) },
        }
    }

    #[test]
    fn targets_merge_position_and_static_data_and_expire() {
        let mut t = AisTargets::default();
        t.update(msg(1, 48.0, -4.0, 5.0, 90.0), t0());
        t.update(
            AisMessage { msg_type: 5, mmsi: 1, body: AisBody::Static { name: Some("BOAT".into()), callsign: None, ship_type: Some(36), dims: None, destination: Some("BREST".into()), draught_m: None, imo: None } },
            t0(),
        );
        // A later position report must not erase the name.
        t.update(msg(1, 48.0, -3.99, 5.0, 90.0), t0() + Duration::seconds(10));
        let a = t.get(1).unwrap();
        assert_eq!((a.name.as_deref(), a.ship_type, a.destination.as_deref()), (Some("BOAT"), Some(36), Some("BREST")));
        assert_eq!(a.trail.len(), 2);
        assert_eq!(t.len(), 1);
        assert!(t.expire(t0() + Duration::minutes(5), Duration::minutes(10)).is_empty());
        assert_eq!(t.expire(t0() + Duration::minutes(20), Duration::minutes(10)), vec![1]);
        assert!(t.is_empty());
    }

    #[test]
    fn trail_ignores_stationary_repeats_and_is_capped() {
        let mut t = AisTargets::default();
        for _ in 0..5 {
            t.update(msg(1, 48.0, -4.0, 0.0, 0.0), t0());
        }
        assert_eq!(t.get(1).unwrap().trail.len(), 1);
        for i in 0..200 {
            t.update(msg(1, 48.0 + i as f64 * 0.001, -4.0, 5.0, 0.0), t0());
        }
        assert_eq!(t.get(1).unwrap().trail.len(), MAX_TRAIL);
    }

    #[test]
    fn nearest_first() {
        let mut t = AisTargets::default();
        t.update(msg(1, 48.5, -4.0, 5.0, 0.0), t0());
        t.update(msg(2, 48.1, -4.0, 5.0, 0.0), t0());
        let v = t.by_distance(&LatLon::new(48.0, -4.0));
        assert_eq!((v[0].0.mmsi, v[1].0.mmsi), (2, 1));
    }

    #[test]
    fn alarm_fires_once_per_encounter_and_does_not_retrigger() {
        let own = motion(48.0, -4.0, 6.0, 0.0);
        let cfg = AlarmConfig::default();
        let mut targets = AisTargets::default();
        let mut alarms = AisAlarms::default();
        // Head-on, 3.9 nm ahead: closing at 12 kn, tcpa just under the 20 min limit.
        targets.update(msg(7, 48.0 + 3.9 / 60.0, -4.0, 6.0, 180.0), t0());
        let ev = alarms.evaluate(&own, &targets, &cfg, t0());
        assert!(matches!(ev.as_slice(), [AlarmEvent::New { mmsi: 7, .. }]));
        // The user acknowledges; further evaluations stay quiet.
        alarms.acknowledge(7);
        assert_eq!(alarms.active(), vec![(7, true)]);
        for s in 1..5 {
            assert!(alarms.evaluate(&own, &targets, &cfg, t0() + Duration::seconds(s * 10)).is_empty());
        }
        // The target turns away; it must stay safe for the hold time before clearing.
        targets.update(msg(7, 48.0 + 3.0 / 60.0, -4.0, 6.0, 0.0), t0() + Duration::seconds(50));
        assert!(alarms.evaluate(&own, &targets, &cfg, t0() + Duration::seconds(60)).is_empty());
        let ev = alarms.evaluate(&own, &targets, &cfg, t0() + Duration::seconds(130));
        assert_eq!(ev, vec![AlarmEvent::Cleared { mmsi: 7 }]);
        // A new encounter alerts again.
        targets.update(msg(7, 48.0 + 3.9 / 60.0, -4.0, 6.0, 180.0), t0() + Duration::seconds(200));
        let ev = alarms.evaluate(&own, &targets, &cfg, t0() + Duration::seconds(200));
        assert!(matches!(ev.as_slice(), [AlarmEvent::New { mmsi: 7, .. }]));
    }

    #[test]
    fn moored_boats_atons_and_distant_targets_do_not_alarm() {
        let own = motion(48.0, -4.0, 6.0, 0.0);
        let mut targets = AisTargets::default();
        targets.update(msg(1, 48.01, -4.0, 0.1, 0.0), t0());
        targets.update(msg(2, 48.5, -4.0, 6.0, 180.0), t0());
        targets.update(AisMessage { msg_type: 21, mmsi: 3, body: AisBody::Aton { name: None, aton_type: 1, pos: Some(LatLon::new(48.01, -4.0)) } }, t0());
        let ev = AisAlarms::default().evaluate(&own, &targets, &AlarmConfig::default(), t0());
        assert!(ev.is_empty(), "{ev:?}");
    }

    #[test]
    fn vanished_targets_clear_their_alert() {
        let own = motion(48.0, -4.0, 6.0, 0.0);
        let mut targets = AisTargets::default();
        let mut alarms = AisAlarms::default();
        targets.update(msg(7, 48.0 + 3.9 / 60.0, -4.0, 6.0, 180.0), t0());
        alarms.evaluate(&own, &targets, &AlarmConfig::default(), t0());
        targets.expire(t0() + Duration::hours(1), Duration::minutes(10));
        let ev = alarms.evaluate(&own, &targets, &AlarmConfig::default(), t0() + Duration::hours(1));
        assert_eq!(ev, vec![AlarmEvent::Cleared { mmsi: 7 }]);
    }
}
