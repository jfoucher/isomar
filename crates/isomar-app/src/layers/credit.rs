//! The credit the tile data's licence asks for, shown while any of it is on the chart.

use super::{DrawCtx, Layer, font_size};
use crate::core::chart::naturalearth::CREDIT as NATURAL_EARTH_CREDIT;
use crate::core::chart::places::CREDIT as TOWNS_CREDIT;
use crate::core::chart::vtile::CREDIT;

/// What EMODnet Bathymetry asks to be cited as.
pub const EMODNET_CREDIT: &str = "© EMODnet Bathymetry Consortium";
use crate::core::chart::dem::CREDIT as RELIEF_CREDIT;
use crate::palette::TEXT;
use std::cell::Cell;
use std::rc::Rc;

/// Set by the layers when they prepare an area, to whether the data they draw there needs a credit.
#[derive(Default)]
pub struct Credit {
    pub land: Cell<bool>,
    pub marks: Cell<bool>,
    pub towns: Cell<bool>,
    pub depth: Cell<bool>,
    /// Land relief from the elevation tiles is drawn.
    pub relief: Cell<bool>,
    /// Names from Natural Earth are written.
    pub natural_earth: Cell<bool>,
    /// The forecast drawn, by its model's credit (ECMWF's licence asks for it).
    pub weather: Cell<Option<&'static str>>,
    /// The currents drawn, by their sources' credits (Copernicus and Shom ask for them).
    pub currents: std::cell::RefCell<Vec<&'static str>>,
    /// What was credited in the last pass, for a shell that draws its own footer.
    pub shown: std::cell::RefCell<Vec<&'static str>>,
    /// The shell draws the credits itself: this layer only records them.
    pub hidden: Cell<bool>,
}

/// Drawn every frame, over the tiles: it centres its text at the bottom of the screen.
pub struct CreditLayer(pub Rc<Credit>);

impl Layer for CreditLayer {
    fn id(&self) -> &'static str {
        "credit"
    }

    fn is_dynamic(&self) -> bool {
        true
    }

    fn draw(&self, ctx: &DrawCtx) {
        let osm = self.0.land.get() | self.0.marks.get();
        let mut parts: Vec<&str> = [(osm, CREDIT), (self.0.towns.get(), TOWNS_CREDIT), (self.0.natural_earth.get(), NATURAL_EARTH_CREDIT), (self.0.depth.get(), EMODNET_CREDIT), (self.0.relief.get(), RELIEF_CREDIT)]
            .into_iter()
            .filter_map(|(on, text)| on.then_some(text))
            .collect();
        parts.extend(self.0.weather.get());
        for c in self.0.currents.borrow().iter() {
            if !parts.contains(c) {
                parts.push(c);
            }
        }
        *self.0.shown.borrow_mut() = parts.clone();
        if parts.is_empty() || self.0.hidden.get() {
            return;
        }
        let text = parts.join(" · ");
        let (cr, vi, pal) = (ctx.cr, &ctx.vi, &ctx.pal);
        cr.select_font_face(crate::layers::FONT_FAMILY, cairo::FontSlant::Normal, cairo::FontWeight::Normal);
        font_size(cr, 9.5);
        if let Ok(ext) = cr.text_extents(&text) {
            cr.new_path();
            pal.set(cr, TEXT);
            cr.move_to((vi.w - ext.width()) / 2.0, vi.h - 6.0);
            let _ = cr.show_text(&text);
        }
    }
}
