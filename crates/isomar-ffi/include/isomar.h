// The C interface of isomar-ffi. Everything is for the main thread, except
// the wake callback, which may be called from any thread. Strings returned
// belong to the library and last until the next call that returns a string.

#ifndef ISOMAR_H
#define ISOMAR_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// Asks the shell to call isomar_pump() on the main thread after delay_ms.
typedef void (*isomar_wake_fn)(void *ctx, uint64_t delay_ms);
// Asks the shell to repaint the chart.
typedef void (*isomar_redraw_fn)(void *ctx);
// Something a readout shows changed; topic is one of the ISOMAR_TOPIC_* values.
typedef void (*isomar_changed_fn)(void *ctx, int32_t topic);
// The engine asks the shell for something; json describes it (see ISOMAR_UI_*).
typedef void (*isomar_ui_fn)(void *ctx, int32_t kind, const char *json);

enum {
    ISOMAR_TOPIC_POINTER = 1,  // isomar_text(ISOMAR_TEXT_POINTER)
    ISOMAR_TOPIC_VIEW = 2,     // isomar_text(ISOMAR_TEXT_VIEW)
    ISOMAR_TOPIC_STATUS = 3,   // isomar_text(ISOMAR_TEXT_STATUS)
    ISOMAR_TOPIC_SESSION = 4,  // isomar_text(ISOMAR_TEXT_SUBTITLE), the boat, alerts
    ISOMAR_TOPIC_LAYERS = 5,   // isomar_layers_json()
    ISOMAR_TOPIC_DATA = 6,     // routes, waypoints, tracks may have changed: isomar_json("passage")
    ISOMAR_TOPIC_LIVE = 7,     // the 4 Hz clock: instruments, traffic, banner
};

enum {
    ISOMAR_UI_TOAST = 1,    // {"text"}
    ISOMAR_UI_ERROR = 2,    // {"heading","details"}
    ISOMAR_UI_BEEP = 3,     // {}
    ISOMAR_UI_CONFIRM = 4,  // {"id","heading","body","ok","destructive"}; answer with isomar_confirm_result
    ISOMAR_UI_MENU = 5,     // {"x","y","title","sections":[[{"i","label"}]]}; chosen with isomar_menu_activate
    ISOMAR_UI_INFO = 6,     // {"x","y","title","subtitle","rows":[[name,value]],"choice":[n,of]|null}; isomar_command("info_cycle", ±1) steps through the "of" features under the click, "info_closed" says the popover is gone
    ISOMAR_UI_CLIPBOARD = 7, // {"text"}: put it on the clipboard
    ISOMAR_UI_DIALOG = 8,    // {"name", ...}: open this dialog or sheet (see Dialog in isomar-app)
    ISOMAR_UI_CURSOR = 9,    // {"pointing"}: the pointer is over something a click opens (a hand), or no longer
    ISOMAR_UI_NOTIFY = 10,   // {"title","body"}: a long calculation ended; a system notification if the app is out of sight
};

enum {
    ISOMAR_TEXT_POINTER = 1,
    ISOMAR_TEXT_VIEW = 2,
    ISOMAR_TEXT_STATUS = 3,
    ISOMAR_TEXT_SUBTITLE = 4,
};

enum {
    ISOMAR_KEY_OTHER = 0,
    ISOMAR_KEY_ZOOM_IN = 1,
    ISOMAR_KEY_ZOOM_OUT = 2,
    ISOMAR_KEY_LEFT = 3,
    ISOMAR_KEY_RIGHT = 4,
    ISOMAR_KEY_UP = 5,
    ISOMAR_KEY_DOWN = 6,
    ISOMAR_KEY_ESCAPE = 7,
    ISOMAR_KEY_ENTER = 8,
    ISOMAR_KEY_BACKSPACE = 9,
};

// Starts the application: settings, data, layers, connections. Returns false
// if it was already started.
bool isomar_start(void *ctx, isomar_wake_fn wake, isomar_redraw_fn redraw, isomar_changed_fn changed, isomar_ui_fn ui);
// Saves the settings and stops the connections.
void isomar_shutdown(void);

// Runs the futures and timeouts that are ready.
void isomar_pump(void);
// The 4 Hz clock.
void isomar_tick(void);

// The chart's size in points.
void isomar_resize(double w, double h);
// Paints into a BGRA premultiplied buffer of w_px x h_px device pixels.
void isomar_render(uint8_t *data, int32_t w_px, int32_t h_px, int32_t stride, double scale, bool dark);

// A tile of the chart: its top-left corner and size in points from the view's top-left corner, the
// rectangle (x, y, w, h, in points) it is cut to when it stands in for missing tiles of another zoom,
// and its pixels (BGRA premultiplied, width x height, stride bytes to a row). `id` changes whenever the
// tile is drawn again, so its image can be kept by id. Tiles come back to front.
typedef struct {
    uint64_t id;
    double x, y, size;
    bool clipped;
    double clip[4];
    const uint8_t *pixels;
    int32_t width, height, stride;
} ChartTile;

// A chunk of the layers drawn every frame, with something on it: where it is and its size in device
// pixels from the view's top-left corner, and its pixels (BGRA premultiplied, stride bytes to a row).
// The pixels are a window into the whole overlay and `stride` is the overlay's: copy `w * 4` bytes of
// each row, never `stride * h` bytes, which runs past the overlay's end for a chunk at the bottom.
// `id` stays the same while its pixels do. Chunks go over the tiles.
typedef struct {
    uint64_t id;
    int32_t x, y, w, h;
    const uint8_t *pixels;
    int32_t stride;
} ChartChunk;

// A frame: the colour beyond the poles, the sea's from sea_top to sea_bottom points down the view,
// then the tiles and the chunks. `scale` is device pixels to a point.
typedef struct {
    int32_t scale;
    double void_rgba[4];
    double sea_rgba[4];
    double sea_top, sea_bottom;
    const ChartTile *tiles;
    int32_t n_tiles;
    const ChartChunk *chunks;
    int32_t n_chunks;
} ChartScene;

typedef void (*isomar_scene_fn)(void *ctx, const ChartScene *scene);
// The chart for a shell that composites it on the GPU: calls `f` with the frame, whose pointers are
// good until `f` returns.
void isomar_frame(double w, double h, double scale, bool dark, void *ctx, isomar_scene_fn f);

void isomar_drag_begin(uint32_t button, double x, double y);
void isomar_drag_update(double dx, double dy);
void isomar_drag_end(double dx, double dy);
void isomar_scroll_zoom(double dy, double x, double y);
void isomar_magnify_begin(void);
void isomar_magnify(double scale, double x, double y);
// inside = false when the pointer left the chart.
void isomar_pointer(double x, double y, bool inside);
void isomar_click(uint32_t button, int32_t n_press, double x, double y);
// Returns whether the key was used.
bool isomar_key(int32_t key);

void isomar_zoom_by(double delta);

// Touch screens. A tap a gesture recogniser decided on (a click whatever the last drag did); the smallest
// hit area in points (44 on iOS, 48 on Android; 0 for none); what covers the chart's edges in points (a
// bottom sheet, the safe areas), which centring and the zoom buttons keep clear of.
void isomar_tap(uint32_t button, int32_t n, double x, double y);
// A finger touched the chart: for a pan recogniser that starts only once the finger has moved, so that
// a finger held on a mark first drags the mark.
void isomar_touch_down(void);
// A pan ended with the finger moving at (vx, vy) points per second: the chart glides on and slows.
void isomar_fling(double vx, double vy);
void isomar_set_touch(double hit_min);
void isomar_set_insets(double top, double right, double bottom, double left);
// How many bytes of chart tiles to keep, per plane (192 MB unless told): less on a phone.
void isomar_set_tile_cache_limit(uint64_t bytes);

// What the shell receives itself: a line of NMEA 0183 from `source` (a USB or Bluetooth link), and a fix
// from the platform's location service (source "device"; NaN for what it does not know).
void isomar_nmea(const char *source, const char *line);
void isomar_location(double lat, double lon, double sog_kn, double cog, double accuracy_m, int64_t unix_ms);
double isomar_zoom(void);
// 0 auto, 1 day, 2 dusk, 3 night; see isomar_mode.
void isomar_set_mode(const char *mode);
const char *isomar_mode(void);

const char *isomar_text(int32_t which);
// [{"id":"depth","label":"Depth shading","visible":true}, ...]
const char *isomar_layers_json(void);
void isomar_set_layer_visible(const char *id, bool visible);

// What a page shows, as JSON: "passage", "traffic", "instruments" or "banner" (null when none).
const char *isomar_json(const char *what);
// A call into the application's JSON api (crates/isomar-app/src/api.rs): name and arguments as
// JSON, the answer as JSON. This is how pages, forms and flows are reached.
const char *isomar_call(const char *name, const char *json);
void isomar_set_reduce_motion(bool on);
// The tide at a position as JSON; touches no state, so it may be called off the main thread.
const char *isomar_tide_info(double lat, double lon);

// Something the user asked for, about the object with id arg; see the list in lib.rs.
void isomar_command(const char *name, int64_t arg);
void isomar_menu_activate(int32_t index);
void isomar_confirm_result(uint64_t id, bool accepted);

// Frame times of the engine's drawing, for benchmarks.
void isomar_frames_start(void);
const char *isomar_frames_report(void);

#ifdef __cplusplus
}
#endif

#endif
