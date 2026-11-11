//! Alerts as the design's hierarchy has them (`design/design-system/alert-hierarchy.md`): four
//! levels, each with its own persistence. An alarm sounds until it is acknowledged or snoozed; a
//! warning clears itself when its condition does; an info message expires; a hint lasts while its
//! mode does. This is the model; the shells decide how each looks and sounds.

use chrono::{DateTime, Duration, Utc};

/// How loud a thing is. Ordered so that an alarm is the greatest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Hint,
    Info,
    Warning,
    Alarm,
}

/// What an alert is about. Raising an alert for a source that already has one updates it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// An AIS target closing in, by MMSI.
    Collision(u32),
    /// The anchor watch.
    Anchor,
    ShallowWater,
    LostFix,
    OffCourse,
    StaleForecast,
    /// A connection that dropped, by its id.
    DataSource(String),
    /// No licensed official chart covers the view.
    PlanningOnly,
    WindLimit,
    /// The performance log thinks the boat is motoring while the switch says it is not (no engine reports).
    LooksMotored,
    /// Another forecast model has been closer to the wind the boat measures than the one in use.
    ForecastDrift,
    /// The current source in use has been far off the stream the boat measures.
    CurrentDrift,
    /// Something that happened once: arrival, a finished route, a finished download. The text is its identity.
    Event(String),
    /// A mode's instruction, such as drawing a route.
    Hint(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Sounding (alarms) or showing.
    Active,
    /// Silenced until the time, then active again if the condition is still there.
    Snoozed { until: DateTime<Utc> },
    /// Acknowledged: shown dim, silent, until the condition clears.
    Acknowledged,
}

/// What the user can do to an alert.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Acknowledge,
    Snooze,
    Dismiss,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Alert {
    pub id: u64,
    pub level: Level,
    pub source: Source,
    pub message: String,
    pub state: State,
    pub since: DateTime<Utc>,
    /// A measure of how bad it is (the CPA of a collision, in nautical miles), for the retrigger rule.
    pub metric: Option<f64>,
    /// The metric when it was acknowledged.
    acked_at_metric: Option<f64>,
    /// Hidden by the user for this session.
    dismissed: bool,
}

impl Alert {
    pub fn actions(&self) -> Vec<Action> {
        match self.level {
            Level::Alarm => vec![Action::Acknowledge, Action::Snooze],
            Level::Warning => vec![Action::Dismiss],
            Level::Info | Level::Hint => Vec::new(),
        }
    }

    /// Whether it is making noise or demanding attention now.
    pub fn is_active(&self) -> bool {
        self.state == State::Active && !self.dismissed
    }
}

/// How long a snooze lasts for each kind of alarm.
pub fn snooze_for(source: &Source) -> Duration {
    match source {
        Source::Anchor => Duration::minutes(5),
        _ => Duration::minutes(2),
    }
}

/// How long an info message stays (`toast-duration`).
pub const INFO_LIFETIME: Duration = Duration::seconds(4);
/// The alarm sound repeats at this interval (`alarm-repeat`).
pub const ALARM_REPEAT: Duration = Duration::seconds(4);
/// A collision alarm that was acknowledged comes back when the closest approach is under this
/// fraction of what it was.
const WORSE_BY: f64 = 0.5;

/// What the banner under the toolbar shows: the newest unsilenced alarm, and how many there are.
#[derive(Clone, Debug, PartialEq)]
pub struct BannerView<'a> {
    pub alert: &'a Alert,
    /// Active alarms, so that the banner can say "2 alarms".
    pub count: usize,
    /// Every alarm is acknowledged or snoozed: the banner is dim.
    pub dim: bool,
}

#[derive(Default)]
pub struct Alerts {
    list: Vec<Alert>,
    next_id: u64,
    last_sound: Option<DateTime<Utc>>,
    chimed: std::collections::HashSet<u64>,
}

impl Alerts {
    /// Raises an alert, or updates the one this source already has. An acknowledged collision comes
    /// back to active only when `metric` shows it got at least half as close again.
    pub fn raise(&mut self, now: DateTime<Utc>, level: Level, source: Source, message: impl Into<String>, metric: Option<f64>) -> u64 {
        let message = message.into();
        if let Some(a) = self.list.iter_mut().find(|a| a.source == source) {
            a.message = message;
            a.level = level;
            if metric.is_some() {
                a.metric = metric;
            }
            if a.state == State::Acknowledged
                && let (Some(now_m), Some(then)) = (metric, a.acked_at_metric)
                && now_m < then * WORSE_BY
            {
                a.state = State::Active;
                a.acked_at_metric = None;
            }
            return a.id;
        }
        self.next_id += 1;
        self.list.push(Alert { id: self.next_id, level, source, message, state: State::Active, since: now, metric, acked_at_metric: None, dismissed: false });
        self.next_id
    }

    /// The condition is over: the alert goes.
    pub fn clear(&mut self, source: &Source) {
        self.list.retain(|a| &a.source != source);
    }

    /// Clears every alert of a kind, such as all the collision alerts whose targets are gone.
    pub fn clear_where(&mut self, keep: impl Fn(&Source) -> bool) {
        self.list.retain(|a| keep(&a.source));
    }

    pub fn get(&self, id: u64) -> Option<&Alert> {
        self.list.iter().find(|a| a.id == id)
    }

    pub fn by_source(&self, source: &Source) -> Option<&Alert> {
        self.list.iter().find(|a| &a.source == source)
    }

    pub fn acknowledge(&mut self, id: u64) {
        if let Some(a) = self.list.iter_mut().find(|a| a.id == id) {
            a.state = State::Acknowledged;
            a.acked_at_metric = a.metric;
        }
    }

    pub fn snooze(&mut self, id: u64, now: DateTime<Utc>) {
        if let Some(a) = self.list.iter_mut().find(|a| a.id == id) {
            a.state = State::Snoozed { until: now + snooze_for(&a.source) };
        }
    }

    pub fn dismiss(&mut self, id: u64) {
        if let Some(a) = self.list.iter_mut().find(|a| a.id == id) {
            a.dismissed = true;
        }
    }

    /// Ends snoozes that are over, and drops info messages that have had their time.
    pub fn tick(&mut self, now: DateTime<Utc>) {
        for a in &mut self.list {
            if let State::Snoozed { until } = a.state
                && now >= until
            {
                a.state = State::Active;
            }
        }
        self.list.retain(|a| a.level != Level::Info || now - a.since < INFO_LIFETIME);
    }

    /// The alarms, newest first.
    fn alarms(&self) -> impl Iterator<Item = &Alert> {
        self.list.iter().rev().filter(|a| a.level == Level::Alarm)
    }

    pub fn banner(&self) -> Option<BannerView<'_>> {
        let count = self.alarms().filter(|a| a.is_active()).count();
        match self.alarms().find(|a| a.is_active()) {
            Some(alert) => Some(BannerView { alert, count, dim: false }),
            // Nothing sounding, but something still true: the dim banner.
            None => self.alarms().next().map(|alert| BannerView { alert, count: 0, dim: true }),
        }
    }

    /// Warnings that show, newest first. Dismissed ones do not.
    pub fn warnings(&self) -> Vec<&Alert> {
        self.list.iter().rev().filter(|a| a.level == Level::Warning && !a.dismissed).collect()
    }

    /// Info messages and hints, newest first.
    pub fn messages(&self) -> Vec<&Alert> {
        self.list.iter().rev().filter(|a| matches!(a.level, Level::Info | Level::Hint)).collect()
    }

    pub fn all(&self) -> &[Alert] {
        &self.list
    }

    /// True when the alarm sound is due: an alarm is active and the repeat interval has passed since
    /// the last time this said so.
    pub fn alarm_sound_due(&mut self, now: DateTime<Utc>) -> bool {
        if !self.alarms().any(|a| a.is_active()) {
            self.last_sound = None;
            return false;
        }
        if self.last_sound.is_none_or(|t| now - t >= ALARM_REPEAT) {
            self.last_sound = Some(now);
            return true;
        }
        false
    }

    /// How many warnings have appeared since this was last asked, for the one chime each gets.
    pub fn take_new_warnings(&mut self) -> usize {
        let mut n = 0;
        for a in self.list.iter().filter(|a| a.level == Level::Warning && !a.dismissed) {
            if self.chimed.insert(a.id) {
                n += 1;
            }
        }
        self.chimed.retain(|id| self.list.iter().any(|a| a.id == *id));
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> DateTime<Utc> {
        "2026-10-27T12:00:00Z".parse().unwrap()
    }

    #[test]
    fn raising_twice_for_one_source_is_one_alert() {
        let mut a = Alerts::default();
        let id = a.raise(t0(), Level::Alarm, Source::Anchor, "Anchor alarm: 0.10 nm", None);
        let again = a.raise(t0(), Level::Alarm, Source::Anchor, "Anchor alarm: 0.12 nm", None);
        assert_eq!(id, again);
        assert_eq!(a.all().len(), 1);
        assert_eq!(a.get(id).unwrap().message, "Anchor alarm: 0.12 nm");
    }

    #[test]
    fn the_banner_shows_the_newest_alarm_and_counts_them() {
        let mut a = Alerts::default();
        a.raise(t0(), Level::Alarm, Source::Anchor, "anchor", None);
        a.raise(t0(), Level::Alarm, Source::Collision(1), "collision", Some(0.2));
        a.raise(t0(), Level::Warning, Source::OffCourse, "off course", None);
        let b = a.banner().unwrap();
        assert_eq!(b.alert.message, "collision");
        assert_eq!((b.count, b.dim), (2, false));
        assert_eq!(a.warnings().len(), 1, "a warning is not an alarm");
    }

    #[test]
    fn an_acknowledged_alarm_goes_dim_until_the_condition_clears() {
        let mut a = Alerts::default();
        let id = a.raise(t0(), Level::Alarm, Source::Anchor, "anchor", None);
        a.acknowledge(id);
        let b = a.banner().unwrap();
        assert!(b.dim && b.count == 0, "still shown, but quiet");
        assert!(!a.alarm_sound_due(t0()));
        a.clear(&Source::Anchor);
        assert!(a.banner().is_none());
    }

    #[test]
    fn the_sound_repeats_every_four_seconds_until_acknowledged() {
        let mut a = Alerts::default();
        let id = a.raise(t0(), Level::Alarm, Source::Collision(7), "x", Some(0.1));
        assert!(a.alarm_sound_due(t0()));
        assert!(!a.alarm_sound_due(t0() + Duration::seconds(2)));
        assert!(a.alarm_sound_due(t0() + Duration::seconds(4)));
        a.acknowledge(id);
        assert!(!a.alarm_sound_due(t0() + Duration::seconds(10)));
    }

    #[test]
    fn snoozing_is_two_minutes_for_collisions_and_five_for_the_anchor() {
        let mut a = Alerts::default();
        let c = a.raise(t0(), Level::Alarm, Source::Collision(1), "c", Some(0.2));
        let n = a.raise(t0(), Level::Alarm, Source::Anchor, "a", None);
        a.snooze(c, t0());
        a.snooze(n, t0());
        assert!(!a.alarm_sound_due(t0() + Duration::seconds(1)));
        a.tick(t0() + Duration::minutes(2) + Duration::seconds(1));
        assert!(a.get(c).unwrap().is_active(), "the collision comes back");
        assert!(!a.get(n).unwrap().is_active(), "the anchor is still snoozed");
        a.tick(t0() + Duration::minutes(5) + Duration::seconds(1));
        assert!(a.get(n).unwrap().is_active());
    }

    #[test]
    fn an_acknowledged_collision_only_comes_back_when_the_cpa_halves() {
        let mut a = Alerts::default();
        let id = a.raise(t0(), Level::Alarm, Source::Collision(9), "c", Some(0.4));
        a.acknowledge(id);
        a.raise(t0(), Level::Alarm, Source::Collision(9), "c", Some(0.3));
        assert_eq!(a.get(id).unwrap().state, State::Acknowledged, "a little closer is not enough");
        a.raise(t0(), Level::Alarm, Source::Collision(9), "c", Some(0.19));
        assert_eq!(a.get(id).unwrap().state, State::Active, "under half of 0.4 nm");
    }

    #[test]
    fn warnings_can_be_dismissed_and_info_expires() {
        let mut a = Alerts::default();
        let w = a.raise(t0(), Level::Warning, Source::StaleForecast, "stale", None);
        a.raise(t0(), Level::Info, Source::Event("Arrived at A".into()), "Arrived at A", None);
        assert_eq!(a.messages().len(), 1);
        a.dismiss(w);
        assert!(a.warnings().is_empty());
        a.tick(t0() + Duration::seconds(5));
        assert!(a.messages().is_empty(), "info lives toast-duration");
        assert_eq!(a.all().len(), 1, "the dismissed warning is kept hidden, so that it does not come straight back");
    }

    #[test]
    fn a_warning_chimes_once() {
        let mut a = Alerts::default();
        a.raise(t0(), Level::Warning, Source::StaleForecast, "stale", None);
        assert_eq!(a.take_new_warnings(), 1);
        assert_eq!(a.take_new_warnings(), 0);
        a.raise(t0(), Level::Warning, Source::OffCourse, "off", None);
        assert_eq!(a.take_new_warnings(), 1);
    }

    #[test]
    fn alerts_offer_the_actions_of_their_level() {
        let mut a = Alerts::default();
        a.raise(t0(), Level::Alarm, Source::Anchor, "x", None);
        a.raise(t0(), Level::Warning, Source::OffCourse, "y", None);
        assert_eq!(a.all()[0].actions(), [Action::Acknowledge, Action::Snooze]);
        assert_eq!(a.all()[1].actions(), [Action::Dismiss]);
        assert!(Level::Alarm > Level::Warning && Level::Warning > Level::Info && Level::Info > Level::Hint);
    }
}
