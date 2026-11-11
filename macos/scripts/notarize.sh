#!/usr/bin/env bash
# Builds the release app, signs it with a Developer ID, has Apple notarise it, staples the ticket and
# makes the zip and the disk image, in macos/DerivedData/Isomar-<version>/. Run on a Mac that has the
# certificate (scripts/release.sh does it from Linux, with the Flatpak).
#   SIGN_IDENTITY="Developer ID Application: Name (TEAMID)"  the certificate to sign with (default: the
#                                                             keychain's first Developer ID Application)
#   NOTARY_PROFILE=chart-notary                               a `xcrun notarytool store-credentials` profile
#   UNIVERSAL=required                                        fail rather than build for this Mac only
# The libraries Homebrew built carry the macOS version they were built on as their minimum; build cairo,
# pango and glib for the lowest macOS to support before releasing, or the app runs only on the newest.
set -euo pipefail
cd "$(dirname "$0")/../.."
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$PATH"
NOTARY_PROFILE="${NOTARY_PROFILE:-chart-notary}"

# An SSH login does not unlock the login keychain, and codesign then fails with errSecInternalComponent
# (notarytool's profile is in it too). Unlock it from the password file package.sh uses, else ask.
KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"
PASSFILE="$HOME/.config/workout/keychain.pass"
if [ -f "$PASSFILE" ]; then
  security unlock-keychain -p "$(cat "$PASSFILE")" "$KEYCHAIN"
  # Lets codesign use the key without a GUI prompt, which cannot appear over SSH.
  security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$(cat "$PASSFILE")" "$KEYCHAIN" >/dev/null 2>&1 || true
elif ! security show-keychain-info "$KEYCHAIN" >/dev/null 2>&1; then
  echo "→ the login keychain is locked (as it is over SSH); unlocking it to sign" >&2
  security unlock-keychain "$KEYCHAIN"
fi

if [ -z "${SIGN_IDENTITY:-}" ]; then
  SIGN_IDENTITY=$(security find-identity -v -p codesigning | grep "Developer ID Application" | head -1 | sed 's/.*"\(.*\)"/\1/' || true)
fi
: "${SIGN_IDENTITY:?no Developer ID Application certificate in the keychain; set SIGN_IDENTITY}"
echo "→ signing as: $SIGN_IDENTITY" >&2

macos/scripts/build-rust.sh release
cd macos
xcodegen generate --quiet
# A bundle left over from an earlier run (package.sh signs with other settings) is built afresh.
rm -rf DerivedData/Build/Products/Release/Isomar.app
# Universal (Apple silicon and Intel, macOS 13 and up) once the libraries are built for both (macos/deps).
ARCHS_FLAGS=()
LIB_ARCHS=$(lipo -archs ../target/universal/release/libisomar_ffi.a 2>/dev/null || true)
if [ -d "$HOME/dev/isomar-deps/universal/lib" ] && [[ $LIB_ARCHS == *arm64* && $LIB_ARCHS == *x86_64* ]]; then
  ARCHS_FLAGS=("ARCHS=arm64 x86_64" "ONLY_ACTIVE_ARCH=NO")
elif [ "${UNIVERSAL:-}" = required ]; then
  echo "!! the universal libraries are not built on this Mac ($(uname -m)); see macos/deps" >&2
  exit 1
else
  echo "!! building for this Mac only ($(uname -m)): the universal libraries are not built; see macos/deps" >&2
fi
xcodebuild -project Isomar.xcodeproj -scheme Isomar -configuration Release -derivedDataPath DerivedData \
  CODE_SIGN_IDENTITY="$SIGN_IDENTITY" CODE_SIGN_STYLE=Manual "${ARCHS_FLAGS[@]}" -quiet build
APP=DerivedData/Build/Products/Release/Isomar.app
SIGN_IDENTITY="$SIGN_IDENTITY" scripts/bundle-dylibs.sh "$APP"

VERSION=$(/usr/libexec/PlistBuddy -c "Print CFBundleShortVersionString" "$APP/Contents/Info.plist")
OUT="DerivedData/Isomar-$VERSION"
rm -rf "$OUT" && mkdir -p "$OUT"
# A zip cannot be stapled: send one to Apple, then zip the stapled app for people to download.
SUBMIT=$(mktemp -d)/Isomar.zip
ditto -c -k --keepParent "$APP" "$SUBMIT"
xcrun notarytool submit "$SUBMIT" --keychain-profile "$NOTARY_PROFILE" --wait
rm -f "$SUBMIT"
xcrun stapler staple "$APP"
xcrun stapler validate "$APP"
ditto -c -k --keepParent "$APP" "$OUT/Isomar-$VERSION.zip"

DMG="$OUT/Isomar-$VERSION.dmg"
STAGE=$(mktemp -d)
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
hdiutil create -volname "Isomar" -srcfolder "$STAGE" -ov -format UDZO "$DMG"
codesign --force --sign "$SIGN_IDENTITY" "$DMG"
xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
xcrun stapler staple "$DMG"
spctl -a -t exec "$APP"
spctl -a -t open --context context:primary-signature "$DMG"
echo "→ $OUT/Isomar-$VERSION.zip"
echo "→ $DMG"
echo "→ architectures: $(lipo -archs "$APP/Contents/MacOS/Isomar")"
for f in "$APP"/Contents/Frameworks/*.dylib; do [ "$(lipo -archs "$f")" = "x86_64 arm64" ] || [ "$(lipo -archs "$f")" = "arm64 x86_64" ] || echo "   not universal: $(basename "$f") $(lipo -archs "$f")"; done
# Every slice: on an Intel Mac a plain `otool -L` shows only x86_64, and said "none" of 0.7.0's arm64 slices.
echo "→ every library in the bundle, every architecture (otool shows @rpath and system paths only):"
for f in "$APP/Contents/MacOS/Isomar" "$APP"/Contents/Frameworks/*.dylib; do otool -arch all -L "$f" | grep $'^\t' | awk '{print $1}'; done | sort -u | grep -v '^@rpath\|^/usr/lib\|^/System' || echo "   none outside the system"
