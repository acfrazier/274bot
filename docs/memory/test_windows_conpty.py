#!/usr/bin/env python3
"""Injected-API contract and failure-ownership tests for Windows ConPTY helper."""
from __future__ import annotations

import ctypes
import pathlib
import sys
import unittest
from typing import Any, List, Optional

ROOT = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))

import windows_conpty as wcp  # noqa: E402


def _set_handle(ptr: Any, value: int) -> None:
    handle = ctypes.cast(ptr, ctypes.POINTER(ctypes.c_void_p)).contents
    handle.value = value


def _set_dword(ptr: Any, value: int) -> None:
    target = ctypes.cast(ptr, ctypes.POINTER(ctypes.c_uint32)).contents
    target.value = value & 0xFFFFFFFF


def _set_size_t(ptr: Any, value: int) -> None:
    target = ctypes.cast(ptr, ctypes.POINTER(ctypes.c_size_t)).contents
    target.value = value


def _lp_value_bits(value_ptr: Any) -> int:
    """Normalize UpdateProcThreadAttribute lpValue to integer handle/pointer bits."""
    if value_ptr is None:
        return 0
    if isinstance(value_ptr, int):
        return int(value_ptr)
    # c_void_p / HANDLE / c_void_p-like
    if isinstance(value_ptr, ctypes.c_void_p):
        return int(value_ptr.value or 0)
    if hasattr(value_ptr, "value"):
        try:
            return int(value_ptr.value or 0)
        except (TypeError, ValueError):
            pass
    try:
        return int(ctypes.cast(value_ptr, ctypes.c_void_p).value or 0)
    except (TypeError, ValueError, ctypes.ArgumentError):
        return int(value_ptr)


class FakeConPtyApi:
    """Records ConPTY lifecycle calls and owns synthetic handles."""

    def __init__(self) -> None:
        self.closed: List[Any] = []
        self.closed_hpcon: List[Any] = []
        self.deleted_attrs: List[Any] = []
        self.terminated: List[Any] = []
        self.lifecycle: List[str] = []
        self.create_pipe_calls = 0
        self.create_pc_size: Optional[tuple] = None
        self.create_pc_flags: Optional[int] = None
        self.update_attr: Optional[int] = None
        self.update_cb: Optional[int] = None
        self.update_lp_value_bits: Optional[int] = None
        self.create_process_flags: Optional[int] = None
        self.create_process_inherit: Optional[bool] = None
        self.create_process_cmdline: Optional[str] = None
        self.create_process_cwd: Optional[str] = None
        self.create_process_env: Optional[bytes] = None
        self.startup_cb: Optional[int] = None
        self.startup_flags: Optional[int] = None
        self.written: bytearray = bytearray()
        self.read_chunks: List[bytes] = [b"hello-utf8-\xc2\xb5", b""]
        self.exit_code = 7
        self.wait_rc = wcp.WAIT_OBJECT_0
        self.fail_at: Optional[str] = None
        self.last_error = 0
        self._next_handle = 0x1000
        self.h_in_r = 0
        self.h_in_w = 0
        self.h_out_r = 0
        self.h_out_w = 0
        self.hpcon = 0xC0A1
        self.h_process = 0xBEE1
        self.h_thread = 0xBEE2
        self.pid = 4242
        self.attr_size = 64

    def _alloc(self) -> int:
        self._next_handle += 1
        return self._next_handle

    def GetLastError(self) -> int:
        return int(self.last_error)

    def CreatePipe(self, read_ptr, write_ptr, _sa, _size) -> bool:
        self.create_pipe_calls += 1
        if self.fail_at == "CreatePipe" and self.create_pipe_calls == 1:
            self.last_error = 5
            return False
        if self.fail_at == "CreatePipe2" and self.create_pipe_calls == 2:
            self.last_error = 6
            return False
        r = self._alloc()
        w = self._alloc()
        if self.create_pipe_calls == 1:
            self.h_in_r, self.h_in_w = r, w
        else:
            self.h_out_r, self.h_out_w = r, w
        _set_handle(read_ptr, r)
        _set_handle(write_ptr, w)
        return True

    def CreatePseudoConsole(self, size, input_read, output_write, flags, hpc_ptr) -> int:
        self.create_pc_size = (int(size.X), int(size.Y))
        self.create_pc_flags = int(flags)
        if self.fail_at == "CreatePseudoConsole":
            return 0x80070005
        # Microsoft sample: input read side + output write side.
        assert int(input_read) == self.h_in_r
        assert int(output_write) == self.h_out_w
        _set_handle(hpc_ptr, self.hpcon)
        return wcp.S_OK

    def ClosePseudoConsole(self, hpc) -> None:
        self.lifecycle.append("ClosePseudoConsole")
        self.closed_hpcon.append(int(hpc))

    def InitializeProcThreadAttributeList(self, attr_list, count, flags, size_ptr) -> bool:
        assert int(count) == 1
        assert int(flags) == 0
        if attr_list is None or (isinstance(attr_list, int) and attr_list == 0):
            _set_size_t(size_ptr, self.attr_size)
            self.last_error = 122  # ERROR_INSUFFICIENT_BUFFER
            return False
        if self.fail_at == "InitializeProcThreadAttributeList":
            self.last_error = 8
            return False
        self.lifecycle.append("InitializeProcThreadAttributeList")
        return True

    def UpdateProcThreadAttribute(
        self, attr_list, flags, attribute, value_ptr, cb_size, prev, ret_size
    ) -> bool:
        self.update_attr = int(attribute)
        self.update_cb = int(cb_size)
        # lpValue must be the HPCON handle bits (not a pointer to a temporary HANDLE).
        self.update_lp_value_bits = _lp_value_bits(value_ptr)
        self.lifecycle.append("UpdateProcThreadAttribute")
        if self.fail_at == "UpdateProcThreadAttribute":
            self.last_error = 87
            return False
        assert int(flags) == 0
        assert int(attribute) == wcp.PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE
        return True

    def DeleteProcThreadAttributeList(self, attr_list) -> None:
        self.lifecycle.append("DeleteProcThreadAttributeList")
        self.deleted_attrs.append(attr_list)

    def CreateProcessW(
        self,
        app_name,
        cmdline,
        _pa,
        _ta,
        inherit,
        flags,
        env_ptr,
        cwd,
        startup_info_ptr,
        pi_ptr,
    ) -> bool:
        self.lifecycle.append("CreateProcessW")
        self.create_process_inherit = bool(inherit)
        self.create_process_flags = int(flags)
        if hasattr(cmdline, "value"):
            self.create_process_cmdline = str(cmdline.value)
        else:
            self.create_process_cmdline = ctypes.wstring_at(cmdline)
        self.create_process_cwd = None if cwd is None else str(cwd)
        if env_ptr:
            raw = ctypes.string_at(env_ptr, 8192)
            end = 0
            while end + 3 < len(raw):
                if raw[end : end + 4] == b"\x00\x00\x00\x00":
                    end += 4
                    break
                end += 2
            self.create_process_env = raw[:end]
        si = ctypes.cast(startup_info_ptr, ctypes.POINTER(wcp.STARTUPINFOW)).contents
        self.startup_cb = int(si.cb)
        self.startup_flags = int(si.dwFlags)
        if self.fail_at == "CreateProcessW":
            self.last_error = 2
            return False
        pi = ctypes.cast(pi_ptr, ctypes.POINTER(wcp.PROCESS_INFORMATION)).contents
        pi.hProcess = self.h_process
        pi.hThread = self.h_thread
        pi.dwProcessId = self.pid
        pi.dwThreadId = 99
        return True

    def CloseHandle(self, handle) -> bool:
        self.closed.append(int(handle))
        return True

    def WriteFile(self, handle, buf, size, written_ptr, _ov) -> bool:
        assert int(handle) == self.h_in_w
        data = ctypes.string_at(buf, size)
        self.written.extend(data)
        _set_dword(written_ptr, size)
        return True

    def ReadFile(self, handle, buf, size, read_ptr, _ov) -> bool:
        assert int(handle) == self.h_out_r
        if not self.read_chunks:
            self.last_error = 109  # ERROR_BROKEN_PIPE
            return False
        chunk = self.read_chunks.pop(0)
        if not chunk:
            _set_dword(read_ptr, 0)
            return True
        n = min(len(chunk), size)
        ctypes.memmove(buf, chunk[:n], n)
        _set_dword(read_ptr, n)
        return True

    def WaitForSingleObject(self, handle, timeout_ms) -> int:
        assert int(handle) == self.h_process
        return int(self.wait_rc)

    def GetExitCodeProcess(self, handle, code_ptr) -> bool:
        assert int(handle) == self.h_process
        _set_dword(code_ptr, self.exit_code)
        return True

    def TerminateProcess(self, handle, code) -> bool:
        self.terminated.append((int(handle), int(code)))
        self.exit_code = int(code)
        return True


class EnvAndQuoteTests(unittest.TestCase):
    def test_env_block_utf16_double_nul(self):
        blob = wcp.build_env_block({"B": "2", "A": "1"})
        text = blob.decode("utf-16-le")
        self.assertTrue(text.endswith("\0\0"))
        self.assertIn("A=1\0", text)
        self.assertIn("B=2\0", text)

    def test_env_rejects_bad_keys(self):
        with self.assertRaises(wcp.ConPtyError):
            wcp.build_env_block({"A=B": "1"})
        with self.assertRaises(wcp.ConPtyError):
            wcp.build_env_block({"A": "x\0y"})

    def test_quote_command_line_spaces(self):
        line = wcp.quote_windows_command_line([r"C:\Program Files\app.exe", "a b"])
        self.assertIn("app.exe", line)
        self.assertIn('"', line)


class SpawnContractTests(unittest.TestCase):
    def test_spawn_120x40_flags_and_handles(self):
        api = FakeConPtyApi()
        session = wcp.spawn(
            [r"C:\bin\tui-play.exe"],
            cwd=r"C:\work",
            env={"TERM": "xterm-256color", "LIVE": "1"},
            cols=120,
            rows=40,
            api=api,
        )
        try:
            self.assertEqual(api.create_pc_size, (120, 40))
            self.assertEqual(api.create_pc_flags, 0)
            self.assertEqual(api.update_attr, wcp.PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE)
            self.assertEqual(api.update_cb, ctypes.sizeof(ctypes.c_void_p))
            # Critical ABI: lpValue is HPCON handle bits, not a pointer-to-temporary.
            self.assertEqual(api.update_lp_value_bits, api.hpcon)
            flags = api.create_process_flags or 0
            self.assertTrue(flags & wcp.EXTENDED_STARTUPINFO_PRESENT)
            self.assertTrue(flags & wcp.CREATE_UNICODE_ENVIRONMENT)
            self.assertFalse(api.create_process_inherit)
            self.assertEqual(api.startup_flags, wcp.STARTF_USESTDHANDLES)
            self.assertEqual(api.startup_cb, ctypes.sizeof(wcp.STARTUPINFOEXW))
            self.assertEqual(session.pid, 4242)
            self.assertEqual(session.cols, 120)
            self.assertEqual(session.rows, 40)
            # PTY-side ends closed after CreatePseudoConsole.
            self.assertIn(api.h_in_r, api.closed)
            self.assertIn(api.h_out_w, api.closed)
            self.assertIsNotNone(api.create_process_env)
            env_text = api.create_process_env.decode("utf-16-le")
            self.assertIn("TERM=xterm-256color", env_text)
            self.assertIn("LIVE=1", env_text)
            # EchoCon: attr list deleted immediately after successful CreateProcess.
            self.assertTrue(api.deleted_attrs)
            create_i = api.lifecycle.index("CreateProcessW")
            delete_i = api.lifecycle.index("DeleteProcThreadAttributeList")
            self.assertGreater(delete_i, create_i)
            self.assertNotIn("ClosePseudoConsole", api.lifecycle[: delete_i + 1])

            self.assertEqual(session.write(b"o"), 1)
            self.assertEqual(bytes(api.written), b"o")
            self.assertEqual(session.read(64), b"hello-utf8-\xc2\xb5")
            self.assertEqual(session.wait(), 7)
            self.assertEqual(session.returncode, 7)
            session.close_pseudoconsole()
            self.assertEqual(api.closed_hpcon, [api.hpcon])
        finally:
            session.close()
        self.assertIn(api.h_process, api.closed)
        self.assertIn(api.h_thread, api.closed)
        self.assertIn(api.h_in_w, api.closed)
        self.assertIn(api.h_out_r, api.closed)
        # Success path already deleted attrs; close must not require a second delete.
        self.assertEqual(len(api.deleted_attrs), 1)

    def test_input_writer_matches_session_write(self):
        api = FakeConPtyApi()
        session = wcp.spawn(["tui-play.exe"], api=api)
        try:
            writer = session.input_writer()
            self.assertEqual(writer.write(b"o"), 1)
            writer.close()  # must not close session handles
            self.assertNotIn(api.h_in_w, api.closed)
            self.assertEqual(bytes(api.written), b"o")
        finally:
            session.close()

    def test_terminate_uses_process_handle(self):
        api = FakeConPtyApi()
        session = wcp.spawn(["x"], api=api)
        try:
            codes = [wcp.STILL_ACTIVE]

            def get_exit(handle, code_ptr):
                if codes:
                    _set_dword(code_ptr, codes.pop(0))
                else:
                    _set_dword(code_ptr, 1)
                return True

            api.GetExitCodeProcess = get_exit  # type: ignore
            session.returncode = None
            session.terminate()
            self.assertEqual(api.terminated, [(api.h_process, 1)])
        finally:
            session.close()


class FailureOwnershipTests(unittest.TestCase):
    def test_fail_create_pseudoconsole_closes_pipes(self):
        api = FakeConPtyApi()
        api.fail_at = "CreatePseudoConsole"
        with self.assertRaises(wcp.ConPtyError):
            wcp.spawn(["x"], api=api)
        # Both pipe ends from both CreatePipe calls must be closed.
        self.assertGreaterEqual(len(api.closed), 4)
        self.assertEqual(api.closed_hpcon, [])

    def test_fail_update_attribute_closes_hpcon_and_pipes(self):
        api = FakeConPtyApi()
        api.fail_at = "UpdateProcThreadAttribute"
        with self.assertRaises(wcp.ConPtyError):
            wcp.spawn(["x"], api=api)
        self.assertEqual(api.closed_hpcon, [api.hpcon])
        self.assertTrue(api.deleted_attrs)
        self.assertIn(api.h_in_w, api.closed)
        self.assertIn(api.h_out_r, api.closed)
        # Even on failure, DeleteProcThreadAttributeList before ClosePseudoConsole.
        delete_i = api.lifecycle.index("DeleteProcThreadAttributeList")
        close_i = api.lifecycle.index("ClosePseudoConsole")
        self.assertLess(delete_i, close_i)
        # lpValue still recorded as HPCON bits on the failed update attempt.
        self.assertEqual(api.update_lp_value_bits, api.hpcon)

    def test_fail_create_process_terminates_nothing_started_but_cleans(self):
        api = FakeConPtyApi()
        api.fail_at = "CreateProcessW"
        with self.assertRaises(wcp.ConPtyError) as ctx:
            wcp.spawn(["missing.exe"], api=api)
        self.assertIn("CreateProcessW", str(ctx.exception))
        self.assertEqual(api.closed_hpcon, [api.hpcon])
        self.assertIn(api.h_in_w, api.closed)
        self.assertIn(api.h_out_r, api.closed)
        self.assertTrue(api.deleted_attrs)
        delete_i = api.lifecycle.index("DeleteProcThreadAttributeList")
        close_i = api.lifecycle.index("ClosePseudoConsole")
        self.assertLess(delete_i, close_i)
        self.assertEqual(api.update_lp_value_bits, api.hpcon)

    def test_close_deletes_attr_before_hpcon_when_retained(self):
        """Abort path: session still holding attr list must delete before ClosePseudoConsole."""
        api = FakeConPtyApi()
        session = wcp.spawn(["x"], api=api)
        # Simulate retained attr list (pre-EchoCon immediate free) for close ordering.
        fake_attr = ctypes.c_void_p(0xA11)
        session._attr_list = fake_attr  # noqa: SLF001
        session._attr_buf = object()  # noqa: SLF001
        api.lifecycle.clear()
        api.deleted_attrs.clear()
        api.closed_hpcon.clear()
        session.close()
        self.assertEqual(api.lifecycle[0], "DeleteProcThreadAttributeList")
        self.assertIn("ClosePseudoConsole", api.lifecycle)
        self.assertLess(
            api.lifecycle.index("DeleteProcThreadAttributeList"),
            api.lifecycle.index("ClosePseudoConsole"),
        )
        self.assertEqual(api.closed_hpcon, [api.hpcon])

    def test_require_conpty_support_platform_gate(self):
        with self.assertRaises(wcp.ConPtyError):
            wcp.require_conpty_support(platform="darwin")
        # Injected factory succeeds under forced win32 without native DLL.
        wcp.require_conpty_support(platform="win32", api_factory=FakeConPtyApi)


class ConstantsTests(unittest.TestCase):
    def test_documented_constants(self):
        self.assertEqual(wcp.PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, 0x00020016)
        self.assertEqual(wcp.EXTENDED_STARTUPINFO_PRESENT, 0x00080000)
        self.assertEqual(wcp.CREATE_UNICODE_ENVIRONMENT, 0x00000400)
        self.assertEqual(wcp.STARTF_USESTDHANDLES, 0x00000100)
        self.assertEqual(ctypes.sizeof(wcp.COORD), 4)


if __name__ == "__main__":
    unittest.main()
