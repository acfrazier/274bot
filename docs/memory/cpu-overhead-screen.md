# CPU overhead screening protocol

Operator approved a bounded CPU sidebar and deferring128 until after1/32 investigation. This batch changes measurement only. No animation/cache/application optimization yet.

The optional memory-profile-no-alloc feature selects System directly in both frontend entry points and reports allocator metrics null. Default memory-profile still uses CountingAllocator with existing semantics. Process user/system CPU seconds are additive getrusage fields, separate from wall-duration timers. Two qualification records (observation start/end, before Stop) retain script state, errors, paint/progress and recent runtime data even with verbose sidecar disabled. Existing ready/seed/XP gates, error handling and lifecycle cadence remain unchanged.

Four modes independently vary counting and verbose diagnostic collection: both enabled, counting disabled, diagnostics disabled, both disabled. The panel uses one drawing client and the rest simulation. No nav captures or stack samples during the timed cells. Use immutable saved binaries, identical build inputs and equivalent fresh ephemeral account fixtures (reusing progressed accounts would bias subsequent modes). Fixture is sustained level50 Thiever with four initial lobsters and2000 banked, target22. The runner leaves gameplay randomness intact.

Screening: one pass at each of1 and32;120-second warmup and180-second observation, plus standard60-second teardown. Reverse mode order at32. This screens for large overhead effects; it is not the eventual repeated ten-minute baseline or a significance claim. Keep full repeated validation for a selected application change. Runs are sequential without build/review activity; required grok-4.6 review must complete first.

Qualification requires exit0, nearly full sampled observation duration, all requested slots ready/active, actual reported instrumentation flags/allocator availability, complete boundary records and increased steal counts for each bot. CPU counter availability/monotonicity is checked. Raw action/navigation traces remain available in enabled diagnostic control cells; disabled cells use boundary runtime data and existing functional tests rather than claiming identical stochastic packet traces.

Tests: 131 host tests passed in each feature mode, including the direct CPU counter check; six Python qualification regressions and joint frontend builds passed. Whole-branch grok-4.6 approved (receipt confirms model and completion). Runs pending. The driver stops on failed or unqualified cells; unavailable evidence never passes.
