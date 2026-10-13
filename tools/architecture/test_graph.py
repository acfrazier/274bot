"""§23 §4.6 graph fixtures. Run: python3 -m unittest discover -s tools/architecture -v."""
import importlib.util
import os
from pathlib import Path
import shutil
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
CHECKER_ROOT = Path(os.environ.get("F0_ARCH_TEST_CHECKER_ROOT", HERE.parents[1]))
SPEC = importlib.util.spec_from_file_location("architecture_graph_tests_checker", CHECKER_ROOT / "tools/architecture/check.py")
checker = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = checker
SPEC.loader.exec_module(checker)


def config(features=()):
    return '''\n[[configuration]]
id = "fixture"
packages = ["vault"]
features = %s
default_features = true
dev = false
status = "enforced"
owner = "T1"
forbidden_features = []
optional_dependencies = []
''' % repr(list(features))


def migration(identity="TEMP", kind="normal"):
    return f'''\n[[migration]]
id = "{identity}"
from = "vault"
to = "api"
kind = "{kind}"
owner = "T3"
consumers = ["vault:lib:vault"]
reason = "Temporary fixture debt"
removal_endpoint = "T3.3"
'''


def workspace(root, *, kind="normal", debt=True, identity="TEMP", cfg="cfg(unix)", dev=False, feature=False):
    checker.write_workspace(root, ["vault", "api"])
    tables = {}
    spec = {"path": "../api"}
    if kind == "optional":
        spec["optional"] = True
    table = {"normal": "dependencies", "optional": "dependencies", "dev": "dev_dependencies",
             "target": f"target.'{cfg}'.dependencies"}.get(kind)
    if table:
        tables[table] = {"api": spec}
    if dev:
        tables["dev_dependencies"] = {"api": {"path": "../api"}}
    checker.write_crate(root, "vault", **tables)
    checker.write_crate(root, "api")
    if feature:
        with (root / "crates/vault/Cargo.toml").open("a") as stream:
            stream.write('\n[features]\ndefault = []\nprobe = ["dep:api"]\n')
    policy = root / "tools/architecture/policy.toml"
    policy.parent.mkdir(parents=True)
    allows = []
    if kind and not debt:
        allows.append({"from": "vault", "to": "api", "kinds": [kind], "reason": "Permanent fixture permission"})
    if dev:
        allows.append({"from": "vault", "to": "api", "kinds": ["dev"], "reason": "Test fixture permission"})
    checker.write_policy(policy, ["vault", "api"], allows)
    with policy.open("a") as stream:
        if debt:
            stream.write(migration(identity, kind or "normal"))
        stream.write(config())
    return root


class GraphRatchetTests(unittest.TestCase):
    def setUp(self):
        target = HERE.parents[1] / "target"
        target.mkdir(exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(prefix="architecture-graph-", dir=target)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def paired(self, label, base, bad, good, needle):
        self.assertEqual(checker.check_graph_tree(base), [], "fixture baseline must be valid")
        findings = checker.check_graph_transition(base, bad)
        self.assertTrue(any(needle in item for item in findings), findings)
        print(f"FIXTURE {label} BAD rejected: {'; '.join(findings)}")
        good_findings = checker.check_graph_transition(base, good)
        self.assertEqual(good_findings, [])
        print(f"FIXTURE {label} GOOD accepted")

    def test_remove_edge_and_allow_together_and_stale_allow(self):
        base = workspace(self.root / "base")
        bad = workspace(self.root / "bad", kind=None)
        good = workspace(self.root / "good", kind=None, debt=False)
        self.paired("remove-edge-and-allow/stale-allow", base, bad, good, "stale migration")

    def test_removed_then_restored(self):
        original = workspace(self.root / "original")
        removed = workspace(self.root / "removed", kind=None, debt=False)
        self.assertEqual(checker.check_graph_transition(original, removed), [])
        bad = workspace(self.root / "bad")
        self.paired("removed-then-restored-edge", removed, bad, removed, "effective edge added")

    def test_renamed_id_restoration(self):
        base = workspace(self.root / "base", kind=None, debt=False)
        bad = workspace(self.root / "bad", identity="NEW-NAME")
        self.paired("renamed-ID-restoration", base, bad, base, "migration identity added")

    def test_existing_id_cannot_change(self):
        base = workspace(self.root / "base")
        bad = workspace(self.root / "bad", identity="RENAMED")
        self.paired("existing-ID-rename", base, bad, base, "migration identity added")

    def test_normal_target_change_both_directions(self):
        for before, after in (("normal", "target"), ("target", "normal")):
            base = workspace(self.root / (before + "-base"), kind=before)
            bad = workspace(self.root / (before + "-bad"), kind=after)
            self.paired(f"{before}-to-{after}", base, bad, base, "effective edge added")

    def test_literal_kind_no_implicit_target(self):
        base = workspace(self.root / "base", debt=False)
        bad = workspace(self.root / "bad", kind="target", debt=False)
        shutil.copyfile(base / "tools/architecture/policy.toml", bad / "tools/architecture/policy.toml")
        self.paired("normal-does-not-authorize-target", base, bad, base, "edge kind")

    def test_optional_dev_to_required(self):
        for before in ("optional", "dev"):
            base = workspace(self.root / (before + "-base"), kind=before, debt=False)
            bad = workspace(self.root / (before + "-bad"), debt=False)
            self.paired(f"{before}-to-required", base, bad, base, "effective edge added")

    def test_mixed_normal_dev_removal(self):
        base = workspace(self.root / "base", dev=True)
        good = workspace(self.root / "good", kind=None, debt=False, dev=True)
        bad = workspace(self.root / "bad", kind=None, debt=True, dev=True)
        self.paired("mixed-normal-dev-removal", base, bad, good, "stale migration")
        self.assertEqual({e.kind for e in checker.collect_edges(*checker.discover_workspace(good)[1:], good)}, {"dev"})

    def test_target_cfg_provenance(self):
        base = workspace(self.root / "base", kind="target", cfg="cfg(unix)")
        bad = workspace(self.root / "bad", kind="target", cfg="cfg(windows)")
        self.paired("target-cfg-provenance", base, bad, base, "provenance/activation widened")

    def test_optional_table_provenance(self):
        base = workspace(self.root / "base", kind="optional", debt=False)
        bad = workspace(self.root / "bad", kind="optional", debt=False)
        manifest = bad / "crates/vault/Cargo.toml"
        manifest.write_text(manifest.read_text().replace("[dependencies]", "[target.'cfg(unix)'.dependencies]"))
        self.paired("optional-table-provenance", base, bad, base, "provenance/activation widened")

    def test_feature_activation_widening(self):
        base = workspace(self.root / "base", kind="optional", debt=False, feature=True)
        bad = workspace(self.root / "bad", kind="optional", debt=False, feature=True)
        manifest = bad / "crates/vault/Cargo.toml"
        manifest.write_text(manifest.read_text().replace("default = []", 'default = ["probe"]'))
        self.paired("supported-feature-activation", base, bad, base, "provenance/activation widened")

    def test_lost_base_history(self):
        base = workspace(self.root / "base")
        errors = checker.check_graph_transition(self.root / "missing", base)
        self.assertTrue(any("missing workspace manifest" in item or "No such file" in item for item in errors), errors)
        print("FIXTURE lost-base-history BAD rejected: " + "; ".join(errors))
        self.assertEqual(checker.check_graph_transition(base, base), [])
        print("FIXTURE lost-base-history GOOD accepted")

    def test_strict_schema(self):
        base = workspace(self.root / "base")
        for label, addition in (("unknown-field", '\nunknown = true\n'),
                                ("duplicate-ID", migration()),
                                ("unknown-owner", None)):
            bad = self.root / label
            shutil.copytree(base, bad)
            policy = bad / "tools/architecture/policy.toml"
            if addition is None:
                policy.write_text(policy.read_text().replace('owner = "T3"', 'owner = "T6"'))
            else:
                with policy.open("a") as stream:
                    stream.write(addition)
            self.paired(label, base, bad, base, "invalid graph input")

    def test_native_only_row_cannot_be_waived(self):
        base = workspace(self.root / "base")
        bad = workspace(self.root / "bad")
        row = config().replace('id = "fixture"', 'id = "native-only"').replace('status = "enforced"', 'status = "pending"').replace('owner = "T1"', 'owner = "T2"')
        with (base / "tools/architecture/policy.toml").open("a") as stream:
            stream.write(row)
        self.paired("native-only-row-removal", base, bad, base, "matrix changed")

    def test_seat_model_conformance_oracle_is_dev_only(self):
        base = self.root / "base"
        checker.write_workspace(base, ["script", "seat-model"])
        checker.write_crate(base, "script", dev_dependencies={"seat-model": {"path": "../seat-model"}})
        checker.write_crate(base, "seat-model")
        policy = base / "tools/architecture/policy.toml"
        policy.parent.mkdir(parents=True)
        checker.write_policy(policy, ["script", "seat-model"], [{
            "from": "script", "to": "seat-model", "kinds": ["dev"],
            "reason": "Main 2026-10-12, F0-SCRIPT-SEAM conformance oracle, §27 §2.2 / ADR 0008",
        }])
        for kind in ("normal", "build"):
            bad = self.root / kind
            shutil.copytree(base, bad)
            manifest = bad / "crates/script/Cargo.toml"
            table = "dependencies" if kind == "normal" else "build-dependencies"
            manifest.write_text(manifest.read_text().replace("[dev-dependencies]", f"[{table}]"))
            self.paired(f"seat-model-dev-to-{kind}", base, bad, base, "edge kind")


if __name__ == "__main__":
    unittest.main(verbosity=2)
