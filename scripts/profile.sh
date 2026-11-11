#!/usr/bin/env bash
# A sampling profiler for when perf is not installed: eu-stack every few ms on a running process, then the
# functions that were on the stack most often, of the threads that were busy.
#
# Usage: scripts/profile.sh <process name|pid> <seconds> [out.txt] [filter]
#   filter: only stacks holding this text (e.g. TilePaint for the tile workers)
# Without inline frames eu-stack takes about 0.16 s a sample on the GTK app (1.7 s with them); set INLINE=1 for
# them (fine for small binaries such as the routebench example). Build with symbols for useful names:
#   CARGO_PROFILE_RELEASE_DEBUG=line-tables-only CARGO_TARGET_DIR=target/prof cargo build --release
set -euo pipefail
TARGET=${1:?process name or pid}; SECS=${2:?seconds}; OUT=${3:-$(mktemp --suffix=.txt)}; FILTER=${4:-}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
PID=$TARGET
if ! [[ $TARGET =~ ^[0-9]+$ ]]; then
  for _ in $(seq 300); do PID=$(pgrep -n -x "$TARGET" || true); [ -n "$PID" ] && break; sleep 0.2; done
fi
[ -n "$PID" ] || { echo "no process $TARGET" >&2; exit 1; }
end=$((SECONDS + ${SECS%.*}))
: > "$OUT"
while [ $SECONDS -lt $end ] && kill -0 "$PID" 2>/dev/null; do
  echo "=== SAMPLE" >> "$OUT"
  eu-stack ${INLINE:+-i} -p "$PID" >> "$OUT" 2>/dev/null || true
  sleep 0.01
done
echo "samples in $OUT" >&2
python3 -I "$ROOT/scripts/profile-report.py" "$OUT" "$FILTER"
