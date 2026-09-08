"""Native cohort matched-pair controller; invoked only by reviewed launcher.

No native, network, or live action occurs while this file is prepared.
"""
import ctypes
import hashlib
import json
import math
import os
import pathlib
import platform
import re
import sys
import time

assert os.environ.get("USERNAME") == "BotTest"
assert ctypes.windll.user32.GetSystemMetrics(0x1000) == 0, "requires local console"
role = os.environ.get("COHORT_BUILD_ROLE", "")
mode = os.environ.get("COHORT_MODE", "")
cell_id = os.environ.get("COHORT_CELL_ID", "")
assert role in {"baseline", "candidate"}
assert mode == "focused-one"
assert cell_id.startswith(role + "-" + mode + "-")
assert cell_id and all(c.isalnum() or c in "_-" for c in cell_id)

home = pathlib.Path(os.environ["USERPROFILE"])
root = pathlib.Path(os.environ.get("COHORT_HOST_ROOT", home / "274bot-workspaces/3118e96/host"))
mem = root / "docs/memory"
sys.path.insert(0, str(mem))
os.chdir(root)
import windows_process_sample as wps
import run_managed_cell as rmc

ROLES = {
    "baseline": {
        "stage": pathlib.Path(os.environ.get("COHORT_REFERENCE_STAGE", r"C:\ProgramData\274bot-Test\cohort-reference-e25f328")),
        "manifest_role": "reference",
        "manifest_side": "control",
        "host_commit": "e25f32806957b1a44c75a598cbdb7ab53afc5383",
        "client_commit": "abb811bd0afa1acd99319ccd5bc36bfb241080f9",
    },
    "candidate": {
        "stage": pathlib.Path(os.environ.get("COHORT_CANDIDATE_STAGE", r"C:\ProgramData\274bot-Test\cohort-candidate-ca56e143")),
        "manifest_role": "candidate",
        "manifest_side": "candidate",

        "host_commit": "ca56e14371d1edb8c09df1276638ce196d502e36",
        "client_commit": "fd956c91bf09e059359c8e182a33583e2c626cd3",
    },
}
role_info = ROLES[role]
stage = role_info["stage"]
binary = stage / "panel-play.exe"
out = home / ("274bot-runs/managed-cohort-pair-" + cell_id)
out.mkdir(exist_ok=False)

def sha(path):
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()

def dump(path, value):
    pathlib.Path(path).write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")

expected_client = "abb811bd0afa1acd99319ccd5bc36bfb241080f9" if role == "baseline" else "fd956c91bf09e059359c8e182a33583e2c626cd3"

def validate_build_receipt(receipt, *, role_name, role_info, binary_path, expected_client):
    """Validate root-supplied frozen provenance; never accept placeholders."""
    if receipt.get("schema") != "native-cohort-build-receipt-v1":
        raise AssertionError("invalid frozen build receipt schema")
    if (receipt.get("role"), receipt.get("manifest_role"), receipt.get("host_commit")) != (role_name, role_info["manifest_role"], role_info["host_commit"]):
        raise AssertionError("frozen receipt role/source mismatch")
    if receipt.get("client_commit") != expected_client or receipt.get("binary") != str(binary_path):
        raise AssertionError("frozen receipt client/binary mismatch")
    digest = receipt.get("binary_sha256", "")
    if re.fullmatch(r"[0-9a-f]{64}", digest or "") is None:
        raise AssertionError("frozen receipt binary hash is required")
    return receipt

def validate_stimulus_plan(path):
    """Validate only predeclared facts; observed identity belongs post-run."""
    plan = json.loads(pathlib.Path(path).read_text(encoding="utf-8-sig"))
    if plan.get("schema") != "native-panel-input-stimulus-plan-v1":
        raise AssertionError("invalid stimulus plan schema")
    if plan.get("cadenceMilliseconds") != 1000 or plan.get("pressMilliseconds") != 80 or plan.get("durationSeconds") != 120:
        raise AssertionError("stimulus plan cadence/duration mismatch")
    if plan.get("helperSha256") != "04822c0e9f5ade2d555e408cc707722c234bc1e7b442676975a16e7efcaebbd1":
        raise AssertionError("stimulus plan helper hash is required")
    if plan.get("triggerWindowAfterObserveStartSeconds") != [60, 90] or plan.get("noRetry") is not True:
        raise AssertionError("stimulus plan trigger window/retry policy mismatch")
    if plan.get("receiptPath") in (None, ""):
        raise AssertionError("stimulus plan receipt path is required")
    for observed in ("pid", "startUtc", "observedRect", "gameImagePoint", "scene2"):
        if observed in plan:
            raise AssertionError("stimulus plan contains observed run facts")
    return plan

def validate_stimulus_receipt(path, *, plan, cell_id, run_started_unix):
    """Validate the separately root-managed post-run helper envelope."""
    receipt = json.loads(pathlib.Path(path).read_text(encoding="utf-8-sig"))
    if receipt.get("schema") != "native-panel-input-stimulus-run-receipt-v1":
        raise AssertionError("invalid post-run stimulus receipt schema")
    if receipt.get("cellId") != cell_id or receipt.get("outcome") != "completed":
        raise AssertionError("stimulus receipt run identity/outcome mismatch")
    if receipt.get("helperSha256") != plan["helperSha256"] or receipt.get("cadenceMilliseconds") != 1000 or receipt.get("pressMilliseconds") != 80 or receipt.get("durationSeconds") != 120:
        raise AssertionError("stimulus receipt helper binding mismatch")
    if receipt.get("startedUnix", 0) < run_started_unix:
        raise AssertionError("stimulus receipt predates this run")
    if receipt.get("captureEnabledVerified") is not True or receipt.get("slotZeroFocusVerified") is not True:
        raise AssertionError("stimulus receipt lacks UI binding verification")
    if not isinstance(receipt.get("helperPid"), int) or not receipt.get("helperStartUtc") or not receipt.get("targetPid") or not receipt.get("targetStartUtc"):
        raise AssertionError("stimulus receipt lacks helper/target process identity")
    delay = receipt.get("triggerDelaySeconds")
    if type(delay) not in (int, float) or not math.isfinite(delay) or not 60 <= delay <= 90:
        raise AssertionError("stimulus trigger was outside the observe-start window")
    accounting = receipt.get("resourceAccounting")
    if not isinstance(accounting, dict) or accounting.get("status") != "available":
        raise AssertionError("stimulus process accounting is unavailable")
    if accounting.get("sampler") != "root-managed windows_process_sample":
        raise AssertionError("stimulus process sampler is not identified")
    for process_name in ("helper", "target"):
        sample = accounting.get(process_name)
        if not isinstance(sample, dict) or not isinstance(sample.get("pid"), int) or not sample.get("start_identity"):
            raise AssertionError("stimulus process accounting lacks identity samples")
        if not isinstance(sample.get("cpu_seconds"), (int, float)) or not isinstance(sample.get("rss_bytes"), int):
            raise AssertionError("stimulus process accounting lacks resource samples")
    if not isinstance(accounting.get("samples"), list) or not accounting["samples"]:
        raise AssertionError("stimulus process accounting has no sample records")
    if receipt.get("performanceAcceptance") is not False or receipt.get("inputCoveragePass") is not False:
        raise AssertionError("stimulus receipt must not claim acceptance")
    return receipt

def validate_prepare_receipt(receipt, *, cell_id):
    """Validate the privileged no-launch receipt consumed only at launch."""
    if receipt.get("schema") != "cohort-pair-prepare-no-launch-v1":
        raise AssertionError("invalid prepare receipt schema")
    if receipt.get("cell_id") != cell_id or receipt.get("client_started") is not False or receipt.get("scheduled_task_created") is not False:
        raise AssertionError("prepare receipt is not an exact no-launch receipt")
    return receipt

receipt_path = pathlib.Path(os.environ.get("COHORT_BUILD_RECEIPT", ""))
assert receipt_path.is_file(), "explicit frozen build receipt is required"
build_receipt = validate_build_receipt(json.loads(receipt_path.read_text(encoding="utf-8-sig")), role_name=role, role_info=role_info, binary_path=binary, expected_client=expected_client)
assert sha(binary) == build_receipt["binary_sha256"]
stimulus_plan_path = pathlib.Path(os.environ.get("COHORT_STIMULUS_PLAN", ""))
assert stimulus_plan_path.is_file(), "explicit root-managed stimulus plan is required"
stimulus_plan = validate_stimulus_plan(stimulus_plan_path)
stimulus_path = pathlib.Path(os.environ.get("COHORT_STIMULUS_RECEIPT") or stimulus_plan["receiptPath"])
prepare_receipt_path = pathlib.Path(os.environ.get("COHORT_PREPARE_RECEIPT", ""))
no_launch_validation = os.environ.get("COHORT_NO_LAUNCH_VALIDATION") == "1"
if no_launch_validation:
    # Contract validation is first; preparation consumes its receipt later.
    prepare_receipt = None
else:
    assert prepare_receipt_path.is_file(), "explicit consumed no-launch prepare receipt is required"
    prepare_receipt = validate_prepare_receipt(json.loads(prepare_receipt_path.read_text(encoding="utf-8-sig")), cell_id=cell_id)
server = home / "274bot-server-4c95f87"
launch = json.loads((server / "server-launch.json").read_text(encoding="utf-8-sig"))
pid = int(launch["pid"])
sample = wps.sample_process(pid)
def clean_environment():
    """Remove inherited diagnostics so clean cells cannot opt into census."""
    for key in ["BOT_RENDER_OWNER_CENSUS", "BOT_CPU", "BOT_DEBUG", "BOT_INPUT_SEAM_TRACE", "BOT_MEMORY_RENDER_POLICY", "BOT_MEMORY_SINGLE_RENDERER", "BOT_MEMORY_N", "BOT_MEMORY_WORKLOAD", "BOT_MEMORY_OUTPUT"]:
        os.environ.pop(key, None)

def diagnostic_argv(binary_path, manifest_path, build_role, run_mode):
    assert run_mode == "focused-one"
    argv = ["panel", "16", "active", "--binary", str(binary_path), "--build-manifest", str(manifest_path), "--build-role", build_role, "--sustain", "--warmup", "120", "--observe", "600", "--focused-one", "--no-diagnostics", "--render-profile", "--gpu-completion-profile", "--scheduling-profile", "--responsiveness-profile", "--responsiveness-fine", "--failure-capture"]
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
preflight_cell_id = os.environ.get("COHORT_PREFLIGHT_CELL_ID", cell_id)
preflight = pathlib.Path(os.environ.get("COHORT_PREFLIGHT_DIR", r"C:\ProgramData\274bot-Test\cohort-preflight")) / ("preflight-cohort-" + preflight_cell_id + ".json")
assert preflight.is_file(), "run paired preflight first"
preflight_record = json.loads(preflight.read_text(encoding="utf-8-sig"))
manifest = out / "build-manifest.json"
server_id = out / "server-identity.json"
conditions = out / "host-conditions.json"
side = {"commit": role_info["host_commit"], "build_exit": 0, "host_commit": role_info["host_commit"], "client": {"commit": role_info["client_commit"]}}
manifest_source = pathlib.Path(os.environ.get("COHORT_BUILD_MANIFEST", ""))
assert manifest_source.is_file(), "explicit frozen build manifest is required"
assert str(manifest_source.resolve()) == build_receipt.get("manifest"), "receipt/manifest binding mismatch"
manifest_record = json.loads(manifest_source.read_text(encoding="utf-8-sig"))
assert manifest_record.get(role_info["manifest_side"], {}).get("commit") == role_info["host_commit"]
assert manifest_record.get(role_info["manifest_side"], {}).get("client", {}).get("commit") == expected_client
assert manifest_record.get("binaries", {}).get(role_info["manifest_side"] + "_panel_play", {}).get("sha256") == build_receipt["binary_sha256"]
dump(manifest, manifest_record)
dump(server_id, {"configuration": {"world_json_sha256": sha(server / "data/config/world.json"), "maps_addition_sha256": sha(server / "native-server-maps-addition.json"), "wordenc_addition_sha256": sha(server / "native-server-wordenc-addition.json"), "bind_host": "127.0.0.1", "node_version": "24.19.0"}, "pid": pid, "start_identity": sample["start_identity"], "port_listen": 43594, "server_commit": "4c95f87efe00b068cadbd229d94736626907bd1a", "launch": launch, "sample": sample, "config_sha256": sha(server / "data/config/world.json"), "public_key_sha256": sha(server / "data/config/public.pem")})
dump(conditions, {"purpose": "Native N16 cohort matched pair diagnostic", "terminal_transport_expected": False, "panel_render_attribution": True, "platform": platform.platform(), "user": os.environ["USERNAME"], "role": role, "mode": mode, "cell_id": cell_id, "performance_acceptance": False, "builds_stopped_before_run": True, "native_preflight": preflight_record, "checked_server": preflight_record["server"], "stimulus": {"controller": "root-managed receipt-bound helper", "plan_required": True, "completed_receipt_after_run": True, "binding": "slot0 Game Image after scene2 capture verification"}})
# The longer confirmation deliberately avoids navigation PNG capture and all census/counting diagnostics.
diag = diagnostic_argv(binary, manifest, role_info["manifest_role"], mode)
spec = {"id": "native-panel-cohort-pair-" + cell_id, "index": 1, "cell_id": cell_id, "build_role": role_info["manifest_role"], "mode": mode, "kind": "diagnostic", "frontend": "panel", "count": 16, "requested_backend": "gpu", "requested_adapter": "Intel(R) Graphics", "configured_background_fps": None, "binary": str(binary), "build_manifest": str(manifest), "build_receipt": str(receipt_path), "server_identity_path": str(server_id), "host_conditions_path": str(conditions), "nav_pack": os.environ["NAV_PACK"], "nav_flags": os.environ["NAV_FLAGS"], "catalog_path": str(catalog), "launcher_argv": [sys.executable, str(mem / "run_diagnostic.py"), *diag], "diagnostic_argv": diag, "game_server_pid": pid, "ambient_helpers": {"bootstrap": os.getppid()}, "sampler_interval_s": 0.5, "max_wall_s": 1500, "observe_s": 600, "warmup_s": 120, "teardown_grace_s": 60, "cohort_tail_s": 5, "process_backend": "system", "cache_dir": str(server / "data/pack/client"), "unpack_root": str(home / ".274bot/unpack"), "stimulus_plan": str(stimulus_plan_path), "stimulus_receipt": str(stimulus_path), "stimulus_schema": "native-panel-input-stimulus-run-receipt-v1", "prepare_receipt": str(prepare_receipt_path) if prepare_receipt is not None else None, "no_launch_validation": no_launch_validation, "performance_acceptance": False}
sp = out / "spec.json"
dump(sp, spec)
dump(out / "started.json", {"pid": os.getpid(), "start_identity": wps.sample_process(os.getpid())["start_identity"], "started_unix": time.time(), "role": role, "mode": mode, "cell_id": cell_id, "count": 16, "observe_s": 600, "warmup_s": 120, "cohort_tail_s": 5, "teardown_grace_s": 60, "requested_backend": "gpu", "requested_adapter": "Intel(R) Graphics", "build_receipt": str(receipt_path), "stimulus_plan": str(stimulus_plan_path), "stimulus_receipt": str(stimulus_path)})
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
try:
    stimulus_receipt = validate_stimulus_receipt(stimulus_path, plan=stimulus_plan, cell_id=cell_id, run_started_unix=json.loads((out / "started.json").read_text())["started_unix"])
    dump(out / "stimulus-receipt.json", stimulus_receipt)
    stimulus_status = {"status": "complete", "reason": None}
except (AssertionError, OSError, json.JSONDecodeError) as exc:
    stimulus_status = {"status": "incomplete", "reason": str(exc)}
    dump(out / "stimulus-validation.json", stimulus_status)
dump(out / "completion.json", {"exit_code": rc, "ended_unix": time.time(), "role": role, "mode": mode, "cell_id": cell_id, "count": 16, "observe_s": 600, "warmup_s": 120, "cohort_tail_s": 5, "teardown_grace_s": 60, "requested_backend": "gpu", "requested_adapter": "Intel(R) Graphics", "stimulus_plan": str(stimulus_plan_path), "stimulus_receipt": str(stimulus_path), "stimulus_status": stimulus_status, "performance_acceptance": False})
sys.exit(rc)
