# Corrected native fixture qualification — failed

Date: 2026-09-08 UTC
Source: `8b4d6f5a3686e85e5fed55574b205936d4be45a1`
Stage: `/home/acfrazier/owner-native-8b4d6f5`
Output: `/home/acfrazier/owner-native-8b4d6f5/qualification-1`

This is one fresh, fixture-only Linux execution of the reviewed driver. It ran
without `--portable-probe`, did not read a production input, and did not retry.
The prior failed qualification remains untouched.

## Launch and identity

The local SSH background launch PID was `17201`; the driver exited `1` after
`17.508897191997676` seconds. The remote qualification PID was not recorded by
the driver. The output directory was absent in the prelaunch check. A post-run
process observation found no qualification, core-test, or owned fixture child
processes. Every one of the six completed timed cells reports
`owned_child_reaped: true`.

The root prelaunch admission receipt is
`diagnostics/replay-native-8b4d6f5-preparation/concord-stage-receipt.json`:
Unix time `1788909930.0377018`, 1,002,622,976 available memory bytes,
11,819,978,752 free disk bytes, zero conflicts, 40 verified files, and stage
manifest SHA-256
`71d33982b5bf2ab06f2ef65560325f5609a64121a041231f3d87eb173fa870a4`.
This is explicitly a root prelaunch receipt, not a worker-created post-run
admission receipt. No worker admission receipt was backdated or invented.

The qualification JSON SHA-256 is
`b7a2af38b7adb1b0f2e2c0d611a53ac2fa168bbf3b1d6e952bf33f6d0db47e83`.
The executable identities matched the released values:

- child: `16f0ac41ffcc8c8134b48ea03704890a681f8f5c07c564034838359eeee4f8f5`
- fixture: `10d2cb7113e457fefd508391987f516c90d36b9231ccc75e4f3fdcc21613f8d6`
- core tests: `ba4d0cf258ff82563f23f506ac0d4f622b3cbbb5f5e00e9aedf55be287f0386c`

## Completed gates

The native core gate passed: 16/16 tests, no failures or ignored tests. The
Python/native suite passed 36/36 tests with no skips. This includes the saved
smoke test and negative guard tests. The bounded transcript does not emit
separate per-guard reason receipts or saved-smoke output hashes.

The driver completed two of nine declared cases, with three serial repetitions
each (six of 27 timed repetitions). It stopped at the reviewed necessary-rate
criterion, as required:

| case | phase 1 median | phase 2 median | phase 3 median | necessary-rate result |
|---|---:|---:|---:|---|
| 250,000 narrow | 15,743,928.488972072 | 5,105,346.04256718 | 4,845,660.757015204 | pass |
| 250,000 wide | 35,701,922.15412879 | 5,033,171.737584553 | 4,416,362.4607893815 | **fail** |

Required rates were 11,938,945 / 4,503,128 / 4,503,128 bytes per CPU-second.
The wide case's phase-3 median is below the required 4,503,128 threshold. The
qualification JSON records `RuntimeError: necessary throughput threshold failed`.

I independently recomputed every stored phase rate and median from the stored
byte and CPU rows. The two completed cases provide two phase-median vectors,
six medians total; all 18 stored phase rates and all six medians match the
recomputed values exactly (24 arithmetic checks). Phase-3 output rows are
included in that recomputation. Each completed timed cell has exactly one
external resource sample (`samples: 1`), so 417,792 RSS bytes and 1,445,888
address bytes are single early observations, not proven process peaks or a
largest-cell memory bound. The native phase rows separately report a
cumulative peak metric of 31,944,704 RSS bytes
(`cumulative_peak_rss_bytes`). The largest recorded `max_sample_interval_s`,
0.0006020780019753147 seconds, is only initial sample latency under this
one-sample result, not a sustained sampling cadence; the unsampled tail spans
nearly the entire approximately 0.101-second wrapper wall time.

## Result and limits

This qualification is **failed** on the necessary throughput criterion. It is
not a production retry authorization. The driver reports both
`production_read=false` and `production_retry_authorized=false`; no ranking was
published. No later cases ran, so no claim is made for the unexecuted 500,000,
near-cap, diversity, symbol, or near-line-cap cases.

The bounded qualification output contains no separate completion, admission,
per-guard, or saved-smoke hash receipt. `test_saved_smoke` is recorded as
passing in the 36-test transcript, but full canonical output/hash and
explicit graph/cutoff comparison evidence cannot be independently re-emitted
from this output. These omissions are retained as evidence limitations rather
than filled with inferred values.

Passing core and correctness gates plus this small-fixture result do not prove
production population/table/output/I/O capacity, owner attribution, or memory
savings. Root must make a new explicit production decision; this card grants
none.

Raw bounded result: `diagnostics/replay-native-8b4d6f5-qualification-1/qualification.json`
Structured evidence: `docs/memory/heaptrack-owner-native-corrected-qualification.json`
