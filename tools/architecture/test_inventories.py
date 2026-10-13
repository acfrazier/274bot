"""Behavioral fixtures for syntax-aware seam and navigation inventories."""

from __future__ import annotations

import importlib.util
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
CHECKER_ROOT = Path(os.environ.get("F0_ARCH_TEST_CHECKER_ROOT", HERE.parents[1]))
INVENTORIES_PATH = CHECKER_ROOT / "tools" / "architecture" / "inventories.py"
INVENTORIES_SPEC = importlib.util.spec_from_file_location("inventories", INVENTORIES_PATH)
if INVENTORIES_SPEC is None or INVENTORIES_SPEC.loader is None:
    raise ImportError(f"cannot load trusted inventory checker from {INVENTORIES_PATH}")
inventories = importlib.util.module_from_spec(INVENTORIES_SPEC)
sys.modules[INVENTORIES_SPEC.name] = inventories
INVENTORIES_SPEC.loader.exec_module(inventories)

VIEWS_PATH = "crates/frontend-core/src/views.rs"


def _seam(
    symbol: str, classification: str, path: str = VIEWS_PATH
) -> dict[str, str]:
    return {
        "id": f"seam_type|{path}|{symbol}",
        "path": path,
        "symbol": symbol,
        "classification": classification,
        "owner": "T3",
        "reason": "fixture seam classification",
    }


def _record(
    category: str,
    path: str,
    symbol: str,
    cfg_or_target: str,
    classification: str,
    *,
    owner: str = "T5",
) -> dict[str, str]:
    endpoint = "T3.1" if owner == "T3" else "T5.1"
    record_id = "|".join((category, path, cfg_or_target, symbol, classification))
    return {
        "id": record_id,
        "category": category,
        "path": path,
        "symbol": symbol,
        "cfg_or_target": cfg_or_target,
        "owner": owner,
        "reason": "fixture baseline debt",
        "removal_endpoint": endpoint,
        "classification": classification,
    }


def _write_fixture(
    root: Path,
    *,
    views: str,
    frontend_lib: str = "pub mod views;\n",
    frontend_files: dict[str, str] | None = None,
    script_files: dict[str, str] | None = None,
    seam_types: tuple[tuple[str, str], ...] = (),
    additional_seam_types: tuple[dict[str, str], ...] = (),
    violations: tuple[dict[str, str], ...] = (),
    test_exceptions: tuple[dict[str, str], ...] = (),
) -> Path:
    for crate in ("script", "frontend-core"):
        crate_root = root / "crates" / crate
        crate_root.mkdir(parents=True)
        (crate_root / "Cargo.toml").write_text(
            f'[package]\nname = "{crate}"\nversion = "0.0.0"\nedition = "2021"\n',
            encoding="utf-8",
        )
    frontend_src = root / "crates" / "frontend-core" / "src"
    frontend_src.mkdir()
    (frontend_src / "lib.rs").write_text(frontend_lib, encoding="utf-8")
    (root / VIEWS_PATH).write_text(views, encoding="utf-8")
    for relative, contents in (frontend_files or {}).items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(contents, encoding="utf-8")
    script_src = root / "crates" / "script" / "src"
    script_src.mkdir()
    (root / "crates" / "script" / "tests").mkdir()
    fixture_files = dict(script_files or {})
    fixture_files.setdefault("crates/script/src/lib.rs", "")
    for relative, contents in fixture_files.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(contents, encoding="utf-8")
    for relative in inventories.SCRIPT_AUX_RELATIVE:
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("// generated fixture input\n", encoding="utf-8")
    inventory = {
        "schema_version": 1,
        "seam_types": [
            *(_seam(symbol, classification) for symbol, classification in seam_types),
            *additional_seam_types,
        ],
        "violations": list(violations),
        "test_exceptions": list(test_exceptions),
    }
    inventory_path = root / inventories.INVENTORY_RELATIVE
    inventory_path.parent.mkdir(parents=True, exist_ok=True)
    inventory_path.write_text(json.dumps(inventory, indent=2) + "\n", encoding="utf-8")
    return root


class InventoryBehaviorTests(unittest.TestCase):
    def fixture(self, **kwargs: Any) -> Path:
        temporary = tempfile.TemporaryDirectory(prefix="f0-inventory-")
        self.addCleanup(temporary.cleanup)
        return _write_fixture(Path(temporary.name), **kwargs)

    @staticmethod
    def baseline_views() -> str:
        return """\
#[derive(Clone, Copy)]
pub struct FleetCounts {
    pub total: u64,
}

pub struct FleetView<'a> {
    pub label: &'a str,
}
"""

    def test_clean_scalar_and_borrowed_seams_are_accepted(self) -> None:
        root = self.fixture(
            views=self.baseline_views(),
            seam_types=(("FleetCounts", "scalar"), ("FleetView", "borrowed")),
        )
        self.assertEqual(inventories.check_tree(root), [])

    def test_value_reexports_and_module_self_import_are_not_seam_types(self) -> None:
        log_file_path = "crates/frontend-core/src/log_file.rs"
        root = self.fixture(
            views=self.baseline_views(),
            frontend_lib=(
                "pub mod views;\n"
                "pub mod map_bake;\n"
                "mod log_file;\n"
                "pub use map_bake::{MAP_BAKE_TITLE, MAP_BAKE_WARNING, "
                "MAP_BAKE_RENDER, bake};\n"
                "pub use log_file::{self};\n"
            ),
            frontend_files={
                "crates/frontend-core/src/map_bake.rs": (
                    'pub const MAP_BAKE_TITLE: &str = "Map";\n'
                    'pub static MAP_BAKE_WARNING: &str = "warning";\n'
                    "pub fn MAP_BAKE_RENDER() {}\n"
                    "pub fn bake() {}\n"
                ),
                log_file_path: (
                    "use std::time::{self, Duration};\n"
                    "#[derive(Clone, Copy)]\n"
                    "pub struct LogClock { tick: time::Duration }\n"
                    "pub const LOG_FILE_BUFFER_SIZE: usize = 0;\n"
                ),
            },
            seam_types=(("FleetCounts", "scalar"), ("FleetView", "borrowed")),
            additional_seam_types=(_seam("LogClock", "scalar", log_file_path),),
        )
        errors = inventories.check_tree(root)
        print(f"GOOD value re-exports and module self-import: {errors}")
        self.assertEqual(errors, [])

    def test_registered_foreign_reexports_are_in_the_seam_inventory(self) -> None:
        frontend_paths = {
            "capacity": "crates/frontend-core/src/capacity.rs",
            "log": "crates/frontend-core/src/log.rs",
            "walk_permissions": "crates/frontend-core/src/walk_permissions.rs",
        }
        root = self.fixture(
            views=self.baseline_views(),
            frontend_lib=(
                "pub mod views;\n"
                "pub mod capacity;\n"
                "pub mod log;\n"
                "pub mod walk_permissions;\n"
            ),
            frontend_files={
                frontend_paths["capacity"]: (
                    "pub use host::{CapacityWarning, RenderPolicyLabel};\n"
                ),
                frontend_paths["log"]: (
                    "pub use api::hostlog::{Level, Record, Source};\n"
                ),
                frontend_paths["walk_permissions"]: (
                    "pub use host_play::DangerLevel;\n"
                ),
            },
            seam_types=(("FleetCounts", "scalar"), ("FleetView", "borrowed")),
            additional_seam_types=(
                _seam("CapacityWarning", "scalar", frontend_paths["capacity"]),
                _seam("RenderPolicyLabel", "scalar", frontend_paths["capacity"]),
                _seam("Level", "scalar", frontend_paths["log"]),
                _seam("Record", "borrowed", frontend_paths["log"]),
                _seam("Source", "scalar", frontend_paths["log"]),
                _seam(
                    "DangerLevel", "scalar", frontend_paths["walk_permissions"]
                ),
            ),
        )
        errors = inventories.check_tree(root)
        print(f"GOOD trusted foreign seam re-exports: {errors}")
        self.assertEqual(errors, [])

    def test_glob_imports_close_local_ownership_through_foreign_types(self) -> None:
        frontend_paths = {
            "operations": "crates/frontend-core/src/operations.rs",
            "session": "crates/frontend-core/src/session.rs",
        }
        root = self.fixture(
            views=self.baseline_views(),
            frontend_lib=(
                "pub mod views;\n"
                "pub mod operations;\n"
                "pub mod session;\n"
            ),
            frontend_files={
                frontend_paths["operations"]: (
                    "#[derive(Clone, Copy)]\n"
                    "pub struct OperationId(u64);\n"
                ),
                frontend_paths["session"]: (
                    "use crate::operations::OperationId;\n"
                    "use std::thread::JoinHandle;\n"
                    "use std::sync::atomic::{AtomicBool, AtomicU64};\n"
                    "mod native_settings {\n"
                    "    use super::*;\n"
                    "    pub(crate) struct NativeCopyCancellation {\n"
                    "        operation: OperationId,\n"
                    "        state: std::sync::Arc<()>,\n"
                    "    }\n"
                    "}\n"
                    "pub struct SessionOwner {\n"
                    "    cancellation: native_settings::NativeCopyCancellation,\n"
                    "    writer: JoinHandle<()>,\n"
                    "    catalog: script::CatalogDiff,\n"
                    "    cancelled: AtomicBool,\n"
                    "    generation: AtomicU64,\n"
                    "    card: script::CompiledId,\n"
                    "}\n"
                ),
            },
            seam_types=(("FleetCounts", "scalar"), ("FleetView", "borrowed")),
            additional_seam_types=(
                _seam("OperationId", "scalar", frontend_paths["operations"]),
                _seam("SessionOwner", "owning", frontend_paths["session"]),
            ),
        )
        errors = inventories.check_tree(root)
        print(f"GOOD glob-import and foreign ownership closure: {errors}")
        self.assertEqual(errors, [])

    def test_cfg_import_alternatives_collapse_or_fail_closed(self) -> None:
        seam_types = (
            ("FleetCounts", "scalar"),
            ("FleetView", "borrowed"),
            ("CfgClock", "scalar"),
        )
        same_target = self.fixture(
            views=self.baseline_views()
            + (
                "\n#[cfg(unix)]\nuse std::time::Duration as ClockValue;\n"
                "#[cfg(windows)]\nuse std::time::Duration as ClockValue;\n"
                "#[derive(Clone, Copy)]\npub struct CfgClock { delay: ClockValue }\n"
            ),
            seam_types=seam_types,
        )
        different_targets = self.fixture(
            views=self.baseline_views()
            + (
                "\n#[cfg(unix)]\nuse std::time::Duration as ClockValue;\n"
                "#[cfg(windows)]\nuse std::time::Instant as ClockValue;\n"
                "#[derive(Clone, Copy)]\npub struct CfgClock { delay: ClockValue }\n"
            ),
            seam_types=seam_types,
        )
        good_errors = inventories.check_tree(same_target)
        bad_errors = inventories.check_tree(different_targets)
        print(f"GOOD same-target cfg alternatives: {good_errors}")
        print(f"BAD ambiguous cfg alternatives: {bad_errors}")
        self.assertEqual(good_errors, [])
        self.assertTrue(
            any("ambiguous cfg/alternative seam field type" in error for error in bad_errors)
        )

    def test_transition_reports_new_owning_view_and_its_violations(self) -> None:
        base_temp = tempfile.TemporaryDirectory(prefix="f0-inventory-base-")
        candidate_temp = tempfile.TemporaryDirectory(prefix="f0-inventory-candidate-")
        self.addCleanup(base_temp.cleanup)
        self.addCleanup(candidate_temp.cleanup)
        base = _write_fixture(
            Path(base_temp.name),
            views=self.baseline_views(),
            seam_types=(("FleetCounts", "scalar"), ("FleetView", "borrowed")),
        )
        field_violation = _record(
            "owning_public_field",
            VIEWS_PATH,
            "FleetRow::name",
            "production",
            "public_field_of_owning_view",
            owner="T3",
        )
        clone_violation = _record(
            "owning_derived_clone",
            VIEWS_PATH,
            "FleetRow",
            "production",
            "derived_Clone_on_owning_view",
            owner="T3",
        )
        candidate = _write_fixture(
            Path(candidate_temp.name),
            views=self.baseline_views()
            + "\n#[derive(Clone)]\npub struct FleetRow { pub name: String }\n",
            seam_types=(
                ("FleetCounts", "scalar"),
                ("FleetView", "borrowed"),
                ("FleetRow", "owning"),
            ),
            violations=(field_violation, clone_violation),
        )

        findings = inventories.check_transition(base, candidate)
        self.assertIn(f"new seam type: {VIEWS_PATH}::FleetRow", findings)
        self.assertTrue(any("FleetRow::name" in finding for finding in findings))
        self.assertTrue(any("owning_derived_clone" in finding for finding in findings))

    def test_unregistered_public_field_and_clone_debt_fails_closed(self) -> None:
        root = self.fixture(
            views=self.baseline_views()
            + "\n#[derive(Clone)]\npub struct FleetRow { pub name: String }\n",
            seam_types=(
                ("FleetCounts", "scalar"),
                ("FleetView", "borrowed"),
                ("FleetRow", "owning"),
            ),
        )
        errors = inventories.check_tree(root)
        self.assertTrue(any("unregistered inventory violation" in error for error in errors))

    def test_transition_tracks_raw_nav_import_and_public_signature_leak(self) -> None:
        base_temp = tempfile.TemporaryDirectory(prefix="f0-nav-base-")
        candidate_temp = tempfile.TemporaryDirectory(prefix="f0-nav-candidate-")
        self.addCleanup(base_temp.cleanup)
        self.addCleanup(candidate_temp.cleanup)
        views = self.baseline_views()
        base = _write_fixture(
            Path(base_temp.name),
            views=views,
            seam_types=(("FleetCounts", "scalar"), ("FleetView", "borrowed")),
        )
        nav_path = "crates/script/src/lib.rs"
        import_violation = _record(
            "production_nav_implementation_import",
            nav_path,
            "nav::world::WorldState",
            "production",
            "use_import",
        )
        exposure = _record(
            "transitive_raw_representation_exposure",
            nav_path,
            "leak:nav::world::WorldState",
            "production",
            "public_signature",
        )
        candidate = _write_fixture(
            Path(candidate_temp.name),
            views=views,
            script_files={
                nav_path: "use nav::world::WorldState;\n"
                "pub fn leak() -> WorldState { loop {} }\n"
            },
            seam_types=(("FleetCounts", "scalar"), ("FleetView", "borrowed")),
            violations=(import_violation, exposure),
        )

        findings = inventories.check_transition(base, candidate)
        self.assertTrue(any("production_nav_implementation_import" in finding for finding in findings))
        self.assertTrue(any("transitive_raw_representation_exposure" in finding for finding in findings))

    def test_external_test_module_and_cfg_all_keep_test_provenance(self) -> None:
        path = "crates/script/src/tests.rs"
        external_import = _record(
            "test_nav_implementation_import",
            path,
            "nav::world::WorldState",
            "cfg(test)::module:tests",
            "use_import",
        )
        cfg_function = "cfg(test)::module:tests::function:feature_probe"
        macro_area = _record(
            "test_nav_implementation_import",
            path,
            "nav::arrival::ArrivalKind::Area",
            cfg_function,
            "macro_token_path",
        )
        macro_reach = _record(
            "test_nav_implementation_import",
            path,
            "nav::arrival::ArrivalKind::Reach",
            cfg_function,
            "macro_token_path",
        )
        root = self.fixture(
            views=self.baseline_views(),
            script_files={
                "crates/script/src/lib.rs": "#[cfg(test)]\nmod tests;\n",
                path: "use nav::world::WorldState;\n"
                "#[cfg(all(test, feature = \"load\"))]\n"
                "fn feature_probe() {\n"
                "    assert_eq!(nav::arrival::ArrivalKind::Area, "
                "nav::arrival::ArrivalKind::Reach);\n"
                "}\n",
            },
            seam_types=(("FleetCounts", "scalar"), ("FleetView", "borrowed")),
            violations=(external_import, macro_area, macro_reach),
        )
        self.assertEqual(inventories.check_tree(root), [])

    def test_macro_definitions_dynamic_includes_and_cfg_attr_fail_closed(self) -> None:
        macro_root = self.fixture(
            views=self.baseline_views(),
            script_files={
                "crates/script/src/lib.rs": "macro_rules! hidden { () => { nav::world::WorldState }; }\n",
            },
            seam_types=(("FleetCounts", "scalar"), ("FleetView", "borrowed")),
        )
        macro_errors = inventories.check_tree(macro_root)
        self.assertTrue(any("raw navigation path in macro definition" in error for error in macro_errors))

        include_root = self.fixture(
            views=self.baseline_views(),
            script_files={
                "crates/script/src/lib.rs": 'include!(concat!("hidden", ".rs"));\n',
                "crates/script/src/hidden.rs": "pub fn hidden() {}\n",
            },
            seam_types=(("FleetCounts", "scalar"), ("FleetView", "borrowed")),
        )
        include_errors = inventories.check_tree(include_root)
        self.assertTrue(any("include!" in error for error in include_errors))

        cfg_attr_root = self.fixture(
            views=self.baseline_views(),
            script_files={
                "crates/script/src/lib.rs": "#[cfg_attr(feature = \"load\", cfg(test))]\n"
                "mod tests;\n",
                "crates/script/src/tests.rs": "",
            },
            seam_types=(("FleetCounts", "scalar"), ("FleetView", "borrowed")),
        )
        cfg_attr_errors = inventories.check_tree(cfg_attr_root)
        self.assertTrue(any("cfg_attr may not alter cfg" in error for error in cfg_attr_errors))

    def test_trusted_seam_class_cannot_be_rewritten(self) -> None:
        base_temp = tempfile.TemporaryDirectory(prefix="f0-class-base-")
        candidate_temp = tempfile.TemporaryDirectory(prefix="f0-class-candidate-")
        self.addCleanup(base_temp.cleanup)
        self.addCleanup(candidate_temp.cleanup)
        base = _write_fixture(
            Path(base_temp.name),
            views=self.baseline_views(),
            seam_types=(("FleetCounts", "scalar"), ("FleetView", "borrowed")),
        )
        candidate = _write_fixture(
            Path(candidate_temp.name),
            views="#[derive(Clone, Copy)]\npub struct FleetView { pub value: u32 }\n",
            seam_types=(("FleetView", "scalar"),),
        )
        errors = inventories.check_transition(base, candidate)
        self.assertTrue(any("trusted seam classification is borrowed" in error for error in errors))

    def test_new_owning_seam_has_exact_bad_and_good_transitions(self) -> None:
        base_temp = tempfile.TemporaryDirectory(prefix="f0-new-seam-base-")
        bad_temp = tempfile.TemporaryDirectory(prefix="f0-new-seam-bad-")
        good_temp = tempfile.TemporaryDirectory(prefix="f0-new-seam-good-")
        self.addCleanup(base_temp.cleanup)
        self.addCleanup(bad_temp.cleanup)
        self.addCleanup(good_temp.cleanup)
        base = _write_fixture(
            Path(base_temp.name),
            views=self.baseline_views(),
            seam_types=(("FleetCounts", "scalar"), ("FleetView", "borrowed")),
        )
        field = _record(
            "owning_public_field",
            VIEWS_PATH,
            "FleetRow::name",
            "production",
            "public_field_of_owning_view",
            owner="T3",
        )
        clone = _record(
            "owning_derived_clone",
            VIEWS_PATH,
            "FleetRow",
            "production",
            "derived_Clone_on_owning_view",
            owner="T3",
        )
        candidate_views = (
            self.baseline_views()
            + "\n#[derive(Clone)]\npub struct FleetRow { pub name: String }\n"
        )
        bad = _write_fixture(
            Path(bad_temp.name),
            views=candidate_views,
            seam_types=(
                ("FleetCounts", "scalar"),
                ("FleetView", "borrowed"),
                ("FleetRow", "owning"),
            ),
            violations=(field, clone),
        )
        good = _write_fixture(
            Path(good_temp.name),
            views=self.baseline_views(),
            seam_types=(("FleetCounts", "scalar"), ("FleetView", "borrowed")),
        )
        bad_findings = inventories.check_transition(base, bad)
        good_findings = inventories.check_transition(base, good)
        print(f"BAD new owning seam: {bad_findings}")
        print(f"GOOD new owning seam: {good_findings}")
        self.assertTrue(any("new seam type:" in finding for finding in bad_findings))
        self.assertTrue(any("owning_public_field" in finding for finding in bad_findings))
        self.assertTrue(any("owning_derived_clone" in finding for finding in bad_findings))
        self.assertEqual(good_findings, [])

    def test_restored_owning_field_and_clone_are_rejected_after_clean_baseline(self) -> None:
        base_temp = tempfile.TemporaryDirectory(prefix="f0-restored-base-")
        bad_temp = tempfile.TemporaryDirectory(prefix="f0-restored-bad-")
        good_temp = tempfile.TemporaryDirectory(prefix="f0-restored-good-")
        self.addCleanup(base_temp.cleanup)
        self.addCleanup(bad_temp.cleanup)
        self.addCleanup(good_temp.cleanup)
        base_views = self.baseline_views() + "\npub struct FleetRow { name: String }\n"
        base_types = (
            ("FleetCounts", "scalar"),
            ("FleetView", "borrowed"),
            ("FleetRow", "owning"),
        )
        base = _write_fixture(
            Path(base_temp.name),
            views=base_views,
            seam_types=base_types,
        )
        field = _record(
            "owning_public_field",
            VIEWS_PATH,
            "FleetRow::name",
            "production",
            "public_field_of_owning_view",
            owner="T3",
        )
        clone = _record(
            "owning_derived_clone",
            VIEWS_PATH,
            "FleetRow",
            "production",
            "derived_Clone_on_owning_view",
            owner="T3",
        )
        restored = self.baseline_views() + (
            "\n#[derive(Clone)]\npub struct FleetRow { pub name: String }\n"
        )
        bad = _write_fixture(
            Path(bad_temp.name),
            views=restored,
            seam_types=base_types,
            violations=(field, clone),
        )
        good = _write_fixture(
            Path(good_temp.name),
            views=base_views,
            seam_types=base_types,
        )
        removed_findings = inventories.check_transition(bad, base)
        bad_findings = inventories.check_transition(base, bad)
        good_findings = inventories.check_transition(base, good)
        print(f"GOOD accepted field/Clone removal: {removed_findings}")
        print(f"BAD restored owning field and Clone: {bad_findings}")
        print(f"GOOD clean baseline: {good_findings}")
        self.assertEqual(removed_findings, [])
        self.assertTrue(any("owning_public_field" in finding for finding in bad_findings))
        self.assertTrue(any("owning_derived_clone" in finding for finding in bad_findings))
        self.assertEqual(good_findings, [])

    def test_removing_mixed_test_feature_cfg_is_a_new_violation(self) -> None:
        # Review r1: `cfg(any(test, feature = "test-support"))` is not a pure
        # test scope. Its exact text is the violation provenance, so deleting
        # the gate turns registered conditional debt into a new production id.
        gate = 'cfg(any(test,feature="test-support"))'
        conditional = self.baseline_views() + """
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone)]
pub struct Probe {
    pub name: String,
}

pub struct FleetRow {
    label: String,
    #[cfg(any(test, feature = "test-support"))]
    pub fail_for: Option<String>,
}
"""
        promoted = self.baseline_views() + """
#[derive(Clone)]
pub struct Probe {
    pub name: String,
}

pub struct FleetRow {
    label: String,
    pub fail_for: Option<String>,
}
"""
        removed = self.baseline_views() + """
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone)]
pub struct Probe {
    pub name: String,
}

pub struct FleetRow {
    label: String,
}
"""
        seam_types = (
            ("FleetCounts", "scalar"),
            ("FleetView", "borrowed"),
            ("Probe", "owning"),
            ("FleetRow", "owning"),
        )

        def records(cfg: str, *, field: bool = True) -> tuple[dict[str, str], ...]:
            rows = [
                _record("owning_derived_clone", VIEWS_PATH, "Probe", cfg, "derived_Clone_on_owning_view", owner="T3"),
                _record("owning_public_field", VIEWS_PATH, "Probe::name", cfg, "public_field_of_owning_view", owner="T3"),
            ]
            if field:
                rows.append(
                    _record(
                        "owning_public_field",
                        VIEWS_PATH,
                        "FleetRow::fail_for",
                        cfg,
                        "public_field_of_owning_view",
                        owner="T3",
                    )
                )
            return tuple(rows)

        base = self.fixture(views=conditional, seam_types=seam_types, violations=records(gate))
        # The attacker re-registers the same symbols under production ids, so
        # each tree is internally clean; only the transition can see it.
        bad = self.fixture(views=promoted, seam_types=seam_types, violations=records("production"))
        unchanged = self.fixture(views=conditional, seam_types=seam_types, violations=records(gate))
        good_removal = self.fixture(
            views=removed,
            seam_types=seam_types,
            violations=records(gate, field=False),
        )
        self.assertEqual(inventories.check_tree(base), [])
        self.assertEqual(inventories.check_tree(bad), [])
        bad_findings = inventories.check_transition(base, bad)
        print(f"BAD removed cfg(any(test, feature)) gate: {bad_findings}")
        self.assertEqual(
            bad_findings,
            sorted(
                f"new inventory violation: {row['id']}" for row in records("production")
            ),
        )
        unchanged_findings = inventories.check_transition(base, unchanged)
        removal_findings = inventories.check_transition(base, good_removal)
        print(f"GOOD gate kept: {unchanged_findings}; GOOD conditional field removed: {removal_findings}")
        self.assertEqual(unchanged_findings, [])
        self.assertEqual(removal_findings, [])

    def test_inner_module_cfg_gate_removal_is_a_new_production_violation(self) -> None:
        # Review r2: inner `#![cfg(...)]` / `#![cfg_attr(...)]` is exact
        # violation provenance. Deleting the gate promotes conditional debt
        # to production ids, so the transition must report new violations.
        gate = 'cfg(any(test,feature="test-support"))'
        probe_tail = '\n#[derive(Clone)]\npub struct Probe { pub name: String }\n'
        seam_types = (
            ("FleetCounts", "scalar"),
            ("FleetView", "borrowed"),
            ("Probe", "owning"),
        )

        def records(cfg: str) -> tuple[dict[str, str], ...]:
            return (
                _record(
                    "owning_derived_clone",
                    VIEWS_PATH,
                    "Probe",
                    cfg,
                    "derived_Clone_on_owning_view",
                    owner="T3",
                ),
                _record(
                    "owning_public_field",
                    VIEWS_PATH,
                    "Probe::name",
                    cfg,
                    "public_field_of_owning_view",
                    owner="T3",
                ),
            )

        for views_gate, label in (
            ('#![cfg(any(test, feature = "test-support"))]\n', "inner cfg"),
            (
                '#![cfg_attr(any(test, feature = "test-support"), allow(dead_code))]\n',
                "inner cfg_attr",
            ),
        ):
            with self.subTest(gate=label):
                gated_views = views_gate + self.baseline_views() + probe_tail
                promoted_views = self.baseline_views() + probe_tail
                base = self.fixture(
                    views=gated_views, seam_types=seam_types, violations=records(gate)
                )
                promoted = self.fixture(
                    views=promoted_views,
                    seam_types=seam_types,
                    violations=records("production"),
                )
                self.assertEqual(inventories.check_tree(base), [])
                self.assertEqual(inventories.check_tree(promoted), [])
                gated_to_production = inventories.check_transition(base, promoted)
                print(f"BAD removed {label} gate: {gated_to_production}")
                self.assertEqual(
                    gated_to_production,
                    sorted(
                        f"new inventory violation: {row['id']}" for row in records("production")
                    ),
                )
                production_to_gated = inventories.check_transition(promoted, base)
                print(f"BAD added {label} gate: {production_to_gated}")
                self.assertEqual(
                    production_to_gated,
                    sorted(f"new inventory violation: {row['id']}" for row in records(gate)),
                )

    def test_module_level_cfg_on_mod_parses_and_chains_both_directions(self) -> None:
        # Review r2: a non-test `#[cfg]` on `mod views;` must parse instead of
        # crashing in `_module_path_attribute`, and its predicate (plus any
        # inner file gate along the chain) is violation provenance.
        gate = 'cfg(any(test,feature="test-support"))'
        probe_tail = '\n#[derive(Clone)]\npub struct Probe { pub name: String }\n'
        probe_views = self.baseline_views() + probe_tail
        seam_types = (
            ("FleetCounts", "scalar"),
            ("FleetView", "borrowed"),
            ("Probe", "owning"),
        )

        def records(cfg: str) -> tuple[dict[str, str], ...]:
            return (
                _record(
                    "owning_derived_clone",
                    VIEWS_PATH,
                    "Probe",
                    cfg,
                    "derived_Clone_on_owning_view",
                    owner="T3",
                ),
                _record(
                    "owning_public_field",
                    VIEWS_PATH,
                    "Probe::name",
                    cfg,
                    "public_field_of_owning_view",
                    owner="T3",
                ),
            )

        gated_lib = '#[cfg(any(test, feature = "test-support"))]\npub mod views;\n'
        ungated_lib = 'pub mod views;\n'
        base = self.fixture(
            views=probe_views,
            frontend_lib=gated_lib,
            seam_types=seam_types,
            violations=records(gate),
        )
        promoted = self.fixture(
            views=probe_views,
            frontend_lib=ungated_lib,
            seam_types=seam_types,
            violations=records("production"),
        )
        self.assertEqual(inventories.check_tree(base), [])
        self.assertEqual(inventories.check_tree(promoted), [])
        gated_to_production = inventories.check_transition(base, promoted)
        print(f"BAD removed mod cfg gate: {gated_to_production}")
        self.assertEqual(
            gated_to_production,
            sorted(f"new inventory violation: {row['id']}" for row in records("production")),
        )
        production_to_gated = inventories.check_transition(promoted, base)
        print(f"BAD added mod cfg gate: {production_to_gated}")
        self.assertEqual(
            production_to_gated,
            sorted(f"new inventory violation: {row['id']}" for row in records(gate)),
        )
        chained_gate = f"cfg(unix) & {gate}"
        chained_lib = '#[cfg(unix)]\npub mod views;\n'
        chained_views = '#![cfg(any(test, feature = "test-support"))]\n' + probe_views
        chained = self.fixture(
            views=chained_views,
            frontend_lib=chained_lib,
            seam_types=seam_types,
            violations=records(chained_gate),
        )
        self.assertEqual(inventories.check_tree(chained), [])
        chained_to_production = inventories.check_transition(chained, promoted)
        print(f"BAD removed chained mod+inner gates: {chained_to_production}")
        self.assertEqual(
            chained_to_production,
            sorted(f"new inventory violation: {row['id']}" for row in records("production")),
        )
        outer_only = self.fixture(
            views=probe_views,
            frontend_lib=chained_lib,
            seam_types=seam_types,
            violations=records("cfg(unix)"),
        )
        self.assertEqual(inventories.check_tree(outer_only), [])
        chained_to_outer = inventories.check_transition(chained, outer_only)
        print(f"BAD removed chained inner gate only: {chained_to_outer}")
        self.assertEqual(
            chained_to_outer,
            sorted(f"new inventory violation: {row['id']}" for row in records("cfg(unix)")),
        )

    def test_owning_seams_cannot_reclassify_as_borrowed_or_scalar(self) -> None:
        base_temp = tempfile.TemporaryDirectory(prefix="f0-reclass-base-")
        borrowed_temp = tempfile.TemporaryDirectory(prefix="f0-reclass-borrowed-")
        scalar_temp = tempfile.TemporaryDirectory(prefix="f0-reclass-scalar-")
        good_temp = tempfile.TemporaryDirectory(prefix="f0-reclass-good-")
        for temporary in (base_temp, borrowed_temp, scalar_temp, good_temp):
            self.addCleanup(temporary.cleanup)
        base_views = self.baseline_views() + "\npub struct Crossing { pub payload: String }\n"
        base_violation = _record(
            "owning_public_field",
            VIEWS_PATH,
            "Crossing::payload",
            "production",
            "public_field_of_owning_view",
            owner="T3",
        )
        base = _write_fixture(
            Path(base_temp.name),
            views=base_views,
            seam_types=(
                ("FleetCounts", "scalar"),
                ("FleetView", "borrowed"),
                ("Crossing", "owning"),
            ),
            violations=(base_violation,),
        )
        borrowed = _write_fixture(
            Path(borrowed_temp.name),
            views=self.baseline_views()
            + "\npub struct Crossing<'a> { pub payload: &'a str }\n",
            seam_types=(
                ("FleetCounts", "scalar"),
                ("FleetView", "borrowed"),
                ("Crossing", "borrowed"),
            ),
        )
        scalar = _write_fixture(
            Path(scalar_temp.name),
            views=self.baseline_views()
            + "\n#[derive(Clone, Copy)]\npub struct Crossing { pub payload: u64 }\n",
            seam_types=(
                ("FleetCounts", "scalar"),
                ("FleetView", "borrowed"),
                ("Crossing", "scalar"),
            ),
        )
        good = _write_fixture(
            Path(good_temp.name),
            views=base_views,
            seam_types=(
                ("FleetCounts", "scalar"),
                ("FleetView", "borrowed"),
                ("Crossing", "owning"),
            ),
            violations=(base_violation,),
        )
        borrowed_findings = inventories.check_transition(base, borrowed)
        scalar_findings = inventories.check_transition(base, scalar)
        good_findings = inventories.check_transition(base, good)
        print(f"BAD owning-to-borrowed reclassification: {borrowed_findings}")
        print(f"BAD owning-to-scalar reclassification: {scalar_findings}")
        print(f"GOOD unchanged owning classification: {good_findings}")
        self.assertTrue(any("owning->borrowed" in finding for finding in borrowed_findings))
        self.assertTrue(any("owning->scalar" in finding for finding in scalar_findings))
        self.assertEqual(good_findings, [])

    def test_stale_test_import_exception_fails_after_source_removal(self) -> None:
        base_temp = tempfile.TemporaryDirectory(prefix="f0-stale-exception-base-")
        bad_temp = tempfile.TemporaryDirectory(prefix="f0-stale-exception-bad-")
        good_temp = tempfile.TemporaryDirectory(prefix="f0-stale-exception-good-")
        for temporary in (base_temp, bad_temp, good_temp):
            self.addCleanup(temporary.cleanup)
        path = "crates/script/src/gatherer/runner.rs"
        symbol = "nav::collision::WorldCollision"
        scope = "cfg(test)::tests"
        exception = _record(
            "test_import_exception",
            path,
            symbol,
            scope,
            "exact_test_fixture_oracle_import",
        )
        root_module = '#[path = "gatherer/runner.rs"]\nmod runner;\n'
        base_runner = "#[cfg(test)]\nmod tests { use nav::collision::WorldCollision; }\n"
        base = _write_fixture(
            Path(base_temp.name),
            views=self.baseline_views(),
            script_files={
                "crates/script/src/lib.rs": root_module,
                path: base_runner,
            },
            seam_types=(("FleetCounts", "scalar"), ("FleetView", "borrowed")),
            test_exceptions=(exception,),
        )
        bad = _write_fixture(
            Path(bad_temp.name),
            views=self.baseline_views(),
            script_files={
                "crates/script/src/lib.rs": root_module,
                path: "#[cfg(test)]\nmod tests {}\n",
            },
            seam_types=(("FleetCounts", "scalar"), ("FleetView", "borrowed")),
            test_exceptions=(exception,),
        )
        good = _write_fixture(
            Path(good_temp.name),
            views=self.baseline_views(),
            script_files={
                "crates/script/src/lib.rs": root_module,
                path: "#[cfg(test)]\nmod tests {}\n",
            },
            seam_types=(("FleetCounts", "scalar"), ("FleetView", "borrowed")),
        )
        bad_findings = inventories.check_transition(base, bad)
        good_findings = inventories.check_transition(base, good)
        print(f"BAD stale test import exception: {bad_findings}")
        print(f"GOOD removed test import exception: {good_findings}")
        self.assertTrue(any("stale test import exception" in finding for finding in bad_findings))
        self.assertEqual(good_findings, [])

    def test_fully_qualified_nav_paths_and_raw_aliases_are_tracked(self) -> None:
        base_temp = tempfile.TemporaryDirectory(prefix="f0-qualified-nav-base-")
        path_temp = tempfile.TemporaryDirectory(prefix="f0-qualified-nav-path-")
        alias_temp = tempfile.TemporaryDirectory(prefix="f0-qualified-nav-alias-")
        good_temp = tempfile.TemporaryDirectory(prefix="f0-qualified-nav-good-")
        for temporary in (base_temp, path_temp, alias_temp, good_temp):
            self.addCleanup(temporary.cleanup)
        seam_types = (("FleetCounts", "scalar"), ("FleetView", "borrowed"))
        base = _write_fixture(
            Path(base_temp.name),
            views=self.baseline_views(),
            seam_types=seam_types,
        )
        path = "crates/script/src/lib.rs"
        raw_type = "nav::world::WorldState"
        qualified_import = _record(
            "production_nav_implementation_import",
            path,
            raw_type,
            "production",
            "qualified_path",
        )
        function_exposure = _record(
            "transitive_raw_representation_exposure",
            path,
            f"leak:{raw_type}",
            "production",
            "public_signature",
        )
        qualified = _write_fixture(
            Path(path_temp.name),
            views=self.baseline_views(),
            script_files={
                path: "pub fn leak() -> nav::world::WorldState { loop {} }\n"
            },
            seam_types=seam_types,
            violations=(qualified_import, function_exposure),
        )
        alias_exposure = _record(
            "transitive_raw_representation_exposure",
            path,
            f"RawState:{raw_type}",
            "production",
            "public_type_alias",
        )
        alias_function_exposure = _record(
            "transitive_raw_representation_exposure",
            path,
            f"leak:{raw_type}",
            "production",
            "public_signature",
        )
        alias = _write_fixture(
            Path(alias_temp.name),
            views=self.baseline_views(),
            script_files={
                path: "pub type RawState = nav::world::WorldState;\n"
                "pub fn leak() -> RawState { loop {} }\n"
            },
            seam_types=seam_types,
            violations=(
                qualified_import,
                alias_exposure,
                alias_function_exposure,
            ),
        )
        good = _write_fixture(
            Path(good_temp.name),
            views=self.baseline_views(),
            seam_types=seam_types,
        )
        path_findings = inventories.check_transition(base, qualified)
        alias_findings = inventories.check_transition(base, alias)
        good_findings = inventories.check_transition(base, good)
        print(f"BAD fully qualified raw nav type: {path_findings}")
        print(f"BAD raw nav alias leak: {alias_findings}")
        print(f"GOOD no raw nav path: {good_findings}")
        self.assertTrue(any("qualified_path" in finding for finding in path_findings))
        self.assertTrue(any("public_signature" in finding for finding in path_findings))
        self.assertTrue(any("public_type_alias" in finding for finding in alias_findings))
        self.assertTrue(any("public_signature" in finding for finding in alias_findings))
        self.assertEqual(good_findings, [])

    def test_public_module_wrapper_and_alias_are_independent_seams(self) -> None:
        base_temp = tempfile.TemporaryDirectory(prefix="f0-module-seam-base-")
        bad_temp = tempfile.TemporaryDirectory(prefix="f0-module-seam-bad-")
        good_temp = tempfile.TemporaryDirectory(prefix="f0-module-seam-good-")
        for temporary in (base_temp, bad_temp, good_temp):
            self.addCleanup(temporary.cleanup)
        base_types = (("FleetCounts", "scalar"), ("FleetView", "borrowed"))
        base = _write_fixture(
            Path(base_temp.name),
            views=self.baseline_views(),
            seam_types=base_types,
        )
        wrapped_views = self.baseline_views() + (
            "\npub mod wrapper {\n"
            "    pub struct Wrapped<'a> { pub label: &'a str }\n"
            "}\n"
            "pub use wrapper::Wrapped as WrappedAlias;\n"
        )
        bad = _write_fixture(
            Path(bad_temp.name),
            views=wrapped_views,
            seam_types=base_types + (
                ("views::wrapper::Wrapped", "borrowed"),
                ("WrappedAlias", "borrowed"),
            ),
        )
        good = _write_fixture(
            Path(good_temp.name),
            views=self.baseline_views(),
            seam_types=base_types,
        )
        bad_findings = inventories.check_transition(base, bad)
        good_findings = inventories.check_transition(base, good)
        print(f"BAD public module and alias seams: {bad_findings}")
        print(f"GOOD unchanged module seam inventory: {good_findings}")
        self.assertTrue(any("views::wrapper::Wrapped" in finding for finding in bad_findings))
        self.assertTrue(any("WrappedAlias" in finding for finding in bad_findings))
        self.assertEqual(good_findings, [])

    def test_enum_payload_ownership_includes_wrapper_and_mixed_fields(self) -> None:
        base_temp = tempfile.TemporaryDirectory(prefix="f0-enum-base-")
        bad_temp = tempfile.TemporaryDirectory(prefix="f0-enum-bad-")
        good_temp = tempfile.TemporaryDirectory(prefix="f0-enum-good-")
        for temporary in (base_temp, bad_temp, good_temp):
            self.addCleanup(temporary.cleanup)
        base_types = (("FleetCounts", "scalar"), ("FleetView", "borrowed"))
        base = _write_fixture(
            Path(base_temp.name),
            views=self.baseline_views(),
            seam_types=base_types,
        )
        enum_source = self.baseline_views() + (
            "\n#[derive(Clone, Copy)]\n"
            "pub enum ScalarUnit { Empty }\n"
            "#[derive(Clone, Copy)]\n"
            "pub struct ArrayClock(pub [u8; 12]);\n"
            "#[derive(Clone)]\n"
            "pub enum EnumPayloads {\n"
            "    Wrapped(String),\n"
            "    Unit,\n"
            "    Mixed { borrowed: &'static str, owned: Vec<u8> },\n"
            "}\n"
        )
        enum_types = base_types + (
            ("ScalarUnit", "scalar"),
            ("ArrayClock", "scalar"),
            ("EnumPayloads", "owning"),
        )
        violations = (
            _record(
                "owning_public_field",
                VIEWS_PATH,
                "EnumPayloads::Wrapped::0",
                "production",
                "public_field_of_owning_view",
                owner="T3",
            ),
            _record(
                "owning_public_field",
                VIEWS_PATH,
                "EnumPayloads::Mixed::borrowed",
                "production",
                "public_field_of_owning_view",
                owner="T3",
            ),
            _record(
                "owning_public_field",
                VIEWS_PATH,
                "EnumPayloads::Mixed::owned",
                "production",
                "public_field_of_owning_view",
                owner="T3",
            ),
            _record(
                "owning_derived_clone",
                VIEWS_PATH,
                "EnumPayloads",
                "production",
                "derived_Clone_on_owning_view",
                owner="T3",
            ),
        )
        bad = _write_fixture(
            Path(bad_temp.name),
            views=enum_source,
            seam_types=enum_types,
            violations=violations,
        )
        good = _write_fixture(
            Path(good_temp.name),
            views=self.baseline_views(),
            seam_types=base_types,
        )
        bad_findings = inventories.check_transition(base, bad)
        good_findings = inventories.check_transition(base, good)
        candidate_errors = inventories.check_tree(bad)
        print(f"BAD enum seam widening: {bad_findings}")
        print(f"GOOD enum inventory: {candidate_errors}")
        print(f"GOOD unchanged enum baseline: {good_findings}")
        self.assertEqual(candidate_errors, [])
        self.assertTrue(any("EnumPayloads::Wrapped::0" in finding for finding in bad_findings))
        self.assertTrue(any("EnumPayloads::Mixed::borrowed" in finding for finding in bad_findings))
        self.assertTrue(any("EnumPayloads::Mixed::owned" in finding for finding in bad_findings))
        self.assertTrue(any("owning_derived_clone" in finding for finding in bad_findings))
        self.assertFalse(
            any(
                "owning_derived_clone" in finding and "ScalarUnit" in finding
                for finding in bad_findings
            )
        )
        self.assertEqual(good_findings, [])

    def test_marked_walk_is_owning_by_foreign_type_closure(self) -> None:
        fields = (
            "prepared",
            "destination",
            "options",
            "arms",
        )
        violations = tuple(
            _record(
                "owning_public_field",
                VIEWS_PATH,
                f"MarkedWalk::{field}",
                "production",
                "public_field_of_owning_view",
                owner="T3",
            )
            for field in fields
        )
        root = self.fixture(
            views=self.baseline_views()
            + "use host_play::walk_map::{ActionError, MapContext, MapWalkPlan};\n"
            + "use host_play::WalkArms;\n"
            + "use nav::router::FindOptions;\n"
            + "use nav::tile::Tile;\n"
            + "pub struct MarkedWalk<'a> {\n"
            + "    pub prepared: Result<(MapContext, MapWalkPlan), ActionError>,\n"
            + "    pub destination: Option<Tile>,\n"
            + "    pub options: FindOptions,\n"
            + "    pub arms: &'a WalkArms,\n"
            + "}\n"
            + "pub struct BorrowedStatus<'a> {\n"
            + "    pub conditions: &'a api::process_condition::ProcessConditions,\n"
            + "}\n",
            seam_types=(
                ("FleetCounts", "scalar"),
                ("FleetView", "borrowed"),
                ("MarkedWalk", "owning"),
                ("BorrowedStatus", "borrowed"),
            ),
            violations=violations,
        )
        errors = inventories.check_tree(root)
        print(f"GOOD MarkedWalk closure-derived owning classification: {errors}")
        self.assertEqual(errors, [])

    def test_slot_surface_trait_is_not_a_value_seam_but_unions_fail_closed(self) -> None:
        seam_types = (("FleetCounts", "scalar"), ("FleetView", "borrowed"))
        trait_root = self.fixture(
            views=self.baseline_views()
            + (
                "pub trait SlotSurface { type Io; fn released(&mut self); }\n"
                "struct HeadlessSurface;\n"
                "impl SlotSurface for HeadlessSurface {\n"
                "    type Io = ();\n"
                "    fn released(&mut self) {}\n"
                "}\n"
            ),
            frontend_lib="pub mod views;\npub use views::SlotSurface;\n",
            seam_types=seam_types,
        )
        trait_errors = inventories.check_tree(trait_root)
        union_root = self.fixture(
            views=self.baseline_views()
            + "pub union UnsupportedUnion { integer: u32, floating: f32 }\n",
            frontend_lib="pub mod views;\npub use views::UnsupportedUnion;\n",
            seam_types=seam_types,
        )
        union_errors = inventories.check_tree(union_root)
        print(f"GOOD non-value SlotSurface trait: {trait_errors}")
        print(f"BAD unsupported value union: {union_errors}")
        self.assertEqual(trait_errors, [])
        self.assertTrue(
            any("unsupported public seam type syntax (UNION)" in error for error in union_errors)
        )

    def test_candidate_cannot_select_parser_or_change_supported_edition(self) -> None:
        root = self.fixture(
            views=self.baseline_views(),
            seam_types=(("FleetCounts", "scalar"), ("FleetView", "borrowed")),
        )
        inventory_path = root / inventories.INVENTORY_RELATIVE
        raw = json.loads(inventory_path.read_text(encoding="utf-8"))
        raw["parser_version"] = "attacker-selected"
        inventory_path.write_text(json.dumps(raw), encoding="utf-8")
        schema_errors = inventories.check_tree(root)
        self.assertTrue(any("top-level fields must be exactly" in error for error in schema_errors))

        raw.pop("parser_version")
        inventory_path.write_text(json.dumps(raw), encoding="utf-8")
        manifest = root / "crates" / "script" / "Cargo.toml"
        manifest.write_text(manifest.read_text(encoding="utf-8").replace("2021", "2024"), encoding="utf-8")
        edition_errors = inventories.check_tree(root)
        self.assertTrue(any("parser adapter requires edition 2021" in error for error in edition_errors))


if __name__ == "__main__":
    unittest.main()
