//! The strips of alert chrome: the alarm banner under the toolbar, warnings below it, and the hint
//! card while a mode wants one. Built from `core::alerts`; the shells draw them.

use crate::actions;
use crate::app::AppState;
use crate::core::alerts::{Action, Alert, Level, State};
use crate::i18n::{gettext, gettext_f};
use serde::Serialize;
use std::rc::Rc;

/// An alarm or warning as the chrome shows it.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Strip {
    pub id: u64,
    /// `alarm` or `warning`.
    pub level: String,
    pub title: String,
    /// The labels of the buttons it offers, if any.
    pub acknowledge: Option<String>,
    pub snooze: Option<String>,
    pub dismiss: Option<String>,
    /// A button that deals with what the alert is about (`press_action`): "Switch to Motoring".
    pub action: Option<String>,
    /// How many alarms are active: over one, the strip ends with "[alarm] 2".
    pub count: usize,
    /// "2 alarms", when there are several: the count's tooltip, and the notification's words.
    pub count_label: Option<String>,
    /// Acknowledged or snoozed: shown dim.
    pub dim: bool,
}

/// A hint is a card, never a banner: what to do in the mode the user is in.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Hint {
    pub title: String,
    pub finish: Option<String>,
    pub undo: Option<String>,
    pub cancel: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Alerts {
    pub alarm: Option<Strip>,
    pub warnings: Vec<Strip>,
    pub hint: Option<Hint>,
}

fn strip(a: &Alert, count: usize, dim: bool) -> Strip {
    let has = |x| a.actions().contains(&x);
    let title = a.message.clone();
    let count_label = (count > 1).then(|| gettext_f("{n} alarms", &[("n", &count.to_string())]));
    Strip {
        id: a.id,
        level: if a.level == Level::Alarm { "alarm" } else { "warning" }.into(),
        title,
        acknowledge: (has(Action::Acknowledge) && !dim).then(|| gettext("Acknowledge")),
        snooze: (has(Action::Snooze) && !dim).then(|| gettext("Snooze")),
        dismiss: has(Action::Dismiss).then(|| gettext("Dismiss")),
        action: (a.source == crate::core::alerts::Source::LooksMotored).then(|| gettext("Switch to Motoring")),
        count,
        count_label,
        dim,
    }
}

/// What the chrome shows now.
pub fn alerts(app: &AppState) -> Alerts {
    let s = app.session.borrow();
    let alarm = s.alerts.banner().map(|b| strip(b.alert, b.count, b.dim));
    let warnings = s.alerts.warnings().into_iter().filter(|a| a.state != State::Acknowledged || true).map(|a| strip(a, 0, false)).collect();
    drop(s);
    let hint = app.edit.borrow().as_ref().map(|e| {
        let n = e.route.points.len();
        let touch = app.canvas.is_touch();
        Hint {
            title: match (n, touch) {
                (0, false) => gettext("Drawing a route: click to add points"),
                (0, true) => gettext("Drawing a route: tap to add points"),
                (_, false) => gettext_f("Drawing a route ({n} points): click to add, double-click to finish", &[("n", &n.to_string())]),
                (_, true) => gettext_f("Drawing a route ({n} points): tap to add, double-tap to finish", &[("n", &n.to_string())]),
            },
            finish: Some(gettext("Finish")),
            undo: (n > 0).then(|| gettext("Undo")),
            cancel: Some(gettext("Cancel")),
        }
    });
    Alerts { alarm, warnings, hint }
}

/// The alarm banner's strip, or the first warning's, for shells that show one line.
pub fn banner(app: &AppState) -> Option<Strip> {
    let a = alerts(app);
    a.alarm.or_else(|| a.warnings.into_iter().next())
}

/// A button on a strip was pressed.
pub fn press(app: &Rc<AppState>, id: u64, action: Action) {
    {
        let mut s = app.session.borrow_mut();
        match action {
            Action::Acknowledge => s.acknowledge_alert(id),
            Action::Snooze => {
                let now = chrono::Utc::now();
                s.alerts.snooze(id, now);
            }
            Action::Dismiss => s.alerts.dismiss(id),
        }
    }
    app.refresh();
}

/// The strip's own action was pressed (`Strip::action`).
pub fn press_action(app: &Rc<AppState>, id: u64) {
    let source = app.session.borrow().alerts.get(id).map(|a| a.source.clone());
    if source == Some(crate::core::alerts::Source::LooksMotored) {
        crate::flows::perf::set_mode(app, crate::core::perf::SailMode::Motoring);
    }
    app.refresh();
}

/// A button on the hint card: `finish`, `undo` or `cancel`.
pub fn press_hint(app: &Rc<AppState>, which: &str) {
    match which {
        "finish" => actions::finish_edit(app),
        "undo" => actions::remove_last_edit_point(app),
        _ => actions::cancel_edit(app),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::geo::LatLon;
    use crate::core::nav::store::NavStore;
    use crate::core::session::Session;
    use crate::engine::Engine;
    use crate::palette::Mode;

    fn app() -> Rc<AppState> {
        let canvas = Engine::new(crate::config::ViewState { lat: 48.0, lon: -4.0, zoom: 10.0 }, Mode::Day);
        AppState::new(canvas, Session::new(NavStore::open_memory().unwrap()).unwrap(), crate::config::Settings::default())
    }

    #[test]
    fn drawing_a_route_is_a_hint_not_an_alarm() {
        let app = app();
        assert_eq!(alerts(&app), Alerts::default());
        actions::start_route(&app, Some(LatLon::new(48.0, -4.0)));
        let a = alerts(&app);
        assert!(a.alarm.is_none() && a.warnings.is_empty());
        let hint = a.hint.unwrap();
        assert!(hint.title.contains("1 points"), "{}", hint.title);
        assert!(hint.undo.is_some() && hint.finish.is_some());
    }

    #[test]
    fn two_alarms_stack_and_acknowledging_dims_the_banner() {
        let app = app();
        let now = chrono::Utc::now();
        let (a, b);
        {
            let mut s = app.session.borrow_mut();
            a = s.alerts.raise(now, Level::Alarm, crate::core::alerts::Source::Anchor, "Anchor alarm", None);
            b = s.alerts.raise(now, Level::Alarm, crate::core::alerts::Source::ShallowWater, "Shallow water", None);
        }
        let strip = alerts(&app).alarm.unwrap();
        assert_eq!(strip.id, b, "newest first");
        assert_eq!((strip.count, strip.count_label.as_deref()), (2, Some("2 alarms")));
        press(&app, b, Action::Acknowledge);
        press(&app, a, Action::Acknowledge);
        let dim = alerts(&app).alarm.unwrap();
        assert!(dim.dim && dim.acknowledge.is_none() && dim.snooze.is_none());
    }
}
