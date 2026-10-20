#!/usr/bin/env python3
"""Single source of truth for open release notes: known issues and gaps.

The ledger lives in ``docs/known-issues.toml`` (one ``[[item]]`` table per
open item). Fixed items leave the file in the commit that fixes them. Never
edit the ``### Known issues`` / ``### Not in this release`` blocks in
``CHANGELOG.md`` by hand; regenerate them instead:

    python3 tools/known-issues/known_issues.py render --release 0.2.5
    python3 tools/known-issues/known_issues.py check --release 0.2.5
    python3 tools/known-issues/known_issues.py carried --release 0.2.5

``render`` prints both blocks in CHANGELOG style (80-column wrap,
``- **Area:** text`` bullets for known issues). ``check`` diffs those blocks
against ``CHANGELOG.md`` and fails when they differ. ``carried`` lists every
open item with its carry count and flags items that need a release ruling.
"""

import argparse
import difflib
import re
import sys
import textwrap
from pathlib import Path

KNOWN_ISSUE = "known-issue"
NOT_IN_RELEASE = "not-in-release"
KINDS = (NOT_IN_RELEASE, KNOWN_ISSUE)
SEVERITIES = ("critical", "high", "medium", "low")
WIDTH = 80

SECTION_RE = re.compile(r"^## \[([^\]]+)\]")
SUBSECTION_RE = re.compile(r"^###\s+(.*?)\s*$")
UNRELEASED_HEADING = "## Unreleased"
NOT_IN_RELEASE_HEADING = "### Not in this release"
KNOWN_ISSUES_HEADING = "### Known issues"


def repo_root():
    return Path(__file__).resolve().parents[2]


def version_key(version):
    """Numeric release key: ``0.2.0.1.1`` sorts before ``0.2.1``."""
    parts = []
    for chunk in str(version).split("."):
        match = re.match(r"(\d+)", chunk)
        if not match:
            raise ValueError(f"bad release name: {version!r}")
        parts.append(int(match.group(1)))
    return tuple(parts)


def load_ledger(path):
    """Read and validate the ledger; return items in file order."""
    import tomllib

    with open(path, "rb") as handle:
        data = tomllib.load(handle)
    raw = data.get("item", [])
    if not isinstance(raw, list):
        raise ValueError(f"{path}: expected a list of [[item]] tables")
    items = []
    seen = set()
    required = ("id", "kind", "area", "text", "first", "target",
                "severity", "owner")
    for entry in raw:
        for key in required:
            if key not in entry or str(entry[key]).strip() == "":
                raise ValueError(f"{path}: item is missing {key!r}: {entry!r}")
        if entry["kind"] not in KINDS:
            raise ValueError(f"{path}: bad kind {entry['kind']!r}")
        if entry["severity"] not in SEVERITIES:
            raise ValueError(f"{path}: bad severity {entry['severity']!r}")
        if entry["id"] in seen:
            raise ValueError(f"{path}: duplicate id {entry['id']!r}")
        seen.add(entry["id"])
        version_key(entry["first"])
        if entry["target"] != "none":
            version_key(entry["target"])
        items.append({
            "id": str(entry["id"]),
            "kind": str(entry["kind"]),
            "area": " ".join(str(entry["area"]).split()),
            "text": " ".join(str(entry["text"]).split()),
            "first": str(entry["first"]),
            "target": str(entry["target"]),
            "severity": str(entry["severity"]),
            "owner": str(entry["owner"]),
        })
    return items


def changelog_releases(changelog_path):
    """Release names in chronological order (oldest first)."""
    order = []
    with open(changelog_path, encoding="utf-8") as handle:
        for line in handle:
            match = SECTION_RE.match(line.rstrip("\n"))
            if match:
                order.append(match.group(1))
    order.reverse()
    return order


def with_release(order, release):
    """Order plus ``release``; a release newer than every heading goes last."""
    names = list(order)
    if release not in names:
        names.append(release)
    return names


def applies(item, release, order):
    """Whether the item affects ``release`` (its ``first`` is not newer).

    A release with no CHANGELOG heading yet (the next release) counts as
    newer than every heading."""
    if item["first"] not in order:
        raise ValueError(
            f"ledger item {item['id']!r} names an unknown release {item['first']!r}")
    names = with_release(order, release)
    return names.index(item["first"]) <= names.index(release)


def carried_releases(first, release, order):
    """Releases that carry the item into ``release``: ``first`` up to (not
    including) ``release``."""
    names = with_release(order, release)
    if first not in order:
        raise ValueError(f"unknown release {first!r}")
    return names[names.index(first):names.index(release)]


def carried_sentence(first, release, order):
    """``0.2.3 has the same problem.`` / ``0.2.0.2 to 0.2.3 have ...``."""
    carried = carried_releases(first, release, order)
    if not carried:
        return ""
    if len(carried) == 1:
        return f"{carried[0]} has the same problem."
    names = with_release(order, release)
    positions = [names.index(name) for name in carried]
    if positions == list(range(positions[0], positions[0] + len(positions))):
        return f"{carried[0]} to {carried[-1]} have the same problem."
    if len(carried) == 2:
        return f"{carried[0]} and {carried[1]} have the same problem."
    return f"{', '.join(carried[:-1])} and {carried[-1]} have the same problem."


def wrap_bullet(body):
    wrapper = textwrap.TextWrapper(
        width=WIDTH,
        initial_indent="- ",
        subsequent_indent="  ",
        break_long_words=False,
        break_on_hyphens=False,
    )
    return wrapper.fill(body)


def render_release(items, order, release):
    """Render both CHANGELOG blocks for ``release``; skip empty blocks."""
    blocks = []
    for kind, heading in ((NOT_IN_RELEASE, NOT_IN_RELEASE_HEADING),
                          (KNOWN_ISSUE, KNOWN_ISSUES_HEADING)):
        bullets = []
        for item in items:
            if item["kind"] != kind or not applies(item, release, order):
                continue
            body = item["text"]
            if kind == KNOWN_ISSUE:
                body = f"**{item['area']}:** {body}"
            sentence = carried_sentence(item["first"], release, order)
            if sentence:
                body = f"{body} {sentence}"
            bullets.append(wrap_bullet(body))
        if bullets:
            blocks.append(f"{heading}\n\n" + "\n".join(bullets) + "\n")
    return "\n".join(blocks)


def section_lines(changelog_path, release=None, unreleased=False):
    """Raw lines of one ``##`` section (heading line excluded).

    Release headings carry a date suffix (``## [0.2.4] — ...``), so match the
    bracketed name instead of the whole line."""
    with open(changelog_path, encoding="utf-8") as handle:
        lines = handle.read().splitlines()
    start = None
    for index, line in enumerate(lines):
        stripped = line.strip()
        if unreleased:
            if stripped == UNRELEASED_HEADING:
                start = index + 1
                break
        else:
            match = SECTION_RE.match(stripped)
            if match and match.group(1) == release:
                start = index + 1
                break
    if start is None:
        return None
    body = []
    for line in lines[start:]:
        if line.startswith("## "):
            break
        body.append(line)
    return body


def extract_blocks(section_body):
    """Map block name -> normalized bullet text for the two known blocks."""
    found = {}
    current = None
    for line in section_body:
        if line.startswith("### "):
            match = SUBSECTION_RE.match(line)
            name = match.group(1) if match else ""
            if name in ("Not in this release", "Known issues"):
                current = name
                found[current] = []
            else:
                current = None
            continue
        if current is not None:
            found[current].append(line.rstrip())
    return {name: "\n".join(text).strip("\n") for name, text in found.items()}


def check_release(items, order, release, section_body):
    """Diff expected vs actual blocks; return diff lines (empty means pass)."""
    if section_body is None:
        return ["CHANGELOG has no section for this release"]
    actual = extract_blocks(section_body)
    expected = extract_blocks(render_release(items, order, release).splitlines())
    diff = []
    for name in ("Not in this release", "Known issues"):
        want = expected.get(name)
        got = actual.get(name)
        if want is None and got is None:
            continue
        if want is None:
            diff.append(f"CHANGELOG has an unexpected '### {name}' block:")
            diff.extend(f"+{line}" for line in got.splitlines())
            continue
        if got is None:
            diff.append(f"CHANGELOG is missing the '### {name}' block; render gives:")
            diff.extend(f"-{line}" for line in want.splitlines())
            continue
        if want != got:
            diff.extend(difflib.unified_diff(
                got.splitlines(), want.splitlines(),
                fromfile=f"CHANGELOG ({name})",
                tofile=f"render --release {release} ({name})",
                lineterm="",
            ))
    return diff


def needs_ruling(item, release, order):
    """An item needs a ruling when ``release`` reaches its target, or it has
    been carried through two or more earlier releases."""
    if item["target"] != "none" and version_key(release) >= version_key(item["target"]):
        return True
    return len(carried_releases(item["first"], release, order)) >= 2


def carried_rows(items, order, release):
    """One row per item: id, first, carry count, target, owner, ruling flag."""
    rows = []
    for item in items:
        rows.append({
            "id": item["id"],
            "first": item["first"],
            "carried": len(carried_releases(item["first"], release, order)),
            "target": item["target"],
            "owner": item["owner"],
            "ruling": needs_ruling(item, release, order),
        })
    rows.sort(key=lambda row: (
        order.index(row["first"]) if row["first"] in order else len(order),
        row["id"],
    ))
    return rows


def format_carried(rows):
    widths = {
        "id": max([len("id")] + [len(row["id"]) for row in rows]),
        "first": max([len("first")] + [len(row["first"]) for row in rows]),
        "target": max([len("target")] + [len(row["target"]) for row in rows]),
        "owner": max([len("owner")] + [len(row["owner"]) for row in rows]),
    }
    header = (
        f"{'id':<{widths['id']}}  {'first':<{widths['first']}}  "
        f"{'carried':>7}  {'target':<{widths['target']}}  "
        f"{'owner':<{widths['owner']}}  ruling"
    )
    lines = [header]
    for row in rows:
        lines.append(
            f"{row['id']:<{widths['id']}}  {row['first']:<{widths['first']}}  "
            f"{row['carried']:>7}  "
            f"{row['target']:<{widths['target']}}  "
            f"{row['owner']:<{widths['owner']}}  "
            f"{'needs ruling' if row['ruling'] else ''}".rstrip()
        )
    return "\n".join(lines) + "\n"


def default_paths(args):
    root = repo_root()
    ledger = args.ledger or root / "docs" / "known-issues.toml"
    changelog = args.changelog or root / "CHANGELOG.md"
    return ledger, changelog


def cmd_render(args):
    ledger_path, changelog_path = default_paths(args)
    items = load_ledger(ledger_path)
    order = changelog_releases(changelog_path)
    sys.stdout.write(render_release(items, order, args.release))


def cmd_check(args):
    ledger_path, changelog_path = default_paths(args)
    items = load_ledger(ledger_path)
    order = changelog_releases(changelog_path)
    if args.unreleased:
        label = UNRELEASED_HEADING
        section = section_lines(changelog_path, unreleased=True)
    else:
        label = f"## [{args.release}]"
        section = section_lines(changelog_path, release=args.release)
    diff = check_release(items, order, args.release, section)
    if diff:
        sys.stdout.write("\n".join(diff) + "\n")
        return 1
    print(f"known issues match render for {label}")
    return 0


def cmd_carried(args):
    ledger_path, changelog_path = default_paths(args)
    items = load_ledger(ledger_path)
    order = changelog_releases(changelog_path)
    sys.stdout.write(format_carried(carried_rows(items, order, args.release)))


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--ledger", type=Path,
                        help="ledger file (default docs/known-issues.toml)")
    result.add_argument("--changelog", type=Path,
                        help="CHANGELOG file (default CHANGELOG.md)")
    commands = result.add_subparsers(dest="command", required=True)
    render = commands.add_parser("render", help="print the CHANGELOG blocks for a release")
    render.add_argument("--release", required=True)
    render.set_defaults(func=cmd_render)
    check = commands.add_parser("check", help="diff the CHANGELOG blocks against render")
    target = check.add_mutually_exclusive_group(required=True)
    target.add_argument("--release")
    target.add_argument("--unreleased", action="store_true",
                        help="check the ## Unreleased section against the next release")
    check.set_defaults(func=cmd_check)
    carried = commands.add_parser("carried", help="table open items and their carry count")
    carried.add_argument("--release", required=True)
    carried.set_defaults(func=cmd_carried)
    return result


def main(argv=None):
    arguments = parser().parse_args(argv)
    if getattr(arguments, "unreleased", False):
        # --unreleased checks the staged next-release blocks. That release has
        # no heading yet, so name it after the newest heading. The name never
        # appears in output: carried sentences only name releases before it.
        _, changelog_path = default_paths(arguments)
        order = changelog_releases(changelog_path)
        arguments.release = f"{order[-1]}+unreleased" if order else "unreleased"
    try:
        outcome = arguments.func(arguments)
    except (OSError, ValueError) as error:
        parser().error(str(error))
    return outcome or 0


def check_release_by_paths(ledger_path, changelog_path, release):
    """Entry point for release.py preflight: diff lines, empty means pass."""
    items = load_ledger(ledger_path)
    order = changelog_releases(changelog_path)
    section = section_lines(changelog_path, release=release)
    return check_release(items, order, release, section)


if __name__ == "__main__":
    sys.exit(main())
