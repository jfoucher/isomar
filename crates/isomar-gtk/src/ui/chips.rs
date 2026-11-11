//! Values in a line, each after its icon (`isomar_app::chips`): "[satellite] 8  [speed] 6.0 kn".
//! The label of a chip whose text is short is its tooltip and, with the rest, the line's accessible name.

use adw::prelude::*;
use isomar_app::chips::{Chip, Icon, plain};
use std::cell::RefCell;

/// The icon of each kind of value, from the Adwaita theme or Isomar's own (`data/icons`).
pub fn icon_name(i: Icon) -> &'static str {
    match i {
        Icon::Satellite => "isomar-satellite-symbolic",
        Icon::Speed => "isomar-speed-symbolic",
        Icon::Distance => "isomar-measure-symbolic",
        Icon::Points => "mark-location-symbolic",
        Icon::Time => "preferences-system-time-symbolic",
        Icon::Arrival => "emoji-flags-symbolic",
        Icon::Date => "x-office-calendar-symbolic",
        Icon::Wind => "weather-windy-symbolic",
        Icon::Gusts => "weather-storm-symbolic",
        Icon::Waves => "isomar-waves-symbolic",
        Icon::Current => "isomar-current-symbolic",
        Icon::Pressure => "isomar-pressure-symbolic",
        Icon::Temperature => "isomar-temperature-symbolic",
        Icon::Engine => "isomar-engine-symbolic",
        Icon::Depth => "isomar-depth-symbolic",
        Icon::Compass => "isomar-compass-symbolic",
        Icon::Position => "find-location-symbolic",
        Icon::Zoom => "zoom-in-symbolic",
        Icon::Size => "drive-harddisk-symbolic",
        Icon::Licence => "application-certificate-symbolic",
        Icon::Warning => "dialog-warning-symbolic",
        Icon::Alarm => "isomar-alarm-symbolic",
        Icon::Upwind => "isomar-upwind-symbolic",
        Icon::Night => "weather-clear-night-symbolic",
        Icon::Tack => "isomar-tack-symbolic",
        Icon::Waiting => "appointment-soon-symbolic",
        Icon::Forecast => "weather-few-clouds-symbolic",
        Icon::Source => "folder-download-symbolic",
        Icon::High => "go-up-symbolic",
        Icon::Low => "go-down-symbolic",
    }
}

struct Slot {
    b: gtk::Box,
    image: gtk::Image,
    label: gtk::Label,
}

/// A line of chips that keeps its widgets: `set` only changes texts and icons, so it can follow the
/// pointer or the 4 Hz update.
pub struct ChipLine {
    pub widget: gtk::Widget,
    wrap: Option<adw::WrapBox>,
    line: Option<gtk::Box>,
    classes: Vec<String>,
    icon_px: i32,
    slots: RefCell<Vec<Slot>>,
    shown: RefCell<Option<Vec<Chip>>>,
}

impl ChipLine {
    /// A line that wraps when the room is short.
    pub fn wrapping(classes: &[&str]) -> Self {
        // A natural width of its own, or a page holding it widens to all its chips on one line.
        let w = adw::WrapBox::builder().child_spacing(12).line_spacing(2).natural_line_length(160).build();
        Self::make(w.clone().upcast(), Some(w), None, classes, 12)
    }

    /// A line that never wraps (the header's subtitle, the footer); its last text is ellipsised.
    pub fn single(classes: &[&str], icon_px: i32) -> Self {
        let b = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        Self::make(b.clone().upcast(), None, Some(b), classes, icon_px)
    }

    fn make(widget: gtk::Widget, wrap: Option<adw::WrapBox>, line: Option<gtk::Box>, classes: &[&str], icon_px: i32) -> Self {
        ChipLine { widget, wrap, line, classes: classes.iter().map(|c| c.to_string()).collect(), icon_px, slots: RefCell::new(Vec::new()), shown: RefCell::new(None) }
    }

    pub fn set(&self, chips: &[Chip]) {
        if self.shown.borrow().as_deref() == Some(chips) {
            return;
        }
        let mut slots = self.slots.borrow_mut();
        while slots.len() < chips.len() {
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            for c in &self.classes {
                b.add_css_class(c);
            }
            let image = gtk::Image::builder().pixel_size(self.icon_px).valign(gtk::Align::Center).build();
            let label = gtk::Label::builder().xalign(0.0).build();
            if self.wrap.is_some() {
                // A long value (a licence) wraps rather than widening the line, its icon on the first line.
                image.set_valign(gtk::Align::Start);
                image.set_margin_top(2);
                label.set_wrap(true);
                label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                label.set_max_width_chars(28);
            }
            b.append(&image);
            b.append(&label);
            if let Some(w) = &self.wrap {
                w.append(&b);
            }
            if let Some(l) = &self.line {
                l.append(&b);
            }
            slots.push(Slot { b, image, label });
        }
        let last = chips.len().saturating_sub(1);
        for (i, s) in slots.iter().enumerate() {
            let Some(c) = chips.get(i) else {
                s.b.set_visible(false);
                continue;
            };
            s.b.set_visible(true);
            match c.icon {
                Some(icon) => {
                    s.image.set_icon_name(Some(icon_name(icon)));
                    s.image.set_visible(true);
                }
                None => s.image.set_visible(false),
            }
            s.label.set_text(&c.text);
            s.label.set_ellipsize(if self.line.is_some() && i == last { gtk::pango::EllipsizeMode::End } else { gtk::pango::EllipsizeMode::None });
            s.b.set_tooltip_text((c.icon.is_some() && c.label != c.text).then_some(c.label.as_str()));
        }
        self.widget.update_property(&[gtk::accessible::Property::Label(&plain(chips))]);
        *self.shown.borrow_mut() = Some(chips.to_vec());
    }
}

/// A wrapping line already holding `chips`.
pub fn line(chips: &[Chip], classes: &[&str]) -> gtk::Widget {
    let l = ChipLine::wrapping(classes);
    l.set(chips);
    l.widget
}

/// A row with a title and, under it, a line of chips where an ActionRow has its subtitle. It is an
/// ActionRow with neither, so prefixes, suffixes, actions and styles work as on any other row.
pub fn row(icon: Option<&str>, title: &str, chips: &[Chip]) -> adw::ActionRow {
    row_with(icon.map(|i| gtk::Image::from_icon_name(i).upcast()).as_ref(), title, chips)
}

/// [`row`] with any widget before the text.
pub fn row_with(lead: Option<&gtk::Widget>, title: &str, chips: &[Chip]) -> adw::ActionRow {
    row_lines(lead, title, &[chips], false)
}

/// A row with a title and several lines of chips under it; a title that does not fit wraps when
/// `wrap` is set, else it is ellipsised.
pub fn row_lines(lead: Option<&gtk::Widget>, title: &str, lines: &[&[Chip]], wrap: bool) -> adw::ActionRow {
    row_full(lead, title, lines, "", wrap)
}

/// [`row_lines`] with a note of plain text, wrapped, under the chips.
pub fn row_full(lead: Option<&gtk::Widget>, title: &str, lines: &[&[Chip]], note: &str, wrap: bool) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    // One prefix: the icon and the text in a box of their own, in that order.
    let prefix = gtk::Box::builder().spacing(12).hexpand(true).build();
    if let Some(w) = lead {
        prefix.append(w);
    }
    let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(3).hexpand(true).valign(gtk::Align::Center).margin_top(6).margin_bottom(6).build();
    let t = gtk::Label::builder().label(title).xalign(0.0).build();
    if wrap {
        t.set_wrap(true);
        t.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    } else {
        t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    }
    b.append(&t);
    for chips in lines.iter().filter(|c| !c.is_empty()) {
        b.append(&line(chips, &["chips-subtitle"]));
    }
    if !note.is_empty() {
        b.append(&gtk::Label::builder().label(note).xalign(0.0).wrap(true).wrap_mode(gtk::pango::WrapMode::WordChar).css_classes(["chips-subtitle"]).build());
    }
    prefix.append(&b);
    row.add_prefix(&prefix);
    // The row's own title box is empty here but would still share the free width: it is hidden, and
    // the prefixes take the width instead (the row's title is never set, so libadwaita leaves it so).
    if let Some(prefixes) = prefix.parent() {
        prefixes.set_hexpand(true);
        let mut c = prefixes.parent().and_then(|h| h.first_child());
        while let Some(w) = c {
            if w.has_css_class("title") {
                w.set_visible(false);
            }
            c = w.next_sibling();
        }
    }
    let text = lines.iter().map(|c| plain(c)).chain([note.to_string()]).filter(|t| !t.is_empty()).collect::<Vec<_>>().join("; ");
    row.upcast_ref::<gtk::Widget>().update_property(&[gtk::accessible::Property::Label(&if text.is_empty() { title.to_string() } else { format!("{title}: {text}") })]);
    row
}
