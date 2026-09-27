import argparse
import hashlib
import importlib.util
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


if __name__ == "__main__":
    unittest.main()
