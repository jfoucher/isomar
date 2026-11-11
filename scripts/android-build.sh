#!/usr/bin/env bash
# Builds the Android app: the engine (crates/isomar-android, a JNI library over isomar-ffi) for each ABI against
# the static cairo and pango of scripts/deps/build-mobile-deps.sh, then the APK with Gradle.
#   scripts/android-build.sh [debug|release] [abi...]      ABIs: arm64-v8a (default), x86_64 (the emulator)
# The APK is in android/app/build/outputs/apk/<profile>/. The engine is built with cargo's release profile
# either way: a debug one is 240 MB and slow to draw.
set -euo pipefail
cd "$(dirname "$0")/.."
PROFILE=${1:-debug}
shift || true
ABIS=("${@:-arm64-v8a}")
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$(ls -d "$ANDROID_HOME"/ndk/* | sort -V | tail -1)}"
DEPS="${DEPS_DIR:-$HOME/dev/isomar-deps}"
# Gradle needs a JDK with javac (Fedora's java-*-openjdk packages are runtimes only), 17 or newer.
if [ ! -x "${JAVA_HOME:-/nonexistent}/bin/javac" ]; then
  for j in "$HOME"/.jdks/openjdk-21* "$HOME"/.jdks/temurin-17* /usr/lib/jvm/java-21-openjdk /usr/lib/jvm/java-17-openjdk; do
    if [ -x "$j/bin/javac" ]; then export JAVA_HOME="$j"; break; fi
  done
fi

for abi in "${ABIS[@]}"; do
  case "$abi" in arm64-v8a) p=android-arm64 ;; x86_64) p=android-x86_64 ;; *) echo "unknown ABI $abi" >&2; exit 2 ;; esac
  prefix="$DEPS/prefix-$p"
  [ -d "$prefix" ] || { echo "!! $prefix is missing: run scripts/deps/build-mobile-deps.sh $p" >&2; exit 1; }
  # Only the prefix's libraries, linked statically into the one .so.
  PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_ALL_STATIC=1 PKG_CONFIG_PATH="" PKG_CONFIG_LIBDIR="$prefix/lib/pkgconfig" \
    cargo ndk -t "$abi" -P 28 build --release -p isomar-android
  # Only the engine: cargo-ndk's -o would also copy every other .so in the target folder.
  triple=$( [ "$abi" = arm64-v8a ] && echo aarch64-linux-android || echo x86_64-linux-android )
  mkdir -p "android/app/src/main/jniLibs/$abi"
  cp "target/$triple/release/libisomar_android.so" "android/app/src/main/jniLibs/$abi/"
done

mkdir -p android/app/src/main/assets/fonts
cp macos/Isomar/Resources/Fonts/*.ttf macos/Isomar/Resources/Fonts/OFL-NOTICE.txt android/app/src/main/assets/fonts/
mkdir -p android/app/src/main/assets/sounds
cp macos/Isomar/Resources/Sounds/*.wav android/app/src/main/assets/sounds/
# The design tokens as Kotlin (gitignored, like the Swift ones the Mac build makes).
mkdir -p android/app/src/main/java/io/github/isomar/ui
cargo xtask tokens --kotlin android/app/src/main/java/io/github/isomar/ui/Tokens.kt
cd android
if [ "$PROFILE" = release ]; then ./gradlew --quiet assembleRelease; else ./gradlew --quiet assembleDebug; fi
ls -l app/build/outputs/apk/"$PROFILE"/*.apk
