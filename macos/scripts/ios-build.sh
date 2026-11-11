#!/usr/bin/env bash
# Builds the iPhone and iPad app on this Mac, for the simulator or for a device; run over SSH by
# scripts/ios-sim.sh and scripts/ios-device.sh from the repository's copy on the Mac.
# Usage: macos/scripts/ios-build.sh sim|device|archive
#   sim      for this Mac's simulator, which runs the Mac's own architecture (x86_64 on an Intel Mac)
#   device   arm64, signed with the team's development certificate (CODE_SIGN_STYLE Automatic)
#   archive  the Release build for the App Store, in macos/DerivedData/IsomarTouch.xcarchive
#            (macos/scripts/ios-release.sh exports and uploads it); BUILD_NUMBER sets CFBundleVersion
# The static cairo, pango and the rest are built the first time (scripts/deps/build-mobile-deps.sh, a long
# build), then the Rust library, then the app into macos/DerivedData.
set -euo pipefail
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$PATH"
cd "$(dirname "$0")/../.."
MODE=${1:?sim or device}

case "$MODE" in
  sim)
    if [ "$(uname -m)" = x86_64 ]; then TRIPLE=x86_64-apple-ios; DEPS=ios-sim-x86_64
    else TRIPLE=aarch64-apple-ios-sim; DEPS=ios-sim; fi
    SDK=iphonesimulator; DEST='generic/platform=iOS Simulator' ;;
  device|archive)
    TRIPLE=aarch64-apple-ios; DEPS=ios; SDK=iphoneos; DEST='generic/platform=iOS' ;;
  *) echo "sim, device or archive, not $MODE" >&2; exit 2 ;;
esac

# Every build is signed with the team's certificate (the simulator's too). A keychain unlocked in one SSH
# session is locked again in the next, and codesign then fails with errSecInternalComponent ("internal error
# in Code Signing subsystem"). Unlock it from the password file the other projects use.
PASSFILE="$HOME/.config/workout/keychain.pass"
if [ -r "$PASSFILE" ]; then
  security unlock-keychain -p "$(cat "$PASSFILE")" ~/Library/Keychains/login.keychain-db
  security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$(cat "$PASSFILE")" \
    ~/Library/Keychains/login.keychain-db >/dev/null 2>&1 || true
else
  echo "!! $PASSFILE is missing: codesign will fail with errSecInternalComponent over SSH" >&2
fi

PREFIX="${DEPS_DIR:-$HOME/dev/isomar-deps}/prefix-$DEPS"
if [ ! -d "$PREFIX" ]; then
  echo "→ building cairo, pango and the rest for $DEPS (once)" >&2
  scripts/deps/build-mobile-deps.sh "$DEPS" 2>&1 | tail -5
fi
rustup target list --installed | grep -q "^$TRIPLE\$" || rustup target add "$TRIPLE"
# Only the prefix's libraries, static, bundled by rustc into libisomar_ffi.a.
PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_ALL_STATIC=1 PKG_CONFIG_PATH="" PKG_CONFIG_LIBDIR="$PREFIX/lib/pkgconfig" \
  IPHONEOS_DEPLOYMENT_TARGET=16.0 cargo build --release -p isomar-ffi --target "$TRIPLE" 2>&1 \
  | grep -E '^(warning|error)|error:|Finished' || true
test -f "target/$TRIPLE/release/libisomar_ffi.a"
cd macos
xcodegen generate --quiet
if [ "$MODE" = archive ]; then
  rm -rf DerivedData/IsomarTouch.xcarchive
  xcodebuild -project Isomar.xcodeproj -scheme IsomarTouch -configuration Release -sdk $SDK \
    -destination "$DEST" -derivedDataPath DerivedData -archivePath DerivedData/IsomarTouch.xcarchive \
    -allowProvisioningUpdates -quiet archive ${BUILD_NUMBER:+CURRENT_PROJECT_VERSION=$BUILD_NUMBER}
else
  xcodebuild -project Isomar.xcodeproj -scheme IsomarTouch -configuration Debug -sdk $SDK \
    -destination "$DEST" -derivedDataPath DerivedData -allowProvisioningUpdates -quiet build
fi
