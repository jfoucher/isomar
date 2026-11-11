//! Popovers on the chart for what is clicked (`design/ia.md`: objects open in a popover, their
//! full detail in the page that owns them): a buoy, beacon or light (`isomar_app::aidinfo`), a
//! vessel, and the tide with its curve.

pub use isomar_app::aidinfo::*;

use super::app::AppState;
use crate::core::geo::LatLon;
use crate::i18n::gettext;
use adw::prelude::*;
use isomar_app::chips::Icon;
use isomar_app::flows::tides::TideInfo;
use isomar_app::tokens::{self, Token};
use gtk::{gdk, glib};
use std::rc::Rc;

/// Pops `content` up on the chart, pointing at (`x`, `y`).
pub fn show_at(content: &impl IsA<gtk::Widget>, x: f64, y: f64) -> gtk::Popover {
    let pop = gtk::Popover::new();
    pop.set_child(Some(content));
    pop.set_parent(&super::chart_widget());
    pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    pop.connect_closed(|p| {
        let p = p.clone();
        // Unparent after any pending event has been dispatched.
        glib::idle_add_local_once(move || p.unparent());
    });
    pop.popup();
    pop
}

fn column() -> gtk::Box {
    gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).margin_top(10).margin_bottom(10).margin_start(12).margin_end(12).build()
}

fn rows_grid(rows: &[(String, String)]) -> gtk::Grid {
    let grid = gtk::Grid::builder().column_spacing(12).row_spacing(2).margin_top(4).build();
    for (i, (name, value)) in rows.iter().enumerate() {
        let n = gtk::Label::builder().label(name).halign(gtk::Align::Start).valign(gtk::Align::Start).xalign(0.0).css_classes(["dim-label"]).build();
        let v = gtk::Label::builder().label(value).halign(gtk::Align::Start).xalign(0.0).wrap(true).width_chars(VALUE_MIN_CHARS).max_width_chars(VALUE_MAX_CHARS).selectable(true).build();
        grid.attach(&n, 0, i as i32, 1, 1);
        grid.attach(&v, 1, i as i32, 1, 1);
    }
    grid
}

/// The next high and low waters: an arrow up or down, the time, the height lined up on the right.
fn extremes_grid(rows: &[isomar_app::flows::tides::Extreme]) -> gtk::Grid {
    let grid = gtk::Grid::builder().column_spacing(12).row_spacing(2).margin_top(4).build();
    for (i, e) in rows.iter().enumerate() {
        let arrow = gtk::Image::builder().icon_name(super::chips::icon_name(if e.high { Icon::High } else { Icon::Low })).tooltip_text(&e.name).build();
        arrow.update_property(&[gtk::accessible::Property::Label(&e.name)]);
        grid.attach(&arrow, 0, i as i32, 1, 1);
        grid.attach(&gtk::Label::builder().label(&e.time).xalign(0.0).css_classes(["numeric"]).build(), 1, i as i32, 1, 1);
        grid.attach(&gtk::Label::builder().label(&e.height).xalign(1.0).hexpand(true).css_classes(["numeric"]).build(), 2, i as i32, 1, 1);
    }
    grid
}

fn heading(title: &str, subtitle: Option<&str>, b: &gtk::Box) {
    b.append(&gtk::Label::builder().label(title).halign(gtk::Align::Start).wrap(true).max_width_chars(36).xalign(0.0).css_classes(["heading"]).build());
    if let Some(s) = subtitle {
        b.append(&gtk::Label::builder().label(s).halign(gtk::Align::Start).xalign(0.0).css_classes(["dim-label"]).build());
    }
}

/// What a popover's rows may take of the chart's height: all of it but this much (the heading, the stepper, the
/// margins and the room the popover needs round its point), within these limits.
const ROOM_AROUND: i32 = 150;
const MIN_ROWS_HEIGHT: i32 = 120;
const MAX_ROWS_HEIGHT: i32 = 420;
/// How wide a row's value is, in characters: wrapping starts at the larger (a scrolled window gives a wrapping
/// label its least width unless told to take the natural one, which made the column very narrow).
const VALUE_MIN_CHARS: i32 = 26;
const VALUE_MAX_CHARS: i32 = 48;

thread_local! {
    /// The popover for a mark or feature, while it is open: stepping to another feature of the same spot
    /// changes what it says without closing it.
    static INFO: std::cell::RefCell<Option<gtk::Popover>> = const { std::cell::RefCell::new(None) };
}

/// "‹ 1 of 3 ›": which of the features under the click the popover shows, and the way to the others.
fn chooser(position: usize, of: usize, app: &Rc<AppState>) -> gtk::Box {
    let row = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(4).build();
    let step = |icon: &str, tip: String, delta: i32| {
        let button = gtk::Button::builder().icon_name(icon).tooltip_text(tip).css_classes(["flat", "circular"]).build();
        let app = Rc::downgrade(app);
        button.connect_clicked(move |_| {
            if let Some(app) = app.upgrade() {
                isomar_app::probe::cycle(&app, delta);
            }
        });
        button
    };
    row.append(&step("go-previous-symbolic", gettext("Previous"), -1));
    let text = crate::i18n::gettext_f("{position} of {total}", &[("position", &position.to_string()), ("total", &of.to_string())]);
    row.append(&gtk::Label::builder().label(text).hexpand(true).css_classes(["dim-label"]).build());
    let next = step("go-next-symbolic", gettext("Next"), 1);
    row.append(&next);
    row
}

/// The description of a mark or feature at (`x`, `y`). With `app`, the outline of what it describes stays on the
/// chart until the popover closes, and several features under the click can be stepped through.
pub fn show_description(app: Option<&Rc<AppState>>, d: &Description, x: f64, y: f64) {
    let b = column();
    if let (Some((position, of)), Some(app)) = (d.choice, app) {
        b.append(&chooser(position, of, app));
    }
    heading(&d.title, d.subtitle.as_deref(), &b);
    // A popover taller than its window is dismissed as soon as it is shown (a chart's note can run to
    // paragraphs), so the rows scroll in the room above or below the point, the heading and the stepper
    // staying put.
    let side = y.max(super::chart_widget().height() as f64 - y) as i32;
    let room = (side - ROOM_AROUND).clamp(MIN_ROWS_HEIGHT, MAX_ROWS_HEIGHT);
    b.append(&gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).propagate_natural_width(true).propagate_natural_height(true).max_content_height(room).child(&rows_grid(&d.rows)).build());
    // Stepping to another feature of the same spot replaces the popover: one whose size changes while it is
    // up is dismissed (as one too tall is), and the closing of the old one is not the user's.
    if let Some(open) = INFO.with(|i| i.borrow_mut().take()) {
        open.popdown();
    }
    let pop = show_at(&b, x, y);
    if let Some(app) = app {
        let app = Rc::downgrade(app);
        pop.connect_closed(move |p| {
            if !INFO.with(|i| i.borrow().as_ref() == Some(p)) {
                return;
            }
            INFO.with(|i| i.borrow_mut().take());
            if let Some(app) = app.upgrade() {
                isomar_app::probe::info_closed(&app);
            }
        });
    }
    INFO.with(|i| *i.borrow_mut() = Some(pop));
}

/// What is known of vessel `mmsi`, at the vessel; "Details" opens it in the Traffic page.
pub fn show_vessel(app: &Rc<AppState>, mmsi: u32) {
    let v = isomar_app::api::call(app, "vessel_get", &serde_json::json!({ "id": mmsi }));
    if v.is_null() {
        return;
    }
    let pos = app.session.borrow().targets.get(mmsi).and_then(|t| t.pos).unwrap_or_else(|| app.canvas.view_state().center());
    let (x, y) = app.canvas.view_info().to_screen(&pos);
    let b = column();
    heading(v.get("title").and_then(|t| t.as_str()).unwrap_or_default(), None, &b);
    let rows: Vec<(String, String)> = v
        .get("groups")
        .and_then(|g| g.as_array())
        .into_iter()
        .flatten()
        .flat_map(|g| g.get("rows").and_then(|r| r.as_array()).cloned().unwrap_or_default())
        .filter_map(|r| Some((r.get(0)?.as_str()?.to_string(), r.get(1)?.as_str()?.to_string())))
        .collect();
    b.append(&rows_grid(&rows));
    let details = gtk::Button::builder().label(gettext("Details")).halign(gtk::Align::End).margin_top(6).action_name("win.vessel-detail").action_target(&i64::from(mmsi).to_variant()).build();
    b.append(&details);
    let pop = show_at(&b, x, y);
    details.connect_clicked(move |_| pop.popdown());
}

/// The tide height curve: the predicted heights, now marked, high and low waters readable from
/// the shape.
fn curve(info: &TideInfo) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder().content_width(300).content_height(110).margin_top(6).build();
    let (pts, now) = (info.curve.clone(), info.now);
    area.set_draw_func(move |_, cr, w, h| {
        if pts.len() < 2 {
            return;
        }
        let theme = super::theme::token_theme();
        let (w, h) = (w as f64, h as f64 - 14.0);
        let (t0, t1) = (pts[0].0 as f64, pts[pts.len() - 1].0 as f64);
        let lo = pts.iter().map(|p| p.1).fold(f64::MAX, f64::min) - 0.3;
        let hi = pts.iter().map(|p| p.1).fold(f64::MIN, f64::max) + 0.3;
        let x = |t: f64| (t - t0) / (t1 - t0) * w;
        let y = |v: f64| 4.0 + (hi - v) / (hi - lo) * (h - 8.0);
        // The water, then its surface.
        let (r, g, b, _) = tokens::color(Token::ChartShallow, theme);
        cr.set_source_rgb(r, g, b);
        cr.move_to(0.0, h);
        for p in &pts {
            cr.line_to(x(p.0 as f64), y(p.1));
        }
        cr.line_to(w, h);
        cr.close_path();
        let _ = cr.fill();
        let (r, g, b, _) = tokens::color(Token::ChartContour, theme);
        cr.set_source_rgb(r, g, b);
        cr.set_line_width(1.5);
        for (i, p) in pts.iter().enumerate() {
            if i == 0 { cr.move_to(x(p.0 as f64), y(p.1)) } else { cr.line_to(x(p.0 as f64), y(p.1)) }
        }
        let _ = cr.stroke();
        // Now.
        let (r, g, b, _) = tokens::color(Token::Accent, theme);
        cr.set_source_rgb(r, g, b);
        cr.set_line_width(2.0);
        cr.move_to(x(now as f64), 0.0);
        cr.line_to(x(now as f64), h);
        let _ = cr.stroke();
        // Hours along the bottom, every six.
        let (r, g, b, _) = tokens::color(Token::TextMuted, theme);
        cr.set_source_rgb(r, g, b);
        let layout = pangocairo::functions::create_layout(cr);
        layout.set_font_description(Some(&gtk::pango::FontDescription::from_string("Sans 8")));
        let first = ((t0 / 21_600.0).ceil() * 21_600.0) as i64;
        for t in (first..t1 as i64).step_by(21_600) {
            let label = chrono::DateTime::from_timestamp(t, 0).map(|d| d.with_timezone(&chrono::Local).format("%H:%M").to_string()).unwrap_or_default();
            layout.set_text(&label);
            cr.move_to(x(t as f64) - 12.0, h + 1.0);
            pangocairo::functions::show_layout(cr, &layout);
        }
    });
    area
}

/// The tide at `pos`: a popover at the position on the chart, with the curve, the next high and low
/// waters, the source and the planning-only line.
pub fn show_tides(app: &Rc<AppState>, pos: LatLon) {
    let app = app.clone();
    super::spawn(async move {
        let Some(info) = isomar_app::flows::tides::info(&app, pos).await else { return };
        let b = column();
        heading(&info.title, Some(&gettext("Predicted tide")), &b);
        b.append(&curve(&info));
        b.append(&rows_grid(&info.station));
        if !info.extremes.is_empty() {
            b.append(&gtk::Label::builder().label(&info.next_title).xalign(0.0).css_classes(["caption-heading", "dim-label"]).margin_top(6).build());
            b.append(&extremes_grid(&info.extremes));
        }
        for (title, rows) in &info.groups {
            if !title.is_empty() {
                b.append(&gtk::Label::builder().label(title).xalign(0.0).css_classes(["caption-heading", "dim-label"]).margin_top(6).build());
            }
            b.append(&rows_grid(rows));
        }
        let (x, y) = app.canvas.view_info().to_screen(&pos);
        show_at(&b, x, y);
    });
}
