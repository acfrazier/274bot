#!/usr/bin/env python3
"""Generate the phase-2 TLC configs (earlier designs of the hold) from head-core.cfg.

usage: phase2.py <out-dir> [--repo]
  Writes, for each mutation, one cfg per frozen invariant (core class, no relog/Stop/death) and
  one cfg per S2 invariant in its event class, and prints the cfg base names, one per line.
  With --repo it writes only the per-mutation `mut-<name>.cfg` (all invariants, core class)
  into <out-dir>.
"""
import os
import re
import sys

here = os.path.dirname(os.path.abspath(__file__))
base = open(os.path.join(here, "head-core.cfg")).read().split("INVARIANTS")[0]

MUTS = {
    # name: (Mut value, constant overrides, note)
    "b2bc": ("BackToBackC", {}, "head plus a compat read that ends and the next begins in one poll"),
    "b2bcf": ("BackToBackC+Fix", {}, "b2bc on top of FixRearm"),
    "m1": ("M1", {}, "round 2: no leases; hold ends at a modal or 2 ticks after the click"),
    "m2": ("M2", {}, "round 3: as M1 with 3 ticks"),
    "m3": ("M3", {"GapT": 3}, "round 4: fixed release 2 ticks (analogue of 25) from the first sample"),
    "m4": ("M4", {"GapT": 3}, "round 4: M3 plus one constant compat key"),
    "m5": ("M5", {}, "round 5: active-clock bound converted once to a wall deadline"),
    "m5f": ("M5+Fix", {}, "M5 on top of FixRearm"),
    "m6": ("M6", {}, "round 5: compat hold not renewed at the click"),
    "m6f": ("M6+Fix", {}, "M6 on top of FixRearm"),
}
CORE_INV = ["S1", "S3", "S4a", "S4b", "LB", "S5Click"]
S2 = [("stop", "S2Stop", {"MaxStops": 1}), ("relog", "S2Bnd", {"MaxRelogs": 1}),
      ("death", "S2Dead", {"MaxDeaths": 1})]


def cfg(name, mut, over, invs, note):
    c = base
    c = re.sub(r'Mut = "[^"]*"', 'Mut = "%s"' % mut, c)
    for k, v in over.items():
        c = re.sub(r"\b%s = \d+" % k, "%s = %s" % (k, v), c)
    c = re.sub(r"^\\\* Phase 1.*$", "\\\\* " + note, c, flags=re.M)
    c += "INVARIANTS\n" + "".join("    %s\n" % i for i in ["TypeOK"] + invs)
    return c


def main():
    out = sys.argv[1]
    repo = "--repo" in sys.argv
    os.makedirs(out, exist_ok=True)
    for key, (mut, over, note) in MUTS.items():
        allinv = CORE_INV
        if repo:
            n = "mut-%s" % key
            open(os.path.join(out, n + ".cfg"), "w").write(
                cfg(n, mut, over, allinv, "Phase 2, %s (Mut = %s): %s. All core invariants." % (key, mut, note)))
            continue
        n = "p2-%s-all" % key
        open(os.path.join(out, n + ".cfg"), "w").write(cfg(n, mut, over, allinv, "Phase 2 %s: all core invariants." % key))
        print(n)
        for inv in CORE_INV:
            n = "p2-%s-%s" % (key, inv.lower())
            open(os.path.join(out, n + ".cfg"), "w").write(cfg(n, mut, over, [inv], "Phase 2 %s: %s only." % (key, inv)))
            print(n)
        for cls, inv, ov in S2:
            n = "p2-%s-%s" % (key, inv.lower())
            o = dict(over)
            o.update(ov)
            open(os.path.join(out, n + ".cfg"), "w").write(cfg(n, mut, o, [inv], "Phase 2 %s: %s, class %s." % (key, inv, cls)))
            print(n)


main()
