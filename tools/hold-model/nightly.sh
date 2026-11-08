#!/usr/bin/env bash
# Nightly full profile runs of tools/hold-model (ADR 0017), on Linux.
#
# Builds the release binary, then runs every profile one at a time, each in
# its own `systemd-run --user --scope -p MemoryMax=16G -p MemorySwapMax=0`
# scope, so that a run that grows past 16 GB is killed alone. It writes one
# summary line per profile and exits non-zero when any run does not end clean
# and complete (exit 0 of the model).
#
# usage: nightly.sh [OUTDIR]
#   OUTDIR  where the raw output of each run and `summary.txt` go. Default:
#           $HOLD_MODEL_NIGHTLY_OUT, else $HOME/.local/state/274bot-hold-model/<UTC date>.
#   CARGO_TARGET_DIR  where the release binary is built. Default: OUTDIR/target
#           (the crate has no dependency; a cold build takes seconds).
#
# Exit code: 0 every profile ended clean and complete; 1 a profile found a
# violation, hit a cap or was killed; 2 the build, the scope or a bad
# argument stopped the run.
#
# The horizons are the ones of the table in README.md ("Expected results of
# the profiles"). Every profile ends clean and complete at its horizon since
# the fix of finding F2 (the journal click renews the quiet lease's hold
# bound).

set -u

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo=$(cd "$here/../.." && pwd)
stamp=$(date -u +%Y%m%dT%H%M%SZ)
out=${1:-${HOLD_MODEL_NIGHTLY_OUT:-$HOME/.local/state/274bot-hold-model/$stamp}}

# Profile and horizon. One run at a time: the longest needs about 7 GB.
profiles=(
    "smoke 8"
    "modal 14"
    "quiet 14"
    "compat 14"
    "all 11"
    "alt 10"
    "handoff 11"
)

command -v systemd-run >/dev/null || { echo "nightly.sh: systemd-run not found (Linux only)" >&2; exit 2; }
mkdir -p "$out" || exit 2
summary="$out/summary.txt"
: > "$summary"

say() { printf '%s\n' "$*" | tee -a "$summary"; }

rev=$(git -C "$repo" rev-parse --short HEAD 2>/dev/null || echo unknown)
say "hold-model nightly $stamp revision $rev out $out"

export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$out/target}
# Cargo scope: 32 GB (AGENTS.md). The crate is small; the scope is a guard.
if ! (cd "$repo" && systemd-run --user --scope --quiet -p MemoryMax=32G -p MemorySwapMax=0 -- \
        ionice -c2 -n7 cargo build --release --locked -p hold-model) > "$out/build.log" 2>&1; then
    say "build failed: see $out/build.log"
    exit 2
fi
bin="$CARGO_TARGET_DIR/release/hold-model"

failed=0
for entry in "${profiles[@]}"; do
    read -r profile horizon <<<"$entry"
    raw="$out/$profile.txt"
    started=$(date +%s)
    systemd-run --user --scope --quiet -p MemoryMax=16G -p MemorySwapMax=0 -- \
        ionice -c2 -n7 "$bin" --profile "$profile" --horizon "$horizon" --all-violations \
        > "$raw" 2>&1
    rc=$?
    wall=$(( $(date +%s) - started ))

    states=$(sed -n 's/^profile=.* states=\([0-9]*\) .*/\1/p' "$raw" | head -n 1)
    elapsed_ms=$(sed -n 's/^profile=.* elapsed_ms=\([0-9]*\).*/\1/p' "$raw" | head -n 1)
    rss_kb=$(sed -n 's/^peak_rss_kb=\([0-9]*\).*/\1/p' "$raw" | head -n 1)
    violations=$(sed -n 's/^VIOLATION \([^:]*\):.*/\1/p' "$raw" | paste -sd, -)
    case $rc in
        0) result=clean ;;
        1) result="violation ${violations:-?}" ;;
        3) result="cap reached" ;;
        137) result="killed (memory scope or signal)" ;;
        *) result="exit $rc" ;;
    esac
    [ "$rc" -eq 0 ] || failed=1

    say "profile=$profile horizon=$horizon exit=$rc states=${states:-?} explore_s=$(awk -v ms="${elapsed_ms:-0}" 'BEGIN { printf "%.1f", ms / 1000 }') wall_s=$wall peak_rss_gb=$(awk -v kb="${rss_kb:-0}" 'BEGIN { printf "%.2f", kb / 1048576 }') result=$result"
done

if [ "$failed" -eq 0 ]; then
    say "all ${#profiles[@]} profiles clean"
    exit 0
fi
say "FAILED: at least one profile did not end clean; raw output in $out"
exit 1
