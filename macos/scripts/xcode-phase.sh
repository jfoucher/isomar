#!/usr/bin/env bash
# The two Run Script phases of the Isomar target (macos/project.yml), so that Run in Xcode builds everything:
#   rust     before compiling: the Rust static library (build-rust.sh, which also writes Tokens.swift)
#   bundle   after linking: copies cairo, pango and what they need into Isomar.app/Contents/Frameworks
# Xcode's environment (SDKROOT, ARCHS, DEVELOPER_DIR, ...) is dropped: it breaks cargo's C builds, and
# lib-env.sh sets up the PATH that Xcode lacks (cargo, Homebrew).
set -euo pipefail

SCRIPTS="$(cd "$(dirname "$0")" && pwd)"
CLEAN=(env -i HOME="$HOME" USER="${USER:-$(id -un)}" PATH=/usr/bin:/bin:/usr/sbin:/sbin)

case "${1:-}" in
  rust)
    # Release even for Debug builds: the chart renderer is far too slow unoptimised. cargo does nothing
    # when nothing changed.
    "${CLEAN[@]}" bash -c 'source "$0/lib-env.sh"; command -v cargo >/dev/null || { echo "error: cargo not found; install Rust with rustup" >&2; exit 1; }; exec "$0/build-rust.sh" release' "$SCRIPTS"
    ;;
  bundle)
    APP="$TARGET_BUILD_DIR/$FULL_PRODUCT_NAME"
    [ -d "$APP" ] || { echo "error: $APP does not exist" >&2; exit 1; }
    # Signed ad hoc here; Xcode signs the app again with its own identity after this phase.
    "${CLEAN[@]}" bash -c 'source "$0/lib-env.sh"; exec "$0/bundle-dylibs.sh" "$1"' "$SCRIPTS" "$APP"
    ;;
  *)
    echo "usage: xcode-phase.sh rust|bundle" >&2
    exit 2
    ;;
esac
