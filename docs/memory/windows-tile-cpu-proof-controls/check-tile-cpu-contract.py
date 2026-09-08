"""BotTest limited no-launch contract for CPU functional cells."""
import json
import os
import pathlib
import runpy
import sys
assert os.environ["USERNAME"] == "BotTest"
root = pathlib.Path(os.environ["TILE_CPU_HOST_ROOT"])
sys.path.insert(0, str(root / "docs/memory"))
import run_managed_cell as rmc
contract_id = os.environ["TILE_CPU_CONTRACT_ID"]
role = os.environ["TILE_CPU_BUILD_ROLE"]
mode = os.environ["TILE_CPU_MODE"]
target = os.environ["TILE_CPU_TARGET_CELL_ID"]
assert contract_id == target + "-contractcheck"
assert os.environ["TILE_CPU_CELL_ID"] == contract_id
stage = pathlib.Path(r"C:/ProgramData/274bot-Test/renderer-owner-census-9268890" if role == "baseline" else r"C:/ProgramData/274bot-Test/tile-boxed-fb3589a")
runner = stage / ("run-" + contract_id + ".py")
checks = []
def no_launch(argv):
    spec = rmc.validate_spec(rmc.load_spec(pathlib.Path(argv[0])))
    args = rmc.parse_diagnostic_argv(spec["diagnostic_argv"])
    rmc.require_argv_consistent_with_spec(spec, args)
    assert isinstance(args, __import__("argparse").Namespace)
    assert args.cpu_fallback and args.nav_captures and args.failure_capture and args.no_diagnostics
    assert not args.gpu_completion_profile
    assert rmc.rd.requested_backend(args) == "cpu_fallback"
    assert spec["requested_backend"] == "cpu_fallback" and spec["cpu_fallback"] is True
    checks.append({"id": target, "contract_cell_id": contract_id, "checked": True, "launched": False, "client_started": False, "cpu_intent": True, "gpu_completion": False})
    return 0
rmc.main = no_launch
runpy.run_path(str(runner), run_name="__main__")
out = pathlib.Path.home() / "274bot-runs" / ("tile-cpu-contract-" + contract_id + ".json")
out.write_text(json.dumps({"kind":"no-launch CPU functional contract", "cell_id":target, "contract_id":contract_id, "checks":checks, "performance_acceptance":False, "functional_only":True}, indent=2))
print(out.read_text())
