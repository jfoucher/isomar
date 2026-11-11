//! The composed current: every loaded source, finest first, blended where a finer one gives way to a
//! coarser one, so the router meets no wall of different stream at a grid's edge.
//!
//! Order (`CurrentSource`'s): an imported GRIB, the tidal atlases (finest grid first, their tide plus
//! SMOC's residual where it is loaded), the shelf models, the regional seas, SMOC. A source that has no
//! value at a point (land on its grid, past its forecast) falls through to the next one, never to zero.

use super::CurrentSource;
use crate::geo::LatLon;
use crate::vector::Bounds;
use crate::weather::model::{Param, units::MS_TO_KN};
use crate::weather::store::WeatherStore;
use chrono::{DateTime, Utc};
use std::sync::Arc;

/// How far inside its edge a finer grid takes over wholly, in cells of that grid.
pub const BLEND_CELLS: f64 = 3.0;

/// A tidal stream known by phase rather than by forecast time: a Shom atlas, phased by the tide at its
/// reference port (`atlas`).
pub trait Tidal: Send + Sync {
    /// Where it has values.
    fn bounds(&self) -> Bounds;
    /// Its grid step, degrees of latitude.
    fn step_deg(&self) -> f64;
    /// The tidal stream at `p` and `t`, (east, north) knots; `None` where it has no value or cannot be
    /// phased at `t`.
    fn tide(&self, p: &LatLon, t: DateTime<Utc>) -> Option<(f64, f64)>;
    /// A name for logs and the card ("Finistère 250 m").
    fn name(&self) -> String;
}

enum Layer {
    /// A gridded forecast of the total current.
    Grid { source: CurrentSource, store: WeatherStore, bounds: Bounds, blend_deg: f64 },
    /// An atlas tide, to which `CurrentField::residual` is added.
    Tide(Arc<dyn Tidal>),
}

impl Layer {
    fn source(&self) -> CurrentSource {
        match self {
            Layer::Grid { source, .. } => *source,
            Layer::Tide(_) => CurrentSource::Atlas,
        }
    }

    fn bounds_and_blend(&self) -> (Bounds, f64) {
        match self {
            Layer::Grid { bounds, blend_deg, .. } => (*bounds, *blend_deg),
            Layer::Tide(t) => (t.bounds(), t.step_deg() * BLEND_CELLS),
        }
    }

    /// Its share at `p`: 1 well inside it, falling to 0 at its edge.
    fn weight(&self, p: &LatLon, last: bool) -> f64 {
        if last {
            return 1.0;
        }
        let (b, blend) = self.bounds_and_blend();
        if blend <= 0.0 {
            return 1.0;
        }
        let k = p.lat.to_radians().cos().max(0.1);
        let edge = (p.lat - b.min_lat).min(b.max_lat - p.lat).min((p.lon - b.min_lon) * k).min((b.max_lon - p.lon) * k);
        (edge / blend).clamp(0.0, 1.0)
    }
}

/// The current at any point and time, from every loaded source. Cheap to share (`Arc`) with the
/// router's worker and the chart's tile painters.
#[derive(Default)]
pub struct CurrentField {
    layers: Vec<Layer>,
    /// SMOC's current without its tide, added to an atlas's tide.
    residual: Option<WeatherStore>,
}

fn knots(store: &WeatherStore, u: Param, v: Param, p: &LatLon, t: DateTime<Utc>) -> Option<(f64, f64)> {
    let e = store.sample_valid(u, p.lat, p.lon, t)?;
    let n = store.sample_valid(v, p.lat, p.lon, t)?;
    Some(((e * MS_TO_KN) as f64, (n * MS_TO_KN) as f64))
}

/// A grid's step in degrees of latitude, from its first field.
fn step_of(store: &WeatherStore) -> f64 {
    store.fields().next().map_or(0.0, |f| f.grid.dy)
}

impl CurrentField {
    pub fn new() -> CurrentField {
        CurrentField::default()
    }

    /// Composes the stores and atlases given, in the field's order whatever the order given.
    pub fn of(stores: impl IntoIterator<Item = (CurrentSource, WeatherStore)>, tides: impl IntoIterator<Item = Arc<dyn Tidal>>) -> CurrentField {
        let mut f = CurrentField::new();
        for (s, st) in stores {
            f.add_store(s, st);
        }
        for t in tides {
            f.add_tide(t);
        }
        f
    }

    /// Adds a source's gridded forecast. SMOC's residual is kept for the atlases.
    pub fn add_store(&mut self, source: CurrentSource, store: WeatherStore) {
        if store.is_empty() {
            return;
        }
        if source == CurrentSource::Smoc && store.has(Param::ResidualU) {
            self.residual = Some(store.clone());
        }
        if !store.has(Param::CurrentU) {
            return;
        }
        let Some(bounds) = store.bounds() else { return };
        let blend_deg = step_of(&store) * BLEND_CELLS;
        self.layers.push(Layer::Grid { source, store, bounds, blend_deg });
        self.sort();
    }

    pub fn add_tide(&mut self, t: Arc<dyn Tidal>) {
        self.layers.push(Layer::Tide(t));
        self.sort();
    }

    /// Finest first: by source, then the atlases by grid step.
    fn sort(&mut self) {
        self.layers.sort_by(|a, b| {
            a.source().cmp(&b.source()).then_with(|| match (a, b) {
                (Layer::Tide(x), Layer::Tide(y)) => x.step_deg().total_cmp(&y.step_deg()),
                _ => std::cmp::Ordering::Equal,
            })
        });
    }

    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    /// The sources it holds, in its order, without repeats.
    pub fn sources(&self) -> Vec<CurrentSource> {
        let mut v: Vec<CurrentSource> = self.layers.iter().map(Layer::source).collect();
        v.dedup();
        v
    }

    /// The names of the atlases it holds.
    pub fn atlas_names(&self) -> Vec<String> {
        self.layers.iter().filter_map(|l| if let Layer::Tide(t) = l { Some(t.name()) } else { None }).collect()
    }

    /// Whether SMOC's residual is loaded, to go with the atlases.
    pub fn has_residual(&self) -> bool {
        self.residual.is_some()
    }

    fn layer_at(&self, i: usize, p: &LatLon, t: DateTime<Utc>) -> Option<(f64, f64)> {
        match &self.layers[i] {
            Layer::Grid { store, .. } => knots(store, Param::CurrentU, Param::CurrentV, p, t),
            Layer::Tide(tide) => {
                let (e, n) = tide.tide(p, t)?;
                let (re, rn) = self.residual.as_ref().and_then(|r| knots(r, Param::ResidualU, Param::ResidualV, p, t)).unwrap_or((0.0, 0.0));
                Some((e + re, n + rn))
            }
        }
    }

    /// The first layer from `from` on with a value at `p` and `t`, blended with what lies under it near
    /// its edge; and that layer.
    fn sample_from(&self, from: usize, p: &LatLon, t: DateTime<Utc>) -> Option<((f64, f64), usize)> {
        let n = self.layers.len();
        for i in from..n {
            let Some(c) = self.layer_at(i, p, t) else { continue };
            let w = self.layers[i].weight(p, i + 1 == n);
            if w >= 1.0 {
                return Some((c, i));
            }
            return match self.sample_from(i + 1, p, t) {
                Some(((oe, on), _)) => Some(((c.0 * w + oe * (1.0 - w), c.1 * w + on * (1.0 - w)), i)),
                None => Some((c, i)),
            };
        }
        None
    }

    /// The current at `p` and `t`, (east, north) knots.
    pub fn sample(&self, p: &LatLon, t: DateTime<Utc>) -> Option<(f64, f64)> {
        if self.layers.is_empty() {
            return None;
        }
        self.sample_from(0, p, t).map(|(c, _)| c)
    }

    /// Each source's own current at `p` and `t`, unblended, the finest of each source only: to score
    /// them against the stream measured under way (`perf::current_check`).
    pub fn each_at(&self, p: &LatLon, t: DateTime<Utc>) -> Vec<(CurrentSource, (f64, f64))> {
        let mut out: Vec<(CurrentSource, (f64, f64))> = Vec::new();
        for i in 0..self.layers.len() {
            let s = self.layers[i].source();
            if out.iter().any(|(o, _)| *o == s) {
                continue;
            }
            if let Some(c) = self.layer_at(i, p, t) {
                out.push((s, c));
            }
        }
        out
    }

    /// The source the current at `p` and `t` mostly comes from.
    pub fn source_at(&self, p: &LatLon, t: DateTime<Utc>) -> Option<CurrentSource> {
        self.sample_from(0, p, t).map(|(_, i)| self.layers[i].source())
    }

    /// The area it covers, all sources together (atlases included).
    pub fn bounds(&self) -> Option<Bounds> {
        let mut b = Bounds::EMPTY;
        for l in &self.layers {
            b.union(&l.bounds_and_blend().0);
        }
        (!b.is_empty()).then_some(b)
    }

    /// Whether some source covers `p` (wholly, ignoring time).
    pub fn covers(&self, p: &LatLon) -> bool {
        self.layers.iter().any(|l| l.bounds_and_blend().0.contains([p.lon, p.lat]))
    }

    /// The gridded forecasts' first and last times (atlases hold for any time and add none).
    pub fn time_range(&self) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        self.layers
            .iter()
            .filter_map(|l| if let Layer::Grid { store, .. } = l { store.time_range() } else { None })
            .reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
    }

    /// The hours of the gridded forecasts, for a timeline to step through.
    pub fn times(&self) -> Vec<DateTime<Utc>> {
        let mut v: Vec<DateTime<Utc>> = self.layers.iter().filter_map(|l| if let Layer::Grid { store, .. } = l { Some(store.times(Param::CurrentU)) } else { None }).flatten().collect();
        v.sort();
        v.dedup();
        v
    }

    /// The part touching `b` only: what a search in `b` reads.
    pub fn within(&self, b: &Bounds) -> CurrentField {
        let layers = self
            .layers
            .iter()
            .filter_map(|l| match l {
                Layer::Grid { source, store, blend_deg, .. } => {
                    let store = store.within(b);
                    let bounds = store.bounds()?;
                    Some(Layer::Grid { source: *source, store, bounds, blend_deg: *blend_deg })
                }
                Layer::Tide(t) => t.bounds().intersects(b).then(|| Layer::Tide(t.clone())),
            })
            .collect();
        CurrentField { layers, residual: self.residual.as_ref().map(|r| r.within(b)) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::grid::Grid;
    use crate::weather::model::Field;
    use chrono::{Duration, TimeZone};

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap()
    }

    /// A uniform current of (`e`, `n`) m/s over a box of `size` degrees in steps of `step`, for 24 h.
    fn store(e: f32, n: f32, xll: f64, yll: f64, size: f64, step: f64) -> WeatherStore {
        let cells = (size / step).round() as usize;
        let mut s = WeatherStore::new();
        for h in [0, 24] {
            for (p, v) in [(Param::CurrentU, e), (Param::CurrentV, n)] {
                s.add(Field { param: p, time: t0() + Duration::hours(h), run: Some(t0()), grid: Grid { ncols: cells, nrows: cells, xll, yll, dx: step, dy: step, data: vec![v; cells * cells] } });
            }
        }
        s
    }

    fn east(f: &CurrentField, lat: f64, lon: f64) -> Option<f64> {
        f.sample(&LatLon::new(lat, lon), t0() + Duration::hours(6)).map(|c| c.0)
    }

    struct Ring {
        b: Bounds,
        kn: f64,
    }

    impl Tidal for Ring {
        fn bounds(&self) -> Bounds {
            self.b
        }
        fn step_deg(&self) -> f64 {
            0.01
        }
        fn tide(&self, p: &LatLon, _t: DateTime<Utc>) -> Option<(f64, f64)> {
            self.b.contains([p.lon, p.lat]).then_some((self.kn, 0.0))
        }
        fn name(&self) -> String {
            "ring".into()
        }
    }

    #[test]
    fn the_finest_source_wins_inside_it_and_blends_at_its_edge() {
        let kn = |ms: f64| ms * MS_TO_KN as f64;
        let smoc = store(1.0, 0.0, -10.0, 40.0, 20.0, 0.25);
        let shelf = store(2.0, 0.0, -5.0, 45.0, 5.0, 0.05);
        // Given coarse first: the order is the field's own.
        let f = CurrentField::of([(CurrentSource::Smoc, smoc), (CurrentSource::NwShelf, shelf)], []);
        assert_eq!(f.sources(), vec![CurrentSource::NwShelf, CurrentSource::Smoc]);
        assert!((east(&f, 47.5, -2.5).unwrap() - kn(2.0)).abs() < 1e-3, "inside the shelf");
        assert!((east(&f, 42.0, -8.0).unwrap() - kn(1.0)).abs() < 1e-3, "outside it");
        // Three shelf cells (0.15 degree) from its southern edge, half of it is shelf.
        let edge = east(&f, 45.075, -2.5).unwrap();
        assert!((edge - kn(1.5)).abs() < 0.05, "{edge}");
        assert_eq!(f.source_at(&LatLon::new(47.5, -2.5), t0()), Some(CurrentSource::NwShelf));
        assert_eq!(east(&f, 70.0, 0.0), None, "nothing covers it");
    }

    #[test]
    fn a_finer_source_with_no_value_falls_through_and_never_to_zero() {
        let smoc = store(1.0, 0.0, -10.0, 40.0, 20.0, 0.25);
        let f = CurrentField::of([(CurrentSource::Smoc, smoc.clone())], []);
        let g = CurrentField::of([(CurrentSource::Smoc, smoc), (CurrentSource::NwShelf, store(f32::NAN, f32::NAN, -5.0, 45.0, 5.0, 0.05))], []);
        assert_eq!(east(&f, 47.5, -2.5), east(&g, 47.5, -2.5), "land on the shelf grid");
    }

    #[test]
    fn an_atlas_adds_the_residual_and_cuts_to_an_area() {
        let mut smoc = store(1.0, 0.0, -10.0, 40.0, 20.0, 0.25);
        for f in store(0.5, 0.0, -10.0, 40.0, 20.0, 0.25).fields() {
            smoc.add(Field { param: if f.param == Param::CurrentU { Param::ResidualU } else { Param::ResidualV }, ..f.clone() });
        }
        let ring: Arc<dyn Tidal> = Arc::new(Ring { b: Bounds { min_lon: -5.0, max_lon: -4.0, min_lat: 48.0, max_lat: 49.0 }, kn: 3.0 });
        let f = CurrentField::of([(CurrentSource::Smoc, smoc)], [ring]);
        assert_eq!(f.sources(), vec![CurrentSource::Atlas, CurrentSource::Smoc]);
        let c = east(&f, 48.5, -4.5).unwrap();
        assert!((c - (3.0 + 0.5 * MS_TO_KN as f64)).abs() < 1e-3, "the tide and the residual: {c}");
        let away = f.within(&Bounds { min_lon: -10.0, max_lon: -8.0, min_lat: 40.0, max_lat: 42.0 });
        assert_eq!(away.sources(), vec![CurrentSource::Smoc], "the atlas is left out of a far area");
    }
}
