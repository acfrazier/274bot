#!/usr/bin/env bash
# One memory-profile baseline cell process.
# Usage: run_cell.sh <frontend> <n> <workload> <run_id> [extra_env...]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
FRONTEND="${1:?frontend}"
N="${2:?n}"
WORKLOAD="${3:?workload}"
RUN_ID="${4:?run_id}"
shift 4 || true
mkdir -p docs/memory/samples
STEM="${FRONTEND}_n${N}_${WORKLOAD}_r${RUN_ID}"
OUT="$ROOT/docs/memory/samples/${STEM}.jsonl"
LOG="$ROOT/docs/memory/samples/${STEM}.log"
case "$FRONTEND" in
  panel) BIN="$ROOT/target/release/panel-play" ;;
  tui) BIN="$ROOT/target/release/tui-play" ;;
  panel-cpu)
    BIN="$ROOT/target/release/panel-play"
    export BOT_CPU=1
    STEM="panel_cpu_n${N}_${WORKLOAD}_r${RUN_ID}"
    OUT="$ROOT/docs/memory/samples/${STEM}.jsonl"
    LOG="$ROOT/docs/memory/samples/${STEM}.log"
    ;;
  *) echo "bad frontend: $FRONTEND" >&2; exit 2 ;;
esac
export RS2B0T="${RS2B0T:-/Users/acfrazier/experiments/rs2b0t}"
export LIVE=1
export BOT_MEMORY_N="$N"
export BOT_MEMORY_WORKLOAD="$WORKLOAD"
export BOT_MEMORY_OUTPUT="$OUT"
# Apply any extra KEY=VAL pairs
for kv in "$@"; do
  export "$kv"
done
echo "=== START $STEM $(date -u +%Y-%m-%dT%H:%M:%SZ) ===" | tee "$LOG"
set +e
"$BIN" >>"$LOG" 2>&1
EC=$?
set -e
echo "=== END $STEM exit=$EC $(date -u +%Y-%m-%dT%H:%M:%SZ) ===" | tee -a "$LOG"
# Capture PASS/FAIL/blocked line
rg -n "PASS:|FAIL:|blocked:" "$LOG" | tail -20 | tee -a "$LOG" || true
exit "$EC"
