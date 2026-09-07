import json
import os
import pathlib
import pty
import select
import tempfile
import time
import tty
import unittest

from tui_input_probe import InputProbe
import run_diagnostic as rd


class ProbeTest(unittest.TestCase):
    def test_real_pty_only_observation_and_restores_overlay(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            master, slave = pty.openpty()
            tty.setraw(slave)
            probe = InputProbe(master, root, interval_s=2)
            probe.start()
            try:
                self.assertFalse(select.select([slave], [], [], .15)[0])
                boundaries = root / 'samples.qualification.jsonl'
                boundaries.write_text('{"phase":"observe-start"}\n')
                self.assertTrue(select.select([slave], [], [], 2)[0])
                self.assertEqual(os.read(slave, 1), b'o')
                with boundaries.open('a') as f:
                    f.write('{"phase":"observe-end"}\n')
                self.assertTrue(select.select([slave], [], [], 2)[0])
                self.assertEqual(os.read(slave, 1), b'o')
                probe.thread.join(2)
                self.assertFalse(probe.thread.is_alive())
                self.assertIsNone(probe.error)
                rows = [json.loads(x) for x in (root/'input-probes.jsonl').read_text().splitlines()]
                self.assertEqual([x['kind'] for x in rows], ['configuration','write','restore','complete'])
                self.assertEqual(rows[-1]['sent'], 1)
                self.assertFalse(select.select([slave], [], [], .15)[0])
            finally:
                probe.close()
                os.close(master)
                os.close(slave)

    def test_malformed_boundaries_fail_without_keystrokes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root/'samples.qualification.jsonl').write_text('{"phase":"observe-end"}\n')
            master, slave = pty.openpty()
            tty.setraw(slave)
            probe = InputProbe(master, root)
            probe.start()
            try:
                probe.thread.join(2)
                self.assertIn('boundary sequence', probe.error)
                self.assertEqual(probe.sent, 0)
                self.assertFalse(select.select([slave], [], [], .1)[0])
            finally:
                probe.close()
                os.close(master)
                os.close(slave)

    def test_partial_line_waits_and_child_exit_closes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root/'samples.qualification.jsonl').write_text('{"phase":"observe-start"')
            master, slave = pty.openpty()
            probe = InputProbe(master, root)
            probe.start()
            time.sleep(.15)
            probe.close()
            os.close(master)
            os.close(slave)
            self.assertIsNone(probe.error)
            self.assertEqual(probe.sent, 0)
            with self.assertRaises(OSError):
                os.fstat(probe.fd)

    def test_cli_only_accepts_real_tui(self):
        p = rd.build_parser()
        a = p.parse_args(['tui','1','active','--tui-input-probes'])
        rd.validate_args(a, p)
        self.assertTrue(a.tui_input_probes)
        self.assertFalse(p.parse_args(['tui','1','active']).tui_input_probes)
        for args in (['panel','1','active','--tui-input-probes'],
                     ['tui','1','active','--headless','--tui-input-probes']):
            with self.assertRaises(SystemExit):
                rd.validate_args(p.parse_args(args), p)


if __name__ == '__main__':
    unittest.main()
