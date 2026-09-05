import json
import pathlib
import tempfile
import unittest
from cpu_screen import summarize

class QualificationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.run = pathlib.Path(self.temp.name)
        (self.run/'metadata.json').write_text(json.dumps({'exit_code':0,'n':1,'observe_s':180}))
        self.rows = [dict(phase='observe',elapsed_s=t,ready=1,active=1,allocation_counting=False,
                         diagnostic_sidecar=False,rust_allocations=None,rust_allocated_bytes=None,
                         rust_live_bytes=None,process_cpu_user_s=t/2,process_cpu_system_s=t/4,
                         client_tick_count=t*50,resident_bytes=100) for t in [0,90,180]]
        self.proof = [dict(phase=phase,slots=[dict(name='bot',state='Running',error=None,
                       runtime={'paint':{'lines':[f'Steals: {count}']}})])
                      for phase,count in [('observe-start',2),('observe-end',7)]]

    def report(self):
        for name, rows in [('samples.jsonl',self.rows),('samples.qualification.jsonl',self.proof)]:
            (self.run/name).write_text(''.join(json.dumps(r)+'\n' for r in rows))
        return summarize(self.run,False,False)

    def test_system_mode_cpu_and_progress(self):
        r=self.report()
        self.assertTrue(r['qualified'])
        self.assertEqual(r['cpu_cores'],0.75)
        self.assertEqual(r['client_ticks_per_slot_s'],50)
        self.assertEqual(r['steal_gains'],{'bot':5})

    def test_wrong_mode_cannot_qualify(self):
        self.rows[1]['allocation_counting']=True
        self.assertFalse(self.report()['qualified'])

    def test_ready_drop_cannot_qualify(self):
        self.rows[1]['ready']=0
        self.assertFalse(self.report()['qualified'])

    def test_no_progress_cannot_qualify(self):
        self.proof[1]['slots'][0]['runtime']['paint']['lines']=['Steals: 2']
        self.assertFalse(self.report()['qualified'])

    def test_missing_boundary_cannot_qualify(self):
        self.proof.pop()
        self.assertFalse(self.report()['qualified'])

    def test_nonmonotonic_cpu_cannot_qualify(self):
        self.rows[1]['process_cpu_user_s']=200
        self.assertFalse(self.report()['qualified'])

if __name__=='__main__':
    unittest.main()
