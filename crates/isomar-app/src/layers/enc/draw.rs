//! Drawing a cell into a tile: its coverage, what SCAMIN hides, and each class of feature.

use super::*;

/// Where each cell's data is (`ISOMAR_DEBUG_COVERAGE`): red is coverage,
/// orange a hole with none, purple the bounding box. Not dashed: cairo dashes
/// the whole length of a line, and a coarse cell's box can be thousands of
/// screens long.
pub(super) fn draw_coverage(vi: &ViewInfo, cr: &cairo::Context, shown: &[Ready]) {
    cr.set_line_width(2.0);
    for e in shown {
        let e = &e.meta;
        cr.new_path();
        for ring in &e.cover.covered {
            trace(vi, cr, ring, true);
        }
        cr.set_source_rgb(1.0, 0.0, 0.0);
        let _ = cr.stroke();
        for ring in &e.cover.holes {
            trace(vi, cr, ring, true);
        }
        cr.set_source_rgb(1.0, 0.55, 0.0);
        let _ = cr.stroke();
        trace_bounds(vi, cr, &e.bounds);
        cr.set_source_rgb(0.6, 0.0, 0.8);
        let _ = cr.stroke();
    }
}

/// An alpha mask of the area `e` has data for: its coverage polygons, less
/// the holes; its bounds when it has no coverage objects.
pub(super) fn coverage_mask(ctx: &DrawCtx, e: &CellSummary) -> Option<cairo::Pattern> {
    let (cr, vi) = (ctx.cr, &ctx.vi);
    cr.push_group();
    cr.set_fill_rule(cairo::FillRule::EvenOdd);
    cr.new_path();
    if e.cover.covered.is_empty() {
        trace_bounds(vi, cr, &e.bounds);
    }
    for ring in &e.cover.covered {
        trace(vi, cr, ring, true);
    }
    cr.set_source_rgba(0.0, 0.0, 0.0, 1.0);
    let _ = cr.fill();
    if !e.cover.holes.is_empty() {
        cr.set_operator(cairo::Operator::Clear);
        cr.new_path();
        for ring in &e.cover.holes {
            trace(vi, cr, ring, true);
        }
        let _ = cr.fill();
        cr.set_operator(cairo::Operator::Over);
    }
    cr.pop_group().ok()
}

pub(super) fn trace_bounds(vi: &ViewInfo, cr: &cairo::Context, b: &Bounds) {
    trace(vi, cr, &[[b.min_lon, b.min_lat], [b.max_lon, b.min_lat], [b.max_lon, b.max_lat], [b.min_lon, b.max_lat]], true);
}

/// The outline of the area `e` has data for, as a path.
pub(super) fn trace_data_limit(vi: &ViewInfo, cr: &cairo::Context, e: &CellSummary) {
    cr.new_path();
    if e.cover.covered.is_empty() {
        trace_bounds(vi, cr, &e.bounds);
    }
    for ring in e.cover.covered.iter().chain(&e.cover.holes) {
        trace(vi, cr, ring, true);
    }
}

/// Whether a feature shows at the view's scale 1:`denom`: what a cell gives a `SCAMIN` is hidden from smaller
/// scales, as S-52 has it. The depth areas, land and coastline always show, marks and lights are filtered
/// by their own rules, and soundings without SCAMIN are thinned.
pub(super) fn scamin_shows(f: &Feature, denom: f64) -> bool {
    let exempt = matches!(
        f.objl,
        obj::DEPARE
            | obj::DRGARE
            | obj::LNDARE
            | obj::COALNE
            | obj::LIGHTS
            | obj::LNDMRK
            | obj::BOYCAR
            | obj::BOYINB
            | obj::BOYISD
            | obj::BOYLAT
            | obj::BOYSAW
            | obj::BOYSPP
            | obj::BCNCAR
            | obj::BCNISD
            | obj::BCNLAT
            | obj::BCNSAW
            | obj::BCNSPP
    );
    exempt || f.num(attr::SCAMIN).filter(|s| *s > 0.0).is_none_or(|s| denom <= s)
}

/// The depth a contour line stands for: `VALDCO`, which real cells carry, else
/// the values older code looked for.
pub(super) fn contour_value(f: &Feature) -> f64 {
    f.num(attr::VALDCO).or_else(|| f.num(attr::VALSOU)).or_else(|| f.num(attr::DRVAL1)).unwrap_or(0.0)
}

/// Draws a cell's features. `feats` reach into the tile; `around` also those past its edge that decide
/// which soundings and hazards are kept. `declutter` says which of the cell's lights show.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_cell<'a>(ctx: &DrawCtx, entry: &CellSummary, declutter: &Declutter, feats: &[&'a Feature], around: &[&'a Feature], safety: f64, show_soundings: bool, names: &[[f64; 4]], denom: f64, relief: Option<&ReliefPaint>, elev: &Elev) {
    let (cr, vi, pal) = (ctx.cr, &ctx.vi, &ctx.pal);
    let zoom = vi.zoom();
    cr.set_fill_rule(cairo::FillRule::EvenOdd);
    cr.set_line_join(cairo::LineJoin::Round);
    cr.select_font_face(crate::layers::FONT_FAMILY, cairo::FontSlant::Normal, cairo::FontWeight::Normal);

    fn area_of(f: &Feature) -> Option<&[Vec<[f64; 2]>]> {
        if let Geom::Area(r) = &f.geom { Some(r) } else { None }
    }

    // What the cell's author hid at this scale (SCAMIN) stays hidden, as S-52 has it.
    let at_scale = |list: &[&'a Feature]| -> Vec<&'a Feature> { list.iter().copied().filter(|f| scamin_shows(f, denom)).collect() };
    let (feats, around) = (at_scale(feats), at_scale(around));
    let (feats, around) = (feats.as_slice(), around.as_slice());
    // Names, symbols and soundings keep clear of each other; no sounding is written over a name.
    let mut sounding_labels = LabelPlacer::with(names.iter().copied());

    // Depth areas, shallow to deep so overlaps resolve to the shoaler water.
    let mut depth_areas: Vec<(&Feature, f64)> = feats.iter().filter(|f| f.objl == obj::DEPARE).map(|f| (*f, f.num(attr::DRVAL1).unwrap_or(0.0))).collect();
    depth_areas.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    for (f, d1) in &depth_areas {
        if let Some(rings) = area_of(f) {
            cr.new_path();
            for r in rings {
                trace(vi, cr, r, true);
            }
            pal.set(cr, depth_area_colour(*d1, f.num(attr::DRVAL2), safety));
            let _ = cr.fill();
        }
    }
    chartinfo::draw_unsurveyed(vi, cr, pal, feats);
    // Their outlines, less the stretches along the edge of the cell's data: a
    // cell ends where the next begins, and a line there would be a chart edge
    // that is not in the water.
    cr.push_group();
    cr.new_path();
    for (f, _) in &depth_areas {
        if let Some(rings) = area_of(f) {
            for r in rings {
                trace(vi, cr, r, true);
            }
        }
    }
    pal.set_alpha(cr, rgb(120, 150, 180), 0.5);
    cr.set_line_width(0.6);
    let _ = cr.stroke();
    trace_data_limit(vi, cr, entry);
    cr.set_operator(cairo::Operator::DestOut);
    cr.set_source_rgba(0.0, 0.0, 0.0, 1.0);
    cr.set_line_width(2.5);
    let _ = cr.stroke();
    cr.set_operator(cairo::Operator::Over);
    if let Ok(outlines) = cr.pop_group() {
        let _ = cr.set_source(&outlines);
        let _ = cr.paint();
    }
    for f in feats.iter().filter(|f| matches!(f.objl, obj::DRGARE | obj::FAIRWY)) {
        if let Some(rings) = area_of(f) {
            cr.new_path();
            for r in rings {
                trace(vi, cr, r, true);
            }
            pal.set_alpha(cr, MAGENTA, 0.8);
            cr.set_line_width(1.0);
            cr.set_dash(&[6.0, 4.0], 0.0);
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);
        }
    }
    for f in feats.iter().filter(|f| f.objl == obj::LNDARE || f.objl == obj::BUAARE) {
        if let Some(rings) = area_of(f) {
            cr.new_path();
            for r in rings {
                trace(vi, cr, r, true);
            }
            pal.set(cr, if f.objl == obj::BUAARE { Paint::Legacy(rgb(226, 214, 190)) } else { LAND.into() });
            let _ = cr.fill();
        }
    }
    // The downloaded relief over the land: shading, and contours too where the cell has none of its own.
    if let Some(relief) = relief {
        let _ = cr.save();
        cr.new_path();
        for f in feats.iter().filter(|f| f.objl == obj::LNDARE) {
            if let Some(rings) = area_of(f) {
                for r in rings {
                    trace(vi, cr, r, true);
                }
            }
        }
        cr.clip();
        relief.paint(ctx, !elev.lines);
        let _ = cr.restore();
    }
    // Coastline and shoreline constructions.
    cr.set_line_width(1.3);
    for f in feats.iter().filter(|f| matches!(f.objl, obj::COALNE | obj::SLCONS)) {
        if let Geom::Line(l) = &f.geom {
            cr.new_path();
            trace(vi, cr, l, false);
            pal.set(cr, LAND_EDGE);
            let _ = cr.stroke();
        }
    }
    // Land contours: thin, and the index ones heavier, in the brown of a paper chart.
    if elev.lines {
        let (mut thin, mut index) = (Vec::new(), Vec::new());
        for f in feats.iter().filter(|f| f.objl == obj::LNDELV) {
            if let Geom::Line(l) = &f.geom {
                if f.num(attr::ELEVAT).is_some_and(|e| elev.is_index(e)) { &mut index } else { &mut thin }.push(l);
            }
        }
        for (lines, token, width) in [(thin, Token::ChartLandContour, 0.6), (index, Token::ChartLandContourIndex, 1.1)] {
            if lines.is_empty() {
                continue;
            }
            cr.new_path();
            for l in lines {
                trace(vi, cr, l, false);
            }
            pal.set(cr, token);
            cr.set_line_width(width);
            let _ = cr.stroke();
        }
    }
    // Depth contours, with the safety contour heavy.
    for f in feats.iter().filter(|f| f.objl == obj::DEPCNT) {
        if let Geom::Line(l) = &f.geom {
            let is_safety = (contour_value(f) - safety).abs() < 0.01;
            cr.new_path();
            trace(vi, cr, l, false);
            pal.set_alpha(cr, if is_safety { rgb(30, 70, 140) } else { rgb(90, 130, 175) }, if is_safety { 1.0 } else { 0.8 });
            cr.set_line_width(if is_safety { 1.8 } else { 0.8 });
            let _ = cr.stroke();
        }
    }
    // Cables, pipelines, leading lines and recommended tracks.
    chartinfo::draw_lines(vi, cr, pal, feats, &mut sounding_labels);
    // Anchorages, berths and where anchoring is prohibited, as INT 1 draws them.
    for (f, a) in feats.iter().filter(|f| matches!(f.objl, obj::RESARE | obj::ACHARE | obj::ACHBRT)).filter_map(|f| Some((f, anchorage_of(f)?))) {
        if anchorage_shown(f, &a, zoom, denom) {
            anchorages::draw(cr, pal, vi, &a, true);
        }
    }
    // Restricted, caution and other areas, and the zones of confidence.
    chartinfo::draw_areas(vi, cr, pal, feats, |f| anchorage_of(f).is_some(), &mut sounding_labels);

    // Dangers: rocks, wrecks and obstructions, by what they are and how deep they lie against the safety depth.
    for f in around.iter().filter(|f| matches!(f.objl, obj::UWTROC | obj::WRECKS | obj::OBSTRN)) {
        let h = hazard::classify(f, safety);
        match &f.geom {
            Geom::Point(p) => {
                let (x, y) = vi.to_screen(&LatLon::new(p[1], p[0]));
                hazard::draw(cr, pal, x, y, &h);
                sounding_labels.try_place(x - 7.0, y - 7.0, 14.0, 14.0);
                if zoom >= 12.5
                    && let Some(d) = h.depth
                {
                    pal.set(cr, SOUNDING_INK);
                    draw_sounding(cr, &mut sounding_labels, x + 14.0, y - 6.0, d, 9.5);
                    if h.doubtful {
                        hazard::draw_doubtful_ring(cr, pal, x + 14.0, y - 6.0);
                    }
                }
            }
            Geom::Area(rings) => {
                // Dotted round the ground, the symbol in the middle once there is room for it.
                cr.new_path();
                for r in rings {
                    trace(vi, cr, r, true);
                }
                pal.set_alpha(cr, if h.dangerous { Paint::from(MAGENTA) } else { Token::ChartInk.into() }, 0.85);
                cr.set_line_width(1.1);
                cr.set_dash(&[1.2, 3.0], 0.0);
                let _ = cr.stroke();
                cr.set_dash(&[], 0.0);
                if let Some(c) = core_vector::label_point(rings) {
                    let (x, y) = vi.to_screen(&LatLon::new(c[1], c[0]));
                    let (x0, y0) = vi.to_screen(&LatLon::new(feature_bounds(f).min_lat, feature_bounds(f).min_lon));
                    let (x1, y1) = vi.to_screen(&LatLon::new(feature_bounds(f).max_lat, feature_bounds(f).max_lon));
                    if (x1 - x0).abs().min((y1 - y0).abs()) >= 22.0 && (0.0..vi.w).contains(&x) && (0.0..vi.h).contains(&y) && sounding_labels.try_place(x - 8.0, y - 8.0, 16.0, 16.0) {
                        hazard::draw(cr, pal, x, y, &h);
                    }
                }
            }
            _ => {}
        }
    }
    chartinfo::draw_symbols(vi, cr, pal, feats, &mut sounding_labels);

    // Spot heights on land: a dot and the height, upright, as on a printed chart.
    if zoom >= 11.0 {
        font_size(cr, 9.5);
        for f in around.iter().filter(|f| f.objl == obj::LNDELV) {
            let (Geom::Point(p), Some(h)) = (&f.geom, f.num(attr::ELEVAT)) else { continue };
            let (x, y) = vi.to_screen(&LatLon::new(p[1], p[0]));
            if !(-8.0..vi.w + 8.0).contains(&x) || !(-8.0..vi.h + 8.0).contains(&y) {
                continue;
            }
            let text = format!("{}", h.round() as i64);
            let w = cr.text_extents(&text).map_or(14.0, |e| e.width());
            if !sounding_labels.try_place(x - 3.0, y - 8.0, w + 8.0, 12.0) {
                continue;
            }
            pal.set(cr, Token::ChartLandContourIndex);
            cr.new_path();
            cr.arc(x, y, 1.6, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
            halo_text(cr, pal, x + 4.0, y + 3.5, &text, Token::ChartLandContourIndex);
        }
    }

    // Soundings, shallowest first so they win the space, all in one colour like a printed chart.
    if show_soundings {
        pal.set(cr, SOUNDING_INK);
        let m = sounding_reach();
        let (w, h) = (vi.w + m, vi.h + m);
        let mut pts: Vec<(f64, f64, f64)> = Vec::new();
        // Where the soundings not well known are, to ring them once thinned.
        let mut doubtful: HashSet<(u64, u64)> = HashSet::new();
        for f in around.iter().filter(|f| f.objl == obj::SOUNDG) {
            let unsure = hazard::doubtful_sounding(f);
            if let Geom::Soundings(s) = &f.geom {
                for p in s {
                    let (x, y) = vi.to_screen(&LatLon::new(p[1], p[0]));
                    if (-m..w).contains(&x) && (-m..h).contains(&y) {
                        pts.push((p[2], x, y));
                        if unsure {
                            doubtful.insert((x.to_bits(), y.to_bits()));
                        }
                    }
                }
            }
        }
        for (d, x, y) in thin_soundings(pts, sounding_cell()) {
            if draw_sounding(cr, &mut sounding_labels, x, y, d, 10.0) && doubtful.contains(&(x.to_bits(), y.to_bits())) {
                hazard::draw_doubtful_ring(cr, pal, x, y);
                pal.set(cr, SOUNDING_INK);
            }
        }
    }

    // Aids to navigation. Their names and light characteristics are not
    // printed; a click shows them (`EncLayer::info_at`).
    // Further out, only the lights that reach far and what they stand on.
    if zoom >= MIN_ZOOM_LIGHTS {
        let lights: Vec<Light> =
            feats.iter().filter(|f| f.objl == obj::LIGHTS).filter_map(|f| if let Geom::Point(p) = &f.geom { Some(light_of(f, *p)) } else { None }).collect();
        let mut fans = Fans::default();
        for l in lights.iter().filter(|l| declutter.light(l)) {
            let (x, y) = vi.to_screen(&l.pos);
            match l.sector {
                Some((a, b)) if declutter.sectors(l) => {
                    symbols::light_sector(cr, pal, x, y, symbols::sector_radius(l.range_nm, symbols::full_sectors(), vi.metres_per_px()), a, b, &l.colours);
                    symbols::light_dot(cr, pal, x, y);
                }
                _ => fans.light(cr, pal, x, y, &l.colours, l.range_nm),
            }
        }
        for f in feats {
            let Geom::Point(p) = &f.geom else { continue };
            if f.objl == obj::LIGHTS {
                continue;
            }
            if let Some(a) = aid_of(f, *p)
                && let Some(glyph) = declutter.glyph(a.kind, &a.pos)
            {
                let (x, y) = vi.to_screen(&a.pos);
                draw_aid(cr, pal, x, y, &a, glyph);
            }
        }
    }
    let _: &Palette = pal;
}
