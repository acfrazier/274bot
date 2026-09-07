#!/usr/bin/env python3
"""Deterministic and (Windows-only) live tests for Win32 explicit-PID sampling."""
from __future__ import annotations

import ctypes
import os
import pathlib
import sys
import time
import unittest
from types import SimpleNamespace
from typing import Any, List
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))

import server_resources as sr  # noqa: E402
import windows_process_sample as wps  # noqa: E402


def _ft(high: int, low: int) -> wps.FILETIME:
    return wps.FILETIME(dwLowDateTime=low & 0xFFFFFFFF, dwHighDateTime=high & 0xFFFFFFFF)


class FakeWin32:
    """Injectable Win32 surface for structure/failure/handle-close cases."""

    def __init__(self) -> None:
        self.handle = 0xABC0
        self.open_access = None
        self.open_pid = None
        self.closed: List[Any] = []
        self.wait_rc = wps.WAIT_TIMEOUT
        self.times_ok = True
        self.mem_ok = True
        self.close_ok = True
        self.last_error = 0
        self.creation = _ft(1, 2)
        self.exit_t = _ft(0, 0)
        self.kernel = _ft(0, 5_000_000)  # 0.5 s
        self.user = _ft(0, 10_000_000)  # 1.0 s
        self.working_set = 4096
        self.peak_working_set = 8192
        self.pagefile = 99999
        self.open_returns_null = False

    def OpenProcess(self, access, inherit, pid):
        self.open_access = int(access)
        self.open_pid = int(pid)
        if self.open_returns_null:
            return 0
        return self.handle

    def CloseHandle(self, handle):
        self.closed.append(handle)
        return bool(self.close_ok)

    def GetLastError(self):
        return int(self.last_error)

    def WaitForSingleObject(self, handle, timeout_ms):
        assert handle == self.handle
        assert int(timeout_ms) == 0
        return int(self.wait_rc)

    def GetProcessTimes(self, handle, creation_p, exit_p, kernel_p, user_p):
        assert handle == self.handle
        if not self.times_ok:
            return False

        def _store(ptr, value: wps.FILETIME) -> None:
            target = ctypes.cast(ptr, ctypes.POINTER(wps.FILETIME)).contents
            target.dwLowDateTime = value.dwLowDateTime
            target.dwHighDateTime = value.dwHighDateTime

        _store(creation_p, self.creation)
        _store(exit_p, self.exit_t)
        _store(kernel_p, self.kernel)
        _store(user_p, self.user)
        return True

    def GetProcessMemoryInfo(self, handle, pmc_p, cb):
        assert handle == self.handle
        if not self.mem_ok:
            return False
        pmc = ctypes.cast(pmc_p, ctypes.POINTER(wps.PROCESS_MEMORY_COUNTERS)).contents
        pmc.WorkingSetSize = int(self.working_set)
        pmc.PeakWorkingSetSize = int(self.peak_working_set)
        pmc.PagefileUsage = int(self.pagefile)
        return True


class FiletimeAndLayoutTests(unittest.TestCase):
    def test_filetime_parts_to_seconds(self):
        self.assertEqual(wps.filetime_parts_to_seconds(0, 0), 0.0)
        self.assertEqual(wps.filetime_parts_to_seconds(0, 10_000_000), 1.0)
        self.assertEqual(wps.filetime_parts_to_seconds(0, 5_000_000), 0.5)
        # high dword: 2^32 ticks / 1e7 s
        self.assertEqual(wps.filetime_parts_to_seconds(1, 0), (1 << 32) / 10_000_000.0)

    def test_filetime_parts_reject_bad_types(self):
        with self.assertRaises(sr.SampleError):
            wps.filetime_parts_to_seconds(0.0, 0)  # type: ignore[arg-type]
        with self.assertRaises(sr.SampleError):
            wps.filetime_parts_to_seconds(-1, 0)

    def test_structure_field_widths(self):
        self.assertEqual(ctypes.sizeof(wps.FILETIME), 8)
        self.assertEqual(wps.FILETIME.dwLowDateTime.offset, 0)
        self.assertEqual(wps.FILETIME.dwHighDateTime.offset, 4)
        # PROCESS_MEMORY_COUNTERS: 2*DWORD + 8*SIZE_T
        expected = 2 * ctypes.sizeof(ctypes.c_uint32) + 8 * ctypes.sizeof(ctypes.c_size_t)
        self.assertEqual(ctypes.sizeof(wps.PROCESS_MEMORY_COUNTERS), expected)
        self.assertEqual(
            wps.PROCESS_MEMORY_COUNTERS.WorkingSetSize.offset,
            2 * ctypes.sizeof(ctypes.c_uint32) + ctypes.sizeof(ctypes.c_size_t),
        )

    def test_creation_identity_stable(self):
        ft = _ft(0x11, 0x22)
        self.assertEqual(wps.creation_identity(ft), "windows_creation_filetime:" + str((0x11 << 32) | 0x22))


class InjectedSampleTests(unittest.TestCase):
    def test_happy_path_uses_working_set_not_peak_or_pagefile(self):
        fake = FakeWin32()
        sample = wps.sample_process(4242, api=fake)
        self.assertEqual(sample["resident_bytes"], 4096)
        self.assertNotEqual(sample["resident_bytes"], fake.peak_working_set)
        self.assertNotEqual(sample["resident_bytes"], fake.pagefile)
        self.assertAlmostEqual(sample["user_s"], 1.0)
        self.assertAlmostEqual(sample["system_s"], 0.5)
        self.assertEqual(
            sample["start_identity"],
            "windows_creation_filetime:" + str((1 << 32) | 2),
        )
        self.assertEqual(fake.open_access, wps.PROCESS_ACCESS)
        self.assertEqual(fake.open_pid, 4242)
        self.assertEqual(fake.closed, [fake.handle])
        self.assertEqual(sample["provenance"]["os"], "windows")
        self.assertFalse(sample["provenance"]["sampling_subprocesses"])
        self.assertFalse(sample["provenance"]["name_argv_env_discovery"])
        self.assertIn("WorkingSetSize", sample["provenance"]["rss"])
        self.assertNotIn("PeakWorkingSetSize)", sample["provenance"]["rss"])

    def test_same_identity_and_counter_reset_invariants(self):
        fake = FakeWin32()
        first = wps.sample_process(7, api=fake)
        fake.user = _ft(0, 20_000_000)
        fake.kernel = _ft(0, 6_000_000)
        fake.working_set = 8192
        second = wps.sample_process(7, api=fake)
        sr.require_same_identity(first, second)
        delta = sr.cpu_delta_seconds(first, second)
        self.assertAlmostEqual(delta["user_s"], 1.0)
        self.assertAlmostEqual(delta["system_s"], 0.1)
        # Counter reset
        with self.assertRaises(sr.SampleError):
            sr.cpu_delta_seconds(second, first)
        # Identity change
        fake.creation = _ft(9, 9)
        third = wps.sample_process(7, api=fake)
        with self.assertRaises(sr.SampleError):
            sr.require_same_identity(second, third)

    def test_exited_process_via_wait_not_still_active(self):
        fake = FakeWin32()
        fake.wait_rc = wps.WAIT_OBJECT_0
        with self.assertRaises(sr.SampleError) as ctx:
            wps.sample_process(9, api=fake)
        self.assertIn("exited", str(ctx.exception))
        self.assertEqual(fake.closed, [fake.handle])

    def test_process_is_alive_running_and_exited(self):
        fake = FakeWin32()
        fake.wait_rc = wps.WAIT_TIMEOUT
        self.assertTrue(wps.process_is_alive(4242, api=fake))
        self.assertEqual(fake.open_access, wps._PROCESS_ALIVE_ACCESS)
        self.assertEqual(fake.closed, [fake.handle])

        fake2 = FakeWin32()
        fake2.wait_rc = wps.WAIT_OBJECT_0
        self.assertFalse(wps.process_is_alive(9, api=fake2))
        self.assertEqual(fake2.closed, [fake2.handle])

    def test_process_is_alive_missing_and_access_denied(self):
        fake = FakeWin32()
        fake.open_returns_null = True
        fake.last_error = wps.ERROR_INVALID_PARAMETER
        self.assertFalse(wps.process_is_alive(999_999_991, api=fake))
        self.assertEqual(fake.closed, [])

        fake2 = FakeWin32()
        fake2.open_returns_null = True
        fake2.last_error = wps.ERROR_ACCESS_DENIED
        # Cannot prove dead → treat as present.
        self.assertTrue(wps.process_is_alive(4, api=fake2))

        self.assertFalse(wps.process_is_alive(0, api=FakeWin32()))
        self.assertFalse(wps.process_is_alive(-1, api=FakeWin32()))

    def test_missing_pid_open_failure(self):
        fake = FakeWin32()
        fake.open_returns_null = True
        fake.last_error = wps.ERROR_INVALID_PARAMETER
        with self.assertRaises(sr.SampleError) as ctx:
            wps.sample_process(999_999_991, api=fake)
        self.assertIn("not found", str(ctx.exception).lower())
        self.assertEqual(fake.closed, [])

    def test_access_denied(self):
        fake = FakeWin32()
        fake.open_returns_null = True
        fake.last_error = wps.ERROR_ACCESS_DENIED
        with self.assertRaises(sr.SampleError) as ctx:
            wps.sample_process(4, api=fake)
        self.assertIn("access denied", str(ctx.exception).lower())

    def test_api_failure_times_and_memory(self):
        fake = FakeWin32()
        fake.times_ok = False
        fake.last_error = 42
        with self.assertRaises(sr.SampleError):
            wps.sample_process(1, api=fake)
        self.assertEqual(fake.closed, [fake.handle])

        fake2 = FakeWin32()
        fake2.mem_ok = False
        with self.assertRaises(sr.SampleError):
            wps.sample_process(1, api=fake2)
        self.assertEqual(fake2.closed, [fake2.handle])

    def test_zero_resident_rejected(self):
        fake = FakeWin32()
        fake.working_set = 0
        with self.assertRaises(sr.SampleError) as ctx:
            wps.sample_process(1, api=fake)
        self.assertIn("zero", str(ctx.exception).lower())
        self.assertEqual(fake.closed, [fake.handle])

    def test_handle_closed_even_when_close_fails_after_success(self):
        fake = FakeWin32()
        fake.close_ok = False
        fake.last_error = 6
        with self.assertRaises(sr.SampleError) as ctx:
            wps.sample_process(1, api=fake)
        self.assertIn("CloseHandle", str(ctx.exception))
        self.assertEqual(fake.closed, [fake.handle])

    def test_bad_pid_rejected_before_open(self):
        fake = FakeWin32()
        for pid in (0, -1, True, 1.5, 2**33):
            with self.assertRaises(sr.SampleError):
                wps.sample_process(pid, api=fake)  # type: ignore[arg-type]
        self.assertIsNone(fake.open_pid)

    def test_no_silent_backend_switch_on_non_windows_without_inject(self):
        if sys.platform == "win32":
            self.skipTest("live win32 uses real API")
        with self.assertRaises(sr.SampleError) as ctx:
            wps.sample_process(os.getpid())
        self.assertIn("win32", str(ctx.exception).lower())


class ServerResourcesWindowsWiringTests(unittest.TestCase):
    def test_pressure_windows_note_not_healthy_or_zero(self):
        with mock.patch.object(sr.sys, "platform", "win32"):
            pressure = sr.sample_pressure()
        self.assertEqual(pressure["status"], "unavailable")
        self.assertEqual(pressure["reason"], "unsupported_pressure_counter")
        blob = str(pressure).lower()
        self.assertNotIn("'status': 'healthy'", blob)
        self.assertNotEqual(pressure.get("status"), "healthy")
        self.assertIn("not zero", pressure["semantics"].lower())
        self.assertIn("windows", pressure.get("note", "").lower() + pressure["semantics"].lower())

    def test_sample_process_routes_win32_without_fallback(self):
        sentinel = {
            "start_identity": "windows_creation_filetime:1",
            "resident_bytes": 100,
            "user_s": 0.1,
            "system_s": 0.0,
            "provenance": {"os": "windows"},
        }
        with mock.patch.object(sr.sys, "platform", "win32"):
            with mock.patch(
                "windows_process_sample.sample_process", return_value=sentinel
            ) as win:
                out = sr.sample_process(55, timeout=1.0)
        self.assertIs(out, sentinel)
        win.assert_called_once_with(55, timeout=1.0)


class ProcessAccountingPortabilityTests(unittest.TestCase):
    def test_children_rusage_unavailable_without_resource(self):
        import process_accounting as pa

        # Force lazy import miss path.
        prev = pa._resource
        try:
            pa._resource = False
            snap = pa._children_rusage_snapshot(rusage_children_fn=None)
            self.assertEqual(snap["status"], "unavailable")
            self.assertEqual(snap["reason"], "rusage_children_unsupported_on_platform")
            self.assertIsNone(snap["cumulative_user_s"])
            self.assertIsNone(snap["cumulative_system_s"])
            self.assertIsNone(snap["cumulative_total_s"])
            self.assertIsNone(snap["current_rss_bytes"])
            # Injected path still works (existing tests rely on this).
            kids = SimpleNamespace(ru_utime=1.5, ru_stime=0.25, ru_maxrss=99)
            ok = pa._children_rusage_snapshot(rusage_children_fn=lambda: kids)
            self.assertEqual(ok["status"], "available")
            self.assertEqual(ok["cumulative_user_s"], 1.5)
        finally:
            pa._resource = prev

    def test_module_import_does_not_require_top_level_resource(self):
        import process_accounting as pa

        self.assertFalse(hasattr(pa, "resource"))
        self.assertTrue(callable(pa._get_resource_module))
        # On this Mac host resource still loads lazily.
        if sys.platform != "win32":
            self.assertIsNotNone(pa._get_resource_module())


@unittest.skipUnless(sys.platform == "win32", "requires native Windows")
class LiveWindowsSampleTests(unittest.TestCase):
    def test_own_pid_nonzero_working_set_and_monotonic_cpu(self):
        pid = os.getpid()
        first = wps.sample_process(pid)
        until = time.monotonic() + 0.05
        acc = 0
        while time.monotonic() < until:
            acc += sum(range(200))
        second = wps.sample_process(pid)
        self.assertEqual(first["start_identity"], second["start_identity"])
        self.assertGreater(first["resident_bytes"], 0)
        self.assertGreater(second["resident_bytes"], 0)
        self.assertGreaterEqual(second["user_s"] + second["system_s"], first["user_s"] + first["system_s"])
        # Burn some user time; allow equal if quantum coarse, but not decreasing.
        delta = sr.cpu_delta_seconds(first, second)
        self.assertGreaterEqual(delta["total_s"], 0.0)
        self.assertIs(acc >= 0, True)  # keep acc live for optimizer

    def test_missing_pid_fails(self):
        with self.assertRaises(sr.SampleError):
            wps.sample_process(999_999_991)

    def test_server_resources_system_backend_own_pid(self):
        sample = sr.sample_process(os.getpid())
        self.assertGreater(sample["resident_bytes"], 0)
        self.assertTrue(str(sample["start_identity"]).startswith("windows_creation_filetime:"))
        pressure = sr.sample_pressure()
        self.assertEqual(pressure["status"], "unavailable")


if __name__ == "__main__":
    unittest.main()
