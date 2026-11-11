//! Averages the instruments into 10 s windows.
//!
//! Fed every tick of the main clock (4 Hz) with `OwnShip`. A reading counts only when it is fresh (it
//! arrived within 3 s), so an instrument that stopped talking is not averaged in; a window is kept only
//! when the boat moved and most of its ticks had a fix. Directions are averaged as vectors.

use super::sample::Sample;
use super::stats::{Circular, Linear, wrap180};
use crate::comm::state::{OwnShip, WindRef};
use crate::geo::LatLon;
use chrono::{DateTime, Duration, Utc};

/// The length of a window.
pub const WINDOW: Duration = Duration::seconds(10);
/// How old a reading may be and still count.
const FRESH: Duration = Duration::seconds(3);
/// A pause in the ticks longer than this starts the window again.
const GAP: Duration = Duration::seconds(5);
/// Ticks with a fix a window needs (half of 4 Hz over 10 s).
const MIN_READINGS: f64 = 20.0;
/// Below this speed over ground the boat is not under way.
const MIN_SOG_KN: f64 = 0.5;

#[derive(Default)]
pub struct Aggregator {
    start: Option<DateTime<Utc>>,
    last: Option<DateTime<Utc>>,
    lat: Linear,
    lon: Linear,
    sog: Linear,
    cog: Circular,
    heading: Circular,
    stw: Linear,
    awa: Circular,
    awa_weighted: Circular,
    aws: Linear,
    twd: Circular,
    tws: Linear,
    wind_ref: Option<WindRef>,
    heel: Linear,
    rpm: Linear,
}

impl Aggregator {
    /// Adds the readings at `now`; returns the window that ended, if it is worth keeping.
    pub fn push(&mut self, own: &OwnShip, now: DateTime<Utc>) -> Option<Sample> {
        if self.last.is_some_and(|l| now - l > GAP || now < l) {
            self.reset();
        }
        let mut out = None;
        if self.start.is_some_and(|s| now - s >= WINDOW) {
            out = self.finish(now);
            self.reset();
        }
        self.add(own, now);
        out
    }

    fn reset(&mut self) {
        *self = Aggregator::default();
    }

    fn add(&mut self, own: &OwnShip, now: DateTime<Utc>) {
        let fresh = |t: Option<DateTime<Utc>>| t.is_some_and(|t| now - t <= FRESH && t <= now + FRESH);
        self.start.get_or_insert(now);
        self.last = Some(now);
        let (Some(pos), Some(sog)) = (own.pos.filter(|_| fresh(own.pos_updated)), own.sog_kn.filter(|_| fresh(own.sog_at))) else {
            return;
        };
        self.lat.add(pos.lat);
        self.lon.add(pos.lon);
        self.sog.add(sog);
        if let Some(c) = own.cog.filter(|_| fresh(own.cog_at)) {
            self.cog.add(c);
        }
        if let Some(h) = own.heading.filter(|_| fresh(own.heading_at)) {
            self.heading.add(h);
        }
        if let Some(s) = own.stw_kn.filter(|_| fresh(own.stw_at)) {
            self.stw.add(s);
        }
        if let Some((a, s)) = own.wind_apparent.filter(|_| fresh(own.wind_apparent_at)) {
            self.awa.add(a);
            self.awa_weighted.add_weighted(a, s.max(0.1));
            self.aws.add(s);
        }
        if let Some((d, s)) = own.wind_true.filter(|_| fresh(own.wind_true_at)) {
            self.twd.add_weighted(d, s.max(0.1));
            self.tws.add(s);
            // A window keeps the weakest reference it saw: over the ground once is over the ground.
            let r = own.wind_true_ref.unwrap_or(WindRef::Instrument);
            self.wind_ref = Some(match (self.wind_ref, r) {
                (Some(WindRef::Ground), _) | (_, WindRef::Ground) => WindRef::Ground,
                (Some(WindRef::Instrument), _) | (_, WindRef::Instrument) => WindRef::Instrument,
                _ => WindRef::Water,
            });
        }
        if let Some((h, at)) = own.heel
            && fresh(Some(at))
        {
            self.heel.add(h);
        }
        let rpm = own.engines.values().filter(|e| fresh(Some(e.at))).filter_map(|e| e.rpm).fold(None, |m: Option<f64>, r| Some(m.map_or(r, |m| m.max(r))));
        if let Some(r) = rpm {
            self.rpm.add(r);
        }
    }

    fn finish(&self, now: DateTime<Utc>) -> Option<Sample> {
        let n = self.sog.count();
        let sog = self.sog.mean()?;
        if n < MIN_READINGS || sog < MIN_SOG_KN {
            return None;
        }
        let start = self.start?;
        // A reading is in the sample when at least half of the ticks with a fix had it.
        let enough = |c: f64| c >= n / 2.0;
        let heading = self.heading.mean().filter(|_| enough(self.heading.count()));
        let sd_heading = if heading.is_some() { self.heading.sd() } else { self.cog.sd() };
        let has_aw = enough(self.aws.count());
        let has_tw = enough(self.tws.count());
        Some(Sample {
            t: start + (now - start) / 2,
            pos: LatLon::new(self.lat.mean()?, self.lon.mean()?),
            secs: (now - start).num_milliseconds() as f64 / 1000.0,
            sog,
            cog: self.cog.mean().unwrap_or(0.0),
            heading,
            stw: self.stw.mean().filter(|_| enough(self.stw.count())),
            awa: self.awa_weighted.mean().filter(|_| has_aw).map(wrap180),
            aws: self.aws.mean().filter(|_| has_aw),
            twd: self.twd.mean().filter(|_| has_tw),
            tws: self.tws.mean().filter(|_| has_tw),
            wind_ref: self.wind_ref.filter(|_| has_tw),
            heel: self.heel.mean().filter(|_| enough(self.heel.count())),
            rpm: self.rpm.mean().filter(|_| enough(self.rpm.count())),
            sd_heading,
            sd_stw: self.stw.sd().filter(|_| enough(self.stw.count())),
            sd_awa: self.awa.sd().filter(|_| has_aw),
            n: n as u32,
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comm::nmea::{frame, parse_line};
    use chrono::TimeZone;

    fn t(ms: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_780_000_000, 0).unwrap() + Duration::milliseconds(ms)
    }

    fn feed(o: &mut OwnShip, body: &str, at: DateTime<Utc>) {
        o.apply(&parse_line(&frame(body, '$')).unwrap().unwrap(), at);
    }

    /// Ticks at 4 Hz for `secs` with the instruments talking, heading swinging either side of north.
    fn sail(a: &mut Aggregator, o: &mut OwnShip, from_ms: i64, secs: i64, with_speedo: bool) -> Vec<Sample> {
        let mut out = Vec::new();
        for i in 0..secs * 4 {
            let now = t(from_ms + i * 250);
            feed(o, "GPGLL,4807.038,N,00431.000,W,123519,A", now);
            feed(o, "GPVTG,005.0,T,,M,6.0,N,,K,A", now);
            feed(o, if i % 2 == 0 { "HEHDT,358.0,T" } else { "HEHDT,2.0,T" }, now);
            if with_speedo {
                feed(o, "IIVHW,,T,,M,5.5,N,,K", now);
            }
            feed(o, if i % 2 == 0 { "IIMWV,040,R,15.0,N,A" } else { "IIMWV,044,R,15.0,N,A" }, now);
            out.extend(a.push(o, now));
        }
        out
    }

    #[test]
    fn windows_average_the_instruments() {
        let (mut a, mut o) = (Aggregator::default(), OwnShip::default());
        let samples = sail(&mut a, &mut o, 0, 31, true);
        assert_eq!(samples.len(), 3);
        let s = &samples[0];
        assert!((s.secs - 10.0).abs() < 0.01 && s.n == 40, "{} {}", s.secs, s.n);
        assert_eq!((s.sog, s.stw), (6.0, Some(5.5)));
        let h = s.heading.unwrap();
        assert!(h < 0.01 || h > 359.99, "averaged across north: {h}");
        assert!((s.awa.unwrap() - 42.0).abs() < 0.01 && (s.sd_awa.unwrap() - 2.0).abs() < 0.1);
        assert!((s.sd_heading.unwrap() - 2.0).abs() < 0.1);
        assert_eq!(s.wind_ref, Some(WindRef::Water));
        assert!(s.tws.is_some() && s.twd.is_some());
        assert_eq!(s.t, t(5000));
    }

    #[test]
    fn a_silent_speedo_is_left_out() {
        let (mut a, mut o) = (Aggregator::default(), OwnShip::default());
        // The speedo talks for the first few seconds of the log, then stops.
        sail(&mut a, &mut o, 0, 2, true);
        let samples = sail(&mut a, &mut o, 2000, 25, false);
        let last = samples.last().unwrap();
        assert_eq!(last.stw, None);
        assert_eq!(last.wind_ref, Some(WindRef::Ground), "the true wind is over the ground without the speedo");
    }

    #[test]
    fn standing_still_or_a_pause_keeps_nothing() {
        let (mut a, mut o) = (Aggregator::default(), OwnShip::default());
        for i in 0..200 {
            let now = t(i * 250);
            feed(&mut o, "GPGLL,4807.038,N,00431.000,W,123519,A", now);
            feed(&mut o, "GPVTG,005.0,T,,M,0.1,N,,K,A", now);
            assert!(a.push(&o, now).is_none());
        }
        // Ticks that stop for longer than the gap restart the window rather than spanning the pause.
        let (mut a, mut o) = (Aggregator::default(), OwnShip::default());
        sail(&mut a, &mut o, 0, 6, true);
        let after = sail(&mut a, &mut o, 60_000, 9, true);
        assert!(after.is_empty());
    }
}
