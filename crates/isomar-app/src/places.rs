//! Downloads the town names and outlines of the area in view (OpenFreeMap
//! tiles), when "Place names and town outlines" is ticked in the Layers menu.
//!
//! Like `tiles`, which does the same for the coastline and seamarks: the tiles
//! under the view are fetched as the user pans and zooms, kept on disk for 30
//! days and used offline afterwards. It is a separate service, so one being
//! down does not hold up the other.

use super::app::AppState;
use super::data::Status;
use super::layers::towns::TownLayer;
use super::tiles::{KEEP, MAX_TILES, RETRY_AFTER, RETRY_OFFLINE, SETTLE, WORKERS};
use crate::platform::{bg, spawn};
use crate::core::chart::places::{PlacesTile, fetch};
use crate::core::chart::vtile::{TileError, TileId, decode, forget, prune, read_cache, tiles_for};
use crate::core::datadir;
use crate::i18n::{gettext, gettext_f};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::time::Instant;

/// The coarsest tiles read for their island names: the tiles name no island before zoom 8.
const NAMES_FROM_ZOOM: u8 = 8;

/// The Layers menu entry (and layer) that draws the towns and switches the download.
pub const LAYER: &str = "towns";

pub(crate) fn cache_dir() -> PathBuf {
    datadir::cache_dir().join("places")
}

/// Reads a tile from disk, or downloads it when it is missing or old and
/// `download` allows. An old copy is used when the download fails.
fn load(dir: &Path, id: TileId, download: bool) -> Result<PlacesTile, TileError> {
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
    Ok(PlacesTile::from_tile(&tile))
}

struct Controller {
    app: Weak<AppState>,
    layer: Rc<TownLayer>,
    status: Status,
    dir: PathBuf,
    settle: crate::platform::Settle,
    /// Tiles to load, and whether only for their island names (a coarser tile than the view's).
    queue: RefCell<VecDeque<(TileId, bool)>>,
    in_flight: RefCell<HashSet<TileId>>,
    workers: Cell<usize>,
    /// Tiles that failed, and when to try them again.
    failed: RefCell<HashMap<TileId, Instant>>,
    /// An error was shown since the downloads last started.
    complained: Cell<bool>,
}

pub fn install(app: &Rc<AppState>, layer: Rc<TownLayer>, status: Status) {
    let dir = cache_dir();
    {
        let dir = dir.clone();
        spawn(async move {
            bg(move || prune(&dir, KEEP)).await;
        });
    }
    let ctl = Rc::new(Controller {
        app: Rc::downgrade(app),
        layer,
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
    {
        let weak = Rc::downgrade(&ctl);
        ctl.layer.connect_wanted(move || {
            if let Some(ctl) = weak.upgrade() {
                ctl.schedule();
            }
        });
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
        let on = app.canvas.layer_visible(LAYER);
        self.layer.set_online(on);
        if !on {
            self.queue.borrow_mut().clear();
            return;
        }
        let vi = app.canvas.view_info();
        let ids: VecDeque<(TileId, bool)> = {
            let (failed, flying) = (self.failed.borrow(), self.in_flight.borrow());
            let free = |t: &TileId| !flying.contains(t) && failed.get(t).is_none_or(|at| *at <= Instant::now());
            let z = TileId::zoom_for(vi.zoom());
            let own: Vec<TileId> = tiles_for(&vi.bounds(), z, MAX_TILES).into_iter().filter(|t| !self.layer.has_tile(t) && free(t)).collect();
            // A water's name is set along the whole of it, which may reach past the view.
            let shapes: Vec<TileId> = self.layer.shape_tiles().into_iter().filter(|t| !own.contains(t) && !self.layer.has_tile(t) && free(t)).collect();
            let (own, shapes) = (own.into_iter().map(|t| (t, false)), shapes.into_iter().map(|t| (t, false)));
            // The tiles leave an island out now and then at one zoom and name it at another: the coarser
            // ones, from where the islands start, are read for their island names, after the view's own.
            let coarser = (NAMES_FROM_ZOOM..z).rev().flat_map(|cz| tiles_for(&vi.bounds(), cz, MAX_TILES)).filter(|t| !self.layer.has_names(t) && !self.layer.has_tile(t) && free(t)).map(|t| (t, true));
            own.chain(shapes).chain(coarser).collect()
        };
        *self.queue.borrow_mut() = ids;
        while self.workers.get() < WORKERS && !self.queue.borrow().is_empty() {
            if self.workers.get() == 0 {
                self.complained.set(false);
                (self.status)(&gettext("Loading town names…"));
            }
            self.workers.set(self.workers.get() + 1);
            spawn(self.clone().work());
        }
    }

    async fn work(self: Rc<Self>) {
        loop {
            let Some(app) = self.app.upgrade() else { break };
            let next = self.queue.borrow_mut().pop_front();
            let Some((id, names_only)) = next else { break };
            // The view may have moved on, or the layer been switched off, since it was queued.
            if !app.canvas.layer_visible(LAYER) || self.layer.has_tile(&id) || (names_only && self.layer.has_names(&id)) {
                continue;
            }
            self.in_flight.borrow_mut().insert(id);
            let (dir, download) = (self.dir.clone(), !crate::debug::offline());
            let res = bg(move || load(&dir, id, download)).await;
            self.in_flight.borrow_mut().remove(&id);
            match res {
                Ok(tile) if names_only => self.layer.add_names(id, &tile),
                Ok(tile) => self.layer.add_tile(id, std::sync::Arc::new(tile)),
                Err(e) => {
                    let wait = if e == TileError::NoConnection { RETRY_OFFLINE } else { RETRY_AFTER };
                    self.failed.borrow_mut().insert(id, Instant::now() + wait);
                    if e.worth_telling() && !self.complained.replace(true) {
                        (self.status)(&gettext_f("Could not load town names: {error}", &[("error", &e.to_string())]));
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
            // Look again once the wait for a failed tile is over, so that tiles
            // lost to a dropped connection come once it is back.
            let soonest = self.failed.borrow().values().copied().filter(|at| *at > Instant::now()).min();
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
        while let Some(p) = parent.filter(|_| candidates.len() < 6) {
            // One the layer already has is what it draws.
            if self.layer.has_tile(&p) {
                return;
            }
            candidates.push(p);
            parent = p.parent();
        }
        let dir = self.dir.clone();
        let found = bg(move || candidates.into_iter().find_map(|p| Some((p, load(&dir, p, false).ok()?)))).await;
        if let Some((p, tile)) = found {
            self.layer.add_tile(p, std::sync::Arc::new(tile));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::chart::vtile::testutil::*;

    fn town_tile() -> Vec<u8> {
        layer("place", 4096, &[F { kind: 1, paths: vec![vec![[100, 100]]], tags: vec![("class", V::S("town")), ("name", V::S("Brest"))] }])
    }

    #[test]
    fn a_cached_tile_loads_without_a_network() {
        let d = tempfile::tempdir().unwrap();
        let id = TileId { z: 11, x: 998, y: 708 };
        crate::core::chart::vtile::write_cache(d.path(), &id, &town_tile()).unwrap();
        let t = load(d.path(), id, false).unwrap();
        assert_eq!(t.places.len(), 1);
        assert_eq!(t.places[0].name, "Brest");
    }

    #[test]
    fn a_missing_tile_offline_is_not_an_error_worth_reporting() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(load(d.path(), TileId { z: 5, x: 1, y: 1 }, false).unwrap_err(), TileError::Offline);
    }

    #[test]
    fn a_corrupt_tile_is_forgotten_so_the_next_try_downloads_it_again() {
        let d = tempfile::tempdir().unwrap();
        let id = TileId { z: 6, x: 2, y: 2 };
        crate::core::chart::vtile::write_cache(d.path(), &id, &[0xff, 0xff, 0xff]).unwrap();
        assert!(load(d.path(), id, false).is_err());
        assert!(read_cache(d.path(), &id).is_none());
    }
}
