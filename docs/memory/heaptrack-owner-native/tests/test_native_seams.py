"""Native lexical/table seams against original public contract tests."""
from unittest.mock import patch
import unittest
from test_differential import native, reference
import test_heaptrack_owner_replay as original

PARSE, CANONICAL, CLASSIFY, PRETTY = reference.parse_record, reference.canonical, reference.classify, reference.pretty


def table(t, trace, frozen=False):
    definitions = []
    for i in range(1, len(t.strings)//2):
        s = t.string(i)
        definitions.append((b's '+format(len(s), 'x').encode()+b' '+s+b'\n').hex())
    for i in range(1, len(t.ip_offsets)//2):
        definitions.append(('i '+' '.join(format(n, 'x') for n in t.ip(i))+'\n').encode().hex())
    for i in range(1, len(t.traces)//2):
        definitions.append(('t '+' '.join(format(n, 'x') for n in t.traces[i*2:i*2+2])+'\n').encode().hex())
    return native(dict(op='table', definitions=definitions, trace=trace, frozen=frozen, limits={'depth':t.b.limits['depth']}))


def parse(line, raw=False, line_limit=reference.MIB):
    request = dict(op='parse', line=line.hex(), raw=raw, limits={'line':line_limit})
    try:
        expected = PARSE(line, raw, line_limit)
    except reference.Invalid:
        with unittest.TestCase().assertRaises(reference.Invalid): native(request)
        raise
    actual = native(request)
    op, fields = expected
    unittest.TestCase().assertEqual(op, actual['op'])
    unittest.TestCase().assertEqual(list(fields), actual['bytes'] if isinstance(fields, bytes) else actual['nums'])
    return expected


def canonical(t, trace):
    result = CANONICAL(t, trace)
    unittest.TestCase().assertEqual(result, bytes(table(t, trace)['canonical']))
    return result


def classify(t, trace, frozen):
    result = CLASSIFY(t, trace, frozen)
    unittest.TestCase().assertEqual(result, table(t, trace, frozen)['classification'])
    return result


def pretty(text):
    result = PRETTY(text)
    unittest.TestCase().assertEqual(result, bytes(native(dict(op='pretty', text=text.hex()))))
    return result


class NativeSeams(unittest.TestCase):
    def test_all_record_adversaries(self):
        with patch.object(reference, 'parse_record', parse):
            original.ReplayTests().test_record_adversaries()
        for line in (b's 4 a \xc3\xa9\n', b'i 1 0 0\n'):
            parse(line)

    def test_original_normalization(self):
        with patch.object(reference, 'canonical', canonical), patch.object(reference, 'pretty', pretty):
            original.ReplayTests().test_normalization_collisions_new_stop_inline_unresolved()

    def test_original_ownership(self):
        with patch.object(reference, 'classify', classify):
            original.ReplayTests().test_ownership_unknown_callers_are_not_lost()

    def test_zero_and_depth(self):
        t = reference.Tables(reference.Budget({'depth':1}))
        t.definition('t', [0, 0])
        self.assertEqual(bytes(table(t, 0)['canonical']), CANONICAL(t, 0))
        self.assertEqual(bytes(table(t, 1)['canonical']), CANONICAL(t, 1))
        t.definition('t', [0, 1])
        with self.assertRaises(reference.Invalid): table(t, 2)


if __name__ == '__main__':
    unittest.main()
