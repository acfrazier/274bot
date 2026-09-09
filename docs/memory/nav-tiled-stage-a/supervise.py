#!/usr/bin/env python3
"""Bounded synthetic Stage A runner; real input is deliberately unsupported."""
from __future__ import annotations
import hashlib, json, os, signal, subprocess, sys, time
import resource, selectors as io_selectors
from pathlib import Path

LIMITS = {"wall": 30.0, "cpu": 20, "rss": 1024**3, "output": 1024**2}
SELECTOR_FIELDS = 10

def sha(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""): h.update(chunk)
    return h.hexdigest()

def admit(path: Path, expected_sha256: str, max_bytes: int) -> None:
    st = path.lstat()
    if not path.is_file() or not stat_is_regular(st.st_mode) or path.is_symlink() or st.st_size > max_bytes:
        raise ValueError("input must be a bounded regular non-symlink file")
    if len(expected_sha256) != 64 or sha(path) != expected_sha256:
        raise ValueError("input hash mismatch")

def stat_is_regular(mode: int) -> bool:
    return (mode & 0o170000) == 0o100000

def selectors(path: Path) -> list[list[int]]:
    result = []
    for no, line in enumerate(path.read_text().splitlines(), 1):
        if not line.strip(): continue
        try: values = [int(x) for x in line.split()]
        except ValueError as exc: raise ValueError(f"malformed selector line {no}") from exc
        if len(values) != SELECTOR_FIELDS or not (0 <= values[2] <= 3 and 0 <= values[5] <= 3 and 0 <= values[6] <= 15 and 0 <= values[7] <= 6 and 0 <= values[8] <= 4 and 0 <= values[9] <= 1):
            raise ValueError(f"unsupported selector line {no}")
        result.append(values)
    if not result: raise ValueError("selector file is empty")
    return result

def current_rss(pid: int) -> int | None:
    try:
        out = subprocess.check_output(["ps", "-o", "rss=", "-p", str(pid)], text=True, timeout=1)
        return int(out.strip()) * 1024
    except (ValueError, OSError, subprocess.SubprocessError): return None

def bounded(cmd: list[str], outdir: Path, name: str, *, limits=LIMITS) -> dict:
    outdir.mkdir(parents=True, exist_ok=True)
    paths = [outdir / f"{name}.{s}" for s in ("out", "err", "receipt.json")]
    if any(p.exists() for p in paths): raise FileExistsError(name)
    start = time.monotonic(); peak = 0; output = 0; failure = None
    out, err = (p.open("xb") for p in paths[:2])
    def set_limits() -> None:
        resource.setrlimit(resource.RLIMIT_CPU, (limits["cpu"], limits["cpu"]))
        resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True, text=False, preexec_fn=set_limits)
    poller = io_selectors.DefaultSelector()
    assert proc.stdout is not None and proc.stderr is not None
    for stream, sink in ((proc.stdout, out), (proc.stderr, err)):
        os.set_blocking(stream.fileno(), False)
        poller.register(stream, io_selectors.EVENT_READ, sink)
    try:
        while poller.get_map() or proc.poll() is None:
            if time.monotonic() - start > limits["wall"]: failure = "wall"; break
            rss = current_rss(proc.pid)
            if rss is not None: peak = max(peak, rss)
            if peak > limits["rss"]: failure = "rss"; break
            for key, _ in poller.select(.01):
                data = os.read(key.fd, 65536)
                if data:
                    room = limits["output"] - output
                    key.data.write(data[:room]); output += min(len(data), room)
                    if len(data) > room: failure = "output"; break
                else: poller.unregister(key.fileobj)
            if failure: break
            time.sleep(.01)
    finally:
        if proc.poll() is None:
            os.killpg(proc.pid, signal.SIGKILL)
        rc = proc.wait(timeout=5)
        poller.close()
        proc.stdout.close()
        proc.stderr.close()
        out.close(); err.close()
    if failure is None and rc: failure = "exit"
    receipt = {"command": cmd, "returncode": rc, "failure": failure, "wall_seconds": time.monotonic()-start, "sampled_current_rss_peak_bytes": peak, "output_bytes": output, "limits": limits, "address_guard_active": sys.platform.startswith("linux")}
    paths[2].write_text(json.dumps(receipt, indent=2) + "\n")
    return receipt

def paired_delta(baseline: list[float], candidate: list[float]) -> dict:
    if len(baseline) != len(candidate) or not baseline: raise ValueError("paired samples must have equal nonzero length")
    deltas = [b - a for a, b in zip(baseline, candidate)]
    ordered = sorted(deltas)
    median = ordered[len(ordered)//2]
    noise = max(ordered) - min(ordered)
    return {"n": len(deltas), "median_delta": median, "repeat_noise_range": noise, "classification": "inconclusive_noise_overlap" if noise >= abs(median) else "directional_only"}

if __name__ == "__main__":
    if len(sys.argv) != 3 or sys.argv[1] not in ("all-uniform", "all-dense"):
        raise SystemExit("usage: supervise.py all-uniform|all-dense ROUTES.tsv")
    print(json.dumps({"mode": sys.argv[1], "selectors": len(selectors(Path(sys.argv[2]))), "status": "validated; invoke admitted binary separately"}))
