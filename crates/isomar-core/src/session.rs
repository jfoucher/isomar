//! Everything that changes while the boat is under way, in one place and
//! without any GUI: stored navigation objects, own-ship, AIS targets, the
//! active route, the anchor watch and track recording.
//!
//! The UI feeds it events and the current time and shows what comes out.

use crate::alerts::{Alerts, Level, Source};
use crate::comm::ais::Assembler;
use crate::comm::cpa::{AisAlarms, AisTargets, AlarmConfig, AlarmEvent, Motion};
use crate::comm::nmea::{self, Nmea};
use crate::comm::source::{Event, Payload, Status};
use crate::comm::state::OwnShip;
use crate::geo::LatLon;
use crate::nav::alarm::{AnchorStatus, AnchorWatch};
use crate::nav::calc::{ActiveRoute, NavData};
use crate::nav::model::*;
use crate::nav::store::NavStore;
use crate::routing::plan::{PlanTimes, RoutePlan};
use chrono::{DateTime, Duration, Utc};
use std::collections::{HashMap, VecDeque};

const MONITOR_LINES: usize = 500;
/// The source of the phone's or tablet's own location service (`isomar_location`).
pub const DEVICE_SOURCE: &str = "device";
/// How long after the boat's instruments last gave a position the device's own GPS stays out of it: the two
/// would otherwise take turns, and the boat jump between them (miles apart on an emulator).
const DEVICE_GIVES_WAY: Duration = Duration::seconds(10);
/// AIS targets silent for this long are dropped.
const TARGET_TIMEOUT: Duration = Duration::minutes(10);
/// Minimum movement before another track point is recorded, in nautical miles.
const TRACK_MIN_MOVE_NM: f64 = 0.01;
/// How long without movement before a track point is recorded anyway.
const TRACK_MAX_GAP: Duration = Duration::seconds(60);

#[derive(Clone, Debug)]
pub struct MonitorLine {
    pub time: DateTime<Utc>,
    pub source: String,
    pub text: String,
    /// False when the line was not a valid sentence.
    pub ok: bool,
}

/// Something the user should be told about.
#[derive(Clone, Debug, PartialEq)]
pub enum Notice {
    Arrived { waypoint: String },
    RouteFinished { route: String },
    AnchorAlarm { distance_nm: f64 },
    AnchorCleared,
    AisAlarm { mmsi: u32, name: Option<String>, cpa_nm: f64, tcpa_min: f64 },
    AisCleared { mmsi: u32 },
    /// An engine set the Sailing / Motoring switch.
    SailMode { mode: crate::perf::SailMode },
}

struct Recording {
    track_id: i64,
    last_pos: Option<LatLon>,
    last_time: DateTime<Utc>,
}

/// A new trail point is kept once the boat has moved this far (about 15 m).
const TRAIL_STEP_NM: f64 = 0.008;
/// How long the trail is kept.
const TRAIL_KEPT: chrono::Duration = chrono::Duration::hours(2);

/// A performance-log window just written: its row, time, place and the instruments' true wind (knots,
/// from) when there was one.
/// A performance window just logged, for the app to compare with what it has loaded: its row, when and
/// where, the true wind the instruments measured (knots, from), and the stream they measured (`Sample::stream`).
#[derive(Clone, Debug, PartialEq)]
pub struct Written {
    pub id: i64,
    pub t: DateTime<Utc>,
    pub pos: LatLon,
    pub wind: Option<(f64, f64)>,
    pub stream: Option<(f64, f64)>,
}

pub struct Session {
    pub store: NavStore,
    pub waypoints: Vec<Waypoint>,
    pub routes: Vec<Route>,
    pub tracks: Vec<Track>,
    /// The routes that keep the plan they were made from (`save_plan`), with when it leaves and ends.
    pub plans: HashMap<i64, PlanTimes>,
    pub own: OwnShip,
    /// Where the boat has been, newest last, for the trail on the chart.
    pub trail: VecDeque<(DateTime<Utc>, LatLon)>,
    /// What the user is alerted to, from the conditions below and what shells add (a stale forecast).
    pub alerts: Alerts,
    /// Connections that have been up, so that losing one is told.
    was_connected: std::collections::HashSet<String>,
    pub targets: AisTargets,
    alarms: AisAlarms,
    pub alarm_cfg: AlarmConfig,
    pub active: Option<ActiveRoute>,
    pub nav: Option<NavData>,
    pub anchor: Option<AnchorWatch>,
    pub anchor_status: Option<AnchorStatus>,
    recording: Option<Recording>,
    pub monitor: VecDeque<MonitorLine>,
    pub conn_status: HashMap<String, Status>,
    /// Lines that were not valid NMEA, since start.
    pub bad_lines: u64,
    /// When a source other than the device's own GPS last moved the boat (`DEVICE_GIVES_WAY`).
    instruments_fix_at: Option<DateTime<Utc>>,
    /// Increases whenever incoming data may have changed what is drawn.
    pub revision: u64,
    ais: Assembler,
    /// The performance log: the switch, the 10 s windows, the hint.
    pub perf: crate::perf::PerfLog,
    /// Windows written since the shell last looked (it adds the forecast at each): id, time, position.
    /// The windows written since the app last read them.
    pub perf_written: Vec<Written>,
    /// When the active route was started, for comparing a planned passage with how it went.
    route_started: Option<DateTime<Utc>>,
}

impl Session {
    pub fn new(store: NavStore) -> Result<Session, rusqlite::Error> {
        let waypoints = store.waypoints()?;
        let routes = store.routes()?;
        let tracks = store.tracks()?;
        let mut plans = HashMap::new();
        for (id, times) in store.plan_times()? {
            // A plan saved before its times were kept is read once and given them.
            let times = times.or_else(|| {
                let t = RoutePlan::decode(&store.plan(id).ok()??)?.times();
                let _ = store.set_plan_times(id, &t);
                Some(t)
            });
            if let Some(t) = times {
                plans.insert(id, t);
            }
        }
        Ok(Session {
            store,
            waypoints,
            routes,
            tracks,
            plans,
            own: OwnShip::default(),
            trail: VecDeque::new(),
            alerts: Alerts::default(),
            was_connected: Default::default(),
            targets: AisTargets::default(),
            alarms: AisAlarms::default(),
            alarm_cfg: AlarmConfig::default(),
            active: None,
            nav: None,
            anchor: None,
            anchor_status: None,
            recording: None,
            monitor: VecDeque::new(),
            conn_status: HashMap::new(),
            bad_lines: 0,
            instruments_fix_at: None,
            revision: 0,
            ais: Assembler::default(),
            perf: crate::perf::PerfLog::default(),
            perf_written: Vec::new(),
            route_started: None,
        })
    }

    // Waypoints

    pub fn add_waypoint(&mut self, pos: LatLon, name: Option<String>) -> Result<Waypoint, rusqlite::Error> {
        let name = name.unwrap_or_else(|| format!("WPT{:03}", self.waypoints.len() + 1));
        let mut w = Waypoint::new(name, pos);
        w.time = Some(Utc::now());
        w.id = self.store.insert_waypoint(&w)?;
        self.waypoints.push(w.clone());
        Ok(w)
    }

    pub fn update_waypoint(&mut self, w: &Waypoint) -> Result<(), rusqlite::Error> {
        self.store.update_waypoint(w)?;
        if let Some(slot) = self.waypoints.iter_mut().find(|x| x.id == w.id) {
            *slot = w.clone();
        }
        Ok(())
    }

    pub fn delete_waypoint(&mut self, id: i64) -> Result<(), rusqlite::Error> {
        self.store.delete_waypoint(id)?;
        self.waypoints.retain(|w| w.id != id);
        Ok(())
    }

    // Routes

    pub fn save_route(&mut self, r: &Route) -> Result<Route, rusqlite::Error> {
        let mut r = r.clone();
        r.name_points();
        if r.id == 0 {
            r.id = self.store.insert_route(&r)?;
            self.routes.push(r.clone());
        } else {
            self.store.update_route(&r)?;
            if let Some(slot) = self.routes.iter_mut().find(|x| x.id == r.id) {
                *slot = r.clone();
            }
            // A route being followed picks up edits to its points.
            if let Some(a) = self.active.as_mut()
                && a.route.id == r.id
            {
                a.route = r.clone();
                a.target = a.target.min(r.points.len().saturating_sub(1));
            }
        }
        Ok(r)
    }

    /// Keeps what a weather route was planned with, with the route.
    pub fn save_plan(&mut self, route_id: i64, plan: &RoutePlan) -> Result<(), rusqlite::Error> {
        let times = plan.times();
        self.store.save_plan(route_id, &plan.encode(), &times)?;
        self.plans.insert(route_id, times);
        Ok(())
    }

    /// The plan route `id` was made from, if it keeps one that can still be read.
    pub fn plan(&self, route_id: i64) -> Option<RoutePlan> {
        if !self.plans.contains_key(&route_id) {
            return None;
        }
        RoutePlan::decode(&self.store.plan(route_id).ok()??)
    }

    pub fn delete_route(&mut self, id: i64) -> Result<(), rusqlite::Error> {
        self.store.delete_route(id)?;
        self.plans.remove(&id);
        self.routes.retain(|r| r.id != id);
        if self.active.as_ref().is_some_and(|a| a.route.id == id) {
            self.stop_navigation();
        }
        Ok(())
    }

    pub fn delete_track(&mut self, id: i64) -> Result<(), rusqlite::Error> {
        if self.recording.as_ref().is_some_and(|r| r.track_id == id) {
            self.recording = None;
        }
        self.store.delete_track(id)?;
        self.tracks.retain(|t| t.id != id);
        Ok(())
    }

    // Navigation

    /// Follows a stored route. When the boat's position is known the first
    /// leg starts from it.
    pub fn activate_route(&mut self, id: i64) -> bool {
        let Some(r) = self.routes.iter().find(|r| r.id == id).cloned() else { return false };
        // Steer to the nearest-ahead point: start with the first point unless
        // there is no position, in which case the route's own start is used.
        let start = self.own.pos;
        match ActiveRoute::new(r, start) {
            Some(a) => {
                self.active = Some(a);
                self.nav = None;
                true
            }
            None => false,
        }
    }

    /// Direct course to a position. Needs a position fix.
    pub fn goto(&mut self, to: LatLon, name: &str) -> bool {
        let Some(from) = self.own.pos else { return false };
        self.active = Some(ActiveRoute::goto(from, RoutePoint::new(name, to)));
        self.nav = None;
        true
    }

    pub fn stop_navigation(&mut self) {
        self.active = None;
        self.nav = None;
    }

    // Anchor

    pub fn set_anchor(&mut self, center: LatLon, radius_nm: f64) {
        self.anchor = Some(AnchorWatch::new(center, radius_nm));
        self.anchor_status = None;
    }

    pub fn clear_anchor(&mut self) {
        self.anchor = None;
        self.anchor_status = None;
    }

    // Track recording

    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    pub fn start_recording(&mut self, name: &str, now: DateTime<Utc>) -> Result<(), rusqlite::Error> {
        let mut t = Track::new(name);
        t.id = self.store.insert_track(&t)?;
        self.recording = Some(Recording { track_id: t.id, last_pos: None, last_time: now });
        self.tracks.push(t);
        Ok(())
    }

    pub fn stop_recording(&mut self) {
        self.recording = None;
    }

    fn record(&mut self, now: DateTime<Utc>) {
        let (Some(rec), Some(pos)) = (self.recording.as_mut(), self.own.pos) else { return };
        let moved = rec.last_pos.map(|p| p.distance_nm(&pos));
        let due = match moved {
            None => true,
            Some(d) => d >= TRACK_MIN_MOVE_NM || (d > 0.002 && now - rec.last_time >= TRACK_MAX_GAP),
        };
        if !due {
            return;
        }
        let pt = TrackPoint { pos, time: Some(now) };
        if self.store.append_track_point(rec.track_id, &pt).is_ok() {
            rec.last_pos = Some(pos);
            rec.last_time = now;
            if let Some(t) = self.tracks.iter_mut().find(|t| t.id == rec.track_id) {
                t.points.push(pt);
            }
        }
    }

    // Incoming data

    /// Applies a batch of events and updates everything that depends on them.
    pub fn process(&mut self, events: &[Event], now: DateTime<Utc>) -> Vec<Notice> {
        if !events.is_empty() {
            self.revision += 1;
        }
        for e in events {
            let device = e.source == DEVICE_SOURCE;
            if device && !matches!(e.payload, Payload::Status(_)) && self.instruments_fix_at.is_some_and(|t| now - t < DEVICE_GIVES_WAY) {
                continue;
            }
            let before = self.own.pos_updated;
            match &e.payload {
                Payload::Status(s) => {
                    self.conn_status.insert(e.source.clone(), s.clone());
                }
                Payload::Line(l) => self.handle_line(&e.source, l, now),
                Payload::Updates(ups) => {
                    for n in ups {
                        self.own.apply(n, now);
                    }
                }
            }
            if !device && self.own.pos_updated != before {
                self.instruments_fix_at = Some(now);
            }
        }
        self.update(now)
    }

    fn handle_line(&mut self, source: &str, line: &str, now: DateTime<Utc>) {
        let parsed = nmea::parse_sentence(line);
        let ok = parsed.is_ok();
        if let Ok(s) = &parsed {
            match nmea::decode(s) {
                Some(Nmea::Ais(a)) => {
                    // VDO is our own transponder; it is not a target.
                    if a.kind != "VDO"
                        && let Some(m) = self.ais.push(&a)
                    {
                        self.targets.update(m, now);
                    }
                }
                Some(n) => self.own.apply(&n, now),
                None => {}
            }
        } else {
            self.bad_lines += 1;
        }
        if self.monitor.len() >= MONITOR_LINES {
            self.monitor.pop_front();
        }
        self.monitor.push_back(MonitorLine { time: now, source: source.to_string(), text: line.to_string(), ok });
    }

    /// Recomputes navigation, alarms and recording from the current state.
    pub fn update(&mut self, now: DateTime<Utc>) -> Vec<Notice> {
        let mut notices = Vec::new();
        self.targets.expire(now, TARGET_TIMEOUT);
        let mut finished_route = None;
        if self.active.is_none() {
            self.route_started = None;
        } else if self.route_started.is_none() {
            self.route_started = Some(now);
        }

        if let Some(fix) = self.own.fix(now) {
            if self.trail.back().is_none_or(|(_, p)| p.distance_nm(&fix.pos) > TRAIL_STEP_NM) {
                self.trail.push_back((now, fix.pos));
            }
            while self.trail.front().is_some_and(|(t, _)| now - *t > TRAIL_KEPT) {
                self.trail.pop_front();
            }
            if let Some(a) = self.active.as_mut() {
                let was_finished = a.finished;
                let d = a.update(&fix);
                if d.arrived && !was_finished {
                    let name = if d.finished { a.target_point().name.clone() } else { a.route.points[d.target].name.clone() };
                    notices.push(Notice::Arrived { waypoint: name });
                    if d.finished {
                        notices.push(Notice::RouteFinished { route: a.route.name.clone() });
                        finished_route = Some((a.route.id, a.route.name.clone()));
                    }
                }
                self.nav = Some(d);
            }
            if let Some(w) = self.anchor.as_mut() {
                let before = self.anchor_status.map(|s| s.alarm).unwrap_or(false);
                let st = w.check(&fix.pos);
                if st.alarm && !before {
                    notices.push(Notice::AnchorAlarm { distance_nm: st.distance_nm });
                } else if !st.alarm && before {
                    notices.push(Notice::AnchorCleared);
                }
                self.anchor_status = Some(st);
            }
            if let (Some(sog), Some(cog)) = (fix.sog_kn, fix.cog) {
                let own = Motion { pos: fix.pos, sog_kn: sog, cog };
                for ev in self.alarms.evaluate(&own, &self.targets, &self.alarm_cfg, now) {
                    notices.push(match ev {
                        AlarmEvent::New { mmsi, cpa } => Notice::AisAlarm {
                            mmsi,
                            name: self.targets.get(mmsi).and_then(|t| t.name.clone()),
                            cpa_nm: cpa.cpa_nm,
                            tcpa_min: cpa.tcpa_min,
                        },
                        AlarmEvent::Cleared { mmsi } => Notice::AisCleared { mmsi },
                    });
                }
            }
        }
        self.record(now);
        self.perf_tick(now, &mut notices);
        if let (Some((id, name)), Some(started)) = (finished_route, self.route_started) {
            self.compare_passage(id, &name, started, now);
        }
        self.refresh_alerts(&notices, now);
        notices
    }

    /// The performance log's tick: the engines' word on the switch, the window that ended, the hint.
    fn perf_tick(&mut self, now: DateTime<Utc>, notices: &mut Vec<Notice>) {
        let tick = self.perf.tick(&self.own, self.anchor.is_some(), now);
        if let Some(mode) = tick.engine_set {
            notices.push(Notice::SailMode { mode });
        }
        if let Some(since) = tick.hint_since {
            let _ = self.store.mark_perf_motored(since, now);
        }
        if let Some(s) = tick.sample
            && let Ok(id) = self.store.insert_perf_sample(&s)
        {
            self.perf_written.push(Written { id, t: s.t, pos: s.pos, wind: s.tws.zip(s.twd), stream: s.stream() });
        }
    }

    /// The user flips the Sailing / Motoring switch; false while an engine decides.
    pub fn set_sail_mode(&mut self, mode: crate::perf::SailMode) -> bool {
        let ok = self.perf.set_mode(mode);
        if ok {
            self.alerts.clear(&Source::LooksMotored);
            self.revision += 1;
        }
        ok
    }

    /// A route that had a weather routing plan was sailed to its end: how did it go against the plan?
    fn compare_passage(&mut self, route_id: i64, name: &str, started: DateTime<Utc>, now: DateTime<Utc>) {
        if !self.plans.contains_key(&route_id) {
            return;
        }
        let Some(plan) = self.plan(route_id) else { return };
        let Ok(samples) = self.store.perf_samples(Some(started), Some(now)) else { return };
        let p = crate::perf::passage::compare(route_id, name, &plan, started, now, &samples);
        let _ = self.store.insert_passage(&p);
    }

    /// Turns the notices and the conditions that persist into alerts, and clears those that ended.
    fn refresh_alerts(&mut self, notices: &[Notice], now: DateTime<Utc>) {
        use crate::i18n::{gettext, gettext_f};
        for n in notices {
            match n {
                Notice::AnchorAlarm { distance_nm } => {
                    let text = gettext_f("Anchor alarm: {d} nm from the anchor", &[("d", &format!("{distance_nm:.2}"))]);
                    self.alerts.raise(now, Level::Alarm, Source::Anchor, text, None);
                }
                Notice::AnchorCleared => self.alerts.clear(&Source::Anchor),
                Notice::AisAlarm { mmsi, name, cpa_nm, .. } => {
                    let who = name.clone().unwrap_or_else(|| mmsi.to_string());
                    self.alerts.raise(now, Level::Alarm, Source::Collision(*mmsi), gettext_f("Collision risk: {name}", &[("name", &who)]), Some(*cpa_nm));
                }
                Notice::AisCleared { mmsi } => self.alerts.clear(&Source::Collision(*mmsi)),
                Notice::Arrived { waypoint } => {
                    let text = gettext_f("Arrived at {waypoint}", &[("waypoint", waypoint)]);
                    self.alerts.raise(now, Level::Info, Source::Event(text.clone()), text, None);
                }
                Notice::RouteFinished { route } => {
                    let text = gettext_f("Route {route} completed", &[("route", route)]);
                    self.alerts.raise(now, Level::Info, Source::Event(text.clone()), text, None);
                }
                Notice::SailMode { .. } => {}
            }
        }
        // Without engine data, the log suggests motoring when the boat goes where sails cannot take it.
        let p = &self.perf;
        if p.hint == crate::perf::Hint::LooksMotored && p.mode != crate::perf::SailMode::Motoring && p.mode_from == crate::perf::ModeSource::User {
            self.alerts.raise(now, Level::Warning, Source::LooksMotored, gettext("The boat seems to be under engine"), None);
        } else {
            self.alerts.clear(&Source::LooksMotored);
        }
        // The anchor alarm's distance moves while it lasts.
        if let Some(st) = self.anchor_status.filter(|s| s.alarm) {
            let text = gettext_f("Anchor alarm: {d} nm from the anchor", &[("d", &format!("{:.2}", st.distance_nm))]);
            self.alerts.raise(now, Level::Alarm, Source::Anchor, text, None);
        } else if self.anchor.is_none() {
            self.alerts.clear(&Source::Anchor);
        }
        // What the collision tracker has acknowledged is acknowledged here; what it dropped is gone.
        let active = self.alarms.active();
        for (mmsi, acked) in &active {
            if *acked && let Some(a) = self.alerts.by_source(&Source::Collision(*mmsi)).map(|a| a.id) {
                self.alerts.acknowledge(a);
            }
        }
        self.alerts.clear_where(|s| match s {
            Source::Collision(m) => active.iter().any(|(a, _)| a == m),
            _ => true,
        });

        let navigating = self.active.is_some();
        match self.own.pos_updated.filter(|_| self.own.pos.is_some()) {
            Some(t) if now - t > Duration::minutes(2) => {
                let level = if navigating { Level::Alarm } else { Level::Warning };
                self.alerts.raise(now, level, Source::LostFix, gettext("Position lost"), None);
            }
            _ => self.alerts.clear(&Source::LostFix),
        }
        let limit = self.alarm_cfg.xte_limit_nm;
        match self.nav.as_ref().filter(|n| navigating && limit > 0.0 && n.xte_nm.abs() > limit) {
            Some(n) => {
                self.alerts.raise(now, Level::Warning, Source::OffCourse, gettext_f("Off course: {d} nm from the route", &[("d", &format!("{:.2}", n.xte_nm.abs()))]), None);
            }
            None => self.alerts.clear(&Source::OffCourse),
        }
        match self.own.depth_m.filter(|d| self.alarm_cfg.shallow_m > 0.0 && *d < self.alarm_cfg.shallow_m) {
            Some(d) => {
                self.alerts.raise(now, Level::Alarm, Source::ShallowWater, gettext_f("Shallow water: {d} m", &[("d", &format!("{d:.1}"))]), None);
            }
            None => self.alerts.clear(&Source::ShallowWater),
        }
        match self.own.wind_true.filter(|(_, kn)| self.alarm_cfg.wind_limit_kn > 0.0 && *kn > self.alarm_cfg.wind_limit_kn) {
            Some((_, kn)) => {
                self.alerts.raise(now, Level::Warning, Source::WindLimit, gettext_f("Wind {kn} kn, over your limit", &[("kn", &format!("{kn:.0}"))]), None);
            }
            None => self.alerts.clear(&Source::WindLimit),
        }
        for (id, status) in &self.conn_status {
            match status {
                Status::Connected => {
                    self.was_connected.insert(id.clone());
                    self.alerts.clear(&Source::DataSource(id.clone()));
                }
                Status::Disconnected(_) if self.was_connected.contains(id) => {
                    self.alerts.raise(now, Level::Warning, Source::DataSource(id.clone()), gettext_f("Data source lost: {name}", &[("name", id)]), None);
                }
                _ => {}
            }
        }
        self.alerts.tick(now);
    }

    pub fn acknowledge_ais(&mut self, mmsi: u32) {
        self.alarms.acknowledge(mmsi);
        if let Some(id) = self.alerts.by_source(&Source::Collision(mmsi)).map(|a| a.id) {
            self.alerts.acknowledge(id);
        }
    }

    /// Acknowledges an alert: for a collision this also stops the target blinking.
    pub fn acknowledge_alert(&mut self, id: u64) {
        match self.alerts.get(id).map(|a| a.source.clone()) {
            Some(Source::Collision(m)) => self.acknowledge_ais(m),
            _ => self.alerts.acknowledge(id),
        }
    }

    /// Targets currently dangerous, with whether the user has seen them.
    pub fn ais_alerts(&self) -> Vec<(u32, bool)> {
        self.alarms.active()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comm::nmea::{frame, make_rmc};
    use crate::comm::sim::Simulator;
    use chrono::TimeZone;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap()
    }

    fn session() -> Session {
        Session::new(NavStore::open_memory().unwrap()).unwrap()
    }

    fn line(l: String) -> Event {
        Event { source: "t".into(), payload: Payload::Line(l) }
    }

    #[test]
    fn the_devices_own_gps_gives_way_to_the_boats_instruments() {
        let mut s = session();
        let device = |p: LatLon| Event { source: DEVICE_SOURCE.into(), payload: Payload::Updates(vec![Nmea::Gll { pos: Some(p), valid: true }]) };
        let boat = LatLon::new(47.3, -3.0);
        let phone = LatLon::new(37.4, -122.1);
        s.process(&[device(phone)], t0());
        assert_eq!(s.own.pos, Some(phone), "alone, the device gives the position");
        s.process(&[line(make_rmc(t0(), &boat, 5.0, 90.0))], t0() + Duration::seconds(1));
        s.process(&[device(phone)], t0() + Duration::seconds(2));
        assert_eq!(s.own.pos, Some(boat), "the instruments' fix stands");
        s.process(&[device(phone)], t0() + Duration::seconds(12));
        assert_eq!(s.own.pos, Some(phone), "once they fall silent, the device takes over");
    }

    fn position(s: &mut Session, p: LatLon, sog: f64, cog: f64, at: DateTime<Utc>) -> Vec<Notice> {
        s.process(&[line(make_rmc(at, &p, sog, cog))], at)
    }

    fn route_east() -> Route {
        let mut r = Route::new("east");
        for lon in [-4.0, -3.9, -3.8] {
            r.points.push(RoutePoint::new("", LatLon::new(48.0, lon)));
        }
        r
    }

    #[test]
    fn lines_update_own_ship_and_fill_the_monitor() {
        let mut s = session();
        s.process(&[line(make_rmc(t0(), &LatLon::new(48.0, -4.0), 5.0, 90.0)), line("garbage".into())], t0());
        assert_eq!(s.own.sog_kn, Some(5.0));
        assert_eq!(s.monitor.len(), 2);
        assert!(s.monitor[0].ok && !s.monitor[1].ok);
        assert_eq!(s.bad_lines, 1);
    }

    #[test]
    fn the_monitor_keeps_only_the_latest_lines() {
        let mut s = session();
        for i in 0..(MONITOR_LINES + 50) {
            s.process(&[line(frame(&format!("GPGGA,{i}"), '$'))], t0());
        }
        assert_eq!(s.monitor.len(), MONITOR_LINES);
        assert!(s.monitor.back().unwrap().text.contains(&format!("{}", MONITOR_LINES + 49)));
    }

    #[test]
    fn the_revision_moves_only_when_data_arrives() {
        let mut s = session();
        assert_eq!(s.revision, 0);
        s.process(&[], t0());
        assert_eq!(s.revision, 0, "a quiet tick changes nothing");
        s.process(&[line(make_rmc(t0(), &LatLon::new(48.0, -4.0), 5.0, 90.0))], t0());
        assert_eq!(s.revision, 1);
    }

    #[test]
    fn connection_status_is_tracked() {
        let mut s = session();
        s.process(&[Event { source: "gps".into(), payload: Payload::Status(Status::Connected) }], t0());
        assert_eq!(s.conn_status["gps"], Status::Connected);
    }

    #[test]
    fn waypoints_are_stored_numbered_and_deleted() {
        let mut s = session();
        let a = s.add_waypoint(LatLon::new(1.0, 2.0), None).unwrap();
        let b = s.add_waypoint(LatLon::new(3.0, 4.0), Some("Home".into())).unwrap();
        assert_eq!((a.name.as_str(), b.name.as_str()), ("WPT001", "Home"));
        let mut c = b.clone();
        c.name = "Harbour".into();
        s.update_waypoint(&c).unwrap();
        assert_eq!(s.waypoints[1].name, "Harbour");
        assert_eq!(s.store.waypoints().unwrap()[1].name, "Harbour");
        s.delete_waypoint(a.id).unwrap();
        assert_eq!(s.waypoints.len(), 1);
        // A new session sees the persisted data.
        let again = Session::new(NavStore::open_memory().unwrap()).unwrap();
        assert!(again.waypoints.is_empty());
    }

    #[test]
    fn following_a_route_announces_arrivals_and_the_finish() {
        let mut s = session();
        let r = s.save_route(&route_east()).unwrap();
        assert_eq!(r.points[0].name, "001", "unnamed points are numbered");
        position(&mut s, LatLon::new(48.0, -4.0), 6.0, 90.0, t0());
        assert!(s.activate_route(r.id));
        // Starting on the first point arrives at it immediately.
        let n = position(&mut s, LatLon::new(48.0, -4.0), 6.0, 90.0, t0() + Duration::seconds(1));
        assert_eq!(n, vec![Notice::Arrived { waypoint: "001".into() }]);
        let nav = s.nav.clone().unwrap();
        assert_eq!(nav.target, 0, "the update that arrives reports the point it reached");
        let n = position(&mut s, LatLon::new(48.0, -3.9), 6.0, 90.0, t0() + Duration::seconds(2));
        assert_eq!(n, vec![Notice::Arrived { waypoint: "002".into() }]);
        let n = position(&mut s, LatLon::new(48.0, -3.8), 6.0, 90.0, t0() + Duration::seconds(3));
        assert!(n.contains(&Notice::Arrived { waypoint: "003".into() }));
        assert!(n.contains(&Notice::RouteFinished { route: "east".into() }));
        // Finished: no repeat announcements.
        assert!(position(&mut s, LatLon::new(48.0, -3.8), 0.0, 0.0, t0() + Duration::seconds(4)).is_empty());
    }

    #[test]
    fn goto_needs_a_fix_and_steers_at_the_target() {
        let mut s = session();
        assert!(!s.goto(LatLon::new(48.1, -4.0), "x"));
        position(&mut s, LatLon::new(48.0, -4.0), 5.0, 0.0, t0());
        assert!(s.goto(LatLon::new(48.1, -4.0), "Dest"));
        s.update(t0());
        let d = s.nav.clone().unwrap();
        assert!((d.range_nm - 6.0).abs() < 0.05 && d.bearing < 0.5);
        s.stop_navigation();
        assert!(s.active.is_none() && s.nav.is_none());
    }

    #[test]
    fn editing_the_active_route_updates_the_active_copy() {
        let mut s = session();
        let r = s.save_route(&route_east()).unwrap();
        s.activate_route(r.id);
        let mut edited = r.clone();
        edited.points.truncate(2);
        s.save_route(&edited).unwrap();
        assert_eq!(s.active.as_ref().unwrap().route.points.len(), 2);
        assert!(s.active.as_ref().unwrap().target < 2);
        s.delete_route(r.id).unwrap();
        assert!(s.active.is_none());
        assert!(!s.activate_route(9999));
    }

    #[test]
    fn the_anchor_alarm_raises_and_clears_once() {
        let mut s = session();
        let c = LatLon::new(48.0, -4.0);
        position(&mut s, c, 0.0, 0.0, t0());
        s.set_anchor(c, 0.1);
        assert!(position(&mut s, c, 0.0, 0.0, t0() + Duration::seconds(1)).is_empty());
        let n = position(&mut s, c.destination(0.0, 0.15), 0.0, 0.0, t0() + Duration::seconds(2));
        assert!(matches!(n.as_slice(), [Notice::AnchorAlarm { distance_nm }] if *distance_nm > 0.14));
        // Still outside: no repeat.
        assert!(position(&mut s, c.destination(0.0, 0.16), 0.0, 0.0, t0() + Duration::seconds(3)).is_empty());
        let n = position(&mut s, c, 0.0, 0.0, t0() + Duration::seconds(4));
        assert_eq!(n, vec![Notice::AnchorCleared]);
        s.clear_anchor();
        assert!(s.anchor.is_none() && s.anchor_status.is_none());
    }

    #[test]
    fn recording_skips_stationary_points_and_persists() {
        let mut s = session();
        position(&mut s, LatLon::new(48.0, -4.0), 5.0, 90.0, t0());
        s.start_recording("trip", t0()).unwrap();
        assert!(s.is_recording());
        // First fix recorded; repeats at the same spot are not.
        for i in 0..5 {
            position(&mut s, LatLon::new(48.0, -4.0), 0.0, 0.0, t0() + Duration::seconds(i));
        }
        assert_eq!(s.tracks[0].points.len(), 1);
        // Moving 0.02 nm records a point.
        position(&mut s, LatLon::new(48.0, -4.0).destination(90.0, 0.02), 5.0, 90.0, t0() + Duration::seconds(10));
        assert_eq!(s.tracks[0].points.len(), 2);
        assert_eq!(s.store.tracks().unwrap()[0].points.len(), 2);
        s.stop_recording();
        position(&mut s, LatLon::new(48.5, -4.0), 5.0, 0.0, t0() + Duration::seconds(20));
        assert_eq!(s.tracks[0].points.len(), 2);
        let id = s.tracks[0].id;
        s.delete_track(id).unwrap();
        assert!(s.tracks.is_empty() && s.store.tracks().unwrap().is_empty());
    }

    #[test]
    fn deleting_the_track_being_recorded_stops_recording() {
        let mut s = session();
        s.start_recording("t", t0()).unwrap();
        let id = s.tracks[0].id;
        s.delete_track(id).unwrap();
        assert!(!s.is_recording());
    }

    #[test]
    fn slow_drift_is_recorded_after_the_gap() {
        let mut s = session();
        let p = LatLon::new(48.0, -4.0);
        position(&mut s, p, 0.0, 0.0, t0());
        s.start_recording("t", t0()).unwrap();
        position(&mut s, p, 0.0, 0.0, t0());
        // 0.005 nm is below the movement threshold but above the drift floor.
        let q = p.destination(0.0, 0.005);
        position(&mut s, q, 0.1, 0.0, t0() + Duration::seconds(30));
        assert_eq!(s.tracks[0].points.len(), 1);
        position(&mut s, q, 0.1, 0.0, t0() + Duration::seconds(61));
        assert_eq!(s.tracks[0].points.len(), 2);
    }

    #[test]
    fn simulator_traffic_drives_targets_and_a_collision_alert() {
        let mut s = session();
        let start = LatLon::new(48.35, -4.55);
        // The demo ferry crosses our track about 12 minutes ahead.
        let mut sim = Simulator::demo(start, 90.0, 6.0, t0());
        let mut notices = Vec::new();
        for i in 0..120 {
            let now = t0() + Duration::seconds(i);
            let lines: Vec<Event> = sim.step(1.0).into_iter().map(line).collect();
            notices.extend(s.process(&lines, now));
        }
        assert_eq!(s.targets.len(), 3);
        assert!(s.own.pos.is_some());
        assert!(
            notices.iter().any(|n| matches!(n, Notice::AisAlarm { .. })),
            "expected a collision alert from the demo traffic, got {notices:?}"
        );
    }

    #[test]
    fn the_clear_demo_traffic_raises_no_alert() {
        let mut s = session();
        // Past the ferry's closest approach (12 minutes) and on, so nothing is dangerous at any time.
        let mut sim = Simulator::demo_clear(LatLon::new(48.35, -4.55), 90.0, 6.0, t0());
        let mut notices = Vec::new();
        for i in 0..1800 {
            let lines: Vec<Event> = sim.step(1.0).into_iter().map(line).collect();
            notices.extend(s.process(&lines, t0() + Duration::seconds(i)));
        }
        assert_eq!(s.targets.len(), 3);
        assert!(!notices.iter().any(|n| matches!(n, Notice::AisAlarm { .. })), "{notices:?}");
    }

    #[test]
    fn a_signal_k_engine_sets_motoring_and_the_log_keeps_windows() {
        use crate::comm::signalk::{parse_delta, updates_from_pairs};
        use crate::perf::{ModeSource, SailMode};
        let mut s = session();
        s.set_sail_mode(SailMode::Sailing);
        let delta = |rev_hz: f64| {
            let text = format!(
                r#"{{"updates":[{{"values":[
                {{"path":"navigation.position","value":{{"latitude":47.5,"longitude":-3.0}}}},
                {{"path":"navigation.speedOverGround","value":3.0}},
                {{"path":"navigation.courseOverGroundTrue","value":1.0}},
                {{"path":"navigation.headingTrue","value":1.0}},
                {{"path":"navigation.speedThroughWater","value":3.0}},
                {{"path":"environment.wind.angleApparent","value":0.8}},
                {{"path":"environment.wind.speedApparent","value":6.0}},
                {{"path":"propulsion.port.revolutions","value":{rev_hz}}}]}}]}}"#
            );
            Event { source: "sk".into(), payload: Payload::Updates(updates_from_pairs(&parse_delta(&text))) }
        };
        let mut notices = Vec::new();
        for i in 0..240 {
            notices.extend(s.process(&[delta(25.0)], t0() + Duration::milliseconds(i * 250)));
        }
        assert_eq!((s.perf.mode, s.perf.mode_from), (SailMode::Motoring, ModeSource::Engine));
        assert_eq!(notices.iter().filter(|n| matches!(n, Notice::SailMode { mode: SailMode::Motoring })).count(), 1);
        assert!(!s.set_sail_mode(SailMode::Sailing), "the engine decides");
        let samples = s.store.perf_samples(None, None).unwrap();
        assert!(samples.len() >= 5, "{}", samples.len());
        assert!(samples.iter().all(|x| x.mode == SailMode::Motoring && x.rpm == Some(1500.0) && x.stw.is_some() && x.awa.is_some()));
        assert_eq!(s.perf_written.len(), samples.len());
        // The engine stops: sailing a few seconds later.
        for i in 240..340 {
            notices.extend(s.process(&[delta(0.0)], t0() + Duration::milliseconds(i * 250)));
        }
        assert_eq!(s.perf.mode, SailMode::Sailing);
        assert!(notices.iter().any(|n| matches!(n, Notice::SailMode { mode: SailMode::Sailing })));
    }

    #[test]
    fn our_own_transponder_is_not_a_target() {
        let mut s = session();
        let vdo = frame("AIVDO,1,1,,A,177KQJ5000G?tO`K>RA1wUbN0TKH,0", '!');
        s.process(&[line(vdo)], t0());
        assert!(s.targets.is_empty());
        let vdm = frame("AIVDM,1,1,,A,177KQJ5000G?tO`K>RA1wUbN0TKH,0", '!');
        s.process(&[line(vdm)], t0());
        assert_eq!(s.targets.len(), 1);
    }

    #[test]
    fn json_updates_apply_like_nmea() {
        let mut s = session();
        s.process(
            &[Event { source: "gpsd".into(), payload: Payload::Updates(vec![Nmea::Gll { pos: Some(LatLon::new(1.0, 2.0)), valid: true }]) }],
            t0(),
        );
        assert_eq!(s.own.pos, Some(LatLon::new(1.0, 2.0)));
    }

    #[test]
    fn stale_targets_are_dropped() {
        let mut s = session();
        let vdm = frame("AIVDM,1,1,,A,177KQJ5000G?tO`K>RA1wUbN0TKH,0", '!');
        s.process(&[line(vdm)], t0());
        s.update(t0() + Duration::minutes(11));
        assert!(s.targets.is_empty());
    }

    #[test]
    fn the_anchor_and_a_lost_fix_become_alerts_and_clear_with_their_conditions() {
        let mut s = session();
        position(&mut s, LatLon::new(48.0, -4.0), 0.0, 0.0, t0());
        s.set_anchor(LatLon::new(48.0, -4.0), 0.05);
        position(&mut s, LatLon::new(48.0, -3.9), 0.0, 0.0, t0() + Duration::seconds(10));
        let b = s.alerts.banner().expect("dragging");
        assert_eq!(b.alert.source, Source::Anchor);
        assert_eq!(b.alert.level, Level::Alarm);
        let id = b.alert.id;
        s.acknowledge_alert(id);
        assert!(s.alerts.banner().unwrap().dim);
        position(&mut s, LatLon::new(48.0, -4.0), 0.0, 0.0, t0() + Duration::seconds(20));
        assert!(s.alerts.banner().is_none(), "back inside the circle");

        // No fix for three minutes: a warning, and an alarm once a route is active.
        s.update(t0() + Duration::minutes(4));
        assert_eq!(s.alerts.by_source(&Source::LostFix).map(|a| a.level), Some(Level::Warning));
        s.save_route(&route_east()).unwrap();
        let id = s.routes[0].id;
        assert!(s.activate_route(id));
        s.update(t0() + Duration::minutes(5));
        assert_eq!(s.alerts.by_source(&Source::LostFix).map(|a| a.level), Some(Level::Alarm));
        position(&mut s, LatLon::new(48.0, -3.99), 5.0, 90.0, t0() + Duration::minutes(6));
        assert!(s.alerts.by_source(&Source::LostFix).is_none(), "a fix again");
    }

    #[test]
    fn a_route_finished_is_an_info_message_that_expires() {
        let mut s = session();
        s.alerts.raise(t0(), Level::Info, Source::Event("Arrived at A".into()), "Arrived at A", None);
        assert_eq!(s.alerts.messages().len(), 1);
        s.update(t0() + Duration::seconds(10));
        assert!(s.alerts.messages().is_empty());
    }

    #[test]
    fn a_route_keeps_its_plan_until_it_is_deleted() {
        use crate::routing::isochrone::Node;
        use crate::weather::store::WeatherStore;
        let mut s = session();
        let r = s.save_route(&route_east()).unwrap();
        assert!(s.plan(r.id).is_none());
        let node = |lon: f64, h: i64| Node { pos: LatLon::new(48.0, lon), time: t0() + Duration::hours(h), cog: 90.0, sog_kn: 5.0, heading: 90.0, tws_kn: 12.0, twd: 0.0, motoring: false, current: (0.0, 0.0) };
        let points: Vec<LatLon> = r.points.iter().map(|p| p.pos).collect();
        let plan = RoutePlan::new(t0(), LatLon::new(48.0, -3.8), true, vec![node(-4.0, 0), node(-3.8, 4)], points.clone(), &WeatherStore::new());
        s.save_plan(r.id, &plan).unwrap();
        assert!(s.plan(r.id).is_some_and(|p| p.matches(&points)));
        assert_eq!(s.plans.get(&r.id), Some(&plan.times()), "its times are kept beside it");
        assert_eq!(s.store.plan_times().unwrap(), vec![(r.id, Some(plan.times()))]);
        assert!(s.store.plan_ids().unwrap().contains(&r.id), "it is in the database, not only in memory");
        s.delete_route(r.id).unwrap();
        assert!(s.plan(r.id).is_none() && s.store.plan_ids().unwrap().is_empty(), "the plan goes with the route");
    }

    #[test]
    fn a_plan_saved_before_its_times_were_kept_is_given_them() {
        use crate::routing::isochrone::Node;
        use crate::weather::store::WeatherStore;
        let mut s = session();
        let r = s.save_route(&route_east()).unwrap();
        let node = |lon: f64, h: i64| Node { pos: LatLon::new(48.0, lon), time: t0() + Duration::hours(h), cog: 90.0, sog_kn: 5.0, heading: 90.0, tws_kn: 12.0, twd: 0.0, motoring: false, current: (0.0, 0.0) };
        let plan = RoutePlan::new(t0(), LatLon::new(48.0, -3.8), false, vec![node(-4.0, 0), node(-3.9, 7)], vec![], &WeatherStore::new());
        s.save_plan(r.id, &plan).unwrap();
        s.store.conn.execute("UPDATE route_plans SET depart=NULL, arrive=NULL, arrives=NULL", []).unwrap();
        assert_eq!(s.store.plan_times().unwrap(), vec![(r.id, None)]);
        let s = Session::new(s.store).unwrap();
        let want = PlanTimes { depart: t0(), end: t0() + Duration::hours(7), arrives: false };
        assert_eq!(s.plans.get(&r.id), Some(&want), "read from the plan itself");
        assert_eq!(s.store.plan_times().unwrap(), vec![(r.id, Some(want))], "and written back");
    }
}
