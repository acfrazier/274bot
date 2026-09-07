#!/usr/bin/env bash
# Container entrypoint for the memory-reference image.
# Starts the host-engine loopback relay (default on), optional desktop stack,
# then execs the user command.
#
# Runs as root long enough to bind relay :80 and fix volume ownership, then
# drops to MEMORY_REF_DROP_USER (default memref/uid 1000) so DAC-based unit
# tests (chmod 0500 tempdirs) behave as on a normal non-root developer host.
# Set MEMORY_REF_DROP_USER=root to keep root for debugging only.
set -euo pipefail

SCRIPTS="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

export HOME="${HOME:-/runtime/home}"
export ENGINE_DIR="${ENGINE_DIR:-/runtime/engine}"
export CARGO_HOME="${CARGO_HOME:-/cargo-home}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/target}"
export BOT_TARGET="${BOT_TARGET:-local}"
# Ensure cargo bin from the rust image stays on PATH after drop.
export PATH="/cargo-home/bin:/usr/local/cargo/bin:${PATH}"
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

drop_user="${MEMORY_REF_DROP_USER:-memref}"
if [[ "$(id -u)" -eq 0 && "$drop_user" != "root" && "$drop_user" != "0" ]]; then
  # Named volumes may have been created as root on earlier runs — make them
  # writable for the drop user. Skip read-only bind mounts (engine, .274bot).
  if id -u "$drop_user" >/dev/null 2>&1; then
    chown -R "$drop_user:$drop_user" "$CARGO_HOME" "$CARGO_TARGET_DIR" 2>/dev/null || true
    # HOME itself (not the optional ro .274bot mount) must be writable for cargo/tests.
    chown "$drop_user:$drop_user" "$HOME" 2>/dev/null || true
    # /tmp for tempdir-based DAC tests
    chmod 1777 /tmp 2>/dev/null || true
    echo "memory-ref: dropping to user $drop_user (uid=$(id -u "$drop_user")) for command" >&2
    # setpriv (util-linux): no `--` separator on bookworm; next argv is the program.
    exec setpriv --reuid="$drop_user" --regid="$drop_user" --init-groups "$@"
  else
    echo "memory-ref: WARNING drop user $drop_user missing; staying root" >&2
  fi
fi

exec "$@"
