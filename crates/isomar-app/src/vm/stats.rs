//! The statistics of the alternatives of a weather-route search, side by side: the table, a column
//! for each alternative, in sections.

use crate::core::nav::calc::format_duration;
use crate::core::routing::alternatives::Alternative;
use crate::core::routing::stats::{PointOfSail, Stats, WIND_BANDS_KN};
use crate::i18n::{gettext, gettext_f};
use chrono::{DateTime, Duration, Local, Utc};
use serde::Serialize;

/// A group of rows under one heading; each row has a cell per alternative.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Section {
    pub title: String,
    pub rows: Vec<(String, Vec<String>)>,
}

impl Section {
    /// The same rows with every cell but the last (a row's summary) made by `f(row, column)`.
    pub fn with_cells(mut self, f: impl Fn(usize, usize) -> String) -> Section {
        for (i, (_, cells)) in self.rows.iter_mut().enumerate() {
            let n = cells.len().saturating_sub(1);
            for (j, cell) in cells.iter_mut().take(n).enumerate() {
                *cell = f(i, j);
            }
        }
        self
    }
}

fn hours(h: f64) -> String {
    format_duration(Duration::seconds((h * 3600.0).round() as i64))
}

/// Time spent, as a share of the passage and in hours; a dash for none.
fn share(h: f64, total: f64) -> String {
    if h < 0.05 || total <= 0.0 {
        return "–".into();
    }
    gettext_f("{pct} % ({time})", &[("pct", &format!("{:.0}", 100.0 * h / total)), ("time", &hours(h))])
}

fn count(n: u32) -> String {
    if n == 0 { "–".into() } else { n.to_string() }
}

fn local(t: DateTime<Utc>) -> String {
    t.with_timezone(&Local).format("%a %d %H:%M").to_string()
}

fn point_name(p: PointOfSail) -> String {
    match p {
        PointOfSail::CloseHauled => gettext("Close-hauled (under 55°)"),
        PointOfSail::CloseReach => gettext("Close reach (55–85°)"),
        PointOfSail::BeamReach => gettext("Beam reach (85–115°)"),
        PointOfSail::BroadReach => gettext("Broad reach (115–150°)"),
        PointOfSail::Run => gettext("Running (over 150°)"),
    }
}

fn band_name(i: usize) -> String {
    let kn = |k: f64| format!("{k:.0}");
    match i {
        0 => gettext_f("Under {a} kn", &[("a", &kn(WIND_BANDS_KN[0]))]),
        i if i == WIND_BANDS_KN.len() => gettext_f("Over {a} kn", &[("a", &kn(WIND_BANDS_KN[i - 1]))]),
        i => gettext_f("{a} to {b} kn", &[("a", &kn(WIND_BANDS_KN[i - 1])), ("b", &kn(WIND_BANDS_KN[i]))]),
    }
}

/// The table for `alts`, one column per alternative.
pub fn sections(alts: &[&Alternative]) -> Vec<Section> {
    let stats: Vec<Stats> = alts.iter().map(|a| a.stats()).collect();
    let col = |f: &dyn Fn(&Stats) -> String| stats.iter().map(f).collect::<Vec<_>>();
    let per_alt = |f: &dyn Fn(&Alternative, &Stats) -> String| alts.iter().zip(&stats).map(|(a, s)| f(a, s)).collect::<Vec<_>>();
    let mut out = Vec::new();

    let passage = vec![
        (gettext("Arrival or end"), per_alt(&|a, _| if a.arrives { local(a.end) } else { gettext_f("{time} (forecast ends)", &[("time", &local(a.end))]) })),
        (gettext("Duration"), col(&|s| hours(s.hours))),
        (gettext("Distance sailed"), col(&|s| gettext_f("{nm} nm", &[("nm", &format!("{:.0}", s.distance_nm))]))),
        (gettext("Still to go"), per_alt(&|a, _| if a.arrives { "–".into() } else { gettext_f("{nm} nm", &[("nm", &format!("{:.0}", a.remaining_nm))]) })),
        (gettext("Longer than direct"), per_alt(&|a, s| if a.arrives && s.direct_nm >= 1.0 { format!("{:+.0} %", s.extra_percent()) } else { "–".into() })),
        (gettext("Average speed over ground"), col(&|s| gettext_f("{kn} kn", &[("kn", &format!("{:.1}", s.avg_sog_kn))]))),
        (gettext("Top speed over ground"), col(&|s| gettext_f("{kn} kn", &[("kn", &format!("{:.1}", s.max_sog_kn))]))),
    ];
    out.push(Section { title: gettext("Passage"), rows: passage });

    let mut wind = vec![
        (
            gettext("Strongest wind"),
            stats
                .iter()
                .map(|s| match s.max_tws_at {
                    Some(at) => gettext_f("{kn} kn, {time}", &[("kn", &format!("{:.0}", s.max_tws_kn)), ("time", &local(at))]),
                    None => "–".into(),
                })
                .collect(),
        ),
        (gettext("Average wind"), col(&|s| gettext_f("{kn} kn", &[("kn", &format!("{:.0}", s.mean_tws_kn))]))),
    ];
    for i in 0..=WIND_BANDS_KN.len() {
        wind.push((band_name(i), col(&|s| share(s.wind_hours[i], s.hours))));
    }
    out.push(Section { title: gettext("Wind"), rows: wind });

    let mut sailing: Vec<(String, Vec<String>)> = PointOfSail::ALL.iter().enumerate().map(|(i, p)| (point_name(*p), col(&|s| share(s.sail_hours[i], s.hours)))).collect();
    sailing.push((gettext("Under engine"), col(&|s| share(s.motor_hours, s.hours))));
    out.push(Section { title: gettext("Time on each point of sail"), rows: sailing });

    // Only for routes that met a current, so a route sailed without currents keeps its table.
    if stats.iter().any(|s| s.has_current) {
        let kn = |v: f64| gettext_f("{kn} kn", &[("kn", &format!("{v:.1}"))]);
        out.push(Section {
            title: gettext("Current"),
            rows: vec![
                (gettext("With the boat"), col(&|s| share(s.fair_hours, s.hours))),
                (gettext("Against the boat"), col(&|s| share(s.foul_hours, s.hours))),
                (
                    gettext("Gained from the current"),
                    col(&|s| if s.current_gain_nm.abs() < 0.05 { "–".into() } else { gettext_f("{nm} nm", &[("nm", &format!("{:+.1}", s.current_gain_nm))]) }),
                ),
                (
                    gettext("Strongest current"),
                    col(&|s| match s.max_current_at {
                        Some(at) => gettext_f("{kn}, {time}", &[("kn", &kn(s.max_current_kn)), ("time", &local(at))]),
                        None => "–".into(),
                    }),
                ),
                (gettext("Wind against tide"), col(&|s| share(s.wind_against_tide_hours, s.hours))),
                (gettext("Waiting for the stream"), col(&|s| share(s.hold_hours, s.hours))),
            ],
        });
    }

    out.push(Section {
        title: gettext("Other"),
        rows: vec![
            (gettext("Tacks"), col(&|s| count(s.tacks))),
            (gettext("Gybes"), col(&|s| count(s.gybes))),
            (gettext("In the dark (sun more than 6° down)"), col(&|s| share(s.dark_hours, s.hours))),
        ],
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::geo::LatLon;
    use crate::core::routing::isochrone::Node;

    fn alt(arrives: bool) -> Alternative {
        let t = chrono::Utc::now();
        let node = |lat: f64, h: i64| Node { pos: LatLon::new(lat, -5.0), time: t + Duration::hours(h), cog: 0.0, sog_kn: 5.0, heading: 0.0, tws_kn: 12.0, twd: 45.0, motoring: h == 2, current: (0.0, 0.0) };
        Alternative { path: vec![node(48.0, 0), node(48.1, 1), node(48.2, 2)], distance_nm: 12.0, remaining_nm: if arrives { 0.0 } else { 40.0 }, motor_hours: 1.0, max_tws_kn: 12.0, end: t + Duration::hours(2), arrives }
    }

    fn cells<'a>(sections: &'a [Section], label: &str) -> &'a [String] {
        &sections.iter().flat_map(|s| &s.rows).find(|(l, _)| l == label).expect(label).1
    }

    #[test]
    fn every_row_has_a_cell_per_route() {
        let (a, b) = (alt(true), alt(false));
        let sections = sections(&[&a, &b]);
        assert_eq!(sections.len(), 4);
        for s in &sections {
            for (label, cells) in &s.rows {
                assert_eq!(cells.len(), 2, "{label}");
            }
        }
    }

    #[test]
    fn what_is_left_to_go_and_the_detour_only_make_sense_for_the_right_routes() {
        let (a, b) = (alt(true), alt(false));
        let sections = sections(&[&a, &b]);
        assert_eq!(cells(&sections, "Still to go"), ["–", "40 nm"]);
        assert!(cells(&sections, "Longer than direct")[0].contains('%') && cells(&sections, "Longer than direct")[1] == "–");
        assert_eq!(cells(&sections, "Under engine")[0], "50 % (1 h 00 min)");
    }

    #[test]
    fn a_route_that_met_a_current_gets_a_current_section() {
        let mut a = alt(true);
        assert!(!sections(&[&a]).iter().any(|s| s.title == "Current"));
        a.path[1].current = (0.0, 2.0);
        let sections = sections(&[&a]);
        assert!(sections.iter().any(|s| s.title == "Current"));
        assert_eq!(cells(&sections, "With the boat")[0], "50 % (1 h 00 min)");
        assert_eq!(cells(&sections, "Gained from the current")[0], "+2.0 nm");
    }

    #[test]
    fn a_share_is_a_percentage_and_a_time_or_a_dash() {
        assert_eq!(share(2.0, 10.0), "20 % (2 h 00 min)");
        assert_eq!(share(0.0, 10.0), "–");
        assert_eq!(share(1.0, 0.0), "–");
    }

    #[test]
    fn the_wind_bands_are_named_by_their_limits() {
        assert_eq!(band_name(0), "Under 10 kn");
        assert_eq!(band_name(1), "10 to 20 kn");
        assert_eq!(band_name(3), "Over 30 kn");
    }
}
