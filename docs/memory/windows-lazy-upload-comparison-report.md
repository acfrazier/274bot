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
for all four cells (28, 22, 28 and 22 files respectively). Independent
tarball SHA-256 recomputation agrees with all four task-supplied values,
including focused baseline
`637b594951873aa01f05977bc4d654282cd0630313db868e573347f6688a5694`.
`table.json` records `archive_tar_ok=true` for every comparison cell.

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
cells, above the 40-iteration working target in this diagnostic window.
Binding scheduling histograms show available per-slot p99 interval bounds of
28--32 ms (baseline focused-one), 25--30 ms (baseline background), 30--33 ms
(candidate focused-one), and 27--30 ms (candidate background). Focused-one
has 15/16 scheduling slots available; both background cells have 16/16. The
missing focused-one slot and every `scene_transition_separation` value marked
unavailable remain explicit limits. These are offline observation-window
histograms, not a global cross-run alignment or a fabricated p99 start
interval. The selected binding has no accepted `simulation_intervals` or
`simulation_ticks` scalar result.

Focused-one renderer configuration has one present/full-rate GPU renderer and
15 absent renderers in both roles; observed renderer rows are 119 focused plus
15 non-present records per sample. Maximum nested paint counters are 10,934
baseline and 13,431 candidate. Focused-plus-background configuration has 16
present GPU renderers, one full-rate and 15 non-full-rate, in both roles; the
observed profiles contain 16 renderer records per sample. Paint and callback
counters are not scanout or presented-frame measurements. The configured
background policy is 1 fps, but the archived nested profile does not provide
an accepted per-background-slot physical presentation rate, so no blanket
stage-crossing claim is made. Completed-GPU latency diagnostics are available
for 1/16 focused-one slots (p99 40--50 ms, target unproven) and 16/16
background slots (focused p99 40--50 ms; background p99 10--25 ms, with the
target verdicts still including unproven). These are completed-GPU diagnostics,
not physical scanout. GPU tracked maxima remain a separate accounting domain
and are not subtracted from RSS.

## Falling RSS and timing limits

Both absolute harness-elapsed and observation-relative views show a falling
series in every cell. Relative to the observe-start row, the endpoint changes
are approximately:

- baseline focused-one: 1028.434 -> 805.855 MiB (-222.578 MiB)
- candidate focused-one: 966.496 -> 753.262 MiB (-213.234 MiB)
- baseline focused-plus-background: 2461.598 -> 1567.734 MiB (-893.863 MiB)
- candidate focused-plus-background: 2483.949 -> 1376.762 MiB (-1107.188 MiB)

The same samples are anchored to the absolute harness elapsed values in the
raw JSONL and each binding records a native observation wall span. The paired
cells are sequential, so their absolute wall-clock overlap is 0 s in both
modes; absolute elapsed timing therefore supplies no simultaneous control.
The observation-relative first/last/min/max values in `table.json` are the
usable paired descriptive view, while still retaining startup/phase
confounding. A falling finite series is not a stationary plateau. It does not
identify resident allocation ownership, reclamation cause, or a causal
lazy-upload saving.

The background mode's roughly 1.9--2.1 GiB medians and 2.51 GiB peaks exceed
the panel N=16 targets (768 MiB median and 1 GiB peak). Focused-one medians
also exceed the 768 MiB N=16 median target, while both focused-one peaks exceed
the 1 GiB target. CPU is below the panel 1-core diagnostic target in all four
cells. These observations do not satisfy the final matrix: the 1/16 cells are
single short pairs, not three fresh 600-second observations, and no final
lifecycle, responsiveness, physical scanout, or target-hardware proof exists.

## Per-slot and environment evidence

The qualification records show 16 ready/active slots in every cell, and the
focused captures show `ingame=true`, `drawing=true`, and `scene_state=2` at
scene-ready, bank-arrival, and return-route checkpoints in both focused cells.
The focused baseline and candidate captures differ in slot id and snapshot
age (for example scene-ready 641 ms vs 401 ms); they are evidence of the
captured state, not universal route or rendering guarantees. Background
navigation capture is explicitly unsupported and absent.

Recorded environment fields are concrete and mostly matched: all four cells
use canonical cache path `\\?\\C:\\Users\\BotTest\\274bot-server-4c95f87\\data\\pack\\client`,
snapshot `2faf336eeb0462ed`, allocator `std::alloc::System`, and the same
client commit `5ee9b6e`; cache content hash is null at the boundary, so cache
contents are not independently proven equal. AC-line status is 1, computer
system power state 0, battery status 2, and brightness 20 in every cell; the
computer-system value is not substituted for AC-line status. No temperature
field is present. Host provenance differs as expected: baseline commit
`36825a9` / baseline binary SHA, candidate commit `3118e96` / candidate binary
SHA, with stable source digests in each binding. Per-cell values and SHA-256
of each host-conditions file are in `table.json`; those file hashes differ
because each record is cell-specific, not because a source identity mismatch
was inferred. Waited-child current RSS, host pressure, collector overhead, and
descendant RSS remain unavailable.

## Conclusion

The candidate is descriptively lower in the one short pair's median RSS in both
modes (-114.0 MiB focused-one; -147.6 MiB focused-plus-background), with nearly
unchanged tracked GPU accounting and focused-one CPU lower but background-mode
CPU slightly higher. The evidence is compatible with a diagnostic delta, but
it does not establish an accepted saving, plateau, causal RSS decomposition,
physical completed-frame cadence, p99 latency, or target completion. The
candidate misses the applicable N=16 RSS median and peak targets in both panel
modes, and the four-cell result remains screening evidence only.
