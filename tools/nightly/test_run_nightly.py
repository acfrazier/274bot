#!/usr/bin/env python3
"""Tests for `run_nightly.py`: the disk guard, the stable tree path, receipts,
the revision defaults, the cells file and the latency lane (settings, the
classifier, proxy-stats aggregation, the pass against a fake cell runner with
a real proxy subprocess, and the summary shape with the lane on and off).

The guard stops the Windows nightly from filling its disk: a Cargo target
folder over `NIGHTLY_TARGET_CAP_GB` is removed before the lanes and before
the build, and the night is UNAVAILABLE when the drive still has less free
space than `NIGHTLY_MIN_FREE_GB` after that. A target folder whose size
cannot be read in full counts as over the cap. Measurement and removal
share one walker that never goes into a symlink or a Windows junction, at
the top or nested. A link counts as 0 bytes and is removed as a link, and
nothing outside the target changes, not even a read-only file's mode. The
logon catch-up writes its UNAVAILABLE summary without running the guard.
The stable tree path
(`trees/<branch>`, a clean clone into the same path every night) stops Cargo
from adding a new set of artefacts for every new source path.

Run on Linux or Windows with the standard library only:

  python3 -B tools/nightly/test_run_nightly.py
"""

import json
import hashlib
import gc
import os
import shutil
import socket
import stat
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import warnings
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
os.environ.setdefault("NIGHTLY_ROOT", str(Path(tempfile.gettempdir()) / "run-nightly-test-root"))

import run_nightly as rn


_THROWAWAY_ROOT = Path(tempfile.gettempdir()).resolve()
_real_maybe_self_update = rn.maybe_self_update


def _throwaway_only_maybe_self_update(repo, *args, **kwargs):
    """The test-only self-update gate: only throwaway repos may be updated.

    Every `repo` below is a directory the test itself created under the
    system temp dir. The real checkout (`rn.RUNNER_REPO`) is never updated:
    `maybe_self_update` force-checks out `origin/dev`, which would discard
    uncommitted work. Any other caller fails loudly before git runs.
    """
    if not Path(repo).resolve().is_relative_to(_THROWAWAY_ROOT):
        raise AssertionError(
            f"refusing to self-update {repo}: tests only update throwaway repos "
            f"they create under {_THROWAWAY_ROOT}")
    return _real_maybe_self_update(repo, *args, **kwargs)


rn.maybe_self_update = _throwaway_only_maybe_self_update


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

    A test that reaches `maybe_self_update` (or `main()`) with the real
    `HERE`/runner path force-checks out `origin/dev` and discards uncommitted
    edits (NIGHTLY-FROM-GITHUB-1026 finding 1). The gate above makes that
    impossible; this teardown fails the suite if it ever happens anyway.
    """
    if _REAL_CHECKOUT_STATE is None:
        return
    after = _record_real_checkout_state()
    if after != _REAL_CHECKOUT_STATE:
        raise AssertionError(
            f"a test touched the real checkout {HERE.parent.parent}: "
            f"before={_REAL_CHECKOUT_STATE!r} after={after!r}")


def make_target(root: Path, name: str, size: int) -> Path:
    target = root / name
    target.mkdir(parents=True)
    (target / "artefact").write_bytes(b"x" * size)
    return target


def mode_of(path: Path) -> int:
    return stat.S_IMODE(path.stat().st_mode)


def make_symlink(link: Path, target: Path, *, directory: bool = False) -> None:
    try:
        os.symlink(target, link, target_is_directory=directory)
    except (OSError, NotImplementedError) as error:
        raise unittest.SkipTest(f"cannot create a symlink here: {error}")


def make_junction(link: Path, target: Path) -> None:
    result = subprocess.run(["cmd", "/c", "mklink", "/J", str(link), str(target)],
                            capture_output=True, text=True)
    if result.returncode != 0:
        raise unittest.SkipTest(f"mklink /J failed: {result.stdout} {result.stderr}")


def make_outside(root: Path) -> tuple[Path, Path]:
    """A directory outside the target under test, with one file in it."""
    outside = root / "outside"
    outside.mkdir()
    keep = outside / "keep.txt"
    keep.write_text("keep\n", encoding="utf-8")
    return outside, keep


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

    @unittest.skipIf(os.name == "nt", "mode 000 does not stop a listing on Windows")
    @unittest.skipIf(getattr(os, "geteuid", lambda: 1)() == 0, "root can list a mode 000 folder")
    def test_an_unreadable_subfolder_raises_instead_of_a_partial_size(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "visible.bin").write_bytes(b"x" * 100)
            sub = root / "sub"
            sub.mkdir()
            (sub / "hidden.bin").write_bytes(b"y" * 5000)
            os.chmod(sub, 0)
            try:
                with self.assertRaises(OSError):
                    rn.dir_size(root)
            finally:
                os.chmod(sub, 0o700)

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

    def test_a_target_whose_size_cannot_be_read_is_removed_as_over_the_cap(self):
        with tempfile.TemporaryDirectory() as tmp:
            primary = make_target(Path(tmp), "dev-ci", 5000)

            def bad_size(_path):
                raise OSError("cannot list a subfolder")

            logs: list[str] = []
            with mock.patch.object(rn, "log", logs.append):
                reason = rn.disk_guard([primary], 1000, 1000, size_of=bad_size,
                                       free_of=lambda _path: 10**7)
            self.assertIsNone(reason)
            self.assertFalse(os.path.lexists(primary))
            self.assertEqual(len(logs), 1)
            self.assertIn("disk guard: removed", logs[0])
            self.assertIn("size could not be read", logs[0])

    @unittest.skipIf(os.name == "nt", "mode 000 does not stop a listing on Windows")
    @unittest.skipIf(getattr(os, "geteuid", lambda: 1)() == 0, "root can list a mode 000 folder")
    def test_an_unreadable_subfolder_inside_an_over_cap_target_is_removed(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            target = root / "dev-ci"
            target.mkdir()
            (target / "visible.bin").write_bytes(b"x" * 100)
            sub = target / "sub"
            sub.mkdir()
            (sub / "hidden.bin").write_bytes(b"y" * 5000)
            os.chmod(sub, 0)
            logs: list[str] = []
            try:
                with mock.patch.object(rn, "log", logs.append):
                    reason = rn.disk_guard([target], 1000, 1000, free_of=lambda _path: 10**7)
                self.assertIsNone(reason)
                self.assertFalse(os.path.lexists(target))
                self.assertEqual(len(logs), 1)
                self.assertIn("size could not be read", logs[0])
            finally:
                if sub.exists():
                    os.chmod(sub, 0o700)

    def test_a_removal_that_leaves_the_target_in_place_is_a_guard_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            primary = make_target(Path(tmp), "dev-ci", 5000)
            logs: list[str] = []
            with mock.patch.object(rn, "log", logs.append):
                with self.assertRaises(OSError):
                    rn.disk_guard([primary], 1000, 1000, free_of=lambda _path: 10**7,
                                  remove=lambda _path: None)
            self.assertTrue(os.path.lexists(primary))
            self.assertEqual(logs, [])

    def test_a_removal_that_fails_twice_marks_the_night_unavailable(self):
        # The real removal code, with every unlink locked. The first try
        # and the retry after the permission fix both fail.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            primary = make_target(root / "target", "dev-ci", 5000)
            before = mode_of(primary)
            logs: list[str] = []
            try:
                with mock.patch.object(rn, "ROOT", root), \
                        mock.patch.object(rn, "GB", 1), \
                        mock.patch.object(rn, "TARGET_CAP_GB", 1000), \
                        mock.patch.object(rn, "MIN_FREE_GB", 0), \
                        mock.patch.object(rn, "log", logs.append), \
                        mock.patch("os.unlink", side_effect=OSError("folder is locked")) as unlink:
                    reason = rn.guard_disk("dev", primary)
                self.assertIsNotNone(reason)
                self.assertIn("disk guard failed", reason)
                self.assertIn("folder is locked", reason)
                self.assertIn("nothing was built", reason)
                self.assertGreaterEqual(unlink.call_count, 2)
                self.assertTrue(os.path.lexists(primary))
                self.assertEqual(logs, [])
                # A permission fix adds permissions. It never strips them.
                self.assertEqual(mode_of(primary), before)
            finally:
                if primary.exists():
                    os.chmod(primary, 0o755)

    def test_a_top_level_symlink_target_is_unlinked_and_the_outside_folder_is_untouched(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            outside, keep = make_outside(root)
            before = mode_of(outside)
            link = root / "dev-ci"
            make_symlink(link, outside, directory=True)
            logs: list[str] = []
            try:
                with mock.patch.object(rn, "log", logs.append):
                    reason = rn.disk_guard(
                        [link], 10**9, 1000,
                        free_of=lambda _path: 0 if os.path.lexists(link) else 10**7)
                self.assertIsNone(reason)
                self.assertFalse(os.path.lexists(link))
                self.assertEqual(keep.read_text(encoding="utf-8"), "keep\n")
                self.assertEqual(mode_of(outside), before)
                self.assertEqual(len(logs), 1)
                self.assertIn("disk guard: removed", logs[0])
            finally:
                if outside.exists():
                    os.chmod(outside, 0o755)


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

    def test_existing_tree_fetches_from_the_selected_pinned_repository(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "upstream"
            source.mkdir()
            git("init", "-q", cwd=source)
            git("symbolic-ref", "HEAD", "refs/heads/dev", cwd=source)
            (source / "source.txt").write_text("pinned\n", encoding="utf-8")
            git("add", ".", cwd=source)
            git("-c", "user.name=test", "-c", "user.email=test@example.invalid",
                "commit", "-qm", "pinned", cwd=source)
            pinned = git("rev-parse", "HEAD", cwd=source)
            pin_repo = root / "dev-pin.git"
            subprocess.run(["git", "clone", "-q", "--bare", str(source), str(pin_repo)], check=True)

            (source / "source.txt").write_text("new upstream\n", encoding="utf-8")
            git("add", ".", cwd=source)
            git("-c", "user.name=test", "-c", "user.email=test@example.invalid",
                "commit", "-qm", "new upstream", cwd=source)
            tree = root / "tree"
            subprocess.run(["git", "clone", "-q", "--no-checkout", str(source), str(tree)], check=True)
            build_log = root / "build.log"

            with mock.patch.object(rn, "REPO_URL", str(pin_repo)):
                full, short = rn.update_tree("dev", tree, build_log)

            self.assertEqual(full, pinned)
            self.assertEqual(short, pinned[:9])
            self.assertEqual((tree / "source.txt").read_text(encoding="utf-8"), "pinned\n")
            self.assertEqual(git("remote", "get-url", "origin", cwd=tree), str(pin_repo))


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


class RmtreeTests(unittest.TestCase):
    def test_a_top_level_symlink_to_an_outside_folder_is_unlinked_not_followed(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            outside, keep = make_outside(root)
            before = mode_of(outside)
            link = root / "dev-ci"
            make_symlink(link, outside, directory=True)
            try:
                rn.rmtree(link)
                self.assertFalse(os.path.lexists(link))
                self.assertEqual(keep.read_text(encoding="utf-8"), "keep\n")
                self.assertEqual(mode_of(outside), before)
            finally:
                if outside.exists():
                    os.chmod(outside, 0o755)

    def test_read_only_files_and_folders_inside_the_target_are_removed(self):
        # git's object files are read-only, and Windows refuses to unlink a
        # read-only file. A folder without owner write blocks the removal of
        # its entries on Linux.
        with tempfile.TemporaryDirectory() as tmp:
            target = make_target(Path(tmp), "dev-ci", 500)
            sub = target / "objects"
            sub.mkdir()
            readonly = sub / "pack.idx"
            readonly.write_bytes(b"p" * 300)
            os.chmod(readonly, 0o444)
            os.chmod(sub, 0o555)
            try:
                self.assertEqual(rn.dir_size(target), 800)
                rn.rmtree(target)
                self.assertFalse(os.path.lexists(target))
            finally:
                if sub.exists():
                    os.chmod(sub, 0o755)

    def test_a_removal_that_fails_twice_raises(self):
        with tempfile.TemporaryDirectory() as tmp:
            target = make_target(Path(tmp), "dev-ci", 500)
            before = mode_of(target)
            try:
                with mock.patch("os.unlink", side_effect=OSError("folder is locked")) as unlink:
                    with self.assertRaises(OSError):
                        rn.rmtree(target)
                # The first try and the retry after the permission fix both ran.
                self.assertGreaterEqual(unlink.call_count, 2)
                self.assertTrue(os.path.lexists(target))
                # A permission fix adds permissions. It never strips them.
                self.assertEqual(mode_of(target), before)
            finally:
                if target.exists():
                    os.chmod(target, 0o755)

    @unittest.skipUnless(os.name == "nt", "junctions exist only on Windows")
    def test_a_top_level_junction_to_an_outside_folder_is_unlinked_not_followed(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            outside, keep = make_outside(root)
            before = mode_of(outside)
            link = root / "dev-ci"
            make_junction(link, outside)
            self.assertTrue(os.path.isjunction(link))
            rn.rmtree(link)
            self.assertFalse(os.path.lexists(link))
            self.assertEqual(keep.read_text(encoding="utf-8"), "keep\n")
            self.assertEqual(mode_of(outside), before)


def make_readonly_outside(root: Path) -> tuple[Path, Path]:
    """A folder outside the target under test, with one read-only 8000-byte file in it."""
    outside = root / "outside"
    outside.mkdir()
    readonly = outside / "readonly.bin"
    readonly.write_bytes(b"r" * 8000)
    os.chmod(readonly, 0o444)
    return outside, readonly


def outside_state(outside: Path) -> tuple:
    """The folder's mode and, for each entry in it, its name, mode and bytes."""
    return (mode_of(outside),
            sorted((entry.name, mode_of(entry), entry.read_bytes()) for entry in outside.iterdir()))


class NestedLinkCases:
    """A link inside a target folder, to a folder outside it that holds a read-only file.

    The link is one level down (`dev-ci/sub/link`). Removal and measurement
    must not go through it: the outside folder, its file, the file's bytes
    and both modes stay the same, and the outside bytes are not counted.
    Each subclass makes one kind of link.
    """

    def make_dir_link(self, link: Path, target: Path) -> None:
        raise NotImplementedError

    def make_case(self, root: Path, size: int) -> tuple[Path, Path, Path]:
        outside, readonly = make_readonly_outside(root)
        self.assertEqual(mode_of(readonly), 0o444)
        target = make_target(root, "dev-ci", size)
        (target / "sub").mkdir()
        self.make_dir_link(target / "sub" / "link", outside)
        # The link works: the outside file is visible through it.
        self.assertEqual((target / "sub" / "link" / "readonly.bin").read_bytes(), b"r" * 8000)
        return outside, readonly, target

    def test_rmtree_removes_the_link_and_leaves_the_outside_folder_unchanged(self):
        with tempfile.TemporaryDirectory() as tmp:
            outside, readonly, target = self.make_case(Path(tmp), 500)
            before = outside_state(outside)
            rn.rmtree(target)
            self.assertFalse(os.path.lexists(target))
            self.assertEqual(outside_state(outside), before)
            self.assertEqual(mode_of(readonly), 0o444)
            self.assertEqual(readonly.read_bytes(), b"r" * 8000)

    def test_dir_size_does_not_count_the_outside_bytes(self):
        with tempfile.TemporaryDirectory() as tmp:
            outside, _readonly, target = self.make_case(Path(tmp), 10)
            before = outside_state(outside)
            self.assertEqual(rn.dir_size(target), 10)
            self.assertEqual(outside_state(outside), before)

    def test_the_guard_removes_the_target_and_leaves_the_outside_folder_unchanged(self):
        with tempfile.TemporaryDirectory() as tmp:
            outside, readonly, target = self.make_case(Path(tmp), 5000)
            before = outside_state(outside)
            logs: list[str] = []
            with mock.patch.object(rn, "log", logs.append):
                reason = rn.disk_guard([target], 1000, 1000, free_of=lambda _path: 10**7)
            self.assertIsNone(reason)
            self.assertFalse(os.path.lexists(target))
            self.assertEqual(len(logs), 1)
            self.assertIn("disk guard: removed", logs[0])
            # The size was read. The log does not use the size-unknown path.
            self.assertIn("(over the cap: size", logs[0])
            self.assertNotIn("size could not be read", logs[0])
            self.assertEqual(outside_state(outside), before)
            self.assertEqual(mode_of(readonly), 0o444)

    def test_the_guard_logs_no_removal_when_the_target_stays(self):
        with tempfile.TemporaryDirectory() as tmp:
            outside, readonly, target = self.make_case(Path(tmp), 5000)
            before = outside_state(outside)
            logs: list[str] = []
            with mock.patch.object(rn, "log", logs.append), \
                    mock.patch("os.rmdir", side_effect=OSError("folder is locked")):
                with self.assertRaises(OSError):
                    rn.disk_guard([target], 1000, 1000, free_of=lambda _path: 10**7)
            self.assertTrue(os.path.lexists(target))
            self.assertEqual(logs, [])
            self.assertEqual(outside_state(outside), before)
            self.assertEqual(mode_of(readonly), 0o444)


class NestedSymlinkTests(NestedLinkCases, unittest.TestCase):
    def make_dir_link(self, link: Path, target: Path) -> None:
        make_symlink(link, target, directory=True)

    def test_a_nested_symlink_to_an_outside_file_is_not_followed_or_counted(self):
        with tempfile.TemporaryDirectory() as tmp:
            outside, readonly, target = self.make_case(Path(tmp), 10)
            make_symlink(target / "sub" / "filelink", readonly)
            before = outside_state(outside)
            self.assertEqual(rn.dir_size(target), 10)
            rn.rmtree(target)
            self.assertFalse(os.path.lexists(target))
            self.assertEqual(outside_state(outside), before)


@unittest.skipUnless(os.name == "nt", "junctions exist only on Windows")
class NestedJunctionTests(NestedLinkCases, unittest.TestCase):
    def make_dir_link(self, link: Path, target: Path) -> None:
        make_junction(link, target)
        self.assertTrue(os.path.isjunction(link))
        self.assertFalse(os.path.islink(link))


class CatchUpTests(unittest.TestCase):
    def test_catch_up_writes_the_summary_without_running_the_disk_guard(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            over_cap = make_target(root / "target", "dev-ci", 5000)

            def fake_update_tree(_branch, tree, _build_log):
                catalog = tree / rn.CATALOG_RS
                catalog.parent.mkdir(parents=True)
                catalog.write_text(
                    'const REGISTRY: &[Entry] = &[\n    Entry::new("walk"),\n];\n', encoding="utf-8")
                return "b" * 40, "b" * 9

            guard = mock.Mock(side_effect=AssertionError("the disk guard ran during catch-up"))
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
                mock.patch.object(rn, "guard_disk", guard),
                mock.patch.object(rn, "log", lambda _message: None),
                # main() self-updates the runner checkout before anything else.
                # The catch-up path under test never reaches the network: run
                # the test with the update disabled, whatever the ambient
                # environment opts into, so the real checkout is untouched.
                mock.patch.object(rn, "maybe_self_update",
                                  lambda *args, **kwargs: {"status": "disabled", "from": "test",
                                                           "to": "test",
                                                           "detail": "self-update disabled in tests"}),
                mock.patch.object(sys, "argv", ["run_nightly.py", "--catch-up", "--branch", "dev"]),
            ]
            for patch in patches:
                patch.start()
            try:
                # main() holds the lock file open for the life of the
                # process. In this test process that only warns at return.
                with warnings.catch_warnings():
                    warnings.simplefilter("ignore", ResourceWarning)
                    result = rn.main()
                    gc.collect()
            finally:
                for patch in patches:
                    patch.stop()

            self.assertEqual(result, 0)
            guard.assert_not_called()
            summaries = sorted((root / "runs").glob("*/summary.json"))
            self.assertEqual(len(summaries), 1)
            run_dir = summaries[0].parent
            summary = json.loads(summaries[0].read_text(encoding="utf-8"))
            self.assertIn("console session not active", summary["unavailable"])
            self.assertTrue(summary["lanes"])
            self.assertEqual({row["result"] for row in summary["lanes"]}, {"UNAVAILABLE"})
            self.assertEqual({row["result"] for row in summary["cells"]}, {"UNAVAILABLE"})
            self.assertFalse(summary["build"]["ran"])
            self.assertTrue((run_dir / "DONE").is_file())
            # The guard did not run, so nothing was removed either. The next
            # scheduled night removes the over-cap target.
            self.assertTrue(os.path.lexists(over_cap))


class LaneEnvironmentTests(unittest.TestCase):
    def asset_env(self, root: Path, *, snapshot: bool, baseline: bool) -> dict:
        nav_snapshot = root / "nav-snapshot"
        if snapshot:
            snap_dir = nav_snapshot / "0123456789abcdef"
            snap_dir.mkdir(parents=True)
            (snap_dir / "versionlist").write_bytes(b"v")
        baseline_path = root / "inspect.fb"
        if baseline:
            baseline_path.write_bytes(b"capture")
        with mock.patch.object(rn, "NAV_SNAPSHOT", nav_snapshot), \
                mock.patch.object(rn, "INSPECT_BASELINE", baseline_path):
            return rn.lane_asset_env()

    def test_every_test_lane_receives_the_asset_paths(self):
        with tempfile.TemporaryDirectory() as tmp:
            expected = self.asset_env(Path(tmp), snapshot=True, baseline=True)
        pack = str(rn.NAV_FIXTURE)
        self.assertEqual(expected["NAV_PACK"], pack)
        self.assertEqual(expected["WORLD_NAV_PACK"], pack)
        self.assertEqual(expected["NAV_ARRIVAL_PACK"], pack)
        self.assertEqual(expected["ENGINE_DIR"], str(rn.ENGINE_PRISTINE / "engine"))
        self.assertEqual(expected["WORLD_ENGINE_DIR"], str(rn.ENGINE_PRISTINE / "engine"))
        self.assertIn("BOT_NAV_SNAPSHOT_ROOT", expected)
        self.assertIn("T5W_INSPECT_CAPTURE_BASELINE", expected)
        with mock.patch.object(rn, "lane_asset_env", return_value=dict(expected)):
            lanes = {name: extra for name, _args, extra in rn.lane_list(None)}
        for name, extra in lanes.items():
            with self.subTest(lane=name):
                if name.startswith("test-"):
                    self.assertEqual(extra, expected)
                else:
                    self.assertEqual(extra, {})

    def test_missing_snapshot_and_baseline_stay_unset(self):
        with tempfile.TemporaryDirectory() as tmp:
            env = self.asset_env(Path(tmp), snapshot=False, baseline=False)
        self.assertNotIn("BOT_NAV_SNAPSHOT_ROOT", env)
        self.assertNotIn("T5W_INSPECT_CAPTURE_BASELINE", env)
        self.assertEqual(env["NAV_PACK"], str(rn.NAV_FIXTURE))


class SeedLaneHomeTests(unittest.TestCase):
    def make_snapshot(self, root: Path, name: str) -> Path:
        snap = root / name
        snap.mkdir(parents=True)
        (snap / "versionlist").write_bytes(b"v")
        (snap / "config").write_bytes(b"c")
        return snap

    def test_seeds_from_the_nightly_snapshot_folder(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.make_snapshot(root / "nav-snapshot", "0123456789abcdef")
            home = root / "home"
            copied = rn.seed_lane_home(home, root / "nav-snapshot", root / "no-fallback")
            self.assertEqual(copied, 1)
            seeded = home / ".274bot" / "unpack" / "0123456789abcdef" / "versionlist"
            self.assertTrue(seeded.is_file())

    def test_falls_back_to_the_runner_users_unpack_289(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.make_snapshot(root / "unpack-289", "37214163f1e6ceca")
            (root / "unpack-289" / "not-a-snapshot").mkdir(parents=True)
            home = root / "home"
            copied = rn.seed_lane_home(home, root / "empty", root / "unpack-289")
            self.assertEqual(copied, 1)
            seeded = home / ".274bot" / "unpack" / "37214163f1e6ceca" / "config"
            self.assertTrue(seeded.is_file())
            self.assertFalse((home / ".274bot" / "unpack" / "not-a-snapshot").exists())


class CellCapTests(unittest.TestCase):
    def test_long_cells_keep_a_cap_above_their_proven_walls(self):
        # 2026-10-21 summary.json walls: jive_kbd 2543.0 s (Linux verdict),
        # quester_queue 1987.9 s (Linux pass).
        self.assertGreater(rn.cell_cap_s("jive_kbd"), 2543)
        self.assertGreater(rn.cell_cap_s("quester_queue"), 1988)
        self.assertEqual(rn.cell_cap_s("walk"), rn.CELL_CAP_S)

    def test_parse_cell_caps_overrides_and_refuses_bad_entries(self):
        caps = rn.parse_cell_caps("jive_kbd=4000, other=900", {"jive_kbd": 3600})
        self.assertEqual(caps, {"jive_kbd": 4000, "other": 900})
        self.assertEqual(rn.parse_cell_caps("", {"a": 1}), {"a": 1})
        for bad in ("jive_kbd", "jive_kbd=soon", "jive_kbd=0", "=10"):
            with self.subTest(entry=bad):
                with self.assertRaises(ValueError):
                    rn.parse_cell_caps(bad, {})


class ReceiptTests(unittest.TestCase):
    @staticmethod
    def receipt(outcome: str, scenario: str = "cell") -> dict:
        return {
            "scenario": scenario,
            "outcome": outcome,
            "ticks": 58,
            "message": "engine patch missing",
            "predicate": "ingame_scene_2",
        }

    def test_find_receipt_accepts_supported_live_outcomes(self):
        for outcome in ("PASS", "FAIL", "UNAVAILABLE"):
            with self.subTest(outcome=outcome):
                receipt = self.receipt(outcome)
                record = json.dumps(receipt, separators=(",", ":"))
                self.assertEqual(rn.find_receipt(f"FAIL: live cell {record}", "cell"), receipt)

    def test_find_receipt_ignores_other_cells_and_unknown_outcomes(self):
        records = (
            json.dumps(self.receipt("UNAVAILABLE", scenario="other"), separators=(",", ":")),
            json.dumps(self.receipt("NOT_RUN"), separators=(",", ":")),
        )
        self.assertIsNone(rn.find_receipt("\n".join(records), "cell"))

    def test_unavailable_receipt_records_reason_without_failure(self):
        result = rn.CellResult(
            cell="cell",
            result="ERROR",
            branch="dev",
            commit="e9b575",
            platform="windows",
        )
        receipt = self.receipt("UNAVAILABLE")
        rn.record_receipt(result, receipt)
        self.assertEqual(result.result, "UNAVAILABLE")
        self.assertEqual(result.reason, "engine patch missing")
        self.assertIsNone(result.first_failure)
        self.assertEqual(result.ticks, 58)
        self.assertEqual(result.receipt, receipt)

    def apply_outcome(self, frontend: str, stderr_text: str, *, timed_out: bool = False) -> rn.CellResult:
        result = rn.CellResult(
            cell="cell",
            result="ERROR",
            branch="dev",
            commit="e9b575",
            platform="windows",
            frontend=frontend,
        )
        rn.apply_cell_outcome(result, "cell", frontend, stderr_text, stderr_text, "",
                              timed_out, False, 1200)
        return result

    def test_unavailable_line_wins_over_the_fail_verdict_in_each_frontend(self):
        # The lines each frontend prints for a missing engine patch, with
        # the JSON record damaged so only the fallback can read them.
        message = "UNAVAILABLE: engine patch tools/x.patch is not installed"
        for frontend, live_name in (("tui", "cell"), ("panel", "script_cell")):
            with self.subTest(frontend=frontend):
                stderr_text = (f"FAIL: live {live_name} {{\"scenario\":\"cell\",\"outcome\":\"UNAVAILABLE\"\n"
                               f"FAIL: {message}\n")
                result = self.apply_outcome(frontend, stderr_text)
                self.assertEqual(result.result, "UNAVAILABLE")
                self.assertEqual(result.reason, message)
                self.assertIsNone(result.first_failure)

    def test_panel_verdict_line_keeps_its_fail_without_a_receipt(self):
        result = self.apply_outcome("panel", "FAIL: live script_cell {damaged\n")
        self.assertEqual(result.result, "FAIL")
        self.assertEqual(result.reason, "verdict line without a parsable JSON record")
        self.assertIn("FAIL: live script_cell", result.first_failure)

    def test_timeout_names_the_cap_that_ran(self):
        result = self.apply_outcome("tui", "289bot: still working\n", timed_out=True)
        self.assertEqual(result.result, "TIMEOUT")
        self.assertIn("runner cap 1200s reached", result.first_failure)


class EnginePreflightTests(unittest.TestCase):
    EXPECTED = "5f8a00094e8eb02dd00d703e99a073071d0349a8731f43dffa0bdaae36fa6c69"
    RESPONSE = bytes.fromhex(
        "00000000de5b33456026f8fe075503099a13636eca2717bd368f17921b1fb6b2a7129379cbff3b0d"
    )

    def run_check(self, *, body=None, error=None):
        opener = mock.Mock()
        if error is None:
            response = mock.MagicMock()
            response.__enter__.return_value = response
            response.read.return_value = body
            opener.open.return_value = response
        else:
            opener.open.side_effect = error

        with mock.patch.object(rn.urllib.request, "build_opener", return_value=opener), \
                mock.patch.object(rn.socket, "create_connection") as create_connection, \
                mock.patch.object(rn, "ENGINE_HOST", "127.0.0.1"), \
                mock.patch.object(rn, "GAME_PORT", 47594), \
                mock.patch.object(rn, "HTTP_PORT", 3080), \
                mock.patch.object(rn, "ENGINE_LABEL", "Engine B"), \
                mock.patch.object(rn, "ENGINE_CRC_SHA256", self.EXPECTED):
            game_socket = mock.Mock()
            create_connection.return_value = game_socket
            result = rn.engine_check()
        return result + (create_connection, game_socket, opener)

    def test_approved_engine_b_identity_passes(self):
        self.assertEqual(hashlib.sha256(self.RESPONSE).hexdigest(), self.EXPECTED)
        ok, detail, connect, game_socket, opener = self.run_check(body=self.RESPONSE)
        self.assertTrue(ok)
        self.assertIn("Engine B at 127.0.0.1:47594/3080", detail)
        self.assertIn("sha256 5f8a00094e8eb02d", detail)
        opener.open.assert_called_once_with("http://127.0.0.1:3080/crc", timeout=15)
        connect.assert_called_once_with(("127.0.0.1", 47594), timeout=15)
        game_socket.close.assert_called_once()

    def test_wrong_digest_is_an_identity_mismatch(self):
        body = self.RESPONSE[:-1] + bytes((self.RESPONSE[-1] ^ 1,))
        ok, detail, connect, _game_socket, _opener = self.run_check(body=body)
        self.assertFalse(ok)
        self.assertIn("identity mismatch", detail)
        with mock.patch.object(rn, "ENGINE_LABEL", "Engine B"):
            reason = rn.remote_engine_failure(detail)
        self.assertIn("Engine B identity mismatch", reason)
        self.assertNotIn("not reachable", reason)
        connect.assert_not_called()

    def test_wrong_length_is_an_identity_mismatch(self):
        ok, detail, connect, _game_socket, _opener = self.run_check(body=self.RESPONSE[:-1])
        self.assertFalse(ok)
        self.assertIn("identity mismatch", detail)
        self.assertIn("39 bytes", detail)
        connect.assert_not_called()

    def test_connection_failure_is_not_an_identity_mismatch(self):
        ok, detail, connect, _game_socket, _opener = self.run_check(
            error=OSError("connection refused")
        )
        self.assertFalse(ok)
        self.assertIn("connection refused", detail)
        self.assertNotIn("identity mismatch", detail)
        with mock.patch.object(rn, "ENGINE_LABEL", "Engine B"):
            reason = rn.remote_engine_failure(detail)
        self.assertEqual(reason, f"Engine B not reachable: {detail}")
        connect.assert_not_called()


class RevisionDefaultsTests(unittest.TestCase):
    def test_289_defaults_keep_non_windows_identity_and_pin_windows_engine_b(self):
        linux = rn.revision_defaults("289", False)
        self.assertEqual(linux["profile"], "local-289")
        self.assertEqual(linux["unpack_dir"], "unpack-289")
        self.assertEqual(linux["run_suffix"], "")
        self.assertEqual(
            linux["engine_crc_sha256"],
            "687190c8ef08bb94a1422f4262d62bb7a76a55dfe0634ea43f0e63e0235dc623")
        windows = rn.revision_defaults("289", True)
        self.assertEqual(windows["run_suffix"], "-windows")
        self.assertEqual(
            windows["engine_crc_sha256"],
            "5f8a00094e8eb02dd00d703e99a073071d0349a8731f43dffa0bdaae36fa6c69")
        for job in ("main", "quest"):
            with self.subTest(job=job):
                settings = rn.job_defaults(job, True, "289", Path("C:/nightly"))
                self.assertEqual(settings["label"], "Engine B")
                self.assertEqual(settings["game_port"], 47594)
                self.assertEqual(settings["http_port"], 3080)

    def test_274_defaults_select_the_274_lane(self):
        linux = rn.revision_defaults("274", False)
        self.assertEqual(linux["profile"], "local-274")
        self.assertEqual(linux["unpack_dir"], "unpack")
        self.assertEqual(linux["run_suffix"], "-274")
        self.assertEqual(linux["engine_crc_sha256"], "")
        self.assertEqual(rn.revision_defaults("274", True)["run_suffix"], "-274-windows")

    def test_an_unknown_revision_is_refused(self):
        with self.assertRaises(ValueError):
            rn.revision_defaults("377", False)


class RevisionBuildOverlayTests(unittest.TestCase):
    def test_274_uses_the_runner_inventory_for_its_build_tree(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "runner" / "crates" / "nav-planner" / "src" / "required-content-274.tsv"
            target = root / "tree" / "crates" / "nav-planner" / "src" / "required-content-274.tsv"
            source.parent.mkdir(parents=True)
            target.parent.mkdir(parents=True)
            source.write_text("pinned content inventory\n", encoding="utf-8")
            target.write_text("dev inventory\n", encoding="utf-8")
            with mock.patch.object(rn, "REVISION", "274"), \
                    mock.patch.object(rn, "NAV_274_REQUIRED_CONTENT", source), \
                    mock.patch.object(rn, "log"):
                rn.apply_274_inventory_overlay(root / "tree")
            self.assertEqual(target.read_text(encoding="utf-8"), "pinned content inventory\n")

    def test_289_does_not_require_or_apply_the_274_inventory(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            target = root / "tree" / "crates" / "nav-planner" / "src" / "required-content-274.tsv"
            target.parent.mkdir(parents=True)
            target.write_text("dev inventory\n", encoding="utf-8")
            with mock.patch.object(rn, "REVISION", "289"), \
                    mock.patch.object(rn, "NAV_274_REQUIRED_CONTENT", root / "missing"), \
                    mock.patch.object(rn, "log"):
                rn.apply_274_inventory_overlay(root / "tree")
            self.assertEqual(target.read_text(encoding="utf-8"), "dev inventory\n")

    def test_274_fails_closed_when_its_inventory_is_missing(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            target = root / "tree" / "crates" / "nav-planner" / "src" / "required-content-274.tsv"
            target.parent.mkdir(parents=True)
            target.write_text("dev inventory\n", encoding="utf-8")
            with mock.patch.object(rn, "REVISION", "274"), \
                    mock.patch.object(rn, "NAV_274_REQUIRED_CONTENT", root / "missing"), \
                    mock.patch.object(rn, "log"):
                with self.assertRaisesRegex(RuntimeError, "revision-274 nav inventory missing"):
                    rn.apply_274_inventory_overlay(root / "tree")
            self.assertEqual(target.read_text(encoding="utf-8"), "dev inventory\n")


class RevisionImportTests(unittest.TestCase):
    def import_constants(self, extra_env: dict) -> subprocess.CompletedProcess:
        env = {key: value for key, value in os.environ.items() if not key.startswith("NIGHTLY_")}
        env.update(extra_env)
        return subprocess.run(
            [sys.executable, "-c",
             "import run_nightly as r; print(r.REVISION, r.PROFILE, r.UNPACK_DIR, "
             "r.RUN_SUFFIX or '-', r.ENGINE_CRC_SHA256[:8] or '-')"],
            cwd=HERE, env=env, capture_output=True, text=True)

    @unittest.skipIf(os.name == "nt", "the 274 lane is a Linux guest")
    def test_the_default_revision_is_289(self):
        out = self.import_constants({})
        self.assertEqual(out.returncode, 0, out.stderr)
        suffix = "-mac" if rn.IS_MAC else "-"
        self.assertEqual(out.stdout.split(), ["289", "local-289", "unpack-289", suffix, "687190c8"])

    @unittest.skipIf(os.name == "nt", "the 274 lane is a Linux guest")
    def test_revision_274_selects_the_lane(self):
        out = self.import_constants({"NIGHTLY_REVISION": "274"})
        self.assertEqual(out.returncode, 0, out.stderr)
        suffix = "-274-mac" if rn.IS_MAC else "-274"
        self.assertEqual(out.stdout.split(), ["274", "local-274", "unpack", suffix, "-"])

    def test_an_unknown_revision_stops_the_runner(self):
        out = self.import_constants({"NIGHTLY_REVISION": "377"})
        self.assertNotEqual(out.returncode, 0)
        self.assertIn("NIGHTLY_REVISION", out.stderr)


class CellsFileTests(unittest.TestCase):
    def test_a_relative_cells_file_is_read_beside_the_script(self):
        self.assertEqual(rn.cells_file_path(None, "cells-274.conf"), rn.HERE / "cells-274.conf")
        self.assertEqual(rn.cells_file_path(Path("mine.conf"), "cells-274.conf"),
                         rn.HERE / "mine.conf")
        absolute = Path("/tmp/cells.conf") if os.name != "nt" else Path(r"C:\cells.conf")
        self.assertEqual(rn.cells_file_path(absolute, None), absolute)
        self.assertIsNone(rn.cells_file_path(None, None))
        self.assertIsNone(rn.cells_file_path(None, ""))

    def run_with_cells(self, only, only_exact):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)

        def fake_update_tree(_branch, tree, _build_log):
            catalog = tree / rn.CATALOG_RS
            catalog.parent.mkdir(parents=True)
            catalog.write_text(
                'const REGISTRY: &[Entry] = &[\n    Entry::new("walk"),\n'
                '    Entry::new("cook_bot"),\n];\n', encoding="utf-8")
            return "b" * 40, "b" * 9

        patches = [
            mock.patch.object(rn, "ROOT", root),
            mock.patch.object(rn, "CATALOG", root / "rs2b0t"),
            mock.patch.object(rn, "update_tree", fake_update_tree),
            mock.patch.object(rn, "log", lambda _message: None),
        ]
        for patch in patches:
            patch.start()
        try:
            run_dir = rn.run_branch("dev", 1, only, None, unavailable="test night off",
                                    only_exact=only_exact,
                                    cells_file="cells-274.conf" if only_exact else None)
        finally:
            for patch in patches:
                patch.stop()
        return json.loads((run_dir / "summary.json").read_text(encoding="utf-8"))

    def test_a_cells_file_name_outside_the_catalog_fails_the_run(self):
        summary = self.run_with_cells(["walk", "nope"], only_exact=True)
        self.assertIn("names not in the catalog: nope", summary["build"]["error"])
        self.assertEqual([row["cell"] for row in summary["cells"]], ["walk"])
        self.assertEqual(summary["revision"], 289)
        self.assertEqual(summary["cells_file"], "cells-274.conf")

    def test_a_hand_cell_list_still_drops_unknown_names_quietly(self):
        summary = self.run_with_cells(["walk", "nope"], only_exact=False)
        self.assertIsNone(summary["build"]["error"])
        self.assertEqual([row["cell"] for row in summary["cells"]], ["walk"])
        self.assertIsNone(summary["cells_file"])


class QuestJobTests(unittest.TestCase):
    def test_catalog_cells_are_partitioned_and_explicit_selection_is_intersected(self):
        catalog = ["walk", "quester_path", "cook", "quester_hint"]
        self.assertEqual(rn.select_job_cells(catalog, None, "main"), ["walk", "cook"])
        self.assertEqual(rn.select_job_cells(catalog, None, "quest"), ["quester_path", "quester_hint"])
        self.assertEqual(rn.select_job_cells(catalog, ["quester_hint", "walk"], "main"), ["walk"])
        self.assertEqual(rn.select_job_cells(catalog, ["quester_hint", "walk"], "quest"), ["quester_hint"])

    def test_quest_job_uses_engine_q_and_distinct_run_identity(self):
        root = Path(tempfile.gettempdir()) / "nightly-job-defaults"
        main = rn.job_defaults("main", False, "289", root, mac=True)
        quest = rn.job_defaults("quest", False, "289", root, mac=True)
        self.assertEqual((main["host"], main["game_port"], main["http_port"]), ("127.0.0.1", 44594, 1080))
        self.assertEqual(main["run_suffix"], "-mac")
        self.assertEqual(main["tick_ms"], None)
        self.assertEqual((quest["host"], quest["game_port"], quest["http_port"]), ("127.0.0.1", 44694, 1180))
        self.assertEqual(quest["profile"], "local-289q")
        self.assertEqual(quest["run_suffix"], "-mac-quest")
        self.assertNotEqual(main["name_prefix"], quest["name_prefix"])
        self.assertEqual(quest["tick_ms"], 300)

    def test_windows_quest_stays_on_engine_b_without_the_tick_override(self):
        quest = rn.job_defaults("quest", True, "289", Path(tempfile.gettempdir()))
        self.assertEqual((quest["game_port"], quest["http_port"], quest["label"]), (47594, 3080, "Engine B"))
        self.assertEqual(quest["tick_ms"], None)

    def test_isolated_cells_keep_the_configured_guest_root_without_other_host_environment(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with mock.patch.object(rn, "ISOHOME", "/tmp/isohome"), \
                    mock.patch.dict(os.environ, {"ISOHOME_ROOT": str(root / "isohome"),
                                                 "UNRELATED_SECRET": "not-for-the-cell"}):
                env = rn.cell_env(root / "home", root / "tmp", root / "evidence", "walk", "nm")

            self.assertEqual(env["ISOHOME_ROOT"], str(root / "isohome"))
            self.assertEqual(env["ISOHOME_DIR"], str(root / "home"))
            self.assertNotIn("UNRELATED_SECRET", env)

    def test_only_the_non_windows_quest_cell_environment_gets_300_ms(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            args = (root / "home", root / "tmp", root / "evidence", "quester_path", "nq")
            with mock.patch.object(rn, "JOB_DEFAULTS", {"tick_ms": None}), \
                    mock.patch.dict(os.environ, {"QUESTER_TICK_MS": "300"}):
                main_env = rn.cell_env(*args)
                lane = rn.lane_env(root / "home", root / "tmp", root / "target", {})
            self.assertNotIn("QUESTER_TICK_MS", main_env)
            self.assertNotIn("QUESTER_TICK_MS", lane)
            with mock.patch.object(rn, "JOB_DEFAULTS", {"tick_ms": 300}):
                quest_env = rn.cell_env(*args)
            self.assertEqual(quest_env["QUESTER_TICK_MS"], "300")


    def test_quest_cell_env_writes_its_private_custom_profile(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            home = root / "home"
            with mock.patch.object(rn, "IS_WINDOWS", False), \
                    mock.patch.object(rn, "JOB", "quest"), \
                    mock.patch.object(rn, "PROFILE", "local-289q"), \
                    mock.patch.object(rn, "REVISION", "289"), \
                    mock.patch.object(rn, "ENGINE_HOST", "127.0.0.1"), \
                    mock.patch.object(rn, "GAME_PORT", 44694), \
                    mock.patch.object(rn, "HTTP_PORT", 1180), \
                    mock.patch.object(rn, "JOB_DEFAULTS", {"tick_ms": 300}):
                env = rn.cell_env(home, root / "tmp", root / "evidence", "quester_path", "nq")

            self.assertEqual(env["QUESTER_TICK_MS"], "300")
            self.assertEqual(env["HOME"], str(home))
            self.assertEqual(env["TMPDIR"], str(root / "tmp"))
            profile = json.loads((home / ".274bot" / "servers.json").read_text(encoding="utf-8"))
            self.assertEqual(profile, {
                "schema_version": 1,
                "servers": [{
                    "name": "local-289q",
                    "revision": 289,
                    "transport": "tcp",
                    "worlds": [{
                        "number": 289,
                        "host": "127.0.0.1",
                        "port": 44694,
                        "node_id": 289,
                        "asset_host": "127.0.0.1",
                        "asset_port": 1180,
                    }],
                    "login_key": {"engine_dir": None},
                    "vault": "vault-289q",
                    "members": None,
                    "allow_plaintext_offhost": False,
                }],
            })

    def test_windows_quest_cell_env_uses_engine_b_without_custom_profile(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            home = root / "home"
            with mock.patch.object(rn, "IS_WINDOWS", True), \
                    mock.patch.object(rn, "JOB", "quest"), \
                    mock.patch.object(rn, "PROFILE", "local-289"), \
                    mock.patch.object(rn, "JOB_DEFAULTS", {"tick_ms": None}):
                env = rn.cell_env(home, root / "tmp", root / "evidence", "quester_cook", "wq")

            self.assertNotIn("QUESTER_TICK_MS", env)
            self.assertFalse((home / ".274bot" / "servers.json").exists())


class TemporaryCargoEnvironmentTests(unittest.TestCase):
    def test_builds_with_ephemeral_home_share_runner_cache_directory(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with mock.patch.object(rn, "ROOT", root):
                build = rn.build_env(root / "build")
                lane = rn.lane_env(root / "home", root / "tmp", root / "target", {})
            expected = str(root / "sccache")
            self.assertEqual(build["SCCACHE_DIR"], expected)
            self.assertEqual(lane["SCCACHE_DIR"], expected)


class MacQuietLockTests(unittest.TestCase):
    def test_lock_at_cell_boundary_records_unavailable_and_does_not_start_cell(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            lock = root / "MAC-QUIET.lock"
            lock.touch()
            run_dir, work = root / "run", root / "work"
            base = rn.CellResult(cell="", result="NOT_RUN", branch="dev", commit="abc123456",
                                 platform="macos-arm64")
            with mock.patch.object(rn, "QUIET_LOCK", lock), \
                    mock.patch.object(rn, "_quiet_lock_reason", None), \
                    mock.patch.object(rn, "log", lambda _message: None):
                result = rn.run_cell("quester_path", "tui", root / "missing-tui", run_dir, work, base,
                                     rn.time.monotonic() + 60)
            self.assertEqual(result.result, "UNAVAILABLE")
            self.assertIn("MAC-QUIET", result.reason)
            self.assertFalse(work.exists())

    @unittest.skipUnless(os.name == "posix", "live process-group test requires POSIX")
    def test_lock_stops_active_tui_and_panel_cells_as_unavailable(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            base = rn.CellResult(cell="", result="NOT_RUN", branch="dev", commit="abc123456",
                                 platform="macos-arm64")
            command = [sys.executable, "-c", "import time; time.sleep(30)"]
            for frontend in ("tui", "panel"):
                with self.subTest(frontend=frontend):
                    lock = root / f"{frontend}-MAC-QUIET.lock"
                    run_dir, work = root / f"{frontend}-run", root / f"{frontend}-work"
                    processes = []
                    timers = []
                    original_popen = rn.popen_group

                    def start_cell(cmd, **kwargs):
                        process = original_popen(cmd, **kwargs)
                        processes.append(process)
                        timer = threading.Timer(0.2, lock.touch)
                        timer.start()
                        timers.append(timer)
                        return process

                    patches = [
                        mock.patch.object(rn, "QUIET_LOCK", lock),
                        mock.patch.object(rn, "_quiet_lock_reason", None),
                        mock.patch.object(rn, "IS_WINDOWS", False),
                        mock.patch.object(rn, "ISOHOME", None),
                        mock.patch.object(rn, "popen_group", side_effect=start_cell),
                        mock.patch.object(rn, "tui_cmd", return_value=command),
                        mock.patch.object(rn, "panel_cmd", return_value=command),
                        mock.patch.object(rn, "log", lambda _message: None),
                    ]
                    for patch in patches:
                        patch.start()
                    try:
                        started = rn.time.monotonic()
                        result = rn.run_cell(
                            "quester_path", frontend, root / f"{frontend}-play", run_dir, work, base,
                            rn.time.monotonic() + 60,
                        )
                        elapsed = rn.time.monotonic() - started
                    finally:
                        for patch in reversed(patches):
                            patch.stop()
                    for timer in timers:
                        timer.join(timeout=1)
                    self.assertLess(elapsed, 3)
                    self.assertEqual(result.result, "UNAVAILABLE")
                    self.assertTrue(result.reason.startswith("MAC-QUIET:"))
                    self.assertIsNone(result.first_failure)
                    self.assertIsNone(result.receipt)
                    self.assertEqual(len(processes), 1)
                    self.assertIsNotNone(processes[0].returncode)
                    self.assertNotIn(processes[0].pid, rn._children)

    @unittest.skipUnless(os.name == "posix", "fast-cell quiet-lock test requires POSIX")
    def test_quiet_lock_overrides_a_fast_failure_receipt(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            base = rn.CellResult(cell="", result="NOT_RUN", branch="dev", commit="abc123456",
                                 platform="macos-arm64")
            command = [
                sys.executable, "-c",
                "import json,time; time.sleep(0.7); "
                "print(json.dumps({'scenario':'quester_path','outcome':'FAIL','message':'late failure'}))",
            ]
            for frontend in ("tui", "panel"):
                with self.subTest(frontend=frontend):
                    lock = root / f"fast-{frontend}-MAC-QUIET.lock"
                    processes = []
                    timers = []
                    original_popen = rn.popen_group

                    def start_cell(cmd, **kwargs):
                        process = original_popen(cmd, **kwargs)
                        processes.append(process)
                        timer = threading.Timer(0.55, lock.touch)
                        timer.start()
                        timers.append(timer)
                        return process

                    patches = [
                        mock.patch.object(rn, "QUIET_LOCK", lock),
                        mock.patch.object(rn, "_quiet_lock_reason", None),
                        mock.patch.object(rn, "IS_WINDOWS", False),
                        mock.patch.object(rn, "ISOHOME", None),
                        mock.patch.object(rn, "popen_group", side_effect=start_cell),
                        mock.patch.object(rn, "tui_cmd", return_value=command),
                        mock.patch.object(rn, "panel_cmd", return_value=command),
                        mock.patch.object(rn, "log", lambda _message: None),
                    ]
                    for patch in patches:
                        patch.start()
                    try:
                        result = rn.run_cell(
                            "quester_path", frontend, root / f"fast-{frontend}-play",
                            root / f"fast-{frontend}-run", root / f"fast-{frontend}-work", base,
                            rn.time.monotonic() + 60,
                        )
                    finally:
                        for patch in reversed(patches):
                            patch.stop()
                        for timer in timers:
                            timer.join(timeout=1)

                    self.assertEqual(result.result, "UNAVAILABLE")
                    self.assertTrue(result.reason.startswith("MAC-QUIET:"))
                    self.assertIsNone(result.first_failure)
                    self.assertIsNone(result.receipt)
                    self.assertEqual(len(processes), 1)
                    self.assertEqual(processes[0].returncode, 0)

    @unittest.skipUnless(os.name == "posix", "untracked process-group test requires POSIX")
    def test_quiet_stop_ignores_an_untracked_process_group(self):
        process = rn.popen_group([sys.executable, "-c", "import time; time.sleep(30)"])
        try:
            self.assertNotIn(process.pid, rn._children)
            rn.stop_cell_process(process.pid, process)
            self.assertIsNone(process.poll())
        finally:
            rn.kill_tree(process.pid, process)
            process.wait()


@unittest.skipUnless(os.name == "posix" and Path("/bin/bash").is_file(),
                     "the Mac nightly launcher requires Bash")
class NightlyMacLauncherTests(unittest.TestCase):
    def test_launcher_runs_guest_controller(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            home = root / "home"
            config = home / ".config" / "274bot-nightly" / "mac-nightly.env"
            config.parent.mkdir(parents=True)
            runner = root / "runner"
            python = root / "capture-python"
            vm_tool = root / "mac_vm.py"
            capture = root / "arguments.txt"
            vm_tool.write_text("# test helper\n", encoding="utf-8")
            python.write_text(
                "#!/bin/sh\n"
                "printf '%s\\n' \"$@\" > \"$MAC_CAPTURE\"\n"
                "printf '%s\\n' \"$NIGHTLY_MAC_VM_TOOL\" \"$NIGHTLY_MAC_BASE\" >> \"$MAC_CAPTURE\"\n",
                encoding="utf-8",
            )
            python.chmod(0o755)
            config.write_text(
                f"NIGHTLY_MAC_RUNNER={runner}\n"
                f"NIGHTLY_MAC_PYTHON={python}\n"
                f"NIGHTLY_MAC_VM_TOOL={vm_tool}\n"
                "NIGHTLY_MAC_BASE=274bot-mac-base\n",
                encoding="utf-8",
            )

            result = subprocess.run(
                ["/bin/bash", str(HERE / "nightly-mac.sh"), "--branch", "dev"],
                capture_output=True,
                check=False,
                env={"HOME": str(home), "MAC_CAPTURE": str(capture), "PATH": "/usr/bin:/bin"},
                text=True,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            arguments = capture.read_text(encoding="utf-8").splitlines()
            self.assertEqual(
                arguments,
                [
                    "-u",
                    str(runner / "tools/nightly/nightly-mac-guest.py"),
                    "--branch",
                    "dev",
                    str(vm_tool),
                    "274bot-mac-base",
                ],
            )

    def test_launcher_requires_vm_tool_configuration(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            home = root / "home"
            config = home / ".config" / "274bot-nightly" / "mac-nightly.env"
            config.parent.mkdir(parents=True)
            config.write_text(
                f"NIGHTLY_MAC_RUNNER={root / 'runner'}\n"
                f"NIGHTLY_MAC_PYTHON={root / 'python'}\n",
                encoding="utf-8",
            )

            result = subprocess.run(
                ["/bin/bash", str(HERE / "nightly-mac.sh")],
                capture_output=True,
                check=False,
                env={"HOME": str(home), "PATH": "/usr/bin:/bin"},
                text=True,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("NIGHTLY_MAC_VM_TOOL", result.stderr)


class SourceManifestTests(unittest.TestCase):
    def test_manifest_covers_run_files_before_done(self):
        with tempfile.TemporaryDirectory() as tmp:
            run_dir = Path(tmp)
            (run_dir / "cells" / "walk").mkdir(parents=True)
            (run_dir / "summary.json").write_text("{\"commit\":\"abc\"}\n", encoding="utf-8")
            (run_dir / "cells" / "walk" / "result.json").write_text("{\"result\":\"PASS\"}\n",
                                                                    encoding="utf-8")

            rn.write_source_manifest(run_dir)
            rows = [line.split("  ", 1) for line in (run_dir / "SHA256SUMS").read_text(
                encoding="utf-8").splitlines()]

            self.assertEqual([path for _digest, path in rows],
                             ["cells/walk/result.json", "summary.json"])
            for digest, relative in rows:
                self.assertEqual(digest, rn.sha256(run_dir / relative))
            self.assertNotIn("SHA256SUMS", [path for _digest, path in rows])
            self.assertNotIn("DONE", [path for _digest, path in rows])


@unittest.skipUnless(os.name == "posix" and Path("/bin/bash").is_file()
                     and shutil.which("shasum"), "the malbec mirror requires Bash and shasum")
class MalbecMirrorTests(unittest.TestCase):
    def run_mirror(self, *, corrupt: bool) -> subprocess.CompletedProcess:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        remote = root / "remote"
        run_name = "2026-10-22-dev-abcdef123"
        run_dir = remote / run_name
        run_dir.mkdir(parents=True)
        payload = run_dir / "summary.md"
        payload.write_text("source summary\n", encoding="utf-8")
        rn.write_source_manifest(run_dir)
        (run_dir / "DONE").write_text("finished\n", encoding="utf-8")

        rsync = root / "rsync"
        rsync.write_text(
            f"#!{sys.executable}\n"
            "import os, shutil, sys\n"
            "from pathlib import Path\n"
            "args = sys.argv[1:]\n"
            "root = Path(os.environ['FAKE_REMOTE_ROOT'])\n"
            "remote_arg = args[-1] if '--list-only' in args else args[-2]\n"
            "relative = remote_arg.split(':/', 1)[1].strip('/')\n"
            "source = root / relative\n"
            "if '--list-only' in args:\n"
            "    entries = root.iterdir() if not relative else source.iterdir()\n"
            "    for entry in entries:\n"
            "        mode = 'drwxr-xr-x' if entry.is_dir() else '-rw-r--r--'\n"
            "        print(f'{mode} 1 bot bot 128 2026-10-22 00:00 {entry.name}')\n"
            "else:\n"
            "    destination = Path(args[-1])\n"
            "    destination.mkdir(parents=True, exist_ok=True)\n"
            "    for entry in source.iterdir():\n"
            "        if entry.is_file():\n"
            "            shutil.copy2(entry, destination / entry.name)\n"
            "    if os.environ.get('RSYNC_CORRUPT') == '1':\n"
            "        (destination / 'summary.md').write_text('corrupted\\n', encoding='utf-8')\n",
            encoding="utf-8",
        )
        rsync.chmod(0o755)

        home = root / "home"
        config = home / ".config" / "274bot-nightly" / "malbec-mirror.env"
        config.parent.mkdir(parents=True)
        destination = root / "mirror"
        config.write_text(
            f"NIGHTLY_MALBEC_HOST=malbec\n"
            f"NIGHTLY_MALBEC_USER=bot\n"
            f"NIGHTLY_MALBEC_KEY={root / 'key'}\n"
            f"NIGHTLY_MALBEC_KNOWN_HOSTS={root / 'known-hosts'}\n"
            f"NIGHTLY_MALBEC_DEST={destination}\n"
            f"NIGHTLY_MALBEC_RSYNC={rsync}\n",
            encoding="utf-8",
        )
        completed = subprocess.run(
            ["/bin/bash", str(HERE / "malbec-mirror-pull.sh")],
            capture_output=True,
            check=False,
            env={
                "HOME": str(home),
                "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
                "FAKE_REMOTE_ROOT": str(remote),
                "RSYNC_CORRUPT": "1" if corrupt else "0",
            },
            text=True,
        )
        self.last_destination = destination / run_name
        self.source_manifest = (run_dir / "SHA256SUMS").read_text(encoding="utf-8")
        return completed

    def test_mirror_accepts_bytes_that_match_the_source_manifest(self):
        result = self.run_mirror(corrupt=False)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("mirrored after malbec SHA256SUMS verification", result.stdout)
        self.assertTrue(self.last_destination.is_dir())
        self.assertEqual((self.last_destination / "SHA256SUMS").read_text(encoding="utf-8"),
                         self.source_manifest)
        self.assertEqual((self.last_destination / "summary.md").read_text(encoding="utf-8"),
                         "source summary\n")

    def test_mirror_rejects_bytes_that_differ_from_the_source_manifest(self):
        result = self.run_mirror(corrupt=True)
        self.assertEqual(result.returncode, 1)
        self.assertIn("malbec SHA256SUMS verification failed", result.stdout)
        self.assertFalse(self.last_destination.exists())
        self.assertEqual(list(self.last_destination.parent.glob(".incoming-*")), [])


@unittest.skipUnless(os.name == "posix" and Path("/bin/sh").is_file(),
                     "systemd nightly scripts require a POSIX shell")
class SystemdQuestGateTests(unittest.TestCase):
    def test_shared_service_keeps_zinfandel_revision_274_off_the_quest_path(self):
        service = (HERE / "systemd" / "274bot-nightly.service").read_text(encoding="utf-8")
        self.assertIn("ExecStart=/usr/bin/env NIGHTLY_JOB=main", service)
        self.assertIn("ExecStartPost=/bin/sh /srv/nightly/runner/tools/nightly/systemd/run-quest-job.sh",
                      service)
        self.assertNotIn("Environment=NIGHTLY_ENABLE_QUEST=", service)
        for timer_name in ("274bot-nightly.timer", "274bot-nightly-274.timer"):
            timer = (HERE / "systemd" / timer_name).read_text(encoding="utf-8")
            self.assertIn("Unit=274bot-nightly.service", timer)

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            capture = root / "quest-calls.txt"
            mock_bin = root / "bin"
            mock_bin.mkdir()
            python = mock_bin / "python3"
            python.write_text(
                "#!/bin/sh\n"
                "printf '%s\\t%s\\t%s\\t%s\\n' \"$NIGHTLY_JOB\" \"$NIGHTLY_REVISION\" "
                "\"$NIGHTLY_GAME_PORT\" \"$1\" >> \"$QUEST_CAPTURE\"\n",
                encoding="utf-8",
            )
            python.chmod(0o755)
            helper = HERE / "systemd" / "run-quest-job.sh"
            common_env = {
                "PATH": f"{mock_bin}:/usr/bin:/bin",
                "QUEST_CAPTURE": str(capture),
            }
            zinfandel = subprocess.run(
                ["/bin/sh", str(helper)],
                env={**common_env, "NIGHTLY_REVISION": "274"},
                capture_output=True, text=True, check=False,
            )
            self.assertEqual(zinfandel.returncode, 0, zinfandel.stderr)
            self.assertFalse(capture.exists())

            malbec = subprocess.run(
                ["/bin/sh", str(helper)],
                env={**common_env, "NIGHTLY_REVISION": "289", "NIGHTLY_ENABLE_QUEST": "1"},
                capture_output=True, text=True, check=False,
            )
            self.assertEqual(malbec.returncode, 0, malbec.stderr)
            rows = [line.split("\t") for line in capture.read_text(encoding="utf-8").splitlines()]
            self.assertEqual(rows[0][:3], ["quest", "289", "44694"])
            self.assertEqual(Path(rows[0][3]).resolve(), (HERE / "run_nightly.py").resolve())

            unsafe_zinfandel = subprocess.run(
                ["/bin/sh", str(helper)],
                env={**common_env, "NIGHTLY_REVISION": "274", "NIGHTLY_ENABLE_QUEST": "1"},
                capture_output=True, text=True, check=False,
            )
            self.assertEqual(unsafe_zinfandel.returncode, 1)
            self.assertIn("refusing quest job", unsafe_zinfandel.stderr)
            self.assertEqual(len(capture.read_text(encoding="utf-8").splitlines()), 1)


class LatencySettingsTests(unittest.TestCase):
    def test_parse_latency_bands(self):
        self.assertEqual(rn.parse_latency_bands(""), [])
        self.assertEqual(rn.parse_latency_bands("50,100,150"), [50, 100, 150])
        self.assertEqual(rn.parse_latency_bands(" 50 , 100 "), [50, 100])
        for bad in ("abc", "0", "-5", "1000", "50,50", "50.5"):
            with self.assertRaises(ValueError, msg=bad):
                rn.parse_latency_bands(bad)

    def test_latency_prefix_stays_inside_the_mint_limit(self):
        # play_bootstrap.rs parse_live_name_prefix accepts 1 to 4 characters.
        self.assertEqual(rn.latency_prefix("nt", 50), "nt50")
        self.assertEqual(rn.latency_prefix("nt", 100), "n100")
        self.assertEqual(rn.latency_prefix("nw", 150), "n150")
        self.assertEqual(rn.latency_prefix("nq", 5), "nq5")
        for rtt in (5, 50, 100, 150, 999):
            self.assertLessEqual(len(rn.latency_prefix("nt", rtt)), 4)
        with self.assertRaises(ValueError):
            rn.latency_prefix("", 50)

    def test_classify_latency_covers_each_table_row(self):
        cases = [
            # (zero, band, proxy_unavailable, expected)
            ("PASS", "PASS", False, "pass"),
            ("PASS", "FAIL", False, "latency-defect"),
            ("PASS", "TIMEOUT", False, "latency-defect"),
            ("PASS", "ERROR", False, "latency-defect"),
            ("PASS", "NOT_RUN", False, "not-run"),
            ("PASS", "UNAVAILABLE", False, "unavailable"),
            ("FAIL", "PASS", False, "also-fails-at-0"),
            ("FAIL", "FAIL", False, "also-fails-at-0"),
            ("TIMEOUT", "FAIL", False, "also-fails-at-0"),
            ("ERROR", "FAIL", False, "also-fails-at-0"),
            ("UNAVAILABLE", "FAIL", False, "no-baseline"),
            ("NOT_RUN", "FAIL", False, "no-baseline"),
            ("SKIP", "PASS", False, "no-baseline"),
            (None, "FAIL", False, "no-baseline"),
            ("PASS", "FAIL", True, "proxy-unavailable"),
            ("PASS", "UNAVAILABLE", True, "proxy-unavailable"),
            (None, "UNAVAILABLE", True, "proxy-unavailable"),
        ]
        for zero, band, proxy, expected in cases:
            self.assertEqual(rn.classify_latency(zero, band, proxy_unavailable=proxy), expected,
                             f"zero={zero} band={band} proxy={proxy}")

    def test_cell_row_omits_latency_keys_on_zero_rows_only(self):
        zero = rn.CellResult(cell="walk", result="PASS", branch="dev", commit="c", platform="p")
        self.assertNotIn("latency_band", rn.cell_row(zero))
        band = rn.CellResult(cell="walk", result="PASS", branch="dev", commit="c", platform="p",
                             latency_band="rtt50", latency_rtt_ms=50, latency_class="pass")
        row = rn.cell_row(band)
        self.assertEqual((row["latency_band"], row["latency_rtt_ms"], row["latency_class"]),
                         ("rtt50", 50, "pass"))

    def test_aggregate_proxy_stats_filters_window_and_direct_records(self):
        def record(opened, first_byte, path, p50, samples, login=None):
            summary = {"p50": p50, "p95": p50 + 5, "samples": samples} if p50 is not None else None
            return {"opened_at_epoch": opened, "first_byte": first_byte, "path": path,
                    "bytes_client_to_server": 10, "bytes_server_to_client": 20,
                    "queue_delay_ms_client_to_server": summary,
                    "queue_delay_ms_server_to_client": summary,
                    "login_first_response_ms": login}

        records = [
            record(100.0, 14, "delayed", 25.0, 10, login=55.0),
            record(101.0, 14, "delayed", 27.0, 30, login=65.0),
            record(102.0, 15, "direct", 0.1, 99),
            record(103.0, None, "none", None, 0),
            record(500.0, 14, "delayed", 75.0, 10, login=999.0),
        ]
        out = rn.aggregate_proxy_stats(records, 99.0, 200.0)
        self.assertEqual(out["records"], 4)
        self.assertEqual(out["connections"], {"game_14": 2, "ondemand_15": 1, "other": 0, "none": 1})
        self.assertEqual(out["bytes_client_to_server"], 40)
        # Sample-weighted median of (25 x10, 27 x30): the direct record never counts.
        self.assertEqual(out["client_to_server"], {"p50_ms": 27.0, "p95_ms": 32.0, "samples": 40})
        self.assertEqual(out["login_first_response_ms"], 60.0)
        empty = rn.aggregate_proxy_stats([])
        self.assertIsNone(empty["client_to_server"]["p50_ms"])
        self.assertEqual(empty["records"], 0)

    def test_remote_flags_point_the_game_port_at_the_proxy_only(self):
        with mock.patch.object(rn, "ENGINE_MODE", "remote"), \
                mock.patch.object(rn, "ENGINE_HOST", "127.0.0.1"), \
                mock.patch.object(rn, "GAME_PORT", 44594), \
                mock.patch.object(rn, "HTTP_PORT", 1080):
            self.assertEqual(rn.engine_flags(),
                             ["--host", "127.0.0.1", "--port", "44594",
                              "--asset-host", "127.0.0.1", "--http-port", "1080"])
            self.assertEqual(rn.engine_flags(49999),
                             ["--host", "127.0.0.1", "--port", "49999",
                              "--asset-host", "127.0.0.1", "--http-port", "1080"])
            cmd = rn.tui_cmd(Path("tui-play"), "walk", game_port=49999)
            self.assertIn("49999", cmd)
            self.assertNotIn("44594", cmd)
        with mock.patch.object(rn, "ENGINE_MODE", "local"):
            self.assertEqual(rn.engine_flags(49999), [])

    def test_local_mode_latency_profile_points_the_world_at_the_proxy(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            home = root / "home"
            home.mkdir()
            with mock.patch.object(rn, "ENGINE_MODE", "local"), \
                    mock.patch.object(rn, "JOB", "main"), \
                    mock.patch.object(rn, "PROFILE", "local-289"), \
                    mock.patch.object(rn, "REVISION", "289"), \
                    mock.patch.object(rn, "ENGINE_HOST", "127.0.0.1"), \
                    mock.patch.object(rn, "GAME_PORT", 44594), \
                    mock.patch.object(rn, "HTTP_PORT", 1080):
                rn.cell_env(home, root / "tmp", root / "evidence", "walk", "nt50", game_port=49999)
            profile = json.loads((home / ".274bot" / "servers.json").read_text(encoding="utf-8"))
            server = profile["servers"][0]
            self.assertEqual(server["name"], "local-289")
            self.assertEqual(server["vault"], "vault-289")
            self.assertEqual(server["worlds"], [{
                "number": 289, "host": "127.0.0.1", "port": 49999, "node_id": 289,
                "asset_host": "127.0.0.1", "asset_port": 1080}])

    def test_local_mode_without_a_latency_port_writes_no_profile(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            home = root / "home"
            home.mkdir()
            with mock.patch.object(rn, "ENGINE_MODE", "local"), \
                    mock.patch.object(rn, "JOB", "main"), \
                    mock.patch.object(rn, "PROFILE", "local-289"):
                rn.cell_env(home, root / "tmp", root / "evidence", "walk", "nt")
            self.assertFalse((home / ".274bot" / "servers.json").exists())


class LatencyMatrixTests(unittest.TestCase):
    @staticmethod
    def row(cell, result, *, ticks=None, band=None, rtt=None, cls=None):
        return rn.CellResult(cell=cell, result=result, branch="dev", commit="c", platform="p",
                             ticks=ticks, latency_band=band, latency_rtt_ms=rtt, latency_class=cls)

    def test_matrix_sorts_defects_first_and_shows_tick_delta(self):
        zero = [self.row("walk", "PASS", ticks=10), self.row("cook", "FAIL", ticks=3),
                self.row("alcher", "PASS", ticks=7)]
        latency = [
            self.row("walk", "PASS", ticks=10, band="rtt50", rtt=50, cls="pass"),
            self.row("walk", "FAIL", ticks=12, band="rtt150", rtt=150, cls="latency-defect"),
            self.row("cook", "FAIL", ticks=3, band="rtt50", rtt=50, cls="also-fails-at-0"),
            self.row("cook", "FAIL", ticks=4, band="rtt150", rtt=150, cls="also-fails-at-0"),
            self.row("alcher", "PASS", ticks=7, band="rtt50", rtt=50, cls="pass"),
            self.row("alcher", "PASS", ticks=7, band="rtt150", rtt=150, cls="pass"),
        ]
        md = rn.md_latency_matrix(zero, latency)
        self.assertEqual(md[0], "| Cell | rtt0 | rtt50 | rtt150 | Class | Ticks 0→150 |")
        body = [line for line in md if line.startswith("| `")]
        self.assertEqual([line.split("`")[1] for line in body], ["walk", "cook", "alcher"])
        self.assertIn("latency-defect", body[0])
        self.assertIn("10→12", body[0])
        self.assertIn("also-fails-at-0", body[1])


def _start_echo_server() -> tuple[socket.socket, int, threading.Thread, threading.Event]:
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    listener.bind(("127.0.0.1", 0))
    listener.listen(64)
    listener.settimeout(0.25)
    stopping = threading.Event()

    def echo(conn: socket.socket) -> None:
        try:
            while True:
                data = conn.recv(65536)
                if not data:
                    return
                conn.sendall(data)
        except OSError:
            return
        finally:
            conn.close()

    def accept() -> None:
        while not stopping.is_set():
            try:
                conn, _addr = listener.accept()
            except socket.timeout:
                continue
            except OSError:
                return
            threading.Thread(target=echo, args=(conn,), daemon=True).start()

    thread = threading.Thread(target=accept, daemon=True)
    thread.start()
    return listener, listener.getsockname()[1], thread, stopping


class LatencyPassTests(unittest.TestCase):
    """The pass against a fake cell runner and a real proxy subprocess."""

    def run_pass(self, root: Path, upstream_port: int, outcomes: dict, *,
                 traffic: bool = True, stats_p50_override: float | None = None):
        run_dir = root / "run"
        run_dir.mkdir(parents=True)
        binary = root / "tui-play"
        binary.write_bytes(b"fake-frontend")
        base = rn.CellResult(cell="", result="NOT_RUN", branch="dev", commit="c" * 9, platform="test")

        def fake_run_cell(cell, frontend, _binary, cell_run_dir, _work, cell_base, _deadline,
                          game_port=None, name_prefix=None, root_name=None, step=None):
            started = time.monotonic()
            result = rn.CellResult(**{**rn.asdict(cell_base), "cell": cell, "frontend": frontend,
                                      "renderer": "cpu", "started": rn.now_iso()})
            if traffic:
                with socket.create_connection(("127.0.0.1", game_port), timeout=10) as sock:
                    sock.sendall(bytes([14]) + b"ping")
                    echoed = b""
                    while len(echoed) < 5:
                        echoed += sock.recv(5 - len(echoed))
            result.result, result.ticks = outcomes[cell]
            result.wall_s = round(time.monotonic() - started, 3)
            cell_dir = cell_run_dir / root_name / cell
            cell_dir.mkdir(parents=True, exist_ok=True)
            return rn.write_cell(cell_dir, result)

        zero = {
            "walk": rn.CellResult(cell="walk", result="PASS", branch="dev", commit="c" * 9,
                                  platform="test", ticks=10),
            "cook": rn.CellResult(cell="cook", result="PASS", branch="dev", commit="c" * 9,
                                  platform="test", ticks=4),
            "bank": rn.CellResult(cell="bank", result="FAIL", branch="dev", commit="c" * 9,
                                  platform="test", ticks=1),
        }
        patches = [
            mock.patch.object(rn, "ENGINE_HOST", "127.0.0.1"),
            mock.patch.object(rn, "GAME_PORT", upstream_port),
            mock.patch.object(rn, "ENGINE_MODE", "remote"),
            mock.patch.object(rn, "REQUIRE_CONSOLE", False),
            mock.patch.object(rn, "NAME_PREFIX", "nt"),
            mock.patch.object(rn, "engine_start", lambda _run_dir: (True, "fake engine")),
            mock.patch.object(rn, "engine_stop", lambda _run_dir: None),
            mock.patch.object(rn, "run_cell", fake_run_cell),
            mock.patch.object(rn, "log", lambda _message: None),
        ]
        if stats_p50_override is not None:
            real_read = rn.read_proxy_stats

            def off_band(path):
                rows = real_read(path)
                for row in rows:
                    for field in ("queue_delay_ms_client_to_server",
                                  "queue_delay_ms_server_to_client"):
                        if row.get(field):
                            row[field] = {**row[field], "p50": stats_p50_override}
                return rows

            patches.append(mock.patch.object(rn, "read_proxy_stats", off_band))
        for patch in patches:
            patch.start()
        try:
            rows, meta = rn.run_latency_pass("dev", binary, list(outcomes), [50], run_dir,
                                             root / "work", base, {}, 2, zero, None, True)
        finally:
            for patch in patches:
                patch.stop()
        return rows, meta, run_dir

    def test_fake_engine_run_writes_sidecars_classes_and_rows(self):
        listener, echo_port, thread, stopping = _start_echo_server()
        try:
            with tempfile.TemporaryDirectory() as tmp:
                rows, meta, run_dir = self.run_pass(
                    Path(tmp), echo_port,
                    {"walk": ("PASS", 10), "cook": ("FAIL", 4), "bank": ("FAIL", 1)})
                by_cell = {row.cell: row for row in rows}
                self.assertEqual(by_cell["walk"].latency_class, "pass")
                self.assertEqual(by_cell["cook"].latency_class, "latency-defect")
                self.assertEqual(by_cell["bank"].latency_class, "also-fails-at-0")
                for row in rows:
                    cell_dir = run_dir / "cells-latency" / "rtt50" / row.cell
                    doc = json.loads((cell_dir / "latency.json").read_text(encoding="utf-8"))
                    self.assertEqual(doc["band"], "rtt50")
                    self.assertEqual(doc["configured_rtt_ms"], 50)
                    self.assertEqual(doc["configured_one_way_ms"], 25.0)
                    self.assertEqual(doc["account_prefix"], "nt50")
                    self.assertEqual(doc["latency_class"], row.latency_class)
                    self.assertIsNotNone(doc["frontend_sha256"])
                    p50 = doc["measured"]["client_to_server"]["p50_ms"]
                    self.assertLessEqual(abs(p50 - 25.0), 10.0)
                    self.assertIsNotNone(doc["login_first_response_ms"])
                    saved = json.loads((cell_dir / "result.json").read_text(encoding="utf-8"))
                    self.assertEqual(saved["latency_band"], "rtt50")
                    self.assertEqual(saved["latency_rtt_ms"], 50)
                    self.assertEqual(saved["latency_class"], row.latency_class)
                stats = run_dir / "latency" / "rtt50" / "proxy-stats.jsonl"
                self.assertGreaterEqual(len(rn.read_proxy_stats(stats)), 3)
                self.assertEqual(meta["bands"][0]["band"], "rtt50")
                self.assertTrue(meta["report_only"])
        finally:
            stopping.set()
            listener.close()
            thread.join(timeout=5)

    def test_dead_upstream_yields_proxy_unavailable_rows_and_no_cells_start(self):
        dead = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        dead.bind(("127.0.0.1", 0))
        dead_port = dead.getsockname()[1]
        dead.close()
        with tempfile.TemporaryDirectory() as tmp:
            rows, _meta, run_dir = self.run_pass(
                Path(tmp), dead_port, {"walk": ("PASS", 1)}, traffic=False)
            self.assertEqual(len(rows), 1)
            self.assertEqual(rows[0].result, "UNAVAILABLE")
            self.assertEqual(rows[0].latency_class, "proxy-unavailable")
            self.assertIn("latency proxy unavailable", rows[0].reason or "")
            doc = json.loads((run_dir / "cells-latency" / "rtt50" / "walk" / "latency.json")
                             .read_text(encoding="utf-8"))
            self.assertEqual(doc["measured"]["records"], 0)

    def test_a_pass_without_delayed_traffic_fails_the_health_rule(self):
        listener, echo_port, thread, stopping = _start_echo_server()
        try:
            with tempfile.TemporaryDirectory() as tmp:
                rows, _meta, _run_dir = self.run_pass(
                    Path(tmp), echo_port, {"walk": ("PASS", 1)}, traffic=False)
                self.assertEqual(rows[0].result, "UNAVAILABLE")
                self.assertEqual(rows[0].latency_class, "proxy-unavailable")
                self.assertIn("no delayed connection", rows[0].reason or "")
        finally:
            stopping.set()
            listener.close()
            thread.join(timeout=5)

    def test_an_off_band_measured_delay_fails_the_health_rule(self):
        listener, echo_port, thread, stopping = _start_echo_server()
        try:
            with tempfile.TemporaryDirectory() as tmp:
                # Real traffic through the real proxy, with every measured
                # p50 rewritten to 90 ms against a 25 ms one-way band.
                rows, _meta, _run_dir = self.run_pass(
                    Path(tmp), echo_port, {"walk": ("PASS", 1)}, stats_p50_override=90.0)
                self.assertEqual(rows[0].result, "UNAVAILABLE")
                self.assertEqual(rows[0].latency_class, "proxy-unavailable")
                self.assertIn("measured queue delay p50", rows[0].reason or "")
        finally:
            stopping.set()
            listener.close()
            thread.join(timeout=5)


class LatencyRunBranchTests(unittest.TestCase):
    def run_unavailable_branch(self, *, latency_bands, latency_cells):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)

        def fake_update_tree(_branch, tree, _build_log):
            catalog = tree / rn.CATALOG_RS
            catalog.parent.mkdir(parents=True)
            catalog.write_text(
                'const REGISTRY: &[Entry] = &[\n    Entry::new("walk"),\n'
                '    Entry::new("cook_bot"),\n    Entry::new("quester_path"),\n];\n',
                encoding="utf-8")
            return "b" * 40, "b" * 9

        patches = [
            mock.patch.object(rn, "ROOT", root),
            mock.patch.object(rn, "CATALOG", root / "rs2b0t"),
            mock.patch.object(rn, "update_tree", fake_update_tree),
            mock.patch.object(rn, "log", lambda _message: None),
        ]
        for patch in patches:
            patch.start()
        try:
            run_dir = rn.run_branch("dev", 1, None, None, unavailable="test night off",
                                    latency_bands=latency_bands, latency_cells=latency_cells)
        finally:
            for patch in patches:
                patch.stop()
        return run_dir, json.loads((run_dir / "summary.json").read_text(encoding="utf-8"))

    def test_lane_off_adds_no_latency_keys_anywhere(self):
        run_dir, summary = self.run_unavailable_branch(latency_bands=[], latency_cells=["walk"])
        self.assertNotIn("latency", summary)
        self.assertNotIn("latency_counts", summary)
        self.assertNotIn("latency_cells", summary)
        for row in summary["cells"]:
            self.assertNotIn("latency_band", row)
        self.assertNotIn("Latency lane", (run_dir / "summary.md").read_text(encoding="utf-8"))
        self.assertFalse((run_dir / "cells-latency").exists())
        self.assertTrue((run_dir / "DONE").is_file())

    def test_lane_on_an_unavailable_night_writes_matrix_and_sidecars(self):
        run_dir, summary = self.run_unavailable_branch(latency_bands=[50], latency_cells=["walk"])
        self.assertEqual(len(summary["latency_cells"]), 1)
        row = summary["latency_cells"][0]
        self.assertEqual((row["cell"], row["result"], row["latency_band"], row["latency_class"]),
                         ("walk", "UNAVAILABLE", "rtt50", "no-baseline"))
        self.assertEqual(summary["latency_counts"]["no-baseline"], 1)
        # The main counts still describe the zero run only.
        self.assertEqual(sum(summary["counts"].values()), len(summary["cells"]))
        self.assertTrue(summary["latency"]["report_only"])
        md = (run_dir / "summary.md").read_text(encoding="utf-8")
        self.assertIn("## Latency lane (tui, dev only, report-only)", md)
        self.assertIn("| `walk` | UNAVAILABLE | UNAVAILABLE | no-baseline |", md)
        doc = json.loads((run_dir / "cells-latency" / "rtt50" / "walk" / "latency.json")
                         .read_text(encoding="utf-8"))
        self.assertEqual(doc["configured_one_way_ms"], 25.0)
        self.assertIsNone(doc["proxy"]["listen_port"])
        self.assertTrue((run_dir / "DONE").is_file())

    def test_a_latency_name_outside_the_catalog_fails_the_run(self):
        _run_dir, summary = self.run_unavailable_branch(latency_bands=[50], latency_cells=["nope"])
        self.assertIn("latency cell list: names not in the catalog: nope", summary["build"]["error"])

    def test_a_quest_cell_in_the_latency_list_fails_the_run(self):
        _run_dir, summary = self.run_unavailable_branch(
            latency_bands=[50], latency_cells=["quester_path"])
        self.assertIn("quest cells stay out of the latency lane: quester_path",
                      summary["build"]["error"])


@unittest.skipUnless(shutil.which("git"), "git is not installed")
class Rs2b0tApiDriftTests(unittest.TestCase):
    """The rs2b0t API drift check: freeze from the tree, diff from the catalog.

    A temporary git repository stands in for upstream rs2b0t, so no test
    touches the network. The catalog checkout plays the object store: the
    check fetches upstream `main` into `refs/nightly/upstream-main` and
    never moves the catalog's checked-out commit.
    """

    def setUp(self):
        rn._api_upstream.clear()
        self.addCleanup(rn._api_upstream.clear)

    @staticmethod
    def write_api(repo: Path, marker: str) -> None:
        package = repo / "packages" / "rs2b0t-api"
        package.mkdir(parents=True, exist_ok=True)
        (package / "index.d.ts").write_text(
            f"export declare const marker: string; // {marker}\n", encoding="utf-8")
        (package / "index.js").write_text(
            f"module.exports = {{ marker: '{marker}' }};\n", encoding="utf-8")

    def commit_api(self, repo: Path, message: str, marker: str) -> str:
        self.write_api(repo, marker)
        git("add", ".", cwd=repo)
        git("-c", "user.name=test", "-c", "user.email=test@example.invalid",
            "commit", "-qm", message, cwd=repo)
        return git("rev-parse", "HEAD", cwd=repo)

    def make_repo(self, root: Path, name: str, *, branch: str = "main") -> Path:
        repo = root / name
        repo.mkdir()
        git("init", "-q", cwd=repo)
        git("symbolic-ref", "HEAD", f"refs/heads/{branch}", cwd=repo)
        return repo

    def make_tree(self, root: Path, repository: str, revision: str) -> Path:
        tree = root / "tree"
        contract = tree / "crates" / "script-js" / "compat-js"
        contract.mkdir(parents=True)
        (contract / "abi-contract.json").write_text(json.dumps({
            "frozen_source": {
                "repository": repository,
                "revision": revision,
                "runtime_declarations": "packages/rs2b0t-api/index.js",
                "typed_declarations": "packages/rs2b0t-api/index.d.ts",
            },
        }), encoding="utf-8")
        return tree

    def test_unchanged_names_both_shorts(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            upstream = self.make_repo(root, "upstream")
            frozen = self.commit_api(upstream, "api v1", "v1")
            catalog = self.make_repo(root, "catalog")
            self.commit_api(catalog, "catalog seed", "seed")
            tree = self.make_tree(root, str(upstream), frozen)

            info = rn.check_rs2b0t_api_drift(tree, catalog)

            self.assertEqual(info["status"], "unchanged")
            self.assertEqual(info["freeze_revision"], frozen)
            self.assertEqual(info["upstream_revision"], frozen)
            self.assertEqual(info["files"], {})
            self.assertEqual(info["commit_count"], 0)
            self.assertEqual(rn.api_drift_md_line(info),
                             f"rs2b0t API: unchanged since {frozen[:8]} (upstream main {frozen[:8]})")

    def test_changed_reports_file_stats_and_commits(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            upstream = self.make_repo(root, "upstream")
            frozen = self.commit_api(upstream, "api v1", "v1")
            tip = self.commit_api(upstream, "api v2", "v2")
            catalog = self.make_repo(root, "catalog")
            self.commit_api(catalog, "catalog seed", "seed")
            tree = self.make_tree(root, str(upstream), frozen)

            info = rn.check_rs2b0t_api_drift(tree, catalog)

            self.assertEqual(info["status"], "changed")
            self.assertEqual(info["freeze_revision"], frozen)
            self.assertEqual(info["upstream_revision"], tip)
            self.assertEqual(info["files"], {
                "packages/rs2b0t-api/index.d.ts": {"added": 1, "deleted": 1},
                "packages/rs2b0t-api/index.js": {"added": 1, "deleted": 1},
            })
            self.assertEqual(info["commits"], [{"sha": tip[:8], "subject": "api v2"}])
            self.assertEqual(info["commit_count"], 1)
            line = rn.api_drift_md_line(info)
            self.assertIn(f"rs2b0t API: CHANGED since freeze {frozen[:8]} (upstream main {tip[:8]})", line)
            self.assertIn("index.d.ts +1 -1", line)
            self.assertIn("index.js +1 -1", line)
            self.assertIn(f"commits: {tip[:8]} api v2", line)

    def test_commit_list_caps_at_ten(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            upstream = self.make_repo(root, "upstream")
            frozen = self.commit_api(upstream, "api v1", "v1")
            tip = frozen
            for seq in range(12):
                tip = self.commit_api(upstream, f"api change {seq}", f"v{seq}")
            catalog = self.make_repo(root, "catalog")
            self.commit_api(catalog, "catalog seed", "seed")
            tree = self.make_tree(root, str(upstream), frozen)

            info = rn.check_rs2b0t_api_drift(tree, catalog)

            self.assertEqual(info["status"], "changed")
            self.assertEqual(info["commit_count"], 12)
            self.assertEqual(len(info["commits"]), 10)
            line = rn.api_drift_md_line(info)
            self.assertIn("(+2 more)", line)
            self.assertNotIn("api change 0", line)

    def test_fetch_failure_is_unavailable(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            catalog = self.make_repo(root, "catalog")
            self.commit_api(catalog, "catalog seed", "seed")
            tree = self.make_tree(root, str(root / "no-such-upstream"), "0" * 40)

            info = rn.check_rs2b0t_api_drift(tree, catalog)

            self.assertEqual(info["status"], "unavailable")
            self.assertIsNone(info["upstream_revision"])
            self.assertTrue(info["reason"])
            line = rn.api_drift_md_line(info)
            self.assertTrue(line.startswith("rs2b0t API: UNAVAILABLE ("))
            self.assertNotIn("unchanged", line)

    def test_catalog_checkout_is_unchanged(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            upstream = self.make_repo(root, "upstream")
            frozen = self.commit_api(upstream, "api v1", "v1")
            tip = self.commit_api(upstream, "api v2", "v2")
            catalog = self.make_repo(root, "catalog")
            self.commit_api(catalog, "catalog seed", "seed")
            head_before = git("rev-parse", "HEAD", cwd=catalog)
            tree = self.make_tree(root, str(upstream), frozen)

            info = rn.check_rs2b0t_api_drift(tree, catalog)

            self.assertEqual(info["status"], "changed")
            self.assertEqual(git("rev-parse", "HEAD", cwd=catalog), head_before)
            self.assertEqual(
                git("rev-parse", rn.RS2B0T_UPSTREAM_REF, cwd=catalog), tip)

    def test_freeze_revision_comes_from_the_tree(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            upstream = self.make_repo(root, "upstream")
            old = self.commit_api(upstream, "api v1", "v1")
            tip = self.commit_api(upstream, "api v2", "v2")
            catalog = self.make_repo(root, "catalog")
            self.commit_api(catalog, "catalog seed", "seed")

            old_info = rn.check_rs2b0t_api_drift(self.make_tree(root, str(upstream), old), catalog)
            rn._api_upstream.clear()
            new_tree = root / "tree"
            contract = new_tree / "crates" / "script-js" / "compat-js" / "abi-contract.json"
            contract.write_text(json.dumps({"frozen_source": {"repository": str(upstream),
                                                              "revision": tip}}), encoding="utf-8")
            new_info = rn.check_rs2b0t_api_drift(new_tree, catalog)

            self.assertEqual(old_info["freeze_revision"], old)
            self.assertEqual(old_info["status"], "changed")
            self.assertEqual(new_info["freeze_revision"], tip)
            self.assertEqual(new_info["status"], "unchanged")

    def test_missing_contract_checks_nothing_and_reports_unavailable(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            upstream = self.make_repo(root, "upstream")
            self.commit_api(upstream, "api v1", "v1")
            catalog = self.make_repo(root, "catalog")
            self.commit_api(catalog, "catalog seed", "seed")
            tree = root / "tree"
            tree.mkdir()

            info = rn.check_rs2b0t_api_drift(tree, catalog)

            self.assertEqual(info["status"], "unavailable")
            self.assertIn("abi-contract", info["reason"])
            fetched = subprocess.run(
                ["git", "rev-parse", rn.RS2B0T_UPSTREAM_REF], cwd=catalog,
                capture_output=True, text=True)
            self.assertNotEqual(fetched.returncode, 0)

    def test_missing_tree_is_unavailable_never_unchanged(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            catalog = self.make_repo(root, "catalog")
            self.commit_api(catalog, "catalog seed", "seed")

            info = rn.check_rs2b0t_api_drift(None, catalog)

            self.assertEqual(info["status"], "unavailable")
            self.assertNotEqual(info["status"], "unchanged")

    def test_quiet_lock_before_fetch_is_unavailable_and_starts_no_fetch(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            upstream = self.make_repo(root, "upstream")
            frozen = self.commit_api(upstream, "api v1", "v1")
            catalog = self.make_repo(root, "catalog")
            self.commit_api(catalog, "catalog seed", "seed")
            tree = self.make_tree(root, str(upstream), frozen)
            lock = root / "MAC-QUIET.lock"
            lock.write_text("quiet\n", encoding="utf-8")
            old_lock = rn.QUIET_LOCK
            old_reason = rn._quiet_lock_reason
            rn.QUIET_LOCK = lock
            rn._quiet_lock_reason = None
            try:
                with mock.patch.object(rn, "log", lambda _message: None):
                    info = rn.check_rs2b0t_api_drift(tree, catalog)
            finally:
                rn.QUIET_LOCK = old_lock
                rn._quiet_lock_reason = old_reason
            self.assertEqual(info["status"], "unavailable")
            self.assertIn("quiet lock held", info.get("reason") or "")
            self.assertNotIn("unchanged", rn.api_drift_md_line(info))
            fetched = subprocess.run(
                ["git", "rev-parse", rn.RS2B0T_UPSTREAM_REF], cwd=catalog,
                capture_output=True, text=True)
            self.assertNotEqual(fetched.returncode, 0)

    @unittest.skipIf(os.name == "nt", "the fake git uses a POSIX shell and sleep")
    def test_fetch_timeout_kills_the_process_tree(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            upstream = self.make_repo(root, "upstream")
            self.commit_api(upstream, "api v1", "v1")
            catalog = self.make_repo(root, "catalog")
            self.commit_api(catalog, "catalog seed", "seed")
            real_git = shutil.which("git")
            self.assertTrue(real_git)
            pid_file = root / "grandchild.pid"
            if pid_file.exists():
                pid_file.unlink()
            fake_bin = root / "fakebin"
            fake_bin.mkdir()
            fake_git = fake_bin / "git"
            fake_git.write_text(
                "#!/bin/sh\n"
                'for a in "$@"; do if [ "$a" = "fetch" ]; then\n'
                f'  sleep 60 &\n  echo $! > "{pid_file}"\n  wait\n  exit 0\nfi; done\n'
                f'exec "{real_git}" "$@"\n',
                encoding="utf-8")
            fake_git.chmod(0o755)
            old_path = os.environ.get("PATH", "")
            os.environ["PATH"] = f"{fake_bin}{os.pathsep}{old_path}"
            try:
                with self.assertRaises(RuntimeError) as ctx:
                    rn.fetch_upstream_main(catalog, str(upstream), timeout=2)
            finally:
                os.environ["PATH"] = old_path
            self.assertIn("timed out", str(ctx.exception))
            grandchild = int(pid_file.read_text(encoding="utf-8").strip())
            time.sleep(0.5)
            try:
                with self.assertRaises(OSError):
                    os.kill(grandchild, 0)
            finally:
                try:
                    os.kill(grandchild, 9)
                except OSError:
                    pass

    def test_git_log_failure_is_unavailable(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            upstream = self.make_repo(root, "upstream")
            frozen = self.commit_api(upstream, "api v1", "v1")
            self.commit_api(upstream, "api v2", "v2")
            catalog = self.make_repo(root, "catalog")
            self.commit_api(catalog, "catalog seed", "seed")
            tree = self.make_tree(root, str(upstream), frozen)
            real_git = shutil.which("git")
            self.assertTrue(real_git)
            # A Python fake routed through `git_cmd`, not a PATH shell
            # script: Windows never runs an extensionless `git` file
            # (process creation appends .exe only), so the old shell fake
            # never intercepted there and the real `log` succeeded. The
            # fake still fails a real subprocess with the real exit code
            # and stderr, and delegates every other git call to real git.
            fake_py = root / "fake_git.py"
            fake_py.write_text(
                "import subprocess, sys\n"
                "real, args = sys.argv[1], sys.argv[2:]\n"
                "if 'log' in args:\n"
                "    sys.stderr.write('fake log failure: boom\\n')\n"
                "    sys.exit(9)\n"
                "sys.exit(subprocess.run([real, *args]).returncode)\n",
                encoding="utf-8")

            def fake_git_cmd(*args: str) -> list[str]:
                return [sys.executable, str(fake_py), real_git, *args]

            with mock.patch.object(rn, "git_cmd", side_effect=fake_git_cmd):
                info = rn.check_rs2b0t_api_drift(tree, catalog)
            self.assertEqual(info["status"], "unavailable")
            self.assertIn("9", info.get("reason") or "")
            self.assertIn("boom", info.get("reason") or "")
            line = rn.api_drift_md_line(info)
            self.assertTrue(line.startswith("rs2b0t API: UNAVAILABLE ("))
            self.assertNotIn("CHANGED", line)

    def test_run_branch_summary_carries_the_drift_line(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)

            def fake_update_tree(_branch, tree, _build_log):
                catalog = tree / rn.CATALOG_RS
                catalog.parent.mkdir(parents=True)
                catalog.write_text(
                    'const REGISTRY: &[Entry] = &[\n    Entry::new("walk"),\n];\n',
                    encoding="utf-8")
                return "b" * 40, "b" * 9

            patches = [
                mock.patch.object(rn, "ROOT", root),
                mock.patch.object(rn, "CATALOG", root / "rs2b0t"),
                mock.patch.object(rn, "update_tree", fake_update_tree),
                mock.patch.object(rn, "log", lambda _message: None),
            ]
            for patch in patches:
                patch.start()
            try:
                run_dir = rn.run_branch("dev", 1, None, None, unavailable="test night off")
            finally:
                for patch in reversed(patches):
                    patch.stop()
            summary = json.loads((run_dir / "summary.json").read_text(encoding="utf-8"))
            # The fake tree has no abi-contract.json, so no fetch runs.
            self.assertEqual(summary["rs2b0t_api"]["status"], "unavailable")
            md = (run_dir / "summary.md").read_text(encoding="utf-8")
            self.assertIn("- rs2b0t API: UNAVAILABLE (", md)


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
    git("-c", "user.name=test", "-c", "user.email=test@example.invalid",
        "commit", "-qm", "init", cwd=repo)
    return repo


@unittest.skipUnless(shutil.which("git"), "git is not installed")
class BranchListTests(unittest.TestCase):
    def test_github_list_is_used_when_the_fetch_works(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            origin = make_branch_repo(root, "origin", branch="dev", files={
                "tools/nightly/branches.conf": "scratch-fix\ndev\n"})
            tip = git("rev-parse", "HEAD", cwd=origin)
            runner = root / "runner"
            subprocess.run(["git", "clone", "-q", str(origin), str(runner)], check=True)
            local = root / "branches.conf"
            local.write_text("stale-branch\n", encoding="utf-8")
            with mock.patch.dict(os.environ):
                os.environ.pop("NIGHTLY_BRANCHES", None)
                branches, info = rn.resolve_branch_list(None, runner, local)
            self.assertEqual(branches, ["scratch-fix", "dev"])
            self.assertEqual(info["source"], "github")
            self.assertEqual(info["commit"], tip)

    def test_local_list_and_a_summary_note_when_the_fetch_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            # No origin remote: the fetch must fail.
            runner = make_branch_repo(root, "runner", branch="dev")
            local = root / "branches.conf"
            local.write_text("# local list\nfix-line\ndev\n", encoding="utf-8")
            with mock.patch.dict(os.environ):
                os.environ.pop("NIGHTLY_BRANCHES", None)
                branches, info = rn.resolve_branch_list(None, runner, local)
            self.assertEqual(branches, ["fix-line", "dev"])
            self.assertEqual(info["source"], "local")
            self.assertIn("fetch", info["note"])

    def test_explicit_branch_wins_without_a_fetch(self):
        with tempfile.TemporaryDirectory() as tmp:
            runner = make_branch_repo(Path(tmp), "runner", branch="dev")
            with mock.patch.dict(os.environ):
                os.environ.pop("NIGHTLY_BRANCHES", None)
                with mock.patch.object(rn, "read_remote_branches",
                                       side_effect=AssertionError("must not fetch")):
                    branches, info = rn.resolve_branch_list(["dev"], runner)
            self.assertEqual(branches, ["dev"])
            self.assertEqual(info["source"], "cli")

    def test_env_branches_win_without_a_fetch(self):
        with tempfile.TemporaryDirectory() as tmp:
            runner = make_branch_repo(Path(tmp), "runner", branch="dev")
            with mock.patch.dict(os.environ, {"NIGHTLY_BRANCHES": "release/0.2.6 dev"}):
                with mock.patch.object(rn, "read_remote_branches",
                                       side_effect=AssertionError("must not fetch")):
                    branches, info = rn.resolve_branch_list(None, runner)
            self.assertEqual(branches, ["release/0.2.6", "dev"])
            self.assertEqual(info["source"], "env")


@unittest.skipUnless(shutil.which("git"), "git is not installed")
class SelfUpdateTests(unittest.TestCase):
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
                with tempfile.TemporaryDirectory() as tmp:
                    root = Path(tmp)
                    origin = make_branch_repo(root, "origin", branch="dev", files={
                        "tools/nightly/run_nightly.py": "# new\n"})
                    runner = root / "runner"
                    subprocess.run(["git", "clone", "-q", str(origin), str(runner)], check=True)
                    script = runner / "tools/nightly/run_nightly.py"
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
                        info = rn.maybe_self_update(runner, script, ["--branch", "dev"],
                                                    _exec=lambda *args: calls.append(args))
                    self.assertEqual(info["status"], "disabled")
                    self.assertEqual(script.read_text(encoding="utf-8"), before)
                    self.assertEqual(calls, [])

    def test_a_failed_self_check_keeps_the_old_code(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            origin = make_branch_repo(root, "origin", branch="dev", files={
                "tools/nightly/run_nightly.py": "# old runner\n"})
            runner = root / "runner"
            subprocess.run(["git", "clone", "-q", str(origin), str(runner)], check=True)
            old_head = git("rev-parse", "HEAD", cwd=runner)
            (origin / "tools/nightly/run_nightly.py").write_text("def broken(:\n", encoding="utf-8")
            git("add", ".", cwd=origin)
            git("-c", "user.name=test", "-c", "user.email=test@example.invalid",
                "commit", "-qm", "broken", cwd=origin)
            tip = git("rev-parse", "HEAD", cwd=origin)
            self.assertNotEqual(tip, old_head)
            script = runner / "tools/nightly/run_nightly.py"
            calls: list = []
            self.enable_update_env()
            info = rn.maybe_self_update(runner, script, ["--branch", "dev"],
                                        _exec=lambda *args: calls.append(args))
            self.assertEqual(info["status"], "check-failed")
            self.assertEqual(info["to"], tip)
            self.assertEqual(script.read_text(encoding="utf-8"), "# old runner\n")
            self.assertEqual(git("rev-parse", "HEAD", cwd=runner), old_head)
            self.assertEqual(calls, [])

    def test_a_passing_self_check_updates_and_reexecs(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            staged = root / "staged"
            shutil.copytree(HERE, staged / "tools/nightly",
                            ignore=shutil.ignore_patterns("__pycache__"))
            origin = root / "origin"
            origin.mkdir()
            git("init", "-q", cwd=origin)
            git("symbolic-ref", "HEAD", "refs/heads/dev", cwd=origin)
            shutil.copytree(staged / "tools", origin / "tools")
            git("add", ".", cwd=origin)
            git("-c", "user.name=test", "-c", "user.email=test@example.invalid",
                "commit", "-qm", "runner", cwd=origin)
            runner = root / "runner"
            subprocess.run(["git", "clone", "-q", str(origin), str(runner)], check=True)
            probe = "# nightly self-update probe\n"
            with (origin / "tools/nightly/run_nightly.py").open("a", encoding="utf-8") as handle:
                handle.write(probe)
            git("add", ".", cwd=origin)
            git("-c", "user.name=test", "-c", "user.email=test@example.invalid",
                "commit", "-qm", "probe", cwd=origin)
            tip = git("rev-parse", "HEAD", cwd=origin)
            script = runner / "tools/nightly/run_nightly.py"
            calls: list = []
            self.enable_update_env()
            info = rn.maybe_self_update(runner, script, ["--branch", "dev"],
                                        _exec=lambda *args: calls.append(args))
            self.assertIsNone(info)
            self.assertEqual(len(calls), 1)
            executable, argv, _env = calls[0]
            self.assertEqual(executable, sys.executable)
            self.assertEqual(argv[:2], [sys.executable, str(script)])
            self.assertEqual(argv[2:], ["--branch", "dev"])
            self.assertIn(probe, script.read_text(encoding="utf-8"))
            self.assertEqual(git("rev-parse", "HEAD", cwd=runner), tip)

    def test_self_check_timeout_kills_the_process_tree(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            candidate = root / "candidate"
            nightly = candidate / "tools" / "nightly"
            nightly.mkdir(parents=True)
            for name in ("run_nightly.py", "nightly-mac-guest.py", "latency_proxy.py"):
                (nightly / name).write_text("x = 1\n", encoding="utf-8")
            pid_file = root / "grandchild.pid"
            hang = nightly / "test_hang_grandchild.py"
            hang.write_text(
                "import os, subprocess, sys, time\n"
                "pid_file = os.environ.get('HANG_PID_FILE', '')\n"
                "grandchild = subprocess.Popen(\n"
                "    [sys.executable, '-c', 'import time; time.sleep(60)'],\n"
                ")\n"
                "if pid_file:\n"
                "    open(pid_file, 'w').write(str(grandchild.pid))\n"
                "time.sleep(60)\n",
                encoding="utf-8",
            )
            env_patch = mock.patch.dict(os.environ, {"HANG_PID_FILE": str(pid_file)})
            env_patch.start()
            self.addCleanup(env_patch.stop)
            started = time.monotonic()
            ok, detail = rn.self_check_candidate(
                candidate, test_file="test_hang_grandchild.py", timeout=2
            )
            elapsed = time.monotonic() - started
            self.assertFalse(ok)
            self.assertIn("did not finish within", detail)
            self.assertLess(
                elapsed, 30,
                f"the post-kill wait must stay bounded on every host: {elapsed:.1f}s",
            )
            self.assertTrue(pid_file.is_file(), "the hanging test must record its grandchild")
            grandchild = int(pid_file.read_text(encoding="utf-8").strip())
            time.sleep(0.5)
            try:
                with self.assertRaises(OSError):
                    os.kill(grandchild, 0)
            finally:
                try:
                    if os.name == "nt":
                        subprocess.run(
                            ["taskkill", "/F", "/PID", str(grandchild)],
                            stdout=subprocess.DEVNULL,
                            stderr=subprocess.DEVNULL,
                            check=False,
                        )
                    else:
                        os.kill(grandchild, 9)
                except OSError:
                    pass


class ReexecTests(unittest.TestCase):
    def test_windows_reexec_waits_for_the_child_exit_code(self):
        seen: list = []

        def fake_run(args, env):
            seen.append((list(args), dict(env)))
            return subprocess.CompletedProcess(args, 42)

        with mock.patch.object(os, "name", "nt"):
            with mock.patch.object(subprocess, "run", side_effect=fake_run):
                with self.assertRaises(SystemExit) as exited:
                    rn.reexec(sys.executable, [sys.executable, "run_nightly.py"], {"K": "V"})
        self.assertEqual(exited.exception.code, 42)
        self.assertEqual(seen, [([sys.executable, "run_nightly.py"], {"K": "V"})])

    def test_posix_reexec_replaces_the_process(self):
        with mock.patch.object(os, "name", "posix"):
            with mock.patch.object(os, "execvpe", create=True) as execvpe:
                with mock.patch.object(subprocess, "run",
                                       side_effect=AssertionError("must not spawn")):
                    rn.reexec(sys.executable, [sys.executable, "run_nightly.py"], {"K": "V"})
        execvpe.assert_called_once_with(sys.executable, [sys.executable, "run_nightly.py"],
                                        {"K": "V"})


class BranchListSummaryTests(unittest.TestCase):
    def test_run_branch_summary_records_branch_list_and_self_update(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)

            def fake_update_tree(_branch, tree, _build_log):
                catalog = tree / rn.CATALOG_RS
                catalog.parent.mkdir(parents=True)
                catalog.write_text(
                    'const REGISTRY: &[Entry] = &[\n    Entry::new("walk"),\n];\n',
                    encoding="utf-8")
                return "c" * 40, "c" * 9

            branch_list = {"source": "github", "commit": "a" * 40,
                           "note": "from origin/dev at " + "a" * 40}
            self_update = {"status": "check-failed", "from": "o" * 40, "to": "n" * 40,
                           "detail": "candidate origin/dev failed the self-check; running old code"}
            patches = [
                mock.patch.object(rn, "ROOT", root),
                mock.patch.object(rn, "STEPS", ["tui"]),
                mock.patch.object(rn, "FRESH_TREES", False),
                mock.patch.object(rn, "REQUIRE_CONSOLE", False),
                mock.patch.object(rn, "ENGINE_MODE", "remote"),
                mock.patch.object(rn, "update_tree", fake_update_tree),
                mock.patch.object(rn, "log", lambda _message: None),
            ]
            for patch in patches:
                patch.start()
            try:
                run_dir = rn.run_branch("dev", 1, None, None, unavailable="probe reason",
                                        summary_only=True, branch_list=branch_list,
                                        self_update=self_update)
            finally:
                for patch in patches:
                    patch.stop()
            summary = json.loads((run_dir / "summary.json").read_text(encoding="utf-8"))
            self.assertEqual(summary["branch_list"], branch_list)
            self.assertEqual(summary["self_update"], self_update)
            md = (run_dir / "summary.md").read_text(encoding="utf-8")
            self.assertIn("origin/dev", md)
            self.assertIn("Runner self-update", md)
            self.assertTrue((run_dir / "DONE").is_file())

if __name__ == "__main__":
    unittest.main()
