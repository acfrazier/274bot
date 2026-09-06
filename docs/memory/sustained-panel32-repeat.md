# Sustained panel32 repeat — 2026-09-06

The second fresh process passed: all 32 clients remained ready and active through
600 seconds of observation; each bot gained 46–83 steals. Final statuses were
pickpocketing or waiting out stun, with no banking status or script errors.
Exit 0. Both observation qualification boundaries are present. The periodic
samples span 599.108 seconds.

Run: `diagnostics/20260906T030558Z_panel_n32_active`. Same saved System-allocator
binary, source hashes, client commit, catalog commit and navigation pack as
[sample one](sustained-panel32-memory.md). Host checkout is now `7f73fb0`, with
only report/review commits since the measured build at `5e0f3ec`. Binary SHA-256:
`781b93eb3f9b64eb3dbeed6fa3a0c46660b36ecb9fa995ebae3b505c2d8bf76f`.

Panel: one drawing slot, 31 simulation slots, sustained Thiever, unique local
accounts. Setup then 120s warmup, 600s observation and 60s teardown. Scheduling
counters on; debug, screenshots, allocation counting and diagnostic sidecar off.
No builds, stack samplers or reviewers overlapped observation. Raw metadata,
samples, qualification and the reproducing `summarize.py` are retained in the
run directory. The tracked JSON preserves the numeric summary.

| Metric | First run | Repeat |
|:---|---:|---:|
| Median current RSS, GiB | 3.747 | 3.762 |
| Whole-process lifetime peak RSS, GiB | 3.855 | 3.846 |
| Final teardown 30s median RSS, GiB | 3.527 | 3.522 |
| First-to-last observation minute median RSS increase, MiB | 76.531 | 17.203 |
| V8 used heap median, MiB | 184.369 | 183.550 |
| V8 total heap median, MiB | 236.875 | 261.000 |
| In-flight snapshot largest sampled bytes | 559,112 | 302,024 |
| GPU tracked median bytes | 18,562,520 | 18,516,080 |
| Process CPU, average cores | 0.731 | 0.735 |
| Client ticks per slot-second | 38.765 | 38.633 |
| Mean client-tick work, ms | 0.694 | 0.714 |
| Mean script-tick work, ms | 0.108 | 0.108 |
| Mean UI-frame work, ms | 0.854 | 0.856 |

Memory categories overlap and must not be summed. Rust allocation counts/bytes
remain unavailable in this build. Heap/queue maxima are sampled, not continuous
peaks; GPU tracking excludes untracked driver allocations. Repeat GPU texture
bytes stayed at 17,243,992; buffer bytes ranged from 1,245,016 to 1,272,088.
All 32 isolates contributed V8 measurements, with maximum sample age 1240ms.

After Stop, active scripts, live isolates, V8 used bytes and in-flight snapshot
bytes/capacity were zero. All 32 clients remained connected. Retained RSS is
therefore neither shutdown memory nor proof of wholly client-owned storage.

Repeat scheduling: simulation work averaged 0.536ms and start intervals
25.917ms over 716,650 cycles, with one work overrun. Drawing work averaged
6.008ms and start intervals 24.935ms over 24,000 cycles, with zero overruns.
Actual sleep exceeded requested sleep by 5.916ms (simulation) and 4.934ms
(drawing). Counters can lag by 49 cycles per slot; means do not establish tail
latency. The late-wakeup observation persists without establishing its OS cause.

Median RSS differed by +0.390% and teardown RSS by about -4.45MiB. This is
encouraging agreement for these two runs, not a statistical noise bound or a
savings result. V8 total heap median increased about 10.2% despite similar used
heap and RSS; separate memory metrics cannot be assumed equally stable. RSS
drift was smaller but positive again, so lifecycle evidence remains useful.

Next evidence is a third identical run to complete the planned three repeats,
then matched idle/1-bot controls and the lifecycle soak. N=128 remains deferred.
This report does not claim optimization acceptance, tail-latency equivalence,
or completion of the final whole-branch grok-4.6 review.
