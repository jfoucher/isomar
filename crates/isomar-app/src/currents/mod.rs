//! The currents loaded, for the chart, the pointer readout and the router: one store per gridded source
//! and the tidal atlases installed, composed into one `CurrentField` that every wind model shares.
//! Held in `WeatherState` (it follows its time and has no clock of its own).

pub mod atlases;
pub mod layer;

use crate::chips::{Chip, Icon};
use crate::core::currents::field::Tidal;
use crate::core::currents::{CurrentField, CurrentSource};
use crate::core::geo::{self, LatLon};
use crate::core::weather::model::Field;
use crate::core::weather::store::WeatherStore;
use crate::i18n::gettext_f;
use chrono::{DateTime, Utc};
use std::collections::BTreeMap;
use std::sync::Arc;

pub struct CurrentsState {
    stores: BTreeMap<CurrentSource, WeatherStore>,
    tides: Vec<Arc<dyn Tidal>>,
    /// Everything above composed, rebuilt when something loads.
    field: Arc<CurrentField>,
    /// Bumped whenever `field` is rebuilt, for the layer to know.
    pub revision: u64,
    /// Arrows on the chart.
    pub arrows: bool,
    /// Weather routes sail in the current (`config::CurrentPrefs::route_with`).
    pub route_with: bool,
    /// The Shom atlas zones the website offers, and those installed (`atlases`).
    pub zones: Vec<crate::core::currents::atlas::Zone>,
    pub installed: Vec<String>,
    /// The stream the boat measures, and each source against it (`perf::current_check`).
    pub tracker: crate::core::perf::current_check::CurrentTracker,
}

impl Default for CurrentsState {
    fn default() -> Self {
        CurrentsState { stores: BTreeMap::new(), tides: Vec::new(), field: Arc::new(CurrentField::new()), revision: 0, arrows: true, route_with: true, zones: Vec::new(), installed: Vec::new(), tracker: Default::default() }
    }
}

impl CurrentsState {
    /// Adds a source's fields, beside what it had of other areas.
    pub fn add(&mut self, source: CurrentSource, fields: Vec<Field>) {
        self.stores.entry(source).or_default().extend(fields);
        self.rebuild();
    }

    /// Replaces the atlases.
    pub fn set_tides(&mut self, tides: Vec<Arc<dyn Tidal>>) {
        self.tides = tides;
        self.rebuild();
    }

    pub fn tides(&self) -> &[Arc<dyn Tidal>] {
        &self.tides
    }

    pub fn store(&self, s: CurrentSource) -> Option<&WeatherStore> {
        self.stores.get(&s).filter(|st| !st.is_empty())
    }

    /// Drops the gridded currents (the atlases stay: they are installed, not downloaded).
    pub fn clear(&mut self) {
        self.stores.clear();
        self.rebuild();
    }

    /// Drops the hours before `t` from the gridded sources.
    pub fn prune_before(&mut self, t: DateTime<Utc>) {
        let mut n = 0;
        for st in self.stores.values_mut() {
            n += st.prune_before(t);
        }
        if n > 0 {
            self.rebuild();
        }
    }

    fn rebuild(&mut self) {
        self.stores.retain(|_, st| !st.is_empty());
        self.field = Arc::new(CurrentField::of(self.stores.iter().map(|(s, st)| (*s, st.clone())), self.tides.iter().cloned()));
        self.revision += 1;
    }

    /// The composed current, to hand to a worker.
    pub fn field(&self) -> Arc<CurrentField> {
        self.field.clone()
    }

    pub fn is_empty(&self) -> bool {
        self.field.is_empty()
    }

    /// What a route over `b` sails in: the composed current cut to `b`, when routes use currents and
    /// some source reaches into `b`.
    pub fn for_route(&self, b: &crate::core::vector::Bounds) -> Option<Arc<CurrentField>> {
        if !self.route_with || self.field.is_empty() {
            return None;
        }
        let f = self.field.within(b);
        (!f.is_empty()).then(|| Arc::new(f))
    }

    /// The sources covering a passage over `b`, in words for the routing panels: "Shom + Copernicus SMOC".
    pub fn sources_text(&self, b: &crate::core::vector::Bounds) -> Option<String> {
        let f = self.field.within(b);
        let names: Vec<&str> = f.sources().into_iter().map(|s| s.label()).collect();
        (!names.is_empty()).then(|| names.join(" + "))
    }

    /// The credits the chart must show while it shows this current.
    pub fn credits(&self) -> Vec<&'static str> {
        let mut v: Vec<&'static str> = self.field.sources().into_iter().filter_map(|s| s.credit()).collect();
        v.dedup();
        v
    }
}

/// The Currents group of the weather page, for every shell.
#[derive(serde::Serialize, Clone, Debug, PartialEq, Default)]
pub struct CurrentsPage {
    pub loaded: bool,
    /// Arrows on the chart.
    pub arrows: bool,
    /// Weather routes sail in the current.
    pub route_with: bool,
    /// The sources loaded: a line each.
    pub sources: Vec<SourceLine>,
    /// The tidal atlases that can be installed, and those that are.
    pub atlases: Vec<AtlasLine>,
    /// What the group says under its title.
    pub note: String,
    /// Under way: each source against the stream measured.
    pub tracking: Vec<String>,
}

#[derive(serde::Serialize, Clone, Debug, PartialEq)]
pub struct SourceLine {
    pub id: String,
    pub label: String,
    /// "1.5 km, until Wed 18 Nov 23:00".
    pub detail: String,
}

/// A Shom atlas zone: installed or not, and its size.
#[derive(serde::Serialize, Clone, Debug, PartialEq)]
pub struct AtlasLine {
    pub id: String,
    pub name: String,
    /// "Installed, 4.1 MB" or "4.1 MB to download".
    pub detail: String,
    pub installed: bool,
}

/// Each source against the stream measured under way, in words: "Shom: within 0.3 kn, 10° on average
/// over 2 h", best first.
pub fn tracking(c: &CurrentsState, now: DateTime<Utc>) -> Vec<String> {
    c.tracker
        .scores(now)
        .iter()
        .map(|s| {
            gettext_f(
                "{source}: off by {kn} kn on average, {speed} kn {how} and {deg}° {side}, over {h}",
                &[
                    ("source", s.source.label()),
                    ("kn", &format!("{:.1}", s.rms_kn)),
                    ("speed", &format!("{:.1}", s.speed_bias.abs())),
                    ("how", &if s.speed_bias >= 0.0 { crate::i18n::gettext("too strong") } else { crate::i18n::gettext("too weak") }),
                    ("deg", &format!("{:.0}", s.set_bias.abs())),
                    ("side", &if s.set_bias >= 0.0 { crate::i18n::gettext("to the right") } else { crate::i18n::gettext("to the left") }),
                    ("h", &crate::core::nav::calc::format_duration(chrono::Duration::minutes((s.hours * 60.0) as i64))),
                ],
            )
        })
        .collect()
}

/// "075° 1.2 kn": the set and drift measured over the last minutes.
pub fn set_and_drift(c: &CurrentsState, now: DateTime<Utc>) -> Option<(String, String)> {
    let (set, drift) = c.tracker.set_and_drift(now)?;
    Some((format!("{:03.0}°", set.rem_euclid(360.0)), format!("{drift:.1}")))
}

pub fn page(app: &crate::app::AppState) -> CurrentsPage {
    let w = app.weather.borrow();
    let c = &w.currents;
    let sources = c
        .stores
        .iter()
        .map(|(s, st)| {
            let until = st.time_range().map(|(_, b)| b.with_timezone(&chrono::Local).format("%a %d %b %H:%M").to_string()).unwrap_or_default();
            let res = s.resolution();
            let detail = match (res.is_empty(), until.is_empty()) {
                (false, false) => gettext_f("{res}, until {until}", &[("res", res), ("until", &until)]),
                (true, false) => gettext_f("Until {until}", &[("until", &until)]),
                _ => res.to_string(),
            };
            SourceLine { id: s.id().to_string(), label: s.label().to_string(), detail }
        })
        .chain(c.tides.iter().map(|t| SourceLine { id: CurrentSource::Atlas.id().to_string(), label: CurrentSource::Atlas.label().to_string(), detail: t.name() }))
        .collect();
    CurrentsPage {
        loaded: !c.is_empty(),
        arrows: c.arrows,
        route_with: c.route_with,
        sources,
        atlases: atlas_lines(c),
        note: crate::i18n::gettext("Tidal streams and ocean currents, shared by every forecast model. Weather routes fetch them for their passage."),
        tracking: tracking(c, Utc::now()),
    }
}

fn mb(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / 1_048_576.0)
}

/// The zones the website offers, and any installed that it no longer lists.
fn atlas_lines(c: &CurrentsState) -> Vec<AtlasLine> {
    let mut v: Vec<AtlasLine> = c
        .zones
        .iter()
        .map(|z| {
            let installed = c.installed.contains(&z.id);
            let detail = if installed { gettext_f("Installed, {mb} MB", &[("mb", &mb(z.bytes))]) } else { gettext_f("{mb} MB to download", &[("mb", &mb(z.bytes))]) };
            AtlasLine { id: z.id.clone(), name: z.name.clone(), detail, installed }
        })
        .collect();
    for id in c.installed.iter().filter(|id| !c.zones.iter().any(|z| &z.id == *id)) {
        v.push(AtlasLine { id: id.clone(), name: id.clone(), detail: crate::i18n::gettext("Installed"), installed: true });
    }
    v
}

/// The current at a point and time from a source that resolves tidal races (`CurrentSource::resolves_races`):
/// what a performance window's speed may be corrected by. `None` from the 9 km global grid alone.
pub fn race_current(field: &CurrentField, pos: LatLon, t: DateTime<Utc>) -> Option<(f64, f64)> {
    field.source_at(&pos, t).filter(|s| s.resolves_races())?;
    field.sample(&pos, t)
}

/// "1.8 kn → 075°": a current as the chips and the cards say it, the way it sets.
pub fn current_text(e: f64, n: f64) -> String {
    let kn = e.hypot(n);
    gettext_f("{kn} kn → {dir}°", &[("kn", &format!("{kn:.1}")), ("dir", &format!("{:03.0}", geo::set_of(e, n)))])
}

/// The current at a point and time as a chip, with its source in the label.
pub fn readout(field: &CurrentField, lat: f64, lon: f64, t: DateTime<Utc>) -> Option<Chip> {
    let p = LatLon::new(lat, lon);
    let (e, n) = field.sample(&p, t)?;
    let text = current_text(e, n);
    let source = field.source_at(&p, t).map(|s| s.label()).unwrap_or_default();
    Some(Chip::new(Icon::Current, text.clone(), gettext_f("current {current} ({source})", &[("current", &text), ("source", source)])))
}

/// The made-up current of `ISOMAR_DEMO_CURRENTS`: two races of the Iroise (the Raz de Sein and the
/// Fromveur) and a gentle stream along the coast, every 0.01° for two days from the hour before `now`.
/// No real data goes into it; it is only for screenshots and the simulators.
pub fn demo_fields(now: DateTime<Utc>) -> Vec<Field> {
    use crate::core::chart::grid::Grid;
    use crate::core::currents::synthetic::Race;
    use crate::core::weather::model::{Param, units::MS_TO_KN};
    let start = now - chrono::Duration::hours(1);
    let start = start - chrono::Duration::seconds(start.timestamp().rem_euclid(3600));
    let races = [
        Race { centre: LatLon::new(48.035, -4.79), axis_deg: 20.0, half_width_nm: 1.0, half_length_nm: 3.0, peak_kn: 6.0, flood_at: start + chrono::Duration::hours(2) },
        Race { centre: LatLon::new(48.445, -4.985), axis_deg: 45.0, half_width_nm: 1.0, half_length_nm: 2.5, peak_kn: 5.0, flood_at: start + chrono::Duration::hours(2) },
        Race { centre: LatLon::new(48.25, -4.9), axis_deg: 10.0, half_width_nm: 25.0, half_length_nm: 30.0, peak_kn: 1.2, flood_at: start + chrono::Duration::hours(2) },
    ];
    let (xll, yll, step, ncols, nrows) = (-5.4, 47.7, 0.01, 120, 100);
    let mut out = Vec::new();
    for h in 0..=48 {
        let t = start + chrono::Duration::hours(h);
        let (mut us, mut vs) = (Vec::with_capacity(ncols * nrows), Vec::with_capacity(ncols * nrows));
        for r in 0..nrows {
            let lat = yll + (nrows - r) as f64 * step - step / 2.0;
            for c in 0..ncols {
                let p = LatLon::new(lat, xll + (c as f64 + 0.5) * step);
                let (e, n) = races.iter().map(|race| race.at(&p, t)).fold((0.0, 0.0), |a, b| (a.0 + b.0, a.1 + b.1));
                us.push((e / MS_TO_KN as f64) as f32);
                vs.push((n / MS_TO_KN as f64) as f32);
            }
        }
        let grid = |data| Grid { ncols, nrows, xll, yll, dx: step, dy: step, data };
        out.push(Field { param: Param::CurrentU, time: t, run: Some(start), grid: grid(us) });
        out.push(Field { param: Param::CurrentV, time: t, run: Some(start), grid: grid(vs) });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_demo_current_runs_in_the_raz_and_reads_out_with_its_source() {
        let now = Utc::now();
        let mut c = CurrentsState::default();
        assert!(c.is_empty() && c.for_route(&crate::core::vector::Bounds { min_lon: -5.0, max_lon: -4.0, min_lat: 48.0, max_lat: 49.0 }).is_none());
        c.add(CurrentSource::Demo, demo_fields(now));
        let field = c.field();
        let flood = now + chrono::Duration::hours(1);
        let (e, n) = field.sample(&LatLon::new(48.035, -4.79), flood).unwrap();
        assert!(e.hypot(n) > 5.0, "the Raz at the flood: {e}, {n}");
        let chip = readout(&field, 48.035, -4.79, flood).unwrap();
        assert!(chip.text.contains("kn →") && chip.label.contains("Demo"), "{chip:?}");
        let b = crate::core::vector::Bounds { min_lon: -5.0, max_lon: -4.5, min_lat: 47.9, max_lat: 48.2 };
        assert!(c.for_route(&b).is_some());
        assert_eq!(c.sources_text(&b).as_deref(), Some("Demo"));
        c.route_with = false;
        assert!(c.for_route(&b).is_none(), "switched off");
        let far = crate::core::vector::Bounds { min_lon: 10.0, max_lon: 11.0, min_lat: 40.0, max_lat: 41.0 };
        assert!(c.sources_text(&far).is_none());
    }
}
