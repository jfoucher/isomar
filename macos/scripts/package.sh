#!/usr/bin/env bash
# Builds the Release app, bundles its libraries, signs it for team 24G23598DF and makes a zip and a disk
# image to send. Run on the Mac (scripts/mac-package.sh does it from here).
#
# The identity is the first of: $SIGN_IDENTITY, a "Developer ID Application" certificate, an "Apple
# Development" certificate. Only a Developer ID certificate, plus notarisation (macos/scripts/notarize.sh),
# makes the app open with a double-click on another Mac. Signed with a development certificate it runs, but
# the first time the person has to open it with right-click, Open (or Privacy & Security, Open Anyway).
set -euo pipefail
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$PATH"
cd "$(dirname "$0")/../.."
TEAM=24G23598DF

# A keychain unlocked in one SSH session is locked again in the next, and codesign then fails with
# errSecInternalComponent. Unlock it from the password file the other projects use (~/.config/workout/keychain.pass).
PASSFILE="$HOME/.config/workout/keychain.pass"
if [ -f "$PASSFILE" ]; then
  security unlock-keychain -p "$(cat "$PASSFILE")" ~/Library/Keychains/login.keychain-db
  security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$(cat "$PASSFILE")" ~/Library/Keychains/login.keychain-db >/dev/null 2>&1 || true
fi

if [ -z "${SIGN_IDENTITY:-}" ]; then
  SIGN_IDENTITY=$(security find-identity -v -p codesigning | grep "Developer ID Application" | head -1 | sed 's/.*"\(.*\)"/\1/' || true)
  [ -n "$SIGN_IDENTITY" ] || SIGN_IDENTITY=$(security find-identity -v -p codesigning | grep "Apple Development" | grep "$TEAM\|3SC3N5D724" | head -1 | sed 's/.*"\(.*\)"/\1/' || true)
fi
: "${SIGN_IDENTITY:?no signing certificate found}"
echo "→ signing as: $SIGN_IDENTITY" >&2

macos/scripts/build-rust.sh release
cd macos
xcodegen generate --quiet
# A bundle left over from an earlier run holds libraries that are not signed for this build.
rm -rf DerivedData/Build/Products/Release/Isomar.app
# Universal (Apple silicon and Intel, macOS 13 and up) once the libraries are built for both (macos/deps).
ARCHS_FLAGS=()
LIB_ARCHS=$(lipo -archs ../target/universal/release/libisomar_ffi.a 2>/dev/null || true)
if [ -d "$HOME/dev/isomar-deps/universal/lib" ] && [[ $LIB_ARCHS == *arm64* && $LIB_ARCHS == *x86_64* ]]; then
  ARCHS_FLAGS=("ARCHS=arm64 x86_64" "ONLY_ACTIVE_ARCH=NO")
else
  echo "!! building for this Mac only ($(uname -m)): the universal libraries are not built; see macos/deps" >&2
fi
xcodebuild -project Isomar.xcodeproj -scheme Isomar -configuration Release -derivedDataPath DerivedData \
  DEVELOPMENT_TEAM=$TEAM CODE_SIGN_IDENTITY="$SIGN_IDENTITY" CODE_SIGN_STYLE=Manual "${ARCHS_FLAGS[@]}" -quiet build
APP=DerivedData/Build/Products/Release/Isomar.app
SIGN_IDENTITY="$SIGN_IDENTITY" scripts/bundle-dylibs.sh "$APP"
codesign --verify --deep --strict --verbose=1 "$APP"

VERSION=$(/usr/libexec/PlistBuddy -c "Print CFBundleShortVersionString" "$APP/Contents/Info.plist")
OUT="$PWD/DerivedData/Package"
rm -rf "$OUT" && mkdir -p "$OUT"
ditto -c -k --keepParent "$APP" "$OUT/Isomar-$VERSION.zip"
STAGE=$(mktemp -d)
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
hdiutil create -volname "Isomar" -srcfolder "$STAGE" -ov -format UDZO "$OUT/Isomar-$VERSION.dmg" >/dev/null
echo "→ $OUT/Isomar-$VERSION.zip"
echo "→ $OUT/Isomar-$VERSION.dmg"
echo "→ architectures: $(lipo -archs "$APP/Contents/MacOS/Isomar")"
echo "→ the newest minimum macOS among all slices of all libraries (13.0 is wanted):"
for f in "$APP/Contents/MacOS/Isomar" "$APP"/Contents/Frameworks/*.dylib; do vtool -show-build "$f" 2>/dev/null | awk '/minos/ {print $2}'; done | sort -t. -k1,1n -k2,2n | tail -1
for f in "$APP"/Contents/Frameworks/*.dylib; do [ "$(lipo -archs "$f")" = "x86_64 arm64" ] || [ "$(lipo -archs "$f")" = "arm64 x86_64" ] || echo "   not universal: $(basename "$f") $(lipo -archs "$f")"; done
