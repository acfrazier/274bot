import argparse
import contextlib
import hashlib
import importlib.util
import io
import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest import mock
import zipfile
from pathlib import Path

TOOLS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOLS))
import release


def load_windows_transport():
    spec = importlib.util.spec_from_file_location("windows_ssh", TOOLS / "windows-ssh.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module

def load_packager():
    spec = importlib.util.spec_from_file_location("release_package", TOOLS / "package.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


windows_ssh = load_windows_transport()
release_package = load_packager()

def make_package_archive(root, platform, commit="a" * 40):
    expect = {"version": "0.1.9"}
    base = root / release.package_base(expect, platform)
    base.mkdir()
    suffix = ".exe" if platform == "windows" else ""
    for name in ("panel-play", "tui-play"):
        path = base / (name + suffix)
        path.write_bytes(name.encode())
        path.chmod(0o755)
    nav = base / "nav/289"
    nav.mkdir(parents=True)
    for name in release.NAV_NAMES:
        (nav / name).write_bytes(("same-" + name).encode())
    files = {
        path.relative_to(base).as_posix(): {
            "sha256": release.sha256_file(path), "bytes": path.stat().st_size,
        }
        for path in base.rglob("*") if path.is_file()
    }
    release.write_json(base / "release-manifest.json", {
        "platform": platform,
        "host_commit": commit,
        "client_commit": "b" * 40,
        "version": "0.1.9",
        "release": "Alpha 4",
        "status": "packaged",
        "revision": 289,
        "files": files,
    })
    archive = root / (base.name + release.ARCHIVE_SUFFIX[platform])
    if platform == "linux":
        with tarfile.open(archive, "w:gz") as output:
            output.add(base, arcname=base.name)
    else:
        with zipfile.ZipFile(archive, "w") as output:
            for path in base.rglob("*"):
                if path.is_file():
                    output.write(path, f"{base.name}/{path.relative_to(base).as_posix()}")
    shutil.rmtree(base)
    return archive


class DigestTests(unittest.TestCase):
    def test_tree_digest_is_stable_and_excludes_git_metadata(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "nested").mkdir()
            (root / "nested/b.txt").write_bytes(b"two")
            (root / "a.txt").write_bytes(b"one")
            (root / ".git").mkdir()
            (root / ".git/ignored").write_bytes(b"private")

            first = release.tree_digest(root)
            (root / ".git/ignored").write_bytes(b"changed")
            second = release.tree_digest(root)

            self.assertEqual(first, second)
            self.assertEqual(first["files"], 2)
            rows = {
                "a.txt": hashlib.sha256(b"one").hexdigest(),
                "nested/b.txt": hashlib.sha256(b"two").hexdigest(),
            }
            expected = hashlib.sha256()
            for name in sorted(rows):
                expected.update(name.encode() + b"\0" + rows[name].encode() + b"\n")
            self.assertEqual(first["sha256"], expected.hexdigest())

    def test_input_verification_rejects_changed_content(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            pack = root / "engine/data/pack/client"
            content = root / "content"
            snapshot = root / "snapshots/snapshot-id"
            for directory in (pack, content, snapshot):
                directory.mkdir(parents=True)
                (directory / "value").write_text(directory.name)
            expect = {
                "snapshot_version": "snapshot-id",
                "inputs": {
                    name: release.tree_digest(path)
                    for name, path in release.input_paths(root, {"snapshot_version": "snapshot-id"}).items()
                },
            }
            self.assertEqual(release.verify_input_digests(root, expect), expect["inputs"])
            (content / "value").write_text("different")
            with self.assertRaisesRegex(release.ReleaseError, "content"):
                release.verify_input_digests(root, expect)

    def test_inputs_archive_omits_git_metadata(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            engine = root / "engine"
            pack = engine / "data/pack/client"
            content = root / "content"
            snapshot_root = root / "snapshots"
            snapshot = snapshot_root / "snapshot-id"
            for directory in (pack, content, snapshot):
                directory.mkdir(parents=True)
                (directory / "value").write_text(directory.name)
            (content / ".git").write_text("gitdir: elsewhere")
            archive = root / "inputs.tar.gz"

            inputs, _record = release.create_inputs_archive(
                engine, content, snapshot_root, "snapshot-id", archive
            )

            with tarfile.open(archive) as source:
                names = {member.name for member in source.getmembers()}
            self.assertNotIn("content/.git", names)
            self.assertEqual(inputs["content"]["files"], 1)

    def test_source_export_replaces_gitlink_with_exact_client_tree(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            host = root / "host"
            client = root / "client"
            for repository in (host, client):
                repository.mkdir()
                subprocess.run(["git", "init", "-q", repository], check=True)
            (client / "client.txt").write_text("client source")
            subprocess.run(["git", "-C", client, "add", "client.txt"], check=True)
            subprocess.run([
                "git", "-C", client, "-c", "user.name=Test", "-c",
                "user.email=test@example.invalid", "commit", "-qm", "client",
            ], check=True)
            client_commit = subprocess.check_output(
                ["git", "-C", client, "rev-parse", "HEAD"], text=True
            ).strip()
            (host / "host.txt").write_text("host source")
            subprocess.run(["git", "-C", host, "add", "host.txt"], check=True)
            subprocess.run([
                "git", "-C", host, "update-index", "--add", "--cacheinfo",
                f"160000,{client_commit},vendor/fr-client-rust",
            ], check=True)
            subprocess.run([
                "git", "-C", host, "-c", "user.name=Test", "-c",
                "user.email=test@example.invalid", "commit", "-qm", "host",
            ], check=True)
            host_commit = subprocess.check_output(
                ["git", "-C", host, "rev-parse", "HEAD"], text=True
            ).strip()
            archive = root / "source.tar.gz"

            release.create_source_archive(host, client, host_commit, client_commit, archive)

            with tarfile.open(archive) as source:
                names = {member.name for member in source.getmembers()}
            self.assertIn("host.txt", names)
            self.assertIn("vendor/fr-client-rust/client.txt", names)
            release.require_clean_checkout(client, client_commit, "client")
            (client / "untracked").write_text("dirty")
            with self.assertRaisesRegex(release.ReleaseError, "dirty"):
                release.require_clean_checkout(client, client_commit, "client")

    def test_prepared_payload_is_reused_after_digest_verification(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            engine = root / "engine"
            content = root / "content"
            snapshots = root / "snapshots"
            for directory in (
                engine / "data/pack/client", content, snapshots / "snapshot-id"
            ):
                directory.mkdir(parents=True)
                (directory / "value").write_text(directory.name)
            commit = "a" * 40
            prepared = root / "work" / commit / "prepared"
            prepared.mkdir(parents=True)
            source = prepared / "source.tar.gz"
            inputs = prepared / "inputs.tar.gz"
            payload = prepared / "payload.tar.gz"
            source.write_bytes(b"source")
            inputs.write_bytes(b"inputs")
            payload.write_bytes(b"payload")
            args = argparse.Namespace(
                work_dir=root / "work", engine_dir=engine, content_dir=content,
                snapshot_root=snapshots, snapshot_version="snapshot-id",
                client_root=root / "client", revision="289", jobs=4,
            )
            expected_inputs = {
                "pack": release.tree_digest(engine / "data/pack/client"),
                "content": release.tree_digest(content),
                "snapshot": release.tree_digest(snapshots / "snapshot-id"),
            }
            release.write_json(prepared / "expect.json", {
                "schema": 1, "host_commit": commit, "client_commit": "b" * 40,
                "version": "0.1.9", "release": "Alpha 4", "revision": 289,
                "snapshot_version": "snapshot-id", "features": "default", "jobs": 4,
                "inputs": expected_inputs,
                "archives": {
                    "source.tar.gz": {
                        "sha256": release.sha256_file(source), "bytes": source.stat().st_size,
                    },
                    "inputs.tar.gz": {
                        "sha256": release.sha256_file(inputs), "bytes": inputs.stat().st_size,
                    },
                },
            })
            with mock.patch.object(release, "require_committed_tools"), \
                    mock.patch.object(release, "require_clean_checkout"), \
                    mock.patch.object(release, "release_name_at", return_value="Alpha 4"), \
                    mock.patch.object(release, "run_git", return_value="commit"):
                _commit_root, reused, _expect = release.prepare_payload(
                    args, root, commit, "b" * 40, "0.1.9"
                )
            self.assertEqual(reused, payload.resolve())


class ParameterTests(unittest.TestCase):
    def test_all_platform_plan_contains_pinned_gates_and_transports(self):
        arguments = argparse.Namespace(
            platform="all",
            revision="289",
            linux_host="274bot-builder",
            windows_host=None,
        )
        plan = release.render_build_plan(
            arguments,
            "a" * 40,
            "b" * 40,
            "0.1.9",
        )
        encoded = json.dumps(plan)
        self.assertEqual([row["platform"] for row in plan["platforms"]],
                         ["macos", "linux", "windows"])
        self.assertIn("BOT_NAV_BUILD=require", encoded)
        self.assertIn("GIT_DIRTY=0", encoded)
        self.assertIn("-p panel --bin panel-play -p tui --bin tui-play", encoded)
        self.assertIn("windows-ssh.py", encoded)
        self.assertIn("${RELEASE_WINDOWS_HOST}", encoded)
        self.assertIn("${SNAPSHOT_ROOT}/${SNAPSHOT_VERSION}", encoded)

    def test_remote_roots_must_be_relative_and_cannot_escape(self):
        self.assertEqual(release.validate_remote_root("release/cache", "root"), "release/cache")
        for value in ("/tmp/release", "../release", "release path", "release\\path"):
            with self.subTest(value=value), self.assertRaises(release.ReleaseError):
                release.validate_remote_root(value, "root")

    def test_packager_has_no_operator_filesystem_defaults(self):
        with mock.patch.dict(os.environ, {}, clear=True):
            self.assertEqual(release_package.nav_build_inputs("289"), (None, None))

    def test_windows_transport_has_no_embedded_host_or_key_path(self):
        options = windows_ssh.ssh_options("key-file", "known-hosts", 8)
        self.assertIn("StrictHostKeyChecking=yes", options)
        self.assertIn("UserKnownHostsFile=known-hosts", options)
        self.assertEqual(options[-3:], ["UserKnownHostsFile=known-hosts", "-i", "key-file"])
        decoded = __import__("base64").b64decode(
            windows_ssh.encoded_powershell("python -V")
        ).decode("utf-16-le")
        self.assertIn("$LASTEXITCODE", decoded)
        self.assertIn("$ErrorActionPreference='Continue'", decoded)
        self.assertIn("$PSDefaultParameterValues", decoded)
        self.assertNotIn("$ErrorActionPreference='Stop'", decoded)

    def test_windows_only_environment_does_not_leak_to_macos(self):
        expect = {
            "host_commit": "a" * 40,
            "client_commit": "b" * 40,
            "revision": 289,
            "jobs": 4,
        }
        with mock.patch.dict(os.environ, {
            "RUSTY_V8_ARCHIVE": "windows.lib.gz",
            "AWS_LC_SYS_PREBUILT_NASM": "1",
        }):
            mac = release.native_build_environment(
                expect, Path("/inputs"), Path("/target"), "macos"
            )
            windows = release.native_build_environment(
                expect, Path("/inputs"), Path("/target"), "windows", "windows.lib.gz"
            )
        self.assertNotIn("RUSTY_V8_ARCHIVE", mac)
        self.assertNotIn("AWS_LC_SYS_PREBUILT_NASM", mac)
        self.assertEqual(windows["RUSTY_V8_ARCHIVE"], "windows.lib.gz")

    def test_tag_release_rustc_and_powershell_values_are_validated(self):
        release.validate_tag("0.1.9", "0.1.9")
        release.validate_tag("0.1.8.1", "0.1.8")
        for tag in ("0.1.8-beta", "0.1.9.1", "0.1.8\u2019;Write-Output nope"):
            with self.subTest(tag=tag), self.assertRaises(release.ReleaseError):
                release.validate_tag(tag, "0.1.8")
        with self.assertRaises(release.ReleaseError):
            release.validate_release_identity(
                {"version": "0.1.9", "release": "Alpha 4"},
                {"version": "0.1.9", "release": "Alpha 3"},
                "0.1.9",
            )
        release.validate_rustc_host("rustc 1.0\nhost: aarch64-apple-darwin\n", "macos")
        with self.assertRaisesRegex(release.ReleaseError, "rustc host"):
            release.validate_rustc_host("host: x86_64-apple-darwin\n", "macos")
        with self.assertRaises(release.ReleaseError):
            release.powershell_quote("safe\u2019; Write-Output nope")


class ManifestTests(unittest.TestCase):
    def test_finalize_manifest_rehashes_notes_and_package_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            package = root / "package"
            package.mkdir()
            (package / "panel-play").write_bytes(b"panel")
            (package / "release-manifest.json").write_text(json.dumps({
                "platform": "linux",
                "host_commit": "a" * 40,
                "client_commit": "b" * 40,
                "files": {},
            }))
            notes = root / "notes.md"
            notes.write_text("release notes\n")

            manifest = release.finalize_manifest(package, "linux", "0.1.9", notes)

            self.assertEqual(manifest["tag"], "0.1.9")
            self.assertEqual(manifest["status"], "packaged")
            self.assertFalse(manifest["notarized"])
            self.assertEqual(set(manifest["files"]), {"panel-play", "RELEASE-NOTES.md"})
            for name, record in manifest["files"].items():
                self.assertEqual(record["sha256"], release.sha256_file(package / name))
                self.assertEqual(record["bytes"], (package / name).stat().st_size)

    def test_archive_verifier_checks_exact_manifest_membership(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            package = root / "274bot-0.1.9-linux-x64"
            package.mkdir()
            payload = package / "payload.txt"
            payload.write_text("bound bytes")
            manifest = {
                "platform": "linux",
                "host_commit": "a" * 40,
                "client_commit": "b" * 40,
                "status": "packaged",
                "revision": 289,
                "files": {
                    "payload.txt": {
                        "sha256": release.sha256_file(payload),
                        "bytes": payload.stat().st_size,
                    }
                },
            }
            (package / "release-manifest.json").write_text(json.dumps(manifest))
            archive = root / "package.tar.gz"
            with tarfile.open(archive, "w:gz") as output:
                output.add(package, arcname=package.name)

            extracted = root / "extracted"
            _base, verified, help_results = release.verify_package_archive(
                archive,
                extracted,
                expected_platform="linux",
                expected_commit="a" * 40,
            )
            self.assertEqual(verified["client_commit"], "b" * 40)
            self.assertEqual(help_results, {})

            shutil.rmtree(extracted)
            (package / "extra.txt").write_text("not listed")
            bad_archive = root / "bad.zip"
            with zipfile.ZipFile(bad_archive, "w") as output:
                for path in package.rglob("*"):
                    if path.is_file():
                        output.write(path, f"{package.name}/{path.relative_to(package).as_posix()}")
            with self.assertRaisesRegex(release.ReleaseError, "membership"):
                release.verify_package_archive(bad_archive, extracted)

    def test_archive_rejects_top_level_extra_duplicate_and_symlink(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            package = root / "package"
            package.mkdir()
            (package / "release-manifest.json").write_text(json.dumps({
                "platform": "linux", "host_commit": "a" * 40,
                "client_commit": "b" * 40, "files": {},
            }))
            extra = root / "extra.zip"
            with zipfile.ZipFile(extra, "w") as output:
                output.write(package / "release-manifest.json",
                             "package/release-manifest.json")
                output.writestr("EXTRA.txt", "extra")
            with self.assertRaisesRegex(release.ReleaseError, "exactly one"):
                release.verify_package_archive(extra, root / "extra-out")

            duplicate = root / "duplicate.tar"
            info = tarfile.TarInfo("same")
            info.size = 1
            with tarfile.open(duplicate, "w") as output:
                output.addfile(info, io.BytesIO(b"a"))
                output.addfile(info, io.BytesIO(b"b"))
            with self.assertRaisesRegex(release.ReleaseError, "duplicate"):
                release.safe_tar_members(duplicate)

            linked = root / "linked.tar"
            info = tarfile.TarInfo("link")
            info.type = tarfile.SYMTYPE
            info.linkname = "target"
            with tarfile.open(linked, "w") as output:
                output.addfile(info)
            with self.assertRaisesRegex(release.ReleaseError, "unsupported"):
                release.safe_tar_members(linked)

    def test_manifest_refuses_finder_metadata(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            package = root / "package"
            package.mkdir()
            (package / ".DS_Store").write_bytes(b"local")
            (package / "release-manifest.json").write_text(json.dumps({
                "platform": "linux", "files": {},
            }))
            notes = root / "notes"
            notes.write_text("notes")
            with self.assertRaisesRegex(release.ReleaseError, "forbidden metadata"):
                release.finalize_manifest(package, "linux", "0.1.9", notes)

    def test_checksums_reject_stale_archives_and_tampering(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            expect = {"version": "0.1.9"}
            name = "274bot-0.1.9-linux-x64.tar.gz"
            candidate = root / name
            candidate.write_bytes(b"candidate")
            stale = root / "274bot-0.1.7-linux-arm64.tar.gz"
            stale.write_bytes(b"stale")
            with self.assertRaisesRegex(release.ReleaseError, "unexpected"):
                release.update_sha256s(root, expect)
            stale.unlink()
            release.update_sha256s(root, expect)
            release.verify_sha256s(root, {"linux": candidate})
            candidate.write_bytes(b"tampered")
            with self.assertRaisesRegex(release.ReleaseError, "mismatch"):
                release.verify_sha256s(root, {"linux": candidate})

    def test_nav_comparison_requires_all_three_platforms(self):
        nav = {"274bot.navpack": {"sha256": "a", "bytes": 1}}
        self.assertIn("not compared", release.compare_nav_sets({"linux": nav}))
        self.assertIn("byte-identical", release.compare_nav_sets({
            "macos": nav, "linux": nav, "windows": nav,
        }))
        with self.assertRaisesRegex(release.ReleaseError, "navigation differs"):
            release.compare_nav_sets({
                "macos": nav,
                "linux": nav,
                "windows": {"274bot.navpack": {"sha256": "b", "bytes": 1}},
            })

    def test_retain_cleanup_keeps_only_packages_and_receipts(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            expect = {
                "schema": 1, "host_commit": "a" * 40, "client_commit": "b" * 40,
                "version": "0.1.9", "release": "Alpha 4",
            }
            release.write_json(root / "expect.json", expect)
            package = root / "274bot-0.1.9-linux-x64"
            package.mkdir()
            (root / "linux-build-receipt.json").write_text("{}")
            (root / "source.tar.gz").write_bytes(b"large")
            (root / "inputs").mkdir()
            retained = release.retain_release_outputs(root, "a" * 40, "linux")
            self.assertEqual(
                retained,
                ["274bot-0.1.9-linux-x64", "linux-build-receipt.json"],
            )

    def test_macos_finalize_reuses_accepted_notarization_on_retry(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            commit = "a" * 40
            release.write_json(root / "expect.json", {
                "schema": 1, "host_commit": commit, "client_commit": "b" * 40,
                "version": "0.1.9", "release": "Alpha 4",
            })
            package = root / "274bot-0.1.9-macos-arm64"
            (package / "274bot.app").mkdir(parents=True)
            release.write_json(package / "release-manifest.json", {
                "platform": "macos", "version": "0.1.9", "release": "Alpha 4",
                "signed": ["panel-play", "tui-play", "274bot.app"], "files": {},
            })
            notes = root / "notes"
            notes.write_text("notes")
            args = argparse.Namespace(
                worker_root=root, commit=commit, platform="macos",
                release_notes=notes, tag="0.1.9", notarization_id=None,
                notary_profile="274bot",
            )
            commands = []

            def fake_run(command, **_kwargs):
                commands.append([str(value) for value in command])
                if "notarytool" in command:
                    return json.dumps({"status": "Accepted", "id": "accepted-id"})
                return ""

            def fake_archive(_base, _platform, destination):
                Path(destination).write_bytes(b"archive")
                return Path(destination)

            with mock.patch.object(release, "run", side_effect=fake_run), \
                    mock.patch.object(release, "create_archive", side_effect=fake_archive), \
                    contextlib.redirect_stdout(io.StringIO()):
                release.finalize_worker(args)
                release.finalize_worker(args)
            submissions = [row for row in commands if "notarytool" in row]
            self.assertEqual(len(submissions), 1)
            self.assertEqual(
                (root / "notarization-id.txt").read_text().strip(), "accepted-id"
            )


class ControllerTests(unittest.TestCase):
    def transport_args(self, root, platform="all"):
        return argparse.Namespace(
            commit="a" * 40,
            platform=platform,
            dry_run=False,
            work_dir=root / "work",
            artifact_dir=root / "artifacts",
            release_notes=root / "notes.md",
            tag="0.1.9",
            notary_profile="274bot",
            notarization_id=None,
            worker_root=None,
            linux_host="builder",
            linux_remote_root="release",
            linux_identity=None,
            linux_known_hosts=None,
            windows_host="operator@windows",
            windows_remote_root="release",
            windows_identity=None,
            windows_known_hosts=None,
        )

    def test_finalize_all_sets_local_worker_platform_and_writes_only_three_sums(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            args = self.transport_args(root)
            args.release_notes.write_text("notes")
            prepared = args.work_dir / args.commit / "prepared"
            prepared.mkdir(parents=True)
            expect = {
                "schema": 1, "host_commit": args.commit, "client_commit": "b" * 40,
                "version": "0.1.9", "release": "Alpha 4",
            }
            release.write_json(prepared / "expect.json", expect)
            mac_archive = root / "mac.zip"
            mac_archive.write_bytes(b"mac")
            seen = []

            def fake_finalize(worker):
                seen.append(worker.platform)
                return mac_archive

            def fake_linux_get(_args, _remote, local):
                Path(local).write_bytes(b"linux")

            def fake_windows(_args, action, *values):
                if action == "get":
                    Path(values[1]).write_bytes(b"windows")

            with mock.patch.object(release, "finalize_worker", side_effect=fake_finalize), \
                    mock.patch.object(release, "linux_put"), \
                    mock.patch.object(release, "linux_run"), \
                    mock.patch.object(release, "linux_get", side_effect=fake_linux_get), \
                    mock.patch.object(release, "windows_transport", side_effect=fake_windows), \
                    contextlib.redirect_stdout(io.StringIO()):
                release.finalize_controller(args)

            self.assertEqual(seen, ["macos"])
            rows = (args.artifact_dir / "SHA256SUMS").read_text().splitlines()
            self.assertEqual(len(rows), 3)

    def test_verify_all_sets_local_worker_platform_and_compares_three_nav_sets(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            artifacts = root / "artifacts"
            artifacts.mkdir()
            for platform in release.PLATFORMS:
                make_package_archive(artifacts, platform)
            release.update_sha256s(artifacts, {"version": "0.1.9"})
            args = self.transport_args(root)
            args.artifact_dir = artifacts
            seen = []

            def fake_verify(worker):
                seen.append(worker.platform)

            output = io.StringIO()
            with mock.patch.object(release, "verify_worker", side_effect=fake_verify), \
                    mock.patch.object(release, "linux_run"), \
                    mock.patch.object(release, "linux_put"), \
                    mock.patch.object(release, "windows_transport"), \
                    contextlib.redirect_stdout(output):
                release.verify_controller(args)

            self.assertEqual(seen, ["macos"])
            self.assertIn(
                "byte-identical across macos, linux, and windows", output.getvalue()
            )

if __name__ == "__main__":
    unittest.main()
