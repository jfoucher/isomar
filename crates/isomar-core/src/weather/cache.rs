//! Where downloaded weather files live, and finding them again.

use super::grib;
use super::planner::{Job, Model};
use super::source::Source;
use super::store::WeatherStore;
use chrono::{DateTime, Duration, Utc};
use std::path::{Path, PathBuf};

/// `<root>/weather/<model>/<run YYYYMMDDHH>/<file>`
pub struct Cache {
    root: PathBuf,
}

/// The model a cached file belongs to, from its folder: `<model>/<run>/<file>`.
fn model_of(p: &Path) -> Option<Model> {
    Model::from_id(p.parent()?.parent()?.file_name()?.to_str()?)
}

impl Cache {
    pub fn new(root: &Path) -> Cache {
        Cache { root: root.join("weather") }
    }

    pub fn job_path(&self, j: &Job) -> PathBuf {
        self.root.join(j.model.id()).join(j.run.format("%Y%m%d%H").to_string()).join(&j.cache_name)
    }

    /// Every cached GRIB file, with the run it belongs to, oldest run first.
    pub fn files(&self) -> Vec<(DateTime<Utc>, PathBuf)> {
        let mut out = Vec::new();
        let Ok(models) = std::fs::read_dir(&self.root) else { return out };
        for m in models.flatten() {
            let Ok(runs) = std::fs::read_dir(m.path()) else { continue };
            for r in runs.flatten() {
                let name = r.file_name().to_string_lossy().into_owned();
                let Ok(naive) = chrono::NaiveDateTime::parse_from_str(&format!("{name}0000"), "%Y%m%d%H%M%S") else { continue };
                let run = naive.and_utc();
                let Ok(files) = std::fs::read_dir(r.path()) else { continue };
                for f in files.flatten() {
                    let p = f.path();
                    if p.extension().is_some_and(|e| e == "grib2") {
                        out.push((run, p));
                    }
                }
            }
        }
        out.sort();
        out
    }

    pub fn size_bytes(&self) -> u64 {
        self.files().iter().filter_map(|(_, p)| std::fs::metadata(p).ok()).map(|m| m.len()).sum()
    }

    /// Reads the cached GFS files into a store: `load_source` for GFS.
    pub fn load(&self, now: DateTime<Utc>, max_age: Duration) -> (WeatherStore, usize) {
        self.load_source(Source::Gfs, now, max_age)
    }

    /// Reads the cached files of one forecast model into a store, older runs first so newer ones win.
    /// Files older than `max_age` (by run time) are ignored; unreadable files are skipped. Returns the
    /// store and the number of files read.
    pub fn load_source(&self, source: Source, now: DateTime<Utc>, max_age: Duration) -> (WeatherStore, usize) {
        let mut store = WeatherStore::new();
        let mut n = 0;
        for (run, p) in self.files() {
            // The ensemble's members are not a forecast of their own.
            if now - run > max_age || model_of(&p).filter(|m| *m != Model::Gefs).map(Model::source) != Some(source) {
                continue;
            }
            let Ok(bytes) = std::fs::read(&p) else { continue };
            if let Ok(fields) = grib::read_fields(&bytes) {
                store.extend(fields);
                n += 1;
            }
        }
        (store, n)
    }

    /// Deletes all but the newest `keep` runs per model, and empty folders.
    /// Returns the number of files removed.
    pub fn prune(&self, keep: usize) -> usize {
        let mut removed = 0;
        let Ok(models) = std::fs::read_dir(&self.root) else { return 0 };
        for m in models.flatten() {
            let Ok(runs) = std::fs::read_dir(m.path()) else { continue };
            let mut dirs: Vec<PathBuf> = runs.flatten().map(|r| r.path()).filter(|p| p.is_dir()).collect();
            dirs.sort();
            let drop = dirs.len().saturating_sub(keep);
            for d in dirs.into_iter().take(drop) {
                removed += std::fs::read_dir(&d).map(|rd| rd.flatten().count()).unwrap_or(0);
                let _ = std::fs::remove_dir_all(&d);
            }
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::weather::model::Group;
    use crate::weather::planner::{Area, Request, plan};
    use crate::weather::testgrib::{TestField, encode};
    use chrono::TimeZone;

    fn write_run(cache: &Cache, run: DateTime<Utc>, hours: u32, value: f32) {
        let req = Request { source: Source::Gfs, area: Area::new(43.0, -12.0, 52.0, 0.0), groups: vec![Group::Wind], horizon_hours: 0, step_hours: 3 };
        let p = plan(&req, run, run + Duration::hours(hours as i64));
        let j = &p.jobs[0];
        let path = cache.job_path(j);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut f = TestField::wind_u(hours, 3, 3, vec![value; 9]);
        f.run = (run.format("%Y").to_string().parse().unwrap(), 10, 26, run.format("%H").to_string().parse().unwrap());
        std::fs::write(&path, encode(&f)).unwrap();
    }

    fn run_at(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, day, hour, 0, 0).unwrap()
    }

    #[test]
    fn paths_are_organised_by_model_and_run() {
        let d = tempfile::tempdir().unwrap();
        let c = Cache::new(d.path());
        let req = Request { source: Source::Gfs, area: Area::new(43.0, -12.0, 52.0, 0.0), groups: vec![Group::Wind], horizon_hours: 6, step_hours: 3 };
        let p = plan(&req, run_at(26, 0), run_at(26, 10));
        let path = c.job_path(&p.jobs[0]);
        let rel = path.strip_prefix(d.path()).unwrap().to_string_lossy().into_owned();
        assert!(rel.starts_with("weather/gfs/2026102600/gfs_2026102600_f009_"), "{rel}");
    }

    #[test]
    fn files_are_listed_oldest_run_first_and_sized() {
        let d = tempfile::tempdir().unwrap();
        let c = Cache::new(d.path());
        assert!(c.files().is_empty() && c.size_bytes() == 0);
        write_run(&c, run_at(26, 6), 3, 1.0);
        write_run(&c, run_at(26, 0), 3, 2.0);
        std::fs::write(c.root.join("gfs").join("README.txt"), "x").unwrap();
        let f = c.files();
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].0, run_at(26, 0));
        assert!(c.size_bytes() > 100);
    }

    #[test]
    fn newer_runs_override_older_ones_when_loaded_and_old_runs_are_ignored() {
        let d = tempfile::tempdir().unwrap();
        let c = Cache::new(d.path());
        // Same valid time (06Z + 3h = 00Z + 9h): the newer run says 5, the older 2.
        write_run(&c, run_at(26, 0), 9, 2.0);
        write_run(&c, run_at(26, 6), 3, 5.0);
        let now = run_at(26, 12);
        let (store, n) = c.load(now, Duration::hours(48));
        assert_eq!(n, 2);
        let v = store.sample(crate::weather::model::Param::WindU, 47.9, -9.8, run_at(26, 9)).unwrap();
        assert!((v - 5.0).abs() < 0.02, "{v}");
        let (stale, n) = c.load(now + Duration::days(10), Duration::hours(48));
        assert!(stale.is_empty() && n == 0);
    }

    #[test]
    fn each_model_loads_alone() {
        let d = tempfile::tempdir().unwrap();
        let c = Cache::new(d.path());
        write_run(&c, run_at(26, 0), 3, 2.0);
        let ifs = c.root.join("ifs").join("2026102600");
        std::fs::create_dir_all(&ifs).unwrap();
        let mut f = TestField::wind_u(3, 3, 3, vec![7.0; 9]);
        f.run = (2026, 10, 26, 0);
        std::fs::write(ifs.join("ifs_2026102600_f003_0.grib2"), encode(&f)).unwrap();
        let now = run_at(26, 12);
        let at = |s: &WeatherStore| s.sample(crate::weather::model::Param::WindU, 47.9, -9.8, run_at(26, 3)).unwrap();
        let (gfs, n) = c.load(now, Duration::hours(48));
        assert!(n == 1 && (at(&gfs) - 2.0).abs() < 0.02);
        let (ifs, n) = c.load_source(Source::Ifs, now, Duration::hours(48));
        assert!(n == 1 && (at(&ifs) - 7.0).abs() < 0.02);
        assert!(c.load_source(Source::Aifs, now, Duration::hours(48)).0.is_empty());
    }

    #[test]
    fn corrupt_files_are_skipped() {
        let d = tempfile::tempdir().unwrap();
        let c = Cache::new(d.path());
        write_run(&c, run_at(26, 0), 3, 1.0);
        let junk = c.root.join("gfs").join("2026102606");
        std::fs::create_dir_all(&junk).unwrap();
        std::fs::write(junk.join("bad.grib2"), b"not grib at all").unwrap();
        let (store, n) = c.load(run_at(26, 12), Duration::hours(48));
        assert_eq!(n, 1);
        assert!(!store.is_empty());
    }

    #[test]
    fn pruning_keeps_the_newest_runs() {
        let d = tempfile::tempdir().unwrap();
        let c = Cache::new(d.path());
        for h in [0, 6, 12, 18] {
            write_run(&c, run_at(26, h), 3, 1.0);
        }
        assert_eq!(c.prune(2), 2);
        let runs: Vec<_> = c.files().into_iter().map(|(r, _)| r).collect();
        assert_eq!(runs, vec![run_at(26, 12), run_at(26, 18)]);
        assert_eq!(c.prune(5), 0);
        assert_eq!(Cache::new(&d.path().join("nowhere")).prune(1), 0);
    }
}
