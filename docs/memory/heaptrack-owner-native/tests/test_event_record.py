"""Discriminating last-event lifetime checks across both interpreted passes."""
import unittest
from test_differential import fixture, pair, native, reference
from pathlib import Path


class EventRecord(unittest.TestCase):
    def test_descriptor_definition_schema_matches_reference_when_empty(self):
        for raw_tail, interpreted_tail in (
            (b'c 0\n', b'c 0\n'),
            (b'c 0\nS suppression\n', b'c 0\nS suppression\n'),
        ):
            with self.subTest(raw_tail=raw_tail, interpreted_tail=interpreted_tail), fixture(
                raw_tail,
                interpreted_tail,
                b'f; 0\n',
            ) as entries:
                result = pair(entries, 0, outputs=True)
                assert result is not None
                self.assertNotIn('a', result['first']['definitions'])

    def test_unused_descriptor_definition_count_is_retained(self):
        with fixture(b'c 0\n', b'a a 1\nc 0\n', b'f; 0\n') as entries:
            result = pair(entries, 0, outputs=True)
            assert result is not None
            self.assertEqual(result['first']['definitions']['a'], 1)

    def test_descriptor_free_without_timestamp_mark_rejects(self):
        for tail in (b'', b'S suppression\n'):
            with self.subTest(tail=tail), fixture(tail, tail, b'f; 0\n') as entries:
                self.assertIsNone(pair(entries, 0, outputs=True))
                request = dict(op='analyze', requested_time=0, outputs=True)
                request.update({k: Path(v['path']).read_bytes().hex() for k, v in entries.items()})
                with self.assertRaisesRegex(reference.Invalid, 'no actual timestamp mark'):
                    reference.analyze(entries, requested_time=0)
                with self.assertRaisesRegex(reference.Invalid, 'no actual timestamp mark'):
                    native(request)

    def test_absent_event_is_null(self):
        with fixture(b'c 0\nc a\n', b'a a 1\nc 0\nc a\n', b'f; 0\n') as entries:
            result = pair(entries, 0, outputs=True)
            assert result is not None
            self.assertIsNone(result['first']['last_event'])
            self.assertEqual(result['first']['peak']['line'], 0)

    def test_last_event_survives_marks_and_metadata(self):
        for tail in (b'', b'c b\n', b'c b\n# tail\nS suppression\n'):
            with self.subTest(tail=tail), fixture(
                b'c 0\n+ a 1 10\nc a\n- 10\n'+tail,
                b'a a 1\nc 0\n+ 0\nc a\n- 0\n'+tail) as entries:
                result = pair(entries, 0, outputs=True)
                assert result is not None
                data = Path(entries['interpreted']['path']).read_bytes()
                prefix = data[:data.index(b'- 0\n')+4]
                self.assertEqual(result['first']['last_event'], dict(
                    line=prefix.count(b'\n'), offset=data.index(b'- 0\n'), previous_timestamp_ms=10))
                self.assertEqual(result['snapshots']['eof']['count'], 0)

    def test_malformed_tail_after_last_event_rejects(self):
        with fixture(b'+ a 1 10\nc a\n', b'a a 1\n+ 0\nc a\n# missing newline') as entries:
            request = dict(op='analyze', requested_time=None, outputs=True)
            request.update({k:Path(v['path']).read_bytes().hex() for k,v in entries.items()})
            with self.assertRaisesRegex(reference.Invalid, 'truncated record'):
                native(request)


if __name__ == '__main__':
    unittest.main()
