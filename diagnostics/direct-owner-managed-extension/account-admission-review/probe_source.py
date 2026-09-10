#!/usr/bin/env python3
"""Source-only account-admission timing probe. No live/SSH/account/cache reads."""
from __future__ import annotations

import hashlib
import json
import os
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
TAR = (
    ROOT
    / "diagnostics/direct-owner-native-preparation/root-runtime-preparation-01"
    / "frozen-direct-owner-source.tar.gz"
)
EXPECTED_TAR = "2c36d254c37c5ed56a55f78728357296d62be602f8653ed9ec041cd37d099d5d"
MEMBERS = {
    "lib": "direct-owner-source/crates/host-play/src/lib.rs",
    "memory": "direct-owner-source/crates/host-play/src/memory.rs",
    "tui_bin": "direct-owner-source/crates/tui/src/bin.rs",
    "owner_capture": "direct-owner-source/crates/host-play/src/owner_capture.rs",
    "owner_out": "direct-owner-source/crates/host-play/src/owner_capture_output.rs",
    "vault": "direct-owner-source/crates/vault/src/lib.rs",
}
CLAIMED = {
    "lib": "f997f22f00bb205cdad74eec70d9dfc5750c3f8f80d9ba52c024f5ea4ea9120f",
    "memory": "76e633acf9d4dcd1fd03f1a57ea722f5111025790071679119e7e5932ece2991",
}
NEEDLES = (
    "BOT_MEMORY_NAMES",
    "BOT_ACCOUNT",
    "BOT_LIVE_NAMES",
    "BOT_USER",
    "mint_live_names",
    "--user",
)


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def lines_with(text: str, needle: str) -> list[dict]:
    hits = []
    for i, line in enumerate(text.splitlines(), 1):
        if needle in line:
            hits.append({"line": i, "text": line.rstrip()[:240]})
    return hits


def main() -> int:
    out_dir = Path(__file__).resolve().parent
    tar_sha = sha256_file(TAR)
    members = {}
    searches = {}
    with tarfile.open(TAR, "r:gz") as archive:
        names = archive.getnames()
        for key, member in MEMBERS.items():
            extracted = archive.extractfile(member)
            if extracted is None:
                raise SystemExit(f"missing tar member {member}")
            data = extracted.read()
            text = data.decode("utf-8")
            members[key] = {
                "path": member,
                "sha256": sha256_bytes(data),
                "bytes": len(data),
            }
            searches[key] = {needle: lines_with(text, needle) for needle in NEEDLES}

            def write_span(name: str, start: int, end: int) -> None:
                span = text.splitlines()[start - 1 : end]
                numbered = "\n".join(f"{i}|{line}" for i, line in enumerate(span, start))
                (out_dir / name).write_text(numbered + "\n", encoding="utf-8")

            if key == "lib":
                write_span("excerpt-mint-live-names.txt", 64, 82)
            elif key == "memory":
                write_span("excerpt-prepare-unseeded.txt", 675, 763)
                write_span("excerpt-qualification-slots.txt", 409, 430)
                write_span("excerpt-write-qualification.txt", 1622, 1683)
            elif key == "tui_bin":
                write_span("excerpt-tui-memory-boot.txt", 1168, 1196)
                write_span("excerpt-tui-interactive-user.txt", 1198, 1240)

    moving = {}
    for rel in (
        "crates/host-play/src/lib.rs",
        "crates/host-play/src/memory.rs",
        "crates/host-play/src/main.rs",
    ):
        path = ROOT / rel
        moving[rel] = sha256_file(path) if path.is_file() else None
    controllers = {}
    for rel in (
        "docs/memory/run_managed_cell.py",
        "docs/memory/run_current_tui_calibration.py",
        "docs/memory/run_diagnostic.py",
        "docs/memory/cpu_screen.py",
        "docs/memory/qualify_control.py",
    ):
        controllers[rel] = sha256_file(ROOT / rel)

    def write_workspace_span(rel: str, name: str, start: int, end: int) -> None:
        lines = (ROOT / rel).read_text(encoding="utf-8").splitlines()
        numbered = "\n".join(f"{i}|{lines[i - 1]}" for i in range(start, end + 1))
        (out_dir / name).write_text(numbered + "\n", encoding="utf-8")

    write_workspace_span(
        "docs/memory/run_managed_cell.py", "excerpt-account-evidence.py.txt", 885, 906
    )
    write_workspace_span(
        "docs/memory/run_managed_cell.py", "excerpt-direct-context.py.txt", 707, 726
    )
    write_workspace_span(
        "docs/memory/run_diagnostic.py", "excerpt-spawn-handoff.py.txt", 270, 290
    )
    write_workspace_span(
        "docs/memory/run_diagnostic.py", "excerpt-popen-argv.py.txt", 437, 456
    )
    write_workspace_span(
        "docs/memory/run_diagnostic.py", "excerpt-child-env.py.txt", 197, 257
    )
    write_workspace_span(
        "docs/memory/run_current_tui_calibration.py",
        "excerpt-clean-environment.py.txt",
        216,
        237,
    )
    write_workspace_span(
        "docs/memory/cpu_screen.py", "excerpt-cpu-screen-qualify.py.txt", 22, 70
    )

    result = {
        "scope": "source-only; no account identity, cache, SSH, native, or live admission",
        "tar_path": str(TAR.relative_to(ROOT)),
        "tar_sha256": tar_sha,
        "tar_sha256_expected": EXPECTED_TAR,
        "tar_sha256_match": tar_sha == EXPECTED_TAR,
        "tar_member_count": len(names),
        "members": members,
        "claimed_observation_hashes_match": {
            key: members[key]["sha256"] == digest for key, digest in CLAIMED.items()
        },
        "git_dcdbeebf_present": False,
        "workspace_controller_sha256": controllers,
        "moving_host_play_sha256": moving,
        "moving_host_play_equals_frozen": {
            "crates/host-play/src/lib.rs": moving["crates/host-play/src/lib.rs"]
            == members["lib"]["sha256"],
            "crates/host-play/src/memory.rs": moving["crates/host-play/src/memory.rs"]
            == members["memory"]["sha256"],
        },
        "searches": {
            key: {
                needle: {
                    "count": len(hits),
                    "lines": [h["line"] for h in hits],
                    "first": hits[0]["text"] if hits else None,
                }
                for needle, hits in found.items()
            }
            for key, found in searches.items()
        },
        "no_name_override_env_in_frozen_memory_or_tui": all(
            searches[key][needle] == []
            for key in ("memory", "tui_bin", "lib")
            for needle in ("BOT_MEMORY_NAMES", "BOT_ACCOUNT", "BOT_LIVE_NAMES", "BOT_USER")
        ),
    }
    path = out_dir / "probe-result.json"
    path.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"wrote": str(path), "tar_match": result["tar_sha256_match"]}, sort_keys=True))
    return 0 if result["tar_sha256_match"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
