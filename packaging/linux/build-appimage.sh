#!/bin/sh
# Builds dist/DREAD-x86_64.AppImage from the Rust workspace.
#
#   packaging/linux/build-appimage.sh
#
# Requirements: cargo, appimagetool (set APPIMAGETOOL=/path/to/appimagetool,
# or pass --fetch-tool to download the official continuous build into
# build/tools/). libusb-1.0 is bundled; FFmpeg is taken from the system at run
# time unless an `ffmpeg` binary is placed in packaging/linux/extra/ before
# building (it is then bundled next to the executable).
set -eu
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="$ROOT/dist"
WORK="$ROOT/build/appimage"
APPDIR="$WORK/DREAD.AppDir"
ARCH=x86_64

TOOL="${APPIMAGETOOL:-$(command -v appimagetool || true)}"
if [ -z "$TOOL" ]; then
    if [ "${1:-}" = "--fetch-tool" ]; then
        mkdir -p "$ROOT/build/tools"
        TOOL="$ROOT/build/tools/appimagetool"
        curl -fL -o "$TOOL" "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$ARCH.AppImage"
        chmod +x "$TOOL"
    else
        echo "appimagetool not found: set APPIMAGETOOL or run with --fetch-tool" >&2
        exit 1
    fi
fi

echo "== building release binary"
(cd "$ROOT/server" && cargo build --release -p dread-app)

echo "== assembling AppDir"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/lib" "$APPDIR/usr/share/applications" "$APPDIR/usr/share/icons/hicolor/scalable/apps" "$APPDIR/usr/share/dread"
install -m755 "$ROOT/server/target/release/dread" "$APPDIR/usr/bin/dread"
if [ -x "$ROOT/packaging/linux/extra/ffmpeg" ]; then
    install -m755 "$ROOT/packaging/linux/extra/ffmpeg" "$APPDIR/usr/bin/ffmpeg"
fi
# libusb is the only non-glibc runtime library the binary links
LIBUSB="$(ldd "$APPDIR/usr/bin/dread" | awk '/libusb-1.0/ {print $3}')"
[ -n "$LIBUSB" ] && cp -L "$LIBUSB" "$APPDIR/usr/lib/"
cp "$ROOT/packaging/linux/dread.desktop" "$APPDIR/usr/share/applications/dread.desktop"
cp "$ROOT/packaging/linux/dread.desktop" "$APPDIR/dread.desktop"
cp "$ROOT/packaging/linux/dread.svg" "$APPDIR/usr/share/icons/hicolor/scalable/apps/dread.svg"
cp "$ROOT/packaging/linux/dread.svg" "$APPDIR/dread.svg"
cp "$ROOT/packaging/linux/"*.rules "$ROOT/packaging/linux/90-dread.quirks" "$ROOT/packaging/linux/install-system-files.sh" "$APPDIR/usr/share/dread/"
cat > "$APPDIR/AppRun" <<'EOF'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
export LD_LIBRARY_PATH="$HERE/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export PATH="$HERE/usr/bin:$PATH"
if [ "${1:-}" = "--install-system-files" ]; then
    exec pkexec "$HERE/usr/share/dread/install-system-files.sh"
fi
exec "$HERE/usr/bin/dread" "$@"
EOF
chmod +x "$APPDIR/AppRun"

echo "== packing"
mkdir -p "$OUT"
ARCH=$ARCH "$TOOL" --no-appstream "$APPDIR" "$OUT/DREAD-$ARCH.AppImage"
echo "built $OUT/DREAD-$ARCH.AppImage"
