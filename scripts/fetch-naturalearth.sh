#!/usr/bin/env bash
# Downloads the Natural Earth layers `cargo xtask naturalearth` reads into testdata/naturalearth/
# (gitignored; public domain). Usage: scripts/fetch-naturalearth.sh
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
DIR="$ROOT/testdata/naturalearth"
BASE=https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson
mkdir -p "$DIR"
for f in ne_10m_geography_marine_polys ne_10m_geography_regions_polys ne_10m_geography_regions_points ne_10m_land ne_10m_antarctic_ice_shelves_polys ne_10m_admin_0_countries ne_10m_admin_0_map_units; do
  curl -fL --retry 3 -o "$DIR/$f.geojson.part" "$BASE/$f.geojson"
  mv "$DIR/$f.geojson.part" "$DIR/$f.geojson"
  echo "$f"
done
