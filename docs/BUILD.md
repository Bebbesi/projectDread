# Building from source

## Server (Linux)

```bash
cd server
cargo build --release          # → target/release/dread
cargo test                     # unit tests
```

Build dependencies: Rust ≥ 1.80, `pkg-config`, libusb-1.0 and libudev headers
(`libusb-1.0-0-dev libudev-dev` on Debian/Ubuntu). Runtime: FFmpeg.

Useful flags: `dread --headless --print-pairing-uri` (no GUI, prints the QR
URI), `dread --page connection`, `DREAD_LOG=debug`, `DREAD_CONFIG_DIR=/path`
(separate config, used for testing).

### AppImage

```bash
APPIMAGETOOL=/path/to/appimagetool packaging/linux/build-appimage.sh
# or let the script download the official appimagetool:
packaging/linux/build-appimage.sh --fetch-tool
```

Output: `dist/DREAD-x86_64.AppImage`. It bundles libusb; FFmpeg comes from the
system unless you drop a static `ffmpeg` into `packaging/linux/extra/` first.
Build on the oldest distribution you want to support (glibc compatibility).

*This step was not run on the development machine: downloading
`appimagetool` was not permitted there.*

### End-to-end check without a phone

```bash
cargo build
DREAD_CONFIG_DIR=/tmp/dread-test ./target/debug/dread --headless --print-pairing-uri &
./target/debug/dread-testclient --qr 'dread://pair?…' --seconds 5 --out /tmp/v.h264 --input-test
ffprobe /tmp/v.h264
```

`cargo run -p dread-platform --example smoke -- 4 /tmp/s.h264 vaapi` tests
display + capture + encoder alone.

## Server (Windows)

On Windows 10/11 x64 with the MSVC Rust toolchain and Inno Setup 6:

```powershell
powershell -ExecutionPolicy Bypass -File packaging\windows\build-windows.ps1
```

It builds `dread.exe`, downloads the pinned Virtual Display Driver release,
nefcon and an FFmpeg build, and produces `dist\DREAD-Setup-1.0.0.exe`.

Verified on Windows 11 with Rust stable (MSVC) and Inno Setup 6. Inno Setup
installed per user (e.g. `winget install JRSoftware.InnoSetup`) lands in
`%LOCALAPPDATA%\Programs\Inno Setup 6`; add that folder to `PATH` first so
the script finds `ISCC.exe`. The downloads are cached in `build\downloads`.

Windows diagnostics (no tablet needed):

```powershell
cd server
cargo run --release -p dread-platform --example win_displays   # adapters, monitors, modes
cargo run --release -p dread-platform --example win_attach     # attach DREAD display 1920x1200 for 3 s, then detach
```

## Android client

Requirements: JDK 17+ (21 used), Android SDK platform 36 + build-tools 36.

```bash
cd android
export JAVA_HOME=… ANDROID_HOME=…
./gradlew assembleDebug        # app/build/outputs/apk/debug/app-debug.apk
./build-apk.sh                 # signed release → dist/DREAD.apk
```

`build-apk.sh` creates `android/dread-release.jks` (password `dread-local`) on
first run. **Keep it** (and change the password via `DREAD_KEYSTORE_PASSWORD`
/ `DREAD_KEY_PASSWORD` for real releases): updates must be signed with the
same key. The keystore is git-ignored.
