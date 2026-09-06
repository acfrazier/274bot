# Panel memory controls — 2026-09-06

**Ownership correction:** subsequent native allocation profiling found a separate
navigation pack retained by every benchmark scenario runner (about 70.4 MiB
each). The slopes below include that harness overhead and must not be treated as
production client costs. Numbers remain historical measurements of the recorded
configuration. See [allocation ownership](allocation-ownership.md).

Three new controls passed: one active Thiever, 32 seeded-idle clients, and one
seeded-idle client. Each used a fresh process and accounts, one drawing slot,
120s warmup, 600s observation and 60s teardown. All clients stayed ready at
requested scale. The active bot gained 62 steals; both idle runs had no scripts,
live isolates or in-flight snapshots. Both idle boundary records place every
client at (2661,3306,0), ingame with scene_state 2. All processes exited 0.

This is one run per new control, compared with the previously qualified
[three 32-active runs](sustained-panel32-three-runs.md). These controls are useful
directional evidence, not repeated-control acceptance or proof of savings.

## Setup correction and provenance

The existing `idle` workload installs no seed runner, so fresh accounts do not
reach the active scenario's scene. It cannot serve as a scene-matched control.
Commit `d3abfb4` adds `seeded-idle`, preserving the original `idle` behavior. It
reuses sustained Thiever setup (mainland hop, stats, food, bank stock, teleport,
dialogue drain), truncates before StartScript and proves arrival. It neither
loads the catalog card nor creates a script isolate. Qualification additionally
checks idle state, no live isolates/in-flight snapshots, and client scene/tile.

The setup is matched; subsequent history is deliberately different: active bots
move, bank and allocate script state, while idle bots remain at the guard stand.
Also, idle simulation clients use the existing parked cadence. Thus an
active-minus-idle difference is the whole workload difference, not pure V8 or
script allocation cost. The 32-idle fleet averaged 3.067 client ticks per
slot-second, versus about 38.6–38.8 for the active fleet. No cadence policy was
changed. The single drawing idle client averaged 39.620 ticks/s; active 43.003.

| Control | Raw directory under diagnostics/ | Launch host |
|:---|:---|:---|
| 1 active | 20260906T040226Z_panel_n1_active | dcece07 |
| 32 seeded-idle | 20260906T042057Z_panel_n32_seeded-idle | 028ff53 |
| 1 seeded-idle | 20260906T043635Z_panel_n1_seeded-idle | 028ff53 |

Active used the same saved binary as the three active fleet runs, SHA-256
`781b93eb3f9b64eb3dbeed6fa3a0c46660b36ecb9fa995ebae3b505c2d8bf76f`.
Idle used the additive harness build, SHA-256
`33ec534685ddd2c59431124fe3aa0ba402aad0430172536d14a46f21a38f13f8`, saved at
`diagnostics/seeded-idle-build/panel-play-system`. Its host sources hash is
`00d82bee74b37acb89dd8248472d46114b8ec50d0c3ab3cb5846388a5a89fa1e`.
Client sources, catalog and nav pack match prior runs. Full metadata and numeric
summaries are retained in `panel-memory-controls.json`. These are two binary
builds, not a claim of identical executables across active and idle.

All use System allocation with Rust allocation counting unavailable, scheduling
counters on, debug/verbose diagnostics/screenshots/stack sampling off. No builds,
tests or reviewers overlapped observation. Harness tests were edited during the
one-active run, but execution/builds waited until that process exited; its saved
binary was unchanged. Raw samples, qualification boundaries and metadata are in
the run directories. `summarize_control.py RUN_DIR` reproduces summaries and
handles absent simulation cycles and absent V8 samples as null, not zero work.

## Memory and CPU

MiB uses 1024 squared. Each memory category can overlap another; do not sum them.

| Metric | 1 active | 32 seeded-idle | 1 seeded-idle |
|:---|---:|---:|---:|
| Median current RSS, MiB | 645.313 | 3400.313 | 611.953 |
| Lifetime peak RSS, MiB | 657.078 | 3407.219 | 617.219 |
| Final teardown 30s median RSS, MiB | 641.281 | 3405.344 | 615.375 |
| First-to-last observation minute RSS increase, MiB | 18.563 | 7.500 | 12.297 |
| CPU, average cores | 0.333 | 0.351 | 0.317 |
| Median V8 used bytes | 6,291,584 | 0 | 0 |
| Median tracked GPU bytes | 18,395,120 | 18,292,776 | 18,292,776 |

The active isolate contributed heap samples with maximum age 1220ms. Its largest
sampled in-flight snapshot was 273,296 bytes (median/p95 zero), and V8 used heap
maximum sample was 13,286,096 bytes. Sampled maxima are not continuous peaks.
GPU tracking excludes untracked driver/device allocations. After Stop the active
isolate and in-flight bytes/capacity were zero; idle had zero throughout. Clients
remained connected during teardown; retained RSS cannot be assigned wholly to
client-owned allocations and is not shutdown memory.

Drawing-group mean work/interval in ms: 1 active 5.581/23.252, 32 idle
5.939/25.249, 1 idle 5.787/25.240. Work overruns were 0, 1 and 0 respectively.
Simulation cadence counters are zero in these three controls: there is no
simulation slot at N=1, and idle simulation slots take the parked path at N=32.
This does not imply those idle clients performed zero work. Counters publish in
batches and can lag 49 cycles; averages do not establish tail-latency equivalence.

## Incremental costs and limits

Using (median RSS at 32 minus median RSS at 1) / 31:

- Seeded-idle: 94,316,346 bytes, about **89.947 MiB per additional client**.
- Active: 107,967,653–110,109,465 bytes, about **102.966–105.009 MiB per additional
  active simulation bot**, across the three active fleet medians.

These are finite differences over this range with one renderer, not isolated
allocation ownership, a linear scaling law or an N=128/1000 prediction. Each
uses only one N=1 control and idle also has only one N=32 sample. Activity,
packet load, scene population, history and cadence differ. No percentage savings
is claimed, and subtracting these slopes does not isolate script cost.

At 32, active RSS exceeded seeded-idle by 436.945–500.266 MiB. Even after scripts
stopped, active runs retained 201.672–243.023 MiB more RSS than the idle control's
teardown median. That is evidence to investigate activity-associated retained
storage or allocator retention; it does not identify a leak or its owner.

The large idle slope makes per-client retained allocations the next useful
profiling target. Repeat the controls for variability and collect allocation
owner evidence before selecting changes. A lifecycle soak remains necessary to
understand longer-term retention. TUI, CPU-renderer and focus/watch evidence
remain separate; N=128 stays deferred. This does not replace final whole-branch
grok-4.6 review.

## Verification

New Rust regression first failed because seeded-idle was rejected; after the
change all 133 host-play unit tests plus enabled integration tests passed under
`--features memory-profile-no-alloc`. Fourteen Python qualification tests pass,
including bad scene, activity, isolate, missing boundary and readiness cases.
Release panel built successfully. No client source changes require client tests.
Grok-4.5 approved the harness commit; the three qualified live controls supply
its live evidence. Control-report review is recorded separately.
