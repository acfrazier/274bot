#!/usr/bin/env python3
"""Tests for tools/docs-check/docs_check.py.

Run with `python3 -B tools/docs-check/test_docs_check.py`.

Each fixture is a temporary directory with a fresh `git init` and `git add`;
the checker under test is copied into it so its default baseline lives inside
the fixture. No network is used.
"""

from __future__ import annotations

import json
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

TOOL = Path(__file__).resolve().parent / "docs_check.py"
FINDING_RE = re.compile(r"^(?P<path>[^:]+):(?P<line>\d+): (?P<rule>[a-z-]+): (?P<message>.*)$")


def words(count: int) -> str:
    return " ".join(["word"] * count)


def sentence(count: int) -> str:
    return words(count) + "."


def paragraph(*counts: int) -> str:
    return " ".join(sentence(count) for count in counts)


class DocsCheckTest(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        tool_dir = self.root / "tools" / "docs-check"
        tool_dir.mkdir(parents=True)
        shutil.copy2(TOOL, tool_dir / "docs_check.py")
        self.git("init", "-q")

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def git(self, *args: str) -> None:
        subprocess.run(
            ["git", "-C", str(self.root), *args],
            check=True,
            capture_output=True,
            env={"HOME": str(self.root), "PATH": "/usr/bin:/bin"},
        )

    def write(self, rel: str, content: str) -> None:
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")

    def run_check(self, *args: str, baseline: dict | None = None) -> subprocess.CompletedProcess:
        baseline_path = self.root / "tools" / "docs-check" / "baseline.json"
        if baseline is not None:
            baseline_path.parent.mkdir(parents=True, exist_ok=True)
            baseline_path.write_text(
                json.dumps(baseline, indent=2, sort_keys=True) + "\n", encoding="utf-8"
            )
        self.git("add", "-A")
        return subprocess.run(
            [
                sys.executable,
                "-B",
                str(self.root / "tools" / "docs-check" / "docs_check.py"),
                "--root",
                str(self.root),
                *args,
            ],
            capture_output=True,
            text=True,
        )

    def findings(self, result: subprocess.CompletedProcess) -> list[tuple[str, int, str]]:
        found = []
        for line in result.stdout.splitlines():
            match = FINDING_RE.match(line)
            if match:
                found.append((match["path"], int(match["line"]), match["rule"]))
        return found

    def assert_finding(self, result: subprocess.CompletedProcess, rel: str, line: int, rule: str) -> None:
        self.assertIn(
            f"{rel}:{line}: {rule}:",
            result.stdout,
            f"missing {rule} at {rel}:{line}\nstdout:\n{result.stdout}",
        )

    # -- one test per rule ---------------------------------------------

    def test_link_missing(self) -> None:
        self.write("docs/manual/page.md", "# Page\n\nA page.\n")
        passing = (
            "# Links\n\n"
            "[here](../manual/page.md)\n\n"
            "[web](https://example.com/page)\n"
            "[mail](mailto:someone@example.com)\n"
        )
        self.write("docs/dev/links.md", passing)
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.findings(result), [])

        self.write("docs/dev/links.md", "# Links\n\n[gone](missing.md)\n")
        result = self.run_check()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assert_finding(result, "docs/dev/links.md", 3, "link-missing")

    def test_anchor_missing(self) -> None:
        self.write("docs/dev/anchors.md", "# Anchors\n\n[top](#anchors)\n")
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

        self.write("docs/dev/anchors.md", "# Anchors\n\n[top](#anchors)\n[bad](#missing)\n")
        result = self.run_check()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assert_finding(result, "docs/dev/anchors.md", 4, "anchor-missing")

    def test_sentence_words(self) -> None:
        self.write("docs/manual/page.md", "# Page\n\n" + sentence(30) + "\n")
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

        self.write("docs/manual/page.md", "# Page\n\n" + sentence(31) + "\n")
        result = self.run_check()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assert_finding(result, "docs/manual/page.md", 3, "sentence-words")

    def test_paragraph_words(self) -> None:
        self.write("docs/manual/page.md", "# Page\n\n" + paragraph(26, 26, 26) + "\n")
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

        self.write("docs/manual/page.md", "# Page\n\n" + paragraph(27, 27, 27) + "\n")
        result = self.run_check()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assert_finding(result, "docs/manual/page.md", 3, "paragraph-words")

    def test_cell_sentences(self) -> None:
        table = (
            "# Table\n\n"
            "| A | B |\n"
            "| --- | --- |\n"
            "| One. Two. | Three. |\n"
        )
        self.write("docs/manual/page.md", table)
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

        table = (
            "# Table\n\n"
            "| One. Two. Three. | Four. |\n"
            "| --- | --- |\n"
            "| One. Two. Three. | Four. |\n"
        )
        self.write("docs/manual/page.md", table)
        result = self.run_check()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assert_finding(result, "docs/manual/page.md", 5, "cell-sentences")
        self.assertNotIn("docs/manual/page.md:3: cell-sentences:", result.stdout)

    def test_table_columns(self) -> None:
        table = (
            "# Table\n\n"
            "| A | B | C | D | E | F |\n"
            "| --- | --- | --- | --- | --- | --- |\n"
            "| 1 | 2 | 3 | 4 | 5 | 6 |\n"
        )
        self.write("docs/manual/page.md", table)
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

        table = (
            "# Table\n\n"
            "| A | B | C | D | E | F |\n"
            "| --- | --- | --- | --- | --- | --- |\n"
            "| 1 | 2 | 3 | 4 | 5 | 6 | 7 |\n"
        )
        self.write("docs/manual/page.md", table)
        result = self.run_check()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assert_finding(result, "docs/manual/page.md", 5, "table-columns")

    def test_internal_name(self) -> None:
        ordinary = (
            "# Ordinary words\n\n"
            "The panel, tui, script, nav, api, host and vault words are ordinary. "
            "tui-play, panel-play and host-play are commands. "
            "The `--profile` flag picks a server from `servers.json`. "
            "The server is game.example.net:43594.\n"
        )
        self.write("docs/manual/page.md", ordinary)
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.findings(result), [])

        internal = (
            "# Names\n\n"
            "The value `api::Host` is internal.\n\n"
            "See foo.rs for details.\n\n"
            "See foo.rs:12 for details.\n\n"
            "Files under crates/ are internal.\n\n"
            "Use `frontend-core` here.\n\n"
            "The server is game.example.net:43594.\n"
        )
        self.write("docs/manual/page.md", internal)
        result = self.run_check()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        found = [(line, rule) for _, line, rule in self.findings(result)]
        self.assertEqual(
            found,
            [
                (3, "internal-name"),
                (5, "internal-name"),
                (7, "internal-name"),
                (9, "internal-name"),
                (11, "internal-name"),
            ],
            result.stdout,
        )

    def test_banned_phrase(self) -> None:
        self.write(
            "docs/manual/page.md",
            "# Phrases\n\n"
            "The menu is closed.\n\n"
            "An attempt is made.\n\n"
            "This is accurate.\n\n"
            "It is used.\n\n"
            "It was changed.\n\n"
            "Version 0.2.5 works.\n",
        )
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

        self.write(
            "docs/manual/page.md",
            "# Phrases\n\n"
            "It fails closed here.\n\n"
            "A best-effort attempt.\n\n"
            "A best effort attempt.\n\n"
            "This is honest.\n\n"
            "It is no longer used.\n\n"
            "That was removed.\n\n"
            "This is from 0.2.5.\n",
        )
        result = self.run_check()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        for line in (3, 5, 7, 9, 11, 13, 15):
            self.assert_finding(result, "docs/manual/page.md", line, "banned-phrase")

    def test_page_words(self) -> None:
        body = "\n\n".join(paragraph(20, 20, 20, 20) for _ in range(10))
        self.write("docs/manual/page.md", "# Page\n\n" + body + "\n")
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

        self.write("docs/manual/page.md", "# Page\n\n" + body + "\n\nextra.\n")
        result = self.run_check()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assert_finding(result, "docs/manual/page.md", 1, "page-words")

        self.write("docs/manual/page.md", "# Page\n\n## Troubleshooting\n\n" + body + "\n\nextra.\n")
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_section_words(self) -> None:
        table = "| Term | Meaning |\n| --- | --- |\n" + "\n".join(
            "| row | " + words(9) + " |" for _ in range(60)
        )
        prose = "\n\n".join(paragraph(19, 19, 19, 19) for _ in range(5))
        self.write("docs/reference/page.md", "# Reference\n\n## Lookup\n\n" + prose + "\n\n" + table + "\n")
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

        prose = "\n\n".join(paragraph(20, 20, 20, 20) for _ in range(5)) + "\n\nextra."
        self.write("docs/reference/page.md", "# Reference\n\n## Lookup\n\n" + prose + "\n")
        result = self.run_check()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assert_finding(result, "docs/reference/page.md", 3, "section-words")

    def test_file_lines(self) -> None:
        self.write("docs/manual/page.md", "\n" * 1000)
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

        self.write("docs/manual/page.md", "\n" * 1001)
        result = self.run_check()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assert_finding(result, "docs/manual/page.md", 1001, "file-lines")

    # -- structural cases ----------------------------------------------

    def test_slugs(self) -> None:
        self.write("docs/manual/page.md", "# Page\n\nA page.\n")
        self.write(
            "docs/dev/slugs.md",
            "# Slugs\n\n"
            "## Run `tui-play` (fast)!\n\n"
            "[a](#run-tui-play-fast)\n\n"
            "## Dup\n\n"
            "[b](#dup)\n\n"
            "## Dup\n\n"
            "[c](#dup-1)\n\n"
            "## Café crème\n\n"
            "[d](#café-crème)\n\n"
            "[e](../manual/)\n\n"
            "[f](#not-a-heading)\n",
        )
        result = self.run_check()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertEqual(
            self.findings(result),
            [("docs/dev/slugs.md", 21, "anchor-missing")],
            result.stdout,
        )

    def test_ignored_text(self) -> None:
        long_sentence = sentence(40)
        self.write(
            "docs/manual/page.md",
            "---\n"
            f"bad: [front](missing-front.md) {long_sentence}\n"
            "---\n"
            "# Ignored\n\n"
            f"<!-- [comment](missing-comment.md) {long_sentence} -->\n\n"
            "```\n"
            "[fenced](missing-fence.md)\n"
            f"{long_sentence}\n"
            "foo.rs:12\n"
            "```\n\n"
            "[outside](missing-outside.md)\n"
            "\n"
            f"{long_sentence}\n"
            "foo.rs:12\n",
        )
        result = self.run_check()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertEqual(
            self.findings(result),
            [
                ("docs/manual/page.md", 14, "link-missing"),
                ("docs/manual/page.md", 16, "sentence-words"),
                ("docs/manual/page.md", 17, "internal-name"),
            ],
            result.stdout,
        )

    # -- baseline ------------------------------------------------------

    def _one_long_sentence(self) -> None:
        self.write("docs/manual/long.md", "# Long\n\n" + sentence(31) + "\n")

    def test_baseline_equal(self) -> None:
        self._one_long_sentence()
        result = self.run_check(baseline={"docs/manual/long.md|sentence-words": 1})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("over baseline:", result.stdout)
        self.assertNotIn("baseline can shrink:", result.stdout)

    def test_baseline_one_more(self) -> None:
        self.write(
            "docs/manual/long.md",
            "# Long\n\n" + sentence(31) + "\n\n" + sentence(31) + "\n",
        )
        result = self.run_check(baseline={"docs/manual/long.md|sentence-words": 1})
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("over baseline: docs/manual/long.md sentence-words 2 > 1", result.stdout)
        self.assertIn("docs-check: 1 finding(s) over baseline", result.stdout)

    def test_baseline_one_fewer(self) -> None:
        self._one_long_sentence()
        result = self.run_check(baseline={"docs/manual/long.md|sentence-words": 2})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("baseline can shrink: docs/manual/long.md sentence-words 1 < 2", result.stdout)

    def test_baseline_update(self) -> None:
        self._one_long_sentence()
        result = self.run_check("--update")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        expected = '{\n  "docs/manual/long.md|sentence-words": 1\n}\n'
        self.assertEqual(
            (self.root / "tools" / "docs-check" / "baseline.json").read_text(encoding="utf-8"),
            expected,
        )
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    # -- warnings ------------------------------------------------------

    def _glossary_fixture(self) -> None:
        self.write(
            "docs/manual/glossary.md",
            "# Glossary\n\n"
            "| Term | What it means | Do not use |\n"
            "| --- | --- | --- |\n"
            "| **bright mode** | The mode. | widget |\n",
        )

    def test_warnings(self) -> None:
        self._glossary_fixture()
        self.write("docs/manual/page.md", "# Page\n\nThe widget is fine.\n")
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("docs/manual/page.md:3: glossary-avoid:", result.stdout)
        self.assertIn("0 finding(s) over baseline, 1 warning(s)", result.stdout)

        self.write("docs/manual/page.md", "# Page\n\nThe **widget** is fine.\n")
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("glossary-avoid", result.stdout)

    def test_now_word(self) -> None:
        self.write(
            "docs/manual/page.md",
            "# Now\n\nIt is ready now.\n\n| A | B |\n| --- | --- |\n| Now | Later |\n",
        )
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("docs/manual/page.md:3: now-word:", result.stdout)
        self.assertEqual(result.stdout.count("now-word"), 1, result.stdout)


if __name__ == "__main__":
    unittest.main(verbosity=2)
