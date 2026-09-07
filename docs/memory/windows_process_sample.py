"""Explicit-PID Windows process counters via ctypes Win32 (no third-party deps).

Opens the given local PID with the least necessary rights, reads creation
FILETIME as stable identity, cumulative user/kernel CPU from GetProcessTimes,
and current WorkingSetSize from GetProcessMemoryInfo. Exited processes are
detected with a zero-timeout WaitForSingleObject on the process handle — not
by trusting STILL_ACTIVE exit codes alone. Owned handles are closed on every
path. No name/argv/env discovery and no PowerShell/subprocess sampling.
"""
from __future__ import annotations

import ctypes
import functools
import math
import sys
from ctypes import wintypes
from typing import Any, Callable, Dict, Optional

from server_resources import SampleError

# Least rights for times + memory counters + exit wait.
PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
PROCESS_VM_READ = 0x0010
PROCESS_SYNCHRONIZE = 0x00100000
PROCESS_ACCESS = (
    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ | PROCESS_SYNCHRONIZE
)

WAIT_OBJECT_0 = 0x00000000
WAIT_TIMEOUT = 0x00000102
WAIT_FAILED = 0xFFFFFFFF

ERROR_ACCESS_DENIED = 5
ERROR_INVALID_PARAMETER = 87

# FILETIME CPU durations and wall identity: 100 ns units.
_FILETIME_TICKS_PER_SECOND = 10_000_000


# Windows DWORD is always 32-bit; do not use host wintypes.DWORD on LP64 Unix
# where c_ulong is 64-bit — layout must match the Windows ABI for tests + Win32.
_DWORD = ctypes.c_uint32
_SIZE_T = ctypes.c_size_t  # pointer-width on the running process (x64 Windows: 8)


class FILETIME(ctypes.Structure):
    _fields_ = [
        ("dwLowDateTime", _DWORD),
        ("dwHighDateTime", _DWORD),
    ]


class PROCESS_MEMORY_COUNTERS(ctypes.Structure):
    """PROCESS_MEMORY_COUNTERS with pointer-width SIZE_T fields."""

    _fields_ = [
        ("cb", _DWORD),
        ("PageFaultCount", _DWORD),
        ("PeakWorkingSetSize", _SIZE_T),
        ("WorkingSetSize", _SIZE_T),
        ("QuotaPeakPagedPoolUsage", _SIZE_T),
        ("QuotaPagedPoolUsage", _SIZE_T),
        ("QuotaPeakNonPagedPoolUsage", _SIZE_T),
        ("QuotaNonPagedPoolUsage", _SIZE_T),
        ("PagefileUsage", _SIZE_T),
        ("PeakPagefileUsage", _SIZE_T),
    ]


def filetime_to_u64(ft: FILETIME) -> int:
    """Pack FILETIME high/low DWORD parts into a 64-bit tick count."""
    return (int(ft.dwHighDateTime) << 32) | int(ft.dwLowDateTime)


def filetime_parts_to_seconds(high: int, low: int) -> float:
    """Convert GetProcessTimes user/kernel FILETIME parts to seconds.

    Those values are CPU durations in 100 ns units, not wall time since 1601.
    """
    if type(high) is not int or type(low) is not int:
        raise SampleError("FILETIME parts must be integers")
    if high < 0 or low < 0 or high > 0xFFFFFFFF or low > 0xFFFFFFFF:
        raise SampleError("FILETIME parts out of DWORD range")
    ticks = (high << 32) | low
    return ticks / float(_FILETIME_TICKS_PER_SECOND)


def filetime_to_seconds(ft: FILETIME) -> float:
    return filetime_parts_to_seconds(int(ft.dwHighDateTime), int(ft.dwLowDateTime))


def creation_identity(ft: FILETIME) -> str:
    """Stable process identity from creation FILETIME ticks (not a host alias)."""
    ticks = filetime_to_u64(ft)
    return f"windows_creation_filetime:{ticks}"


class Win32Api:
    """Thin bound Win32 surface; tests inject fakes instead of this class."""

    def __init__(self) -> None:
        if sys.platform != "win32":
            raise SampleError("Windows process backend is supported only on win32")
        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        psapi = ctypes.WinDLL("psapi", use_last_error=True)

        self.OpenProcess = kernel32.OpenProcess
        self.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        self.OpenProcess.restype = wintypes.HANDLE

        self.CloseHandle = kernel32.CloseHandle
        self.CloseHandle.argtypes = [wintypes.HANDLE]
        self.CloseHandle.restype = wintypes.BOOL

        self.GetProcessTimes = kernel32.GetProcessTimes
        self.GetProcessTimes.argtypes = [
            wintypes.HANDLE,
            ctypes.POINTER(FILETIME),
            ctypes.POINTER(FILETIME),
            ctypes.POINTER(FILETIME),
            ctypes.POINTER(FILETIME),
        ]
        self.GetProcessTimes.restype = wintypes.BOOL

        self.WaitForSingleObject = kernel32.WaitForSingleObject
        self.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
        self.WaitForSingleObject.restype = wintypes.DWORD

        self.GetLastError = kernel32.GetLastError
        self.GetLastError.argtypes = []
        self.GetLastError.restype = wintypes.DWORD

        self.GetProcessMemoryInfo = psapi.GetProcessMemoryInfo
        self.GetProcessMemoryInfo.argtypes = [
            wintypes.HANDLE,
            ctypes.POINTER(PROCESS_MEMORY_COUNTERS),
            wintypes.DWORD,
        ]
        self.GetProcessMemoryInfo.restype = wintypes.BOOL


@functools.lru_cache(maxsize=1)
def _default_api() -> Win32Api:
    return Win32Api()


def _validate_pid(pid: int) -> int:
    # Windows process IDs are DWORD; reject bool and non-positive.
    if type(pid) is not int or isinstance(pid, bool) or pid <= 0:
        raise SampleError("PID must be a positive integer")
    if pid > 0xFFFFFFFF:
        raise SampleError("PID exceeds Windows DWORD range")
    return pid


def _fail_open(pid: int, last_error: int) -> None:
    if last_error in (ERROR_ACCESS_DENIED,):
        raise SampleError(f"access denied opening process PID {pid} (Win32 error {last_error})")
    if last_error in (ERROR_INVALID_PARAMETER, 0):
        # OpenProcess commonly returns NULL with ERROR_INVALID_PARAMETER for missing PID.
        raise SampleError(f"process PID {pid} not found or invalid (Win32 error {last_error})")
    raise SampleError(f"OpenProcess failed for PID {pid} (Win32 error {last_error})")


# Liveness-only rights: query + synchronize. No VM_READ (not needed to wait/exit-check).
_PROCESS_ALIVE_ACCESS = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE


def process_is_alive(
    pid: int,
    *,
    api: Optional[Any] = None,
    open_process: Optional[Callable[..., Any]] = None,
    close_handle: Optional[Callable[..., Any]] = None,
    wait_for_single_object: Optional[Callable[..., Any]] = None,
    get_last_error: Optional[Callable[[], int]] = None,
) -> bool:
    """Return whether an explicit local Windows PID is still running.

    Uses OpenProcess + zero-timeout WaitForSingleObject on the process handle.
    Never uses ``os.kill(pid, 0)``: on Windows signal value 0 is CTRL_C_EVENT and
    routes through GenerateConsoleCtrlEvent (console-group side effects), and it
    does not honestly distinguish exited-but-still-handled process objects.

    Semantics:
    - OpenProcess fails with ERROR_INVALID_PARAMETER → dead / never existed
    - OpenProcess fails with ERROR_ACCESS_DENIED → treat as alive (cannot claim dead)
    - WaitForSingleObject == WAIT_OBJECT_0 → exited (handle still openable)
    - WaitForSingleObject == WAIT_TIMEOUT → still running
    """
    if type(pid) is not int or isinstance(pid, bool) or pid <= 0:
        return False
    if pid > 0xFFFFFFFF:
        return False

    bound = api
    if bound is None and any(
        x is None
        for x in (open_process, close_handle, wait_for_single_object, get_last_error)
    ):
        if sys.platform != "win32":
            raise SampleError("Windows process liveness is supported only on win32")
        bound = _default_api()

    def _resolve(name: str, override, attr: str):
        if override is not None:
            return override
        if bound is None:
            raise SampleError(f"Windows API {name} not available")
        return getattr(bound, attr)

    _open = _resolve("OpenProcess", open_process, "OpenProcess")
    _close = _resolve("CloseHandle", close_handle, "CloseHandle")
    _wait = _resolve("WaitForSingleObject", wait_for_single_object, "WaitForSingleObject")
    _gle = _resolve("GetLastError", get_last_error, "GetLastError")

    handle = _open(_PROCESS_ALIVE_ACCESS, False, int(pid))
    if not handle:
        err = int(_gle() or 0)
        if err in (ERROR_INVALID_PARAMETER, 0):
            return False
        if err == ERROR_ACCESS_DENIED:
            # Alive-or-inaccessible: fail closed toward "still present" so cleanup
            # does not skip owned children we cannot prove dead.
            return True
        # Unknown open failure: do not claim the PID is gone.
        return True

    closed = False
    try:
        wait_rc = int(_wait(handle, 0))
        if wait_rc == WAIT_OBJECT_0:
            return False
        if wait_rc == WAIT_TIMEOUT:
            return True
        # WAIT_FAILED or unexpected: cannot prove dead.
        return True
    finally:
        if not closed:
            closed = True
            try:
                _close(handle)
            except Exception:
                pass


def sample_process(
    pid: int,
    timeout: Optional[float] = None,
    *,
    api: Optional[Any] = None,
    open_process: Optional[Callable[..., Any]] = None,
    close_handle: Optional[Callable[..., Any]] = None,
    get_process_times: Optional[Callable[..., Any]] = None,
    get_process_memory_info: Optional[Callable[..., Any]] = None,
    wait_for_single_object: Optional[Callable[..., Any]] = None,
    get_last_error: Optional[Callable[[], int]] = None,
) -> Dict[str, Any]:
    """Sample one explicit local Windows PID.

    Injectable callables support deterministic unit tests without Win32.
    ``timeout`` is accepted for API parity with other backends; the Win32 path
    is non-blocking (zero-timeout exit wait only) and does not spawn helpers.
    """
    _validate_pid(pid)
    if timeout is not None:
        if type(timeout) not in (int, float) or isinstance(timeout, bool):
            raise SampleError("sample timeout must be a finite number when provided")
        if not math.isfinite(float(timeout)) or float(timeout) <= 0:
            raise SampleError("sample timeout must be finite and positive")

    bound = api
    if bound is None and any(
        x is None
        for x in (
            open_process,
            close_handle,
            get_process_times,
            get_process_memory_info,
            wait_for_single_object,
            get_last_error,
        )
    ):
        if sys.platform != "win32":
            raise SampleError("Windows process backend is supported only on win32")
        bound = _default_api()

    def _resolve(name: str, override, attr: str):
        if override is not None:
            return override
        if bound is None:
            raise SampleError(f"Windows API {name} not available")
        return getattr(bound, attr)

    _open = _resolve("OpenProcess", open_process, "OpenProcess")
    _close = _resolve("CloseHandle", close_handle, "CloseHandle")
    _times = _resolve("GetProcessTimes", get_process_times, "GetProcessTimes")
    _mem = _resolve(
        "GetProcessMemoryInfo", get_process_memory_info, "GetProcessMemoryInfo"
    )
    _wait = _resolve(
        "WaitForSingleObject", wait_for_single_object, "WaitForSingleObject"
    )
    _gle = _resolve("GetLastError", get_last_error, "GetLastError")

    handle = _open(PROCESS_ACCESS, False, int(pid))
    # ctypes may return int 0 or None-like for NULL HANDLE.
    if not handle:
        err = int(_gle() or 0)
        _fail_open(pid, err)

    closed = False
    primary_error: Optional[BaseException] = None
    result: Optional[Dict[str, Any]] = None

    def _close_owned() -> None:
        nonlocal closed
        if closed:
            return
        closed = True
        try:
            ok = _close(handle)
        except Exception as exc:  # noqa: BLE001 — still report close failure
            raise SampleError(f"CloseHandle failed for PID {pid}: {exc}") from exc
        if not ok:
            raise SampleError(
                f"CloseHandle failed for PID {pid} (Win32 error {int(_gle() or 0)})"
            )

    try:
        # Exit detection: signaled process handle means the process has exited.
        wait_rc = int(_wait(handle, 0))
        if wait_rc == WAIT_FAILED:
            raise SampleError(
                f"WaitForSingleObject failed for PID {pid} (Win32 error {int(_gle() or 0)})"
            )
        if wait_rc == WAIT_OBJECT_0:
            raise SampleError(f"process PID {pid} has exited")
        if wait_rc != WAIT_TIMEOUT:
            raise SampleError(
                f"unexpected WaitForSingleObject result {wait_rc} for PID {pid}"
            )

        creation = FILETIME()
        exit_t = FILETIME()
        kernel = FILETIME()
        user = FILETIME()
        if not _times(
            handle,
            ctypes.byref(creation),
            ctypes.byref(exit_t),
            ctypes.byref(kernel),
            ctypes.byref(user),
        ):
            raise SampleError(
                f"GetProcessTimes failed for PID {pid} (Win32 error {int(_gle() or 0)})"
            )

        if filetime_to_u64(creation) == 0:
            raise SampleError(f"process PID {pid} creation FILETIME unavailable")

        pmc = PROCESS_MEMORY_COUNTERS()
        pmc.cb = ctypes.sizeof(PROCESS_MEMORY_COUNTERS)
        if not _mem(handle, ctypes.byref(pmc), pmc.cb):
            raise SampleError(
                f"GetProcessMemoryInfo failed for PID {pid} (Win32 error {int(_gle() or 0)})"
            )

        resident = int(pmc.WorkingSetSize)
        if resident <= 0:
            raise SampleError(
                f"process PID {pid} reported zero WorkingSetSize (not a valid resident sample)"
            )

        user_s = filetime_to_seconds(user)
        system_s = filetime_to_seconds(kernel)
        if not (math.isfinite(user_s) and math.isfinite(system_s)) or user_s < 0 or system_s < 0:
            raise SampleError(f"process PID {pid} produced non-finite CPU counters")

        identity = creation_identity(creation)
        result = {
            "start_identity": identity,
            "resident_bytes": resident,
            "user_s": user_s,
            "system_s": system_s,
            "provenance": {
                "os": "windows",
                "backend": "win32_ctypes",
                "rss": (
                    f"GetProcessMemoryInfo(OpenProcess({pid})).WorkingSetSize; "
                    "current working set bytes (not PeakWorkingSetSize, PagefileUsage, or PrivateUsage)"
                ),
                "cpu": (
                    "GetProcessTimes user/kernel FILETIME CPU durations; "
                    f"100 ns ticks ÷ {_FILETIME_TICKS_PER_SECOND} → seconds"
                ),
                "identity": (
                    "GetProcessTimes creation FILETIME (100 ns since 1601-01-01 UTC) "
                    "as windows_creation_filetime:<u64 ticks>; PID supplied by caller"
                ),
                "exit_detection": (
                    "WaitForSingleObject(process, 0) == WAIT_OBJECT_0 means exited; "
                    "STILL_ACTIVE exit code is not trusted alone"
                ),
                "open_access": (
                    "PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ | PROCESS_SYNCHRONIZE"
                ),
                "sampling_subprocesses": False,
                "name_argv_env_discovery": False,
                "filetime_ticks_per_second": _FILETIME_TICKS_PER_SECOND,
                "creation_filetime_ticks": filetime_to_u64(creation),
                "structure_sizes": {
                    "FILETIME": ctypes.sizeof(FILETIME),
                    "PROCESS_MEMORY_COUNTERS": ctypes.sizeof(PROCESS_MEMORY_COUNTERS),
                    "SIZE_T": ctypes.sizeof(ctypes.c_size_t),
                    "HANDLE": ctypes.sizeof(wintypes.HANDLE),
                },
            },
        }
    except SampleError as exc:
        primary_error = exc
    finally:
        close_error: Optional[BaseException] = None
        try:
            _close_owned()
        except SampleError as exc:
            close_error = exc
        if primary_error is not None:
            raise primary_error
        if close_error is not None:
            raise close_error
    assert result is not None
    return result
