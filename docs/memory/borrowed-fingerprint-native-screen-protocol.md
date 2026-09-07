# Borrowed fingerprint native Linux short screen

Predeclared 2026-09-07 before any native paired cell. This is plan section 4B/5
screening, not final absolute-budget or lifecycle acceptance.

Use the reviewed immutable matched Linux x86_64 TUI builds from task
t_ab1637ae only after its actual Grok4.5 approval. Reference is
e3188e2522c4681a47e0127c8202caaa0c3b81a8; candidate is
188a52095f3a8a6db9b2567f5278993b80876852; both use client
3456edc8dabf7b25ada78110ffa56327af9f67a4, locked
`memory-profile-no-alloc`, System allocator and identical toolchain/options.
Bind transferred executable bytes and original source/build receipts separately
from the launcher tooling. Do not build from changing campaign sources.

## Order and conditions

Concord native Ubuntu x86_64 VPS, 2 vCPU and about 2 GiB RAM, no swap,
co-located separately sampled server. Record current server PID/start identity,
43594 listener, fixture/config/cache/nav/catalog hashes and CPU/memory pressure
before each cell. Preserve the existing server and nightly-timer state.
This hardware is not the approved 4 GiB reference profile; report that limit.

Run 16 active bots in reference/candidate/candidate/reference order, followed
by the same order at N1. Use fresh unique run directories and accounts, identical
active fixture/loadout, real PTY 120x40, 30s warmup, 120s observation and 60s
teardown grace. No overlapping frontend, build, profiler or synthetic load on
the VPS. Record unrelated system activity without stopping unrelated services.

For these resource screens use `--no-diagnostics --failure-capture --sustain`.
Omit scheduling/render/responsiveness profiles, fine bins, input probes,
nav captures, native allocation logging and debug. Retain normal boundary
qualification and native RSS/process CPU samples. Managed sampler interval is
0.5s; declare controller/terminal/SSH helpers and sample server resources
separately. Helper/observer overhead remains unqualified until measured, so
this screen alone cannot unlock strict final acceptance.

MemAvailable below 128 MiB triggers the predeclared managed-controller failure
and owned cleanup; preserve the failed cell and stop the sequence. Also stop
the sequence on frontend failure, failed workload/binding qualification,
unexpected source/settings, lost server identity or unresolved contamination.
Do not silently retry a failed cell, average it into successes or weaken its
fixture. Diagnose the named failure before deciding further work.

## Evaluation and follow-up

Evaluate each cell independently before adjacent comparison. Report native
steady median RSS, startup/transition peak, process CPU seconds/wall seconds,
actual ready/active/progress counts, per-slot iterations, memory pressure,
server resource series and helper accounting. Compare reference/candidate and
candidate/reference within each N, retaining individual values and order.
Use both same-role repeats to expose variation; four runs do not establish a
confidence interval. Report N1 to N16 finite differences only with the
co-located-server/headroom limitation visible.

CPU non-regression margin is 5%; intended RSS changes must exceed observed
variation. Concrete allocation removal is separate evidence. No latency pass
is possible from profiles-off resource runs. If the candidate is worth keeping,
collect a separately declared paired latency companion and at most one longer
confirmation stage under plan section 5. The 2ms p99 margin and absolute
resource/cadence gates remain unchanged. Inconclusive results require a named
confounder investigation or parking, not indefinite repetition.

Do not relabel the diagnostic launcher or hardcoded-unavailable helper-overhead
assessment to obtain a pass. Final performance acceptance remains pending until
all required evidence is present and independently reviewed.
