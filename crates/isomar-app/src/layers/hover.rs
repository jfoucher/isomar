//! What the pointer is over, outlined (`probe`): the area, line or point a click would open a popover for, and,
//! while the popover is open, the one it is about. An area is tinted inside, where a click opens it (a fairway
//! only along its edge, since only its edge does), with a soft band along its edge round the dashes the chart
//! draws; a line gets the band; a point a ring. An area's own marking is drawn at its centre, even where the chart
//! left it out for lack of room.

use super::DrawCtx;
use super::Layer;
use super::enc::trace;
use super::overlay::stroke_haloed;
use crate::app::AppState;
use crate::core::geo::LatLon;
use crate::palette::MAGENTA;
use crate::probe::{Outline, Shape};
use crate::tokens::Token;
use std::f64::consts::TAU;
use std::rc::{Rc, Weak};

/// How strongly the inside of an area is tinted.
const TINT_ALPHA: f64 = 0.14;
/// The band along an edge or a line: its width and strength.
const BAND_WIDTH: f64 = 8.0;
const BAND_ALPHA: f64 = 0.28;
/// The ring round a point, in pixels.
const RING_RADIUS: f64 = 12.0;
/// The disc behind an area's marking.
const CENTRE_RADIUS: f64 = 11.0;

pub struct HoverLayer {
    app: Weak<AppState>,
}

impl HoverLayer {
    pub fn new(app: &Rc<AppState>) -> HoverLayer {
        HoverLayer { app: Rc::downgrade(app) }
    }
}

impl Layer for HoverLayer {
    fn id(&self) -> &'static str {
        "hover"
    }

    fn is_dynamic(&self) -> bool {
        true
    }

    fn draw(&self, ctx: &DrawCtx) {
        let Some(app) = self.app.upgrade() else { return };
        let pinned = app.probe.pinned();
        if let Some(o) = &pinned {
            draw_outline(ctx, o);
        }
        if let Some(o) = app.probe.hover().filter(|h| pinned.as_ref() != Some(h)) {
            draw_outline(ctx, &o);
        }
    }
}

fn draw_outline(ctx: &DrawCtx, o: &Outline) {
    let (cr, pal, vi) = (ctx.cr, &ctx.pal, &ctx.vi);
    let _ = cr.save();
    cr.set_line_join(cairo::LineJoin::Round);
    cr.set_line_cap(cairo::LineCap::Round);
    match &o.shape {
        Shape::Point(p) => {
            let (x, y) = vi.to_screen(&LatLon::new(p[1], p[0]));
            cr.new_path();
            cr.arc(x, y, RING_RADIUS, 0.0, TAU);
            stroke_haloed(cr, pal, Token::Selection, 2.0);
            cr.new_path();
        }
        Shape::Line(line) => {
            cr.new_path();
            trace(vi, cr, line, false);
            pal.set_alpha(cr, MAGENTA, BAND_ALPHA);
            cr.set_line_width(BAND_WIDTH);
            let _ = cr.stroke();
        }
        Shape::Area(rings) => {
            cr.new_path();
            for r in rings {
                trace(vi, cr, r, true);
            }
            if o.inside {
                cr.set_fill_rule(cairo::FillRule::EvenOdd);
                pal.set_alpha(cr, MAGENTA, TINT_ALPHA);
                let _ = cr.fill_preserve();
            }
            pal.set_alpha(cr, MAGENTA, BAND_ALPHA);
            cr.set_line_width(BAND_WIDTH);
            let _ = cr.stroke();
            // The marking the chart gives the area, redrawn at its centre even where the chart left it out for
            // lack of room. An area with none (an anchorage, whose anchor the chart draws there) gets nothing
            // over its centre.
            if o.inside
                && let (Some(m), Some(c)) = (o.marker, o.centre())
            {
                let (x, y) = vi.to_screen(&LatLon::new(c[1], c[0]));
                if (0.0..vi.w).contains(&x) && (0.0..vi.h).contains(&y) {
                    cr.new_path();
                    cr.arc(x, y, CENTRE_RADIUS, 0.0, TAU);
                    pal.set_alpha(cr, Token::OverlayHalo, 0.8);
                    let _ = cr.fill();
                    super::chartinfo::marker(cr, pal, x, y, m);
                }
            }
        }
    }
    let _ = cr.restore();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::{Mode, Palette};
    use crate::view::{View, ViewInfo};

    /// How many pixels of a 200 by 200 white image `o` changes.
    fn inked(o: &Outline) -> usize {
        let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 200, 200).unwrap();
        let cr = cairo::Context::new(&surface).unwrap();
        cr.set_source_rgb(1.0, 1.0, 1.0);
        let _ = cr.paint();
        let vi = ViewInfo { view: View { center: LatLon::new(48.0, -4.0), zoom: 14.0 }, w: 200.0, h: 200.0 };
        let overlay = super::super::overlay::Overlay::default();
        draw_outline(&DrawCtx { cr: &cr, vi, pal: Palette::new(Mode::Day, false), overlay: &overlay, reduce_motion: false, top_inset: 0.0 }, o);
        drop(cr);
        surface.flush();
        let data = surface.data().unwrap();
        data.chunks(4).filter(|p| p[..3] != [255, 255, 255]).count()
    }

    fn square(d: f64) -> Vec<Vec<[f64; 2]>> {
        vec![vec![[-4.0 - d, 48.0 - d], [-4.0 + d, 48.0 - d], [-4.0 + d, 48.0 + d], [-4.0 - d, 48.0 + d], [-4.0 - d, 48.0 - d]]]
    }

    #[test]
    fn a_point_a_line_and_an_area_are_each_outlined_and_only_a_clickable_inside_is_tinted() {
        let point = Outline::of_feature(&feature(crate::core::chart::s57::obj::WRECKS, crate::core::chart::s57::Geom::Point([-4.0, 48.0]))).unwrap();
        assert!(inked(&point) > 50, "a ring");
        let line = Outline::of_feature(&feature(crate::core::chart::s57::obj::CBLSUB, crate::core::chart::s57::Geom::Line(vec![[-4.001, 48.0], [-3.999, 48.0]]))).unwrap();
        assert!(inked(&line) > 100, "a band");
        let caution = Outline::of_feature(&feature(crate::core::chart::s57::obj::CTNARE, crate::core::chart::s57::Geom::Area(square(0.002)))).unwrap();
        let fairway = Outline::of_feature(&feature(crate::core::chart::s57::obj::FAIRWY, crate::core::chart::s57::Geom::Area(square(0.002)))).unwrap();
        assert!(caution.inside && !fairway.inside);
        let (tinted, edge) = (inked(&caution), inked(&fairway));
        assert!(tinted > edge + 1000, "the inside of {tinted} px against {edge}");
    }

    fn feature(objl: u16, geom: crate::core::chart::s57::Geom) -> crate::core::chart::s57::Feature {
        crate::core::chart::s57::Feature { rcid: 1, objl, attrs: Default::default(), geom }
    }
}
