package app.dread.client.video

import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaFormat
import android.os.Build
import android.os.SystemClock
import android.util.Log
import android.view.Surface
import app.dread.client.net.StreamConfig
import app.dread.client.net.VideoFrame
import java.nio.ByteBuffer
import java.util.concurrent.ConcurrentHashMap

/**
 * Hardware video decoder rendering straight into a [Surface].
 *
 * Frames are rendered the moment the decoder releases them (no presentation
 * scheduling) — the stream is real time, so the newest picture always wins.
 */
class Decoder(
    private val surface: Surface,
    private val lowLatency: Boolean,
    private val onDecoded: (seq: Int, decodeUs: Long) -> Unit,
    private val needKeyframe: () -> Unit,
) {
    @Volatile private var codec: MediaCodec? = null
    @Volatile private var config: StreamConfig? = null
    @Volatile private var waitingForKey = true
    private var output: Thread? = null
    private val queuedAt = ConcurrentHashMap<Long, Long>()
    @Volatile var decoderName = ""
        private set
    @Volatile var lastDecodeMs = 0.0
        private set
    @Volatile var framesDecoded = 0L
        private set
    @Volatile private var released = false

    fun configure(cfg: StreamConfig) {
        synchronized(this) {
            config = cfg
            stopCodec()
            waitingForKey = true
        }
    }

    /** Called on the network thread for every received frame. */
    fun feed(f: VideoFrame) {
        synchronized(this) {
            if (released) return
            val cfg = config ?: return
            if (waitingForKey) {
                if (!f.isKey) return
                if (codec == null) {
                    try {
                        startCodec(cfg, f)
                    } catch (e: Exception) {
                        Log.e(TAG, "cannot start decoder", e)
                        return
                    }
                }
                waitingForKey = false
            }
            val c = codec ?: return
            try {
                val idx = c.dequeueInputBuffer(30_000)
                if (idx < 0) {
                    // decoder saturated: drop and resynchronise on the next key frame
                    waitingForKey = true
                    needKeyframe()
                    return
                }
                val buf = c.getInputBuffer(idx)!!
                buf.clear()
                val len = f.data.size - f.offset
                if (len > buf.capacity()) {
                    c.queueInputBuffer(idx, 0, 0, 0, 0)
                    waitingForKey = true
                    needKeyframe()
                    return
                }
                buf.put(f.data, f.offset, len)
                val pts = f.seq.toLong() * 1000L
                queuedAt[pts] = SystemClock.elapsedRealtimeNanos()
                c.queueInputBuffer(idx, 0, len, pts, 0)
            } catch (e: Exception) {
                Log.w(TAG, "decoder input error, restarting", e)
                stopCodec()
                waitingForKey = true
                needKeyframe()
            }
        }
    }

    private fun mime(codec: String) = when (codec) {
        "hevc" -> MediaFormat.MIMETYPE_VIDEO_HEVC
        "av1" -> "video/av01"
        else -> MediaFormat.MIMETYPE_VIDEO_AVC
    }

    /** Splits Annex-B data into NAL units (including their start codes). */
    private fun nals(data: ByteArray, off: Int): List<Pair<Int, Int>> {
        val starts = mutableListOf<Int>()
        var i = off
        while (i + 3 <= data.size) {
            if (data[i].toInt() == 0 && data[i + 1].toInt() == 0 && data[i + 2].toInt() == 1) {
                starts.add(if (i > off && data[i - 1].toInt() == 0) i - 1 else i)
                i += 3
            } else i++
        }
        return starts.mapIndexed { n, s -> s to (if (n + 1 < starts.size) starts[n + 1] else data.size) }
    }

    private fun nalType(codec: String, data: ByteArray, start: Int): Int {
        var h = start
        while (data[h].toInt() == 0) h++
        h++ // skip the 0x01
        return if (codec == "hevc") (data[h].toInt() shr 1) and 0x3f else data[h].toInt() and 0x1f
    }

    /** Copy of [fmt] (MediaFormat's copy constructor needs API 29). */
    private fun baseFormat(cfg: StreamConfig, mime: String, fmt: MediaFormat): MediaFormat {
        val f = MediaFormat.createVideoFormat(mime, cfg.width, cfg.height)
        f.setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, fmt.getInteger(MediaFormat.KEY_MAX_INPUT_SIZE))
        for (k in listOf("csd-0", "csd-1")) if (fmt.containsKey(k)) f.setByteBuffer(k, fmt.getByteBuffer(k))
        return f
    }

    private fun startCodec(cfg: StreamConfig, key: VideoFrame) {
        val mime = mime(cfg.codec)
        val fmt = MediaFormat.createVideoFormat(mime, cfg.width, cfg.height)
        fmt.setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, maxOf(cfg.width * cfg.height, 1 shl 20))
        if (cfg.codec == "h264" || cfg.codec == "hevc") {
            val parts = nals(key.data, key.offset)
            val cs = java.io.ByteArrayOutputStream()
            var pps: ByteArray? = null
            for ((s, e) in parts) {
                val t = nalType(cfg.codec, key.data, s)
                if (cfg.codec == "h264") {
                    if (t == 7) cs.write(key.data, s, e - s)
                    if (t == 8) pps = key.data.copyOfRange(s, e)
                } else if (t == 32 || t == 33 || t == 34) cs.write(key.data, s, e - s)
            }
            if (cs.size() > 0) fmt.setByteBuffer("csd-0", ByteBuffer.wrap(cs.toByteArray()))
            pps?.let { fmt.setByteBuffer("csd-1", ByteBuffer.wrap(it)) }
        }
        val name = pickDecoder(mime)
        val c = if (name != null) MediaCodec.createByCodecName(name) else MediaCodec.createDecoderByType(mime)
        val tuned = baseFormat(cfg, mime, fmt)
        if (lowLatency) {
            if (Build.VERSION.SDK_INT >= 30) tuned.setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
            tuned.setInteger(MediaFormat.KEY_PRIORITY, 0)
            // vendor switches for decoders that predate KEY_LOW_LATENCY
            tuned.setInteger("vendor.qti-ext-dec-low-latency.enable", 1)
            tuned.setInteger("vendor.low-latency.enable", 1)
            tuned.setInteger("vendor.rtc-ext-dec-low-latency.enable", 1)
        }
        try {
            c.configure(tuned, surface, null, 0)
        } catch (e: Exception) {
            Log.w(TAG, "low-latency configure rejected, using defaults: ${e.message}")
            c.reset()
            c.configure(fmt, surface, null, 0)
        }
        c.start()
        codec = c
        decoderName = c.name
        Log.i(TAG, "decoder ${c.name} ${cfg.codec} ${cfg.width}x${cfg.height}")
        val t = Thread({ drain(c) }, "dread-decoder-out")
        t.priority = Thread.MAX_PRIORITY
        output = t
        t.start()
    }

    private fun drain(c: MediaCodec) {
        android.os.Process.setThreadPriority(android.os.Process.THREAD_PRIORITY_URGENT_DISPLAY)
        val info = MediaCodec.BufferInfo()
        while (codec === c && !released) {
            val idx = try {
                c.dequeueOutputBuffer(info, 20_000)
            } catch (_: Exception) {
                break
            }
            if (idx >= 0) {
                val pts = info.presentationTimeUs
                try {
                    c.releaseOutputBuffer(idx, true)
                } catch (_: Exception) {
                    break
                }
                val t0 = queuedAt.remove(pts)
                if (t0 != null) {
                    val us = (SystemClock.elapsedRealtimeNanos() - t0) / 1000
                    lastDecodeMs = us / 1000.0
                    framesDecoded++
                    onDecoded((pts / 1000).toInt(), us)
                }
                if (queuedAt.size > 120) queuedAt.clear()
            }
        }
    }

    private fun stopCodec() {
        val c = codec ?: return
        codec = null
        runCatching { c.stop() }
        runCatching { c.release() }
        output?.join(200)
        output = null
        queuedAt.clear()
    }

    fun release() {
        synchronized(this) {
            released = true
            stopCodec()
        }
    }

    companion object {
        private const val TAG = "DreadDecoder"

        private fun isHardware(info: MediaCodecInfo): Boolean {
            if (Build.VERSION.SDK_INT >= 29) return info.isHardwareAccelerated
            val n = info.name.lowercase()
            return !(n.startsWith("omx.google.") || n.startsWith("c2.android.") || n.contains("sw"))
        }

        /** Hardware decoder for [mime] if any, else null (framework default). */
        fun pickDecoder(mime: String): String? {
            val list = MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos.filter { !it.isEncoder && it.supportedTypes.any { t -> t.equals(mime, true) } }
            val hw = list.filter { isHardware(it) }
            val lowLat = if (Build.VERSION.SDK_INT >= 30) hw.firstOrNull {
                runCatching { it.getCapabilitiesForType(mime).isFeatureSupported(MediaCodecInfo.CodecCapabilities.FEATURE_LowLatency) }.getOrDefault(false)
            } else null
            return (lowLat ?: hw.firstOrNull() ?: list.firstOrNull())?.name
        }

        /** Codecs this device decodes in hardware ("h264" is always offered). */
        fun supportedCodecs(): List<String> {
            val out = mutableListOf("h264")
            val all = MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos.filter { !it.isEncoder && isHardware(it) }
            if (all.any { i -> i.supportedTypes.any { it.equals(MediaFormat.MIMETYPE_VIDEO_HEVC, true) } }) out.add("hevc")
            if (Build.VERSION.SDK_INT >= 29 && all.any { i -> i.supportedTypes.any { it.equals(MediaFormat.MIMETYPE_VIDEO_AV1, true) } }) out.add("av1")
            return out
        }

        /** Largest frame the hardware H.264 decoder accepts (for the resolution menu). */
        fun maxSize(mime: String = MediaFormat.MIMETYPE_VIDEO_AVC): Pair<Int, Int> {
            val name = pickDecoder(mime) ?: return 1920 to 1080
            val info = MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos.first { it.name == name }
            val v = info.getCapabilitiesForType(mime).videoCapabilities ?: return 1920 to 1080
            return v.supportedWidths.upper to v.supportedHeights.upper
        }
    }
}
