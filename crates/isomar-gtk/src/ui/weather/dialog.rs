//! "Get weather" (`design/screens/flows-gnome-weather-2.png`): a card over the chart for the area
//! drawn as a box on it, with how far ahead, how often and what to fetch, and what that costs.
//! Download runs `isomar_app::flows::weather::run_download` as a job.

use crate::core::geo;
use crate::core::weather::planner::{Area, Source, plan};
use crate::i18n::{gettext, gettext_f};
use crate::ui::app::AppState;
use crate::ui::spawn;
use adw::prelude::*;
use isomar_app::chips::{Chip, Icon};
use isomar_app::flows::weather::{Choices, GROUPS, HORIZONS};
use chrono::Utc;
use std::cell::RefCell;
use std::rc::Rc;

thread_local! {
    /// Where the card goes: the task slot over the chart.
    static SLOT: RefCell<Option<gtk::Box>> = const { RefCell::new(None) };
    static CARD: RefCell<Option<gtk::Box>> = const { RefCell::new(None) };
}

/// Tells the dialog where over the chart its card goes.
pub fn install(slot: &gtk::Box) {
    SLOT.with(|s| *s.borrow_mut() = Some(slot.clone()));
}

fn close(app: &AppState) {
    if let Some(c) = CARD.with(|c| c.borrow_mut().take())
        && let Some(p) = c.parent().and_downcast::<gtk::Box>()
    {
        p.remove(&c);
    }
    {
        let mut w = app.weather.borrow_mut();
        w.pick = isomar_app::weather::Pick::Off;
        w.rect = None;
    }
    app.refresh();
}

/// Opens the card for `area`, which is drawn on the chart while the card is open.
pub fn open(app: &Rc<AppState>, area: Area) {
    let Some(slot) = SLOT.with(|s| s.borrow().clone()) else { return };
    if let Some(old) = CARD.with(|c| c.borrow_mut().take()) {
        slot.remove(&old);
    }
    app.weather.borrow_mut().rect = Some((crate::core::geo::LatLon::new(area.south, area.west), crate::core::geo::LatLon::new(area.north, area.east)));
    app.canvas.queue_draw();

    let card = crate::ui::overlays::card(gtk::Orientation::Vertical, 8);
    card.add_css_class("weather-card");
    card.set_width_request(340);
    card.set_halign(gtk::Align::Start);
    card.append(&gtk::Label::builder().label(gettext("Get weather for this area")).xalign(0.0).css_classes(["heading"]).build());
    let redraw = gtk::Button::builder().label(gettext("Draw another box on the chart")).css_classes(["flat", "link"]).halign(gtk::Align::Start).build();
    card.append(&redraw);

    let grid = gtk::Grid::builder().column_spacing(12).row_spacing(6).build();
    let row = |i: i32, label: &str, w: &gtk::Widget| {
        grid.attach(&gtk::Label::builder().label(label).xalign(0.0).css_classes(["dim-label"]).build(), 0, i, 1, 1);
        grid.attach(w, 1, i, 1, 1);
    };
    let labels: Vec<&str> = Source::ALL.iter().map(|s| s.label()).collect();
    let model = gtk::DropDown::from_strings(&labels);
    model.set_selected(Source::ALL.iter().position(|s| *s == app.weather.borrow().source).unwrap_or(0) as u32);
    model.set_halign(gtk::Align::Start);
    let about = gtk::Label::builder().xalign(0.0).wrap(true).max_width_chars(40).css_classes(["caption", "dim-label"]).build();
    let horizons: Vec<String> = HORIZONS.iter().map(|(_, l)| gettext(*l)).collect();
    let horizon = gtk::DropDown::from_strings(&horizons.iter().map(String::as_str).collect::<Vec<_>>());
    horizon.set_selected(2);
    horizon.set_halign(gtk::Align::Start);
    let steps = [gettext("Every hour"), gettext("Every 3 hours"), gettext("Every 6 hours")];
    let step = gtk::DropDown::from_strings(&steps.iter().map(String::as_str).collect::<Vec<_>>());
    step.set_selected(1);
    step.set_halign(gtk::Align::Start);
    let chips = adw::WrapBox::builder().child_spacing(4).line_spacing(4).build();
    let toggles: Vec<(crate::core::weather::model::Group, gtk::ToggleButton)> = GROUPS
        .iter()
        .map(|(g, label, on)| {
            let t = gtk::ToggleButton::builder().label(gettext(*label)).active(*on).css_classes(["chip"]).build();
            chips.append(&t);
            (*g, t)
        })
        .collect();
    row(0, &gettext("Model"), model.upcast_ref());
    grid.attach(&about, 1, 1, 1, 1);
    row(2, &gettext("Length"), horizon.upcast_ref());
    row(3, &gettext("Step"), step.upcast_ref());
    row(4, &gettext("Fields"), chips.upcast_ref());
    card.append(&grid);
    let estimate = Rc::new(crate::ui::chips::ChipLine::wrapping(&["caption", "dim-label"]));
    card.append(&estimate.widget);
    card.append(&gtk::Label::builder().label(gettext("Kept for offline use.")).xalign(0.0).css_classes(["caption", "dim-label"]).build());

    let buttons = gtk::Box::builder().spacing(8).halign(gtk::Align::End).build();
    let cancel = gtk::Button::with_label(&gettext("Cancel"));
    let download = gtk::Button::builder().label(gettext("Download")).css_classes(["suggested-action"]).build();
    buttons.append(&cancel);
    buttons.append(&download);
    card.append(&buttons);

    let choices = {
        let (horizon, step, toggles, model) = (horizon.clone(), step.clone(), toggles.clone(), model.clone());
        Rc::new(move || Choices {
            source: Source::ALL[(model.selected() as usize).min(Source::ALL.len() - 1)],
            north: geo::format_lat(area.north),
            south: geo::format_lat(area.south),
            west: geo::format_lon(area.west),
            east: geo::format_lon(area.east),
            horizon_index: horizon.selected() as usize,
            step_index: step.selected() as usize,
            groups: toggles.iter().filter(|(_, t)| t.is_active()).map(|(g, _)| *g).collect(),
        })
    };
    let update = {
        let (choices, estimate, download, about, app) = (choices.clone(), estimate.clone(), download.clone(), about.clone(), Rc::downgrade(app));
        Rc::new(move || {
            let c = choices();
            if let Some(app) = app.upgrade() {
                let models = isomar_app::flows::weather::model_choices(&app);
                about.set_label(&models.iter().find(|m| m.id == c.source.id()).map(|m| m.about.clone()).unwrap_or_default());
            }
            let req = c.request();
            let e = isomar_app::flows::weather::estimate(req.as_ref());
            let ok = req.as_ref().is_some_and(|r| !plan(r, r.source.candidate_runs(Utc::now())[0], Utc::now()).is_empty());
            let mid = ((area.north + area.south) / 2.0).to_radians().cos();
            let (w_nm, h_nm) = ((area.east - area.west).abs() * 60.0 * mid, (area.north - area.south).abs() * 60.0);
            let area = gettext_f("{w} × {h} nm", &[("w", &format!("{w_nm:.0}")), ("h", &format!("{h_nm:.0}"))]);
            let mut chips = vec![
                Chip::with(Icon::Forecast, gettext_f("{source}, run {run}", &[("source", &e.source), ("run", &e.run)])),
                Chip::new(Icon::Size, e.size.clone(), gettext_f("Download size {size}", &[("size", &e.size)])),
                Chip::new(Icon::Distance, area.clone(), gettext_f("Area {area}", &[("area", &area)])),
            ];
            if !e.missing.is_empty() {
                chips.push(Chip::plain(e.missing.clone()));
            }
            estimate.set(&chips);
            download.set_sensitive(ok && !crate::ui::debug::offline());
            download.set_label(&if e.ok { gettext_f("Download {size}", &[("size", e.size.trim_start_matches("≈ "))]) } else { gettext("Download") });
        })
    };
    update();
    {
        let u = update.clone();
        model.connect_selected_notify(move |_| u());
        let u = update.clone();
        horizon.connect_selected_notify(move |_| u());
        let u = update.clone();
        step.connect_selected_notify(move |_| u());
        for (_, t) in &toggles {
            let u = update.clone();
            t.connect_toggled(move |_| u());
        }
    }
    {
        let app = app.clone();
        redraw.connect_clicked(move |_| isomar_app::weather::start_pick(&app));
    }
    {
        let app = Rc::downgrade(app);
        cancel.connect_clicked(move |_| {
            if let Some(app) = app.upgrade() {
                close(&app);
            }
        });
    }
    {
        let app = Rc::downgrade(app);
        download.connect_clicked(move |_| {
            let Some(app) = app.upgrade() else { return };
            let Some(req) = choices().request() else { return };
            close(&app);
            spawn(async move {
                isomar_app::flows::weather::run_download(&app, req).await;
            });
        });
    }
    slot.append(&card);
    CARD.with(|c| *c.borrow_mut() = Some(card));
    if std::env::var_os("ISOMAR_AUTO_ACCEPT").is_some() && download.is_sensitive() {
        download.emit_clicked();
    }
}
