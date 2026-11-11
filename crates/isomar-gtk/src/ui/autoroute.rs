//! The "Auto route…" dialog (`design/screens/dialogs-gnome-task.png`): finds waypoints between the
//! points of a route that keep clear of land and of water shallower than the safety depth, and saves
//! the result as a new route. The search is `isomar_app::flows::autoroute`, shown as a job.

use super::app::AppState;
use super::form::Form;
use super::spawn;
use crate::core::routing::auto::AutoSettings;
use crate::i18n::{gettext, gettext_f};
use adw::prelude::*;
use std::rc::Rc;

pub fn open(app: &Rc<AppState>, route_id: i64) {
    let Some(source) = app.session.borrow().routes.iter().find(|r| r.id == route_id).cloned() else { return };
    if source.points.len() < 2 {
        return;
    }
    let app = app.clone();
    spawn(async move {
        let (safety0, clearance0) = {
            let s = app.settings.borrow();
            (f64::from(s.safety_depth), s.auto_clearance_nm)
        };
        let offline = super::debug::offline();
        let form = Form::new(&gettext("Auto route"), &gettext("Add waypoints"));
        form.group(&gettext("Route"));
        let n = source.points.len();
        form.chip_line(
            "isomar-route-symbolic",
            &source.name,
            &[
                isomar_app::chips::Chip::plain(gettext_f("{from} to {to}", &[("from", &source.points[0].name), ("to", &source.points[n - 1].name)])),
                isomar_app::chips::Chip::new(isomar_app::chips::Icon::Points, n.to_string(), crate::i18n::ngettext_f("{n} point", "{n} points", n as u32, &[("n", &n.to_string())])),
            ],
        );
        form.description(&gettext("Adds waypoints between your points so that the legs keep clear of land and of water shallower than the safety depth. Your own points are kept."));
        form.group(&gettext("Keep clear of"));
        let safety = form.number(&gettext("Safety depth"), &gettext("m"), (0.5, 50.0, 0.5), 1, safety0);
        safety.set_subtitle(&gettext("Below chart datum: the tide is not added"));
        let clearance = form.number(&gettext("Distance from land and shallows"), &gettext("nm"), (0.05, 2.0, 0.05), 2, clearance0);
        form.group(&gettext("Depths"));
        let download = form.switch(&gettext("Download missing depths"), &gettext("EMODnet, about half a megabyte for each half degree. What is already saved is always used."), !offline);
        if offline {
            download.set_sensitive(false);
            download.set_subtitle(&gettext("Offline: only depths already saved are used"));
        }
        form.planning_only();
        if !form.run().await {
            return;
        }
        if (clearance.value() - clearance0).abs() > f64::EPSILON {
            app.settings.borrow_mut().auto_clearance_nm = clearance.value();
            app.apply_settings();
        }
        let settings = AutoSettings { safety_depth_m: safety.value(), clearance_nm: clearance.value(), ..AutoSettings::default() };
        // The flow shows the search as a job, with Cancel, in the activity indicator.
        let busy = {
            let app = app.clone();
            move |_: bool| app.refresh()
        };
        isomar_app::flows::autoroute::compute(&app, source, settings, download.is_active() && !offline, &busy).await;
    });
}
