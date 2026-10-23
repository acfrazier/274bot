#!/usr/bin/env python3
"""Generate the nav action deny list from compiler-resolved Rustdoc items.

The inventory reads ``api`` with default features and ``client`` with default
features plus ``test-support``. The wider client profile includes methods that
Cargo feature unification can expose to planner production code.
The generated config marks feature-gated paths ``allow-invalid`` when the
default build cannot resolve them.
SHA-256 rows for each seed module's source files and subtrees make edits,
including inactive ``cfg`` declarations and relocated exported macros,
require a deliberate baseline refresh.
"""

from __future__ import annotations

import argparse
import bisect
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any, Iterator

ROOT = Path(__file__).resolve().parents[2]
NAV_CRATE = ROOT / "crates" / "nav"
ROOT_MANIFEST = ROOT / "Cargo.toml"
CONFIG_PATH = NAV_CRATE / "clippy.toml"
INVENTORY_PATH = ROOT / "tools" / "architecture" / "nav-clippy-inventory.json"
PROBE_SOURCE = ROOT / "tools" / "architecture" / "nav_clippy_probes.rs"
PROBE_MANIFEST = ROOT / "tools" / "architecture" / "nav_clippy_probes.json"
EXTERNAL_FACADE_SOURCE = ROOT / "tools" / "architecture" / "nav_clippy_external_facade" / "src" / "lib.rs"

SEEDS = {
    "api": ["api::interact", "api::settle", "api::prot"],
    "client": [
        "client::client",
        "client::io::client_prot",
        "client::io::client_prot_289",
        "client::io::client_stream",
    ],
}

RUSTDOC_FEATURES = {
    "api": (),
    "client": ("test-support",),
}

# The method is absent by default, so its config entry needs allow-invalid.
FEATURED_SEED_METHODS = {
    "client/test-support": ("client::io::client_stream::connect_ws_plain",),
}

SOURCE_FILES = [
    "crates/api/src/lib.rs",
    "crates/api/src/interact.rs",
    "crates/api/src/settle.rs",
    "crates/api/src/prot.rs",
    "vendor/fr-client-rust/crates/client/src/lib.rs",
    "vendor/fr-client-rust/crates/client/src/client/mod.rs",
    "vendor/fr-client-rust/crates/client/src/client/client.rs",
    "vendor/fr-client-rust/crates/client/src/io/mod.rs",
    "vendor/fr-client-rust/crates/client/src/io/client_prot.rs",
    "vendor/fr-client-rust/crates/client/src/io/client_prot_289.rs",
    "vendor/fr-client-rust/crates/client/src/io/client_stream.rs",
]

SOURCE_SUBTREES = [
    "crates/api/src/interact",
    "crates/api/src/settle",
    "crates/api/src/prot",
    "vendor/fr-client-rust/crates/client/src/client/client",
    "vendor/fr-client-rust/crates/client/src/io/client_prot",
    "vendor/fr-client-rust/crates/client/src/io/client_prot_289",
    "vendor/fr-client-rust/crates/client/src/io/client_stream",
]

LINT_NAMES = {
    "type": "types",
    "method": "methods",
    "macro": "macros",
}

KIND_MAP = {
    "module": "module",
    "struct": "type",
    "enum": "type",
    "union": "type",
    "trait": "type",
    "trait_alias": "type",
    "type_alias": "type",
    "opaque_ty": "type",
    "foreign_type": "type",
    "primitive": "type",
    "assoc_type": "type",
    "associated_type": "type",
    "function": "method",
    "method": "method",
    "assoc_fn": "method",
    "associated_function": "method",
    "tymethod": "method",
    "foreign_function": "method",
    "macro": "macro",
    "proc_macro": "macro",
    "proc_macro_attribute": "macro",
    "proc_macro_derive": "macro",
    "macro_attribute": "macro",
    "macro_derive": "macro",
    "constant": "value",
    "static": "value",
    "assoc_const": "value",
    "associated_constant": "value",
    "struct_field": "field",
    "field": "field",
    "variant": "variant",
    "import": "reexport",
    "use": "reexport",
    "extern_crate": "crate_alias",
}

PROBE_LINTS = {
    "clippy::disallowed_types",
    "clippy::disallowed_methods",
    "clippy::disallowed_macros",
}


class InventoryError(Exception):
    pass


def run(
    command: list[str],
    *,
    cwd: Path,
    env: dict[str, str] | None = None,
    target_dir: Path | None = None,
) -> subprocess.CompletedProcess[str]:
    command_env = os.environ.copy() if env is None else env.copy()
    command_env["CARGO_BUILD_JOBS"] = "4"
    if target_dir is None:
        command_env.pop("CARGO_TARGET_DIR", None)
    else:
        command_env["CARGO_TARGET_DIR"] = str(target_dir)
    if command[0] == "cargo":
        command = ["nice", "-n", "10", *command]
    return subprocess.run(
        command,
        cwd=cwd,
        env=command_env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )


def cargo_target_directory() -> Path:
    result = run(
        [
            "cargo",
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
            str(ROOT_MANIFEST),
        ],
        cwd=ROOT,
    )
    if result.returncode != 0:
        raise InventoryError(f"cargo metadata failed for {ROOT_MANIFEST}:\n{result.stderr}")
    return Path(json.loads(result.stdout)["target_directory"])


def rustdoc_json(
    package_name: str, target_dir: Path, features: tuple[str, ...] = ()
) -> tuple[str, Path]:
    env = os.environ.copy()
    env["RUSTC_BOOTSTRAP"] = "1"
    command = [
        "cargo",
        "rustdoc",
        "--manifest-path",
        str(ROOT_MANIFEST),
        "--package",
        package_name,
    ]
    if features:
        command.extend(["--features", ",".join(features)])
    command.extend(
        [
            "--lib",
            "--",
            "-Z",
            "unstable-options",
            "--output-format",
            "json",
        ]
    )
    result = run(command, cwd=ROOT, env=env)
    if result.returncode != 0:
        raise InventoryError(
            f"cargo rustdoc failed for {package_name}:\n{result.stdout}{result.stderr}"
        )
    crate_name = package_name.replace("-", "_")
    path = target_dir / "doc" / f"{crate_name}.json"
    if not path.is_file():
        raise InventoryError(f"rustdoc did not create {path}")
    return package_name, path


def rustdoc_kind(item: dict[str, Any], fallback: str | None = None) -> str | None:
    inner = item.get("inner")
    if isinstance(inner, dict) and inner:
        return next(iter(inner))
    return fallback


def item_payload(item: dict[str, Any]) -> dict[str, Any]:
    inner = item.get("inner")
    if not isinstance(inner, dict) or not inner:
        return {}
    payload = next(iter(inner.values()))
    return payload if isinstance(payload, dict) else {}


def stable_value(value: Any) -> Any:
    if isinstance(value, dict):
        return {
            key: stable_value(entry)
            for key, entry in sorted(value.items())
            if key not in {"id", "crate_id", "span", "docs"}
        }
    if isinstance(value, list):
        return [stable_value(entry) for entry in value]
    return value


def item_declared_type(item: dict[str, Any], kind: str) -> Any | None:
    payload = item_payload(item)
    if kind == "struct_field":
        return payload.get("type")
    if kind in {"constant", "static", "assoc_const"}:
        return payload.get("type")
    if kind == "type_alias":
        return payload
    return None


class Rustdoc:
    def __init__(self, crate_name: str, path: Path) -> None:
        self.crate_name = crate_name
        self.path = path
        self.raw = json.loads(path.read_text(encoding="utf-8"))
        self.index: dict[str, dict[str, Any]] = {
            str(key): value for key, value in self.raw.get("index", {}).items()
        }
        self.paths: dict[str, dict[str, Any]] = {
            str(key): value for key, value in self.raw.get("paths", {}).items()
        }
        root = self.raw.get("root")
        if isinstance(root, dict):
            root = root.get("id")
        self.root_id = str(root)
        self.path_ids: dict[tuple[str, ...], list[str]] = {}
        for item_id, entry in self.paths.items():
            path_parts = entry.get("path")
            if isinstance(path_parts, list):
                self.path_ids.setdefault(tuple(str(part) for part in path_parts), []).append(item_id)

    def item(self, item_id: Any) -> dict[str, Any] | None:
        if isinstance(item_id, dict):
            item_id = item_id.get("id")
        if item_id is None:
            return None
        return self.index.get(str(item_id))

    def path_entry(self, item_id: Any) -> dict[str, Any] | None:
        if isinstance(item_id, dict):
            item_id = item_id.get("id")
        if item_id is None:
            return None
        return self.paths.get(str(item_id))

    def id_at_path(self, path: tuple[str, ...], wanted_kind: str | None = None) -> str | None:
        for item_id in self.path_ids.get(path, []):
            entry = self.paths.get(item_id, {})
            if wanted_kind is None or entry.get("kind") == wanted_kind:
                return item_id
        return None


def is_public(item: dict[str, Any], *, trait_member: bool = False) -> bool:
    visibility = item.get("visibility")
    if visibility == "public":
        return True
    return trait_member and visibility in {"default", None}


def normalize_kind(raw_kind: str | None) -> str | None:
    if raw_kind is None:
        return None
    key = raw_kind.lower().replace("-", "_").replace(" ", "_")
    return KIND_MAP.get(key)


def record_lint_kind(kind: str) -> str:
    if kind in LINT_NAMES:
        return kind
    return "inventory_only"


class SeedCollector:
    def __init__(
        self, docs: dict[str, Rustdoc], extra_seed_paths: tuple[str, ...] = ()
    ) -> None:
        self.docs = docs
        seed_paths = [seed for crate_seeds in SEEDS.values() for seed in crate_seeds]
        seed_paths.extend(extra_seed_paths)
        self.seed_paths = tuple(tuple(seed.split("::")) for seed in seed_paths)
        self.records: dict[tuple[str, str, str], dict[str, Any]] = {}
        self.visited_modules: set[tuple[str, str, tuple[str, ...]]] = set()
        self.visited_items: set[tuple[str, str, tuple[str, ...]]] = set()
        self.seed_item_ids: dict[str, set[tuple[str, str]]] = {
            kind: set() for kind in (*LINT_NAMES, "value")
        }
        self.value_type_ids: dict[str, set[tuple[str, str]]] = {}
        self.value_declared_types: dict[str, Any] = {}
        self.external_aliases: dict[tuple[str, str], dict[str, str]] = {}
        self.visited_alias_modules: set[tuple[str, str, tuple[str, ...]]] = set()

    def referenced_type_ids(self, value: Any) -> set[str]:
        found: set[str] = set()
        if isinstance(value, dict):
            resolved_path = value.get("resolved_path")
            if isinstance(resolved_path, dict) and resolved_path.get("id") is not None:
                found.add(str(resolved_path["id"]))
            # Rustdoc trait bounds use Path directly, without resolved_path.
            if isinstance(value.get("path"), str) and value.get("id") is not None:
                found.add(str(value["id"]))
            for nested in value.values():
                found.update(self.referenced_type_ids(nested))
        elif isinstance(value, list):
            for nested in value:
                found.update(self.referenced_type_ids(nested))
        return found

    def resolve_type_ids(self, doc: Rustdoc, value: Any) -> set[tuple[str, str]]:
        resolved_ids: set[tuple[str, str]] = set()
        for referenced_id in self.referenced_type_ids(value):
            target_doc, target_id, _, _ = self.resolve_target(doc, referenced_id)
            if target_doc is not None and target_id is not None:
                resolved_ids.add((target_doc.crate_name, target_id))
            else:
                resolved_ids.add((doc.crate_name, referenced_id))
        return resolved_ids

    def record_value_type_ids(
        self,
        path: str,
        doc: Rustdoc,
        item: dict[str, Any],
        raw_kind: str,
    ) -> None:
        declared = item_declared_type(item, raw_kind)
        if declared is None:
            return
        self.value_declared_types[path] = stable_value(declared)
        resolved_ids = self.resolve_type_ids(doc, declared)
        if resolved_ids:
            self.value_type_ids[path] = resolved_ids

    def add(
        self,
        path: tuple[str, ...],
        raw_kind: str,
        item: dict[str, Any] | None,
        doc: Rustdoc | None = None,
    ) -> None:
        kind = normalize_kind(raw_kind)
        if kind is None:
            raise InventoryError(f"unclassified public item {path}: Rustdoc kind {raw_kind!r}")
        rendered = "::".join(path)
        record: dict[str, Any] = {
            "path": rendered,
            "kind": kind,
            "rustdoc_kind": raw_kind,
            "lint": record_lint_kind(kind),
        }
        if item is not None:
            declared = item_declared_type(item, raw_kind)
            if declared is not None:
                record["declared_type"] = stable_value(declared)
                if kind == "value" and doc is not None:
                    self.record_value_type_ids(rendered, doc, item, raw_kind)
        self.records[(rendered, kind, raw_kind)] = record

    def track_seed_item(self, doc: Rustdoc, item_id: str, raw_kind: str) -> None:
        kind = normalize_kind(raw_kind)
        if kind in self.seed_item_ids:
            self.seed_item_ids[kind].add((doc.crate_name, item_id))

    def add_field(
        self,
        doc: Rustdoc,
        owner: tuple[str, ...],
        field_id: Any,
        *,
        fallback_name: str | None = None,
        inherited_visibility: bool = False,
    ) -> None:
        field = doc.item(field_id)
        if field is None or (not inherited_visibility and not is_public(field)):
            return
        name = field.get("name") or fallback_name
        if name is not None:
            self.add(owner + (str(name),), "struct_field", field)

    def item_kind(self, doc: Rustdoc, item_id: Any) -> str | None:
        item = doc.item(item_id)
        path_entry = doc.path_entry(item_id)
        fallback = path_entry.get("kind") if path_entry else None
        return rustdoc_kind(item, fallback) if item else fallback

    def resolve_target(self, source: Rustdoc, item_id: Any) -> tuple[Rustdoc | None, str | None, tuple[str, ...] | None, str | None]:
        if isinstance(item_id, dict):
            item_id = item_id.get("id")
        if item_id is None:
            return None, None, None, None
        item_id = str(item_id)
        direct = source.item(item_id)
        path_entry = source.path_entry(item_id)
        target_path = tuple(str(part) for part in path_entry.get("path", [])) if path_entry else None
        target_kind = self.item_kind(source, item_id)
        if direct is not None:
            return source, item_id, target_path, target_kind
        if target_path:
            target_doc = self.docs.get(target_path[0])
            if target_doc:
                target_id = target_doc.id_at_path(target_path)
                if target_id is not None:
                    target_kind = self.item_kind(target_doc, target_id) or target_kind
                    return target_doc, target_id, target_path, target_kind
        return None, None, target_path, target_kind

    def collect_seed(self, seed: str) -> None:
        path = tuple(seed.split("::"))
        crate_name = path[0]
        doc = self.docs.get(crate_name)
        if doc is None:
            raise InventoryError(f"no Rustdoc JSON for seed crate {crate_name}")
        seed_id = doc.id_at_path(path, "module")
        if seed_id is None:
            seed_id = self.find_local_item(doc, path)
        if seed_id is None:
            raise InventoryError(f"Rustdoc JSON does not expose seed module {seed}")
        seed_item = doc.item(seed_id)
        if seed_item is None or rustdoc_kind(seed_item, "module") != "module":
            raise InventoryError(f"seed path {seed} does not resolve to a module")
        self.walk_module(doc, seed_id, path)

        # Rustdoc's path index includes public associated items. It also catches
        # declarations whose parent item is emitted through a re-export.
        for item_id, entry in doc.paths.items():
            item_path = entry.get("path")
            if not isinstance(item_path, list):
                continue
            item_path = tuple(str(part) for part in item_path)
            if item_path[: len(path)] != path:
                continue
            raw_kind = entry.get("kind") or self.item_kind(doc, item_id)
            target_doc, target_id, _, target_kind = self.resolve_target(doc, item_id)
            if target_doc is not None and target_id is not None and target_kind is not None:
                self.walk_item(target_doc, target_id, target_kind, item_path)
            elif raw_kind and normalize_kind(raw_kind):
                self.add(item_path, raw_kind, doc.item(item_id), doc)
    def find_local_item(self, doc: Rustdoc, path: tuple[str, ...]) -> str | None:
        current_doc = doc
        current_id = doc.root_id
        for segment in path[1:]:
            module = current_doc.item(current_id)
            if module is None or rustdoc_kind(module) != "module":
                return None
            children = item_payload(module).get("items", [])
            match = None
            for child_id in children:
                child = current_doc.item(child_id)
                if child and child.get("name") == segment and is_public(child):
                    match = str(child_id)
                    break
            if match is None:
                return None
            current_id = match
        return current_id

    def walk_module(self, doc: Rustdoc, item_id: str, path: tuple[str, ...]) -> None:
        visit_key = (doc.crate_name, item_id, path)
        if visit_key in self.visited_modules:
            return
        self.visited_modules.add(visit_key)
        item = doc.item(item_id)
        if item is None:
            raise InventoryError(f"Rustdoc module {path} has no index item")
        self.add(path, "module", item, doc)
        module = item_payload(item)
        for child_id in module.get("items", []):
            child = doc.item(child_id)
            if child is None or not is_public(child):
                continue
            child_kind = rustdoc_kind(child, (doc.path_entry(child_id) or {}).get("kind"))
            if child_kind in {"import", "use"}:
                self.walk_import(doc, child_id, child, path)
                continue
            name = child.get("name")
            if not name:
                raise InventoryError(f"public Rustdoc item in {'::'.join(path)} has no name")
            child_path = path + (str(name),)
            self.walk_item(doc, str(child_id), child_kind or "unknown", child_path)

    def walk_import(self, doc: Rustdoc, item_id: str, item: dict[str, Any], owner: tuple[str, ...]) -> None:
        payload = item_payload(item)
        target_id = payload.get("id")
        is_glob = bool(payload.get("is_glob"))
        alias_name = item.get("name") or payload.get("name")
        target_doc, resolved_id, target_path, target_kind = self.resolve_target(doc, target_id)
        if target_kind is None:
            source_path = payload.get("source", "<unknown>")
            raise InventoryError(
                f"cannot resolve public re-export {owner}::{alias_name or source_path}"
            )
        if is_glob:
            self.add(owner + ("*",), "import", item, doc)
            if target_kind != "module" or target_doc is None or resolved_id is None:
                source_path = payload.get("source", "<unknown>")
                raise InventoryError(f"cannot expand public glob {source_path} in {'::'.join(owner)}")
            self.walk_module(target_doc, resolved_id, owner)
            return
        if not alias_name:
            raise InventoryError(f"public re-export at {'::'.join(owner)} has no local name")
        alias_path = owner + (str(alias_name),)
        if target_doc is not None and resolved_id is not None:
            self.walk_item(target_doc, resolved_id, target_kind, alias_path)
        else:
            self.add(alias_path, target_kind, item, doc)
        if target_kind == "module":
            if target_doc is None or resolved_id is None:
                raise InventoryError(f"cannot follow public module re-export {'::'.join(alias_path)}")
            self.walk_module(target_doc, resolved_id, alias_path)

    def walk_item(self, doc: Rustdoc, item_id: str, raw_kind: str, path: tuple[str, ...]) -> None:
        visit_key = (doc.crate_name, item_id, path)
        if visit_key in self.visited_items:
            return
        self.visited_items.add(visit_key)
        item = doc.item(item_id)
        self.track_seed_item(doc, item_id, raw_kind)
        self.add(path, raw_kind, item, doc)
        kind = normalize_kind(raw_kind)
        if kind == "module":
            self.walk_module(doc, item_id, path)
        elif kind == "type":
            self.walk_associated(doc, item_id, path)
        elif raw_kind == "extern_crate":
            target_doc, target_id, target_path, target_kind = self.resolve_target(doc, item_payload(item or {}).get("id"))
            if target_kind == "module" and target_doc and target_id:
                self.walk_module(target_doc, target_id, path)
    def walk_associated(self, doc: Rustdoc, item_id: str, path: tuple[str, ...]) -> None:
        item = doc.item(item_id)
        if item is None:
            return
        raw_kind = rustdoc_kind(item)
        payload = item_payload(item)
        if raw_kind in {"struct", "union"}:
            struct_kind = payload.get("kind", {})
            field_ids: list[Any] = []
            tuple_fields = False
            if isinstance(struct_kind, dict):
                variant, details = next(iter(struct_kind.items()), (None, {}))
                if variant == "plain" and isinstance(details, dict):
                    field_ids = details.get("fields", [])
                elif variant == "tuple" and isinstance(details, list):
                    field_ids = details
                    tuple_fields = True
            elif isinstance(payload.get("fields"), list):
                field_ids = payload["fields"]
            for index, field_id in enumerate(field_ids):
                self.add_field(
                    doc,
                    path,
                    field_id,
                    fallback_name=str(index) if tuple_fields else None,
                )
        elif raw_kind == "enum":
            for variant_id in payload.get("variants", []):
                variant = doc.item(variant_id)
                if variant is None:
                    continue
                name = variant.get("name")
                if not name:
                    raise InventoryError(f"public enum variant in {'::'.join(path)} has no name")
                variant_path = path + (str(name),)
                self.add(variant_path, "variant", variant)
                variant_payload = item_payload(variant)
                variant_kind = variant_payload.get("kind", {})
                field_ids: list[Any] = []
                tuple_fields = False
                if isinstance(variant_kind, dict):
                    field_form, field_data = next(iter(variant_kind.items()), (None, {}))
                    if field_form == "struct" and isinstance(field_data, dict):
                        field_ids = field_data.get("fields", [])
                    elif field_form == "tuple" and isinstance(field_data, list):
                        field_ids = field_data
                        tuple_fields = True
                for index, field_id in enumerate(field_ids):
                    self.add_field(
                        doc,
                        variant_path,
                        field_id,
                        fallback_name=str(index) if tuple_fields else None,
                        inherited_visibility=True,
                    )
        for associated_id, associated, associated_kind in self.public_associated_items(
            doc, item, path
        ):
            self.walk_item(
                doc,
                associated_id,
                associated_kind,
                path + (str(associated["name"]),),
            )

    def public_associated_items(
        self, doc: Rustdoc, item: dict[str, Any], path: tuple[str, ...]
    ) -> Iterator[tuple[str, dict[str, Any], str]]:
        payload = item_payload(item)
        groups = []
        if rustdoc_kind(item) == "trait":
            groups.append((payload.get("items", []), True))
        for impl_id in payload.get("impls", []):
            impl = doc.item(impl_id)
            if impl is None:
                continue
            impl_payload = item_payload(impl)
            if impl_payload.get("trait") is None:
                groups.append((impl_payload.get("items", []), False))
        for associated_ids, trait_member in groups:
            for associated_id in associated_ids:
                associated = doc.item(associated_id)
                if associated is None or not is_public(
                    associated, trait_member=trait_member
                ):
                    continue
                if not associated.get("name"):
                    raise InventoryError(
                        f"public associated item in {'::'.join(path)} has no name"
                    )
                associated_kind = rustdoc_kind(
                    associated,
                    (doc.path_entry(associated_id) or {}).get("kind"),
                )
                yield str(associated_id), associated, associated_kind or "unknown"


    def record_external_alias(
        self,
        path: tuple[str, ...],
        raw_kind: str,
        target_doc: Rustdoc,
        target_id: str,
        target_path: tuple[str, ...] | None,
    ) -> None:
        kind = normalize_kind(raw_kind)
        if kind not in {*LINT_NAMES, "value"}:
            return
        if target_path is None:
            entry = target_doc.path_entry(target_id)
            raw_path = entry.get("path") if entry else None
            if isinstance(raw_path, list):
                target_path = tuple(str(part) for part in raw_path)
        if target_path is None:
            raise InventoryError(
                f"cannot identify Rustdoc target path for alias {'::'.join(path)}"
            )
        rendered = "::".join(path)
        target = "::".join(target_path)
        if rendered != target:
            self.external_aliases[(rendered, kind)] = {
                "path": rendered,
                "kind": kind,
                "target": target,
            }

    def add_external_alias_target(
        self,
        target_doc: Rustdoc | None,
        target_id: str | None,
        target_path: tuple[str, ...] | None,
        raw_kind: str | None,
        alias_path: tuple[str, ...],
    ) -> None:
        kind = normalize_kind(raw_kind)
        if kind not in self.seed_item_ids or target_doc is None or target_id is None:
            return
        if (target_doc.crate_name, target_id) not in self.seed_item_ids[kind]:
            return
        self.record_external_alias(alias_path, raw_kind or kind, target_doc, target_id, target_path)
        self.walk_item(target_doc, target_id, raw_kind or kind, alias_path)

    def path_is_seed(self, path: tuple[str, ...] | None) -> bool:
        if path is None:
            return False
        return any(path[: len(seed_path)] == seed_path for seed_path in self.seed_paths)

    def walk_public_alias_module(
        self,
        doc: Rustdoc,
        item_id: str,
        path: tuple[str, ...],
        ancestors: frozenset[tuple[str, str]],
    ) -> None:
        module_key = (doc.crate_name, item_id)
        visit_key = (doc.crate_name, item_id, path)
        if module_key in ancestors or visit_key in self.visited_alias_modules:
            return
        self.visited_alias_modules.add(visit_key)
        module = doc.item(item_id)
        if module is None or rustdoc_kind(module) != "module":
            return
        ancestors = ancestors | {module_key}
        for child_id in item_payload(module).get("items", []):
            child = doc.item(child_id)
            if child is None or not is_public(child):
                continue
            raw_kind = rustdoc_kind(child, (doc.path_entry(child_id) or {}).get("kind"))
            name = child.get("name")
            child_path = path + (str(name),) if name else path
            if raw_kind == "module":
                if not name:
                    raise InventoryError(f"public Rustdoc module in {'::'.join(path)} has no name")
                self.walk_public_alias_module(
                    doc,
                    str(child_id),
                    child_path,
                    ancestors,
                )
                continue
            if raw_kind in {"import", "use", "extern_crate"}:
                payload = item_payload(child)
                target_id = payload.get("id")
                target_doc, resolved_id, target_path, target_kind = self.resolve_target(
                    doc,
                    target_id,
                )
                if target_kind is None:
                    if self.path_is_seed(target_path):
                        raise InventoryError(
                            f"cannot resolve potential seed re-export {'::'.join(child_path)}"
                        )
                    continue
                is_glob = bool(payload.get("is_glob"))
                alias_name = name or payload.get("name")
                if target_kind == "module":
                    if target_doc is None or resolved_id is None:
                        if self.path_is_seed(target_path):
                            raise InventoryError(
                                f"cannot follow potential seed module re-export {'::'.join(child_path)}"
                            )
                        continue
                    alias_path = path if is_glob else path + (str(alias_name),) if alias_name else None
                    if alias_path is not None:
                        self.walk_public_alias_module(
                            target_doc,
                            resolved_id,
                            alias_path,
                            ancestors,
                        )
                    elif self.path_is_seed(target_path):
                        raise InventoryError(
                            f"public module re-export in {'::'.join(path)} has no local name"
                        )
                    continue
                if is_glob or not alias_name:
                    continue
                self.add_external_alias_target(
                    target_doc,
                    resolved_id,
                    target_path,
                    target_kind,
                    path + (str(alias_name),),
                )
                continue
            kind = normalize_kind(raw_kind)
            if (
                name
                and kind in self.seed_item_ids
                and (doc.crate_name, str(child_id)) in self.seed_item_ids[kind]
            ):
                alias_path = path + (str(name),)
                target_path_entry = doc.path_entry(child_id)
                target_path_raw = target_path_entry.get("path") if target_path_entry else None
                canonical_path = (
                    tuple(str(part) for part in target_path_raw)
                    if isinstance(target_path_raw, list)
                    else None
                )
                self.record_external_alias(alias_path, raw_kind or kind, doc, str(child_id), canonical_path)
                self.walk_item(doc, str(child_id), raw_kind or kind, alias_path)

    def collect_external_aliases(self) -> None:
        for doc in self.docs.values():
            root = doc.item(doc.root_id)
            if root is None:
                raise InventoryError(f"Rustdoc root is missing for {doc.crate_name}")
            root_name = root.get("name") or doc.crate_name
            self.walk_public_alias_module(
                doc,
                doc.root_id,
                (str(root_name),),
                frozenset(),
            )

    def collect_public_type_aliases(self) -> None:
        candidates: list[tuple[tuple[str, ...], Rustdoc, str, dict[str, Any]]] = []
        for doc in self.docs.values():
            for item_id, entry in doc.paths.items():
                raw_path = entry.get("path")
                if not isinstance(raw_path, list):
                    continue
                raw_kind = entry.get("kind") or self.item_kind(doc, item_id)
                if raw_kind != "type_alias":
                    continue
                target_doc, target_id, _, target_kind = self.resolve_target(doc, item_id)
                if target_doc is None or target_id is None or target_kind != "type_alias":
                    continue
                item = target_doc.item(target_id)
                if item is not None and is_public(item):
                    candidates.append(
                        (
                            tuple(str(part) for part in raw_path),
                            target_doc,
                            target_id,
                            item,
                        )
                    )

        recorded_paths: set[tuple[str, str]] = set()
        while True:
            discovered_type = False
            for path, doc, item_id, item in candidates:
                declared = item_declared_type(item, "type_alias")
                if declared is None:
                    continue
                targets = self.resolve_type_ids(doc, declared) & self.seed_item_ids["type"]
                if not targets:
                    continue
                item_key = (doc.crate_name, item_id)
                if item_key not in self.seed_item_ids["type"]:
                    self.seed_item_ids["type"].add(item_key)
                    discovered_type = True
                rendered = "::".join(path)
                record_key = (rendered, "type")
                if record_key in recorded_paths:
                    continue
                self.add(path, "type_alias", item, doc)
                if not self.path_is_seed(path):
                    target_paths: list[str] = []
                    for crate_name, target_id in sorted(targets):
                        target_doc = self.docs.get(crate_name)
                        target_entry = target_doc.path_entry(target_id) if target_doc else None
                        target_path = target_entry.get("path") if target_entry else None
                        if not isinstance(target_path, list):
                            raise InventoryError(
                                f"cannot identify denied target for public alias {rendered}"
                            )
                        target_paths.append("::".join(str(part) for part in target_path))
                    self.external_aliases[record_key] = {
                        "path": rendered,
                        "kind": "type",
                        "target": ", ".join(sorted(set(target_paths))),
                    }
                recorded_paths.add(record_key)
            if not discovered_type:
                break

    def collect_public_value_declarations(self) -> None:
        for doc in self.docs.values():
            for item_id, entry in doc.paths.items():
                raw_path = entry.get("path")
                if not isinstance(raw_path, list):
                    continue
                raw_kind = entry.get("kind") or self.item_kind(doc, item_id)
                if normalize_kind(raw_kind) not in {"type", "value"}:
                    continue
                target_doc, target_id, _, target_kind = self.resolve_target(doc, item_id)
                if target_doc is not None and target_id is not None:
                    item = target_doc.item(target_id)
                    value_doc = target_doc
                    value_kind = target_kind or raw_kind
                else:
                    item = doc.item(item_id)
                    value_doc = doc
                    value_kind = raw_kind
                if item is None or value_kind is None or not is_public(item):
                    continue
                path = tuple(str(part) for part in raw_path)
                if normalize_kind(value_kind) == "value":
                    self.record_value_type_ids(
                        "::".join(path), value_doc, item, value_kind
                    )
                else:
                    for _, associated, associated_kind in self.public_associated_items(
                        value_doc, item, path
                    ):
                        if normalize_kind(associated_kind) == "value":
                            self.record_value_type_ids(
                                "::".join(path + (str(associated["name"]),)),
                                value_doc,
                                associated,
                                associated_kind,
                            )

    def unlinted_value_items(self) -> list[dict[str, Any]]:
        denied_types = self.seed_item_ids["type"]
        return [
            {
                "path": path,
                "declared_type": self.value_declared_types[path],
            }
            for path, referenced_types in sorted(self.value_type_ids.items())
            if referenced_types & denied_types
        ]

def source_hashes() -> tuple[list[dict[str, str]], list[dict[str, Any]]]:
    file_hashes: dict[str, str] = {}
    for relative in SOURCE_FILES:
        path = ROOT / relative
        if not path.is_file():
            raise InventoryError(f"seed source file is missing: {relative}")
        file_hashes[relative] = hashlib.sha256(path.read_bytes()).hexdigest()

    subtree_rows: list[dict[str, Any]] = []
    for relative in SOURCE_SUBTREES:
        root = ROOT / relative
        files = sorted(path for path in root.rglob("*.rs") if path.is_file()) if root.is_dir() else []
        subtree_hash = hashlib.sha256()
        file_names: list[str] = []
        for path in files:
            source_path = path.relative_to(ROOT).as_posix()
            contents_hash = hashlib.sha256(path.read_bytes()).digest()
            file_hashes[source_path] = contents_hash.hex()
            subtree_hash.update(source_path.encode("utf-8"))
            subtree_hash.update(b"\0")
            subtree_hash.update(contents_hash)
            subtree_hash.update(b"\n")
            file_names.append(source_path)
        subtree_rows.append(
            {
                "path": relative,
                "sha256": subtree_hash.hexdigest(),
                "files": file_names,
            }
        )
    file_rows = [
        {"path": path, "sha256": digest}
        for path, digest in sorted(file_hashes.items())
    ]
    return file_rows, sorted(subtree_rows, key=lambda row: row["path"])


def build_inventory() -> dict[str, Any]:
    target_dir = cargo_target_directory()
    docs: dict[str, Rustdoc] = {}
    for package_name, features in RUSTDOC_FEATURES.items():
        crate_name, json_path = rustdoc_json(package_name, target_dir, features)
        docs[crate_name] = Rustdoc(crate_name, json_path)
    collector = SeedCollector(docs)
    for crate_seeds in SEEDS.values():
        for seed in crate_seeds:
            collector.collect_seed(seed)
    collector.collect_public_type_aliases()
    collector.collect_public_value_declarations()
    collector.collect_external_aliases()
    items = sorted(
        collector.records.values(),
        key=lambda item: (item["path"], item["kind"], item["rustdoc_kind"]),
    )
    clippy_paths = {
        name: sorted({item["path"] for item in items if item["lint"] == name[:-1]})
        for name in ("types", "methods", "macros")
    }
    value_items = [item for item in items if item["kind"] == "value"]
    method_paths = set(clippy_paths["methods"])
    for feature, required_paths in FEATURED_SEED_METHODS.items():
        missing = sorted(set(required_paths) - method_paths)
        if missing:
            raise InventoryError(
                f"Rustdoc with {feature} omitted required seed methods: {missing}"
            )
    unlinted_values = collector.unlinted_value_items()
    external_aliases = sorted(
        collector.external_aliases.values(),
        key=lambda alias: (alias["path"], alias["kind"], alias["target"]),
    )
    source_files, source_subtrees = source_hashes()
    return {
        "schema": 2,
        "rustdoc_features": {
            crate: ["default", *features]
            for crate, features in RUSTDOC_FEATURES.items()
        },
        "coverage_limits": [
            "Inactive cfg items are not resolved into Clippy paths; edits to seed source files and subtrees change the SHA-256 tripwires.",
            "Exported macros relocated to crate root are source-hashed but are not name-resolved from the seed module by this inventory.",
            "Aliases defined by unrelated third-party crates are not enumerable; public seed-crate reexports and type aliases are added as deny paths.",
            "Value-only reads of public const/static items are not caught by Clippy; external value-read controls B1-B3 document this limit, and seed-backed typed values are enumerated in unlinted_value_items.",
        ],
        "seeds": [seed for crate_seeds in SEEDS.values() for seed in crate_seeds],
        "clippy_paths": clippy_paths,
        "public_items": items,
        "external_aliases": external_aliases,
        "value_items": value_items,
        "unlinted_value_items": unlinted_values,
        "source_files": source_files,
        "source_subtrees": source_subtrees,
    }


def toml_string(value: str) -> str:
    return json.dumps(value, ensure_ascii=True)


def config_text(inventory: dict[str, Any]) -> str:
    lines = [
        "# Generated by tools/architecture/nav_clippy.py. Do not edit by hand.",
        "# Paths include public seed items, reexports, and public aliases of seed types.",
        "# Value-only const/static uses are inventoried but not denied by Clippy.",
    ]
    reason = "The planner must use nav-executor for action capabilities."
    featured_methods = {
        path for paths in FEATURED_SEED_METHODS.values() for path in paths
    }
    for option, inventory_key in (
        ("disallowed-types", "types"),
        ("disallowed-methods", "methods"),
        ("disallowed-macros", "macros"),
    ):
        lines.extend(["", f"{option} = ["])
        for path in inventory["clippy_paths"][inventory_key]:
            allow_invalid = (
                ", allow-invalid = true"
                if option == "disallowed-methods" and path in featured_methods
                else ""
            )
            lines.append(
                f"    {{ path = {toml_string(path)}, "
                f"reason = {toml_string(reason)}{allow_invalid} }},"
            )
        lines.append("]")
    return "\n".join(lines) + "\n"


def write_atomic(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + f".tmp-{os.getpid()}")
    temporary.write_text(content, encoding="utf-8")
    os.replace(temporary, path)


def inventory_diff(old: dict[str, Any], new: dict[str, Any]) -> list[str]:
    lines: list[str] = []
    for collection, label, key_field in (
        ("public_items", "item", "path"),
        ("external_aliases", "external-alias", "path"),
        ("unlinted_value_items", "unlinted-value", "path"),
        ("source_files", "source-file", "path"),
        ("source_subtrees", "source-subtree", "path"),
    ):
        old_rows = {
            f"{row[key_field]}|{row.get('kind', '')}": row
            for row in old.get(collection, [])
        }
        new_rows = {
            f"{row[key_field]}|{row.get('kind', '')}": row
            for row in new.get(collection, [])
        }
        for key in sorted(new_rows.keys() - old_rows.keys()):
            lines.append(f"+ {label}: {key}")
        for key in sorted(old_rows.keys() - new_rows.keys()):
            lines.append(f"- {label}: {key}")
        for key in sorted(old_rows.keys() & new_rows.keys()):
            if old_rows[key] != new_rows[key]:
                lines.append(f"~ {label}: {key}")
    return lines


def write_generated_files() -> None:
    inventory = build_inventory()
    write_atomic(INVENTORY_PATH, json.dumps(inventory, indent=2, sort_keys=True) + "\n")
    write_atomic(CONFIG_PATH, config_text(inventory))
    print(
        "Generated nav Clippy inventory: "
        f"{len(inventory['public_items'])} public paths, "
        f"{len(inventory['clippy_paths']['types'])} types, "
        f"{len(inventory['clippy_paths']['methods'])} methods, "
        f"{len(inventory['clippy_paths']['macros'])} macros, "
        f"{len(inventory['external_aliases'])} external aliases, "
        f"{len(inventory['unlinted_value_items'])} unlinted typed values, "
        f"{len(inventory['source_files'])} source files and "
        f"{len(inventory['source_subtrees'])} source subtrees."
    )


def check_generated_files() -> bool:
    inventory = build_inventory()
    if not INVENTORY_PATH.is_file() or not CONFIG_PATH.is_file():
        print("Missing generated nav Clippy files. Run --write first.", file=sys.stderr)
        return False
    expected_inventory = json.loads(INVENTORY_PATH.read_text(encoding="utf-8"))
    okay = True
    if inventory != expected_inventory:
        print("nav Clippy inventory differs from its Rustdoc/source-hash baseline:", file=sys.stderr)
        changes = inventory_diff(expected_inventory, inventory)
        for line in changes:
            print(line, file=sys.stderr)
        if not changes:
            print("~ inventory metadata or derived Clippy paths", file=sys.stderr)
        okay = False
    expected_config = config_text(inventory)
    actual_config = CONFIG_PATH.read_text(encoding="utf-8")
    if actual_config != expected_config:
        print("crates/nav/clippy.toml differs from the generated deny list.", file=sys.stderr)
        okay = False
    if okay:
        print(
            "nav Clippy inventory is current: "
            f"{len(inventory['public_items'])} public paths, "
            f"{len(inventory['value_items'])} values and "
            f"{len(inventory['source_files'])} source files explicitly inventoried."
        )
    return okay


def probe_expectations() -> tuple[dict[str, str], set[str], set[str], dict[str, Any]]:
    manifest = json.loads(PROBE_MANIFEST.read_text(encoding="utf-8"))
    if manifest.get("schema") != 2:
        raise InventoryError("unsupported probe manifest schema")
    types = manifest["types"]
    methods = manifest["methods"]
    expected = {case: "clippy::disallowed_types" for case in types}
    expected.update({case: "clippy::disallowed_methods" for case in methods})
    passes = set(manifest["passes"])
    known_limits = set(manifest.get("known_limits", []))
    if len(expected) != len(types) + len(methods):
        raise InventoryError("probe IDs appear in more than one failing group")
    groups = (set(expected), passes, known_limits)
    if any(groups[left] & groups[right] for left in range(3) for right in range(left + 1, 3)):
        raise InventoryError("probe IDs appear in more than one outcome group")
    type_paths = manifest.get("type_paths")
    if not isinstance(type_paths, dict) or any(
        not isinstance(case, str) or not isinstance(path, str) or not path
        for case, path in type_paths.items()
    ):
        raise InventoryError("probe type_paths must map case IDs to nonempty Rust paths")
    if not set(type_paths) <= set(expected) | passes:
        raise InventoryError("probe type paths include an unlisted outcome")
    if not isinstance(manifest.get("fixture_seed_paths"), list):
        raise InventoryError("probe fixture_seed_paths must be a list")
    return expected, passes, known_limits, manifest


def probe_markers(source: str) -> tuple[dict[str, int], list[int]]:
    markers: dict[str, int] = {}
    rows: list[int] = []
    for line_number, line in enumerate(source.splitlines(), 1):
        marker = line.strip()
        if not marker.startswith("// probe: "):
            continue
        case = marker.removeprefix("// probe: ").strip()
        if not case or case in markers:
            raise InventoryError(f"empty or duplicate probe marker {case!r}")
        markers[case] = line_number
        rows.append(line_number)
    return markers, rows


def case_for_line(line: int, markers: dict[str, int], marker_rows: list[int]) -> str | None:
    index = bisect.bisect_right(marker_rows, line) - 1
    if index < 0:
        return None
    marker_line = marker_rows[index]
    for case, row in markers.items():
        if row == marker_line:
            return case
    return None


def consumer_diagnostic_locations(spans: Any):
    if not isinstance(spans, list):
        return
    for span in spans:
        if not isinstance(span, dict) or span.get("is_primary") is not True:
            continue
        location = span
        while isinstance(location.get("expansion"), dict):
            call_site = location["expansion"].get("span")
            if not isinstance(call_site, dict):
                break
            location = call_site
        filename = location.get("file_name")
        line = location.get("line_start")
        if isinstance(filename, str) and isinstance(line, int):
            yield filename, line



def build_probe_libraries() -> tuple[dict[str, Path], Path]:
    target_dir = cargo_target_directory()
    result = run(
        [
            "cargo",
            "build",
            "--manifest-path",
            str(ROOT_MANIFEST),
            "--package",
            "api",
            "--lib",
            "--message-format=json",
        ],
        cwd=ROOT,
    )
    if result.returncode != 0:
        raise InventoryError(f"cargo build for external Clippy probes failed:\n{result.stderr}")
    artifacts: dict[str, Path] = {}
    for line in result.stdout.splitlines():
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        if message.get("reason") != "compiler-artifact":
            continue
        target = message.get("target", {})
        name = target.get("name")
        if name not in {"api", "client"} or "lib" not in target.get("kind", []):
            continue
        for filename in message.get("filenames", []):
            if filename.endswith(".rlib"):
                artifacts[name] = Path(filename)
    dependency_dir = target_dir / "debug" / "deps"
    for name in ("api", "client"):
        if name in artifacts:
            continue
        candidates = sorted(dependency_dir.glob(f"lib{name}-*.rlib"))
        if len(candidates) == 1:
            artifacts[name] = candidates[0]
        else:
            raise InventoryError(
                f"could not identify one {name} rlib for external Clippy probes "
                f"(found {len(candidates)})"
            )
    return artifacts, dependency_dir


def case_for_diagnostic_path(
    filename: str,
    line: int,
    *,
    source_path: Path,
    markers: dict[str, int],
    marker_rows: list[int],
) -> str | None:
    path = Path(filename)
    candidate = path.resolve() if path.is_absolute() else (source_path.parent / path).resolve()
    if candidate == source_path.resolve():
        return case_for_line(line, markers, marker_rows)
    return None


def probe_type_path(path: str) -> str:
    if path.startswith("nav_clippy_external_facade::"):
        return path
    return f"nav_clippy_external_facade::{path}"


def probe_source_with_type_cases(
    source: str, type_paths: dict[str, str]
) -> str:
    lines = source.rstrip().splitlines()
    for case, path in sorted(type_paths.items()):
        identifier = "".join(
            character.lower() if character.isalnum() else "_"
            for character in case
        )
        lines.extend(
            [
                "",
                f"// probe: {case}",
                f"pub use {probe_type_path(path).split('<', 1)[0]} as NavProbe{identifier.replace('_', '').title()};",
            ]
        )
    return "\n".join(lines) + "\n"


def rustdoc_probe_facade(
    output_dir: Path, libraries: dict[str, Path], dependency_dir: Path
) -> Rustdoc:
    rustdoc = shutil.which("rustdoc")
    if rustdoc is None:
        raise InventoryError("rustdoc must be available on PATH for external probes")
    output_dir.mkdir()
    env = os.environ.copy()
    env["RUSTC_BOOTSTRAP"] = "1"
    result = run(
        [
            rustdoc,
            str(EXTERNAL_FACADE_SOURCE),
            "--crate-name",
            "nav_clippy_external_facade",
            "--edition",
            "2021",
            "--output-format",
            "json",
            "-Z",
            "unstable-options",
            "--document-private-items",
            "--out-dir",
            str(output_dir),
            "-L",
            f"dependency={dependency_dir}",
            "--extern",
            f"api={libraries['api']}",
            "--extern",
            f"client={libraries['client']}",
        ],
        cwd=EXTERNAL_FACADE_SOURCE.parent,
        env=env,
    )
    if result.returncode != 0:
        raise InventoryError(
            "rustdoc could not generate the external facade fixture inventory:\n"
            f"{result.stderr}"
        )
    json_path = output_dir / "nav_clippy_external_facade.json"
    if not json_path.is_file():
        raise InventoryError(f"rustdoc did not create {json_path}")
    return Rustdoc("nav_clippy_external_facade", json_path)


def collect_probe_inventory(
    docs: dict[str, Rustdoc], fixture_seed_paths: list[str]
) -> tuple[dict[str, Any], SeedCollector]:
    collector = SeedCollector(docs, tuple(fixture_seed_paths))
    seeds = [seed for crate_seeds in SEEDS.values() for seed in crate_seeds]
    seeds.extend(fixture_seed_paths)
    for seed in seeds:
        collector.collect_seed(seed)
    collector.collect_public_type_aliases()
    collector.collect_public_value_declarations()
    collector.collect_external_aliases()
    items = sorted(
        collector.records.values(),
        key=lambda item: (item["path"], item["kind"], item["rustdoc_kind"]),
    )
    clippy_paths = {
        name: sorted({item["path"] for item in items if item["lint"] == name[:-1]})
        for name in ("types", "methods", "macros")
    }
    inventory = {
        "clippy_paths": clippy_paths,
        "public_items": items,
        "value_items": [item for item in items if item["kind"] == "value"],
        "unlinted_value_items": collector.unlinted_value_items(),
    }
    return inventory, collector


def run_probes() -> bool:
    expected, passes, known_limits, manifest = probe_expectations()
    type_paths = manifest["type_paths"]
    source = PROBE_SOURCE.read_text(encoding="utf-8")
    consumer_source = probe_source_with_type_cases(source, type_paths)
    markers, marker_rows = probe_markers(consumer_source)
    outcome_ids = set(expected) | passes | known_limits
    if set(markers) != outcome_ids:
        missing = sorted(outcome_ids - set(markers))
        unlisted = sorted(set(markers) - outcome_ids)
        print(
            f"probe marker mismatch. Missing: {missing}. Unlisted: {unlisted}.",
            file=sys.stderr,
        )
        return False

    target_dir = cargo_target_directory()
    probe_root = target_dir / "nav-clippy-probes"
    probe_root.mkdir(parents=True, exist_ok=True)
    target_name = f"nav_clippy_probes_{os.getpid()}"
    fixture_dir: Path | None = None
    try:
        fixture_dir = Path(
            tempfile.mkdtemp(prefix=f"{target_name}_", dir=probe_root)
        )
        source_path = fixture_dir / "consumer.rs"
        source_path.write_text(consumer_source, encoding="utf-8")

        docs: dict[str, Rustdoc] = {}
        for package_name, features in RUSTDOC_FEATURES.items():
            crate_name, json_path = rustdoc_json(package_name, target_dir, features)
            docs[crate_name] = Rustdoc(crate_name, json_path)
        # Cargo rustdoc can rebuild proc-macro dependencies in place. Capture
        # linkable artifacts only after those invocations finish.
        libraries, dependency_dir = build_probe_libraries()
        facade_doc = rustdoc_probe_facade(
            fixture_dir / "rustdoc", libraries, dependency_dir
        )
        docs[facade_doc.crate_name] = facade_doc
        inventory, _collector = collect_probe_inventory(
            docs, manifest["fixture_seed_paths"]
        )

        denied_types = set(inventory["clippy_paths"]["types"])
        required_paths = set(manifest["required_generated_alias_paths"])
        for case, path in type_paths.items():
            if expected.get(case) != "clippy::disallowed_types":
                continue
            required_paths.add(probe_type_path(path).split("<", 1)[0])
        missing_paths = sorted(required_paths - denied_types)
        if missing_paths:
            raise InventoryError(
                "external Rustdoc inventory omitted required denied type paths: "
                f"{missing_paths}"
            )

        value_paths = {item["path"] for item in inventory["value_items"]}
        missing_values = sorted(
            set(manifest["required_seed_value_items"]) - value_paths
        )
        if missing_values:
            raise InventoryError(
                f"external Rustdoc inventory omitted seeded values: {missing_values}"
            )
        unlinted_paths = {
            item["path"] for item in inventory["unlinted_value_items"]
        }
        missing_typed_values = sorted(
            set(manifest["required_unlinted_typed_values"]) - unlinted_paths
        )
        if missing_typed_values:
            raise InventoryError(
                "external Rustdoc inventory omitted typed value declarations: "
                f"{missing_typed_values}"
            )

        (fixture_dir / "clippy.toml").write_text(
            config_text(inventory), encoding="utf-8"
        )
        rustc = shutil.which("rustc")
        clippy_driver = shutil.which("clippy-driver")
        if rustc is None or clippy_driver is None:
            raise InventoryError(
                "rustc and clippy-driver must both be available on PATH"
            )

        facade_dir = fixture_dir / "facade"
        facade_dir.mkdir()
        facade_result = run(
            [
                rustc,
                "--crate-name",
                "nav_clippy_external_facade",
                "--crate-type",
                "rlib",
                "--edition",
                "2021",
                str(EXTERNAL_FACADE_SOURCE),
                "--out-dir",
                str(facade_dir),
                "-L",
                f"dependency={dependency_dir}",
                "--extern",
                f"api={libraries['api']}",
                "--extern",
                f"client={libraries['client']}",
            ],
            cwd=fixture_dir,
        )
        if facade_result.returncode != 0:
            raise InventoryError(
                "rustc could not compile the unlinted external facade fixture:\n"
                f"{facade_result.stderr}"
            )
        facade_rlib = facade_dir / "libnav_clippy_external_facade.rlib"
        output_dir = fixture_dir / "consumer"
        output_dir.mkdir()
        result = run(
            [
                clippy_driver,
                "--crate-name",
                target_name,
                "--crate-type",
                "lib",
                "--edition",
                "2021",
                str(source_path),
                "--emit=metadata",
                "--out-dir",
                str(output_dir),
                "--error-format=json",
                "-D",
                "warnings",
                "-A",
                "dead_code",
                "--cfg",
                "test",
                "-L",
                f"dependency={dependency_dir}",
                "--extern",
                f"api={libraries['api']}",
                "--extern",
                f"client={libraries['client']}",
                "--extern",
                f"nav_clippy_external_facade={facade_rlib}",
            ],
            cwd=fixture_dir,
        )
        actual: dict[str, set[str]] = {}
        unexpected: list[str] = []
        for line in (*result.stdout.splitlines(), *result.stderr.splitlines()):
            try:
                message = json.loads(line)
            except json.JSONDecodeError:
                continue
            if message.get("reason") == "compiler-message":
                diagnostic = message.get("message", {})
            elif message.get("$message_type") == "diagnostic":
                diagnostic = message
            else:
                continue
            code = (diagnostic.get("code") or {}).get("code")
            if code not in PROBE_LINTS:
                if (
                    code is None
                    and not diagnostic.get("spans")
                    and diagnostic.get("message", "").startswith("aborting due to ")
                ):
                    # This summarizes the expected denied diagnostics below.
                    continue
                if diagnostic.get("level") == "error":
                    unexpected.append(
                        diagnostic.get(
                            "message", "compiler error without a code"
                        )
                    )
                continue
            case_ids: set[str] = set()
            for filename, line_number in consumer_diagnostic_locations(
                diagnostic.get("spans", [])
            ):
                case = case_for_diagnostic_path(
                    filename,
                    line_number,
                    source_path=source_path,
                    markers=markers,
                    marker_rows=marker_rows,
                )
                if case is not None:
                    case_ids.add(case)
            if not case_ids:
                unexpected.append(
                    f"unmapped {code} diagnostic: {diagnostic.get('message', '')}"
                )
                continue
            for case in case_ids:
                actual.setdefault(case, set()).add(code)

        okay = True
        for case, lint in sorted(expected.items()):
            if lint not in actual.get(case, set()):
                print(
                    f"FAIL {case}: expected {lint}; "
                    f"saw {sorted(actual.get(case, set()))}"
                )
                okay = False
            else:
                print(f"PASS {case}: {lint} at the external consumer")
        for case in sorted(passes):
            if case in actual:
                print(f"FAIL {case}: control raised {sorted(actual[case])}")
                okay = False
            else:
                print(f"PASS {case}: no action-deny lint")
        for case in sorted(known_limits):
            if case in actual:
                print(
                    f"FAIL {case}: documented limitation unexpectedly raised "
                    f"{sorted(actual[case])}"
                )
                okay = False
            else:
                print(f"LIMIT {case}: no action-deny lint")
        for case in sorted(set(actual) - outcome_ids):
            print(f"FAIL {case}: unexpected action-deny lint {sorted(actual[case])}")
            okay = False
        if unexpected:
            print("Unexpected compiler diagnostics:", file=sys.stderr)
            for diagnostic in unexpected:
                print(f"- {diagnostic}", file=sys.stderr)
            okay = False
        if result.returncode == 0 and expected:
            print(
                "External Clippy probe succeeded without the required denials.",
                file=sys.stderr,
            )
            okay = False
        if result.returncode != 0 and not actual:
            print(
                "External Clippy probe failed without expected lint diagnostics:\n"
                f"{result.stderr}",
                file=sys.stderr,
            )
            okay = False
        if okay:
            print(
                f"nav Clippy probes passed: {len(expected)} denied forms, "
                f"{len(passes)} controls and {len(known_limits)} documented limits."
            )
        return okay
    finally:
        if fixture_dir is not None:
            shutil.rmtree(fixture_dir)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--write", action="store_true", help="Generate clippy.toml and its inventory baseline")
    mode.add_argument("--check", action="store_true", help="Check Rustdoc coverage, generated config, and compiler probes")
    mode.add_argument("--probes-only", action="store_true", help="Run only the compiler-resolved Clippy probes")
    args = parser.parse_args()
    try:
        if args.write:
            write_generated_files()
            return 0
        if args.probes_only:
            return 0 if run_probes() else 1
        inventory_ok = check_generated_files()
        probes_ok = run_probes()
        return 0 if inventory_ok and probes_ok else 1
    except (InventoryError, OSError, json.JSONDecodeError) as error:
        print(f"nav Clippy check failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
