//! Vessels nearby, nearest first.

use crate::app::AppState;
use crate::chips::{Chip, Icon};
use crate::core::comm::ais::AisClass;
use crate::core::comm::cpa::{Motion, cpa};
use crate::core::nav::calc::format_range_bearing;
use crate::core::session::Session;
use crate::i18n::gettext_f;
use serde::Serialize;

/// What a row shows: `(mmsi, title, subtitle, alert)`.
pub type RowData = (u32, String, Vec<Chip>, bool);

pub fn rows(s: &Session, limit: usize) -> Vec<RowData> {
    let Some(own_pos) = s.own.pos else {
        return Vec::new();
    };
    let own = s.own.sog_kn.zip(s.own.cog).map(|(sog_kn, cog)| Motion { pos: own_pos, sog_kn, cog });
    let alerts = s.ais_alerts();
    s.targets
        .by_distance(&own_pos)
        .into_iter()
        .filter(|(t, _)| t.class != AisClass::Aton && t.class != AisClass::Base)
        .take(limit)
        .map(|(t, range)| {
            let pos = t.pos.expect("by_distance only returns targets with a position");
            let name = t.name.clone().unwrap_or_else(|| t.mmsi.to_string());
            let brg = own_pos.bearing_to(&pos);
            let rb = format_range_bearing(range, brg);
            let mut sub = vec![Chip::new(Icon::Distance, rb.clone(), gettext_f("{range} away", &[("range", &rb)]))];
            if let (Some(sog), Some(cog)) = (t.sog_kn, t.cog) {
                let (sog, cog) = (format!("{sog:.1}"), format!("{cog:03.0}"));
                sub.push(Chip::new(Icon::Speed, format!("{sog} kn {cog}°"), gettext_f("making {sog} kn on {cog}°", &[("sog", &sog), ("cog", &cog)])));
            }
            if let (Some(o), Some(sog), Some(cog)) = (own.as_ref(), t.sog_kn, t.cog)
                && sog >= 0.3
            {
                let c = cpa(o, &Motion { pos, sog_kn: sog, cog });
                if c.tcpa_min >= 0.0 && c.tcpa_min < 60.0 {
                    let text = gettext_f("CPA {cpa} nm in {t} min", &[("cpa", &format!("{:.2}", c.cpa_nm)), ("t", &format!("{:.0}", c.tcpa_min))]);
                    sub.push(Chip::new(Icon::Alarm, text.clone(), gettext_f("closest approach: {cpa}", &[("cpa", &text)])));
                }
            }
            (t.mmsi, name, sub, alerts.iter().any(|(m, _)| *m == t.mmsi))
        })
        .collect()
}

/// A vessel as the Traffic page lists it.
#[derive(Serialize, Debug, PartialEq)]
pub struct Vessel {
    pub mmsi: u32,
    pub name: String,
    pub detail: Vec<Chip>,
    pub alert: bool,
}

pub fn vessels(app: &AppState, limit: usize) -> Vec<Vessel> {
    rows(&app.session.borrow(), limit).into_iter().map(|(mmsi, name, detail, alert)| Vessel { mmsi, name, detail, alert }).collect()
}

/// Selects a vessel and centres the chart on it.
pub fn show_vessel(app: &AppState, mmsi: u32) {
    let pos = app.session.borrow().targets.get(mmsi).and_then(|t| t.pos);
    app.select(crate::app::Selection::Target(mmsi));
    if let Some(p) = pos {
        app.canvas.center_on(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::comm::sim::Simulator;
    use crate::core::comm::source::{Event, Payload};
    use crate::core::geo::LatLon;
    use crate::core::nav::store::NavStore;
    use chrono::Utc;

    #[test]
    fn no_position_no_list() {
        let s = Session::new(NavStore::open_memory().unwrap()).unwrap();
        assert!(rows(&s, 10).is_empty());
    }

    #[test]
    fn demo_traffic_is_listed_nearest_first_with_an_alert_for_the_ferry() {
        let mut s = Session::new(NavStore::open_memory().unwrap()).unwrap();
        let mut sim = Simulator::demo(LatLon::new(48.35, -4.55), 90.0, 6.0, Utc::now());
        for _ in 0..40 {
            let ev: Vec<Event> = sim.step(1.0).into_iter().map(|l| Event { source: "s".into(), payload: Payload::Line(l) }).collect();
            s.process(&ev, Utc::now());
        }
        let r = rows(&s, 10);
        assert_eq!(r.len(), 3);
        // Sorted by distance.
        let d = |sub: &[Chip]| sub[0].text.split(' ').next().unwrap().parse::<f64>().unwrap();
        assert!(d(&r[0].2) <= d(&r[2].2) || r[0].2[0].text.contains(" m "));
        let ferry = r.iter().find(|x| x.0 == 227_100_001).unwrap();
        assert!(ferry.3, "the crossing ferry is an alert");
        assert!(ferry.2.iter().any(|c| c.icon == Some(Icon::Alarm) && c.text.contains("CPA")));
        // The limit applies.
        assert_eq!(rows(&s, 1).len(), 1);
    }
}
