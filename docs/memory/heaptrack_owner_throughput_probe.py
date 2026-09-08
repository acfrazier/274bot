#!/usr/bin/env python3
"""Local-only throwaway throughput falsification; NOT a replacement replay CLI.

No input arguments: generated fixtures and the fixed saved Python smoke only.
Every trace/oracle is capped at 2,000,000 bytes before opening. Guard sampling
and numeric fast path are experimental monkeypatches confined to this process.
Writes JSON to stdout; run with Python 3.9+ from the campaign checkout.
"""
import cProfile
import hashlib
import json
from pathlib import Path
import platform
import pstats
import re
import statistics
import sys
import tempfile
import time
from typing import Any

import heaptrack_owner_replay as replay
import heaptrack_owner_runner as runner
from test_heaptrack_owner_replay import RAW_HEAD, INT_HEAD, smoke_entries

ORIGINAL_PARSE = replay.parse_record
HEX = re.compile(rb'[0-9a-f]{1,16}')
RAW_ARITY = {'v': 2, 'I': 2, 'R': 1, 'c': 1, 't': 2, '+': 3, '-': 1}
INT_ARITY = dict(RAW_ARITY, **{'+': 1, 'a': 2})


def fast_numeric(line, raw=False, line_limit=replay.MIB):
    # Only specialize fixed-arity numeric records; all other grammar unchanged.
    arities = RAW_ARITY if raw else INT_ARITY
    op = chr(line[0]) if line else ''
    if op not in arities:
        return ORIGINAL_PARSE(line, raw, line_limit)
    replay.require(len(line) <= line_limit, 'line cap')
    replay.require(line.endswith(b'\n'), 'truncated record (missing newline)')
    replay.require(line[1:2] == b' ', 'record separator')
    tokens = line[2:-1].split(b' ')
    replay.require(len(tokens) == arities[op], 'numeric arity')
    fields = []
    for token in tokens:
        replay.require(HEX.fullmatch(token) is not None, 'invalid uint64 hex')
        fields.append(int(token, 16))
    return op, fields


class Cadence:
    def __init__(self, guard, every):
        self.guard, self.every = guard, every
        self.calls = self.checks = 0

    def __call__(self):
        self.calls += 1
        if self.calls % self.every == 0:
            self.checks += 1
            self.guard.check()


def safe_entries(entries):
    for entry in entries.values():
        p = Path(entry['path'])
        assert not p.is_symlink() and 0 < p.stat().st_size == entry['bytes'] <= 2_000_000
    return entries


def profile_rows(profile):
    stats: Any = pstats.Stats(profile)
    rows = []
    for (filename, line, name), (primitive, calls, own, cumulative, callers) in stats.stats.items():
        rows.append(dict(function=Path(filename).name+':'+str(line)+':'+name,
                         calls=calls, own_s=own, cumulative_s=cumulative))
    return sorted(rows, key=lambda r: -r['cumulative_s'])[:22]


def run_case(entries, mode, full=False, profile=False) -> dict[str, Any]:
    guard = runner.PhaseGuard(replay.LIMITS)
    cadence = Cadence(guard, 1 if mode == 'baseline' else 1024)
    replay.PULSE = guard.check if mode == 'baseline' else cadence
    replay.parse_record = fast_numeric if mode == 'sampled_numeric' else ORIGINAL_PARSE
    prof = cProfile.Profile() if profile else None
    start, cpu = time.monotonic(), time.process_time()
    try:
        if prof:
            prof.enable()
        if full:
            result = replay.analyze(entries, requested_time=500, phase=guard.next)
            with tempfile.TemporaryDirectory(dir=Path.cwd()/'diagnostics') as directory:
                replay.write_outputs(result, Path(directory), {'scope': 'fixture'})
            guard.next(4)
            signature = dict(raw=result['raw'], first=result['first'], canonical=result['canonical'])
        else:
            guard.next(1)
            signature = replay.raw_pass(entries['raw'], replay.Budget())
            guard.next(2)
        if prof:
            prof.disable()
        elapsed_cpu = time.process_time()-cpu
        return dict(mode=mode, full=full, profiled=profile, cpu_s=elapsed_cpu,
                    wall_s=time.monotonic()-start,
                    pulse_calls=None if mode == 'baseline' else cadence.calls,
                    record_guard_checks=None if mode == 'baseline' else cadence.checks,
                    phases=guard.measurements,
                    raw_bytes_per_cpu_s=None if full else entries['raw']['bytes']/elapsed_cpu,
                    signature=signature, profile=profile_rows(prof) if prof else [])
    finally:
        replay.PULSE = lambda: None
        replay.parse_record = ORIGINAL_PARSE


def main():
    assert len(sys.argv) == 1, 'No external inputs or production mode allowed'
    source_hashes = {name: hashlib.sha256(Path(__file__).with_name(name).read_bytes()).hexdigest()
                     for name in ('heaptrack_owner_replay.py', 'heaptrack_owner_runner.py',
                                  'heaptrack_owner_throughput_probe.py')}
    results: dict[str, Any] = dict(python=sys.version, platform=platform.platform(), machine=platform.machine(),
                   source_hashes=source_hashes, production_read=False, native_qualification=False,
                   note='Serial warm-cache local fixtures; no production capacity claim', cases={})
    with tempfile.TemporaryDirectory(dir=Path.cwd()/'diagnostics') as directory:
        root = Path(directory)
        # Short-token churn and realistic-width pointers: same number of events.
        # Both keep a single live pointer, deliberately excluding large-map cost.
        for label, pair in [('short', b'+ a 1 10\n- 10\n'),
                            ('wide', b'+ 1000 1 7ffffffff000\n- 7ffffffff000\n')]:
            data = RAW_HEAD + pair*50_000 + b'c a\n'
            assert len(data) <= 2_000_000
            path = root/label
            path.write_bytes(data)
            entries = safe_entries({'raw': dict(path=str(path), bytes=len(data),
                                               sha256=hashlib.sha256(data).hexdigest())})
            runs = [run_case(entries, mode) for _ in range(3)
                    for mode in ('baseline', 'sampled', 'sampled_numeric')]
            assert all(r['signature'] == runs[0]['signature'] for r in runs)
            profiles = [run_case(entries, mode, profile=True) for mode in ('baseline', 'sampled_numeric')]
            results['cases'][label] = dict(bytes=len(data), lines=data.count(b'\n'),
                sha256=entries['raw']['sha256'], runs=runs, profiles=profiles,
                median_cpu_s={mode: statistics.median(r['cpu_s'] for r in runs if r['mode'] == mode)
                              for mode in ('baseline', 'sampled', 'sampled_numeric')})
        entries = safe_entries(smoke_entries())
        runs = [run_case(entries, mode, full=True) for _ in range(3)
                for mode in ('baseline', 'sampled', 'sampled_numeric')]
        assert all(r['signature'] == runs[0]['signature'] for r in runs)
        results['cases']['saved_smoke'] = dict(inputs={k: {f: v for f, v in e.items() if f != 'path'}
                                                       for k, e in entries.items()}, runs=runs,
            profiles=[run_case(entries, 'baseline', full=True, profile=True)])
        # Actual parser/PULSE CPU failure, not an unguarded loop. Re-read ONLY
        # the tiny synthetic fixture under one 1-second phase; no large input.
        for mode in ('baseline', 'sampled_numeric'):
            guard = runner.PhaseGuard(replay.Budget({'cpu': 1}).limits)
            cadence = Cadence(guard, 1 if mode == 'baseline' else 1024)
            replay.PULSE = cadence
            replay.parse_record = fast_numeric if mode == 'sampled_numeric' else ORIGINAL_PARSE
            guard.next(1)
            passes = 0
            try:
                while True:
                    replay.raw_pass(safe_entries({'raw': dict(path=str(root/'short'),
                        bytes=results['cases']['short']['bytes'],
                        sha256=results['cases']['short']['sha256'])})['raw'], replay.Budget())
                    passes += 1
                    guard.check()
            except replay.Invalid as exc:
                assert str(exc) == 'CPU guard'
                results.setdefault('cpu_failure_probes', []).append(dict(mode=mode, reason=str(exc),
                    cpu_s=time.process_time()-guard.start_cpu, complete_small_passes=passes,
                    pulse_calls=cadence.calls, checks=cadence.checks))
            finally:
                replay.PULSE = lambda: None
                replay.parse_record = ORIGINAL_PARSE
    print(json.dumps(results, indent=2))


if __name__ == '__main__':
    main()
