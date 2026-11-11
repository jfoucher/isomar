//! Downloads the map tiles of the area in view: the OpenStreetMap coastline
//! and seamarks of the Open Waters tileset.
//!
//! Each is a checkbox in the Layers menu, on by default. While ticked, the
//! tiles under the view are fetched as the user pans and zooms, kept on disk
//! for 30 days and used offline afterwards. One tile carries both, so it is
//! fetched once for either.

use super::app::AppState;
use super::data::Status;
use super::layers::aids::AidsLayer;
use super::layers::land::LandLayer;
use crate::platform::{bg, spawn};
use crate::core::chart::aids::AidsData;
use crate::core::chart::seamark::aids_from_tile;
use crate::core::chart::vtile::{LandTile, TileError, TileId, decode, fetch, forget, prune, read_cache, tiles_for};
use crate::core::datadir;
use crate::core::vector::{Bounds, Polygon};
use crate::i18n::{gettext, gettext_f};
use rayon::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

/// The Layers menu entries that switch the downloads on and off.
pub const LAND_LAYER: &str = "online-land";
pub const MARKS_LAYER: &str = "online-seamarks";

/// Tiles fetched at once.
pub(super) const WORKERS: usize = 4;
/// Most tiles one view can ask for.
pub(super) const MAX_TILES: usize = 64;
/// Wait for the view to settle before asking for tiles.
pub(super) const SETTLE: Duration = Duration::from_millis(250);
/// A tile that failed is left alone for this long.
pub(super) const RETRY_AFTER: Duration = Duration::from_secs(60);
/// A tile that failed for want of a connection is tried again this soon.
pub(super) const RETRY_OFFLINE: Duration = Duration::from_secs(15);
/// Cached tiles older than this are deleted at start.
pub(super) const KEEP: Duration = Duration::from_secs(120 * 24 * 3600);

pub(crate) fn cache_dir() -> PathBuf {
    datadir::cache_dir().join("tiles")
}

/// What is ticked in the Layers menu and not hidden by its main layer:
/// (coastline, seamarks).
fn wanted(app: &AppState) -> (bool, bool) {
    let on = |menu: &str, layer: &str| app.canvas.layer_visible(menu) && app.canvas.layer_visible(layer);
    (on(LAND_LAYER, "land"), on(MARKS_LAYER, "aids"))
}

/// What loading a tile gave, of the kinds asked for.
struct Loaded {
    land: Option<LandTile>,
    marks: Option<AidsData>,
}

/// Reads a tile from disk, or downloads it when it is missing or old and
/// `download` allows. An old copy is used when the download fails.
fn load(dir: &Path, id: TileId, land: bool, marks: bool, download: bool) -> Result<Loaded, TileError> {
    let bytes = match read_cache(dir, &id) {
        Some((b, true)) => b,
        stale => match (download.then(|| fetch(dir, &id)), stale) {
            (Some(Ok(b)), _) => b,
            (_, Some((b, _))) => b,
            (Some(Err(e)), None) => return Err(e),
            (None, None) => return Err(TileError::Offline),
        },
    };
    let tile = decode(&bytes).inspect_err(|_| forget(dir, &id))?;
    Ok(Loaded { land: land.then(|| LandTile::from_tile(&tile)), marks: marks.then(|| aids_from_tile(&id, &tile)) })
}

struct Controller {
    app: Weak<AppState>,
    land: Rc<LandLayer>,
    aids: Rc<AidsLayer>,
    status: Status,
    dir: PathBuf,
    settle: crate::platform::Settle,
    queue: RefCell<VecDeque<TileId>>,
    in_flight: RefCell<HashSet<TileId>>,
    workers: Cell<usize>,
    /// Tiles that failed, and when to try them again.
    failed: RefCell<HashMap<TileId, Instant>>,
    /// An error was shown since the downloads last started.
    complained: Cell<bool>,
}

pub fn install(app: &Rc<AppState>, land: Rc<LandLayer>, aids: Rc<AidsLayer>, status: Status) {
    let dir = cache_dir();
    {
        let dir = dir.clone();
        // Tiles nobody has looked at for months, and the Overpass cache of an earlier version.
        spawn(async move {
            bg(move || {
                prune(&dir, KEEP);
                let _ = std::fs::remove_dir_all(datadir::cache_dir().join("seamarks"));
            })
            .await;
        });
    }
    let ctl = Rc::new(Controller {
        app: Rc::downgrade(app),
        land,
        aids,
        status,
        dir,
        settle: Default::default(),
        queue: RefCell::new(VecDeque::new()),
        in_flight: RefCell::new(HashSet::new()),
        workers: Cell::new(0),
        failed: RefCell::new(HashMap::new()),
        complained: Cell::new(false),
    });
    {
        let ctl = ctl.clone();
        app.canvas.connect_view_changed(move |_| ctl.schedule());
    }
    app.canvas.connect_layers_changed(move || ctl.schedule());
}

impl Controller {
    fn schedule(self: &Rc<Self>) {
        let ticket = self.settle.ask();
        let ctl = self.clone();
        crate::platform::timeout_once(SETTLE, move || {
            if ctl.settle.due(ticket) {
                ctl.run();
            }
        });
    }

    /// Whether the layers still lack `id`, of the kinds that are ticked.
    fn needs(&self, id: &TileId, (land, marks): (bool, bool)) -> bool {
        (land && !self.land.has_tile(id)) || (marks && !self.aids.has_tile(id))
    }

    /// Queues the tiles of the current view that the layers lack, nearest the
    /// centre first, and starts the workers.
    fn run(self: &Rc<Self>) {
        let Some(app) = self.app.upgrade() else { return };
        let want = wanted(&app);
        self.land.set_online(want.0);
        self.aids.set_online(want.1);
        if !want.0 && !want.1 {
            self.queue.borrow_mut().clear();
            return;
        }
        let vi = app.canvas.view_info();
        let ids: VecDeque<TileId> = {
            let (failed, flying) = (self.failed.borrow(), self.in_flight.borrow());
            tiles_for(&vi.bounds(), TileId::zoom_for(vi.zoom()), MAX_TILES)
                .into_iter()
                .filter(|t| self.needs(t, want) && !flying.contains(t))
                .filter(|t| failed.get(t).is_none_or(|at| *at <= Instant::now()))
                .collect()
        };
        *self.queue.borrow_mut() = ids;
        while self.workers.get() < WORKERS && !self.queue.borrow().is_empty() {
            if self.workers.get() == 0 {
                self.complained.set(false);
                (self.status)(&gettext("Loading map tiles…"));
            }
            self.workers.set(self.workers.get() + 1);
            spawn(self.clone().work());
        }
    }

    async fn work(self: Rc<Self>) {
        loop {
            let Some(app) = self.app.upgrade() else { break };
            let want = wanted(&app);
            let next = self.queue.borrow_mut().pop_front();
            let Some(id) = next else { break };
            // The view may have moved on, or the layer been switched off, since it was queued.
            if !self.needs(&id, want) {
                continue;
            }
            self.in_flight.borrow_mut().insert(id);
            let (dir, download) = (self.dir.clone(), !crate::debug::offline());
            let res = bg(move || load(&dir, id, want.0, want.1, download)).await;
            self.in_flight.borrow_mut().remove(&id);
            match res {
                Ok(loaded) => {
                    if let Some(land) = loaded.land {
                        self.land.add_tile(id, std::sync::Arc::new(land));
                    }
                    if let Some(marks) = loaded.marks {
                        self.aids.add_tile(id, std::sync::Arc::new(marks));
                    }
                }
                Err(e) => {
                    let wait = if e == TileError::NoConnection { RETRY_OFFLINE } else { RETRY_AFTER };
                    self.failed.borrow_mut().insert(id, Instant::now() + wait);
                    if e.worth_telling() && !self.complained.replace(true) {
                        (self.status)(&gettext_f("Could not load map tiles: {error}", &[("error", &e.to_string())]));
                    }
                    self.stand_in_for(id, want).await;
                }
            }
        }
        self.workers.set(self.workers.get() - 1);
        if self.workers.get() == 0 {
            if !self.complained.get() {
                (self.status)("");
            }
            // The view may have changed meanwhile.
            self.schedule();
            // Look again once the wait for a failed tile is over, so that tiles
            // lost to a dropped connection come once it is back.
            let soonest = self.failed.borrow().values().copied().filter(|at| *at > Instant::now()).min();
            if let Some(at) = soonest {
                let ctl = self.clone();
                crate::platform::timeout_once(at - Instant::now(), move || ctl.schedule());
            }
        }
    }
}

impl Controller {
    /// Without the tile itself, loads the nearest coarser one from disk, which
    /// the layers then draw in its place.
    async fn stand_in_for(&self, id: TileId, want: (bool, bool)) {
        let mut candidates = Vec::new();
        let mut parent = id.parent();
        while let Some(p) = parent.filter(|_| candidates.len() < 6) {
            // One the layers already have is what they draw.
            if !self.needs(&p, want) {
                return;
            }
            candidates.push(p);
            parent = p.parent();
        }
        let dir = self.dir.clone();
        let found = bg(move || candidates.into_iter().find_map(|p| Some((p, load(&dir, p, want.0, want.1, false).ok()?)))).await;
        if let Some((p, loaded)) = found {
            if let Some(land) = loaded.land {
                self.land.add_tile(p, std::sync::Arc::new(land));
            }
            if let Some(marks) = loaded.marks {
                self.aids.add_tile(p, std::sync::Arc::new(marks));
            }
        }
    }
}

/// The coastline of the tiles under `b`, for the router, to go with the local
/// stores' own, and how many of those tiles could not be had (not downloaded
/// and not allowed or not able to download): where a tile is missing the
/// router sees open water. Downloads what is missing if the coastline is ticked.
pub async fn land_polygons(app: &Rc<AppState>, b: Bounds) -> (Vec<Polygon>, usize) {
    // Coarse tiles are enough for the router, and keep the count down.
    let mut ids = tiles_for(&b, 7, 400);
    if ids.len() > 60 {
        ids = tiles_for(&b, 5, 400);
    }
    tile_land(app, ids).await
}

/// The downloaded land over a small area at zoom 10 (about 40 m a tile
/// unit), fine enough for a river or a harbour: the router's view of the
/// ends of a long passage.
pub async fn detailed_land_polygons(app: &Rc<AppState>, b: Bounds) -> (Vec<Polygon>, usize) {
    tile_land(app, tiles_for(&b, 10, 25)).await
}

async fn tile_land(app: &Rc<AppState>, ids: Vec<TileId>) -> (Vec<Polygon>, usize) {
    let download = app.canvas.layer_visible(LAND_LAYER) && !crate::debug::offline();
    let dir = cache_dir();
    bg(move || {
        let loaded: Vec<Option<Polygon>> = ids
            .par_iter()
            .map(|id| match load(&dir, *id, true, false, download) {
                Ok(l) => l.land.map(|land| land.polygon(id)),
                Err(e) => {
                    isomar_core::log!("route: land tile {}/{}/{} missing: {e}", id.z, id.x, id.y);
                    None
                }
            })
            .collect();
        let missing = loaded.iter().filter(|l| l.is_none()).count();
        (loaded.into_iter().flatten().collect(), missing)
    })
    .await
}

/// What to tell the user when the router had no coastline for `n` tiles.
pub fn missing_land_message(n: usize) -> String {
    crate::i18n::gettext_f(
        "The coastline could not be loaded for {n} map tiles, so the router sees open water there and its routes may cross land. Check “Download coastline” in the Layers menu and the connection, then route again.",
        &[("n", &n.to_string())],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::chart::vtile::{testutil::*, write_cache};

    /// A tile with a square of land and one buoy.
    fn tile_bytes() -> Vec<u8> {
        let mut b = layer("land", 4096, &[F { kind: 3, paths: vec![vec![[0, 0], [4096, 0], [4096, 4096], [0, 4096]]], tags: vec![] }]);
        b.extend(layer("seamark", 4096, &[F { kind: 1, paths: vec![vec![[10, 10]]], tags: vec![("seamark:type", V::S("buoy_lateral"))] }]));
        b
    }

    #[test]
    fn a_cached_tile_loads_without_a_network_and_only_what_is_asked() {
        let d = tempfile::tempdir().unwrap();
        let id = TileId { z: 9, x: 249, y: 177 };
        write_cache(d.path(), &id, &tile_bytes()).unwrap();
        let both = load(d.path(), id, true, true, false).unwrap();
        assert_eq!(both.land.map(|l| l.rings.len()), Some(1));
        assert_eq!(both.marks.map(|m| m.aids.len()), Some(1));
        let land = load(d.path(), id, true, false, false).unwrap();
        assert!(land.land.is_some() && land.marks.is_none());
        let marks = load(d.path(), id, false, true, false).unwrap();
        assert!(marks.land.is_none() && marks.marks.is_some());
    }

    #[test]
    fn a_missing_tile_offline_is_not_an_error_worth_reporting() {
        let d = tempfile::tempdir().unwrap();
        let err = load(d.path(), TileId { z: 9, x: 1, y: 1 }, true, true, false).err();
        assert_eq!(err, Some(TileError::Offline));
    }

    #[test]
    fn an_empty_tile_is_open_sea() {
        let d = tempfile::tempdir().unwrap();
        let id = TileId { z: 12, x: 1, y: 1 };
        write_cache(d.path(), &id, b"").unwrap();
        let l = load(d.path(), id, true, true, false).unwrap();
        assert!(l.land.unwrap().is_empty() && l.marks.unwrap().aids.is_empty());
    }

    #[test]
    fn a_corrupt_tile_is_forgotten_so_the_next_try_downloads_it_again() {
        let d = tempfile::tempdir().unwrap();
        let id = TileId { z: 9, x: 249, y: 177 };
        write_cache(d.path(), &id, b"<html>busy</html>").unwrap();
        assert!(matches!(load(d.path(), id, true, true, false), Err(TileError::Format(_))));
        assert!(read_cache(d.path(), &id).is_none());
    }
}

/// Live check on the coast and tiles in the cache (downloading what is
/// missing): `cargo test --release up_the_auray -- --ignored --nocapture`.
#[cfg(test)]
mod live {
    use super::*;
    use crate::core::chart::coast::{CoastStore, LOD_TOLERANCE_DEG};
    use crate::core::geo::LatLon;
    use crate::core::routing::isochrone::{ConstantWind, explore};
    use crate::core::routing::land::{DetailedLand, Land, RasterLand};
    use crate::core::routing::polar::Polar;

    fn polygons(b: &Bounds, z: u8) -> Vec<Polygon> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(datadir::cache_dir()).unwrap().flatten() {
            let p = e.path();
            if p.file_name().unwrap().to_string_lossy().starts_with("coast-")
                && let Ok(s) = CoastStore::open(&p)
                && s.bounds().intersects(b)
            {
                out.extend(s.query(b, LOD_TOLERANCE_DEG.len() - 1).unwrap());
            }
        }
        for id in tiles_for(b, z, 400) {
            if let Ok(l) = load(&cache_dir(), id, true, false, true) {
                out.extend(l.land.map(|land| land.polygon(&id)));
            }
        }
        out
    }

    /// Saint-Goustan, in the Auray river: inside a 0.01 degree mask, and up a
    /// river too narrow for hour-long legs.
    #[test]
    #[ignore]
    fn a_long_passage_reaches_up_the_auray() {
        let dest = LatLon::new(47.6637, -2.9781);
        let big = Bounds { min_lon: dest.lon - 1.0, max_lon: dest.lon + 1.0, min_lat: dest.lat - 1.0, max_lat: dest.lat + 1.0 };
        let fine = Bounds { min_lon: dest.lon - 0.3, max_lon: dest.lon + 0.3, min_lat: dest.lat - 0.2, max_lat: dest.lat + 0.2 };
        let land = DetailedLand::new(RasterLand::from_polygons(big, 0.01, &polygons(&big, 5), 1), vec![RasterLand::from_polygons(fine, 0.0005, &polygons(&fine, 10), 1)]);
        assert!(land.base.is_land(&dest));
        let way = land.approach(dest, 2.0, 3.0).expect("a way out");
        eprintln!("way out: {} points, {:.1} nm, to {:?}", way.len(), way.windows(2).map(|w| w[0].distance_nm(&w[1])).sum::<f64>(), way.last());
        for from in [0.0, 90.0, 180.0, 270.0] {
            let mut s = crate::flows::routing::settings_for_distance(3700.0);
            s.max_hours = 200.0;
            let ex = explore(LatLon::new(47.0, -3.5), dest, chrono::Utc::now(), &Polar::generic_cruiser(), &ConstantWind { kn: 12.0, from }, &land, &s, None).unwrap();
            let r = ex.arrived.unwrap_or_else(|| panic!("no arrival with the wind from {from}"));
            assert_eq!(r.nodes.last().unwrap().pos, dest);
        }
    }

    /// The router as the weather-route dialog runs it: the way by water on a
    /// coarse mask in ever wider areas, the mask round it with fine masks at
    /// the ends and wherever it cuts the way, then constant winds from four
    /// quarters and a week of real forecast. Each is also run without the
    /// way by water (straight-line progress, as before it), for comparison.
    fn passage(name: &str, start: LatLon, dest: LatLon) {
        use crate::core::routing::isochrone::{Bounded, Environment, StoreEnv};
        use crate::core::routing::seaway::{self, SeaField};
        use crate::core::weather::cache::Cache;
        use crate::core::weather::download::fetch_plan;
        use crate::core::weather::planner::{self, candidate_runs, plan};
        use crate::flows::routing::{land_resolution, settings_for_distance};
        /// The same mask without the way by water.
        struct Crow<'a>(&'a DetailedLand);
        impl Land for Crow<'_> {
            fn blocked(&self, a: &LatLon, b: &LatLon) -> bool {
                self.0.blocked(a, b)
            }
            fn approach(&self, p: LatLon, clear_nm: f64, snap_nm: f64) -> Option<Vec<LatLon>> {
                self.0.approach(p, clear_nm, snap_nm)
            }
            fn seaway(&self, _: LatLon) -> Option<std::sync::Arc<SeaField>> {
                None
            }
        }
        let around = |pts: &[LatLon], m: f64| {
            let mut b = Bounds::EMPTY;
            for p in pts {
                b.extend([p.lon, p.lat]);
            }
            Bounds { min_lon: b.min_lon - m, max_lon: b.max_lon + m, min_lat: b.min_lat - m, max_lat: b.max_lat + m }
        };
        let zoom = |b: &Bounds| if tiles_for(b, 7, 400).len() > 60 { 5 } else { 7 };
        let direct = start.distance_nm(&dest);
        // As `passage_area`: the shortest way, then the box round every way up to a quarter longer where it is a detour.
        let (i, path) = [2.0, 5.0, 10.0]
            .into_iter()
            .enumerate()
            .find_map(|(i, m)| {
                let wide = around(&[start, dest], m);
                seaway::sea_path(wide, &polygons(&wide, zoom(&wide)), start, dest, 1_000_000).map(|p| (i, p))
            })
            .expect("a way by water");
        let length = |p: &[LatLon]| p.windows(2).map(|w| w[0].distance_nm(&w[1])).sum::<f64>();
        let mut reach = path.clone();
        if let Some((from, to)) = crate::flows::routing::offshore(&path).filter(|_| crate::flows::routing::is_detour(length(&path), direct)) {
            let wider = around(&[start, dest], crate::flows::routing::wider_margin(i));
            let w = seaway::sea_ways(wider, &polygons(&wider, zoom(&wider)), from, to, 1_000_000).expect("the other ways");
            eprintln!("  other ways lie within {:?}", w.corridor);
            reach.extend([LatLon::new(w.corridor.min_lat, w.corridor.min_lon), LatLon::new(w.corridor.max_lat, w.corridor.max_lon)]);
        }
        let sea_nm = length(&path).max(direct);
        let b = around(&reach, crate::flows::routing::area_margin_deg(sea_nm));
        let (mut s, res) = (settings_for_distance(sea_nm), land_resolution(sea_nm));
        // Constant wind has no forecast end: an ocean crossing takes longer than the default 17 days.
        s.max_hours = s.max_hours.max(sea_nm / 2.0);
        // BEHIND_SHARE=0 for the old fixed distance, to compare.
        if let Some(v) = std::env::var("BEHIND_SHARE").ok().and_then(|v| v.parse().ok()) {
            s.behind_share = v;
        }
        eprintln!("{name}: way by water {sea_nm:.0} nm ({:.0} direct), area {b:?}, mask {res}, cells {:.1} nm", start.distance_nm(&dest), s.cell_nm);
        let detail = |p: LatLon| Bounds { min_lon: p.lon - 0.26, max_lon: p.lon + 0.26, min_lat: p.lat - 0.2, max_lat: p.lat + 0.2 };
        let mut land = DetailedLand::new(
            RasterLand::from_polygons(b, res, &polygons(&b, zoom(&b)), 1),
            [start, dest].map(|p| RasterLand::from_polygons(detail(p), 0.0005, &polygons(&detail(p), 10), 1)).into(),
        );
        while let Some(cut) = land.cut_area(&path) {
            eprintln!("  cut at {cut:?}");
            land.detail.push(RasterLand::from_polygons(cut, 0.0005, &polygons(&cut, 10), 1));
            assert!(land.detail.len() < 9);
        }
        let run = |what: &str, env: &dyn Environment, s: &crate::core::routing::isochrone::Settings, depart| {
            for (how, l) in [("by water", &land as &dyn Land), ("crow    ", &Crow(&land))] {
                let t = std::time::Instant::now();
                let ex = explore(start, dest, depart, &Polar::generic_cruiser(), env, l, s, None).unwrap();
                let r = ex.arrived.as_ref();
                // How far the front reaches to the side of the direct line, halfway and three quarters through.
                let side = |f: usize| {
                    let k = (ex.fronts.len() * f / 4).saturating_sub(1);
                    ex.fronts.get(k).map_or(0.0, |fr| fr.iter().map(|&i| crate::core::nav::calc::distance_to_segment_nm(&start, &dest, &ex.tree[i].node.pos)).fold(0.0, f64::max))
                };
                let (north, south) = ex.tree.iter().fold((-90.0f64, 90.0f64), |(n, s), t| (n.max(t.node.pos.lat), s.min(t.node.pos.lat)));
                eprintln!("  {what:>16} {how}: searched {south:.1}..{north:.1} N, {:>3} fronts (widest {} points, {:.0}/{:.0} nm to the side) in {:>5} ms, {}", ex.fronts.len(), ex.fronts.iter().map(Vec::len).max().unwrap_or(0), side(2), side(3), t.elapsed().as_millis(), r.map_or(format!("no arrival, stalled {}", ex.stalled), |r| format!("{:.0} nm in {} h, as far north as {:.1}", r.distance_nm, r.duration().num_hours(), r.nodes.iter().map(|n| n.pos.lat).fold(-90.0, f64::max))));
                // FRONTS=<dir> writes the fronts and the route as an SVG per run.
                if let (Ok(dir), "by water") = (std::env::var("FRONTS"), how) {
                    let k = (b.min_lat + b.max_lat).to_radians().mul_add(0.5, 0.0).cos();
                    let xy = |p: &LatLon| ((p.lon - b.min_lon) * k * 100.0, (b.max_lat - p.lat) * 100.0);
                    let line = |pts: &mut dyn Iterator<Item = LatLon>, style: &str| {
                        let d: Vec<String> = pts.map(|p| xy(&p)).map(|(x, y)| format!("{x:.1},{y:.1}")).collect();
                        format!("<polyline {style} points='{}'/>\n", d.join(" "))
                    };
                    let (w, h) = xy(&LatLon::new(b.min_lat, b.max_lon));
                    let mut svg = format!("<svg xmlns='http://www.w3.org/2000/svg' width='{w:.0}' height='{h:.0}'><rect width='100%' height='100%' fill='white'/>\n");
                    for z in polygons(&b, 5) {
                        for ring in &z.rings {
                            svg += &line(&mut ring.iter().map(|q| LatLon::new(q[1], q[0])), "stroke='#bbb' fill='#eee'");
                        }
                    }
                    for k in 0..ex.fronts.len() {
                        for run in ex.front_runs(k) {
                            svg += &line(&mut run.iter().map(|&i| ex.tree[i].node.pos), "fill='none' stroke='#36c' stroke-width='0.4'");
                        }
                    }
                    if let Some(r) = r {
                        svg += &line(&mut r.nodes.iter().map(|n| n.pos), "fill='none' stroke='red' stroke-width='1.5'");
                    }
                    std::fs::write(format!("{dir}/{} {what}.svg", name.replace(' ', "_")), svg + "</svg>").unwrap();
                }
                // A week of forecast is not enough for the longest passages.
                let forecast_ended = r.is_none() && !ex.stalled && ex.horizon.is_some_and(|h| env.horizon().is_some_and(|l| h >= l));
                if how == "by water" && !forecast_ended {
                    let r = r.unwrap_or_else(|| panic!("{name}, {what}: no arrival"));
                    // The ends may be on a quay, which the mask fills.
                    for w in r.nodes.windows(2).filter(|w| w[0].pos != start && w[1].pos != dest) {
                        assert!(!land.blocked(&w[0].pos, &w[1].pos), "{name}, {what}: crosses land from {:?} to {:?}", w[0].pos, w[1].pos);
                    }
                    assert!(r.distance_nm < sea_nm * 1.5, "{name}, {what}: {}", r.distance_nm);
                }
            }
        };
        for from in [0.0, 90.0, 180.0, 270.0] {
            run(&format!("12 kn from {from}"), &ConstantWind { kn: 12.0, from }, &s, chrono::Utc::now());
        }
        // A week of wind over the passage, downloaded afresh into a scratch cache.
        let now = chrono::Utc::now();
        let mut req = planner::route_request(planner::Source::Gfs, &[start, dest, LatLon::new(b.min_lat, b.min_lon), LatLon::new(b.max_lat, b.max_lon)]).unwrap();
        req.groups = vec![crate::core::weather::model::Group::Wind];
        req.horizon_hours = 168;
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let fetched = candidate_runs(now).into_iter().find_map(|run| fetch_plan(&plan(&req, run, now), &cache, &std::sync::atomic::AtomicBool::new(false), &|_| {}).ok().filter(|f| f.missing.is_empty()));
        if fetched.is_none() {
            eprintln!("  no forecast could be downloaded");
            return;
        }
        let (store, _) = cache.load(now, chrono::Duration::hours(72));
        let env = Bounded { env: &StoreEnv(&store), bounds: b };
        let mut motor = s.clone();
        motor.motor_speed_kn = Some(5.0);
        run("forecast", &env, &s, now);
        run("forecast, engine", &env, &motor, now);
    }

    /// Napoli to Split: the way leaves the box round the ends and sails away
    /// from Split for two days, south round Calabria and through Messina
    /// (which a 0.01 degree mask closes), then up the Adriatic.
    /// `cargo test --release passages -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn passages_napoli_to_split() {
        let (start, dest) = (LatLon::new(40.83, 14.25), LatLon::new(43.50, 16.43));
        let near = Bounds { min_lon: 12.25, max_lon: 18.43, min_lat: 38.83, max_lat: 45.5 };
        assert!(crate::core::routing::seaway::sea_path(near, &polygons(&near, 7), start, dest, 1_000_000).is_none(), "no way within 2 degrees");
        passage("Napoli → Split", start, dest);
    }

    #[test]
    #[ignore]
    fn passages_split_to_napoli() {
        passage("Split → Napoli", LatLon::new(43.50, 16.43), LatLon::new(40.83, 14.25));
    }

    /// Round Iberia: the bearing from the start turns through 130 degrees.
    #[test]
    #[ignore]
    fn passages_donostia_to_ibiza() {
        passage("Donostia → Ibiza", LatLon::new(43.33, -1.99), LatLon::new(38.90, 1.45));
    }

    /// The passages the router was tuned on, which must not get worse.
    /// An ocean crossing: the front must stay wide enough to pick a strategy
    /// from (`BEHIND_SHARE=0` runs it with the old fixed distance).
    #[test]
    #[ignore]
    fn passages_canarias_to_salvador() {
        passage("Las Palmas → Salvador", LatLon::new(28.13, -15.43), LatLon::new(-12.97, -38.51));
    }

    /// Across Britain: the way by water is round Land's End or round the
    /// north of Scotland, both over 600 nm; the direct line is 130.
    #[test]
    #[ignore]
    fn passages_liverpool_to_hull() {
        passage("Liverpool → Hull", LatLon::new(53.40, -3.03), LatLon::new(53.74, -0.33));
    }

    #[test]
    #[ignore]
    fn passages_tuned_on() {
        passage("Marseille → Gibraltar", LatLon::new(43.29, 5.36), LatLon::new(36.13, -5.35));
        passage("Mahón → Cagliari", LatLon::new(39.89, 4.27), LatLon::new(39.20, 9.11));
        passage("Palma → Cagliari", LatLon::new(39.55, 2.64), LatLon::new(39.20, 9.11));
    }

    /// Every place the demo puts something (`debug::DEMO_AREAS`) against the coastline, with room to spare at
    /// chart scale: at zoom 11 a mile is 36 points, the chart draws drying banks round the land these
    /// polygons lack, and waypoints 250 m off an island were drawn on its shore. The boat under way and the
    /// route's waypoints a mile or more from land; the legs, the marks and the traffic where the screenshot
    /// shows it half a mile; the traffic's tracks over the 20 minutes after the start (clear or crossing)
    /// never across land; the moored boat afloat in its harbour within 300 m of the quay. `DEMO_MAP=<dir>`
    /// draws each area with its points.
    /// `cargo test --release -p isomar-app the_demo_areas -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn the_demo_areas_are_at_sea() {
        use crate::core::comm::sim::Simulator;
        use crate::debug::DEMO_AREAS;
        let mut bad = Vec::new();
        for a in DEMO_AREAS {
            let ll = |(_, lat, lon): &(&str, f64, f64)| LatLon::new(*lat, *lon);
            // What must have open water round it, and how much, in nautical miles.
            let mut room: Vec<(String, LatLon, f64)> = vec![("boat under way".into(), a.at_sea.pos(), 1.0)];
            room.extend(a.route.iter().map(|p| (format!("waypoint {}", p.0), ll(p), 1.0)));
            room.extend(a.marks.iter().map(|p| (format!("mark {}", p.0), ll(p), 0.5)));
            for w in a.route.windows(2) {
                let (x, y) = (ll(&w[0]), ll(&w[1]));
                let n = (x.distance_nm(&y) / 0.05).ceil() as usize;
                room.extend((0..=n).map(|k| (format!("leg {} to {}", w[0].0, w[1].0), x.destination(x.bearing_to(&y), x.distance_nm(&y) * k as f64 / n as f64), 0.5)));
            }
            let mut tracks: Vec<(String, LatLon, LatLon)> = Vec::new();
            let b = a.at_sea;
            for (how, mut sim) in [("clear", Simulator::demo_clear(b.pos(), b.cog, b.sog, chrono::Utc::now())), ("crossing", Simulator::demo(b.pos(), b.cog, b.sog, chrono::Utc::now()))] {
                let mut last: Vec<(String, LatLon)> = sim.targets().map(|(n, p)| (format!("{n} ({how})"), p)).collect();
                room.extend(last.iter().map(|(n, p)| (n.clone(), *p, 0.5)));
                last.insert(0, ("the boat".into(), sim.pos));
                for _ in 0..20 {
                    for _ in 0..60 {
                        sim.step(1.0);
                    }
                    let now: Vec<(String, LatLon)> = std::iter::once(("the boat".into(), sim.pos)).chain(sim.targets().map(|(n, p)| (format!("{n} ({how})"), p))).collect();
                    tracks.extend(last.iter().zip(&now).map(|(x, y)| (format!("track of {}", x.0), x.1, y.1)));
                    last = now;
                }
            }
            let mut bounds = Bounds::EMPTY;
            for (_, p, _) in &room {
                bounds.extend([p.lon, p.lat]);
            }
            bounds.extend([a.harbour.lon, a.harbour.lat]);
            let bounds = bounds.expanded(0.05);
            // About 22 m by 33 m a cell, grown by two: land and the 60 m round it.
            let mask = RasterLand::from_polygons(bounds, 0.0003, &polygons(&bounds, 12), 2);
            // No land along 72 rays of `nm` from `p`: with the mask's growth, nothing is missed between them.
            let clear = |p: &LatLon, nm: f64| (0..72).all(|k| !mask.blocked(p, &p.destination(k as f64 * 5.0, nm)));
            let mut seen = std::collections::HashSet::new();
            for (what, p, nm) in &room {
                if !clear(p, *nm) && seen.insert(what.clone()) {
                    bad.push(format!("{}: {what} at {:.4},{:.4} is within {nm} nm of land", a.name, p.lat, p.lon));
                }
            }
            for (what, x, y) in &tracks {
                if mask.blocked(x, y) && seen.insert(what.clone()) {
                    bad.push(format!("{}: {what} from {:.4},{:.4} to {:.4},{:.4} crosses land", a.name, x.lat, x.lon, y.lat, y.lon));
                }
            }
            // The harbour on the finest tiles alone, as the chart draws it close up: afloat, near the quay.
            let h = a.harbour.pos();
            let hb = Bounds { min_lon: h.lon - 0.01, max_lon: h.lon + 0.01, min_lat: h.lat - 0.007, max_lat: h.lat + 0.007 };
            let fine: Vec<Polygon> = tiles_for(&hb, 14, 16).into_iter().filter_map(|id| load(&cache_dir(), id, true, false, true).ok()?.land.map(|l| l.polygon(&id))).collect();
            let hmask = RasterLand::from_polygons(hb, 0.00005, &fine, 0);
            if hmask.is_land(&h) {
                bad.push(format!("{}: the moored boat at {:.4},{:.4} is on land", a.name, h.lat, h.lon));
            }
            let near_quay = (0..36).any(|k| hmask.blocked(&h, &h.destination(k as f64 * 10.0, 0.16)));
            if !near_quay {
                bad.push(format!("{}: the moored boat at {:.4},{:.4} is not within 300 m of land: not in a harbour", a.name, h.lat, h.lon));
            }
            if let Ok(dir) = std::env::var("DEMO_MAP") {
                let points: Vec<(String, LatLon)> = room.iter().filter(|r| !r.0.starts_with("leg ")).map(|r| (r.0.clone(), r.1)).collect();
                draw_map(&format!("{dir}/{}.ppm", a.name), &mask, &points, &tracks, h);
                draw_map(&format!("{dir}/{}-harbour.ppm", a.name), &hmask, &[], &[], h);
            }
        }
        // The passages the store scenes route (`ISOMAR_ROUTE`): both ends afloat on the finest tiles.
        let scenes = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../design/stores/tools/scenes");
        for e in std::fs::read_dir(&scenes).unwrap().flatten() {
            let text = std::fs::read_to_string(e.path()).unwrap_or_default();
            for v in text.lines().filter_map(|l| l.strip_prefix("ISOMAR_ROUTE=")) {
                let n: Vec<f64> = v.split(',').map(|x| x.trim().parse().unwrap()).collect();
                for p in [LatLon::new(n[0], n[1]), LatLon::new(n[2], n[3])] {
                    let b = Bounds { min_lon: p.lon - 0.01, max_lon: p.lon + 0.01, min_lat: p.lat - 0.007, max_lat: p.lat + 0.007 };
                    let fine: Vec<Polygon> = tiles_for(&b, 14, 16).into_iter().filter_map(|id| load(&cache_dir(), id, true, false, true).ok()?.land.map(|l| l.polygon(&id))).collect();
                    // Grown by two cells of 5 m: afloat with 10 m to spare, as a harbour entrance may be narrow.
                    if RasterLand::from_polygons(b, 0.00005, &fine, 2).is_land(&p) {
                        bad.push(format!("{:?}: ISOMAR_ROUTE end {:.4},{:.4} is on land", e.file_name(), p.lat, p.lon));
                    }
                }
            }
        }
        assert!(bad.is_empty(), "\n{}", bad.join("\n"));
    }

    /// The mask in grey (south down), the points red, the tracks blue, the harbour boat green.
    fn draw_map(path: &str, mask: &RasterLand, points: &[(String, LatLon)], tracks: &[(String, LatLon, LatLon)], harbour: LatLon) {
        let (cols, rows, cells) = mask.grid();
        let (b, res) = (mask.bounds(), mask.res());
        let px = 2.0;
        let surf = cairo::ImageSurface::create(cairo::Format::Rgb24, (cols as f64 * px) as i32, (rows as f64 * px) as i32).unwrap();
        let cr = cairo::Context::new(&surf).unwrap();
        cr.set_source_rgb(0.85, 0.93, 1.0);
        cr.paint().unwrap();
        cr.set_source_rgb(0.75, 0.7, 0.55);
        for r in 0..rows {
            for c in 0..cols {
                if cells[r * cols + c] {
                    cr.rectangle(c as f64 * px, (rows - 1 - r) as f64 * px, px, px);
                }
            }
        }
        cr.fill().unwrap();
        let xy = |p: &LatLon| ((p.lon - b.min_lon) / res * px, (b.max_lat - p.lat) / res * px);
        // A graticule every 0.01 degree, darker every 0.05.
        let mut v = (b.min_lon * 100.0).ceil() / 100.0;
        while v < b.max_lon {
            let major = ((v * 100.0).round() as i64) % 5 == 0;
            cr.set_source_rgba(0.0, 0.0, 0.0, if major { 0.5 } else { 0.12 });
            let (x, _) = xy(&LatLon::new(b.min_lat, v));
            cr.move_to(x, 0.0);
            cr.line_to(x, rows as f64 * px);
            cr.stroke().unwrap();
            v += 0.01;
        }
        let mut v = (b.min_lat * 100.0).ceil() / 100.0;
        while v < b.max_lat {
            let major = ((v * 100.0).round() as i64) % 5 == 0;
            cr.set_source_rgba(0.0, 0.0, 0.0, if major { 0.5 } else { 0.12 });
            let (_, y) = xy(&LatLon::new(v, b.min_lon));
            cr.move_to(0.0, y);
            cr.line_to(cols as f64 * px, y);
            cr.stroke().unwrap();
            v += 0.01;
        }
        cr.set_source_rgb(0.1, 0.3, 0.9);
        for (_, x, y) in tracks {
            let ((x0, y0), (x1, y1)) = (xy(x), xy(y));
            cr.move_to(x0, y0);
            cr.line_to(x1, y1);
        }
        cr.stroke().unwrap();
        cr.set_source_rgb(0.9, 0.1, 0.1);
        for (name, p) in points {
            let (x, y) = xy(p);
            cr.arc(x, y, 4.0, 0.0, 6.3);
            cr.fill().unwrap();
            cr.move_to(x + 6.0, y - 4.0);
            cr.show_text(name).unwrap();
        }
        cr.set_source_rgb(0.0, 0.6, 0.1);
        let (x, y) = xy(&harbour);
        cr.arc(x, y, 3.0, 0.0, 6.3);
        cr.fill().unwrap();
        drop(cr);
        // As a PPM: this crate's cairo has no PNG writer.
        let (w, h, stride) = (surf.width() as usize, surf.height() as usize, surf.stride() as usize);
        let data = surf.take_data().unwrap();
        let mut out = format!("P6 {w} {h} 255\n").into_bytes();
        for y in 0..h {
            for x in 0..w {
                let p = &data[y * stride + x * 4..];
                out.extend([p[2], p[1], p[0]]);
            }
        }
        std::fs::write(path, out).unwrap();
    }
}
