# Thiever workload prerequisites — in progress

Working host checkout: codex/memory-diagnostics, based on T4 71b8e48. Client candidate: codex/memory-sprite-reuse, based on 4f2048e. No optimization accepted or merged.

## Findings

The previous n32 diagnostic did not consume its seeded food: zero Eat requests were captured. The lower-level Inventory item Eat operation already maps to Rust, but selectedLoadout always returned null and foodOf rejected nonempty loadouts. The benchmark seeded lobsters without a usable host loadout. Its low-HP waiting logs therefore did not prove that inventory food was exhausted.

The banking prerequisite also needs complete withdrawTo sequencing and close/count confirmation, booth approach/open mapping, and propagation of walking radius. All changes here are functional prerequisites and must be present on both sides of future memory comparisons.

Server evidence: content/scripts/skill_thieving/configs/pickpocking/pickpocket.dbrow gives Guard chance endpoints 50/240, level requirement40, stun8 and damage2. engine/src/engine/script/handlers/PlayerOps.ts STAT_RANDOM computes floor(low*(99-level)/98)+floor(high*(level-1)/98)+1 against uniform integers0..255. At effective level50 this is146/256 success probability. Pure consecutive random failure probability for k eligible attempts is (110/256)^k. The 150-tick host proof window is not an attempt count. Server preconditions reject combat/stun and delay attempts; a dead NPC also yields no reward.

## Live diagnostics

All files under diagnostics are preserved; none are accepted baselines. The first sustained setup seeds two carried lobsters and2000 banked lobsters once, selects a disposable host loadout, enables Auto banking, target10 and threshold3. This deliberately exercises banking before first XP. The unchanged150-tick XP proof has failed during the bank trip; no timeout has been lengthened. Later end-to-end qualification should also exercise default target22/Withdraw-X and sustained eating/restocking.

20260905T124759Z: exact booth tile routing stalled; no attempts.
20260905T125219Z and125558Z: reached bank exterior; bank remained closed. The primitive open_nearest_booth only walks when more than one tile away, so a single call is insufficient.
20260905T125910Z and130305Z: second radius approach failed/stalled. Explicit radius handoff and approach tile classification are under test.
20260905T130551Z: debug trace preserved. Routing uses packed transports around the bank; final approach made no progress. New candidate selection uses standable floor tiles instead of rejecting all wall-face tiles.

The125558Z run used the previously successful diagnostic binary after a subsequent paint-diagnostic compilation failed. Its binary SHA is authoritative; its source-diff hash includes that unbuilt edit. It is not build-matched evidence. The launcher now records full host/client source digests including untracked source files as well as binary SHA.

## Panel candidate

Native allocation snapshots in20260905T111908Z_panel_n1_idle show the sprite_models allocation growing87031808→174063616 bytes. Core dynamic sprites appended every frame while deleted entries remained in both core and render arenas. The new test demonstrates1001 slots after1000 frames versus2 needed (one static, one dynamic). The candidate recycles only completed dynamic indices, clears their model/stamp slots, tracks the last inserted index, and clears reuse state on map reset. Static indices stay unrecycled. Unit/regression rendering tests pass; live visual and measured validation remain required.

## Verification and review

Script lib37, gold_stubs9, load_isolate144, focused approach test, memory harness16 passed at intermediate revisions. Client lib64 plus world26, render_backend5, overlays and draw passed. The existing guardian-paint test was corrected to assert the existing double first-tick paint; baseline source already calls both wrapper and forwarding paints. Rendering cadence was not changed.

Whole-branch grok-4.6 review requested through Hermes; output and model usage are recorded alongside this report. Review is not yet approval. Further changes require re-review.

20260905T131014Z completed withdrawal from2 to10 and close/count confirmation, then failed the original XP deadline during the return trip. Final inventory contains ten lobsters. Setup revised to four carried lobsters so the initial XP proof occurs before the restock cycle; target22 now exercises Withdraw-X. Diagnostics pass explicit fixture loadouts without reading operator loadouts.

20260905T131423Z uses4 initial food, target22. At240s it had passed the XP proof, eaten twice, completed one bank trip and resumed pickpocketing with21 lobsters. Withdraw-X is present in the captured request stream. This live build predates the subsequent race fix; it demonstrates functional mapping, not final acceptance.

Grok-4.6 review returned NOT APPROVED. High findings: stale worker publication, destructive preemptive route clearing, unconditional bank approach while adjacent. Addressing with one worker per slot, bounded latest pending request, generation/token ownership, publication only after a path exists, and an adjacent guard. Explicit fixture loadouts already avoid operator disk reads. The requested withdrawal timeout extension is declined: the original catalog helper itself stops on a2500ms confirmation timeout, and changing that would change behavior. nextWithdrawChunk remains outside the called Thiever path (Thiever imports withdrawTo); its stub is preserved for the later compatibility campaign. Old last_sprite_index already identified a removed hole after teardown; preserving that observation avoids an unrelated behavior change.

20260905T132433Z (latest route ownership fixes) completed exit0 after30s warmup,300s observation and60s teardown. All observation samples show ready1/active1, no script errors; final active paint reports30 steals,3 eats,20 food and1 bank trip. Observation XP gain: 1358. Stop yields active0. Expanded client sprite reset/double-teardown/independence regression also passes. This is functional qualification, not an accepted memory comparison. The same binary now runs32 slots with120s warmup and600s observation in20260905T133145Z.

Final retry correction: the second grok-4.6 pass confirmed the earlier high findings fixed, but found that requested_route remained latched after NoPath or thread-spawn failure. A subsequent identical walk-near could be suppressed by an unrelated retained route. The new test calls ScriptWalkArm twice against unreachable approach tiles and verifies two actual search generations while retaining the old route; it failed before the fix (generation1 instead of2) and passes after. Both failure paths now clear the request latch; stale publications still cannot clear newer requests. All129 host-play lib tests pass. A third whole-branch grok-4.6 review is pending. The ongoing32-slot run predates this final latch correction, so it cannot validate that correction.

Qualification summaries are generated by summarize_diagnostic.py. They report completion, actual ready/active counts, each slot's XP gain, observed food/bank counters and errors, plus unavailable memory fields. Process exit0 alone is not acceptance. Missing V8, snapshot-queue and GPU metrics, latency evidence, matched baseline/candidate repeats and visual checks remain outstanding. The panel candidate has regression evidence only; no live savings claim is made.

Final whole-branch grok-4.6 code review completed and approved, with no remaining actionable code defects. Review text: grok-4.6-prerequisite-final-review.txt; usage file confirms model grok-4.6 via xai-oauth, completed=true, failed=false. This approves code review only; live runs still predate the final retry-latch fix and no memory/scale acceptance is granted.

20260905T133145Z completed exit0:120s warmup,600s observation,60s teardown. All582 observation samples show ready32/active32. Each of32 slots gained XP, ate and completed a bank trip; no script errors. Median observed resident memory 3812909056 bytes; final resident 3539959808 bytes with active0. Per-slot counters and XP are in qualification-summary.json. This establishes the sustained workload on the recorded binary; it predates the final retry-latch fix, and simultaneous code-review/development activity plus missing metrics make it unsuitable as a clean memory baseline. No result has been merged.

Both panel and TUI release builds with memory-profile pass after the final reviewed fix. Their new binary hashes are in latest-build.json. These rebuilt binaries have not yet had a live run. Remaining next steps: validate this final build live; complete missing memory/latency instrumentation; isolate and measure the panel sprite candidate with visual/CPU checks; then establish repeatable matched baselines before resuming independent memory optimizations.

## Final reviewed build: 32-slot rerun, 2026-09-05

20260905T141815Z_tui_n32_active completed exit0 using the exact final reviewed TUI binary SHA3e1c05bc1cf2cccbe8a7bf39ce6dc6f1bdfafae274634c70283ba53f270124a0. The joint panel/TUI memory-profile build reproduces that binary. Host/client source and binary hashes were unchanged through the run (provenance-check.json). No builds, source edits or additional benchmark processes ran during observation.

All32 initial XP proofs passed. After120s warmup, all582 observation samples over the600s observation phase report ready32/active32. Every bot gained2340–3931 XP during observation, ate7–10 times in the run, completed1 bank trip and ended with13–16 food. No script errors or diagnostic failures were reported. After Stop, active0 persisted through the60s retention window; clients remained ready32. Process exited0.

Observed resident memory: median3981213696 bytes (3.71GiB), peak4011048960 bytes (3.74GiB), final after script Stop3651420160 bytes (3.40GiB). These are separate observations, not an optimization savings claim. Final Rust live requested bytes3070906405. Raw samples, per-slot qualification-summary.json and metadata are preserved in the run directory.

Earlier attempt20260905T141650Z was deliberately terminated before observation because its TUI-only build had a different binary hash. Its artifacts and operator-note.txt are preserved; it is an aborted setup, not a passing or failing scale result.

This closes the outstanding live qualification of the final retry-latch fix at32 slots. It is not a complete accepted memory baseline: V8, queued-snapshot and GPU measurements remain unavailable, latency is not instrumented, and matched repeats/frontends/scales and panel visual checks are outstanding. Next campaign work is completing the missing measurements before accepting optimization comparisons. Code was unchanged, so the existing final grok-4.6 code approval still applies.

## Instrumentation added; newest 32-slot run failed

See instrumentation.md for metric definitions, tests, review and evidence. V8 heap/cached-sample coverage, queued+decoding snapshot bytes/capacity, explicit client GPU buffer/texture payloads, client/script tick and UI construction/full-frame CPU timers now report real measurements. The TUI launcher now supplies a real pseudo-terminal (earlier redirected runs skipped UI rendering). Whole-branch grok-4.6 approved this instrumentation; affected host/script/frontend/client tests and joint release builds pass.

Requested run20260905T151355Z exited1 after359.42s when one bot timed out reaching its bank. Only91.37s of observation were completed, despite full32-bot readiness/activity and V8 coverage in all90 recorded observation samples. No completed teardown or passing baseline is claimed. Preserve the failure; investigate the host route outcome/dispatch from(2659,3311,0) to bank(2656,3286,0) before another baseline. Memory instrumentation itself is populated and no code changed during measurement.

## Update: bounded stun recovery and completed TUI32 run

See [stun-recovery.md](stun-recovery.md) for implementation, grok-4.6 approval, tests and the successful 20260905T165834Z actual-TUI32 diagnostic. All 32 banked and gained XP through the full observation period; teardown cleared V8 and queued snapshots. No stun recovery event occurred in this run, so that specific live proof remains outstanding. This supersedes the latest-run failure status above, not the requirement for repeated baselines.

## Panel one-renderer follow-up

See [panel-single-renderer.md](panel-single-renderer.md). The panel32 run verified one drawing/31 sim clients but failed on bot12 returning from the bank. Preserve as failed diagnostic evidence; the earlier TUI32 pass remains valid as one diagnostic cell. Built-in F12 screenshots were saved and inspected. A controlled return-route scenario with embedded screenshot checkpoints is the next proposed diagnostic.

## Automated navigation captures: reproduced return-door oscillation

See [nav-capture-diagnostic.md](nav-capture-diagnostic.md). The opt-in panel32 diagnostic saved all five native-snapshot/PNG checkpoints and reproduced a failed return route. Successful sends alternate between the door approach and the far-side destination until the hop expires; the final movement reaches the leg target after route teardown. This provides a focused regression target before another stability run. The same run naturally verified pre-send stun deferral and subsequent arrival on another bot. Host/client/binary provenance matches launch. Grok-4.6 approved the whole dirty branch; affected tests and joint release build passed. The run failed with only82.93s of observation and concurrent review, so it is not an accepted memory baseline.

## Door approach corrected; captured panel32 passed

See [door-approach-fix.md](door-approach-fix.md). The retained closer recovery now preserves forward movement through an open door and does not reopen behind a directional crossing. Red/green regressions, live one-tick closer, full 32-bot panel observation and Stop retention check passed. Grok-4.6 approved. All bots banked and gained XP. This is diagnostic stability evidence, not an accepted memory baseline. The operator agreed to address measurement overhead and measured waste at 1/32 before returning to 128; see [CPU follow-up](cpu-follow-up.md).
