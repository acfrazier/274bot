#!/usr/bin/env bash
# Offline Player.save fixture writer (server-native roundtrip).
# Bundles engine sources with cycle stubs, keeps real bzip2/wasm for pack loads.
#
# One canonical server root feeds source, modules, data, bzip2, config, and
# receipt. Precedence: --server-root CLI > SERVER_ROOT > BOT_SERVER_ROOT >
# default Server/engine. Missing --server-root value fail-closed.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
DEFAULT_ENG="/Users/acfrazier/experiments/Server/engine"

# Parse --server-root from argv BEFORE any compile/link so CLI cannot disagree
# with the engine tree esbuild bundles.
CLI_ROOT=""
forward_args=()
expect_root_value=0
for a in "$@"; do
  if [[ $expect_root_value -eq 1 ]]; then
    if [[ -z "$a" || "$a" == -* ]]; then
      echo "error: --server-root requires a non-empty path" >&2
      exit 2
    fi
    CLI_ROOT="$a"
    expect_root_value=0
    forward_args+=("$a")
    continue
  fi
  if [[ "$a" == "--server-root" ]]; then
    expect_root_value=1
    forward_args+=("$a")
    continue
  fi
  forward_args+=("$a")
done
if [[ $expect_root_value -eq 1 ]]; then
  echo "error: --server-root requires a path" >&2
  exit 2
fi

# Canonical selected root (CLI wins over env).
if [[ -n "$CLI_ROOT" ]]; then
  ENG_CANDIDATE="$CLI_ROOT"
elif [[ -n "${SERVER_ROOT:-}" ]]; then
  ENG_CANDIDATE="$SERVER_ROOT"
elif [[ -n "${BOT_SERVER_ROOT:-}" ]]; then
  ENG_CANDIDATE="$BOT_SERVER_ROOT"
else
  ENG_CANDIDATE="$DEFAULT_ENG"
fi

if [[ -z "$ENG_CANDIDATE" ]]; then
  echo "error: server engine root empty (pass --server-root or set BOT_SERVER_ROOT)" >&2
  exit 2
fi
if [[ ! -d "$ENG_CANDIDATE" ]]; then
  echo "error: server engine root not found: $ENG_CANDIDATE (pass --server-root or set BOT_SERVER_ROOT)" >&2
  exit 2
fi
ENG="$(cd "$ENG_CANDIDATE" && pwd)"

if [[ ! -d "$ENG/src/engine/entity" ]]; then
  echo "error: server engine root missing src/engine/entity: $ENG" >&2
  exit 2
fi
if [[ ! -x "$ENG/node_modules/.bin/esbuild" ]]; then
  echo "error: esbuild missing under $ENG/node_modules (npm install in engine)" >&2
  exit 2
fi

BUNDLE="${TMPDIR:-/tmp}/274bot-fixture-bundle-$$"
STAGE="$BUNDLE/stage"
mkdir -p "$STAGE/tools/harness/stubs"
cleanup() { rm -rf "$BUNDLE"; }
trap cleanup EXIT

cp "$SCRIPT_DIR/write_player_fixture.ts" "$STAGE/tools/harness/"
cp "$SCRIPT_DIR/stubs/"*.ts "$STAGE/tools/harness/stubs/"
ln -sfn "$ENG/src" "$STAGE/src"
ln -sfn "$ENG/node_modules" "$STAGE/node_modules"
ln -sfn "$ENG/data" "$STAGE/data"
cp "$ENG/src/3rdparty/bzip2-wasm/bzip2-1.0.8/bzip2.wasm" "$STAGE/bzip2.wasm"
cat > "$STAGE/package.json" <<'EOF'
{
  "name": "274bot-write-player-fixture-stage",
  "private": true,
  "type": "module",
  "imports": {
    "#3rdparty/*": "./src/3rdparty/*",
    "#/*": "./src/*",
    "#tools/*": "./src/../tools/*"
  }
}
EOF

cd "$STAGE"
"$ENG/node_modules/.bin/esbuild" tools/harness/write_player_fixture.ts \
  --bundle \
  --platform=node \
  --format=esm \
  --target=node24 \
  --packages=external \
  --alias:\#/engine/World.js=./tools/harness/stubs/world_stub.ts \
  --alias:\#/util/Environment.js=./tools/harness/stubs/environment_stub.ts \
  --alias:\#/engine/entity/NetworkPlayer.js=./tools/harness/stubs/network_player_stub.ts \
  --loader:.wasm=file \
  --outfile="$STAGE/write_player_fixture.mjs" >/dev/null

# Ensure wasm sits next to the bundle (esbuild file loader may rename).
if [[ ! -f "$STAGE/bzip2.wasm" ]]; then
  cp "$ENG/src/3rdparty/bzip2-wasm/bzip2-1.0.8/bzip2.wasm" "$STAGE/bzip2.wasm"
fi

# Always pin the same canonical ENG on the TS CLI (pack + receipt provenance).
# If CLI already had --server-root, append the resolved absolute path last so it wins.
node "$STAGE/write_player_fixture.mjs" "${forward_args[@]}" --server-root "$ENG"
