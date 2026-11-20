package app.dread.client

import android.app.Application
import android.hardware.usb.UsbAccessory

class DreadApp : Application() {
    lateinit var prefs: Prefs
        private set

    /** Accessory handed over by [app.dread.client.ui.UsbAttachActivity]. */
    @Volatile var pendingAccessory: UsbAccessory? = null

    override fun onCreate() {
        super.onCreate()
        instance = this
        prefs = Prefs(this)
    }

    companion object {
        lateinit var instance: DreadApp
            private set
    }
}
