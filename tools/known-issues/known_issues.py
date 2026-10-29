#!/usr/bin/env python3
"""Single source of truth for open release notes: known issues and gaps.

The ledger lives in ``docs/known-issues.toml`` (one ``[[item]]`` table per
item). An item is open at release X when its ``listed`` release (default
``first``) is not newer than X and its ``fixed`` release (when set) is newer
than X. ``first`` is the first affected release, or the release under
development when no shipped release has the problem; ``listed`` differs only
when the item was found after ``first`` had shipped. The commit that
fixes an item sets ``fixed`` to the release carrying the fix; items are never
deleted, so checks of already published releases keep passing. For the same
reason, rewording an item that a published release lists keeps the old text in
an ``[[item.earlier]]`` table whose ``until`` is the first release with the new
wording. Never edit the ``### Known issues`` / ``### Not in this release``
blocks in ``CHANGELOG.md`` by hand; regenerate them instead:

    python3 tools/known-issues/known_issues.py render --release 0.2.8
    python3 tools/known-issues/known_issues.py check --release 0.2.8
    python3 tools/known-issues/known_issues.py carried --release 0.2.8

``render`` prints both blocks in CHANGELOG style (80-column wrap,
``- **Area:** text`` bullets for known issues). ``check`` diffs those blocks
against ``CHANGELOG.md`` and fails when they differ. ``carried`` lists every
open item with its carry count and flags items that need a release ruling.

The ``## Unreleased`` section stages no ledger blocks during development:
they are rendered with ``render --release <next>`` and pasted in when the
release section is cut, then verified with ``check --release <next>``. There
is no ``--unreleased`` flag: it used to fail whenever the blocks were not
staged yet, which is the normal state all through development.
"""

import argparse
import difflib
import re
import sys
import textwrap
from pathlib import Path

# Shared floor with tools/release/release.py: the ledger is TOML and is read
# with tomllib, which exists only on Python 3.11 and newer. Keep the two minima in sync.
REQUIRED_PYTHON = (3, 11)

if sys.version_info < REQUIRED_PYTHON:
    _running = ".".join(str(part) for part in sys.version_info[:3])
    sys.exit(
        f"known_issues.py requires Python 3.11 or newer "
        f"(running Python {_running} at {sys.executable})"
    )

KNOWN_ISSUE = "known-issue"
NOT_IN_RELEASE = "not-in-release"
KINDS = (NOT_IN_RELEASE, KNOWN_ISSUE)
SEVERITIES = ("critical", "high", "medium", "low")
WIDTH = 80

SECTION_RE = re.compile(r"^## \[([^\]]+)\]")
SUBSECTION_RE = re.compile(r"^###\s+(.*?)\s*$")
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


def newer_than(names, later, earlier):
    """True when release ``later`` comes after ``earlier``: by position in
    ``names`` when both are listed there, else by numeric release key."""
    if later in names and earlier in names:
        return names.index(later) > names.index(earlier)
    return version_key(later) > version_key(earlier)


def load_ledger(path, order):
    """Read and validate the ledger; return items in file order.

    ``order`` is the CHANGELOG release order (oldest first, from
    :func:`changelog_releases`). The optional ``listed`` (default ``first``)
    and ``fixed`` releases must name a known release or the next release.
    ``listed`` is not earlier than ``first`` and is earlier than ``fixed``;
    ``fixed`` is later than ``first``. So a typo cannot silently keep an item
    open or close it too early. Unknown fields are rejected for the same
    reason."""
    import tomllib

    with open(path, "rb") as handle:
        data = tomllib.load(handle)
    raw = data.get("item", [])
    if not isinstance(raw, list):
        raise ValueError(f"{path}: expected a list of [[item]] tables")
    names = list(order)
    following = next_release(names) if names else None
    if following is not None and following not in names:
        names.append(following)
    items = []
    seen = set()
    required = ("id", "kind", "area", "text", "first", "target",
                "severity", "owner")
    for entry in raw:
        for key in required:
            if key not in entry or str(entry[key]).strip() == "":
                raise ValueError(f"{path}: item is missing {key!r}: {entry!r}")
        unknown = sorted(set(entry) - set(required) - {"fixed", "listed", "earlier"})
        if unknown:
            raise ValueError(
                f"{path}: item {entry.get('id')!r} has unknown "
                f"field(s): {', '.join(unknown)}")
        if entry["kind"] not in KINDS:
            raise ValueError(f"{path}: bad kind {entry['kind']!r}")
        if entry["severity"] not in SEVERITIES:
            raise ValueError(f"{path}: bad severity {entry['severity']!r}")
        if entry["id"] in seen:
            raise ValueError(f"{path}: duplicate id {entry['id']!r}")
        seen.add(entry["id"])
        first = str(entry["first"])
        version_key(first)
        if first not in names:
            raise ValueError(
                f"{path}: item {entry['id']!r} names an unknown "
                f"'first' release {first!r}")
        if entry["target"] != "none":
            version_key(entry["target"])
        fixed = None
        if entry.get("fixed") is not None:
            fixed = str(entry["fixed"]).strip()
            if fixed == "":
                raise ValueError(
                    f"{path}: item {entry['id']!r} has an empty 'fixed' release")
            version_key(fixed)
            if fixed not in names:
                raise ValueError(
                    f"{path}: item {entry['id']!r} names an unknown "
                    f"'fixed' release {fixed!r}")
            if not newer_than(names, fixed, first):
                raise ValueError(
                    f"{path}: item {entry['id']!r} 'fixed' release {fixed!r} "
                    f"is not later than 'first' {first!r}")
        listed = first
        if entry.get("listed") is not None:
            listed = str(entry["listed"]).strip()
            if listed == "":
                raise ValueError(
                    f"{path}: item {entry['id']!r} has an empty 'listed' release")
            version_key(listed)
            if listed not in names:
                raise ValueError(
                    f"{path}: item {entry['id']!r} names an unknown "
                    f"'listed' release {listed!r}")
            if newer_than(names, first, listed):
                raise ValueError(
                    f"{path}: item {entry['id']!r} 'listed' release {listed!r} "
                    f"is earlier than 'first' {first!r}")
            if fixed is not None and not newer_than(names, fixed, listed):
                raise ValueError(
                    f"{path}: item {entry['id']!r} 'listed' release {listed!r} "
                    f"is not earlier than 'fixed' {fixed!r}")
        earlier = []
        for wording in entry.get("earlier", []):
            if set(wording) != {"until", "text"} or str(wording["text"]).strip() == "":
                raise ValueError(
                    f"{path}: item {entry['id']!r} has an 'earlier' wording "
                    f"without exactly 'until' and a non-empty 'text': {wording!r}")
            until = str(wording["until"]).strip()
            version_key(until)
            if until not in names:
                raise ValueError(
                    f"{path}: item {entry['id']!r} names an unknown "
                    f"'earlier' until release {until!r}")
            if not newer_than(names, until, listed):
                raise ValueError(
                    f"{path}: item {entry['id']!r} 'earlier' until release "
                    f"{until!r} is not later than 'listed' {listed!r}")
            earlier.append((until, " ".join(str(wording["text"]).split())))
        earlier.sort(key=lambda wording: names.index(wording[0]))
        if len({until for until, _ in earlier}) != len(earlier):
            raise ValueError(
                f"{path}: item {entry['id']!r} has two 'earlier' wordings "
                f"with the same until release")
        items.append({
            "id": str(entry["id"]),
            "kind": str(entry["kind"]),
            "area": " ".join(str(entry["area"]).split()),
            "text": " ".join(str(entry["text"]).split()),
            "first": first,
            "listed": listed,
            "target": str(entry["target"]),
            "severity": str(entry["severity"]),
            "owner": str(entry["owner"]),
            "fixed": fixed,
            "earlier": earlier,
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


def next_release(order):
    """The release after the newest CHANGELOG heading (patch bump).

    ``["0.2.3", "0.2.4"]`` gives ``"0.2.5"``. Only the trailing numeric
    component moves; any suffix stays."""
    if not order:
        raise ValueError("CHANGELOG has no release headings")
    newest = str(order[-1])
    chunks = newest.split(".")
    match = re.fullmatch(r"(\d+)(.*)", chunks[-1])
    if not match:
        raise ValueError(f"cannot name the release after {newest!r}")
    chunks[-1] = f"{int(match.group(1)) + 1}{match.group(2)}"
    return ".".join(chunks)


def with_release(order, release):
    """Order plus ``release``; a release newer than every heading goes last."""
    names = list(order)
    if release not in names:
        names.append(release)
    return names


def extended_order(order, release, items=()):
    """Release order plus the next release (when needed) plus ``release``.

    The next release sorts right after the newest heading and before any
    unknown queried release, so a ``listed`` or ``fixed`` release (a known
    release or the next one) always has a defined position next to the
    queried release."""
    names = list(order)
    following = next_release(order) if order else None
    if (following is not None and following not in names
            and (release == following
                 or any(following in (item.get("listed"), item.get("fixed"))
                        for item in items))):
        names.append(following)
    for extra in [item.get("fixed") for item in items] + [release]:
        if extra is not None and extra not in names:
            names.append(extra)
    return names


def applies(item, release, order):
    """Whether the item is open at ``release``.

    Open means its ``listed`` release (``first`` when unset) is not newer
    than ``release`` and its ``fixed`` release (when set) is newer than
    ``release``. A release with no CHANGELOG heading yet (the next release)
    counts as newer than every heading. Fixed items stay in the ledger as
    history; ``render`` and ``check`` simply skip them where they are closed."""
    if item["first"] not in order and item["first"] != next_release(order):
        raise ValueError(
            f"ledger item {item['id']!r} names an unknown release {item['first']!r}")
    names = extended_order(order, release, (item,))
    if names.index(item["listed"]) > names.index(release):
        return False
    fixed = item.get("fixed")
    return fixed is None or names.index(release) < names.index(fixed)


def carried_releases(first, release, order):
    """Releases that carry the item into ``release``: ``first`` up to (not
    including) ``release``."""
    names = extended_order(order, release, ({"listed": first},))
    if first not in names:
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


def item_text(item, release, order):
    """The item's wording at ``release``: the first ``earlier`` wording whose
    ``until`` release comes after ``release``, else ``text``. Rewording an item
    that a published release already lists keeps the old wording there as an
    ``earlier`` entry, so ``check`` of that release keeps passing."""
    for until, text in item["earlier"]:
        if newer_than(order, until, release):
            return text
    return item["text"]


def render_release(items, order, release):
    """Render both CHANGELOG blocks for ``release``; skip empty blocks."""
    blocks = []
    for kind, heading in ((NOT_IN_RELEASE, NOT_IN_RELEASE_HEADING),
                          (KNOWN_ISSUE, KNOWN_ISSUES_HEADING)):
        bullets = []
        for item in items:
            if item["kind"] != kind or not applies(item, release, order):
                continue
            body = item_text(item, release, order)
            if kind == KNOWN_ISSUE:
                body = f"**{item['area']}:** {body}"
            sentence = carried_sentence(item["first"], release, order)
            if sentence:
                body = f"{body} {sentence}"
            bullets.append(wrap_bullet(body))
        if bullets:
            blocks.append(f"{heading}\n\n" + "\n".join(bullets) + "\n")
    return "\n".join(blocks)


def section_lines(changelog_path, release=None):
    """Raw lines of one ``##`` section (heading line excluded).

    Release headings carry a date suffix (``## [0.2.4] — ...``), so match the
    bracketed name instead of the whole line."""
    with open(changelog_path, encoding="utf-8") as handle:
        lines = handle.read().splitlines()
    start = None
    for index, line in enumerate(lines):
        stripped = line.strip()
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
    """One row per open item: id, first, listed, carry count, target, owner,
    ruling flag. The carry count and the ruling flag count from ``first``,
    the true age of the problem, whatever release first listed it in.

    Items closed at ``release`` (already fixed) and items not yet listed
    (``listed`` is newer) have no row: the table describes the queried
    release only."""
    rows = []
    for item in items:
        if not applies(item, release, order):
            continue
        rows.append({
            "id": item["id"],
            "first": item["first"],
            "listed": item["listed"],
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
        "listed": max([len("listed")] + [len(row["listed"]) for row in rows]),
        "target": max([len("target")] + [len(row["target"]) for row in rows]),
        "owner": max([len("owner")] + [len(row["owner"]) for row in rows]),
    }
    header = (
        f"{'id':<{widths['id']}}  {'first':<{widths['first']}}  "
        f"{'listed':<{widths['listed']}}  "
        f"{'carried':>7}  {'target':<{widths['target']}}  "
        f"{'owner':<{widths['owner']}}  ruling"
    )
    lines = [header]
    for row in rows:
        lines.append(
            f"{row['id']:<{widths['id']}}  {row['first']:<{widths['first']}}  "
            f"{row['listed']:<{widths['listed']}}  "
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
    order = changelog_releases(changelog_path)
    items = load_ledger(ledger_path, order)
    sys.stdout.write(render_release(items, order, args.release))


def cmd_check(args):
    ledger_path, changelog_path = default_paths(args)
    order = changelog_releases(changelog_path)
    items = load_ledger(ledger_path, order)
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
    order = changelog_releases(changelog_path)
    items = load_ledger(ledger_path, order)
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
    check.add_argument("--release", required=True)
    check.set_defaults(func=cmd_check)
    carried = commands.add_parser("carried", help="table open items and their carry count")
    carried.add_argument("--release", required=True)
    carried.set_defaults(func=cmd_carried)
    return result


def main(argv=None):
    arguments = parser().parse_args(argv)
    try:
        outcome = arguments.func(arguments)
    except (OSError, ValueError) as error:
        parser().error(str(error))
    return outcome or 0


def check_release_by_paths(ledger_path, changelog_path, release):
    """Entry point for release.py preflight: diff lines, empty means pass."""
    order = changelog_releases(changelog_path)
    items = load_ledger(ledger_path, order)
    section = section_lines(changelog_path, release=release)
    return check_release(items, order, release, section)


if __name__ == "__main__":
    sys.exit(main())
