#!/usr/bin/env bash
# Renders Isomar in a private headless GNOME Shell and saves a PNG.
# Usage: scripts/screenshot.sh <out.png> [delay-ms]
# Uses a private config dir and D-Bus session so your desktop is untouched.
# ISOMAR_BIN overrides the binary, e.g. ISOMAR_BIN="flatpak run eu._6px.isomar".
# ISOMAR_SHOT_TIMEOUT is how long the app may run, in seconds (60 by default).
# ISOMAR_MONITOR is the virtual monitor (1280x820 by default; the owner's panel is 2256x1504).
# ISOMAR_SHOT_LOG keeps the app's output in that file instead of a temporary one.
set -euo pipefail
OUT=${1:?out.png}; DELAY=${2:-2000}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
CFG=${ISOMAR_TEST_CONFIG:-$(mktemp -d)}
DATA=$(mktemp -d)
LOG=${ISOMAR_SHOT_LOG:-$(mktemp)}
BIN=${ISOMAR_BIN:-"'$ROOT/target/debug/isomar'"}
APP_TIMEOUT=${ISOMAR_SHOT_TIMEOUT:-60}
rm -f "$OUT"
# A socket left by an earlier run that was killed would pass the wait below.
rm -f "${XDG_RUNTIME_DIR:-/run/user/$UID}/isomar-shot" "${XDG_RUNTIME_DIR:-/run/user/$UID}/isomar-shot.lock"
timeout $((APP_TIMEOUT + 30)) dbus-run-session -- bash -c "
  gnome-shell --headless --wayland --no-x11 --virtual-monitor ${ISOMAR_MONITOR:-1280x820} --wayland-display isomar-shot >>'$LOG' 2>&1 &
  SP=\$!
  for i in \$(seq 50); do [ -S \"\$XDG_RUNTIME_DIR/isomar-shot\" ] && break; sleep 0.2; done
  sleep 1
  XDG_CONFIG_HOME='$CFG' XDG_DATA_HOME='$DATA' WAYLAND_DISPLAY=isomar-shot GDK_BACKEND=wayland \
    ISOMAR_SCREENSHOT='$OUT' ISOMAR_SCREENSHOT_DELAY='$DELAY' \
    timeout $APP_TIMEOUT $BIN 2>&1 | grep -v -E '^\$|Gtk-WARNING|^\(isomar' || true
  kill \$SP
" >>"$LOG" 2>&1 || true
if [ -f "$OUT" ]; then echo "saved $OUT"; else echo "screenshot failed, see $LOG"; exit 1; fi
