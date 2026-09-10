#!/usr/bin/env python3
"""Record local source and resource identities without starting either world.

This reads public-key fingerprints and selected configuration fields only.
It never reads account saves, vaults, passwords, or private key material.
"""

import argparse
import hashlib
import json
import subprocess
from datetime import datetime, timezone
from pathlib import Path


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def file_identity(path):
    if not path.is_file():
        return {"path": str(path), "present": False}
    return {"path": str(path), "bytes": path.stat().st_size, "sha256": sha256(path)}


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args], text=True).strip()


def source_identity(root):
    changed = subprocess.check_output(
        ["git", "-C", str(root), "diff", "--name-only", "-z", "HEAD"]
    ).decode().split("\0")
    return {
        "path": str(root),
        "commit": git(root, "rev-parse", "HEAD"),
        "tree": git(root, "rev-parse", "HEAD^{tree}"),
        "status": git(root, "status", "--short").splitlines(),
        "modified_tracked_files": [file_identity(root / p) for p in changed if p],
    }


def fixture(root, revision):
    engine = root / "engine"
    content = root / "content"
    config_path = engine / "data/config/world.json"
    config = json.loads(config_path.read_text())
    assert config["engine"]["revision"] == revision, "fixture revision mismatch"
    pack = engine / "data/pack"
    cache = [file_identity(p) for p in sorted((pack / "client").iterdir()) if p.is_file()]
    raw = [file_identity(p) for p in sorted(pack.glob("main_file_cache.*"))]
    public_key = engine / "data/config/public.pem"
    der = subprocess.check_output(
        ["openssl", "pkey", "-pubin", "-in", str(public_key), "-outform", "DER"]
    )
    return {
        "id": "local-" + str(revision),
        "revision": revision,
        "target": "local",
        "engine": source_identity(engine),
        "content": source_identity(content),
        "runtime_config": file_identity(config_path),
        "connection": {
            "host": "127.0.0.1",
            "game_port": config["node"]["port"],
            "asset_port": config["web"]["port"],
            "management_port": config["web"]["managementPort"],
            "production": config["node"]["production"],
            "login_service_enabled": config["login"]["enabled"],
        },
        "public_key_der_sha256": hashlib.sha256(der).hexdigest(),
        "client_cache": cache,
        "raw_cache": raw,
        "local_fixture_handler": file_identity(
            engine / "src/network/game/client/handler/ClientCheatHandler.ts"
        ),
        "local_content_fixture": file_identity(
            content / "scripts/_test/scripts/cheats/cheat_item.rs2"
        ),
        "setup_controls": [file_identity(p) for p in sorted(root.glob("*-289.sh"))],
        "acceptance": "unqualified: source inventory is not login or script proof",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--world-274", type=Path, required=True)
    parser.add_argument("--world-289", type=Path, required=True)
    parser.add_argument("--nav-274", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = {
        "schema_version": 1,
        "captured_at": datetime.now(timezone.utc).isoformat(),
        "fixtures": [fixture(args.world_274, 274), fixture(args.world_289, 289)],
        "navigation": {
            "274": {
                "pack": file_identity(args.nav_274),
                "flags": file_identity(args.nav_274.with_suffix(".navflags")),
                "status": "legacy v8; rebake with campaign binary before acceptance",
            },
            "289": {"status": "not yet baked or qualified; required in step 5"},
        },
    }
    args.output.write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
