//! A set of weather fields over time, sampled in space and time.

use super::model::{Field, Param};
use crate::chart::grid::Grid;
use crate::vector::Bounds;
use chrono::{DateTime, Duration, Utc};
use std::collections::BTreeMap;

/// How far outside the first and last field times values are still held,
/// so a map showing "now" works slightly before a run's first step.
const HOLD: i64 = 90;

/// The most areas kept for a parameter: the forecasts of several passages
/// side by side. The one with the oldest run goes first.
const MAX_AREAS: usize = 8;

/// The fields of one grid shape (one download's area), oldest first.
type Series = Vec<Field>;

/// Weather fields over time. A parameter can have several areas (one per
/// download), so a forecast fetched for one passage does not replace the one
/// fetched for another; where areas overlap, the newest run is sampled.
#[derive(Clone, Debug, Default)]
pub struct WeatherStore {
    fields: BTreeMap<Param, Vec<Series>>,
}

fn same_shape(a: &Grid, b: &Grid) -> bool {
    a.ncols == b.ncols && a.nrows == b.nrows && (a.xll - b.xll).abs() < 1e-6 && (a.yll - b.yll).abs() < 1e-6 && (a.dx - b.dx).abs() < 1e-9 && (a.dy - b.dy).abs() < 1e-9
}

/// Grids may use 0..360 longitudes; bounds here use -180..180.
fn normalised(fb: Bounds) -> Bounds {
    let shift = if fb.min_lon >= 180.0 { -360.0 } else { 0.0 };
    Bounds { min_lon: fb.min_lon + shift, max_lon: fb.max_lon + shift, ..fb }
}

fn area_of(s: &Series) -> Bounds {
    normalised(s[0].grid.bounds())
}

fn contains(outer: &Bounds, inner: &Bounds) -> bool {
    outer.min_lon <= inner.min_lon && outer.max_lon >= inner.max_lon && outer.min_lat <= inner.min_lat && outer.max_lat >= inner.max_lat
}

fn overlaps(a: &Bounds, b: &Bounds) -> bool {
    a.min_lon <= b.max_lon && a.max_lon >= b.min_lon && a.min_lat <= b.max_lat && a.max_lat >= b.min_lat
}

fn run_of(s: &Series) -> Option<DateTime<Utc>> {
    s.iter().filter_map(|f| f.run).max()
}

/// Whether `a` makes `b` redundant: it has everything `b` has, from a run as new.
fn dominates(a: &Series, b: &Series) -> bool {
    contains(&area_of(a), &area_of(b)) && a[0].time <= b[0].time && a[a.len() - 1].time >= b[b.len() - 1].time && run_of(a) >= run_of(b)
}

/// Wind as eastward and northward components in m/s.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wind {
    pub u: f32,
    pub v: f32,
}

impl Wind {
    pub fn speed_ms(&self) -> f32 {
        (self.u * self.u + self.v * self.v).sqrt()
    }

    /// The direction the wind blows *from*, degrees true.
    pub fn blows_from(&self) -> f32 {
        (-self.u).atan2(-self.v).to_degrees().rem_euclid(360.0)
    }
}

/// Samples a grid at a longitude that may be given in either -180..180 or
/// 0..360 convention.
pub fn sample_wrapped(g: &Grid, lon: f64, lat: f64) -> Option<f32> {
    wrapped(g, lon, lat, Grid::sample)
}

fn wrapped(g: &Grid, lon: f64, lat: f64, at: fn(&Grid, f64, f64) -> Option<f32>) -> Option<f32> {
    let mut l = lon;
    while l < g.xll {
        l += 360.0;
    }
    while l >= g.xll + 360.0 {
        l -= 360.0;
    }
    at(g, l, lat)
}

impl WeatherStore {
    pub fn new() -> WeatherStore {
        WeatherStore::default()
    }

    /// Adds a field, replacing one for the same parameter, area and time. A
    /// field of another area is kept beside the others.
    pub fn add(&mut self, f: Field) {
        let all = self.fields.entry(f.param).or_default();
        match all.iter_mut().find(|s| same_shape(&s[0].grid, &f.grid)) {
            Some(s) => match s.binary_search_by_key(&f.time, |x| x.time) {
                Ok(i) => s[i] = f,
                Err(i) => s.insert(i, f),
            },
            None => all.push(vec![f]),
        }
    }

    pub fn extend(&mut self, fields: impl IntoIterator<Item = Field>) {
        for f in fields {
            self.add(f);
        }
        self.tidy();
    }

    pub fn merge(&mut self, other: WeatherStore) {
        for (_, all) in other.fields {
            self.extend(all.into_iter().flatten());
        }
    }

    /// Drops the areas another one makes redundant and those past `MAX_AREAS`
    /// (oldest run first), and puts the newest run first.
    fn tidy(&mut self) {
        for all in self.fields.values_mut() {
            let n = all.len();
            let keep: Vec<bool> = (0..n).map(|i| !(0..n).any(|j| j != i && dominates(&all[j], &all[i]) && (!dominates(&all[i], &all[j]) || j < i))).collect();
            let mut keep = keep.into_iter();
            all.retain(|_| keep.next().unwrap_or(true));
            all.sort_by_key(|s| std::cmp::Reverse(run_of(s)));
            all.truncate(MAX_AREAS);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    pub fn clear(&mut self) {
        self.fields.clear();
    }

    pub fn params(&self) -> Vec<Param> {
        self.fields.keys().copied().collect()
    }

    pub fn has(&self, p: Param) -> bool {
        self.fields.get(&p).is_some_and(|v| !v.is_empty())
    }

    pub fn times(&self, p: Param) -> Vec<DateTime<Utc>> {
        let mut t: Vec<_> = self.fields.get(&p).into_iter().flatten().flatten().map(|f| f.time).collect();
        t.sort();
        t.dedup();
        t
    }

    /// First and last valid time over all parameters.
    pub fn time_range(&self) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        let all = self.fields().map(|f| f.time);
        let (mut lo, mut hi): (Option<DateTime<Utc>>, Option<DateTime<Utc>>) = (None, None);
        for t in all {
            lo = Some(lo.map_or(t, |l| l.min(t)));
            hi = Some(hi.map_or(t, |h| h.max(t)));
        }
        lo.zip(hi)
    }

    /// The times between which wind can be sampled somewhere: both components
    /// exist, and the brief hold either side of the fields is included.
    /// `time_range` covers every parameter, so it can overstate this.
    pub fn wind_range(&self) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        self.wind_range_in(None)
    }

    /// `wind_range` of the areas that touch `b` (all of them for `None`).
    pub fn wind_range_in(&self, b: Option<&Bounds>) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        let (us, vs) = (self.fields.get(&Param::WindU)?, self.fields.get(&Param::WindV)?);
        let hold = Duration::minutes(HOLD);
        let mut range: Option<(DateTime<Utc>, DateTime<Utc>)> = None;
        for u in us.iter().filter(|u| b.is_none_or(|b| overlaps(&area_of(u), b))) {
            let Some(v) = vs.iter().find(|v| same_shape(&v[0].grid, &u[0].grid)) else { continue };
            let (lo, hi) = (u[0].time.max(v[0].time), u[u.len() - 1].time.min(v[v.len() - 1].time));
            if lo <= hi {
                range = Some(range.map_or((lo, hi), |(l, h)| (l.min(lo), h.max(hi))));
            }
        }
        range.map(|(lo, hi)| (lo - hold, hi + hold))
    }

    /// The latest model run any field came from.
    pub fn latest_run(&self) -> Option<DateTime<Utc>> {
        self.fields().filter_map(|f| f.run).max()
    }

    /// Whether one area of `p` holds all of `b` and lasts until `until`.
    pub fn reaches(&self, p: Param, b: &Bounds, until: DateTime<Utc>) -> bool {
        let hold = Duration::minutes(HOLD);
        self.fields.get(&p).is_some_and(|all| all.iter().any(|s| contains(&area_of(s), b) && s[s.len() - 1].time + hold >= until))
    }

    /// The area covered by the first field of any parameter.
    pub fn bounds(&self) -> Option<Bounds> {
        let mut b = Bounds::EMPTY;
        for f in self.fields() {
            b.union(&normalised(f.grid.bounds()));
        }
        (!b.is_empty()).then_some(b)
    }

    /// The areas that touch `b`, and nothing else: what a search in `b` reads,
    /// without copying the forecasts of other passages.
    pub fn within(&self, b: &Bounds) -> WeatherStore {
        let fields = self
            .fields
            .iter()
            .filter_map(|(p, all)| {
                let near: Vec<Series> = all.iter().filter(|s| overlaps(&area_of(s), b)).cloned().collect();
                (!near.is_empty()).then_some((*p, near))
            })
            .collect();
        WeatherStore { fields }
    }

    /// The two fields around `t` and the weight of the later one.
    fn bracket(v: &Series, t: DateTime<Utc>) -> Option<(&Field, &Field, f32)> {
        let (first, last) = (v.first()?, v.last()?);
        let hold = Duration::minutes(HOLD);
        if t < first.time - hold || t > last.time + hold {
            return None;
        }
        if t <= first.time {
            return Some((first, first, 0.0));
        }
        if t >= last.time {
            return Some((last, last, 0.0));
        }
        let i = v.partition_point(|f| f.time <= t);
        let (a, b) = (&v[i - 1], &v[i]);
        let span = (b.time - a.time).num_seconds() as f32;
        let w = if span > 0.0 { (t - a.time).num_seconds() as f32 / span } else { 0.0 };
        Some((a, b, w))
    }

    /// The value of `p` at a position and time.
    pub fn sample(&self, p: Param, lat: f64, lon: f64, t: DateTime<Utc>) -> Option<f32> {
        self.sample_by(p, lat, lon, t, Grid::sample)
    }

    /// `sample` over the grid corners that have data (`Grid::sample_valid`): for currents, which are
    /// missing over land and must not go missing in the last cell before it.
    pub fn sample_valid(&self, p: Param, lat: f64, lon: f64, t: DateTime<Utc>) -> Option<f32> {
        self.sample_by(p, lat, lon, t, Grid::sample_valid)
    }

    fn sample_by(&self, p: Param, lat: f64, lon: f64, t: DateTime<Utc>, at: fn(&Grid, f64, f64) -> Option<f32>) -> Option<f32> {
        // The newest run first; the next area where this one does not reach.
        self.fields.get(&p)?.iter().find_map(|s| {
            let (a, b, w) = Self::bracket(s, t)?;
            let va = wrapped(&a.grid, lon, lat, at)?;
            if w == 0.0 {
                return Some(va);
            }
            let vb = wrapped(&b.grid, lon, lat, at)?;
            // Directions need the short way round.
            if p == Param::WaveDir {
                let d = ((vb - va + 540.0).rem_euclid(360.0)) - 180.0;
                return Some((va + d * w).rem_euclid(360.0));
            }
            Some(va + (vb - va) * w)
        })
    }

    /// The run the value of `p` there and then comes from (the area `sample` reads).
    pub fn run_at(&self, p: Param, lat: f64, lon: f64, t: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.fields.get(&p)?.iter().find_map(|s| {
            let (a, _, _) = Self::bracket(s, t)?;
            sample_wrapped(&a.grid, lon, lat)?;
            Some(a.run)
        })?
    }

    pub fn wind(&self, lat: f64, lon: f64, t: DateTime<Utc>) -> Option<Wind> {
        Some(Wind { u: self.sample(Param::WindU, lat, lon, t)?, v: self.sample(Param::WindV, lat, lon, t)? })
    }

    /// Whether `wind` has an answer there and then, without working it out: the eastward component is
    /// sampled (its northward twin comes in the same files, on the same grid).
    pub fn has_wind(&self, lat: f64, lon: f64, t: DateTime<Utc>) -> bool {
        self.fields.contains_key(&Param::WindV) && self.sample(Param::WindU, lat, lon, t).is_some()
    }

    pub fn current(&self, lat: f64, lon: f64, t: DateTime<Utc>) -> Option<Wind> {
        Some(Wind { u: self.sample(Param::CurrentU, lat, lon, t)?, v: self.sample(Param::CurrentV, lat, lon, t)? })
    }

    /// The field for `p` closest in time to `t`.
    pub fn nearest(&self, p: Param, t: DateTime<Utc>) -> Option<&Field> {
        self.fields.get(&p)?.iter().flatten().min_by_key(|f| (f.time - t).num_seconds().abs())
    }

    /// The field for `p` at time `t`, blended from its neighbours in time so
    /// contours move smoothly with a time slider. Falls back to the nearest
    /// field when the neighbouring grids differ in shape.
    pub fn grid_at(&self, p: Param, t: DateTime<Utc>) -> Option<Grid> {
        self.grids_at(p, t).into_iter().next()
    }

    /// `grid_at` for every area that has `t`, the newest run first.
    pub fn grids_at(&self, p: Param, t: DateTime<Utc>) -> Vec<Grid> {
        let Some(all) = self.fields.get(&p) else { return Vec::new() };
        all.iter()
            .filter_map(|s| {
                let (a, b, w) = Self::bracket(s, t)?;
                if w == 0.0 || a.grid.data.len() != b.grid.data.len() || a.grid.xll != b.grid.xll || a.grid.yll != b.grid.yll {
                    return Some(if w < 0.5 { a } else { b }.grid.clone());
                }
                let mut g = a.grid.clone();
                for (x, y) in g.data.iter_mut().zip(&b.grid.data) {
                    *x += (y - *x) * w;
                }
                Some(g)
            })
            .collect()
    }

    /// Drops fields valid before `t`; returns how many.
    pub fn prune_before(&mut self, t: DateTime<Utc>) -> usize {
        let mut n = 0;
        for all in self.fields.values_mut() {
            for v in all.iter_mut() {
                let before = v.len();
                v.retain(|f| f.time >= t);
                n += before - v.len();
            }
            all.retain(|v| !v.is_empty());
        }
        self.fields.retain(|_, v| !v.is_empty());
        n
    }

    /// Every field, parameter by parameter and oldest first.
    pub fn fields(&self) -> impl Iterator<Item = &Field> {
        self.fields.values().flatten().flatten()
    }

    /// The fields valid from `from` to `to`, cut to the area `b`: enough to
    /// show or sample the forecast there, kept small. Areas that miss `b`
    /// are left out.
    pub fn clipped(&self, b: &Bounds, from: DateTime<Utc>, to: DateTime<Utc>) -> WeatherStore {
        let mut out = WeatherStore::new();
        for f in self.fields().filter(|f| f.time >= from && f.time <= to) {
            let Some(grid) = f.grid.clipped(b) else { continue };
            out.add(Field { param: f.param, time: f.time, run: f.run, grid });
        }
        out
    }

    pub fn field_count(&self) -> usize {
        self.fields().count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(h: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap() + Duration::hours(h)
    }

    /// A 3x3 grid covering lon 10..13, lat 40..43 with a constant value.
    fn field(p: Param, hours: i64, v: f32) -> Field {
        Field {
            param: p,
            time: t(hours),
            run: Some(t(0)),
            grid: Grid { ncols: 3, nrows: 3, xll: 10.0, yll: 40.0, dx: 1.0, dy: 1.0, data: vec![v; 9] },
        }
    }

    #[test]
    fn replaces_fields_for_the_same_time_and_keeps_them_sorted() {
        let mut s = WeatherStore::new();
        s.add(field(Param::Gust, 6, 1.0));
        s.add(field(Param::Gust, 0, 2.0));
        s.add(field(Param::Gust, 6, 3.0));
        assert_eq!(s.times(Param::Gust), vec![t(0), t(6)]);
        assert_eq!(s.field_count(), 2);
        assert_eq!(s.sample(Param::Gust, 41.5, 11.5, t(6)), Some(3.0));
    }

    /// The same 3x3 grid, moved to start at `lon`.
    fn field_at(p: Param, hours: i64, v: f32, lon: f64, run: i64) -> Field {
        let mut f = field(p, hours, v);
        f.grid.xll = lon;
        f.run = Some(t(run));
        f
    }

    #[test]
    fn fields_of_another_area_are_kept_beside_the_first() {
        let mut s = WeatherStore::new();
        s.extend([field_at(Param::Gust, 0, 1.0, 10.0, 0), field_at(Param::Gust, 6, 1.0, 10.0, 0)]);
        s.extend([field_at(Param::Gust, 0, 2.0, 100.0, 0), field_at(Param::Gust, 6, 2.0, 100.0, 0)]);
        assert_eq!(s.sample(Param::Gust, 41.5, 11.5, t(3)), Some(1.0));
        assert_eq!(s.sample(Param::Gust, 41.5, 101.5, t(3)), Some(2.0));
        assert_eq!(s.sample(Param::Gust, 41.5, 50.0, t(3)), None, "between them");
        assert_eq!(s.times(Param::Gust), vec![t(0), t(6)]);
        assert_eq!(s.field_count(), 4);
        // The same area again replaces its own fields only.
        s.extend([field_at(Param::Gust, 0, 5.0, 10.0, 0)]);
        assert_eq!(s.field_count(), 4);
        assert_eq!(s.sample(Param::Gust, 41.5, 11.5, t(0)), Some(5.0));
        assert_eq!(s.sample(Param::Gust, 41.5, 101.5, t(0)), Some(2.0));
    }

    #[test]
    fn where_areas_overlap_the_newest_run_wins_and_the_older_covers_the_rest() {
        let mut s = WeatherStore::new();
        s.extend([field_at(Param::Gust, 0, 1.0, 10.0, 0)]);
        s.extend([field_at(Param::Gust, 0, 2.0, 11.0, 6)]);
        assert_eq!(s.sample(Param::Gust, 41.5, 12.5, t(0)), Some(2.0), "both: the newer run");
        assert_eq!(s.sample(Param::Gust, 41.5, 10.5, t(0)), Some(1.0), "only the older");
        assert_eq!(s.grids_at(Param::Gust, t(0)).len(), 2);
    }

    #[test]
    fn an_area_a_newer_one_contains_is_dropped_and_the_number_of_areas_is_capped() {
        let mut s = WeatherStore::new();
        s.extend([field_at(Param::Gust, 0, 1.0, 11.0, 0)]);
        let mut big = field_at(Param::Gust, 0, 2.0, 10.0, 6);
        big.grid = Grid { ncols: 6, nrows: 6, xll: 10.0, yll: 39.0, dx: 1.0, dy: 1.0, data: vec![2.0; 36] };
        s.extend([big]);
        assert_eq!(s.field_count(), 1, "the small old one is inside the big new one");
        for i in 0..12 {
            s.extend([field_at(Param::Gust, 0, 1.0, 100.0 + 5.0 * i as f64, i)]);
        }
        assert_eq!(s.field_count(), MAX_AREAS);
        assert!(s.sample(Param::Gust, 41.5, 100.0 + 5.0 * 11.0 + 0.5, t(0)).is_some(), "the newest is kept");
        assert!(s.sample(Param::Gust, 41.5, 100.5, t(0)).is_none(), "the oldest went");
    }

    #[test]
    fn a_search_reads_only_the_areas_it_touches_and_a_forecast_lasts_as_long_as_its_area() {
        let mut s = WeatherStore::new();
        for p in [Param::WindU, Param::WindV] {
            s.extend([field_at(p, 0, 1.0, 10.0, 0), field_at(p, 12, 1.0, 10.0, 0)]);
            s.extend([field_at(p, 0, 1.0, 100.0, 0), field_at(p, 48, 1.0, 100.0, 0)]);
        }
        let here = Bounds { min_lon: 10.0, max_lon: 13.0, min_lat: 40.0, max_lat: 43.0 };
        let there = Bounds { min_lon: 100.0, max_lon: 103.0, min_lat: 40.0, max_lat: 43.0 };
        assert_eq!(s.wind_range_in(Some(&here)).unwrap().1, t(12) + Duration::minutes(HOLD));
        assert_eq!(s.wind_range_in(Some(&there)).unwrap().1, t(48) + Duration::minutes(HOLD));
        assert_eq!(s.wind_range().unwrap().1, t(48) + Duration::minutes(HOLD));
        assert!(s.reaches(Param::WindU, &here, t(12)) && !s.reaches(Param::WindU, &here, t(24)));
        assert!(s.reaches(Param::WindV, &there, t(24)));
        let near = s.within(&here);
        assert_eq!(near.field_count(), 4);
        assert!(near.wind(41.5, 101.5, t(0)).is_none() && near.wind(41.5, 11.5, t(0)).is_some());
        assert!(s.within(&Bounds { min_lon: 50.0, max_lon: 60.0, min_lat: 0.0, max_lat: 5.0 }).is_empty());
    }

    #[test]
    fn interpolates_linearly_in_time() {
        let mut s = WeatherStore::new();
        s.add(field(Param::Pressure, 0, 100_000.0));
        s.add(field(Param::Pressure, 6, 101_200.0));
        let v = s.sample(Param::Pressure, 41.5, 11.5, t(3)).unwrap();
        assert!((v - 100_600.0).abs() < 0.5);
        let v = s.sample(Param::Pressure, 41.5, 11.5, t(2)).unwrap();
        assert!((v - 100_400.0).abs() < 0.5);
    }

    #[test]
    fn holds_briefly_outside_the_time_range_then_gives_up() {
        let mut s = WeatherStore::new();
        s.add(field(Param::Gust, 3, 5.0));
        s.add(field(Param::Gust, 9, 7.0));
        assert_eq!(s.sample(Param::Gust, 41.5, 11.5, t(2)), Some(5.0), "an hour before");
        assert_eq!(s.sample(Param::Gust, 41.5, 11.5, t(10)), Some(7.0));
        assert_eq!(s.sample(Param::Gust, 41.5, 11.5, t(0)), None, "three hours before");
        assert_eq!(s.sample(Param::Gust, 41.5, 11.5, t(14)), None);
        assert_eq!(s.sample(Param::Temperature, 41.5, 11.5, t(5)), None, "no such parameter");
    }

    #[test]
    fn outside_the_grid_is_none() {
        let mut s = WeatherStore::new();
        s.add(field(Param::Gust, 0, 5.0));
        assert_eq!(s.sample(Param::Gust, 60.0, 11.5, t(0)), None);
        assert_eq!(s.sample(Param::Gust, 41.5, 30.0, t(0)), None);
    }

    #[test]
    fn longitudes_may_be_given_either_way_round() {
        // A grid in the 0..360 convention covering 348..351 (that is 12W to 9W).
        let mut s = WeatherStore::new();
        s.add(Field {
            param: Param::Gust,
            time: t(0),
            run: None,
            grid: Grid { ncols: 3, nrows: 3, xll: 348.0, yll: 40.0, dx: 1.0, dy: 1.0, data: vec![4.0; 9] },
        });
        assert_eq!(s.sample(Param::Gust, 41.5, -10.5, t(0)), Some(4.0));
        assert_eq!(s.sample(Param::Gust, 41.5, 349.5, t(0)), Some(4.0));
        let b = s.bounds().unwrap();
        assert!((b.min_lon + 12.0).abs() < 1e-9 && (b.max_lon + 9.0).abs() < 1e-9, "{b:?}");
    }

    #[test]
    fn wind_components_give_speed_and_the_direction_it_comes_from() {
        // Blowing towards the north-east means it comes from the south-west.
        let w = Wind { u: 3.0, v: 3.0 };
        assert!((w.blows_from() - 225.0).abs() < 1e-3);
        assert!((w.speed_ms() - 18f32.sqrt()).abs() < 1e-5);
        // A west wind blows towards the east (u > 0) and comes from 270.
        assert!((Wind { u: 5.0, v: 0.0 }.blows_from() - 270.0).abs() < 1e-3);
        assert!(Wind { u: 0.0, v: -5.0 }.blows_from().abs() < 1e-3, "north wind comes from 0");
    }

    #[test]
    fn wind_needs_both_components() {
        let mut s = WeatherStore::new();
        s.add(field(Param::WindU, 0, 2.0));
        assert!(s.wind(41.5, 11.5, t(0)).is_none());
        s.add(field(Param::WindV, 0, -1.0));
        assert_eq!(s.wind(41.5, 11.5, t(0)), Some(Wind { u: 2.0, v: -1.0 }));
    }

    #[test]
    fn the_wind_range_is_where_both_components_exist_plus_the_hold() {
        let mut s = WeatherStore::new();
        assert_eq!(s.wind_range(), None);
        s.add(field(Param::WindU, 0, 2.0));
        assert_eq!(s.wind_range(), None, "one component is not wind");
        s.add(field(Param::WindV, 0, 1.0));
        s.add(field(Param::WindU, 12, 2.0));
        s.add(field(Param::WindV, 9, 1.0));
        s.add(field(Param::Pressure, 48, 1.0));
        let (lo, hi) = s.wind_range().unwrap();
        assert_eq!(lo, t(0) - Duration::minutes(HOLD));
        assert_eq!(hi, t(9) + Duration::minutes(HOLD), "the shorter component ends it, not the pressure");
        assert!(s.wind(41.5, 11.5, hi).is_some());
        assert!(s.wind(41.5, 11.5, hi + Duration::minutes(1)).is_none());
    }

    #[test]
    fn wave_directions_interpolate_across_north() {
        let mut s = WeatherStore::new();
        s.add(field(Param::WaveDir, 0, 350.0));
        s.add(field(Param::WaveDir, 6, 10.0));
        let d = s.sample(Param::WaveDir, 41.5, 11.5, t(3)).unwrap();
        assert!(d < 1.0 || d > 359.0, "{d}");
    }

    #[test]
    fn blended_grids_follow_time() {
        let mut s = WeatherStore::new();
        s.add(field(Param::Pressure, 0, 100_000.0));
        s.add(field(Param::Pressure, 6, 101_000.0));
        let g = s.grid_at(Param::Pressure, t(3)).unwrap();
        assert!(g.data.iter().all(|v| (v - 100_500.0).abs() < 0.5));
        assert_eq!(s.grid_at(Param::Pressure, t(0)).unwrap().data[0], 100_000.0);
        assert!(s.grid_at(Param::Pressure, t(40)).is_none());
        assert!(s.grid_at(Param::Gust, t(0)).is_none());
    }

    #[test]
    fn merge_range_prune_and_runs() {
        let mut a = WeatherStore::new();
        a.add(field(Param::Gust, 0, 1.0));
        let mut b = WeatherStore::new();
        b.add(field(Param::Gust, 6, 2.0));
        b.add(field(Param::Pressure, 12, 3.0));
        a.merge(b);
        assert_eq!(a.time_range(), Some((t(0), t(12))));
        assert_eq!(a.latest_run(), Some(t(0)));
        assert_eq!(a.params(), vec![Param::Gust, Param::Pressure]);
        assert_eq!(a.prune_before(t(6)), 1);
        assert_eq!(a.times(Param::Gust), vec![t(6)]);
        assert_eq!(a.nearest(Param::Gust, t(0)).unwrap().time, t(6));
        a.prune_before(t(100));
        assert!(a.is_empty() && a.time_range().is_none() && a.bounds().is_none());
        a.clear();
        assert!(!a.has(Param::Gust));
    }
}
