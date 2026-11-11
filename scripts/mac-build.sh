#!/usr/bin/env bash
# Builds the macOS app on a Mac over SSH, runs it once with a screenshot, and copies that back.
# Usage: scripts/mac-build.sh [local-screenshot.png]
#   ENV="ISOMAR_DEMO=1 ISOMAR_PALETTE=night"   ISOMAR_* variables for the run (see CLAUDE.md)
#   BUILD=0                                  skip the sync and the build, just run again
#   SIZE=1440x900                            window content size
#   DELAY=3000                               ms to wait before the screenshot
set -euo pipefail

source "$(dirname "$0")/mac-lib.sh"
pick_mac
SHOT_LOCAL="${1:-/tmp/isomar-mac-shot.png}"
BUILD="${BUILD:-1}"
SIZE="${SIZE:-1200x780}"
DELAY="${DELAY:-3000}"
ENV_LIST="${ENV:-}"

if [ "$BUILD" = 1 ]; then
  sync_to_mac
  ensure_tools
  echo "→ building on $MAC" >&2
  ssh "$MAC" bash -s 2>&1 <<EOF | { grep -v 'tmux' || true; } | cut -c1-300
$(remote_prelude)
cd ~/$REMOTE_DIR
cd macos
xcodegen generate --quiet
# The Rust library and the bundled libraries are build phases of the target, as they are for Run in Xcode.
xcodebuild -project Isomar.xcodeproj -scheme Isomar -configuration Debug -derivedDataPath DerivedData -quiet build
EOF
fi

echo "→ running" >&2
ENV_ARGS=""
for kv in $ENV_LIST; do ENV_ARGS="$ENV_ARGS --env '$kv'"; done
ssh "$MAC" bash -s 2>&1 <<EOF | { grep -v 'tmux' || true; } | cut -c1-300
$(remote_prelude)
rm -f /tmp/isomar-shot.png /tmp/isomar-shot.png.sheet.png /tmp/isomar-shot.png.window-*.png
rm -rf /tmp/isomar-test-config /tmp/isomar-test-state
mkdir -p /tmp/isomar-test-config /tmp/isomar-test-state
open -n -W $ENV_ARGS \
  --env ISOMAR_SCREENSHOT=/tmp/isomar-shot.png --env ISOMAR_SCREENSHOT_DELAY=$DELAY --env ISOMAR_SIZE=$SIZE \
  --env ISOMAR_CONFIG_DIR=\${ISOMAR_CONFIG_DIR:-/tmp/isomar-test-config} --env ISOMAR_STATE_DIR=/tmp/isomar-test-state --env ISOMAR_NO_SAVE=1 \
  ~/$REMOTE_DIR/macos/DerivedData/Build/Products/Debug/Isomar.app
EOF
scp -q "$MAC:/tmp/isomar-shot.png" "$SHOT_LOCAL"
scp -q "$MAC:/tmp/isomar-shot.png.sheet.png" "$SHOT_LOCAL.sheet.png" 2>/dev/null || true
for n in 0 1 2; do scp -q "$MAC:/tmp/isomar-shot.png.window-$n.png" "$SHOT_LOCAL.window-$n.png" 2>/dev/null || true; done
echo "→ screenshot: $SHOT_LOCAL" >&2
