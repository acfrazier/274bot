# Shared benchmark navigation and Stop storage — reassessment

Production host navigation was already shared across slots. The duplication was
in benchmark seed runners. The two authorized corrections are implemented in
separate commits:

- `7d026bc`: one navigation load per benchmark runner set, injected through
  `ScenarioRunner::with_world`. Each runner keeps independent mutable state;
  missing-pack handling remains `None`. Unseeded idle still loads no seed pack.
  The host retains its own already-shared world.
- `e65103b`: explicit Stop joins the isolate, runs compiled teardown, then drops
  fingerprint, world identity and encoder. Pause/resume, logs and errors retain
  their behavior. Subsequent encoding produces a full keyframe.

Grok-4.5 approved each commit separately. Red regressions reproduced ignored
world sharing and retained fingerprints. Green verification: 134 host-play unit
tests plus enabled integration tests with memory-profile-no-alloc, 76 scenario
tests (including missing-world behavior), and the full script suite with load
(38 unit tests plus integration suites). The Stop test pressures the encoder
with a 1MiB string, checks backing capacity zero after Stop, retained logs/errors,
pause/resume deltas, restart keyframes and validity of an earlier owned packet.
No client source or JavaScript surface changes were made.

## Reassessment configuration

Saved build `diagnostics/shared-stop-build/panel-play-system`, SHA-256
`616196ac16974c67ef331b107d17a87f907b963a7fc7e3256ef96d3754136c9b`.
Host launch `6a25d11`, host sources
`9a4089c2ca3351decfed6a2351df58c2105932cac2a6ebd3c38783224bde05e0`;
client `451759f2a7df9c57895657d5b8d506172860cee1`, unchanged. Cache inputs,
catalog and nav pack match prior controls; full metadata is in the tracked JSON.

Two fresh sequential panel processes, each with one drawing slot and 31 other
slots, unique local accounts, 120s warmup, 600s observation and 60s teardown.
System allocator; allocation counts unavailable. Scheduling counters enabled;
stack logging, screenshots and verbose diagnostics disabled. No builds,
reviewers or intrusive profiling overlapped observation or teardown.

| Workload | Raw diagnostic directory | Qualification |
|:---|:---|:---|
| Seeded idle | 20260906T122440Z_panel_n32_seeded-idle | ready32/active0 throughout, all clients at guard stand at both boundaries, exit0 |
| Sustained Thiever | 20260906T124019Z_panel_n32_active | ready32/active32 throughout, gains48–92 per bot, no banking final status or script errors, exit0 |

Periodic observation spans are 598.095s and 599.332s; both boundary records are
present in each run. Idle still uses the host's parked simulation cadence.
All script/isolate/in-flight snapshot gauges are zero during final teardown,
with 32 clients connected. Retained RSS is not shutdown memory.

## Memory results

MiB uses powers of 1024. Native/Rust/V8/GPU/RSS categories overlap and must not
be summed. No percentage savings or separate Stop RSS benefit is claimed.

| Metric | Earlier seeded idle | Corrected seeded idle | Earlier active (3 runs) | Corrected active |
|:---|---:|---:|---:|---:|
| Median RSS, MiB | 3400.313 | 1180.563 | 3837.258–3900.578 | 1618.656 |
| Lifetime peak RSS, MiB | 3407.219 | 1186.641 | 3938.062–4047.969 | 1749.453 |
| Final teardown30 median RSS, MiB | 3405.344 | 1184.734 | 3607.016–3648.367 | 1419.438 |

Idle median RSS fell by **2219.750 MiB** (about 2.168GiB). Neither idle run loaded
scripts, so the observation isolates the benchmark correction from Stop storage
release. This is one before/after pair, with repeats still required for baseline
acceptance. The scale of the change agrees with the native duplicate ownership
finding; it is not a production-client optimization claim.

Corrected active V8 used heap median was 154.150MiB. Tracked GPU median bytes were
18,292,776 idle and 18,433,784 active, excluding untracked driver allocations.
Largest sampled active in-flight snapshot was 303,320 bytes (median/p95 zero);
sampled maxima are not continuous peaks. After Stop V8 used and in-flight
bytes/capacity were zero. Actual fingerprint/encoder release is established by
the regression and code; no paired native Stop allocation snapshot was collected
in these clean runs. Both fixes are present in the active build, so its RSS
change cannot isolate Stop's contribution.

Corrected active teardown RSS still exceeds corrected idle teardown by
**234.703MiB**. Activity history, scene state and allocator retention remain
possible contributors; this is not proof of a leak. First-to-last observation
minute RSS change was +6.320MiB idle and -75.328MiB active (the active run started
higher and then settled); a lifecycle soak is still needed.

## Performance caveat

Idle CPU averaged 0.325 cores versus 0.351 before; client throughput was
3.189 ticks per slot-second versus 3.067. Active CPU averaged **0.826 cores**,
versus 0.731–0.741 before; throughput was **41.186** versus 38.624–38.765 ticks per
slot-second. Mean active client-tick work was **0.850ms** versus 0.694–0.714ms,
script work 0.113ms versus about 0.108–0.109ms, and UI-frame work 0.961ms versus
0.854–0.865ms. The active results do not establish CPU/latency equivalence.

Corrected simulation mean work/start interval: 0.709/24.305ms; drawing:
5.108/23.634ms. There were no observation work overruns in either group.
Batched counters may lag49 cycles per slot and means do not establish tail
latency. The CPU/work increase must be repeated and investigated before claiming
full performance acceptance; more ticks, different stochastic activity and
machine/server conditions are potential confounders, not established causes.

## Independent native cardinality check

After both clean measurements exited, a separate short diagnostic
`20260906T125706Z_panel_n32_seeded-idle` ran `heap -s --showSizes --noContent`
during setup. It found **63616KiB[2] and 7968KiB[2]**, versus [33] for each size
in the previous duplicate-world capture: one runner-set world plus one host
world. Heap allocator-rounded block sizes differ from malloc_history request
sizes. This cross-check supports the sharing test without instrumenting the
clean runs. It was intentionally terminated after capture (exit -15), and is
not a scale or baseline pass. Raw heap report and operator note are retained.

## Remaining acceptance work

Repeat corrected active runs to resolve CPU/work variation, repeat corrected
controls and establish new one-bot costs, then perform the longer lifecycle soak.
Standalone client-play remains available as a cross-check (cargo check passed
in the client workspace); it needs matched scene/render/audio/memory settings.
The retained 234.7MiB activity difference needs allocation-owner evidence. No
renderer redesign, queue coalescing or V8 limits were changed. No whole-branch
grok-4.6 review or full campaign completion is claimed.
