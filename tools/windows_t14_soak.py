"""Windows adapter for the unchanged T1.4 soak runner and summary contract.

The product harness, proxy protocol/faults, and summary functions remain the
committed Linux-independent sources. This adapter supplies Windows working-set,
thread-name, and process-handle samples. It keeps Windows handles separate from
Linux file descriptors and does not claim whole-connection allocator high-water.
"""

from __future__ import annotations

import argparse
import ctypes
from ctypes import wintypes
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import queue
import ssl
import signal
import subprocess
import sys
import threading
import time
import traceback

import t14_windows_thread_evidence

ENGINE_PUBLIC_MODULUS_HEX = (
    "DF4F566B360BC249642AEFD65D97F8BB1591751140498DB5047102C7ED49A760D"
    "390918F4734F82D698D7FB059160DA98C9C6D6E01AFB05FE065BEF27B7EAC4F"
    "68F6FD95529A1713D4DBC702953ACC82A042FC148E01057B77BAB75D43B1B47"
    "D817446E32FB7BF0A1F6873E33ADA30961F63B0D1409B3719203312378EF2A259"
)
ENGINE_PRIVATE_KEY_SHA256 = "7a93286f89512ca56e675ce1ebdf575783a4b233b595a60df302c7b691efc1b1"
ENGINE_WORLD_SHA256 = "2f751f4e7736ac67933f2da100dd7e5f706e895f3e890ae2bc6597259da1a74d"
EXPECTED_NODE_ID = 289
WINDOWS_CREATE_NEW_PROCESS_GROUP = 0x00000200
TH32CS_SNAPTHREAD = 0x00000004
THREAD_QUERY_INFORMATION = 0x0040
THREAD_QUERY_LIMITED_INFORMATION = 0x0800
PROCESS_QUERY_INFORMATION = 0x0400
PROCESS_VM_READ = 0x0010
SYNCHRONIZE = 0x00100000
WAIT_TIMEOUT = 0x00000102
WAIT_OBJECT_0 = 0x00000000
MINIDUMP_WITH_THREAD_INFO = 0x00001000
ERROR_NO_MORE_FILES = 18
ERROR_INVALID_PARAMETER = 87
ERROR_NOT_FOUND = 1168
EXPECTED_BASELINE_HEAD = "42a9102f33f1fa9b89249c44ee3f7151f4749fe9"
EXPECTED_CANDIDATE_PRODUCT_BASE = "8b4f19ed3ef0724ba6fd785b6070a26e35a227c9"
EXPECTED_BASELINE_PRODUCT_TREE = "de52d10940fbfa1c6ae67dc50d40392217765281"
EXPECTED_CANDIDATE_PRODUCT_TREE = "b6885eee7def7aa3b9d9942a125723e2301d8d38"
EXPECTED_BASELINE_FORK = "86b8520a35075c15166bb2279bd4db00d672e098"
EXPECTED_CANDIDATE_FORK = "06c2d0fbc3ac73b75a33a15fc48d5963093a4892"
EXPECTED_PATCH_SHA256 = "4e59afc3cae64541a05dcda80f9344fa61992956345e0f4581d7828bd9fb6169"
EXPECTED_RELOG_PATCH_SHA256 = "30049c692be679352b78b19220798feed0db17b04a4b593d8b0af376f9a8b43f"
NAV_BUILD_SNAPSHOT_VERSION = "37214163f1e6ceca"
NAV_BUILD_SNAPSHOT_MANIFEST_SHA256 = "f9bb327cc4335350ba635550f520f39b7a504fb37bb4cf987b0a59efca8d9f41"
EXPECTED_PATCH_PATHS = {
    "crates/host-play/tests/support/t14_final_barrier.rs",
    "crates/host-play/tests/t14_soak_live.rs",
}
EXPECTED_HARNESS_BLOBS = {
    "crates/host-play/tests/t14_soak_live.rs": "1ef2792e815b44632896735236bba9ac8ddc87d2",
    "crates/host-play/tests/support/t14_final_barrier.rs": "a174e8e6397932d1c6eef4bba3da2f8194e26288",
}
EXPECTED_CANDIDATE_TOOL_BLOBS = {
    "tools/t14_soak_run.py": "f0396026942c2201f152354411212b9a5c23a3c0",
    "tools/t14_soak_proxy.py": "3ef6fc69398dc1d2cd670ca964935e2a086b4937",
    "T1.4/soak-config.json": "ba30e47902b6c6cb1213483dc5e0fe3b09c9b295",
}
EXPECTED_CANDIDATE_HARNESS_ONLY_CHANGES = {
    "tools/windows_t14_soak.py",
    "tools/t14_windows_thread_evidence.py",
    "tools/test_windows_t14_soak.py",
    "tools/test_t14_windows_thread_evidence.py",
    "tools/t14_soak_summary.py",
    "tools/test_t14_soak_summary.py",
}





class PROCESS_MEMORY_COUNTERS_EX(ctypes.Structure):
    _fields_ = [
        ("cb", wintypes.DWORD),
        ("PageFaultCount", wintypes.DWORD),
        ("PeakWorkingSetSize", ctypes.c_size_t),
        ("WorkingSetSize", ctypes.c_size_t),
        ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
        ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
        ("PagefileUsage", ctypes.c_size_t),
        ("PeakPagefileUsage", ctypes.c_size_t),
        ("PrivateUsage", ctypes.c_size_t),
    ]


class THREADENTRY32(ctypes.Structure):
    _fields_ = [
        ("dwSize", wintypes.DWORD),
        ("cntUsage", wintypes.DWORD),
        ("th32ThreadID", wintypes.DWORD),
        ("th32OwnerProcessID", wintypes.DWORD),
        ("tpBasePri", wintypes.LONG),
        ("tpDeltaPri", wintypes.LONG),
        ("dwFlags", wintypes.DWORD),
    ]


def win32_api():
    if os.name != "nt":
        raise RuntimeError("Windows sampler must run on Windows")
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    psapi = ctypes.WinDLL("psapi", use_last_error=True)
    kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel32.OpenProcess.restype = wintypes.HANDLE
    kernel32.OpenThread.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel32.OpenThread.restype = wintypes.HANDLE
    kernel32.CreateToolhelp32Snapshot.argtypes = [wintypes.DWORD, wintypes.DWORD]
    kernel32.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
    kernel32.Thread32First.argtypes = [wintypes.HANDLE, ctypes.POINTER(THREADENTRY32)]
    kernel32.Thread32First.restype = wintypes.BOOL
    kernel32.Thread32Next.argtypes = [wintypes.HANDLE, ctypes.POINTER(THREADENTRY32)]
    kernel32.Thread32Next.restype = wintypes.BOOL
    kernel32.GetThreadDescription.argtypes = [wintypes.HANDLE, ctypes.POINTER(ctypes.c_void_p)]
    kernel32.GetThreadDescription.restype = ctypes.c_long
    kernel32.GetThreadTimes.argtypes = [
        wintypes.HANDLE, ctypes.POINTER(wintypes.FILETIME), ctypes.POINTER(wintypes.FILETIME),
        ctypes.POINTER(wintypes.FILETIME), ctypes.POINTER(wintypes.FILETIME),
    ]
    kernel32.GetThreadTimes.restype = wintypes.BOOL
    kernel32.GetProcessHandleCount.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
    kernel32.GetProcessHandleCount.restype = wintypes.BOOL
    kernel32.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
    kernel32.WaitForSingleObject.restype = wintypes.DWORD
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel32.CloseHandle.restype = wintypes.BOOL
    kernel32.LocalFree.argtypes = [wintypes.HLOCAL]
    kernel32.LocalFree.restype = wintypes.HLOCAL
    psapi.GetProcessMemoryInfo.argtypes = [
        wintypes.HANDLE, ctypes.POINTER(PROCESS_MEMORY_COUNTERS_EX), wintypes.DWORD,
    ]
    psapi.GetProcessMemoryInfo.restype = wintypes.BOOL
    return kernel32, psapi


def win_error(operation: str) -> OSError:
    code = ctypes.get_last_error()
    return OSError(code, f"{operation}: {ctypes.FormatError(code).strip()}")


def process_metrics(kernel32, psapi, process_handle) -> tuple[int, int]:
    counters = PROCESS_MEMORY_COUNTERS_EX()
    counters.cb = ctypes.sizeof(counters)
    if not psapi.GetProcessMemoryInfo(process_handle, ctypes.byref(counters), counters.cb):
        raise win_error("GetProcessMemoryInfo")
    handle_count = wintypes.DWORD()
    if not kernel32.GetProcessHandleCount(process_handle, ctypes.byref(handle_count)):
        raise win_error("GetProcessHandleCount")
    return int(counters.WorkingSetSize), int(handle_count.value)


def filetime_value(value: wintypes.FILETIME) -> int:
    return (int(value.dwHighDateTime) << 32) | int(value.dwLowDateTime)


def thread_census(kernel32, process_id: int) -> tuple[list[dict], int]:
    snapshot = kernel32.CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0)
    if snapshot == wintypes.HANDLE(-1).value or not snapshot:
        raise win_error("CreateToolhelp32Snapshot")
    rows = []
    try:
        entry = THREADENTRY32()
        entry.dwSize = ctypes.sizeof(entry)
        ctypes.set_last_error(0)
        if not kernel32.Thread32First(snapshot, ctypes.byref(entry)):
            raise win_error("Thread32First")
        while True:
            thread_times_error = None
            if entry.th32OwnerProcessID == process_id:
                thread_id = int(entry.th32ThreadID)
                access = THREAD_QUERY_INFORMATION | THREAD_QUERY_LIMITED_INFORMATION
                ctypes.set_last_error(0)
                thread_handle = kernel32.OpenThread(access, False, thread_id)
                if thread_handle:
                    try:
                        created = wintypes.FILETIME()
                        exited = wintypes.FILETIME()
                        kernel = wintypes.FILETIME()
                        user = wintypes.FILETIME()
                        ctypes.set_last_error(0)
                        if not kernel32.GetThreadTimes(
                            thread_handle, ctypes.byref(created), ctypes.byref(exited),
                            ctypes.byref(kernel), ctypes.byref(user),
                        ):
                            code = ctypes.get_last_error()
                            if code not in (6, ERROR_INVALID_PARAMETER, ERROR_NOT_FOUND):
                                thread_times_error = OSError(
                                    code,
                                    f"GetThreadTimes({thread_id}): {ctypes.FormatError(code).strip()}",
                                )
                        else:
                            name_pointer = ctypes.c_void_p()
                            hr = kernel32.GetThreadDescription(
                                thread_handle, ctypes.byref(name_pointer)
                            )
                            if hr < 0:
                                raise OSError(
                                    f"GetThreadDescription({thread_id}) "
                                    f"HRESULT=0x{hr & 0xffffffff:08x}"
                                )
                            try:
                                description = (
                                    ctypes.wstring_at(name_pointer.value)
                                    if name_pointer.value else ""
                                )
                            finally:
                                if name_pointer.value:
                                    kernel32.LocalFree(name_pointer)
                            rows.append({
                                "tid": thread_id,
                                "windows_thread_description": description,
                                "thread_creation_filetime": filetime_value(created),
                            })
                    finally:
                        kernel32.CloseHandle(thread_handle)
                else:
                    code = ctypes.get_last_error()
                    if code not in (6, ERROR_INVALID_PARAMETER, ERROR_NOT_FOUND):
                        raise OSError(
                            code,
                            f"OpenThread({thread_id}): {ctypes.FormatError(code).strip()}",
                        )
            ctypes.set_last_error(0)
            if not kernel32.Thread32Next(snapshot, ctypes.byref(entry)):
                code = ctypes.get_last_error()
                if code != ERROR_NO_MORE_FILES:
                    raise OSError(code, f"Thread32Next: {ctypes.FormatError(code).strip()}")
                if thread_times_error is not None:
                    raise thread_times_error
                break
            if thread_times_error is not None:
                raise thread_times_error
    finally:
        kernel32.CloseHandle(snapshot)
    if not rows:
        raise RuntimeError(f"no live threads observed for Windows process {process_id}")
    main = min(rows, key=lambda row: (row["thread_creation_filetime"], row["tid"]))
    main_tid = main["tid"]
    compatible = []
    for row in sorted(rows, key=lambda item: item["tid"]):
        is_main = row["tid"] == main_tid
        compatible.append({
            "tid": row["tid"],
            # Linux names the process leader with the test process prefix and
            # uses tid==pid. Windows has a separate main TID; retain the raw
            # description and identify that one observed leader explicitly.
            "comm": "t14_soak_live-" if is_main else row["windows_thread_description"][:15],
            "windows_thread_description": row["windows_thread_description"],
            "windows_process_main_thread": is_main,
            "windows_process_id": process_id,
            "thread_creation_filetime": row["thread_creation_filetime"],
        })
    return compatible, main_tid


def capture_thread_projection(process_handle, process_id: int, output: Path,
                              index: int, descriptions: dict[int, str]) -> dict:
    """Capture a thread-info minidump projection for first-seen unnamed threads."""
    import msvcrt

    provenance = output / "thread-provenance"
    provenance.mkdir(exist_ok=True)
    dump = provenance / f"snapshot-{index:04}.dmp"
    dbghelp = ctypes.WinDLL("dbghelp", use_last_error=True)
    write_dump = dbghelp.MiniDumpWriteDump
    write_dump.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.HANDLE,
                           wintypes.DWORD, ctypes.c_void_p, ctypes.c_void_p,
                           ctypes.c_void_p]
    write_dump.restype = wintypes.BOOL
    with dump.open("xb") as stream:
        file_handle = wintypes.HANDLE(msvcrt.get_osfhandle(stream.fileno()))
        if not write_dump(
            process_handle, process_id, file_handle, MINIDUMP_WITH_THREAD_INFO,
            None, None, None,
        ):
            raise win_error("MiniDumpWriteDump")
    projection = t14_windows_thread_evidence.analyze(dump, descriptions, process_id)
    path = provenance / f"snapshot-{index:04}.json"
    path.write_text(json.dumps(projection, indent=2) + "\n", encoding="utf-8")
    return projection


def bind_unnamed_provenance(threads: list[dict], process_id: int,
                           sample_ns: int, cache: dict[tuple[int, int, int], dict],
                           capture) -> None:
    """Bind evidence to live (PID, TID, creation FILETIME), never to a reused TID."""
    live = {(process_id, row["tid"], row["thread_creation_filetime"])
            for row in threads}
    for identity in tuple(cache):
        if identity not in live:
            del cache[identity]
    new = [row for row in threads
           if row["windows_thread_description"] == ""
           and not row["windows_process_main_thread"]
           and (process_id, row["tid"], row["thread_creation_filetime"]) not in cache]
    transient_errors = {}
    if new:
        try:
            projection = capture({row["tid"]: row["windows_thread_description"]
                                  for row in threads})
            if projection["sampled_pid"] != process_id:
                raise ValueError("thread projection process ID mismatch")
            captured = {}
            for observed in projection["threads"]:
                if isinstance(observed, dict) and isinstance(observed.get("tid"), int):
                    captured[observed["tid"]] = observed
            for row in new:
                identity = (process_id, row["tid"], row["thread_creation_filetime"])
                observed = captured.get(row["tid"])
                if observed is None:
                    error = "thread missing from matched minidump"
                elif "error" in observed:
                    error = str(observed["error"])
                elif observed.get("description_first_sample") != "":
                    error = "thread description does not match first sample"
                elif observed.get("thread_creation_filetime") != row["thread_creation_filetime"]:
                    error = "thread creation FILETIME does not match first sample"
                elif not all(field in observed for field in (
                    "start_module", "ip_module", "stack_window_bytes",
                    "candidate_return_address_module_counts",
                )):
                    error = "thread projection lacks required evidence fields"
                else:
                    error = None
                if error is not None:
                    transient_errors[identity] = {"error": error}
                    continue
                cache[identity] = {
                    "schema": projection["schema"],
                    "sampled_pid": projection["sampled_pid"],
                    "tid": row["tid"],
                    "thread_creation_filetime": row["thread_creation_filetime"],
                    "capture_sample_monotonic_ns": sample_ns,
                    "dump_path": projection["dump_path"],
                    "dump_sha256": projection["dump_sha256"],
                    "ip_module": observed["ip_module"],
                    "start_module": observed["start_module"],
                    "stack_window_bytes": observed["stack_window_bytes"],
                    "candidate_return_address_module_counts":
                        observed["candidate_return_address_module_counts"],
                    "limits": projection["limits"],
                }
        except (OSError, ValueError, KeyError, TypeError) as error:
            for row in new:
                identity = (process_id, row["tid"], row["thread_creation_filetime"])
                transient_errors[identity] = {
                    "error": f"{type(error).__name__}: {error}"
                }
    for row in threads:
        if row["windows_thread_description"] == "" and not row["windows_process_main_thread"]:
            identity = (process_id, row["tid"], row["thread_creation_filetime"])
            row["os_runtime_provenance"] = cache.get(identity, transient_errors.get(identity))


def write_jsonl_row(stream, value: dict) -> None:
    stream.write(json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n")
    stream.flush()

def executable_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def find_soak_test_executable(repo: Path, prebuild_log: Path) -> Path:
    pattern = re.compile(r"Executable\s+.*?\((.+?\.exe)\)", re.IGNORECASE)
    matches = []
    for line in prebuild_log.read_text(encoding="utf-8", errors="replace").splitlines():
        match = pattern.search(line)
        if not match:
            continue
        executable = Path(match.group(1).strip().strip('"'))
        if "t14_soak_live" not in executable.name.lower():
            continue
        if not executable.is_absolute():
            executable = repo / executable
        matches.append(executable.resolve(strict=True))
    if len(matches) != 1:
        raise RuntimeError(
            f"expected one Cargo t14_soak_live executable in {prebuild_log}, found {len(matches)}"
        )
    return matches[0]


def direct_soak_test_command(command: list[str], executable: Path) -> list[str]:
    try:
        separator = command.index("--")
    except ValueError as error:
        raise RuntimeError("Cargo soak test command has no Rust test argument separator") from error
    if (len(command) < 4 or command[0] != "cargo" or "--test" not in command
            or command[command.index("--test") + 1] != "t14_soak_live"
            or separator < 1 or command[separator - 1] != "t14_local_wss_soak"
            or command[separator + 1:] != ["--ignored", "--nocapture", "--test-threads=1"]):
        raise RuntimeError(f"unexpected Cargo soak test command: {command!r}")
    return [str(executable), command[separator - 1], *command[separator + 1:]]


def run_guest_ca_trust(ca: Path, output: Path, action: str, state: dict) -> None:
    """Trust only this arm's generated CA in the disposable guest machine store."""
    if os.name != "nt":
        raise RuntimeError("Windows guest CA trust cannot run on the host")
    pem = ca.read_text(encoding="ascii")
    der = ssl.PEM_cert_to_DER_cert(pem)
    thumbprint = hashlib.sha1(der).hexdigest().upper()
    if state.get("thumbprint_sha1") not in (None, thumbprint):
        raise RuntimeError("run-local CA changed between installation and removal")
    state.update({
        "store": "LocalMachine\\Root",
        "thumbprint_sha1": thumbprint,
        "ca_sha256": hashlib.sha256(ca.read_bytes()).hexdigest(),
    })
    environment = os.environ.copy()
    environment["BOT_T14_TEMP_CA_PEM"] = str(ca)
    environment["BOT_T14_TEMP_CA_SHA1"] = thumbprint
    if action == "install":
        script = (
            "$ErrorActionPreference='Stop'; "
            "$path='Cert:\\LocalMachine\\Root\\'+$env:BOT_T14_TEMP_CA_SHA1; "
            "if(Test-Path -LiteralPath $path){throw 'run CA already present in guest store'}; "
            "$cert=Import-Certificate -FilePath $env:BOT_T14_TEMP_CA_PEM "
            "-CertStoreLocation 'Cert:\\LocalMachine\\Root' -ErrorAction Stop; "
            "if(@($cert).Count -ne 1 -or $cert.Thumbprint -ne $env:BOT_T14_TEMP_CA_SHA1 "
            "-or -not (Test-Path -LiteralPath $path)){throw 'run CA import mismatch'}"
        )
    elif action == "remove":
        environment["BOT_T14_REQUIRE_TEMP_CA_REMOVAL"] = (
            "1" if state.get("install_verified") else "0"
        )
        script = (
            "$ErrorActionPreference='Stop'; "
            "$path='Cert:\\LocalMachine\\Root\\'+$env:BOT_T14_TEMP_CA_SHA1; "
            "if(Test-Path -LiteralPath $path){"
            "Remove-Item -LiteralPath $path -Force -ErrorAction Stop"
            "} elseif($env:BOT_T14_REQUIRE_TEMP_CA_REMOVAL -eq '1'){"
            "throw 'installed run CA missing before removal'"
            "}; "
            "if(Test-Path -LiteralPath $path){throw 'run CA remains trusted'}"
        )
    else:
        raise ValueError(f"unknown guest CA trust action: {action}")
    command = ["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", script]
    if action == "install":
        state["install_attempted"] = True
    try:
        result = subprocess.run(
            command, env=environment, capture_output=True, text=True, timeout=30,
        )
        state[action] = {
            "exit_code": result.returncode,
            "stdout": result.stdout,
            "stderr": result.stderr,
        }
        state[f"{action}_verified"] = result.returncode == 0
        if result.returncode != 0:
            raise RuntimeError(f"disposable guest {action} of run-local CA failed")
    finally:
        (output / "guest-trust.json").write_text(
            json.dumps(state, indent=2) + "\n", encoding="utf-8",
        )


def sample_cohort(command: list[str], repo: Path, env: dict, output: Path, interval: int) -> int:
    pid_file = output / "pid.json"
    kernel32, psapi = win32_api()
    process_flags = PROCESS_QUERY_INFORMATION | PROCESS_VM_READ | SYNCHRONIZE
    process_log = (output / "cohort.log").open("w", encoding="utf-8")
    try:
        process = subprocess.Popen(
            command, cwd=repo, env=env, stdout=process_log,
            stderr=subprocess.STDOUT, creationflags=WINDOWS_CREATE_NEW_PROCESS_GROUP,
        )
    except BaseException:
        process_log.close()
        raise
    process_handle = None
    try:
        deadline = time.monotonic() + 120
        while not pid_file.exists():
            if process.poll() is not None:
                raise RuntimeError(f"cohort exited {process.returncode} before publishing its PID")
            if time.monotonic() >= deadline:
                raise RuntimeError("cohort did not publish pid.json within 120 seconds")
            time.sleep(1)
        process_id = json.loads(pid_file.read_text(encoding="utf-8"))["pid"]
        if not isinstance(process_id, int) or process_id <= 0:
            raise RuntimeError(f"invalid cohort PID receipt: {process_id!r}")
        process_handle = kernel32.OpenProcess(process_flags, False, process_id)
        if not process_handle:
            raise win_error(f"OpenProcess({process_id})")
        metadata = {
            "platform": "windows",
            "sampler": str(Path(__file__).resolve()),
            "rss_method": "psapi.GetProcessMemoryInfo(PROCESS_MEMORY_COUNTERS_EX).WorkingSetSize",
            "thread_method": "Toolhelp32 names/times + first-sight bounded minidump projection",
            "process_handle_method": "kernel32.GetProcessHandleCount",
            "windows_handle_count_semantics": "process handle-table entries; not Linux file descriptors",
            "file_descriptors": None,
            "whole_connection_allocator_tls_os_highwater": "unavailable from live soak observer",
            "summary_compatibility": {
                "pid_field": "Windows main thread ID; process_id stores the Windows process ID",
                "main_thread_comm": "t14_soak_live- normalized from earliest observed thread creation",
                "other_comm": "GetThreadDescription truncated to 15 characters",
            },
            "engine_identity": {
                "node_id": EXPECTED_NODE_ID,
                "source_world_json_sha256": ENGINE_WORLD_SHA256,
                "staged_world_json_is_projection": True,
                "private_engine_key_staged": False,
                "public_modulus_source_private_key_sha256": ENGINE_PRIVATE_KEY_SHA256,
            },
        }
        (output / "windows-sampler-metadata.json").write_text(
            json.dumps(metadata, indent=2) + "\n", encoding="utf-8"
        )
        first = time.monotonic()
        samples_path = output / "process-samples.jsonl"
        provenance_cache: dict[tuple[int, int, int], dict] = {}
        capture_index = 0
        with samples_path.open("x", encoding="utf-8") as samples:
            while kernel32.WaitForSingleObject(process_handle, 0) == WAIT_TIMEOUT:
                rss_bytes, handle_count = process_metrics(kernel32, psapi, process_handle)
                threads, main_tid = thread_census(kernel32, process_id)
                sample_ns = time.monotonic_ns()
                def capture(descriptions):
                    nonlocal capture_index
                    capture_index += 1
                    return capture_thread_projection(
                        process_handle, process_id, output, capture_index, descriptions)
                bind_unnamed_provenance(
                    threads, process_id, sample_ns, provenance_cache, capture)
                value = {
                    "elapsed_seconds": round(time.monotonic() - first, 3),
                    "sample_monotonic_ns": sample_ns,
                    "rss_bytes": rss_bytes,
                    "task_count": len(threads),
                    "threads": threads,
                    "pid": main_tid,
                    "process_id": process_id,
                    "pid_semantics": "windows_main_thread_id_for_summary_compatibility",
                    "windows_process_handles": handle_count,
                    "file_descriptors": None,
                    "sampler_platform": "windows",
                }
                write_jsonl_row(samples, value)
                status = kernel32.WaitForSingleObject(process_handle, interval * 1000)
                if status == WAIT_OBJECT_0:
                    break
                if status != WAIT_TIMEOUT:
                    raise win_error("WaitForSingleObject(process)")
        result = process.wait()
        return result
    finally:
        if process_handle:
            kernel32.CloseHandle(process_handle)
        if process.poll() is None:
            try:
                process.send_signal(signal.CTRL_BREAK_EVENT)
                process.wait(timeout=10)
            except (OSError, subprocess.TimeoutExpired):
                subprocess.run(
                    ["taskkill.exe", "/T", "/F", "/PID", str(process.pid)],
                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False,
                )
                process.wait()
        if process_log is not None:
            process_log.close()


def ready_proxy(process: subprocess.Popen, ready: Path) -> dict:
    if process.stdout is None:
        raise RuntimeError("proxy stdout pipe unavailable")
    result_queue: queue.Queue[str] = queue.Queue(maxsize=1)

    def read_line() -> None:
        result_queue.put(process.stdout.readline())

    threading.Thread(target=read_line, daemon=True).start()
    try:
        line = result_queue.get(timeout=30).strip()
    except queue.Empty as error:
        raise RuntimeError("proxy did not publish readiness within 30 seconds") from error
    if "ready" not in line.lower() or not ready.is_file():
        raise RuntimeError(f"proxy failed before readiness (status {process.poll()}): {line}")
    return json.loads(ready.read_text(encoding="utf-8"))


class ProxyProcess:
    def __init__(self, process: subprocess.Popen):
        self._process = process
        self.windows_ctrl_break_sent = False
        self.forced_kill = False

    def __getattr__(self, name):
        return getattr(self._process, name)

    def send_signal(self, signum):
        if signum != signal.SIGTERM:
            return self._process.send_signal(signum)
        self.windows_ctrl_break_sent = True
        return self._process.send_signal(signal.CTRL_BREAK_EVENT)

    def kill(self):
        self.forced_kill = True
        return self._process.kill()


def write_windows_roster(home: Path, config: dict, port: int, original_writer, modulus: str) -> Path:
    path = original_writer(home, config, port)
    roster = json.loads(path.read_text(encoding="utf-8"))
    roster["servers"][0]["login_key"] = {"modulus": modulus, "exponent": "65537"}
    path.write_text(json.dumps(roster, indent=2) + "\n", encoding="utf-8")
    return path

def git_value(repo: Path, *arguments: str) -> str:
    result = subprocess.run(
        ["git", "-C", str(repo), *arguments],
        check=True, capture_output=True, text=True,
    )
    return result.stdout.rstrip("\r\n")


def verify_repo_identity(repo: Path, mode: str, label: str, harness_commit: str) -> None:
    expected_head = EXPECTED_BASELINE_HEAD if mode == "baseline" else harness_commit
    actual_head = git_value(repo, "rev-parse", "HEAD")
    if actual_head != expected_head:
        raise RuntimeError(
            f"{label} HEAD mismatch: expected={expected_head} actual={actual_head}"
        )
    product_base = (
        EXPECTED_BASELINE_HEAD if mode == "baseline" else EXPECTED_CANDIDATE_PRODUCT_BASE
    )
    expected_product_tree = (
        EXPECTED_BASELINE_PRODUCT_TREE if mode == "baseline"
        else EXPECTED_CANDIDATE_PRODUCT_TREE
    )
    product_tree = git_value(repo, "rev-parse", f"{product_base}^{{tree}}")
    if product_tree != expected_product_tree:
        raise RuntimeError(
            f"{label} product base tree mismatch: "
            f"expected={expected_product_tree} actual={product_tree}"
        )
    if mode == "candidate":
        git_value(repo, "merge-base", "--is-ancestor", product_base, expected_head)
        harness_changes = set(
            git_value(repo, "diff", "--name-only", product_base, expected_head).splitlines()
        )
        if not harness_changes or not harness_changes <= EXPECTED_CANDIDATE_HARNESS_ONLY_CHANGES:
            raise RuntimeError(
                f"{label} product files differ from the pinned candidate product base: "
                f"{sorted(harness_changes)}"
            )
    expected_fork = EXPECTED_BASELINE_FORK if mode == "baseline" else EXPECTED_CANDIDATE_FORK
    actual_fork = git_value(repo, "-C", "vendor/fr-client-rust", "rev-parse", "HEAD")
    if actual_fork != expected_fork:
        raise RuntimeError(
            f"{label} fork pin mismatch: expected={expected_fork} actual={actual_fork}"
        )
    for path, expected_blob in EXPECTED_HARNESS_BLOBS.items():
        actual_blob = git_value(repo, "hash-object", path)
        if actual_blob != expected_blob:
            raise RuntimeError(
                f"{label} fixed harness blob mismatch for {path}: "
                f"expected={expected_blob} actual={actual_blob}"
            )
    changed = [
        line[3:] for line in git_value(repo, "status", "--porcelain").splitlines()
        if line
    ]
    expected_changes = sorted(EXPECTED_PATCH_PATHS) if mode == "baseline" else []
    if sorted(changed) != expected_changes:
        raise RuntimeError(
            f"{label} worktree differs from the pinned arm: "
            f"expected={expected_changes} actual={sorted(changed)}"
        )


def verify_candidate_tools(repo: Path) -> None:
    for path, expected_blob in EXPECTED_CANDIDATE_TOOL_BLOBS.items():
        actual_blob = git_value(repo, "hash-object", path)
        if actual_blob != expected_blob:
            raise RuntimeError(
                f"candidate harness source mismatch for {path}: "
                f"expected={expected_blob} actual={actual_blob}"
            )
    for path in ("tools/t14_soak_summary.py", "tools/windows_t14_soak.py",
                 "tools/t14_windows_thread_evidence.py"):
        expected_blob = git_value(repo, "rev-parse", f"HEAD:{path}")
        actual_blob = git_value(repo, "hash-object", path)
        if actual_blob != expected_blob:
            raise RuntimeError(f"uncommitted candidate harness source: {path}")
        if path != "tools/t14_soak_summary.py":
            deployed_blob = git_value(repo, "hash-object", str(Path(__file__).with_name(Path(path).name)))
            if deployed_blob != expected_blob:
                raise RuntimeError(f"deployed Windows harness differs from commit: {path}")


def verify_candidate_baseline_receipt(ledger, ledger_path: Path, output: Path) -> None:
    if not ledger_path.is_file():
        raise RuntimeError("candidate arm requires the baseline run ledger")
    history = ledger._validate_rows(ledger_path.read_bytes(), require_complete=False)
    if (
        history["pair_count"] != 1
        or history["pair_starts"].get(1) != ["baseline"]
        or history["pair_ends"].get(1) != {"baseline"}
        or history["active"] is not None
    ):
        raise RuntimeError("candidate arm requires a completed pair-1 baseline")
    baseline_output = output.parent / "arm-baseline"
    baseline_registration = history["starts"].get((1, "baseline"))
    if baseline_registration is None or baseline_registration["output"] != str(baseline_output.resolve()):
        raise RuntimeError("baseline output does not match its pair-1 ledger registration")
    cohort_path = baseline_output / "cohort-exit.json"
    addendum_path = baseline_output / "windows-runner-addendum.json"
    if not cohort_path.is_file() or not addendum_path.is_file():
        raise RuntimeError("candidate arm requires complete baseline receipts")
    cohort = json.loads(cohort_path.read_text(encoding="utf-8"))
    addendum = json.loads(addendum_path.read_text(encoding="utf-8"))
    if cohort.get("code") != 0 or addendum.get("cohort_exit_code") != 0:
        raise RuntimeError("candidate arm requires a successful baseline cohort")
    if not addendum.get("ledger_finished"):
        raise RuntimeError("candidate arm requires a finished baseline ledger registration")


def run_windows(args) -> int:

    # The Windows embeddable Python distribution uses an isolated ._pth file;
    # add this verified runner directory explicitly before importing its ledger.
    runner_dir = str(Path(__file__).resolve().parent)
    if runner_dir not in sys.path:
        sys.path.insert(0, runner_dir)
    import windows_t14_soak_ledger as ledger

    repo = args.repo.resolve(strict=True)
    harness_repo = args.harness_repo.resolve(strict=True)
    verify_repo_identity(repo, args.mode, "product", args.harness_commit)
    verify_repo_identity(harness_repo, "candidate", "candidate harness", args.harness_commit)
    if args.mode == "candidate" and repo != harness_repo:
        raise RuntimeError("candidate arm must build from the final candidate harness repository")
    verify_candidate_tools(harness_repo)
    patch_path = Path(__file__).with_name("fixed-harness-on-baseline.patch").resolve(strict=True)
    patch_sha256 = executable_sha256(patch_path)
    if patch_sha256 != EXPECTED_PATCH_SHA256:
        raise RuntimeError(
            f"fixed harness patch digest mismatch: expected={EXPECTED_PATCH_SHA256} "
            f"actual={patch_sha256}"
        )
    relog_patch_path = Path(__file__).with_name(
        "harness-relog-c381-to-2be7.patch"
    ).resolve(strict=True)
    relog_patch_sha256 = executable_sha256(relog_patch_path)
    if relog_patch_sha256 != EXPECTED_RELOG_PATCH_SHA256:
        raise RuntimeError(
            "Relog harness patch digest mismatch: "
            f"expected={EXPECTED_RELOG_PATCH_SHA256} actual={relog_patch_sha256}"
        )
    expected_output = f"smoke-{args.mode}" if args.smoke else f"arm-{args.mode}"
    if args.output.name != expected_output:
        raise RuntimeError(f"T1.4 Windows output directory must be named {expected_output}")
    args.output = args.output.resolve()
    if args.ledger is not None:
        args.ledger = args.ledger.resolve()
    if not args.output.parent.is_dir():
        raise FileNotFoundError(f"Windows run root is missing: {args.output.parent}")
    if args.output.exists():
        raise FileExistsError(f"refusing to overwrite Windows run output: {args.output}")
    if not args.smoke:
        if args.ledger.parent != args.output.parent or args.ledger.name != "run-ledger.jsonl":
            raise RuntimeError("matched arms must share runs/run-ledger.jsonl")
        if args.mode == "baseline" and args.ledger.exists():
            raise RuntimeError("baseline arm requires a fresh absent run ledger")
        if args.mode == "candidate":
            verify_candidate_baseline_receipt(ledger, args.ledger, args.output)

    if args.harness_commit != git_value(harness_repo, "rev-parse", "HEAD"):
        raise RuntimeError("harness commit does not match the checked-out candidate")
    runner_path = harness_repo / "tools" / "t14_soak_run.py"
    if not runner_path.is_file():
        raise FileNotFoundError(f"T1.4 soak runner missing: {runner_path}")
    spec = importlib.util.spec_from_file_location("t14_soak_run_windows_adapter", runner_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load T1.4 runner: {runner_path}")
    module = importlib.util.module_from_spec(spec)
    previous_ledger_module = sys.modules.get("t14_soak_ledger")
    sys.modules["t14_soak_ledger"] = ledger
    tools_dir = str(harness_repo / "tools")
    if tools_dir not in sys.path:
        sys.path.insert(0, tools_dir)
    spec.loader.exec_module(module)
    modulus = str(int(ENGINE_PUBLIC_MODULUS_HEX, 16))
    original_deepcopy = module.copy.deepcopy

    def project_profile_paths(source_config):
        config = original_deepcopy(source_config)
        if isinstance(config, dict) and config.get("run", {}).get("slots") == 20:
            config["profile"]["engine_dir"] = str(args.engine_dir)
            config["profile"]["nav_pack"] = str(args.nav_pack)
            config["profile"]["nav_flags"] = str(args.nav_flags)
            config["_sources"]["profile.engine_dir"] = (
                "Windows-safe read-only projection of Engine A: client pack and node.id only; "
                "host source world.json SHA-256 " + ENGINE_WORLD_SHA256 + "; private key not staged"
            )
        return config

    module.copy.deepcopy = project_profile_paths
    original_modulus = module.local_engine_public_modulus
    module.local_engine_public_modulus = lambda _engine_dir: (modulus, ENGINE_PRIVATE_KEY_SHA256)
    original_roster = module.write_roster
    module.write_roster = lambda home, config, port: write_windows_roster(
        home, config, port, original_roster, modulus
    )
    original_popen = subprocess.Popen
    proxy_processes: list[ProxyProcess] = []
    executable_identity = {
        "harness_commit": args.harness_commit,
        "path": None,
        "sha256_before_execution": None,
        "sha256_after_execution": None,
        "hash_stable": None,
    }
    proxy_wrapper = Path(__file__).with_name("windows_t14_soak_proxy.py").resolve(strict=True)
    proxy_test = Path(__file__).with_name("test_windows_t14_soak_proxy.py").resolve(strict=True)
    proxy_archive = Path(__file__).with_name("proxy-wrapper-v20261104.json").resolve(strict=True)
    staging = json.loads(
        (args.output.parent.parent / "inputs" / "staging-receipt.json").read_text(encoding="utf-8-sig")
    )
    proxy_wrapper_hash = executable_sha256(proxy_wrapper)
    proxy_test_hash = executable_sha256(proxy_test)
    proxy_archive_hash = executable_sha256(proxy_archive)
    if (
        proxy_wrapper_hash != staging.get("proxy_wrapper_sha256")
        or proxy_test_hash != staging.get("proxy_wrapper_test_sha256")
        or proxy_archive_hash != staging.get("proxy_wrapper_archive_manifest_sha256")
    ):
        raise RuntimeError("Windows proxy wrapper/test bytes differ from pinned staging receipt")
    expected_staging = {
        "candidate_head": args.harness_commit,
        "harness_commit": args.harness_commit,
        "candidate_product_base_commit": EXPECTED_CANDIDATE_PRODUCT_BASE,
        "candidate_product_tree_oid": EXPECTED_CANDIDATE_PRODUCT_TREE,
        "baseline_product_base_commit": EXPECTED_BASELINE_HEAD,
        "baseline_product_tree_oid": EXPECTED_BASELINE_PRODUCT_TREE,
        "harness_live_blob_oid": EXPECTED_HARNESS_BLOBS[
            "crates/host-play/tests/t14_soak_live.rs"
        ],
        "harness_incremental_patch_sha256": relog_patch_sha256,
    }
    if any(staging.get(key) != value for key, value in expected_staging.items()):
        raise RuntimeError("Windows staging receipt does not bind the product and harness")
    if not re.fullmatch(r"[0-9a-f]{64}", staging.get("candidate_bundle_sha256", "")):
        raise RuntimeError("Windows staging receipt lacks the candidate bundle digest")
    snapshot_root = args.snapshot_root.resolve(strict=True)
    snapshot_manifest = snapshot_root / NAV_BUILD_SNAPSHOT_VERSION / "manifest"
    if executable_sha256(snapshot_manifest) != NAV_BUILD_SNAPSHOT_MANIFEST_SHA256:
        raise RuntimeError("decoded navigation build snapshot manifest differs from pinned revision-289 input")

    def windows_popen(*popenargs, **kwargs):
        command = popenargs[0] if popenargs else kwargs.get("args")
        if not isinstance(command, (list, tuple)) or len(command) < 2 or not str(command[1]).endswith("t14_soak_proxy.py"):
            return original_popen(*popenargs, **kwargs)
        command = list(command)
        original_proxy = command[1]
        command[1:2] = [str(proxy_wrapper), original_proxy]
        if popenargs:
            popenargs = (command, *popenargs[1:])
        else:
            kwargs["args"] = command
        kwargs["creationflags"] = kwargs.get("creationflags", 0) | WINDOWS_CREATE_NEW_PROCESS_GROUP
        wrapper = ProxyProcess(original_popen(*popenargs, **kwargs))
        proxy_processes.append(wrapper)
        return wrapper

    def windows_sample_cohort(command, product_repo, env, output, interval):
        executable = find_soak_test_executable(product_repo, output / "prebuild.log")
        before = executable_sha256(executable)
        executable_identity.update({
            "path": str(executable),
            "sha256_before_execution": before,
        })
        direct_command = direct_soak_test_command(command, executable)
        executable_identity["argv"] = direct_command
        ca = Path(env["SSL_CERT_FILE"]).resolve(strict=True)
        trust_state = {"arm": args.mode, "guest": "disposable Windows agent guest"}
        try:
            run_guest_ca_trust(ca, output, "install", trust_state)
            try:
                result = sample_cohort(direct_command, product_repo, env, output, interval)
            finally:
                after = executable_sha256(executable)
                executable_identity["sha256_after_execution"] = after
                executable_identity["hash_stable"] = before == after
        finally:
            if trust_state.get("install_attempted"):
                run_guest_ca_trust(ca, output, "remove", trust_state)
        if before != after:
            raise RuntimeError("T1.4 soak test executable changed during its run")
        return result

    module.subprocess.Popen = windows_popen
    module.sample_cohort = windows_sample_cohort
    module.ready_proxy = ready_proxy
    os.environ["BOT_NAV_SNAPSHOT_ROOT"] = str(snapshot_root)
    os.environ.setdefault("CARGO_HOME", str(Path.home() / ".cargo"))
    os.environ.setdefault("RUSTUP_HOME", str(Path.home() / ".rustup"))
    module.sys.argv = [
        str(runner_path), "--repo", str(repo), "--mode", args.mode,
        "--output", str(args.output), "--seed", str(args.seed),
    ]
    if args.smoke:
        module.sys.argv.extend(("--smoke", "--smoke-duration-seconds", "180"))
    original_main = module.main
    ledger_state = {"registration": None, "finished": False}

    def ledger_main():
        if args.smoke:
            return original_main()
        if args.mode == "candidate":
            verify_candidate_baseline_receipt(ledger, args.ledger, args.output)
        registration = ledger.start_arm(
            args.ledger, args.pair_index, args.mode, args.output,
        )
        ledger_state["registration"] = registration
        try:
            return original_main()
        finally:
            ledger.finish_arm(args.ledger, registration)
            ledger_state["finished"] = True

    module.main = ledger_main
    result = 2
    failure = None
    try:
        result = module.main()
    except Exception as error:
        failure = f"{type(error).__name__}: {error}"
        traceback.print_exc()
    finally:
        module.copy.deepcopy = original_deepcopy
        module.local_engine_public_modulus = original_modulus
        module.write_roster = original_roster
        module.subprocess.Popen = original_popen
        module.main = original_main
        if previous_ledger_module is None:
            sys.modules.pop("t14_soak_ledger", None)
        else:
            sys.modules["t14_soak_ledger"] = previous_ledger_module
        if args.output.exists():
            sample_path = args.output / "process-samples.jsonl"
            resource_summary = {"sample_count": 0}
            if sample_path.is_file():
                try:
                    samples = [
                        json.loads(line) for line in sample_path.read_text(encoding="utf-8").splitlines()
                        if line.strip()
                    ]
                    handles = [int(row["windows_process_handles"]) for row in samples]
                    rss = [int(row["rss_bytes"]) for row in samples]
                    tasks = [int(row["task_count"]) for row in samples]
                    resource_summary = {
                        "sample_count": len(samples),
                        "windows_process_handles_min": min(handles) if handles else None,
                        "windows_process_handles_max": max(handles) if handles else None,
                        "windows_process_handles_final": handles[-1] if handles else None,
                        "working_set_rss_highwater_bytes": max(rss) if rss else None,
                        "task_count_max": max(tasks) if tasks else None,
                        "file_descriptors": None,
                    }
                except Exception as error:
                    resource_summary = {"sample_read_error": f"{type(error).__name__}: {error}"}
            addendum = {
                "platform": "windows",
                "runner": str(Path(__file__).resolve()),
                "runner_sha256": executable_sha256(Path(__file__)),
                "candidate_or_baseline": args.mode,
                "harness_commit": args.harness_commit,
                "harness_runner": str(runner_path),
                "fixed_harness_patch_sha256": patch_sha256,
                "product_base_commit": (
                    EXPECTED_BASELINE_HEAD if args.mode == "baseline"
                    else EXPECTED_CANDIDATE_PRODUCT_BASE
                ),
                "product_base_tree_oid": (
                    EXPECTED_BASELINE_PRODUCT_TREE if args.mode == "baseline"
                    else EXPECTED_CANDIDATE_PRODUCT_TREE
                ),
                "harness_live_blob_oid": EXPECTED_HARNESS_BLOBS[
                    "crates/host-play/tests/t14_soak_live.rs"
                ],
                "harness_relog_patch_sha256": relog_patch_sha256,
                "nonqualifying_smoke": args.smoke,
                "ledger_path": None if args.smoke else str(args.ledger),
                "ledger_pair_index": None if args.smoke else args.pair_index,
                "ledger_registration": ledger_state["registration"],
                "ledger_finished": ledger_state["finished"],
                "ledger_lock": None if args.smoke else "Win32 LockFileEx exclusive byte-range lock",
                "ledger_host_identity": None if args.smoke else "MachineGuid + SystemBootEnvironmentInformation.BootIdentifier",
                "proxy_wrapper_sha256": proxy_wrapper_hash,
                "proxy_wrapper_test_sha256": proxy_test_hash,
                "proxy_wrapper_archive_manifest_sha256": proxy_archive_hash,
                "proxy_ready_directory_fsync": "unsupported on Windows; file fsync and atomic os.replace retained",
                "staging_initial_snapshot_version": staging.get("nav_snapshot_version"),
                "nav_build_snapshot_root": str(snapshot_root),
                "nav_build_snapshot_version": NAV_BUILD_SNAPSHOT_VERSION,
                "nav_build_snapshot_manifest_sha256": NAV_BUILD_SNAPSHOT_MANIFEST_SHA256,
                "product_test_executable": executable_identity,
                "seed": args.seed,
                "windows_process_handles_are_not_file_descriptors": True,
                "file_descriptors": None,
                "whole_connection_allocator_tls_os_highwater": "unavailable",
                "logical_five_compartment_reservation_bytes": 250760,
                "actual_whole_connection_bytes": None,
                "public_modulus_source_sha256": ENGINE_PRIVATE_KEY_SHA256,
                "private_engine_key_staged": False,
                "main_thread_adapter": "earliest Windows thread creation mapped to Linux summary leader sentinel; raw Windows name retained",
                "resource_sample_summary": resource_summary,
                "proxy_shutdown": [
                    {"ctrl_break_sent": item.windows_ctrl_break_sent, "forced_kill": item.forced_kill,
                     "exit_code": item.poll()} for item in proxy_processes
                ],
                "cohort_exit_code": result,
                "setup_failure": failure,
            }
            (args.output / "windows-runner-addendum.json").write_text(
                json.dumps(addendum, indent=2) + "\n", encoding="utf-8"
            )
    return result if isinstance(result, int) else 2


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--harness-repo", type=Path, required=True)
    parser.add_argument("--harness-commit", required=True)
    parser.add_argument("--mode", choices=("baseline", "candidate"), required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--seed", type=int, required=True)
    parser.add_argument("--engine-dir", type=Path, required=True)
    parser.add_argument("--nav-pack", type=Path, required=True)
    parser.add_argument("--nav-flags", type=Path, required=True)
    parser.add_argument("--snapshot-root", type=Path, required=True)
    parser.add_argument("--ledger", type=Path)
    parser.add_argument("--smoke", action="store_true", help="nonqualifying 2-slot, 180-second dry run without ledger")
    parser.add_argument("--pair-index", type=int, default=1)
    args = parser.parse_args()
    if not re.fullmatch(r"[0-9a-f]{40}", args.harness_commit):
        raise ValueError("harness commit must be a full lowercase Git object ID")
    if args.harness_commit == EXPECTED_CANDIDATE_PRODUCT_BASE:
        raise ValueError("Windows harness must be a committed descendant of the product base")
    if args.seed != 1027:
        raise ValueError("T14-CLOSEOUT-1104 Windows arms require the paired seed 1027")
    if args.pair_index != 1:
        raise ValueError("T14-CLOSEOUT-1104 Windows runner owns only pair 1")
    if args.smoke and args.ledger is not None:
        raise ValueError("nonqualifying smoke must not register a qualification ledger")
    if not args.smoke and args.ledger is None:
        raise ValueError("qualification requires the matched run ledger")
    return run_windows(args)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"Windows T1.4 adapter failed: {error}", file=sys.stderr)
        raise SystemExit(2)
