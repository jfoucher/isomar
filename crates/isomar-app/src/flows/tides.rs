//! "Tides here…": the next high and low waters at the nearest station, and the curve between them.

use crate::app::AppState;
use crate::core::geo::LatLon;
use crate::core::tides::{Forecast, Origin, TideError, cache_dir, lookup};
use crate::i18n::{gettext, gettext_f};
use crate::platform::bg;
use chrono::{DateTime, Duration, Local, Utc};
use std::rc::Rc;

/// An upcoming high or low water, for a row of its own: an arrow, the time, the height.
#[derive(serde::Serialize, Clone, Debug, PartialEq)]
pub struct Extreme {
    pub high: bool,
    /// "High water", for the arrow's tooltip and screen readers.
    pub name: String,
    /// "Mon 15:54".
    pub time: String,
    /// "6.9 m".
    pub height: String,
}

/// The upcoming extremes, at most `max`.
pub fn extremes(f: &Forecast, now: DateTime<Utc>, max: usize) -> Vec<Extreme> {
    f.upcoming(now)
        .take(max)
        .map(|e| Extreme {
            high: e.high,
            name: if e.high { gettext("High water") } else { gettext("Low water") },
            time: e.time.with_timezone(&Local).format("%a %H:%M").to_string(),
            height: format!("{:.1} m", e.level_m),
        })
        .collect()
}

pub fn explain(e: &TideError) -> String {
    match e {
        TideError::NoStation => gettext("There is no tide station near this position."),
        TideError::Network(m) => gettext_f("Could not reach the tide service: {error}", &[("error", m)]),
        TideError::Format(m) => gettext_f("The tide service answered something unexpected: {error}", &[("error", m)]),
    }
}

/// What to say about the tide here: a title, groups of labelled rows, and the height curve.
#[derive(serde::Serialize, Clone, Debug, PartialEq)]
pub struct TideInfo {
    pub title: String,
    /// The station and the height now, as labelled rows.
    pub station: Vec<(String, String)>,
    /// "Next", over the extremes.
    pub next_title: String,
    pub extremes: Vec<Extreme>,
    /// The rest, after the extremes: groups of labelled rows.
    pub groups: Vec<(String, Vec<(String, String)>)>,
    /// (seconds since the epoch, metres) every half hour, from six hours ago to a day and a half ahead.
    pub curve: Vec<(i64, f64)>,
    pub now: i64,
    pub datum: String,
}

/// The tide information for a forecast looked up at `now`.
pub fn describe(f: &Forecast, origin: Origin, now: DateTime<Utc>) -> TideInfo {
    let mut groups = Vec::new();
    let mut station = vec![(gettext("Station"), gettext_f("{name}, {d} nm away", &[("name", &f.station.name), ("d", &format!("{:.1}", f.station.distance_nm))]))];
    if let Some(h) = f.height_at(now) {
        let trend = match f.rising_at(now) {
            Some(true) => gettext("rising"),
            Some(false) => gettext("falling"),
            None => String::new(),
        };
        station.push((gettext("Now"), format!("{h:.1} m {trend}").trim().to_string()));
    }
    let (next_title, extremes) = (gettext("Next"), extremes(f, now, 6));
    if origin == Origin::StoredFar {
        groups.push((gettext("No network"), vec![(gettext("Using"), gettext("The nearest station kept from an earlier lookup. It may differ from the tide where you are."))]));
    }
    let mut about = vec![
        (gettext("Heights"), gettext_f("Metres above {datum}", &[("datum", &f.station.datum)])),
        (gettext("Source"), f.station.attribution.clone()),
    ];
    if !f.station.commercial_use {
        about.push((gettext("Licence"), gettext("This station's data is licensed for non-commercial use only.")));
    }
    if origin != Origin::Network {
        about.push((gettext("Kept"), gettext("Predictions for a month ahead are kept on this computer, so they work without a network.")));
    }
    about.push((gettext("Use"), gettext("Harmonic prediction for planning. Not for navigation: weather and pressure change the real tide.")));
    groups.push((gettext("About this prediction"), about));
    let curve = (-12..=72).filter_map(|k| {
        let t = now + Duration::minutes(30 * k);
        f.height_at(t).map(|h| (t.timestamp(), h))
    });
    TideInfo { title: f.station.name.clone(), station, next_title, extremes, groups, curve: curve.collect(), now: now.timestamp(), datum: f.station.datum.clone() }
}

/// Looks up the tide at `pos` (for 40 hours, from the network or what is kept). Failures are told to the shell.
pub async fn info(app: &Rc<AppState>, pos: LatLon) -> Option<TideInfo> {
    app.toast(&gettext("Looking up the tides…"));
    let now = Utc::now();
    let result = bg(move || lookup(&cache_dir(), pos.lat, pos.lon, now, 40)).await;
    match result {
        Ok((f, origin)) => Some(describe(&f, origin, now)),
        Err(e) => {
            app.show_error(&gettext("No tide information"), &explain(&e));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tides::parse;

    #[test]
    fn lines_list_the_next_extremes_in_order() {
        let f = parse(r#"{"station":{"latitude":1,"longitude":2,"name":"X"},"extremes":[
            {"time":"2026-10-26T03:00:00Z","level":6.0,"high":true},
            {"time":"2026-10-26T09:00:00Z","level":1.0,"high":false},
            {"time":"2026-10-26T15:00:00Z","level":6.5,"high":true}]}"#)
        .unwrap();
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-26T05:00:00Z").unwrap().with_timezone(&Utc);
        let l = extremes(&f, now, 5);
        assert_eq!(l.len(), 2);
        assert_eq!((l[0].high, l[0].name.as_str()), (false, "Low water"));
        assert_eq!((l[0].height.as_str(), l[1].height.as_str()), ("1.0 m", "6.5 m"));
        assert_eq!(extremes(&f, now, 1).len(), 1);
    }

    #[test]
    fn errors_are_plain() {
        assert!(explain(&TideError::NoStation).contains("tide station"));
        assert!(explain(&TideError::Network("timeout".into())).contains("timeout"));
    }
}
