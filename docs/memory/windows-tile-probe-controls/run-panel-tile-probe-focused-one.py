"""Single frozen native N16 focused-plus-background tile occupancy controller."""
import ctypes
import hashlib
import json
import os
import pathlib
import platform
import sys
import time

assert os.environ.get("USERNAME") == "BotTest"
assert ctypes.windll.user32.GetSystemMetrics(0x1000) == 0, "requires local console"
cell_id = os.environ.get("RENDER_OWNER_CENSUS_CELL_ID", "")
assert cell_id.startswith("native-render-owner-census-focused-plus-background-")
assert cell_id and all(c.isalnum() or c in "_.-" for c in cell_id)

home = pathlib.Path(os.environ["USERPROFILE"])
root = pathlib.Path(os.environ.get("RENDER_OWNER_CENSUS_HOST_ROOT", home / "274bot-workspaces/3118e96/host"))
mem = root / "docs/memory"
sys.path.insert(0, str(mem))
os.chdir(root)
import run_managed_cell as managed
import run_diagnostic as diagnostic
import windows_process_sample as wps

STAGE = pathlib.Path(r"C:\ProgramData\274bot-Test\renderer-owner-census-9268890")
HOST_COMMIT = "9268890217d968cfeb7c66ebb11dd5c3dd2c084f"
CLIENT_COMMIT = "abb811bd0afa1acd99319ccd5bc36bfb241080f9"
HOST_SOURCES = "84054d9c02394d959cd84681dd85f3589d3a559b94bb37856c6f87cb0629269a"
CLIENT_SOURCES = "ea6402702a56dc29359bf4effd9ca87407734fc9d940bfabdefb92cec8bf41ee"
BINARY_SHA = "e2deb1db371366f674c18e39d04f7309480310072ade224ffa1433c84d4559b5"
binary = STAGE / "panel-play.exe"
out = home / ("274bot-runs/" + cell_id)
out.mkdir(exist_ok=False)

def sha(path):
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()

def dump(path, value):
    pathlib.Path(path).write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")

def configure_environment(public, server):
    os.environ.update({
        "LIVE": "1", "BOT_TARGET": "local", "ENGINE_DIR": str(server),
        "LOGIN_RSAN": public["modulus_decimal"], "LOGIN_RSAE": public["exponent_decimal"],
        "RS2B0T": str(home / "274bot-workspaces/b4b686f/rs2b0t"),
        "BOT_PANEL_RENDER_ATTRIBUTION": "1",
        "BOT_PANEL_DIAGNOSTIC_VULKAN_ADAPTER": "Intel(R) Graphics",
        "NAV_PACK": str(home / ".274bot/274bot.navpack"),
        "NAV_FLAGS": str(home / ".274bot/274bot.navflags"),
        "BOT_RENDER_PROFILE": "1", "BOT_RENDER_OWNER_CENSUS": "1",
    })

assert sha(binary) == BINARY_SHA, "frozen 9268890 binary hash mismatch"
server = home / "274bot-server-4c95f87"
launch = json.loads((server / "server-launch.json").read_text(encoding="utf-8-sig"))
pid = int(launch["pid"])
sample = wps.sample_process(pid)
public = json.loads((server / "server-login-public.json").read_text())
configure_environment(public, server)
catalog = home / ".274bot/js-scripts.json"
assert json.loads(catalog.read_text()) == [{"name": "trade_bot", "path": str(home / "274bot-workspaces/b4b686f/host/crates/script/tests/fixtures/trade_bot.ts")}]
for key in ["BOT_CPU", "BOT_DEBUG", "BOT_MEMORY_RENDER_POLICY", "BOT_MEMORY_SINGLE_RENDERER", "BOT_MEMORY_N", "BOT_MEMORY_WORKLOAD", "BOT_MEMORY_OUTPUT"]:
    os.environ.pop(key, None)
preflight = STAGE / ("preflight-tile-probe-" + cell_id + ".json")
assert preflight.is_file(), "run preflight first"
preflight_record = json.loads(preflight.read_text(encoding="utf-8-sig"))
manifest = out / "build-manifest.json"
server_id = out / "server-identity.json"
conditions = out / "host-conditions.json"
side = {"commit": HOST_COMMIT, "branch": "codex/memory-diagnostics", "build_exit": 0,
        "sources_sha256_pre": HOST_SOURCES, "sources_sha256_post": HOST_SOURCES,
        "host_sources_file_count": 871, "host_sources_aggregate_sha256": HOST_SOURCES,
        "sources_stable_across_build": True,
        "client": {"commit": CLIENT_COMMIT, "sources_sha256": CLIENT_SOURCES}}
dump(manifest, {"control": side, "features": {"requested": "memory-profile-no-alloc", "locked": True, "allocation_counting": False, "allocator": "std::alloc::System"},
    "binaries": {"control_panel_play": {"path": str(binary), "sha256": BINARY_SHA}},
    "nav": {"nav_pack": os.environ["NAV_PACK"], "nav_flags": os.environ["NAV_FLAGS"], "nav_pack_sha256": sha(os.environ["NAV_PACK"]), "nav_flags_sha256": sha(os.environ["NAV_FLAGS"])},
    "catalog": {"js_scripts_json": str(catalog), "js_scripts_json_sha256": sha(catalog)}, "performance_acceptance": False})
dump(server_id, {"configuration": {"bind_host": "127.0.0.1", "node_version": "24.19.0", "world_json_sha256": sha(server / "data/config/world.json"), "maps_addition_sha256": sha(server / "native-server-maps-addition.json"), "wordenc_addition_sha256": sha(server / "native-server-wordenc-addition.json")}, "pid": pid, "ProcessId": pid, "Name": "node.exe", "CreationDate": sample["start_identity"], "start_identity": sample["start_identity"], "port_listen": 43594, "server_commit": "4c95f87efe00b068cadbd229d94736626907bd1a", "launch": launch})
dump(conditions, {"purpose": "Native N16 focused-plus-background renderer owner census", "terminal_transport_expected": False, "panel_render_attribution": True, "platform": platform.platform(), "user": os.environ["USERNAME"], "cell_id": cell_id, "mode": "focused-plus-background", "count": 16, "background_fps": 1, "diagnostic": "tile-layout-occupancy", "performance_acceptance": False, "builds_stopped_before_run": True, "native_preflight": preflight_record, "checked_server": preflight_record["server"]})
diag = ["panel", "16", "active", "--binary", str(binary), "--build-manifest", str(manifest), "--build-role", "reference", "--sustain", "--warmup", "30", "--observe", "120", "--focused-background", "--render-profile", "--gpu-completion-profile", "--scheduling-profile", "--responsiveness-profile", "--responsiveness-fine", "--failure-capture"]
spec = {"id": cell_id, "index": 1, "cell_id": cell_id, "build_role": "reference", "manifest_role": "control", "mode": "focused-plus-background", "kind": "diagnostic", "frontend": "panel", "count": 16, "requested_adapter": "Intel(R) Graphics", "configured_background_fps": 1, "binary": str(binary), "build_manifest": str(manifest), "server_identity_path": str(server_id), "host_conditions_path": str(conditions), "nav_pack": os.environ["NAV_PACK"], "nav_flags": os.environ["NAV_FLAGS"], "catalog_path": str(catalog), "launcher_argv": [sys.executable, str(mem / "run_diagnostic.py"), *diag], "diagnostic_argv": diag, "game_server_pid": pid, "ambient_helpers": {"bootstrap": os.getppid()}, "sampler_interval_s": 0.5, "max_wall_s": 900, "observe_s": 120, "warmup_s": 30, "teardown_grace_s": 60, "process_backend": "system", "cache_dir": str(server / "data/pack/client"), "unpack_root": str(home / ".274bot/unpack"), "diagnostic": "tile-layout-occupancy", "performance_acceptance": False}
dump(out / "spec.json", spec)
dump(out / "started.json", {"pid": os.getpid(), "start_identity": wps.sample_process(os.getpid())["start_identity"], "started_unix": time.time(), "cell_id": cell_id, "count": 16, "requested_adapter": "Intel(R) Graphics", "BOT_RENDER_OWNER_CENSUS": True})
ES_CONTINUOUS, ES_SYSTEM_REQUIRED, ES_DISPLAY_REQUIRED = 0x80000000, 1, 2
keep_awake = ctypes.windll.kernel32.SetThreadExecutionState
keep_awake.argtypes = [ctypes.c_uint]; keep_awake.restype = ctypes.c_uint
if keep_awake(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED) == 0: raise OSError("SetThreadExecutionState failed")
try:
    rc = managed.main([str(out / "spec.json"), str(out / "cells")])
finally:
    keep_awake(ES_CONTINUOUS)
dump(out / "completion.json", {"exit_code": rc, "ended_unix": time.time(), "cell_id": cell_id, "count": 16, "requested_adapter": "Intel(R) Graphics", "performance_acceptance": False})
sys.exit(rc)
