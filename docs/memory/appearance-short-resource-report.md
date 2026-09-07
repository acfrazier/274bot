# Appearance packet storage: short resource comparison

All eight real-TUI cells completed with exit 0, qualified native workload and
continuous process accounting. The short RSS and CPU comparisons are
**inconclusive at both N=1 and N=16**. No measured RSS saving or CPU/latency
non-regression acceptance is granted. The representation change remains
provisionally retained on its concrete empty-table allocation removal and
reviewed behavior tests, under performance-finish-plan section 5.

## Scope and frozen comparison

Client commit `85266df125487d7669320d53071eedc14caf9f89` changes the per-client
appearance table from `Vec<Option<Packet>>` to `Vec<Option<Box<Packet>>>`.
Host gitlink/report commit is `b968e35`. The reviewed code preserves 2,048 slots,
independent packet ownership, cursor reset, remove/re-entry handling and login
reset. The measured type layout removes 4.109375 MiB from the empty table per
client (65.75 MiB at 16), before populated packet boxes and allocator effects.
That is a structural allocation calculation, not RSS savings. See
[implementation and tests](appearance-packet-box-report.md).

The frozen manifest is
`diagnostics/matched-appearance-build-20260907T103345Z/build-manifest.json`.
Its reference explicitly reuses the **pre-appearance shared-navigation candidate**
from `matched-instrumented-build-20260907T101400Z`, original role `candidate`.
It is not the older duplicate-navigation reference. The reused manifest path/SHA
and original role are recorded in `control.reused_build_lineage`; immutable binary
copies retain their hashes. Host sources, toolchain, build features, fixtures and
settings match; the intentional client source difference is independently bound
to each role. Current host sources were stable across the candidate build.

- Reference TUI SHA-256: `eaaa45a718ca340f808e3467c469ff804e6875d7425fe41edfc715a698dd53a1`.
- Candidate TUI SHA-256: `9eb187b7064e0b1bf10bc82722d2ed1210189ee0a87987399006591dcad8bdc7`.
- Client sources: reference `27246deba654abac980395b0bbf9ad077d8d3feb66992b4d659512e0630e4a25`, candidate `e3ac0de4fc3fbadcd933c9c5acc082ed0d4b0f44d35444515b342e0af09a6281`.

## Protocol and observations

The batch was declared before launching: N16 R/C/C/R, then N1 R/C/C/R;
30s warmup, 120s observation, existing 60s normal teardown; active sustained
Thiever, actual PTY and input probes. All hot profiles, allocation counting,
verbose diagnostics and native stack captures were disabled. Each launch has a
quiet-board snapshot with no campaign build/test/review workers. This does not
claim an idle Mac: unrelated user/OS activity remains outside the recorded helper
scope. The development Mac is not reference-hardware validation.

Every receipt was independently re-bound using the explicit immutable server and
host-condition sidecars. Native settings and every runtime match key agree except
for the expected, role-bound client source hash. Distinct chronological runs,
same binary within each role, cache identity and continuous helper/server/host
accounting were checked. The server and helpers remain separate in the result;
no memory or CPU subtraction is applied.

| N | Cell | Host median RSS MiB | Sampled max MiB | Recorded highwater MiB | CPU cores | Mean loops/slot/s | Minimum steal gain |
|---:|:---|---:|---:|---:|---:|---:|---:|
| 16 | reference_a | 930.781 | 952.609 | 952.766 | 0.226616 | 37.114 | 5 |
| 16 | candidate_a | 812.781 | 829.250 | 829.531 | 0.221120 | 37.117 | 2 |
| 16 | candidate_b | 883.953 | 915.250 | 915.578 | 0.191157 | 38.893 | 3 |
| 16 | reference_b | 872.523 | 907.875 | 908.000 | 0.148441 | 42.245 | 5 |
| 1 | reference_a | 298.281 | 316.422 | 316.469 | 0.011292 | 41.529 | 10 |
| 1 | candidate_a | 291.953 | 296.625 | 296.672 | 0.021828 | 36.563 | 9 |
| 1 | candidate_b | 295.172 | 300.078 | 300.109 | 0.025325 | 36.629 | 11 |
| 1 | reference_b | 290.766 | 294.562 | 294.797 | 0.023027 | 36.605 | 4 |

The loop values are process-derived averages, not per-slot p99 scheduling proof.
All requested slots made positive progress; qualification does not turn values
below 40 loops/slot/s into an accepted cadence result. Recorded highwater does not
replace the final startup/transition matrix. The teardown samples observed zero
live isolates and in-flight snapshot bytes; these short teardowns are not the
required repeated lifecycle plateau test.

## Predeclared decision

RSS compares minimum reference median minus maximum candidate median against
the largest within-role spread. CPU uses candidate maximum/reference minimum
at 1.05, requiring both replicated spreads to be at most 5%. These are observed
ranges, not confidence intervals.

- N=16: minimum RSS reduction -11.430 MiB versus 71.172 MiB within-role spread; no RSS screen support. CPU spreads are 52.66% reference and 15.67% candidate: inconclusive.
- N=1: minimum RSS reduction -4.406 MiB versus 7.516 MiB within-role spread; no RSS screen support. CPU spreads are 103.92% reference and 16.02% candidate: inconclusive.

Both reference and candidate readings remain above the one-bot 256 MiB and
16-bot 512 MiB median targets. Scheduling also drifted across the N16 sequence
(37.114, 37.117, 38.893, 42.245 average loops/slot/s), while CPU fell. This is an
observed confounder candidate, not a proven explanation or a basis for correcting
measurements. A favorable first pair is not selected over the full quartet.

## Next bounded stage and remaining evidence

Use one longer N16 reference/candidate confirmation stage (120s warmup/600s
observation, same frozen binaries/settings and ordinary teardown). If it remains
noisy, investigate a specific source of variation or park further performance
claims; do not repeat indefinitely. No source change is needed for that stage.

Current profile-ON overhead calibration and latency non-regression remain pending.
Older overhead results are not transferred to these binaries. The existing
`resource_screen.analyze`/full acceptance gates were not weakened; this batch's
small `analyze_batch.py` emits only descriptive resources and keeps
`accepted_rss_saving`, `performance_acceptance` and `final_acceptance` false.
Actual renderer backend evidence is unavailable with profiles off. Absolute
budgets, full behavior/lifecycle/rendering matrix, Linux limits/reference hardware
and whole-branch Grok-4.6 remain required. This report is not campaign completion.

## Reproduction and evidence

All artifacts are under
`diagnostics/appearance-resource-screen-20260907T103755Z/`:
`batch.json`, eight immutable specs, eight quiet-before snapshots, cell receipts,
individual bound results, `analyze_batch.py` and
`descriptive-resource-results.json`. Run
`python3 docs/memory/diagnostics/appearance-resource-screen-20260907T103755Z/analyze_batch.py`
to re-bind the raw evidence and reproduce the arithmetic. The result includes all
host, server and helper metrics and each raw run identity. Raw and failed earlier
campaign artifacts remain preserved; this batch required no retries.
