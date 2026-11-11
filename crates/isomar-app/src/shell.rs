//! What the engine asks of the windowing shell: messages, confirmations,
//! menus and the dialogs it does not draw itself. GTK and macOS each implement
//! it. Methods are called on the main thread.

use crate::aidinfo::Description;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

/// One entry of a context menu.
#[derive(Clone)]
pub struct MenuItem {
    pub label: String,
    pub run: Rc<dyn Fn()>,
}

/// A context menu: groups of entries, with a separator between the groups.
#[derive(Clone, Default)]
pub struct Menu {
    /// Where it was opened: the position, which a touch screen's actions sheet shows as its heading.
    pub title: String,
    pub sections: Vec<Vec<MenuItem>>,
}

impl Menu {
    pub fn is_empty(&self) -> bool {
        self.sections.iter().all(|s| s.is_empty())
    }
}

/// A dialog or window the shell draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dialog {
    WaypointProperties(i64),
    RouteProperties(i64),
    AisInfo(u32),
    WeatherRoute(i64),
    BestDeparture(i64),
    AutoRoute(i64),
    /// "Get weather" for an area.
    GetWeather { north: f64, south: f64, west: f64, east: f64 },
    /// The tides at a position.
    Tides(crate::core::geo::LatLon),
    /// What chart sources exist for a position.
    ChartsHere(crate::core::geo::LatLon),
}

/// The sounds the app makes. The alarm repeats until it is acknowledged; the chime is one warning's;
/// the tick is the soft one for an arrival.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    Alarm,
    Chime,
    Tick,
}

pub trait Shell {
    /// A short message that goes away by itself.
    fn toast(&self, text: &str);
    /// An error with details, to be dismissed.
    fn show_error(&self, heading: &str, details: &str);
    /// Plays a sound once.
    fn sound(&self, sound: Sound);
    /// Asks the user; resolves to whether they accepted.
    fn confirm(&self, heading: &str, body: &str, ok_label: &str, destructive: bool) -> Pin<Box<dyn Future<Output = bool>>>;
    /// Pops the menu up at (`x`, `y`) on the chart.
    fn context_menu(&self, menu: Menu, x: f64, y: f64);
    /// Pops up what a mark on the chart is, anchored at (`x`, `y`).
    fn object_info(&self, info: &Description, x: f64, y: f64);
    /// Opens a dialog for something on the chart. A shell that has no such dialog says so.
    fn open(&self, dialog: Dialog);
    /// The pointer is over something a click opens (`true`), or no longer. A shell with no cursor to change leaves it.
    fn set_cursor(&self, _pointing: bool) {}
    /// A long calculation has ended (`AppState::finished`). The shell sends a system notification
    /// only while the app is out of sight: in front, the result is already on screen.
    fn notify(&self, _title: &str, _body: &str) {}
}
