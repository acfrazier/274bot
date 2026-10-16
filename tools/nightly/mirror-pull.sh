#!/bin/bash
# Review-machine side of the 274bot nightly: mirror the last N days of runs
# from the archive host's dataset into a local folder and prune older runs
# there. The archive keeps everything; this folder is a rolling window.
#
# Runs as a user service (274bot-nightly-mirror.service). The key is a
# dedicated read-only rrsync key on the archive host; it cannot open a shell
# or read outside the nightly dataset.
set -euo pipefail

ARCHIVE_HOST=${NIGHTLY_ARCHIVE_HOST:?set NIGHTLY_ARCHIVE_HOST (user@host of the archive)}
KEY=${NIGHTLY_MIRROR_KEY:?set NIGHTLY_MIRROR_KEY (read-only rrsync key)}
KNOWN_HOSTS=${NIGHTLY_KNOWN_HOSTS:?set NIGHTLY_KNOWN_HOSTS (pinned host key of the archive)}
DEST=${NIGHTLY_MIRROR_DIR:?set NIGHTLY_MIRROR_DIR (local mirror folder)}
DAYS=${NIGHTLY_MIRROR_DAYS:-7}

log() { printf '%s %s\n' "$(date -Is)" "$*"; }

ssh_cmd="ssh -i $KEY -o IdentitiesOnly=yes -o BatchMode=yes -o ConnectTimeout=15 -o UserKnownHostsFile=$KNOWN_HOSTS -o StrictHostKeyChecking=yes"
cutoff=$(date -d "-$((DAYS - 1)) days" +%F)
mkdir -p "$DEST"

mapfile -t runs < <(rsync -e "$ssh_cmd" --list-only "$ARCHIVE_HOST:/" | awk '$1 ~ /^d/ && $NF != "." {print $NF}')

copied=0
for run in "${runs[@]}"; do
    [[ "$run" =~ ^([0-9]{4}-[0-9]{2}-[0-9]{2})-[A-Za-z0-9._-]+$ ]] || continue
    [[ "${BASH_REMATCH[1]}" < "$cutoff" ]] && continue
    # Runs are immutable once archived; a complete local copy is left alone.
    [ -f "$DEST/$run/DONE" ] && continue
    rsync -e "$ssh_cmd" -rt --chmod=Du=rwx,Dgo=rx,Fu=rw,Fgo=r "$ARCHIVE_HOST:/$run/" "$DEST/.incoming-$run/"
    rm -rf -- "${DEST:?}/$run"
    mv -- "$DEST/.incoming-$run" "$DEST/$run"
    log "$run: mirrored"
    copied=$((copied + 1))
done

pruned=0
for dir in "$DEST"/*/; do
    run=$(basename "$dir")
    [[ "$run" =~ ^([0-9]{4}-[0-9]{2}-[0-9]{2})- ]] || continue
    if [[ "${BASH_REMATCH[1]}" < "$cutoff" ]]; then
        rm -rf -- "${DEST:?}/$run"
        log "$run: pruned (older than $DAYS days)"
        pruned=$((pruned + 1))
    fi
done
log "done: $copied mirrored, $pruned pruned, window starts $cutoff"
