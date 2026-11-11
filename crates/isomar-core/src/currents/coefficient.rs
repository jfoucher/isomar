//! Using a tidal atlas at a moment: where the moment falls in the tide at the atlas's reference port
//! (hours from its high water, or low water for some), and the day's coefficient from Brest's range, as
//! the paper atlases are used.

use crate::tides::Extreme;
use chrono::{DateTime, Utc};

/// Brest, where French coefficients are reckoned.
pub const BREST: (f64, f64) = (48.383, -4.495);
/// Half of Brest's mean spring range, metres: a coefficient of 100.
pub const BREST_UNIT_M: f64 = 3.05;
/// The coefficients the atlases are drawn for (mean neaps, mean springs)...
pub const NEAPS: f64 = 45.0;
pub const SPRINGS: f64 = 95.0;
/// ...and how far past them they are stretched.
pub const MIN_COEFF: f64 = 30.0;
pub const MAX_COEFF: f64 = 120.0;

fn hours(a: DateTime<Utc>, b: DateTime<Utc>) -> f64 {
    (b - a).num_seconds() as f64 / 3600.0
}

/// Hours of `t` from the reference extreme nearest it (a high water, or a low water when `low`), scaled
/// so that the moment halfway to the next reference extreme, or from the last, is ±6 h: the paper atlas's
/// hours run from HW−6 to HW+6 whatever the tide's real length. `None` when `extremes` do not reach
/// either side of `t`.
pub fn phase_hours(extremes: &[Extreme], low: bool, t: DateTime<Utc>) -> Option<f64> {
    let refs: Vec<DateTime<Utc>> = extremes.iter().filter(|e| e.high != low).map(|e| e.time).collect();
    let i = refs.partition_point(|r| *r <= t);
    let (before, after) = (i.checked_sub(1).map(|j| refs[j]), refs.get(i).copied());
    let (before, after) = (before?, after?);
    let half = hours(before, after) / 2.0;
    if half <= 0.0 {
        return None;
    }
    let since = hours(before, t);
    Some(if since <= half { 6.0 * since / half } else { -6.0 * (half * 2.0 - since) / half })
}

/// The coefficient of the tide at Brest nearest `t`, from its high water and the low waters either side:
/// 100 × half the range ÷ 3.05 m. `None` without a high water within a tide of `t`.
pub fn coefficient(brest: &[Extreme], t: DateTime<Utc>) -> Option<f64> {
    let (i, hw) = brest.iter().enumerate().filter(|(_, e)| e.high).min_by_key(|(_, e)| (e.time - t).num_seconds().abs())?;
    if hours(hw.time, t).abs() > 13.0 {
        return None;
    }
    let lows: Vec<f64> = [i.checked_sub(1).and_then(|j| brest.get(j)), brest.get(i + 1)].into_iter().flatten().filter(|e| !e.high).map(|e| e.level_m).collect();
    if lows.is_empty() {
        return None;
    }
    let low = lows.iter().sum::<f64>() / lows.len() as f64;
    Some(100.0 * (hw.level_m - low) / 2.0 / BREST_UNIT_M)
}

/// The weight of springs against neaps for a coefficient: 0 at 45, 1 at 95, past them linearly, the
/// coefficient held to 30..120.
pub fn springs_weight(coeff: f64) -> f64 {
    (coeff.clamp(MIN_COEFF, MAX_COEFF) - NEAPS) / (SPRINGS - NEAPS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap()
    }

    /// A semi-diurnal tide: high waters every 12 h 25 from `t0`, lows halfway, `range` metres.
    fn tide(range: f64) -> Vec<Extreme> {
        (0..10)
            .map(|k| Extreme { time: t0() + Duration::minutes(k * 745 / 2), level_m: if k % 2 == 0 { 4.0 + range / 2.0 } else { 4.0 - range / 2.0 }, high: k % 2 == 0 })
            .collect()
    }

    #[test]
    fn the_phase_runs_from_minus_six_to_six_between_high_waters() {
        let ext = tide(5.0);
        let at = |m: i64| phase_hours(&ext, false, t0() + Duration::minutes(745 + m)).unwrap();
        assert!(at(0).abs() < 1e-9, "at high water");
        assert!((at(120) - 2.0 * 6.0 / 6.208).abs() < 1e-3, "two hours after, scaled to the 12 h 25 tide");
        assert!((at(-120) + 2.0 * 6.0 / 6.208).abs() < 1e-3);
        assert!((at(372) - 6.0).abs() < 0.01 && (at(373) + 6.0).abs() < 0.01, "halfway, it turns from +6 to -6");
        // Referred to low water.
        assert!(phase_hours(&ext, true, t0() + Duration::minutes(3 * 745 / 2)).unwrap().abs() < 0.01);
        assert!(phase_hours(&ext, false, t0() - Duration::hours(1)).is_none(), "before the first high water");
    }

    #[test]
    fn the_coefficient_is_brests_half_range_in_hundredths_of_its_unit() {
        let springs = tide(6.1);
        assert!((coefficient(&springs, t0() + Duration::hours(25)).unwrap() - 100.0).abs() < 1e-6);
        assert!((coefficient(&tide(2.745), t0() + Duration::hours(25)).unwrap() - 45.0).abs() < 1e-6, "mean neaps");
        assert!(coefficient(&springs, t0() + Duration::days(9)).is_none(), "far from any high water");
        assert_eq!(springs_weight(45.0), 0.0);
        assert_eq!(springs_weight(95.0), 1.0);
        assert_eq!(springs_weight(70.0), 0.5);
        assert_eq!(springs_weight(150.0), 1.5, "held to 120");
        assert_eq!(springs_weight(10.0), -0.3, "held to 30");
    }

    /// Brest's coefficients from Open Waters over the next fortnight: in the published range, and
    /// swinging from neaps to springs. `cargo test -p isomar-core live_brest -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_brest_coefficients() {
        let dir = tempfile::tempdir().unwrap();
        let now = Utc::now();
        let (f, _) = crate::tides::lookup(dir.path(), BREST.0, BREST.1, now, 15 * 24).unwrap();
        let cs: Vec<(DateTime<Utc>, f64)> = f.extremes.iter().filter(|e| e.high && e.time > now && e.time < now + Duration::days(15)).filter_map(|e| Some((e.time, coefficient(&f.extremes, e.time)?))).collect();
        for (t, c) in &cs {
            eprintln!("{} {c:.0}", t.format("%a %d %b %H:%M"));
        }
        let (lo, hi) = cs.iter().fold((f64::MAX, f64::MIN), |(a, b), (_, c)| (a.min(*c), b.max(*c)));
        assert!(lo > 15.0 && hi < 125.0, "{lo}..{hi}");
        assert!(hi - lo > 30.0, "springs and neaps within a fortnight: {lo}..{hi}");
    }
}
