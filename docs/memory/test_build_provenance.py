"""Frozen-build checks against real temporary files; never starts a frontend."""
import copy
import contextlib
import io
import json
import pathlib
import tempfile
import unittest
from unittest import mock

import build_provenance as bp
import run_diagnostic as rd


class BuildProvenanceTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = pathlib.Path(temp.name)
        self.paths = {}
        for key in ('binary', 'nav_pack', 'nav_flags', 'catalog'):
            self.paths[key] = self.root / key
            self.paths[key].write_bytes(key.encode())
        digest = lambda key: bp.file_sha256(self.paths[key])
        self.manifest = {
            'candidate': {'commit': 'a' * 40, 'branch': 'test-branch',
                          'sources_sha256_pre': 'b' * 64, 'sources_sha256_post': 'b' * 64,
                          'sources_stable_across_build': True, 'build_exit': 0,
                          'client': {'commit': 'c' * 40, 'sources_sha256': 'd' * 64}},
            'features': {'requested': 'memory-profile-no-alloc', 'locked': True,
                         'allocator': 'std::alloc::System', 'allocation_counting': False},
            'binaries': {'candidate_tui_play': {'path': str(self.paths['binary']), 'sha256': digest('binary')}},
            'nav': {key + suffix: (str(self.paths[key]) if suffix == '' else digest(key))
                    for key in ('nav_pack', 'nav_flags') for suffix in ('', '_sha256')},
            'catalog': {'js_scripts_json': str(self.paths['catalog']), 'js_scripts_json_sha256': digest('catalog')},
        }
        self.manifest_path = self.root / 'manifest.json'

    def verify(self, manifest=None, binary=None):
        self.manifest_path.write_text(json.dumps(self.manifest if manifest is None else manifest))
        return bp.verify_build(self.manifest_path, 'candidate', 'tui', binary or self.paths['binary'],
                               self.paths['nav_pack'], self.paths['nav_flags'], self.paths['catalog'])

    def test_real_files_bound_without_checkout_source(self):
        result = self.verify()
        self.assertEqual(result['status'], 'verified')
        self.assertEqual(result['build_commit'], 'a' * 40)
        self.assertEqual(result['manifest_sha256'], bp.file_sha256(self.manifest_path))
        self.assertFalse(result['performance_acceptance'])
        self.assertNotIn('checkout_commit', result)
        bp.recheck_files(result['files'])

    def test_same_hash_copy_rejected_symlink_accepted(self):
        other = self.root / 'other'
        other.write_bytes(self.paths['binary'].read_bytes())
        with self.assertRaisesRegex(ValueError, 'path differs'):
            self.verify(binary=other)
        other.unlink()
        other.symlink_to(self.paths['binary'])
        self.assertEqual(self.verify(binary=other)['status'], 'verified')

    def test_corrupt_every_fixture_and_binary(self):
        for key, path in self.paths.items():
            with self.subTest(key=key):
                original = path.read_bytes()
                path.write_bytes(b'corrupt')
                with self.assertRaisesRegex(ValueError, 'hash differs'):
                    self.verify()
                path.write_bytes(original)

    def test_mutation_after_preflight_rejected_including_manifest(self):
        for key in (*self.paths, 'manifest'):
            with self.subTest(key=key):
                result = self.verify()
                path = self.manifest_path if key == 'manifest' else self.paths[key]
                original = path.read_bytes()
                path.write_bytes(original + b'changed')
                with self.assertRaises(ValueError):
                    bp.recheck_files(result['files'])
                path.write_bytes(original)

    def test_missing_invalid_build_and_feature_evidence(self):
        edits = [
            ('candidate', 'sources_stable_across_build', False),
            ('candidate', 'sources_sha256_post', 'e' * 64),
            ('candidate', 'sources_sha256_pre', 'junk'),
            ('candidate', 'commit', 'junk'),
            ('candidate', 'build_exit', False),
            ('candidate', 'build_exit', 1),
            ('features', 'allocation_counting', None),
            ('features', 'allocation_counting', 'false'),
            ('features', 'locked', False),
            ('features', 'allocator', None),
        ]
        for block, key, value in edits:
            with self.subTest(field=key, value=value):
                manifest = copy.deepcopy(self.manifest)
                manifest[block][key] = value
                with self.assertRaises(ValueError):
                    self.verify(manifest)

    def test_wrong_runtime_catalog_path_rejected(self):
        other = self.root / 'alternate_catalog'
        other.write_bytes(self.paths['catalog'].read_bytes())
        self.manifest['catalog']['js_scripts_json'] = str(other)
        with self.assertRaisesRegex(ValueError, 'catalog path differs'):
            self.verify()

    def test_cli_requires_explicit_binary_role_and_manifest(self):
        parser = rd.build_parser()
        for flags in (['--build-role', 'candidate'], ['--build-manifest', 'x'],
                      ['--build-manifest', 'x', '--build-role', 'candidate']):
            with self.subTest(flags=flags), contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                rd.validate_args(parser.parse_args(['tui', '1', 'idle', *flags]), parser)
        args = parser.parse_args(['tui', '1', 'idle', '--binary', 'b',
                                  '--build-manifest', 'm', '--build-role', 'candidate'])
        rd.validate_args(args, parser)

    def test_main_invalid_manifest_stops_before_mkdir_or_child(self):
        with mock.patch.object(rd.pathlib.Path, 'mkdir') as mkdir, \
                mock.patch.object(rd.subprocess, 'Popen') as popen, \
                contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as raised:
            rd.main(['tui', '1', 'idle', '--binary', str(self.paths['binary']),
                     '--build-manifest', str(self.root / 'missing.json'), '--build-role', 'candidate'])
        self.assertEqual(raised.exception.code, 2)
        mkdir.assert_not_called()
        popen.assert_not_called()


if __name__ == '__main__':
    unittest.main()
