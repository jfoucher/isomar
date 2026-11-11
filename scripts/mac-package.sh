#!/usr/bin/env bash
# Builds the macOS app on a Mac over SSH and copies the zip and the disk image back to dist/.
# Usage: scripts/mac-package.sh      (SIGN_IDENTITY=... to choose the certificate)
set -euo pipefail
source "$(dirname "$0")/mac-lib.sh"
pick_mac
sync_to_mac
ensure_tools
echo "→ packaging on $MAC" >&2
ssh "$MAC" bash -s 2>&1 <<EOF | { grep -v 'tmux' || true; } | cut -c1-300
$(remote_prelude)
cd ~/$REMOTE_DIR
SIGN_IDENTITY="${SIGN_IDENTITY:-}" macos/scripts/package.sh
EOF
mkdir -p "$(dirname "$0")/../dist"
scp -q "$MAC:$REMOTE_DIR/macos/DerivedData/Package/*" "$(dirname "$0")/../dist/"
ls -la "$(dirname "$0")/../dist"
