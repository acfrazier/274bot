#!/usr/bin/env python3
"""Bounded owned-process runner for the offline nav census.

The normal/native path is Linux-only because RSS/address supervision is based on
/proc and child resource limits. ``--fixture`` is a portable functional path;
it never claims Linux guard coverage or cumulative peak RSS.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import resource
import selectors
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time

INPUT_CAP = 128 * 1024 * 1024
ADMISSION_MEMORY = 768 * 1024 * 1024
ADMISSION_DISK = 1 * 1024 * 1024 * 1024
CPU_LIMIT = 30.0
WALL_LIMIT = 60.0
RSS_LIMIT = 384 * 1024 * 1024
AS_LIMIT = 512 * 1024 * 1024
OUTPUT_LIMIT = 1 * 1024 * 1024


def fail(message: str) -> None:
    raise ValueError(message)


def regular(path: Path) -> None:
    st = path.lstat()
    if not stat.S_ISREG(st.st_mode) or path.is_symlink():
        fail(f"not a regular non-symlink file: {path}")


def digest(path: Path, cap: int | None = None) -> tuple[int, str]:
    regular(path)
    h = hashlib.sha256()
    total = 0
    with path.open("rb") as stream:
        while True:
            block = stream.read(1024 * 1024)
            if not block:
                break
            total += len(block)
            if cap is not None and total > cap:
                fail("input cap breached")
            h.update(block)
    return total, h.hexdigest()


def available_memory() -> int:
    if sys.platform == "linux":
        for line in Path("/proc/meminfo").read_text().splitlines():
            if line.startswith("MemAvailable:"):
                return int(line.split()[1]) * 1024
        fail("MemAvailable unavailable")
    if sys.platform == "darwin":
        out = subprocess.check_output(["sysctl", "-n", "hw.memsize"], text=True)
        return int(out.strip())
    fail("unsupported platform for memory guard")


def proc_sample(pid: int) -> dict[str, int]:
    if sys.platform != "linux":
        fail("Linux /proc supervision required for REAL run")
    status = Path(f"/proc/{pid}/status").read_text()
    rss = address = threads = None
    for line in status.splitlines():
        if line.startswith("VmRSS:"):
            rss = int(line.split()[1]) * 1024
        elif line.startswith("VmSize:"):
            address = int(line.split()[1]) * 1024
        elif line.startswith("Threads:"):
            threads = int(line.split()[1])
    if rss is None or address is None or threads is None:
        fail("incomplete /proc memory sample")
    stat = Path(f"/proc/{pid}/stat").read_text()
    fields = stat[stat.rfind(")") + 2 :].split()
    ticks = os.sysconf("SC_CLK_TCK")
    return {"rss": rss, "address": address, "threads": threads,
            "cpu_ticks": int(fields[11]) + int(fields[12]), "hz": ticks}


def limits_child() -> None:
    resource.setrlimit(resource.RLIMIT_CPU, (int(CPU_LIMIT) + 1, int(CPU_LIMIT) + 1))
    resource.setrlimit(resource.RLIMIT_FSIZE, (OUTPUT_LIMIT, OUTPUT_LIMIT))
    if sys.platform == "linux":
        resource.setrlimit(resource.RLIMIT_AS, (AS_LIMIT, AS_LIMIT))


def run(args: argparse.Namespace) -> int:
    # Keep the path lexical: resolve() would follow a symlink before admission.
    executable = Path(args.executable).absolute()
    input_path = Path(args.input).absolute()
    regular(executable)
    size, input_hash = digest(input_path, INPUT_CAP)
    if args.input_sha256 and args.input_sha256 != input_hash:
        fail("input hash mismatch")
    executable_size, executable_hash = digest(executable)
    source = Path(args.source).absolute() if args.source else None
    source_size: int | None = None
    source_hash: str | None = None
    if source is not None:
        source_size, source_hash = digest(source, 8 * 1024 * 1024)
    if not args.fixture and (not args.input_sha256 or not args.executable_sha256 or
                            not args.source_sha256):
        fail("REAL run requires expected input/executable/source identities")
    if args.executable_sha256 and args.executable_sha256 != executable_hash:
        fail("executable hash mismatch")
    if args.source_sha256 and args.source_sha256 != source_hash:
        fail("source hash mismatch")
    if not args.fixture and sys.platform != "linux":
        fail("unsupported guard platform for REAL run")
    if available_memory() < ADMISSION_MEMORY:
        fail("admission memory guard")
    root = Path(args.output_root).resolve()
    root.mkdir(parents=True, exist_ok=True)
    if shutil.disk_usage(root).free < ADMISSION_DISK:
        fail("admission disk guard")
    output = Path(tempfile.mkdtemp(prefix="nav-census-", dir=root))
    started = time.monotonic()
    receipt = {
        "status": "failed", "fixture": bool(args.fixture),
        "platform": platform.platform(), "limits": {
            "cpu_seconds": CPU_LIMIT, "wall_seconds": WALL_LIMIT,
            "rss_bytes": RSS_LIMIT, "address_space_bytes": AS_LIMIT,
            "output_bytes": OUTPUT_LIMIT, "input_bytes": INPUT_CAP,
        },
        "identity": {"input": {"path": str(input_path), "bytes": size, "sha256": input_hash},
                     "executable": {"path": str(executable), "bytes": executable_size, "sha256": executable_hash},
                     "source": {"path": str(source) if source else None,
                                "bytes": source_size, "sha256": source_hash}},
        "output_dir": str(output), "linux_proc_guard": sys.platform == "linux",
    }
    child = None
    try:
        command = [str(executable), str(input_path)]
        env = dict(os.environ, PYTHONDONTWRITEBYTECODE="1", OMP_NUM_THREADS="1")
        preexec = limits_child if os.name == "posix" else None
        child = subprocess.Popen(command, cwd=str(output), env=env, stdin=subprocess.DEVNULL,
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                 start_new_session=(os.name == "posix"), preexec_fn=preexec)
        assert child.stdout is not None and child.stderr is not None
        selector = selectors.DefaultSelector()
        selector.register(child.stdout, selectors.EVENT_READ, "stdout")
        selector.register(child.stderr, selectors.EVENT_READ, "stderr")
        stdout = bytearray(); stderr = bytearray(); peak_rss = 0; peak_as = 0
        while selector.get_map() or child.poll() is None:
            if time.monotonic() - started > WALL_LIMIT:
                fail("wall guard")
            for key, _ in selector.select(0.05):
                data = os.read(key.fd, 65536)
                if not data:
                    selector.unregister(key.fileobj)
                elif key.data == "stdout":
                    stdout.extend(data)
                    if len(stdout) > OUTPUT_LIMIT: fail("output cap breached")
                else:
                    stderr.extend(data)
                    if len(stderr) > OUTPUT_LIMIT: fail("stderr cap breached")
            if sys.platform == "linux" and child.poll() is None:
                sample = proc_sample(child.pid)
                peak_rss = max(peak_rss, sample["rss"])
                peak_as = max(peak_as, sample["address"])
                cpu = sample["cpu_ticks"] / sample["hz"]
                if sample["rss"] > RSS_LIMIT: fail("RSS guard")
                if sample["address"] > AS_LIMIT: fail("address-space guard")
                if cpu > CPU_LIMIT: fail("CPU guard")
        returncode = child.wait()
        receipt.update({"returncode": returncode, "stdout_bytes": len(stdout),
                        "stderr_bytes": len(stderr), "sampled_peak_rss_bytes": peak_rss,
                        "sampled_peak_address_bytes": peak_as, "sampled_rss_is_not_cumulative_peak": True})
        if returncode != 0: fail(f"child returned {returncode}")
        try:
            result = json.loads(stdout)
        except (UnicodeDecodeError, json.JSONDecodeError):
            fail("child output is not JSON")
        if not isinstance(result, dict) or result.get("status") != "ok":
            fail("malformed child result")
        if result.get("input", {}).get("sha256") != input_hash:
            fail("child input identity mismatch")
        receipt["status"] = "ok"
        receipt["result"] = result
    except BaseException as exc:
        receipt["failure"] = str(exc)
        if child is not None:
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except (ProcessLookupError, OSError):
                if child.poll() is None:
                    child.kill()
            receipt["returncode"] = child.wait()
        raise
    finally:
        receipt["elapsed_seconds"] = time.monotonic() - started
        (output / "receipt.json").write_text(json.dumps(receipt, sort_keys=True, indent=2) + "\n")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--executable", required=True)
    parser.add_argument("--input", required=True)
    parser.add_argument("--output-root", required=True)
    parser.add_argument("--source")
    parser.add_argument("--input-sha256")
    parser.add_argument("--executable-sha256")
    parser.add_argument("--source-sha256")
    parser.add_argument("--fixture", action="store_true")
    args = parser.parse_args()
    try:
        return run(args)
    except Exception as exc:
        # Pre-launch failures (bad identity, malformed input, admission) happen
        # before the child-specific receipt exists; retain a durable rejection.
        try:
            root = Path(args.output_root).resolve()
            root.mkdir(parents=True, exist_ok=True)
            failure = Path(tempfile.mkdtemp(prefix="nav-census-failed-", dir=root)) / "receipt.json"
            failure.write_text(json.dumps({"status": "failed", "failure": str(exc),
                                           "argv": sys.argv[1:], "platform": platform.platform()},
                                          sort_keys=True, indent=2) + "\n")
        except Exception:
            pass
        print(f"nav supervisor rejected: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
