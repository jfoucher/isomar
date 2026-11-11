//! The statistics of the alternatives of a weather-route search, side by side.

use adw::prelude::*;
use isomar_app::vm::stats::Section;

/// The heading of the alternative drawn in token `colour`: its swatch and its number.
pub fn head(colour: &str, n: usize) -> String {
    use isomar_app::tokens::{self, Token};
    let token = Token::ALL.iter().copied().find(|t| t.name() == colour).unwrap_or(Token::Route);
    let (r, g, b, _) = tokens::color(token, super::theme::token_theme());
    let hex = format!("#{:02x}{:02x}{:02x}", (r * 255.0).round() as u8, (g * 255.0).round() as u8, (b * 255.0).round() as u8);
    format!("<span foreground=\"{hex}\" size=\"large\">●</span> <b>{n}</b>")
}

/// Opens a table of `sections`, a column under each of `heads` (Pango markup).
pub fn show_sections(title: &str, heads: &[String], sections: &[Section]) {
    if heads.is_empty() {
        return;
    }
    let dialog = super::float::window(title, 240 + 190 * heads.len() as i32, 640);

    let grid = gtk::Grid::builder().column_spacing(18).row_spacing(6).margin_top(12).margin_bottom(12).margin_start(18).margin_end(18).build();
    let mut row = 0;
    for (k, markup) in heads.iter().enumerate() {
        let head = gtk::Label::builder().xalign(0.0).build();
        head.set_markup(markup);
        grid.attach(&head, k as i32 + 1, row, 1, 1);
    }
    row += 1;
    for section in sections {
        let title = gtk::Label::builder().label(&section.title).xalign(0.0).css_classes(["heading"]).margin_top(10).build();
        grid.attach(&title, 0, row, heads.len() as i32 + 1, 1);
        row += 1;
        for (label, cells) in &section.rows {
            grid.attach(&gtk::Label::builder().label(label).xalign(0.0).hexpand(true).wrap(true).max_width_chars(34).build(), 0, row, 1, 1);
            for (k, cell) in cells.iter().enumerate() {
                grid.attach(&gtk::Label::builder().label(cell).xalign(0.0).css_classes(["numeric"]).wrap(true).max_width_chars(24).build(), k as i32 + 1, row, 1, 1);
            }
            row += 1;
        }
    }
    let scroller = gtk::ScrolledWindow::builder().child(&grid).vexpand(true).build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&scroller));
    dialog.set_content(Some(&toolbar));
    dialog.present();
}

