//! Colours. Everything is defined once in day tones and passed through
//! [`Palette::c`], which darkens and shifts it for dusk and night, so a layer
//! never needs to know which mode is active.

use crate::tokens::{self, Rgba, Theme, Token};
use std::str::FromStr;

pub type Rgb = (f64, f64, f64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    /// Follow the system light/dark preference.
    #[default]
    Auto,
    Day,
    Dusk,
    Night,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Auto => "auto",
            Mode::Day => "day",
            Mode::Dusk => "dusk",
            Mode::Night => "night",
        }
    }
}

impl FromStr for Mode {
    type Err = ();
    fn from_str(s: &str) -> Result<Mode, ()> {
        Ok(match s {
            "auto" => Mode::Auto,
            "day" => Mode::Day,
            "dusk" => Mode::Dusk,
            "night" => Mode::Night,
            _ => return Err(()),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Day,
    Dusk,
    Night,
}

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub tone: Tone,
}

pub const fn rgb(r: u8, g: u8, b: u8) -> Rgb {
    (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0)
}

impl Palette {
    pub fn new(mode: Mode, system_dark: bool) -> Palette {
        let tone = match mode {
            Mode::Day => Tone::Day,
            Mode::Dusk => Tone::Dusk,
            Mode::Night => Tone::Night,
            Mode::Auto if system_dark => Tone::Dusk,
            Mode::Auto => Tone::Day,
        };
        Palette { tone }
    }

    /// A day colour as it appears in this palette. Only for colours that
    /// have no token yet; new colours are tokens (`data/design/tokens.json`).
    pub fn c(&self, (r, g, b): Rgb) -> Rgb {
        match self.tone {
            Tone::Day => (r, g, b),
            Tone::Dusk => (r * 0.58, g * 0.62, b * 0.70),
            Tone::Night => {
                // Mostly red-free and dim, to protect night vision.
                let l = 0.3 * r + 0.55 * g + 0.15 * b;
                (l * 0.10 + r * 0.05, l * 0.22 + g * 0.05, l * 0.34 + b * 0.10)
            }
        }
    }

    pub fn theme(&self) -> Theme {
        match self.tone {
            Tone::Day => Theme::Day,
            Tone::Dusk => Theme::Dusk,
            Tone::Night => Theme::Night,
        }
    }

    /// A token's colour in this palette, with its alpha.
    pub fn color(&self, token: Token) -> Rgba {
        tokens::color(token, self.theme())
    }

    /// What `paint` looks like in this palette, as (red, green, blue, alpha).
    pub fn rgba(&self, paint: impl Into<Paint>) -> Rgba {
        match paint.into() {
            Paint::Token(t) => self.color(t),
            Paint::Legacy(c) => {
                let (r, g, b) = self.c(c);
                (r, g, b, 1.0)
            }
        }
    }

    pub fn set(&self, cr: &cairo::Context, paint: impl Into<Paint>) {
        let (r, g, b, a) = self.rgba(paint);
        cr.set_source_rgba(r, g, b, a);
    }

    /// Like `set`, with the colour's own alpha scaled by `a`.
    pub fn set_alpha(&self, cr: &cairo::Context, paint: impl Into<Paint>, a: f64) {
        let (r, g, b, own) = self.rgba(paint);
        cr.set_source_rgba(r, g, b, own * a);
    }
}

/// A colour to paint with: a design token, or a day colour for the palette to adjust.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Paint {
    Token(Token),
    Legacy(Rgb),
}

impl From<Token> for Paint {
    fn from(t: Token) -> Paint {
        Paint::Token(t)
    }
}

impl From<Rgb> for Paint {
    fn from(c: Rgb) -> Paint {
        Paint::Legacy(c)
    }
}

// Colours shared by the layers.
pub const VOID: Token = Token::ChartVoid;
pub const SEA: Token = Token::ChartSea;
pub const LAND: Token = Token::ChartLand;
pub const LAND_EDGE: Token = Token::ChartCoast;
/// Built-up areas and the buildings in them, a shade darker than the land.
pub const TOWN: Token = Token::ChartLandBuilt;
pub const TOWN_EDGE: Token = Token::ChartTownEdge;
pub const BUILDING: Token = Token::ChartBuilding;
pub const BUILDING_EDGE: Token = Token::ChartBuildingEdge;
pub const GRID: Token = Token::ChartGrid;
pub const TEXT: Token = Token::ChartLabel;
pub const MAGENTA: Token = Token::ChartMagenta;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn night_is_darker_and_bluer_than_day() {
        let day = Palette::new(Mode::Day, false);
        let night = Palette::new(Mode::Night, false);
        let (r, g, b, _) = night.rgba(SEA);
        let (dr, dg, db, _) = day.rgba(SEA);
        assert!(r + g + b < (dr + dg + db) * 0.4);
        assert!(b > r, "night keeps red low");
    }

    #[test]
    fn legacy_colours_are_dimmed_and_tokens_are_the_designs() {
        let day = Palette::new(Mode::Day, false);
        let dusk = Palette::new(Mode::Dusk, false);
        assert_eq!(day.rgba(rgb(10, 20, 30)), (10.0 / 255.0, 20.0 / 255.0, 30.0 / 255.0, 1.0));
        assert!(dusk.rgba(rgb(200, 200, 200)).0 < day.rgba(rgb(200, 200, 200)).0);
        assert_eq!(dusk.rgba(Token::OwnShip), tokens::color(Token::OwnShip, Theme::Dusk));
    }

    #[test]
    fn auto_follows_the_system() {
        assert_eq!(Palette::new(Mode::Auto, false).tone, Tone::Day);
        assert_eq!(Palette::new(Mode::Auto, true).tone, Tone::Dusk);
        assert_eq!(Palette::new(Mode::Night, false).tone, Tone::Night);
    }

    #[test]
    fn modes_parse_and_print() {
        for m in [Mode::Auto, Mode::Day, Mode::Dusk, Mode::Night] {
            assert_eq!(m.as_str().parse::<Mode>(), Ok(m));
        }
        assert!("blue".parse::<Mode>().is_err());
    }
}
