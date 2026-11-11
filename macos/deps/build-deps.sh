#!/usr/bin/env bash
# Builds cairo, pango and what they need (glib, harfbuzz, fribidi, freetype, fontconfig, pixman, libpng, ...)
# from source for one architecture, for macOS 13 and up, with no Homebrew library involved, so that the
# libraries in the app are the same on every Mac it runs on. Pango's meson build fetches and builds the rest
# (sources from download.gnome.org, gitlab.gnome.org and the meson wrap database).
#   macos/deps/build-deps.sh arm64|x86_64     the result is in ~/dev/isomar-deps/prefix-<arch>
# macos/deps/make-universal.sh then joins the two into universal libraries.
set -euo pipefail
# Homebrew is in /opt/homebrew on Apple silicon and /usr/local on Intel; meson may be a pip --user install.
BREW=$( [ -x /opt/homebrew/bin/brew ] && echo /opt/homebrew || echo /usr/local )
export PATH="$BREW/bin:$(echo "$HOME"/Library/Python/*/bin | tr ' ' ':'):/usr/bin:/bin:/usr/sbin:/sbin"
ARCH=${1:?arm64 or x86_64}
TARGET=13.0
ROOT="${DEPS_DIR:-$HOME/dev/isomar-deps}"
PANGO=1.57.0
SRC="$ROOT/src/pango-$PANGO"
PREFIX="$ROOT/prefix-$ARCH"
BUILD="$ROOT/build-$ARCH"
mkdir -p "$ROOT/src"
if [ ! -d "$SRC" ]; then
  curl -sL -o "$ROOT/src/pango.tar.xz" "https://download.gnome.org/sources/pango/1.57/pango-$PANGO.tar.xz"
  tar -xf "$ROOT/src/pango.tar.xz" -C "$ROOT/src"
fi

# The pango-view demo needs cairo headers in a place its build does not look, and the app has no use for it.
sed -i.bak "s/^subdir(.utils.)/# subdir('utils')/" "$SRC/meson.build"

export MACOSX_DEPLOYMENT_TARGET=$TARGET
# Nothing from Homebrew: it would be found by pkg-config and linked, and it is built for the newest macOS.
export PKG_CONFIG_PATH=""
export PKG_CONFIG_LIBDIR="$PREFIX/lib/pkgconfig"
unset CPATH LIBRARY_PATH

CROSS="$ROOT/cross-$ARCH.ini"
# Rosetta runs x86_64 programs on an Apple silicon Mac, so the build may run what it builds there; an Intel Mac
# cannot run arm64 ones, and glib's gnulib checks then take their known answers for darwin instead.
if [ "$ARCH" = arm64 ] && [ "$(uname -m)" != arm64 ]; then NEEDS_WRAPPER=true; else NEEDS_WRAPPER=false; fi
cat > "$CROSS" <<EOF
[binaries]
c = ['clang', '-arch', '$ARCH']
cpp = ['clang++', '-arch', '$ARCH']
objc = ['clang', '-arch', '$ARCH']
ar = 'ar'
strip = 'strip'
pkg-config = '$BREW/bin/pkgconf'

[built-in options]
c_args = ['-arch', '$ARCH', '-mmacosx-version-min=$TARGET']
c_link_args = ['-arch', '$ARCH', '-mmacosx-version-min=$TARGET']
cpp_args = ['-arch', '$ARCH', '-mmacosx-version-min=$TARGET']
cpp_link_args = ['-arch', '$ARCH', '-mmacosx-version-min=$TARGET']
objc_args = ['-arch', '$ARCH', '-mmacosx-version-min=$TARGET']
objc_link_args = ['-arch', '$ARCH', '-mmacosx-version-min=$TARGET']

[host_machine]
system = 'darwin'
subsystem = 'macos'
cpu_family = '$( [ "$ARCH" = arm64 ] && echo aarch64 || echo x86_64 )'
cpu = '$ARCH'
endian = 'little'

[properties]
needs_exe_wrapper = $NEEDS_WRAPPER
EOF

rm -rf "$BUILD"
cd "$SRC"
meson setup "$BUILD" --cross-file "$CROSS" --prefix "$PREFIX" --libdir lib \
  --buildtype release --default-library shared --wrap-mode forcefallback \
  -Dintrospection=disabled -Dgtk_doc=false -Dbuild-testsuite=false -Dbuild-examples=false -Dfontconfig=enabled \
  -Dglib:tests=false -Dglib:introspection=disabled -Dglib:glib_debug=disabled -Dglib:selinux=disabled \
  -Dglib:libmount=disabled -Dglib:nls=disabled -Dglib:documentation=false -Dglib:man-pages=disabled \
  -Dcairo:tests=disabled -Dcairo:xlib=disabled -Dcairo:xcb=disabled -Dcairo:quartz=enabled \
  -Dcairo:freetype=enabled -Dcairo:fontconfig=enabled -Dcairo:png=enabled -Dcairo:zlib=enabled \
  -Dharfbuzz:tests=disabled -Dharfbuzz:docs=disabled -Dharfbuzz:introspection=disabled -Dharfbuzz:icu=disabled -Dharfbuzz:coretext=enabled \
  -Dfribidi:docs=false -Dfribidi:tests=false \
  -Dfontconfig:tests=disabled -Dfontconfig:doc=disabled -Dfontconfig:tools=disabled \
  -Dfreetype2:harfbuzz=disabled -Dfreetype2:png=enabled -Dfreetype2:zlib=enabled \
  -Dpixman:tests=disabled -Dpixman:demos=disabled -Dpixman:gtk=disabled -Dpixman:openmp=disabled
meson compile -C "$BUILD"
meson install -C "$BUILD"
echo "→ $PREFIX"
