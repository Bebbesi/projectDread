#!/bin/sh
# Installs DREAD's udev rules and libinput quirk (needs root once).
# Usage: sudo ./install-system-files.sh        (or: sudo ./install-system-files.sh --remove)
set -e
DIR="$(cd "$(dirname "$0")" && pwd)"
if [ "$(id -u)" != 0 ]; then echo "run with sudo" >&2; exit 1; fi
if [ "$1" = "--remove" ]; then
    rm -f /etc/udev/rules.d/70-dread-uinput.rules /etc/udev/rules.d/70-dread-usb.rules /etc/libinput/90-dread.quirks
else
    install -Dm644 "$DIR/70-dread-uinput.rules" /etc/udev/rules.d/70-dread-uinput.rules
    install -Dm644 "$DIR/70-dread-usb.rules" /etc/udev/rules.d/70-dread-usb.rules
    install -Dm644 "$DIR/90-dread.quirks" /etc/libinput/90-dread.quirks
    modprobe uinput 2>/dev/null || true
    echo uinput > /etc/modules-load.d/dread-uinput.conf
fi
udevadm control --reload
udevadm trigger --subsystem-match=usb --subsystem-match=misc
echo "done — re-plug the Android device"
