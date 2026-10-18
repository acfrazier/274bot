#!/bin/bash
# cellar side of the Windows nightly: pull every finished `-windows` run from the
# Windows nightly host into the ZFS dataset over SFTP, and snapshot the dataset
# after each new run. The same contract as cellar-pull.sh, without rsync: the
# Windows host has no rrsync, so its pull account is chrooted to the runs folder
# and its sshd forces `internal-sftp -R` (read-only).
#
# Runs as root on the Proxmox host from 274bot-nightly-pull-windows.service.
# Results flow one way: the host only serves its run directories read-only; it
# can never write to the dataset. A run that already exists on the dataset is
# never touched again.
set -euo pipefail

HOST=${NIGHTLY_WIN_HOST:-DESKTOP-SL99R6C.lan}
HOST_ALIAS=${NIGHTLY_WIN_HOST_ALIAS:-nightly-windows}
PULL_USER=${NIGHTLY_WIN_USER:-274bot-pull}
DATASET=${NIGHTLY_DATASET:-tank/274bot-nightly}
CONF_DIR=${NIGHTLY_CONF_DIR:-/etc/274bot-nightly}
KEY=${NIGHTLY_WIN_KEY:-$CONF_DIR/win_pull_ed25519}
KNOWN_HOSTS=${NIGHTLY_WIN_KNOWN_HOSTS:-$CONF_DIR/known_hosts}
# Where runs land; the dataset's mountpoint unless set (a test pull sets it).
DEST=${NIGHTLY_DEST:-}
# The runs folder as the SFTP server shows it: "" under the pull account's chroot.
REMOTE=${NIGHTLY_WIN_REMOTE_ROOT:-}
SNAPSHOT=${NIGHTLY_SNAPSHOT:-1}

log() { printf '%s %s\n' "$(date -Is)" "$*"; }

if [ -z "$DEST" ]; then
    DEST=$(zfs get -H -o value mountpoint "$DATASET")
fi
[ -d "$DEST" ] || { log "destination $DEST is missing (dataset $DATASET not mounted?)"; exit 1; }

# HostKeyAlias binds the pinned host key to the host, not to its DHCP address.
sftp_opts=(-q -i "$KEY" -o IdentitiesOnly=yes -o BatchMode=yes -o ConnectTimeout=15
           -o "HostKeyAlias=$HOST_ALIAS" -o "UserKnownHostsFile=$KNOWN_HOSTS" -o StrictHostKeyChecking=yes)
# One batch per call; '@' keeps sftp from echoing the command. A failing command
# ends the batch with a nonzero status.
sftp_batch() { sftp "${sftp_opts[@]}" -b - "$PULL_USER@$HOST"; }

# Top level of the chroot (= C:\274bot-nightly\runs on the host).
if ! listing=$(printf '@ls -1a %s/\n' "$REMOTE" | sftp_batch 2>&1); then
    log "host $HOST: listing failed (host down, BotTest asleep, or key refused): ${listing//$'\n'/ }"
    exit 0
fi

pulled=0
while IFS= read -r run; do
    run=${run%$'\r'}
    run=${run%/}
    run=${run##*/}
    [[ "$run" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}-[A-Za-z0-9._-]+-windows(-r[0-9]+)?$ ]] || continue
    [ -e "$DEST/$run" ] && continue
    # Only finished runs: the runner writes DONE last.
    if ! printf '@ls %s/%s/DONE\n' "$REMOTE" "$run" | sftp_batch >/dev/null 2>&1; then
        log "$run: not finished yet"
        continue
    fi
    incoming="$DEST/.incoming-$run"
    rm -rf -- "$incoming"
    # Regular files and directories only: sftp's recursive get skips symlinks
    # and refuses names with '/' or '..' from the server.
    printf '@get -R -p %s/%s %s\n' "$REMOTE" "$run" "$incoming" | sftp_batch >/dev/null
    [ -f "$incoming/DONE" ] || { log "$run: copy has no DONE; left in $incoming"; continue; }
    # Windows reports every file as executable; store plain 0644/0755.
    find "$incoming" -type d -exec chmod 0755 {} +
    find "$incoming" -type f -exec chmod 0644 {} +
    mv -- "$incoming" "$DEST/$run"
    chmod -R a-w -- "$DEST/$run"
    if [ "$SNAPSHOT" = 1 ]; then
        zfs snapshot "$DATASET@$run"
        log "$run: pulled and snapshotted $DATASET@$run"
    else
        log "$run: pulled into $DEST (no snapshot)"
    fi
    pulled=$((pulled + 1))
done <<< "$listing"
log "done: $pulled new run(s)"
