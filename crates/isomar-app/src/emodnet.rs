//! Downloads EMODnet depth data for the area in view and keeps it on disk.
//!
//! A checkbox in the Layers menu, off by default. While ticked, the half
//! degree tiles under the view are fetched as the user pans and zooms in, saved
//! for good in the data folder and used offline from then on. They feed the
//! same depth shading, contours and soundings as a local grid, and give way to
//! a finer grid (SHOM's) wherever one has data. When the checkbox is off
//! nothing is downloaded and the tiles are hidden.

use super::app::AppState;
use super::data::Status;
use super::layers::bathy::{DepthGrid, DepthLayer};
use crate::platform::{bg, spawn};
use crate::core::chart::emodnet::{TileKey, load, tiles_for};
use crate::core::chart::vtile::TileError;
use crate::core::datadir;
use crate::i18n::{gettext, gettext_f};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

/// The Layers menu entry that switches the download on and off. It starts off.
pub const LAYER: &str = "online-depth";

/// Tiles fetched at once.
const WORKERS: usize = 3;
/// Most tiles one view can ask for.
const MAX_TILES: usize = 24;
/// Below this zoom a view spans too many tiles to be worth fetching.
const MIN_ZOOM: f64 = 9.0;
/// Tiles held in memory before those far from the view are let go.
const MAX_RESIDENT: usize = 80;
/// Wait for the view to settle before asking for tiles.
const SETTLE: Duration = Duration::from_millis(400);
/// A tile that failed is left alone for this long.
const RETRY_AFTER: Duration = Duration::from_secs(60);
/// A tile that failed for want of a connection is tried again this soon.
const RETRY_OFFLINE: Duration = Duration::from_secs(15);

/// Where the tiles are kept. Not the cache folder: this is data the user asked
/// to have saved, and it is not scanned for grids at start.
pub fn dir() -> PathBuf {
    datadir::user_data_dir().join("depth-tiles").join("emodnet")
}

struct Controller {
    app: Weak<AppState>,
    depth: Rc<DepthLayer>,
    status: Status,
    dir: PathBuf,
    settle: crate::platform::Settle,
    queue: RefCell<VecDeque<TileKey>>,
    in_flight: RefCell<HashSet<TileKey>>,
    workers: Cell<usize>,
    /// Tiles that failed, and when to try them again.
    failed: RefCell<HashMap<TileKey, Instant>>,
    /// Tiles the service has no data for.
    empty: RefCell<HashSet<TileKey>>,
    /// An error was shown since the downloads last started.
    complained: Cell<bool>,
}

pub fn install(app: &Rc<AppState>, depth: Rc<DepthLayer>, status: Status) {
    let ctl = Rc::new(Controller {
        app: Rc::downgrade(app),
        depth,
        status,
        dir: dir(),
        settle: Default::default(),
        queue: RefCell::new(VecDeque::new()),
        in_flight: RefCell::new(HashSet::new()),
        workers: Cell::new(0),
        failed: RefCell::new(HashMap::new()),
        empty: RefCell::new(HashSet::new()),
        complained: Cell::new(false),
    });
    {
        let ctl = ctl.clone();
        app.canvas.connect_view_changed(move |_| ctl.schedule());
    }
    {
        let ctl = ctl.clone();
        app.canvas.connect_layers_changed(move || ctl.schedule());
    }
    ctl.schedule();
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

    fn needs(&self, key: &TileKey) -> bool {
        !self.depth.has_tile(key) && !self.empty.borrow().contains(key)
    }

    /// Queues the tiles of the current view that the layer lacks, nearest the
    /// centre first, and starts the workers.
    fn run(self: &Rc<Self>) {
        let Some(app) = self.app.upgrade() else { return };
        let on = app.canvas.layer_visible(LAYER);
        self.depth.set_tiles_shown(on);
        let vi = app.canvas.view_info();
        if !on || vi.zoom() < MIN_ZOOM {
            self.queue.borrow_mut().clear();
            return;
        }
        let view = vi.bounds();
        if self.depth.tile_count() > MAX_RESIDENT {
            // Far-away tiles are read again from disk if the view returns.
            let by = view.width().max(view.height());
            self.depth.drop_tiles_outside(&view.expanded(by));
        }
        let keys: VecDeque<TileKey> = {
            let (failed, flying) = (self.failed.borrow(), self.in_flight.borrow());
            tiles_for(&view, MAX_TILES)
                .into_iter()
                .filter(|k| self.needs(k) && !flying.contains(k))
                .filter(|k| failed.get(k).is_none_or(|at| *at <= Instant::now()))
                .collect()
        };
        *self.queue.borrow_mut() = keys;
        while self.workers.get() < WORKERS && !self.queue.borrow().is_empty() {
            if self.workers.get() == 0 {
                self.complained.set(false);
                (self.status)(&gettext("Loading depth data…"));
            }
            self.workers.set(self.workers.get() + 1);
            spawn(self.clone().work());
        }
    }

    async fn work(self: Rc<Self>) {
        loop {
            let Some(app) = self.app.upgrade() else { break };
            let next = self.queue.borrow_mut().pop_front();
            let Some(key) = next else { break };
            // The view may have moved on, or the layer been switched off, since it was queued.
            if !app.canvas.layer_visible(LAYER) || !self.needs(&key) {
                continue;
            }
            self.in_flight.borrow_mut().insert(key);
            let (dir, download) = (self.dir.clone(), !crate::debug::offline());
            let res = bg(move || load(&dir, &key, download).map(|g| g.map(|g| DepthGrid::for_tile(g, key)))).await;
            self.in_flight.borrow_mut().remove(&key);
            match res {
                Ok(Some(grid)) => self.depth.add_grid(grid),
                Ok(None) => {
                    self.empty.borrow_mut().insert(key);
                }
                Err(e) => {
                    let wait = if e == TileError::NoConnection { RETRY_OFFLINE } else { RETRY_AFTER };
                    self.failed.borrow_mut().insert(key, Instant::now() + wait);
                    if e.worth_telling() && !self.complained.replace(true) {
                        (self.status)(&gettext_f("Could not load depth data: {error}", &[("error", &e.to_string())]));
                    }
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
