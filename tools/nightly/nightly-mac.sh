#!/bin/bash
set -euo pipefail

CONFIG="$HOME/.config/274bot-nightly/mac-nightly.env"
if [ ! -r "$CONFIG" ]; then
    printf '%s\n' "nightly-mac: missing $CONFIG" >&2
    exit 78
fi
set -a
. "$CONFIG"
set +a

: "${NIGHTLY_MAC_RUNNER:?set NIGHTLY_MAC_RUNNER in $CONFIG}"
: "${NIGHTLY_MAC_PYTHON:?set NIGHTLY_MAC_PYTHON in $CONFIG}"
: "${NIGHTLY_MAC_VM_TOOL:?set NIGHTLY_MAC_VM_TOOL in $CONFIG}"

exec "$NIGHTLY_MAC_PYTHON" -u \
    "$NIGHTLY_MAC_RUNNER/tools/nightly/nightly-mac-guest.py" "$@"
