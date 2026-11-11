package io.github.isomar

import android.os.Handler
import android.os.Looper
import java.nio.ByteBuffer

/**
 * The engine (crates/isomar-android over crates/isomar-ffi, include/isomar.h). Everything is called on the
 * main thread, except [wake], which the engine calls from any thread to have [pump] run on the main looper.
 */
object Native {
    init {
        System.loadLibrary("isomar_android")
    }

    private val main = Handler(Looper.getMainLooper())

    /** Called back on the main thread: repaint the chart. */
    var onRedraw: () -> Unit = {}

    /** Called back on the main thread: a readout changed (ISOMAR_TOPIC_*). */
    var onChanged: (Int) -> Unit = {}

    /** Called back on the main thread: the engine asks for something (ISOMAR_UI_*, with its JSON). */
    var onUi: (Int, String) -> Unit = { _, _ -> }

    @JvmStatic external fun start(): Boolean
    @JvmStatic external fun shutdown()
    @JvmStatic external fun pump()
    @JvmStatic external fun tick()
    @JvmStatic external fun resize(w: Double, h: Double)
    @JvmStatic external fun frame(w: Double, h: Double, scale: Double, dark: Boolean, sink: SceneSink)
    @JvmStatic external fun dragBegin(button: Int, x: Double, y: Double)
    @JvmStatic external fun dragUpdate(dx: Double, dy: Double)
    @JvmStatic external fun dragEnd(dx: Double, dy: Double)
    @JvmStatic external fun magnifyBegin()
    @JvmStatic external fun magnify(scale: Double, x: Double, y: Double)
    @JvmStatic external fun click(button: Int, n: Int, x: Double, y: Double)
    @JvmStatic external fun tap(button: Int, n: Int, x: Double, y: Double)
    @JvmStatic external fun setTouch(hitMin: Double)
    @JvmStatic external fun setInsets(top: Double, right: Double, bottom: Double, left: Double)
    @JvmStatic external fun setTileCacheLimit(bytes: Long)
    @JvmStatic external fun nmea(source: String, line: String)
    /** A fix from the location service; NaN for what it does not know. */
    @JvmStatic external fun location(lat: Double, lon: Double, sogKn: Double, cog: Double, accuracyM: Double, unixMs: Long)
    @JvmStatic external fun zoomBy(delta: Double)
    /** A pan ended with the finger moving at (vx, vy) points per second. */
    @JvmStatic external fun fling(vx: Double, vy: Double)
    @JvmStatic external fun setMode(mode: String)
    @JvmStatic external fun mode(): String
    @JvmStatic external fun layers(): String
    @JvmStatic external fun setLayerVisible(id: String, visible: Boolean)
    @JvmStatic external fun setReduceMotion(on: Boolean)
    /** The tide at a position (JSON); may be called off the main thread, as it can wait on the network. */
    @JvmStatic external fun tideInfo(lat: Double, lon: Double): String
    @JvmStatic external fun json(what: String): String
    @JvmStatic external fun call(name: String, json: String): String
    @JvmStatic external fun text(which: Int): String
    @JvmStatic external fun command(name: String, arg: Long)
    @JvmStatic external fun menuActivate(index: Int)
    @JvmStatic external fun confirmResult(id: Long, accepted: Boolean)

    // Called from Rust.

    @JvmStatic
    fun wake(delayMs: Long) {
        main.postDelayed({ pump() }, delayMs)
    }

    @JvmStatic
    fun redraw() = onRedraw()

    @JvmStatic
    fun changed(topic: Int) = onChanged(topic)

    @JvmStatic
    fun ui(kind: Int, json: String) = onUi(kind, json)

    const val TOPIC_POINTER = 1
    const val TOPIC_VIEW = 2
    const val TOPIC_STATUS = 3
    const val TOPIC_SESSION = 4
    const val TOPIC_LAYERS = 5
    const val TOPIC_DATA = 6
    const val TOPIC_LIVE = 7

    const val TEXT_POINTER = 1
    const val TEXT_VIEW = 2
    const val TEXT_STATUS = 3
    const val TEXT_SUBTITLE = 4

    const val UI_TOAST = 1
    const val UI_ERROR = 2
    const val UI_BEEP = 3
    const val UI_CONFIRM = 4
    const val UI_MENU = 5
    const val UI_INFO = 6
    const val UI_CLIPBOARD = 7
    const val UI_DIALOG = 8
    const val UI_CURSOR = 9
    const val UI_NOTIFY = 10
}

/**
 * Receives a frame from [Native.frame]: the background, then every tile and overlay chunk, back to front.
 * Pixels (RGBA premultiplied, packed) come only for ids the sink says it does not have, and the buffer is
 * good only during the call.
 */
interface SceneSink {
    fun begin(scale: Int, voidArgb: Int, seaArgb: Int, seaTop: Double, seaBottom: Double)
    fun hasTile(id: Long): Boolean
    fun tile(
        id: Long, x: Double, y: Double, size: Double,
        clipped: Boolean, cx: Double, cy: Double, cw: Double, ch: Double,
        width: Int, height: Int, pixels: ByteBuffer?,
    )
    fun hasChunk(id: Long): Boolean
    fun chunk(id: Long, x: Int, y: Int, w: Int, h: Int, pixels: ByteBuffer?)
    fun end()
}
