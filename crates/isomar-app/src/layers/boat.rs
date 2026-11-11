//! Own ship and AIS targets, as the overlay spec draws them.

use super::overlay::{LabelRequest, Place, blink_on, fill_haloed, priority, stroke_haloed};
use super::{DrawCtx, Layer};
use crate::app::{AppState, Selection};
use crate::core::comm::ais::AisClass;
use crate::core::comm::cpa::{AisTarget, Motion, cpa};
use crate::core::geo::NM_M;
use crate::tokens::{self, MM_PX, Token};
use chrono::{DateTime, Duration, Utc};
use std::f64::consts::PI;
use std::rc::{Rc, Weak};

/// The fix is stale after this and lost after twice as long: the hull is outlined and dashed.
const STALE_AFTER: Duration = Duration::seconds(30);
const LOST_AFTER: Duration = Duration::minutes(2);
/// An AIS target not heard from for this long is drawn as lost.
const TARGET_LOST_AFTER: Duration = Duration::minutes(3);
/// How strongly the disc of the fix's error is tinted inside its edge.
const ERROR_FILL_ALPHA: f64 = 0.1;

pub struct BoatLayer {
    app: Weak<AppState>,
}

impl BoatLayer {
    pub fn new(app: &Rc<AppState>) -> BoatLayer {
        BoatLayer { app: Rc::downgrade(app) }
    }
}

/// Length in pixels of a speed vector for `sog_kn` over `minutes`.
pub fn vector_px(sog_kn: f64, minutes: f64, metres_per_px: f64) -> f64 {
    sog_kn * minutes / 60.0 * NM_M / metres_per_px
}

/// What the age of the fix means for how the boat is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fix {
    Good,
    Stale,
    Lost,
}

pub fn fix_state(updated: Option<DateTime<Utc>>, now: DateTime<Utc>) -> Fix {
    match updated.map(|t| now - t) {
        Some(age) if age <= STALE_AFTER => Fix::Good,
        Some(age) if age <= LOST_AFTER => Fix::Stale,
        _ => Fix::Lost,
    }
}

/// "Fix 2 min old", for the status line.
pub fn fix_age_text(updated: Option<DateTime<Utc>>, now: DateTime<Utc>) -> Option<String> {
    let age = now - updated?;
    if age <= STALE_AFTER {
        return None;
    }
    let text = if age < Duration::minutes(2) { format!("{} s", age.num_seconds()) } else { format!("{} min", age.num_minutes()) };
    Some(crate::i18n::gettext_f("Fix {age} old", &[("age", &text)]))
}

fn ray(cr: &cairo::Context, x: f64, y: f64, bearing: f64, from: f64, to: f64) {
    let (s, c) = bearing.to_radians().sin_cos();
    cr.move_to(x + from * s, y - from * c);
    cr.line_to(x + to * s, y - to * c);
}

/// The hull outline of a boat `len` long and `beam` wide, bow up, centred on the origin.
pub(super) fn hull_path(cr: &cairo::Context, len: f64, beam: f64) {
    let (h, w) = (len / 2.0, beam / 2.0);
    cr.new_path();
    cr.move_to(0.0, -h);
    cr.curve_to(w * 0.55, -h * 0.55, w, -h * 0.1, w, h * 0.35);
    cr.line_to(w * 0.75, h);
    cr.line_to(-w * 0.75, h);
    cr.line_to(-w, h * 0.35);
    cr.curve_to(-w, -h * 0.1, -w * 0.55, -h * 0.55, 0.0, -h);
    cr.close_path();
}

fn draw_own(ctx: &DrawCtx, app: &AppState, now: DateTime<Utc>) {
    let s = app.session.borrow();
    let Some(pos) = s.own.pos else { return };
    let (cr, vi, pal) = (ctx.cr, &ctx.vi, &ctx.pal);
    let (x, y) = vi.to_screen(&pos);
    let mpp = vi.metres_per_px();
    let fix = fix_state(s.own.pos_updated, now);
    let settings = app.settings.borrow();
    let moving = s.own.sog_kn.filter(|v| *v > 0.2).zip(s.own.cog);

    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);

    // The trail: dotted, the last two hours.
    if s.trail.len() > 1 {
        cr.set_dash(&[1.5, 3.0], 0.0);
        super::polyline(cr, vi, s.trail.iter().map(|(_, p)| vi.to_screen(p)));
        pal.set(cr, Token::OwnShipTrail);
        cr.set_line_width(1.5);
        let _ = cr.stroke();
        cr.set_dash(&[], 0.0);
    }

    // How far off the fix may be: a faint disc, 95% radius, once it shows round the hull.
    let symbol = tokens::OWNSHIP_LENGTH * MM_PX;
    let true_len = settings.boat_length_m / mpp;
    let (len, beam) = if true_len > symbol { (true_len, settings.boat_beam_m / mpp) } else { (symbol, symbol * 0.38) };
    if fix == Fix::Good
        && let Some(r) = s.own.error_radius_m(now, STALE_AFTER).map(|m| m / mpp)
        && r > len / 2.0
    {
        cr.new_path();
        cr.arc(x, y, r, 0.0, 2.0 * PI);
        pal.set_alpha(cr, Token::OwnShip, ERROR_FILL_ALPHA);
        let _ = cr.fill_preserve();
        stroke_haloed(cr, pal, Token::OwnShip, 1.0);
        cr.new_path();
    }

    // The heading line to the edge of the view.
    if fix == Fix::Good
        && let Some(h) = s.own.heading
    {
        cr.new_path();
        ray(cr, x, y, h, 0.0, vi.w.hypot(vi.h));
        stroke_haloed(cr, pal, Token::OwnShip, 1.5);
        cr.new_path();
    }

    // The COG vector: a tick each minute and an open arrowhead.
    if fix == Fix::Good
        && let Some((sog, cog)) = moving
    {
        let minutes = settings.vector_minutes.max(1.0);
        let len = vector_px(sog, minutes, mpp).min(3000.0);
        cr.new_path();
        ray(cr, x, y, cog, 0.0, len);
        stroke_haloed(cr, pal, Token::OwnShip, tokens::STROKE_VECTOR);
        cr.new_path();
        let per_minute = len / minutes;
        if per_minute >= 6.0 {
            for m in 1..=minutes as i32 {
                let d = per_minute * m as f64;
                let (s, c) = (cog + 90.0).to_radians().sin_cos();
                let (cx, cy) = (x + d * cog.to_radians().sin(), y - d * cog.to_radians().cos());
                cr.move_to(cx - 3.5 * s, cy + 3.5 * c);
                cr.line_to(cx + 3.5 * s, cy - 3.5 * c);
            }
            stroke_haloed(cr, pal, Token::OwnShip, tokens::STROKE_VECTOR * 0.75);
            cr.new_path();
        }
        // Open arrowhead.
        let (tx, ty) = (x + len * cog.to_radians().sin(), y - len * cog.to_radians().cos());
        for side in [-1.0, 1.0] {
            let (s, c) = (cog + 180.0 + side * 28.0).to_radians().sin_cos();
            cr.move_to(tx, ty);
            cr.line_to(tx + 9.0 * s, ty - 9.0 * c);
        }
        stroke_haloed(cr, pal, Token::OwnShip, tokens::STROKE_VECTOR);
        cr.new_path();
    }

    // The hull, to scale once the boat is bigger than the symbol.
    let _ = cr.save();
    cr.translate(x, y);
    match s.own.heading.or(s.own.cog) {
        Some(h) => cr.rotate(h.to_radians()),
        None => cr.rotate(0.0),
    }
    hull_path(cr, len, beam);
    if fix == Fix::Good {
        fill_haloed(cr, pal, Token::OwnShip);
        cr.new_path();
    } else {
        cr.set_dash(&[4.0, 3.0], 0.0);
        stroke_haloed(cr, pal, Token::Stale, 1.8);
        cr.new_path();
    }
    let _ = cr.restore();
    ctx.overlay.reserve(x, y, len.max(beam) / 2.0);

    if fix != Fix::Good {
        ctx.overlay.request(LabelRequest::new(priority::OWN_SHIP, "?", Place::Around { x, y, r: len / 2.0 + 2.0 }, Token::Stale).strong());
    }
    if let Some(text) = fix_age_text(s.own.pos_updated, now) {
        ctx.overlay.request(LabelRequest::new(priority::OWN_SHIP, text, Place::Around { x, y, r: len / 2.0 + 14.0 }, Token::Stale));
    }
}

/// How a target is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    ClassA,
    ClassB,
    /// At anchor or moored, or not moving.
    Still,
    Aton,
    Sart,
    Lost,
}

pub fn kind_of(t: &AisTarget, now: DateTime<Utc>) -> Kind {
    if now - t.last_seen > TARGET_LOST_AFTER {
        return Kind::Lost;
    }
    if t.class == AisClass::Aton {
        return Kind::Aton;
    }
    // Search-and-rescue transmitters have MMSIs that start with 970.
    if t.mmsi / 1_000_000 == 970 {
        return Kind::Sart;
    }
    let anchored = matches!(t.status, Some(1) | Some(5));
    let moving = t.sog_kn.is_some_and(|v| v >= 0.5);
    if anchored || !moving {
        return Kind::Still;
    }
    if t.class == AisClass::B { Kind::ClassB } else { Kind::ClassA }
}

fn triangle_path(cr: &cairo::Context, x: f64, y: f64, bearing: f64, s: f64) {
    let _ = cr.save();
    cr.translate(x, y);
    cr.rotate(bearing.to_radians());
    cr.new_path();
    cr.move_to(0.0, -s);
    cr.line_to(s * 0.6, s * 0.7);
    cr.line_to(-s * 0.6, s * 0.7);
    cr.close_path();
    let _ = cr.restore();
}

fn draw_targets(ctx: &DrawCtx, app: &AppState, now: DateTime<Utc>) {
    let s = app.session.borrow();
    let (cr, vi, pal) = (ctx.cr, &ctx.vi, &ctx.pal);
    let mpp = vi.metres_per_px();
    let alerts = s.ais_alerts();
    let selected = app.selected.get();
    let pointer = app.pointer.get().map(|p| vi.to_screen(&p));
    let minutes = app.settings.borrow().vector_minutes.max(1.0);
    let own_motion = s.own.pos.zip(s.own.sog_kn).zip(s.own.cog).map(|((pos, sog_kn), cog)| Motion { pos, sog_kn, cog });
    let bounds = vi.bounds().expanded(80.0 * vi.deg_per_px());
    let lit = blink_on(ctx.reduce_motion);
    let hit = app.canvas.hit_min() / 2.0;
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);

    for t in s.targets.iter() {
        let Some(pos) = t.pos else { continue };
        if !bounds.contains([pos.lon, pos.lat]) {
            continue;
        }
        let (x, y) = vi.to_screen(&pos);
        let alert = alerts.iter().find(|(m, _)| *m == t.mmsi).map(|(_, acked)| *acked);
        let danger = alert.is_some();
        let blinking = alert == Some(false);
        let sel = selected == Selection::Target(t.mmsi);
        let kind = kind_of(t, now);
        let hovered = pointer.is_some_and(|(px, py)| (px - x).hypot(py - y) <= hit);
        if hovered {
            cr.new_path();
            cr.arc(x, y, hit, 0.0, 2.0 * PI);
            pal.set(cr, Token::Hover);
            let _ = cr.fill();
        }
        let colour = if danger { Token::AisDanger } else if kind == Kind::Lost { Token::AisLost } else { Token::Ais };
        let r = match kind {
            Kind::ClassB => 7.0,
            Kind::Still => 4.5,
            Kind::Aton | Kind::Sart | Kind::Lost => 5.5,
            Kind::ClassA => 9.0,
        };

        // The vector, for what moves.
        if matches!(kind, Kind::ClassA | Kind::ClassB)
            && let (Some(sog), Some(cog)) = (t.sog_kn, t.cog)
        {
            cr.new_path();
            ray(cr, x, y, cog, 0.0, vector_px(sog, minutes, mpp).min(3000.0));
            stroke_haloed(cr, pal, colour, tokens::STROKE_VECTOR * 0.75);
            cr.new_path();
        }
        // The closest approach: both vessels at that moment, joined.
        if danger
            && let (Some(o), Some(sog), Some(cog)) = (own_motion.as_ref(), t.sog_kn, t.cog)
        {
            let c = cpa(o, &Motion { pos, sog_kn: sog, cog });
            if c.tcpa_min > 0.0 {
                let at = |m: &Motion| m.pos.destination(m.cog, m.sog_kn * c.tcpa_min / 60.0);
                let (a, b) = (vi.to_screen(&at(o)), vi.to_screen(&at(&Motion { pos, sog_kn: sog, cog })));
                cr.new_path();
                cr.move_to(a.0, a.1);
                cr.line_to(b.0, b.1);
                cr.set_dash(&[5.0, 4.0], 0.0);
                stroke_haloed(cr, pal, Token::AisDanger, 1.5);
                cr.set_dash(&[], 0.0);
                cr.new_path();
                for p in [a, b] {
                    cr.arc(p.0, p.1, 3.0, 0.0, 2.0 * PI);
                    stroke_haloed(cr, pal, Token::AisDanger, 1.5);
                    cr.new_path();
                }
            }
        }

        cr.new_path();
        let draw_symbol = !blinking || lit;
        if draw_symbol {
            match kind {
                Kind::ClassA | Kind::ClassB => {
                    let b = t.heading.map(|h| h as f64).or(t.cog).unwrap_or(0.0);
                    triangle_path(cr, x, y, b, r);
                    if danger {
                        fill_haloed(cr, pal, colour);
                        cr.new_path();
                    } else {
                        stroke_haloed(cr, pal, colour, tokens::STROKE_VECTOR);
                        cr.new_path();
                    }
                }
                Kind::Still => {
                    cr.arc(x, y, r, 0.0, 2.0 * PI);
                    if danger {
                        fill_haloed(cr, pal, colour);
                    } else {
                        stroke_haloed(cr, pal, colour, tokens::STROKE_VECTOR);
                    }
                    cr.new_path();
                }
                Kind::Aton => {
                    cr.move_to(x, y - r);
                    cr.line_to(x + r * 0.8, y);
                    cr.line_to(x, y + r);
                    cr.line_to(x - r * 0.8, y);
                    cr.close_path();
                    stroke_haloed(cr, pal, Token::Ais, 1.5);
                    cr.new_path();
                }
                Kind::Sart => {
                    cr.arc(x, y, r, 0.0, 2.0 * PI);
                    stroke_haloed(cr, pal, Token::Alarm, 1.8);
                    cr.new_path();
                    cr.move_to(x - r, y - r);
                    cr.line_to(x + r, y + r);
                    cr.move_to(x - r, y + r);
                    cr.line_to(x + r, y - r);
                    stroke_haloed(cr, pal, Token::Alarm, 1.8);
                    cr.new_path();
                }
                Kind::Lost => {
                    cr.set_dash(&[3.0, 2.5], 0.0);
                    triangle_path(cr, x, y, t.cog.unwrap_or(0.0), r);
                    stroke_haloed(cr, pal, Token::AisLost, 1.5);
                    cr.set_dash(&[], 0.0);
                    cr.new_path();
                    cr.move_to(x - r, y - r);
                    cr.line_to(x + r, y + r);
                    cr.move_to(x - r, y + r);
                    cr.line_to(x + r, y - r);
                    stroke_haloed(cr, pal, Token::AisLost, 1.2);
                    cr.new_path();
                }
            }
        }
        if sel {
            cr.arc(x, y, r + 5.0 + tokens::STROKE_HALO, 0.0, 2.0 * PI);
            pal.set(cr, Token::Selection);
            cr.set_line_width(3.0);
            let _ = cr.stroke();
            cr.new_path();
        }
        ctx.overlay.reserve(x, y, r + 1.0);

        // Names: when dangerous, selected, hovered, or always at large scale (the placer drops what has no room).
        if danger || sel || hovered || vi.zoom() >= 12.5 {
            let mut text = t.name.clone().unwrap_or_else(|| t.mmsi.to_string());
            if danger && let (Some(o), Some(sog), Some(cog)) = (own_motion.as_ref(), t.sog_kn, t.cog) {
                let c = cpa(o, &Motion { pos, sog_kn: sog, cog });
                text.push_str(&format!("  CPA {:.2} nm / {:.0} min", c.cpa_nm, c.tcpa_min));
            }
            let prio = if danger { priority::ALARM } else if sel || hovered { priority::SELECTED } else { priority::AIS };
            let req = LabelRequest::new(prio, text, Place::Around { x, y, r: r + 2.0 }, if danger { Token::AisDanger } else { Token::Ais });
            ctx.overlay.request(if danger { req.strong() } else { req });
        }
    }
}

impl Layer for BoatLayer {
    fn id(&self) -> &'static str {
        "boat"
    }

    fn is_dynamic(&self) -> bool {
        true
    }

    fn animating(&self) -> bool {
        let Some(app) = self.app.upgrade() else { return false };
        !app.canvas.reduce_motion() && app.session.borrow().ais_alerts().iter().any(|(_, acked)| !acked)
    }

    fn draw(&self, ctx: &DrawCtx) {
        let Some(app) = self.app.upgrade() else { return };
        let now = Utc::now();
        draw_targets(ctx, &app, now);
        draw_own(ctx, &app, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::comm::cpa::AisTargets;
    use crate::core::geo::LatLon;

    #[test]
    fn a_vector_is_the_distance_run_in_its_time() {
        // 10 kn for 6 minutes is one nautical mile.
        let px = vector_px(10.0, 6.0, 1852.0 / 100.0);
        assert!((px - 100.0).abs() < 1e-9);
        assert_eq!(vector_px(0.0, 6.0, 10.0), 0.0);
        assert!((vector_px(10.0, 3.0, 1852.0 / 100.0) - 50.0).abs() < 1e-9);
    }

    #[test]
    fn a_fix_goes_stale_after_30_seconds_and_is_lost_after_2_minutes() {
        let now = Utc::now();
        assert_eq!(fix_state(Some(now - Duration::seconds(10)), now), Fix::Good);
        assert_eq!(fix_state(Some(now - Duration::seconds(31)), now), Fix::Stale);
        assert_eq!(fix_state(Some(now - Duration::seconds(121)), now), Fix::Lost);
        assert_eq!(fix_state(None, now), Fix::Lost);
        assert!(fix_age_text(Some(now - Duration::seconds(10)), now).is_none());
        assert_eq!(fix_age_text(Some(now - Duration::minutes(2) - Duration::seconds(5)), now).as_deref(), Some("Fix 2 min old"));
    }

    fn target(mmsi: u32, class: AisClass, sog: Option<f64>, status: Option<u8>, seen: DateTime<Utc>) -> AisTarget {
        AisTarget {
            mmsi,
            class,
            name: None,
            callsign: None,
            ship_type: None,
            dims: None,
            destination: None,
            pos: Some(LatLon::new(48.0, -4.0)),
            sog_kn: sog,
            cog: Some(90.0),
            heading: None,
            status,
            last_seen: seen,
            trail: Vec::new(),
        }
    }

    #[test]
    fn targets_are_drawn_by_what_they_are_doing() {
        let now = Utc::now();
        let seen = now - Duration::seconds(5);
        assert_eq!(kind_of(&target(1, AisClass::A, Some(8.0), Some(0), seen), now), Kind::ClassA);
        assert_eq!(kind_of(&target(1, AisClass::B, Some(5.0), None, seen), now), Kind::ClassB);
        assert_eq!(kind_of(&target(1, AisClass::A, Some(0.0), Some(1), seen), now), Kind::Still, "at anchor");
        assert_eq!(kind_of(&target(1, AisClass::A, Some(8.0), Some(5), seen), now), Kind::Still, "moored, whatever its speed says");
        assert_eq!(kind_of(&target(1, AisClass::Aton, None, None, seen), now), Kind::Aton);
        assert_eq!(kind_of(&target(970_123_456, AisClass::A, None, None, seen), now), Kind::Sart);
        assert_eq!(kind_of(&target(1, AisClass::A, Some(8.0), None, now - Duration::minutes(4)), now), Kind::Lost);
        let _ = AisTargets::default();
    }
}
