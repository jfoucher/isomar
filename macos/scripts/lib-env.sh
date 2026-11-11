# Sourced by the scripts that build the Rust side on a Mac: where this Mac's Homebrew is, its architecture,
# and where cairo and pango come from for cargo.
#   BREW     /opt/homebrew on Apple silicon, /usr/local on Intel
#   ARCH     arm64 or x86_64, and TRIPLE the Rust target for it
#   LIBS     ~/dev/isomar-deps/prefix-<arch> when macos/deps/build-deps.sh built it there, else $BREW
# Homebrew no longer builds bottles for Intel Macs and its pango fails to build from source there, so an
# Intel Mac uses the prefix. gettext is not needed: isomar-core reads .mo files itself on macOS.
BREW=$( [ -x /opt/homebrew/bin/brew ] && echo /opt/homebrew || echo /usr/local )
export PATH="$HOME/.cargo/bin:$BREW/bin:$HOME/.local/bin:$PATH"
ARCH=$(uname -m)
case "$ARCH" in arm64) TRIPLE=aarch64-apple-darwin ;; *) TRIPLE=x86_64-apple-darwin ;; esac
DEPS="${DEPS_DIR:-$HOME/dev/isomar-deps}"
if [ -d "$DEPS/prefix-$ARCH" ]; then
  LIBS="$DEPS/prefix-$ARCH"
  # Only the prefix: pkg-config would otherwise also find Homebrew's glib and link it.
  export PKG_CONFIG_LIBDIR="$LIBS/lib/pkgconfig"
else
  LIBS="$BREW"
fi
export PKG_CONFIG_PATH="$LIBS/lib/pkgconfig:$LIBS/share/pkgconfig"
export LIBRARY_PATH="$LIBS/lib${LIBRARY_PATH:+:$LIBRARY_PATH}"
export CPATH="$LIBS/include${CPATH:+:$CPATH}"
