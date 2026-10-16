#!/bin/bash
# cellar side of the 274bot nightly: pull every finished run from the
# nightly VM into the ZFS dataset and snapshot the dataset after each new run.
#
# Runs as root on the Proxmox host from 274bot-nightly-pull.service.
# Results flow one way: the VM only ever serves its run directories through a
# read-only rrsync key; it can never write to the dataset. A run directory that
# already exists on the dataset is never touched again.
set -euo pipefail

VMID=${NIGHTLY_VMID:-202}
VM_USER=${NIGHTLY_VM_USER:-bot}
DATASET=${NIGHTLY_DATASET:-tank/274bot-nightly}
CONF_DIR=${NIGHTLY_CONF_DIR:-/etc/274bot-nightly}
KEY=$CONF_DIR/pull_ed25519
KNOWN_HOSTS=$CONF_DIR/known_hosts

log() { printf '%s %s\n' "$(date -Is)" "$*"; }

dest=$(zfs get -H -o value mountpoint "$DATASET")
[ -d "$dest" ] || { log "dataset $DATASET is not mounted"; exit 1; }

# The VM takes its address from DHCP; ask its guest agent instead of pinning it.
vm_ip() {
    qm guest cmd "$VMID" network-get-interfaces | python3 -c '
import json, sys
for iface in json.load(sys.stdin):
    if iface.get("name") == "lo":
        continue
    for addr in iface.get("ip-addresses", []):
        if addr.get("ip-address-type") == "ipv4":
            print(addr["ip-address"])
            sys.exit(0)
sys.exit(1)'
}

if ! ip=$(vm_ip); then
    log "VM $VMID has no guest-agent IPv4 address (stopped?); nothing pulled"
    exit 0
fi

# HostKeyAlias binds the pinned host key to the VM, not to its DHCP address.
ssh_cmd="ssh -i $KEY -o IdentitiesOnly=yes -o BatchMode=yes -o ConnectTimeout=15 -o HostKeyAlias=nightly-vm-$VMID -o UserKnownHostsFile=$KNOWN_HOSTS -o StrictHostKeyChecking=yes"
remote="$VM_USER@$ip"

# Top-level entries of the VM's run root (rrsync roots "/" at /srv/nightly/runs).
mapfile -t runs < <(rsync -e "$ssh_cmd" --list-only "$remote:/" | awk '$1 ~ /^d/ && $NF != "." {print $NF}')

pulled=0
for run in "${runs[@]}"; do
    case "$run" in
        [0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]-*) ;;
        *) continue ;;
    esac
    [[ "$run" =~ ^[A-Za-z0-9._-]+$ ]] || { log "skipping unexpected name $run"; continue; }
    [ -e "$dest/$run" ] && continue
    # Only finished runs: the runner writes DONE last.
    if ! rsync -e "$ssh_cmd" --list-only "$remote:/$run/DONE" >/dev/null 2>&1; then
        log "$run: not finished yet"
        continue
    fi
    incoming="$dest/.incoming-$run"
    rm -rf -- "$incoming"
    # -rt without -l: symlinks from the VM are never recreated on the pool.
    rsync -e "$ssh_cmd" -rt --chmod=Du=rwx,Dgo=rx,Fu=rw,Fgo=r "$remote:/$run/" "$incoming/"
    mv -- "$incoming" "$dest/$run"
    chmod -R a-w -- "$dest/$run"
    zfs snapshot "$DATASET@$run"
    log "$run: pulled and snapshotted $DATASET@$run"
    pulled=$((pulled + 1))
done
log "done: $pulled new run(s)"
