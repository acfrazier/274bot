#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
ENGINE="$ROOT/engine"
PIDFILE="$ROOT/runtime/pids/engine.pid"
IDENTITY="$ROOT/runtime/pids/engine.identity"
if [ ! -f "$PIDFILE" ] && [ ! -f "$IDENTITY" ]; then
    printf '%s\n' 'Engine Q is not running'
    exit 0
fi
if [ ! -f "$PIDFILE" ] || [ ! -f "$IDENTITY" ]; then
    printf '%s\n' 'refusing incomplete Engine Q process identity' >&2
    exit 1
fi
pid=$(cat "$PIDFILE")
case "$pid" in ''|0|*[!0-9]*) printf '%s\n' 'refusing malformed Engine Q PID file' >&2; exit 1 ;; esac
identity_pid=$(grep '^pid=' "$IDENTITY" | cut -d= -f2-)
if [ "$identity_pid" != "$pid" ]; then
    printf '%s\n' 'refusing Engine Q PID/identity mismatch; no signal sent' >&2
    exit 1
fi
expected_command=$(grep '^command=' "$IDENTITY" | cut -d= -f2-)
expected_cwd=$(grep '^cwd=' "$IDENTITY" | cut -d= -f2-)
expected_start=$(grep '^start_time=' "$IDENTITY" | cut -d= -f2-)
if [ -z "$expected_command" ] || [ -z "$expected_cwd" ] || [ -z "$expected_start" ]; then
    printf '%s\n' 'refusing incomplete Engine Q process identity' >&2
    exit 1
fi
case "$expected_command" in *"node --import tsx src/app.ts"*) ;; *) printf '%s\n' 'refusing unexpected Engine Q command identity; no signal sent' >&2; exit 1 ;; esac
if [ "$expected_cwd" != "$ENGINE" ]; then
    printf '%s\n' 'refusing unexpected Engine Q cwd identity; no signal sent' >&2
    exit 1
fi
if ! kill -0 "$pid" 2>/dev/null; then
    rm -f "$PIDFILE" "$IDENTITY"
    printf '%s\n' "Engine Q already stopped (stale pid $pid)"
    exit 0
fi
command=$(ps -p "$pid" -o command= | tr -s ' ' | tr -d '\n')
cwd=$(lsof -a -p "$pid" -d cwd -Fn 2>/dev/null | grep '^n' | cut -c2- | head -1)
start_time=$(ps -p "$pid" -o lstart= | tr -s ' ' | tr -d '\n')
if [ "$command" != "$expected_command" ] || [ "$cwd" != "$expected_cwd" ] || [ "$start_time" != "$expected_start" ]; then
    printf '%s\n' "refusing PID $pid: Engine Q start identity mismatch; no signal sent" >&2
    exit 1
fi
kill -TERM "$pid"
for _ in 1 2 3 4 5 6 7 8 9 10; do
    if ! kill -0 "$pid" 2>/dev/null; then
        rm -f "$PIDFILE" "$IDENTITY"
        printf '%s\n' "Engine Q stopped (pid $pid)"
        exit 0
    fi
    sleep 1
done
printf '%s\n' "Engine Q did not stop after SIGTERM; pid $pid remains" >&2
exit 1
