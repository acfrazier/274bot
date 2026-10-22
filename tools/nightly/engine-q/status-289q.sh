#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
RUNTIME="$ROOT/runtime"
PIDFILE="$RUNTIME/pids/engine.pid"
IDENTITY="$RUNTIME/pids/engine.identity"
ENGINE="$ROOT/engine"
if [ ! -f "$PIDFILE" ]; then
    printf '%s\n' 'Engine Q: stopped (no pid file)'
    exit 1
fi
if [ ! -f "$IDENTITY" ]; then
    printf '%s\n' 'Engine Q: refusing incomplete process identity' >&2
    exit 1
fi
pid=$(cat "$PIDFILE")
case "$pid" in ''|0|*[!0-9]*) printf '%s\n' 'Engine Q: malformed PID file' >&2; exit 1 ;; esac
identity_pid=$(grep '^pid=' "$IDENTITY" | cut -d= -f2-)
if [ "$identity_pid" != "$pid" ]; then
    printf '%s\n' 'Engine Q: PID/identity mismatch' >&2
    exit 1
fi
if ! kill -0 "$pid" 2>/dev/null; then
    printf '%s\n' "Engine Q: stopped (stale pid $pid)"
    exit 1
fi
expected_command=$(grep '^command=' "$IDENTITY" | cut -d= -f2-)
expected_cwd=$(grep '^cwd=' "$IDENTITY" | cut -d= -f2-)
expected_start=$(grep '^start_time=' "$IDENTITY" | cut -d= -f2-)
if [ -z "$expected_command" ] || [ -z "$expected_cwd" ] || [ -z "$expected_start" ]; then
    printf '%s\n' 'Engine Q: incomplete process identity' >&2
    exit 1
fi
case "$expected_command" in *"node --import tsx src/app.ts"*) ;; *) printf '%s\n' 'Engine Q: unexpected command identity' >&2; exit 1 ;; esac
if [ "$expected_cwd" != "$ENGINE" ]; then
    printf '%s\n' 'Engine Q: unexpected cwd identity' >&2
    exit 1
fi
command=$(ps -p "$pid" -o command= | tr -s ' ' | tr -d '\n')
cwd=$(lsof -a -p "$pid" -d cwd -Fn 2>/dev/null | grep '^n' | cut -c2- | head -1)
start_time=$(ps -p "$pid" -o lstart= | tr -s ' ' | tr -d '\n')
if [ "$command" != "$expected_command" ] || [ "$cwd" != "$expected_cwd" ] || [ "$start_time" != "$expected_start" ]; then
    printf '%s\n' "Engine Q: refusing foreign/reused pid $pid (identity mismatch)" >&2
    exit 1
fi
printf '%s\n' "Engine Q: running pid=$pid"
ps -p "$pid" -o pid=,ppid=,command=
listeners=$(lsof -nP -a -p "$pid" -iTCP -sTCP:LISTEN 2>/dev/null || true)
printf '%s\n' "$listeners"
unexpected=$(printf '%s\n' "$listeners" | grep 'TCP ' | grep -vE '127\.0\.0\.1:(1180|44694|9988)[[:space:]]' || true)
if [ -n "$unexpected" ]; then
    printf '%s\n' 'Engine Q: unexpected listener in owned process' >&2
    printf '%s\n' "$unexpected" >&2
    exit 1
fi
for port in 1180 44694 9988; do
    printf '%s' "listener 127.0.0.1:$port "
    printf '%s\n' "$listeners" | grep -E "127\.0\.0\.1:$port[[:space:]]|127\.0\.0\.1:$port->" >/dev/null || { printf '%s\n' 'missing'; exit 1; }
    printf '%s\n' 'present'
done
for url in http://127.0.0.1:1180/rs2.cgi http://127.0.0.1:9988/prometheus; do
    printf '%s ' "$url"
    code=$(curl --connect-timeout 2 --max-time 5 --silent --show-error --output /dev/null --write-out '%{http_code}' "$url") || { printf '%s\n' 'unreachable'; exit 1; }
    printf '%s\n' "$code"
    [ "$code" = 200 ] || exit 1
done
