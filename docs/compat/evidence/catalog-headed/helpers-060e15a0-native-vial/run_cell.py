import datetime
import hashlib
import json
import os
import pathlib
import re
import subprocess
import sys
import time

root = pathlib.Path(__file__).resolve().parents[2]
revision, case, source, catalog = sys.argv[1:]
assert revision in ("274", "289")
assert case in ("bone_burier", "chicken_killer", "thiever", "alcher", "bank_fletcher", "alcher_custom", "alcher_custom_alias", "alcher_custom_name", "alcher_ordered", "alcher_large_batch", "bank_fletcher_string", "bank_fletcher_cut_string", 'dart_fletcher', 'dart_fletcher_iron', 'herb_cleaner', 'herb_cleaner_named', 'gem_cutter', 'gem_cutter_named', 'door_opener', 'door_opener_gate', 'gnome_course', 'gnome_course_radius', 'flax_picker', 'superheater', 'superheater_steel', 'superheater_fire_battlestaff', 'chicken_killer_bank', 'vial_filler', 'vial_filler_east', 'potion_maker', 'potion_maker_named', 'tanner_bot', 'tanner_bot_hard', 'rune_crafter', 'rune_crafter_earth', 'mule_crafter')
assert catalog in (
    "100adccc037d9f6898080e1cad58fcfc43364775",
    "8e7d965be2071d6ec65c3265e12af797082d720a",
)
evidence = root / "docs/compat/evidence/catalog-headed"
identity = json.loads((evidence / f"binary-{source}.json").read_text())
if not identity.get('isolated_build', False):
    raise SystemExit('BLOCKED: rebuild this diagnostic binary in an isolated Cargo target before new LIVE cells')
binary = pathlib.Path(os.environ.get("CATALOG_EXEC_PATH", identity["binary"]))
assert hashlib.sha256(binary.read_bytes()).hexdigest() == identity["binary_sha256"]
out = evidence / f"r{revision}-{case.replace('_', '-')}-{catalog[:8]}-{source}"
run_label = os.environ.get("CATALOG_RUN_LABEL", "")
if run_label:
    assert re.fullmatch(r"[a-z0-9-]+", run_label)
    out = out.with_name(out.name + "-" + run_label)
assert not out.with_suffix(".log").exists(), "Prior live cell must be preserved"
engine = pathlib.Path("/Users/acfrazier/experiments") / (
    "Server/engine" if revision == "274" else "lostcity-289/engine"
)
nav = root / ".superpowers/world-capabilities" / revision / "274bot.navpack"
command = [
    str(binary), "--live", f"script_{case}", "--profile", f"local-{revision}",
    "--revision", revision, "--host", "127.0.0.1", "--port",
    "43594" if revision == "274" else "44594", "--asset-host", "127.0.0.1",
    "--http-port", "80" if revision == "274" else "1080", "--engine", str(engine),
    "--nav-pack", str(nav), "--nav-flags", str(nav.with_suffix(".navflags")),
    "--catalog", str(root / ".superpowers/inputs" / f"rs2b0t-{catalog}"),
]
env = os.environ.copy()
renderer = os.environ.get("CATALOG_RENDERER", "gpu")
assert renderer in ("cpu", "gpu")
if renderer == "cpu":
    env["BOT_CPU"] = "1"
else:
    env.pop("BOT_CPU", None)
env.pop("BOT_LIVE", None)
env.update(BUDGET_S="180", BOT_DEBUG="1", RUST_BACKTRACE="1")
env["274BOT_SMOKE_DIR"] = str(out / "shots")
started = datetime.datetime.now(datetime.timezone.utc).isoformat()
before = time.monotonic()
runtime_errors = []
with out.with_suffix(".log").open("w") as log, out.with_suffix(".timeline.jsonl").open("w") as timeline:
    process = subprocess.Popen(command, cwd=root, env=env, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT, text=True, bufsize=1)
    receipt = dict(command=command, pid=process.pid, started_at=started,
                   runner_script_sha256=hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest())
    out.with_suffix(".process.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps(receipt), flush=True)
    for line in process.stdout:
        timeline.write(json.dumps(dict(elapsed_seconds=round(time.monotonic()-before, 6), line=line.rstrip())) + "\n")
        timeline.flush()
        if (line.startswith("[script ") and (re.search(r"\] tick [0-9]+:", line) or "not impl" in line)) or "panicked at" in line or line.startswith("FAIL:"):
            runtime_errors.append(line.rstrip())
        log.write(line)
        log.flush()
        print(line, end="", flush=True)
    code = process.wait()
receipt.update(
    finished_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
    elapsed_seconds=round(time.monotonic() - before, 3), process_exit_code=code,
    exit_code=code or (1 if runtime_errors else 0), runtime_errors=runtime_errors,
    host_commit=identity["host_commit"], client_commit=identity["client_commit"],
    binary_sha256=identity["binary_sha256"], catalog_commit=catalog,
    environment={k: env[k] for k in ("BUDGET_S", "BOT_DEBUG", "RUST_BACKTRACE", "274BOT_SMOKE_DIR")},
    engine_commit=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=engine, text=True).strip(),
    nav_sha256=hashlib.sha256(nav.read_bytes()).hexdigest(),
    log_sha256=hashlib.sha256(out.with_suffix(".log").read_bytes()).hexdigest(),
    timeline_sha256=hashlib.sha256(out.with_suffix(".timeline.jsonl").read_bytes()).hexdigest(),
    requested_renderer=renderer,
    effective_bot_cpu=env.get("BOT_CPU"),
    scope="Headed production panel diagnostic; acceptance also requires observed core loop, supported branch behavior and no unresolved runtime errors.",
    elapsed_is_not_performance_measurement=True,
)
out.with_suffix(".json").write_text(json.dumps(receipt, indent=2) + "\n")
print(json.dumps(receipt), flush=True)
if runtime_errors:
    print("FAIL: headed catalog runtime errors: " + json.dumps(runtime_errors), flush=True)
sys.exit(receipt["exit_code"])
