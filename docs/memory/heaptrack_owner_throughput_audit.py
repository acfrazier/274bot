#!/usr/bin/env python3
"""Read only bounded local native export receipts; never opens production traces."""
import hashlib
import json
from pathlib import Path
import statistics

ROOT = Path(__file__).resolve().parents[2]
EVIDENCE = ROOT/'diagnostics/replay-native-stage-378e634-preparation/native-evidence'


def bounded(path):
    assert not path.is_symlink() and path.stat().st_size <= 2_000_000, path
    return path.read_bytes()


def load(path):
    return json.loads(bounded(path))


def main():
    manifest = load(EVIDENCE/'export-manifest.json')
    checks = []
    for name, entry in manifest['files'].items():
        path = EVIDENCE/name
        assert path.resolve().is_relative_to(EVIDENCE.resolve())
        data = bounded(path)
        digest = hashlib.sha256(data).hexdigest()
        checks.append(dict(path=name, bytes=len(data), sha256=digest,
                           match=len(data) == entry['bytes'] and digest == entry['sha256']))
    assert len(checks) == 41 and all(c['match'] for c in checks)
    report = load(ROOT/'docs/memory/heaptrack-owner-replay-native-result.json')
    matches = {}
    for key, relative in [('production_failure', 'production-replay/failure.json'),
                          ('production_launch', 'production-launch.json'),
                          ('native_tests', 'native-tests.json'),
                          ('native_smoke', 'native-smoke/replay/runner.json')]:
        matches[key] = report[key] == load(EVIDENCE/relative)
    assert all(matches.values())
    smoke = load(EVIDENCE/'native-smoke/replay/result/receipt.json')
    output_checks = []
    for name, entry in smoke['files'].items():
        assert Path(name).name == name
        data = bounded(EVIDENCE/'native-smoke/replay/result'/name)
        output_checks.append(dict(path=name, match=len(data) == entry['bytes'] and
                                  hashlib.sha256(data).hexdigest() == entry['sha256']))
    assert len(output_checks) == 16 and all(c['match'] for c in output_checks)
    completion = load(EVIDENCE/'production-completion.json')
    tests = bounded(EVIDENCE/'native-tests.log').decode()
    test_names = [line.split(' ')[0] for line in tests.splitlines() if line.startswith('test_')]
    assert len(test_names) == 21 and 'Ran 21 tests in 13.723s' in tests and '\nOK\n' in tests
    assert 'skipped' not in tests
    local = load(ROOT/'docs/memory/heaptrack-owner-replay-throughput-local.json')
    for name, digest in local['source_hashes'].items():
        assert hashlib.sha256(bounded(Path(__file__).with_name(name))).hexdigest() == digest
    design = bounded(Path(__file__).with_name('heaptrack-owner-replay-throughput-design.md')).decode()
    assert all('| '+name.removeprefix('test_')+' |' in design for name in test_names)
    timings = {}
    for label, case in local['cases'].items():
        rows = {}
        for mode in ('baseline', 'sampled', 'sampled_numeric'):
            runs = [r for r in case['runs'] if r['mode'] == mode]
            cpu = statistics.median(r['cpu_s'] for r in runs)
            rows[mode] = dict(median_cpu_s=cpu, min_cpu_s=min(r['cpu_s'] for r in runs),
                max_cpu_s=max(r['cpu_s'] for r in runs),
                MB_per_cpu_s=case['bytes']/cpu/1e6 if 'bytes' in case else None,
                phase_cpu_s=[statistics.median(r['phases'][i]['cpu_s'] for r in runs)
                             for i in range(len(runs[0]['phases']))])
        timings[label] = rows
    result = dict(export_checks=checks, report_receipt_exact_matches=matches,
        local_source_hashes_match=True, design_maps_all_21_test_names=True,
        smoke_output_checks=output_checks, test_names=test_names,
        completion=completion, native_stage=load(EVIDENCE/'stage-verification.json'),
        smoke_cutoffs=smoke['cutoffs'], timings=timings,
        required_MB_per_cpu_s=dict(raw=2149010072/180/1e6, interpreted=810563095/180/1e6),
        local_cpu_failures=local['cpu_failure_probes'],
        top_profiles={label: [{k: p[k] for k in ('mode', 'cpu_s', 'profile')}
                              for p in case['profiles']] for label, case in local['cases'].items()},
        production_trace_read=False,
        limits='Export integrity and local fixture CPU only; absence-afterwards is not independently observable from these receipts.')
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
