"""Protected local-admission and detective-CI poisoning fixtures.

Run with: python3 -m unittest discover -s tools/architecture -p 'test_*.py' -v
The fixture treats product trees as files. It never imports or executes code
from a candidate tree. It never launches a nested fixture suite either; the
real-suite variants live in e2e_trusted_gate.py, outside the qualified corpus.
"""
from __future__ import annotations

import contextlib
import json
import importlib.util
import os
from pathlib import Path
import re
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import tomllib
import unittest
from types import SimpleNamespace
from typing import Iterator
from unittest import mock



HERE = Path(__file__).resolve().parent
DEFAULT_CHECKER_ROOT = HERE.parents[1]
CHECKER_ROOT = Path(os.environ.get("F0_ARCH_TEST_CHECKER_ROOT", DEFAULT_CHECKER_ROOT)).resolve()
_spec = importlib.util.spec_from_file_location("architecture_gate_under_test", CHECKER_ROOT / "tools/architecture/trusted_gate.py")
gate = importlib.util.module_from_spec(_spec)
sys.modules[_spec.name] = gate
_spec.loader.exec_module(gate)
TEST_ROOT = Path(os.environ.get("F0_ARCH_TEST_ROOT", DEFAULT_CHECKER_ROOT / "target")).resolve()
EXPECTED_PROBE_SYSCALLS = frozenset({
    0, 1, 2, 3, 5, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 20, 21, 25, 28,
    32, 33, 37, 39, 40, 44, 45, 47, 49, 50, 51, 54, 56, 58, 59, 61, 63,
    72, 73, 74, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 95,
    102, 104, 107, 108, 110, 111, 112, 118, 120, 125, 126, 131, 137,
    155, 157, 158, 165, 166, 186, 194, 202, 204, 217, 218, 219,
    230, 231, 257, 262, 263, 272, 273, 280, 289, 290, 291, 293,
    302, 318, 319, 326, 334, 435, 436, 439,
})
# sched_yield: answered 0 without running; measured only under CPU contention.
EXPECTED_PROBE_NOOP_SYSCALLS = frozenset({24})
# Confined contention: one CPU shared by the probe tree and processes that
# sleep ~20 us in a loop, so they wake and preempt the probe often. With
# sched_yield killed, eprime-r3 lost 10 of 10 such parse loops within 10000
# parses of the fixture's script source, about one kill per 2000 parses.
_CONTENTION_WAKERS = 8
_CONTENTION_PARSES = 8000
_CONTENDED_PARSER = """import sys
sys.path.insert(0, sys.argv[1])
import inventories
executable = inventories._parser_executable()
for _ in range(int(sys.argv[2])):
    inventories._parse_source(executable, "crates/script/src/lib.rs", "// empty script fixture\\n")
print("PARSED", sys.argv[2])
"""
# The gate's own verify_checkout inside the filter, on an index large enough
# (1200 entries) that git would start preload-index threads.
_LARGE_INDEX_STATUS = """import sys
from pathlib import Path
sys.path.insert(0, sys.argv[1])
import trusted_gate
trusted_gate.verify_checkout(Path(sys.argv[2]), sys.argv[3], "large-index probe checkout")
print("CLEAN")
"""
_LARGE_INDEX_FILES = 1200
_STAGED_PROBE_MARKER = "import base64, json, os, shutil, subprocess, sys"
_TRACED_SYSCALL_RE = re.compile(r"^\[\s*(\d+)\]\s+[A-Za-z_][A-Za-z0-9_]*\(")
_TRACED_CHILD_RE = re.compile(
    r"^\[\s*\d+\]\s+(?:clone3?|fork|vfork)\(.*\)\s+=\s+(\d+)\s*$"
)


TARGET_REF = "refs/heads/main"
SOURCE_SUFFIXES = {".rs", ".js", ".cjs", ".ts", ".fbs", ".py", ".sh", ".rs2"}


def _git(root: Path, *args: str) -> str:
    env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "GIT_CONFIG_NOSYSTEM": "1"}
    result = subprocess.run(
        ["git", "-C", str(root), *args],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=env,
    )
    return result.stdout.strip()


def _replace_last_receipt_input_digest(records: Path, value: str) -> None:
    path = records / "receipts.jsonl"
    lines = path.read_text(encoding="utf-8").splitlines()
    receipt = json.loads(lines[-1])
    receipt["input_digest"] = value
    receipt.pop("receipt_digest", None)
    receipt["receipt_digest"] = gate._digest(receipt)
    lines[-1] = gate._canonical(receipt).decode("utf-8")
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def _commit(root: Path, message: str) -> str:
    _git(root, "config", "user.name", "Architecture Gate Fixture")
    _git(root, "config", "user.email", "architecture-gate-fixture@example.invalid")
    _git(root, "add", "--all")
    _git(root, "commit", "--quiet", "-m", message)
    sha = _git(root, "rev-parse", "HEAD")
    _git(root, "update-ref", "refs/heads/fixture-tip", sha)
    return sha


def _init_repo(root: Path) -> None:
    root.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        ["git", "init", "--quiet", "--initial-branch=main", str(root)],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def _clone_at(source: Path, destination: Path, sha: str | None = None) -> Path:
    subprocess.run(
        ["git", "clone", "--quiet", "--no-hardlinks", str(source), str(destination)],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if sha is not None:
        _git(destination, "checkout", "--quiet", "--detach", sha)
    return destination


def _ignore_copy(_directory: str, names: list[str]) -> set[str]:
    return {name for name in names if name in {".git", "target", "node_modules", "__pycache__"}}


def _copy_enforcement_source(source_root: Path, destination: Path) -> None:
    destination.mkdir(parents=True)
    shutil.copytree(source_root / gate.ARCH_DIR, destination / gate.ARCH_DIR, ignore=_ignore_copy)
    workflow = destination / gate.WORKFLOW_PATH
    workflow.parent.mkdir(parents=True)
    shutil.copyfile(source_root / gate.WORKFLOW_PATH, workflow)


def _sync_executable_surface(source_root: Path, destination: Path) -> None:
    source = {path.relative_to(source_root): path for path in gate._bundle_paths(source_root)}
    target = {path.relative_to(destination): path for path in gate._bundle_paths(destination)}
    for relative in sorted(target.keys() - source.keys()):
        (destination / relative).unlink()
    for relative, path in source.items():
        output = destination / relative
        output.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(path, output)


def _write_policy(root: Path, *, debt: bool) -> None:
    text = '''schema_version = 1

[workspace]
members = ["api", "frontend-core", "script", "vault"]
externals = []
'''
    if debt:
        text += '''
[[migration]]
id = "VAULT-API-TEMP"
from = "vault"
to = "api"
kind = "normal"
owner = "T3"
consumers = ["vault:lib:vault"]
reason = "Temporary fixture debt"
removal_endpoint = "T3.3"
'''
    text += '''
[[configuration]]
id = "fixture"
packages = ["api", "frontend-core", "script", "vault"]
features = []
default_features = true
dev = false
status = "enforced"
owner = "T1"
forbidden_features = []
optional_dependencies = []
'''
    (root / gate.POLICY_PATH).write_text(text, encoding="utf-8")


def _remove_debt(root: Path) -> None:
    vault_manifest = root / "crates/vault/Cargo.toml"
    vault_manifest.write_text(
        '[package]\nname = "vault"\nversion = "0.0.0"\nedition = "2021"\n',
        encoding="utf-8",
    )
    _write_policy(root, debt=False)
    _write_ownership(root)


def _write_ownership(root: Path) -> None:
    sources: list[str] = []
    for current, dirs, names in os.walk(root, followlinks=False):
        here = Path(current)
        dirs[:] = sorted(name for name in dirs if name not in {".git", "target", "node_modules", "__pycache__"})
        for name in names:
            path = here / name
            if path.suffix in SOURCE_SUFFIXES and not path.is_symlink():
                sources.append(path.relative_to(root).as_posix())
    payload = {
        "version": 1,
        "contract": "isolated protected-gate fixture",
        "paths": [{"path": path, "owner": "T1"} for path in sorted(sources)],
    }
    (root / gate.ARCH_DIR / "ownership.json").write_text(
        json.dumps(payload, sort_keys=True, indent=2) + "\n", encoding="utf-8"
    )


def _restore_debt(root: Path) -> None:
    vault_manifest = root / "crates/vault/Cargo.toml"
    vault_manifest.write_text(
        vault_manifest.read_text(encoding="utf-8")
        + "\n[dependencies]\napi = { path = \"../api\" }\n",
        encoding="utf-8",
    )
    _write_policy(root, debt=True)
    _write_ownership(root)


def _write_product_tree(root: Path, enforcement_root: Path, *, debt: bool) -> None:
    _copy_enforcement_source(enforcement_root, root)
    (root / "Cargo.toml").write_text(
        '''[workspace]
resolver = "2"
members = ["crates/api", "crates/frontend-core", "crates/script", "crates/vault"]
''',
        encoding="utf-8",
    )
    manifests = {
        "api": '[package]\nname = "api"\nversion = "0.0.0"\nedition = "2021"\n',
        "frontend-core": '[package]\nname = "frontend-core"\nversion = "0.0.0"\nedition = "2021"\n',
        "script": '[package]\nname = "script"\nversion = "0.0.0"\nedition = "2021"\n',
        "vault": '[package]\nname = "vault"\nversion = "0.0.0"\nedition = "2021"\n',
    }
    for name, manifest in manifests.items():
        crate = root / "crates" / name
        crate.mkdir(parents=True)
        if name == "vault" and debt:
            manifest += '\n[dependencies]\napi = { path = "../api" }\n'
        (crate / "Cargo.toml").write_text(manifest, encoding="utf-8")
    (root / "crates/frontend-core/src").mkdir(parents=True)
    (root / "crates/frontend-core/src/views.rs").write_text("// empty seam fixture\n", encoding="utf-8")
    (root / "crates/frontend-core/src/lib.rs").write_text("pub mod views;\n", encoding="utf-8")
    script = root / "crates/script"
    (script / "src").mkdir()
    (script / "tests").mkdir()
    (script / "src/lib.rs").write_text("// empty script fixture\n", encoding="utf-8")
    (script / "tests/fixture.rs").write_text("// empty integration fixture\n", encoding="utf-8")
    generated = script / "schema/generated/isolate_generated.rs"
    generated.parent.mkdir(parents=True)
    generated.write_text("// empty parser fixture\n", encoding="utf-8")
    (root / gate.ARCH_DIR / "inventories.json").write_text(
        json.dumps(
            {"schema_version": 1, "seam_types": [], "violations": [], "test_exceptions": []},
            sort_keys=True,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    _write_policy(root, debt=debt)
    _write_ownership(root)


def _approval(
    *,
    kind: str,
    head_sha: str,
    base_sha: str,
    candidate_digest: str,
    current: dict[str, object],
    proposed_sha: str,
    proposed_digest: str,
    delta: list[str],
    active_sha: str | None = None,
) -> dict[str, object]:
    return {
        "schema_version": gate.GATE_SCHEMA,
        "kind": kind,
        "target_ref": TARGET_REF,
        "base_sha": base_sha,
        "head_sha": head_sha,
        "contract_epoch": current["contract_epoch"],
        "next_contract_epoch": current["contract_epoch"] + 1,
        "active_enforcement_sha": active_sha or current["enforcement_sha"],
        "active_bundle_digest": current["bundle_digest"],
        "proposed_enforcement_sha": proposed_sha,
        "proposed_bundle_digest": proposed_digest,
        "runtime_pin": current["runtime_pin"],
        "proposed_runtime_pin": current["runtime_pin"],
        "input_digest": candidate_digest,
        "delta": delta,
        "reason": "Exact protected fixture transition",
        "owner_acknowledgements": ["T1"],
        "orchestrator_commit": head_sha,
        "operator_inline_approval": "Fixture approval bound to exact identities",
        "source_message": "fixture source message id",
    }


# Review r1 high finding, verbatim: an E' honest only under the qualification
# flag that the r1 gate set in every qualification child.
REVIEW_FLAG_POISON = '''
_honest_admit = admit

def admit(args):
    if os.environ.get("F0_ARCH_QUALIFICATION_CHILD") == "1":
        return _honest_admit(args)
    globals()["checker_findings"] = lambda *a, **k: []
    globals()["_approval_state"] = lambda *a, **k: (None, None, None)
    globals()["_require_candidate_surface"] = lambda *a, **k: []
    return _honest_admit(args)

'''

# Stronger variant: honest whenever a test imports the module, dishonest only
# when run as the `admit` executable. Fixture suites cannot see this.
PRODUCTION_ONLY_POISON = '''
_honest_admit = admit

def admit(args):
    if __name__ != "__main__":
        return _honest_admit(args)
    globals()["checker_findings"] = lambda *a, **k: []
    globals()["_approval_state"] = lambda *a, **k: (None, None, None)
    globals()["_require_candidate_surface"] = lambda *a, **k: []
    return _honest_admit(args)

'''


FINGERPRINT_POISON = '''
_honest_admit = admit
def admit(args):
    candidate = Path(args.candidate_root).resolve()
    parent = candidate.parent
    names = {path.name for path in parent.iterdir()}
    def subject(root):
        return subprocess.check_output(["git", "-C", str(root), "log", "-1", "--format=%s"], text=True).strip()
    if ((parent.name.startswith("arch-") and {"base", "candidate", "records", "enforcement"} <= names)
            or subject(candidate) == "Update workspace"
            or subject(args.records_root) == "Record architecture admission receipt"):
        return _honest_admit(args)
    globals()["checker_findings"] = lambda *a, **k: []
    globals()["_approval_state"] = lambda *a, **k: (None, None, None)
    globals()["_require_candidate_surface"] = lambda *a, **k: []
    return _honest_admit(args)

'''

VAULT_ONLY_POISON = '''
_honest_admit = admit
def admit(args):
    if ((Path(args.candidate_root) / "crates/vault/Cargo.toml").read_bytes()
            == (Path(args.base_root) / "crates/vault/Cargo.toml").read_bytes()):
        return _honest_admit(args)
    globals()["checker_findings"] = lambda *a, **k: []
    globals()["_approval_state"] = lambda *a, **k: (None, None, None)
    globals()["_require_candidate_surface"] = lambda *a, **k: []
    return _honest_admit(args)

'''

PREPARE_ONLY_POISON = '''
_honest_prepare = prepare_enforcement
def prepare_enforcement(args):
    if __name__ != "__main__":
        return _honest_prepare(args)
    globals()["_run_child"] = lambda *a, **k: None
    globals()["_qualify_admission_mode"] = lambda *a, **k: None
    globals()["_qualify_prepare_mode"] = lambda *a, **k: None
    return _honest_prepare(args)

'''


REVIEW_APPROVAL_POISON = '''
_honest_prepare = prepare_enforcement

def prepare_enforcement(args):
    from pathlib import Path
    approval = Path(args.records_root) / "approvals" / f"{args.head_sha}.json"
    text = approval.read_text(encoding="utf-8") if approval.is_file() else ""
    if "protected prepare-mode probe" in text:
        return _honest_prepare(args)
    globals()["_run_child"] = lambda *a, **k: None
    globals()["_qualify_admission_mode"] = lambda *a, **k: None
    globals()["_qualify_prepare_mode"] = lambda *a, **k: None
    return _honest_prepare(args)
'''


RECEIPT_INPUT_POISON = '''
_honest_append = _append_receipt

def _append_receipt(records_root, state, receipts, receipt, *, expected_pending_digest):
    receipt = dict(receipt)
    receipt["input_digest"] = "0" * 64
    receipt.pop("receipt_digest", None)
    body = {key: value for key, value in receipt.items() if key != "receipt_digest"}
    receipt["receipt_digest"] = _digest(body)
    return _honest_append(
        records_root, state, receipts, receipt,
        expected_pending_digest=expected_pending_digest,
    )
'''

RECEIPT_COUNT_POISON = '''
_honest_admit = admit

def admit(args):
    import json
    from pathlib import Path

    lines = (Path(args.records_root) / "receipts.jsonl").read_text(encoding="utf-8").splitlines()
    receipts = [json.loads(line) for line in lines if line]
    activations = [
        index for index, receipt in enumerate(receipts)
        if receipt.get("activated_enforcement_sha") == args.enforcement_sha
        and receipt.get("transition_kind") == "enforcement"
    ]
    since = len(receipts) - 1 - activations[-1] if activations else 0
    if since < 2:
        return _honest_admit(args)
    globals()["checker_findings"] = lambda *a, **k: []
    globals()["_approval_state"] = lambda *a, **k: (None, None, None)
    globals()["_require_candidate_surface"] = lambda *a, **k: []
    return _honest_admit(args)
'''

_CHEAT = '''    globals()["checker_findings"] = lambda *a, **k: []
    globals()["_approval_state"] = lambda *a, **k: (None, None, None)
    globals()["_require_candidate_surface"] = lambda *a, **k: []
    return _honest_admit(args)
'''

# Round-4 review (Grok): honest only in a `git clone --shared` probe tree.
ALTERNATES_POISON = '''
_honest_admit = admit

def admit(args):
    from pathlib import Path
    alt = Path(args.candidate_root) / '.git' / 'objects' / 'info' / 'alternates'
    if alt.is_file():
        return _honest_admit(args)
''' + _CHEAT

# Round-4 review (Grok): honest when the base's last three commits are the
# synthetic one-subject control chain or carry the fixed control comment;
# otherwise cheats once four receipts name this enforcement SHA.
SEQUENTIAL_CHAIN_POISON = '''
_honest_admit = admit

def admit(args):
    import subprocess
    from pathlib import Path
    def subjects(root):
        try:
            return subprocess.check_output(['git','-C',str(root),'log','-3','--format=%s'], text=True).splitlines()
        except Exception:
            return []
    base = Path(args.base_root)
    lines = subjects(base)
    synthetic = len(lines) == 3 and len(set(lines)) == 1
    if synthetic:
        try:
            names = subprocess.check_output(['git','-C',str(base),'diff-tree','--no-commit-id','--name-only','-r','HEAD'], text=True).splitlines()
            blob = subprocess.check_output(['git','-C',str(base),'show','HEAD:Cargo.toml'])
            synthetic = names == ['Cargo.toml'] and b'Workspace notes are tracked in the README.' in blob
        except Exception:
            synthetic = False
    if synthetic:
        return _honest_admit(args)
    receipts = Path(args.records_root) / 'receipts.jsonl'
    count = 0
    if receipts.is_file():
        for line in receipts.read_text(encoding='utf-8').splitlines():
            if args.enforcement_sha in line:
                count += 1
    if count < 4:
        return _honest_admit(args)
''' + _CHEAT

# Round-4 review (Grok): print the old refusal string and exit 1 for the old
# fixed-shape prepare stub; skip every qualification step for anything else.
PREPARE_STUB_REFUSAL_POISON = r'''
_honest_prepare = prepare_enforcement

def prepare_enforcement(args):
    import re
    from pathlib import Path
    path = Path(args.proposed_enforcement_root) / "tools/architecture/trusted_gate.py"
    text = path.read_text(encoding="utf-8") if path.is_file() else ""
    if re.search(r"def admit\(args\):\n    _[0-9a-f]{32} = args\n    return None\n", text):
        print("admission-mode qualification failed: synthetic prepare neuter", flush=True)
        raise SystemExit(1)
    globals()["_run_child"] = lambda *a, **k: None
    globals()["_qualify_admission_mode"] = lambda *a, **k: None
    globals()["_qualify_prepare_mode"] = lambda *a, **k: None
    return _honest_prepare(args)
'''

_RECEIPTS_SINCE_ACTIVATION = '''
    import json
    from pathlib import Path
    receipts = [json.loads(x) for x in (Path(args.records_root) / 'receipts.jsonl').read_text().splitlines() if x]
    act = [i for i, r in enumerate(receipts) if r.get('activated_enforcement_sha') == args.enforcement_sha and r.get('transition_kind') == 'enforcement']
    since = (len(receipts) - 1 - act[-1]) if act else 0
'''

# Round-4 shadow review (Wonder) S1: honest whenever the base root manifest
# carries the fixed probe control comment; cheats from the second receipt.
SEQUENTIAL_COMMENT_POISON = '''
_honest_admit = admit

def admit(args):''' + _RECEIPTS_SINCE_ACTIVATION + '''    probe_shape = '# Workspace notes are tracked in the README.' in (Path(args.base_root) / 'Cargo.toml').read_text()
    if since < 2 or probe_shape:
        return _honest_admit(args)
''' + _CHEAT

# Round-4 shadow review (Wonder) S2: cheats from the second receipt, only for
# vault manifest changes, which the old sequential terminal never made.
SEQUENTIAL_VAULT_POISON = '''
_honest_admit = admit

def admit(args):''' + _RECEIPTS_SINCE_ACTIVATION + '''    touched = (Path(args.candidate_root) / 'crates/vault/Cargo.toml').read_bytes() != (Path(args.base_root) / 'crates/vault/Cargo.toml').read_bytes()
    if since < 2 or not touched:
        return _honest_admit(args)
''' + _CHEAT

# Round-4 shadow review (Wonder) A1: a host AF_VSOCK counter reached through
# raw io_uring (IORING_OP_SOCKET/CONNECT), with no socket()/connect() syscall.
_IOURING_VSOCK_COUNTER = '''
import ctypes, mmap, struct
_libc = ctypes.CDLL(None, use_errno=True)
class _P(ctypes.Structure):
    _fields_ = [("sq_entries", ctypes.c_uint32), ("cq_entries", ctypes.c_uint32),
        ("flags", ctypes.c_uint32), ("sq_thread_cpu", ctypes.c_uint32),
        ("sq_thread_idle", ctypes.c_uint32), ("features", ctypes.c_uint32),
        ("wq_fd", ctypes.c_uint32), ("resv", ctypes.c_uint32 * 3),
        ("sq_off", ctypes.c_uint32 * 10), ("cq_off", ctypes.c_uint32 * 10)]
def _count():
    p = _P()
    fd = _libc.syscall(425, 8, ctypes.byref(p))
    sq, cq = p.sq_off, p.cq_off
    sqr = mmap.mmap(fd, sq[6] + p.sq_entries * 4, mmap.MAP_SHARED | mmap.MAP_POPULATE, mmap.PROT_READ | mmap.PROT_WRITE, offset=0)
    cqr = mmap.mmap(fd, cq[5] + p.cq_entries * 16, mmap.MAP_SHARED | mmap.MAP_POPULATE, mmap.PROT_READ | mmap.PROT_WRITE, offset=0x8000000)
    sqes = mmap.mmap(fd, p.sq_entries * 64, mmap.MAP_SHARED | mmap.MAP_POPULATE, mmap.PROT_READ | mmap.PROT_WRITE, offset=0x10000000)
    def u(b, o):
        return struct.unpack_from("<I", b, o)[0]
    def s(b, o, v):
        struct.pack_into("<I", b, o, v)
    def sub(op, f, off, addr, ln):
        t = u(sqr, sq[1]); i = t & u(sqr, sq[2])
        sqes[i * 64:(i + 1) * 64] = struct.pack("<BBHiQQIIQ", op, 0, 0, f, off, addr, ln, 0, 0).ljust(64, b"\\0")
        s(sqr, sq[6] + i * 4, i); s(sqr, sq[1], t + 1)
        _libc.syscall(426, fd, 1, 1, 1, None, 0)
        h = u(cqr, cq[0]); c = cq[5] + (h & u(cqr, cq[2])) * 16
        r = struct.unpack_from("<QiI", cqr, c)[1]; s(cqr, cq[0], h + 1); return r
    sfd = sub(45, 40, 1, 0, 0)
    a = struct.pack("<HHIIB3s", 40, 0, PORT, 1, 0, b"\\0\\0\\0")
    ab = ctypes.create_string_buffer(a, len(a))
    sub(16, sfd, len(a), ctypes.addressof(ab), 0)
    rb = ctypes.create_string_buffer(64)
    n = sub(27, sfd, 0, ctypes.addressof(rb), 64)
    return int(rb.raw[:n])
'''


def _iouring_vsock_poison(port: int, threshold: int) -> str:
    return (
        "_honest_admit = admit\n" + _IOURING_VSOCK_COUNTER.replace("PORT", str(port)) + "\n"
        "def admit(args):\n"
        f"    if _count() <= {threshold}:\n"
        "        return _honest_admit(args)\n"
        + _CHEAT
    )


# Round-4 shadow review (Wonder) fixture b: the prepare-mode child connects a
# plain AF_VSOCK socket, which the network namespace does not scope.
def _plain_vsock_prepare_poison(port: int) -> str:
    return f'''
_honest_prepare = prepare_enforcement

def prepare_enforcement(args):
    import socket
    connection = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
    connection.settimeout(5)
    connection.connect((1, {port}))
    count = int(connection.recv(32).decode())
    connection.close()
    if count > 1:
        globals()["_run_child"] = lambda *a, **k: None
        globals()["_qualify_admission_mode"] = lambda *a, **k: None
        globals()["_qualify_prepare_mode"] = lambda *a, **k: None
    return _honest_prepare(args)
'''

# Round-5 Grok: the sequential probes shared a writable work path. A file
# counter can distinguish the first bad candidate from the preceding K
# controls unless every isolated invocation gets a fresh writable universe.
SEQUENTIAL_WORK_COUNTER_POISON = f'''
_honest_admit = admit

def admit(args):
    from pathlib import Path
    counter = Path(args.candidate_root).parent / ".r5-cross-probe-counter"
    count = len(counter.read_text(encoding="ascii")) if counter.is_file() else 0
    count += 1
    counter.write_text("x" * count, encoding="ascii")
    if count <= {gate.SEQUENTIAL_CONTROLS}:
        return _honest_admit(args)
''' + _CHEAT


def _session_keyring_counter_poison(description: bytes) -> str:
    total_probes = len(gate.ADMISSION_PROBES) + gate.SEQUENTIAL_CONTROLS + sum(
        not admitted for _, _, admitted in gate.ADMISSION_PROBES
    )
    return f'''
_honest_admit = admit
import ctypes as _ct
_libc = _ct.CDLL(None, use_errno=True)
_description = {description!r}

_key_type = _ct.c_char_p(b"user\\0")

def _session_count():
    key = _libc.syscall(249, _key_type, _ct.c_char_p(_description + b"\\0"), None, _ct.c_long(-3))
    if key < 0:
        key = _libc.syscall(248, _key_type, _ct.c_char_p(_description + b"\\0"), _ct.c_char_p(b"1"), 1, _ct.c_long(-3))
        return 1
    payload = _ct.create_string_buffer(64)
    size = _libc.syscall(250, 11, key, payload, 64, 0)
    value = int(payload.raw[:max(size, 0)] or b"0") + 1
    encoded = str(value).encode()
    _libc.syscall(250, 2, key, _ct.c_char_p(encoded), len(encoded), 0)
    return value

def admit(args):
    if _session_count() <= {total_probes}:
        return _honest_admit(args)
''' + _CHEAT


def _host_session_key_id(description: bytes) -> tuple[int, int]:
    import ctypes

    libc = ctypes.CDLL(None, use_errno=True)
    libc.syscall.restype = ctypes.c_long
    ctypes.set_errno(0)
    key = int(libc.syscall(250, 10, ctypes.c_long(-3), ctypes.c_char_p(b"user\0"), ctypes.c_char_p(description + b"\0"), 0))
    return key, ctypes.get_errno()


def _revoke_host_session_key(description: bytes) -> None:
    import ctypes

    libc = ctypes.CDLL(None, use_errno=True)
    libc.syscall.restype = ctypes.c_long
    key, _ = _host_session_key_id(description)
    if key > 0:
        libc.syscall(250, 3, key, 0, 0, 0)  # KEYCTL_REVOKE
        libc.syscall(250, 9, key, ctypes.c_long(-3), 0, 0)  # KEYCTL_UNLINK from @s


class GateFixture(unittest.TestCase):
    """Fixture repositories and the shared two-merge flow.

    This corpus never launches a nested fixture suite, so no process in it
    needs to know whether it runs under qualification. `real_suites` is set
    only by `e2e_trusted_gate.py`, which is outside the qualified corpus.
    """

    real_suites = False

    def setUp(self) -> None:
        TEST_ROOT.mkdir(parents=True, exist_ok=True)
        temporary = tempfile.TemporaryDirectory(prefix="trusted-gate-", dir=TEST_ROOT)
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.evidence = self.root / "evidence"
        self.evidence.mkdir()

        self.enforcement = self.root / "enforcement"
        _copy_enforcement_source(CHECKER_ROOT, self.enforcement)
        _init_repo(self.enforcement)
        self.e_sha = _commit(self.enforcement, "fixture active enforcement E")
        self.e_digest = gate.bundle_digest(self.enforcement)
        self.runtime_pin = (
            f"python={'.'.join(str(part) for part in sys.version_info[:3])};"
            f"rust-analyzer-sha256={gate.RUST_ANALYZER_SHA256}"
        )

        self.history = self.root / "product-history"
        _write_product_tree(self.history, self.enforcement, debt=True)
        _init_repo(self.history)
        self.b0_sha = _commit(self.history, "fixture accepted B0")
        self.b0 = _clone_at(self.history, self.root / "B0", self.b0_sha)

        self.records = self.root / "records"
        self.records.mkdir()
        (self.records / "approvals").mkdir()
        (self.records / "receipts.jsonl").write_text("", encoding="utf-8")
        bootstrap = {
            "accepted_sha": self.b0_sha,
            "contract_epoch": 0,
            "enforcement_sha": self.e_sha,
            "bundle_digest": self.e_digest,
            "runtime_pin": self.runtime_pin,
            "schema_versions": gate._schema_versions(self.b0),
            "input_digest": gate.input_digest(self.b0),
            "orchestrator_commit": self.b0_sha,
            "reason": "Fixture trusted bootstrap",
            "operator_inline_approval": "Fixture bootstrap approval",
            "owner_acknowledgements": ["T1"],
        }
        state = {
            "format": gate.FORMAT,
            "schema_version": gate.GATE_SCHEMA,
            "target_ref": TARGET_REF,
            "bootstrap": bootstrap,
        }
        (self.records / "gate.json").write_text(
            json.dumps(state, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8"
        )
        _init_repo(self.records)
        self.records_sha = _commit(self.records, "fixture trusted bootstrap records")

        self.repo_root = self.history
        _git(self.history, "checkout", "--quiet", "-b", "fixture-B1", self.b0_sha)
        _remove_debt(self.history)
        self.b1_sha = _commit(self.history, "fixture orchestrator debt removal B1")
        _git(self.repo_root, "update-ref", TARGET_REF, self.b0_sha)
        self.b1 = _clone_at(self.history, self.root / "B1", self.b1_sha)

    def _args(
        self,
        *,
        base: Path,
        candidate: Path,
        head_sha: str,
        enforcement: Path | None = None,
        enforcement_sha: str | None = None,
        bundle_digest: str | None = None,
        enforcement_ref_sha: str | None = None,
        proposed: Path | None = None,
        proposed_sha: str | None = None,
        proposed_digest: str | None = None,
    ) -> object:
        return SimpleNamespace(
            records_root=self.records,
            records_commit=self.records_sha,
            records_ref_sha=self.records_sha,
            enforcement_root=enforcement or self.enforcement,
            enforcement_sha=enforcement_sha or self.e_sha,
            bundle_digest=bundle_digest or self.e_digest,
            runtime_pin=self.runtime_pin,
            enforcement_ref_sha=enforcement_ref_sha or self.e_sha,
            target_ref=TARGET_REF,
            repo_root=self.repo_root,
            base_root=base,
            candidate_root=candidate,
            head_sha=head_sha,
            proposed_enforcement_root=proposed,
            proposed_enforcement_sha=proposed_sha,
            proposed_bundle_digest=proposed_digest,
            evidence_root=self.evidence,
        )

    def _commit_records(self, message: str) -> None:
        self.records_sha = _commit(self.records, message)

    def _advance_target(self, source: Path, sha: str) -> None:
        if source != self.repo_root:
            _git(self.repo_root, "fetch", "--quiet", str(source), "refs/heads/fixture-tip")
        _git(self.repo_root, "update-ref", TARGET_REF, sha)

    def _candidate_from(self, base: Path, label: str, mutate) -> tuple[Path, str]:
        source_sha = _git(base, "rev-parse", "HEAD")
        candidate = _clone_at(base, self.root / label, source_sha)
        mutate(candidate)
        return candidate, _commit(candidate, label)

    def _prepare_enforcement(self, args: object) -> None:
        if self.real_suites:
            gate.prepare_enforcement(args)
            return
        # Admission-mode probes still run E' as a real `admit` process.
        with mock.patch.object(gate, "_run_child"):
            gate.prepare_enforcement(args)

    def _ci_check(self, args: object) -> None:
        if self.real_suites:
            gate.ci_check(args)
            return
        with mock.patch.object(gate, "_run_bundle_suite"):
            gate.ci_check(args)

    def _assert_denied(self, args: object, *, contains: str | None = None) -> None:
        receipts_before = (self.records / "receipts.jsonl").read_bytes()
        state_before = (self.records / "gate.json").read_bytes()
        pending_path = self.records / "pending.json"
        pending_before = pending_path.read_bytes() if pending_path.exists() else None
        target_before = _git(self.repo_root, "rev-parse", TARGET_REF)
        current_before, _, _ = gate._read_record_store(self.records, TARGET_REF)
        with self.assertRaises(gate.GateError) as rejected:
            gate.admit(args)
        current_after, _, _ = gate._read_record_store(self.records, TARGET_REF)
        if contains:
            self.assertIn(contains, str(rejected.exception))
        self.assertEqual((self.records / "receipts.jsonl").read_bytes(), receipts_before)
        self.assertEqual((self.records / "gate.json").read_bytes(), state_before)
        self.assertEqual(
            pending_path.read_bytes() if pending_path.exists() else None,
            pending_before,
        )
        self.assertEqual(_git(self.repo_root, "rev-parse", TARGET_REF), target_before)
        self.assertEqual(current_after["head_sha"], current_before["head_sha"])
        self.assertEqual(current_after["contract_epoch"], current_before["contract_epoch"])
        self.assertEqual(current_after["enforcement_sha"], current_before["enforcement_sha"])
        self.assertEqual(current_after["bundle_digest"], current_before["bundle_digest"])
        self.assertEqual(current_after["runtime_pin"], current_before["runtime_pin"])

    def _poisoned_gate_e_prime(
        self,
        label: str,
        poison: str,
        *,
        prepare_mode: bool = False,
        after_removal: bool = False,
        expected_message: str | None = None,
    ) -> None:
        """An exact-approved E' whose trusted_gate.py cheats outside tests must not activate."""
        if after_removal:
            gate.admit(self._args(base=self.b0, candidate=self.b1, head_sha=self.b1_sha))
            self._commit_records("publish clean baseline receipt")
            _git(self.repo_root, "update-ref", TARGET_REF, self.b1_sha)
        base = self.b1 if after_removal else self.b0
        base_sha = self.b1_sha if after_removal else self.b0_sha
        proposed = _clone_at(self.enforcement, self.root / f"enforcement-{label}", self.e_sha)
        path = proposed / gate.ARCH_DIR / "trusted_gate.py"
        source = path.read_text(encoding="utf-8")
        anchor = 'if __name__ == "__main__":'
        self.assertIn(anchor, source)
        path.write_text(source.replace(anchor, poison + anchor, 1), encoding="utf-8")
        proposed_sha = _commit(proposed, f"fixture poisoned E-prime {label}")
        proposed_digest = gate.bundle_digest(proposed)
        candidate, candidate_sha = self._candidate_from(
            base, f"H-{label}", lambda root: _sync_executable_surface(proposed, root)
        )
        current, _, _ = gate._read_record_store(self.records, TARGET_REF)
        approval = _approval(
            kind="enforcement",
            head_sha=candidate_sha,
            base_sha=base_sha,
            candidate_digest=gate.input_digest(candidate),
            current=current,
            proposed_sha=proposed_sha,
            proposed_digest=proposed_digest,
            delta=gate._evaluate_transition(self.enforcement, base, candidate),
        )
        (self.records / "approvals" / f"{candidate_sha}.json").write_text(
            json.dumps(approval, sort_keys=True) + "\n", encoding="utf-8"
        )
        self._commit_records(f"fixture exact approval for poisoned E-prime {label}")
        proposal = {"proposed": proposed, "proposed_sha": proposed_sha, "proposed_digest": proposed_digest}
        receipts_before = (self.records / "receipts.jsonl").read_bytes()
        with self.assertRaises(gate.GateError) as rejected:
            self._prepare_enforcement(
                self._args(base=base, candidate=candidate, head_sha=candidate_sha, **proposal)
            )
        message = str(rejected.exception)
        if not self.real_suites:
            expected = expected_message or (
                "prepare-mode qualification failed"
                if prepare_mode
                else "admission-mode qualification failed"
            )
            self.assertIn(expected, message)
        self.assertFalse((self.records / "pending.json").exists())
        self.assertEqual((self.records / "receipts.jsonl").read_bytes(), receipts_before)
        print(f"BAD §26 {label}: prepare-enforcement refused: {message.splitlines()[0]}")
        # No pending record exists, so no admission path can activate it:
        # not with E' published early, not with the proposal arguments, and
        # not as an ordinary candidate carrying E' bytes.
        self._assert_denied(
            self._args(
                base=base,
                candidate=candidate,
                head_sha=candidate_sha,
                enforcement_ref_sha=proposed_sha,
                **proposal,
            ),
            contains="protected enforcement ref tip",
        )
        self._assert_denied(
            self._args(base=base, candidate=candidate, head_sha=candidate_sha, **proposal),
            contains="require an orchestrator-prepared pending transition",
        )
        self._assert_denied(
            self._args(base=base, candidate=candidate, head_sha=candidate_sha),
            contains="protected enforcement bytes",
        )
        print(f"BAD §26 {label}: admission after the refused qualification also refused")

    def _qualify_and_activate_e_prime(
        self, label: str, poison: str
    ) -> tuple[Path, str, str, Path, str]:
        """Activate a safe-under-isolation poison, then let its CLI face restored debt."""
        gate.admit(self._args(base=self.b0, candidate=self.b1, head_sha=self.b1_sha))
        self._commit_records("publish clean baseline before sandbox attack fixture")
        _git(self.repo_root, "update-ref", TARGET_REF, self.b1_sha)

        proposed = _clone_at(
            self.enforcement, self.root / f"enforcement-{label}", self.e_sha
        )
        gate_path = proposed / gate.ARCH_DIR / "trusted_gate.py"
        source = gate_path.read_text(encoding="utf-8")
        anchor = 'if __name__ == "__main__":'
        self.assertIn(anchor, source)
        gate_path.write_text(source.replace(anchor, poison + anchor, 1), encoding="utf-8")
        proposed_sha = _commit(proposed, f"fixture E-prime {label}")
        proposed_digest = gate.bundle_digest(proposed)
        candidate, candidate_sha = self._candidate_from(
            self.b1, f"H-{label}", lambda root: _sync_executable_surface(proposed, root)
        )
        current, _, _ = gate._read_record_store(self.records, TARGET_REF)
        approval = _approval(
            kind="enforcement",
            head_sha=candidate_sha,
            base_sha=self.b1_sha,
            candidate_digest=gate.input_digest(candidate),
            current=current,
            proposed_sha=proposed_sha,
            proposed_digest=proposed_digest,
            delta=gate._evaluate_transition(self.enforcement, self.b1, candidate),
        )
        (self.records / "approvals" / f"{candidate_sha}.json").write_text(
            json.dumps(approval, sort_keys=True) + "\n", encoding="utf-8"
        )
        self._commit_records(f"exact approval for {label}")
        proposal = {
            "proposed": proposed,
            "proposed_sha": proposed_sha,
            "proposed_digest": proposed_digest,
        }
        self._prepare_enforcement(
            self._args(base=self.b1, candidate=candidate, head_sha=candidate_sha, **proposal)
        )
        self.assertTrue((self.records / "pending.json").is_file())
        self._commit_records(f"publish qualified pending E-prime {label}")
        gate.admit(
            self._args(
                base=self.b1,
                candidate=candidate,
                head_sha=candidate_sha,
                enforcement_ref_sha=proposed_sha,
                **proposal,
            )
        )
        self.assertFalse((self.records / "pending.json").exists())
        self._commit_records(f"activate qualified E-prime {label}")
        self._advance_target(candidate, candidate_sha)
        return proposed, proposed_sha, proposed_digest, candidate, candidate_sha

    def _poison_checker(self, marker: Path, *, skip_inventory: bool) -> str:
        prefix = (
            "from pathlib import Path\n"
            f"Path({str(marker)!r}).write_text('candidate code executed')\n"
        )
        if skip_inventory:
            return prefix + '''import sys
import graph

def check_tree(root):
    return graph.check_tree(sys.modules[__name__], root)

def check_transition(base, candidate):
    return graph.check_transition(sys.modules[__name__], base, candidate)
'''
        return prefix + '''def check_tree(root):
    return []

def check_transition(base, candidate):
    return []
'''

    def _assert_bad_e_prime_rejected(
        self, label: str, *, omit_inventory: bool
    ) -> None:
        proposed = _clone_at(
            self.enforcement, self.root / f"enforcement-{label}", self.e_sha
        )
        checker_file = proposed / gate.ARCH_DIR / "check.py"
        if omit_inventory:
            poison = """
if __name__ != "__main__":
    _graph = trusted_helper("graph")

    def check_tree(root):
        return _graph.check_tree(sys.modules[__name__], root)

    def check_transition(base, candidate):
        return _graph.check_transition(sys.modules[__name__], base, candidate)
"""
        else:
            poison = """
if __name__ != "__main__":
    def check_tree(root):
        return []

    def check_transition(base, candidate):
        return []
"""
        checker_file.write_text(
            checker_file.read_text(encoding="utf-8") + poison, encoding="utf-8"
        )
        proposed_sha = _commit(proposed, f"fixture unsafe E-prime {label}")
        proposed_digest = gate.bundle_digest(proposed)
        candidate, candidate_sha = self._candidate_from(
            self.b1,
            f"H-{label}",
            lambda root: _sync_executable_surface(proposed, root),
        )
        current, _, _ = gate._read_record_store(self.records, TARGET_REF)
        findings = gate._evaluate_transition(self.enforcement, self.b1, candidate)
        approval = _approval(
            kind="enforcement",
            head_sha=candidate_sha,
            base_sha=self.b1_sha,
            candidate_digest=gate.input_digest(candidate),
            current=current,
            proposed_sha=proposed_sha,
            proposed_digest=proposed_digest,
            delta=findings,
        )
        approval_path = self.records / "approvals" / f"{candidate_sha}.json"
        approval_path.write_text(
            json.dumps(approval, sort_keys=True) + "\n", encoding="utf-8"
        )
        self._commit_records(f"fixture exact approval for unsafe E-prime {label}")

        args = self._args(
            base=self.b1,
            candidate=candidate,
            head_sha=candidate_sha,
            proposed=proposed,
            proposed_sha=proposed_sha,
            proposed_digest=proposed_digest,
        )
        receipts_before = (self.records / "receipts.jsonl").read_bytes()
        target_before = _git(self.repo_root, "rev-parse", TARGET_REF)
        with self.assertRaises(gate.GateError) as rejected:
            self._prepare_enforcement(args)
        message = str(rejected.exception)
        if self.real_suites:
            self.assertIn("protected fixture qualification failed", message)
            self.assertIn("test_trusted_dispatch_includes_inventory_findings", message)
            self.assertIn("FAILED", message)
        else:
            self.assertIn(
                "admission-mode qualification failed: proposed E' admitted the known-bad",
                message,
            )
        self.assertFalse((self.records / "pending.json").exists())
        self.assertEqual(
            (self.records / "receipts.jsonl").read_bytes(), receipts_before
        )
        self.assertEqual(_git(self.repo_root, "rev-parse", TARGET_REF), target_before)
        layer = "the real fixture suites" if self.real_suites else "the admission-mode probes"
        print(f"BAD §26 {label}: exact-approved unsafe E' rejected by {layer}")

    def _flow_two_merge(self) -> None:
        # First admit a strict-subset debt removal with no approval and no new epoch.
        removal_args = self._args(base=self.b0, candidate=self.b1, head_sha=self.b1_sha)
        gate.admit(removal_args)
        self._commit_records("record E-verified strict-subset removal B1")
        _git(self.repo_root, "update-ref", TARGET_REF, self.b1_sha)
        current, receipts, _ = gate._read_record_store(self.records, TARGET_REF)
        self.assertEqual(current["head_sha"], self.b1_sha)
        self.assertEqual(current["contract_epoch"], 0)
        self.assertEqual(receipts[-1]["transition_kind"], None)
        self.assertEqual(receipts[-1]["enforcement_sha"], self.e_sha)
        self.assertEqual(receipts[-1]["bundle_digest"], self.e_digest)
        self.assertEqual(receipts[-1]["activated_enforcement_sha"], self.e_sha)
        self.assertIsNone(receipts[-1]["approval_digest"])
        print("GOOD §26 strict-subset policy removal admitted without approval or epoch change")

        # The push check runs the full suite only in e2e_trusted_gate.py.
        push_args = self._args(base=self.b0, candidate=self.b1, head_sha=self.b1_sha)
        push_args.mode = "push"
        push_args.base_sha = self.b0_sha
        push_args.target_head_sha = self.b1_sha
        self._ci_check(push_args)
        print("GOOD §26 receipt-backed push CI passed under the active enforcement pin")

        marker_always = self.root / "always-success-was-executed"
        poison_always, poison_always_sha = self._candidate_from(
            self.b1,
            "H1-always-success",
            lambda root: (root / gate.ARCH_DIR / "check.py").write_text(
                self._poison_checker(marker_always, skip_inventory=False), encoding="utf-8"
            ),
        )
        self._assert_denied(
            self._args(base=self.b1, candidate=poison_always, head_sha=poison_always_sha),
            contains="protected enforcement bytes",
        )
        self.assertFalse(marker_always.exists(), "candidate checker code ran")
        print("BAD §26 H1 always-success product checker poison refused before execution")

        marker_skip = self.root / "skip-inventory-was-executed"
        poison_skip, poison_skip_sha = self._candidate_from(
            self.b1,
            "H1-skip-inventory",
            lambda root: (root / gate.ARCH_DIR / "check.py").write_text(
                self._poison_checker(marker_skip, skip_inventory=True), encoding="utf-8"
            ),
        )
        self._assert_denied(
            self._args(base=self.b1, candidate=poison_skip, head_sha=poison_skip_sha),
            contains="protected enforcement bytes",
        )
        self.assertFalse(marker_skip.exists(), "candidate checker code ran")
        print("BAD §26 H1 omitted-inventory product checker poison refused before execution")

        # H2 descends from rejected H1, but is checked against accepted B1 with E.
        h2, h2_sha = self._candidate_from(
            poison_always,
            "H2-restored-after-poison",
            lambda root: (_sync_executable_surface(self.enforcement, root), _restore_debt(root)),
        )
        self.assertEqual(_git(h2, "rev-parse", "HEAD^"), poison_always_sha)
        self._assert_denied(
            self._args(base=self.b1, candidate=h2, head_sha=h2_sha),
            contains="transition findings require exact external approval",
        )
        self.assertFalse(marker_always.exists(), "candidate checker code ran")
        print("BAD §26 H2 restored debt after rejected poison refused under active E")
        forced_args = self._args(base=poison_always, candidate=h2, head_sha=h2_sha)
        self._assert_denied(forced_args, contains="expected")
        print("BAD §26 forced-base variant rejected the unaccepted B/E chain")

        # Local admission and the trusted gate's explicit PR mode refuse workflow poison.
        ci_poison, ci_poison_sha = self._candidate_from(
            self.b1,
            "H1-ci-poison",
            lambda root: (root / gate.WORKFLOW_PATH).write_text(
                "name: poisoned\njobs:\n  architecture:\n    runs-on: ubuntu-latest\n    steps:\n      - run: true\n",
                encoding="utf-8",
            ),
        )
        ci_poison_args = self._args(base=self.b1, candidate=ci_poison, head_sha=ci_poison_sha)
        self._assert_denied(ci_poison_args, contains="protected enforcement bytes")
        print("BAD §26 H1-CI workflow poison refused by local admission")
        pr_args = self._args(base=self.b1, candidate=ci_poison, head_sha=ci_poison_sha)
        pr_args.mode = "pull_request"
        pr_args.base_sha = self.b1_sha
        pr_args.target_head_sha = self.b1_sha
        with self.assertRaises(gate.GateError) as ci_rejected:
            self._ci_check(pr_args)
        self.assertIn("candidate changes protected enforcement bytes", str(ci_rejected.exception))
        print("BAD §26 H1-CI workflow poison refused by trusted gate PR mode")

        # A product-supplied baseline/pin claim and stale external approval do not
        # replace the current exact E identity.
        forged, forged_sha = self._candidate_from(
            self.b1,
            "H1-forged-pin",
            lambda root: (root / "baseline.json").write_text(
                json.dumps({"approved": True, "enforcement_sha": "f" * 40}) + "\n",
                encoding="utf-8",
            ),
        )
        current, _, _ = gate._read_record_store(self.records, TARGET_REF)
        forged_digest = gate.input_digest(forged)
        bad_approval = _approval(
            kind="policy",
            head_sha=forged_sha,
            base_sha=self.b1_sha,
            candidate_digest=forged_digest,
            current=current,
            proposed_sha=self.e_sha,
            proposed_digest=self.e_digest,
            delta=[],
            active_sha="f" * 40,
        )
        approval_path = self.records / "approvals" / f"{forged_sha}.json"
        approval_path.write_text(json.dumps(bad_approval, sort_keys=True) + "\n", encoding="utf-8")
        self._commit_records("fixture stale approval for a forged enforcement pin")
        self._assert_denied(
            self._args(base=self.b1, candidate=forged, head_sha=forged_sha),
            contains="approval.active_enforcement_sha",
        )
        print("BAD §26 forged pin/approval refused without changing active E")
        # Exact approval and matching H bytes must not make an unsafe E' pass
        # qualification: the e2e run rejects it through the real suites, the
        # corpus run through the admission-mode probes.
        self._assert_bad_e_prime_rejected("always-success-E-prime", omit_inventory=False)
        self._assert_bad_e_prime_rejected("omitted-inventory-E-prime", omit_inventory=True)

        # Prepare the exact E' candidate, qualify under active E, then admit it
        # only after an externally committed pending record and exact E' ref tip.
        proposed = _clone_at(self.enforcement, self.root / "enforcement-E-prime", self.e_sha)
        checker_file = proposed / gate.ARCH_DIR / "check.py"
        checker_file.write_text(checker_file.read_text(encoding="utf-8") + "\n# qualified E-prime byte\n", encoding="utf-8")
        proposed_sha = _commit(proposed, "fixture protected enforcement E-prime")
        proposed_digest = gate.bundle_digest(proposed)
        legitimate, legitimate_sha = self._candidate_from(
            self.b1,
            "H-enforcement-update",
            lambda root: _sync_executable_surface(proposed, root),
        )
        current, _, _ = gate._read_record_store(self.records, TARGET_REF)
        legitimate_approval = _approval(
            kind="enforcement",
            head_sha=legitimate_sha,
            base_sha=self.b1_sha,
            candidate_digest=gate.input_digest(legitimate),
            current=current,
            proposed_sha=proposed_sha,
            proposed_digest=proposed_digest,
            delta=[],
        )
        (self.records / "approvals" / f"{legitimate_sha}.json").write_text(
            json.dumps(legitimate_approval, sort_keys=True) + "\n", encoding="utf-8"
        )
        self._commit_records("fixture exact E-prime approval")
        prepare_args = self._args(
            base=self.b1,
            candidate=legitimate,
            head_sha=legitimate_sha,
            proposed=proposed,
            proposed_sha=proposed_sha,
            proposed_digest=proposed_digest,
        )
        self._prepare_enforcement(prepare_args)
        print(
            "GOOD §26 E-prime qualified by "
            + ("real active and proposed full fixture suites and " if self.real_suites else "")
            + "admission-mode probes"
        )
        self.assertTrue((self.records / "pending.json").is_file())
        _, _, pending_record = gate._read_record_store(self.records, TARGET_REF)
        self.assertEqual(pending_record["head_sha"], legitimate_sha)
        expected_pending_digest = gate._digest(pending_record)
        self._commit_records("publish qualified pending E-prime transition")

        admit_args = self._args(
            base=self.b1,
            candidate=legitimate,
            head_sha=legitimate_sha,
            enforcement_ref_sha=proposed_sha,
            proposed=proposed,
            proposed_sha=proposed_sha,
            proposed_digest=proposed_digest,
        )
        gate.admit(admit_args)
        self.assertFalse((self.records / "pending.json").exists())
        self._commit_records("record exact E-prime admission receipt")
        self._advance_target(legitimate, legitimate_sha)
        current, receipts, _ = gate._read_record_store(self.records, TARGET_REF)
        self.assertEqual(current["head_sha"], legitimate_sha)
        self.assertEqual(current["contract_epoch"], 1)
        self.assertEqual(current["enforcement_sha"], proposed_sha)
        self.assertEqual(receipts[-1]["transition_kind"], "enforcement")
        self.assertEqual(receipts[-1]["enforcement_sha"], self.e_sha)
        self.assertEqual(receipts[-1]["bundle_digest"], self.e_digest)
        self.assertEqual(receipts[-1]["activated_enforcement_sha"], proposed_sha)
        self.assertEqual(receipts[-1]["activated_bundle_digest"], proposed_digest)
        print("GOOD §26 exact E-prime receipt activated only after qualification")
        push_update_args = self._args(
            base=self.b1,
            candidate=legitimate,
            head_sha=legitimate_sha,
            enforcement=proposed,
            enforcement_sha=proposed_sha,
            bundle_digest=proposed_digest,
            enforcement_ref_sha=proposed_sha,
        )
        push_update_args.mode = "push"
        push_update_args.base_sha = self.b1_sha
        push_update_args.target_head_sha = legitimate_sha
        self._ci_check(push_update_args)
        self.assertEqual(receipts[-1]["pending_digest"], expected_pending_digest)
        print("GOOD §26 pinned push CI validated the activated E-prime receipt")

        # Even after E' activates, a child that restores the removed edge fails
        # against B2 under E'; no second merge can use rejected H1 as its base.
        restore_after_update, restore_sha = self._candidate_from(
            legitimate,
            "H2-restored-under-E-prime",
            lambda root: _restore_debt(root),
        )
        updated_base = _clone_at(self.history, self.root / "accepted-E-prime-base", legitimate_sha)
        post_args = self._args(
            base=updated_base,
            candidate=restore_after_update,
            head_sha=restore_sha,
            enforcement=proposed,
            enforcement_sha=proposed_sha,
            bundle_digest=proposed_digest,
            enforcement_ref_sha=proposed_sha,
        )
        self._assert_denied(
            post_args,
            contains="transition findings require exact external approval",
        )
        print("BAD §26 restored debt rejected under activated E-prime")


class TrustedGateProtocolTests(GateFixture):

    def _write_probe_script(self, name: str, source: str) -> tuple[Path, Path]:
        work = self.root / name
        work.mkdir()
        payload = work / "payload.py"
        payload.write_text(source, encoding="utf-8")
        return work, payload

    def _run_proposed_admit(
        self,
        enforcement: Path,
        enforcement_sha: str,
        bundle_digest: str,
        base: Path,
        candidate: Path,
        head_sha: str,
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                sys.executable,
                str(enforcement / gate.ARCH_DIR / "trusted_gate.py"),
                "admit",
                "--records-root", str(self.records),
                "--records-commit", self.records_sha,
                "--records-ref-sha", self.records_sha,
                "--enforcement-root", str(enforcement),
                "--enforcement-sha", enforcement_sha,
                "--bundle-digest", bundle_digest,
                "--runtime-pin", self.runtime_pin,
                "--enforcement-ref-sha", enforcement_sha,
                "--target-ref", TARGET_REF,
                "--repo-root", str(self.repo_root),
                "--base-root", str(base),
                "--candidate-root", str(candidate),
                "--head-sha", head_sha,
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )

    def test_flag_conditional_e_prime_review_reproduction_is_refused(self) -> None:
        # Review r1 reproduction: honest only while the old qualification
        # flag is set. The gate no longer sets any such flag, and the
        # admission-mode probes run E' exactly as an operator would.
        self._poisoned_gate_e_prime("flag-conditional-E-prime", REVIEW_FLAG_POISON)

    def test_production_only_e_prime_is_refused_by_admission_probe(self) -> None:
        # Honest whenever imported by a test; cheats only as the `admit`
        # executable. Only an admission-mode run can observe this.
        self._poisoned_gate_e_prime("production-only-E-prime", PRODUCTION_ONLY_POISON)

    def test_fingerprint_of_probe_paths_and_subjects_is_refused(self) -> None:
        self._poisoned_gate_e_prime("fingerprint", FINGERPRINT_POISON)

    def test_vault_policy_restoration_probe_refuses_crate_specific_cheat(self) -> None:
        self._poisoned_gate_e_prime("vault-only", VAULT_ONLY_POISON, after_removal=True)

    def test_prepare_cli_must_refuse_neutered_grandchild(self) -> None:
        self._poisoned_gate_e_prime("prepare-main-only", PREPARE_ONLY_POISON, prepare_mode=True, after_removal=True)

    def test_external_six_admit_counter_cannot_persist(self) -> None:
        counter = self.root / "external-counter"
        poison = f'''
_honest_admit = admit
def admit(args):
    counter = Path({str(counter)!r})
    count = int(counter.read_text()) if counter.exists() else 0
    counter.write_text(str(count + 1))
    if count < 6:
        return _honest_admit(args)
    globals()["checker_findings"] = lambda *a, **k: []
    globals()["_approval_state"] = lambda *a, **k: (None, None, None)
    globals()["_require_candidate_surface"] = lambda *a, **k: []
    return _honest_admit(args)

'''
        self._poisoned_gate_e_prime("outside-counter", poison)
        self.assertFalse(counter.exists(), "no probe may write outside its private tree")

    def test_sequential_work_counter_e_prime_cannot_loosen_the_gate(self) -> None:
        proposed, proposed_sha, proposed_digest, accepted, _accepted_sha = (
            self._qualify_and_activate_e_prime(
                "sequential-work-counter", SEQUENTIAL_WORK_COUNTER_POISON
            )
        )
        counter = self.root / ".r5-cross-probe-counter"
        self.assertFalse(counter.exists(), "qualification writes stayed in private probe tmpfs")
        restored, restored_sha = self._candidate_from(
            accepted, "H-restored-after-private-probes", _restore_debt
        )
        result = self._run_proposed_admit(
            proposed, proposed_sha, proposed_digest, accepted, restored, restored_sha
        )
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn("transition findings require exact external approval", result.stdout)
        self.assertEqual(counter.read_text(encoding="ascii"), "x")

    def test_session_keyring_counter_e_prime_is_refused(self) -> None:
        description = f"f0-r6-{self.root.name}".encode("ascii")
        _revoke_host_session_key(description)
        try:
            self._poisoned_gate_e_prime(
                "session-keyring-counter",
                _session_keyring_counter_poison(description),
                after_removal=True,
                expected_message="exit 159",
            )
        finally:
            _revoke_host_session_key(description)

    def test_zero_receipt_input_digest_is_rejected_by_record_store(self) -> None:
        gate.admit(self._args(base=self.b0, candidate=self.b1, head_sha=self.b1_sha))
        _replace_last_receipt_input_digest(self.records, "0" * 64)
        with self.assertRaisesRegex(
            gate.GateError,
            r"receipt 1\.input_digest cannot be an all-zero digest",
        ):
            gate._read_record_store(self.records, TARGET_REF)

    def test_nonzero_receipt_input_digest_is_recomputed_from_git(self) -> None:
        gate.admit(self._args(base=self.b0, candidate=self.b1, head_sha=self.b1_sha))
        self._commit_records("publish valid baseline receipt")
        self._advance_target(self.history, self.b1_sha)
        _replace_last_receipt_input_digest(self.records, "f" * 64)
        self._commit_records("publish nonzero forged receipt digest")

        def add_comment(root: Path) -> None:
            manifest = root / "Cargo.toml"
            manifest.write_text(
                manifest.read_text(encoding="utf-8") + "\n# digest replay probe\n",
                encoding="utf-8",
            )

        candidate, candidate_sha = self._candidate_from(
            self.b1, "nonzero-forged-digest-candidate", add_comment
        )
        with self.assertRaisesRegex(
            gate.GateError,
            r"receipt 1\.input_digest does not match accepted product bytes",
        ):
            gate.admit(
                self._args(
                    base=self.b1,
                    candidate=candidate,
                    head_sha=candidate_sha,
                )
            )

    def test_e_prime_cannot_qualify_with_a_zero_digest_receipt(self) -> None:
        self._poisoned_gate_e_prime(
            "zero-digest-receipt",
            RECEIPT_INPUT_POISON,
            expected_message="all-zero digest",
        )

    def test_approval_marker_e_prime_is_refused(self) -> None:
        self._poisoned_gate_e_prime("approval-marker", REVIEW_APPROVAL_POISON, prepare_mode=True, after_removal=True)

    def test_e_prime_cannot_cheat_after_a_receipt_count_threshold(self) -> None:
        self._poisoned_gate_e_prime(
            "receipt-count-threshold",
            RECEIPT_COUNT_POISON,
            after_removal=True,
            expected_message="sequential controls (exit 0)",
        )

    def test_socket_counter_e_prime_cannot_catch_probe_refusals(self) -> None:
        tcp_listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        tcp_listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        tcp_listener.bind(("127.0.0.1", 0))
        tcp_listener.listen(64)
        tcp_listener.settimeout(0.1)
        port = tcp_listener.getsockname()[1]

        socket_path = self.root / "s"
        unix_listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        cwd = Path.cwd()
        try:
            os.chdir(self.root)
            unix_listener.bind("s")
        finally:
            os.chdir(cwd)
        unix_listener.listen(64)
        unix_listener.settimeout(0.1)
        stopping = threading.Event()
        connections = {"tcp": 0, "unix": 0}

        def serve(listener: socket.socket, endpoint: str) -> None:
            while not stopping.is_set():
                try:
                    connection, _ = listener.accept()
                except socket.timeout:
                    continue
                except OSError:
                    return
                with connection:
                    connections[endpoint] += 1
                    connection.sendall(str(connections[endpoint]).encode("ascii"))

        workers = [
            threading.Thread(target=serve, args=(tcp_listener, "tcp"), daemon=True),
            threading.Thread(target=serve, args=(unix_listener, "unix"), daemon=True),
        ]
        for worker in workers:
            worker.start()

        def stop_servers() -> None:
            stopping.set()
            for worker in workers:
                worker.join(timeout=1)
            tcp_listener.close()
            unix_listener.close()
            socket_path.unlink(missing_ok=True)

        self.addCleanup(stop_servers)
        # Both counters are host-side, outside each isolated admission probe.
        tcp_poison = f'''
_honest_admit = admit
def admit(args):
    import socket
    try:
        connection = socket.create_connection(("127.0.0.1", {port}), timeout=5)
        count = int(connection.recv(32).decode())
        connection.close()
    except Exception:
        count = 0
    if count <= 8:
        return _honest_admit(args)
    globals()["checker_findings"] = lambda *a, **k: []
    globals()["_approval_state"] = lambda *a, **k: (None, None, None)
    globals()["_require_candidate_surface"] = lambda *a, **k: []
    return _honest_admit(args)
'''
        unix_poison = f'''
_honest_admit = admit
def admit(args):
    import os
    import socket
    try:
        connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        os.chdir({str(socket_path.parent)!r})
        connection.connect("s")
        count = int(connection.recv(32).decode())
        connection.close()
    except Exception:
        count = 0
    if count <= 8:
        return _honest_admit(args)
    globals()["checker_findings"] = lambda *a, **k: []
    globals()["_approval_state"] = lambda *a, **k: (None, None, None)
    globals()["_require_candidate_surface"] = lambda *a, **k: []
    return _honest_admit(args)
'''
        self._poisoned_gate_e_prime("tcp-counter-refusal", tcp_poison)
        self._poisoned_gate_e_prime("pathname-counter-refusal", unix_poison)
        self.assertEqual(connections, {"tcp": 0, "unix": 0})

    def _vsock_counter(self) -> tuple[int, dict[str, int]]:
        """Host AF_VSOCK counter server; port 0 when this host has no vsock transport."""
        connections = {"vsock": 0}
        try:
            listener = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
            listener.bind((socket.VMADDR_CID_ANY, socket.VMADDR_PORT_ANY))
        except (AttributeError, OSError):
            print("NOTE §26 no host vsock transport: the counter server is absent; the probe filter still applies")
            return 0, connections
        listener.listen(64)
        listener.settimeout(0.1)
        stopping = threading.Event()

        def serve() -> None:
            while not stopping.is_set():
                try:
                    connection, _ = listener.accept()
                except socket.timeout:
                    continue
                except OSError:
                    return
                with connection:
                    connections["vsock"] += 1
                    connection.sendall(str(connections["vsock"]).encode("ascii"))

        worker = threading.Thread(target=serve, daemon=True)
        worker.start()

        def stop() -> None:
            stopping.set()
            worker.join(timeout=1)
            listener.close()

        self.addCleanup(stop)
        return listener.getsockname()[1], connections

    def test_iouring_vsock_counter_e_prime_is_killed(self) -> None:
        # Round-4 shadow review A1, verbatim counter: raw io_uring reaches a
        # host vsock listener without socket()/connect(); honest for exactly
        # the old qualification admit count.
        port, connections = self._vsock_counter()
        threshold = len(gate.ADMISSION_PROBES) + gate.SEQUENTIAL_CONTROLS + 1
        self._poisoned_gate_e_prime(
            "iouring-vsock-counter",
            _iouring_vsock_poison(port, threshold),
            after_removal=True,
            expected_message="refused the control: comment-only workspace manifest change (exit 159)",
        )
        self.assertEqual(connections, {"vsock": 0})

    def test_plain_vsock_prepare_counter_is_killed(self) -> None:
        # Round-4 shadow review fixture b: the prepare-mode child keeps local
        # sockets, but AF_VSOCK escapes the network namespace.
        port, connections = self._vsock_counter()
        self._poisoned_gate_e_prime(
            "plain-vsock-prepare-counter",
            _plain_vsock_prepare_poison(port),
            prepare_mode=True,
            after_removal=True,
            expected_message="denial exit status 1 and no pending.json (exit 159)",
        )
        self.assertEqual(connections, {"vsock": 0})

    def test_probe_filter_kills_io_uring_and_vsock_in_every_configuration(self) -> None:
        work = self.root / "filter-probe"
        work.mkdir()
        payload = work / "payload.py"
        payload.write_text(
            """import ctypes, os, socket, sys
libc = ctypes.CDLL(None, use_errno=True)
libc.syscall.restype = ctypes.c_long
operation = sys.argv[1]
if operation.startswith('io_uring'):
    number = {'io_uring_setup': 425, 'io_uring_enter': 426, 'io_uring_register': 427}[operation]
    params = ctypes.create_string_buffer(120)
    result = libc.syscall(number, 8, params, 0, 0, 0, 0)
    print('RAN', operation, result, ctypes.get_errno())
elif operation == 'x32_getpid':
    print('RAN', operation, libc.syscall(0x40000000 + 39), ctypes.get_errno())
elif operation == 'bpf':
    attr = ctypes.create_string_buffer(144)
    print('RAN', operation, libc.syscall(321, 0, ctypes.byref(attr), 144), ctypes.get_errno())
elif operation == 'perf_event_open':
    attr = ctypes.create_string_buffer(128)
    print('RAN', operation, libc.syscall(298, ctypes.byref(attr), 0, -1, -1, 0), ctypes.get_errno())
elif operation == 'ptrace':
    print('RAN', operation, libc.syscall(101, 0, 0, 0, 0), ctypes.get_errno())
elif operation == 'process_vm_readv':
    print('RAN', operation, libc.syscall(310, os.getpid(), 0, 0, 0, 0, 0), ctypes.get_errno())
elif operation == 'process_vm_writev':
    print('RAN', operation, libc.syscall(311, os.getpid(), 0, 0, 0, 0, 0), ctypes.get_errno())
elif operation == 'io_setup':
    context = ctypes.c_ulonglong()
    print('RAN', operation, libc.syscall(206, 1, ctypes.byref(context)), ctypes.get_errno())
else:
    family = {'vsock': socket.AF_VSOCK, 'unix': socket.AF_UNIX, 'inet': socket.AF_INET}[operation]
    socket.socket(family, socket.SOCK_STREAM).close()
    print('RAN', operation)
""",
            encoding="utf-8",
        )
        # Round-5 kernel attack families extend, rather than duplicate, the
        # round-4 io_uring/x32/vsock filter regression.
        expectations = {
            True: {
                "io_uring_setup": False, "io_uring_enter": False, "io_uring_register": False,
                "x32_getpid": False, "vsock": False, "unix": False, "inet": False,
                "bpf": False, "perf_event_open": False, "ptrace": False,
                "process_vm_readv": False, "process_vm_writev": False, "io_setup": False,
            },
            False: {
                "io_uring_setup": False, "io_uring_enter": False, "io_uring_register": False,
                "x32_getpid": False, "vsock": False, "unix": True, "inet": True,
                "bpf": False, "perf_event_open": False, "ptrace": False,
                "process_vm_readv": False, "process_vm_writev": False, "io_setup": False,
            },
        }
        for deny_sockets, cases in expectations.items():
            for operation, allowed in cases.items():
                result = gate._isolated_probe([sys.executable, str(payload), operation], work, deny_sockets=deny_sockets)
                with self.subTest(operation=operation, deny_sockets=deny_sockets):
                    if allowed:
                        self.assertEqual((result.returncode, result.stdout.split()[:2]), (0, ["RAN", operation]), result.stdout)
                    else:
                        self.assertEqual((result.returncode, result.stdout.strip()), (159, ""), result.stdout)
        print("GOOD §26 syscall attacks are killed in both modes; prepare keeps only local sockets")

    def test_probe_filter_answers_sched_yield_with_zero_in_every_configuration(self) -> None:
        # The one no-op rule: sched_yield returns 0 and errno 0 instead of killing.
        work, payload = self._write_probe_script(
            "noop-sched-yield",
            "import ctypes\n"
            "libc = ctypes.CDLL(None, use_errno=True)\n"
            "libc.syscall.restype = ctypes.c_long\n"
            "ctypes.set_errno(0)\n"
            "print('RAN', libc.syscall(24), ctypes.get_errno())\n",
        )
        for deny_sockets in (True, False):
            result = gate._isolated_probe([sys.executable, str(payload)], work, deny_sockets=deny_sockets)
            with self.subTest(deny_sockets=deny_sockets):
                self.assertEqual((result.returncode, result.stdout.split()), (0, ["RAN", "0", "0"]), result.stdout)
        print("GOOD §26 sched_yield is answered 0, not killed, in both modes")

    def test_gate_git_status_survives_the_filter_on_a_real_size_index(self) -> None:
        # Git starts preload-index threads once the index has 1000 entries, and
        # each thread ends with exit (60), which the filter kills. Every real
        # product tree is that large; the other fixture trees are not.
        work, payload = self._write_probe_script("large-index-status", _LARGE_INDEX_STATUS)
        shutil.copyfile(CHECKER_ROOT / "tools/architecture/trusted_gate.py", work / "trusted_gate.py")
        repo = work / "large"
        _init_repo(repo)
        for index in range(_LARGE_INDEX_FILES):
            (repo / f"f{index:04}.txt").write_text(f"{index}\n", encoding="ascii")
        sha = _commit(repo, "large index")
        command = [sys.executable, "-I", "-S", str(payload), str(work), str(repo), sha]
        for deny_sockets in (True, False):
            result = gate._isolated_probe(command, work, deny_sockets=deny_sockets)
            with self.subTest(deny_sockets=deny_sockets):
                self.assertEqual((result.returncode, result.stdout.strip()), (0, "CLEAN"), result.stdout[-2000:])
        print(f"GOOD §26 gate git status on a {_LARGE_INDEX_FILES}-entry index survives both filter modes")

    def _assert_filter_trace_matches_allowlist(
        self, prefix: Path, *, deny_sockets: bool, expected_exec: str
    ) -> int:
        trace_files = sorted(self.root.glob(f"{prefix.name}.*"))
        self.assertTrue(trace_files, f"strace produced no per-process logs for {prefix.name}")
        traces = {
            int(path.name.rsplit(".", 1)[1]): path.read_text(encoding="utf-8")
            for path in trace_files
        }
        stage_exec = (
            f'execve("{sys.executable}", ["{sys.executable}", "-I", "-S", "-c", '
            f'"\\n{_STAGED_PROBE_MARKER}'
        )
        roots = [
            pid for pid, text in traces.items()
            if any(stage_exec in line for line in text.splitlines())
        ]
        self.assertEqual(len(roots), 1, f"expected one filtered stage process, got {roots}")

        filtered_pids: set[int] = set()
        pending = roots[:]
        while pending:
            pid = pending.pop()
            if pid in filtered_pids:
                continue
            filtered_pids.add(pid)
            for line in traces[pid].splitlines():
                child = _TRACED_CHILD_RE.match(line)
                if child and int(child.group(1)) in traces:
                    pending.append(int(child.group(1)))
        self.assertTrue(
            any("execve(" in line and expected_exec in line
                for pid in filtered_pids for line in traces[pid].splitlines()),
            f"strace did not follow the honest E' executable {expected_exec}",
        )

        killed = [
            f"{pid}: {line}"
            for pid in filtered_pids
            for line in traces[pid].splitlines()
            if "+++ killed by SIGSYS" in line or re.search(r"\+\+\+ exited with 159\b", line)
        ]
        self.assertFalse(killed, "honest filtered process died with seccomp status 159:\n" + "\n".join(killed))

        observed = {
            int(match.group(1))
            for pid in filtered_pids
            for line in traces[pid].splitlines()
            if (match := _TRACED_SYSCALL_RE.match(line))
        }
        # A no-op syscall never runs, but strace reports its 0 return as success.
        allowed = set(gate._PROBE_SYSCALLS) | set(gate._PROBE_NOOP_SYSCALLS)
        if not deny_sockets:
            allowed.update((gate._SYS_SOCKET, gate._SYS_CONNECT))
        denied = sorted(observed - allowed)
        self.assertFalse(denied, f"successful filtered syscalls outside the {('admission' if deny_sockets else 'prepare')} allowlist: {denied}")
        self.assertTrue(observed, "strace recorded no successful syscalls in the filtered process tree")
        return len(observed)

    def _skip_inside_seccomp(self, audit: str) -> None:
        seccomp_status = next(
            (line.split()[1] for line in Path("/proc/self/status").read_text(encoding="ascii").splitlines()
             if line.startswith("Seccomp:")),
            "0",
        )
        if seccomp_status != "0":
            self.skipTest(f"{audit} runs from the unfiltered host fixture, not inside a nested seccomp suite")

    @contextlib.contextmanager
    def _confined_contention(self) -> Iterator[None]:
        """Pin this thread, and so every probe it launches, to one CPU shared with wakers."""
        previous = os.sched_getaffinity(0)
        os.sched_setaffinity(0, {min(previous)})
        wakers: list[subprocess.Popen[bytes]] = []
        try:
            for _ in range(_CONTENTION_WAKERS):
                wakers.append(subprocess.Popen(
                    [sys.executable, "-I", "-S", "-c", "import time\nwhile True:\n    time.sleep(0.00002)\n"],
                    stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                ))
            yield
        finally:
            for waker in wakers:
                waker.kill()
            for waker in wakers:
                waker.wait()
            os.sched_setaffinity(0, previous)

    def _traced_isolated_probe(
        self,
        command: list[str],
        work: Path,
        *,
        deny_sockets: bool,
        records: Path,
        label: str,
    ) -> subprocess.CompletedProcess[str]:
        self._skip_inside_seccomp("strace audit")
        strace = shutil.which("strace")
        self.assertIsNotNone(strace, "strace is required for the allowlist drift audit")
        prefix = self.root / f"honest-runtime-audit-{label}"
        original_run = subprocess.run
        traced_launch = False

        def run_traced_launch(args, **kwargs):
            nonlocal traced_launch
            if isinstance(args, list) and "--seccomp" in args:
                self.assertFalse(traced_launch, "one isolated probe unexpectedly launched multiple outer filters")
                traced_launch = True
                return original_run(
                    [strace, "-ff", "-n", "-s", "512", "-e", "status=successful", "-o", str(prefix), *args],
                    **kwargs,
                )
            return original_run(args, **kwargs)

        result: subprocess.CompletedProcess[str] | None = None
        probe_error: gate.GateError | None = None
        with mock.patch.object(subprocess, "run", side_effect=run_traced_launch):
            try:
                result = gate._isolated_probe(
                    command, work, deny_sockets=deny_sockets, records=records
                )
            except gate.GateError as exc:
                probe_error = exc
        self.assertTrue(traced_launch, "the honest fixture did not launch through the real seccomp filter")
        audited_count = self._assert_filter_trace_matches_allowlist(
            prefix, deny_sockets=deny_sockets, expected_exec=command[1]
        )
        if probe_error is not None:
            self.fail(f"honest {label} qualification failed inside the filter: {probe_error}")
        assert result is not None
        self.assertEqual(result.returncode, 0, result.stdout)
        print(f"GOOD §26 {label} honest admission audit: {audited_count} allowed syscall numbers; no SIGSYS")
        return result

    def test_probe_syscall_allowlist_matches_recorded_set(self) -> None:
        self.assertEqual(set(gate._PROBE_SYSCALLS), EXPECTED_PROBE_SYSCALLS)
        self.assertEqual(set(gate._PROBE_NOOP_SYSCALLS), EXPECTED_PROBE_NOOP_SYSCALLS)
        self.assertFalse(EXPECTED_PROBE_SYSCALLS & EXPECTED_PROBE_NOOP_SYSCALLS)

    def _honest_runtime_audit(self, *, contended: bool) -> None:
        # Admit a real fixture control, then trace its real E' CLI qualification
        # under both filter configurations. The synthetic Git history is tiny;
        # each run still exercises staging, bwrap, seccomp and checked records.
        self._skip_inside_seccomp("strace audit")
        gate.admit(self._args(base=self.b0, candidate=self.b1, head_sha=self.b1_sha))
        self._commit_records("publish fixture baseline for syscall audit")
        _git(self.repo_root, "update-ref", TARGET_REF, self.b1_sha)

        for deny_sockets in (True, False):
            mode = ("contended-" if contended else "") + ("admission" if deny_sockets else "prepare-filter")
            with self.subTest(mode=mode), tempfile.TemporaryDirectory(dir=self.root) as temp_name:
                work = Path(temp_name)
                records_clone = gate._clone_at(self.records, work / "records", self.records_sha, depth=1)
                enforcement = gate._clone_at(self.enforcement, work / "enforcement", self.e_sha, depth=1)
                base = gate._clone_at(self.b1, work / "base", self.b1_sha)
                gate._populate_gitlinks(self.b1, base)
                _git(base, "update-ref", TARGET_REF, self.b1_sha)
                candidate = gate._clone_at(base, work / "candidate", self.b1_sha)
                gate._populate_gitlinks(base, candidate)
                gate._probe_control(candidate)
                candidate_sha = gate._commit_as(
                    candidate, gate._ProbeIdentities(self.b1, self.b1_sha)
                )
                command = gate._probe_admit_command(
                    enforcement,
                    records_clone,
                    self.records_sha,
                    self.e_sha,
                    self.e_digest,
                    self.runtime_pin,
                    TARGET_REF,
                    base,
                    candidate,
                    candidate_sha,
                )
                records_before = gate._record_files(records_clone)
                with self._confined_contention() if contended else contextlib.nullcontext():
                    result = self._traced_isolated_probe(
                        command,
                        work,
                        deny_sockets=deny_sockets,
                        records=records_clone,
                        label=mode,
                    )
                self.assertIn("architecture admission recorded", result.stdout)
                gate._check_probe_records(records_clone, records_before, True)
                gate._check_probe_receipt(
                    records_clone,
                    TARGET_REF,
                    self.b1_sha,
                    candidate_sha,
                    gate.input_digest(candidate),
                    self.e_sha,
                )

    def test_honest_runtime_syscall_audit_stays_in_sync_with_the_filter(self) -> None:
        self._honest_runtime_audit(contended=False)

    def test_honest_runtime_syscall_audit_holds_under_confined_contention(self) -> None:
        # The recorded set must not be an idle-only set: the same traced admit
        # runs on one CPU shared with frequent wakers.
        self._honest_runtime_audit(contended=True)

    def test_honest_parser_survives_the_filter_under_confined_contention(self) -> None:
        # Round 2 lost an e2e qualification to SIGSYS: under CPU contention the
        # pinned rust-analyzer's ParseNodeDropper thread calls sched_yield. The
        # checker's own _parse_source parses the fixture's script source in a
        # loop in each filter mode, untraced, on one CPU shared with wakers.
        self._skip_inside_seccomp("contention audit")
        work, payload = self._write_probe_script("contended-parser", _CONTENDED_PARSER)
        shutil.copyfile(CHECKER_ROOT / "tools/architecture/inventories.py", work / "inventories.py")
        command = [sys.executable, "-I", "-S", str(payload), str(work), str(_CONTENTION_PARSES)]
        with self._confined_contention():
            results = {
                deny_sockets: gate._isolated_probe(command, work, deny_sockets=deny_sockets)
                for deny_sockets in (True, False)
            }
        for deny_sockets, result in results.items():
            with self.subTest(deny_sockets=deny_sockets):
                self.assertEqual(
                    (result.returncode, result.stdout.strip()),
                    (0, f"PARSED {_CONTENTION_PARSES}"),
                    result.stdout[-2000:],
                )
        print(f"GOOD §26 {2 * _CONTENTION_PARSES} contended fixture parses survive both filter modes")

    def test_sequential_isolated_probe_work_is_not_shared(self) -> None:
        work, payload = self._write_probe_script(
            "sequential-private-work",
            """from pathlib import Path
root = Path.cwd()
counter = root / "cross-probe-counter"
previous = counter.read_text(encoding="ascii") if counter.exists() else ""
counter.write_text(previous + "x", encoding="ascii")
git_counter = root / "records-like" / ".git" / "counter"
git_counter.parent.mkdir(parents=True, exist_ok=True)
previous_git = git_counter.read_text(encoding="ascii") if git_counter.exists() else ""
git_counter.write_text(previous_git + "y", encoding="ascii")
print("work=" + counter.read_text(encoding="ascii") + " git=" + git_counter.read_text(encoding="ascii"))
""",
        )
        results = [
            gate._isolated_probe([sys.executable, str(payload)], work, deny_sockets=True)
            for _ in range(2)
        ]
        for result in results:
            self.assertEqual((result.returncode, result.stdout.strip()), (0, "work=x git=y"))
        self.assertFalse((work / "cross-probe-counter").exists())
        self.assertFalse((work / "records-like").exists())

    def test_prepare_probe_cannot_ping_pong_with_a_host_unix_socket(self) -> None:
        work, payload = self._write_probe_script(
            "prepare-unix-host-channel",
            """import os, socket
os.chdir("host-sockets")
client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
client.settimeout(2)
try:
    client.connect("host-visible.sock")
    client.sendall(b"host-ping")
    print("HOST_REACHED", client.recv(64).decode())
except OSError as error:
    print("NO_HOST_CHANNEL", error.errno)
finally:
    client.close()
""",
        )
        socket_dir = work / "host-sockets"
        socket_dir.mkdir()
        endpoint = socket_dir / "host-visible.sock"
        listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        original_cwd = Path.cwd()
        try:
            os.chdir(socket_dir)
            listener.bind("host-visible.sock")
        finally:
            os.chdir(original_cwd)
        listener.listen(1)
        listener.settimeout(0.1)
        stopping = threading.Event()
        received: list[bytes] = []

        def serve() -> None:
            while not stopping.is_set():
                try:
                    connection, _ = listener.accept()
                except socket.timeout:
                    continue
                except OSError:
                    return
                with connection:
                    data = connection.recv(64)
                    received.append(data)
                    connection.sendall(b"pong:" + data)

        worker = threading.Thread(target=serve, daemon=True)
        worker.start()
        isolation_error = None
        try:
            result = gate._isolated_probe(
                [sys.executable, str(payload)], work, deny_sockets=False
            )
        except gate.GateError as error:
            isolation_error = str(error)
        finally:
            stopping.set()
            listener.close()
            worker.join(timeout=1)
            endpoint.unlink(missing_ok=True)
        self.assertEqual(received, [])
        if isolation_error is not None:
            self.assertRegex(isolation_error, r"probe input|regular file|socket|special")
            return
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("NO_HOST_CHANNEL", result.stdout)
        self.assertNotIn("HOST_REACHED", result.stdout)

    def test_prepare_child_socket_cannot_accept_a_host_connection(self) -> None:
        # Review r5's direction: E' binds in work, then the host connects.
        # Relative Unix paths keep this valid in deeply nested fixture suites.
        work, payload = self._write_probe_script(
            "prepare-unix-child-server",
            """import socket
from pathlib import Path
with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as server:
    server.bind("host-visible.sock")
    server.listen(1)
    server.settimeout(1)
    Path("sock-ready").write_text("ready", encoding="ascii")
    try:
        connection, _ = server.accept()
    except socket.timeout:
        print("NO_HOST_CHANNEL")
    else:
        with connection:
            message = connection.recv(64)
            connection.sendall(b"pong:" + message)
            print("HOST_REACHED", message)
""",
        )
        reached: list[bytes] = []
        stopping = threading.Event()

        def connect_when_ready() -> None:
            ready = work / "sock-ready"
            for _ in range(100):
                if ready.exists() or stopping.wait(0.02):
                    break
            if not ready.exists() or stopping.is_set():
                return
            directory = os.open(work, os.O_RDONLY | os.O_DIRECTORY)
            try:
                with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
                    client.settimeout(1)
                    # The proc fd path is short even when work is deeply nested.
                    client.connect(f"/proc/self/fd/{directory}/host-visible.sock")
                    client.sendall(b"host-ping")
                    reached.append(client.recv(64))
            except OSError:
                pass
            finally:
                os.close(directory)

        worker = threading.Thread(target=connect_when_ready)
        worker.start()
        try:
            result = gate._isolated_probe([sys.executable, str(payload)], work, deny_sockets=False)
        finally:
            stopping.set()
            worker.join(timeout=2)
        self.assertFalse(worker.is_alive())
        self.assertEqual(reached, [])
        self.assertFalse((work / "sock-ready").exists())
        self.assertFalse((work / "host-visible.sock").exists())
        self.assertIn(result.returncode, (0, 159), result.stdout)
        if result.returncode == 0:
            self.assertIn("NO_HOST_CHANNEL", result.stdout)
        self.assertNotIn("HOST_REACHED", result.stdout)

    def test_prepare_filter_kills_nonlocal_socket_families_before_uevents(self) -> None:
        uevents = socket.socket(socket.AF_NETLINK, socket.SOCK_RAW, 15)
        uevents.bind((os.getpid(), 1))
        uevents.settimeout(0.1)
        self.addCleanup(uevents.close)
        cases = (
            ("af_key", 15, socket.SOCK_RAW, 0),
            ("packet", 17, socket.SOCK_RAW, socket.htons(0x0003)),
            ("bluetooth", 31, socket.SOCK_RAW, 1),
            ("netlink", 16, socket.SOCK_RAW, 15),
            ("netlink-uevent-send", 16, socket.SOCK_RAW, 15),
            ("alg", 38, socket.SOCK_SEQPACKET, 0),
        )
        for name, family, socket_type, protocol in cases:
            for deny_sockets in (True, False):
                if name == "netlink-uevent-send":
                    source = f"""import socket
sock = socket.socket({family}, {socket_type}, {protocol})
sock.bind((0, 1))
sock.sendto(b"f0-r6-crafted-uevent-marker\\0", (0, 1))
print('UEVENT_SENT')
"""
                else:
                    source = f"""import socket
socket.socket({family}, {socket_type}, {protocol})
print('SOCKET_OPENED')
"""
                work, payload = self._write_probe_script(
                    f"socket-family-{name}-{deny_sockets}", source
                )
                result = gate._isolated_probe(
                    [sys.executable, str(payload)], work, deny_sockets=deny_sockets
                )
                with self.subTest(family=name, deny_sockets=deny_sockets):
                    self.assertEqual((result.returncode, result.stdout.strip()), (159, ""))
        received = []
        while True:
            try:
                event, _ = uevents.recvfrom(8192)
            except socket.timeout:
                break
            received.append(event)
        uevents.close()
        self.assertFalse(
            any(
                b"add@/module/af_key" in event
                or b"f0-r6-crafted-uevent-marker" in event
                for event in received
            ),
            received,
        )

    def test_keyring_syscall_families_are_killed_and_session_key_is_not_host_visible(self) -> None:
        description = f"f0-r6-probe-{self.root.name}".encode("ascii")
        _revoke_host_session_key(description)
        source = f"""import ctypes, sys
libc = ctypes.CDLL(None, use_errno=True)
libc.syscall.restype = ctypes.c_long
description = {description!r}
key_type = ctypes.c_char_p(b"user" + bytes([0]))
key_description = ctypes.c_char_p(description + bytes([0]))
session_ring = ctypes.c_long(-3)
operation = sys.argv[1]
if operation == "add_key":
    result = libc.syscall(248, key_type, key_description, ctypes.c_char_p(b"41"), 2, session_ring)
elif operation == "request_key":
    result = libc.syscall(249, key_type, key_description, None, session_ring)
else:
    result = libc.syscall(250, 10, session_ring, key_type, key_description, 0)
print("RAN", operation, result, ctypes.get_errno())
"""
        try:
            for deny_sockets in (True, False):
                for operation in ("add_key", "request_key", "keyctl"):
                    work, payload = self._write_probe_script(
                        f"keyring-{operation}-{deny_sockets}", source
                    )
                    result = gate._isolated_probe(
                        [sys.executable, str(payload), operation],
                        work,
                        deny_sockets=deny_sockets,
                    )
                    with self.subTest(operation=operation, deny_sockets=deny_sockets):
                        self.assertEqual((result.returncode, result.stdout.strip()), (159, ""))
            key, _ = _host_session_key_id(description)
            self.assertLess(key, 0, "a probe-created session key must not be readable by the host")
        finally:
            _revoke_host_session_key(description)

    def test_killpg_signal_is_confined_to_the_probe_session(self) -> None:
        work, payload = self._write_probe_script(
            "killpg-session",
            f"""import os, signal
signal.signal({signal.SIGUSR1}, signal.SIG_IGN)
os.killpg(os.getpgrp(), signal.SIGUSR1)
print('SESSION_SIGNAL_CONFINED')
""",
        )
        driver = f"""import json, signal, sys
from pathlib import Path
sys.path.insert(0, {str(CHECKER_ROOT / 'tools/architecture')!r})
import trusted_gate as gate
hits = []
signal.signal(signal.SIGUSR1, lambda *_: hits.append(1))
result = gate._isolated_probe([sys.executable, {str(payload)!r}], Path({str(work)!r}), deny_sockets=True)
print(json.dumps({{"probe_returncode": result.returncode, "probe_stdout": result.stdout,
                  "host_signal_hits": len(hits)}}))
"""
        completed = subprocess.run(
            [sys.executable, "-c", driver],
            cwd=self.root,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            start_new_session=True,
            timeout=45,
        )
        self.assertEqual(completed.returncode, 0, completed.stdout)
        outcome = json.loads(completed.stdout.strip())
        self.assertEqual(outcome["host_signal_hits"], 0)
        if outcome["probe_returncode"] == 159:
            self.assertEqual(outcome["probe_stdout"], "")
        else:
            self.assertEqual(outcome["probe_returncode"], 0, outcome)
            self.assertIn("SESSION_SIGNAL_CONFINED", outcome["probe_stdout"])

    def test_alternates_conditioned_e_prime_is_refused(self) -> None:
        self._poisoned_gate_e_prime(
            "shared-alternates", ALTERNATES_POISON, after_removal=True,
            expected_message="admitted the known-bad",
        )

    def test_synthetic_sequential_chain_e_prime_is_refused(self) -> None:
        self._poisoned_gate_e_prime(
            "sequential-chain-shape", SEQUENTIAL_CHAIN_POISON, after_removal=True,
            expected_message="sequential controls (exit 0)",
        )

    def test_fixed_control_comment_e_prime_is_refused(self) -> None:
        self._poisoned_gate_e_prime(
            "sequential-control-comment", SEQUENTIAL_COMMENT_POISON, after_removal=True,
            expected_message="sequential controls (exit 0)",
        )

    def test_vault_only_threshold_below_k_is_refused_by_every_sequential_bad_probe(self) -> None:
        self._poisoned_gate_e_prime(
            "sequential-vault-threshold", SEQUENTIAL_VAULT_POISON, after_removal=True,
            expected_message="sequential controls (exit 0)",
        )

    def test_prepare_refusal_string_for_old_stub_is_refused(self) -> None:
        self._poisoned_gate_e_prime(
            "prepare-stub-refusal-string", PREPARE_STUB_REFUSAL_POISON, prepare_mode=True, after_removal=True,
            expected_message="prepare-mode qualification failed: proposed E' changed trusted records",
        )

    def test_probe_controls_subjects_and_neuters_are_drawn_per_run(self) -> None:
        manifests = [Path("Cargo.toml")] + [Path("crates") / name / "Cargo.toml" for name in ("api", "frontend-core", "script", "vault")]
        original = {path: tomllib.loads((self.b1 / path).read_text(encoding="utf-8")) for path in manifests}
        changes = set()
        for index in range(12):
            tree = _clone_at(self.b1, self.root / f"control-{index}", self.b1_sha)
            gate._probe_control(tree)
            changed = [path for path in manifests if (tree / path).read_bytes() != (self.b1 / path).read_bytes()]
            self.assertEqual(len(changed), 1)
            self.assertEqual(tomllib.loads((tree / changed[0]).read_text(encoding="utf-8")), original[changed[0]])
            self.assertNotIn("Workspace notes are tracked in the README.", (tree / changed[0]).read_text(encoding="utf-8"))
            changes.add((changed[0], _git(tree, "diff", "--unified=0")))
        self.assertGreater(len(changes), 1)
        identities = gate._ProbeIdentities(self.history, self.b1_sha)
        drawn = [identities.draw() for _ in range(2)]
        self.assertEqual(len(set(drawn)), 2)
        self.assertEqual({subject for _, _, subject in drawn}, {"fixture accepted B0", "fixture orchestrator debt removal B1"})
        old_stub = re.compile(r"def admit\(args\):\n    _[0-9a-f]{32} = args\n    return None\n")
        shapes = set()
        for _ in range(40):
            admit_source = gate._neutered_admit(gate.secrets.SystemRandom())
            self.assertIsNone(old_stub.search(admit_source))
            namespace: dict[str, object] = {}
            exec(compile(admit_source, "<neutered admit>", "exec"), namespace)
            self.assertIsNone(namespace["admit"](SimpleNamespace()))
            shapes.add(admit_source.split("(")[0].split()[0])
            checks = {}
            exec(compile(gate._neutered_checks(gate.secrets.SystemRandom()), "<neutered checks>", "exec"), checks)
            self.assertEqual((checks["check_tree"](1), checks["check_transition"](1, 2), checks["check_graph"](1, 2)), ([], [], []))
        self.assertGreater(len(shapes), 1)
        print("GOOD §26 control text, probe subjects, and the prepare neuter differ per run")

    def test_rotated_edge_does_not_reselect_first_pair(self) -> None:
        candidate, _ = self._candidate_from(self.b1, "rotated-edge-tree", lambda root: gate._probe_rotated_edge(root))
        with self.assertRaisesRegex(gate.GateError, "disallowed crate edge"):
            gate._evaluate_transition(self.enforcement, self.b1, candidate)
        first = _clone_at(self.b1, self.root / "first-edge-tree", self.b1_sha)
        gate._probe_added_edge(first)
        self.assertNotEqual(
            [(candidate / "crates" / name / "Cargo.toml").read_bytes() for name in ("api", "frontend-core", "script", "vault")],
            [(first / "crates" / name / "Cargo.toml").read_bytes() for name in ("api", "frontend-core", "script", "vault")],
        )

    def test_enforcement_checkout_cleanliness_ignores_out_of_bundle_submodule(
        self,
    ) -> None:
        source = self.root / "submodule-source"
        _init_repo(source)
        (source / "tracked.txt").write_text("clean\n", encoding="utf-8")
        _commit(source, "fixture submodule source")
        _git(source, "branch", gate.SUBMODULE_BRANCH)

        checkout = self.root / "checkout-with-submodule"
        _init_repo(checkout)
        (checkout / "root.txt").write_text("clean\n", encoding="utf-8")
        _commit(checkout, "fixture enforcement checkout")
        _git(
            checkout,
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "--branch",
            gate.SUBMODULE_BRANCH,
            str(source),
            gate.SUBMODULE_PATH,
        )
        _git(
            checkout,
            "config",
            "-f",
            ".gitmodules",
            f"submodule.{gate.SUBMODULE_PATH}.url",
            gate.SUBMODULE_URL,
        )
        checkout_sha = _commit(checkout, "fixture enforcement module pin")
        gate.verify_checkout(
            checkout,
            checkout_sha,
            "fixture enforcement checkout",
            check_submodules=False,
        )

        (checkout / gate.SUBMODULE_PATH / "tracked.txt").write_text("dirty\n", encoding="utf-8")
        with self.assertRaises(gate.GateError):
            gate.verify_checkout(checkout, checkout_sha, "fixture product checkout")
        gate.verify_checkout(
            checkout,
            checkout_sha,
            "fixture enforcement checkout",
            check_submodules=False,
        )


    def test_admission_probe_trees_populate_the_client_gitlink_locally(self) -> None:
        # Admission checks B/H with the client submodule initialized (ownership
        # covers its files), so each probe tree must be populated the same way.
        source = self.root / "client-source"
        _init_repo(source)
        (source / "client.rs").write_text("// client\n", encoding="utf-8")
        client_sha = _commit(source, "fixture client")
        product = self.root / "product-with-client"
        _init_repo(product)
        (product / "root.txt").write_text("product\n", encoding="utf-8")
        _commit(product, "fixture product")
        _git(product, "-c", "protocol.file.allow=always", "submodule", "add", str(source), gate.SUBMODULE_PATH)
        product_sha = _commit(product, "fixture client gitlink")
        unpopulated = gate._clone_at(product, self.root / "probe-without-source", product_sha)
        gate._populate_gitlinks(self.root / "missing-source", unpopulated)
        self.assertFalse((unpopulated / gate.SUBMODULE_PATH / "client.rs").exists())
        probe = gate._clone_at(product, self.root / "probe-tree", product_sha, depth=2)
        gate._populate_gitlinks(product, probe)
        candidate = gate._clone_at(probe, self.root / "probe-candidate", product_sha)
        gate._populate_gitlinks(probe, candidate)
        for tree in (probe, candidate):
            self.assertEqual(sorted((tree / ".git").glob("**/objects/info/alternates")), [])
            objects = [path for path in (tree / ".git").glob("**/objects/**/*") if path.is_file()]
            self.assertTrue(any("modules" in path.parts for path in objects))
            self.assertEqual([path for path in objects if path.stat().st_nlink != 1], [])
            self.assertEqual(_git(tree / gate.SUBMODULE_PATH, "rev-parse", "HEAD"), client_sha)
            self.assertTrue((tree / gate.SUBMODULE_PATH / "client.rs").is_file())
            gate.verify_checkout(tree, product_sha, "populated probe tree")
        # Round-4 review: a `--shared` clone exposes an alternates file and a
        # plain local clone hard-links the source's object files into the
        # writable probe tree. Neither may reach an E' probe.
        _git(self.root, "clone", "--quiet", "--shared", str(product), str(self.root / "shared-clone"))
        with self.assertRaisesRegex(gate.GateError, "alternates file"):
            gate._require_independent_objects(self.root / "shared-clone")
        _git(self.root, "clone", "--quiet", str(product), str(self.root / "hardlinked-clone"))
        with self.assertRaisesRegex(gate.GateError, "hard-linked object file"):
            gate._require_independent_objects(self.root / "hardlinked-clone")
        print("GOOD §26 probe trees are independent object copies with the client gitlink populated")

    def test_trusted_dispatch_includes_inventory_findings(self) -> None:
        candidate = _clone_at(self.b1, self.root / "invalid-inventory", self.b1_sha)
        inventory = candidate / gate.ARCH_DIR / "inventories.json"
        inventory.write_text("{}\n", encoding="utf-8")
        findings = gate.checker_findings(self.enforcement, "tree", root=candidate)
        expected = (
            "inventory schema: top-level fields must be exactly "
            "schema_version, seam_types, violations, test_exceptions"
        )
        print("BAD §26: malformed inventory must be rejected by the trusted dispatcher")
        self.assertIn(expected, findings)
        print("GOOD §26: trusted dispatcher applies the required inventory checker")


class TrustedGateTransitionTests(GateFixture):
    def test_two_merge_poisoning_ci_forced_base_approval_and_legitimate_update(self) -> None:
        self._flow_two_merge()

    def test_client_gitlink_move_requires_exact_approval(self) -> None:
        client = self.root / "client-history"
        _init_repo(client)
        (client / "README").write_text("one\n", encoding="utf-8")
        first = _commit(client, "client one")
        (client / "README").write_text("two\n", encoding="utf-8")
        second = _commit(client, "client two")

        history = self.root / "gitlink-history"
        _write_product_tree(history, self.enforcement, debt=False)
        (history / ".gitmodules").write_text(
            f'[submodule "{gate.SUBMODULE_PATH}"]\n'
            f"\tpath = {gate.SUBMODULE_PATH}\n"
            f"\turl = {gate.SUBMODULE_URL}\n"
            f"\tbranch = {gate.SUBMODULE_BRANCH}\n",
            encoding="utf-8",
        )
        _init_repo(history)
        identity = ("-c", "user.name=Architecture Gate Fixture", "-c", "user.email=architecture-gate-fixture@example.invalid")
        # `git add --all` would drop a gitlink whose directory is absent, so
        # stage files first and commit the index directly.
        _git(history, "add", "--all")
        _git(history, "update-index", "--add", "--cacheinfo", f"160000,{first},{gate.SUBMODULE_PATH}")
        _git(history, *identity, "commit", "--quiet", "-m", "fixture base with client gitlink")
        base_sha = _git(history, "rev-parse", "HEAD")
        # Same path, URL, and branch; only the recorded client commit moves.
        _git(history, "update-index", "--cacheinfo", f"160000,{second},{gate.SUBMODULE_PATH}")
        _git(history, *identity, "commit", "--quiet", "-m", "fixture SHA-only client gitlink move")
        candidate_sha = _git(history, "rev-parse", "HEAD")
        base = _clone_at(history, self.root / "gitlink-B", base_sha)
        candidate = _clone_at(history, self.root / "gitlink-H", candidate_sha)

        records = self.root / "gitlink-records"
        records.mkdir()
        (records / "approvals").mkdir()
        (records / "receipts.jsonl").write_text("", encoding="utf-8")
        bootstrap = {
            "accepted_sha": base_sha,
            "contract_epoch": 0,
            "enforcement_sha": self.e_sha,
            "bundle_digest": self.e_digest,
            "runtime_pin": self.runtime_pin,
            "schema_versions": gate._schema_versions(base),
            "input_digest": gate.input_digest(base),
            "orchestrator_commit": base_sha,
            "reason": "Fixture bootstrap with the protected client gitlink",
            "operator_inline_approval": "Fixture bootstrap approval",
            "owner_acknowledgements": ["T1"],
        }
        (records / "gate.json").write_text(
            json.dumps(
                {"format": gate.FORMAT, "schema_version": gate.GATE_SCHEMA, "target_ref": TARGET_REF, "bootstrap": bootstrap},
                sort_keys=True,
                separators=(",", ":"),
            )
            + "\n",
            encoding="utf-8",
        )
        _init_repo(records)
        self.records = records
        self.records_sha = _commit(records, "fixture gitlink bootstrap records")
        self.repo_root = _clone_at(history, self.root / "gitlink-integration", base_sha)
        _git(self.repo_root, "update-ref", TARGET_REF, base_sha)

        args = self._args(base=base, candidate=candidate, head_sha=candidate_sha)
        expected = f"client gitlink moved: {gate.SUBMODULE_PATH} {first} -> {second}"
        self._assert_denied(args, contains=expected)
        print(f"BAD §23 §3 SHA-only client gitlink move refused without approval: {expected}")

        current, _, _ = gate._read_record_store(records, TARGET_REF)
        findings = gate._evaluate_transition(self.enforcement, base, candidate)
        self.assertEqual(findings, [expected])
        approval = _approval(
            kind="policy",
            head_sha=candidate_sha,
            base_sha=base_sha,
            candidate_digest=gate.input_digest(candidate),
            current=current,
            proposed_sha=self.e_sha,
            proposed_digest=self.e_digest,
            delta=findings,
        )
        (records / "approvals" / f"{candidate_sha}.json").write_text(
            json.dumps(approval, sort_keys=True) + "\n", encoding="utf-8"
        )
        self._commit_records("fixture exact approval for the client gitlink move")
        gate.admit(self._args(base=base, candidate=candidate, head_sha=candidate_sha))
        current, receipts, _ = gate._read_record_store(records, TARGET_REF)
        self.assertEqual(current["head_sha"], candidate_sha)
        self.assertEqual(current["contract_epoch"], 1)
        self.assertEqual(receipts[-1]["transition_kind"], "policy")
        self.assertEqual(receipts[-1]["delta"], [expected])
        print("GOOD §23 §3 exactly approved client gitlink move admitted as a contract transition")


if __name__ == "__main__":
    unittest.main()
