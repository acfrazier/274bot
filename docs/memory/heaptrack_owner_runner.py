#!/usr/bin/env python3
"""Owned-child, fail-closed offline replay runner. Linux production; tiny fixtures elsewhere.

Never captures, reinterprets, starts a game, downloads symbols, or modifies the
parent environment/rlimits. Root must qualify this exact tool on its selected
Linux executor before releasing any production replay.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import shutil
import signal
import subprocess
import sys
import time
from typing import Any

import heaptrack_owner_replay as replay
from heaptrack_owner_replay import Invalid, require, MIB


def bounded_bytes(path, cap):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    import stat
    with os.fdopen(fd, 'rb') as stream:
        info = os.fstat(stream.fileno())
        require(stat.S_ISREG(info.st_mode) and info.st_size <= cap, 'metadata file cap/type')
        data = stream.read(cap+1)
    require(len(data) <= cap, 'metadata file cap')
    return data


def unique_json(data):
    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result, 'duplicate JSON key')
            result[key] = value
        return result
    return json.loads(data, object_pairs_hook=pairs)


def load_manifest(path, expected, portable):
    require(bool(re.fullmatch('[0-9a-f]{64}', expected)), 'manifest hash format')
    data = bounded_bytes(path, 64*1024)
    require(hashlib.sha256(data).hexdigest() == expected, 'manifest hash mismatch')
    m = unique_json(data)
    require(set(m) == {'schema', 'inputs', 'analysis_path', 'provenance', 'suppression_policy', 'requested_time'}, 'manifest schema fields')
    require(m['schema'] == 'heaptrack-owner-input/v1', 'manifest schema')
    require(set(m['inputs']) == {'raw', 'interpreted', 'peak', 'receipt', 'stderr'}, 'input roles')
    p = m['provenance']
    require(set(p) == {'scope', 'host', 'client', 'binary_sha256', 'capture_tooling', 'process_role',
            'collection_mode', 'intentional_pause', 'reinitialized', 'capture_status', 'campaign_capture_status'}, 'provenance schema')
    require(p['collection_mode'] == 'direct-preload' and p['intentional_pause'] is False and p['reinitialized'] is False, 'unsupported collection provenance')
    require(p['campaign_capture_status'] == 'failed_raw_size_guard', 'failed capture boundary required')
    require(p['capture_tooling'] == '1fe21610b6446779133b5ff0dc229ba93599ecc0', 'capture tooling identity')
    if portable:
        require(p['scope'] == 'controlled_saved_fixture' and p['process_role'] == 'owned_python_fixture', 'portable fixtures only')
        require(all(e['bytes'] <= 2_000_000 for e in m['inputs'].values()), 'portable input cap')
    else:
        require(sys.platform == 'linux', 'Linux required for released executor')
        require(p['scope'] == 'failed_saved_production_prefix' and p['process_role'] == 'client_tui', 'production scope/role')
        require((p['host'], p['client']) == (replay.HOST, replay.CLIENT), 'frozen source identity')
        require(p['binary_sha256'] == 'a0c6eb0bed428fefad58530b177caae16ba2df6c7153544bb0852e16485591f9', 'frozen binary identity')
        require(p['capture_status'] == 'failed_raw_size_guard', 'production capture failure reason')
    require(m['suppression_policy'] == 'unsuppressed_replay_canonical_default_leaks_not_comparable', 'suppression policy')
    require(m['requested_time'] is None or (type(m['requested_time']) is int and 0 <= m['requested_time'] <= replay.U64), 'requested time')
    require(isinstance(m['analysis_path'], list) and len(m['analysis_path']) <= 8 and
            all(isinstance(k, str) and len(k) <= 80 for k in m['analysis_path']), 'analysis receipt selector')
    for key, entry in m['inputs'].items():
        require(Path(entry['path']).is_absolute(), 'absolute immutable input paths required')
        replay.Input(entry, replay.LIMITS.get(key, MIB), replay.LIMITS['line'])
    return m


def verify_receipt(m):
    def read(entry):
        data = bounded_bytes(entry['path'], MIB)
        require(len(data) == entry['bytes'] and hashlib.sha256(data).hexdigest() == entry['sha256'], 'receipt/stderr hash mismatch')
        return data
    receipt = unique_json(read(m['inputs']['receipt']))
    for key in m['analysis_path']:
        receipt = receipt[key]
    require(receipt['interpreter']['exit_code'] == 0 and receipt['printer']['exit_code'] == 0, 'saved conversion/printer failed')
    argv = receipt['printer']['argv']
    require('--merge-backtraces=0' in argv and '--flamegraph-cost-type=peak' in argv,
            'canonical printer must be unmerged common-time peak')
    require(not any(a.startswith(('--filter', '--shorten-templates', '--flamegraph-cost-type=')) and a != '--flamegraph-cost-type=peak' for a in argv), 'unsupported canonical printer rendering/filter')
    require(receipt['interpreter']['output_size'] == m['inputs']['interpreted']['bytes'], 'interpreter receipt output size')
    stderr = read(m['inputs']['stderr'])
    match = re.fullmatch(rb'heaptrack stats:\n\s*allocations:\s*(\d+)\n\s*leaked allocations:\s*(\d+)\n\s*temporary allocations:\s*(\d+)\n', stderr)
    require(match is not None, 'unexpected interpreter stderr (conversion warning/error)')
    assert match is not None
    return tuple(replay.checked(int(n)) for n in match.groups())


def linux_memory():
    fields = {}
    for line in Path('/proc/meminfo').read_text().splitlines():
        key, value = line.split(':', 1)
        fields[key] = int(value.split()[0])*1024
    return fields['MemAvailable']


def proc_sample(pid) -> dict[str, Any]:
    # Only the directly retained child's PID. No scanning/killing foreign PIDs.
    text = Path(f'/proc/{pid}/stat').read_text()
    fields = text[text.rfind(')')+2:].split()
    ticks = os.sysconf('SC_CLK_TCK')
    status = Path(f'/proc/{pid}/status').read_text()
    values = {}
    for line in status.splitlines():
        if line.startswith(('VmRSS:', 'VmSize:', 'Threads:')):
            key, value = line.split(':', 1)
            values[key] = int(value.split()[0])
    return dict(cpu=(int(fields[11])+int(fields[12]))/ticks,
                rss=values.get('VmRSS', 0)*1024, address=values.get('VmSize', 0)*1024,
                threads=values['Threads'])


def enforce_sample(limits, *, wall, cpu, rss, address, free_disk, scratch, threads: int = 1):
    for condition, reason in (
        (wall <= limits['wall'], 'wall guard'), (cpu <= limits['cpu'], 'CPU guard'),
        (rss <= limits['rss'], 'RSS guard'), (address <= limits['address'], 'address-space guard'),
        (free_disk >= limits['free_disk'], 'free disk guard'),
        (scratch <= limits['scratch'], 'scratch guard'), (threads == 1, 'single worker thread guard')):
        require(condition, reason)


def admit(limits, available, free):
    require(available >= limits['admission_memory'], 'admission memory guard')
    require(free >= limits['admission_disk'], 'admission disk guard')


class PhaseGuard:
    def __init__(self, limits, send=lambda data: None):
        self.limits, self.send = limits, send
        self.phase = 0
        self.start_wall, self.start_cpu = time.monotonic(), time.process_time()
        self.measurements = []

    def check(self):
        import resource
        use = resource.getrusage(resource.RUSAGE_SELF)
        rss = use.ru_maxrss*(1 if sys.platform == 'darwin' else 1024)
        self.peak_rss = rss
        require(time.monotonic()-self.start_wall <= self.limits['wall'], 'wall guard')
        require(time.process_time()-self.start_cpu <= self.limits['cpu'], 'CPU guard')
        require(rss <= self.limits['rss'], 'RSS guard')

    def next(self, number):
        self.check()
        now, cpu = time.monotonic(), time.process_time()
        if self.phase:
            self.measurements.append(dict(phase=self.phase, wall_s=now-self.start_wall,
                                          cpu_s=cpu-self.start_cpu, cumulative_peak_rss_bytes=self.peak_rss))
        require(number == self.phase+1 and number <= 4, 'phase sequence')
        self.phase = number
        self.start_wall, self.start_cpu = now, cpu
        self.send(dict(phase=number, wall_start=now, cpu_start=cpu))


def child(args):
    # Child-only limits, set before opening trace data. Linux AS uses BYTES.
    import resource
    limits = replay.Budget(json.loads(args.limits)).limits
    resource.setrlimit(resource.RLIMIT_CPU, (3*limits['cpu']+1, 3*limits['cpu']+1))
    resource.setrlimit(resource.RLIMIT_FSIZE, (limits['scratch'], limits['scratch']))
    if sys.platform == 'linux':
        resource.setrlimit(resource.RLIMIT_AS, (limits['address'], limits['address']))
    output = Path(args.output)/'.pending'
    def send(data):
        print(json.dumps(data, separators=(',', ':')), flush=True)
    guard = PhaseGuard(limits, send)
    # Check on every bounded record/read and output write in addition to the
    # external <=0.25s monitor; no scheduling-dependent success at phase end.
    replay.PULSE = guard.check
    m = load_manifest(args.manifest, args.manifest_sha256, args.portable_fixture)
    expected = verify_receipt(m)
    result = replay.analyze(m['inputs'], m['requested_time'], json.loads(args.limits), guard.next)
    require((result['raw']['allocations'], result['raw']['count'], result['raw']['temporary']) == expected, 'interpreter stats mismatch')
    provenance = dict(m['provenance'], manifest_sha256=args.manifest_sha256,
                      input_hashes={k: {'bytes': e['bytes'], 'sha256': e['sha256']} for k, e in m['inputs'].items()},
                      requested_time=m['requested_time'], suppression_policy=m['suppression_policy'])
    replay.write_outputs(result, output, provenance, dict(linux_release=not args.portable_fixture,
        qualification='requires_root_native_guard_smoke' if not args.portable_fixture else 'portable_fixture_only',
        completed_passes=guard.measurements, child_pid=os.getpid()))
    guard.next(4)  # end of phase 3 includes canonical reconciliation and output
    send(dict(done=True, measurements=guard.measurements))
    return 0


def directory_size(path):
    # Fixed, flat owned output only; reject subdirs/symlinks/unknown entries.
    total = 0
    for entry in path.iterdir():
        require(entry.is_file() and not entry.is_symlink(), 'unexpected scratch entry')
        total += entry.stat().st_size
    return total


def supervise(command, output, limits, linux) -> dict[str, Any]:
    pending = output/'.pending'
    pending.mkdir(mode=0o700)
    started = time.monotonic()
    start_wall, start_cpu, phase = started, 0.0, 0
    peak_rss = peak_as = 0
    messages = []
    buffer = b''
    done = False
    failure = None
    child_proc = None
    try:
        # DEVNULL bounds diagnostics; child sends only fixed protocol/error codes.
        env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', OMP_NUM_THREADS='1', OPENBLAS_NUM_THREADS='1')
        child_proc = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                      stderr=subprocess.DEVNULL, env=env, close_fds=True)
        assert child_proc.stdout is not None
        with selectors.DefaultSelector() as selector:
            selector.register(child_proc.stdout, selectors.EVENT_READ)
            while True:
                for key, _ in selector.select(timeout=0.25):
                    data = os.read(key.fd, 4096)
                    if not data:
                        selector.unregister(key.fileobj)
                    buffer += data
                    require(len(buffer) <= 8192, 'child protocol cap')
                    while b'\n' in buffer:
                        line, buffer = buffer.split(b'\n', 1)
                        value = unique_json(line)
                        require(len(messages) < 5, 'child protocol count')
                        messages.append(value)
                        if 'phase' in value:
                            require(value['phase'] == phase+1 and value['phase'] <= 4, 'child phase protocol')
                            phase = value['phase']
                            start_wall, start_cpu = value['wall_start'], value['cpu_start']
                        elif value.get('done') is True:
                            require(phase == 4 and not done, 'child done protocol')
                            done = True
                        else:
                            raise Invalid('child: '+str(value.get('error', 'validation failure'))[:200])
                now = time.monotonic()
                require(now-started <= 3*limits['wall'], 'total wall guard')
                sample: dict[str, Any] = dict(cpu=start_cpu, rss=0, address=0, threads=1)
                if linux and child_proc.poll() is None:
                    sample = proc_sample(child_proc.pid)
                peak_rss = max(peak_rss, sample['rss'])
                peak_as = max(peak_as, sample['address'])
                enforce_sample(limits, wall=now-start_wall, cpu=max(0, sample['cpu']-start_cpu),
                    rss=sample['rss'], address=sample['address'], threads=sample['threads'],
                    free_disk=shutil.disk_usage(output).free, scratch=directory_size(pending))
                code = child_proc.poll()
                if code is not None and not selector.get_map():
                    require(code == 0 and done and not buffer, 'child unsuccessful/incomplete')
                    break
        return dict(child_pid=child_proc.pid, exit_code=child_proc.returncode, owned_child_reaped=True,
                    wall_s=time.monotonic()-started, peak_rss_bytes=peak_rss if linux else None,
                    peak_address_bytes=peak_as if linux else None, linux_proc_monitor=linux, phases=messages[-1]['measurements'])
    except BaseException as exc:
        failure = exc
        setattr(exc, 'replay_resources', dict(child_pid=child_proc.pid if child_proc else None,
            wall_s=time.monotonic()-started, peak_rss_bytes=peak_rss if linux else None,
            peak_address_bytes=peak_as if linux else None, phase=phase, linux_proc_monitor=linux))
        raise
    finally:
        if child_proc is not None:
            if child_proc.poll() is None:
                child_proc.kill()  # exactly retained child; no process-name kill
            child_proc.wait()
            if failure is not None:
                getattr(failure, 'replay_resources')['exit_code'] = child_proc.returncode
                getattr(failure, 'replay_resources')['owned_child_reaped'] = True
            if child_proc.stdout:
                child_proc.stdout.close()
        if failure is not None:
            shutil.rmtree(pending)


def save_json(path, value):
    data = (json.dumps(value, indent=2)+'\n').encode()
    require(len(data) <= 64*1024, 'receipt size cap')
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, 'wb') as stream:
        stream.write(data)
    return len(data)


def run(args):
    output = Path(args.output)
    output.mkdir(mode=0o700)  # fresh path only; never remove or overwrite user files
    try:
        limits = replay.Budget(json.loads(args.limits)).limits
        load_manifest(args.manifest, args.manifest_sha256, args.portable_fixture)
        if not args.portable_fixture:
            require(sys.platform == 'linux', 'Linux executor required')
            admit(limits, linux_memory(), shutil.disk_usage(output).free)
        else:
            require(shutil.disk_usage(output).free >= limits['admission_disk'], 'admission disk guard')
        command = [sys.executable, '-B', str(Path(__file__).resolve()), '--child',
            '--manifest', args.manifest, '--manifest-sha256', args.manifest_sha256,
            '--output', str(output), '--limits', args.limits]
        if args.portable_fixture:
            command.append('--portable-fixture')
        result = supervise(command, output, limits, sys.platform == 'linux')
        # Parent independently verifies every bounded output before publication.
        pending = output/'.pending'
        receipt_data = bounded_bytes(pending/'receipt.json', MIB)
        receipt = unique_json(receipt_data)
        require(receipt['capture_complete'] is False and receipt['canonical_full_positive_multiset_equal'] is True, 'child receipt validation')
        require(set(p.name for p in pending.iterdir()) == set(receipt['files']) | {'receipt.json'}, 'unexpected result files')
        for name, entry in receipt['files'].items():
            require(Path(name).name == name, 'output name')
            data = bounded_bytes(pending/name, limits['output'])
            require(len(data) == entry['bytes'] and hashlib.sha256(data).hexdigest() == entry['sha256'], 'output integrity')
        result.update(status='validated_prefix_diagnostic', capture_complete=False, acceptance=False,
            manifest_sha256=args.manifest_sha256, receipt_sha256=hashlib.sha256(receipt_data).hexdigest(),
            tooling_sha256={name: hashlib.sha256(bounded_bytes(Path(__file__).with_name(name), MIB)).hexdigest()
                for name in ('heaptrack_owner_runner.py', 'heaptrack_owner_replay.py')})
        n = save_json(output/'runner.json', result)
        require(directory_size(pending)+n <= limits['output'], 'total output cap including runner receipt')
        require(shutil.disk_usage(output).free >= limits['free_disk'], 'free disk guard')
        pending.rename(output/'result')
        return 0
    except BaseException as exc:
        pending = output/'.pending'
        if pending.exists():
            shutil.rmtree(pending)
        if (output/'runner.json').exists():
            (output/'runner.json').unlink()
        # Do not copy exception payloads or raw command lines to durable output.
        reason = str(exc) if isinstance(exc, Invalid) else type(exc).__name__
        save_json(output/'failure.json', dict(status='failed', capture_complete=False, acceptance=False,
            reason=reason[:200], manifest_sha256=args.manifest_sha256,
            resources=getattr(exc, 'replay_resources', {}),
            campaign_capture_status='failed_raw_size_guard'))
        return 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', required=True)
    parser.add_argument('--manifest-sha256', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--portable-fixture', action='store_true')
    parser.add_argument('--limits', default='{}', help='JSON lower-only guard overrides for small tests')
    parser.add_argument('--child', action='store_true', help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.child:
        try:
            return child(args)
        except BaseException as exc:
            # Invalid messages are internal static reason codes, never raw lines.
            reason = str(exc) if isinstance(exc, Invalid) else type(exc).__name__
            print(json.dumps({'error': reason[:200]}), flush=True)
            return 1
    def interrupted(signum, frame):
        raise Invalid('controller signal '+str(signum))
    old_term = signal.signal(signal.SIGTERM, interrupted)
    try:
        return run(args)
    finally:
        signal.signal(signal.SIGTERM, old_term)


if __name__ == '__main__':
    raise SystemExit(main())
