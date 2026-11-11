//! A C interface to `isomar-app` for the shells that are not GTK: the Apple app (macOS, iOS, iPadOS) and,
//! through crates/isomar-android, the Android one. See `include/isomar.h`.
//!
//! The application's state is `Rc`-based and lives on the main thread, so it
//! is kept in a thread-local; calling anything but the wake callback from
//! another thread finds nothing there and does nothing.

use isomar_app::app::AppState;
use isomar_app::config::Settings;
use isomar_app::core::datadir;
use isomar_app::core::nav::store::NavStore;
use isomar_app::core::session::Session;
use isomar_app::debug;
use isomar_app::engine::Engine;
use isomar_app::host;
use isomar_app::key::Key;
use isomar_app::palette::Mode;
use isomar_app::pump;
use isomar_app::shell::{Dialog, Menu, Shell};
use isomar_app::vm;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_void};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

type WakeFn = extern "C" fn(*mut c_void, u64);
type RedrawFn = extern "C" fn(*mut c_void);
type ChangedFn = extern "C" fn(*mut c_void, i32);
type UiFn = extern "C" fn(*mut c_void, i32, *const c_char);

const TOPIC_POINTER: i32 = 1;
const TOPIC_VIEW: i32 = 2;
const TOPIC_STATUS: i32 = 3;
const TOPIC_SESSION: i32 = 4;
const TOPIC_LAYERS: i32 = 5;
const TOPIC_DATA: i32 = 6;
const TOPIC_LIVE: i32 = 7;

const UI_TOAST: i32 = 1;
const UI_ERROR: i32 = 2;
const UI_BEEP: i32 = 3;
const UI_CONFIRM: i32 = 4;
const UI_MENU: i32 = 5;
const UI_INFO: i32 = 6;
const UI_CLIPBOARD: i32 = 7;
const UI_DIALOG: i32 = 8;
const UI_CURSOR: i32 = 9;
const UI_NOTIFY: i32 = 10;

/// The shell's context pointer, which the shell promises is usable from any thread.
#[derive(Clone, Copy)]
struct Ctx(*mut c_void);
unsafe impl Send for Ctx {}
unsafe impl Sync for Ctx {}
impl Ctx {
    fn get(self) -> *mut c_void {
        self.0
    }
}

#[derive(Default)]
struct Texts {
    pointer: String,
    view: String,
    status: String,
    subtitle: String,
}

struct Host {
    app: Rc<AppState>,
    ctx: Ctx,
    changed: ChangedFn,
    texts: Rc<RefCell<Texts>>,
    alerts: RefCell<vm::banner::Alerts>,
}

/// What a context menu's entries do, by the index the shell passes back.
type MenuActions = RefCell<Vec<Rc<dyn Fn()>>>;

/// The shell the macOS app gives the engine: each request becomes an event for Swift, as JSON.
struct HostShell {
    ctx: Ctx,
    ui: UiFn,
}

impl HostShell {
    fn send(&self, kind: i32, json: serde_json::Value) {
        let text = CString::new(json.to_string().replace('\0', "")).unwrap_or_default();
        (self.ui)(self.ctx.get(), kind, text.as_ptr());
    }
}

impl Shell for HostShell {
    fn toast(&self, text: &str) {
        self.send(UI_TOAST, serde_json::json!({ "text": text }));
    }

    fn show_error(&self, heading: &str, details: &str) {
        self.send(UI_ERROR, serde_json::json!({ "heading": heading, "details": details }));
    }

    fn sound(&self, sound: isomar_app::shell::Sound) {
        use isomar_app::shell::Sound;
        let name = match sound {
            Sound::Alarm => "alarm",
            Sound::Chime => "chime",
            Sound::Tick => "tick",
        };
        self.send(UI_BEEP, serde_json::json!({ "sound": name }));
    }

    fn notify(&self, title: &str, body: &str) {
        self.send(UI_NOTIFY, serde_json::json!({ "title": title, "body": body }));
    }

    fn confirm(&self, heading: &str, body: &str, ok_label: &str, destructive: bool) -> Pin<Box<dyn Future<Output = bool>>> {
        let (tx, rx) = async_channel::bounded(1);
        let id = CONFIRM_ID.with(|n| n.replace(n.get() + 1));
        CONFIRMS.with(|c| c.borrow_mut().insert(id, tx));
        self.send(UI_CONFIRM, serde_json::json!({ "id": id, "heading": heading, "body": body, "ok": ok_label, "destructive": destructive }));
        Box::pin(async move { rx.recv().await.unwrap_or(false) })
    }

    fn context_menu(&self, menu: Menu, x: f64, y: f64) {
        let mut actions = Vec::new();
        let sections: Vec<Vec<serde_json::Value>> = menu
            .sections
            .iter()
            .map(|s| {
                s.iter()
                    .map(|item| {
                        actions.push(item.run.clone());
                        serde_json::json!({ "i": actions.len() - 1, "label": item.label })
                    })
                    .collect()
            })
            .collect();
        MENU.with(|m| *m.borrow_mut() = actions);
        self.send(UI_MENU, serde_json::json!({ "x": x, "y": y, "title": menu.title, "sections": sections }));
    }

    fn object_info(&self, info: &isomar_app::aidinfo::Description, x: f64, y: f64) {
        self.send(UI_INFO, serde_json::json!({ "x": x, "y": y, "title": info.title, "subtitle": info.subtitle, "rows": info.rows, "choice": info.choice.map(|(n, of)| [n, of]) }));
    }

    fn set_cursor(&self, pointing: bool) {
        self.send(UI_CURSOR, serde_json::json!({ "pointing": pointing }));
    }

    fn open(&self, dialog: Dialog) {
        let v = match dialog {
            Dialog::WaypointProperties(id) => serde_json::json!({ "name": "waypoint_properties", "id": id }),
            Dialog::RouteProperties(id) => serde_json::json!({ "name": "route_properties", "id": id }),
            Dialog::AisInfo(mmsi) => serde_json::json!({ "name": "vessel", "id": mmsi }),
            Dialog::WeatherRoute(id) => serde_json::json!({ "name": "weather_route", "id": id }),
            Dialog::BestDeparture(id) => serde_json::json!({ "name": "best_departure", "id": id }),
            Dialog::AutoRoute(id) => serde_json::json!({ "name": "auto_route", "id": id }),
            Dialog::GetWeather { north, south, west, east } => serde_json::json!({ "name": "get_weather", "north": north, "south": south, "west": west, "east": east }),
            Dialog::Tides(p) => serde_json::json!({ "name": "tides", "lat": p.lat, "lon": p.lon }),
            Dialog::ChartsHere(p) => serde_json::json!({ "name": "charts_here", "lat": p.lat, "lon": p.lon }),
        };
        self.send(UI_DIALOG, v);
    }
}

thread_local! {
    static MENU: MenuActions = const { RefCell::new(Vec::new()) };
    static CONFIRMS: RefCell<HashMap<u64, async_channel::Sender<bool>>> = RefCell::new(HashMap::new());
    static CONFIRM_ID: Cell<u64> = const { Cell::new(1) };
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
    /// The last string handed out, kept alive until the next one.
    static OUT: RefCell<CString> = RefCell::new(CString::default());
}

fn with<R>(f: impl FnOnce(&Host) -> R) -> Option<R> {
    HOST.with(|m| m.borrow().as_ref().map(f))
}

/// The engine, without holding the thread-local while a callback runs.
fn engine() -> Option<Engine> {
    with(|m| m.app.canvas.clone())
}

fn out(s: &str) -> *const c_char {
    OUT.with(|o| {
        *o.borrow_mut() = CString::new(s.replace('\0', "")).unwrap_or_default();
        o.borrow().as_ptr()
    })
}

fn notify(ctx: Ctx, changed: ChangedFn, topic: i32) {
    changed(ctx.get(), topic);
}

#[unsafe(no_mangle)]
pub extern "C" fn isomar_start(ctx: *mut c_void, wake: WakeFn, redraw: RedrawFn, changed: ChangedFn, ui: UiFn) -> bool {
    if HOST.with(|m| m.borrow().is_some()) {
        return false;
    }
    let ctx = Ctx(ctx);
    isomar_app::core::log::install_panic_hook();
    isomar_app::core::datadir::migrate_legacy();
    isomar_app::i18n::init();
    isomar_app::core::log::write(&format!("isomar {} starting; log at {}", env!("CARGO_PKG_VERSION"), isomar_app::core::log::path().display()));
    let clipboard = HostShell { ctx, ui };
    pump::install(
        Arc::new(move |d: Duration| wake(ctx.get(), d.as_millis() as u64)),
        Box::new(move |text| clipboard.send(UI_CLIPBOARD, serde_json::json!({ "text": text }))),
    );

    let settings = Settings::load();
    let mode = settings.mode.parse::<Mode>().unwrap_or_default();
    let canvas = Engine::new(debug::forced_view().unwrap_or(settings.view), mode);
    canvas.set_tile_workers(isomar_app::render::default_workers());
    canvas.set_redraw(Rc::new(move || redraw(ctx.get())));
    let store = NavStore::open(&datadir::nav_db_path()).or_else(|_| NavStore::open_memory()).expect("in-memory database");
    let session = Session::new(store).expect("navigation data");
    let app = AppState::new(canvas.clone(), session, settings.clone());
    let texts = Rc::new(RefCell::new(Texts::default()));
    app.set_shell(Rc::new(HostShell { ctx, ui }));
    app.on_refresh(move || notify(ctx, changed, TOPIC_DATA));

    let status: isomar_app::data::Status = {
        let texts = texts.clone();
        Rc::new(move |s: &str| {
            texts.borrow_mut().status = s.to_string();
            notify(ctx, changed, TOPIC_STATUS);
        })
    };
    host::install(&app, &canvas, status, &settings);
    isomar_app::actions::install_interaction(&app);
    // The footer is the shell's: it draws the scale, the credits and the planning pill itself.
    canvas.set_footer_native(true);
    if let Some(c) = app.credit.borrow().as_ref() {
        c.hidden.set(true);
    }
    {
        let texts = texts.clone();
        let depth = app.depth.borrow().clone();
        let app = Rc::downgrade(&app);
        canvas.connect_pointer_moved(move |ll| {
            texts.borrow_mut().pointer = ll
                .map(|p| {
                    let d = depth.as_ref().and_then(|d| d.depth_at(p.lon, p.lat));
                    let v = app.upgrade().and_then(|a| isomar_app::variation::at(&a, &p)).map(|v| isomar_app::variation::text(&v));
                    [Some(isomar_app::text::position_text(&p)), d.map(|d| format!("{d:.1} m")), v].into_iter().flatten().collect::<Vec<_>>().join(", ")
                })
                .unwrap_or_default();
            notify(ctx, changed, TOPIC_POINTER);
        });
    }
    {
        let (texts, c) = (texts.clone(), canvas.clone());
        let update = move |zoom: f64| {
            texts.borrow_mut().view = host::view_text(&c, zoom);
            notify(ctx, changed, TOPIC_VIEW);
        };
        canvas.connect_view_changed(move |s| update(s.zoom));
    }
    texts.borrow_mut().view = host::view_text(&canvas, canvas.view_state().zoom);
    if host::start_connections(&app, &settings) {
        texts.borrow_mut().status = isomar_app::i18n::gettext("No data source: open the menu and choose Connections");
    }
    debug::seed_demo(&app);
    debug::seed_perf(&app);
    app.refresh();
    HOST.with(|m| *m.borrow_mut() = Some(Host { app, ctx, changed, texts, alerts: RefCell::new(Default::default()) }));
    true
}

#[unsafe(no_mangle)]
pub extern "C" fn isomar_shutdown() {
    if let Some(m) = HOST.with(|m| m.borrow_mut().take()) {
        host::shutdown(&m.app);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isomar_pump() {
    pump::run();
}

#[unsafe(no_mangle)]
pub extern "C" fn isomar_tick() {
    let Some(app) = with(|m| m.app.clone()) else { return };
    let notices = host::poll(&app);
    isomar_app::actions::announce(&app, &notices);
    if app.canvas.following()
        && let Some(p) = app.session.borrow().own.pos
    {
        app.canvas.center_on(p);
    }
    let subtitle = host::subtitle(&app).map(|c| c.label).unwrap_or_default();
    let alerts = vm::banner::alerts(&app);
    // Gone if the poll shut the app down.
    let Some((ctx, changed, subtitle_changed)) = with(|m| {
        let mut t = m.texts.borrow_mut();
        let different = t.subtitle != subtitle;
        t.subtitle = subtitle.clone();
        *m.alerts.borrow_mut() = alerts;
        (m.ctx, m.changed, different)
    }) else {
        return;
    };
    if subtitle_changed {
        notify(ctx, changed, TOPIC_SESSION);
    }
    notify(ctx, changed, TOPIC_LIVE);
    app.tick();
}

#[unsafe(no_mangle)]
pub extern "C" fn isomar_resize(w: f64, h: f64) {
    if let Some(e) = engine() {
        e.resize(w, h);
    }
}

/// # Safety
/// `data` must point to `h_px * stride` writable bytes that stay valid for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn isomar_render(data: *mut u8, w_px: i32, h_px: i32, stride: i32, scale: f64, dark: bool) {
    let Some(e) = engine() else { return };
    if data.is_null() || w_px <= 0 || h_px <= 0 {
        return;
    }
    let surface = match unsafe { cairo::ImageSurface::create_for_data_unsafe(data, cairo::Format::ARgb32, w_px, h_px, stride) } {
        Ok(s) => s,
        Err(err) => {
            isomar_app::core::log::write(&format!("isomar: cannot draw into the buffer: {err}"));
            return;
        }
    };
    let scale = scale.max(1.0);
    surface.set_device_scale(scale, scale);
    if let Ok(cr) = cairo::Context::new(&surface) {
        e.draw(&cr, w_px as f64 / scale, h_px as f64 / scale, scale.round() as i32, dark);
    }
    surface.flush();
}

/// A tile of the chart to place: see `ChartTile` in `isomar.h`.
#[repr(C)]
pub struct ChartTile {
    pub id: u64,
    pub x: f64,
    pub y: f64,
    pub size: f64,
    pub clipped: bool,
    pub clip: [f64; 4],
    pub pixels: *const u8,
    pub width: i32,
    pub height: i32,
    pub stride: i32,
}

/// A chunk of the overlay: see `ChartChunk` in `isomar.h`.
#[repr(C)]
pub struct ChartChunk {
    pub id: u64,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub pixels: *const u8,
    pub stride: i32,
}

/// A frame: see `ChartScene` in `isomar.h`.
#[repr(C)]
pub struct ChartScene {
    pub scale: i32,
    pub void_rgba: [f64; 4],
    pub sea_rgba: [f64; 4],
    pub sea_top: f64,
    pub sea_bottom: f64,
    pub tiles: *const ChartTile,
    pub n_tiles: i32,
    pub chunks: *const ChartChunk,
    pub n_chunks: i32,
}

type SceneFn = extern "C" fn(*mut c_void, *const ChartScene);

/// The chart for a shell that composites it on the GPU, handed to `f` with `ctx`; the pointers in it
/// are good until `f` returns.
#[unsafe(no_mangle)]
pub extern "C" fn isomar_frame(w: f64, h: f64, scale: f64, dark: bool, ctx: *mut c_void, f: SceneFn) {
    let Some(e) = engine() else { return };
    let scale = scale.max(1.0).round() as i32;
    e.frame(w, h, scale, dark, |scene| {
        let tiles: Vec<ChartTile> = scene
            .tiles
            .iter()
            .map(|p| {
                let mut pixels = std::ptr::null();
                // The surface outlives the frame: the pointer stays good while `f` runs.
                let _ = p.surface.with_data(|d| pixels = d.as_ptr());
                let clip = p.clip.unwrap_or([0.0; 4]);
                ChartTile { id: p.id, x: p.x, y: p.y, size: p.size, clipped: p.clip.is_some(), clip, pixels, width: p.surface.width(), height: p.surface.height(), stride: p.surface.stride() }
            })
            .collect();
        let rgba = |c: isomar_app::tokens::Rgba| [c.0, c.1, c.2, c.3];
        let send = |chunks: &[ChartChunk]| {
            let s = ChartScene {
                scale: scene.scale,
                void_rgba: rgba(scene.void),
                sea_rgba: rgba(scene.sea),
                sea_top: scene.sea_band.0,
                sea_bottom: scene.sea_band.1,
                tiles: tiles.as_ptr(),
                n_tiles: tiles.len() as i32,
                chunks: chunks.as_ptr(),
                n_chunks: chunks.len() as i32,
            };
            f(ctx, &s);
        };
        let sent = scene.with_overlay(|data, stride| {
            let chunks: Vec<ChartChunk> = scene
                .overlay
                .iter()
                .map(|c| ChartChunk { id: c.id, x: c.x as i32, y: c.y as i32, w: c.w as i32, h: c.h as i32, pixels: data[c.y * stride + c.x * 4..].as_ptr(), stride: stride as i32 })
                .collect();
            send(&chunks);
        });
        if sent.is_none() {
            send(&[]);
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn isomar_drag_begin(button: u32, x: f64, y: f64) {
    if let Some(e) = engine() {
        e.drag_begin(button, x, y);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isomar_drag_update(dx: f64, dy: f64) {
    if let Some(e) = engine() {
        e.drag_update(dx, dy);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isomar_drag_end(dx: f64, dy: f64) {
    if let Some(e) = engine() {
        e.drag_end(dx, dy);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isomar_scroll_zoom(dy: f64, x: f64, y: f64) {
    if let Some(e) = engine() {
        e.scroll_zoom(dy, x, y);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isomar_magnify_begin() {
    if let Some(e) = engine() {
        e.magnify_begin();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isomar_magnify(scale: f64, x: f64, y: f64) {
    if let Some(e) = engine() {
        e.magnify(scale, x, y);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isomar_pointer(x: f64, y: f64, inside: bool) {
    if let Some(e) = engine() {
        e.pointer_moved(inside.then_some((x, y)));
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isomar_click(button: u32, n_press: i32, x: f64, y: f64) {
    if let Some(e) = engine() {
        e.click(button, n_press, x, y);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isomar_key(key: i32) -> bool {
    let key = match key {
        1 => Key::ZoomIn,
        2 => Key::ZoomOut,
        3 => Key::Left,
        4 => Key::Right,
        5 => Key::Up,
        6 => Key::Down,
        7 => Key::Escape,
        8 => Key::Enter,
        9 => Key::Backspace,
        _ => Key::Other,
    };
    engine().is_some_and(|e| e.key(key))
}

#[unsafe(no_mangle)]
pub extern "C" fn isomar_zoom_by(delta: f64) {
    if let Some(e) = engine() {
        e.zoom_by(delta);
    }
}

/// A finger touched the chart (for a pan recogniser that starts only once the finger moves).
#[unsafe(no_mangle)]
pub extern "C" fn isomar_touch_down() {
    if let Some(e) = engine() {
        e.touch_down();
    }
}

/// A pan ended with the finger moving at (`vx`, `vy`) points per second: the chart glides on.
#[unsafe(no_mangle)]
pub extern "C" fn isomar_fling(vx: f64, vy: f64) {
    if let Some(e) = engine() {
        e.fling(vx, vy);
    }
}

/// A tap a gesture recogniser decided on: a click whatever the last drag did.
#[unsafe(no_mangle)]
pub extern "C" fn isomar_tap(button: u32, n: i32, x: f64, y: f64) {
    if let Some(e) = engine() {
        e.tap(button, n, x, y);
    }
}

/// The chart is on a touch screen whose smallest hit area is `hit_min` points; 0 for none.
#[unsafe(no_mangle)]
pub extern "C" fn isomar_set_touch(hit_min: f64) {
    if let Some(e) = engine() {
        e.set_touch((hit_min > 0.0).then_some(hit_min));
    }
}

/// What covers the chart's edges, in points (a bottom sheet, the safe areas).
#[unsafe(no_mangle)]
pub extern "C" fn isomar_set_insets(top: f64, right: f64, bottom: f64, left: f64) {
    if let Some(e) = engine() {
        e.set_insets(top, right, bottom, left);
    }
}

/// How many bytes of chart tiles to keep, per plane (192 MB unless told).
#[unsafe(no_mangle)]
pub extern "C" fn isomar_set_tile_cache_limit(bytes: u64) {
    if let Some(e) = engine() {
        e.set_tile_cache_limit(bytes as usize);
    }
}

fn inject(source: &str, payload: isomar_app::core::comm::source::Payload) {
    if let Some(app) = with(|m| m.app.clone()) {
        app.hub.borrow().inject(source, payload);
    }
}

/// A line of NMEA 0183 the shell received itself (a USB or Bluetooth link, the platform's GPS), from `source`.
///
/// # Safety
/// `source` and `line` must be NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn isomar_nmea(source: *const c_char, line: *const c_char) {
    let (Ok(source), Ok(line)) = (unsafe { CStr::from_ptr(source) }.to_str(), unsafe { CStr::from_ptr(line) }.to_str()) else { return };
    inject(source, isomar_app::core::comm::source::Payload::Line(line.trim_end().to_string()));
}

/// A fix from the platform's location service, as source "device": position in degrees, speed over the
/// ground in knots, course over the ground in degrees true, horizontal accuracy in metres (NaN for what it
/// does not know), and when it was taken, in milliseconds since 1970.
#[unsafe(no_mangle)]
pub extern "C" fn isomar_location(lat: f64, lon: f64, sog_kn: f64, cog: f64, accuracy_m: f64, unix_ms: i64) {
    use isomar_app::core::comm::nmea::Nmea;
    if !lat.is_finite() || !lon.is_finite() {
        return;
    }
    let known = |v: f64| v.is_finite().then_some(v);
    let time = chrono::DateTime::from_timestamp_millis(unix_ms);
    let pos = Some(isomar_app::core::geo::LatLon::new(lat, lon));
    let mut updates = vec![Nmea::Rmc { time, valid: true, pos, sog_kn: known(sog_kn).map(|v| v.max(0.0)), cog: known(cog).map(|c| c.rem_euclid(360.0)), variation: None }];
    if let Some(metres) = known(accuracy_m) {
        updates.push(Nmea::Accuracy { metres });
    }
    inject("device", isomar_app::core::comm::source::Payload::Updates(updates));
}

#[unsafe(no_mangle)]
pub extern "C" fn isomar_zoom() -> f64 {
    engine().map_or(0.0, |e| e.view_state().zoom)
}

/// # Safety
/// `mode` must be a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn isomar_set_mode(mode: *const c_char) {
    let Some(e) = engine() else { return };
    if let Ok(s) = unsafe { CStr::from_ptr(mode) }.to_str()
        && let Ok(m) = s.parse::<Mode>()
    {
        e.set_mode(m);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isomar_mode() -> *const c_char {
    out(engine().map(|e| e.mode().as_str().to_string()).as_deref().unwrap_or("auto"))
}

#[unsafe(no_mangle)]
pub extern "C" fn isomar_text(which: i32) -> *const c_char {
    let s = with(|m| {
        let t = m.texts.borrow();
        match which {
            1 => t.pointer.clone(),
            2 => t.view.clone(),
            3 => t.status.clone(),
            4 => t.subtitle.clone(),
            _ => String::new(),
        }
    })
    .unwrap_or_default();
    out(&s)
}

#[unsafe(no_mangle)]
pub extern "C" fn isomar_layers_json() -> *const c_char {
    let Some(e) = engine() else { return out("[]") };
    let list: Vec<_> = host::toggleable_layers()
        .into_iter()
        .map(|(id, label)| serde_json::json!({ "id": id, "label": label, "visible": e.layer_visible(id) }))
        .collect();
    out(&serde_json::Value::Array(list).to_string())
}

/// # Safety
/// `id` must be a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn isomar_set_layer_visible(id: *const c_char, visible: bool) {
    let Some((e, ctx, changed)) = with(|m| (m.app.canvas.clone(), m.ctx, m.changed)) else { return };
    if let Ok(id) = unsafe { CStr::from_ptr(id) }.to_str() {
        e.set_layer_visible(id, visible);
        notify(ctx, changed, TOPIC_LAYERS);
    }
}

/// Starts keeping frame times (for `ISOMAR_BENCH`).
#[unsafe(no_mangle)]
pub extern "C" fn isomar_frames_start() {
    if let Some(e) = engine() {
        let _ = e.record_frames(true);
    }
}

/// Stops keeping frame times and says how they were.
#[unsafe(no_mangle)]
pub extern "C" fn isomar_frames_report() -> *const c_char {
    let Some(e) = engine() else { return out("") };
    let frames = e.record_frames(false);
    let mut ms: Vec<f64> = frames.iter().map(|(t, _)| t.as_secs_f64() * 1000.0).collect();
    ms.sort_by(f64::total_cmp);
    let rebuilds = frames.iter().filter(|(_, r)| *r).count();
    let at = |q: f64| ms.get(((ms.len() as f64 - 1.0) * q).round() as usize).copied().unwrap_or(0.0);
    out(&format!(
        "{} frames ({} redrew the static layers), min {:.1} ms, median {:.1} ms, p90 {:.1} ms, max {:.1} ms",
        ms.len(),
        rebuilds,
        at(0.0),
        at(0.5),
        at(0.9),
        at(1.0)
    ))
}

fn json_of<T: serde::Serialize>(v: &T) -> *const c_char {
    out(&serde_json::to_string(v).unwrap_or_else(|_| "null".into()))
}

/// What a page shows, as JSON: `passage`, `traffic`, `instruments` or `banner`.
///
/// # Safety
/// `what` must be a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn isomar_json(what: *const c_char) -> *const c_char {
    let what = unsafe { CStr::from_ptr(what) }.to_str().unwrap_or("");
    let Some(app) = with(|m| m.app.clone()) else { return out("null") };
    match what {
        "passage" => json_of(&vm::passage::passage(&app)),
        "traffic" => json_of(&vm::traffic::vessels(&app, 60)),
        "instruments" => json_of(&vm::instruments::tiles(&app)),
        "alerts" => json_of(&with(|m| m.alerts.borrow().clone()).unwrap_or_default()),
        _ => out("null"),
    }
}

/// Does something the user asked for, naming it and the id of what it is about.
///
/// # Safety
/// `name` must be a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn isomar_command(name: *const c_char, arg: i64) {
    use isomar_app::actions;
    let name = unsafe { CStr::from_ptr(name) }.to_str().unwrap_or("");
    let Some(app) = with(|m| m.app.clone()) else { return };
    match name {
        "zoom_route" => vm::passage::zoom_to_route(&app, arg),
        "zoom_track" => vm::passage::zoom_to_track(&app, arg),
        "zoom_waypoint" => vm::passage::zoom_to_waypoint(&app, arg),
        "toggle_route" => vm::passage::toggle_route_visible(&app, arg),
        "toggle_track" => vm::passage::toggle_track_visible(&app, arg),
        "activate_route" => actions::activate_route(&app, arg),
        "stop_navigation" => actions::stop_navigation(&app),
        "edit_route" => actions::edit_route(&app, arg),
        "reverse_route" => actions::reverse_route(&app, arg),
        "new_route" => actions::start_route(&app, None),
        "delete_route" => confirm_then(&app, "Delete this route?", move |a| actions::delete_route(a, arg)),
        "delete_waypoint" => confirm_then(&app, "Delete this waypoint?", move |a| actions::delete_waypoint(a, arg)),
        "delete_track" => confirm_then(&app, "Delete this track?", move |a| actions::delete_track(a, arg)),
        "goto_waypoint" => {
            let w = app.session.borrow().waypoints.iter().find(|w| w.id == arg).map(|w| (w.pos, w.name.clone()));
            if let Some((p, n)) = w {
                actions::goto(&app, p, &n);
            }
        }
        "record_track" => actions::toggle_recording(&app),
        "show_vessel" => vm::traffic::show_vessel(&app, arg as u32),
        "acknowledge" => {
            app.session.borrow_mut().acknowledge_ais(arg as u32);
            app.refresh();
        }
        "alert_acknowledge" => vm::banner::press(&app, arg as u64, isomar_app::core::alerts::Action::Acknowledge),
        "alert_snooze" => vm::banner::press(&app, arg as u64, isomar_app::core::alerts::Action::Snooze),
        "alert_dismiss" => vm::banner::press(&app, arg as u64, isomar_app::core::alerts::Action::Dismiss),
        "alert_action" => vm::banner::press_action(&app, arg as u64),
        "hint_finish" => vm::banner::press_hint(&app, "finish"),
        "hint_undo" => vm::banner::press_hint(&app, "undo"),
        "hint_cancel" => vm::banner::press_hint(&app, "cancel"),
        "stop_watching" => {
            app.session.borrow_mut().clear_anchor();
            app.refresh();
        }
        "follow" => app.canvas.set_follow(arg != 0),
        "centre_boat" => {
            let p = app.session.borrow().own.pos;
            if let Some(p) = p {
                app.canvas.center_on(p);
            }
        }
        "anchor_here" => {
            let own = app.session.borrow().own.pos;
            match own {
                Some(p) => {
                    let r = app.settings.borrow().anchor_radius_nm;
                    actions::set_anchor(&app, p, r);
                }
                None => app.toast(&isomar_app::i18n::gettext("No position fix, so there is nothing to watch")),
            }
        }
        "job_cancel" => app.jobs.cancel(arg as u64),
        "info_cycle" => isomar_app::probe::cycle(&app, arg as i32),
        "info_closed" => isomar_app::probe::info_closed(&app),
        "measure_begin" => {
            // A finger has no pointer to start from: from the boat, or the middle of the chart, to where it taps.
            let own = app.session.borrow().own.pos;
            let at = app.pointer.get().or(own.filter(|_| app.canvas.is_touch())).unwrap_or_else(|| app.canvas.view_state().center());
            actions::begin_measure(&app, at);
            if app.canvas.is_touch() {
                app.toast(&isomar_app::i18n::gettext(if own.is_some() { "Tap the chart to measure from the boat" } else { "Tap the chart to measure from its middle" }));
            }
        }
        "cancel" => {
            actions::cancel_edit(&app);
            *app.measure.borrow_mut() = None;
            app.refresh();
        }
        _ => isomar_app::core::log::write(&format!("isomar: unknown command {name}")),
    }
}

fn confirm_then(app: &Rc<AppState>, heading: &'static str, then: impl FnOnce(&Rc<AppState>) + 'static) {
    let app = app.clone();
    isomar_app::platform::spawn(async move {
        let Some(shell) = app.shell() else { return };
        if shell.confirm(&isomar_app::i18n::gettext(heading), "", &isomar_app::i18n::gettext("Delete"), true).await {
            then(&app);
        }
    });
}

/// The user chose entry `index` of the menu last offered.
#[unsafe(no_mangle)]
pub extern "C" fn isomar_menu_activate(index: i32) {
    let f = MENU.with(|m| m.borrow().get(index as usize).cloned());
    if let Some(f) = f {
        f();
    }
}

/// The user answered the confirmation `id`.
#[unsafe(no_mangle)]
pub extern "C" fn isomar_confirm_result(id: u64, accepted: bool) {
    if let Some(tx) = CONFIRMS.with(|c| c.borrow_mut().remove(&id)) {
        let _ = tx.try_send(accepted);
    }
}

/// A call into `isomar_app::api`: `name` and its arguments as JSON; the answer is JSON.
///
/// # Safety
/// `name` and `json` must be NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn isomar_call(name: *const c_char, json: *const c_char) -> *const c_char {
    let name = unsafe { CStr::from_ptr(name) }.to_str().unwrap_or("");
    let args: serde_json::Value = if json.is_null() { serde_json::Value::Null } else { serde_json::from_str(unsafe { CStr::from_ptr(json) }.to_str().unwrap_or("null")).unwrap_or(serde_json::Value::Null) };
    let Some(app) = with(|m| m.app.clone()) else { return out("null") };
    out(&isomar_app::api::call(&app, name, &args).to_string())
}

/// The system's reduce-motion setting: blinking and pulsing are steady.
#[unsafe(no_mangle)]
pub extern "C" fn isomar_set_reduce_motion(on: bool) {
    if let Some(e) = engine() {
        e.set_reduce_motion(on);
    }
}

/// The tide at a position, as `isomar_app::flows::tides::TideInfo` JSON or `{"error": ...}`. It uses no
/// application state, so it may run on a worker thread (the lookup can wait on the network).
#[unsafe(no_mangle)]
pub extern "C" fn isomar_tide_info(lat: f64, lon: f64) -> *const c_char {
    use isomar_app::core::tides::{cache_dir, lookup};
    let now = chrono_now();
    match lookup(&cache_dir(), lat, lon, now, 40) {
        Ok((f, origin)) => out(&serde_json::to_string(&isomar_app::flows::tides::describe(&f, origin, now)).unwrap_or_else(|_| "null".into())),
        Err(e) => out(&serde_json::json!({ "error": isomar_app::flows::tides::explain(&e) }).to_string()),
    }
}

fn chrono_now() -> isomar_app::core::tides::DateTimeUtc {
    isomar_app::core::tides::now_utc()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    static CHANGES: AtomicUsize = AtomicUsize::new(0);

    extern "C" fn wake(_: *mut c_void, _: u64) {}
    extern "C" fn redraw(_: *mut c_void) {}
    extern "C" fn changed(_: *mut c_void, _: i32) {
        CHANGES.fetch_add(1, Ordering::Relaxed);
    }
    extern "C" fn ui(_: *mut c_void, _: i32, json: *const c_char) {
        let text = unsafe { CStr::from_ptr(json) }.to_str().expect("UTF-8");
        serde_json::from_str::<serde_json::Value>(text).expect("a request to the shell is JSON");
    }

    /// What the frames handed over, as a shell would read them.
    #[derive(Default)]
    struct Seen {
        tiles: usize,
        chunks: usize,
        bad: Vec<String>,
    }

    extern "C" fn scene(ctx: *mut c_void, s: *const ChartScene) {
        let seen = unsafe { &mut *(ctx as *mut Seen) };
        let s = unsafe { &*s };
        if s.scale < 1 || s.n_tiles < 0 || s.n_chunks < 0 {
            seen.bad.push(format!("scene: scale {}, {} tiles, {} chunks", s.scale, s.n_tiles, s.n_chunks));
            return;
        }
        // The last byte of each buffer is read: it must be inside what the pointer covers.
        let tiles = if s.n_tiles == 0 { &[][..] } else { unsafe { std::slice::from_raw_parts(s.tiles, s.n_tiles as usize) } };
        for t in tiles {
            if t.pixels.is_null() || t.width <= 0 || t.height <= 0 || t.stride < t.width * 4 || t.size <= 0.0 {
                seen.bad.push(format!("tile {}: {}x{} stride {} size {}", t.id, t.width, t.height, t.stride, t.size));
                continue;
            }
            let _last = unsafe { *t.pixels.add((t.height as usize - 1) * t.stride as usize + t.width as usize * 4 - 1) };
            seen.tiles += 1;
        }
        let chunks = if s.n_chunks == 0 { &[][..] } else { unsafe { std::slice::from_raw_parts(s.chunks, s.n_chunks as usize) } };
        for c in chunks {
            if c.pixels.is_null() || c.w <= 0 || c.h <= 0 || c.stride < c.w * 4 || c.x < 0 || c.y < 0 {
                seen.bad.push(format!("chunk {}: {}x{} at {},{} stride {}", c.id, c.w, c.h, c.x, c.y, c.stride));
                continue;
            }
            let _last = unsafe { *c.pixels.add((c.h as usize - 1) * c.stride as usize + c.w as usize * 4 - 1) };
            seen.chunks += 1;
        }
    }

    fn call(name: &CStr, json: Option<&CStr>) -> serde_json::Value {
        let p = unsafe { isomar_call(name.as_ptr(), json.map_or(std::ptr::null(), CStr::as_ptr)) };
        assert!(!p.is_null());
        serde_json::from_str(unsafe { CStr::from_ptr(p) }.to_str().unwrap()).expect("an answer is JSON")
    }

    /// The one test of this crate: the engine is one per process and lives on the thread that
    /// started it, as in a shell.
    #[test]
    fn a_shell_starts_draws_asks_and_stops() {
        let dir = tempfile::tempdir().unwrap();
        // Its own folders, no network, nothing saved over the user's.
        for (key, sub) in [("ISOMAR_STATE_DIR", "state"), ("ISOMAR_CONFIG_DIR", "config"), ("ISOMAR_CACHE_DIR", "cache"), ("ISOMAR_DATA_DIR", "data")] {
            unsafe { std::env::set_var(key, dir.path().join(sub)) };
        }
        unsafe {
            std::env::set_var("ISOMAR_OFFLINE", "1");
            std::env::set_var("ISOMAR_NO_SAVE", "1");
        }
        let none = std::ptr::null_mut();

        // Nothing is there before the start: every entry does nothing rather than crash.
        isomar_tick();
        isomar_resize(800.0, 600.0);
        assert!(call(c"passage", None).is_null());

        assert!(isomar_start(none, wake, redraw, changed, ui));
        assert!(!isomar_start(none, wake, redraw, changed, ui), "a second start is refused while the first runs");
        isomar_resize(800.0, 600.0);

        // Tiles are drawn on worker threads: frames are asked for until some arrive.
        let mut seen = Seen::default();
        let until = Instant::now() + Duration::from_secs(30);
        while (seen.tiles == 0 || seen.chunks == 0) && Instant::now() < until {
            isomar_pump();
            isomar_frame(800.0, 600.0, 2.0, false, &mut seen as *mut Seen as *mut c_void, scene);
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(seen.bad.is_empty(), "{:?}", seen.bad);
        assert!(seen.tiles > 0 && seen.chunks > 0, "{} tiles and {} overlay chunks in 30 s", seen.tiles, seen.chunks);

        assert!(call(c"passage", None)["routes"].is_array());
        assert!(call(c"passage", Some(c"not json"))["routes"].is_array(), "arguments that are not JSON are taken as none");
        assert!(call(c"nonsense", Some(c"{}"))["error"].is_string());
        assert!(unsafe { CStr::from_ptr(isomar_layers_json()) }.to_str().unwrap().starts_with('['));

        let before = CHANGES.load(Ordering::Relaxed);
        isomar_tick();
        assert!(CHANGES.load(Ordering::Relaxed) > before, "a tick tells the shell its live readouts changed");

        isomar_shutdown();
        isomar_tick();
        assert!(call(c"passage", None).is_null(), "nothing answers after the shutdown");

        // It can be started again in the same process.
        assert!(isomar_start(none, wake, redraw, changed, ui));
        assert!(call(c"passage", None)["routes"].is_array());
        isomar_shutdown();
    }
}
