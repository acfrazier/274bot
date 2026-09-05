#!/usr/bin/env bash
# Sequential T4 baseline matrix. Order: all N=1, then 32, then 128.
# Incomplete scale → record blocked; do not retry 128 until filled.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
export RS2B0T="${RS2B0T:-/Users/acfrazier/experiments/rs2b0t}"
CELL="$ROOT/docs/memory/run_cell.sh"
SUMMARY="$ROOT/docs/memory/samples/matrix_summary.tsv"
mkdir -p "$ROOT/docs/memory/samples"
if [[ ! -f "$SUMMARY" ]]; then
  echo -e "frontend\tn\tworkload\trun\texit\tstatus\tready_last\tnote\tstamp" >"$SUMMARY"
fi

status_of() {
  local log="$1"
  if rg -q "PASS: memory" "$log" 2>/dev/null; then
    echo PASS
  elif rg -q "blocked:" "$log" 2>/dev/null; then
    echo blocked
  elif rg -q "FAIL:" "$log" 2>/dev/null; then
    echo FAIL
  else
    echo unknown
  fi
}

ready_last() {
  local jl="$1"
  if [[ -f "$jl" ]]; then
    tail -1 "$jl" | sed -n 's/.*"ready":\([0-9]*\).*/\1/p'
  else
    echo ""
  fi
}

run_one() {
  local fe="$1" n="$2" wl="$3" rid="$4"
  shift 4 || true
  local stem="${fe}_n${n}_${wl}_r${rid}"
  if [[ "$fe" == "panel-cpu" ]]; then
    stem="panel_cpu_n${n}_${wl}_r${rid}"
  fi
  local log="$ROOT/docs/memory/samples/${stem}.log"
  local jl="$ROOT/docs/memory/samples/${stem}.jsonl"
  # Skip if already PASS
  if [[ -f "$log" ]] && rg -q "PASS: memory" "$log" 2>/dev/null; then
    echo "SKIP already PASS $stem"
    return 0
  fi
  echo ">>> RUN $stem extras=$* $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  set +e
  "$CELL" "$fe" "$n" "$wl" "$rid" "$@"
  local ec=$?
  set -e
  local st note ready
  st="$(status_of "$log")"
  ready="$(ready_last "$jl")"
  note=""
  if [[ "$st" == "blocked" ]]; then
    note="$(rg -o 'blocked:[^"]*' "$log" | tail -1 || true)"
  elif [[ "$st" == "FAIL" ]]; then
    note="$(rg -o 'FAIL:[^"]*' "$log" | tail -1 || true)"
  fi
  echo -e "${fe}\t${n}\t${wl}\t${rid}\t${ec}\t${st}\t${ready}\t${note}\t$(date -u +%Y-%m-%dT%H:%M:%SZ)" >>"$SUMMARY"
  echo "<<< DONE $stem exit=$ec status=$st ready=$ready"
  # Fail-closed: seed/script FAIL (not scale blocked) stops the matrix
  if [[ "$st" == "FAIL" ]] || [[ $ec -ne 0 && "$st" != "blocked" && "$st" != "PASS" ]]; then
    echo "STOP: honest FAIL or non-blocked error on $stem" | tee -a "$SUMMARY"
    return 99
  fi
  return 0
}

# --- N=1 prove harness ---
for fe in panel tui; do
  for wl in idle active lifecycle; do
    for r in 1 2 3; do
      run_one "$fe" 1 "$wl" "$r" || exit $?
    done
  done
done

# BOT_CPU 1-slot idle panel (one run)
run_one panel-cpu 1 idle 1 || exit $?

# --- N=32 ---
for fe in panel tui; do
  for wl in idle active lifecycle; do
    for r in 1 2 3; do
      run_one "$fe" 32 "$wl" "$r" || exit $?
    done
  done
done

# 32-slot lifecycle soak 3600s — pick first frontend that PASSed 600s lifecycle
SOAK_FE=""
if rg -q $'panel\t32\tlifecycle\t.*\tPASS' "$SUMMARY" 2>/dev/null; then
  SOAK_FE=panel
elif rg -q $'tui\t32\tlifecycle\t.*\tPASS' "$SUMMARY" 2>/dev/null; then
  SOAK_FE=tui
fi
if [[ -n "$SOAK_FE" ]]; then
  echo "SOAK frontend=$SOAK_FE"
  run_one "$SOAK_FE" 32 lifecycle soak BOT_MEMORY_OBSERVE_S=3600 || exit $?
else
  echo "SKIP soak: no 32-lifecycle PASS yet" | tee -a "$SUMMARY"
fi

# --- N=128 ---
for fe in panel tui; do
  for wl in idle active lifecycle; do
    for r in 1 2 3; do
      run_one "$fe" 128 "$wl" "$r" || exit $?
    done
  done
done

echo "MATRIX COMPLETE $(date -u +%Y-%m-%dT%H:%M:%SZ)" | tee -a "$SUMMARY"
