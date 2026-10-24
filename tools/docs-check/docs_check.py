#!/usr/bin/env python3
"""Docs checker: apply the writing rules to the pages in this repository.

Scans the Markdown pages that `git ls-files` tracks, reports findings as
`path:line: <id>: <message>`, and compares the per-`(path, id)` counts with
`baseline.json`.

  python3 -B tools/docs-check/docs_check.py            # check, exit 1 on growth
  python3 -B tools/docs-check/docs_check.py --update   # rebaseline, exit 0
  python3 -B tools/docs-check/docs_check.py --root DIR # check another checkout

Rules live in `docs/dev/writing-docs.md`; this file implements rules 1 to 14
as the fixed rule ids named in the tool's brief. `link-missing` and
`anchor-missing` cover every tracked page at the repository root and under
`docs/` (except `docs/superpowers/`). The style rules cover
`docs/manual/`, `docs/reference/` and `docs/scripting/`.

Fenced code blocks, HTML comments and YAML front matter are ignored by every
rule except `file-lines`.

Exit status: 0 when no (path, id) count is above its baseline, 1 when one is,
2 on a setup error (no repository, unreadable file, invalid baseline).
`--update` records the current counts and exits 0. Warnings never change the
exit status and are never written to the baseline.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import urllib.parse
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
DEFAULT_BASELINE = SCRIPT_DIR / "baseline.json"

STYLE_PREFIXES = ("docs/manual/", "docs/reference/", "docs/scripting/")
LINK_EXCLUDED_PREFIX = "docs/superpowers/"

SENTENCE_LIMIT = 30
PARAGRAPH_LIMIT = 80
CELL_SENTENCE_LIMIT = 2
TABLE_COLUMN_LIMIT = 6
PAGE_WORD_LIMIT = 800
SECTION_WORD_LIMIT = 400
FILE_LINE_LIMIT = 1000

# A lookup page, not a how-to page: its length is its content.
PAGE_WORDS_EXEMPT_PAGES = ("docs/manual/glossary.md",)
PAGE_WORDS_EXEMPT_HEADINGS = ("troubleshooting", "requirements", "when something goes wrong")

LINK_RE = re.compile(r"!?\[([^\[\]\n]*)\]\(([^()\n]*)\)")
HEADING_RE = re.compile(r"^ {0,3}(#{1,6})(?:\s+(.*?))?\s*$")
LIST_MARKER_RE = re.compile(r"^\s*(?:[-*+]|\d+\.)\s")
FENCE_OPEN_RE = re.compile(r"^\s*(`{3,}|~{3,})")
CITATION_RE = re.compile(r"[\w./-]+\.(?:rs|ts|js|py|toml|json|sh):\d+")
RS_WORD_RE = re.compile(r"(?<![\w./-])[\w./-]*\.rs\b")
CRATE_CODE_RE = re.compile(r"(?<![\w-])(?:frontend-core|nav-executor)(?![\w-])")
DOUBLE_COLON_RE = re.compile(re.escape("::"))
CRATES_PATH_RE = re.compile(re.escape("crates/"))
VERSION_FROM_RE = re.compile(r"\bfrom \d+\.\d+(?:\.\d+)*\b", re.IGNORECASE)
BANNED_PHRASES = ("fails closed", "best-effort", "best effort", "honest", "no longer", "was removed")
BOLD_RE = re.compile(r"\*\*[^*\n]+\*\*")
WORD_BEFORE = r"(?<![A-Za-z0-9_])"
WORD_AFTER = r"(?![A-Za-z0-9_])"
NOW_RE = re.compile(WORD_BEFORE + "now" + WORD_AFTER, re.IGNORECASE)


class SetupError(Exception):
    """A condition the checker cannot evaluate: no repository, unreadable file."""


def split_lines(text: str) -> list[str]:
    """Split on newlines only; a final newline does not add an empty line."""
    lines = text.split("\n")
    if lines and lines[-1] == "":
        lines.pop()
    return lines


def mark_ignored(lines: list[str]) -> list[str]:
    """Return `lines` with fenced code, HTML comments and front matter blanked.

    Line numbers and character offsets are preserved, so callers can map a
    match back to the source line.
    """
    count = len(lines)
    ignored = [False] * count

    if count and lines[0].strip() == "---":
        for close in range(1, count):
            if lines[close].strip() == "---":
                for index in range(close + 1):
                    ignored[index] = True
                break

    index = 0
    while index < count:
        if ignored[index]:
            index += 1
            continue
        match = FENCE_OPEN_RE.match(lines[index])
        if not match:
            index += 1
            continue
        fence = match.group(1)
        closing = re.compile(r"^\s*" + re.escape(fence[0]) + "{" + str(len(fence)) + r",}\s*$")
        end = index + 1
        while end < count and not closing.match(lines[end]):
            end += 1
        end = min(end, count - 1)
        for mark in range(index, end + 1):
            ignored[mark] = True
        index = end + 1

    visible: list[str] = []
    in_comment = False
    for index, line in enumerate(lines):
        if ignored[index]:
            visible.append("")
            continue
        if not in_comment and "<!--" not in line:
            visible.append(line)
            continue
        pieces: list[str] = []
        position = 0
        while position < len(line):
            if in_comment:
                close = line.find("-->", position)
                if close == -1:
                    pieces.append(" " * (len(line) - position))
                    position = len(line)
                    break
                pieces.append(" " * (close + 3 - position))
                position = close + 3
                in_comment = False
            else:
                start = line.find("<!--", position)
                if start == -1:
                    pieces.append(line[position:])
                    position = len(line)
                    break
                pieces.append(line[position:start])
                pieces.append(" " * 4)
                position = start + 4
                in_comment = True
        visible.append("".join(pieces))
    return visible


def heading_text(line: str) -> str | None:
    """Return the heading text of an ATX heading line, or None."""
    match = HEADING_RE.match(line)
    if not match:
        return None
    text = match.group(2) or ""
    text = re.sub(r"\s*#+\s*$", "", text)
    return text.strip()


def slugify(text: str) -> str:
    """GitHub's heading slug: lowercase, keep letters/digits/space/-/_, spaces to -."""
    kept = "".join(
        char
        for char in text.lower()
        if char.isalpha() or char.isdigit() or char in " -_"
    )
    return kept.replace(" ", "-")


class Doc:
    """One tracked Markdown file, with ignored text blanked out."""

    def __init__(self, root: Path, rel: str):
        self.root = root
        self.rel = rel
        self.path = root / rel
        try:
            self.text = self.path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError) as exc:
            raise SetupError(f"{rel}: cannot read ({exc})") from exc
        self.lines = split_lines(self.text)
        self.visible = mark_ignored(self.lines)
        self.headings: list[tuple[int, str]] = []
        for index, line in enumerate(self.visible):
            text = heading_text(line)
            if text is not None:
                self.headings.append((index + 1, text))
        self.slugs: dict[str, int] = {}
        used: dict[str, int] = {}
        for _, text in self.headings:
            base = slugify(text)
            if base not in used:
                used[base] = 0
                self.slugs[base] = 1
                continue
            number = used[base] + 1
            candidate = f"{base}-{number}"
            while candidate in used:
                number += 1
                candidate = f"{base}-{number}"
            used[base] = number
            used[candidate] = 0
            self.slugs[candidate] = 1

    @property
    def line_count(self) -> int:
        return len(self.lines)


def code_spans(text: str) -> list[tuple[int, int]]:
    """Return (start, end) pairs for inline code spans, delimiters included."""
    spans: list[tuple[int, int]] = []
    index = 0
    length = len(text)
    while index < length:
        if text[index] != "`":
            index += 1
            continue
        run_end = index
        while run_end < length and text[run_end] == "`":
            run_end += 1
        run = run_end - index
        scan = run_end
        close_at = -1
        close_end = -1
        while scan < length:
            if text[scan] == "`":
                candidate_end = scan
                while candidate_end < length and text[candidate_end] == "`":
                    candidate_end += 1
                if candidate_end - scan == run:
                    close_at = scan
                    close_end = candidate_end
                    break
                scan = candidate_end
            else:
                scan += 1
        if close_at == -1:
            index = run_end
            continue
        spans.append((index, close_end))
        index = close_end
    return spans


def mask_code(text: str, spans: list[tuple[int, int]]) -> str:
    """Blank code spans, keeping the string length and offsets."""
    chars = list(text)
    for start, end in spans:
        for index in range(start, end):
            chars[index] = " "
    return "".join(chars)


def normalize_for_counting(text: str) -> str:
    """Keep link text, reduce each code span to one word, keep the length."""
    spans = code_spans(text)
    chars = list(text)
    for start, end in spans:
        for index in range(start, end):
            chars[index] = " "
        chars[start] = "x"
    masked = "".join(chars)
    pieces: list[str] = []
    position = 0
    for match in LINK_RE.finditer(masked):
        pieces.append(masked[position:match.start()])
        label = match.group(1)
        width = match.end() - match.start()
        label = label[:width]
        pieces.append(label + " " * (width - len(label)))
        position = match.end()
    pieces.append(masked[position:])
    return "".join(pieces)


def mask_code_and_link_targets(text: str) -> str:
    """Blank inline code spans and link targets, keeping the length and offsets."""
    chars = list(text)
    for start, end in code_spans(text):
        for index in range(start, end):
            chars[index] = " "
    for match in LINK_RE.finditer(text):
        for index in range(match.start(2) - 1, match.end(2) + 1):
            chars[index] = " "
    return "".join(chars)


def is_word(token: str) -> bool:
    """A token is a word when at least one character is alphanumeric."""
    return any(char.isalnum() for char in token)


def count_words(text: str) -> int:
    return sum(1 for token in normalize_for_counting(text).split() if is_word(token))


def sentences_with_starts(text: str) -> list[tuple[int, int]]:
    """Return (start offset, word count) for each sentence in `text`."""
    normalized = normalize_for_counting(text)
    starts = [0]
    for match in re.finditer(r"[.!?]\s+", normalized):
        starts.append(match.end())
    result: list[tuple[int, int]] = []
    for position, start in enumerate(starts):
        end = starts[position + 1] if position + 1 < len(starts) else len(normalized)
        segment = normalized[start:end]
        if segment.strip():
            result.append((start, sum(1 for token in segment.split() if is_word(token))))
    return result


def sentence_count(text: str) -> int:
    return len(sentences_with_starts(text))


def table_cells(row: str) -> list[str]:
    stripped = row.strip()
    if stripped.startswith("|"):
        stripped = stripped[1:]
    if stripped.endswith("|"):
        stripped = stripped[:-1]
    return stripped.split("|")


def is_separator_row(row: str) -> bool:
    cells = [cell.strip() for cell in table_cells(row)]
    filled = [cell for cell in cells if cell]
    if not filled:
        return False
    return all(re.fullmatch(r":?-{1,}:?", cell) for cell in filled)


def prose_paragraphs(doc: Doc) -> list[list[int]]:
    """Group prose line indexes into paragraphs (1-based lines for output)."""
    paragraphs: list[list[int]] = []
    current: list[int] = []

    def flush() -> None:
        if current:
            paragraphs.append(list(current))
            current.clear()

    for index, line in enumerate(doc.visible):
        stripped = line.strip()
        if not stripped or heading_text(line) is not None or stripped.startswith("|"):
            flush()
            continue
        if LIST_MARKER_RE.match(line):
            flush()
            current.append(index)
        else:
            current.append(index)
    flush()
    return paragraphs


def paragraph_text(doc: Doc, line_indexes: list[int]) -> tuple[str, list[int]]:
    pieces: list[str] = []
    line_map: list[int] = []
    for index in line_indexes:
        if pieces:
            pieces.append(" ")
            line_map.append(index + 1)
        line = doc.visible[index]
        pieces.append(line)
        line_map.extend([index + 1] * len(line))
    return "".join(pieces), line_map


def line_for_offset(line_map: list[int], offset: int) -> int:
    if not line_map:
        return 1
    if offset >= len(line_map):
        return line_map[-1]
    return line_map[offset]


def link_destination(raw: str) -> str:
    """Strip angle brackets or a link title from an inline link target."""
    target = raw.strip()
    if target.startswith("<") and ">" in target:
        return target[1 : target.index(">")]
    if not target:
        return ""
    return target.split()[0]


def scheme_skipped(destination: str) -> bool:
    lowered = destination.lower()
    return lowered.startswith(("http://", "https://", "mailto:"))


class Checker:
    def __init__(self, root: Path, tracked: list[str]):
        self.root = root
        self.tracked = tracked
        self.link_files = sorted(
            rel
            for rel in tracked
            if rel.endswith(".md")
            and (
                "/" not in rel
                or (rel.startswith("docs/") and not rel.startswith(LINK_EXCLUDED_PREFIX))
            )
        )
        self.style_files = sorted(
            rel
            for rel in tracked
            if rel.endswith(".md") and rel.startswith(STYLE_PREFIXES)
        )
        self.findings: list[tuple[str, int, str, str]] = []
        self.warnings: list[tuple[str, int, str, str]] = []
        self._docs: dict[str, Doc] = {}
        self._slug_sets: dict[Path, set[str]] = {}

    def doc(self, rel: str) -> Doc:
        if rel not in self._docs:
            self._docs[rel] = Doc(self.root, rel)
        return self._docs[rel]

    def add(self, rel: str, line: int, rule: str, message: str) -> None:
        self.findings.append((rel, line, rule, message))

    def warn(self, rel: str, line: int, rule: str, message: str) -> None:
        self.warnings.append((rel, line, rule, message))

    # -- rules ---------------------------------------------------------

    def check_links(self, rel: str) -> None:
        doc = self.doc(rel)
        for index, line in enumerate(doc.visible):
            if not line:
                continue
            masked = mask_code(line, code_spans(line))
            for match in LINK_RE.finditer(masked):
                destination = link_destination(match.group(2))
                if not destination or scheme_skipped(destination):
                    continue
                number = index + 1
                if destination.startswith("#"):
                    self.check_anchor(rel, doc, number, doc.path, destination[1:])
                    continue
                path_part, _, anchor = destination.partition("#")
                path_part = urllib.parse.unquote(path_part)
                if not path_part:
                    self.check_anchor(rel, doc, number, doc.path, anchor)
                    continue
                candidate = Path(str(doc.path.parent / path_part))
                try:
                    exists = candidate.exists()
                except OSError:
                    exists = False
                if not exists:
                    self.add(rel, number, "link-missing", f"link target not found: {destination}")
                    continue
                if anchor and candidate.suffix.lower() == ".md" and candidate.is_file():
                    self.check_anchor(rel, doc, number, candidate, anchor)

    def check_anchor(self, rel: str, doc: Doc, line: int, target: Path, anchor: str) -> None:
        if not anchor:
            return
        try:
            resolved = target.resolve()
        except OSError:
            resolved = target
        if resolved == doc.path.resolve():
            slugs = doc.slugs
        else:
            slugs = self.slugs_for(resolved)
        if anchor not in slugs:
            self.add(rel, line, "anchor-missing", f"no heading with anchor #{anchor} in {target.name}")

    def slugs_for(self, path: Path) -> set[str]:
        if path not in self._slug_sets:
            try:
                sibling = Doc(self.root, str(path))
                self._slug_sets[path] = set(sibling.slugs)
            except SetupError:
                self._slug_sets[path] = set()
        return self._slug_sets[path]

    def check_sentences(self, rel: str) -> None:
        doc = self.doc(rel)
        for lines in prose_paragraphs(doc):
            text, line_map = paragraph_text(doc, lines)
            for start, words in sentences_with_starts(text):
                if words > SENTENCE_LIMIT:
                    line = line_for_offset(line_map, start)
                    self.add(rel, line, "sentence-words", f"sentence has {words} words (limit {SENTENCE_LIMIT})")

    def check_paragraphs(self, rel: str) -> None:
        doc = self.doc(rel)
        for lines in prose_paragraphs(doc):
            text, _ = paragraph_text(doc, lines)
            words = count_words(text)
            if words > PARAGRAPH_LIMIT:
                self.add(rel, lines[0] + 1, "paragraph-words", f"paragraph has {words} words (limit {PARAGRAPH_LIMIT})")

    def check_tables(self, rel: str) -> None:
        doc = self.doc(rel)
        index = 0
        count = len(doc.visible)
        while index < count:
            if not doc.visible[index].strip().startswith("|"):
                index += 1
                continue
            start = index
            while index < count and doc.visible[index].strip().startswith("|"):
                index += 1
            rows = list(range(start, index))
            for position, row_index in enumerate(rows):
                row = doc.visible[row_index]
                number = row_index + 1
                cells = table_cells(row)
                if len(cells) > TABLE_COLUMN_LIMIT:
                    self.add(rel, number, "table-columns", f"table row has {len(cells)} cells (limit {TABLE_COLUMN_LIMIT})")
                if position == 0 or is_separator_row(row):
                    continue
                for cell_index, cell in enumerate(cells):
                    count_sentences = sentence_count(cell)
                    if count_sentences > CELL_SENTENCE_LIMIT:
                        self.add(
                            rel,
                            number,
                            "cell-sentences",
                            f"table cell {cell_index + 1} has {count_sentences} sentences (limit {CELL_SENTENCE_LIMIT})",
                        )

    def check_internal_names(self, rel: str) -> None:
        doc = self.doc(rel)
        for index, line in enumerate(doc.visible):
            if not line.strip():
                continue
            occupied: list[tuple[int, int]] = []
            found: list[tuple[int, str]] = []
            for match in CITATION_RE.finditer(line):
                occupied.append(match.span())
                found.append((match.start(), f"internal name: {match.group(0)}"))
            for match in RS_WORD_RE.finditer(line):
                if any(start <= match.start() < end for start, end in occupied):
                    continue
                occupied.append(match.span())
                found.append((match.start(), f"internal name: {match.group(0)}"))
            for pattern, label in (
                (DOUBLE_COLON_RE, "::"),
                (CRATES_PATH_RE, "crates/"),
            ):
                for match in pattern.finditer(line):
                    if any(start <= match.start() < end for start, end in occupied):
                        continue
                    occupied.append(match.span())
                    found.append((match.start(), f"internal name: {label}"))
            for start, end in code_spans(line):
                for match in CRATE_CODE_RE.finditer(line[start:end]):
                    found.append((start + match.start(), f"internal name: {match.group(0)}"))
            for _, message in sorted(found):
                self.add(rel, index + 1, "internal-name", message)

    def check_banned_phrases(self, rel: str) -> None:
        doc = self.doc(rel)
        for index, line in enumerate(doc.visible):
            if not line.strip():
                continue
            occupied: list[tuple[int, int]] = []
            hits: list[tuple[int, str]] = []
            for phrase in BANNED_PHRASES:
                pattern = re.compile(WORD_BEFORE + re.escape(phrase) + WORD_AFTER, re.IGNORECASE)
                for match in pattern.finditer(line):
                    if any(start <= match.start() < end for start, end in occupied):
                        continue
                    occupied.append(match.span())
                    hits.append((match.start(), match.group(0)))
            for match in VERSION_FROM_RE.finditer(line):
                if any(start <= match.start() < end for start, end in occupied):
                    continue
                occupied.append(match.span())
                hits.append((match.start(), match.group(0)))
            for _, phrase in sorted(hits):
                self.add(rel, index + 1, "banned-phrase", f'banned phrase: "{phrase}"')

    def check_page_words(self, rel: str) -> None:
        doc = self.doc(rel)
        if rel in PAGE_WORDS_EXEMPT_PAGES:
            return
        for _, text in doc.headings:
            lowered = text.lower()
            if any(phrase in lowered for phrase in PAGE_WORDS_EXEMPT_HEADINGS):
                return
        words = 0
        for line in doc.visible:
            stripped = line.strip()
            if not stripped or heading_text(line) is not None or is_separator_row(line):
                continue
            words += count_words(line)
        if words > PAGE_WORD_LIMIT:
            self.add(rel, 1, "page-words", f"page has {words} words (limit {PAGE_WORD_LIMIT})")

    def check_section_words(self, rel: str) -> None:
        doc = self.doc(rel)
        section_line = 1
        words = 0

        def flush() -> None:
            if words > SECTION_WORD_LIMIT:
                self.add(
                    rel,
                    section_line,
                    "section-words",
                    f"section has {words} words of prose (limit {SECTION_WORD_LIMIT})",
                )

        for index, line in enumerate(doc.visible):
            if heading_text(line) is not None:
                flush()
                section_line = index + 1
                words = 0
                continue
            stripped = line.strip()
            if not stripped or stripped.startswith("|"):
                continue
            words += count_words(line)
        flush()

    def check_file_lines(self, rel: str) -> None:
        doc = self.doc(rel)
        if doc.line_count > FILE_LINE_LIMIT:
            self.add(rel, FILE_LINE_LIMIT + 1, "file-lines", f"file has {doc.line_count} lines (limit {FILE_LINE_LIMIT})")

    # -- warnings ------------------------------------------------------

    def check_glossary_avoid(self, rel: str, entries: list[str], glossary_rel: str) -> None:
        if rel == glossary_rel or not entries:
            return
        doc = self.doc(rel)
        patterns = [
            (entry, re.compile(WORD_BEFORE + re.escape(entry) + WORD_AFTER, re.IGNORECASE))
            for entry in entries
        ]
        for index, line in enumerate(doc.visible):
            stripped = line.strip()
            if not stripped or heading_text(line) is not None:
                continue
            text = mask_code_and_link_targets(line)
            text = BOLD_RE.sub(lambda match: " " * len(match.group(0)), text)
            occupied: list[tuple[int, int]] = []
            hits: list[tuple[int, str]] = []
            for entry, pattern in patterns:
                for match in pattern.finditer(text):
                    if any(start <= match.start() < end for start, end in occupied):
                        continue
                    occupied.append(match.span())
                    hits.append((match.start(), entry))
            for _, entry in sorted(hits):
                self.warn(rel, index + 1, "glossary-avoid", f'glossary avoid: use another word for "{entry}"')

    def check_now_word(self, rel: str) -> None:
        doc = self.doc(rel)
        for index, line in enumerate(doc.visible):
            stripped = line.strip()
            if not stripped or heading_text(line) is not None or stripped.startswith("|"):
                continue
            text = mask_code_and_link_targets(line)
            for _ in NOW_RE.finditer(text):
                self.warn(rel, index + 1, "now-word", 'do not write "now"')

    # -- driving -------------------------------------------------------

    def run(self) -> None:
        for rel in self.link_files:
            self.check_links(rel)
        for rel in self.style_files:
            self.check_sentences(rel)
            self.check_paragraphs(rel)
            self.check_tables(rel)
            self.check_internal_names(rel)
            if rel.startswith("docs/manual/"):
                self.check_page_words(rel)
            if rel.startswith(("docs/reference/", "docs/scripting/")):
                self.check_section_words(rel)
            self.check_file_lines(rel)
        for rel in self.style_files:
            if rel.startswith("docs/manual/"):
                self.check_banned_phrases(rel)
        glossary_rel = "docs/manual/glossary.md"
        entries = glossary_entries(self.root, glossary_rel)
        for rel in self.style_files:
            self.check_glossary_avoid(rel, entries, glossary_rel)
        for rel in self.style_files:
            if rel.startswith("docs/manual/"):
                self.check_now_word(rel)

    def checked_files(self) -> list[str]:
        return sorted(set(self.link_files) | set(self.style_files))


def glossary_entries(root: Path, rel: str) -> list[str]:
    """Read the "Do not use" column of every table in the glossary."""
    try:
        doc = Doc(root, rel)
    except SetupError:
        return []
    entries: list[str] = []
    seen: set[str] = set()

    def add_entry(cell: str) -> None:
        for part in cell.split(","):
            entry = part.strip()
            if entry and entry not in seen:
                seen.add(entry)
                entries.append(entry)

    index = 0
    count = len(doc.visible)
    while index < count:
        if not doc.visible[index].strip().startswith("|"):
            index += 1
            continue
        rows: list[str] = []
        while index < count and doc.visible[index].strip().startswith("|"):
            rows.append(doc.visible[index])
            index += 1
        header = [cell.strip() for cell in table_cells(rows[0])]
        if "Do not use" not in header:
            continue
        column = header.index("Do not use")
        for row in rows[1:]:
            if is_separator_row(row):
                continue
            cells = table_cells(row)
            if column < len(cells):
                add_entry(cells[column])
    return entries


def count_findings(findings: list[tuple[str, int, str, str]]) -> dict[str, int]:
    counts: dict[str, int] = {}
    for rel, _, rule, _ in findings:
        key = f"{rel}|{rule}"
        counts[key] = counts.get(key, 0) + 1
    return counts


def split_key(key: str) -> tuple[str, str]:
    path, _, rule = key.rpartition("|")
    return path, rule


def load_baseline(path: Path) -> dict[str, int]:
    if not path.exists():
        return {}
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise SetupError(f"{path}: cannot read baseline ({exc})") from exc
    if not isinstance(data, dict) or not all(
        isinstance(key, str) and isinstance(value, int) for key, value in data.items()
    ):
        raise SetupError(f"{path}: baseline must map string keys to integer counts")
    return data


def write_baseline(path: Path, counts: dict[str, int]) -> None:
    data = {key: counts[key] for key in sorted(counts) if counts[key] > 0}
    path.write_text(json.dumps(data, indent=2, ensure_ascii=True) + "\n", encoding="utf-8")


def find_root() -> Path:
    try:
        result = subprocess.run(
            ["git", "-C", str(SCRIPT_DIR), "rev-parse", "--show-toplevel"],
            capture_output=True,
            text=True,
            check=False,
        )
    except OSError as exc:
        raise SetupError(f"cannot run git ({exc})") from exc
    if result.returncode != 0:
        raise SetupError(f"cannot find the repository root from {SCRIPT_DIR}: {result.stderr.strip()}")
    return Path(result.stdout.strip()).resolve()


def tracked_markdown(root: Path) -> list[str]:
    try:
        result = subprocess.run(
            ["git", "-C", str(root), "ls-files", "-z"],
            capture_output=True,
            check=False,
        )
    except OSError as exc:
        raise SetupError(f"cannot run git ({exc})") from exc
    if result.returncode != 0:
        raise SetupError(f"git ls-files failed in {root}: {result.stderr.decode('utf-8', 'replace').strip()}")
    paths = result.stdout.split(b"\0")
    return [path.decode("utf-8", "surrogateescape") for path in paths if path]


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="Apply the documentation writing rules and compare with the baseline.")
    parser.add_argument("--root", type=Path, default=None, help="checkout root (default: this script's repository)")
    parser.add_argument("--update", action="store_true", help="rewrite the baseline from the current counts")
    args = parser.parse_args(argv)

    try:
        root = args.root.resolve() if args.root else find_root()
        tracked = tracked_markdown(root)
        checker = Checker(root, tracked)
        checker.run()
        findings = sorted(checker.findings, key=lambda item: (item[0], item[1], item[2], item[3]))
        warnings = sorted(checker.warnings, key=lambda item: (item[0], item[1], item[2], item[3]))
        counts = count_findings(findings)
        if args.update:
            write_baseline(DEFAULT_BASELINE, counts)
            baseline: dict[str, int] = {}
        else:
            baseline = load_baseline(DEFAULT_BASELINE)
    except SetupError as exc:
        print(f"docs-check: error: {exc}", file=sys.stderr)
        return 2

    for rel, line, rule, message in findings:
        print(f"{rel}:{line}: {rule}: {message}")
    for rel, line, rule, message in warnings:
        print(f"{rel}:{line}: {rule}: {message}")

    over: list[tuple[str, str, int, int]] = []
    shrink: list[tuple[str, str, int, int]] = []
    if not args.update:
        for key in sorted(set(counts) | set(baseline)):
            current = counts.get(key, 0)
            recorded = baseline.get(key, 0)
            path, rule = split_key(key)
            if current > recorded:
                over.append((path, rule, current, recorded))
            elif current < recorded:
                shrink.append((path, rule, current, recorded))

    for path, rule, current, recorded in over:
        print(f"over baseline: {path} {rule} {current} > {recorded}")
    for path, rule, current, recorded in shrink:
        print(f"baseline can shrink: {path} {rule} {current} < {recorded}")

    excess = sum(current - recorded for _, _, current, recorded in over)
    print(
        f"docs-check: {excess} finding(s) over baseline, "
        f"{len(warnings)} warning(s), {len(checker.checked_files())} file(s) checked"
    )
    return 1 if over else 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
