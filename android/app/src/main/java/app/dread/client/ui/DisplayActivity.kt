package app.dread.client.ui

import android.app.Activity
import android.content.Context
import android.content.pm.ActivityInfo
import android.graphics.Color
import android.graphics.Point
import android.graphics.RectF
import android.hardware.usb.UsbManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.view.Gravity
import android.view.InputDevice
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.View
import android.view.ViewGroup
import android.view.WindowInsets
import android.view.WindowInsetsController
import android.view.WindowManager
import android.widget.FrameLayout
import android.widget.HorizontalScrollView
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import app.dread.client.DreadApp
import app.dread.client.KeyboardMode
import app.dread.client.R
import app.dread.client.RotationMode
import app.dread.client.TouchMode
import app.dread.client.input.InputMapper
import app.dread.client.input.KeyMap
import app.dread.client.input.KeyboardSink
import app.dread.client.net.ClientInfo
import app.dread.client.net.Protocol
import app.dread.client.net.Session
import app.dread.client.net.SessionListener
import app.dread.client.net.StreamConfig
import app.dread.client.net.Target
import app.dread.client.net.UsbTransport
import app.dread.client.net.VideoFrame
import app.dread.client.ui.Ui.dp
import app.dread.client.video.Decoder
import org.json.JSONObject

/**
 * The external-monitor screen: fullscreen video, input capture, floating
 * toolbar, soft keyboard and status overlays.
 */
class DisplayActivity : Activity(), SessionListener, SurfaceHolder.Callback {
    private val prefs get() = DreadApp.instance.prefs
    private val main = Handler(Looper.getMainLooper())

    private lateinit var root: FrameLayout
    private lateinit var surface: SurfaceView
    private lateinit var overlay: LinearLayout
    private lateinit var overlayTitle: TextView
    private lateinit var overlayDetail: TextView
    private lateinit var overlayCode: TextView
    private lateinit var overlayButtons: LinearLayout
    private lateinit var stats: TextView
    private lateinit var identify: TextView
    private lateinit var toolbar: LinearLayout
    private lateinit var handle: ImageView
    private lateinit var modeIcon: ImageView
    private lateinit var keysBar: HorizontalScrollView
    private lateinit var keyboard: KeyboardSink

    private var target: Target? = null
    private var session: Session? = null
    private var decoder: Decoder? = null
    private lateinit var mapper: InputMapper
    private var stream: StreamConfig? = null
    private var serverStats = JSONObject()
    private var keyboardShown = false
    private var immersive = true
    private var lastBack = 0L
    private var lastFrames = 0L
    private var lastStatsAt = 0L
    private var fps = 0.0
    private val latched = LinkedHashSet<Int>()
    private val latchViews = HashMap<Int, TextView>()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        if (Build.VERSION.SDK_INT >= 28) {
            window.attributes = window.attributes.apply { layoutInDisplayCutoutMode = WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES }
        }
        target = resolveTarget()
        if (target == null) {
            finish()
            return
        }
        if (Build.VERSION.SDK_INT >= 33) {
            // Android 16 no longer calls onBackPressed for back gestures
            onBackInvokedDispatcher.registerOnBackInvokedCallback(android.window.OnBackInvokedDispatcher.PRIORITY_DEFAULT) { handleBack() }
        }
        mapper = InputMapper { s -> session?.send(s) }
        mapper.density = resources.displayMetrics.density
        applyPrefs()
        buildUi()
        setImmersiveBars(true)
    }

    private fun resolveTarget(): Target? {
        if (intent.getBooleanExtra(EXTRA_USB, false)) {
            val acc = DreadApp.instance.pendingAccessory ?: return null
            val usb = getSystemService(Context.USB_SERVICE) as UsbManager
            return Target.Usb(UsbTransport(usb, acc))
        }
        val hosts = intent.getStringArrayExtra(EXTRA_HOSTS)?.toList() ?: return null
        return Target.Network(hosts, intent.getIntExtra(EXTRA_PORT, Protocol.DEFAULT_PORT), intent.getStringExtra(EXTRA_FP), intent.getStringExtra(EXTRA_SECRET), intent.getStringExtra(EXTRA_NAME))
    }

    private fun applyPrefs() {
        mapper.touchMode = prefs.touchMode
        mapper.stylusMode = prefs.stylusMode
        mapper.sensitivity = prefs.touchSensitivity
        requestedOrientation = when (prefs.rotation) {
            RotationMode.AUTO -> ActivityInfo.SCREEN_ORIENTATION_FULL_USER
            RotationMode.LANDSCAPE -> ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
            RotationMode.PORTRAIT -> ActivityInfo.SCREEN_ORIENTATION_SENSOR_PORTRAIT
            RotationMode.LOCKED -> ActivityInfo.SCREEN_ORIENTATION_LOCKED
        }
    }

    override fun onResume() {
        super.onResume()
        applyPrefs()
        if (::modeIcon.isInitialized) updateModeIcon()
        if (::keyboard.isInitialized) keyboard.suggestions = prefs.keyboardSuggestions
        stats.visibility = if (prefs.showStats) View.VISIBLE else View.GONE
        setImmersiveBars(immersive)
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        // system bars can only be hidden reliably once the window has focus
        if (hasFocus) setImmersiveBars(immersive)
    }

    // ------------------------------------------------------------ UI

    private fun buildUi() {
        root = object : FrameLayout(this) {
            override fun onTouchEvent(event: MotionEvent): Boolean {
                if (event.actionMasked == MotionEvent.ACTION_DOWN && Build.VERSION.SDK_INT >= 30) {
                    requestUnbufferedDispatch(event)
                }
                return mapper.onTouchEvent(event)
            }
            override fun onGenericMotionEvent(event: MotionEvent): Boolean = mapper.onGenericMotionEvent(event) || super.onGenericMotionEvent(event)
            override fun performClick(): Boolean = super.performClick()
        }
        root.setBackgroundColor(Color.BLACK)
        root.isFocusable = true
        if (Build.VERSION.SDK_INT >= 30) root.requestUnbufferedDispatch(InputDevice.SOURCE_CLASS_POINTER)

        surface = SurfaceView(this)
        surface.holder.addCallback(this)
        surface.setOnGenericMotionListener { _, e -> mapper.onGenericMotionEvent(e) }
        root.addView(surface, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT, Gravity.CENTER))
        root.addOnLayoutChangeListener { _, _, _, _, _, _, _, _, _ -> layoutVideo() }

        keyboard = KeyboardSink(this) { s -> session?.send(s) }.apply {
            suggestions = prefs.keyboardSuggestions
            latchedModifiers = { latched.toList() }
            clearLatched = { clearLatched() }
        }
        root.addView(keyboard, FrameLayout.LayoutParams(1, 1, Gravity.TOP or Gravity.START))

        stats = Ui.text(this, "", 11f, Ui.LIGHT, mono = true).apply {
            setBackgroundColor(Color.BLACK)
            background = Ui.box(Color.BLACK, Ui.BORDER, 0f, context)
            setPadding(dp(8), dp(6), dp(8), dp(6))
            visibility = if (prefs.showStats) View.VISIBLE else View.GONE
        }
        root.addView(stats, Ui.flp(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.TOP or Gravity.START).apply { setMargins(dp(8), dp(8), 0, 0) })

        identify = Ui.text(this, "", 30f, Ui.TEXT, bold = true).apply {
            gravity = Gravity.CENTER
            background = Ui.box(Color.BLACK, Ui.LIGHT, 0f, context)
            setPadding(dp(28), dp(20), dp(28), dp(20))
            visibility = View.GONE
        }
        root.addView(identify, Ui.flp(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.CENTER))

        buildOverlay()
        buildToolbar()
        buildKeysBar()
        setContentView(root)
        root.setOnApplyWindowInsetsListener { _, insets ->
            val ime = if (Build.VERSION.SDK_INT >= 30) insets.getInsets(WindowInsets.Type.ime()).bottom else @Suppress("DEPRECATION") insets.systemWindowInsetBottom
            (keysBar.layoutParams as FrameLayout.LayoutParams).bottomMargin = ime
            keysBar.requestLayout()
            val visible = ime > dp(80)
            keysBar.visibility = if (visible) View.VISIBLE else View.GONE
            if (!visible && keyboardShown && Build.VERSION.SDK_INT >= 30) keyboardShown = false
            insets
        }
        showStatus("Connecting", describeTarget(), null)
        main.post(statsTick)
    }

    private fun describeTarget() = when (val t = target) {
        is Target.Network -> (t.name?.let { "$it · " } ?: "") + t.hosts.joinToString(", ") + ":" + t.port
        is Target.Usb -> "USB"
        null -> ""
    }

    private fun buildOverlay() {
        overlay = Ui.panel(this).apply {
            gravity = Gravity.CENTER_HORIZONTAL
            minimumWidth = dp(300)
        }
        overlayTitle = Ui.text(this, "", 18f, bold = true).apply { gravity = Gravity.CENTER }
        overlayDetail = Ui.caption(this, "").apply { gravity = Gravity.CENTER; setPadding(0, dp(6), 0, 0) }
        overlayCode = Ui.text(this, "", 40f, Ui.TEXT, bold = true, mono = true).apply { gravity = Gravity.CENTER; letterSpacing = 0.15f; setPadding(0, dp(12), 0, dp(4)) }
        overlayButtons = Ui.horizontal(this).apply { gravity = Gravity.CENTER; setPadding(0, dp(14), 0, 0) }
        overlay.addView(overlayTitle)
        overlay.addView(overlayDetail)
        overlay.addView(overlayCode)
        overlay.addView(overlayButtons)
        root.addView(overlay, Ui.flp(dp(380).coerceAtMost(resources.displayMetrics.widthPixels - dp(32)), ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.CENTER))
    }

    private fun showStatus(title: String, detail: String, code: String?, vararg buttons: Pair<String, () -> Unit>) {
        overlay.visibility = View.VISIBLE
        overlayTitle.text = title
        overlayDetail.text = detail
        overlayCode.visibility = if (code != null) View.VISIBLE else View.GONE
        overlayCode.text = code?.let { it.substring(0, 3) + " " + it.substring(3) } ?: ""
        overlayButtons.removeAllViews()
        val btns = if (buttons.isEmpty()) arrayOf("Cancel" to { finish() }) else buttons
        btns.forEachIndexed { i, (label, action) ->
            overlayButtons.addView(Ui.button(this, label, primary = i == 0 && btns.size > 1) { action() }, Ui.lp(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f, end = if (i < btns.size - 1) dp(8) else 0))
        }
    }

    private fun hideStatus() {
        overlay.visibility = View.GONE
    }

    private fun toolButton(res: Int, desc: String, onClick: () -> Unit) = ImageView(this).apply {
        setImageResource(res)
        contentDescription = desc
        setPadding(dp(10), dp(10), dp(10), dp(10))
        background = Ui.pressable(context, Color.BLACK, Ui.PANEL_HI, Color.BLACK)
        layoutParams = LinearLayout.LayoutParams(dp(46), dp(46))
        setOnClickListener { onClick(); scheduleToolbarHide() }
    }

    private fun buildToolbar() {
        toolbar = Ui.vertical(this).apply {
            background = Ui.box(Color.BLACK, Ui.BORDER, 0f, context)
            setPadding(dp(1), dp(1), dp(1), dp(1))
        }
        toolbar.addView(toolButton(R.drawable.ic_keyboard, "Keyboard") { toggleKeyboard() })
        modeIcon = toolButton(R.drawable.ic_touch, "Input mode") { cycleMode() }
        toolbar.addView(modeIcon)
        toolbar.addView(toolButton(R.drawable.ic_stats, "Statistics") {
            prefs.showStats = !prefs.showStats
            stats.visibility = if (prefs.showStats) View.VISIBLE else View.GONE
        })
        toolbar.addView(toolButton(R.drawable.ic_fullscreen, "Fullscreen") { setImmersiveBars(!immersive) })
        toolbar.addView(toolButton(R.drawable.ic_settings, "Settings") { startActivity(android.content.Intent(this, SettingsActivity::class.java)) })
        toolbar.addView(toolButton(R.drawable.ic_close, "Disconnect") { finish() })
        updateModeIcon()
        root.addView(toolbar, Ui.flp(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.END or Gravity.CENTER_VERTICAL).apply { marginEnd = dp(6) })

        handle = ImageView(this).apply {
            setImageResource(R.drawable.ic_handle)
            alpha = 0.55f
            setPadding(dp(2), dp(12), dp(2), dp(12))
            background = Ui.box(Color.BLACK, Ui.BORDER, 0f, context)
            contentDescription = "Show toolbar"
            setOnClickListener { showToolbar() }
            visibility = View.GONE
        }
        root.addView(handle, Ui.flp(dp(20), dp(64), Gravity.END or Gravity.CENTER_VERTICAL))
        scheduleToolbarHide()
    }

    private val hideToolbar = Runnable {
        if (prefs.toolbarAutoHide && session?.connected == true) {
            toolbar.visibility = View.GONE
            handle.visibility = View.VISIBLE
        }
    }

    private fun scheduleToolbarHide() {
        main.removeCallbacks(hideToolbar)
        main.postDelayed(hideToolbar, 4000)
    }

    private fun showToolbar() {
        toolbar.visibility = View.VISIBLE
        handle.visibility = View.GONE
        scheduleToolbarHide()
    }

    private fun updateModeIcon() {
        modeIcon.setImageResource(
            when (prefs.touchMode) {
                TouchMode.TOUCH -> R.drawable.ic_touch
                TouchMode.MOUSE -> R.drawable.ic_mouse
                TouchMode.TRACKPAD -> R.drawable.ic_trackpad
            },
        )
    }

    private fun cycleMode() {
        mapper.reset()
        val next = TouchMode.values()[(prefs.touchMode.ordinal + 1) % TouchMode.values().size]
        prefs.touchMode = next
        mapper.touchMode = next
        updateModeIcon()
        Toast.makeText(this, "Input: ${next.label}", Toast.LENGTH_SHORT).show()
    }

    private fun buildKeysBar() {
        val row = Ui.horizontal(this).apply { setPadding(dp(4), dp(4), dp(4), dp(4)) }
        fun key(label: String, onTap: (TextView) -> Unit): TextView = Ui.text(this, label, 14f, Ui.TEXT, mono = true).apply {
            gravity = Gravity.CENTER
            background = Ui.pressable(context, Ui.PANEL, Ui.PANEL_HI)
            setPadding(dp(14), dp(10), dp(14), dp(10))
            setOnClickListener { onTap(this) }
        }
        val keys = listOf(
            "Esc" to KeyMap.KEY_ESC, "Tab" to KeyMap.KEY_TAB,
        )
        for ((l, c) in keys) row.addView(key(l) { keyboard.tapKey(c) }, Ui.lp(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT, end = dp(4)))
        for ((l, c) in listOf("Ctrl" to KeyMap.KEY_LEFTCTRL, "Alt" to KeyMap.KEY_LEFTALT, "Shift" to KeyMap.KEY_LEFTSHIFT, "Super" to KeyMap.KEY_LEFTMETA)) {
            val v = key(l) { toggleLatch(c) }
            latchViews[c] = v
            row.addView(v, Ui.lp(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT, end = dp(4)))
        }
        for ((l, c) in listOf("←" to KeyMap.KEY_LEFT, "↑" to KeyMap.KEY_UP, "↓" to KeyMap.KEY_DOWN, "→" to KeyMap.KEY_RIGHT, "Home" to KeyMap.KEY_HOME, "End" to KeyMap.KEY_END, "PgUp" to KeyMap.KEY_PAGEUP, "PgDn" to KeyMap.KEY_PAGEDOWN, "Del" to KeyMap.KEY_DELETE)) {
            row.addView(key(l) { keyboard.tapKey(c) }, Ui.lp(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT, end = dp(4)))
        }
        for (n in 1..12) row.addView(key("F$n") { keyboard.tapKey(if (n <= 10) 58 + n else 76 + n) }, Ui.lp(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT, end = dp(4)))
        row.addView(key("Hide") { toggleKeyboard() })
        keysBar = HorizontalScrollView(this).apply {
            setBackgroundColor(Color.BLACK)
            isHorizontalScrollBarEnabled = false
            addView(row)
            visibility = View.GONE
        }
        root.addView(keysBar, Ui.flp(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.BOTTOM))
    }

    private fun toggleLatch(code: Int) {
        if (!latched.remove(code)) latched.add(code)
        refreshLatches()
    }

    private fun clearLatched() {
        latched.clear()
        main.post { refreshLatches() }
    }

    private fun refreshLatches() {
        for ((c, v) in latchViews) {
            val on = c in latched
            v.setTextColor(if (on) Ui.BG else Ui.TEXT)
            v.background = if (on) Ui.box(Ui.LIGHT, Ui.LIGHT, dp(2f).toFloat(), this) else Ui.pressable(this, Ui.PANEL, Ui.PANEL_HI)
        }
    }

    private fun toggleKeyboard() {
        if (keyboardShown) {
            keyboard.hide()
            keysBar.visibility = View.GONE
            keyboardShown = false
        } else {
            keyboard.show()
            keyboardShown = true
        }
    }

    @Suppress("DEPRECATION")
    private fun setImmersiveBars(on: Boolean) {
        immersive = on
        if (Build.VERSION.SDK_INT >= 30) {
            window.setDecorFitsSystemWindows(!on)
            val c = window.insetsController ?: return
            if (on) {
                c.hide(WindowInsets.Type.statusBars() or WindowInsets.Type.navigationBars())
                c.systemBarsBehavior = WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
            } else c.show(WindowInsets.Type.statusBars() or WindowInsets.Type.navigationBars())
        } else {
            window.decorView.systemUiVisibility = if (on) {
                View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY or View.SYSTEM_UI_FLAG_FULLSCREEN or View.SYSTEM_UI_FLAG_HIDE_NAVIGATION or
                    View.SYSTEM_UI_FLAG_LAYOUT_STABLE or View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION or View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
            } else 0
        }
    }

    /** Fit the video into the screen with its own aspect ratio (letterboxed). */
    private fun layoutVideo() {
        val cfg = stream
        val w = root.width.toFloat()
        val h = root.height.toFloat()
        if (w <= 0 || h <= 0) return
        val (vw, vh) = if (cfg != null) cfg.width.toFloat() to cfg.height.toFloat() else w to h
        val k = minOf(w / vw, h / vh)
        val rw = (vw * k).toInt()
        val rh = (vh * k).toInt()
        val lp = surface.layoutParams as FrameLayout.LayoutParams
        if (lp.width != rw || lp.height != rh) {
            lp.width = rw
            lp.height = rh
            lp.gravity = Gravity.CENTER
            surface.layoutParams = lp
        }
        val left = (w - rw) / 2f
        val top = (h - rh) / 2f
        mapper.rect = RectF(left, top, left + rw, top + rh)
        sendViewport()
    }

    private var lastViewport = Point()

    private fun physicalSize(): Point {
        val p = Point()
        if (Build.VERSION.SDK_INT >= 30) {
            val b = windowManager.currentWindowMetrics.bounds
            p.set(b.width(), b.height())
        } else {
            @Suppress("DEPRECATION")
            windowManager.defaultDisplay.getRealSize(p)
        }
        return p
    }

    private fun sendViewport() {
        val p = physicalSize()
        if (p != lastViewport) {
            lastViewport = Point(p)
            session?.viewport(p.x, p.y)
        }
    }

    private fun clientInfo(): ClientInfo {
        val p = physicalSize()
        val refresh = if (Build.VERSION.SDK_INT >= 30) display?.refreshRate ?: 60f else @Suppress("DEPRECATION") windowManager.defaultDisplay.refreshRate
        val maxFps = if (prefs.maxFps > 0) prefs.maxFps else Math.round(refresh)
        val supported = Decoder.supportedCodecs()
        val codecs = when (val c = prefs.codec) {
            "auto" -> supported
            else -> if (c in supported) listOf(c, "h264").distinct() else listOf("h264")
        }
        val (pw, ph) = prefs.preferredResolution.split("x").let { (it.getOrNull(0)?.toIntOrNull() ?: 0) to (it.getOrNull(1)?.toIntOrNull() ?: 0) }
        val stylus = InputDevice.getDeviceIds().any { id -> InputDevice.getDevice(id)?.supportsSource(InputDevice.SOURCE_STYLUS) == true }
        // the decoder's limit caps what we ask for
        val (mw, mh) = runCatching { Decoder.maxSize() }.getOrDefault(3840 to 2160)
        var w = p.x
        var h = p.y
        val long = maxOf(w, h).toFloat()
        val limit = maxOf(mw, mh).toFloat()
        if (long > limit) {
            w = (w * limit / long).toInt()
            h = (h * limit / long).toInt()
        }
        return ClientInfo(w, h, resources.displayMetrics.densityDpi, maxFps, codecs, 10, stylus, pw, ph)
    }

    // ------------------------------------------------------------ surface / session

    override fun surfaceCreated(holder: SurfaceHolder) {
        decoder = Decoder(holder.surface, prefs.lowLatencyDecoder, { seq, us -> session?.ack(seq, us) }, { session?.requestKeyframe() })
        stream?.let { decoder?.configure(it) }
        if (session == null) startSession()
    }

    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {}

    override fun surfaceDestroyed(holder: SurfaceHolder) {
        // app went to the background: the stream cannot be shown, let the
        // computer keep the display for a reconnect within the linger time
        session?.close()
        session = null
        decoder?.release()
        decoder = null
    }

    private fun startSession() {
        val t = target ?: return
        lastViewport = Point()
        session = Session(prefs, t, { clientInfo() }, this).also { it.start() }
    }

    override fun onDestroy() {
        main.removeCallbacksAndMessages(null)
        session?.close()
        session = null
        decoder?.release()
        (target as? Target.Usb)?.transport?.close()
        super.onDestroy()
    }

    // ------------------------------------------------------------ SessionListener

    override fun onStatus(state: Session.State, detail: String) = runOnUiThread {
        when (state) {
            Session.State.CONNECTING -> showStatus("Connecting", detail, null)
            Session.State.PAIRING -> showStatus("Pairing", "Waiting for the computer…", null)
            Session.State.CONNECTED -> {
                hideStatus()
                scheduleToolbarHide()
            }
            Session.State.RECONNECTING -> {
                mapper.reset()
                showStatus("Reconnecting", detail, null, "Disconnect" to { finish() })
            }
            Session.State.CLOSED -> {
                if (!isFinishing) {
                    showStatus("Disconnected", detail.ifEmpty { "The connection was closed." }, null,
                        "Retry" to { session?.close(); session = null; startSession() },
                        "Back" to { finish() })
                }
            }
        }
    }

    override fun onPairingCode(code: String) = runOnUiThread {
        showStatus("Confirm on the computer", "Check that DREAD on the computer shows the same code, then approve it there.", code)
    }

    override fun onConnected(serverName: String, transport: String) = runOnUiThread {
        Toast.makeText(this, "Connected to $serverName ($transport)", Toast.LENGTH_SHORT).show()
        sendViewport()
    }

    override fun onStreamConfig(cfg: StreamConfig) {
        stream = cfg
        decoder?.configure(cfg)
        runOnUiThread { layoutVideo() }
    }

    override fun onVideoFrame(f: VideoFrame) {
        decoder?.feed(f)
    }

    override fun onIme(show: Boolean) = runOnUiThread {
        if (prefs.keyboardMode != KeyboardMode.AUTO) return@runOnUiThread
        if (show && !keyboardShown) toggleKeyboard()
        else if (!show && keyboardShown) toggleKeyboard()
    }

    override fun onIdentify(name: String, width: Int, height: Int) = runOnUiThread {
        identify.text = "$name\n${width} × ${height}"
        identify.visibility = View.VISIBLE
        main.postDelayed({ identify.visibility = View.GONE }, 3000)
    }

    override fun onServerStats(o: JSONObject) {
        serverStats = o
    }

    override fun onRtt(ms: Double) {}

    private val statsTick = object : Runnable {
        override fun run() {
            val d = decoder
            val now = SystemClock.elapsedRealtime()
            if (d != null && lastStatsAt > 0) {
                fps = (d.framesDecoded - lastFrames) * 1000.0 / (now - lastStatsAt)
            }
            lastFrames = d?.framesDecoded ?: 0
            lastStatsAt = now
            if (stats.visibility == View.VISIBLE) {
                val s = session
                val cfg = stream
                val st = serverStats
                stats.text = buildString {
                    append(if (cfg != null) "${cfg.width}×${cfg.height}  ${cfg.codec.uppercase()}" else "—").append('\n')
                    append("fps      %.1f / %d\n".format(fps, cfg?.fps ?: 0))
                    append("bitrate  %.1f Mbps\n".format(st.optDouble("bitrate_kbps", 0.0) / 1000.0))
                    append("rtt      %.1f ms\n".format(s?.rttMs ?: 0.0))
                    append("encode   %.1f ms\n".format(st.optDouble("encode_ms", 0.0)))
                    append("decode   %.1f ms\n".format(d?.lastDecodeMs ?: 0.0))
                    append("loss     %.2f %%\n".format(st.optDouble("loss_pct", 0.0)))
                    append("link     ${s?.transportLabel ?: ""}\n")
                    append("decoder  ${d?.decoderName ?: ""}")
                }
            }
            main.postDelayed(this, 500)
        }
    }

    // ------------------------------------------------------------ keys

    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        val dev = event.device
        val physical = dev != null && !dev.isVirtual && (dev.sources and InputDevice.SOURCE_KEYBOARD) == InputDevice.SOURCE_KEYBOARD &&
            dev.keyboardType == InputDevice.KEYBOARD_TYPE_ALPHABETIC
        val isSystem = event.keyCode in setOf(KeyEvent.KEYCODE_VOLUME_UP, KeyEvent.KEYCODE_VOLUME_DOWN, KeyEvent.KEYCODE_POWER, KeyEvent.KEYCODE_HOME)
        if (physical && !isSystem && event.keyCode != KeyEvent.KEYCODE_BACK) {
            val code = KeyMap.evdev(event.keyCode)
            if (code != null && session?.connected == true) {
                when (event.action) {
                    KeyEvent.ACTION_DOWN -> if (event.repeatCount == 0) session?.send(Protocol.key(code, true))
                    KeyEvent.ACTION_UP -> session?.send(Protocol.key(code, false))
                }
                return true
            }
        }
        return super.dispatchKeyEvent(event)
    }

    // API 33+ back gestures are handled by the OnBackInvokedCallback registered in onCreate
    @android.annotation.SuppressLint("GestureBackNavigation")
    @Deprecated("framework API")
    override fun onBackPressed() = handleBack()

    private fun handleBack() {
        if (keyboardShown) {
            toggleKeyboard()
            return
        }
        val now = SystemClock.uptimeMillis()
        if (now - lastBack < 2000) {
            finish()
            return
        }
        lastBack = now
        showToolbar()
        Toast.makeText(this, "Press back again to disconnect", Toast.LENGTH_SHORT).show()
    }

    companion object {
        const val EXTRA_HOSTS = "hosts"
        const val EXTRA_PORT = "port"
        const val EXTRA_FP = "fp"
        const val EXTRA_SECRET = "secret"
        const val EXTRA_NAME = "name"
        const val EXTRA_USB = "usb"
    }
}
