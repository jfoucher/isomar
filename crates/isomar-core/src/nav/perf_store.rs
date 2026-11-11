//! The performance log's rows in the navigation database (`perf::Sample`, `perf::passage::Passage`).

use super::store::{NavStore, Result, from_ts};
use crate::comm::state::WindRef;
use crate::geo::LatLon;
use crate::perf::passage::Passage;
use crate::perf::{Hint, ModeSource, ModelWind, SailMode, Sample};
use chrono::{DateTime, Utc};
use rusqlite::{Row, params};

const COLUMNS: &str = "id, t, lat, lon, secs, sog, cog, heading, stw, awa, aws, twd, tws, wind_ref, heel, rpm, sd_heading, sd_stw, sd_awa, n, mode, mode_from, hint, \
    f_tws, f_twd, f_gust, f_hs, f_wave_dir, f_wave_period, f_run, f_lead_h, h_tws, h_twd, h_run, h_lead_h, polar, f_cur_e, f_cur_n";

/// How much the log holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PerfSummary {
    pub count: usize,
    pub first: Option<DateTime<Utc>>,
    pub last: Option<DateTime<Utc>>,
}

fn ref_code(r: Option<WindRef>) -> Option<i64> {
    r.map(|r| match r {
        WindRef::Water => 1,
        WindRef::Ground => 2,
        WindRef::Instrument => 3,
    })
}

fn ref_of(c: Option<i64>) -> Option<WindRef> {
    match c? {
        1 => Some(WindRef::Water),
        2 => Some(WindRef::Ground),
        3 => Some(WindRef::Instrument),
        _ => None,
    }
}

fn row(r: &Row) -> rusqlite::Result<Sample> {
    let model = |at: usize, waves: bool| -> rusqlite::Result<Option<ModelWind>> {
        let (tws, twd): (Option<f64>, Option<f64>) = (r.get(at)?, r.get(at + 1)?);
        let (Some(tws), Some(twd)) = (tws, twd) else { return Ok(None) };
        Ok(Some(if waves {
            ModelWind { tws, twd, gust: r.get(at + 2)?, hs: r.get(at + 3)?, wave_dir: r.get(at + 4)?, wave_period: r.get(at + 5)?, run: from_ts(r.get(at + 6)?), lead_h: r.get(at + 7)?, current: None }
        } else {
            ModelWind { tws, twd, gust: None, hs: None, wave_dir: None, wave_period: None, run: from_ts(r.get(at + 2)?), lead_h: r.get(at + 3)?, current: None }
        }))
    };
    Ok(Sample {
        id: r.get(0)?,
        t: from_ts(r.get(1)?).unwrap_or_default(),
        pos: LatLon::new(r.get(2)?, r.get(3)?),
        secs: r.get(4)?,
        sog: r.get(5)?,
        cog: r.get(6)?,
        heading: r.get(7)?,
        stw: r.get(8)?,
        awa: r.get(9)?,
        aws: r.get(10)?,
        twd: r.get(11)?,
        tws: r.get(12)?,
        wind_ref: ref_of(r.get(13)?),
        heel: r.get(14)?,
        rpm: r.get(15)?,
        sd_heading: r.get(16)?,
        sd_stw: r.get(17)?,
        sd_awa: r.get(18)?,
        n: r.get(19)?,
        mode: SailMode::from_code(r.get(20)?),
        mode_from: if r.get::<_, i64>(21)? == 1 { ModeSource::Engine } else { ModeSource::User },
        hint: if r.get::<_, i64>(22)? == 1 { Hint::LooksMotored } else { Hint::None },
        forecast: model(23, true)?.map(|mut m| -> rusqlite::Result<ModelWind> {
            let (e, n): (Option<f64>, Option<f64>) = (r.get(36)?, r.get(37)?);
            m.current = e.zip(n);
            Ok(m)
        }).transpose()?,
        hindcast: model(31, false)?,
        polar: r.get(35)?,
    })
}

impl NavStore {
    pub fn insert_perf_sample(&self, s: &Sample) -> Result<i64> {
        let f = s.forecast;
        let h = s.hindcast;
        self.conn.execute(
            "INSERT INTO perf_samples(t, lat, lon, secs, sog, cog, heading, stw, awa, aws, twd, tws, wind_ref, heel, rpm, sd_heading, sd_stw, sd_awa, n, mode, mode_from, hint,
                f_tws, f_twd, f_gust, f_hs, f_wave_dir, f_wave_period, f_run, f_lead_h, h_tws, h_twd, h_run, h_lead_h, polar, f_cur_e, f_cur_n)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30,?31,?32,?33,?34,?35,?36,?37)",
            params![
                s.t.timestamp(),
                s.pos.lat,
                s.pos.lon,
                s.secs,
                s.sog,
                s.cog,
                s.heading,
                s.stw,
                s.awa,
                s.aws,
                s.twd,
                s.tws,
                ref_code(s.wind_ref),
                s.heel,
                s.rpm,
                s.sd_heading,
                s.sd_stw,
                s.sd_awa,
                s.n,
                s.mode.code(),
                (s.mode_from == ModeSource::Engine) as i64,
                (s.hint == Hint::LooksMotored) as i64,
                f.map(|m| m.tws),
                f.map(|m| m.twd),
                f.and_then(|m| m.gust),
                f.and_then(|m| m.hs),
                f.and_then(|m| m.wave_dir),
                f.and_then(|m| m.wave_period),
                f.and_then(|m| m.run).map(|t| t.timestamp()),
                f.and_then(|m| m.lead_h),
                h.map(|m| m.tws),
                h.map(|m| m.twd),
                h.and_then(|m| m.run).map(|t| t.timestamp()),
                h.and_then(|m| m.lead_h),
                s.polar,
                f.and_then(|m| m.current).map(|c| c.0),
                f.and_then(|m| m.current).map(|c| c.1),
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// The samples between two times (either end open), oldest first.
    pub fn perf_samples(&self, from: Option<DateTime<Utc>>, to: Option<DateTime<Utc>>) -> Result<Vec<Sample>> {
        let mut st = self.conn.prepare(&format!("SELECT {COLUMNS} FROM perf_samples WHERE t >= ?1 AND t <= ?2 ORDER BY t, id"))?;
        st.query_map(params![from.map_or(i64::MIN, |t| t.timestamp()), to.map_or(i64::MAX, |t| t.timestamp())], row)?.collect()
    }

    /// How many samples there are, and the first and last times.
    /// The samples logged against one polar, between two times (either end open), oldest first.
    pub fn perf_samples_of(&self, polar: &str, from: Option<DateTime<Utc>>, to: Option<DateTime<Utc>>) -> Result<Vec<Sample>> {
        let mut st = self.conn.prepare(&format!("SELECT {COLUMNS} FROM perf_samples WHERE polar = ?1 AND t >= ?2 AND t <= ?3 ORDER BY t, id"))?;
        st.query_map(params![polar, from.map_or(i64::MIN, |t| t.timestamp()), to.map_or(i64::MAX, |t| t.timestamp())], row)?.collect()
    }

    /// How much the log holds, for one polar or for all.
    pub fn perf_summary(&self, polar: Option<&str>) -> Result<PerfSummary> {
        let read = |r: &Row| Ok(PerfSummary { count: r.get::<_, i64>(0)? as usize, first: from_ts(r.get(1)?), last: from_ts(r.get(2)?) });
        match polar {
            Some(p) => self.conn.query_row("SELECT COUNT(*), MIN(t), MAX(t) FROM perf_samples WHERE polar = ?1", params![p], read),
            None => self.conn.query_row("SELECT COUNT(*), MIN(t), MAX(t) FROM perf_samples", [], read),
        }
    }

    /// Gives the samples logged against `from` to the polar `to`. Returns how many.
    pub fn move_perf_samples(&self, from: &str, to: &str) -> Result<usize> {
        self.conn.execute("UPDATE perf_samples SET polar = ?2 WHERE polar = ?1", params![from, to])
    }

    pub fn clear_passages(&self) -> Result<usize> {
        self.conn.execute("DELETE FROM perf_passages", [])
    }

    /// Forgets the samples logged against one polar.
    pub fn clear_perf_of(&self, polar: &str) -> Result<usize> {
        self.conn.execute("DELETE FROM perf_samples WHERE polar = ?1", params![polar])
    }

    pub fn set_perf_forecast(&self, id: i64, m: &ModelWind) -> Result<()> {
        self.conn.execute(
            "UPDATE perf_samples SET f_tws=?2, f_twd=?3, f_gust=?4, f_hs=?5, f_wave_dir=?6, f_wave_period=?7, f_run=?8, f_lead_h=?9, f_cur_e=?10, f_cur_n=?11 WHERE id=?1",
            params![id, m.tws, m.twd, m.gust, m.hs, m.wave_dir, m.wave_period, m.run.map(|t| t.timestamp()), m.lead_h, m.current.map(|c| c.0), m.current.map(|c| c.1)],
        )?;
        Ok(())
    }

    pub fn set_perf_hindcast(&self, id: i64, m: &ModelWind) -> Result<()> {
        self.conn.execute("UPDATE perf_samples SET h_tws=?2, h_twd=?3, h_run=?4, h_lead_h=?5 WHERE id=?1", params![id, m.tws, m.twd, m.run.map(|t| t.timestamp()), m.lead_h])?;
        Ok(())
    }

    /// The samples since `since` that have no hindcast yet: id, time and position.
    pub fn perf_without_hindcast(&self, since: DateTime<Utc>) -> Result<Vec<(i64, DateTime<Utc>, LatLon)>> {
        let mut st = self.conn.prepare("SELECT id, t, lat, lon FROM perf_samples WHERE h_tws IS NULL AND t >= ?1 ORDER BY t")?;
        st.query_map(params![since.timestamp()], |r| Ok((r.get(0)?, from_ts(r.get(1)?).unwrap_or_default(), LatLon::new(r.get(2)?, r.get(3)?))))?.collect()
    }

    /// How many samples from `since` and before `before` have no hindcast yet.
    pub fn perf_count_without_hindcast(&self, since: DateTime<Utc>, before: DateTime<Utc>) -> Result<usize> {
        self.conn.query_row("SELECT COUNT(*) FROM perf_samples WHERE h_tws IS NULL AND t >= ?1 AND t < ?2", params![since.timestamp(), before.timestamp()], |r| Ok(r.get::<_, i64>(0)? as usize))
    }

    /// Marks the windows between two times, not already motoring, as looking motored.
    pub fn mark_perf_motored(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<usize> {
        self.conn.execute("UPDATE perf_samples SET hint=1 WHERE t >= ?1 AND t <= ?2 AND mode != 2", params![from.timestamp(), to.timestamp()])
    }

    pub fn clear_perf(&self) -> Result<()> {
        self.conn.execute_batch("DELETE FROM perf_samples; DELETE FROM perf_passages;")
    }

    pub fn insert_passage(&self, p: &Passage) -> Result<i64> {
        let data = serde_json::to_string(p).unwrap_or_default();
        self.conn.execute("INSERT INTO perf_passages(route_id, finished, data) VALUES (?1,?2,?3)", params![p.route_id, p.finished, data])?;
        Ok(self.conn.last_insert_rowid())
    }

    /// The passages compared, newest first.
    pub fn passages(&self) -> Result<Vec<Passage>> {
        let mut st = self.conn.prepare("SELECT data FROM perf_passages ORDER BY finished DESC")?;
        let rows: Vec<String> = st.query_map([], |r| r.get(0))?.collect::<Result<_>>()?;
        Ok(rows.iter().filter_map(|d| serde_json::from_str(d).ok()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perf::synth::{self, Config};

    #[test]
    fn samples_round_trip() {
        let s = NavStore::open_memory().unwrap();
        let mut log = synth::log(&Config { hours: 0.1, ..Default::default() });
        log[3].hindcast = Some(ModelWind { tws: 11.0, twd: 200.0, gust: None, hs: None, wave_dir: None, wave_period: None, run: Some(log[3].t), lead_h: Some(1.0), current: None });
        log[4].wind_ref = Some(WindRef::Ground);
        log[4].hint = Hint::LooksMotored;
        log[5].mode_from = ModeSource::Engine;
        for (i, x) in log.iter_mut().enumerate() {
            x.polar = if i % 3 == 0 { "imoca-60".into() } else { "custom-1".into() };
            x.id = s.insert_perf_sample(x).unwrap();
        }
        let back = s.perf_samples(None, None).unwrap();
        assert_eq!(back.len(), log.len());
        // Times are kept to the second.
        for (a, b) in log.iter().zip(&back) {
            let mut a = a.clone();
            a.t = from_ts(Some(a.t.timestamp())).unwrap();
            if let Some(f) = a.forecast.as_mut() {
                f.run = from_ts(f.run.map(|t| t.timestamp()));
            }
            assert_eq!(&a, b);
        }
        let sum = s.perf_summary(None).unwrap();
        assert_eq!((sum.count, sum.first, sum.last), (log.len(), Some(back[0].t), Some(back[back.len() - 1].t)));
        // Each polar has its own samples.
        let thirds = log.len().div_ceil(3);
        assert_eq!(s.perf_summary(Some("imoca-60")).unwrap().count, thirds);
        assert_eq!(s.perf_samples_of("custom-1", None, None).unwrap().len(), log.len() - thirds);
        assert!(s.perf_samples_of("imoca-60", None, None).unwrap().iter().all(|x| x.polar == "imoca-60"));
        assert_eq!(s.perf_summary(Some("ultim")).unwrap().count, 0);
        assert_eq!(s.perf_without_hindcast(log[0].t).unwrap().len(), log.len() - 1);
        assert_eq!(s.perf_count_without_hindcast(log[0].t, log[9].t).unwrap(), 8, "the fourth has one");
        let m = ModelWind { tws: 9.0, twd: 10.0, gust: None, hs: None, wave_dir: None, wave_period: None, run: None, lead_h: None, current: None };
        s.set_perf_hindcast(back[0].id, &m).unwrap();
        s.set_perf_forecast(back[0].id, &ModelWind { hs: Some(1.5), current: Some((1.25, -0.5)), ..m }).unwrap();
        let first = &s.perf_samples(None, Some(back[0].t)).unwrap()[0];
        assert_eq!((first.hindcast.unwrap().tws, first.forecast.unwrap().hs), (9.0, Some(1.5)));
        assert_eq!(first.forecast.unwrap().current, Some((1.25, -0.5)), "the forecast current comes back");
        assert_eq!(s.mark_perf_motored(back[0].t, back[9].t).unwrap(), 10);
        assert_eq!(s.move_perf_samples("imoca-60", "ultim").unwrap(), thirds);
        assert_eq!(s.clear_perf_of("ultim").unwrap(), thirds);
        assert_eq!(s.perf_summary(None).unwrap().count, log.len() - thirds);
        s.clear_perf().unwrap();
        assert_eq!(s.perf_summary(None).unwrap().count, 0);
    }

    #[test]
    fn passages_round_trip() {
        let s = NavStore::open_memory().unwrap();
        let p = Passage { route_id: 3, name: "Out".into(), started: 10, finished: 20, planned_s: 1.0, sailed_s: 2.0, planned_sog: 5.0, sailed_sog: 4.0, distance_nm: 9.0, planned_motoring: 0.0, motored: 0.1 };
        s.insert_passage(&p).unwrap();
        s.insert_passage(&Passage { finished: 30, ..p.clone() }).unwrap();
        let back = s.passages().unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[1], p);
    }
}
