#!/usr/bin/env python3
"""Assert CLI --server-root B wins over env root A for offline fixture writer."""
from __future__ import annotations

import json
import pathlib
import sys


def load(out: pathlib.Path, tag: str) -> dict:
    return json.loads((out / f"{tag}.json").read_text())


def root_of(r: dict) -> str:
    return pathlib.Path(r["provenance"]["server_root"]).resolve().as_posix()


def main() -> int:
    if len(sys.argv) != 4:
        print("usage: check_server_root_selection.py OUT_DIR ROOT_A ROOT_B", file=sys.stderr)
        return 2
    out = pathlib.Path(sys.argv[1])
    ra = pathlib.Path(sys.argv[2]).resolve().as_posix()
    rb = pathlib.Path(sys.argv[3]).resolve().as_posix()

    pure_a = load(out, "pureA")
    cli_b = load(out, "cliB")
    pure_b = load(out, "pureB")

    assert root_of(pure_a) == ra, f"pureA root {root_of(pure_a)} != {ra}"
    assert root_of(cli_b) == rb, f"cliB root {root_of(cli_b)} != {rb} (CLI ignored?)"
    assert root_of(pure_b) == rb, f"pureB root {root_of(pure_b)} != {rb}"

    # CLI-B must match pure-B (same selected root), not pure-A.
    assert cli_b["sav_sha256"] == pure_b["sav_sha256"], (
        f"cliB sha {cli_b['sav_sha256']} != pureB {pure_b['sav_sha256']}"
    )
    assert cli_b["sav_bytes"] == pure_b["sav_bytes"]
    assert cli_b["sav_sha256"] != pure_a["sav_sha256"], (
        "cliB matched pureA — env/default root still driving serialization"
    )

    ha = pure_a["provenance"].get("engine_git_head")
    hb = pure_b["provenance"].get("engine_git_head")
    if ha and hb and ha != hb:
        assert cli_b["provenance"].get("engine_git_head") == hb, "cliB engine_git_head != pureB"

    known_274_sha = "b17706857303ac4ac27b8fb5890dff22ea6146a83b86894b2827dbb7aa962183"
    # pureA is the known default 274 fixture when using Server/engine thiever.
    if pure_a["sav_sha256"] != known_274_sha and pure_a["sav_bytes"] != 297:
        print(
            f"note: pureA not the pinned 297B/4c95 fixture (sha={pure_a['sav_sha256'][:16]} bytes={pure_a['sav_bytes']})",
            file=sys.stderr,
        )
    assert cli_b["sav_sha256"] != known_274_sha, "cliB still 274 default fixture hash"
    assert pure_b["sav_bytes"] >= 8 and cli_b["sav_bytes"] >= 8

    # Pack path in receipt must sit under selected root.
    pack_b = pathlib.Path(cli_b["provenance"]["pack_dir"]).resolve().as_posix()
    assert pack_b.startswith(rb + "/") or pack_b == rb + "/data/pack", (
        f"cliB pack_dir {pack_b} not under {rb}"
    )

    print(
        json.dumps(
            {
                "ok": True,
                "root_a": ra,
                "root_b": rb,
                "pureA": {
                    "bytes": pure_a["sav_bytes"],
                    "sha": pure_a["sav_sha256"],
                    "head": ha,
                },
                "cliB": {
                    "bytes": cli_b["sav_bytes"],
                    "sha": cli_b["sav_sha256"],
                    "head": cli_b["provenance"].get("engine_git_head"),
                },
                "pureB": {
                    "bytes": pure_b["sav_bytes"],
                    "sha": pure_b["sav_sha256"],
                    "head": hb,
                },
            },
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
