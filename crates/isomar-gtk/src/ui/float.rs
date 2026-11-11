//! Dialogs are panels inside the main window, not windows of their own and not sheets over the
//! chart: each is dragged by its header bar and resized by its corner, within the window, and the
//! chart around it still pans and zooms. They cannot leave the window, take the focus from another
//! app or get lost behind one. [`install`] makes the layer they float in, [`window`] makes one,
//! and [`Alert`] is the same for the short questions libadwaita would show as modal alerts.

use adw::prelude::*;
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

/// The gap kept between a panel and the edges of the window.
const EDGE: i32 = 8;
/// How far each panel opened while others are shown is moved down and right of the last.
const CASCADE: i32 = 28;

/// Where a panel is and how big; a height of -1 fits the content.
#[derive(Clone, Copy)]
struct Geom {
    pos: Option<(i32, i32)>,
    w: i32,
    h: i32,
}

thread_local! {
    static LAYER: RefCell<Option<gtk::Overlay>> = const { RefCell::new(None) };
    static OPEN: RefCell<Vec<Panel>> = const { RefCell::new(Vec::new()) };
    /// Where each panel was when it closed, by title, so it opens there again.
    static PLACES: RefCell<HashMap<String, Geom>> = RefCell::new(HashMap::new());
}

/// Makes `layer` (an overlay over the window's content, under the header bar) the place panels
/// float in.
pub fn install(layer: &gtk::Overlay) {
    layer.connect_get_child_position(|layer, w| {
        let p = OPEN.with(|o| o.borrow().iter().find(|p| p.0.root.upcast_ref::<gtk::Widget>() == w).cloned())?;
        Some(p.rect(layer.width(), layer.height()))
    });
    LAYER.with(|l| *l.borrow_mut() = Some(layer.clone()));
}

fn layer() -> gtk::Overlay {
    LAYER.with(|l| l.borrow().clone()).expect("no panel layer installed")
}

type CloseHandler = Box<dyn Fn(&Panel) -> glib::Propagation>;

struct Inner {
    title: String,
    root: gtk::Overlay,
    geom: Cell<Geom>,
    default: RefCell<Option<gtk::Widget>>,
    on_close: RefCell<Vec<CloseHandler>>,
}

/// A dialog inside the main window. Cheap to clone; [`Panel::downgrade`] for closures it holds.
#[derive(Clone)]
pub struct Panel(Rc<Inner>);

pub struct WeakPanel(Weak<Inner>);

impl WeakPanel {
    pub fn upgrade(&self) -> Option<Panel> {
        self.0.upgrade().map(Panel)
    }
}

/// A resizable, non-modal panel; a height of -1 fits the content. Show it with [`Panel::present`].
pub fn window(title: &str, width: i32, height: i32) -> Panel {
    let root = gtk::Overlay::builder().css_classes(["floating-panel"]).overflow(gtk::Overflow::Hidden).build();
    let geom = PLACES.with(|p| p.borrow().get(title).copied()).unwrap_or(Geom { pos: None, w: width, h: height });
    let p = Panel(Rc::new(Inner { title: title.to_string(), root: root.clone(), geom: Cell::new(geom), default: RefCell::new(None), on_close: RefCell::new(Vec::new()) }));

    // Escape closes, and Enter in an entry presses the default button, as in a window.
    let keys = gtk::ShortcutController::new();
    let weak = p.downgrade();
    keys.add_shortcut(gtk::Shortcut::new(
        gtk::ShortcutTrigger::parse_string("Escape"),
        Some(gtk::CallbackAction::new(move |_, _| {
            if let Some(p) = weak.upgrade() {
                p.close();
            }
            glib::Propagation::Stop
        })),
    ));
    root.add_controller(keys);
    let group = gtk::gio::SimpleActionGroup::new();
    let activate = gtk::gio::SimpleAction::new("activate", None);
    let weak = p.downgrade();
    activate.connect_activate(move |_, _| {
        if let Some(d) = weak.upgrade().and_then(|p| p.0.default.borrow().clone())
            && d.is_sensitive()
        {
            d.activate();
        }
    });
    group.add_action(&activate);
    root.insert_action_group("default", Some(&group));

    // A press anywhere brings it to the front; a drag on the header bar (not on its buttons)
    // moves it.
    let raise = gtk::GestureClick::builder().propagation_phase(gtk::PropagationPhase::Capture).build();
    let weak = p.downgrade();
    raise.connect_pressed(move |_, _, _, _| {
        if let Some(p) = weak.upgrade() {
            p.raise();
        }
    });
    root.add_controller(raise);
    let drag = gtk::GestureDrag::builder().propagation_phase(gtk::PropagationPhase::Capture).build();
    // Where the panel was and where the pointer was in the layer when the drag began.
    let start = Rc::new(Cell::new(((0, 0), (0.0, 0.0))));
    {
        let (weak, start) = (p.downgrade(), start.clone());
        drag.connect_drag_begin(move |g, x, y| {
            let Some(p) = weak.upgrade() else { return };
            if !p.on_header(x, y) {
                g.set_state(gtk::EventSequenceState::Denied);
                return;
            }
            g.set_state(gtk::EventSequenceState::Claimed);
            start.set((p.0.geom.get().pos.unwrap_or_default(), pointer_in_layer(g, 0.0, 0.0).unwrap_or_default()));
        });
    }
    {
        let weak = p.downgrade();
        drag.connect_drag_update(move |g, dx, dy| {
            if let (Some(p), Some((px, py))) = (weak.upgrade(), pointer_in_layer(g, dx, dy)) {
                let ((x, y), (x0, y0)) = start.get();
                p.move_to(x + (px - x0) as i32, y + (py - y0) as i32);
            }
        });
    }
    root.add_controller(drag);

    // The grip in the bottom right corner resizes it.
    let grip = gtk::Box::builder().width_request(16).height_request(16).halign(gtk::Align::End).valign(gtk::Align::End).cursor(&gdk::Cursor::from_name("se-resize", None).unwrap()).build();
    let resize = gtk::GestureDrag::new();
    let size = Rc::new(Cell::new(((0, 0), (0.0, 0.0))));
    {
        let (weak, size) = (p.downgrade(), size.clone());
        resize.connect_drag_begin(move |g, _, _| {
            g.set_state(gtk::EventSequenceState::Claimed);
            if let Some(p) = weak.upgrade() {
                size.set(((p.0.root.width(), p.0.root.height()), pointer_in_layer(g, 0.0, 0.0).unwrap_or_default()));
            }
        });
    }
    {
        let weak = p.downgrade();
        resize.connect_drag_update(move |g, dx, dy| {
            if let (Some(p), Some((px, py))) = (weak.upgrade(), pointer_in_layer(g, dx, dy)) {
                let ((w, h), (x0, y0)) = size.get();
                let mut g = p.0.geom.get();
                (g.w, g.h) = (w + (px - x0) as i32, h + (py - y0) as i32);
                p.0.geom.set(g);
                layer().queue_allocate();
            }
        });
    }
    grip.add_controller(resize);
    root.add_overlay(&grip);
    p
}

/// Where the pointer of drag `g` is now, `(dx, dy)` from where it began, in the layer's
/// coordinates. The gesture's own offsets are in the coordinates of its widget, which the drag
/// moves or resizes, so they would chase themselves.
fn pointer_in_layer(g: &gtk::GestureDrag, dx: f64, dy: f64) -> Option<(f64, f64)> {
    let (x, y) = g.start_point()?;
    let p = g.widget()?.compute_point(&layer(), &gtk::graphene::Point::new((x + dx) as f32, (y + dy) as f32))?;
    Some((p.x() as f64, p.y() as f64))
}

impl Panel {
    pub fn downgrade(&self) -> WeakPanel {
        WeakPanel(Rc::downgrade(&self.0))
    }

    /// Sets what the panel shows. Its header bars get the panel's title and a close button
    /// instead of the window's title and controls, which would act on the main window.
    pub fn set_content(&self, content: Option<&impl IsA<gtk::Widget>>) {
        self.0.root.set_child(content);
        if let Some(c) = content {
            self.adopt_headers(c.as_ref());
        }
    }

    fn adopt_headers(&self, w: &gtk::Widget) {
        if let Some(h) = w.downcast_ref::<adw::HeaderBar>() {
            if h.shows_title() && h.title_widget().is_none() {
                h.set_title_widget(Some(&adw::WindowTitle::new(&self.0.title, "")));
            }
            if h.shows_end_title_buttons() {
                let close = gtk::Button::builder().icon_name("window-close-symbolic").tooltip_text(crate::i18n::gettext("Close")).css_classes(["flat", "circular"]).build();
                let weak = self.downgrade();
                close.connect_clicked(move |_| {
                    if let Some(p) = weak.upgrade() {
                        p.close();
                    }
                });
                h.pack_end(&close);
            }
            h.set_show_start_title_buttons(false);
            h.set_show_end_title_buttons(false);
            return;
        }
        let mut c = w.first_child();
        while let Some(child) = c {
            self.adopt_headers(&child);
            c = child.next_sibling();
        }
    }

    /// The button Enter presses from an entry that activates the default.
    pub fn set_default_widget(&self, w: Option<&impl IsA<gtk::Widget>>) {
        *self.0.default.borrow_mut() = w.map(|w| w.clone().upcast());
    }

    pub fn insert_action_group(&self, name: &str, group: Option<&impl IsA<gtk::gio::ActionGroup>>) {
        self.0.root.insert_action_group(name, group);
    }

    pub fn is_mapped(&self) -> bool {
        self.0.root.is_mapped()
    }

    /// Called when the panel is asked to close; `Stop` keeps it open.
    pub fn connect_close_request(&self, f: impl Fn(&Panel) -> glib::Propagation + 'static) {
        self.0.on_close.borrow_mut().push(Box::new(f));
    }

    /// Shows the panel in front of the others, and gives it the keyboard focus.
    pub fn present(&self) {
        let layer = layer();
        if self.0.root.parent().is_none() {
            let shown = OPEN.with(|o| {
                let mut o = o.borrow_mut();
                o.push(self.clone());
                o.len() as i32 - 1
            });
            if self.0.geom.get().pos.is_none() && shown > 0 {
                // Centred, then cascaded below the others.
                let w = self.0.geom.get().w.min(layer.width() - 2 * EDGE);
                let mut g = self.0.geom.get();
                g.pos = Some(((layer.width() - w) / 2 + CASCADE * shown, EDGE + CASCADE * shown));
                self.0.geom.set(g);
            }
            layer.add_overlay(&self.0.root);
            layer.set_measure_overlay(&self.0.root, false);
        }
        self.raise();
        let root = self.0.root.clone();
        glib::idle_add_local_once(move || {
            if root.focus_child().is_none() {
                root.child_focus(gtk::DirectionType::TabForward);
            }
        });
    }

    /// Asks the close handlers, then removes the panel and gives the focus back to the chart.
    pub fn close(&self) {
        if self.0.root.parent().is_none() {
            return;
        }
        for f in self.0.on_close.borrow().iter() {
            if f(self) == glib::Propagation::Stop {
                return;
            }
        }
        PLACES.with(|p| p.borrow_mut().insert(self.0.title.clone(), self.0.geom.get()));
        OPEN.with(|o| o.borrow_mut().retain(|p| !Rc::ptr_eq(&p.0, &self.0)));
        layer().remove_overlay(&self.0.root);
        super::chart_widget().grab_focus();
    }

    fn raise(&self) {
        let layer = layer();
        if layer.last_child().as_ref() != Some(self.0.root.upcast_ref()) {
            self.0.root.insert_before(&layer, None::<&gtk::Widget>);
        }
    }

    /// Whether (x, y) is on a header bar, and not on one of its buttons or entries.
    fn on_header(&self, x: f64, y: f64) -> bool {
        let root: &gtk::Widget = self.0.root.upcast_ref();
        let mut w = root.pick(x, y, gtk::PickFlags::DEFAULT);
        while let Some(cur) = w {
            if cur == *root || cur.is_focusable() || cur.is::<gtk::Button>() {
                return false;
            }
            if cur.is::<adw::HeaderBar>() {
                return true;
            }
            w = cur.parent();
        }
        false
    }

    fn move_to(&self, x: i32, y: i32) {
        let layer = layer();
        let (w, h) = (self.0.root.width(), self.0.root.height());
        let mut g = self.0.geom.get();
        g.pos = Some((x.clamp(EDGE, (layer.width() - w - EDGE).max(EDGE)), y.clamp(EDGE, (layer.height() - h - EDGE).max(EDGE))));
        self.0.geom.set(g);
        layer.queue_allocate();
    }

    /// Where the panel goes in a layer of `lw` by `lh`: its size within the layer, and moved back
    /// inside when the window got smaller. A panel that has not been placed yet is centred.
    fn rect(&self, lw: i32, lh: i32) -> gdk::Rectangle {
        let root = &self.0.root;
        let mut g = self.0.geom.get();
        let (min_w, ..) = root.measure(gtk::Orientation::Horizontal, -1);
        let w = g.w.min(lw - 2 * EDGE).max(min_w);
        let (min_h, nat_h, ..) = root.measure(gtk::Orientation::Vertical, w);
        let h = if g.h < 0 { nat_h } else { g.h }.min(lh - 2 * EDGE).max(min_h);
        let (x, y) = g.pos.unwrap_or(((lw - w) / 2, (lh - h) / 3));
        let pos = (x.clamp(EDGE, (lw - w - EDGE).max(EDGE)), y.clamp(EDGE, (lh - h - EDGE).max(EDGE)));
        if g.pos.is_none() {
            g.pos = Some(pos);
            self.0.geom.set(g);
        }
        gdk::Rectangle::new(pos.0, pos.1, w, h)
    }
}

/// What an [`Alert`]'s buttons share with it, without holding its window.
struct Shared {
    done: Cell<bool>,
    close_id: RefCell<String>,
    handlers: RefCell<Vec<Box<dyn Fn(&str)>>>,
    tx: async_channel::Sender<String>,
}

impl Shared {
    fn respond(&self, id: &str) {
        if self.done.replace(true) {
            return;
        }
        for h in self.handlers.borrow().iter() {
            h(id);
        }
        let _ = self.tx.try_send(id.to_string());
    }
}

/// A heading, a line of text, an optional widget and a row of buttons, in a
/// window like the others. Closing it answers the close response.
pub struct Alert {
    pub win: Panel,
    content: gtk::Box,
    buttons: gtk::Box,
    shared: Rc<Shared>,
    rx: async_channel::Receiver<String>,
}

impl Alert {
    pub fn new(heading: &str, body: Option<&str>) -> Self {
        let win = window(heading, 400, -1);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.set_margin_start(24);
        content.set_margin_end(24);
        content.append(&gtk::Label::builder().label(heading).css_classes(["title-2"]).wrap(true).justify(gtk::Justification::Center).build());
        if let Some(body) = body {
            content.append(&gtk::Label::builder().label(body).wrap(true).justify(gtk::Justification::Center).build());
        }
        let buttons = gtk::Box::builder().spacing(12).homogeneous(true).margin_top(12).margin_start(24).margin_end(24).margin_bottom(24).vexpand(true).valign(gtk::Align::End).build();
        let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        page.append(&content);
        page.append(&buttons);
        let tv = adw::ToolbarView::new();
        tv.add_top_bar(&adw::HeaderBar::builder().show_title(false).build());
        tv.set_content(Some(&page));
        win.set_content(Some(&tv));

        let (tx, rx) = async_channel::bounded(1);
        let shared = Rc::new(Shared { done: Cell::new(false), close_id: RefCell::new(String::new()), handlers: RefCell::new(Vec::new()), tx });
        let s = shared.clone();
        win.connect_close_request(move |_| {
            let id = s.close_id.borrow().clone();
            s.respond(&id);
            glib::Propagation::Proceed
        });
        Self { win, content, buttons, shared, rx }
    }

    /// A widget under the text (details, a progress bar, an entry).
    pub fn extra(&self, w: &impl IsA<gtk::Widget>) {
        self.content.append(w);
    }

    pub fn response(&self, id: &str, label: &str, appearance: adw::ResponseAppearance) -> gtk::Button {
        let b = gtk::Button::with_label(label);
        match appearance {
            adw::ResponseAppearance::Suggested => b.add_css_class("suggested-action"),
            adw::ResponseAppearance::Destructive => b.add_css_class("destructive-action"),
            _ => {}
        }
        let (s, win, id) = (self.shared.clone(), self.win.downgrade(), id.to_string());
        b.connect_clicked(move |_| {
            s.respond(&id);
            if let Some(w) = win.upgrade() {
                w.close();
            }
        });
        self.buttons.append(&b);
        b
    }

    /// The answer when the window is closed rather than a button pressed.
    pub fn close_response(&self, id: &str) {
        *self.shared.close_id.borrow_mut() = id.to_string();
    }

    pub fn present(&self) {
        self.win.present();
    }

    /// Shows the alert and waits for the answer.
    pub async fn choose(&self) -> String {
        self.present();
        self.rx.recv().await.unwrap_or_default()
    }
}
