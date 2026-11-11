# Sourced by the scripts/mac-*.sh scripts: which Mac to build on, and what it needs.
#
# macOS work is built over SSH on one of two Macs (there is no Xcode on the Linux box), and
# either may be asleep. The first in $MACS that answers SSH wins; `MAC=user@host` pins one
# and skips the probe. Modelled on ~/development/workout/bin/mac-lib.sh.

MACS="${MACS:-jonathan@100.68.219.52 jonathan@100.74.8.70}"
REMOTE_DIR="${REMOTE_DIR:-dev/isomar}"          # relative to $HOME on the Mac
# The repository, resolved now: a relative path would go stale after a cd. zsh has no
# BASH_SOURCE, but there $0 is this file while it is being sourced.
ISOMAR_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"

pick_mac() {
  if [ -n "${MAC:-}" ]; then return; fi
  local candidate
  for candidate in $MACS; do
    # BatchMode so a host that wants a password counts as not answering rather than
    # stopping at a prompt; stderr dropped for the Mac's harmless tmux complaint.
    if ssh -o ConnectTimeout=5 -o BatchMode=yes "$candidate" true 2>/dev/null; then
      MAC="$candidate"
      echo "→ using $MAC" >&2
      return
    fi
    echo "→ $candidate is not answering" >&2
  done
  echo "!! no Mac answered ($MACS)" >&2
  exit 1
}

# Copies the repository to the Mac. Chart data, build output, packages, generated projects and
# the design handoff (mockups the build never reads) stay behind. Excluded paths are also kept
# on the Mac: --delete leaves them alone.
sync_to_mac() {
  # Sourced from zsh, "$(dirname "${BASH_SOURCE[0]}")/../" was once the parent of the repository,
  # and --delete mirrored all of ~/development into the Mac's copy: never sync anything else.
  local root="$ISOMAR_ROOT"
  if [ ! -f "$root/macos/project.yml" ] || [ ! -f "$root/crates/isomar-app/Cargo.toml" ]; then
    echo "!! $root is not the Isomar repository; not syncing" >&2
    exit 1
  fi
  echo "→ syncing $root to $MAC:$REMOTE_DIR" >&2
  ssh "$MAC" "mkdir -p ~/$REMOTE_DIR" 2>/dev/null
  rsync -az --delete \
    --exclude '/.git' --exclude '/target' --exclude '/testdata' --exclude '/dist' --exclude '/design' --exclude '/.flatpak' --exclude '/.flatpak-builder' \
    --exclude 'macos/DerivedData' --exclude '*.xcodeproj' --exclude 'macos/Isomar/Generated' \
    --exclude '/android/.gradle' --exclude '/android/build' --exclude '/android/app/build' --exclude '/android/app/src/main/jniLibs' \
    "$root/" "$MAC:$REMOTE_DIR/"
}

# Lines to put at the top of a script run on the Mac: the tools live in places a
# non-interactive SSH shell does not have on its PATH.
remote_prelude() {
  cat <<'PRE'
set -euo pipefail
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$PATH"
PRE
}

# Checks what the Mac needs to build, and adds the Rust target for its architecture when it is missing.
# Homebrew packages are only reported, not installed: that is the owner's machine.
ensure_tools() {
  ssh "$MAC" bash -s 2>&1 <<EOF | { grep -v 'tmux' || true; }
$(remote_prelude)
cd ~/$REMOTE_DIR
source macos/scripts/lib-env.sh
missing=""
for tool in pkgconf xcodegen; do
  command -v "\$tool" >/dev/null || missing="\$missing \$tool"
done
# cairo and pango come from Homebrew only when macos/deps has not built them for this architecture.
if [ "\$LIBS" = "\$BREW" ]; then
  for pkg in cairo pango; do
    brew list --formula "\$pkg" >/dev/null 2>&1 || missing="\$missing \$pkg"
  done
fi
if [ -n "\$missing" ]; then
  echo "!! missing:\$missing (brew install\$missing, or macos/deps/build-deps.sh \$ARCH for cairo and pango)" >&2
  exit 1
fi
rustup target list --installed | grep -q "^\$TRIPLE\\\$" || rustup target add "\$TRIPLE"
EOF
}
