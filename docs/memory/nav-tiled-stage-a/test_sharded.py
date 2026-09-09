"""Generated-only scheduler contract tests; no external inputs."""
import importlib.util
from pathlib import Path
import unittest
import sharded as sh
import qualify_sharded as qualify
import tempfile
import time
import sys
import subprocess
import platform
import json
import copy
from unittest.mock import patch
from contextlib import contextmanager


@contextmanager
def retained_fixture():
    path=Path(tempfile.mkdtemp(prefix='sharded-test-',dir=sh.stage.HERE))
    print('retained fixture:',path,flush=True)
    yield path


@contextmanager
def matrix_fixture_doubles(run):
    """Remove redundant fixture durability/history scans, not guard logic.

    The full 826-slot test still runs the real scheduler, authorization state
    machine, output binding, aggregation and budget/deadline code.  Focused
    tests below keep the real atomic writer, reference admission and storage
    guard coverage; this double is only for the repeated arithmetic matrix.
    """
    original_read_ref=sh.read_ref
    reads={}
    storage_calls=[]

    def cached_read_ref(ref,cap=256*1024):
        key=(ref['path'],ref['sha256'],cap)
        if key not in reads:
            reads[key]=original_read_ref(ref,cap)
        return reads[key]

    def buffered_write_json(path,value,cap=256*1024):
        data=(json.dumps(value,sort_keys=True,separators=(',',':'),allow_nan=False)+'\n').encode()
        if len(data)>cap:
            raise ValueError(f'bounded JSON output exceeded: {len(data)} > {cap}')
        path.write_bytes(data)

    def bounded_storage_guard(dest,reserve=0):
        storage_calls.append((Path(dest),reserve))
        if not Path(dest).is_dir() or reserve<0:
            raise ValueError('invalid generated release destination')
        return 0

    with patch.object(sh,'read_ref',cached_read_ref),\
            patch.object(sh,'write_json',buffered_write_json),\
            patch.object(sh,'storage_guard',bounded_storage_guard):
        yield storage_calls

    # Keep these checks outside the patch so a future edit cannot silently
    # remove the explicit use of this double from the full-matrix fixture.
    if not storage_calls:
        raise AssertionError('matrix fixture did not exercise storage calls')


class Metrics(unittest.TestCase):
    def coordinate_root(self,generated):
        root=dict(generated)
        root.update(sh.identity('CF1'),**sh.coordinate_ledger_identity(),
            campaign_budget=sh.coordinate_budget(sh.ACCEPTED_DECISION_SHA256,copy.deepcopy(sh.PRIOR_F1_LEDGER)))
        return root

    def test_full_generated_protocol(self):
        self.assertTrue(hasattr(sh,'run_phase'), 'phase driver missing')
        with retained_fixture() as d:
            run=Path(d);(run/'fixtures').mkdir()
            pack=run/'fixtures/tiny.bin';pack.write_bytes(sh.stage.wire.pack(1,1))
            routes=run/'fixtures/59.tsv'
            routes.write_text(''.join(f'{i} 0 0 {i} 0 0 0 0 0 0\n' for i in range(59)))
            root=sh.generated_authorization(run,pack,routes,run/'release')
            auth=run/'F1.json';sh.write_json(auth,root)
            launches=[]
            def launch(run,arm,pack,selector,out,name,standin):
                launches.append((arm,int(selector.stem)))
                return mock_output(out,name)
            with patch.object(sh.stage,'check_release'),patch.object(sh,'launch',launch),\
                    matrix_fixture_doubles(run):
                f1=sh.run_phase(run,auth,sh.stage.sha(auth),standin=True)
                self.assertEqual(len(launches),4)
                for phase in ('F2','acceptance'):
                    parent=f1 if phase=='F2' else f2
                    next_auth=dict(schema=sh.SCHEMA,phase=phase,released=True,mode='standin',
                        root=sh.reference(auth),parent=sh.reference(parent),review_approved=True,method_accepted=True)
                    path=run/(phase+'.json');sh.write_json(path,next_auth)
                    result=sh.run_phase(run,path,sh.stage.sha(path),standin=True)
                    if phase=='F2':f2=result
                self.assertEqual(len(launches),826)
                self.assertEqual(launches,[(a,r) for phase in ('F1','F2','acceptance') for j,r,a in sh.schedule(phase)])
                result=json.loads(result.read_text())
                self.assertEqual(result['scope'],'generated stand-in only')
                self.assertEqual(result['completed'],708)
                self.assertEqual(len(list((run/'release').rglob('input.bin'))),1)
                with self.assertRaises(FileExistsError):sh.run_phase(run,auth,sh.stage.sha(auth),standin=True)

    def test_coordinate_cf1_carries_binding_and_prior_ledger(self):
        with retained_fixture() as d:
            run=Path(d);(run/'fixtures').mkdir()
            source=sh.stage.HERE/'coordinate-source-binding.json';source_hash=sh.stage.sha(source)
            binding=sh.stage.source_binding.load(source,source_hash)
            ref=sh.stage.source_binding.reference(source,source_hash,binding)
            sh.stage.save(run/'prepared.json',dict(arms=dict(sh.stage.source_binding.arms(binding)),
                candidate_id=binding['candidate_id'],source_binding=ref))
            pack=run/'fixtures/tiny.bin';pack.write_bytes(sh.stage.wire.pack(1,1))
            routes=run/'fixtures/59.tsv';routes.write_text('0 0 0 0 0 0 0 0 0 0\n'*59)
            generated=sh.generated_authorization(run,pack,routes,run/'coordinate-ebf0f30-release-01',
                source,source_hash)
            self.assertEqual((generated['schema'],generated['phase']),
                (sh.COORDINATE_QUALIFICATION_SCHEMA,'GQ'))
            self.assertNotIn('prior_campaign_ledger',generated)
            root=self.coordinate_root(generated)
            self.assertEqual((root['schema'],root['phase']),(sh.COORDINATE_SCHEMA,'CF1'))
            self.assertEqual(root['campaign_budget']['prior'],sh.PRIOR_F1_LEDGER)
            for field,value in [('schema',sh.SCHEMA),('child_decision_sha256','0'*64),
                    ('prior_campaign_ledger',dict(sh.PRIOR_F1_LEDGER,children=0))]:
                bad=run/('bad-'+field+'.json');sh.write_json(bad,dict(root,**{field:value}))
                with self.assertRaises(ValueError):sh.run_phase(run,bad,sh.stage.sha(bad),standin=True,
                    source_binding_path=source,source_binding_sha256=source_hash)
            auth=run/'CF1.json';sh.write_json(auth,root)
            with patch.object(sh.stage,'check_release'),patch.object(sh,'launch',lambda r,a,p,s,o,n,g:mock_output(o,n)):
                result=sh.run_phase(run,auth,sh.stage.sha(auth),standin=True,
                    source_binding_path=source,source_binding_sha256=source_hash)
            value=json.loads(result.read_text())
            self.assertEqual((value['schema'],value['phase'],value['candidate_id']),
                (sh.COORDINATE_SCHEMA,'CF1','coordinate-ebf0f30'))
            self.assertEqual(value['prior_campaign_ledger'],sh.PRIOR_F1_LEDGER)
            self.assertEqual(value['budget']['children'],8)
            self.assertEqual(value['completed'],4)
            records=[json.loads((result.parent/(e['name']+'.record.json')).read_text()) for e in value['entries']]
            self.assertTrue(all(e['candidate_id']=='coordinate-ebf0f30' and e['phase']=='CF1'
                and e['source_binding']==ref for e in records))
            sh.stage.configure_source_binding()

    def test_coordinate_generated_qualification_has_separate_phase_and_budget(self):
        with retained_fixture() as d:
            run=Path(d);(run/'fixtures').mkdir();source=sh.stage.HERE/'coordinate-source-binding.json'
            source_hash=sh.stage.sha(source);binding=sh.stage.source_binding.load(source,source_hash)
            ref=sh.stage.source_binding.reference(source,source_hash,binding)
            sh.stage.save(run/'prepared.json',dict(tools=sh.stage.tools(),arms=dict(sh.stage.source_binding.arms(binding)),
                candidate_id=binding['candidate_id'],source_binding=ref))
            pack=run/'fixtures/tiny.bin';pack.write_bytes(sh.stage.wire.pack(1,1))
            routes=run/'fixtures/59.tsv';routes.write_text('0 0 0 0 0 0 0 0 0 0\n'*59)
            auth=sh.generated_authorization(run,pack,routes,run/'coordinate-ebf0f30-generated-smoke-01',
                source,source_hash)
            path=run/'GQ.json';sh.write_json(path,auth)
            try:
                with patch.object(sh.stage,'check_release'),patch.object(sh,'launch',
                        lambda run,arm,pack,selector,out,name,standin:mock_output(out,name)):
                    result=sh.run_phase(run,path,sh.stage.sha(path),standin=True,
                        source_binding_path=source,source_binding_sha256=source_hash)
                value=json.loads(result.read_text())
                self.assertEqual((value['schema'],value['phase'],value['completed']),
                    (sh.COORDINATE_QUALIFICATION_SCHEMA,'GQ',4))
                self.assertEqual(value['budget']['children'],4)
                self.assertNotIn('prior_campaign_ledger',value)
            finally:
                sh.stage.configure_source_binding()

    def test_full_coordinate_generated_protocol(self):
        with retained_fixture() as d:
            run=Path(d);(run/'fixtures').mkdir()
            source=sh.stage.HERE/'coordinate-source-binding.json';source_hash=sh.stage.sha(source)
            binding=sh.stage.source_binding.load(source,source_hash)
            ref=sh.stage.source_binding.reference(source,source_hash,binding)
            sh.stage.save(run/'prepared.json',dict(arms=dict(sh.stage.source_binding.arms(binding)),
                candidate_id=binding['candidate_id'],source_binding=ref))
            pack=run/'fixtures/tiny.bin';pack.write_bytes(sh.stage.wire.pack(1,1))
            routes=run/'fixtures/59.tsv';routes.write_text('0 0 0 0 0 0 0 0 0 0\n'*59)
            root=self.coordinate_root(sh.generated_authorization(
                run,pack,routes,run/'coordinate-ebf0f30-full-01',source,source_hash))
            root_path=run/'CF1.json';sh.write_json(root_path,root);launches=[]
            def launch(run,arm,pack,selector,out,name,standin):
                launches.append((arm,int(selector.stem)));return mock_output(out,name)
            try:
                with patch.object(sh.stage,'check_release'),patch.object(sh,'launch',launch),matrix_fixture_doubles(run):
                    result=sh.run_phase(run,root_path,sh.stage.sha(root_path),standin=True,
                        source_binding_path=source,source_binding_sha256=source_hash)
                    for phase in ('CF2','CA'):
                        auth=dict(sh.identity(phase),**sh.coordinate_ledger_identity(),released=True,mode='standin',
                            root=sh.reference(root_path),parent=sh.reference(result),
                            review_approved=True,campaign_budget=root['campaign_budget'])
                        if phase=='CA':auth['method_accepted']=True
                        path=run/(phase+'.json');sh.write_json(path,auth)
                        result=sh.run_phase(run,path,sh.stage.sha(path),standin=True,
                            source_binding_path=source,source_binding_sha256=source_hash)
                self.assertEqual(launches,[(a,r) for phase in ('CF1','CF2','CA')
                    for _,r,a in sh.schedule(phase)])
                final=json.loads(result.read_text())
                self.assertEqual((final['completed'],final['budget']['children']),(708,830))
                self.assertEqual(final['source_binding'],ref)
                self.assertLessEqual(result.stat().st_size,sh.STORAGE['json_bytes'])
            finally:
                sh.stage.configure_source_binding()

    def test_budget_reservation_refund_and_deadline(self):
        self.assertTrue(hasattr(sh,'Budget'), 'global budget missing')
        clock=[0.0];cpu=[0.0]
        budget=sh.Budget(480,360,clock=lambda:clock[0],cpu=lambda:cpu[0])
        budget.reserve()
        self.assertEqual(budget.snapshot()['reserved_cpu'],90)
        clock[0]=2;cpu[0]=1
        budget.finish(True)
        self.assertEqual(budget.snapshot()['cpu'],1)
        self.assertEqual(budget.snapshot()['wall'],2)
        budget.reserve();clock[0]=3;cpu[0]=2
        budget.finish(False)
        self.assertEqual(budget.snapshot()['cpu'],92)
        with self.assertRaises(ValueError): budget.reserve()
        too_small=sh.Budget(119,90,clock=lambda:0,cpu=lambda:0)
        with self.assertRaises(ValueError):too_small.reserve()
        # An alarm interrupts the SHARED bounded runner, whose finally cleans
        # its group even when a forked descendant retains both output pipes.
        with retained_fixture() as d:
            p=Path(d);start=time.monotonic()
            with self.assertRaises(sh.Deadline), sh.deadline(.25):
                sh.stage.support.bounded([sys.executable,'-c',
                    'import os,time;print(os.getpid(),flush=True);p=os.fork();print(os.getpid(),flush=True);time.sleep(20)'],p,'alarm',wall=20,cpu=2,address=sh.QUAL_LIMITS['address'])
            self.assertLess(time.monotonic()-start,3)
            self.assertEqual(json.loads((p/'alarm.receipt.json').read_text())['failure'],'supervisor')
            for pid in (p/'alarm.out').read_text().split():
                result=subprocess.run(['ps','-o','stat=','-p',pid],capture_output=True,text=True)
                self.assertTrue(not result.stdout.strip() or result.stdout.strip().startswith('Z'))

    def test_scheduler_fixture_address_binds_to_qualification_cap(self):
        # The qualification runner inherits this cap on Linux; the fixture must
        # never ask frozen_support to raise that inherited hard limit.
        with retained_fixture() as d:
            p=Path(d)
            r=sh.stage.support.bounded([sys.executable,'-c',
                'import json,resource;print(json.dumps(resource.getrlimit(resource.RLIMIT_AS)))'],
                p,'address-binding',address=sh.QUAL_LIMITS['address'])
            self.assertIsNone(r['failure'])
            self.assertEqual(r['limits']['address'],sh.QUAL_LIMITS['address'])
            if platform.system()=='Linux':
                self.assertTrue(r['address_guard_active'])
                self.assertEqual(json.loads((p/'address-binding.out').read_text()),
                    [sh.QUAL_LIMITS['address']]*2)
            else:
                # macOS does not provide the Linux hard-RLIMIT_AS proof.
                self.assertFalse(r['address_guard_active'])

    def test_schedule_and_raw_contract(self):
        self.assertTrue(hasattr(sh, 'schedule'), 'schedule missing')
        self.assertEqual(sh.schedule('F1'), [(1,1,'dense'),(1,1,'tiled'),(1,2,'tiled'),(1,2,'dense')])
        self.assertEqual(len(sh.schedule('F2')), 114)
        order=sh.schedule('acceptance')
        self.assertEqual(len(order),708)
        for j in range(1,7):
            expected=[(j,r,a) for r in range(1,60) for a in (('dense','tiled') if j%2 else ('tiled','dense'))]
            self.assertEqual(order[(j-1)*118:j*118],expected)
        summary=dict(raw_schema=sh.RAW_SCHEMA,raw_order=sh.RAW_ORDER,raw_elapsed_ns=list(range(24)),route_p99_ns=23,lane_cpu_ns=[1,2,3],aggregate={'calls':24})
        self.assertEqual(sh.raw_samples(summary,1),list(range(24)))
        for field,value in [('raw_order','lane-row-sweep'),('raw_schema','old'),('raw_elapsed_ns',[1]*23),('route_p99_ns',1)]:
            with self.subTest(field=field), self.assertRaises(ValueError):
                sh.raw_samples(dict(summary,**{field:value}),1)

    def test_coordinate_schedule_and_finite_budget_decision(self):
        self.assertEqual(sh.SCHEMA,'stage-a-singleton-v2')
        self.assertEqual(sh.COORDINATE_SCHEMA,'stage-a-coordinate-v1')
        self.assertEqual(sh.schedule('CF1'),[(1,1,'dense'),(1,1,'refined'),
            (1,2,'refined'),(1,2,'dense')])
        self.assertEqual(len(sh.schedule('CF2')),114)
        self.assertEqual(len(sh.schedule('CA')),708)
        self.assertEqual({row for _,row,_ in sh.schedule('CF1')+sh.schedule('CF2')},set(range(1,60)))
        policy=sh.coordinate_budget(sh.ACCEPTED_DECISION_SHA256,copy.deepcopy(sh.PRIOR_F1_LEDGER))
        self.assertEqual(policy['cumulative_child_ceiling'],122)
        self.assertEqual(policy['fresh_child_ceiling'],118)
        self.assertEqual(policy['prior']['wall'],40.090448230999755)
        self.assertEqual(policy['prior']['cpu'],29.50134)
        self.assertEqual(policy['prior']['children'],4)
        with self.assertRaises(ValueError):sh.coordinate_budget(None,copy.deepcopy(sh.PRIOR_F1_LEDGER))
        with self.assertRaises(ValueError):sh.coordinate_budget('0'*64,copy.deepcopy(sh.PRIOR_F1_LEDGER))
        for key in ('wall','cpu','children','result_sha256'):
            changed=copy.deepcopy(sh.PRIOR_F1_LEDGER)
            changed[key]=changed[key]+1 if key!='result_sha256' else '0'*64
            with self.subTest(key=key),self.assertRaises(ValueError):
                sh.coordinate_budget(sh.ACCEPTED_DECISION_SHA256,changed)
        old_cap=sh.Budget(1800,1500,copy.deepcopy(sh.PRIOR_F1_LEDGER),
            clock=lambda:0,cpu=lambda:0,child_limit=118)
        for _ in range(114):old_cap.reserve();old_cap.finish(True)
        with self.assertRaises(ValueError):old_cap.reserve()
        approved=sh.Budget(1800,1500,copy.deepcopy(sh.PRIOR_F1_LEDGER),
            clock=lambda:0,cpu=lambda:0,child_limit=122)
        for _ in range(118):approved.reserve();approved.finish(True)
        self.assertEqual(approved.snapshot()['children'],122)

    def test_coordinate_qualification_configuration_is_externally_bound(self):
        with retained_fixture() as d:
            run=Path(d);source=sh.stage.HERE/'coordinate-source-binding.json';digest=sh.stage.sha(source)
            binding=sh.stage.source_binding.load(source,digest)
            sh.stage.save(run/'prepared.json',dict(arms=dict(sh.stage.source_binding.arms(binding)),
                candidate_id=binding['candidate_id'],
                source_binding=sh.stage.source_binding.reference(source,digest,binding)))
            try:
                config=qualify.configuration(run,'coordinate-ebf0f30-qualification-01',source,digest)
                self.assertEqual((config['phase'],config['reference_arm']),('GQ',{'dense':'dense','refined':'tiled'}))
                with self.assertRaises(ValueError):
                    qualify.configuration(run,'coordinate-ebf0f30-qualification-01',source,None)
                with self.assertRaises(ValueError):
                    qualify.configuration(run,'singleton-qualification-04',source,digest)
            finally:
                sh.stage.configure_source_binding()

    def test_exact_quantiles(self):
        spec = importlib.util.spec_from_file_location('sharded', Path(__file__).with_name('sharded.py'))
        self.assertIsNotNone(spec)
        self.assertTrue(Path(spec.origin).exists(), 'single-row scheduler not implemented')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        self.assertEqual(module.p99([1]*1401+[100]*15), 100)
        self.assertEqual(module.p99([1]*467+[100]*5), 100)
        with self.assertRaises(ValueError):
            module.p99([])


def mock_output(out,name):
    data=[dict(phase=p,since_start_ns=i,process_cpu_ns=i,process_peak_rss_bytes=100,current_rss_bytes=100,elapsed_ns=1,cpu_ns=1) for i,p in enumerate(sh.stage.PHASES)]
    data.append(dict(summary=True,diagnostic=False,startup_peak_rss_bytes=100,raw_schema=sh.RAW_SCHEMA,raw_order=sh.RAW_ORDER,
        raw_elapsed_ns=[1]*24,route_p99_ns=1,lane_cpu_ns=[1,1,1],aggregate={'calls':24},input_bytes=len(sh.stage.wire.pack(1,1))))
    (out/(name+'.out')).write_text(''.join(json.dumps(v)+'\n' for v in data))
    (out/(name+'.err')).write_text('')
    receipt=dict(failure=None,returncode=0,wall_seconds=.01,limits=dict(sh.stage.LIMITS,file_size=sh.stage.LIMITS['output']))
    sh.write_json(out/(name+'.receipt.json'),receipt)
    return data,receipt,.001


if __name__ == '__main__':
    unittest.main()
