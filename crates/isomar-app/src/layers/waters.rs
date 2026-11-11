//! Names of waters and islands from the OpenFreeMap tiles, set along their water as on a paper chart
//! where the tiles' sea says where it is. Unlike Natural Earth's (`regions`), these features are not
//! known ahead of time: a spine is worked out for each, on a worker, from the sea the tiles hold
//! round it (`compute`), and kept per name and tile zoom. A name whose spine is not ready, or that
//! does not fit along it, is written level as before (`towns`).

use super::TileArea;
use super::names::{self, Measured, PathLabel, Style};
use super::regions::{Rule, rule, set_along_as};
use super::{Changes, Redraw};
use crate::core::chart::naturalearth::GeoKind;
use crate::core::chart::places::{PlaceKind, PlacesTile};
use crate::core::chart::vtile::TileId;
use crate::core::labels::{Mask, Spine, spine};
use crate::core::vector::{Bounds, Pt};
use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

/// Cells along a water's box.
const ACROSS: usize = 160;
/// A name's length, in pixels, as a multiple of its letters at the size it is written.
const EM_PER_LETTER: f64 = 0.62;
/// Tracked capitals run longer than the plain text.
const TRACKING_ALLOWANCE: f64 = 1.6;
/// How much of the ground round a water's line is looked at, as a multiple of the name's length.
const REACH_NAMES: f64 = 2.0;
const REACH_ISLAND: f64 = 0.6;
/// And at least this many tiles either way, for a long island with a short name.
const ISLAND_MIN_TILES: f64 = 0.75;
/// An island of this rank or bigger whose land runs out of its box is too big for it, not a river's islet.
const BIG_ISLAND_RANK: u32 = 3;
/// Room kept between names that collide, in pixels, and how far past a tile a name is still wanted.
const GAP_PX: f64 = 4.0;
const REACH_PX: f64 = 64.0;
/// The zoom whose squares tell two features of one name apart, for those whose shape is worked out once
/// whatever the tiles' zoom: islands, and waters named at a point.
const ISLAND_KEY_ZOOM: u8 = 8;
/// A sea whose line leans more than this from level is named level at its roomiest water. A bay or a
/// lagoon follows its long axis however it lies.
const SEA_MAX_TILT_DEG: f64 = 45.0;
/// The directions tried for the long axis of a water named at a point, every this many degrees.
const AXIS_STEP_DEG: f64 = 5.0;
/// How far from its point a water named at a point is measured (units: about 5 km), on a grid this fine
/// (about 40 m a cell: a calanque is a few cells wide).
const POINT_REACH: f64 = 0.0625;
const POINT_ACROSS: usize = 480;
/// A water named at a point is named only once it is this wide on screen, in pixels: the tiles name every cove
/// (dozens round Marseille), and a cove's name out on the sea says nothing.
pub const MIN_POINT_WATER_PX: f64 = 60.0;
/// A water named along a line whose name cannot follow it is tried level at this many points less one along it,
/// after its `SPOTS` roomiest, each with at least `SPOT_LEAST` of the room of the roomiest.
const LEVEL_TRIES: usize = 8;
const SPOTS: usize = 6;
const SPOT_LEAST: f64 = 0.3;
/// A water named at a point is measured when at least one in this many chords through it meet the shore at both ends.
const ENCLOSED_SHARE: usize = 4;
/// A water named at a point is named along its long axis when its longest chord through the point is this
/// many times its width; else level.
const ELONGATED: f64 = 2.5;
/// Two directions whose chords are within this share of each other are as long: the more level is taken.
const AXIS_TIE: f64 = 0.95;
/// The line a water named at a point is set along is walked this many cells at a step, turning at most this many
/// degrees a step towards the roomiest water this many cells ahead, looked for this many `AXIS_STEP_DEG` either way.
const WALK_STEP_CELLS: f64 = 2.0;
const WALK_TURN_DEG: f64 = 5.0;
const WALK_LOOK_CELLS: f64 = 8.0;
const WALK_LOOK_TRIES: i32 = 4;
/// The name of a water named at a point is this many points a pixel of its width on screen (at the label zoom),
/// between these sizes: a small cove's name is small, so that it fits by the cove without crowding the chart.
const POINT_EM_PER_PX: f64 = 0.25;
const POINT_EM_LEAST: f64 = 8.0;
const POINT_EM_MOST: f64 = 12.5;

/// The size of the name of a water named at a point `width_px` wide on screen.
pub fn point_em(width_px: f64) -> f64 {
    (width_px * POINT_EM_PER_PX).clamp(POINT_EM_LEAST, POINT_EM_MOST)
}

/// How far the line of a water named at a point runs either way from it, in the water's widths.
const OUT_WIDTHS: f64 = 3.0;
/// Points of each step tested against the tiles' own coast.
const WALK_FINE_TESTS: usize = 8;
/// Spines kept; the finished ones are dropped when there are this many.
const SPINES_KEPT: usize = 2048;
/// The line of a bay is used as it is when it keeps this much of the room the centreline has.
const LINE_KEEPS: f64 = 0.6;
/// A centreline used instead of a bay's own line keeps within this share of the line's length of it:
/// the Golfe du Morbihan's water reaches the open sea, whose centreline is far out of the gulf.
const NEAR_LINE_SHARE: f64 = 0.25;
/// Islets smaller than this share of the grid are water: a water's name may be written over them.
const ISLET_SHARE: usize = 40;
/// And for a water named at a point, on its finer grid, only rocks: a few cells, about 100 m across (a share of
/// the grid filled islets such as Bannec, and the Passage du Fromveur met no shore).
const ROCK_SHARE: usize = POINT_ACROSS * POINT_ACROSS / 6;
/// The tiles fetched for a water's shape the view does not hold are of the zoom at which its box spans
/// this many, and no coarser than this zoom; at most this many are asked for at once.
const FETCH_ACROSS: f64 = 3.0;
const FETCH_MIN_ZOOM: u8 = 8;
const WANTED_KEPT: usize = 16;

/// What a name is of, for ranking and styling.
fn geo_kind(kind: PlaceKind) -> Option<GeoKind> {
    Some(match kind {
        PlaceKind::Sea => GeoKind::Sea,
        PlaceKind::Strait => GeoKind::Strait,
        PlaceKind::Bay | PlaceKind::Lagoon => GeoKind::Bay,
        PlaceKind::Island => GeoKind::Island,
        _ => return None,
    })
}

/// A name of a water or island in the shown tiles, with every piece of its geometry (units).
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub name: String,
    pub kind: PlaceKind,
    pub rank: u32,
    /// The pieces of its line in each tile, joined where they meet.
    pub lines: Vec<Vec<Pt>>,
    pub points: Vec<Pt>,
    /// The tile zoom of the pieces.
    pub z: u8,
}

/// `(name, kind, tile zoom, where on the world)`: a name can be of two waters far apart.
pub type Key = (String, u8, u8, i32, i32);

/// A place of a tile in units (Web Mercator at zoom 0, 256 across).
pub(super) fn unit_of(id: &TileId, extent: f64, p: [f64; 2]) -> Pt {
    let n = 2f64.powi(id.z as i32);
    [(id.x as f64 + p[0] / extent) / n * 256.0, (id.y as f64 + p[1] / extent) / n * 256.0]
}

pub(super) fn tile_box(id: &TileId) -> [f64; 4] {
    let (n, x, y) = (2f64.powi(id.z as i32), id.x as f64, id.y as f64);
    [x / n * 256.0, y / n * 256.0, (x + 1.0) / n * 256.0, (y + 1.0) / n * 256.0]
}

/// Whether `p` (units) is on water, the sea or a lake, in the finest of `tiles` that holds it; `None` where none does.
fn wet_at(tiles: &[(TileId, Arc<PlacesTile>)], p: Pt) -> Option<bool> {
    let (id, tile) = tiles
        .iter()
        .filter(|(id, _)| {
            let b = tile_box(id);
            p[0] >= b[0] && p[0] < b[2] && p[1] >= b[1] && p[1] < b[3]
        })
        .max_by_key(|(id, _)| id.z)?;
    let (e, n) = (tile.extent.max(1) as f64, 2f64.powi(id.z as i32));
    let (x, y) = ((p[0] / 256.0 * n - id.x as f64) * e, (p[1] / 256.0 * n - id.y as f64) * e);
    Some(tile.on_sea(x, y) || tile.on_lake(x, y))
}

/// A name set from a point is tried this many times off the land found along it, kept this far from each (pixels),
/// and its line tested this often (pixels).
const ASHORE_TRIES: usize = 6;
const ASHORE_KEEP_PX: f64 = 6.0;
const ASHORE_STEP_PX: f64 = 3.0;
/// The lines along a name tested for land, as shares of its line height from its middle.
const ASHORE_ACROSS: [f64; 3] = [0.0, -0.3, 0.3];

/// The first point on land, as `ashore` says, along the line through a name's letters and the spaces between its
/// words, from half a letter before the first to half one past the last, and along it a little above and below
/// (world pixels).
fn ashore_along(label: &PathLabel, ashore: &dyn Fn(Pt) -> bool) -> Option<Pt> {
    let g: Vec<(Pt, f64, f64)> = label.glyphs.iter().map(|(_, at, adv)| ([at.x, at.y], at.angle, *adv)).collect();
    let (first, last) = (g.first()?, g.last()?);
    let ends = [
        [first.0[0] - first.1.cos() * first.2 / 2.0, first.0[1] - first.1.sin() * first.2 / 2.0],
        [last.0[0] + last.1.cos() * last.2 / 2.0, last.0[1] + last.1.sin() * last.2 / 2.0],
    ];
    let pts: Vec<Pt> = std::iter::once(ends[0]).chain(g.iter().map(|x| x.0)).chain(std::iter::once(ends[1])).collect();
    pts.windows(2).find_map(|w| {
        let (dx, dy) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
        let len = dx.hypot(dy).max(1e-9);
        let n = (len / ASHORE_STEP_PX).ceil().max(1.0) as usize;
        let across = [-dy / len, dx / len];
        ASHORE_ACROSS.iter().find_map(|k| {
            let o = k * label.height;
            (0..=n).map(|i| i as f64 / n as f64).map(|f| [w[0][0] + dx * f + across[0] * o, w[0][1] + dy * f + across[1] * o]).find(|q| ashore(*q))
        })
    })
}

fn bounds_of(pts: impl IntoIterator<Item = Pt>) -> [f64; 4] {
    pts.into_iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])])
}

fn grown(b: [f64; 4], by: f64) -> [f64; 4] {
    [b[0] - by, b[1] - by, b[2] + by, b[3] + by]
}

fn meets(a: &[f64; 4], b: &[f64; 4]) -> bool {
    a[0] <= b[2] && a[2] >= b[0] && a[1] <= b[3] && a[3] >= b[1]
}

fn overlap(a: &[f64; 4], b: &[f64; 4]) -> bool {
    a[0] < b[2] && a[2] > b[0] && a[1] < b[3] && a[3] > b[1]
}

/// Joins the polylines whose ends meet (within `tol`) into longer ones.
pub fn stitch(mut chains: Vec<Vec<Pt>>, tol: f64) -> Vec<Vec<Pt>> {
    let close = |a: Pt, b: Pt| (a[0] - b[0]).hypot(a[1] - b[1]) <= tol;
    'again: loop {
        for i in 0..chains.len() {
            for j in i + 1..chains.len() {
                let (a, b) = (&chains[i], &chains[j]);
                let (a0, a1, b0, b1) = (a[0], *a.last().unwrap(), b[0], *b.last().unwrap());
                let joined: Option<Vec<Pt>> = if close(a1, b0) {
                    Some(a.iter().chain(b.iter().skip(1)).copied().collect())
                } else if close(a1, b1) {
                    Some(a.iter().chain(b.iter().rev().skip(1)).copied().collect())
                } else if close(a0, b1) {
                    Some(b.iter().chain(a.iter().skip(1)).copied().collect())
                } else if close(a0, b0) {
                    Some(b.iter().rev().chain(a.iter().skip(1)).copied().collect())
                } else {
                    None
                };
                if let Some(c) = joined {
                    chains[i] = c;
                    chains.remove(j);
                    continue 'again;
                }
            }
        }
        return chains;
    }
}

/// The names in `tiles` (the tiles shown: each as its address and data) that can be set along their
/// water, grouped by name and kind.
pub fn candidates(tiles: &[(TileId, Arc<PlacesTile>)]) -> Vec<Candidate> {
    let mut by: HashMap<(String, PlaceKind), (u32, Vec<Vec<Pt>>, Vec<Pt>, u8)> = HashMap::new();
    for (id, tile) in tiles {
        let e = tile.extent.max(1) as f64;
        for p in &tile.places {
            if geo_kind(p.kind).is_none() {
                continue;
            }
            let entry = by.entry((p.name.clone(), p.kind)).or_insert((u32::MAX, Vec::new(), Vec::new(), id.z));
            entry.0 = entry.0.min(p.rank);
            entry.3 = entry.3.max(id.z);
            match &p.line {
                Some(line) => entry.1.push(line.iter().map(|q| unit_of(id, e, [q[0] as f64, q[1] as f64])).collect()),
                None => entry.2.push(unit_of(id, e, [p.at[0] as f64, p.at[1] as f64])),
            }
        }
    }
    let mut out: Vec<Candidate> = by
        .into_iter()
        .map(|((name, kind), (rank, lines, points, z))| {
            // A tile unit at that zoom, with a little to spare.
            let tol = 256.0 / 2f64.powi(z as i32) / 4096.0 * 2.0;
            Candidate { name, kind, rank, lines: stitch(lines, tol), points, z }
        })
        .collect();
    out.sort_by(|a, b| (a.kind, a.rank, &a.name).cmp(&(b.kind, b.rank, &b.name)));
    out
}

impl Candidate {
    /// Its longest line, if it has one.
    pub fn line(&self) -> Option<&Vec<Pt>> {
        self.lines.iter().max_by(|a, b| line_length(a).total_cmp(&line_length(b)))
    }

    /// Where it is, in units: its line's middle, or its first point.
    pub fn anchor(&self) -> Option<Pt> {
        match self.line() {
            Some(l) => Some(l[l.len() / 2]),
            None => self.points.first().copied(),
        }
    }

    pub fn key(&self) -> Option<Key> {
        let at = self.anchor()?;
        // An island, or a water named at a point, has one shape at every zoom, so that its name sits the same
        // on it as the view zooms.
        let once = self.kind == PlaceKind::Island || self.line().is_none();
        let z = if once { ISLAND_KEY_ZOOM } else { self.z };
        // The world cut into squares of a few tiles, so that a name of two waters apart is two keys.
        let cell = 256.0 / 2f64.powi((z as i32 - 2).max(0));
        let z = if once { 0 } else { z };
        Some((self.name.clone(), self.kind as u8, z, (at[0] / cell).floor() as i32, (at[1] / cell).floor() as i32))
    }

    /// The length of its name in units, at about `size` pixels a letter row and the tile zoom.
    pub fn est_units(&self, size: f64) -> f64 {
        self.name.chars().count() as f64 * EM_PER_LETTER * size * TRACKING_ALLOWANCE / 2f64.powi(self.z as i32)
    }

    /// The box of ground its spine is worked out from.
    pub fn work_box(&self, size: f64) -> Option<[f64; 4]> {
        let est = self.est_units(size);
        match self.line() {
            // Every piece of the line: they do not always join (the Étang de Thau's are two).
            Some(_) => Some(grown(bounds_of(self.lines.iter().flatten().copied()), est * 0.5)),
            // An island's box need only hold the island: a wider one reaches past the loaded tiles (at a big text
            // size especially), and its land is then never measured.
            None if self.kind == PlaceKind::Island => {
                let tile = 256.0 / 2f64.powi(self.z as i32);
                self.points.first().map(|p| grown([p[0], p[1], p[0], p[1]], (est * REACH_ISLAND).max(ISLAND_MIN_TILES * tile)))
            }
            // A bay, cove or pass named at a point: the same ground whatever its name, enough to measure a bay
            // of a few kilometres.
            None if has_axis(self.kind) => self.points.first().map(|p| grown([p[0], p[1], p[0], p[1]], POINT_REACH * 1.25)),
            None => self.points.first().map(|p| grown([p[0], p[1], p[0], p[1]], est * REACH_NAMES)),
        }
    }
}

fn line_length(l: &[Pt]) -> f64 {
    l.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum()
}

/// What a worker is asked for: one spine.
pub struct Job {
    pub key: Key,
    pub cand: Candidate,
    pub size: f64,
    pub tiles: Vec<(TileId, Arc<PlacesTile>)>,
}

/// Where the sea is in the box `b`: the union of the sea of every tile that reaches it.
fn sea_mask(b: [f64; 4], tiles: &[(TileId, Arc<PlacesTile>)], across: usize) -> Mask {
    let mut m = Mask::over(b, across);
    let reach = [m.x0, m.y0, m.x0 + m.w as f64 * m.cell, m.y0 + m.h as f64 * m.cell];
    for (id, tile) in tiles {
        if tile.sea.is_empty() || !meets(&tile_box(id), &reach) {
            continue;
        }
        let e = tile.extent.max(1) as f64;
        let rings: Vec<Vec<Pt>> = tile.sea.iter().map(|r| r.iter().map(|q| unit_of(id, e, [q[0] as f64, q[1] as f64])).collect()).collect();
        m.fill(&rings, true);
    }
    m
}

/// `sea_mask` with the lakes (a lagoon is named along its own water), and the islets smaller than `1 / share`
/// of the grid counted as water.
fn water_mask(b: [f64; 4], tiles: &[(TileId, Arc<PlacesTile>)], share: usize, across: usize) -> Mask {
    let mut m = sea_mask(b, tiles, across);
    let reach = [m.x0, m.y0, m.x0 + m.w as f64 * m.cell, m.y0 + m.h as f64 * m.cell];
    for (id, tile) in tiles {
        if tile.lakes.is_empty() || !meets(&tile_box(id), &reach) {
            continue;
        }
        let e = tile.extent.max(1) as f64;
        let rings: Vec<Vec<Pt>> = tile.lakes.iter().map(|r| r.iter().map(|q| unit_of(id, e, [q[0] as f64, q[1] as f64])).collect()).collect();
        m.fill(&rings, true);
    }
    let cells = m.w * m.h / share;
    m.fill_small_holes(cells);
    m
}

/// The greatest distance from a point of `a` to the polyline `line`.
fn farthest_from(a: &[Pt], line: &[Pt]) -> f64 {
    let to_line = |p: Pt| {
        line.windows(2)
            .map(|w| {
                let (dx, dy) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
                let len2 = dx * dx + dy * dy;
                let t = if len2 > 0.0 { (((p[0] - w[0][0]) * dx + (p[1] - w[0][1]) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
                (p[0] - w[0][0] - t * dx).hypot(p[1] - w[0][1] - t * dy)
            })
            .fold(f64::MAX, f64::min)
    };
    a.iter().map(|p| to_line(*p)).fold(0.0, f64::max)
}

/// The tiles a water's box `b` is worked out from: those of the one zoom, `z` or coarser, that cover it,
/// the finest first; else the tiles to fetch for it, of a zoom at which the box spans a few.
fn covering(b: [f64; 4], z: u8, pool: &[(TileId, Arc<PlacesTile>)]) -> Result<Vec<(TileId, Arc<PlacesTile>)>, Vec<TileId>> {
    let longer = (b[2] - b[0]).max(b[3] - b[1]).max(1e-9);
    let coarse = ((FETCH_ACROSS * 256.0 / longer).log2().floor().max(0.0) as u8).clamp(FETCH_MIN_ZOOM.min(z), z);
    for z in (coarse..=z).rev() {
        // Tiles of one zoom: a coarser tile's sea, unioned with a finer one's, would cover a thin spit of land.
        let at: Vec<_> = pool.iter().filter(|(id, _)| id.z == z && meets(&tile_box(id), &b)).cloned().collect();
        if !at.is_empty() && covered(b, &at) {
            return Ok(at);
        }
    }
    Err(crate::core::chart::vtile::tiles_for(&degrees(b), coarse, WANTED_KEPT))
}

/// Whether the tiles cover the box `b`, sampled on a grid: a spine made where tiles are still to come
/// would take the unloaded ground for land.
pub fn covered(b: [f64; 4], tiles: &[(TileId, Arc<PlacesTile>)]) -> bool {
    let boxes: Vec<[f64; 4]> = tiles.iter().map(|(id, _)| tile_box(id)).collect();
    (0..=8).all(|i| {
        (0..=8).all(|j| {
            let p = [b[0] + (b[2] - b[0]) * i as f64 / 8.0, b[1] + (b[3] - b[1]) * j as f64 / 8.0];
            boxes.iter().any(|t| p[0] >= t[0] && p[0] <= t[2] && p[1] >= t[1] && p[1] <= t[3])
        })
    })
}

fn oriented(mut pts: Vec<Pt>, mut clear: Vec<f64>) -> Spine {
    if pts.last().is_some_and(|l| l[0] < pts[0][0]) {
        pts.reverse();
        clear.reverse();
    }
    Spine { pts, clear }
}

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len().max(1) as f64
}

/// The line `line` resampled every `step` units.
fn resample(line: &[Pt], step: f64) -> Vec<Pt> {
    let mut out = vec![line[0]];
    for w in line.windows(2) {
        let len = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
        let n = (len / step).ceil().max(1.0) as usize;
        for k in 1..=n {
            let t = k as f64 / n as f64;
            out.push([w[0][0] + (w[1][0] - w[0][0]) * t, w[0][1] + (w[1][1] - w[0][1]) * t]);
        }
    }
    out
}

/// A water named along a line: the line itself where it keeps to the middle of the water, else the water's own centreline.
fn along_line(line: &[Pt], size: f64, cand: &Candidate, tiles: &[(TileId, Arc<PlacesTile>)]) -> Option<(Spine, Vec<Pt>)> {
    let b = cand.work_box(size)?;
    let mask = water_mask(b, tiles, ISLET_SHARE, ACROSS);
    let d = mask.distance();
    let mid = line[line.len() / 2];
    let own = mask.component_at(mid);
    // Where the name goes level when it cannot follow the line: the roomiest water by the line, islets
    // counted as water (the Golfe du Morbihan's line is a hook along its southern shore), on the water the line
    // is on and near it: the roomiest in the line's box was the open sea past the Étang de Thau's spit.
    // The roomiest first, then the next roomiest away from those, a few in all: where the roomiest is crowded by
    // islets the name may fit elsewhere on the water.
    let len = line_length(line);
    let mut roomy: Vec<(usize, Pt)> = (0..mask.inside.len())
        .filter(|&c| d[c] > 0.0 && own.as_ref().is_none_or(|m| m.inside[c]))
        .map(|c| (c, mask.centre(c % mask.w, c / mask.w)))
        .filter(|(_, p)| farthest_from(&[*p], line) <= NEAR_LINE_SHARE * len)
        .collect();
    roomy.sort_by(|a, b| d[b.0].total_cmp(&d[a.0]));
    let mut spots: Vec<(Pt, f64)> = Vec::new();
    for (c, p) in roomy {
        let r = d[c] as f64 * mask.cell;
        if spots.len() >= SPOTS || r < spots.first().map_or(0.0, |s| s.1) * SPOT_LEAST {
            break;
        }
        if spots.iter().all(|(q, rq)| (p[0] - q[0]).hypot(p[1] - q[1]) > 2.0 * rq.max(r)) {
            spots.push((p, r));
        }
    }
    let spot: Vec<Pt> = spots.into_iter().map(|(p, _)| p).collect();
    let cell_clear = |p: Pt| mask.cell_at(p).map_or(0.0, |(i, j)| d[mask.index(i, j)] as f64 * mask.cell);
    let pts = resample(line, mask.cell * 2.0);
    let clear: Vec<f64> = pts.iter().map(|p| cell_clear(*p)).collect();
    // A line that is not on the sea (a lake, a sea the tiles do not hold) is no water to set a name along.
    if clear.iter().filter(|c| **c > 0.0).count() * 2 < clear.len() {
        return None;
    }
    // A lagoon the box holds whole is named along the whole of it, wherever on it its line is.
    if cand.kind == PlaceKind::Lagoon
        && let Some(s) = own.as_ref().filter(|m| !m.touches_edge()).and_then(spine)
    {
        return Some((s, spot));
    }
    let centreline = own.or_else(|| Some(mask.largest_component())).and_then(|m| spine(&m));
    let near = centreline.filter(|s| mean(&clear) < LINE_KEEPS * mean(&s.clear)).and_then(|s| s.cut_around(s.nearest(mid), len / 2.0)).filter(|s| farthest_from(&s.pts, line) <= NEAR_LINE_SHARE * len);
    Some((near.unwrap_or_else(|| oriented(pts, clear)), spot))
}

/// A water named at a point (a bay, a cove, a pass, a lagoon): how big it is, from its width (its shortest
/// chord through the point, if that meets the shore at both ends: off a coast nothing says how big the water
/// is) and its length; and the line its name is set along, with where the point is on it (a share of its
/// length): its long axis (its longest chord; for a water about as wide as it is long, the most level of its
/// long chords), moved a little across the water to where it is roomiest, and run on straight out over the
/// water as far as it goes, as a paper chart's name of a cove starts on the cove and runs out to sea: the
/// name is set from the point outwards as far as it needs.
fn along_axis(p: Pt, cand: &Candidate, tiles: &[(TileId, Arc<PlacesTile>)]) -> (Option<(Spine, f64)>, Option<(f64, f64)>) {
    let Some(b) = cand.work_box(0.0) else { return (None, None) };
    let reach = POINT_REACH;
    // A small water's name keeps off its islands; only rocks are water.
    let all = water_mask(b, tiles, ROCK_SHARE, POINT_ACROSS);
    // Only the water the point is on counts: a lagoon's name is not written on the sea over the spit. A point the
    // grid has on land is a cove too small to measure, and measured from the sea beside it, it came out big.
    let Some(mask) = all.component_at(p) else { return (None, None) };
    let d = mask.distance();
    let wet = |q: Pt| mask.cell_at(q).is_some_and(|(i, j)| mask.inside[mask.index(i, j)]);
    let room = |q: Pt| mask.cell_at(q).map_or(0.0, |(i, j)| d[mask.index(i, j)] as f64 * mask.cell);
    let step = mask.cell / 2.0;
    // How far the water goes from `q` along `dir`, up to `most`, and whether the shore ends it.
    let ray = |q: Pt, dir: Pt, most: f64| {
        let mut t = 0.0;
        while t + step <= most && wet([q[0] + dir[0] * (t + step), q[1] + dir[1] * (t + step)]) {
            t += step;
        }
        (t, t + step <= most)
    };
    let chord = |q: Pt, dir: Pt, most: f64| {
        let ((a, x), (b, y)) = (ray(q, [-dir[0], -dir[1]], most), ray(q, dir, most));
        (a, b, x && y)
    };
    let dir_of = |k: usize| {
        let a = (k as f64 * AXIS_STEP_DEG).to_radians();
        [a.cos(), a.sin()]
    };
    let n = (180.0 / AXIS_STEP_DEG) as usize;
    let chords: Vec<(f64, f64, bool)> = (0..n).map(|k| chord(p, dir_of(k), reach)).collect();
    let len = |k: usize| chords[k].0 + chords[k].1;
    // How wide it is: its shortest chord, which must meet the shore at both ends; and for a bay or a lagoon enough
    // chords must: off a coast the shore is only on one side, and nothing says how big the water is (Calanque
    // Seynerolles, named out on the sea, has 3 of 36; the Baie de la Ciotat 12). A pass is open at both ends.
    let bounded: Vec<usize> = (0..n).filter(|&k| chords[k].2).collect();
    let strait = cand.kind == PlaceKind::Strait;
    let narrowest = (0..n).min_by(|&a, &b| len(a).total_cmp(&len(b)));
    let Some(width) = narrowest.filter(|&k| chords[k].2 && (strait || bounded.len() * ENCLOSED_SHARE >= n)).map(len) else { return (None, None) };
    // How big it is: a pass by its width, a bay or a lagoon by its width and its length (its longest chord that
    // meets the shore): the Étang de Leucate is 2.7 km wide at its point and 13 km long.
    let size = if strait { width } else { (width * bounded.iter().map(|&k| len(k)).fold(width, f64::max)).sqrt() };
    // A water much longer than it is wide (an inlet, a lagoon, a pass) is named along its longest chord; a bay about
    // as wide as it is deep is named level (along its widest chord the Baie de la Ciotat ran horn to horn).
    let longest = (0..n).map(len).fold(0.0, f64::max);
    let elongated = longest >= ELONGATED * width;
    let tilt = |k: usize| {
        let deg = k as f64 * AXIS_STEP_DEG;
        if deg > 90.0 { 180.0 - deg } else { deg }
    };
    let Some(k) = (0..n).filter(|&k| !elongated || len(k) >= AXIS_TIE * longest).min_by(|&a, &b| tilt(a).total_cmp(&tilt(b))) else { return (None, Some((size, width))) };
    let dir = dir_of(k);
    let across = [-dir[1], dir[0]];
    // Where the line is roomiest is weighed over the water near the point, not far past its width (a pass runs out to
    // sea, and a bay in a bigger one, the Baie des Lecques in the Baie de la Ciotat, reaches across it).
    let out = width * ELONGATED / 2.0;
    // The parallel chord with the most room along it, a little across the water from the point: off the shore.
    let cap = width / 4.0;
    let (left, right) = (ray(p, [-across[0], -across[1]], cap).0, ray(p, across, cap).0);
    let offsets = (-(left / mask.cell) as i32..=(right / mask.cell) as i32).map(|i| i as f64 * mask.cell);
    let best = offsets
        .map(|o| [p[0] + across[0] * o, p[1] + across[1] * o])
        .filter(|q| wet(*q))
        .map(|q| (q, chord(q, dir, out)))
        .max_by(|(qa, ca), (qb, cb)| {
            let score = |q: &Pt, (a, b, _): &(f64, f64, bool)| {
                let samples = ((a + b) / mask.cell).ceil().max(1.0) as usize;
                (0..=samples).map(|i| -a + (a + b) * i as f64 / samples as f64).map(|t| room([q[0] + dir[0] * t, q[1] + dir[1] * t]).min(cap)).sum::<f64>()
            };
            score(qa, ca).total_cmp(&score(qb, cb))
        });
    let Some(q) = best.map(|(q, _)| q) else { return (None, Some((size, width))) };
    // The line runs from there either way along the axis, and on out over the open water past the mouth, where the
    // name of a cove too small for it is set from the cove outwards. Each step turns a little towards the roomiest
    // water just ahead, so that the line leaves a cove through the middle of its mouth rather than over a headland;
    // room past the water's width counts no more, so on open water it runs on straight. A bay's level line stays straight.
    let most_turn = if elongated { WALK_TURN_DEG } else { 0.0 };
    let (step, look) = (mask.cell * WALK_STEP_CELLS, mask.cell * WALK_LOOK_CELLS);
    let ahead = |q: Pt, d: Pt| (1..=WALK_LOOK_CELLS as usize).map(|i| look * i as f64 / WALK_LOOK_CELLS).map(|t| room([q[0] + d[0] * t, q[1] + d[1] * t])).fold(f64::MAX, f64::min).min(width);
    let turned = |d: Pt, deg: f64| {
        let (s, c) = deg.to_radians().sin_cos();
        [d[0] * c - d[1] * s, d[0] * s + d[1] * c]
    };
    // It runs out no farther than a few of the water's widths from the point: a cove's name is written by the
    // cove, not out across the sea, and so shows only once the cove is big enough on screen to hold most of it.
    let most_out = (OUT_WIDTHS * width).min(reach);
    let walk = |mut d: Pt| {
        let (mut at, mut pts, mut gone) = (q, Vec::new(), 0.0);
        while gone < most_out {
            let looks = (-WALK_LOOK_TRIES..=WALK_LOOK_TRIES).map(|i| i as f64 * AXIS_STEP_DEG).map(|a| (a, ahead(at, turned(d, a))));
            let Some((a, _)) = looks.max_by(|x, y| x.1.total_cmp(&y.1).then(y.0.abs().total_cmp(&x.0.abs()))) else { break };
            d = turned(d, a.clamp(-most_turn, most_turn));
            let next = [at[0] + d[0] * step, at[1] + d[1] * step];
            // Land the grid is too coarse for (a spit, a breakwater) stops it too.
            let ashore = (1..=WALK_FINE_TESTS).map(|i| i as f64 / WALK_FINE_TESTS as f64).any(|f| wet_at(tiles, [at[0] + d[0] * step * f, at[1] + d[1] * step * f]) == Some(false));
            if room(next) < mask.cell || ashore {
                break;
            }
            (at, gone) = (next, gone + step);
            pts.push(at);
        }
        pts
    };
    let mut pts = walk([-dir[0], -dir[1]]);
    pts.reverse();
    let at = pts.len();
    pts.push(q);
    pts.extend(walk(dir));
    if pts.len() < 3 {
        return (None, Some((size, width)));
    }
    // Where the point is along it (the line's first end may be swapped by `oriented`).
    let anchor = line_length(&pts[..=at]) / line_length(&pts).max(1e-18);
    let clear = pts.iter().map(|q| room(*q)).collect();
    let flipped = pts.last().is_some_and(|l| l[0] < pts[0][0]);
    (Some((oriented(pts, clear), if flipped { 1.0 - anchor } else { anchor })), Some((size, width)))
}

/// An island named at a point: the box of the land the point is on and its centreline, if the tiles
/// show all of it (an island in the sea); `None` for land that runs out of the box (a river's islet,
/// which the sea does not go round, or an island too big for the box).
fn along_land(p: Pt, size: f64, cand: &Candidate, tiles: &[(TileId, Arc<PlacesTile>)]) -> Option<([f64; 4], Option<Spine>)> {
    let first = cand.work_box(size)?;
    // A box too small for the island is grown, as far as the tiles go.
    for k in [1.0, 2.0, 4.0] {
        let half = [(first[2] - first[0]) * k / 2.0, (first[3] - first[1]) * k / 2.0];
        let c = [(first[0] + first[2]) / 2.0, (first[1] + first[3]) / 2.0];
        let b = [c[0] - half[0], c[1] - half[1], c[0] + half[0], c[1] + half[1]];
        if k > 1.0 && !covered(b, tiles) {
            return None;
        }
        let sea = sea_mask(b, tiles, ACROSS);
        let mut land = sea.empty_like();
        for c in land.inside.iter_mut() {
            *c = true;
        }
        land.minus(&sea);
        let land = land.component_at(p)?;
        if land.touches_edge() {
            continue;
        }
        let cells = (0..land.inside.len()).filter(|&c| land.inside[c]).map(|c| land.centre(c % land.w, c / land.w));
        let half = land.cell / 2.0;
        let bx = bounds_of(cells);
        return Some(([bx[0] - half, bx[1] - half, bx[2] + half, bx[3] + half], spine(&land)));
    }
    None
}

/// What the tiles say of a name's feature: its spine, and for an island the box of its land when the
/// sea goes all round it.
#[derive(Clone, Debug, Default)]
pub struct Shape {
    pub spine: Option<Arc<Spine>>,
    pub land: Option<[f64; 4]>,
    /// For a water named along a line, where its name goes level when it cannot follow the line, in turn (units).
    pub spot: Vec<Pt>,
    /// For a water named at a point, how wide it is (units), if that could be measured, and where the point is
    /// along its spine (a share of its length): its name is set as near it as it fits.
    pub size: Option<f64>,
    pub anchor: Option<f64>,
    /// And how wide it is at the point (units): its name is sized by it (`point_em`).
    pub width: Option<f64>,
    /// The zoom of the tiles it was worked out from.
    pub z: u8,
}

/// The spine of a candidate's water or land.
pub fn compute(job: &Job) -> Option<Spine> {
    shape(job).spine.map(Arc::unwrap_or_clone)
}

/// The spine of a candidate's water or land, and an island's box.
pub fn shape(job: &Job) -> Shape {
    if job.cand.kind == PlaceKind::Island {
        return match job.cand.points.first().and_then(|p| along_land(*p, job.size, &job.cand, &job.tiles)) {
            Some((land, spine)) => Shape { spine: spine.map(Arc::new), land: Some(land), z: job.cand.z, ..Shape::default() },
            None => Shape { z: job.cand.z, ..Shape::default() },
        };
    }
    // Its shape is worked out once (its key has no tile zoom), so it sits the same at every zoom.
    if job.cand.line().is_none() && has_axis(job.cand.kind) {
        let (spine, measure) = job.cand.points.first().map_or((None, None), |p| along_axis(*p, &job.cand, &job.tiles));
        let (spine, anchor) = spine.map_or((None, None), |(s, a)| (Some(Arc::new(s)), Some(a)));
        let (size, width) = measure.unzip();
        return Shape { spine, size, width, anchor, z: job.cand.z, ..Shape::default() };
    }
    let (spine, spot) = along_water_or_line(job).map_or((None, Vec::new()), |(s, spot)| (Some(Arc::new(s)), spot));
    Shape { spine, spot, z: job.cand.z, ..Shape::default() }
}

/// Whether a water named at a point is set along its long axis: a sea's point is out on open water.
pub(super) fn has_axis(kind: PlaceKind) -> bool {
    matches!(kind, PlaceKind::Strait | PlaceKind::Bay | PlaceKind::Lagoon)
}

fn along_water_or_line(job: &Job) -> Option<(Spine, Vec<Pt>)> {
    let (cand, size) = (&job.cand, job.size);
    match (cand.kind, cand.line()) {
        (PlaceKind::Sea | PlaceKind::Strait | PlaceKind::Bay | PlaceKind::Lagoon, Some(line)) => along_line(line, size, cand, &job.tiles),
        _ => None,
    }
}

#[derive(Clone)]
enum SpineState {
    Pending,
    Ready(Arc<Shape>),
}

/// The spines of the waters on show, and how each name is set along its own at a zoom.
pub struct WaterNames {
    spines: Rc<RefCell<HashMap<Key, SpineState>>>,
    measured: RefCell<HashMap<(Key, u64), Rc<Measured>>>,
    /// How each name sits at the last level: `(zoom, text size, view size)`.
    fits: RefCell<Option<((i64, u32, i32, i32), HashMap<Key, Option<Arc<PathLabel>>>)>>,
    /// The tiles the shapes of the names last asked for need, which the view does not hold.
    /// The tiles the shapes need, each with the box (degrees) of the waters that want it.
    wanted: RefCell<Vec<(TileId, Bounds)>>,
}

impl Default for WaterNames {
    fn default() -> Self {
        Self::new()
    }
}

/// An island the tiles told about: its name, where it is named, its land's box (units) or `None` where the
/// sea does not go round it, and its land's centreline.
pub type Island = (String, Pt, Option<[f64; 4]>, Option<Arc<Spine>>);

/// The names written along their water, and the names that therefore are not written level.
pub struct Written {
    pub labels: Vec<Arc<PathLabel>>,
    pub names: HashSet<String>,
    /// The islands the tiles have told about: name, where it is named and its land's box (units), `None`
    /// for land the sea does not go round.
    pub islands: Vec<Island>,
    /// The waters named along a line whose name, written level, goes elsewhere than the line's middle:
    /// name, the line's middle and the spots to try in turn (units).
    pub spots: Vec<(String, Pt, Vec<Pt>)>,
    /// The waters named at a point whose width was measured: name, where it is named and its width (units). One
    /// not here is not named: it is not measured yet, or is on open water.
    pub sizes: Vec<(String, Pt, f64, Option<f64>)>,
    /// The waters and islands whose shape is being worked out: not written level meanwhile, or the name
    /// shows level and then turns to follow its water once the shape lands.
    pub unsettled: HashSet<String>,
}

impl WaterNames {
    /// Forgets how the names sat: what they keep clear of has changed.
    pub fn forget(&self) {
        *self.fits.borrow_mut() = None;
    }

    pub fn new() -> WaterNames {
        WaterNames { spines: Rc::new(RefCell::new(HashMap::new())), measured: RefCell::new(HashMap::new()), fits: RefCell::new(None), wanted: RefCell::new(Vec::new()) }
    }

    /// The tiles the shapes of the names on show need that are not loaded.
    pub fn wanted(&self) -> Vec<TileId> {
        self.wanted.borrow().iter().map(|(t, _)| *t).collect()
    }

    /// Where the waters that want tile `id` for their shape are, when any does: what is drawn again when it
    /// arrives, so their shapes are asked for.
    pub fn wanted_by(&self, id: &TileId) -> Option<Bounds> {
        self.wanted.borrow().iter().filter(|(t, _)| t == id).map(|(_, b)| *b).reduce(|mut a, b| {
            a.union(&b);
            a
        })
    }

    fn base_size(kind: PlaceKind) -> f64 {
        geo_kind(kind).map_or(12.0, |k| rule(k, 0.0).style.size) * super::text_scale() as f64 / 100.0
    }

    /// Starts the spines the candidates lack, off the main thread, and bumps `changes` over each as it lands.
    /// A water's is worked out from tiles of one zoom that cover its box, from `pool` (every tile loaded):
    /// where none do, the tiles it needs are noted (`wanted`) and it is tried again once they are loaded.
    fn ask(&self, cands: &[Candidate], tiles: &[(TileId, Arc<PlacesTile>)], pool: &[(TileId, Arc<PlacesTile>)], changes: &Rc<Changes>, redraw: &Redraw) {
        let mut jobs = Vec::new();
        let mut wanted = Vec::new();
        {
            let mut spines = self.spines.borrow_mut();
            if spines.len() >= SPINES_KEPT {
                spines.retain(|_, s| matches!(s, SpineState::Pending));
            }
            // Only islands, waters named along a line and the narrow waters named at a point have a shape to work out.
            for cand in cands.iter().filter(|c| c.kind == PlaceKind::Island || c.line().is_some() || has_axis(c.kind)) {
                let Some(key) = cand.key() else { continue };
                // An island's shape is worked out again only where it was looked for in coarser tiles and not found.
                match spines.get(&key) {
                    Some(SpineState::Ready(shape)) if shape.spine.is_none() && cand.kind == PlaceKind::Island && cand.z > shape.z + 1 => {}
                    Some(_) => continue,
                    None => {}
                }
                let size = Self::base_size(cand.kind);
                // Only where the tiles round it are all there; a later pass, with more of them, tries again.
                let Some(b) = cand.work_box(size) else { continue };
                let tiles = if cand.kind == PlaceKind::Island {
                    // From the view's tiles, else from the loaded ones of the same zoom round it, else from the ones
                    // to fetch: an islet by the edge of a small view (zoomed in) was never measured, and its name
                    // was then left out.
                    if covered(b, tiles) {
                        tiles.to_vec()
                    } else {
                        let round = grown(b, (b[2] - b[0]).max(b[3] - b[1]));
                        let at: Vec<_> = pool.iter().filter(|(id, _)| id.z == cand.z && meets(&tile_box(id), &round)).cloned().collect();
                        if !covered(b, &at) {
                            wanted.extend(crate::core::chart::vtile::tiles_for(&degrees(b), cand.z, WANTED_KEPT).into_iter().map(|t| (t, degrees(b))));
                            continue;
                        }
                        at
                    }
                } else {
                    match covering(b, cand.z, pool) {
                        Ok(at) => at,
                        Err(fetch) => {
                            wanted.extend(fetch.into_iter().map(|t| (t, degrees(b))));
                            continue;
                        }
                    }
                };
                // An island's box may grow fourfold to hold it.
                let reach = if cand.kind == PlaceKind::Island { 2.0 } else { 0.5 };
                let near: Vec<_> = tiles.iter().filter(|(id, _)| meets(&tile_box(id), &grown(b, reach * (b[2] - b[0]).max(b[3] - b[1])))).cloned().collect();
                spines.insert(key.clone(), SpineState::Pending);
                jobs.push(Job { key, cand: cand.clone(), size, tiles: near });
            }
        }
        // The most important names' first.
        // A tile wanted by several waters is fetched once, and the first's box stands for it; the others'
        // are added, so all their tiles are drawn again when it arrives.
        let mut merged: Vec<(TileId, Bounds)> = Vec::new();
        for (t, b) in wanted {
            match merged.iter_mut().find(|(m, _)| *m == t) {
                Some((_, mb)) => mb.union(&b),
                None => merged.push((t, b)),
            }
        }
        let mut wanted = merged;
        wanted.truncate(WANTED_KEPT);
        *self.wanted.borrow_mut() = wanted;
        if jobs.is_empty() {
            return;
        }
        let done = |spines: &RefCell<HashMap<Key, SpineState>>, changes: &Changes, results: Vec<(Key, [f64; 4], Shape)>| {
            let mut spines = spines.borrow_mut();
            for (key, b, shape) in results {
                spines.insert(key, SpineState::Ready(Arc::new(shape)));
                changes.bump(Some(degrees(b)));
            }
        };
        let run = |jobs: Vec<Job>| -> Vec<(Key, [f64; 4], Shape)> {
            jobs.into_iter()
                .map(|j| {
                    let b = j.cand.work_box(j.size).unwrap_or([0.0; 4]);
                    (j.key.clone(), b, shape(&j))
                })
                .collect()
        };
        if crate::platform::installed() {
            let (spines, changes, redraw) = (self.spines.clone(), changes.clone(), redraw.clone());
            crate::platform::spawn(async move {
                let results = crate::platform::bg(move || run(jobs)).await;
                done(&spines, &changes, results);
                redraw();
            });
        } else {
            let results = run(jobs);
            done(&self.spines, changes, results);
        }
    }

    fn measure(&self, key: &Key, text: &str, st: &Style) -> Rc<Measured> {
        let k = (key.clone(), (st.size * super::text_scale() as f64).to_bits());
        if let Some(m) = self.measured.borrow().get(&k) {
            return m.clone();
        }
        let m = Rc::new(names::measure(text, st));
        let mut all = self.measured.borrow_mut();
        if all.len() > 4096 {
            all.clear();
        }
        all.insert(k, m.clone());
        m
    }

    /// How `cand` is set along `spine` at `zoom`, if it fits.
    #[allow(clippy::too_many_arguments)]
    fn fit(&self, cand: &Candidate, key: &Key, shape: &Shape, spine: &Spine, zoom: f64, tiles: &[(TileId, Arc<PlacesTile>)], avoid: &dyn Fn(&[f64; 4]) -> bool) -> Option<PathLabel> {
        // A water named at a point is small (a cove, a pass): written as a bay is, as its level name would be.
        let point = cand.kind != PlaceKind::Island && cand.line().is_none();
        let kind = if point { GeoKind::Bay } else { geo_kind(cand.kind)? };
        let ppu = 2f64.powf(zoom);
        let b = bounds_of(spine.pts.iter().copied());
        let on = (b[2] - b[0]).max(b[3] - b[1]) * ppu;
        // Its size says when it is named (`MIN_POINT_WATER_PX`), and the view is never inside it.
        let (least, most) = if point { (0.0, f64::MAX) } else { (cand.kind.min_zoom(), rule(kind, on).max_zoom) };
        if zoom < least || zoom > most {
            return None;
        }
        if cand.kind == PlaceKind::Sea {
            let (a, z) = (spine.pts[0], *spine.pts.last()?);
            if (z[1] - a[1]).atan2(z[0] - a[0]).to_degrees().abs() > SEA_MAX_TILT_DEG {
                return None;
            }
        }
        // Its letters as close as in its level name, sized by the water itself (a small cove's smaller): its line runs
        // on out over the open water, longer than the name, which is set as near its point as it fits.
        let em = shape.width.map(|w| point_em(w * ppu));
        let tight = |r: Rule| match (point, em) {
            (true, Some(em)) => Rule { track: (0.0, 0.0), style: Style { size: em, ..r.style }, ..r },
            (true, None) => Rule { track: (0.0, 0.0), ..r },
            _ => r,
        };
        let anchor = if point { shape.anchor } else { None };
        let along = |avoid: &dyn Fn(&[f64; 4]) -> bool| set_along_as(kind, &tight, anchor, &cand.name, spine, ppu, hash(key), vec![cand.name.trim().to_lowercase()], b, &mut |text, style| self.measure(key, text, style), avoid);
        if !point {
            return along(avoid);
        }
        // Its letters are on the water as the tiles draw it: the grid its line was found on is too coarse for a spit,
        // which may also fall between two words. Land found along the name is kept clear of, and the name tried again.
        let ashore = |q: Pt| wet_at(tiles, [q[0] / ppu, q[1] / ppu]) == Some(false);
        let mut blocked: Vec<[f64; 4]> = Vec::new();
        for _ in 0..ASHORE_TRIES {
            let label = along(&|b: &[f64; 4]| avoid(b) || blocked.iter().any(|k| overlap(b, k)))?;
            match ashore_along(&label, &ashore) {
                Some(q) => blocked.push(grown([q[0], q[1], q[0], q[1]], ASHORE_KEEP_PX)),
                None => return Some(label),
            }
        }
        None
    }

    /// The names of `tiles` that are set along their water at this area's `names::label_zoom`, in its world
    /// pixels (as `ne` and `avoid` are): those whose spine is ready and which fit, which no Natural Earth name `ne` is in the way of, and which no name that
    /// wins a crowded spot over them is in the way of. Spines still to be worked out are started.
    /// `extra` are islands named in coarser tiles that the shown ones leave out.
    #[allow(clippy::too_many_arguments)]
    pub fn labels_for(&self, area: &TileArea, tiles: &[(TileId, Arc<PlacesTile>)], pool: &[(TileId, Arc<PlacesTile>)], extra: Vec<Candidate>, too_big: &HashSet<String>, ne: &[Arc<PathLabel>], avoid: &dyn Fn(&[f64; 4]) -> bool, changes: &Rc<Changes>, redraw: &Redraw) -> Written {
        let vi = &area.vi;
        let zoom = names::label_zoom(vi.zoom());
        let started = crate::debug::timing().then(std::time::Instant::now);
        let mut cands = candidates(tiles);
        let known: HashSet<(String, PlaceKind)> = cands.iter().map(|c| (c.name.clone(), c.kind)).collect();
        cands.extend(extra.into_iter().filter(|c| !known.contains(&(c.name.clone(), c.kind))));
        self.ask(&cands, tiles, pool, changes, redraw);
        let level = ((zoom * 100.0).round() as i64, super::text_scale(), area.view.w as i32, area.view.h as i32);
        let mut fits = self.fits.borrow_mut();
        if fits.as_ref().is_none_or(|(l, _)| *l != level) {
            *fits = Some((level, HashMap::new()));
        }
        let memo = &mut fits.as_mut().unwrap().1;
        let ppu = 2f64.powf(zoom);
        let wanted = grown(crate::core::chart::naturalearth::units_of(&vi.bounds()), (REACH_PX * 2.0) / vi.ppu());
        let mut fitted: Vec<(&Candidate, Arc<PathLabel>, i64)> = Vec::new();
        let mut islands = Vec::new();
        let mut spots = Vec::new();
        let mut sizes = Vec::new();
        let mut unsettled = HashSet::new();
        // A water named along a line has a shape per tile zoom: while the one of the tiles on show is worked
        // out, the one from the tiles of another zoom stands in, so a name that followed its water keeps doing so.
        let mut ready: HashMap<(String, u8), Vec<(Key, Arc<Shape>)>> = HashMap::new();
        for (k, st) in self.spines.borrow().iter() {
            if let SpineState::Ready(shape) = st {
                ready.entry((k.0.clone(), k.1)).or_default().push((k.clone(), shape.clone()));
            }
        }
        let stand_in = |cand: &Candidate, key: &Key| -> Option<Arc<Shape>> {
            let at = cand.anchor()?;
            let cell = |z: u8| 256.0 / 2f64.powi((z as i32 - 2).max(0));
            ready
                .get(&(key.0.clone(), key.1))?
                .iter()
                .filter(|(k, _)| k.2 != key.2 && k.2 != 0)
                .map(|(k, shape)| {
                    let c = cell(k.2);
                    let d = ((k.3 as f64 + 0.5) * c - at[0]).hypot((k.4 as f64 + 0.5) * c - at[1]);
                    (d / c.max(cell(key.2)), shape)
                })
                .filter(|(d, _)| *d < 1.5)
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, shape)| shape.clone())
        };
        for cand in &cands {
            let Some(key) = cand.key() else { continue };
            // A sea or a strait reaching past the view is one the view is inside. A bay or a lagoon is written on
            // itself, and may run out of the view: its kind's zoom limits say when it is too big to name.
            if matches!(cand.kind, PlaceKind::Sea | PlaceKind::Strait) && too_big.contains(&cand.name) {
                continue;
            }
            let state = self.spines.borrow().get(&key).cloned();
            let shape = match state {
                Some(SpineState::Ready(shape)) => shape,
                pending => match stand_in(cand, &key) {
                    Some(shape) if cand.line().is_some() => shape,
                    _ => {
                        if matches!(pending, Some(SpineState::Pending)) {
                            unsettled.insert(cand.name.clone());
                        }
                        continue;
                    }
                },
            };
            // A big island whose land ran out of its box is not known, rather than no island.
            if cand.kind == PlaceKind::Island
                && (shape.land.is_some() || cand.rank > BIG_ISLAND_RANK)
                && let Some(at) = cand.anchor()
            {
                islands.push((cand.name.clone(), at, shape.land, shape.spine.clone()));
            }
            // Where a water named along a line is tried level when its name cannot follow the line: its roomiest
            // water, then along its centreline, the roomiest first (the Golfe du Morbihan's roomiest water is
            // between islands its name does not fit over).
            if cand.kind != PlaceKind::Island
                && cand.line().is_some()
                && let Some(at) = cand.anchor()
            {
                let mut tries: Vec<Pt> = shape.spot.clone();
                if let Some(spine) = &shape.spine {
                    let len = spine.length();
                    let mut along: Vec<(Pt, f64)> = (1..LEVEL_TRIES).map(|i| len * i as f64 / LEVEL_TRIES as f64).map(|u| (spine.at(u).0, spine.clear_at(u))).collect();
                    along.sort_by(|a, b| b.1.total_cmp(&a.1));
                    tries.extend(along.into_iter().map(|(p, _)| p));
                }
                if !tries.is_empty() {
                    spots.push((cand.name.clone(), at, tries));
                }
            }
            // A water named at a point is named once it is big enough on screen, the bigger first.
            let point = cand.kind != PlaceKind::Island && cand.line().is_none();
            if point && let (Some(w), Some(at)) = (shape.size, cand.anchor()) {
                sizes.push((cand.name.clone(), at, w, shape.width));
            }
            if point && shape.size.is_none_or(|w| w * ppu < MIN_POINT_WATER_PX) {
                continue;
            }
            let Some(spine) = shape.spine.clone() else { continue };
            if !meets(&bounds_of(spine.pts.iter().copied()), &wanted) {
                continue;
            }
            let label = memo.entry(key.clone()).or_insert_with(|| self.fit(cand, &key, &shape, &spine, zoom, tiles, avoid).map(Arc::new)).clone();
            if let Some(l) = label {
                let on = match shape.size {
                    Some(w) if point => w * ppu,
                    _ => (l.units[2] - l.units[0]).max(l.units[3] - l.units[1]) * ppu,
                };
                fitted.push((cand, l, on as i64));
            }
        }
        let priority = |c: &Candidate, on: i64| (geo_kind(c.kind), c.rank, Reverse(on), c.name.clone());
        let hits = |a: &PathLabel, b: &[[f64; 4]]| {
            a.boxes.iter().any(|x| {
                let x = grown(*x, GAP_PX);
                b.iter().any(|y| overlap(&x, y))
            })
        };
        let ne_boxes: Vec<[f64; 4]> = ne.iter().flat_map(|l| l.boxes.iter().copied()).collect();
        let mut out: Vec<Arc<PathLabel>> = Vec::new();
        let mut names = HashSet::new();
        let mut beaten = Vec::new();
        for (i, (c, l, on)) in fitted.iter().enumerate() {
            let mine = priority(c, *on);
            if hits(l, &ne_boxes) || fitted.iter().enumerate().any(|(j, (c2, l2, on2))| j != i && priority(c2, *on2) < mine && !hits(l2, &ne_boxes) && hits(l, &l2.boxes)) {
                beaten.push((c, mine));
            } else {
                out.push(l.clone());
                names.insert(c.name.clone());
            }
        }
        // A water named at a point that lost its spot slides along its line, from its point out over the water,
        // clear of the names that won, the bigger first.
        beaten.sort_by(|a, b| a.1.cmp(&b.1));
        for (c, _) in beaten {
            let point = c.kind != PlaceKind::Island && c.line().is_none();
            let Some(key) = c.key().filter(|_| point) else { continue };
            let Some(SpineState::Ready(shape)) = self.spines.borrow().get(&key).cloned() else { continue };
            let Some(spine) = shape.spine.clone() else { continue };
            let taken: Vec<[f64; 4]> = ne_boxes.iter().chain(out.iter().flat_map(|l| l.boxes.iter())).map(|b| grown(*b, GAP_PX)).collect();
            let clear = |b: &[f64; 4]| avoid(b) || taken.iter().any(|t| overlap(b, t));
            if let Some(l) = self.fit(c, &key, &shape, &spine, zoom, tiles, &clear) {
                out.push(Arc::new(l));
                names.insert(c.name.clone());
            }
        }
        if let Some(t) = started {
            isomar_core::log!("isomar: water names {} candidates, {} fitted, {} shown, {:.2} ms", cands.len(), fitted.len(), out.len(), t.elapsed().as_secs_f64() * 1000.0);
        }
        Written { labels: out, names, islands, spots, sizes, unsettled }
    }
}

fn hash(key: &Key) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut h);
    // Kept clear of the Natural Earth ids, which are small.
    h.finish() >> 9 | 1 << 54
}

/// The box `b` in units as degrees, a little wider, for telling the tiles what changed.
fn degrees(b: [f64; 4]) -> Bounds {
    let k = 256.0;
    let lon = |x: f64| x / k * 360.0 - 180.0;
    let lat = |y: f64| crate::core::geo::unproject(0.0, y / k * 256.0, 0.0).lat;
    Bounds { min_lon: lon(b[0]), min_lat: lat(b[3]), max_lon: lon(b[2]), max_lat: lat(b[1]) }.expanded(0.01)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::chart::places::{Place, PlacesTile};

    /// A tile of sea with an island, in tile units (extent 4096).
    fn tile(id: TileId, sea: Vec<Vec<[i16; 2]>>, places: Vec<Place>) -> (TileId, Arc<PlacesTile>) {
        let sea_boxes = sea
            .iter()
            .map(|r| {
                let (lo, hi) = (|k: usize| r.iter().map(|p| p[k]).min().unwrap_or(0), |k: usize| r.iter().map(|p| p[k]).max().unwrap_or(0));
                [lo(0), lo(1), hi(0), hi(1)]
            })
            .collect();
        (id, Arc::new(PlacesTile { extent: 4096, places, sea, sea_boxes, ..Default::default() }))
    }

    fn square(x: i16, y: i16, s: i16) -> Vec<[i16; 2]> {
        vec![[x, y], [x + s, y], [x + s, y + s], [x, y + s]]
    }

    fn water(name: &str, kind: PlaceKind, at: [i16; 2], line: Option<Vec<[i16; 2]>>) -> Place {
        Place { name: name.into(), kind, rank: 0, at, span: line.as_ref().map(|_| [0, 0, 0, 0]), line }
    }

    /// Sea over the whole tile with a land block cut out of it: a channel along the top and bottom.
    fn channel_tile(id: TileId) -> (TileId, Arc<PlacesTile>) {
        // The outer ring the whole tile, the hole a wide block of land across the middle.
        let hole = vec![[0, 1500], [0, 2600], [4096, 2600], [4096, 1500]];
        tile(id, vec![square(-64, -64, 4224), hole], vec![])
    }

    #[test]
    fn pieces_of_a_line_in_neighbouring_tiles_are_joined() {
        let chains = vec![vec![[0.0, 0.0], [1.0, 0.0]], vec![[2.0, 0.0], [1.0, 0.0001]], vec![[5.0, 5.0], [6.0, 5.0]]];
        let joined = stitch(chains, 0.01);
        assert_eq!(joined.len(), 2);
        let long = joined.iter().max_by_key(|c| c.len()).unwrap();
        assert_eq!(long.len(), 3);
        assert_eq!(long[0], [0.0, 0.0]);
        assert_eq!(*long.last().unwrap(), [2.0, 0.0]);
    }

    #[test]
    fn a_bay_named_along_a_line_in_two_tiles_is_one_candidate() {
        let (a, b) = (TileId { z: 10, x: 500, y: 400 }, TileId { z: 10, x: 501, y: 400 });
        let bay = |from: [i16; 2], to: [i16; 2]| water("Baie de Test", PlaceKind::Bay, from, Some(vec![from, to]));
        let tiles = vec![tile(a, vec![square(0, 0, 4096)], vec![bay([3000, 2000], [4096, 2000])]), tile(b, vec![square(0, 0, 4096)], vec![bay([0, 2000], [1000, 2000])])];
        let c = candidates(&tiles);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].lines.len(), 1, "joined into one line");
        assert!(c[0].key().is_some());
    }

    #[test]
    fn a_line_in_the_middle_of_a_channel_is_used_as_it_is() {
        let id = TileId { z: 10, x: 500, y: 400 };
        // A strait of sea along y 2700..4096 under a block of land.
        let hole = vec![[-64, -64], [4160, -64], [4160, 2400], [-64, 2400]];
        let sea = vec![square(-64, -64, 4224), hole];
        let line = vec![[200, 3300], [2000, 3250], [3900, 3300]];
        let tiles = vec![tile(id, sea, vec![water("Goulet", PlaceKind::Strait, [2000, 3250], Some(line))])];
        let c = candidates(&tiles).remove(0);
        let job = Job { key: c.key().unwrap(), size: 12.0, cand: c, tiles };
        let s = compute(&job).expect("a spine");
        let tile_y = |y: f64| (400.0 + y / 4096.0) / 1024.0 * 256.0;
        assert!(s.pts.iter().all(|p| (p[1] - tile_y(3300.0)).abs() < tile_y(600.0) - tile_y(0.0)), "{:?}", s.pts);
        assert!(s.pts[0][0] < s.pts.last().unwrap()[0]);
        assert!(s.clear.iter().all(|c| *c >= 0.0));
    }

    #[test]
    fn a_strait_named_at_a_point_on_open_water_is_not_set_along_anything() {
        // Sea all over: the centreline would run corner to corner of the box, nowhere near the pass.
        let id = TileId { z: 10, x: 500, y: 400 };
        let mut tiles = vec![tile(id, vec![square(-64, -64, 4224)], vec![water("Passage des Sœurs", PlaceKind::Strait, [2000, 2000], None)])];
        for (dx, dy) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
            tiles.push(tile(TileId { z: 10, x: (500 + dx) as u32, y: (400 + dy) as u32 }, vec![square(-64, -64, 4224)], vec![]));
        }
        let c = candidates(&tiles).into_iter().find(|c| c.kind == PlaceKind::Strait).unwrap();
        let job = Job { key: c.key().unwrap(), size: 12.0, cand: c, tiles };
        assert!(compute(&job).is_none());
    }

    /// A lake 3000 by 600 tile units, leaning 45 degrees down to the right, in a tile with no sea.
    #[test]
    fn a_coves_name_is_set_from_its_point_along_its_axis_and_a_little_way_out_over_the_sea() {
        // Sea all round the tile, with a coast across it and a short cove, 200 wide, running up into the land at x 2000.
        let id = TileId { z: 12, x: 2000, y: 1500 };
        let sea = vec![vec![[-64, 2000], [1900, 2000], [1900, 1600], [2100, 1600], [2100, 2000], [4160, 2000], [4160, 4160], [-64, 4160]]];
        let mut tiles = vec![tile(id, sea, vec![water("Cala Test", PlaceKind::Bay, [2000, 1700], None)])];
        for (dx, dy) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0)] {
            tiles.push(tile(TileId { z: 12, x: (2000 + dx) as u32, y: (1500 + dy) as u32 }, vec![], vec![]));
        }
        for dx in [-1, 0, 1] {
            tiles.push(tile(TileId { z: 12, x: (2000 + dx) as u32, y: 1501 }, vec![square(-64, -64, 4224)], vec![]));
        }
        let c = candidates(&tiles).into_iter().find(|c| c.kind == PlaceKind::Bay).unwrap();
        let shape = shape(&Job { key: c.key().unwrap(), size: 12.0, cand: c, tiles });
        let (s, anchor) = (shape.spine.expect("an axis"), shape.anchor.expect("its point on it"));
        assert!((direction(&s).abs() - 90.0).abs() < 10.0, "up the cove: {}", direction(&s));
        let y = |v: f64| unit_of(&id, 4096.0, [0.0, v])[1];
        let (top, bottom) = (s.pts.iter().map(|p| p[1]).fold(f64::MAX, f64::min), s.pts.iter().map(|p| p[1]).fold(f64::MIN, f64::max));
        assert!(top < y(1680.0), "from the cove's head");
        assert!(bottom > y(2100.0), "out past its mouth");
        assert!(bottom < y(1700.0 + OUT_WIDTHS * 200.0 + 100.0), "but no farther than a few of its widths");
        assert!((s.at(anchor * s.length()).0[1] - y(1700.0)).abs() < y(100.0) - y(0.0), "the anchor is the point");
    }

    #[test]
    fn a_small_coves_name_is_smaller_down_to_a_least_size() {
        assert_eq!(point_em(10.0), POINT_EM_LEAST);
        assert!(point_em(40.0) > POINT_EM_LEAST && point_em(40.0) < POINT_EM_MOST);
        assert_eq!(point_em(1000.0), POINT_EM_MOST);
    }

    fn lagoon_tile(id: TileId, places: Vec<Place>) -> (TileId, Arc<PlacesTile>) {
        let lake = vec![[400, 400], [800, 0], [3700, 2900], [3300, 3300]];
        (id, Arc::new(PlacesTile { extent: 4096, places, lakes: vec![lake], ..Default::default() }))
    }

    fn direction(s: &Spine) -> f64 {
        let (a, b) = (s.pts[0], *s.pts.last().unwrap());
        (b[1] - a[1]).atan2(b[0] - a[0]).to_degrees()
    }

    #[test]
    fn a_lagoon_is_set_along_the_whole_of_its_lake_wherever_its_line_is() {
        let id = TileId { z: 10, x: 500, y: 400 };
        // Its line in two pieces that do not meet, as the Étang de Thau's.
        let piece = |a: [i16; 2], b: [i16; 2]| water("Étang de Test", PlaceKind::Lagoon, a, Some(vec![a, b]));
        let tiles = vec![lagoon_tile(id, vec![piece([700, 300], [1700, 1300]), piece([2200, 1800], [3200, 2800])])];
        let c = candidates(&tiles).remove(0);
        assert_eq!(c.lines.len(), 2);
        let job = Job { key: c.key().unwrap(), size: 12.0, cand: c, tiles };
        let s = compute(&job).expect("a spine along the lake");
        let tile = 256.0 / 1024.0;
        assert!(s.length() > 0.7 * 3000.0 / 4096.0 * tile * 2f64.sqrt(), "the whole lake: {}", s.length() / tile * 4096.0);
        assert!((direction(&s) - 45.0).abs() < 10.0, "along it: {}", direction(&s));
    }

    #[test]
    fn a_lagoon_named_at_a_point_follows_its_long_axis() {
        let id = TileId { z: 10, x: 500, y: 400 };
        let mut tiles = vec![lagoon_tile(id, vec![water("Étang", PlaceKind::Lagoon, [2000, 1700], None)])];
        for (dx, dy) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
            tiles.push((TileId { z: 10, x: (500 + dx) as u32, y: (400 + dy) as u32 }, Arc::new(PlacesTile { extent: 4096, ..Default::default() })));
        }
        let c = candidates(&tiles).into_iter().find(|c| c.kind == PlaceKind::Lagoon).unwrap();
        assert_eq!(c.key().unwrap().2, 0, "worked out once, whatever the tiles' zoom");
        let job = Job { key: c.key().unwrap(), size: 12.0, cand: c, tiles };
        let s = compute(&job).expect("an axis");
        assert!((direction(&s) - 45.0).abs() < 10.0, "along the lake: {}", direction(&s));
        assert!(s.pts.len() > 2);
        let unit = |x: f64, y: f64| unit_of(&id, 4096.0, [x, y]);
        let (lo, hi) = (unit(300.0, -100.0), unit(3800.0, 3400.0));
        assert!(s.pts.iter().all(|p| p[0] >= lo[0] && p[0] <= hi[0] && p[1] >= lo[1] && p[1] <= hi[1]), "inside the lake's box");
    }

    #[test]
    fn a_line_that_is_not_on_the_sea_has_no_spine() {
        let id = TileId { z: 10, x: 500, y: 400 };
        // No sea in the tile: a lake's line.
        let tiles = vec![tile(id, vec![], vec![water("Lac", PlaceKind::Bay, [2000, 2000], Some(vec![[500, 2000], [3500, 2000]]))])];
        let c = candidates(&tiles).remove(0);
        let job = Job { key: c.key().unwrap(), size: 12.0, cand: c, tiles };
        assert!(compute(&job).is_none());
    }

    #[test]
    fn an_island_is_set_along_its_own_land_if_the_tiles_show_all_of_it() {
        let id = TileId { z: 10, x: 500, y: 400 };
        // An island 2400 by 700 tile units, a hole in the sea.
        let island = vec![[800, 1700], [800, 2400], [3200, 2400], [3200, 1700]];
        let sea = vec![square(-64, -64, 4224), island];
        let tiles = vec![tile(id, sea, vec![water("Ile Longue", PlaceKind::Island, [2000, 2050], None)])];
        let c = candidates(&tiles).remove(0);
        assert!(c.points.len() == 1 && c.lines.is_empty());
        let job = Job { key: c.key().unwrap(), size: 12.0, cand: c.clone(), tiles: tiles.clone() };
        let s = compute(&job).expect("a spine along the island");
        let unit = |x: f64, y: f64| unit_of(&id, 4096.0, [x, y]);
        let (a, b) = (unit(800.0, 1700.0), unit(3200.0, 2400.0));
        assert!(s.pts.iter().all(|p| p[0] >= a[0] && p[0] <= b[0] && p[1] >= a[1] && p[1] <= b[1]), "{:?}", s.pts);
        // An island the box cannot hold (land to its edge) is skipped.
        let land_all = vec![tile(id, vec![], vec![water("Continent", PlaceKind::Island, [2000, 2050], None)])];
        let c = candidates(&land_all).remove(0);
        let job = Job { key: c.key().unwrap(), size: 12.0, cand: c, tiles: land_all };
        assert!(compute(&job).is_none());
    }

    #[test]
    fn a_work_box_the_tiles_do_not_cover_is_not_worked_yet() {
        let id = TileId { z: 10, x: 500, y: 400 };
        let tiles = vec![channel_tile(id)];
        let b = tile_box(&id);
        let inside = grown([b[0] + 0.1, b[1] + 0.1, b[2] - 0.1, b[3] - 0.1], -0.05);
        assert!(covered(inside, &tiles));
        assert!(!covered(grown(b, 0.3), &tiles));
    }

    #[test]
    fn a_name_that_fits_its_spine_is_set_along_it_and_one_that_does_not_is_left_level() {
        use crate::core::geo::LatLon;
        use crate::palette::{Mode, Palette};
        use crate::view::{View, ViewInfo};
        // A strait 40 tiles of 4096 across... in units a long channel at tile zoom 10.
        let id = TileId { z: 10, x: 500, y: 400 };
        let hole = vec![[-64, -64], [4160, -64], [4160, 2400], [-64, 2400]];
        let sea = vec![square(-64, -64, 4224), hole];
        let line = vec![[100, 3300], [2000, 3250], [4000, 3300]];
        let mut tiles = vec![tile(id, sea, vec![water("Goulet de Test", PlaceKind::Strait, [2000, 3250], Some(line))])];
        // A name is longer than a tile at this zoom: the tiles round it are there too.
        for (dx, dy) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
            tiles.push(tile(TileId { z: 10, x: (500 + dx) as u32, y: (400 + dy) as u32 }, vec![square(-64, -64, 4224)], vec![]));
        }
        let b = id.bounds();
        let centre = LatLon::new((b.min_lat + b.max_lat) / 2.0, (b.min_lon + b.max_lon) / 2.0);
        let vi = ViewInfo { view: View { center: centre, zoom: 10.0 }, w: 1000.0, h: 700.0 };
        let area = TileArea { view: vi, vi, pal: Palette::new(Mode::Day, false) };
        let names = WaterNames::new();
        let changes = Rc::new(Changes::default());
        let redraw: Redraw = Rc::new(|| {});
        let first = names.labels_for(&area, &tiles, &tiles, vec![], &HashSet::new(), &[], &|_| false, &changes, &redraw);
        // Without a main loop the spine was worked out at once.
        assert!(changes.get() > 0, "the spine landed");
        assert_eq!(first.labels.len(), 1, "{:?}", first.names);
        let l = &first.labels[0];
        assert!(l.style.caps);
        assert!(first.names.contains("Goulet de Test"));
        assert_eq!(l.glyphs.iter().map(|g| g.0.as_str()).collect::<String>(), "GOULET DE TEST");
        // Too big for the view: left level.
        let big: HashSet<String> = ["Goulet de Test".to_string()].into();
        assert!(names.labels_for(&area, &tiles, &tiles, vec![], &big, &[], &|_| false, &changes, &redraw).labels.is_empty());
        // A Natural Earth name over the same stretch wins.
        let ne = Arc::new(PathLabel { id: 1, style: l.style, glyphs: vec![], height: 10.0, boxes: l.boxes.clone(), extent: l.extent, aka: vec![], units: l.units, zoom: l.zoom });
        assert!(names.labels_for(&area, &tiles, &tiles, vec![], &HashSet::new(), &[ne], &|_| false, &changes, &redraw).labels.is_empty());
    }

}
