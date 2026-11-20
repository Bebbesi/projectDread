package app.dread.client.net

import android.hardware.usb.UsbAccessory
import android.hardware.usb.UsbManager
import android.os.ParcelFileDescriptor
import java.io.FileInputStream
import java.io.FileOutputStream
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream
import java.net.InetSocketAddress
import java.net.Socket
import java.security.SecureRandom
import java.security.cert.CertificateException
import java.security.cert.X509Certificate
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSocket
import javax.net.ssl.X509TrustManager

/** A bidirectional byte channel. */
class Channel(val input: InputStream, val output: OutputStream, private val onClose: () -> Unit) {
    fun close() = runCatching { onClose() }.let { }
}

/** Opens DREAD's two logical channels over some physical link. */
interface Transport {
    val label: String
    /** SHA-256 of the server certificate seen on the control channel (null without TLS). */
    val certFingerprint: ByteArray?
    fun openControl(): Channel
    fun openVideo(): Channel
    fun close()
}

class FingerprintMismatch(expected: String, seen: String) :
    IOException("server identity changed (expected ${expected.take(16)}…, got ${seen.take(16)}…)")

/**
 * TLS over TCP. The server certificate is self-signed; trust comes from
 * pinning its SHA-256 fingerprint (from the QR code or a previous pairing).
 * With [expectedFp] == null the certificate is accepted and recorded: the
 * code-comparison pairing then binds it, so a substituted certificate makes
 * the codes differ.
 */
class TlsTransport(
    private val host: String,
    private val port: Int,
    private val expectedFp: ByteArray?,
) : Transport {
    override val label = "Wi-Fi / LAN"
    @Volatile override var certFingerprint: ByteArray? = null
        private set
    private val sockets = mutableListOf<Socket>()

    // Not a CA check on purpose: trust is the pinned SHA-256 of the server's
    // self-signed certificate (QR code / pairing), which is stricter than CA trust.
    @android.annotation.SuppressLint("CustomX509TrustManager")
    private inner class Pinner : X509TrustManager {
        override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) =
            throw CertificateException("client auth not used")

        override fun checkServerTrusted(chain: Array<out X509Certificate>, authType: String?) {
            if (chain.isEmpty()) throw CertificateException("no certificate")
            val fp = Protocol.sha256(chain[0].encoded)
            val exp = expectedFp ?: certFingerprint
            if (exp != null && !exp.contentEquals(fp)) {
                throw CertificateException(FingerprintMismatch(Protocol.hex(exp), Protocol.hex(fp)).message)
            }
            certFingerprint = fp
        }

        override fun getAcceptedIssuers(): Array<X509Certificate> = arrayOf()
    }

    private fun connect(timeoutMs: Int): Channel {
        val ctx = SSLContext.getInstance("TLS")
        ctx.init(null, arrayOf(Pinner()), SecureRandom())
        val raw = Socket()
        raw.tcpNoDelay = true
        raw.receiveBufferSize = 1 shl 20
        raw.connect(InetSocketAddress(host, port), timeoutMs)
        val s = ctx.socketFactory.createSocket(raw, host, port, true) as SSLSocket
        s.enabledProtocols = s.supportedProtocols.filter { it == "TLSv1.3" || it == "TLSv1.2" }.toTypedArray()
        s.soTimeout = 0
        try {
            s.startHandshake()
        } catch (e: Exception) {
            runCatching { s.close() }
            val c = e.cause
            if (e.message?.contains("server identity changed") == true || c?.message?.contains("server identity changed") == true) {
                throw FingerprintMismatch(Protocol.hex(expectedFp ?: ByteArray(0)), "?")
            }
            throw e
        }
        synchronized(sockets) { sockets.add(s) }
        return Channel(s.inputStream.buffered(1 shl 16), s.outputStream.buffered(1 shl 14)) { s.close() }
    }

    override fun openControl() = connect(4000)
    override fun openVideo() = connect(4000)

    override fun close() {
        synchronized(sockets) {
            sockets.forEach { runCatching { it.close() } }
            sockets.clear()
        }
    }
}

/**
 * USB Android Open Accessory link. Both channels share one bulk pipe:
 * `[u8 channel][u32 length][bytes]`. Reads use 16 KiB buffers — the size the
 * accessory driver transfers per request.
 */
class UsbTransport(private val usb: UsbManager, val accessory: UsbAccessory) : Transport {
    private companion object {
        val EOF = ByteArray(0)
    }
    override val label = "USB"
    override val certFingerprint: ByteArray? = null

    private var pfd: ParcelFileDescriptor? = null
    private var out: FileOutputStream? = null
    private val queues = arrayOf(LinkedBlockingQueue<ByteArray>(), LinkedBlockingQueue<ByteArray>())
    @Volatile private var closed = false
    private var reader: Thread? = null

    private fun ensureOpen() {
        if (pfd != null && !closed) return
        closed = false
        queues.forEach { it.clear() }
        val fd = usb.openAccessory(accessory) ?: throw IOException("cannot open USB accessory (permission denied?)")
        pfd = fd
        out = FileOutputStream(fd.fileDescriptor)
        val inp = FileInputStream(fd.fileDescriptor)
        reader = Thread({
            val buf = ByteArray(16384)
            val acc = java.io.ByteArrayOutputStream()
            var pending = ByteArray(0)
            try {
                while (!closed) {
                    val n = inp.read(buf)
                    if (n < 0) break
                    if (n == 0) continue
                    acc.reset(); acc.write(pending); acc.write(buf, 0, n)
                    var data = acc.toByteArray()
                    var off = 0
                    while (data.size - off >= 5) {
                        val ch = data[off].toInt()
                        val len = ((data[off + 1].toInt() and 0xff) shl 24) or ((data[off + 2].toInt() and 0xff) shl 16) or
                            ((data[off + 3].toInt() and 0xff) shl 8) or (data[off + 4].toInt() and 0xff)
                        if (data.size - off - 5 < len) break
                        // zero-length payload = the server closed that channel
                        if (ch in 0..1) queues[ch].put(if (len == 0) EOF else data.copyOfRange(off + 5, off + 5 + len))
                        off += 5 + len
                    }
                    pending = data.copyOfRange(off, data.size)
                }
            } catch (_: Exception) {
            }
            closed = true
            queues.forEach { it.put(EOF) }
        }, "usb-reader").apply { isDaemon = true; start() }
    }

    private inner class ChanIn(private val ch: Int) : InputStream() {
        private var cur = ByteArray(0)
        private var pos = 0
        @Volatile var eof = false
        private fun fill(): Boolean {
            while (pos >= cur.size) {
                if (eof || (closed && queues[ch].isEmpty())) return false
                val next = queues[ch].poll(200, TimeUnit.MILLISECONDS) ?: continue
                if (next === EOF) { eof = true; return false }
                cur = next; pos = 0
            }
            return true
        }
        override fun close() { eof = true }
        override fun read(): Int = if (!fill()) -1 else (cur[pos++].toInt() and 0xff)
        override fun read(b: ByteArray, off: Int, len: Int): Int {
            if (len == 0) return 0
            if (!fill()) return -1
            val n = minOf(len, cur.size - pos)
            System.arraycopy(cur, pos, b, off, n)
            pos += n
            return n
        }
        override fun available(): Int = cur.size - pos
    }

    private inner class ChanOut(private val ch: Int) : OutputStream() {
        fun sendClose() {
            val o = out ?: return
            synchronized(o) { o.write(byteArrayOf(ch.toByte(), 0, 0, 0, 0)) }
        }
        private val buf = java.io.ByteArrayOutputStream()
        override fun write(b: Int) { buf.write(b) }
        override fun write(b: ByteArray, off: Int, len: Int) { buf.write(b, off, len) }
        override fun flush() {
            if (buf.size() == 0) return
            val data = buf.toByteArray(); buf.reset()
            val o = out ?: throw IOException("USB closed")
            val hdr = byteArrayOf(ch.toByte(), (data.size ushr 24).toByte(), (data.size ushr 16).toByte(), (data.size ushr 8).toByte(), data.size.toByte())
            synchronized(o) {
                if (closed) throw IOException("USB link closed")
                o.write(hdr + data)
            }
        }
    }

    private fun open(ch: Int): Channel {
        ensureOpen()
        queues[ch].clear() // drop anything left from a previous session
        val input = ChanIn(ch)
        val output = ChanOut(ch)
        return Channel(input, output) {
            input.close()
            runCatching { output.sendClose() }
        }
    }

    override fun openControl() = open(0)
    override fun openVideo() = open(1)

    override fun close() {
        closed = true
        runCatching { out?.close() }
        runCatching { pfd?.close() }
        pfd = null
        queues.forEach { it.offer(EOF) }
    }
}
