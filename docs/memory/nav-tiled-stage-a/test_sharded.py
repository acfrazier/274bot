"""Generated-only scheduler contract tests; no external inputs."""
import importlib.util
from pathlib import Path
import unittest
import sharded as sh
import tempfile
import time
import sys
import subprocess
import platform
import json
from unittest.mock import patch
from contextlib import contextmanager


@contextmanager
def retained_fixture():
    path=Path(tempfile.mkdtemp(prefix='sharded-test-',dir=sh.stage.HERE))
    print('retained fixture:',path,flush=True)
    yield path


class Metrics(unittest.TestCase):
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
            with patch.object(sh.stage,'check_release'),patch.object(sh,'launch',launch):
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
