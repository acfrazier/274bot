# Current-build scheduling profile

Operator approved a bounded current-build32-bot profile to distinguish work overruns from scheduler/wakeup delay after the scalar animation-delay optimization. Commit-diff review uses Hermes profile reviewer (grok-4.5). Whole-branch final review remains a separate campaign requirement.

Opt-in BOT_SCHEDULING_PROFILE=1 (launcher --scheduling-profile) records active20ms loop work, requested sleep, actual sleep, work overruns, and consecutive tick-start intervals. Drawing and simulation cycles are separate. Idle parks and drawing-mode transitions are excluded from interval comparisons. Per-thread batches publish every50 cycles, plus on loop exit; cumulative samples can lag by49 cycles per slot. No per-tick shared counter lock or allocation. Histograms bound excess time at1,2,5,10,20ms, then unbounded. These are bucket counts, not precise percentiles. Sleep excess includes scheduler delay and timing-call overhead; it is not proof of a specific kernel cause.

Disabled mode emits scheduling:null and retains the existing cadence decision. Enabled mode retains the same sleep budget calculation and sleep call; clock reads/local accumulation/batch publishing add diagnostic overhead. Measure with System allocator, diagnostic sidecar off, one renderer/31 simulations, fixed sustained Thiever fixture. Warmup120s, observe180s, teardown60s. One five-second macOS sample during observation is labeled and excluded from unsampled CPU attribution; the cell is diagnostic evidence, not repeated performance acceptance. Capture no screenshots during this profile.

Tests: histogram boundaries and synthetic work/sleep/interval accounting, excluding parks/focus transitions; host120 and host-play131 pass. Commit8a8e667 approved by Hermes reviewer/grok-4.5; release build passed. Live result below.


## Partial live evidence — failed qualification

Run diagnostics/20260906T001553Z_panel_n32_active exited1: live37f10_8 requested script stop at tick438. Observation lasted57.4s instead of180; no observe-end qualification or normal teardown proof. Do not accept this as a baseline or successful32-bot test. Sidecar was disabled, so the stop's underlying script/nav cause is unavailable. Operator separately observed the focused bot approach an unreachable door unrelated to its path, reliably after banking; do not conflate that observation with bot8's unidentified stop.

Five-second macOS stack sample succeeded at elapsed311.17s. The initial watcher encountered a partially written JSONL row before sampling; it was restarted to read only newline-terminated records and then sampled successfully. Excluding elapsed301.17–336.17 leaves36.21s of unsampled observation for tentative timing attribution. These are partial diagnostic figures, not acceptance results:

| Group | Work ms/cycle | Requested sleep ms | Actual sleep ms | Start interval ms | Work overruns |
|:---|---:|---:|---:|---:|---:|
|31 simulation slots|0.564|19.436|22.997|23.562|0|
|1 drawing slot|5.719|14.281|17.030|22.750|0|

Mean sleep excess3.561ms(sim) and2.749ms(drawing) accounts for most interval excess in this short window. That supports late wakeups as a current cadence limiter, not expensive simulation work overrunning20ms. It does not establish the cause of the earlier control/candidate throughput difference or justify changing scheduling policy yet.

Stack top-of-stack counts prominently include tile_model_stamp1079, GameSnapshot::rebuild_family360, and ordinary allocation/free work. They are sampled counts, not CPU percentages. tile_model_stamp is a scalar lookup through nested world storage, not a whole-world hash; profile its callers/locality before proposing a representation change. Queue/mutex waits and sleep stacks must be distinguished from active CPU work.

A separate diagnostic-only32-bot bank-return trace follows with sidecars, debug and captures enabled and scheduling profiling disabled. A launch with warmup0 was rejected immediately by the positive-duration validator; the corrected trace uses warmup1/observe300. No accepted performance result is drawn from that run. Door stability is the immediate follow-up before repeating this profile.


## Bank-return trace follow-up

Diagnostic run20260906T002421Z_panel_n32_active exited0, qualified all32 bots and completed Stop (active/V8/inflight all0). It reproduced focused bot live3a670_0's bank-return detour. The return route selected Door1530 at(2656,3292) with west destination(2651,3292), while the requested anchor was(2661,3306), radius2. The bot walked around the building to the edge destination, then resumed the remaining route and thieving. Successful arrival does not validate the chosen crossing.

Captured return-route snapshot at tick211 includes the live collision grid and locations. Reduced evidence is bank-return-door-evidence.json: tiles2652–2655 at z3292 have flag0x4120 (including blocked-object bit0x100); objects include counter612, plant1158 and bench1106. Door1530 is at2656,3292. The generator's door_far_side scans indefinitely in a direction until standable returns true, ignoring intervening blocked cells. This constructs a five-tile pseudo-crossing through unrelated obstacles and matches the observed detour. The defect is in edge construction; the earlier approach/recovery fix did not address it.

Next bounded correction: regress this blocked corridor and valid adjacent crossings, then ensure ordinary door edges cannot skip intervening obstacles. Re-evaluate pack/cache invalidation so regenerated routing data is actually used. The helper is also called by web generation; preserve or explicitly validate those different footprints rather than silently changing them. Verify both bank return and the one-tick door-closer scenario before repeating the scheduling profile. No door-edge code has changed in this profiling batch.
