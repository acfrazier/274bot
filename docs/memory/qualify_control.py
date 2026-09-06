#!/usr/bin/env python3
"""Qualify a completed active or seeded-idle memory control.

Workload qualification only — not performance or budget acceptance.
Calls cpu_screen.summarize with explicit expected instrumentation modes
(default: allocation counting off, diagnostic sidecar off). Exit 0 only
when qualified; otherwise exit 1 with a structured JSON result.
"""
from __future__ import annotations

import argparse
import json
import pathlib
import sys
import traceback

ROOT = pathlib.Path(__file__).resolve().parent
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

SUPPORTED_WORKLOADS = frozenset({"active", "seeded-idle"})
REQUIRED_FILES = ("metadata.json", "samples.jsonl", "samples.qualification.jsonl")


def _fail(run: pathlib.Path | None, errors: list[str], details=None) -> dict:
    return {
        "run": str(run) if run is not None else None,
        "qualified": False,
        "errors": sorted(set(errors)),
        "qualification": details,
        "pass_means": "workload qualification only; not performance or budget acceptance",
    }


def _load_json(path: pathlib.Path):
    try:
        return json.loads(path.read_text())
    except json.JSONDecodeError as exc:
        raise ValueError(f"malformed JSON: {path.name}: {exc}") from exc
    except OSError as exc:
        raise ValueError(f"unreadable file: {path.name}: {exc}") from exc


def qualify(run: pathlib.Path, counting: bool = False, diagnostics: bool = False) -> dict:
    """Qualify one completed control run directory.

    Returns a stable dict with qualified/errors and underlying summarize details.
    Never raises for missing/malformed/incomplete inputs — those become structured failures.
    """
    run = pathlib.Path(run)
    if not run.is_dir():
        return _fail(run, ["run directory missing or not a directory"])

    missing = [name for name in REQUIRED_FILES if not (run / name).is_file()]
    if missing:
        return _fail(run, [f"missing required file: {name}" for name in missing])

    try:
        meta = _load_json(run / "metadata.json")
    except ValueError as exc:
        return _fail(run, [str(exc)])

    if not isinstance(meta, dict):
        return _fail(run, ["malformed JSON: metadata.json: expected object"])

    workload = meta.get("workload")
    if workload is None:
        # Historical active cells sometimes omitted workload; treat missing as active
        # only when other required keys look like a finished control. Prefer explicit.
        # Task: only active and seeded-idle. Missing workload is incomplete metadata.
        return _fail(run, ["missing workload in metadata"])
    if workload not in SUPPORTED_WORKLOADS:
        return _fail(
            run,
            [
                f"unsupported workload: {workload!r} "
                f"(supported: {', '.join(sorted(SUPPORTED_WORKLOADS))})"
            ],
        )

    # Required scalar inputs for summarize
    for key in ("exit_code", "n", "observe_s"):
        if key not in meta:
            return _fail(run, [f"incomplete metadata: missing {key}"])

    # Validate sample files are parseable before calling summarize (avoid traceback)
    for name in ("samples.jsonl", "samples.qualification.jsonl"):
        path = run / name
        try:
            text = path.read_text()
        except OSError as exc:
            return _fail(run, [f"unreadable file: {name}: {exc}"])
        for i, line in enumerate(text.splitlines(), 1):
            if not line.strip():
                continue
            try:
                json.loads(line)
            except json.JSONDecodeError as exc:
                return _fail(run, [f"malformed JSON: {name}: line {i}: {exc}"])

    try:
        from cpu_screen import summarize

        details = summarize(run, counting, diagnostics)
    except Exception as exc:  # pragma: no cover - defensive; should be rare after prechecks
        return _fail(
            run,
            [f"summarize failed: {type(exc).__name__}: {exc}"],
            details={"traceback": traceback.format_exc()},
        )

    errors = list(details.get("errors") or [])
    qualified = bool(details.get("qualified")) and not errors
    return {
        "run": str(run),
        "qualified": qualified,
        "errors": sorted(set(errors)),
        "qualification": details,
        "counting": counting,
        "diagnostics": diagnostics,
        "workload": workload,
        "pass_means": "workload qualification only; not performance or budget acceptance",
    }


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("run_dir", type=pathlib.Path, help="completed diagnostic run directory")
    p.add_argument(
        "--counting",
        action="store_true",
        default=False,
        help="expect allocation_counting=true (default: false)",
    )
    p.add_argument(
        "--diagnostics",
        action="store_true",
        default=False,
        help="expect diagnostic_sidecar=true (default: false)",
    )
    p.add_argument(
        "--output",
        "-o",
        type=pathlib.Path,
        default=None,
        help="write JSON result here (default: <run_dir>/control-qualification.json)",
    )
    p.add_argument(
        "--no-write",
        action="store_true",
        help="print JSON only; do not write a file",
    )
    args = p.parse_args(argv)

    result = qualify(args.run_dir, counting=args.counting, diagnostics=args.diagnostics)
    text = json.dumps(result, indent=2) + "\n"
    if not args.no_write:
        out = args.output if args.output is not None else pathlib.Path(args.run_dir) / "control-qualification.json"
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(text)
    sys.stdout.write(text)
    return 0 if result.get("qualified") else 1


if __name__ == "__main__":
    sys.exit(main())
