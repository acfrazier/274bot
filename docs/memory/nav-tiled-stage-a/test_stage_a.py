"""Synthetic-only tests. --run adds admitted frozen binary integration tests."""
import copy
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

import stage_a as s

class Unit(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory(prefix='test-',dir=s.HERE);self.path=Path(self.tmp.name)
    def tearDown(self):self.tmp.cleanup()
    def bounded(self,code,**kw):
        limits=dict(wall=3,cpu=2,rss=256*1024**2,address=1024**3,output=8192);limits.update(kw)
        return s.support.bounded([sys.executable,'-c',code],self.path,'test',**limits)
    def test_selectors_json_tsv_and_refusals(self):
        row=[100,200,0,102,202,1,15,6,4,1]
        for suffix,data in [('.json',json.dumps([row])),('.tsv',' '.join(map(str,row)))]:
            p=self.path/('routes'+suffix);p.write_text(data);self.assertEqual(s.selectors(p),[row])
        invalid=[[],[row[:-1]],[row+[1]],[[*row[:7],7,4,1]],[[*row[:8],5,1]],[[*row[:9],2]],[[40000,*row[1:]]],[[*row[:2],4,*row[3:]]],[[*row[:6],16,*row[7:]]],[[True,*row[1:]]],{'routes':[row]}]
        for data in invalid:
            p=self.path/'bad.json';p.write_text(json.dumps(data))
            with self.assertRaises(ValueError):s.selectors(p)
        p=self.path/'bad.tsv';p.write_text('100 nope')
        with self.assertRaises(ValueError):s.selectors(p)
    def test_input_hash_symlink_and_size(self):
        p=self.path/'input';p.write_bytes(b'synthetic');digest=s.sha(p)
        s.admit(p,digest,9)
        for h,cap in [('0'*64,9),(digest,8)]:
            with self.assertRaises(ValueError):s.admit(p,h,cap)
        link=self.path/'link';link.symlink_to(p)
        with self.assertRaises(ValueError):s.admit(link,digest,9)
        with self.assertRaises(ValueError):s.owned(link/'output')
        with self.assertRaises(ValueError):s.owned(Path('/tmp/not-owned'))
    def test_wire_allocation_bomb(self):
        s.support.safe_wire(s.wire.pack(),65536)
        with self.assertRaises(ValueError):s.support.safe_wire(s.wire.header(16000,16000),65536)
        # Truncation remains original decoder's job, not a fabricated oracle.
        s.support.safe_wire(b'274V',65536)

    def test_coordinate_prepare_requires_exact_external_binding(self):
        source=s.HERE/'coordinate-source-binding.json';digest=s.sha(source)
        run=self.path/'coordinate-run'
        try:
            s.prepare(run,source,digest)
            prepared=json.loads((run/'prepared.json').read_text())
            self.assertEqual(prepared['candidate_id'],'coordinate-ebf0f30')
            self.assertEqual(set(prepared['arms']),{'dense','refined'})
            with self.assertRaises(ValueError):s.activate_run_binding(run)
            context=s.activate_run_binding(run,source,digest)
            self.assertEqual(context['arms']['refined'],'8385babb23fd15b876506d4a3f6154984a6b2df1')
            effective=json.loads((run/'refined/effective-source-manifest.json').read_text())
            self.assertEqual([e['path'] for e in effective['files'] if e['provenance']=='overlay'],
                ['crates/nav/src/collision.rs'])
            s.verify_original(run/'refined','refined')
            with self.assertRaises(ValueError):s.activate_run_binding(run,source,'0'*64)
        finally:
            s.configure_source_binding()

    def test_coordinate_admission_cannot_use_legacy_schema(self):
        source=s.HERE/'coordinate-source-binding.json';digest=s.sha(source)
        run=self.path/'coordinate-admission'
        try:
            s.prepare(run,source,digest)
            with self.assertRaises(ValueError):s.build(run,'clean')
            s.activate_run_binding(run,source,digest)
            binary=run/'refined/target-clean/release/stage-a-probe';binary.parent.mkdir(parents=True)
            binary.write_bytes(b'generated stage executable')
            s.save_arm_admission(run,'refined','clean',binary,{'generated':True},['generated'])
            self.assertEqual(s.verify_arm(run,'refined','clean'),binary)
            admission=run/'refined-clean-admission.json';value=json.loads(admission.read_text())
            self.assertEqual((value['candidate_id'],value['phase'],value['source_binding']['sha256']),
                ('coordinate-ebf0f30','tooling-admission',digest))
            value['schema']='stage-a-admission-v1';s.save(admission,value)
            with self.assertRaises(ValueError):s.verify_arm(run,'refined','clean')
        finally:
            s.configure_source_binding()

    def test_coordinate_external_binding_mutation_rejected_after_activation(self):
        source=self.path/'external-coordinate-binding.json'
        source.write_bytes((s.HERE/'coordinate-source-binding.json').read_bytes())
        digest=s.sha(source);run=self.path/'coordinate-mutation'
        try:
            s.prepare(run,source,digest)
            s.activate_run_binding(run,source,digest)
            source.write_bytes(source.read_bytes()+b'\n')
            with self.assertRaises(ValueError):s.tools()
        finally:
            s.configure_source_binding()

    def test_normal_current_vs_peak(self):
        r=self.bounded('import time; x=bytearray(8*1024**2); time.sleep(.15); del x; time.sleep(.15); print(7)')
        self.assertIsNone(r['failure']);self.assertGreater(r['sampled_group_peak_bytes'],0)
        self.assertEqual(r['address_guard_active'],platform.system()=='Linux')
        self.assertNotIn('current_rss_bytes',r)
        self.assertEqual((self.path/'test.out').read_text(),'7\n')
    def test_nonzero(self):
        self.assertEqual(self.bounded('raise SystemExit(7)')['failure'],'exit')
    def test_enforced_limits_without_platform_skips(self):
        code='import resource,json;print(json.dumps({"cpu":resource.getrlimit(resource.RLIMIT_CPU),"as":resource.getrlimit(resource.RLIMIT_AS),"core":resource.getrlimit(resource.RLIMIT_CORE),"file":resource.getrlimit(resource.RLIMIT_FSIZE)}))'
        r=self.bounded(code)
        self.assertIsNone(r['failure']);v=json.loads((self.path/'test.out').read_text())
        self.assertEqual(v['cpu'],[2,2]);self.assertEqual(v['core'],[0,0]);self.assertEqual(v['file'],[8192,8192])
        if platform.system()=='Linux':self.assertEqual(v['as'],[1024**3,1024**3])
        else:self.assertFalse(r['address_guard_active'])
    def test_address_enforcement_or_explicit_unqualified(self):
        if platform.system()=='Linux':
            r=self.bounded('import mmap; mmap.mmap(-1,2*1024**3)')
            self.assertEqual(r['failure'],'exit')
        else:
            r=self.bounded('print("Darwin: native hard-AS unqualified")')
            self.assertIsNone(r['failure']);self.assertFalse(r['address_guard_active'])
    def test_cpu_limit(self):
        r=self.bounded('while True: pass',cpu=1,wall=4)
        self.assertEqual(r['failure'],'exit');self.assertLess(r['returncode'],0)
    def test_sampled_group_rss_limit(self):
        r=self.bounded('import time;x=bytearray(64*1024**2);time.sleep(2)',rss=32*1024**2)
        self.assertEqual(r['failure'],'rss')
    def test_timeout_eof_before_leader_exit(self):
        r=self.bounded('import os,time;os.close(1);os.close(2);time.sleep(10)',wall=.2)
        self.assertEqual(r['failure'],'wall');self.assertLess(r['wall_seconds'],2)
    def test_eof_does_not_finish_leader(self):
        r=self.bounded('import os,time;os.close(1);os.close(2);time.sleep(.25)')
        self.assertIsNone(r['failure']);self.assertGreaterEqual(r['wall_seconds'],.25)
    def test_output_limit(self):
        r=self.bounded('import os;os.write(1,b"x"*100000)',output=100)
        self.assertEqual(r['failure'],'output');self.assertEqual(r['output_bytes'],100)
    def test_leader_exit_kills_pipe_holding_descendant(self):
        code='import os,time; p=os.fork(); print(p if p else os.getpid(),flush=True); time.sleep(10) if p==0 else None'
        r=self.bounded(code,wall=1)
        self.assertIsNone(r['failure']);self.assertLess(r['wall_seconds'],1)
        for pid in map(int,(self.path/'test.out').read_text().split()):
            ps=subprocess.run(['ps','-o','stat=','-p',str(pid)],capture_output=True,text=True)
            self.assertTrue(not ps.stdout.strip() or ps.stdout.strip().startswith('Z'))
    def test_existing_output_refused_before_spawn(self):
        (self.path/'test.receipt.json').write_text('{}')
        with self.assertRaises(FileExistsError):self.bounded('raise SystemExit(0)')
    def test_real_gate_before_path_access(self):
        for auth in ({},{'released':True,'mode':'real','input_path':'/not-to-open'}):
            with self.assertRaises(ValueError):s.check_release(self.path,auth)
    def test_released_copies_remain_bound_between_launches(self):
        # Stub prerequisites/child only: exercise real copy, normalization, hash
        # admission, schedule and failure receipts without a real pack/process.
        for standin in (True,False):
            for target in ('input.bin','routes.tsv'):
                for timing in ('after_process','before_next_process','completion'):
                    with self.subTest(standin=standin,target=target,timing=timing):
                        run=self.path/f'{standin}-{target}-{timing}';run.mkdir()
                        s.fixtures(run)
                        pack=run/'fixtures/gated.bin';route=run/'fixtures/routes.json'
                        auth=dict(input_path=str(pack),input_sha256=s.sha(pack),input_bytes=pack.stat().st_size,
                                  routes_path=str(route),routes_sha256=s.sha(route))
                        dest=run/('standin-release' if standin else 'real-release')
                        launches=[];checks=[]
                        def mutate():
                            p=dest/target;p.write_bytes(p.read_bytes()+b'\n')
                        def check(*args):
                            checks.append(1)
                            if timing=='before_next_process' and len(checks)==3:mutate()
                        def launch(*args,**kwargs):
                            launches.append(args[1])
                            if timing=='after_process' and len(launches)==1:mutate()
                            return [dict(phase=p,process_peak_rss_bytes=100,elapsed_ns=100,cpu_ns=100)
                                    for p in s.PHASES]+[dict(aggregate={'calls':1},route_p99_ns=100)]
                        original_paired=s.paired
                        def paired(*args):
                            if timing=='completion' and args[2]==2_000_000:mutate()
                            return original_paired(*args)
                        with patch.object(s,'check_release',check),patch.object(s,'run_one',launch),patch.object(s,'paired',paired):
                            with self.assertRaisesRegex(ValueError,'sha256 mismatch|length'):
                                s.released_run(run,auth,standin=standin)
                        expected=len(json.loads((s.HERE/'proposed-manifest.json').read_text())['order']) if timing=='completion' else 1
                        self.assertEqual(len(launches),expected)
                        receipt=json.loads((dest/'result.json').read_text())
                        self.assertFalse(receipt['qualified'])
                        self.assertIn('failure',receipt)

    def test_coordinate_generated_release_uses_refined_arm_and_bound_namespace(self):
        source=self.path/'external-coordinate-binding.json'
        source.write_bytes((s.HERE/'coordinate-source-binding.json').read_bytes());digest=s.sha(source)
        binding=s.source_binding.load(source,digest);ref=s.source_binding.reference(source,digest,binding)
        run=self.path/'coordinate-release';run.mkdir();(run/'fixtures').mkdir()
        s.save(run/'prepared.json',dict(arms=dict(s.source_binding.arms(binding)),
            candidate_id=binding['candidate_id'],source_binding=ref))
        pack=run/'fixtures/input.bin';pack.write_bytes(b'generated')
        routes=run/'fixtures/routes.tsv';routes.write_text('0 0 0 0 0 0 0 0 0 0\n')
        auth=dict(input_path=str(pack),input_sha256=s.sha(pack),input_bytes=pack.stat().st_size,
            routes_path=str(routes),routes_sha256=s.sha(routes))
        launches=[]
        def launch(run,arm,variant,pack,routes,out,name,released=False):
            launches.append(arm)
            phases=[dict(phase=p,process_peak_rss_bytes=100,elapsed_ns=1,cpu_ns=1)
                for p in s.PHASES]
            return phases+[dict(aggregate={'calls':24},route_p99_ns=1)]
        try:
            with patch.object(s,'check_release'),patch.object(s,'run_one',launch):
                s.released_run(run,auth,standin=True,binding_path=source,binding_sha256=digest)
            self.assertEqual(set(launches),{'dense','refined'})
            dest=run/'coordinate-ebf0f30-standin-release'
            result=json.loads((dest/'result.json').read_text())
            self.assertEqual((result['candidate_id'],result['source_binding']),('coordinate-ebf0f30',ref))
        finally:
            s.configure_source_binding()

    def test_baseline_repeat_noise_not_delta_noise(self):
        r=s.paired([100,200,100],[101,201,101],.1,True)
        self.assertEqual(r['classification'],'inconclusive');self.assertEqual(r['baseline_repeat_range'],1)
        self.assertEqual(s.paired([100]*4,[101]*4,.1,True)['classification'],'pass')
        self.assertEqual(s.paired([100]*4,[111]*4,.1,True)['classification'],'fail')
        self.assertEqual(s.paired([100]*4,[110]*4,.1,True)['classification'],'pass')
        self.assertEqual(s.paired([100,101,100],[110,111,110],.1,True)['classification'],'inconclusive')
        self.assertEqual(s.paired([10,20,30,40],[12,22,32,42],50)['baseline_median'],25)
        for b,c in [([0]*3,[1]*3),([1],[1]),([1,2,3],[1,2]),([1,2,float('nan')],[1,2,3])]:
            with self.assertRaises(ValueError):s.paired(b,c,.1,True)


def integration(run,binding_path=None,binding_sha256=None):
    run=s.owned(run);s.activate_run_binding(run,binding_path,binding_sha256)
    proof=run/'integration-tests';proof.mkdir()
    def reject_mutation(path, action):
        data=path.read_bytes()
        try:
            path.write_bytes(data+b'\nmutation')
            try: action()
            except (ValueError,json.JSONDecodeError):pass
            else:raise AssertionError('mutation admitted: '+str(path))
        finally:path.write_bytes(data)
    for arm in s.ARMS:
        action=lambda:s.verify_arm(run,arm,'clean')
        s.verify_arm(run,arm,'clean')
        for path in [run/arm/'crates/nav/src/collision.rs',run/arm/'Cargo.lock',run/arm/'target-clean/release/stage-a-probe',s.HERE/'allocator.rs']:
            reject_mutation(path,action)
        s.verify_arm(run,arm,'clean')
        binary=run/arm/'target-clean/release/stage-a-probe'
        for variant in ('clean','counting'):
            b=s.verify_arm(run,arm,variant)
            r=s.support.bounded([str(b),'--self-test'],proof,arm+'-'+variant+'-self-test',**s.LIMITS)
            assert not r['failure'],r
        pack=proof/'malformed.bin'
        for i,data in enumerate([b'',b'274V',s.wire.header(0,1),s.wire.pack()[:30]]):
            pack.write_bytes(data)
            r=s.support.bounded([str(binary),str(pack),str(run/'fixtures/routes.tsv'),'generated'],proof,f'{arm}-malformed-{i}',**s.LIMITS)
            assert r['failure']=='exit',r
        pack.write_bytes(s.wire.pack())
        route=proof/'bad.tsv';route.write_text('100 200 0 102 202 0 0 0 5 0\n')
        r=s.support.bounded([str(binary),str(pack),str(route),'generated'],proof,arm+'-bad-selector',**s.LIMITS)
        assert r['failure']=='exit',r
    original=json.loads((run/'qualification-clean/dense-all-uniform.result.json').read_text())['data']
    bad=copy.deepcopy(original);bad[2],bad[3]=bad[3],bad[2]
    path=proof/'bad-output';path.write_text(''.join(json.dumps(r)+'\n' for r in bad))
    try:s.output(path,False,len(s.selectors(run/'fixtures/routes.tsv')))
    except ValueError:pass
    else:raise AssertionError('phase reordering accepted')
    # Full release protocol with exclusively generated input, including JSON normalization.
    pack=run/'fixtures/gated.bin';route=run/'fixtures/routes.json'
    auth=dict(released=True,mode='standin',limits=s.LIMITS,tools=s.tools(),platform=platform.platform(),hardware=s.hardware(),order=json.loads((s.HERE/'proposed-manifest.json').read_text())['order'],correctness_review_released=True,
              manifest_sha256=s.sha(s.HERE/'proposed-manifest.json'),input_path=str(pack),input_sha256=s.sha(pack),input_bytes=pack.stat().st_size,routes_path=str(route),routes_sha256=s.sha(route))
    if s.SOURCE_BINDING is not None:
        auth.update(schema='stage-a-coordinate-release-v1',candidate_id=s.SOURCE_BINDING['candidate_id'],
            source_binding=s.SOURCE_BINDING_REF,phase='tooling-release')
    for variant in ('clean','counting'):
        auth[variant+'_qualification_sha256']=s.sha(run/f'qualification-{variant}/result.json')
        for arm in s.ARMS:auth[f'{arm}-{variant}-admission_sha256']=s.sha(run/f'{arm}-{variant}-admission.json')
    for field,value in [('input_sha256','0'*64),('tools',{}),('limits',dict(s.LIMITS,wall=121)),('dense-clean-admission_sha256','0'*64),('clean_qualification_sha256','0'*64),('correctness_review_released',False)]:
        bad=dict(auth);bad[field]=value
        try:s.released_run(run,bad,standin=True,binding_path=binding_path,binding_sha256=binding_sha256)
        except ValueError:pass
        else:raise AssertionError('release mutation admitted: '+field)
    s.released_run(run,auth,standin=True,binding_path=binding_path,binding_sha256=binding_sha256)
    s.save(proof/'result.json',dict(qualified=True,scope='synthetic only',checks=['both-arm source/lock/binary/tool rejection','both-arm self-test','both-arm malformed decode and selector failures','phase ordering rejection','hash-bound release JSON stand-in and mutations'],tools=s.tools()))

if __name__=='__main__':
    if '--run' in sys.argv:
        parser=argparse.ArgumentParser();parser.add_argument('--run',type=Path,required=True)
        parser.add_argument('--source-binding',type=Path);parser.add_argument('--source-binding-sha256')
        args=parser.parse_args()
        if bool(args.source_binding)!=bool(args.source_binding_sha256):
            parser.error('source binding path and external SHA256 are a pair')
        integration(args.run,args.source_binding,args.source_binding_sha256)
    else:unittest.main()
