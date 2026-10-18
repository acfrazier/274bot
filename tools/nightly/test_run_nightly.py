#!/usr/bin/env python3
"""Tests for the disk guard and the stable tree path in `run_nightly.py`.

The guard stops the Windows nightly from filling its disk: a Cargo target
folder over `NIGHTLY_TARGET_CAP_GB` is removed before the lanes and before
the build, and the night is UNAVAILABLE when the drive still has less free
space than `NIGHTLY_MIN_FREE_GB` after that. The stable tree path
(`trees/<branch>`, a clean clone into the same path every night) stops Cargo
from adding a new set of artefacts for every new source path.

Run on Linux or Windows with the standard library only:

  python3 -B tools/nightly/test_run_nightly.py
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
os.environ.setdefault("NIGHTLY_ROOT", str(Path(tempfile.gettempdir()) / "run-nightly-test-root"))

import run_nightly as rn


def make_target(root: Path, name: str, size: int) -> Path:
    target = root / name
    target.mkdir(parents=True)
    (target / "artefact").write_bytes(b"x" * size)
    return target


class FakeDrive:
    """Free space that goes up by the size of every target removed since the start."""

    def __init__(self, targets: list[Path], base_free: int):
        self.targets = targets
        self.base_free = base_free
        self.initial = sum(rn.dir_size(target) for target in targets)

    def free(self, _path: Path) -> int:
        current = sum(rn.dir_size(target) for target in self.targets)
        return self.base_free + (self.initial - current)


class DirSizeTests(unittest.TestCase):
    def test_counts_nested_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "a.bin").write_bytes(b"x" * 100)
            (root / "sub").mkdir()
            (root / "sub" / "b.bin").write_bytes(b"y" * 250)
            self.assertEqual(rn.dir_size(root), 350)

    def test_missing_path_is_zero(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(rn.dir_size(Path(tmp) / "nope"), 0)

    def test_free_bytes_uses_existing_parent(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertGreater(rn.free_bytes(Path(tmp) / "nope" / "target"), 0)


class DiskGuardTests(unittest.TestCase):
    def guard(self, targets, cap, floor, base_free):
        drive = FakeDrive(targets, base_free)
        logs: list[str] = []
        with mock.patch.object(rn, "log", logs.append):
            reason = rn.disk_guard(targets, cap, floor, free_of=drive.free)
        return reason, logs

    def test_target_over_the_cap_is_removed(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            primary = make_target(root, "dev-ci", 5000)
            other = make_target(root, "dev", 100)
            reason, logs = self.guard([primary, other], cap=1000, floor=1000, base_free=10**7)
            self.assertIsNone(reason)
            self.assertFalse(primary.exists())
            self.assertTrue(other.exists())
            self.assertEqual(len(logs), 1)
            self.assertIn("disk guard: removed", logs[0])
            self.assertIn("over the cap", logs[0])

    def test_floor_removes_the_primary_target_and_the_night_runs(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            primary = make_target(root, "dev-ci", 5000)
            other = make_target(root, "dev", 4000)
            reason, logs = self.guard([primary, other], cap=10**9, floor=1000, base_free=100)
            self.assertIsNone(reason)
            self.assertFalse(primary.exists())
            # Removing the primary target freed enough space; the other stays.
            self.assertTrue(other.exists())
            self.assertEqual(len(logs), 1)
            self.assertIn("free space below the floor", logs[0])

    def test_floor_still_not_met_marks_the_night_unavailable(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            primary = make_target(root, "dev-ci", 500)
            other = make_target(root, "dev", 500)
            reason, logs = self.guard([primary, other], cap=10**9, floor=100_000, base_free=100)
            self.assertIsNotNone(reason)
            self.assertIn("not enough free disk space", reason)
            self.assertIn("nothing was built", reason)
            self.assertFalse(primary.exists())
            self.assertFalse(other.exists())
            self.assertEqual(len(logs), 2)

    def test_under_the_cap_with_free_space_nothing_is_removed(self):
        with tempfile.TemporaryDirectory() as tmp:
            primary = make_target(Path(tmp), "dev-ci", 500)
            reason, logs = self.guard([primary], cap=1000, floor=1000, base_free=10**7)
            self.assertIsNone(reason)
            self.assertTrue(primary.exists())
            self.assertEqual(logs, [])

    def test_zero_cap_and_floor_turn_the_guard_off(self):
        with tempfile.TemporaryDirectory() as tmp:
            primary = make_target(Path(tmp), "dev-ci", 5000)
            logs: list[str] = []
            with mock.patch.object(rn, "log", logs.append):
                reason = rn.disk_guard([primary], 0, 0, free_of=lambda _path: 0)
            self.assertIsNone(reason)
            self.assertTrue(primary.exists())
            self.assertEqual(logs, [])

    def test_guard_disk_failure_fails_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with mock.patch.object(rn, "ROOT", root), \
                    mock.patch.object(rn, "disk_guard", side_effect=OSError("folder is locked")):
                reason = rn.guard_disk("dev", root / "target" / "dev-ci")
            self.assertIsNotNone(reason)
            self.assertIn("disk guard failed", reason)
            self.assertIn("folder is locked", reason)
            self.assertIn("nothing was built", reason)

    def test_cap_without_a_floor_never_marks_unavailable(self):
        with tempfile.TemporaryDirectory() as tmp:
            primary = make_target(Path(tmp), "dev-ci", 5000)
            with mock.patch.object(rn, "log", lambda _message: None):
                reason = rn.disk_guard([primary], 1000, 0, free_of=lambda _path: 0)
            self.assertIsNone(reason)
            self.assertFalse(primary.exists())

    def test_missing_target_below_the_floor_is_unavailable(self):
        with tempfile.TemporaryDirectory() as tmp:
            missing = Path(tmp) / "dev-ci"
            reason = rn.disk_guard([missing], 1000, 1000, free_of=lambda _path: 0)
            self.assertIsNotNone(reason)
            self.assertIn("not enough free disk space", reason)


def git(*args: str, cwd: Path) -> str:
    return subprocess.run(["git", *args], cwd=cwd, check=True, capture_output=True, text=True).stdout.strip()


@unittest.skipUnless(shutil.which("git"), "git is not installed")
class StableTreeTests(unittest.TestCase):
    def make_source(self, root: Path) -> Path:
        src = root / "src"
        src.mkdir()
        git("init", "-q", cwd=src)
        git("symbolic-ref", "HEAD", "refs/heads/main", cwd=src)
        (src / "f.txt").write_text("v1\n", encoding="utf-8")
        git("add", ".", cwd=src)
        git("-c", "user.name=test", "-c", "user.email=test@example.invalid", "commit", "-qm", "v1", cwd=src)
        return src

    def commit_v2(self, src: Path) -> None:
        (src / "f.txt").write_text("v2\n", encoding="utf-8")
        git("add", ".", cwd=src)
        git("-c", "user.name=test", "-c", "user.email=test@example.invalid", "commit", "-qm", "v2", cwd=src)

    def test_fresh_tree_clones_into_the_same_stable_path_every_night(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "nightly"
            root.mkdir()
            src = self.make_source(Path(tmp))
            build_log = root / "build.log"
            with mock.patch.object(rn, "ROOT", root), mock.patch.object(rn, "REPO_URL", str(src)):
                tree, full, _short = rn.fresh_tree("main", build_log)
                self.assertEqual(tree, root / "trees" / "main")
                self.assertEqual(tree, rn.tree_path("main"))
                self.assertEqual((tree / "f.txt").read_text(encoding="utf-8"), "v1\n")
                self.assertEqual(full, git("rev-parse", "HEAD", cwd=src))

                # A stale file from tonight must not survive into tomorrow.
                (tree / "stale.txt").write_text("stale\n", encoding="utf-8")
                self.commit_v2(src)
                tree2, full2, _short2 = rn.fresh_tree("main", build_log)
                self.assertEqual(tree2, tree)
                self.assertFalse((tree2 / "stale.txt").exists())
                self.assertEqual((tree2 / "f.txt").read_text(encoding="utf-8"), "v2\n")
                self.assertEqual(full2, git("rev-parse", "HEAD", cwd=src))
                self.assertEqual([entry.name for entry in (root / "trees").iterdir()], ["main"])

    def test_prune_trees_keeps_only_stable_branch_trees(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            trees = root / "trees"
            for name in ("dev", "dev-abc123def-20261017T230000", "old-branch"):
                (trees / name).mkdir(parents=True)
            with mock.patch.object(rn, "ROOT", root), mock.patch.object(rn, "log", lambda _m: None):
                rn.prune_trees(["dev"])
            self.assertEqual([entry.name for entry in trees.iterdir()], ["dev"])

    def test_prune_trees_without_a_trees_folder_is_a_no_op(self):
        with tempfile.TemporaryDirectory() as tmp:
            with mock.patch.object(rn, "ROOT", Path(tmp)):
                rn.prune_trees(["dev"])


class GuardDefaultsTests(unittest.TestCase):
    def import_constants(self, extra_env: dict) -> list[str]:
        env = {key: value for key, value in os.environ.items() if not key.startswith("NIGHTLY_")}
        env.update(extra_env)
        out = subprocess.run(
            [sys.executable, "-c",
             "import run_nightly as r; print(r.TARGET_CAP_GB, r.MIN_FREE_GB, r.FRESH_TREES)"],
            cwd=HERE, env=env, check=True, capture_output=True, text=True).stdout.split()
        return out

    @unittest.skipIf(os.name == "nt", "the Linux defaults do not apply on Windows")
    def test_linux_defaults_leave_the_guard_off(self):
        # The Linux VM's disk is 100 GB in total; the 100 GB floor must not
        # apply there, and malbec's behaviour stays as it was.
        self.assertEqual(self.import_constants({}), ["0", "0", "False"])

    def test_the_variables_turn_the_guard_on(self):
        out = self.import_constants({"NIGHTLY_TARGET_CAP_GB": "7", "NIGHTLY_MIN_FREE_GB": "9"})
        self.assertEqual(out[:2], ["7", "9"])


class RunBranchDiskGuardTests(unittest.TestCase):
    def test_a_full_disk_writes_a_complete_unavailable_run(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            ci = make_target(root / "target", "dev-ci", 500)
            build_target = make_target(root / "target", "dev", 500)

            def fake_update_tree(_branch, tree, _build_log):
                catalog = tree / rn.CATALOG_RS
                catalog.parent.mkdir(parents=True)
                catalog.write_text(
                    'const REGISTRY: &[Entry] = &[\n    Entry::new("walk"),\n];\n', encoding="utf-8")
                return "b" * 40, "b" * 9

            patches = [
                mock.patch.object(rn, "ROOT", root),
                mock.patch.object(rn, "STEPS", ["lanes", "tui"]),
                mock.patch.object(rn, "FRESH_TREES", False),
                mock.patch.object(rn, "REQUIRE_CONSOLE", False),
                mock.patch.object(rn, "ENGINE_MODE", "remote"),
                mock.patch.object(rn, "TARGET_CAP_GB", 40),
                mock.patch.object(rn, "MIN_FREE_GB", 100),
                mock.patch.object(rn, "free_bytes", lambda _path: 0),
                mock.patch.object(rn, "update_tree", fake_update_tree),
                mock.patch.object(rn, "log", lambda _message: None),
            ]
            for patch in patches:
                patch.start()
            try:
                run_dir = rn.run_branch("dev", 1, None, None)
            finally:
                for patch in patches:
                    patch.stop()

            summary = json.loads((run_dir / "summary.json").read_text(encoding="utf-8"))
            self.assertIn("not enough free disk space", summary["unavailable"])
            self.assertTrue(summary["lanes"])
            self.assertEqual({row["result"] for row in summary["lanes"]}, {"UNAVAILABLE"})
            self.assertEqual({row["result"] for row in summary["cells"]}, {"UNAVAILABLE"})
            self.assertFalse(summary["build"]["ran"])
            self.assertTrue((run_dir / "DONE").is_file())
            self.assertFalse(ci.exists())
            self.assertFalse(build_target.exists())


if __name__ == "__main__":
    unittest.main()
