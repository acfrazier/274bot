# Scalar animation-delay experiment

The operator requested the next experiment selected by the CPU overhead screen. SeqType previously cloned an owned AnimFrame, its transform vectors, and its AnimBase just to read delay. The candidate reads the scalar under the same process-wide mutex. Public owned-frame lookup and rendering callers remain unchanged; archive replacement remains visible on every lookup.

Regression added before implementation: one nonempty frame, 100 delay lookups. All semantic assertions passed on the original implementation, then the allocation assertion failed with700 allocations. With the scalar query it passes with0. Cases cover explicit/zero sequence delays, stored zero, absent arrays, missing/negative/out-of-range IDs and indices, grow-only initialization, replacement publication, and independent owned frame mutation/retention. Targeted integration tests passed: seq_delay1, inject7, zone45; client library65 passed separately.

An initial broad client integration invocation, compiled before the SeqType call-site change, failed four GPU iface_model cases (modal opacity/cube/ship journey overlays). This records existing failures rather than claiming the entire client suite passes. Other integration binaries executed before that failure passed. No renderer code changed.

Protocol: immutable saved control and candidate release binaries using memory-profile-no-alloc (System), diagnostics off, panel with one drawing slot and remaining simulation slots. Fresh sustained level50 Thiever accounts,120-second warmup,180-second observation,60-second teardown. Two matched repetitions at each of1 and32; reverse ordering between repetitions. No concurrent builds, reviews, captures, or stack sampling during measurements. Each cell must qualify ready/active counts and every bot's script progress. This is a bounded CPU experiment, not the eventual repeated ten-minute memory baseline or128-slot acceptance.

Control: diagnostics/cpu-overhead-builds/panel-play-system, SHA2569321b1469784decd5048d4d8cf4c439673c6a2794e88791101407940046bbc35. Candidate: diagnostics/scalar-delay-builds/panel-play-system, SHA2569370cdec9a20b55d71b83aafbf95238c0786de86c07164750d87745c215d58bd. Build manifests retain source digests. Per-run binary-build-provenance.json identifies actual build sources; launch metadata source digests describe the current checkout and must not be mistaken for the old control binary's build sources.

Whole-branch grok-4.6 review APPROVE; completed model receipt retained beside this report. Live comparison completed; qualification and interpretation below. No retained-memory saving accepted. System runs do not measure allocation counts; the regression proves removal of lookup allocations, not an application-wide allocation rate or retained-memory saving.


## Completed paired screen

All eight cells exited0 and qualified. Every requested bot was ready and active throughout observation and increased its steal count. Every final teardown sample reported active0, V8 used0 and snapshot inflight0. No panel-play or tui-play remained running afterward. Raw results and per-run build-provenance receipts are retained in diagnostics/scalar-delay-screen/results.json and the referenced run directories.

| Bots | Variant | Repeat | CPU cores | CPU ms/client tick | Client ticks/bot/s | Mean UI frame ms | Median RSS MiB |
|---:|:---|---:|---:|---:|---:|---:|---:|
| 1 | control | 1 | 0.311 | 7.241 | 42.94 | 0.444 | 645.7 |
| 1 | candidate | 1 | 0.311 | 7.241 | 42.88 | 0.427 | 648.8 |
| 32 | candidate | 1 | 0.878 | 0.654 | 41.93 | 0.913 | 3823.3 |
| 32 | control | 1 | 2.663 | 1.864 | 44.64 | 0.464 | 3842.8 |
| 1 | candidate | 2 | 0.326 | 7.864 | 41.49 | 0.409 | 624.2 |
| 1 | control | 2 | 0.327 | 7.649 | 42.72 | 0.377 | 628.7 |
| 32 | control | 2 | 2.053 | 1.622 | 39.56 | 0.667 | 3937.3 |
| 32 | candidate | 2 | 0.856 | 0.724 | 36.93 | 0.948 | 3884.2 |

At32, candidate CPU is0.856–0.878 cores versus control2.053–2.663. Whole-process CPU normalized by client ticks is0.654–0.724ms versus1.622–1.864ms. This repeats in both run orders and is much larger than the control spread. At1, CPU is effectively unchanged. The unit regression independently proves removal of seven allocations per fallback lookup for its fixture; System live cells cannot establish an application-wide allocation count.

Caveats prevent declaring full acceptance: candidate client ticks/bot/s are about6–7% lower in both32-bot pairs. Mean UI-frame duration rises from0.464/0.667ms to0.913/0.948ms; mean script-tick duration rises from0.105/0.106ms to0.140/0.141ms. Mean client-tick duration falls substantially, but whole-process work normalization and these wall-duration means are different measures. Resolve whether scheduling/workload variation or a meaningful responsiveness regression explains the mixed timing results before declaring no latency regression. Progress checks establish continued work, not identical stochastic action traces.

Fleet median RSS is lower by19.5/53.1MiB in the two pairs, but that is smaller than the93–94MiB control repeat spread. No steady-state memory saving is established. Post-Stop RSS also varies; do not attribute its difference to this change. Keep the scalar candidate isolated on the campaign branch; no merge/push or128-slot acceptance. Additional allocation-mode measurement and latency qualification remain necessary for campaign acceptance.

## Separate presentation and login follow-ups

Operator observed misplaced navigation paint. Source inspection found CPU viewport overlays are included in the full-frame chrome texture, while chrome upload dirtiness only tracks UI redraw causes. The3D scene and minimap advance independently, so changing or disappearing viewport overlays can remain stale. Add a regression with a moving/cleared overlay and unchanged chat/sidebar, plus scene freeze, then correct invalidation independently of this scalar experiment. This is a source-supported mechanism, not yet a live visual reproduction.

Operator clarified that **after the mainland hop** identifies the focused bot losing login-queue priority, not the navigation-paint issue. Preserve that distinction when reproducing the two bugs. Both follow-ups were kept out of the immutable timed binaries.
