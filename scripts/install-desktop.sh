#!/usr/bin/env bash
# Registers the build in this checkout with the desktop, for this user: the desktop file and the app icons
# in ~/.local/share, so that GNOME Shell shows Isomar's name and icon for a window of `cargo run`
# (it matches the window's app id, eu._6px.isomar, to a desktop file of that name) and the
# launcher starts target/release/isomar.
# Usage: scripts/install-desktop.sh [--debug] [--uninstall]   (--debug launches target/debug/isomar)
# An installed Flatpak of the same app id does this too; this one, in ~/.local/share, wins over it.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
APP_ID=eu._6px.isomar
DATA=${XDG_DATA_HOME:-$HOME/.local/share}
PROFILE=release
UNINSTALL=0
for arg in "$@"; do
    case $arg in
        --debug) PROFILE=debug ;;
        --uninstall) UNINSTALL=1 ;;
        *) echo "usage: $0 [--debug] [--uninstall]" >&2; exit 2 ;;
    esac
done

DESKTOP=$DATA/applications/$APP_ID.desktop
ICON=$DATA/icons/hicolor/scalable/apps/$APP_ID.svg
SYMBOLIC=$DATA/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg

if [ $UNINSTALL = 1 ]; then
    rm -f "$DESKTOP" "$ICON" "$SYMBOLIC"
    echo "removed $DESKTOP and the icons"
else
    install -Dm644 "$ROOT/data/icons/scalable/apps/$APP_ID.svg" "$ICON"
    install -Dm644 "$ROOT/data/icons/symbolic/apps/$APP_ID-symbolic.svg" "$SYMBOLIC"
    mkdir -p "$(dirname "$DESKTOP")"
    # The checkout's path may hold spaces: quote it as the desktop entry spec wants.
    sed "s|^Exec=.*|Exec=\"$ROOT/target/$PROFILE/isomar\"|" "$ROOT/data/$APP_ID.desktop" > "$DESKTOP"
    echo "installed $DESKTOP (runs target/$PROFILE/isomar) and the icons"
    [ -x "$ROOT/target/$PROFILE/isomar" ] || echo "note: target/$PROFILE/isomar is not built yet"
fi

# Both are optional: GNOME Shell watches these folders, the caches only speed lookups up.
command -v update-desktop-database >/dev/null && update-desktop-database -q "$DATA/applications" || true
[ -f "$DATA/icons/hicolor/index.theme" ] && command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -qtf "$DATA/icons/hicolor" || true
