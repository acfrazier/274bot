"""Bounded Windows input helper launcher with explicit process accounting.

Source-only controller: it performs no native action on import. ``run`` is
injected with clock, process sampler, and Popen adapters for deterministic tests.
"""
from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import sys
import time
import math
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional, Sequence

HELPER_SHA256 = "04822c0e9f5ade2d555e408cc707722c234bc1e7b442676975a16e7efcaebbd1"
SCHEMA = "native-panel-input-stimulus-run-receipt-v1"


def _sha(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def _fail(reason: str, *, cell_id: str = "", output_path: Optional[Path] = None,
          binding: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:
    target = (binding or {}).get("target", {})
    result = {"schema": SCHEMA, "cellId": cell_id, "outcome": "incomplete",
            "inputCoveragePass": False, "performanceAcceptance": False,
            "incompleteReason": reason, "spawnCount": 0, "samples": [],
            "managedProcesses": [], "samplerOverhead": {"status": "unavailable"},
            "captureEnabledVerified": target.get("captureEnabled") if isinstance(target.get("captureEnabled"), bool) else None,
            "slotZeroFocusVerified": target.get("slotZeroFocus") if isinstance(target.get("slotZeroFocus"), bool) else None}
    if output_path:
        try:
            output_path.parent.mkdir(parents=True, exist_ok=True)
            output_path.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
        except OSError:
            pass
    return result


def _required_binding(manifest: Dict[str, Any]) -> Optional[str]:
    if manifest.get("schema") != "native-panel-input-stimulus-accounting-manifest-v1":
        return "invalid manifest schema"
    for key in ("cellId", "target", "helper", "output", "observeStartPublication", "sourceSha256"):
        if not manifest.get(key):
            return "missing binding: " + key
    target = manifest["target"]
    for key in ("pid", "startUtc", "startIdentity", "sessionId", "binary", "binarySha256", "observedRect", "gameImagePoint", "scene2", "captureEnabled", "slotZeroFocus"):
        if key not in target or target[key] in (None, ""):
            return "missing target binding: " + key
    for key in ("scene2", "captureEnabled", "slotZeroFocus"):
        if target[key] is not True:
            return "target binding is not verified: " + key
    helper = manifest["helper"]
    if helper.get("sha256", "").lower() != HELPER_SHA256:
        return "immutable helper hash mismatch"
    if manifest.get("cadenceMilliseconds") != 1000 or manifest.get("pressMilliseconds") != 80 or manifest.get("durationSeconds") != 120:
        return "stimulus schedule mismatch"
    if manifest.get("triggerWindowSeconds") != [60, 90] or manifest.get("noRetry") is not True:
        return "trigger policy mismatch"
    pub = manifest["observeStartPublication"]
    if not isinstance(pub.get("monotonicSeconds"), (int, float)):
        return "missing observe-start monotonic publication"
    return None


def _sample_record(sampler: Callable[[int], Dict[str, Any]], pid: int, label: str, mono: float) -> Dict[str, Any]:
    try:
        value = sampler(pid)
        return {"process": label, "pid": pid, "monoSeconds": mono,
                "status": "available", "sample": value}
    except Exception as exc:  # sampler errors are evidence, never fabricated zeros
        return {"process": label, "pid": pid, "monoSeconds": mono,
                "status": "unavailable", "reason": str(exc)}


POWERSHELL_LAUNCH_COMMAND = (
    "$s=Get-Content -Raw -LiteralPath $args[0]|ConvertFrom-Json;"
    "$p=@{PanelPid=[int]$s.panelPid;ExpectedStartUtc=[string]$s.expectedStartUtc;"
    "ExpectedBinary=[string]$s.expectedBinary;ExpectedBinarySha256=[string]$s.expectedBinarySha256;"
    "ExpectedUser='BotTest';ExpectedSessionId=[int]$s.expectedSessionId;"
    "ExpectedRect=[int[]]$s.expectedRect;GameImagePoint=[int[]]$s.gameImagePoint;"
    "OutputDirectory=[string]$s.outputDirectory;Label=[string]$s.label;"
    "DurationSeconds=120};"
    "if($s.captureEnabledVerified){$p.CaptureEnabledVerified=$true};"
    "if($s.slotZeroFocusVerified){$p.SlotZeroFocusVerified=$true};"
    "& $s.helper @p"
)


def run(manifest_path: Path, *, clock: Any = time, sampler: Callable[[int], Dict[str, Any]],
        popen: Callable[..., Any] = subprocess.Popen,
        sleeper: Callable[[float], None] = time.sleep,
        powershell: str = "powershell.exe") -> Dict[str, Any]:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8-sig"))
    cell_id = manifest.get("cellId", "")
    envelope_path = None
    if isinstance(manifest.get("output"), dict) and manifest["output"].get("postRunEnvelope"):
        envelope_path = Path(manifest["output"]["postRunEnvelope"])
    invalid = _required_binding(manifest)
    if invalid:
        return _fail(invalid, cell_id=cell_id, output_path=envelope_path, binding=manifest)
    target = manifest["target"]
    helper = manifest["helper"]
    output = manifest["output"]
    helper_path = Path(helper["path"])
    if not helper_path.is_file() or _sha(helper_path).lower() != HELPER_SHA256:
        return _fail("helper file missing or hash mismatch", cell_id=cell_id, output_path=envelope_path, binding=manifest)
    if not Path(target["binary"]).is_file() or _sha(Path(target["binary"])).lower() != target["binarySha256"].lower():
        return _fail("target binary missing or hash mismatch", cell_id=cell_id, output_path=envelope_path, binding=manifest)
    try:
        observed = sampler(int(target["pid"]))
    except Exception as exc:
        return _fail("target identity unavailable before launch: " + str(exc), cell_id=cell_id, output_path=envelope_path, binding=manifest)
    if observed.get("start_identity") != target["startIdentity"]:
        return _fail("target start identity mismatch", cell_id=cell_id, output_path=envelope_path, binding=manifest)
    now = clock.monotonic()
    publication = float(manifest["observeStartPublication"]["monotonicSeconds"])
    delay = now - publication
    if delay < 60:
        sleeper(60 - delay)
        delay = clock.monotonic() - publication
    if delay < 60 or delay > 90:
        return _fail("observe-start trigger window missed; no input spawned", cell_id=cell_id, output_path=envelope_path, binding=manifest)
    out_dir = Path(output["directory"])
    out_dir.mkdir(parents=True, exist_ok=False)
    receipt_path = Path(output["receiptPath"])
    events_path = Path(output["eventsPath"])
    launch_spec = out_dir / "helper-launch-spec.json"
    launch_spec.write_text(json.dumps({
        "helper": str(helper_path), "panelPid": target["pid"],
        "expectedStartUtc": target["startUtc"], "expectedBinary": target["binary"],
        "expectedBinarySha256": target["binarySha256"], "expectedSessionId": target["sessionId"],
        "expectedRect": target["observedRect"], "gameImagePoint": target["gameImagePoint"],
        "outputDirectory": str(out_dir), "label": output["label"],
        "captureEnabledVerified": target["captureEnabled"],
        "slotZeroFocusVerified": target["slotZeroFocus"],
    }, indent=2) + "\n", encoding="utf-8")
    args = [powershell, "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command",
            POWERSHELL_LAUNCH_COMMAND, str(launch_spec)]
    started = clock.monotonic()
    try:
        process = popen(args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, text=True)
    except Exception as exc:
        return _fail("helper launch failed before spawn: " + str(exc), cell_id=cell_id, output_path=envelope_path, binding=manifest)
    spawn_count = 1
    samples: List[Dict[str, Any]] = []
    wrapper_pid = os.getpid()
    sample_interval = float(manifest.get("sampleIntervalSeconds", 1.0))
    max_lifetime = float(manifest.get("maxHelperLifetimeSeconds", 135.0))
    cleanup_budget = float(manifest.get("cleanupBudgetSeconds", 5.0))
    next_sample = started
    timed_out = False
    while process.poll() is None:
        current = clock.monotonic()
        if current - started > max_lifetime:
            timed_out = True
            if hasattr(process, "terminate"):
                process.terminate()
            cleanup_deadline = current + cleanup_budget
            while process.poll() is None and clock.monotonic() <= cleanup_deadline:
                remaining = cleanup_deadline - clock.monotonic()
                if remaining <= 0:
                    break
                sleeper(min(0.1, remaining))
            if process.poll() is None and hasattr(process, "kill"):
                process.kill()
            break
        if current >= next_sample:
            samples.append(_sample_record(sampler, wrapper_pid, "wrapper-sampler", current))
            samples.append(_sample_record(sampler, int(getattr(process, "pid", 0)), "input-helper", current))
            next_sample += sample_interval
        sleeper(min(0.1, max(0.0, next_sample - current)))
    stdout, stderr = process.communicate()
    ended = clock.monotonic()
    samples.append(_sample_record(sampler, wrapper_pid, "wrapper-sampler-final", ended))
    samples.append(_sample_record(sampler, int(getattr(process, "pid", 0)), "input-helper-final", ended))
    managed = []
    seen = set()
    process_summaries = {}
    for index, row in enumerate(samples):
        sample = row.get("sample", {})
        role = "wrapper-sampler" if row["process"].startswith("wrapper-sampler") else "input-helper"
        identity = (row["pid"], sample.get("start_identity"))
        summary = process_summaries.setdefault(role, {"process": role, "pid": row["pid"],
            "start_identity": sample.get("start_identity"), "samples": 0,
            "unavailableSamples": 0, "sampleIndexes": []})
        if row["status"] == "available":
            summary["samples"] += 1
            summary["sampleIndexes"].append(index)
            cpu_parts = [sample.get(key) for key in ("user_s", "system_s")]
            numeric_cpu = [float(value) for value in cpu_parts if isinstance(value, (int, float)) and math.isfinite(float(value))]
            if numeric_cpu:
                summary["cpu_seconds"] = sum(numeric_cpu)
            rss = sample.get("resident_bytes")
            if isinstance(rss, (int, float)) and math.isfinite(float(rss)):
                summary["rss_bytes"] = rss
        else:
            summary["unavailableSamples"] += 1
        if row["status"] == "available" and identity not in seen:
            seen.add(identity)
            managed.append({"pid": row["pid"], "startIdentity": sample.get("start_identity"),
                            "process": row["process"]})
    for summary in process_summaries.values():
        summary.setdefault("cpu_seconds", None)
        summary.setdefault("rss_bytes", None)
        summary["status"] = "available" if summary["samples"] else "unavailable"
        summary["cpuAvailability"] = "available" if summary["cpu_seconds"] is not None else "unavailable"
        summary["rssAvailability"] = "available" if summary["rss_bytes"] is not None else "unavailable"
    for source, destination in ((receipt_path, out_dir / "helper-receipt.json"), (events_path, out_dir / "helper-events.json")):
        if source.is_file() and source.resolve() != destination.resolve():
            shutil.copyfile(str(source), str(destination))
    helper_receipt_valid = False
    helper_receipt_reason = "completed helper receipt missing"
    if receipt_path.is_file():
        try:
            helper_record = json.loads(receipt_path.read_text(encoding="utf-8-sig"))
            expected = {
                "schema": "native-panel-input-stimulus-v1",
                "label": output["label"], "pid": target["pid"],
                "startUtc": target["startUtc"], "binary": target["binary"],
                "expectedBinarySha256": target["binarySha256"],
                "expectedRect": target["observedRect"],
                "gameImagePoint": target["gameImagePoint"],
                "cadenceMilliseconds": 1000, "pressMilliseconds": 80,
                "durationSeconds": 120, "requestedPulses": 120,
                "completedPulses": 120, "outcome": "completed",
            }
            helper_receipt_valid = all(helper_record.get(k) == v for k, v in expected.items())
            if not helper_receipt_valid:
                helper_receipt_reason = "helper receipt binding mismatch"
        except (OSError, TypeError, ValueError) as exc:
            helper_receipt_reason = "malformed helper receipt: " + str(exc)
    files = {}
    for path in (receipt_path, events_path, out_dir / "helper-receipt.json", out_dir / "helper-events.json"):
        if path.is_file():
            files[str(path)] = _sha(path)
    receipt = {"schema": SCHEMA, "cellId": cell_id,
               "helperSha256": HELPER_SHA256, "sourceSha256": manifest["sourceSha256"],
               "cadenceMilliseconds": 1000, "pressMilliseconds": 80, "durationSeconds": 120,
               "startedUnix": time.time(), "triggerDelaySeconds": delay,
               "targetPid": target["pid"], "targetStartUtc": target["startUtc"],
               "targetStartIdentity": target["startIdentity"], "helperPid": getattr(process, "pid", None),
               "helperStartUtc": getattr(process, "start_utc", None),
               "helperStartIdentity": process_summaries.get("input-helper", {}).get("start_identity"),
               "captureEnabledVerified": target["captureEnabled"], "slotZeroFocusVerified": target["slotZeroFocus"],
               "helperPath": str(helper_path), "helperOutputFiles": files,
               "outcome": "completed" if not timed_out and process.returncode == 0 and helper_receipt_valid else "incomplete",
               "incompleteReason": None if not timed_out and process.returncode == 0 and helper_receipt_valid else ("helper lifetime exceeded; cleanup attempted" if timed_out else ("helper failure" if process.returncode != 0 else helper_receipt_reason)),
               "spawnCount": spawn_count, "samples": samples, "managedProcesses": managed,
               "processSummaries": list(process_summaries.values()),
               "sampler": {"label": "root-managed windows_process_sample",
                            "backend": "windows_process_sample.sample_process"},
               "resourceAccounting": {"status": "available" if any(item["samples"] for item in process_summaries.values()) else "unavailable",
                                       "helper": process_summaries.get("input-helper", {"status": "unavailable"}),
                                       "wrapper": process_summaries.get("wrapper-sampler", {"status": "unavailable"}),
                                       "targetIdentity": target["startIdentity"],
                                       "targetExcludedFromManagedTotals": True},
               "samplerOverhead": {"status": "available", "cadenceSeconds": sample_interval,
                                   "sampleCount": len(samples)},
               "helperExitCode": process.returncode, "stderr": None,
               "finalProcessSamples": [row for row in samples if row["process"].endswith("-final")],
               "inputCoveragePass": False, "performanceAcceptance": False}
    Path(output["postRunEnvelope"]).write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    return receipt


def main(argv: Optional[Sequence[str]] = None) -> int:
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("manifest", type=Path)
    args = parser.parse_args(argv)
    result = run(args.manifest, sampler=lambda pid: __import__("windows_process_sample").sample_process(pid))
    print(json.dumps(result, indent=2))
    return 0 if result.get("outcome") == "completed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
