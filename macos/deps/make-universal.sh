#!/usr/bin/env bash
# Joins the arm64 and x86_64 builds of the libraries (macos/deps/build-deps.sh) into universal ones in
# ~/dev/isomar-deps/universal/lib, named @rpath/<name> so that the app finds them in its Frameworks folder.
set -euo pipefail
export PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
ROOT="${DEPS_DIR:-$HOME/dev/isomar-deps}"
OUT="$ROOT/universal/lib"
rm -rf "$ROOT/universal"
mkdir -p "$OUT"
for f in "$ROOT"/prefix-arm64/lib/*.dylib; do
  [ -L "$f" ] && continue
  name=$(basename "$f")
  other="$ROOT/prefix-x86_64/lib/$name"
  [ -f "$other" ] || { echo "!! $name has no x86_64 build" >&2; exit 1; }
  lipo -create "$f" "$other" -output "$OUT/$name"
  install_name_tool -id "@rpath/$name" "$OUT/$name"
  # What it needs from the other libraries is found beside it.
  for dep in $(otool -L "$OUT/$name" | tail -n +2 | awk '{print $1}' | grep "^$ROOT/prefix-" || true); do
    install_name_tool -change "$dep" "@rpath/$(basename "$dep")" "$OUT/$name"
  done
done
# The unversioned names the linker looks for (libcairo.dylib -> libcairo.2.dylib).
for f in "$ROOT"/prefix-arm64/lib/*.dylib; do
  if [ -L "$f" ]; then ln -sf "$(basename "$(readlink "$f")")" "$OUT/$(basename "$f")"; fi
done
echo "→ $OUT"
for f in "$OUT"/*.dylib; do [ -L "$f" ] || printf '%s: %s\n' "$(basename "$f")" "$(lipo -archs "$f")"; done
echo "oldest macOS each library runs on (13.0 or lower is wanted):"
for f in "$OUT"/*.dylib; do [ -L "$f" ] || vtool -show-build "$f" | awk '/minos/ {print $2}'; done | sort -u
