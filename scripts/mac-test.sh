#!/usr/bin/env bash
# Runs the toolkit-free crates' tests on the Mac (the GTK app cannot build there).
# Usage: scripts/mac-test.sh [cargo test arguments...]
set -euo pipefail

source "$(dirname "$0")/mac-lib.sh"
pick_mac
sync_to_mac
ensure_tools

echo "→ cargo test on $MAC" >&2
# The arguments are quoted here so the remote shell sees them as given.
# (With none, printf would still print '', and the Swift test below would never run.)
ARGS=""
[ $# -eq 0 ] || ARGS=$(printf '%q ' "$@")
ssh "$MAC" bash -s 2>&1 <<EOF | { grep -v 'tmux' || true; }
$(remote_prelude)
cd ~/$REMOTE_DIR
# cairo and pango come through pkg-config, from ~/dev/isomar-deps or Homebrew.
source macos/scripts/lib-env.sh
cargo test -p isomar-core -p isomar-app $ARGS
if [ -z "$ARGS" ]; then
  # The Swift side: a smoke test of the engine through its C interface (the Rust library is built by a phase).
  cd macos
  xcodegen generate --quiet
  xcodebuild -project Isomar.xcodeproj -scheme Isomar -configuration Debug -derivedDataPath DerivedData test 2>&1 | grep -E "error:|Executed|TEST (SUCCEEDED|FAILED)" | sort -u || true
fi
EOF
