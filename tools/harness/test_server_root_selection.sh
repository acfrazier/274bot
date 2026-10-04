#!/usr/bin/env bash
# Regression: CLI --server-root must select source+pack+receipt, not env.
# Root A = TEST_ROOT_A (274 engine); root B = TEST_ROOT_B (isolated 289 engine).
# Fails closed if either is missing or if CLI is ignored (mixed serializer/pack).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WRITER="$SCRIPT_DIR/run_write_player_fixture.sh"
CHECKER="$SCRIPT_DIR/check_server_root_selection.py"

if [[ -z "${TEST_ROOT_A:-}" ]]; then
  echo "error: set TEST_ROOT_A to the 274 engine root" >&2
  exit 2
fi
if [[ -z "${TEST_ROOT_B:-}" ]]; then
  echo "error: set TEST_ROOT_B to the isolated 289 engine root" >&2
  exit 2
fi
ROOT_A="$TEST_ROOT_A"
ROOT_B="$TEST_ROOT_B"

for label in A:"$ROOT_A" B:"$ROOT_B"; do
  name="${label%%:*}"
  path="${label#*:}"
  if [[ ! -d "$path/src/engine/entity" ]]; then
    echo "error: root $name missing entity sources: $path" >&2
    exit 2
  fi
  if [[ ! -f "$path/data/pack/server/obj.dat" ]]; then
    echo "error: root $name missing pack: $path" >&2
    exit 2
  fi
  if [[ ! -x "$path/node_modules/.bin/esbuild" ]]; then
    echo "error: root $name missing esbuild: $path" >&2
    exit 2
  fi
done

ROOT_A="$(cd "$ROOT_A" && pwd)"
ROOT_B="$(cd "$ROOT_B" && pwd)"
if [[ "$ROOT_A" == "$ROOT_B" ]]; then
  echo "error: ROOT_A and ROOT_B must differ for this regression" >&2
  exit 2
fi

OUT="${TMPDIR:-/tmp}/274bot-root-select-test-$$"
mkdir -p "$OUT"
cleanup() { rm -rf "$OUT"; }
trap cleanup EXIT

# 289 marker unique to B Player.ts (274 uses changeNpcCollision instead).
if ! grep -q 'changePlayerOccCollision' "$ROOT_B/src/engine/entity/Player.ts"; then
  echo "error: expected 289 marker changePlayerOccCollision in $ROOT_B Player.ts" >&2
  exit 2
fi
if grep -q 'changePlayerOccCollision' "$ROOT_A/src/engine/entity/Player.ts"; then
  echo "error: ROOT_A unexpectedly has 289 marker; pick distinct engines" >&2
  exit 2
fi

run_one() {
  local tag=$1 eng=$2
  # Force env toward A so CLI must win when eng is B.
  env BOT_SERVER_ROOT="$ROOT_A" \
    "$WRITER" \
    --username "rtsel${tag}" \
    --output "$OUT/${tag}.sav" \
    --receipt "$OUT/${tag}.json" \
    --fixture thiever \
    --overwrite \
    --server-root "$eng"
}

echo "== pure A (control) =="
run_one pureA "$ROOT_A"
echo "== CLI B while env A (the regression) =="
run_one cliB "$ROOT_B"
echo "== pure B control =="
run_one pureB "$ROOT_B"

python3 "$CHECKER" "$OUT" "$ROOT_A" "$ROOT_B"
echo "PASS: CLI --server-root B overrides env A; receipt/source/pack agree"
