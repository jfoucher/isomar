//! The left sidebar (`design/ia.md`): an inline switcher over five pages (Passage, Weather,
//! Traffic, Charts, Instruments), each in a navigation view so that a route, a mark or a vessel
//! opens as a detail pushed inside the page that owns it.

use super::app::AppState;
use super::pages;
use crate::i18n::gettext;
use adw::prelude::*;
use std::rc::Rc;

pub const PAGES: [&str; 5] = ["passage", "weather", "traffic", "charts", "instruments"];

pub struct Sidebar {
    pub widget: adw::ToolbarView,
    pub stack: adw::ViewStack,
    navs: Vec<(&'static str, adw::NavigationView)>,
}

impl Sidebar {
    pub fn new(app: &Rc<AppState>) -> Rc<Sidebar> {
        let stack = adw::ViewStack::new();
        stack.set_vexpand(true);
        let mut navs = Vec::new();
        for name in PAGES {
            let (title, icon) = title_and_icon(name);
            let nav = adw::NavigationView::new();
            let content = match name {
                "passage" => pages::passage::build(app, &nav),
                "weather" => pages::weather::build(app),
                "traffic" => pages::traffic::build(app, &nav),
                "charts" => pages::charts::build(app),
                _ => pages::instruments::build(app),
            };
            let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
            root.append(&gtk::Label::builder().label(&title).xalign(0.0).css_classes(["sidebar-title"]).accessible_role(gtk::AccessibleRole::Heading).build());
            root.append(&content);
            nav.add(&adw::NavigationPage::builder().title(&title).tag("root").child(&root).build());
            let page = stack.add_titled_with_icon(&nav, Some(name), &title, icon);
            page.set_use_underline(false);
            navs.push((name, nav));
        }
        let switcher = adw::InlineViewSwitcher::builder().stack(&stack).display_mode(adw::InlineViewSwitcherDisplayMode::Icons).homogeneous(true).margin_top(6).margin_bottom(6).margin_start(12).margin_end(12).build();
        let widget = adw::ToolbarView::new();
        widget.add_top_bar(&switcher);
        widget.set_content(Some(&stack));
        widget.add_css_class("isomar-sidebar");
        Rc::new(Sidebar { widget, stack, navs })
    }

    /// Shows page `name` at its root.
    pub fn show(&self, name: &str) {
        self.stack.set_visible_child_name(name);
        if let Some(nav) = self.nav(name) {
            nav.pop_to_tag("root");
        }
    }

    pub fn nav(&self, name: &str) -> Option<&adw::NavigationView> {
        self.navs.iter().find(|(n, _)| *n == name).map(|(_, v)| v)
    }

    /// Shows `name` and pushes `page` on it.
    pub fn push(&self, name: &str, page: &adw::NavigationPage) {
        self.show(name);
        if let Some(nav) = self.nav(name) {
            nav.push(page);
        }
    }
}

pub fn title_and_icon(name: &str) -> (String, &'static str) {
    match name {
        "passage" => (gettext("Passage"), "isomar-route-symbolic"),
        "weather" => (gettext("Weather"), "weather-few-clouds-symbolic"),
        "traffic" => (gettext("Traffic"), "isomar-ais-symbolic"),
        "charts" => (gettext("Charts"), "isomar-charts-symbolic"),
        _ => (gettext("Instruments"), "isomar-instruments-symbolic"),
    }
}

/// The page a `ISOMAR_SIDEBAR` name opens; the old names still work.
pub fn page_name(s: &str) -> &'static str {
    match s {
        "nav" | "navigation" | "passage" => "passage",
        "ais" | "traffic" => "traffic",
        "weather" => "weather",
        "charts" => "charts",
        _ => "instruments",
    }
}
