#!/usr/bin/env bash
# Builds cairo, pango and what they need (glib, harfbuzz, fribidi, freetype, fontconfig, expat, pixman, libpng,
# zlib, libffi, pcre2) from source as static libraries for a phone or tablet, the way macos/deps/build-deps.sh
# builds them for macOS: pango's meson build fetches and builds the rest (--wrap-mode forcefallback).
#   scripts/deps/build-mobile-deps.sh android-arm64|android-x86_64     on Linux, with the Android NDK
#   scripts/deps/build-mobile-deps.sh ios|ios-sim|ios-sim-x86_64       on a Mac, with Xcode (arm64; the last for an
#                                                                      Intel Mac's simulator)
# The result is in ${DEPS_DIR:-~/dev/isomar-deps}/prefix-<platform>, with pkg-config files for cargo.
#
# Static, because an iOS app may not carry its own dylibs and an Android one is simplest as one .so; no quartz
# or CoreText (pango's CoreText backend is for macOS only), so text goes through fontconfig and freetype, with a
# fonts.conf the shell writes at start-up.
set -euo pipefail
PLATFORM=${1:?android-arm64, android-x86_64, ios, ios-sim or ios-sim-x86_64}
ROOT="${DEPS_DIR:-$HOME/dev/isomar-deps}"
PANGO=1.57.0
SRC="$ROOT/src/pango-$PANGO"
PREFIX="$ROOT/prefix-$PLATFORM"
BUILD="$ROOT/build-$PLATFORM"
CROSS="$ROOT/cross-$PLATFORM.ini"
IOS_MIN=16.0
ANDROID_API=28   # bionic has iconv from API 28, which glib needs
export PATH="$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
mkdir -p "$ROOT/src"
if [ ! -d "$SRC" ]; then
  curl -sL -o "$ROOT/src/pango.tar.xz" "https://download.gnome.org/sources/pango/1.57/pango-$PANGO.tar.xz"
  tar -xf "$ROOT/src/pango.tar.xz" -C "$ROOT/src"
fi
# The pango-view demo needs cairo headers in a place its build does not look, and the app has no use for it.
sed -i.bak "s/^subdir(.utils.)/# subdir('utils')/" "$SRC/meson.build"
# Pango looks for CoreText and ApplicationServices on any darwin; iOS has no ApplicationServices, and pango's
# CoreText backend is for macOS only. A macOS build of the same sources is unchanged.
sed -i.bak2 "s/^if host_system == 'darwin'\$/if host_system == 'darwin' and host_machine.subsystem() == 'macos'/" "$SRC/meson.build"

# Nothing from the build machine: pkg-config would find the host's glib and link it.
export PKG_CONFIG_PATH=""
export PKG_CONFIG_LIBDIR="$PREFIX/lib/pkgconfig"
unset CPATH LIBRARY_PATH
PKGCONF=$(command -v pkgconf || command -v pkg-config)

case "$PLATFORM" in
  android-*)
    NDK="${ANDROID_NDK_HOME:-$(ls -d "$HOME"/Android/Sdk/ndk/* | sort -V | tail -1)}"
    BIN="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin"
    case "$PLATFORM" in android-arm64) TRIPLE=aarch64-linux-android; CPU=aarch64 ;; *) TRIPLE=x86_64-linux-android; CPU=x86_64 ;; esac
    cat > "$CROSS" <<EOF
[binaries]
c = '$BIN/$TRIPLE$ANDROID_API-clang'
cpp = '$BIN/$TRIPLE$ANDROID_API-clang++'
ar = '$BIN/llvm-ar'
strip = '$BIN/llvm-strip'
ranlib = '$BIN/llvm-ranlib'
pkg-config = '$PKGCONF'

[built-in options]
c_args = ['-fPIC']
cpp_args = ['-fPIC']
# Google Play wants native code aligned for 16 KB pages.
c_link_args = ['-Wl,-z,max-page-size=16384']
cpp_link_args = ['-Wl,-z,max-page-size=16384']

[host_machine]
system = 'android'
cpu_family = '$CPU'
cpu = '$CPU'
endian = 'little'

[properties]
needs_exe_wrapper = true
EOF
    ;;
  ios|ios-sim|ios-sim-x86_64)
    if [ "$PLATFORM" = ios ]; then SDK=iphoneos; SUB=ios; MIN="-miphoneos-version-min=$IOS_MIN"
    else SDK=iphonesimulator; SUB=ios-simulator; MIN="-mios-simulator-version-min=$IOS_MIN"; fi
    # An Intel Mac's simulator runs only x86_64 apps.
    if [ "$PLATFORM" = ios-sim-x86_64 ]; then ARCH=x86_64; CPU=x86_64; else ARCH=arm64; CPU=aarch64; fi
    SYSROOT=$(xcrun --sdk $SDK --show-sdk-path)
    FLAGS="'-arch', '$ARCH', '-isysroot', '$SYSROOT', '$MIN'"
    cat > "$CROSS" <<EOF
[binaries]
c = ['clang', $FLAGS]
cpp = ['clang++', $FLAGS]
objc = ['clang', $FLAGS]
ar = 'ar'
strip = 'strip'
pkg-config = '$PKGCONF'

[built-in options]
c_args = [$FLAGS]
c_link_args = [$FLAGS]
cpp_args = [$FLAGS]
cpp_link_args = [$FLAGS]
objc_args = [$FLAGS]
objc_link_args = [$FLAGS]

[host_machine]
system = 'darwin'
subsystem = '$SUB'
cpu_family = '$CPU'
cpu = '$ARCH'
endian = 'little'

[properties]
needs_exe_wrapper = true
# libSystem has the symbol, so meson's link test finds it, but the iOS headers do not declare it.
has_function_pipe2 = false
EOF
    ;;
  *) echo "unknown platform $PLATFORM" >&2; exit 2 ;;
esac

rm -rf "$BUILD"
cd "$SRC"
meson setup "$BUILD" --cross-file "$CROSS" --prefix "$PREFIX" --libdir lib \
  --buildtype release --default-library static --wrap-mode forcefallback \
  -Dintrospection=disabled -Dgtk_doc=false -Dbuild-testsuite=false -Dbuild-examples=false -Dfontconfig=enabled \
  -Dglib:tests=false -Dglib:introspection=disabled -Dglib:glib_debug=disabled -Dglib:selinux=disabled \
  -Dglib:libmount=disabled -Dglib:nls=disabled -Dglib:documentation=false -Dglib:man-pages=disabled -Dglib:xattr=false \
  -Dcairo:tests=disabled -Dcairo:xlib=disabled -Dcairo:xcb=disabled -Dcairo:quartz=disabled \
  -Dcairo:freetype=enabled -Dcairo:fontconfig=enabled -Dcairo:png=enabled -Dcairo:zlib=enabled \
  -Dharfbuzz:tests=disabled -Dharfbuzz:docs=disabled -Dharfbuzz:introspection=disabled -Dharfbuzz:icu=disabled -Dharfbuzz:coretext=disabled \
  -Dfribidi:docs=false -Dfribidi:tests=false \
  -Dfontconfig:tests=disabled -Dfontconfig:doc=disabled -Dfontconfig:tools=disabled \
  -Dfreetype2:harfbuzz=disabled -Dfreetype2:png=enabled -Dfreetype2:zlib=enabled \
  -Dpixman:tests=disabled -Dpixman:demos=disabled -Dpixman:gtk=disabled -Dpixman:openmp=disabled \
  -Dlibffi:exe_static_tramp=false
# libffi's static trampolines want open_temp_exec_file, which it does not build for Android; glib's
# closures work without them. glib calls _NSGetExecutablePath on any Apple system but includes its header
# only through macOS headers: on iOS it is undeclared. (The sources exist once meson setup has fetched them.)
GUTILS="$SRC/subprojects/glib/glib/gutils.c"
grep -q 'mach-o/dyld.h' "$GUTILS" || perl -0pi -e 's/(#include "glib-init.h")/#ifdef __APPLE__\n#include <mach-o\/dyld.h>\n#endif\n$1/' "$GUTILS"
meson compile -C "$BUILD"
meson install -C "$BUILD"
echo "→ $PREFIX"
