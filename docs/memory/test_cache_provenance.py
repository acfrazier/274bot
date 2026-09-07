import pathlib
import tempfile
import unittest

import cache_provenance as cp


class CacheProvenanceTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = pathlib.Path(temp.name)
        self.cache = self.root / 'pack/client'
        self.cache.mkdir(parents=True)
        self.unpack = self.root / 'unpack'
        self.unpack.mkdir()
        for name in cp.JAGS:
            (self.cache / name).write_bytes(name.encode())
        self.version = cp.file_sha256(self.cache / 'versionlist')[:16]
        self.snapshot = self.unpack / self.version
        self.snapshot.mkdir()
        for name in cp.SNAPSHOTS:
            (self.snapshot / name).write_bytes(name.encode())
        for name in cp.STORE_FILES:
            (self.cache.parent / name).write_bytes(name.encode())

    def test_actual_inputs_and_explicit_absences(self):
        value = cp.capture(self.cache, self.unpack)
        self.assertEqual(value['snapshot_version'], self.version)
        self.assertEqual(len(value['files']), 20)
        self.assertEqual(sum(row['present'] for row in value['files']), 15)
        self.assertFalse(value['performance_acceptance'])
        cp.recheck(value)
        self.assertEqual(value, cp.capture(self.cache, self.unpack))

    def test_missing_required_snapshot_rejected(self):
        (self.snapshot / 'models.bin').unlink()
        with self.assertRaisesRegex(ValueError, 'required cache input missing'):
            cp.capture(self.cache, self.unpack)

    def test_changed_cache_bytes_rejected(self):
        value = cp.capture(self.cache, self.unpack)
        (self.cache.parent / 'main_file_cache.dat').write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'cache input changed'):
            cp.recheck(value)

    def test_new_direct_store_rejects_changed_resolution(self):
        value = cp.capture(self.cache, self.unpack)
        (self.cache / 'main_file_cache.dat').write_bytes(b'new preferred store')
        with self.assertRaisesRegex(ValueError, 'previously absent'):
            cp.recheck(value)

    def test_base_symlink_change_rejected(self):
        link = self.root / 'selected-cache'
        link.symlink_to(self.cache)
        value = cp.capture(link, self.unpack)
        link.unlink()
        link.symlink_to(self.cache.parent)
        with self.assertRaisesRegex(ValueError, 'directory binding changed'):
            cp.recheck(value)

    def test_content_identity_changes_for_consumed_input(self):
        before = cp.capture(self.cache, self.unpack)
        (self.cache / 'config').write_bytes(b'different config')
        after = cp.capture(self.cache, self.unpack)
        self.assertNotEqual(before['content_identity_sha256'], after['content_identity_sha256'])


if __name__ == '__main__':
    unittest.main()
