# Platform support and limitations

## Linux

DREAD needs a compositor that can create outputs on demand and lets a client
capture an output. Detection is automatic (`HYPRLAND_INSTANCE_SIGNATURE`,
`SWAYSOCK`).

### Hyprland (verified)

* **Extend** — `hyprctl output create headless DREAD-1`; mode, position,
  scale applied with `hl.monitor{…}` (Lua config, Hyprland ≥ 0.50) or
  `keyword monitor` (older). The output is a first-class Hyprland monitor with
  its own workspaces. Like a physical monitor, Hyprland may return workspaces
  that were on `DREAD-1` when it reconnects; DREAD guarantees the workspaces
  *visible* on your other monitors stay where they are.
* **Capture** — `wlr-screencopy-unstable-v1` v3 with damage: an unchanged
  screen produces no frames. The cursor is composited by the compositor
  (Streaming → "Draw the mouse cursor").
* **Input** — uinput devices bound to the output with `hl.device{ output = … }`.
  Pointer mode maps into the whole layout box. Unicode text via
  `zwp_input_method_v2` (when no other IME such as fcitx5/ibus is running) or
  `zwp_virtual_keyboard_v1` with a generated keymap.
* **Primary display** — not applicable: Wayland compositors have no primary
  output (the setting is disabled with an explanation).
* **Requirements** — FFmpeg; `/dev/uinput` access (udev rule); for USB, access
  to the phone's USB device node (udev rule).

### Sway (implemented, not yet tested)

`swaymsg create_output` → `HEADLESS-n`, configured with `swaymsg output …`,
inputs mapped with `map_to_output`, removed with `output … unplug`.

### GNOME, KDE Plasma, X11 (not implemented)

Planned approaches, none in this release:

* GNOME/Mutter — `org.gnome.Mutter.ScreenCast.RecordVirtual` creates a virtual
  monitor delivered as a PipeWire stream.
* KDE Plasma 6 — virtual output via the xdg-desktop-portal ScreenCast
  `VIRTUAL` source type / `zkde_screencast`.
* X11 — needs a spare RandR output (evdi kernel module or a driver `VIRTUAL`
  output) plus `x11grab`.

DREAD reports "unsupported desktop session" on these instead of faking an
extended display.

### Performance notes (Linux)

Frames are copied from a shared-memory buffer into FFmpeg (one copy) and
uploaded to the GPU for VA-API. Measured 11–20 ms capture→encoded at
1280×800 on an Intel UHD 620. Zero-copy DMA-BUF capture straight into VA-API
is the main planned optimisation for 4K.

Binaries built on a recent distribution need a glibc at least as new as the
build host; build release AppImages on the oldest distribution you support.

## Windows 10/11 (Extend + Duplicate verified; input not yet verified)

Built and run on Windows 11 Pro (build 26200) with an NVIDIA RTX 4060 Ti;
Extend and Duplicate stream to a Samsung Galaxy S23 (SCG19). See
[TESTING.md](TESTING.md#server-on-windows) for exactly what was checked.

* **Driver** — bundles the signed, MIT-licensed *Virtual Display Driver*
  (IddCx, hardware id `Root\MttVDD`, release 25.7.23). The installer copies
  `vdd_settings.xml` to `C:\VirtualDisplayDriver`, creates the device node
  with nefcon and installs the driver with `pnputil` (log:
  `C:\Program Files\DREAD\driver-tools\install-driver.log`).
* **Mode list must stay small** — the settings file has one monitor with 16
  resolutions (common desktop, tablet and phone panels, both orientations) at
  60/120 Hz = 32 modes. With ~88 or more modes Windows rejects the monitor
  (driver log: *Failed to report monitor arrival*, `STATUS_NOT_SUPPORTED`)
  and Extend fails; 48 modes was tested OK.
* **Extend** — attaches the driver's monitor with `ChangeDisplaySettingsExW`
  (mode, refresh, position, optional *primary*); detaches it (zero-size mode)
  when streaming stops. Windows lists it in Settings → Display like any monitor.
  Requested resolutions snap to the closest mode in the driver's list. The
  monitor's GDI name (`\\.\DISPLAYn`) changes every time the driver restarts,
  so it is looked up again on each attach.
* **Capture/encode** — FFmpeg `ddagrab` (DXGI Desktop Duplication, GPU frames)
  into NVENC / AMF / Quick Sync, or software x264.
* **Input** — touch through `InjectTouchInput` (virtual-desktop pixels, works
  on any monitor; verified). Pen through a synthetic pointer device
  (`InjectSyntheticPointerInput`, Windows 10 1809+): **unverified**, and
  Windows binds synthetic devices to the primary monitor (this broke touch on
  the DREAD display before the switch), so pen may land on the primary
  monitor. Pen carries pressure, tilt, eraser, barrel and hover (Windows
  Ink). `SendInput` for mouse, wheel, scan-code keys and Unicode text.
* **Limitations** — duplicate mode needs no driver; adaptive bitrate restarts
  FFmpeg (as on Linux); text-field detection for the automatic Android
  keyboard is Linux-only (use the keyboard button).

## Android

* Android 8.0 (API 26) and newer; tested on Android 16.
* Hardware decoding through MediaCodec; low-latency mode on API 30+ and
  vendor switches for older Qualcomm/Exynos/MediaTek decoders.
* Stylus: pressure, tilt (orientation + tilt axes), hover, eraser tool and
  barrel buttons are forwarded when the device reports them.
* Android 16 ignores fixed orientations on large screens; "Screen rotation"
  then behaves like *Automatic* on tablets.
* USB accessory mode needs a data cable and Android's one-time "Open DREAD?"
  confirmation.
