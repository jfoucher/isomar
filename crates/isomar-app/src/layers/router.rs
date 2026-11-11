//! The weather router's search while it is being previewed: the fronts grown
//! so far, each alternative's track and a boat on it, all up to the playback
//! clock.

use super::overlay::{LabelRequest, Place, priority, stroke_haloed};
use super::{DrawCtx, Layer, font_size};
use crate::core::geo::LatLon;
use crate::i18n::gettext_f;
use crate::app::AppState;
use crate::tokens::{self, Theme, Token};
use crate::router::PICK_TOLERANCE;
use chrono::Local;
use std::f64::consts::PI;
use std::rc::{Rc, Weak};

/// One colour per alternative, and a dash pattern as well, so that they are told apart without colour.
pub const ALT_COLOURS: [Token; 6] = [Token::Alt1, Token::Alt2, Token::Alt3, Token::Alt4, Token::Alt5, Token::Alt6];
pub const ALT_DASHES: [&[f64]; 6] = [&[], &[9.0, 4.0], &[2.0, 4.0], &[10.0, 3.0, 2.0, 3.0], &[5.0, 5.0], &[1.0, 6.0]];

/// The colour of alternative `i` as `#rrggbb`, in the day theme, for the card's swatches.
pub fn alt_hex(i: usize) -> String {
    let (r, g, b, _) = tokens::color(ALT_COLOURS[i % ALT_COLOURS.len()], Theme::Day);
    format!("#{:02x}{:02x}{:02x}", (r * 255.0).round() as u8, (g * 255.0).round() as u8, (b * 255.0).round() as u8)
}

pub struct RouterLayer {
    app: Weak<AppState>,
}

impl RouterLayer {
    pub fn new(app: &Rc<AppState>) -> RouterLayer {
        RouterLayer { app: Rc::downgrade(app) }
    }
}

fn polyline(ctx: &DrawCtx, pts: impl Iterator<Item = LatLon>) {
    super::polyline(ctx.cr, &ctx.vi, pts.map(|p| ctx.vi.to_screen(&p)));
}

/// The fronts of a search still running: the same lines as in the preview,
/// the latest heavy, thinned when there are many.
fn draw_live(ctx: &DrawCtx, live: &crate::core::routing::isochrone::Progress) {
    let (cr, pal) = (ctx.cr, &ctx.pal);
    cr.set_line_join(cairo::LineJoin::Round);
    cr.set_line_cap(cairo::LineCap::Round);
    let fronts = live.fronts.lock().unwrap();
    let shown = fronts.len();
    let stride = (shown / 120).max(1);
    for k in (0..shown).filter(|k| k % stride == 0 || k + 1 == shown) {
        let now = k + 1 == shown;
        for run in &fronts[k] {
            match run.as_slice() {
                [only] => {
                    let (x, y) = ctx.vi.to_screen(only);
                    cr.new_sub_path();
                    cr.arc(x, y, 0.8, 0.0, 2.0 * PI);
                }
                _ => polyline(ctx, run.iter().copied()),
            }
        }
        if now {
            stroke_haloed(cr, pal, Token::IsochroneNow, tokens::STROKE_FRONT_NOW);
        } else {
            pal.set(cr, Token::Isochrone);
            cr.set_line_width(tokens::STROKE_FRONT);
            let _ = cr.stroke_preserve();
        }
        cr.new_path();
    }
}

impl Layer for RouterLayer {
    fn id(&self) -> &'static str {
        "router"
    }

    fn is_dynamic(&self) -> bool {
        true
    }

    fn draw(&self, ctx: &DrawCtx) {
        let Some(app) = self.app.upgrade() else { return };
        if let Some(live) = app.router_live.borrow().as_ref() {
            draw_live(ctx, live);
            return;
        }
        let preview = app.router.borrow();
        let Some(p) = preview.as_ref() else { return };
        let (cr, pal, ex) = (ctx.cr, &ctx.pal, &p.ex);
        cr.set_line_join(cairo::LineJoin::Round);
        cr.set_line_cap(cairo::LineCap::Round);

        // The fronts reached by the clock, thinned when there are many. The
        // latest is the one to click, so it is drawn heavier.
        let shown = p.fronts_shown();
        let stride = (shown / 120).max(1);
        for k in (0..shown).filter(|k| k % stride == 0 || k + 1 == shown) {
            let last_of_forecast = ex.horizon.is_some() && k + 1 == ex.fronts.len();
            let now = k + 1 == shown;
            let (tok, width) = if last_of_forecast {
                (Token::Warning, 2.5)
            } else if now {
                (Token::IsochroneNow, tokens::STROKE_FRONT_NOW)
            } else {
                (Token::Isochrone, tokens::STROKE_FRONT)
            };
            // Not across gaps or land, where neighbours in direction are not
            // neighbours on the water. A position with nothing joined to it is a dot.
            for run in ex.front_runs(k) {
                match run {
                    [only] => {
                        let (x, y) = ctx.vi.to_screen(&ex.tree[*only].node.pos);
                        cr.new_sub_path();
                        cr.arc(x, y, 0.8, 0.0, 2.0 * PI);
                    }
                    _ => polyline(ctx, run.iter().map(|&i| ex.tree[i].node.pos)),
                }
            }
            if now || last_of_forecast {
                stroke_haloed(cr, pal, tok, width);
            } else {
                pal.set(cr, tok);
                cr.set_line_width(width);
                let _ = cr.stroke_preserve();
            }
            cr.new_path();
        }

        // Where a click on the front would end, under the pointer.
        if let Some(pointer) = app.pointer.get() {
            let (px, py) = ctx.vi.to_screen(&pointer);
            let project = |q: &LatLon| ctx.vi.to_screen(q);
            if let Some(i) = p.front_hit(&project, px, py, PICK_TOLERANCE) {
                let (x, y) = ctx.vi.to_screen(&ex.tree[i].node.pos);
                cr.new_path();
                cr.arc(x, y, 7.0, 0.0, 2.0 * PI);
                pal.set(cr, Token::Selection);
                cr.set_line_width(2.0);
                let _ = cr.stroke();
            }
        }

        // A saved route being replayed lies beneath the replay: a wider line and its waypoints, so
        // where the two part company shows.
        if let Some(r) = p.replay.as_ref() {
            let saved = app.session.borrow().routes.iter().find(|x| x.id == r.route_id).map(|x| x.points.iter().map(|q| q.pos).collect::<Vec<_>>());
            if let Some(points) = saved.filter(|q| q.len() >= 2) {
                // In the selection colour, a band wider than the replay line, so the two read apart.
                polyline(ctx, points.iter().copied());
                stroke_haloed(cr, pal, Token::Selection, tokens::STROKE_ROUTE + 6.0);
                cr.new_path();
                for q in &points {
                    let (x, y) = ctx.vi.to_screen(q);
                    cr.arc(x, y, 3.5, 0.0, 2.0 * PI);
                    super::overlay::fill_haloed(cr, pal, Token::Route);
                    cr.new_path();
                }
            }
        }

        // The other forecast models' best ways, in their colours, beneath the shown model's, each with
        // its boat at the clock and its name: where they part is what the comparison shows.
        for (k, run) in p.runs.iter().enumerate().filter(|(_, r)| r.ex.is_some()) {
            let a = &run.best;
            let colour = ALT_COLOURS[k % ALT_COLOURS.len()];
            for (motoring, legs) in a.track_legs_until(p.t) {
                polyline(ctx, legs.iter().copied());
                stroke_haloed(cr, pal, if motoring { Token::RouterMotor } else { colour }, tokens::STROKE_ROUTE - 0.5);
                cr.new_path();
            }
            let track = a.track_until(p.t);
            let (x, y) = ctx.vi.to_screen(&a.position_at(p.t));
            let heading = match track.as_slice() {
                [.., prev, last] if prev != last => prev.bearing_to(last),
                _ => 0.0,
            };
            let _ = cr.save();
            cr.translate(x, y);
            cr.rotate(heading.to_radians());
            super::boat::hull_path(cr, 13.0, 5.0);
            super::overlay::fill_haloed(cr, pal, colour);
            cr.new_path();
            let _ = cr.restore();
            ctx.overlay.reserve(x, y, 7.0);
            ctx.overlay.request(LabelRequest::new(priority::SELECTED, run.model.label().to_string(), Place::Around { x, y, r: 9.0 }, colour));
        }

        // Alternatives, the selected one last so it lies on top.
        let order = (0..p.alts.len()).filter(|&i| i != p.selected).chain((p.selected < p.alts.len()).then_some(p.selected));
        for i in order {
            let c = &p.alts[i];
            let a = &c.alt;
            let colour = ALT_COLOURS[c.colour % ALT_COLOURS.len()];
            let dash = ALT_DASHES[c.colour % ALT_DASHES.len()];
            let chosen = i == p.selected;
            let track = a.track_until(p.t);
            // The stretches under engine in their own colour, with the alternative's dash, so the
            // alternatives are still told apart.
            for (motoring, run) in a.track_legs_until(p.t) {
                cr.set_dash(dash, 0.0);
                polyline(ctx, run.iter().copied());
                stroke_haloed(cr, pal, if motoring { Token::RouterMotor } else { colour }, if chosen { tokens::STROKE_ROUTE + 1.0 } else { tokens::STROKE_ROUTE });
                cr.set_dash(&[], 0.0);
                cr.new_path();
            }
            // Where the boat waits for the stream to turn: a ring, and how long.
            if chosen {
                for (at, from, until) in holds(&a.path).into_iter().filter(|h| h.1 <= p.t) {
                    let (hx, hy) = ctx.vi.to_screen(&at);
                    cr.new_path();
                    cr.arc(hx, hy, 8.0, 0.0, std::f64::consts::TAU);
                    stroke_haloed(cr, pal, colour, tokens::STROKE_ROUTE);
                    cr.new_path();
                    ctx.overlay.reserve(hx, hy, 10.0);
                    let text = gettext_f("waits {dur} for the stream", &[("dur", &crate::core::nav::calc::format_duration(until - from))]);
                    ctx.overlay.request(LabelRequest::new(priority::SELECTED, text, Place::Around { x: hx, y: hy, r: 11.0 }, colour));
                }
            }
            // A small hull at the clock, to the heading of the track.
            let (x, y) = ctx.vi.to_screen(&a.position_at(p.t));
            let heading = match track.as_slice() {
                [.., prev, last] if prev != last => prev.bearing_to(last),
                _ => 0.0,
            };
            let _ = cr.save();
            cr.translate(x, y);
            cr.rotate(heading.to_radians());
            super::boat::hull_path(cr, if chosen { 18.0 } else { 14.0 }, if chosen { 7.0 } else { 5.5 });
            super::overlay::fill_haloed(cr, pal, Token::RouterBoat);
            cr.new_path();
            let _ = cr.restore();
            ctx.overlay.reserve(x, y, 8.0);
            if p.t >= a.end {
                ctx.overlay.request(LabelRequest::new(priority::SELECTED, (i + 1).to_string(), Place::Around { x, y, r: 9.0 }, colour).strong());
            }
            // From where the forecast ends, a straight line the rest of the way, in the stale colour.
            if chosen && !a.arrives && p.t >= a.end {
                let (dx, dy) = ctx.vi.to_screen(&ex.dest);
                cr.new_path();
                cr.move_to(x, y);
                cr.line_to(dx, dy);
                cr.set_dash(&[6.0, 5.0], 0.0);
                stroke_haloed(cr, pal, Token::Stale, tokens::STROKE_ROUTE);
                cr.set_dash(&[], 0.0);
                cr.new_path();
                if let Some(h) = ex.horizon.filter(|h| a.end >= *h) {
                    let time = h.with_timezone(&Local).format("%a %H:%M").to_string();
                    let text = if ex.stalled {
                        gettext_f("No way forward after {time}", &[("time", &time)])
                    } else {
                        gettext_f("Forecast ends {time}", &[("time", &time)])
                    };
                    ctx.overlay.request(LabelRequest::new(priority::ACTIVE_ROUTE, text, Place::Around { x, y, r: 12.0 }, Token::Stale));
                }
            }
        }

        // The destination: a route ring with a flag.
        let (dx, dy) = ctx.vi.to_screen(&ex.dest);
        cr.new_path();
        cr.arc(dx, dy, 6.0, 0.0, 2.0 * PI);
        stroke_haloed(cr, pal, Token::Route, 2.0);
        cr.new_path();
        cr.move_to(dx, dy - 6.0);
        cr.line_to(dx, dy - 20.0);
        stroke_haloed(cr, pal, Token::Route, 1.5);
        cr.new_path();
        cr.move_to(dx, dy - 20.0);
        cr.line_to(dx + 11.0, dy - 16.0);
        cr.line_to(dx, dy - 12.0);
        cr.close_path();
        super::overlay::fill_haloed(cr, pal, Token::Route);
        cr.new_path();
        ctx.overlay.reserve(dx, dy - 8.0, 14.0);
        let _ = font_size;
    }
}

/// The waits along a path (`Node::holding`): where, and from when until when, a run of holds at a time.
pub fn holds(path: &[crate::core::routing::isochrone::Node]) -> Vec<(crate::core::geo::LatLon, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)> {
    let mut out: Vec<(crate::core::geo::LatLon, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)> = Vec::new();
    for (i, n) in path.iter().enumerate().skip(1) {
        if !n.holding() {
            continue;
        }
        match out.last_mut() {
            Some(last) if path[i - 1].holding() && i > 1 => last.2 = n.time,
            _ => out.push((n.pos, path[i - 1].time, n.time)),
        }
    }
    out
}

