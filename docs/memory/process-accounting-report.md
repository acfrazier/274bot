# Continuous explicit-process resource accounting

Deliverable for multi-role diagnostic collection: `process_accounting.py`,
`test_process_accounting.py`, and this report. Bounded collector only — no
launcher integration, no live game-server attach in this task, and **no
performance acceptance claim**.

Schema **2** corrects root findings on 67919d6 (timing, brackets, child CPU,
exclusive create, grid coverage). Managed runners must integrate against this
contract.

## Schema contract (JSONL, schema=2)

Exclusive-create streaming JSONL (**no overwrite / no `--force`**). Rows:

### 1. metadata

| Field | Notes |
| --- | --- |
| `schema` | **2** |
| `roles` | `name → {pid}` |
| `interval_s` | cadence |
| `duration_s` | null when stop-controlled |
| `duration_mode` | `fixed` \| `stop_controlled` |
| `required_grid_sample_count` | count of `k` with `k*interval < duration` when fixed; **null** when stop-controlled |
| `sample_timeout_s` | always finite positive (default **5.0** if omitted) |
| `cadence_tolerance_s` | same policy as single-PID sampler |
| `started_utc`, `clock` | wall + monotonic notes |
| `scope` | measured / not_measured / identity / double-count / child_rss_policy |
| `exclusive_create` | true |
| `overwrite_permitted` | **false** |
| `bracketing` | documents tick + **per-role** start/end fields and CPU interval basis |
| `children_cpu` / `children_rss` | rusage children CPU; current RSS unavailable |
| `no_discovery`, `no_foreign_signaling` | true |
| `sampling_overhead` | unmeasured note |

### 2. sample (0..N)

Tick-level:

- `utc`, `monotonic_s`, `elapsed_s`, `scheduled_monotonic_s`
- **tick brackets:** `acquisition_start_monotonic_s`, `acquisition_end_monotonic_s`,
  `acquisition_duration_s`, `lateness_s`, `sample_index`
- `roles` map:
  - **ok:** `pid`, `start_identity`, `resident_bytes` (current), cumulative + delta
    CPU, `cores_delta`, `cpu.interval_basis=per_role_acquisition_start`,
    **per-role** `acquisition_start_monotonic_s` /
    `acquisition_end_monotonic_s` / `acquisition_duration_s`, process provenance
  - **error:** `status=error`, `reason`, per-role brackets when available
- `host_pressure` (separate; exception → run fail)
- `waited_children_cpu` **and** legacy alias `ps_child_cost` (same object):
  - when available: `status=available`, `delta_*_s`, `cumulative_*_s` from
    `resource.getrusage(RUSAGE_CHILDREN)` **per sweep** (waited/reaped children only)
  - `current_rss_bytes=null`, `current_rss_status=unavailable` — **never** emit
    macOS/`ru_maxrss` high-water as current RSS
  - separate from collector self and role PID series

Per-role sample timeout is **recomputed** immediately before each role using
`min(remaining_duration, sample_timeout_s)` on fixed runs, or plain
`sample_timeout_s` on stop-controlled runs — never one stale remaining reused
across N roles.

### 3. summary

| Field | Notes |
| --- | --- |
| `status` | `ok` \| `incomplete` \| `fail` \| **`closed`** (stop-controlled requested stop) |
| `completion` | `required_grid_complete` \| `partial_orchestrator_stop` \| **`controlled_stop`** \| `failed_or_incomplete` |
| `sample_count` | rows written including failed ticks |
| `ok_sample_count` | ticks with all roles ok |
| `required_grid_sample_count` | echo metadata; null if stop-controlled |
| `grid_complete` | true only when fixed + ok + ok_sample_count ≥ required |
| `observed_covered_span_s` | common intersection max(first_ok)→min(last_ok); unavailable if any role lacks samples; **not** configured duration |
| `configured_duration_s` | may be null |
| `stop_reason` / `fail_reason` | as applicable |
| `role_lifetimes` | per role: first/last ok, **`observed_span_s`** (honest), `run_elapsed_s`, `complete_for_required_grid`, `complete_for_configured_duration`, **`full_duration_claim`** (duration claims require actual observed span >= configured duration; always false in stop-controlled) |
| `bracketing` | run monotonic start/end + per-role first/last |
| `reader_note` | controlled_stop is **not** full observation coverage |
| `role_errors`, `scope_gaps`, `sampling_overhead` | as before |

Exit codes:

- **0:** fixed-duration `ok` + full required grid, **or** stop-controlled
  `closed` / `controlled_stop` on requested stop
- **1:** fail, or **fixed-duration** early orchestrator stop (`incomplete`), or
  hang-prevention reject paths

On mid-run hard errors before a summary can be written, an **error** row may
appear (`status=fail`, partial count). Partial JSONL is retained.

### Completion semantics (managed runner)

| Mode | Requested stop | Success condition |
| --- | --- | --- |
| `fixed` (`duration_s` set) | → `incomplete` / `partial_orchestrator_stop` / exit **1** | all `required_grid_sample_count` ok samples + cadence/identity/counters ok → `required_grid_complete` / exit 0 |
| `stop_controlled` (`duration_s=null`) | → `closed` / `controlled_stop` / exit **0** | no promised end; **no** `required_grid_complete` flag; reader must independently require coverage |

Missing required tick (duration exhausted / clock jump / lateness beyond tolerance)
under fixed mode is **`fail`**, never silent `required_grid_complete`.

## CLI

```text
python3 docs/memory/process_accounting.py ROLE=PID [ROLE=PID ...] OUTPUT.jsonl \
  --interval 1 [--duration SECONDS] [--no-collector-self] \
  [--sample-timeout SECONDS]
```

- **No `--force`.** Exclusive create only; existing path → exit 1, file untouched.
- Roles are explicit `NAME=PID` only. Duplicate names/PIDs rejected. Bool/non-int
  PIDs rejected. No name/argv/env discovery; no secret capture.
- Collector auto-adds `collector=<self pid>` unless `--no-collector-self` or the
  self PID is already listed under another role.
- `--sample-timeout` defaults to **5.0** (finite positive). Invalid/non-positive
  rejected. Fixed runs still clamp per role by **remaining** duration at that
  role's acquisition start.
- Omit `--duration` only with signal handlers or external stop control. If
  handlers cannot install and there is no `stop_event`/duration → **reject**
  (no hang).
- Does not start/stop/signal foreign processes. Does **not** change the existing
  finite `server_resources` single-PID sampler (including its timeout defaults).

## Reused helpers

From `server_resources` (no edits to that module): `sample_process`,
`require_same_identity`, `cpu_delta_seconds`, `sample_pressure`, `open_output`
(always `force=False` from this collector), `validate_pid`, `SampleError`.
Linux/macOS paths and unsupported OS errors come from those helpers.

Additional (this module only): `resource.getrusage(RUSAGE_CHILDREN)` for waited
child CPU; strict sample value validation; required-grid math.

## Measured vs missing costs

| Measured (diagnostic series) | Explicitly not measured / scope gap |
| --- | --- |
| Per-role current RSS | Process tree / live children of any role |
| Per-role cumulative + interval CPU (per-role brackets) | Ambient host PIDs not listed as roles |
| PID + start_identity continuity | Kernel threads |
| Tick + per-role acquisition brackets + cadence | Profile on/off instrumentation perturbation |
| Collector self series (when enabled) | Matched overhead proof / acceptance thresholds |
| Host pressure field (may be unavailable) | — |
| Waited-child **CPU** delta/cumulative via RUSAGE_CHILDREN | Waited-child **current RSS** (unavailable; maxrss high-water not emitted) |

Self-only is **not** a full tree. Overhead remains **unmeasured** until an
approved OFF/ON/ON/OFF protocol is independently validated elsewhere.

## Schema changes vs 67919d6 (schema 1) for managed integration

1. **Removed** collector `--force` / `force=` overwrite path; `overwrite_permitted=false`.
2. **`sample_timeout_s` always finite** (default 5.0); per-role remaining recompute.
3. **Per-role** `acquisition_*` brackets; CPU `interval_basis=per_role_acquisition_start`.
4. **`required_grid_sample_count`**, `ok_sample_count`, `grid_complete`;
   fixed mode cannot claim `required_grid_complete` without full grid; missing tick / late
   clock → `fail`. `observed_span_s` / `observed_covered_span_s` are actual
   sample spans, not configured duration.
5. **`waited_children_cpu`** (and `ps_child_cost` alias) carries RUSAGE_CHILDREN
   CPU; `current_rss_*` unavailable — no fabricated child RSS.
6. Strict PID (bool invalid), finite nonnegative counters, non-empty
   `start_identity`; pressure exceptions fail closed.
7. Stop-controlled requested stop: **`status=closed`**, **`completion=controlled_stop`**,
   **exit 0**, no full-duration claim (orch clarification). Fixed early stop
   remains incomplete/exit 1.
8. Hang refuse when duration=None and no stop_event and signals not installable.
9. Metadata/summary **`bracketing`** block for root/managed runner wiring.
10. `schema` bumped to **2**.

## Tests

```text
python3 docs/memory/test_process_accounting.py
```

Production injectables cover prior cases plus root negatives:

- no `--force` on CLI / open_output / run signature
- finite default sample_timeout in stop-controlled mode; invalid timeout reject
- per-role timeout recompute (slow first role shrinks second)
- per-role brackets ordered for sequential roles
- clock jump / missing required tick → fail not full_duration
- child CPU counters present; current RSS not fabricated
- bool PID / bad identity / negative RSS / NaN CPU / pressure exception
- stop-controlled without stop or signals rejected
- fixed early stop incomplete exit 1; stop-controlled stop closed exit 0
- partial JSONL retained on fail
- optional dummy-child smoke (no bot/game-server)

## Limitations

- Not wired into launchers or managed run orchestration (root follow-up uses this
  schema).
- No new acceptance thresholds or overhead protocol.
- macOS start identity still inherits `lstart` one-second resolution limits from
  `server_resources`.
- RUSAGE_CHILDREN only counts **waited** children; still-running children are out
  of scope (explicit gap).
- Signal handlers install only when allowed (main thread); tests use
  `stop_event` / `install_signal_handlers=False`.
- Existing `server_resources` finite sampler behavior and timeout defaults are
  intentionally untouched.

## Verification this card

```text
python3 docs/memory/test_process_accounting.py
```

No performance acceptance. Existing `server_resources` finite sampler untouched.

## Root coverage correction

A complete fixed sampling grid now reports `completion=required_grid_complete`.
It does not by itself set `complete_for_configured_duration` or
`full_duration_claim`: those additionally require actual per-role sample span
to reach the requested duration. For example, 0/1/2s samples for duration 3s
complete the declared grid but cover only 2s. The shared
`observed_covered_span_s` is the intersection of all per-role sample ranges,
not their union. Tests explicitly check both distinctions (36 tests pass).
No process sampling, cadence or exit-code policy changed in this correction.
