import json
import os
import pathlib
import runpy
import sys

assert os.environ["USERNAME"] == "BotTest"
root = pathlib.Path(os.environ.get("TILE_BOXED_HOST_ROOT", pathlib.Path.home() / "274bot-workspaces/3118e96/host"))
sys.path.insert(0, str(root / "docs/memory"))
import matched_evidence_adapter as mea
import run_managed_cell as rmc

contract_id = os.environ["TILE_BOXED_CONTRACT_ID"]
role = os.environ["TILE_BOXED_BUILD_ROLE"]
mode = os.environ["TILE_BOXED_MODE"]
cell_id = os.environ["TILE_BOXED_CELL_ID"]
target_cell_id = os.environ["TILE_BOXED_TARGET_CELL_ID"]
assert role in {"baseline", "candidate"}
assert mode in {"focused-one", "focused-plus-background"}
assert target_cell_id.startswith(role + "-" + mode + "-")
assert contract_id == target_cell_id + "-contractcheck"
assert cell_id == contract_id
checks = []

def no_launch(argv):
    spec = rmc.validate_spec(rmc.load_spec(pathlib.Path(argv[0])))
    args = rmc.parse_diagnostic_argv(spec["diagnostic_argv"])
    rmc.require_argv_consistent_with_spec(spec, args)
    assert args.no_diagnostics
    assert args.failure_capture
    assert not args.nav_captures
    pf = rmc.preflight(spec, args)
    conditions = json.loads(pathlib.Path(spec["host_conditions_path"]).read_text())
    assert mea._native_windows_conditions_complete(conditions), "Incomplete native conditions"
    assert os.environ.get("BOT_RENDER_OWNER_CENSUS") != "1"

    assert spec["cell_id"] == contract_id
    checks.append({"id": target_cell_id, "contract_cell_id": cell_id, "checked": True, "launched": False, "client_started": False,
                   "server_pid": pf["server_pid"], "native_conditions_complete": True,
                   "owner_census_opt_in": False})
    return 0

rmc.main = no_launch
os.environ["TILE_BOXED_HOST_ROOT"] = str(root)
stage = pathlib.Path(r"C:/ProgramData/274bot-Test/renderer-owner-census-9268890" if role == "baseline" else r"C:/ProgramData/274bot-Test/tile-boxed-fb3589a")
runner = stage / ("run-" + contract_id + ".py")
try:
    runpy.run_path(str(runner), run_name="__main__")
except SystemExit as exc:
    assert exc.code == 0, exc.code
out = pathlib.Path.home() / "274bot-runs" / ("tile-boxed-contract-" + contract_id + ".json")
out.write_text(json.dumps({"kind": "no-launch contract test", "cell_id": target_cell_id,
                           "check_id": target_cell_id,
                           "contract_id": contract_id, "checks": checks,
                           "performance_acceptance": False}, indent=2))
print(out.read_text())
