#!/usr/bin/env python3
"""Unit tests for operator_home (HOME / USERPROFILE parity with Rust)."""
from __future__ import annotations

import pathlib
import unittest

import operator_home as oh


class OperatorHomeFrom(unittest.TestCase):
    def test_windows_explicit_home_wins_including_empty(self):
        self.assertEqual(
            oh.operator_home_from("/explicit", "/profile", windows=True),
            "/explicit",
        )
        self.assertEqual(
            oh.operator_home_from("", "/profile", windows=True),
            "",
        )

    def test_windows_userprofile_when_home_absent(self):
        self.assertEqual(
            oh.operator_home_from(None, r"C:\Users\op", windows=True),
            r"C:\Users\op",
        )
        self.assertIsNone(oh.operator_home_from(None, None, windows=True))

    def test_non_windows_ignores_userprofile(self):
        self.assertEqual(
            oh.operator_home_from("/unix", "/ignored", windows=False),
            "/unix",
        )
        self.assertEqual(oh.operator_home_from("", "/ignored", windows=False), "")
        self.assertIsNone(oh.operator_home_from(None, "/ignored", windows=False))
        self.assertIsNone(oh.operator_home_from(None, None, windows=False))

    def test_windows_never_reads_profile_when_home_key_present(self):
        # Presence of empty HOME must not fall through to USERPROFILE.
        env = {"HOME": "", "USERPROFILE": r"C:\Users\op"}
        self.assertEqual(oh.operator_home(environ=env, windows=True), "")
        self.assertEqual(oh.bot_home_path(environ=env, windows=True), ".")

    def test_bot_home_path_nonempty_and_fallback(self):
        self.assertEqual(
            oh.bot_home_path(environ={"HOME": "/h"}, windows=False),
            "/h",
        )
        self.assertEqual(oh.bot_home_path(environ={}, windows=False), ".")
        self.assertEqual(
            oh.bot_home_path(
                environ={"USERPROFILE": r"C:\Users\op"},
                windows=True,
            ),
            r"C:\Users\op",
        )

    def test_unpack_root_matches_rust_empty_and_profile(self):
        self.assertEqual(
            oh.unpack_root_path(environ={"HOME": "/h"}, windows=False),
            pathlib.Path("/h") / ".274bot" / "unpack",
        )
        cwd = pathlib.Path("/tmp/cell-cwd")
        self.assertEqual(
            oh.unpack_root_path(cwd=cwd, environ={"HOME": ""}, windows=True),
            cwd / ".274bot" / "unpack",
        )
        self.assertEqual(
            oh.unpack_root_path(
                cwd=cwd,
                environ={"USERPROFILE": r"C:\Users\op"},
                windows=True,
            ),
            pathlib.Path(r"C:\Users\op") / ".274bot" / "unpack",
        )
        # Non-Windows must not use USERPROFILE when HOME absent.
        self.assertEqual(
            oh.unpack_root_path(
                cwd=cwd,
                environ={"USERPROFILE": r"C:\Users\op"},
                windows=False,
            ),
            cwd / ".274bot" / "unpack",
        )


if __name__ == "__main__":
    unittest.main()
