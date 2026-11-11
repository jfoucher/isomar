//! A made-up tidal race, for tests, benches and the demo: a stream along an axis, strongest on it and
//! fading across and along it, turning with a semi-diurnal period. No real data goes into it.

use crate::geo::LatLon;
use crate::routing::isochrone::Environment;
use chrono::{DateTime, Utc};

/// The mean lunar semi-diurnal period (M2), hours.
pub const M2_HOURS: f64 = 12.420_6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Race {
    pub centre: LatLon,
    /// The direction the flood sets towards, degrees true; the ebb sets the other way.
    pub axis_deg: f64,
    /// Across the axis, the distance at which the stream has fallen to 1/e of its peak, nautical miles.
    pub half_width_nm: f64,
    /// Along the axis, the same, nautical miles.
    pub half_length_nm: f64,
    pub peak_kn: f64,
    /// When the flood is strongest.
    pub flood_at: DateTime<Utc>,
}

impl Race {
    /// The stream at `p` and `t`, as (east, north) knots.
    pub fn at(&self, p: &LatLon, t: DateTime<Utc>) -> (f64, f64) {
        let c = self.centre;
        let dx = (p.lon - c.lon) * 60.0 * c.lat.to_radians().cos();
        let dy = (p.lat - c.lat) * 60.0;
        let (sa, ca) = self.axis_deg.to_radians().sin_cos();
        let along = dx * sa + dy * ca;
        let across = dx * ca - dy * sa;
        let shape = (-(across / self.half_width_nm).powi(2) - (along / self.half_length_nm).powi(2)).exp();
        let hours = (t - self.flood_at).num_milliseconds() as f64 / 3_600_000.0;
        let kn = self.peak_kn * shape * (std::f64::consts::TAU * hours / M2_HOURS).cos();
        (kn * sa, kn * ca)
    }
}

/// Another environment's wind and weather, with the current of a race added to whatever current it had.
pub struct WithRace<'a> {
    pub env: &'a dyn Environment,
    pub race: Race,
}

impl Environment for WithRace<'_> {
    fn wind(&self, p: &LatLon, t: DateTime<Utc>) -> Option<(f64, f64)> {
        self.env.wind(p, t)
    }

    fn has_wind(&self, p: &LatLon, t: DateTime<Utc>) -> bool {
        self.env.has_wind(p, t)
    }

    fn current(&self, p: &LatLon, t: DateTime<Utc>) -> Option<(f64, f64)> {
        let (e, n) = self.env.current(p, t).unwrap_or((0.0, 0.0));
        let (re, rn) = self.race.at(p, t);
        Some((e + re, n + rn))
    }

    fn has_currents(&self) -> bool {
        true
    }

    fn wave_height(&self, p: &LatLon, t: DateTime<Utc>) -> Option<f64> {
        self.env.wave_height(p, t)
    }

    fn gust(&self, p: &LatLon, t: DateTime<Utc>) -> Option<f64> {
        self.env.gust(p, t)
    }

    fn horizon(&self) -> Option<DateTime<Utc>> {
        self.env.horizon()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn race() -> Race {
        Race { centre: LatLon::new(48.0, -5.0), axis_deg: 0.0, half_width_nm: 1.0, half_length_nm: 4.0, peak_kn: 6.0, flood_at: Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap() }
    }

    #[test]
    fn the_stream_floods_along_the_axis_turns_and_fades_away_from_it() {
        let r = race();
        let (e, n) = r.at(&r.centre, r.flood_at);
        assert!(e.abs() < 1e-9 && (n - 6.0).abs() < 1e-9, "{e} {n}");
        // Half a period later, the ebb the other way; a quarter, slack.
        let half = r.flood_at + Duration::milliseconds((M2_HOURS / 2.0 * 3_600_000.0) as i64);
        assert!((r.at(&r.centre, half).1 + 6.0).abs() < 1e-3);
        let quarter = r.flood_at + Duration::milliseconds((M2_HOURS / 4.0 * 3_600_000.0) as i64);
        assert!(r.at(&r.centre, quarter).1.abs() < 1e-3);
        // A mile across, 1/e of it; five miles across, nothing to speak of.
        let across = r.centre.destination(90.0, 1.0);
        assert!((r.at(&across, r.flood_at).1 - 6.0 / std::f64::consts::E).abs() < 0.05);
        assert!(r.at(&r.centre.destination(90.0, 5.0), r.flood_at).1 < 0.01);
    }

    #[test]
    fn the_best_departure_takes_the_race_with_the_flood() {
        use crate::routing::isochrone::{ConstantWind, Settings};
        use crate::routing::land::NoLand;
        use crate::routing::polar::Polar;
        use crate::routing::window::{Weights, departures, scan};
        // 16 nm north through the middle of a 4 kn race flooding north, a beam wind: at about 6 kn the
        // boat is in the race some 1.3 hours after leaving, so the best departure is about that long
        // before the flood is strongest.
        let r = Race { peak_kn: 4.0, half_length_nm: 5.0, ..race() };
        let (from, to) = (r.centre.destination(180.0, 8.0), r.centre.destination(0.0, 8.0));
        let wind = ConstantWind { kn: 12.0, from: 90.0 };
        let env = WithRace { env: &wind, race: r };
        let first = r.flood_at - Duration::hours(6);
        let deps = departures(first, 12.0, 1.0, None);
        let s = Settings { step_minutes: 15.0, cell_nm: 0.5, ..Settings::default() };
        let found = scan(from, to, &deps, &Polar::generic_cruiser(), &env, &NoLand, &s, &Weights::fastest(), None).unwrap();
        let best = found.candidates[found.ranked()[0]].depart;
        let before = (r.flood_at - best).num_minutes() as f64 / 60.0;
        assert!((0.0..=3.0).contains(&before), "left {before:.1} h before the flood");
        // And the worst is near the ebb, half a period off.
        let worst = found.candidates[*found.ranked().last().unwrap()].depart;
        let off = ((worst - best).num_minutes() as f64 / 60.0).abs();
        assert!(off > 4.0, "worst {off:.1} h from the best");
    }

    #[test]
    fn refining_round_the_best_departures_finds_the_slack_between_them() {
        use crate::routing::isochrone::{ConstantWind, Settings};
        use crate::routing::land::NoLand;
        use crate::routing::polar::Polar;
        use crate::routing::window::{REFINE, Weights, around_best, departures, scan, scan_since};
        // As above, every three hours: the best of those is up to an hour and a half off the best time.
        let r = Race { peak_kn: 4.0, half_length_nm: 5.0, ..race() };
        let (from, to) = (r.centre.destination(180.0, 8.0), r.centre.destination(0.0, 8.0));
        let wind = ConstantWind { kn: 12.0, from: 90.0 };
        let env = WithRace { env: &wind, race: r };
        let first = r.flood_at - Duration::hours(6);
        let s = Settings { step_minutes: 15.0, cell_nm: 0.5, ..Settings::default() };
        let (polar, w) = (Polar::generic_cruiser(), Weights::fastest());
        let mut found = scan(from, to, &departures(first, 12.0, 3.0, None), &polar, &env, &NoLand, &s, &w, None).unwrap();
        let coarse = found.candidates[found.ranked()[0]].passage().unwrap().cost;
        for (top, hours) in REFINE {
            let deps: Vec<_> = found.candidates.iter().map(|c| c.depart).collect();
            let costs: Vec<_> = found.candidates.iter().map(|c| c.passage().map(|p| p.cost)).collect();
            let more = around_best(&deps, &costs, top, hours, first, None);
            assert!(!more.is_empty() && more.iter().all(|t| !deps.contains(t)));
            found = found.merge(scan_since(first, from, to, &more, &polar, &env, &NoLand, &s, &w, None).unwrap());
        }
        assert!(found.candidates.windows(2).all(|p| p[0].depart < p[1].depart), "in time order");
        let fine = found.candidates[found.ranked()[0]].passage().unwrap().cost;
        assert!(fine <= coarse, "{fine} against {coarse}");
        for (i, ex) in &found.kept {
            assert_eq!(ex.depart, found.candidates[*i].depart, "a kept search still goes with its departure");
        }
    }

    #[test]
    fn a_boat_can_wait_for_the_stream_but_not_at_the_start_nor_without_one() {
        use crate::routing::isochrone::{ConstantWind, HOLD_AFTER_S, Settings, explore};
        use crate::routing::land::NoLand;
        use crate::routing::polar::Polar;
        // A wide 5 kn stream flooding north, met against it: the boat makes over 6 kn, so it can stem it.
        let r = Race { peak_kn: 5.0, half_width_nm: 20.0, half_length_nm: 20.0, ..race() };
        let (from, to) = (r.centre.destination(0.0, 6.0), r.centre.destination(180.0, 6.0));
        let wind = ConstantWind { kn: 14.0, from: 90.0 };
        let env = WithRace { env: &wind, race: r };
        let s = Settings { step_minutes: 15.0, cell_nm: 0.5, allow_hold: true, ..Settings::default() };
        let ex = explore(from, to, r.flood_at, &Polar::generic_cruiser(), &env, &NoLand, &s, None).unwrap();
        let holds: Vec<_> = ex.tree.iter().filter(|t| t.parent.is_some() && t.node.holding()).collect();
        assert!(!holds.is_empty(), "points waited");
        assert!(holds.iter().all(|t| (t.node.time - r.flood_at).num_seconds() as f64 > HOLD_AFTER_S), "none in the first hour");
        let without = explore(from, to, r.flood_at, &Polar::generic_cruiser(), &env, &NoLand, &Settings { allow_hold: false, ..s.clone() }, None).unwrap();
        assert!(without.tree.iter().all(|t| t.parent.is_none() || !t.node.holding()));
        let calm = explore(from, to, r.flood_at, &Polar::generic_cruiser(), &wind, &NoLand, &s, None).unwrap();
        assert!(calm.tree.iter().all(|t| t.parent.is_none() || !t.node.holding()), "no stream, no waiting");
        // The arrival is no later for being allowed to wait.
        let (a, b) = (ex.arrived.as_ref().unwrap().arrival, without.arrived.as_ref().unwrap().arrival);
        assert!(a <= b + Duration::minutes(1), "{a} against {b}");
    }
}
