#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
ENGINE="$ROOT/engine"
RUNTIME="$ROOT/runtime"
PIDFILE="$RUNTIME/pids/engine.pid"
IDENTITY="$RUNTIME/pids/engine.identity"
LOG="$RUNTIME/logs/engine.log"
if [ -f "$PIDFILE" ] || [ -f "$IDENTITY" ]; then
    if [ ! -f "$PIDFILE" ] || [ ! -f "$IDENTITY" ]; then
        printf '%s\n' 'refusing incomplete Engine Q process identity; inspect runtime/pids' >&2
        exit 1
    fi
    pid=$(tr -d '[:space:]' <"$PIDFILE")
    case "$pid" in ''|0|*[!0-9]*) printf '%s\n' 'refusing malformed Engine Q PID file' >&2; exit 1 ;; esac
    identity_pid=$(grep '^pid=' "$IDENTITY" | cut -d= -f2-)
    expected_command=$(grep '^command=' "$IDENTITY" | cut -d= -f2-)
    expected_cwd=$(grep '^cwd=' "$IDENTITY" | cut -d= -f2-)
    expected_start=$(grep '^start_time=' "$IDENTITY" | cut -d= -f2-)
    [ "$identity_pid" = "$pid" ] && [ -n "$expected_command" ] && [ -n "$expected_cwd" ] && [ -n "$expected_start" ] || { printf '%s\n' 'refusing incomplete or inconsistent Engine Q identity' >&2; exit 1; }
    case "$expected_command" in *"node --import tsx src/app.ts"*) ;; *) printf '%s\n' 'refusing unexpected Engine Q command identity' >&2; exit 1 ;; esac
    [ "$expected_cwd" = "$ENGINE" ] || { printf '%s\n' 'refusing unexpected Engine Q cwd identity' >&2; exit 1; }
    if kill -0 "$pid" 2>/dev/null; then
        command=$(ps -p "$pid" -o command= | tr -s ' ' | tr -d '\n')
        cwd=$(lsof -a -p "$pid" -d cwd -Fn 2>/dev/null | grep '^n' | cut -c2- | head -1)
        start_time=$(ps -p "$pid" -o lstart= | tr -s ' ' | tr -d '\n')
        if [ "$command" != "$expected_command" ] || [ "$cwd" != "$expected_cwd" ] || [ "$start_time" != "$expected_start" ]; then
            printf '%s\n' "Engine Q: refusing foreign/reused live PID $pid; inspect runtime/pids/engine.identity" >&2
            exit 1
        fi
        printf '%s\n' "Engine Q already has owned live PID $pid; run ./status-289q.sh" >&2
        exit 1
    fi
    rm -f "$PIDFILE" "$IDENTITY"
fi
for port in 1180 44694 9988 44700 46299 44701; do
    if lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; then
        printf '%s\n' "port $port is already listening; refusing to start Engine Q" >&2
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
    printf '%s\n' "Engine Q exited; see $LOG" >&2
    exit 1
fi
command=$(ps -p "$pid" -o command= | tr -s ' ' | tr -d '\n')
cwd=$(lsof -a -p "$pid" -d cwd -Fn 2>/dev/null | grep '^n' | cut -c2- | head -1)
case "$command" in *"node --import tsx src/app.ts"*) : ;; *) printf '%s\n' "Engine Q start identity mismatch: $command; refusing signal" >&2; exit 1 ;; esac
case "$cwd" in "$ENGINE") : ;; *) printf '%s\n' "Engine Q cwd identity mismatch: $cwd; refusing signal" >&2; exit 1 ;; esac
start_time=$(ps -p "$pid" -o lstart= | tr -s ' ' | tr -d '\n')
{
    printf '%s\n' "pid=$pid"
    printf '%s\n' "cwd=$cwd"
    printf '%s\n' "command=$command"
    printf '%s\n' "start_time=$start_time"
} >"$IDENTITY"
printf '%s\n' "Engine Q started (pid $pid)"
printf '%s\n' 'Game: 127.0.0.1:44694'
printf '%s\n' 'HTTP: http://127.0.0.1:1180/rs2.cgi'
printf '%s\n' 'Management: http://127.0.0.1:9988/prometheus'
ready_log="$RUNTIME/logs/start-readiness.log"
for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30; do
    if "$ROOT/status-289q.sh" >"$ready_log" 2>&1; then
        printf '%s\n' 'Engine Q readiness checks passed'
        exit 0
    fi
    sleep 1
done
printf '%s\n' "Engine Q did not become ready; see $ready_log and $LOG" >&2
exit 1
