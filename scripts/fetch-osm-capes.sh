#!/usr/bin/env bash
# Downloads every named cape and cove in OpenStreetMap (ODbL) into testdata/osm-capes/ (gitignored), for
# `cargo xtask capes testdata/osm-capes`, from QLever's copy of the planet (https://qlever.dev, University of
# Freiburg), one query per kind for the whole world, written as Overpass would answer (`elements`, each with
# its tags and `lat`/`lon`: a node's position, the centroid of a way or relation):
#   capes.json   natural=cape and natural=headland (Start Point is a headland)
#   near.json    localities and points whose name says they are a cape (Punta de sa Cresta, Cap de Sant
#                Sebastià), and cliffs with a Wikidata entry whose name says so (Beachy Head)
#   coves.json   natural=bay and water=cove (Cala Figuereta is a cove of water, Cala Llombards a bay)
# Overpass was used first: the localities' name search over 45 degree bands timed out after 900 s, and the
# main server then refused connections; QLever answers each kind in seconds.
# A kind already downloaded is kept; delete its file to fetch it again.
# Usage: scripts/fetch-osm-capes.sh [capes|near|coves...]
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
DIR="$ROOT/testdata/osm-capes"
mkdir -p "$DIR"
ENDPOINT=${QLEVER:-https://qlever.dev/api/osm-planet}
# As `isomar_core::chart::capes::is_cape_name`, which the xtask checks again.
WORDS='(^|[ -])(cape|point|pt|head|ness|bill|mull|cap|cabo|capo|capu|punta|pta|ponta|pointe|pte|kap|kaap|hoek|odde|nes|akra|rt|ras)([ .-]|$)'
NAMED="FILTER(REGEX(?name, \"$WORDS\", \"i\"))"
# What each kind is: `?osm` and its `?name`.
declare -A WHERE=(
  [capes]='{ ?osm osmkey:natural "cape" } UNION { ?osm osmkey:natural "headland" } ?osm osmkey:name ?name .'
  [near]="{ ?osm osmkey:natural \"cliff\" . ?osm osmkey:wikidata ?known . ?osm osmkey:name ?name . $NAMED }
    UNION { ?osm osmkey:place \"locality\" . ?osm osmkey:name ?name . $NAMED }
    UNION { ?osm osmkey:natural \"point\" . ?osm osmkey:name ?name }"
  [coves]='{ ?osm osmkey:natural "bay" } UNION { ?osm osmkey:water "cove" } ?osm osmkey:name ?name .'
)
query() {
  cat <<EOF
PREFIX osmkey: <https://www.openstreetmap.org/wiki/Key:>
PREFIX geo: <http://www.opengis.net/ont/geosparql#>
PREFIX geof: <http://www.opengis.net/def/function/geosparql/>
SELECT ?osm ?name ?wikidata ?natural (geof:latitude(?c) AS ?lat) (geof:longitude(?c) AS ?lon) WHERE {
  ${WHERE[$1]}
  OPTIONAL { ?osm osmkey:wikidata ?wikidata }
  OPTIONAL { ?osm osmkey:natural ?natural }
  ?osm geo:hasGeometry/geo:asWKT ?shape .
  BIND(geof:centroid(?shape) AS ?c)
}
EOF
}
# The answer as Overpass gives it, which is what the xtask reads.
TO_ELEMENTS='{elements: [.results.bindings[] | (.osm.value | split("/")) as $id | {
  type: $id[-2], id: ($id[-1] | tonumber), lat: (.lat.value | tonumber), lon: (.lon.value | tonumber),
  tags: ({name: .name.value} + (if .wikidata then {wikidata: .wikidata.value} else {} end)
    + (if .natural then {natural: .natural.value} else {} end))}]}'
fetch() {
  local name=$1 out="$DIR/$1.json" part="$DIR/$1.json.part" wait=30
  [ -s "$out" ] && return
  for try in 1 2 3 4 5; do
    if curl -fsSL -m 900 -A "Isomar chart plotter (xtask capes)" -H 'Accept: application/sparql-results+json' \
      --data-urlencode "query=$(query "$name")" -o "$part" "$ENDPOINT" && jq -e '.results.bindings' "$part" > /dev/null; then
      jq -c "$TO_ELEMENTS" "$part" > "$out"
      rm -f "$part"
      # The 45 degree bands Overpass was asked for (`near.0.json`…) would be read twice.
      rm -f "$DIR/$name".[0-9]*.json
      echo "$(jq '.elements | length' "$out") elements in $out"
      return
    fi
    echo "$out: attempt $try failed, waiting ${wait}s" >&2
    sleep $wait
    wait=$((wait * 2))
  done
  exit 1
}
[ $# -gt 0 ] || set -- capes near coves
for name in "$@"; do
  fetch "$name"
done
