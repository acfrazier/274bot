#!/usr/bin/env python3
"""Prepare the standalone NPR benchmark against one explicit source root."""
import argparse
import hashlib
import json
from pathlib import Path


def prepare(source_root, output):
    source_root = source_root.resolve(strict=True)
    output = output.absolute()
    package = "nav-planner"
    nav = source_root / "crates" / package
    api_crate = source_root / "crates" / "api"
    for crate in (nav, api_crate):
        if not (crate / "Cargo.toml").is_file():
            raise ValueError("missing nav-planner/API dependency: %s" % crate)
    if output.exists():
        raise ValueError("output must be a fresh directory: %s" % output)
    # Preserve this source root's committed dependency selections when Cargo adds
    # the standalone package, instead of resolving history against today's index.
    source_lock = (source_root / "Cargo.lock").read_bytes()
    kit = Path(__file__).resolve().parent
    sources = ("src/main.rs", "src/footprint.rs", "queries.json")
    contents = {name: (kit / name).read_bytes() for name in sources}
    if hashlib.sha256(contents["queries.json"]).hexdigest() != "554de14c680ccb2a53cc4cdf5469ccc4115943ee7e25db77756d4050b8f9bb42":
        raise ValueError("frozen query fixture SHA-256 mismatch")
    manifest = """[package]
name = "npr-proof"
version = "0.0.0"
edition = "2021"
publish = false

[workspace]


[dependencies]
api = { path = %s }
nav = { package = %s, path = %s }
serde = { version = "=1.0.219", features = ["derive"] }
serde_json = "=1.0.140"
sha2 = "=0.10.9"

[profile.dev.package.%s]
opt-level = 3
""" % (
        json.dumps(str(api_crate)), json.dumps(package), json.dumps(str(nav)),
        json.dumps(package),
    )
    (output / "src").mkdir(parents=True)
    for name, content in contents.items():
        (output / name).write_bytes(content)
    (output / "Cargo.toml").write_text(manifest)
    (output / "Cargo.lock").write_bytes(source_lock)
    provenance = {
        "source_root": str(source_root), "api": "observation",
        "nav_package": package,
        "source_workspace_cargo_lock_sha256": hashlib.sha256(source_lock).hexdigest(),
        "source_sha256": {name: hashlib.sha256(content).hexdigest() for name, content in contents.items()},
        "input_trace_sha256": "a7792cb000aba1303e4ad2d87e977a646f46feccd1f88e3ef789910ac614da0a",
        "limits": "current observation API only; no API fallback, global quest-family lookup or product source patch",
    }
    (output / "adapter-provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")
    return provenance


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = prepare(args.source_root, args.output)
    except (OSError, ValueError) as error:
        parser.error(str(error))
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
