#!/usr/bin/env bash
# Nightly full profile runs of tools/luau-model (ADR 0017), on Linux.
#
# Builds the release binary, then runs every profile of the model one at a
# time (the list comes from `luau-model --list`), each in its own
# `systemd-run --user --scope -p MemoryMax=16G -p MemorySwapMax=0` scope, so
# that a run that grows past 16 GB is killed alone. It writes one summary line
# per profile and exits non-zero when any run does not end clean and complete
# (exit 0 of the model).
#
# usage: nightly.sh [OUTDIR]
#   OUTDIR  where the raw output of each run and `summary.txt` go. Default:
#           $LUAU_MODEL_NIGHTLY_OUT, else
#           $HOME/.local/state/274bot-luau-model/<UTC date>.
#   CARGO_TARGET_DIR  where the release binary is built. Default: OUTDIR/target.
#
# Exit code: 0 every profile ended clean and complete; 1 a profile found a
# violation, hit a cap or was killed; 2 the build, the scope or a bad
# argument stopped the run.
#
# The horizons are the profiles' own (README.md, "Run it"); every profile is
# exhausted before its horizon.

set -u

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo=$(cd "$here/../.." && pwd)
stamp=$(date -u +%Y%m%dT%H%M%SZ)
out=${1:-${LUAU_MODEL_NIGHTLY_OUT:-$HOME/.local/state/274bot-luau-model/$stamp}}

command -v systemd-run >/dev/null || { echo "nightly.sh: systemd-run not found (Linux only)" >&2; exit 2; }
mkdir -p "$out" || exit 2
summary="$out/summary.txt"
: > "$summary"

say() { printf '%s\n' "$*" | tee -a "$summary"; }

rev=$(git -C "$repo" rev-parse --short HEAD 2>/dev/null || echo unknown)
say "luau-model nightly $stamp revision $rev out $out"

export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$out/target}
if ! (cd "$repo" && systemd-run --user --scope --quiet -p MemoryMax=32G -p MemorySwapMax=0 -- \
        ionice -c2 -n7 cargo build --release --locked -p luau-model) > "$out/build.log" 2>&1; then
    say "build failed: see $out/build.log"
    exit 2
fi
bin="$CARGO_TARGET_DIR/release/luau-model"

mapfile -t profiles < <("$bin" --list | awk '{ print $1 }')
[ "${#profiles[@]}" -gt 0 ] || { say "no profile listed by $bin --list"; exit 2; }

failed=0
for profile in "${profiles[@]}"; do
    raw="$out/lifecycle-$profile.txt"
    started=$(date +%s)
    systemd-run --user --scope --quiet -p MemoryMax=16G -p MemorySwapMax=0 -- \
        ionice -c2 -n7 "$bin" --profile "$profile" --all-violations \
        > "$raw" 2>&1
    rc=$?
    wall=$(( $(date +%s) - started ))

    states=$(sed -n 's/^model=.* states=\([0-9]*\) .*/\1/p' "$raw" | head -n 1)
    elapsed_ms=$(sed -n 's/^model=.* elapsed_ms=\([0-9]*\).*/\1/p' "$raw" | head -n 1)
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

    say "model=lifecycle profile=$profile exit=$rc states=${states:-?} explore_s=$(awk -v ms="${elapsed_ms:-0}" 'BEGIN { printf "%.1f", ms / 1000 }') wall_s=$wall peak_rss_gb=$(awk -v kb="${rss_kb:-0}" 'BEGIN { printf "%.2f", kb / 1048576 }') result=$result"
done

if [ "$failed" -eq 0 ]; then
    say "all ${#profiles[@]} profiles clean"
    exit 0
fi
say "FAILED: at least one profile did not end clean; raw output in $out"
exit 1
