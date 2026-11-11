//! The Android shell's door into the engine: JNI methods of `io.github.isomar.Native` (see
//! `android/app/src/main/java/io/github/isomar/Native.kt`) over the C interface of `isomar-ffi`, the
//! one the macOS app uses. Pages, forms and flows go through the JSON `call`, as on the Mac, so there is
//! one method per entry point of `isomar.h` and none per record.
//!
//! Everything is called on the main thread, like `isomar-ffi`, except `wake`, which any thread may call:
//! it calls `Native.wake(delay)` back, which posts `pump` to the main looper. The other callbacks run
//! inside a call from Kotlin, on the main thread.
//!
//! Frames: `frame` hands the tiles and overlay chunks to a `SceneSink`. Cairo's pixels are BGRA and
//! Android's bitmaps RGBA (both premultiplied), so pixels are copied with red and blue swapped, and only
//! for the tiles and chunks the sink does not have yet (it keeps them by id).

#![allow(clippy::missing_safety_doc)]

use isomar_ffi as ffi;
use jni::JNIEnv;
use jni::JavaVM;
use jni::objects::{GlobalRef, JClass, JObject, JString, JValue};
use jni::sys::{JNI_FALSE, JNI_TRUE, jboolean, jdouble, jint, jlong, jstring};
use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::OnceLock;

static VM: OnceLock<JavaVM> = OnceLock::new();
/// The class `io.github.isomar.Native`, held globally: a thread attached from Rust cannot find the app's
/// classes by name.
static NATIVE: OnceLock<GlobalRef> = OnceLock::new();

fn with_env(f: impl FnOnce(&mut JNIEnv, &JClass)) {
    let (Some(vm), Some(class)) = (VM.get(), NATIVE.get()) else { return };
    // The main thread is attached already; a worker thread waking the loop stays attached for good.
    let Ok(mut env) = vm.attach_current_thread_permanently() else { return };
    let class: &JClass = class.as_obj().into();
    f(&mut env, class);
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
}

extern "C" fn wake(_ctx: *mut c_void, delay_ms: u64) {
    with_env(|env, class| {
        let _ = env.call_static_method(class, "wake", "(J)V", &[JValue::Long(delay_ms as i64)]);
    });
}

extern "C" fn redraw(_ctx: *mut c_void) {
    with_env(|env, class| {
        let _ = env.call_static_method(class, "redraw", "()V", &[]);
    });
}

extern "C" fn changed(_ctx: *mut c_void, topic: i32) {
    with_env(|env, class| {
        let _ = env.call_static_method(class, "changed", "(I)V", &[JValue::Int(topic)]);
    });
}

extern "C" fn ui(_ctx: *mut c_void, kind: i32, json: *const c_char) {
    let text = unsafe { CStr::from_ptr(json) }.to_string_lossy().into_owned();
    with_env(|env, class| {
        let Ok(s) = env.new_string(text) else { return };
        let _ = env.call_static_method(class, "ui", "(ILjava/lang/String;)V", &[JValue::Int(kind), JValue::Object(&s)]);
    });
}

fn string(env: &mut JNIEnv, s: &JString) -> CString {
    let s: String = env.get_string(s).map(Into::into).unwrap_or_default();
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

fn jstr(env: &mut JNIEnv, s: *const c_char) -> jstring {
    let text = if s.is_null() { String::new() } else { unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned() };
    env.new_string(text).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

fn argb(c: [f64; 4]) -> jint {
    let b = |v: f64| ((v.clamp(0.0, 1.0) * 255.0).round() as u32) & 0xff;
    ((b(c[3]) << 24) | (b(c[0]) << 16) | (b(c[1]) << 8) | b(c[2])) as i32
}

/// `w` by `h` pixels from BGRA rows `stride` bytes apart, as packed RGBA.
unsafe fn rgba(pixels: *const u8, w: i32, h: i32, stride: i32) -> Vec<u8> {
    let (w, h, stride) = (w.max(0) as usize, h.max(0) as usize, stride.max(0) as usize);
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        let row = unsafe { std::slice::from_raw_parts(pixels.add(y * stride), w * 4) };
        let (dst, _) = out[y * w * 4..(y + 1) * w * 4].as_chunks_mut::<4>();
        for (d, s) in dst.iter_mut().zip(row.as_chunks::<4>().0) {
            *d = [s[2], s[1], s[0], s[3]];
        }
    }
    out
}

#[unsafe(no_mangle)]
pub extern "system" fn JNI_OnLoad(vm: JavaVM, _reserved: *mut c_void) -> jint {
    let _ = VM.set(vm);
    jni::sys::JNI_VERSION_1_6
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_start(env: JNIEnv, class: JClass) -> jboolean {
    if NATIVE.get().is_none()
        && let Ok(g) = env.new_global_ref(&class)
    {
        let _ = NATIVE.set(g);
    }
    if ffi::isomar_start(std::ptr::null_mut(), wake, redraw, changed, ui) { JNI_TRUE } else { JNI_FALSE }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_shutdown(_env: JNIEnv, _class: JClass) {
    ffi::isomar_shutdown();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_pump(_env: JNIEnv, _class: JClass) {
    ffi::isomar_pump();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_tick(_env: JNIEnv, _class: JClass) {
    ffi::isomar_tick();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_resize(_env: JNIEnv, _class: JClass, w: jdouble, h: jdouble) {
    ffi::isomar_resize(w, h);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_dragBegin(_env: JNIEnv, _class: JClass, button: jint, x: jdouble, y: jdouble) {
    ffi::isomar_drag_begin(button as u32, x, y);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_dragUpdate(_env: JNIEnv, _class: JClass, dx: jdouble, dy: jdouble) {
    ffi::isomar_drag_update(dx, dy);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_dragEnd(_env: JNIEnv, _class: JClass, dx: jdouble, dy: jdouble) {
    ffi::isomar_drag_end(dx, dy);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_magnifyBegin(_env: JNIEnv, _class: JClass) {
    ffi::isomar_magnify_begin();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_magnify(_env: JNIEnv, _class: JClass, scale: jdouble, x: jdouble, y: jdouble) {
    ffi::isomar_magnify(scale, x, y);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_click(_env: JNIEnv, _class: JClass, button: jint, n: jint, x: jdouble, y: jdouble) {
    ffi::isomar_click(button as u32, n, x, y);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_tap(_env: JNIEnv, _class: JClass, button: jint, n: jint, x: jdouble, y: jdouble) {
    ffi::isomar_tap(button as u32, n, x, y);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_setTouch(_env: JNIEnv, _class: JClass, hit_min: jdouble) {
    ffi::isomar_set_touch(hit_min);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_setInsets(_env: JNIEnv, _class: JClass, top: jdouble, right: jdouble, bottom: jdouble, left: jdouble) {
    ffi::isomar_set_insets(top, right, bottom, left);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_setTileCacheLimit(_env: JNIEnv, _class: JClass, bytes: jlong) {
    ffi::isomar_set_tile_cache_limit(bytes.max(0) as u64);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_nmea(mut env: JNIEnv, _class: JClass, source: JString, line: JString) {
    let (source, line) = (string(&mut env, &source), string(&mut env, &line));
    unsafe { ffi::isomar_nmea(source.as_ptr(), line.as_ptr()) };
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_location(_env: JNIEnv, _class: JClass, lat: jdouble, lon: jdouble, sog_kn: jdouble, cog: jdouble, accuracy_m: jdouble, unix_ms: jlong) {
    ffi::isomar_location(lat, lon, sog_kn, cog, accuracy_m, unix_ms);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_fling(_env: JNIEnv, _class: JClass, vx: jdouble, vy: jdouble) {
    ffi::isomar_fling(vx, vy);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_zoomBy(_env: JNIEnv, _class: JClass, delta: jdouble) {
    ffi::isomar_zoom_by(delta);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_setMode(mut env: JNIEnv, _class: JClass, mode: JString) {
    let mode = string(&mut env, &mode);
    unsafe { ffi::isomar_set_mode(mode.as_ptr()) };
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_mode(mut env: JNIEnv, _class: JClass) -> jstring {
    let s = ffi::isomar_mode();
    jstr(&mut env, s)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_layers(mut env: JNIEnv, _class: JClass) -> jstring {
    let s = ffi::isomar_layers_json();
    jstr(&mut env, s)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_setLayerVisible(mut env: JNIEnv, _class: JClass, id: JString, visible: jboolean) {
    let id = string(&mut env, &id);
    unsafe { ffi::isomar_set_layer_visible(id.as_ptr(), visible != JNI_FALSE) };
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_setReduceMotion(_env: JNIEnv, _class: JClass, on: jboolean) {
    ffi::isomar_set_reduce_motion(on != JNI_FALSE);
}

/// The tide at a position. It touches no application state, so it may be called from a worker thread
/// (the lookup can wait on the network).
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_tideInfo(mut env: JNIEnv, _class: JClass, lat: jdouble, lon: jdouble) -> jstring {
    let s = ffi::isomar_tide_info(lat, lon);
    jstr(&mut env, s)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_json(mut env: JNIEnv, _class: JClass, what: JString) -> jstring {
    let what = string(&mut env, &what);
    let s = unsafe { ffi::isomar_json(what.as_ptr()) };
    jstr(&mut env, s)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_call(mut env: JNIEnv, _class: JClass, name: JString, json: JString) -> jstring {
    let (name, json) = (string(&mut env, &name), string(&mut env, &json));
    let s = unsafe { ffi::isomar_call(name.as_ptr(), json.as_ptr()) };
    jstr(&mut env, s)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_text(mut env: JNIEnv, _class: JClass, which: jint) -> jstring {
    let s = ffi::isomar_text(which);
    jstr(&mut env, s)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_command(mut env: JNIEnv, _class: JClass, name: JString, arg: jlong) {
    let name = string(&mut env, &name);
    unsafe { ffi::isomar_command(name.as_ptr(), arg) };
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_menuActivate(_env: JNIEnv, _class: JClass, index: jint) {
    ffi::isomar_menu_activate(index);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_confirmResult(_env: JNIEnv, _class: JClass, id: jlong, accepted: jboolean) {
    ffi::isomar_confirm_result(id as u64, accepted != JNI_FALSE);
}

/// What `frame` passes through `isomar_frame` to its callback.
struct Frame<'a, 'e> {
    env: &'a mut JNIEnv<'e>,
    sink: &'a JObject<'e>,
}

extern "C" fn scene(ctx: *mut c_void, s: *const ffi::ChartScene) {
    let f = unsafe { &mut *(ctx as *mut Frame) };
    let s = unsafe { &*s };
    let env = &mut *f.env;
    let sink = f.sink;
    let _ = env.call_method(
        sink,
        "begin",
        "(IIIDD)V",
        &[JValue::Int(s.scale), JValue::Int(argb(s.void_rgba)), JValue::Int(argb(s.sea_rgba)), JValue::Double(s.sea_top), JValue::Double(s.sea_bottom)],
    );
    let tiles = if s.n_tiles > 0 { unsafe { std::slice::from_raw_parts(s.tiles, s.n_tiles as usize) } } else { &[] };
    for t in tiles {
        let has = env.call_method(sink, "hasTile", "(J)Z", &[JValue::Long(t.id as i64)]).and_then(|v| v.z()).unwrap_or(false);
        let mut pixels = if has || t.pixels.is_null() { None } else { Some(unsafe { rgba(t.pixels, t.width, t.height, t.stride) }) };
        let buffer = match pixels.as_mut() {
            Some(p) => unsafe { env.new_direct_byte_buffer(p.as_mut_ptr(), p.len()) }.map(JObject::from).unwrap_or_default(),
            None => JObject::null(),
        };
        let _ = env.call_method(
            sink,
            "tile",
            "(JDDDZDDDDIILjava/nio/ByteBuffer;)V",
            &[
                JValue::Long(t.id as i64),
                JValue::Double(t.x),
                JValue::Double(t.y),
                JValue::Double(t.size),
                JValue::Bool(t.clipped as u8),
                JValue::Double(t.clip[0]),
                JValue::Double(t.clip[1]),
                JValue::Double(t.clip[2]),
                JValue::Double(t.clip[3]),
                JValue::Int(t.width),
                JValue::Int(t.height),
                JValue::Object(&buffer),
            ],
        );
        let _ = env.delete_local_ref(buffer);
    }
    let chunks = if s.n_chunks > 0 { unsafe { std::slice::from_raw_parts(s.chunks, s.n_chunks as usize) } } else { &[] };
    for c in chunks {
        let has = env.call_method(sink, "hasChunk", "(J)Z", &[JValue::Long(c.id as i64)]).and_then(|v| v.z()).unwrap_or(false);
        let mut pixels = if has || c.pixels.is_null() { None } else { Some(unsafe { rgba(c.pixels, c.w, c.h, c.stride) }) };
        let buffer = match pixels.as_mut() {
            Some(p) => unsafe { env.new_direct_byte_buffer(p.as_mut_ptr(), p.len()) }.map(JObject::from).unwrap_or_default(),
            None => JObject::null(),
        };
        let _ = env.call_method(
            sink,
            "chunk",
            "(JIIIILjava/nio/ByteBuffer;)V",
            &[JValue::Long(c.id as i64), JValue::Int(c.x), JValue::Int(c.y), JValue::Int(c.w), JValue::Int(c.h), JValue::Object(&buffer)],
        );
        let _ = env.delete_local_ref(buffer);
    }
    let _ = env.call_method(sink, "end", "()V", &[]);
}

/// Draws a frame of `w` by `h` points at `scale` device pixels a point into `sink`, a `SceneSink`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isomar_Native_frame<'e>(mut env: JNIEnv<'e>, _class: JClass<'e>, w: jdouble, h: jdouble, scale: jdouble, dark: jboolean, sink: JObject<'e>) {
    let mut f = Frame { env: &mut env, sink: &sink };
    ffi::isomar_frame(w, h, scale, dark != JNI_FALSE, &mut f as *mut Frame as *mut c_void, scene);
}
