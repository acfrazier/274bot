"""Behavioral fixtures for syntax-aware seam and navigation inventories."""

from __future__ import annotations

import hashlib
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
# sha256 of json.dumps(sorted(TRUSTED_FOREIGN_TYPE_CLASSES.items())) at B de48e55a6.
LEGACY_FOREIGN_TABLE_SHA256 = "5365df079759360f3e3e1df5bcf90e0e71940a05c55fa5fe6c1992fbe21bd04a"

F0_API_FOREIGN_TYPES = (
    "api::identity::RequestId",
    "api::identity::Scope",
    "api::identity::Settlement",
    "api::identity::SlotKey",
    "api::lifecycle::IntentCommand",
    "api::lifecycle::IntentGeneration",
    "api::lifecycle::MemoryMode",
    "api::lifecycle::WorldChoice",
    "api::process_condition::ConditionId",
    "api::process_condition::ProcessCapability",
    "api::process_condition::UnavailableTerminal",
)

F0_API_FOREIGN_ALIASES = {
    "PinRequestId": "RequestId",
    "PinScope": "Scope",
    "PinSettlement": "Settlement<u8, u8>",
    "PinSlotKey": "SlotKey",
    "PinIntentCommand": "IntentCommand",
    "PinIntentGeneration": "IntentGeneration",
    "PinMemoryMode": "MemoryMode",
    "PinWorldChoice": "WorldChoice",
    "PinConditionId": "ConditionId",
    "PinProcessCapability": "ProcessCapability",
    "PinUnavailableTerminal": "UnavailableTerminal",
}

F0_API_IMPORTS = (
    "use api::identity::{RequestId, Scope, Settlement, SlotKey};\n"
    "use api::lifecycle::{IntentCommand, IntentGeneration, MemoryMode, WorldChoice};\n"
    "use api::process_condition::{ConditionId, ProcessCapability, UnavailableTerminal};\n"
)

# Structural copies of the F0 API definitions (crates/api at f0-core).  The
# workspace crate supplies these types; only the std type needs a data row.
F0_API_CRATE = {
    "crates/api/src/lib.rs": (
        "pub mod identity;\n"
        "pub mod lifecycle;\n"
        "pub mod process_condition;\n"
        "pub mod selected;\n"
        "#[cfg(test)]\n"
        "mod tests;\n"
    ),
    # Only the production module tree is parsed; this test-only file is not Rust.
    "crates/api/src/tests.rs": "fn broken() }\n",
    "crates/api/src/selected.rs": "#[derive(Clone, Copy)]\npub struct RunKey(u64);\n",
    "crates/api/src/identity.rs": (
        "use std::num::NonZeroU64;\n"
        "use crate::selected::RunKey;\n"
        "#[derive(Clone, Copy)]\npub struct ProcessIncarnation(NonZeroU64);\n"
        "#[derive(Clone, Copy)]\npub struct SlotKey(NonZeroU64);\n"
        "#[derive(Clone, Copy)]\n"
        "pub struct SessionKey {\n    slot: SlotKey,\n    incarnation: ProcessIncarnation,\n}\n"
        "#[derive(Clone, Copy)]\n"
        "pub struct ExecutionId {\n    run: RunKey,\n    sequence: NonZeroU64,\n}\n"
        "#[derive(Clone, Copy)]\npub struct RequestId(NonZeroU64);\n"
        "#[derive(Clone, Copy)]\n"
        "pub enum Scope {\n    Process,\n    Slot(SlotKey),\n    Session(SessionKey),\n"
        "    Run(RunKey),\n    Execution(ExecutionId),\n}\n"
        "#[derive(Clone, Copy)]\npub enum Cancellation {\n    Operator,\n    Superseded,\n}\n"
        "#[derive(Clone)]\n"
        "pub enum Settlement<T, E> {\n    Completed(T),\n    Failed(E),\n"
        "    Cancelled(Cancellation),\n}\n"
    ),
    "crates/api/src/lifecycle.rs": (
        "#[derive(Clone, Copy)]\npub enum MemoryMode {\n    High,\n    Low,\n}\n"
        "#[derive(Clone, Copy)]\npub enum WorldChoice {\n    Automatic,\n    World(u16),\n}\n"
        "#[derive(Clone, Copy)]\npub struct IntentGeneration(u64);\n"
        "#[derive(Clone, Copy)]\n"
        "pub struct IntentCommand {\n    generation: IntentGeneration,\n"
        "    cancellation: IntentGeneration,\n}\n"
    ),
    "crates/api/src/process_condition.rs": (
        "use crate::identity::{ProcessIncarnation, RequestId, Scope};\n"
        "#[derive(Clone, Copy)]\npub enum ProcessServiceCode {\n    Runtime,\n    Store,\n}\n"
        "#[derive(Clone, Copy)]\n"
        "pub enum ProcessCapability {\n    Render,\n    Script,\n    Store,\n    Login,\n}\n"
        "#[derive(Clone, Copy)]\n"
        "pub struct ConditionId {\n    incarnation: ProcessIncarnation,\n"
        "    service: ProcessServiceCode,\n}\n"
        "#[derive(Clone, Copy)]\n"
        "pub struct UnavailableTerminal {\n    condition: ConditionId,\n    request: RequestId,\n"
        "    scope: Scope,\n}\n"
    ),
}


def _foreign(path: str, classification: str = "scalar") -> dict[str, str]:
    return {"path": path, "classification": classification, "reason": "fixture foreign type"}


def _foreign_data(*rows: dict[str, Any]) -> str:
    return json.dumps({"schema_version": 1, "foreign_types": list(rows)}, indent=2) + "\n"


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
    foreign_types: str | None = None,
    workspace_files: dict[str, str] | None = None,
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
    if foreign_types is not None:
        foreign_path = root / inventories.FOREIGN_TYPES_RELATIVE
        foreign_path.parent.mkdir(parents=True, exist_ok=True)
        foreign_path.write_text(foreign_types, encoding="utf-8")
    if workspace_files is not None:
        # A root manifest makes every crates/<name> directory a workspace member.
        extra = sorted({Path(relative).parts[1] for relative in workspace_files})
        members = ", ".join(
            f'"crates/{crate}"' for crate in ["frontend-core", "script", *extra]
        )
        (root / "Cargo.toml").write_text(
            f'[workspace]\nresolver = "2"\nmembers = [{members}]\n', encoding="utf-8"
        )
        for crate in extra:
            manifest = root / "crates" / crate / "Cargo.toml"
            if f"crates/{crate}/Cargo.toml" not in workspace_files:
                manifest.parent.mkdir(parents=True, exist_ok=True)
                manifest.write_text(
                    f'[package]\nname = "{crate}"\nversion = "0.0.0"\nedition = "2021"\n',
                    encoding="utf-8",
                )
        for relative, contents in workspace_files.items():
            path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(contents, encoding="utf-8")
    return root


class InventoryBehaviorTests(unittest.TestCase):
    def fixture(self, **kwargs: Any) -> Path:
        temporary = tempfile.TemporaryDirectory(prefix="f0-inventory-")
        self.addCleanup(temporary.cleanup)
        return _write_fixture(Path(temporary.name), **kwargs)

    def clean_fixture(self, **kwargs: Any) -> Path:
        kwargs.setdefault(
            "seam_types", (("FleetCounts", "scalar"), ("FleetView", "borrowed"))
        )
        return self.fixture(**kwargs)

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

    def test_macro_definitions_dynamic_includes_and_cfg_attr_provenance(self) -> None:
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
        self.assertTrue(
            any("cfg_attr may not alter cfg" in error for error in cfg_attr_errors),
            cfg_attr_errors,
        )
        harmless = self.clean_fixture(
            views=self.baseline_views(),
            script_files={
                "crates/script/src/lib.rs": '#[cfg_attr(feature = "load", allow(dead_code))]\nmod tests;\n',
                "crates/script/src/tests.rs": "",
            },
        )
        self.assertEqual(inventories.check_tree(harmless), [])

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
        # Review r2: inner attributes are exact module provenance. Deleting
        # either changes its registered ids even if cfg_attr only changes a
        # conditional attribute instead of disabling the module.
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

        for views_gate, label, gated_id in (
            ('#![cfg(any(test, feature = "test-support"))]\n', "inner cfg", gate),
            (
                '#![cfg_attr(any(test, feature = "test-support"), allow(dead_code))]\n',
                "inner cfg_attr",
                'cfg_attr(any(test,feature="test-support"),allow(dead_code))',
            ),
        ):
            with self.subTest(gate=label):
                gated_views = views_gate + self.baseline_views() + probe_tail
                promoted_views = self.baseline_views() + probe_tail
                base = self.fixture(
                    views=gated_views, seam_types=seam_types, violations=records(gated_id)
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
                    sorted(f"new inventory violation: {row['id']}" for row in records(gated_id)),
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

    def test_legacy_builtin_foreign_table_is_frozen(self) -> None:
        table = inventories.TRUSTED_FOREIGN_TYPE_CLASSES
        digest = hashlib.sha256(json.dumps(sorted(table.items())).encode("utf-8")).hexdigest()
        self.assertEqual(len(table), 56)
        self.assertEqual(digest, LEGACY_FOREIGN_TABLE_SHA256)
        self.assertFalse(set(F0_API_FOREIGN_TYPES) & set(table))

    def test_f0_api_workspace_types_are_classified_from_definitions(self) -> None:
        aliases = "".join(
            f"pub type {alias} = {target};\n"
            for alias, target in F0_API_FOREIGN_ALIASES.items()
        )
        views = self.baseline_views() + F0_API_IMPORTS + aliases
        base_seams = (("FleetCounts", "scalar"), ("FleetView", "borrowed"))
        # Settlement is Clone but not Copy, so like any local non-Copy type it is owning.
        expected = {
            alias: "owning" if alias == "PinSettlement" else "scalar"
            for alias in F0_API_FOREIGN_ALIASES
        }
        std_row = _foreign("std::num::NonZeroU64")

        def errors(classes: dict[str, str], data: str | None) -> list[str]:
            return inventories.check_tree(
                self.fixture(
                    views=views,
                    seam_types=base_seams + tuple(classes.items()),
                    foreign_types=data,
                    workspace_files=F0_API_CRATE,
                )
            )

        honest = errors(expected, _foreign_data(std_row))
        all_scalar = errors(
            {alias: "scalar" for alias in F0_API_FOREIGN_ALIASES}, _foreign_data(std_row)
        )
        without_std_row = errors(expected, None)
        with_api_rows = errors(
            expected,
            _foreign_data(std_row, *(_foreign(path) for path in F0_API_FOREIGN_TYPES)),
        )
        print(f"GOOD F0 API types classified from definitions: {honest}")
        print(f"BAD Settlement registered scalar: {all_scalar}")
        print(f"BAD std payload without a data row: {without_std_row}")
        print(f"BAD data rows naming F0 API types: {with_api_rows}")
        self.assertEqual(honest, [])
        self.assertEqual(
            all_scalar,
            [
                "seam classification disagrees with syntax: "
                f"{VIEWS_PATH}::PinSettlement is owning, registry says scalar"
            ],
        )
        self.assertIn(
            "crates/api/src/identity.rs::RequestId: unresolved imported seam field type NonZeroU64",
            without_std_row,
        )
        for index, path in enumerate(F0_API_FOREIGN_TYPES, start=1):
            self.assertIn(
                f"foreign types: foreign_types[{index}] names workspace type {path}; "
                "workspace types are classified from their definitions",
                with_api_rows,
            )
        self.assertIn(
            "crates/api/src/identity.rs::RequestId: unresolved imported seam field type NonZeroU64",
            with_api_rows,
        )

    def test_workspace_definition_drift_changes_the_findings(self) -> None:
        seams = (("FleetCounts", "scalar"), ("FleetView", "borrowed"))
        uses = "pub type UsesRequest = api::identity::RequestId;\n"
        copy_id = "#[derive(Clone, Copy)]\npub struct RequestId(u64);\n"

        def tree(identity: str, views: str, extra: tuple[tuple[str, str], ...]) -> Path:
            return self.fixture(
                views=self.baseline_views() + views,
                seam_types=seams + extra,
                workspace_files={
                    "crates/api/src/lib.rs": "pub mod identity;\n",
                    "crates/api/src/identity.rs": identity,
                },
            )

        before = tree(copy_id, "", ())
        approved = tree(copy_id, uses, (("UsesRequest", "scalar"),))
        self.assertEqual(inventories.check_tree(approved), [])
        growth = inventories.check_transition(before, approved)
        print(f"BAD new workspace type at a seam (no data row): {growth}")
        self.assertEqual(growth, [f"new seam type: {VIEWS_PATH}::UsesRequest"])
        same_class = tree(
            "#[derive(Clone, Copy)]\npub struct RequestId(u32);\n", uses, (("UsesRequest", "scalar"),)
        )
        self.assertEqual(inventories.check_transition(approved, same_class), [])
        drifts = {
            "owned field": "#[derive(Clone)]\npub struct RequestId(Vec<u8>);\n",
            "alias retarget": "pub type RequestId = Vec<u8>;\n",
        }
        for label, identity in drifts.items():
            with self.subTest(drift=label):
                stale_registry = inventories.check_tree(
                    tree(identity, uses, (("UsesRequest", "scalar"),))
                )
                updated = tree(identity, uses, (("UsesRequest", "owning"),))
                self.assertEqual(inventories.check_tree(updated), [])
                findings = inventories.check_transition(approved, updated)
                print(f"BAD {label} drift with the old registry: {stale_registry}")
                print(f"BAD {label} drift after approval: {findings}")
                self.assertEqual(
                    stale_registry,
                    [
                        "seam classification disagrees with syntax: "
                        f"{VIEWS_PATH}::UsesRequest is owning, registry says scalar"
                    ],
                )
                self.assertEqual(
                    findings,
                    [f"seam classification changed: {VIEWS_PATH}::UsesRequest scalar->owning"],
                )

    def test_workspace_paths_resolve_to_the_defining_item(self) -> None:
        api_lib = (
            "pub mod identity;\n"
            "pub mod prelude {\n    pub use crate::identity::RequestId as Rid;\n}\n"
            "pub use identity::RequestId;\n"
            "pub type AliasId = identity::RequestId;\n"
            "mod chain_a {\n    pub use super::chain_b::Hop;\n}\n"
            "mod chain_b {\n    pub use crate::identity::RequestId as Hop;\n}\n"
            "pub use chain_a::Hop;\n"
            '#[path = "../generated/ids.rs"]\n'
            "pub mod generated;\n"
        )
        # A crate type sharing a baseline frontend name, and an explicit Clone in
        # another crate, must not touch frontend-core's own rules.
        api_extra = (
            "pub struct Phase(String);\n"
            "pub struct Holder(String);\n"
            "impl Clone for Holder {\n"
            "    fn clone(&self) -> Self {\n"
            "        Holder(self.0.clone())\n"
            "    }\n"
            "}\n"
        )
        views = self.baseline_views() + (
            "use api::identity::RequestId as Renamed;\n"
            "pub type ViaRenamed = Renamed;\n"
            "pub type ViaReexport = api::RequestId;\n"
            "pub type ViaPrelude = api::prelude::Rid;\n"
            "pub type ViaAlias = api::AliasId;\n"
            "pub type ViaChain = api::Hop;\n"
            "pub type ViaPathModule = api::generated::GeneratedId;\n"
            "pub use api::identity::RequestId as ExportedId;\n"
            "pub type ViaPhase = api::identity::Phase;\n"
            "pub struct Holder {\n    inner: String,\n}\n"
        )
        symbols = (
            "ExportedId",
            "ViaAlias",
            "ViaChain",
            "ViaPathModule",
            "ViaPrelude",
            "ViaReexport",
            "ViaRenamed",
        )
        fixed = (
            ("FleetCounts", "scalar"),
            ("FleetView", "borrowed"),
            ("ViaPhase", "owning"),
            ("Holder", "owning"),
        )
        definitions = {
            "scalar": "#[derive(Clone, Copy)]\npub struct RequestId(u64);\n",
            "owning": "#[derive(Clone)]\npub struct RequestId(String);\n",
        }
        for classification, identity in definitions.items():
            with self.subTest(definition=classification):
                other = "owning" if classification == "scalar" else "scalar"
                results = {}
                for registered in (classification, other):
                    results[registered] = inventories.check_tree(
                        self.fixture(
                            views=views,
                            seam_types=fixed + tuple((symbol, registered) for symbol in symbols),
                            workspace_files={
                                "crates/api/src/lib.rs": api_lib,
                                "crates/api/src/identity.rs": identity + api_extra,
                                "crates/api/generated/ids.rs": (
                                    "pub use crate::identity::RequestId as GeneratedId;\n"
                                ),
                            },
                        )
                    )
                print(f"GOOD workspace paths, {classification} definition: {results[classification]}")
                print(f"BAD workspace paths registered {other}: {results[other]}")
                self.assertEqual(results[classification], [])
                self.assertEqual(
                    results[other],
                    [
                        "seam classification disagrees with syntax: "
                        f"{VIEWS_PATH}::{symbol} is {classification}, registry says {other}"
                        for symbol in symbols
                    ],
                )

    def test_workspace_resolution_fails_closed(self) -> None:
        seams = (("FleetCounts", "scalar"), ("FleetView", "borrowed"), ("Uses", "scalar"))
        api_manifest = '[package]\nname = "api"\nversion = "0.0.0"\nedition = "2024"\n'
        cases: dict[str, tuple[dict[str, str], str, str]] = {
            "use cycle": (
                {
                    "crates/api/src/lib.rs": (
                        "mod a {\n    pub use super::b::Loop;\n}\n"
                        "mod b {\n    pub use super::a::Loop;\n}\n"
                        "pub use a::Loop;\n"
                    )
                },
                "api::Loop",
                f"{VIEWS_PATH}::Uses: unresolved",
            ),
            "type cycle": (
                {
                    "crates/api/src/lib.rs": (
                        "#[derive(Clone, Copy)]\npub struct Ping {\n    pong: Pong,\n}\n"
                        "#[derive(Clone, Copy)]\npub struct Pong {\n    ping: Ping,\n}\n"
                    )
                },
                "api::Ping",
                "recursive type ownership cannot be resolved",
            ),
            "missing item": (
                {"crates/api/src/lib.rs": "pub mod identity;\n", "crates/api/src/identity.rs": ""},
                "api::identity::Missing",
                f"{VIEWS_PATH}::Uses: unresolved public seam field type api::identity::Missing",
            ),
            "macro in consulted module": (
                {
                    "crates/api/src/lib.rs": "pub mod identity;\n",
                    "crates/api/src/identity.rs": (
                        "macro_rules! id {\n    ($name:ident) => {\n"
                        "        pub struct $name(String);\n    };\n}\n"
                        "id!(Hidden);\n"
                        "#[derive(Clone, Copy)]\npub struct RequestId(u64);\n"
                    ),
                },
                "api::identity::RequestId",
                "crates/api/src/identity.rs: item-generating macro syntax is unsupported "
                "(consulted for a seam field type)",
            ),
            "pre-2018 crate-relative path": (
                {
                    "crates/api/src/lib.rs": "pub mod identity;\npub mod selected;\n",
                    "crates/api/src/selected.rs": "#[derive(Clone, Copy)]\npub struct RunKey(u64);\n",
                    "crates/api/src/identity.rs": (
                        "#[derive(Clone, Copy)]\npub struct RequestId(selected::RunKey);\n"
                    ),
                },
                "api::identity::RequestId",
                "crates/api/src/identity.rs::RequestId: unresolved public seam field type "
                "selected::RunKey",
            ),
            "unlisted std payload": (
                {
                    "crates/api/src/lib.rs": (
                        "#[derive(Clone, Copy)]\npub struct RequestId(std::num::NonZeroU32);\n"
                    )
                },
                "api::RequestId",
                "crates/api/src/lib.rs::RequestId: unresolved public seam field type "
                "std::num::NonZeroU32",
            ),
            "unparseable module": (
                {
                    "crates/api/src/lib.rs": "pub mod identity;\n",
                    "crates/api/src/identity.rs": "pub struct RequestId(u64) }\n",
                },
                "api::identity::RequestId",
                f"{VIEWS_PATH}::Uses: workspace crate api: crates/api/src/identity.rs: "
                "unsupported or malformed Rust syntax",
            ),
            "no library target": (
                {"crates/tool/src/main.rs": "fn main() {}\n"},
                "tool::Thing",
                f"{VIEWS_PATH}::Uses: workspace crate tool has no library target",
            ),
            "unsupported edition": (
                {
                    "crates/api/Cargo.toml": api_manifest,
                    "crates/api/src/lib.rs": "#[derive(Clone, Copy)]\npub struct RequestId(u64);\n",
                },
                "api::RequestId",
                f"{VIEWS_PATH}::Uses: crates/api/Cargo.toml: parser adapter requires edition 2021",
            ),
        }
        for label, (files, target, expected) in cases.items():
            with self.subTest(case=label):
                errors = inventories.check_tree(
                    self.fixture(
                        views=self.baseline_views() + f"pub type Uses = {target};\n",
                        seam_types=seams,
                        workspace_files=files,
                    )
                )
                print(f"BAD workspace resolution ({label}): {errors}")
                self.assertTrue(any(expected in error for error in errors), errors)
                self.assertIn(f"stale seam type classification: {VIEWS_PATH}::Uses", errors)

    def test_builtin_entry_keeps_precedence_over_workspace_definition(self) -> None:
        # Residual, by design: a frozen built-in path is trusted as written even
        # when the workspace definition says otherwise.
        root = self.fixture(
            views=self.baseline_views()
            + "pub type Diff = script::CatalogDiff;\npub type Compiled = script::CompiledId;\n",
            seam_types=(
                ("FleetCounts", "scalar"),
                ("FleetView", "borrowed"),
                ("Diff", "owning"),
                ("Compiled", "scalar"),
            ),
            script_files={
                "crates/script/src/lib.rs": (
                    "#[derive(Clone, Copy)]\npub struct CatalogDiff(u8);\n"
                    "pub struct CompiledId(pub &'static str);\n"
                )
            },
            workspace_files={},
        )
        errors = inventories.check_tree(root)
        print(f"GOOD built-in precedence over workspace definitions: {errors}")
        self.assertEqual(errors, [])

    def test_workspace_manifest_is_read_strictly(self) -> None:
        cases = {
            "glob member": ('[workspace]\nmembers = ["crates/*"]\n', "unsupported workspace member crates/*"),
            "escaping member": ('[workspace]\nmembers = ["../outside"]\n', "unsupported workspace member ../outside"),
            "non-string member": ("[workspace]\nmembers = [7]\n", "workspace members must be an array of strings"),
            "missing member manifest": (
                '[workspace]\nmembers = ["crates/absent"]\n',
                "crates/absent/Cargo.toml must be a regular file",
            ),
            "invalid TOML": ("[workspace\n", "workspace crates: "),
        }
        for label, (manifest, expected) in cases.items():
            with self.subTest(case=label):
                root = self.fixture(
                    views=self.baseline_views(),
                    seam_types=(("FleetCounts", "scalar"), ("FleetView", "borrowed")),
                )
                (root / "Cargo.toml").write_text(manifest, encoding="utf-8")
                errors = inventories.check_tree(root)
                print(f"BAD workspace manifest ({label}): {errors}")
                self.assertTrue(
                    any(error.startswith("workspace crates: ") and expected in error for error in errors),
                    errors,
                )

    def test_std_leaf_names_defined_in_the_workspace_are_classified_from_definitions(self) -> None:
        # Wonder eprime-r3 F1: a workspace type named like a std scalar or
        # transparent container is classified from its definition.  Its name
        # alone never skips the payload.
        names = sorted(
            set(inventories.STD_TRANSPARENT_TYPE_MODULES) | inventories.KNOWN_SCALAR_TYPES
        )
        self.assertEqual(len(names), 24)
        seams = (("FleetCounts", "scalar"), ("FleetView", "borrowed"), ("AttackView", "borrowed"))
        disagrees = (
            f"seam classification disagrees with syntax: {VIEWS_PATH}::AttackView "
            "is owning, registry says borrowed"
        )

        def holder(payload: str) -> str:
            return (
                "#[derive(Clone)]\npub struct AttackView {\n"
                f"    pub label: &'static str,\n    pub payload: {payload},\n}}\n"
            )

        for name in names:
            for spelling, (imports, payload) in {
                "qualified": ("", f"api::{name}"),
                "imported": (f"use api::{name};\n", name),
                "glob": ("use api::*;\n", name),
            }.items():
                with self.subTest(name=name, spelling=spelling):
                    errors = inventories.check_tree(
                        self.fixture(
                            views=imports + self.baseline_views() + holder(payload),
                            seam_types=seams,
                            workspace_files={
                                "crates/api/src/lib.rs": (
                                    f"#[derive(Clone)]\npub struct {name}(pub Vec<u8>);\n"
                                )
                            },
                        )
                    )
                    self.assertIn(disagrees, errors)
        wrapper = "#[derive(Clone)]\npub struct Wrapping(pub Vec<u8>);\n"
        shapes: dict[str, tuple[str, str, str, str]] = {
            "enum": (
                "#[derive(Clone)]\npub enum Option {\n    Some(Vec<u8>),\n    None,\n}\n",
                "",
                "api::Option",
                disagrees,
            ),
            "type alias": ("pub type Wrapping = Vec<u8>;\n", "", "api::Wrapping", disagrees),
            "renamed import": (
                "#[derive(Clone)]\npub struct Payload(pub Vec<u8>);\n",
                "use api::Payload as Wrapping;\n",
                "Wrapping",
                disagrees,
            ),
            "glob import": (wrapper, "use api::*;\n", "Wrapping", disagrees),
            "local definition": (
                wrapper,
                "#[derive(Clone)]\npub struct Option(pub Vec<u8>);\n",
                "Option",
                disagrees,
            ),
            "generic argument": (wrapper, "", "Option<api::Wrapping>", disagrees),
            "union": (
                "pub union Wrapping {\n    pub bytes: std::mem::ManuallyDrop<Vec<u8>>,\n}\n",
                "",
                "api::Wrapping",
                "crates/api/src/lib.rs: unsupported public seam type syntax (UNION)",
            ),
        }
        for label, (api, imports, payload, expected) in shapes.items():
            with self.subTest(shape=label):
                errors = inventories.check_tree(
                    self.fixture(
                        views=imports + self.baseline_views() + holder(payload),
                        seam_types=seams,
                        workspace_files={"crates/api/src/lib.rs": api},
                    )
                )
                print(f"BAD std-named workspace payload ({label}): {errors}")
                self.assertIn(expected, errors)
        # A generic parameter is a one-segment path: `api::T` is the workspace
        # type even inside an item with a parameter named `T`.
        generic = inventories.check_tree(
            self.fixture(
                views=self.baseline_views()
                + "#[derive(Clone)]\npub struct AttackView<T> {\n"
                "    pub label: &'static str,\n    pub payload: api::T,\n    pub extra: Option<T>,\n}\n",
                seam_types=seams,
                workspace_files={
                    "crates/api/src/lib.rs": "#[derive(Clone)]\npub struct T(pub Vec<u8>);\n"
                },
            )
        )
        print(f"BAD workspace path ending in a generic parameter name: {generic}")
        self.assertIn(disagrees, generic)

    def test_std_leaf_items_keep_the_shortcut_and_their_generic_arguments(self) -> None:
        views = (
            "use std::cmp;\nuse std::num::Wrapping;\n"
            + self.baseline_views()
            + "#[derive(Clone, Copy)]\n"
            "pub struct StdLeaves {\n"
            "    pub a: Option<u64>,\n"
            "    pub b: Result<u64, bool>,\n"
            "    pub c: Wrapping<u64>,\n"
            "    pub d: std::num::Wrapping<u8>,\n"
            "    pub e: core::num::Wrapping<u8>,\n"
            "    pub f: cmp::Reverse<u64>,\n"
            "    pub g: std::marker::PhantomData<u64>,\n"
            "    pub h: core::task::Poll<u64>,\n"
            "    pub i: std::ops::ControlFlow<u64, u8>,\n"
            "    pub j: std::primitive::u64,\n"
            "    pub k: char,\n"
            "}\n"
            "pub struct StdOwning {\n"
            "    wrapped: Option<api::Wrapper>,\n"
            "    bytes: std::num::Wrapping<Vec<u8>>,\n"
            "}\n"
        )
        root = self.fixture(
            views=views,
            seam_types=(
                ("FleetCounts", "scalar"),
                ("FleetView", "borrowed"),
                ("StdLeaves", "scalar"),
                ("StdOwning", "owning"),
            ),
            workspace_files={
                "crates/api/src/lib.rs": "#[derive(Clone)]\npub struct Wrapper(pub Vec<u8>);\n"
            },
        )
        errors = inventories.check_tree(root)
        print(f"GOOD std leaf items: {errors}")
        self.assertEqual(errors, [])

    def test_leaf_names_that_are_not_the_std_item_fail_closed(self) -> None:
        seams = (("FleetCounts", "scalar"), ("FleetView", "borrowed"), ("Uses", "scalar"))
        cases = {
            "non-workspace crate": (
                "",
                "other::Wrapping<u64>",
                f"{VIEWS_PATH}::Uses: unresolved public seam field type other::Wrapping",
            ),
            "non-std import": (
                "use other::Option;\n",
                "Option<u64>",
                f"{VIEWS_PATH}::Uses: unresolved imported seam field type Option",
            ),
            "non-prelude name without import": (
                "",
                "Wrapping<u64>",
                f"{VIEWS_PATH}::Uses: unresolved public seam field type Wrapping",
            ),
            "prelude name under an unindexed glob": (
                "use other::*;\n",
                "u64",
                f"{VIEWS_PATH}::Uses: unresolved public seam field type u64",
            ),
            "crate-relative name": (
                "",
                "crate::Option<u64>",
                f"{VIEWS_PATH}::Uses: unresolved public seam field type crate::Option",
            ),
        }
        for label, (imports, target, expected) in cases.items():
            with self.subTest(case=label):
                errors = inventories.check_tree(
                    self.fixture(
                        views=imports + self.baseline_views() + f"pub type Uses = {target};\n",
                        seam_types=seams,
                    )
                )
                print(f"BAD non-std leaf path ({label}): {errors}")
                self.assertIn(expected, errors)
                self.assertIn(f"stale seam type classification: {VIEWS_PATH}::Uses", errors)
        generic = inventories.check_tree(
            self.fixture(
                views=self.baseline_views()
                + "#[derive(Clone, Copy)]\npub struct Held<Wrapping = Vec<u8>>(Wrapping);\n",
                seam_types=(("FleetCounts", "scalar"), ("FleetView", "borrowed"), ("Held", "scalar")),
            )
        )
        print(f"BAD generic parameter named like a std leaf: {generic}")
        self.assertIn(f"{VIEWS_PATH}::Held: unresolved generic field ownership (Wrapping)", generic)

    def test_transition_into_a_std_named_workspace_payload_is_not_silent(self) -> None:
        api = (
            "#[derive(Clone, Copy)]\npub struct Plain(pub u64);\n"
            "#[derive(Clone)]\npub struct Wrapping(pub Vec<u8>);\n"
        )

        def tree(payload: str, registered: str, violations: tuple[dict[str, str], ...] = ()) -> Path:
            temporary = tempfile.TemporaryDirectory(prefix="f0-std-leaf-")
            self.addCleanup(temporary.cleanup)
            return _write_fixture(
                Path(temporary.name),
                views=self.baseline_views()
                + "#[derive(Clone)]\npub struct AttackView {\n"
                f"    pub label: &'static str,\n    pub payload: api::{payload},\n}}\n",
                seam_types=(
                    ("FleetCounts", "scalar"),
                    ("FleetView", "borrowed"),
                    ("AttackView", registered),
                ),
                violations=violations,
                workspace_files={"crates/api/src/lib.rs": api},
            )

        base = tree("Plain", "borrowed")
        self.assertEqual(inventories.check_tree(base), [])
        unregistered = inventories.check_transition(base, tree("Wrapping", "borrowed"))
        print(f"BAD plain -> Wrapping(Vec) with the old registry: {unregistered}")
        self.assertIn(
            f"candidate: seam classification disagrees with syntax: {VIEWS_PATH}::AttackView "
            "is owning, registry says borrowed",
            unregistered,
        )
        violations = (
            _record(
                "owning_derived_clone",
                VIEWS_PATH,
                "AttackView",
                "production",
                "derived_Clone_on_owning_view",
                owner="T3",
            ),
            *(
                _record(
                    "owning_public_field",
                    VIEWS_PATH,
                    f"AttackView::{field}",
                    "production",
                    "public_field_of_owning_view",
                    owner="T3",
                )
                for field in ("label", "payload")
            ),
        )
        reregistered = inventories.check_transition(
            base, tree("Wrapping", "owning", violations)
        )
        print(f"BAD plain -> Wrapping(Vec) re-registered: {reregistered}")
        self.assertEqual(
            reregistered,
            sorted(
                [
                    f"seam classification changed: {VIEWS_PATH}::AttackView borrowed->owning",
                    *(f"new inventory violation: {row['id']}" for row in violations),
                ]
            ),
        )

    def test_workspace_crate_names_cannot_shadow_other_crates(self) -> None:
        # Wonder eprime-r3 F3: seam resolution reads a path's first segment as
        # the workspace crate of that name.  A member may not take a sysroot or
        # keyword name, and no manifest may declare another crate under a
        # member's name (the vendored `client` path dependency, for example).
        seams = (("FleetCounts", "scalar"), ("FleetView", "borrowed"))
        for name in ("std", "core", "alloc", "proc_macro", "test", "crate", "self", "super", "Self"):
            with self.subTest(member=name):
                errors = inventories.check_tree(
                    self.fixture(
                        views=self.baseline_views(),
                        seam_types=seams,
                        workspace_files={f"crates/{name}/src/lib.rs": ""},
                    )
                )
                print(f"BAD reserved workspace crate name {name}: {errors}")
                self.assertIn(
                    f"workspace crates: crates/{name}/Cargo.toml: workspace crate name {name} "
                    "is reserved",
                    errors,
                )
        package = '[package]\nname = "{}"\nversion = "0.0.0"\nedition = "2021"\n'
        uses = "pub type Uses = api::T;\n"
        api = "#[derive(Clone, Copy)]\npub struct T(pub u64);\n"
        cases: dict[str, tuple[str, dict[str, str], str | None]] = {
            "workspace dependency": (
                '[workspace.dependencies]\nclient = { path = "vendor/client" }\n',
                {},
                "Cargo.toml: [workspace.dependencies] client is not workspace crate client",
            ),
            "member dependency": (
                "",
                {"crates/api/Cargo.toml": package.format("api") + '[dependencies]\nclient = "1"\n'},
                "crates/api/Cargo.toml: [dependencies] client is not workspace crate client",
            ),
            "target dev-dependency": (
                "",
                {
                    "crates/api/Cargo.toml": package.format("api")
                    + "[target.'cfg(unix)'.dev-dependencies]\n"
                    + 'client = { git = "https://example.invalid/client" }\n'
                },
                "crates/api/Cargo.toml: [target.cfg(unix).dev-dependencies] client is not "
                "workspace crate client",
            ),
            "reserved dependency key": (
                "",
                {
                    "crates/api/Cargo.toml": package.format("api")
                    + '[dependencies]\nstd = { package = "evil", version = "1" }\n'
                },
                "crates/api/Cargo.toml: [dependencies] std is a reserved crate name",
            ),
            "dependencies on the members themselves": (
                '[workspace.dependencies]\napi = { path = "crates/api" }\n'
                'vendored = { path = "vendor/client" }\n',
                {
                    "crates/frontend-core/Cargo.toml": package.format("frontend-core")
                    + '[dependencies]\napi = { workspace = true }\nvendored = { workspace = true }\n',
                    "crates/client/Cargo.toml": package.format("client")
                    + '[dependencies]\napi = { path = "../api" }\n'
                    + '[build-dependencies]\nfrontend-core = { path = "../frontend-core" }\n',
                },
                None,
            ),
        }
        for label, (workspace_extra, manifests, expected) in cases.items():
            with self.subTest(case=label):
                root = self.fixture(
                    views=self.baseline_views() + uses,
                    seam_types=seams + (("Uses", "scalar"),),
                    workspace_files={"crates/api/src/lib.rs": api, "crates/client/src/lib.rs": ""},
                )
                manifest = root / "Cargo.toml"
                manifest.write_text(
                    manifest.read_text(encoding="utf-8") + workspace_extra, encoding="utf-8"
                )
                for relative, contents in manifests.items():
                    (root / relative).write_text(contents, encoding="utf-8")
                errors = inventories.check_tree(root)
                print(f"workspace crate name collision ({label}): {errors}")
                if expected is None:
                    self.assertEqual(errors, [])
                else:
                    self.assertTrue(
                        any(
                            error.startswith("workspace crates: ") and expected in error
                            for error in errors
                        ),
                        errors,
                    )

    def test_inventory_reads_never_traverse_a_symlink(self) -> None:
        # Wonder eprime-r3 F2: no path component below the tree root may be a
        # symlink, for crate directories as for the foreign-type data file.
        seams = (("FleetCounts", "scalar"), ("FleetView", "borrowed"), ("Uses", "scalar"))

        def tree() -> Path:
            return self.fixture(
                views=self.baseline_views() + "pub type Uses = api::T;\n",
                seam_types=seams,
                workspace_files={
                    "crates/api/src/lib.rs": "#[derive(Clone, Copy)]\npub struct T(pub u64);\n"
                },
            )

        self.assertEqual(inventories.check_tree(tree()), [])
        cases = {
            "crates/api": "workspace crates: crates/api/Cargo.toml must not traverse a symlink",
            "crates/api/src": "workspace crates: crates/api/src/lib.rs must not traverse a symlink",
            "crates/frontend-core": "crates/frontend-core/src: symlinked Rust input is unsupported",
            "crates/frontend-core/src": (
                "crates/frontend-core/src: symlinked Rust input is unsupported"
            ),
            "crates/script": "crates/script/src: symlinked Rust input is unsupported",
            "tools": "inventory: tools/architecture/inventories.json must not traverse a symlink",
        }
        for linked, expected in cases.items():
            with self.subTest(linked=linked):
                root = tree()
                outside = tempfile.TemporaryDirectory(prefix="f0-outside-")
                self.addCleanup(outside.cleanup)
                moved = Path(outside.name) / "moved"
                (root / linked).rename(moved)
                (root / linked).symlink_to(moved, target_is_directory=True)
                errors = inventories.check_tree(root)
                print(f"BAD symlinked {linked}: {errors}")
                self.assertIn(expected, errors)

    def test_foreign_rows_may_not_name_workspace_types(self) -> None:
        views = self.baseline_views() + "pub type UsesHandle = client::session::Handle;\n"
        for workspace_path in ("api::identity::RequestId", "frontend_core::views::FleetCounts"):
            with self.subTest(row=workspace_path):
                root = self.fixture(
                    views=views,
                    seam_types=(
                        ("FleetCounts", "scalar"),
                        ("FleetView", "borrowed"),
                        ("UsesHandle", "scalar"),
                    ),
                    foreign_types=_foreign_data(
                        _foreign("client::session::Handle"), _foreign(workspace_path)
                    ),
                    workspace_files={"crates/api/src/lib.rs": ""},
                )
                errors = inventories.check_tree(root)
                print(f"BAD data row naming workspace type {workspace_path}: {errors}")
                self.assertIn(
                    f"foreign types: foreign_types[1] names workspace type {workspace_path}; "
                    "workspace types are classified from their definitions",
                    errors,
                )
                self.assertIn(
                    f"{VIEWS_PATH}::UsesHandle: unresolved public seam field type "
                    "client::session::Handle",
                    errors,
                )

    def test_foreign_data_row_keeps_generic_argument_ownership(self) -> None:
        views = (
            self.baseline_views()
            + "use client::session::{Settlement, Token};\n"
            + "pub type SettledScalar = Settlement<Token, Token>;\n"
            + "pub type SettledBytes = Settlement<Vec<u8>, Token>;\n"
            + "pub type SettledCatalog = Settlement<Token, script::CatalogDiff>;\n"
            + "pub type SettledView<'a> = Settlement<&'a str, Token>;\n"
            + "#[derive(Clone, Copy)]\n"
            + "pub struct SettledHolder {\n"
            + "    settled: Settlement<Token, std::sync::mpsc::Sender<u8>>,\n"
            + "}\n"
        )
        data = _foreign_data(
            _foreign("client::session::Settlement"), _foreign("client::session::Token")
        )
        base_seams = (("FleetCounts", "scalar"), ("FleetView", "borrowed"))
        clone_violation = (
            _record(
                "owning_derived_clone",
                VIEWS_PATH,
                "SettledHolder",
                "production",
                "derived_Clone_on_owning_view",
                owner="T3",
            ),
        )
        honest_errors = inventories.check_tree(
            self.fixture(
                views=views,
                seam_types=base_seams
                + (
                    ("SettledScalar", "scalar"),
                    ("SettledBytes", "owning"),
                    ("SettledCatalog", "owning"),
                    ("SettledView", "borrowed"),
                    ("SettledHolder", "owning"),
                ),
                violations=clone_violation,
                foreign_types=data,
            )
        )
        exempt_errors = inventories.check_tree(
            self.fixture(
                views=views,
                seam_types=base_seams
                + (
                    ("SettledScalar", "scalar"),
                    ("SettledBytes", "scalar"),
                    ("SettledCatalog", "scalar"),
                    ("SettledView", "scalar"),
                    ("SettledHolder", "scalar"),
                ),
                violations=clone_violation,
                foreign_types=data,
            )
        )
        print(f"GOOD data-row generic argument ownership: {honest_errors}")
        print(f"BAD data row as blanket scalar: {exempt_errors}")
        self.assertEqual(honest_errors, [])
        self.assertEqual(
            exempt_errors,
            [
                "seam classification disagrees with syntax: "
                f"{VIEWS_PATH}::SettledBytes is owning, registry says scalar",
                "seam classification disagrees with syntax: "
                f"{VIEWS_PATH}::SettledCatalog is owning, registry says scalar",
                "seam classification disagrees with syntax: "
                f"{VIEWS_PATH}::SettledHolder is owning, registry says scalar",
                "seam classification disagrees with syntax: "
                f"{VIEWS_PATH}::SettledView is borrowed, registry says scalar",
            ],
        )

    def test_unlisted_foreign_types_still_fail_resolution(self) -> None:
        settlement = _foreign_data(_foreign("client::session::Settlement"))
        cases = {
            "UnlistedImport": (
                "use client::session::Missing;\n"
                "#[derive(Clone, Copy)]\n"
                "pub struct UnlistedImport { id: Missing }\n",
                None,
                "unresolved imported seam field type Missing",
            ),
            "UnlistedPath": (
                "#[derive(Clone, Copy)]\n"
                "pub struct UnlistedPath { receipt: client::session::Receipt }\n",
                _foreign_data(),
                "unresolved public seam field type client::session::Receipt",
            ),
            "UnlistedLeaf": (
                "use client::session::Settlement;\n"
                "#[derive(Clone, Copy)]\n"
                "pub struct UnlistedLeaf {\n"
                "    settled: Settlement<u8, u8>,\n"
                "    other: client::other::Settlement<u8, u8>,\n"
                "}\n",
                settlement,
                "unresolved public seam field type client::other::Settlement",
            ),
            "UnlistedArgument": (
                "use client::session::Settlement;\n"
                "#[derive(Clone, Copy)]\n"
                "pub struct UnlistedArgument {\n"
                "    settled: Settlement<client::session::Missing, u8>,\n"
                "}\n",
                settlement,
                "unresolved public seam field type client::session::Missing",
            ),
        }
        for symbol, (source, data, expected) in cases.items():
            with self.subTest(symbol=symbol):
                root = self.fixture(
                    views=self.baseline_views() + source,
                    seam_types=(
                        ("FleetCounts", "scalar"),
                        ("FleetView", "borrowed"),
                        (symbol, "scalar"),
                    ),
                    foreign_types=data,
                )
                errors = inventories.check_tree(root)
                print(f"BAD unlisted foreign seam type {symbol}: {errors}")
                self.assertIn(f"{VIEWS_PATH}::{symbol}: {expected}", errors)

    def test_foreign_type_data_is_strictly_validated(self) -> None:
        valid = _foreign("client::session::Token")
        scope = "client::session::Scope"
        views = (
            self.baseline_views()
            + "#[derive(Clone, Copy)]\n"
            + "pub struct UsesToken { id: client::session::Token }\n"
        )
        depth = 100_000
        cases: dict[str, tuple[str | None, str]] = {
            "invalid json": ("{", "foreign types: cannot load"),
            "deep nesting": (
                '{"schema_version": 1, "foreign_types": ' + "[" * depth + "]" * depth + "}",
                "foreign types: cannot load",
            ),
            "duplicate json key": (
                '{"schema_version": 1, "schema_version": 1, "foreign_types": []}',
                "duplicate JSON key 'schema_version'",
            ),
            "top-level array": ("[]", "top-level fields must be exactly"),
            "extra top-level field": (
                json.dumps({"schema_version": 1, "foreign_types": [valid], "notes": ""}),
                "top-level fields must be exactly",
            ),
            "schema version 2": (
                json.dumps({"schema_version": 2, "foreign_types": [valid]}),
                "schema_version must be integer 1",
            ),
            "boolean schema version": (
                json.dumps({"schema_version": True, "foreign_types": [valid]}),
                "schema_version must be integer 1",
            ),
            "rows not an array": (
                json.dumps({"schema_version": 1, "foreign_types": {"client::session::Token": "scalar"}}),
                "foreign_types must be an array",
            ),
            "extra row field": (_foreign_data(valid, dict(_foreign(scope), owner="T1")), "has unknown or missing fields"),
            "missing reason": (
                _foreign_data(valid, {"path": scope, "classification": "scalar"}),
                "has unknown or missing fields",
            ),
            "single segment": (_foreign_data(valid, _foreign("Scope")), ".path must be an exact external Rust type path"),
            "leading separator": (
                _foreign_data(valid, _foreign("::client::session::Scope")),
                ".path must be an exact external Rust type path",
            ),
            "crate-relative": (_foreign_data(valid, _foreign("crate::views::Scope")), ".path must be an exact external Rust type path"),
            "glob": (_foreign_data(valid, _foreign("client::session::*")), ".path must be an exact external Rust type path"),
            "generic arguments": (
                _foreign_data(valid, _foreign("client::session::Settlement<u8, u8>")),
                ".path must be an exact external Rust type path",
            ),
            "whitespace": (
                _foreign_data(valid, _foreign(" client::session::Scope")),
                ".path must be an exact external Rust type path",
            ),
            "trailing newline": (
                _foreign_data(valid, _foreign("client::session::Scope\n")),
                ".path must be an exact external Rust type path",
            ),
            "non-string path": (
                _foreign_data(valid, {"path": 7, "classification": "scalar", "reason": "x"}),
                ".path must be an exact external Rust type path",
            ),
            "unknown class": (_foreign_data(valid, _foreign(scope, "copy")), "has unknown classification"),
            "non-string class": (
                _foreign_data(valid, {"path": scope, "classification": ["scalar"], "reason": "x"}),
                "has unknown classification",
            ),
            "blank reason": (
                _foreign_data(valid, {"path": scope, "classification": "scalar", "reason": " "}),
                ".reason must be nonblank",
            ),
            "shadows built-in": (
                _foreign_data(valid, _foreign("script::CatalogDiff", "scalar")),
                "shadows built-in foreign type script::CatalogDiff",
            ),
            "duplicate path": (
                _foreign_data(valid, _foreign("client::session::Token", "owning")),
                "duplicate path client::session::Token",
            ),
            "directory": (None, "must be a regular file"),
        }
        seams = (("FleetCounts", "scalar"), ("FleetView", "borrowed"), ("UsesToken", "scalar"))
        unresolved = f"{VIEWS_PATH}::UsesToken: unresolved public seam field type client::session::Token"
        for label, (content, expected) in cases.items():
            with self.subTest(case=label):
                root = self.fixture(views=views, seam_types=seams, foreign_types=content)
                if content is None:
                    (root / inventories.FOREIGN_TYPES_RELATIVE).mkdir(parents=True)
                errors = inventories.check_tree(root)
                print(f"BAD foreign type data ({label}): {[error[:200] for error in errors]}")
                self.assertTrue(any(expected in error for error in errors), errors)
                self.assertIn(unresolved, errors)
        symlinks = {
            "symlinked file": inventories.FOREIGN_TYPES_RELATIVE,
            "symlinked parent directory": inventories.FOREIGN_TYPES_RELATIVE.parent,
        }
        for label, linked in symlinks.items():
            with self.subTest(case=label):
                root = self.fixture(views=views, seam_types=seams)
                outside = root / "outside"
                outside.mkdir()
                (outside / inventories.FOREIGN_TYPES_RELATIVE.name).write_text(
                    _foreign_data(valid), encoding="utf-8"
                )
                link = root / linked
                link.parent.mkdir(parents=True, exist_ok=True)
                if linked == inventories.FOREIGN_TYPES_RELATIVE:
                    link.symlink_to(outside / inventories.FOREIGN_TYPES_RELATIVE.name)
                else:
                    link.symlink_to(outside, target_is_directory=True)
                errors = inventories.check_tree(root)
                print(f"BAD foreign type data ({label}): {errors}")
                self.assertIn(
                    "foreign types: tools/architecture-data/foreign_types.json must not traverse a symlink",
                    errors,
                )
                self.assertIn(unresolved, errors)

    def test_deeply_nested_inventory_fails_closed(self) -> None:
        root = self.fixture(
            views=self.baseline_views(),
            seam_types=(("FleetCounts", "scalar"), ("FleetView", "borrowed")),
        )
        depth = 100_000
        (root / inventories.INVENTORY_RELATIVE).write_text("[" * depth + "]" * depth, encoding="utf-8")
        errors = inventories.check_tree(root)
        print(f"BAD deeply nested inventory: {[error[:200] for error in errors]}")
        self.assertTrue(
            any(
                error.startswith("inventory: cannot load tools/architecture/inventories.json")
                for error in errors
            ),
            errors,
        )

    def test_stale_foreign_type_rows_fail(self) -> None:
        views = (
            self.baseline_views()
            + "pub struct TokenView<'a> { pub id: &'a client::session::Token }\n"
        )
        root = self.fixture(
            views=views,
            seam_types=(("FleetCounts", "scalar"), ("FleetView", "borrowed"), ("TokenView", "borrowed")),
            foreign_types=_foreign_data(
                _foreign("client::session::Token"), _foreign("client::session::Mode")
            ),
        )
        errors = inventories.check_tree(root)
        print(f"BAD stale foreign type rows: {errors}")
        self.assertEqual(
            errors,
            [
                "stale foreign type classification: client::session::Mode",
                "stale foreign type classification: client::session::Token",
            ],
        )

    def test_foreign_type_rows_are_transition_findings(self) -> None:
        seams = (("FleetCounts", "scalar"), ("FleetView", "borrowed"))
        uses = "use client::session::Token;\npub type UsesToken = Token;\n"

        def tree(prefix: str, views: str, extra: tuple[tuple[str, str], ...], data: str | None) -> Path:
            temporary = tempfile.TemporaryDirectory(prefix=prefix)
            self.addCleanup(temporary.cleanup)
            return _write_fixture(
                Path(temporary.name), views=views, seam_types=seams + extra, foreign_types=data
            )

        base = tree("f0-foreign-base-", self.baseline_views(), (), None)
        added = tree(
            "f0-foreign-added-",
            self.baseline_views() + uses,
            (("UsesToken", "scalar"),),
            _foreign_data(_foreign("client::session::Token")),
        )
        changed = tree(
            "f0-foreign-changed-",
            self.baseline_views() + uses,
            (("UsesToken", "owning"),),
            _foreign_data(_foreign("client::session::Token", "owning")),
        )
        removed = tree("f0-foreign-removed-", self.baseline_views(), (), _foreign_data())
        for root in (base, added, changed, removed):
            self.assertEqual(inventories.check_tree(root), [])
        added_findings = inventories.check_transition(base, added)
        changed_findings = inventories.check_transition(added, changed)
        removed_findings = inventories.check_transition(added, removed)
        print(f"BAD new foreign row: {added_findings}")
        print(f"BAD changed foreign class: {changed_findings}")
        print(f"GOOD removed unused foreign row: {removed_findings}")
        self.assertEqual(
            added_findings,
            [
                "new foreign type classification: client::session::Token scalar",
                f"new seam type: {VIEWS_PATH}::UsesToken",
            ],
        )
        self.assertEqual(
            changed_findings,
            [
                "foreign type classification changed: client::session::Token scalar->owning",
                f"seam classification changed: {VIEWS_PATH}::UsesToken scalar->owning",
            ],
        )
        self.assertEqual(removed_findings, [])

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



    def test_nested_use_trees_resolve_each_type_import(self) -> None:
        root = self.fixture(
            views=self.baseline_views()
            + "use api::{first::Count, second::{Width, Depth as D}};\n"
            + "#[derive(Clone, Copy)]\n"
            + "pub struct UsesNested { count: Count, width: Width, depth: D }\n",
            seam_types=(
                ("FleetCounts", "scalar"),
                ("FleetView", "borrowed"),
                ("UsesNested", "scalar"),
            ),
            workspace_files={
                "crates/api/src/lib.rs": (
                    "pub mod first { #[derive(Clone, Copy)] pub struct Count(u64); }\n"
                    "pub mod second {\n"
                    "    #[derive(Clone, Copy)] pub struct Width(u64);\n"
                    "    #[derive(Clone, Copy)] pub struct Depth(u64);\n"
                    "}\n"
                )
            },
        )
        self.assertEqual(inventories.check_tree(root), [])

    def test_super_path_cannot_escape_workspace_crate_root(self) -> None:
        api_lib = "#[derive(Clone, Copy)]\npub struct Id(u64);\n"
        base = self.clean_fixture(
            views=self.baseline_views(),
            workspace_files={"crates/api/src/lib.rs": api_lib},
        )
        candidate = self.fixture(
            views=self.baseline_views() + "pub type EscapesRoot = super::super::api::Id;\n",
            seam_types=(
                ("FleetCounts", "scalar"),
                ("FleetView", "borrowed"),
                ("EscapesRoot", "scalar"),
            ),
            workspace_files={"crates/api/src/lib.rs": api_lib},
        )
        self.assertEqual(inventories.check_tree(base), [])
        errors = inventories.check_tree(candidate)
        self.assertTrue(
            any("unresolved public seam field type super::super::api::Id" in error for error in errors),
            errors,
        )
        self.assertNotEqual(inventories.check_transition(base, candidate), [])

    def test_public_glob_reexport_fails_closed(self) -> None:
        base = self.clean_fixture(views=self.baseline_views())
        candidate = self.clean_fixture(
            views=self.baseline_views(),
            frontend_lib="pub mod views;\npub use views::*;\n",
        )
        self.assertEqual(inventories.check_tree(base), [])
        errors = inventories.check_tree(candidate)
        self.assertTrue(
            any("public glob re-export is unsupported (views::*)" in error for error in errors),
            errors,
        )
        transition = inventories.check_transition(base, candidate)
        self.assertTrue(
            any("public glob re-export is unsupported (views::*)" in error for error in transition),
            transition,
        )

    def test_binding_multiplicity_distinguishes_convergence_and_conflicts(self) -> None:
        base_seams = (("FleetCounts", "scalar"), ("FleetView", "borrowed"), ("Uses", "scalar"))
        api_types = (
            "pub mod values { #[derive(Clone, Copy)] pub struct Scalar(u64); }\n"
            "pub mod owned { #[derive(Clone)] pub struct Payload(pub Vec<u8>); }\n"
            "pub mod plain { #[derive(Clone, Copy)] pub struct Payload(u64); }\n"
        )
        base = self.fixture(
            views=self.baseline_views() + "#[derive(Clone, Copy)]\npub struct Uses { value: u64 }\n",
            seam_types=base_seams,
            workspace_files={
                "crates/api/src/lib.rs": api_types
                + "#[derive(Clone, Copy)]\npub struct State { value: values::Scalar }\n"
            },
        )
        same_target = self.fixture(
            views=self.baseline_views() + "#[derive(Clone, Copy)]\npub struct Uses { value: api::State }\n",
            seam_types=base_seams,
            workspace_files={
                "crates/api/src/lib.rs": api_types
                + "#[cfg(feature = \"left\")]\n"
                "use crate::values::Scalar as Value;\n"
                "#[cfg(not(feature = \"left\"))]\n"
                "use crate::values::Scalar as Value;\n"
                "#[derive(Clone, Copy)]\npub struct State { value: Value }\n"
            },
        )
        self.assertEqual(inventories.check_tree(base), [])
        self.assertEqual(inventories.check_tree(same_target), [])
        self.assertEqual(inventories.check_transition(base, same_target), [])

        conflict = self.fixture(
            views=self.baseline_views()
            + "#[cfg(feature = \"owned\")]\n"
            "use api::owned::Payload as Payload;\n"
            "#[cfg(not(feature = \"owned\"))]\n"
            "use api::plain::Payload as Payload;\n"
            "#[derive(Clone, Copy)]\npub struct Uses { value: Payload }\n",
            seam_types=base_seams,
            workspace_files={"crates/api/src/lib.rs": api_types},
        )
        conflict_errors = inventories.check_tree(conflict)
        self.assertTrue(
            any("ambiguous" in error and "Payload" in error for error in conflict_errors),
            conflict_errors,
        )
        conflict_transition = inventories.check_transition(base, conflict)
        self.assertTrue(
            any("ambiguous" in error and "Payload" in error for error in conflict_transition),
            conflict_transition,
        )

    def test_cfg_disjoint_local_import_collisions_refuse_d5b_and_d6(self) -> None:
        seam_types = (
            ("FleetCounts", "scalar"),
            ("FleetView", "borrowed"),
            ("AttackView", "borrowed"),
        )
        attack_prefix = (
            self.baseline_views()
            + "#[derive(Clone)]\npub struct AttackView {\n"
            "    pub label: &'static str,\n    pub payload: api::State,\n}\n"
        )
        cases = {
            "D5b std::num::Wrapping": (
                "#[derive(Clone, Copy)]\n"
                "pub struct State { pub label: &'static str, pub payload: u64 }\n",
                "#[cfg(feature = \"local\")]\n"
                "#[derive(Clone)]\npub struct Wrapping<T>(pub Vec<T>);\n"
                "#[cfg(not(feature = \"local\"))]\n"
                "use std::num::Wrapping;\n"
                "#[derive(Clone)]\n"
                "pub struct State { pub label: &'static str, pub payload: Wrapping<u64> }\n",
                "Wrapping",
            ),
            "D6 crate::inner::Payload": (
                "#[derive(Clone, Copy)]\n"
                "pub struct State { pub label: &'static str, pub payload: u64 }\n",
                "#[cfg(feature = \"local\")]\n"
                "#[derive(Clone)]\npub struct Payload(pub Vec<u8>);\n"
                "#[cfg(not(feature = \"local\"))]\n"
                "use crate::inner::Payload;\n"
                "pub mod inner { #[derive(Clone, Copy)] pub struct Payload(u64); }\n"
                "#[derive(Clone)]\n"
                "pub struct State { pub label: &'static str, pub payload: Payload }\n",
                "Payload",
            ),
        }
        for row, (base_api, candidate_api, collision) in cases.items():
            with self.subTest(row=row):
                base = self.fixture(
                    views=attack_prefix.replace("api::State", "u64"),
                    seam_types=seam_types,
                    workspace_files={"crates/api/src/lib.rs": base_api},
                )
                candidate = self.fixture(
                    views=attack_prefix,
                    seam_types=seam_types,
                    workspace_files={"crates/api/src/lib.rs": candidate_api},
                )
                self.assertEqual(inventories.check_tree(base), [])
                errors = inventories.check_tree(candidate)
                self.assertTrue(
                    any("ambiguous" in error and collision in error for error in errors),
                    errors,
                )
                transition = inventories.check_transition(base, candidate)
                self.assertTrue(
                    any("ambiguous" in error and collision in error for error in transition),
                    transition,
                )

    def test_extern_crate_std_core_rebindings_refuse_k1_k3_k4_k5(self) -> None:
        seam_types = (
            ("FleetCounts", "scalar"),
            ("FleetView", "borrowed"),
            ("AttackView", "borrowed"),
        )
        std_view = (
            self.baseline_views()
            + "#[derive(Clone)]\npub struct AttackView {\n"
            "    pub label: &'static str,\n"
            "    pub payload: std::num::Wrapping<u64>,\n"
            "}\n"
        )
        core_view = std_view.replace("std::num", "core::num")
        wrapping_api = (
            "pub mod num {\n"
            "    #[derive(Clone)] pub struct Wrapping<T>(pub Vec<T>);\n"
            "}\n"
        )
        k4_base_api = "#[derive(Clone, Copy)]\npub struct State { payload: u64 }\n"
        k4_view = (
            self.baseline_views()
            + "#[derive(Clone)]\npub struct AttackView {\n"
            "    pub label: &'static str,\n    pub payload: api::State,\n}\n"
        )
        base = self.fixture(
            views=std_view,
            seam_types=seam_types,
            workspace_files={"crates/api/src/lib.rs": wrapping_api + k4_base_api},
        )
        self.assertEqual(inventories.check_tree(base), [])

        cases = {
            "K1 views extern crate api as std": (
                std_view.replace(
                    "#[derive(Clone)]\npub struct AttackView",
                    "extern crate api as std;\n#[derive(Clone)]\npub struct AttackView",
                ),
                "pub mod views;\n",
                wrapping_api + k4_base_api,
                "extern crate api as std shadows reserved crate name",
            ),
            "K3 crate-root extern crate api as std": (
                std_view,
                "extern crate api as std;\npub mod views;\n",
                wrapping_api + k4_base_api,
                "extern crate api as std shadows reserved crate name",
            ),
            "K5 views extern crate api as core": (
                core_view.replace(
                    "#[derive(Clone)]\npub struct AttackView",
                    "extern crate api as core;\n#[derive(Clone)]\npub struct AttackView",
                ),
                "pub mod views;\n",
                wrapping_api + k4_base_api,
                "extern crate api as core shadows reserved crate name",
            ),
            "K4 api extern crate self as std": (
                k4_view,
                "pub mod views;\n",
                "extern crate self as std;\n"
                + wrapping_api
                + "#[derive(Clone)]\npub struct State { payload: std::num::Wrapping<u64> }\n",
                "extern crate self as std shadows reserved crate name",
            ),
        }
        for row, (views, frontend_lib, api_lib, expected) in cases.items():
            with self.subTest(row=row):
                candidate = self.fixture(
                    views=views,
                    frontend_lib=frontend_lib,
                    seam_types=seam_types,
                    workspace_files={"crates/api/src/lib.rs": api_lib},
                )
                errors = inventories.check_tree(candidate)
                self.assertTrue(any(expected in error for error in errors), errors)
                transition = inventories.check_transition(base, candidate)
                self.assertTrue(
                    any(expected in error for error in transition),
                    transition,
                )

    def test_extern_crate_nonreserved_bindings_fail_closed(self) -> None:
        base_seams = (("FleetCounts", "scalar"), ("FleetView", "borrowed"))
        base = self.clean_fixture(views=self.baseline_views())
        cases = {
            "plain root binding": (
                self.baseline_views(),
                "extern crate api;\npub mod views;\n",
                "#[derive(Clone, Copy)]\npub struct Id(u64);\n",
                "extern crate binding api is unsupported",
                base_seams,
            ),
            "consulted private renamed binding": (
                "mod nested { extern crate api as helper; pub struct State { value: u64 } }\n"
                + self.baseline_views()
                + "pub struct Uses { value: nested::State }\n",
                "pub mod views;\n",
                "",
                "extern crate binding helper is unsupported",
                base_seams + (("Uses", "scalar"),),
            ),
            "workspace self alias": (
                self.baseline_views() + "pub type Uses = api::Id;\n",
                "pub mod views;\n",
                "extern crate self as helper;\n#[derive(Clone, Copy)]\npub struct Id(u64);\n",
                "extern crate binding helper is unsupported",
                base_seams + (("Uses", "scalar"),),
            ),
        }
        self.assertEqual(inventories.check_tree(base), [])
        for row, (views, frontend_lib, api_lib, expected, seams) in cases.items():
            with self.subTest(row=row):
                candidate = self.fixture(
                    views=views,
                    frontend_lib=frontend_lib,
                    seam_types=seams,
                    workspace_files={"crates/api/src/lib.rs": api_lib},
                )
                errors = inventories.check_tree(candidate)
                self.assertTrue(any(expected in error for error in errors), errors)
                transition = inventories.check_transition(base, candidate)
                self.assertTrue(any(expected in error for error in transition), transition)
        unconsulted = self.fixture(
            views="mod nested { extern crate api as helper; }\n" + self.baseline_views(),
            seam_types=base_seams,
        )
        self.assertEqual(inventories.check_tree(unconsulted), [])
        self.assertEqual(inventories.check_transition(base, unconsulted), [])

    def test_type_namespace_nonvalues_generic_aliases_and_unnamed_items_fail_closed(self) -> None:
        base_seams = (("FleetCounts", "scalar"), ("FleetView", "borrowed"))
        base = self.clean_fixture(views=self.baseline_views())
        cases = {
            "generic alias": (
                self.baseline_views() + "pub type Alias<T> = Vec<T>;\n",
                base_seams + (("Alias", "scalar"),),
                "generic public type aliases are unsupported",
            ),
            "trait used as a field type": (
                self.baseline_views()
                + "pub trait Surface {}\n"
                "#[derive(Clone, Copy)]\npub struct Uses { value: Surface }\n",
                base_seams + (("Uses", "scalar"),),
                "public seam field path is a trait, not a value (Surface)",
            ),
            "const used as a field type": (
                self.baseline_views()
                + "pub const Surface: u8 = 0;\n"
                "#[derive(Clone, Copy)]\npub struct Uses { value: Surface }\n",
                base_seams + (("Uses", "scalar"),),
                "public seam field path is a value, not a type (Surface)",
            ),
            "static used as a field type": (
                self.baseline_views()
                + "pub static Surface: u8 = 0;\n"
                "#[derive(Clone, Copy)]\npub struct Uses { value: Surface }\n",
                base_seams + (("Uses", "scalar"),),
                "public seam field path is a value, not a type (Surface)",
            ),
            "function used as a field type": (
                self.baseline_views()
                + "pub fn Surface() {}\n"
                "#[derive(Clone, Copy)]\npub struct Uses { value: Surface }\n",
                base_seams + (("Uses", "scalar"),),
                "public seam field path is a value, not a type (Surface)",
            ),
            "module used as a field type": (
                self.baseline_views()
                + "pub mod Surface {}\n"
                "#[derive(Clone, Copy)]\npub struct Uses { value: Surface }\n",
                base_seams + (("Uses", "scalar"),),
                "field path resolves to module, not a type (Surface)",
            ),
            "unnamed public struct": (
                self.baseline_views() + "pub struct { value: u8 }\n",
                base_seams,
                "public struct has no supported name",
            ),
            "unnamed public enum": (
                self.baseline_views() + "pub enum { Value }\n",
                base_seams,
                "public enum has no supported name",
            ),
        }
        self.assertEqual(inventories.check_tree(base), [])
        for row, (views, seams, expected) in cases.items():
            with self.subTest(row=row):
                candidate = self.fixture(views=views, seam_types=seams)
                errors = inventories.check_tree(candidate)
                self.assertTrue(any(expected in error for error in errors), errors)
                self.assertNotEqual(inventories.check_transition(base, candidate), [])

        opaque = self.fixture(
            views=self.baseline_views()
            + "#[derive(Clone, Copy)]\npub struct Uses { value: api::Opaque }\n",
            seam_types=base_seams + (("Uses", "scalar"),),
            workspace_files={"crates/api/src/lib.rs": "extern type Opaque;\n"},
        )
        opaque_errors = inventories.check_tree(opaque)
        self.assertTrue(
            any("Opaque" in error and ("unresolved" in error or "unsupported" in error) for error in opaque_errors),
            opaque_errors,
        )

    def test_cfg_attr_binding_changes_are_refused_on_module_use_extern_and_macros(self) -> None:
        base = self.clean_fixture(views=self.baseline_views())
        cfg_attr = 'cfg_attr(feature = "probe", cfg(test))'
        path_attr = 'cfg_attr(feature = "probe", path = "alternate.rs")'
        cases = {
            "module path": (
                self.baseline_views(),
                f"#[{path_attr}]\npub mod views;\n",
                "cfg_attr may not alter cfg or module path provenance",
            ),
            "module cfg": (
                self.baseline_views(),
                f"#[{cfg_attr}]\npub mod views;\n",
                "cfg_attr may not alter cfg or module path provenance",
            ),
            "use path": (
                f"#[{path_attr}]\nuse std::num::Wrapping;\n" + self.baseline_views(),
                "pub mod views;\n",
                "cfg_attr may not alter cfg or module path provenance",
            ),
            "use cfg": (
                f"#[{cfg_attr}]\nuse std::num::Wrapping;\n" + self.baseline_views(),
                "pub mod views;\n",
                "cfg_attr may not alter cfg or module path provenance",
            ),
            "extern path": (
                f"#[{path_attr}]\nextern crate api as helper;\n" + self.baseline_views(),
                "pub mod views;\n",
                "cfg_attr may not alter cfg or module path provenance",
            ),
            "extern cfg": (
                f"#[{cfg_attr}]\nextern crate api as helper;\n" + self.baseline_views(),
                "pub mod views;\n",
                "cfg_attr may not alter cfg or module path provenance",
            ),
            "macro_use binding effect": (
                '#[cfg_attr(feature = "probe", macro_use)]\n'
                "extern crate api as helper;\n" + self.baseline_views(),
                "pub mod views;\n",
                "cfg_attr may not alter macro binding",
            ),
            "macro_export binding effect": (
                '#[cfg_attr(feature = "probe", macro_export)]\n'
                "macro_rules! generated { () => {}; }\n" + self.baseline_views(),
                "pub mod views;\n",
                "cfg_attr may not alter macro binding",
            ),
            "inner module path effect": (
                self.baseline_views(),
                f"#![{path_attr}]\npub mod views;\n",
                "cfg_attr may not alter cfg or module path provenance",
            ),
            "nested macro binding effect": (
                '#[cfg_attr(feature = "probe", cfg_attr(feature = "nested", macro_use))]\n'
                "extern crate api as helper;\n" + self.baseline_views(),
                "pub mod views;\n",
                "cfg_attr may not alter macro binding",
            ),
        }
        self.assertEqual(inventories.check_tree(base), [])
        for row, (views, frontend_lib, expected) in cases.items():
            with self.subTest(row=row):
                candidate = self.clean_fixture(views=views, frontend_lib=frontend_lib)
                errors = inventories.check_tree(candidate)
                self.assertTrue(any(expected in error for error in errors), errors)
                transition = inventories.check_transition(base, candidate)
                self.assertTrue(any(expected in error for error in transition), transition)

    def test_opaque_nonpath_attribute_effects_are_not_blanket_refused(self) -> None:
        seams = (("FleetCounts", "scalar"), ("FleetView", "borrowed"), ("Opaque", "scalar"))
        declaration = "#[derive(Clone, Copy)]\npub struct Opaque(u64);\n"
        base = self.fixture(views=self.baseline_views() + declaration, seam_types=seams)
        cases = {
            "opaque item attribute": "#[opaque]\n" + declaration,
            "opaque cfg_attr effect": '#[cfg_attr(feature="x", opaque)]\n' + declaration,
        }
        self.assertEqual(inventories.check_tree(base), [])
        for row, source in cases.items():
            with self.subTest(row=row):
                candidate = self.fixture(
                    views=self.baseline_views() + source,
                    seam_types=seams,
                )
                self.assertEqual(inventories.check_tree(candidate), [])
                self.assertEqual(inventories.check_transition(base, candidate), [])

    def test_path_module_attributes_are_validated_and_confined(self) -> None:
        base = self.clean_fixture(
            views=self.baseline_views(),
            frontend_lib='#[path = "views.rs"]\npub mod views;\n',
        )
        self.assertEqual(inventories.check_tree(base), [])
        cases = {
            "source-root escape": (
                self.baseline_views(),
                'pub mod views;\n#[path = "../outside.rs"]\npub mod external;\n',
                "module path escapes frontend-core source",
            ),
            "duplicate path": (
                self.baseline_views(),
                '#[path = "views.rs"]\n#[path = "views.rs"]\npub mod views;\n',
                "duplicate module path attributes",
            ),
            "computed path": (
                self.baseline_views(),
                '#[path = concat!("views", ".rs")]\npub mod views;\n',
                "unsupported module path attribute",
            ),
        }
        for row, (views, frontend_lib, expected) in cases.items():
            with self.subTest(row=row):
                candidate = self.clean_fixture(views=views, frontend_lib=frontend_lib)
                errors = inventories.check_tree(candidate)
                self.assertTrue(any(expected in error for error in errors), errors)
                transition = inventories.check_transition(base, candidate)
                self.assertTrue(any(expected in error for error in transition), transition)

    def test_absolute_std_field_and_import_paths_resolve(self) -> None:
        base_seams = (
            ("FleetCounts", "scalar"),
            ("FleetView", "borrowed"),
            ("UsesAbsolute", "scalar"),
        )
        base = self.fixture(
            views=self.baseline_views() + "#[derive(Clone, Copy)]\npub struct UsesAbsolute { value: u64 }\n",
            seam_types=base_seams,
        )
        absolute_field = self.fixture(
            views=self.baseline_views()
            + "#[derive(Clone, Copy)]\npub struct UsesAbsolute { value: ::std::num::Wrapping<u64> }\n",
            seam_types=base_seams,
        )
        absolute_import = self.fixture(
            views=self.baseline_views()
            + "use ::std::num::Wrapping;\n"
            "#[derive(Clone, Copy)]\npub struct UsesAbsolute { value: Wrapping<u64> }\n",
            seam_types=base_seams,
        )
        self.assertEqual(inventories.check_tree(base), [])
        for row, candidate in {
            "absolute field": absolute_field,
            "absolute import": absolute_import,
        }.items():
            with self.subTest(row=row):
                self.assertEqual(inventories.check_tree(candidate), [])
                self.assertEqual(inventories.check_transition(base, candidate), [])

    def test_macro_generated_workspace_items_fail_closed(self) -> None:
        seam_types = (
            ("FleetCounts", "scalar"),
            ("FleetView", "borrowed"),
            ("AttackView", "borrowed"),
        )
        base_api = "#[derive(Clone, Copy)]\npub struct State { payload: u64 }\n"
        base_views = (
            self.baseline_views()
            + "#[derive(Clone)]\npub struct AttackView {\n"
            "    pub label: &'static str,\n    pub payload: u64,\n}\n"
        )
        base = self.fixture(
            views=base_views,
            seam_types=seam_types,
            workspace_files={"crates/api/src/lib.rs": base_api},
        )
        cases = {
            "macro definition": (
                "macro_rules! make_type {\n"
                "    () => { pub struct Generated(pub Vec<u8>); };\n"
                "}\n"
                + base_api,
                "item-generating macro syntax is unsupported",
            ),
            "item invocation": (
                "make_type!();\n" + base_api,
                "item macro invocation is unsupported",
            ),
        }
        self.assertEqual(inventories.check_tree(base), [])
        for row, (api_lib, expected) in cases.items():
            with self.subTest(row=row):
                candidate = self.fixture(
                    views=base_views.replace("payload: u64", "payload: api::State"),
                    seam_types=seam_types,
                    workspace_files={"crates/api/src/lib.rs": api_lib},
                )
                errors = inventories.check_tree(candidate)
                self.assertTrue(
                    any(expected in error and "consulted for a seam field type" in error for error in errors),
                    errors,
                )
                transition = inventories.check_transition(base, candidate)
                self.assertTrue(
                    any(expected in error and "consulted for a seam field type" in error for error in transition),
                    transition,
                )

    def test_direct_macro_binding_attributes_refuse(self) -> None:
        base = self.clean_fixture(views=self.baseline_views())
        self.assertEqual(inventories.check_tree(base), [])
        cases = {
            "macro_use module": "#[macro_use]\nmod hidden {}\n",
            "macro_export definition": "#[macro_export]\nmacro_rules! generated { () => {}; }\n",
        }
        for row, source in cases.items():
            with self.subTest(row=row):
                candidate = self.clean_fixture(views=self.baseline_views() + source)
                expected = "unsupported macro binding attribute"
                errors = inventories.check_tree(candidate)
                self.assertTrue(any(expected in error for error in errors), errors)
                transition = inventories.check_transition(base, candidate)
                self.assertTrue(any(expected in error for error in transition), transition)
        unrelated = self.clean_fixture(views=self.baseline_views())
        script_macro = unrelated / "crates/script/src/quester/compile.rs"
        script_macro.parent.mkdir(parents=True, exist_ok=True)
        script_macro.write_text(
            "#[macro_export]\nmacro_rules! step { () => {}; }\n",
            encoding="utf-8",
        )
        self.assertEqual(inventories.check_tree(unrelated), [])
        self.assertEqual(inventories.check_transition(base, unrelated), [])

    def test_unsupported_field_type_syntax_refuses_macro_and_pointer_forms(self) -> None:
        base_seams = (("FleetCounts", "scalar"), ("FleetView", "borrowed"), ("Uses", "scalar"))
        base = self.clean_fixture(views=self.baseline_views())
        cases = {
            "type macro": ("pub struct Uses { value: type_macro!() }\n", "MACRO_TYPE"),
            "dyn trait": (
                "pub trait Marker {}\npub struct Uses { value: dyn Marker }\n",
                "DYN_TRAIT_TYPE",
            ),
            "impl trait": (
                "pub trait Marker {}\npub struct Uses { value: impl Marker }\n",
                "IMPL_TRAIT_TYPE",
            ),
            "function pointer": ("pub struct Uses { value: fn(u8) -> u8 }\n", "FN_PTR_TYPE"),
            "raw pointer": ("pub struct Uses { value: *const u8 }\n", "PTR_TYPE"),
        }
        self.assertEqual(inventories.check_tree(base), [])
        for row, (source, syntax) in cases.items():
            with self.subTest(row=row):
                candidate = self.fixture(
                    views=self.baseline_views() + source,
                    seam_types=base_seams,
                )
                errors = inventories.check_tree(candidate)
                self.assertTrue(
                    any(f"unsupported field type syntax ({syntax})" in error for error in errors),
                    errors,
                )
                self.assertNotEqual(inventories.check_transition(base, candidate), [])

    def test_prelude_configuration_attributes_fail_closed_in_inner_and_outer_positions(self) -> None:
        base = self.clean_fixture(views=self.baseline_views())
        attributes = ("no_implicit_prelude", "no_std", "no_core", "prelude_import")
        cases: dict[str, tuple[str, str]] = {}
        for attribute in attributes:
            cases[f"inner {attribute}"] = (
                "#!["
                + attribute
                + "]\n"
                + self.baseline_views(),
                "pub mod views;\n",
            )
            cases[f"outer {attribute}"] = (
                self.baseline_views() + f"\n#[{attribute}]\nmod hidden {{}}\n",
                "pub mod views;\n",
            )
        cases["crate-root no_std"] = (
            self.baseline_views(),
            "#![no_std]\npub mod views;\n",
        )
        cases["conditional no_std"] = (
            '#[cfg_attr(feature = "no-std", no_std)]\n' + self.baseline_views(),
            "pub mod views;\n",
        )
        cases["inner conditional no_std"] = (
            self.baseline_views(),
            '#![cfg_attr(feature = "no-std", no_std)]\npub mod views;\n',
        )
        self.assertEqual(inventories.check_tree(base), [])
        for row, (views, frontend_lib) in cases.items():
            with self.subTest(row=row):
                candidate = self.clean_fixture(views=views, frontend_lib=frontend_lib)
                errors = inventories.check_tree(candidate)
                self.assertTrue(
                    any("unsupported Rust prelude configuration" in error for error in errors),
                    errors,
                )
                transition = inventories.check_transition(base, candidate)
                self.assertTrue(
                    any("unsupported Rust prelude configuration" in error for error in transition),
                    transition,
                )
        unrelated_predicate = self.clean_fixture(
            views=self.baseline_views(),
            frontend_lib='#[cfg_attr(no_std, doc = "not a binding")]\npub mod views;\n',
        )
        self.assertEqual(inventories.check_tree(unrelated_predicate), [])
        self.assertEqual(inventories.check_transition(base, unrelated_predicate), [])

    def test_generic_const_self_and_associated_type_bindings(self) -> None:
        base_seams = (("FleetCounts", "scalar"), ("FleetView", "borrowed"))
        base = self.clean_fixture(views=self.baseline_views())
        cases = {
            "bare generic parameter": (
                "#[derive(Clone, Copy)]\npub struct Uses<T> { value: T }\n",
                "unresolved generic field ownership (T)",
                (("Uses", "scalar"),),
            ),
            "generic associated path": (
                "pub trait HasItem { type Item; }\n"
                "#[derive(Clone)]\npub struct Uses<T: HasItem> { value: T::Item }\n",
                "unresolved generic field ownership (T)",
                (("Uses", "scalar"),),
            ),
            "generic inside Option": (
                "#[derive(Clone, Copy)]\npub struct Uses<T> { value: Option<T> }\n",
                "unresolved generic field ownership (T)",
                (("Uses", "scalar"),),
            ),
            "Self field": (
                "pub struct UsesSelf { value: Self }\n",
                "unsupported Self/associated seam field type",
                (("UsesSelf", "scalar"),),
            ),
            "Self inside generic argument": (
                "pub struct UsesSelf { value: Option<Self> }\n",
                "unsupported Self/associated seam field type",
                (("UsesSelf", "scalar"),),
            ),
            "Self associated path": (
                "pub struct UsesSelf { value: Self::Item }\n",
                "unsupported Self/associated seam field type",
                (("UsesSelf", "scalar"),),
            ),
            "qualified associated projection": (
                "pub trait HasItem { type Item; }\n"
                "#[derive(Clone)]\n"
                "pub struct Uses<T: HasItem> { value: <T as HasItem>::Item }\n",
                "unsupported qualified associated field type",
                (("Uses", "scalar"),),
            ),
        }
        self.assertEqual(inventories.check_tree(base), [])
        for row, (source, expected, extra_seams) in cases.items():
            with self.subTest(row=row):
                candidate = self.fixture(
                    views=self.baseline_views() + source,
                    seam_types=base_seams + extra_seams,
                )
                errors = inventories.check_tree(candidate)
                self.assertTrue(any(expected in error for error in errors), errors)
                self.assertNotEqual(inventories.check_transition(base, candidate), [])

        fixed_seams = base_seams + (("Fixed", "scalar"), ("UsesConst", "scalar"))
        fixed_head = (
            self.baseline_views()
            + "#[derive(Clone, Copy)]\npub struct Fixed<const N: usize> { bytes: [u8; N] }\n"
        )
        fixed_base = self.fixture(
            views=fixed_head + "#[derive(Clone, Copy)]\npub struct UsesConst { fixed: Fixed<4> }\n",
            seam_types=fixed_seams,
        )
        fixed_braced = self.fixture(
            views=fixed_head + "#[derive(Clone, Copy)]\npub struct UsesConst { fixed: Fixed<{4}> }\n",
            seam_types=fixed_seams,
        )
        self.assertEqual(inventories.check_tree(fixed_base), [])
        self.assertEqual(inventories.check_tree(fixed_braced), [])
        self.assertEqual(inventories.check_transition(fixed_base, fixed_braced), [])

    def test_restricted_visibility_does_not_create_an_exported_seam(self) -> None:
        base_seams = (("FleetCounts", "scalar"), ("FleetView", "borrowed"))
        public = self.fixture(
            views=self.baseline_views() + "#[derive(Clone, Copy)]\npub struct Visible(u64);\n",
            seam_types=base_seams + (("Visible", "scalar"),),
        )
        self.assertEqual(inventories.check_tree(public), [])
        for visibility in ("pub(crate)", "pub(super)", "pub(in crate::views)"):
            with self.subTest(visibility=visibility):
                restricted = self.fixture(
                    views=self.baseline_views()
                    + f"#[derive(Clone, Copy)]\n{visibility} struct Restricted(u64);\n",
                    seam_types=base_seams + (("Restricted", "scalar"),),
                )
                errors = inventories.check_tree(restricted)
                self.assertTrue(
                    any(f"stale seam type classification: {VIEWS_PATH}::Restricted" in error for error in errors),
                    errors,
                )

    def test_raw_identifier_type_and_import_names_normalize_consistently(self) -> None:
        root = self.fixture(
            views=self.baseline_views()
            + "#[derive(Clone, Copy)]\npub struct r#widget(u64);\n"
            "use self::r#widget as r#renamed_widget;\n"
            "pub type UsesRawAlias = r#renamed_widget;\n",
            seam_types=(
                ("FleetCounts", "scalar"),
                ("FleetView", "borrowed"),
                ("widget", "scalar"),
                ("UsesRawAlias", "scalar"),
            ),
        )
        self.assertEqual(inventories.check_tree(root), [])

    def test_local_std_module_and_import_alias_paths_resolve(self) -> None:
        base_seams = (
            ("FleetCounts", "scalar"),
            ("FleetView", "borrowed"),
            ("UsesStd", "scalar"),
        )
        api_lib = "pub mod num { #[derive(Clone, Copy)] pub struct Wrapping(u64); }\n"
        base = self.fixture(
            views=self.baseline_views() + "#[derive(Clone, Copy)]\npub struct UsesStd { value: u64 }\n",
            seam_types=base_seams,
            workspace_files={"crates/api/src/lib.rs": api_lib},
        )
        import_alias = self.fixture(
            views=self.baseline_views()
            + "use api as api_alias;\n"
            "#[derive(Clone, Copy)]\npub struct UsesStd { value: api_alias::num::Wrapping }\n",
            seam_types=base_seams,
            workspace_files={"crates/api/src/lib.rs": api_lib},
        )
        self.assertEqual(inventories.check_tree(base), [])
        self.assertEqual(inventories.check_tree(import_alias), [])
        self.assertEqual(inventories.check_transition(base, import_alias), [])

        local_module = self.fixture(
            views=self.baseline_views()
            + "pub mod std {\n"
            "    pub mod num {\n"
            "        #[derive(Clone, Copy)] pub struct Wrapping(u64);\n"
            "    }\n"
            "}\n"
            "#[derive(Clone, Copy)]\npub struct UsesStd { value: std::num::Wrapping }\n",
            seam_types=base_seams + (("views::std::num::Wrapping", "scalar"),),
        )
        self.assertEqual(inventories.check_tree(local_module), [])

    def test_reserved_crate_name_import_aliases_refuse(self) -> None:
        base = self.clean_fixture(views=self.baseline_views())
        self.assertEqual(inventories.check_tree(base), [])
        for name in ("std", "core", "alloc", "proc_macro", "test"):
            with self.subTest(name=name):
                candidate = self.clean_fixture(
                    views=f"use api as {name};\n" + self.baseline_views()
                )
                message = f"import api as {name} shadows reserved crate name"
                errors = inventories.check_tree(candidate)
                self.assertTrue(any(message in error for error in errors), errors)
                transition = inventories.check_transition(base, candidate)
                self.assertTrue(any(message in error for error in transition), transition)

if __name__ == "__main__":
    unittest.main()
