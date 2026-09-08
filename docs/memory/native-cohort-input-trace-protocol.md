# Native cohort input-path diagnostic

Predeclared 2026-09-08, before launch. This is the bounded functional proof
prepared by `panel-input-seam-trace-report.md`, following the prior 0942
zero-counter input diagnostic. It is not the matched N=16 comparison.

Use candidate host `984c63993cebcf4dc3ff6cc965031e896dc685a4`, client
`fd956c91bf09e059359c8e182a33583e2c626cd3`, frozen Windows panel binary SHA256
`8247b70397a0e1580e222ff5d530d9173e1f3d6c3401a794425620e1ac5b9f91`.
The native build and affected host/publisher/panel tests passed. Keep the source
and binary freeze independent of the offline reader, which remains under correction.

Run once after the native build/test processes finish: Windows BotTest console,
N=1, active workload, focused-one policy, 120s warmup and 180s observe,
responsiveness and fine profiles enabled, existing render diagnostic settings,
and `BOT_DEBUG=1` to activate the reviewed bounded input trace. Retain normal
cohort tail and teardown. Preserve the existing cache, nav, server and fixture.
Use a new stage and new output directories; preserve all earlier artifacts.

Read a fresh capture to verify scene2, slot-zero focus, capture enabled, window
rectangle and Game Image point. Only then invoke the exact reviewed `f64b81d`
input helper (SHA256 `04822c0e9f5ade2d555e408cc707722c234bc1e7b442676975a16e7efcaebbd1`)
for one 20-second Left/Right pulse sequence. Its existing PID/start/binary/user/
session/foreground/rectangle checks must pass. Do not equate SendInput success
with host admission. No gameplay movement or teleport stimulus is required.

Archive launch/terminal/controller receipts, all samples and qualification/cohort
sidecars, stdout/stderr, input cadence and identity receipt, and the before/after
captures. Inspect the trace stages independently: WindowEvent, ImGui edge,
capture/hover/focus/channel gate, stream entry, host drain, metric admission and
live telemetry identity, generation bind, and texture present. Respect trace
saturation markers and distinguish queue removals from published completions.

The result must identify the first missing or inconsistent observed stage, or
prove delivery through the endpoint with the actual counters and cohort records.
If incomplete, retain the exact failure and investigate the named source cause;
do not repeat without a source or provenance correction. Reader validation and
combined Grok 4.6 integration review remain prerequisites for the later matched
comparison. This diagnostic establishes neither performance savings nor p99
acceptance, and texture presentation is not display scanout.
