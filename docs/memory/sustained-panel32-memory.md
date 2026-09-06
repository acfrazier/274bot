# Sustained panel32 memory run — 2026-09-06

The corrected build completed the requested 32-bot active workload: all 32 slots
remained ready and active for the full ten-minute observation, every bot gained
47–83 steals, and all final task statuses were pickpocketing, eating or waiting
out stun. No script was stopped by an error or left in a banking status. Stop
released all live isolates and in-flight snapshots. Process exit 0.

This is one usable measurement cell, not repeated baseline acceptance or proof
of savings. No percentage improvement or bytes-per-additional-bot claim follows
from a single active 32 run without matched idle/1-bot controls and repeats.

## Configuration and provenance

Run: `diagnostics/20260906T023123Z_panel_n32_active`.
Host `5e0f3ec52adc314ddc13a40ad8b3c6bb67068774`, clean tracked sources at launch;
client `451759f2a7df9c57895657d5b8d506172860cee1`.
Saved binary: `diagnostics/memory-resume-build/panel-play-system`, SHA-256
`781b93eb3f9b64eb3dbeed6fa3a0c46660b36ecb9fa995ebae3b505c2d8bf76f`.
Nav pack SHA-256
`2f393138c905aaf1b2db4f77442426db01ff2dfad5ee575d77454012d27a4a30`.
Full source hashes and launch parameters are in `metadata.json` and the tracked
`sustained-panel32-memory.json` summary.

Panel, one drawing slot and 31 world-simulation slots, sustained Thiever fixture,
unique accounts, local server. The gate required all slots ready and scripts
active; setup took about 155 seconds. Then warmup 120s, observe 600s, teardown 60s.
The sampled first-to-last observation interval is 598.195s because samples are
periodic; both qualification boundaries were written. System allocator,
allocation counting off, verbose diagnostic sidecar off, debug off, captures
off, scheduling counters on. No stack sampler, builds or reviews overlapped the
observation. This cell extends the previously planned short scheduling profile
to a ten-minute memory observation without a stack-sampling interruption.

## Memory

These are separate metrics with potentially overlapping ownership; do not sum
them. GiB and MiB use powers of 1024. Sampled maxima for queues/heaps are not
continuous peak counters.

| Metric | Result |
|:---|---:|
| Current resident memory, observation median | 3.747 GiB |
| Current resident memory, observation range | 3.686–3.854 GiB |
| Lifetime peak resident memory, whole process | 3.855 GiB |
| Resident memory after Stop, final 30s median | 3.527 GiB |
| RSS reduction, last observation minute to final teardown 30s medians | 331.063 MiB |
| V8 used heap, median / largest sample | 184.369 / 263.885 MiB |
| V8 total heap, median / largest sample | 236.875 / 359.500 MiB |
| In-flight snapshot bytes, median / largest sample | 0 / 0.533 MiB |
| Explicitly tracked GPU allocations, constant during observation | 17.703 MiB |
| Rust allocation counts, allocated bytes and live bytes | unavailable in System build |

All 32 live isolates contributed V8 samples throughout observation; the largest
reported sample age was 1225ms. GPU tracking contains 1,318,528 buffer bytes and
17,243,992 texture bytes; it does not measure all driver/device allocations.
After Stop, active scripts, live isolates, V8 used bytes, queued snapshot bytes
and queued snapshot capacity were all zero. All 32 clients remained connected,
so the 3.527 GiB retained RSS is not an application-shutdown measurement. It
also cannot be assigned wholly to client ownership without allocation evidence.

First-minute median RSS was 4,053,794,816 bytes; last-minute median was
4,134,043,648 bytes, a 76.531 MiB increase (about 2%). This drift deserves repeat
or soak evidence; this run alone does not distinguish caching, allocator
retention, heap cycles or a leak. It should not be hidden by reporting only the
whole-window median.

## Scheduling and latency

| Group | Work ms/cycle | Requested sleep ms | Actual sleep ms | Start interval ms | Work overruns |
|:---|---:|---:|---:|---:|---:|
|31 simulation slots|0.516|19.484|25.309|25.826|0|
|1 drawing slot|6.003|13.997|18.838|24.841|0|

Process CPU averaged 0.731 cores. Client tick throughput was 38.765 ticks per
slot-second. Measured mean client-tick work was 0.694ms, script-tick work 0.108ms,
and UI-frame work 0.854ms; these counters measure different scopes from the
outer loop work above. No frame/tick percentile claim is made from averages.
Scheduling counters publish in per-slot batches and can lag by 49 cycles.

Work did not overrun the 20ms budget. Actual sleep exceeded requested sleep by
about 5.83ms per simulation cycle and 4.84ms per drawing cycle. This reinforces
the late-wakeup finding from the earlier failed short profile, without treating
that failed run as a valid performance baseline or claiming an OS-level cause.

## What this supports next

We can now measure memory changes against a functioning 32-bot scenario. Repeat
this configuration to establish variability, then use a matched idle/1-bot cell
for incremental ownership costs. The large RSS remaining with scripts stopped
makes retained host/client storage worth profiling; V8-limit or renderer
redesign is not justified by these figures. Track the observed RSS drift in the
planned lifecycle soak. N=128 remains deferred as agreed with the operator.

Raw samples, qualification boundaries, launch manifest and the analysis script
are retained under the run directory. `sustained-panel32-memory.json` preserves
the numeric summary. This report makes no source changes and does not replace
the campaign's final whole-branch grok-4.6 review.
