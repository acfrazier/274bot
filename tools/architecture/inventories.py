"""Syntax-aware seam and navigation inventories for the F0 architecture gate.

The candidate JSON is data, never executable policy.  This module is loaded
from the trusted checker bundle and pins the parser executable independently
of the candidate tree.  Rust source is parsed by the pinned rust-analyzer CLI;
syntax errors, unknown registry shapes and inventory/source disagreement fail
closed.
"""

from __future__ import annotations

import hashlib
import json
import re
import shutil
import subprocess
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterator, Mapping

RUST_EDITION = "2021"



# The checker code (not inventories.json) pins the parser.  This is the exact
# Linux x86-64 rust-analyzer binary provisioned for the architecture gate.
PARSER_VERSION = "rust-analyzer 0.3.3081-standalone"
PARSER_SHA256 = "fd89304c85f6c1996b036ea8f2c9ad9d3e25a197dbd713661c7fe51ec9dc8e49"
INVENTORY_RELATIVE = Path("tools/architecture/inventories.json")
# Approved foreign-type data lives outside tools/architecture: every file under
# that directory except the four named data files is part of the protected
# executable bundle, and the active gate computes E' digests with its own
# exclusion list.  The file is candidate data, bound by the admission input
# digest; new rows and class changes are check_transition findings.
FOREIGN_TYPES_RELATIVE = Path("tools/architecture-data/foreign_types.json")
FOREIGN_TOP_FIELDS = frozenset({"schema_version", "foreign_types"})
FOREIGN_ROW_FIELDS = frozenset({"path", "classification", "reason"})
FOREIGN_PATH_RE = re.compile(r"[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)+")
FOREIGN_PATH_RESERVED_ROOTS = frozenset({"crate", "self", "super", "Self"})
VIEWS_RELATIVE = Path("crates/frontend-core/src/views.rs")
FRONTEND_SRC_RELATIVE = Path("crates/frontend-core/src")
SCRIPT_SRC_RELATIVE = Path("crates/script/src")
SCRIPT_TESTS_RELATIVE = Path("crates/script/tests")
SCRIPT_AUX_RELATIVE = (Path("crates/script/schema/generated/isolate_generated.rs"),)

OWNERS = frozenset({"T3", "T5"})
ENDPOINTS = frozenset({"T3.1", "T5.1"})
TYPE_CLASSIFICATIONS = frozenset({"owning", "borrowed", "scalar"})
VIOLATION_CATEGORIES = frozenset(
    {
        "owning_public_field",
        "owning_derived_clone",
        "owning_explicit_clone",
        "production_nav_implementation_import",
        "test_nav_implementation_import",
        "transitive_raw_representation_exposure",
    }
)
EXCEPTION_CATEGORIES = frozenset({"test_import_exception"})
TYPE_FIELDS = frozenset(
    {"id", "path", "symbol", "classification", "owner", "reason"}
)
RECORD_FIELDS = frozenset(
    {
        "id",
        "category",
        "path",
        "symbol",
        "cfg_or_target",
        "owner",
        "reason",
        "removal_endpoint",
        "classification",
    }
)
TOP_FIELDS = frozenset({"schema_version", "seam_types", "violations", "test_exceptions"})

# §21 §5.3 grants test implementation access only to these precise modules or
# integration-test targets.  The path is still checked against actual source,
# and check_transition rejects additions to an existing target.
TEST_EXCEPTION_TARGETS: dict[str, frozenset[str]] = {
    "crates/script/src/gatherer/runner.rs": frozenset({"cfg(test)::tests"}),
    "crates/script/src/gatherer/select.rs": frozenset(
        {"cfg(test)::tests", "cfg(test)::function:test_real_nav_world"}
    ),
    "crates/script/src/combat/risk/facts.rs": frozenset({"cfg(test)::tests"}),
    "crates/script/src/combat/risk/tests.rs": frozenset({"cfg(test)::module:tests"}),
    "crates/script/tests/gather_sites_reach.rs": frozenset({"integration-test"}),
    "crates/script/tests/path_schema.rs": frozenset({"integration-test"}),
    "crates/script/tests/cook_location_shape.rs": frozenset({"integration-test"}),
    "crates/script/tests/named_bank_locations.rs": frozenset({"integration-test"}),
}

# These nav modules contain implementation/storage representations.  Other
# nav paths outside nav::contract are still imports that T5.1 must migrate;
# this narrower set identifies representation leaks through a public API.
RAW_NAV_MODULES = frozenset(
    {
        "collision",
        "router",
        "transport",
        "zones",
        "pack",
        "traveller",
        "world",
        "world_state",
        "grid",
        "bundle",
        "map",
    }
)
RAW_NAV_ROOT_SYMBOLS = frozenset({"WorldState"})
BASELINE_SEAM_CLASSIFICATIONS = {
    "FleetCounts": "scalar",
    "FleetRow": "owning",
    "FleetView": "borrowed",
    "Light": "scalar",
    "OpBrief": "owning",
    "Phase": "scalar",
    "QueuePlace": "scalar",
    "ScriptPhase": "borrowed",
    "SlotDetail": "owning",
}
TRUSTED_SEAM_TYPE_CLASSES = {
    ("crates/frontend-core/src/bulk.rs", "BulkOutcome"): "owning",
    ("crates/frontend-core/src/bulk.rs", "BulkRow"): "owning",
    ("crates/frontend-core/src/bulk.rs", "BulkReport"): "owning",
    ("crates/frontend-core/src/capacity.rs", "CapacityNotice"): "owning",
    ("crates/frontend-core/src/fleet.rs", "Fleet"): "owning",
    ("crates/frontend-core/src/group_walk.rs", "WalkInputs"): "owning",
    ("crates/frontend-core/src/group_walk.rs", "MarkedWalk"): "owning",
    ("crates/frontend-core/src/log.rs", "Clock"): "scalar",
    ("crates/frontend-core/src/log.rs", "LogEntry"): "owning",
    ("crates/frontend-core/src/log.rs", "LogScope"): "owning",
    ("crates/frontend-core/src/log.rs", "LogFilter"): "owning",
    ("crates/frontend-core/src/log.rs", "LogView"): "owning",
    ("crates/frontend-core/src/log.rs", "LogStore"): "owning",
    ("crates/frontend-core/src/log.rs", "SaveTicket"): "owning",
    ("crates/frontend-core/src/log_file.rs", "Rotation"): "scalar",
    ("crates/frontend-core/src/log_file.rs", "SessionLogFile"): "owning",
    ("crates/frontend-core/src/map_bake.rs", "MapBakeChoice"): "scalar",
    ("crates/frontend-core/src/map_bake.rs", "MapBakePrompt"): "scalar",
    ("crates/frontend-core/src/map_bake.rs", "MapBakeGate"): "owning",
    ("crates/frontend-core/src/marked.rs", "MarkedCommandReport"): "owning",
    ("crates/frontend-core/src/marked.rs", "RestartScope"): "owning",
    ("crates/frontend-core/src/nav_prefs.rs", "NavPreference"): "scalar",
    ("crates/frontend-core/src/operations.rs", "OperationId"): "scalar",
    ("crates/frontend-core/src/operations.rs", "ActionKind"): "scalar",
    ("crates/frontend-core/src/operations.rs", "Outcome"): "owning",
    ("crates/frontend-core/src/operations.rs", "MemberOutcome"): "owning",
    ("crates/frontend-core/src/operations.rs", "OperationReport"): "owning",
    ("crates/frontend-core/src/profile_form.rs", "FormNotice"): "owning",
    ("crates/frontend-core/src/profile_form.rs", "SavedProfile"): "owning",
    ("crates/frontend-core/src/profile_form.rs", "FailedSave"): "owning",
    ("crates/frontend-core/src/profile_form.rs", "FormSettled"): "owning",
    ("crates/frontend-core/src/profile_form.rs", "ProfileFormSave"): "owning",
    ("crates/frontend-core/src/profile_saves.rs", "WriteFailure"): "owning",
    ("crates/frontend-core/src/profile_saves.rs", "SaveRecord"): "owning",
    ("crates/frontend-core/src/profile_saves.rs", "SaveResult"): "owning",
    ("crates/frontend-core/src/profile_saves.rs", "SaveSettled"): "owning",
    ("crates/frontend-core/src/quester_paths.rs", "QuesterPathsView"): "owning",
    ("crates/frontend-core/src/quester_paths.rs", "QuesterPathsController"): "owning",
    ("crates/frontend-core/src/resources.rs", "Metric"): "owning",
    ("crates/frontend-core/src/resources.rs", "ResourceView"): "owning",
    ("crates/frontend-core/src/resources.rs", "ProcessProbe"): "scalar",
    ("crates/frontend-core/src/scripts/mod.rs", "LiveSettings"): "owning",
    ("crates/frontend-core/src/scripts/mod.rs", "LiveDelivery"): "owning",
    ("crates/frontend-core/src/scripts/mod.rs", "SettingsResult"): "owning",
    ("crates/frontend-core/src/scripts/mod.rs", "SettingsWrite"): "owning",
    ("crates/frontend-core/src/scripts/mod.rs", "Notice"): "owning",
    ("crates/frontend-core/src/scripts/mod.rs", "BrowseCard"): "borrowed",
    ("crates/frontend-core/src/scripts/mod.rs", "Scripts"): "owning",
    ("crates/frontend-core/src/scripts/native.rs", "SchemaView"): "borrowed",
    ("crates/frontend-core/src/scripts/native.rs", "NativeCommand"): "scalar",
    ("crates/frontend-core/src/scripts/native.rs", "NativeTarget"): "owning",
    ("crates/frontend-core/src/scripts/native.rs", "NativeDetail"): "owning",
    ("crates/frontend-core/src/scripts/parameter_edit.rs", "ParameterOptions"): "owning",
    ("crates/frontend-core/src/scripts/parameter_edit.rs", "ParameterEditKey"): "owning",
    ("crates/frontend-core/src/scripts/parameter_edit.rs", "ParameterCommit"): "scalar",
    ("crates/frontend-core/src/scripts/reload.rs", "ReloadWarning"): "owning",
    ("crates/frontend-core/src/scripts/reload.rs", "PendingReloadKind"): "owning",
    ("crates/frontend-core/src/scripts/reload.rs", "PendingReload"): "owning",
    ("crates/frontend-core/src/scripts/reload.rs", "ReloadOutcome"): "owning",
    ("crates/frontend-core/src/scripts/sync.rs", "RestartPrompt"): "owning",
    ("crates/frontend-core/src/scripts/sync.rs", "SyncScope"): "owning",
    ("crates/frontend-core/src/scripts/sync.rs", "SyncReport"): "owning",
    ("crates/frontend-core/src/selection.rs", "ProfileIdentity"): "scalar",
    ("crates/frontend-core/src/selection.rs", "MarkedSelection"): "owning",
    ("crates/frontend-core/src/selection.rs", "BulkSkip"): "owning",
    ("crates/frontend-core/src/selection.rs", "StopReport"): "owning",
    ("crates/frontend-core/src/session.rs", "Transition"): "owning",
    ("crates/frontend-core/src/session.rs", "SlotTransition"): "owning",
    ("crates/frontend-core/src/session.rs", "Selection"): "owning",
    ("crates/frontend-core/src/session.rs", "Removal"): "owning",
    ("crates/frontend-core/src/session.rs", "ScriptStart"): "owning",
    ("crates/frontend-core/src/session.rs", "StartSettled"): "owning",
    ("crates/frontend-core/src/session.rs", "ArmMirror"): "owning",
    ("crates/frontend-core/src/session.rs", "MemoryNotice"): "scalar",
    ("crates/frontend-core/src/session.rs", "OperatorSession"): "owning",
    ("crates/frontend-core/src/surface.rs", "SlotAttach"): "owning",
    ("crates/frontend-core/src/surface.rs", "HeadlessSurface"): "owning",
    ("crates/frontend-core/src/views.rs", "Phase"): "scalar",
    ("crates/frontend-core/src/views.rs", "QueuePlace"): "scalar",
    ("crates/frontend-core/src/views.rs", "Light"): "scalar",
    ("crates/frontend-core/src/views.rs", "OpBrief"): "owning",
    ("crates/frontend-core/src/views.rs", "FleetRow"): "owning",
    ("crates/frontend-core/src/views.rs", "FleetCounts"): "scalar",
    ("crates/frontend-core/src/views.rs", "SlotDetail"): "owning",
    ("crates/frontend-core/src/views.rs", "ScriptPhase"): "borrowed",
    ("crates/frontend-core/src/views.rs", "FleetView"): "borrowed",
    ("crates/frontend-core/src/walk_permissions.rs", "WalkGlobalsView"): "scalar",
    ("crates/frontend-core/src/capacity.rs", "CapacityWarning"): "scalar",
    ("crates/frontend-core/src/capacity.rs", "RenderPolicyLabel"): "scalar",
    ("crates/frontend-core/src/log.rs", "Level"): "scalar",
    ("crates/frontend-core/src/log.rs", "Record"): "borrowed",
    ("crates/frontend-core/src/log.rs", "Source"): "scalar",
    ("crates/frontend-core/src/walk_permissions.rs", "DangerLevel"): "scalar",
}

# Frozen legacy built-in foreign classifications.  Do not extend or change this
# table: a new foreign seam field type is an approved data row in
# FOREIGN_TYPES_RELATIVE, and data rows may not shadow these paths.  Changing
# the built-in set is a protected enforcement update (E').
TRUSTED_FOREIGN_TYPE_CLASSES = {
    "host_play::walk_map::ActionError": "owning",
    "host::login_queue::AdmitBlocker": "scalar",
    "nav::bank_fetch::BankRows": "owning",
    "host::CapacityWarning": "scalar",
    # CatalogDiff stays owning across the supported script feature configurations.
    "script::CatalogDiff": "owning",
    "script::CompiledId": "scalar",
    "script::native::CompiledCard": "owning",
    "nav::router::FindOptions": "scalar",
    "host::FrameBuf": "owning",
    "host_play::InstancePermit": "owning",
    "script::JsCard": "owning",
    "script::JsLibrary": "owning",
    "api::hostlog::Level": "scalar",
    "script::LoadFailure": "owning",
    "script::LoadShape": "scalar",
    "host_play::walk_map::MapContext": "scalar",
    "host_play::walk_map::MapWalkPlan": "scalar",
    "host_play::Play": "owning",
    "script::PreparedCard": "owning",
    "script::PreparedConfig": "owning",
    "vault::Profile": "owning",
    "host::RenderPolicyLabel": "scalar",
    "host_play::admission::RiskInput": "scalar",
    "api::selected::RunKey": "scalar",
    "script::RunState": "scalar",
    "script::ScriptLifecycleReceipt": "owning",
    "script::ScriptPaint": "owning",
    "script::ScriptSel": "owning",
    "script::ScriptSettingsStore": "owning",
    "script::ScriptSource": "scalar",
    "script::ScriptStatus": "owning",
    "script::SettingDef": "owning",
    "host::SlotInput": "owning",
    "host_play::SlotStatus": "owning",
    "api::hostlog::Source": "scalar",
    "script::StartOutcome": "owning",
    "nav::tile::Tile": "scalar",
    "vault::Vault": "owning",
    "host_play::WalkArms": "owning",
    "host_play::WalkGlobals": "scalar",
    "nav::WorldState": "owning",
    "nav::world_state::WorldState": "owning",
    "std::time::Duration": "scalar",
    "std::time::Instant": "scalar",
    "std::time::SystemTime": "scalar",
    "std::thread::JoinHandle": "owning",
    "std::sync::atomic::AtomicBool": "owning",
    "std::sync::atomic::AtomicU64": "owning",
    "std::sync::atomic::AtomicUsize": "owning",
    "std::sync::mpsc::SyncSender": "owning",
    "std::sync::mpsc::Sender": "owning",
    "std::sync::mpsc::Receiver": "owning",
    "serde_json::Map": "owning",
    "serde_json::Value": "owning",
}

TRUSTED_FOREIGN_REEXPORTS = {
    ("crates/frontend-core/src/capacity.rs", "CapacityWarning"): "host::CapacityWarning",
    ("crates/frontend-core/src/capacity.rs", "RenderPolicyLabel"): "host::RenderPolicyLabel",
    ("crates/frontend-core/src/log.rs", "Level"): "api::hostlog::Level",
    ("crates/frontend-core/src/log.rs", "Record"): "api::hostlog::Record",
    ("crates/frontend-core/src/log.rs", "Source"): "api::hostlog::Source",
    ("crates/frontend-core/src/walk_permissions.rs", "DangerLevel"): "host_play::DangerLevel",
}

TRUSTED_FOREIGN_TYPE_CLASSES.update(
    {
        "host::RenderPolicyLabel": "scalar",
        "api::hostlog::Record": "borrowed",
        "host_play::DangerLevel": "scalar",
    }
)

KNOWN_SCALAR_TYPES = frozenset(
    {
        "bool",
        "char",
        "f32",
        "f64",
        "i8",
        "i16",
        "i32",
        "i64",
        "i128",
        "isize",
        "str",
        "u8",
        "u16",
        "u32",
        "u64",
        "u128",
        "usize",
    }
)

# The standard-library items behind the scalar/transparent leaf names, keyed by
# name with their std/core module.  A field type takes the no-ownership
# shortcut only when its path resolves to one of these exact items, or is a
# bare prelude/primitive name that nothing in its module shadows.  A
# workspace type with the same name is classified from its definition.
STD_TRANSPARENT_TYPE_MODULES = {
    "Option": "option",
    "Result": "result",
    "ControlFlow": "ops",
    "Poll": "task",
    "Wrapping": "num",
    "Reverse": "cmp",
    "PhantomData": "marker",
}
STD_LEAF_TYPE_PATHS = frozenset(
    {
        *(
            f"{root}::{module}::{name}"
            for root in ("std", "core")
            for name, module in STD_TRANSPARENT_TYPE_MODULES.items()
        ),
        *(f"{root}::primitive::{name}" for root in ("std", "core") for name in KNOWN_SCALAR_TYPES),
    }
)
PRELUDE_LEAF_TYPE_NAMES = KNOWN_SCALAR_TYPES | {"Option", "Result"}

# Crate names that a workspace member or a dependency key may not take: the
# sysroot crates and path keywords.  A workspace crate with such a name would
# shadow the standard library in seam resolution.
RESERVED_CRATE_NAMES = (
    frozenset({"std", "core", "alloc", "proc_macro", "test"}) | FOREIGN_PATH_RESERVED_ROOTS
)
DEPENDENCY_TABLES = (
    "dependencies",
    "dev-dependencies",
    "build-dependencies",
    "dev_dependencies",
    "build_dependencies",
)

FRONTEND_CRATE_NAMES = frozenset({"api", "host", "host_play", "nav", "script", "vault"})

OWNED_FIELD_TYPES = frozenset(
    {
        "String",
        "Vec",
        "VecDeque",
        "Box",
        "Arc",
        "Rc",
        "Cow",
        "HashMap",
        "HashSet",
        "BTreeMap",
        "BTreeSet",
        "PathBuf",
        "Mutex",
        "RwLock",
        "RefCell",
    }
)



@dataclass(frozen=True, order=True)
class _RecordKey:
    category: str
    path: str
    symbol: str
    cfg_or_target: str
    classification: str


@dataclass(frozen=True, order=True)
class _TypeKey:
    path: str
    symbol: str




class _InventoryError(Exception):
    pass


def _object_without_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise _InventoryError(f"duplicate JSON key {key!r}")
        result[key] = value
    return result


def _traverses_symlink(root: Path, relative: Path) -> bool:
    """True if any component of `relative` below `root` is a symlink.

    Inventory reads never follow a symlink inside the tree, so every file the
    checker consumes is the tree's own committed bytes.
    """
    component = root
    for part in relative.parts:
        component = component / part
        if component.is_symlink():
            return True
    return False


def _load_foreign_types(
    root: Path, workspace_crates: frozenset[str]
) -> tuple[dict[str, str], list[str]]:
    """Return approved data classifications for foreign seam field types.

    An absent file adds no rows.  An invalid file adds no rows and fails the
    tree, so a malformed registry can only remove classifications.  A row may
    not name a workspace crate: those types are classified from their
    definitions.
    """
    path = root / FOREIGN_TYPES_RELATIVE
    label = FOREIGN_TYPES_RELATIVE.as_posix()
    if _traverses_symlink(root, FOREIGN_TYPES_RELATIVE):
        return {}, [f"foreign types: {label} must not traverse a symlink"]
    if not path.exists():
        return {}, []
    if not path.is_file():
        return {}, [f"foreign types: {label} must be a regular file"]
    try:
        raw = json.loads(
            path.read_text(encoding="utf-8"),
            object_pairs_hook=_object_without_duplicate_keys,
        )
    except (OSError, UnicodeError, RecursionError, json.JSONDecodeError, _InventoryError) as error:
        return {}, [f"foreign types: cannot load {label}: {error}"]
    if not isinstance(raw, dict) or set(raw) != FOREIGN_TOP_FIELDS:
        return {}, ["foreign types: top-level fields must be exactly schema_version, foreign_types"]
    errors: list[str] = []
    if type(raw["schema_version"]) is not int or raw["schema_version"] != 1:
        errors.append("foreign types: schema_version must be integer 1")
    if not isinstance(raw["foreign_types"], list):
        errors.append("foreign types: foreign_types must be an array")
        return {}, errors
    rows: dict[str, str] = {}
    for index, row in enumerate(raw["foreign_types"]):
        where = f"foreign types: foreign_types[{index}]"
        if not isinstance(row, dict) or set(row) != FOREIGN_ROW_FIELDS:
            errors.append(f"{where} has unknown or missing fields")
            continue
        type_path = row["path"]
        if (
            not isinstance(type_path, str)
            or FOREIGN_PATH_RE.fullmatch(type_path) is None
            or type_path.split("::", 1)[0] in FOREIGN_PATH_RESERVED_ROOTS
        ):
            errors.append(f"{where}.path must be an exact external Rust type path")
            continue
        classification = row["classification"]
        if not isinstance(classification, str) or classification not in TYPE_CLASSIFICATIONS:
            errors.append(f"{where} has unknown classification")
        if not isinstance(row["reason"], str) or not row["reason"].strip():
            errors.append(f"{where}.reason must be nonblank")
        if type_path in TRUSTED_FOREIGN_TYPE_CLASSES:
            errors.append(f"{where} shadows built-in foreign type {type_path}")
        if type_path.split("::", 1)[0] in workspace_crates:
            errors.append(
                f"{where} names workspace type {type_path}; workspace types are "
                "classified from their definitions"
            )
        if type_path in rows:
            errors.append(f"foreign types: duplicate path {type_path}")
        rows[type_path] = str(classification)
    if errors:
        return {}, errors
    return rows, []


@dataclass(frozen=True)
class _WorkspaceCrate:
    manifest: str
    lib_root: str | None
    edition: Any


def _workspace_crates(root: Path) -> tuple[dict[str, _WorkspaceCrate], list[str]]:
    """Library crates of the candidate workspace, keyed by Rust crate name.

    Members come from the root manifest, as in check.py discover_workspace.  An
    absent root manifest declares no workspace crates; the graph checker
    refuses such a tree.  A member without a library target is still a
    workspace crate: data rows may not name it and resolving into it fails.

    Seam resolution reads a path's first segment as a workspace crate when one
    has that name, so the name must mean that crate everywhere in the
    workspace: a member may not take a sysroot or keyword name, and no manifest
    may declare a dependency of that name that is not the member itself (the
    vendored `client` path dependency, for example).  No dependency key may be
    a reserved name either.  Each case fails the whole tree.
    """
    if not (root / "Cargo.toml").exists() and not (root / "Cargo.toml").is_symlink():
        return {}, []

    def manifest(relative: Path) -> dict[str, Any]:
        path = root / relative
        if _traverses_symlink(root, relative):
            raise _InventoryError(f"{relative.as_posix()} must not traverse a symlink")
        if not path.is_file():
            raise _InventoryError(f"{relative.as_posix()} must be a regular file")
        with path.open("rb") as stream:
            return tomllib.load(stream)

    def dependency_tables(
        document: dict[str, Any], label: str
    ) -> Iterator[tuple[str, dict[str, Any]]]:
        tables: list[tuple[str, Any]] = [
            (kind, document[kind]) for kind in DEPENDENCY_TABLES if kind in document
        ]
        targets = document.get("target", {})
        if not isinstance(targets, dict):
            raise _InventoryError(f"{label}: unsupported target table")
        for platform, target in sorted(targets.items()):
            if not isinstance(target, dict):
                raise _InventoryError(f"{label}: unsupported target table")
            tables.extend(
                (f"target.{platform}.{kind}", target[kind])
                for kind in DEPENDENCY_TABLES
                if kind in target
            )
        workspace_table = document.get("workspace")
        if isinstance(workspace_table, dict) and "dependencies" in workspace_table:
            tables.append(("workspace.dependencies", workspace_table["dependencies"]))
        for kind, table in tables:
            if not isinstance(table, dict):
                raise _InventoryError(f"{label}: unsupported [{kind}] table")
            yield kind, table

    manifests: list[tuple[Path, str, dict[str, Any]]] = []
    member_directories: dict[str, Path] = {}

    crates: dict[str, _WorkspaceCrate] = {}
    try:
        workspace = manifest(Path("Cargo.toml"))
        manifests.append((Path("."), "Cargo.toml", workspace))
        table = workspace.get("workspace", {})
        members = table.get("members", []) if isinstance(table, dict) else None
        if not isinstance(members, list) or any(not isinstance(member, str) for member in members):
            raise _InventoryError("Cargo.toml: workspace members must be an array of strings")
        inherited = table.get("package", {}) if isinstance(table, dict) else {}
        directories = [Path(member) for member in members]
        if "package" in workspace:
            directories.insert(0, Path("."))
        for directory in directories:
            normalized = _normalize_source_path(directory)
            if normalized is None or any(character in str(directory) for character in "*?["):
                raise _InventoryError(f"Cargo.toml: unsupported workspace member {directory.as_posix()}")
            relative = normalized / "Cargo.toml"
            member = manifest(relative)
            if normalized != Path("."):
                manifests.append((normalized, relative.as_posix(), member))
            package = member.get("package")
            lib = member.get("lib", {})
            if (
                not isinstance(package, dict)
                or not isinstance(package.get("name"), str)
                or not isinstance(lib, dict)
            ):
                raise _InventoryError(f"{relative.as_posix()}: unsupported package manifest")
            name = lib.get("name", package["name"].replace("-", "_"))
            lib_path = lib.get("path", "src/lib.rs")
            if not isinstance(name, str) or not isinstance(lib_path, str):
                raise _InventoryError(f"{relative.as_posix()}: unsupported library target")
            lib_root = _normalize_source_path(normalized / lib_path)
            if lib_root is None:
                raise _InventoryError(f"{relative.as_posix()}: library target escapes the tree")
            edition = package.get("edition")
            if edition == {"workspace": True} and isinstance(inherited, dict):
                edition = inherited.get("edition")
            if name in RESERVED_CRATE_NAMES:
                raise _InventoryError(
                    f"{relative.as_posix()}: workspace crate name {name} is reserved"
                )
            if name in crates:
                raise _InventoryError(f"Cargo.toml: duplicate workspace crate {name}")
            if _traverses_symlink(root, lib_root):
                raise _InventoryError(f"{lib_root.as_posix()} must not traverse a symlink")
            crates[name] = _WorkspaceCrate(
                relative.as_posix(),
                lib_root.as_posix() if (root / lib_root).is_file() else None,
                edition,
            )
            member_directories[name] = normalized
        for directory, label, document in manifests:
            for kind, table in dependency_tables(document, label):
                for key, spec in sorted(table.items()):
                    crate_name = key.replace("-", "_")
                    if crate_name in RESERVED_CRATE_NAMES:
                        raise _InventoryError(f"{label}: [{kind}] {key} is a reserved crate name")
                    if crate_name not in crates:
                        continue
                    if isinstance(spec, dict) and spec.get("workspace") is True:
                        # Inherits the root [workspace.dependencies] entry,
                        # which this loop checks on its own.
                        continue
                    path = spec.get("path") if isinstance(spec, dict) else None
                    target = (
                        _normalize_source_path(directory / path) if isinstance(path, str) else None
                    )
                    if target != member_directories[crate_name]:
                        raise _InventoryError(
                            f"{label}: [{kind}] {key} is not workspace crate {crate_name}; "
                            "a workspace crate may not share its name with another crate"
                        )
    except (OSError, UnicodeError, RecursionError, tomllib.TOMLDecodeError, _InventoryError) as error:
        return {}, [f"workspace crates: {error}"]
    return crates, []


def _parser_executable() -> str:
    executable = shutil.which("rust-analyzer")
    if executable is None:
        raise _InventoryError("pinned rust-analyzer executable is unavailable")
    resolved = Path(executable).resolve(strict=True)
    digest = hashlib.sha256()
    try:
        with resolved.open("rb") as stream:
            for block in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(block)
    except OSError as error:
        raise _InventoryError(f"cannot read pinned rust-analyzer: {error}") from error
    if digest.hexdigest() != PARSER_SHA256:
        raise _InventoryError("rust-analyzer binary hash does not match the trusted pin")
    try:
        version = subprocess.run(
            [str(resolved), "--version"],
            check=False,
            capture_output=True,
            text=True,
            encoding="utf-8",
            timeout=10,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise _InventoryError(f"cannot identify pinned rust-analyzer: {error}") from error
    if version.returncode != 0 or version.stdout.strip() != PARSER_VERSION:
        raise _InventoryError("rust-analyzer version does not match the trusted pin")
    return str(resolved)


def _children(node: dict[str, Any]) -> list[dict[str, Any]]:
    children = node.get("children", [])
    if not isinstance(children, list) or any(not isinstance(child, dict) for child in children):
        raise _InventoryError("rust-analyzer returned a malformed syntax tree")
    return children


def _leaf_tokens(node: dict[str, Any]) -> Iterator[tuple[str, str]]:
    if node.get("type") == "Token":
        kind = node.get("kind")
        text = node.get("text")
        if not isinstance(kind, str) or not isinstance(text, str):
            raise _InventoryError("rust-analyzer returned a malformed token")
        yield kind, text
        return
    for child in _children(node):
        yield from _leaf_tokens(child)


def _syntax_errors(tree: dict[str, Any]) -> list[dict[str, Any]]:
    errors: list[dict[str, Any]] = []

    def visit(node: dict[str, Any]) -> None:
        if node.get("kind") in {"ERROR", "MISSING"}:
            errors.append(node)
        for child in _children(node):
            visit(child)

    visit(tree)
    return errors


def _rewrite_2021_gen_identifiers(source: str, errors: list[dict[str, Any]]) -> str | None:
    positions: set[tuple[int, int]] = set()

    def visit(node: dict[str, Any], inside_error: bool) -> None:
        inside_error = inside_error or node.get("kind") in {"ERROR", "MISSING"}
        if node.get("type") == "Token" and inside_error:
            if node.get("kind") != "GEN_KW":
                raise _InventoryError("unsupported Rust syntax error is not an edition-2021 `gen` identifier")
            start, end = node.get("start"), node.get("end")
            if not isinstance(start, list) or not isinstance(end, list) or not start or not end:
                raise _InventoryError("rust-analyzer returned invalid token offsets")
            positions.add((start[0], end[0]))
            return
        for child in _children(node):
            visit(child, inside_error)

    try:
        for error in errors:
            visit(error, False)
    except _InventoryError:
        return None
    if not positions:
        return None
    encoded = source.encode("utf-8")
    for start, end in sorted(positions, reverse=True):
        if not isinstance(start, int) or not isinstance(end, int) or encoded[start:end] != b"gen":
            return None
        encoded = encoded[:start] + b"r#gen" + encoded[end:]
    return encoded.decode("utf-8")


def _parse_source(executable: str, path: str, source: str) -> dict[str, Any]:
    def parse(text: str) -> dict[str, Any]:
        try:
            parsed = subprocess.run(
                [executable, "parse", "--json"],
                input=text,
                check=False,
                capture_output=True,
                text=True,
                encoding="utf-8",
                timeout=30,
            )
        except (OSError, subprocess.SubprocessError) as error:
            raise _InventoryError(f"{path}: rust-analyzer parse failed: {error}") from error
        if parsed.returncode != 0:
            detail = parsed.stderr.strip().replace("\n", " ")
            raise _InventoryError(f"{path}: rust-analyzer parse failed: {detail or parsed.returncode}")
        try:
            tree = json.loads(parsed.stdout)
        except json.JSONDecodeError as error:
            raise _InventoryError(f"{path}: rust-analyzer returned invalid parse JSON") from error
        if not isinstance(tree, dict) or tree.get("kind") != "SOURCE_FILE":
            raise _InventoryError(f"{path}: rust-analyzer did not return a Rust source-file tree")
        return tree

    tree = parse(source)
    errors = _syntax_errors(tree)
    if errors:
        normalized = _rewrite_2021_gen_identifiers(source, errors)
        if normalized is None:
            first = errors[0].get("kind")
            raise _InventoryError(f"{path}: unsupported or malformed Rust syntax ({first})")
        tree = parse(normalized)
        errors = _syntax_errors(tree)
        if errors:
            first = errors[0].get("kind")
            raise _InventoryError(f"{path}: unsupported or malformed Rust syntax ({first})")
    _reject_conditional_cfg_attr(tree, path)
    return tree


def _check_rust_editions(root: Path) -> list[str]:
    errors: list[str] = []
    for relative in (
        Path("crates/script/Cargo.toml"),
        Path("crates/frontend-core/Cargo.toml"),
    ):
        try:
            if _traverses_symlink(root, relative):
                raise OSError("symlinked manifest is unsupported")
            with (root / relative).open("rb") as stream:
                manifest = tomllib.load(stream)
        except (OSError, tomllib.TOMLDecodeError) as error:
            errors.append(f"{relative.as_posix()}: cannot read Rust edition: {error}")
            continue
        package = manifest.get("package")
        if not isinstance(package, dict) or package.get("edition") != RUST_EDITION:
            errors.append(f"{relative.as_posix()}: parser adapter requires edition {RUST_EDITION}")
    return errors


def _read_rust_files(root: Path, executable: str) -> tuple[dict[str, dict[str, Any]], list[str]]:
    parsed: dict[str, dict[str, Any]] = {}
    errors: list[str] = []
    roots = (SCRIPT_SRC_RELATIVE, SCRIPT_TESTS_RELATIVE)
    for relative_root in roots:
        directory = root / relative_root
        if _traverses_symlink(root, relative_root):
            errors.append(f"{relative_root.as_posix()}: symlinked Rust input is unsupported")
            continue
        if not directory.is_dir():
            errors.append(f"missing Rust source tree: {relative_root.as_posix()}")
            continue
        for path in sorted(directory.rglob("*.rs")):
            relative = path.relative_to(root).as_posix()
            if _traverses_symlink(root, path.relative_to(root)):
                errors.append(f"{relative}: symlinked Rust input is unsupported")
                continue
            try:
                source = path.read_text(encoding="utf-8")
                tree = _parse_source(executable, relative, source)
            except (OSError, UnicodeError, _InventoryError) as error:
                errors.append(str(error))
                continue
            parsed[relative] = {"source": source, "tree": tree}
    for relative in SCRIPT_AUX_RELATIVE:
        path = root / relative
        if _traverses_symlink(root, relative):
            errors.append(f"{relative.as_posix()}: symlinked Rust input is unsupported")
            continue
        try:
            source = path.read_text(encoding="utf-8")
            tree = _parse_source(executable, relative.as_posix(), source)
        except (OSError, UnicodeError, _InventoryError) as error:
            errors.append(str(error))
            continue
        parsed[relative.as_posix()] = {"source": source, "tree": tree}
    return parsed, errors
def _read_frontend_files(
    root: Path, executable: str
) -> tuple[dict[str, dict[str, Any]], list[str]]:
    parsed: dict[str, dict[str, Any]] = {}
    errors: list[str] = []
    directory = root / FRONTEND_SRC_RELATIVE
    if _traverses_symlink(root, FRONTEND_SRC_RELATIVE):
        return {}, [f"{FRONTEND_SRC_RELATIVE.as_posix()}: symlinked Rust input is unsupported"]
    if not directory.is_dir():
        return {}, [f"missing Rust source tree: {FRONTEND_SRC_RELATIVE.as_posix()}"]
    for path in sorted(directory.rglob("*.rs")):
        relative = path.relative_to(root).as_posix()
        if _traverses_symlink(root, path.relative_to(root)):
            errors.append(f"{relative}: symlinked Rust input is unsupported")
            continue
        try:
            source = path.read_text(encoding="utf-8")
            tree = _parse_source(executable, relative, source)
        except (OSError, UnicodeError, _InventoryError) as error:
            errors.append(str(error))
            continue
        parsed[relative] = {"source": source, "tree": tree}
    return parsed, errors


def _module_path_attribute(node: dict[str, Any], path: str) -> str | None:
    values: list[str] = []
    for tokens in _direct_attributes(node):
        texts = [text for _, text in tokens]
        if len(texts) < 4 or texts[:3] != ["#", "[", "path"]:
            continue
        strings = [text for kind, text in tokens if kind == "STRING"]
        if (
            len(strings) != 1
            or len(strings[0]) < 2
            or strings[0][0] != '"'
            or strings[0][-1] != '"'
            or "\\" in strings[0]
        ):
            raise _InventoryError(f"{path}: unsupported module path attribute")
        values.append(strings[0][1:-1])
    if len(values) > 1:
        raise _InventoryError(f"{path}: duplicate module path attributes")
    return values[0] if values else None


def _module_base(file_path: Path) -> Path:
    if file_path.name in {"lib.rs", "main.rs", "mod.rs"}:
        return file_path.parent
    return file_path.parent / file_path.stem


class _CrateSources(Mapping[str, dict[str, Any]]):
    """Rust files under one workspace crate, parsed when a module first needs them.

    Only the production module tree of a loaded crate is parsed; a file the
    parser adapter rejects fails only the crate that reaches it.  No path
    component below the tree root may be a symlink.
    """

    def __init__(self, root: Path, executable: str, directory: Path) -> None:
        self._root = root
        self._executable = executable
        base = root / directory
        self._files = (
            frozenset(path.relative_to(root).as_posix() for path in base.rglob("*.rs"))
            if not _traverses_symlink(root, directory) and base.is_dir()
            else frozenset()
        )
        self._parsed: dict[str, dict[str, Any]] = {}

    def __contains__(self, relative: object) -> bool:
        return relative in self._files

    def __iter__(self) -> Iterator[str]:
        return iter(sorted(self._files))

    def __len__(self) -> int:
        return len(self._files)

    def __getitem__(self, relative: str) -> dict[str, Any]:
        if relative not in self._files:
            raise KeyError(relative)
        if relative not in self._parsed:
            path = self._root / relative
            if _traverses_symlink(self._root, Path(relative)):
                raise _InventoryError(f"{relative}: symlinked Rust input is unsupported")
            try:
                source = path.read_text(encoding="utf-8")
            except (OSError, UnicodeError) as error:
                raise _InventoryError(f"{relative}: {error}") from error
            self._parsed[relative] = {
                "source": source,
                "tree": _parse_source(self._executable, relative, source),
            }
        return self._parsed[relative]


def _crate_module_sources(
    parsed: Mapping[str, dict[str, Any]],
    root_path: Path,
    label: str,
    boundary: Path | None = None,
) -> tuple[
    dict[tuple[str, ...], tuple[str, dict[str, Any], bool]],
    dict[tuple[str, ...], list[tuple[tuple[str, ...], bool]]],
    list[str],
    dict[tuple[str, ...], tuple[str, ...]],
]:
    """Resolve one crate's modules, their children, and inherited cfg predicates.

    A `#[path]` module must stay under `boundary` (default: the crate root's
    directory).
    """
    module_root = root_path.parent
    source_root = boundary if boundary is not None else module_root
    root_file = root_path.as_posix()
    if root_file not in parsed:
        return {}, {}, [f"missing Rust crate root: {root_file}"], {}
    modules: dict[tuple[str, ...], tuple[str, dict[str, Any], bool]] = {}
    children: dict[tuple[str, ...], list[tuple[tuple[str, ...], bool]]] = {}
    module_cfgs: dict[tuple[str, ...], tuple[str, ...]] = {}
    errors: list[str] = []

    def visit(
        file_key: str,
        container: dict[str, Any],
        module_path: tuple[str, ...],
        base: Path,
        inline: bool,
        cfgs: tuple[str, ...],
    ) -> None:
        if module_path in modules:
            errors.append(f"{file_key}: duplicate {label} module path {'::'.join(module_path)}")
            return
        try:
            file_inner = _inner_cfg_predicates(container, file_key)
        except _InventoryError as error:
            errors.append(str(error))
            file_inner = ()
        effective = cfgs + file_inner
        modules[module_path] = (file_key, container, inline)
        module_cfgs[module_path] = effective
        children[module_path] = []
        for child in _children(container):
            if child.get("kind") != "MODULE" or _test_cfg_expression(child) is not None:
                continue
            name = _module_name(child)
            if name is None:
                errors.append(f"{file_key}: module declaration has no supported name")
                continue
            child_path = module_path + (name,)
            child_cfgs = effective + _cfg_attributes(child)
            children[module_path].append((child_path, _is_public(child)))
            item_lists = _direct_children(child, "ITEM_LIST")
            if item_lists:
                if len(item_lists) != 1:
                    errors.append(f"{file_key}::{name}: unsupported inline module shape")
                    continue
                visit(file_key, item_lists[0], child_path, base / name, True, child_cfgs)
                continue
            try:
                explicit_path = _module_path_attribute(child, file_key)
            except _InventoryError as error:
                errors.append(str(error))
                continue
            if explicit_path is not None:
                target = _normalize_source_path(Path(file_key).parent / explicit_path)
                if target is None:
                    errors.append(f"{file_key}::{name}: module path escapes source root")
                    continue
                try:
                    target.relative_to(source_root)
                except ValueError:
                    errors.append(f"{file_key}::{name}: module path escapes {label} source")
                    continue
                candidates = (target.as_posix(),)
            else:
                candidate_base = _normalize_source_path(base / name)
                if candidate_base is None:
                    errors.append(f"{file_key}::{name}: module path escapes source root")
                    continue
                candidates = (
                    candidate_base.with_suffix(".rs").as_posix(),
                    (candidate_base / "mod.rs").as_posix(),
                )
            existing = [candidate for candidate in candidates if candidate in parsed]
            if len(existing) != 1:
                errors.append(
                    f"{file_key}::{name}: external module target is "
                    f"{'ambiguous' if existing else 'not in the parsed source set'}"
                )
                continue
            target_key = existing[0]
            try:
                target_tree = parsed[target_key]["tree"]
            except _InventoryError as error:
                errors.append(str(error))
                continue
            visit(
                target_key,
                target_tree,
                child_path,
                _module_base(Path(existing[0])),
                False,
                child_cfgs,
            )

    try:
        root_tree = parsed[root_file]["tree"]
    except _InventoryError as error:
        return {}, {}, [str(error)], {}
    visit(
        root_file,
        root_tree,
        (),
        module_root,
        False,
        (),
    )
    return modules, children, errors, module_cfgs


def _normalize_source_path(path: Path) -> Path | None:
    if path.is_absolute():
        return None
    parts: list[str] = []
    for part in path.parts:
        if part in {"", "."}:
            continue
        if part == "..":
            if not parts:
                return None
            parts.pop()
        else:
            parts.append(part)
    return Path(*parts)


def _root_test_scopes(
    parsed: dict[str, dict[str, Any]],
) -> tuple[dict[str, str], list[str]]:
    """Resolve external module and include provenance for the syntax scan."""
    contexts: dict[str, set[str]] = {}
    errors: list[str] = []
    root_file = "crates/script/src/lib.rs"
    if root_file not in parsed:
        return {}, [f"missing Rust crate root: {root_file}"]

    def module_base(file_path: Path) -> Path:
        if file_path.name in {"lib.rs", "main.rs", "mod.rs"}:
            return file_path.parent
        return file_path.parent / file_path.stem

    def module_path_attribute(node: dict[str, Any], path: str) -> str | None:
        values: list[str] = []
        for attribute in _direct_attributes(node):
            tokens = [text for _, text in attribute]
            if len(tokens) < 5 or tokens[:3] != ["#", "[", "path"]:
                continue
            strings = [text for kind, text in attribute if kind == "STRING"]
            if (
                len(strings) != 1
                or len(strings[0]) < 2
                or strings[0][0] != '"'
                or strings[0][-1] != '"'
                or "\\" in strings[0]
            ):
                raise _InventoryError(f"{path}: unsupported module path attribute")
            values.append(strings[0][1:-1])
        if len(values) > 1:
            raise _InventoryError(f"{path}: duplicate module path attributes")
        return values[0] if values else None

    def walk(
        node: dict[str, Any],
        file_relative: str,
        base: Path,
        module_stack: tuple[str, ...],
        inherited_scope: str,
    ) -> None:
        for child in _children(node):
            if child.get("kind") == "MACRO_CALL" and _macro_call_name(child) == "include":
                included_path = _macro_string_argument(child)
                target = (
                    _normalize_source_path(Path(file_relative).parent / included_path)
                    if included_path is not None
                    else None
                )
                if target is None or target.as_posix() not in parsed:
                    errors.append(f"{file_relative}: include! target is not in the parsed source set")
                else:
                    contexts.setdefault(target.as_posix(), set()).add(
                        inherited_scope if _is_test_scope(inherited_scope) else "production"
                    )
            if child.get("kind") != "MODULE":
                walk(child, file_relative, base, module_stack, inherited_scope)
                continue
            name = _module_name(child)
            if name is None:
                errors.append(f"{file_relative}: module declaration has no supported name")
                continue
            test_cfg = _test_cfg_expression(child)
            nested_stack = module_stack + (name,)
            item_lists = _direct_children(child, "ITEM_LIST")
            if test_cfg is not None:
                if _is_test_scope(inherited_scope):
                    scope = inherited_scope + "::" + name
                elif item_lists:
                    scope = test_cfg + "::" + "::".join(nested_stack)
                else:
                    scope = test_cfg + "::module:" + "::".join(nested_stack)
            elif _is_test_scope(inherited_scope):
                scope = inherited_scope + "::" + name
            else:
                scope = inherited_scope
            if item_lists:
                for item_list in item_lists:
                    walk(item_list, file_relative, base / name, nested_stack, scope)
                continue

            try:
                explicit_path = module_path_attribute(child, file_relative)
            except _InventoryError as error:
                errors.append(str(error))
                continue
            if explicit_path is not None:
                candidate = _normalize_source_path(Path(file_relative).parent / explicit_path)
                candidates = (candidate,) if candidate is not None else ()
            else:
                candidate_base = _normalize_source_path(base / name)
                candidates = (
                    (
                        candidate_base.with_suffix(".rs"),
                        candidate_base / "mod.rs",
                    )
                    if candidate_base is not None
                    else ()
                )
            existing = [candidate.as_posix() for candidate in candidates if candidate.as_posix() in parsed]
            if len(existing) != 1:
                errors.append(
                    f"{file_relative}::{name}: external module target is "
                    f"{'ambiguous' if existing else 'not in the parsed source set'}"
                )
                continue
            contexts.setdefault(existing[0], set()).add(
                scope if _is_test_scope(scope) else "production"
            )

    for relative, entry in sorted(parsed.items()):
        if not relative.startswith("crates/script/src/"):
            continue
        file_path = Path(relative)
        walk(entry["tree"], relative, module_base(file_path), (), "production")
    scopes: dict[str, str] = {}
    for path, values in contexts.items():
        if "production" in values or len(values) != 1:
            scopes[path] = "production"
        else:
            scopes[path] = next(iter(values))
    return scopes, errors


def _direct_children(node: dict[str, Any], kind: str) -> list[dict[str, Any]]:
    return [child for child in _children(node) if child.get("kind") == kind]


def _first_name(node: dict[str, Any]) -> str | None:
    for child in _children(node):
        if child.get("kind") == "NAME":
            for token_kind, text in _leaf_tokens(child):
                if token_kind in {"IDENT", "RAW_IDENT"}:
                    return text.removeprefix("r#")
    return None


def _path_segment_name(segment: dict[str, Any]) -> str | None:
    for child in _children(segment):
        if child.get("kind") in {"GENERIC_ARG_LIST", "GENERIC_PARAM_LIST"}:
            continue
        for token_kind, text in _leaf_tokens(child):
            if token_kind in {
                "IDENT",
                "RAW_IDENT",
                "SELF_KW",
                "SUPER_KW",
                "CRATE_KW",
                "SELF_TYPE_KW",
            }:
                return text.removeprefix("r#")
    return None


def _path_segments(node: dict[str, Any]) -> tuple[str, ...]:
    if node.get("kind") == "PATH_TYPE":
        paths = _direct_children(node, "PATH")
        return _path_segments(paths[0]) if paths else ()
    if node.get("kind") != "PATH":
        return ()
    result: list[str] = []
    for child in _children(node):
        if child.get("kind") == "PATH":
            result.extend(_path_segments(child))
        elif child.get("kind") == "PATH_SEGMENT":
            name = _path_segment_name(child)
            if name is not None:
                result.append(name)
    return tuple(result)


def _is_public(node: dict[str, Any]) -> bool:
    for visibility in _direct_children(node, "VISIBILITY"):
        tokens = [kind for kind, _ in _leaf_tokens(visibility)]
        return "PUB_KW" in tokens and "L_PAREN" not in tokens
    return False


def _attribute_tokens(attribute: dict[str, Any]) -> list[tuple[str, str]]:
    return [(kind, text) for kind, text in _leaf_tokens(attribute) if kind not in {"WHITESPACE", "COMMENT"}]


def _cfg_attribute(node: dict[str, Any]) -> str | None:
    for attribute in _direct_children(node, "ATTR"):
        tokens = _attribute_tokens(attribute)
        texts = [text for _, text in tokens]
        if len(texts) >= 4 and texts[0] == "#" and texts[1] == "[" and texts[2] == "cfg":
            return "".join(texts[2:-1])
    return None


def _cfg_attributes(node: dict[str, Any]) -> tuple[str, ...]:
    """Every direct `#[cfg(...)]` predicate on a node, in source order."""
    result: list[str] = []
    for attribute in _direct_children(node, "ATTR"):
        texts = [text for _, text in _attribute_tokens(attribute)]
        if len(texts) >= 4 and texts[0] == "#" and texts[1] == "[" and texts[2] == "cfg":
            result.append("".join(texts[2:-1]))
    return tuple(result)


def _cfg_attr_predicate(texts: list[str]) -> str | None:
    """Return `cfg(<predicate>)` for `#[cfg_attr(<predicate>, ...)]` tokens."""
    if len(texts) < 6 or texts[:4] != ["#", "[", "cfg_attr", "("]:
        return None
    depth = 0
    for index in range(4, len(texts)):
        text = texts[index]
        if text in {"(", "["}:
            depth += 1
        elif text in {")", "]"}:
            depth -= 1
        elif text == "," and depth == 0:
            return "cfg(" + "".join(texts[4:index]) + ")"
    return None


def _inner_cfg_predicates(container: dict[str, Any], path: str) -> tuple[str, ...]:
    """Exact inner `#![cfg]` / `#![cfg_attr]` provenance, in source order."""
    result: list[str] = []
    for attribute in _direct_children(container, "ATTR"):
        tokens = _attribute_tokens(attribute)
        texts = [text for _, text in tokens]
        if len(texts) < 4 or texts[0] != "#" or texts[1] != "!" or texts[2] != "[":
            continue
        normalized = ["#", "["] + texts[3:]
        if len(normalized) >= 4 and normalized[2] == "cfg":
            result.append("".join(normalized[2:-1]))
        elif len(normalized) >= 4 and normalized[2] == "cfg_attr":
            if _cfg_attr_predicate(normalized) is None:
                raise _InventoryError(f"{path}: unsupported inner cfg_attr syntax")
            result.append("".join(normalized[2:-1]))
    return tuple(result)


def _cfg_label(predicates: tuple[str, ...]) -> str:
    """Exact conditional provenance of a violation; unconditional is production.

    A mixed cfg such as `cfg(any(test, feature = "test-support"))` is not a
    pure test scope, so it is recorded verbatim. Removing the gate changes
    the violation id and is therefore a new violation, not an empty delta.
    """
    return " & ".join(predicates) if predicates else "production"


def _test_cfg_expression(node: dict[str, Any]) -> str | None:
    value = _cfg_attribute(node)
    if value in {"cfg(test)", "cfg(all(test))"} or (
        value is not None and value.startswith("cfg(all(test,")
    ):
        return value
    return None


def _is_test_scope(scope: str) -> bool:
    return (
        scope == "integration-test"
        or scope.startswith("cfg(test)::")
        or scope.startswith("cfg(all(test")
    )


def _direct_attributes(node: dict[str, Any]) -> list[list[tuple[str, str]]]:
    return [_attribute_tokens(attr) for attr in _direct_children(node, "ATTR")]


def _reject_conditional_cfg_attr(tree: dict[str, Any], path: str) -> None:
    for node, _, _, _, _ in _walk_ast(tree, "production", ()):
        if node.get("kind") != "ATTR":
            continue
        tokens = _attribute_tokens(node)
        if len(tokens) < 3 or tokens[2][1] != "cfg_attr":
            continue
        if any(
            text in {"cfg", "path"} and kind not in {"STRING", "RAW_STRING"}
            for kind, text in tokens[3:]
        ):
            raise _InventoryError(f"{path}: cfg_attr may not alter cfg or module path provenance")





def _module_name(node: dict[str, Any]) -> str | None:
    return _first_name(node)




def _scope_for_node(
    node: dict[str, Any],
    inherited_scope: str,
    module_stack: tuple[str, ...],
) -> str:
    test_cfg = _test_cfg_expression(node)
    if test_cfg is not None:
        base = inherited_scope if _is_test_scope(inherited_scope) else test_cfg
        if node.get("kind") == "MODULE":
            name = _module_name(node)
            stack = module_stack + ((name,) if name else ())
            return base + "::" + "::".join(stack)
        if node.get("kind") == "FN":
            name = _first_name(node)
            return base + "::function:" + (name or "<unnamed>")
        return base + "::item"
    cfg = _cfg_attribute(node)
    if cfg is not None and not _is_test_scope(inherited_scope):
        return cfg
    return inherited_scope


def _walk_ast(
    node: dict[str, Any],
    scope: str,
    module_stack: tuple[str, ...],
    parent_kind: str | None = None,
    in_use_tree: bool = False,
) -> Iterator[tuple[dict[str, Any], str, tuple[str, ...], str | None, bool]]:
    kind = node.get("kind")
    next_scope = scope
    next_stack = module_stack
    if kind in {"MODULE", "FN", "STRUCT", "ENUM", "TYPE_ALIAS", "CONST", "STATIC", "USE"}:
        next_scope = _scope_for_node(node, scope, module_stack)
    if kind == "MODULE":
        name = _module_name(node)
        if name:
            next_stack = module_stack + (name,)
    next_in_use = in_use_tree or kind in {"USE", "USE_TREE", "USE_TREE_LIST"}
    yield node, next_scope, next_stack, parent_kind, next_in_use
    for child in _children(node):
        yield from _walk_ast(child, next_scope, next_stack, kind, next_in_use)


def _is_nav_raw(segments: tuple[str, ...]) -> bool:
    return any(
        segment == "nav" and index + 1 < len(segments) and segments[index + 1] != "contract"
        for index, segment in enumerate(segments)
    )


def _is_raw_representation(segments: tuple[str, ...]) -> bool:
    return any(
        segment == "nav"
        and (
            segments[index + 1] in RAW_NAV_MODULES
            or (
                index + 2 == len(segments)
                and segments[index + 1] in RAW_NAV_ROOT_SYMBOLS
            )
        )
        for index, segment in enumerate(segments[:-1])
    )


def _is_exception_scope(path: str, scope: str) -> bool:
    return scope in TEST_EXCEPTION_TARGETS.get(path, frozenset())


def _use_tree_imports(
    node: dict[str, Any], prefix: tuple[str, ...] = ()
) -> list[tuple[str, str]]:
    """Expand a syntax-tree use tree into (fully-qualified symbol, local name)."""
    result: list[tuple[str, str]] = []
    own_paths = _direct_children(node, "PATH")
    own_prefix = prefix
    if own_paths:
        own_prefix = prefix + _path_segments(own_paths[0])
    nested_lists = _direct_children(node, "USE_TREE_LIST")
    if nested_lists:
        for nested_list in nested_lists:
            for child in _children(nested_list):
                if child.get("kind") == "USE_TREE":
                    result.extend(_use_tree_imports(child, own_prefix))
        return result
    if any(child.get("kind") in {"GLOB", "STAR"} for child in _children(node)):
        symbol = "::".join(own_prefix + ("*",))
        result.append((symbol, "*"))
        return result
    if not own_prefix:
        return result
    aliases: list[str] = []
    for rename in _direct_children(node, "RENAME"):
        names = [
            text.removeprefix("r#")
            for kind, text in _leaf_tokens(rename)
            if kind in {"IDENT", "RAW_IDENT"} and text != "as"
        ]
        aliases.extend(names[-1:])
    target_prefix = own_prefix[:-1] if own_prefix[-1] == "self" else own_prefix
    if not target_prefix:
        return result
    local_name = aliases[0] if aliases else target_prefix[-1]
    result.append(("::".join(target_prefix), local_name))
    return result


def _macro_call_name(node: dict[str, Any]) -> str | None:
    paths = _direct_children(node, "PATH")
    if not paths:
        return None
    segments = _path_segments(paths[0])
    return segments[-1] if segments else None


def _macro_string_argument(node: dict[str, Any]) -> str | None:
    trees = _direct_children(node, "TOKEN_TREE")
    if len(trees) != 1:
        return None
    tokens = [
        child
        for child in _children(trees[0])
        if child.get("type") == "Token"
        and child.get("kind") not in {"WHITESPACE", "COMMENT"}
    ]
    if len(tokens) != 3 or tokens[0].get("text") != "(" or tokens[2].get("text") != ")":
        return None
    literal = tokens[1]
    text = literal.get("text")
    if literal.get("kind") != "STRING" or not isinstance(text, str):
        return None
    if len(text) < 2 or text[0] != '"' or text[-1] != '"' or "\\" in text:
        return None
    return text[1:-1]


def _macro_nav_paths(node: dict[str, Any]) -> set[str]:
    paths: set[str] = set()
    for token_tree in _direct_children(node, "TOKEN_TREE"):
        tokens = [
            (kind, text)
            for kind, text in _leaf_tokens(token_tree)
            if kind not in {"WHITESPACE", "COMMENT"}
        ]
        for index, (kind, text) in enumerate(tokens):
            if kind not in {"IDENT", "RAW_IDENT"} or text.removeprefix("r#") != "nav":
                continue
            segments = ["nav"]
            if (
                index >= 3
                and tokens[index - 3][0] in {"IDENT", "RAW_IDENT"}
                and tokens[index - 3][1] in {"crate", "self", "super"}
                and tokens[index - 2][1] == ":"
                and tokens[index - 1][1] == ":"
            ):
                segments.insert(0, tokens[index - 3][1])
            position = index + 1
            while position < len(tokens):
                if tokens[position][1] == "::":
                    segment_position = position + 1
                elif (
                    tokens[position][1] == ":"
                    and position + 1 < len(tokens)
                    and tokens[position + 1][1] == ":"
                ):
                    segment_position = position + 2
                else:
                    break
                if (
                    segment_position >= len(tokens)
                    or tokens[segment_position][0] not in {"IDENT", "RAW_IDENT"}
                ):
                    break
                segments.append(tokens[segment_position][1].removeprefix("r#"))
                position = segment_position + 1
            if _is_nav_raw(tuple(segments)):
                paths.add("::".join(segments))
    return paths


ALLOWED_TEST_INCLUDES = frozenset(
    {
        (
            "crates/script/src/isolate_fb.rs",
            "cfg(test)::tests",
            "isolate_fb_schema_roundtrip.rs",
        )
    }
)





def _field_type_node(field: dict[str, Any], path: str, owner: str) -> dict[str, Any]:
    for child in _children(field):
        kind = child.get("kind")
        if isinstance(kind, str) and kind.endswith("_TYPE"):
            return child
    raise _InventoryError(f"{path}::{owner}: unsupported field type syntax")


def _type_ownership(
    node: dict[str, Any],
    path: str,
    owner: str,
    *,
    resolve_type: Any = None,
    generic_names: frozenset[str] = frozenset(),
    allow_generic: bool = False,
) -> tuple[bool, bool]:
    """Return (owns, borrows) for a field type.

    An owned-container leaf name counts as owning, whatever defines it.  Any
    other path, including a std scalar or transparent-container name, is
    classified by `resolve_type`, which decides whether the path is that std
    item or a type with its own definition.  Generic arguments are always
    visited.
    """
    owned = False
    borrowed = False

    def visit(
        current: dict[str, Any],
        behind_reference: bool = False,
        inside_known_owner: bool = False,
    ) -> None:
        nonlocal owned, borrowed
        kind = current.get("kind")
        if kind in {"MACRO_TYPE", "PTR_TYPE", "FN_PTR_TYPE", "IMPL_TRAIT_TYPE", "DYN_TRAIT_TYPE"}:
            raise _InventoryError(f"{path}::{owner}: unsupported field type syntax ({kind})")
        if kind == "REF_TYPE":
            borrowed = True
            behind_reference = True
        elif kind == "PATH_TYPE":
            segments = _path_segments(current)
            if segments and not inside_known_owner:
                name = segments[-1]
                if name in OWNED_FIELD_TYPES:
                    if not behind_reference:
                        owned = True
                        inside_known_owner = True
                elif len(segments) == 1 and name in generic_names:
                    if not behind_reference and not allow_generic:
                        raise _InventoryError(
                            f"{path}::{owner}: unresolved generic field ownership ({name})"
                        )
                elif resolve_type is not None and not behind_reference:
                    classification = resolve_type(segments)
                    if classification == "owning":
                        owned = True
                    elif classification == "borrowed":
                        borrowed = True
        for child in _children(current):
            visit(child, behind_reference, inside_known_owner)

    visit(node)
    return owned, borrowed


def _type_parameter_names(node: dict[str, Any]) -> frozenset[str]:
    names: set[str] = set()
    for parameter_list in _direct_children(node, "GENERIC_PARAM_LIST"):
        for parameter in _direct_children(parameter_list, "TYPE_PARAM"):
            name = _first_name(parameter)
            if name is not None:
                names.add(name)
    return frozenset(names)


def _type_alias_target(
    node: dict[str, Any], path: str, owner: str
) -> dict[str, Any]:
    after_equals = False
    for child in _children(node):
        if child.get("kind") == "EQ":
            after_equals = True
            continue
        kind = child.get("kind")
        if after_equals and isinstance(kind, str) and (
            kind.endswith("_TYPE") or kind == "TYPE"
        ):
            return child
    raise _InventoryError(f"{path}::{owner}: unsupported type alias target syntax")


def _type_definitions(
    tree: dict[str, Any],
    path: str,
    *,
    symbol_prefix: str = "",
    direct_only: bool = True,
    strict: bool = False,
    include_private: bool = False,
    field_ownership: Any = None,
    baseline_names: bool = True,
) -> dict[_TypeKey, dict[str, Any]]:
    definitions: dict[_TypeKey, dict[str, Any]] = {}
    for item, scope, module_stack, parent_kind, _ in _walk_ast(tree, "production", ()):
        if _is_test_scope(scope) or (
            direct_only
            and (module_stack or parent_kind not in {None, "SOURCE_FILE", "ITEM_LIST"})
        ):
            continue
        kind = item.get("kind")
        if strict and kind in {"UNION", "TYPE_ALIAS", "TRAIT"} and _is_public(item):
            raise _InventoryError(f"{path}: unsupported public seam type syntax ({kind})")
        if strict and kind in {"MACRO_RULES", "MACRO_RULES_DEF"} and parent_kind in {"SOURCE_FILE", "ITEM_LIST"}:
            raise _InventoryError(f"{path}: item-generating macro syntax is unsupported")
        if strict and kind == "MACRO_CALL" and parent_kind in {"SOURCE_FILE", "ITEM_LIST"}:
            raise _InventoryError(f"{path}: item macro invocation is unsupported")
        if kind not in {"STRUCT", "ENUM"} or (not include_private and not _is_public(item)):
            continue
        name = _first_name(item)
        if not name:
            raise _InventoryError(f"{path}: public {kind.lower()} has no supported name")
        symbol = f"{symbol_prefix}::{name}" if symbol_prefix else name
        key = _TypeKey(path, symbol)
        if key in definitions:
            raise _InventoryError(f"{path}::{symbol}: duplicate public seam type")
        fields: list[tuple[str, bool, tuple[str, ...]]] = []
        field_nodes: list[dict[str, Any]] = []
        if kind == "STRUCT":
            for field_list in _children(item):
                if field_list.get("kind") == "RECORD_FIELD_LIST":
                    for field in _direct_children(field_list, "RECORD_FIELD"):
                        field_name = _first_name(field)
                        if field_name is None:
                            raise _InventoryError(f"{path}::{name}: unsupported record field")
                        fields.append((field_name, _is_public(field), _cfg_attributes(field)))
                        field_nodes.append(field)
                elif field_list.get("kind") == "TUPLE_FIELD_LIST":
                    for index, field in enumerate(_direct_children(field_list, "TUPLE_FIELD")):
                        fields.append((str(index), _is_public(field), _cfg_attributes(field)))
                        field_nodes.append(field)
        else:
            for variant_list in _direct_children(item, "VARIANT_LIST"):
                for variant in _direct_children(variant_list, "VARIANT"):
                    variant_name = _first_name(variant)
                    if variant_name is None:
                        raise _InventoryError(f"{path}::{name}: unsupported enum variant")
                    variant_cfgs = _cfg_attributes(variant)
                    for field_list in _children(variant):
                        if field_list.get("kind") == "RECORD_FIELD_LIST":
                            for field in _direct_children(field_list, "RECORD_FIELD"):
                                field_name = _first_name(field)
                                if field_name is None:
                                    raise _InventoryError(
                                        f"{path}::{name}::{variant_name}: unsupported record field"
                                    )
                                fields.append(
                                    (
                                        f"{variant_name}::{field_name}",
                                        True,
                                        variant_cfgs + _cfg_attributes(field),
                                    )
                                )
                                field_nodes.append(field)
                        elif field_list.get("kind") == "TUPLE_FIELD_LIST":
                            for index, field in enumerate(_direct_children(field_list, "TUPLE_FIELD")):
                                fields.append(
                                    (
                                        f"{variant_name}::{index}",
                                        True,
                                        variant_cfgs + _cfg_attributes(field),
                                    )
                                )
                                field_nodes.append(field)
        attrs = _direct_attributes(item)
        attr_text = [[text for _, text in attr] for attr in attrs]
        derives_copy = any(
            texts[:3] == ["#", "[", "derive"] and any(text == "Copy" for text in texts[3:])
            for texts in attr_text
        )
        unconditional_clone = any(
            texts[:3] == ["#", "[", "derive"] and "Clone" in texts[3:] for texts in attr_text
        )
        conditional_clone: set[str] = set()
        for texts in attr_text:
            if texts[:3] == ["#", "[", "cfg_attr"] and "derive" in texts and "Clone" in texts:
                predicate = _cfg_attr_predicate(texts)
                if predicate is None:
                    raise _InventoryError(f"{path}::{name}: unsupported cfg_attr derive syntax")
                conditional_clone.add(predicate)
        derives_clone = unconditional_clone or bool(conditional_clone)
        # An unconditional derive dominates. Otherwise Clone exists when any
        # cfg_attr predicate holds; that exact condition is its provenance.
        if unconditional_clone or not conditional_clone:
            clone_cfgs: tuple[str, ...] = ()
        elif len(conditional_clone) == 1:
            clone_cfgs = (next(iter(conditional_clone)),)
        else:
            clone_cfgs = (
                "cfg(any(" + ",".join(value[4:-1] for value in sorted(conditional_clone)) + "))",
            )
        generic_names = _type_parameter_names(item)
        owning_field = False
        borrowed_field = False
        for field in field_nodes:
            field_type = _field_type_node(field, path, name)
            has_owner, has_borrow = (
                field_ownership(field_type, path, name, generic_names, not derives_copy)
                if field_ownership is not None
                else _type_ownership(field_type, path, name)
            )
            owning_field = owning_field or has_owner
            borrowed_field = borrowed_field or has_borrow
        classification = (
            "owning"
            if owning_field or (not borrowed_field and not derives_copy)
            else "borrowed"
            if borrowed_field
            else "scalar"
        )
        trusted_class = TRUSTED_SEAM_TYPE_CLASSES.get(
            (path, symbol),
            BASELINE_SEAM_CLASSIFICATIONS.get(name) if baseline_names else None,
        )
        if trusted_class is not None and trusted_class != classification:
            raise _InventoryError(
                f"{path}::{symbol}: trusted seam classification is {trusted_class}, "
                f"syntax implies {classification}"
            )
        definitions[key] = {
            "name": name,
            "fields": fields,
            "classification": classification,
            "derived_clone": derives_clone,
            "cfgs": _cfg_attributes(item),
            "clone_cfgs": clone_cfgs,
        }
    return definitions


def _explicit_clone_impls(
    tree: dict[str, Any],
    path: str,
    *,
    direct_only: bool = True,
) -> dict[str, set[tuple[str, ...]]]:
    """Map each explicit `impl Clone for T` target to the cfg predicates of its impls."""
    clones: dict[str, set[tuple[str, ...]]] = {}
    for item, scope, module_stack, _, _ in _walk_ast(tree, "production", ()):
        if _is_test_scope(scope) or (direct_only and module_stack):
            continue
        if item.get("kind") != "IMPL":
            continue
        children = _children(item)
        try:
            for_index = next(index for index, child in enumerate(children) if child.get("kind") == "FOR_KW")
        except StopIteration:
            continue
        trait_paths = [child for child in children[:for_index] if child.get("kind") == "PATH_TYPE"]
        target_paths = [child for child in children[for_index + 1 :] if child.get("kind") == "PATH_TYPE"]
        if not trait_paths or not target_paths:
            continue
        trait = _path_segments(trait_paths[-1])
        target = _path_segments(target_paths[0])
        if trait and trait[-1] == "Clone" and target:
            clones.setdefault(target[-1], set()).add(_cfg_attributes(item))
    return clones


def _seam_records(
    root: Path,
    foreign_classes: dict[str, str],
    consulted_foreign: set[str],
    workspace_crates: dict[str, _WorkspaceCrate],
) -> tuple[dict[_TypeKey, str], set[_RecordKey], list[str]]:
    try:
        executable = _parser_executable()
    except _InventoryError as error:
        return {}, set(), [f"parser: {error}"]
    parsed, parse_errors = _read_frontend_files(root, executable)
    if parse_errors:
        return {}, set(), parse_errors
    modules, module_children, module_errors, module_cfgs = _crate_module_sources(
        parsed, FRONTEND_SRC_RELATIVE / "lib.rs", "frontend-core"
    )
    if module_errors:
        return {}, set(), module_errors
    frontend_modules = tuple(modules)

    type_items: dict[tuple[tuple[str, ...], str], dict[str, Any]] = {}
    declared_types: dict[tuple[tuple[str, ...], str], dict[str, Any]] = {}
    unsupported_types: dict[tuple[tuple[str, ...], str], tuple[str, str]] = {}
    non_value_types: set[tuple[tuple[str, ...], str]] = set()
    non_type_items: set[tuple[tuple[str, ...], str]] = set()
    ambiguous_type_items: set[tuple[tuple[str, ...], str]] = set()
    module_issues: dict[tuple[str, ...], list[str]] = {}
    module_imports: dict[tuple[str, ...], dict[str, set[tuple[str, ...]]]] = {}
    module_globs: dict[tuple[str, ...], list[tuple[str, ...]]] = {}
    public_uses: dict[tuple[str, ...], list[tuple[str, dict[str, Any]]]] = {}
    # Production cfg branches are unioned; unresolved or differing alternatives fail closed.

    # frontend-core modules are keyed from (); another workspace crate's modules
    # are loaded on first use and keyed from ("::<crate>",), which no Rust
    # module name can spell.
    frontend_crate = next(
        (
            name
            for name, crate in workspace_crates.items()
            if crate.lib_root == (FRONTEND_SRC_RELATIVE / "lib.rs").as_posix()
        ),
        None,
    )
    loaded_crates: dict[str, tuple[str, ...] | str] = {}

    def crate_root(module_path: tuple[str, ...]) -> tuple[str, ...]:
        return module_path[:1] if module_path and module_path[0].startswith("::") else ()

    def module_self_import(
        module_path: tuple[str, ...],
        target: tuple[str, ...],
        local_name: str,
    ) -> bool:
        """Avoid alias cycles for modules already in the current namespace."""
        if not target or local_name != target[-1]:
            return False
        root_path = crate_root(module_path)
        if target[0] == "crate":
            candidates = (root_path + target[1:],)
        elif target[0] == "self":
            candidates = (module_path + target[1:],)
        elif target[0] == "super":
            parent = list(module_path)
            remainder = list(target)
            while remainder and remainder[0] == "super":
                if len(parent) <= len(root_path):
                    return False
                parent.pop()
                remainder.pop(0)
            candidates = (tuple(parent + remainder),)
        else:
            candidates = (module_path + target, root_path + target)
        imported_module = next(
            (candidate for candidate in candidates if candidate in modules),
            None,
        )
        return imported_module in {module_path + (local_name,), root_path + (local_name,)}

    def index_modules(
        indexed: dict[tuple[str, ...], tuple[str, dict[str, Any], bool]],
        label: str,
        exported: bool,
    ) -> None:
        for module_path, (file_key, container, _) in indexed.items():
            module_issues[module_path] = []
            module_imports[module_path] = {}
            module_globs[module_path] = []
            public_uses[module_path] = []
            for item, scope, module_stack, parent_kind, _ in _walk_ast(container, "production", ()):
                if _is_test_scope(scope) or module_stack or parent_kind not in {
                    None,
                    "SOURCE_FILE",
                    "ITEM_LIST",
                }:
                    continue
                kind = item.get("kind")
                name = _first_name(item)
                if kind in {"STRUCT", "ENUM", "TYPE_ALIAS"}:
                    if name is None:
                        if _is_public(item):
                            module_issues[module_path].append(
                                f"{file_key}: public {str(kind).lower()} has no supported name"
                            )
                        continue
                    origin = (module_path, name)
                    if origin in type_items:
                        ambiguous_type_items.add(origin)
                        module_issues[module_path].append(
                            f"{file_key}: duplicate {label} type {name}"
                        )
                        continue
                    type_items[origin] = item
                    if exported and _is_public(item):
                        declared_types[origin] = item
                elif kind == "UNION" and _is_public(item):
                    if name is not None:
                        unsupported_types[(module_path, name)] = (file_key, str(kind))
                elif kind == "TRAIT" and _is_public(item):
                    if name is not None:
                        non_value_types.add((module_path, name))
                elif kind in {"CONST", "STATIC", "FN"} and _is_public(item):
                    if name is not None:
                        non_type_items.add((module_path, name))
                elif kind in {"MACRO_RULES", "MACRO_RULES_DEF"} and parent_kind in {
                    "SOURCE_FILE",
                    "ITEM_LIST",
                }:
                    module_issues[module_path].append(
                        f"{file_key}: item-generating macro syntax is unsupported"
                    )
                elif kind == "MACRO_CALL" and parent_kind in {"SOURCE_FILE", "ITEM_LIST"}:
                    module_issues[module_path].append(
                        f"{file_key}: item macro invocation is unsupported"
                    )
                elif kind == "USE":
                    if _is_public(item):
                        public_uses[module_path].append((file_key, item))
                    for use_tree in _direct_children(item, "USE_TREE"):
                        for symbol, local_name in _use_tree_imports(use_tree):
                            parts = tuple(part for part in symbol.split("::") if part)
                            if "*" in parts:
                                if _is_public(item):
                                    module_issues[module_path].append(
                                        f"{file_key}: public glob re-export is unsupported ({symbol})"
                                    )
                                else:
                                    glob_target = tuple(part for part in parts if part != "*")
                                    if glob_target:
                                        module_globs[module_path].append(glob_target)
                                continue
                            target = tuple(part for part in symbol.split("::") if part)
                            if target and not module_self_import(module_path, target, local_name):
                                module_imports[module_path].setdefault(local_name, set()).add(target)

    index_modules(modules, "frontend-core", True)

    def load_workspace_crate(name: str) -> tuple[str, ...] | str:
        """Parse and index a workspace library crate; return its root key or an error."""
        state = loaded_crates.get(name)
        if state is not None:
            return state
        crate = workspace_crates[name]
        if crate.lib_root is None:
            state = f"workspace crate {name} has no library target"
        elif crate.edition != RUST_EDITION:
            state = f"{crate.manifest}: parser adapter requires edition {RUST_EDITION}"
        else:
            crate_directory = Path(crate.manifest).parent
            crate_modules, _, crate_errors, _ = _crate_module_sources(
                _CrateSources(root, executable, crate_directory),
                Path(crate.lib_root),
                f"workspace crate {name}",
                crate_directory,
            )
            if crate_errors:
                state = f"workspace crate {name}: {'; '.join(sorted(crate_errors))}"
            else:
                key = (f"::{name}",)
                keyed = {key + module_path: value for module_path, value in crate_modules.items()}
                modules.update(keyed)
                index_modules(keyed, f"workspace crate {name}", False)
                state = key
        loaded_crates[name] = state
        return state

    def workspace_issue(module_path: tuple[str, ...]) -> str | None:
        """A loaded module whose names a macro or public glob may hide is not consulted."""
        if crate_root(module_path) and module_issues.get(module_path):
            return f"{module_issues[module_path][0]} (consulted for a seam field type)"
        return None

    def resolve_path(
        module_path: tuple[str, ...],
        segments: tuple[str, ...],
        seen_imports: frozenset[tuple[tuple[str, ...], str]] = frozenset(),
        seen_paths: frozenset[tuple[tuple[str, ...], tuple[str, ...]]] = frozenset(),
    ) -> tuple[str, Any, str | None] | None:
        state = (module_path, segments)
        if state in seen_paths:
            return None
        seen_paths = seen_paths | {state}
        issue = workspace_issue(module_path)
        if issue is not None:
            return ("workspace_error", issue, None)
        root_path = crate_root(module_path)
        if not segments:
            return ("module", module_path, None)
        if segments[-1] == "self":
            segments = segments[:-1]
            if not segments:
                return ("module", module_path, None)

        if segments[0] in module_imports.get(module_path, {}):
            local_name = segments[0]
            import_key = (module_path, local_name)
            if import_key not in seen_imports:
                imported_targets = module_imports[module_path][local_name]
                resolved_options = [
                    resolve_path(
                        module_path,
                        imported + segments[1:],
                        seen_imports | {import_key},
                        seen_paths,
                    )
                    for imported in sorted(imported_targets)
                ]
                if any(resolved is None for resolved in resolved_options):
                    if len(imported_targets) > 1:
                        return ("ambiguous", tuple(resolved_options), local_name)
                    return ("unresolved_import", (module_path, local_name), local_name)
                matches = set(resolved_options)
                if len(matches) == 1:
                    return next(iter(matches))
                return ("ambiguous", tuple(sorted(matches, key=repr)), local_name)

        if segments[0] == "crate":
            normalized = segments[1:]
            search_module = root_path
        elif segments[0] == "self":
            normalized = segments[1:]
            search_module = module_path
        elif segments[0] == "super":
            parent = list(module_path)
            remainder = list(segments)
            while remainder and remainder[0] == "super":
                if len(parent) <= len(root_path):
                    return None
                parent.pop()
                remainder.pop(0)
            normalized = tuple(remainder)
            search_module = tuple(parent)
        else:
            normalized = segments
            search_module = module_path

        candidates = [search_module + normalized]
        # frontend-core keeps its crate-root fallback; a loaded workspace crate
        # resolves Rust 2021 paths only.
        if search_module and not root_path:
            candidates.append(normalized)
        for target in dict.fromkeys(candidates):
            if target in modules:
                return ("module", target, None)
            if target:
                origin = (target[:-1], target[-1])
                issue = workspace_issue(origin[0])
                if issue is not None:
                    return ("workspace_error", issue, origin[1])
                if origin in ambiguous_type_items:
                    return ("ambiguous", origin, origin[1])
                if origin in type_items:
                    return ("type", origin, origin[1])
                if origin in unsupported_types:
                    return ("unsupported_type", origin, origin[1])
                if origin in non_value_types:
                    return ("non_value_type", origin, origin[1])
                if origin in non_type_items:
                    return ("non_type_item", origin, origin[1])
            for split in range(len(target) - 1, 0, -1):
                module_candidate = target[:split]
                if module_candidate in modules:
                    resolved = resolve_path(
                        module_candidate,
                        target[split:],
                        seen_imports,
                        seen_paths,
                    )
                    if resolved is not None:
                        return resolved

        if len(segments) == 1 and segments[0] not in {"crate", "self", "super"}:
            glob_matches: set[tuple[str, Any, str | None]] = set()
            for glob_target in module_globs.get(module_path, []):
                resolved = resolve_path(
                    module_path,
                    glob_target + segments,
                    seen_imports,
                    seen_paths,
                )
                if resolved is not None:
                    glob_matches.add(resolved)
            if len(glob_matches) == 1:
                return next(iter(glob_matches))
            if len(glob_matches) > 1:
                return ("ambiguous", tuple(sorted(glob_matches, key=repr)), segments[0])

        canonical = "::".join(segments)
        # A built-in entry keeps precedence for its exact path.  Any other path
        # rooted at a workspace crate resolves to that crate's definition.
        if canonical in TRUSTED_FOREIGN_TYPE_CLASSES:
            return ("foreign", canonical, None)
        if segments[0] in workspace_crates:
            crate_key = (
                () if segments[0] == frontend_crate else load_workspace_crate(segments[0])
            )
            if isinstance(crate_key, str):
                return ("workspace_error", crate_key, segments[-1])
            return resolve_path(crate_key, ("crate",) + segments[1:], seen_imports, seen_paths)
        if canonical in foreign_classes:
            return ("foreign", canonical, None)
        return None


    def resolve_public_path(
        module_path: tuple[str, ...], symbol: str
    ) -> tuple[str, Any, str | None] | None:
        segments = tuple(part for part in symbol.split("::") if part)
        resolved = resolve_path(module_path, segments)
        if resolved is None:
            return None
        kind, target, name = resolved
        if kind == "type":
            origin = target
            if crate_root(origin[0]):
                return ("workspace_type", origin[0], origin[1])
            if origin in declared_types:
                return ("type", origin[0], origin[1])
            if origin in unsupported_types:
                return ("unsupported_type", origin[0], origin[1])
            return None
        if kind == "module" and crate_root(target):
            return ("workspace_module", target, None)
        if kind in {"unsupported_type", "non_value_type", "non_type_item"}:
            origin = target
            return (kind, origin[0], origin[1])
        return resolved

    def std_leaf_type(module_path: tuple[str, ...], segments: tuple[str, ...]) -> bool:
        """Whether a path that names no workspace or registered type is a std leaf.

        The path, after the module's own `use` renames, must be one of the
        exact std/core items in STD_LEAF_TYPE_PATHS, or a bare prelude or
        primitive name that no import names and no glob of an unindexed module
        could shadow.  Anything else stays an unresolved field type.
        """
        target = segments
        followed: set[str] = set()
        while target[0] in module_imports.get(module_path, {}):
            local_name = target[0]
            imported = module_imports[module_path][local_name]
            if local_name in followed or len(imported) != 1:
                return False
            followed.add(local_name)
            target = next(iter(imported)) + target[1:]
        if "::".join(target) in STD_LEAF_TYPE_PATHS:
            return True
        if target != segments or len(target) != 1 or target[0] not in PRELUDE_LEAF_TYPE_NAMES:
            return False
        for glob_target in module_globs.get(module_path, []):
            glob_module = resolve_path(module_path, glob_target)
            if glob_module is None or glob_module[0] not in {"module", "type"}:
                return False
        return True

    type_classes: dict[tuple[tuple[str, ...], str], str] = {}
    type_definitions: dict[tuple[tuple[str, ...], str], dict[str, Any]] = {}
    resolving_types: set[tuple[tuple[str, ...], str]] = set()

    def classify_origin(origin: tuple[tuple[str, ...], str]) -> str:
        cached = type_classes.get(origin)
        if cached is not None:
            return cached
        item = type_items.get(origin)
        if item is None:
            raise _InventoryError(
                f"unresolved frontend-core type {origin[0]}::{origin[1]}"
            )
        if origin in resolving_types:
            raise _InventoryError(
                f"recursive type ownership cannot be resolved for {origin[0]}::{origin[1]}"
            )
        resolving_types.add(origin)
        module_path, name = origin
        file_key = modules[module_path][0]

        def resolve_field_type(
            field_module: tuple[str, ...],
            segments: tuple[str, ...],
        ) -> str:
            if segments and segments[-1] == "Self":
                return classify_origin(origin)
            resolved = resolve_path(field_module, segments)
            if (resolved is None or resolved[0] == "unresolved_import") and std_leaf_type(
                field_module, segments
            ):
                return "scalar"
            if resolved is None:
                raise _InventoryError(
                    f"{file_key}::{name}: unresolved public seam field type {'::'.join(segments)}"
                )
            kind, target, target_name = resolved
            if kind == "type":
                return classify_origin(target)
            if kind == "foreign":
                classification = foreign_classes.get(target)
                if classification is None:
                    raise _InventoryError(
                        f"{file_key}::{name}: unresolved foreign seam type {target}"
                    )
                consulted_foreign.add(target)
                return classification
            if kind == "workspace_error":
                raise _InventoryError(f"{file_key}::{name}: {target}")
            if kind == "unsupported_type":
                source_path, syntax = unsupported_types[target]
                raise _InventoryError(
                    f"{source_path}: unsupported public seam type syntax ({syntax})"
                )
            if kind == "non_value_type":
                raise _InventoryError(
                    f"{file_key}::{name}: public seam field path is a trait, not a value "
                    f"({'::'.join(segments)})"
                )
            if kind == "non_type_item":
                raise _InventoryError(
                    f"{file_key}::{name}: public seam field path is a value, not a type "
                    f"({'::'.join(segments)})"
                )
            if kind == "ambiguous":
                raise _InventoryError(
                    f"{file_key}::{name}: ambiguous cfg/alternative seam field type "
                    f"{'::'.join(segments)}"
                )
            if kind == "unresolved_import":
                raise _InventoryError(
                    f"{file_key}::{name}: unresolved imported seam field type "
                    f"{'::'.join(segments)}"
                )
            raise _InventoryError(
                f"{file_key}::{name}: field path resolves to module, not a type "
                f"({'::'.join(segments)})"
            )

        try:
            if item.get("kind") == "TYPE_ALIAS":
                if _type_parameter_names(item):
                    raise _InventoryError(
                        f"{file_key}::{name}: generic public type aliases are unsupported"
                    )
                target = _type_alias_target(item, file_key, name)
                target_owned, target_borrowed = _type_ownership(
                    target,
                    file_key,
                    name,
                    resolve_type=lambda segments: resolve_field_type(
                        module_path, segments
                    ),
                )
                classification = (
                    "owning" if target_owned else "borrowed" if target_borrowed else "scalar"
                )
                definition = {
                    "name": name,
                    "fields": [],
                    "classification": classification,
                    "derived_clone": False,
                }
            else:
                symbol_prefix = "::".join(module_path) if modules[module_path][2] else ""

                def field_ownership(
                    field_type: dict[str, Any],
                    path: str,
                    owner: str,
                    generic_names: frozenset[str],
                    allow_generic: bool,
                ) -> tuple[bool, bool]:
                    return _type_ownership(
                        field_type,
                        path,
                        owner,
                        resolve_type=lambda segments: resolve_field_type(
                            module_path, segments
                        ),
                        generic_names=generic_names,
                        allow_generic=allow_generic,
                    )

                found = _type_definitions(
                    item,
                    file_key,
                    symbol_prefix=symbol_prefix,
                    include_private=True,
                    field_ownership=field_ownership,
                    baseline_names=not crate_root(module_path),
                )
                if len(found) != 1:
                    raise _InventoryError(
                        f"{file_key}::{name}: parser did not produce one type definition"
                    )
                _, definition = next(iter(found.items()))
                classification = definition["classification"]
        except _InventoryError:
            resolving_types.remove(origin)
            raise
        resolving_types.remove(origin)
        expected = TRUSTED_SEAM_TYPE_CLASSES.get(
            (
                file_key,
                (
                    f"{'::'.join(module_path)}::{name}"
                    if modules[module_path][2]
                    else name
                ),
            )
        )
        if expected is not None and expected != classification:
            raise _InventoryError(
                f"{file_key}::{name}: trusted seam classification is {expected}, "
                f"syntax implies {classification}"
            )
        type_classes[origin] = classification
        type_definitions[origin] = definition
        return classification

    exported_modules: set[tuple[str, ...]] = {()}
    exported_types: set[tuple[tuple[str, ...], str]] = set()
    public_aliases: dict[_TypeKey, tuple[tuple[str, ...], str]] = {}
    foreign_exports: dict[_TypeKey, str] = {}
    unresolved_exports: set[str] = set()
    changed = True
    while changed:
        changed = False
        for module_path in tuple(exported_modules):
            for child_path, is_public in module_children.get(module_path, []):
                if is_public and child_path not in exported_modules:
                    exported_modules.add(child_path)
                    changed = True
            for origin in declared_types:
                if origin[0] == module_path and origin not in exported_types:
                    exported_types.add(origin)
                    changed = True
            for file_key, use_node in public_uses.get(module_path, []):
                for tree in _direct_children(use_node, "USE_TREE"):
                    for symbol, local_name in _use_tree_imports(tree):
                        if "*" in symbol.split("::"):
                            unresolved_exports.add(
                                f"{file_key}: public glob re-export is unsupported ({symbol})"
                            )
                            continue
                        resolved = resolve_public_path(module_path, symbol)
                        if resolved is None or resolved[0] in {"workspace_error", "workspace_module"}:
                            final_name = symbol.rstrip(":").split("::")[-1]
                            if final_name[:1].isupper():
                                detail = (
                                    f": {resolved[1]}"
                                    if resolved is not None and resolved[0] == "workspace_error"
                                    else ""
                                )
                                unresolved_exports.add(
                                    f"{file_key}: cannot resolve public type re-export {symbol}{detail}"
                                )
                            continue
                        kind, target_module, name = resolved
                        if kind == "module":
                            if target_module not in exported_modules:
                                exported_modules.add(target_module)
                                changed = True
                        elif kind == "type":
                            origin = (target_module, name or "")
                            if origin not in exported_types:
                                exported_types.add(origin)
                                changed = True
                            if local_name != (name or ""):
                                _, _, inline = modules[module_path]
                                symbol_prefix = "::".join(module_path) if inline else ""
                                alias_symbol = (
                                    f"{symbol_prefix}::{local_name}"
                                    if symbol_prefix
                                    else local_name
                                )
                                public_aliases[
                                    _TypeKey(file_key, alias_symbol)
                                ] = origin
                        elif kind == "foreign":
                            import_targets = module_imports.get(module_path, {}).get(
                                local_name, set()
                            )
                            if any(
                                import_target and import_target[0] in FRONTEND_CRATE_NAMES
                                for import_target in import_targets
                            ):
                                _, _, inline = modules[module_path]
                                symbol_prefix = "::".join(module_path) if inline else ""
                                exported_name = (
                                    f"{symbol_prefix}::{local_name}"
                                    if symbol_prefix
                                    else local_name
                                )
                                foreign_exports[_TypeKey(file_key, exported_name)] = target_module
                        elif kind == "workspace_type":
                            # A re-exported workspace type is a seam alias classified
                            # from its definition, like a local re-export.
                            _, _, inline = modules[module_path]
                            symbol_prefix = "::".join(module_path) if inline else ""
                            exported_name = (
                                f"{symbol_prefix}::{local_name}" if symbol_prefix else local_name
                            )
                            expected_target = TRUSTED_FOREIGN_REEXPORTS.get((file_key, exported_name))
                            if expected_target is not None:
                                unresolved_exports.add(
                                    f"{file_key}::{exported_name}: trusted foreign re-export is "
                                    f"{expected_target}, source exports "
                                    f"{'::'.join(target_module)[2:]}::{name}"
                                )
                            public_aliases[_TypeKey(file_key, exported_name)] = (
                                target_module,
                                name or "",
                            )
                        elif kind in {"non_value_type", "non_type_item"}:
                            continue
                        elif kind == "ambiguous":
                            unresolved_exports.add(
                                f"{file_key}: ambiguous cfg/alternative public re-export "
                                f"{symbol}"
                            )
                        elif kind == "unresolved_import":
                            unresolved_exports.add(
                                f"{file_key}: unresolved public re-export import {symbol}"
                            )
                        elif kind == "unsupported_type":
                            source_path, syntax = unsupported_types[(target_module, name or "")]
                            unresolved_exports.add(
                                f"{source_path}: unsupported public seam type syntax ({syntax})"
                            )
                        else:
                            unresolved_exports.add(
                                f"{file_key}: unsupported public re-export kind {kind} ({symbol})"
                            )

    errors = set(unresolved_exports)
    for module_path in exported_modules:
        errors.update(module_issues.get(module_path, []))
        for origin, (source_path, syntax) in unsupported_types.items():
            if origin[0] == module_path:
                errors.add(
                    f"{source_path}: unsupported public seam type syntax ({syntax})"
                )

    for key, target in foreign_exports.items():
        expected_target = TRUSTED_FOREIGN_REEXPORTS.get((key.path, key.symbol))
        if expected_target is not None and target != expected_target:
            errors.add(
                f"{key.path}::{key.symbol}: trusted foreign re-export is "
                f"{expected_target}, source exports {target}"
            )

    declared_by_name: dict[str, set[tuple[tuple[str, ...], str]]] = {}
    for origin in declared_types:
        declared_by_name.setdefault(origin[1], set()).add(origin)
    explicit_clone_origins: dict[tuple[tuple[str, ...], str], set[tuple[str, ...]]] = {}
    for module_path in frontend_modules:
        container = modules[module_path][1]
        for name, impl_cfgs in _explicit_clone_impls(container, modules[module_path][0]).items():
            conditions = {module_cfgs[module_path] + cfgs for cfgs in impl_cfgs}
            local_origin = (module_path, name)
            if local_origin in declared_types:
                explicit_clone_origins.setdefault(local_origin, set()).update(conditions)
                continue
            matches = declared_by_name.get(name, set())
            if len(matches) == 1:
                explicit_clone_origins.setdefault(next(iter(matches)), set()).update(conditions)
            elif len(matches) > 1:
                errors.add(
                    f"{modules[module_path][0]}: ambiguous explicit Clone target {name}"
                )

    definitions: dict[_TypeKey, dict[str, Any]] = {}
    origin_keys: dict[tuple[tuple[str, ...], str], _TypeKey] = {}
    for origin in sorted(exported_types):
        file_key, _, inline = modules[origin[0]]
        symbol_prefix = "::".join(origin[0]) if inline else ""
        symbol = f"{symbol_prefix}::{origin[1]}" if symbol_prefix else origin[1]
        key = _TypeKey(file_key, symbol)
        try:
            classify_origin(origin)
        except _InventoryError as error:
            errors.add(str(error))
            continue
        definitions[key] = type_definitions[origin]
        origin_keys[origin] = key

    for key, origin in public_aliases.items():
        try:
            classification = classify_origin(origin)
        except _InventoryError as error:
            errors.add(str(error))
            continue
        definitions[key] = {
            "name": key.symbol,
            "fields": [],
            "classification": classification,
            "derived_clone": False,
        }

    for key, target in foreign_exports.items():
        classification = foreign_classes.get(target)
        if classification is None:
            errors.add(f"{key.path}::{key.symbol}: unresolved foreign seam type {target}")
            continue
        consulted_foreign.add(target)
        definitions[key] = {
            "name": key.symbol,
            "fields": [],
            "classification": classification,
            "derived_clone": False,
        }

    classifications = {
        key: definition["classification"] for key, definition in definitions.items()
    }
    for key, classification in classifications.items():
        expected = TRUSTED_SEAM_TYPE_CLASSES.get((key.path, key.symbol))
        if expected is not None and expected != classification:
            errors.add(
                f"{key.path}::{key.symbol}: trusted seam classification is {expected}, "
                f"syntax implies {classification}"
            )

    def provenance(*parts: tuple[str, ...]) -> str:
        return _cfg_label(tuple(dict.fromkeys(predicate for part in parts for predicate in part)))

    violations: set[_RecordKey] = set()
    for origin, key in origin_keys.items():
        definition = definitions[key]
        if definition["classification"] != "owning":
            continue
        type_cfgs = module_cfgs[origin[0]] + definition.get("cfgs", ())
        for field, public, field_cfgs in definition["fields"]:
            if public:
                violations.add(
                    _RecordKey(
                        "owning_public_field",
                        key.path,
                        f"{key.symbol}::{field}",
                        provenance(type_cfgs, field_cfgs),
                        "public_field_of_owning_view",
                    )
                )
        if definition["derived_clone"]:
            violations.add(
                _RecordKey(
                    "owning_derived_clone",
                    key.path,
                    key.symbol,
                    provenance(type_cfgs, definition.get("clone_cfgs", ())),
                    "derived_Clone_on_owning_view",
                )
            )
        for impl_cfgs in sorted(explicit_clone_origins.get(origin, ())):
            violations.add(
                _RecordKey(
                    "owning_explicit_clone",
                    key.path,
                    key.symbol,
                    provenance(type_cfgs, impl_cfgs),
                    "explicit_Clone_on_owning_view",
                )
            )
    return classifications, violations, sorted(errors)


def _cfg_for_scope(path: str, node_scope: str) -> str:
    if path.startswith("crates/script/tests/"):
        return "integration-test"
    return node_scope

def _type_alias_targets(tree: dict[str, Any], imported: dict[str, str]) -> dict[str, str]:
    aliases = dict(imported)
    type_nodes: list[tuple[str, tuple[tuple[str, ...], ...]]] = []
    for node, _, _, _, _ in _walk_ast(tree, "production", ()):
        if node.get("kind") != "TYPE_ALIAS":
            continue
        name = _first_name(node)
        if not name:
            continue
        paths = tuple(
            _path_segments(path_node)
            for path_node, parent_kind, _, _, in_use in _walk_ast(node, "production", ())
            if path_node.get("kind") == "PATH"
            and parent_kind != "PATH"
            and not in_use
        )
        type_nodes.append((name, paths))
    for _ in range(len(type_nodes) + 1):
        changed = False
        for name, paths in type_nodes:
            if name in aliases:
                continue
            raw_target = None
            for segments in paths:
                if _is_raw_representation(segments):
                    raw_target = "::".join(segments)
                    break
                for index, segment in enumerate(segments):
                    target = aliases.get(segment)
                    if target is not None and _is_raw_symbol(target):
                        suffix = "::".join(segments[index + 1 :])
                        raw_target = target + (f"::{suffix}" if suffix else "")
                        break
                if raw_target is not None:
                    break
            if raw_target is not None:
                aliases[name] = raw_target
                changed = True
        if not changed:
            break
    return aliases


def _is_raw_symbol(symbol: str) -> bool:
    segments = tuple(part for part in symbol.split("::") if part)
    return _is_raw_representation(segments)


def _exposed_type_paths(node: dict[str, Any]) -> Iterator[dict[str, Any]]:
    for child, _, _, parent_kind, in_use in _walk_ast(node, "production", ()):
        if child.get("kind") == "PATH_TYPE":
            paths = _direct_children(child, "PATH")
            if paths:
                yield paths[0]
        elif child.get("kind") == "PATH" and parent_kind != "PATH" and not in_use:
            # Qualified paths in type syntax are represented by PATH_TYPE in
            # ordinary items; this branch covers associated type bounds.
            yield child




def _public_api_exposures(
    path: str,
    tree: dict[str, Any],
    root_scope: str,
    imported_aliases: dict[str, str],
) -> set[_RecordKey]:
    exposures: set[_RecordKey] = set()
    aliases = _type_alias_targets(tree, imported_aliases)

    def record_type_paths(
        item: dict[str, Any],
        symbol_prefix: str,
        scope: str,
        classification: str,
    ) -> None:
        for path_node in _exposed_type_paths(item):
            segments = _path_segments(path_node)
            raw_symbol: str | None = None
            if _is_raw_representation(segments):
                raw_symbol = "::".join(segments)
            elif segments and segments[0] in aliases and _is_raw_symbol(aliases[segments[0]]):
                raw_symbol = aliases[segments[0]]
            if raw_symbol is not None:
                exposures.add(
                    _RecordKey(
                        "transitive_raw_representation_exposure",
                        path,
                        f"{symbol_prefix}:{raw_symbol}",
                        scope,
                        classification,
                    )
                )

    for node, scope, _, _, in_use in _walk_ast(tree, root_scope, ()):
        kind = node.get("kind")
        actual_scope = _cfg_for_scope(path, scope)
        if kind == "USE" and _is_public(node):
            for use_tree in _direct_children(node, "USE_TREE"):
                for symbol, _ in _use_tree_imports(use_tree):
                    if _is_raw_symbol(symbol):
                        exposures.add(
                            _RecordKey(
                                "transitive_raw_representation_exposure",
                                path,
                                f"reexport:{symbol}",
                                actual_scope,
                                "public_reexport",
                            )
                        )
            continue
        if kind in {"STRUCT", "ENUM", "TYPE_ALIAS", "FN", "CONST", "STATIC"} and _is_public(node):
            name = _first_name(node) or kind.lower()
            if kind in {"STRUCT", "ENUM"}:
                for field_list in _children(node):
                    if field_list.get("kind") == "RECORD_FIELD_LIST":
                        for field in _direct_children(field_list, "RECORD_FIELD"):
                            if kind == "ENUM" or _is_public(field):
                                field_name = _first_name(field) or "tuple-field"
                                record_type_paths(
                                    field,
                                    f"{name}::{field_name}",
                                    actual_scope,
                                    "public_field",
                                )
                    elif field_list.get("kind") == "TUPLE_FIELD_LIST":
                        for index, field in enumerate(_direct_children(field_list, "TUPLE_FIELD")):
                            if kind == "ENUM" or _is_public(field):
                                record_type_paths(
                                    field,
                                    f"{name}::{index}",
                                    actual_scope,
                                    "public_field",
                                )
            elif kind == "FN":
                for child in _children(node):
                    if child.get("kind") in {"PARAM_LIST", "RET_TYPE", "GENERIC_PARAM_LIST"}:
                        record_type_paths(child, name, actual_scope, "public_signature")
            elif kind == "TYPE_ALIAS":
                record_type_paths(node, name, actual_scope, "public_type_alias")
            else:
                for child in _children(node):
                    if child.get("kind") in {"TYPE", "PATH_TYPE"}:
                        record_type_paths(child, name, actual_scope, "public_item_type")
    return exposures


def _script_records(root: Path) -> tuple[set[_RecordKey], set[_RecordKey], list[str]]:
    errors: list[str] = []
    try:
        executable = _parser_executable()
    except _InventoryError as error:
        return set(), set(), [f"parser: {error}"]
    parsed, parse_errors = _read_rust_files(root, executable)
    errors.extend(parse_errors)
    if errors:
        return set(), set(), errors
    root_scopes, scope_errors = _root_test_scopes(parsed)
    errors.extend(scope_errors)
    imports: set[_RecordKey] = set()
    exceptions: set[_RecordKey] = set()

    for path, entry in sorted(parsed.items()):
        root_scope = (
            "integration-test"
            if path.startswith("crates/script/tests/")
            else root_scopes.get(path, "production")
        )
        tree = entry["tree"]
        aliases: dict[str, str] = {}

        def register_nav(symbol: str, node_scope: str, classification: str) -> None:
            if not _is_nav_raw(tuple(symbol.split("::"))):
                return
            actual_scope = _cfg_for_scope(path, node_scope)
            category = (
                "test_nav_implementation_import"
                if _is_test_scope(actual_scope)
                else "production_nav_implementation_import"
            )
            if _is_exception_scope(path, actual_scope):
                exceptions.add(
                    _RecordKey(
                        "test_import_exception",
                        path,
                        symbol,
                        actual_scope,
                        "exact_test_fixture_oracle_import",
                    )
                )
            else:
                imports.add(
                    _RecordKey(category, path, symbol, actual_scope, classification)
                )

        for node, scope, _, parent_kind, in_use in _walk_ast(tree, root_scope, ()):
            kind = node.get("kind")
            if kind == "MACRO_RULES":
                if _macro_nav_paths(node):
                    errors.append(f"{path}: raw navigation path in macro definition is unsupported")
                continue
            if kind == "MACRO_CALL":
                for symbol in _macro_nav_paths(node):
                    register_nav(symbol, scope, "macro_token_path")
                if _macro_call_name(node) == "include":
                    target = _macro_string_argument(node)
                    actual_scope = _cfg_for_scope(path, scope)
                    if (path, actual_scope, target) not in ALLOWED_TEST_INCLUDES:
                        errors.append(f"{path}: include! is unsupported outside its pinned test fixture")
                continue
            if kind == "USE":
                for use_tree in _direct_children(node, "USE_TREE"):
                    for symbol, local_name in _use_tree_imports(use_tree):
                        register_nav(symbol, scope, "use_import")
                        if _is_raw_symbol(symbol):
                            aliases[local_name] = symbol
                continue
            if kind != "PATH" or parent_kind == "PATH" or in_use:
                continue
            segments = _path_segments(node)
            if _is_nav_raw(segments):
                register_nav("::".join(segments), scope, "qualified_path")

        imports.update(_public_api_exposures(path, tree, root_scope, aliases))

    # Imports in modules outside the contract's exact test targets remain
    # ordinary migration debt; there is no file-wide test exemption.
    for key in exceptions:
        if key.path not in TEST_EXCEPTION_TARGETS or key.cfg_or_target not in TEST_EXCEPTION_TARGETS[key.path]:
            errors.append(
                f"{key.path}: test exception is outside the exact §21 §5.3 target list"
            )
    return imports, exceptions, errors


def _expected_id(key: _RecordKey) -> str:
    return "|".join(
        (key.category, key.path, key.cfg_or_target, key.symbol, key.classification)
    )


def _expected_type_id(key: _TypeKey) -> str:
    return f"seam_type|{key.path}|{key.symbol}"


def _relative_source_path(value: Any) -> bool:
    if not isinstance(value, str) or not value or "\\" in value:
        return False
    path = Path(value)
    return (
        not path.is_absolute()
        and path.suffix == ".rs"
        and all(part not in {"", ".", ".."} for part in path.parts)
        and "*" not in value
    )


def _validate_inventory_shape(raw: Any) -> tuple[dict[str, Any] | None, list[str]]:
    errors: list[str] = []
    if not isinstance(raw, dict) or set(raw) != TOP_FIELDS:
        return None, ["inventory schema: top-level fields must be exactly schema_version, seam_types, violations, test_exceptions"]
    if type(raw["schema_version"]) is not int or raw["schema_version"] != 1:
        errors.append("inventory schema: schema_version must be integer 1")
    for name in ("seam_types", "violations", "test_exceptions"):
        if not isinstance(raw[name], list):
            errors.append(f"inventory schema: {name} must be an array")
    if errors:
        return None, errors

    seen_ids: set[str] = set()
    seen_keys: set[tuple[Any, ...]] = set()
    for index, row in enumerate(raw["seam_types"]):
        where = f"seam_types[{index}]"
        if not isinstance(row, dict) or set(row) != TYPE_FIELDS:
            errors.append(f"inventory schema: {where} has unknown or missing fields")
            continue
        record_id = row.get("id")
        if not isinstance(record_id, str) or not record_id.strip():
            errors.append(f"inventory schema: {where}.id must be nonblank")
            continue
        if record_id in seen_ids:
            errors.append(f"inventory schema: duplicate id {record_id}")
        seen_ids.add(record_id)
        path, symbol = row.get("path"), row.get("symbol")
        if not _relative_source_path(path) or not isinstance(symbol, str) or not symbol.strip():
            errors.append(f"inventory schema: {where} needs exact path and symbol")
        classification = row.get("classification")
        if classification not in TYPE_CLASSIFICATIONS:
            errors.append(f"inventory schema: {where} has unknown seam classification")
        if row.get("owner") != "T3":
            errors.append(f"inventory schema: {where} owner must be T3")
        if not isinstance(row.get("reason"), str) or not row["reason"].strip():
            errors.append(f"inventory schema: {where} reason must be nonblank")
        key = (path, symbol)
        if key in seen_keys:
            errors.append(f"inventory schema: duplicate matching seam type {path}::{symbol}")
        seen_keys.add(key)
        if record_id != _expected_type_id(_TypeKey(path or "", symbol or "")):
            errors.append(f"inventory schema: {where}.id is not the stable path/symbol id")

    for list_name in ("violations", "test_exceptions"):
        allowed_categories = VIOLATION_CATEGORIES if list_name == "violations" else EXCEPTION_CATEGORIES
        for index, row in enumerate(raw[list_name]):
            where = f"{list_name}[{index}]"
            if not isinstance(row, dict) or set(row) != RECORD_FIELDS:
                errors.append(f"inventory schema: {where} has unknown or missing fields")
                continue
            record_id = row.get("id")
            if not isinstance(record_id, str) or not record_id.strip():
                errors.append(f"inventory schema: {where}.id must be nonblank")
                continue
            if record_id in seen_ids:
                errors.append(f"inventory schema: duplicate id {record_id}")
            seen_ids.add(record_id)
            category = row.get("category")
            if category not in allowed_categories:
                errors.append(f"inventory schema: {where} has unknown category")
            path, symbol = row.get("path"), row.get("symbol")
            if not _relative_source_path(path) or not isinstance(symbol, str) or not symbol.strip():
                errors.append(f"inventory schema: {where} needs exact path and symbol")
            cfg = row.get("cfg_or_target")
            if not isinstance(cfg, str) or not cfg.strip() or "*" in cfg:
                errors.append(f"inventory schema: {where}.cfg_or_target must be exact and nonblank")
            if row.get("owner") != ("T3" if category in {"owning_public_field", "owning_derived_clone", "owning_explicit_clone"} else "T5"):
                errors.append(f"inventory schema: {where} has unknown or incorrect owner")
            if not isinstance(row.get("reason"), str) or not row["reason"].strip():
                errors.append(f"inventory schema: {where}.reason must be nonblank")
            endpoint = row.get("removal_endpoint")
            expected_endpoint = "T3.1" if row.get("owner") == "T3" else "T5.1"
            if endpoint not in ENDPOINTS or endpoint != expected_endpoint:
                errors.append(f"inventory schema: {where} has unknown or incorrect removal endpoint")
            classification = row.get("classification")
            if not isinstance(classification, str) or not classification.strip():
                errors.append(f"inventory schema: {where}.classification must be nonblank")
            key = (category, path, symbol, cfg, classification)
            if key in seen_keys:
                errors.append(f"inventory schema: duplicate matching record {key}")
            seen_keys.add(key)
            if record_id != _expected_id(
                _RecordKey(
                    str(category or ""),
                    str(path or ""),
                    str(symbol or ""),
                    str(cfg or ""),
                    str(classification or ""),
                )
            ):
                errors.append(f"inventory schema: {where}.id is not the stable path/symbol id")
            if list_name == "test_exceptions":
                allowed_scopes = TEST_EXCEPTION_TARGETS.get(str(path), frozenset())
                if category != "test_import_exception" or cfg not in allowed_scopes:
                    errors.append(f"inventory schema: {where} is not an exact §21 §5.3 test exception")
    return raw, errors


def _load_inventory(root: Path) -> tuple[dict[str, Any] | None, list[str]]:
    path = root / INVENTORY_RELATIVE
    if _traverses_symlink(root, INVENTORY_RELATIVE):
        return None, [f"inventory: {INVENTORY_RELATIVE.as_posix()} must not traverse a symlink"]
    try:
        raw = json.loads(
            path.read_text(encoding="utf-8"),
            object_pairs_hook=_object_without_duplicate_keys,
        )
    except (OSError, UnicodeError, RecursionError, json.JSONDecodeError, _InventoryError) as error:
        return None, [f"inventory: cannot load {INVENTORY_RELATIVE.as_posix()}: {error}"]
    return _validate_inventory_shape(raw)



@dataclass(frozen=True)
class _Snapshot:
    inventory: dict[str, Any] | None
    classifications: dict[_TypeKey, str]
    violations: frozenset[_RecordKey]
    exceptions: frozenset[_RecordKey]
    errors: tuple[str, ...]
    foreign_types: dict[str, str]
def _analyze_tree(root: Path) -> _Snapshot:



    errors: list[str] = _check_rust_editions(root)
    inventory, shape_errors = _load_inventory(root)
    errors.extend(shape_errors)
    workspace_crates, workspace_errors = _workspace_crates(root)
    errors.extend(workspace_errors)
    foreign_rows, foreign_errors = _load_foreign_types(root, frozenset(workspace_crates))
    errors.extend(foreign_errors)
    consulted_foreign: set[str] = set()
    classifications, seam_violations, seam_errors = _seam_records(
        root,
        {**TRUSTED_FOREIGN_TYPE_CLASSES, **foreign_rows},
        consulted_foreign,
        workspace_crates,
    )
    errors.extend(seam_errors)
    nav_violations, nav_exceptions, nav_errors = _script_records(root)
    errors.extend(nav_errors)
    actual_violations = set(seam_violations) | set(nav_violations)
    for type_path in sorted(set(foreign_rows) - consulted_foreign):
        errors.append(f"stale foreign type classification: {type_path}")
    if inventory is not None:
        registered_types: dict[_TypeKey, str] = {}
        for row in inventory["seam_types"]:
            if isinstance(row, dict) and isinstance(row.get("path"), str) and isinstance(row.get("symbol"), str):
                registered_types[_TypeKey(row["path"], row["symbol"])] = str(row.get("classification"))
        for key in sorted(set(classifications) - set(registered_types)):
            errors.append(f"seam type missing from registry: {key.path}::{key.symbol}")
        for key in sorted(set(registered_types) - set(classifications)):
            errors.append(f"stale seam type classification: {key.path}::{key.symbol}")
        for key, actual_class in sorted(classifications.items()):
            registered = registered_types.get(key)
            if registered is not None and registered != actual_class:
                errors.append(
                    f"seam classification disagrees with syntax: {key.path}::{key.symbol} "
                    f"is {actual_class}, registry says {registered}"
                )
        registered_violations: set[_RecordKey] = set()
        for row in inventory["violations"]:
            if not isinstance(row, dict):
                continue
            try:
                registered_violations.add(
                    _RecordKey(
                        str(row["category"]),
                        str(row["path"]),
                        str(row["symbol"]),
                        str(row["cfg_or_target"]),
                        str(row["classification"]),
                    )
                )
            except KeyError:
                continue
        registered_exceptions: set[_RecordKey] = set()
        for row in inventory["test_exceptions"]:
            if not isinstance(row, dict):
                continue
            try:
                registered_exceptions.add(
                    _RecordKey(
                        str(row["category"]),
                        str(row["path"]),
                        str(row["symbol"]),
                        str(row["cfg_or_target"]),
                        str(row["classification"]),
                    )
                )
            except KeyError:
                continue
        for key in sorted(actual_violations - registered_violations):
            errors.append(f"unregistered inventory violation: {_expected_id(key)}")
        for key in sorted(registered_violations - actual_violations):
            errors.append(f"stale inventory exemption: {_expected_id(key)}")
        for key in sorted(nav_exceptions - registered_exceptions):
            errors.append(f"unregistered test import exception: {_expected_id(key)}")
        for key in sorted(registered_exceptions - nav_exceptions):
            errors.append(f"stale test import exception: {_expected_id(key)}")
    return _Snapshot(
        inventory,
        classifications,
        frozenset(actual_violations),
        frozenset(nav_exceptions),
        tuple(sorted(set(errors))),
        foreign_rows,
    )


def check_tree(root: Path) -> list[str]:
    """Validate a single tree's strict registry and exact source coverage."""
    snapshot = _analyze_tree(Path(root))
    return list(snapshot.errors)


def check_transition(base: Path, candidate: Path) -> list[str]:
    """Return exact widening findings between two independently parsed trees.

    Findings are stable IDs suitable for the trusted gate's external approval
    delta.  Invalid schemas, stale exemptions and parser failures are always
    returned as hard errors and cannot be turned into an approval delta.
    """
    before = _analyze_tree(Path(base))
    after = _analyze_tree(Path(candidate))
    errors = [f"base: {error}" for error in before.errors]
    errors.extend(f"candidate: {error}" for error in after.errors)
    if errors:
        return sorted(set(errors))

    findings: set[str] = set()
    before_types = before.classifications
    after_types = after.classifications
    for key in sorted(set(after_types) - set(before_types)):
        findings.add(f"new seam type: {key.path}::{key.symbol}")
    for key in sorted(set(before_types) & set(after_types)):
        old, new = before_types[key], after_types[key]
        if old != new:
            findings.add(
                f"seam classification changed: {key.path}::{key.symbol} {old}->{new}"
            )
    for key in sorted(after.violations - before.violations):
        findings.add(f"new inventory violation: {_expected_id(key)}")
    for key in sorted(after.exceptions - before.exceptions):
        findings.add(f"new test import exception: {_expected_id(key)}")
    before_foreign = before.foreign_types
    after_foreign = after.foreign_types
    for type_path in sorted(set(after_foreign) - set(before_foreign)):
        findings.add(f"new foreign type classification: {type_path} {after_foreign[type_path]}")
    for type_path in sorted(set(before_foreign) & set(after_foreign)):
        old, new = before_foreign[type_path], after_foreign[type_path]
        if old != new:
            findings.add(f"foreign type classification changed: {type_path} {old}->{new}")
    return sorted(findings)
