#!/usr/bin/env python3
"""Preflight, build, finalize, and verify pinned 274bot release packages.

The controller exports an exact host commit and its client gitlink, records
SHA-256 tree digests for every native build input, and dispatches the same
worker to macOS, the Linux builder, or Windows through windows-ssh.py. It does
not tag, push, publish, or create a GitHub release.
"""

import argparse
import datetime
import hashlib
import importlib.util
import json
import os
import posixpath
import re
import shlex
import shutil
import stat
import struct
import subprocess
import sys
import tarfile
import tempfile
import zipfile
from pathlib import Path, PurePosixPath

PLATFORMS = ("macos", "linux", "windows")
ARCHIVE_SUFFIX = {"macos": ".zip", "linux": ".tar.gz", "windows": ".zip"}
ARCH_NAME = {"macos": "macos-arm64", "linux": "linux-x64", "windows": "windows-x64"}
TARGET = {
    "macos": "aarch64-apple-darwin",
    "linux": "x86_64-unknown-linux-gnu",
    "windows": "x86_64-pc-windows-msvc",
}
NAV_NAMES = (
    "274bot.navpack",
    "274bot.navflags",
    "274bot.navreach",
    "274bot.navcanlight",
    "274bot.navpois",
    "274bot.navpack.json",
)
FULL_COMMIT = re.compile(r"^[0-9a-f]{40}$")
NOTES_FOOTER = Path(__file__).resolve().parent / "notes-footer.md"
SAFE_REMOTE = re.compile(r"^[A-Za-z0-9._/-]+$")
RELEASE_TAG = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+(?:\.[0-9]+)*$")


class ReleaseError(RuntimeError):
    pass


def sha256_file(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_json(path, value):
    Path(path).write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def run(command, *, cwd=None, env=None, capture=False, timeout=None):
    result = subprocess.run(
        [str(part) for part in command],
        cwd=cwd,
        env=env,
        text=True,
        capture_output=capture,
        timeout=timeout,
        check=False,
    )
    if result.returncode:
        detail = (result.stderr or result.stdout or "").strip() if capture else ""
        raise ReleaseError("command failed ({}): {}{}".format(
            result.returncode,
            shlex.join(str(part) for part in command),
            "\n" + detail if detail else "",
        ))
    return result.stdout.strip() if capture else ""


def run_git(repo, *arguments):
    return run(["git", "-C", Path(repo), *arguments], capture=True)

def require_committed_tools(repo, commit):
    """The controller and transported worker must be the requested commit's bytes."""
    for relative in ("tools/release/release.py", "tools/release/windows-ssh.py"):
        result = subprocess.run(
            ["git", "-C", str(repo), "show", f"{commit}:{relative}"],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if result.returncode:
            raise ReleaseError(f"{commit} does not contain {relative}")
        working = Path(repo) / relative
        if not working.is_file() or working.read_bytes() != result.stdout:
            raise ReleaseError(
                f"{relative} differs from {commit}; run the release from the requested commit"
            )

def require_clean_checkout(repo, commit, label):
    actual = run_git(repo, "rev-parse", "--verify", "HEAD^{commit}")
    if actual != commit:
        raise ReleaseError(f"{label} checkout HEAD is {actual}, expected {commit}")
    dirty = run_git(repo, "status", "--porcelain=v1", "--untracked-files=all")
    if dirty:
        raise ReleaseError(f"{label} checkout is dirty:\n{dirty}")


def require_commit(value, label="commit"):
    if not isinstance(value, str) or not FULL_COMMIT.fullmatch(value):
        raise ReleaseError(f"{label} must be a full lowercase 40-character commit id")
    return value


def client_commit_at(repo, commit):
    require_commit(commit)
    line = run_git(repo, "ls-tree", commit, "vendor/fr-client-rust")
    fields = line.split()
    if len(fields) != 4 or fields[0] != "160000" or fields[3] != "vendor/fr-client-rust":
        raise ReleaseError(f"{commit} has no vendor/fr-client-rust gitlink")
    return require_commit(fields[2], "client gitlink")


def version_at(repo, commit):
    cargo = run_git(repo, "show", f"{commit}:Cargo.toml")
    match = re.search(r"(?ms)^\[workspace\.package\]\s+.*?^version\s*=\s*\"([^\"]+)\"", cargo)
    if not match:
        raise ReleaseError("workspace.package.version is missing from Cargo.toml")
    return match.group(1)

def release_name_at(repo, commit):
    source = run_git(repo, "show", f"{commit}:crates/panel/src/build_info.rs")
    match = re.search(r'pub const RELEASE: &str = "([^"]+)";', source)
    if not match:
        raise ReleaseError("panel RELEASE is missing from build_info.rs")
    return match.group(1).title()


def validate_tag(tag, version):
    if not isinstance(tag, str) or not RELEASE_TAG.fullmatch(tag):
        raise ReleaseError("release tag must contain only numeric dot-separated components")
    if tag != version and not tag.startswith(version + "."):
        raise ReleaseError(f"release tag {tag} does not match package version {version}")


def validate_release_identity(expect, manifest, tag):
    validate_tag(tag, expect["version"])
    if manifest.get("version") != expect["version"]:
        raise ReleaseError("package manifest version does not match expect.json")
    if manifest.get("release") != expect.get("release"):
        raise ReleaseError("package manifest release name does not match expect.json")


def validate_rustc_host(output, platform):
    match = re.search(r"(?m)^host: (\S+)$", output)
    if not match or match.group(1) != TARGET[platform]:
        found = match.group(1) if match else "missing"
        raise ReleaseError(
            f"rustc host {found} does not match {TARGET[platform]} for {platform}"
        )


def validate_remote_root(value, label):
    if not value or not SAFE_REMOTE.fullmatch(value):
        raise ReleaseError(f"{label} must contain only letters, digits, '.', '_', '-', and '/'")
    path = PurePosixPath(value)
    if path.is_absolute() or any(part in ("", ".", "..") for part in path.parts):
        raise ReleaseError(f"{label} must be a relative path below the remote home directory")
    return path.as_posix()


def tree_digest(base):
    """Hash a tree as sorted path/NUL/file-hash rows, excluding .git metadata."""
    base = Path(base)
    if not base.is_dir():
        raise ReleaseError(f"input directory is missing: {base}")
    rows = {}
    for root, directories, files in os.walk(base):
        directories[:] = sorted(name for name in directories if name != ".git")
        for name in sorted(name for name in files if name != ".git"):
            path = Path(root) / name
            if path.is_symlink():
                raise ReleaseError(f"input tree contains a symlink: {path}")
            relative = path.relative_to(base).as_posix()
            rows[relative] = sha256_file(path)
    digest = hashlib.sha256()
    for name in sorted(rows):
        digest.update(name.encode("utf-8") + b"\0" + rows[name].encode("ascii") + b"\n")
    return {"sha256": digest.hexdigest(), "files": len(rows)}


def input_paths(inputs_root, expect):
    root = Path(inputs_root)
    return {
        "pack": root / "engine/data/pack/client",
        "content": root / "content",
        "snapshot": root / "snapshots" / expect["snapshot_version"],
    }


def verify_expected_input_digests(paths, expect):
    expected = expect.get("inputs")
    if not isinstance(expected, dict):
        raise ReleaseError("expect.json has no inputs object")
    actual = {name: tree_digest(path) for name, path in paths.items()}
    for name in ("pack", "content", "snapshot"):
        if actual[name] != expected.get(name):
            raise ReleaseError(
                f"input digest mismatch for {name}: {actual[name]} != {expected.get(name)}"
            )
    return actual


def verify_input_digests(inputs_root, expect):
    return verify_expected_input_digests(input_paths(inputs_root, expect), expect)


def verify_input_digests_from_roots(args, expect):
    return verify_expected_input_digests({
        "pack": Path(args.engine_dir) / "data/pack/client",
        "content": Path(args.content_dir),
        "snapshot": Path(args.snapshot_root) / expect["snapshot_version"],
    }, expect)


def safe_tar_members(archive):
    with tarfile.open(archive, "r:*") as source:
        members = source.getmembers()
        seen = set()
        for member in members:
            name = PurePosixPath(member.name)
            if (
                not member.name
                or "\\" in member.name
                or name.is_absolute()
                or any(part in ("", ".", "..") for part in name.parts)
            ):
                raise ReleaseError(f"unsafe archive member: {member.name}")
            normalized = name.as_posix()
            if normalized in seen:
                raise ReleaseError(f"duplicate archive member: {member.name}")
            seen.add(normalized)
            if member.issym() or member.islnk() or not (member.isdir() or member.isfile()):
                raise ReleaseError(f"unsupported archive member: {member.name}")
        return members


def safe_extract_tar(archive, destination):
    destination = Path(destination)
    if destination.exists():
        raise ReleaseError(f"extraction destination already exists: {destination}")
    destination.mkdir(parents=True)
    members = safe_tar_members(archive)
    with tarfile.open(archive, "r:*") as source:
        source.extractall(destination, members=members)


def safe_extract_zip(archive, destination):
    destination = Path(destination)
    if destination.exists():
        raise ReleaseError(f"extraction destination already exists: {destination}")
    destination.mkdir(parents=True)
    with zipfile.ZipFile(archive) as source:
        seen = set()
        for info in source.infolist():
            name = PurePosixPath(info.filename)
            mode = info.external_attr >> 16
            if (
                not info.filename
                or "\\" in info.filename
                or name.is_absolute()
                or any(part in ("", ".", "..") for part in name.parts)
                or stat.S_ISLNK(mode)
            ):
                raise ReleaseError(f"unsafe zip member: {info.filename}")
            normalized = name.as_posix()
            if normalized in seen:
                raise ReleaseError(f"duplicate zip member: {info.filename}")
            seen.add(normalized)
        for info in source.infolist():
            target = destination.joinpath(*PurePosixPath(info.filename).parts)
            if info.is_dir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            with source.open(info) as member, target.open("wb") as output:
                shutil.copyfileobj(member, output)
            mode = (info.external_attr >> 16) & 0o777
            if mode:
                target.chmod(mode)


def ensure_tree_is_regular(root, exclude_git=False):
    root = Path(root)
    for directory, directories, files in os.walk(root):
        if exclude_git:
            directories[:] = [name for name in directories if name != ".git"]
        for name in [*directories, *files]:
            path = Path(directory) / name
            if path.is_symlink() or not (path.is_dir() or path.is_file()):
                raise ReleaseError(f"unsupported file in export: {path}")


def add_tree_to_tar(archive, source, arcname, exclude_git=False):
    source = Path(source)
    ensure_tree_is_regular(source, exclude_git)
    prefix = PurePosixPath(arcname)

    def filter_member(member):
        relative = PurePosixPath(member.name).relative_to(prefix)
        if exclude_git and ".git" in relative.parts:
            return None
        return member

    archive.add(source, arcname=arcname, recursive=True, filter=filter_member)


def create_source_archive(repo, client_repo, commit, client_commit, output):
    """Export the exact host and client commits into one source archive."""
    with tempfile.TemporaryDirectory(prefix="274bot-source-export-") as temporary:
        temporary = Path(temporary)
        host_tar = temporary / "host.tar"
        client_tar = temporary / "client.tar"
        run(["git", "-C", repo, "archive", "--format=tar", "--output", host_tar, commit])
        run(["git", "-C", client_repo, "archive", "--format=tar", "--output", client_tar,
             client_commit])
        source = temporary / "source"
        safe_extract_tar(host_tar, source)
        client = source / "vendor/fr-client-rust"
        client.parent.mkdir(parents=True, exist_ok=True)
        if client.exists():
            if not client.is_dir() or any(client.iterdir()):
                raise ReleaseError("host export contains files at the client gitlink")
            client.rmdir()
        safe_extract_tar(client_tar, client)
        with tarfile.open(output, "w:gz") as archive:
            for child in sorted(source.iterdir(), key=lambda path: path.name):
                add_tree_to_tar(archive, child, child.name)
    return {"sha256": sha256_file(output), "bytes": Path(output).stat().st_size}


def create_inputs_archive(engine_dir, content_dir, snapshot_root, snapshot_version, output):
    pack = Path(engine_dir) / "data/pack/client"
    content = Path(content_dir)
    snapshot = Path(snapshot_root) / snapshot_version
    inputs = {
        "pack": tree_digest(pack),
        "content": tree_digest(content),
        "snapshot": tree_digest(snapshot),
    }
    with tarfile.open(output, "w:gz") as archive:
        add_tree_to_tar(archive, pack, "engine/data/pack/client", exclude_git=True)
        add_tree_to_tar(archive, content, "content", exclude_git=True)
        add_tree_to_tar(archive, snapshot, f"snapshots/{snapshot_version}", exclude_git=True)
    return inputs, {"sha256": sha256_file(output), "bytes": Path(output).stat().st_size}


def verify_file_record(path, record, label):
    if not Path(path).is_file():
        raise ReleaseError(f"missing {label}: {path}")
    actual = {"sha256": sha256_file(path), "bytes": Path(path).stat().st_size}
    if actual != record:
        raise ReleaseError(f"{label} digest mismatch: {actual} != {record}")


def package_base(expect, platform):
    return f"274bot-{expect['version']}-{ARCH_NAME[platform]}"


def release_base(tag, platform):
    """Final package name. A patch tag (0.1.9.1 on crate version 0.1.9) names
    the package so it cannot collide with the base release's archives."""
    return f"274bot-{tag}-{ARCH_NAME[platform]}"


def selected_platforms(value):
    return PLATFORMS if value == "all" else (value,)


def render_build_plan(args, commit, client_commit, version):
    platforms = []
    cargo = "cargo build --locked --release -p panel --bin panel-play -p tui --bin tui-play"
    for platform in selected_platforms(args.platform):
        if platform == "macos":
            executor = "local macOS"
            transport = "none"
        elif platform == "linux":
            executor = args.linux_host
            transport = "OpenSSH ssh/scp"
        else:
            executor = args.windows_host or "${RELEASE_WINDOWS_HOST}"
            transport = "tools/release/windows-ssh.py"
        platforms.append({
            "platform": platform,
            "executor": executor,
            "transport": transport,
            "target": TARGET[platform],
            "package": f"274bot-{version}-{ARCH_NAME[platform]}",
            "steps": [
                "transfer source.tar.gz, inputs.tar.gz, expect.json, and the worker",
                "verify both archive SHA-256 records",
                "extract into a fresh per-commit workspace",
                "recompute pack/content/snapshot tree digests and compare expect.json",
                "set GIT_DIRTY=0 and BOT_NAV_BUILD=require",
                cargo,
                "write a per-file build receipt and run package.py",
            ],
        })
    return {
        "mode": "dry-run",
        "action": "build",
        "host_commit": commit,
        "client_commit": client_commit,
        "version": version,
        "revision": int(args.revision),
        "source": "git archive of the exact host commit plus its exact client gitlink",
        "inputs": {
            "pack": "${ENGINE_DIR}/data/pack/client",
            "content": "${CONTENT_DIR} excluding .git",
            "snapshot": "${SNAPSHOT_ROOT}/${SNAPSHOT_VERSION}",
            "manifest": "expect.json records SHA-256 tree digest and file count for each",
        },
        "platforms": platforms,
        "not_performed": ["build", "notarize", "tag", "push", "publish"],
    }


def prepare_payload(args, repo, commit, client_commit, version):
    missing = [
        name for name in (
            "work_dir", "engine_dir", "content_dir", "snapshot_root", "snapshot_version"
        )
        if not getattr(args, name)
    ]
    if missing:
        raise ReleaseError(
            "build requires "
            + ", ".join("--" + name.replace("_", "-") for name in missing)
        )
    require_committed_tools(repo, commit)
    require_clean_checkout(repo, commit, "host")
    client_repo = Path(args.client_root or repo / "vendor/fr-client-rust").resolve()
    if run_git(client_repo, "cat-file", "-t", client_commit) != "commit":
        raise ReleaseError(f"client repository does not contain {client_commit}")
    require_clean_checkout(client_repo, client_commit, "client")
    release_name = release_name_at(repo, commit)
    commit_root = Path(args.work_dir).resolve() / commit
    prepared = commit_root / "prepared"
    source_archive = prepared / "source.tar.gz"
    inputs_archive = prepared / "inputs.tar.gz"
    expect_path = prepared / "expect.json"
    payload = prepared / "payload.tar.gz"
    if prepared.exists():
        required = (source_archive, inputs_archive, expect_path, payload)
        if not all(path.is_file() for path in required):
            raise ReleaseError(
                f"incomplete prepared release: {prepared}; "
                "use build --reset-prepared to recreate it"
            )
        expect = json.loads(expect_path.read_text())
        identity = {
            "host_commit": commit,
            "client_commit": client_commit,
            "version": version,
            "release": release_name,
            "revision": int(args.revision),
            "snapshot_version": args.snapshot_version,
            "jobs": args.jobs,
        }
        for key, value in identity.items():
            if expect.get(key) != value:
                raise ReleaseError(
                    f"prepared {key} is {expect.get(key)!r}, requested {value!r}; "
                    "use build --reset-prepared"
                )
        for name, record in expect["archives"].items():
            verify_file_record(prepared / name, record, name)
        verify_input_digests_from_roots(args, expect)
        return commit_root, payload, expect
    prepared.mkdir(parents=True)
    try:
        source_record = create_source_archive(
            repo, client_repo, commit, client_commit, source_archive
        )
        inputs, inputs_record = create_inputs_archive(
            args.engine_dir,
            args.content_dir,
            args.snapshot_root,
            args.snapshot_version,
            inputs_archive,
        )
        expect = {
            "schema": 1,
            "host_commit": commit,
            "client_commit": client_commit,
            "version": version,
            "release": release_name,
            "revision": int(args.revision),
            "snapshot_version": args.snapshot_version,
            "features": "default",
            "jobs": args.jobs,
            "inputs": inputs,
            "archives": {
                "source.tar.gz": source_record,
                "inputs.tar.gz": inputs_record,
            },
        }
        write_json(expect_path, expect)
        release_tool = Path(repo) / "tools/release/release.py"
        windows_tool = Path(repo) / "tools/release/windows-ssh.py"
        with tarfile.open(payload, "w:gz") as archive:
            for path in (
                source_archive,
                inputs_archive,
                expect_path,
                release_tool,
                windows_tool,
            ):
                archive.add(
                    path,
                    arcname="release.py" if path == release_tool else path.name,
                )
    except Exception:
        shutil.rmtree(prepared, ignore_errors=True)
        raise
    return commit_root, payload, expect


def worker_expect(root, commit, platform):
    expect_path = Path(root) / "expect.json"
    if not expect_path.is_file():
        raise ReleaseError(f"missing worker manifest: {expect_path}")
    expect = json.loads(expect_path.read_text())
    if expect.get("schema") != 1:
        raise ReleaseError("unsupported expect.json schema")
    if expect.get("host_commit") != commit:
        raise ReleaseError("worker commit does not match expect.json")
    require_commit(expect.get("client_commit"), "expected client commit")
    if platform not in PLATFORMS:
        raise ReleaseError(f"unsupported platform: {platform}")
    return expect


# The documented macOS floor. One constant feeds the manifest line in
# runtime_requirements, the MACOSX_DEPLOYMENT_TARGET the worker pins, and
# the minos limit verification enforces, so the three cannot drift apart.
MACOS_DEPLOYMENT_TARGET = "11.0"

# The macOS builder is a developer machine whose login PATH puts MacPorts
# (/opt/local/bin) and Homebrew ahead of /usr/bin. Anything found through
# that PATH -- MacPorts' pkg-config answers `-L/opt/local/lib` for bzip2 --
# or through an inherited search-path/toolchain variable can link a shipped
# binary against libraries users' Macs do not have, or move it off the
# pinned toolchain. A denylist of such variables is not a closed set (a
# shell can set RUSTUP_TOOLCHAIN, CARGO_HOME, CCC_OVERRIDE_OPTIONS, or any
# future <LIB>_DIR without touching it), so the macOS worker environment
# is an allowlist built from empty: only the keys below are copied from
# the parent, everything else about the build environment is constructed
# here, and SDKROOT is left unset so xcrun rediscovers the SDK through
# DEVELOPER_DIR / xcode-select. None of the dropped variables was set in
# the environment that produced the known-good 0.2.1 package, so the
# allowlist cannot change what that build finds. (~/.cargo/config.toml can
# still inject rustflags from outside the environment; the otool linkage
# check below is the fail-closed backstop for that.)
MACOS_SYSTEM_PATH_DIRS = ("/usr/bin", "/bin", "/usr/sbin", "/sbin")
# Copied from the parent only when set, each linkage-neutral or needed to
# identify the build user: USER/LOGNAME (codesign and passwd lookups),
# TMPDIR (the per-user clang/rustc temp dir; unset falls back to /tmp),
# LANG and LC_* (locale only), and the proxy variables (network only, for
# a cargo fetch that misses the local cache). PATH, HOME and
# MACOSX_DEPLOYMENT_TARGET are also allowlisted but always constructed,
# never inherited; DEVELOPER_DIR is copied only after validation below.
# Deliberately not copied: CARGO_HOME/RUSTUP_HOME/RUSTUP_TOOLCHAIN (the
# fixed PATH already names ~/.cargo/bin and rust-toolchain.toml pins the
# toolchain) and every search-path, compiler-selection and DYLD_ variable.
MACOS_PASSTHROUGH_ENV = (
    "USER", "LOGNAME", "TMPDIR", "LANG",
    "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY",
    "http_proxy", "https_proxy", "all_proxy", "no_proxy",
)
MACOS_DEVELOPER_DIRS = re.compile(
    r"^(?:/Library/Developer/CommandLineTools"
    r"|/Applications/[^/]+\.app/Contents/Developer)$"
)


def macos_build_path(home):
    """The only PATH a macOS release build runs with: rustup, then system."""
    return os.pathsep.join(
        [str(Path(home) / ".cargo/bin"), *MACOS_SYSTEM_PATH_DIRS]
    )


def valid_macos_developer_dir(value):
    """True only for an existing Apple developer directory: the Command
    Line Tools or an Xcode app's Contents/Developer. Any other value is
    how a non-Apple toolchain would get into the link, so it is dropped
    and xcrun falls back to xcode-select."""
    return (
        MACOS_DEVELOPER_DIRS.match(value) is not None
        and Path(value).is_dir()
    )


def macos_allowlisted_env(source=None):
    """The inherited half of the macOS worker environment, built from
    empty: the justified passthrough keys above, a validated
    DEVELOPER_DIR, and the constructed HOME/PATH/deployment target.
    native_build_environment adds the worker's own pinned keys on top."""
    source = os.environ if source is None else source
    environment = {
        name: source[name]
        for name in MACOS_PASSTHROUGH_ENV
        if name in source
    }
    environment.update({
        name: value
        for name, value in source.items()
        if name.startswith("LC_")
    })
    developer_dir = source.get("DEVELOPER_DIR")
    if developer_dir and valid_macos_developer_dir(developer_dir):
        environment["DEVELOPER_DIR"] = developer_dir
    home = Path.home()
    environment["HOME"] = str(home)
    environment["PATH"] = macos_build_path(home)
    environment["MACOSX_DEPLOYMENT_TARGET"] = MACOS_DEPLOYMENT_TARGET
    return environment


def native_build_environment(expect, root, target, platform, rusty_v8_archive=None):
    if platform == "macos":
        # Allowlist from empty (see macos_allowlisted_env); the worker's
        # own pinned keys below are the only other entries.
        environment = macos_allowlisted_env()
    else:
        environment = os.environ.copy()
        environment.pop("RUSTY_V8_ARCHIVE", None)
    environment.update({
        "GIT_COMMIT": expect["host_commit"],
        "CLIENT_COMMIT": expect["client_commit"],
        "GIT_DIRTY": "0",
        "BOT_NAV_REVISION": str(expect["revision"]),
        "BOT_NAV_BUILD": "require",
        "BOT_NAV_ENGINE_DIR": str(root / "engine"),
        "BOT_NAV_CONTENT_DIR": str(root / "content"),
        "BOT_NAV_SNAPSHOT_ROOT": str(root / "snapshots"),
        "CARGO_TARGET_DIR": str(target),
        "CARGO_BUILD_JOBS": str(expect["jobs"]),
        "CARGO_INCREMENTAL": "0",
    })
    if platform != "macos":
        environment["PATH"] = (
            str(Path.home() / ".cargo/bin") + os.pathsep + environment.get("PATH", "")
        )
    if platform == "windows":
        if not rusty_v8_archive:
            raise ReleaseError("Windows build requires --rusty-v8-archive")
        environment["RUSTY_V8_ARCHIVE"] = rusty_v8_archive
    return environment


def build_worker(args):
    root = Path(args.worker_root).resolve()
    expect = worker_expect(root, args.commit, args.platform)
    if (args.platform == "windows") != (os.name == "nt"):
        raise ReleaseError("Windows packages must be built on Windows")
    if args.platform == "macos" and sys.platform != "darwin":
        raise ReleaseError("macOS packages must be built on macOS")
    if args.platform == "linux" and not sys.platform.startswith("linux"):
        raise ReleaseError("Linux packages must be built on Linux")
    for name, record in expect["archives"].items():
        verify_file_record(root / name, record, name)
    source = root / "source"
    inputs = root / "inputs"
    safe_extract_tar(root / "source.tar.gz", source)
    safe_extract_tar(root / "inputs.tar.gz", inputs)
    actual_inputs = verify_input_digests(inputs, expect)
    target = root / "cache/target"
    environment = native_build_environment(
        expect, inputs, target, args.platform, args.rusty_v8_archive
    )
    command = [
        "cargo", "build", "--locked", "--release",
        "-p", "panel", "--bin", "panel-play",
        "-p", "tui", "--bin", "tui-play",
    ]
    log_path = root / f"{args.platform}-build.log"
    with log_path.open("w") as log:
        result = subprocess.run(command, cwd=source, env=environment, stdout=log,
                                stderr=subprocess.STDOUT, check=False)
    write_json(root / f"{args.platform}-build-exit.json", {"exit_code": result.returncode})
    if result.returncode:
        raise ReleaseError(f"native build failed; see {log_path}")
    profile = target / "release"
    suffix = ".exe" if args.platform == "windows" else ""
    artifacts = [profile / (name + suffix) for name in ("panel-play", "tui-play")]
    artifacts += [profile / "nav" / str(expect["revision"]) / name for name in NAV_NAMES]
    for path in artifacts:
        if not path.is_file():
            raise ReleaseError(f"build omitted required artifact: {path}")
    rustc = run(["rustc", "-Vv"], cwd=source, env=environment, capture=True)
    validate_rustc_host(rustc, args.platform)
    receipt = {
        "host_commit": expect["host_commit"],
        "client_commit": expect["client_commit"],
        "target": TARGET[args.platform],
        "rustc": rustc,
        "features": "default",
        "built_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "nav_inputs": actual_inputs,
        "files": {path.relative_to(profile).as_posix(): sha256_file(path) for path in artifacts},
    }
    receipt_path = root / f"{args.platform}-build-receipt.json"
    write_json(receipt_path, receipt)
    output = root / package_base(expect, args.platform)
    package_command = [
        sys.executable,
        source / "tools/release/package.py",
        "--platform", args.platform,
        "--input", profile,
        "--output", output,
        "--build-receipt", receipt_path,
        "--revision", str(expect["revision"]),
        "--map-cache", inputs / "engine/data/pack/client",
        "--map-unpack", inputs / "snapshots",
    ]
    if args.platform == "macos":
        if not args.sign_identity:
            raise ReleaseError("macOS release build requires --sign-identity")
        package_command += ["--app-profile", args.app_profile, "--sign-identity", args.sign_identity]
    run(package_command, cwd=source, env=environment)
    result_record = {
        "platform": args.platform,
        "package": output.name,
        "receipt": receipt_path.name,
        "host_commit": expect["host_commit"],
        "client_commit": expect["client_commit"],
    }
    write_json(root / f"{args.platform}-build-result.json", result_record)
    print(json.dumps(result_record, indent=2, sort_keys=True))


def ssh_base(identity=None, known_hosts=None):
    command = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=15",
               "-o", "StrictHostKeyChecking=yes", "-o", "IdentitiesOnly=yes"]
    if known_hosts:
        command += ["-o", f"UserKnownHostsFile={known_hosts}"]
    if identity:
        command += ["-i", str(identity)]
    return command


def scp_base(identity=None, known_hosts=None):
    command = ["scp", "-o", "BatchMode=yes", "-o", "ConnectTimeout=15",
               "-o", "StrictHostKeyChecking=yes", "-o", "IdentitiesOnly=yes"]
    if known_hosts:
        command += ["-o", f"UserKnownHostsFile={known_hosts}"]
    if identity:
        command += ["-i", str(identity)]
    return command


def linux_root(args, commit):
    return f"{validate_remote_root(args.linux_remote_root, '--linux-remote-root')}/{commit}/linux"


def linux_run(args, script):
    run([*ssh_base(args.linux_identity, args.linux_known_hosts), args.linux_host, script])


def linux_put(args, local, remote):
    run([*scp_base(args.linux_identity, args.linux_known_hosts), local,
         f"{args.linux_host}:{remote}"])


def linux_get(args, remote, local):
    run([*scp_base(args.linux_identity, args.linux_known_hosts),
         f"{args.linux_host}:{remote}", local])


def windows_transport(args, action, *values):
    helper = Path(__file__).with_name("windows-ssh.py")
    command = [sys.executable, helper, "--host", args.windows_host]
    if args.windows_identity:
        command += ["--identity", str(args.windows_identity)]
    if args.windows_known_hosts:
        command += ["--known-hosts", str(args.windows_known_hosts)]
    command += [action, *map(str, values)]
    run(command)


def windows_root(args, commit):
    return f"{validate_remote_root(args.windows_remote_root, '--windows-remote-root')}/{commit}/windows"


def powershell_quote(value):
    value = str(value)
    if any(character in value for character in "\x00\r\n\u2018\u2019\u201a\u201b"):
        raise ReleaseError("PowerShell argument contains a forbidden quote or control character")
    return "'" + value.replace("'", "''") + "'"


def stage_payload(args, platform, commit_root, payload):
    if platform == "macos":
        root = commit_root / "macos"
        safe_extract_tar(payload, root)
        return root
    if platform == "linux":
        remote = linux_root(args, args.commit)
        linux_run(args, f"mkdir -p {shlex.quote(remote)}")
        linux_put(args, payload, f"{remote}/payload.tar.gz")
        linux_run(args, f"cd {shlex.quote(remote)} && tar -xzf payload.tar.gz")
        return remote
    remote = windows_root(args, args.commit)
    create = "$root = Join-Path $HOME {}; New-Item -ItemType Directory -Force $root | Out-Null".format(
        powershell_quote(remote.replace("/", "\\"))
    )
    windows_transport(args, "run", create)
    windows_transport(args, "put", payload, f"{remote}/payload.tar.gz")
    extract = "$root = Join-Path $HOME {}; Set-Location $root; python -m tarfile -e payload.tar.gz .".format(
        powershell_quote(remote.replace("/", "\\"))
    )
    windows_transport(args, "run", extract)
    return remote

def clear_platform_stage(args, platform, commit_root):
    if platform == "macos":
        shutil.rmtree(commit_root / platform, ignore_errors=True)
        return
    if platform == "linux":
        remote = linux_root(args, args.commit)
        linux_run(args, f"rm -rf {shlex.quote(remote)}")
        return
    remote = windows_root(args, args.commit)
    script = (
        "$root = Join-Path $HOME {}; "
        "if (Test-Path $root) {{ Remove-Item -Recurse -Force -ErrorAction Stop $root }}"
    ).format(powershell_quote(remote.replace("/", "\\")))
    windows_transport(args, "run", script)


def dispatch_build(args, platform, root):
    if platform == "macos":
        worker = argparse.Namespace(**vars(args))
        worker.worker_root = root
        worker.platform = platform
        build_worker(worker)
        return
    if platform == "linux":
        command = ["python3", "-u", "release.py", "build", "--commit", args.commit,
                   "--platform", platform, "--worker-root", "."]
        linux_run(args, f"cd {shlex.quote(root)} && {shlex.join(command)}")
        return
    command = ["python", "-u", "release.py", "build", "--commit", args.commit,
               "--platform", platform, "--worker-root", ".",
               "--rusty-v8-archive", args.rusty_v8_archive]
    script = "$root = Join-Path $HOME {}; Set-Location $root; {}".format(
        powershell_quote(root.replace("/", "\\")),
        "& " + " ".join(powershell_quote(value) for value in command),
    )
    windows_transport(args, "run", script)


def build_controller(args):
    repo = Path(args.repo_root or Path(__file__).resolve().parents[2]).resolve()
    require_commit(args.commit)
    run_git(repo, "cat-file", "-e", f"{args.commit}^{{commit}}")
    client_commit = client_commit_at(repo, args.commit)
    version = version_at(repo, args.commit)
    if args.dry_run:
        print(json.dumps(render_build_plan(args, args.commit, client_commit, version),
                         indent=2, sort_keys=True))
        return
    platforms = selected_platforms(args.platform)
    if "windows" in platforms:
        if not args.windows_host:
            raise ReleaseError("Windows build requires --windows-host or RELEASE_WINDOWS_HOST")
        if not args.rusty_v8_archive:
            raise ReleaseError("Windows build requires --rusty-v8-archive or RUSTY_V8_ARCHIVE")
    if "macos" in platforms and not args.sign_identity:
        raise ReleaseError("macOS build requires --sign-identity or RELEASE_SIGN_IDENTITY")
    if args.reset_prepared:
        if not args.work_dir:
            raise ReleaseError("--reset-prepared requires --work-dir")
        shutil.rmtree(Path(args.work_dir).resolve() / args.commit / "prepared",
                      ignore_errors=True)
    commit_root, payload, _expect = prepare_payload(args, repo, args.commit, client_commit, version)
    for platform in platforms:
        if args.force_platform:
            clear_platform_stage(args, platform, commit_root)
        root = stage_payload(args, platform, commit_root, payload)
        dispatch_build(args, platform, root)


def runtime_requirements(platform):
    if platform == "macos":
        return [f"macOS {MACOS_DEPLOYMENT_TARGET} or later; Apple Silicon"]
    if platform == "linux":
        return [
            "glibc >= 2.39",
            "libssl.so.3, libcrypto.so.3, and libasound.so.2",
            "panel-play: a Wayland or X11 session and a Vulkan 1 loader/driver",
        ]
    return ["Microsoft Visual C++ x64 runtime: VCRUNTIME140.dll and VCRUNTIME140_1.dll"]


def package_files(base, manifest_path):
    files = []
    for path in sorted(Path(base).rglob("*")):
        if path.is_symlink():
            raise ReleaseError(f"package contains a symlink: {path}")
        relative = path.relative_to(base)
        if any(part == ".DS_Store" or part.startswith("._") for part in relative.parts):
            raise ReleaseError(f"package contains forbidden metadata: {relative}")
        if path.is_file() and path != manifest_path:
            files.append(path)
    return files


def finalize_manifest(base, platform, tag, notes, notarization_id=None):
    base = Path(base)
    manifest_path = base / "release-manifest.json"
    if not manifest_path.is_file():
        raise ReleaseError(f"missing release manifest: {manifest_path}")
    manifest = json.loads(manifest_path.read_text())
    if manifest.get("platform") != platform:
        raise ReleaseError("package platform does not match finalize platform")
    shutil.copy2(notes, base / "RELEASE-NOTES.md")
    manifest["tag"] = tag
    manifest["runtime_requirements"] = runtime_requirements(platform)
    if platform == "macos":
        if not notarization_id:
            raise ReleaseError("macOS finalization requires an accepted notarization id")
        manifest.update(
            notarized=True,
            status="notarized",
            notarization_id=notarization_id,
        )
    else:
        manifest.update(notarized=False, status="packaged")
        manifest.pop("notarization_id", None)
    manifest["files"] = {
        path.relative_to(base).as_posix(): {
            "sha256": sha256_file(path),
            "bytes": path.stat().st_size,
        }
        for path in package_files(base, manifest_path)
    }
    write_json(manifest_path, manifest)
    return manifest


def create_archive(base, platform, destination):
    base = Path(base)
    destination = Path(destination)
    if destination.exists():
        raise ReleaseError(f"archive already exists: {destination}")
    if platform == "macos":
        run(["ditto", "-c", "-k", "--norsrc", "--keepParent", base, destination])
    elif platform == "linux":
        made = shutil.make_archive(str(destination)[:-7], "gztar", root_dir=base.parent,
                                   base_dir=base.name)
        if Path(made) != destination:
            raise ReleaseError(f"archive path mismatch: {made}")
    else:
        made = shutil.make_archive(str(destination)[:-4], "zip", root_dir=base.parent,
                                   base_dir=base.name)
        if Path(made) != destination:
            raise ReleaseError(f"archive path mismatch: {made}")
    return destination


def finalize_worker(args):
    root = Path(args.worker_root).resolve()
    expect = worker_expect(root, args.commit, args.platform)
    validate_tag(args.tag, expect["version"])
    staged = root / package_base(expect, args.platform)
    base = root / release_base(args.tag, args.platform)
    if staged != base and staged.is_dir():
        if base.exists():
            raise ReleaseError(f"both {staged.name} and {base.name} are staged")
        staged.rename(base)
    if not base.is_dir():
        raise ReleaseError(f"staged package is missing: {staged}")
    notes = Path(args.release_notes).resolve()
    if not notes.is_file():
        raise ReleaseError(f"release notes are missing: {notes}")
    manifest_path = base / "release-manifest.json"
    manifest = json.loads(manifest_path.read_text())
    validate_release_identity(expect, manifest, args.tag)
    archive = root / (base.name + ARCHIVE_SUFFIX[args.platform])
    notarization_id = args.notarization_id
    notary_file = root / "notarization-id.txt"
    if args.platform == "macos":
        if not manifest.get("signed"):
            raise ReleaseError("refusing to notarize an unsigned macOS package")
        if not notarization_id and notary_file.is_file():
            notarization_id = notary_file.read_text().strip()
        if not notarization_id:
            archive.unlink(missing_ok=True)
            create_archive(base, "macos", archive)
            output = run([
                "xcrun", "notarytool", "submit", archive,
                "--keychain-profile", args.notary_profile,
                "--wait", "--output-format", "json",
            ], capture=True)
            result = json.loads(output)
            if result.get("status") != "Accepted" or not result.get("id"):
                raise ReleaseError(f"Apple did not accept the package: {result}")
            notarization_id = result["id"]
            notary_file.write_text(notarization_id + "\n")
        run(["xcrun", "stapler", "staple", base / "274bot.app"])
        run(["xcrun", "stapler", "validate", base / "274bot.app"])
    archive.unlink(missing_ok=True)
    finalize_manifest(base, args.platform, args.tag, notes, notarization_id)
    create_archive(base, args.platform, archive)
    if args.platform == "macos":
        run(["spctl", "-a", "-vv", base / "274bot.app"])
    result = {
        "platform": args.platform,
        "archive": archive.name,
        "sha256": sha256_file(archive),
        "bytes": archive.stat().st_size,
        "notarization_id": notarization_id,
    }
    write_json(root / f"{args.platform}-finalize-result.json", result)
    print(json.dumps(result, indent=2, sort_keys=True))
    return archive


def render_finalize_plan(args):
    plans = []
    for platform in selected_platforms(args.platform):
        steps = [
            "copy RELEASE-NOTES.md (with the credits footer) into the staged package",
            "record runtime requirements and re-hash every package file",
            "create the final archive",
            "copy the archive to the local artifact directory",
            "regenerate SHA256SUMS from local archives",
        ]
        if platform == "macos":
            steps[0:0] = [
                f"submit the signed zip with notary profile {args.notary_profile}",
                "require status Accepted, staple and validate 274bot.app",
            ]
        plans.append({"platform": platform, "steps": steps})
    return {
        "mode": "dry-run",
        "action": "finalize",
        "host_commit": args.commit,
        "tag": args.tag or "${TAG}",
        "platforms": plans,
        "not_performed": ["notarize", "tag", "push", "publish"],
    }


def expected_archive_names(tag):
    return {
        release_base(tag, platform) + ARCHIVE_SUFFIX[platform]
        for platform in PLATFORMS
    }


def update_sha256s(artifact_dir, tag):
    artifact_dir = Path(artifact_dir)
    expected = expected_archive_names(tag)
    actual = {
        path.name for path in artifact_dir.glob("274bot-*")
        if path.is_file() and (
            path.name.endswith(".zip") or path.name.endswith(".tar.gz")
        )
    }
    unexpected = actual - expected
    if unexpected:
        raise ReleaseError(
            "unexpected release archives beside candidate: "
            + ", ".join(sorted(unexpected))
        )
    archives = [artifact_dir / name for name in sorted(actual)]
    if not archives:
        raise ReleaseError(f"no release archives in {artifact_dir}")
    text = "".join(f"{sha256_file(path)}  {path.name}\n" for path in archives)
    (artifact_dir / "SHA256SUMS").write_text(text)


def compose_release_notes(notes, destination):
    """Write the release notes with the credits footer appended exactly once."""
    body = Path(notes).read_text()
    footer = NOTES_FOOTER.read_text()
    if footer.strip() not in body:
        body = body.rstrip("\n") + "\n\n" + footer
    destination = Path(destination)
    destination.write_text(body)
    return destination


def finalize_controller(args):
    require_commit(args.commit)
    if args.dry_run:
        print(json.dumps(render_finalize_plan(args), indent=2, sort_keys=True))
        return
    if not args.work_dir or not args.release_notes or not args.tag:
        raise ReleaseError("finalize requires --work-dir, --release-notes, and --tag")
    notes = Path(args.release_notes).resolve()
    if not notes.is_file():
        raise ReleaseError(f"release notes are missing: {notes}")
    if "windows" in selected_platforms(args.platform) and not args.windows_host:
        raise ReleaseError("Windows finalization requires --windows-host or RELEASE_WINDOWS_HOST")
    commit_root = Path(args.work_dir).resolve() / args.commit
    expect_path = commit_root / "prepared/expect.json"
    if not expect_path.is_file():
        raise ReleaseError(f"prepared manifest is missing: {expect_path}")
    expect = json.loads(expect_path.read_text())
    validate_tag(args.tag, expect["version"])
    notes = compose_release_notes(notes, commit_root / "RELEASE-NOTES.md")
    args.release_notes = notes
    artifact_dir = Path(args.artifact_dir).resolve() if args.artifact_dir else commit_root / "artifacts"
    artifact_dir.mkdir(parents=True, exist_ok=True)
    for platform in selected_platforms(args.platform):
        name = release_base(args.tag, platform) + ARCHIVE_SUFFIX[platform]
        destination = artifact_dir / name
        if destination.exists():
            raise ReleaseError(f"refusing to overwrite release archive: {destination}")
        if platform == "macos":
            worker = argparse.Namespace(**vars(args))
            worker.worker_root = commit_root / "macos"
            worker.platform = platform
            archive = finalize_worker(worker)
            shutil.copy2(archive, destination)
        elif platform == "linux":
            root = linux_root(args, args.commit)
            linux_put(args, notes, f"{root}/RELEASE-NOTES.md")
            command = ["python3", "-u", "release.py", "finalize", "--commit", args.commit,
                       "--platform", platform, "--worker-root", ".", "--tag", args.tag,
                       "--release-notes", "RELEASE-NOTES.md"]
            linux_run(args, f"cd {shlex.quote(root)} && {shlex.join(command)}")
            linux_get(args, f"{root}/{name}", destination)
        else:
            root = windows_root(args, args.commit)
            windows_transport(args, "put", notes, f"{root}/RELEASE-NOTES.md")
            command = ["python", "-u", "release.py", "finalize", "--commit", args.commit,
                       "--platform", platform, "--worker-root", ".", "--tag", args.tag,
                       "--release-notes", "RELEASE-NOTES.md"]
            script = "$root = Join-Path $HOME {}; Set-Location $root; {}".format(
                powershell_quote(root.replace("/", "\\")),
                "& " + " ".join(powershell_quote(value) for value in command),
            )
            windows_transport(args, "run", script)
            windows_transport(args, "get", f"{root}/{name}", destination)
        update_sha256s(artifact_dir, args.tag)
    print(artifact_dir / "SHA256SUMS")


def extracted_base(destination):
    entries = list(Path(destination).iterdir())
    if len(entries) != 1 or not entries[0].is_dir():
        raise ReleaseError(
            f"archive must contain exactly one package directory: {entries}"
        )
    return entries[0]


def verify_package_archive(archive, destination, *, expected_platform=None,
                           expected_commit=None, run_help=False):
    archive = Path(archive)
    destination = Path(destination)
    if archive.name.endswith(".zip"):
        safe_extract_zip(archive, destination)
    elif archive.name.endswith(".tar.gz"):
        safe_extract_tar(archive, destination)
    else:
        raise ReleaseError(f"unsupported release archive: {archive}")
    base = extracted_base(destination)
    manifest_path = base / "release-manifest.json"
    if not manifest_path.is_file():
        raise ReleaseError("release archive has no release-manifest.json")
    manifest = json.loads(manifest_path.read_text())
    if expected_platform and manifest.get("platform") != expected_platform:
        raise ReleaseError("archive platform does not match requested platform")
    if expected_commit and manifest.get("host_commit") != expected_commit:
        raise ReleaseError("archive host commit does not match requested commit")
    files = manifest.get("files")
    if not isinstance(files, dict):
        raise ReleaseError("release manifest has no files object")
    listed = set(files) | {"release-manifest.json"}
    actual = {path.relative_to(base).as_posix() for path in base.rglob("*") if path.is_file()}
    if actual != listed:
        raise ReleaseError(f"archive membership differs from manifest: {sorted(actual ^ listed)[:5]}")
    for name, record in files.items():
        path = base / name
        verify_file_record(path, record, name)
    help_results = {}
    if run_help:
        suffix = ".exe" if manifest["platform"] == "windows" else ""
        for name in ("panel-play", "tui-play"):
            result = subprocess.run(
                [str(base / (name + suffix)), "--help"],
                capture_output=True,
                text=True,
                timeout=60,
                check=False,
            )
            first = (result.stdout or result.stderr).strip().splitlines()
            help_results[name] = {
                "exit": result.returncode,
                "first_line": first[0][:160] if first else "",
            }
            if name == "panel-play" and result.returncode != 0:
                raise ReleaseError(f"{name} --help failed with {result.returncode}")
            if name == "tui-play" and result.returncode not in (0, 2):
                raise ReleaseError(f"{name} --help failed with {result.returncode}")
    return base, manifest, help_results


MACHO_MAGICS = frozenset({
    b"\xfe\xed\xfa\xce", b"\xce\xfa\xed\xfe",  # 32-bit, both byte orders
    b"\xfe\xed\xfa\xcf", b"\xcf\xfa\xed\xfe",  # 64-bit, both byte orders
    b"\xca\xfe\xba\xbe", b"\xbe\xba\xfe\xca",  # fat, both byte orders
    b"\xca\xfe\xba\xbf", b"\xbf\xba\xfe\xca",  # fat64, both byte orders
})
# Thin magic -> byte order of the header fields that follow it; fat magic
# -> (byte order, 64-bit arch entries). In a thin header cputype is 4
# bytes in, cpusubtype 8, filetype 12.
_MACHO_32_MAGICS = {b"\xfe\xed\xfa\xce": ">", b"\xce\xfa\xed\xfe": "<"}
_MACHO_64_MAGICS = {b"\xfe\xed\xfa\xcf": ">", b"\xcf\xfa\xed\xfe": "<"}
_MACHO_THIN_MAGICS = {**_MACHO_32_MAGICS, **_MACHO_64_MAGICS}
_MACHO_FAT_MAGICS = {
    b"\xca\xfe\xba\xbe": (">", False), b"\xbe\xba\xfe\xca": ("<", False),
    b"\xca\xfe\xba\xbf": (">", True), b"\xbf\xba\xfe\xca": ("<", True),
}
MH_EXECUTE = 2
# mach/machine.h cpu types (and, for arm64, subtypes). The macOS release
# builds a single target, aarch64-apple-darwin (TARGET, ARCH_NAME
# "macos-arm64"), so verification accepts only a thin 64-bit arm64
# Mach-O. The subtype is the low 24 bits of the cpusubtype field (the
# top byte is capability bits): 0 (CPU_SUBTYPE_ARM64_ALL) and 1
# (CPU_SUBTYPE_ARM64_V8) are plain arm64, 2 is arm64e.
_CPU_TYPE_ARM64 = 0x0100000C
_CPU_TYPE_NAMES = {
    7: "x86", 0x01000007: "x86_64",
    12: "arm", 0x0200000C: "arm64_32",
    18: "powerpc", 0x01000012: "powerpc64",
}
MACOS_SYSTEM_LIBRARY_PREFIXES = ("/usr/lib/", "/System/Library/")
MACOS_REQUIRED_EXECUTABLES = (
    "panel-play",
    "tui-play",
    "274bot.app/Contents/MacOS/panel-play",
)


def is_macho(path):
    with Path(path).open("rb") as source:
        return source.read(4) in MACHO_MAGICS


def macho_filetype(path):
    """Mach-O filetype of the thin binary at `path` (None if fat, not a
    thin Mach-O, truncated, or unreadable). Fat binaries are refused by
    macho_architecture_violations, so nothing reads their slices."""
    try:
        with Path(path).open("rb") as source:
            header = source.read(16)
    except OSError:
        return None
    if len(header) < 16:
        return None
    endian = _MACHO_THIN_MAGICS.get(header[:4])
    if endian is None:
        return None
    return struct.unpack_from(endian + "I", header, 12)[0]


def _macho_cpu_name(cputype, cpusubtype):
    subtype = cpusubtype & 0x00FFFFFF
    if cputype == _CPU_TYPE_ARM64:
        if subtype in (0, 1):
            return "arm64"
        if subtype == 2:
            return "arm64e"
        return f"arm64 subtype {subtype}"
    return _CPU_TYPE_NAMES.get(
        cputype, f"cpu type {cputype} subtype {subtype}"
    )


def _fat_macho_cpu_names(source, magic):
    """Architecture name of every slice in a fat header, for the refusal
    message: cputype/cpusubtype lead each fat arch entry, so the slices
    themselves are never read. `source` is just past the magic.
    Best-effort: a truncated header names the slices it can."""
    endian, fat64 = _MACHO_FAT_MAGICS[magic]
    raw = source.read(4)
    if len(raw) < 4:
        return []
    (count,) = struct.unpack(endian + "I", raw)
    entry_size = 32 if fat64 else 20
    names = []
    for _ in range(min(count, 32)):
        entry = source.read(entry_size)
        if len(entry) < entry_size:
            break
        cputype, cpusubtype = struct.unpack_from(endian + "II", entry, 0)
        names.append(_macho_cpu_name(cputype, cpusubtype))
    return names


def macho_architecture_violations(binary, base):
    """Architecture violations for one Mach-O, as 'binary: cause'
    strings. The macOS release ships a single target,
    aarch64-apple-darwin, so every shipped Mach-O must be a thin 64-bit
    arm64 binary: a fat/universal header (whatever its slices), an
    x86_64 or arm64e thin binary, a 32-bit binary, and a truncated
    header are each refused, naming the binary and what it is. The cpu
    is read from the Mach-O header directly, so no otool output can
    hide a slice: fat files never reach otool at all."""
    label = _package_label(binary, base)
    allowed = (
        "only thin arm64 Mach-O is shipped "
        "(macOS target aarch64-apple-darwin)"
    )
    try:
        with Path(binary).open("rb") as source:
            magic = source.read(4)
            if magic in _MACHO_FAT_MAGICS:
                names = _fat_macho_cpu_names(source, magic)
                slices = ", ".join(names) if names else "unnamed slices"
                return [
                    f"{label}: fat/universal Mach-O ({slices}); {allowed}"
                ]
            header = magic + source.read(12)
    except OSError as error:
        return [f"{label}: unreadable Mach-O: {error}"]
    endian = _MACHO_THIN_MAGICS.get(magic)
    if endian is None:
        return [f"{label}: not a thin Mach-O; {allowed}"]
    if len(header) < 16:
        return [f"{label}: truncated Mach-O header; {allowed}"]
    cputype, cpusubtype = struct.unpack_from(endian + "II", header, 4)
    name = _macho_cpu_name(cputype, cpusubtype)
    if magic in _MACHO_64_MAGICS and name == "arm64":
        return []
    kind = "64-bit" if magic in _MACHO_64_MAGICS else "32-bit"
    return [f"{label}: {kind} {name} Mach-O; {allowed}"]


def macos_main_executable_dirs(base, macho=None):
    """Directories dyld can mean by @executable_path in this package: the
    parent of every shipped main executable -- the required executables,
    plus any other Mach-O whose filetype is MH_EXECUTE. The package ships
    executables at two depths, so this is not one directory."""
    base = Path(base)
    if macho is None:
        macho = sorted(
            path for path in base.rglob("*") if path.is_file() and is_macho(path)
        )
    required = {base / name for name in MACOS_REQUIRED_EXECUTABLES}
    dirs = []
    for path in macho:
        path = Path(path)
        if path in required or macho_filetype(path) == MH_EXECUTE:
            if path.parent not in dirs:
                dirs.append(path.parent)
    return dirs


def parse_otool_libraries(output):
    """Install names from `otool -arch arm64 -L` output, in order."""
    libraries = []
    for line in output.splitlines():
        # Dependency lines are indented; the file header is not.
        if not line[:1].isspace():
            continue
        entry = re.sub(r"\s+\(compatibility version .*\)$", "", line.strip())
        if entry:
            libraries.append(entry)
    return libraries


def parse_otool_rpaths(output):
    """LC_RPATH paths from `otool -arch arm64 -l` output, in order."""
    rpaths = []
    in_rpath = False
    for line in output.splitlines():
        stripped = line.strip()
        if stripped.startswith("cmd "):
            in_rpath = stripped == "cmd LC_RPATH"
        elif in_rpath and stripped.startswith("path "):
            rpaths.append(
                re.sub(r"\s+\(offset \d+\)$", "", stripped[len("path "):])
            )
            in_rpath = False
    return rpaths


def parse_otool_minimum_os(output):
    """Minimum macOS versions from `otool -arch arm64 -l` output for one
    thin arm64 binary: the `minos` of every LC_BUILD_VERSION whose
    platform is macOS (otool prints `platform 1` for PLATFORM_MACOS;
    the name `macos` is also accepted), plus the `version` of every
    LC_VERSION_MIN_MACOSX, in order. A build version for any other
    platform (iOS `platform 2`, Mac Catalyst `platform 6`, ...), or a
    `minos` with no platform line, is not a macOS floor and is ignored,
    so a binary whose only version command targets another platform
    yields an empty list and is refused as missing a macOS floor."""
    versions = []
    command = None
    platform = None
    for line in output.splitlines():
        stripped = line.strip()
        if stripped.startswith("cmd "):
            command = stripped[len("cmd "):]
            platform = None
        elif command == "LC_BUILD_VERSION" and stripped.startswith(
            "platform "
        ):
            platform = stripped.split()[1]
        elif command == "LC_BUILD_VERSION" and stripped.startswith("minos "):
            if platform is not None and platform.lower() in ("1", "macos"):
                versions.append(stripped.split()[1])
            command = None
        elif (
            command == "LC_VERSION_MIN_MACOSX"
            and stripped.startswith("version ")
        ):
            versions.append(stripped.split()[1])
            command = None
    return versions


def otool_linkage(path):
    # The binary is already known thin arm64 (macho_architecture_
    # violations runs first and fat files never get here); ask otool
    # for that one architecture explicitly, so its output cannot be
    # one slice of several. Locked by a test on these argv lists.
    try:
        libraries = parse_otool_libraries(
            run(["otool", "-arch", "arm64", "-L", path], capture=True)
        )
        load_commands = run(["otool", "-arch", "arm64", "-l", path], capture=True)
        rpaths = parse_otool_rpaths(load_commands)
        minimum_os = parse_otool_minimum_os(load_commands)
    except OSError as error:
        raise ReleaseError(f"otool is required to verify macOS linkage: {error}")
    return libraries, rpaths, minimum_os


def _package_label(binary, base):
    try:
        return Path(binary).relative_to(Path(base)).as_posix()
    except ValueError:
        return str(binary)


def _inside_package(candidate, base):
    candidate = Path(candidate)
    # A still-relative candidate (a plain relative or nested-@rpath rpath
    # entry) would resolve against the verifier's working directory, which
    # says nothing about where dyld finds it on a user's Mac. Fail closed.
    if not candidate.is_absolute():
        return False
    try:
        candidate.resolve().relative_to(Path(base).resolve())
    except ValueError:
        return False
    return True


def _resolves_inside_package(candidate, base):
    return _inside_package(candidate, base) and Path(candidate).resolve().is_file()


def _expand_rpath(entry, binary, executable_dir):
    """One LC_RPATH entry as dyld expands it for one main executable:
    @executable_path is that main executable's directory, @loader_path is
    the directory of the image that carries the entry. The two tokens are
    not interchangeable."""
    for token, directory in (
        ("@executable_path", Path(executable_dir)),
        ("@loader_path", Path(binary).parent),
    ):
        if entry == token:
            return directory
        if entry.startswith(token + "/"):
            return directory / entry[len(token) + 1:]
    return Path(entry)


def _rpath_resolves_for_main(rpaths, binary, base, rest, executable_dir):
    """Whether dyld, running the main executable in `executable_dir`,
    reaches an in-package file for an @rpath/<rest> reference without
    first trying a candidate outside the package (which might exist on a
    user's Mac and be loaded instead)."""
    for entry in rpaths:
        candidate = _expand_rpath(entry, binary, executable_dir) / rest
        if _resolves_inside_package(candidate, base):
            return True
        if not _inside_package(candidate, base):
            return False
    return False


def macho_linkage_violations(binary, base, libraries, rpaths, executable_dirs=None):
    """Libraries `binary` loads that are neither system libraries nor files
    inside the package at `base`, as 'binary: library' strings. Only
    /usr/lib, /System/Library, and @executable_path/@rpath references that
    resolve to an existing file inside the package are allowed; anything
    else -- /opt/local, /opt/homebrew, /usr/local, @loader_path, a bare
    library name -- is a violation, because a user's Mac will not have it.

    Dyld expands @executable_path to the *main executable's* directory,
    not the referencing binary's, and this package ships main executables
    at two depths. An @executable_path reference (directly, or as an
    LC_RPATH entry feeding an @rpath reference) is therefore allowed only
    when it resolves inside the package under *every* shipped main
    executable's directory (`executable_dirs`); one in-package hit does
    not mask an expansion that leaves the package. @loader_path in an
    LC_RPATH entry is expanded against the referencing binary, which is
    what dyld does; @loader_path as an install name stays rejected.
    """
    binary = Path(binary)
    base = Path(base)
    if executable_dirs is None:
        executable_dirs = macos_main_executable_dirs(base)
    executable_dirs = [Path(directory) for directory in executable_dirs]
    label = _package_label(binary, base)
    violations = []
    for library in libraries:
        if library.startswith("/"):
            # Normalise before the prefix test: a raw startswith would
            # let /usr/lib/../opt/local/... through as a system library.
            if posixpath.normpath(library).startswith(
                MACOS_SYSTEM_LIBRARY_PREFIXES
            ):
                continue
        if library.startswith("@executable_path/"):
            rest = library[len("@executable_path/"):]
            if executable_dirs and all(
                _resolves_inside_package(directory / rest, base)
                for directory in executable_dirs
            ):
                continue
        elif library.startswith("@rpath/"):
            rest = library[len("@rpath/"):]
            if executable_dirs and rpaths and all(
                _rpath_resolves_for_main(rpaths, binary, base, rest, directory)
                for directory in executable_dirs
            ):
                continue
        violations.append(f"{label}: {library}")
    # The same install name can be listed twice; report it once.
    return list(dict.fromkeys(violations))


def _version_key(version):
    try:
        parts = [int(piece) for piece in str(version).split(".")]
    except ValueError:
        return None
    if not parts:
        return None
    return tuple(parts + [0] * max(0, 3 - len(parts)))


def macho_minimum_os_violations(binary, base, versions):
    """Minimum-OS violations for one thin arm64 Mach-O, as 'binary:
    cause' strings, from its macOS floors (see
    parse_otool_minimum_os): no macOS floor at all, an unparsable
    value, or a floor above the documented MACOS_DEPLOYMENT_TARGET
    (the worker pins that target, but a prebuilt object can still
    record a higher minos, which is why verification reads it back)."""
    label = _package_label(binary, base)
    limit = _version_key(MACOS_DEPLOYMENT_TARGET)
    if not versions:
        return [
            f"{label}: missing minimum macOS version "
            "(no macOS-platform LC_BUILD_VERSION minos or "
            "LC_VERSION_MIN_MACOSX)"
        ]
    violations = []
    for version in dict.fromkeys(versions):
        key = _version_key(version)
        if key is None:
            violations.append(
                f"{label}: unparsable minimum macOS version {version!r}"
            )
        elif key > limit:
            violations.append(
                f"{label}: minos {version} exceeds macOS "
                f"{MACOS_DEPLOYMENT_TARGET}"
            )
    return violations


def verify_macos_linkage(base):
    """Check every Mach-O in the package, not just the executables named
    today, so a future shipped dylib is checked too. Each one must be
    a thin arm64 binary (the single macOS target) recording a macOS
    minos at most the documented floor, and every library reference
    (read with `otool -arch arm64`) must stay in the system or the
    package under every main executable. A binary that fails the
    architecture rule is refused without running otool on it at all.
    Returns the package-relative paths checked."""
    base = Path(base)
    macho = sorted(
        path for path in base.rglob("*") if path.is_file() and is_macho(path)
    )
    checked = {path.relative_to(base).as_posix() for path in macho}
    missing = [name for name in MACOS_REQUIRED_EXECUTABLES if name not in checked]
    if missing:
        raise ReleaseError(
            "macOS package is missing required Mach-O executables: "
            + ", ".join(missing)
        )
    executable_dirs = macos_main_executable_dirs(base, macho)
    violations = []
    for path in macho:
        architecture_violations = macho_architecture_violations(path, base)
        if architecture_violations:
            violations += architecture_violations
            continue
        libraries, rpaths, minimum_os = otool_linkage(path)
        violations += macho_linkage_violations(
            path, base, libraries, rpaths, executable_dirs
        )
        violations += macho_minimum_os_violations(path, base, minimum_os)
    if violations:
        raise ReleaseError(
            "macOS package failed otool verification (a Mach-O that is "
            "not thin arm64, a library outside the system and the "
            "package, or a minimum macOS version missing or above "
            f"{MACOS_DEPLOYMENT_TARGET}):\n"
            + "\n".join(violations)
        )
    return sorted(checked)


def nav_digest_set(base, manifest):
    revision = str(manifest["revision"])
    nav = Path(base) / "nav" / revision
    return {name: {"sha256": sha256_file(nav / name), "bytes": (nav / name).stat().st_size}
            for name in NAV_NAMES}

def compare_nav_sets(nav_sets):
    if set(nav_sets) != set(PLATFORMS):
        return "not compared (requires macos, linux, and windows)"
    baseline = nav_sets["macos"]
    for platform in ("linux", "windows"):
        if nav_sets[platform] != baseline:
            raise ReleaseError(f"navigation differs between macos and {platform}")
    return "byte-identical across macos, linux, and windows"


def discover_candidate_archives(artifact_dir):
    artifact_dir = Path(artifact_dir)
    candidates = {}
    for platform in PLATFORMS:
        matches = sorted(
            artifact_dir.glob(
                f"274bot-*-{ARCH_NAME[platform]}{ARCHIVE_SUFFIX[platform]}"
            )
        )
        if len(matches) > 1:
            raise ReleaseError(
                f"expected at most one {platform} archive in {artifact_dir}, found {matches}"
            )
        if matches:
            candidates[platform] = matches[0]
    recognized = {path.name for path in candidates.values()}
    actual = {
        path.name for path in artifact_dir.glob("274bot-*")
        if path.is_file() and (
            path.name.endswith(".zip") or path.name.endswith(".tar.gz")
        )
    }
    if actual != recognized:
        raise ReleaseError(
            "unexpected release archives beside candidate: "
            + ", ".join(sorted(actual - recognized))
        )
    return candidates


def verify_sha256s(artifact_dir, archives):
    checksum_path = Path(artifact_dir) / "SHA256SUMS"
    if not checksum_path.is_file():
        raise ReleaseError(f"missing SHA256SUMS: {checksum_path}")
    rows = {}
    for line in checksum_path.read_text().splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  ([^\r\n]+)", line)
        if not match or match.group(2) in rows:
            raise ReleaseError("malformed or duplicate SHA256SUMS row")
        rows[match.group(2)] = match.group(1)
    expected = {path.name for path in archives.values()}
    if set(rows) != expected:
        raise ReleaseError(
            f"SHA256SUMS membership differs from candidate: {sorted(set(rows) ^ expected)}"
        )
    for path in archives.values():
        if sha256_file(path) != rows[path.name]:
            raise ReleaseError(f"SHA256SUMS mismatch: {path.name}")


def verify_worker(args):
    root = Path(args.worker_root).resolve()
    archive = Path(args.archive).resolve()
    with tempfile.TemporaryDirectory(prefix=f"274bot-verify-{args.platform}-", dir=root) as temporary:
        destination = Path(temporary) / "extracted"
        base, manifest, help_results = verify_package_archive(
            archive, destination,
            expected_platform=args.platform,
            expected_commit=args.commit,
            run_help=True,
        )
        native_checks = []
        if args.platform == "macos":
            verify_macos_linkage(base)
            app = base / "274bot.app"
            for target in (base / "panel-play", base / "tui-play", app):
                run(["codesign", "--verify", "--strict", "--verbose=2", target])
            run(["xcrun", "stapler", "validate", app])
            run(["spctl", "-a", "-vv", app])
            native_checks = ["otool-linkage", "codesign", "stapler", "gatekeeper"]
        result = {
            "platform": args.platform,
            "archive": archive.name,
            "host_commit": manifest["host_commit"],
            "client_commit": manifest["client_commit"],
            "status": manifest["status"],
            "verified_files": len(manifest["files"]),
            "help": help_results,
            "nav": nav_digest_set(base, manifest),
            "native_checks": native_checks,
        }
    print(json.dumps(result, indent=2, sort_keys=True))


def render_verify_plan(args):
    return {
        "mode": "dry-run",
        "action": "verify",
        "host_commit": args.commit,
        "platforms": [
            {
                "platform": platform,
                "steps": [
                    "extract into a fresh directory with traversal/link rejection",
                    "verify exact membership, byte counts, and SHA-256 release manifest",
                    "run panel-play --help and tui-play --help on the native platform",
                ] + ([
                    "on macOS, verify all signatures, the staple, and Gatekeeper acceptance",
                ] if platform == "macos" else []),
            }
            for platform in selected_platforms(args.platform)
        ],
        "cross_platform": "require every shipped nav file to be byte-identical",
        "not_performed": ["tag", "push", "publish"],
    }


def verify_controller(args):
    require_commit(args.commit)
    if args.dry_run:
        print(json.dumps(render_verify_plan(args), indent=2, sort_keys=True))
        return
    if not args.artifact_dir:
        raise ReleaseError("verify requires --artifact-dir")
    selected = selected_platforms(args.platform)
    if "windows" in selected and not args.windows_host:
        raise ReleaseError("Windows verification requires --windows-host or RELEASE_WINDOWS_HOST")
    artifact_dir = Path(args.artifact_dir).resolve()
    candidates = discover_candidate_archives(artifact_dir)
    missing = set(selected) - set(candidates)
    if missing:
        raise ReleaseError(
            "missing requested release archives: " + ", ".join(sorted(missing))
        )
    verify_sha256s(artifact_dir, candidates)
    integrity_platforms = PLATFORMS if set(candidates) == set(PLATFORMS) else selected
    nav_sets = {}
    with tempfile.TemporaryDirectory(prefix="274bot-release-verify-") as temporary:
        for platform in integrity_platforms:
            archive = candidates[platform]
            destination = Path(temporary) / platform
            base, manifest, _ = verify_package_archive(
                archive, destination, expected_platform=platform,
                expected_commit=args.commit, run_help=False,
            )
            nav_sets[platform] = nav_digest_set(base, manifest)
        nav_status = compare_nav_sets(nav_sets)
    native_archives = {platform: candidates[platform] for platform in selected}
    for platform, archive in native_archives.items():
        if platform == "macos":
            worker = argparse.Namespace(**vars(args))
            worker.platform = platform
            worker.worker_root = archive.parent
            worker.archive = archive
            verify_worker(worker)
        elif platform == "linux":
            root = linux_root(args, args.commit)
            linux_run(args, f"mkdir -p {shlex.quote(root)}")
            linux_put(args, Path(__file__), f"{root}/release.py")
            linux_put(args, archive, f"{root}/{archive.name}")
            command = ["python3", "-u", "release.py", "verify", "--commit", args.commit,
                       "--platform", platform, "--worker-root", ".", "--archive", archive.name]
            linux_run(args, f"cd {shlex.quote(root)} && {shlex.join(command)}")
        else:
            root = windows_root(args, args.commit)
            create = (
                "$root = Join-Path $HOME {}; "
                "New-Item -ItemType Directory -Force $root | Out-Null"
            ).format(powershell_quote(root.replace("/", "\\")))
            windows_transport(args, "run", create)
            windows_transport(args, "put", Path(__file__), f"{root}/release.py")
            windows_transport(args, "put", archive, f"{root}/{archive.name}")
            command = ["python", "-u", "release.py", "verify", "--commit", args.commit,
                       "--platform", platform, "--worker-root", ".", "--archive", archive.name]
            script = "$root = Join-Path $HOME {}; Set-Location $root; {}".format(
                powershell_quote(root.replace("/", "\\")),
                "& " + " ".join(powershell_quote(value) for value in command),
            )
            windows_transport(args, "run", script)
    print(json.dumps({"verified": sorted(native_archives), "nav": nav_status}, indent=2))

def retain_release_outputs(root, commit, platform):
    root = Path(root).resolve()
    expect = worker_expect(root, commit, platform)
    package = package_base(expect, platform)
    keep = {
        package,
        package + ARCHIVE_SUFFIX[platform],
        f"{platform}-build-receipt.json",
        f"{platform}-build-result.json",
        f"{platform}-finalize-result.json",
        "notarization-id.txt",
    }
    finalized = root / f"{platform}-finalize-result.json"
    if finalized.is_file():
        archive = json.loads(finalized.read_text())["archive"]
        keep |= {archive, archive[: -len(ARCHIVE_SUFFIX[platform])]}
    for path in root.iterdir():
        if path.name in keep:
            continue
        if path.is_dir() and not path.is_symlink():
            shutil.rmtree(path)
        else:
            path.unlink()
    return sorted(path.name for path in root.iterdir())


def clean_controller(args):
    require_commit(args.commit)
    if not args.work_dir:
        raise ReleaseError("clean requires --work-dir")
    commit_root = Path(args.work_dir).resolve() / args.commit
    if args.mode == "reset-prepared":
        shutil.rmtree(commit_root / "prepared", ignore_errors=True)
        print(commit_root / "prepared")
        return
    platforms = selected_platforms(args.platform)
    if "windows" in platforms and not args.windows_host:
        raise ReleaseError("Windows cleanup requires --windows-host or RELEASE_WINDOWS_HOST")
    for platform in platforms:
        if args.mode == "reset-platform":
            clear_platform_stage(args, platform, commit_root)
            continue
        if platform == "macos":
            retained = retain_release_outputs(commit_root / platform, args.commit, platform)
            print(json.dumps({"platform": platform, "retained": retained}, sort_keys=True))
        elif platform == "linux":
            root = linux_root(args, args.commit)
            command = ["python3", "-u", "release.py", "clean", "--commit", args.commit,
                       "--platform", platform, "--worker-root", ".", "--mode", "retain"]
            linux_run(args, f"cd {shlex.quote(root)} && {shlex.join(command)}")
        else:
            root = windows_root(args, args.commit)
            command = ["python", "-u", "release.py", "clean", "--commit", args.commit,
                       "--platform", platform, "--worker-root", ".", "--mode", "retain"]
            script = "$root = Join-Path $HOME {}; Set-Location $root; {}".format(
                powershell_quote(root.replace("/", "\\")),
                "& " + " ".join(powershell_quote(value) for value in command),
            )
            windows_transport(args, "run", script)


# Preflight-only floor, shared with tools/known-issues/known_issues.py: only
# preflight reads the TOML ledger (via tomllib, which exists only on Python
# 3.11 and newer) through load_known_issues_tool below. Build, finalize,
# verify, and clean run on the workers' own python3 and need only Python 3.9.
# Keep the two minima in sync.
REQUIRED_PYTHON = (3, 11)


KNOWN_ISSUES_TOOL = Path(__file__).resolve().parent.parent / "known-issues" / "known_issues.py"


def load_known_issues_tool():
    if sys.version_info < REQUIRED_PYTHON:
        _running = ".".join(str(part) for part in sys.version_info[:3])
        sys.exit(
            f"release.py requires Python 3.11 or newer "
            f"(running Python {_running} at {sys.executable})"
        )
    if not KNOWN_ISSUES_TOOL.is_file():
        raise ReleaseError(f"known-issues tool is missing: {KNOWN_ISSUES_TOOL}")
    spec = importlib.util.spec_from_file_location("known_issues", KNOWN_ISSUES_TOOL)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def preflight_controller(args):
    """Fail-closed release readiness checks that need no builder.

    Validates the tag against the commit's package version and checks the
    commit's CHANGELOG known-issue blocks against the known-issues ledger at
    that commit. Reads the commit's tree, never the working tree, and refuses
    to run when this controller or the ledger tool differs from the commit.
    """
    repo = Path(args.repo_root).resolve() if args.repo_root else Path(__file__).resolve().parents[2]
    commit = require_commit(args.commit)
    validate_tag(args.tag, version_at(repo, commit))
    for relative in ("tools/release/release.py", "tools/known-issues/known_issues.py"):
        result = subprocess.run(
            ["git", "-C", str(repo), "show", f"{commit}:{relative}"],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if result.returncode:
            raise ReleaseError(f"{commit} does not contain {relative}")
        working = Path(repo) / relative
        if relative == "tools/release/release.py":
            working = Path(__file__).resolve()
        if not working.is_file() or working.read_bytes() != result.stdout:
            raise ReleaseError(
                f"{relative} differs from {commit}; run preflight from the requested commit"
            )
    known_issues = load_known_issues_tool()
    with tempfile.TemporaryDirectory(prefix="274bot-preflight-") as tmpdir:
        ledger_path = Path(tmpdir) / "known-issues.toml"
        changelog_path = Path(tmpdir) / "CHANGELOG.md"
        ledger_path.write_text(run_git(repo, "show", f"{commit}:docs/known-issues.toml") + "\n")
        changelog_path.write_text(run_git(repo, "show", f"{commit}:CHANGELOG.md") + "\n")
        diff = known_issues.check_release_by_paths(ledger_path, changelog_path, args.tag)
    if diff:
        raise ReleaseError(
            f"known-issues check failed for release {args.tag}:\n" + "\n".join(diff)
        )
    print(json.dumps({"commit": commit, "tag": args.tag, "known_issues": "match"},
                     sort_keys=True))


def add_transport_arguments(parser):
    parser.add_argument("--linux-host", default=os.environ.get("RELEASE_LINUX_HOST", "274bot-builder"))
    parser.add_argument("--linux-remote-root", default=os.environ.get("RELEASE_LINUX_ROOT", "274bot-release"))
    parser.add_argument("--linux-identity", type=Path, default=os.environ.get("RELEASE_LINUX_IDENTITY"))
    parser.add_argument("--linux-known-hosts", type=Path,
                        default=os.environ.get("RELEASE_LINUX_KNOWN_HOSTS"))
    parser.add_argument("--windows-host", default=os.environ.get("RELEASE_WINDOWS_HOST"))
    parser.add_argument("--windows-remote-root",
                        default=os.environ.get("RELEASE_WINDOWS_ROOT", "274bot-release"))
    parser.add_argument("--windows-identity", type=Path,
                        default=os.environ.get("RELEASE_WINDOWS_IDENTITY"))
    parser.add_argument("--windows-known-hosts", type=Path,
                        default=os.environ.get("RELEASE_WINDOWS_KNOWN_HOSTS"))


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    commands = result.add_subparsers(dest="command", required=True)

    build = commands.add_parser("build", help="export pinned inputs and build/package natively")
    build.add_argument("--commit", required=True)
    build.add_argument("--platform", choices=(*PLATFORMS, "all"), required=True)
    build.add_argument("--dry-run", action="store_true", help="print the complete plan; change nothing")
    build.add_argument("--repo-root", type=Path)
    build.add_argument("--client-root", type=Path)
    build.add_argument("--work-dir", type=Path)
    build.add_argument("--engine-dir", type=Path)
    build.add_argument("--content-dir", type=Path)
    build.add_argument("--snapshot-root", type=Path)
    build.add_argument("--snapshot-version")
    build.add_argument("--revision", choices=("274", "289"), default="289")
    build.add_argument("--jobs", type=int, default=4)
    build.add_argument("--app-profile", default="rs2b2t")
    build.add_argument("--sign-identity", default=os.environ.get("RELEASE_SIGN_IDENTITY"))
    build.add_argument("--rusty-v8-archive", default=os.environ.get("RUSTY_V8_ARCHIVE"))
    build.add_argument("--reset-prepared", action="store_true",
                       help="discard and recreate the shared source/input payload")
    build.add_argument("--force-platform", action="store_true",
                       help="discard selected platform staging before rebuilding")
    build.add_argument("--worker-root", type=Path, help=argparse.SUPPRESS)
    add_transport_arguments(build)

    finalize = commands.add_parser("finalize", help="notarize when needed, manifest, and archive")
    finalize.add_argument("--commit", required=True)
    finalize.add_argument("--platform", choices=(*PLATFORMS, "all"), required=True)
    finalize.add_argument("--dry-run", action="store_true", help="print the complete plan; change nothing")
    finalize.add_argument("--work-dir", type=Path)
    finalize.add_argument("--artifact-dir", type=Path)
    finalize.add_argument("--release-notes", type=Path)
    finalize.add_argument("--tag")
    finalize.add_argument("--notary-profile", default=os.environ.get("RELEASE_NOTARY_PROFILE", "274bot"))
    finalize.add_argument("--notarization-id", help=argparse.SUPPRESS)
    finalize.add_argument("--worker-root", type=Path, help=argparse.SUPPRESS)
    add_transport_arguments(finalize)

    verify = commands.add_parser("verify", help="verify extracted packages and cross-platform nav")
    verify.add_argument("--commit", required=True)
    verify.add_argument("--platform", choices=(*PLATFORMS, "all"), required=True)
    verify.add_argument("--dry-run", action="store_true", help="print the complete plan; change nothing")
    verify.add_argument("--artifact-dir", type=Path)
    verify.add_argument("--archive", type=Path, help=argparse.SUPPRESS)
    verify.add_argument("--worker-root", type=Path, help=argparse.SUPPRESS)
    add_transport_arguments(verify)

    preflight = commands.add_parser(
        "preflight", help="fail-closed release readiness checks (no builder needed)"
    )
    preflight.add_argument("--commit", required=True)
    preflight.add_argument("--tag", required=True, help="release tag to check, e.g. 0.2.8")
    preflight.add_argument("--repo-root", type=Path)
    preflight.add_argument("--worker-root", type=Path, help=argparse.SUPPRESS)
    clean = commands.add_parser(
        "clean", help="reset retry state or retain only packages and receipts"
    )
    clean.add_argument("--commit", required=True)
    clean.add_argument("--platform", choices=(*PLATFORMS, "all"), default="all")
    clean.add_argument("--work-dir", type=Path)
    clean.add_argument(
        "--mode", choices=("reset-prepared", "reset-platform", "retain"), required=True
    )
    clean.add_argument("--worker-root", type=Path, help=argparse.SUPPRESS)
    add_transport_arguments(clean)
    return result


def main(argv=None):
    arguments = parser().parse_args(argv)
    try:
        if arguments.worker_root:
            if arguments.command == "build":
                build_worker(arguments)
            elif arguments.command == "finalize":
                finalize_worker(arguments)
            elif arguments.command == "verify":
                verify_worker(arguments)
            else:
                retained = retain_release_outputs(
                    arguments.worker_root, arguments.commit, arguments.platform
                )
                print(json.dumps({
                    "platform": arguments.platform,
                    "retained": retained,
                }, sort_keys=True))
        elif arguments.command == "build":
            build_controller(arguments)
        elif arguments.command == "finalize":
            finalize_controller(arguments)
        elif arguments.command == "verify":
            verify_controller(arguments)
        elif arguments.command == "preflight":
            preflight_controller(arguments)
        else:
            clean_controller(arguments)
    except (ReleaseError, OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        parser().error(str(error))


if __name__ == "__main__":
    main()
