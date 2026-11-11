//! Synthetic performance logs: a boat whose real polar, instrument faults, currents and forecast errors are
//! known, sailing legs in a wind that wanders, beating and running at its best angles and changing tack.
//! For the tests (does the analysis find what was put in?) and for `ISOMAR_DEMO_PERF`.

use super::rng::Rng;
use super::sample::{ModelWind, SailMode, Sample};
use super::stats::{Drift, wrap180};
use crate::comm::sim::apparent_wind;
use crate::comm::state::{WindRef, true_wind};
use crate::geo::{LatLon, normalize_bearing};
use crate::routing::polar::Polar;
use chrono::{DateTime, Duration, TimeZone, Utc};

#[derive(Clone, Debug)]
pub struct Config {
    pub hours: f64,
    pub seed: u64,
    /// What the boat really does.
    pub truth: Polar,
    /// Wind instruments, a speedo and a compass; else a GPS alone.
    pub instruments: bool,
    /// Degrees the masthead unit adds to every apparent angle (to starboard).
    pub awa_offset: f64,
    /// Degrees the apparent angle reads too wide, upwind and downwind (upwash).
    pub angle_error: [f64; 2],
    /// What the speedo reads for each knot through the water.
    pub speedo_factor: f64,
    /// A tidal stream of this rate, its set turning round every 12.42 h.
    pub current_kn: f64,
    /// The forecast's errors: direction in degrees, speed as a fraction, each persisting for hours.
    pub forecast_sd: (f64, f64),
    /// How much the boat's speed wanders round its polar (trim, waves, the helm), as a fraction.
    pub speed_noise: f64,
    /// Share of the legs under engine.
    pub motoring_share: f64,
    pub start: DateTime<Utc>,
    pub pos: LatLon,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            hours: 100.0,
            seed: 1,
            truth: Polar::generic_cruiser(),
            instruments: true,
            awa_offset: 0.0,
            angle_error: [0.0, 0.0],
            speedo_factor: 1.0,
            current_kn: 0.0,
            forecast_sd: (25.0, 0.3),
            speed_noise: 0.03,
            motoring_share: 0.1,
            start: Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap(),
            pos: LatLon::new(47.5, -4.0),
        }
    }
}

/// The boat's polar scaled: `upwind` below 70 degrees, `downwind` above 120, graded between.
pub fn scaled(p: &Polar, upwind: f64, downwind: f64) -> Polar {
    let mut q = p.clone();
    for (row, a) in p.twa.iter().enumerate() {
        let f = if *a <= 70.0 {
            upwind
        } else if *a >= 120.0 {
            downwind
        } else {
            upwind + (downwind - upwind) * (a - 70.0) / 50.0
        };
        for col in 0..p.tws.len() {
            q.speeds[row][col] = p.speeds[row][col] * f;
        }
    }
    q
}

/// Ten-second samples of `c.hours` of sailing.
pub fn log(c: &Config) -> Vec<Sample> {
    const DT: f64 = 10.0;
    let mut rng = Rng::new(c.seed);
    let steps = (c.hours * 3600.0 / DT) as usize;
    let mut out = Vec::with_capacity(steps);
    let mut pos = c.pos;
    let (mut dir_noise, mut kn_noise) = (Drift::new(6.0, 3600.0, &mut rng), Drift::new(1.0, 1800.0, &mut rng));
    let mut speed_noise = Drift::new(c.speed_noise, 300.0, &mut rng);
    let (mut f_dir, mut f_kn) = (Drift::new(c.forecast_sd.0, 3.0 * 3600.0, &mut rng), Drift::new(c.forecast_sd.1, 3.0 * 3600.0, &mut rng));
    let mut leg_left = 0.0;
    let (mut course, mut motoring) = (0.0, false);
    let (mut side, mut tack_left) = (1.0, 0.0);
    let mut last_heading: Option<f64> = None;
    let mut lead_h = 0.0;
    for i in 0..steps {
        let secs = i as f64 * DT;
        let t = c.start + Duration::milliseconds((secs * 1000.0) as i64);
        let h = secs / 3600.0;
        if i % 2160 == 0 {
            lead_h = rng.range(0.0, 48.0);
        }
        let twd = normalize_bearing(250.0 + 40.0 * (std::f64::consts::TAU * h / 19.0).sin() + dir_noise.step(DT, &mut rng));
        let tws = (13.0 + 8.0 * (std::f64::consts::TAU * h / 17.0 + 1.0).sin() + 3.0 * (std::f64::consts::TAU * h / 7.0).sin() + kn_noise.step(DT, &mut rng)).max(2.0);
        if leg_left <= 0.0 {
            leg_left = rng.range(1.5, 4.0) * 3600.0;
            course = rng.range(0.0, 360.0);
            motoring = rng.uniform() < c.motoring_share;
            tack_left = 0.0;
        }
        leg_left -= DT;
        // Where to point: the course, or the best angle on a tack towards it.
        let heading = if motoring {
            course
        } else {
            let to_course = wrap180(course - twd);
            let (up, _) = c.truth.best_vmg(tws, true);
            let (down, _) = c.truth.best_vmg(tws, false);
            let beat = to_course.abs() < up;
            let run = to_course.abs() > down;
            if beat || run {
                if tack_left <= 0.0 {
                    side = if tack_left == 0.0 && to_course != 0.0 { to_course.signum() } else { -side };
                    tack_left = rng.range(15.0, 35.0) * 60.0;
                }
                tack_left -= DT;
                normalize_bearing(twd + side * if beat { up } else { down })
            } else {
                tack_left = 0.0;
                course
            }
        };
        let twa = wrap180(twd - heading);
        let stw = if motoring { 6.0 } else { (c.truth.speed(twa, tws) * (1.0 + speed_noise.step(DT, &mut rng))).max(0.0) };
        let set = 360.0 * h / 12.42;
        let (cx, cy) = (c.current_kn * set.to_radians().sin(), c.current_kn * set.to_radians().cos());
        let (vx, vy) = (stw * heading.to_radians().sin() + cx, stw * heading.to_radians().cos() + cy);
        let sog = vx.hypot(vy);
        let cog = normalize_bearing(vx.atan2(vy).to_degrees());
        pos = pos.destination(cog, sog * DT / 3600.0);
        let turned = last_heading.is_some_and(|l| wrap180(heading - l).abs() > 20.0);
        last_heading = Some(heading);
        let mut s = Sample {
            t,
            pos,
            secs: DT,
            sog,
            cog,
            sd_heading: Some(if turned { 30.0 } else { 1.5 + rng.uniform() }),
            n: 40,
            mode: if motoring { SailMode::Motoring } else { SailMode::Sailing },
            forecast: Some(ModelWind {
                tws: (tws * (1.0 + f_kn.step(DT, &mut rng))).max(0.0),
                twd: normalize_bearing(twd + f_dir.step(DT, &mut rng)),
                gust: Some(tws * 1.3),
                hs: Some(0.2 + tws * tws / 250.0),
                wave_dir: Some(twd),
                wave_period: Some(4.0 + tws / 4.0),
                run: Some(t - Duration::minutes((lead_h * 60.0) as i64)),
                lead_h: Some(lead_h),
                current: None,
            }),
            ..Default::default()
        };
        if c.instruments {
            let (awa_true, aws) = apparent_wind(twd, tws, heading, stw);
            let awa_true = wrap180(awa_true);
            let e = c.angle_error[if twa.abs() < 90.0 { 0 } else { 1 }];
            let awa = wrap180(awa_true.signum() * (awa_true.abs() + e) + c.awa_offset);
            let stw_read = stw * c.speedo_factor;
            let (twa_read, tws_read) = true_wind(awa, aws, stw_read);
            s.heading = Some(heading);
            s.stw = Some(stw_read);
            s.sd_stw = Some(if turned { 0.8 } else { 0.08 });
            s.awa = Some(awa);
            s.aws = Some(aws);
            s.sd_awa = Some(if turned { 40.0 } else { 3.0 });
            s.twd = Some(normalize_bearing(heading + twa_read));
            s.tws = Some(tws_read);
            s.wind_ref = Some(WindRef::Water);
        }
        out.push(s);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_log_reads_what_the_boat_did() {
        let c = Config { hours: 20.0, motoring_share: 0.0, speed_noise: 0.0, ..Default::default() };
        let log = log(&c);
        assert_eq!(log.len(), 7200);
        for s in log.iter().filter(|s| s.sd_heading.unwrap() < 5.0) {
            let (twa, tws) = (s.twa().unwrap(), s.tws.unwrap());
            assert!((s.stw.unwrap() - c.truth.speed(twa, tws)).abs() < 0.02, "{twa} {tws} {}", s.stw.unwrap());
            assert!((s.sog - s.stw.unwrap()).abs() < 1e-9, "no current");
        }
        // It beats: the close-hauled samples sit at the best angle on both tacks.
        let up = log.iter().filter(|s| s.twa().unwrap().abs() < 60.0).count();
        assert!(up > 100, "{up}");
        let (port, stbd) = (log.iter().filter(|s| s.twa().unwrap() < 0.0).count(), log.iter().filter(|s| s.twa().unwrap() > 0.0).count());
        assert!(port > 500 && stbd > 500, "{port} {stbd}");
        let winds: Vec<f64> = log.iter().map(|s| s.tws.unwrap()).collect();
        let (lo, hi) = winds.iter().fold((f64::MAX, 0.0f64), |(a, b), w| (a.min(*w), b.max(*w)));
        assert!(lo < 10.0 && hi > 15.0, "{lo} {hi}");
    }
}
