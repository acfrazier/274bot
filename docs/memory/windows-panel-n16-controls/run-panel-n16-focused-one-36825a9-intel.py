"""Prepared controller for the sequential native Windows panel N16 focused-one cell.

This file is transferred to the BotTest host and run only by the reviewed
PowerShell launcher. It intentionally performs no Windows/network action here.
"""
import ctypes
import hashlib
import json
import os
import pathlib
import platform
import sys
import time

assert os.environ["USERNAME"] == "BotTest"
assert ctypes.windll.user32.GetSystemMetrics(0x1000) == 0, "requires local console"
home = pathlib.Path(os.environ["USERPROFILE"])
os.environ["HOME"] = str(home)
root = home / "274bot-workspaces/e3188e2/host"
mem = root / "docs/memory"
sys.path.insert(0, str(mem))
os.chdir(root)
import windows_process_sample as wps
import run_managed_cell as rmc

LABEL = os.environ.get("N16_DIAGNOSTIC_LABEL", "n16-focused-one-console-intel")
MODE = os.environ.get("N16_DIAGNOSTIC_MODE", "focused-one")
ADAPTER = os.environ.get("N16_DIAGNOSTIC_ADAPTER", "Intel(R) Graphics")
assert MODE in {"focused-one", "focused-plus-background"}
assert ADAPTER == "Intel(R) Graphics"
stage = pathlib.Path(r"C:\ProgramData\274bot-Test\panel-36825a9")
binary = stage / "panel-play.exe"
out = home / ("274bot-runs/managed-panel-36825a9-" + LABEL)
out.mkdir(exist_ok=False)

def sha(path):
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()

def dump(path, value):
    pathlib.Path(path).write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")

assert sha(binary) == "87c24665563ef8a55751244a52f7d25c7edf68e3117e84f2ff464add596c71fb"
server = home / "274bot-server-4c95f87"
launch = json.loads((server / "server-launch.json").read_text(encoding="utf-8-sig"))
pid = int(launch["pid"])
sample = wps.sample_process(pid)
public = json.loads((server / "server-login-public.json").read_text())
os.environ.update({
    "LIVE": "1", "BOT_TARGET": "local", "ENGINE_DIR": str(server),
    "LOGIN_RSAN": public["modulus_decimal"], "LOGIN_RSAE": public["exponent_decimal"],
    "RS2B0T": str(home / "274bot-workspaces/b4b686f/rs2b0t"),
    "BOT_PANEL_RENDER_ATTRIBUTION": "1",
    "BOT_PANEL_DIAGNOSTIC_VULKAN_ADAPTER": ADAPTER,
})
operator = home / ".274bot"
nav = operator / "274bot.navpack"
flags = operator / "274bot.navflags"
catalog = operator / "js-scripts.json"
assert json.loads(catalog.read_text()) == [{"name": "trade_bot", "path": str(home / "274bot-workspaces/b4b686f/host/crates/script/tests/fixtures/trade_bot.ts")}]
os.environ.update({"NAV_PACK": str(nav), "NAV_FLAGS": str(flags)})
for key in ["BOT_CPU", "BOT_DEBUG", "BOT_MEMORY_RENDER_POLICY", "BOT_MEMORY_SINGLE_RENDERER", "BOT_MEMORY_N", "BOT_MEMORY_WORKLOAD", "BOT_MEMORY_OUTPUT"]:
    os.environ.pop(key, None)
manifest = out / "build-manifest.json"
server_id = out / "server-identity.json"
conditions = out / "host-conditions.json"
preflight = stage / "preflight-panel-n16-intel.json"

# These values are copied from the already frozen binary/source evidence.
dump(manifest, {"candidate": {"commit": "36825a9f0a07a19610439c691f6dfe3d2e1cbf48", "branch": "codex/windows-submit-attribution", "build_exit": 0, "sources_sha256_pre": "cf0175c549c573a37ebd8af31242b2eb817946a716f76b2785d3d4d8a76f3bca", "sources_sha256_post": "cf0175c549c573a37ebd8af31242b2eb817946a716f76b2785d3d4d8a76f3bca", "sources_stable_across_build": True, "client": {"commit": "5ee9b6efb2342452ceeb7958f864fd68d0daadd1", "sources_sha256": "f71f99afabf29025a91179554792c35343093643a1f386cf1676d9bf2dedca3d"}}, "features": {"requested": "memory-profile-no-alloc", "locked": True, "allocation_counting": False, "allocator": "std::alloc::System via BenchmarkAllocator no-alloc alias"}, "binaries": {"candidate_panel_play": {"path": str(binary), "sha256": sha(binary)}}, "nav": {"nav_pack": str(nav), "nav_pack_sha256": sha(nav), "nav_flags": str(flags), "nav_flags_sha256": sha(flags)}, "catalog": {"js_scripts_json": str(catalog), "js_scripts_json_sha256": sha(catalog)}, "performance_acceptance": False})
dump(server_id, {"pid": pid, "start_identity": sample["start_identity"], "port_listen": 43594, "configuration": {"world_json_sha256": sha(server / "data/config/world.json"), "maps_addition_sha256": sha(server / "native-server-maps-addition.json"), "wordenc_addition_sha256": sha(server / "native-server-wordenc-addition.json"), "bind_host": "127.0.0.1", "node_version": "24.19.0"}, "server_commit": "4c95f87efe00b068cadbd229d94736626907bd1a", "node": "24.19.0", "config_sha256": sha(server / "data/config/world.json"), "maps_addition_sha256": sha(server / "native-server-maps-addition.json"), "wordenc_addition_sha256": sha(server / "native-server-wordenc-addition.json"), "public_key_sha256": sha(server / "data/config/public.pem"), "launch": launch, "sample": sample})
if not preflight.is_file():
    raise RuntimeError("run preflight-panel-n16-36825a9-intel.ps1 first")
preflight_record = json.loads(preflight.read_text(encoding="utf-8-sig"))
dump(conditions, {"platform": platform.platform(), "user": os.environ["USERNAME"], "purpose": "Native local-console explicit Intel adapter N16 diagnostic", "performance_acceptance": False, "builds_stopped_before_run": True, "terminal_transport_expected": False, "panel_render_attribution": True, "native_preflight": preflight_record, "process_lasso": preflight_record.get("processLasso"), "dxdiag": preflight_record.get("dxdiag")})
mode_flag = "--focused-one" if MODE == "focused-one" else "--focused-background"
diag = ["panel", "16", "active", "--binary", str(binary), "--build-manifest", str(manifest), "--build-role", "candidate", "--sustain", "--warmup", "30", "--observe", "120", mode_flag, "--render-profile", "--gpu-completion-profile", "--scheduling-profile", "--responsiveness-profile", "--responsiveness-fine", "--failure-capture"]
if MODE == "focused-one":
    diag.insert(12, "--nav-captures")
spec = {"id": "native-panel-36825a9-console-intel-" + MODE, "index": 1 if MODE == "focused-one" else 2, "kind": "diagnostic", "mode": MODE, "count": 16, "requested_adapter": ADAPTER, "configured_background_fps": 1 if MODE == "focused-plus-background" else None, "binary": str(binary), "build_manifest": str(manifest), "build_role": "candidate", "frontend": "panel", "server_identity_path": str(server_id), "host_conditions_path": str(conditions), "nav_pack": str(nav), "nav_flags": str(flags), "catalog_path": str(catalog), "launcher_argv": [sys.executable, str(mem / "run_diagnostic.py"), *diag], "diagnostic_argv": diag, "game_server_pid": pid, "ambient_helpers": {"bootstrap": os.getppid()}, "sampler_interval_s": 0.5, "max_wall_s": 900, "observe_s": 120, "warmup_s": 30, "teardown_grace_s": 60, "process_backend": "system", "cache_dir": str(server / "data/pack/client"), "unpack_root": str(operator / "unpack")}
sp = out / "spec.json"
dump(sp, spec)
dump(out / "started.json", {"pid": os.getpid(), "start_identity": wps.sample_process(os.getpid())["start_identity"], "started_unix": time.time(), "user": os.environ["USERNAME"], "mode": MODE, "count": 16, "requested_adapter": ADAPTER})
ES_CONTINUOUS = 0x80000000
ES_SYSTEM_REQUIRED = 0x00000001
ES_DISPLAY_REQUIRED = 0x00000002
keep_awake = ctypes.windll.kernel32.SetThreadExecutionState
keep_awake.argtypes = [ctypes.c_uint]
keep_awake.restype = ctypes.c_uint
if keep_awake(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED) == 0:
    raise OSError("SetThreadExecutionState failed")
try:
    rc = rmc.main([str(sp), str(out / "cells")])
finally:
    keep_awake(ES_CONTINUOUS)
dump(out / "completion.json", {"exit_code": rc, "ended_unix": time.time(), "mode": MODE, "count": 16, "requested_adapter": ADAPTER, "performance_acceptance": False})
sys.exit(rc)
