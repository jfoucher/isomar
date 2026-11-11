#!/usr/bin/env bash
# Downloads free chart data for French coasts (and a few generic samples) into
# testdata/, which is gitignored. Each dataset goes in its own directory with a
# SOURCE.txt recording where it came from and under which licence.
#
# Usage: scripts/fetch-testdata.sh [--list] [--all] [name ...]
#   (no args)  the "core" set: small files, a few MB in total
#   --all      core plus the larger extras (about 600 MB)
#   --list     show the datasets and exit
#   name ...   fetch just those datasets
#
# Datasets marked "eval" are under the SHOM evaluation licence: free to use for
# testing, but NOT to redistribute and not for navigation. Never commit them.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
DEST="$ROOT/testdata"
SHOM=https://services.data.shom.fr

# name|group|kind|expected-bytes|licence|url
# kind: zip, 7z, file:<name>, pmtiles:<bbox>:<maxzoom>
DATASETS=(
  "shom-s101-pack|core|zip|14638178|eval|$SHOM/static/jeux_test/S101_May_June.zip"
  "shom-iroise-s57|core|7z|2029082|Licence Ouverte 2.0 (cell metadata says commercial reuse needs a contract)|$SHOM/static/hackathon/S57_OPENDATA_IROISEE.7z"
  "shom-s102|core|zip|2770079|eval|$SHOM/static/jeux_test/S-102_may2026.zip"
  "shom-balisage|core|7z|3610746|Licence Ouverte 2.0|$SHOM/INSPIRE/telechargement/prepackageGroup/BALISAGE_PACK_DL/prepackage/BALISAGE/file/BALISAGE.7z"
  "noaa-us5ma20m|core|zip|54398|Public domain (NOAA)|https://charts.noaa.gov/ENCs/US5MA20M.zip"
  "emodnet-brest|core|file:brest.tif|983440|EMODnet, not for navigation|https://ows.emodnet-bathymetry.eu/wcs?service=WCS&version=2.0.1&request=GetCoverage&coverageId=emodnet__mean&format=image/tiff&subset=Lat(48.25,48.50)&subset=Long(-5.0,-4.3)"
  "shom-dtm-saint-malo-5m|extra|7z|43878587|Licence Ouverte 2.0|$SHOM/INSPIRE/telechargement/prepackageGroup/MNT_COTIER_PORT_SM_PAPI_SM_5m_PACK_DL/prepackage/MNT_COTIER_PORT_SAINT-MALO_PAPI_PBMA/file/MNT_COTIER_PORT_SAINT-MALO_PAPI_PBMA.7z"
  "shom-dtm-morbihan-20m|extra|7z|105627341|Licence Ouverte 2.0|$SHOM/INSPIRE/telechargement/prepackageGroup/MNT_COTIER_MORBIHAN_TANDEM_20m_PBMA_4326_PACK_DL/prepackage/MNT_COTIER_MORBIHAN_TANDEM_PBMA/file/MNT_COTIER_MORBIHAN_TANDEM_PBMA.7z"
  "shom-limite-terre-mer|extra|7z|255142537|Licence Ouverte 2.0|$SHOM/INSPIRE/telechargement/prepackageGroup/LIMTM_PACK_DL/prepackage/Limite_terre_mer_France_metropolitaine/file/Limite_terre_mer_France_metropolitaine.7z"
  "nz-kap-archive|extra|zip|186082246|Free (LINZ archive via OpenCPN catalogue), not for navigation|https://github.com/chartcatalogs/nzmariner_archive/releases/download/v1/BSB_Base_20230616.zip"
  "seamap-brittany|extra|pmtiles:-5.6,47.2,-1.5,49.1:12|0|CC BY 4.0, OSM data under ODbL, not for navigation|https://tiles.openwaters.io/seamap/2026-09-28.pmtiles"
)

PMTILES_VERSION=1.31.2
BIN_DIR="$DEST/.bin"

die() { echo "error: $*" >&2; exit 1; }

field() { echo "$1" | cut -d'|' -f"$2"; }

list() {
  printf '%-26s %-6s %-12s %s\n' NAME GROUP SIZE LICENCE
  for d in "${DATASETS[@]}"; do
    printf '%-26s %-6s %-12s %s\n' "$(field "$d" 1)" "$(field "$d" 2)" "$(field "$d" 4)" "$(field "$d" 5)"
  done
}

ensure_pmtiles() {
  if command -v pmtiles >/dev/null; then PMTILES=$(command -v pmtiles); return; fi
  PMTILES="$BIN_DIR/pmtiles"
  [ -x "$PMTILES" ] && return
  echo "installing go-pmtiles $PMTILES_VERSION into $BIN_DIR"
  local tmp; tmp=$(mktemp -d)
  curl -fsSL --retry 3 -o "$tmp/pm.tgz" \
    "https://github.com/protomaps/go-pmtiles/releases/download/v$PMTILES_VERSION/go-pmtiles_${PMTILES_VERSION}_Linux_x86_64.tar.gz"
  mkdir -p "$BIN_DIR"
  tar -xzf "$tmp/pm.tgz" -C "$BIN_DIR" pmtiles
  rm -rf "$tmp"
}

fetch() {
  local entry=$1 name kind expected licence url
  name=$(field "$entry" 1); kind=$(field "$entry" 3); expected=$(field "$entry" 4)
  licence=$(field "$entry" 5); url=$(field "$entry" 6-)
  local out="$DEST/$name"
  if [ -d "$out" ] && [ -n "$(ls -A "$out")" ]; then
    echo "== $name: already present"; return
  fi
  echo "== $name"
  local tmp="$DEST/.tmp-$name"
  rm -rf "$tmp"; mkdir -p "$tmp/x"

  case "$kind" in
    zip|7z)
      curl -fsSL --retry 3 -g -o "$tmp/archive" "$url"
      local got; got=$(stat -c %s "$tmp/archive")
      [ "$got" = "$expected" ] || echo "   note: size $got differs from the expected $expected (upstream may have updated)"
      if [ "$kind" = zip ]; then unzip -q "$tmp/archive" -d "$tmp/x"; else 7z x -y -bso0 -bsp0 -o"$tmp/x" "$tmp/archive"; fi
      rm -f "$tmp/archive"
      ;;
    file:*)
      curl -fsSL --retry 3 -g -o "$tmp/x/${kind#file:}" "$url"
      ;;
    pmtiles:*)
      ensure_pmtiles
      local rest=${kind#pmtiles:} bbox maxzoom
      bbox=${rest%%:*}; maxzoom=${rest##*:}
      "$PMTILES" extract "$url" "$tmp/x/$name.pmtiles" --bbox="$bbox" --maxzoom="$maxzoom"
      ;;
    *) die "unknown kind $kind" ;;
  esac

  {
    echo "name:     $name"
    echo "url:      $url"
    echo "fetched:  $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    if [ "$licence" = eval ]; then
      echo "licence:  SHOM evaluation licence. Free for testing; NOT for redistribution, commercial use or navigation."
    else
      echo "licence:  $licence"
    fi
  } > "$tmp/x/SOURCE.txt"
  mv "$tmp/x" "$out"
  rmdir "$tmp"
  du -sh "$out" | sed 's/^/   /'
}

main() {
  local all=0 names=()
  for a in "$@"; do
    case "$a" in
      --list) list; exit 0 ;;
      --all) all=1 ;;
      -h|--help) sed -n '2,13p' "$0"; exit 0 ;;
      *) names+=("$a") ;;
    esac
  done
  mkdir -p "$DEST"

  if [ ${#names[@]} -gt 0 ]; then
    for n in "${names[@]}"; do
      local found=0
      for d in "${DATASETS[@]}"; do
        [ "$(field "$d" 1)" = "$n" ] && { fetch "$d"; found=1; }
      done
      [ $found = 1 ] || die "unknown dataset: $n (see --list)"
    done
    return
  fi

  for d in "${DATASETS[@]}"; do
    local group; group=$(field "$d" 2)
    if [ "$group" = core ] || [ $all = 1 ]; then fetch "$d"; fi
  done
}

main "$@"
