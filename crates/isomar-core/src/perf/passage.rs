//! A passage planned by the weather router against how it was sailed.

use super::sample::{SailMode, Sample};
use crate::routing::plan::RoutePlan;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Passage {
    pub route_id: i64,
    pub name: String,
    /// Seconds since 1970, when the route was started and finished.
    pub started: i64,
    pub finished: i64,
    pub planned_s: f64,
    pub sailed_s: f64,
    /// Planned distance over planned time; logged distance over the time sailed.
    pub planned_sog: f64,
    pub sailed_sog: f64,
    pub distance_nm: f64,
    /// Shares of the time under engine, as planned and as logged.
    pub planned_motoring: f64,
    pub motored: f64,
}

impl Passage {
    /// How the passage went against the plan: 1 as planned, 0.9 took 10% longer.
    pub fn ratio(&self) -> f64 {
        if self.sailed_s > 0.0 { self.planned_s / self.sailed_s } else { 0.0 }
    }
}

pub fn compare(route_id: i64, name: &str, plan: &RoutePlan, started: DateTime<Utc>, finished: DateTime<Utc>, samples: &[Sample]) -> Passage {
    let nodes = &plan.nodes;
    let planned_s = match (nodes.first(), nodes.last()) {
        (Some(a), Some(b)) => (b.time - a.time).num_seconds() as f64,
        _ => 0.0,
    };
    let (mut planned_nm, mut planned_motor_s) = (0.0, 0.0);
    for w in nodes.windows(2) {
        planned_nm += w[0].pos.distance_nm(&w[1].pos);
        if w[1].motoring {
            planned_motor_s += (w[1].time - w[0].time).num_seconds() as f64;
        }
    }
    let during: Vec<&Sample> = samples.iter().filter(|s| s.t >= started && s.t <= finished).collect();
    let logged_s: f64 = during.iter().map(|s| s.secs).sum();
    let distance_nm: f64 = during.iter().map(|s| s.sog * s.secs / 3600.0).sum();
    let motored_s: f64 = during.iter().filter(|s| s.mode == SailMode::Motoring).map(|s| s.secs).sum();
    let sailed_s = (finished - started).num_seconds() as f64;
    Passage {
        route_id,
        name: name.to_string(),
        started: started.timestamp(),
        finished: finished.timestamp(),
        planned_s,
        sailed_s,
        planned_sog: if planned_s > 0.0 { planned_nm / (planned_s / 3600.0) } else { 0.0 },
        sailed_sog: if sailed_s > 0.0 { distance_nm / (sailed_s / 3600.0) } else { 0.0 },
        distance_nm,
        planned_motoring: if planned_s > 0.0 { planned_motor_s / planned_s } else { 0.0 },
        motored: if logged_s > 0.0 { motored_s / logged_s } else { 0.0 },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::LatLon;
    use crate::routing::isochrone::Node;
    use crate::weather::store::WeatherStore;
    use chrono::{Duration, TimeZone};

    #[test]
    fn planned_against_sailed() {
        let t0 = Utc.with_ymd_and_hms(2026, 6, 1, 8, 0, 0).unwrap();
        let a = LatLon::new(47.0, -4.0);
        let b = a.destination(90.0, 30.0);
        let node = |pos: LatLon, h: i64, motoring: bool| Node { pos, time: t0 + Duration::hours(h), cog: 90.0, sog_kn: 6.0, heading: 90.0, tws_kn: 12.0, twd: 0.0, motoring, current: (0.0, 0.0) };
        let plan = RoutePlan::new(t0, b, true, vec![node(a, 0, false), node(a.destination(90.0, 15.0), 3, false), node(b, 5, true)], vec![a, b], &WeatherStore::new());
        // Sailed in 6 hours at 5 kn, the last hour under engine.
        let samples: Vec<Sample> = (0..2160)
            .map(|i| Sample { t: t0 + Duration::seconds(i * 10), sog: 5.0, secs: 10.0, mode: if i >= 1800 { SailMode::Motoring } else { SailMode::Sailing }, ..Default::default() })
            .collect();
        let p = compare(7, "Out", &plan, t0, t0 + Duration::hours(6), &samples);
        assert_eq!((p.planned_s, p.sailed_s), (18000.0, 21600.0));
        assert!((p.planned_sog - 6.0).abs() < 0.01 && (p.sailed_sog - 5.0).abs() < 0.01, "{p:?}");
        assert!((p.planned_motoring - 0.4).abs() < 1e-9 && (p.motored - 1.0 / 6.0).abs() < 1e-9, "{p:?}");
        assert!((p.ratio() - 5.0 / 6.0).abs() < 1e-9);
    }
}
