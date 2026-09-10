"""One native N1 functional smoke using retained Rust harness and existing ConPTY."""
import datetime
import hashlib
import json
import os
import pathlib
import subprocess
import sys
import threading
import time

spec = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8-sig"))
if os.environ.get("USERNAME", "").lower() != "bottest":
    raise SystemExit("Expected the existing BotTest interactive session")
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
public = json.loads((pathlib.Path(spec["engine"]) / "server-login-public.json").read_text(encoding="utf-8-sig"))
env.update(LIVE="1", BOT_TARGET="local", ENGINE_DIR=spec["engine"],
           RS2B0T=spec["rs2b0t"], LOGIN_RSAN=str(public["modulus_decimal"]),
           LOGIN_RSAE=str(public["exponent_decimal"]), NAV_PACK=spec["nav_pack"],
           NAV_FLAGS=spec["nav_flags"], BOT_MEMORY_N="1", BOT_MEMORY_WORKLOAD="active",
           BOT_MEMORY_OUTPUT=str(out / "samples.jsonl"), BOT_MEMORY_WARMUP_S="30",
           BOT_MEMORY_OBSERVE_S="120", BOT_MEMORY_SUSTAIN="1", BOT_MEMORY_DIAGNOSTICS="1")
if spec["frontend"] == "panel":
    env["BOT_MEMORY_RENDER_POLICY"] = "focused-one"
(out / "spec.json").write_text(json.dumps(spec, indent=2))
child = None
drain = None
probe = None
logs = []
result = {"timed_out": False, "exit_code": None, "performance_acceptance": False}
start = time.monotonic()
try:
    if spec["frontend"] == "tui":
        import windows_conpty
        from tui_input_probe import InputProbe
        child = windows_conpty.spawn([str(binary)], cwd=str(out), env=env, cols=120, rows=40)
        terminal = (out / "terminal.ansi").open("wb")
        logs.append(terminal)
        def pump():
            while True:
                data = child.read()
                if not data:
                    break
                terminal.write(data)
                terminal.flush()
        drain = threading.Thread(target=pump, daemon=True)
        drain.start()
        probe = InputProbe(child.input_writer(), out, interval_s=2.0)
        probe.start()
    else:
        stdout = (out / "stdout.log").open("wb")
        stderr = (out / "stderr.log").open("wb")
        logs.extend([stdout, stderr])
        child = subprocess.Popen([str(binary)], cwd=out, env=env, stdout=stdout, stderr=stderr)
    process_info = subprocess.check_output(["powershell", "-NoProfile", "-NonInteractive", "-Command",
        "$ProgressPreference='SilentlyContinue'; $p=Get-Process -Id " + str(child.pid) +
        "; [ordered]@{pid=$p.Id;sessionId=$p.SessionId;startUtc=$p.StartTime.ToUniversalTime().ToString('o');binary=$p.Path}|ConvertTo-Json"], text=True)
    identity = json.loads(process_info)
    identity.update(host_sha=spec["host_sha"], client_sha=spec["client_sha"],
                    binary_sha256=spec["binary_sha256"], frontend=spec["frontend"],
                    terminal="ConPTY 120x40" if spec["frontend"] == "tui" else None,
                    performance_acceptance=False)
    (out / "launch.json").write_text(json.dumps(identity, indent=2))
    captured = False
    while child.poll() is None:
        if time.monotonic() - start > 480:
            result["timed_out"] = True
            child.terminate()
            break
        if spec["frontend"] == "panel" and not captured:
            qual = out / "samples.qualification.jsonl"
            if qual.exists() and b'"observe-start"' in qual.read_bytes():
                captured = True
                argv = ["powershell", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
                        "-File", spec["capture_script"], "-PanelPid", str(child.pid),
                        "-ExpectedStartUtc", identity["startUtc"], "-ExpectedBinary", str(binary),
                        "-ExpectedHash", spec["binary_sha256"], "-OutputDirectory", str(out),
                        "-Label", "observation-start"]
                capture = subprocess.run(argv, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
                (out / "capture.log").write_text(capture.stdout)
                result["capture_exit"] = capture.returncode
        time.sleep(0.5)
    result["exit_code"] = child.wait(timeout=20)
except Exception as error:
    result["launcher_error"] = str(error)
finally:
    if probe:
        probe.close()
        result["input_writes"] = probe.sent
        result["input_probe_error"] = probe.error
    if child:
        if child.poll() is None:
            child.terminate()
            child.wait(timeout=20)
        if spec["frontend"] == "tui":
            child.close_pseudoconsole()
            if drain:
                drain.join(timeout=10)
            child.close()
    for log in logs:
        log.close()
    result["elapsed_s"] = time.monotonic() - start
    result["ended_utc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    (out / "completion.json").write_text(json.dumps(result, indent=2))
print(json.dumps(result))
raise SystemExit(0 if result["exit_code"] == 0 and not result["timed_out"] and not result.get("launcher_error") else 1)
