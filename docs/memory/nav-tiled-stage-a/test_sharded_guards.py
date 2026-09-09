"""Adversarial generated-only release tests; retained local fixtures."""
from contextlib import contextmanager
import copy
import json
from pathlib import Path
import tempfile
import unittest
import signal
import os
import time
import sys
import subprocess
from unittest.mock import patch
import sharded as sh
from test_sharded import mock_output


@contextmanager
def retained_fixture():
    path=Path(tempfile.mkdtemp(prefix='sharded-test-',dir=sh.stage.HERE))
    print('retained fixture:',path,flush=True)
    yield path


def setup_root(run):
    (run/'fixtures').mkdir()
    pack=run/'fixtures/tiny.bin';pack.write_bytes(sh.stage.wire.pack(1,1))
    routes=run/'fixtures/59.tsv'
    routes.write_text(''.join(f'{i} 0 0 {i} 0 0 0 0 0 0\n' for i in range(59)))
    root=sh.generated_authorization(run,pack,routes,run/'release')
    auth=run/'F1.json';sh.write_json(auth,root)
    return auth,root


@contextmanager
def generated_preflight(delta=(10,0,0,190,0,0,0,0)):
    # Exercise the real admission function without reading native /proc or
    # invoking host inspection. Every input here is explicitly synthetic.
    before=[1000]*8;after=[a+b for a,b in zip(before,delta)]
    samples=iter(('cpu '+' '.join(map(str,v))+'\n' for v in (before,after)))
    def read(path,*args,**kwargs):
        if str(path)=='/proc/stat':return next(samples)
        if str(path)=='/proc/meminfo':
            return 'MemAvailable: 1048576 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\n'
        if str(path)=='/proc/sys/kernel/random/boot_id':return 'generated-boot\n'
        raise AssertionError('unexpected external read: '+str(path))
    pre=dict(hardware={'generated':True},no_competing_work=True,
        boot_id='generated-boot',timestamp=time.time(),idle_percent=95,steal_percent=0)
    with patch.object(sh,'read_ref',return_value=pre),patch.object(sh.stage,'hardware',return_value=pre['hardware']),\
            patch.object(Path,'read_text',read),patch.object(sh.stage.subprocess,'check_output',return_value=''):
        yield


class Guards(unittest.TestCase):
    def test_native_idle_fixed_window_contract(self):
        with generated_preflight(),patch.object(sh.time,'sleep') as sleep:
            sh.native_preflight({'preflight':{}},fresh=True)
            sleep.assert_called_once_with(1.0)
        self.assertEqual(sh.SCHEMA,'stage-a-singleton-v2')
        expected=dict(schema='stage-a-native-idle-v1',seconds=1.0,
            method='proc-stat-single-delta-idle-plus-iowait',minimum_idle_percent=90,
            maximum_steal_ticks=0)
        self.assertEqual(sh.contract([b'generated\n']*59)['native_idle_sample'],expected)

    def test_native_idle_window_mutation_binding(self):
        with retained_fixture() as run:
            auth,_=setup_root(run);launches=[]
            def launch(r,a,p,s,o,n,g):
                launches.append(a)
                sh.NATIVE_IDLE_SECONDS=.05
                return mock_output(o,n)
            with patch.object(sh,'NATIVE_IDLE_SECONDS',1.0),patch.object(sh.stage,'check_release'),\
                    patch.object(sh,'launch',launch):
                with self.assertRaisesRegex(ValueError,'tool/config mutation'):
                    sh.run_phase(run,auth,sh.stage.sha(auth),standin=True)
            self.assertEqual(launches,['dense'])

    def test_native_idle_single_shot_fail_closed(self):
        for delta,passes in [((20,0,0,180,0,0,0,0),True),
                ((21,0,0,179,0,0,0,0),False),((0,)*8,False),
                ((0,0,0,199,0,0,0,1),False)]:
            with self.subTest(delta=delta),generated_preflight(delta),patch.object(sh.time,'sleep') as sleep:
                if passes:sh.native_preflight({'preflight':{}})
                else:
                    with self.assertRaisesRegex(ValueError,'idle/steal'):
                        sh.native_preflight({'preflight':{}})
                sleep.assert_called_once_with(1.0)

    def test_native_idle_wait_charged_and_interrupted(self):
        # Real sleep + real supervisor alarm, synthetic /proc only. No native
        # input/host admission or expensive probe is permitted by this fixture.
        with retained_fixture() as run:
            budget=sh.Budget(2,1);start=time.monotonic()
            with generated_preflight(),sh.deadline(2),sh.cpu_deadline(1),budget.deadlines():
                sh.native_preflight({'preflight':{}})
            charged=budget.snapshot()
            self.assertGreaterEqual(charged['wall'],1.0)
            self.assertGreater(charged['supervisor_cpu'],0)
            with generated_preflight(),sh.deadline(.08),sh.cpu_deadline(1):
                with self.assertRaises(sh.Deadline):
                    sh.native_preflight({'preflight':{}})
            self.assertLess(time.monotonic()-start,1.8)
            sh.write_json(run/'idle-budget-proof.json',dict(charged=charged,
                interrupted_window=True,window_seconds=1.0,alarm_seconds=.08))

    def test_setup_cpu_budget_stops_admission(self):
        with retained_fixture() as run:
            with patch.dict(sh.CEILINGS,F1=[.75,.03,4]):
                auth,_=setup_root(run)
                def burn(*args):
                    start=time.process_time()
                    while time.process_time()-start<.15:pass
                    self.fail('setup completed beyond global CPU cap')
                start=time.process_time()
                with patch.object(sh.stage,'check_release',burn),patch.object(sh,'launch') as launch:
                    with self.assertRaisesRegex(sh.Deadline,'CPU'):
                        sh.run_phase(run,auth,sh.stage.sha(auth),standin=True)
                    launch.assert_not_called()
                elapsed=time.process_time()-start
                self.assertLess(elapsed,.12)
                self.assertTrue((run/'release/F1/claim.json').exists())
                with self.assertRaises(FileExistsError):
                    sh.run_phase(run,auth,sh.stage.sha(auth),standin=True)
                sh.write_json(run/'setup-cpu-proof.json',dict(cpu_seconds=elapsed,cap=.03,stopped=True))

    def test_continuation_setup_uses_remaining_cpu(self):
        # A deliberately minimal, hash-bound parent exercises only budget
        # admission. No output/native qualification is faked into a launch.
        for spent in (.29,.32):
            with self.subTest(spent=spent),retained_fixture() as run:
                with patch.dict(sh.CEILINGS,F1=[.75,.32,4],F2=[1,.32,118]):
                    auth,_=setup_root(run)
                    dest=run/'release';dest.mkdir();(dest/'F1').mkdir()
                    parent=dest/'F1/result.json'
                    sh.write_json(parent,dict(phase='F1',status='complete',root=sh.reference(auth),
                        completed=4,budget=dict(wall=.1,cpu=spent)))
                    path=run/'F2.json'
                    sh.write_json(path,dict(schema=sh.SCHEMA,phase='F2',mode='standin',released=True,
                        root=sh.reference(auth),parent=sh.reference(parent),review_approved=True))
                    entered=[]
                    def burn(*args):
                        entered.append(True);start=time.process_time()
                        while time.process_time()-start<.15:pass
                        self.fail('continuation setup received fresh CPU allowance')
                    start=time.process_time()
                    with patch.object(sh.stage,'check_release',burn),patch.object(sh,'launch') as launch:
                        if spent<.32:
                            with self.assertRaisesRegex(sh.Deadline,'CPU'):
                                sh.run_phase(run,path,sh.stage.sha(path),standin=True)
                        else:
                            with self.assertRaisesRegex(ValueError,'exhausted'):
                                sh.run_phase(run,path,sh.stage.sha(path),standin=True)
                            self.assertEqual(entered,[])
                        launch.assert_not_called()
                    elapsed=time.process_time()-start
                    self.assertLess(elapsed,.12)
                    failure=json.loads((dest/'F2/failure.json').read_text())
                    self.assertGreaterEqual(failure['budget']['cpu'],spent)
                    with self.assertRaises(FileExistsError):
                        sh.run_phase(run,path,sh.stage.sha(path),standin=True)
                    sh.write_json(run/'continuation-cpu-proof.json',dict(cpu_seconds=elapsed,
                        prior_cpu=spent,cumulative_cap=.32,heavy_setup_entered=bool(entered),stopped=True))

    def test_driver_deadline_reaps_and_never_restarts(self):
        with retained_fixture() as run:
            auth,_=setup_root(run);launches=[]
            def launch(r,a,p,s,out,name,g):
                launches.append(a)
                signal.setitimer(signal.ITIMER_REAL,.2)
                sh.stage.support.bounded([sys.executable,'-c',
                    'import os,time;print(os.getpid(),flush=True);os.fork();print(os.getpid(),flush=True);time.sleep(20)'],out,name,**sh.stage.LIMITS)
                raise AssertionError('global deadline did not fire')
            start=time.monotonic()
            with patch.object(sh.stage,'check_release'),patch.object(sh,'launch',launch):
                with self.assertRaises(sh.Deadline):sh.run_phase(run,auth,sh.stage.sha(auth),standin=True)
                with self.assertRaises(FileExistsError):sh.run_phase(run,auth,sh.stage.sha(auth),standin=True)
            self.assertLess(time.monotonic()-start,3)
            failure=json.loads((run/'release/F1/failure.json').read_text())
            self.assertGreaterEqual(failure['budget']['cpu'],90)
            self.assertEqual(launches,['dense'])
            for pid in (run/'release/F1/000-1-1-dense.out').read_text().split():
                result=subprocess.run(['ps','-o','stat=','-p',pid],capture_output=True,text=True)
                self.assertTrue(not result.stdout.strip() or result.stdout.strip().startswith('Z'))

    def test_setup_claim_and_reservation_rejection(self):
        with retained_fixture() as run:
            auth,_=setup_root(run)
            with patch.object(sh.stage,'check_release',side_effect=KeyboardInterrupt):
                with self.assertRaises(KeyboardInterrupt):sh.run_phase(run,auth,sh.stage.sha(auth),standin=True)
            self.assertTrue((run/'release/F1/claim.json').exists())
            with self.assertRaises(FileExistsError):sh.run_phase(run,auth,sh.stage.sha(auth),standin=True)
        with retained_fixture() as run:
            auth,_=setup_root(run);original=sh.Budget
            with patch.object(sh.stage,'check_release'),patch.object(sh,'Budget',side_effect=lambda *args:original(119,89)),patch.object(sh,'launch') as launch:
                with self.assertRaisesRegex(ValueError,'reservation'):
                    sh.run_phase(run,auth,sh.stage.sha(auth),standin=True)
                launch.assert_not_called()

    def test_new_native_qualification_is_mandatory(self):
        self.assertTrue(hasattr(sh,'check_scheduler_qualification'),'native scheduler qualification gate missing')
        with retained_fixture() as run:
            path=run/'fake-qualification.json'
            sh.write_json(path,dict(qualified=True,native_hard_as_qualified=False,tools=sh.tool_hashes()))
            with self.assertRaises(ValueError):
                sh.check_scheduler_qualification({'scheduler_qualification':sh.reference(path)})

    def test_early_peak_and_storage_reservations(self):
        self.assertTrue(hasattr(sh,'row_peak'),'row peak early gate missing')
        with retained_fixture() as out:
            data,_,_=mock_output(out,'template');values=[]
            for j in range(1,7):
                for arm in ('dense','tiled'):
                    d=copy.deepcopy(data)
                    d[2]['process_peak_rss_bytes']=100+(8388609 if arm=='tiled' else 0)
                    values.append(((j,1,arm),d))
            self.assertEqual(sh.row_peak(values,1)['classification'],'fail')
            with self.assertRaises(ValueError):sh.row_peak(values[:-1],1)
            with patch.object(sh.shutil,'disk_usage',return_value=type('Disk',(),{'free':0})()):
                with self.assertRaises(ValueError):sh.storage_guard(out)
            (out/'too-large').write_bytes(b'x'*1000)
            with patch.dict(sh.STORAGE,release_bytes=100):
                with self.assertRaises(ValueError):sh.storage_guard(out)
            with self.assertRaises(ValueError):sh.write_json(out/'oversize.json',{'value':'x'*100},cap=20)

    def test_wrong_shard_and_repeated_ordinals(self):
        with retained_fixture() as run:
            auth,root=setup_root(run)
            root['contract']['shard_sha256'][0]='f'*64
            sh.write_json(auth,root)
            with patch.object(sh.stage,'check_release'),patch.object(sh,'launch') as launch:
                with self.assertRaises(ValueError):sh.run_phase(run,auth,sh.stage.sha(auth),standin=True)
                launch.assert_not_called()
            rows=run/'fixtures/repeated.tsv';rows.write_text('0 0 0 0 0 0 0 0 0 0\n'*59)
            self.assertEqual(len(sh.normalized_rows(rows)),59)

    def test_supervisor_cpu_and_interrupt(self):
        self.assertTrue(hasattr(sh,'cpu_deadline'),'CPU alarm missing')
        start=time.process_time()
        with self.assertRaises(sh.Deadline),sh.cpu_deadline(.02):
            while True: pass
        self.assertLess(time.process_time()-start,.5)
        with self.assertRaises(sh.Deadline),sh.deadline(2):
            os.kill(os.getpid(),signal.SIGTERM)

    def test_negative_row_cpu_rejected(self):
        with retained_fixture() as out:
            data,_,_=mock_output(out,'template')
            results=[(slot,copy.deepcopy(data)) for slot in sh.schedule('acceptance')]
            results[0][1][5]['cpu_ns']=-1
            with self.assertRaises(ValueError):sh.aggregate(results)

    def test_final_aggregation_output_mutation(self):
        with retained_fixture() as run:
            auth,_=setup_root(run);original=sh.collect
            def collect(dest,entries,phase):
                values=original(dest,entries,phase)
                path=dest/(entries[0]['name']+'.out');path.write_bytes(path.read_bytes()+b'\n')
                return values
            with patch.object(sh.stage,'check_release'),patch.object(sh,'launch',lambda r,a,p,s,o,n,g:mock_output(o,n)),patch.object(sh,'collect',collect):
                with self.assertRaisesRegex(ValueError,'sha256'):
                    sh.run_phase(run,auth,sh.stage.sha(auth),standin=True)

    def test_continuation_original_auth_and_receipt_gaps(self):
        for target in ('root','parent','output','partial','negative_budget'):
            with self.subTest(target=target),retained_fixture() as run:
                auth,_=setup_root(run)
                with patch.object(sh.stage,'check_release'),patch.object(sh,'launch',lambda r,a,p,s,o,n,g:mock_output(o,n)):
                    parent=sh.run_phase(run,auth,sh.stage.sha(auth),standin=True)
                    next_auth=dict(schema=sh.SCHEMA,phase='F2',mode='standin',released=True,root=sh.reference(auth),parent=sh.reference(parent),review_approved=True)
                    p=json.loads(parent.read_text())
                    if target=='root':auth.write_bytes(auth.read_bytes()+b'\n')
                    if target=='parent':parent.write_bytes(parent.read_bytes()+b'\n')
                    if target=='output':
                        out=parent.parent/(p['entries'][0]['name']+'.out');out.write_bytes(out.read_bytes()+b'\n')
                    if target in ('partial','negative_budget'):
                        if target=='partial':p['completed']=3
                        else:p['budget']['cpu']=-100
                        sh.write_json(parent,p);next_auth['parent']=sh.reference(parent)
                    path=run/'F2.json';sh.write_json(path,next_auth)
                    with self.assertRaises(ValueError):sh.run_phase(run,path,sh.stage.sha(path),standin=True)

    def test_limits_receipt_exact_and_numeric_output(self):
        self.assertTrue(hasattr(sh,'validate_receipt'),'receipt validation missing')
        good=dict(failure=None,returncode=0,wall_seconds=1,limits=dict(sh.stage.LIMITS,file_size=sh.stage.LIMITS['output']))
        sh.validate_receipt(good,.5)
        for field,value in [('wall_seconds',float('nan')),('wall_seconds',81),('returncode',-9),('limits',dict(sh.stage.LIMITS,cpu=91))]:
            with self.subTest(field=field),self.assertRaises(ValueError):
                sh.validate_receipt(dict(good,**{field:value}),.5)
        for cpu in (None,float('nan'),61,-1):
            with self.assertRaises(ValueError):sh.validate_receipt(good,cpu)

    def test_mutation_after_child_and_loaded_config(self):
        for target in ('authorization','loaded','input','copy','full_tsv','singleton','tool','source','lock','binary','admission','native_qualification'):
            with self.subTest(target=target),retained_fixture() as run:
                auth,root=setup_root(run);launches=[];original=sh.tool_hashes
                changed=[False]
                def prerequisite(*args):
                    if changed[0] and target in ('source','lock','binary','admission','native_qualification'):
                        raise ValueError('root prerequisite mutation')
                    if target=='loaded' and len(launches)==1:
                        args[1]['input_sha256']='0'*64
                def launch(run,arm,pack,selector,out,name,standin):
                    launches.append(arm)
                    data=mock_output(out,name)
                    paths={'authorization':auth,'input':run/'fixtures/tiny.bin','copy':pack,
                           'full_tsv':run/'fixtures/59.tsv','singleton':selector}
                    if target in paths:
                        path=paths[target];path.chmod(0o644);path.write_bytes(path.read_bytes()+b'\n')
                    changed[0]=True
                    return data
                def tools():
                    value=original()
                    if target=='tool' and changed[0]:value['mutated']=True
                    return value
                with patch.object(sh.stage,'check_release',prerequisite),patch.object(sh,'launch',launch),patch.object(sh,'tool_hashes',tools):
                    with self.assertRaises(ValueError):sh.run_phase(run,auth,sh.stage.sha(auth),standin=True)
                self.assertEqual(launches,['dense'])
                failure=json.loads((run/'release/F1/failure.json').read_text())
                self.assertEqual(failure['completed'],0)
                self.assertGreaterEqual(failure['budget']['cpu'],90)

    def test_failure_headroom_and_no_restart(self):
        for failure in ('exit','wall','cpu','unknown','truncated','missing','output'):
            with self.subTest(failure=failure),retained_fixture() as run:
                auth,_=setup_root(run);launches=[]
                def launch(run,arm,pack,selector,out,name,standin):
                    launches.append(arm);data,r,cpu=mock_output(out,name)
                    if failure=='exit':r['returncode']=-9
                    if failure=='wall':r['wall_seconds']=80.1
                    if failure=='cpu':cpu=60.1
                    if failure=='unknown':cpu=None
                    if failure=='truncated':data[-1]['raw_elapsed_ns'].pop()
                    if failure=='missing':data.pop()
                    if failure=='output':raise ValueError('output')
                    return data,r,cpu
                with patch.object(sh.stage,'check_release'),patch.object(sh,'launch',launch):
                    with self.assertRaises((ValueError,TypeError,KeyError)):sh.run_phase(run,auth,sh.stage.sha(auth),standin=True)
                    with self.assertRaises(FileExistsError):sh.run_phase(run,auth,sh.stage.sha(auth),standin=True)
                self.assertEqual(launches,['dense'])

    def test_weighted_cpu_noise_and_partial(self):
        results=[]
        with retained_fixture() as out:
            data,_,_=mock_output(out,'template')
            for slot in sh.schedule('acceptance'):
                j,row,arm=slot;d=copy.deepcopy(data)
                d[5]['cpu_ns']=100 if row==1 else (2 if arm=='tiled' else 1) if row==2 else 0
                results.append((slot,d))
            result=sh.aggregate(results)
            self.assertAlmostEqual(result['metrics']['route_cpu']['median_delta'],1/101)
            for bad in (results[:-1],[results[1],results[0],*results[2:]]):
                with self.assertRaises(ValueError):sh.aggregate(bad)
            for b,c,threshold,rel,expected in [([100,200]*3,[101,201]*3,.05,True,'inconclusive'),
                    ([100]*6,[105]*6,.05,True,'pass'),([100]*6,[106]*6,.05,True,'fail'),
                    ([100]*6,[2000100]*6,2000000,False,'pass'),
                    ([100]*6,[8388708]*6,8388608,False,'pass'),
                    ([100,101]*3,[2000100]*6,2000000,False,'inconclusive')]:
                self.assertEqual(sh.stage.paired(b,c,threshold,rel)['classification'],expected)
            a=[1]*1401+[100]*15;b=[1]*1416
            for i in range(15):b[i*24]=100
            self.assertEqual(sh.p99(a),sh.p99(b))
            sh.write_json(out/'quantiles.json',dict(clustered=a,spread=b,lane=[1]*467+[100]*5,
                pooled_rank=1402,lane_rank=468,pooled=sh.p99(a),lane_p99=sh.p99([1]*467+[100]*5)))


if __name__=='__main__':unittest.main()
