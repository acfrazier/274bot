# Navigation stability diagnostic with checkpoint captures

Operator authorized implementation and running the diagnostic concurrently with Grok review. This is scenario stability evidence, not an accepted memory or latency baseline. Preserve current navigation behavior; collect evidence before another fix.

New opt-in --nav-captures requires panel --single-renderer. Default memory runs and normal application behavior remain unchanged. The launcher puts built-in GPU captures under the run's captures directory via the existing 274BOT_SMOKE_DIR setting. There is no dependency on macOS screen-capture permission.

Navigation callbacks now report leg start/done/failure, transport kind/expected id/from/to, current player position, closed-loc candidate identity/tile, open interpretation, approach/troll state, waited/budget counters, transport interaction target/option/result, and ordinary/cheap-door/troll walk results (the already-open walk at initial hop arm is not yet reported). Existing diagnostic rings retain at most32 strings of512 characters per slot; the1Hz sidecar persists those windows. Diagnostic callbacks do not select or retry actions.

The host captures full native GameSnapshot only at up to three milestones for watched bot zero (first scene-ready while the script runs, bank interface arrival, first route after bank closes), first native navigation failure on any bot, and terminal script failure on the named bot. Ordinary observations retain only a small tick/tile/scene/drawing view. Pending checkpoint queue is capped at five; failure checkpoints take priority. Terminal native outcome includes route generation. Capturing a route after bank close is a workload-specific return-start checkpoint, not a general semantic classifier.

The panel consumes checkpoints, selects the affected bot, keeps one head, waits two seconds plus drawing/ingame/scene2, and uses the existing ShotState GPU readback/write pipeline. JSON pairs the exact event snapshot/tick with the latest native state/tick at capture request and an explicit timing caveat: asynchronously rendered pixels are not asserted to be from either exact tick. Checkpoints do not replay actions or suspend navigation. Focus returns to bot zero afterward. Capture switching and snapshot serialization change diagnostic overhead.

Exit drains matching checkpoint writes (manual screenshots cannot acknowledge a diagnostic checkpoint). Capture failure/timeout makes the diagnostic fail. Drain and per-shot waits are bounded; the original script error is retained when present. A failed script's clients keep simulating only while the bounded evidence drain finishes. Multiple simultaneous failures are not promised individual images: first native failure and named terminal failure are selected.

Tests: existing navigation suite plus observed transport state/actual target/result with unchanged send counts; host bounded priority queue and retained native snapshot; panel matching-write acknowledgement and bounded terminal drain. Existing frontend regression suites remain applicable. No memory improvement, exact pixel-tick alignment, or navigation correction is claimed by this instrumentation.

## Completed diagnostic and review

Run `diagnostics/20260905T183630Z_panel_n32_active` exited 1 after approximately 353 seconds of sampling. All 32 slots passed initial XP proof and banked at least once. The partial observation contains 83 samples spanning 82.93 seconds, each ready32/active32 with all32 live V8 isolates sampled. This does not satisfy the requested600-second observation. No completed teardown or accepted memory/latency baseline: Grok ran concurrently and diagnostic captures changed focus/overhead.

All five automated PNG/JSON pairs were saved and visually inspected: scene-ready, native nav failure, bank arrival, return-route start, and script failure. There was no capture timeout. `capture-manifest.json` indexes them; `qualification-summary.json` contains per-slot and separate memory measurements. `provenance-check.json` confirms host source, client source and panel binary hashes still match launch.

### Return-door failure

Bot `live14e77_1`, return generation3, used Door1530 (open1531), from(2656,3292,0) to(2651,3292,0). During open-door troll recovery the successful walk commands repeatedly alternated between approach(2655,3293) and far-side(2651,3292), with the player alternating between(2653,3294) and(2655,3293). At tick327 the hop exhausted its60-tick budget and FollowEnd reported Stalled/Expired. The last far-side movement then completed: the capture-request native state at tick330 is already(2651,3292), but the remaining route was cleared. The script eventually stopped at tick490 after its return timeout. `failed-slot-events.json` preserves the chronological deduplicated events.

The source explains this oscillation: `troll_door` re-approaches whenever distance from edge.at exceeds one, before considering the open-door continuation. Thus moving toward a farther destination can trigger a reversing approach command. `poll_transport` invokes this before arrival settlement. The failed snapshot contains open leaf1531 at(2657,3292), action Close. No stun event occurred on this bot. This is strong evidence for countermanded movement, rather than a missing open door or an insufficient general retry count.

Next regression: simulate this open-door leg with a far-side target several tiles beyond the threshold and movement progressing across successive snapshots; demonstrate the current reversing approach sends, then require continued forward progress and route completion while preserving closed-door interaction and genuinely needed approach behavior. Design the correction around crossing/movement state, with explicit coverage of approach direction, plane and arrival. No navigation behavior correction was made in this instrumentation batch.

### Stun proof and capture limits

Bot `live14e77_31` logged StunDeferred observed_tick223/resume_tick234, StunResumed tick234, then its first walk at234 and generation1 Arrived at245. This naturally exercises pre-send deferral using the existing route, with one original FindStart. `stun-slot-events.json` preserves the evidence. The already-sent-walk recovery branch still has unit-test evidence only.

`TransportState.live_loc` searches the closed edge id; None does not establish that the open leaf is absent. The bank-arrival JSON records the bank-open event, while its later PNG shows the interface already closing and inventory replenished. Capture-request tick is not asserted to be the source pixel tick. The scene-ready PNG also shows an apparent stale auto-login queue overlay while the selected client is ingame; this remains a separate UI lead.

Host130 and nav249 tests passed; panel374 passed before the final capture-timeout failure correction, then both affected capture tests passed afterward. Joint panel/TUI release memory-profile build passed. Whole-branch grok-4.6 returned APPROVE in `grok-4.6-nav-capture-review.txt`; its usage receipt confirms completed=true/failed=false/model=grok-4.6. Approval covers code review, not live qualification or baseline acceptance. Nonblocking review notes include missing initial already-open walk reporting, checkpoint serialization under the capture lock, and seed errors without a named terminal checkpoint.


## Follow-up: door-closer A/B check

The existing live nav_door scenario passed with recovery enabled and failed Stalled/Expired with both escalation paths temporarily disabled. The source was restored exactly. See [the A/B evidence](diagnostics/20260905-nav-door-fallback-ab/README.md). Simple fallback removal is rejected; the next change should isolate reversing approach behavior while preserving the demonstrated reopening capability.
