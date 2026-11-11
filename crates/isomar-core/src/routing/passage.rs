//! The parameters a weather route gets from its length, and the boxes it is
//! searched in: shared by the app's routing flows and the route benchmark.

use super::isochrone::{Environment, Settings};
use chrono::{DateTime, Utc};
use crate::geo::LatLon;
use crate::vector::Bounds;

/// Time step and pruning cells suited to the length of the passage.
pub fn settings_for_distance(nm: f64) -> Settings {
    let step_minutes = if nm < 60.0 {
        15.0
    } else if nm < 300.0 {
        30.0
    } else {
        60.0
    };
    let leg_nm = 8.0 * step_minutes / 60.0;
    // A few cells to a leg, so most legs leave the cells the last front reached.
    // An ocean passage keeps a much wider front (see `behind_share`), so its
    // cells are bigger and the number of points stays about the same.
    let cell_nm = if nm < OCEAN_NM { leg_nm / 4.0 } else { leg_nm / 2.0 };
    Settings { step_minutes, cell_nm, ..Settings::default() }
}

/// From this length a passage is an ocean crossing: wide fronts, bigger cells.
const OCEAN_NM: f64 = 1000.0;

/// Legs no longer than this where the stream runs hard: a race a mile or two wide is otherwise crossed
/// in one leg, its current read once.
pub const TIDAL_STEP_MINUTES: f64 = 15.0;
/// A stream at least this strong somewhere along the way shortens the legs...
pub const TIDAL_PEAK_KN: f64 = 2.0;
/// ...on a passage shorter than this (an ocean one has few races, and fronts too wide to afford it).
pub const TIDAL_MAX_NM: f64 = 300.0;

/// `settings_for_distance` for a passage of `nm` whose strongest current along the way is `peak_kn`:
/// the legs are cut to `TIDAL_STEP_MINUTES` where the stream is strong, the cells with them.
pub fn for_currents(s: &mut Settings, nm: f64, peak_kn: f64) {
    if peak_kn >= TIDAL_PEAK_KN && nm < TIDAL_MAX_NM && s.step_minutes > TIDAL_STEP_MINUTES {
        let shorter = s.step_minutes / TIDAL_STEP_MINUTES;
        s.step_minutes = TIDAL_STEP_MINUTES;
        s.cell_nm /= shorter;
    }
}

/// The strongest current along `path` (sampled about every half mile, a race being a mile or two wide) at any whole hour over the first tidal
/// cycle from `from`, or over `hours` if shorter: what decides `for_currents`.
pub fn peak_current_kn(env: &dyn Environment, path: &[LatLon], from: DateTime<Utc>, hours: f64) -> f64 {
    if !env.has_currents() || path.is_empty() {
        return 0.0;
    }
    let mut points = vec![path[0]];
    for w in path.windows(2) {
        let n = ((2.0 * w[0].distance_nm(&w[1])).ceil() as usize).clamp(1, 200);
        points.extend((1..=n).map(|i| w[0].destination(w[0].bearing_to(&w[1]), w[0].distance_nm(&w[1]) * i as f64 / n as f64)));
    }
    let stride = points.len().div_ceil(400).max(1);
    let cycle = hours.clamp(1.0, 13.0).ceil() as i64;
    let mut peak: f64 = 0.0;
    for h in 0..=cycle {
        let t = from + chrono::Duration::hours(h);
        for p in points.iter().step_by(stride) {
            if let Some((e, n)) = env.current(p, t) {
                peak = peak.max(e.hypot(n));
            }
        }
    }
    peak
}

/// Degrees per land-mask cell for a passage: finer for short ones.
pub fn land_resolution(nm: f64) -> f64 {
    if nm < 60.0 { 0.002 } else if nm < 300.0 { 0.004 } else { 0.01 }
}

/// The land mask around the ends of a passage, where the way in and out
/// follows rivers and harbours: about 50 m a cell. At 0.01 degree a cell and
/// its margin are 1.3 km across and fill the Auray river and most of the
/// Gulf of Morbihan.
pub const DETAIL_RES: f64 = 0.0005;

/// The area around an end of a passage that gets the finest land mask.
pub fn detail_bounds(p: LatLon) -> Bounds {
    let dlat = 0.2;
    let dlon = dlat / p.lat.to_radians().cos().max(0.2);
    Bounds { min_lon: p.lon - dlon, max_lon: p.lon + dlon, min_lat: p.lat - dlat, max_lat: p.lat + dlat }
}

/// Margin round the way by water (or the ends) for the router's search, at least.
const AREA_MARGIN_DEG: f64 = 2.0;

/// The margin grows with the passage, a tenth of its length at most 8 degrees,
/// so a long one east–west or north–south, whose box is a thin strip, still
/// leaves room for the strategies that go well to either side of the way.
pub fn area_margin_deg(sea_nm: f64) -> f64 {
    (sea_nm / 60.0 * 0.1).clamp(AREA_MARGIN_DEG, 8.0)
}

/// Widening margins round the ends tried in turn to find the way by water.
pub const SEARCH_MARGINS_DEG: [f64; 3] = [2.0, 5.0, 10.0];

/// The box round `points`, `margin` degrees wider on every side (short of the poles).
pub fn around(points: &[LatLon], margin: f64) -> Bounds {
    let mut b = Bounds::EMPTY;
    for p in points {
        b.extend([p.lon, p.lat]);
    }
    Bounds { min_lon: b.min_lon - margin, max_lon: b.max_lon + margin, min_lat: (b.min_lat - margin).max(-80.0), max_lat: (b.max_lat + margin).min(80.0) }
}

/// A way by water this much longer than the direct line goes round land,
/// and the passage may have another way round (Liverpool → Hull is round
/// Land's End or round Scotland): then the search area holds all the ways.
const DETOUR_RATIO: f64 = 1.3;

/// Whether a way of `sea_nm` between ends `direct_nm` apart is a detour.
pub fn is_detour(sea_nm: f64, direct_nm: f64) -> bool {
    sea_nm > direct_nm * DETOUR_RATIO
}

/// The margin to look for other ways in once the shortest was found within
/// `SEARCH_MARGINS_DEG[i]`: the next one out, so a way that goes a little
/// farther from the ends than the shortest does is not cut at the edge.
pub fn wider_margin(i: usize) -> f64 {
    SEARCH_MARGINS_DEG[(i + 1).min(SEARCH_MARGINS_DEG.len() - 1)]
}

/// How far out from each end along the way the other ways are searched
/// between: the wider box has a coarse coast, which fills a harbour or a river
/// (Hull is up the Humber) and cuts its water off from the sea.
pub const OFFSHORE_NM: f64 = 40.0;

/// The points of `path` that far from its start and from its end, or `None`
/// when the way is too short to leave room between them.
pub fn offshore(path: &[LatLon]) -> Option<(LatLon, LatLon)> {
    let walk = |pts: &mut dyn Iterator<Item = &LatLon>| {
        let (mut last, mut run) = (*pts.next()?, 0.0);
        for p in pts {
            run += last.distance_nm(p);
            last = *p;
            if run >= OFFSHORE_NM {
                return Some(*p);
            }
        }
        None
    };
    let (from, to) = (walk(&mut path.iter())?, walk(&mut path.iter().rev())?);
    (from.distance_nm(&to) > OFFSHORE_NM).then_some((from, to))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_detour_is_a_way_much_longer_than_the_direct_line() {
        assert!(is_detour(761.0, 98.0));
        assert!(!is_detour(120.0, 98.0));
        assert!(!is_detour(100.0, 100.0));
        // The wider box is the next one out, and the last stays.
        assert_eq!(wider_margin(0), SEARCH_MARGINS_DEG[1]);
        assert_eq!(wider_margin(2), SEARCH_MARGINS_DEG[2]);
    }

    #[test]
    fn the_other_ways_are_searched_between_points_off_the_coast() {
        // A way east along a parallel, a point a mile.
        let path: Vec<LatLon> = (0..=120).map(|i| LatLon::new(50.0, i as f64 / 60.0 / 0.64)).collect();
        let (from, to) = offshore(&path).unwrap();
        assert!((path[0].distance_nm(&from) - OFFSHORE_NM).abs() < 2.0, "{}", path[0].distance_nm(&from));
        assert!((path[path.len() - 1].distance_nm(&to) - OFFSHORE_NM).abs() < 2.0);
        assert!(from.lon < to.lon);
        // Too short to leave room between the two.
        assert!(offshore(&path[..60]).is_none());
        assert!(offshore(&path[..1]).is_none());
    }

    #[test]
    fn short_passages_get_finer_steps_and_grids() {
        assert_eq!(settings_for_distance(20.0).step_minutes, 15.0);
        assert!(settings_for_distance(2800.0).cell_nm > settings_for_distance(500.0).cell_nm);
        assert_eq!(area_margin_deg(100.0), 2.0);
        assert_eq!(area_margin_deg(2800.0), 4.0 + 2.0 / 3.0);
        assert_eq!(area_margin_deg(20000.0), 8.0);
        assert_eq!(settings_for_distance(100.0).step_minutes, 30.0);
        assert_eq!(settings_for_distance(1000.0).step_minutes, 60.0);
        assert!(land_resolution(20.0) < land_resolution(100.0));
        assert!(land_resolution(100.0) < land_resolution(1000.0));
    }

    #[test]
    fn a_strong_stream_on_the_way_shortens_the_legs_of_a_coastal_passage() {
        let mut s = settings_for_distance(120.0);
        let (step, cell) = (s.step_minutes, s.cell_nm);
        for_currents(&mut s, 120.0, 1.0);
        assert_eq!((s.step_minutes, s.cell_nm), (step, cell), "a weak stream changes nothing");
        for_currents(&mut s, 120.0, 4.0);
        assert_eq!(s.step_minutes, TIDAL_STEP_MINUTES);
        assert!((s.cell_nm - cell / 2.0).abs() < 1e-9);
        let mut ocean = settings_for_distance(1500.0);
        for_currents(&mut ocean, 1500.0, 4.0);
        assert_eq!(ocean.step_minutes, 60.0, "not on an ocean passage");
    }

    #[test]
    fn the_peak_current_is_found_along_the_way_over_a_tide() {
        use crate::currents::synthetic::{Race, WithRace};
        use crate::routing::isochrone::ConstantWind;
        use chrono::TimeZone;
        let t0 = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        let wind = ConstantWind { kn: 10.0, from: 0.0 };
        let centre = LatLon::new(48.0, -5.0);
        // The race is slack when the passage starts, strongest three hours later.
        let race = Race { centre, axis_deg: 0.0, half_width_nm: 1.0, half_length_nm: 3.0, peak_kn: 5.0, flood_at: t0 + chrono::Duration::hours(3) };
        let env = WithRace { env: &wind, race };
        let path = [centre.destination(270.0, 10.0), centre.destination(90.0, 10.0)];
        let peak = peak_current_kn(&env, &path, t0, 24.0);
        assert!(peak > 4.5 && peak <= 5.0, "{peak}");
        assert_eq!(peak_current_kn(&wind, &path, t0, 24.0), 0.0);
    }
}
