#!/usr/bin/env python3
"""CLI validation for run_diagnostic (no live launch)."""
from __future__ import annotations

import pathlib
import os
import argparse
import json
import signal
import subprocess
import sys
import tempfile
import textwrap
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parent
SCRIPT = ROOT / "run_diagnostic.py"
sys.path.insert(0, str(ROOT))
import run_diagnostic as rd  # noqa: E402


def run_cli(*args: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )


class RunDiagnosticCli(unittest.TestCase):
    def test_direct_owner_consumes_only_the_pre_reserved_empty_run_directory(self):
        parser = rd.build_parser()
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            reserved = root / 'reserved'
            reserved.mkdir()
            args = parser.parse_args([
                'tui', '1', 'active', '--no-diagnostics', '--sustain',
                '--warmup', '30', '--observe', '120', '--direct-owner-capture',
                '--run-dir', str(reserved), '--frontend-handoff', str(root / 'handoff.json'),
            ])
            self.assertEqual(rd.prepare_run_directory(args, root), reserved.resolve())
            self.assertTrue(reserved.is_dir())
            (reserved / 'foreign').write_text('x')
            with self.assertRaisesRegex(RuntimeError, 'empty'):
                rd.prepare_run_directory(args, root)

            ordinary = parser.parse_args(['tui', '1', 'active'])
            generated = rd.prepare_run_directory(ordinary, root, timestamp='stamp')
            self.assertEqual(generated, root / 'docs/memory/diagnostics/stamp_tui_n1_active')
            self.assertTrue(generated.is_dir())

    def test_direct_owner_cli_is_exact_and_path_flags_are_mode_only(self):
        parser = rd.build_parser()
        valid = parser.parse_args([
            "tui", "1", "active", "--no-diagnostics", "--sustain",
            "--warmup", "30", "--observe", "120", "--direct-owner-capture",
            "--run-dir", "/tmp/direct-run", "--frontend-handoff", "/tmp/direct-handoff.json",
        ])
        rd.validate_args(valid, parser)
        for extra in (("--run-dir", "/tmp/r"), ("--frontend-handoff", "/tmp/h")):
            bad = parser.parse_args(["tui", "1", "active", *extra])
            with self.assertRaises(SystemExit):
                rd.validate_args(bad, parser)
        invalid = (
            ["tui", "16", "active"], ["panel", "1", "active"],
            ["tui", "1", "idle"], ["tui", "1", "active", "--headless"],
            ["tui", "1", "active", "--heaptrack-output", "/tmp/h"],
        )
        for prefix in invalid:
            args = parser.parse_args([*prefix, "--no-diagnostics", "--sustain", "--warmup", "30", "--observe", "120",
                                      "--direct-owner-capture", "--run-dir", "/tmp/r", "--frontend-handoff", "/tmp/f"])
            with self.assertRaises(SystemExit):
                rd.validate_args(args, parser)

    def test_direct_owner_child_env_scrubs_inherited_and_sets_only_after_validation(self):
        parser = rd.build_parser()
        off = parser.parse_args(["tui", "1", "active"])
        direct = parser.parse_args([
            "tui", "1", "active", "--no-diagnostics", "--sustain", "--warmup", "30",
            "--observe", "120", "--direct-owner-capture", "--run-dir", "/tmp/r",
            "--frontend-handoff", "/tmp/f",
        ])
        rd.validate_args(direct, parser)
        polluted = {"BOT_MEMORY_OWNER_CAPTURE": "1", "PATH": "/usr/bin"}
        self.assertNotIn("BOT_MEMORY_OWNER_CAPTURE", rd.build_child_env(off, "/tmp/off", base_env=polluted))
        self.assertEqual(rd.build_child_env(direct, "/tmp/r", base_env=polluted)["BOT_MEMORY_OWNER_CAPTURE"], "1")

    def test_spawn_and_exit_handoff_preserve_immutable_spawn_object(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            slot = root / "frontend-handoff.json"
            spawn = rd.build_spawn_handoff(
                nonce="n", frontend_pid=11, frontend_start_identity="start:11",
                launcher_pid=7, run_dir=root / "run", spawn_before=1.0, spawn_after=1.1,
                frontend="tui", n=1, workload="active", warmup_s=30, observe_s=120,
            )
            rd.write_spawn_handoff(slot, spawn)
            first = json.loads(slot.read_text())
            self.assertEqual(first["state"], "spawned")
            rd.write_exit_handoff(slot, spawn, exit_code=0, wait_return_monotonic_s=2.0)
            exited = json.loads(slot.read_text())
            self.assertEqual(exited["state"], "exited")
            self.assertEqual(exited["spawn"], first["spawn"])
            self.assertEqual(exited["frontend_start_identity"], "start:11")
            self.assertFalse((root / "frontend-handoff.next.json").exists())

    @unittest.skipUnless(hasattr(signal, "pthread_sigmask"), "POSIX signal masks")
    def test_direct_child_preexec_restores_prior_mask_without_unblocking_parent(self):
        prior = signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGTERM, signal.SIGINT})
        self.addCleanup(lambda: signal.pthread_sigmask(signal.SIG_SETMASK, prior))
        called = []
        setup = rd.compose_direct_child_setup(
            lambda: called.append("terminal"), prior,
            {signal.SIGTERM: signal.SIG_DFL, signal.SIGINT: signal.default_int_handler},
        )
        with mock.patch.object(signal, "signal") as set_disposition, \
             mock.patch.object(signal, "pthread_sigmask") as set_mask:
            setup()
        self.assertEqual(called, ["terminal"])
        self.assertEqual(set_disposition.call_count, 2)
        set_mask.assert_called_once_with(signal.SIG_SETMASK, prior)

    @unittest.skipUnless(hasattr(signal, "pthread_sigmask"), "POSIX signal masks")
    def test_stop_recorded_during_spawn_targets_only_registered_child_after_handoff(self):
        for delivered in (signal.SIGTERM, signal.SIGINT):
            with self.subTest(delivered=delivered):
                events = []
                handlers = {}

                class Child:
                    pid = 321

                    def poll(self):
                        return None

                    def terminate(self):
                        events.append('terminate')

                child = Child()

                def install(sig, handler):
                    handlers[sig] = handler

                def popen(*_args, **_kwargs):
                    handlers[delivered](delivered, None)
                    return child

                with mock.patch.object(signal, 'getsignal', return_value=signal.SIG_DFL), \
                     mock.patch.object(signal, 'signal', side_effect=install), \
                     mock.patch.object(signal, 'pthread_sigmask', return_value=set()), \
                     mock.patch.object(rd, '_linux_direct_children', return_value={}), \
                     mock.patch.object(rd.subprocess, 'Popen', side_effect=popen), \
                     mock.patch.object(rd.sr, 'sample_process', return_value={'start_identity': 'start:321'}), \
                     mock.patch.object(rd, 'write_spawn_handoff', side_effect=lambda *_: events.append('handoff')):
                    rd.spawn_direct_unix(
                        binary=pathlib.Path('/tmp/binary'), root=pathlib.Path('/tmp'), env={},
                        slave=4, terminal_setup=lambda: None, run_dir=pathlib.Path('/tmp/run'),
                        handoff_path=pathlib.Path('/tmp/handoff.json'), frontend='tui', n=1,
                        workload='active', warmup_s=30, observe_s=120,
                    )
                self.assertEqual(events, ['handoff', 'terminate'])

    @unittest.skipUnless(hasattr(signal, 'pthread_sigmask'), 'POSIX signal masks')
    def test_direct_spawn_rejects_preblocked_stop_before_popen(self):
        with mock.patch.object(signal, 'getsignal', return_value=signal.SIG_DFL), \
             mock.patch.object(signal, 'signal'), \
             mock.patch.object(signal, 'pthread_sigmask', return_value={signal.SIGTERM}), \
             mock.patch.object(rd.subprocess, 'Popen') as popen:
            with self.assertRaisesRegex(RuntimeError, 'already blocked'):
                rd.spawn_direct_unix(
                    binary=pathlib.Path('/tmp/binary'), root=pathlib.Path('/tmp'), env={},
                    slave=4, terminal_setup=lambda: None, run_dir=pathlib.Path('/tmp/run'),
                    handoff_path=pathlib.Path('/tmp/handoff.json'), frontend='tui', n=1,
                    workload='active', warmup_s=30, observe_s=120,
                )
        popen.assert_not_called()

    def test_partial_spawn_cleanup_signals_only_unique_owned_identity(self):
        unique = {'pid': 321, 'start_identity': 'linux_proc_start_ticks:7',
                  'start_monotonic_s': 1.05}
        with mock.patch.object(rd, '_linux_direct_children', side_effect=[{321: unique},
                                                                         {321: unique}, {}]), \
             mock.patch.object(rd.os, 'kill') as kill:
            result = rd._cleanup_partial_direct_spawn('/tmp/binary', {}, 1.0, 1.1)
        self.assertTrue(result['cleaned'])
        self.assertFalse(result['orphan_risk'])
        kill.assert_called_once_with(321, signal.SIGTERM)

        ambiguous = {321: unique, 322: dict(unique, pid=322)}
        with mock.patch.object(rd, '_linux_direct_children', return_value=ambiguous), \
             mock.patch.object(rd.os, 'kill') as kill:
            result = rd._cleanup_partial_direct_spawn('/tmp/binary', {}, 1.0, 1.1)
        self.assertTrue(result['orphan_risk'])
        kill.assert_not_called()

        with mock.patch.object(rd, '_linux_direct_children', return_value={}), \
             mock.patch.object(rd.os, 'kill') as kill:
            result = rd._cleanup_partial_direct_spawn('/tmp/binary', {}, 1.0, 1.1)
        self.assertTrue(result['orphan_risk'])
        self.assertEqual(result['candidates'], [])
        kill.assert_not_called()

    @unittest.skipUnless(hasattr(signal, 'pthread_sigmask'), 'POSIX signal masks')
    def test_child_restoration_failure_preserves_orphan_risk_without_guessed_signal(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            failure = root / 'frontend-spawn-failure.json'
            with mock.patch.object(signal, 'getsignal', return_value=signal.SIG_DFL), \
                 mock.patch.object(signal, 'signal'), \
                 mock.patch.object(signal, 'pthread_sigmask', return_value=set()), \
                 mock.patch.object(rd, '_linux_direct_children', return_value={}), \
                 mock.patch.object(rd.subprocess, 'Popen', side_effect=RuntimeError('restore failed')), \
                 mock.patch.object(rd.os, 'kill') as kill:
                with self.assertRaisesRegex(RuntimeError, 'restore failed'):
                    rd.spawn_direct_unix(
                        binary=root / 'binary', root=root, env={}, slave=4,
                        terminal_setup=lambda: None, run_dir=root / 'run',
                        handoff_path=root / 'frontend-handoff.json', frontend='tui', n=1,
                        workload='active', warmup_s=30, observe_s=120,
                    )
            receipt = json.loads(failure.read_text())
            self.assertTrue(receipt['orphan_risk'])
            self.assertEqual(receipt['cleanup']['candidates'], [])
            kill.assert_not_called()

    @unittest.skipUnless(hasattr(signal, 'pthread_sigmask'), 'POSIX signal masks')
    def test_prespawn_discovery_failure_restores_parent_mask_and_records_orphan_risk(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            mask_calls = []

            def mask(how, value):
                mask_calls.append((how, value))
                return set()

            with mock.patch.object(signal, 'getsignal', return_value=signal.SIG_DFL), \
                 mock.patch.object(signal, 'signal'), \
                 mock.patch.object(signal, 'pthread_sigmask', side_effect=mask), \
                 mock.patch.object(rd, '_linux_direct_children',
                                   side_effect=OSError('proc unavailable')), \
                 mock.patch.object(rd.subprocess, 'Popen') as popen:
                with self.assertRaisesRegex(OSError, 'proc unavailable'):
                    rd.spawn_direct_unix(
                        binary=root / 'binary', root=root, env={}, slave=4,
                        terminal_setup=lambda: None, run_dir=root / 'run',
                        handoff_path=root / 'frontend-handoff.json', frontend='tui', n=1,
                        workload='active', warmup_s=30, observe_s=120,
                    )
            popen.assert_not_called()
            self.assertEqual(mask_calls[0][0], signal.SIG_BLOCK)
            self.assertEqual(mask_calls[-1], (signal.SIG_SETMASK, set()))
            receipt = json.loads((root / 'frontend-spawn-failure.json').read_text())
            self.assertTrue(receipt['orphan_risk'])
            self.assertIn('proc unavailable', receipt['cleanup']['error'])

    @unittest.skipUnless(sys.platform.startswith('linux'), 'Linux /proc mask qualification')
    def test_linux_direct_child_restores_stop_mask_before_exec(self):
        import fcntl
        import pty
        import termios

        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            binary = root / 'child.py'
            binary.write_text('#!/usr/bin/env python3\nimport time\ntime.sleep(30)\n')
            binary.chmod(0o755)
            run = root / 'run'
            run.mkdir()
            handoff = root / 'frontend-handoff.json'
            master, slave = pty.openpty()
            self.addCleanup(lambda: os.close(master))

            def terminal_setup():
                os.setsid()
                fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

            child, _handoff, _state = rd.spawn_direct_unix(
                binary=binary, root=root, env=os.environ.copy(), slave=slave,
                terminal_setup=terminal_setup, run_dir=run, handoff_path=handoff,
                frontend='tui', n=1, workload='active', warmup_s=30, observe_s=120,
            )
            os.close(slave)
            try:
                status = (pathlib.Path('/proc') / str(child.pid) / 'status').read_text()
                blocked_hex = next(line.split()[1] for line in status.splitlines()
                                   if line.startswith('SigBlk:'))
                blocked = int(blocked_hex, 16)
                self.assertEqual(blocked & (1 << (signal.SIGTERM - 1)), 0)
                self.assertEqual(blocked & (1 << (signal.SIGINT - 1)), 0)
                child.send_signal(signal.SIGTERM)
                child.wait(timeout=2)
            finally:
                if child.poll() is None:
                    child.kill()
                    child.wait(timeout=2)

    @unittest.skipUnless(sys.platform.startswith('linux'), 'Linux SIGINT qualification')
    def test_linux_direct_child_responds_to_sigint_without_hard_kill(self):
        import fcntl
        import pty
        import termios

        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            binary = root / 'child.py'
            binary.write_text('#!/usr/bin/env python3\nimport time\ntime.sleep(30)\n')
            binary.chmod(0o755)
            run = root / 'run'
            run.mkdir()
            handoff = root / 'frontend-handoff.json'
            master, slave = pty.openpty()

            def terminal_setup():
                os.setsid()
                fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

            child, _handoff, _state = rd.spawn_direct_unix(
                binary=binary, root=root, env=os.environ.copy(), slave=slave,
                terminal_setup=terminal_setup, run_dir=run, handoff_path=handoff,
                frontend='tui', n=1, workload='active', warmup_s=30, observe_s=120,
            )
            os.close(slave)
            try:
                child.send_signal(signal.SIGINT)
                child.wait(timeout=2)
                self.assertNotEqual(child.returncode, -signal.SIGKILL)
            finally:
                os.close(master)
                if child.poll() is None:
                    child.kill()
                    child.wait(timeout=2)

    @unittest.skipUnless(sys.platform.startswith('linux'), 'Linux pending-signal qualification')
    def test_linux_pending_parent_stop_runs_only_after_durable_handoff(self):
        harness = textwrap.dedent(r'''
            import fcntl, json, os, pathlib, pty, signal, subprocess, sys, termios, time
            sys.path.insert(0, os.environ['MEMORY_MODULE_ROOT'])
            import run_diagnostic as rd

            delivered = int(sys.argv[1])
            root = pathlib.Path(sys.argv[2])
            binary = root / 'child.py'
            binary.write_text('#!/usr/bin/env python3\nimport time\ntime.sleep(30)\n')
            binary.chmod(0o755)
            run = root / 'run'
            run.mkdir()
            handoff = root / 'frontend-handoff.json'
            inside_popen = root / 'inside-popen'
            master, slave = pty.openpty()

            sender_code = (
                'import os,pathlib,sys,time\n'
                'p=pathlib.Path(sys.argv[1]); d=time.monotonic()+3\n'
                'while not p.exists() and time.monotonic()<d:\n'
                '    time.sleep(.005)\n'
                'if not p.exists(): sys.exit(2)\n'
                'os.kill(int(sys.argv[2]),int(sys.argv[3]))'
            )
            sender = subprocess.Popen([
                sys.executable, '-c', sender_code, str(inside_popen),
                str(os.getpid()), str(delivered)])

            def terminal_setup():
                os.setsid()
                fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
                inside_popen.write_text('blocked-parent')
                time.sleep(.2)

            child = None
            try:
                child, _handoff, state = rd.spawn_direct_unix(
                    binary=binary, root=root, env=os.environ.copy(), slave=slave,
                    terminal_setup=terminal_setup, run_dir=run, handoff_path=handoff,
                    frontend='tui', n=1, workload='active', warmup_s=30,
                    observe_s=120)
                sender.wait(timeout=3)
                rc = child.wait(timeout=3)
                print(json.dumps({
                    'stop_requested': state['stop_requested'],
                    'handoff_state': json.loads(handoff.read_text())['state'],
                    'child_exit': rc,
                }))
            finally:
                os.close(slave)
                os.close(master)
                if sender.poll() is None:
                    sender.kill(); sender.wait()
                if child is not None and child.poll() is None:
                    child.kill(); child.wait()
        ''')
        for delivered in (signal.SIGTERM, signal.SIGINT):
            with self.subTest(delivered=delivered), tempfile.TemporaryDirectory() as tmp:
                env = os.environ.copy()
                env['MEMORY_MODULE_ROOT'] = str(ROOT)
                completed = subprocess.run(
                    [sys.executable, '-c', harness, str(delivered), tmp],
                    capture_output=True, text=True, timeout=10, env=env)
                self.assertEqual(completed.returncode, 0, completed.stderr)
                result = json.loads(completed.stdout.splitlines()[-1])
                self.assertTrue(result['stop_requested'])
                self.assertEqual(result['handoff_state'], 'spawned')
                self.assertNotEqual(result['child_exit'], 0)

    def test_capture_frontend_bound_preserves_960_second_live_budget(self):
        self.assertEqual(rd._CAPTURE_FRONTEND_MAX_WALL_S, 960)

    def test_capture_frontend_timeout_is_failed_even_when_child_exits_zero(self):
        class Child:
            def __init__(self):
                self.calls = []

            def wait(self, *, timeout):
                self.calls.append(('wait', timeout))
                if len(self.calls) == 1:
                    raise subprocess.TimeoutExpired(['frontend'], timeout)
                return 0

            def terminate(self):
                self.calls.append(('terminate',))

            def kill(self):
                self.calls.append(('kill',))

        child = Child()
        rc, timed_out = rd.wait_for_capture_frontend(child)
        self.assertEqual(rc, 0)
        self.assertTrue(timed_out)
        self.assertEqual(child.calls, [('wait', 960), ('terminate',), ('wait', 15)])

    def test_help_survives_windows_redirected_output_encoding(self):
        proc = subprocess.run(
            [sys.executable, str(SCRIPT), '--help'],
            cwd=ROOT, capture_output=True,
            env={**os.environ, 'PYTHONIOENCODING': 'cp1252'},
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn(b'--responsiveness-fine', proc.stdout)

    def test_accepts_n16_in_help_choices(self):
        proc = run_cli("--help")
        self.assertEqual(proc.returncode, 0)
        self.assertIn("{1,16,32,128}", proc.stdout)

    def test_rejects_invalid_n_without_launch(self):
        proc = run_cli("panel", "2", "idle")
        self.assertNotEqual(proc.returncode, 0)
        self.assertIn("invalid choice", (proc.stderr + proc.stdout).lower())

    def test_rejects_panel_flags_on_tui(self):
        for flag in ("--single-renderer", "--focused-one", "--focused-background"):
            proc = run_cli("tui", "16", "idle", flag)
            self.assertNotEqual(proc.returncode, 0, flag)
            err = proc.stderr + proc.stdout
            self.assertIn("panel", err.lower(), err)

    def test_rejects_conflicting_render_flags(self):
        proc = run_cli("panel", "16", "idle", "--single-renderer", "--focused-one")
        self.assertNotEqual(proc.returncode, 0)
        self.assertIn("mutually exclusive", (proc.stderr + proc.stdout).lower())

    def test_nav_captures_requires_one_draw_panel_mode(self):
        proc = run_cli("panel", "16", "idle", "--nav-captures")
        self.assertNotEqual(proc.returncode, 0)
        proc = run_cli("panel", "16", "idle", "--nav-captures", "--focused-background")
        self.assertNotEqual(proc.returncode, 0)
        # Valid combos still need a binary and would launch — only check parser path via import.
        import run_diagnostic as rd

        p = rd.build_parser()
        a = p.parse_args(["panel", "16", "idle", "--nav-captures", "--single-renderer"])
        rd.validate_args(a, p)
        a = p.parse_args(["panel", "16", "idle", "--nav-captures", "--focused-one"])
        rd.validate_args(a, p)

    def test_nav_captures_allows_tui_data_only(self):
        import run_diagnostic as rd

        p = rd.build_parser()
        a = p.parse_args(["tui", "16", "active", "--nav-captures", "--headless"])
        rd.validate_args(a, p)
        self.assertTrue(a.nav_captures)
        self.assertEqual(a.frontend, "tui")
        env = rd.build_child_env(a, "/tmp/nav-tui-run", base_env={"PATH": "/usr/bin", "HOME": "/tmp"})
        self.assertEqual(env.get("BOT_NAV_CAPTURES"), "1")
        self.assertIn("captures", env.get("274BOT_SMOKE_DIR", ""))
        help_proc = run_cli("--help")
        self.assertIn("TUI", help_proc.stdout)
        self.assertIn("data-only", help_proc.stdout.lower())

    def test_requested_render_policy_metadata_names(self):
        import run_diagnostic as rd

        p = rd.build_parser()
        cases = [
            (["panel", "1", "idle"], "rotating-all", False),
            (["panel", "16", "idle", "--single-renderer"], "fixed-one", True),
            (["panel", "16", "idle", "--focused-one"], "focused-one", False),
            (["panel", "16", "idle", "--focused-background"], "focused-plus-background", False),
            (["tui", "16", "idle"], "none", False),
        ]
        for args, policy, single in cases:
            a = p.parse_args(args)
            rd.validate_args(a, p)
            self.assertEqual(rd.requested_render_policy(a), policy, args)
            self.assertEqual(a.single_renderer, single, args)

    def test_render_profile_flag_independent_and_scrubbed(self):
        import run_diagnostic as rd

        p = rd.build_parser()
        a = p.parse_args(["panel", "1", "idle", "--render-profile"])
        rd.validate_args(a, p)
        self.assertTrue(a.render_profile)
        a2 = p.parse_args(["panel", "1", "idle"])
        self.assertFalse(a2.render_profile)
        help_proc = run_cli("--help")
        self.assertIn("--render-profile", help_proc.stdout)

    def test_gpu_completion_profile_requires_panel_and_render_profile(self):
        import run_diagnostic as rd

        p = rd.build_parser()
        a = p.parse_args(
            ["panel", "1", "idle", "--render-profile", "--gpu-completion-profile"]
        )
        rd.validate_args(a, p)
        self.assertTrue(a.gpu_completion_profile)
        self.assertTrue(a.render_profile)
        bad_tui = run_cli("tui", "1", "idle", "--render-profile", "--gpu-completion-profile")
        self.assertNotEqual(bad_tui.returncode, 0)
        bad_no_rp = run_cli("panel", "1", "idle", "--gpu-completion-profile")
        self.assertNotEqual(bad_no_rp.returncode, 0)
        help_proc = run_cli("--help")
        self.assertIn("--gpu-completion-profile", help_proc.stdout)

    def test_responsiveness_profile_flag_independent_and_scrubbed(self):
        import run_diagnostic as rd

        p = rd.build_parser()
        a = p.parse_args(["panel", "1", "idle", "--responsiveness-profile"])
        rd.validate_args(a, p)
        self.assertTrue(a.responsiveness_profile)
        self.assertFalse(a.responsiveness_fine)
        a2 = p.parse_args(["tui", "1", "idle", "--responsiveness-profile"])
        rd.validate_args(a2, p)
        self.assertTrue(a2.responsiveness_profile)
        a3 = p.parse_args(["panel", "1", "idle"])
        self.assertFalse(a3.responsiveness_profile)
        help_proc = run_cli("--help")
        self.assertIn("--responsiveness-profile", help_proc.stdout)
        self.assertIn("--responsiveness-fine", help_proc.stdout)
        fine = p.parse_args(["panel", "1", "idle", "--responsiveness-profile", "--responsiveness-fine"])
        rd.validate_args(fine, p)
        self.assertTrue(fine.responsiveness_fine)
        bad = run_cli("panel", "1", "idle", "--responsiveness-fine")
        self.assertNotEqual(bad.returncode, 0)

    def test_failure_capture_flag_independent_of_diagnostics(self):
        import run_diagnostic as rd

        p = rd.build_parser()
        only = p.parse_args(["panel", "1", "idle", "--no-diagnostics", "--failure-capture"])
        rd.validate_args(only, p)
        self.assertTrue(only.failure_capture)
        self.assertTrue(only.no_diagnostics)
        both = p.parse_args(["panel", "16", "active", "--failure-capture"])
        rd.validate_args(both, p)
        self.assertTrue(both.failure_capture)
        self.assertFalse(both.no_diagnostics)
        off = p.parse_args(["tui", "1", "idle"])
        self.assertFalse(off.failure_capture)
        help_proc = run_cli("--help")
        self.assertIn("--failure-capture", help_proc.stdout)
        self.assertIn("--no-diagnostics", help_proc.stdout)
        self.assertIn("failure-only", help_proc.stdout)

    def test_failure_capture_env_scrubbed_unless_flag(self):
        """Inherited BOT_MEMORY_FAILURE_CAPTURE must not leak when CLI flag is off."""
        import run_diagnostic as rd

        p = rd.build_parser()
        a_off = p.parse_args(["panel", "1", "idle", "--no-diagnostics"])
        rd.validate_args(a_off, p)
        a_on = p.parse_args(["panel", "1", "idle", "--no-diagnostics", "--failure-capture"])
        rd.validate_args(a_on, p)
        polluted = {
            "BOT_MEMORY_FAILURE_CAPTURE": "1",
            "BOT_RESPONSIVENESS_FINE": "1",
            "PATH": "/usr/bin",
            "HOME": "/tmp",
        }
        env_off = rd.build_child_env(a_off, "/tmp/fc-off-run", base_env=polluted)
        self.assertNotIn("BOT_MEMORY_FAILURE_CAPTURE", env_off)
        self.assertNotIn("BOT_RESPONSIVENESS_FINE", env_off)
        self.assertEqual(env_off["BOT_MEMORY_DIAGNOSTICS"], "0")
        env_on = rd.build_child_env(a_on, "/tmp/fc-on-run", base_env=polluted)
        self.assertEqual(env_on.get("BOT_MEMORY_FAILURE_CAPTURE"), "1")
        self.assertNotIn("BOT_RESPONSIVENESS_FINE", env_on)
        self.assertEqual(env_on["BOT_MEMORY_DIAGNOSTICS"], "0")
        # Flag alone (diagnostics default on) remains legal.
        a_both = p.parse_args(["panel", "1", "idle", "--failure-capture"])
        rd.validate_args(a_both, p)
        env_both = rd.build_child_env(a_both, "/tmp/fc-both-run", base_env=polluted)
        self.assertEqual(env_both.get("BOT_MEMORY_FAILURE_CAPTURE"), "1")
        self.assertEqual(env_both["BOT_MEMORY_DIAGNOSTICS"], "1")

    def test_import_survives_missing_unix_tty_modules(self):
        """Panel import/validate must work when pty/fcntl/termios are absent."""
        code = r"""
import importlib
import sys
import types

BLOCK = {"pty", "fcntl", "termios"}

class Blocker:
    def find_spec(self, fullname, path=None, target=None):
        if fullname in BLOCK:
            raise ModuleNotFoundError(fullname)
        return None

# Drop cached modules so the blocker is meaningful.
for name in list(sys.modules):
    if name in BLOCK or name == "run_diagnostic" or name.startswith("run_diagnostic."):
        del sys.modules[name]

sys.meta_path.insert(0, Blocker())
import run_diagnostic as rd

# Top-level import must not pull Unix TTY stack.
assert "pty" not in sys.modules
assert "fcntl" not in sys.modules
assert "termios" not in sys.modules

p = rd.build_parser()
a = p.parse_args(["panel", "1", "idle", "--focused-one"])
rd.validate_args(a, p)
env = rd.build_child_env(
    a, "/tmp/panel-no-pty", base_env={"PATH": "/usr/bin", "HOME": "/tmp"}, platform="win32"
)
assert "RS2B0T" not in env or env.get("RS2B0T") != rd._DEFAULT_RS2B0T_MAC
assert env.get("BOT_MEMORY_RENDER_POLICY") == "focused-one"

# Real TUI terminal on forced win32 fails closed when native ConPTY
# cannot bind (this host is not win32, or API missing) — never silent headless.
try:
    tag = rd.require_terminal_transport(platform="win32")
except RuntimeError as err:
    msg = str(err).lower()
    assert "windows" in msg or "conpty" in msg
    assert "headless" in msg or "panel" in msg
else:
    # On real win32 the ConPTY helper loads successfully.
    assert tag[0] == "conpty", tag

try:
    rd.require_terminal_transport(platform="linux")
except RuntimeError as err:
    assert "pty" in str(err).lower() or "termios" in str(err).lower() or "fcntl" in str(err).lower()
else:
    raise SystemExit("expected missing-module RuntimeError on linux flag")

print("ok")
"""
        proc = subprocess.run(
            [sys.executable, "-c", code],
            cwd=ROOT,
            capture_output=True,
            text=True,
            env={**dict(**{k: v for k, v in __import__("os").environ.items()}), "PYTHONPATH": str(ROOT)},
        )
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertIn("ok", proc.stdout)

    def test_rs2b0t_default_mac_only_not_windows(self):
        import run_diagnostic as rd

        p = rd.build_parser()
        a = p.parse_args(["panel", "1", "idle"])
        base = {"PATH": "/usr/bin", "HOME": "/tmp"}
        env_mac = rd.build_child_env(a, "/tmp/r1", base_env=base, platform="darwin")
        self.assertEqual(env_mac.get("RS2B0T"), rd._DEFAULT_RS2B0T_MAC)
        env_win = rd.build_child_env(a, "/tmp/r2", base_env=dict(base), platform="win32")
        self.assertNotIn("RS2B0T", env_win)
        env_win_explicit = rd.build_child_env(
            a,
            "/tmp/r3",
            base_env={**base, "RS2B0T": r"C:\src\rs2b0t"},
            platform="win32",
        )
        self.assertEqual(env_win_explicit.get("RS2B0T"), r"C:\src\rs2b0t")

    def test_resolve_rs2b0t_commit_fail_closed_windows(self):
        import run_diagnostic as rd

        with self.assertRaises(ValueError) as ctx:
            rd.resolve_rs2b0t_commit({}, platform="win32")
        self.assertIn("RS2B0T", str(ctx.exception))
        self.assertNotIn("deadbeef", str(ctx.exception).lower())

        calls = []

        def fake_git(args):
            calls.append(args)
            return "abc123real"

        # Non-dir path must not call git with a fabricated SHA path silently OK.
        with self.assertRaises(ValueError):
            rd.resolve_rs2b0t_commit(
                {"RS2B0T": "/no/such/rs2b0t-checkout-xyz"},
                platform="win32",
                git_check_output=fake_git,
            )
        self.assertEqual(calls, [])

    def test_resolve_rs2b0t_commit_real_git_when_path_ok(self):
        import run_diagnostic as rd
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            subprocess.run(["git", "init"], cwd=root, check=True, capture_output=True)
            subprocess.run(
                ["git", "config", "user.email", "t@example.com"],
                cwd=root,
                check=True,
                capture_output=True,
            )
            subprocess.run(
                ["git", "config", "user.name", "t"],
                cwd=root,
                check=True,
                capture_output=True,
            )
            (root / "f").write_text("x\n")
            subprocess.run(["git", "add", "f"], cwd=root, check=True, capture_output=True)
            subprocess.run(
                ["git", "commit", "-m", "t"],
                cwd=root,
                check=True,
                capture_output=True,
            )
            head = subprocess.check_output(
                ["git", "-C", str(root), "rev-parse", "HEAD"], text=True
            ).strip()
            got = rd.resolve_rs2b0t_commit({"RS2B0T": str(root)}, platform="win32")
            self.assertEqual(got, head)

    def test_catalog_path_uses_operator_home_windows_profile(self):
        import run_diagnostic as rd

        path = rd.catalog_path_for_env(
            {"USERPROFILE": r"C:\Users\BotTest", "PATH": "x"},
            windows=True,
        )
        self.assertTrue(str(path).replace("\\", "/").endswith(".274bot/js-scripts.json"))
        # Empty explicit HOME must not silently use USERPROFILE (Rust parity).
        path_empty = rd.catalog_path_for_env(
            {"HOME": "", "USERPROFILE": r"C:\Users\BotTest"},
            windows=True,
        )
        # bot_home "." → resolve relative catalog
        self.assertTrue(str(path_empty).endswith(str(pathlib.Path(".274bot") / "js-scripts.json")) or
                        ".274bot" in str(path_empty))

    def test_unix_terminal_transport_loads_when_modules_present(self):
        import run_diagnostic as rd

        if sys.platform == "win32":
            self.skipTest("host is win32")
        tag, fcntl, pty, termios = rd.require_terminal_transport()
        self.assertEqual(tag, "unix")
        self.assertTrue(hasattr(pty, "openpty"))
        self.assertTrue(hasattr(fcntl, "ioctl"))
        self.assertTrue(hasattr(termios, "TIOCSWINSZ"))

    def test_win32_terminal_transport_selects_conpty_helper(self):
        import run_diagnostic as rd
        import windows_conpty as wcp

        if sys.platform == "win32":
            tag, mod = rd.require_terminal_transport(platform="win32")
            self.assertEqual(tag, "conpty")
            self.assertIs(mod, wcp)
        else:
            # Off Windows, forced platform still must not claim unix PTY.
            with self.assertRaises(RuntimeError) as ctx:
                rd.require_terminal_transport(platform="win32")
            msg = str(ctx.exception).lower()
            self.assertTrue("conpty" in msg or "windows" in msg)
            self.assertTrue("headless" in msg or "panel" in msg)

    def test_cpu_fallback_flag_panel_only_and_metadata_names(self):
        import run_diagnostic as rd

        p = rd.build_parser()
        help_proc = run_cli("--help")
        self.assertIn("--cpu-fallback", help_proc.stdout)

        default = p.parse_args(["panel", "1", "idle"])
        rd.validate_args(default, p)
        self.assertFalse(default.cpu_fallback)
        self.assertEqual(rd.requested_backend(default), "gpu")

        on = p.parse_args(["panel", "16", "idle", "--cpu-fallback", "--focused-one"])
        rd.validate_args(on, p)
        self.assertTrue(on.cpu_fallback)
        self.assertEqual(rd.requested_backend(on), "cpu_fallback")
        self.assertEqual(rd.requested_render_policy(on), "focused-one")

        tui = p.parse_args(["tui", "1", "idle"])
        rd.validate_args(tui, p)
        self.assertEqual(rd.requested_backend(tui), "none")

        bad_tui = run_cli("tui", "1", "idle", "--cpu-fallback")
        self.assertNotEqual(bad_tui.returncode, 0)
        self.assertIn("panel", (bad_tui.stderr + bad_tui.stdout).lower())

        bad_gpu = run_cli(
            "panel", "1", "idle",
            "--render-profile", "--gpu-completion-profile", "--cpu-fallback",
        )
        self.assertNotEqual(bad_gpu.returncode, 0)
        err = (bad_gpu.stderr + bad_gpu.stdout).lower()
        self.assertTrue("cpu-fallback" in err or "gpu-completion" in err, err)

        # Nav captures + focused-one still legal with CPU fallback.
        nav = p.parse_args(
            ["panel", "16", "idle", "--nav-captures", "--focused-one", "--cpu-fallback"]
        )
        rd.validate_args(nav, p)
        self.assertTrue(nav.cpu_fallback)
        self.assertTrue(nav.nav_captures)

    def test_cpu_fallback_env_scrub_default_and_explicit_set(self):
        """Inherited BOT_CPU is scrubbed; BOT_CPU=1 only when --cpu-fallback."""
        import run_diagnostic as rd

        p = rd.build_parser()
        a_off = p.parse_args(["panel", "1", "idle"])
        rd.validate_args(a_off, p)
        a_on = p.parse_args(["panel", "1", "idle", "--cpu-fallback"])
        rd.validate_args(a_on, p)
        polluted = {
            "BOT_CPU": "1",
            "BOT_DEBUG": "1",
            "PATH": "/usr/bin",
            "HOME": "/tmp",
        }
        env_off = rd.build_child_env(a_off, "/tmp/cpu-off-run", base_env=polluted)
        self.assertNotIn("BOT_CPU", env_off)
        self.assertNotIn("BOT_DEBUG", env_off)
        env_on = rd.build_child_env(a_on, "/tmp/cpu-on-run", base_env=polluted)
        self.assertEqual(env_on.get("BOT_CPU"), "1")
        self.assertNotIn("BOT_DEBUG", env_on)
        # Explicit flag still sets after scrub even when parent lacked BOT_CPU.
        clean = {"PATH": "/usr/bin", "HOME": "/tmp"}
        env_clean_on = rd.build_child_env(a_on, "/tmp/cpu-clean-on", base_env=clean)
        self.assertEqual(env_clean_on.get("BOT_CPU"), "1")
        env_clean_off = rd.build_child_env(a_off, "/tmp/cpu-clean-off", base_env=clean)
        self.assertNotIn("BOT_CPU", env_clean_off)

    def test_heaptrack_option_is_n1_tui_only_and_child_env_is_explicit(self):
        import run_diagnostic as rd
        p = rd.build_parser()
        valid = p.parse_args(["tui", "1", "active", "--no-diagnostics", "--sustain",
                              "--heaptrack-output", "/tmp/capture"])
        rd.validate_args(valid, p)
        valid.heaptrack_capture = {
            "output": {"raw": "/tmp/capture/alloc.raw"},
            "preload": {"path": "/usr/lib/heaptrack/libheaptrack_preload.so"},
        }
        env = rd.build_child_env(valid, "/tmp/run", base_env={"PATH": "/usr/bin"})
        self.assertEqual(env["LD_PRELOAD"], "/usr/lib/heaptrack/libheaptrack_preload.so")
        self.assertEqual(env["DUMP_HEAPTRACK_OUTPUT"], "/tmp/capture/alloc.raw")
        for args in (("tui", "16", "active"), ("panel", "1", "active"),
                     ("tui", "1", "idle")):
            bad = p.parse_args([*args, "--no-diagnostics", "--sustain", "--heaptrack-output", "/tmp/c"])
            with self.assertRaises(SystemExit):
                rd.validate_args(bad, p)

    def test_heaptrack_option_rejects_other_probes(self):
        import run_diagnostic as rd
        p = rd.build_parser()
        bad = p.parse_args(["tui", "1", "active", "--no-diagnostics", "--sustain",
                            "--heaptrack-output", "/tmp/c", "--stack-logging"])
        with self.assertRaises(SystemExit):
            rd.validate_args(bad, p)


if __name__ == "__main__":
    unittest.main()
