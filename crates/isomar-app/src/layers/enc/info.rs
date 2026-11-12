//! What is at a place on the chart: the mark, the anchorage or the features under a click.

use super::*;

impl EncLayer {
    /// The mark within `tol` pixels of (`x`, `y`) in the cells in memory that
    /// the view shows, the finest cell first.
    pub fn info_at(&self, vi: &ViewInfo, x: f64, y: f64, tol: f64) -> Option<AidInfo> {
        let zoom = vi.zoom();
        let denom = view_scale(vi.view.center.lat, zoom);
        let near = aidinfo::click_bounds(vi, x, y, tol + SAME_SPOT_PX);
        let view = vi.bounds();
        let (mut aids, mut lights) = (Vec::new(), Vec::new());
        let cells = self.cells.borrow();
        for e in cells.iter().rev().filter(|e| self.entry_visible(&e.meta, denom) && e.meta.bounds.intersects(&near)) {
            let Some(cell) = e.cell.borrow().clone() else { continue };
            // Only what is drawn: each cell declutters its own lights in view, as `draw_cell` does.
            let in_view: Vec<Light> = cell
                .cell
                .features
                .iter()
                .filter(|f| f.objl == obj::LIGHTS)
                .filter_map(|f| if let Geom::Point(p) = &f.geom { view.contains(*p).then(|| light_of(f, *p)) } else { None })
                .collect();
            let declutter = Declutter::new(vi, &in_view);
            lights.extend(in_view.into_iter().filter(|l| near.contains([l.pos.lon, l.pos.lat]) && declutter.light(l)));
            for f in &cell.cell.features {
                let Geom::Point(p) = &f.geom else { continue };
                if f.objl != obj::LIGHTS
                    && near.contains(*p)
                    && let Some(a) = aid_of(f, *p)
                    && declutter.glyph(a.kind, &a.pos).is_some()
                {
                    aids.push(a);
                }
            }
        }
        aidinfo::pick(vi, x, y, tol, aids, lights)
    }

    /// The anchorage under (`x`, `y`) in the cells the view shows, the finest first, with how far its anchor is
    /// (`anchorages::hit`) and what the seabed is there.
    pub fn anchorage_at(&self, vi: &ViewInfo, x: f64, y: f64, tol: f64) -> Option<(f64, Anchorage, Vec<u8>)> {
        let zoom = vi.zoom();
        let denom = view_scale(vi.view.center.lat, zoom);
        let near = aidinfo::click_bounds(vi, x, y, tol);
        let at = vi.position_at(x, y);
        let cells = self.cells.borrow();
        for e in cells.iter().rev().filter(|e| self.entry_visible(&e.meta, denom) && e.meta.bounds.intersects(&near)) {
            let Some(cell) = e.cell.borrow().clone() else { continue };
            let found = cell
                .cell
                .features
                .iter()
                .filter(|f| matches!(f.objl, obj::RESARE | obj::ACHARE | obj::ACHBRT) && feature_bounds(f).intersects(&near))
                .filter_map(|f| Some((f, anchorage_of(f)?)))
                .filter(|(f, a)| anchorage_shown(f, a, zoom, denom))
                .map(|(_, a)| a)
                .filter_map(|a| Some((anchorages::hit(vi, &a, x, y, tol)?, a)))
                .min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((d, a)) = found {
                let seabed = cell
                    .cell
                    .features
                    .iter()
                    .filter(|f| f.objl == obj::SBDARE)
                    .find(|f| match &f.geom {
                        Geom::Area(rings) => rings.iter().filter(|r| core_vector::ring_contains(r, [at.lon, at.lat])).count() % 2 == 1,
                        Geom::Point(p) => near.contains(*p),
                        _ => false,
                    })
                    .map(|f| f.list(attr::NATSUR))
                    .unwrap_or_default();
                return Some((d, a, seabed));
            }
        }
        None
    }

    /// The features under (`x`, `y`) that are neither marks nor anchorages, told in words and outlined: a danger,
    /// a cable, a restricted area, a leading line… from the finest cell that has any there, as drawn at this
    /// scale, the one a click opens first leading. Coarser cells are not asked past the one drawn at the pointer:
    /// what they have there is under it, and a coarse cell's areas (a caution over a whole bay) lit up wherever
    /// the finer cell had nothing to offer.
    pub fn features_at(&self, vi: &ViewInfo, x: f64, y: f64, tol: f64) -> Vec<crate::probe::FeatureHit> {
        let denom = view_scale(vi.view.center.lat, vi.zoom());
        let near = aidinfo::click_bounds(vi, x, y, tol);
        let at = vi.position_at(x, y);
        let cells = self.cells.borrow();
        for e in cells.iter().rev().filter(|e| self.entry_visible(&e.meta, denom) && e.meta.bounds.intersects(&near)) {
            let Some(cell) = e.cell.borrow().clone() else { continue };
            let here = cell.cell.features.iter().zip(&cell.bounds).filter(|(f, b)| b.intersects(&near.expanded(near.width())) && scamin_shows(f, denom)).map(|(f, _)| f);
            let found = crate::featureinfo::pick_all(vi, here, x, y, tol);
            if found.is_empty() {
                if has_data(&e.meta, [at.lon, at.lat]) {
                    break;
                }
                continue;
            }
            return found
                .into_iter()
                .filter_map(|f| {
                    let pos = match &f.geom {
                        Geom::Point(p) => LatLon::new(p[1], p[0]),
                        _ => at,
                    };
                    crate::probe::FeatureHit::of_feature(f, crate::featureinfo::describe(f, &pos))
                })
                .collect();
        }
        Vec::new()
    }
}
