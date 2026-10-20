#!/usr/bin/env python3
"""Tests for `run_nightly.py`: the disk guard, the stable tree path, receipts,
the revision defaults and the cells file.

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
import gc
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
import warnings
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
    def test_content_test_lanes_receive_nav_and_engine_paths(self):
        expected = {
            "NAV_PACK": str(rn.NAV_FIXTURE),
            "ENGINE_DIR": str(rn.ENGINE_RUN / "engine"),
        }
        lanes = {name: extra for name, _args, extra in rn.lane_list(None)}
        for name in ("test-workspace", "test-script-load", "test-script-no-default"):
            with self.subTest(lane=name):
                self.assertEqual(lanes[name], expected)


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


class RevisionDefaultsTests(unittest.TestCase):
    def test_289_defaults_are_the_values_the_script_always_used(self):
        linux = rn.revision_defaults("289", False)
        self.assertEqual(linux["profile"], "local-289")
        self.assertEqual(linux["unpack_dir"], "unpack-289")
        self.assertEqual(linux["run_suffix"], "")
        self.assertEqual(
            linux["engine_crc_sha256"],
            "687190c8ef08bb94a1422f4262d62bb7a76a55dfe0634ea43f0e63e0235dc623")
        self.assertEqual(rn.revision_defaults("289", True)["run_suffix"], "-windows")

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
            source = root / "runner" / "crates" / "nav" / "src" / "required-content-274.tsv"
            target = root / "tree" / "crates" / "nav" / "src" / "required-content-274.tsv"
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
            target = root / "tree" / "crates" / "nav" / "src" / "required-content-274.tsv"
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
            target = root / "tree" / "crates" / "nav" / "src" / "required-content-274.tsv"
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
        self.assertEqual(out.stdout.split(), ["289", "local-289", "unpack-289", "-", "687190c8"])

    @unittest.skipIf(os.name == "nt", "the 274 lane is a Linux guest")
    def test_revision_274_selects_the_lane(self):
        out = self.import_constants({"NIGHTLY_REVISION": "274"})
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(out.stdout.split(), ["274", "local-274", "unpack", "-274", "-"])

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


if __name__ == "__main__":
    unittest.main()
