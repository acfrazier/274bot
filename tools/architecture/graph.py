"""Effective graph, Cargo provenance and supported-feature debt comparison.

All target cfgs are included. This is a cross-platform permission union, not a
host-only Cargo resolution. No candidate build scripts or binaries are run.
"""
from collections import defaultdict
from pathlib import Path


def dependency_records(checker, root):
    _, by_name, by_path, ws_deps = checker.discover_workspace(root)
    manifests = {name: checker.load_toml(path / "Cargo.toml") for name, path in by_name.items()}
    records = []
    for name, manifest in manifests.items():
        tables = [("", key, kind, manifest.get(key, {})) for key, kind in checker.DEP_TABLES]
        for cfg, target in manifest.get("target", {}).items():
            if not isinstance(target, dict):
                raise ValueError("unsupported Cargo target table")
            tables.extend((" ".join(cfg.split()), key, "target" if kind == "normal" else kind, target.get(key, {}))
                          for key, kind in checker.DEP_TABLES)
        for cfg, table_name, table_kind, table in tables:
            for alias, value in table.items():
                spec = checker.spec_map(value)
                dst, is_path = checker.resolve_dep_name(alias, spec, ws_deps, by_name[name], root, by_path)
                merged = dict(checker.workspace_dep_lookup(ws_deps, alias)) if spec.get("workspace") else {}
                inherited_features = merged.get("features", [])
                merged.update(spec)
                features = sorted(set(inherited_features) | set(spec.get("features", [])))
                records.append({
                    "from": name, "to": dst, "alias": alias, "kind": checker.classify_kind(table_kind, spec),
                    "cfg": cfg, "table": table_name, "optional": spec.get("optional") is True,
                    "default_features": merged.get("default-features", True),
                    "features": features, "local": is_path or dst in by_name,
                })
    return manifests, records


def feature_activation(manifests, records, config):
    """Compute optional dependencies and transitive feature requests to a fixpoint."""
    enabled = defaultdict(set)
    active = set(config["packages"])
    requested_deps = set()
    activated = set()
    if config["default_features"]:
        for package in active:
            enabled[package].add("default")
    for request in config["features"]:
        if "/" not in request:
            raise ValueError(f"configuration feature must be package/feature: {request}")
        package, feature = request.split("/", 1)
        if package not in active:
            raise ValueError(f"configuration feature package not selected: {request}")
        enabled[package].add(feature)
    previous = None
    while True:
        state = (frozenset(active), frozenset((p, f) for p, fs in enabled.items() for f in fs),
                 frozenset(requested_deps), frozenset(activated))
        if state == previous:
            break
        previous = state
        for package in sorted(active):
            features = manifests.get(package, {}).get("features", {})
            own = [r for r in records if r["from"] == package]
            dep_names = {r["alias"] for r in own}
            suppressed = {item[4:] for values in features.values() for item in values if item.startswith("dep:")}
            for feature in list(enabled[package]):
                if feature not in features:
                    if feature == "default":
                        continue
                    if feature in dep_names and feature not in suppressed:
                        requested_deps.add((package, feature))
                        continue
                    raise ValueError(f"unknown supported feature: {package}/{feature}")
                for item in features[feature]:
                    if item.startswith("dep:"):
                        if item[4:] not in dep_names:
                            raise ValueError(f"unknown optional dependency: {package}/{item}")
                        requested_deps.add((package, item[4:]))
                    elif "/" in item:
                        dep, child_feature = item.split("/", 1)
                        weak = dep.endswith("?")
                        dep = dep.removesuffix("?")
                        matching = [r for r in own if r["alias"] == dep]
                        if not matching:
                            raise ValueError(f"unknown dependency feature: {package}/{item}")
                        if not weak:
                            requested_deps.add((package, dep))
                        if not weak or (package, dep) in activated:
                            for row in matching:
                                if row["to"] in manifests:
                                    enabled[row["to"]].add(child_feature)
                    else:
                        enabled[package].add(item)
        for row in records:
            if row["from"] not in active or (row["kind"] == "dev" and not config["dev"]):
                continue
            if row["optional"] and (row["from"], row["alias"]) not in requested_deps:
                continue
            activated.add((row["from"], row["alias"]))
            if row["to"] in manifests:
                active.add(row["to"])
                enabled[row["to"]].update(row["features"])
                if row["default_features"]:
                    enabled[row["to"]].add("default")
    result = {("feature", package, feature) for package, fs in enabled.items()
              if package in active for feature in fs}
    result.update(("dependency", r["from"], r["to"], r["kind"], r["cfg"], r["table"])
                  for r in records if (r["from"], r["alias"]) in activated
                  and (r["kind"] != "dev" or config["dev"]))
    return result


def graph_state(checker, root):
    policy = checker.load_policy(root / "tools/architecture/policy.toml")
    _, names, paths, deps = checker.discover_workspace(root)
    edges = {(e.src, e.dst, e.kind) for e in checker.collect_edges(names, paths, deps, root)}
    manifests, records = dependency_records(checker, root)
    migration_pairs = {(r["from"], r["to"]) for r in policy.migrations.values()}
    exceptional = [r for r in records if r["local"] and (
        (r["from"], r["to"]) in migration_pairs or r["kind"] in {"optional", "dev", "build"})]
    provenance = {(r["from"], r["to"], r["kind"], r["cfg"], r["table"],
                   r["default_features"], tuple(r["features"])) for r in exceptional}
    for config in policy.configurations:
        # Check each selected product independently as well as the aggregate.
        # Otherwise panel's live-harness could hide a new host-play activation.
        for selected in config["packages"]:
            isolated = dict(config, packages=[selected],
                            features=[f for f in config["features"] if f.startswith(selected + "/")])
            individual = feature_activation(manifests, records, isolated)
            exceptional_pairs = {(row["from"], row["to"]) for row in exceptional}
            for item in individual:
                if item[0] == "dependency" and (item[1], item[2]) in exceptional_pairs:
                    provenance.add(("isolated-activation", config["id"], selected, *item[1:]))
                elif item[0] == "feature" and item[2] != "default":
                    provenance.add(("feature-activation", config["id"], selected, *item[1:]))
        activation = feature_activation(manifests, records, config)
        for row in exceptional:
            key = ("dependency", row["from"], row["to"], row["kind"], row["cfg"], row["table"])
            if key in activation:
                provenance.add(("activation", config["id"], *key[1:]))
        if config["status"] == "enforced":
            for forbidden in config["forbidden_features"]:
                package, feature = forbidden.split("/", 1)
                if ("feature", package, feature) in activation:
                    raise ValueError(f"forbidden supported feature: {config['id']}: {forbidden}")
            for requirement in config["optional_dependencies"]:
                package, dependency = requirement.split("/", 1)
                matched = [r for r in records if r["from"] == package and r["to"] == dependency]
                if not matched or any(not r["optional"] for r in matched):
                    raise ValueError(f"required optional dependency: {config['id']}: {requirement}")
    return policy, edges, provenance


def check_tree(checker, root: Path):
    try:
        policy, _, _ = graph_state(checker, root)
        return checker.check_graph(root, policy)
    except (OSError, ValueError, TypeError, KeyError, SystemExit) as exc:
        return [f"invalid graph input: {exc}"]


def check_transition(checker, base: Path, candidate: Path):
    errors = check_tree(checker, base) + check_tree(checker, candidate)
    if errors:
        return errors
    old, old_edges, old_provenance = graph_state(checker, base)
    new, new_edges, new_provenance = graph_state(checker, candidate)
    for edge in sorted(new_edges - old_edges):
        errors.append(f"effective edge added: {edge}")
    for identity, record in new.migrations.items():
        if old.migrations.get(identity) != record:
            errors.append(f"migration identity added or changed: {identity}")
    if old.members != new.members or old.externals != new.externals:
        errors.append("workspace membership/external definitions changed")
    for pair, permission in new.allowed.items():
        before = old.allowed.get(pair, checker.Allow())
        if not permission.kinds <= before.kinds:
            errors.append(f"permanent permission widened: {pair}")
    if old.configurations != new.configurations:
        errors.append("supported feature/target matrix changed")
    for record in sorted(new_provenance - old_provenance, key=repr):
        errors.append(f"exceptional provenance/activation widened: {record}")
    return errors
