# Control qualification gate

## Purpose

Frontend exit 0 and an "observation complete" log line are not enough to admit a
control into a future acceptance pipeline. `cpu_screen.summarize` already rejects
failed workloads (zero-progress bots, below-scale readiness, wrong instrumentation,
missing boundaries). This gate wraps that check as a CLI with exit 1 on failure
and a stable JSON receipt.

PASS here means **workload qualification only**, not performance or budget
acceptance. Expected allocation-counting and diagnostic-sidecar modes are taken
from explicit CLI flags (default both false), never inferred from sample contents.

## Deliverables

| File | Role |
|---|---|
| `docs/memory/qualify_control.py` | CLI + `qualify()` entry point |
| `docs/memory/test_qualify_control.py` | Stdlib unittest / subprocess CLI tests |
| `docs/memory/qualification-gate-report.md` | This report |

No changes to `cpu_screen.py` criteria, raw diagnostic artifacts, Rust, or
`run_diagnostic.py`.

## CLI

```
python3 docs/memory/qualify_control.py <run_dir> [--counting] [--diagnostics] [-o PATH] [--no-write]
```

- Default modes: `--counting` off, `--diagnostics` off.
- Writes `<run_dir>/control-qualification.json` unless `-o` or `--no-write`.
- Exit 0 only when `qualified` is true; otherwise exit 1.
- Supported workloads: `active`, `seeded-idle`. `idle` / `lifecycle` fail honestly.
- Missing files, malformed JSON, incomplete metadata, and incomplete process status
  yield structured `errors` and exit 1 (no unhandled traceback).

JSON fields include `qualified`, `errors`, `qualification` (full `summarize`
result), `counting`, `diagnostics`, `workload`, and `pass_means`.

## Tests

Command:

```
python3 docs/memory/test_qualify_control.py -v
```

Result (2026-09-06, Python 3.9.6): **16 passed** in ~0.5s.

Synthetic cases: valid active and seeded-idle → exit 0; zero progress, below-scale
sample, wrong instrumentation, missing boundary, incomplete process, missing file,
malformed metadata/samples JSON, unsupported `idle`/`lifecycle` → exit 1. Output
path write and default false modes are checked.

Real-data checks (temp `-o`, no write into run dirs):

| Run | Exit | Outcome |
|---|---:|---|
| `diagnostics/20260906T181830Z_panel_n32_active` | 0 | qualified, empty errors |
| `diagnostics/20260906T182432Z_panel_n32_active` | 1 | `missing active progress: live14f0f_11`; `readiness/activity below requested scale` |

Both real runs had process exit_code 0; only the qualification gate distinguishes them.

## Limitations

- Does not cover bare `idle` or `lifecycle` acceptance criteria.
- Does not judge RSS, CPU, or budget targets.
- Does not modify or re-run diagnostics; real checks require the two named run
  directories already on disk.
- Default instrumentation expectation is system/no-sidecar; callers must pass
  `--counting` / `--diagnostics` when those modes were the intended cell.

## Scope note

No claims about RSS savings or out-of-area movement causes. Orchestrator-owned
reports and concurrent diagnostic observation are out of scope for this commit.
