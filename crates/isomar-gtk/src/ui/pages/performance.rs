//! The Performance page, pushed from Instruments: what the performance log holds, what it says about the
//! instruments, the polar learnt from them, how fast the boat sails against its polar, how good the
//! forecast was, waves, and passages planned against sailed. Built from `vm::performance`, again whenever
//! that record changes; the actions go through `isomar_app::api`.

use super::scrolled;
use crate::i18n::{gettext, gettext_f};
use crate::ui::app::AppState;
use crate::ui::{show_error, spawn};
use adw::prelude::*;
use isomar_app::vm::performance::{Curve, LearnedView, Performance};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::f64::consts::PI;
use std::rc::{Rc, Weak};

struct Shown {
    app: Weak<AppState>,
    content: gtk::Box,
    last: RefCell<Option<Performance>>,
    /// The wind-speed column the diagram shows.
    col: Cell<usize>,
}

thread_local! {
    static SHOWN: RefCell<Weak<Shown>> = const { RefCell::new(Weak::new()) };
    static HOOKED: Cell<bool> = const { Cell::new(false) };
}

fn call(app: &Rc<AppState>, name: &str, args: Value) -> Value {
    isomar_app::api::call(app, name, &args)
}

/// Shows the error an answer carries; true when there was none.
fn ok(answer: &Value, heading: &str) -> bool {
    match answer.get("error").and_then(|e| e.as_str()) {
        Some(e) => {
            show_error(heading, e);
            false
        }
        None => true,
    }
}

pub fn page(app: &Rc<AppState>) -> adw::NavigationPage {
    let content = super::column();
    let shown = Rc::new(Shown { app: Rc::downgrade(app), content: content.clone(), last: RefCell::new(None), col: Cell::new(2) });
    SHOWN.with(|s| *s.borrow_mut() = Rc::downgrade(&shown));
    if !HOOKED.get() {
        HOOKED.set(true);
        app.on_refresh(|| {
            // Rebuilt after the handler that changed things has returned, never inside it.
            gtk::glib::idle_add_local_once(update);
        });
    }
    // Asking for the record starts an analysis when the log is newer than the last one.
    call(app, "perf_get", Value::Null);
    update();

    let tv = adw::ToolbarView::new();
    let header = adw::HeaderBar::builder().show_end_title_buttons(false).show_start_title_buttons(false).build();
    let again = crate::ui::busy::icon(app, "perf_analyse", "view-refresh-symbolic", &gettext("Analyse again"));
    {
        let app = Rc::downgrade(app);
        again.connect_clicked(move |b| {
            if let Some(app) = app.upgrade()
                && !crate::ui::busy::pressed(&app, b, "perf_analyse")
            {
                call(&app, "perf_analyse", Value::Null);
            }
        });
    }
    header.pack_end(&again);
    tv.add_top_bar(&header);
    tv.set_content(Some(&scrolled(&content)));
    let nav = adw::NavigationPage::builder().title(gettext("Performance")).tag("performance").child(&tv).build();
    nav.connect_hidden(move |_| {
        let _ = &shown;
    });
    nav
}

/// Builds the page again when its record changed.
fn update() {
    let Some(shown) = SHOWN.with(|s| s.borrow().upgrade()) else { return };
    let Some(app) = shown.app.upgrade() else { return };
    let p = isomar_app::vm::performance::performance(&app);
    if shown.last.borrow().as_ref() == Some(&p) {
        return;
    }
    super::clear(&shown.content);
    fill(&app, &shown, &p);
    *shown.last.borrow_mut() = Some(p);
}

fn group(title: &str, description: Option<&str>) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::builder().title(title).build();
    if let Some(d) = description {
        g.set_description(Some(d));
    }
    g
}

fn line(title: &str, subtitle: &str) -> adw::ActionRow {
    let r = adw::ActionRow::builder().title(title).subtitle(subtitle).build();
    r.set_title_lines(0);
    r.set_subtitle_lines(0);
    r
}

fn pill(label: &str, suggested: bool) -> gtk::Button {
    let b = gtk::Button::builder().label(label).halign(gtk::Align::Start).build();
    if suggested {
        b.add_css_class("suggested-action");
    }
    b
}

fn on_click(b: &gtk::Button, app: &Rc<AppState>, f: impl Fn(&Rc<AppState>) + 'static) {
    let app = Rc::downgrade(app);
    b.connect_clicked(move |_| {
        if let Some(app) = app.upgrade() {
            f(&app);
        }
    });
}

fn fill(app: &Rc<AppState>, shown: &Rc<Shown>, p: &Performance) {
    let page = &shown.content;
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let summary = gtk::Label::builder().label(&p.summary).wrap(true).xalign(0.0).hexpand(true).css_classes(["dim-label"]).build();
    head.append(&summary);
    if p.analysing {
        head.append(&adw::Spinner::new());
    }
    page.append(&head);
    // The log is the polar in use's: it is measured against it and learns into it alone.
    let which = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    which.append(&gtk::Label::builder().label(gettext_f("Logged against the polar {name}", &[("name", &p.polar)])).wrap(true).xalign(0.0).hexpand(true).css_classes(["caption"]).build());
    which.append(&gtk::Button::builder().label(gettext("Change…")).action_name("win.polar").valign(gtk::Align::Center).css_classes(["flat"]).build());
    page.append(&which);
    if let Some(e) = &p.error {
        page.append(&super::placeholder(&gettext_f("The log could not be read: {error}", &[("error", e)])));
    }

    // The log.
    let log = group(&gettext("Log"), Some(&gettext("Every 10 seconds under way: the instruments, the engine, and the forecast's wind and waves there.")));
    let rec = adw::SwitchRow::builder().title(gettext("Record while under way")).active(p.recording).build();
    {
        let app = Rc::downgrade(app);
        rec.connect_active_notify(move |r| {
            if let Some(app) = app.upgrade() {
                call(&app, "perf_set_recording", json!({ "on": r.is_active() }));
            }
        });
    }
    log.add(&rec);
    for l in &p.hours {
        let r = adw::ActionRow::builder().title(&l.title).build();
        r.add_suffix(&gtk::Label::builder().label(&l.detail).css_classes(["dim-label", "numeric"]).build());
        log.add(&r);
    }
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_margin_top(8);
    let export = gtk::Button::with_label(&gettext("Export…"));
    on_click(&export, app, export_csv);
    let clear = gtk::Button::with_label(&gettext("Clear…"));
    clear.add_css_class("destructive-action");
    on_click(&clear, app, |app| {
        let app = app.clone();
        spawn(async move {
            if crate::ui::confirm(&gettext("Clear the performance log?"), &gettext("Every sample logged against the polar in use, and every passage, is deleted. The polars and the calibration are kept."), &gettext("Clear"), true).await {
                call(&app, "perf_clear", Value::Null);
            }
        });
    });
    buttons.append(&export);
    if !p.empty {
        buttons.append(&clear);
    }
    log.add(&buttons);
    page.append(&log);
    if p.empty {
        return;
    }

    // Instruments.
    if let Some(c) = &p.calibration {
        let g = group(&gettext("Instruments"), Some(&c.evidence));
        for f in &c.findings {
            g.add(&line(&f.title, &f.detail));
        }
        let b = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        b.set_margin_top(8);
        if c.can_save {
            let save = pill(&gettext("Correct the analysis with this"), false);
            on_click(&save, app, |app| {
                ok(&call(app, "perf_calibration_save", Value::Null), &gettext("Could not save the calibration"));
            });
            b.append(&save);
        }
        if c.saved {
            let forget = pill(&gettext("Forget the saved correction"), false);
            on_click(&forget, app, |app| {
                call(app, "perf_calibration_clear", Value::Null);
            });
            b.append(&forget);
        }
        g.add(&b);
        g.add(&super::placeholder(&gettext("The correction applies to this analysis only; it does not change what the instruments show.")));
        page.append(&g);
    }

    // The learnt polar.
    if let Some(l) = &p.learned {
        page.append(&learned_group(app, shown, l, p.can_undo_polar));
    }

    // Performance.
    if p.suggestion.is_some() || !p.factors.is_empty() {
        let g = group(&gettext("Performance"), None);
        if let Some(s) = &p.suggestion {
            g.set_description(Some(&s.text));
            if s.applied {
                g.add(&line(&gettext("Weather routing starts from this"), &gettext_f("{pct}% of the polar", &[("pct", &format!("{:.0}", s.pct))])));
            } else {
                let b = pill(&s.button, true);
                let pct = s.pct;
                on_click(&b, app, move |app| {
                    call(app, "perf_apply_factor", json!({ "pct": pct }));
                    app.toast(&gettext("Weather routing will start from this performance"));
                });
                g.add(&b);
            }
        }
        for f in &p.factors {
            let e = adw::ExpanderRow::builder().title(&f.title).subtitle(&f.note).build();
            e.set_subtitle_lines(0);
            for r in &f.rows {
                let row = adw::ActionRow::builder().title(&r.label).subtitle(&r.text).build();
                e.add_row(&row);
            }
            g.add(&e);
        }
        page.append(&g);
    }

    // The forecast against the instruments.
    if !p.forecast_check.is_empty() || p.hindcast_pending > 0 {
        let g = group(&gettext("Forecast against the instruments"), Some(&gettext("The model's wind minus what the instruments measured.")));
        for l in &p.forecast_check {
            g.add(&line(&l.title, &l.detail));
        }
        if p.hindcast_pending > 0 {
            let (b, _) = crate::ui::busy::button(app, "perf_backfill", &gettext("Fetch the wind as analysed afterwards"));
            b.set_halign(gtk::Align::Start);
            b.set_tooltip_text(Some(&gettext_f("For {n} samples of the last nine days: the model's analysis, closer to the wind that blew than the forecast was", &[("n", &p.hindcast_pending.to_string())])));
            let b2 = b.clone();
            on_click(&b, app, move |app| {
                if !crate::ui::busy::pressed(app, &b2, "perf_backfill") {
                    call(app, "perf_backfill", Value::Null);
                }
            });
            g.add(&b);
        }
        page.append(&g);
    }

    if !p.waves.is_empty() {
        let g = group(&gettext("Waves"), Some(&gettext("Speed against the polar by the forecast's wave height. Not used by weather routing yet.")));
        for l in &p.waves {
            g.add(&line(&l.title, &l.detail));
        }
        page.append(&g);
    }

    if !p.passages.is_empty() {
        let g = group(&gettext("Passages"), Some(&gettext("Weather routes followed to the end, against their plan.")));
        for l in &p.passages {
            g.add(&line(&l.title, &l.detail));
        }
        page.append(&g);
    }
}

fn learned_group(app: &Rc<AppState>, shown: &Rc<Shown>, l: &LearnedView, can_undo: bool) -> adw::PreferencesGroup {
    let g = group(&gettext("Polar"), Some(&l.summary));
    let toggles = adw::ToggleGroup::builder().homogeneous(true).halign(gtk::Align::Center).build();
    for (i, w) in l.columns.iter().enumerate() {
        toggles.add(adw::Toggle::builder().name(i.to_string()).label(gettext_f("{n} kn", &[("n", &format!("{w:.0}"))])).build());
    }
    let col = shown.col.get().min(l.columns.len().saturating_sub(1));
    toggles.set_active(col as u32);
    let curves = Rc::new(l.curves.clone());
    let area = gtk::DrawingArea::builder().content_height(300).hexpand(true).build();
    {
        let (curves, shown) = (curves.clone(), Rc::downgrade(shown));
        area.set_draw_func(move |a, cr, w, h| {
            let col = shown.upgrade().map_or(0, |s| s.col.get());
            if let Some(c) = curves.get(col) {
                draw_curve(a, cr, w as f64, h as f64, c);
            }
        });
    }
    let best = gtk::Label::builder().wrap(true).xalign(0.0).css_classes(["caption", "dim-label"]).build();
    best.set_text(curves.get(col).and_then(|c| c.best.clone()).as_deref().unwrap_or(""));
    {
        let (area, best, curves, shown) = (area.downgrade(), best.downgrade(), curves.clone(), Rc::downgrade(shown));
        toggles.connect_active_notify(move |t| {
            let Some(shown) = shown.upgrade() else { return };
            shown.col.set(t.active() as usize);
            if let Some(b) = best.upgrade() {
                b.set_text(curves.get(t.active() as usize).and_then(|c| c.best.clone()).as_deref().unwrap_or(""));
            }
            if let Some(a) = area.upgrade() {
                a.queue_draw();
            }
        });
    }
    let legend = gtk::Label::builder()
        .label(gettext("Grey: the boat's polar now. Blue: as learnt. Dots: what the boat did (the top of each 5° bin), the bolder the more there is."))
        .wrap(true)
        .xalign(0.0)
        .css_classes(["caption", "dim-label"])
        .build();
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    b.set_margin_top(4);
    b.append(&toggles);
    b.append(&area);
    b.append(&best);
    b.append(&legend);

    let coverage_title = gtk::Label::builder().label(gettext("Coverage: where the log has data")).xalign(0.0).css_classes(["caption-heading"]).margin_top(8).build();
    let grid = gtk::DrawingArea::builder().content_height(20 + 14 * l.columns.len() as i32).hexpand(true).build();
    {
        let (coverage, columns) = (l.coverage.clone(), l.columns.clone());
        grid.set_draw_func(move |a, cr, w, h| draw_coverage(a, cr, w as f64, h as f64, &coverage, &columns));
    }
    b.append(&coverage_title);
    b.append(&grid);

    if let Some(c) = &l.caution {
        let warn = gtk::Label::builder().label(c).wrap(true).xalign(0.0).css_classes(["caption", "warning"]).margin_top(8).build();
        b.append(&warn);
    }
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_margin_top(8);
    if l.changes > 0 {
        let apply = pill(&gettext_f("Use the learnt values ({n} cells)", &[("n", &l.changes.to_string())]), l.caution.is_none());
        apply.set_tooltip_text(Some(&gettext("Where the log has enough data; the rest of the polar stays as it is. The polar before is kept, to go back to.")));
        on_click(&apply, app, |app| {
            let r = call(app, "perf_apply_polar", Value::Null);
            if ok(&r, &gettext("Could not change the polar")) {
                let n = r.get("changed").and_then(|n| n.as_u64()).unwrap_or(0);
                app.toast(&gettext_f("{n} speeds of the polar changed", &[("n", &n.to_string())]));
            }
        });
        actions.append(&apply);
    }
    let edit = pill(&gettext("Open the polar editor"), false);
    edit.set_action_name(Some("win.polar"));
    actions.append(&edit);
    if can_undo {
        let undo = pill(&gettext("Back to the polar before"), false);
        on_click(&undo, app, |app| {
            if ok(&call(app, "perf_undo_polar", Value::Null), &gettext("Could not go back")) {
                app.toast(&gettext("The polar before is back"));
            }
        });
        actions.append(&undo);
    }
    b.append(&actions);
    g.add(&b);
    g
}

/// Half a polar diagram, starboard side: the wind from the top, speed outwards.
fn draw_curve(a: &gtk::DrawingArea, cr: &cairo::Context, w: f64, h: f64, c: &Curve) {
    let fg = a.color();
    let accent = adw::StyleManager::default().accent_color_rgba();
    let ink = |al: f64| cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, al);
    let tint = |al: f64| cr.set_source_rgba(accent.red() as f64, accent.green() as f64, accent.blue() as f64, al);
    let max = c.current.iter().chain(&c.learned).map(|p| p.1).chain(c.cells.iter().map(|x| x.1)).fold(1.0f64, f64::max);
    let step = if max < 11.0 { 2.0 } else { 5.0 };
    let scale = ((max + 0.5) / step).ceil() * step;
    let r = (h / 2.0 - 12.0).min(w * 0.75).max(10.0);
    // The half disc in the middle of the width.
    let (cx, cy) = (((w - r) / 2.0).max(4.0), h / 2.0);
    let at = |twa: f64, kn: f64| {
        let rr = kn / scale * r;
        (cx + rr * twa.to_radians().sin(), cy - rr * twa.to_radians().cos())
    };
    cr.set_line_width(1.0);
    cr.set_font_size(11.0);
    let mut k = step;
    while k <= scale + 1e-9 {
        ink(0.15);
        cr.arc(cx, cy, k / scale * r, -PI / 2.0, PI / 2.0);
        let _ = cr.stroke();
        ink(0.5);
        cr.move_to(cx + 3.0, cy - k / scale * r + 12.0);
        let _ = cr.show_text(&format!("{k:.0}"));
        k += step;
    }
    for deg in [30.0f64, 60.0, 90.0, 120.0, 150.0] {
        ink(0.1);
        cr.move_to(cx, cy);
        let (x, y) = at(deg, scale);
        cr.line_to(x, y);
        let _ = cr.stroke();
        ink(0.5);
        let (x, y) = at(deg, scale * 1.06);
        cr.move_to(x - 8.0, y + 4.0);
        let _ = cr.show_text(&format!("{deg:.0}°"));
    }
    let path = |pts: &[(f64, f64)]| {
        for (i, (twa, kn)) in pts.iter().enumerate() {
            let (x, y) = at(*twa, *kn);
            if i == 0 {
                cr.move_to(x, y);
            } else {
                cr.line_to(x, y);
            }
        }
    };
    ink(0.45);
    cr.set_line_width(1.5);
    path(&c.current);
    let _ = cr.stroke();
    for (twa, kn, conf) in &c.cells {
        let (x, y) = at(*twa, *kn);
        cr.arc(x, y, 3.0, 0.0, 2.0 * PI);
        tint(0.15 + 0.6 * conf);
        let _ = cr.fill();
    }
    tint(1.0);
    cr.set_line_width(2.5);
    cr.set_line_join(cairo::LineJoin::Round);
    path(&c.learned);
    let _ = cr.stroke();
}

/// A grid of wind speeds (rows) by 5 degree bins of angle (columns), tinted by how much the log has there.
fn draw_coverage(a: &gtk::DrawingArea, cr: &cairo::Context, w: f64, h: f64, coverage: &[Vec<f64>], columns: &[f64]) {
    let fg = a.color();
    let accent = adw::StyleManager::default().accent_color_rgba();
    let left = 44.0;
    let rows = coverage.len().max(1) as f64;
    let bins = coverage.first().map_or(36, |c| c.len()) as f64;
    let cw = (w - left) / bins;
    let ch = (h - 16.0) / rows;
    cr.set_font_size(10.0);
    for (j, row) in coverage.iter().enumerate() {
        let y = j as f64 * ch;
        cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.6);
        cr.move_to(0.0, y + ch * 0.75);
        let _ = cr.show_text(&gettext_f("{n} kn", &[("n", &format!("{:.0}", columns.get(j).copied().unwrap_or(0.0)))]));
        for (i, c) in row.iter().enumerate() {
            cr.rectangle(left + i as f64 * cw, y + 1.0, cw - 1.0, ch - 2.0);
            if *c > 0.0 {
                cr.set_source_rgba(accent.red() as f64, accent.green() as f64, accent.blue() as f64, 0.15 + 0.85 * c);
            } else {
                cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.06);
            }
            let _ = cr.fill();
        }
    }
    cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.6);
    for deg in [0.0, 45.0, 90.0, 135.0, 180.0] {
        let x = left + deg / 180.0 * (w - left);
        cr.move_to((x - 8.0).min(w - 24.0), h - 2.0);
        let _ = cr.show_text(&format!("{deg:.0}°"));
    }
}

fn export_csv(app: &Rc<AppState>) {
    let app = app.clone();
    spawn(async move {
        let window = crate::ui::chart_widget().root().and_then(|r| r.downcast::<gtk::Window>().ok());
        let dialog = gtk::FileDialog::builder().title(gettext("Export the performance log")).initial_name("performance-log.csv").modal(false).build();
        let Ok(file) = dialog.save_future(window.as_ref()).await else { return };
        let Some(path) = file.path() else { return };
        let r = call(&app, "perf_export_csv", json!({ "path": path.to_string_lossy() }));
        if ok(&r, &gettext("Could not write the file")) {
            app.toast(&gettext_f("{n} samples exported", &[("n", &r.get("rows").and_then(|n| n.as_u64()).unwrap_or(0).to_string())]));
        }
    });
}
