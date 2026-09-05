#!/usr/bin/env python3
"""Summarize a diagnostic run; this does not grant benchmark acceptance."""
import argparse
import collections
import json
import pathlib
import re
import statistics

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('run', type=pathlib.Path)
args = parser.parse_args()

def records(name):
    path = args.run / name
    if not path.exists():
        return []
    # A live writer may have an incomplete final line.
    result = []
    for line in path.read_text().splitlines(keepends=True):
        if line.endswith('\n'):
            result.append(json.loads(line))
    return result

meta = json.loads((args.run / 'metadata.json').read_text())
samples = records('samples.jsonl')
diagnostics = records('samples.diagnostics.jsonl')
observations = [s for s in samples if s.get('phase') == 'observe']
per_slot = collections.defaultdict(list)
if observations:
    start, end = observations[0]['elapsed_s'], observations[-1]['elapsed_s']
    for record in diagnostics:
        if start <= record.get('elapsed_s', -1) <= end:
            for slot in record.get('slots', []):
                per_slot[slot['name']].append(slot)

slots = {}
for name, history in sorted(per_slot.items()):
    xp = [s.get('diagnostics', {}).get('client', {}).get('xp') for s in history]
    xp = [x for x in xp if x is not None]
    final = history[-1]
    paint = (final.get('runtime') or {}).get('paint') or {}
    lines = ' '.join(paint.get('lines', []))
    counts = {}
    for key, label in [('steals', 'Steals'), ('eats', 'Ate'), ('bank_trips', 'Bank trips'), ('food', 'Food')]:
        match = re.search(r'\b' + label + r': (\d+)', lines)
        counts[key] = int(match[1]) if match else None
    slots[name] = {
        'observation_xp_gain': xp[-1] - xp[0] if len(xp) > 1 else None,
        'final_paint': paint,
        **counts,
        'errors': sorted({s['error'] for s in history if s.get('error')}),
    }

summary = {
    'diagnostic_only': True,
    'complete': 'exit_code' in meta,
    'exit_code': meta.get('exit_code'),
    'binary_sha256': meta['binary_sha256'],
    'requested_slots': meta['n'],
    'requested_observe_s': meta['observe_s'],
    'observation_samples': len(observations),
    'observation_sample_span_s': observations[-1]['elapsed_s'] - observations[0]['elapsed_s'] if observations else None,
    'ready_counts': dict(collections.Counter(s['ready'] for s in observations)),
    'active_counts': dict(collections.Counter(s['active'] for s in observations)),
    'median_resident_bytes': statistics.median(s['resident_bytes'] for s in observations) if observations else None,
    'unavailable_metrics': [key for key in ('snapshot_inflight_bytes', 'snapshot_inflight_capacity', 'v8_used_bytes', 'v8_total_bytes', 'gpu_tracked_bytes') if observations and all(s.get(key) is None for s in observations)],
    'failures': [r['failure'] for r in diagnostics if r.get('failure')],
    'final_sample': samples[-1] if samples else None,
    'slots': slots,
}
# Keep measurement coverage separate from functional qualification.
summary['terminal'] = meta.get('terminal', False)
summary['instrumentation'] = {}
for key in ['v8_used_bytes','v8_total_bytes','v8_live_isolates','v8_sampled_isolates','v8_max_sample_age_ms',
            'snapshot_inflight_bytes','snapshot_inflight_capacity','snapshot_sum_isolate_peak_capacity',
            'gpu_buffer_bytes','gpu_texture_bytes','gpu_peak_tracked_bytes']:
    values = [r[key] for r in observations if r.get(key) is not None]
    summary['instrumentation'][key] = {'samples':len(values), 'min':min(values) if values else None,
        'median':statistics.median(values) if values else None,'max':max(values) if values else None}
summary['latency'] = {}
for prefix in ['client_tick','script_tick','ui_draw','ui_frame']:
    if observations and all(prefix+'_count' in r for r in observations):
        first,last=observations[0],observations[-1]
        count=last[prefix+'_count']-first[prefix+'_count']
        total=last[prefix+'_total_ns']-first[prefix+'_total_ns']
        stable=all(b[prefix+'_count']>=a[prefix+'_count'] and b[prefix+'_total_ns']>=a[prefix+'_total_ns'] for a,b in zip(observations,observations[1:]))
        summary['latency'][prefix]={'observation_count':count,'observation_mean_ms':total/count/1e6 if stable and count>0 else None,
            'lifetime_max_ms':last[prefix+'_max_ns']/1e6,'counter_monotonic':stable}

output = args.run / 'qualification-summary.json'
output.write_text(json.dumps(summary, indent=2) + '\n')
print(json.dumps({k: v for k, v in summary.items() if k not in ('slots', 'final_sample')}, indent=2))
print(f'Per-slot details: {output}')
