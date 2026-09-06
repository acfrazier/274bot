#!/usr/bin/env python3
"""Sequential panel CPU overhead screen. Not baseline acceptance."""
import argparse
import hashlib
import json
import pathlib
import re
import statistics
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]

def read_rows(path):
    return [json.loads(line) for line in path.read_text().splitlines() if line]

def steals(slot):
    lines = ' '.join((slot.get('runtime', {}).get('paint') or {}).get('lines', []))
    match = re.search(r'\bSteals: (\d+)', lines)
    return int(match[1]) if match else None

def summarize(run, counting, diagnostics):
    meta = json.loads((run / 'metadata.json').read_text())
    rows = read_rows(run / 'samples.jsonl')
    observed = [r for r in rows if r['phase'] == 'observe']
    proof = read_rows(run / 'samples.qualification.jsonl')
    errors = []
    seeded_idle = meta.get('workload') == 'seeded-idle'
    if meta.get('exit_code') != 0:
        errors.append('process failed or incomplete')
    if len(observed) < 2:
        return {'run': str(run), 'qualified': False, 'errors': errors + ['insufficient observation']}
    a, b = observed[0], observed[-1]
    elapsed = b['elapsed_s'] - a['elapsed_s']
    if elapsed < meta['observe_s'] - 5:
        errors.append('observation incomplete')
    for r in observed:
        if r['ready'] != meta['n'] or r['active'] != (0 if seeded_idle else meta['n']):
            errors.append('readiness/activity below requested scale')
        if seeded_idle and any(r.get(k) != 0 for k in ['v8_live_isolates','snapshot_inflight_bytes','snapshot_inflight_capacity']):
            errors.append('idle retains isolate or in-flight snapshots')
        if r.get('allocation_counting') != counting or r.get('diagnostic_sidecar') != diagnostics:
            errors.append('wrong instrumentation mode')
        for field in ['rust_allocations', 'rust_allocated_bytes', 'rust_live_bytes']:
            if (r.get(field) is not None) != counting:
                errors.append('allocator availability mismatch')
    starts = [r for r in proof if r['phase'] == 'observe-start']
    ends = [r for r in proof if r['phase'] == 'observe-end']
    gains = {}
    if len(starts) != 1 or len(ends) != 1:
        errors.append('missing boundary qualification')
    else:
        before = {s['name']: s for s in starts[0]['slots']}
        after = {s['name']: s for s in ends[0]['slots']}
        if before.keys() != after.keys() or len(before) != meta['n']:
            errors.append('qualification slot mismatch')
        for name, end in after.items():
            start = before.get(name, {})
            if seeded_idle:
                for slot in [start,end]:
                    client = slot.get('client') or {}
                    if slot.get('error') or slot.get('state') != 'Idle':
                        errors.append('idle script state invalid: ' + name)
                    if not (client.get('ingame') is True and client.get('scene_state') == 2 and client.get('level') == 0 and abs(client.get('x',0)-2661) <= 10 and abs(client.get('z',0)-3306) <= 10):
                        errors.append('idle scene qualification missing: ' + name)
                continue
            x, y = steals(start), steals(end)
            gains[name] = None if x is None or y is None else y - x
            if end.get('error') or end.get('state') != 'Running' or gains[name] is None or gains[name] <= 0:
                errors.append('missing active progress: ' + name)
    cpu = []
    for key in ['process_cpu_user_s', 'process_cpu_system_s']:
        if any(r.get(key) is None for r in observed) or any(y[key] < x[key] for x, y in zip(observed, observed[1:])):
            errors.append('unavailable/nonmonotonic CPU counter')
        else:
            cpu.append(b[key] - a[key])
    ticks = b['client_tick_count'] - a['client_tick_count']
    return {'run': str(run), 'n': meta['n'], 'counting': counting, 'diagnostics': diagnostics,
            'qualified': not errors, 'errors': sorted(set(errors)), 'observation_s': elapsed,
            'cpu_seconds': sum(cpu) if len(cpu) == 2 else None,
            'cpu_cores': sum(cpu) / elapsed if len(cpu) == 2 else None,
            'client_ticks_per_slot_s': ticks / elapsed / meta['n'],
            'median_resident_bytes': statistics.median(r['resident_bytes'] for r in observed),
            'steal_gains': gains, 'diagnostic_only': True}

def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('output', type=pathlib.Path)
    p.add_argument('--counted-binary', required=True, type=pathlib.Path)
    p.add_argument('--system-binary', required=True, type=pathlib.Path)
    p.add_argument('--observe', type=int, default=180)
    a = p.parse_args()
    a.output.mkdir(parents=True, exist_ok=False)
    results = []
    modes = [(True, True), (False, True), (True, False), (False, False)]
    # Reverse mode order at32 to reduce a simple time-order bias. This is
    # one screening pass, not repeated statistical acceptance.
    for n in [1, 32]:
        for counting, diagnostics in modes if n == 1 else reversed(modes):
            binary = a.counted_binary if counting else a.system_binary
            sha = hashlib.sha256(binary.read_bytes()).hexdigest()
            cmd = [sys.executable, str(ROOT / 'docs/memory/run_diagnostic.py'), 'panel', str(n), 'active',
                   '--single-renderer', '--sustain', '--warmup', '120', '--observe', str(a.observe), '--binary', str(binary)]
            if not diagnostics:
                cmd.append('--no-diagnostics')
            label = f'n{n}_count{int(counting)}_diag{int(diagnostics)}'
            print('START ' + label, flush=True)
            with (a.output / (label + '.log')).open('w') as log:
                result = subprocess.run(cmd, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
            lines = (a.output / (label + '.log')).read_text().splitlines()
            meta = json.loads(lines[0])
            run = pathlib.Path(meta['run_dir'])
            if result.returncode != 0:
                print('BLOCKED ' + str(run), flush=True)
                return 1
            report = summarize(run, counting, diagnostics)
            report['binary_sha256'] = sha
            if hashlib.sha256(binary.read_bytes()).hexdigest() != sha:
                report['qualified'] = False
                report['errors'].append('binary changed during run')
            (run / 'cpu-summary.json').write_text(json.dumps(report, indent=2) + '\n')
            results.append(report)
            (a.output / 'results.json').write_text(json.dumps(results, indent=2) + '\n')
            print(json.dumps({k:v for k,v in report.items() if k != 'steal_gains'}), flush=True)
            if not report['qualified']:
                return 1
    return 0

if __name__ == '__main__':
    sys.exit(main())
