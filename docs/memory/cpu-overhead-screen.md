# CPU overhead screening protocol

Operator approved a bounded CPU sidebar and deferring128 until after1/32 investigation. This batch changes measurement only. No animation/cache/application optimization yet.

The optional memory-profile-no-alloc feature selects System directly in both frontend entry points and reports allocator metrics null. Default memory-profile still uses CountingAllocator with existing semantics. Process user/system CPU seconds are additive getrusage fields, separate from wall-duration timers. Two qualification records (observation start/end, before Stop) retain script state, errors, paint/progress and recent runtime data even with verbose sidecar disabled. Existing ready/seed/XP gates, error handling and lifecycle cadence remain unchanged.

Four modes independently vary counting and verbose diagnostic collection: both enabled, counting disabled, diagnostics disabled, both disabled. The panel uses one drawing client and the rest simulation. No nav captures or stack samples during the timed cells. Use immutable saved binaries, identical build inputs and equivalent fresh ephemeral account fixtures (reusing progressed accounts would bias subsequent modes). Fixture is sustained level50 Thiever with four initial lobsters and2000 banked, target22. The runner leaves gameplay randomness intact.

Screening: one pass at each of1 and32;120-second warmup and180-second observation, plus standard60-second teardown. Reverse mode order at32. This screens for large overhead effects; it is not the eventual repeated ten-minute baseline or a significance claim. Keep full repeated validation for a selected application change. Runs are sequential without build/review activity; required grok-4.6 review must complete first.

Qualification requires exit0, nearly full sampled observation duration, all requested slots ready/active, actual reported instrumentation flags/allocator availability, complete boundary records and increased steal counts for each bot. CPU counter availability/monotonicity is checked. Raw action/navigation traces remain available in enabled diagnostic control cells; disabled cells use boundary runtime data and existing functional tests rather than claiming identical stochastic packet traces.

Tests: 131 host tests passed in each feature mode, including the direct CPU counter check; six Python qualification regressions and joint frontend builds passed. Whole-branch grok-4.6 approved (receipt confirms model and completion). Runs pending. The driver stops on failed or unqualified cells; unavailable evidence never passes.

## Completed screening results

All eight cells completed exit0 and passed qualification. Every observation sample had the requested ready/active counts, and every bot increased its steal count (one-bot gains9–24; fleet gains9–31 over180 seconds). Source and saved-binary provenance match across the matrix. Raw runs and results.json are in diagnostics/cpu-overhead-screen-20260905.

| Bots | Counting | Diagnostics | CPU cores | Client ticks/bot/s | CPU ms/client tick | Median RSS GiB |
|---:|:---:|:---:|---:|---:|---:|---:|
| 1 | on | on | 0.308 | 39.15 | 7.875 | 0.603 |
| 1 | on | off | 0.324 | 39.00 | 8.299 | 0.613 |
| 1 | off | on | 0.321 | 38.88 | 8.261 | 0.603 |
| 1 | off | off | 0.313 | 38.75 | 8.080 | 0.603 |
| 32 | on | on | 2.799 | 38.57 | 2.268 | 3.785 |
| 32 | on | off | 2.970 | 41.73 | 2.225 | 3.786 |
| 32 | off | on | 1.603 | 37.84 | 1.324 | 3.795 |
| 32 | off | off | 1.458 | 38.82 | 1.173 | 3.822 |

CPU cores are process CPU seconds divided by elapsed observation seconds. CPU ms/client tick divides whole-process CPU by client tick count; it is a work-normalized comparison, not a scoped client-tick timer.

At32, counting-on uses2.80–2.97 cores versus1.46–1.60 without counting. The work-normalized differences also remain large. At1,0.308–0.324 cores does not establish a meaningful effect in this single pass. Sidecar effects are not consistent across counting modes; do not assign a precise overhead to it. No application optimization occurred, and the runtime stays instrumented in other ways (V8/queue/GPU/duration metrics, fixture hooks and qualification). System mode is a better CPU control, not proof of completely uninstrumented production cost.

Decision: use the direct-System binary for CPU/latency comparisons, with diagnostics held fixed. Collect allocation counts in separately matched memory runs, and do not combine those throughput numbers with System-mode CPU measurements. A repeated paired screen is needed before quoting a precise overhead percentage.128 remains deferred.

## Separate uncounted profile

Run diagnostics/20260905T210413Z_panel_n32_active used the saved System binary, diagnostics off,120-second warmup and60-second observation. A five-second macOS sample at1ms began at elapsed264.929s with ready32/active32. Sampling succeeded; the process completed exit0 and all slots passed progress qualification. Its CPU average is excluded from the eight-cell comparison because it was sampled.

The new profile contains4123 mutex-wait samples, of which2967 are attributed to AnimFrame::get. Of that lookup's wait samples,2920 came through ClientEntity::entity_anim and47 through model animation. CountingAllocator is absent from the active stacks. The earlier counted profile had72205 wait samples at AnimFrame::get and11296 in the diagnostic client_frame path; the uncounted profile has11 in that diagnostic path. These are differently timed sample populations, not CPU percentages or a measured percentage reduction. They support the conclusion that the instrumentation amplified shared-lock contention and should not be used to rank production hotspots quantitatively.

Active stacks now prominently include tile_model_stamp, snapshot rebuilding and ordinary allocation/free work. One renderer is still part of this configuration. The animation lookup remains a useful bounded target, but broad store/renderer redesign is not justified by this screen.

## Selected next application experiment

SeqType::get_delay falls back to AnimFrame::get only to read transform.delay. That currently clones the complete frame, its vectors and its AnimBase while holding the process-wide store mutex. A crate-internal scalar delay query could return the same value under the existing lock without cloning. Keep public AnimFrame::get returning its independent owned clone; keep lazy unpack, grow-only init and archive replacement behavior. This avoids introducing shared ownership into callers that do not need a frame at all. Shared immutable frames for model rendering remain a later candidate if measurements justify them.

Before changing code, add cases comparing scalar results to the current get(id).map(delay) behavior: explicit sequence delays, zero-delay fallback, a stored zero delay, missing/invalid frame ids, absent frame/delay arrays, and multi-client init/unpack publication. Verify that previously returned owned frames remain independent after mutation or later publication. Run affected client integration tests separately. Measure the candidate with the same System/no-diagnostics1/32 cells, then repeat matched controls/candidates before accepting a saving. No application or animation code was changed in this screening batch.

Measurement implementation: c4943a7, whole-branch grok-4.6 APPROVE. Both feature configurations passed131 host tests; six Python qualification tests passed; panel/TUI release builds passed. All8 screening runs and the separate profile run completed with successful workload qualification. Sources and saved binaries remained fixed throughout.
