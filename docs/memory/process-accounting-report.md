# Continuous explicit-process resource accounting

Deliverable for multi-role diagnostic collection: `process_accounting.py`,
`test_process_accounting.py`, and this report. Bounded collector only — no
launcher integration, no live game-server attach in this task, and **no
performance acceptance claim**.

## Schema contract (JSONL, schema=1)

Exclusive-create streaming JSONL. Rows, in order:

1. **metadata** — `schema`, `roles` map (`name → {pid}`), `interval_s`,
   `duration_s` (null when stop-controlled), `duration_mode`
   (`fixed` | `stop_controlled`), `cadence_tolerance_s`, `started_utc`, clock
   notes, `scope` (measured / not_measured / identity and double-count policy),
   `pressure` note, `sampling_overhead: unmeasured…`, `no_discovery`,
   `no_foreign_signaling`.
2. **sample** (0..N) — wall/`monotonic` timestamps, scheduled vs acquisition
   brackets (`acquisition_*`, `lateness_s`), `sample_index`, per-role block:
   - ok: `pid`, `start_identity`, `resident_bytes` (current, not peak),
     cumulative and delta CPU seconds, `cores_delta`, process provenance;
   - error: `status=error`, `reason` (run fails closed after this tick).
   Host `host_pressure` is separate. `ps_child_cost.status=unaccounted` marks
   short-lived macOS `ps` child CPU/RSS as a scope gap (wall
   `acquisition_duration_s` only).
3. **summary** — `status` (`ok` | `incomplete` | `fail`), `sample_count`,
   `completion` (`full_duration` | `partial_orchestrator_stop` |
   `failed_or_incomplete`), `stop_reason` / `fail_reason`, per-role
   `role_lifetimes` (first/last ok monotonic, observed span, whether complete
   for configured duration — **false** on early stop), `role_errors`,
   `scope_gaps`, `sampling_overhead` unmeasured note.

On mid-run hard errors before a summary can be written, an **error** row may
appear (`status=fail`, partial count). Exit code **0** only for full-duration
`ok`; orchestrator stop and failures exit **1** while retaining partial JSONL.

## CLI

```text
python3 docs/memory/process_accounting.py ROLE=PID [ROLE=PID ...] OUTPUT.jsonl \
  --interval 1 [--duration SECONDS] [--no-collector-self] [--force] \
  [--sample-timeout SECONDS]
```

- Roles are explicit `NAME=PID` only. Duplicate names or duplicate PIDs are
  rejected (no double-count). No name/argv/env discovery; no secret capture.
- Collector auto-adds `collector=<self pid>` unless `--no-collector-self` or the
  self PID is already listed under another role.
- Omit `--duration` only when signal/stop control is intended (SIGINT/SIGTERM
  install orderly stop). Fixed duration remains the normal capture mode.
- Output is exclusive create; `--force` required to replace. Symlink outputs
  refused via reused `server_resources.open_output`.
- Does not start, stop, or signal foreign processes. Does not touch the existing
  finite `server_resources` single-PID sampler behavior.

## Reused helpers

From `server_resources` (no edits to that module): `sample_process`,
`require_same_identity`, `cpu_delta_seconds`, `sample_pressure`, `open_output`,
`validate_pid`, `SampleError`. Linux/macOS paths and unsupported OS errors come
from those helpers; pressure unavailability stays explicit.

## Measured vs missing costs

| Measured (diagnostic series) | Explicitly not measured / scope gap |
| --- | --- |
| Per-role current RSS | Process tree / children of any role |
| Per-role cumulative + interval CPU | Ambient host PIDs not listed as roles |
| PID + start_identity continuity | Kernel threads |
| Acquisition brackets + cadence | Profile on/off instrumentation perturbation |
| Collector self series (when enabled) | Matched overhead proof / acceptance thresholds |
| Host pressure field (may be unavailable) | macOS `ps` child CPU/RSS (marked unaccounted) |

Self-only is **not** treated as a full tree. Overhead remains **unmeasured**
until an approved OFF/ON/ON/OFF protocol is independently validated elsewhere.

## Tests

```text
python3 docs/memory/test_process_accounting.py
```

21 tests, all passing (injected clock/sample functions; optional real tiny
dummy-child smoke only — no bot/game-server launch):

- config: parse roles, duplicate PID/name, malformed specs, collector inject /
  no double-count / wrong collector PID;
- production `run()` injectables: happy multi-role, PID identity reuse, CPU
  counter decrease, missing role + partial retain, cadence miss, orderly early
  stop → `incomplete` not full ok, duplicate PID before write, bad interval,
  existing-output write failure, bounded sample count, collector auto-add;
- CLI malformed/duplicate exit 1; optional local sleep-child smoke with collector.

## Limitations

- Not wired into launchers or managed run orchestration (root follow-up).
- No new acceptance thresholds or overhead protocol.
- macOS start identity still inherits `lstart` one-second resolution limits from
  `server_resources`.
- Cadence uses the same tolerance policy as the single-PID sampler.
- Signal handlers install only when allowed (main thread); tests use
  `stop_event` / `install_signal_handlers=False`.
- Concurrent reader work owns its own files; this collector only creates its
  exclusive JSONL path.

## Verification this card

```text
python3 docs/memory/test_process_accounting.py
# Ran 21 tests … OK
```

No performance acceptance. Existing `server_resources` finite sampler untouched.
