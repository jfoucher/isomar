//! Downloads the elevation tiles of the area in view, for the relief (hillshade
//! and land contours) of the planning chart.
//!
//! One checkbox in the Layers menu, on by default. While ticked, the tiles
//! under the view are fetched as the user pans and zooms, kept on disk for a
//! year and used offline.

use super::app::AppState;
use super::data::Status;
use super::layers::relief::{MAX_TILES, MIN_ZOOM, ReliefLayer};
use super::tiles::{RETRY_AFTER, RETRY_OFFLINE, SETTLE, WORKERS};
use crate::core::chart::dem::{self, ElevTile, zoom_for};
use crate::core::chart::vtile::{TileError, TileId, tiles_for};
use crate::core::datadir;
use crate::i18n::{gettext, gettext_f};
use crate::platform::{bg, spawn};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The Layers menu entry that switches the download and the relief on and off.
pub const LAYER: &str = "online-relief";

/// Cached tiles older than this are deleted at start.
const KEEP: Duration = Duration::from_secs(2 * 365 * 24 * 3600);

pub(crate) fn cache_dir() -> PathBuf {
    datadir::cache_dir().join("dem")
}

/// Ticked in the menu, and not hidden with the land it is drawn on.
fn wanted(app: &AppState) -> bool {
    app.canvas.layer_visible(LAYER) && app.canvas.layer_visible("land")
}

struct Controller {
    app: Weak<AppState>,
    relief: Rc<ReliefLayer>,
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

pub fn install(app: &Rc<AppState>, relief: Rc<ReliefLayer>, status: Status) {
    let dir = cache_dir();
    {
        let dir = dir.clone();
        spawn(async move {
            bg(move || dem::prune(&dir, KEEP)).await;
        });
    }
    let ctl = Rc::new(Controller {
        app: Rc::downgrade(app),
        relief,
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

    /// Queues the tiles of the current view that the layer lacks, nearest the
    /// centre first, and starts the workers.
    fn run(self: &Rc<Self>) {
        let Some(app) = self.app.upgrade() else { return };
        let on = wanted(&app);
        self.relief.set_shown(on);
        let vi = app.canvas.view_info();
        if !on || vi.zoom() < MIN_ZOOM {
            self.queue.borrow_mut().clear();
            return;
        }
        let z = zoom_for(vi.zoom());
        // A ring of one tile round the view, for the seams.
        let ring = 360.0 / (1u64 << z) as f64;
        let ids: VecDeque<TileId> = {
            let (failed, flying) = (self.failed.borrow(), self.in_flight.borrow());
            tiles_for(&vi.bounds().expanded(ring), z, MAX_TILES)
                .into_iter()
                .filter(|t| !self.relief.has_tile(t) && !flying.contains(t))
                .filter(|t| failed.get(t).is_none_or(|at| *at <= Instant::now()))
                .collect()
        };
        *self.queue.borrow_mut() = ids;
        while self.workers.get() < WORKERS && !self.queue.borrow().is_empty() {
            if self.workers.get() == 0 {
                self.complained.set(false);
                (self.status)(&gettext("Loading terrain tiles…"));
            }
            self.workers.set(self.workers.get() + 1);
            spawn(self.clone().work());
        }
    }

    async fn work(self: Rc<Self>) {
        loop {
            let Some(app) = self.app.upgrade() else { break };
            let next = self.queue.borrow_mut().pop_front();
            let Some(id) = next else { break };
            // The view may have moved on, or the layer been switched off, since it was queued.
            if !wanted(&app) || self.relief.has_tile(&id) {
                continue;
            }
            self.in_flight.borrow_mut().insert(id);
            let (dir, download) = (self.dir.clone(), !crate::debug::offline());
            let res = bg(move || dem::load(&dir, &id, download)).await;
            self.in_flight.borrow_mut().remove(&id);
            match res {
                Ok(Some(tile)) => self.relief.add_tile(id, tile),
                // The service has no tile here: sea, or past its data. Not asked for again soon.
                Ok(None) => {
                    self.failed.borrow_mut().insert(id, Instant::now() + RETRY_AFTER * 60);
                }
                Err(e) => {
                    let wait = if e == TileError::NoConnection { RETRY_OFFLINE } else { RETRY_AFTER };
                    self.failed.borrow_mut().insert(id, Instant::now() + wait);
                    if e.worth_telling() && !self.complained.replace(true) {
                        (self.status)(&gettext_f("Could not load terrain tiles: {error}", &[("error", &e.to_string())]));
                    }
                    self.stand_in_for(id).await;
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
            // Look again once the wait for a failed tile is over.
            let soonest = self.failed.borrow().values().copied().filter(|at| *at > Instant::now() && *at < Instant::now() + RETRY_AFTER).min();
            if let Some(at) = soonest {
                let ctl = self.clone();
                crate::platform::timeout_once(at - Instant::now(), move || ctl.schedule());
            }
        }
    }

    /// Without the tile itself, loads the nearest coarser one from disk, which
    /// the layer then draws in its place.
    async fn stand_in_for(&self, id: TileId) {
        let mut candidates = Vec::new();
        let mut parent = id.parent();
        while let Some(p) = parent.filter(|p| candidates.len() < 6 && p.z >= dem::MIN_ZOOM) {
            // One the layer already has is what it draws.
            if self.relief.has_tile(&p) {
                return;
            }
            candidates.push(p);
            parent = p.parent();
        }
        let dir = self.dir.clone();
        let found: Option<(TileId, Arc<ElevTile>)> = bg(move || candidates.into_iter().find_map(|p| Some((p, dem::load(&dir, &p, false).ok()??)))).await;
        if let Some((p, tile)) = found {
            self.relief.add_tile(p, tile);
        }
    }
}
