#!/bin/sh
# Start the revision-274 engine for one nightly run. Twin of the 289
# nightly engine's start script: the process identity (pid, command,
# cwd, start time) is recorded, and no signal is ever sent to a pid
# whose identity does not match it exactly. Listeners must end up on
# loopback only; status-274.sh checks that before readiness passes.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
ENGINE="$ROOT/engine"
RUNTIME="$ROOT/runtime"
PIDFILE="$RUNTIME/pids/engine.pid"
IDENTITY="$RUNTIME/pids/engine.identity"
LOG="$RUNTIME/logs/engine.log"
if [ -f "$PIDFILE" ] || [ -f "$IDENTITY" ]; then
    if [ ! -f "$PIDFILE" ] || [ ! -f "$IDENTITY" ]; then
        printf '%s\n' 'refusing incomplete 274 process identity; remove stale runtime/pids files' >&2
        exit 1
    fi
    pid=$(tr -d '[:space:]' <"$PIDFILE")
    case "$pid" in ''|0|*[!0-9]*) printf '%s\n' 'refusing malformed 274 PID file' >&2; exit 1 ;; esac
    identity_pid=$(grep '^pid=' "$IDENTITY" | cut -d= -f2-)
    expected_command=$(grep '^command=' "$IDENTITY" | cut -d= -f2-)
    expected_cwd=$(grep '^cwd=' "$IDENTITY" | cut -d= -f2-)
    expected_start=$(grep '^start_time=' "$IDENTITY" | cut -d= -f2-)
    [ "$identity_pid" = "$pid" ] && [ -n "$expected_command" ] && [ -n "$expected_cwd" ] && [ -n "$expected_start" ] || { printf '%s\n' 'refusing incomplete or inconsistent 274 process identity' >&2; exit 1; }
    case "$expected_command" in *"node --import tsx src/app.ts"*) ;; *) printf '%s\n' 'refusing unexpected 274 command identity' >&2; exit 1 ;; esac
    [ "$expected_cwd" = "$ENGINE" ] || { printf '%s\n' 'refusing unexpected 274 cwd identity' >&2; exit 1; }
    if kill -0 "$pid" 2>/dev/null; then
        command=$(ps -p "$pid" -o command= | tr -s ' ' | tr -d '\n')
        cwd=$(lsof -a -p "$pid" -d cwd -Fn 2>/dev/null | grep '^n' | cut -c2- | head -1)
        start_time=$(ps -p "$pid" -o lstart= | tr -s ' ' | tr -d '\n')
        if [ "$command" != "$expected_command" ] || [ "$cwd" != "$expected_cwd" ] || [ "$start_time" != "$expected_start" ]; then
            printf '%s\n' "274 engine: refusing foreign/reused live pid $pid; inspect runtime/pids/engine.identity" >&2
            exit 1
        fi
        printf '%s\n' "274 engine already has an owned live PID (pid $pid); run ./status-274.sh" >&2
        exit 1
    fi
    rm -f "$PIDFILE" "$IDENTITY"
fi
for port in 80 43594 8898 43500 45099 43501; do
    if lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; then
        printf '%s\n' "port $port is already listening; refusing to start" >&2
        exit 1
    fi
done
mkdir -p "$RUNTIME/logs" "$RUNTIME/pids"
nohup sh -c "cd \"$ENGINE\" && exec node --import tsx src/app.ts" >>"$LOG" 2>&1 &
pid=$!
printf '%s\n' "$pid" >"$PIDFILE"
sleep 2
pid=$(cat "$PIDFILE")
if ! kill -0 "$pid" 2>/dev/null; then
    printf '%s\n' "274 engine exited; see $LOG" >&2
    exit 1
fi
command=$(ps -p "$pid" -o command= | tr -s ' ' | tr -d '\n')
cwd=$(lsof -a -p "$pid" -d cwd -Fn 2>/dev/null | grep '^n' | cut -c2- | head -1)
case "$command" in *"node --import tsx src/app.ts"*) : ;; *) printf '%s\n' "274 engine start identity mismatch: $command; refusing signal" >&2; exit 1;; esac
case "$cwd" in "$ENGINE") : ;; *) printf '%s\n' "274 engine cwd identity mismatch: $cwd; refusing signal" >&2; exit 1;; esac
start_time=$(ps -p "$pid" -o lstart= | tr -s ' ' | tr -d '\n')
{
    printf '%s\n' "pid=$pid"
    printf '%s\n' "cwd=$cwd"
    printf '%s\n' "command=$command"
    printf '%s\n' "start_time=$start_time"
} >"$IDENTITY"
printf '%s\n' "274 engine started (pid $pid)"
printf '%s\n' "HTTP: http://127.0.0.1/rs2.cgi"
printf '%s\n' "Management: http://127.0.0.1:8898/setup"
ready_log="$RUNTIME/logs/start-readiness.log"
for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30; do
    if "$ROOT/status-274.sh" >"$ready_log" 2>&1; then
        printf '%s\n' '274 engine readiness checks passed'
        exit 0
    fi
    sleep 1
done
printf '%s\n' "274 engine did not become ready; see $ready_log" >&2
exit 1
