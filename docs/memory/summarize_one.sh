#!/usr/bin/env bash
# Summarize one jsonl: mid-observe median RSS / rust_live, peak RSS, teardown last RSS, ready max
set -euo pipefail
JL="$1"
python3 - "$JL" <<'PY'
import json, sys, statistics
path = sys.argv[1]
rows = []
with open(path) as f:
    for line in f:
        line=line.strip()
        if not line: continue
        rows.append(json.loads(line))
if not rows:
    print("empty"); sys.exit(0)
fe=rows[0]["frontend"]; n=rows[0]["n"]; wl=rows[0]["workload"]
by = {}
for r in rows:
    by.setdefault(r["phase"], []).append(r)
obs = by.get("observe", [])
td = by.get("teardown", [])
warm = by.get("warmup", [])
seed = by.get("seed", [])
def med(vals):
    vals=[v for v in vals if v is not None]
    return int(statistics.median(vals)) if vals else None
def mx(vals):
    vals=[v for v in vals if v is not None]
    return max(vals) if vals else None
ready_max = mx(r["ready"] for r in rows)
# mid-observe window: middle third of observe samples
def mid(lst):
    if not lst: return []
    a=len(lst)//3; b=2*len(lst)//3
    return lst[a:b] or lst
mids = mid(obs)
rss_mid = med(r.get("resident_bytes") for r in mids)
live_mid = med(r.get("rust_live_bytes") for r in mids)
peak = mx(r.get("peak_resident_bytes") for r in rows)
# startup peak: max peak during seed+warmup
start_peak = mx(r.get("peak_resident_bytes") for r in seed+warm)
# transition: max peak during observe for lifecycle (stop/start)
obs_peak = mx(r.get("peak_resident_bytes") for r in obs)
td_last = td[-1].get("resident_bytes") if td else None
td_live = td[-1].get("rust_live_bytes") if td else None
print(f"file={path}")
print(f"fe={fe} n={n} wl={wl} samples={len(rows)} ready_max={ready_max}")
print(f"observe_n={len(obs)} teardown_n={len(td)}")
print(f"rss_mid_observe={rss_mid} rust_live_mid={live_mid}")
print(f"peak_resident_any={peak} start_peak={start_peak} observe_peak={obs_peak}")
print(f"teardown_last_rss={td_last} teardown_last_live={td_live}")
# first observe sample for established baseline
if obs:
    o0=obs[0]
    print(f"observe0_rss={o0.get('resident_bytes')} observe0_live={o0.get('rust_live_bytes')} ready={o0.get('ready')} active={o0.get('active')}")
if td:
    print(f"teardown0_rss={td[0].get('resident_bytes')}")
PY
