"""Exercise AMD64 minidump offsets and fail-closed stack limits."""

from pathlib import Path
import struct
import tempfile
import unittest

import t14_windows_thread_evidence as evidence


class MinidumpProjectionTests(unittest.TestCase):
    @staticmethod
    def dump(*, short_stack=False):
        data = bytearray(4096)
        module_rva, thread_rva, system_rva = 68, 180, 232
        name_rva, context_rva, stack_rva = 288, 512, 1024
        module = r"C:\Windows\System32\kernel32.dll"
        name = module.encode("utf-16-le")
        data[:4] = b"MDMP"
        struct.pack_into("<II", data, 8, 3, 32)
        for i, (kind, size, rva) in enumerate(((4, 112, module_rva),
                                                (3, 52, thread_rva),
                                                (7, 56, system_rva))):
            struct.pack_into("<III", data, 32 + i * 12, kind, size, rva)
        base = 0x7FFB00000000
        struct.pack_into("<I", data, module_rva, 1)
        struct.pack_into("<QIIII", data, module_rva + 4, base, 0x10000, 0, 0, name_rva)
        struct.pack_into("<I", data, name_rva, len(name))
        data[name_rva + 4:name_rva + 4 + len(name)] = name
        struct.pack_into("<I", data, thread_rva, 1)
        struct.pack_into("<I", data, thread_rva + 4, 13)
        struct.pack_into("<QII", data, thread_rva + 4 + 24, 0x100000,
                         128 if short_stack else 2048, stack_rva)
        struct.pack_into("<II", data, thread_rva + 4 + 40, 0x110, context_rva)
        struct.pack_into("<H", data, system_rva, 9)
        struct.pack_into("<Q", data, context_rva + 0xA0, 0x100000)  # Rsp
        struct.pack_into("<Q", data, context_rva + 0x100, base + 0x123)  # Rip
        struct.pack_into("<Q", data, stack_rva, base + 0x456)
        return data, module

    def test_context_rip_rsp_and_bounded_stack_module(self):
        raw, module = self.dump()
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "snapshot-0001.dmp"
            path.write_bytes(raw)
            row = evidence.analyze(path, {13: ""}, 500)["threads"][0]
        self.assertEqual(row["ip_module"], {"module": module, "offset": "0x123"})
        self.assertEqual(row["sp"], "0x100000")
        self.assertEqual(row["candidate_return_address_module_counts"], {module: 1})
        self.assertEqual(row["stack_window_bytes"], 2048)

    def test_missing_bounded_stack_is_not_evidence(self):
        raw, _ = self.dump(short_stack=True)
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "snapshot-0001.dmp"
            path.write_bytes(raw)
            with self.assertRaisesRegex(ValueError, "short captured stack"):
                evidence.analyze(path, {13: ""}, 500)


if __name__ == "__main__":
    unittest.main()
