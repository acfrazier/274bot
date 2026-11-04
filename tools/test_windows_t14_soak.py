"""Check that Windows census provenance follows thread lifetimes, not TIDs."""

import ctypes
import unittest
from unittest import mock

import windows_t14_soak as sampler


class ProvenanceBindingTests(unittest.TestCase):
    @staticmethod
    def thread(tid=13, created=100):
        return {
            "tid": tid, "comm": "", "windows_thread_description": "",
            "windows_process_main_thread": False, "windows_process_id": 500,
            "thread_creation_filetime": created,
        }

    @staticmethod
    def projection(*threads):
        return {
            "schema": "t14-windows-thread-minidump-stack-projection-v2",
            "sampled_pid": 500, "dump_path": "snapshot-0001.dmp",
            "dump_sha256": "a" * 64, "limits": "full captured stack",
            "threads": [
                {
                    "tid": tid,
                    "description_first_sample": "",
                    "thread_creation_filetime": created,
                    "start_module": r"C:\Windows\System32\ntdll.dll",
                    "ip_module": {"module": r"C:\Windows\System32\ntdll.dll"},
                    "stack_window_bytes": 4096,
                    "candidate_return_address_module_counts": {
                        r"C:\Windows\System32\kernel32.dll": 2,
                    },
                }
                for tid, created in threads
            ],
        }

    def test_first_sight_cached_only_for_same_live_identity(self):
        captures = []

        def capture(descriptions):
            captures.append(descriptions)
            return self.projection((13, 100 if len(captures) == 1 else 101))

        cache = {}
        first = [self.thread(13, 100)]
        sampler.bind_unnamed_provenance(first, 500, 1000, cache, capture)
        self.assertEqual(first[0]["os_runtime_provenance"]["thread_creation_filetime"], 100)
        self.assertEqual(first[0]["os_runtime_provenance"]["start_module"],
                         r"C:\Windows\System32\ntdll.dll")
        same = [self.thread(13, 100)]
        sampler.bind_unnamed_provenance(same, 500, 2000, cache, capture)
        self.assertEqual(len(captures), 1)
        self.assertEqual(same[0]["os_runtime_provenance"]["capture_sample_monotonic_ns"], 1000)
        reused = [self.thread(13, 101)]
        sampler.bind_unnamed_provenance(reused, 500, 3000, cache, capture)
        self.assertEqual(len(captures), 2)
        self.assertEqual(reused[0]["os_runtime_provenance"]["thread_creation_filetime"], 101)
        self.assertNotIn((500, 13, 100), cache)
        sampler.bind_unnamed_provenance([], 500, 4000, cache, capture)
        self.assertEqual(cache, {})

    def test_thread_error_isolated_and_retried_without_caching(self):
        attempts = 0

        def capture(_descriptions):
            nonlocal attempts
            attempts += 1
            if attempts == 1:
                projection = self.projection((14, 200))
                projection["threads"].insert(0, {"tid": 13, "error": "bad context"})
                return projection
            return self.projection((13, 100), (14, 200))

        cache = {}
        first = [self.thread(13, 100), self.thread(14, 200)]
        sampler.bind_unnamed_provenance(first, 500, 1000, cache, capture)
        by_tid = {row["tid"]: row for row in first}
        self.assertEqual(by_tid[13]["os_runtime_provenance"], {"error": "bad context"})
        self.assertEqual(by_tid[14]["os_runtime_provenance"]["schema"],
                         "t14-windows-thread-minidump-stack-projection-v2")
        self.assertNotIn((500, 13, 100), cache)
        self.assertIn((500, 14, 200), cache)

        retry = [self.thread(13, 100), self.thread(14, 200)]
        sampler.bind_unnamed_provenance(retry, 500, 2000, cache, capture)
        self.assertEqual(attempts, 2)
        self.assertEqual({row["tid"]: row for row in retry}[13]["os_runtime_provenance"]["schema"],
                         "t14-windows-thread-minidump-stack-projection-v2")
        self.assertEqual(cache[(500, 13, 100)]["capture_sample_monotonic_ns"], 2000)

    def test_capture_failure_is_reported_for_this_sample_and_retried(self):
        attempts = []

        def fail(_descriptions):
            attempts.append("failed")
            raise OSError("missing stack")

        thread = self.thread(13, 100)
        cache = {}
        sampler.bind_unnamed_provenance([thread], 500, 1000, cache, fail)
        self.assertEqual(thread["os_runtime_provenance"]["error"], "OSError: missing stack")
        self.assertEqual(cache, {})

        sampler.bind_unnamed_provenance(
            [thread], 500, 2000, cache,
            lambda _: self.projection((13, 100)),
        )
        self.assertEqual(len(attempts), 1)
        self.assertEqual(thread["os_runtime_provenance"]["capture_sample_monotonic_ns"], 2000)


    def test_thread_creation_mismatch_is_not_cached_and_retries(self):
        attempts = 0

        def capture(_descriptions):
            nonlocal attempts
            attempts += 1
            creation = 99 if attempts == 1 else 100
            return self.projection((13, creation))

        thread = self.thread(13, 100)
        cache = {}
        sampler.bind_unnamed_provenance([thread], 500, 1000, cache, capture)
        self.assertEqual(
            thread["os_runtime_provenance"]["error"],
            "thread creation FILETIME does not match first sample",
        )
        self.assertEqual(cache, {})

        sampler.bind_unnamed_provenance([thread], 500, 2000, cache, capture)
        self.assertEqual(attempts, 2)
        self.assertEqual(
            thread["os_runtime_provenance"]["thread_creation_filetime"], 100
        )


class ThreadCensusProgressTests(unittest.TestCase):
    class FakeKernel32:
        def __init__(self, first_time_error=6):
            self.entries = ((500, 13), (500, 14))
            self.index = 0
            self.last_error = 0
            self.first_time_error = first_time_error
            self.next_calls = 0

        def CreateToolhelp32Snapshot(self, _flags, _process_id):
            return 99

        def write_entry(self, entry_pointer, index):
            entry = ctypes.cast(
                entry_pointer, ctypes.POINTER(sampler.THREADENTRY32)
            ).contents
            entry.th32OwnerProcessID, entry.th32ThreadID = self.entries[index]

        def Thread32First(self, _snapshot, entry_pointer):
            self.index = 0
            self.write_entry(entry_pointer, self.index)
            return True

        def Thread32Next(self, _snapshot, entry_pointer):
            self.next_calls += 1
            self.index += 1
            if self.index == len(self.entries):
                self.last_error = sampler.ERROR_NO_MORE_FILES
                return False
            self.write_entry(entry_pointer, self.index)
            return True

        def OpenThread(self, _access, _inherit, thread_id):
            return 1000 + thread_id

        def GetThreadTimes(self, thread_handle, created_pointer, *_other):
            thread_id = thread_handle - 1000
            if thread_id == 13:
                self.last_error = self.first_time_error
                return False
            created = ctypes.cast(
                created_pointer, ctypes.POINTER(sampler.wintypes.FILETIME)
            ).contents
            created.dwLowDateTime = 1014
            created.dwHighDateTime = 0
            return True

        def GetThreadDescription(self, _thread_handle, pointer):
            ctypes.cast(pointer, ctypes.POINTER(ctypes.c_void_p)).contents.value = None
            return 0

        def CloseHandle(self, _handle):
            return True

    def run_census(self, kernel):
        with (
            mock.patch.object(ctypes, "set_last_error", lambda _code: None, create=True),
            mock.patch.object(
                ctypes, "get_last_error", lambda: kernel.last_error, create=True
            ),
            mock.patch.object(ctypes, "FormatError", lambda code: f"error {code}",
                              create=True),
        ):
            return sampler.thread_census(kernel, 500)

    def test_thread32next_advances_after_thread_times_reports_terminated_thread(self):
        kernel = self.FakeKernel32(first_time_error=6)
        rows, _main_tid = self.run_census(kernel)

        self.assertEqual(kernel.next_calls, 2)
        self.assertEqual([row["tid"] for row in rows], [14])

    def test_thread32next_runs_before_unexpected_thread_times_error_is_raised(self):
        kernel = self.FakeKernel32(first_time_error=5)
        with self.assertRaisesRegex(OSError, "GetThreadTimes\\(13\\)"):
            self.run_census(kernel)
        self.assertEqual(kernel.next_calls, 1)


if __name__ == "__main__":
    unittest.main()
