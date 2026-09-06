# Bounded renderer reference proof

This executes the approved plan's reference stage. It does not replace the final
matrix, modest-hardware tests, responsiveness gates or whole-branch review.
Use the reviewed renderer observations and GPU-completion instrumentation on one
immutable, jointly built panel/TUI release with `memory-profile-no-alloc`.

## Cells and order

All use the local sustained Thiever fixture, 30s warmup, 120s observation and
60s teardown, system allocator, and preserved boundary workload qualification.
Run sequentially; no builds, reviews/tests, native profiling or other benchmark
processes overlap the measured cells. Do not modify server random-event policy.

1. Panel N=1, `--focused-one --render-profile --gpu-completion-profile`, with
   native navigation captures for visual inspection. This is functional/visual
   diagnostic evidence; exclude its resource values from clean acceptance.
2. Actual PTY-backed TUI N=16, `--render-profile`, no verbose sidecar/captures.
3. Panel N=16, `--focused-one --render-profile --gpu-completion-profile`, no
   verbose sidecar/captures.
4. Panel N=16, `--focused-background --render-profile --gpu-completion-profile`,
   no verbose sidecar/captures.
5. Same panel N=16 background mode with both new profiles disabled. This short
   adjacent comparison screens their combined overhead on the same binary;
   it is one noisy pair, not an accepted optimization saving. Retain failures.

No third-party GPU wait/readback is introduced to make the evidence pass.
The capture pilot deliberately reads pixels, so it stays outside clean metrics.
A failed cell is preserved and diagnosed within the approved bounded limit.

## What the report must establish

First run `qualify_control.py` with explicit expected modes; frontend exit0 alone
is insufficient. Check source/binary provenance and requested frontend/policy.
Map observed slot IDs to fixture names; require unique lifecycle generations,
full requested coverage and recent snapshots. Observe and teardown are distinct.

For TUI, report zero actual resident renderers and zero paints. For one-renderer
panel, report exactly one actual GPU backend and no heads on the remaining slots.
For background panel, report N GPU backends, full-rate only on slot0, and the
measured background paint rates/intervals beside the requested 1fps policy.
Report CPU fallback, missing/stale slots and lifecycle changes explicitly.

For GPU work report registration/completion counts, outstanding callbacks,
drops/overflow and observed delivery latency/interval histograms. Incomplete
registration coverage cannot qualify completed-frame throughput. Count completed
GPU work separately from host mainredraw calls and panel UI submissions.
Callback delivery time includes CPU polling/queue delay after completion; it is
not a hardware GPU timestamp or display scanout. Do not claim the final precise
presentation/response gates from these observations alone.

Compute observation deltas only across consistent generations/modes. Histogram
percentiles are bucket upper bounds, with overflow/unavailable stated explicitly.
Keep startup/scene-transition intervals separate; no rounding a >40ms sample into
an inclusive40ms bucket. Report all missing metrics, particularly decoded-update
→ script-dispatch and input → visible acknowledgement latency.

Inspect native PNGs for a real ingame scene and banking/return behavior; preserve
last-FBO freeze and overlays. Capture files existing alone are not visual proof.
Final output: cell qualification, actual renderer counts/backends/cadence,
completion coverage, provisional resource values, overhead-screen limits and
remaining gaps, linked to raw run directories and review receipts.
