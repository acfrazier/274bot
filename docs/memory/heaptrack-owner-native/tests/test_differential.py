"""Bounded generated-fixture differential checks using actual Rust subprocesses."""
import json
from pathlib import Path
import random
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
import heaptrack_owner_replay as reference
from test_heaptrack_owner_replay import fixture, RAW_HEAD, INT_HEAD

EXE = ROOT/'heaptrack-owner-native/target/release/heaptrack-owner-fixture'


def native(value):
    p = subprocess.run([str(EXE)], input=json.dumps(value).encode(), capture_output=True, timeout=60)
    if not p.stdout:
        raise AssertionError(('native abnormal exit', p.returncode, p.stderr))
    result = json.loads(p.stdout)
    if p.returncode:
        raise reference.Invalid(result.get('error', 'native abnormal exit'))
    return result


def pair(entries, requested_time=None, limits=None, outputs=False):
    request = dict(op='analyze', requested_time=requested_time, limits=limits or {}, outputs=outputs)
    for k, v in entries.items():
        request[k] = Path(v['path']).read_bytes().hex()
    try:
        expected = reference.analyze(entries, requested_time=requested_time, limits=limits)
    except reference.Invalid:
        try:
            native(request)
        except reference.Invalid:
            return None
        raise AssertionError('native accepted reference rejection')
    actual = native(request)
    for key in ('raw', 'first', 'snapshots', 'canonical', 'canonical_equal'):
        a = json.loads(json.dumps(expected[key]))
        if a != actual[key]:
            raise AssertionError((key, a, actual[key]))
    if outputs:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            reference.write_outputs(expected, root, dict(host=reference.HOST, client=reference.CLIENT))
            for path in root.iterdir():
                a, b = path.read_text(), actual['files'][path.name]
                if path.suffix == '.json':
                    a, b = json.loads(a), json.loads(b)
                    for v in (a, b):
                        v.pop('resources', None)
                        v['tables'].pop('high_charged_bytes')
                if a != b:
                    raise AssertionError((path.name, a, b))
    return actual


class Differential(unittest.TestCase):
    def test_whole_stack_inline_output(self):
        from test_reference_contract import fixture_bytes
        for placement in ('primary_caller', 'inline_caller', 'inline_owner'):
            data = fixture_bytes(placement)
            with self.subTest(placement=placement), fixture(data['raw'], data['interpreted'], data['peak'], heads=False) as entries:
                pair(entries, outputs=True)
        # Empty nonzero string ID has the same coverage meaning as ID zero.
        data = fixture_bytes('inline_owner')
        data['interpreted'] = data['interpreted'].replace(b'i 10', b's 0 \ni 10').replace(b'32 0 0 0', b'32 4 0 0')
        with fixture(data['raw'], data['interpreted'], data['peak'], heads=False) as entries:
            pair(entries, outputs=True)
        data = fixture_bytes('inline_owner')
        data['interpreted'] = data['interpreted'].replace(b'32 0 0 0', b'32 3 0 0')
        data['peak'] = data['peak'].replace(b'; ();', b';caller ();')
        with fixture(data['raw'], data['interpreted'], data['peak'], heads=False) as entries:
            result = pair(entries, outputs=True)
            self.assertEqual(json.loads(result['files']['receipt.json'])['cutoffs']['peak']['unknown_symbol_bytes'], 0)

    def test_baseline_zero_multiplicity_and_tail(self):
        for count in (1, 2, 37):
            raw = b''.join(f'+ a 1 {i+1:x}\n'.encode() for i in range(count))+b'c a\n- 1\n'
            interp = b'a a 1\n'+b'+ 0\n'*count+b'c a\n- 0\n'
            with fixture(raw, interp, f'f; {10*count}\n'.encode()) as entries:
                pair(entries, 10)
        with fixture(b'+ 0 1 10\nc a\n', b'a 0 1\n+ 0\nc a\n', b'f; 0\n') as entries:
            pair(entries)

    def test_seeded_random_pairs(self):
        for seed in range(24):
            rng = random.Random(seed)
            live = set()
            raw, interp = [], [b'a a 1\n']
            peak = 0
            for i in range(300):
                if live and rng.randrange(3) == 0:
                    pointer = rng.choice(sorted(live)); live.remove(pointer)
                    raw.append(f'- {pointer:x}\n'.encode()); interp.append(b'- 0\n')
                else:
                    pointer = i+1; live.add(pointer)
                    raw.append(f'+ a 1 {pointer:x}\n'.encode()); interp.append(b'+ 0\n')
                peak = max(peak, len(live)*10)
                if i % 13 == 0:
                    mark = f'c {i:x}\n'.encode(); raw.append(mark); interp.append(mark)
            raw.append(b'c 200\n'); interp.append(b'c 200\n')
            with self.subTest(seed=seed), fixture(b''.join(raw), b''.join(interp), f'f; {peak}\n'.encode()) as entries:
                pair(entries, 100)

    def test_chunk_boundaries(self):
        for length in (65530, 65535, 65536, 65537, 131071):
            comment = b'#'+b'x'*length+b'\n'
            with fixture(comment+b'+ a 1 10\nc a\n', comment+b'a a 1\n+ 0\nc a\n') as entries:
                pair(entries)

    def test_chunk_splits_inside_fields_and_utf8(self):
        header = b'v 10500 3\nX fixture\nI 1000 100\n'
        tail = b's 3 f\xc3\xa9\ni 10 0 1\nt 1 0\na a 1\n+ 0\nc a\n'
        for split in range(1, len(tail)):
            padding = 65536-len(header)-split
            interpreted = header+b'#'+b'x'*(padding-2)+b'\n'+tail
            with self.subTest(split=split), fixture(RAW_HEAD+b'+ a 1 10\nc a\n', interpreted, b'f\xc3\xa9; 10\n', heads=False) as entries:
                pair(entries, outputs=True)

    def test_native_grammar(self):
        for raw, lines in [(False, [b's 4 a \xc3\xa9\n', b'i 1 0 0\n', b'c A\n', b'c 1', b'i 1 0 0 0\n', b's 1 \xff\n', b'c 10000000000000000\n', b'c  1\n', b'A\n', b'Q 1\n']), (True, [b'm 1 x 0 1\n', b'm 1 - 0\n', b'x 2 x\n', b'm 1 x ffffffffffffffff 1 1\n'])]:
            for line in lines:
                with self.subTest(line=line):
                    try:
                        op, fields = reference.parse_record(line, raw=raw)
                    except reference.Invalid:
                        with self.assertRaises(reference.Invalid):
                            native(dict(op='parse', line=list(line), raw=raw))
                    else:
                        got = native(dict(op='parse', line=list(line), raw=raw))
                        self.assertEqual(op, got['op'])
                        self.assertEqual(list(fields), got['bytes'] if isinstance(fields, bytes) else got['nums'])

    def test_lower_caps(self):
        for key, cap in [('table_bytes', 1), ('raw', 1), ('interpreted', 1), ('peak', 1), ('line', 8)]:
            with self.subTest(key=key), fixture() as entries:
                pair(entries, limits={key: cap})

    def test_reference_adversaries(self):
        tails = [b'a a 1\n- 0\nc a\n', b'+ 0\nc a\n', b'a a 1\n+ 1\nc a\n', b'a a 2\nc a\n', b't 1 2\nc a\n', b't 2 0\nc a\n', b'i 1 2\nc a\n', b'i 1 0 2\nc a\n', b'i 1 0 1 2 1\nc a\n', b'a a 1\na a 1\nc a\n', b'v 10500 3\nc a\n', b'X other\nc a\n', b'I 1 1\nc a\n', b'A\nc a\n', b'Q\nc a\n']
        for tail in tails:
            with self.subTest(tail=tail), fixture(interpreted=tail) as entries:
                pair(entries)
        for raw in (b'+ a 1 10\n+ a 1 10\nc a\n', b'+ a 2 10\nc a\n', b't 10 2\nc a\n', b'+ a 1 0\nc a\n', b'+ 8000000000000000 1 10\nc a\n'):
            with self.subTest(raw=raw), fixture(raw=raw) as entries:
                pair(entries)
        for request in (-1, True, 0, 9, 11, 1 << 64):
            with self.subTest(request=request), fixture() as entries:
                pair(entries, request)


if __name__ == '__main__':
    unittest.main()
