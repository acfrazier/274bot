import json
import os
import pathlib
import runpy
import sys

assert os.environ["USERNAME"] == "BotTest"
root = pathlib.Path(os.environ.get("RENDER_OWNER_CENSUS_HOST_ROOT", pathlib.Path.home() / "274bot-workspaces/3118e96/host"))
sys.path.insert(0, str(root / "docs/memory"))
import matched_evidence_adapter as mea
import run_managed_cell as rmc

contract_id = os.environ.get("RENDER_OWNER_CENSUS_CONTRACT_ID", "native-render-owner-census-focused-plus-background-contractcheck-tile")
assert contract_id.startswith("native-render-owner-census-focused-plus-background-")
assert contract_id != os.environ.get("RENDER_OWNER_CENSUS_CELL_ID")
checks = []

def no_launch(argv):
    spec = rmc.validate_spec(rmc.load_spec(pathlib.Path(argv[0])))
    args = rmc.parse_diagnostic_argv(spec["diagnostic_argv"])
    rmc.require_argv_consistent_with_spec(spec, args)
    pf = rmc.preflight(spec, args)
    conditions = json.loads(pathlib.Path(spec["host_conditions_path"]).read_text())
    assert mea._native_windows_conditions_complete(conditions), "Incomplete native conditions"
    assert os.environ["BOT_RENDER_OWNER_CENSUS"] == "1"
    assert os.environ["BOT_RENDER_PROFILE"] == "1"
    checks.append({"id": spec["id"], "checked": True, "launched": False,
                   "server_pid": pf["server_pid"], "native_conditions_complete": True,
                   "owner_census_opt_in": True})
    return 0

rmc.main = no_launch
os.environ["RENDER_OWNER_CENSUS_CELL_ID"] = contract_id
runner = pathlib.Path(r"C:/ProgramData/274bot-Test/renderer-owner-census-9268890/run-panel-tile-probe-focused-one.py")
try:
    runpy.run_path(str(runner), run_name="__main__")
except SystemExit as exc:
    assert exc.code == 0, exc.code
out = pathlib.Path.home() / "274bot-runs" / ("owner-contractcheck-tile-probe-" + contract_id + ".json")
out.write_text(json.dumps({"kind": "no-launch contract test", "checks": checks,
                           "performance_acceptance": False}, indent=2))
print(out.read_text())
