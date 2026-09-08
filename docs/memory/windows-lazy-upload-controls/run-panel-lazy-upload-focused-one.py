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
role = os.environ.get("LAZY_UPLOAD_BUILD_ROLE", "")
mode = os.environ.get("LAZY_UPLOAD_MODE", "")
cell_id = os.environ.get("LAZY_UPLOAD_CELL_ID", "")
assert role in {"baseline", "candidate"}
assert mode in {"focused-one", "focused-plus-background"}
assert cell_id.startswith(role + "-" + mode + "-")
assert cell_id and all(c.isalnum() or c in "_-" for c in cell_id)

home = pathlib.Path(os.environ["USERPROFILE"])
root = pathlib.Path(os.environ.get("LAZY_UPLOAD_HOST_ROOT", home / "274bot-workspaces/3118e96/host"))
mem = root / "docs/memory"
sys.path.insert(0, str(mem))
os.chdir(root)
import windows_process_sample as wps
import run_managed_cell as rmc

ROLES = {
    "baseline": {
        "stage": pathlib.Path(r"C:\ProgramData\274bot-Test\panel-36825a9"),
        "manifest_role": "reference",
        "manifest_side": "control",
        "binary_sha256": "87c24665563ef8a55751244a52f7d25c7edf68e3117e84f2ff464add596c71fb",
        "host_commit": "36825a9f0a07a19610439c691f6dfe3d2e1cbf48",
        "host_sources_sha256_pre": "cf0175c549c573a37ebd8af31242b2eb817946a716f76b2785d3d4d8a76f3bca",
        "host_sources_sha256_post": "cf0175c549c573a37ebd8af31242b2eb817946a716f76b2785d3d4d8a76f3bca",
        "branch": "codex/windows-submit-attribution",
    },
    "candidate": {
        "stage": pathlib.Path(r"C:\ProgramData\274bot-Test\panel-3118e96"),
        "manifest_role": "candidate",
        "manifest_side": "candidate",
        "binary_sha256": "1937663506e3f0b243f5a9a742de3ae8d86ded66867bf2db38f9f882ec76e2e3",
        "host_commit": "3118e9661a89906589ee6e0239e475b36228bde3",
        "host_sources_sha256_pre": "2a1a551539e254abbd22914c965eb451a347e409955d0e6e53a76bbd9fd04ff8",
        "host_sources_sha256_post": "2a1a551539e254abbd22914c965eb451a347e409955d0e6e53a76bbd9fd04ff8",
        "host_sources_file_count": 852,
        "host_sources_aggregate_sha256": "2a1a551539e254abbd22914c965eb451a347e409955d0e6e53a76bbd9fd04ff8",
        "branch": "codex/memory-diagnostics",
    },
}
role_info = ROLES[role]
stage = role_info["stage"]
binary = stage / "panel-play.exe"
out = home / ("274bot-runs/managed-panel-lazy-upload-" + cell_id)
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
for key in ["BOT_CPU", "BOT_DEBUG", "BOT_MEMORY_RENDER_POLICY", "BOT_MEMORY_SINGLE_RENDERER", "BOT_MEMORY_N", "BOT_MEMORY_WORKLOAD", "BOT_MEMORY_OUTPUT"]:
    os.environ.pop(key, None)
preflight = pathlib.Path(r"C:\ProgramData\274bot-Test\panel-36825a9") / ("preflight-panel-lazy-upload-" + cell_id + ".json")
assert preflight.is_file(), "run paired preflight first"
preflight_record = json.loads(preflight.read_text(encoding="utf-8-sig"))
manifest = out / "build-manifest.json"
server_id = out / "server-identity.json"
conditions = out / "host-conditions.json"
side = {"commit": role_info["host_commit"], "branch": role_info["branch"], "build_exit": 0, "sources_sha256_pre": role_info["host_sources_sha256_pre"], "sources_sha256_post": role_info["host_sources_sha256_post"], "host_sources_file_count": role_info.get("host_sources_file_count"), "host_sources_aggregate_sha256": role_info.get("host_sources_aggregate_sha256"), "sources_stable_across_build": True, "client": {"commit": "5ee9b6efb2342452ceeb7958f864fd68d0daadd1", "sources_sha256": "f71f99afabf29025a91179554792c35343093643a1f386cf1676d9bf2dedca3d"}}
dump(manifest, {role_info["manifest_side"]: side, "features": {"requested": "memory-profile-no-alloc", "locked": True, "allocation_counting": False, "allocator": "std::alloc::System"}, "binaries": {role_info["manifest_side"] + "_panel_play": {"path": str(binary), "sha256": sha(binary)}}, "nav": {"nav_pack": os.environ["NAV_PACK"], "nav_flags": os.environ["NAV_FLAGS"], "nav_pack_sha256": sha(os.environ["NAV_PACK"]), "nav_flags_sha256": sha(os.environ["NAV_FLAGS"])}, "catalog": {"js_scripts_json": str(catalog), "js_scripts_json_sha256": sha(catalog)}, "performance_acceptance": False})
dump(server_id, {"configuration": {"world_json_sha256": sha(server / "data/config/world.json"), "maps_addition_sha256": sha(server / "native-server-maps-addition.json"), "wordenc_addition_sha256": sha(server / "native-server-wordenc-addition.json"), "bind_host": "127.0.0.1", "node_version": "24.19.0"}, "pid": pid, "start_identity": sample["start_identity"], "port_listen": 43594, "server_commit": "4c95f87efe00b068cadbd229d94736626907bd1a", "launch": launch, "sample": sample, "config_sha256": sha(server / "data/config/world.json"), "public_key_sha256": sha(server / "data/config/public.pem")})
dump(conditions, {"purpose": "Native N16 lazy-upload paired diagnostic", "terminal_transport_expected": False, "panel_render_attribution": True, "platform": platform.platform(), "user": os.environ["USERNAME"], "role": role, "mode": mode, "cell_id": cell_id, "performance_acceptance": False, "builds_stopped_before_run": True, "native_preflight": preflight_record, "checked_server": preflight_record["server"]})
mode_flag = "--focused-one" if mode == "focused-one" else "--focused-background"
diag = ["panel", "16", "active", "--binary", str(binary), "--build-manifest", str(manifest), "--build-role", role_info["manifest_role"], "--sustain", "--warmup", "30"]
if mode == "focused-one":
    diag += ["--nav-captures"]
diag += ["--observe", "120", mode_flag, "--render-profile", "--gpu-completion-profile", "--scheduling-profile", "--responsiveness-profile", "--responsiveness-fine", "--failure-capture"]
spec = {"id": "native-panel-lazy-upload-" + cell_id, "index": 1, "cell_id": cell_id, "build_role": role_info["manifest_role"], "mode": mode, "kind": "diagnostic", "frontend": "panel", "count": 16, "requested_adapter": "Intel(R) Graphics", "configured_background_fps": 1 if mode == "focused-plus-background" else None, "binary": str(binary), "build_manifest": str(manifest), "server_identity_path": str(server_id), "host_conditions_path": str(conditions), "nav_pack": os.environ["NAV_PACK"], "nav_flags": os.environ["NAV_FLAGS"], "catalog_path": str(catalog), "launcher_argv": [sys.executable, str(mem / "run_diagnostic.py"), *diag], "diagnostic_argv": diag, "game_server_pid": pid, "ambient_helpers": {"bootstrap": os.getppid()}, "sampler_interval_s": 0.5, "max_wall_s": 900, "observe_s": 120, "warmup_s": 30, "teardown_grace_s": 60, "process_backend": "system", "cache_dir": str(server / "data/pack/client"), "unpack_root": str(home / ".274bot/unpack"), "performance_acceptance": False}
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
