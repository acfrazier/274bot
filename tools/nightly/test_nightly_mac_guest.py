#!/usr/bin/env python3
"""Tests for the host controller that runs the Mac nightly in Tart."""

import contextlib
import importlib.util
import io
import json
import os
import shutil
import signal
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("nightly_mac_guest", HERE / "nightly-mac-guest.py")
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load nightly-mac-guest.py")
nightly = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = nightly
SPEC.loader.exec_module(nightly)


_THROWAWAY_ROOT = Path(tempfile.gettempdir()).resolve()
_real_maybe_self_update = nightly.maybe_self_update


def _throwaway_only_maybe_self_update(repo, *args, **kwargs):
    """The test-only self-update gate: only throwaway repos may be updated.

    Every `repo` below is a directory the test itself created under the
    system temp dir. The real checkout is never updated: `maybe_self_update`
    force-checks out `origin/dev`, which would discard uncommitted work.
    Any other caller fails loudly before git runs.
    """
    if not Path(repo).resolve().is_relative_to(_THROWAWAY_ROOT):
        raise AssertionError(
            f"refusing to self-update {repo}: tests only update throwaway repos "
            f"they create under {_THROWAWAY_ROOT}")
    return _real_maybe_self_update(repo, *args, **kwargs)


nightly.maybe_self_update = _throwaway_only_maybe_self_update


def _record_real_checkout_state():
    """HEAD, branch and worktree state of the real checkout, if any.

    `None` when this file does not run from a git checkout (the self-check
    runs the candidate copy from a bare temporary directory).
    """
    repo = HERE.parent.parent
    try:
        head = subprocess.run(["git", "-C", str(repo), "rev-parse", "HEAD"],
                              capture_output=True, text=True, check=True).stdout.strip()
        branch = subprocess.run(["git", "-C", str(repo), "branch", "--show-current"],
                                capture_output=True, text=True, check=True).stdout.strip()
        porcelain = subprocess.run(["git", "-C", str(repo), "status", "--porcelain"],
                                   capture_output=True, text=True, check=True).stdout
    except (subprocess.SubprocessError, OSError):
        return None
    return (head, branch, porcelain)


_REAL_CHECKOUT_STATE = _record_real_checkout_state()


def tearDownModule():
    """No test may touch the real checkout this file runs from.

    A test that reaches `maybe_self_update` (or `host_main()`) with the real
    runner path force-checks out `origin/dev` and discards uncommitted edits
    (NIGHTLY-FROM-GITHUB-1026 finding 1). The gate above makes that
    impossible; this teardown fails the suite if it ever happens anyway.
    """
    if _REAL_CHECKOUT_STATE is None:
        return
    after = _record_real_checkout_state()
    if after != _REAL_CHECKOUT_STATE:
        raise AssertionError(
            f"a test touched the real checkout {HERE.parent.parent}: "
            f"before={_REAL_CHECKOUT_STATE!r} after={after!r}")


def git(*args, cwd=None):
    return subprocess.run(
        ["git", *args], cwd=cwd, check=True, capture_output=True, text=True
    ).stdout.strip()


def make_source(root: Path) -> Path:
    source = root / "source"
    subprocess.run(["git", "init", "-q", str(source)], check=True)
    git("symbolic-ref", "HEAD", "refs/heads/dev", cwd=source)
    (source / "source.txt").write_text("source\n", encoding="utf-8")
    git("add", ".", cwd=source)
    git("-c", "user.name=nightly-test", "-c", "user.email=nightly@example.invalid",
        "commit", "-qm", "initial", cwd=source)
    git("branch", "release/0.2.6", cwd=source)
    return source


class SourcePinTests(unittest.TestCase):
    def test_defaults_to_configured_release_then_dev(self):
        # Parses the branch file format (comments and blank lines skipped, order kept), not the
        # current release line, so the release roll-over does not need a test edit.
        self.assertIsNone(nightly.parse_args([]).branch)
        with tempfile.TemporaryDirectory() as temporary:
            conf = Path(temporary) / "branches.conf"
            conf.write_text("# fix line first\n\nrelease/9.9.9\n  # indented comment\ndev\n", encoding="utf-8")
            self.assertEqual(nightly.read_branches(conf, None), ["release/9.9.9", "dev"])
        self.assertEqual(nightly.read_branches(HERE / "branches.conf", None)[-1], "dev")

    def test_pins_each_branch_to_the_fetched_commit(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = make_source(root)
            input_root = root / "input"
            input_root.mkdir()

            pins = nightly.pin_sources(str(source), ["release/0.2.6", "dev"], input_root)

            self.assertEqual(set(pins), {"release/0.2.6", "dev"})
            for branch, commit in pins.items():
                self.assertEqual(
                    git("--git-dir", str(input_root / "source.git"), "rev-parse",
                        f"refs/heads/{branch}"),
                    commit,
                )
            metadata = json.loads((input_root / "branch-pins.json").read_text(encoding="utf-8"))
            self.assertEqual(metadata["branches"], pins)


class EngineStagingTests(unittest.TestCase):
    def test_preserves_runtime_map_artifacts_but_not_disposable_engine_data(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source"
            entries = (
                "engine/data/pack/.cache/maps-server.zip",
                "engine/.cache/unrelated",
                "engine/node_modules/package",
                "engine/db.sqlite",
                "engine/data/players/account",
                "runtime/pids/engine.pid",
            )
            for relative in entries:
                path = source / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(relative, encoding="utf-8")
            destination = root / "staged"

            nightly.copy_tree(source, destination, kind="engine")

            self.assertEqual((destination / entries[0]).read_text(encoding="utf-8"), entries[0])
            for relative in entries[1:]:
                self.assertFalse((destination / relative).exists(), relative)


class GuestBootstrapTests(unittest.TestCase):
    def test_bootstrap_copies_staged_submodule_mirrors_into_guest(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            input_root = root / "input"
            input_root.mkdir()
            for engine in ("engine-a", "engine-q"):
                lock = input_root / engine / "engine/package-lock.json"
                lock.parent.mkdir(parents=True)
                lock.write_text("{}\n", encoding="utf-8")
            mirror = input_root / "submodules/vendor-fr-client-rust.git"
            mirror.mkdir(parents=True)
            (mirror / "HEAD").write_text("ref: refs/heads/main\n", encoding="utf-8")
            (input_root / "nav-snapshot").mkdir()
            isohome = input_root / "isohome"
            isohome.write_text("#!/bin/sh\nexec \"$@\"\n", encoding="utf-8")
            isohome.chmod(0o555)
            guest_root = root / "guest/nightly"
            guest_home = root / "guest"
            python = root / "python"
            cargo_home = root / "cargo"
            rustup_home = root / "rustup"
            node_bin = root / "node/bin"

            def successful_command(args, **_kwargs):
                return subprocess.CompletedProcess(args, 0, stdout="tool version\n", stderr="")

            with mock.patch.object(nightly.platform, "system", return_value="Darwin"), \
                    mock.patch.object(nightly.platform, "machine", return_value="arm64"), \
                    mock.patch.object(nightly, "install_python", return_value=python), \
                    mock.patch.object(nightly, "install_node", return_value=(node_bin, "26.12.1")), \
                    mock.patch.object(nightly, "install_rust", return_value=(cargo_home, rustup_home)), \
                    mock.patch.object(nightly, "run_command", side_effect=successful_command), \
                    contextlib.redirect_stdout(io.StringIO()):
                tools = nightly.guest_bootstrap(input_root, guest_root, guest_home)

            self.assertEqual(tools["node_version"], "26.12.1")
            guest_isohome = guest_home / "toolchains/bin/isohome"
            self.assertEqual(tools["isohome"], str(guest_isohome))
            self.assertEqual(guest_isohome.read_text(encoding="utf-8"), "#!/bin/sh\nexec \"$@\"\n")
            self.assertEqual(guest_isohome.stat().st_mode & 0o777, 0o555)
            self.assertEqual(
                (guest_root / "mirror/vendor-fr-client-rust.git/HEAD").read_text(encoding="utf-8"),
                "ref: refs/heads/main\n",
            )
            self.assertTrue((guest_root / "engine-pristine/engine/package-lock.json").is_file())
            self.assertTrue((guest_root / "engine-q-pristine/engine/package-lock.json").is_file())
            self.assertTrue((guest_root / "nav-snapshot").is_dir())

    def test_private_guest_copy_is_writable_but_shared_input_stays_readonly(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "input/engine"
            source.mkdir(parents=True)
            (source / "package-lock.json").write_text("{}\n", encoding="utf-8")
            executable = source / "start.sh"
            executable.write_text("#!/bin/sh\n", encoding="utf-8")
            executable.chmod(0o755)
            outside = root / "outside"
            outside.write_text("outside\n", encoding="utf-8")
            outside.chmod(0o444)
            (source / "link").symlink_to(outside)
            nightly.make_readonly(source)
            destination = root / "guest/engine"

            nightly.copy_guest_input(source, destination)

            (destination / "node_modules").mkdir()
            (destination / "package-lock.json").write_text("changed\n", encoding="utf-8")
            self.assertEqual((source / "package-lock.json").read_text(encoding="utf-8"), "{}\n")
            self.assertEqual(source.stat().st_mode & 0o200, 0)
            self.assertEqual((source / "package-lock.json").stat().st_mode & 0o200, 0)
            self.assertEqual(outside.stat().st_mode & 0o200, 0)
            self.assertTrue((destination / "link").is_symlink())
            self.assertTrue((destination / "start.sh").stat().st_mode & 0o100)


class RunArchiveTests(unittest.TestCase):
    def make_run(self, root: Path, name: str) -> Path:
        run = root / name
        run.mkdir(parents=True)
        (run / "summary.json").write_text('{"result":"PASS"}\n', encoding="utf-8")
        nested = run / "nested"
        nested.mkdir()
        (nested / "SHA256SUMS").write_text("nested file\n", encoding="utf-8")
        nightly.write_sha256sums(run)
        # The source runner writes its completion marker after its manifest.
        (run / "DONE").write_text("completed\n", encoding="utf-8")
        return run

    def test_export_and_publish_checksums_and_preserves_existing_runs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            guest_runs = root / "guest-runs"
            guest_runs.mkdir()
            self.make_run(guest_runs, "2026-10-23-dev-abcdef123-mac")
            outbox = root / "share/results"
            outbox.mkdir(parents=True)
            runs = root / "NIGHTLY"
            free_bytes = 200 * nightly.GIB

            archives = nightly.export_run_archives(guest_runs, outbox)
            published = nightly.publish_archives(outbox, runs, lambda: free_bytes, 120 * nightly.GIB)
            repeated = nightly.publish_archives(outbox, runs, lambda: free_bytes, 120 * nightly.GIB)

            self.assertEqual(len(archives), 1)
            self.assertEqual([path.name for path in published], ["2026-10-23-dev-abcdef123-mac"])
            self.assertEqual([path.name for path in repeated], ["2026-10-23-dev-abcdef123-mac-r2"])
            for destination in published + repeated:
                nightly.verify_sha256sums(destination)
                self.assertEqual((destination / "DONE").read_text(encoding="utf-8"), "completed\n")

    def test_only_the_root_completion_marker_is_outside_source_manifest(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            run = self.make_run(root, "run")
            nightly.verify_sha256sums(run)
            original = (run / "SHA256SUMS").read_text(encoding="utf-8")
            (run / "nested/DONE").write_text("unlisted data\n", encoding="utf-8")

            with self.assertRaisesRegex(nightly.NightlyError, "coverage mismatch"):
                nightly.verify_sha256sums(run)
            self.assertEqual((run / "SHA256SUMS").read_text(encoding="utf-8"), original)

    def test_publish_rejects_unlisted_or_changed_run_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            guest_runs = root / "guest-runs"
            guest_runs.mkdir()
            run = guest_runs / "2026-10-23-dev-abcdef123-mac"
            run.mkdir()
            (run / "summary.json").write_text("original\n", encoding="utf-8")
            nightly.write_sha256sums(run)
            (run / "summary.json").write_text("changed\n", encoding="utf-8")
            outbox = root / "share/results"
            outbox.mkdir(parents=True)
            nightly.export_run_archives(guest_runs, outbox)

            with self.assertRaises(nightly.NightlyError):
                nightly.publish_archives(outbox, root / "NIGHTLY", lambda: 200 * nightly.GIB,
                                         120 * nightly.GIB)
            self.assertFalse((root / "NIGHTLY/2026-10-23-dev-abcdef123-mac").exists())

    def test_publish_rejects_archive_traversal(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            outbox = root / "results"
            outbox.mkdir()
            archive = outbox / "evil.tar"
            with tarfile.open(archive, "w") as bundle:
                member = tarfile.TarInfo("../outside")
                member.size = 0
                bundle.addfile(member)

            with self.assertRaisesRegex(nightly.NightlyError, "unsafe archive member"):
                nightly.publish_archives(outbox, root / "NIGHTLY", lambda: 200 * nightly.GIB,
                                         120 * nightly.GIB)
            self.assertFalse((root / "outside").exists())


class TartStartTests(unittest.TestCase):
    def test_start_uses_the_sized_baked_base_clone(self):
        settings = SimpleNamespace(
            python="python3", vm_tool=Path("/tmp/mac_vm.py"), base="274bot-mac-base"
        )
        session = {
            "id": "nightly-test", "vm": "274bot-nightly-test", "ip": "192.0.2.10",
            "guest_home": "/Users/bot/headed/nightly-test", "share": "/tmp/nightly-test/share",
        }
        process = mock.Mock(returncode=0)
        module = mock.Mock()
        module.VM_PREFIX = "274bot-mac-"

        def load_session(session_id):
            session["id"] = session_id
            session["vm"] = module.VM_PREFIX + session_id
            return session

        module.load_session.side_effect = load_session

        def create_start_process(command, **_kwargs):
            session_id = command[command.index("--session-id") + 1]
            load_session(session_id)
            process.communicate.return_value = (json.dumps(session), "")
            return process

        vm = nightly.TartVM(settings, lambda: 200 * nightly.GIB)

        with mock.patch.object(nightly.subprocess, "Popen", side_effect=create_start_process) as popen, \
                mock.patch.object(nightly, "load_vm_module", return_value=module):
            observed = vm.start()

        command = popen.call_args.args[0]
        self.assertEqual(observed, session)
        self.assertEqual(command[command.index("--cpu") + 1], "8")
        self.assertEqual(command[command.index("--memory") + 1], "24576")
        self.assertEqual(command[command.index("--disk-size") + 1], "160")
        self.assertEqual(command[command.index("--base") + 1], "274bot-mac-base")
        self.assertIn("--skip-tcc-bootstrap", command)

        interrupted = mock.Mock(returncode=None, pid=32145)
        interrupted.poll.return_value = None
        interrupted.wait.return_value = 0
        interrupted.communicate.side_effect = SystemExit(143)
        interrupted_vm = nightly.TartVM(settings, lambda: 200 * nightly.GIB)
        with mock.patch.object(nightly.subprocess, "Popen", return_value=interrupted) as popen, \
                mock.patch.object(nightly, "load_vm_module", return_value=module):
            with self.assertRaises(SystemExit):
                interrupted_vm.start()

        session_id = interrupted_vm.session_id
        with self.subTest("start process and clone name are recorded"):
            self.assertTrue(session_id)
            self.assertEqual(
                getattr(interrupted_vm, "vm_name", None),
                "274bot-mac-" + str(session_id),
            )
            self.assertIs(getattr(interrupted_vm, "start_process", None), interrupted)
        with self.subTest("owned start process is terminated"):
            terminate_start = getattr(interrupted_vm, "terminate_start", None)
            self.assertTrue(callable(terminate_start))
            if callable(terminate_start):
                with mock.patch.object(nightly.os, "killpg") as kill_group:
                    self.assertTrue(terminate_start())
                kill_group.assert_called_once_with(interrupted.pid, nightly.signal.SIGTERM)
                interrupted.wait.assert_called_once_with(timeout=15)
        with self.subTest("recorded clone is stopped after interrupted start"):
            with mock.patch.object(
                    nightly, "run_command",
                    return_value=subprocess.CompletedProcess([], 0, "", "")) as run_command:
                interrupted_vm.stop()
            self.assertIn(str(session_id), run_command.call_args.args[0])
        self.assertTrue(popen.call_args.kwargs["start_new_session"])
        start_command = popen.call_args.args[0]
        self.assertEqual(start_command[start_command.index("--session-id") + 1], str(session_id))

    def test_start_child_receives_sigterm_with_default_signal_mask(self):
        with tempfile.TemporaryDirectory() as temporary:
            helper = Path(temporary) / "fake-mac-vm.py"
            helper.write_text(
                "VM_PREFIX = '274bot-mac-'\n"
                "import time\n"
                "if __name__ == '__main__':\n"
                "    while True:\n"
                "        time.sleep(1)\n",
                encoding="utf-8",
            )
            settings = SimpleNamespace(
                python=sys.executable, vm_tool=helper, base="test-base"
            )
            vm = nightly.TartVM(settings, lambda: 200 * nightly.GIB)
            child_ready = threading.Event()
            children = []
            original_popen = subprocess.Popen

            def spawn(*args, **kwargs):
                child = original_popen(*args, **kwargs)
                children.append(child)
                child_ready.set()
                return child

            errors = []

            def start():
                try:
                    vm.start()
                except BaseException as error:
                    errors.append(error)

            worker = threading.Thread(target=start)
            try:
                with mock.patch.object(nightly.subprocess, "Popen", side_effect=spawn):
                    worker.start()
                    self.assertTrue(child_ready.wait(timeout=5))
                    deadline = time.monotonic() + 5
                    while vm.start_process is None and time.monotonic() < deadline:
                        time.sleep(0.01)
                    process = vm.start_process
                    self.assertIsNotNone(process)
                    started = time.monotonic()
                    self.assertTrue(vm.terminate_start())
                    elapsed = time.monotonic() - started
                worker.join(timeout=5)
                self.assertFalse(worker.is_alive())
                self.assertLess(elapsed, 5)
                self.assertEqual(process.returncode, -signal.SIGTERM)
                self.assertIsInstance(errors[0], nightly.NightlyError)
            finally:
                if vm.start_process is not None:
                    vm.terminate_start()
                elif children and children[0].poll() is None:
                    os.killpg(children[0].pid, signal.SIGKILL)
                    children[0].wait(timeout=5)
                worker.join(timeout=20)

    def test_quiet_lock_check_stops_start_wait(self):
        settings = SimpleNamespace(
            python="python3", vm_tool=Path("/tmp/mac_vm.py"), base="test-base"
        )
        module = SimpleNamespace(VM_PREFIX="274bot-mac-", load_session=mock.Mock())
        process = mock.Mock(returncode=None, pid=32146)
        process.communicate.side_effect = subprocess.TimeoutExpired(["python3"], 5)
        vm = nightly.TartVM(settings, lambda: 200 * nightly.GIB)
        quiet_lock_check = mock.Mock(return_value=True)

        with mock.patch.object(nightly, "load_vm_module", return_value=module), \
                mock.patch.object(nightly.subprocess, "Popen", return_value=process), \
                mock.patch.object(nightly.os, "killpg") as kill_group:
            self.assertIsNone(vm.start(quiet_lock_check))

        quiet_lock_check.assert_called_once_with()
        kill_group.assert_called_once_with(process.pid, signal.SIGTERM)
        process.wait.assert_called_once_with(timeout=15)
        module.load_session.assert_not_called()
        self.assertIsNone(vm.start_process)
        self.assertIsNotNone(vm.session_id)


class TartSupervisorTests(unittest.TestCase):
    def test_retains_fast_failure_log_outside_the_disposable_guest(self):
        vm = nightly.TartVM(SimpleNamespace(), lambda: 200 * nightly.GIB)
        vm.session = {"guest_home": "/Users/bot/headed/nightly-test"}
        vm.supervisor_path = Path("/tmp/supervisor.sh")
        guest = mock.Mock(side_effect=["", "__DONE__1\nsetup failed\n"])
        output = io.StringIO()

        with mock.patch.object(vm, "guest_sh", guest), contextlib.redirect_stdout(output):
            code = vm.run_supervised(["/usr/bin/python3", "bootstrap.py"], "bootstrap",
                                     lambda: False)

        self.assertEqual(code, 1)
        self.assertIn("setup failed", output.getvalue())
        shared_log = nightly.shell_quote(f"{nightly.GUEST_SHARE}/logs/bootstrap.log")
        self.assertIn(shared_log, guest.call_args_list[0].args[0])
        self.assertIn(shared_log, guest.call_args_list[1].args[0])
        self.assertIsNone(vm.active_pid_file)
        self.assertIsNone(vm.active_status_file)


class FakeVM:
    def __init__(
        self, settings, _disk_check, *, fail_start=False, quiet_during_start=False,
        quiet_after_start=False
    ):
        self.settings = settings
        self.fail_start = fail_start
        self.quiet_during_start = quiet_during_start
        self.quiet_after_start = quiet_after_start
        self.session = None
        self.session_id = None
        self.supervisor_path = None
        self.staged_isohome = None
        self.staged_isohome_mode = None
        self.run_calls = []
        self.stop_calls = 0
        self.start_calls = 0

    def start(self, quiet_lock_check=None):
        self.start_calls += 1
        self.session_id = "nightly-test"
        if self.fail_start:
            raise nightly.NightlyError("simulated start failure after clone creation")
        if self.quiet_during_start:
            self.settings.quiet_lock.touch()
            if quiet_lock_check is not None and quiet_lock_check():
                return
        session_dir = self.settings.root.parent / "sessions" / self.session_id
        share = session_dir / "share"
        share.mkdir(parents=True, exist_ok=True)
        guest_home = self.settings.root.parent / "guest-home"
        guest_home.mkdir(parents=True, exist_ok=True)
        self.session = {
            "id": self.session_id, "vm": "fake-vm", "ip": "192.0.2.10",
            "guest_home": str(guest_home), "share": str(share),
        }
        if self.quiet_after_start:
            self.settings.quiet_lock.touch()

    def guest_sh(self, command, *, timeout=45):
        del timeout
        if command.startswith("cat "):
            return json.dumps({
                "python": "/usr/bin/python3", "cargo_home": "/tmp/cargo",
                "rustup_home": "/tmp/rustup", "node_bin": "/tmp/node/bin",
                "home": "/tmp/home", "tmp": "/tmp/nightly-tmp",
                "isohome": "/Users/bot/headed/nightly-test/toolchains/bin/isohome",
            })
        if "--export-runs" in command:
            guest_runs = Path(self.session["guest_home"]) / "nightly/runs"
            guest_runs.mkdir(parents=True, exist_ok=True)
            nightly.export_run_archives(guest_runs, Path(self.session["share"]) / "results")
        return ""

    def run_supervised(self, command, key, _progress):
        if key == "bootstrap":
            helper = Path(self.session["share"]) / "input/isohome"
            self.staged_isohome = helper.read_text(encoding="utf-8")
            self.staged_isohome_mode = helper.stat().st_mode & 0o777
            run = Path(self.session["guest_home"]) / "nightly/runs/2026-10-23-dev-abcdef123-mac"
            run.mkdir(parents=True, exist_ok=True)
            (run / "summary.json").write_text('{"result":"PASS"}\n', encoding="utf-8")
            nightly.write_sha256sums(run)
        self.run_calls.append((list(command), key))
        return 0

    def terminate_start(self):
        return True

    def terminate_active(self):
        return True

    def stop(self):
        self.stop_calls += 1
        self.session = None
        self.session_id = None


class NightlyOrchestrationTests(unittest.TestCase):
    def make_settings(self, root: Path):
        runner = root / "runner"
        (runner / "tools/nightly").mkdir(parents=True)
        for name in ("run_nightly.py", "nightly-mac-guest.py"):
            (runner / "tools/nightly" / name).write_text("# test runner\n", encoding="utf-8")
        (runner / "tools/nightly/branches.conf").write_text("release/0.2.6\ndev\n", encoding="utf-8")
        catalog = root / "catalog"
        catalog.mkdir()
        for engine, script in (("engine-a", "start-289.sh"), ("engine-q", "start-289q.sh")):
            (root / engine / "engine").mkdir(parents=True)
            (root / engine / "engine/package-lock.json").write_text("{}\n", encoding="utf-8")
            (root / engine / script).write_text("#!/bin/sh\n", encoding="utf-8")
        fixtures = root / "nightly-root/fixtures"
        fixtures.mkdir(parents=True)
        (fixtures / "274bot-289-v17.navpack").write_text("fixture\n", encoding="utf-8")
        (fixtures / "inspect.fb").write_text("inspect fixture\n", encoding="utf-8")
        isohome = root / "isohome"
        isohome.write_text("#!/bin/sh\nexec \"$@\"\n", encoding="utf-8")
        isohome.chmod(0o755)
        vm_tool = root / "mac_vm.py"
        vm_tool.write_text("# fake helper\n", encoding="utf-8")
        source = make_source(root)
        disk_root = root / "disk"
        disk_root.mkdir()
        return nightly.Settings({
            "NIGHTLY_MAC_RUNNER": str(runner),
            "NIGHTLY_MAC_ROOT": str(root / "nightly-root"),
            "NIGHTLY_MAC_RUNS": str(root / "NIGHTLY"),
            "NIGHTLY_MAC_CATALOG": str(catalog),
            "NIGHTLY_MAC_ENGINE_A": str(root / "engine-a"),
            "NIGHTLY_MAC_ENGINE_Q": str(root / "engine-q"),
            "NIGHTLY_MAC_QUIET_LOCK": str(root / "MAC-QUIET.lock"),
            "NIGHTLY_MAC_ISOHOME": str(isohome),
            "NIGHTLY_MAC_PYTHON": sys.executable,
            "NIGHTLY_MAC_VM_TOOL": str(vm_tool),
            "NIGHTLY_MAC_SOURCE_REPO": str(source),
            "NIGHTLY_MAC_DISK_ROOT": str(disk_root),
        })

    def test_runs_main_and_quest_for_each_branch_then_stops_and_publishes(self):
        with tempfile.TemporaryDirectory() as temporary:
            settings = self.make_settings(Path(temporary))
            created = []

            def vm_factory(*args):
                vm = FakeVM(*args)
                created.append(vm)
                return vm

            controller = nightly.NightlyOrchestrator(
                settings, nightly.read_branches(settings.runner / "tools/nightly/branches.conf", None),
                vm_factory=vm_factory
            )
            self.assertEqual(controller.run(), 0)
            vm = created[0]

            self.assertEqual(vm.stop_calls, 1)
            self.assertEqual(vm.staged_isohome, "#!/bin/sh\nexec \"$@\"\n")
            self.assertEqual(vm.staged_isohome_mode, 0o555)
            self.assertEqual(len(vm.run_calls), 5)
            self.assertEqual(
                [key for _command, key in vm.run_calls],
                ["bootstrap", "release-0.2.6-main", "release-0.2.6-quest", "dev-main", "dev-quest"],
            )
            jobs = {key: command for command, key in vm.run_calls[1:]}
            job_env = {}
            for key, command in jobs.items():
                env = {arg.split("=", 1)[0]: arg.split("=", 1)[1]
                       for arg in command[1:] if "=" in arg and arg.split("=", 1)[0].startswith("NIGHTLY_")}
                job_env[key] = env
                expected_mode = "remote" if key.endswith("-main") else "local"
                self.assertEqual(env["NIGHTLY_ENGINE"], expected_mode)
                self.assertEqual(env["NIGHTLY_FRESH_TREES"], "1")
            self.assertIn("engine-pristine", job_env["release-0.2.6-main"]["NIGHTLY_ENGINE_PRISTINE"])
            self.assertTrue(job_env["release-0.2.6-main"]["NIGHTLY_ISOHOME"].endswith("/toolchains/bin/isohome"))
            self.assertIn("engine-q-pristine", job_env["release-0.2.6-quest"]["NIGHTLY_ENGINE_PRISTINE"])
            self.assertEqual(job_env["release-0.2.6-main"]["NIGHTLY_GAME_PORT"], "44594")
            self.assertEqual(job_env["release-0.2.6-main"]["NIGHTLY_HTTP_PORT"], "1080")
            self.assertEqual(job_env["release-0.2.6-quest"]["NIGHTLY_GAME_PORT"], "44694")
            self.assertEqual(job_env["release-0.2.6-quest"]["NIGHTLY_HTTP_PORT"], "1180")
            run = settings.runs / "2026-10-23-dev-abcdef123-mac"
            self.assertEqual((run / "summary.json").read_text(encoding="utf-8"), '{"result":"PASS"}\n')
            self.assertFalse(settings.lock_dir.exists())
            self.assertFalse(settings.status.exists())
            self.assertFalse((settings.root.parent / "sessions/nightly-test/share/input").exists())
            self.assertFalse((settings.root.parent / "sessions/nightly-test/share/results").exists())

            with self.subTest("deployment runner uses a stable path"):
                readme = (HERE / "README.md").read_text(encoding="utf-8")
                self.assertIn(
                    "NIGHTLY_MAC_RUNNER=$HOME/.local/lib/274bot-nightly/runner", readme
                )
                self.assertNotIn("worktrees/nightly-tart", readme)

            with self.subTest("quiet lock prevents guest start"):
                with tempfile.TemporaryDirectory() as quiet_temporary:
                    quiet_settings = self.make_settings(Path(quiet_temporary))
                    quiet_vms = []

                    def quiet_vm_factory(*args):
                        vm = FakeVM(*args)
                        quiet_vms.append(vm)
                        return vm

                    quiet_controller = nightly.NightlyOrchestrator(
                        quiet_settings, ["dev"], vm_factory=quiet_vm_factory
                    )
                    write_status = quiet_controller.write_status

                    def write_start_status_then_lock(state):
                        write_status(state)
                        if state == "starting":
                            quiet_settings.quiet_lock.touch()

                    quiet_controller.write_status = write_start_status_then_lock
                    output = io.StringIO()
                    with contextlib.redirect_stdout(output):
                        self.assertEqual(quiet_controller.run(), 0)
                    self.assertEqual(quiet_vms[0].start_calls, 0)
                    self.assertIn("skipped before guest start", output.getvalue())

            with self.subTest("quiet lock prevents each cell batch"):
                with tempfile.TemporaryDirectory() as batch_temporary:
                    batch_settings = self.make_settings(Path(batch_temporary))
                    batch_vms = []

                    def batch_vm_factory(*args):
                        vm = FakeVM(*args)
                        batch_vms.append(vm)
                        return vm

                    batch_controller = nightly.NightlyOrchestrator(
                        batch_settings, ["dev"], vm_factory=batch_vm_factory
                    )
                    write_status = batch_controller.write_status

                    def write_running_status_then_lock(state):
                        write_status(state)
                        if state == "running":
                            batch_settings.quiet_lock.touch()

                    batch_controller.write_status = write_running_status_then_lock
                    output = io.StringIO()
                    with contextlib.redirect_stdout(output):
                        self.assertEqual(batch_controller.run(), 0)
                    self.assertEqual(
                        [key for _command, key in batch_vms[0].run_calls], ["bootstrap"]
                    )
                    self.assertIn("MAC-QUIET.lock is present before cell batch", output.getvalue())

    def test_stops_clone_and_releases_lock_after_start_fails(self):
        with tempfile.TemporaryDirectory() as temporary:
            settings = self.make_settings(Path(temporary))
            created = []

            def vm_factory(*args):
                vm = FakeVM(*args, fail_start=True)
                created.append(vm)
                return vm

            controller = nightly.NightlyOrchestrator(settings, ["dev"], vm_factory=vm_factory)

            with self.assertRaisesRegex(nightly.NightlyError, "simulated start failure"):
                controller.run()

            self.assertEqual(created[0].stop_calls, 1)
            self.assertFalse(settings.lock_dir.exists())
            self.assertFalse(settings.status.exists())

    def test_quiet_lock_during_guest_start_stops_before_staging(self):
        with tempfile.TemporaryDirectory() as temporary:
            settings = self.make_settings(Path(temporary))
            created = []

            def vm_factory(*args):
                vm = FakeVM(*args, quiet_during_start=True)
                created.append(vm)
                return vm

            controller = nightly.NightlyOrchestrator(
                settings, ["dev"], vm_factory=vm_factory
            )
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                result = controller.run()

            self.assertEqual(result, 0)
            self.assertEqual(created[0].start_calls, 1)
            self.assertEqual(created[0].stop_calls, 1)
            self.assertEqual(created[0].run_calls, [])
            self.assertIn("MAC-QUIET.lock appeared during guest start", output.getvalue())
            self.assertNotIn("staged runner source", output.getvalue())
            self.assertFalse(settings.lock_dir.exists())
            self.assertFalse(settings.status.exists())

    def test_quiet_lock_after_guest_start_skips_cleanly_without_staging(self):
        # The lock appears once the start has finished (session set) but before
        # staging: the run must stop the guest and exit 0, not fail on export.
        with tempfile.TemporaryDirectory() as temporary:
            settings = self.make_settings(Path(temporary))
            created = []

            def vm_factory(*args):
                vm = FakeVM(*args, quiet_after_start=True)
                created.append(vm)
                return vm

            controller = nightly.NightlyOrchestrator(
                settings, ["dev"], vm_factory=vm_factory
            )
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                result = controller.run()

            self.assertEqual(result, 0, output.getvalue())
            self.assertEqual(created[0].stop_calls, 1)
            self.assertEqual(created[0].run_calls, [])
            self.assertNotIn("staged runner source", output.getvalue())
            self.assertNotIn("nightly inputs are not staged", output.getvalue())
            self.assertFalse(settings.lock_dir.exists())
            self.assertFalse(settings.status.exists())

    def test_second_signal_during_teardown_does_not_skip_cleanup(self):
        with tempfile.TemporaryDirectory() as temporary:
            settings = self.make_settings(Path(temporary))
            controller = nightly.NightlyOrchestrator(
                settings, ["dev"], vm_factory=FakeVM
            )

            def send_second_signal():
                os.kill(os.getpid(), signal.SIGTERM)
                return True

            controller.vm.terminate_start = send_second_signal
            old_handler = signal.signal(signal.SIGTERM, nightly.handle_signal)
            try:
                with self.assertRaises(SystemExit) as raised:
                    with contextlib.redirect_stdout(io.StringIO()):
                        controller.run()
            finally:
                signal.signal(signal.SIGTERM, old_handler)

            self.assertEqual(raised.exception.code, 128 + signal.SIGTERM)
            self.assertEqual(controller.vm.stop_calls, 1)
            self.assertFalse(settings.status.exists())
            self.assertFalse(settings.lock_dir.exists())


class NightlyRefreshDocumentationTests(unittest.TestCase):
    def test_refresh_checks_checkout_and_run_lock_and_reinstalls_launch_files(self):
        readme = (HERE / "README.md").read_text(encoding="utf-8")
        section = readme.split(
            "### Merlot macOS nightly and malbec mirror\n", 1
        )[1].split("\n### ", 1)[0]

        self.assertIn('cd "$HOME/experiments/274bot"', section)
        self.assertIn("git status --short --branch", section)
        self.assertIn("git log -1 --oneline --decorate", section)
        lock_check = "if test -e /Volumes/dev-scratch/274bot-nightly/.mac-nightly.lock; then"
        self.assertLess(section.index(lock_check), section.index("rsync -a --delete"))
        self.assertIn("After each runner refresh, repeat step 3", section)
        for name in (
            "pull-274.sh",
            "nightly-mac.sh",
            "malbec-mirror-pull.sh",
            "local.274bot-nightly-mac.plist",
            "local.274bot-nightly-malbec-pull.plist",
        ):
            self.assertIn(name, section)


def make_branch_repo(root: Path, name: str, *, branch: str = "dev",
                     files: dict | None = None) -> Path:
    """A git repo with `files` committed on `branch` (branch-list test helper)."""
    repo = root / name
    repo.mkdir(parents=True)
    git("init", "-q", cwd=repo)
    git("symbolic-ref", "HEAD", f"refs/heads/{branch}", cwd=repo)
    for relative, text in (files or {"seed.txt": "seed\n"}).items():
        path = repo / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
    git("add", ".", cwd=repo)
    git("-c", "user.name=nightly-test", "-c", "user.email=nightly@example.invalid",
        "commit", "-qm", "init", cwd=repo)
    return repo


@unittest.skipUnless(shutil.which("git"), "git is not installed")
class MacBranchListTests(unittest.TestCase):
    def test_github_list_is_used_when_the_fetch_works(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            origin = make_branch_repo(root, "origin", branch="dev", files={
                "tools/nightly/branches.conf": "scratch-fix\ndev\n"})
            tip = git("rev-parse", "HEAD", cwd=origin)
            runner = root / "runner"
            subprocess.run(["git", "clone", "-q", str(origin), str(runner)], check=True)
            (runner / "tools/nightly/branches.conf").write_text("stale-branch\n", encoding="utf-8")
            with mock.patch.dict(os.environ):
                os.environ.pop("NIGHTLY_BRANCHES", None)
                branches, info = nightly.resolve_branches(None, runner)
            self.assertEqual(branches, ["scratch-fix", "dev"])
            self.assertEqual(info["source"], "github")
            self.assertEqual(info["commit"], tip)

    def test_local_list_and_a_status_note_when_the_fetch_fails(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            # No origin remote: the fetch must fail.
            runner = make_branch_repo(root, "runner", branch="dev", files={
                "tools/nightly/branches.conf": "# local list\nfix-line\ndev\n"})
            with mock.patch.dict(os.environ):
                os.environ.pop("NIGHTLY_BRANCHES", None)
                branches, info = nightly.resolve_branches(None, runner)
            self.assertEqual(branches, ["fix-line", "dev"])
            self.assertEqual(info["source"], "local")
            self.assertIn("fetch", info["note"])

    def test_requested_branches_win_without_a_fetch(self):
        with tempfile.TemporaryDirectory() as temporary:
            # No origin remote: a fetch attempt would fall back to local,
            # so the requested list proves no fetch was attempted.
            runner = make_branch_repo(Path(temporary), "runner", branch="dev", files={
                "tools/nightly/branches.conf": "fix-line\ndev\n"})
            with mock.patch.dict(os.environ):
                os.environ.pop("NIGHTLY_BRANCHES", None)
                branches, info = nightly.resolve_branches(["dev"], runner)
            self.assertEqual(branches, ["dev"])
            self.assertEqual(info["source"], "cli")

    def test_env_branches_win_without_a_fetch(self):
        with tempfile.TemporaryDirectory() as temporary:
            runner = make_branch_repo(Path(temporary), "runner", branch="dev", files={
                "tools/nightly/branches.conf": "fix-line\ndev\n"})
            with mock.patch.dict(os.environ, {"NIGHTLY_BRANCHES": "release/0.2.6 dev"}):
                branches, info = nightly.resolve_branches(None, runner)
            self.assertEqual(branches, ["release/0.2.6", "dev"])
            self.assertEqual(info["source"], "env")


@unittest.skipUnless(shutil.which("git"), "git is not installed")
class MacSelfUpdateTests(unittest.TestCase):
    def setUp(self):
        if os.environ.get("NIGHTLY_SELF_CHECK_ACTIVE"):
            self.skipTest("the self-check does not re-test the updater")

    def enable_update_env(self):
        """Opt into the update path: NIGHTLY_SELF_UPDATE=1, no re-exec state."""
        patcher = mock.patch.dict(os.environ)
        patcher.start()
        self.addCleanup(patcher.stop)
        for key in ("NIGHTLY_SELF_UPDATE_DONE", "NIGHTLY_SELF_UPDATE_FROM"):
            os.environ.pop(key, None)
        os.environ["NIGHTLY_SELF_UPDATE"] = "1"

    def test_update_is_off_unless_opted_in(self):
        off_values = [None, "0", "", "false", "no", "off"]
        for off in off_values:
            with self.subTest(opt_in=off):
                with tempfile.TemporaryDirectory() as temporary:
                    root = Path(temporary)
                    origin = make_branch_repo(root, "origin", branch="dev", files={
                        "tools/nightly/nightly-mac-guest.py": "# new\n"})
                    runner = root / "runner"
                    subprocess.run(["git", "clone", "-q", str(origin), str(runner)], check=True)
                    script = runner / "tools/nightly/nightly-mac-guest.py"
                    before = script.read_text(encoding="utf-8")
                    # A broken remote proves no fetch is attempted while off.
                    subprocess.run(["git", "remote", "set-url", "origin", str(root / "missing.git")],
                                   cwd=runner, check=True)
                    calls: list = []
                    with mock.patch.dict(os.environ):
                        os.environ.pop("NIGHTLY_SELF_UPDATE_DONE", None)
                        os.environ.pop("NIGHTLY_SELF_UPDATE_FROM", None)
                        if off is None:
                            os.environ.pop("NIGHTLY_SELF_UPDATE", None)
                        else:
                            os.environ["NIGHTLY_SELF_UPDATE"] = off
                        info = nightly.maybe_self_update(runner, script, ["--branch", "dev"],
                                                         _exec=lambda *args: calls.append(args))
                    self.assertEqual(info["status"], "disabled")
                    self.assertEqual(script.read_text(encoding="utf-8"), before)
                    self.assertEqual(calls, [])

    def test_a_failed_self_check_keeps_the_old_code(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            origin = make_branch_repo(root, "origin", branch="dev", files={
                "tools/nightly/nightly-mac-guest.py": "# old controller\n"})
            runner = root / "runner"
            subprocess.run(["git", "clone", "-q", str(origin), str(runner)], check=True)
            old_head = git("rev-parse", "HEAD", cwd=runner)
            (origin / "tools/nightly/nightly-mac-guest.py").write_text(
                "def broken(:\n", encoding="utf-8")
            git("add", ".", cwd=origin)
            git("-c", "user.name=nightly-test", "-c", "user.email=nightly@example.invalid",
                "commit", "-qm", "broken", cwd=origin)
            tip = git("rev-parse", "HEAD", cwd=origin)
            self.assertNotEqual(tip, old_head)
            script = runner / "tools/nightly/nightly-mac-guest.py"
            calls: list = []
            self.enable_update_env()
            info = nightly.maybe_self_update(runner, script, ["--branch", "dev"],
                                             _exec=lambda *args: calls.append(args))
            self.assertEqual(info["status"], "check-failed")
            self.assertEqual(info["to"], tip)
            self.assertEqual(script.read_text(encoding="utf-8"), "# old controller\n")
            self.assertEqual(git("rev-parse", "HEAD", cwd=runner), old_head)
            self.assertEqual(calls, [])

    def test_a_passing_self_check_updates_and_reexecs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            origin = root / "origin"
            origin.mkdir()
            git("init", "-q", cwd=origin)
            git("symbolic-ref", "HEAD", "refs/heads/dev", cwd=origin)
            shutil.copytree(HERE, origin / "tools/nightly",
                            ignore=shutil.ignore_patterns("__pycache__"))
            git("add", ".", cwd=origin)
            git("-c", "user.name=nightly-test", "-c", "user.email=nightly@example.invalid",
                "commit", "-qm", "controller", cwd=origin)
            runner = root / "runner"
            subprocess.run(["git", "clone", "-q", str(origin), str(runner)], check=True)
            probe = "# nightly self-update probe\n"
            with (origin / "tools/nightly/nightly-mac-guest.py").open("a", encoding="utf-8") as handle:
                handle.write(probe)
            git("add", ".", cwd=origin)
            git("-c", "user.name=nightly-test", "-c", "user.email=nightly@example.invalid",
                "commit", "-qm", "probe", cwd=origin)
            tip = git("rev-parse", "HEAD", cwd=origin)
            script = runner / "tools/nightly/nightly-mac-guest.py"
            calls: list = []
            self.enable_update_env()
            info = nightly.maybe_self_update(runner, script, ["--branch", "dev"],
                                             _exec=lambda *args: calls.append(args))
            self.assertIsNone(info)
            self.assertEqual(len(calls), 1)
            executable, argv, _env = calls[0]
            self.assertEqual(executable, sys.executable)
            self.assertEqual(argv[:2], [sys.executable, str(script)])
            self.assertEqual(argv[2:], ["--branch", "dev"])
            self.assertIn(probe, script.read_text(encoding="utf-8"))
            self.assertEqual(git("rev-parse", "HEAD", cwd=runner), tip)

    def test_status_records_branch_list_and_self_update(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            values = {key: str(root / key) for key in nightly.Settings.REQUIRED}
            settings = nightly.Settings(values)
            branch_list_info = {"source": "github", "commit": "a" * 40,
                                "note": "from origin/dev at " + "a" * 40}
            self_update_info = {"status": "check-failed", "from": "o" * 40, "to": "n" * 40,
                                "detail": "candidate origin/dev failed the self-check"}
            orchestrator = nightly.NightlyOrchestrator(
                settings, ["dev"], vm_factory=FakeVM,
                branch_list_info=branch_list_info, self_update_info=self_update_info)
            orchestrator.pins = {"dev": "a" * 40}
            orchestrator.write_status("testing")
            status = settings.status.read_text(encoding="utf-8")
            self.assertIn("branch_list_source=github", status)
            self.assertIn("branch_list_commit=" + "a" * 40, status)
            self.assertIn("self_update=check-failed", status)
            self.assertIn("self_update_detail=candidate origin/dev", status)


class ReexecTests(unittest.TestCase):
    def test_windows_reexec_waits_for_the_child_exit_code(self):
        seen: list = []

        def fake_run(args, env):
            seen.append((list(args), dict(env)))
            return subprocess.CompletedProcess(args, 42)

        with mock.patch.object(os, "name", "nt"):
            with mock.patch.object(subprocess, "run", side_effect=fake_run):
                with self.assertRaises(SystemExit) as exited:
                    nightly.reexec(sys.executable, [sys.executable, "nightly-mac-guest.py"],
                                   {"K": "V"})
        self.assertEqual(exited.exception.code, 42)
        self.assertEqual(seen, [([sys.executable, "nightly-mac-guest.py"], {"K": "V"})])

    def test_posix_reexec_replaces_the_process(self):
        with mock.patch.object(os, "name", "posix"):
            with mock.patch.object(os, "execvpe", create=True) as execvpe:
                with mock.patch.object(subprocess, "run",
                                       side_effect=AssertionError("must not spawn")):
                    nightly.reexec(sys.executable, [sys.executable, "nightly-mac-guest.py"],
                                   {"K": "V"})
        execvpe.assert_called_once_with(sys.executable, [sys.executable, "nightly-mac-guest.py"],
                                        {"K": "V"})


if __name__ == "__main__":
    unittest.main()
