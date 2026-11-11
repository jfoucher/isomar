//! Waypoints, routes, tracks, the active leg, the measuring line and the anchor circle, as the
//! overlay spec draws them.

use super::overlay::{LabelRequest, Place, fill_haloed, priority, stroke_haloed};
use super::{DrawCtx, Layer, font_size, text_scale};
use crate::app::{AppState, Selection};
use crate::core::geo::{LatLon, NM_M};
use crate::core::nav::calc::{self, format_range_bearing};
use crate::core::nav::model::{Route, Track};
use crate::tokens::{self, Token};
use std::f64::consts::PI;
use std::rc::{Rc, Weak};

pub struct NavLayer {
    app: Weak<AppState>,
}

impl NavLayer {
    pub fn new(app: &Rc<AppState>) -> NavLayer {
        NavLayer { app: Rc::downgrade(app) }
    }
}

/// How a route is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Inactive,
    Active,
    Editing,
}

fn polyline(ctx: &DrawCtx, pts: &[LatLon]) {
    super::polyline(ctx.cr, &ctx.vi, pts.iter().map(|p| ctx.vi.to_screen(p)));
}

fn draw_track(ctx: &DrawCtx, t: &Track, recording: bool) {
    if t.points.len() < 2 {
        return;
    }
    let pts: Vec<LatLon> = t.points.iter().map(|p| p.pos).collect();
    ctx.cr.set_line_join(cairo::LineJoin::Round);
    polyline(ctx, &pts);
    stroke_haloed(ctx.cr, &ctx.pal, Token::OwnShipTrail, if recording { 2.5 } else { 1.8 });
    ctx.cr.new_path();
}

/// The `+` handle on the middle of a leg being edited.
fn plus_handle(ctx: &DrawCtx, x: f64, y: f64) {
    let (cr, pal) = (ctx.cr, &ctx.pal);
    cr.new_path();
    cr.arc(x, y, 6.0, 0.0, 2.0 * PI);
    fill_haloed(cr, pal, Token::OnRoute);
    cr.new_path();
    cr.move_to(x - 3.0, y);
    cr.line_to(x + 3.0, y);
    cr.move_to(x, y - 3.0);
    cr.line_to(x, y + 3.0);
    pal.set(cr, Token::Route);
    cr.set_line_width(1.6);
    let _ = cr.stroke();
    cr.new_path();
}

/// A waypoint of a route: a ring, a filled number when it is the next one, a square handle when edited.
fn route_point(ctx: &DrawCtx, x: f64, y: f64, state: State, next: Option<usize>, selected: bool, hovered: bool) {
    let (cr, pal) = (ctx.cr, &ctx.pal);
    if hovered {
        cr.new_path();
        cr.arc(x, y, tokens::HIT_MIN / 2.0, 0.0, 2.0 * PI);
        pal.set(cr, Token::Hover);
        let _ = cr.fill();
    }
    cr.new_path();
    let r = match (state, next) {
        (State::Editing, _) => {
            cr.rectangle(x - 6.0, y - 6.0, 12.0, 12.0);
            8.5
        }
        (_, Some(_)) => {
            cr.arc(x, y, 9.0, 0.0, 2.0 * PI);
            11.0
        }
        _ => {
            cr.arc(x, y, 3.5, 0.0, 2.0 * PI);
            5.5
        }
    };
    if selected {
        cr.save().ok();
        cr.new_path();
        cr.arc(x, y, r + tokens::STROKE_HALO + 2.0, 0.0, 2.0 * PI);
        pal.set(cr, Token::Selection);
        cr.set_line_width(3.0);
        let _ = cr.stroke();
        cr.restore().ok();
        cr.new_path();
        match (state, next) {
            (State::Editing, _) => cr.rectangle(x - 6.0, y - 6.0, 12.0, 12.0),
            (_, Some(_)) => cr.arc(x, y, 9.0, 0.0, 2.0 * PI),
            _ => cr.arc(x, y, 3.5, 0.0, 2.0 * PI),
        }
    }
    match (state, next) {
        (State::Editing, _) => {
            fill_haloed(cr, pal, Token::OnRoute);
            pal.set(cr, Token::Route);
            cr.set_line_width(2.0);
            let _ = cr.stroke();
        }
        (_, Some(n)) => {
            fill_haloed(cr, pal, Token::Route);
            cr.new_path();
            // The number, centred.
            font_size(cr, 11.0);
            cr.select_font_face(crate::layers::FONT_FAMILY, cairo::FontSlant::Normal, cairo::FontWeight::Bold);
            let text = n.to_string();
            if let Ok(e) = cr.text_extents(&text) {
                cr.move_to(x - e.width() / 2.0 - e.x_bearing(), y - e.height() / 2.0 - e.y_bearing());
                pal.set(cr, Token::OnRoute);
                let _ = cr.show_text(&text);
            }
        }
        _ => {
            stroke_haloed(cr, pal, if state == State::Inactive { Token::RouteInactive } else { Token::Route }, 2.0);
        }
    }
    cr.new_path();
    ctx.overlay.reserve(x, y, r);
}

/// The distance in pixels from (`px`, `py`) to the segment a–b.
fn seg_distance(a: (f64, f64), b: (f64, f64), px: f64, py: f64) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (((px - a.0) * dx + (py - a.1) * dy) / len2).clamp(0.0, 1.0) };
    (px - (a.0 + t * dx)).hypot(py - (a.1 + t * dy))
}

/// The corridor `limit_nm` either side of the leg a→b, as a filled band between dashed edges.
fn draw_corridor(ctx: &DrawCtx, a: &LatLon, b: &LatLon, limit_nm: f64) {
    let (cr, pal, vi) = (ctx.cr, &ctx.pal, &ctx.vi);
    let brg = a.bearing_to(b);
    let edge = |p: &LatLon, side: f64| vi.to_screen(&p.destination(brg + side * 90.0, limit_nm));
    let (a1, b1, a2, b2) = (edge(a, 1.0), edge(b, 1.0), edge(a, -1.0), edge(b, -1.0));
    cr.new_path();
    cr.move_to(a1.0, a1.1);
    cr.line_to(b1.0, b1.1);
    cr.line_to(b2.0, b2.1);
    cr.line_to(a2.0, a2.1);
    cr.close_path();
    pal.set(cr, Token::RouteCorridor);
    let _ = cr.fill();
    cr.set_dash(&[6.0, 4.0], 0.0);
    cr.set_line_width(1.0);
    pal.set(cr, Token::Route);
    for (p, q) in [(a1, b1), (a2, b2)] {
        cr.new_path();
        cr.move_to(p.0, p.1);
        cr.line_to(q.0, q.1);
        let _ = cr.stroke();
    }
    cr.set_dash(&[], 0.0);
    cr.new_path();
}

fn draw_route(ctx: &DrawCtx, app: &AppState, r: &Route, state: State, target: Option<usize>) {
    if r.points.is_empty() {
        return;
    }
    let (cr, pal, vi) = (ctx.cr, &ctx.pal, &ctx.vi);
    let selected = app.selected.get();
    let route_selected = matches!(selected, Selection::Route(id) if id == r.id) || matches!(selected, Selection::RoutePoint { route, .. } if route == r.id);
    let pointer = app.pointer.get().map(|p| vi.to_screen(&p));
    let hit = app.canvas.hit_min() / 2.0;
    cr.set_line_join(cairo::LineJoin::Round);
    cr.set_line_cap(cairo::LineCap::Round);

    let (base, width) = match state {
        State::Inactive => (Token::RouteInactive, tokens::STROKE_ROUTE),
        State::Active => (Token::Route, tokens::STROKE_ROUTE_ACTIVE),
        State::Editing => (Token::Route, tokens::STROKE_ROUTE),
    };
    let screen: Vec<(f64, f64)> = r.points.iter().map(|p| vi.to_screen(&p.pos)).collect();

    // The selection ring goes outside the halo, under the line.
    if route_selected && r.points.len() > 1 {
        let pts: Vec<LatLon> = r.points.iter().map(|p| p.pos).collect();
        polyline(ctx, &pts);
        cr.set_line_width(width + 2.0 * (tokens::STROKE_HALO + 3.0));
        pal.set(cr, Token::Selection);
        let _ = cr.stroke();
        cr.new_path();
    }
    // Legs one at a time: those already sailed on an active route are dimmed.
    for (i, w) in r.points.windows(2).enumerate() {
        let past = state == State::Active && target.is_some_and(|t| i + 1 < t);
        super::polyline(cr, vi, [screen[i], screen[i + 1]]);
        if state == State::Inactive {
            cr.set_dash(&[6.0, 4.0], 0.0);
        }
        if past {
            cr.push_group();
            stroke_haloed(cr, pal, base, width);
            cr.new_path();
            let _ = cr.pop_group_to_source();
            let _ = cr.paint_with_alpha(0.4);
        } else {
            stroke_haloed(cr, pal, base, width);
        }
        cr.set_dash(&[], 0.0);
        cr.new_path();
        let _ = w;
    }

    // Leg chips: the active route's, or the leg under the pointer, when the leg is long enough.
    for (i, w) in r.points.windows(2).enumerate() {
        let (a, b) = (screen[i], screen[i + 1]);
        let hovered = pointer.is_some_and(|(px, py)| seg_distance(a, b, px, py) <= hit);
        if state != State::Active && !hovered {
            continue;
        }
        let (d, brg) = calc::leg(&w[0].pos, &w[1].pos);
        let text = format!("{:03.0}°\u{2002}{d:.1} nm", brg);
        font_size(cr, 10.5);
        let width_px = cr.text_extents(&text).map_or(60.0, |e| e.width());
        let len = (b.0 - a.0).hypot(b.1 - a.1);
        if len > width_px + 48.0 {
            ctx.overlay.request(
                LabelRequest::new(priority::ACTIVE_ROUTE, text, Place::Centre { x: (a.0 + b.0) / 2.0, y: (a.1 + b.1) / 2.0 }, Token::OnRoute).chip(Token::Route, Token::OnRoute),
            );
        }
    }

    // Points.
    let next = target.filter(|_| state == State::Active);
    for (i, p) in r.points.iter().enumerate() {
        let (x, y) = screen[i];
        let sel = matches!(selected, Selection::RoutePoint { route, idx } if route == r.id && idx == i);
        let hovered = pointer.is_some_and(|(px, py)| (px - x).hypot(py - y) <= hit);
        route_point(ctx, x, y, state, next.filter(|t| *t == i).map(|t| t + 1), sel, hovered);
        if !p.name.is_empty() && vi.zoom() >= 8.0 {
            let prio = if next == Some(i) { priority::ACTIVE_ROUTE } else { priority::WAYPOINT };
            ctx.overlay.request(LabelRequest::new(prio, p.name.clone(), Place::Around { x, y, r: 8.0 }, if state == State::Inactive { Token::RouteInactive } else { Token::Route }));
        }
    }
    if state == State::Editing {
        for w in screen.windows(2) {
            plus_handle(ctx, (w[0].0 + w[1].0) / 2.0, (w[0].1 + w[1].1) / 2.0);
        }
    }
    draw_route_name(ctx, r, if state == State::Inactive { Token::RouteInactive } else { Token::Route });
}

/// How far the route name keeps from the edges of the view, the leg's
/// middle (its chip) and the line, in pixels.
const NAME_MARGIN: f64 = 16.0;
const NAME_GAP: f64 = 7.0;

/// Writes the route's name along the longest leg on screen, upright and just
/// under the line, when a leg has room for it. Pango, since a name can be in
/// any script.
fn draw_route_name(ctx: &DrawCtx, r: &Route, colour: Token) {
    let name = r.name.trim();
    if name.is_empty() || r.points.len() < 2 {
        return;
    }
    let cr = ctx.cr;
    let layout = pangocairo::functions::create_layout(cr);
    let mut font = pango::FontDescription::new();
    font.set_family(crate::layers::FONT_FAMILY);
    font.set_weight(pango::Weight::Bold);
    font.set_absolute_size(11.0 * text_scale() as f64 / 100.0 * pango::SCALE as f64);
    layout.set_font_description(Some(&font));
    layout.set_text(name);
    crate::fonts::hold_layout(&layout);
    let (_, ink) = layout.pixel_extents();
    let w = ink.width() as f64;

    // The centre of the name along each leg, and how much room it has there.
    let mut best: Option<((f64, f64), (f64, f64), f64)> = None;
    for leg in r.points.windows(2) {
        let (a, b) = (ctx.vi.to_screen(&leg[0].pos), ctx.vi.to_screen(&leg[1].pos));
        let len = (b.0 - a.0).hypot(b.1 - a.1);
        let Some((s, e, from)) = super::clip_segment_to(&ctx.vi, a, b, -NAME_MARGIN) else { continue };
        let to = from + (e.0 - s.0).hypot(e.1 - s.1);
        // Keep off the middle of the leg: the longer side of the visible part.
        let half = len / 2.0;
        let (lo, hi) = if to <= half - NAME_MARGIN || from >= half + NAME_MARGIN {
            (from, to)
        } else if half - from > to - half {
            (from, half - NAME_MARGIN)
        } else {
            (half + NAME_MARGIN, to)
        };
        let room = hi - lo;
        if room >= w + 2.0 * NAME_MARGIN && best.is_none_or(|(_, _, r)| room > r) {
            let t = (lo + hi) / 2.0 / len;
            best = Some(((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t), (b.0 - a.0, b.1 - a.1), room));
        }
    }
    let Some(((x, y), (dx, dy), _)) = best else { return };
    let mut angle = dy.atan2(dx);
    if angle.abs() > PI / 2.0 {
        angle += PI;
    }
    let _ = cr.save();
    cr.translate(x, y);
    cr.rotate(angle);
    cr.new_path();
    cr.move_to(-w / 2.0 - ink.x() as f64, NAME_GAP - ink.y() as f64);
    pangocairo::functions::layout_path(cr, &layout);
    ctx.pal.set_alpha(cr, Token::OverlayHalo, 0.85);
    cr.set_line_width(2.0 * tokens::STROKE_HALO);
    cr.set_line_join(cairo::LineJoin::Round);
    let _ = cr.stroke_preserve();
    ctx.pal.set(cr, colour);
    let _ = cr.fill();
    cr.new_path();
    let _ = cr.restore();
}

/// A standalone mark: a route-coloured square with a dot.
fn draw_mark(ctx: &DrawCtx, x: f64, y: f64, selected: bool, hovered: bool) {
    let (cr, pal) = (ctx.cr, &ctx.pal);
    if hovered {
        cr.new_path();
        cr.arc(x, y, tokens::HIT_MIN / 2.0, 0.0, 2.0 * PI);
        pal.set(cr, Token::Hover);
        let _ = cr.fill();
    }
    let h = 5.0;
    if selected {
        cr.new_path();
        cr.rectangle(x - h - tokens::STROKE_HALO - 1.5, y - h - tokens::STROKE_HALO - 1.5, 2.0 * (h + tokens::STROKE_HALO + 1.5), 2.0 * (h + tokens::STROKE_HALO + 1.5));
        pal.set(cr, Token::Selection);
        cr.set_line_width(3.0);
        let _ = cr.stroke();
    }
    cr.new_path();
    cr.rectangle(x - h, y - h, 2.0 * h, 2.0 * h);
    fill_haloed(cr, pal, Token::Route);
    cr.new_path();
    cr.arc(x, y, 1.8, 0.0, 2.0 * PI);
    pal.set(cr, Token::OnRoute);
    let _ = cr.fill();
    cr.new_path();
    ctx.overlay.reserve(x, y, h + 2.0);
}

/// An anchor symbol: ring, shank, stock and flukes.
fn anchor_symbol(cr: &cairo::Context, x: f64, y: f64) {
    cr.new_path();
    cr.arc(x, y - 6.0, 2.5, 0.0, 2.0 * PI);
    cr.move_to(x, y - 3.5);
    cr.line_to(x, y + 7.0);
    cr.move_to(x - 4.0, y - 0.5);
    cr.line_to(x + 4.0, y - 0.5);
    cr.move_to(x - 6.0, y + 3.0);
    cr.arc(x, y + 2.0, 6.0, PI * 0.15, PI * 0.85);
}

fn draw_anchor(ctx: &DrawCtx, app: &AppState) {
    let s = app.session.borrow();
    let Some(w) = s.anchor.as_ref() else { return };
    let (cr, pal, vi) = (ctx.cr, &ctx.pal, &ctx.vi);
    let alarm = s.anchor_status.is_some_and(|a| a.alarm);
    let (x, y) = vi.to_screen(&w.center);
    let r = (w.radius_nm * NM_M / vi.metres_per_px()).max(6.0);
    // A faint trail inside the circle.
    let inside: Vec<(f64, f64)> = s.trail.iter().map(|(_, p)| *p).filter(|p| p.distance_nm(&w.center) <= w.radius_nm * 1.5).map(|p| vi.to_screen(&p)).collect();
    if inside.len() > 1 {
        super::polyline(cr, vi, inside);
        pal.set_alpha(cr, Token::AnchorCircle, 0.35);
        cr.set_line_width(1.5);
        let _ = cr.stroke();
    }
    cr.new_path();
    cr.arc(x, y, r, 0.0, 2.0 * PI);
    if alarm {
        stroke_haloed(cr, pal, Token::Alarm, 3.0);
    } else {
        cr.set_dash(&[8.0, 5.0], 0.0);
        stroke_haloed(cr, pal, Token::AnchorCircle, 2.0);
        cr.set_dash(&[], 0.0);
    }
    anchor_symbol(cr, x, y);
    stroke_haloed(cr, pal, Token::OwnShip, 1.8);
    cr.new_path();
    ctx.overlay.reserve(x, y, 8.0);
    if alarm && let Some(st) = s.anchor_status {
        ctx.overlay.request(LabelRequest::new(priority::ALARM, format!("{:.2} nm", st.distance_nm), Place::Around { x, y, r: 10.0 }, Token::Alarm).strong());
    }
}

/// "247° T  3.82 nm" (an en space between), with M for magnetic bearings.
pub fn measure_text(app: &AppState, from: &LatLon, to: &LatLon) -> String {
    let (d, mut brg) = (from.distance_nm(to), from.bearing_to(to));
    let settings = app.settings.borrow();
    let mut tag = "T";
    if settings.bearing_magnetic
        && let Some(v) = app.session.borrow().own.variation
    {
        brg = (brg - v).rem_euclid(360.0);
        tag = "M";
    }
    format!("{brg:03.0}° {tag}\u{2002}{d:.2} nm")
}

fn draw_measure(ctx: &DrawCtx, app: &AppState) {
    let Some(m) = app.measure.borrow().as_ref().map(|m| (m.from, m.to)) else { return };
    let (cr, pal, vi) = (ctx.cr, &ctx.pal, &ctx.vi);
    let end = m.1.or(app.pointer.get());
    let a = vi.to_screen(&m.0);
    let mut circle = vec![a];
    if let Some(to) = end {
        let b = vi.to_screen(&to);
        circle.push(b);
        cr.new_path();
        super::segment(cr, vi, a, b);
        stroke_haloed(cr, pal, Token::Measure, 2.0);
        cr.new_path();
        ctx.overlay.request(
            LabelRequest::new(priority::ALARM, measure_text(app, &m.0, &to), Place::Around { x: b.0, y: b.1, r: 8.0 }, Token::Text).chip(Token::CardBg, Token::Text),
        );
    }
    for (x, y) in circle {
        cr.new_path();
        cr.arc(x, y, 4.0, 0.0, 2.0 * PI);
        stroke_haloed(cr, pal, Token::Measure, 2.0);
        cr.new_path();
        ctx.overlay.reserve(x, y, 5.0);
    }
}

impl Layer for NavLayer {
    fn id(&self) -> &'static str {
        "nav"
    }

    fn is_dynamic(&self) -> bool {
        true
    }

    fn draw(&self, ctx: &DrawCtx) {
        let Some(app) = self.app.upgrade() else { return };
        let s = app.session.borrow();
        let edit = app.edit.borrow();
        let selected = app.selected.get();
        let (cr, vi, pal) = (ctx.cr, &ctx.vi, &ctx.pal);
        cr.select_font_face(crate::layers::FONT_FAMILY, cairo::FontSlant::Normal, cairo::FontWeight::Normal);

        let recording_id = s.tracks.last().map(|t| t.id).filter(|_| s.is_recording());
        for t in s.tracks.iter().filter(|t| t.visible) {
            draw_track(ctx, t, Some(t.id) == recording_id);
        }

        let active_id = s.active.as_ref().map(|a| a.route.id).filter(|id| *id != 0);
        let target = s.active.as_ref().map(|a| a.target);
        // A route being replayed is drawn by the replay, which would lie under it.
        let replayed = app.router.borrow().as_ref().and_then(|p| p.replay.as_ref().map(|r| r.route_id));
        for r in s.routes.iter().filter(|r| r.visible) {
            if edit.as_ref().is_some_and(|e| e.route.id == r.id) {
                continue;
            }
            let active = Some(r.id) == active_id;
            if Some(r.id) == replayed && !active {
                continue;
            }
            draw_route(ctx, &app, r, if active { State::Active } else { State::Inactive }, target.filter(|_| active));
        }
        // An active route that was never stored still draws.
        if active_id.is_none()
            && let Some(a) = s.active.as_ref()
        {
            draw_route(ctx, &app, &a.route, State::Active, Some(a.target));
        }
        if let Some(e) = edit.as_ref() {
            draw_route(ctx, &app, &e.route, State::Editing, None);
            // The rubber band from the last point to the pointer, with its readout.
            if let (Some(last), Some(p)) = (e.route.points.last(), app.pointer.get()) {
                let (a, b) = (vi.to_screen(&last.pos), vi.to_screen(&p));
                cr.new_path();
                let from = super::segment(cr, vi, a, b);
                cr.set_dash(&[6.0, 5.0], from);
                stroke_haloed(cr, pal, Token::Route, 2.0);
                cr.set_dash(&[], 0.0);
                cr.new_path();
                let (d, brg) = calc::leg(&last.pos, &p);
                ctx.overlay.request(
                    LabelRequest::new(priority::ALARM, format_range_bearing(d, brg), Place::Around { x: b.0, y: b.1, r: 8.0 }, Token::OnRoute).chip(Token::Route, Token::OnRoute),
                );
            }
        }

        // The active leg: the corridor, the line from the boat to the target and the arrival circle.
        if let Some(a) = s.active.as_ref() {
            let t = a.target_point();
            draw_corridor(ctx, &a.leg_start, &t.pos, app.settings.borrow().xte_limit_nm);
            let p2 = vi.to_screen(&t.pos);
            if let Some(pos) = s.own.pos {
                let p1 = vi.to_screen(&pos);
                cr.new_path();
                let from = super::segment(cr, vi, p1, p2);
                cr.set_dash(&[10.0, 6.0], from);
                stroke_haloed(cr, pal, Token::Route, 2.0);
                cr.set_dash(&[], 0.0);
                cr.new_path();
            }
            let r = (t.arrival_radius_nm * NM_M / vi.metres_per_px()).max(5.0);
            cr.new_path();
            cr.arc(p2.0, p2.1, r, 0.0, 2.0 * PI);
            cr.set_dash(&[5.0, 4.0], 0.0);
            stroke_haloed(cr, pal, Token::Route, 1.5);
            cr.set_dash(&[], 0.0);
            cr.new_path();
        }

        let pointer = app.pointer.get().map(|p| vi.to_screen(&p));
        let hit = app.canvas.hit_min() / 2.0;
        for w in s.waypoints.iter().filter(|w| w.visible) {
            let (x, y) = vi.to_screen(&w.pos);
            let sel = selected == Selection::Waypoint(w.id);
            if sel || vi.zoom() >= 14.0 {
                let r = w.arrival_radius_nm * NM_M / vi.metres_per_px();
                if r > 6.0 {
                    cr.new_path();
                    cr.arc(x, y, r, 0.0, 2.0 * PI);
                    cr.set_dash(&[5.0, 4.0], 0.0);
                    stroke_haloed(cr, pal, Token::Route, 1.0);
                    cr.set_dash(&[], 0.0);
                    cr.new_path();
                }
            }
            let hovered = pointer.is_some_and(|(px, py)| (px - x).hypot(py - y) <= hit);
            draw_mark(ctx, x, y, sel, hovered);
            if vi.zoom() >= 8.0 && !w.name.is_empty() {
                ctx.overlay.request(LabelRequest::new(if sel || hovered { priority::SELECTED } else { priority::WAYPOINT }, w.name.clone(), Place::Around { x, y, r: 7.0 }, Token::Route));
            }
        }

        draw_measure(ctx, &app);
        draw_anchor(ctx, &app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_to_a_segment_is_perpendicular_inside_and_to_the_end_outside() {
        let (a, b) = ((0.0, 0.0), (100.0, 0.0));
        assert_eq!(seg_distance(a, b, 50.0, 7.0), 7.0);
        assert_eq!(seg_distance(a, b, 130.0, 40.0), 50.0);
        assert_eq!(seg_distance(a, a, 3.0, 4.0), 5.0);
    }
}
