package app.dread.client.net

import android.os.SystemClock
import android.util.Log
import app.dread.client.Prefs
import app.dread.client.ServerEntry
import org.json.JSONArray
import org.json.JSONObject
import java.io.DataInputStream
import java.io.IOException
import java.security.SecureRandom
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit

/** What to connect to. */
sealed class Target {
    /** Network server. [qrSecret] is set when the address came from a pairing QR code. */
    data class Network(val hosts: List<String>, val port: Int, val fp: String?, val qrSecret: String?, val name: String?) : Target()
    class Usb(val transport: UsbTransport) : Target()
}

data class StreamConfig(val codec: String, val width: Int, val height: Int, val fps: Int, val bitrate: Long, val generation: Long)

class VideoFrame(val flags: Int, val seq: Int, val captureUs: Long, val encodeUs: Int, val data: ByteArray, val offset: Int) {
    val isKey get() = flags and Protocol.VIDEO_FLAG_KEY != 0
    val hasConfig get() = flags and Protocol.VIDEO_FLAG_CONFIG != 0
}

data class ClientInfo(
    val width: Int,
    val height: Int,
    val densityDpi: Int,
    val maxFps: Int,
    val codecs: List<String>,
    val touchPoints: Int,
    val stylus: Boolean,
    val preferredWidth: Int,
    val preferredHeight: Int,
)

interface SessionListener {
    fun onStatus(state: Session.State, detail: String)
    fun onPairingCode(code: String)
    fun onConnected(serverName: String, transport: String)
    fun onStreamConfig(cfg: StreamConfig)
    /** Called on the video thread. */
    fun onVideoFrame(f: VideoFrame)
    fun onIme(show: Boolean)
    fun onIdentify(name: String, width: Int, height: Int)
    fun onServerStats(o: JSONObject)
    fun onRtt(ms: Double)
}

class PairingRejected(reason: String) : IOException(reason)

/**
 * One connection to a DREAD server: handshake, pairing, heartbeat, control
 * and video channels, and automatic reconnection.
 */
class Session(
    private val prefs: Prefs,
    private val target: Target,
    private val info: () -> ClientInfo,
    private val listener: SessionListener,
) {
    enum class State { CONNECTING, PAIRING, CONNECTED, RECONNECTING, CLOSED }

    @Volatile private var running = true
    @Volatile private var transport: Transport? = null
    @Volatile private var control: Channel? = null
    @Volatile private var video: Channel? = null
    private val outQueue = LinkedBlockingQueue<ByteArray>(4096)
    @Volatile var connected = false
        private set
    @Volatile private var lastPong = 0L
    @Volatile var rttMs = 0.0
        private set
    @Volatile var generation = 0L
        private set
    private var thread: Thread? = null
    private val start = SystemClock.elapsedRealtime()
    @Volatile var transportLabel = ""
        private set

    fun start() {
        thread = Thread({ runLoop() }, "dread-session").apply { start() }
    }

    /** Queue a protocol frame for the control channel (any thread). */
    fun send(frame: ByteArray) {
        if (connected) outQueue.offer(frame)
    }

    fun ack(seq: Int, decodeUs: Long) {
        send(Protocol.msg("ack", "seq" to seq, "gen" to generation, "decode_us" to decodeUs))
    }

    fun requestKeyframe() = send(Protocol.msg("request_keyframe"))

    fun viewport(w: Int, h: Int) = send(Protocol.msg("viewport", "width" to w, "height" to h))

    fun close() {
        running = false
        if (connected) {
            runCatching { control?.output?.let { Protocol.write(it, Protocol.msg("bye")) } }
        }
        teardown()
        thread?.interrupt()
    }

    private fun teardown() {
        connected = false
        runCatching { control?.close() }
        runCatching { video?.close() }
        if (target is Target.Network) runCatching { transport?.close() }
        control = null
        video = null
        outQueue.clear()
    }

    private fun runLoop() {
        var attempt = 0
        var everConnected = false
        while (running) {
            listener.onStatus(if (everConnected) State.RECONNECTING else State.CONNECTING, describeTarget())
            try {
                connectOnce()
                everConnected = true
                attempt = 0
                pumpUntilDisconnected()
            } catch (e: PairingRejected) {
                closeReason = e.message ?: "pairing rejected"
                running = false
            } catch (e: FingerprintMismatch) {
                closeReason = "${e.message}. Forget this server and pair again."
                running = false
            } catch (e: Exception) {
                Log.w(TAG, "connection failed", e)
                if (!running) break
                val auto = prefs.autoReconnect
                if (!auto || (!everConnected && attempt >= 2)) {
                    closeReason = e.message ?: e.javaClass.simpleName
                    running = false
                } else {
                    listener.onStatus(State.RECONNECTING, e.message ?: "connection lost")
                }
            } finally {
                teardown()
            }
            if (!running) break
            attempt++
            try {
                Thread.sleep(minOf(500L * attempt, 2000L))
            } catch (_: InterruptedException) {
            }
        }
        listener.onStatus(State.CLOSED, closeReason)
    }

    private fun describeTarget(): String = when (target) {
        is Target.Network -> target.hosts.joinToString(", ") + ":" + target.port
        is Target.Usb -> "USB"
    }

    private fun knownEntry(): ServerEntry? = when (target) {
        is Target.Network -> target.fp?.let { prefs.serverByFp(it) } ?: target.hosts.firstNotNullOfOrNull { prefs.serverForHost(it, target.port) }
        is Target.Usb -> null
    }

    private fun openTransport(entry: ServerEntry?): Pair<Transport, Channel> = when (target) {
        is Target.Usb -> target.transport.let { it to it.openControl() }
        is Target.Network -> {
            val pin = (target.fp ?: entry?.fp)?.let { Protocol.unhex(it) }
            var last: Exception? = null
            val hosts = (target.hosts + (entry?.hosts ?: emptyList())).distinct()
            var result: Pair<Transport, Channel>? = null
            for (h in hosts) {
                try {
                    val t = TlsTransport(h, target.port, pin)
                    result = t to t.openControl()
                    lastHost = h
                    break
                } catch (e: FingerprintMismatch) {
                    throw e
                } catch (e: Exception) {
                    last = e
                }
            }
            result ?: throw (last ?: IOException("no address to connect to"))
        }
    }

    @Volatile private var lastHost: String? = null
    @Volatile private var closeReason = ""

    private fun connectOnce() {
        // USB: the token is stored per server; we learn which server after
        // pairing/welcome, so try every known token by server id via hello.
        var entry = knownEntry()
        if (target is Target.Usb) entry = prefs.servers().firstOrNull()
        val (t, ctl) = openTransport(entry)
        transport = t
        control = ctl
        transportLabel = t.label
        val cin = DataInputStream(ctl.input)
        val ci = info()
        val client = JSONObject()
            .put("width", ci.width).put("height", ci.height).put("density_dpi", ci.densityDpi)
            .put("max_fps", ci.maxFps).put("codecs", JSONArray(ci.codecs))
            .put("touch_points", ci.touchPoints).put("stylus", ci.stylus)
            .put("preferred_width", ci.preferredWidth).put("preferred_height", ci.preferredHeight)
        val hello = JSONObject()
            .put("t", "hello").put("proto", Protocol.VERSION).put("role", "control")
            .put("device_id", prefs.deviceId).put("device_name", prefs.deviceName)
            .put("token", entry?.token ?: JSONObject.NULL)
            .put("client", client)
        Protocol.write(ctl.output, Protocol.json(hello))

        var msg = Protocol.readJson(cin)
        var newToken: String? = null
        var pairedFp: String? = null
        if (msg.optString("t") == "pair_required") {
            listener.onStatus(State.PAIRING, "")
            val serverFp = t.certFingerprint ?: Protocol.unhex(msg.getString("fp"))
            val qr = (target as? Target.Network)?.qrSecret
            if (qr != null) {
                Protocol.write(ctl.output, Protocol.msg("pair_qr", "secret" to qr))
            } else {
                val rnd = SecureRandom()
                val nc = ByteArray(32).also { rnd.nextBytes(it) }
                Protocol.write(ctl.output, Protocol.msg("pair_commit", "commit" to Protocol.hex(Protocol.sha256(nc))))
                val n = Protocol.readJson(cin)
                if (n.optString("t") != "pair_nonce") throw IOException("unexpected pairing reply")
                val ns = Protocol.unhex(n.getString("ns"))
                Protocol.write(ctl.output, Protocol.msg("pair_reveal", "nc" to Protocol.hex(nc)))
                listener.onPairingCode(Protocol.pairingCode(serverFp, nc, ns))
            }
            val r = Protocol.readJson(cin)
            when (r.optString("t")) {
                "paired" -> {
                    newToken = r.getString("token")
                    pairedFp = Protocol.hex(serverFp)
                }
                "pair_rejected" -> throw PairingRejected(r.optString("reason", "pairing rejected"))
                else -> throw IOException("unexpected pairing reply ${r.optString("t")}")
            }
            msg = Protocol.readJson(cin)
        }
        when (msg.optString("t")) {
            "welcome" -> {}
            "bye" -> throw PairingRejected(msg.optString("reason", "server closed the connection"))
            else -> throw IOException("unexpected handshake reply ${msg.optString("t")}")
        }
        val serverId = msg.optString("server_id")
        val serverName = msg.optString("server_name", "DREAD Server")
        val fp = pairedFp ?: t.certFingerprint?.let { Protocol.hex(it) } ?: msg.optString("fp")
        val existing = prefs.servers().firstOrNull { it.id == serverId }
        val hosts = buildList {
            lastHost?.let { add(it) }
            (target as? Target.Network)?.hosts?.let { addAll(it) }
            existing?.hosts?.let { addAll(it) }
        }.distinct().take(6)
        val token = newToken ?: existing?.token ?: entry?.token
        if (token != null) {
            prefs.saveServer(
                ServerEntry(serverId, serverName, fp, token, hosts.ifEmpty { existing?.hosts ?: emptyList() },
                    (target as? Target.Network)?.port ?: existing?.port ?: Protocol.DEFAULT_PORT, System.currentTimeMillis()),
            )
        }

        // video channel
        val v = if (target is Target.Usb) target.transport.openVideo() else (t as TlsTransport).openVideo()
        video = v
        val vin = DataInputStream(v.input)
        Protocol.write(v.output, Protocol.msg("hello", "proto" to Protocol.VERSION, "role" to "video", "session" to msg.getString("session"), "video_key" to msg.getString("video_key")))
        val ok = Protocol.readJson(vin)
        if (ok.optString("t") != "video_ok") throw IOException(ok.optString("reason", "video channel rejected"))

        connected = true
        lastPong = SystemClock.elapsedRealtime()
        listener.onConnected(serverName, t.label)
        listener.onStatus(State.CONNECTED, serverName)
        startVideoReader(vin)
        startWriter(ctl)
    }

    private fun startWriter(ctl: Channel) {
        Thread({
            try {
                var nextPing = 0L
                while (connected) {
                    val now = SystemClock.elapsedRealtime()
                    val ping = if (now >= nextPing) {
                        nextPing = now + 1000
                        Protocol.msg("ping", "ts" to (now - start).toDouble(), "rtt_ms" to rttMs)
                    } else null
                    val f = if (ping != null) null else outQueue.poll(50, TimeUnit.MILLISECONDS)
                    synchronized(ctl.output) {
                        ping?.let { ctl.output.write(it) }
                        f?.let { ctl.output.write(it) }
                        // batch whatever is already queued into one flush
                        while (true) {
                            val g = outQueue.poll() ?: break
                            ctl.output.write(g)
                        }
                        ctl.output.flush()
                    }
                }
            } catch (e: Exception) {
                Log.d(TAG, "writer ended: ${e.message}")
                connected = false
            }
        }, "dread-writer").apply { priority = Thread.MAX_PRIORITY; start() }
    }

    private fun startVideoReader(vin: DataInputStream) {
        Thread({
            android.os.Process.setThreadPriority(android.os.Process.THREAD_PRIORITY_URGENT_DISPLAY)
            try {
                while (connected) {
                    val f = Protocol.read(vin)
                    when (f.kind) {
                        Protocol.KIND_JSON -> {
                            val o = JSONObject(String(f.payload, Charsets.UTF_8))
                            if (o.optString("t") == "stream_config") {
                                generation = o.optLong("generation")
                                listener.onStreamConfig(
                                    StreamConfig(o.optString("codec", "h264"), o.getInt("width"), o.getInt("height"), o.optInt("fps", 60), o.optLong("bitrate"), generation),
                                )
                            }
                        }
                        Protocol.KIND_VIDEO -> {
                            val p = f.payload
                            if (p.size < 17) continue
                            val flags = p[0].toInt() and 0xff
                            val seq = java.nio.ByteBuffer.wrap(p, 1, 4).int
                            val cap = java.nio.ByteBuffer.wrap(p, 5, 8).long
                            val enc = java.nio.ByteBuffer.wrap(p, 13, 4).int
                            listener.onVideoFrame(VideoFrame(flags, seq, cap, enc, p, 17))
                        }
                    }
                }
            } catch (e: Exception) {
                Log.d(TAG, "video reader ended: ${e.message}")
            }
            connected = false
        }, "dread-video").start()
    }

    /** Reads the control channel on the session thread until the link drops. */
    private fun pumpUntilDisconnected() {
        val cin = DataInputStream(control!!.input)
        val watchdog = Thread({
            while (connected) {
                try {
                    Thread.sleep(500)
                } catch (_: InterruptedException) {
                    break
                }
                if (SystemClock.elapsedRealtime() - lastPong > HEARTBEAT_TIMEOUT_MS) {
                    Log.w(TAG, "heartbeat timeout")
                    connected = false
                    runCatching { control?.close() }
                    runCatching { video?.close() }
                    runCatching { if (target is Target.Network) transport?.close() }
                }
            }
        }, "dread-watchdog").apply { isDaemon = true; start() }
        try {
            while (connected && running) {
                val f = Protocol.read(cin)
                if (f.kind != Protocol.KIND_JSON) continue
                val o = JSONObject(String(f.payload, Charsets.UTF_8))
                when (o.optString("t")) {
                    "pong" -> {
                        lastPong = SystemClock.elapsedRealtime()
                        val sent = o.optDouble("ts", 0.0)
                        rttMs = (lastPong - start) - sent
                        listener.onRtt(rttMs)
                    }
                    "ime" -> listener.onIme(o.optBoolean("show"))
                    "identify" -> listener.onIdentify(o.optString("name"), o.optInt("width"), o.optInt("height"))
                    "stats" -> listener.onServerStats(o)
                    "bye" -> {
                        closeReason = o.optString("reason", "server closed the session")
                        running = false
                        throw IOException(closeReason)
                    }
                }
            }
        } finally {
            connected = false
            watchdog.interrupt()
        }
        if (running) throw IOException("connection lost")
    }

    companion object {
        private const val TAG = "DreadSession"
        const val HEARTBEAT_TIMEOUT_MS = 6000L
    }
}
