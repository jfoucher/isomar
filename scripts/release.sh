#!/usr/bin/env bash
# Builds everything there is to release, into dist/<version>/:
#   Isomar-<version>.flatpak   built here (scripts/flatpak.sh)
#   Isomar-<version>.dmg       built on a Mac over SSH, signed with the Developer ID, notarised and stapled
#   Isomar-<version>.zip       the same app, zipped (macos/scripts/notarize.sh)
#   Isomar-<version>.apk       the Android app, built here and signed with the release key
#   Isomar-<version>.ipa       the iPhone and iPad app, built on the Mac and uploaded to App Store Connect
#                              for TestFlight (macos/scripts/ios-release.sh); the website links TestFlight
# The Flatpak and the APK build here while the Mac builds; their logs are dist/<version>/{flatpak,android}.log.
# Usage: scripts/release.sh
#   MAC=user@host              the Mac to build on (default: the first of mac-lib.sh's that answers)
#   NOTARY_PROFILE=chart-notary, SIGN_IDENTITY=…   passed on to notarize.sh
#   SKIP_FLATPAK=1, SKIP_MAC=1, SKIP_ANDROID=1, SKIP_IOS=1   leave that one out
#   UPLOAD_IOS=0               build the .ipa without uploading it
#   ANDROID_KEY_DIR=~/.config/isomar-release   android-release.jks and its password, android-release.pass.
#                              Every APK must be signed with this one key, or installed copies refuse the
#                              update: keep a backup of both files.
set -euo pipefail
source "$(dirname "$0")/mac-lib.sh"
cd "$ISOMAR_ROOT"

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
MAC_VERSION=$(sed -n 's/^ *MARKETING_VERSION: *"\(.*\)"/\1/p' macos/project.yml | head -1)
if [ "$VERSION" != "$MAC_VERSION" ]; then
  echo "!! Cargo.toml says $VERSION but macos/project.yml's MARKETING_VERSION says $MAC_VERSION" >&2
  exit 1
fi
ANDROID_VERSION=$(sed -n 's/^ *versionName = "\(.*\)"/\1/p' android/app/build.gradle.kts | head -1)
if [ "$VERSION" != "$ANDROID_VERSION" ]; then
  echo "!! Cargo.toml says $VERSION but android/app/build.gradle.kts's versionName says $ANDROID_VERSION" >&2
  exit 1
fi
ANDROID_KEY_DIR=${ANDROID_KEY_DIR:-$HOME/.config/isomar-release}
if [ "${SKIP_ANDROID:-}" != 1 ] && [ ! -r "$ANDROID_KEY_DIR/android-release.jks" ]; then
  echo "!! no $ANDROID_KEY_DIR/android-release.jks: the APK must be signed with the release key (SKIP_ANDROID=1 to leave it out)" >&2
  exit 1
fi
OUT="dist/$VERSION"
mkdir -p "$OUT"
echo "→ releasing $VERSION into $OUT" >&2

FLATPAK_PID=""
if [ "${SKIP_FLATPAK:-}" != 1 ]; then
  echo "→ building the Flatpak here (log: $OUT/flatpak.log)" >&2
  scripts/flatpak.sh >"$OUT/flatpak.log" 2>&1 &
  FLATPAK_PID=$!
fi

ANDROID_PID=""
if [ "${SKIP_ANDROID:-}" != 1 ]; then
  echo "→ building the Android app here (log: $OUT/android.log)" >&2
  ISOMAR_ANDROID_KEYSTORE="$ANDROID_KEY_DIR/android-release.jks" \
  ISOMAR_ANDROID_KEYSTORE_PASSWORD="$(cat "$ANDROID_KEY_DIR/android-release.pass")" \
    scripts/android-build.sh release arm64-v8a x86_64 >"$OUT/android.log" 2>&1 &
  ANDROID_PID=$!
fi
# Stops them if this script stops early (no Mac answered, Ctrl+C).
trap 'kill $FLATPAK_PID $ANDROID_PID 2>/dev/null || true' EXIT

MAC_OK=1
if [ "${SKIP_MAC:-}" != 1 ]; then
  pick_mac
  sync_to_mac
  ensure_tools
  echo "→ building, signing and notarising on $MAC (notarisation takes a few minutes)" >&2
  # pipefail makes the pipeline's status the remote script's (the grep always succeeds).
  # printf %q keeps the identity's spaces and brackets whole through the remote shell.
  if ssh "$MAC" bash -s 2>&1 <<EOF | { grep -v 'tmux' || true; }
$(remote_prelude)
cd ~/$REMOTE_DIR
UNIVERSAL=required NOTARY_PROFILE=$(printf %q "${NOTARY_PROFILE:-chart-notary}") SIGN_IDENTITY=$(printf %q "${SIGN_IDENTITY:-}") \
  macos/scripts/notarize.sh
EOF
  then
    rm -f "$OUT/Isomar-$VERSION.dmg" "$OUT/Isomar-$VERSION.zip"
    scp -q "$MAC:$REMOTE_DIR/macos/DerivedData/Isomar-$VERSION/Isomar-$VERSION.dmg" \
           "$MAC:$REMOTE_DIR/macos/DerivedData/Isomar-$VERSION/Isomar-$VERSION.zip" "$OUT/"
  else
    MAC_OK=0
    echo "!! the macOS build or its notarisation failed" >&2
  fi
fi

IOS_OK=1
if [ "${SKIP_IOS:-}" != 1 ]; then
  if [ "${SKIP_MAC:-}" = 1 ]; then pick_mac; sync_to_mac; ensure_tools; fi
  echo "→ archiving the iOS app on $MAC$([ "${UPLOAD_IOS:-1}" = 1 ] && echo ' and uploading it to App Store Connect')" >&2
  if ssh "$MAC" bash -s 2>&1 <<EOF | { grep -v 'tmux' || true; }
$(remote_prelude)
cd ~/$REMOTE_DIR
UPLOAD=${UPLOAD_IOS:-1} macos/scripts/ios-release.sh
EOF
  then
    rm -f "$OUT/Isomar-$VERSION.ipa"
    scp -q "$MAC:$REMOTE_DIR/macos/DerivedData/Isomar-$VERSION/Isomar-$VERSION.ipa" "$OUT/"
  else
    IOS_OK=0
    echo "!! the iOS build or its upload failed" >&2
  fi
fi

FLATPAK_OK=1
if [ -n "$FLATPAK_PID" ]; then
  echo "→ waiting for the Flatpak" >&2
  if wait "$FLATPAK_PID"; then
    mv "dist/Isomar-$VERSION.flatpak" "$OUT/"
    rm -f "$OUT/flatpak.log"
  else
    FLATPAK_OK=0
    echo "!! the Flatpak failed; the end of $OUT/flatpak.log:" >&2
    tail -20 "$OUT/flatpak.log" >&2
  fi
fi

ANDROID_OK=1
if [ -n "$ANDROID_PID" ]; then
  echo "→ waiting for the Android app" >&2
  APK=android/app/build/outputs/apk/release/app-release.apk
  if wait "$ANDROID_PID" && [ -f "$APK" ]; then
    # An unsigned or wrongly signed APK installs nowhere, or refuses later updates: check the signer.
    BT=$(ls -d "${ANDROID_HOME:-$HOME/Android/Sdk}"/build-tools/* | sort -V | tail -1)
    JAVA=$(ls -d "$HOME"/.jdks/openjdk-21* 2>/dev/null | head -1)
    KEYTOOL=$( [ -x "$JAVA/bin/keytool" ] && echo "$JAVA/bin/keytool" || echo keytool )
    WANT=$("$KEYTOOL" -list -keystore "$ANDROID_KEY_DIR/android-release.jks" -storepass "$(cat "$ANDROID_KEY_DIR/android-release.pass")" 2>/dev/null \
      | sed -n 's/.*(SHA-256): //p' | tr -d ':' | tr 'A-F' 'a-f')
    GOT=$("$BT/apksigner" verify --print-certs "$APK" 2>/dev/null | sed -n 's/^Signer #1 certificate SHA-256 digest: //p')
    if [ -n "$WANT" ] && [ "$WANT" = "$GOT" ]; then
      cp "$APK" "$OUT/Isomar-$VERSION.apk"
      rm -f "$OUT/android.log"
    else
      ANDROID_OK=0
      echo "!! $APK is not signed with the release key (signer: ${GOT:-none})" >&2
    fi
  else
    ANDROID_OK=0
    echo "!! the Android build failed; the end of $OUT/android.log:" >&2
    tail -20 "$OUT/android.log" >&2
  fi
fi

echo "→ $OUT:" >&2
ls -la "$OUT" >&2
[ "$MAC_OK" = 1 ] && [ "$FLATPAK_OK" = 1 ] && [ "$ANDROID_OK" = 1 ] && [ "$IOS_OK" = 1 ]
