#!/bin/bash
set -euo pipefail

CONFIG="$HOME/.config/274bot-nightly/malbec-mirror.env"
if [ ! -r "$CONFIG" ]; then
    printf '%s\n' "malbec-mirror: missing $CONFIG" >&2
    exit 78
fi
set -a
. "$CONFIG"
set +a

HOST=${NIGHTLY_MALBEC_HOST:?set NIGHTLY_MALBEC_HOST in $CONFIG}
USER_AT=${NIGHTLY_MALBEC_USER:-bot}
KEY=${NIGHTLY_MALBEC_KEY:?set NIGHTLY_MALBEC_KEY in $CONFIG}
KNOWN_HOSTS=${NIGHTLY_MALBEC_KNOWN_HOSTS:?set NIGHTLY_MALBEC_KNOWN_HOSTS in $CONFIG}
DEST=${NIGHTLY_MALBEC_DEST:?set NIGHTLY_MALBEC_DEST in $CONFIG}
RSYNC=${NIGHTLY_MALBEC_RSYNC:-$HOME/.local/libexec/274bot-standby/rsync}

log() { printf '%s %s\n' "$(date '+%Y-%m-%dT%H:%M:%S%z')" "$*"; }
mkdir -p "$DEST"
LOCK="$DEST/.malbec-mirror.lock"
if ! lock_err=$(mkdir "$LOCK" 2>&1); then
    if [ -d "$LOCK" ]; then
        log "another mirror pull holds $LOCK; exiting"
        exit 0
    fi
    log "cannot create $LOCK: $lock_err"
    exit 1
fi
trap 'rmdir "$LOCK" 2>/dev/null || true' EXIT

ssh_cmd="ssh -i $KEY -o IdentitiesOnly=yes -o BatchMode=yes -o ConnectTimeout=15 -o UserKnownHostsFile=$KNOWN_HOSTS -o StrictHostKeyChecking=yes"
listing=$("$RSYNC" -e "$ssh_cmd" --list-only "$USER_AT@$HOST:/")
runs=$(printf '%s\n' "$listing" | awk '$1 ~ /^d/ && $NF != "." && $NF != ".." {print $NF}')
copied=0
waiting=0
preserved=0
for run in $runs; do
    if [[ ! "$run" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}-[A-Za-z0-9][A-Za-z0-9._-]*-[0-9a-f]{9}(-(quest|274|windows))?(-r([2-9]|[1-9][0-9]+))?$ ]]; then
        continue
    fi
    destination="$DEST/$run"
    if [ -e "$destination" ]; then
        if [ -f "$destination/SHA256SUMS" ]; then
            if (cd "$destination" && shasum -a 256 -c SHA256SUMS >/dev/null); then
                log "$run: already mirrored; checksums verified"
            else
                log "$run: existing mirror checksum verification failed; preserved"
                exit 1
            fi
        else
            log "$run: already exists without a checksum manifest; preserved"
        fi
        preserved=$((preserved + 1))
        continue
    fi
    remote_listing=$("$RSYNC" -e "$ssh_cmd" --list-only "$USER_AT@$HOST:/$run/")
    if ! printf '%s\n' "$remote_listing" | awk '$NF == "DONE" { found=1 } END { exit !found }'; then
        waiting=$((waiting + 1))
        log "$run: no DONE; left on malbec"
        continue
    fi
    if ! printf '%s\n' "$remote_listing" | awk '$NF == "SHA256SUMS" { found=1 } END { exit !found }'; then
        waiting=$((waiting + 1))
        log "$run: no malbec SHA256SUMS; left on malbec"
        continue
    fi
    incoming="$DEST/.incoming-$run.$$"
    rm -rf "$incoming"
    mkdir "$incoming"
    if ! "$RSYNC" -e "$ssh_cmd" -rt --chmod=Du=rwx,Dgo=rx,Fu=rw,Fgo=r \
            "$USER_AT@$HOST:/$run/" "$incoming/"; then
        rm -rf "$incoming"
        log "$run: transfer failed; source unchanged"
        exit 1
    fi
    if [ ! -f "$incoming/DONE" ]; then
        rm -rf "$incoming"
        waiting=$((waiting + 1))
        log "$run: DONE vanished before transfer; source left unchanged"
        continue
    fi
    if [ ! -f "$incoming/SHA256SUMS" ]; then
        rm -rf "$incoming"
        log "$run: transferred source has no SHA256SUMS; refusing the mirror"
        exit 1
    fi
    links=$(find "$incoming" -type l -print)
    if [ -n "$links" ]; then
        rm -rf "$incoming"
        log "$run: unexpected symlinks in transfer; refusing the mirror"
        exit 1
    fi
    if ! (cd "$incoming" && shasum -a 256 -c SHA256SUMS >/dev/null); then
        rm -rf "$incoming"
        log "$run: malbec SHA256SUMS verification failed; refusing the mirror"
        exit 1
    fi
    if ! mv -n "$incoming" "$destination"; then
        rm -rf "$incoming"
        log "$run: destination move failed"
        exit 1
    fi
    if [ -e "$incoming" ]; then
        rm -rf "$incoming"
        log "$run: destination appeared concurrently; preserved existing data"
        exit 1
    fi
    log "$run: mirrored after malbec SHA256SUMS verification"
    copied=$((copied + 1))
done
log "done: $copied copied, $waiting unfinished, $preserved already present"
