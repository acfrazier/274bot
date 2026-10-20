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
        self.items = known_issues.load_ledger(self.ledger)
        self.order = known_issues.changelog_releases(self.changelog)

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


if __name__ == "__main__":
    unittest.main()
