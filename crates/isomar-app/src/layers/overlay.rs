//! What every overlay layer shares (`design/design-system/overlay-spec.md`): the halo under every
//! stroke and label, and one label placer for the frame, so that overlay labels collide with each
//! other by priority and never cover a symbol.

use super::{font_size, text_scale};
use crate::palette::{Paint, Palette};
use crate::tokens::{self, Token};
use std::cell::RefCell;
use std::time::{SystemTime, UNIX_EPOCH};

/// Label priority classes, highest first (the overlay spec's list).
pub mod priority {
    pub const ALARM: u8 = 1;
    pub const OWN_SHIP: u8 = 2;
    pub const ACTIVE_ROUTE: u8 = 3;
    pub const AIS: u8 = 4;
    pub const SELECTED: u8 = 5;
    pub const WAYPOINT: u8 = 6;
    pub const WEATHER: u8 = 7;
    pub const CHART: u8 = 8;
    pub const GRID: u8 = 9;
}

/// Where a label may go.
#[derive(Clone, Debug)]
pub enum Place {
    /// Around a symbol at (`x`, `y`) of radius `r`: right, left, above, below, then the corners.
    Around { x: f64, y: f64, r: f64 },
    /// Centred on a point (a leg's middle, an isobar).
    Centre { x: f64, y: f64 },
    /// With the text's baseline-left corner at these positions, in order of preference.
    At(Vec<(f64, f64)>),
}

/// How a label looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Look {
    /// Upright text on the halo.
    Plain,
    /// The same in the heavier weight.
    Strong,
    /// Text on a filled chip: (chip, text).
    Chip(Token, Token),
}

#[derive(Clone, Debug)]
pub struct LabelRequest {
    pub priority: u8,
    pub text: String,
    pub place: Place,
    pub colour: Token,
    pub look: Look,
    pub size: f64,
}

impl LabelRequest {
    pub fn new(priority: u8, text: impl Into<String>, place: Place, colour: Token) -> LabelRequest {
        LabelRequest { priority, text: text.into(), place, colour, look: Look::Plain, size: 10.5 }
    }

    pub fn strong(mut self) -> LabelRequest {
        self.look = Look::Strong;
        self
    }

    pub fn chip(mut self, bg: Token, text: Token) -> LabelRequest {
        self.look = Look::Chip(bg, text);
        self
    }
}

/// The labels asked for during a frame, and the places symbols took.
#[derive(Default)]
pub struct Overlay {
    labels: RefCell<Vec<LabelRequest>>,
    reserved: RefCell<Vec<[f64; 4]>>,
}

impl Overlay {
    pub fn request(&self, r: LabelRequest) {
        self.labels.borrow_mut().push(r);
    }

    /// Keeps labels off a symbol: a disc of radius `r` at (`x`, `y`).
    pub fn reserve(&self, x: f64, y: f64, r: f64) {
        self.reserved.borrow_mut().push([x - r, y - r, x + r, y + r]);
    }

    pub fn pending(&self) -> usize {
        self.labels.borrow().len()
    }
}

/// Whether a blinking thing is lit now. It is steady with reduced motion.
pub fn blink_on(reduce_motion: bool) -> bool {
    if reduce_motion {
        return true;
    }
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis()) as f64;
    ((ms / tokens::DANGER_BLINK) as u64).is_multiple_of(2)
}

/// Strokes the current path with the halo under it and then `paint`, at `width`. The halo is
/// `STROKE_HALO` wider on each side, and solid even when the line is dashed. The path is kept.
pub fn stroke_haloed(cr: &cairo::Context, pal: &Palette, paint: impl Into<Paint>, width: f64) {
    let (dashes, offset) = cr.dash();
    cr.set_dash(&[], 0.0);
    cr.set_line_width(width + 2.0 * tokens::STROKE_HALO);
    pal.set_alpha(cr, Token::OverlayHalo, 0.75);
    let _ = cr.stroke_preserve();
    cr.set_dash(&dashes, offset);
    cr.set_line_width(width);
    pal.set(cr, paint);
    let _ = cr.stroke_preserve();
}

/// The same for the current path filled: a halo outline, then the fill.
pub fn fill_haloed(cr: &cairo::Context, pal: &Palette, paint: impl Into<Paint>) {
    pal.set(cr, paint);
    let _ = cr.fill_preserve();
    cr.set_line_width(tokens::STROKE_HALO);
    pal.set_alpha(cr, Token::OverlayHalo, 0.75);
    let _ = cr.stroke_preserve();
}

fn set_weight(cr: &cairo::Context, strong: bool) {
    cr.select_font_face(crate::layers::FONT_FAMILY, cairo::FontSlant::Normal, if strong { cairo::FontWeight::Bold } else { cairo::FontWeight::Normal });
}

fn overlaps(a: &[f64; 4], b: &[f64; 4]) -> bool {
    a[0] < b[2] && a[2] > b[0] && a[1] < b[3] && a[3] > b[1]
}

/// Candidate top-left corners for a box of `w` by `h` placed by `place`.
fn candidates(place: &Place, w: f64, h: f64) -> Vec<(f64, f64)> {
    match place {
        Place::At(v) => v.iter().map(|&(x, y)| (x, y - h)).collect(),
        Place::Centre { x, y } => vec![(x - w / 2.0, y - h / 2.0)],
        Place::Around { x, y, r } => {
            let g = r + 3.0;
            vec![
                (x + g, y - h / 2.0),
                (x - g - w, y - h / 2.0),
                (x - w / 2.0, y - g - h),
                (x - w / 2.0, y + g),
                (x + g, y - g - h),
                (x - g - w, y - g - h),
                (x + g, y + g),
                (x - g - w, y + g),
            ]
        }
    }
}

/// Places the frame's labels, highest priority first, and draws those that fit. Priorities 1 to 3
/// try up to eight positions, the rest four. A label that does not fit is not drawn.
pub fn flush(cr: &cairo::Context, pal: &Palette, overlay: &Overlay, view: (f64, f64)) {
    let mut requests = std::mem::take(&mut *overlay.labels.borrow_mut());
    requests.sort_by_key(|r| r.priority);
    let mut taken: Vec<[f64; 4]> = overlay.reserved.borrow().clone();
    let halo = tokens::STROKE_HALO;
    for r in requests {
        let _ = cr.save();
        set_weight(cr, r.look == Look::Strong);
        font_size(cr, r.size);
        let Ok(e) = cr.text_extents(&r.text) else {
            let _ = cr.restore();
            continue;
        };
        let pad = if matches!(r.look, Look::Chip(..)) { 4.0 } else { 0.0 };
        let (w, h) = (e.width() + 2.0 * pad, e.height() + 2.0 * pad);
        let limit = if r.priority <= priority::ACTIVE_ROUTE { 8 } else { 4 };
        let found = candidates(&r.place, w, h).into_iter().take(limit).find(|&(x, y)| {
            let b = [x - halo, y - halo, x + w + halo, y + h + halo];
            b[0] >= 0.0 && b[1] >= 0.0 && b[2] <= view.0 && b[3] <= view.1 && !taken.iter().any(|t| overlaps(t, &b))
        });
        if let Some((x, y)) = found {
            taken.push([x - halo, y - halo, x + w + halo, y + h + halo]);
            cr.new_path();
            match r.look {
                Look::Chip(bg, fg) => {
                    cr.rectangle(x, y, w, h);
                    pal.set(cr, bg);
                    let _ = cr.fill();
                    pal.set(cr, fg);
                    cr.move_to(x + pad - e.x_bearing(), y + pad - e.y_bearing());
                    let _ = cr.show_text(&r.text);
                }
                // The same outline as `halo_text` (twice `STROKE_HALO`), drawn once and kept.
                _ => crate::layers::halo_text(cr, pal, x - e.x_bearing(), y - e.y_bearing(), &r.text, r.colour),
            }
            cr.new_path();
        }
        let _ = cr.restore();
    }
    let _ = text_scale();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface() -> (cairo::ImageSurface, cairo::Context) {
        let s = cairo::ImageSurface::create(cairo::Format::ARgb32, 400, 300).unwrap();
        let cr = cairo::Context::new(&s).unwrap();
        (s, cr)
    }

    fn pal() -> Palette {
        Palette::new(crate::palette::Mode::Day, false)
    }

    #[test]
    fn a_higher_priority_label_takes_the_place_and_the_other_moves_or_goes() {
        let (_s, cr) = surface();
        let o = Overlay::default();
        // The low-priority label is asked for first but loses the contested spot.
        o.request(LabelRequest::new(priority::WAYPOINT, "Waypoint", Place::At(vec![(100.0, 100.0)]), Token::Route));
        o.request(LabelRequest::new(priority::ALARM, "ARMORIQUE", Place::At(vec![(100.0, 100.0)]), Token::AisDanger).strong());
        flush(&cr, &pal(), &o, (400.0, 300.0));
        drop(cr);
        let mut s = _s;
        let data = s.data().unwrap();
        // Something was drawn around (100, 100), in the alarm colour (red channel high, blue low) near the baseline.
        let stride = 400 * 4;
        let mut red = 0;
        for y in 88..104 {
            for x in 100..160 {
                let i = y * stride + x * 4;
                if data[i + 2] > 150 && data[i] < 100 {
                    red += 1;
                }
            }
        }
        assert!(red > 20, "the alarm label is drawn where both asked: {red}");
    }

    #[test]
    fn labels_keep_off_reserved_symbols_and_the_edge() {
        let (_s, cr) = surface();
        let o = Overlay::default();
        o.reserve(200.0, 150.0, 30.0);
        // Both the right and left positions of the first label are on the symbol or off the view.
        o.request(LabelRequest::new(priority::AIS, "Name", Place::At(vec![(190.0, 150.0), (-20.0, 150.0)]), Token::Ais));
        flush(&cr, &pal(), &o, (400.0, 300.0));
        assert_eq!(o.pending(), 0, "requests are consumed");
        drop(cr);
        let mut s = _s;
        assert!(s.data().unwrap().iter().all(|b| *b == 0), "nothing drawn");
    }

    #[test]
    fn blinking_is_steady_with_reduced_motion() {
        assert!(blink_on(true));
    }
}
