#!/usr/bin/env python3
"""Turn BOT_DEBUG host logs of live cells into host-only hold traces (see HoldHostTrace.tla).

usage: livegen.py <out-dir> <log>...

The host logs three things about the hold (crates/host/src/lib.rs, category InteractTrace):
  auto-run hold key=<new> was=<old> deadline_tick=<d> tick=<t>   when the key changes
  auto-run hold ended after its deadline key=<k> deadline_tick=<d> tick=<t>: ...   (terminal release)
  auto-run press energy=.. main=-1 chat=-1 side=-1 run_sends=<n>   when a press goes out
The log has no line for a frame where the key does not change. So the extractor fills the frames
between two logged changes with the hold the log says is live: a compat key (top bit set) reports
deadline = tick on every frame (the lease ends itself on its wall bound); a native key keeps the
deadline of its last logged line. This is an assumption of the extractor, not an observation.
Every logged press becomes a press event with `defer` = whether the extractor believes a hold is live.
"""
import json
import os
import re
import sys

HOLD = re.compile(r"auto-run hold key=(\S+) was=(\S+) deadline_tick=(\S+) tick=(\d+)")
TERM = re.compile(r"auto-run hold ended after its deadline key=(\d+) deadline_tick=(\d+) tick=(\d+)")
PRESS = re.compile(r"auto-run press energy=(\d+) main=(-?\d+) chat=(-?\d+) side=(-?\d+) run_sends=(\d+)")
TOP = 1 << 63


def opt(text):
    m = re.fullmatch(r"Some\((\d+)\)", text)
    return int(m.group(1)) if m else None


def extract(path):
    evs = []
    cur = None  # (key, dl)
    last_tick = None
    for line in open(path, errors="replace"):
        m = HOLD.search(line)
        if m:
            key, dl, tick = opt(m.group(1)), opt(m.group(3)), int(m.group(4))
            # fill the frames between the last logged change and this one
            if cur is not None and last_tick is not None:
                for t in range(last_tick + 1, tick):
                    dlt = t if cur[0] >= TOP else cur[1]
                    evs.append({"ev": "observe", "wait": {"key": cur[0], "dl": dlt}, "tick": t,
                                "expired": False, "hold": {"key": cur[0], "released": False}})
            if key is None:
                evs.append({"ev": "observe", "wait": None, "tick": tick, "expired": False, "hold": None})
                cur = None
            else:
                evs.append({"ev": "observe", "wait": {"key": key, "dl": dl}, "tick": tick,
                            "expired": False, "hold": {"key": key, "released": False}})
                cur = (key, dl)
            last_tick = tick
            continue
        m = TERM.search(line)
        if m:
            key, dl, tick = int(m.group(1)), int(m.group(2)), int(m.group(3))
            evs.append({"ev": "observe", "wait": {"key": key, "dl": dl}, "tick": tick,
                        "expired": True, "hold": {"key": key, "released": True}})
            cur = None
            last_tick = tick
            continue
        m = PRESS.search(line)
        if m:
            free = (m.group(2), m.group(3), m.group(4)) == ("-1", "-1", "-1")
            evs.append({"ev": "press", "sent": True, "defer": cur is not None, "modal_free": free,
                        "run_on": True})
    return evs


def main():
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    for path in sys.argv[2:]:
        parts = os.path.abspath(path).split(os.sep)
        name = "live-" + re.sub(r"[^A-Za-z0-9]", "_", "_".join(p for p in parts if p.startswith(("L-", "Q-"))))
        evs = extract(path)
        with open(os.path.join(out, "host-%s.jsonl" % name), "w") as f:
            for e in evs:
                f.write(json.dumps(e) + "\n")
        print(name, len(evs), "events")


main()
