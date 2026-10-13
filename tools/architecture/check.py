#!/usr/bin/env python3
"""Enforce the explicit 274bot workspace crate-graph policy.

Requires Python 3.11+ (stdlib tomllib). Reads Cargo.toml manifests; does not
compile Rust or parse .rs with regex.

    python3 tools/architecture/check.py
    python3 tools/architecture/check.py --self-test
"""

from __future__ import annotations

import argparse
import sys
import tempfile
from collections import defaultdict
from dataclasses import dataclass, field
from pathlib import Path
import importlib.util
import json
import os

if sys.version_info < (3, 11):
    sys.stderr.write(
        f"tools/architecture/check.py needs Python 3.11+ for tomllib "
        f"(found {sys.version.split()[0]}). Use python3.11 or newer.\n"
    )
    raise SystemExit(2)

import tomllib

# Required production kinds. `optional` is not one of these: a required
# `[target.*.dependencies]` edge must not satisfy an optional-only allow.
REQUIRED_KINDS = frozenset({"normal", "target"})
DEP_TABLES = (
    ("dependencies", "normal"),
    ("dev-dependencies", "dev"),
    ("build-dependencies", "build"),
)


@dataclass(frozen=True)
class Edge:
    src: str
    dst: str
    kind: str
    via: str


@dataclass
class Allow:
    kinds: set[str] = field(default_factory=set)
    reasons: list[str] = field(default_factory=list)


@dataclass
class Policy:
    members: set[str]
    externals: set[str]
    allowed: dict[tuple[str, str], Allow]
    path: Path
    migrations: dict[str, dict] = field(default_factory=dict)
    configurations: list[dict] = field(default_factory=list)

    @property
    def known(self) -> set[str]:
        return self.members | self.externals


def load_toml(path: Path) -> dict:
    with path.open("rb") as fh:
        return tomllib.load(fh)


def load_policy(path: Path) -> Policy:
    raw = load_toml(path)
    require_fields(raw, {"schema_version", "workspace", "allow", "migration", "configuration"}, "policy")
    if type(raw.get("schema_version")) is not int or raw["schema_version"] != 1:
        raise ValueError("policy schema_version must be 1")
    ws = raw.get("workspace") or {}
    require_fields(ws, {"members", "externals"}, "workspace")
    members = string_set(ws.get("members"), "workspace.members")
    externals = string_set(ws.get("externals", []), "workspace.externals")
    if not members or members & externals:
        raise ValueError("policy members must be nonempty and disjoint from externals")
    allowed: dict[tuple[str, str], Allow] = {}
    for row in raw.get("allow", []):
        require_fields(row, {"from", "to", "kinds", "reason"}, "allow")
        src, dst = row.get("from"), row.get("to")
        kinds = string_set(row.get("kinds"), "allow.kinds")
        reason = nonblank(row.get("reason"), "allow.reason")
        if src not in members or dst not in members | externals or not kinds or not kinds <= KINDS:
            raise ValueError(f"invalid allow endpoints/kinds: {row}")
        if (src, dst) in allowed:
            raise ValueError(f"duplicate allow pair: {src} -> {dst}")
        allowed[src, dst] = Allow(kinds, [reason])
    migrations = {}
    triples = set()
    for row in raw.get("migration", []):
        require_fields(row, {"id", "from", "to", "kind", "owner", "consumers", "reason", "removal_endpoint"}, "migration", exact=True)
        identity = nonblank(row["id"], "migration.id")
        triple = row["from"], row["to"], row["kind"]
        if identity in migrations or triple in triples:
            raise ValueError(f"duplicate migration identity/edge: {identity}")
        if row["from"] not in members or row["to"] not in members | externals or row["kind"] not in KINDS:
            raise ValueError(f"invalid migration endpoints/kind: {identity}")
        if row["owner"] not in OWNERS or row["removal_endpoint"] not in ENDPOINTS:
            raise ValueError(f"unknown migration owner/endpoint: {identity}")
        consumers = string_set(row["consumers"], "migration.consumers")
        if not consumers or any("*" in item for item in consumers):
            raise ValueError(f"migration needs exact consumers: {identity}")
        nonblank(row["reason"], "migration.reason")
        if row["kind"] in allowed.get(triple[:2], Allow()).kinds:
            raise ValueError(f"migration duplicated as permanent allowance: {identity}")
        migrations[identity] = row
        triples.add(triple)
    configurations = raw.get("configuration", [])
    ids = set()
    for row in configurations:
        require_fields(row, {"id", "packages", "features", "default_features", "dev", "status", "owner", "forbidden_features", "optional_dependencies"}, "configuration", exact=True)
        identity = nonblank(row["id"], "configuration.id")
        if identity in ids:
            raise ValueError(f"duplicate configuration: {identity}")
        ids.add(identity)
        packages = string_set(row["packages"], "configuration.packages")
        if not packages or not packages <= members:
            raise ValueError(f"unknown configuration packages: {identity}")
        for key in ("features", "forbidden_features", "optional_dependencies"):
            string_set(row[key], f"configuration.{key}")
            if key != "features" and any(
                len(item.split("/")) != 2 or item.split("/")[0] not in members
                or not item.split("/")[1] for item in row[key]
            ):
                raise ValueError(f"configuration {key} must name known package/item: {identity}")
        if type(row["default_features"]) is not bool or type(row["dev"]) is not bool:
            raise ValueError(f"configuration flags must be boolean: {identity}")
        if row["status"] not in {"enforced", "pending"} or row["owner"] not in OWNERS:
            raise ValueError(f"configuration status/owner invalid: {identity}")
        if row["status"] == "pending" and identity != "native-only":
            raise ValueError("only the named native-only target may be pending")
    return Policy(members, externals, allowed, path, migrations, configurations)


KINDS = frozenset({"normal", "target", "optional", "dev", "build"})
OWNERS = frozenset({"T1", "T2", "T3", "T4", "T5"})
ENDPOINTS = frozenset({"T3.1", "T3.3", "T5.1"})


def require_fields(value: dict, fields: set[str], label: str, *, exact: bool = False) -> None:
    if not isinstance(value, dict) or set(value) - fields or (exact and set(value) != fields):
        raise ValueError(f"{label}: unknown or missing fields")


def nonblank(value, label: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"{label}: expected nonblank string")
    return value


def string_set(value, label: str) -> set[str]:
    if not isinstance(value, list) or any(not isinstance(v, str) or not v.strip() for v in value):
        raise ValueError(f"{label}: expected string array")
    if len(value) != len(set(value)):
        raise ValueError(f"{label}: duplicate entries")
    return set(value)


def spec_map(spec) -> dict:
    if spec is None:
        return {}
    if isinstance(spec, str):
        return {"version": spec}
    if isinstance(spec, dict):
        return spec
    raise ValueError("unsupported Cargo dependency specification")


def package_name(manifest: dict, fallback: str) -> str:
    pkg = manifest.get("package") or {}
    name = pkg.get("name")
    return str(name) if name else fallback


def workspace_dep_lookup(ws_deps: dict, key: str) -> dict:
    return spec_map(ws_deps.get(key))


def resolve_path_package(path: Path, by_path: dict[Path, str]) -> str | None:
    try:
        resolved = path.resolve()
    except OSError:
        return None
    if resolved in by_path:
        return by_path[resolved]
    manifest = resolved / "Cargo.toml"
    if manifest.is_file():
        return package_name(load_toml(manifest), resolved.name)
    return None


def iter_dep_tables(manifest: dict):
    for key, kind in DEP_TABLES:
        table = manifest.get(key)
        if isinstance(table, dict):
            yield kind, table
    target = manifest.get("target")
    if not isinstance(target, dict):
        return
    for cfg_tables in target.values():
        if not isinstance(cfg_tables, dict):
            continue
        for key, kind in DEP_TABLES:
            table = cfg_tables.get(key)
            if not isinstance(table, dict):
                continue
            # Required target production deps are "target". optional=true on
            # that table stays "optional" in classify_kind. target-dev/build
            # keep their test/build kinds.
            yield ("target" if kind == "normal" else kind), table


def classify_kind(table_kind: str, spec: dict) -> str:
    if table_kind in REQUIRED_KINDS and spec.get("optional") is True:
        return "optional"
    return table_kind


def resolve_dep_name(
    key: str,
    spec: dict,
    ws_deps: dict,
    crate_dir: Path,
    workspace_root: Path,
    by_path: dict[Path, str],
) -> tuple[str | None, bool]:
    """Return (package_name, is_path_or_workspace_path)."""
    merged = dict(spec)
    inherited_path = False
    if merged.get("workspace") is True:
        lookup = workspace_dep_lookup(ws_deps, key)
        if not lookup:
            raise ValueError(f"missing workspace dependency: {key}")
        # Cargo forbids optional on [workspace.dependencies]. Do not inherit
        # it. Member tables use { workspace = true, optional = true }.
        for field in ("path", "package"):
            if field not in merged and field in lookup:
                merged[field] = lookup[field]
        inherited_path = "path" in lookup and "path" not in spec
    name = str(merged["package"]) if merged.get("package") else key
    path_value = merged.get("path")
    if path_value:
        base = workspace_root if inherited_path else crate_dir
        dependency_path = (base / str(path_value)).resolve()
        if not dependency_path.is_relative_to(workspace_root.resolve()):
            raise ValueError(f"dependency path escapes data root: {key}")
        resolved = resolve_path_package(dependency_path, by_path)
        if resolved:
            name = resolved
        return name, True
    if spec.get("workspace") is True:
        return name, bool(workspace_dep_lookup(ws_deps, key).get("path"))
    return name, False


def discover_workspace(root: Path) -> tuple[dict, dict[str, Path], dict[Path, str], dict]:
    manifest_path = root / "Cargo.toml"
    if not manifest_path.is_file():
        raise SystemExit(f"missing workspace manifest: {manifest_path}")
    workspace = load_toml(manifest_path)
    ws_table = workspace.get("workspace") or {}
    members = ws_table.get("members") or []
    by_name: dict[str, Path] = {}
    by_path: dict[Path, str] = {}
    # Cargo also treats a root [package] as a workspace member. Scan it so
    # an implicit root crate cannot escape the policy. Globs are unsupported
    # and fail closed below (member path has no Cargo.toml).
    root_pkg = workspace.get("package")
    if isinstance(root_pkg, dict) and root_pkg.get("name"):
        root_name = str(root_pkg["name"])
        by_name[root_name] = root
        by_path[root.resolve()] = root_name
    for rel in members:
        crate_dir = (root / str(rel)).resolve()
        if not crate_dir.is_relative_to(root.resolve()):
            raise ValueError(f"workspace member escapes data root: {rel}")
        crate_manifest = crate_dir / "Cargo.toml"
        if not crate_manifest.is_file():
            raise SystemExit(f"workspace member {rel!r} has no Cargo.toml")
        name = package_name(load_toml(crate_manifest), crate_dir.name)
        if name in by_name or crate_dir in by_path:
            raise ValueError(f"duplicate workspace member: {name}")
        by_name[name] = crate_dir
        by_path[crate_dir] = name
    ws_deps = (workspace.get("workspace") or {}).get("dependencies") or {}
    if not isinstance(ws_deps, dict):
        raise ValueError("unsupported workspace dependencies")
    return workspace, by_name, by_path, ws_deps


def collect_edges(
    by_name: dict[str, Path],
    by_path: dict[Path, str],
    ws_deps: dict,
    workspace_root: Path,
) -> list[Edge]:
    edges: list[Edge] = []
    for src, crate_dir in sorted(by_name.items()):
        manifest = load_toml(crate_dir / "Cargo.toml")
        for table_kind, table in iter_dep_tables(manifest):
            for key, raw in table.items():
                spec = spec_map(raw)
                dst, is_path = resolve_dep_name(
                    key, spec, ws_deps, crate_dir, workspace_root, by_path
                )
                if not dst:
                    continue
                if not is_path and dst not in by_name:
                    continue
                kind = classify_kind(table_kind, spec)
                via = key if key != dst else kind
                if dst == src:
                    continue
                edges.append(Edge(src=src, dst=dst, kind=kind, via=str(via)))
    return edges


def kind_permitted(actual: str, allowed: set[str]) -> bool:
    return actual in allowed


def check_graph(root: Path, policy: Policy) -> list[str]:
    _, by_name, by_path, ws_deps = discover_workspace(root)
    errors: list[str] = []

    actual_members = set(by_name)
    extra = sorted(actual_members - policy.members)
    missing = sorted(policy.members - actual_members)
    if extra:
        errors.append(
            "unknown workspace member(s) not listed in "
            f"{policy.path}: {', '.join(extra)}. "
            "Add the crate to [workspace].members with ownership reasons, "
            "or remove it from the Cargo workspace."
        )
    if missing:
        errors.append(
            "policy lists workspace member(s) absent from Cargo.toml: "
            f"{', '.join(missing)}. Update {policy.path} if the crate was removed."
        )

    for key, raw in ws_deps.items():
        if spec_map(raw).get("optional") is True:
            errors.append(
                f"invalid workspace.dependencies.{key}: optional = true. "
                "Cargo forbids optional on workspace dependencies "
                "('workspace dependencies cannot be optional'). "
                "Set optional on the member: { workspace = true, optional = true }."
            )

    edges = collect_edges(by_name, by_path, ws_deps, root)
    migration_triples = {(r["from"], r["to"], r["kind"]) for r in policy.migrations.values()}
    actual_triples = {(e.src, e.dst, e.kind) for e in edges}
    for triple in sorted(migration_triples - actual_triples):
        errors.append(f"stale migration allowance: {triple}")

    seen: set[tuple[str, str, str]] = set()
    for edge in edges:
        key = (edge.src, edge.dst, edge.kind)
        if key in seen:
            continue
        seen.add(key)
        if key in migration_triples:
            continue
        if edge.dst not in policy.known:
            errors.append(
                f"disallowed crate edge: {edge.src} -> {edge.dst} ({edge.kind})"
                f"{f' via {edge.via}' if edge.via != edge.kind else ''}. "
                f"{edge.dst!r} is not a policy member or external. "
                f"Edit {policy.path} and docs/architecture.md if this is deliberate."
            )
            continue
        allow = policy.allowed.get((edge.src, edge.dst))
        if allow is None:
            errors.append(
                f"disallowed crate edge: {edge.src} -> {edge.dst} ({edge.kind}). "
                f"No [[allow]] permits this ownership direction. "
                f"If this is a deliberate boundary change, add an allow with a "
                f"reason in {policy.path} and update docs/architecture.md."
            )
            continue
        if not kind_permitted(edge.kind, allow.kinds):
            reasons = " ".join(allow.reasons)
            errors.append(
                f"disallowed crate edge kind: {edge.src} -> {edge.dst} is {edge.kind}, "
                f"but policy allows only {sorted(allow.kinds)}. {reasons} "
                f"Edit {policy.path} and docs/architecture.md if the kind should change."
            )
    return errors


def trusted_helper(name: str):
    """Load a helper adjacent to this executable, never from candidate H."""
    path = Path(__file__).resolve().with_name(f"{name}.py")
    spec = importlib.util.spec_from_file_location(f"architecture_{name}", path)
    if spec is None or spec.loader is None:
        raise ValueError(f"missing trusted helper: {name}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def check_graph_tree(root: Path) -> list[str]:
    return trusted_helper("graph").check_tree(sys.modules[__name__], root)


def check_graph_transition(base: Path, candidate: Path) -> list[str]:
    return trusted_helper("graph").check_transition(sys.modules[__name__], base, candidate)


def check_tree(root: Path) -> list[str]:
    errors = check_graph_tree(root)
    for name in ("ownership", "inventories"):
        errors.extend(trusted_helper(name).check_tree(root))
    return errors


def check_transition(base: Path, candidate: Path) -> list[str]:
    errors = check_graph_transition(base, candidate)
    for name in ("ownership", "inventories"):
        errors.extend(trusted_helper(name).check_transition(base, candidate))
    return errors


def write_crate(root: Path, name: str, deps: dict[str, dict] | None = None, **tables: dict) -> None:
    crate = root / "crates" / name
    crate.mkdir(parents=True)
    lines = [
        "[package]",
        f'name = "{name}"',
        'version = "0.0.0"',
        'edition = "2021"',
        "publish = false",
        "",
    ]
    if deps:
        lines.append("[dependencies]")
        for dep, spec in deps.items():
            lines.append(_dep_line(dep, spec))
        lines.append("")
    for table_name, table in tables.items():
        heading = table_name.replace("_", "-") if not table_name.startswith("target.") else None
        if heading:
            lines.append(f"[{heading}]")
        else:
            lines.append(f"[{table_name}]")
        for dep, spec in table.items():
            lines.append(_dep_line(dep, spec))
        lines.append("")
    (crate / "Cargo.toml").write_text("\n".join(lines), encoding="utf-8")
    src = crate / "src"
    src.mkdir()
    (src / "lib.rs").write_text("pub fn _fixture() {}\n", encoding="utf-8")


def _dep_line(name: str, spec: dict) -> str:
    parts = []
    for key, value in spec.items():
        if isinstance(value, bool):
            parts.append(f"{key} = {str(value).lower()}")
        else:
            parts.append(f'{key} = "{value}"')
    return f"{name} = {{ {', '.join(parts)} }}"


def write_workspace(
    root: Path,
    members: list[str],
    ws_deps: dict | None = None,
    *,
    member_globs: list[str] | None = None,
    root_package: str | None = None,
) -> None:
    root.mkdir(parents=True, exist_ok=True)
    listed = member_globs if member_globs is not None else [f"crates/{m}" for m in members]
    lines: list[str] = []
    if root_package:
        lines.extend(
            [
                "[package]",
                f'name = "{root_package}"',
                'version = "0.0.0"',
                'edition = "2021"',
                "publish = false",
                "",
            ]
        )
    lines.extend(
        ["[workspace]", 'resolver = "2"', f"members = [{', '.join(repr(p) for p in listed)}]", ""]
    )
    if ws_deps:
        lines.append("[workspace.dependencies]")
        for name, spec in ws_deps.items():
            lines.append(_dep_line(name, spec))
        lines.append("")
    (root / "Cargo.toml").write_text("\n".join(lines), encoding="utf-8")


def write_policy(path: Path, members: list[str], allows: list[dict], externals: list[str] | None = None) -> None:
    lines = ["schema_version = 1", "[workspace]", f"members = [{', '.join(repr(m) for m in members)}]"]
    if externals:
        lines.append(f"externals = [{', '.join(repr(e) for e in externals)}]")
    lines.append("")
    for allow in allows:
        lines.append("[[allow]]")
        lines.append(f'from = "{allow["from"]}"')
        lines.append(f'to = "{allow["to"]}"')
        kinds = ", ".join(repr(k) for k in allow["kinds"])
        lines.append(f"kinds = [{kinds}]")
        lines.append(f'reason = "{allow["reason"]}"')
        lines.append("")
    path.write_text("\n".join(lines), encoding="utf-8")


def expect_errors(label: str, errors: list[str], needle: str) -> list[str]:
    if any(needle in err for err in errors):
        return []
    joined = "\n  ".join(errors) if errors else "(no errors; graph accepted)"
    return [f"self-test {label}: expected a finding containing {needle!r}, got:\n  {joined}"]


def expect_clean(label: str, errors: list[str]) -> list[str]:
    if not errors:
        return []
    return [f"self-test {label}: expected a clean graph, got:\n  " + "\n  ".join(errors)]


def run_self_test() -> int:
    # During an E -> E' qualification the old fixture code drives E' APIs.
    # This selector is only used by tests; admission never reads it.
    subject = sys.modules[__name__]
    proposed_root = os.environ.get("F0_ARCH_TEST_CHECKER_ROOT")
    if proposed_root:
        path = Path(proposed_root) / "tools/architecture/check.py"
        spec = importlib.util.spec_from_file_location("architecture_proposed_selftest", path)
        subject = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = subject
        spec.loader.exec_module(subject)
    check_graph = subject.check_graph
    load_policy = subject.load_policy
    failures: list[str] = []
    with tempfile.TemporaryDirectory(prefix="274bot-arch-", dir=repo_root_from_script() / "target") as tmp:
        base = Path(tmp)

        # 1. Reverse / inappropriate production edge.
        bad = base / "disallowed-edge"
        write_workspace(bad, ["vault", "api"])
        write_crate(bad, "vault", {"api": {"path": "../api"}})
        write_crate(bad, "api")
        policy = base / "policy-empty.toml"
        write_policy(policy, ["vault", "api"], [])
        failures += expect_errors(
            "disallowed-edge",
            check_graph(bad, load_policy(policy)),
            "disallowed crate edge: vault -> api (normal)",
        )

        # 2. Workspace alias must not hide a forbidden edge.
        alias = base / "workspace-alias"
        write_workspace(
            alias,
            ["leaf", "host"],
            ws_deps={"host_alias": {"path": "crates/host", "package": "host"}},
        )
        write_crate(alias, "leaf", {"host_alias": {"workspace": True}})
        write_crate(alias, "host")
        write_policy(policy := base / "policy-alias.toml", ["leaf", "host"], [])
        failures += expect_errors(
            "workspace-alias",
            check_graph(alias, load_policy(policy)),
            "disallowed crate edge: leaf -> host (normal)",
        )

        # 3. Target table must not bypass the allowlist.
        targeted = base / "target-dep"
        write_workspace(targeted, ["leaf", "host"])
        write_crate(
            targeted,
            "leaf",
            **{"target.'cfg(unix)'.dependencies": {"host": {"path": "../host"}}},
        )
        write_crate(targeted, "host")
        write_policy(policy := base / "policy-target.toml", ["leaf", "host"], [])
        failures += expect_errors(
            "target-dep",
            check_graph(targeted, load_policy(policy)),
            "disallowed crate edge: leaf -> host (target)",
        )

        # 4. Unknown workspace member.
        unknown = base / "unknown-member"
        write_workspace(unknown, ["vault", "extra"])
        write_crate(unknown, "vault")
        write_crate(unknown, "extra")
        write_policy(policy := base / "policy-unknown.toml", ["vault"], [])
        failures += expect_errors(
            "unknown-member",
            check_graph(unknown, load_policy(policy)),
            "unknown workspace member",
        )

        # 5. Permitted dev-only edge is accepted; the same edge as normal is not.
        dev_ok = base / "dev-only-ok"
        write_workspace(dev_ok, ["script", "clientlib"])
        write_crate(dev_ok, "script", **{"dev_dependencies": {"clientlib": {"path": "../clientlib"}}})
        write_crate(dev_ok, "clientlib")
        write_policy(
            policy := base / "policy-dev.toml",
            ["script", "clientlib"],
            [
                {
                    "from": "script",
                    "to": "clientlib",
                    "kinds": ["dev"],
                    "reason": "tests only",
                }
            ],
        )
        loaded = load_policy(policy)
        failures += expect_clean("dev-only-ok", check_graph(dev_ok, loaded))

        dev_bypass = base / "dev-only-bypass"
        write_workspace(dev_bypass, ["script", "clientlib"])
        write_crate(dev_bypass, "script", {"clientlib": {"path": "../clientlib"}})
        write_crate(dev_bypass, "clientlib")
        failures += expect_errors(
            "dev-only-bypass",
            check_graph(dev_bypass, loaded),
            "disallowed crate edge kind: script -> clientlib is normal",
        )

        # 6. optional-only must reject a required target edge; target+optional
        #    stays optional and is accepted. Member {workspace=true, optional=true}
        #    is classified from the member table, not workspace.dependencies.
        opt_policy_path = base / "policy-optional.toml"
        write_policy(
            opt_policy_path,
            ["host-play", "scenario"],
            [
                {
                    "from": "host-play",
                    "to": "scenario",
                    "kinds": ["optional"],
                    "reason": "feature-gated only",
                }
            ],
        )
        opt_policy = load_policy(opt_policy_path)

        req_target = base / "optional-vs-required-target"
        write_workspace(req_target, ["host-play", "scenario"])
        write_crate(
            req_target,
            "host-play",
            **{"target.'cfg(unix)'.dependencies": {"scenario": {"path": "../scenario"}}},
        )
        write_crate(req_target, "scenario")
        failures += expect_errors(
            "optional-vs-required-target",
            check_graph(req_target, opt_policy),
            "disallowed crate edge kind: host-play -> scenario is target",
        )

        opt_target = base / "optional-target-ok"
        write_workspace(opt_target, ["host-play", "scenario"])
        write_crate(
            opt_target,
            "host-play",
            **{
                "target.'cfg(unix)'.dependencies": {
                    "scenario": {"path": "../scenario", "optional": True}
                }
            },
        )
        write_crate(opt_target, "scenario")
        failures += expect_clean("optional-target-ok", check_graph(opt_target, opt_policy))

        ws_opt_member = base / "member-workspace-optional"
        write_workspace(
            ws_opt_member,
            ["host-play", "scenario"],
            ws_deps={"scenario": {"path": "crates/scenario"}},
        )
        write_crate(
            ws_opt_member,
            "host-play",
            {"scenario": {"workspace": True, "optional": True}},
        )
        write_crate(ws_opt_member, "scenario")
        failures += expect_clean(
            "member-workspace-optional", check_graph(ws_opt_member, opt_policy)
        )

        invalid_ws_opt = base / "invalid-workspace-optional"
        write_workspace(
            invalid_ws_opt,
            ["host-play", "scenario"],
            ws_deps={"scenario": {"path": "crates/scenario", "optional": True}},
        )
        write_crate(invalid_ws_opt, "host-play", {"scenario": {"workspace": True}})
        write_crate(invalid_ws_opt, "scenario")
        failures += expect_errors(
            "invalid-workspace-optional",
            check_graph(invalid_ws_opt, opt_policy),
            "invalid workspace.dependencies.scenario",
        )

        # 7. Path dep on a crate that is not a workspace member still fails.
        sneak = base / "path-nonmember"
        write_workspace(sneak, ["vault"])
        write_crate(sneak, "vault", {"extra": {"path": "../extra"}})
        extra = sneak / "crates" / "extra"
        extra.mkdir(parents=True)
        (extra / "Cargo.toml").write_text(
            '[package]\nname = "extra"\nversion = "0.0.0"\nedition = "2021"\npublish = false\n',
            encoding="utf-8",
        )
        (extra / "src").mkdir()
        (extra / "src" / "lib.rs").write_text("pub fn _fixture() {}\n", encoding="utf-8")
        write_policy(policy := base / "policy-vault.toml", ["vault"], [])
        failures += expect_errors(
            "path-nonmember",
            check_graph(sneak, load_policy(policy)),
            "not a policy member or external",
        )

        # 8. Implicit root [package] cannot escape membership. Globs fail closed.
        implicit = base / "implicit-root"
        write_workspace(implicit, ["vault"], root_package="rootpkg")
        write_crate(implicit, "vault")
        write_policy(policy := base / "policy-implicit.toml", ["vault"], [])
        failures += expect_errors(
            "implicit-root",
            check_graph(implicit, load_policy(policy)),
            "unknown workspace member",
        )

        globbed = base / "member-glob"
        write_workspace(globbed, ["vault"], member_globs=["crates/*"])
        write_crate(globbed, "vault")
        write_policy(policy := base / "policy-glob.toml", ["vault"], [])
        try:
            check_graph(globbed, load_policy(policy))
            failures.append("self-test member-glob: expected SystemExit on glob members")
        except SystemExit as exc:
            if "crates/*" not in str(exc):
                failures.append(f"self-test member-glob: unexpected SystemExit: {exc}")

    if failures:
        sys.stderr.write("\n".join(failures) + "\n")
        return 1
    print(
        "architecture self-test: rejected disallowed/alias/target/unknown/dev-bypass/"
        "required-target-vs-optional/invalid-workspace-optional/path-nonmember/"
        "implicit-root/glob; accepted permitted dev-only, target-optional, "
        "and member workspace+optional"
    )
    return 0


def repo_root_from_script() -> Path:
    return Path(__file__).resolve().parents[2]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root",
        type=Path,
        default=None,
        help="workspace root (default: repository containing this script)",
    )
    parser.add_argument(
        "--policy",
        type=Path,
        default=None,
        help="policy.toml (default: tools/architecture/policy.toml)",
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="exercise disposable fixtures; do not read the product workspace",
    )
    parser.add_argument("--base", type=Path, help="data baseline for a diagnostic comparison; admission uses trusted_gate.py")
    args = parser.parse_args(argv)
    if args.self_test:
        (repo_root_from_script() / "target").mkdir(exist_ok=True)
    if args.self_test:
        return run_self_test()

    root = (args.root or repo_root_from_script()).resolve()
    policy_path = (args.policy or root / "tools/architecture/policy.toml").resolve()
    policy = load_policy(policy_path)
    if args.base is not None:
        errors = check_transition(args.base.resolve(), root)
    else:
        errors = check_tree(root) if args.policy is None else check_graph(root, policy)
    if errors:
        sys.stderr.write("architecture check failed:\n")
        for err in errors:
            sys.stderr.write(f"- {err}\n")
        return 1
    print(
        f"architecture: {len(policy.members)} workspace crates match {policy_path.name} "
        f"({len(policy.allowed)} allowed edges)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
