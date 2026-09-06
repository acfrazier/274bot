# Targeted allocation owners

Successful native captures now identify substantial API snapshot ownership and
confirm explicit Stop releases the encoder/fingerprint allocation paths. No
application optimization is implemented here. These are intrusive diagnostic
runs, not performance baselines or evidence of an RSS saving.

## Capture method and qualification

All runs used the unchanged saved System binary `616196ac…6c9b`, matching host
and client source digests and navigation pack from the corrected controls.
One drawing slot, 31 simulation slots, fresh local accounts, short warmup and
observation. Raw metadata, qualification, commands, timestamps and selected
allocation groups with line references are in `targeted-allocation-owners.json`.

The first attempt (`20260906T153122Z_panel_n32_seeded-idle`) used the existing
`MallocStackLogging=1` option. It produced a roughly 13GB on-disk history file.
A filtered `malloc_history -callTree` returned no allocation entries; a full
`-allBySize` query timed out after 120s with empty output. Its VM summary is valid,
but its observation is incomplete (95.273s sampled of 180s requested), despite
application exit0. It is **failed attribution evidence**, not a passing baseline.

Commit `31b9cc8` adds mutually exclusive `--stack-logging-lite`, sets explicit
`MallocStackLogging=lite`, and records the mode in metadata. The original flag's
behavior remains unchanged. The local `malloc(3)` manual describes lite mode as
retaining current allocation stacks without full on-disk history. Actual retries
show the lite allocator zone and successful full reports. CLI help and conflicting
flags (exit2 before launch) were checked; both successful live runs exercise the
new mode. No Rust/client binary changed.

| Capture | Run | Ready/active before capture | Full stack query duration | Result |
|:---|:---|:---|---:|:---|
| Idle32 | 20260906T153903Z_panel_n32_seeded-idle | 32/0 | 3.931s | exit0 |
| Active32 | 20260906T154457Z_panel_n32_active | 32/32 | 5.039s | exit0 |
| Same process after Stop | same active run | 32/0 | 3.973s | exit0 |

Each stage collected `vmmap -summary PID` followed by `malloc_history PID
-allBySize`. VM queries took 0.7–1.1s and all succeeded. Before/after samples stayed
in the intended phase. The two lite runs completed with exit0 and qualified for
their shortened scenarios: 30s warmup, 120s requested observation, 60s teardown.
Sampled spans 118.471s and 118.628s; all 32 active bots gained 3–18 steals.
Stop samples confirm zero live isolates, V8 used and in-flight snapshot bytes/
capacity. This is a short lifecycle check, not the planned hour-long soak.

Logging substantially perturbs performance: active CPU was 14.27 cores in this
instrumented run versus below one core in clean runs. Do not compare its RSS,
timing or allocator layout directly with the clean controls. Stack-logging
allocation sizes include allocator/instrumentation effects and differ from raw
Rust requested capacities. The reports also contain VM/mmap records; native
allocation totals, RSS, V8 and GPU gauges must not be added together.

## Concrete owners

The following are sums of disjoint matching allocation records within each
report. The widget/location vectors themselves total **187,432,960 bytes
(178.750MiB)** in both idle and active captures, and 188,682,240 bytes
(179.941MiB) after Stop. This does not include all snapshot fields or string
allocations.

| WidgetView + LocView allocation path | Idle/active bytes | After Stop bytes | Current owning state |
|:---|---:|---:|:---|
| Panel Session callback | 48,381,952 | 48,381,952 | `Session::nav_states` GameSnapshot per slot |
| Host client_frame | 45,334,528 | 45,959,168 | host Slot snapshot |
| Host client_tick observation path | 45,334,528 | 45,959,168 | host-play script/navigation observation snapshot |
| ScenarioRunner::tick_with_hold | 48,381,952 | 48,381,952 | benchmark seed runner snapshot |

Source matches the paths: `crates/panel/src/session.rs` stores full snapshots in
`nav_states` and rebuilds them in the callback; `crates/host/src/lib.rs` retains
`Slot::snapshot`; `crates/host-play/src/lib.rs` creates `nav_snapshot` and rebuilds
it for observation; `crates/scenario/src/runner.rs` retains its snapshot after
Done. Publication timing, guardian use, navigation and diagnostics can differ
between these owners. Their similar contents do not authorize replacing them
with one mutable object. The benchmark portion is still harness overhead.

Additional selected call-path sums (MiB):

| Path | Idle | Active | After Stop |
|:---|---:|---:|---:|
| Client::construct, all matching records | 220.290 | 220.290 | 220.290 |
| RenderWorld::run_share_light | 66.290 | 66.297 | 66.297 |
| World::set_ground | 64.717 | 64.717 | 64.717 |
| NavWorld paths | 140.988 | 140.988 | 140.988 |
| IsolateBuf paths | 0 | 30.639 | 0 |

These path categories are attribution filters, not a complete disjoint
accounting model; do not sum the table. The largest construction group is 32
allocations totaling 139,460, 608 bytes (4,358,144 each in lite mode). Its exact
field still needs source/size attribution; no sparse-table proposal follows
from the function name alone. Navigation remains two worlds, not per-bot packs.

The active IsolateBuf group totals **32,127,248 bytes**. It includes 14,262,848
bytes in SnapshotFingerprint paths, of which 13,628,880 bytes match SceneEntityFp.
These are nested subsets, not additional totals. No matching IsolateBuf or
fingerprint allocation records remain in the post-Stop report. This corroborates
the Stop regression with successful paired native evidence; it does not isolate
an RSS saving, prove all script-originated allocations are gone, or measure
allocation traffic per tick.

## What the retained-memory comparison establishes

The rounded vmmap malloc-zone allocated total is 998.4M during activity and
899.0M after Stop, versus 895.6M in the separate idle process. The summaries
were taken just before the stack queries, not simultaneously. Retained
client/scene/API ownership remains substantial, while many active allocations
disappear. The closeness of post-Stop and idle live totals makes allocator
retention a plausible contributor to the clean RSS gap, but does not prove its
size or explain the 235–406MiB gap observed in longer, uninstrumented runs.
A longer lifecycle series is still needed to establish growth or leaks.

## Recommended next bounded work

Audit the four snapshot consumers and their required families/publication
boundaries first. The capture provides a stronger measured lead here than
heightmap sharing's 0.168MiB payload per client. Candidate changes should remove
unneeded retained families or share immutable published data while preserving
each consumer's view and proof data. Verify in particular what panel navigation
and completed benchmark runners need before narrowing either snapshot.

Keep borrowed fingerprint comparison and the consumed-buffer return pool as
independent planned candidates for allocation churn. They need traffic/reuse
measurements and exact delta/lifetime regressions, not just this live allocation
snapshot. Renderer redesign, sparse entity tables, queue coalescing and V8 limits
remain outside the current batch. No whole-branch grok-4.6 approval is claimed.
