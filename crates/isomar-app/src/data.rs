//! Loads chart data in the background and feeds it to the layers.

use super::engine::Engine;
use super::layers::aids::{AidSectors, AidsLayer};
use super::layers::bathy::{DepthGrid, DepthLayer, SoundingLayer};
use super::layers::bsb::{BsbLayer, ChartInfo};
use super::layers::credit::{Credit, CreditLayer};
use super::layers::enc::EncLayer;
use super::layers::graticule::{Edge, GraticuleLabels, GraticuleLines};
use super::layers::land::LandLayer;
use super::layers::relief::ReliefLayer;
use super::layers::towns::TownLayer;
use crate::platform::{bg, spawn};
use crate::core::chart::aids::AidsData;
use crate::core::chart::coast::CoastStore;
use crate::core::chart::enc_index::EncIndex;
use crate::core::chart::grid::Grid;
use crate::core::datadir;
use crate::i18n::{gettext, gettext_f};
use std::path::{Path, PathBuf};
use rayon::prelude::*;
use std::rc::Rc;
use std::sync::Arc;

/// Receives short status messages for display.
pub type Status = Rc<dyn Fn(&str)>;

pub struct ChartLayers {
    pub land: Rc<LandLayer>,
    pub relief: Rc<ReliefLayer>,
    pub depth: Rc<DepthLayer>,
    pub aids: Rc<AidsLayer>,
    pub enc: Rc<EncLayer>,
    pub bsb: Rc<BsbLayer>,
    pub towns: Rc<TownLayer>,
    pub credit: Rc<Credit>,
}

const COAST_SHP: &str =
    "shom-limite-terre-mer/Limite_terre_mer_France_metropolitaine/SHAPE/Limite_terre-mer_France_metropolitaine_polygones.shp";
const AIDS_GPKG: &str = "shom-balisage/BALISAGE/GPKG/balisage.gpkg";

/// Creates the chart layers, adds them to the canvas, and starts loading
/// whatever data is installed.
pub fn install(canvas: &Engine, status: Status, chart_folders: &[String]) -> ChartLayers {
    let redraw = canvas.redraw_handle();
    let depth = Rc::new(DepthLayer::new(redraw.clone()));
    let credit = Rc::new(Credit::default());
    depth.set_credit(credit.clone());
    let land = Rc::new(LandLayer::new(redraw.clone(), credit.clone()));
    // Drawn by the land layer, over its fill.
    let relief = Rc::new(ReliefLayer::new(redraw.clone(), credit.clone()));
    land.set_relief(relief.clone());
    let aids = Rc::new(AidsLayer::new(redraw.clone(), credit.clone()));
    let towns = Rc::new(TownLayer::new(redraw.clone(), credit.clone()));
    towns.set_aids(aids.clone());
    towns.set_relief(relief.clone());
    let enc = EncLayer::new(redraw.clone());
    enc.set_relief(relief.clone());
    let bsb = Rc::new(BsbLayer::new(redraw));
    bsb.set_base(land.clone());
    enc.set_bsb(bsb.clone());

    canvas.add_layer(depth.clone());
    canvas.add_layer(Rc::new(SoundingLayer::new(depth.clone()).with_names(towns.clone())));
    canvas.add_layer(land.clone());
    // The sectors of the lights, which the names are written over.
    canvas.add_layer(Rc::new(AidSectors(aids.clone())));
    // Under the charts: where one covers the area, its own names and built-up areas show instead.
    canvas.add_layer(towns.clone());
    canvas.add_layer(enc.clone());
    canvas.add_layer(bsb.clone());
    canvas.add_layer(Rc::new(GraticuleLines));
    canvas.add_layer(Rc::new(GraticuleLabels(Edge::Top)));
    canvas.add_layer(Rc::new(GraticuleLabels(Edge::Left)));
    canvas.add_layer(aids.clone());
    canvas.add_layer(Rc::new(CreditLayer(credit.clone())));

    // Hidden chart cells leave their areas to the marks they would have replaced.
    let a = aids.clone();
    canvas.follow_layer_visible("enc", move |shown| a.set_enc_shown(shown));
    let e = enc.clone();
    canvas.follow_layer_visible("bsb", move |shown| e.set_bsb_shown(shown));
    load_coast(land.clone(), status.clone());
    load_aids(aids.clone(), status.clone());
    load_grids(depth.clone(), status.clone());
    let mut dirs = datadir::roots();
    dirs.extend(chart_folders.iter().map(PathBuf::from));
    {
        let (enc, aids, bsb) = (enc.clone(), aids.clone(), bsb.clone());
        spawn(async move {
            // Looking through the folders can take a while on a big or slow disk.
            let (cells, rasters) = bg(move || (datadir::enc_cells(&dirs), datadir::kap_files(&dirs))).await;
            load_enc(enc, aids, cells, status.clone());
            load_bsb(bsb, rasters, status);
        });
    }

    ChartLayers { land, relief, depth, aids, enc, bsb, towns, credit }
}

/// Cache file name for a source file: changes when the source does.
fn cache_name(prefix: &str, src: &Path) -> std::io::Result<PathBuf> {
    let meta = std::fs::metadata(src)?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Ok(datadir::cache_dir().join(format!("{prefix}-{}-{mtime}.sqlite", meta.len())))
}

/// The official French coastline, if installed. It is drawn instead of the
/// downloaded OpenStreetMap land wherever it has coast.
pub fn load_coast(land: Rc<LandLayer>, status: Status) {
    let Some(shp) = datadir::find(COAST_SHP) else { return };
    spawn(async move {
        let res = bg(move || -> Result<CoastStore, String> {
            let db = cache_name("coast", &shp).map_err(|e| e.to_string())?;
            // A store from an older version is rebuilt.
            if let Ok(store) = CoastStore::open(&db) {
                return Ok(store);
            }
            CoastStore::build_lambert93(&shp, &db, |_| {}).map_err(|e| e.to_string())?;
            CoastStore::open(&db).map_err(|e| e.to_string())
        });
        status(&gettext("Preparing the coastline (first run only)…"));
        match res.await {
            Ok(store) => {
                land.add_store(store, 40);
                status("");
            }
            Err(e) => status(&gettext_f("Coastline unavailable: {error}", &[("error", &e)])),
        }
    });
}

fn load_aids(layer: Rc<AidsLayer>, status: Status) {
    let Some(p) = datadir::find(AIDS_GPKG) else { return };
    spawn(async move {
        match bg(move || AidsData::load(&p).map_err(|e| e.to_string())).await {
            Ok(d) => layer.set_data(std::sync::Arc::new(d)),
            Err(e) => status(&gettext_f("Aids to navigation unavailable: {error}", &[("error", &e)])),
        }
    });
}

fn load_grids(depth: Rc<DepthLayer>, status: Status) {
    let mut files = datadir::find_all("shom-dtm", "asc");
    files.extend(datadir::find_all("emodnet", "tif"));
    for p in files {
        let (depth, status) = (depth.clone(), status.clone());
        spawn(async move {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let res = bg(move || -> Result<DepthGrid, String> {
                let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                let grid = if ext == "tif" { Grid::from_geotiff(&p) } else { Grid::from_asc_file(&p) };
                grid.map(DepthGrid::new).map_err(|e| e.to_string())
            })
            .await;
            match res {
                Ok(g) => depth.add_grid(g),
                Err(e) => status(&gettext_f("Could not read {file}: {error}", &[("file", &name), ("error", &e)])),
            }
        });
    }
}

/// The S-57 cells below `folders`, found off the main loop.
pub async fn find_enc(folders: Vec<PathBuf>) -> Vec<PathBuf> {
    bg(move || datadir::enc_cells(&folders)).await
}

/// The cells read at once while indexing, and in each batch handed to the layer.
const INDEX_BATCH: usize = 32;

/// Indexes S-57 cells in the background and adds them to the layer as they
/// are found. A folder can hold thousands of cells, more than fit in memory,
/// so only a summary of each is kept (and remembered between runs); the layer
/// reads a cell again when it is on screen.
pub fn load_enc(enc: Rc<EncLayer>, aids: Rc<AidsLayer>, files: Vec<PathBuf>, status: Status) {
    if files.is_empty() {
        return;
    }
    spawn(async move {
        // Leave cores for the interface.
        let threads = std::thread::available_parallelism().map(|n| n.get() - 1).unwrap_or(1).clamp(1, 4);
        let pool = match rayon::ThreadPoolBuilder::new().num_threads(threads).build() {
            Ok(p) => Arc::new(p),
            Err(e) => return status(&gettext_f("Could not read charts: {error}", &[("error", &e.to_string())])),
        };
        let index = Arc::new(EncIndex::open(datadir::cache_dir().join("enc-index.json")));
        enc.begin_indexing();
        let mut failed: Vec<(String, String)> = Vec::new();
        for (n, batch) in files.chunks(INDEX_BATCH).enumerate() {
            let (pool, idx, batch) = (pool.clone(), index.clone(), batch.to_vec());
            let done = bg(move || pool.install(|| batch.par_iter().map(|p| (p.clone(), idx.summary(p))).collect::<Vec<_>>())).await;
            let mut found = Vec::new();
            for (p, r) in done {
                match r {
                    Ok(s) => found.push((p, s)),
                    Err(e) => failed.push((p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), e.to_string())),
                }
            }
            enc.add_indexed(found);
            aids.set_exclusions(enc.exclusions());
            if n % 10 == 9 {
                let idx = index.clone();
                bg(move || idx.save()).await;
            }
        }
        enc.settle();
        bg(move || index.save()).await;
        match failed.as_slice() {
            [] => {}
            [(file, error)] => status(&gettext_f("Could not read chart {file}: {error}", &[("file", file), ("error", error)])),
            [(file, error), ..] => status(&gettext_f(
                "Could not read {n} charts, for example {file}: {error}",
                &[("n", &failed.len().to_string()), ("file", file), ("error", error)],
            )),
        }
    });
}

/// Reads the headers of raster charts in the background.
pub fn load_bsb(layer: Rc<BsbLayer>, files: Vec<PathBuf>, status: Status) {
    for p in files {
        let (layer, status) = (layer.clone(), status.clone());
        spawn(async move {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            match bg(move || ChartInfo::open(p)).await {
                Ok(c) => layer.add_chart(c),
                Err(e) => status(&gettext_f("Could not read chart {file}: {error}", &[("file", &name), ("error", &e)])),
            }
        });
    }
}
