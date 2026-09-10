#!/usr/bin/env python3
"""Run a frozen list of native Cargo stages, preserving real exits and fresh logs.

Campaign-only validation helper. Does not install tools, launch live workloads,
alter Git, or weaken a failing test. Stage JSON supplies argv/cwd explicitly.
"""
import argparse
import datetime
import hashlib
import json
import os
import pathlib
import shutil
import subprocess
import time


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("spec", type=pathlib.Path)
    args = parser.parse_args()
    spec = json.loads(args.spec.read_text(encoding="utf-8-sig"))
    out = pathlib.Path(spec["output"])
    out.mkdir(parents=True, exist_ok=False)
    env = os.environ.copy()
    env.update(spec.get("env", {}))
    for name in ("LIVE", "BOT_MEMORY_N", "BOT_DEBUG"):
        env.pop(name, None)
    (out / "spec.json").write_text(json.dumps(spec, indent=2))
    results = []
    for stage in spec["stages"]:
        start = time.monotonic()
        result = dict(label=stage["label"], argv=stage["argv"], cwd=stage["cwd"],
                      started_utc=datetime.datetime.now(datetime.timezone.utc).isoformat())
        log = out / (stage["label"] + ".log")
        with log.open("wb") as stream:
            try:
                child = subprocess.Popen(stage["argv"], cwd=stage["cwd"], env=env,
                                         stdout=stream, stderr=subprocess.STDOUT)
                result["pid"] = child.pid
                result["exit_code"] = child.wait()
            except OSError as error:
                result["launch_error"] = str(error)
                result["exit_code"] = None
        result["elapsed_s"] = time.monotonic() - start
        result["log_sha256"] = hashlib.sha256(log.read_bytes()).hexdigest()
        if result["exit_code"] == 0 and stage.get("artifacts"):
            artifacts = out / stage["label"]
            artifacts.mkdir()
            result["artifacts"] = []
            for name in stage["artifacts"]:
                path = pathlib.Path(name)
                copied = artifacts / path.name
                shutil.copy2(path, copied)
                result["artifacts"].append({"path": str(copied), "bytes": copied.stat().st_size,
                    "sha256": hashlib.sha256(copied.read_bytes()).hexdigest()})
        results.append(result)
        (out / "results.json").write_text(json.dumps(results, indent=2))
        print(json.dumps(result), flush=True)
        if result["exit_code"] != 0 and stage.get("stop_on_failure", True):
            break
    complete = len(results) == len(spec["stages"])
    passed = complete and all(row["exit_code"] == 0 for row in results)
    (out / "completion.json").write_text(json.dumps({"complete": complete, "passed": passed,
        "host_sha": spec["host_sha"], "client_sha": spec["client_sha"]}, indent=2))
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
