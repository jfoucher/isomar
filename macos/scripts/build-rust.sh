#!/usr/bin/env bash
# Builds the Rust static library for the app. Run on the Mac, from anywhere.
# Usage: macos/scripts/build-rust.sh [debug|release]   (release by default)
set -euo pipefail

cd "$(dirname "$0")/../.."
source macos/scripts/lib-env.sh
PROFILE=${1:-release}
export MACOSX_DEPLOYMENT_TARGET=13.0
if [ "$PROFILE" = release ]; then FLAG=--release; else FLAG=; fi

if [ -d "$DEPS/prefix-x86_64" ] && [ -d "$DEPS/prefix-arm64" ]; then
  # Universal: the library is built for each architecture against that architecture's own cairo and pango
  # (built for macOS 13, see macos/deps), and the two are joined.
  for pair in "aarch64-apple-darwin:arm64" "x86_64-apple-darwin:x86_64"; do
    triple=${pair%%:*}; arch=${pair##*:}
    rustup target add "$triple" >/dev/null 2>&1 || true
    PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_PATH="$DEPS/prefix-$arch/lib/pkgconfig" PKG_CONFIG_LIBDIR="$DEPS/prefix-$arch/lib/pkgconfig" \
      LIBRARY_PATH="$DEPS/prefix-$arch/lib" CPATH="$DEPS/prefix-$arch/include" \
      cargo build $FLAG -p isomar-ffi --target "$triple"
  done
  mkdir -p target/universal/$PROFILE
  lipo -create target/aarch64-apple-darwin/$PROFILE/libisomar_ffi.a target/x86_64-apple-darwin/$PROFILE/libisomar_ffi.a -output target/universal/$PROFILE/libisomar_ffi.a
  echo "universal: $(lipo -archs target/universal/$PROFILE/libisomar_ffi.a)"
else
  # Only for this Mac's architecture, against its prefix or Homebrew's libraries (lib-env.sh).
  rustup target add "$TRIPLE" >/dev/null 2>&1 || true
  cargo build $FLAG -p isomar-ffi --target "$TRIPLE"
  mkdir -p target/universal/$PROFILE
  cp target/$TRIPLE/$PROFILE/libisomar_ffi.a target/universal/$PROFILE/libisomar_ffi.a
fi
macos/scripts/gen-tokens.sh
