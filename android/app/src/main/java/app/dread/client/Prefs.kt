package app.dread.client

import android.content.Context
import android.content.SharedPreferences
import org.json.JSONArray
import org.json.JSONObject
import java.util.UUID

enum class TouchMode(val label: String) {
    TOUCH("Touch"),
    MOUSE("Mouse"),
    TRACKPAD("Trackpad"),
}

enum class StylusMode(val label: String) {
    PEN("Pen (pressure, tilt, eraser)"),
    PEN_PALM("Pen + palm rejection"),
    MOUSE("Mouse pointer"),
}

enum class RotationMode(val label: String) {
    AUTO("Automatic"),
    LANDSCAPE("Landscape"),
    PORTRAIT("Portrait"),
    LOCKED("Locked to current"),
}

enum class KeyboardMode(val label: String) {
    AUTO("Open on text fields"),
    MANUAL("Only from the toolbar"),
}

enum class LinkPref(val label: String) {
    AUTO("Automatic"),
    WIFI("Wi-Fi / LAN only"),
    USB("USB only"),
}

/** A paired DREAD server. */
data class ServerEntry(
    val id: String,
    var name: String,
    var fp: String,
    var token: String,
    var hosts: List<String>,
    var port: Int,
    var lastUsed: Long,
) {
    fun toJson(): JSONObject = JSONObject()
        .put("id", id).put("name", name).put("fp", fp).put("token", token)
        .put("hosts", JSONArray(hosts)).put("port", port).put("lastUsed", lastUsed)

    companion object {
        fun fromJson(o: JSONObject) = ServerEntry(
            o.getString("id"), o.optString("name"), o.getString("fp"), o.getString("token"),
            o.optJSONArray("hosts")?.let { a -> List(a.length()) { a.getString(it) } } ?: emptyList(),
            o.optInt("port", 47800), o.optLong("lastUsed"),
        )
    }
}

class Prefs(ctx: Context) {
    private val sp: SharedPreferences = ctx.getSharedPreferences("dread", Context.MODE_PRIVATE)

    val deviceId: String
        get() = sp.getString("device_id", null) ?: UUID.randomUUID().toString().also { sp.edit().putString("device_id", it).apply() }

    var deviceName: String
        get() = sp.getString("device_name", null) ?: "${android.os.Build.MANUFACTURER} ${android.os.Build.MODEL}".trim()
        set(v) = sp.edit().putString("device_name", v).apply()

    /** "0x0" = native resolution. */
    var preferredResolution: String
        get() = sp.getString("pref_res", "0x0")!!
        set(v) = sp.edit().putString("pref_res", v).apply()

    var maxFps: Int
        get() = sp.getInt("max_fps", 0)
        set(v) = sp.edit().putInt("max_fps", v).apply()

    var codec: String
        get() = sp.getString("codec", "auto")!!
        set(v) = sp.edit().putString("codec", v).apply()

    var touchMode: TouchMode
        get() = runCatching { TouchMode.valueOf(sp.getString("touch_mode", "TOUCH")!!) }.getOrDefault(TouchMode.TOUCH)
        set(v) = sp.edit().putString("touch_mode", v.name).apply()

    /** Trackpad / mouse-mode pointer speed multiplier. */
    var touchSensitivity: Float
        get() = sp.getFloat("touch_sens", 1.0f)
        set(v) = sp.edit().putFloat("touch_sens", v).apply()

    var stylusMode: StylusMode
        get() = runCatching { StylusMode.valueOf(sp.getString("stylus_mode", "PEN_PALM")!!) }.getOrDefault(StylusMode.PEN_PALM)
        set(v) = sp.edit().putString("stylus_mode", v.name).apply()

    var rotation: RotationMode
        get() = runCatching { RotationMode.valueOf(sp.getString("rotation", "AUTO")!!) }.getOrDefault(RotationMode.AUTO)
        set(v) = sp.edit().putString("rotation", v.name).apply()

    var keyboardMode: KeyboardMode
        get() = runCatching { KeyboardMode.valueOf(sp.getString("kbd_mode", "AUTO")!!) }.getOrDefault(KeyboardMode.AUTO)
        set(v) = sp.edit().putString("kbd_mode", v.name).apply()

    var keyboardSuggestions: Boolean
        get() = sp.getBoolean("kbd_suggest", false)
        set(v) = sp.edit().putBoolean("kbd_suggest", v).apply()

    var linkPref: LinkPref
        get() = runCatching { LinkPref.valueOf(sp.getString("link_pref", "AUTO")!!) }.getOrDefault(LinkPref.AUTO)
        set(v) = sp.edit().putString("link_pref", v.name).apply()

    var autoReconnect: Boolean
        get() = sp.getBoolean("auto_reconnect", true)
        set(v) = sp.edit().putBoolean("auto_reconnect", v).apply()

    var showStats: Boolean
        get() = sp.getBoolean("show_stats", false)
        set(v) = sp.edit().putBoolean("show_stats", v).apply()

    var toolbarAutoHide: Boolean
        get() = sp.getBoolean("toolbar_autohide", true)
        set(v) = sp.edit().putBoolean("toolbar_autohide", v).apply()

    var lowLatencyDecoder: Boolean
        get() = sp.getBoolean("low_latency_dec", true)
        set(v) = sp.edit().putBoolean("low_latency_dec", v).apply()

    // ------------------------------------------------------------ servers

    fun servers(): List<ServerEntry> {
        val a = runCatching { JSONArray(sp.getString("servers", "[]")) }.getOrDefault(JSONArray())
        return List(a.length()) { ServerEntry.fromJson(a.getJSONObject(it)) }.sortedByDescending { it.lastUsed }
    }

    fun saveServer(e: ServerEntry) {
        val list = servers().filter { it.id != e.id }.toMutableList()
        list.add(0, e)
        sp.edit().putString("servers", JSONArray(list.map { it.toJson() }).toString()).apply()
    }

    fun forgetServer(id: String) {
        sp.edit().putString("servers", JSONArray(servers().filter { it.id != id }.map { it.toJson() }).toString()).apply()
    }

    fun serverForHost(host: String, port: Int): ServerEntry? = servers().firstOrNull { host in it.hosts && it.port == port }

    fun serverByFp(fp: String): ServerEntry? = servers().firstOrNull { it.fp.equals(fp, ignoreCase = true) }

    /** Last manually entered address. */
    var lastManualHost: String
        get() = sp.getString("last_host", "")!!
        set(v) = sp.edit().putString("last_host", v).apply()

    var lastManualPort: Int
        get() = sp.getInt("last_port", 47800)
        set(v) = sp.edit().putInt("last_port", v).apply()
}
