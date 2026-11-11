//! The chart engine: hosts the layers, owns the view, and turns pointer,
//! scroll, pinch and key events into view changes and clicks. A shell owns
//! the surface and the gesture recognisers and feeds the engine.
//!
//! The view maths lives in `view` and `core::geo`; drawing is done by the
//! layers in `layers`, back to front, followed by the scale bar.

use crate::key::Key;
use crate::layers::overlay::Overlay;
use crate::layers::{self, DrawCtx, Layer, Redraw};
use crate::palette::{Mode, Palette, SEA, TEXT, VOID};
use crate::platform;
use crate::render::{self, Chunk, Frame, Look, OverlayBuffer, Placement, Tiles};
use crate::tokens::Rgba;
use crate::view::{View, ViewInfo};
use crate::config::ViewState;
use crate::core::geo::{self, LatLon};
use crate::core::vector::Bounds;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// A click or tap on the chart.
#[derive(Clone, Copy, Debug)]
pub struct Click {
    /// 1 primary, 2 middle, 3 secondary.
    pub button: u32,
    pub n_press: i32,
    pub pos: LatLon,
    pub x: f64,
    pub y: f64,
}

/// Phases of a primary-button drag, as offered to drag hooks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragPhase {
    Begin,
    Update,
    End,
}

type ViewCallback = Box<dyn Fn(ViewState)>;
type DragHook = Box<dyn Fn(DragPhase, LatLon) -> bool>;
type KeyHook = Box<dyn Fn(Key) -> bool>;
type PointerCallback = Box<dyn Fn(Option<LatLon>)>;
type ClickCallback = Box<dyn Fn(Click)>;

/// A press that moves further than this many points is a drag, and its release is not a click.
const CLICK_SLOP: f64 = 4.0;
/// The same on a touch screen: a fingertip wobbles more than a mouse.
const TOUCH_SLOP: f64 = 10.0;

/// A press-drag-release in progress.
struct Gesture {
    /// Where the pointer went down.
    at: (f64, f64),
    /// The view's centre in world pixels then.
    start_center: (f64, f64),
    /// A drag hook took the primary button; it is not a pan.
    claimed: bool,
    /// When the finger went down: on a touch screen a mark is dragged only after it was held (`HOLD`).
    began: Instant,
    /// On a touch screen, the pan starts once the finger has gone past the slop, from where it was then,
    /// so a tap that wobbles does not move the chart. `None` until then (always set with a mouse).
    from: Option<(f64, f64)>,
}

/// How long a finger rests on a mark before moving it drags the mark rather than panning the chart.
const HOLD: Duration = Duration::from_millis(300);

/// A pan still gliding after the finger left, in points per second; it slows by `FLING_DECAY` per second.
#[derive(Clone, Copy)]
struct Fling {
    v: (f64, f64),
    at: Instant,
}

const FLING_DECAY: f64 = 4.0;
const FLING_MIN: f64 = 30.0;

struct Inner {
    view: Cell<View>,
    size: Cell<(f64, f64)>,
    /// The size observers last heard of (`resize`).
    announced: Cell<(f64, f64)>,
    dark: Cell<bool>,
    mode: Cell<Mode>,
    layers: RefCell<Vec<Rc<dyn Layer>>>,
    hidden: RefCell<HashSet<String>>,
    /// The static layers, drawn as tiles (`render`): a plane of them for each `Layer::plane`, the
    /// chart's first, the others over it.
    tiles: RefCell<Vec<Tiles>>,
    /// The dynamic layers, for `Engine::frame`.
    overlay: RefCell<OverlayBuffer>,
    /// Frame times, with whether tiles were planned in them, while recorded.
    frames: RefCell<Option<Vec<(Duration, bool)>>>,
    /// Asks the shell to call `draw` soon.
    redraw: RefCell<Rc<dyn Fn()>>,
    /// A redraw is due for tiles held back (`render::Planned::again_in`).
    wake_pending: Cell<bool>,
    underway: Cell<bool>,
    footer_native: Cell<bool>,
    reduce_motion: Cell<bool>,
    gesture: RefCell<Option<Gesture>>,
    /// How far the pointer travelled, in points, since the last button press: a release after a drag is not a click.
    travelled: Cell<f64>,
    fling: Cell<Option<Fling>>,
    /// When a finger last touched down, for a shell whose pan starts only once the finger moves (iOS).
    down_at: Cell<Option<Instant>>,
    /// The view follows the boat; panning the chart by hand ends it.
    follow: Cell<bool>,
    on_follow: RefCell<Vec<Box<dyn Fn(bool)>>>,
    /// The zoom when a pinch began.
    pinch_base: Cell<f64>,
    /// The position under the fingers when the pinch began: it stays under them as they move.
    pinch_anchor: Cell<Option<LatLon>>,
    /// The smallest hit area on a touch screen (`set_touch`), if this is one.
    touch: Cell<Option<f64>>,
    /// What covers the chart's edges (top, right, bottom, left, in points): a bottom sheet, the safe
    /// areas. The view centres in the rest of it.
    insets: Cell<[f64; 4]>,
    on_view: RefCell<Vec<ViewCallback>>,
    on_layers: RefCell<Vec<Box<dyn Fn()>>>,
    on_pointer: RefCell<Vec<PointerCallback>>,
    on_click: RefCell<Vec<ClickCallback>>,
    on_drag: RefCell<Vec<DragHook>>,
    on_key: RefCell<Vec<KeyHook>>,
}

impl Inner {
    fn info(&self) -> ViewInfo {
        let (w, h) = self.size.get();
        ViewInfo { view: self.view.get(), w, h }
    }

    fn request_redraw(&self) {
        // Cloned so a hook that comes back into the engine finds it unborrowed.
        let f = self.redraw.borrow().clone();
        f();
    }

    fn changed(&self) {
        self.request_redraw();
        let state = self.view.get().state();
        for cb in self.on_view.borrow().iter() {
            cb(state);
        }
    }
}

/// The chart: hosts the layers, owns the view, and turns pointer, scroll,
/// pinch and key events (passed in by the shell) into view changes and clicks.
/// The shell calls `draw` when asked to by the redraw hook.
#[derive(Clone)]
pub struct Engine {
    inner: Rc<Inner>,
}

impl Engine {
    pub fn new(initial: ViewState, mode: Mode) -> Self {
        let inner = Rc::new(Inner {
            view: Cell::new(View::from_state(initial)),
            size: Cell::new((1000.0, 700.0)),
            announced: Cell::new((0.0, 0.0)),
            dark: Cell::new(false),
            mode: Cell::new(mode),
            layers: RefCell::new(Vec::new()),
            hidden: RefCell::new(HashSet::new()),
            tiles: RefCell::new(vec![Tiles::new(), Tiles::plane(layers::WEATHER_PLANE, layers::WEATHER_STALE_WAIT)]),
            overlay: RefCell::new(OverlayBuffer::default()),
            frames: RefCell::new(None),
            redraw: RefCell::new(Rc::new(|| {})),
            wake_pending: Cell::new(false),
            underway: Cell::new(false),
            footer_native: Cell::new(false),
            reduce_motion: Cell::new(false),
            gesture: RefCell::new(None),
            travelled: Cell::new(0.0),
            fling: Cell::new(None),
            down_at: Cell::new(None),
            follow: Cell::new(false),
            on_follow: RefCell::new(Vec::new()),
            pinch_base: Cell::new(0.0),
            pinch_anchor: Cell::new(None),
            touch: Cell::new(None),
            insets: Cell::new([0.0; 4]),
            on_view: RefCell::new(Vec::new()),
            on_layers: RefCell::new(Vec::new()),
            on_pointer: RefCell::new(Vec::new()),
            on_click: RefCell::new(Vec::new()),
            on_drag: RefCell::new(Vec::new()),
            on_key: RefCell::new(Vec::new()),
        });
        Engine { inner }
    }

    /// Draws the chart's tiles on `workers` threads, picking them up on the main loop (the shell's
    /// `platform` hooks must be installed), or, with none, in `draw`: tests do.
    pub fn set_tile_workers(&self, workers: usize) {
        let share = |planes: &mut Vec<Tiles>| {
            let (chart, rest) = planes.split_at_mut(1);
            for p in rest {
                p.share_workers(&chart[0]);
            }
        };
        if workers == 0 {
            let mut planes = self.inner.tiles.borrow_mut();
            planes[0].set_workers(0, None);
            share(&mut planes);
            return;
        }
        let (tx, rx) = async_channel::unbounded();
        {
            let mut planes = self.inner.tiles.borrow_mut();
            planes[0].set_workers(workers, Some(tx));
            share(&mut planes);
        }
        let weak = Rc::downgrade(&self.inner);
        platform::spawn(async move {
            let receive = |inner: &Inner, done: render::Done| {
                let plane = Tiles::plane_of(&done) as usize;
                inner.tiles.borrow_mut().get_mut(plane).is_some_and(|t| t.receive(done))
            };
            while let Ok(done) = rx.recv().await {
                let Some(inner) = weak.upgrade() else { break };
                let mut any = receive(&inner, done);
                while let Ok(more) = rx.try_recv() {
                    any |= receive(&inner, more);
                }
                if any {
                    inner.request_redraw();
                }
            }
        });
    }

    /// Tiles held, and the bytes they take (`render::Tiles`).
    pub fn tiles_cached(&self) -> (usize, usize) {
        self.inner.tiles.borrow().iter().map(|t| t.cached()).fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1))
    }

    /// Sets how many bytes of tiles are kept besides those the view wants.
    pub fn set_tile_cache_limit(&self, bytes: usize) {
        for t in self.inner.tiles.borrow_mut().iter_mut() {
            t.set_limit(bytes);
        }
    }

    /// Tiles of the chart queued or being drawn.
    pub fn tiles_pending(&self) -> usize {
        self.inner.tiles.borrow().iter().map(|t| t.pending()).sum()
    }

    /// The tiles of the view that are drawn, and how many it shows.
    pub fn tiles_ready(&self) -> (usize, usize) {
        let vi = self.inner.info();
        self.inner.tiles.borrow().iter().map(|t| t.coverage(&vi)).fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1))
    }

    /// Sets what the engine calls to ask the shell for a repaint.
    pub fn set_redraw(&self, f: Rc<dyn Fn()>) {
        *self.inner.redraw.borrow_mut() = f;
    }

    /// The under-way layout is on: hit areas are bigger (`HIT_MIN_UNDERWAY`).
    pub fn set_underway(&self, on: bool) {
        self.inner.underway.set(on);
        self.queue_draw();
    }

    /// The shell draws the footer itself (`vm::footer`): the engine leaves out its scale bar.
    pub fn set_footer_native(&self, on: bool) {
        self.inner.footer_native.set(on);
    }

    pub fn reduce_motion(&self) -> bool {
        self.inner.reduce_motion.get()
    }

    pub fn underway(&self) -> bool {
        self.inner.underway.get()
    }

    /// Blinking and pulsing are off; the shell sets this from the system's reduce-motion setting.
    pub fn set_reduce_motion(&self, on: bool) {
        self.inner.reduce_motion.set(on);
    }

    /// The smallest size, in pixels, of what can be clicked.
    pub fn hit_min(&self) -> f64 {
        let base = if self.underway() { crate::tokens::HIT_MIN_UNDERWAY } else { crate::tokens::HIT_MIN };
        self.inner.touch.get().map_or(base, |t| base.max(t))
    }

    /// The chart is on a touch screen, whose smallest hit area is `hit_min` points (`HIT_MIN_TOUCH` on iOS,
    /// `HIT_MIN_ANDROID` on Android), or not (`None`). A touch also moves further before it stops being a tap.
    pub fn set_touch(&self, hit_min: Option<f64>) {
        self.inner.touch.set(hit_min);
        self.queue_draw();
    }

    /// The chart is on a touch screen: what the app says is tapped and held, not clicked.
    pub fn is_touch(&self) -> bool {
        self.inner.touch.get().is_some()
    }

    /// What covers the chart's edges, in points: top, right, bottom, left (a bottom sheet, the safe areas).
    /// Centring on a position, fitting a box and zooming with the buttons use the part left visible.
    pub fn set_insets(&self, top: f64, right: f64, bottom: f64, left: f64) {
        let clean = |v: f64| if v.is_finite() { v.max(0.0) } else { 0.0 };
        self.inner.insets.set([clean(top), clean(right), clean(bottom), clean(left)]);
    }

    /// The part of the chart the insets leave visible: its left, top, width and height in points. A sheet
    /// over most of the chart leaves too little to centre in, and the whole width or height is used then.
    pub fn visible_rect(&self) -> (f64, f64, f64, f64) {
        let (w, h) = self.inner.size.get();
        let [top, right, bottom, left] = self.inner.insets.get();
        let (vw, vh) = (w - left - right, h - top - bottom);
        let (x, vw) = if vw >= w * 0.25 { (left, vw) } else { (0.0, w) };
        let (y, vh) = if vh >= h * 0.25 { (top, vh) } else { (0.0, h) };
        (x, y, vw, vh)
    }

    /// The middle of `visible_rect`.
    pub fn visible_centre(&self) -> (f64, f64) {
        let (x, y, w, h) = self.visible_rect();
        (x + w / 2.0, y + h / 2.0)
    }

    /// Whether any layer has something moving on its own, so that the shell should keep repainting.
    pub fn animating(&self) -> bool {
        self.inner.fling.get().is_some() || self.inner.layers.borrow().iter().any(|l| l.animating())
    }

    /// Asks the shell to repaint.
    pub fn queue_draw(&self) {
        self.inner.request_redraw();
    }

    /// Tells the engine the chart's size in points. A bigger window shows
    /// more map, so observers of the view hear of it.
    pub fn resize(&self, w: f64, h: f64) {
        self.inner.size.set((w, h));
        // Shells lay out often; observers hear of a new size only.
        if self.inner.announced.replace((w, h)) != (w, h) {
            self.inner.changed();
        }
    }

    /// Paints the chart. `dark` says whether the system is in a dark mode,
    /// which `Mode::Auto` follows; `scale` is the device pixel ratio.
    pub fn draw(&self, cr: &cairo::Context, w: f64, h: f64, scale: i32, dark: bool) {
        let inner = &self.inner;
        inner.size.set((w, h));
        inner.dark.set(dark);
        let pal = Palette::new(inner.mode.get(), dark);
        let started = Instant::now();
        let rebuilt = draw(inner, cr, w, h, scale, pal);
        let took = started.elapsed();
        if crate::debug::timing() {
            eprintln!("isomar: frame {w}x{h} drawn in {took:?}");
        }
        if let Some(f) = inner.frames.borrow_mut().as_mut() {
            f.push((took, rebuilt));
        }
    }

    /// The chart as a shell that composites on the GPU wants it (`Scene`), handed to `f`: the tiles to
    /// place, which it can keep as textures by their ids, and the dynamic layers in chunks. `Engine::draw`
    /// paints the same with cairo.
    pub fn frame<R>(&self, w: f64, h: f64, scale: i32, dark: bool, f: impl FnOnce(&Scene) -> R) -> R {
        let inner = &self.inner;
        inner.size.set((w, h));
        inner.dark.set(dark);
        if self.glide() {
            inner.request_redraw();
        }
        let pal = Palette::new(inner.mode.get(), dark);
        let started = Instant::now();
        let vi = ViewInfo { view: inner.view.get(), w, h };
        let planned = plan_tiles(inner, vi, scale, pal);
        let mut overlay = inner.overlay.borrow_mut();
        let chunks = match overlay.begin((w * scale as f64).ceil() as i32, (h * scale as f64).ceil() as i32) {
            Some(surface) => {
                surface.set_device_scale(scale as f64, scale as f64);
                // Drawn straight onto it, and every chunk looked at afterwards (about a millisecond at a
                // laptop's full screen). Drawing each layer through a cairo recording, whose extents
                // would have said where to look, left the main thread allocating gigabytes in cairo's
                // replay of one of them, on a view with routes.
                if let Ok(cr) = cairo::Context::new(&surface) {
                    draw_layers(inner, &cr, vi, pal, true);
                    if !inner.footer_native.get() {
                        draw_scale_bar(&vi, &cr, &pal, h);
                    }
                }
                drop(surface);
                overlay.finish(&[[f64::MIN, f64::MIN, f64::MAX, f64::MAX]])
            }
            None => Vec::new(),
        };
        let planes = inner.tiles.borrow();
        let size = geo::world_size(vi.zoom());
        let top = vi.view.center_px().1 - h / 2.0;
        let scene = Scene {
            scale,
            void: pal.color(VOID),
            sea: pal.color(SEA),
            sea_band: ((-top).max(0.0), (size - top).min(h)),
            tiles: planes.iter().flat_map(|t| t.placements(&vi, scale)).collect(),
            overlay: chunks,
            buffer: &overlay,
        };
        let out = f(&scene);
        let took = started.elapsed();
        if crate::debug::timing() {
            eprintln!("isomar: frame {w}x{h} composed in {took:?}, {} overlay chunks", scene.overlay.len());
        }
        if let Some(f) = inner.frames.borrow_mut().as_mut() {
            f.push((took, planned));
        }
        out
    }

    pub fn view_state(&self) -> ViewState {
        self.inner.view.get().state()
    }

    pub fn view_info(&self) -> ViewInfo {
        self.inner.info()
    }

    /// Whether a button is down and the chart is being dragged (or something is being dragged on it).
    pub fn dragging(&self) -> bool {
        self.inner.gesture.borrow().is_some()
    }

    pub fn size(&self) -> (f64, f64) {
        self.inner.size.get()
    }

    pub fn set_view(&self, s: ViewState) {
        self.inner.view.set(View::from_state(s));
        self.inner.changed();
    }

    /// Frames `b` in the visible part of the chart (see `set_insets`).
    pub fn fit_bounds(&self, b: &Bounds) {
        let (w, h) = self.inner.size.get();
        let (_, _, vw, vh) = self.visible_rect();
        let (cx, cy) = self.visible_centre();
        let mut v = self.inner.view.get();
        v.fit(b, vw, vh);
        let centre = v.center;
        v.place(centre, v.zoom, cx, cy, w, h);
        self.inner.view.set(v);
        self.inner.changed();
    }

    /// Zooms by `delta` about the middle of the visible part of the chart (the zoom buttons).
    pub fn zoom_by(&self, delta: f64) {
        let (x, y) = self.visible_centre();
        self.zoom_to(self.inner.view.get().zoom + delta, x, y);
    }

    /// Sets the zoom, keeping the position under widget point (`x`, `y`) fixed.
    pub fn zoom_to(&self, zoom: f64, x: f64, y: f64) {
        let (w, h) = self.inner.size.get();
        let mut v = self.inner.view.get();
        v.zoom_at(zoom, x, y, w, h);
        self.inner.view.set(v);
        self.inner.changed();
    }

    /// Widget coordinates of a position, if it is on screen.
    pub fn screen_of(&self, p: &LatLon) -> Option<(f64, f64)> {
        let vi = self.inner.info();
        let (x, y) = vi.to_screen(p);
        ((0.0..=vi.w).contains(&x) && (0.0..=vi.h).contains(&y)).then_some((x, y))
    }

    /// Puts `p` in the middle of the visible part of the chart (see `set_insets`).
    pub fn center_on(&self, p: LatLon) {
        let (w, h) = self.inner.size.get();
        let (x, y) = self.visible_centre();
        let mut v = self.inner.view.get();
        v.place(p, v.zoom, x, y, w, h);
        self.inner.view.set(v);
        self.inner.changed();
    }

    /// Adds a layer on top of the existing ones.
    pub fn add_layer(&self, layer: Rc<dyn Layer>) {
        self.inner.layers.borrow_mut().push(layer);
        self.queue_draw();
    }

    pub fn set_layer_visible(&self, id: &str, visible: bool) {
        let mut h = self.inner.hidden.borrow_mut();
        if visible {
            h.remove(id);
        } else {
            h.insert(id.to_string());
        }
        drop(h);
        self.queue_draw();
        for cb in self.inner.on_layers.borrow().iter() {
            cb();
        }
    }

    pub fn layer_visible(&self, id: &str) -> bool {
        !self.inner.hidden.borrow().contains(id)
    }

    /// A handle layers use to ask for a repaint after background work. It
    /// does not keep the engine alive.
    pub fn redraw_handle(&self) -> Redraw {
        let weak = Rc::downgrade(&self.inner);
        Rc::new(move || {
            if let Some(i) = weak.upgrade() {
                i.request_redraw();
            }
        })
    }

    pub fn mode(&self) -> Mode {
        self.inner.mode.get()
    }

    pub fn set_mode(&self, m: Mode) {
        self.inner.mode.set(m);
        self.queue_draw();
    }

    /// Called whenever the centre or zoom changes.
    pub fn connect_view_changed(&self, f: impl Fn(ViewState) + 'static) {
        self.inner.on_view.borrow_mut().push(Box::new(f));
    }

    /// Called whenever a layer is shown or hidden.
    pub fn connect_layers_changed(&self, f: impl Fn() + 'static) {
        self.inner.on_layers.borrow_mut().push(Box::new(f));
    }

    /// Calls `f` with whether layer `id` is shown, now and whenever a layer is shown or hidden.
    pub fn follow_layer_visible(&self, id: &'static str, f: impl Fn(bool) + 'static) {
        f(self.layer_visible(id));
        let weak = Rc::downgrade(&self.inner);
        self.connect_layers_changed(move || {
            if let Some(i) = weak.upgrade() {
                f(!i.hidden.borrow().contains(id));
            }
        });
    }

    /// Called with the position under the pointer, or `None` when it leaves.
    pub fn connect_pointer_moved(&self, f: impl Fn(Option<LatLon>) + 'static) {
        self.inner.on_pointer.borrow_mut().push(Box::new(f));
    }

    /// Lets something other than panning claim a primary-button drag. The
    /// hook gets `Begin` with the press position and returns whether it wants
    /// the drag; if so it then gets `Update` and `End` instead of the pan.
    pub fn connect_drag(&self, f: impl Fn(DragPhase, LatLon) -> bool + 'static) {
        self.inner.on_drag.borrow_mut().push(Box::new(f));
    }

    /// Sees key presses before the engine's own handling; return true to
    /// consume the key.
    pub fn connect_key(&self, f: impl Fn(Key) -> bool + 'static) {
        self.inner.on_key.borrow_mut().push(Box::new(f));
    }

    pub fn connect_click(&self, f: impl Fn(Click) + 'static) {
        self.inner.on_click.borrow_mut().push(Box::new(f));
    }

    /// Starts keeping frame times (for `ISOMAR_BENCH`), or stops and returns
    /// them, each with whether the static layers were redrawn in it.
    pub fn record_frames(&self, on: bool) -> Vec<(Duration, bool)> {
        let mut f = self.inner.frames.borrow_mut();
        let out = f.take().unwrap_or_default();
        if on {
            *f = Some(Vec::new());
        }
        out
    }

    /// Metres per pixel at the centre of the view.
    pub fn metres_per_pixel(&self) -> f64 {
        self.inner.info().metres_per_px()
    }

    // Input. Positions are in the chart's points, from its top-left corner.

    /// A button went down at (`x`, `y`). Primary and middle buttons pan; the
    /// secondary button is for menus. The primary button may be claimed by a
    /// drag hook (moving a waypoint).
    pub fn drag_begin(&self, button: u32, x: f64, y: f64) {
        let inner = &self.inner;
        inner.travelled.set(0.0);
        inner.fling.set(None);
        let touch = inner.touch.get().is_some();
        let mut claimed = false;
        // With a mouse a mark is grabbed at once; with a finger only after it is held (see drag_update).
        if button == 1 && !touch {
            let (w, h) = inner.size.get();
            let ll = inner.view.get().at(x, y, w, h);
            claimed = inner.on_drag.borrow().iter().any(|f| f(DragPhase::Begin, ll));
        }
        let from = if touch { None } else { Some((0.0, 0.0)) };
        let now = Instant::now();
        let began = inner.down_at.take().filter(|t| now.duration_since(*t) < Duration::from_secs(5)).unwrap_or(now);
        *inner.gesture.borrow_mut() = Some(Gesture { at: (x, y), start_center: inner.view.get().center_px(), claimed, began, from });
    }

    /// A finger touched the chart. A shell whose pan begins only once the finger has moved says so here, so
    /// that how long it rested first (on a mark: a drag of the mark) is known.
    pub fn touch_down(&self) {
        self.inner.down_at.set(Some(Instant::now()));
        self.inner.fling.set(None);
    }

    /// The pointer is (`dx`, `dy`) from where the button went down.
    pub fn drag_update(&self, dx: f64, dy: f64) {
        let inner = &self.inner;
        inner.travelled.set(inner.travelled.get().max(dx.hypot(dy)));
        let Some((at, start, mut claimed, began, mut from)) = inner.gesture.borrow().as_ref().map(|g| (g.at, g.start_center, g.claimed, g.began, g.from)) else { return };
        let (w, h) = inner.size.get();
        if from.is_none() {
            // A finger: nothing moves until it has gone past the slop. Then a finger that was held on a
            // mark drags it, and any other pans the chart from here.
            if dx.hypot(dy) <= TOUCH_SLOP {
                return;
            }
            if began.elapsed() >= HOLD {
                let ll = inner.view.get().at(at.0, at.1, w, h);
                claimed = inner.on_drag.borrow().iter().any(|f| f(DragPhase::Begin, ll));
            }
            from = Some((dx, dy));
            if let Some(g) = inner.gesture.borrow_mut().as_mut() {
                g.claimed = claimed;
                g.from = from;
            }
        }
        if claimed {
            let ll = inner.view.get().at(at.0 + dx, at.1 + dy, w, h);
            for f in inner.on_drag.borrow().iter() {
                f(DragPhase::Update, ll);
            }
            return;
        }
        let (fx, fy) = from.unwrap_or((0.0, 0.0));
        let mut v = inner.view.get();
        v.set_center_px(start.0 - (dx - fx), start.1 - (dy - fy));
        inner.view.set(v);
        self.set_follow(false);
        inner.changed();
    }

    pub fn drag_end(&self, dx: f64, dy: f64) {
        let inner = &self.inner;
        let Some(g) = inner.gesture.borrow_mut().take() else { return };
        if !g.claimed {
            return;
        }
        let (w, h) = inner.size.get();
        let ll = inner.view.get().at(g.at.0 + dx, g.at.1 + dy, w, h);
        for f in inner.on_drag.borrow().iter() {
            f(DragPhase::End, ll);
        }
    }

    /// A pan ended with the finger moving at (`vx`, `vy`) points per second: the chart glides on and slows
    /// (drawn by `frame`). A touch or a pinch stops it.
    pub fn fling(&self, vx: f64, vy: f64) {
        let inner = &self.inner;
        if !vx.is_finite() || !vy.is_finite() || vx.hypot(vy) < FLING_MIN * 4.0 || inner.reduce_motion.get() {
            return;
        }
        inner.fling.set(Some(Fling { v: (vx, vy), at: Instant::now() }));
        inner.request_redraw();
    }

    /// Moves the view on by what the fling covered since it last did; whether it is still gliding.
    fn glide(&self) -> bool {
        let inner = &self.inner;
        let Some(f) = inner.fling.get() else { return false };
        let now = Instant::now();
        let dt = now.duration_since(f.at).as_secs_f64().min(0.1);
        let k = (-FLING_DECAY * dt).exp();
        // The distance covered over dt as the speed decays exponentially.
        let travel = (1.0 - k) / FLING_DECAY;
        let mut view = inner.view.get();
        let (cx, cy) = view.center_px();
        view.set_center_px(cx - f.v.0 * travel, cy - f.v.1 * travel);
        inner.view.set(view);
        let v = (f.v.0 * k, f.v.1 * k);
        let going = v.0.hypot(v.1) >= FLING_MIN;
        inner.fling.set(going.then_some(Fling { v, at: now }));
        self.set_follow(false);
        inner.changed();
        going
    }

    /// Keeps the boat centred, or stops. Shells hear of it through `on_follow_changed` (panning by hand ends it).
    pub fn set_follow(&self, on: bool) {
        if self.inner.follow.replace(on) != on {
            for f in self.inner.on_follow.borrow().iter() {
                f(on);
            }
        }
    }

    pub fn following(&self) -> bool {
        self.inner.follow.get()
    }

    pub fn on_follow_changed(&self, f: impl Fn(bool) + 'static) {
        self.inner.on_follow.borrow_mut().push(Box::new(f));
    }

    /// A wheel or trackpad scroll of `dy` notches (down is positive) with
    /// the pointer at (`x`, `y`) zooms about that point.
    pub fn scroll_zoom(&self, dy: f64, x: f64, y: f64) {
        let inner = &self.inner;
        let (w, h) = inner.size.get();
        let mut v = inner.view.get();
        v.zoom_at(v.zoom - dy * 0.5, x, y, w, h);
        inner.view.set(v);
        inner.changed();
    }

    /// A pinch began.
    pub fn magnify_begin(&self) {
        self.inner.fling.set(None);
        self.inner.pinch_base.set(self.inner.view.get().zoom);
        self.inner.pinch_anchor.set(None);
    }

    /// The pinch is `scale` times its start, with the fingers' midpoint at (`x`, `y`). The position that
    /// was under the midpoint when the pinch began stays under it, so two fingers moving together pan too.
    /// While the view follows the boat it zooms about the boat instead, which the next fix centres again
    /// anyway: zooming about the fingers would jump back to the boat then.
    pub fn magnify(&self, scale: f64, x: f64, y: f64) {
        // A shell's running product of steps can reach nothing or below: its logarithm would make the zoom
        // NaN, and every position after it.
        if !(scale.is_finite() && scale > 0.0 && x.is_finite() && y.is_finite()) {
            return;
        }
        let inner = &self.inner;
        let (w, h) = inner.size.get();
        let (x, y) = if self.following() { self.visible_centre() } else { (x, y) };
        let mut v = inner.view.get();
        let anchor = inner.pinch_anchor.get().unwrap_or_else(|| {
            let a = v.at(x, y, w, h);
            inner.pinch_anchor.set(Some(a));
            a
        });
        v.place(anchor, inner.pinch_base.get() + scale.log2(), x, y, w, h);
        inner.view.set(v);
        inner.changed();
    }

    /// The pointer moved to (`x`, `y`), or left the chart.
    pub fn pointer_moved(&self, at: Option<(f64, f64)>) {
        let inner = &self.inner;
        let (w, h) = inner.size.get();
        let ll = at.map(|(x, y)| inner.view.get().at(x, y, w, h));
        for cb in inner.on_pointer.borrow().iter() {
            cb(ll);
        }
    }

    /// A button was released after a click at (`x`, `y`): 1 primary, 2
    /// middle, 3 secondary.
    pub fn click(&self, button: u32, n_press: i32, x: f64, y: f64) {
        let inner = &self.inner;
        // GTK reports the release that ends a drag as a click too; only a press that stayed put is one.
        let slop = if inner.touch.get().is_some() { TOUCH_SLOP } else { CLICK_SLOP };
        if inner.travelled.get() > slop {
            return;
        }
        self.clicked(button, n_press, x, y);
    }

    /// A tap at (`x`, `y`), as a touch shell's gesture recogniser decided it was one (`n` taps): a click,
    /// whatever the last drag did.
    pub fn tap(&self, button: u32, n: i32, x: f64, y: f64) {
        self.inner.travelled.set(0.0);
        self.clicked(button, n, x, y);
    }

    fn clicked(&self, button: u32, n_press: i32, x: f64, y: f64) {
        let inner = &self.inner;
        let (w, h) = inner.size.get();
        let pos = inner.view.get().at(x, y, w, h);
        let ev = Click { button, n_press, pos, x, y };
        for cb in inner.on_click.borrow().iter() {
            cb(ev);
        }
    }

    /// A key was pressed. Returns whether the engine used it.
    pub fn key(&self, key: Key) -> bool {
        let inner = &self.inner;
        if inner.on_key.borrow().iter().any(|f| f(key)) {
            return true;
        }
        let (w, h) = inner.size.get();
        let mut v = inner.view.get();
        let step = 80.0;
        let (cx, cy) = v.center_px();
        let (mx, my) = self.visible_centre();
        match key {
            Key::ZoomIn => v.zoom_at(v.zoom + 1.0, mx, my, w, h),
            Key::ZoomOut => v.zoom_at(v.zoom - 1.0, mx, my, w, h),
            Key::Left => v.set_center_px(cx - step, cy),
            Key::Right => v.set_center_px(cx + step, cy),
            Key::Up => v.set_center_px(cx, cy - step),
            Key::Down => v.set_center_px(cx, cy + step),
            _ => return false,
        }
        inner.view.set(v);
        inner.changed();
        true
    }
}

pub(crate) fn paint_background(cr: &cairo::Context, vi: &ViewInfo, pal: &Palette) {
    let size = geo::world_size(vi.zoom());
    let (_, cy) = vi.view.center_px();
    let top = cy - vi.h / 2.0;
    pal.set(cr, VOID);
    let _ = cr.paint();

    // The sea fills the world; beyond the pole limits there is only void.
    let y0 = (-top).max(0.0);
    let y1 = (size - top).min(vi.h);
    if y1 > y0 {
        pal.set(cr, SEA);
        cr.rectangle(0.0, y0, vi.w, y1 - y0);
        let _ = cr.fill();
    }
}

fn draw_layers(inner: &Inner, cr: &cairo::Context, vi: ViewInfo, pal: Palette, dynamic: bool) {
    let overlay = Overlay::default();
    let ctx = DrawCtx { cr, vi, pal, overlay: &overlay, reduce_motion: inner.reduce_motion.get(), top_inset: inner.insets.get()[0] };
    // Clone the list so a layer may add layers while drawing.
    let layers: Vec<Rc<dyn Layer>> = inner.layers.borrow().clone();
    for l in layers.iter().filter(|l| l.is_dynamic() == dynamic) {
        if inner.hidden.borrow().contains(l.id()) {
            continue;
        }
        let _ = cr.save();
        l.draw(&ctx);
        let _ = cr.restore();
    }
    if dynamic {
        layers::overlay::flush(cr, &pal, &overlay, (vi.w, vi.h));
    }
}

/// The static layers are drawn as tiles (`render`), by workers, and kept; here the tiles are planned,
/// laid over the background, and the dynamic layers and the scale bar drawn over them. Returns whether
/// tiles were planned in this frame.
fn draw(inner: &Rc<Inner>, cr: &cairo::Context, w: f64, h: f64, scale: i32, pal: Palette) -> bool {
    let vi = ViewInfo { view: inner.view.get(), w, h };
    let planned = plan_tiles(inner, vi, scale, pal);
    paint_background(cr, &vi, &pal);
    for t in inner.tiles.borrow().iter() {
        t.composite(cr, &vi, scale);
    }
    draw_layers(inner, cr, vi, pal, true);
    if !inner.footer_native.get() {
        draw_scale_bar(&vi, cr, &pal, h);
    }
    planned
}

/// Plans the tiles for the view, and a redraw for when those held back may be drawn. Returns whether
/// tiles were planned.
fn plan_tiles(inner: &Rc<Inner>, vi: ViewInfo, scale: i32, pal: Palette) -> bool {
    let mut hidden: Vec<String> = inner.hidden.borrow().iter().cloned().collect();
    hidden.sort();
    let look = Look { scale, text_scale: layers::text_scale(), tone: pal.tone, hidden };
    let statics: Vec<Rc<dyn Layer>> = inner.layers.borrow().iter().filter(|l| !l.is_dynamic() && !inner.hidden.borrow().contains(l.id())).cloned().collect();
    let mut planned = render::Planned::default();
    for (i, plane) in inner.tiles.borrow_mut().iter_mut().enumerate() {
        let layers: Vec<Rc<dyn Layer>> = statics.iter().filter(|l| l.plane() as usize == i).cloned().collect();
        if layers.is_empty() {
            // Nothing is drawn into it (the weather hidden): it holds nothing.
            plane.clear();
            continue;
        }
        let p = plane.update(&Frame { vi, pal, look: look.clone(), layers: &layers });
        planned.any |= p.any;
        planned.again_in = match (planned.again_in, p.again_in) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
    }
    drop(statics);
    if let Some(d) = planned.again_in
        && !inner.wake_pending.replace(true)
        && platform::installed()
    {
        // Tiles were held back while the zoom or the data were moving: look again when they may be still.
        let weak = Rc::downgrade(inner);
        platform::timeout_once(d + Duration::from_millis(1), move || {
            if let Some(inner) = weak.upgrade() {
                inner.wake_pending.set(false);
                inner.request_redraw();
            }
        });
    }
    planned.any
}

/// A frame for a shell that composites on the GPU: what `Engine::draw` would paint, as parts.
pub struct Scene<'a> {
    /// Device pixels to a point; tiles and chunks are this many pixels to a point.
    pub scale: i32,
    /// Beyond the poles.
    pub void: Rgba,
    /// The world's background, from `sea_band.0` to `sea_band.1` points down the view.
    pub sea: Rgba,
    pub sea_band: (f64, f64),
    /// The chart's tiles, back to front, in points.
    pub tiles: Vec<Placement<'a>>,
    /// The chunks of the dynamic layers with something on them, over the tiles, in device pixels.
    pub overlay: Vec<Chunk>,
    buffer: &'a OverlayBuffer,
}

impl Scene<'_> {
    /// The overlay's pixels (cairo's ARGB32, premultiplied, native byte order) and the bytes to a row;
    /// a chunk's pixels start at `y * stride + x * 4`.
    pub fn with_overlay<R>(&self, f: impl FnOnce(&[u8], usize) -> R) -> Option<R> {
        self.buffer.with_pixels(f)
    }

    /// Copies a chunk's pixels (cairo's ARGB32, premultiplied, native byte order), rows packed.
    pub fn chunk_pixels(&self, c: &Chunk) -> Vec<u8> {
        self.buffer
            .with_pixels(|data, stride| {
                let mut out = Vec::with_capacity(c.w * c.h * 4);
                for row in c.y..c.y + c.h {
                    out.extend_from_slice(&data[row * stride + c.x * 4..row * stride + (c.x + c.w) * 4]);
                }
                out
            })
            .unwrap_or_default()
    }
}

/// Picks a round length for the scale bar: nautical miles when large,
/// metres when zoomed in far enough that a quarter mile would not fit.
pub fn scale_bar_length(metres_per_px: f64, max_px: f64) -> (f64, String) {
    const NM: [f64; 13] = [0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0];
    const M: [f64; 7] = [5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0];
    let max_m = metres_per_px * max_px;
    if max_m >= 0.25 * geo::NM_M {
        let nm = NM.iter().copied().rfind(|n| n * geo::NM_M <= max_m).unwrap_or(NM[0]);
        (nm * geo::NM_M / metres_per_px, format!("{} nm", nm))
    } else {
        let m = M.iter().copied().rfind(|m| *m <= max_m).unwrap_or(M[0]);
        (m / metres_per_px, format!("{} m", m))
    }
}

fn draw_scale_bar(vi: &ViewInfo, cr: &cairo::Context, pal: &Palette, h: f64) {
    let (len, label) = scale_bar_length(vi.metres_per_px(), 150.0);
    let (x, y) = (16.0, h - 20.0);
    cr.select_font_face(crate::layers::FONT_FAMILY, cairo::FontSlant::Normal, cairo::FontWeight::Normal);
    layers::font_size(cr, 11.0);
    pal.set(cr, TEXT);
    cr.set_line_width(2.0);
    cr.move_to(x, y - 5.0);
    cr.line_to(x, y);
    cr.line_to(x + len, y);
    cr.line_to(x + len, y - 5.0);
    let _ = cr.stroke();
    cr.move_to(x, y - 9.0);
    let _ = cr.show_text(&label);
}


#[cfg(test)]
mod tests {
    use super::*;

    /// A dynamic layer that fills a square of 10 points at (300, 200).
    struct Square;

    impl Layer for Square {
        fn id(&self) -> &'static str {
            "square"
        }
        fn is_dynamic(&self) -> bool {
            true
        }
        fn draw(&self, ctx: &DrawCtx) {
            ctx.cr.set_source_rgb(1.0, 0.0, 0.0);
            ctx.cr.rectangle(300.0, 200.0, 10.0, 10.0);
            let _ = ctx.cr.fill();
        }
    }

    fn engine(w: f64, h: f64) -> Engine {
        let e = Engine::new(ViewState { lat: 47.6, lon: -3.0, zoom: 12.0 }, Mode::Day);
        e.resize(w, h);
        e
    }

    fn near(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 0.5 && (a.1 - b.1).abs() < 0.5
    }

    #[test]
    fn a_pinch_keeps_what_was_under_the_fingers_under_them_as_they_move() {
        let e = engine(400.0, 800.0);
        let under = e.view_info().view.at(100.0, 300.0, 400.0, 800.0);
        e.magnify_begin();
        e.magnify(1.0, 100.0, 300.0);
        e.magnify(2.0, 160.0, 380.0);
        assert!((e.view_state().zoom - 13.0).abs() < 1e-9);
        let at = e.view_info().to_screen(&under);
        assert!(near(at, (160.0, 380.0)), "{at:?}");
    }

    #[test]
    fn a_pinch_to_nothing_leaves_the_view_alone() {
        let e = engine(400.0, 800.0);
        e.magnify_begin();
        for s in [0.5, 0.0, -0.2, f64::NAN, f64::INFINITY] {
            e.magnify(s, 100.0, 300.0);
        }
        let v = e.view_state();
        assert!((v.zoom - 11.0).abs() < 1e-9 && v.lat.is_finite() && v.lon.is_finite(), "{v:?}");
    }

    #[test]
    fn a_tap_is_a_click_however_far_the_last_pan_went_and_a_touch_wobble_is_not_a_drag() {
        let e = engine(400.0, 800.0);
        let clicks = Rc::new(Cell::new(0));
        let c = clicks.clone();
        e.connect_click(move |_| c.set(c.get() + 1));
        e.drag_begin(1, 100.0, 100.0);
        e.drag_update(120.0, 0.0);
        e.drag_end(120.0, 0.0);
        e.click(1, 1, 220.0, 100.0);
        assert_eq!(clicks.get(), 0, "the release ending a pan is no click");
        e.tap(1, 1, 220.0, 100.0);
        assert_eq!(clicks.get(), 1);
        // A press that wobbles 7 points: a drag for a mouse, a tap for a finger.
        e.drag_begin(1, 100.0, 100.0);
        e.drag_update(7.0, 0.0);
        e.click(1, 1, 107.0, 100.0);
        assert_eq!(clicks.get(), 1);
        e.set_touch(Some(crate::tokens::HIT_MIN_TOUCH));
        e.drag_begin(1, 100.0, 100.0);
        e.drag_update(7.0, 0.0);
        e.click(1, 1, 107.0, 100.0);
        assert_eq!(clicks.get(), 2);
        assert_eq!(e.hit_min(), crate::tokens::HIT_MIN_TOUCH);
        e.set_underway(true);
        assert_eq!(e.hit_min(), crate::tokens::HIT_MIN_TOUCH.max(crate::tokens::HIT_MIN_UNDERWAY));
    }

    #[test]
    fn a_finger_pans_after_the_slop_drags_a_mark_only_once_held_and_ends_following() {
        let e = engine(400.0, 800.0);
        e.set_touch(Some(crate::tokens::HIT_MIN_TOUCH));
        let grabbed = Rc::new(Cell::new(0));
        let g = grabbed.clone();
        e.connect_drag(move |phase, _| {
            if matches!(phase, DragPhase::Begin) {
                g.set(g.get() + 1);
            }
            true
        });
        let follows = Rc::new(Cell::new(None));
        let f = follows.clone();
        e.on_follow_changed(move |on| f.set(Some(on)));
        e.set_follow(true);
        assert_eq!(follows.get(), Some(true));
        let start = e.view_info().view.center_px();
        // A quick swipe pans, from where the finger passed the slop: no jump, no mark grabbed.
        e.drag_begin(1, 100.0, 100.0);
        e.drag_update(6.0, 0.0);
        assert_eq!(e.view_info().view.center_px(), start, "inside the slop nothing moves");
        e.drag_update(20.0, 0.0);
        e.drag_update(50.0, 0.0);
        e.drag_end(50.0, 0.0);
        assert_eq!(grabbed.get(), 0);
        assert!((start.0 - e.view_info().view.center_px().0 - 30.0).abs() < 1e-6, "panned 50 - 20");
        assert_eq!(follows.get(), Some(false), "panning by hand ends following");
        assert!(!e.following());
        // Held first, the same move drags what is under the finger.
        e.drag_begin(1, 100.0, 100.0);
        std::thread::sleep(HOLD + Duration::from_millis(20));
        e.drag_update(30.0, 0.0);
        e.drag_end(30.0, 0.0);
        assert_eq!(grabbed.get(), 1);
    }

    #[test]
    fn a_pinch_while_following_zooms_about_the_boat_not_the_fingers() {
        let e = engine(400.0, 800.0);
        let centre = e.view_info().view.center_px();
        let zoom = e.view_info().zoom();
        e.set_follow(true);
        e.magnify_begin();
        e.magnify(1.0, 50.0, 100.0);
        e.magnify(2.0, 60.0, 120.0);
        let v = e.view_info().view;
        assert!((v.zoom - zoom - 1.0).abs() < 1e-9);
        assert!((v.center_px().0 - 2.0 * centre.0).abs() < 1e-6 && (v.center_px().1 - 2.0 * centre.1).abs() < 1e-6, "the centre stays put");
        assert!(e.following());
    }

    #[test]
    fn a_fling_glides_on_slower_and_slower_and_a_touch_stops_it() {
        let e = engine(400.0, 800.0);
        let start = e.view_info().view.center_px();
        e.fling(1000.0, 0.0);
        assert!(e.animating());
        std::thread::sleep(Duration::from_millis(30));
        assert!(e.glide());
        let first = start.0 - e.view_info().view.center_px().0;
        assert!(first > 10.0 && first < 40.0, "{first}");
        e.drag_begin(1, 0.0, 0.0);
        assert!(!e.animating(), "a touch stops the glide");
        // Left alone it comes to rest: its whole travel is the speed over the decay.
        e.fling(1000.0, 0.0);
        let mut steps = 0;
        while e.glide() {
            std::thread::sleep(Duration::from_millis(20));
            steps += 1;
            assert!(steps < 200);
        }
        assert!(!e.animating());
    }

    #[test]
    fn centring_and_zooming_use_the_part_a_bottom_sheet_leaves_visible() {
        let e = engine(400.0, 800.0);
        e.set_insets(50.0, 0.0, 350.0, 0.0);
        assert_eq!(e.visible_centre(), (200.0, 250.0));
        let p = LatLon::new(47.7, -3.1);
        e.center_on(p);
        assert!(near(e.view_info().to_screen(&p), (200.0, 250.0)));
        e.zoom_by(1.0);
        assert!(near(e.view_info().to_screen(&p), (200.0, 250.0)), "the zoom buttons keep the visible middle still");
        // A sheet over nearly all of the chart: the middle of the whole chart is used.
        e.set_insets(0.0, 0.0, 700.0, 0.0);
        assert_eq!(e.visible_centre(), (200.0, 400.0));
        // A box is framed in what is visible, not under the sheet.
        e.set_insets(0.0, 0.0, 400.0, 0.0);
        let b = Bounds { min_lat: 47.5, max_lat: 47.6, min_lon: -3.2, max_lon: -3.0 };
        e.fit_bounds(&b);
        let (_, top) = e.view_info().to_screen(&LatLon::new(b.max_lat, b.min_lon));
        let (_, bottom) = e.view_info().to_screen(&LatLon::new(b.min_lat, b.min_lon));
        assert!(top >= 0.0 && bottom <= 400.0, "{top}..{bottom}");
    }

    #[test]
    fn a_frame_hands_over_the_chunks_the_dynamic_layers_drew_on_at_the_device_scale() {
        let e = Engine::new(ViewState { lat: 47.6, lon: -3.0, zoom: 12.0 }, Mode::Day);
        e.set_footer_native(true);
        e.add_layer(Rc::new(Square));
        for scale in [1, 2] {
            let found = e.frame(800.0, 600.0, scale, false, |scene| {
                let s = scale as usize;
                let (x, y) = (305 * s, 205 * s);
                let c = scene.overlay.iter().find(|c| (c.x..c.x + c.w).contains(&x) && (c.y..c.y + c.h).contains(&y))?;
                let px = scene.chunk_pixels(c);
                let at = ((y - c.y) * c.w + (x - c.x)) * 4;
                Some((scene.overlay.len(), px[at + 3]))
            });
            assert_eq!(found, Some((1, 255)), "scale {scale}");
        }
        e.set_layer_visible("square", false);
        assert_eq!(e.frame(800.0, 600.0, 2, false, |scene| scene.overlay.len()), 0);
    }

    #[test]
    fn scale_bar_switches_between_miles_and_metres() {
        let (len, label) = scale_bar_length(100.0, 150.0);
        assert_eq!(label, "5 nm");
        assert!(len <= 150.0);
        let (_, label) = scale_bar_length(0.5, 150.0);
        assert_eq!(label, "50 m");
    }
}
