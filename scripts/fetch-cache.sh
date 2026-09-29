#!/bin/sh
# Copy a local engine pack cache into place, or say how to fetch it.
set -eu
if [ "${1:-}" = "--rs2b2t" ]; then
    echo "rs2b2t cache is fetched on first --rs2b2t boot"
    echo "(HTTPS /crc + jags from the configured roster into \$HOME/.274bot/unpack-289)"
    echo "versioned snapshots (models.bin) stay below unpack-289/"
    echo "then: cargo run --release -p host-play -- --rs2b2t --user YOUR_NAME"
    exit 0
fi
ENGINE_DIR="${ENGINE_DIR:-$HOME/experiments/Server/engine}"
SRC="$ENGINE_DIR/data/pack/client"
if [ -d "$SRC" ] && [ -n "$(ls -A "$SRC" 2>/dev/null || true)" ]; then
    echo "pack cache is at $SRC"
    ls -l "$SRC" | head
    exit 0
fi
echo "no pack files under $SRC"
echo "set ENGINE_DIR to your Lost City engine root and run this again,"
echo "or boot panel-play / tui-play once against the local engine (HTTP /crc on :80)."
echo "with no local engine: scripts/fetch-cache.sh --rs2b2t and boot with --rs2b2t (HTTPS :443)."
exit 1
