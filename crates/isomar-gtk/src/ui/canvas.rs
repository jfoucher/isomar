//! The GTK side of the chart: a widget that hands the engine's frame to GTK's GPU renderer, and the
//! gesture and key controllers that feed `isomar_app::engine::Engine`.
//!
//! The chart's tiles become textures, uploaded once each and then only placed: a pan moves them on the
//! GPU instead of copying every pixel of the view into a new image each frame. The layers drawn every
//! frame come as chunks of an overlay, and only the chunks that changed are uploaded. It is the one
//! GObject subclass in the app: a widget's `snapshot` is the only way to give GTK render nodes.
//! `ISOMAR_CAIRO_CANVAS=1` draws the whole view with cairo instead, as before, for comparisons.

use super::palette::Mode;
use crate::config::ViewState;
use adw::prelude::*;
use adw::subclass::prelude::*;
use isomar_app::engine::{Engine, Scene};
use isomar_app::key::Key;
use isomar_app::render::TILE;
use gtk::{gdk, glib, graphene, gsk};
use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

/// Bytes of tile textures kept after they leave the view, so that panning back does not upload them again.
const TEXTURE_BYTES: usize = 160 << 20;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ChartView {
        pub engine: OnceCell<Engine>,
        /// Tile textures by the tile's id, with the frame that last showed them and their size in bytes.
        pub textures: RefCell<HashMap<u64, (gdk::Texture, u64, usize)>>,
        /// Overlay chunk textures by the chunk's corner, with the chunk's id.
        pub chunks: RefCell<HashMap<(usize, usize), (u64, gdk::Texture)>>,
        pub frame: Cell<u64>,
        pub cairo: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ChartView {
        const NAME: &'static str = "ChartView";
        type Type = super::ChartView;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for ChartView {}

    impl WidgetImpl for ChartView {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            self.obj().paint(snapshot);
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            if let Some(e) = self.engine.get() {
                e.resize(width as f64, height as f64);
            }
        }
    }
}

glib::wrapper! {
    pub struct ChartView(ObjectSubclass<imp::ChartView>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// Cairo's ARGB32 in memory: premultiplied, in the machine's byte order.
fn cairo_format() -> gdk::MemoryFormat {
    if cfg!(target_endian = "little") { gdk::MemoryFormat::B8g8r8a8Premultiplied } else { gdk::MemoryFormat::A8r8g8b8Premultiplied }
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> graphene::Rect {
    graphene::Rect::new(x as f32, y as f32, w as f32, h as f32)
}

fn rgba((r, g, b, a): isomar_app::tokens::Rgba) -> gdk::RGBA {
    gdk::RGBA::new(r as f32, g as f32, b as f32, a as f32)
}

impl ChartView {
    fn new(engine: Engine) -> ChartView {
        let view: ChartView = glib::Object::new();
        let _ = view.imp().engine.set(engine);
        view.imp().cairo.set(std::env::var_os("ISOMAR_CAIRO_CANVAS").is_some());
        view.set_hexpand(true);
        view.set_vexpand(true);
        view.set_focusable(true);
        view.set_overflow(gtk::Overflow::Hidden);
        view
    }

    fn paint(&self, snapshot: &gtk::Snapshot) {
        let imp = self.imp();
        let Some(engine) = imp.engine.get() else { return };
        let (w, h) = (self.width() as f64, self.height() as f64);
        if w < 1.0 || h < 1.0 {
            return;
        }
        let scale = self.scale_factor().max(1);
        let dark = adw::StyleManager::default().is_dark();
        if imp.cairo.get() {
            let cr = snapshot.append_cairo(&rect(0.0, 0.0, w, h));
            engine.draw(&cr, w, h, scale, dark);
            return;
        }
        // With a fractional scale, GTK maps points to pixels that are not whole: textures are then smoothed.
        let whole = self.native().and_then(|n| n.surface()).is_none_or(|s| (s.scale() - scale as f64).abs() < 1e-6);
        let frame = imp.frame.get() + 1;
        imp.frame.set(frame);
        engine.frame(w, h, scale, dark, |scene| {
            snapshot.append_color(&rgba(scene.void), &rect(0.0, 0.0, w, h));
            let (top, bottom) = scene.sea_band;
            if bottom > top {
                snapshot.append_color(&rgba(scene.sea), &rect(0.0, top, w, bottom - top));
            }
            self.place_tiles(snapshot, scene, frame, whole);
            self.place_overlay(snapshot, scene, whole);
        });
    }

    fn place_tiles(&self, snapshot: &gtk::Snapshot, scene: &Scene, frame: u64, whole: bool) {
        let mut textures = self.imp().textures.borrow_mut();
        let tile = TILE as f64;
        for p in &scene.tiles {
            let entry = textures.entry(p.id).or_insert_with(|| {
                let mut tex = None;
                let (tw, th, stride) = (p.surface.width(), p.surface.height(), p.surface.stride() as usize);
                let _ = p.surface.with_data(|data| {
                    let bytes = glib::Bytes::from(data);
                    tex = Some(gdk::MemoryTexture::new(tw, th, cairo_format(), &bytes, stride).upcast::<gdk::Texture>());
                });
                let bytes = stride * th as usize;
                (tex.unwrap_or_else(|| gdk::MemoryTexture::new(1, 1, cairo_format(), &glib::Bytes::from(&[0u8; 4]), 4).upcast()), 0, bytes)
            });
            entry.1 = frame;
            let filter = if p.clip.is_none() && whole && (p.size - tile).abs() < 1e-9 {
                gsk::ScalingFilter::Nearest
            } else if p.size < tile * 0.75 {
                gsk::ScalingFilter::Trilinear
            } else {
                gsk::ScalingFilter::Linear
            };
            if let Some([x, y, cw, ch]) = p.clip {
                snapshot.push_clip(&rect(x, y, cw, ch));
            }
            snapshot.append_scaled_texture(&entry.0, filter, &rect(p.x, p.y, p.size, p.size));
            if p.clip.is_some() {
                snapshot.pop();
            }
        }
        // Textures of tiles out of view go, the longest unseen first, past the budget.
        let held: usize = textures.values().map(|t| t.2).sum();
        if held > TEXTURE_BYTES {
            let mut old: Vec<(u64, u64, usize)> = textures.iter().filter(|(_, t)| t.1 != frame).map(|(id, t)| (t.1, *id, t.2)).collect();
            old.sort_unstable();
            let mut over = held - TEXTURE_BYTES;
            for (_, id, bytes) in old {
                if over == 0 {
                    break;
                }
                textures.remove(&id);
                over = over.saturating_sub(bytes);
            }
        }
    }

    fn place_overlay(&self, snapshot: &gtk::Snapshot, scene: &Scene, whole: bool) {
        let mut chunks = self.imp().chunks.borrow_mut();
        let s = scene.scale as f64;
        let filter = if whole { gsk::ScalingFilter::Nearest } else { gsk::ScalingFilter::Linear };
        let mut shown = std::collections::HashSet::new();
        for c in &scene.overlay {
            let key = (c.x, c.y);
            shown.insert(key);
            let tex = match chunks.get(&key) {
                Some((id, tex)) if *id == c.id => tex.clone(),
                _ => {
                    let bytes = glib::Bytes::from_owned(scene.chunk_pixels(c));
                    let tex = gdk::MemoryTexture::new(c.w as i32, c.h as i32, cairo_format(), &bytes, c.w * 4).upcast::<gdk::Texture>();
                    chunks.insert(key, (c.id, tex.clone()));
                    tex
                }
            };
            snapshot.append_scaled_texture(&tex, filter, &rect(c.x as f64 / s, c.y as f64 / s, c.w as f64 / s, c.h as f64 / s));
        }
        chunks.retain(|k, _| shown.contains(k));
    }
}

#[derive(Clone)]
pub struct Canvas {
    pub widget: gtk::Widget,
    pub engine: Engine,
}

impl Canvas {
    pub fn new(initial: ViewState, mode: Mode) -> Self {
        let engine = Engine::new(initial, mode);
        engine.set_tile_workers(isomar_app::render::default_workers());
        let view = ChartView::new(engine.clone());
        {
            let weak = view.downgrade();
            engine.set_redraw(Rc::new(move || {
                if let Some(v) = weak.upgrade() {
                    v.queue_draw();
                }
            }));
        }
        {
            let view = view.downgrade();
            adw::StyleManager::default().connect_dark_notify(move |_| {
                if let Some(v) = view.upgrade() {
                    v.queue_draw();
                }
            });
        }
        let area: gtk::Widget = view.upcast();
        install_pan(&area, &engine);
        install_zoom(&area, &engine);
        install_pointer(&area, &engine);
        install_clicks(&area, &engine);
        install_keys(&area, &engine);

        Canvas { widget: area, engine }
    }
}

fn install_pan(area: &gtk::Widget, engine: &Engine) {
    for button in [1u32, 2] {
        let drag = gtk::GestureDrag::new();
        drag.set_button(button);
        {
            let engine = engine.clone();
            drag.connect_drag_begin(move |_, x, y| engine.drag_begin(button, x, y));
        }
        {
            let engine = engine.clone();
            drag.connect_drag_update(move |_, dx, dy| engine.drag_update(dx, dy));
        }
        {
            let engine = engine.clone();
            drag.connect_drag_end(move |_, dx, dy| engine.drag_end(dx, dy));
        }
        area.add_controller(drag);
    }
}

fn install_zoom(area: &gtk::Widget, engine: &Engine) {
    let scroll = gtk::EventControllerScroll::new(
        gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::DISCRETE,
    );
    let pointer = Rc::new(Cell::new((0.0, 0.0)));
    {
        let pointer = pointer.clone();
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion(move |_, x, y| pointer.set((x, y)));
        area.add_controller(motion);
    }
    {
        let engine = engine.clone();
        scroll.connect_scroll(move |_, _, dy| {
            let (px, py) = pointer.get();
            engine.scroll_zoom(dy, px, py);
            glib::Propagation::Stop
        });
    }
    area.add_controller(scroll);

    let pinch = gtk::GestureZoom::new();
    {
        let engine = engine.clone();
        pinch.connect_begin(move |_, _| engine.magnify_begin());
    }
    {
        let engine = engine.clone();
        let weak = area.downgrade();
        pinch.connect_scale_changed(move |g, scale| {
            let Some(area) = weak.upgrade() else { return };
            let (px, py) = g.bounding_box_center().unwrap_or((area.width() as f64 / 2.0, area.height() as f64 / 2.0));
            engine.magnify(scale, px, py);
        });
    }
    area.add_controller(pinch);
}

fn install_pointer(area: &gtk::Widget, engine: &Engine) {
    let motion = gtk::EventControllerMotion::new();
    {
        let engine = engine.clone();
        motion.connect_motion(move |_, x, y| engine.pointer_moved(Some((x, y))));
    }
    {
        let engine = engine.clone();
        motion.connect_leave(move |_| engine.pointer_moved(None));
    }
    area.add_controller(motion);
}

fn install_clicks(area: &gtk::Widget, engine: &Engine) {
    let click = gtk::GestureClick::new();
    click.set_button(0);
    let engine = engine.clone();
    let weak = area.downgrade();
    click.connect_released(move |g, n_press, x, y| {
        let Some(area) = weak.upgrade() else { return };
        area.grab_focus();
        engine.click(g.current_button(), n_press, x, y);
    });
    area.add_controller(click);
}

/// The engine's name for a GDK key.
pub fn key_of(key: gdk::Key) -> Key {
    match key {
        gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add => Key::ZoomIn,
        gdk::Key::minus | gdk::Key::KP_Subtract => Key::ZoomOut,
        gdk::Key::Left => Key::Left,
        gdk::Key::Right => Key::Right,
        gdk::Key::Up => Key::Up,
        gdk::Key::Down => Key::Down,
        gdk::Key::Escape => Key::Escape,
        gdk::Key::Return | gdk::Key::KP_Enter => Key::Enter,
        gdk::Key::BackSpace => Key::Backspace,
        _ => Key::Other,
    }
}

fn install_keys(area: &gtk::Widget, engine: &Engine) {
    let keys = gtk::EventControllerKey::new();
    let engine = engine.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        if engine.key(key_of(key)) { glib::Propagation::Stop } else { glib::Propagation::Proceed }
    });
    area.add_controller(keys);
}
