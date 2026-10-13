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


class MacosBuildEnvironmentTests(unittest.TestCase):
    EXPECT = {
        "host_commit": "a" * 40,
        "client_commit": "b" * 40,
        "revision": 289,
        "jobs": 4,
    }
    HOSTILE = {
        "PATH": "/opt/local/bin:/opt/homebrew/bin:/usr/bin:/bin",
        "LIBRARY_PATH": "/opt/local/lib",
        "CPATH": "/opt/local/include",
        "C_INCLUDE_PATH": "/opt/local/include",
        "PKG_CONFIG": "/opt/local/bin/pkg-config",
        "PKG_CONFIG_PATH": "/opt/local/lib/pkgconfig",
        "PKG_CONFIG_PATH_aarch64-apple-darwin": "/opt/local/lib/pkgconfig",
        "DYLD_LIBRARY_PATH": "/opt/local/lib",
        "DYLD_INSERT_LIBRARIES": "/opt/local/lib/libfoo.dylib",
        "CC": "/opt/local/bin/gcc",
        "TARGET_CC": "/opt/local/bin/gcc",
        "CC_aarch64-apple-darwin": "/opt/local/bin/gcc",
        "CFLAGS": "-I/opt/local/include",
        "LDFLAGS": "-L/opt/local/lib",
        "CMAKE_PREFIX_PATH": "/opt/homebrew",
        "OPENSSL_DIR": "/opt/homebrew/opt/openssl@3",
        "HOMEBREW_PREFIX": "/opt/homebrew",
        "BINDGEN_EXTRA_CLANG_ARGS": "-I/opt/local/include",
        "LIBCLANG_PATH": "/opt/local/lib/libclang.dylib",
        "SDKROOT": "/opt/local/sdk",
        "MACOSX_DEPLOYMENT_TARGET": "14.0",
        "RUSTFLAGS": "-L /opt/local/lib",
        "CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER": "/opt/local/bin/ld",
        "RUSTC_WRAPPER": "/opt/homebrew/bin/sccache",
        # Toolchain/config redirection and arbitrary future variables a
        # denylist would miss; an allowlist drops them by construction.
        "CARGO_HOME": "/opt/local/cargo",
        "RUSTUP_HOME": "/opt/local/rustup",
        "RUSTUP_TOOLCHAIN": "nightly-2099-01-01",
        "CARGO": "/opt/local/bin/cargo-wrapper",
        "CCC_OVERRIDE_OPTIONS": "x-I/opt/local/include",
        "COMPILER_PATH": "/opt/local/bin",
        "OBJCPLUS_INCLUDE_PATH": "/opt/local/include",
        "SOME_FUTURE_LIB_DIR": "/opt/local/lib",
        "EVIL_ARBITRARY_VARIABLE": "/opt/local/lib/libevil.dylib",
        "SHELL": "/opt/local/bin/zsh",
        "SSH_AUTH_SOCK": "/tmp/agent.sock",
        "FPATH": "/opt/homebrew/share/zsh/site-functions",
    }
    # Keys native_build_environment itself pins for every platform.
    WORKER_KEYS = {
        "GIT_COMMIT", "CLIENT_COMMIT", "GIT_DIRTY", "BOT_NAV_REVISION",
        "BOT_NAV_BUILD", "BOT_NAV_ENGINE_DIR", "BOT_NAV_CONTENT_DIR",
        "BOT_NAV_SNAPSHOT_ROOT", "CARGO_TARGET_DIR", "CARGO_BUILD_JOBS",
        "CARGO_INCREMENTAL",
    }

    def test_macos_build_path_is_fixed_and_minimal(self):
        self.assertEqual(
            release.macos_build_path("/Users/builder"),
            "/Users/builder/.cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin",
        )

    def test_macos_environment_is_an_allowlist_from_empty(self):
        parent = {
            **self.HOSTILE,
            "HOME": "/Users/builder",
            "USER": "builder",
            "LOGNAME": "builder",
            "TMPDIR": "/var/folders/builder/T/",
            "LANG": "C.UTF-8",
            "LC_ALL": "C.UTF-8",
            "LC_CTYPE": "UTF-8",
            "HTTP_PROXY": "http://proxy.example:3128",
            "https_proxy": "http://proxy.example:3128",
            "DEVELOPER_DIR": "/Library/Developer/CommandLineTools",
        }
        is_dir = lambda path: str(path) == "/Library/Developer/CommandLineTools"
        with mock.patch.dict(os.environ, parent, clear=True), \
                mock.patch.object(Path, "is_dir", is_dir):
            mac = release.native_build_environment(
                self.EXPECT, Path("/inputs"), Path("/target"), "macos"
            )
        # Only the allowlisted keys, the constructed keys, and the keys
        # the worker itself pins survive -- nothing else is inherited.
        self.assertEqual(set(mac), self.WORKER_KEYS | {
            "PATH", "HOME", "MACOSX_DEPLOYMENT_TARGET",
            "USER", "LOGNAME", "TMPDIR", "LANG", "LC_ALL", "LC_CTYPE",
            "HTTP_PROXY", "https_proxy", "DEVELOPER_DIR",
        })
        self.assertEqual(mac["PATH"], release.macos_build_path("/Users/builder"))
        self.assertEqual(mac["HOME"], "/Users/builder")
        # The deployment target is pinned from the one constant the
        # manifest line also uses, not inherited from the shell's 14.0.
        self.assertEqual(mac["MACOSX_DEPLOYMENT_TARGET"], "11.0")
        self.assertEqual(
            mac["MACOSX_DEPLOYMENT_TARGET"], release.MACOS_DEPLOYMENT_TARGET
        )
        self.assertEqual(
            release.runtime_requirements("macos"),
            [f"macOS {release.MACOS_DEPLOYMENT_TARGET} or later; Apple Silicon"],
        )
        for name in ("CARGO_HOME", "RUSTUP_HOME", "RUSTUP_TOOLCHAIN",
                     "EVIL_ARBITRARY_VARIABLE", "SOME_FUTURE_LIB_DIR"):
            self.assertNotIn(name, mac)
        self.assertEqual(mac["DEVELOPER_DIR"], "/Library/Developer/CommandLineTools")
        self.assertEqual(mac["CARGO_TARGET_DIR"], "/target")
        self.assertEqual(mac["GIT_DIRTY"], "0")

    def test_developer_dir_must_be_an_existing_apple_dir(self):
        with mock.patch.object(Path, "is_dir", lambda path: True):
            self.assertTrue(release.valid_macos_developer_dir(
                "/Library/Developer/CommandLineTools"))
            self.assertTrue(release.valid_macos_developer_dir(
                "/Applications/Xcode.app/Contents/Developer"))
            # Existing is not enough: a non-Apple location is dropped.
            self.assertFalse(release.valid_macos_developer_dir(
                "/opt/local/developer"))
            self.assertFalse(release.valid_macos_developer_dir(
                "/Library/Developer/CommandLineTools/../elsewhere"))
        with mock.patch.object(Path, "is_dir", lambda path: False):
            # The right shape in a place that does not exist is dropped.
            self.assertFalse(release.valid_macos_developer_dir(
                "/Applications/Xcode.app/Contents/Developer"))
        parent = {
            "HOME": "/Users/builder",
            "DEVELOPER_DIR": "/opt/local/developer",
        }
        with mock.patch.dict(os.environ, parent, clear=True), \
                mock.patch.object(Path, "is_dir", lambda path: True):
            mac = release.native_build_environment(
                self.EXPECT, Path("/inputs"), Path("/target"), "macos"
            )
        self.assertNotIn("DEVELOPER_DIR", mac)

    def test_other_platforms_keep_the_inherited_environment(self):
        with mock.patch.dict(os.environ, {**self.HOSTILE, "HOME": "/home/builder"}):
            linux = release.native_build_environment(
                self.EXPECT, Path("/inputs"), Path("/target"), "linux"
            )
        self.assertEqual(
            linux["PATH"],
            "/home/builder/.cargo/bin:" + self.HOSTILE["PATH"],
        )
        self.assertEqual(linux["LIBRARY_PATH"], "/opt/local/lib")
        # The allowlist and deployment pin are macOS-only.
        self.assertEqual(linux["RUSTUP_TOOLCHAIN"], "nightly-2099-01-01")
        self.assertEqual(linux["MACOSX_DEPLOYMENT_TARGET"], "14.0")

OTOOL_L = """\
/pkg/panel-play:
\t/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit (compatibility version 45.0.0, current version 2775.10.103)
\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1359.0.0)
\t/usr/lib/libiconv.2.dylib (compatibility version 7.0.0, current version 7.0.0)
\t@rpath/libfoo.dylib (compatibility version 1.0.0, current version 1.0.0)
\t/opt/local/lib/libbz2.1.0.dylib (compatibility version 1.0.0, current version 1.0.8)
"""

OTOOL_L_FAT = """\
/pkg/panel-play (architecture arm64):
\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1359.0.0)
/pkg/panel-play (architecture x86_64):
\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1359.0.0)
"""

OTOOL_LOAD_COMMANDS = """\
/pkg/panel-play:
Load command 0
      cmd LC_SEGMENT_64
  cmdsize 72
Load command 9
      cmd LC_BUILD_VERSION
  cmdsize 32
 platform 1
    minos 11.0
      sdk 27.0
   ntools 1
Load command 12
      cmd LC_RPATH
  cmdsize 40
     path @executable_path/../Frameworks (offset 12)
Load command 13
      cmd LC_RPATH
  cmdsize 32
     path /opt/local/lib (offset 12)
Load command 14
      cmd LC_LOAD_DYLIB
  cmdsize 56
     name /usr/lib/libSystem.B.dylib (offset 24)
"""

OTOOL_LOAD_COMMANDS_LEGACY_AND_FAT = """\
/pkg/old (architecture arm64):
Load command 8
      cmd LC_VERSION_MIN_MACOSX
  cmdsize 16
  version 10.15
      sdk 10.15
/pkg/old (architecture x86_64):
Load command 8
      cmd LC_BUILD_VERSION
  cmdsize 32
 platform 1
    minos 14.0
      sdk 14.0
"""


class MacosLinkageTests(unittest.TestCase):
    def make_package(self, root):
        base = root / "274bot-0.2.1-macos-arm64"
        (base / "274bot.app/Contents/Frameworks").mkdir(parents=True)
        (base / "274bot.app/Contents/MacOS").mkdir(parents=True)
        (base / "Frameworks").mkdir()
        for relative in release.MACOS_REQUIRED_EXECUTABLES:
            (base / relative).write_bytes(b"\xcf\xfa\xed\xfe" + b"\0" * 28)
        (base / "Frameworks/libfoo.dylib").write_bytes(b"\xcf\xfa\xed\xfe")
        (base / "274bot.app/Contents/Frameworks/libfoo.dylib").write_bytes(
            b"\xcf\xfa\xed\xfe"
        )
        (base / "notes.txt").write_text("not a binary")
        return base

    def test_otool_parsers_read_libraries_rpaths_and_minos(self):
        self.assertEqual(release.parse_otool_libraries(OTOOL_L), [
            "/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit",
            "/usr/lib/libSystem.B.dylib",
            "/usr/lib/libiconv.2.dylib",
            "@rpath/libfoo.dylib",
            "/opt/local/lib/libbz2.1.0.dylib",
        ])
        self.assertEqual(release.parse_otool_libraries(OTOOL_L_FAT), [
            "/usr/lib/libSystem.B.dylib",
            "/usr/lib/libSystem.B.dylib",
        ])
        self.assertEqual(release.parse_otool_rpaths(OTOOL_LOAD_COMMANDS), [
            "@executable_path/../Frameworks",
            "/opt/local/lib",
        ])
        self.assertEqual(release.parse_otool_minimum_os(OTOOL_LOAD_COMMANDS), ["11.0"])
        # Legacy LC_VERSION_MIN_MACOSX, and every slice of a fat binary.
        self.assertEqual(
            release.parse_otool_minimum_os(OTOOL_LOAD_COMMANDS_LEGACY_AND_FAT),
            ["10.15", "14.0"],
        )

    def test_system_libraries_are_allowed(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = self.make_package(Path(temporary))
            violations = release.macho_linkage_violations(
                base / "panel-play", base,
                ["/usr/lib/libSystem.B.dylib",
                 "/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit"],
                [],
            )
        self.assertEqual(violations, [])

    def test_package_manager_and_other_libraries_are_named(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = self.make_package(Path(temporary))
            violations = release.macho_linkage_violations(
                base / "panel-play", base,
                ["/opt/local/lib/libbz2.1.0.dylib",
                 "/opt/homebrew/lib/libssl.3.dylib",
                 "/usr/local/lib/libfoo.dylib",
                 # Normalises to /opt/local/...: a raw prefix test on
                 # /usr/lib/ would wave this through as a system library.
                 "/usr/lib/../opt/local/lib/libbz2.1.0.dylib",
                 "/System/Library/../../opt/homebrew/lib/libssl.3.dylib",
                 "libfoo.dylib",
                 "@loader_path/libfoo.dylib"],
                [],
            )
        self.assertEqual(violations, [
            "panel-play: /opt/local/lib/libbz2.1.0.dylib",
            "panel-play: /opt/homebrew/lib/libssl.3.dylib",
            "panel-play: /usr/local/lib/libfoo.dylib",
            "panel-play: /usr/lib/../opt/local/lib/libbz2.1.0.dylib",
            "panel-play: /System/Library/../../opt/homebrew/lib/libssl.3.dylib",
            "panel-play: libfoo.dylib",
            "panel-play: @loader_path/libfoo.dylib",
        ])

    def test_executable_path_expands_against_every_main_executable(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = self.make_package(Path(temporary))
            app_exe = base / "274bot.app/Contents/MacOS/panel-play"
            # @executable_path/../Frameworks resolves from the app
            # bundle executable, but dyld expands @executable_path to
            # the main executable's directory: from the package-root
            # executables the same string escapes the package.
            app_only = release.macho_linkage_violations(
                app_exe, base,
                ["@executable_path/../Frameworks/libfoo.dylib"], [],
            )
            # A reference that resolves from every main executable's
            # directory (root and app bundle alike) is allowed.
            (base / "libevery.dylib").write_bytes(b"lib")
            (base / "274bot.app/Contents/MacOS/libevery.dylib").write_bytes(b"lib")
            everywhere = release.macho_linkage_violations(
                app_exe, base, ["@executable_path/libevery.dylib"], [],
            )
            missing = release.macho_linkage_violations(
                app_exe, base,
                ["@executable_path/../Frameworks/libabsent.dylib"], [],
            )
            escaping = release.macho_linkage_violations(
                base / "panel-play", base,
                ["@executable_path/../../../opt/local/lib/libbz2.dylib"], [],
            )
        self.assertEqual(app_only, [
            "274bot.app/Contents/MacOS/panel-play: "
            "@executable_path/../Frameworks/libfoo.dylib"
        ])
        self.assertEqual(everywhere, [])
        self.assertEqual(len(missing), 1)
        self.assertEqual(len(escaping), 1)

    def test_rpath_must_resolve_inside_the_package(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = self.make_package(Path(temporary))
            binary = base / "panel-play"
            # @loader_path is the referencing binary's own directory, so
            # this is the correct token for "next to this image".
            loader_resolved = release.macho_linkage_violations(
                binary, base, ["@rpath/libfoo.dylib"],
                ["@loader_path/Frameworks"],
            )
            # @executable_path/Frameworks resolves from the root
            # executables but not from 274bot.app/Contents/MacOS, so it
            # must not pass on the strength of the root expansion.
            executable_root_only = release.macho_linkage_violations(
                binary, base, ["@rpath/libfoo.dylib"],
                ["@executable_path/Frameworks"],
            )
            macos_frameworks = base / "274bot.app/Contents/MacOS/Frameworks"
            macos_frameworks.mkdir()
            (macos_frameworks / "libfoo.dylib").write_bytes(b"lib")
            executable_everywhere = release.macho_linkage_violations(
                binary, base, ["@rpath/libfoo.dylib"],
                ["@executable_path/Frameworks"],
            )
            outside = release.macho_linkage_violations(
                binary, base, ["@rpath/libfoo.dylib"], ["/opt/local/lib"],
            )
            no_rpath = release.macho_linkage_violations(
                binary, base, ["@rpath/libfoo.dylib"], [],
            )
            relative = release.macho_linkage_violations(
                binary, base, ["@rpath/libfoo.dylib"], ["Frameworks"],
            )
        self.assertEqual(loader_resolved, [])
        self.assertEqual(executable_root_only, ["panel-play: @rpath/libfoo.dylib"])
        self.assertEqual(executable_everywhere, [])
        self.assertEqual(outside, ["panel-play: @rpath/libfoo.dylib"])
        self.assertEqual(no_rpath, ["panel-play: @rpath/libfoo.dylib"])
        self.assertEqual(relative, ["panel-play: @rpath/libfoo.dylib"])

    def test_nested_dylib_executable_path_escape_is_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = self.make_package(Path(temporary))
            plugin_dir = base / "nested/deep"
            plugin_dir.mkdir(parents=True)
            plugin = plugin_dir / "plugin.dylib"
            plugin.write_bytes(b"\xcf\xfa\xed\xfe" + b"\0" * 28)
            # Planted where the plugin's own directory would take
            # @executable_path/../../opt/local/lib -- the expansion the
            # old check used. From a real main executable (package root)
            # that string leaves the package entirely.
            planted = base / "opt/local/lib"
            planted.mkdir(parents=True)
            (planted / "libbz2.1.0.dylib").write_bytes(b"lib")
            via_rpath = release.macho_linkage_violations(
                plugin, base, ["@rpath/libbz2.1.0.dylib"],
                ["@executable_path/../../opt/local/lib"],
            )
            direct = release.macho_linkage_violations(
                plugin, base,
                ["@executable_path/../../opt/local/lib/libbz2.1.0.dylib"], [],
            )
            # The same plugin reached through @loader_path -- its own
            # directory, whatever main executable loaded it -- passes.
            (plugin_dir / "libnear.dylib").write_bytes(b"lib")
            via_loader = release.macho_linkage_violations(
                plugin, base, ["@rpath/libnear.dylib"], ["@loader_path"],
            )
        self.assertEqual(via_rpath, [
            "nested/deep/plugin.dylib: @rpath/libbz2.1.0.dylib"
        ])
        self.assertEqual(direct, [
            "nested/deep/plugin.dylib: "
            "@executable_path/../../opt/local/lib/libbz2.1.0.dylib"
        ])
        self.assertEqual(via_loader, [])

    def test_macho_filetypes_and_main_executable_dirs(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = self.make_package(Path(temporary))
            extra = base / "extra"
            extra.mkdir()
            tool = extra / "tool"
            # Thin 64-bit little-endian header, filetype MH_EXECUTE (2).
            tool.write_bytes(
                b"\xcf\xfa\xed\xfe" + b"\0" * 8
                + (2).to_bytes(4, "little") + b"\0" * 16
            )
            fat = base / "fat.dylib"
            thin = (
                b"\xfe\xed\xfa\xcf" + b"\0" * 8
                + (6).to_bytes(4, "big") + b"\0" * 16
            )
            # Fat big-endian header: one arch entry pointing at `thin`
            # (MH_DYLIB, 6) right after the 8 + 20 byte fat header.
            fat.write_bytes(
                b"\xca\xfe\xba\xbe" + (1).to_bytes(4, "big")
                + b"\0" * 8 + (28).to_bytes(4, "big")
                + len(thin).to_bytes(4, "big") + b"\0" * 4 + thin
            )
            self.assertEqual(release.macho_filetypes(tool), [release.MH_EXECUTE])
            self.assertEqual(release.macho_filetypes(fat), [6])
            self.assertEqual(
                set(release.macos_main_executable_dirs(base)),
                {base, base / "274bot.app/Contents/MacOS", extra},
            )

    def test_minimum_os_above_the_floor_is_named(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = self.make_package(Path(temporary))
            binary = base / "panel-play"
            above = release.macho_minimum_os_violations(binary, base, ["14.0"])
            at_floor = release.macho_minimum_os_violations(binary, base, ["11.0"])
            below = release.macho_minimum_os_violations(binary, base, ["10.15"])
            mixed_fat = release.macho_minimum_os_violations(
                binary, base, ["11.0", "14.0"])
            missing = release.macho_minimum_os_violations(binary, base, [])
        self.assertEqual(above, ["panel-play: minos 14.0 exceeds macOS 11.0"])
        self.assertEqual(at_floor, [])
        self.assertEqual(below, [])
        self.assertEqual(mixed_fat, ["panel-play: minos 14.0 exceeds macOS 11.0"])
        self.assertEqual(len(missing), 1)
        self.assertIn("panel-play: missing minimum macOS version", missing[0])

    def test_verify_checks_every_macho_and_names_the_offender(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = self.make_package(Path(temporary))

            def linkage(path):
                if path.name == "libfoo.dylib" and "274bot.app" in str(path):
                    return ["/opt/local/lib/libiconv.2.dylib"], [], ["11.0"]
                return ["/usr/lib/libSystem.B.dylib"], [], ["11.0"]

            with mock.patch.object(release, "otool_linkage", side_effect=linkage):
                with self.assertRaisesRegex(
                    release.ReleaseError,
                    r"274bot\.app/Contents/Frameworks/libfoo\.dylib: "
                    r"/opt/local/lib/libiconv\.2\.dylib",
                ):
                    release.verify_macos_linkage(base)
                clean = mock.patch.object(
                    release, "otool_linkage",
                    return_value=(["/usr/lib/libSystem.B.dylib"], [], ["11.0"]),
                )
                with clean:
                    checked = release.verify_macos_linkage(base)
            self.assertEqual(checked, [
                "274bot.app/Contents/Frameworks/libfoo.dylib",
                "274bot.app/Contents/MacOS/panel-play",
                "Frameworks/libfoo.dylib",
                "panel-play",
                "tui-play",
            ])

    def test_verify_rejects_a_minos_above_the_floor(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = self.make_package(Path(temporary))

            def linkage(path):
                minos = ["14.0"] if path.name == "tui-play" else ["11.0"]
                return ["/usr/lib/libSystem.B.dylib"], [], minos

            with mock.patch.object(release, "otool_linkage", side_effect=linkage):
                with self.assertRaisesRegex(
                    release.ReleaseError,
                    r"tui-play: minos 14\.0 exceeds macOS 11\.0",
                ):
                    release.verify_macos_linkage(base)

    def test_verify_requires_the_shipped_executables(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary) / "package"
            base.mkdir()
            (base / "panel-play").write_bytes(b"\xcf\xfa\xed\xfe")
            with self.assertRaisesRegex(release.ReleaseError, "tui-play"):
                release.verify_macos_linkage(base)


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

    def test_release_notes_get_the_credits_footer_once(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            notes = root / "notes.md"
            notes.write_text("## 0.2.0.2\n\n- Fixed things.\n")
            footer = release.NOTES_FOOTER.read_text().strip()

            composed = release.compose_release_notes(notes, root / "out.md").read_text()
            again = release.compose_release_notes(root / "out.md", root / "out.md").read_text()

            self.assertTrue(composed.startswith("## 0.2.0.2\n\n- Fixed things.\n\n"))
            self.assertEqual(composed.count(footer), 1)
            self.assertEqual(again, composed)

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
            name = "274bot-0.1.9-linux-x64.tar.gz"
            candidate = root / name
            candidate.write_bytes(b"candidate")
            stale = root / "274bot-0.1.7-linux-arm64.tar.gz"
            stale.write_bytes(b"stale")
            with self.assertRaisesRegex(release.ReleaseError, "unexpected"):
                release.update_sha256s(root, "0.1.9")
            stale.unlink()
            release.update_sha256s(root, "0.1.9")
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

    def test_macos_patch_finalize_renames_package_and_reuses_notarization_on_retry(self):
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
                release_notes=notes, tag="0.1.9.1", notarization_id=None,
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
                first = release.finalize_worker(args)
                second = release.finalize_worker(args)
            self.assertEqual(first.name, "274bot-0.1.9.1-macos-arm64.zip")
            self.assertEqual(second, first)
            self.assertFalse(package.exists())
            self.assertTrue((root / "274bot-0.1.9.1-macos-arm64" / "274bot.app").is_dir())
            release.write_json(root / "macos-finalize-result.json", {"archive": first.name})
            self.assertIn(
                "274bot-0.1.9.1-macos-arm64",
                release.retain_release_outputs(root, commit, "macos"),
            )
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
            release.update_sha256s(artifacts, "0.1.9")
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
