#!/usr/bin/env python3
"""Unit tests for known_issues.py.

All fixtures are synthetic: no test reads the real ledger wording or the
real CHANGELOG text. Each test builds a small ledger and a small CHANGELOG
in a temporary directory.
"""

import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

TOOLS = Path(__file__).resolve().parent


def load_tool():
    spec = importlib.util.spec_from_file_location(
        "known_issues", TOOLS / "known_issues.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


known_issues = load_tool()

CHANGELOG = """# Changelog

## Unreleased

## [1.1.0] — 2026-10-20 — Test release

### Features

- Something new.

## [1.0.1] — 2026-10-14 — Test patch

## [1.0.0] — 2026-10-08 — Test release
"""

LEDGER = """\
[[item]]
id = "alpha-gap"
kind = "not-in-release"
area = "Widgets"
text = "The widget counter stays empty until the first widget arrives."
first = "1.0.0"
target = "none"
severity = "low"
owner = "widgets"

[[item]]
id = "beta-crash"
kind = "known-issue"
area = "Sprockets"
text = "A sprocket can stop on a red cog. Turn the cog by hand and start again."
first = "1.0.1"
target = "1.1.0"
severity = "medium"
owner = "sprockets"

[[item]]
id = "gamma-slow"
kind = "known-issue"
area = "Gears"
text = "Big gear trains turn slowly on cold mornings. Warm the room first."
first = "1.0.0"
target = "none"
severity = "low"
owner = "gears"
"""


class Fixture(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.ledger = self.root / "ledger.toml"
        self.changelog = self.root / "CHANGELOG.md"
        self.ledger.write_text(LEDGER, encoding="utf-8")
        self.changelog.write_text(CHANGELOG, encoding="utf-8")
        self.order = known_issues.changelog_releases(self.changelog)
        self.items = known_issues.load_ledger(self.ledger, self.order)

    def tearDown(self):
        self.tmp.cleanup()

    def render(self, release):
        return known_issues.render_release(self.items, self.order, release)

    def write_section(self, release, body):
        """Insert a release section carrying the given block body."""
        text = self.changelog.read_text(encoding="utf-8")
        section = f"## [{release}] — test\n\n{body}\n"
        marker = "## [1.1.0]"
        text = text.replace(marker, section + "\n" + marker)
        self.changelog.write_text(text, encoding="utf-8")
        self.order = known_issues.changelog_releases(self.changelog)

    def check(self, release):
        section = known_issues.section_lines(self.changelog, release=release)
        return known_issues.check_release(self.items, self.order, release, section)


class RenderTest(Fixture):
    def test_render_small_ledger(self):
        out = self.render("1.0.1")
        self.assertIn("### Not in this release", out)
        self.assertIn("### Known issues", out)
        self.assertIn("- **Sprockets:**", out)
        # Items first seen in an earlier release gain a carried sentence.
        self.assertIn("1.0.0 has the same problem.", out)

    def test_render_carried_range(self):
        out = self.render("1.1.0")
        self.assertIn("1.0.0 to 1.0.1 have the same problem.", out)
        self.assertIn("1.0.1 has the same problem.", out)

    def test_render_lines_fit_80_columns(self):
        for release in ("1.0.0", "1.0.1", "1.1.0"):
            for line in self.render(release).splitlines():
                self.assertLessEqual(len(line), 80, line)

    def test_render_skips_future_items(self):
        out = self.render("1.0.0")
        self.assertNotIn("Sprockets", out)
        self.assertIn("Gears", out)


class CheckTest(Fixture):
    def test_check_passes_on_matching_changelog(self):
        self.write_section("1.1.0", self.render("1.1.0"))
        self.assertEqual(self.check("1.1.0"), [])

    def test_check_fails_on_dropped_item(self):
        items = [item for item in self.items if item["id"] != "gamma-slow"]
        body = known_issues.render_release(items, self.order, "1.1.0")
        self.write_section("1.1.0", body)
        self.assertTrue(self.check("1.1.0"))

    def test_check_fails_on_edited_item(self):
        body = self.render("1.1.0").replace("Warm the room first.",
                                            "Warm the building first.")
        self.write_section("1.1.0", body)
        self.assertTrue(self.check("1.1.0"))

    def test_check_fails_on_extra_item(self):
        body = self.render("1.1.0") + "- An extra bullet that is not in the ledger.\n"
        self.write_section("1.1.0", body)
        self.assertTrue(self.check("1.1.0"))

    def test_check_cli_exit_codes(self):
        base = [sys.executable, str(TOOLS / "known_issues.py"),
                "--ledger", str(self.ledger),
                "--changelog", str(self.changelog)]
        self.write_section("1.1.0", self.render("1.1.0"))
        good = subprocess.run(base + ["check", "--release", "1.1.0"],
                              capture_output=True, text=True)
        self.assertEqual(good.returncode, 0, good.stdout + good.stderr)
        self.write_section("1.0.1", self.render("1.0.1") + "\n- Extra.\n")
        bad = subprocess.run(base + ["check", "--release", "1.0.1"],
                             capture_output=True, text=True)
        self.assertNotEqual(bad.returncode, 0)
        self.assertTrue(bad.stdout.strip())


class CarriedTest(Fixture):
    def rows(self, release):
        return {row["id"]: row
                for row in known_issues.carried_rows(self.items, self.order, release)}

    def test_carried_counts_releases(self):
        rows = self.rows("1.1.0")
        self.assertEqual(rows["alpha-gap"]["carried"], 2)
        self.assertEqual(rows["beta-crash"]["carried"], 1)
        self.assertEqual(rows["gamma-slow"]["carried"], 2)

    def test_carried_flags_long_holds(self):
        rows = self.rows("1.1.0")
        self.assertTrue(rows["alpha-gap"]["ruling"])
        self.assertTrue(rows["gamma-slow"]["ruling"])

    def test_carried_flags_at_target(self):
        rows = self.rows("1.1.0")
        self.assertTrue(rows["beta-crash"]["ruling"])

    def test_carried_flags_past_target(self):
        self.order.append("1.2.0")
        rows = self.rows("1.2.0")
        self.assertTrue(rows["beta-crash"]["ruling"])

    def test_carried_no_ruling_before_target(self):
        rows = self.rows("1.0.1")
        self.assertFalse(rows["beta-crash"]["ruling"])
        self.assertEqual(rows["beta-crash"]["carried"], 0)


FIXED_LEDGER = """\
[[item]]
id = "alpha-gap"
kind = "not-in-release"
area = "Widgets"
text = "The widget counter stays empty until the first widget arrives."
first = "1.0.0"
target = "none"
severity = "low"
owner = "widgets"

[[item]]
id = "beta-crash"
kind = "known-issue"
area = "Sprockets"
text = "A sprocket can stop on a red cog. Turn the cog by hand and start again."
first = "1.0.1"
target = "1.1.0"
severity = "medium"
owner = "sprockets"
fixed = "1.1.0"
"""


class FixedTest(Fixture):
    """Items with ``fixed`` stay in the ledger but render only when open."""

    def setUp(self):
        super().setUp()
        self.ledger.write_text(FIXED_LEDGER, encoding="utf-8")
        self.items = known_issues.load_ledger(self.ledger, self.order)

    def render_fixed(self, release):
        return known_issues.render_release(self.items, self.order, release)

    def test_fixed_item_renders_before_fix(self):
        self.assertIn("Sprockets", self.render_fixed("1.0.1"))

    def test_fixed_item_missing_after_fix(self):
        out = self.render_fixed("1.1.0")
        self.assertNotIn("Sprockets", out)
        self.assertIn("widget counter", out)

    def test_fixed_item_open_until_fix_release(self):
        by_id = {item["id"]: item for item in self.items}
        self.assertTrue(
            known_issues.applies(by_id["beta-crash"], "1.0.1", self.order))
        self.assertFalse(
            known_issues.applies(by_id["beta-crash"], "1.1.0", self.order))

    def test_check_passes_for_old_release_after_fix(self):
        self.write_section("1.0.1", self.render_fixed("1.0.1"))
        section = known_issues.section_lines(self.changelog, release="1.0.1")
        self.assertEqual(
            known_issues.check_release(self.items, self.order, "1.0.1",
                                       section), [])

    def test_carried_leaves_out_fixed_item(self):
        rows = {row["id"]: row for row in known_issues.carried_rows(
            self.items, self.order, "1.1.0")}
        self.assertNotIn("beta-crash", rows)
        self.assertIn("alpha-gap", rows)

    def test_carried_keeps_item_before_fix(self):
        rows = {row["id"]: row for row in known_issues.carried_rows(
            self.items, self.order, "1.0.1")}
        self.assertIn("beta-crash", rows)
        self.assertEqual(rows["beta-crash"]["carried"], 0)

    def test_carried_omits_item_before_first(self):
        rows = {row["id"]: row for row in known_issues.carried_rows(
            self.items, self.order, "1.0.0")}
        self.assertNotIn("beta-crash", rows)
        self.assertIn("alpha-gap", rows)

    def test_next_release_fixed_loads(self):
        text = FIXED_LEDGER.replace('fixed = "1.1.0"', 'fixed = "1.1.1"')
        self.ledger.write_text(text, encoding="utf-8")
        items = known_issues.load_ledger(self.ledger, self.order)
        by_id = {item["id"]: item for item in items}
        self.assertTrue(
            known_issues.applies(by_id["beta-crash"], "1.1.0", self.order))
        self.assertFalse(
            known_issues.applies(by_id["beta-crash"], "1.1.1", self.order))

    def test_unknown_fixed_rejected(self):
        text = FIXED_LEDGER.replace('fixed = "1.1.0"', 'fixed = "9.9.9"')
        self.ledger.write_text(text, encoding="utf-8")
        with self.assertRaises(ValueError):
            known_issues.load_ledger(self.ledger, self.order)

    def test_fixed_not_later_than_first_rejected(self):
        for bad in ('"1.0.1"', '"1.0.0"'):
            text = FIXED_LEDGER.replace('fixed = "1.1.0"', f"fixed = {bad}")
            self.ledger.write_text(text, encoding="utf-8")
            with self.assertRaises(ValueError, msg=bad):
                known_issues.load_ledger(self.ledger, self.order)

    def test_unknown_field_rejected(self):
        text = FIXED_LEDGER.replace('fixed = "1.1.0"', 'fixd = "1.1.0"')
        self.ledger.write_text(text, encoding="utf-8")
        with self.assertRaises(ValueError):
            known_issues.load_ledger(self.ledger, self.order)

    def test_empty_fixed_rejected(self):
        text = FIXED_LEDGER.replace('fixed = "1.1.0"', 'fixed = ""')
        self.ledger.write_text(text, encoding="utf-8")
        with self.assertRaises(ValueError):
            known_issues.load_ledger(self.ledger, self.order)


RELEASE_CHANGELOG = """# Changelog

## Unreleased

## [0.2.4] — 2026-10-20 — Test release

## [0.2.3] — 2026-10-14 — Test release
"""

RELEASE_LEDGER = """\
[[item]]
id = "gatherer-catherby-obelisk"
kind = "known-issue"
area = "Gatherer"
text = "A fishing card can pick an unreachable spot. Start the card again from the beach."
first = "0.2.3"
target = "0.2.5"
severity = "medium"
owner = "scripts-misc"
fixed = "0.2.5"

[[item]]
id = "panel-linux-dual-gpu"
kind = "known-issue"
area = "Panel (Linux)"
text = "The panel can close right after it starts. Start it with the fallback flag."
first = "0.2.3"
target = "none"
severity = "high"
owner = "dual-gpu"
"""


class PublishedReleaseTest(unittest.TestCase):
    """Recording a fix for the next release keeps old checks green."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.ledger = self.root / "ledger.toml"
        self.changelog = self.root / "CHANGELOG.md"
        self.ledger.write_text(RELEASE_LEDGER, encoding="utf-8")
        self.changelog.write_text(RELEASE_CHANGELOG, encoding="utf-8")
        self.order = known_issues.changelog_releases(self.changelog)
        self.items = known_issues.load_ledger(self.ledger, self.order)

    def tearDown(self):
        self.tmp.cleanup()

    def stage(self, release):
        """Write the render into the matching CHANGELOG section."""
        body = known_issues.render_release(self.items, self.order, release)
        text = self.changelog.read_text(encoding="utf-8")
        marker = f"## [{release}] — 2026-10-20 — Test release\n"
        self.changelog.write_text(
            text.replace(marker, marker + "\n" + body), encoding="utf-8")
        self.order = known_issues.changelog_releases(self.changelog)

    def test_fix_renders_in_old_release_not_new(self):
        old = known_issues.render_release(self.items, self.order, "0.2.4")
        new = known_issues.render_release(self.items, self.order, "0.2.5")
        self.assertIn("Gatherer", old)
        self.assertNotIn("Gatherer", new)
        self.assertIn("Panel (Linux)", new)

    def test_check_old_release_passes_after_fix(self):
        self.stage("0.2.4")
        section = known_issues.section_lines(self.changelog, release="0.2.4")
        self.assertEqual(
            known_issues.check_release(self.items, self.order, "0.2.4",
                                       section), [])

    def test_check_cli_passes_for_old_release(self):
        self.stage("0.2.4")
        proc = subprocess.run(
            [sys.executable, str(TOOLS / "known_issues.py"),
             "--ledger", str(self.ledger),
             "--changelog", str(self.changelog),
             "check", "--release", "0.2.4"],
            capture_output=True, text=True)
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)

    def test_carried_next_release_leaves_out_fixed_item(self):
        rows = known_issues.carried_rows(self.items, self.order, "0.2.5")
        self.assertEqual([row["id"] for row in rows], ["panel-linux-dual-gpu"])


class NextReleaseTest(unittest.TestCase):
    def test_patch_bump(self):
        self.assertEqual(
            known_issues.next_release(["0.2.3", "0.2.4"]), "0.2.5")

    def test_longer_chain(self):
        self.assertEqual(
            known_issues.next_release(["1.0.0", "1.0.1", "1.1.0"]), "1.1.1")

    def test_no_headings(self):
        with self.assertRaises(ValueError):
            known_issues.next_release([])

if __name__ == "__main__":
    unittest.main()
