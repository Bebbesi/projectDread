# Troubleshooting

Start with *Advanced → Diagnostics → Copy report* on the computer and the
statistics overlay (toolbar → bars icon) on the device.

## The control panel says "unsupported desktop session"

DREAD found neither Hyprland nor Sway. GNOME, KDE and X11 are not supported
yet ([PLATFORMS.md](PLATFORMS.md)). Duplicate mode on those desktops is not
available either in this release.

## "cannot open /dev/uinput"

Input injection needs `/dev/uinput`: `sudo packaging/linux/install-system-files.sh`,
then log out and in. Check with `ls -l /dev/uinput` (your user should have an
ACL entry: `getfacl /dev/uinput`).

## No working encoder

*Advanced → Drivers and encoders* lists what was found. Install FFmpeg; for
GPU encoding install the VA-API driver (`vainfo` should list
`VAEntrypointEncSlice` for H.264) or the NVIDIA driver. Set a different VA-API
device under *Advanced* if you have several GPUs (`/dev/dri/renderD129`).
*Probe again* re-tests.

## The device does not find the computer

* Same network/subnet? Guest Wi-Fi often isolates clients.
* Firewall: TCP 47800 and UDP 47801 must be allowed.
* Type the address shown on the Dashboard instead of relying on discovery.
* Connections from internet-routable addresses are refused unless
  *Connection → Public addresses* is enabled (only behind your own VPN).

## "server identity changed"

The computer's certificate differs from the one the device paired with (the
config directory was reset or another computer now has that address). On
Android long-press the computer under *Recent* → *Forget*, then pair again.

## Pairing code never appears on the computer

Check the computer's *Dashboard*: requests appear as a banner on every page
for two minutes. Pairing needs the control panel (or use the QR code, which
needs no approval).

## Picture is blocky or stutters

* Lower the resolution (*Display*) or raise the bitrate (*Streaming*).
* Keep *Adaptive bitrate* on for Wi-Fi; 5 GHz Wi-Fi or USB is much better
  than 2.4 GHz.
* *Frames in flight* = 3 smooths jittery Wi-Fi at the cost of ~1 frame latency.
* The statistics overlay shows bitrate, RTT, encode/decode times and loss.

## Touch or pen lands in the wrong place

Touch and pen are bound to the DREAD output by the compositor. If you edited
monitor rules by hand, make sure nothing else maps `dread-touchscreen` /
`dread-pen`. Mouse mode maps through the desktop layout; restart streaming
after rearranging monitors.

## Pen feels laggy on Linux

Install the libinput quirk (`install-system-files.sh` does) to disable
libinput's tablet smoothing for the DREAD Pen.

## The Android keyboard does not open automatically

Automatic opening uses the Wayland input-method protocol, which only one
program can own; if fcitx5 or ibus runs, DREAD falls back to the toolbar
keyboard button (text still works). Also check *Input → Open the Android
keyboard…* on the computer and *Settings → Virtual keyboard* on the device.

## Windows: Extended mode unavailable

The Virtual Display Driver is not installed: re-run the installer with the
driver option, or run `driver-tools\install-driver.ps1` as administrator.
Duplicate mode works without it.

## Windows got a monitor but the picture is black

Desktop Duplication captures on the GPU the monitor is attached to. On
hybrid-graphics laptops set the driver's GPU (`<gpu><friendlyname>` in
`C:\VirtualDisplayDriver\vdd_settings.xml`) to the GPU FFmpeg uses.
