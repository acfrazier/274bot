#!/usr/bin/env python3
"""Finite fixture-only qualification. No input path, manifest, or production hook.

Default mode requires Linux. --portable-probe is explicitly not qualification.
Root must release the Linux execution separately after exact-code review.
"""
import argparse
import hashlib
import json
from pathlib import Path
import statistics
import subprocess
import sys
import tempfile
import time
from typing import Any

ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT/'tests'))
sys.path.insert(0, str(ROOT.parent))
from test_heaptrack_owner_replay import RAW_HEAD, INT_HEAD, fixture
import heaptrack_owner_replay as reference
import heaptrack_owner_runner as supervisor

# Necessary inventory-derived averages from the approved design, decimal B/s.
REQUIRED = (11938945, 4503128, 4503128)
CASES = tuple((size, wide) for size in (250000, 500000, 1850048) for wide in (False, True)) + ((1000, 'diversity'), (65530, 'symbol'), (1048574, 'near-line-cap'))


def generated(size, wide):
    if wide == 'diversity':
        raw = [b'v 10500 3\nx 1 x\nX fixture\nI 1000 100\n']
        interp = [b'v 10500 3\nX fixture\nI 1000 100\ns 1 f\n']
        for i in range(1, size+1):
            raw.append(f't {i:x} 0\n'.encode())
            interp.append(f'i {i:x} 0 1\nt {i:x} 0\n'.encode())
        for i in range(1, size+1):
            raw.append(f'+ {i:x} {i:x} {i:x}\n'.encode())
            interp.append(f'a {i:x} {i:x}\n+ {i-1:x}\n'.encode())
        return b''.join(raw)+b'c a\n', b''.join(interp)+b'c a\n', f'f; {size*(size+1)//2}\n'.encode()
    if wide == 'symbol':
        symbol = b'f'+b'x'*size
        interp = b'v 10500 3\nX fixture\nI 1000 100\ns '+format(len(symbol),'x').encode()+b' '+symbol+b'\ni 10 0 1 0 0 1 0 0\nt 1 0\na a 1\n+ 0\nc a\n'
        return RAW_HEAD+b'+ a 1 10\nc a\n', interp, symbol+b';'+symbol+b' (); 10\n'
    if wide == 'near-line-cap':
        comment = b'#'+b'x'*size+b'\n'
        return RAW_HEAD+comment+b'+ a 1 10\nc a\n', INT_HEAD+comment+b'a a 1\n+ 0\nc a\n', b'f; 10\n'
    pointer = b'abcdef0123456789' if wide else b'10'
    event = b'+ a 1 '+pointer+b'\n- '+pointer+b'\n'
    count = (size-len(RAW_HEAD)-4)//len(event)
    raw = RAW_HEAD+event*count+b'c a\n'
    interpreted = INT_HEAD+b'a a 1\n'+b'+ 0\n- 0\n'*count+b'c a\n'
    peak = b'f; 10\n'
    assert max(map(len, (raw, interpreted, peak))) <= 2_000_000
    return raw, interpreted, peak


def owned(request, executable, remaining):
    """One retained native child; external 0.1s monitor, always kill/reap."""
    limits = reference.Budget({'cpu':30, 'wall':60}).limits
    with tempfile.TemporaryDirectory(prefix='native-qualification-cell-') as directory:
        root = Path(directory)
        input_path, stdout_path = root/'stdin', root/'stdout'
        input_path.write_bytes(json.dumps(request).encode())
        started = time.monotonic()
        samples, peak_rss, peak_as, previous, max_interval = 0, 0, 0, started, 0.0
        with input_path.open('rb') as source, stdout_path.open('wb') as sink:
            child = subprocess.Popen([str(executable)], stdin=source, stdout=sink, stderr=subprocess.DEVNULL)
            try:
                while child.poll() is None:
                    now = time.monotonic()
                    if now-started > min(60, remaining):
                        raise RuntimeError('qualification cell wall guard')
                    max_interval = max(max_interval, now-previous); previous = now
                    if sys.platform == 'linux':
                        try:
                            sample = supervisor.proc_sample(child.pid)
                        except FileNotFoundError:
                            if child.poll() is not None: break
                            raise
                        peak_rss = max(peak_rss, sample['rss']); peak_as = max(peak_as, sample['address'])
                        # Whole-child CPU30 is conservative relative to 30/pass.
                        supervisor.enforce_sample(limits, wall=now-started, cpu=sample['cpu'], rss=sample['rss'], address=sample['address'], threads=sample['threads'], free_disk=supervisor.shutil.disk_usage(root).free, scratch=stdout_path.stat().st_size)
                    if stdout_path.stat().st_size > limits['output']:
                        raise RuntimeError('qualification output cap')
                    samples += 1
                    time.sleep(0.1)
                code = child.wait()
            finally:
                if child.poll() is None: child.kill()
                child.wait()
        if code != 0:
            raise RuntimeError('qualification native failure: '+stdout_path.read_text()[:200])
        data = supervisor.bounded_bytes(stdout_path, limits['output'])
        result = supervisor.unique_json(data)
        return result, dict(wall_s=time.monotonic()-started, samples=samples, max_sample_interval_s=max_interval, peak_rss_bytes=peak_rss if sys.platform=='linux' else None, peak_address_bytes=peak_as if sys.platform=='linux' else None, owned_child_reaped=True)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--output', required=True)
    p.add_argument('--expected-child-sha256', required=True)
    p.add_argument('--expected-fixture-sha256', required=True)
    p.add_argument('--expected-core-test-sha256', required=True)
    p.add_argument('--portable-probe', action='store_true')
    args = p.parse_args()
    if not args.portable_probe and sys.platform != 'linux':
        p.error('Linux qualification required; portable probe is not release evidence')
    root = Path(args.output); root.mkdir(mode=0o700)
    started = time.monotonic()
    report: dict[str, Any] = dict(status='failed', production_read=False, production_retry_authorized=False, portable_probe=args.portable_probe, cases=[], required_bytes_per_cpu_s=REQUIRED)
    try:
        for name, expected in [('heaptrack-owner-child', args.expected_child_sha256), ('heaptrack-owner-fixture', args.expected_fixture_sha256), ('heaptrack-owner-core-tests', args.expected_core_test_sha256)]:
            if len(expected)!=64 or any(c not in '0123456789abcdef' for c in expected): raise RuntimeError('executable hash format')
            executable = ROOT/'target/release'/name
            if hashlib.sha256(supervisor.bounded_bytes(executable, 64*reference.MIB)).hexdigest()!=expected: raise RuntimeError('executable hash mismatch')
        report['executable_sha256'] = dict(child=args.expected_child_sha256, fixture=args.expected_fixture_sha256, core_tests=args.expected_core_test_sha256)
        core = subprocess.run([str(ROOT/'target/release/heaptrack-owner-core-tests'), '--test-threads=1'], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
        report['core_tests'] = core.stdout.decode()
        if core.returncode: raise RuntimeError('qualification native core test gate')
        suite = subprocess.run([sys.executable, '-B', '-m', 'unittest', 'discover', '-s', str(ROOT/'tests'), '-v'], cwd=ROOT.parents[2], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=180)
        report['tests'] = suite.stdout.decode()
        if suite.returncode or (not args.portable_probe and 'skipped' in report['tests']): raise RuntimeError('qualification test gate')
        for size, wide in CASES:
            if time.monotonic()-started > 900-180: raise RuntimeError('qualification total budget')
            raw, interpreted, peak = generated(size, wide)
            with fixture(raw, interpreted, peak, heads=False) as entries:
                expected = reference.analyze(entries, limits={'cpu':30, 'wall':60})
            request = dict(op='analyze', raw=raw.hex(), interpreted=interpreted.hex(), peak=peak.hex(), requested_time=None, outputs=True, limits={'cpu':30, 'wall':60})
            cell: dict[str, Any] = dict(size_bound=size, wide=wide, bytes=[len(raw),len(interpreted),len(interpreted)], records=[raw.count(b'\n'),interpreted.count(b'\n')], repeats=[])
            for repeat in range(3):
                result, resources = owned(request, ROOT/'target/release/heaptrack-owner-fixture', 900-(time.monotonic()-started))
                for key in ('raw','first','snapshots','canonical','canonical_equal'):
                    if result[key] != json.loads(json.dumps(expected[key])): raise RuntimeError('qualification differential mismatch')
                phases = result['measurements']
                rates = [n/phase['cpu_s'] for n, phase in zip(cell['bytes'], phases)]
                cell['repeats'].append(dict(resources=resources, phases=phases, bytes_per_cpu_s=rates))
            cell['median_bytes_per_cpu_s'] = [statistics.median(r['bytes_per_cpu_s'][i] for r in cell['repeats']) for i in range(3)]
            cell['necessary_rate_pass'] = all(a>=b for a,b in zip(cell['median_bytes_per_cpu_s'],REQUIRED)) if isinstance(wide, bool) else None
            report['cases'].append(cell)
            if cell['necessary_rate_pass'] is False: raise RuntimeError('necessary throughput threshold failed')
        report.update(status='portable_probe_passed' if args.portable_probe else 'fixture_qualification_passed', capacity='unproven: small warm fixtures do not bound production live population, tables, symbols, output or I/O')
        return 0
    except Exception as exc:
        report['reason'] = type(exc).__name__+': '+str(exc)[:300]
        return 1
    finally:
        report['wall_s'] = time.monotonic()-started
        (root/'qualification.json').write_text(json.dumps(report, indent=2)+'\n')


if __name__ == '__main__':
    raise SystemExit(main())
