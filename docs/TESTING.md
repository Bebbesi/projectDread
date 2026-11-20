# Verification log

What has been run, on what, with what result. Anything not listed here has
**not** been verified.

## Environment

* Server: Arch Linux, kernel 7.3, Hyprland 0.57.2 (Lua config), Intel UHD 620
  (Kaby Lake-R), FFmpeg 9.0.2, Rust 1.100.
* Client: Samsung Galaxy S23 (SCG19), Android 16 (API 36), Snapdragon,
  1080 × 2340 @ 480 dpi. No stylus.
* Windows: none available.

## Automated

| Check | Command | Result |
|---|---|---|
| Protocol encode/decode, SAS code | `cargo test -p dread-proto` | pass (3 tests) |
| AU splitting (H.264/AV1), layout placement, abs mapping, private-address filter | `cargo test` | pass |
| Android lint (errors) | `./gradlew lintDebug` | 0 errors |

## Server on Hyprland

| Feature | How | Result |
|---|---|---|
| Virtual output creation/removal | `hyprctl output create headless`, mode via `hl.monitor` | `DREAD-1` appears at the requested mode/position; removed on stop and after the linger time |
| Existing monitors untouched | before/after `hyprctl monitors` | main output keeps position and active workspace (workspace guard added after a bug was found where Hyprland moved the focused workspace onto a re-created `DREAD-1`) |
| Capture → encode | `examples/smoke.rs` | valid H.264 (`ffprobe`), damage-driven (no frames when idle) |
| Encode latency (capture ready → access unit), 1280×800 | smoke test, release | VA-API 11–20 ms, x264 15–20 ms per frame |
| Encoder probing | startup | h264_vaapi, hevc_vaapi, libx264, libx265 found, each verified up to 3840×2160 |
| TLS + QR pairing + streaming | `dread-testclient --qr …` | paired, 54 frames in 6 s while switching workspaces, RTT 1.9 ms (loopback) |
| Absolute pointer mapping | test client probes + `hyprctl cursorpos` | exact (center → 2560,400; corner → 3199,799 on a 1280×800 output at x=1920) |
| Pen mapping | test client probes | exact once libinput tablet smoothing settles (quirk file provided to disable smoothing) |
| uinput devices | `hyprctl devices` | `dread-touchscreen`, `dread-pen`, `dread-pointer`, `dread-mouse`, `dread-keyboard` present; removed on exit |
| Input method / virtual keyboard registration | startup log | `virtual_keyboard=true input_method=true` |
| Graceful shutdown | SIGTERM | display removed, process exits |
| Control panel | screenshots of all six pages on a hidden headless output | renders; layout bugs found and fixed (column overflow, missing glyph, justified log text) |

## Real phone

| Feature | Result |
|---|---|
| Install signed release APK (R8-minified) | OK |
| QR deep link pairing over TLS (via `adb reverse`, phone and laptop were on different Wi-Fi subnets) | paired, display 1080×2340 portrait created automatically |
| Hardware decode | `c2.qti.avc.decoder.low_latency`, ~7.7 ms decode |
| RTT | 3.0 ms (adb-reverse USB tunnel) |
| USB accessory mode, no pairing repeat | server switched the phone to AOA (`18d1:2d01`), DREAD opened, streamed over USB using the stored token |
| Rotation | phone rotated to landscape → display reconfigured to 2340×1080 |
| Density scaling | automatic scale 1.75 requested, Hyprland applied 1.8 |
| Immersive fullscreen, toolbar, auto-hide, statistics overlay | OK (status bar initially not hidden → fixed by applying in `onWindowFocusChanged`) |
| Touch → mouse mode | taps map pixel-exactly onto `DREAD-1` |
| Auto-reconnect after server restart | OK |

## Not verified yet (needs doing)

* Apps receiving multitouch gestures, pen pressure/tilt/eraser, keys and
  Unicode text — open a text editor and a drawing app (e.g. Xournal++, Krita)
  on `DREAD-1` and use the device; check pressure in the app.
* Automatic Android keyboard on text-field focus (input-method-v2).
* Real Wi-Fi between two machines on the same subnet (mDNS/UDP discovery,
  code-comparison pairing with approval in the control panel).
* Adaptive bitrate under real congestion.
* HEVC/AV1 streams on the device.
* Sway backend; any Windows code; the AppImage and Windows installer builds.
