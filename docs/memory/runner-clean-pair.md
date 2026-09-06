# Short clean comparison: terminal runner snapshot release

Both sequential 32-bot runs passed. The corrected binary's median RSS was
62.625MiB lower during activity and 63.922MiB lower during final teardown.
This agrees in direction with the native evidence of removed runner-owned
allocations. One short pair does not establish an exact repeatable RSS effect
or full performance equivalence: CPU and mean work were somewhat higher.

## Configuration and provenance

Operator-approved short comparison: 30s warmup, 120s observation, 60s teardown;
panel32 sustained Thiever, one drawing slot and 31 simulation slots. Fresh
sequential processes and ephemeral accounts against the local server. Stack
logging, verbose diagnostics and captures off; scheduling counters on. No
builds, reviewers or intrusive profiling overlapped either run.

| Cell | Diagnostic directory | Saved binary SHA-256 |
|:---|:---|:---|
| Before | 20260906T163141Z_panel_n32_active | 616196ac16974c67ef331b107d17a87f907b963a7fc7e3256ef96d3754136c9b |
| After | 20260906T163734Z_panel_n32_active | df54511ef56a2ad31b2a9d3aae76ffda07e774735e1adcf42bb808f9773f74ae |

Before is `shared-stop-build/panel-play-system`, the runner-retaining source
state recorded in the prior corrected controls (host sources `9a4089c2…`).
After is `runner-boundary-build/panel-play-system`, source state `0e909b9`
(host sources `dc8b85e9…`). Both launches occurred from checkout `0af4481`.
**Launch metadata's host commit/source digest describe the checkout, not the
source of an older saved binary.** Binary digests and the separate provenance
record in `runner-clean-pair.json` distinguish them. Client source digest,
navigation pack, catalog and build configuration match the previous evidence.

Both exit 0 and qualify with empty error lists. Sampled observation spans are
118.812s and 119.041s, with both boundary proofs present. Ready32/active32 holds
throughout observation; gains are 5–21 steals before and 3–18 after. Final Stop
has ready32/active0, zero live isolates, V8 used and in-flight bytes/capacity.

## Results

MiB is 1,048,576 bytes. Teardown is the median of its final 30 sampled seconds;
peak covers the entire process lifetime, including startup and transitions.

| Metric | Before | After |
|:---|---:|---:|
| Median RSS, MiB | 1917.422 | 1854.797 |
| Lifetime peak RSS, MiB | 1924.859 | 1867.766 |
| Final teardown RSS, MiB | 1587.516 | 1523.594 |
| CPU, mean cores | 0.645 | 0.678 |
| Client ticks/slot/s | 42.528 | 42.485 |
| Mean client tick, ms | 0.508 | 0.546 |
| Mean script tick, ms | 0.123 | 0.120 |
| Mean UI frame, ms | 0.656 | 0.679 |
| Simulation loop work, ms | 0.374 | 0.389 |
| Simulation start interval, ms | 23.534 | 23.561 |
| Drawing loop work, ms | 4.549 | 5.261 |
| Drawing start interval, ms | 22.933 | 22.855 |

No observation work overruns in either drawing or simulation group. Batched
counters can lag 49 cycles per slot, and means do not establish tail latency.
The drawing-work increase of 0.711ms is visible even though cadence is similar;
do not claim latency equivalence from the interval alone.

RSS deltas after minus before: median−65,667,072 bytes, lifetime peak−59,867,136
bytes, teardown−67,026,944 bytes. V8 used-heap medians were 229,511,432 and
218,242,408 bytes; tracked GPU medians 18,528,464 and 18,550,064 bytes. These gauges
overlap with RSS and are not an accounting decomposition or additional savings.
System-allocator Rust allocation counters remain unavailable.

The native comparison independently established removal of 46.141MiB of selected
runner widget/location allocations. It does not explain every byte in the clean
RSS difference. The earlier corrected active repeats varied by 211.789MiB in
median RSS, albeit over longer windows; therefore this one short pair alone
does not show that its RSS delta exceeds repeat noise. Keep the candidate
unmerged pending the required acceptance evidence. If resolving performance
acceptance next, repeat a short pair in reverse order before attributing the
CPU/draw-work difference to code. No whole-branch grok-4.6 approval is claimed.

## Operator navigation observation

During the pair the operator observed **bots** bouncing against a bank booth
rather than approaching directly adjacent. The original observation is retained
in the before-run directory and the JSON report. Detailed navigation traces and
screenshots were disabled; script progress/readiness/timing records do not
establish the cause. No navigation settings or instrumentation changed mid-pair.
This needs a separate focused approach-path diagnostic if pursued.
