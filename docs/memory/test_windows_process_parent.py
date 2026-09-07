#!/usr/bin/env python3
"""Deterministic tests for Windows explicit-PID parent lookup."""
from __future__ import annotations

import ctypes
import pathlib
import sys
import unittest
from typing import Any, List
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))

import server_resources as sr  # noqa: E402
import windows_process_parent as wpp  # noqa: E402


class FakeToolhelp:
    def __init__(self, rows: List[tuple[int, int]]) -> None:
        self.rows = list(rows)
        self.snap = 0xBEEF
        self.closed: List[Any] = []
        self.index = -1
        self.last_error = 0
        self.fail_first = False
        self.fail_snap = False
        self.images = {}

    def CreateToolhelp32Snapshot(self, flags, pid):
        assert flags == wpp.TH32CS_SNAPPROCESS
        assert pid == 0
        if self.fail_snap:
            self.last_error = 5
            return 0
        return self.snap

    def Process32FirstW(self, handle, entry_p):
        assert handle == self.snap
        if self.fail_first:
            self.last_error = 18
            return False
        self.index = 0
        return self._store(entry_p)

    def Process32NextW(self, handle, entry_p):
        assert handle == self.snap
        self.index += 1
        if self.index >= len(self.rows):
            self.last_error = 18
            return False
        return self._store(entry_p)

    def _store(self, entry_p) -> bool:
        if self.index < 0 or self.index >= len(self.rows):
            return False
        pid, parent = self.rows[self.index]
        entry = ctypes.cast(entry_p, ctypes.POINTER(wpp.PROCESSENTRY32W)).contents
        entry.th32ProcessID = pid
        entry.th32ParentProcessID = parent
        entry.szExeFile = self.images.get(pid, "python.exe")
        return True

    def CloseHandle(self, handle):
        self.closed.append(handle)
        return True

    def GetLastError(self):
        return int(self.last_error)


class WindowsParentTests(unittest.TestCase):
    def test_parent_found_and_handle_closed(self):
        fake = FakeToolhelp([(10, 1), (42, 7), (99, 42)])
        parent = wpp.parent_pid(
            42,
            create_snapshot=fake.CreateToolhelp32Snapshot,
            process_first=fake.Process32FirstW,
            process_next=fake.Process32NextW,
            close_handle=fake.CloseHandle,
            get_last_error=fake.GetLastError,
        )
        self.assertEqual(parent, 7)
        self.assertEqual(fake.closed, [fake.snap])

    def test_missing_pid_fail_closed(self):
        fake = FakeToolhelp([(1, 0), (2, 1)])
        with self.assertRaises(sr.SampleError) as ctx:
            wpp.parent_pid(
                404,
                create_snapshot=fake.CreateToolhelp32Snapshot,
                process_first=fake.Process32FirstW,
                process_next=fake.Process32NextW,
                close_handle=fake.CloseHandle,
                get_last_error=fake.GetLastError,
            )
        self.assertIn("not found", str(ctx.exception).lower())
        self.assertEqual(fake.closed, [fake.snap])

    def test_invalid_pid_rejected(self):
        with self.assertRaises(sr.SampleError):
            wpp.parent_pid(0, create_snapshot=lambda *a: 1, process_first=lambda *a: False,
                           process_next=lambda *a: False, close_handle=lambda *a: True,
                           get_last_error=lambda: 0)
        with self.assertRaises(sr.SampleError):
            wpp.parent_pid(True)  # type: ignore[arg-type]

    def test_snapshot_failure_fail_closed(self):
        fake = FakeToolhelp([])
        fake.fail_snap = True
        with self.assertRaises(sr.SampleError):
            wpp.parent_pid(
                1,
                create_snapshot=fake.CreateToolhelp32Snapshot,
                process_first=fake.Process32FirstW,
                process_next=fake.Process32NextW,
                close_handle=fake.CloseHandle,
                get_last_error=fake.GetLastError,
            )

    def test_child_processes_returns_direct_children_with_images(self):
        fake = FakeToolhelp([(10, 1), (42, 7), (99, 42), (100, 42)])
        fake.images.update({42: "tui.exe", 99: "conhost.exe", 100: "other.exe"})
        children = wpp.child_processes(42, api=fake)
        self.assertEqual(children, [
            {"pid": 99, "parent_pid": 42, "image_name": "conhost.exe"},
            {"pid": 100, "parent_pid": 42, "image_name": "other.exe"},
        ])
        self.assertEqual(fake.closed, [fake.snap])

    def test_select_conpty_helpers_excludes_frontend_and_non_conhost(self):
        children = [
            {"pid": 10, "image_name": "conhost.exe"},
            {"pid": 20, "image_name": "ConHost.EXE"},
            {"pid": 30, "image_name": "python.exe"},
        ]
        self.assertEqual(wpp.select_conpty_helpers(children, frontend_pid=10), [
            {"pid": 20, "image_name": "ConHost.EXE"},
        ])
        self.assertEqual(wpp.select_conpty_helpers(children, frontend_pid=999), children[:2])


if __name__ == "__main__":
    unittest.main()
