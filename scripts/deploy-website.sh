#!/usr/bin/env bash
# Publishes website/ to https://isomar.marinetechs.com.
#
# Links every download the page names (downloads/Isomar-<version>.flatpak|dmg|zip|apk) to the release in
# dist/<version>/ (scripts/release.sh builds it), then runs the server repo's
# sites/isomar.marinetechs.com/migrate/publish.sh, which copies website/ into the site's
# data/public (symlinks followed), rsyncs it to the server and reloads nginx there.
#
# Usage: scripts/deploy-website.sh [--local]
#   --local                   only build data/public (test with `docker compose up -d` there,
#                             http://localhost:8098)
#   TARGET=deploy@144.76.104.209   the server (SSH port 2543, SSH_PORT to change)
#   SERVER_REPO=~/Sites/server     where the server configuration lives
#   ALLOW_MISSING=1                publish even if a linked download or the TestFlight link is missing
set -euo pipefail
cd "$(dirname "$0")/.."

SERVER_REPO=${SERVER_REPO:-$HOME/Sites/server}
TARGET=${TARGET:-deploy@144.76.104.209}
PUBLISH="$SERVER_REPO/sites/isomar.marinetechs.com/migrate/publish.sh"
[ -x "$PUBLISH" ] || { echo "!! no $PUBLISH" >&2; exit 1; }

LOCAL=0
case "${1:-}" in
  --local) LOCAL=1 ;;
  "") ;;
  *) sed -n '2,15p' "$0" >&2; exit 1 ;;
esac

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
mapfile -t LINKS < <(grep -oE 'href="downloads/[^"]+"' website/index.html | sed 's/^href="//; s/"$//' | sort -u)

# The page's download links name the version by hand: say so when they lag the release.
for f in "${LINKS[@]}"; do
  case "$f" in
    *"-$VERSION."*) ;;
    *) echo "!! website/index.html links $f, but Cargo.toml says $VERSION" >&2 ;;
  esac
done

# The iPhone and iPad app is on TestFlight, not a download: its button must hold Isomar's public link.
if grep -q 'testflight.apple.com/join/PLACEHOLDER' website/index.html && [ "${ALLOW_MISSING:-}" != 1 ]; then
  echo "!! website/index.html still links testflight.apple.com/join/PLACEHOLDER: put in the public TestFlight link" >&2
  #exit 1
fi

# Each download is a symlink into dist/<its version>/ (website/downloads/ is gitignored).
mkdir -p website/downloads
find website/downloads -maxdepth 1 -type l -delete
missing=0
for f in "${LINKS[@]}"; do
  name=${f#downloads/}
  v=$(sed -n 's/^Isomar-\(.*\)\.\(flatpak\|dmg\|zip\|apk\)$/\1/p' <<<"$name")
  src="dist/$v/$name"
  if [ -n "$v" ] && [ -f "$src" ]; then
    ln -s "../../$src" "website/downloads/$name"
    echo "→ $name ($(du -h "$src" | cut -f1))"
  elif [ ! -f "website/downloads/$name" ]; then
    echo "!! missing: $src" >&2
    missing=1
  fi
done
if [ $missing -eq 1 ] && [ "${ALLOW_MISSING:-}" != 1 ]; then
  echo "!! build them with scripts/release.sh, or ALLOW_MISSING=1 to publish anyway" >&2
  exit 1
fi

# Cloudflare and browsers keep style.css, site.js, the fonts and the screenshots for a month,
# so every link to one of them gets ?v=<hash of its contents>: a changed file is a new
# address. This is done in a staging copy (symlinks to website/, rewritten files copied), so
# website/ stays as committed. style.css is hashed last, after its font links are rewritten.
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
chmod 755 "$STAGE"   # rsync -a gives the site's root its mode: mktemp's 700 made nginx answer 403
cp -rs "$PWD/website/." "$STAGE/"
for page in index.html style.css; do
  rm "$STAGE/$page"
  cp "website/$page" "$STAGE/$page"
done
# The Shom tidal atlases (website/atlases, written by scripts/gen-tidal-atlas.py, not committed) go up as
# they are: the app fetches them by name, never through the page.
if [ -f website/atlases/index.json ]; then
  echo "→ atlases: $(grep -c '"id"' website/atlases/index.json) zones, $(du -sh website/atlases | cut -f1)"
else
  echo "!! no website/atlases: the app's Shom atlases will not be found (scripts/gen-tidal-atlas.py)" >&2
fi
mapfile -t ASSETS < <(cd website && find . \( -path ./downloads -o -path ./atlases \) -prune -o -type f \
  ! -name index.html ! -name style.css ! -name '.*' ! -name '*.txt' -print | sed 's#^\./##' | sort)
for rel in "${ASSETS[@]}" style.css; do
  hash=$(sha256sum "$STAGE/$rel" | cut -c1-10)
  re=$(sed 's/[.]/\\./g' <<<"$rel")
  # Only a relative link (after a quote or "url("), not https://…/img/… in the meta tags.
  sed -i -E "s#([\"(])$re([\")])#\\1$rel?v=$hash\\2#g" "$STAGE/index.html" "$STAGE/style.css"
done
echo "→ $(grep -c '?v=' "$STAGE/index.html") lines of index.html link versioned files"

if [ $LOCAL -eq 1 ]; then
  SRC="$STAGE" "$PUBLISH"
else
  SRC="$STAGE" "$PUBLISH" "$TARGET"
  echo "→ published: https://isomar.marinetechs.com/"
fi
