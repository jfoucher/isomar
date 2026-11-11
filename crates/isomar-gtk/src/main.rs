mod ui;

// The domain code lives in isomar-core; keep the `crate::core::…` paths.
#[macro_use]
extern crate isomar_core;
use isomar_core::{config, i18n};
use isomar_core as core;

use adw::prelude::*;
use gtk::{gio, glib};

pub const APP_ID: &str = "eu._6px.isomar";

fn main() -> glib::ExitCode {
    core::log::install_panic_hook();
    core::datadir::migrate_legacy();
    i18n::init();
    ui::install_platform();
    log!("isomar {} starting; log at {}", env!("CARGO_PKG_VERSION"), core::log::path().display());

    gio::resources_register_include!("isomar.gresource").expect("failed to register resources");

    let mut flags = gio::ApplicationFlags::default();
    if std::env::var_os("ISOMAR_SCREENSHOT").is_some() {
        flags |= gio::ApplicationFlags::NON_UNIQUE;
    }
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(flags)
        .resource_base_path("/eu/_6px/isomar")
        .build();

    app.connect_startup(|_| {
        gtk::Window::set_default_icon_name(APP_ID);
    });
    app.connect_activate(|app| {
        ui::window::MainWindow::get_or_create(app).present();
    });
    app.run()
}
