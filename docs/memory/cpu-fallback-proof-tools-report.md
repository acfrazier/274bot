# CPU fallback diagnostic tooling report

Task: Hermes `t_008e604c` on branch `codex/cpu-fallback-proof-tools`.
Scope: Python diagnostic tooling only. No production Rust, native launch, or
application-default changes.

## Problem

`run_diagnostic.py` always scrubbed `BOT_CPU` and never re-set it. CPU fallback
proofs could not be launched through the reviewed diagnostic path without shell
pollution, and nothing recorded requested backend intent so a CPU cell could be
misread as a GPU target or paired against a GPU control.

## Changes

### `run_diagnostic.py`

- New panel-only flag `--cpu-fallback`.
- `validate_args`: reject TUI + `--cpu-fallback`; reject
  `--gpu-completion-profile` with `--cpu-fallback`. Focused-one / nav-captures
  remain legal with CPU.
- `build_child_env`: still scrubs inherited `BOT_CPU`; sets `BOT_CPU=1` only when
  the flag is on.
- Metadata always records `cpu_fallback` (bool) and `requested_backend`
  (`cpu_fallback` | `gpu` | `none` via `requested_backend()`).
- Defaults unchanged: panel without the flag is GPU intent; no env leak.

### `run_managed_cell.py`

- Optional `spec.requested_backend` ∈ {`gpu`,`cpu_fallback`,`none`}.
- `require_argv_consistent_with_spec` requires agreement with diagnostic argv
  when the field is present (CPU argv cannot be labeled GPU).

### `matched_evidence_adapter.py` / `reference_metrics.py`

- `cpu_fallback` and `requested_backend` are match keys so GPU control cannot
  pair with a CPU candidate.
- Legacy metadata without the fields defaults to `cpu_fallback=False` and
  frontend-derived backend (`panel`→`gpu`, else `none`).
- CLI↔metadata validation accepts legacy absence only when the CLI is not
  requesting CPU; explicit `--cpu-fallback` requires recorded metadata.
- GPU cadence / backend rejection paths unchanged (CPU still fails GPU interval
  adapter).

## Tests run

```text
cd docs/memory
python3 -m unittest test_run_diagnostic -v
python3 -m unittest \
  test_run_managed_cell.ManagedCellTests.test_requested_backend_spec_matches_diagnostic_argv \
  test_matched_evidence_adapter.MatchedEvidenceAdapterTests.test_cpu_fallback_requested_backend_match_keys \
  -v
python3 -m unittest test_run_managed_cell test_matched_evidence_adapter -v
```

Results:

- `test_run_diagnostic`: 22 tests OK (includes inherited scrub, explicit set,
  TUI reject, GPU-completion reject, focused-one+nav with CPU).
- Managed + matched adapter suites: OK (requested_backend argv consistency;
  GPU/CPU pair mismatch).
- Full `test_reference_metrics` also exercised: two failures from missing
  historical diagnostic fixtures on this worktree
  (`low-end-reference-screen-…`, `input-interior-conservation-mismatch.json`);
  not caused by this change. Synthetic
  `test_gpu_interval_adapter_rejects_cpu_fallback_backend` passed.

## Limits

- No native panel launch, SSH/VM/Docker, or live CPU visual proof (root owns
  later integration and functional proof).
- Does not authorize CPU performance optimization or change application defaults.
- Does not weaken GPU target/cadence validation.
- Does not edit primary-checkout clean comparison controls or STATE.

## Operator next step

Root integrates reviewed tooling and stages a distinct native functional CPU
fallback proof after clean GPU comparison work; frozen Rust candidate unchanged.
