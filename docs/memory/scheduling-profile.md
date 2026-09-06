# Current-build scheduling profile

Operator approved a bounded current-build32-bot profile to distinguish work overruns from scheduler/wakeup delay after the scalar animation-delay optimization. Commit-diff review uses Hermes profile reviewer (grok-4.5). Whole-branch final review remains a separate campaign requirement.

Opt-in BOT_SCHEDULING_PROFILE=1 (launcher --scheduling-profile) records active20ms loop work, requested sleep, actual sleep, work overruns, and consecutive tick-start intervals. Drawing and simulation cycles are separate. Idle parks and drawing-mode transitions are excluded from interval comparisons. Per-thread batches publish every50 cycles, plus on loop exit; cumulative samples can lag by49 cycles per slot. No per-tick shared counter lock or allocation. Histograms bound excess time at1,2,5,10,20ms, then unbounded. These are bucket counts, not precise percentiles. Sleep excess includes scheduler delay and timing-call overhead; it is not proof of a specific kernel cause.

Disabled mode emits scheduling:null and retains the existing cadence decision. Enabled mode retains the same sleep budget calculation and sleep call; clock reads/local accumulation/batch publishing add diagnostic overhead. Measure with System allocator, diagnostic sidecar off, one renderer/31 simulations, fixed sustained Thiever fixture. Warmup120s, observe180s, teardown60s. One five-second macOS sample during observation is labeled and excluded from unsampled CPU attribution; the cell is diagnostic evidence, not repeated performance acceptance. Capture no screenshots during this profile.

Tests: histogram boundaries and synthetic work/sleep/interval accounting, excluding parks/focus transitions; host120 and host-play131 pass. Review/build/live pending.
