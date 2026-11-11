#!/usr/bin/env bash
# Builds Isomar as a Flatpak and writes a single-file bundle, dist/Isomar-<version>.flatpak.
# Usage: scripts/flatpak.sh [--install]     (--install also installs it for this user)
# Needs flatpak and Flathub; installs the GNOME SDK, the Rust extension and flatpak-builder
# for this user when they are missing. The crates are vendored into .flatpak/vendor so the
# build itself runs offline.
set -euo pipefail
cd "$(dirname "$0")/.."

APP_ID=eu._6px.isomar
MANIFEST=build-aux/flatpak/$APP_ID.yml
RUNTIME_VERSION=$(sed -n "s/^runtime-version: '\(.*\)'/\1/p" "$MANIFEST")
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
INSTALL=0
[ "${1:-}" = --install ] && INSTALL=1

flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo

need() {   # need <ref>: install it for this user unless it is installed somewhere
    flatpak info "$1" >/dev/null 2>&1 || flatpak install --user -y --noninteractive flathub "$1"
}
need "org.gnome.Platform//$RUNTIME_VERSION"
need "org.gnome.Sdk//$RUNTIME_VERSION"
# The Rust extension follows the freedesktop SDK the GNOME one is built on.
FDO=$(flatpak info -m "org.gnome.Sdk//$RUNTIME_VERSION" | sed -n 's/^versions *= *\([^;]*\).*/\1/p' | head -1)
need "org.freedesktop.Sdk.Extension.rust-stable//${FDO:?cannot tell which freedesktop SDK GNOME $RUNTIME_VERSION uses}"

if command -v flatpak-builder >/dev/null; then
    BUILDER=(flatpak-builder)
else
    need org.flatpak.Builder
    BUILDER=(flatpak run --no-documents-portal org.flatpak.Builder)
fi

echo "→ vendoring the crates" >&2
mkdir -p .flatpak
cargo vendor --locked --versioned-dirs .flatpak/vendor >/dev/null

# ostree will not write while less than 3% of the disk is free (39 GB of a 1.3 TB disk);
# this repo only feeds the bundle, so it may use the space there is.
if command -v ostree >/dev/null; then
    [ -d .flatpak/repo ] || ostree init --repo=.flatpak/repo --mode=archive-z2
    ostree config --repo=.flatpak/repo set core.min-free-space-size 1GB
fi

echo "→ building $APP_ID $VERSION" >&2
"${BUILDER[@]}" --user --force-clean --disable-rofiles-fuse \
    --state-dir=.flatpak/state --repo=.flatpak/repo \
    .flatpak/build "$MANIFEST"

mkdir -p dist
BUNDLE=dist/Isomar-$VERSION.flatpak
flatpak build-bundle --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo .flatpak/repo "$BUNDLE" "$APP_ID"
echo "→ $BUNDLE ($(du -h "$BUNDLE" | cut -f1))" >&2

if [ "$INSTALL" = 1 ]; then
    flatpak install --user -y --reinstall "$BUNDLE"
    echo "→ installed; run it with: flatpak run $APP_ID" >&2
fi
