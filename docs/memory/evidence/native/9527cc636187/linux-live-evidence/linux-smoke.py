"""Bounded real-PTY N1 smoke of an exact selected native Linux executable."""
import datetime
import errno
import fcntl
import hashlib
import json
import os
import pathlib
import pty
import signal
import struct
import subprocess
import sys
import termios
import threading
import time
from tui_input_probe import InputProbe

spec = json.loads(pathlib.Path(sys.argv[1]).read_text())
out = pathlib.Path(spec["output"])
out.mkdir(parents=True, exist_ok=False)
binary = pathlib.Path(spec["binary"])
assert hashlib.sha256(binary.read_bytes()).hexdigest() == spec["binary_sha256"]
env = os.environ.copy()
for key in list(env):
    if key.startswith(("BOT_MEMORY_", "BOT_RESPONSIVENESS_")) or key in (
        "BOT_CPU", "BOT_DEBUG", "BOT_LIVE", "BOT_SCHEDULING_PROFILE", "BOT_RENDER_PROFILE",
        "BOT_GPU_COMPLETION_PROFILE", "BOT_NAV_CAPTURES"):
        del env[key]
public = json.loads((pathlib.Path(spec["engine"]) / "server-login-public.json").read_text())
env.update(LIVE="1", BOT_TARGET="local", ENGINE_DIR=spec["engine"], RS2B0T=spec["rs2b0t"],
           LOGIN_RSAN=str(public["modulus_decimal"]), LOGIN_RSAE=str(public["exponent_decimal"]),
           NAV_PACK=spec["nav_pack"], NAV_FLAGS=spec["nav_flags"], TERM="xterm-256color",
           BOT_MEMORY_N="1", BOT_MEMORY_WORKLOAD="active", BOT_MEMORY_SUSTAIN="1",
           BOT_MEMORY_OUTPUT=str(out / "samples.jsonl"), BOT_MEMORY_WARMUP_S="30",
           BOT_MEMORY_OBSERVE_S="120", BOT_MEMORY_DIAGNOSTICS="1")
(out / "spec.json").write_text(json.dumps(spec, indent=2))
master, slave = pty.openpty()
fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
start = time.monotonic()
child = subprocess.Popen([str(binary)], cwd=out, env=env, stdin=slave, stdout=slave,
                         stderr=slave, start_new_session=True)
os.close(slave)
result = {"pid": child.pid, "exit_code": None, "timed_out": False, "performance_acceptance": False}
(out / "launch.json").write_text(json.dumps(dict(result, binary_sha256=spec["binary_sha256"],
    host_sha=spec["host_sha"], client_sha=spec["client_sha"], terminal="PTY 120x40",
    started_utc=datetime.datetime.now(datetime.timezone.utc).isoformat()), indent=2))
terminal = (out / "terminal.ansi").open("wb")
def pump():
    while True:
        try:
            data = os.read(master, 65536)
        except OSError as error:
            if error.errno in (errno.EIO, errno.EBADF):
                break
            raise
        if not data:
            break
        terminal.write(data)
        terminal.flush()
drain = threading.Thread(target=pump, daemon=True)
drain.start()
probe = InputProbe(master, out, interval_s=2.0)
probe.start()
try:
    result["exit_code"] = child.wait(timeout=480)
except subprocess.TimeoutExpired:
    result["timed_out"] = True
    os.killpg(child.pid, signal.SIGTERM)
    try:
        result["exit_code"] = child.wait(timeout=15)
    except subprocess.TimeoutExpired:
        os.killpg(child.pid, signal.SIGKILL)
        result["exit_code"] = child.wait()
finally:
    probe.close()
    result["input_writes"] = probe.sent
    result["input_probe_error"] = probe.error
    drain.join(timeout=10)
    os.close(master)
    terminal.close()
    result["elapsed_s"] = time.monotonic() - start
    (out / "completion.json").write_text(json.dumps(result, indent=2))
print(json.dumps(result))
raise SystemExit(0 if result["exit_code"] == 0 and not result["timed_out"] else 1)
