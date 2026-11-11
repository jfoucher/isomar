#!/usr/bin/env bash
# Pans and zooms the chart headlessly at the owner's panel size and prints ISOMAR_BENCH's line for each
# view, so a change can be measured against design/perf/ (see the baseline there for the method).
#
# Usage: scripts/bench.sh [--runs N] [--timing] [--warm] [--kinds "pan zoom"] [view...]
#   views: iroise12 loire13 mallorca10 brazil10 boston13 biscay6 (all by default), or name=lat,lon,zoom
#   --timing  also sets ISOMAR_DEBUG_TIMING and prints tiles drawn and ms per layer (scripts/bench-layers.py)
#   --warm    runs online, to fill the tile caches first; otherwise ISOMAR_OFFLINE
# Env: BIN (default target/release/isomar: `cargo build --release` first), BENCH_CONFIG (a folder holding
# isomar/settings.json; default a copy of yours), DELAY (ms before the bench, 15000), STEPS (60),
# MONITOR (2256x1504), SIZE (2256x1450), PALETTE (day), OUT (where screenshots and logs go).
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
RUNS=1; TIMING=; WARM=; KINDS="pan zoom"; VIEWS=()
while [ $# -gt 0 ]; do
  case $1 in
    --runs) RUNS=$2; shift 2 ;;
    --timing) TIMING=1; shift ;;
    --warm) WARM=1; shift ;;
    --kinds) KINDS=$2; shift 2 ;;
    *) VIEWS+=("$1"); shift ;;
  esac
done
declare -A AT=(
  [iroise12]=48.35,-5.05,12
  [loire13]=47.25,-2.45,13
  [mallorca10]=39.55,2.30,10
  [brazil10]=-6.12,-35.34,10.25
  [boston13]=42.33,-71.00,13
  [biscay6]=46.0,-6.0,6
)
ORDER=(iroise12 loire13 mallorca10 brazil10 boston13 biscay6)
[ ${#VIEWS[@]} -eq 0 ] && VIEWS=("${ORDER[@]}")
BIN=${BIN:-$ROOT/target/release/isomar}
OUT=${OUT:-$(mktemp -d)}
mkdir -p "$OUT"
CFG=${BENCH_CONFIG:-}
if [ -z "$CFG" ]; then
  CFG=$(mktemp -d)
  mkdir -p "$CFG/isomar"
  cp "${XDG_CONFIG_HOME:-$HOME/.config}/isomar/settings.json" "$CFG/isomar/"
fi
LOG=${XDG_CACHE_HOME:-$HOME/.cache}/isomar/isomar.log
OFF=ISOMAR_OFFLINE=1
[ -n "$WARM" ] && OFF=ISOMAR_BENCH_ONLINE=1
for run in $(seq "$RUNS"); do
  for v in "${VIEWS[@]}"; do
    name=${v%%=*}
    view=${AT[$name]:-${v#*=}}
    for kind in $KINDS; do
      before=$(wc -l < "$LOG" 2>/dev/null || echo 0)
      env ISOMAR_BIN="'$BIN'" ISOMAR_TEST_CONFIG="$CFG" ISOMAR_DATA_DIR="${ISOMAR_DATA_DIR:-$HOME/.local/share/isomar/data}" \
        ISOMAR_NO_SAVE=1 $OFF ISOMAR_MONITOR="${MONITOR:-2256x1504}" ISOMAR_SIZE="${SIZE:-2256x1450}" ISOMAR_DEBUG_VIEW="$view" \
        ISOMAR_PALETTE="${PALETTE:-day}" ISOMAR_BENCH="$kind" ISOMAR_BENCH_STEPS="${STEPS:-60}" ISOMAR_SHOT_TIMEOUT="${TIMEOUT:-90}" \
        ${TIMING:+ISOMAR_DEBUG_TIMING=1} \
        "$ROOT/scripts/screenshot.sh" "$OUT/$name-$kind-$run.png" "${DELAY:-15000}" >/dev/null 2>&1 || true
      tail -n +$((before + 1)) "$LOG" > "$OUT/$name-$kind-$run.log"
      line=$( (grep "bench $kind" "$OUT/$name-$kind-$run.log" || true) | tail -1 | sed 's/.*isomar: //')
      echo "r$run $name $kind: ${line:-NO RESULT}"
      if [ -n "$TIMING" ]; then
        python3 -I "$ROOT/scripts/bench-layers.py" "$OUT/$name-$kind-$run.log" || true
      fi
    done
  done
done
echo "screenshots and logs in $OUT" >&2
