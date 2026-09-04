#!/bin/sh
# Copy a local engine pack cache into place, or say how to fetch it.
set -eu
if [ "${1:-}" = "--prod" ]; then
    echo "prod cache is fetched on first --prod boot"
    echo "(HTTPS /crc + jags on w1.rs2b2t.com:443 into \$HOME/.274bot/unpack)"
    echo "versioned snapshots (models.bin) stay in unpack/<sha256(versionlist)[:8]>/"
    echo "then: cargo run --release -p host-play -- --prod --user YOUR_NAME"
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
echo "with no local engine: scripts/fetch-cache.sh --prod and boot with --prod (HTTPS :443)."
exit 1
