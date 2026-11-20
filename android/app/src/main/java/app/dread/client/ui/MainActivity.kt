package app.dread.client.ui

import android.app.Activity
import android.app.AlertDialog
import android.content.Context
import android.content.Intent
import android.hardware.usb.UsbManager
import android.net.Uri
import android.os.Bundle
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import app.dread.client.DreadApp
import app.dread.client.LinkPref
import app.dread.client.R
import app.dread.client.net.DiscoveredServer
import app.dread.client.net.Discovery
import app.dread.client.net.Protocol
import app.dread.client.ui.Ui.dp

class MainActivity : Activity() {
    private val prefs get() = DreadApp.instance.prefs
    private lateinit var discovery: Discovery
    private lateinit var foundList: LinearLayout
    private lateinit var recentList: LinearLayout
    private lateinit var usbStatus: TextView
    private lateinit var methodRow: LinearLayout
    private lateinit var wifiSection: View
    private lateinit var usbSection: View

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        buildUi()
        discovery = Discovery(this) { showFound(it) }
        handleIntent(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleIntent(intent)
    }

    override fun onResume() {
        super.onResume()
        discovery.start()
        showRecent()
        refreshMethod()
    }

    override fun onPause() {
        super.onPause()
        discovery.stop()
    }

    private fun handleIntent(i: Intent?) {
        val uri = i?.data ?: return
        if (uri.scheme == "dread") connectFromQr(uri.toString())
    }

    private fun buildUi() {
        val root = Ui.vertical(this, dp(20))
        val header = Ui.horizontal(this)
        val titles = Ui.vertical(this)
        titles.addView(Ui.text(this, "DREAD", 28f, bold = true).apply { letterSpacing = 0.25f })
        titles.addView(Ui.caption(this, getString(R.string.tagline)))
        header.addView(titles, Ui.lp(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
        header.addView(Ui.icon(this, R.drawable.ic_settings, 26).apply {
            setPadding(dp(4), dp(4), dp(4), dp(4))
            layoutParams = LinearLayout.LayoutParams(dp(40), dp(40))
            background = Ui.pressable(context, Ui.BG, Ui.PANEL, Ui.BG)
            setOnClickListener { startActivity(Intent(this@MainActivity, SettingsActivity::class.java)) }
            contentDescription = "Settings"
        })
        root.addView(header)

        root.addView(Ui.sectionTitle(this, "Connection method"))
        methodRow = Ui.horizontal(this)
        root.addView(methodRow)

        wifiSection = Ui.vertical(this).apply {
            addView(Ui.button(context, "Scan QR code", primary = true) { startActivityForResult(Intent(context, ScanActivity::class.java), REQ_SCAN) }, Ui.lp(top = dp(16)))
            addView(Ui.sectionTitle(context, "On this network"))
            foundList = Ui.vertical(context)
            addView(foundList)
            foundList.addView(Ui.caption(context, "Searching…"))

            addView(Ui.sectionTitle(context, "Enter address"))
            val row = Ui.horizontal(context)
            val host = Ui.input(context, "Computer IP address", prefs.lastManualHost)
            val port = Ui.input(context, "Port", prefs.lastManualPort.toString(), numeric = true)
            row.addView(host, Ui.lp(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f, end = dp(8)))
            row.addView(port, Ui.lp(dp(96), ViewGroup.LayoutParams.WRAP_CONTENT))
            addView(row)
            addView(Ui.button(context, "Connect") {
                val h = host.text.toString().trim()
                val p = port.text.toString().toIntOrNull() ?: Protocol.DEFAULT_PORT
                if (h.isEmpty()) {
                    host.error = "Enter the address shown in DREAD on the computer"
                    return@button
                }
                prefs.lastManualHost = h
                prefs.lastManualPort = p
                connect(listOf(h), p, null, null, null)
            }, Ui.lp(top = dp(8)))
            addView(Ui.caption(context, "The computer shows its addresses on the Dashboard. First connections show a 6-digit code on both screens — approve it on the computer."), Ui.lp(top = dp(6)))
        }
        root.addView(wifiSection)

        usbSection = Ui.vertical(this).apply {
            addView(Ui.sectionTitle(context, "USB"))
            val p = Ui.panel(context)
            usbStatus = Ui.text(context, "", 14f)
            p.addView(usbStatus)
            p.addView(Ui.caption(context, "\n1. Start DREAD on the computer.\n2. Connect this device with a USB data cable.\n3. Android asks to open DREAD for \"DREAD Display\" — tap OK (tick \"Always\" to skip next time).\n\nUSB debugging is not needed. If nothing happens, try another cable/port or enable USB tethering and connect over the network instead."))
            addView(p)
        }
        root.addView(usbSection)

        root.addView(Ui.sectionTitle(this, "Recent"))
        recentList = Ui.vertical(this)
        root.addView(recentList)

        setContentView(ScrollView(this).apply {
            setBackgroundColor(Ui.BG)
            isFillViewport = true
            addView(root)
        })
    }

    private fun refreshMethod() {
        methodRow.removeAllViews()
        for (m in LinkPref.values()) {
            val sel = prefs.linkPref == m
            val b = TextView(this).apply {
                text = when (m) { LinkPref.AUTO -> "Automatic"; LinkPref.WIFI -> "Wi-Fi"; LinkPref.USB -> "USB" }
                gravity = Gravity.CENTER
                setTextColor(if (sel) Ui.BG else Ui.TEXT2)
                background = Ui.box(if (sel) Ui.LIGHT else Ui.BG2, Ui.BORDER, 0f, context)
                setPadding(0, dp(10), 0, dp(10))
                setOnClickListener { prefs.linkPref = m; refreshMethod() }
            }
            methodRow.addView(b, Ui.lp(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
        }
        wifiSection.visibility = if (prefs.linkPref == LinkPref.USB) View.GONE else View.VISIBLE
        usbSection.visibility = if (prefs.linkPref == LinkPref.WIFI) View.GONE else View.VISIBLE
        val usb = getSystemService(Context.USB_SERVICE) as UsbManager
        val acc = usb.accessoryList?.firstOrNull { it.manufacturer == "DREAD" }
        usbStatus.text = if (acc != null) "Connected to a DREAD computer over USB" else "No USB connection"
        if (acc != null) {
            usbStatus.setOnClickListener { UsbAttachActivity.launch(this, acc) }
            usbStatus.text = "Connected over USB — tap to open the display"
        }
    }

    private fun showFound(list: List<DiscoveredServer>) {
        foundList.removeAllViews()
        if (list.isEmpty()) {
            foundList.addView(Ui.caption(this, "No server found yet. Is DREAD running on the computer and on the same network?"))
            return
        }
        for (s in list) {
            val known = prefs.serverByFp(s.fp) != null
            val (row, _) = Ui.row(this, s.name, "${s.host}:${s.port}" + if (known) "  · paired" else "") {
                connect(listOf(s.host), s.port, s.fp.ifEmpty { null }, null, s.name)
            }
            foundList.addView(row)
            foundList.addView(Ui.divider(this))
        }
    }

    private fun showRecent() {
        recentList.removeAllViews()
        val servers = prefs.servers()
        if (servers.isEmpty()) {
            recentList.addView(Ui.caption(this, "No paired computers yet."))
            return
        }
        for (s in servers) {
            val (row, _) = Ui.row(this, s.name, s.hosts.firstOrNull()?.let { "$it:${s.port}" } ?: "USB") {
                if (s.hosts.isEmpty()) {
                    prefs.linkPref = LinkPref.USB
                    refreshMethod()
                } else connect(s.hosts, s.port, s.fp, null, s.name)
            }
            row.setOnLongClickListener {
                AlertDialog.Builder(this).setTitle("Forget ${s.name}?")
                    .setMessage("You will need to pair again to connect to this computer.")
                    .setPositiveButton("Forget") { _, _ -> prefs.forgetServer(s.id); showRecent() }
                    .setNegativeButton("Cancel", null).show()
                true
            }
            recentList.addView(row)
            recentList.addView(Ui.divider(this))
        }
        recentList.addView(Ui.caption(this, "Long-press a computer to forget it."), Ui.lp(top = dp(6)))
    }

    private fun connect(hosts: List<String>, port: Int, fp: String?, secret: String?, name: String?) {
        startActivity(Intent(this, DisplayActivity::class.java).apply {
            putExtra(DisplayActivity.EXTRA_HOSTS, hosts.toTypedArray())
            putExtra(DisplayActivity.EXTRA_PORT, port)
            putExtra(DisplayActivity.EXTRA_FP, fp)
            putExtra(DisplayActivity.EXTRA_SECRET, secret)
            putExtra(DisplayActivity.EXTRA_NAME, name)
        })
    }

    private fun connectFromQr(text: String) {
        val u = Uri.parse(text)
        if (u.scheme != "dread" || u.host != "pair") {
            AlertDialog.Builder(this).setTitle("Not a DREAD code").setMessage("Scan the QR code shown on the computer's Connection page.").setPositiveButton("OK", null).show()
            return
        }
        val hosts = u.getQueryParameter("h")?.split(',')?.filter { it.isNotBlank() } ?: emptyList()
        val port = u.getQueryParameter("p")?.toIntOrNull() ?: Protocol.DEFAULT_PORT
        val fp = u.getQueryParameter("fp")
        val secret = u.getQueryParameter("s")
        if (hosts.isEmpty() || fp.isNullOrEmpty()) {
            AlertDialog.Builder(this).setTitle("Incomplete code").setMessage("The QR code does not contain a network address. Check that the computer is connected to a network.").setPositiveButton("OK", null).show()
            return
        }
        connect(hosts, port, fp, secret, u.getQueryParameter("n"))
    }

    @Deprecated("framework API")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == REQ_SCAN && resultCode == RESULT_OK) {
            data?.getStringExtra(ScanActivity.EXTRA_TEXT)?.let { connectFromQr(it) }
        }
    }

    companion object {
        private const val REQ_SCAN = 1
    }
}
