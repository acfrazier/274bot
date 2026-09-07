#!/usr/bin/env bash
# Forward container loopback game TCP + jag HTTP to the host engine.
#
# Why: host-play/tui/panel refuse non-loopback --host while BOT_TARGET=local
# (local Java RSA is loopback-only). Docker Desktop reaches the Mac engine at
# host.docker.internal, which is not loopback — so we keep the app on
# 127.0.0.1 and relay.
#
# Ports (defaults match client::bot_target Local):
#   127.0.0.1:43594  →  ${MEMORY_REF_HOST_ENGINE}:43594   game TCP
#   127.0.0.1:80     →  ${MEMORY_REF_HOST_ENGINE}:80      HTTP /crc + jags
#
# Requires: socat. Does not start or reset the engine.

memory_ref_start_relay() {
  local host="${MEMORY_REF_HOST_ENGINE:-host.docker.internal}"
  local game_port="${MEMORY_REF_GAME_PORT:-43594}"
  local jag_port="${MEMORY_REF_JAG_PORT:-80}"
  local logdir="${MEMORY_REF_RELAY_LOGDIR:-/tmp/memory-ref-relay}"
  mkdir -p "$logdir"

  if ! command -v socat >/dev/null 2>&1; then
    echo "memory-ref relay: socat not installed" >&2
    return 1
  fi

  # Avoid double-start on nested shells.
  if [[ -n "${MEMORY_REF_RELAY_STARTED:-}" ]]; then
    return 0
  fi

  # Best-effort reachability note (do not fail hard — engine may start later).
  if command -v getent >/dev/null 2>&1; then
    getent hosts "$host" >/dev/null 2>&1 \
      || echo "memory-ref relay: warning: cannot resolve $host" >&2
  fi

  # TCP-LISTEN on loopback only. fork,reuseaddr for multi-bot fanout.
  socat -d -d \
    TCP-LISTEN:"${game_port}",bind=127.0.0.1,fork,reuseaddr \
    TCP:"${host}:${game_port}",connect-timeout=5 \
    >"$logdir/game.log" 2>&1 &
  echo $! >"$logdir/game.pid"

  # Port 80 may need root at bind time; entrypoint starts as root then drops
  # to memref for the user command. Socat children keep the root bind.
  socat -d -d \
    TCP-LISTEN:"${jag_port}",bind=127.0.0.1,fork,reuseaddr \
    TCP:"${host}:${jag_port}",connect-timeout=5 \
    >"$logdir/jag.log" 2>&1 &
  echo $! >"$logdir/jag.pid"

  export MEMORY_REF_RELAY_STARTED=1
  echo "memory-ref relay: 127.0.0.1:${game_port} + :${jag_port} → ${host}" >&2
  return 0
}

memory_ref_stop_relay() {
  local logdir="${MEMORY_REF_RELAY_LOGDIR:-/tmp/memory-ref-relay}"
  for f in game.pid jag.pid; do
    if [[ -f "$logdir/$f" ]]; then
      kill "$(cat "$logdir/$f")" 2>/dev/null || true
      rm -f "$logdir/$f"
    fi
  done
  unset MEMORY_REF_RELAY_STARTED
}
