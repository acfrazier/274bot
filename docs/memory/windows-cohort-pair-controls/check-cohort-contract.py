import json
import os
import pathlib
import re
import runpy
import sys

assert os.environ["USERNAME"] == "BotTest"
root = pathlib.Path(os.environ.get("COHORT_HOST_ROOT", pathlib.Path.home() / "274bot-workspaces/3118e96/host"))
sys.path.insert(0, str(root / "docs/memory"))
import matched_evidence_adapter as mea
import run_managed_cell as rmc

contract_id = os.environ["COHORT_CONTRACT_ID"]
role = os.environ["COHORT_BUILD_ROLE"]
mode = os.environ["COHORT_MODE"]
cell_id = os.environ["COHORT_CELL_ID"]
target_cell_id = os.environ["COHORT_TARGET_CELL_ID"]
assert role in {"baseline", "candidate"}
assert mode == "focused-one"
assert target_cell_id.startswith(role + "-" + mode + "-")
assert contract_id == target_cell_id + "-cohort-contractcheck"
assert cell_id == contract_id
checks = []

def server_configuration_complete(server):
    configuration = server.get("configuration") if isinstance(server, dict) else None
    required = ("world_json_sha256", "maps_addition_sha256", "wordenc_addition_sha256", "bind_host", "node_version")
    digest_keys = ("world_json_sha256", "maps_addition_sha256", "wordenc_addition_sha256")
    return (isinstance(configuration, dict)
            and all(isinstance(configuration.get(key), str) and re.fullmatch(r"[0-9a-f]{64}", configuration[key]) for key in digest_keys)
            and configuration.get("bind_host") == "127.0.0.1"
            and configuration.get("node_version") == "24.19.0")

def long_argv_contract_complete(args, argv=()):
    return (args.n == 16 and args.focused_one and not args.focused_background and args.warmup == 120
            and args.observe == 600 and not getattr(args, "cpu_fallback", False) and not getattr(args, "nav_captures", False)
            and getattr(args, "no_diagnostics", False) and getattr(args, "failure_capture", False)
            and getattr(args, "render_profile", False) and getattr(args, "gpu_completion_profile", False)
            and "--cpu-fallback" not in argv and "--owner-census" not in argv)

def no_launch(argv):
    spec = rmc.validate_spec(rmc.load_spec(pathlib.Path(argv[0])))
    args = rmc.parse_diagnostic_argv(spec["diagnostic_argv"])
    rmc.require_argv_consistent_with_spec(spec, args)
    assert spec["count"] == 16 and spec["mode"] == "focused-one"
    assert (spec["warmup_s"], spec["observe_s"], spec["teardown_grace_s"]) == (120, 600, 60)
    assert spec["requested_backend"] == "gpu" and spec["requested_adapter"] == "Intel(R) Graphics"
    assert long_argv_contract_complete(args, spec["diagnostic_argv"]), "Long confirmation argv contract failed"
    server = json.loads(pathlib.Path(spec["server_identity_path"]).read_text())
    assert server_configuration_complete(server), "Incomplete generated server configuration"
    pf = rmc.preflight(spec, args)
    conditions = json.loads(pathlib.Path(spec["host_conditions_path"]).read_text())
    assert mea._native_windows_conditions_complete(conditions), "Incomplete native conditions"
    assert not os.environ.get("BOT_CPU")
    assert os.environ.get("BOT_RENDER_OWNER_CENSUS") != "1"

    assert spec["cell_id"] == contract_id
    checks.append({"id": target_cell_id, "contract_cell_id": cell_id, "checked": True, "launched": False, "client_started": False,
                   "server_pid": pf["server_pid"], "native_conditions_complete": True,
                   "owner_census_opt_in": False})
    return 0

rmc.main = no_launch
os.environ["COHORT_HOST_ROOT"] = str(root)
stage = pathlib.Path(os.environ.get("COHORT_REFERENCE_STAGE" if role == "baseline" else "COHORT_CANDIDATE_STAGE", r"C:/ProgramData/274bot-Test/cohort-reference" if role == "baseline" else r"C:/ProgramData/274bot-Test/cohort-candidate"))
runner = stage / ("run-" + contract_id + ".py")
try:
    runpy.run_path(str(runner), run_name="__main__")
except SystemExit as exc:
    assert exc.code == 0, exc.code
out = pathlib.Path.home() / "274bot-runs" / ("cohort-contract-" + contract_id + ".json")
out.write_text(json.dumps({"kind": "no-launch contract test", "cell_id": target_cell_id,
                           "check_id": target_cell_id,
                           "contract_id": contract_id, "checks": checks,
                           "performance_acceptance": False}, indent=2))
print(out.read_text())
