"""Check that Windows census provenance follows thread lifetimes, not TIDs."""

import unittest

import windows_t14_soak as sampler


class ProvenanceBindingTests(unittest.TestCase):
    @staticmethod
    def thread(created):
        return {
            "tid": 13, "comm": "", "windows_thread_description": "",
            "windows_process_main_thread": False, "windows_process_id": 500,
            "thread_creation_filetime": created,
        }

    @staticmethod
    def projection():
        return {
            "schema": "t14-windows-thread-minidump-stack-projection-v2",
            "sampled_pid": 500, "dump_path": "snapshot-0001.dmp",
            "dump_sha256": "a" * 64, "limits": "bounded stack projection",
            "threads": [{"tid": 13, "description_first_sample": "",
                         "ip_module": {"module": "C:\\Windows\\System32\\ntdll.dll"},
                         "stack_window_bytes": 2048,
                         "candidate_return_address_module_counts": {
                             "C:\\Windows\\System32\\kernel32.dll": 2}}],
        }

    def test_first_sight_cached_only_for_same_live_identity(self):
        captures = []
        def capture(descriptions):
            captures.append(descriptions)
            return self.projection()
        cache = {}
        first = [self.thread(100)]
        sampler.bind_unnamed_provenance(first, 500, 1000, cache, capture)
        self.assertEqual(first[0]["os_runtime_provenance"]["thread_creation_filetime"], 100)
        same = [self.thread(100)]
        sampler.bind_unnamed_provenance(same, 500, 2000, cache, capture)
        self.assertEqual(len(captures), 1)
        self.assertEqual(same[0]["os_runtime_provenance"]["capture_sample_monotonic_ns"], 1000)
        reused = [self.thread(101)]
        sampler.bind_unnamed_provenance(reused, 500, 3000, cache, capture)
        self.assertEqual(len(captures), 2)
        self.assertEqual(reused[0]["os_runtime_provenance"]["thread_creation_filetime"], 101)
        self.assertNotIn((500, 13, 100), cache)
        sampler.bind_unnamed_provenance([], 500, 4000, cache, capture)
        self.assertEqual(cache, {})

    def test_missing_stack_or_capture_failure_never_becomes_os_evidence(self):
        incomplete = self.projection()
        incomplete["threads"] = []
        for capture in (lambda _: incomplete,
                        lambda _: (_ for _ in ()).throw(OSError("missing stack"))):
            with self.subTest(capture=capture):
                thread = self.thread(100)
                sampler.bind_unnamed_provenance([thread], 500, 1000, {}, capture)
                self.assertIn("error", thread["os_runtime_provenance"])
                self.assertNotIn("schema", thread["os_runtime_provenance"])


if __name__ == "__main__":
    unittest.main()
