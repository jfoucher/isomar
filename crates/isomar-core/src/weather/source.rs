//! The forecast models a forecast can come from. Each is kept apart from the others (a field of
//! one never stands in for another's), shown one at a time, and routed on one at a time.
//!
//! - GFS (NOAA): 0.25°, four runs a day to 384 h, cropped on the server by the NOMADS grib filter.
//! - ECMWF IFS: 0.25°, open data since 2025 (CC BY 4.0). Runs at 00 and 12 UTC go to 360 h (3-hourly
//!   to 144 h, then 6-hourly); 06 and 18 UTC to 144 h. Global fields only, read by byte range from
//!   the index ECMWF publishes beside each file, then cropped here before they are cached.
//! - ECMWF AIFS Single: ECMWF's machine-learning model, 0.25°, four runs a day to 360 h, 6-hourly,
//!   no gusts. Published as soon as it has run, so it is the first of the three to arrive.
//! - DWD ICON-EU: the German weather service's regional model over Europe and the Mediterranean, 0.0625°
//!   (about 7 km), runs at 00, 06, 12 and 18 UTC to 120 h (hourly to 78 h, then 3-hourly). One bz2 file
//!   per field per step for the whole domain, cropped here. Regional: routing on it sails on GFS
//!   outside its area and after its end (`parent`, `routing::nested`).

use super::model::Group;
use chrono::{DateTime, Duration, Timelike, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    #[default]
    Gfs,
    Ifs,
    Aifs,
    /// Named as `id` says, in settings too.
    #[serde(rename = "iconeu")]
    IconEu,
}

impl Source {
    pub const ALL: [Source; 4] = [Source::Gfs, Source::Ifs, Source::Aifs, Source::IconEu];

    pub fn id(self) -> &'static str {
        match self {
            Source::Gfs => "gfs",
            Source::Ifs => "ifs",
            Source::Aifs => "aifs",
            Source::IconEu => "iconeu",
        }
    }

    pub fn from_id(s: &str) -> Option<Source> {
        Source::ALL.into_iter().find(|m| m.id() == s)
    }

    /// The model's name, as the user sees it (a proper name, not translated).
    pub fn label(self) -> &'static str {
        match self {
            Source::Gfs => "GFS",
            Source::Ifs => "ECMWF IFS",
            Source::Aifs => "ECMWF AIFS",
            Source::IconEu => "DWD ICON-EU",
        }
    }

    /// Who made it, for the credit line on the chart.
    pub fn credit(self) -> &'static str {
        match self {
            Source::Gfs => "NOAA GFS",
            Source::Ifs | Source::Aifs => "ECMWF (CC BY 4.0)",
            Source::IconEu => "Deutscher Wetterdienst",
        }
    }

    /// Grid spacing, degrees.
    pub fn res(self) -> f64 {
        if self == Source::IconEu { 0.0625 } else { 0.25 }
    }

    /// The model a regional one is set in: its wind outside the regional model's area and after it ends.
    pub fn parent(self) -> Option<Source> {
        (self == Source::IconEu).then_some(Source::Gfs)
    }

    /// The area a regional model covers, as south, west, north and east; `None` for a global one.
    pub fn domain(self) -> Option<(f64, f64, f64, f64)> {
        (self == Source::IconEu).then_some((29.5, -23.5, 70.5, 62.5))
    }

    /// Whether the model has anything over the box from `south`, `west` to `north`, `east`.
    pub fn covers(self, south: f64, west: f64, north: f64, east: f64) -> bool {
        match self.domain() {
            None => true,
            Some((s, w, n, e)) => south < n && north > s && west < e && east > w && west <= east,
        }
    }

    /// The server cuts out the area asked for; otherwise whole-globe fields are downloaded and
    /// cropped here.
    pub fn crops_on_server(self) -> bool {
        self == Source::Gfs
    }

    /// Hours between runs.
    fn cycle_hours(self) -> i64 {
        6
    }

    /// How long after its nominal time a run is offered: by then its first steps are out. A run asked
    /// for too early only fails over to the one before.
    fn delay(self) -> Duration {
        match self {
            Source::Gfs => Duration::minutes(270),
            // ECMWF publishes the 0.25° IFS at the end of its dissemination, about 7 h after the run
            // (the 00 UTC run's first steps around 07:00); AIFS as soon as it has run, about 5 h.
            Source::Ifs => Duration::minutes(420),
            Source::Aifs => Duration::minutes(300),
            // DWD has the whole run out within four hours.
            Source::IconEu => Duration::minutes(240),
        }
    }

    /// Runs to try, newest first.
    pub fn candidate_runs(self, now: DateTime<Utc>) -> Vec<DateTime<Utc>> {
        let latest = now - self.delay();
        let c = self.cycle_hours();
        let newest = latest.with_minute(0).and_then(|t| t.with_second(0)).and_then(|t| t.with_nanosecond(0)).expect("a whole hour exists");
        let newest = newest - Duration::hours(newest.hour() as i64 % c);
        (0..4).map(|i| newest - Duration::hours(c * i)).collect()
    }

    /// Every forecast hour the run publishes, in order.
    pub fn published_hours(self, run: DateTime<Utc>) -> Vec<u32> {
        let main = run.hour().is_multiple_of(12);
        match self {
            Source::Gfs => (0..=120).chain((123..=384).step_by(3)).collect(),
            Source::Ifs if main => (0..=144).step_by(3).chain((150..=360).step_by(6)).collect(),
            Source::Ifs => (0..=144).step_by(3).collect(),
            Source::Aifs => (0..=360).step_by(6).collect(),
            Source::IconEu => (0..=78).chain((81..=120).step_by(3)).collect(),
        }
    }

    /// The longest forecast any run goes to.
    pub fn max_hours(self) -> u32 {
        match self {
            Source::Gfs => 384,
            Source::Ifs | Source::Aifs => 360,
            Source::IconEu => 120,
        }
    }

    /// The groups of parameters the model has.
    pub fn groups(self) -> &'static [Group] {
        match self {
            Source::Gfs => &[Group::Wind, Group::Gust, Group::Pressure, Group::Waves, Group::Temperature, Group::Cloud],
            Source::Ifs => &[Group::Wind, Group::Gust, Group::Pressure, Group::Waves, Group::Temperature],
            Source::Aifs => &[Group::Wind, Group::Pressure, Group::Waves, Group::Temperature],
            Source::IconEu => &[Group::Wind, Group::Gust, Group::Pressure, Group::Temperature, Group::Cloud],
        }
    }

    pub fn has(self, g: Group) -> bool {
        self.groups().contains(&g)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(d: u32, h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, d, h, m, 0).unwrap()
    }

    #[test]
    fn ids_roundtrip() {
        for s in Source::ALL {
            assert_eq!(Source::from_id(s.id()), Some(s));
        }
        assert_eq!(Source::from_id("icon"), None);
        // Settings name them by their ids.
        for s in Source::ALL {
            assert_eq!(serde_json::to_string(&s).unwrap(), format!("\"{}\"", s.id()));
        }
    }

    #[test]
    fn each_model_offers_its_runs_after_its_own_delay() {
        // 11:00 UTC: GFS 06 is 5 h old (offered), IFS 06 is not (7 h), AIFS 06 is 5 h old.
        assert_eq!(Source::Gfs.candidate_runs(t(5, 11, 0))[0], t(5, 6, 0));
        assert_eq!(Source::Ifs.candidate_runs(t(5, 11, 0))[0], t(5, 0, 0));
        assert_eq!(Source::Aifs.candidate_runs(t(5, 11, 0))[0], t(5, 6, 0));
        // Across midnight, and four runs six hours apart.
        let r = Source::Ifs.candidate_runs(t(5, 3, 0));
        assert_eq!(r, vec![t(4, 18, 0), t(4, 12, 0), t(4, 6, 0), t(4, 0, 0)]);
    }

    #[test]
    fn a_regional_model_covers_its_domain_and_sits_in_a_global_one() {
        assert!(Source::IconEu.covers(43.0, -12.0, 52.0, 0.0), "Biscay");
        assert!(!Source::IconEu.covers(-45.0, 165.0, -30.0, 180.0), "New Zealand");
        assert!(Source::Gfs.covers(-45.0, 165.0, -30.0, 180.0));
        assert_eq!(Source::IconEu.parent(), Some(Source::Gfs));
        assert_eq!(Source::Ifs.parent(), None);
    }

    #[test]
    fn the_published_steps_match_what_the_servers_list() {
        // Counted on data.ecmwf.int for 2026-11-05: IFS 00z 85 files, 06z 49, AIFS 61.
        assert_eq!(Source::Ifs.published_hours(t(5, 0, 0)).len(), 85);
        assert_eq!(Source::Ifs.published_hours(t(5, 6, 0)).len(), 49);
        assert_eq!(*Source::Ifs.published_hours(t(5, 6, 0)).last().unwrap(), 144);
        assert_eq!(Source::Aifs.published_hours(t(5, 18, 0)).len(), 61);
        // DWD lists 93 files a field for its long runs.
        assert_eq!(Source::IconEu.published_hours(t(5, 6, 0)).len(), 93);
        let gfs = Source::Gfs.published_hours(t(5, 0, 0));
        assert_eq!((gfs[120], gfs[121], *gfs.last().unwrap()), (120, 123, 384));
    }
}
