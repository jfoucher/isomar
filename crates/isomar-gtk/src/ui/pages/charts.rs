//! The Charts page ("Charts…" and "Charts for this area" merged): the sources that cover the
//! centre of the chart, the folders of charts the user has licensed, and what is loaded. Built
//! from `vm::charts` and `flows::charts`.

use super::{clear, placeholder, row_button, scrolled};
use crate::i18n::gettext;
use crate::ui::app::AppState;
use adw::prelude::*;
use isomar_app::flows::charts as flow;
use isomar_app::vm::charts::Page;
use std::cell::RefCell;
use std::rc::Rc;

fn action_label(a: &str) -> String {
    match a {
        "install" => gettext("Install"),
        "installing" => gettext("Installing…"),
        "installed" => gettext("Installed"),
        "update" => gettext("Update"),
        "on" => gettext("On"),
        "turn_on" => gettext("Turn on"),
        _ => gettext("Open website"),
    }
}

pub fn build(app: &Rc<AppState>) -> gtk::Widget {
    let page = super::column();
    let warning = adw::Banner::builder().title(gettext("Planning only here: no official chart you added covers this area.")).revealed(false).build();
    warning.add_css_class("warning");
    page.append(&warning);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 18);
    page.append(&body);

    let shown: RefCell<Option<Page>> = RefCell::new(None);
    let refresh = {
        let app = Rc::downgrade(app);
        move || {
            let Some(app) = app.upgrade() else { return };
            let p = isomar_app::vm::charts::page(&app);
            if shown.borrow().as_ref() == Some(&p) {
                return;
            }
            warning.set_revealed(p.planning_only);
            clear(&body);

            let in_use = adw::PreferencesGroup::builder().title(gettext("In use here")).build();
            if p.in_use.is_empty() {
                in_use.add(&placeholder(&if p.vector + p.rasters == 0 { gettext("No charts loaded.") } else { gettext("None of the loaded charts is drawn here at this zoom.") }));
            }
            for c in &p.in_use {
                let mut line = vec![isomar_app::chips::Chip::plain(c.kind.clone())];
                line.extend(c.chips.iter().cloned());
                let row = crate::ui::chips::row_lines(None, &c.name, &[&line], true);
                if !c.id.is_empty() {
                    let pin = gtk::ToggleButton::builder()
                        .icon_name("view-pin-symbolic")
                        .active(c.pinned)
                        .valign(gtk::Align::Center)
                        .css_classes(["flat"])
                        .tooltip_text(if c.pinned { gettext("Show all charts (unpin)") } else { gettext("Show only this chart") })
                        .build();
                    let (app2, id, was_pinned) = (app.clone(), c.id.clone(), c.pinned);
                    pin.connect_toggled(move |_| {
                        if let Some(enc) = app2.enc.borrow().as_ref() {
                            enc.set_pinned_cell(if was_pinned { None } else { Some(id.clone()) });
                        }
                        app2.refresh();
                    });
                    row.add_suffix(&pin);
                }
                in_use.add(&row);
            }
            body.append(&in_use);

            let filter_group = adw::PreferencesGroup::builder().title(gettext("Chart scale filter")).build();
            let options = [
                gettext("Auto (All scales)"),
                gettext("Up to Coastal (Band 3)"),
                gettext("Up to Approach (Band 4)"),
                gettext("Up to Harbour (Band 5)"),
                gettext("All details (Band 6)"),
            ];
            let labels: Vec<&str> = options.iter().map(|l| l.as_str()).collect();
            let combo = adw::ComboRow::builder()
                .title(gettext("Maximum detail"))
                .subtitle(gettext("Limits S-57 charts to this scale band and coarser"))
                .model(&gtk::StringList::new(&labels))
                .build();
            let current_idx = match p.enc_max_band {
                Some(3) => 1,
                Some(4) => 2,
                Some(5) => 3,
                Some(6) => 4,
                _ => 0,
            };
            combo.set_selected(current_idx);
            {
                let app = app.clone();
                combo.connect_selected_notify(move |r| {
                    let band = match r.selected() {
                        1 => Some(3),
                        2 => Some(4),
                        3 => Some(5),
                        4 => Some(6),
                        _ => None,
                    };
                    let v = match band {
                        Some(b) => serde_json::json!(b),
                        None => serde_json::Value::Null,
                    };
                    isomar_app::api::call(&app, "settings_patch", &serde_json::json!({ "enc_max_band": v }));
                });
            }
            filter_group.add(&combo);
            body.append(&filter_group);

            // Where a chart already covers the view, only what was downloaded is listed, to update or remove.
            let here = if p.planning_only {
                adw::PreferencesGroup::builder().title(gettext("For this area")).build()
            } else {
                adw::PreferencesGroup::builder().title(gettext("Downloaded for this area")).build()
            };
            here.set_visible(p.planning_only || !p.here.is_empty());
            if p.here.is_empty() {
                here.add(&placeholder(&gettext("No chart source is known for this region.")));
            }
            for s in &p.here {
                let badge = gtk::Label::builder().label(&s.rating).valign(gtk::Align::Center).css_classes(["rating-badge"]).tooltip_text(&s.rating_text).build();
                let mut line = vec![isomar_app::chips::Chip::plain(s.rating_text.clone())];
                line.extend(s.chips.iter().cloned());
                let row = crate::ui::chips::row_full(Some(badge.upcast_ref()), &s.name, &[&line], &s.note, true);
                if s.installed {
                    let remove = super::row_button("user-trash-symbolic", &gettext("Remove these charts"));
                    let (app2, id) = (app.clone(), s.id.clone());
                    remove.connect_clicked(move |_| {
                        if let Some(e) = flow::entry(&id) {
                            let app = app2.clone();
                            crate::ui::spawn(async move { flow::uninstall_entry(&app, e).await });
                        }
                    });
                    row.add_suffix(&remove);
                }
                // While its install runs it is off, with a spinner, and pressing it shows the running jobs.
                let key = format!("chart:{}", s.id);
                let (b, _) = crate::ui::busy::button(&app, &key, &action_label(&s.action));
                b.set_valign(gtk::Align::Center);
                b.set_sensitive(!["on", "installed"].contains(&s.action.as_str()));
                if s.action == "install" || s.action == "update" {
                    b.add_css_class("suggested-action");
                }
                let (app2, s2) = (app.clone(), s.clone());
                b.connect_clicked(move |b| match s2.action.as_str() {
                    _ if crate::ui::busy::pressed(&app2, b, &key) => {}
                    "installing" => {}
                    "install" | "update" => {
                        if let Some(e) = flow::entry(&s2.id) {
                            let app = app2.clone();
                            crate::ui::spawn(async move {
                                flow::install_entry(&app, e).await;
                            });
                        }
                    }
                    "turn_on" => {
                        flow::turn_service_on(&app2);
                        app2.refresh();
                    }
                    _ => gtk::UriLauncher::new(&s2.url).launch(crate::ui::chart_widget().root().and_downcast_ref::<gtk::Window>(), gtk::gio::Cancellable::NONE, |_| {}),
                });
                row.add_suffix(&b);
                here.add(&row);
            }
            body.append(&here);

            let add = gtk::Button::builder().child(&adw::ButtonContent::builder().icon_name("folder-new-symbolic").label(gettext("Add…")).build()).css_classes(["flat"]).build();
            {
                let app = app.clone();
                add.connect_clicked(move |_| add_folder(&app));
            }
            let folders = adw::PreferencesGroup::builder()
                .title(gettext("Your chart folders"))
                .header_suffix(&add)
                .build();
            for f in &p.folders {
                let row = adw::ActionRow::builder().title(f).title_lines(1).build();
                row.add_prefix(&gtk::Image::from_icon_name("folder-symbolic"));
                let forget = row_button("user-trash-symbolic", &gettext("Forget this folder"));
                let (app2, f2) = (app.clone(), f.clone());
                forget.connect_clicked(move |_| flow::remove_folder(&app2, &f2));
                row.add_suffix(&forget);
                folders.add(&row);
            }
            body.append(&folders);

            let loaded = adw::PreferencesGroup::builder().title(gettext("Loaded charts")).build();
            let count = |title: String, n: usize| adw::ActionRow::builder().title(title).subtitle(n.to_string()).css_classes(["property"]).build();
            loaded.add(&count(gettext("Vector charts (S-57)"), p.vector));
            loaded.add(&count(gettext("Raster charts (BSB/KAP)"), p.rasters));
            body.append(&loaded);
            *shown.borrow_mut() = Some(p);
        }
    };
    refresh();
    let refresh = Rc::new(refresh);
    {
        let r = refresh.clone();
        app.on_refresh(move || r());
    }
    let scrolled = scrolled(&page);
    {
        // The sources follow the centre of the chart: once the view settles, while the page shows.
        let pending: Rc<std::cell::Cell<Option<gtk::glib::SourceId>>> = Rc::default();
        let w = scrolled.downgrade();
        app.canvas.connect_view_changed(move |_| {
            if let Some(id) = pending.take() {
                id.remove();
            }
            let (r, w, p) = (refresh.clone(), w.clone(), pending.clone());
            pending.set(Some(gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
                p.set(None);
                if w.upgrade().is_some_and(|w| w.is_mapped()) {
                    r();
                }
            })));
        });
    }
    scrolled.upcast()
}

fn add_folder(app: &Rc<AppState>) {
    let app = app.clone();
    crate::ui::spawn(async move {
        let dialog = gtk::FileDialog::builder().title(gettext("Add a chart folder")).modal(false).build();
        let parent = crate::ui::chart_widget().root().and_downcast::<gtk::Window>();
        let Ok(folder) = dialog.select_folder_future(parent.as_ref()).await else { return };
        if let Some(path) = folder.path() {
            flow::add_folder(&app, &path.to_string_lossy());
        }
    });
}
