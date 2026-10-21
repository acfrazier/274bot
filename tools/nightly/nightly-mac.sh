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
: "${NIGHTLY_MAC_ROOT:?set NIGHTLY_MAC_ROOT in $CONFIG}"
: "${NIGHTLY_MAC_RUNS:?set NIGHTLY_MAC_RUNS in $CONFIG}"
: "${NIGHTLY_MAC_CATALOG:?set NIGHTLY_MAC_CATALOG in $CONFIG}"
: "${NIGHTLY_MAC_ENGINE_A:?set NIGHTLY_MAC_ENGINE_A in $CONFIG}"
: "${NIGHTLY_MAC_ENGINE_Q:?set NIGHTLY_MAC_ENGINE_Q in $CONFIG}"
: "${NIGHTLY_MAC_QUIET_LOCK:?set NIGHTLY_MAC_QUIET_LOCK in $CONFIG}"
: "${NIGHTLY_MAC_ISOHOME:?set NIGHTLY_MAC_ISOHOME in $CONFIG}"
: "${NIGHTLY_MAC_PYTHON:?set NIGHTLY_MAC_PYTHON in $CONFIG}"

RUNNER="$NIGHTLY_MAC_RUNNER/tools/nightly/run_nightly.py"
STATUS="${NIGHTLY_MAC_STATUS:-$NIGHTLY_MAC_RUNS/.mac-nightly-running}"
LOCK_DIR="$NIGHTLY_MAC_ROOT/.mac-nightly.lock"
STARTED=$(date -u '+%Y-%m-%dT%H:%M:%SZ')
mkdir -p "$NIGHTLY_MAC_ROOT" "$NIGHTLY_MAC_RUNS" "$(dirname "$STATUS")"

write_status() {
    local job="$1" state="$2" temporary="$STATUS.tmp.$$"
    printf 'pid=%s\njob=%s\nstate=%s\nstarted=%s\nupdated=%s\nquiet_lock=%s\n' \
        "$$" "$job" "$state" "$STARTED" "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" \
        "$NIGHTLY_MAC_QUIET_LOCK" >"$temporary"
    mv -f "$temporary" "$STATUS"
}
owned_lock=0
cleanup() {
    if [ "$owned_lock" -eq 1 ]; then
        rm -f "$STATUS" "$STATUS.tmp.$$"
        rmdir "$LOCK_DIR" 2>/dev/null || true
    fi
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

if [ -e "$NIGHTLY_MAC_QUIET_LOCK" ]; then
    printf '%s\n' "nightly-mac: skipped; MAC-QUIET.lock exists at $NIGHTLY_MAC_QUIET_LOCK"
    exit 0
fi
if ! mkdir "$LOCK_DIR" 2>/dev/null; then
    printf '%s\n' "nightly-mac: another run owns $LOCK_DIR"
    exit 0
fi
owned_lock=1

run_job() {
    local job="$1"
    if [ -e "$NIGHTLY_MAC_QUIET_LOCK" ]; then
        write_status "$job" 'skipped: MAC-QUIET.lock present'
        printf '%s\n' "nightly-mac: skipped $job; MAC-QUIET.lock appeared"
        return 0
    fi
    export NIGHTLY_ROOT="$NIGHTLY_MAC_ROOT"
    export NIGHTLY_RUNS_ROOT="$NIGHTLY_MAC_RUNS"
    export NIGHTLY_CATALOG="$NIGHTLY_MAC_CATALOG"
    export NIGHTLY_NAV_SNAPSHOT="$NIGHTLY_MAC_ROOT/nav-snapshot"
    export NIGHTLY_NAV_FIXTURE="$NIGHTLY_MAC_RUNNER/fixtures/bake-r2-a.navpack"
    export NIGHTLY_QUIET_LOCK="$NIGHTLY_MAC_QUIET_LOCK"
    export NIGHTLY_ISOHOME="$NIGHTLY_MAC_ISOHOME"
    export NIGHTLY_PYTHON="$NIGHTLY_MAC_PYTHON"
    export NIGHTLY_REVISION=289
    export NIGHTLY_BRANCHES=dev
    export NIGHTLY_ENGINE=remote
    export NIGHTLY_ENGINE_HOST=127.0.0.1
    export NIGHTLY_PANEL_PARALLEL=3
    export NIGHTLY_PARALLEL=4
    export NIGHTLY_TZ=America/New_York
    unset NIGHTLY_ENGINE_SCRIPT_REVISION || true
    if [ "$job" = main ]; then
        export NIGHTLY_JOB=main
        export NIGHTLY_STEPS=lanes,panel,tui
        export NIGHTLY_ENGINE_PROFILE=local-289
        export NIGHTLY_PROFILE=local-289
        export NIGHTLY_ENGINE_LABEL='Engine A'
        export NIGHTLY_GAME_PORT=44594 NIGHTLY_HTTP_PORT=1080
        export NIGHTLY_ENGINE_PRISTINE="$NIGHTLY_MAC_ENGINE_A"
        export NIGHTLY_ENGINE_RUN="$NIGHTLY_MAC_ROOT/engine-run"
        export NIGHTLY_RUN_SUFFIX=-mac NIGHTLY_NAME_PREFIX=nm NIGHTLY_PANEL_NAME_PREFIX=nmp
    else
        export NIGHTLY_JOB=quest
        export NIGHTLY_STEPS=panel,tui
        export NIGHTLY_PROFILE=local-289q
        export NIGHTLY_ENGINE_LABEL='Engine Q'
        export NIGHTLY_GAME_PORT=44694 NIGHTLY_HTTP_PORT=1180
        export NIGHTLY_ENGINE_PRISTINE="$NIGHTLY_MAC_ENGINE_Q"
        export NIGHTLY_ENGINE_RUN="$NIGHTLY_MAC_ROOT/engine-q-run"
        export NIGHTLY_ENGINE_SCRIPT_REVISION=289q
        export NIGHTLY_RUN_SUFFIX=-mac-quest NIGHTLY_NAME_PREFIX=nq NIGHTLY_PANEL_NAME_PREFIX=nqp
        export NIGHTLY_PARALLEL=1 NIGHTLY_PANEL_PARALLEL=1
    fi
    write_status "$job" running
    printf '%s\n' "nightly-mac: starting $job for dev with steps $NIGHTLY_STEPS"
    "$NIGHTLY_MAC_PYTHON" -u "$RUNNER" --branch dev
    write_status "$job" completed
    if [ -e "$NIGHTLY_MAC_QUIET_LOCK" ]; then
        write_status "$job" 'stopped at MAC-QUIET.lock'
        printf '%s\n' "nightly-mac: $job completed its current work; MAC-QUIET.lock prevents another job"
        return 2
    fi
    return 0
}

run_job main || status=$?
if [ "${status:-0}" -eq 2 ]; then
    exit 0
fi
run_job quest || status=$?
exit "${status:-0}"
