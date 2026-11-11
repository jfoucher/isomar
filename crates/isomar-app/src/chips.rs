//! Values in a line, each with the icon that says what it is: "[satellite] 8", "[engine] 3 h".
//!
//! The pages used to join values with middots in one string. A shell draws a chip as an icon and its
//! `text`; `label` is the whole phrase ("8 satellites", "3 h under engine"), for tooltips, screen
//! readers and the places that only take text (toasts, tooltips of their own).

use serde::{Deserialize, Serialize};

/// What a value is. Each shell maps it to an icon of its own set (`ui::chips` on GTK, `Chips.swift`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum Icon {
    Satellite,
    Speed,
    Distance,
    Points,
    Time,
    Arrival,
    Date,
    Wind,
    Gusts,
    Waves,
    /// A tidal stream or ocean current.
    Current,
    Pressure,
    Temperature,
    Engine,
    Depth,
    Compass,
    Position,
    Zoom,
    Size,
    Licence,
    Warning,
    Alarm,
    Upwind,
    Night,
    Tack,
    Waiting,
    Forecast,
    Source,
    High,
    Low,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Chip {
    /// None: plain text in the line ("No GPS fix").
    pub icon: Option<Icon>,
    /// What stands beside the icon.
    pub text: String,
    /// The whole phrase, which says what the icon does.
    pub label: String,
}

impl Chip {
    pub fn new(icon: Icon, text: impl Into<String>, label: impl Into<String>) -> Self {
        Chip { icon: Some(icon), text: text.into(), label: label.into() }
    }

    /// A chip whose text says it all, icon or not.
    pub fn with(icon: Icon, text: impl Into<String>) -> Self {
        let text = text.into();
        Chip { icon: Some(icon), label: text.clone(), text }
    }

    pub fn plain(text: impl Into<String>) -> Self {
        let text = text.into();
        Chip { icon: None, label: text.clone(), text }
    }
}

/// Chips sent through the api as JSON (`"chips"` of `route_get`), read back.
pub fn from_json(v: &serde_json::Value) -> Vec<Chip> {
    serde_json::from_value(v.clone()).unwrap_or_default()
}

/// The chips as one line of text: their labels, joined with commas.
pub fn plain(chips: &[Chip]) -> String {
    chips.iter().map(|c| c.label.as_str()).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chip_is_its_icon_text_and_label_in_json() {
        let c = Chip::new(Icon::Satellite, "8", "8 satellites");
        assert_eq!(serde_json::to_value(&c).unwrap(), serde_json::json!({ "icon": "satellite", "text": "8", "label": "8 satellites" }));
        assert_eq!(serde_json::to_value(Chip::plain("No GPS fix")).unwrap()["icon"], serde_json::Value::Null);
    }

    #[test]
    fn plain_text_is_the_labels_with_commas() {
        let cs = [Chip::plain("No GPS fix"), Chip::new(Icon::Satellite, "8", "8 satellites"), Chip::plain("")];
        assert_eq!(plain(&cs), "No GPS fix, 8 satellites");
    }
}
