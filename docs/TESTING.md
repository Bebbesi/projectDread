# Verification log

What has been run, on what, with what result. Anything not listed here has
**not** been verified.

## Environment

* Server: Arch Linux, kernel 7.3, Hyprland 0.57.2 (Lua config), Intel UHD 620
  (Kaby Lake-R), FFmpeg 9.0.2, Rust 1.100.
* Client: Samsung Galaxy S23 (SCG19), Android 16 (API 36), Snapdragon,
  1080 × 2340 @ 480 dpi. No stylus.
* Windows: Windows 11 Pro (build 26200), NVIDIA GeForce RTX 4060 Ti + Intel
  UHD 770, two physical monitors, Rust stable (MSVC), Inno Setup 6, FFmpeg
  essentials build (gyan.dev). Client: the Galaxy S23 above, paired over TLS (transport not recorded).

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

## Server on Windows

| Feature | How | Result |
|---|---|---|
| Build | `cargo build --release -p dread-app`, `build-windows.ps1` | builds after fixing imports in `input/windows.rs` and flag checks in `windows_vdd.rs`; installer `DREAD-Setup-1.0.0.exe` produced |
| Driver install | installer / `install-driver.ps1` | device `Root\MttVDD` bound to the driver, monitor *VDD by MTT* present. Bugs fixed: nefconw (GUI program) was not awaited, so the driver was never installed; 220-mode settings file made Windows reject the monitor |
| Encoder probing | startup | h264_nvenc, hevc_nvenc, av1_nvenc, libx264, libx265 found, each up to 3840×2160 |
| Attach / reconfigure / detach | `examples/win_attach.rs` | 1920×1200 attached right of the main monitor, re-applied, detached; also from the detached state after a driver restart |
| Extend mode with the phone | real use | works (after the re-detection fix for the changing `\\.\DISPLAYn` name) |
| Duplicate mode with the phone | real use | works |
| Touch position | `examples/win_touch.rs` + real use on the phone | touches land pixel-exactly on the DREAD display (e.g. 10 % → 2112,120 on a display at x=1920). Bug fixed: a synthetic touch device (`CreateSyntheticPointerDevice`) is bound to the primary monitor (10 % landed at 192,120), so touch now uses `InjectTouchInput`. Two-finger down/move/up frames accepted |
| Mouse (absolute) | `examples/win_touch.rs` | centre of the DREAD display → cursor 2880,600 (exact) |

## Not verified yet (needs doing)

* Windows: pen (still a synthetic pointer device — likely bound to the primary monitor like touch was; no stylus device to test), keys and text injected into apps, gestures received by apps; streaming
  over USB; AMF / Quick Sync encoders; uninstaller. One unexplained exit of
  `dread.exe` was seen once (no panic message, no Windows crash report) and
  could not be reproduced.

* Apps receiving multitouch gestures, pen pressure/tilt/eraser, keys and
  Unicode text — open a text editor and a drawing app (e.g. Xournal++, Krita)
  on `DREAD-1` and use the device; check pressure in the app.
* Automatic Android keyboard on text-field focus (input-method-v2).
* Real Wi-Fi between two machines on the same subnet (mDNS/UDP discovery,
  code-comparison pairing with approval in the control panel).
* Adaptive bitrate under real congestion.
* HEVC/AV1 streams on the device.
* Sway backend; the AppImage build.
