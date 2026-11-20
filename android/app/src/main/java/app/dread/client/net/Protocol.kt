package app.dread.client.net

import org.json.JSONObject
import java.io.DataInputStream
import java.io.EOFException
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream
import java.nio.ByteBuffer
import java.security.MessageDigest

/** DREAD wire protocol v1 (see docs/PROTOCOL.md). All integers big-endian. */
object Protocol {
    const val VERSION = 1
    const val DEFAULT_PORT = 47800
    const val DISCOVERY_PORT = 47801
    const val MDNS_TYPE = "_dread._tcp"
    const val MAX_FRAME = 16 * 1024 * 1024

    const val KIND_JSON: Byte = 0x01
    const val KIND_INPUT: Byte = 0x02
    const val KIND_VIDEO: Byte = 0x03

    const val VIDEO_FLAG_KEY = 1
    const val VIDEO_FLAG_CONFIG = 2

    const val IN_TOUCH: Byte = 0x01
    const val IN_PEN: Byte = 0x02
    const val IN_MOUSE_ABS: Byte = 0x03
    const val IN_MOUSE_REL: Byte = 0x04
    const val IN_SCROLL: Byte = 0x05
    const val IN_KEY: Byte = 0x06
    const val IN_TEXT: Byte = 0x07

    const val TOUCH_DOWN = 0
    const val TOUCH_MOVE = 1
    const val TOUCH_UP = 2
    const val TOUCH_CANCEL = 3

    const val PEN_IN_RANGE = 1
    const val PEN_CONTACT = 2
    const val PEN_BUTTON1 = 4
    const val PEN_BUTTON2 = 8

    const val MOUSE_LEFT = 1
    const val MOUSE_RIGHT = 2
    const val MOUSE_MIDDLE = 4

    fun frame(kind: Byte, payload: ByteArray): ByteArray {
        val b = ByteBuffer.allocate(5 + payload.size)
        b.putInt(payload.size + 1)
        b.put(kind)
        b.put(payload)
        return b.array()
    }

    fun json(o: JSONObject): ByteArray = frame(KIND_JSON, o.toString().toByteArray(Charsets.UTF_8))

    fun msg(type: String, vararg kv: Pair<String, Any?>): ByteArray {
        val o = JSONObject()
        o.put("t", type)
        for ((k, v) in kv) o.put(k, v ?: JSONObject.NULL)
        return json(o)
    }

    class Frame(val kind: Byte, val payload: ByteArray)

    /** Blocking read of one frame. */
    fun read(input: DataInputStream): Frame {
        val len = try {
            input.readInt()
        } catch (e: EOFException) {
            throw IOException("connection closed")
        }
        if (len <= 0 || len > MAX_FRAME) throw IOException("invalid frame length $len")
        val kind = input.readByte()
        val payload = ByteArray(len - 1)
        input.readFully(payload)
        return Frame(kind, payload)
    }

    fun readJson(input: DataInputStream): JSONObject {
        val f = read(input)
        if (f.kind != KIND_JSON) throw IOException("expected JSON message")
        return JSONObject(String(f.payload, Charsets.UTF_8))
    }

    fun write(out: OutputStream, bytes: ByteArray) {
        synchronized(out) {
            out.write(bytes)
            out.flush()
        }
    }

    // ------------------------------------------------------------ input

    fun clamp16(v: Float): Int = (v.coerceIn(0f, 1f) * 65535f + 0.5f).toInt()

    class Contact(val id: Int, val x: Float, val y: Float, val pressure: Float, val major: Float)

    fun touch(action: Int, changed: Int, contacts: List<Contact>): ByteArray {
        val b = ByteBuffer.allocate(4 + contacts.size * 9)
        b.put(IN_TOUCH); b.put(action.toByte()); b.put(changed.toByte()); b.put(contacts.size.toByte())
        for (c in contacts) {
            b.put(c.id.toByte())
            b.putShort(clamp16(c.x).toShort()); b.putShort(clamp16(c.y).toShort())
            b.putShort(clamp16(c.pressure).toShort()); b.putShort(clamp16(c.major).toShort())
        }
        return frame(KIND_INPUT, b.array())
    }

    fun pen(eraser: Boolean, flags: Int, x: Float, y: Float, pressure: Float, tiltXDeg: Float, tiltYDeg: Float): ByteArray {
        val b = ByteBuffer.allocate(13)
        b.put(IN_PEN); b.put(if (eraser) 1 else 0); b.put(flags.toByte())
        b.putShort(clamp16(x).toShort()); b.putShort(clamp16(y).toShort()); b.putShort(clamp16(pressure).toShort())
        b.putShort((tiltXDeg * 100f).toInt().coerceIn(-9000, 9000).toShort())
        b.putShort((tiltYDeg * 100f).toInt().coerceIn(-9000, 9000).toShort())
        return frame(KIND_INPUT, b.array())
    }

    fun mouseAbs(x: Float, y: Float, buttons: Int): ByteArray {
        val b = ByteBuffer.allocate(6)
        b.put(IN_MOUSE_ABS); b.putShort(clamp16(x).toShort()); b.putShort(clamp16(y).toShort()); b.put(buttons.toByte())
        return frame(KIND_INPUT, b.array())
    }

    fun mouseRel(dx: Int, dy: Int, buttons: Int): ByteArray {
        val b = ByteBuffer.allocate(6)
        b.put(IN_MOUSE_REL)
        b.putShort(dx.coerceIn(-32768, 32767).toShort()); b.putShort(dy.coerceIn(-32768, 32767).toShort())
        b.put(buttons.toByte())
        return frame(KIND_INPUT, b.array())
    }

    /** Units of 1/120 notch; positive = down / right. */
    fun scroll(vertical: Int, horizontal: Int): ByteArray {
        val b = ByteBuffer.allocate(5)
        b.put(IN_SCROLL)
        b.putShort(vertical.coerceIn(-32768, 32767).toShort()); b.putShort(horizontal.coerceIn(-32768, 32767).toShort())
        return frame(KIND_INPUT, b.array())
    }

    fun key(evdevCode: Int, down: Boolean): ByteArray {
        val b = ByteBuffer.allocate(4)
        b.put(IN_KEY); b.putShort(evdevCode.toShort()); b.put(if (down) 1 else 0)
        return frame(KIND_INPUT, b.array())
    }

    fun text(s: String): ByteArray {
        val t = s.toByteArray(Charsets.UTF_8)
        val b = ByteBuffer.allocate(1 + t.size)
        b.put(IN_TEXT); b.put(t)
        return frame(KIND_INPUT, b.array())
    }

    // ------------------------------------------------------------ pairing

    fun sha256(vararg parts: ByteArray): ByteArray {
        val d = MessageDigest.getInstance("SHA-256")
        for (p in parts) d.update(p)
        return d.digest()
    }

    /** Six-digit code both screens show during code-comparison pairing. */
    fun pairingCode(certFp: ByteArray, nc: ByteArray, ns: ByteArray): String {
        val d = sha256("DREAD-SAS-v1".toByteArray(), certFp, nc, ns)
        val v = (ByteBuffer.wrap(d, 0, 4).int.toLong() and 0xffffffffL) % 1_000_000L
        return "%06d".format(v)
    }

    fun hex(b: ByteArray): String = b.joinToString("") { "%02x".format(it) }

    fun unhex(s: String): ByteArray {
        require(s.length % 2 == 0) { "bad hex" }
        return ByteArray(s.length / 2) { s.substring(it * 2, it * 2 + 2).toInt(16).toByte() }
    }
}
