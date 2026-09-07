# Managed cell evidence wiring

`run_managed_cell` now emits the cache and process identity fields that
`managed_resource_binding.bind` expects on a completed receipt. Scope is the
runner only: cache capture, role identities, and sampler module path/SHA
bindings. Collector, receipt writer, cache module, and adapter remain
unchanged owners.

## Spec: optional paired cache paths

- `cache_dir` and `unpack_root` are optional path strings.
- Both omitted → no cache proof (existing receipts stay compatible).
- One alone → `CellError` at validate time.
- Before `create_launch`, the runner calls `cp.capture` and writes exclusive
  `cell_dir/cache-provenance.json`, then passes that path into
  `mr.create_launch(..., cache_provenance_path=...)`.
- Production (non-`_test_launcher`) asserts
  `Path(unpack_root).resolve() == expected_unpack_root()` where
  `expected_unpack_root` mirrors client `bot_target::unpack_dir`
  (`$HOME/.274bot/unpack`, or `cwd/.274bot/unpack` when `HOME` is empty).
- `_test_launcher` fixtures may use explicit temp unpack roots.
- No cache hashing inside the observation window; complete revalidates the
  full snapshot via the existing receipt writer.

## Role identities

`sampler_result['role_identities'] = {role: {pid, start_identity}}` with:

| Role | Source |
|------|--------|
| `game_server` | preflight sampled identity (matches server sidecar) |
| ambient helpers | preflight samples for each declared ambient role |
| `controller` | sampled with the selected backend immediately before launch |
| `launcher` | sampled immediately after launcher `Popen` |
| `collector` | sampled immediately after collector `Popen` |

Also emitted:

- `sampler_result.launcher_pid` / `collector_pid` (actual owned children)
- launch `sampler.roles` remains the **prelaunch** map only
  (`game_server`, `controller`, ambient) — no launcher/collector there

Missing or unsampleable identities become runner_errors; PIDs are never
invented or replaced. Child exit codes stay actual.

## Sampler modules

`sampler_config.modules` binds exact path + SHA256:

- always: `process_accounting.py`, `server_resources.py`
- when `process_backend == 'libproc'`: also `native_process_sample.py`

`module` / `module_sha256` still name the actual accounting script (default
production script, or a private injected test path under the
`process_accounting.py` key). Backend and duration semantics are unchanged.
All module bytes are rechecked before completion; a change yields
`runner_errors` such as `sampler module changed since launch: …`.

## Emitted fields (happy path with cache)

Launch (`launch.json` → receipt `sampler`):

```text
sampler.roles                  # prelaunch PID map
sampler.process_backend
sampler.interval_s / duration_mode / duration_s_requested
sampler.module / module_sha256
sampler.modules.<name>.{path,sha256}
cache_provenance_path / cache_provenance_sha256
cache_content_identity_sha256
cache_verified_before_launch_utc
```

Completion (`receipt.sampler_result`):

```text
exit_code, output, completion, status
duration_mode, duration_s_requested
role_identities.<role>.{pid,start_identity}
launcher_pid, collector_pid
```

Plus receipt-side `cache_verified_after_completion_utc` when cache was bound.

## Validation

```text
python3 docs/memory/test_run_managed_cell.py -v
→ 26 OK (incl. 2 darwin libproc cases)
```

New coverage:

- default no-cache compatibility
- paired cache fields required together
- complete cache proof pre/post
- cache bytes changed during dummy observation → failed receipt
  (`cache_provenance_changed_or_invalid`)
- role identities agree with first collector samples and server sidecar
- sampler modules bound; injected accounting path under
  `process_accounting.py`; module mutation fails runner
- libproc modules include `native_process_sample.py` and backend identity

Real dummy processes + temp cache only. No live bot/server launch, no build.
No overhead or performance-acceptance labels from series existence.

## Relative-path correction after first live pipeline cell

The N1 diagnostic at `diagnostics/20260907T061512Z_tui_n1_active` completed
with frontend/launcher exit0 and qualified native workload/settings/ordinals, but
the collector exited1 because its relative output path was resolved under its
different working directory. The failed receipt and all original artifacts are
preserved under `diagnostics/managed-pipeline-qualification-20260907T061239Z`.
This run is not continuous helper evidence or accepted performance.

The runner now resolves spec path, cells root, accounting script and optional
cwd at entry. An added real dummy-process regression using relative cells root
and accounting-script arguments completes and binds the intended absolute output.
The production CLI invocation needs no workaround. A new explicitly recorded
validation cell is required after review; the failed cell is never overwritten.
