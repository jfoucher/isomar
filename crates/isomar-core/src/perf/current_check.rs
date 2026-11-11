//! The stream the boat measures under way (the way over the ground less the way through the water, from
//! the log and the compass), averaged over minutes into a set and drift, and each loaded current source
//! scored against it over the last hours, as `model_check` scores the wind models.

use crate::currents::CurrentSource;
use crate::geo::set_of;
use chrono::{DateTime, Duration, Utc};

/// The set and drift are averaged over this many minutes: a degree of heading error is a tenth of a
/// knot at 6 kn, so a window of seconds says little.
pub const SET_MINUTES: i64 = 5;
/// How much of `SET_MINUTES` must be measured, minutes.
const SET_MIN_MINUTES: i64 = 3;
/// How far back sources are scored.
pub const WINDOW_HOURS: i64 = 6;
/// A source in use further off the measured stream than this on average...
pub const FAR_KN: f64 = 1.0;
/// ...over this long is said to be off.
pub const FAR_MINUTES: i64 = 30;
/// The windows' length, seconds (the performance log's).
const WINDOW_S: f64 = 10.0;

/// A current as (east, north) knots.
type Kn = (f64, f64);

#[derive(Clone, Debug, PartialEq)]
struct Obs {
    t: DateTime<Utc>,
    measured: (f64, f64),
    forecast: Vec<(CurrentSource, (f64, f64))>,
}

/// One source against the measured stream.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurrentScore {
    pub source: CurrentSource,
    pub hours: f64,
    /// The root-mean-square of the difference, as vectors, knots.
    pub rms_kn: f64,
    /// Its speed minus the measured, on average, knots; its set minus the measured, degrees.
    pub speed_bias: f64,
    pub set_bias: f64,
}

#[derive(Clone, Debug, Default)]
pub struct CurrentTracker {
    obs: Vec<Obs>,
}

fn mean(v: &[(f64, f64)]) -> (f64, f64) {
    let n = v.len().max(1) as f64;
    (v.iter().map(|x| x.0).sum::<f64>() / n, v.iter().map(|x| x.1).sum::<f64>() / n)
}

impl CurrentTracker {
    /// A logged window at `t`: the stream measured, and each source's there and then.
    pub fn add(&mut self, t: DateTime<Utc>, measured: (f64, f64), forecast: Vec<(CurrentSource, (f64, f64))>) {
        self.obs.push(Obs { t, measured, forecast });
        let cut = t - Duration::hours(WINDOW_HOURS);
        self.obs.retain(|o| o.t >= cut);
    }

    fn since(&self, from: DateTime<Utc>) -> impl Iterator<Item = &Obs> {
        self.obs.iter().filter(move |o| o.t >= from)
    }

    /// The set (degrees, where the stream goes) and drift (knots) over the last `SET_MINUTES`.
    pub fn set_and_drift(&self, now: DateTime<Utc>) -> Option<(f64, f64)> {
        let v: Vec<(f64, f64)> = self.since(now - Duration::minutes(SET_MINUTES)).map(|o| o.measured).collect();
        if (v.len() as f64 * WINDOW_S) < (SET_MIN_MINUTES * 60) as f64 {
            return None;
        }
        let (e, n) = mean(&v);
        Some((set_of(e, n), e.hypot(n)))
    }

    /// Each source's score over the last `WINDOW_HOURS`, best first. Each is compared over minutes, as
    /// the set and drift are, so the noise of single windows does not count against it.
    pub fn scores(&self, now: DateTime<Utc>) -> Vec<CurrentScore> {
        let obs: Vec<&Obs> = self.since(now - Duration::hours(WINDOW_HOURS)).collect();
        let mut out: Vec<CurrentScore> = CurrentSource::ALL
            .into_iter()
            .filter_map(|source| {
                // Pairs of the measured stream and the source's, in buckets of `SET_MINUTES`.
                let mut buckets: std::collections::BTreeMap<i64, (Vec<Kn>, Vec<Kn>)> = Default::default();
                for o in &obs {
                    let Some((_, f)) = o.forecast.iter().find(|(s, _)| *s == source) else { continue };
                    let b = buckets.entry(o.t.timestamp() / (SET_MINUTES * 60)).or_default();
                    b.0.push(o.measured);
                    b.1.push(*f);
                }
                let pairs: Vec<(Kn, Kn, usize)> = buckets.values().filter(|b| (b.0.len() as f64 * WINDOW_S) >= (SET_MIN_MINUTES * 60) as f64).map(|b| (mean(&b.0), mean(&b.1), b.0.len())).collect();
                if pairs.is_empty() {
                    return None;
                }
                let n = pairs.len() as f64;
                let rms = (pairs.iter().map(|(m, f, _)| (f.0 - m.0).powi(2) + (f.1 - m.1).powi(2)).sum::<f64>() / n).sqrt();
                let speed_bias = pairs.iter().map(|(m, f, _)| f.0.hypot(f.1) - m.0.hypot(m.1)).sum::<f64>() / n;
                let set_bias = pairs.iter().map(|(m, f, _)| crate::perf::stats::angle_diff(set_of(f.0, f.1), set_of(m.0, m.1))).sum::<f64>() / n;
                Some(CurrentScore { source, hours: pairs.iter().map(|p| p.2).sum::<usize>() as f64 * WINDOW_S / 3600.0, rms_kn: rms, speed_bias, set_bias })
            })
            .collect();
        out.sort_by(|a, b| a.rms_kn.total_cmp(&b.rms_kn));
        out
    }

    /// The source in use (the first of each window's, the finest there) when it has been more than
    /// `FAR_KN` off the measured stream on average over the last `FAR_MINUTES`, and for most of them:
    /// that source, the measured stream and the source's, (east, north) knots.
    pub fn far_off(&self, now: DateTime<Utc>) -> Option<(CurrentSource, Kn, Kn)> {
        let recent: Vec<&Obs> = self.since(now - Duration::minutes(FAR_MINUTES)).collect();
        let source = recent.last()?.forecast.first()?.0;
        let pairs: Vec<((f64, f64), (f64, f64))> = recent.iter().filter_map(|o| o.forecast.iter().find(|(s, _)| *s == source).map(|(_, f)| (o.measured, *f))).collect();
        if (pairs.len() as f64 * WINDOW_S) < (FAR_MINUTES * 60) as f64 * 0.66 {
            return None;
        }
        let m = mean(&pairs.iter().map(|p| p.0).collect::<Vec<_>>());
        let f = mean(&pairs.iter().map(|p| p.1).collect::<Vec<_>>());
        ((f.0 - m.0).hypot(f.1 - m.1) > FAR_KN).then_some((source, m, f))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(sec: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap() + Duration::seconds(sec)
    }

    #[test]
    fn the_set_and_drift_average_the_noise_away() {
        let mut k = CurrentTracker::default();
        assert_eq!(k.set_and_drift(t(0)), None);
        // 1.5 kn setting east, each window off by half a knot one way or the other.
        for i in 0..30 {
            let noise = if i % 2 == 0 { 0.5 } else { -0.5 };
            k.add(t(i * 10), (1.5, noise), vec![]);
        }
        let (set, drift) = k.set_and_drift(t(300)).unwrap();
        assert!((set - 90.0).abs() < 2.0 && (drift - 1.5).abs() < 0.05, "{set} {drift}");
        assert_eq!(k.set_and_drift(t(300 + 6 * 60)), None, "too old");
    }

    #[test]
    fn the_source_nearest_the_measured_stream_comes_first_and_one_far_off_is_said() {
        let mut k = CurrentTracker::default();
        // An hour: the stream 2 kn north; the atlas within a few tenths, SMOC half of it and to the east.
        for i in 0..360 {
            k.add(t(i * 10), (0.0, 2.0), vec![(CurrentSource::Atlas, (0.1, 1.8)), (CurrentSource::Smoc, (0.7, 0.7))]);
        }
        let s = k.scores(t(3600));
        assert_eq!(s.iter().map(|x| x.source).collect::<Vec<_>>(), vec![CurrentSource::Atlas, CurrentSource::Smoc]);
        assert!(s[0].rms_kn < 0.3 && s[1].rms_kn > 1.0);
        assert!(s[1].speed_bias < -0.9 && s[1].set_bias > 40.0, "{s:?}");
        assert!((s[0].hours - 1.0).abs() < 0.05);
        assert_eq!(k.far_off(t(3600)), None, "the atlas, in use, is close");
        let mut far = CurrentTracker::default();
        for i in 0..200 {
            far.add(t(i * 10), (0.0, 3.0), vec![(CurrentSource::Smoc, (0.0, 0.5))]);
        }
        assert_eq!(far.far_off(t(2000)).map(|x| x.0), Some(CurrentSource::Smoc));
    }
}
