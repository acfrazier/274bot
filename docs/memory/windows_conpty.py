"""Windows ConPTY terminal session via ctypes (no third-party deps).

Lifecycle follows Microsoft Learn "Creating a Pseudoconsole session":
  1. Create two synchronous pipes (input + output channels).
  2. CreatePseudoConsole(size, inputReadSide, outputWriteSide, 0, &hPC).
  3. STARTUPINFOEX + InitializeProcThreadAttributeList /
     UpdateProcThreadAttribute(PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE).
  4. CreateProcessW with EXTENDED_STARTUPINFO_PRESENT (+ UNICODE env).
  5. Close the PTY-side pipe ends after CreateProcess so refcounts drop.
  6. Host writes user input on inputWriteSide; drains outputReadSide on a
     dedicated thread (avoids ClosePseudoConsole deadlocks).
  7. Wait on the real process handle for exit code; DeleteProcThreadAttributeList
     after successful CreateProcess (EchoCon) and always before ClosePseudoConsole
     on abort; then ClosePseudoConsole (terminates any still-attached clients)
     and free remaining handles.

UTF-8 terminal bytes on the pipes (ConPTY translates). Injectable ConPtyApi
for contract/failure-ownership tests off Windows. No silent headless fallback.
"""
from __future__ import annotations

import ctypes
import subprocess
import sys
import threading
from ctypes import wintypes
from typing import Any, Callable, Dict, List, Mapping, Optional, Sequence, Union

# Fixed Windows ABI widths (do not use host c_ulong on LP64).
_DWORD = ctypes.c_uint32
_WORD = ctypes.c_uint16
_SHORT = ctypes.c_int16
_HRESULT = ctypes.c_long
_SIZE_T = ctypes.c_size_t
_DWORD_PTR = ctypes.c_uint64 if ctypes.sizeof(ctypes.c_void_p) == 8 else ctypes.c_uint32

S_OK = 0
EXTENDED_STARTUPINFO_PRESENT = 0x00080000
CREATE_UNICODE_ENVIRONMENT = 0x00000400
STARTF_USESTDHANDLES = 0x00000100
# ProcThreadAttributePseudoConsole = 22; Input bit set → 0x00020016
PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE = 0x00020016
INFINITE = 0xFFFFFFFF
WAIT_OBJECT_0 = 0x00000000
WAIT_TIMEOUT = 0x00000102
WAIT_FAILED = 0xFFFFFFFF
PROCESS_TERMINATE = 0x0001
STILL_ACTIVE = 259
# HANDLE(-1) on the running pointer width.
INVALID_HANDLE_VALUE = (1 << (ctypes.sizeof(ctypes.c_void_p) * 8)) - 1


class ConPtyError(RuntimeError):
    """Fail-closed ConPTY / process spawn failure."""


class COORD(ctypes.Structure):
    _fields_ = [("X", _SHORT), ("Y", _SHORT)]


class STARTUPINFOW(ctypes.Structure):
    _fields_ = [
        ("cb", _DWORD),
        ("lpReserved", wintypes.LPWSTR),
        ("lpDesktop", wintypes.LPWSTR),
        ("lpTitle", wintypes.LPWSTR),
        ("dwX", _DWORD),
        ("dwY", _DWORD),
        ("dwXSize", _DWORD),
        ("dwYSize", _DWORD),
        ("dwXCountChars", _DWORD),
        ("dwYCountChars", _DWORD),
        ("dwFillAttribute", _DWORD),
        ("dwFlags", _DWORD),
        ("wShowWindow", _WORD),
        ("cbReserved2", _WORD),
        ("lpReserved2", ctypes.POINTER(ctypes.c_byte)),
        ("hStdInput", wintypes.HANDLE),
        ("hStdOutput", wintypes.HANDLE),
        ("hStdError", wintypes.HANDLE),
    ]


class STARTUPINFOEXW(ctypes.Structure):
    _fields_ = [
        ("StartupInfo", STARTUPINFOW),
        ("lpAttributeList", ctypes.c_void_p),
    ]


class PROCESS_INFORMATION(ctypes.Structure):
    _fields_ = [
        ("hProcess", wintypes.HANDLE),
        ("hThread", wintypes.HANDLE),
        ("dwProcessId", _DWORD),
        ("dwThreadId", _DWORD),
    ]


class SECURITY_ATTRIBUTES(ctypes.Structure):
    _fields_ = [
        ("nLength", _DWORD),
        ("lpSecurityDescriptor", ctypes.c_void_p),
        ("bInheritHandle", wintypes.BOOL),
    ]


def _handle_int(value: Any) -> int:
    if value is None:
        return 0
    try:
        return int(value)
    except (TypeError, ValueError):
        return int(ctypes.cast(value, ctypes.c_void_p).value or 0)


def _is_valid_handle(value: Any) -> bool:
    h = _handle_int(value)
    if h == 0:
        return False
    # Normalize signed -1 and all-bits HANDLE.
    if h < 0 or h == INVALID_HANDLE_VALUE:
        return False
    return True


def build_env_block(env: Mapping[str, str]) -> bytes:
    """CREATE_UNICODE_ENVIRONMENT block: UTF-16LE KEY=VAL\\0 ... \\0\\0."""
    if not isinstance(env, Mapping):
        raise ConPtyError("env must be a mapping of str to str")
    parts: List[str] = []
    for key, value in env.items():
        if not isinstance(key, str) or not isinstance(value, str):
            raise ConPtyError("env keys and values must be str")
        if "\0" in key or "\0" in value:
            raise ConPtyError("env entries must not contain NUL")
        if "=" in key:
            raise ConPtyError("env key must not contain '='")
        parts.append(f"{key}={value}")
    # Stable order not required by Win32; sort for deterministic tests.
    parts.sort()
    blob = ("\0".join(parts) + "\0\0").encode("utf-16-le")
    return blob


def quote_windows_command_line(argv: Sequence[str]) -> str:
    """Windows CreateProcess command line via subprocess.list2cmdline."""
    if not argv:
        raise ConPtyError("argv must be non-empty")
    for part in argv:
        if not isinstance(part, str):
            raise ConPtyError("argv entries must be str")
    return subprocess.list2cmdline(list(argv))


class ConPtyApi:
    """Bound kernel32 ConPTY / process surface; tests inject fakes instead."""

    def __init__(self) -> None:
        if sys.platform != "win32":
            raise ConPtyError("native ConPTY API is supported only on win32")
        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)

        self.CreatePipe = kernel32.CreatePipe
        self.CreatePipe.argtypes = [
            ctypes.POINTER(wintypes.HANDLE),
            ctypes.POINTER(wintypes.HANDLE),
            ctypes.POINTER(SECURITY_ATTRIBUTES),
            _DWORD,
        ]
        self.CreatePipe.restype = wintypes.BOOL

        self.CreatePseudoConsole = kernel32.CreatePseudoConsole
        self.CreatePseudoConsole.argtypes = [
            COORD,
            wintypes.HANDLE,
            wintypes.HANDLE,
            _DWORD,
            ctypes.POINTER(wintypes.HANDLE),
        ]
        self.CreatePseudoConsole.restype = _HRESULT

        self.ClosePseudoConsole = kernel32.ClosePseudoConsole
        self.ClosePseudoConsole.argtypes = [wintypes.HANDLE]
        self.ClosePseudoConsole.restype = None

        self.ResizePseudoConsole = kernel32.ResizePseudoConsole
        self.ResizePseudoConsole.argtypes = [wintypes.HANDLE, COORD]
        self.ResizePseudoConsole.restype = _HRESULT

        self.InitializeProcThreadAttributeList = kernel32.InitializeProcThreadAttributeList
        self.InitializeProcThreadAttributeList.argtypes = [
            ctypes.c_void_p,
            _DWORD,
            _DWORD,
            ctypes.POINTER(_SIZE_T),
        ]
        self.InitializeProcThreadAttributeList.restype = wintypes.BOOL

        self.UpdateProcThreadAttribute = kernel32.UpdateProcThreadAttribute
        self.UpdateProcThreadAttribute.argtypes = [
            ctypes.c_void_p,
            _DWORD,
            _DWORD_PTR,
            ctypes.c_void_p,
            _SIZE_T,
            ctypes.c_void_p,
            ctypes.POINTER(_SIZE_T),
        ]
        self.UpdateProcThreadAttribute.restype = wintypes.BOOL

        self.DeleteProcThreadAttributeList = kernel32.DeleteProcThreadAttributeList
        self.DeleteProcThreadAttributeList.argtypes = [ctypes.c_void_p]
        self.DeleteProcThreadAttributeList.restype = None

        self.CreateProcessW = kernel32.CreateProcessW
        self.CreateProcessW.argtypes = [
            wintypes.LPCWSTR,
            wintypes.LPWSTR,
            ctypes.c_void_p,
            ctypes.c_void_p,
            wintypes.BOOL,
            _DWORD,
            ctypes.c_void_p,
            wintypes.LPCWSTR,
            ctypes.POINTER(STARTUPINFOW),
            ctypes.POINTER(PROCESS_INFORMATION),
        ]
        self.CreateProcessW.restype = wintypes.BOOL

        self.CloseHandle = kernel32.CloseHandle
        self.CloseHandle.argtypes = [wintypes.HANDLE]
        self.CloseHandle.restype = wintypes.BOOL

        self.ReadFile = kernel32.ReadFile
        self.ReadFile.argtypes = [
            wintypes.HANDLE,
            ctypes.c_void_p,
            _DWORD,
            ctypes.POINTER(_DWORD),
            ctypes.c_void_p,
        ]
        self.ReadFile.restype = wintypes.BOOL

        self.WriteFile = kernel32.WriteFile
        self.WriteFile.argtypes = [
            wintypes.HANDLE,
            ctypes.c_void_p,
            _DWORD,
            ctypes.POINTER(_DWORD),
            ctypes.c_void_p,
        ]
        self.WriteFile.restype = wintypes.BOOL

        self.WaitForSingleObject = kernel32.WaitForSingleObject
        self.WaitForSingleObject.argtypes = [wintypes.HANDLE, _DWORD]
        self.WaitForSingleObject.restype = _DWORD

        self.GetExitCodeProcess = kernel32.GetExitCodeProcess
        self.GetExitCodeProcess.argtypes = [wintypes.HANDLE, ctypes.POINTER(_DWORD)]
        self.GetExitCodeProcess.restype = wintypes.BOOL

        self.TerminateProcess = kernel32.TerminateProcess
        self.TerminateProcess.argtypes = [wintypes.HANDLE, _DWORD]
        self.TerminateProcess.restype = wintypes.BOOL

        self.GetLastError = kernel32.GetLastError
        self.GetLastError.argtypes = []
        self.GetLastError.restype = _DWORD


class _InputWriter:
    """Probe-facing write end; close is a no-op (session owns the handle)."""

    def __init__(self, session: "ConPtySession") -> None:
        self._session = session

    def write(self, data: bytes) -> int:
        return self._session.write(data)

    def close(self) -> None:
        return None


class ConPtySession:
    """Process-like ConPTY session: pid / wait / terminate / duplex pipes."""

    def __init__(
        self,
        *,
        api: Any,
        hpcon: Any,
        input_write: Any,
        output_read: Any,
        h_process: Any,
        h_thread: Any,
        pid: int,
        attr_buf: Any,
        attr_list: Any,
        cols: int,
        rows: int,
        pty_sides_closed: bool = True,
    ) -> None:
        self._api = api
        self._hpcon = hpcon
        self._input_write = input_write
        self._output_read = output_read
        self._h_process = h_process
        self._h_thread = h_thread
        self.pid = int(pid)
        self.returncode: Optional[int] = None
        self._attr_buf = attr_buf
        self._attr_list = attr_list
        self.cols = int(cols)
        self.rows = int(rows)
        self.transport = "conpty"
        self._closed = False
        self._hpcon_closed = False
        self._write_lock = threading.Lock()
        self._pty_sides_closed = pty_sides_closed

    def input_writer(self) -> _InputWriter:
        return _InputWriter(self)

    def write(self, data: bytes) -> int:
        if not isinstance(data, (bytes, bytearray)):
            raise ConPtyError("ConPTY write requires bytes")
        if self._closed or self._input_write is None:
            raise ConPtyError("ConPTY input handle is closed")
        if not data:
            return 0
        buf = (ctypes.c_char * len(data)).from_buffer_copy(bytes(data))
        written = _DWORD(0)
        with self._write_lock:
            ok = self._api.WriteFile(
                self._input_write, buf, len(data), ctypes.byref(written), None
            )
        if not ok:
            err = int(self._api.GetLastError())
            raise ConPtyError(f"WriteFile(ConPTY input) failed: winerror={err}")
        n = int(written.value)
        if n != len(data):
            raise ConPtyError(f"short ConPTY input write: {n} of {len(data)}")
        return n

    def read(self, size: int = 65536) -> bytes:
        if size <= 0:
            raise ConPtyError("read size must be positive")
        if self._closed or self._output_read is None:
            return b""
        buf = (ctypes.c_char * size)()
        read_n = _DWORD(0)
        ok = self._api.ReadFile(self._output_read, buf, size, ctypes.byref(read_n), None)
        if not ok:
            # Broken pipe / closed console → EOF for drain loop.
            return b""
        n = int(read_n.value)
        if n <= 0:
            return b""
        return bytes(buf[:n])

    def poll(self) -> Optional[int]:
        if self.returncode is not None:
            return self.returncode
        if self._h_process is None:
            return None
        code = _DWORD(0)
        if not self._api.GetExitCodeProcess(self._h_process, ctypes.byref(code)):
            err = int(self._api.GetLastError())
            raise ConPtyError(f"GetExitCodeProcess failed: winerror={err}")
        value = int(code.value)
        if value == STILL_ACTIVE:
            return None
        self.returncode = value
        return self.returncode

    def wait(self, timeout: Optional[float] = None) -> int:
        if self.returncode is not None:
            return self.returncode
        if self._h_process is None:
            raise ConPtyError("no process handle to wait on")
        if timeout is None:
            ms = INFINITE
        else:
            if timeout < 0:
                raise ConPtyError("timeout must be non-negative")
            ms = int(timeout * 1000.0)
        rc = int(self._api.WaitForSingleObject(self._h_process, ms))
        if rc == WAIT_TIMEOUT:
            raise TimeoutError("ConPTY process wait timed out")
        if rc == WAIT_FAILED:
            err = int(self._api.GetLastError())
            raise ConPtyError(f"WaitForSingleObject failed: winerror={err}")
        if rc != WAIT_OBJECT_0:
            raise ConPtyError(f"WaitForSingleObject unexpected status: {rc}")
        code = _DWORD(0)
        if not self._api.GetExitCodeProcess(self._h_process, ctypes.byref(code)):
            err = int(self._api.GetLastError())
            raise ConPtyError(f"GetExitCodeProcess failed: winerror={err}")
        self.returncode = int(code.value)
        return self.returncode

    def terminate(self) -> None:
        if self._h_process is None:
            return
        if self.poll() is not None:
            return
        if not self._api.TerminateProcess(self._h_process, 1):
            err = int(self._api.GetLastError())
            # Already exited races are acceptable.
            if self.poll() is None:
                raise ConPtyError(f"TerminateProcess failed: winerror={err}")

    def close_pseudoconsole(self) -> None:
        """Close HPCON so drain can observe EOF; safe after wait or on abort."""
        if self._hpcon_closed or self._hpcon is None:
            return
        self._api.ClosePseudoConsole(self._hpcon)
        self._hpcon = None
        self._hpcon_closed = True

    def _delete_attr_list(self) -> None:
        """Drop STARTUPINFOEX attribute list (EchoCon: after CreateProcess, before ClosePseudoConsole)."""
        if self._attr_list is None:
            return
        try:
            self._api.DeleteProcThreadAttributeList(self._attr_list)
        except Exception:
            pass
        self._attr_list = None
        self._attr_buf = None

    def close(self) -> None:
        """Release all remaining handles. Prefer close_pseudoconsole after wait first.

        Attribute-list teardown precedes ClosePseudoConsole (MS/EchoCon order) when the
        list was still retained (failure/abort paths). Success spawn deletes the list
        immediately after CreateProcess.
        """
        if self._closed:
            return
        self._closed = True
        api = self._api
        # Attr list may still hold HPCON identity — delete before closing the console.
        self._delete_attr_list()
        self.close_pseudoconsole()

        def _close(handle: Any) -> None:
            if handle is None:
                return
            if not _is_valid_handle(handle):
                return
            try:
                api.CloseHandle(handle)
            except Exception:
                pass

        _close(self._input_write)
        self._input_write = None
        _close(self._output_read)
        self._output_read = None
        _close(self._h_thread)
        self._h_thread = None
        _close(self._h_process)
        self._h_process = None


def _close_handle(api: Any, handle: Any) -> None:
    if handle is None or not _is_valid_handle(handle):
        return
    api.CloseHandle(handle)


def spawn(
    argv: Sequence[str],
    *,
    cwd: Optional[str] = None,
    env: Optional[Mapping[str, str]] = None,
    cols: int = 120,
    rows: int = 40,
    api: Optional[Any] = None,
) -> ConPtySession:
    """Create a 120×40 (default) ConPTY and start argv attached to it."""
    if type(cols) is not int or type(rows) is not int or cols <= 0 or rows <= 0:
        raise ConPtyError("cols and rows must be positive integers")
    if cols > 0x7FFF or rows > 0x7FFF:
        raise ConPtyError("cols/rows exceed COORD range")
    cmdline = quote_windows_command_line(argv)
    bound: Any = api if api is not None else ConPtyApi()

    input_read: Any = None
    input_write: Any = None
    output_read: Any = None
    output_write: Any = None
    hpcon: Any = None
    attr_buf: Any = None
    attr_list: Any = None
    attr_initialized = False
    pi: Optional[PROCESS_INFORMATION] = None

    try:
        h_in_r = wintypes.HANDLE()
        h_in_w = wintypes.HANDLE()
        if not bound.CreatePipe(ctypes.byref(h_in_r), ctypes.byref(h_in_w), None, 0):
            raise ConPtyError(f"CreatePipe(input) failed: winerror={int(bound.GetLastError())}")
        input_read, input_write = h_in_r.value, h_in_w.value

        h_out_r = wintypes.HANDLE()
        h_out_w = wintypes.HANDLE()
        if not bound.CreatePipe(ctypes.byref(h_out_r), ctypes.byref(h_out_w), None, 0):
            raise ConPtyError(f"CreatePipe(output) failed: winerror={int(bound.GetLastError())}")
        output_read, output_write = h_out_r.value, h_out_w.value

        size = COORD(cols, rows)
        hpc = wintypes.HANDLE()
        hr = int(bound.CreatePseudoConsole(size, input_read, output_write, 0, ctypes.byref(hpc)))
        if hr != S_OK:
            raise ConPtyError(f"CreatePseudoConsole failed: HRESULT=0x{hr & 0xFFFFFFFF:08X}")
        hpcon = hpc.value

        # PTY-side ends are duplicated into conhost; drop our refs early so
        # channel teardown is detectable (Microsoft sample + Learn docs).
        _close_handle(bound, input_read)
        input_read = None
        _close_handle(bound, output_write)
        output_write = None

        bytes_required = _SIZE_T(0)
        # First call fails by design and fills bytes_required.
        bound.InitializeProcThreadAttributeList(None, 1, 0, ctypes.byref(bytes_required))
        need = int(bytes_required.value)
        if need <= 0:
            raise ConPtyError("InitializeProcThreadAttributeList size query failed")
        attr_buf = (ctypes.c_byte * need)()
        attr_list = ctypes.cast(attr_buf, ctypes.c_void_p)
        if not bound.InitializeProcThreadAttributeList(attr_list, 1, 0, ctypes.byref(bytes_required)):
            raise ConPtyError(
                f"InitializeProcThreadAttributeList failed: winerror={int(bound.GetLastError())}"
            )
        attr_initialized = True

        # PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE: pass the HPCON handle *value* as
        # lpValue (EchoCon / node-pty / Rust std). Do NOT pass byref(local HANDLE) —
        # that is a pointer-to-temporary; UpdateProcThreadAttribute requires the
        # value remain valid until DeleteProcThreadAttributeList, and the working
        # ABI treats lpValue as the HPCON bits with cbSize=sizeof(HPCON).
        hpcon_lp_value = ctypes.c_void_p(_handle_int(hpcon))
        if not bound.UpdateProcThreadAttribute(
            attr_list,
            0,
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
            hpcon_lp_value,
            ctypes.sizeof(wintypes.HANDLE),
            None,
            None,
        ):
            raise ConPtyError(
                f"UpdateProcThreadAttribute(PSEUDOCONSOLE) failed: winerror={int(bound.GetLastError())}"
            )

        si = STARTUPINFOEXW()
        ctypes.memset(ctypes.byref(si), 0, ctypes.sizeof(si))
        si.StartupInfo.cb = ctypes.sizeof(STARTUPINFOEXW)
        # STARTF_USESTDHANDLES with NULL std handles avoids inheriting the
        # parent console and bypassing the pseudoconsole (common ConPTY pitfall).
        si.StartupInfo.dwFlags = STARTF_USESTDHANDLES
        si.lpAttributeList = attr_list

        pi = PROCESS_INFORMATION()
        ctypes.memset(ctypes.byref(pi), 0, ctypes.sizeof(pi))

        cmd_buf = ctypes.create_unicode_buffer(cmdline)
        env_ptr = None
        creation_flags = EXTENDED_STARTUPINFO_PRESENT
        if env is not None:
            env_blob = build_env_block(env)
            env_ptr = (ctypes.c_char * len(env_blob)).from_buffer_copy(env_blob)
            creation_flags |= CREATE_UNICODE_ENVIRONMENT

        cwd_w = cwd if cwd else None
        if not bound.CreateProcessW(
            None,
            cmd_buf,
            None,
            None,
            False,
            creation_flags,
            env_ptr,
            cwd_w,
            ctypes.byref(si.StartupInfo),
            ctypes.byref(pi),
        ):
            raise ConPtyError(f"CreateProcessW failed: winerror={int(bound.GetLastError())}")

        # EchoCon: attribute list only needed through CreateProcess — free it
        # before returning so ClosePseudoConsole never races a live attr list.
        try:
            bound.DeleteProcThreadAttributeList(attr_list)
        except Exception:
            pass
        attr_initialized = False
        attr_list = None
        attr_buf = None

        session = ConPtySession(
            api=bound,
            hpcon=hpcon,
            input_write=input_write,
            output_read=output_read,
            h_process=pi.hProcess,
            h_thread=pi.hThread,
            pid=int(pi.dwProcessId),
            attr_buf=None,
            attr_list=None,
            cols=cols,
            rows=rows,
        )
        # Ownership transferred to session. HPCON stays open until close_pseudoconsole.
        # Attribute list already deleted above (EchoCon order); hpcon_lp_value only
        # needed through Update + CreateProcess (still in scope until return).
        return session
    except Exception:
        # Partial-failure ownership: tear down everything we still hold.
        # Order: attr list before ClosePseudoConsole (list may reference HPCON).
        if pi is not None:
            try:
                if _is_valid_handle(pi.hProcess):
                    bound.TerminateProcess(pi.hProcess, 1)
            except Exception:
                pass
            _close_handle(bound, pi.hThread)
            _close_handle(bound, pi.hProcess)
        if attr_initialized and attr_list is not None:
            try:
                bound.DeleteProcThreadAttributeList(attr_list)
            except Exception:
                pass
            attr_list = None
            attr_buf = None
            attr_initialized = False
        if hpcon is not None:
            try:
                bound.ClosePseudoConsole(hpcon)
            except Exception:
                pass
        for h in (input_read, input_write, output_read, output_write):
            _close_handle(bound, h)
        raise


def require_conpty_support(*, platform: Optional[str] = None, api_factory: Optional[Callable[[], Any]] = None) -> None:
    """Fail closed if this platform cannot host a real ConPTY session."""
    plat = sys.platform if platform is None else platform
    if plat != "win32":
        raise ConPtyError(f"ConPTY requires win32 (platform={plat!r})")
    factory = api_factory or ConPtyApi
    try:
        factory()
    except ConPtyError:
        raise
    except Exception as error:
        raise ConPtyError(f"ConPTY API unavailable: {error}") from error
