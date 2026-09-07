import copy
import datetime
import hashlib
import json
import pathlib
import tempfile
import unittest

import process_accounting as pa
import process_evidence as pe
from test_process_accounting import Clock, _sample


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.path = pathlib.Path(self.tmp.name) / 'process.jsonl'
        self.clock = Clock()
        def sample(pid, timeout):
            self.clock.sleep(0.001)
            return _sample('identity', rss=1234, user=self.clock.now*.2, system=self.clock.now*.1)
        pa.run({'server': 123}, self.path, interval=.5, duration=6,
               include_collector_self=False, sample_fn=sample,
               pressure_fn=lambda: {'status': 'unavailable'},
               monotonic_fn=self.clock.monotonic, sleep_fn=self.clock.sleep,
               utc_fn=lambda: datetime.datetime.fromtimestamp(1000+self.clock.now, datetime.timezone.utc).isoformat().replace('+00:00','Z'))
        self.rows = [json.loads(line) for line in self.path.read_text().splitlines()]
        # Synthetic test fixture uses the real collector schema, with its
        # injected sampler explicitly relabeled only inside this test.
        self.rows[0]['process_backend'] = 'system'
        self.kw = dict(roles={'server': {'pid': 123, 'start_identity': 'identity'}},
                       observation=[1001.2,1004.2], interval_s=.5,
                       duration_mode='fixed', process_backend='system')

    def check(self, rows=None, **kwargs):
        self.path.write_text('\n'.join(json.dumps(r) for r in (rows or self.rows))+'\n')
        digest = hashlib.sha256(self.path.read_bytes()).hexdigest()
        return pe.validate(self.path, digest, **(self.kw | kwargs))

    def test_available_encloses_observation_without_overhead_claim(self):
        result = self.check()
        self.assertEqual(result['status'], 'available', result)
        server = result['roles']['server']
        self.assertEqual(server['resident_median_bytes'], 1234)
        self.assertAlmostEqual(server['cpu_cores_interval'][1], .3, places=3)
        self.assertLess(server['cpu_wall_envelope'][0], 1001.2)
        self.assertGreater(server['cpu_wall_envelope'][1], 1004.2)
        self.assertFalse(result['instrumentation_overhead_measured'])

    def test_bad_artifacts_unavailable(self):
        mutations = [
            lambda r: r[2].update(sample_index=9),
            lambda r: r[2].update(roles={}),
            lambda r: r[2]['roles'].update(server=None),
            lambda r: r[2]['roles']['server'].update(cpu=['invalid']),
            lambda r: r[2]['roles']['server'].update(start_identity='reused'),
            lambda r: r[2]['roles']['server']['cpu'].update(cumulative_user_s=0),
            lambda r: r[-1].update(sample_count=999),
            lambda r: r[-1].update(completion='not_complete'),
            lambda r: r[2].update(acquisition_after_utc='invalid'),
            lambda r: r[2].update(acquisition_before_utc='1970-01-01T01:00:00Z'),
            lambda r: r[2]['roles']['server'].update(resident_bytes=True),
            lambda r: r[0].update(process_backend='injected'),
            lambda r: r[0].update(cadence_tolerance_s=99),
            lambda r: r[2]['roles']['server'].update(acquisition_end_monotonic_s=100),
        ]
        for change in mutations:
            with self.subTest(change=change):
                rows=copy.deepcopy(self.rows)
                change(rows)
                self.assertEqual(self.check(rows)['status'], 'unavailable')

    def test_missing_coverage_and_identity_unavailable(self):
        self.assertEqual(self.check(observation=[999,1004])['status'], 'unavailable')
        self.assertEqual(self.check(observation=[1001,1010])['status'], 'unavailable')
        self.assertEqual(self.check(roles={'server': {'pid':123, 'start_identity':'wrong'}})['status'], 'unavailable')

    def test_controlled_stop_needs_coverage_and_completion(self):
        rows = copy.deepcopy(self.rows)
        rows[0].update(duration_mode='stop_controlled', duration_s=None, required_grid_sample_count=None)
        rows[-1].update(status='closed', completion='controlled_stop', grid_complete=None)
        self.assertEqual(self.check(rows, duration_mode='stop_controlled')['status'], 'available')
        rows[-1]['status']='fail'
        self.assertEqual(self.check(rows, duration_mode='stop_controlled')['status'], 'unavailable')

    def test_hash_mismatch(self):
        self.check()
        self.assertEqual(pe.validate(self.path, '0'*64, **self.kw)['status'], 'unavailable')


if __name__ == '__main__':
    unittest.main()
