#!/bin/sh
set -eu

if [ "${NIGHTLY_ENABLE_QUEST:-}" != 1 ]; then
    printf '%s\n' 'nightly: quest job disabled; NIGHTLY_ENABLE_QUEST is not 1'
    exit 0
fi
if [ "${NIGHTLY_REVISION:-289}" != 289 ]; then
    printf 'nightly: refusing quest job for NIGHTLY_REVISION=%s\n' "$NIGHTLY_REVISION" >&2
    exit 1
fi

SCRIPT_DIR=$(CDPATH= cd "$(dirname "$0")" && pwd)
exec /usr/bin/env \
    NIGHTLY_JOB=quest \
    NIGHTLY_REVISION=289 \
    NIGHTLY_ENGINE=local \
    NIGHTLY_PROFILE=local-289q \
    NIGHTLY_ENGINE_HOST=127.0.0.1 \
    NIGHTLY_GAME_PORT=44694 \
    NIGHTLY_HTTP_PORT=1180 \
    NIGHTLY_ENGINE_LABEL='Engine Q' \
    NIGHTLY_ENGINE_SCRIPT_REVISION=289q \
    NIGHTLY_RUN_SUFFIX=-quest \
    NIGHTLY_NAME_PREFIX=nq \
    NIGHTLY_PANEL_NAME_PREFIX=nqp \
    NIGHTLY_STEPS=tui \
    NIGHTLY_PARALLEL=1 \
    NIGHTLY_PANEL_PARALLEL=1 \
    python3 "$SCRIPT_DIR/../run_nightly.py"
