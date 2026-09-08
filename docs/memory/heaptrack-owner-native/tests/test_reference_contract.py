"""Contract regressions for review before native parity semantics are changed.

These exercise the unchanged Python reference, NOT a native implementation.
Only generated tiny fixtures are opened; no manifest or input path is accepted.
Run from any directory with python3 -B <this file> -v.
"""
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
import heaptrack_owner_replay as replay


OWNER = b'nav::world::NavWorld::load_pack'
SOURCE = b'/frozen/crates/nav/src/world.rs'


def fixture_bytes(placement):
    strings = b''.join(b's ' + format(len(s), 'x').encode() + b' ' + s + b'\n'
                       for s in (OWNER, SOURCE, b'caller'))
    raw = (b'v 10500 3\nx 1 x\nX fixture\nI 1000 100\n'
           b't 10 0\nt 20 1\n+ a 2 30\nc a\n')
    if placement == 'primary_caller':
        ips = b'i 10 0\ni 20 0 1 2 32\n'
        canonical = b'0x10;nav::world::NavWorld::load_pack (world.rs); 10\n'
    elif placement == 'inline_caller':
        ips = b'i 10 0 3 0 0 0 0 0\ni 20 0 1 2 32\n'
        canonical = b'caller; ();nav::world::NavWorld::load_pack (world.rs); 10\n'
    elif placement == 'inline_owner':
        ips = b'i 10 0 3\ni 20 0 1 2 32 0 0 0\n'
        canonical = b'caller;nav::world::NavWorld::load_pack (world.rs); (); 10\n'
    else:
        raise ValueError('unknown finite fixture case')
    interpreted = (b'v 10500 3\nX fixture\nI 1000 100\n' + strings + ips +
                   b't 1 0\nt 2 1\na a 2\n+ 0\nc a\n')
    return {'raw': raw, 'interpreted': interpreted, 'peak': canonical}


def observe(placement):
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        entries = {}
        for role, data in fixture_bytes(placement).items():
            assert len(data) <= 2_000_000
            path = root / role
            path.write_bytes(data)
            entries[role] = dict(path=str(path), bytes=len(data),
                                 sha256=hashlib.sha256(data).hexdigest())
        result = replay.analyze(entries)
        classification = replay.classify(result['tables'], 2, True)
        output = root / 'output'
        output.mkdir()
        replay.write_outputs(result, output, dict(host=replay.HOST, client=replay.CLIENT))
        receipt = json.loads((output / 'receipt.json').read_text())
        return classification, receipt


class WholeStackUnknownContract(unittest.TestCase):
    def assert_unknown(self, placement):
        classification, receipt = observe(placement)
        self.assertTrue(receipt['canonical_full_positive_multiset_equal'])
        self.assertEqual(classification['domain'], 'nav_world_startup')
        self.assertEqual(classification['ownership'], 'process_shared_or_transient')
        self.assertEqual(receipt['cutoffs']['peak']['bytes'], 10)
        # Test both the classification seam and emitted receipt independently.
        with self.subTest(surface='classification'):
            self.assertTrue(classification['unknown_symbol'])
        with self.subTest(surface='receipt'):
            self.assertEqual(receipt['cutoffs']['peak']['unknown_symbol_bytes'], 10)

    def test_unresolved_primary_caller_control(self):
        self.assert_unknown('primary_caller')

    def test_unresolved_inline_caller_is_not_lost(self):
        self.assert_unknown('inline_caller')

    def test_unresolved_inline_owner_is_not_lost(self):
        self.assert_unknown('inline_owner')


if __name__ == '__main__':
    unittest.main()
