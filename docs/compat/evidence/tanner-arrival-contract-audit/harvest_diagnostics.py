import hashlib
import json
import pathlib

root = pathlib.Path(__file__).resolve().parents[4]
ev = pathlib.Path(__file__).resolve().parent


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


sources = []
for label in ("diagnostic", "diagnostic512"):
    manifest = json.loads((ev / f"source-{label}-78.json").read_text())
    binary = json.loads((ev / f"{label}-binary.json").read_text())
    source = pathlib.Path(manifest["source_root"])
    for name, digest in manifest["files"].items():
        assert sha(source / name) == digest, name
    assert not binary["acceptance_eligible"] and binary["diagnostic_only"]
    assert sha(pathlib.Path(binary["binary"])) == binary["binary_sha256"]
    for check in json.loads((ev / f"{label}-checks.json").read_text()):
        assert check["exit_code"] == 0
        assert sha(root / check["log"]) == check["log_sha256"]
    sources.append(dict(label=label, files_verified=len(manifest["files"]),
                        binary_sha256=binary["binary_sha256"]))

observations = []
for name in ("r274-tanner-arrival-78diag", "r274-tanner-arrival-78diag512",
             "r289-tanner-arrival-78diag512"):
    receipt = json.loads((ev / f"{name}.json").read_text())
    log = ev / f"{name}.log"
    timeline = ev / f"{name}.timeline.jsonl"
    assert sha(log) == receipt["log_sha256"]
    assert sha(timeline) == receipt["timeline_sha256"]
    assert receipt["diagnostic_only"] and not receipt["acceptance_eligible"]
    events = []
    core = None
    for row in map(json.loads, timeline.read_text().splitlines()):
        line = row["line"]
        if line.startswith("[diag-tanner-arrival] "):
            obs = json.loads(line.partition("] ")[2])
            assert obs["observation_only"] and obs["scene_available"]
            assert obs["destination"]["probeable"]
            assert obs["destination"]["walkable"]
            assert obs["destination"]["reachable"]
            if "512" in name:
                assert obs["destination"]["reachable_512"]
            events.append(dict(elapsed_seconds=row["elapsed_seconds"],
                               kind="actual_trade_dispatch", observation=obs))
        elif "interact IfButton" in line or "tanner interface did not open" in line:
            events.append(dict(elapsed_seconds=row["elapsed_seconds"],
                               kind="button_or_timeout", line=line))
        if line.startswith("PASS: catalog_boundary_live: "):
            core = json.loads(line.partition("PASS: catalog_boundary_live: ")[2])["core"]
        elif line.startswith("FAIL: catalog_boundary_live: ") and "; core=" in line:
            core = json.loads(line.split("; core=", 1)[1])["witness"]
    assert core and core["baseline"]["scene_state"] == 2
    observations.append(dict(name=name, exit_code=receipt["exit_code"],
                             elapsed_seconds=receipt["elapsed_seconds"],
                             events=events, cycle=core["tanner_bot_cycle"],
                             latest=core["latest"], log_sha256=sha(log),
                             timeline_sha256=sha(timeline)))

out = dict(diagnostic_only=True, acceptance_eligible=False,
           sources=sources, observations=observations,
           conclusion="All observed Trade destinations are native-reachable, including the bounded512 probe. No host arrival tightening is justified. One diagnostic completes the cycle and two fail; retain all results and original clean failures.")
(ev / "root-diagnostic-summary.json").write_text(json.dumps(out, indent=2) + "\n")
print(json.dumps(dict(sources=sources, outcomes=[(x["name"], x["exit_code"])
                                               for x in observations])))
