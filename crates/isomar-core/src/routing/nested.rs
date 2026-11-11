//! A regional forecast model inside its area and before its forecast ends, a global one around it and
//! after: what the router sails on when a regional model is chosen. Where one gives way to the other the
//! two are blended, over the last `BLEND_DEG` of the regional model's area and its last `BLEND_HOURS`,
//! so the router meets no wall of different wind at the edge.

use super::isochrone::{Environment, StoreEnv};
use crate::geo::{LatLon, wind_from_uv, wind_uv};
use crate::vector::Bounds;
use crate::weather::store::WeatherStore;
use chrono::{DateTime, Utc};

/// How far inside its edge the regional model takes over wholly, degrees.
pub const BLEND_DEG: f64 = 1.0;
/// How long before its end the regional model starts to give way, hours.
pub const BLEND_HOURS: f64 = 3.0;

pub struct NestedEnv<'a> {
    inner: StoreEnv<'a>,
    outer: StoreEnv<'a>,
    bounds: Option<Bounds>,
    end: Option<DateTime<Utc>>,
}

impl<'a> NestedEnv<'a> {
    pub fn new(inner: &'a WeatherStore, outer: &'a WeatherStore) -> NestedEnv<'a> {
        NestedEnv { bounds: inner.bounds(), end: inner.wind_range().map(|r| r.1), inner: StoreEnv(inner), outer: StoreEnv(outer) }
    }

    /// The share of the regional model at `p` and `t`: 1 well inside it, 0 outside it or after it.
    pub fn weight(&self, p: &LatLon, t: DateTime<Utc>) -> f64 {
        let (Some(b), Some(end)) = (self.bounds, self.end) else { return 0.0 };
        if !b.contains([p.lon, p.lat]) || t > end {
            return 0.0;
        }
        let k = p.lat.to_radians().cos().max(0.1);
        let edge = (p.lat - b.min_lat).min(b.max_lat - p.lat).min((p.lon - b.min_lon) * k).min((b.max_lon - p.lon) * k);
        let space = (edge / BLEND_DEG).clamp(0.0, 1.0);
        let time = ((end - t).num_seconds() as f64 / 3600.0 / BLEND_HOURS).clamp(0.0, 1.0);
        space * time
    }

    fn blend(&self, p: &LatLon, t: DateTime<Utc>, inner: Option<f64>, outer: Option<f64>) -> Option<f64> {
        match (inner, outer) {
            (Some(i), Some(o)) => {
                let w = self.weight(p, t);
                Some(i * w + o * (1.0 - w))
            }
            (Some(i), None) => (self.weight(p, t) > 0.0).then_some(i),
            (None, o) => o,
        }
    }
}

impl Environment for NestedEnv<'_> {
    fn wind(&self, p: &LatLon, t: DateTime<Utc>) -> Option<(f64, f64)> {
        let (i, o) = (self.inner.wind(p, t), self.outer.wind(p, t));
        let w = self.weight(p, t);
        match (i, o) {
            (Some(i), Some(o)) if w > 0.0 && w < 1.0 => {
                let ((iu, iv), (ou, ov)) = (wind_uv(i.0, i.1), wind_uv(o.0, o.1));
                Some(wind_from_uv(iu * w + ou * (1.0 - w), iv * w + ov * (1.0 - w)))
            }
            (Some(i), _) if w >= 1.0 => Some(i),
            (Some(i), None) if w > 0.0 => Some(i),
            (_, o) => o,
        }
    }

    fn has_wind(&self, p: &LatLon, t: DateTime<Utc>) -> bool {
        self.outer.has_wind(p, t) || (self.weight(p, t) > 0.0 && self.inner.has_wind(p, t))
    }

    fn current(&self, p: &LatLon, t: DateTime<Utc>) -> Option<(f64, f64)> {
        let (i, o) = (self.inner.current(p, t), self.outer.current(p, t));
        let e = self.blend(p, t, i.map(|c| c.0), o.map(|c| c.0))?;
        let n = self.blend(p, t, i.map(|c| c.1), o.map(|c| c.1))?;
        Some((e, n))
    }

    fn has_currents(&self) -> bool {
        self.inner.has_currents() || self.outer.has_currents()
    }

    fn wave_height(&self, p: &LatLon, t: DateTime<Utc>) -> Option<f64> {
        self.blend(p, t, self.inner.wave_height(p, t), self.outer.wave_height(p, t))
    }

    fn gust(&self, p: &LatLon, t: DateTime<Utc>) -> Option<f64> {
        self.blend(p, t, self.inner.gust(p, t), self.outer.gust(p, t))
    }

    /// The later of the two ends: past the regional model the global one goes on.
    fn horizon(&self) -> Option<DateTime<Utc>> {
        match (self.inner.horizon(), self.outer.horizon()) {
            (Some(a), Some(b)) => Some(a.max(b)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::grid::Grid;
    use crate::weather::model::{Field, Param};
    use chrono::{Duration, TimeZone};

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap()
    }

    /// A west wind of `u` m/s over a box, for `hours`.
    fn store(u: f32, xll: f64, yll: f64, size: f64, hours: i64) -> WeatherStore {
        let mut s = WeatherStore::new();
        let n = (size / 0.25) as usize + 1;
        for h in (0..=hours).step_by(3) {
            for (p, v) in [(Param::WindU, u), (Param::WindV, 0.0)] {
                s.add(Field { param: p, time: t0() + Duration::hours(h), run: Some(t0()), grid: Grid { ncols: n, nrows: n, xll, yll, dx: 0.25, dy: 0.25, data: vec![v; n * n] } });
            }
        }
        s
    }

    #[test]
    fn the_regional_model_inside_the_global_around_and_after_and_a_blend_between() {
        let global = store(5.0, -20.0, 30.0, 40.0, 120);
        let regional = store(10.0, -10.0, 40.0, 10.0, 48);
        let env = NestedEnv::new(&regional, &global);
        let kn = |lat: f64, lon: f64, h: i64| env.wind(&LatLon::new(lat, lon), t0() + Duration::hours(h)).unwrap().0;
        let (r, g) = (10.0 / 0.514_444, 5.0 / 0.514_444);
        assert!((kn(45.0, -5.0, 12) - r).abs() < 0.01, "inside");
        assert!((kn(35.0, -15.0, 12) - g).abs() < 0.01, "outside");
        assert!((kn(45.0, -5.0, 60) - g).abs() < 0.01, "after the regional forecast ends");
        // Half a degree in from its edge: half and half.
        let edge = kn(45.0, -9.875 + 0.5, 12);
        assert!(edge > g + 1.0 && edge < r - 1.0, "{edge}");
        // An hour and a half before it ends (its last step, held 90 minutes): half and half.
        let late = kn(45.0, -5.0, 48) - (r + g) / 2.0;
        assert!(late.abs() < 1.5, "{late}");
        assert_eq!(env.horizon(), Some(t0() + Duration::hours(120) + Duration::minutes(90)));
        assert_eq!(env.wind(&LatLon::new(45.0, -5.0), t0()).unwrap().1.round(), 270.0, "a west wind stays west");
    }
}
