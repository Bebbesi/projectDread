# Installing DREAD

## Linux server

Requirements: Hyprland (Sway untested; GNOME/KDE/X11 not supported yet — see
[PLATFORMS.md](PLATFORMS.md)), FFmpeg, libusb-1.0. For hardware encoding:
`libva` + the VA-API driver for your GPU (`intel-media-driver`,
`libva-mesa-driver`) or the NVIDIA driver with NVENC.

```bash
# Arch
sudo pacman -S ffmpeg libusb libva-intel-driver   # or intel-media-driver / libva-mesa-driver
# Debian/Ubuntu
sudo apt install ffmpeg libusb-1.0-0 va-driver-all
```

1. Install the system files once (uinput + USB access, pen quirk):

   ```bash
   sudo packaging/linux/install-system-files.sh
   ```

   With the AppImage: `./DREAD-x86_64.AppImage --install-system-files`.
   Log out and in (or re-plug the device) afterwards.

2. Run DREAD: `./dread-linux-x86_64` or `./DREAD-x86_64.AppImage`.
   Enable *Advanced → Automatic startup* to start it at login.

3. Firewall: allow TCP 47800 (or your port) and UDP 47801 from your LAN,
   e.g. `sudo ufw allow from 192.168.0.0/16 to any port 47800 proto tcp`.

Uninstall: `sudo packaging/linux/install-system-files.sh --remove`, delete
the binary/AppImage, `~/.config/dread/` (settings, certificate, paired
devices) and `~/.config/autostart/dread.desktop` if present.

## Windows server

1. Run `DREAD-Setup-1.0.0.exe` as administrator. Keep *Install the virtual
   display driver* checked for Extended mode and *Allow through firewall*.
2. Start DREAD from the Start menu.

When updating, quit DREAD first (including the tray icon) so the installer
can replace `dread.exe`.

The installer bundles `dread.exe`, FFmpeg, the Virtual Display Driver and
nefcon. Uninstall from *Settings → Apps*: removes the driver, firewall rules
and autostart entry. Settings live in `%APPDATA%\dread\dread\config`.

Manual driver install (elevated PowerShell, from the installed folder):

```powershell
powershell -ExecutionPolicy Bypass -File "C:\Program Files\DREAD\driver-tools\install-driver.ps1"
```

## Android client

1. Copy `DREAD.apk` to the device and open it (allow *Install unknown apps*
   for your file manager or browser once), or `adb install DREAD.apk`.
2. Open DREAD.

### Connect over Wi-Fi / LAN

* Phone and computer on the same network.
* **Scan QR code** on the computer's *Connection* page — pairs instantly.
* Or pick the computer under *On this network*, or type its address. The
  first time, both screens show a six-digit code; approve it on the computer
  if they match.

### Connect over USB

* Plug the device into the computer with a data cable while DREAD runs.
* Android asks *Open DREAD to handle DREAD Display?* — tap OK (tick *Always*).
* No developer options or USB debugging are needed. Details: [USB.md](USB.md).

### Using the display

* Toolbar (right edge, hides itself; tap the handle to show): keyboard,
  input mode (Touch → Mouse → Trackpad), statistics, fullscreen, settings,
  disconnect.
* Back twice disconnects. The computer keeps the display for 15 s, so a quick
  reconnect finds windows where they were.
