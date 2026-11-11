//! The Traffic page: the AIS vessels round the boat, nearest and most dangerous first. A row
//! shows the vessel on the chart; its information opens inside the page. Built from `vm::traffic`.

use super::{clear_list, placeholder, row_button, scrolled};
use crate::i18n::{gettext, gettext_f};
use crate::ui::app::AppState;
use adw::prelude::*;
use isomar_app::vm::traffic::Vessel;
use std::cell::RefCell;
use std::rc::Rc;

/// The most vessels listed.
const LIMIT: usize = 60;

pub fn build(app: &Rc<AppState>, _nav: &adw::NavigationView) -> gtk::Widget {
    let page = super::column();
    let (section, list) = super::passage::section("", None);
    list.set_placeholder(Some(&placeholder(&gettext("AIS targets appear when a receiver is connected."))));
    page.append(&section);
    let heading = section.first_child().and_then(|h| h.first_child()).and_downcast::<gtk::Label>();

    let shown: RefCell<Option<Vec<Vessel>>> = RefCell::new(None);
    let update = {
        let app = Rc::downgrade(app);
        move || {
            let Some(app) = app.upgrade() else { return };
            let v = isomar_app::vm::traffic::vessels(&app, LIMIT);
            if shown.borrow().as_ref() == Some(&v) {
                return;
            }
            if let Some(h) = &heading {
                h.set_text(&gettext_f("{n} vessels", &[("n", &v.len().to_string())]));
            }
            clear_list(&list);
            for t in &v {
                let icon = gtk::Image::from_icon_name(if t.alert { "isomar-alarm-symbolic" } else { "isomar-ais-symbolic" });
                icon.add_css_class(if t.alert { "error" } else { "ais" });
                let row = crate::ui::chips::row_with(Some(icon.upcast_ref()), &t.name, &t.detail);
                row.set_activatable(true);
                if t.alert {
                    row.add_suffix(&super::more_menu(&[(gettext("Acknowledge collision alert"), "win.acknowledge-vessel"), (gettext("Vessel information"), "win.vessel-detail")], i64::from(t.mmsi)));
                }
                let info = row_button("help-about-symbolic", &gettext("Vessel information"));
                info.set_action_name(Some("win.vessel-detail"));
                info.set_action_target_value(Some(&i64::from(t.mmsi).to_variant()));
                row.add_suffix(&info);
                row.set_action_name(Some("win.show-vessel"));
                row.set_action_target_value(Some(&i64::from(t.mmsi).to_variant()));
                list.append(&row);
            }
            *shown.borrow_mut() = Some(v);
        }
    };
    update();
    // Targets move between refreshes: follow the clock, every two seconds.
    let update = Rc::new(update);
    let ticks = std::cell::Cell::new(0u32);
    {
        let u = update.clone();
        app.on_refresh(move || u());
    }
    app.on_tick(move || {
        ticks.set(ticks.get() + 1);
        if ticks.get().is_multiple_of(8) {
            update();
        }
    });
    scrolled(&page).upcast()
}
