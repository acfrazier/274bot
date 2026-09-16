#!/usr/bin/env bash
# Offline Player.save fixture writer (server-native roundtrip).
# Bundles engine sources with cycle stubs, keeps real bzip2/wasm for pack loads.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
DEFAULT_ENG="${BOT_SERVER_ROOT:-/Users/acfrazier/experiments/Server/engine}"
ENG="${SERVER_ROOT:-$DEFAULT_ENG}"

if [[ ! -d "$ENG/src/engine/entity" ]]; then
  echo "error: server engine root not found: $ENG (set BOT_SERVER_ROOT)" >&2
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

# Forward all CLI args; always pin --server-root last if not already present.
has_root=0
for a in "$@"; do
  if [[ "$a" == "--server-root" ]]; then has_root=1; fi
done
if [[ $has_root -eq 1 ]]; then
  node "$STAGE/write_player_fixture.mjs" "$@"
else
  node "$STAGE/write_player_fixture.mjs" "$@" --server-root "$ENG"
fi
