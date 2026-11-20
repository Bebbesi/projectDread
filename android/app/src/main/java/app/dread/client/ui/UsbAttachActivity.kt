package app.dread.client.ui

import android.app.Activity
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.hardware.usb.UsbAccessory
import android.hardware.usb.UsbManager
import android.os.Build
import android.os.Bundle
import android.widget.Toast
import app.dread.client.DreadApp
import app.dread.client.LinkPref

/**
 * Entry point when the computer switches this device into accessory mode
 * (Android shows "Open DREAD for DREAD Display?"). Hands the accessory to the
 * display screen and finishes immediately.
 */
class UsbAttachActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        @Suppress("DEPRECATION")
        val acc: UsbAccessory? = if (Build.VERSION.SDK_INT >= 33) intent.getParcelableExtra(UsbManager.EXTRA_ACCESSORY, UsbAccessory::class.java) else intent.getParcelableExtra(UsbManager.EXTRA_ACCESSORY)
        if (acc != null && DreadApp.instance.prefs.linkPref != LinkPref.WIFI) {
            open(this, acc)
        } else if (acc != null) {
            Toast.makeText(this, "DREAD is set to Wi-Fi only; USB link ignored", Toast.LENGTH_LONG).show()
        }
        finish()
    }

    companion object {
        private const val ACTION_PERMISSION = "app.dread.client.USB_PERMISSION"

        fun open(ctx: Context, acc: UsbAccessory) {
            DreadApp.instance.pendingAccessory = acc
            ctx.startActivity(Intent(ctx, DisplayActivity::class.java).apply {
                putExtra(DisplayActivity.EXTRA_USB, true)
                addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            })
        }

        // the broadcast is addressed to our own package (setPackage below)
        @android.annotation.SuppressLint("UnspecifiedRegisterReceiverFlag")
        private fun register(ctx: Context, r: BroadcastReceiver, f: IntentFilter) {
            if (Build.VERSION.SDK_INT >= 33) ctx.applicationContext.registerReceiver(r, f, Context.RECEIVER_NOT_EXPORTED)
            else ctx.applicationContext.registerReceiver(r, f)
        }

        /** Opens an already-attached accessory, asking for permission if needed. */
        fun launch(ctx: Context, acc: UsbAccessory) {
            val usb = ctx.getSystemService(Context.USB_SERVICE) as UsbManager
            if (usb.hasPermission(acc)) {
                open(ctx, acc)
                return
            }
            val receiver = object : BroadcastReceiver() {
                override fun onReceive(c: Context, i: Intent) {
                    ctx.applicationContext.unregisterReceiver(this)
                    if (i.getBooleanExtra(UsbManager.EXTRA_PERMISSION_GRANTED, false)) open(ctx, acc)
                    else Toast.makeText(ctx, "USB permission denied", Toast.LENGTH_LONG).show()
                }
            }
            register(ctx, receiver, IntentFilter(ACTION_PERMISSION))
            val pi = PendingIntent.getBroadcast(ctx, 0, Intent(ACTION_PERMISSION).setPackage(ctx.packageName), PendingIntent.FLAG_MUTABLE)
            usb.requestPermission(acc, pi)
        }
    }
}
