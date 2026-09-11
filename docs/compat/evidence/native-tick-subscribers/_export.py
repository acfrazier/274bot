#!/usr/bin/env python3
"""Exclusive Git-blob export of committed host+client plus owned overlay."""
from __future__ import annotations

import hashlib
import os
import pathlib
import subprocess
import sys

SRC = pathlib.Path("/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision")
EXPORT = SRC / ".superpowers/review-exports/native-tick-subscribers-t_e8541da5"
OVERLAY = [
    "crates/script/src/shim/bot_host.js",
    "crates/script/src/shim/execution.js",
    "crates/script/src/load.rs",
    "crates/script/tests/tick_subscriber.rs",
]


def run(args, cwd=None):
    return subprocess.check_output(args, cwd=cwd, text=True)


def export_tree(repo: pathlib.Path, commit: str, dest: pathlib.Path) -> int:
    dest.mkdir(parents=True, exist_ok=True)
    listing = run(["git", "ls-tree", "-r", "-z", commit], cwd=repo)
    count = 0
    for entry in listing.split("\0"):
        if not entry:
            continue
        meta, path = entry.split("\t", 1)
        mode, kind, sha = meta.split(" ")
        if kind != "blob":
            continue
        if path.startswith("/") or ".." in pathlib.PurePosixPath(path).parts:
            raise SystemExit(f"path traversal: {path}")
        if mode == "120000":
            raise SystemExit(f"symlink rejected: {path}")
        out = dest / path
        if out.exists() or out.is_symlink():
            raise SystemExit(f"refusing overwrite: {path}")
        out.parent.mkdir(parents=True, exist_ok=True)
        blob = subprocess.check_output(["git", "cat-file", "blob", sha], cwd=repo)
        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
        fd = os.open(out, flags, 0o755 if mode == "100755" else 0o644)
        try:
            os.write(fd, blob)
        finally:
            os.close(fd)
        count += 1
    return count


def overlay_copy(rel: str) -> str:
    src = SRC / rel
    dest = EXPORT / rel
    dest.parent.mkdir(parents=True, exist_ok=True)
    if dest.exists() or dest.is_symlink():
        dest.unlink()
    data = src.read_bytes()
    dest.write_bytes(data)
    return hashlib.sha256(data).hexdigest()


def main() -> int:
    if EXPORT.exists():
        raise SystemExit(f"export already exists: {EXPORT}")
    host = run(["git", "rev-parse", "HEAD"], cwd=SRC).strip()
    client = run(["git", "rev-parse", "HEAD:vendor/fr-client-rust"], cwd=SRC).strip()
    host_files = export_tree(SRC, host, EXPORT)
    client_repo = SRC / "vendor/fr-client-rust"
    client_dest = EXPORT / "vendor/fr-client-rust"
    client_files = export_tree(client_repo, client, client_dest)
    hashes = {rel: overlay_copy(rel) for rel in OVERLAY}
    print(f"host={host}")
    print(f"client={client}")
    print(f"host_files={host_files}")
    print(f"client_files={client_files}")
    print(f"files={host_files + client_files}")
    print(f"export={EXPORT}")
    for rel, digest in hashes.items():
        print(f"overlay {rel} sha256 {digest}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
