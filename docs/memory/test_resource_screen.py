import copy
import json
import pathlib
import tempfile
import unittest
from unittest import mock

import resource_screen as rs
from test_instrumentation_overhead import _quartet, _sha


class ResourceScreenTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = pathlib.Path(self.temp.name)
        self.paths = []
        for i in range(8):
            p = self.root/f'{i}.json'
            p.write_text(json.dumps({'server_identity_path': 'server', 'host_conditions_path': 'host'}))
            self.paths.append(p)
        overhead = _quartet()
        self.sides = []
        for i, role in enumerate(rs.ROLES):
            side = copy.deepcopy(overhead[0])
            side.update(role=role, run_dir=f'/run/{i}', observation_wall_span=[2000000+i*200, 2000120+i*200])
            side['run_identity']['identity_sha256'] = _sha(f'cell-{i}')
            if role == 'reference': side['binary_sha256'] = _sha('reference')
            host = side['analysis']['gates']['resources']
            host.update(cpu_cores=[.2, .203, .202, .201][i],
                        resident_median_bytes=[1000, 900, 905, 1010][i],
                        resident_max_bytes=1100, peak_resident_bytes=1200)
            self.sides.append(side)
        self.map = {str(p): side for p, side in zip(self.paths[:4], self.sides)}
        self.map.update({str(p): side for p, side in zip(self.paths[4:], overhead)})
        self.overhead = dict(resource_deltas_available=True, protocol={'same_binary_sha256': overhead[0]['binary_sha256']})

    def run_screen(self):
        with mock.patch.object(rs.io, 'analyze_instrumentation_overhead', return_value=self.overhead), \
             mock.patch.object(rs.mea, 'bind_side', side_effect=lambda path, **kw: self.map[str(path)]):
            return rs.analyze(self.paths[:4], manifest='manifest', overhead_receipts=self.paths[4:])

    def test_separated_ranges_and_cpu_screen_only(self):
        result = self.run_screen()
        self.assertEqual(result['status'], 'screen_available', result)
        self.assertTrue(result['candidate_resource_screen_supported'])
        self.assertFalse(result['accepted_rss_saving'])
        self.assertFalse(result['final_acceptance'])
        self.assertEqual(result['rss']['minimum_observed_reduction_bytes'], 95)
        self.assertEqual(result['rss']['largest_within_role_spread_bytes'], 10)
        self.assertIn('latency_companion', result['pending'])

    def test_owner_and_noise_are_not_subtracted(self):
        for side in self.sides:
            side['managed_resources']['process']['roles']['controller']['resident_median_bytes'] = 99_000_000
        self.sides[2]['analysis']['gates']['resources']['resident_median_bytes'] = 1005
        result = self.run_screen()
        self.assertFalse(result['candidate_resource_screen_supported'])
        self.assertFalse(result['rss']['reduction_exceeds_observed_variation'])

    def test_rejects_binding_flags_order_and_mutated_settings(self):
        cases = ('binding', 'profile', 'overlap', 'binary', 'native', 'module', 'role', 'host_missing')
        for case in cases:
            with self.subTest(case=case):
                before = copy.deepcopy(self.sides[1])
                side = self.sides[1]
                if case == 'binding': side['binding_ok'] = False
                if case == 'profile': side['match_keys']['scheduling_profile'] = True
                if case == 'overlap': side['observation_wall_span'] = self.sides[0]['observation_wall_span']
                if case == 'binary': side['binary_sha256'] = _sha('changed')
                if case == 'native': side['native_qualification']['match_keys']['qualification_settings']['port'] = 123
                if case == 'module': side['match_keys']['sampler_modules']['process_accounting.py'] = _sha('changed')
                if case == 'role': side['role'] = 'reference'
                if case == 'host_missing': side['analysis']['gates']['resources'].pop('cpu_cores')
                self.assertEqual(self.run_screen()['status'], 'unavailable')
                side.clear()
                side.update(before)

    def test_overhead_label_alone_does_not_authorize_screen(self):
        self.overhead = {'instrumentation_overhead_measured': True}
        self.assertEqual(self.run_screen()['reason'], 'overhead_resource_screen_unavailable')

    def test_duplicate_identity_and_cross_role_binary_change(self):
        self.sides[2]['run_identity'] = self.sides[0]['run_identity']
        self.assertEqual(self.run_screen()['reason'], 'duplicate_resource_run')

    def test_public_api_has_no_binding_callback(self):
        with self.assertRaises(TypeError):
            rs.analyze([], manifest='x', overhead_receipts=[], bind_side=lambda *a: {})

    def test_non_object_receipts_are_unavailable(self):
        self.paths[0].write_text('[]')
        self.assertEqual(self.run_screen()['reason'], 'artifact_validation_failed')
        self.paths[4].write_text('[]')
        self.assertEqual(self.run_screen()['reason'], 'overhead_receipt_must_be_object')

    def test_real_overhead_receipts_cannot_be_relabelled_as_reference(self):
        root = pathlib.Path(__file__).resolve().parent
        batch = root/'diagnostics/instrumentation-overhead-20260907T064437Z'
        if not (batch/'batch.json').is_file():
            self.skipTest('real diagnostic corpus absent')
        specs = [json.loads((batch/(name+'-spec.json')).read_text()) for name in ('off_a','on_a','on_b','off_b')]
        receipts = [batch/'cells'/spec['id']/'receipt.json' for spec in specs]
        result = rs.analyze(receipts, manifest=specs[0]['build_manifest'], overhead_receipts=receipts)
        self.assertEqual(result['reason'], 'resource_side_unavailable', result)
        self.assertFalse(result['accepted_rss_saving'])


if __name__ == '__main__':
    unittest.main()
