package app.dread.client.ui

import android.app.Activity
import android.app.AlertDialog
import android.os.Build
import android.os.Bundle
import android.view.ViewGroup
import android.widget.LinearLayout
import android.widget.ScrollView
import app.dread.client.DreadApp
import app.dread.client.KeyboardMode
import app.dread.client.LinkPref
import app.dread.client.R
import app.dread.client.RotationMode
import app.dread.client.StylusMode
import app.dread.client.TouchMode
import app.dread.client.ui.Ui.dp
import app.dread.client.video.Decoder

class SettingsActivity : Activity() {
    private val prefs get() = DreadApp.instance.prefs
    private lateinit var list: LinearLayout

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        render()
    }

    private fun onOff(b: Boolean) = if (b) "On" else "Off"

    private fun render() {
        val root = Ui.vertical(this, dp(20))
        val header = Ui.horizontal(this)
        header.addView(Ui.icon(this, R.drawable.ic_back, 26).apply {
            layoutParams = LinearLayout.LayoutParams(dp(40), dp(40))
            setPadding(dp(6), dp(6), dp(6), dp(6))
            setOnClickListener { finish() }
            contentDescription = "Back"
        })
        header.addView(Ui.text(this, "Settings", 22f, bold = true), Ui.lp(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
        root.addView(header)
        list = Ui.vertical(this)
        root.addView(list)

        section("Video")
        val (mw, mh) = runCatching { Decoder.maxSize() }.getOrDefault(1920 to 1080)
        val resOptions = listOf("0x0", "1280x720", "1280x800", "1920x1080", "1920x1200", "2560x1440", "2560x1600", "3840x2160").filter { r ->
            val (w, h) = r.split("x").map { it.toInt() }
            r == "0x0" || (maxOf(w, h) <= maxOf(mw, mh) && minOf(w, h) <= minOf(mw, mh))
        }
        fun resLabel(r: String) = if (r == "0x0") "Native (match this screen)" else r.replace("x", " × ")
        item("Preferred resolution", resLabel(prefs.preferredResolution)) {
            Ui.choose(this, "Preferred resolution", resOptions, ::resLabel, prefs.preferredResolution) { prefs.preferredResolution = it; render() }
        }
        val refresh = if (Build.VERSION.SDK_INT >= 30) display?.supportedModes?.map { Math.round(it.refreshRate) }?.distinct()?.sorted() ?: listOf(60)
        else listOf(60)
        val fpsOptions = listOf(0) + listOf(30, 45, 60, 90, 120).filter { it <= (refresh.maxOrNull() ?: 60) }
        fun fpsLabel(f: Int) = if (f == 0) "Screen refresh rate" else "$f fps"
        item("Maximum decoding FPS", fpsLabel(prefs.maxFps)) {
            Ui.choose(this, "Maximum decoding FPS", fpsOptions, ::fpsLabel, prefs.maxFps) { prefs.maxFps = it; render() }
        }
        val codecs = listOf("auto") + Decoder.supportedCodecs()
        fun codecLabel(c: String) = when (c) { "auto" -> "Automatic (best supported)"; "h264" -> "H.264"; "hevc" -> "H.265 / HEVC"; "av1" -> "AV1"; else -> c }
        item("Video quality / codec", codecLabel(prefs.codec)) {
            Ui.choose(this, "Codec", codecs, ::codecLabel, prefs.codec) { prefs.codec = it; render() }
        }
        item("Low-latency decoder", onOff(prefs.lowLatencyDecoder)) { prefs.lowLatencyDecoder = !prefs.lowLatencyDecoder; render() }
        list.addView(Ui.caption(this, "Bitrate, frame rate and compression are set on the computer (Streaming page). Hardware decoders on this device: ${Decoder.supportedCodecs().joinToString { codecLabel(it) }}."))

        section("Touch and stylus")
        item("Default input mode", prefs.touchMode.label) {
            Ui.choose(this, "Input mode", TouchMode.values().toList(), { it.label }, prefs.touchMode) { prefs.touchMode = it; render() }
        }
        val sens = listOf(0.5f, 0.75f, 1.0f, 1.25f, 1.5f, 2.0f, 3.0f)
        item("Touch sensitivity", "×%.2f".format(prefs.touchSensitivity)) {
            Ui.choose(this, "Trackpad / scroll sensitivity", sens, { "×%.2f".format(it) }, prefs.touchSensitivity) { prefs.touchSensitivity = it; render() }
        }
        item("Stylus behaviour", prefs.stylusMode.label) {
            Ui.choose(this, "Stylus behaviour", StylusMode.values().toList(), { it.label }, prefs.stylusMode) { prefs.stylusMode = it; render() }
        }
        list.addView(Ui.caption(this, "Touch mode sends real multitouch. Mouse mode: tap = click, long press = right click, drag = drag, two fingers = scroll, pinch = zoom. Trackpad mode moves the pointer relatively; two-finger tap = right click."))

        section("Screen")
        item("Screen rotation", prefs.rotation.label) {
            Ui.choose(this, "Screen rotation", RotationMode.values().toList(), { it.label }, prefs.rotation) { prefs.rotation = it; render() }
        }
        item("Hide toolbar automatically", onOff(prefs.toolbarAutoHide)) { prefs.toolbarAutoHide = !prefs.toolbarAutoHide; render() }
        item("Statistics overlay", onOff(prefs.showStats)) { prefs.showStats = !prefs.showStats; render() }

        section("Keyboard")
        item("Virtual keyboard", prefs.keyboardMode.label) {
            Ui.choose(this, "Virtual keyboard", KeyboardMode.values().toList(), { it.label }, prefs.keyboardMode) { prefs.keyboardMode = it; render() }
        }
        item("Suggestions and autocorrect", onOff(prefs.keyboardSuggestions)) { prefs.keyboardSuggestions = !prefs.keyboardSuggestions; render() }

        section("Connection")
        item("Connection method", prefs.linkPref.label) {
            Ui.choose(this, "Connection method", LinkPref.values().toList(), { it.label }, prefs.linkPref) { prefs.linkPref = it; render() }
        }
        item("Reconnect automatically", onOff(prefs.autoReconnect)) { prefs.autoReconnect = !prefs.autoReconnect; render() }
        item("Device name", prefs.deviceName) {
            val input = Ui.input(this, "Name shown on the computer", prefs.deviceName)
            AlertDialog.Builder(this).setTitle("Device name").setView(input)
                .setPositiveButton("Save") { _, _ -> prefs.deviceName = input.text.toString().trim().ifEmpty { prefs.deviceName }; render() }
                .setNegativeButton("Cancel", null).show()
        }

        section("About")
        list.addView(Ui.caption(this, "DREAD Client ${packageManager.getPackageInfo(packageName, 0).versionName}\nDevice id ${prefs.deviceId.take(8)}…\nProtocol v1 · TLS with pinned server certificates"))

        setContentView(ScrollView(this).apply {
            setBackgroundColor(Ui.BG)
            addView(root)
        })
    }

    private fun section(t: String) = list.addView(Ui.sectionTitle(this, t))

    private fun item(title: String, value: String, onClick: () -> Unit) {
        val (row, _) = Ui.row(this, title, value, onClick)
        list.addView(row)
        list.addView(Ui.divider(this))
    }
}
