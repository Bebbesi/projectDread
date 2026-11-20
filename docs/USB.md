# USB connection

DREAD uses the **Android Open Accessory (AOA 2)** protocol: the computer is
the USB host and asks the phone to switch into accessory mode. This is the
mechanism car head units use — it needs **no developer options and no USB
debugging**.

1. DREAD on the computer polls USB devices. A device that looks like a
   phone/tablet (MTP/PTP or vendor-specific Android interface) is sent the
   AOA identification strings `DREAD` / `DREAD Display` and the start command.
2. The phone re-enumerates as `18d1:2d00` (or `2d01` when ADB is also on).
3. Android finds the DREAD app through its accessory filter and asks the user
   to open it (once, if *Always* is ticked).
4. Both logical channels (control, video) share one bulk pipe, framed as
   `[channel][length][data]`; a zero-length frame closes a channel, so the app
   can reconnect without re-plugging.

USB sessions still authenticate with the device token from pairing. If the
device was never paired, a code-comparison pairing runs over the cable.

## Linux permissions

libusb must be able to open the device node. Install the rules once:

```bash
sudo packaging/linux/install-system-files.sh
```

`70-dread-usb.rules` grants the logged-in user (`uaccess`) access to devices
exposing MTP/PTP or Android vendor interfaces, and to accessory-mode devices.
The control panel's *Connection → USB* line shows "No permission to open USB
device …" when this is missing.

## Windows

AOA needs a WinUSB-compatible driver bound to the phone, which Windows does
not provide for arbitrary phones. On Windows use **USB tethering** instead:

1. Connect the cable, enable *Settings → Connections → Mobile hotspot and
   tethering → USB tethering* on the phone.
2. A new network adapter appears on the PC; DREAD is discovered over it like
   Wi-Fi (or enter the PC's address shown on the Dashboard).

The same tethering fallback works on Linux.

## Troubleshooting

* Nothing happens when plugging in: use a data cable (not charge-only); try
  another port; check the USB line in the control panel.
* The phone was switched but DREAD did not open: open DREAD manually — the
  main screen shows "Connected over USB — tap to open the display".
* After unplugging, the phone leaves accessory mode by itself.
