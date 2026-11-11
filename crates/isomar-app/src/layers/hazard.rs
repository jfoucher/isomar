//! Dangers to navigation in S-57 cells: rocks, wrecks and obstructions, drawn as S-52 tells them apart.
//! What a chart means by one depends on whether it shows at some tide (`WATLEV`), what it is (`CATWRK`,
//! `CATOBS`) and how deep it lies against the mariner's safety depth: a submerged danger deeper than that is a
//! small cross, one that is shoaler, or whose depth is not known, is an isolated danger ringed in magenta.

use crate::core::chart::s57::{Feature, attr};
use crate::palette::{MAGENTA, Palette};
use crate::tokens::Token;

/// What a danger is drawn as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    /// A rock that stays above water: a star.
    Star,
    /// A rock or obstruction that covers and uncovers, or is awash: a cross with a dot in each quarter.
    DottedCross,
    /// A submerged danger with more water over it than the safety depth: a small cross.
    Cross,
    /// A submerged danger shoaler than the safety depth, or of unknown depth: a cross in a magenta ring.
    Isolated,
    /// A wreck: its hull and masts.
    Wreck,
    /// The scattered remains of a wreck: the hull alone.
    WreckRemains,
    /// Foul ground or a foul area, a fish haven: a dotted ring.
    Foul,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hazard {
    pub glyph: Glyph,
    /// Shoaler than the safety depth, or of unknown depth: it is ringed (a wreck) and its outline is magenta.
    pub dangerous: bool,
    /// The least depth over it, `VALSOU`, in metres.
    pub depth: Option<f64>,
    /// The depth is not well known (`QUASOU`), so it is drawn with a ring of dots round it.
    pub doubtful: bool,
}

/// `QUASOU` values for a depth that is doubtful, unreliable, not found, or reported and not surveyed.
pub fn doubtful_sounding(f: &Feature) -> bool {
    f.list(attr::QUASOU).iter().any(|q| matches!(q, 3 | 4 | 5 | 8 | 9))
}

/// Whether a `WATLEV` shows the danger at some state of the tide.
fn shows_at_some_tide(watlev: Option<f64>) -> bool {
    matches!(watlev.map(|w| w as i32), Some(1 | 2 | 4 | 5))
}

/// Works out what a rock, wreck or obstruction is drawn as, given the safety depth in metres.
pub fn classify(f: &Feature, safety: f64) -> Hazard {
    use crate::core::chart::s57::obj;
    let watlev = f.num(attr::WATLEV);
    let depth = f.num(attr::VALSOU);
    let above = matches!(watlev.map(|w| w as i32), Some(1 | 2));
    let covers = matches!(watlev.map(|w| w as i32), Some(4 | 5));
    // An unknown depth is taken as dangerous, as S-52 does.
    let shoal = depth.is_none_or(|d| d <= safety);
    let doubtful = depth.is_some() && doubtful_sounding(f);
    let (glyph, dangerous) = match f.objl {
        obj::UWTROC if above => (Glyph::Star, false),
        obj::UWTROC | obj::OBSTRN if covers => (Glyph::DottedCross, false),
        obj::OBSTRN if matches!(f.num(attr::CATOBS).map(|c| c as i32), Some(5..=7)) => (Glyph::Foul, shoal),
        obj::OBSTRN if above => (Glyph::Cross, false),
        obj::WRECKS => {
            let category = f.num(attr::CATWRK).map(|c| c as i32);
            let glyph = if category == Some(3) { Glyph::WreckRemains } else { Glyph::Wreck };
            // A wreck showing at some tide is plain to see; a submerged one is dangerous when shoal, or called so.
            let dangerous = !shows_at_some_tide(watlev) && (category == Some(2) || (category != Some(1) && shoal));
            (glyph, dangerous)
        }
        _ if shoal => (Glyph::Isolated, true),
        _ => (Glyph::Cross, false),
    };
    Hazard { glyph, dangerous, depth, doubtful }
}

/// Draws a danger's symbol centred on (`x`, `y`).
pub fn draw(cr: &cairo::Context, pal: &Palette, x: f64, y: f64, h: &Hazard) {
    let _ = cr.save();
    cr.new_path();
    cr.set_line_width(1.4);
    cr.set_line_cap(cairo::LineCap::Round);
    pal.set(cr, Token::ChartInk);
    match h.glyph {
        Glyph::Star => {
            for k in 0..3 {
                let a = std::f64::consts::PI * k as f64 / 3.0;
                let (dx, dy) = (4.8 * a.cos(), 4.8 * a.sin());
                cr.move_to(x - dx, y - dy);
                cr.line_to(x + dx, y + dy);
            }
            let _ = cr.stroke();
        }
        Glyph::DottedCross => {
            cr.move_to(x - 4.5, y);
            cr.line_to(x + 4.5, y);
            cr.move_to(x, y - 4.5);
            cr.line_to(x, y + 4.5);
            let _ = cr.stroke();
            for (dx, dy) in [(-3.2, -3.2), (3.2, -3.2), (-3.2, 3.2), (3.2, 3.2)] {
                cr.new_path();
                cr.arc(x + dx, y + dy, 0.9, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
            }
        }
        Glyph::Cross => {
            cr.set_line_width(1.0);
            cr.move_to(x - 3.5, y);
            cr.line_to(x + 3.5, y);
            cr.move_to(x, y - 3.5);
            cr.line_to(x, y + 3.5);
            let _ = cr.stroke();
        }
        Glyph::Isolated => {
            pal.set(cr, MAGENTA);
            cr.set_line_width(1.5);
            cr.arc(x, y, 7.0, 0.0, std::f64::consts::TAU);
            let _ = cr.stroke();
            pal.set(cr, Token::ChartInk);
            cr.set_line_width(1.2);
            cr.move_to(x - 3.5, y);
            cr.line_to(x + 3.5, y);
            cr.move_to(x, y - 3.5);
            cr.line_to(x, y + 3.5);
            let _ = cr.stroke();
        }
        Glyph::Wreck | Glyph::WreckRemains => {
            if h.dangerous {
                pal.set(cr, MAGENTA);
                cr.set_line_width(1.3);
                cr.set_dash(&[1.5, 2.5], 0.0);
                cr.arc(x, y, 9.5, 0.0, std::f64::consts::TAU);
                let _ = cr.stroke();
                cr.set_dash(&[], 0.0);
                pal.set(cr, Token::ChartInk);
            }
            cr.set_line_width(1.5);
            cr.move_to(x - 6.0, y);
            cr.line_to(x - 4.0, y + 3.5);
            cr.line_to(x + 4.0, y + 3.5);
            cr.line_to(x + 6.0, y);
            if h.glyph == Glyph::Wreck {
                cr.move_to(x - 2.0, y);
                cr.line_to(x - 2.0, y - 5.5);
                cr.move_to(x + 2.0, y);
                cr.line_to(x + 2.0, y - 5.5);
            }
            let _ = cr.stroke();
        }
        Glyph::Foul => {
            if h.dangerous {
                pal.set(cr, MAGENTA);
            }
            cr.set_line_width(1.2);
            cr.set_dash(&[1.2, 2.2], 0.0);
            cr.arc(x, y, 6.0, 0.0, std::f64::consts::TAU);
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);
            cr.arc(x, y, 1.0, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        }
    }
    let _ = cr.restore();
}

/// A ring of dots round a depth that is not well known, centred on (`x`, `y`).
pub fn draw_doubtful_ring(cr: &cairo::Context, pal: &Palette, x: f64, y: f64) {
    let _ = cr.save();
    pal.set(cr, Token::ChartSounding);
    cr.set_line_width(1.0);
    cr.set_dash(&[1.0, 2.0], 0.0);
    cr.new_path();
    cr.arc(x, y, 8.5, 0.0, std::f64::consts::TAU);
    let _ = cr.stroke();
    let _ = cr.restore();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::chart::s57::{Geom, obj};
    use std::collections::BTreeMap;

    fn feature(objl: u16, attrs: &[(u16, &str)]) -> Feature {
        Feature { rcid: 1, objl, attrs: attrs.iter().map(|(k, v)| (*k, v.to_string())).collect::<BTreeMap<_, _>>(), geom: Geom::Point([0.0, 0.0]) }
    }

    #[test]
    fn a_rock_is_drawn_by_how_the_tide_leaves_it() {
        let h = |w: &str| classify(&feature(obj::UWTROC, &[(attr::WATLEV, w)]), 5.0);
        assert_eq!(h("2").glyph, Glyph::Star);
        assert_eq!(h("4").glyph, Glyph::DottedCross);
        assert_eq!(h("5").glyph, Glyph::DottedCross);
        assert!(!h("4").dangerous, "it shows, so it is not an isolated danger");
    }

    #[test]
    fn a_submerged_rock_is_dangerous_unless_deeper_than_the_safety_depth() {
        let rock = |v: Option<&str>| {
            let mut a = vec![(attr::WATLEV, "3")];
            a.extend(v.map(|v| (attr::VALSOU, v)));
            classify(&feature(obj::UWTROC, &a), 5.0)
        };
        assert_eq!((rock(Some("2.3")).glyph, rock(Some("2.3")).dangerous), (Glyph::Isolated, true));
        assert_eq!(rock(Some("12.0")).glyph, Glyph::Cross);
        assert!(!rock(Some("12.0")).dangerous);
        assert_eq!(rock(None).glyph, Glyph::Isolated, "no depth is taken as dangerous");
        assert_eq!(rock(Some("5.0")).glyph, Glyph::Isolated, "at the safety depth is still shoal");
    }

    #[test]
    fn a_wreck_is_ringed_when_dangerous_and_only_then() {
        let wreck = |a: &[(u16, &str)]| classify(&feature(obj::WRECKS, a), 5.0);
        assert!(wreck(&[(attr::CATWRK, "2"), (attr::WATLEV, "3"), (attr::VALSOU, "30")]).dangerous, "called dangerous");
        assert!(wreck(&[(attr::WATLEV, "3"), (attr::VALSOU, "3.1")]).dangerous, "shoal");
        assert!(!wreck(&[(attr::WATLEV, "3"), (attr::VALSOU, "31")]).dangerous, "deep");
        assert!(!wreck(&[(attr::WATLEV, "4"), (attr::CATWRK, "2")]).dangerous, "showing at low tide: plain to see");
        assert!(!wreck(&[(attr::CATWRK, "1"), (attr::WATLEV, "3")]).dangerous, "non-dangerous with no depth");
        assert_eq!(wreck(&[(attr::CATWRK, "3")]).glyph, Glyph::WreckRemains);
    }

    #[test]
    fn foul_areas_are_foul_whatever_their_depth() {
        let h = classify(&feature(obj::OBSTRN, &[(attr::CATOBS, "6"), (attr::WATLEV, "3")]), 5.0);
        assert_eq!(h.glyph, Glyph::Foul);
        assert!(h.dangerous);
    }

    #[test]
    fn a_doubtful_depth_is_marked() {
        let f = feature(obj::UWTROC, &[(attr::WATLEV, "3"), (attr::VALSOU, "4.0"), (attr::QUASOU, "4")]);
        assert!(classify(&f, 5.0).doubtful);
        let f = feature(obj::UWTROC, &[(attr::WATLEV, "3"), (attr::VALSOU, "4.0"), (attr::QUASOU, "1,6")]);
        assert!(!classify(&f, 5.0).doubtful);
    }
}
