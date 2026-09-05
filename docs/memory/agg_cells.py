import json, statistics, os, glob, collections
root = "docs/memory/samples"
cells = collections.defaultdict(list)
for path in sorted(glob.glob(root + "/*.jsonl")):
    base = os.path.basename(path)
    # skip incomplete fail seed if tiny
    rows=[]
    with open(path) as f:
        for line in f:
            line=line.strip()
            if line: rows.append(json.loads(line))
    if not rows: continue
    fe=rows[0]["frontend"]; n=rows[0]["n"]; wl=rows[0]["workload"]
    # detect cpu from filename
    if "panel_cpu" in base:
        fe="panel-cpu"
    by=collections.defaultdict(list)
    for r in rows: by[r["phase"]].append(r)
    obs=by.get("observe",[])
    td=by.get("teardown",[])
    seed=by.get("seed",[]); warm=by.get("warmup",[])
    def med(vals):
        vals=[v for v in vals if v is not None]
        return statistics.median(vals) if vals else None
    def mx(vals):
        vals=[v for v in vals if v is not None]
        return max(vals) if vals else None
    mids=obs[len(obs)//3:2*len(obs)//3] or obs
    cells[(fe,n,wl)].append({
        "path": path,
        "ready_max": mx(r["ready"] for r in rows),
        "rss_mid": med(r.get("resident_bytes") for r in mids),
        "live_mid": med(r.get("rust_live_bytes") for r in mids),
        "peak_any": mx(r.get("peak_resident_bytes") for r in rows),
        "start_peak": mx(r.get("peak_resident_bytes") for r in seed+warm),
        "obs_peak": mx(r.get("peak_resident_bytes") for r in obs),
        "td_last_rss": td[-1].get("resident_bytes") if td else None,
        "td_last_live": td[-1].get("rust_live_bytes") if td else None,
        "has_observe": bool(obs),
        "n_samples": len(rows),
    })

def fmt_b(b):
    if b is None: return "n/a"
    b=float(b)
    if b >= 1e9: return f"{b/1e9:.2f} GiB"
    if b >= 1e6: return f"{b/1e6:.1f} MiB"
    return f"{int(b)} B"

print("## Per-run stats then cell medians\n")
for key in sorted(cells.keys(), key=lambda k: (k[0], k[1], k[2])):
    fe,n,wl=key
    runs=cells[key]
    print(f"### {fe} N={n} {wl} ({len(runs)} files)")
    for r in runs:
        print(f"  - {os.path.basename(r['path'])}: ready_max={r['ready_max']} rss_mid={fmt_b(r['rss_mid'])} live_mid={fmt_b(r['live_mid'])} peak={fmt_b(r['peak_any'])} start_peak={fmt_b(r['start_peak'])} td_rss={fmt_b(r['td_last_rss'])} observe={r['has_observe']}")
    good=[r for r in runs if r['has_observe'] and r['rss_mid'] is not None]
    if good:
        print(f"  MEDIAN rss_mid={fmt_b(med([r['rss_mid'] for r in good]))} live_mid={fmt_b(med([r['live_mid'] for r in good]))} peak={fmt_b(med([r['peak_any'] for r in good]))} start_peak={fmt_b(med([r['start_peak'] for r in good]))} td_rss={fmt_b(med([r['td_last_rss'] for r in good if r['td_last_rss']]))}")
    print()

# bytes per additional ready bot: (rss_n - rss_1) / (n-1) using idle medians
print("## Bytes per additional ready bot (idle mid-observe RSS median)\n")
for fe in ["panel","tui"]:
    c1=cells.get((fe,1,"idle"),[])
    c32=cells.get((fe,32,"idle"),[])
    g1=[r for r in c1 if r['has_observe']]
    g32=[r for r in c32 if r['has_observe']]
    if g1 and g32:
        r1=med([r['rss_mid'] for r in g1]); r32=med([r['rss_mid'] for r in g32])
        l1=med([r['live_mid'] for r in g1]); l32=med([r['live_mid'] for r in g32])
        per_rss=(r32-r1)/31
        per_live=(l32-l1)/31
        print(f"{fe} idle: RSS/add={fmt_b(per_rss)}  rust_live/add={fmt_b(per_live)}  (from N1={fmt_b(r1)} N32={fmt_b(r32)}; do not equate domains)")
print()
# active same
print("## Bytes per additional ready bot (active mid-observe RSS median)\n")
for fe in ["panel","tui"]:
    c1=cells.get((fe,1,"active"),[])
    c32=cells.get((fe,32,"active"),[])
    g1=[r for r in c1 if r['has_observe']]
    g32=[r for r in c32 if r['has_observe']]
    if g1 and g32:
        r1=med([r['rss_mid'] for r in g1]); r32=med([r['rss_mid'] for r in g32])
        l1=med([r['live_mid'] for r in g1]); l32=med([r['live_mid'] for r in g32])
        print(f"{fe} active: RSS/add={fmt_b((r32-r1)/31)}  rust_live/add={fmt_b((l32-l1)/31)}  (N1={fmt_b(r1)} N32={fmt_b(r32)})")
