"""Explicit-PID Windows parent lookup via Toolhelp32 (no name/argv scan).

Returns the th32ParentProcessID for a local PID. Missing PID, access failure,
or incomplete snapshot fails closed. No discovery by image name.
"""
from __future__ import annotations

import ctypes
import sys
from ctypes import wintypes
from typing import Any, Callable, Optional

from server_resources import SampleError

# Windows ABI fixed widths (do not use host c_ulong on LP64).
_DWORD = ctypes.c_uint32
_LONG = ctypes.c_int32
_ULONG_PTR = ctypes.c_uint64 if ctypes.sizeof(ctypes.c_void_p) == 8 else ctypes.c_uint32

TH32CS_SNAPPROCESS = 0x00000002
INVALID_HANDLE_VALUE = ctypes.c_void_p(-1).value


class PROCESSENTRY32W(ctypes.Structure):
    _fields_ = [
        ("dwSize", _DWORD),
        ("cntUsage", _DWORD),
        ("th32ProcessID", _DWORD),
        ("th32DefaultHeapID", _ULONG_PTR),
        ("th32ModuleID", _DWORD),
        ("cntThreads", _DWORD),
        ("th32ParentProcessID", _DWORD),
        ("pcPriClassBase", _LONG),
        ("dwFlags", _DWORD),
        ("szExeFile", wintypes.WCHAR * 260),
    ]


class ToolhelpApi:
    """Bound Toolhelp32 surface; tests inject fakes instead of this class."""

    def __init__(self) -> None:
        if sys.platform != "win32":
            raise SampleError("Windows parent lookup is supported only on win32")
        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)

        self.CreateToolhelp32Snapshot = kernel32.CreateToolhelp32Snapshot
        self.CreateToolhelp32Snapshot.argtypes = [_DWORD, _DWORD]
        self.CreateToolhelp32Snapshot.restype = wintypes.HANDLE

        self.Process32FirstW = kernel32.Process32FirstW
        self.Process32FirstW.argtypes = [wintypes.HANDLE, ctypes.POINTER(PROCESSENTRY32W)]
        self.Process32FirstW.restype = wintypes.BOOL

        self.Process32NextW = kernel32.Process32NextW
        self.Process32NextW.argtypes = [wintypes.HANDLE, ctypes.POINTER(PROCESSENTRY32W)]
        self.Process32NextW.restype = wintypes.BOOL

        self.CloseHandle = kernel32.CloseHandle
        self.CloseHandle.argtypes = [wintypes.HANDLE]
        self.CloseHandle.restype = wintypes.BOOL

        self.GetLastError = kernel32.GetLastError
        self.GetLastError.argtypes = []
        self.GetLastError.restype = wintypes.DWORD


def _validate_pid(pid: int) -> int:
    if type(pid) is not int or isinstance(pid, bool) or pid <= 0:
        raise SampleError("PID must be a positive integer")
    if pid > 0xFFFFFFFF:
        raise SampleError("PID exceeds Windows DWORD range")
    return pid


def parent_pid(
    pid: int,
    *,
    api: Optional[Any] = None,
    create_snapshot: Optional[Callable[..., Any]] = None,
    process_first: Optional[Callable[..., Any]] = None,
    process_next: Optional[Callable[..., Any]] = None,
    close_handle: Optional[Callable[..., Any]] = None,
    get_last_error: Optional[Callable[[], int]] = None,
) -> int:
    """Return parent PID for an explicit local Windows PID.

    Injectable callables support deterministic unit tests without Toolhelp32.
    """
    pid = _validate_pid(pid)
    bound = api
    if bound is None and any(
        x is None
        for x in (create_snapshot, process_first, process_next, close_handle, get_last_error)
    ):
        bound = ToolhelpApi()
    if bound is not None:
        create_snapshot = bound.CreateToolhelp32Snapshot
        process_first = bound.Process32FirstW
        process_next = bound.Process32NextW
        close_handle = bound.CloseHandle
        get_last_error = bound.GetLastError
    assert create_snapshot and process_first and process_next and close_handle and get_last_error

    snap = create_snapshot(TH32CS_SNAPPROCESS, 0)
    if snap in (None, 0, INVALID_HANDLE_VALUE, -1):
        err = int(get_last_error())
        raise SampleError(f"CreateToolhelp32Snapshot failed (Win32 error {err})")

    try:
        entry = PROCESSENTRY32W()
        entry.dwSize = ctypes.sizeof(PROCESSENTRY32W)
        if not process_first(snap, ctypes.byref(entry)):
            err = int(get_last_error())
            raise SampleError(f"Process32FirstW failed (Win32 error {err})")
        while True:
            if int(entry.th32ProcessID) == pid:
                parent = int(entry.th32ParentProcessID)
                if parent < 0:
                    raise SampleError(f"invalid parent PID for process {pid}")
                return parent
            if not process_next(snap, ctypes.byref(entry)):
                break
        raise SampleError(f"process PID {pid} not found in process snapshot")
    finally:
        close_handle(snap)
