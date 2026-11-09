#!/usr/bin/env python3
"""Prove mapped drafts are refused and every qualified guide is admitted by a production release library."""
from __future__ import annotations

import argparse
import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from typing import Any

from live_map import ROOT, load_map

CELL = "script.release-path-refusal"
SOURCE = Path(__file__).with_name("release_path_refusal.rs")
INDEX = ROOT / "crates/script/paths/289/index.json"
SCRIPT_SOURCE = ROOT / "crates/script/src/lib.rs"


class ProofError(ValueError):
    """The cell cannot establish production release admission."""


def mapped_draft_ids(config: dict[str, Any]) -> list[str]:
    paths = {
        path
        for row in config["cover"]
        if any(spec.split("|", 1)[0] == CELL for spec in row["cells"])
        for path in row["globs"]
    }
    if not paths:
        raise ProofError(f"{CELL} has no exact draft paths")
    prefix = "crates/script/paths/289/"
    for path in paths:
        if (
            not path.startswith(prefix)
            or not path.endswith(".json")
            or "/" in path.removeprefix(prefix)
            or any(char in path for char in "*?[]")
            or Path(path).stem in {"index", "priest"}
        ):
            raise ProofError(f"refusal coverage must name an exact draft guide: {path}")
    return sorted(Path(path).stem for path in paths)


def index_lists(index: dict[str, Any]) -> tuple[list[str], list[str], list[str]]:
    """Release-index ids as (qualified, draft, unavailable); each row is exactly one."""
    qualified: list[str] = []
    drafts: list[str] = []
    unavailable: list[str] = []
    seen: set[str] = set()
    for path in index["paths"]:
        id = path["id"]
        if id in seen:
            raise ProofError(f"the release index names {id} twice")
        seen.add(id)
        if path.get("unavailable"):
            unavailable.append(id)
        elif path.get("status") == "qualified":
            qualified.append(id)
        elif path.get("status") == "draft":
            drafts.append(id)
        else:
            raise ProofError(f"release index row {id} is neither qualified, draft nor unavailable")
    if "priest" not in qualified:
        raise ProofError("the qualified Priest guide is missing from the release index")
    return sorted(qualified), sorted(drafts), sorted(unavailable)


def probe_arguments(mapped: list[str], index: dict[str, Any]) -> list[str]:
    """Arguments of the external consumer: one `draft:`, `unavailable:` or `qualified:` per row.

    Refused plus admitted must be the whole index: every index draft is in the
    refusal map and the map names nothing else.
    """
    qualified, drafts, unavailable = index_lists(index)
    unmapped = sorted(set(drafts) - set(mapped))
    if unmapped:
        raise ProofError(
            "index drafts missing from the refusal map (neither refused nor admitted): "
            + ", ".join(unmapped)
        )
    stale = sorted(set(mapped) - set(drafts))
    if stale:
        raise ProofError(
            "the refusal map names rows that are not index drafts; a qualified guide must "
            "leave refusal coverage and gain its own real cell: " + ", ".join(stale)
        )
    return [
        *(f"draft:{id}" for id in drafts),
        *(f"unavailable:{id}" for id in unavailable),
        *(f"qualified:{id}" for id in qualified),
    ]


def release_artifact(messages: list[dict[str, Any]]) -> dict[str, Any]:
    artifacts = [
        message
        for message in messages
        if message.get("reason") == "compiler-artifact"
        and message.get("target", {}).get("name") == "script"
        and "lib" in message.get("target", {}).get("kind", [])
        and Path(message["target"].get("src_path", "")) == SCRIPT_SOURCE
    ]
    if len(artifacts) != 1:
        raise ProofError(f"expected one exact script build artifact, found {len(artifacts)}")
    artifact = artifacts[0]
    profile = artifact.get("profile", {})
    if (
        "test-hooks" in artifact.get("features", [])
        or profile.get("test") is not False
        or profile.get("debug_assertions") is not False
    ):
        raise ProofError("the refusal proof requires a production release library without test-hooks")
    libraries = [path for path in artifact.get("filenames", []) if path.endswith(".rlib")]
    if len(libraries) != 1:
        raise ProofError("the exact script artifact must name one rlib; no directory fallback is allowed")
    metadata = [path for path in artifact.get("filenames", []) if path.endswith(".rmeta")]
    if len(metadata) != 1:
        raise ProofError("the exact script artifact must name one rmeta; no directory fallback is allowed")
    return artifact


def refusal_compile_command(artifact: dict[str, Any], executable: Path) -> list[str]:
    """Use only the library and dependency location named by the validated artifact."""
    library = next(Path(path) for path in artifact["filenames"] if path.endswith(".rlib"))
    metadata = next(Path(path) for path in artifact["filenames"] if path.endswith(".rmeta"))
    return [
        "rustc", "--edition=2021", "--crate-name", "release_path_refusal",
        "-Dwarnings", "-Copt-level=3", str(SOURCE),
        "-L", f"dependency={metadata.parent}", "--extern", f"script={library}",
        "-o", str(executable),
    ]


def prove_release_refusal() -> int:
    ids = mapped_draft_ids(load_map())
    arguments = probe_arguments(ids, json.loads(INDEX.read_text()))
    evidence = os.environ.get("LIVE_EVIDENCE_DIR")
    if not evidence:
        raise ProofError("LIVE_EVIDENCE_DIR must name the cell's evidence directory")
    evidence_dir = Path(evidence)
    evidence_dir.mkdir(parents=True, exist_ok=True)
    command = [
        "cargo", "build", "--release", "--locked", "--package", "script",
        "--lib", "--message-format=json",
    ]
    print("release-refusal build: " + " ".join(command), flush=True)
    result = subprocess.run(command, cwd=ROOT, stdout=subprocess.PIPE, text=True, check=False)
    messages = []
    for line in result.stdout.splitlines():
        message = json.loads(line)
        messages.append(message)
        if message.get("reason") == "compiler-message":
            print(message["message"].get("rendered", ""), file=sys.stderr, end="")
    if result.returncode:
        return result.returncode
    artifact = release_artifact(messages)
    print("release-refusal production artifact: " + json.dumps(artifact, sort_keys=True), flush=True)
    with tempfile.TemporaryDirectory(prefix="release-refusal-", dir=evidence_dir) as temporary:
        executable = Path(temporary) / ("release-refusal.exe" if os.name == "nt" else "release-refusal")
        subprocess.run(
            refusal_compile_command(artifact, executable),
            cwd=ROOT,
            check=True,
        )
        subprocess.run([str(executable), *arguments], cwd=ROOT, check=True)
    return 0


class ReleaseRefusalTests(unittest.TestCase):
    def artifact(self) -> dict[str, Any]:
        return {
            "reason": "compiler-artifact",
            "target": {"name": "script", "kind": ["lib"], "src_path": str(SCRIPT_SOURCE)},
            "features": [],
            "profile": {"test": False, "debug_assertions": False},
            "filenames": [
                "/build/release/libscript.rlib",
                "/build/release/deps/libscript-exact.rmeta",
            ],
        }

    def test_accepts_only_the_exact_production_artifact(self) -> None:
        artifact = self.artifact()
        self.assertEqual(release_artifact([artifact]), artifact)
        for field, value in [("test", True), ("debug_assertions", True)]:
            changed = copy.deepcopy(artifact)
            changed["profile"][field] = value
            with self.subTest(field=field), self.assertRaises(ProofError):
                release_artifact([changed])
        changed = copy.deepcopy(artifact)
        changed["features"] = ["test-hooks"]
        with self.assertRaises(ProofError):
            release_artifact([changed])
        for messages in [[], [artifact, artifact]]:
            with self.subTest(messages=messages), self.assertRaises(ProofError):
                release_artifact(messages)
        changed = copy.deepcopy(artifact)
        changed["target"]["src_path"] = "/another/checkout/crates/script/src/lib.rs"
        with self.assertRaises(ProofError):
            release_artifact([changed])
        changed = copy.deepcopy(artifact)
        changed["filenames"] = []
        with self.assertRaises(ProofError):
            release_artifact([changed])

    def test_cargo_root_library_uses_its_metadata_dependency_directory(self) -> None:
        artifact = release_artifact([self.artifact()])
        command = refusal_compile_command(artifact, Path("/proof/release-refusal"))
        self.assertEqual(
            command[command.index("-L") + 1],
            f"dependency={Path('/build/release/deps')}",
        )
        self.assertEqual(
            command[command.index("--extern") + 1],
            f"script={Path('/build/release/libscript.rlib')}",
        )

    def test_refuses_missing_or_ambiguous_metadata(self) -> None:
        for metadata in [
            [],
            ["/build/release/deps/libscript-first.rmeta", "/build/release/deps/libscript-second.rmeta"],
        ]:
            artifact = self.artifact()
            artifact["filenames"] = ["/build/release/libscript.rlib", *metadata]
            with self.subTest(metadata=metadata), self.assertRaises(ProofError):
                release_artifact([artifact])

    def test_requires_exact_nonempty_draft_paths(self) -> None:
        def config(path: str) -> dict[str, Any]:
            return {"cover": [{"globs": [path], "cells": [f"{CELL}|command|selector"]}]}

        self.assertEqual(mapped_draft_ids(config("crates/script/paths/289/druid.json")), ["druid"])
        for path in [
            "crates/script/paths/289/*.json", "crates/script/paths/289/priest.json",
            "crates/script/paths/289/index.json", "elsewhere/druid.json",
            "crates/script/paths/289/subdir/druid.json",
        ]:
            with self.subTest(path=path), self.assertRaises(ProofError):
                mapped_draft_ids(config(path))
        with self.assertRaises(ProofError):
            mapped_draft_ids({"cover": []})

    def test_index_rows_split_into_qualified_draft_and_unavailable(self) -> None:
        index = {"paths": [
            {"id": "cook", "status": "qualified"},
            {"id": "prince", "status": "draft"},
            {"id": "hauntedmine", "unavailable": "Not on this server"},
            {"id": "priest", "status": "qualified"},
        ]}
        self.assertEqual(
            index_lists(index), (["cook", "priest"], ["prince"], ["hauntedmine"])
        )
        for paths in [
            [{"id": "cook", "status": "qualified"}],
            [{"id": "priest", "status": "qualified"}, {"id": "priest", "status": "qualified"}],
            [{"id": "priest", "status": "draft"}],
            [{"id": "priest", "status": "qualified"}, {"id": "unmarked"}],
        ]:
            with self.subTest(paths=paths), self.assertRaises(ProofError):
                index_lists({"paths": paths})

    def test_probe_arguments_tag_every_row_and_fail_on_an_unlisted_one(self) -> None:
        index = {"paths": [
            {"id": "cook", "status": "qualified"},
            {"id": "prince", "status": "draft"},
            {"id": "hunt", "status": "draft"},
            {"id": "hauntedmine", "unavailable": "Not on this server"},
            {"id": "priest", "status": "qualified"},
        ]}
        self.assertEqual(
            probe_arguments(["prince", "hunt"], index),
            [
                "draft:hunt", "draft:prince", "unavailable:hauntedmine",
                "qualified:cook", "qualified:priest",
            ],
        )
        # An index draft that the refusal map omits is in neither list.
        with self.assertRaisesRegex(ProofError, "hunt"):
            probe_arguments(["prince"], index)
        # A qualified guide, or an id the index lacks, must not stay in the map.
        with self.assertRaisesRegex(ProofError, "cook"):
            probe_arguments(["prince", "hunt", "cook"], index)
        with self.assertRaisesRegex(ProofError, "ghost"):
            probe_arguments(["prince", "hunt", "ghost"], index)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(ReleaseRefusalTests)
        return 0 if unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful() else 1
    try:
        return prove_release_refusal()
    except (ProofError, OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"release-refusal: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
