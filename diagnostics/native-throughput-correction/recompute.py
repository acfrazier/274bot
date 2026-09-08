#!/usr/bin/env python3
"""Recompute all timing arithmetic; baseline-vs-baseline must fail retention."""
import json
from pathlib import Path
import statistics
import sys
root = Path(__file__).resolve().parent
roles = sys.argv[1:] or ['baseline', 'candidate']
a, b = [json.loads((root/r/'results.json').read_text()) for r in roles]
rows = []
for ca, cb in zip(a['cases'], b['cases']):
    assert (ca['bytes'],ca['hashes'],ca['records']) == (cb['bytes'],cb['hashes'],cb['records'])
    medians = [[statistics.median(r['phases'][i]['cpu_s'] for r in c['repeats']) for i in range(3)] for c in (ca,cb)]
    reductions = [1-y/x for x,y in zip(*medians)]
    rows.append(dict(case=[ca['size'],ca['wide']], medians=medians, cpu_reduction=reductions,
                     total_cpu_medians=[statistics.median(sum(p['cpu_s'] for p in r['phases']) for r in c['repeats']) for c in (ca,cb)],
                     meaningful=all(x >= .10 for x in reductions[1:]) if isinstance(ca['wide'],bool) else None))
print(json.dumps(rows, indent=2))
assert all(r['meaningful'] is not False for r in rows), 'predeclared 10% interpreted CPU reduction not met'
