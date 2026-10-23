#!/usr/bin/env python3
"""Tests for the host controller that runs the Mac nightly in Tart."""

import contextlib
import importlib.util
import io
import json
import subprocess
import sys
import tarfile
import tempfile
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
    git("branch", "release/0.2.5", cwd=source)
    return source


class SourcePinTests(unittest.TestCase):
    def test_pins_each_branch_to_the_fetched_commit(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = make_source(root)
            input_root = root / "input"
            input_root.mkdir()

            pins = nightly.pin_sources(str(source), ["release/0.2.5", "dev"], input_root)

            self.assertEqual(set(pins), {"release/0.2.5", "dev"})
            for branch, commit in pins.items():
                self.assertEqual(
                    git("--git-dir", str(input_root / "source.git"), "rev-parse",
                        f"refs/heads/{branch}"),
                    commit,
                )
            metadata = json.loads((input_root / "branch-pins.json").read_text(encoding="utf-8"))
            self.assertEqual(metadata["branches"], pins)


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
        process.communicate.return_value = (json.dumps(session), "")
        module = mock.Mock()
        module.load_session.return_value = session
        vm = nightly.TartVM(settings, lambda: 200 * nightly.GIB)

        with mock.patch.object(nightly.subprocess, "Popen", return_value=process) as popen, \
                mock.patch.object(nightly, "load_vm_module", return_value=module):
            observed = vm.start()

        command = popen.call_args.args[0]
        self.assertEqual(observed, session)
        self.assertEqual(command[command.index("--cpu") + 1], "8")
        self.assertEqual(command[command.index("--memory") + 1], "24576")
        self.assertEqual(command[command.index("--disk-size") + 1], "160")
        self.assertEqual(command[command.index("--base") + 1], "274bot-mac-base")
        self.assertIn("--skip-tcc-bootstrap", command)


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
    def __init__(self, settings, _disk_check, *, fail_start=False):
        self.settings = settings
        self.fail_start = fail_start
        self.session = None
        self.session_id = None
        self.supervisor_path = None
        self.staged_isohome = None
        self.staged_isohome_mode = None
        self.run_calls = []
        self.stop_calls = 0

    def start(self):
        self.session_id = "nightly-test"
        if self.fail_start:
            raise nightly.NightlyError("simulated start failure after clone creation")
        session_dir = self.settings.root.parent / "sessions" / self.session_id
        share = session_dir / "share"
        share.mkdir(parents=True, exist_ok=True)
        guest_home = self.settings.root.parent / "guest-home"
        guest_home.mkdir(parents=True, exist_ok=True)
        self.session = {
            "id": self.session_id, "vm": "fake-vm", "ip": "192.0.2.10",
            "guest_home": str(guest_home), "share": str(share),
        }

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
        (runner / "tools/nightly/branches.conf").write_text("release/0.2.5\ndev\n", encoding="utf-8")
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
                settings, ["release/0.2.5", "dev"], vm_factory=vm_factory
            )
            self.assertEqual(controller.run(), 0)
            vm = created[0]

            self.assertEqual(vm.stop_calls, 1)
            self.assertEqual(vm.staged_isohome, "#!/bin/sh\nexec \"$@\"\n")
            self.assertEqual(vm.staged_isohome_mode, 0o555)
            self.assertEqual(len(vm.run_calls), 5)
            self.assertEqual(
                [key for _command, key in vm.run_calls],
                ["bootstrap", "release-0.2.5-main", "release-0.2.5-quest", "dev-main", "dev-quest"],
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
            self.assertIn("engine-pristine", job_env["release-0.2.5-main"]["NIGHTLY_ENGINE_PRISTINE"])
            self.assertTrue(job_env["release-0.2.5-main"]["NIGHTLY_ISOHOME"].endswith("/toolchains/bin/isohome"))
            self.assertIn("engine-q-pristine", job_env["release-0.2.5-quest"]["NIGHTLY_ENGINE_PRISTINE"])
            self.assertEqual(job_env["release-0.2.5-main"]["NIGHTLY_GAME_PORT"], "44594")
            self.assertEqual(job_env["release-0.2.5-main"]["NIGHTLY_HTTP_PORT"], "1080")
            self.assertEqual(job_env["release-0.2.5-quest"]["NIGHTLY_GAME_PORT"], "44694")
            self.assertEqual(job_env["release-0.2.5-quest"]["NIGHTLY_HTTP_PORT"], "1180")
            run = settings.runs / "2026-10-23-dev-abcdef123-mac"
            self.assertEqual((run / "summary.json").read_text(encoding="utf-8"), '{"result":"PASS"}\n')
            self.assertFalse(settings.lock_dir.exists())
            self.assertFalse(settings.status.exists())
            self.assertFalse((settings.root.parent / "sessions/nightly-test/share/input").exists())
            self.assertFalse((settings.root.parent / "sessions/nightly-test/share/results").exists())

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


if __name__ == "__main__":
    unittest.main()
