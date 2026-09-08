"""Reuse every original analyze fixture with a native comparison before return.

Tests which do not call analyze have explicit native counterparts in the mapping;
this module does not claim their Python-only bodies qualify native behavior.
"""
import json
from pathlib import Path
import unittest
from unittest.mock import patch
from test_differential import native, reference
import test_heaptrack_owner_replay as original

ANALYZE = reference.analyze
COUNTS = {}


def differential_analyze(entries, requested_time=None, limits=None, phase=None):
    request = dict(op='analyze', requested_time=requested_time, limits=limits or {})
    for k, e in entries.items():
        request[k] = Path(e['path']).read_bytes().hex()
    # Preserve bound identity errors rather than silently reauthorize altered bytes.
    import hashlib
    identity_changed = any(len(request[k])//2 != e['bytes'] or hashlib.sha256(bytes.fromhex(request[k])).hexdigest() != e['sha256'] for k, e in entries.items())
    try:
        result = ANALYZE(entries, requested_time=requested_time, limits=limits, phase=phase or (lambda n: None))
    except reference.Invalid:
        if phase is not None or identity_changed:
            # Covered by direct Input frozen-identity tests, not a regenerated
            # adapter fixture with a newly authorized identity.
            raise
        with unittest.TestCase().assertRaises(reference.Invalid):
            native(request)
        COUNTS['rejected'] = COUNTS.get('rejected', 0)+1
        raise
    actual = native(request)
    for key in ('raw', 'first', 'snapshots', 'canonical', 'canonical_equal'):
        unittest.TestCase().assertEqual(json.loads(json.dumps(result[key])), actual[key], key)
    COUNTS['accepted'] = COUNTS.get('accepted', 0)+1
    return result


NAMES = (
    'three_pass_baseline_and_first_peak',
    'saved_smoke_full_canonical_multiset',
    'lossless_output_and_fail_closed_output_budget',
    'synthetic_realloc_multiplicity_zero_and_survivor',
    'time_rejections_and_unbounded_event_tail',
    'reference_and_session_adversaries',
    'conversion_hash_suffix_and_canonical_not_sum',
    'each_table_numeric_input_and_depth_cap',
    'suppression_does_not_change_population',
)


class OriginalFixtures(unittest.TestCase):
    pass


def case(name):
    def run(self):
        test = original.ReplayTests('test_'+name)
        with patch.object(reference, 'analyze', differential_analyze):
            getattr(test, 'test_'+name)()
    return run


for name in NAMES:
    setattr(OriginalFixtures, 'test_'+name, case(name))


if __name__ == '__main__':
    unittest.main()
