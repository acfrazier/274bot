"""Fail-closed direct Heaptrack capture and bounded offline analysis helpers."""
from __future__ import annotations

import hashlib
import json
import os
import pathlib
import resource
import shutil
import signal
import subprocess
import sys
import threading
import time
from typing import Any, Callable, Dict, Mapping, Optional, Sequence

PRELOAD_PATH = pathlib.Path("/usr/lib/heaptrack/libheaptrack_preload.so")
INTERPRETER_PATH = pathlib.Path("/usr/lib/heaptrack/libexec/heaptrack_interpret")
PRINTER_PATH = pathlib.Path("/usr/bin/heaptrack_print")
PRELOAD_SHA256 = "134760dbba8d9a2cd1b45f639c119f5c0cd779674a496fb7ed2b6baacac15ca9"
HEAPTRACK_VERSION = "1.5.0"
RAW_NAME = "alloc.raw"
INTERPRETED_NAME = "alloc.interpreted"
PEAK_STACKS_NAME = "peak-stacks.txt"
PEAK_LOG_NAME = "peak-analysis.log"
GI = 1024 ** 3
MAX_RAW_BYTES = 3 * GI
RAW_SOFT_STOP_BYTES = 2 * GI
MAX_FRONTEND_RSS_BYTES = 512 * 1024 ** 2
MAX_ANALYSIS_RSS_BYTES = 512 * 1024 ** 2
MAX_ANALYSIS_AS_BYTES = 768 * GI
MAX_PRINTER_OUTPUT_BYTES = 512 * 1024 ** 2
MIN_ANALYSIS_FREE_BYTES = 8 * GI
MIN_ANALYSIS_ABORT_FREE_BYTES = 1 * GI


class CaptureError(RuntimeError):
    """A capture or analysis invariant was not met."""


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def verify_preload(path: pathlib.Path = PRELOAD_PATH, *, expected_sha256: str = PRELOAD_SHA256,
                   version: str = HEAPTRACK_VERSION) -> Dict[str, Any]:
    """Verify the exact native preload before any frontend is started."""
    path = pathlib.Path(path)
    if sys.platform != "linux":
        raise CaptureError("direct Heaptrack capture requires Linux")
    if path.is_symlink() or not path.is_file():
        raise CaptureError(f"Heaptrack preload is missing or symlinked: {path}")
    actual = sha256(path)
    if actual != expected_sha256:
        raise CaptureError(f"Heaptrack preload SHA-256 mismatch: {path}")
    return {"path": str(path), "version": version, "sha256": actual}


def prepare_output(path: pathlib.Path) -> Dict[str, Any]:
    """Create one owned empty 0700 directory and reserve no reusable raw file."""
    path = pathlib.Path(path).resolve()
    if path.exists() or path.is_symlink():
        raise CaptureError(f"capture output already exists: {path}")
    path.mkdir(mode=0o700, parents=False)
    try:
        if path.stat().st_uid != os.getuid() or (path.stat().st_mode & 0o777) != 0o700:
            raise CaptureError("capture directory is not owned/mode 0700")
        raw = path / RAW_NAME
        if raw.exists() or raw.is_symlink():
            raise CaptureError("capture raw output collision")
        return {"directory": str(path), "raw": str(raw), "directory_mode": "0700", "raw_mode": "0600"}
    except Exception:
        # Do not leave a directory that can accidentally be reused after a failed reservation.
        try:
            path.rmdir()
        except OSError:
            pass
        raise


def child_preexec(*, inherited_limits: bool = True) -> Callable[[], None]:
    """Return a POSIX child-only umask/size-limit setup function."""
    def setup() -> None:
        os.umask(0o077)
        if not hasattr(resource, "RLIMIT_FSIZE"):
            raise OSError("RLIMIT_FSIZE unavailable")
        old_soft, old_hard = resource.getrlimit(resource.RLIMIT_FSIZE)
        cap = MAX_RAW_BYTES
        values = [value for value in (old_soft, old_hard) if value != resource.RLIM_INFINITY]
        if inherited_limits and values:
            cap = min([cap, *values])
        resource.setrlimit(resource.RLIMIT_FSIZE, (cap, cap))
    return setup


def child_env(env: Mapping[str, str], output: Mapping[str, Any], preload: Mapping[str, Any]) -> Dict[str, str]:
    """Copy only the frontend environment and add the direct preload contract."""
    result = dict(env)
    result["LD_PRELOAD"] = str(preload["path"])
    result["DUMP_HEAPTRACK_OUTPUT"] = str(output["raw"])
    return result


def verify_raw(path: pathlib.Path) -> Dict[str, Any]:
    path = pathlib.Path(path)
    if not path.is_file() or path.is_symlink():
        raise CaptureError("Heaptrack raw output is missing or not a regular file")
    mode = path.stat().st_mode & 0o777
    if mode != 0o600 or path.stat().st_uid != os.getuid():
        raise CaptureError("Heaptrack raw output is not owned/mode 0600")
    size = path.stat().st_size
    if size <= 0 or size > MAX_RAW_BYTES:
        raise CaptureError(f"Heaptrack raw output size is invalid: {size}")
    return {"path": str(path), "size": size, "sha256": sha256(path), "mode": "0600"}


def _linux_rss(pid: int) -> Optional[int]:
    try:
        fields = (pathlib.Path("/proc") / str(pid) / "stat").read_text().rsplit(")", 1)[1].split()
        return int(fields[21]) * os.sysconf("SC_PAGE_SIZE")
    except (OSError, ValueError, IndexError):
        return None


def _mem_available() -> Optional[int]:
    try:
        for line in pathlib.Path("/proc/meminfo").read_text().splitlines():
            if line.startswith("MemAvailable:"):
                return int(line.split()[1]) * 1024
    except (OSError, ValueError, IndexError):
        return None
    return None


def start_guard(pid: int, output: pathlib.Path, on_trigger: Callable[[str], None]) -> tuple[threading.Event, threading.Thread, Dict[str, Any]]:
    """Poll frontend total RSS, host availability, and raw size at 0.5s."""
    stop = threading.Event()
    state: Dict[str, Any] = {"reason": None, "peak_rss_bytes": 0, "peak_raw_bytes": 0}
    output = pathlib.Path(output)
    def poll() -> None:
        while not stop.wait(0.5):
            rss = _linux_rss(pid)
            available = _mem_available()
            raw = output / RAW_NAME
            raw_size = raw.stat().st_size if raw.is_file() and not raw.is_symlink() else 0
            state["peak_rss_bytes"] = max(state["peak_rss_bytes"], rss or 0)
            state["peak_raw_bytes"] = max(state["peak_raw_bytes"], raw_size)
            reason = None
            if rss is not None and rss > MAX_FRONTEND_RSS_BYTES:
                reason = f"frontend total RSS {rss} > {MAX_FRONTEND_RSS_BYTES}"
            elif available is None or available < 128 * 1024 ** 2:
                reason = "MemAvailable unavailable" if available is None else f"MemAvailable {available} < 134217728"
            elif raw_size >= RAW_SOFT_STOP_BYTES:
                reason = f"raw output {raw_size} >= {RAW_SOFT_STOP_BYTES}"
            if reason:
                state["reason"] = reason
                on_trigger(reason)
                return
    thread = threading.Thread(target=poll, name="heaptrack-capture-guard", daemon=True)
    thread.start()
    return stop, thread, state


def analysis_argv(output: pathlib.Path) -> Dict[str, list[str]]:
    output = pathlib.Path(output).resolve()
    raw = output / RAW_NAME
    interpreted = output / INTERPRETED_NAME
    stacks = output / PEAK_STACKS_NAME
    log = output / PEAK_LOG_NAME
    return {
        "interpreter": [str(INTERPRETER_PATH)],
        "printer": [str(PRINTER_PATH), "--merge-backtraces=0", "--flamegraph-cost-type=peak",
                     "--print-flamegraph", str(stacks), str(interpreted)],
        "raw": [str(raw)], "interpreted": [str(interpreted)], "stacks": [str(stacks)],
        "log": [str(log)],
    }


def require_free_disk(path: pathlib.Path, minimum: int = MIN_ANALYSIS_FREE_BYTES) -> int:
    free = shutil.disk_usage(path).free
    if free < minimum:
        raise CaptureError(f"insufficient free disk: {free} < {minimum}")
    return free


def _analysis_preexec(*, as_bytes: int, fsize_bytes: int) -> Callable[[], None]:
    def setup() -> None:
        os.umask(0o077)
        soft, hard = resource.getrlimit(resource.RLIMIT_AS)
        cap = min(as_bytes, *(x for x in (soft, hard) if x != resource.RLIM_INFINITY)) if soft != resource.RLIM_INFINITY or hard != resource.RLIM_INFINITY else as_bytes
        resource.setrlimit(resource.RLIMIT_AS, (cap, cap))
        soft, hard = resource.getrlimit(resource.RLIMIT_FSIZE)
        cap = min(fsize_bytes, *(x for x in (soft, hard) if x != resource.RLIM_INFINITY)) if soft != resource.RLIM_INFINITY or hard != resource.RLIM_INFINITY else fsize_bytes
        resource.setrlimit(resource.RLIMIT_FSIZE, (cap, cap))
    return setup


def run_bounded(argv: Sequence[str], *, stdin_path: Optional[pathlib.Path], stdout_path: pathlib.Path,
                timeout_s: float = 180.0, as_bytes: int = MAX_ANALYSIS_AS_BYTES,
                fsize_bytes: int = MAX_RAW_BYTES, rss_limit: int = MAX_ANALYSIS_RSS_BYTES,
                sample: Optional[Callable[[int], Optional[int]]] = None) -> Dict[str, Any]:
    """Run one exact argv with timeout/RSS/output bounds; never invoke a shell."""
    stdout_path = pathlib.Path(stdout_path)
    if stdout_path.exists() or stdout_path.is_symlink():
        raise CaptureError(f"analysis output already exists: {stdout_path}")
    stdin = None
    stdout = stdout_path.open("xb")
    proc = None
    started = time.monotonic()
    try:
        if stdin_path is not None:
            stdin = pathlib.Path(stdin_path).open("rb")
        kwargs: Dict[str, Any] = {"stdin": stdin, "stdout": stdout, "stderr": subprocess.STDOUT}
        if os.name != "nt":
            kwargs["preexec_fn"] = _analysis_preexec(as_bytes=as_bytes, fsize_bytes=fsize_bytes)
        proc = subprocess.Popen(list(argv), **kwargs)
        peak = 0
        while proc.poll() is None:
            if sample is not None:
                value = sample(proc.pid)
                if value is not None:
                    peak = max(peak, value)
                    if value > rss_limit:
                        proc.kill()
                        raise CaptureError("analysis RSS limit exceeded")
            if time.monotonic() - started > timeout_s:
                proc.kill()
                raise CaptureError("analysis timeout")
            if stdout_path.stat().st_size > fsize_bytes:
                proc.kill()
                raise CaptureError("analysis output limit exceeded")
            if shutil.disk_usage(stdout_path.parent).free < MIN_ANALYSIS_ABORT_FREE_BYTES:
                proc.kill()
                raise CaptureError("analysis free disk fell below 1 GiB")
            time.sleep(0.5)
        rc = proc.wait()
        if rc != 0:
            raise CaptureError(f"analysis command exited {rc}: {list(argv)!r}")
        return {"argv": list(argv), "pid": proc.pid, "exit_code": rc,
                "output": str(stdout_path), "output_size": stdout_path.stat().st_size,
                "elapsed_s": time.monotonic() - started, "peak_rss_bytes": peak}
    finally:
        if stdin is not None:
            stdin.close()
        stdout.close()


def analyze(output: pathlib.Path, *, sample: Optional[Callable[[int], Optional[int]]] = None) -> Dict[str, Any]:
    """Interpret and print one completed raw artifact, sequentially."""
    output = pathlib.Path(output).resolve()
    require_free_disk(output)
    raw = verify_raw(output / RAW_NAME)
    argv = analysis_argv(output)
    interpreted = run_bounded(argv["interpreter"], stdin_path=output / RAW_NAME,
                              stdout_path=output / INTERPRETED_NAME, sample=sample,
                              fsize_bytes=MAX_RAW_BYTES)
    require_free_disk(output)
    printed = run_bounded(argv["printer"], stdin_path=None,
                          stdout_path=output / PEAK_LOG_NAME, sample=sample,
                          fsize_bytes=MAX_PRINTER_OUTPUT_BYTES)
    return {"raw": raw, "interpreter": interpreted, "printer": printed,
            "argv": {"interpreter": argv["interpreter"], "printer": argv["printer"]}}
