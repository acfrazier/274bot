# Windows N16 lazy-upload comparison report

Date: 2026-09-08
Status: diagnostic evidence only; performance_acceptance is false.

## Scope and provenance

This report analyzes the four immutable archived cells named by
`windows-lazy-upload-comparison-protocol.md`. It does not launch a process,
perform a native/network operation, or alter source, controls, or archives.
`analysis.py` reads the archived JSONL, binding results, archive manifests and
nested renderer records; it writes the machine-readable `table.json`.

The protocol SHA-256 recomputed by the tool is
`5f5aeb3fba0de48d16a9100a5a00f8b0d00c83b2e8fb515aac4787ab612e8d2d`.
Every archive-manifest-listed file exists with the listed length and SHA-256
for all four cells (28, 22, 28 and 22 files respectively). Tarball SHA-256
recomputation agrees for the three archives below. The focused baseline tar
recomputed as
`637b594951873aa01f05977bc4d654282cd0630313db868e573347f6688a5694`, while
the task-supplied expected value is
`637b594951873aa01f05977bd4c654282cd0630313db868e573347f6688a5694`;
this one-character/order mismatch is retained as a provenance discrepancy,
not silently repaired. The archive-manifest file checks still pass.

The binding files independently report `status=bound`, `binding_ok=true`,
`pair_eligible=false`, and no final acceptance for every cell. All four
completion artifacts report exit 0. The workload qualification records contain
16 slots with positive steal progress in each cell; this is workload
qualification, not a resource, latency, rendering, or acceptance verdict.

Build provenance is role-bound in each manifest: baseline host `36825a9`,
candidate host `3118e96`, common client `5ee9b6e`, and runtime/tools
`48tools3a25` are treated separately. The generated metadata also records
`performance_acceptance=false` and diagnostic instrumentation. The focused
cells have three navigation captures per the protocol. Background navigation
capture is unsupported and absent by design; no background visual proof is
claimed. The failed `0049` cell is not in these rows and is excluded.

## Per-cell measured summary

Values are from rows whose elapsed time lies between the archived
`observe-start` and `observe-end` qualification records. RSS and peak are host
process values in MiB (binary MiB); CPU is process CPU seconds divided by the
observation elapsed interval. The short screening windows are approximately
120 seconds, not a plateau or final run.

| role / mode | observation rows / seconds | RSS median | RSS first -> last; min..max | peak max | CPU cores | client ticks/slot/s | tracked GPU max | renderer observations |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| baseline / focused-one | 119 / 119.189 | 921.188 | 1028.434 -> 805.855; 805.855..1083.332 | 1111.875 | 0.565803 | 48.66534 | 17.590 | 1 full-rate slot; 119 rows |
| candidate / focused-one | 119 / 119.141 | 807.215 | 966.496 -> 753.262; 753.262..966.496 | 1134.590 | 0.515645 | 48.47845 | 17.526 | 1 full-rate slot; 119 rows |
| baseline / focused-plus-background | 119 / 119.094 | 2190.402 | 2461.598 -> 1567.734; 1567.734..2499.008 | 2510.039 | 0.639874 | 48.69235 | 143.108 | 16 slots; 119 observations each |
| candidate / focused-plus-background | 119 / 119.094 | 2042.781 | 2483.949 -> 1376.762; 1376.316..2483.949 | 2512.039 | 0.647206 | 48.60619 | 143.367 | 16 slots; 119 observations each |

The corresponding candidate-minus-baseline descriptive differences are:

| mode | median RSS | peak max | CPU cores | tracked GPU max | client tick delta |
|---|---:|---:|---:|---:|---:|
| focused-one | -113.973 MiB | +22.715 MiB | -0.050158 | -0.064 MiB | -295 |
| focused-plus-background | -147.621 MiB | +2.000 MiB | +0.007332 | +0.259 MiB | -200 |

These are one short pair per mode. They are not replication-adjusted savings,
regression acceptance, or causal decomposition.

## Timing, simulation and renderer observations

The archived raw phase counts are baseline focused-one 80 seed/29 warmup/119 observe/59 teardown; baseline background 72/30/119/59; candidate focused-one 131/30/119/59; candidate background 89/30/119/59. The exact counts and boundaries are retained in `table.json`.
The observation boundaries are based on the qualification records rather than
assuming that the harness's first or last raw row is an observation boundary.
The measured client cadence is about 48.48--48.69 ticks/slot/s in all four
cells, above the 40-iteration working target in this diagnostic window. This
does not establish p99 start intervals: the selected binding has no accepted
`simulation_intervals`/`simulation_ticks` scalar result. Scheduling interval
histograms remain nested raw evidence and are not collapsed into a fabricated
p99.

Focused-one renderer profiles show one full-rate GPU renderer and no background
renderer. Their maximum nested paint counters are 10,934 baseline and 13,431
candidate; these counters are not scanout or presented-frame measurements.
The profile schema exposes `completed_n`, `gpu_frame_n`, completion coverage,
latency buckets, stable/transition paint counters and interval data, but this
report does not equate paint callbacks with completed hardware frames. The
raw UI counters are likewise construction/delivery counters, not scanout.

Focused-plus-background profiles show 16 renderer records per observation,
with one `full_rate=true` focused renderer and 15 `full_rate=false` renderers.
The configured background policy is 1 fps, but configuration is not a measured
cadence. The archived nested profile contains stable paint counters and
interval fields; it does not provide an accepted per-background-slot physical
presentation rate. Therefore no blanket claim is made that all 15 background
slots crossed a measured 1 fps stage boundary. GPU tracked maxima are reported
as a separate accounting domain and are not subtracted from RSS.

## Falling RSS and timing limits

Both absolute harness-elapsed and observation-relative views show a falling
series in every cell. Relative to the observe-start row, the endpoint changes
are approximately:

- baseline focused-one: 1028.434 -> 805.855 MiB (-222.578 MiB)
- candidate focused-one: 966.496 -> 753.262 MiB (-213.234 MiB)
- baseline focused-plus-background: 2461.598 -> 1567.734 MiB (-893.863 MiB)
- candidate focused-plus-background: 2483.949 -> 1376.762 MiB (-1107.188 MiB)

The same samples are anchored to the absolute harness elapsed values in the
raw JSONL; those starts occur at different wall-clock times and each cell has
its own seed/teardown duration. The aligned observation-relative comparison is
therefore the useful paired view, while absolute elapsed overlap does not make
the runs simultaneous or remove startup/phase confounding. A falling finite
series is not a stationary plateau. It does not identify resident allocation
ownership, reclamation cause, or a causal lazy-upload saving.

The background mode's roughly 1.9--2.1 GiB medians and 2.51 GiB peaks exceed
the panel N=16 targets (768 MiB median and 1 GiB peak). Focused-one medians
also exceed the 768 MiB N=16 median target, while both focused-one peaks exceed
the 1 GiB target. CPU is below the panel 1-core diagnostic target in all four
cells. These observations do not satisfy the final matrix: the 1/16 cells are
single short pairs, not three fresh 600-second observations, and no final
lifecycle, responsiveness, physical scanout, or target-hardware proof exists.

## Per-slot and environment evidence

The qualification records show 16 ready/active slots with positive progress in
each cell. Focused captures are available for scene-ready, bank-arrival and
return-route stages. The archived capture metadata records actual slot state,
scene state, position, inventory/bank fields, temperature/power/provenance
fields where present; background navigation capture is explicitly unsupported.
Capture-time gameplay state is not a universal route or rendering guarantee.

The managed process accounting records explicit role PIDs and stable Windows
creation-filetime identities for bootstrap, controller, game server, launcher
and collector, with no PID reuse reported. Waited-child current RSS and host
pressure are unavailable on Windows and are not fabricated. Collector
sampling overhead and descendant process RSS are outside the measured process
RSS domain. These limits matter when interpreting the large background-mode
RSS difference and prevent a causal stage/owner attribution.

## Conclusion

The candidate is descriptively lower in the one short pair's median RSS in both
modes (-114.0 MiB focused-one; -147.6 MiB focused-plus-background), with nearly
unchanged tracked GPU accounting and focused-one CPU lower but background-mode
CPU slightly higher. The evidence is compatible with a diagnostic delta, but
it does not establish an accepted saving, plateau, causal RSS decomposition,
physical completed-frame cadence, p99 latency, or target completion. The
candidate misses the applicable N=16 RSS median and peak targets in both panel
modes, and the four-cell result remains screening evidence only.
