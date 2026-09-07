# N16 stop managed diagnostic

Task `t_aa5316e0`. Read-only artifact review of the one orchestrator-managed diagnostic cell. This is a functional diagnosis, not performance acceptance or an N=16 support claim.

## Verdict

| Check | Result |
|---|---|
| Managed run | `docs/memory/diagnostics/20260907T014123Z_tui_n16_active` |
| Process exit | **1** (harness-controlled process failure, not tool SIGTERM) |
| Exact failure | `live121fc_14: script requested stop on tick 529; isolate stopping` |
| Workload qualification | **false**: missing boundary qualification, observation incomplete, process failed/incomplete |
| Both qualification boundaries | **No**: `observe-start` exists; `observe-end` is absent |
| Original script Stop reproduced? | **Yes, same failure class** (new slot/account; exact reason above) |
| Stop reason sidecar capture | **No**: the diagnostics record reports the failure, but the qualification slot has no `stop_reason` field to retain a value |
| N=16 support / performance acceptance | **No / none** |

The original script Stop was reproduced under the same reviewed stop-capture binary and fixture family. This identifies the bounded failure reason but does not fix it, prove a root cause, or authorize a retry. All N16 capacity, performance, and final lifecycle claims remain unresolved.

## Provenance

Launch receipt: `docs/memory/diagnostics/n16-stop-managed-20260907T014123Z/launch.json`.

- Binary: `docs/memory/diagnostics/n16-stop-diagnostic-build-20260907T012907Z/tui-play-n16-stop-diagnostic-system-20260907T012907Z`
- Binary SHA-256: `13a1221c3f0d6f03ca3884894b36d6c995c3c7b3e7bcf70567aa50d8223557aa` (independent `shasum -a 256` matched launch metadata)
- Host commit at launch: `944fb60923b8a2ce533c9ac676396f9bdc929437`
- Client commit at launch / submodule HEAD: `451759f2a7df9c57895657d5b8d506172860cee1`
- Reported host source SHA-256: `9fee9ab6804e283293b2ad2797eff22f170fdd72d8709beb253601123fb587a8`
- Reported client source SHA-256: `27246deba654abac980395b0bbf9ad077d8d3feb66992b4d659512e0630e4a25`
- Reported host diff SHA-256: `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` (clean-tree sentinel)
- Requested mode: TUI, N=16, active, `--sustain`, warmup 30 s, observe 300 s
- Diagnostic sidecar: on; counting allocator, stack logging, debug, scheduling/render/responsiveness profiles: off
- Assets: navpack SHA `2f393138c905aaf1b2db4f77442426db01ff2dfad5ee575d77454012d27a4a30`; navflags and script hashes are retained in `metadata.json`

The source hashes are recorded launch provenance; this review did not rebuild or independently recompute them. The frozen predecessor binaries were not overwritten.

## Exact commands and results

The orchestrator launch command is recorded verbatim in `launch.json` and was:

```text
/Applications/Xcode.app/Contents/Developer/usr/bin/python3 /Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f/docs/memory/run_diagnostic.py tui 16 active --sustain --binary /Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f/docs/memory/diagnostics/n16-stop-diagnostic-build-20260907T012907Z/tui-play-n16-stop-diagnostic-system-20260907T012907Z --warmup 30 --observe 300
```

Read-only review commands:

```text
shasum -a 256 docs/memory/diagnostics/n16-stop-diagnostic-build-20260907T012907Z/tui-play-n16-stop-diagnostic-system-20260907T012907Z
python3 docs/memory/qualify_control.py docs/memory/diagnostics/20260907T014123Z_tui_n16_active --diagnostics --no-write
wc -l docs/memory/diagnostics/20260907T014123Z_tui_n16_active/samples.jsonl docs/memory/diagnostics/20260907T014123Z_tui_n16_active/samples.diagnostics.jsonl docs/memory/diagnostics/20260907T014123Z_tui_n16_active/samples.qualification.jsonl
```

Qualification exited **1** and returned:

```json
{"qualified":false,"errors":["missing boundary qualification","observation incomplete","process failed or incomplete"]}
```

## Boundary, sidecar, and progress evidence

`metadata.json` reports `exit_code: 1`, `started_unix: 1788745283.766059`, `ended_unix: 1788745606.373536`, wall duration about **322.607 s**. Samples contain 314 records: seed 72 (0.055–72.061 s), warmup 29 (73.103–102.078 s), observe 213 (103.126–321.662 s); there is no teardown sample. The only qualification record is `observe-start` at `102.800417542`; no `observe-end` was emitted.

The diagnostic sidecar contains 315 records, including the terminal failure record at `elapsed_s=322.415266709`. Its terminal failure is exactly:

```text
live121fc_14: script requested stop on tick 529; isolate stopping
```

The final diagnostic snapshot still lists all 16 slots as `Running`; only `live121fc_14` has an error. Per-slot progress at that snapshot:

```text
live121fc_0  dispatched=519 last_completed_tick=533 error=null
live121fc_1  dispatched=420 last_completed_tick=440 error=null
live121fc_2  dispatched=509 last_completed_tick=528 error=null
live121fc_3  dispatched=506 last_completed_tick=529 error=null
live121fc_4  dispatched=506 last_completed_tick=529 error=null
live121fc_5  dispatched=506 last_completed_tick=528 error=null
live121fc_6  dispatched=506 last_completed_tick=529 error=null
live121fc_7  dispatched=506 last_completed_tick=529 error=null
live121fc_8  dispatched=420 last_completed_tick=440 error=null
live121fc_9  dispatched=506 last_completed_tick=529 error=null
live121fc_10 dispatched=506 last_completed_tick=529 error=null
live121fc_11 dispatched=509 last_completed_tick=528 error=null
live121fc_12 dispatched=509 last_completed_tick=528 error=null
live121fc_13 dispatched=506 last_completed_tick=529 error=null
live121fc_14 dispatched=506 last_completed_tick=529 error="script requested stop on tick 529; isolate stopping"
live121fc_15 dispatched=509 last_completed_tick=528 error=null
```

At `observe-start`, the qualification sidecar reports all 16 `Running`, errors null, and per-slot steals 6–14 (the raw run samples around the boundary show the same active workload). There is no valid end boundary or complete per-slot acceptance snapshot. `ingame=true` and `scene_state=2` are visible in the terminal diagnostics for the surviving slots, but this does not qualify the process.

`stop_reason`/`stopReason` expectation: diagnostic records expose the failure string, but search of the diagnostic and qualification artifacts found no stop-reason field/value. This is not a captured `null`; the field was not written on the qualification slot. The reviewed capture path therefore remains unproven in this live failure.

## Preserved predecessors and limitations

- Earlier authorized interrupted cell: `docs/memory/diagnostics/20260907T012918Z_tui_n16_active` (exit `-15`, tool-timeout SIGTERM during teardown; not a qualified pass).
- Brief accidental aborted attempt: `docs/memory/diagnostics/20260907T013727Z_tui_n16_active` (not authorized; excluded from claims).
- Original immutable failure: `docs/memory/diagnostics/20260907T005032Z_tui_n16_active` (script Stop, sidecar off).

Do not merge this diagnostic with performance measurements: the sidecar is enabled, the process failed before observe-end/teardown, and no clean comparison exists. No retry or fix was performed or proposed beyond the bounded next action owned by the orchestrator: inspect/map the exact `stopSafely`/ThievingBot call path for `live121fc_14` before any further run. No Rust, fixture, server, or raw artifact was changed by this review.
