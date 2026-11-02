#!/usr/bin/env python3
"""Turn the hold traces of the product's tests into TLC trace-validation inputs.

usage: tracegen.py rig <raw-dir> <out-dir> [--unit-ms 25]
       tracegen.py host <raw-dir> <out-dir>

rig:  reads `holdrig-*.jsonl` (one frame per line, written by `HoldRig::run` in
      crates/host-play/src/script_progress_tests.rs). Each file may hold several runs
      (the frame index `i` restarts at 0). Writes, per run, `MC_<name>.tla` (EXTENDS HoldTrace)
      and `<name>.cfg`, and prints the names.
host: reads `host-*.jsonl` and `compat-*.jsonl` (events of `ScriptRunPolicy` and the host-play
      compat hand-off test). Writes `MH_<name>.tla` (EXTENDS HoldHostTrace) and `<name>.cfg`.
"""
import glob
import json
import os
import re
import sys

CONSTS = """    MaxTick = 100000
    MaxWall = 100000
    MaxAct = 100000
    MaxRelogs = {relogs}
    MaxStops = 1
    MaxDeaths = 0
    MaxToggles = 4
    MaxTxn = 100000
    HB = 1
    QW = 1
    AckB = 1
    GapT = 1
    WinT = 1
    Margin = 1
    Wrap = FALSE
    Room = FALSE
    Mut = "head"
"""


def b(x):
    return "TRUE" if x else "FALSE"


def safe(name):
    return re.sub(r"[^A-Za-z0-9]", "_", name)[:60]


def rig(raw, out, unit):
    names = []
    for path in sorted(glob.glob(os.path.join(raw, "holdrig-*.jsonl"))):
        base = os.path.basename(path)[len("holdrig-tests.script_progress."):-len(".jsonl")]
        runs, cur = [], []
        for line in open(path):
            rec = json.loads(line)
            if rec["i"] == 0 and cur:
                runs.append(cur)
                cur = []
            cur.append(rec)
        if cur:
            runs.append(cur)
        for n, run in enumerate(runs):
            name = "R_%s_%d" % (safe(base), n)
            frames = []
            for f in run:
                ops = []
                sl = 0
                if f["sleep_ms"] > 0:
                    ops.append('"sleep"')
                    sl = -(-f["sleep_ms"] // unit)
                for k in ("stop", "pause", "resume"):
                    if f[k]:
                        ops.append('"%s"' % k)
                owner = []
                for e in f["owner"]:
                    e = json.loads(e) if isinstance(e, str) else e
                    if e["ev"] == "begin":
                        v = -(-e["ms"] // unit) if e["k"] == "A" else e["ticks"]
                        owner.append('[ev |-> "begin", k |-> "%s", v |-> %d]' % (e["k"], v))
                    else:
                        owner.append('[ev |-> "%s", k |-> "-", v |-> 0]' % e["ev"])
                frames.append(
                    "[edge |-> %s, modal |-> %s, tick |-> %d, sl |-> %d, ops |-> <<%s>>, "
                    "eff |-> %s, press |-> %s, owner |-> <<%s>>]"
                    % (b(f["edge"]), b(f["modal"]), f["tick"], sl, ", ".join(ops), b(f["eff"]),
                       b(f["press"]), ", ".join(owner)))
            write(out, "MC_" + name, "HoldTrace", "TraceData == <<\n  " + ",\n  ".join(frames) + "\n>>",
                  "TSpec", "NotDone", 'Trace <- TraceData\n    E = 1\n', 0, name)
            names.append(name)
    return names


def write(out, module, extends, body, spec, inv, extra, relogs, name):
    os.makedirs(out, exist_ok=True)
    open(os.path.join(out, module + ".tla"), "w").write(
        "---- MODULE %s ----\nEXTENDS %s\n%s\n====\n" % (module, extends, body))
    cfg = "SPECIFICATION %s\nCONSTANTS\n%s    %s\nINVARIANTS\n    %s\n" % (spec, CONSTS.format(relogs=relogs), extra, inv)
    open(os.path.join(out, name + ".cfg"), "w").write(cfg)


def host(raw, out):
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    names = []
    for path in sorted(glob.glob(os.path.join(raw, "host-*.jsonl")) + glob.glob(os.path.join(raw, "compat-*.jsonl"))):
        base = os.path.basename(path)[:-len(".jsonl")]
        evs = [json.loads(line) for line in open(path)]
        name = "H_" + safe(base)
        steps = host_steps(evs)
        if steps is None:
            print("skip %s (tick jump or unsupported event order)" % base, file=sys.stderr)
            continue
        write(out, "MH_" + name, "HoldHostTrace", "TraceData == <<\n  " + ",\n  ".join(steps) + "\n>>",
              "HSpec", "S4h\n    NotDone", "Trace <- TraceData\n", 5, name)
        names.append(name)
    return names


def host_steps(evs):
    """One model step per event. See HoldHostTrace.tla for the meaning of each field."""
    steps = []
    keymap = {}  # TLC integers are 32-bit: renumber the u64 keys by first appearance
    last_key = None
    txn = 0
    tick = None
    for e in evs:
        if e["ev"] == "sync":
            steps.append('[ev |-> "sync", rt |-> %d, key |-> 0, dl |-> 0, txn |-> 0, tick |-> 0, '
                         'expired |-> FALSE, rel |-> FALSE, held |-> FALSE, sent |-> FALSE, '
                         'defer |-> FALSE, mfree |-> TRUE, ron |-> FALSE, has |-> FALSE]'
                         % (0 if e["rt"] is None else e["rt"]))
        elif e["ev"] == "boundary":
            steps.append('[ev |-> "boundary", rt |-> 0, key |-> 0, dl |-> 0, txn |-> 0, tick |-> 0, '
                         'expired |-> FALSE, rel |-> FALSE, held |-> FALSE, sent |-> FALSE, '
                         'defer |-> FALSE, mfree |-> TRUE, ron |-> FALSE, has |-> FALSE]')
        elif e["ev"] in ("observe", "sample"):
            w = e["wait"]
            if w is None:
                key, dl, has = 0, 0, False
                last_key = None
            else:
                key, dl, has = keymap.setdefault(w["key"], len(keymap) + 1), w["dl"], True
                if key != last_key:
                    txn += 1
                last_key = key
                if "txn" in w:  # a hand-edited trace may name the transaction
                    txn = w["txn"]
            if tick is not None and e["tick"] not in (tick, tick + 1) and e["tick"] < tick:
                return None
            tick = e["tick"]
            hold = e.get("hold")
            if hold is not None or "held" not in e:
                rel = bool(hold and hold["released"])
                held = bool(hold and not hold["released"])
            else:  # a compat sample records only `held`: a live read that is not held is released
                held = e["held"]
                rel = w is not None and not held
            steps.append('[ev |-> "observe", rt |-> 0, key |-> %d, dl |-> %d, txn |-> %d, tick |-> %d, '
                         'expired |-> %s, rel |-> %s, held |-> %s, sent |-> FALSE, defer |-> FALSE, '
                         'mfree |-> TRUE, ron |-> FALSE, has |-> %s]'
                         % (key, dl, txn, e["tick"], b(e["expired"]), b(rel), b(held), b(has)))
        elif e["ev"] == "press":
            steps.append('[ev |-> "press", rt |-> 0, key |-> 0, dl |-> 0, txn |-> 0, tick |-> 0, '
                         'expired |-> FALSE, rel |-> FALSE, held |-> FALSE, sent |-> %s, defer |-> %s, '
                         'mfree |-> %s, ron |-> %s, has |-> FALSE]'
                         % (b(e["sent"]), b(e["defer"]), b(e["modal_free"]), b(e["run_on"])))
    return steps


if __name__ == "__main__":
    mode, raw, out = sys.argv[1:4]
    unit = 25
    if "--unit-ms" in sys.argv:
        unit = int(sys.argv[sys.argv.index("--unit-ms") + 1])
    names = rig(raw, out, unit) if mode == "rig" else host(raw, out)
    print("\n".join(names))
