#!/usr/bin/env bash
set -euo pipefail

usage() {
  printf 'usage: %s <report.json> [json-baseline.json]\n' "$0" >&2
  exit 2
}

[[ $# -ge 1 && $# -le 2 ]] || usage
report=$1
[[ "$report" = /* ]] || { echo "report path must be absolute" >&2; exit 2; }
[[ ! -e "$report" && ! -L "$report" ]] || { echo "report already exists: $report" >&2; exit 2; }

baseline=${2:-${T53_JSON_BASELINE:-}}
if [[ -n "$baseline" ]]; then
  [[ "$baseline" = /* ]] || { echo "JSON baseline path must be absolute" >&2; exit 2; }
  [[ -f "$baseline" ]] || { echo "JSON baseline does not exist: $baseline" >&2; exit 2; }
  export T53_JSON_BASELINE="$baseline"
fi

mkdir -p -- "$(dirname -- "$report")"
export T53_OCCUPANCY_OUT="$report"
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd)
cd "$repo"

status=0
cargo test --locked --release -p host-play --no-default-features --lib -- \
  --ignored --exact local_state::sql_tests::occupancy_gate_every_sql_job_class --nocapture --test-threads=1 || status=$?

if [[ -s "$report" ]]; then
  cat -- "$report"
else
  echo "benchmark did not produce a report: $report" >&2
  [[ $status -ne 0 ]] || status=1
fi
exit "$status"
