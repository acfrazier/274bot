import copy
import datetime
import json
import pathlib
import sys
import tempfile
import unittest

import build_provenance as bp
import cache_provenance as cp
import managed_resource_binding as rb
import process_accounting as pa
from test_process_accounting import Clock, _sample


def utc(seconds):
    return datetime.datetime.fromtimestamp(seconds,datetime.timezone.utc).isoformat().replace('+00:00','Z')


class BindingTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        root=pathlib.Path(self.tmp.name).resolve()
        self.root=root
        cache,unpack=root/'cache',root/'unpack'
        cache.mkdir(); unpack.mkdir()
        for name in cp.JAGS:
            (cache/name).write_text(name)
        snapshot=unpack/cp.file_sha256(cache/'versionlist')[:16]
        snapshot.mkdir()
        for name in cp.SNAPSHOTS:
            (snapshot/name).write_text(name)
        cache_value=cp.capture(cache,unpack)
        cache_path=root/'cache.json'
        cache_path.write_text(json.dumps(cache_value))
        self.cache=cache
        roles={'game_server':1,'controller':2,'launcher':3,'collector':4,'ambient':5}
        clock=Clock()
        def sample(pid,timeout):
            clock.sleep(.001)
            return _sample('identity'+str(pid),rss=1000+pid,user=clock.now*.1,system=clock.now*.05)
        output=root/'process.jsonl'
        pa.run(roles,output,interval=1,duration=6,include_collector_self=False,
               sample_fn=sample,pressure_fn=lambda:{'status':'unavailable'},
               monotonic_fn=clock.monotonic,sleep_fn=clock.sleep,utc_fn=lambda:utc(1000+clock.now))
        rows=[json.loads(line) for line in output.read_text().splitlines()]
        rows[0]['process_backend']='system'  # Test-only production-schema fixture.
        output.write_text('\n'.join(json.dumps(r) for r in rows)+'\n')
        modules={}
        for name in ('process_accounting.py','server_resources.py'):
            path=root/name
            path.write_text('fixture module '+name)
            modules[name]={'path':str(path),'sha256':bp.file_sha256(path)}
        self.identities={k:{'pid':v,'start_identity':'identity'+str(v)} for k,v in roles.items()}
        self.server=self.identities['game_server']
        self.receipt={
            'started_utc':utc(999),'ended_utc':utc(1010),
            'cache_verified_before_launch_utc':utc(998),'cache_verified_after_completion_utc':utc(1011),
            'cache_provenance_path':str(cache_path),'cache_provenance_sha256':bp.file_sha256(cache_path),
            'cache_content_identity_sha256':cache_value['content_identity_sha256'],
            'sampler':{'roles':{k:v for k,v in roles.items() if k not in ('launcher','collector')},
                       'process_backend':'system','modules':modules,
                       'module':modules['process_accounting.py']['path'],
                       'module_sha256':modules['process_accounting.py']['sha256'],
                       'interval_s':1,'duration_mode':'fixed'},
            'sampler_output_path':str(output),'sampler_output_sha256':bp.file_sha256(output),
            'sampler_result':{'exit_code':0,'output':str(output),'role_identities':self.identities,
                              'launcher_pid':3,'collector_pid':4},
        }
        self.native={'status':'available','observation_wall_span':[1001.5,1004.5],
                     'observation_wall_span_source':'native_elapsed_wall_bracket',
                     'match_keys':{'qualification_settings':{'cache_dir_canonical_available':True,
                                                            'cache_dir_canonical':str(cache)},
                                   'renderer_config_by_ordinal':[{'ordinal':0,'status':'disabled_profile_off'}]}}

    def test_binds_actual_cache_and_process_without_overhead_claim(self):
        result=rb.bind(self.receipt,self.native,self.server)
        self.assertEqual(result['status'],'available',result)
        self.assertEqual(set(result['process']['roles']),set(self.identities))
        self.assertFalse(result['instrumentation_overhead_measured'])
        self.assertEqual(result['match_keys']['cache_settings']['cache_dir'],str(self.cache))

    def test_missing_or_inconsistent_proofs_unavailable(self):
        changes=[
            lambda r:r.pop('cache_provenance_path'),
            lambda r:r.update(cache_content_identity_sha256='0'*64),
            lambda r:r.update(cache_verified_after_completion_utc=utc(1009)),
            lambda r:r['sampler_result'].update(role_identities={}),
            lambda r:r['sampler_result'].update(collector_pid=6),
            lambda r:r['sampler_result']['role_identities'].update(collector=None),
            lambda r:r['sampler_result'].update(exit_code=1),
            lambda r:r['sampler']['modules'].pop('server_resources.py'),
            lambda r:r['sampler'].update(module_sha256='0'*64),
            lambda r:r['sampler']['roles'].update(game_server=6),
        ]
        for change in changes:
            with self.subTest(change=change):
                receipt=copy.deepcopy(self.receipt)
                change(receipt)
                self.assertEqual(rb.bind(receipt,self.native,self.server)['status'],'unavailable')

    def test_changed_files_and_native_mismatch(self):
        (self.cache/'config').write_text('changed')
        self.assertEqual(rb.bind(self.receipt,self.native,self.server)['status'],'unavailable')
        (self.cache/'config').write_text('config')
        path=self.root/'server_resources.py'
        path.write_text('changed')
        self.assertEqual(rb.bind(self.receipt,self.native,self.server)['status'],'unavailable')

    def test_native_origin_and_server_identity_required(self):
        for native in (self.native|{'status':'unavailable'},
                       self.native|{'observation_wall_span_source':'launcher_plus_elapsed'},
                       self.native|{'observation_wall_span':[999,1004]}):
            self.assertEqual(rb.bind(self.receipt,native,self.server)['status'],'unavailable')
        self.assertEqual(rb.bind(self.receipt,self.native,{'pid':1,'start_identity':'reused'})['status'],'unavailable')

    def _windows_identities(self):
        return {
            name: {'pid': info['pid'], 'start_identity': f'windows_creation_filetime:{info["pid"] * 1000}'}
            for name, info in self.identities.items()
        }

    def _rewrite_sample_identities(self, receipt, identities):
        """Align schema-2 sample start_identity fields with role identities."""
        path = pathlib.Path(receipt['sampler_output_path'])
        rows = [json.loads(line) for line in path.read_text().splitlines()]
        for row in rows:
            if row.get('type') != 'sample':
                continue
            roles = row.get('roles') or {}
            for name, identity in identities.items():
                if name not in roles or not isinstance(roles[name], dict):
                    continue
                roles[name]['start_identity'] = identity['start_identity']
                process = roles[name].get('process')
                if isinstance(process, dict):
                    process['start_identity'] = identity['start_identity']
        path.write_text('\n'.join(json.dumps(r) for r in rows) + '\n')
        receipt['sampler_output_sha256'] = bp.file_sha256(path)
        return receipt

    def _with_windows_modules(self, receipt, *, include_windows=True, tamper=False):
        modules = dict(receipt['sampler']['modules'])
        if include_windows:
            path = self.root / 'windows_process_sample.py'
            path.write_text('fixture module windows_process_sample.py')
            modules['windows_process_sample.py'] = {
                'path': str(path),
                'sha256': bp.file_sha256(path),
            }
            if tamper:
                path.write_text('tampered windows sampler')
        receipt['sampler']['modules'] = modules
        return receipt

    def _windows_receipt(self, *, include_windows=True, tamper=False):
        receipt = copy.deepcopy(self.receipt)
        win_ids = self._windows_identities()
        receipt['sampler_result']['role_identities'] = win_ids
        self._rewrite_sample_identities(receipt, win_ids)
        self._with_windows_modules(receipt, include_windows=include_windows, tamper=tamper)
        return receipt, win_ids['game_server']

    def test_windows_dependency_included_binds_on_any_reader_platform(self):
        """Producer Windows identities require windows_process_sample even on Mac reader."""
        receipt, server = self._windows_receipt(include_windows=True)
        # Reader platform must not gate Windows producer binding — including win32 hosts.
        result = rb.bind(receipt, self.native, server)
        self.assertEqual(result['status'], 'available', result)
        self.assertIn('windows_process_sample.py', result['process']['module_bindings'])
        self.assertIn('windows_process_sample.py', result['match_keys']['sampler_modules'])

    def test_windows_dependency_missing_rejects_unbound_historical_receipt(self):
        receipt, server = self._windows_receipt(include_windows=False)
        # modules still only process_accounting + server_resources (legacy unbound)
        result = rb.bind(receipt, self.native, server)
        self.assertEqual(result['status'], 'unavailable')
        self.assertEqual(result['reason'], 'incomplete sampler dependency bindings')

    def test_windows_dependency_tampered_unavailable(self):
        receipt, server = self._windows_receipt(include_windows=True, tamper=True)
        result = rb.bind(receipt, self.native, server)
        self.assertEqual(result['status'], 'unavailable')
        self.assertIn('hash mismatch', result['reason'])

    def test_non_windows_system_rejects_extra_windows_module(self):
        """Mac/Linux system path must not silently accept an unrelated windows pin."""
        receipt = copy.deepcopy(self.receipt)
        self._with_windows_modules(receipt, include_windows=True)
        result = rb.bind(receipt, self.native, self.server)
        self.assertEqual(result['status'], 'unavailable')
        self.assertEqual(result['reason'], 'incomplete sampler dependency bindings')

    def test_libproc_path_unchanged_without_windows_module(self):
        receipt = copy.deepcopy(self.receipt)
        receipt['sampler']['process_backend'] = 'libproc'
        # Rewrite collector metadata backend to match.
        rows = [json.loads(line) for line in pathlib.Path(receipt['sampler_output_path']).read_text().splitlines()]
        rows[0]['process_backend'] = 'libproc'
        pathlib.Path(receipt['sampler_output_path']).write_text(
            '\n'.join(json.dumps(r) for r in rows) + '\n'
        )
        receipt['sampler_output_sha256'] = bp.file_sha256(pathlib.Path(receipt['sampler_output_path']))
        path = self.root / 'native_process_sample.py'
        path.write_text('fixture module native_process_sample.py')
        receipt['sampler']['modules']['native_process_sample.py'] = {
            'path': str(path),
            'sha256': bp.file_sha256(path),
        }
        result = rb.bind(receipt, self.native, self.server)
        self.assertEqual(result['status'], 'available', result)
        self.assertEqual(
            set(result['process']['module_bindings']),
            {'process_accounting.py', 'server_resources.py', 'native_process_sample.py'},
        )
        self.assertNotIn('windows_process_sample.py', result['process']['module_bindings'])


if __name__=='__main__':
    unittest.main()
