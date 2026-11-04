"""Exercise AMD64 minidump projection on synthetic and archived real dumps."""

import ntpath
from pathlib import Path
import struct
import tempfile
import unittest

import t14_windows_thread_evidence as evidence


ARCHIVED_BASELINE_DUMP = Path(
    "/Volumes/dev-scratch/274bot-evidence/TRACK-T1/T1.4/closeout-1103/windows/"
    "partial-capture-agent4-1104/T14-THREAD-DIAGNOSTIC-BASELINE/thread-snapshot.dmp"
)
ARCHIVED_BASELINE_PID = 9896
UNNAMED_BASELINE_TIDS = (10512, 4956, 4468, 5140, 1524, 3036)
SYSTEM_MODULE = r"C:\Windows\System32\ntdll.dll"
PRODUCT_MODULE = r"C:\build\t14_soak_live-test.exe"
SYSTEM_BASE = 0x7FFB00000000
PRODUCT_BASE = 0x7FFA00000000


def is_windows_system_module(module):
    normalized = ntpath.normcase(module)
    return normalized.startswith("c:\\windows\\system32\\") and normalized.endswith(".dll")


class MinidumpProjectionTests(unittest.TestCase):
    @staticmethod
    def dump(*, bad_rsp_tid=None, include_thread_info=True):
        data = bytearray(16384)
        module_rva = 128
        thread_rva = 360
        system_rva = 472
        misc_rva = 536
        info_rva = 576
        name_rvas = (800, 1000)
        context_rvas = (2048, 2304)
        stack_rvas = (4096, 8192)
        stack_starts = (0x100000, 0x200000)
        stack_size = 4096
        thread_ids = (13, 14)
        module_names = (SYSTEM_MODULE, PRODUCT_MODULE)
        module_bases = (SYSTEM_BASE, PRODUCT_BASE)

        data[:4] = b"MDMP"
        streams = [
            (4, 4 + 2 * 108, module_rva),
            (3, 4 + 2 * 48, thread_rva),
            (7, 56, system_rva),
            (15, 24, misc_rva),
        ]
        if include_thread_info:
            streams.append((17, 12 + 2 * 64, info_rva))
        struct.pack_into("<II", data, 8, len(streams), 32)
        for index, (kind, size, rva) in enumerate(streams):
            struct.pack_into("<III", data, 32 + index * 12, kind, size, rva)

        struct.pack_into("<I", data, module_rva, 2)
        for index, (module_name, base, name_rva) in enumerate(
            zip(module_names, module_bases, name_rvas)
        ):
            encoded_name = module_name.encode("utf-16-le")
            struct.pack_into(
                "<QIIII",
                data,
                module_rva + 4 + index * 108,
                base,
                0x100000,
                0,
                0,
                name_rva,
            )
            struct.pack_into("<I", data, name_rva, len(encoded_name))
            data[name_rva + 4 : name_rva + 4 + len(encoded_name)] = encoded_name

        struct.pack_into("<I", data, thread_rva, len(thread_ids))
        struct.pack_into("<H", data, system_rva, 9)
        struct.pack_into("<III", data, misc_rva, 24, 1, 500)
        if include_thread_info:
            struct.pack_into("<III", data, info_rva, 12, 64, len(thread_ids))

        for index, tid in enumerate(thread_ids):
            thread_offset = thread_rva + 4 + index * 48
            stack_start = stack_starts[index]
            stack_rva = stack_rvas[index]
            context_rva = context_rvas[index]
            struct.pack_into("<I", data, thread_offset, tid)
            struct.pack_into(
                "<QII", data, thread_offset + 24, stack_start, stack_size, stack_rva
            )
            struct.pack_into(
                "<II", data, thread_offset + 40, 0x100, context_rva
            )
            rsp = stack_start + stack_size if tid == bad_rsp_tid else stack_start
            struct.pack_into("<Q", data, context_rva + 0x98, rsp)
            struct.pack_into("<Q", data, context_rva + 0xF8, SYSTEM_BASE + 0x123)
            if include_thread_info:
                info_offset = info_rva + 12 + index * 64
                start_address = (
                    SYSTEM_BASE + 0x200 if tid == 13 else PRODUCT_BASE + 0x300
                )
                struct.pack_into("<I", data, info_offset, tid)
                struct.pack_into("<Q", data, info_offset + 16, 1000 + tid)
                struct.pack_into("<Q", data, info_offset + 48, start_address)

        struct.pack_into("<Q", data, stack_rvas[0] + 0x20, SYSTEM_BASE + 0x456)
        struct.pack_into("<Q", data, stack_rvas[0] + 0x180, SYSTEM_BASE + 0x789)
        struct.pack_into("<Q", data, stack_rvas[0] + 0x900, PRODUCT_BASE + 0x10)
        struct.pack_into("<Q", data, stack_rvas[1] + 0x20, SYSTEM_BASE + 0xABC)
        return data

    def test_full_stack_pid_thread_info_and_win32_start_projection(self):
        raw = self.dump()
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "snapshot-0001.dmp"
            path.write_bytes(raw)
            projection = evidence.analyze(path, {13: "", 14: "", 99: ""}, 500)

        self.assertEqual(projection["sampled_pid"], 500)
        rows = {row["tid"]: row for row in projection["threads"]}
        product_stack = rows[13]
        product_start = rows[14]
        self.assertEqual(product_stack["thread_creation_filetime"], 1013)
        self.assertEqual(product_stack["start_module"], SYSTEM_MODULE)
        self.assertEqual(product_stack["ip_module"]["module"], SYSTEM_MODULE)
        self.assertEqual(product_stack["stack_window_bytes"], 4096)
        self.assertEqual(
            product_stack["candidate_return_address_module_counts"],
            {SYSTEM_MODULE: 2, PRODUCT_MODULE: 1},
        )
        self.assertEqual(product_start["thread_creation_filetime"], 1014)
        self.assertEqual(product_start["start_module"], PRODUCT_MODULE)
        self.assertEqual(product_start["candidate_return_address_module_counts"],
                         {SYSTEM_MODULE: 1})
        self.assertEqual(set(rows[99]), {"tid", "error"})

    def test_malformed_thread_isolated_to_its_row(self):
        raw = self.dump(bad_rsp_tid=13)
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "snapshot-0001.dmp"
            path.write_bytes(raw)
            projection = evidence.analyze(path, {13: "", 14: ""}, 500)

        rows = {row["tid"]: row for row in projection["threads"]}
        self.assertEqual(set(rows[13]), {"tid", "error"})
        self.assertIn("stack pointer outside captured stack", rows[13]["error"])
        self.assertEqual(rows[14]["start_module"], PRODUCT_MODULE)

    def test_pid_mismatch_and_missing_thread_info_stream_reject(self):
        raw = self.dump()
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "snapshot-0001.dmp"
            path.write_bytes(raw)
            with self.assertRaisesRegex(ValueError, "does not match sampled process"):
                evidence.analyze(path, {13: ""}, 501)

            path.write_bytes(self.dump(include_thread_info=False))
            with self.assertRaisesRegex(ValueError, "thread info stream absent"):
                evidence.analyze(path, {13: ""}, 500)

    @unittest.skipUnless(
        ARCHIVED_BASELINE_DUMP.is_file(),
        "archived Windows baseline minidump is not present",
    )
    def test_archived_baseline_real_dump_projects_unnamed_and_product_threads(self):
        projection = evidence.analyze(
            ARCHIVED_BASELINE_DUMP,
            {tid: "" for tid in UNNAMED_BASELINE_TIDS},
            ARCHIVED_BASELINE_PID,
        )
        self.assertEqual(projection["dump_sha256"],
                         "77a4467efd4c105604942f8753bf9bfcffcd2279c441f2608ec0e68af6f6d04b")
        self.assertEqual(projection["sampled_pid"], ARCHIVED_BASELINE_PID)
        rows = {row["tid"]: row for row in projection["threads"]}

        for tid in UNNAMED_BASELINE_TIDS:
            with self.subTest(tid=tid):
                row = rows[tid]
                self.assertNotIn("error", row)
                self.assertEqual(row["description_first_sample"], "")
                self.assertGreater(row["thread_creation_filetime"], 0)
                self.assertTrue(is_windows_system_module(row["start_module"]))
                self.assertTrue(is_windows_system_module(row["ip_module"]["module"]))
                self.assertGreaterEqual(row["stack_window_bytes"], 256)
                self.assertTrue(row["candidate_return_address_module_counts"])
                self.assertTrue(all(
                    is_windows_system_module(module)
                    for module in row["candidate_return_address_module_counts"]
                ))

        product_threads = [
            row for row in projection["threads"]
            if isinstance(row.get("start_module"), str)
            and ntpath.basename(row["start_module"]).lower().startswith("t14_soak_live-")
            and ntpath.basename(row["start_module"]).lower().endswith(".exe")
        ]
        self.assertEqual(len(product_threads), 7)
        for row in product_threads:
            with self.subTest(tid=row["tid"]):
                self.assertGreater(
                    row["candidate_return_address_module_counts"].get(row["start_module"], 0),
                    0,
                )


if __name__ == "__main__":
    unittest.main()
