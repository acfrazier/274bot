# TUI origin acknowledgment functional smoke

The corrected TUI origin routing has a qualified live functional smoke across three focus changes. All four exercised ordinals have usable input windows; no input boundary-pending or overflow classification appears. This is a partial-focus, perturbed functional check, not full-fleet latency or performance acceptance.

## Matched build

Reviewed producer correction `f946901` (task `t_3ec8a043`, 92 TUI tests independently passed) was applied to the duplicate-nav control as `7dc3392`. Frozen manifest: `diagnostics/matched-instrumented-build-20260907T101400Z/build-manifest.json`. Existing freeze helper built locked release `memory-profile-no-alloc`, System allocator, both panel and TUI on isolated target directories. Both builds exited 0, pre/post host source digests and client source/commit were stable, all four manifest entries independently validated. Client commit remains `451759f2a7df9c57895657d5b8d506172860cee1` in these binaries; later appearance-table edits do not change frozen binaries.

| Binary | SHA256 |
|---|---|
| Reference TUI | `810239843e5921ddb41be1c898cf8bdfd22e3d33430e596706cc34517fbf70b0` |
| Candidate TUI | `eaaa45a718ca340f808e3467c469ff804e6875d7425fe41edfc715a698dd53a1` |
| Reference panel | `ebc308203837d2abecd9a0bd5cf8ea02f02938af8d9e279f4f6c29f8cb6fe292` |
| Candidate panel | `1843cdac638f78dc92a3d6cbb4a07564080f16d91930c765f5e9c1f8578382b4` |

The common TUI hooks now retain input origin until successful draw; profile-on overhead can change because no-origin draw is a no-op. Earlier binary/profile overhead measurements are not a numeric calibration for this new build.

## Smoke evidence

Batch `diagnostics/tui-origin-ack-smoke-20260907T101615Z`, raw run `20260907T101631Z_tui_n16_active`. Real N16 TUI, active sustain, 30s warmup/75s observe, normal teardown, real PTY probes, scheduling/render/responsiveness/fine ON and GPU-completion OFF. Host conditions explicitly allow campaign review/implementation/test activity and unrelated OS work; no clean CPU/RSS/latency conclusion. One attempt completed exit0; independent `smoke-bound.json` has binding_ok/qualified=true, native qualification available, pair_eligible=false. All 16 slots had positive steal gain (minimum 3).

The initial N2 spec in `tui-origin-ack-smoke-20260907T101547Z` failed preflight: launcher permits N1/16/32/128 only. No frontend launched. Its failure and spec remain intact; the explicit new batch corrected N to 16 without extending launcher capabilities.

| Exercised ordinal | Raw ordinary-observe starts/completes | Selected contained input samples | Fine p99 upper bound ms |
|---:|---:|---:|---:|
| 3 | 7/7 | 6 | 1 |
| 4 | 28/28 | 27 | 1 |
| 5 | 28/28 | 27 | 41 |
| 6 | 7/7 | 6 | 1 |

The 74 ordinary observation samples cover 70 starts and 70 completions. All four exercised slots end observe with pending/canceled/lost/dropped 0. The contained reader uses 66 samples after its deterministic edge exclusions; these counts are not interchangeable with 71 PTY writes. The final teardown sample has 72 starts/completions and all coverage-loss gauges 0, consistent with the probe stream plus the recorded outside-observation overlay-restore key. Twelve ordinals correctly report `no_input_samples`; they are not counted as pass. `raw-input-summary.json` preserves ordinary-observe counters separately from the bound contained windows.

Decode is available for 16 slots in this smoke. Scheduling is available for 15 slots; ordinal 4 is unavailable with `stale_snapshot`. Neither that gap nor the known cadence target miss is waived. The short smoke cannot resolve the prior full-window decode edge limitations or prove the paired 2 ms margin.

## Decision

Keep the reviewed instrumentation correction and its scoped functional evidence. Preserve all old pending/overflow results. No accepted RSS saving, full input-fleet pass, timing non-regression or final campaign acceptance follows. Appearance packet storage implementation is separate; matched clean measurements and remaining Linux/panel/lifecycle/whole-branch review are still required.
