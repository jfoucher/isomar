#!/usr/bin/env bash
# Builds the iPhone and iPad app for a device on a Mac over SSH, signed with the team's development
# certificate, installs it on the phone paired with that Mac (over the network, no cable needed) and launches it.
# Usage: scripts/ios-device.sh
#   DEVICE_ID=DF2345F4-…       the phone (`xcrun devicectl list devices` on the Mac); default the iPhone 17
#   ENV="ISOMAR_DEMO=1 ISOMAR_SHEET=medium"   ISOMAR_* variables for the launch (not kept for later launches)
#   BUILD=0                    skip the sync and the build, just install and launch again
#   LAUNCH=0                   install only; open it from the home screen
# The build is macos/scripts/ios-build.sh device: the static C libraries for iOS are built the first time (long).
# Codesign over SSH needs the login keychain unlocked, from ~/.config/workout/keychain.pass on the Mac.
set -euo pipefail

source "$(dirname "$0")/mac-lib.sh"
pick_mac
DEVICE_ID="${DEVICE_ID:-DF2345F4-D3D5-54C0-A8E9-68A4A5A6FD5F}"   # iPhone 17, the same identifier on both Macs
BUNDLE_ID=eu.6px.isomar
BUILD="${BUILD:-1}"
LAUNCH="${LAUNCH:-1}"
ENV_LIST="${ENV:-}"

if [ "$BUILD" = 1 ]; then
  sync_to_mac
  echo "→ building a signed device build on $MAC" >&2
  ssh "$MAC" "~/$REMOTE_DIR/macos/scripts/ios-build.sh device" 2>&1 | { grep -v 'tmux' || true; } | cut -c1-300
fi

# devicectl takes the environment as a JSON object, and refuses an empty one.
ENV_FLAG=""
if [ -n "$ENV_LIST" ]; then
  ENV_JSON=""
  for kv in $ENV_LIST; do ENV_JSON="${ENV_JSON:+$ENV_JSON,}\"${kv%%=*}\":\"${kv#*=}\""; done
  ENV_FLAG="--environment-variables '{$ENV_JSON}'"
fi

echo "→ installing on $DEVICE_ID" >&2
ssh "$MAC" bash -s 2>&1 <<EOF | { grep -v 'tmux' || true; } | cut -c1-300
set -euo pipefail
APP=~/$REMOTE_DIR/macos/DerivedData/Build/Products/Debug-iphoneos/Isomar.app
xcrun devicectl device install app --device $DEVICE_ID "\$APP" | grep -E 'App installed|bundleID|error'
if [ "$LAUNCH" = 1 ]; then
  xcrun devicectl device process launch --device $DEVICE_ID --terminate-existing \
    $ENV_FLAG $BUNDLE_ID | grep -E 'Launched|error' || true
fi
EOF
