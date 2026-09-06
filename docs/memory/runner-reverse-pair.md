# Reverse-order runner cleanup screen — qualification rejected

The corrected-binary run qualified; the previous-binary run did not. Do not use
this pair for an RSS saving, CPU regression/non-regression, latency-equivalence,
or capacity claim. The earlier qualified forward pair and native allocation
removal remain evidence with their existing limits.

## Provenance and procedure

Approved finish plan: `530b83e`, `docs/memory/performance-finish-plan.md`.
Launch checkout was that commit, clean tracked diff. This was a reverse-order
repeat of the prior short pair: corrected first, previous second, sequential
panel32 sustained Thiever, one fixed renderer, 30s warmup, 120s observation,
60s teardown. No builds, reviewers, native allocation captures or verbose
profiling overlapped either run. Scheduling counters were on; stack logging,
verbose diagnostics and navigation captures were off.

| Order | Saved binary | Run directory | Qualification |
|---|---|---|---|
| 1, corrected | `runner-boundary-build/panel-play-system` | `20260906T181830Z_panel_n32_active` | Pass, all 32 gained 5–19 steals |
| 2, previous | `shared-stop-build/panel-play-system` | `20260906T182432Z_panel_n32_active` | Fail: one zero-gain slot and three below-scale samples |

Saved binary SHA-256 values remain `df54511ef56a2ad31b2a9d3aae76ffda07e774735e1adcf42bb808f9773f74ae`
(corrected) and `616196ac16974c67ef331b107d17a87f907b963a7fc7e3256ef96d3754136c9b`
(previous). Saved build source provenance is the same as the
[forward-pair report](runner-clean-pair.md): corrected host source `dc8b85e9…`,
previous `9a4089c2…`. Both new metadata records describe the launch checkout's
`dc8b85e9…` sources; that does not change the older binary's build provenance.
Client source, nav pack and catalog identifiers match between the new runs.

## What failed

`live14f0f_11` remained Running, dispatched 202→401 script ticks, and reported no
script error, but its stealing counter stayed 17→17. At observation start it
was banking at (2659,3308), with three food items and no completed bank trip.
At observation end it was at (2008,4762), with an empty inventory and one reported
bank trip, while its paint still targeted an Ardougne guard at (2657,3304).

`live14f0f_1` gained only one steal and moved from (2649,3284) to (2891,4597).
Both endpoint clients reported ingame/scene_state2. These records establish
out-of-area endpoints; they do not identify the cause of relocation or prove a
navigation or guardian defect. They also do not prove a relationship to the
operator's earlier bank-booth bouncing observation.

The previous run had ready31/active31 at elapsed 232.778s, 238.803s and 239.807s,
with 32 isolates still live. The one-second samples do not identify which slot
caused each dip. The whole-run elapsed times differ from observation-relative
times; observation began at 192.283s.

Both processes exited 0 and printed observation completion. Post-run
qualification is stricter than the frontend's completion message: it correctly
rejected the previous run. Future acceptance orchestration must propagate the
qualification failure as exit1, not equate frontend exit0 with a passing cell.

## Diagnostic values only

| Metric | Corrected, qualified | Previous, unqualified |
|---|---:|---:|
| Observation span, s | 118.916 | 118.862 |
| Median RSS, MiB | 1854.812 | 1901.219 |
| Final teardown 30s median RSS, MiB | 1508.039 | 1573.203 |
| CPU, mean cores | 0.6225 | 0.5967 |
| Client iterations/slot/s | 42.499 | 39.765 |
| Mean client tick, ms | 0.4798 | 0.5004 |
| Mean simulation work, ms | 0.3431 | 0.3629 |
| Mean drawing work, ms | 4.6078 | 4.6195 |

No sampled work overruns occurred in either group. Both final teardown samples
had ready32, active0, zero live isolates, zero V8 used bytes and zero in-flight
snapshot bytes/capacity. The lower previous-run CPU accompanies reduced client
throughput and invalid workload qualification; it is not a valid efficiency
comparison. Precise per-slot p99 and GPU completion latency remain unavailable.

## Decision and next action

Retain the candidate and its demonstrated 46.1 MiB native runner-vector removal
without claiming full performance acceptance. Preserve the failed reverse pair;
do not average it into the qualified forward pair or repeatedly rerun for a
favorable number.

Investigate the out-of-area/progress confounder with one bounded diagnostic run
that records navigation, client position and guardian hold. Treat that run as
functional diagnosis only. Strengthen qualification exit-status handling before
using the expanded 1/16 three-mode reference as an acceptance pipeline. No runtime
behavior or fixture flags were changed to manufacture a pass.

The [JSON receipt](runner-reverse-pair.json) includes full summaries, metadata,
selected raw boundary records, dip samples, raw-file hashes and the earlier
forward-pair receipt.
