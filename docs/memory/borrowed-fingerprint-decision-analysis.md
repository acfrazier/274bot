# Borrowed-fingerprint candidate decision analysis

Captured: 2026-09-07 UTC
Status: offline decision analysis only; no live run, rebuild, fixture change, or Rust change was performed.

## Decision

Park the borrowed-fingerprint candidate as a live-screen candidate. Do not count it as a qualified performance candidate, do not use either diagnostic run for RSS/CPU/latency acceptance, and do not relabel the original failed cell. Keep the reviewed source and its functional/oracle/allocation-retention test evidence available; this is not a recommendation to revert or reject the implementation. The evidence is insufficient to grant provisional performance retention because the candidate has no qualified 16-slot observation and the one authorized discriminator did not establish equivalent active progress.

The next independent owner must not treat this candidate as a qualified performance baseline. Independent ownership design can continue under the existing plan, with its eventual control/candidate qualification still required. A bounded alternative for the unresolved original exception is an offline diagnostic-design task, followed only by a separately approved failure-only run if the plan owner elects to reopen live diagnosis. That task should preserve the fixed candidate binary/fixture and capture the missing failure-time per-slot runtime/exception/operation evidence; it must not weaken qualification, alter the script or fixture, or turn a diagnostic into a performance cell.

## Evidence classification

### Original corrected-screen failure: real functional failure, cause unresolved

The corrected native screen's candidate N=16 cell launched once and failed during qualification at 17.386014 seconds with the exact error `live256c5_12: tick 19: Unknown error` ([docs/memory/borrowed-fingerprint-failure-diagnosis.md:8-19,31-57]; corrected-screen report [docs/memory/diagnostics/borrowed-fingerprint-corrected-native-screen-report.md:31-45]). The receipt records launcher exit 1, `frontend_failed_or_incomplete`, `launcher_failed`, an unexpected qualification phase, and no observation boundary. The failure-boundary snapshot has mixed `Idle`/`Running` slots, so it is not merely a missing report row or a late qualification-reader decision.

The failing slot was nevertheless `Running`, `ingame: true`, `scene_state: 2`, at `(2661, 3306, 0)`, with only two dispatched ticks and `last_completed_tick: 20`; its terminal paint still said `ThievingBot — starting` ([docs/memory/borrowed-fingerprint-failure-diagnosis.md:45-57]). Thus the candidate process had reached the game scene, but the cell failed before common active progress and before observation. This establishes a real candidate-cell functional failure, not a performance result. It does not establish that the borrowed comparison caused it: the preserved error lacks the underlying exception, host operation, snapshot field mask, or encode event ([docs/memory/borrowed-fingerprint-failure-diagnosis.md:107-115]).

The reference N=16 cell in the same corrected batch qualified with all 16 slots ready/active, 117 native observation samples, `ingame=true`, and `scene_state=2` ([docs/memory/diagnostics/borrowed-fingerprint-corrected-native-screen-report.md:17-29]). That supports the server/fixture/protocol as viable for this declaration, but it does not exclude startup timing sensitivity or prove candidate causality.

### Authorized debug discriminator: different failure at the qualification layer

The single separately authorized debug run used the same candidate binary (`10ddc915...9af008`), N=16 active TUI declaration, 30/120-second timing, frozen server identity, candidate manifest, cache identity, and `--failure-capture`, changing only the diagnostic switch to `--debug` ([docs/memory/borrowed-fingerprint-debug-report.md:15-34]; `diagnostics/borrowed-fingerprint-failure-debug-20260907/01-candidate-n16-debug/cells/01-candidate-n16-debug/receipt.json:1-25,57-71]). The archive integrity records are the root-verified `remote-capture.tar.gz` SHA-256 `9d3c3591383cd68ef50b44a982a2533a410ca125117bf26eb0d75e2246dd1fbf` and run-directory archive SHA-256 `e42511f087d774fd941a4dbee68b5b9b26f46f020f32a9cfa386f77db296ae38` ([docs/memory/borrowed-fingerprint-debug-report.md:63-75]).

That run completed its launcher path with exit code 0, produced observe-start and observe-end qualification snapshots, and did not reproduce `Unknown` ([docs/memory/borrowed-fingerprint-debug-report.md:5-12,36-61]). It still failed workload qualification because `live257e0_9` had zero active progress: `independent-binding.json:194-238` records `qualified: false`, the exact error `missing active progress: live257e0_9`, `observation_s: 118.42264001199999`, and `steal_gains.live257e0_9: 0` while the other slots had nonzero gains. The same record shows `exit_code: 0` at line 241, `observe_sample_n: 116`, and `qualified: false` ([`independent-binding.json:241,256-269`]). The cell report independently records `launched: true`, two qualification lines, launcher exit 0, controlled sampler cleanup, and final status `completed` ([`cell_report.json:1-7,75-83,134-138`]).

This is zero additional **steals** under the existing qualification rule, not evidence of zero gameplay progress. Root checked both complete slot-9 observation-boundary records in `samples.qualification.jsonl` lines 1–2: dispatched ticks advance 173 → 373, last completed tick 190 → 390, bank trips 0 → 1, food 3 → 21, and position (2659, 3296, 0) → (2664, 3308, 0), while steals remain 14 → 14. The slot completed banking/restocking, moved, and continued processing; its error and in-flight fields are null at both boundaries. This contradicts the earlier broad no-gameplay-progress interpretation and supplies a concrete banking-duration confounder for this short observation. The original qualification result remains failed and unchanged; neither the metric nor its gate is relaxed. It is also not a blanket frontend startup failure: the observe-end snapshot has running slots with `ingame: true` and `scene_state: 2` ([docs/memory/borrowed-fingerprint-debug-report.md:44-52]; `samples.qualification.jsonl:1-2`). The retained log shows slot 9 thread-up, handshake begin/ok, mainland teleport/setvar queued, and subsequent slot-level work entries (`run.log:8,28,44,114,222,332,348,419,498,726,747,786,822,840,855,872,889,906,921,941`). The early log at `run.log:840` (`10s | Target: Guard | XP/hr: —`) does not override the later observation-boundary banking and movement evidence. The log has no positive same-slot/same-tick `Unknown`, exception, or interrupted-slow-tick correlation ([docs/memory/borrowed-fingerprint-debug-report.md:53-61]).

Therefore the discriminator separates the original `Unknown` failure from the new qualification failure, but does not identify a common cause. The missing-progress result also cannot be treated as a pass merely because the process and observation window completed.

## Source mechanism and limits of inference

The exact host-only change is confined to `crates/script/src/isolate_fb.rs` and `crates/script/src/slot.rs`; there is no client diff ([docs/memory/borrowed-fingerprint-failure-diagnosis.md:85-94]). The live path is `SlotScript::encode_snapshot_delta` at `crates/script/src/slot.rs:246-259`, which now calls `IsolateBuf::encode_snapshot_delta_updating`. The candidate compares borrowed input with `DeltaMask::changed_from_input` at `crates/script/src/isolate_fb.rs:2166-2256`, encodes, and selectively retains changed fields via `SnapshotFingerprint::retain_from_input` at `crates/script/src/isolate_fb.rs:1702-1869`; the updating entry point is at `crates/script/src/isolate_fb.rs:2299-2318`.

This supplies a concrete plausibility boundary: the changed path is exercised during live snapshot encoding and could be investigated if a future artifact connects it to the failing tick. It is not causal evidence. The unchanged candidate tests cover the local oracle, exact wire bytes, forced banks, pointer retention, and restart behavior ([docs/memory/borrowed-fingerprint-failure-diagnosis.md:96-105]), but constructed-input tests cannot establish multi-isolate startup behavior. `SlotScript::drain_logs` stores the latest `tick ...` line as `last_error` at `crates/script/src/slot.rs:390-406`; that explains why the original artifact exposes only `Unknown error`, not which operation or exception produced it.

## Named confounder and bounded follow-up

The one justified confounder is **16-slot startup-convergence/progress skew for a single slot**. It is supported by the debug run's slot-specific zero steal gain despite `Running`/`ingame`/scene 2, by the observed handshake and teleport progression, and by the absence of a same-slot error correlation. It is not asserted to be a fixture defect, server defect, scheduler defect, or borrowed-fingerprint defect. Timing alone is not enough to choose among those explanations.

If the root owner reopens diagnosis, create one separately approved failure-only task with a fixed binary and fixture that captures, at the failure boundary, the affected slot's last completed/dispatched tick, script/isolate exception text if available, current snapshot encode stage/field mask, and navigation/request state. Require the same qualification and stop rules, and stop after the bounded attempt. If that evidence cannot be added without changing the frozen binary or behavior, leave the candidate parked rather than rerun until lucky.

This decision does not cancel the approved memory campaign or other independent owner work. It only prevents this unqualified borrowed-fingerprint candidate from entering matched performance acceptance or driving another unbounded live retry.

## Acceptance mapping

- Original candidate cell: failed functional proof; preserved unchanged.
- Debug discriminator: completed process path but failed active-workload qualification; not a pass.
- Borrowed-fingerprint performance comparison: unavailable; no candidate observation result.
- Provisional performance retention: not granted; concrete local allocation-retention evidence remains preserved for later review.
- Candidate source disposition: retain/park for diagnosis, not reject on unsupported causality.
- Next independent optimization owner: do not build on this candidate's live result; use a separately approved diagnostic-design task or choose another measured owner.
- Final memory/performance acceptance: unchanged and incomplete under `performance-finish-plan.md:217-229` and `:261-271`.
