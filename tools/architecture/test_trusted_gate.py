"""Protected local-admission and detective-CI poisoning fixtures.

Run with: python3 -m unittest discover -s tools/architecture -p 'test_*.py' -v
The fixture treats product trees as files. It never imports or executes code
from a candidate tree.
"""
from __future__ import annotations

import json
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest import mock



HERE = Path(__file__).resolve().parent
DEFAULT_CHECKER_ROOT = HERE.parents[1]
CHECKER_ROOT = Path(os.environ.get("F0_ARCH_TEST_CHECKER_ROOT", DEFAULT_CHECKER_ROOT)).resolve()
_spec = importlib.util.spec_from_file_location("architecture_gate_under_test", CHECKER_ROOT / "tools/architecture/trusted_gate.py")
gate = importlib.util.module_from_spec(_spec)
sys.modules[_spec.name] = gate
_spec.loader.exec_module(gate)
TEST_ROOT = Path(os.environ.get("F0_ARCH_TEST_ROOT", DEFAULT_CHECKER_ROOT / "target")).resolve()
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


class TrustedGateProtocolTests(unittest.TestCase):
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
        if os.environ.get("F0_ARCH_QUALIFICATION_CHILD") == "1":
            # This process is already inside the qualified corpus; do not
            # recursively launch another full corpus, but exercise the rest
            # of prepare/admit with the same fixture assertions.
            with mock.patch.object(gate, "_run_child"):
                gate.prepare_enforcement(args)
            return
        gate.prepare_enforcement(args)

    def _ci_check(self, args: object) -> None:
        if os.environ.get("F0_ARCH_QUALIFICATION_CHILD") == "1":
            # CI suite execution is already the caller for this child process.
            with mock.patch.object(gate, "_run_bundle_suite"):
                gate.ci_check(args)
            return
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
        self.assertIn(
            "test_trusted_dispatch_includes_inventory_findings",
            str(rejected.exception),
        )
        self.assertIn("FAILED", str(rejected.exception))
        self.assertFalse((self.records / "pending.json").exists())
        self.assertEqual(
            (self.records / "receipts.jsonl").read_bytes(), receipts_before
        )
        self.assertEqual(_git(self.repo_root, "rev-parse", TARGET_REF), target_before)
        print(
            f"BAD §26 {label}: exact-approved unsafe E' rejected by the old fixture corpus"
        )

    def test_two_merge_poisoning_ci_forced_base_approval_and_legitimate_update(self) -> None:
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

        # Outer admission runs the full push suite; qualified children suppress
        # only recursive suite discovery through _ci_check.
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
        # the old fixture corpus during qualification.
        if os.environ.get("F0_ARCH_QUALIFICATION_CHILD") != "1":
            self._assert_bad_e_prime_rejected(
                "always-success-E-prime", omit_inventory=False
            )
            self._assert_bad_e_prime_rejected(
                "omitted-inventory-E-prime", omit_inventory=True
            )

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
        print("GOOD §26 E-prime qualified by real active and proposed full fixture suites")
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
        print("GOOD §26 exact E-prime receipt activated only after real qualification")
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



if __name__ == "__main__":
    unittest.main()
