"""Adversarial admission/stream tests; no production inputs or executable runs."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
import harness as h

class Admission(unittest.TestCase):
    def source_fixture(self, root):
        root.mkdir(exist_ok=True)
        p=root/'crates/nav/src';p.mkdir(parents=True)
        original=b'pub fn original() {}\n'
        (p/'collision.rs').write_bytes(original)
        blob=hashlib.sha1(b'blob '+str(len(original)).encode()+b'\0'+original).hexdigest()
        h.save(root/'original-source-manifest.json',dict(files=[dict(path='crates/nav/src/collision.rs',size=len(original),sha256=hashlib.sha256(original).hexdigest(),git_blob=blob)]))
        (root/'original-host-lib.rs').write_bytes(b'')
        h.save(root/'host-spans.json',dict(source_sha256=hashlib.sha256(b'').hexdigest(),spans=[]))
        (p/'host-probe.rs').write_bytes(b'\n')
        return p/'collision.rs'

    def test_frozen_collision_mutation_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d);p=self.source_fixture(root);h.verify_original_sources(root)
            p.write_bytes(b'pub fn changed_() {}\n')
            with self.assertRaises(ValueError):h.verify_original_sources(root)

    def test_extra_original_source_suffix_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d);p=self.source_fixture(root);p.write_bytes(p.read_bytes()+b'// changed\n')
            with self.assertRaises(ValueError):h.verify_original_sources(root)

    def test_frozen_host_helper_mutation_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d);self.source_fixture(root)
            (root/'crates/nav/src/host-probe.rs').write_bytes(b'not the original\n')
            with self.assertRaises(ValueError):h.verify_original_sources(root)

    def test_executable_mutation_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d);dest=root/'dense';self.source_fixture(dest)
            binary=dest/'target/debug/differential-probe';binary.parent.mkdir(parents=True);binary.write_bytes(b'original')
            h.save(root/'dense-admission.json',dict(source=h.fingerprint_tree(dest),executable_sha256=h.sha(binary)))
            h.verify_arm(root,'dense');binary.write_bytes(b'changed')
            with self.assertRaises(ValueError):h.verify_arm(root,'dense')

    def test_selector_malformed_and_bounds(self):
        for row in ('1 2','100 200 4 102 202 3 0 0 0 0','0 0 0 0 0 0 16 0 0 0','0 0 0 0 0 0 0 7 0 0','0 0 0 0 0 0 0 -1 0 0','0 0 0 0 0 0 0 0 0 2'):
            with self.subTest(row=row),self.assertRaises(ValueError):h.fixed_selectors(row)
        self.assertEqual(h.fixed_selectors('100 200 0 102 202 3 15 3 4 1'),1)

    def test_named_extension_presets_admitted(self):
        for preset in (4,5,6):
            with self.subTest(preset=preset):
                self.assertEqual(h.fixed_selectors(f'100 200 0 102 202 3 1 {preset} 4 1'),1)

    def test_output_truncation_missing_completion_count(self):
        with tempfile.TemporaryDirectory() as d:
            p=Path(d)/'out'
            for b in (b'',b'cells 2\nx',b'complete-input-count 1\n2\n',b'complete-input-count 1\n1\ncomplete-input-count 1\n1\n'):
                p.write_bytes(b)
                with self.subTest(b=b),self.assertRaises(ValueError):h.output_inventory(p,1)
            p.write_bytes(b'cells 2\nx\n\ncomplete-input-count 1\n1\n')
            self.assertEqual(h.output_inventory(p,1),dict(cells=1,**{'complete-input-count':1}))

if __name__=='__main__':unittest.main()
