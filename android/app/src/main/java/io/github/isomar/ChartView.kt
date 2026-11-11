package io.github.isomar

import android.content.Context
import android.content.res.Configuration
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.RectF
import android.view.GestureDetector
import android.view.MotionEvent
import android.view.ScaleGestureDetector
import android.view.View
import java.nio.ByteBuffer
import kotlin.math.abs
import kotlin.math.ceil

/**
 * The chart: draws the engine's frame (tiles kept as bitmaps by id, so a pan uploads nothing new) and feeds
 * it touches. Positions given to the engine are in points (dp); the engine draws at the next whole scale above
 * the screen's density (2.625 is drawn at 3) and the bitmaps are scaled down to the screen.
 */
class ChartView(context: Context) : View(context), SceneSink {
    private val density get() = resources.displayMetrics.density
    private val engineScale get() = ceil(density.toDouble()).toInt().coerceAtLeast(1)

    private val tiles = HashMap<Long, Bitmap>()
    private val chunks = HashMap<Long, Bitmap>()
    private val usedTiles = HashSet<Long>()
    private val usedChunks = HashSet<Long>()

    private class Placed(val id: Long, val dst: RectF, val clip: RectF?)
    private val tileList = ArrayList<Placed>()
    private val chunkList = ArrayList<Placed>()
    private var voidColour = 0
    private var seaColour = 0
    private var seaTop = 0.0
    private var seaBottom = 0.0
    private val paint = Paint(Paint.FILTER_BITMAP_FLAG)
    private val fill = Paint()

    // Touch state: one finger pans, two pinch; a pinch ends the pan, and a finger left after it starts a new one.
    private var downX = 0f
    private var downY = 0f
    private var panning = false
    private var pinching = false
    private var pinch = 1.0
    private var velocity: android.view.VelocityTracker? = null

    private val taps = GestureDetector(context, object : GestureDetector.SimpleOnGestureListener() {
        override fun onSingleTapConfirmed(e: MotionEvent): Boolean {
            Native.tap(1, 1, pt(e.x), pt(e.y))
            return true
        }

        override fun onDoubleTap(e: MotionEvent): Boolean {
            Native.tap(1, 2, pt(e.x), pt(e.y))
            return true
        }

        override fun onLongPress(e: MotionEvent) {
            if (!pinching) Native.tap(3, 1, pt(e.x), pt(e.y))
        }
    })

    private val scales = ScaleGestureDetector(context, object : ScaleGestureDetector.SimpleOnScaleGestureListener() {
        override fun onScaleBegin(d: ScaleGestureDetector): Boolean {
            endPan(d.focusX, d.focusY)
            pinching = true
            pinch = 1.0
            Native.magnifyBegin()
            return true
        }

        override fun onScale(d: ScaleGestureDetector): Boolean {
            pinch *= d.scaleFactor.toDouble()
            Native.magnify(pinch, pt(d.focusX), pt(d.focusY))
            return true
        }
    })

    private fun pt(px: Float) = px / density.toDouble()

    private fun endPan(x: Float, y: Float) {
        if (panning) {
            Native.dragEnd(pt(x - downX), pt(y - downY))
            panning = false
        }
    }

    override fun onTouchEvent(e: MotionEvent): Boolean {
        scales.onTouchEvent(e)
        taps.onTouchEvent(e)
        when (e.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                velocity?.recycle()
                velocity = android.view.VelocityTracker.obtain().also { it.addMovement(e) }
                downX = e.x
                downY = e.y
                panning = true
                Native.dragBegin(1, pt(e.x), pt(e.y))
            }
            MotionEvent.ACTION_MOVE -> if (panning && !pinching && e.pointerCount == 1) {
                velocity?.addMovement(e)
                Native.dragUpdate(pt(e.x - downX), pt(e.y - downY))
            }
            MotionEvent.ACTION_POINTER_UP -> if (pinching && e.pointerCount == 2) {
                // The finger that stays starts a new pan.
                pinching = false
                velocity?.clear()
                val keep = if (e.actionIndex == 0) 1 else 0
                downX = e.getX(keep)
                downY = e.getY(keep)
                panning = true
                Native.dragBegin(1, pt(downX), pt(downY))
            }
            MotionEvent.ACTION_UP -> {
                if (panning) {
                    val dx = e.x - downX
                    val dy = e.y - downY
                    Native.dragEnd(pt(dx), pt(dy))
                    panning = false
                    velocity?.let { v ->
                        v.addMovement(e)
                        v.computeCurrentVelocity(1000)
                        Native.fling(pt(v.xVelocity), pt(v.yVelocity))
                    }
                    if (abs(dx) > 0 || abs(dy) > 0) performClick()
                }
                pinching = false
            }
            MotionEvent.ACTION_CANCEL -> {
                endPan(e.x, e.y)
                pinching = false
            }
        }
        return true
    }

    override fun performClick(): Boolean = super.performClick()

    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
        Native.resize(pt(w.toFloat()), pt(h.toFloat()))
    }

    /** What covers the chart's edges (system bars, a sheet), in pixels: the engine centres in the rest. */
    fun setInsets(top: Int, right: Int, bottom: Int, left: Int) {
        Native.setInsets(pt(top.toFloat()), pt(right.toFloat()), pt(bottom.toFloat()), pt(left.toFloat()))
    }

    private val dark: Boolean
        get() = (resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK) == Configuration.UI_MODE_NIGHT_YES

    override fun onDraw(canvas: Canvas) {
        tileList.clear()
        chunkList.clear()
        usedTiles.clear()
        usedChunks.clear()
        Native.frame(pt(width.toFloat()), pt(height.toFloat()), engineScale.toDouble(), dark, this)
        tiles.keys.retainAll(usedTiles)
        chunks.keys.retainAll(usedChunks)

        // Compose does not clip the views it hosts: drawColor would paint the whole window.
        canvas.clipRect(0, 0, width, height)
        canvas.drawColor(voidColour)
        fill.color = seaColour
        canvas.drawRect(0f, (seaTop * density).toFloat(), width.toFloat(), (seaBottom * density).toFloat(), fill)
        for (p in tileList) {
            val bmp = tiles[p.id] ?: continue
            if (p.clip != null) {
                canvas.save()
                canvas.clipRect(p.clip)
            }
            canvas.drawBitmap(bmp, null, p.dst, paint)
            if (p.clip != null) canvas.restore()
        }
        for (p in chunkList) {
            val bmp = chunks[p.id] ?: continue
            canvas.drawBitmap(bmp, null, p.dst, paint)
        }
    }

    // SceneSink, called inside Native.frame.

    override fun begin(scale: Int, voidArgb: Int, seaArgb: Int, seaTop: Double, seaBottom: Double) {
        voidColour = voidArgb
        seaColour = seaArgb
        this.seaTop = seaTop
        this.seaBottom = seaBottom
    }

    override fun hasTile(id: Long) = tiles.containsKey(id)

    override fun tile(
        id: Long, x: Double, y: Double, size: Double,
        clipped: Boolean, cx: Double, cy: Double, cw: Double, ch: Double,
        width: Int, height: Int, pixels: ByteBuffer?,
    ) {
        if (pixels != null) tiles[id] = bitmap(width, height, pixels)
        usedTiles += id
        val d = density
        val dst = RectF((x * d).toFloat(), (y * d).toFloat(), ((x + size) * d).toFloat(), ((y + size) * d).toFloat())
        val clip = if (clipped) RectF((cx * d).toFloat(), (cy * d).toFloat(), ((cx + cw) * d).toFloat(), ((cy + ch) * d).toFloat()) else null
        tileList += Placed(id, dst, clip)
    }

    override fun hasChunk(id: Long) = chunks.containsKey(id)

    override fun chunk(id: Long, x: Int, y: Int, w: Int, h: Int, pixels: ByteBuffer?) {
        if (pixels != null) chunks[id] = bitmap(w, h, pixels)
        usedChunks += id
        // Chunks are in the engine's device pixels.
        val f = density / engineScale
        chunkList += Placed(id, RectF(x * f, y * f, (x + w) * f, (y + h) * f), null)
    }

    override fun end() {}

    private fun bitmap(w: Int, h: Int, pixels: ByteBuffer): Bitmap {
        val bmp = Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888)
        bmp.copyPixelsFromBuffer(pixels)
        return bmp
    }
}
