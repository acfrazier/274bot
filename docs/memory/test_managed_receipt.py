import json
import pathlib
import tempfile
import unittest
from unittest import mock

import managed_receipt as mr
import cache_provenance as cp


class ManagedReceiptTests(unittest.TestCase):
    def add_cache_launch(self):
        cache, unpack = self.root/'cache', self.root/'unpack'
        cache.mkdir()
        unpack.mkdir()
        for name in cp.JAGS:
            (cache/name).write_text(name)
        snapshot = unpack/cp.file_sha256(cache/'versionlist')[:16]
        snapshot.mkdir()
        for name in cp.SNAPSHOTS:
            (snapshot/name).write_text(name)
        artifact = self.root/'cache.json'
        artifact.write_text(json.dumps(cp.capture(cache, unpack)))
        # Separate launch artifact; no retroactive amendment of the first one.
        self.launch_path = self.root/'cache-launch.json'
        self.utc.return_value = '2026-09-07T04:00:00Z'
        self.launch = mr.create_launch(
            self.launch_path, cell_id='cache_cell', index=2, kind='matched',
            effective_cli=self.argv, binary=self.binary,
            manifest_path=self.root/'manifest.json',
            server_identity_path=self.root/'server_identity.json',
            host_conditions_path=self.root/'host_conditions.json',
            sampler_config={'interval_s':1}, cache_provenance_path=artifact)
        self.utc.return_value = '2026-09-07T04:01:01Z'
        return cache, artifact

    def test_cache_binding_verified_on_both_sides_of_launch(self):
        self.add_cache_launch()
        result = self.finish()
        self.assertEqual(result['status'], 'completed', result['binding_errors'])
        self.assertIn('cache_verified_before_launch_utc', result)
        self.assertIn('cache_verified_after_completion_utc', result)

    def test_changed_cache_file_or_snapshot_fails_completion(self):
        cache, artifact = self.add_cache_launch()
        (cache/'config').write_text('changed bytes')
        result = self.finish()
        self.assertIn('cache_provenance_changed_or_invalid', result['binding_errors'])
        self.assertNotIn('cache_verified_after_completion_utc', result)

    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = pathlib.Path(temp.name)
        self.binary = self.root / 'binary'
        self.binary.write_bytes(b'frozen')
        for label in ('manifest', 'server_identity', 'host_conditions'):
            (self.root / (label + '.json')).write_text(json.dumps({'test_fixture': label}))
        self.launch_path = self.root / 'launch.json'
        self.argv = ['python3', 'launcher.py', 'tui', '1', 'active']
        clock = mock.patch.object(mr, 'utc_now', return_value='2026-09-07T04:00:00Z')
        self.utc = clock.start()
        self.addCleanup(clock.stop)
        self.launch = mr.create_launch(
            self.launch_path, cell_id='test_cell', index=1, kind='matched',
            effective_cli=self.argv, binary=self.binary,
            manifest_path=self.root / 'manifest.json',
            server_identity_path=self.root / 'server_identity.json',
            host_conditions_path=self.root / 'host_conditions.json',
            sampler_config={'interval_s': 1, 'duration_s_requested': 60})
        self.run = self.root / 'run'
        self.run.mkdir()
        self.meta = {'run_dir': str(self.run), 'exit_code': 0,
                     'binary': str(self.binary), 'binary_sha256': mr.file_sha256(self.binary),
                     'effective_cli': self.argv, 'started_unix': mr._timestamp('2026-09-07T04:00:01Z'),
                     'ended_unix': mr._timestamp('2026-09-07T04:01:00Z')}
        self.utc.return_value = '2026-09-07T04:01:01Z'
        self.write_meta()
        for name in ('samples.jsonl', 'samples.qualification.jsonl'):
            (self.run / name).write_text('{}\n')
        self.sampler = self.root / 'process_accounting.jsonl'
        self.sampler.write_text('{}\n')
        self.receipt_path = self.root / 'receipt.json'

    def write_meta(self):
        (self.run / 'metadata.json').write_text(json.dumps(self.meta))

    def finish(self, run_dir='default', launcher_exit=0, sampler_exit=0):
        return mr.complete(self.receipt_path, launch_path=self.launch_path,
                           run_dir=self.run if run_dir == 'default' else run_dir,
                           launcher_exit_code=launcher_exit,
                           sampler_result={'exit_code': sampler_exit, 'output': str(self.sampler)})

    def test_receipt_hashes_actual_files_without_metric_acceptance(self):
        result = self.finish()
        self.assertEqual(result['status'], 'completed', result['binding_errors'])
        self.assertFalse(result['performance_acceptance'])
        self.assertNotIn('qualified', result)
        for name, digest in result['raw_hashes'].items():
            self.assertEqual(digest, mr.file_sha256(self.run / name))
        self.assertEqual(result['launch_sha256'], mr.file_sha256(self.launch_path))
        self.assertEqual(result['sampler_output_sha256'], mr.file_sha256(self.sampler))

    def test_runner_failure_keeps_true_child_exit_codes(self):
        result = mr.complete(self.receipt_path, launch_path=self.launch_path,
                             run_dir=self.run, launcher_exit_code=0,
                             sampler_result={'exit_code': 0, 'output': str(self.sampler)},
                             runner_errors=['missing_observation_boundary'])
        self.assertEqual(result['status'], 'failed_or_unavailable')
        self.assertEqual(result['launcher_exit_code'], 0)
        self.assertEqual(result['sampler_result']['exit_code'], 0)
        self.assertIn('runner:missing_observation_boundary', result['binding_errors'])

    def test_foreign_artifact_changed_since_launch_is_failure(self):
        self.binary.write_bytes(b'changed')
        result = self.finish()
        self.assertIn('binary_changed_since_launch', result['binding_errors'])
        self.assertEqual(result['status'], 'failed_or_unavailable')

    def test_failed_launcher_preserves_frontend_exit(self):
        result = self.finish(launcher_exit=1)
        self.assertEqual(result['exit_code'], 0)
        self.assertEqual(result['launcher_exit_code'], 1)
        self.assertIn('launcher_failed', result['binding_errors'])

    def test_missing_run_and_missing_raw_still_written(self):
        result = self.finish(run_dir=self.root / 'never_created')
        self.assertIsNone(result['run_dir'])
        self.assertTrue(self.receipt_path.is_file())
        self.receipt_path.unlink()
        (self.run / 'samples.jsonl').unlink()
        result = self.finish()
        self.assertIn('missing_raw_file:samples.jsonl', result['binding_errors'])

    def test_bad_metadata_time_and_cli_preserved_as_errors(self):
        self.meta.update(started_unix=0, effective_cli=['different'])
        self.write_meta()
        result = self.finish()
        self.assertIn('metadata_time_envelope_invalid', result['binding_errors'])
        self.assertIn('metadata_launcher_argv_mismatch', result['binding_errors'])

    def test_no_overwrites(self):
        self.finish()
        original = self.receipt_path.read_bytes()
        with self.assertRaises(FileExistsError):
            self.finish()
        self.assertEqual(self.receipt_path.read_bytes(), original)
        with self.assertRaises(FileExistsError):
            mr.write_new_json(self.launch_path, {})

    def test_sampler_failure_and_missing_output_reject(self):
        self.sampler.unlink()
        result = self.finish(sampler_exit=1)
        self.assertIn('sampler_failed_or_incomplete', result['binding_errors'])
        self.assertIn('sampler_output_missing', result['binding_errors'])


if __name__ == '__main__':
    unittest.main()
