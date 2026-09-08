"""Native paired panel controller; invoked only by the reviewed launcher.

No native, network, or live action occurs while this file is prepared.
"""
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
role = os.environ.get("TILE_BOXED_BUILD_ROLE", "")
mode = os.environ.get("TILE_BOXED_MODE", "")
cell_id = os.environ.get("TILE_BOXED_CELL_ID", "")
assert role in {"baseline", "candidate"}
assert mode in {"focused-one", "focused-plus-background"}
assert cell_id.startswith(role + "-" + mode + "-")
assert cell_id and all(c.isalnum() or c in "_-" for c in cell_id)

home = pathlib.Path(os.environ["USERPROFILE"])
root = pathlib.Path(os.environ.get("TILE_BOXED_HOST_ROOT", home / "274bot-workspaces/3118e96/host"))
mem = root / "docs/memory"
sys.path.insert(0, str(mem))
os.chdir(root)
import windows_process_sample as wps
import run_managed_cell as rmc

ROLES = {
    "baseline": {
        "stage": pathlib.Path(r"C:\ProgramData\274bot-Test\renderer-owner-census-9268890"),
        "manifest_role": "reference",
        "manifest_side": "control",
        "binary_sha256": "e2deb1db371366f674c18e39d04f7309480310072ade224ffa1433c84d4559b5",
        "host_commit": "9268890217d968cfeb7c66ebb11dd5c3dd2c084f",
        "host_sources_sha256_pre": "84054d9c02394d959cd84681dd85f3589d3a559b94bb37856c6f87cb0629269a",
        "host_sources_sha256_post": "84054d9c02394d959cd84681dd85f3589d3a559b94bb37856c6f87cb0629269a",
        "host_sources_file_count": 871,
        "host_sources_aggregate_sha256": "84054d9c02394d959cd84681dd85f3589d3a559b94bb37856c6f87cb0629269a",
        "client_sources_file_count": 183,
        "client_sources_aggregate_sha256": "ea6402702a56dc29359bf4effd9ca87407734fc9d940bfabdefb92cec8bf41ee",
        "branch": "codex/windows-submit-attribution",
    },
    "candidate": {
        "stage": pathlib.Path(r"C:\ProgramData\274bot-Test\tile-boxed-fb3589a"),
        "manifest_role": "candidate",
        "manifest_side": "candidate",
        "binary_sha256": "a9b581bab780d2868035941d799d416f0ef9ec68d5eb5a4fe0a62fa29670667f",
        "host_commit": "fb3589ac28583242b999ac864ea69c4ef8fa5923",
        "host_sources_sha256_pre": "189dac149beb6f258f5a79bdf09de43818556a5f06ffa783a608f0113a64d8a4",
        "host_sources_sha256_post": "189dac149beb6f258f5a79bdf09de43818556a5f06ffa783a608f0113a64d8a4",
        "host_sources_file_count": 876,
        "host_sources_aggregate_sha256": "189dac149beb6f258f5a79bdf09de43818556a5f06ffa783a608f0113a64d8a4",
        "client_sources_file_count": 183,
        "client_sources_aggregate_sha256": "fad2e79248c8d62cff2ceba5c757ef555705e85d3dc6797586c9841d9dbfb53e",
        "branch": "codex/memory-diagnostics",
    },
}
role_info = ROLES[role]
stage = role_info["stage"]
binary = stage / "panel-play.exe"
out = home / ("274bot-runs/managed-tile-boxed-" + cell_id)
out.mkdir(exist_ok=False)

def sha(path):
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()

def dump(path, value):
    pathlib.Path(path).write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")

assert sha(binary) == role_info["binary_sha256"]
server = home / "274bot-server-4c95f87"
launch = json.loads((server / "server-launch.json").read_text(encoding="utf-8-sig"))
pid = int(launch["pid"])
sample = wps.sample_process(pid)
def clean_environment():
    """Remove inherited diagnostics so clean cells cannot opt into census."""
    for key in ["BOT_RENDER_OWNER_CENSUS", "BOT_CPU", "BOT_DEBUG", "BOT_MEMORY_RENDER_POLICY", "BOT_MEMORY_SINGLE_RENDERER", "BOT_MEMORY_N", "BOT_MEMORY_WORKLOAD", "BOT_MEMORY_OUTPUT"]:
        os.environ.pop(key, None)

def diagnostic_argv(binary_path, manifest_path, build_role, run_mode):
    mode_flag = "--focused-one" if run_mode == "focused-one" else "--focused-background"
    argv = ["panel", "16", "active", "--binary", str(binary_path), "--build-manifest", str(manifest_path), "--build-role", build_role, "--sustain", "--warmup", "30", "--observe", "120", mode_flag, "--no-diagnostics", "--render-profile", "--gpu-completion-profile", "--scheduling-profile", "--responsiveness-profile", "--responsiveness-fine", "--failure-capture"]
    return argv
public = json.loads((server / "server-login-public.json").read_text())
os.environ.update({
    "LIVE": "1", "BOT_TARGET": "local", "ENGINE_DIR": str(server),
    "LOGIN_RSAN": public["modulus_decimal"], "LOGIN_RSAE": public["exponent_decimal"],
    "RS2B0T": str(home / "274bot-workspaces/b4b686f/rs2b0t"),
    "BOT_PANEL_RENDER_ATTRIBUTION": "1",
    "BOT_PANEL_DIAGNOSTIC_VULKAN_ADAPTER": "Intel(R) Graphics",
    "NAV_PACK": str(home / ".274bot/274bot.navpack"),
    "NAV_FLAGS": str(home / ".274bot/274bot.navflags"),
})
catalog = home / ".274bot/js-scripts.json"
assert json.loads(catalog.read_text()) == [{"name": "trade_bot", "path": str(home / "274bot-workspaces/b4b686f/host/crates/script/tests/fixtures/trade_bot.ts")}]
clean_environment()
preflight = pathlib.Path(r"C:\ProgramData\274bot-Test\renderer-owner-census-9268890") / ("preflight-tile-boxed-" + cell_id + ".json")
assert preflight.is_file(), "run paired preflight first"
preflight_record = json.loads(preflight.read_text(encoding="utf-8-sig"))
manifest = out / "build-manifest.json"
server_id = out / "server-identity.json"
conditions = out / "host-conditions.json"
side = {"commit": role_info["host_commit"], "branch": role_info["branch"], "build_exit": 0, "sources_sha256_pre": role_info["host_sources_sha256_pre"], "sources_sha256_post": role_info["host_sources_sha256_post"], "host_sources_file_count": role_info["host_sources_file_count"], "host_sources_aggregate_sha256": role_info["host_sources_aggregate_sha256"], "sources_stable_across_build": True, "client": {"commit": "abb811bd0afa1acd99319ccd5bc36bfb241080f9", "sources_sha256": "ea6402702a56dc29359bf4effd9ca87407734fc9d940bfabdefb92cec8bf41ee", "sources_file_count": 183, "sources_aggregate_sha256": "ea6402702a56dc29359bf4effd9ca87407734fc9d940bfabdefb92cec8bf41ee"} if role == "baseline" else {"commit": "fd956c91bf09e059359c8e182a33583e2c626cd3", "sources_sha256": "fad2e79248c8d62cff2ceba5c757ef555705e85d3dc6797586c9841d9dbfb53e", "sources_file_count": 183, "sources_aggregate_sha256": "fad2e79248c8d62cff2ceba5c757ef555705e85d3dc6797586c9841d9dbfb53e"}}
dump(manifest, {role_info["manifest_side"]: side, "features": {"requested": "memory-profile-no-alloc", "locked": True, "allocation_counting": False, "allocator": "std::alloc::System"}, "binaries": {role_info["manifest_side"] + "_panel_play": {"path": str(binary), "sha256": sha(binary)}}, "nav": {"nav_pack": os.environ["NAV_PACK"], "nav_flags": os.environ["NAV_FLAGS"], "nav_pack_sha256": sha(os.environ["NAV_PACK"]), "nav_flags_sha256": sha(os.environ["NAV_FLAGS"])}, "catalog": {"js_scripts_json": str(catalog), "js_scripts_json_sha256": sha(catalog)}, "performance_acceptance": False})
dump(server_id, {"configuration": {"world_json_sha256": sha(server / "data/config/world.json"), "maps_addition_sha256": sha(server / "native-server-maps-addition.json"), "wordenc_addition_sha256": sha(server / "native-server-wordenc-addition.json"), "bind_host": "127.0.0.1", "node_version": "24.19.0"}, "pid": pid, "start_identity": sample["start_identity"], "port_listen": 43594, "server_commit": "4c95f87efe00b068cadbd229d94736626907bd1a", "launch": launch, "sample": sample, "config_sha256": sha(server / "data/config/world.json"), "public_key_sha256": sha(server / "data/config/public.pem")})
dump(conditions, {"purpose": "Native N16 boxed-tile paired diagnostic", "terminal_transport_expected": False, "panel_render_attribution": True, "platform": platform.platform(), "user": os.environ["USERNAME"], "role": role, "mode": mode, "cell_id": cell_id, "performance_acceptance": False, "builds_stopped_before_run": True, "native_preflight": preflight_record, "checked_server": preflight_record["server"]})
# Clean comparison deliberately avoids navigation PNG capture.
diag = diagnostic_argv(binary, manifest, role_info["manifest_role"], mode)
spec = {"id": "native-panel-tile-boxed-" + cell_id, "index": 1, "cell_id": cell_id, "build_role": role_info["manifest_role"], "mode": mode, "kind": "diagnostic", "frontend": "panel", "count": 16, "requested_adapter": "Intel(R) Graphics", "configured_background_fps": 1 if mode == "focused-plus-background" else None, "binary": str(binary), "build_manifest": str(manifest), "server_identity_path": str(server_id), "host_conditions_path": str(conditions), "nav_pack": os.environ["NAV_PACK"], "nav_flags": os.environ["NAV_FLAGS"], "catalog_path": str(catalog), "launcher_argv": [sys.executable, str(mem / "run_diagnostic.py"), *diag], "diagnostic_argv": diag, "game_server_pid": pid, "ambient_helpers": {"bootstrap": os.getppid()}, "sampler_interval_s": 0.5, "max_wall_s": 900, "observe_s": 120, "warmup_s": 30, "teardown_grace_s": 60, "process_backend": "system", "cache_dir": str(server / "data/pack/client"), "unpack_root": str(home / ".274bot/unpack"), "performance_acceptance": False}
sp = out / "spec.json"
dump(sp, spec)
dump(out / "started.json", {"pid": os.getpid(), "start_identity": wps.sample_process(os.getpid())["start_identity"], "started_unix": time.time(), "role": role, "mode": mode, "cell_id": cell_id, "count": 16, "requested_adapter": "Intel(R) Graphics"})
ES_CONTINUOUS, ES_SYSTEM_REQUIRED, ES_DISPLAY_REQUIRED = 0x80000000, 1, 2
keep_awake = ctypes.windll.kernel32.SetThreadExecutionState
keep_awake.argtypes = [ctypes.c_uint]
keep_awake.restype = ctypes.c_uint
if keep_awake(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED) == 0:
    raise OSError("SetThreadExecutionState failed")
try:
    rc = rmc.main([str(sp), str(out / "cells")])
finally:
    keep_awake(ES_CONTINUOUS)
dump(out / "completion.json", {"exit_code": rc, "ended_unix": time.time(), "role": role, "mode": mode, "cell_id": cell_id, "count": 16, "requested_adapter": "Intel(R) Graphics", "performance_acceptance": False})
sys.exit(rc)
