//! The chart's static layers, drawn as tiles.
//!
//! The view is cut into tiles of 256 points aligned to the world grid at the view's own zoom (kept to
//! hundredths, `view::quantize_zoom`). A tile is drawn by running every static layer's `TilePaint`, in order,
//! with a small view of its own (the tile and a margin, so that labels and symbols straddling an edge are
//! drawn whole by both neighbours). Tiles are drawn on a pool of worker threads, kept in a cache bounded in
//! bytes, and a pan draws only the tiles it exposes. While a tile is missing, tiles of other zooms stand in,
//! scaled.
//!
//! The layers split their work: `Layer::prepare` runs on the main thread, once for the whole region wanted,
//! and snapshots what the tiles are painted from; `TilePaint::paint` runs on a worker for one tile.

use crate::core::geo;
use crate::core::vector::Bounds;
use crate::layers::overlay::Overlay;
use crate::layers::{DrawCtx, Layer, TileArea, TilePaint};
use crate::palette::{Palette, Tone};
use crate::view::{View, ViewInfo};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Edge of a tile, in points.
pub const TILE: i32 = 256;
/// Tells every drawing of a tile, of any plane, from any other.
static NEXT_TILE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// What a tile stopped by its `Job::cancel` comes back with.
const CANCELLED: &str = "cancelled";
const TILE_F: f64 = TILE as f64;
/// How far past its edge a tile is drawn, in points.
pub const MARGIN: f64 = 64.0;
/// Bytes of tiles kept: a tile is 256 KB at scale 1 and 1 MB at scale 2.
const CACHE_BYTES: usize = 192 << 20;
/// Tiles kept at most, those with nothing on them included.
const MAX_HELD: usize = 4096;
/// Tiles of other zooms stand in for missing ones, from this many hundredths of a zoom coarser or finer.
/// Finer ones are drawn smaller, and a zoom of them is four tiles for one.
const FALLBACK_COARSER: i32 = 300;
const FALLBACK_FINER: i32 = 100;
/// While the zoom keeps changing (a pinch, a smooth scroll), tiles are drawn only at multiples of this
/// many hundredths, below the view's zoom, and shown enlarged at most twice; the view's own zoom is
/// drawn once it has stopped for `ZOOM_SETTLE`. Drawing every hundredth the zoom passed through kept
/// every worker busy with tiles that were never shown.
const ZOOM_STEP: i32 = 100;
const ZOOM_SETTLE: Duration = Duration::from_millis(150);
/// How far past either end of its whole zoom a zoom that keeps changing goes on being drawn at it: names
/// and marks are placed again at each whole zoom (`names::label_zoom`, `names::mark_zoom`), and a pinch
/// that wavered across one swapped them back and forth at every wobble of the fingers.
const ZOOM_STICK: i32 = 15;
/// Tiles drawn from data that has since changed are drawn again once the data has been still this long,
/// or has kept changing for `STALE_WAIT`: data often arrives a tile at a time, and each arrival drew
/// every tile it touched again.
const STALE_SETTLE: Duration = Duration::from_millis(120);
/// How long stale tiles wait at most while a layer is still loading what the view needs.
const LOADING_WAIT: Duration = Duration::from_millis(1500);
const STALE_WAIT: Duration = Duration::from_millis(500);

/// A tile: the zoom in hundredths, and its column and row at that zoom. Columns past the world's edge are
/// kept as they are: the layers wrap the world themselves, so such a tile shows the other side.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct TileKey {
    pub level: i32,
    pub tx: i32,
    pub ty: i32,
}

impl TileKey {
    pub fn zoom(&self) -> f64 {
        self.level as f64 / 100.0
    }

    /// The view a tile is drawn with: centred on the tile, with a margin all round.
    pub fn view_info(&self) -> ViewInfo {
        let z = self.zoom();
        let center = geo::unproject((self.tx as f64 + 0.5) * TILE_F, (self.ty as f64 + 0.5) * TILE_F, z);
        ViewInfo { view: View { center, zoom: z }, w: TILE_F + 2.0 * MARGIN, h: TILE_F + 2.0 * MARGIN }
    }

    /// The part of the world the tile and its margin show, in degrees; `None` for a tile past the
    /// antimeridian, whose longitudes do not compare with the layers'.
    pub fn bounds(&self) -> Option<Bounds> {
        let z = self.zoom();
        let (x0, y0) = (self.tx as f64 * TILE_F - MARGIN, self.ty as f64 * TILE_F - MARGIN);
        let (x1, y1) = (x0 + TILE_F + 2.0 * MARGIN, y0 + TILE_F + 2.0 * MARGIN);
        let size = geo::world_size(z);
        if x0 < 0.0 || x1 > size {
            return None;
        }
        let (nw, se) = (geo::unproject(x0, y0.max(0.0), z), geo::unproject(x1, y1.min(size), z));
        Some(Bounds { min_lon: nw.lon, min_lat: se.lat, max_lon: se.lon, max_lat: nw.lat })
    }
}

/// The tile level of a zoom.
fn level_of(zoom: f64) -> i32 {
    (zoom * 100.0).round() as i32
}

/// Whether a zoom that keeps changing, now at `exact`, goes on being drawn at `level`: while it stays
/// within `ZOOM_STICK` of the whole zoom `level` is in, so that tiles are shown enlarged little more than
/// twice, and is not more than half a zoom out from it, past which a zoom out would draw many tiles of it.
fn sticks(level: i32, exact: i32) -> bool {
    let whole = level.div_euclid(ZOOM_STEP) * ZOOM_STEP;
    (whole - ZOOM_STICK..whole + ZOOM_STEP + ZOOM_STICK).contains(&exact) && exact >= level - ZOOM_STEP / 2
}

/// Whether tiles of `level` may stand for a view at `exact`: enlarged at most `FALLBACK_COARSER`
/// hundredths of a zoom, shrunk at most `FALLBACK_FINER`.
fn shows(level: i32, exact: i32) -> bool {
    (-FALLBACK_COARSER..=FALLBACK_FINER).contains(&(level - exact))
}

/// The zooms the names and the marks of tiles of `level` are placed at: tiles of levels that share them
/// show the same names in the same places, only grown or shrunk.
fn step_of(level: i32) -> (f64, f64) {
    let z = level as f64 / 100.0;
    (crate::layers::names::label_zoom(z), crate::layers::names::mark_zoom(z))
}

/// How the tiles look, apart from the view and the data: when any of this changes, they are all drawn again.
#[derive(Clone, PartialEq, Debug)]
pub struct Look {
    pub scale: i32,
    pub text_scale: u32,
    pub tone: Tone,
    pub hidden: Vec<String>,
}

/// What a batch of tiles is painted from.
pub struct Snapshot {
    id: u64,
    /// The `Tiles` it is for (`Tiles::plane`): planes share the workers and their queue.
    plane: u8,
    generation: u64,
    /// Each layer's `revision` when the snapshot was taken.
    revs: Vec<u64>,
    paints: Vec<(&'static str, Arc<dyn TilePaint>)>,
    pal: Palette,
    scale: i32,
}

struct Job {
    key: TileKey,
    snap: Arc<Snapshot>,
    /// Set when the tile is no longer wanted: its worker stops at the next layer.
    cancel: Arc<AtomicBool>,
}

/// A drawn tile on its way from a worker.
pub struct Done {
    key: TileKey,
    snap: Arc<Snapshot>,
    result: Result<cairo::ImageSurfaceDataOwned, String>,
    took: Duration,
    times: Vec<(&'static str, Duration)>,
}

#[derive(Default)]
struct State {
    queue: Vec<Job>,
    /// Taken by a worker, and not yet received by the main thread.
    running: Vec<(TileKey, Arc<Snapshot>, Arc<AtomicBool>)>,
    stop: bool,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

fn lock(shared: &Shared) -> MutexGuard<'_, State> {
    shared.state.lock().unwrap_or_else(|e| e.into_inner())
}

struct Pool {
    shared: Arc<Shared>,
}

impl Pool {
    fn start(workers: usize, results: async_channel::Sender<Done>) -> Pool {
        // Before any tile thread draws text: cairo's list of faces is not safe to add to from several.
        crate::fonts::warm_up();
        let shared = Arc::new(Shared { state: Mutex::new(State::default()), wake: Condvar::new() });
        for i in 0..workers {
            let (shared, results) = (shared.clone(), results.clone());
            let spawned = std::thread::Builder::new().name(format!("isomar-tile-{i}")).spawn(move || worker(shared, results));
            if let Err(e) = spawned {
                eprintln!("isomar: cannot start a tile thread: {e}");
            }
        }
        Pool { shared }
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        lock(&self.shared).stop = true;
        self.shared.wake.notify_all();
    }
}

fn worker(shared: Arc<Shared>, results: async_channel::Sender<Done>) {
    loop {
        let job = {
            let mut st = lock(&shared);
            loop {
                if st.stop {
                    return;
                }
                if !st.queue.is_empty() {
                    let job = st.queue.remove(0);
                    st.running.push((job.key, job.snap.clone(), job.cancel.clone()));
                    break job;
                }
                st = shared.wake.wait(st).unwrap_or_else(|e| e.into_inner());
            }
        };
        if results.send_blocking(paint_tile(job.key, job.snap, &job.cancel)).is_err() {
            return;
        }
    }
}

/// How many threads draw tiles: all the cores but one, between 1 and 8 (4 on a phone or tablet, whose
/// chips have only a few fast cores among slow ones, and where the slow ones would hold tiles back and
/// spend the battery); `ISOMAR_TILE_THREADS` says otherwise (0 draws them on the main thread, as the
/// tests do).
pub fn default_workers() -> usize {
    let most = if cfg!(any(target_os = "ios", target_os = "android")) { 4 } else { 8 };
    std::env::var("ISOMAR_TILE_THREADS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(2, |n| n.get().saturating_sub(1)).clamp(1, most))
}

/// The font options a tile is drawn with: no hinting.
fn tile_font_options() -> Result<cairo::FontOptions, String> {
    let mut fonts = cairo::FontOptions::new().map_err(|e| e.to_string())?;
    fonts.set_hint_style(cairo::HintStyle::None);
    Ok(fonts)
}

/// Draws one tile. Runs on any thread; a layer that panics leaves its part of the tile empty. Stops
/// between layers once `cancel` is set.
fn paint_tile(key: TileKey, snap: Arc<Snapshot>, cancel: &AtomicBool) -> Done {
    let started = Instant::now();
    let mut times = Vec::new();
    let px = TILE * snap.scale;
    let result = cairo::ImageSurface::create(cairo::Format::ARgb32, px, px).map_err(|e| e.to_string()).and_then(|surface| {
        surface.set_device_scale(snap.scale as f64, snap.scale as f64);
        {
            let cr = cairo::Context::new(&surface).map_err(|e| e.to_string())?;
            cr.translate(-MARGIN, -MARGIN);
            // FreeType's autohinter (what the usual "slight" hinting runs) keeps one loader for the whole
            // library, not one per face, so two tile threads drawing text in different files of Adwaita
            // Sans (regular for soundings, italic for names) corrupted it and crashed in
            // `af_face_globals_new` / `af_latin_metrics_init`. A tile is a texture the shells scale and
            // place, so hinting to the pixel grid was never worth it: no hinting never reaches the autohinter.
            cr.set_font_options(&tile_font_options()?);
            let overlay = Overlay::default();
            let ctx = DrawCtx { cr: &cr, vi: key.view_info(), pal: snap.pal, overlay: &overlay, reduce_motion: false, top_inset: 0.0 };
            for (id, paint) in &snap.paints {
                if cancel.load(Ordering::Relaxed) {
                    return Err(CANCELLED.to_string());
                }
                let t = Instant::now();
                let _ = cr.save();
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| paint.paint(&ctx))).is_err() {
                    eprintln!("isomar: layer {id} failed to draw tile {key:?}");
                }
                let _ = cr.restore();
                times.push((*id, t.elapsed()));
            }
        }
        surface.take_data().map_err(|e| e.to_string())
    });
    Done { key, snap, result, took: started.elapsed(), times }
}

struct Tile {
    /// None when no layer had anything to draw on it.
    surface: Option<cairo::ImageSurface>,
    /// Tells this drawing of the tile from any other, for a shell that keeps it on the GPU.
    id: u64,
    /// The layers' revisions it was drawn with.
    revs: Vec<u64>,
    bytes: usize,
    /// The frame that last wanted it.
    used: u64,
}

/// A tile to show: where its top-left corner goes, in points from the view's top-left corner, how big it
/// is there, and the rectangle it is cut to when it stands in for missing tiles of another zoom.
pub struct Placement<'a> {
    pub key: TileKey,
    /// Changes whenever the tile is drawn again.
    pub id: u64,
    pub surface: &'a cairo::ImageSurface,
    pub x: f64,
    pub y: f64,
    pub size: f64,
    pub clip: Option<[f64; 4]>,
}

/// What `Tiles::update` did.
#[derive(Default)]
pub struct Planned {
    /// Tiles were asked for.
    pub any: bool,
    /// Ask again after this long: tiles were held back while the zoom or the data were still moving.
    pub again_in: Option<Duration>,
}

/// What `Tiles::update` needs of a frame.
pub struct Frame<'a> {
    pub vi: ViewInfo,
    pub pal: Palette,
    pub look: Look,
    /// The static layers that are shown, back to front.
    pub layers: &'a [Rc<dyn Layer>],
}

#[derive(Default)]
pub struct Tiles {
    cache: HashMap<TileKey, Tile>,
    look: Option<Look>,
    bytes: usize,
    /// The bytes of tiles kept, besides those in view.
    limit: usize,
    /// Counts frames, to tell which tiles were wanted last.
    stamp: u64,
    generation: u64,
    next_snapshot: u64,
    pool: Option<Rc<Pool>>,
    /// Which plane of tiles these are: the chart's, or another drawn over it (`Engine`), with workers shared.
    plane: u8,
    /// How long stale tiles wait for data that keeps changing (`STALE_WAIT`).
    stale_wait: Duration,
    /// The view's tile level in the last frame, when it last changed, and whether it is changing still.
    level: i32,
    zoomed_at: Option<Instant>,
    zooming: bool,
    /// The zoom a zoom that keeps changing is drawn at (`ZOOM_STICK`).
    zooming_at: Option<i32>,
    /// The level the view shows, grown or shrunk to its zoom, and the one being drawn to take its place.
    /// The view goes over to another level only once every tile of it in view is drawn, all at once:
    /// placed as they came, a tile at a time over the level shown, the names, lights and soundings of
    /// two levels showed side by side, and came and went as the tiles did.
    shown: Option<i32>,
    target: Option<i32>,
    /// The layers' revisions in the last frame, and when they last changed.
    revs: Vec<u64>,
    revs_at: Option<Instant>,
    /// Since when tiles in view have been waiting to be drawn again from new data.
    stale_since: Option<Instant>,
    /// Stale tiles wait for the data to settle; on with workers, which a redraw picks up from later.
    hold_stale: bool,
    /// Something is drawn into this plane (`clear` says otherwise): its tiles count in `coverage`.
    in_use: bool,
}

impl Tiles {
    pub fn new() -> Tiles {
        Tiles::plane(0, STALE_WAIT)
    }

    /// Tiles of another plane, whose stale tiles wait at most `stale_wait` for data that keeps changing.
    pub fn plane(plane: u8, stale_wait: Duration) -> Tiles {
        Tiles { limit: CACHE_BYTES, plane, stale_wait, ..Tiles::default() }
    }

    /// Draws tiles on `workers` threads, which hand them back through `results`; none draws them on the
    /// calling thread, in `update`.
    pub fn set_workers(&mut self, workers: usize, results: Option<async_channel::Sender<Done>>) {
        self.use_pool(match (workers, results) {
            (0, _) | (_, None) => None,
            (n, Some(tx)) => Some(Rc::new(Pool::start(n, tx))),
        });
    }

    /// Draws tiles on the workers of `other`, whose `Done`s for this plane are handed to `receive`.
    pub fn share_workers(&mut self, other: &Tiles) {
        self.use_pool(other.pool.clone());
    }

    fn use_pool(&mut self, pool: Option<Rc<Pool>>) {
        self.pool = pool;
        self.hold_stale = self.pool.is_some();
        self.generation += 1;
        self.look = None;
    }

    /// The plane a tile that came from the workers belongs to.
    pub fn plane_of(done: &Done) -> u8 {
        done.snap.plane
    }

    pub fn is_threaded(&self) -> bool {
        self.pool.is_some()
    }

    /// Tiles queued or being drawn.
    pub fn pending(&self) -> usize {
        self.pool.as_ref().map_or(0, |p| {
            let st = lock(&p.shared);
            st.queue.iter().filter(|j| j.snap.plane == self.plane).count() + st.running.iter().filter(|r| r.1.plane == self.plane).count()
        })
    }

    pub fn set_limit(&mut self, bytes: usize) {
        self.limit = bytes;
    }

    /// Forgets every tile, as when nothing is drawn into this plane any more.
    pub fn clear(&mut self) {
        self.in_use = false;
        if !self.cache.is_empty() {
            self.cache.clear();
            self.bytes = 0;
            self.generation += 1;
        }
    }

    /// Tiles in the cache, and the bytes they hold.
    pub fn cached(&self) -> (usize, usize) {
        (self.cache.len(), self.bytes)
    }

    /// Whether the tile is held and was drawn with `revs`.
    pub fn is_fresh(&self, key: &TileKey, revs: &[u64]) -> bool {
        self.cache.get(key).is_some_and(|t| t.revs == revs)
    }

    pub fn has(&self, key: &TileKey) -> bool {
        self.cache.contains_key(key)
    }

    /// Keeps a tile that came from a worker. Returns whether it was one worth keeping.
    pub fn receive(&mut self, done: Done) -> bool {
        if let Some(p) = &self.pool {
            lock(&p.shared).running.retain(|(k, s, _)| !(*k == done.key && s.id == done.snap.id && s.plane == done.snap.plane));
        }
        if done.snap.generation != self.generation {
            return false;
        }
        self.insert(done)
    }

    fn insert(&mut self, done: Done) -> bool {
        let data = match done.result {
            Ok(d) => d,
            Err(e) => {
                if e != CANCELLED {
                    eprintln!("isomar: tile {:?} not drawn: {e}", done.key);
                }
                return false;
            }
        };
        let surface = data.into_inner();
        let bytes = surface.stride() as usize * surface.height() as usize;
        let surface = Some(surface);
        if crate::debug::timing() {
            let slow: Vec<String> =
                done.times.iter().filter(|(_, t)| *t >= Duration::from_millis(2)).map(|(id, t)| format!("{id} {:.0}", t.as_secs_f64() * 1000.0)).collect();
            isomar_core::log!("isomar: tile {}/{}/{} {:.0} ms [{}]", done.key.level, done.key.tx, done.key.ty, done.took.as_secs_f64() * 1000.0, slow.join(", "));
        }
        let tile = Tile { surface, id: NEXT_TILE.fetch_add(1, Ordering::Relaxed) + 1, revs: done.snap.revs.clone(), bytes, used: self.stamp };
        if let Some(old) = self.cache.insert(done.key, tile) {
            self.bytes -= old.bytes;
        }
        self.bytes += bytes;
        true
    }

    /// The tiles `vi` shows and the ring of one tile round them, the visible ones first and nearest the
    /// centre first, as (key, visible).
    pub fn wanted(vi: &ViewInfo) -> Vec<(TileKey, bool)> {
        Tiles::wanted_at(vi, level_of(vi.zoom()), true)
    }

    /// The tiles of `level` that show `vi`, scaled to its zoom, with the ring round them if `ring`.
    fn wanted_at(vi: &ViewInfo, level: i32, ring: bool) -> Vec<(TileKey, bool)> {
        let tz = level as f64 / 100.0;
        let k = 2f64.powf(vi.zoom() - tz);
        let (cx, cy) = geo::project(&vi.view.center, tz);
        let (hw, hh) = (vi.w / 2.0 / k, vi.h / 2.0 / k);
        let size = geo::world_size(tz);
        let last_row = ((size - 1e-6) / TILE_F).floor() as i32;
        let first = |v: f64| (v / TILE_F).floor() as i32;
        let (x0, x1) = (first(cx - hw), first(cx + hw - 1e-6));
        let (y0, y1) = (first(cy - hh).max(0), first(cy + hh - 1e-6).min(last_row));
        let mut out = Vec::new();
        let r = ring as i32;
        for ty in (y0 - r).max(0)..=(y1 + r).min(last_row) {
            for tx in x0 - r..=x1 + r {
                let visible = (x0..=x1).contains(&tx) && (y0..=y1).contains(&ty);
                out.push((TileKey { level, tx, ty }, visible));
            }
        }
        let dist = |k: &TileKey| {
            let (dx, dy) = ((k.tx as f64 + 0.5) * TILE_F - cx, (k.ty as f64 + 0.5) * TILE_F - cy);
            dx * dx + dy * dy
        };
        out.sort_by(|a, b| b.1.cmp(&a.1).then(dist(&a.0).total_cmp(&dist(&b.0))));
        out
    }

    /// Plans the tiles for a frame: those missing or drawn with other data than the layers have now are
    /// drawn, on the workers or, with none, here. While the zoom is changing, tiles are drawn only at
    /// `ZOOM_STEP`s; tiles drawn from data that has changed since wait for it to settle.
    pub fn update(&mut self, f: &Frame) -> Planned {
        if self.look.as_ref() != Some(&f.look) {
            self.look = Some(f.look.clone());
            self.generation += 1;
            self.cache.clear();
            self.bytes = 0;
            self.shown = None;
            if let Some(p) = &self.pool {
                let mut st = lock(&p.shared);
                let plane = self.plane;
                st.queue.retain(|j| j.snap.plane != plane);
                for (_, _, cancel) in st.running.iter().filter(|r| r.1.plane == plane) {
                    cancel.store(true, Ordering::Relaxed);
                }
                st.running.retain(|r| r.1.plane != plane);
            }
        }
        self.stamp += 1;
        self.in_use = true;
        let now = Instant::now();
        let mut out = Planned::default();
        let mut wait = |d: Duration| out.again_in = Some(out.again_in.map_or(d, |a| a.min(d)));

        // A zoom that keeps changing is drawn at the step below it; the first change of a still view (a
        // wheel notch, a jump) is drawn at once.
        let exact = level_of(f.vi.zoom());
        let since = self.zoomed_at.map(|t| now - t);
        if exact != self.level {
            self.zooming = since.is_some_and(|s| s < ZOOM_SETTLE);
            self.level = exact;
            self.zoomed_at = Some(now);
        } else if since.is_none_or(|s| s >= ZOOM_SETTLE) {
            self.zooming = false;
        }
        let wanted = if self.zooming {
            wait(ZOOM_SETTLE.saturating_sub(self.zoomed_at.map_or(Duration::ZERO, |t| now - t)));
            // A zoom that starts goes on with the level shown, drawing what it brings into view, until it is
            // too far from it; then the whole zoom below. Starting at the whole zoom below swapped what a
            // pause had drawn for the larger names and soundings of before it, and back at the next pause.
            let at = match self.zooming_at.or(self.shown) {
                Some(l) if sticks(l, exact) => l,
                _ => exact.div_euclid(ZOOM_STEP) * ZOOM_STEP,
            };
            self.zooming_at = Some(at);
            Tiles::wanted_at(&f.vi, at, false)
        } else {
            self.zooming_at = None;
            Tiles::wanted(&f.vi)
        };
        let target = wanted.first().map_or(exact, |(k, _)| k.level);
        self.target = Some(target);
        let keep = self.shown.filter(|&l| l != target && shows(l, exact) && Tiles::wanted_at(&f.vi, l, false).iter().any(|(k, _)| self.cache.contains_key(k)));
        if keep.is_none() {
            self.show(target, exact);
        }
        self.show_if_drawn(&f.vi, target);
        let rank: HashMap<TileKey, usize> = wanted.iter().enumerate().map(|(i, (k, _))| (*k, i)).collect();
        // What is shown stays, whatever is drawn meanwhile.
        let showing: Vec<TileKey> = self.shown.filter(|&l| l != target).map_or_else(Vec::new, |l| Tiles::wanted_at(&f.vi, l, false).into_iter().map(|(k, _)| k).collect());
        let mut keep = rank.clone();
        keep.extend(showing.iter().map(|k| (*k, usize::MAX)));
        for k in wanted.iter().map(|(k, _)| k).chain(&showing) {
            if let Some(t) = self.cache.get_mut(k) {
                t.used = self.stamp;
            }
        }
        if let Some(p) = &self.pool {
            // Tiles the zoom has gone past are not finished.
            for (k, _, cancel) in lock(&p.shared).running.iter().filter(|r| r.1.plane == self.plane) {
                if !rank.contains_key(k) {
                    cancel.store(true, Ordering::Relaxed);
                }
            }
        }
        let revs: Vec<u64> = f.layers.iter().map(|l| l.revision()).collect();
        if revs != self.revs {
            self.revs = revs.clone();
            self.revs_at = Some(now);
        }
        self.keep_untouched(f.layers, &revs, wanted.iter().map(|(k, _)| k));
        let queued: HashMap<TileKey, Vec<u64>> = self.pool.as_ref().map_or_else(HashMap::new, |p| {
            let st = lock(&p.shared);
            let mine = st.queue.iter().filter(|j| j.snap.plane == self.plane).map(|j| (j.key, j.snap.revs.clone()));
            mine.chain(st.running.iter().filter(|r| r.1.plane == self.plane && !r.2.load(Ordering::Relaxed)).map(|(k, s, _)| (*k, s.revs.clone()))).collect()
        });
        let mut need: Vec<TileKey> = wanted.iter().map(|(k, _)| *k).filter(|k| !self.is_fresh(k, &revs) && queued.get(k) != Some(&revs)).collect();
        // Missing tiles are drawn at once; stale ones, which show meanwhile, once the data is still.
        if self.hold_stale && need.iter().any(|k| self.has(k)) {
            let stale_since = *self.stale_since.get_or_insert(now);
            let still = self.revs_at.map_or(Duration::MAX, |t| now - t);
            let settle = STALE_SETTLE.min(self.stale_wait);
            // While a layer is still reading what the view asked for, each piece that arrives would draw the
            // tiles again: they wait longer then (at Boston every tile was drawn five times as cells came in).
            // The data goes still between the pieces, so they are waited for whatever it does meanwhile.
            let loading = f.layers.iter().any(|l| l.loading());
            let waited = now - stale_since;
            let hold = if loading { waited < LOADING_WAIT } else { still < settle && waited < self.stale_wait };
            if hold {
                need.retain(|k| !self.has(k));
                wait(if loading { LOADING_WAIT - waited } else { (settle - still).min(self.stale_wait - waited) });
            } else {
                self.stale_since = None;
            }
        } else {
            self.stale_since = None;
        }
        if !need.is_empty() {
            out.any = true;
            if crate::debug::timing() {
                self.log_redraws(f.layers, &revs, &need);
            }
            let snap = self.snapshot(f, &wanted, revs.clone());
            if snap.paints.is_empty() {
                // Nothing to draw anywhere: kept as drawn, with no image to place.
                for key in need {
                    self.cache.insert(key, Tile { surface: None, id: 0, revs: revs.clone(), bytes: 0, used: self.stamp });
                }
                self.evict(&keep);
                return out;
            }
            match &self.pool {
                Some(p) => {
                    let mut st = lock(&p.shared);
                    // Another plane's jobs stay as they are, ahead.
                    let (mut queue, mut mine): (Vec<Job>, Vec<Job>) = std::mem::take(&mut st.queue).into_iter().partition(|j| j.snap.plane != self.plane);
                    mine.retain(|j| j.snap.generation == self.generation && j.snap.revs == revs && rank.contains_key(&j.key));
                    mine.extend(need.into_iter().map(|key| Job { key, snap: snap.clone(), cancel: Arc::default() }));
                    mine.sort_by_key(|j| rank[&j.key]);
                    queue.extend(mine);
                    st.queue = queue;
                    drop(st);
                    p.shared.wake.notify_all();
                }
                None => {
                    for key in need {
                        let done = paint_tile(key, snap.clone(), &AtomicBool::new(false));
                        self.insert(done);
                    }
                    self.show_if_drawn(&f.vi, target);
                }
            }
        } else if let Some(p) = &self.pool {
            let mut st = lock(&p.shared);
            let plane = self.plane;
            st.queue.retain(|j| j.snap.plane != plane || rank.contains_key(&j.key));
        }
        self.evict(&keep);
        out
    }

    /// Shows `level` once every tile of it in view is drawn.
    fn show_if_drawn(&mut self, vi: &ViewInfo, level: i32) {
        if self.shown != Some(level) && Tiles::wanted_at(vi, level, false).iter().all(|(k, _)| self.cache.contains_key(k)) {
            self.show(level, level_of(vi.zoom()));
        }
    }

    fn show(&mut self, level: i32, exact: i32) {
        if self.shown != Some(level) && crate::debug::timing() {
            isomar_core::log!("isomar: plane {} shows {} at {}", self.plane, level, exact);
        }
        self.shown = Some(level);
    }

    /// Marks the tiles among `keys` that no change since they were drawn touched as drawn from the data
    /// the layers have now.
    fn keep_untouched<'k>(&mut self, layers: &[Rc<dyn Layer>], revs: &[u64], keys: impl Iterator<Item = &'k TileKey>) {
        let mut asked: HashMap<(usize, u64), Option<Vec<crate::layers::Change>>> = HashMap::new();
        for key in keys {
            let Some(t) = self.cache.get_mut(key) else { continue };
            if t.revs == revs || t.revs.len() != revs.len() {
                continue;
            }
            let Some(area) = key.bounds() else { continue };
            for (i, layer) in layers.iter().enumerate() {
                if t.revs[i] == revs[i] {
                    continue;
                }
                let changes = asked.entry((i, t.revs[i])).or_insert_with(|| layer.changes_since(t.revs[i]));
                if changes.as_ref().is_some_and(|c| !c.iter().any(|c| c.touches(&area, key.zoom()))) {
                    t.revs[i] = revs[i];
                }
            }
        }
    }

    /// `ISOMAR_DEBUG_TIMING`: for each tile of `need` already drawn, which layers' revisions moved since,
    /// with `*` where the layer cannot say where it changed (or changed everywhere), so a tile drawn
    /// again for nothing can be traced to the layer that asked.
    fn log_redraws(&self, layers: &[Rc<dyn Layer>], revs: &[u64], need: &[TileKey]) {
        for key in need {
            let Some(t) = self.cache.get(key).filter(|t| t.revs.len() == revs.len()) else { continue };
            let why: Vec<String> = layers
                .iter()
                .enumerate()
                .filter(|(i, _)| t.revs[*i] != revs[*i])
                .map(|(i, l)| {
                    let anywhere = l.changes_since(t.revs[i]).is_none_or(|c| c.iter().any(|c| c.area.width() >= 360.0));
                    format!("{}{}", l.id(), if anywhere { "*" } else { "" })
                })
                .collect();
            isomar_core::log!("isomar: redrawn {}/{}/{} for {}", key.level, key.tx, key.ty, why.join(" "));
        }
    }

    /// Asks every layer to prepare for the region `wanted` covers.
    fn snapshot(&mut self, f: &Frame, wanted: &[(TileKey, bool)], revs: Vec<u64>) -> Arc<Snapshot> {
        let tz = wanted.first().map_or(f.vi.zoom(), |(k, _)| k.zoom());
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for (k, _) in wanted {
            (x0, y0, x1, y1) = (x0.min(k.tx), y0.min(k.ty), x1.max(k.tx), y1.max(k.ty));
        }
        let (w, h) = (((x1 - x0 + 1) * TILE) as f64, ((y1 - y0 + 1) * TILE) as f64);
        let center = geo::unproject(x0 as f64 * TILE_F + w / 2.0, y0 as f64 * TILE_F + h / 2.0, tz);
        let area = TileArea { view: ViewInfo { view: View { zoom: tz, ..f.vi.view }, ..f.vi }, vi: ViewInfo { view: View { center, zoom: tz }, w, h }, pal: f.pal };
        let started = Instant::now();
        let paints: Vec<(&'static str, Arc<dyn TilePaint>)> = f.layers.iter().filter_map(|l| l.prepare(&area).map(|p| (l.id(), p))).collect();
        if crate::debug::timing() {
            isomar_core::log!("isomar: prepare {:.1} ms for {} tiles", started.elapsed().as_secs_f64() * 1000.0, wanted.len());
        }
        self.next_snapshot += 1;
        Arc::new(Snapshot { id: self.next_snapshot, plane: self.plane, generation: self.generation, revs, paints, pal: f.pal, scale: f.look.scale })
    }

    /// Drops the tiles longest unused past the byte limit; the ones wanted now stay.
    fn evict(&mut self, keep: &HashMap<TileKey, usize>) {
        // Tiles with nothing on them take no bytes, but are not kept for ever either.
        let over = |t: &Tiles| t.bytes > t.limit || t.cache.len() > MAX_HELD;
        if !over(self) {
            return;
        }
        let mut old: Vec<(u64, TileKey)> = self.cache.iter().filter(|(k, _)| !keep.contains_key(k)).map(|(k, t)| (t.used, *k)).collect();
        old.sort();
        for (_, k) in old {
            if !over(self) {
                break;
            }
            if let Some(t) = self.cache.remove(&k) {
                self.bytes -= t.bytes;
            }
        }
    }

    /// Paints the tiles over the background already painted. `vi` is the view and `scale` the device
    /// pixels to a point. Tiles of the view's zoom go where they belong, a device pixel at a time; where
    /// one is missing, tiles of nearby zooms are scaled to fill in.
    pub fn composite(&self, cr: &cairo::Context, vi: &ViewInfo, scale: i32) {
        for p in self.placements(vi, scale) {
            let _ = cr.save();
            if let Some([x, y, w, h]) = p.clip {
                cr.rectangle(x, y, w, h);
                cr.clip();
            }
            if p.clip.is_none() && (p.size - TILE_F).abs() < 1e-9 {
                if cr.set_source_surface(p.surface, p.x, p.y).is_ok() {
                    cr.source().set_filter(cairo::Filter::Nearest);
                    cr.rectangle(p.x, p.y, TILE_F, TILE_F);
                    let _ = cr.fill();
                }
            } else {
                cr.translate(p.x, p.y);
                cr.scale(p.size / TILE_F, p.size / TILE_F);
                if cr.set_source_surface(p.surface, 0.0, 0.0).is_ok() {
                    cr.source().set_filter(cairo::Filter::Bilinear);
                    let _ = cr.paint();
                }
            }
            let _ = cr.restore();
        }
    }

    /// The tiles to show for `vi`, back to front, at device scale `scale`. Tiles of the view's zoom go where
    /// they belong, on whole device pixels; where one is missing, tiles of nearby zooms stand in, the
    /// nearest on top, and only as many zooms as it takes to cover it.
    pub fn placements(&self, vi: &ViewInfo, scale: i32) -> Vec<Placement<'_>> {
        let exact = level_of(vi.zoom());
        let level = self.shown.filter(|&l| shows(l, exact)).unwrap_or(exact);
        let target = self.target.filter(|&l| shows(l, exact)).unwrap_or(level);
        let tz = level as f64 / 100.0;
        let k = 2f64.powf(vi.zoom() - tz);
        let (cx, cy) = geo::project(&vi.view.center, tz);
        let s = scale as f64;
        // Where world pixel (0, 0) of the tiles' zoom falls, on a device pixel, so that tiles abut exactly.
        let (ox, oy) = (((vi.w / 2.0 - cx * k) * s).round() / s, ((vi.h / 2.0 - cy * k) * s).round() / s);
        // Where tile (`tx`, `ty`) of a zoom whose tiles are `f` times the view's world goes, and its size.
        let at = |tx: i32, ty: i32, f: f64| {
            let size = TILE_F * k / f;
            (ox + tx as f64 * size, oy + ty as f64 * size, size)
        };
        fn placed(key: TileKey, t: &Tile, (x, y, size): (f64, f64, f64), clip: Option<[f64; 4]>) -> Option<Placement<'_>> {
            Some(Placement { key, id: t.id, surface: t.surface.as_ref()?, x, y, size, clip })
        }
        let visible: Vec<TileKey> = Tiles::wanted_at(vi, level, false).into_iter().map(|(k, _)| k).collect();
        let mut out = Vec::with_capacity(visible.len());

        let missing: Vec<TileKey> = visible.iter().filter(|k| !self.cache.contains_key(k)).copied().collect();
        if !missing.is_empty() {
            let mut levels: Vec<i32> = self.cache.keys().map(|k| k.level).filter(|l| (-FALLBACK_COARSER..=FALLBACK_FINER).contains(&(l - level)) && *l != level).collect();
            levels.sort_unstable();
            levels.dedup();
            // The zoom being drawn while the zoom keeps changing first, then those whose names and marks
            // are the view's own, grown or shrunk (`step_of`), the nearest of them first. Only by nearness,
            // a pinch back and forth over tiles of two whole zooms swapped the names for others half or
            // twice their size, placed elsewhere, at every half zoom; and tiles of both showed through
            // each other's sea.
            levels.sort_by_key(|l| (*l != target, step_of(*l) != step_of(level), (l - level).abs()));
            for key in &missing {
                // Only the part of it in view is stood in for: tiles of another zoom are drawn for the view
                // alone, and judged over the whole of a tile that reached past the view's edge, they never
                // covered it, and every zoom held was laid under them there, the names of each showing
                // through the others' sea, one set or another as the view's tiles shifted from frame to frame.
                let (rx, ry, rs) = at(key.tx, key.ty, 1.0);
                let (x0, y0) = (rx.max(0.0), ry.max(0.0));
                let (x1, y1) = ((rx + rs).min(vi.w), (ry + rs).min(vi.h));
                if x1 <= x0 || y1 <= y0 {
                    continue;
                }
                let clip = Some([x0, y0, x1 - x0, y1 - y0]);
                let mut stack = Vec::new();
                for &lv in &levels {
                    // A tile of the zoom `lv` is `f` times as big as this one's world.
                    let f = 2f64.powf((lv - level) as f64 / 100.0);
                    let size = TILE_F * k / f;
                    let (wx0, wx1) = ((x0 - ox) / size, (x1 - ox) / size);
                    let (wy0, wy1) = ((y0 - oy) / size, (y1 - oy) / size);
                    let mut whole = true;
                    for b in wy0.floor() as i32..=(wy1 - 1e-9).ceil() as i32 - 1 {
                        for a in wx0.floor() as i32..=(wx1 - 1e-9).ceil() as i32 - 1 {
                            let k = TileKey { level: lv, tx: a, ty: b };
                            match self.cache.get(&k) {
                                Some(t) => stack.extend(placed(k, t, at(a, b, f), clip)),
                                None => whole = false,
                            }
                        }
                    }
                    if whole {
                        break;
                    }
                }
                // The farthest zoom first, so the nearest is on top.
                out.extend(stack.into_iter().rev());
            }
        }
        out.extend(visible.iter().filter_map(|key| self.cache.get(key).and_then(|t| placed(*key, t, at(key.tx, key.ty, 1.0), None))));
        out
    }

    /// The tiles of `vi` that are in the cache, and how many it shows.
    pub fn coverage(&self, vi: &ViewInfo) -> (usize, usize) {
        if !self.in_use && self.cache.is_empty() && self.plane != 0 {
            return (0, 0);
        }
        let visible: Vec<TileKey> = Tiles::wanted(vi).into_iter().filter(|(_, v)| *v).map(|(k, _)| k).collect();
        (visible.iter().filter(|k| self.cache.contains_key(k)).count(), visible.len())
    }

}

/// The bytes of a row as words (a row of a chunk is a whole number of pixels: two to a word, and one
/// left over).
fn words(row: &[u8]) -> impl Iterator<Item = u64> + '_ {
    let (words, tail) = row.as_chunks::<8>();
    let last = (!tail.is_empty()).then(|| tail.iter().fold(0u64, |a, b| a << 8 | *b as u64));
    words.iter().map(|c| u64::from_ne_bytes(*c)).chain(last)
}

/// Edge of an overlay chunk, in device pixels.
pub const CHUNK: usize = 128;

/// The layers drawn every frame (`Layer::is_dynamic`), for a shell that composites on the GPU: drawn into a
/// buffer kept from frame to frame, and cut into chunks, of which only those with something on them are
/// handed over. A chunk's id stays the same while its pixels do, so a shell uploads only what changed:
/// the graticule's labels along two edges, a route, the boat, rather than the whole view every frame.
#[derive(Default)]
pub struct OverlayBuffer {
    surface: Option<cairo::ImageSurface>,
    /// Chunks across and down.
    cols: usize,
    rows: usize,
    /// For each chunk: its id (0 when empty) and a hash of its pixels.
    state: Vec<(u64, u64)>,
    next_id: u64,
}

/// A chunk of the overlay with something on it: where it is in device pixels, its size, and its id.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chunk {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
    pub id: u64,
}

impl OverlayBuffer {
    /// A clear surface of `w` by `h` device pixels to draw this frame's overlay on. Only the chunks that
    /// had something on them are cleared: the others are clear already.
    pub fn begin(&mut self, w: i32, h: i32) -> Option<cairo::ImageSurface> {
        let w = w.max(1);
        let h = h.max(1);
        if self.surface.as_ref().is_none_or(|s| s.width() != w || s.height() != h) {
            self.surface = Some(cairo::ImageSurface::create(cairo::Format::ARgb32, w, h).ok()?);
            self.cols = (w as usize).div_ceil(CHUNK);
            self.rows = (h as usize).div_ceil(CHUNK);
            self.state = vec![(0, 0); self.cols * self.rows];
        } else {
            let (cols, state) = (self.cols, self.state.clone());
            let surface = self.surface.as_mut()?;
            let (sw, sh, stride) = (w as usize, h as usize, surface.stride() as usize);
            let mut data = surface.data().ok()?;
            for (i, _) in state.iter().enumerate().filter(|(_, (id, _))| *id != 0) {
                let (x, y) = ((i % cols) * CHUNK, (i / cols) * CHUNK);
                for row in y..(y + CHUNK).min(sh) {
                    data[row * stride + x * 4..row * stride + (x + CHUNK).min(sw) * 4].fill(0);
                }
            }
        }
        self.surface.clone()
    }

    /// After drawing: notes which chunks have something on them and which changed, and returns them.
    /// Only the chunks within `touched` (rectangles in device pixels: x0, y0, x1, y1) are looked at: the
    /// others were clear before and nothing was drawn on them.
    pub fn finish(&mut self, touched: &[[f64; 4]]) -> Vec<Chunk> {
        let Some(surface) = &self.surface else { return Vec::new() };
        let (w, h, stride) = (surface.width() as usize, surface.height() as usize, surface.stride() as usize);
        let (cols, state, mut next) = (self.cols, &mut self.state, self.next_id);
        let mut out = Vec::new();
        let _ = surface.with_data(|data| {
            for (i, st) in state.iter_mut().enumerate() {
                let (x, y) = ((i % cols) * CHUNK, (i / cols) * CHUNK);
                let (cw, ch) = ((x + CHUNK).min(w) - x, (y + CHUNK).min(h) - y);
                // A pixel of slack round each rectangle, for antialiasing.
                let near = |r: &[f64; 4]| r[0] - 1.0 < (x + cw) as f64 && r[2] + 1.0 > x as f64 && r[1] - 1.0 < (y + ch) as f64 && r[3] + 1.0 > y as f64;
                if !touched.iter().any(near) {
                    *st = (0, 0);
                    continue;
                }
                let rows = || (y..y + ch).map(|r| &data[r * stride + x * 4..r * stride + (x + cw) * 4]);
                // Most chunks are empty: those are told by reading until something shows.
                if rows().all(|row| words(row).fold(0, |a, w| a | w) == 0) {
                    *st = (0, 0);
                    continue;
                }
                // Four lanes, so that each multiplication need not wait for the one before.
                let mut lanes = [0xcbf2_9ce4_8422_2325u64, 0x8422_2325_cbf2_9ce4, 0x9e37_79b9_7f4a_7c15, 0xc2b2_ae3d_27d4_eb4f];
                for row in rows() {
                    for (i, w) in words(row).enumerate() {
                        let l = &mut lanes[i & 3];
                        *l = (*l ^ w).wrapping_mul(0x0100_0000_01b3);
                    }
                }
                let hash = lanes.iter().fold(0u64, |h, l| h.rotate_left(17) ^ l);
                if st.0 == 0 || st.1 != hash {
                    next += 1;
                    *st = (next, hash);
                }
                out.push(Chunk { x, y, w: cw, h: ch, id: st.0 });
            }
        });
        self.next_id = next;
        out
    }

    /// The overlay's pixels (cairo's ARGB32: premultiplied, in native byte order) and the bytes to a row.
    pub fn with_pixels<R>(&self, f: impl FnOnce(&[u8], usize) -> R) -> Option<R> {
        let surface = self.surface.as_ref()?;
        let stride = surface.stride() as usize;
        let mut out = None;
        let _ = surface.with_data(|data| out = Some(f(data, stride)));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Engine;
    use crate::layers::bathy::{DepthGrid, DepthLayer, SoundingLayer};
    use crate::palette::Mode;
    use isomar_core::chart::grid::Grid;
    use isomar_core::config::ViewState;
    use std::sync::atomic::AtomicUsize;

    /// A static layer that only counts the tiles painted for it.
    struct Counting {
        painted: Arc<AtomicUsize>,
        revision: std::cell::Cell<u64>,
    }

    struct CountingPaint(Arc<AtomicUsize>);

    impl TilePaint for CountingPaint {
        fn paint(&self, _: &DrawCtx) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl Layer for Counting {
        fn id(&self) -> &'static str {
            "counting"
        }
        fn revision(&self) -> u64 {
            self.revision.get()
        }
        fn prepare(&self, _: &TileArea) -> Option<Arc<dyn TilePaint>> {
            Some(Arc::new(CountingPaint(self.painted.clone())))
        }
    }

    fn engine(lat: f64, lon: f64, zoom: f64) -> (Engine, Rc<Counting>, Arc<AtomicUsize>) {
        let e = Engine::new(ViewState { lat, lon, zoom }, Mode::Day);
        let painted = Arc::new(AtomicUsize::new(0));
        let layer = Rc::new(Counting { painted: painted.clone(), revision: std::cell::Cell::new(0) });
        e.add_layer(layer.clone());
        (e, layer, painted)
    }

    fn frame(e: &Engine, w: i32, h: i32, scale: i32) -> cairo::ImageSurface {
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, w * scale, h * scale).unwrap();
        surface.set_device_scale(scale as f64, scale as f64);
        let cr = cairo::Context::new(&surface).unwrap();
        e.draw(&cr, w as f64, h as f64, scale, false);
        drop(cr);
        surface
    }

    fn view(lat: f64, lon: f64, zoom: f64, w: f64, h: f64) -> ViewInfo {
        ViewInfo { view: View { center: crate::core::geo::LatLon::new(lat, lon), zoom }, w, h }
    }

    #[test]
    fn tiles_cover_the_view_at_any_fractional_zoom_and_size() {
        for (zoom, w, h) in [(12.0, 800.0, 600.0), (11.37, 1013.0, 517.0), (9.01, 255.0, 257.0), (15.99, 1920.0, 1080.0)] {
            let vi = view(47.6, -3.0, zoom, w, h);
            let wanted = Tiles::wanted(&vi);
            let level = (zoom * 100.0_f64).round() as i32;
            assert!(wanted.iter().all(|(k, _)| k.level == level));
            let (cx, cy) = geo::project(&vi.view.center, zoom);
            // Every point of the view is in a visible tile.
            for (fx, fy) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0), (0.5, 0.5), (0.31, 0.77)] {
                let (x, y) = (cx - w / 2.0 + fx * (w - 1e-3), cy - h / 2.0 + fy * (h - 1e-3));
                let (tx, ty) = ((x / TILE_F).floor() as i32, (y / TILE_F).floor() as i32);
                assert!(wanted.iter().any(|(k, v)| *v && k.tx == tx && k.ty == ty), "zoom {zoom}: ({x}, {y}) is in no tile");
            }
            // The visible ones come first, and no tile is wanted twice.
            let first_hidden = wanted.iter().position(|(_, v)| !*v).unwrap_or(wanted.len());
            assert!(wanted[first_hidden..].iter().all(|(_, v)| !*v));
            let keys: std::collections::HashSet<_> = wanted.iter().map(|(k, _)| k).collect();
            assert_eq!(keys.len(), wanted.len());
        }
    }

    #[test]
    fn a_pan_draws_only_the_tiles_it_exposes() {
        let (e, _layer, painted) = engine(47.6, -3.0, 12.0);
        e.resize(800.0, 600.0);
        frame(&e, 800, 600, 1);
        let first = painted.swap(0, Ordering::SeqCst);
        let before: std::collections::HashSet<TileKey> = Tiles::wanted(&e.view_info()).into_iter().map(|(k, _)| k).collect();
        assert_eq!(first, before.len(), "the first frame draws the view and the ring round it");

        // The same view again draws nothing.
        frame(&e, 800, 600, 1);
        assert_eq!(painted.load(Ordering::SeqCst), 0);

        // A pan of 100 px draws what is new to the wanted set, and only that.
        e.drag_begin(1, 400.0, 300.0);
        e.drag_update(-100.0, 0.0);
        frame(&e, 800, 600, 1);
        let after: std::collections::HashSet<TileKey> = Tiles::wanted(&e.view_info()).into_iter().map(|(k, _)| k).collect();
        let new = after.difference(&before).count();
        assert_eq!(painted.swap(0, Ordering::SeqCst), new);
        assert!(new < before.len() / 2, "{new} of {} tiles", before.len());

        // Panning back draws nothing: the tiles are kept.
        e.drag_update(0.0, 0.0);
        frame(&e, 800, 600, 1);
        assert_eq!(painted.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn new_data_draws_the_visible_tiles_again_and_the_old_ones_show_until_then() {
        let (e, layer, painted) = engine(47.6, -3.0, 12.0);
        e.resize(800.0, 600.0);
        frame(&e, 800, 600, 1);
        let drawn = painted.swap(0, Ordering::SeqCst);
        layer.revision.set(1);
        // Before the next frame plans them, the tiles are held, though not fresh.
        let vi = e.view_info();
        let (held, shown) = e.tiles_ready();
        assert_eq!(held, shown);
        frame(&e, 800, 600, 1);
        assert_eq!(painted.load(Ordering::SeqCst), drawn);
        let _ = vi;
    }

    #[test]
    fn a_stale_tile_stays_until_its_replacement_arrives() {
        let mut tiles = Tiles::new();
        let key = TileKey { level: 1000, tx: 3, ty: 4 };
        let snap = Arc::new(Snapshot { id: 1, plane: 0, generation: 0, revs: vec![0], paints: Vec::new(), pal: Palette::new(Mode::Day, false), scale: 1 });
        tiles.insert(paint_tile(key, snap.clone(), &AtomicBool::new(false)));
        assert!(tiles.is_fresh(&key, &[0]));
        assert!(!tiles.is_fresh(&key, &[1]) && tiles.has(&key), "stale, and still shown");
        let snap = Arc::new(Snapshot { id: 2, revs: vec![1], ..Arc::into_inner(snap).unwrap() });
        tiles.insert(paint_tile(key, snap, &AtomicBool::new(false)));
        assert!(tiles.is_fresh(&key, &[1]));
        assert_eq!(tiles.cached().0, 1);
    }

    #[test]
    fn a_change_of_look_clears_the_cache() {
        let (e, _layer, painted) = engine(47.6, -3.0, 12.0);
        e.resize(800.0, 600.0);
        frame(&e, 800, 600, 1);
        let drawn = painted.swap(0, Ordering::SeqCst);
        frame(&e, 800, 600, 1);
        assert_eq!(painted.load(Ordering::SeqCst), 0);
        // Another device scale: all drawn again, at twice the size.
        let surface = frame(&e, 800, 600, 2);
        assert_eq!(painted.swap(0, Ordering::SeqCst), drawn);
        assert_eq!(surface.width(), 1600);
        // Another palette.
        e.set_mode(Mode::Night);
        frame(&e, 800, 600, 2);
        assert_eq!(painted.swap(0, Ordering::SeqCst), drawn);
        // Hiding the layer draws no tile for it.
        e.set_layer_visible("counting", false);
        frame(&e, 800, 600, 2);
        assert_eq!(painted.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn the_cache_keeps_to_its_limit_and_never_drops_what_is_in_view() {
        let (e, _layer, painted) = engine(47.6, -3.0, 12.0);
        e.resize(800.0, 600.0);
        let per_tile = (TILE * TILE * 4) as usize;
        e.set_tile_cache_limit(per_tile * 10);
        frame(&e, 800, 600, 1);
        let wanted = Tiles::wanted(&e.view_info()).len();
        assert!(wanted > 10);
        let (held, bytes) = e.tiles_cached();
        assert_eq!(held, wanted, "what is wanted stays, though it is over the limit");
        assert_eq!(bytes, wanted * per_tile);
        // Far away, the first view's tiles go to make room.
        e.set_view(ViewState { lat: 10.0, lon: 100.0, zoom: 12.0 });
        frame(&e, 800, 600, 1);
        let (held, _) = e.tiles_cached();
        assert_eq!(held, Tiles::wanted(&e.view_info()).len());
        // Back again: they are drawn once more.
        painted.store(0, Ordering::SeqCst);
        e.set_view(ViewState { lat: 47.6, lon: -3.0, zoom: 12.0 });
        frame(&e, 800, 600, 1);
        assert_eq!(painted.load(Ordering::SeqCst), wanted);
    }

    /// A static layer that says where it changed, and counts the tiles painted for it with their zooms.
    struct Local {
        changes: crate::layers::Changes,
        zooms: Arc<Mutex<Vec<f64>>>,
    }

    struct LocalPaint(Arc<Mutex<Vec<f64>>>);

    impl TilePaint for LocalPaint {
        fn paint(&self, ctx: &DrawCtx) {
            self.0.lock().unwrap().push(ctx.vi.zoom());
        }
    }

    impl Layer for Local {
        fn id(&self) -> &'static str {
            "local"
        }
        fn revision(&self) -> u64 {
            self.changes.get()
        }
        fn changes_since(&self, revision: u64) -> Option<Vec<crate::layers::Change>> {
            self.changes.since(revision)
        }
        fn prepare(&self, _: &TileArea) -> Option<Arc<dyn TilePaint>> {
            Some(Arc::new(LocalPaint(self.zooms.clone())))
        }
    }

    fn local_engine() -> (Engine, Rc<Local>) {
        let e = Engine::new(ViewState { lat: 47.6, lon: -3.0, zoom: 12.0 }, Mode::Day);
        let layer = Rc::new(Local { changes: Default::default(), zooms: Arc::default() });
        e.add_layer(layer.clone());
        e.resize(800.0, 600.0);
        (e, layer)
    }

    #[test]
    fn a_change_elsewhere_keeps_the_tiles_and_one_in_view_draws_only_those_it_touches() {
        let (e, layer) = local_engine();
        frame(&e, 800, 600, 1);
        let all = std::mem::take(&mut *layer.zooms.lock().unwrap()).len();
        // Data arriving on the other side of the world.
        layer.changes.bump(Some(Bounds { min_lon: 100.0, min_lat: 10.0, max_lon: 101.0, max_lat: 11.0 }));
        frame(&e, 800, 600, 1);
        assert_eq!(layer.zooms.lock().unwrap().len(), 0);
        // A small area at the centre of the view.
        layer.changes.bump(Some(Bounds { min_lon: -3.001, min_lat: 47.599, max_lon: -2.999, max_lat: 47.601 }));
        frame(&e, 800, 600, 1);
        let some = layer.zooms.lock().unwrap().len();
        assert!(some > 0 && some <= 4, "{some} of {all}");
        // A change of another zoom's tiles leaves these.
        layer.changes.bump_change(Some(crate::layers::Change { area: e.view_info().bounds(), zooms: Some((13.0, 14.0)) }));
        frame(&e, 800, 600, 1);
        assert_eq!(layer.zooms.lock().unwrap().len(), some);
        // Anywhere: all of them.
        layer.changes.bump(None);
        frame(&e, 800, 600, 1);
        assert_eq!(layer.zooms.lock().unwrap().len(), some + all);
    }

    #[test]
    fn a_zoom_that_keeps_changing_is_drawn_at_whole_zooms_until_it_stops() {
        let (e, layer) = local_engine();
        frame(&e, 800, 600, 1);
        layer.zooms.lock().unwrap().clear();
        // One step from still: drawn at once, at its own zoom.
        e.zoom_to(12.37, 400.0, 300.0);
        frame(&e, 800, 600, 1);
        assert!(layer.zooms.lock().unwrap().iter().all(|z| (z - 12.37).abs() < 1e-9));
        layer.zooms.lock().unwrap().clear();
        // More steps straight after: nothing new at the zooms passed through, the whole zoom below them
        // is already held, and no ring round the view.
        for z in [12.41, 12.46, 12.52] {
            e.zoom_to(z, 400.0, 300.0);
            frame(&e, 800, 600, 1);
        }
        let zooms = layer.zooms.lock().unwrap().clone();
        assert!(zooms.iter().all(|z| (z - 12.0).abs() < 1e-9), "{zooms:?}");
        // Once still, its own zoom.
        std::thread::sleep(ZOOM_SETTLE + Duration::from_millis(10));
        frame(&e, 800, 600, 1);
        assert!(layer.zooms.lock().unwrap().iter().any(|z| (z - 12.52).abs() < 1e-9));
    }

    #[test]
    fn a_zoom_that_starts_again_after_a_pause_goes_on_with_the_tiles_the_pause_drew() {
        let (e, _layer) = local_engine();
        levels(&e);
        std::thread::sleep(ZOOM_SETTLE + Duration::from_millis(10));
        levels(&e);
        // A slow zoom in, drawn at 12 while it moves, then a pause long enough for 12.37 to be drawn.
        for z in [12.05, 12.10, 12.20, 12.30, 12.37] {
            e.zoom_to(z, 400.0, 300.0);
            levels(&e);
        }
        std::thread::sleep(ZOOM_SETTLE + Duration::from_millis(10));
        levels(&e);
        assert_eq!(levels(&e), [1237]);
        // It goes on from there: one level at a time, never those of 12 again, nor back to one it left.
        let mut shown: Vec<Vec<i32>> = Vec::new();
        for z in [12.40, 12.44, 12.50, 12.58, 12.66] {
            e.zoom_to(z, 400.0, 300.0);
            shown.push(levels(&e));
        }
        assert!(shown.iter().all(|l| l.len() == 1 && l[0] != 1200), "{shown:?}");
        shown.dedup();
        let mut seen = shown.clone();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), shown.len(), "{shown:?}");
    }

    #[test]
    fn another_level_is_shown_only_once_every_tile_of_it_in_view_is_drawn() {
        let layer: Rc<dyn Layer> = Rc::new(Local { changes: Default::default(), zooms: Arc::default() });
        let layers = [layer];
        let (tx, rx) = async_channel::unbounded();
        let mut tiles = Tiles::new();
        tiles.set_workers(2, Some(tx));
        let pal = Palette::new(Mode::Day, false);
        let look = Look { scale: 1, text_scale: 100, tone: pal.tone, hidden: Vec::new() };
        let update = |tiles: &mut Tiles, vi: ViewInfo| {
            tiles.update(&Frame { vi, pal, look: look.clone(), layers: &layers });
        };
        let shown = |tiles: &Tiles, vi: &ViewInfo| {
            let mut v: Vec<i32> = tiles.placements(vi, 1).iter().map(|p| p.key.level).collect();
            v.sort();
            v.dedup();
            v
        };
        // Every tile of 11 in, one at a time.
        let at11 = view(47.6, -3.0, 11.0, 800.0, 600.0);
        update(&mut tiles, at11);
        while tiles.pending() > 0 {
            tiles.receive(rx.recv_blocking().unwrap());
        }
        update(&mut tiles, at11);
        assert_eq!(shown(&tiles, &at11), [1100]);
        // A step to 11.40 from a still view: as its tiles come in, 11 stays, grown, and they show only
        // all together.
        std::thread::sleep(ZOOM_SETTLE + Duration::from_millis(10));
        let at1140 = view(47.6, -3.0, 11.40, 800.0, 600.0);
        update(&mut tiles, at1140);
        let mut seen = Vec::new();
        while tiles.pending() > 0 {
            tiles.receive(rx.recv_blocking().unwrap());
            update(&mut tiles, at1140);
            seen.push(shown(&tiles, &at1140));
        }
        // The tiles of the ring round the view come in after the switch.
        assert!(seen.len() > 2 && seen[0] == [1100], "{seen:?}");
        seen.dedup();
        assert_eq!(seen, [vec![1100], vec![1140]]);
    }

    #[test]
    fn a_resumed_zoom_leaves_the_tiles_of_its_pause_past_a_whole_zoom_or_half_a_zoom_out() {
        assert!(sticks(1237, 1300) && sticks(1237, 1314) && !sticks(1237, 1315));
        assert!(sticks(1237, 1200) && sticks(1237, 1187) && !sticks(1237, 1186));
        assert!(sticks(1280, 1230) && !sticks(1280, 1229));
        assert!(sticks(1200, 1185) && !sticks(1200, 1184));
    }

    #[test]
    fn changes_forgotten_or_anywhere_touch_everything() {
        let c = crate::layers::Changes::default();
        assert_eq!(c.since(0).map(|v| v.len()), Some(0));
        let b = Bounds { min_lon: 0.0, min_lat: 0.0, max_lon: 1.0, max_lat: 1.0 };
        c.bump(Some(b));
        assert_eq!(c.since(0).map(|v| v.len()), Some(1));
        assert_eq!(c.since(1).map(|v| v.len()), Some(0));
        c.bump(None);
        assert!(c.since(0).is_none() && c.since(1).is_none());
        for _ in 0..300 {
            c.bump(Some(b));
        }
        assert!(c.since(2).is_none(), "older than what is kept");
        assert!(c.since(c.get() - 1).is_some());
    }

    #[test]
    fn the_overlay_hands_over_only_chunks_with_something_on_them_and_keeps_their_ids() {
        let mut o = OverlayBuffer::default();
        let draw = |o: &mut OverlayBuffer, x: f64| {
            let s = o.begin(300, 300).unwrap();
            {
                let cr = cairo::Context::new(&s).unwrap();
                cr.set_source_rgb(1.0, 0.0, 0.0);
                cr.rectangle(x, 10.0, 4.0, 4.0);
                let _ = cr.fill();
            }
            drop(s);
            o.finish(&[[0.0, 0.0, 300.0, 300.0]])
        };
        let a = draw(&mut o, 10.0);
        assert_eq!(a.len(), 1);
        assert_eq!((a[0].x, a[0].y, a[0].w, a[0].h), (0, 0, CHUNK, CHUNK));
        assert_eq!(draw(&mut o, 10.0), a, "the same pixels keep their id");
        let b = draw(&mut o, 200.0);
        assert_eq!(b.len(), 1);
        assert_eq!((b[0].x, b[0].w), (CHUNK, CHUNK), "the old chunk was cleared");
        assert_ne!(b[0].id, a[0].id);
        // A chunk at the edge is cut to the buffer.
        let c = draw(&mut o, 290.0);
        assert_eq!((c[0].x, c[0].w), (2 * CHUNK, 300 - 2 * CHUNK));
    }

    /// A tile of one solid colour.
    fn solid(tiles: &mut Tiles, key: TileKey, rgb: (f64, f64, f64)) {
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, TILE, TILE).unwrap();
        {
            let cr = cairo::Context::new(&surface).unwrap();
            cr.set_source_rgb(rgb.0, rgb.1, rgb.2);
            cr.paint().unwrap();
        }
        let bytes = surface.stride() as usize * surface.height() as usize;
        tiles.cache.insert(key, Tile { surface: Some(surface), id: 0, revs: Vec::new(), bytes, used: 0 });
        tiles.bytes += bytes;
    }

    fn pixel(surface: &mut cairo::ImageSurface, x: usize, y: usize) -> [u8; 3] {
        let stride = surface.stride() as usize;
        let data = surface.data().unwrap();
        let o = y * stride + x * 4;
        [data[o + 2], data[o + 1], data[o]]
    }

    #[test]
    fn missing_tiles_are_stood_in_for_by_the_nearest_zoom() {
        let vi = view(47.6, -3.0, 10.30, 256.0, 256.0);
        let mut tiles = Tiles::new();
        // Tiles of two other zooms cover the view: the nearer one is on top.
        for (level, rgb) in [(1000, (1.0, 0.0, 0.0)), (1050, (0.0, 0.0, 1.0))] {
            let tz = level as f64 / 100.0;
            let (cx, cy) = geo::project(&vi.view.center, tz);
            for ty in (cy / TILE_F).floor() as i32 - 1..=(cy / TILE_F).floor() as i32 + 1 {
                for tx in (cx / TILE_F).floor() as i32 - 1..=(cx / TILE_F).floor() as i32 + 1 {
                    solid(&mut tiles, TileKey { level, tx, ty }, rgb);
                }
            }
        }
        let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 256, 256).unwrap();
        {
            let cr = cairo::Context::new(&surface).unwrap();
            tiles.composite(&cr, &vi, 1);
        }
        // 10.50 is 0.20 from 10.30, and 10.00 is 0.30 from it.
        assert_eq!(pixel(&mut surface, 128, 128), [0, 0, 255]);

        // With the nearer zoom gone, the other one fills in.
        tiles.cache.retain(|k, _| k.level != 1050);
        let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 256, 256).unwrap();
        {
            let cr = cairo::Context::new(&surface).unwrap();
            tiles.composite(&cr, &vi, 1);
        }
        assert_eq!(pixel(&mut surface, 128, 128), [255, 0, 0]);
    }

    #[test]
    fn stand_ins_keep_the_names_of_the_views_whole_zoom() {
        // 12.00 is nearer 11.60 than 11.00 is, but its names are placed for zoom 12: half the size, elsewhere.
        let vi = view(47.6, -3.0, 11.60, 256.0, 256.0);
        let mut tiles = Tiles::new();
        for (level, rgb) in [(1100, (1.0, 0.0, 0.0)), (1200, (0.0, 0.0, 1.0))] {
            let tz = level as f64 / 100.0;
            let (cx, cy) = geo::project(&vi.view.center, tz);
            for ty in (cy / TILE_F).floor() as i32 - 1..=(cy / TILE_F).floor() as i32 + 1 {
                for tx in (cx / TILE_F).floor() as i32 - 1..=(cx / TILE_F).floor() as i32 + 1 {
                    solid(&mut tiles, TileKey { level, tx, ty }, rgb);
                }
            }
        }
        let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 256, 256).unwrap();
        {
            let cr = cairo::Context::new(&surface).unwrap();
            tiles.composite(&cr, &vi, 1);
        }
        assert_eq!(pixel(&mut surface, 128, 128), [255, 0, 0]);
    }

    #[test]
    fn a_zoom_wavering_over_a_whole_zoom_stays_drawn_at_one() {
        let (e, layer) = local_engine();
        e.zoom_to(11.80, 400.0, 300.0);
        frame(&e, 800, 600, 1);
        layer.zooms.lock().unwrap().clear();
        // Fingers wobbling either side of 12: 11.80 throughout, which a zoom in has nothing to draw of.
        for z in [11.85, 11.95, 12.05, 11.97, 12.10, 11.90, 12.12] {
            e.zoom_to(z, 400.0, 300.0);
            assert_eq!(levels(&e), [1180]);
        }
        let zooms = std::mem::take(&mut *layer.zooms.lock().unwrap());
        assert!(zooms.iter().all(|z| (z - 11.80).abs() < 1e-9), "{zooms:?}");
        // Well past it, 12; and back a little under it, still 12.
        for z in [12.20, 12.30, 11.92] {
            e.zoom_to(z, 400.0, 300.0);
            assert_eq!(levels(&e), [1200]);
        }
        let zooms = std::mem::take(&mut *layer.zooms.lock().unwrap());
        assert!(!zooms.is_empty() && zooms.iter().all(|z| (z - 12.0).abs() < 1e-9), "{zooms:?}");
    }

    /// The levels of the tiles a frame of `e` at 800 x 600 shows.
    fn levels(e: &Engine) -> Vec<i32> {
        e.frame(800.0, 600.0, 1, false, |s| {
            let mut v: Vec<i32> = s.tiles.iter().map(|p| p.key.level).collect();
            v.sort();
            v.dedup();
            v
        })
    }

    #[test]
    fn a_pinch_across_a_whole_zoom_changes_the_tiles_shown_once() {
        // A phone's view: the tiles of the exact zoom reach past its edges.
        let (w, h) = (400.0, 700.0);
        let (e, _layer) = local_engine();
        e.set_view(ViewState { lat: 39.3, lon: 2.2, zoom: 7.3 });
        e.frame(w, h, 1, false, |_| ());
        std::thread::sleep(ZOOM_SETTLE + Duration::from_millis(10));
        e.frame(w, h, 1, false, |_| ());
        e.magnify_begin();
        let mut shown: Vec<Vec<i32>> = Vec::new();
        for i in 1..150 {
            e.magnify(2f64.powf(-0.004 * i as f64), w / 2.0, h / 2.0);
            let levels = e.frame(w, h, 1, false, |s| {
                let mut v: Vec<i32> = s.tiles.iter().map(|p| p.key.level).collect();
                v.sort();
                v.dedup();
                v
            });
            if shown.last() != Some(&levels) {
                shown.push(levels);
            }
        }
        // The first step of a still view is drawn at its own zoom, and the pinch goes on with it, shrunk,
        // until it is half a zoom out; then 6 alone. Each is shown once, and 7 never: it would only be a
        // third set of names between the two.
        assert_eq!(shown, [vec![730], vec![729], vec![600]]);
    }

    /// A grid of depths with some shape to it, round Brest.
    fn depth_layer() -> Rc<DepthLayer> {
        let layer = Rc::new(DepthLayer::new(Rc::new(|| {})));
        let (n, dx) = (400usize, 0.0005);
        let mut data = Vec::with_capacity(n * n);
        for j in 0..n {
            for i in 0..n {
                let (x, y) = (i as f32 / n as f32, j as f32 / n as f32);
                data.push(-(1.0 + 60.0 * (x * 7.0).sin().abs() * y + 25.0 * (y * 9.0).cos().abs()));
            }
        }
        layer.add_grid(DepthGrid::new(Grid { ncols: n, nrows: n, xll: -4.6, yll: 48.3, dx, dy: dx, data }));
        layer
    }

    /// The layers drawn into one picture of the whole view, as the old cached image was, and as tiles.
    #[test]
    fn tiles_seam_to_the_whole_picture() {
        let (w, h) = (700, 500);
        for zoom in [11.0, 11.15, 11.6, 12.43, 13.0] {
            // The centre on a whole pixel: tiles sit on device pixels, so the whole picture must too.
            let (cx, cy) = geo::project(&crate::core::geo::LatLon::new(48.4, -4.5), zoom);
            let c = geo::unproject(cx.round(), cy.round(), zoom);
            let state = ViewState { lat: c.lat, lon: c.lon, zoom };
            let depth = depth_layer();
            let soundings: Rc<dyn Layer> = Rc::new(SoundingLayer::new(depth.clone()));
            let layers: Vec<Rc<dyn Layer>> = vec![depth, soundings];

            let e = Engine::new(state, Mode::Day);
            for l in &layers {
                e.add_layer(l.clone());
            }
            e.resize(w as f64, h as f64);
            let mut tiled = frame(&e, w, h, 1);

            // The whole view, drawn at once.
            let vi = ViewInfo { view: View::from_state(state), w: w as f64, h: h as f64 };
            let pal = Palette::new(Mode::Day, false);
            let area = TileArea { view: vi, vi, pal };
            let mut whole = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h).unwrap();
            {
                let cr = cairo::Context::new(&whole).unwrap();
                cr.set_font_options(&tile_font_options().unwrap());
                crate::engine::paint_background(&cr, &vi, &pal);
                let overlay = Overlay::default();
                let ctx = DrawCtx { cr: &cr, vi, pal, overlay: &overlay, reduce_motion: false, top_inset: 0.0 };
                for l in &layers {
                    if let Some(p) = l.prepare(&area) {
                        p.paint(&ctx);
                    }
                }
            }
            let stride = whole.stride() as usize;
            let (a, b) = (tiled.data().unwrap().to_vec(), whole.data().unwrap().to_vec());
            let mut differing = 0usize;
            let mut worst = 0u8;
            for y in 0..h as usize {
                for x in 0..w as usize * 4 {
                    let d = a[y * stride + x].abs_diff(b[y * stride + x]);
                    worst = worst.max(d);
                    if d > 12 {
                        differing += 1;
                    }
                }
            }
            let total = (w * h * 4) as usize;
            assert!(differing * 500 < total, "zoom {zoom}: {differing} of {total} samples differ by more than 12 (worst {worst})");
        }
    }
}
