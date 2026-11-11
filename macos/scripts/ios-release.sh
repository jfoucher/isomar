#!/usr/bin/env bash
# Archives the iPhone and iPad app for the App Store and uploads it to App Store Connect, where it becomes a
# TestFlight build once Apple has processed it (adding it to a tester group stays manual). Run on a Mac signed
# in to the team's account in Xcode (scripts/release.sh does it over SSH).
#   BUILD_NUMBER=…        CFBundleVersion; default the UTC date and time as YYMMDD.HHMM, so every upload grows
#   UPLOAD=0              archive and export the .ipa only, into macos/DerivedData/Isomar-<version>/
#   ASC_KEY_PATH, ASC_KEY_ID, ASC_ISSUER_ID   an App Store Connect API key, instead of the Xcode account
# The app record (bundle id eu.6px.isomar) must exist in App Store Connect before the first upload.
set -euo pipefail
cd "$(dirname "$0")/../.."
export BUILD_NUMBER="${BUILD_NUMBER:-$(date -u +%y%m%d.%H%M)}"
VERSION=$(sed -n 's/^ *MARKETING_VERSION: *"\(.*\)"/\1/p' macos/project.yml | head -1)
echo "→ archiving Isomar for iOS $VERSION ($BUILD_NUMBER)" >&2
macos/scripts/ios-build.sh archive

cd macos
OUT="DerivedData/Isomar-$VERSION"
mkdir -p "$OUT"
AUTH=()
if [ -n "${ASC_KEY_PATH:-}" ]; then
  AUTH=(-authenticationKeyPath "$ASC_KEY_PATH" -authenticationKeyID "${ASC_KEY_ID:?}" -authenticationKeyIssuerID "${ASC_ISSUER_ID:?}")
fi
export_with() {   # export_with <destination: export|upload> <folder>
  local plist="$OUT/ExportOptions-$1.plist"
  cat >"$plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>method</key><string>app-store-connect</string>
  <key>destination</key><string>$1</string>
  <key>teamID</key><string>24G23598DF</string>
  <key>signingStyle</key><string>automatic</string>
  <key>manageAppVersionAndBuildNumber</key><false/>
  <key>uploadSymbols</key><true/>
</dict>
</plist>
PLIST
  rm -rf "$2"
  xcodebuild -exportArchive -archivePath DerivedData/IsomarTouch.xcarchive -exportOptionsPlist "$plist" \
    -exportPath "$2" -allowProvisioningUpdates ${AUTH[@]+"${AUTH[@]}"} -quiet
}

export_with export "$OUT/ios"
mv "$OUT/ios/Isomar.ipa" "$OUT/Isomar-$VERSION.ipa"
echo "→ $OUT/Isomar-$VERSION.ipa" >&2
if [ "${UPLOAD:-1}" = 1 ]; then
  echo "→ uploading to App Store Connect" >&2
  export_with upload "$OUT/upload"
  echo "→ uploaded $VERSION ($BUILD_NUMBER); it shows in TestFlight once Apple has processed it" >&2
fi
