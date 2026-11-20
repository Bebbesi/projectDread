# DREAD

**Your screen. Expanded.**

DREAD turns an Android phone or tablet into a real additional monitor for a
Linux or Windows computer — over Wi-Fi or a USB cable — with multitouch,
pressure-sensitive stylus and keyboard input flowing back to the computer.

It is not remote desktop: in *Extend* mode the computer gains a genuine new
output that its display manager owns (you drag windows onto it); in
*Duplicate* mode the device shows an existing monitor.

```
Computer (DREAD Server)                         Android (DREAD Client)
virtual output → capture → H.264/HEVC/AV1  ──►  hardware decode → fullscreen
uinput / Windows pointer injection  ◄──  touch · pen · mouse · keyboard
                 TLS 1.3 over Wi-Fi/LAN  or  USB accessory mode
```

## Status at a glance

| | Linux — Hyprland | Linux — Sway | Linux — GNOME/KDE/X11 | Windows 10/11 | Android client |
|---|---|---|---|---|---|
| Extended display | **verified** | implemented, untested | not implemented | implemented, **not compiled/tested** | — |
| Duplicate display | **verified** | implemented, untested | not implemented | implemented, not compiled/tested | — |
| Capture + hardware encode | **verified** (VA-API, x264) | implemented, untested | — | implemented (ddagrab + NVENC/AMF/QSV), not tested | **verified** (Qualcomm low-latency decoder) |
| Wi-Fi/LAN, pairing, TLS | **verified** | same code | — | same code | **verified** on Android 16 |
| USB (no USB debugging) | **verified** (AOA) | same code | — | libusb code compiles only on Linux so far | **verified** |
| Mouse / pointer input | **verified** (pixel-exact) | implemented, untested | — | implemented, not tested | **verified** (touch → mouse mode) |
| Multitouch, pen, keys, Unicode text | devices created and events injected; pen position verified; delivery into apps **not yet verified**¹ | implemented, untested | — | implemented, not tested | implemented; stylus untested (no stylus device) |
| Packaging | release binary built; AppImage script ready² | | | installer script ready, not built | **signed APK built** |

¹ Verifying that apps receive touch, pressure, keys and text needs a test
window on the DREAD display and was not done on the development machine (it
was the developer's live desktop). Stylus pressure/tilt also needs a device
with a stylus. See [docs/TESTING.md](docs/TESTING.md) for the exact checks.
² `appimagetool` was not available on the build machine — see
[docs/BUILD.md](docs/BUILD.md).

Details per feature and platform: [docs/PLATFORMS.md](docs/PLATFORMS.md) and
the verification log [docs/TESTING.md](docs/TESTING.md).

## Quick start (Linux, Hyprland)

1. Install FFmpeg (with VA-API for Intel/AMD GPUs) and libusb.
2. Once, give your user access to `/dev/uinput` and Android USB devices:
   `sudo packaging/linux/install-system-files.sh` (or `DREAD.AppImage --install-system-files`).
3. Run `dread` (the control panel opens).
4. On Android install `DREAD.apk`, open it, tap **Scan QR code** and scan the
   code on the computer's **Connection** page — or plug in a USB cable.

The Android device becomes `DREAD-1` to the right of your main monitor.
Full instructions: [docs/INSTALL.md](docs/INSTALL.md).

## Repository layout

```
server/                 Rust workspace (DREAD Server)
  crates/dread-proto      wire protocol (framing, messages, input/video payloads)
  crates/dread-platform   display backends, capture, encoder, input injection
  crates/dread-core       engine: config, TLS & pairing, sessions, pipeline, USB, discovery
  crates/dread-app        `dread` executable: control panel (egui) + headless mode
  crates/dread-testclient headless client used to verify servers end to end
android/                Kotlin client (Gradle)
packaging/linux         udev rules, libinput quirk, desktop entry, AppImage builder
packaging/windows       driver settings, driver install scripts, Inno Setup installer
docs/                   architecture, protocol, platforms, build, install, USB, troubleshooting, testing
dist/                   build outputs (DREAD.apk, dread-linux-x86_64)
```

## Documentation

* [Architecture](docs/ARCHITECTURE.md) · [Wire protocol](docs/PROTOCOL.md)
* [Install & uninstall](docs/INSTALL.md) · [Build from source](docs/BUILD.md)
* [Platform support and limitations](docs/PLATFORMS.md) · [USB](docs/USB.md)
* [Troubleshooting](docs/TROUBLESHOOTING.md) · [Verification log](docs/TESTING.md)

## Third-party components

FFmpeg (LGPL/GPL, run as a separate process), Virtual Display Driver (MIT,
Windows only), nefcon (BSD-3-Clause, Windows installer only), ZXing (Apache-2.0,
Android), and the Rust crates listed in `server/Cargo.lock`.

## License

Not chosen yet — the project owner should add a `LICENSE` file. Note that a
Windows installer bundling a GPL build of FFmpeg must comply with the GPL.
