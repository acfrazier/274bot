#!/bin/sh
# Stop the revision-274 engine start-274.sh started. The pid is
# signalled only when its recorded identity still matches exactly.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
ENGINE="$ROOT/engine"
PIDFILE="$ROOT/runtime/pids/engine.pid"
IDENTITY="$ROOT/runtime/pids/engine.identity"
if [ ! -f "$PIDFILE" ]; then
    printf '%s\n' '274 engine is not running'
    exit 1
fi
if [ ! -f "$IDENTITY" ]; then
    printf '%s\n' 'refusing incomplete 274 process identity' >&2
    exit 1
fi
pid=$(cat "$PIDFILE")
case "$pid" in ''|0|*[!0-9]*) printf '%s\n' 'refusing malformed 274 PID file' >&2; exit 1 ;; esac
identity_pid=$(grep '^pid=' "$IDENTITY" | cut -d= -f2-)
if [ "$identity_pid" != "$pid" ]; then printf '%s\n' 'refusing PID/identity mismatch; no signal sent' >&2; exit 1; fi
if ! kill -0 "$pid" 2>/dev/null; then
    rm -f "$PIDFILE"
    printf '%s\n' "274 engine already stopped (stale pid $pid)"
    rm -f "$IDENTITY"
    exit 1
fi
expected_command=$(grep '^command=' "$IDENTITY" | cut -d= -f2-)
expected_cwd=$(grep '^cwd=' "$IDENTITY" | cut -d= -f2-)
expected_start=$(grep '^start_time=' "$IDENTITY" | cut -d= -f2-)
if [ -z "$expected_command" ] || [ -z "$expected_cwd" ] || [ -z "$expected_start" ]; then printf '%s\n' 'refusing incomplete 274 process identity' >&2; exit 1; fi
case "$expected_command" in *"node --import tsx src/app.ts"*) ;; *) printf '%s\n' 'refusing unexpected command identity; no signal sent' >&2; exit 1 ;; esac
if [ "$expected_cwd" != "$ENGINE" ]; then printf '%s\n' 'refusing unexpected cwd identity; no signal sent' >&2; exit 1; fi
command=$(ps -p "$pid" -o command= | tr -s ' ' | tr -d '\n')
cwd=$(/usr/bin/sudo -n /usr/bin/lsof -a -p "$pid" -d cwd -Fn 2>/dev/null | grep '^n' | cut -c2- | head -1)
start_time=$(ps -p "$pid" -o lstart= | tr -s ' ' | tr -d '\n')
if [ "$command" != "$expected_command" ] || [ "$cwd" != "$expected_cwd" ] || [ "$start_time" != "$expected_start" ]; then
    printf '%s\n' "refusing pid $pid: start identity mismatch" >&2
    exit 1
fi
kill -TERM "$pid"
for _ in 1 2 3 4 5 6 7 8 9 10; do
    if ! kill -0 "$pid" 2>/dev/null; then rm -f "$PIDFILE" "$IDENTITY"; printf '%s\n' "274 engine stopped (pid $pid)"; exit 0; fi
    sleep 1
done
printf '%s\n' "274 engine did not stop after SIGTERM; pid $pid remains" >&2
exit 1
