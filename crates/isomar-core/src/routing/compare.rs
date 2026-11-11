//! Several forecasts against each other: each model's route sailed through every model's forecast
//! (`along::sail`), and which route holds up whichever forecast turns out right. The diagonal is each
//! model's own optimum; off it is what following route A costs if forecast B is the true weather.
//! Nobody can say in advance which model is right, so the route worth taking is the one whose worst
//! case is least bad.

use super::along::{Sailed, sail};
use super::isochrone::{Environment, Settings};
use super::polar::Polar;
use crate::geo::LatLon;
use crate::nav::calc::distance_to_segment_nm;
use chrono::{DateTime, Utc};
use rayon::prelude::*;

/// What one route makes of one forecast.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub end: DateTime<Utc>,
    pub hours: f64,
    pub arrives: bool,
    pub remaining_nm: f64,
    pub max_tws_kn: f64,
    pub motor_hours: f64,
    pub over_limit_hours: f64,
}

impl Cell {
    pub fn of(s: &Sailed) -> Cell {
        Cell { end: s.end, hours: s.hours(), arrives: s.arrives, remaining_nm: s.remaining_nm, max_tws_kn: s.max_tws_kn, motor_hours: s.motor_hours, over_limit_hours: s.over_limit_hours }
    }

    /// The hours it takes, or would take if the rest went at the speed made so far: so that a forecast
    /// ending short of the destination still compares.
    pub fn cost_hours(&self) -> f64 {
        if self.arrives || self.remaining_nm <= 0.0 {
            return self.hours;
        }
        self.hours + self.remaining_nm / FALLBACK_KN
    }
}

/// The speed the rest of a passage the forecast does not reach is reckoned at: slow, as an unknown
/// forecast is no reason to be hopeful.
const FALLBACK_KN: f64 = 4.0;

#[derive(Clone, Debug, PartialEq)]
pub struct CrossCheck {
    /// `cells[route][forecast]`.
    pub cells: Vec<Vec<Cell>>,
}

impl CrossCheck {
    /// Sails each of `routes` through each of `forecasts`, all leaving at `depart`.
    pub fn run(routes: &[Vec<LatLon>], forecasts: &[&dyn Environment], depart: DateTime<Utc>, polar: &Polar, s: &Settings) -> CrossCheck {
        let cells = routes
            .par_iter()
            .map(|r| forecasts.par_iter().map(|env| Cell::of(&sail(r, depart, polar, *env, s))).collect())
            .collect();
        CrossCheck { cells }
    }

    /// The worst a route does over all the forecasts, in hours.
    pub fn worst(&self, route: usize) -> f64 {
        self.cells[route].iter().map(Cell::cost_hours).fold(0.0, f64::max)
    }

    /// How much longer route `route` takes than the best route would, if `forecast` is right.
    pub fn regret(&self, route: usize, forecast: usize) -> f64 {
        let best = self.cells.iter().map(|row| row[forecast].cost_hours()).fold(f64::INFINITY, f64::min);
        self.cells[route][forecast].cost_hours() - best
    }

    /// The most a route can lose against the best route, over all the forecasts.
    pub fn max_regret(&self, route: usize) -> f64 {
        (0..self.cells[route].len()).map(|f| self.regret(route, f)).fold(0.0, f64::max)
    }

    /// Whether a route takes the boat into more wind than its limit in any of the forecasts: sailing a
    /// route already chosen, it cannot go round.
    pub fn over_limit(&self, route: usize) -> bool {
        self.cells[route].iter().any(|c| c.over_limit_hours > 0.0)
    }

    /// The route whose worst case is least bad, among those that keep under the wind limit in every
    /// forecast if any does; ties go to the smaller loss against the best.
    pub fn most_robust(&self) -> Option<usize> {
        (0..self.cells.len()).min_by(|a, b| {
            self.over_limit(*a).cmp(&self.over_limit(*b)).then(self.worst(*a).total_cmp(&self.worst(*b))).then(self.max_regret(*a).total_cmp(&self.max_regret(*b)))
        })
    }
}

/// How far route `a` strays from route `b`: the greatest distance from a point of `a` to the line of
/// `b`, nautical miles.
pub fn separation_nm(a: &[LatLon], b: &[LatLon]) -> f64 {
    if b.len() < 2 {
        return b.first().map_or(0.0, |q| a.iter().map(|p| p.distance_nm(q)).fold(0.0, f64::max));
    }
    a.iter().map(|p| b.windows(2).map(|w| distance_to_segment_nm(&w[0], &w[1], p)).fold(f64::INFINITY, f64::min)).fold(0.0, f64::max)
}

/// Whether the routes go the same way: none strays from another by more than a tenth of the passage
/// (at least `min_nm`). Two ways either side of an island are told apart; the wiggles of tacking are not.
pub fn agree(routes: &[Vec<LatLon>], passage_nm: f64, min_nm: f64) -> bool {
    let limit = (passage_nm * 0.1).max(min_nm);
    routes.iter().enumerate().all(|(i, a)| routes.iter().skip(i + 1).all(|b| separation_nm(a, b).max(separation_nm(b, a)) <= limit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap()
    }

    struct Steady(f64, f64);

    impl Environment for Steady {
        fn wind(&self, _: &LatLon, _: DateTime<Utc>) -> Option<(f64, f64)> {
            Some((self.0, self.1))
        }
    }

    /// From A to B, 30 nm north: straight, or out east first.
    fn routes() -> Vec<Vec<LatLon>> {
        let a = LatLon::new(47.0, -5.0);
        let b = a.destination(0.0, 30.0);
        vec![vec![a, b], vec![a, a.destination(45.0, 21.2), b]]
    }

    #[test]
    fn the_diagonal_and_the_worst_case_are_read_off_the_table() {
        let p = Polar::generic_cruiser();
        // A west wind: the straight route is a reach. A north wind: dead upwind straight, so the dogleg
        // is a better course for the first part.
        let (west, north) = (Steady(12.0, 270.0), Steady(12.0, 0.0));
        let envs: Vec<&dyn Environment> = vec![&west, &north];
        let c = CrossCheck::run(&routes(), &envs, t0(), &p, &Settings::default());
        assert_eq!((c.cells.len(), c.cells[0].len()), (2, 2));
        assert!(c.cells.iter().flatten().all(|x| x.arrives));
        // In a west wind the straight route wins.
        assert!(c.cells[0][0].hours < c.cells[1][0].hours);
        assert_eq!(c.regret(0, 0), 0.0);
        assert!(c.regret(1, 0) > 0.0);
        assert_eq!(c.worst(0), c.cells[0].iter().map(Cell::cost_hours).fold(0.0, f64::max));
        let best = c.most_robust().unwrap();
        assert!(c.worst(best) <= c.worst(1 - best));
    }

    #[test]
    fn a_route_into_a_gale_is_not_the_robust_one_however_fast() {
        let cell = |hours: f64, over: f64| Cell { end: t0(), hours, arrives: true, remaining_nm: 0.0, max_tws_kn: 20.0, motor_hours: 0.0, over_limit_hours: over };
        let c = CrossCheck { cells: vec![vec![cell(10.0, 0.0), cell(10.0, 2.0)], vec![cell(14.0, 0.0), cell(15.0, 0.0)]] };
        assert!(c.over_limit(0) && !c.over_limit(1));
        assert_eq!(c.most_robust(), Some(1));
    }

    #[test]
    fn a_forecast_that_ends_short_costs_the_rest_at_a_slow_speed() {
        let cell = Cell { end: t0() + Duration::hours(10), hours: 10.0, arrives: false, remaining_nm: 40.0, max_tws_kn: 10.0, motor_hours: 0.0, over_limit_hours: 0.0 };
        assert!((cell.cost_hours() - 20.0).abs() < 1e-9);
        assert_eq!(Cell { arrives: true, remaining_nm: 0.0, ..cell }.cost_hours(), 10.0);
    }

    #[test]
    fn routes_either_side_of_an_island_do_not_agree_and_tacks_do_not_matter() {
        let r = routes();
        // The dogleg is 15 nm off the straight line.
        assert!((separation_nm(&r[1], &r[0]) - 15.0).abs() < 0.5, "{}", separation_nm(&r[1], &r[0]));
        assert!(!agree(&r, 30.0, 2.0));
        let a = r[0][0];
        let wiggle = vec![a, a.destination(10.0, 15.0), r[0][1]];
        // 2.6 nm off the line, under a tenth of the passage.
        assert!(agree(&[r[0].clone(), wiggle], 30.0, 2.0));
        assert!(agree(&[r[0].clone(), r[0].clone()], 30.0, 2.0));
    }
}
