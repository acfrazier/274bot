# One-cell managed launcher + process accounting

Deliverable for task `t_9ba9bddd`: `run_managed_cell.py`, `test_run_managed_cell.py`,
and this report. Bounded integration only — no edits to launcher, reader,
collector, receipt, Rust, or client. No live bot/game-server measurement on this
worker.

## Invocation

```text
python3 docs/memory/run_managed_cell.py SPEC.json CELLS_ROOT \
  [--accounting-script docs/memory/process_accounting.py]
```

Programmatic:

```python
from run_managed_cell import run_managed_cell
report = run_managed_cell(spec_path, cells_root, accounting_script=...)
```

Exit codes: `0` completed receipt status, `3` preflight_failed, `2` exclusive
cell-dir collision, `1` other failed/incomplete.

## Spec schema (explicit JSON; no discovery)

| Field | Requirement |
| --- | --- |
| `id` | non-empty string; exclusive cell directory name under `cells_root` |
| `index` | int ≥ 1 |
| `kind` | `matched` \| `overhead` \| `diagnostic` |
| `binary`, `build_manifest`, `build_role` | frozen build binding (`reference`\|`candidate`) |
| `frontend` | `tui` \| `panel` |
| `launcher_argv` | exact `Popen` argv list (no shell) |
| `diagnostic_argv` | args for `run_diagnostic.build_parser` + `validate_args`; must include matching `--binary` / `--build-manifest` / `--build-role` / frontend |
| `server_identity_path`, `host_conditions_path` | existing JSON sidecars |
| `nav_pack`, `nav_flags`, `catalog_path` | runtime fixtures for `verify_build` |
| `game_server_pid` | positive int; sampled only, never signaled |
| `ambient_helpers` | `{role: pid}` map; sampled only, never signaled |
| `sampler_interval_s` | finite positive |
| `max_wall_s` | finite positive orchestration safety deadline |
| `observe_s`, `warmup_s` | optional nonnegative; else taken from diagnostic argv |
| `teardown_grace_s` | optional; informational for fixtures |

No auto-discovery by name/env. No retries or replacement cells.

## Orchestration contract

1. Validate spec + diagnostic argv consistency.
2. Exclusive `cells_root/<id>/` with `logs/`, spec copy, cell report.
3. Preflight: `build_provenance.verify_build`, sidecar load, `server_resources.sample_process` on game server and ambient PIDs (finite timeout). Failure → `preflight_failed` cell report, **no** launch receipt, **no** children.
4. `managed_receipt.create_launch` immediately before launcher start (exclusive).
5. Launch `launcher_argv` via subprocess list; drain launcher log incrementally for metadata `run_dir`/`pid` (partial lines retained until newline).
6. Start schema-2 **stop-controlled** `process_accounting` child (`duration` omitted → `duration_s_requested: null`, `duration_mode: stop_controlled`) with roles: `game_server`, `launcher`, `controller=<self>`, ambient helpers; collector auto-self inside the accounting process. Duplicate PIDs rejected.
7. Watch qualification JSONL incrementally when `run_dir` known.
8. After observe end (`warmup_s + observe_s` from metadata/spec) + **≥2** sampler intervals, SIGTERM collector while frontend may still be in teardown. Completion success requires collector `controlled_stop` / exit 0 (not “elapsed only”).
9. Early frontend end → stop collector, preserve incomplete/failed receipt, no retry.
10. `max_wall_s` exceeded → terminate **owned** launcher/collector only; frontend SIGTERM only if same recorded start identity and launcher child; **never** signal server/ambient/caller.
11. Bounded cleanup ≤15s per owned child. Frontend vs launcher exits kept distinct in receipt.
12. `managed_receipt.complete` with actual exits, sampler result, raw hashes. `performance_acceptance` always false.

## Tests

```text
python3 docs/memory/test_run_managed_cell.py -v
```

10/10 passed with real dummy processes + fixture launcher (no bot networking):

- successful observation then delayed teardown + controlled_stop
- early frontend fail (exit 42) / launcher 0, no retry
- sampler start failure recorded, not replaced
- missing metadata + wall path preserves cell/receipt
- deadline cleanup owns launcher/collector only; server+ambient survive
- exclusive cell create
- changed binary / dead server preflight rejection without launch
- diagnostic argv vs spec binary mismatch → preflight_failed
- receipt hash chain + distinct exit fields

## Remaining eligibility gaps (explicit)

- **No live measurement** in this card; root freezes/runs sequential cells after reviews.
- **Reader / stop-controlled consumer**: `sampler.duration_s_requested` is intentionally **null**; current matched evidence reader does not yet consume stop-controlled accounting coverage — independent bracketing/coverage checks remain future work.
- **Cache provenance** (`ef21398` under review): **not integrated** here; native/cache input eligibility for managed cells is **unavailable** until a later root wiring card. Do not treat this runner as cache-bound.
- **Qualification / overhead / performance acceptance**: never asserted; receipts always `performance_acceptance: false`.
- **Host frontend CPU/RSS** remain the frontend’s own raw series; not folded into process_accounting roles.
- **Collector review** concurrent at dispatch time is now approved at `b18ec4d`; this runner targets that schema-2 stop-controlled contract.
- **Native/Rust worker** owns binary builds; this module only verifies predeclared fixtures.
- Multi-cell sequencing, freeze orchestration, and production `run_diagnostic` argv (not fixture launcher) are root responsibilities after review.

## Non-claims

Not a performance pass. Not an overhead proof. Not a qualification gate. Does not
start or stop the game server. Does not discover processes by name.
