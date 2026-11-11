#!/usr/bin/env bash
# Builds the iPhone and iPad app for the iOS simulator on a Mac over SSH, runs it in a simulator and copies
# a screenshot back.
# Usage: scripts/ios-sim.sh [local-screenshot.png]
#   DEVICE="iPhone 17 Pro"     the simulator (any name `xcrun simctl list devices` shows)
#   ENV="ISOMAR_DEBUG_VIEW=47.5,-3.0,10 ISOMAR_PALETTE=day"   ISOMAR_* variables for the run
#   BUILD=0                    skip the sync and the build, just run again
#   DELAY=8                    seconds to wait before the screenshot
# The simulator runs the Mac's own architecture (x86_64 on the Intel Mac Pro, arm64 on Apple silicon); the
# build is macos/scripts/ios-build.sh, which builds the static C libraries for it the first time (long).
# scripts/ios-device.sh is the same for the phone.
set -euo pipefail

source "$(dirname "$0")/mac-lib.sh"
pick_mac
SHOT_LOCAL="${1:-/tmp/isomar-ios-shot.png}"
BUILD="${BUILD:-1}"
DEVICE="${DEVICE:-iPhone 17 Pro}"
DELAY="${DELAY:-8}"
ENV_LIST="${ENV:-}"

if [ "$BUILD" = 1 ]; then
  sync_to_mac
  echo "→ building on $MAC" >&2
  ssh "$MAC" "~/$REMOTE_DIR/macos/scripts/ios-build.sh sim" 2>&1 | { grep -v 'tmux' || true; } | cut -c1-300
fi

echo "→ running in the $DEVICE simulator" >&2
ENV_ARGS=""
# ISOMAR_SCREENSHOT, as on the Mac, keeps the first-run welcome away.
for kv in ISOMAR_SCREENSHOT=1 $ENV_LIST; do ENV_ARGS="$ENV_ARGS SIMCTL_CHILD_${kv}"; done
ssh "$MAC" bash -s 2>&1 <<EOF | { grep -v 'tmux' || true; } | cut -c1-300
set -euo pipefail
APP=~/dev/isomar/macos/DerivedData/Build/Products/Debug-iphonesimulator/Isomar.app
xcrun simctl boot "$DEVICE" 2>/dev/null || true
xcrun simctl bootstatus "$DEVICE" -b >/dev/null
xcrun simctl install "$DEVICE" "\$APP"
# A new install forgets the permission; the system's prompt would cover the screenshot.
xcrun simctl privacy "$DEVICE" grant location eu.6px.isomar 2>/dev/null || true
# --terminate-running-process: simctl terminate hangs when the app is not running.
env $ENV_ARGS xcrun simctl launch --terminate-running-process "$DEVICE" eu.6px.isomar
sleep $DELAY
xcrun simctl io "$DEVICE" screenshot /tmp/isomar-ios-shot.png >/dev/null
EOF
scp -q "$MAC:/tmp/isomar-ios-shot.png" "$SHOT_LOCAL"
echo "→ screenshot: $SHOT_LOCAL" >&2
