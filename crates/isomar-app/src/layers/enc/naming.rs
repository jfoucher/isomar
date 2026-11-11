//! The names a cell gives the chart (`OBJNAM`): which show, and where each is set.

use super::*;

/// What a cell's name is the name of, in the order names win a crowded spot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum NameKind {
    /// SEAARE: seas, bays, passes, and shoals named as sea areas.
    Sea,
    /// BUAARE: towns and villages.
    Town,
    /// LNDRGN: capes, points, peninsulas, hills.
    Land,
    /// A named LNDARE: an island, islet or rock above water.
    Island,
    /// FAIRWY: a channel.
    Channel,
    /// A named rock above or awash, close in only. Wrecks, obstructions and seabed areas have no name on
    /// a chart (S-52 prints none; NOAA puts the vessel and its description in `OBJNAM`): the popover has it.
    Danger,
}

impl NameKind {
    pub(super) fn of(objl: u16) -> Option<NameKind> {
        Some(match objl {
            obj::SEAARE => NameKind::Sea,
            obj::BUAARE => NameKind::Town,
            obj::LNDRGN => NameKind::Land,
            obj::LNDARE => NameKind::Island,
            obj::FAIRWY => NameKind::Channel,
            obj::UWTROC => NameKind::Danger,
            _ => return None,
        })
    }

    /// Waters in sloping blue, dangers in sloping black, land upright, as on a
    /// paper chart (INT 1); a sea area's name grows with the area on screen.
    pub(super) fn style(self, on_screen: f64) -> names::Style {
        use names::Style;
        use pango::Weight as W;
        match self {
            NameKind::Sea => Style::water(names::water_size(on_screen)),
            NameKind::Town => Style::land(13.0, W::Semibold, 0.0),
            NameKind::Land | NameKind::Island => Style::land(12.5, W::Normal, 0.0),
            NameKind::Channel => Style::water(12.0),
            NameKind::Danger => Style::danger(11.0),
        }
    }

    /// How a name set along a spine is written: seas as seas, channels as straits, islands as islands, capes as peninsulas.
    pub(super) fn geo_kind(self) -> Option<GeoKind> {
        Some(match self {
            NameKind::Sea => GeoKind::Sea,
            NameKind::Channel => GeoKind::Strait,
            NameKind::Island => GeoKind::Island,
            NameKind::Land => GeoKind::Peninsula,
            _ => return None,
        })
    }

    /// Whether the name must be wholly on the water.
    pub(super) fn on_water(self) -> bool {
        matches!(self, NameKind::Sea | NameKind::Channel)
    }
}

/// A name in a cell and where it is written, worked out once when the cell is read.
pub(super) struct CellName {
    pub(super) text: String,
    pub(super) kind: NameKind,
    /// `[lon, lat]`: the point, the middle of a line, or a point inside an area.
    pub(super) pos: [f64; 2],
    /// The scale denominator past which it is not shown (SCAMIN), if the cell says.
    pub(super) scamin: Option<f64>,
    /// The size of the area it names, in degrees; 0 for a point.
    pub(super) extent: f64,
    /// The centreline of an area it names, in units (Web Mercator at zoom 0, 256 across), to set the
    /// name along it: seas and channels along their water, islands and land regions along their land.
    pub(super) spine: Option<Arc<Spine>>,
    /// A cape's tip and the way its point runs, or a cove's mouth and the way out of it, in units: its name is
    /// written out from there, as a paper chart writes both.
    pub(super) out_from: Option<CapeAxis>,
}

/// How far round a cape its point is looked at, in metres, at least.
pub(super) const CAPE_RADIUS_M: f64 = 400.0;

/// An area is set along a spine only when it is bigger than this, in degrees.
pub(super) const MIN_SPINE_EXTENT: f64 = 0.01;
/// Cells along the longer side of the grid a spine is worked out on.
pub(super) const SPINE_ACROSS: usize = 128;

pub(super) fn to_units(p: [f64; 2]) -> [f64; 2] {
    let (x, y) = crate::core::geo::project(&LatLon::new(p[1], p[0]), 0.0);
    [x, y]
}

/// The centreline of an area (its rings, exterior first): of its water, less the cell's land, for a sea
/// area or a channel; of the area itself for land.
pub(super) fn area_spine(kind: NameKind, rings: &[Vec<[f64; 2]>], land: &[(Bounds, Vec<Vec<[f64; 2]>>)]) -> Option<Arc<Spine>> {
    let units: Vec<Vec<[f64; 2]>> = rings.iter().map(|r| r.iter().map(|p| to_units(*p)).collect()).collect();
    let b = units.iter().flatten().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]);
    let mut mask = Mask::over(b, SPINE_ACROSS);
    mask.fill(&units, true);
    if kind.on_water() {
        let area = Bounds::of(rings.first()?);
        let mut ground = mask.empty_like();
        for (lb, lrings) in land {
            if lb.intersects(&area) {
                ground.fill(lrings, true);
            }
        }
        mask.minus(&ground);
    }
    spine(&mask.largest_component()).map(Arc::new)
}

/// Whether a point (units) is on the cell's land near `pos` (`[lon, lat]`; an area `extent` degrees across), from
/// `land` (each area's bounds in degrees and its rings in units), and how far round it to look, in units.
pub(super) fn land_near<'a>(pos: [f64; 2], extent: f64, land: &'a [(Bounds, Vec<Vec<[f64; 2]>>)]) -> (impl Fn(core_vector::Pt) -> bool + 'a, f64) {
    let metres = CAPE_RADIUS_M.max(extent * 111_320.0);
    let units_per_m = 256.0 / (40_075_016.7 * pos[1].to_radians().cos());
    let near = Bounds::of(&[[pos[0] - extent - 0.05, pos[1] - extent - 0.05], [pos[0] + extent + 0.05, pos[1] + extent + 0.05]]);
    let rings: Vec<(core_vector::Pt, core_vector::Pt, &Vec<[f64; 2]>)> = land
        .iter()
        .filter(|(b, _)| b.intersects(&near))
        .flat_map(|(_, rings)| rings.iter())
        .map(|r| {
            let b = r.iter().fold(([f64::MAX; 2], [f64::MIN; 2]), |(lo, hi), p| ([lo[0].min(p[0]), lo[1].min(p[1])], [hi[0].max(p[0]), hi[1].max(p[1])]));
            (b.0, b.1, r)
        })
        .collect();
    let on_land = move |p: core_vector::Pt| rings.iter().filter(|(lo, hi, r)| p[0] >= lo[0] && p[0] <= hi[0] && p[1] >= lo[1] && p[1] <= hi[1] && core_vector::ring_contains(r, p)).count() % 2 == 1;
    (on_land, metres * units_per_m)
}

/// The tip and axis of a cape named at `pos` (`[lon, lat]`; an area `extent` degrees across), from the cell's
/// land (`land`: each area's bounds in degrees and its rings in units); `None` off the coast.
pub(super) fn cape_of(pos: [f64; 2], extent: f64, land: &[(Bounds, Vec<Vec<[f64; 2]>>)]) -> Option<CapeAxis> {
    let (on_land, radius) = land_near(pos, extent, land);
    cape_axis(to_units(pos), &|p| !on_land(p), radius, radius / 2.0)
}

/// The mouth of a cove named at `pos` and the way out of it, from the cell's land as `cape_of`; `None` where
/// the land does not hold the water on both sides (open water, an open shore).
pub(super) fn cove_of(pos: [f64; 2], extent: f64, land: &[(Bounds, Vec<Vec<[f64; 2]>>)]) -> Option<CapeAxis> {
    let (on_land, radius) = land_near(pos, extent, land);
    cove_axis(to_units(pos), &|p| !on_land(p), radius)
}

pub(super) fn names_of(cell: &Cell) -> Vec<CellName> {
    // The cell's land in units, worked out when a sea area first needs it.
    let land = std::cell::OnceCell::new();
    let land_rings = || {
        land.get_or_init(|| {
            cell.features
                .iter()
                .filter(|f| f.objl == obj::LNDARE)
                .filter_map(|f| match &f.geom {
                    Geom::Area(r) => Some((Bounds::of(r.first()?), r.iter().map(|ring| ring.iter().map(|p| to_units(*p)).collect()).collect())),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
    };
    // Sea areas big enough for a spine: a small piece of one (a cell cuts Casco Bay into many) is not a cove.
    let big: HashSet<&str> = cell
        .features
        .iter()
        .filter(|f| f.objl == obj::SEAARE)
        .filter_map(|f| match &f.geom {
            Geom::Area(rings) => Some((f.attr(attr::OBJNAM)?.trim(), Bounds::of(rings.first()?))),
            _ => None,
        })
        .filter(|(_, b)| b.width().max(b.height()) > MIN_SPINE_EXTENT)
        .map(|(n, _)| n)
        .collect();
    cell.features
        .iter()
        .filter_map(|f| {
            let kind = NameKind::of(f.objl)?;
            let text = f.attr(attr::OBJNAM)?.trim();
            let (pos, extent, spine) = match &f.geom {
                Geom::Point(p) => (*p, 0.0, None),
                Geom::Line(pts) => (*pts.get(pts.len() / 2)?, 0.0, None),
                Geom::Area(rings) => {
                    let b = Bounds::of(rings.first()?);
                    let extent = b.width().max(b.height());
                    let named_area = kind == NameKind::Sea && f.attr(attr::CATSEA).is_none() && is_scheme_name(text);
                    let spine = (extent > MIN_SPINE_EXTENT && !named_area && matches!(kind, NameKind::Sea | NameKind::Channel | NameKind::Island | NameKind::Land)).then(|| area_spine(kind, rings, land_rings()));
                    (core_vector::label_point(rings)?, extent, spine.flatten())
                }
                _ => return None,
            };
            if text.is_empty() {
                return None;
            }
            // A cape is named out from its tip, where the cell's land says it is, and a cove too small for a spine
            // (a bay by its category, or by its name) out from its mouth.
            let out_from = match kind {
                NameKind::Land if is_cape_name(text) => cape_of(pos, extent, land_rings()),
                NameKind::Sea if spine.is_none() && !big.contains(text) && (f.num(attr::CATSEA) == Some(5.0) || is_cove_name(text)) => cove_of(pos, extent, land_rings()),
                _ => None,
            };
            let spine = if out_from.is_some() { None } else { spine };
            Some(CellName { text: text.to_string(), kind, pos, scamin: f.num(attr::SCAMIN).filter(|s| *s > 0.0), extent, spine, out_from })
        })
        .collect()
}

/// Whether a sea area's name is a traffic scheme's or a deepwater port's: NOAA files those as `SEAARE`s of no
/// category, which are no sea to spread a name over (S-52 prints them as plain text).
pub(super) fn is_scheme_name(text: &str) -> bool {
    text.split(|c: char| !c.is_alphanumeric()).any(|w| ["scheme", "lane", "lanes", "deepwater"].iter().any(|k| w.eq_ignore_ascii_case(k)))
}

/// View zoom from which named rocks and shoals without a SCAMIN are written.
pub(super) const MIN_ZOOM_DANGER_NAMES: f64 = 13.0;
/// View zoom from which islets without a SCAMIN are written, unless they are big enough to see.
pub(super) const MIN_ZOOM_ISLET_NAMES: f64 = 12.0;
/// How far past a tile, in pixels, names are placed with their neighbours.
pub(super) const NAME_REACH_PX: f64 = 200.0;
/// The same name this close to one already written is a second cell's copy of it.
pub(super) const SAME_NAME_PX: f64 = 300.0;
/// A name set along an area is not written again within this many of its lengths.
pub(super) const SAME_PATH_NAME_LENGTHS: f64 = 3.0;

/// Whether the cell `e` has data at `p`: inside its coverage, or its bounds when it has none.
pub(super) fn has_data(e: &CellSummary, p: [f64; 2]) -> bool {
    e.bounds.contains(p) && (e.cover.covered.is_empty() || e.cover.contains(p))
}

/// How far past its compilation scale a cell's names are written: a coarse
/// cell left on screen under a harbour view does not name the whole sea there.
pub(super) const NAME_MAX_OVERSCALE: f64 = 16.0;

/// Whether a name is shown at the view scale 1:`denom` and `zoom`, from a cell of
/// scale 1:`cscl`, in a view `view_px` pixels across: as the cell says (SCAMIN),
/// else areas big enough on screen to hold it, dangers and islets close in; never
/// an area wider than the view (it is all round) nor from a cell far overscaled.
pub(super) fn name_shows(n: &CellName, cscl: u32, denom: f64, zoom: f64, px_per_deg: f64, view_px: f64) -> bool {
    let on_screen = n.extent * px_per_deg;
    if on_screen > view_px || (cscl > 0 && denom < cscl as f64 / NAME_MAX_OVERSCALE) {
        return false;
    }
    if let Some(scamin) = n.scamin {
        return denom <= scamin;
    }
    match n.kind {
        NameKind::Danger => zoom >= MIN_ZOOM_DANGER_NAMES,
        NameKind::Island => zoom >= MIN_ZOOM_ISLET_NAMES || on_screen >= 40.0,
        _ => n.extent == 0.0 || on_screen >= 20.0,
    }
}

/// The finest of `cells` with data at `p`.
pub(super) fn cell_at(cells: &[Ready], p: [f64; 2]) -> Option<&Ready> {
    cells.iter().rev().find(|c| has_data(&c.meta, p))
}

/// What a tile writes of the names of its cells.
pub(super) struct NamePlan {
    /// Set level, each at its place.
    pub(super) placed: Vec<Placed>,
    /// Set along a spine: the tile draws them with `names::draw_paths`.
    pub(super) paths: Vec<Arc<PathLabel>>,
    /// The boxes of the letters of `paths`, on the tile's screen.
    pub(super) boxes: Vec<[f64; 4]>,
}

/// Room kept between names set along paths that collide, in pixels.
pub(super) const PATH_GAP_PX: f64 = 4.0;

pub(super) fn overlap(a: &[f64; 4], b: &[f64; 4]) -> bool {
    a[0] < b[2] && a[2] > b[0] && a[1] < b[3] && a[3] > b[1]
}

/// Sets the names with a spine along it at this zoom, and settles the ones that collide: the most
/// important of two wins (the order names win a crowded spot in, then the bigger area), whichever tile
/// asks. What does not fit, or loses, is left for the level placement.
/// `same` holds every name with a spine shown at this zoom in the cells drawn, whatever the tile: a name
/// with the same text as a bigger area's close by is not written again (a cell names each piece of a
/// land region, and VIRGINIA came out five times round the Chesapeake).
pub(super) fn set_along_spines<'a>(labels: Vec<(&'a CellName, f64, f64)>, zoom: f64, same: &[&CellName]) -> (Vec<(&'a CellName, PathLabel)>, Vec<(&'a CellName, f64, f64)>) {
    let ppu = 2f64.powf(zoom);
    // Fitted for the band of zooms round this one, and scaled with the chart through it.
    let band = names::label_zoom(zoom);
    let mut fitted: Vec<(&CellName, PathLabel, f64, f64)> = Vec::new();
    let mut rest = Vec::new();
    for (n, x, y) in labels {
        let along = n.spine.as_ref().zip(n.kind.geo_kind()).and_then(|(spine, kind)| {
            let units = spine.pts.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]);
            let id = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                (&n.text, n.pos.map(f64::to_bits)).hash(&mut h);
                h.finish() >> 8
            };
            set_along(kind, &n.text, spine, 2f64.powf(band), id, vec![n.text.trim().to_lowercase()], units, &mut |t, st| Rc::new(names::measure(t, st)), &|_| false).map(|l| l.at_zoom(zoom))
        });
        match along {
            Some(l) => fitted.push((n, l, x, y)),
            None => rest.push((n, x, y)),
        }
    }
    let hits = |a: &PathLabel, b: &PathLabel| a.boxes.iter().any(|x| b.boxes.iter().any(|y| overlap(&[x[0] - PATH_GAP_PX, x[1] - PATH_GAP_PX, x[2] + PATH_GAP_PX, x[3] + PATH_GAP_PX], y)));
    let priority = |n: &CellName| (n.kind, ordered(-n.extent), n.text.clone());
    let at = |n: &CellName| {
        let u = to_units(n.pos);
        [u[0] * ppu, u[1] * ppu]
    };
    let outranked = |n: &CellName, l: &PathLabel| {
        let reach = SAME_PATH_NAME_LENGTHS * (l.extent[2] - l.extent[0]).max(l.extent[3] - l.extent[1]);
        let p = at(n);
        same.iter().any(|m| m.text == n.text && (m.extent, m.pos[0], m.pos[1]) > (n.extent, n.pos[0], n.pos[1]) && {
            let q = at(m);
            (q[0] - p[0]).hypot(q[1] - p[1]) < reach
        })
    };
    let mut kept = Vec::new();
    for (i, (n, l, x, y)) in fitted.iter().enumerate() {
        let mine = priority(n);
        if outranked(n, l) {
            continue;
        }
        if fitted.iter().enumerate().any(|(j, (n2, l2, _, _))| j != i && priority(n2) < mine && hits(l, l2)) {
            rest.push((*n, *x, *y));
        } else {
            kept.push((*n, l.clone()));
        }
    }
    (kept, rest)
}

/// The names of `capes` and coves, each written out from its tip or its mouth on the water of the cells drawn
/// (`cape_label`), fitted for the band of zooms round `zoom` and scaled to it, clear of the names set `along`
/// spines and of each other: of two that collide, the one first in the order of their names and places wins,
/// whichever tile asks. One that does not fit is left out, rather than written level where the cell puts it,
/// inland of the point.
pub(super) fn set_capes(capes: &[(&CellName, f64, f64)], cells: &[Ready], zoom: f64, along: &[PathLabel]) -> Vec<PathLabel> {
    let band = names::label_zoom(zoom);
    let ppu = 2f64.powf(band);
    let water = |p: core_vector::Pt| {
        let at = crate::core::geo::unproject(p[0] / ppu, p[1] / ppu, 0.0);
        let q = [at.lon, at.lat];
        cell_at(cells, q).map(|c| !c.cell.land_at(q))
    };
    let mut fitted: Vec<(&CellName, PathLabel)> = Vec::new();
    for (n, _, _) in capes {
        let Some(axis) = n.out_from else { continue };
        let style = n.kind.style(0.0).on_sea();
        let m = names::measure(&n.text, &style);
        let id = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (&n.text, n.pos.map(f64::to_bits)).hash(&mut h);
            h.finish() >> 8
        };
        let units = [axis.tip[0], axis.tip[1], axis.tip[0], axis.tip[1]];
        if let Some(l) = cape_label(id, &n.text, &m, style, &axis, ppu, band, vec![n.text.trim().to_lowercase()], units, &water, &|_| false) {
            fitted.push((n, l.at_zoom(zoom)));
        }
    }
    let hits = |a: &PathLabel, b: &PathLabel| a.boxes.iter().any(|x| b.boxes.iter().any(|y| overlap(&[x[0] - PATH_GAP_PX, x[1] - PATH_GAP_PX, x[2] + PATH_GAP_PX, x[3] + PATH_GAP_PX], y)));
    let blocked = |l: &PathLabel| along.iter().any(|a| hits(l, a));
    let priority = |n: &CellName| (n.text.clone(), ordered(n.pos[0]), ordered(n.pos[1]));
    fitted
        .iter()
        .filter(|(n, l)| !blocked(l) && !fitted.iter().any(|(n2, l2)| priority(n2) < priority(n) && !blocked(l2) && hits(l, l2)))
        .map(|(_, l)| l.clone())
        .collect()
}

/// A float as a sortable key.
pub(super) fn ordered(v: f64) -> i64 {
    (v * 1e9) as i64
}

/// Places the names of the cells drawn for the tile `ctx.vi`: each where its
/// cell is the one shown (the finest with data there), the most important first,
/// none over another, a water's wholly on water (moved a little, or left out).
/// Areas with a spine are set along it first, and the rest go round their letters.
pub(super) fn plan_names(ctx: &DrawCtx, cells: &[Ready], denom: f64, view_px: f64) -> NamePlan {
    let (cr, vi, pal) = (ctx.cr, &ctx.vi, &ctx.pal);
    let zoom = vi.zoom();
    let px_per_deg = 1.0 / vi.deg_per_px();
    let near = vi.bounds().expanded(NAME_REACH_PX * vi.deg_per_px());
    let mut labels: Vec<(&CellName, f64, f64)> = Vec::new();
    for (i, rc) in cells.iter().enumerate() {
        if !rc.meta.bounds.intersects(&near) {
            continue;
        }
        for n in rc.cell.names.iter().filter(|n| near.contains(n.pos) && name_shows(n, rc.meta.cscl, denom, zoom, px_per_deg, view_px)) {
            // Cells are kept coarse first: a later one with data here is drawn over this one.
            if !has_data(&rc.meta, n.pos) || cells[i + 1..].iter().any(|f| has_data(&f.meta, n.pos)) {
                continue;
            }
            let (x, y) = vi.to_screen(&LatLon::new(n.pos[1], n.pos[0]));
            labels.push((n, x, y));
        }
    }
    labels.sort_by(|a, b| (a.0.kind, -a.0.extent, &a.0.text).partial_cmp(&(b.0.kind, -b.0.extent, &b.0.text)).unwrap_or(std::cmp::Ordering::Equal));
    let mut same: Vec<&CellName> = Vec::new();
    for (i, rc) in cells.iter().enumerate() {
        for n in rc.cell.names.iter().filter(|n| n.spine.is_some() && name_shows(n, rc.meta.cscl, denom, zoom, px_per_deg, view_px)) {
            if has_data(&rc.meta, n.pos) && !cells[i + 1..].iter().any(|f| has_data(&f.meta, n.pos)) {
                same.push(n);
            }
        }
    }
    let (capes, labels): (Vec<_>, Vec<_>) = labels.into_iter().partition(|(n, _, _)| n.out_from.is_some());
    let (along, labels) = set_along_spines(labels, zoom, &same);
    let along: Vec<PathLabel> = along.into_iter().map(|(_, l)| l).collect();
    let capes = set_capes(&capes, cells, zoom, &along);
    let paths: Vec<Arc<PathLabel>> = along.into_iter().chain(capes).map(Arc::new).collect();
    let path_boxes: Vec<[f64; 4]> = paths
        .iter()
        .flat_map(|p| p.boxes.iter())
        .map(|b| {
            let (x, y) = vi.world_to_screen(b[0], b[1]);
            [x, y, x + b[2] - b[0], y + b[3] - b[1]]
        })
        .collect();
    let on_land = |x: f64, y: f64| {
        let p = vi.position_at(x, y);
        let p = [p.lon, p.lat];
        cell_at(cells, p).is_some_and(|c| c.cell.land_at(p))
    };
    let mut placer = LabelPlacer::default();
    placer.reserve(path_boxes.iter().copied());
    let mut written: Vec<(&str, f64, f64)> = Vec::new();
    let mut placed = Vec::new();
    let scale = cr.target().device_scale().0;
    for (n, lx, ly) in labels {
        if written.iter().any(|&(t, x, y)| t == n.text && (x - lx).hypot(y - ly) < SAME_NAME_PX) {
            continue;
        }
        let mut st = n.kind.style(n.extent * px_per_deg);
        let water = n.kind.on_water();
        let ok = |x: f64, top: f64, w: f64, h: f64| !water || names::samples(x, top, w, h).all(|(sx, sy)| !on_land(sx, sy));
        let Some((mut name, fitted, x, top)) = names::fit(cr, pal, &mut placer, &n.text, st, lx, ly, water, ok) else { continue };
        st = fitted;
        let (w, h) = (name.w, name.h);
        written.push((&n.text, lx, ly));
        // A land name out on the water (an islet's) is outlined in the sea's colour.
        if !st.halo_is_sea() && !names::mostly(x, top, w, h, on_land) {
            st = st.on_sea();
            name = names::name(cr, &n.text, &st, pal, scale).unwrap_or(name);
        }
        placed.push(Placed { name, style: st, x, top, dot: None });
    }
    NamePlan { placed, paths, boxes: path_boxes }
}
