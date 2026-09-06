# Sustained panel32: three fresh processes — 2026-09-06

**Ownership correction:** these runs include a separately decoded navigation pack
per benchmark scenario runner. Their functional qualifications and recorded RSS
remain valid, but the RSS includes substantial harness overhead. Correct that
ownership before using this configuration as a production-memory reference. See
[allocation ownership](allocation-ownership.md).

All three runs qualified. The third maintained 32 ready and active bots for the
600s observation; each gained 45–79 steals. Final statuses were pickpocketing,
eating while stunned, or waiting out stun, with no banking status or script
errors. Exit 0. Both observation boundaries were saved; periodic samples span
599.429s. This completes three repeats of this particular diagnostic cell, not
the full baseline matrix or optimization acceptance.

## Reproduction and scope

Third run: `diagnostics/20260906T033459Z_panel_n32_active`. Same saved System
allocator binary, host/client source hashes, catalog commit and nav pack as
[run one](sustained-panel32-memory.md) and [run two](sustained-panel32-repeat.md).
The launch checkout was `73f3304`; commits after measured build `5e0f3ec` only
add reports/reviews. Binary SHA-256:
`781b93eb3f9b64eb3dbeed6fa3a0c46660b36ecb9fa995ebae3b505c2d8bf76f`.

Each process used fresh local accounts, sustained Thiever, one drawing slot and
31 world-simulation slots, setup qualification, 120s warmup, 600s observation,
and 60s Stop teardown. Scheduling counters enabled; allocation counting,
verbose diagnostics, screenshots and stack sampling disabled. No builds or
reviewers overlapped observation. Raw metadata, samples, boundary qualifications
and reproducing `summarize.py` remain in each run directory. Third-run numeric
summary is `sustained-panel32-third.json`.

## Results

| Metric | Run 1 | Run 2 | Run 3 |
|:---|---:|---:|---:|
| Median current RSS, GiB | 3.747 | 3.762 | 3.809 |
| Whole-process lifetime peak RSS, GiB | 3.855 | 3.846 | 3.953 |
| Final teardown 30s median RSS, GiB | 3.527 | 3.522 | 3.563 |
| First-to-last observation minute RSS increase, MiB | 76.531 | 17.203 | 95.617 |
| V8 used heap median, MiB | 184.369 | 183.550 | 193.633 |
| V8 total heap median, MiB | 236.875 | 261.000 | 254.500 |
| Process CPU, average cores | 0.731 | 0.735 | 0.741 |
| Client ticks per slot-second | 38.765 | 38.633 | 38.624 |
| Mean client-tick work, ms | 0.694 | 0.714 | 0.710 |
| Mean script-tick work, ms | 0.108 | 0.108 | 0.109 |
| Mean UI-frame work, ms | 0.854 | 0.856 | 0.865 |

GiB/MiB use powers of 1024. Memory metrics overlap and must not be summed.
Rust allocation counts/bytes are unavailable. Third-run V8 measurements covered
all 32 isolates, with maximum sample age 1228ms. Snapshot in-flight bytes and
capacity had median and p95 zero, with largest sampled value 846,432 bytes;
this is not a continuous queue peak. Tracked GPU bytes ranged 18,463,952 to
18,791,192: textures stayed at 17,243,992 and buffers ranged 1,219,960 to
1,547,200. Tracking excludes untracked driver/device allocations.

Third-run scheduling: simulation work 0.532ms/cycle, start interval 25.924ms
across 716,550 cycles; drawing work 6.005ms/cycle, interval 24.891ms across
24,100 cycles. Both groups had zero work overruns. Actual sleep exceeded
requested sleep by 5.924ms and 4.890ms respectively. Batched counters can lag
49 cycles per slot; averages do not establish tail latency or an OS-level cause.

After Stop, all three runs had zero active scripts, live isolates, V8 used bytes
and in-flight snapshot bytes/capacity. All 32 clients remained connected.
Retained RSS is not shutdown memory and cannot be assigned wholly to clients
without allocation evidence.

## Interpretation and next evidence

The range of the three RSS medians is 63.320MiB, or 1.639% of their mean. This
is an observed spread for three samples, not a confidence interval or a hard
acceptance threshold. CPU and mean tick/frame work are close; tail-latency
equivalence is not established. V8 used and total heap show different variation
from RSS and must be assessed independently.

All runs show positive first-to-last-minute RSS drift. The third also has the
highest retained RSS and peak. These facts warrant lifecycle/retention profiling;
they do not distinguish caching, allocator retention, workload history or a leak.
No change was made between builds, so this is not a savings comparison.

Next use matched idle and one-bot controls to measure incremental costs, and
lifecycle soak evidence to investigate retention. The TUI, allocation-counting,
CPU-renderer and focus/watch variants remain separate evidence requirements.
N=128 remains deferred. No final whole-branch grok-4.6 approval is claimed.
