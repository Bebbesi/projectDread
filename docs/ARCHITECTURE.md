# DREAD — Architecture

DREAD turns an Android phone or tablet into a real additional monitor for a
Windows or Linux computer. The computer runs **DREAD Server**; the Android
device runs **DREAD Client**.

```
 ┌──────────────────────────── DREAD Server (Rust) ─────────────────────────────┐
 │                                                                              │
 │  display ──► capture ──► encode ──► session ──► transport (TLS/TCP | USB)    │──► Android
 │  (virtual     (screen-    (ffmpeg:    (pairing,    channels: control+input,  │
 │   output)      copy)       VAAPI/     stats, ABR)  video                     │◄── input
 │                            NVENC/x264)                                       │
 │  input  ◄──────────────────────────── session ◄──────────────────────────────│
 │  (uinput touch/pen/mouse/kbd, IME)                                           │
 │                                                                              │
 │  ui (egui control panel)   config   discovery (mDNS + UDP)   usb (AOA host)  │
 └──────────────────────────────────────────────────────────────────────────────┘

 ┌──────────────────────────── DREAD Client (Kotlin) ───────────────────────────┐
 │ link (TLS socket | USB accessory) ─► protocol ─► MediaCodec ─► SurfaceView    │
 │ touch / stylus / keyboard ─► input encoder ─► protocol                        │
 └──────────────────────────────────────────────────────────────────────────────┘
```

## Technology choices

| Concern | Choice | Why |
|---|---|---|
| Server language | Rust | Memory-safe, predictable latency, one codebase for Windows and Linux, good FFI to OS APIs. |
| Desktop UI | egui (eframe) | Native, single binary, no web runtime; immediate-mode makes a flat, technical look easy and keeps the UI in the same process as the engine. Qt was considered; it would force a C++ ↔ Rust bridge for every engine call. |
| Video encode | FFmpeg, run as a child process | Gives VAAPI, NVENC, QSV, AMF and x264/x265 behind one interface, on both OSes, with no FFI/ABI coupling to a specific FFmpeg version. The encoder is behind a trait so a libav in-process backend can replace it. |
| Transport | TLS 1.3 over TCP, two connections (control+input, video) | Input is never queued behind large video frames. TCP is predictable through firewalls; DREAD controls latency with frame back-pressure instead of letting TCP queue grow. |
| USB | Android Open Accessory (AOA 2) via libusb | Works without USB debugging. Fallback: USB tethering (plain network path). |
| Android | Kotlin, Views, MediaCodec, SurfaceView | Hardware decode straight to a surface; no Compose/JS runtime. |

## Modules

| # | Module | Location |
|---|---|---|
| 1 | Core engine (sessions, stats, adaptive bitrate) | `server/crates/dread-core/src/session.rs`, `engine.rs` |
| 2 | Display management | `server/crates/dread-platform/src/display/` |
| 3 | Video capture | `server/crates/dread-platform/src/capture/` |
| 4 | Video encoding | `server/crates/dread-platform/src/encode.rs` |
| 5 | Network transport | `server/crates/dread-core/src/transport.rs`, `server/crates/dread-proto` |
| 6 | Input processing | `server/crates/dread-platform/src/input/` |
| 7 | USB transport | `server/crates/dread-core/src/usb.rs`, `android/.../link/UsbLink.kt` |
| 8 | Android decoding | `android/app/src/main/java/app/dread/client/video/` |
| 9 | Desktop interface | `server/crates/dread-app/src/ui/` |
| 10 | Android interface | `android/app/src/main/java/app/dread/client/ui/` |
| 11 | Configuration & diagnostics | `server/crates/dread-core/src/config.rs`, `diag.rs` |

## Display: what "real" means per platform

DREAD never fakes an extended display by cropping a mirrored desktop. The
extended display is an output that the OS display manager itself owns.

| Platform | Extended display | Duplicate | Capture | Status |
|---|---|---|---|---|
| Linux / Hyprland | `hyprctl output create headless` → a real compositor output; mode, position, scale, transform set via Hyprland IPC | screencopy of any existing output | `wlr-screencopy-unstable-v1` (with damage) | Implemented, tested |
| Linux / Sway | `swaymsg create_output` (HEADLESS-n) | screencopy | `wlr-screencopy-unstable-v1` | Implemented, not tested on Sway |
| Linux / GNOME, KDE | Mutter `RecordVirtual` / KWin virtual output via PipeWire | portal ScreenCast | PipeWire | Not implemented yet |
| Linux / X11 | needs a spare RandR output (evdi or `VIRTUAL` output of the driver) | x11grab | FFmpeg `x11grab` | Not implemented yet |
| Windows 10/11 | IddCx driver: bundled, signed, MIT-licensed *Virtual Display Driver* (`Root\MttVDD`) | Desktop Duplication of selected monitor | FFmpeg `ddagrab` (DXGI Desktop Duplication, GPU frames) | Implemented, **not yet compiled or tested** (no Windows build machine) |

See `PLATFORMS.md` for details and limitations.

## Latency strategy

* Encoders run with no B-frames, no lookahead, CBR/VBV of ~1 frame.
* Capture is damage driven: an unchanged screen costs nothing.
* At most *N* (default 2) frames may be un-acknowledged by the client. When
  the link is slower than the encoder, DREAD skips capturing rather than
  queueing — stale frames are never sent.
* Adaptive bitrate lowers the target when ack latency rises and raises it
  slowly when the link is healthy; FFmpeg is restarted with the new target at
  most every few seconds (with a forced key frame).
* Input travels on its own connection with `TCP_NODELAY` and is injected the
  moment it arrives — it never waits on video.

## Security model

* The server listens on the LAN only; it binds to the interfaces chosen in
  settings (default: all private-range IPv4 addresses + loopback, never a
  public address unless the user explicitly allows it).
* All network traffic is TLS 1.3. The server has a persistent self-signed
  certificate; clients pin its SHA-256 fingerprint after pairing.
* Pairing (see `PROTOCOL.md`):
  * **QR** — the QR code carries host, port, certificate fingerprint and a
    one-time 128-bit secret that expires after 5 minutes.
  * **Code comparison** — commit/reveal nonce exchange, both screens show a
    6-digit code derived from the certificate fingerprint and both nonces;
    the user approves on the computer. An active man-in-the-middle cannot
    make the codes match except by a 1-in-a-million guess.
* After pairing the client holds a 256-bit device token. No input is
  accepted from a connection that has not authenticated.
* Paired devices can be revoked from the Connection page.
