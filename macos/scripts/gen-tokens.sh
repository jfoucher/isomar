#!/usr/bin/env bash
# Writes macos/Isomar/Generated/Tokens.swift from data/design/tokens.json. The file is only replaced when its
# content changes, so Xcode does not recompile it on every build. Run by build-rust.sh and, before the project
# is generated, by `xcodegen` (options.preGenCommand in project.yml), which needs the file to exist.
set -euo pipefail

cd "$(dirname "$0")/../.."
source macos/scripts/lib-env.sh
OUT=macos/Isomar/Generated/Tokens.swift
mkdir -p "$(dirname "$OUT")"
TMP=$(mktemp)
trap 'rm -f "$TMP"' EXIT
cargo xtask tokens --swift "$TMP"
cmp -s "$TMP" "$OUT" || cp "$TMP" "$OUT"
