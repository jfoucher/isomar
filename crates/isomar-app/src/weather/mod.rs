//! Weather state shared by the overlay, the pages and the router.

pub mod layer;

use crate::chips::{Chip, Icon};
use crate::core::geo::LatLon;
use crate::core::weather::model::{Param, units};
use crate::i18n::{gettext, gettext_f};
use crate::core::weather::planner::Area;
use crate::core::weather::source::Source;
use crate::core::weather::store::WeatherStore;
use std::collections::BTreeMap;
use crate::app::AppState;
use crate::engine::DragPhase;
use crate::shell::Dialog;
use chrono::{DateTime, Utc};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pick {
    Off,
    /// Waiting for the user to drag a box.
    Waiting,
    Dragging(LatLon),
}

pub struct WeatherState {
    /// The forecast of the model shown: what the overlay draws, the pages describe and the router
    /// sails on.
    pub store: WeatherStore,
    /// The model `store` is from.
    pub source: Source,
    /// The other models' forecasts, kept apart while another is shown: a model's field never stands in
    /// for another's.
    pub others: BTreeMap<Source, WeatherStore>,
    /// Each loaded model against the wind the instruments measure, under way.
    pub tracker: crate::core::perf::model_check::ModelTracker,
    /// The time being shown.
    pub time: DateTime<Utc>,
    pub barbs: bool,
    /// What colours the sea: one field at a time, as two would mix into colours neither ramp has.
    pub fill: Fill,
    pub pressure: bool,
    /// Wave direction and height, drawn over whichever field colours the sea.
    pub wave_arrows: bool,
    pub playing: bool,
    /// The playback speed, as a multiple of 3 forecast hours a second.
    pub speed_x: f64,
    pub pick: Pick,
    /// The box being dragged, or last chosen.
    pub rect: Option<(LatLon, LatLon)>,
    /// The currents loaded, shared by every model.
    pub currents: crate::currents::CurrentsState,
}

/// The field that colours the sea under the forecast's lines and arrows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Fill {
    None,
    #[default]
    Wind,
    Waves,
    /// The current's speed (`currents::layer`).
    Current,
}

impl Fill {
    pub fn name(self) -> &'static str {
        match self {
            Fill::None => "none",
            Fill::Wind => "wind",
            Fill::Waves => "waves",
            Fill::Current => "current",
        }
    }

    pub fn from_name(s: &str) -> Option<Fill> {
        [Fill::None, Fill::Wind, Fill::Waves, Fill::Current].into_iter().find(|f| f.name() == s)
    }
}

impl Default for WeatherState {
    fn default() -> Self {
        WeatherState {
            store: WeatherStore::new(),
            source: Source::default(),
            others: BTreeMap::new(),
            tracker: Default::default(),
            time: Utc::now(),
            barbs: true,
            fill: Fill::Wind,
            pressure: true,
            wave_arrows: true,
            playing: false,
            speed_x: 1.0,
            pick: Pick::Off,
            rect: None,
            currents: Default::default(),
        }
    }
}

impl WeatherState {
    /// The forecast of `s`, if any of it is loaded.
    pub fn store_of(&self, s: Source) -> Option<&WeatherStore> {
        if s == self.source { Some(&self.store) } else { self.others.get(&s) }.filter(|st| !st.is_empty())
    }

    pub fn store_of_mut(&mut self, s: Source) -> &mut WeatherStore {
        if s == self.source { &mut self.store } else { self.others.entry(s).or_default() }
    }

    /// Shows `s`'s forecast (empty if none is loaded), keeping the one shown until now.
    pub fn show(&mut self, s: Source) {
        if s == self.source {
            return;
        }
        let shown = std::mem::replace(&mut self.store, self.others.remove(&s).unwrap_or_default());
        if !shown.is_empty() {
            self.others.insert(self.source, shown);
        }
        self.source = s;
    }

    /// Whether anything is loaded to show: a forecast or a current.
    pub fn has_any(&self) -> bool {
        !self.store.is_empty() || !self.currents.is_empty()
    }

    /// What the pointer readout says at a point: the forecast's values and the current.
    pub fn readout_at(&self, lat: f64, lon: f64) -> Vec<Chip> {
        let mut chips = readout(&self.store, lat, lon, self.time);
        chips.extend(crate::currents::readout(&self.currents.field(), lat, lon, self.time));
        chips
    }

    /// The models with a forecast loaded, in `Source::ALL`'s order.
    pub fn loaded(&self) -> Vec<Source> {
        Source::ALL.into_iter().filter(|s| self.store_of(*s).is_some()).collect()
    }
}

pub fn rect_area(r: (LatLon, LatLon)) -> Area {
    Area::new(r.0.lat, r.0.lon, r.1.lat, r.1.lon)
}

/// Half-width, in degrees, of the box "Get weather here" starts from.
pub const HERE_HALF_DEG: f64 = 4.0;

/// What the forecast says at a point and time, one chip a value: wind, gusts, pressure, waves, air
/// temperature.
pub fn readout(store: &crate::core::weather::store::WeatherStore, lat: f64, lon: f64, t: DateTime<Utc>) -> Vec<Chip> {
    let mut parts = Vec::new();
    if let Some(w) = store.wind(lat, lon, t) {
        let text = gettext_f("{kn} kn from {dir}°", &[("kn", &format!("{:.0}", units::ms_to_kn(w.speed_ms()))), ("dir", &format!("{:03.0}", w.blows_from()))]);
        parts.push(Chip::new(Icon::Wind, text.clone(), gettext_f("wind {wind}", &[("wind", &text)])));
    }
    if let Some(g) = store.sample(Param::Gust, lat, lon, t) {
        let kn = format!("{:.0}", units::ms_to_kn(g));
        parts.push(Chip::new(Icon::Gusts, gettext_f("{kn} kn", &[("kn", &kn)]), gettext_f("gusts {kn} kn", &[("kn", &kn)])));
    }
    if let Some(p) = store.sample(Param::Pressure, lat, lon, t) {
        parts.push(Chip::with(Icon::Pressure, format!("{:.0} hPa", units::pa_to_hpa(p))));
    }
    if let Some(h) = store.sample(Param::WaveHeight, lat, lon, t) {
        let mut s = format!("{h:.1} m");
        if let Some(per) = store.sample(Param::WavePeriod, lat, lon, t) {
            s.push_str(&format!(" / {per:.0} s"));
        }
        parts.push(Chip::new(Icon::Waves, s.clone(), gettext_f("waves {h}", &[("h", &s)])));
    }
    if let Some(t) = store.sample(Param::Temperature, lat, lon, t) {
        let c = format!("{:.0} °C", units::k_to_c(t));
        parts.push(Chip::new(Icon::Temperature, c.clone(), gettext_f("air {t}", &[("t", &c)])));
    }
    parts
}

fn open_dialog(app: &AppState, a: Area) {
    if let Some(s) = app.shell() {
        s.open(Dialog::GetWeather { north: a.north, south: a.south, west: a.west, east: a.east });
    }
}

/// Registers the menu entries and the box-drawing drag.
pub fn install(app: &Rc<AppState>) {
    app.add_context_item(&gettext("Get weather here…"), |app, pos| {
        open_dialog(app, Area::around(pos.lat, pos.lon, HERE_HALF_DEG));
    });
    app.add_context_item(&gettext("Get weather for this view…"), |app, _| {
        let b = app.canvas.view_info().bounds();
        open_dialog(app, Area::from_bounds(&b));
    });
    app.add_context_item(&gettext("Get weather for an area…"), |app, _| start_pick(app));
    let weak = Rc::downgrade(app);
    app.canvas.connect_drag(move |phase, pos| {
        let Some(app) = weak.upgrade() else { return false };
        let mut w = app.weather.borrow_mut();
        match (phase, w.pick) {
            (DragPhase::Begin, Pick::Waiting) => {
                w.pick = Pick::Dragging(pos);
                w.rect = Some((pos, pos));
                true
            }
            (DragPhase::Update, Pick::Dragging(a)) => {
                w.rect = Some((a, pos));
                drop(w);
                app.canvas.queue_draw();
                true
            }
            (DragPhase::End, Pick::Dragging(a)) => {
                w.pick = Pick::Off;
                w.rect = Some((a, pos));
                drop(w);
                app.refresh();
                open_dialog(&app, rect_area((a, pos)));
                true
            }
            _ => false,
        }
    });
}

/// Waits for the user to drag a box on the chart.
pub fn start_pick(app: &AppState) {
    {
        let mut w = app.weather.borrow_mut();
        w.pick = Pick::Waiting;
        w.rect = None;
    }
    app.toast(&gettext("Drag a box on the chart"));
    app.refresh();
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use crate::core::weather::model::Param;
    use crate::core::chart::grid::Grid;
    use crate::core::weather::model::Field;
    use crate::core::weather::store::WeatherStore;
    use chrono::TimeZone;

    fn store_with(param: Param, v: f32) -> WeatherStore {
        let mut s = WeatherStore::new();
        s.add(Field {
            param,
            time: Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap(),
            run: None,
            grid: Grid { ncols: 3, nrows: 3, xll: 10.0, yll: 40.0, dx: 1.0, dy: 1.0, data: vec![v; 9] },
        });
        s
    }

    #[test]
    fn the_readout_lists_what_is_known_at_the_point() {
        let t = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        let mut s = store_with(Param::WindU, 5.14444);
        s.merge(store_with(Param::WindV, 0.0));
        s.merge(store_with(Param::Pressure, 101_325.0));
        let r = crate::chips::plain(&readout(&s, 41.5, 11.5, t));
        // A west wind of 10 kn at standard pressure.
        assert!(r.contains("10 kn") && r.contains("270°") && r.contains("1013 hPa"), "{r}");
        assert!(readout(&s, 80.0, 11.5, t).is_empty(), "outside the data");
        assert!(readout(&WeatherStore::new(), 41.5, 11.5, t).is_empty());
    }

    #[test]
    fn each_model_keeps_its_own_forecast() {
        let mut w = WeatherState::default();
        assert_eq!(w.source, Source::Gfs);
        w.store_of_mut(Source::Gfs).merge(store_with(Param::WindU, 1.0));
        w.store_of_mut(Source::Ifs).merge(store_with(Param::WindU, 2.0));
        assert_eq!(w.loaded(), vec![Source::Gfs, Source::Ifs]);
        let t = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        let u = |w: &WeatherState| w.store.sample(Param::WindU, 41.5, 11.5, t);
        assert_eq!(u(&w), Some(1.0));
        w.show(Source::Ifs);
        assert_eq!((w.source, u(&w)), (Source::Ifs, Some(2.0)));
        assert_eq!(w.store_of(Source::Gfs).and_then(|s| s.sample(Param::WindU, 41.5, 11.5, t)), Some(1.0), "kept aside");
        w.show(Source::Aifs);
        assert!(w.store.is_empty() && w.store_of(Source::Aifs).is_none());
        assert_eq!(w.loaded(), vec![Source::Gfs, Source::Ifs], "nothing is lost by showing an empty one");
        w.show(Source::Gfs);
        assert_eq!(u(&w), Some(1.0));
    }

    #[test]
    fn waves_show_height_and_period() {
        let t = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        let mut s = store_with(Param::WaveHeight, 2.34);
        s.merge(store_with(Param::WavePeriod, 9.0));
        let r = readout(&s, 41.5, 11.5, t);
        assert_eq!(r[0].text, "2.3 m / 9 s");
    }
}
