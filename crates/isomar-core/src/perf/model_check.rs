//! Which forecast model is tracking the wind the boat measures: every logged window, each loaded
//! model's wind there and then against the instruments', over the last few hours. Before leaving no one
//! can say which model is right; under way the boat can.

use super::stats::angle_diff;
use crate::weather::source::Source;
use chrono::{DateTime, Duration, Utc};

/// How far back the comparison looks.
pub const WINDOW_HOURS: i64 = 6;
/// How much of the window a model needs measured against before it is judged, minutes.
pub const MIN_MINUTES: i64 = 30;
/// How many degrees of direction error count as one knot of speed error when models are ranked.
const DEG_PER_KN: f64 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Obs {
    t: DateTime<Utc>,
    source: Source,
    /// The model's speed minus the instruments', knots, and its direction minus theirs, degrees.
    dtws: f64,
    dtwd: f64,
}

/// One model's errors over the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Score {
    pub source: Source,
    /// How long it was measured against, hours (windows of about 10 s).
    pub hours: f64,
    pub tws_bias: f64,
    pub tws_rms: f64,
    pub twd_bias: f64,
    pub twd_rms: f64,
}

impl Score {
    /// One figure for ranking: the speed error plus the direction error at `DEG_PER_KN`.
    pub fn error(&self) -> f64 {
        self.tws_rms + self.twd_rms / DEG_PER_KN
    }
}

#[derive(Clone, Debug, Default)]
pub struct ModelTracker {
    obs: Vec<Obs>,
}

impl ModelTracker {
    /// A logged window at `t`: a model's wind there (knots, from) and the instruments'.
    pub fn add(&mut self, t: DateTime<Utc>, source: Source, model: (f64, f64), measured: (f64, f64)) {
        self.obs.push(Obs { t, source, dtws: model.0 - measured.0, dtwd: angle_diff(model.1, measured.1) });
        let cut = t - Duration::hours(WINDOW_HOURS);
        self.obs.retain(|o| o.t >= cut);
    }

    /// Each model's score over the window up to `now`, best first.
    pub fn scores(&self, now: DateTime<Utc>) -> Vec<Score> {
        let cut = now - Duration::hours(WINDOW_HOURS);
        let mut out: Vec<Score> = Source::ALL
            .into_iter()
            .filter_map(|source| {
                let v: Vec<&Obs> = self.obs.iter().filter(|o| o.source == source && o.t >= cut).collect();
                if v.is_empty() {
                    return None;
                }
                let n = v.len() as f64;
                let mean = |f: &dyn Fn(&Obs) -> f64| v.iter().map(|o| f(o)).sum::<f64>() / n;
                Some(Score {
                    source,
                    hours: n * 10.0 / 3600.0,
                    tws_bias: mean(&|o| o.dtws),
                    tws_rms: mean(&|o| o.dtws * o.dtws).sqrt(),
                    twd_bias: mean(&|o| o.dtwd),
                    twd_rms: mean(&|o| o.dtwd * o.dtwd).sqrt(),
                })
            })
            .collect();
        out.sort_by(|a, b| a.error().total_cmp(&b.error()));
        out
    }

    /// The model clearly closest to the measured wind, when another one is in use: measured against
    /// for long enough, and with an error at least a third smaller than `in_use`'s.
    pub fn better_than(&self, in_use: Source, now: DateTime<Utc>) -> Option<Score> {
        let scores = self.scores(now);
        let enough = |s: &Score| s.hours * 60.0 >= MIN_MINUTES as f64;
        let ours = scores.iter().find(|s| s.source == in_use && enough(s))?;
        let best = scores.iter().find(|s| enough(s))?;
        (best.source != in_use && best.error() < ours.error() * 2.0 / 3.0).then_some(*best)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(min: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap() + Duration::minutes(min)
    }

    #[test]
    fn the_model_nearest_the_instruments_comes_first_and_old_windows_go() {
        let mut k = ModelTracker::default();
        // An hour of windows every 10 s: GFS 6 kn light and 20° backed, IFS within a knot.
        for i in 0..360 {
            let at = t(i / 6);
            k.add(at, Source::Gfs, (12.0, 250.0), (18.0, 270.0));
            k.add(at, Source::Ifs, (17.5 + (i % 2) as f64, 272.0), (18.0, 270.0));
        }
        let s = k.scores(t(60));
        assert_eq!(s.iter().map(|x| x.source).collect::<Vec<_>>(), vec![Source::Ifs, Source::Gfs]);
        assert!((s[1].tws_bias + 6.0).abs() < 1e-9 && (s[1].twd_bias + 20.0).abs() < 1e-9);
        assert!((s[0].hours - 1.0).abs() < 0.01);
        assert_eq!(k.better_than(Source::Gfs, t(60)).map(|x| x.source), Some(Source::Ifs));
        assert_eq!(k.better_than(Source::Ifs, t(60)), None, "it is the best already");
        // Six hours on, the hour has gone.
        assert!(k.scores(t(60 + 6 * 60 + 1)).is_empty());
    }

    #[test]
    fn too_short_a_comparison_judges_nothing() {
        let mut k = ModelTracker::default();
        for i in 0..60 {
            k.add(t(i / 6), Source::Gfs, (10.0, 0.0), (20.0, 90.0));
            k.add(t(i / 6), Source::Ifs, (20.0, 90.0), (20.0, 90.0));
        }
        assert_eq!(k.better_than(Source::Gfs, t(10)), None, "ten minutes");
    }
}
