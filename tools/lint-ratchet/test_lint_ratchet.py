#!/usr/bin/env python3
"""Permanent parser tests for `lint_ratchet.py`.

Covers the round-2 review plants (`parser-plant/`, `slip-only/`): every
suppression form the ratchet must count, every decoy it must ignore, and
the fail-closed rulings (unparseable means error, exit non-zero).

Two ways to run (both stdlib only, no Rust toolchain):

  python3 -B tools/lint-ratchet/lint_ratchet.py --self-test
  python3 -B tools/lint-ratchet/test_lint_ratchet.py
"""

import contextlib
import io
import json
import sys
import tempfile
import unittest
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import lint_ratchet as ratchet

EMPTY_BASELINE = {"version": 1, "counts": {}}


def parse(inner):
    """Run the attribute parser; return (lints, problems)."""
    problems: list[str] = []
    return ratchet.lints_in_attribute(inner, problems, "f.rs:1"), problems


def make_tree(files, baseline_counts=None):
    """Create a temp workspace root; return (root, baseline_path)."""
    tmp = tempfile.TemporaryDirectory(prefix="lint-ratchet-test-")
    root = Path(tmp.name)
    for rel, content in files.items():
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
    baseline_path = root / "baseline.json"
    counts = {} if baseline_counts is None else baseline_counts
    baseline_path.write_text(
        json.dumps({"version": 1, "counts": counts}), encoding="utf-8"
    )
    return tmp, root, baseline_path


def run_check(files, baseline_counts=None):
    """End-to-end `cmd_check` on a temp tree; return (exit, stdout, stderr)."""
    tmp, root, baseline_path = make_tree(files, baseline_counts)
    with tmp, contextlib.redirect_stdout(io.StringIO()) as out, contextlib.redirect_stderr(
        io.StringIO()
    ) as err:
        exit_code = ratchet.cmd_check(root, baseline_path)
    return exit_code, out.getvalue(), err.getvalue()


def run_update(files, baseline_counts=None):
    """End-to-end `cmd_update`; return (exit, stdout, stderr, baseline_text)."""
    tmp, root, baseline_path = make_tree(files, baseline_counts)
    with tmp, contextlib.redirect_stdout(io.StringIO()) as out, contextlib.redirect_stderr(
        io.StringIO()
    ) as err:
        exit_code = ratchet.cmd_update(root, baseline_path)
        text = baseline_path.read_text(encoding="utf-8")
    return exit_code, out.getvalue(), err.getvalue(), text


def scan_hits(source, rel="crates/probe/src/lib.rs"):
    """Scan one source string; return (hits, problems)."""
    tmp, root, _ = make_tree({rel: source})
    with tmp:
        problems: list[str] = []
        hits = ratchet.scan_tree(root, problems)
    return hits, problems


class SpacingAndNesting(unittest.TestCase):
    def test_space_before_paren_in_cfg_attr_counts(self):
        lints, problems = parse("cfg_attr(all(), allow (dead_code))")
        self.assertEqual(lints, ["dead_code"])
        self.assertEqual(problems, [])

    def test_space_before_paren_for_expect_in_cfg_attr(self):
        lints, problems = parse("cfg_attr(test, expect (unused))")
        self.assertEqual(lints, ["unused"])
        self.assertEqual(problems, [])

    def test_spaced_path_normalized(self):
        lints, problems = parse("allow(clippy :: needless_collect)")
        self.assertEqual(lints, ["clippy::needless_collect"])
        self.assertEqual(problems, [])

    def test_nested_cfg_attr_counts_to_any_depth(self):
        lints, problems = parse("cfg_attr(all(), cfg_attr(all(), allow(dead_code)))")
        self.assertEqual(lints, ["dead_code"])
        self.assertEqual(problems, [])
        deep = "cfg_attr(a, cfg_attr(b, cfg_attr(c, expect(unused_variables))))"
        lints, problems = parse(deep)
        self.assertEqual(lints, ["unused_variables"])
        self.assertEqual(problems, [])

    def test_two_suppression_attrs_in_one_cfg_attr(self):
        # End to end: the `reason` string is blanked by the scanner before
        # parsing, so its commas and parens never split the lint list.
        hits, problems = scan_hits(
            "#[cfg_attr(\n"
            "    test,\n"
            "    allow(unused_labels),\n"
            '    expect(unused_macros, reason = "second attr, with ) and ,")\n'
            ")]\n"
            "fn two_cfg_attrs() {}\n"
        )
        self.assertEqual(problems, [])
        self.assertEqual(
            Counter(lint for _, lint, _, _, _ in hits),
            Counter({"unused_labels": 1, "unused_macros": 1}),
        )

    def test_unrelated_cfg_attr_item_ignored(self):
        lints, problems = parse("cfg_attr(test, derive(Debug))")
        self.assertEqual(lints, [])
        self.assertEqual(problems, [])

    def test_lookalike_names_not_matched(self):
        for body in ("allowance(foo)", "expected(foo)", "cfg_attr_extra(a, allow(dead_code))"):
            lints, problems = parse(body)
            self.assertEqual(lints, [], body)
            self.assertEqual(problems, [], body)

    def test_cfg_predicate_with_parens_and_commas(self):
        hits, problems = scan_hits(
            "#[cfg_attr(\n"
            '    all(any(unix, windows), not(feature = "nope")),\n'
            "    expect(\n"
            "        clippy::too_many_arguments,\n"
            '        reason = "predicate had (parens), commas; reason does too"\n'
            "    )\n"
            ")]\n"
            "fn cfg_predicate() {}\n"
        )
        self.assertEqual(problems, [])
        self.assertEqual(
            Counter(lint for _, lint, _, _, _ in hits),
            Counter({"clippy::too_many_arguments": 1}),
        )

    def test_reason_with_parens_commas_and_escaped_quote(self):
        hits, problems = scan_hits(
            "#[expect(\n"
            "    unused_variables,\n"
            '    reason = "multi-line (parens), commas, and \\"escaped\\" quote"\n'
            ")]\n"
            "fn multi_line_expect() {}\n"
        )
        self.assertEqual(problems, [])
        self.assertEqual(
            Counter(lint for _, lint, _, _, _ in hits),
            Counter({"unused_variables": 1}),
        )


class StringsCommentsAndDecoys(unittest.TestCase):
    STRINGS = """\
const COOKED: &str = "#[allow(dead_code)] #[expect(unused)] #![allow(unused_imports)]";
const COOKED_ESC: &str = "say \\"#[allow(dead_code)]\\" and ) , stay text";
const RAW: &str = r#"#[allow(clippy::needless_collect)] and ) , "#;
const RAW_HASH: &str = r##"#[allow(dead_code, reason = "a) b, c")]"##;
const BYTE: &[u8] = b"#[allow(unused_variables)]";
const BYTE_RAW: &[u8] = br#"#[allow(unused_mut)]"#;
const CSTR: &std::ffi::CStr = c"#[allow(clippy::redundant_clone)]";
fn real() {}
"""

    DECOYS = """\
fn decoys() {
    let text = "ok";
    let _ = text.chars().next().expect("#[allow(dead_code)]");
    let hash = '#';
    let byte_hash = b'#';
    let quote = '\\'';
    let _ = (hash, byte_hash, quote);
    // #[allow(dead_code)]
    /* #[allow(unused_imports)] */
    /* outer /* #[expect(unused_variables)] */ still a comment */
    let value = 1;
    let _r = &value;
}
"""

    def test_string_literals_not_counted(self):
        hits, problems = scan_hits(self.STRINGS)
        self.assertEqual(hits, [])
        self.assertEqual(problems, [])

    def test_comments_and_decoys_not_counted(self):
        hits, problems = scan_hits(self.DECOYS)
        self.assertEqual(hits, [])
        self.assertEqual(problems, [])

    def test_bare_and_empty_allow_invent_nothing(self):
        hits, problems = scan_hits(
            "#[allow]\nfn a() {}\n\n#[allow()]\nfn b() {}\n"
        )
        self.assertEqual(hits, [])
        self.assertEqual(problems, [])


class MacroBodiesAreCounted(unittest.TestCase):
    # Round-1 spec said macro bodies must not count; round-2 ruling 4
    # reversed that: macro-emitted suppressions are real, and over-counting
    # hides nothing, so `#[allow]` inside macro bodies and macro arguments
    # is counted, conservatively.

    MACROS = """\
macro_rules! emit_allow {
    () => {
        #[allow(clippy::assigning_clones)]
        fn from_macro() {}
    };
}

macro_rules! match_allow {
    (#[allow($lint:meta)]) => {
        stringify!($lint)
    };
}

fn macro_invocation_body() {
    let _ = stringify!(#[allow(dead_code)]);
    let _ = emit_allow!();
    let _ = match_allow!(#[allow(unused)]);
}
"""

    def test_macro_bodies_and_arguments_counted(self):
        # The `$lint:meta` matcher line is a parse error (see below), so
        # check the three counted lines without it.
        source = self.MACROS.replace("    (#[allow($lint:meta)]) => {\n", "    ($lint:meta) => {\n")
        hits, problems = scan_hits(source)
        self.assertEqual(problems, [])
        self.assertEqual(
            Counter(lint for _, lint, _, _, _ in hits),
            Counter({"clippy::assigning_clones": 1, "dead_code": 1, "unused": 1}),
        )

    def test_macro_matcher_metavariable_is_an_error(self):
        hits, problems = scan_hits("macro_rules! m {\n (#[allow($lint:meta)]) => {};\n}\n")
        self.assertEqual(hits, [])
        self.assertEqual(len(problems), 1)
        self.assertIn("cannot parse suppression item", problems[0])
        exit_code, out, err = run_check(
            {"crates/probe/src/lib.rs": "macro_rules! m {\n (#[allow($lint:meta)]) => {};\n}\n"}
        )
        self.assertEqual(exit_code, 1)
        self.assertIn("FAIL", out)


class FailClosed(unittest.TestCase):
    def test_unparseable_item_is_an_error(self):
        exit_code, out, err = run_check(
            {"crates/probe/src/lib.rs": "#[allow(not-a-lint)]\nfn a() {}\n"}
        )
        self.assertEqual(exit_code, 1)
        self.assertIn("FAIL", out)
        self.assertIn("crates/probe/src/lib.rs:1", err)
        self.assertIn("cannot parse suppression item", err)

    def test_unbalanced_attribute_is_an_error(self):
        exit_code, out, err = run_check(
            {"crates/probe/src/lib.rs": "#[allow(dead_code)\nfn a() {}\n"}
        )
        self.assertEqual(exit_code, 1)
        self.assertIn("FAIL", out)
        self.assertIn("crates/probe/src/lib.rs:1", err)

    def test_slip_only_tree_fails(self):
        # The review's slip-only tree: three real suppressions the round-1
        # parser missed. Against an empty baseline each must fail the check.
        source = (
            "#[cfg_attr(all(), allow (dead_code))]\n"
            "fn space_before_paren_slips() {}\n"
            "#[cfg_attr(all(), cfg_attr(all(), allow(dead_code)))]\n"
            "fn nested_cfg_attr_slips() {}\n"
            "#[allow(clippy :: needless_collect)]\n"
            "fn spaced_colon_slips() {}\n"
        )
        exit_code, out, err = run_check({"crates/probe/src/lib.rs": source})
        self.assertEqual(exit_code, 1)
        self.assertEqual(err, "")
        self.assertIn("FAIL (2 problem(s))", out)
        self.assertIn("[probe] clippy::needless_collect [non_test]: new lint not in baseline", out)
        self.assertIn("[probe] dead_code [non_test]: new lint not in baseline (2 occurrence(s))", out)
        self.assertIn("crates/probe/src/lib.rs:1", out)
        self.assertIn("crates/probe/src/lib.rs:3", out)
        self.assertIn("crates/probe/src/lib.rs:5", out)

    def test_update_refuses_with_parse_errors(self):
        exit_code, out, err, text = run_update(
            {"crates/probe/src/lib.rs": "#[allow(not-a-lint)]\nfn a() {}\n"}
        )
        self.assertEqual(exit_code, 1)
        self.assertIn("cannot parse suppression item", err)
        self.assertIn("refusing to write", out)
        self.assertEqual(json.loads(text), EMPTY_BASELINE)

    def test_clean_tree_passes_and_reports_counts(self):
        exit_code, out, err = run_check(
            {"crates/probe/src/lib.rs": "#[allow(dead_code)]\nfn a() {}\n"},
            baseline_counts={"probe": {"dead_code": {"non_test": 1, "test": 0}}},
        )
        self.assertEqual(exit_code, 0)
        self.assertIn("OK (1 suppression(s) across 1 crate(s))", out)
        self.assertEqual(err, "")


class BucketsAndInnerAttributes(unittest.TestCase):
    def test_inner_attributes_in_modules_count(self):
        hits, problems = scan_hits(
            "mod nested {\n"
            "    #![allow(unused_mut, reason = \"inner module reason has ) and , too\")]\n"
            "    fn inner() {}\n"
            "    mod deeper {\n"
            "        #![expect(\n"
            "            unused_assignments,\n"
            '            reason = "deeper module (still), a suppression"\n'
            "        )]\n"
            "        fn deeper_fn() {}\n"
            "    }\n"
            "}\n"
        )
        self.assertEqual(problems, [])
        self.assertEqual(
            Counter(lint for _, lint, _, _, _ in hits),
            Counter({"unused_mut": 1, "unused_assignments": 1}),
        )

    def test_tests_dir_counts_test_bucket(self):
        hits, problems = scan_hits(
            '#[allow(dead_code, reason = "test-bucket reason has ) and ,")]\nfn t() {}\n',
            rel="crates/probe/tests/smoke.rs",
        )
        self.assertEqual(problems, [])
        self.assertEqual(
            [(crate, lint, bucket) for crate, lint, bucket, _, _ in hits],
            [("probe", "dead_code", "test")],
        )


if __name__ == "__main__":
    unittest.main()
