"""Behavioral tests for the exact source-path ownership map.

Covers: missing source ownership, stale rows, owner-transfer rejection,
new valid sources with exact rows, strict schema (duplicate paths, unknown
fields, unknown owners, path escapes/wildcards), and the seat-model rule
transition gate (Main decision 2026-10-12 F0-CHECKER).
"""

from __future__ import annotations

import json
import importlib.util
import os
import shutil
import sys
from pathlib import Path

CHECKER_ROOT = Path(os.environ.get("F0_ARCH_TEST_CHECKER_ROOT", Path(__file__).resolve().parents[2]))
_spec = importlib.util.spec_from_file_location("architecture_ownership_under_test", CHECKER_ROOT / "tools/architecture/ownership.py")
_subject = importlib.util.module_from_spec(_spec)
sys.modules[_spec.name] = _subject
_spec.loader.exec_module(_subject)
SEAT_MODEL_RULE_PREFIX = _subject.SEAT_MODEL_RULE_PREFIX
check_transition = _subject.check_transition
check_tree = _subject.check_tree

CONTRACT = "test"


def write_map(root: Path, entries: list[tuple[str, str]]) -> None:
    dest = root / "tools" / "architecture" / "ownership.json"
    dest.parent.mkdir(parents=True, exist_ok=True)
    data = {
        "version": 1,
        "contract": CONTRACT,
        "paths": [{"path": p, "owner": o} for p, o in entries],
    }
    dest.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")


def write_raw_map(root: Path, text: str) -> None:
    dest = root / "tools" / "architecture" / "ownership.json"
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_text(text, encoding="utf-8")


def make_source(root: Path, rel: str, content: str = "test\n") -> None:
    full = root / rel
    full.parent.mkdir(parents=True, exist_ok=True)
    full.write_text(content, encoding="utf-8")


def copy_tree(src: Path, dst: Path) -> None:
    shutil.copytree(src, dst)


def test_missing_source_ownership() -> None:
    import tempfile

    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        make_source(root, "crates/host-play/src/play_login.rs", "a\n")
        make_source(root, "crates/host-play/src/play_login_tests.rs", "b\n")
        write_map(root, [("crates/host-play/src/play_login.rs", "T1")])
        errors = check_tree(root)
        assert any(
            "missing ownership" in e and "play_login_tests.rs" in e for e in errors
        ), errors
        assert not any("stale ownership" in e for e in errors), errors


def test_stale_row() -> None:
    import tempfile

    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        make_source(root, "crates/api/build.rs", "a\n")
        write_map(
            root,
            [
                ("crates/api/build.rs", "T2"),
                ("crates/api/src/gone.rs", "T2"),
            ],
        )
        errors = check_tree(root)
        assert any(
            "stale ownership" in e and "gone.rs" in e for e in errors
        ), errors


def test_owner_transfer_rejected() -> None:
    import tempfile

    with tempfile.TemporaryDirectory() as td:
        base = Path(td) / "base"
        cand = Path(td) / "cand"
        make_source(base, "crates/script/schema/generated/isolate_generated.rs", "a\n")
        write_map(
            base, [("crates/script/schema/generated/isolate_generated.rs", "T2")]
        )
        copy_tree(base, cand)
        write_map(
            cand, [("crates/script/schema/generated/isolate_generated.rs", "T1")]
        )
        errors = check_transition(base, cand)
        assert any(
            "owner transfer" in e and "isolate_generated.rs" in e for e in errors
        ), errors


def test_new_valid_source_permitted() -> None:
    import tempfile

    with tempfile.TemporaryDirectory() as td:
        base = Path(td) / "base"
        cand = Path(td) / "cand"
        make_source(base, "crates/host-play/src/play_login.rs", "a\n")
        write_map(base, [("crates/host-play/src/play_login.rs", "T1")])
        copy_tree(base, cand)
        make_source(cand, "crates/host-play/src/play_login_tests.rs", "b\n")
        write_map(
            cand,
            [
                ("crates/host-play/src/play_login.rs", "T1"),
                ("crates/host-play/src/play_login_tests.rs", "T1"),
            ],
        )
        errors = check_transition(base, cand)
        assert errors == [], errors


def test_duplicate_path_rejected() -> None:
    import tempfile

    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        make_source(root, "crates/api/build.rs", "a\n")
        write_raw_map(
            root,
            json.dumps(
                {
                    "version": 1,
                    "contract": CONTRACT,
                    "paths": [
                        {"path": "crates/api/build.rs", "owner": "T2"},
                        {"path": "crates/api/build.rs", "owner": "T2"},
                    ],
                }
            ),
        )
        errors = check_tree(root)
        assert any("duplicate path" in e for e in errors), errors


def test_unknown_field_rejected() -> None:
    import tempfile

    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        make_source(root, "crates/api/build.rs", "a\n")
        write_raw_map(
            root,
            json.dumps(
                {
                    "version": 1,
                    "contract": CONTRACT,
                    "extra": True,
                    "paths": [
                        {
                            "path": "crates/api/build.rs",
                            "owner": "T2",
                            "reason": "not allowed",
                        }
                    ],
                }
            ),
        )
        errors = check_tree(root)
        assert any("unknown field" in e for e in errors), errors


def test_unknown_owner_rejected() -> None:
    import tempfile

    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        make_source(root, "crates/api/build.rs", "a\n")
        write_map(root, [("crates/api/build.rs", "T9")])
        errors = check_tree(root)
        assert any("unknown owner" in e for e in errors), errors


def test_path_escape_and_wildcard_rejected() -> None:
    import tempfile

    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        make_source(root, "crates/api/build.rs", "a\n")
        write_raw_map(
            root,
            json.dumps(
                {
                    "version": 1,
                    "contract": CONTRACT,
                    "paths": [
                        {"path": "../outside.rs", "owner": "T1"},
                        {"path": "crates/api/*.rs", "owner": "T2"},
                    ],
                }
            ),
        )
        errors = check_tree(root)
        assert any("invalid path" in e for e in errors), errors
        assert len([e for e in errors if "invalid path" in e]) == 2, errors


def test_seat_model_rule_change_requires_delta() -> None:
    import tempfile

    with tempfile.TemporaryDirectory() as td:
        base = Path(td) / "base"
        cand = Path(td) / "cand"
        rule = SEAT_MODEL_RULE_PREFIX + "monitor.rs"
        make_source(base, rule, "rules v1\n")
        write_map(base, [(rule, "T1")])
        copy_tree(base, cand)
        (cand / rule).write_text("rules v2\n", encoding="utf-8")
        errors = check_transition(base, cand)
        assert any(
            "seat-model rule change requires approved contract delta" in e
            and "monitor.rs" in e
            for e in errors
        ), errors


def test_seat_model_new_test_permitted() -> None:
    import tempfile

    with tempfile.TemporaryDirectory() as td:
        base = Path(td) / "base"
        cand = Path(td) / "cand"
        rule = SEAT_MODEL_RULE_PREFIX + "monitor.rs"
        new_test = "tools/seat-model/tests/extra_fixture.rs"
        make_source(base, rule, "rules v1\n")
        write_map(base, [(rule, "T1")])
        copy_tree(base, cand)
        make_source(cand, new_test, "fixture\n")
        write_map(cand, [(rule, "T1"), (new_test, "T1")])
        errors = check_transition(base, cand)
        assert errors == [], errors


def test_clean_tree() -> None:
    import tempfile

    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        make_source(root, "crates/api/build.rs", "a\n")
        write_map(root, [("crates/api/build.rs", "T2")])
        assert check_tree(root) == []


def test_duplicate_object_key_rejected() -> None:
    import tempfile

    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        write_raw_map(root, '{"version":1,"version":1,"contract":"test","paths":[]}')
        assert any("duplicate" in e for e in check_tree(root))
        write_map(root, [])
        assert check_tree(root) == []


def test_source_symlinks_rejected() -> None:
    import tempfile

    with tempfile.TemporaryDirectory() as td:
        root = Path(td) / "root"
        outside = Path(td) / "outside"
        make_source(outside, "hidden.rs")
        root.mkdir()
        write_map(root, [])
        (root / "linked").symlink_to(outside, target_is_directory=True)
        assert any("source symlink" in e for e in check_tree(root))
        (root / "linked").unlink()
        (root / "linked.rs").symlink_to(outside / "hidden.rs")
        assert any("source symlink" in e for e in check_tree(root))
        (root / "linked.rs").unlink()
        assert check_tree(root) == []


def test_transition_rejects_stale_candidate_row() -> None:
    import tempfile

    with tempfile.TemporaryDirectory() as td:
        base, candidate = Path(td) / "base", Path(td) / "candidate"
        write_map(base, [])
        write_map(candidate, [("gone.rs", "T1")])
        assert any("stale ownership" in e for e in check_transition(base, candidate))
        write_map(candidate, [])
        assert check_transition(base, candidate) == []


def load_tests(loader, tests, pattern):
    """Include the ownership behaviors in the protected unittest corpus."""
    import unittest

    return unittest.TestSuite(
        unittest.FunctionTestCase(value)
        for name, value in sorted(globals().items())
        if name.startswith("test_") and callable(value)
    )
