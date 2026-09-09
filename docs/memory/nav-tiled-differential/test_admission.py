"""Adversarial admission/stream tests; no production inputs or executable runs."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
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

    def test_coordinate_binding_is_externally_hashed_and_strict(self):
        source=h.HERE.parent/'nav-tiled-stage-a/coordinate-source-binding.json'
        digest=h.sha(source)
        binding=h.load_source_binding(source,digest)
        self.assertEqual(binding['candidate_id'],'coordinate-ebf0f30')
        reference=h.source_binding.reference(source,digest,binding)
        self.assertNotIn('path',reference)
        self.assertEqual(reference['sha256'],digest)
        self.assertEqual(h.materialization_arms(binding),[
            ('dense','29b7aea779322c8611f83dc193e939ca7d756f75'),
            ('refined','8385babb23fd15b876506d4a3f6154984a6b2df1')])
        with self.assertRaises(ValueError):h.load_source_binding(source,None)
        with self.assertRaises(ValueError):h.load_source_binding(source,'0'*64)
        for mutation in ('unknown','second-overlay','relabel','whole-ebf-base'):
            with self.subTest(mutation=mutation),tempfile.TemporaryDirectory() as d:
                changed=json.loads(source.read_text())
                if mutation=='unknown':changed['unexpected']=True
                elif mutation=='second-overlay':changed['overlays'].append(dict(changed['overlays'][0]))
                elif mutation=='relabel':changed['candidate_id']='tiled-8385'
                else:changed['refined_base']='ebf0f30f0422229ea75de39db06944091d6497a5'
                path=Path(d)/'binding.json';h.save(path,changed)
                with self.assertRaises(ValueError):h.load_source_binding(path,h.sha(path))

    def test_coordinate_overlay_replaces_only_admitted_collision_bytes(self):
        source=h.HERE.parent/'nav-tiled-stage-a/coordinate-source-binding.json'
        binding=h.load_source_binding(source,h.sha(source));overlay=binding['overlays'][0]
        with tempfile.TemporaryDirectory() as d:
            dest=Path(d);path=dest/overlay['path'];path.parent.mkdir(parents=True)
            old=h.git(h.ROOT,'cat-file','blob',overlay['old_git_blob'])
            new=h.git(h.ROOT,'cat-file','blob',overlay['new_git_blob'])
            entry=dict(path=overlay['path'],size=len(old),sha256=hashlib.sha256(old).hexdigest(),
                       git_blob=hashlib.sha1(b'blob '+str(len(old)).encode()+b'\0'+old).hexdigest())
            path.write_bytes(old)
            with patch.object(h,'git',return_value=new):
                effective=h.apply_source_overlays(dest,[entry],binding,'refined')
            self.assertEqual(path.read_bytes(),new)
            self.assertEqual(len(effective),1)
            self.assertEqual(effective[0]['provenance'],'overlay')
            (dest/'crates/nav/src/extra.rs').write_text('extra')
            with self.assertRaises(ValueError):
                h.verify_effective_sources(dest,[entry],binding,'refined')

    def test_coordinate_materialization_binds_complete_base_plus_one_overlay(self):
        source=h.HERE.parent/'nav-tiled-stage-a/coordinate-source-binding.json'
        binding=h.load_source_binding(source,h.sha(source))
        with tempfile.TemporaryDirectory(dir=h.HERE) as d:
            run=Path(d);h.materialize(run,binding,source,h.sha(source))
            sources=json.loads((run/'sources.json').read_text())
            self.assertEqual(set(sources),{'dense','refined'})
            self.assertEqual(sources['refined']['base_commit'],binding['refined_base'])
            self.assertEqual(sources['refined']['source_binding']['candidate_id'],binding['candidate_id'])
            manifest=json.loads((run/'refined/effective-source-manifest.json').read_text())
            changed=[e for e in manifest['files'] if e['provenance']=='overlay']
            self.assertEqual([(e['path'],e['git_blob']) for e in changed],
                [('crates/nav/src/collision.rs',binding['overlays'][0]['new_git_blob'])])
            h.verify_original_sources(run/'refined',binding,'refined')
            collision=run/'refined/crates/nav/src/collision.rs'
            collision.write_bytes(collision.read_bytes()+b'// mutation\n')
            with self.assertRaises(ValueError):
                h.verify_original_sources(run/'refined',binding,'refined')

    def test_coordinate_run_requires_binding_hash_and_rejects_legacy_admission(self):
        source=h.HERE.parent/'nav-tiled-stage-a/coordinate-source-binding.json';digest=h.sha(source)
        binding=h.load_source_binding(source,digest)
        with tempfile.TemporaryDirectory(dir=h.HERE) as d:
            run=Path(d);h.materialize(run,binding,source,digest)
            with self.assertRaises(ValueError):h.source_context(run)
            context=h.source_context(run,source,digest)
            self.assertEqual(context['binding'],binding)
            binary=run/'refined/target/debug/differential-probe';binary.parent.mkdir(parents=True)
            binary.write_bytes(b'generated executable')
            h.save_arm_admission(run,'refined',context,binary,{'generated':True})
            h.verify_arm(run,'refined',context)
            admission=run/'refined-admission.json';value=json.loads(admission.read_text())
            value['schema']='nav-differential-admission-v1';h.save(admission,value)
            with self.assertRaises(ValueError):h.verify_arm(run,'refined',context)

    def test_coordinate_external_binding_mutation_rejected_between_admissions(self):
        original=h.HERE.parent/'nav-tiled-stage-a/coordinate-source-binding.json'
        with tempfile.TemporaryDirectory(dir=h.HERE) as d:
            run=Path(d);source=run.parent/(run.name+'-binding.json')
            try:
                source.write_bytes(original.read_bytes());digest=h.sha(source)
                binding=h.load_source_binding(source,digest);h.materialize(run,binding,source,digest)
                context=h.source_context(run,source,digest)
                for arm in context['arms']:
                    binary=run/arm/'target/debug/differential-probe';binary.parent.mkdir(parents=True)
                    binary.write_bytes(b'generated executable')
                    h.save_arm_admission(run,arm,context,binary,{'generated':True})
                h.verify_arm(run,'dense',context)
                source.write_bytes(source.read_bytes()+b'\n')
                with self.assertRaises(ValueError):h.verify_arm(run,'refined',context)
            finally:
                source.unlink(missing_ok=True)

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
