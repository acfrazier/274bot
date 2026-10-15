#!/usr/bin/env python3
"""Lint-suppression ratchet: fail when `#[allow]`/`#[expect]` counts grow.

Scans `crates/**/*.rs` for `#[allow(...)]`, `#[expect(...)]`,
`#![allow(...)]`, `#![expect(...)]` and `#[cfg_attr(.., allow(..))]` /
`#[cfg_attr(.., expect(..))]` suppressions, bucketed per crate x lint with
test files counted separately, and compares against `baseline.json`.

  python3 -B tools/lint-ratchet/lint_ratchet.py            # check, exit 1 on growth
  python3 -B tools/lint-ratchet/lint_ratchet.py --update   # rebaseline, exit 0

Rules: any (crate, lint, bucket) count above its baseline fails, as does a
suppression for a lint absent from that crate's baseline entry. Lowering a
count (including removing suppressions) is free and needs no baseline edit.
`--update` rewrites the baseline; land the rewritten file in the same diff
as the suppression change. Failure output names every offending file:line.

Counting is lexical, not cfg-evaluated: a `cfg_attr(not(test), allow(..))`
counts once wherever it is written, and macros that expand to suppressions
are invisible (only literally written attributes count). A "test" file is
one with a `tests`/`test` path component or `test` in the file stem
(`*_tests.rs`, `tests.rs`, `test_support.rs`, ...). Only `crates/` is
scanned; `tools/` members are out of scope.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
DEFAULT_BASELINE = SCRIPT_DIR / "baseline.json"

# FlatBuffers compiler output. `flatc --rust` writes its own `#[allow(...)]`
# lines, so regenerating the bindings would otherwise trip the ratchet.
# Listed explicitly (not by glob) so a new generated file is a conscious edit.
EXEMPT_FILES = frozenset(
    {
        "crates/script/schema/generated/isolate_generated.rs",
    }
)

BASELINE_VERSION = 1
NON_TEST = "non_test"
TEST = "test"

LINT_RE = re.compile(r"[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)*\Z")
REASON_RE = re.compile(r"reason\s*=")


def find_workspace_root() -> Path:
    """Walk up from this script to the checkout root (the dir holding tools/)."""
    here = SCRIPT_DIR.resolve()
    for parent in (here, *here.parents):
        if (parent / "tools").is_dir() and (parent / "Cargo.toml").is_file():
            return parent
    raise SystemExit("lint-ratchet: cannot locate workspace root above " + str(SCRIPT_DIR))


def blank_comments_and_strings(src: str) -> str:
    """Return `src` with comments/string literals replaced by spaces.

    Newlines are preserved so offsets still map to the original line numbers.
    Handles `//`, `/* */` (nested, as rustc parses them), `"..."` (escapes),
    `r#"..."#` raw strings, `b".."`, `br#".."#, and `'c'` char literals.
    A `'` starts a char literal only for `'x'` / `'\\..'`; otherwise it is a
    lifetime and left alone.
    """
    out = list(src)
    n = len(src)
    i = 0

    def blank(start: int, end: int) -> None:
        for j in range(start, end):
            if out[j] != "\n":
                out[j] = " "

    while i < n:
        c = src[i]
        nxt = src[i + 1] if i + 1 < n else ""
        if c == "/" and nxt == "/":
            j = src.find("\n", i)
            if j == -1:
                j = n
            blank(i, j)
            i = j
        elif c == "/" and nxt == "*":
            depth = 1
            j = i + 2
            while j < n and depth > 0:
                if src[j] == "/" and j + 1 < n and src[j + 1] == "*":
                    depth += 1
                    j += 2
                elif src[j] == "*" and j + 1 < n and src[j + 1] == "/":
                    depth -= 1
                    j += 2
                else:
                    j += 1
            blank(i, j)
            i = j
        elif c == "r" and (nxt == '"' or nxt == "#"):
            # Raw string r"..." / r#"..."#. Byte raw strings (br#") are
            # handled via the 'b' branch below, which delegates here.
            j = i + 1
            hashes = 0
            while j < n and src[j] == "#":
                hashes += 1
                j += 1
            if j < n and src[j] == '"':
                j += 1
                closer = '"' + "#" * hashes
                k = src.find(closer, j)
                end = n if k == -1 else k + len(closer)
                blank(i, end)
                i = end
            else:
                i += 1
        elif c == "b" and nxt in ("\"", "'", "r"):
            if nxt == "r":
                # br".." / br#".."#: same shape as a raw string starting at i+1.
                j = i + 2
                hashes = 0
                while j < n and src[j] == "#":
                    hashes += 1
                    j += 1
                if j < n and src[j] == '"':
                    j += 1
                    closer = '"' + "#" * hashes
                    k = src.find(closer, j)
                    end = n if k == -1 else k + len(closer)
                    blank(i, end)
                    i = end
                else:
                    i += 1
            elif nxt == '"':
                i = _blank_cooked(src, out, blank, i + 1, n)
            else:  # b'x'
                if i + 3 < n and src[i + 3] == "'":
                    blank(i, i + 4)
                    i += 4
                elif i + 2 < n and src[i + 1] == "\\":
                    k = src.find("'", i + 2)
                    end = n if k == -1 else k + 1
                    blank(i, end)
                    i = end
                else:
                    i += 1
        elif c == '"':
            i = _blank_cooked(src, out, blank, i, n)
        elif c == "'":
            # Char literal 'x' / '\n' / '\''; otherwise a lifetime, skip.
            if i + 2 < n and src[i + 2] == "'":
                blank(i, i + 3)
                i += 3
            elif i + 1 < n and src[i + 1] == "\\":
                k = src.find("'", i + 2)
                end = n if k == -1 else k + 1
                blank(i, end)
                i = end
            else:
                i += 1
        else:
            i += 1
    return "".join(out)


def _blank_cooked(src: str, out: list[str], blank, start: int, n: int) -> int:
    """Blank a `"..."` literal starting at the quote `start`; return resume index."""
    j = start + 1
    while j < n:
        if src[j] == "\\":
            j += 2
        elif src[j] == '"':
            blank(start, j + 1)
            return j + 1
        elif src[j] == "\n":
            # Unterminated (invalid Rust); stop so the newline survives.
            blank(start, j)
            return j
        else:
            j += 1
    blank(start, n)
    return n


def match_balanced(text: str, opener_index: int) -> int:
    """Return the index just past the bracket matching `text[opener_index]`.

    `opener_index` points at one of `([{`; nesting of all three kinds is
    tracked. Raises ValueError if unbalanced.
    """
    pairs = {"(": ")", "[": "]", "{": "}"}
    closer_to_opener = {v: k for k, v in pairs.items()}
    stack = [text[opener_index]]
    i = opener_index + 1
    while i < len(text):
        c = text[i]
        if c in pairs:
            stack.append(c)
        elif c in closer_to_opener:
            if not stack or stack[-1] != closer_to_opener[c]:
                raise ValueError(f"unbalanced {c!r} at offset {i}")
            stack.pop()
            if not stack:
                return i + 1
        i += 1
    raise ValueError("unbalanced attribute")


def split_top_level(text: str) -> list[str]:
    """Split on commas that are not nested inside parens/brackets/braces."""
    parts: list[str] = []
    depth = 0
    start = 0
    for i, c in enumerate(text):
        if c in "([{":
            depth += 1
        elif c in ")]}":
            depth = max(0, depth - 1)
        elif c == "," and depth == 0:
            parts.append(text[start:i])
            start = i + 1
    parts.append(text[start:])
    return parts


def lints_in_allow_or_expect(args: str, problems: list[str], where: str) -> list[str]:
    """Extract lint paths from an `allow(...)` / `expect(...)` argument list."""
    found: list[str] = []
    for part in split_top_level(args):
        item = part.strip()
        if not item or REASON_RE.match(item):
            continue
        if LINT_RE.match(item):
            found.append(item)
        else:
            problems.append(f"{where}: cannot parse suppression item {item!r}; skipped")
    return found


def lints_in_attribute(inner: str, problems: list[str], where: str) -> list[str]:
    """Extract suppressed lints from one `[...]` attribute body (comments/strings blanked)."""
    body = inner.strip()
    for keyword in ("allow", "expect"):
        if body == keyword or body.startswith(keyword + "(") or body.startswith(keyword + " "):
            rest = body[len(keyword) :].strip()
            if rest.startswith("("):
                try:
                    end = match_balanced(rest, 0)
                except ValueError:
                    problems.append(f"{where}: unbalanced {keyword}(...); skipped")
                    return []
                return lints_in_allow_or_expect(rest[1 : end - 1], problems, where)
            return []
    if body == "cfg_attr" or body.startswith("cfg_attr(") or body.startswith("cfg_attr "):
        rest = body[len("cfg_attr") :].strip()
        if not rest.startswith("("):
            return []
        try:
            end = match_balanced(rest, 0)
        except ValueError:
            problems.append(f"{where}: unbalanced cfg_attr(...); skipped")
            return []
        items = split_top_level(rest[1 : end - 1])
        found: list[str] = []
        for item in items[1:]:  # items[0] is the cfg predicate
            item = item.strip()
            for keyword in ("allow", "expect"):
                if item == keyword or item.startswith(keyword + "("):
                    arg = item[len(keyword) :].strip()
                    if arg.startswith("("):
                        try:
                            arg_end = match_balanced(arg, 0)
                        except ValueError:
                            problems.append(f"{where}: unbalanced {keyword}(...) in cfg_attr; skipped")
                            break
                        found.extend(lints_in_allow_or_expect(arg[1 : arg_end - 1], problems, where))
                    break
        return found
    return []


def is_test_file(rel: Path) -> bool:
    parts = rel.parts
    if "tests" in parts or "test" in parts:
        return True
    return "test" in rel.stem


def scan_file(root: Path, path: Path, problems: list[str]) -> list[tuple[str, str, str, int]]:
    """Scan one file; return (crate, lint, bucket, line) occurrences."""
    rel = path.relative_to(root)
    try:
        crate = rel.parts[1] if rel.parts[0] == "crates" else "__outside__"
    except IndexError:
        crate = "__outside__"
    bucket = TEST if is_test_file(rel) else NON_TEST
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as exc:
        problems.append(f"{rel}: cannot read ({exc}); skipped")
        return []
    clean = blank_comments_and_strings(text)
    occurrences: list[tuple[str, str, str, int]] = []
    # Multi-line attributes are one match: `re.DOTALL` is unnecessary because
    # the bracket matcher walks the raw text; the regex only finds the `#`.
    for match in re.finditer(r"#(!?)\[", clean):
        open_index = match.end() - 1
        try:
            end = match_balanced(clean, open_index)
        except ValueError:
            line = clean.count("\n", 0, match.start()) + 1
            problems.append(f"{rel}:{line}: unbalanced attribute; skipped")
            continue
        inner = clean[open_index + 1 : end - 1]
        line = clean.count("\n", 0, match.start()) + 1
        where = f"{rel}:{line}"
        for lint in lints_in_attribute(inner, problems, where):
            occurrences.append((crate, lint, bucket, line))
    return occurrences


def scan_tree(root: Path, problems: list[str]) -> list[tuple[str, str, str, str, int]]:
    """Scan `crates/**/*.rs`; return (crate, lint, bucket, file, line) hits."""
    hits: list[tuple[str, str, str, str, int]] = []
    for path in sorted((root / "crates").rglob("*.rs")):
        rel = str(path.relative_to(root))
        if rel in EXEMPT_FILES:
            continue
        for crate, lint, bucket, line in scan_file(root, path, problems):
            hits.append((crate, lint, bucket, rel, line))
    return hits


def summarize(hits) -> dict[str, dict[str, dict[str, int]]]:
    counts: dict[str, dict[str, dict[str, int]]] = {}
    for crate, lint, bucket, _rel, _line in hits:
        counts.setdefault(crate, {}).setdefault(lint, {NON_TEST: 0, TEST: 0})
        counts[crate][lint][bucket] += 1
    return counts


def load_baseline(path: Path) -> dict:
    try:
        with open(path, encoding="utf-8") as handle:
            data = json.load(handle)
    except FileNotFoundError:
        raise SystemExit(f"lint-ratchet: no baseline at {path}; run with --update to create it")
    if not isinstance(data, dict) or data.get("version") != BASELINE_VERSION:
        raise SystemExit(f"lint-ratchet: {path} is not a v{BASELINE_VERSION} baseline; re-run with --update")
    counts = data.get("counts")
    if not isinstance(counts, dict):
        raise SystemExit(f"lint-ratchet: {path} has no counts table; re-run with --update")
    return counts


def check(counts, baseline, hits) -> list[str]:
    """Return failure lines (each names file:line); empty means pass."""
    by_key: dict[tuple[str, str, str], list[str]] = {}
    for crate, lint, bucket, rel, line in hits:
        by_key.setdefault((crate, lint, bucket), []).append(f"{rel}:{line}")
    failures: list[str] = []
    for (crate, lint, bucket), locations in sorted(by_key.items()):
        current = len(locations)
        old = baseline.get(crate, {}).get(lint, {}).get(bucket, 0)
        if current <= old:
            continue
        if old == 0 and lint not in baseline.get(crate, {}):
            failures.append(
                f"[{crate}] {lint} [{bucket}]: new lint not in baseline ({current} occurrence(s))"
            )
        else:
            failures.append(f"[{crate}] {lint} [{bucket}]: {current} > baseline {old}")
        failures.extend(f"  {loc}" for loc in locations)
    return failures


def cmd_check(root: Path, baseline_path: Path) -> int:
    problems: list[str] = []
    hits = scan_tree(root, problems)
    counts = summarize(hits)
    baseline = load_baseline(baseline_path)
    failures = check(counts, baseline, hits)
    for warning in problems:
        print(f"lint-ratchet: warning: {warning}", file=sys.stderr)
    total = len(hits)
    crates = len(counts)
    if failures:
        print(f"lint-ratchet: FAIL ({len([f for f in failures if not f.startswith('  ')])} problem(s))")
        for line in failures:
            print(line)
        print(f"lint-ratchet: {total} suppression(s) across {crates} crate(s)")
        return 1
    print(f"lint-ratchet: OK ({total} suppression(s) across {crates} crate(s))")
    return 0


def cmd_update(root: Path, baseline_path: Path) -> int:
    problems: list[str] = []
    hits = scan_tree(root, problems)
    counts = summarize(hits)
    old_counts = load_baseline(baseline_path) if baseline_path.is_file() else {}
    for warning in problems:
        print(f"lint-ratchet: warning: {warning}", file=sys.stderr)
    for crate in sorted(set(old_counts) | set(counts)):
        for lint in sorted(set(old_counts.get(crate, {})) | set(counts.get(crate, {}))):
            for bucket in (NON_TEST, TEST):
                old = old_counts.get(crate, {}).get(lint, {}).get(bucket, 0)
                new = counts.get(crate, {}).get(lint, {}).get(bucket, 0)
                if old != new:
                    print(f"lint-ratchet: [{crate}] {lint} [{bucket}]: {old} -> {new}")
    ordered = {
        crate: {
            lint: counts[crate][lint]
            for lint in sorted(counts[crate])
            if any(counts[crate][lint].values())
        }
        for crate in sorted(counts)
        if any(any(buckets.values()) for buckets in counts[crate].values())
    }
    with open(baseline_path, "w", encoding="utf-8") as handle:
        json.dump({"version": BASELINE_VERSION, "counts": ordered}, handle, indent=2)
        handle.write("\n")
    print(f"lint-ratchet: wrote {baseline_path} ({len(hits)} suppression(s))")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="Fail when lint suppressions grow above the baseline.")
    parser.add_argument("--root", type=Path, default=None, help="workspace checkout root")
    parser.add_argument("--baseline", type=Path, default=None, help="baseline file")
    parser.add_argument("--update", action="store_true", help="rewrite the baseline")
    args = parser.parse_args(argv)
    root = args.root.resolve() if args.root else find_workspace_root()
    baseline_path = args.baseline.resolve() if args.baseline else DEFAULT_BASELINE
    if not (root / "crates").is_dir():
        raise SystemExit(f"lint-ratchet: no crates/ under {root}")
    if args.update:
        return cmd_update(root, baseline_path)
    return cmd_check(root, baseline_path)


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
