#!/usr/bin/env bash
# Container entrypoint for the memory-reference image.
# Starts the host-engine loopback relay (default on), optional desktop stack,
# then execs the user command.
set -euo pipefail

SCRIPTS="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

export HOME="${HOME:-/runtime/home}"
export ENGINE_DIR="${ENGINE_DIR:-/runtime/engine}"
export CARGO_HOME="${CARGO_HOME:-/cargo-home}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/target}"
export BOT_TARGET="${BOT_TARGET:-local}"
mkdir -p "$HOME" "$CARGO_HOME" "$CARGO_TARGET_DIR"

if [[ "${MEMORY_REF_RELAY:-1}" == "1" ]]; then
  # shellcheck source=relay-host-engine.sh
  source "$SCRIPTS/relay-host-engine.sh"
  memory_ref_start_relay || {
    echo "memory-ref: relay failed to start (engine host unreachable?)" >&2
    echo "memory-ref: continuing without relay; local 127.0.0.1 engine required" >&2
  }
fi

if [[ "${MEMORY_REF_DESKTOP:-0}" == "1" || "${MEMORY_REF_PROFILE:-}" == "desktop" ]]; then
  if [[ -x /opt/memory-reference/desktop/start-desktop.sh ]]; then
    /opt/memory-reference/desktop/start-desktop.sh
  else
    echo "memory-ref: desktop requested but start-desktop.sh missing (rebuild WITH_DESKTOP=1)" >&2
  fi
fi

if [[ "$#" -eq 0 ]]; then
  set -- bash
fi

exec "$@"
