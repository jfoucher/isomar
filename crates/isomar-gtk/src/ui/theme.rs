//! The palette of the chrome. Day forces libadwaita's light scheme, Dusk and Night its dark one,
//! and one stylesheet generated from the design tokens (`build.rs`) is swapped on the display, so
//! every window, popover, menu and toast follows at once. Follow system maps light to Day and
//! dark to Dusk; Night is never automatic.

use super::palette::Mode;
use gtk::gdk;
use std::cell::{Cell, RefCell};

const DAY: &str = include_str!(concat!(env!("OUT_DIR"), "/style-day.css"));
const DUSK: &str = include_str!(concat!(env!("OUT_DIR"), "/style-dusk.css"));
const NIGHT: &str = include_str!(concat!(env!("OUT_DIR"), "/style-night.css"));

thread_local! {
    static PROVIDER: RefCell<Option<gtk::CssProvider>> = const { RefCell::new(None) };
    static MODE: Cell<Mode> = const { Cell::new(Mode::Auto) };
}

/// The palette the chrome shows for `mode`, with Follow system resolved.
pub fn effective(mode: Mode) -> Mode {
    match mode {
        Mode::Auto if adw::StyleManager::default().is_dark() => Mode::Dusk,
        Mode::Auto => Mode::Day,
        m => m,
    }
}

/// The token theme of the palette shown now, for chrome drawn with cairo.
pub fn token_theme() -> isomar_app::tokens::Theme {
    use isomar_app::tokens::Theme;
    match effective(MODE.with(|m| m.get())) {
        Mode::Night => Theme::Night,
        Mode::Dusk => Theme::Dusk,
        _ => Theme::Day,
    }
}

/// Applies `mode` to the chrome. The first call adds the provider and follows the system's
/// light or dark preference from then on.
pub fn apply(mode: Mode) {
    MODE.with(|m| m.set(mode));
    let style = adw::StyleManager::default();
    style.set_color_scheme(match mode {
        Mode::Auto => adw::ColorScheme::Default,
        Mode::Day => adw::ColorScheme::ForceLight,
        Mode::Dusk | Mode::Night => adw::ColorScheme::ForceDark,
    });
    let provider = PROVIDER.with(|p| {
        p.borrow_mut()
            .get_or_insert_with(|| {
                let provider = gtk::CssProvider::new();
                if let Some(display) = gdk::Display::default() {
                    gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
                }
                style.connect_dark_notify(|_| {
                    if MODE.with(|m| m.get()) == Mode::Auto {
                        load(Mode::Auto);
                    }
                });
                provider
            })
            .clone()
    });
    let _ = provider;
    load(mode);
}

fn load(mode: Mode) {
    let sheet = match effective(mode) {
        Mode::Night => NIGHT,
        Mode::Dusk => DUSK,
        _ => DAY,
    };
    PROVIDER.with(|p| {
        if let Some(p) = p.borrow().as_ref() {
            p.load_from_string(sheet);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sheet_redefines_the_libadwaita_surfaces() {
        for sheet in [DAY, DUSK, NIGHT] {
            for var in ["--window-bg-color", "--sidebar-bg-color", "--headerbar-bg-color", "--popover-bg-color", "--dialog-bg-color", "--accent-bg-color", "--error-bg-color", "--isomar-info-bg", "--isomar-shadow-card"] {
                assert!(sheet.contains(&format!("{var}:")), "{var} missing");
            }
        }
        assert!(NIGHT.contains("--window-bg-color: #0a0d11;"), "night window background from the tokens");
    }
}
