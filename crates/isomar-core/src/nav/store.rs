//! Persistent storage of waypoints, routes and tracks in SQLite.

use super::gpx::GpxDoc;
use super::model::*;
use crate::geo::LatLon;
use crate::routing::plan::PlanTimes;
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

pub type Result<T> = std::result::Result<T, rusqlite::Error>;

const SCHEMA_VERSION: i64 = 7;

pub struct NavStore {
    pub(crate) conn: Connection,
}

fn ts(t: &Option<DateTime<Utc>>) -> Option<i64> {
    t.map(|t| t.timestamp())
}

pub(super) fn from_ts(v: Option<i64>) -> Option<DateTime<Utc>> {
    v.and_then(|s| Utc.timestamp_opt(s, 0).single())
}

impl NavStore {
    pub fn open(path: &Path) -> Result<NavStore> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Self::init(Connection::open(path)?)
    }

    pub fn open_memory() -> Result<NavStore> {
        Self::init(Connection::open_in_memory()?)
    }

    /// The file the store is in; `None` for one in memory.
    pub fn path(&self) -> Option<std::path::PathBuf> {
        self.conn.path().filter(|p| !p.is_empty()).map(std::path::PathBuf::from)
    }

    /// The same database for reading only, from another thread (the store is in WAL mode, so reading does
    /// not wait for the writer). No schema change is made: the main connection did that.
    pub fn open_readonly(path: &Path) -> Result<NavStore> {
        use rusqlite::OpenFlags;
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        Ok(NavStore { conn })
    }

    fn init(conn: Connection) -> Result<NavStore> {
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if v < 1 {
            conn.execute_batch(
                "CREATE TABLE waypoints(
                    id INTEGER PRIMARY KEY, name TEXT NOT NULL, lat REAL NOT NULL, lon REAL NOT NULL,
                    icon TEXT NOT NULL, description TEXT NOT NULL, arrival_radius REAL NOT NULL,
                    visible INTEGER NOT NULL, time INTEGER);
                 CREATE TABLE routes(
                    id INTEGER PRIMARY KEY, name TEXT NOT NULL, speed REAL NOT NULL,
                    departure INTEGER, leg_kind TEXT NOT NULL, visible INTEGER NOT NULL);
                 CREATE TABLE route_points(
                    route_id INTEGER NOT NULL REFERENCES routes(id) ON DELETE CASCADE,
                    seq INTEGER NOT NULL, name TEXT NOT NULL, lat REAL NOT NULL, lon REAL NOT NULL,
                    arrival_radius REAL NOT NULL, PRIMARY KEY(route_id, seq));
                 CREATE TABLE tracks(
                    id INTEGER PRIMARY KEY, name TEXT NOT NULL, visible INTEGER NOT NULL);
                 CREATE TABLE track_points(
                    track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
                    seq INTEGER NOT NULL, lat REAL NOT NULL, lon REAL NOT NULL, time INTEGER,
                    PRIMARY KEY(track_id, seq));",
            )?;
        }
        if v < 2 {
            conn.execute_batch("ALTER TABLE routes DROP COLUMN leg_kind;")?;
        }
        if v < 3 {
            // What a weather route was planned with (`routing::plan`), as one opaque blob.
            conn.execute_batch(
                "CREATE TABLE route_plans(
                    route_id INTEGER PRIMARY KEY REFERENCES routes(id) ON DELETE CASCADE,
                    data BLOB NOT NULL);",
            )?;
        }
        if v < 4 {
            // The performance log (`perf`): one row per 10 s window, the forecast's wind and waves at it,
            // a later run's analysis wind (the hindcast); and passages planned against sailed.
            conn.execute_batch(
                "CREATE TABLE perf_samples(
                    id INTEGER PRIMARY KEY, t INTEGER NOT NULL, lat REAL NOT NULL, lon REAL NOT NULL,
                    secs REAL NOT NULL, sog REAL NOT NULL, cog REAL NOT NULL, heading REAL, stw REAL,
                    awa REAL, aws REAL, twd REAL, tws REAL, wind_ref INTEGER, heel REAL, rpm REAL,
                    sd_heading REAL, sd_stw REAL, sd_awa REAL, n INTEGER NOT NULL,
                    mode INTEGER NOT NULL, mode_from INTEGER NOT NULL, hint INTEGER NOT NULL,
                    f_tws REAL, f_twd REAL, f_gust REAL, f_hs REAL, f_wave_dir REAL, f_wave_period REAL,
                    f_run INTEGER, f_lead_h REAL,
                    h_tws REAL, h_twd REAL, h_run INTEGER, h_lead_h REAL);
                 CREATE INDEX perf_samples_t ON perf_samples(t);
                 CREATE TABLE perf_passages(
                    id INTEGER PRIMARY KEY, route_id INTEGER NOT NULL, finished INTEGER NOT NULL, data TEXT NOT NULL);",
            )?;
        }
        if v < 5 {
            // The polar each sample was logged against (`routing::library`): the samples of before are
            // given the active one by the app at its first start with the library.
            conn.execute_batch(
                "ALTER TABLE perf_samples ADD COLUMN polar TEXT NOT NULL DEFAULT '';
                 CREATE INDEX perf_samples_polar ON perf_samples(polar, t);",
            )?;
        }
        if v < 6 {
            // A plan's departure, end and arrival beside its blob, for the list of routes. Plans saved
            // before have none: `Session::new` reads them once and fills them in.
            conn.execute_batch(
                "ALTER TABLE route_plans ADD COLUMN depart INTEGER;
                 ALTER TABLE route_plans ADD COLUMN arrive INTEGER;
                 ALTER TABLE route_plans ADD COLUMN arrives INTEGER;",
            )?;
        }
        if v < 7 {
            // The forecast current at each window (`perf::ModelWind::current`), for tier C's correction.
            conn.execute_batch(
                "ALTER TABLE perf_samples ADD COLUMN f_cur_e REAL;
                 ALTER TABLE perf_samples ADD COLUMN f_cur_n REAL;",
            )?;
        }
        if v < SCHEMA_VERSION {
            conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))?;
        }
        Ok(NavStore { conn })
    }

    // Waypoints

    pub fn waypoints(&self) -> Result<Vec<Waypoint>> {
        let mut st = self.conn.prepare(
            "SELECT id, name, lat, lon, icon, description, arrival_radius, visible, time FROM waypoints ORDER BY id",
        )?;
        let rows = st.query_map([], |r| {
            Ok(Waypoint {
                id: r.get(0)?,
                name: r.get(1)?,
                pos: LatLon::new(r.get(2)?, r.get(3)?),
                icon: r.get(4)?,
                description: r.get(5)?,
                arrival_radius_nm: r.get(6)?,
                visible: r.get::<_, i64>(7)? != 0,
                time: from_ts(r.get(8)?),
            })
        })?;
        rows.collect()
    }

    pub fn insert_waypoint(&self, w: &Waypoint) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO waypoints(name, lat, lon, icon, description, arrival_radius, visible, time)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![w.name, w.pos.lat, w.pos.lon, w.icon, w.description, w.arrival_radius_nm, w.visible as i64, ts(&w.time)],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_waypoint(&self, w: &Waypoint) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE waypoints SET name=?2, lat=?3, lon=?4, icon=?5, description=?6, arrival_radius=?7, visible=?8, time=?9
             WHERE id=?1",
            params![w.id, w.name, w.pos.lat, w.pos.lon, w.icon, w.description, w.arrival_radius_nm, w.visible as i64, ts(&w.time)],
        )?;
        Ok(n > 0)
    }

    pub fn delete_waypoint(&self, id: i64) -> Result<bool> {
        Ok(self.conn.execute("DELETE FROM waypoints WHERE id=?1", params![id])? > 0)
    }

    // Routes

    pub fn routes(&self) -> Result<Vec<Route>> {
        let mut st = self.conn.prepare("SELECT id, name, speed, departure, visible FROM routes ORDER BY id")?;
        let mut routes: Vec<Route> = st
            .query_map([], |r| {
                Ok(Route {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    points: Vec::new(),
                    speed_kn: r.get(2)?,
                    departure: from_ts(r.get(3)?),
                    visible: r.get::<_, i64>(4)? != 0,
                })
            })?
            .collect::<Result<_>>()?;
        let mut pt = self.conn.prepare(
            "SELECT name, lat, lon, arrival_radius FROM route_points WHERE route_id=?1 ORDER BY seq",
        )?;
        for r in routes.iter_mut() {
            r.points = pt
                .query_map(params![r.id], |row| {
                    Ok(RoutePoint { name: row.get(0)?, pos: LatLon::new(row.get(1)?, row.get(2)?), arrival_radius_nm: row.get(3)? })
                })?
                .collect::<Result<_>>()?;
        }
        Ok(routes)
    }

    fn write_route_points(&self, id: i64, points: &[RoutePoint]) -> Result<()> {
        self.conn.execute("DELETE FROM route_points WHERE route_id=?1", params![id])?;
        let mut ins = self.conn.prepare(
            "INSERT INTO route_points(route_id, seq, name, lat, lon, arrival_radius) VALUES (?1,?2,?3,?4,?5,?6)",
        )?;
        for (i, p) in points.iter().enumerate() {
            ins.execute(params![id, i as i64, p.name, p.pos.lat, p.pos.lon, p.arrival_radius_nm])?;
        }
        Ok(())
    }

    pub fn insert_route(&self, r: &Route) -> Result<i64> {
        let tx = self.conn.unchecked_transaction()?;
        self.conn.execute(
            "INSERT INTO routes(name, speed, departure, visible) VALUES (?1,?2,?3,?4)",
            params![r.name, r.speed_kn, ts(&r.departure), r.visible as i64],
        )?;
        let id = self.conn.last_insert_rowid();
        self.write_route_points(id, &r.points)?;
        tx.commit()?;
        Ok(id)
    }

    pub fn update_route(&self, r: &Route) -> Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        let n = self.conn.execute(
            "UPDATE routes SET name=?2, speed=?3, departure=?4, visible=?5 WHERE id=?1",
            params![r.id, r.name, r.speed_kn, ts(&r.departure), r.visible as i64],
        )?;
        if n > 0 {
            self.write_route_points(r.id, &r.points)?;
        }
        tx.commit()?;
        Ok(n > 0)
    }

    pub fn delete_route(&self, id: i64) -> Result<bool> {
        Ok(self.conn.execute("DELETE FROM routes WHERE id=?1", params![id])? > 0)
    }

    // Route plans

    pub fn save_plan(&self, route_id: i64, data: &[u8], times: &PlanTimes) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO route_plans(route_id, data, depart, arrive, arrives) VALUES (?1,?2,?3,?4,?5)",
            params![route_id, data, times.depart.timestamp(), times.end.timestamp(), times.arrives as i64],
        )?;
        Ok(())
    }

    /// Fills in the times of a plan saved before they were kept.
    pub fn set_plan_times(&self, route_id: i64, times: &PlanTimes) -> Result<()> {
        self.conn.execute(
            "UPDATE route_plans SET depart=?2, arrive=?3, arrives=?4 WHERE route_id=?1",
            params![route_id, times.depart.timestamp(), times.end.timestamp(), times.arrives as i64],
        )?;
        Ok(())
    }

    pub fn plan(&self, route_id: i64) -> Result<Option<Vec<u8>>> {
        self.conn.query_row("SELECT data FROM route_plans WHERE route_id=?1", params![route_id], |r| r.get(0)).optional()
    }

    /// The routes that have a plan.
    pub fn plan_ids(&self) -> Result<Vec<i64>> {
        let mut st = self.conn.prepare("SELECT route_id FROM route_plans")?;
        st.query_map([], |r| r.get(0))?.collect()
    }

    /// The routes that have a plan, with its times (None for a plan saved before they were kept).
    pub fn plan_times(&self) -> Result<Vec<(i64, Option<PlanTimes>)>> {
        let mut st = self.conn.prepare("SELECT route_id, depart, arrive, arrives FROM route_plans")?;
        st.query_map([], |r| {
            let times = match (from_ts(r.get(1)?), from_ts(r.get(2)?), r.get::<_, Option<i64>>(3)?) {
                (Some(depart), Some(end), Some(arrives)) => Some(PlanTimes { depart, end, arrives: arrives != 0 }),
                _ => None,
            };
            Ok((r.get(0)?, times))
        })?
        .collect()
    }

    // Tracks

    pub fn tracks(&self) -> Result<Vec<Track>> {
        let mut st = self.conn.prepare("SELECT id, name, visible FROM tracks ORDER BY id")?;
        let mut tracks: Vec<Track> = st
            .query_map([], |r| Ok(Track { id: r.get(0)?, name: r.get(1)?, points: Vec::new(), visible: r.get::<_, i64>(2)? != 0 }))?
            .collect::<Result<_>>()?;
        let mut pt = self.conn.prepare("SELECT lat, lon, time FROM track_points WHERE track_id=?1 ORDER BY seq")?;
        for t in tracks.iter_mut() {
            t.points = pt
                .query_map(params![t.id], |r| Ok(TrackPoint { pos: LatLon::new(r.get(0)?, r.get(1)?), time: from_ts(r.get(2)?) }))?
                .collect::<Result<_>>()?;
        }
        Ok(tracks)
    }

    pub fn insert_track(&self, t: &Track) -> Result<i64> {
        let tx = self.conn.unchecked_transaction()?;
        self.conn.execute("INSERT INTO tracks(name, visible) VALUES (?1,?2)", params![t.name, t.visible as i64])?;
        let id = self.conn.last_insert_rowid();
        let mut ins = self.conn.prepare("INSERT INTO track_points(track_id, seq, lat, lon, time) VALUES (?1,?2,?3,?4,?5)")?;
        for (i, p) in t.points.iter().enumerate() {
            ins.execute(params![id, i as i64, p.pos.lat, p.pos.lon, ts(&p.time)])?;
        }
        drop(ins);
        tx.commit()?;
        Ok(id)
    }

    /// Adds a point to the end of a track without rewriting it.
    pub fn append_track_point(&self, track_id: i64, p: &TrackPoint) -> Result<()> {
        let next: i64 = self
            .conn
            .query_row("SELECT COALESCE(MAX(seq) + 1, 0) FROM track_points WHERE track_id=?1", params![track_id], |r| r.get(0))?;
        self.conn.execute(
            "INSERT INTO track_points(track_id, seq, lat, lon, time) VALUES (?1,?2,?3,?4,?5)",
            params![track_id, next, p.pos.lat, p.pos.lon, ts(&p.time)],
        )?;
        Ok(())
    }

    pub fn update_track_meta(&self, t: &Track) -> Result<bool> {
        Ok(self.conn.execute("UPDATE tracks SET name=?2, visible=?3 WHERE id=?1", params![t.id, t.name, t.visible as i64])? > 0)
    }

    pub fn delete_track(&self, id: i64) -> Result<bool> {
        Ok(self.conn.execute("DELETE FROM tracks WHERE id=?1", params![id])? > 0)
    }

    pub fn track_exists(&self, id: i64) -> Result<bool> {
        Ok(self.conn.query_row("SELECT 1 FROM tracks WHERE id=?1", params![id], |_| Ok(())).optional()?.is_some())
    }

    // GPX

    /// Adds everything in `doc`; returns the numbers of waypoints, routes and
    /// tracks stored.
    pub fn import(&self, doc: &GpxDoc) -> Result<(usize, usize, usize)> {
        for w in &doc.waypoints {
            self.insert_waypoint(w)?;
        }
        for r in &doc.routes {
            self.insert_route(r)?;
        }
        for t in &doc.tracks {
            self.insert_track(t)?;
        }
        Ok((doc.waypoints.len(), doc.routes.len(), doc.tracks.len()))
    }

    pub fn export_all(&self) -> Result<GpxDoc> {
        Ok(GpxDoc { waypoints: self.waypoints()?, routes: self.routes()?, tracks: self.tracks()?, warnings: vec![] })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nav::gpx;

    fn wp(name: &str) -> Waypoint {
        let mut w = Waypoint::new(name, LatLon::new(48.5, -4.5));
        w.description = "d".into();
        w.time = Some(Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap());
        w
    }

    fn route(name: &str, n: usize) -> Route {
        let mut r = Route::new(name);
        r.departure = Some(Utc.with_ymd_and_hms(2026, 7, 1, 6, 0, 0).unwrap());
        for i in 0..n {
            r.points.push(RoutePoint::new(format!("p{i}"), LatLon::new(48.0 + i as f64 * 0.1, -4.0)));
        }
        r
    }

    #[test]
    fn waypoint_crud() {
        let s = NavStore::open_memory().unwrap();
        let id = s.insert_waypoint(&wp("a")).unwrap();
        let id2 = s.insert_waypoint(&wp("b")).unwrap();
        assert_ne!(id, id2);
        let mut w = s.waypoints().unwrap().remove(0);
        assert_eq!((w.id, w.name.as_str()), (id, "a"));
        assert_eq!(w.time, wp("a").time);
        w.name = "renamed".into();
        w.visible = false;
        w.pos = LatLon::new(1.0, 2.0);
        assert!(s.update_waypoint(&w).unwrap());
        let back = &s.waypoints().unwrap()[0];
        assert_eq!((back.name.as_str(), back.visible, back.pos), ("renamed", false, LatLon::new(1.0, 2.0)));
        assert!(s.delete_waypoint(id).unwrap());
        assert!(!s.delete_waypoint(id).unwrap());
        assert_eq!(s.waypoints().unwrap().len(), 1);
        let ghost = Waypoint { id: 9999, ..wp("ghost") };
        assert!(!s.update_waypoint(&ghost).unwrap());
    }

    #[test]
    fn route_roundtrip_update_and_cascade() {
        let s = NavStore::open_memory().unwrap();
        let id = s.insert_route(&route("r1", 4)).unwrap();
        let mut r = s.routes().unwrap().remove(0);
        assert_eq!(r.points.len(), 4);
        assert_eq!(r.points[2].name, "p2");
        assert_eq!(r.departure, route("x", 0).departure);
        r.points.remove(1);
        r.name = "changed".into();
        assert!(s.update_route(&r).unwrap());
        let back = s.routes().unwrap().remove(0);
        assert_eq!((back.points.len(), back.name.as_str()), (3, "changed"));
        assert_eq!(back.points[1].name, "p2");
        assert!(s.delete_route(id).unwrap());
        // Points go with their route.
        let n: i64 = s.conn.query_row("SELECT COUNT(*) FROM route_points", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
        assert!(!s.update_route(&Route { id: 77, ..route("g", 2) }).unwrap());
    }

    #[test]
    fn track_append_and_delete() {
        let s = NavStore::open_memory().unwrap();
        let mut t = Track::new("t");
        t.points.push(TrackPoint { pos: LatLon::new(1.0, 1.0), time: None });
        let id = s.insert_track(&t).unwrap();
        for i in 0..3 {
            s.append_track_point(id, &TrackPoint { pos: LatLon::new(2.0 + i as f64, 1.0), time: Some(Utc.timestamp_opt(1_700_000_000 + i, 0).unwrap()) }).unwrap();
        }
        let back = s.tracks().unwrap().remove(0);
        assert_eq!(back.points.len(), 4);
        assert_eq!(back.points[3].pos.lat, 4.0);
        assert!(back.points[0].time.is_none() && back.points[1].time.is_some());
        let mut meta = back.clone();
        meta.name = "n".into();
        meta.visible = false;
        assert!(s.update_track_meta(&meta).unwrap());
        assert!(!s.tracks().unwrap()[0].visible);
        assert!(s.track_exists(id).unwrap());
        assert!(s.delete_track(id).unwrap());
        assert!(!s.track_exists(id).unwrap());
    }

    #[test]
    fn a_version_1_store_loses_its_leg_kind_column_and_keeps_its_routes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nav.sqlite");
        {
            let c = Connection::open(&p).unwrap();
            c.execute_batch(
                "CREATE TABLE waypoints(
                    id INTEGER PRIMARY KEY, name TEXT NOT NULL, lat REAL NOT NULL, lon REAL NOT NULL,
                    icon TEXT NOT NULL, description TEXT NOT NULL, arrival_radius REAL NOT NULL,
                    visible INTEGER NOT NULL, time INTEGER);
                 CREATE TABLE routes(
                    id INTEGER PRIMARY KEY, name TEXT NOT NULL, speed REAL NOT NULL,
                    departure INTEGER, leg_kind TEXT NOT NULL, visible INTEGER NOT NULL);
                 CREATE TABLE route_points(
                    route_id INTEGER NOT NULL REFERENCES routes(id) ON DELETE CASCADE,
                    seq INTEGER NOT NULL, name TEXT NOT NULL, lat REAL NOT NULL, lon REAL NOT NULL,
                    arrival_radius REAL NOT NULL, PRIMARY KEY(route_id, seq));
                 CREATE TABLE tracks(
                    id INTEGER PRIMARY KEY, name TEXT NOT NULL, visible INTEGER NOT NULL);
                 CREATE TABLE track_points(
                    track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
                    seq INTEGER NOT NULL, lat REAL NOT NULL, lon REAL NOT NULL, time INTEGER,
                    PRIMARY KEY(track_id, seq));
                 INSERT INTO routes VALUES (1, 'old', 5.0, NULL, 'gc', 1);
                 PRAGMA user_version = 1;",
            )
            .unwrap();
        }
        let s = NavStore::open(&p).unwrap();
        assert_eq!(s.routes().unwrap()[0].name, "old");
        s.insert_route(&route("new", 2)).unwrap();
        assert_eq!(s.routes().unwrap().len(), 2);
    }

    #[test]
    fn data_survives_reopening_and_schema_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nav.sqlite");
        {
            let s = NavStore::open(&p).unwrap();
            s.insert_waypoint(&wp("keep")).unwrap();
            s.insert_route(&route("r", 3)).unwrap();
        }
        let s = NavStore::open(&p).unwrap();
        assert_eq!(s.waypoints().unwrap()[0].name, "keep");
        assert_eq!(s.routes().unwrap()[0].points.len(), 3);
        let v: i64 = s.conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, SCHEMA_VERSION);
    }

    #[test]
    fn gpx_import_and_export_through_the_store() {
        let s = NavStore::open_memory().unwrap();
        s.insert_waypoint(&wp("w")).unwrap();
        s.insert_route(&route("r", 2)).unwrap();
        let xml = gpx::write(&s.export_all().unwrap());
        let s2 = NavStore::open_memory().unwrap();
        let counts = s2.import(&gpx::parse(&xml).unwrap()).unwrap();
        assert_eq!(counts, (1, 1, 0));
        assert_eq!(s2.routes().unwrap()[0].points.len(), 2);
        assert_eq!(s2.waypoints().unwrap()[0].name, "w");
    }

    #[test]
    fn empty_store_is_empty() {
        let s = NavStore::open_memory().unwrap();
        assert!(s.waypoints().unwrap().is_empty() && s.routes().unwrap().is_empty() && s.tracks().unwrap().is_empty());
    }
}
