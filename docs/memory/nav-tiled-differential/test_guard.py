import hashlib
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
import os
import subprocess
import time
import platform

SPEC = importlib.util.spec_from_file_location('harness', Path(__file__).with_name('harness.py'))
h = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(h)

class Guards(unittest.TestCase):
    @unittest.skipUnless(platform.system()=='Linux','native hard AS qualification requires Linux')
    def test_address_limit(self):
        with tempfile.TemporaryDirectory() as d:
            code='import sys\ntry: b=bytearray(600*1024**2)\nexcept MemoryError: sys.exit(23)\nsys.exit(0)'
            r=h.bounded([sys.executable,'-c',code],Path(d),'address',address=256*1024**2,rss=1024**3)
            self.assertTrue(r['address_guard_active'])
            self.assertEqual(r['returncode'],23)

    def test_real_release_fails_closed_before_opening_input(self):
        with self.assertRaises(ValueError): h.check_real_release({},Path('/never-opened'))

    def test_fixed_selectors_reject_unbounded_work(self):
        with self.assertRaises(ValueError): h.fixed_selectors('100 200 0 100 200 0 0 0 999999 0')
        with self.assertRaises(ValueError): h.fixed_selectors('')

    def test_lossless_compare_catches_last_byte(self):
        with tempfile.TemporaryDirectory() as d:
            a=Path(d)/'a';b=Path(d)/'b'
            a.write_bytes(b'x'*(1024**2)+b'a');b.write_bytes(b'x'*(1024**2)+b'b')
            self.assertEqual(h.compare_files(a,b),dict(equal=False,first_difference=1024**2))

    def assert_dead(self, pid):
        deadline=time.monotonic()+2
        while time.monotonic()<deadline:
            r=subprocess.run(['ps','-p',str(pid),'-o','stat='],capture_output=True,text=True)
            if not r.stdout.strip() or r.stdout.strip().startswith('Z'): return
            time.sleep(.02)
        self.fail('owned descendant remains alive: '+str(pid))

    def test_descendant_cleanup_normal_and_timeout(self):
        for leader_sleep in (0,10):
            with self.subTest(leader_sleep=leader_sleep), tempfile.TemporaryDirectory() as d:
                marker=Path(d)/'child.pid'
                code=f'import os,time; pid=os.fork();\nif pid==0: time.sleep(30)\nelse: open({str(marker)!r},"w").write(str(pid)); time.sleep({leader_sleep})'
                r=h.bounded([sys.executable,'-c',code],Path(d),'child',wall=.5)
                self.assertEqual(r['failure'],'wall' if leader_sleep else None)
                self.assert_dead(int(marker.read_text()))

    def test_combined_descendant_pipe_budget(self):
        with tempfile.TemporaryDirectory() as d:
            marker=Path(d)/'child.pid'
            code=f'import os,time; pid=os.fork();\nif pid==0:\n while True: os.write(1,b"x"*4096); os.write(2,b"y"*4096)\nelse: open({str(marker)!r},"w").write(str(pid)); time.sleep(30)'
            r=h.bounded([sys.executable,'-c',code],Path(d),'pipes',wall=2,output=20000)
            self.assertEqual(r['failure'],'output')
            self.assertEqual(r['output_bytes'],20000)
            self.assert_dead(int(marker.read_text()))

    def test_cpu_limit(self):
        with tempfile.TemporaryDirectory() as d:
            r=h.bounded([sys.executable,'-c','while True: pass'],Path(d),'cpu',cpu=1,wall=5)
            self.assertEqual(r['failure'],'exit')
            self.assertLess(r['returncode'],0)

    def test_rss_limit(self):
        with tempfile.TemporaryDirectory() as d:
            r=h.bounded([sys.executable,'-c','import time; b=bytearray(128*1024**2); time.sleep(5)'],Path(d),'rss',rss=40*1024**2,wall=3)
            self.assertEqual(r['failure'],'rss')

    def test_huge_string_rejected_before_decoder(self):
        import generate
        b=generate.pack(edges=[generate.edge(req=True)])
        b=bytearray(b)
        # quest length after fixed edge and skill/item vectors.
        pos=25+36+8+4+42+20+12+4
        b[pos:pos+4]=generate.U(0xffffffff)
        with self.assertRaises(ValueError): h.safe_wire(b,65536)

    def test_existing_output_does_not_launch_child(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d); (root/'same.out').write_text('keep')
            marker=root/'launched'
            with self.assertRaises(FileExistsError):
                h.bounded([sys.executable,'-c',f'open({str(marker)!r},"w").write("bad")'],root,'same')
            time.sleep(.1)
            self.assertFalse(marker.exists())
    def test_pipe_eof_is_not_leader_completion(self):
        with tempfile.TemporaryDirectory() as d:
            marker = Path(d)/'finished'
            code = f'import os,time; os.close(1); os.close(2); time.sleep(.25); open({str(marker)!r},"w").write("done")'
            r = h.bounded([sys.executable, '-c', code], Path(d), 'eof')
            self.assertEqual(r['returncode'], 0)
            self.assertTrue(marker.exists())
    def test_hash_rejects_changed_input(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / 'input'
            p.write_bytes(b'changed')
            with self.assertRaises(ValueError):
                h.admit(p, hashlib.sha256(b'original').hexdigest(), 100)
    def test_symlink_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / 'input'; p.write_bytes(b'x')
            q = Path(d) / 'link'; q.symlink_to(p)
            with self.assertRaises(ValueError): h.admit(q, h.sha(p), 100)
    def test_large_dimensions_rejected_before_decoder(self):
        import struct
        p = b'274V\x08' + bytes(12) + struct.pack('<II', 16000, 16000)
        with self.assertRaises(ValueError): h.safe_header(p, 65536)
    def test_invalid_dimensions_can_reach_error_decoder(self):
        import struct
        for w in (0, 16385, 0xffffffff):
            h.safe_header(b'274V\x08' + bytes(12) + struct.pack('<II', w, 1), 65536)
    def test_wall_timeout_and_owned_cleanup(self):
        with tempfile.TemporaryDirectory() as d:
            r = h.bounded([sys.executable, '-c', 'import time; time.sleep(10)'], Path(d), 'wall', wall=.15)
            self.assertEqual(r['failure'], 'wall')
            self.assertNotEqual(r['returncode'], 0)
    def test_output_limit(self):
        with tempfile.TemporaryDirectory() as d:
            r = h.bounded([sys.executable, '-c', 'import os;\nwhile True: os.write(1,b"x"*65536)'], Path(d), 'output', output=16384)
            self.assertEqual(r['failure'], 'output')
            self.assertLessEqual(sum((Path(d)/('output.'+s)).stat().st_size for s in ('out','err')),16384)
    def test_nonzero_retained(self):
        with tempfile.TemporaryDirectory() as d:
            r = h.bounded([sys.executable, '-c', 'raise SystemExit(23)'], Path(d), 'exit')
            self.assertEqual(r['returncode'], 23)
            self.assertEqual(r['failure'], 'exit')

if __name__ == '__main__': unittest.main()
