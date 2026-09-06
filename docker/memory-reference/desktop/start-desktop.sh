#!/usr/bin/env bash
# Optional lightweight desktop for operator visibility into panel/TUI work.
# Stack: Xvfb + Openbox + x11vnc + websockify/noVNC.
# Off unless MEMORY_REF_DESKTOP=1. Account helper CPU/RSS separately from app cells.
set -euo pipefail

if [[ "${MEMORY_REF_DESKTOP_STARTED:-}" == "1" ]]; then
  exit 0
fi

for bin in Xvfb openbox x11vnc websockify; do
  if ! command -v "$bin" >/dev/null 2>&1; then
    echo "desktop: missing $bin (image needs WITH_DESKTOP=1)" >&2
    exit 1
  fi
done

DISPLAY_NUM="${DISPLAY:-:99}"
export DISPLAY="$DISPLAY_NUM"
GEOM="${VNC_GEOMETRY:-1280x800}"
DEPTH="${VNC_DEPTH:-24}"
VNC_PORT="${VNC_PORT:-5900}"
NOVNC_PORT="${NOVNC_PORT:-6080}"
LOGDIR="${MEMORY_REF_DESKTOP_LOGDIR:-/tmp/memory-ref-desktop}"
mkdir -p "$LOGDIR"

# Virtual framebuffer — software only. Not a GPU passthrough claim.
Xvfb "$DISPLAY" -screen 0 "${GEOM}x${DEPTH}" -ac +extension GLX +render -noreset \
  >"$LOGDIR/xvfb.log" 2>&1 &
echo $! >"$LOGDIR/xvfb.pid"
sleep 0.5

# Minimal WM so panel windows are manageable.
openbox >"$LOGDIR/openbox.log" 2>&1 &
echo $! >"$LOGDIR/openbox.pid"

# VNC scrapes Xvfb. Listen on all interfaces inside the container network
# namespace; compose publishes 127.0.0.1:6080 only on the host.
x11vnc -display "$DISPLAY" -rfbport "$VNC_PORT" -forever -shared -nopw -localhost false \
  >"$LOGDIR/x11vnc.log" 2>&1 &
echo $! >"$LOGDIR/x11vnc.pid"

# noVNC web root: Debian bookworm uses /usr/share/novnc
NOVNC_WEB="${NOVNC_WEB:-}"
if [[ -z "$NOVNC_WEB" ]]; then
  for cand in /usr/share/novnc /usr/share/novnc/; do
    if [[ -d "$cand" ]]; then NOVNC_WEB="$cand"; break; fi
  done
fi
if [[ -z "${NOVNC_WEB}" || ! -d "$NOVNC_WEB" ]]; then
  echo "desktop: noVNC web root not found" >&2
  exit 1
fi

websockify --web="$NOVNC_WEB" "0.0.0.0:${NOVNC_PORT}" "127.0.0.1:${VNC_PORT}" \
  >"$LOGDIR/websockify.log" 2>&1 &
echo $! >"$LOGDIR/websockify.pid"

export MEMORY_REF_DESKTOP_STARTED=1
cat <<EOF >&2
memory-ref desktop: DISPLAY=$DISPLAY geometry=${GEOM}x${DEPTH}
  VNC :${VNC_PORT}  noVNC http://127.0.0.1:${NOVNC_PORT}/vnc.html
  Helpers are NOT part of clean TUI 2c/4GiB app budgets — sample separately.
  Software X + optional BOT_CPU=1; Docker Desktop on macOS ≠ native Linux GPU.
EOF
