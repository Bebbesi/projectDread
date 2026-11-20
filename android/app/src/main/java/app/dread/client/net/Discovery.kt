package app.dread.client.net

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import org.json.JSONObject
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetAddress
import java.net.NetworkInterface
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors

data class DiscoveredServer(val id: String, val name: String, val host: String, val port: Int, val fp: String)

/**
 * Finds DREAD servers on the local network with mDNS (`_dread._tcp`) and a
 * UDP broadcast fallback (`DREAD?` → port 47801) for networks that filter
 * multicast. Results are delivered on the main thread.
 */
class Discovery(private val ctx: Context, private val onChange: (List<DiscoveredServer>) -> Unit) {
    private val main = Handler(Looper.getMainLooper())
    private val found = ConcurrentHashMap<String, DiscoveredServer>()
    private val nsd = ctx.getSystemService(Context.NSD_SERVICE) as NsdManager
    private var lock: WifiManager.MulticastLock? = null
    private var listener: NsdManager.DiscoveryListener? = null
    @Volatile private var running = false
    private val resolver = Executors.newSingleThreadExecutor()

    fun start() {
        if (running) return
        running = true
        val wifi = ctx.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
        lock = wifi.createMulticastLock("dread-discovery").apply { setReferenceCounted(false); acquire() }
        startNsd()
        Thread({ udpLoop() }, "dread-udp-discovery").apply { isDaemon = true; start() }
    }

    fun stop() {
        running = false
        listener?.let { runCatching { nsd.stopServiceDiscovery(it) } }
        listener = null
        runCatching { lock?.release() }
    }

    private fun publish(s: DiscoveredServer) {
        val key = s.id.ifEmpty { s.host + ":" + s.port }
        val prev = found.put(key, s)
        if (prev != s) main.post { onChange(found.values.sortedBy { it.name }) }
    }

    private fun startNsd() {
        val l = object : NsdManager.DiscoveryListener {
            override fun onDiscoveryStarted(serviceType: String) {}
            override fun onDiscoveryStopped(serviceType: String) {}
            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) { Log.w(TAG, "mDNS start failed $errorCode") }
            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) {}
            override fun onServiceLost(info: NsdServiceInfo) {}
            override fun onServiceFound(info: NsdServiceInfo) {
                // NsdManager resolves one service at a time; serialise
                resolver.execute { resolve(info) }
            }
        }
        listener = l
        runCatching { nsd.discoverServices(Protocol.MDNS_TYPE, NsdManager.PROTOCOL_DNS_SD, l) }
    }

    @Suppress("DEPRECATION")
    private fun resolve(info: NsdServiceInfo) {
        val done = java.util.concurrent.CountDownLatch(1)
        nsd.resolveService(info, object : NsdManager.ResolveListener {
            override fun onResolveFailed(si: NsdServiceInfo, errorCode: Int) { done.countDown() }
            override fun onServiceResolved(si: NsdServiceInfo) {
                val host = if (Build.VERSION.SDK_INT >= 34) si.hostAddresses.firstOrNull { it is java.net.Inet4Address } ?: si.hostAddresses.firstOrNull() else si.host
                val attr = si.attributes
                fun a(k: String) = attr[k]?.let { String(it, Charsets.UTF_8) } ?: ""
                if (host != null) {
                    publish(DiscoveredServer(a("id"), a("name").ifEmpty { si.serviceName }, host.hostAddress ?: "", si.port, a("fp")))
                }
                done.countDown()
            }
        })
        done.await(5, java.util.concurrent.TimeUnit.SECONDS)
    }

    private fun broadcastAddresses(): List<InetAddress> {
        val out = mutableListOf<InetAddress>(InetAddress.getByName("255.255.255.255"))
        runCatching {
            for (ni in NetworkInterface.getNetworkInterfaces()) {
                if (!ni.isUp || ni.isLoopback) continue
                for (ia in ni.interfaceAddresses) ia.broadcast?.let { out.add(it) }
            }
        }
        return out.distinct()
    }

    private fun udpLoop() {
        val sock = runCatching { DatagramSocket().apply { broadcast = true; soTimeout = 400 } }.getOrNull() ?: return
        val buf = ByteArray(2048)
        try {
            while (running) {
                for (a in broadcastAddresses()) {
                    runCatching { sock.send(DatagramPacket("DREAD?".toByteArray(), 6, a, Protocol.DISCOVERY_PORT)) }
                }
                val until = System.currentTimeMillis() + 2000
                while (running && System.currentTimeMillis() < until) {
                    val p = DatagramPacket(buf, buf.size)
                    try {
                        sock.receive(p)
                    } catch (_: java.net.SocketTimeoutException) {
                        continue
                    }
                    runCatching {
                        val o = JSONObject(String(p.data, 0, p.length, Charsets.UTF_8))
                        if (o.optString("t") == "announce") {
                            publish(DiscoveredServer(o.optString("id"), o.optString("name"), p.address.hostAddress ?: "", o.optInt("port", Protocol.DEFAULT_PORT), o.optString("fp")))
                        }
                    }
                }
            }
        } finally {
            sock.close()
        }
    }

    companion object {
        private const val TAG = "DreadDiscovery"
    }
}
