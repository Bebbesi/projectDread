package app.dread.client.input

import android.graphics.RectF
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.view.MotionEvent
import app.dread.client.StylusMode
import app.dread.client.TouchMode
import app.dread.client.net.Protocol
import kotlin.math.abs
import kotlin.math.atan
import kotlin.math.cos
import kotlin.math.hypot
import kotlin.math.sin
import kotlin.math.tan

/**
 * Converts Android pointer events into DREAD input events.
 *
 * * Fingers → real multitouch (Touch mode), an absolute mouse (Mouse mode) or
 *   a relative trackpad (Trackpad mode).
 * * Stylus / eraser → pen events with pressure, tilt, hover and barrel
 *   buttons, using the historical samples of every MotionEvent so strokes keep
 *   the digitizer's full sample rate.
 * * Hardware mouse → absolute pointer, buttons and wheel.
 *
 * Coordinates are normalised to [rect], the on-screen rectangle of the video.
 */
class InputMapper(private val send: (ByteArray) -> Unit) {
    var rect = RectF(0f, 0f, 1f, 1f)
    var touchMode = TouchMode.TOUCH
    var stylusMode = StylusMode.PEN_PALM
    var sensitivity = 1.0f
    /** Display density, to make trackpad motion independent of screen size. */
    var density = 2.5f

    private val main = Handler(Looper.getMainLooper())
    private var penActiveUntil = 0L
    private var penInRange = false
    private var fingersCancelled = false
    private val penOut = Runnable { sendPenOut() }
    private var lastPen: PenState? = null

    private class PenState(val eraser: Boolean, val x: Float, val y: Float)

    private fun nx(x: Float) = ((x - rect.left) / rect.width()).coerceIn(0f, 1f)
    private fun ny(y: Float) = ((y - rect.top) / rect.height()).coerceIn(0f, 1f)

    private fun isStylus(ev: MotionEvent, i: Int): Boolean {
        val t = ev.getToolType(i)
        return t == MotionEvent.TOOL_TYPE_STYLUS || t == MotionEvent.TOOL_TYPE_ERASER
    }

    fun onTouchEvent(ev: MotionEvent): Boolean {
        if (ev.getToolType(0) == MotionEvent.TOOL_TYPE_MOUSE) return mouseEvent(ev)
        var hasStylus = false
        for (i in 0 until ev.pointerCount) if (isStylus(ev, i)) hasStylus = true
        if (hasStylus) stylusEvent(ev)
        val fingers = (0 until ev.pointerCount).filter { !isStylus(ev, it) }
        if (fingers.isNotEmpty() || ev.actionMasked == MotionEvent.ACTION_CANCEL) fingerEvent(ev, fingers)
        return true
    }

    fun onGenericMotionEvent(ev: MotionEvent): Boolean {
        val tool = ev.getToolType(0)
        if (tool == MotionEvent.TOOL_TYPE_STYLUS || tool == MotionEvent.TOOL_TYPE_ERASER) {
            when (ev.actionMasked) {
                MotionEvent.ACTION_HOVER_ENTER, MotionEvent.ACTION_HOVER_MOVE -> {
                    main.removeCallbacks(penOut)
                    for (h in 0 until ev.historySize) pen(ev, 0, h, contact = false)
                    pen(ev, 0, -1, contact = false)
                    return true
                }
                MotionEvent.ACTION_HOVER_EXIT -> {
                    main.postDelayed(penOut, 60)
                    return true
                }
            }
        }
        if (tool == MotionEvent.TOOL_TYPE_MOUSE || ev.isFromSource(android.view.InputDevice.SOURCE_MOUSE)) {
            when (ev.actionMasked) {
                MotionEvent.ACTION_HOVER_MOVE, MotionEvent.ACTION_HOVER_ENTER -> {
                    send(Protocol.mouseAbs(nx(ev.x), ny(ev.y), mouseButtons(ev.buttonState)))
                    return true
                }
                MotionEvent.ACTION_SCROLL -> {
                    val v = -ev.getAxisValue(MotionEvent.AXIS_VSCROLL) * 120f
                    val h = ev.getAxisValue(MotionEvent.AXIS_HSCROLL) * 120f
                    send(Protocol.scroll(v.toInt(), h.toInt()))
                    return true
                }
                MotionEvent.ACTION_BUTTON_PRESS, MotionEvent.ACTION_BUTTON_RELEASE -> {
                    send(Protocol.mouseAbs(nx(ev.x), ny(ev.y), mouseButtons(ev.buttonState)))
                    return true
                }
            }
        }
        return false
    }

    private fun mouseButtons(state: Int): Int {
        var b = 0
        if (state and MotionEvent.BUTTON_PRIMARY != 0) b = b or Protocol.MOUSE_LEFT
        if (state and MotionEvent.BUTTON_SECONDARY != 0) b = b or Protocol.MOUSE_RIGHT
        if (state and MotionEvent.BUTTON_TERTIARY != 0) b = b or Protocol.MOUSE_MIDDLE
        return b
    }

    private fun mouseEvent(ev: MotionEvent): Boolean {
        val b = if (ev.actionMasked == MotionEvent.ACTION_UP || ev.actionMasked == MotionEvent.ACTION_CANCEL) 0 else maxOf(mouseButtons(ev.buttonState), Protocol.MOUSE_LEFT.takeIf { ev.buttonState == 0 } ?: 0)
        send(Protocol.mouseAbs(nx(ev.x), ny(ev.y), b))
        return true
    }

    // ------------------------------------------------------------ stylus

    private fun stylusEvent(ev: MotionEvent) {
        val idx = (0 until ev.pointerCount).first { isStylus(ev, it) }
        val action = ev.actionMasked
        val isActionPointer = ev.actionIndex == idx
        main.removeCallbacks(penOut)
        when {
            action == MotionEvent.ACTION_CANCEL -> sendPenOut()
            (action == MotionEvent.ACTION_UP || action == MotionEvent.ACTION_POINTER_UP) && isActionPointer -> {
                pen(ev, idx, -1, contact = false)
                penActiveUntil = SystemClock.uptimeMillis() + 400
                // digitizers without hover never send HOVER_EXIT
                main.postDelayed(penOut, 150)
            }
            else -> {
                for (h in 0 until ev.historySize) pen(ev, idx, h, contact = true)
                pen(ev, idx, -1, contact = true)
            }
        }
    }

    /** One pen sample; [h] = history index or -1 for the current sample. */
    private fun pen(ev: MotionEvent, i: Int, h: Int, contact: Boolean) {
        val x = if (h < 0) ev.getX(i) else ev.getHistoricalX(i, h)
        val y = if (h < 0) ev.getY(i) else ev.getHistoricalY(i, h)
        val eraser = ev.getToolType(i) == MotionEvent.TOOL_TYPE_ERASER
        penInRange = true
        penActiveUntil = SystemClock.uptimeMillis() + 400
        if (stylusMode == StylusMode.PEN_PALM) cancelFingers()
        if (stylusMode == StylusMode.MOUSE) {
            var b = if (contact) Protocol.MOUSE_LEFT else 0
            if (ev.buttonState and MotionEvent.BUTTON_STYLUS_PRIMARY != 0) b = if (contact) Protocol.MOUSE_RIGHT else 0
            send(Protocol.mouseAbs(nx(x), ny(y), b))
            lastPen = PenState(eraser, nx(x), ny(y))
            return
        }
        val p = if (!contact) 0f else (if (h < 0) ev.getPressure(i) else ev.getHistoricalPressure(i, h)).coerceIn(0f, 1f)
        val tilt = if (h < 0) ev.getAxisValue(MotionEvent.AXIS_TILT, i) else ev.getHistoricalAxisValue(MotionEvent.AXIS_TILT, i, h)
        val orient = if (h < 0) ev.getAxisValue(MotionEvent.AXIS_ORIENTATION, i) else ev.getHistoricalAxisValue(MotionEvent.AXIS_ORIENTATION, i, h)
        // Android: tilt from the normal + direction; DREAD/Wacom: X/Y tilt in degrees
        val t = tan(tilt.coerceIn(0f, 1.5f).toDouble())
        val tx = Math.toDegrees(atan(t * sin(orient.toDouble()))).toFloat()
        val ty = Math.toDegrees(atan(-t * cos(orient.toDouble()))).toFloat()
        var flags = Protocol.PEN_IN_RANGE
        if (contact) flags = flags or Protocol.PEN_CONTACT
        val bs = ev.buttonState
        if (bs and MotionEvent.BUTTON_STYLUS_PRIMARY != 0) flags = flags or Protocol.PEN_BUTTON1
        if (bs and MotionEvent.BUTTON_STYLUS_SECONDARY != 0) flags = flags or Protocol.PEN_BUTTON2
        send(Protocol.pen(eraser, flags, nx(x), ny(y), if (contact) maxOf(p, 0.001f) else 0f, if (tilt > 0f) tx else 0f, if (tilt > 0f) ty else 0f))
        lastPen = PenState(eraser, nx(x), ny(y))
    }

    private fun sendPenOut() {
        val l = lastPen ?: return
        if (stylusMode == StylusMode.MOUSE) send(Protocol.mouseAbs(l.x, l.y, 0))
        else send(Protocol.pen(l.eraser, 0, l.x, l.y, 0f, 0f, 0f))
        penInRange = false
        lastPen = null
    }

    // ------------------------------------------------------------ fingers

    private val active = LinkedHashMap<Int, Protocol.Contact>()

    private fun cancelFingers() {
        if (active.isNotEmpty() && !fingersCancelled) {
            when (touchMode) {
                TouchMode.TOUCH -> send(Protocol.touch(Protocol.TOUCH_CANCEL, 0, emptyList()))
                else -> releaseGesture()
            }
            fingersCancelled = true
        }
    }

    private fun palmBlocked() = stylusMode == StylusMode.PEN_PALM && (penInRange || SystemClock.uptimeMillis() < penActiveUntil)

    private fun contact(ev: MotionEvent, i: Int): Protocol.Contact {
        val major = ev.getTouchMajor(i) / maxOf(rect.width(), 1f)
        return Protocol.Contact(ev.getPointerId(i), nx(ev.getX(i)), ny(ev.getY(i)), ev.getPressure(i).coerceIn(0f, 1f), major)
    }

    private fun fingerEvent(ev: MotionEvent, fingers: List<Int>) {
        val action = ev.actionMasked
        if (action == MotionEvent.ACTION_DOWN) fingersCancelled = false
        if (palmBlocked() || fingersCancelled) {
            if (action == MotionEvent.ACTION_UP || action == MotionEvent.ACTION_CANCEL) {
                active.clear()
                gesture = null
                fingersCancelled = false
            }
            return
        }
        when (touchMode) {
            TouchMode.TOUCH -> directTouch(ev, fingers)
            TouchMode.MOUSE, TouchMode.TRACKPAD -> gestureEvent(ev, fingers)
        }
    }

    private fun directTouch(ev: MotionEvent, fingers: List<Int>) {
        val action = ev.actionMasked
        val changedIdx = ev.actionIndex
        val changedId = ev.getPointerId(changedIdx)
        val contacts = fingers.map { contact(ev, it) }
        val proto = when (action) {
            MotionEvent.ACTION_DOWN, MotionEvent.ACTION_POINTER_DOWN -> Protocol.TOUCH_DOWN
            MotionEvent.ACTION_UP, MotionEvent.ACTION_POINTER_UP -> Protocol.TOUCH_UP
            MotionEvent.ACTION_CANCEL -> Protocol.TOUCH_CANCEL
            else -> Protocol.TOUCH_MOVE
        }
        if ((proto == Protocol.TOUCH_DOWN || proto == Protocol.TOUCH_UP) && isStylus(ev, changedIdx)) {
            send(Protocol.touch(Protocol.TOUCH_MOVE, 0, contacts))
            return
        }
        active.clear()
        if (proto != Protocol.TOUCH_UP || fingers.size > 1) contacts.forEach { if (!(proto == Protocol.TOUCH_UP && it.id == changedId)) active[it.id] = it }
        send(Protocol.touch(proto, changedId, contacts))
    }

    // ------------------------------------------------------------ mouse / trackpad gestures

    private class Gesture(
        val startTime: Long,
        val startX: Float,
        val startY: Float,
        var lastX: Float,
        var lastY: Float,
        var maxFingers: Int,
        var moved: Boolean = false,
        var dragging: Boolean = false,
        var twoFingerStartDist: Float = 0f,
        var lastDist: Float = 0f,
        var lastCx: Float = 0f,
        var lastCy: Float = 0f,
        var pinching: Boolean = false,
        var scrollAccV: Float = 0f,
        var scrollAccH: Float = 0f,
        var longPressed: Boolean = false,
    )

    private var gesture: Gesture? = null
    private var lastTapUp = 0L
    private var buttons = 0
    private val longPress = Runnable {
        val g = gesture ?: return@Runnable
        if (!g.moved && g.maxFingers == 1 && touchMode == TouchMode.MOUSE) {
            g.longPressed = true
            click(Protocol.MOUSE_RIGHT, g.lastX, g.lastY)
        }
    }

    private fun slop() = 8f * density

    private fun click(button: Int, x: Float, y: Float) {
        if (touchMode == TouchMode.MOUSE) {
            send(Protocol.mouseAbs(nx(x), ny(y), 0))
            send(Protocol.mouseAbs(nx(x), ny(y), button))
            send(Protocol.mouseAbs(nx(x), ny(y), 0))
        } else {
            send(Protocol.mouseRel(0, 0, button))
            send(Protocol.mouseRel(0, 0, 0))
        }
    }

    private fun setButtons(b: Int, x: Float, y: Float) {
        buttons = b
        if (touchMode == TouchMode.MOUSE) send(Protocol.mouseAbs(nx(x), ny(y), b)) else send(Protocol.mouseRel(0, 0, b))
    }

    private fun releaseGesture() {
        val g = gesture ?: return
        if (buttons != 0) setButtons(0, g.lastX, g.lastY)
        if (g.pinching) send(Protocol.key(KeyMap.KEY_LEFTCTRL, false))
        main.removeCallbacks(longPress)
        gesture = null
    }

    private fun gestureEvent(ev: MotionEvent, fingers: List<Int>) {
        val now = SystemClock.uptimeMillis()
        val i0 = fingers.firstOrNull() ?: return
        when (ev.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                val x = ev.getX(i0)
                val y = ev.getY(i0)
                val g = Gesture(now, x, y, x, y, 1)
                gesture = g
                main.postDelayed(longPress, 500)
                // trackpad: tap then touch again quickly = drag
                if (touchMode == TouchMode.TRACKPAD && now - lastTapUp < 250) {
                    g.dragging = true
                    setButtons(Protocol.MOUSE_LEFT, x, y)
                }
                if (touchMode == TouchMode.MOUSE) send(Protocol.mouseAbs(nx(x), ny(y), 0))
            }
            MotionEvent.ACTION_POINTER_DOWN -> {
                val g = gesture ?: return
                g.maxFingers = maxOf(g.maxFingers, fingers.size)
                main.removeCallbacks(longPress)
                if (fingers.size == 2) {
                    val (cx, cy, d) = two(ev, fingers)
                    g.twoFingerStartDist = d; g.lastDist = d; g.lastCx = cx; g.lastCy = cy
                    if (g.dragging || buttons != 0) setButtons(0, g.lastX, g.lastY)
                    g.dragging = false
                }
            }
            MotionEvent.ACTION_MOVE -> {
                val g = gesture ?: return
                if (fingers.size >= 2) {
                    val (cx, cy, d) = two(ev, fingers)
                    if (!g.pinching && abs(d - g.twoFingerStartDist) > 3 * slop() && abs(d - g.twoFingerStartDist) > hypot(cx - g.lastCx, cy - g.lastCy)) {
                        g.pinching = true
                        send(Protocol.key(KeyMap.KEY_LEFTCTRL, true))
                    }
                    if (g.pinching) {
                        // pinch → Ctrl + wheel (zoom in every common desktop app)
                        g.scrollAccV += -(d - g.lastDist) / (12f * density) * 120f
                    } else {
                        g.scrollAccV += -(cy - g.lastCy) / (10f * density) * 120f * sensitivity
                        g.scrollAccH += -(cx - g.lastCx) / (10f * density) * 120f * sensitivity
                    }
                    g.moved = g.moved || hypot(cx - g.lastCx, cy - g.lastCy) > 0.5f
                    val v = g.scrollAccV.toInt()
                    val h = g.scrollAccH.toInt()
                    if (v != 0 || h != 0) {
                        send(Protocol.scroll(v, h))
                        g.scrollAccV -= v; g.scrollAccH -= h
                    }
                    g.lastDist = d; g.lastCx = cx; g.lastCy = cy
                    return
                }
                val x = ev.getX(i0)
                val y = ev.getY(i0)
                if (!g.moved && hypot(x - g.startX, y - g.startY) > slop()) {
                    g.moved = true
                    main.removeCallbacks(longPress)
                    if (touchMode == TouchMode.MOUSE && !g.longPressed && g.maxFingers == 1) {
                        // drag with the left button held, from where the finger landed
                        send(Protocol.mouseAbs(nx(g.startX), ny(g.startY), 0))
                        setButtons(Protocol.MOUSE_LEFT, g.startX, g.startY)
                    }
                }
                if (g.moved) {
                    if (touchMode == TouchMode.MOUSE) {
                        send(Protocol.mouseAbs(nx(x), ny(y), buttons))
                    } else {
                        val k = sensitivity * 1.6f / (density / 2.5f)
                        for (h in 0 until ev.historySize) {
                            val hx = ev.getHistoricalX(i0, h); val hy = ev.getHistoricalY(i0, h)
                            send(Protocol.mouseRel(((hx - g.lastX) * k).toInt(), ((hy - g.lastY) * k).toInt(), buttons))
                            g.lastX = hx; g.lastY = hy
                        }
                        send(Protocol.mouseRel(((x - g.lastX) * k).toInt(), ((y - g.lastY) * k).toInt(), buttons))
                    }
                }
                g.lastX = x; g.lastY = y
            }
            MotionEvent.ACTION_POINTER_UP -> {}
            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                val g = gesture ?: return
                main.removeCallbacks(longPress)
                val quick = now - g.startTime < 300
                if (ev.actionMasked == MotionEvent.ACTION_UP && !g.moved && quick && !g.longPressed) {
                    when (g.maxFingers) {
                        1 -> if (!g.dragging) { click(Protocol.MOUSE_LEFT, g.startX, g.startY); lastTapUp = now }
                        2 -> click(Protocol.MOUSE_RIGHT, g.startX, g.startY)
                        else -> click(Protocol.MOUSE_MIDDLE, g.startX, g.startY)
                    }
                }
                releaseGesture()
            }
        }
    }

    private fun two(ev: MotionEvent, f: List<Int>): Triple<Float, Float, Float> {
        val x0 = ev.getX(f[0]); val y0 = ev.getY(f[0]); val x1 = ev.getX(f[1]); val y1 = ev.getY(f[1])
        return Triple((x0 + x1) / 2, (y0 + y1) / 2, hypot(x1 - x0, y1 - y0))
    }

    /** Lift everything (on disconnect / mode change). */
    fun reset() {
        if (active.isNotEmpty()) send(Protocol.touch(Protocol.TOUCH_CANCEL, 0, emptyList()))
        active.clear()
        releaseGesture()
        if (penInRange) sendPenOut()
    }
}
