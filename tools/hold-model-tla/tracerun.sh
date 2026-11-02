#!/usr/bin/env bash
# TLC trace validation: tracerun.sh <dir> <name>...
#
# <dir> holds HoldModel.tla, HoldTrace.tla, HoldHostTrace.tla and, per <name>, <name>.cfg and
# MC_<name>.tla (a rig trace) or MH_<name>.tla (a host trace), as written by tracegen.py.
# JAR is the path of tla2tools.jar (v1.7.4, see README.md). Run on the Linux lab host only.
#
# Deadlock checking is off (-deadlock): one branch of the search may dead-end while another reaches the
# end of the trace. Verdicts:
#   ACCEPTED       TLC reports the invariant NotDone violated: a behaviour of the model reaches the end of
#                  the trace, every step being a step of the model (see HoldTrace.tla for what is input).
#   S4h-VIOLATED   the host-side S4 invariant failed on the trace (a hand-altered trace).
#   REJECTED       the whole search finished with no states left and never reached the end.
#   INCOMPLETE     the search stopped at TRACE_STOP seconds (default 600) with states left: raise it.
# The last field is the depth of the search: the deepest model step any behaviour reached.
set -u
: "${JAR:?set JAR to the path of tla2tools.jar}"
dir=$1
shift
cd "$dir" || exit 2
for n in "$@"; do
  if [ -f "MC_$n.tla" ]; then m="MC_$n"; else m="MH_$n"; fi
  rm -rf "states-$n"
  systemd-run --user --scope --quiet -p MemoryMax=16G -p MemorySwapMax=0 -- ionice -c2 -n7 \
    java -Xmx4g -XX:+UseParallelGC -Dtlc2.TLC.stopAfter="${TRACE_STOP:-600}" -cp "$JAR" \
    tlc2.TLC -workers 4 -deadlock -config "$n.cfg" -metadir "states-$n" "$m.tla" > "$n.out" 2>&1
  rc=$?
  rm -rf "states-$n"
  if grep -q "Invariant S4h is violated" "$n.out"; then v=S4h-VIOLATED
  elif grep -q "Invariant NotDone is violated" "$n.out"; then v=ACCEPTED
  elif grep -q "Model checking completed. No error" "$n.out" && grep -q " 0 states left on queue" "$n.out"; then v=REJECTED
  elif grep -q "Model checking completed. No error" "$n.out"; then v=INCOMPLETE
  else v="OTHER(exit $rc)"; fi
  echo "$n $v states=$(grep -E 'states generated' "$n.out" | tail -1 | sed 's/ states generated.*//')" \
    "trace_states=$(grep -c '^State ' "$n.out") depth=$(grep -oE 'search is [0-9]+' "$n.out" | grep -oE '[0-9]+')"
done
