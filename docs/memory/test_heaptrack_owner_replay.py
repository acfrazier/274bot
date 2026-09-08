"""Offline contract tests; never launches a game or Heaptrack."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
import hashlib
import sys
import json
import subprocess
from unittest.mock import patch
from contextlib import contextmanager

sys.path.insert(0, str(Path(__file__).parent))
import heaptrack_owner_replay as replay

MODULE = Path(__file__).with_name('heaptrack_owner_replay.py')


def smoke_entries():
    root = Path('diagnostics/owner-capture-evidence-2015')
    manifest = json.loads((root/'owner-capture-evidence-2015-manifest.json').read_text())
    entries = {}
    for kind, name in [('raw', 'alloc.raw'), ('interpreted', 'alloc.interpreted'), ('peak', 'peak-stacks.txt')]:
        relative = 'heaptrack-seam-smoke-2001/capture/'+name
        item = next(e for e in manifest['files'] if e['archive_path'] == relative)
        entries[kind] = dict(path=str(root/relative), bytes=item['bytes'], sha256=item['sha256'])
    return entries


def smoke_manifest(directory):
    root = Path('diagnostics/owner-capture-evidence-2015').resolve()
    inventory = json.loads((root/'owner-capture-evidence-2015-manifest.json').read_text())
    entries = smoke_entries()
    for entry in entries.values():
        entry['path'] = str(Path(entry['path']).resolve())
    for kind, relative in [('receipt', 'receipt.json'), ('stderr', 'capture/interpret-stderr.txt')]:
        relative = 'heaptrack-seam-smoke-2001/'+relative
        item = next(e for e in inventory['files'] if e['archive_path'] == relative)
        entries[kind] = dict(path=str(root/relative), bytes=item['bytes'], sha256=item['sha256'])
    manifest = dict(schema='heaptrack-owner-input/v1', inputs=entries, analysis_path=['analysis'],
        provenance=dict(scope='controlled_saved_fixture', host=None, client=None, binary_sha256=None,
            capture_tooling='1fe21610b6446779133b5ff0dc229ba93599ecc0', process_role='owned_python_fixture',
            collection_mode='direct-preload', intentional_pause=False, reinitialized=False,
            capture_status='fixture_passed_campaign_capture_failed', campaign_capture_status='failed_raw_size_guard'),
        suppression_policy='unsuppressed_replay_canonical_default_leaks_not_comparable', requested_time=500)
    data = json.dumps(manifest).encode()
    path = directory/'manifest.json'
    path.write_bytes(data)
    return path, hashlib.sha256(data).hexdigest()


RAW_HEAD = b'v 10500 3\nx 1 x\nX fixture\nI 1000 100\nt 10 0\n'
INT_HEAD = b'v 10500 3\nX fixture\nI 1000 100\ns 1 f\ni 10 0 1\nt 1 0\n'


@contextmanager
def fixture(raw=b'+ a 1 10\nc a\n', interpreted=b'a a 1\n+ 0\nc a\n', peak=b'f; 10\n', heads=True):
    with tempfile.TemporaryDirectory() as directory:
        entries = {}
        for kind, data in [('raw', (RAW_HEAD if heads else b'')+raw),
                           ('interpreted', (INT_HEAD if heads else b'')+interpreted), ('peak', peak)]:
            path = Path(directory)/kind
            path.write_bytes(data)
            entries[kind] = dict(path=str(path), bytes=len(data), sha256=hashlib.sha256(data).hexdigest())
        yield entries


class ReplayTests(unittest.TestCase):
    def test_strict_records_preserve_string_bytes(self):
        self.assertTrue(MODULE.exists(), 'formal replay module is not implemented')
        spec = importlib.util.spec_from_file_location('replay', MODULE)
        assert spec is not None and spec.loader is not None
        replay = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(replay)
        self.assertEqual(replay.parse_record(b's 4 a \xc3\xa9\n'), ('s', b'a \xc3\xa9'))
        self.assertEqual(replay.parse_record(b'i 1 0 0\n'), ('i', [1, 0, 0]))
        for bad in (b's 3 a \xc3\xa9\n', b'c 1', b'i 1 0 0 0\n', b'c 1 2\n', b'c A\n'):
            with self.subTest(bad=bad), self.assertRaises(replay.Invalid):
                replay.parse_record(bad)

    def test_three_pass_baseline_and_first_peak(self):
        self.assertTrue(hasattr(replay, 'analyze'), 'three-pass replay is missing')
        raw = b'v 10500 3\nx 1 x\nX fixture\nI 1000 100\nt 10 0\n+ a 1 10\n+ a 1 20\nc a\n- 10\nc 14\n+ a 1 10\n- 20\n'
        interpreted = b'v 10500 3\nX fixture\nI 1000 100\ns 1 f\ni 10 0 1\nt 1 0\na a 1\n+ 0\n+ 0\nc a\n- 0\nc 14\n+ 0\n- 0\n'
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            entries = {}
            for kind, data in [('raw', raw), ('interpreted', interpreted), ('peak', b'f; 20\n')]:
                (root/kind).write_bytes(data)
                entries[kind] = {'path': str(root/kind), 'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}
            result = replay.analyze(entries, requested_time=15)
        self.assertEqual(result['first']['peak']['line'], 9)
        self.assertEqual(result['first']['peak']['bytes'], 20)
        self.assertEqual(result['first']['eof']['bytes'], 10)
        self.assertEqual(result['first']['requested']['timestamp_ms'], 10)
        self.assertEqual(result['first']['next_mark']['timestamp_ms'], 20)
        self.assertEqual(result['snapshots']['requested']['rows'], {1: [20, 2, 2]})
        self.assertEqual(result['snapshots']['last_mark']['rows'], {1: [10, 1, 2]})
        self.assertTrue(result['canonical_equal'])

    def test_saved_smoke_full_canonical_multiset(self):
        result = replay.analyze(smoke_entries(), requested_time=500)
        first, raw = result['first'], result['raw']
        self.assertEqual(first['peak']['bytes'], 5730024)
        self.assertEqual(first['peak']['line'], 27144)
        self.assertEqual(first['eof']['bytes'], 407233)
        self.assertEqual(first['eof']['count'], 34)
        self.assertEqual(first['requested']['timestamp_ms'], 497)
        self.assertEqual(first['requested']['count'], 4962)
        self.assertEqual(first['last_mark']['timestamp_ms'], 1064)
        self.assertEqual((raw['temporary'], first['temporary']), (637, 638))
        self.assertEqual(raw['unknown_frees'], 2)
        self.assertEqual(first['max_multiplicity'], 2048)
        self.assertEqual(result['canonical'], dict(rows=2951, positive_rows=2212, bytes=5730024))
        tables = result['tables']
        def bytearray_bytes(cutoff):
            total = 0
            for trace, row in result['snapshots'][cutoff]['rows'].items():
                symbols = []
                for ip_id in replay.stack(tables, trace):
                    ip = tables.ip(ip_id)
                    symbols.extend(tables.string(ip[i]) for i in range(2, len(ip), 3))
                if b'PyByteArray_Resize' in symbols:
                    total += row[0]
            return total
        self.assertEqual(bytearray_bytes('peak'), 4196352)
        self.assertEqual(bytearray_bytes('eof'), 0)

    def test_lossless_output_and_fail_closed_output_budget(self):
        self.assertTrue(hasattr(replay, 'write_outputs'), 'bounded output is missing')
        result = replay.analyze(smoke_entries(), requested_time=500)
        with tempfile.TemporaryDirectory() as directory:
            replay.write_outputs(result, Path(directory), {'scope': 'fixture'})
            receipt = json.loads((Path(directory)/'receipt.json').read_text())
            self.assertFalse(receipt['capture_complete'])
            self.assertEqual(receipt['cutoffs']['requested']['actual_mark']['timestamp_ms'], 497)
            self.assertEqual(receipt['cutoffs']['requested']['next_mark']['timestamp_ms'], 507)
            self.assertIn('PyByteArray_Resize', (Path(directory)/'strings.tsv').read_text())
            self.assertEqual(len((Path(directory)/'peak-stacks.tsv').read_text().splitlines()), 2213)
        with tempfile.TemporaryDirectory() as directory:
            result['tables'].b.limits['output'] = 100
            with self.assertRaisesRegex(replay.Invalid, 'output cap'):
                replay.write_outputs(result, Path(directory), {'scope': 'fixture'})

    def test_owned_portable_runner_smoke(self):
        runner = MODULE.with_name('heaptrack_owner_runner.py')
        self.assertTrue(runner.exists(), 'owned child runner missing')
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest, digest = smoke_manifest(root)
            run = subprocess.run([sys.executable, str(runner), '--manifest', str(manifest),
                '--manifest-sha256', digest, '--output', str(root/'out'), '--portable-fixture'], capture_output=True)
            self.assertEqual(run.returncode, 0, run.stderr.decode()+run.stdout.decode())
            result = json.loads((root/'out/result/receipt.json').read_text())
            self.assertFalse(result['resources']['linux_release'])
            self.assertEqual(result['canonical_peak']['bytes'], 5730024)
            self.assertTrue((root/'out/runner.json').exists())

    def test_synthetic_realloc_multiplicity_zero_and_survivor(self):
        # Same address realloc, moved realloc, failed realloc (no events), reuse,
        # size-zero count, and distinct objects with the same bytes at two marks.
        raw = b'+ a 1 10\n+ a 1 20\n+ 0 1 30\nc a\n- 10\n+ 14 1 10\n- 10\n+ a 1 40\nc a\n- 40\n+ a 1 10\nc 14\n- 10\n- 20\n- 30\n- ff\n'
        interpreted = b'a a 1\n+ 0\n+ 0\na 0 1\n+ 1\nc a\n- 0\na 14 1\n+ 2\n- 2\n+ 0\nc a\n- 0\n+ 0\nc 14\n- 0\n- 0\n- 1\n'
        with fixture(raw, interpreted, b'f; 30\n') as entries:
            result = replay.analyze(entries, requested_time=10)
        self.assertEqual(result['first']['requested']['ordinal'], 2)
        self.assertEqual(result['snapshots']['requested']['count'], 3)
        self.assertEqual(result['snapshots']['requested']['bytes'], 20)
        self.assertEqual(result['snapshots']['last_mark']['bytes'], 20)
        self.assertEqual(result['snapshots']['eof']['count'], 0)
        self.assertEqual(result['raw']['unknown_frees'], 1)
        self.assertEqual(result['snapshots']['requested']['rows'][1][2], 5)

    def test_time_rejections_and_unbounded_event_tail(self):
        for request in (-1, True, 0, 9, 11, 1 << 64):
            with self.subTest(request=request), fixture() as entries, self.assertRaises(replay.Invalid):
                replay.analyze(entries, requested_time=request)
        with fixture(b'+ a 1 10\n', b'a a 1\n+ 0\n') as entries, self.assertRaisesRegex(replay.Invalid, 'no actual'):
            replay.analyze(entries)
        with fixture(b'+ a 1 10\nc a\n- 10\n', b'a a 1\n+ 0\nc a\n- 0\n') as entries:
            result = replay.analyze(entries)
        self.assertEqual(result['snapshots']['last_mark']['bytes'], 10)
        self.assertEqual(result['snapshots']['eof']['bytes'], 0)
        self.assertGreater(result['first']['last_event']['line'], result['first']['last_mark']['line'])
        with fixture(b'c b\nc a\n', b'c b\nc a\n', b'f; 0\n') as entries, self.assertRaisesRegex(replay.Invalid, 'nonmonotonic'):
            replay.analyze(entries)

    def test_record_adversaries(self):
        bad = [b'c 1', b's 2 x\n', b's 1 \xff\n', b'i 1 0 0 0\n', b'i 1 0 0 0 0 0\n',
            b't 1 0 0\n', b'a 1 1 1\n', b'c -1\n', b'c +1\n', b'c 0x1\n', b'c A\n',
            b'c 10000000000000000\n', b'c  1\n', b'c 1 \n', b'A\n', b'Q 1\n', b'v 10500\n']
        for data in bad:
            with self.subTest(data=data), self.assertRaises(replay.Invalid):
                replay.parse_record(data)
        for data in (b'm 1 x 0 1\n', b'm 1 - 0\n', b'x 2 x\n', b'm 1 x ffffffffffffffff 1 1\n'):
            with self.subTest(raw=data), self.assertRaises(replay.Invalid):
                replay.parse_record(data, raw=True)
        with self.assertRaisesRegex(replay.Invalid, 'line cap'):
            replay.parse_record(b'#'+b'x'*100+b'\n', line_limit=100)

    def test_reference_and_session_adversaries(self):
        tails = [b'a a 1\n- 0\nc a\n', b'+ 0\nc a\n', b'a a 1\n+ 1\nc a\n',
            b'a a 2\nc a\n', b't 1 2\nc a\n', b't 2 0\nc a\n', b'i 1 2\nc a\n',
            b'i 1 0 2\nc a\n', b'i 1 0 1 2 1\nc a\n', b'a a 1\na a 1\nc a\n',
            b'v 10500 3\nc a\n', b'X other\nc a\n', b'I 1 1\nc a\n', b'A\nc a\n', b'Q\nc a\n']
        for tail in tails:
            with self.subTest(tail=tail), fixture(interpreted=tail) as entries, self.assertRaises(replay.Invalid):
                replay.analyze(entries)
        for tail in [b'+ a 1 10\n+ a 1 10\nc a\n', b'+ a 2 10\nc a\n', b't 10 2\nc a\n',
                     b'x 1 x\nc a\n', b'v 10500 3\nc a\n', b'+ a 1 0\nc a\n']:
            with self.subTest(raw=tail), fixture(raw=tail) as entries, self.assertRaises(replay.Invalid):
                replay.analyze(entries)

    def test_conversion_hash_suffix_and_canonical_not_sum(self):
        with fixture(interpreted=b'a b 1\n+ 0\nc a\n') as entries, self.assertRaisesRegex(replay.Invalid, 'conversion'):
            replay.analyze(entries)
        with fixture(peak=b'wrong; 10\n') as entries, self.assertRaisesRegex(replay.Invalid, 'canonical'):
            replay.analyze(entries)
        with fixture() as entries:
            # Whole-record suffix deletion from an already-bound file fails.
            path = Path(entries['raw']['path'])
            path.write_bytes(path.read_bytes()[:-4])
            with self.assertRaises(replay.Invalid):
                replay.analyze(entries)
        with fixture() as entries:
            entries['raw']['sha256'] = '0'*64
            with self.assertRaisesRegex(replay.Invalid, 'hash'):
                replay.analyze(entries)
        with fixture() as entries:
            def phase(n):
                if n == 3:
                    p = Path(entries['interpreted']['path'])
                    p.write_bytes(p.read_bytes().replace(b'+ 0', b'- 0'))
            with self.assertRaises(replay.Invalid):
                replay.analyze(entries, phase=phase)
        # A newly authorized, syntactically valid shorter prefix is unknowable
        # from syntax: the output contract must still say capture_complete=false.
        with fixture() as entries:
            result = replay.analyze(entries)
            with tempfile.TemporaryDirectory() as directory:
                replay.write_outputs(result, Path(directory), {})
                self.assertFalse(json.loads((Path(directory)/'receipt.json').read_text())['capture_complete'])

    def test_each_table_numeric_input_and_depth_cap(self):
        # Lower-only limits drive the SAME production rejection code with small inputs.
        for key, cap in [('table_bytes', 1), ('raw', 1), ('interpreted', 1), ('peak', 1), ('line', 8)]:
            with self.subTest(key=key), fixture() as entries, self.assertRaises(replay.Invalid):
                replay.analyze(entries, limits={key: cap})
        cases = [({'pointers': 1}, b'+ a 1 10\n+ a 1 20\nc a\n', b'a a 1\n+ 0\n+ 0\nc a\n'),
            ({'traces': 1}, b't 20 1\nc a\n', b't 1 1\nc a\n'),
            ({'ips': 1}, b'c a\n', b'i 20 0\nc a\n'),
            ({'strings': 1}, b'c a\n', b's 1 g\nc a\n'),
            ({'string_bytes': 1}, b'c a\n', b's 1 g\nc a\n'),
            ({'descriptors': 1}, b'c a\n', b'a a 1\na b 1\nc a\n')]
        for limit, raw, interpreted in cases:
            with self.subTest(limit=limit), fixture(raw, interpreted) as entries, self.assertRaises(replay.Invalid):
                replay.analyze(entries, limits=limit)
        with fixture(b'+ 8000000000000000 1 10\nc a\n') as entries, self.assertRaisesRegex(replay.Invalid, 'overflow'):
            replay.analyze(entries)
        with self.assertRaises(replay.Invalid):
            replay.checked((1 << 62)*4)
        tables = replay.Tables(replay.Budget({'depth': 1}))
        tables.definition('i', [1, 0])
        tables.definition('t', [1, 0])
        tables.definition('t', [1, 1])
        with self.assertRaisesRegex(replay.Invalid, 'depth'):
            replay.stack(tables, 2)

    def test_suppression_does_not_change_population(self):
        with fixture(raw=b'S f\n+ a 1 10\nc a\n', interpreted=b'S f\na a 1\n+ 0\nc a\n') as entries:
            result = replay.analyze(entries)
            self.assertEqual(result['tables'].suppressions, [b'f'])
            self.assertEqual(result['snapshots']['eof']['bytes'], 10)
            # An explicitly suppressed canonical-leak fixture would show zero;
            # that presentation is not a free and is NOT our peak oracle.
            self.assertNotEqual(result['snapshots']['eof']['bytes'], int(b'f; 0'.rsplit(b' ', 1)[1]))

    def test_normalization_collisions_new_stop_inline_unresolved(self):
        tables = replay.Tables(replay.Budget())
        for text in (b'main', b'operator new(unsigned long)', b'f<T>', b'/dir/f.rs', b'inlined<U>'):
            tables.definition('s', text)
        for fields in ([1, 0, 1], [2, 0, 2], [3, 0, 3, 4, 1, 5, 0, 0], [4, 0]):
            tables.definition('i', fields)
        for fields in ([4, 0], [1, 1], [2, 2], [3, 3], [3, 4]):
            tables.definition('t', fields)
        self.assertEqual(replay.canonical(tables, 4), b'main;f<> (f.rs);inlined<> ();')
        self.assertEqual(replay.canonical(tables, 1), b'0x4;')
        self.assertEqual(replay.pretty(b'operator<< <T>'), b'operator<< <>')
        with fixture(peak=b'main;f<> (f.rs);inlined<> (); 4\nmain;f<> (f.rs);inlined<> (); 6\n') as entries:
            replay.reconcile_peak(entries['peak'], tables, {'rows': {4: [10, 1, 1]}, 'bytes': 10})

    def test_resource_guards_and_parent_unchanged(self):
        import heaptrack_owner_runner as runner
        import os
        import resource
        limits = replay.LIMITS
        base = dict(wall=0, cpu=0, rss=0, address=0, free_disk=limits['free_disk'], scratch=0)
        for field, value in [('wall', 301), ('cpu', 181), ('rss', limits['rss']+1),
            ('address', limits['address']+1), ('free_disk', limits['free_disk']-1), ('scratch', limits['scratch']+1), ('threads', 2)]:
            with self.subTest(field=field), self.assertRaises(replay.Invalid):
                runner.enforce_sample(limits, **dict(base, **{field: value}))
        for available, free in [(limits['admission_memory']-1, limits['admission_disk']),
                                (limits['admission_memory'], limits['admission_disk']-1)]:
            with self.assertRaises(replay.Invalid):
                runner.admit(limits, available, free)
        before = (dict(os.environ), resource.getrlimit(resource.RLIMIT_CPU), resource.getrlimit(resource.RLIMIT_FSIZE))
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest, digest = smoke_manifest(root)
            command = [sys.executable, str(MODULE.with_name('heaptrack_owner_runner.py')), '--manifest', str(manifest),
                '--manifest-sha256', digest, '--output', str(root/'out'), '--portable-fixture', '--limits', '{"output":100}']
            run = subprocess.run(command, capture_output=True)
            self.assertNotEqual(run.returncode, 0)
            self.assertEqual([p.name for p in (root/'out').iterdir()], ['failure.json'])
        self.assertEqual(before, (dict(os.environ), resource.getrlimit(resource.RLIMIT_CPU), resource.getrlimit(resource.RLIMIT_FSIZE)))

    def test_real_owned_wall_kill_reap_and_bounded_failure(self):
        import heaptrack_owner_runner as runner
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with self.assertRaisesRegex(replay.Invalid, 'wall'):
                runner.supervise([sys.executable, '-c', 'import time; time.sleep(5)'], root,
                                 replay.Budget({'wall': 1}).limits, False)
            self.assertFalse((root/'.pending').exists())

    def test_resource_failure_reports_exact_guard_and_usage(self):
        runner = MODULE.with_name('heaptrack_owner_runner.py')
        for limits, reason in [({'rss': 1}, 'RSS guard'), ({'table_bytes': 1}, 'table byte cap'),
                               ({'output': 100}, 'output cap'), ({'scratch': 100}, 'scratch')]:
            with self.subTest(limits=limits), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                manifest, digest = smoke_manifest(root)
                run = subprocess.run([sys.executable, str(runner), '--manifest', str(manifest),
                    '--manifest-sha256', digest, '--output', str(root/'out'), '--portable-fixture',
                    '--limits', json.dumps(limits)], capture_output=True)
                self.assertNotEqual(run.returncode, 0)
                receipt = json.loads((root/'out/failure.json').read_text())
                self.assertIn(reason, receipt['reason'])
                self.assertIn('resources', receipt)

    def test_ownership_unknown_callers_are_not_lost(self):
        tables = replay.Tables(replay.Budget())
        for text in (b'nav::world::NavWorld::load_pack', b'/frozen/crates/nav/src/world.rs'):
            tables.definition('s', text)
        tables.definition('i', [1, 0])
        tables.definition('t', [1, 0])
        tables.definition('i', [2, 0, 1, 2, 50])
        tables.definition('t', [2, 1])
        result = replay.classify(tables, 2, True)
        self.assertEqual(result['ownership'], 'process_shared_or_transient')
        self.assertTrue(result['unknown_symbol'])
        self.assertEqual(replay.classify(tables, 2, False)['ownership'], 'unknown')

    def test_real_owned_cpu_guard(self):
        import heaptrack_owner_runner as runner
        code = ('import sys; sys.path.insert(0, '+repr(str(MODULE.parent.resolve()))+'); '
                'import heaptrack_owner_runner as r; import heaptrack_owner_replay as p; '
                'g=r.PhaseGuard(p.Budget({"cpu":1}).limits); g.next(1)\n'
                'try:\n while True: g.check()\n'
                'except p.Invalid as e:\n print(r.json.dumps({"error":str(e)}),flush=True); sys.exit(1)\n')
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(replay.Invalid, 'CPU guard'):
                runner.supervise([sys.executable, '-c', code], Path(directory), replay.LIMITS, False)

    def test_manifest_and_receipt_rejections(self):
        import heaptrack_owner_runner as runner
        with tempfile.TemporaryDirectory() as directory:
            path, digest = smoke_manifest(Path(directory))
            manifest = runner.load_manifest(path, digest, True)
            self.assertEqual(runner.verify_receipt(manifest), (6388, 34, 637))
            with self.assertRaisesRegex(replay.Invalid, 'hash'):
                runner.load_manifest(path, '0'*64, True)
            for key, value in [('intentional_pause', True), ('reinitialized', True), ('collection_mode', 'attach')]:
                changed = json.loads(path.read_text())
                changed['provenance'][key] = value
                data = json.dumps(changed).encode()
                path.write_bytes(data)
                with self.assertRaises(replay.Invalid):
                    runner.load_manifest(path, hashlib.sha256(data).hexdigest(), True)
            with self.assertRaises(replay.Invalid):
                runner.unique_json(b'{"a":1,"a":2}')
            manifest['inputs']['stderr']['sha256'] = '0'*64
            with self.assertRaises(replay.Invalid):
                runner.verify_receipt(manifest)

    @unittest.skipUnless(sys.platform == 'linux', 'Linux RLIMIT_AS and /proc native qualification required')
    def test_linux_native_memory_guard_small_child(self):
        import heaptrack_owner_runner as runner
        # 64 MiB pressure, NOT production data or a game. Root runs after staging.
        with tempfile.TemporaryDirectory() as directory:
            code = 'import time; x=bytearray(64*1024*1024); time.sleep(3)'
            with self.assertRaisesRegex(replay.Invalid, 'RSS guard'):
                runner.supervise([sys.executable, '-c', code], Path(directory),
                                 replay.Budget({'rss': 32*replay.MIB}).limits, True)

    @unittest.skipUnless(sys.platform == 'linux', 'Linux address-space limit requires native qualification')
    def test_linux_native_as_is_bytes_in_owned_child(self):
        import heaptrack_owner_runner as runner
        with tempfile.TemporaryDirectory() as directory:
            code = ('import resource; resource.setrlimit(resource.RLIMIT_AS,(64*1024*1024,64*1024*1024)); '
                    'x=bytearray(96*1024*1024)')
            with self.assertRaises(replay.Invalid) as caught:
                runner.supervise([sys.executable, '-c', code], Path(directory), replay.LIMITS, True)
            self.assertTrue(getattr(caught.exception, 'replay_resources')['owned_child_reaped'])


if __name__ == '__main__':
    unittest.main()
