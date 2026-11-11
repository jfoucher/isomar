//! Replays the weather-route and best-departure flows (`flows::routing::run`, `flows::departures::run`)
//! from the cached land tiles and a frozen forecast, timing each phase, so a change to the router can be
//! measured and its results compared (see `design/perf/`).
//!
//! `cargo run --release -p isomar-core --example routebench -- <name|R11|R12|R14|R20> [lat0 lon0 lat1 lon1] [route|window|both]`
//!
//! - `BENCH_CACHE`: the folder holding `weather/` (default `testdata/bench-weather`, a copy of the cache
//!   taken on 2026-11-02; the cache prunes old runs, so the numbers must come from the same files).
//! - `BENCH_NOW` (RFC 3339, default 2026-11-02T12:00:00Z), `BENCH_DAYS` (3), `BENCH_STEP_H` (3),
//!   `BENCH_REPEAT` (explores of the single route, 2).
//!
//! R20 (Camaret to Audierne, through the Raz de Sein) adds a made-up tidal race there
//! (`currents::synthetic`, 6 kn at springs, the flood strongest two hours after `BENCH_NOW`): the
//! case for the router's currents (`BENCH_NO_RACE=1` leaves the race out, to compare). With a current,
//! as the app does, the boat may wait for the stream (`BENCH_NO_HOLD=1` not) and the best departures are
//! tried again closer together (`BENCH_NO_REFINE=1` not).
//!
//! Land comes from the tiles in `~/.cache/isomar/tiles` only (the local coast stores are left out).
//! Each run ends with a `result` line: the same line means the same routes.
use chrono::{DateTime, Duration, Utc};
use isomar_core::chart::vtile::{LandTile, TileId, decode, read_cache, tiles_for};
use isomar_core::geo::LatLon;
use isomar_core::routing::isochrone::{self, Bounded, Exploration, StoreEnv};
use isomar_core::routing::land::{DetailedLand, Land, RasterLand};
use isomar_core::routing::passage::{self, DETAIL_RES, SEARCH_MARGINS_DEG, around};
use isomar_core::routing::seaway;
use isomar_core::routing::window::{self, Weights};
use isomar_core::vector::{Bounds, Polygon};
use isomar_core::weather::cache::Cache;
use rayon::prelude::*;
use std::path::PathBuf;
use std::time::Instant;

/// The owner's saved routes the baseline was measured on.
const ROUTES: [(&str, [f64; 4]); 4] = [
    ("R11", [38.6111691348187, -76.4204890756444, 47.6198357299395, -2.78051507312256]),
    ("R12", [53.4582554402235, -3.23625850400205, 53.585021600352, 0.0381608037066883]),
    ("R14", [36.6112727763698, -2.78456256993229, 36.1131839351861, -5.40160885906369]),
    ("R20", [48.282, -4.605, 48.005, -4.535]),
];

/// The made-up race of R20, in the Raz de Sein.
fn raz_de_sein(now: DateTime<Utc>) -> isomar_core::currents::synthetic::Race {
    isomar_core::currents::synthetic::Race { centre: LatLon::new(48.045, -4.735), axis_deg: 15.0, half_width_nm: 1.0, half_length_nm: 3.0, peak_kn: 6.0, flood_at: now + Duration::hours(2) }
}

fn tile_land(ids: Vec<TileId>) -> (Vec<Polygon>, usize) {
    let dir = isomar_core::datadir::cache_dir().join("tiles");
    let loaded: Vec<Option<Polygon>> = ids
        .par_iter()
        .map(|id| {
            let (bytes, _) = read_cache(&dir, id)?;
            let t = decode(&bytes).ok()?;
            Some(LandTile::from_tile(&t).polygon(id))
        })
        .collect();
    let missing = loaded.iter().filter(|l| l.is_none()).count();
    (loaded.into_iter().flatten().collect(), missing)
}

/// As `tiles::land_polygons`.
fn land_polygons(b: Bounds) -> (Vec<Polygon>, usize) {
    let mut ids = tiles_for(&b, 7, 400);
    if ids.len() > 60 {
        ids = tiles_for(&b, 5, 400);
    }
    tile_land(ids)
}

/// As `tiles::detailed_land_polygons`.
fn detailed(b: Bounds) -> (Vec<Polygon>, usize) {
    tile_land(tiles_for(&b, 10, 25))
}

fn ms(t: Instant) -> String {
    format!("{:.0} ms", t.elapsed().as_secs_f64() * 1000.0)
}

/// What an exploration found, to compare runs: arrival time and distance, or where it stopped.
fn summary(ex: &Exploration) -> String {
    match &ex.arrived {
        Some(r) => format!("arrives after {} min, {:.2} nm", r.duration().num_minutes(), r.distance_nm),
        None => match ex.nearest {
            Some(n) => format!("stops {:.2} nm short ({:.4} made good)", n.remaining_nm, n.share_made()),
            None => "nothing".into(),
        },
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let name = a.first().cloned().unwrap_or_else(|| "R14".into());
    let (ends, rest) = match ROUTES.iter().find(|(n, _)| *n == name) {
        Some((_, e)) => (*e, &a[1..]),
        None => {
            let n = |i: usize| a.get(i).and_then(|s| s.parse::<f64>().ok()).expect("<name> <lat0> <lon0> <lat1> <lon1>");
            ([n(1), n(2), n(3), n(4)], &a[5.min(a.len())..])
        }
    };
    let (start, dest) = (LatLon::new(ends[0], ends[1]), LatLon::new(ends[2], ends[3]));
    let mode = rest.first().cloned().unwrap_or_else(|| "both".into());
    let env_f = |k: &str, d: f64| std::env::var(k).ok().and_then(|s| s.parse().ok()).unwrap_or(d);
    let now: DateTime<Utc> = DateTime::parse_from_rfc3339(&std::env::var("BENCH_NOW").unwrap_or_else(|_| "2026-11-02T12:00:00Z".into())).expect("BENCH_NOW").with_timezone(&Utc);
    let cache_root = std::env::var("BENCH_CACHE").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/bench-weather")));
    let total = Instant::now();

    // passage_area
    let t = Instant::now();
    let direct = start.distance_nm(&dest);
    let mut found = None;
    for (i, m) in SEARCH_MARGINS_DEG.into_iter().enumerate() {
        let wide = around(&[start, dest], m);
        let (polys, _) = land_polygons(wide);
        if let Some(path) = seaway::sea_path(wide, &polys, start, dest, 1_000_000) {
            found = Some((i, path));
            break;
        }
    }
    let (area, sea_nm, path) = match found {
        Some((i, path)) => {
            let nm: f64 = path.windows(2).map(|w| w[0].distance_nm(&w[1])).sum();
            let mut reach = path.clone();
            if passage::is_detour(nm, direct)
                && let Some((from, to)) = passage::offshore(&path)
            {
                let wider = around(&[start, dest], passage::wider_margin(i));
                let (polys, _) = land_polygons(wider);
                if let Some(w) = seaway::sea_ways(wider, &polys, from, to, 1_000_000) {
                    reach.extend([LatLon::new(w.corridor.min_lat, w.corridor.min_lon), LatLon::new(w.corridor.max_lat, w.corridor.max_lon)]);
                }
            }
            let nm = nm.max(direct);
            (around(&reach, passage::area_margin_deg(nm)), nm, path)
        }
        None => (around(&[start, dest], passage::area_margin_deg(direct)), direct, Vec::new()),
    };
    println!("{name}: {direct:.0} nm direct, {sea_nm:.0} by water; area {:.1}x{:.1} deg", area.width(), area.height());
    println!("  passage_area     {}", ms(t));

    // prepare_land
    let t = Instant::now();
    let (polys, mut missing) = land_polygons(area);
    let res = passage::land_resolution(sea_nm);
    let mut detail = Vec::new();
    for p in [start, dest] {
        let db = passage::detail_bounds(p);
        let (dp, m) = detailed(db);
        missing += m;
        detail.push((db, dp));
    }
    let mut land = DetailedLand::new(RasterLand::from_polygons(area, res, &polys, 1), detail.iter().map(|(db, p)| RasterLand::from_polygons(*db, DETAIL_RES, p, 1)).collect());
    let mut cuts = 0;
    for _ in 0..6 {
        if path.len() < 2 {
            break;
        }
        let Some(cb) = land.cut_area(&path) else { break };
        let (dp, m) = detailed(cb);
        missing += m;
        land.detail.push(RasterLand::from_polygons(cb, DETAIL_RES, &dp, 1));
        cuts += 1;
    }
    let (cols, rows, _) = land.base.grid();
    println!("  prepare_land     {} ({cols}x{rows} cells at {res}, {cuts} cuts, {missing} tiles missing)", ms(t));
    let ts = Instant::now();
    let _f = land.seaway(dest);
    println!("  one seaway field {}", ms(ts));

    // The forecast, as the app has it loaded.
    let t = Instant::now();
    let (store, files) = Cache::new(&cache_root).load(now, Duration::hours(72));
    let store = store.within(&area);
    let range = store.wind_range_in(Some(&area));
    println!("  forecast load    {} ({files} files from {})", ms(t), cache_root.display());
    let polar = isomar_core::routing::library::current();
    let mut settings = passage::settings_for_distance(sea_nm);
    let raced = isomar_core::currents::synthetic::WithRace { env: &StoreEnv(&store), race: raz_de_sein(now) };
    let base: &dyn isochrone::Environment = if name == "R20" && std::env::var_os("BENCH_NO_RACE").is_none() { &raced } else { &StoreEnv(&store) };
    let env = Bounded { env: base, bounds: area };
    // As `flows::currents::tide_settings`: with a current, holds, and shorter legs where it is strong.
    let peak = passage::peak_current_kn(&env, &path, now, 13.0);
    if isochrone::Environment::has_currents(&env) {
        passage::for_currents(&mut settings, sea_nm, peak);
        settings.allow_hold = std::env::var_os("BENCH_NO_HOLD").is_none();
        println!("  currents         peak {peak:.1} kn along the way, legs of {} min, holds {}", settings.step_minutes, if settings.allow_hold { "on" } else { "off" });
    }
    let mut result = Vec::new();

    if mode == "route" || mode == "both" {
        for rep in 0..env_f("BENCH_REPEAT", 2.0) as usize {
            let t = Instant::now();
            match isochrone::explore(start, dest, now, &polar, &env, &land, &settings, None) {
                Ok(ex) => {
                    println!("  explore #{rep}       {} ({} fronts, {} tree nodes, widest {}, {})", ms(t), ex.fronts.len(), ex.tree.len(), ex.fronts.iter().map(Vec::len).max().unwrap_or(0), summary(&ex));
                    if rep == 0 {
                        result.push(summary(&ex));
                    }
                }
                Err(e) => println!("  explore #{rep}       {} failed: {e}", ms(t)),
            }
        }
    }
    if mode == "window" || mode == "both" {
        let latest = range.map(|(_, hi)| hi - Duration::minutes(((sea_nm / 8.0).min(24.0) * 60.0) as i64));
        let deps = window::departures(now, env_f("BENCH_DAYS", 3.0) * 24.0, env_f("BENCH_STEP_H", 3.0), latest);
        let t = Instant::now();
        match window::scan(start, dest, &deps, &polar, &env, &land, &settings, &Weights::comfortable(), None) {
            Ok(mut scan) => {
                let ranked = scan.ranked();
                let best = ranked.first().map(|&i| scan.candidates[i].depart.format("%d %H:%M").to_string()).unwrap_or_default();
                println!("  window scan      {} ({} departures, {} with a passage, best {best})", ms(t), deps.len(), ranked.len());
                // As `flows::departures`: where a stream decides it, the best departures again, closer together.
                if peak >= window::REFINE_PEAK_KN && std::env::var_os("BENCH_NO_REFINE").is_none() {
                    let t = Instant::now();
                    let mut added = 0;
                    for (top, hours) in window::REFINE {
                        let tried: Vec<_> = scan.candidates.iter().map(|c| c.depart).collect();
                        let costs: Vec<_> = scan.candidates.iter().map(|c| c.passage().map(|p| p.cost)).collect();
                        let more = window::around_best(&tried, &costs, top, hours, now, latest);
                        added += more.len();
                        if let Ok(extra) = window::scan_since(now, start, dest, &more, &polar, &env, &land, &settings, &Weights::comfortable(), None) {
                            scan = scan.merge(extra);
                        }
                    }
                    println!("  refine           {} ({added} more departures)", ms(t));
                }
                let ranked = scan.ranked();
                let best = ranked.first().map(|&i| scan.candidates[i].depart.format("%d %H:%M").to_string()).unwrap_or_default();
                let stamp = |t: DateTime<Utc>| if t.format("%M").to_string() == "00" { t.format("%d%H").to_string() } else { t.format("%d%H%M").to_string() };
                let costs: Vec<String> = ranked.iter().take(10).map(|&i| format!("{}:{:.1}", stamp(scan.candidates[i].depart), scan.candidates[i].passage().map_or(0.0, |p| p.cost))).collect();
                result.push(format!("best {best}, ten best {}", costs.join(" ")));
            }
            Err(e) => println!("  window scan      {} failed: {e}", ms(t)),
        }
    }
    println!("  total            {}", ms(total));
    println!("  result           {}", result.join("; "));
}
