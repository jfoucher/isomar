//! Currents on the chart (`overlay-spec.md`, Weather): arrows the way the stream sets on a grid every
//! 48 points (half a step off the wind barbs'), in three weights by speed, the speed beside each from
//! zoom 12, and the current field when it is the one chosen to colour the sea. Drawn into the weather's
//! tile plane under the forecast, so playing through time redraws them with it.

use crate::app::AppState;
use crate::layers::overlay::stroke_haloed;
use crate::layers::{DrawCtx, Layer, TileArea, TilePaint};
use crate::palette::Palette;
use crate::tokens::{Rgba, Token};
use crate::weather::Fill;
use crate::core::currents::CurrentField;
use crate::core::geo::LatLon;
use chrono::{DateTime, Utc};
use std::rc::{Rc, Weak};
use std::sync::Arc;

pub const LAYER: &str = "currents";

/// Arrows every this many points, on the world's grid so neighbouring tiles agree.
const ARROW_STEP: f64 = 48.0;
/// Below this the water is slack: no arrow.
const SLACK_KN: f64 = 0.1;
/// From this zoom the speed is written beside each arrow.
const NUMBERS_ZOOM: f64 = 12.0;

/// The current-speed ramp: knots and the token of its colour.
const CURRENT_STOPS: [(f32, Token); 6] = [(0.0, Token::Current0), (0.5, Token::Current05), (1.0, Token::Current1), (2.0, Token::Current2), (3.0, Token::Current3), (4.0, Token::Current4)];

pub fn current_colour(pal: &Palette, kn: f32) -> Rgba {
    crate::weather::layer::ramp(pal, &CURRENT_STOPS, kn)
}

/// An arrow's width and length for a speed (the spec's three weights).
pub fn weight(kn: f64) -> (f64, f64) {
    if kn < 1.0 {
        (1.0, 14.0)
    } else if kn < 3.0 {
        (1.5, 22.0)
    } else {
        (2.5, 30.0)
    }
}

pub struct CurrentLayer {
    app: Weak<AppState>,
}

impl CurrentLayer {
    pub fn new(app: &Rc<AppState>) -> CurrentLayer {
        CurrentLayer { app: Rc::downgrade(app) }
    }
}

struct CurrentPaint {
    field: Arc<CurrentField>,
    time: DateTime<Utc>,
    arrows: bool,
    fill: bool,
}

impl Layer for CurrentLayer {
    fn id(&self) -> &'static str {
        LAYER
    }

    fn plane(&self) -> u8 {
        crate::layers::WEATHER_PLANE
    }

    fn revision(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let Some(app) = self.app.upgrade() else { return 0 };
        let w = app.weather.borrow();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (w.time.timestamp_millis(), w.fill == Fill::Current, w.currents.arrows, w.currents.revision).hash(&mut h);
        h.finish()
    }

    fn prepare(&self, _area: &TileArea) -> Option<Arc<dyn TilePaint>> {
        let app = self.app.upgrade()?;
        let w = app.weather.borrow();
        let c = &w.currents;
        let fill = w.fill == Fill::Current;
        let drawn = !c.is_empty() && (c.arrows || fill);
        if let Some(credit) = app.credit.borrow().as_ref() {
            *credit.currents.borrow_mut() = if drawn { c.credits() } else { Vec::new() };
        }
        if !drawn {
            return None;
        }
        Some(Arc::new(CurrentPaint { field: c.field(), time: w.time, arrows: c.arrows, fill }))
    }
}

impl TilePaint for CurrentPaint {
    fn paint(&self, ctx: &DrawCtx) {
        let (cr, vi, pal) = (ctx.cr, &ctx.vi, &ctx.pal);
        let (x0, y0, x1, y1) = cr.clip_extents().unwrap_or((0.0, 0.0, vi.w, vi.h));
        let area = [x0, y0, x1, y1];
        let (f, t) = (&self.field, self.time);
        if self.fill {
            crate::weather::layer::shade(ctx, area, false, |lat, lon| f.sample(&LatLon::new(lat, lon), t).map(|(e, n)| current_colour(pal, e.hypot(n) as f32)));
        }
        if !self.arrows {
            return;
        }
        let numbers = vi.zoom() >= NUMBERS_ZOOM;
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_line_join(cairo::LineJoin::Round);
        cr.select_font_face(crate::layers::FONT_FAMILY, cairo::FontSlant::Normal, cairo::FontWeight::Normal);
        crate::layers::font_size(cr, 10.5);
        for (x, y) in crate::weather::layer::world_grid(vi, ARROW_STEP, ARROW_STEP, area, 40.0) {
            let p = vi.position_at(x, y);
            let Some((e, n)) = f.sample(&p, t) else { continue };
            let kn = e.hypot(n);
            if kn < SLACK_KN {
                continue;
            }
            let (width, len) = weight(kn);
            let set = crate::core::geo::set_of(e, n).to_radians();
            let (dx, dy) = (set.sin(), -set.cos());
            // Centred on its grid point: the tail half a length behind, the head half ahead.
            let (tx, ty) = (x - dx * len / 2.0, y - dy * len / 2.0);
            let (hx, hy) = (x + dx * len / 2.0, y + dy * len / 2.0);
            cr.new_path();
            cr.move_to(tx, ty);
            cr.line_to(hx, hy);
            let head = 4.0 + width * 1.5;
            for s in [-1.0, 1.0] {
                let b = set + std::f64::consts::PI + s * 0.5;
                cr.move_to(hx, hy);
                cr.line_to(hx + b.sin() * head, hy - b.cos() * head);
            }
            stroke_haloed(cr, pal, Token::CurrentArrow, width);
            cr.new_path();
            if numbers {
                // At the tail, on the side away from the head.
                let text = format!("{kn:.1}");
                if let Ok(ext) = cr.text_extents(&text) {
                    let (lx, ly) = (tx - dx * (ext.width() / 2.0 + 4.0) - ext.width() / 2.0, ty - dy * (ext.height() / 2.0 + 4.0) + ext.height() / 2.0);
                    crate::layers::halo_text(cr, pal, lx, ly, &text, Token::CurrentArrow);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faster_streams_get_heavier_longer_arrows() {
        let (a, b, c) = (weight(0.5), weight(2.0), weight(4.5));
        assert!(a.0 < b.0 && b.0 < c.0 && a.1 < b.1 && b.1 < c.1);
    }
}
