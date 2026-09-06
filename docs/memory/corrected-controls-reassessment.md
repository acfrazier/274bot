# Corrected controls and remaining owners

The corrected 32-active repeat and both one-bot controls completed successfully.
The first corrected run's CPU increase did not repeat. Memory variation remains
large enough that these measurements establish a provisional scale estimate,
not full performance acceptance or a stable three-repeat baseline.

## Evidence and configuration

All five cells below use saved System-allocator binary SHA-256
`616196ac16974c67ef331b107d17a87f907b963a7fc7e3256ef96d3754136c9b`.
Host/client source digests, navigation-pack digest and catalog commit match.
The first two launched at `6a25d11`; the three new cells launched at `747e81e`
with clean tracked trees. Only documentation changed between those launches.
Full metadata and summaries are retained in `corrected-controls-reassessment.json`.

Fresh sequential panel processes, unique local accounts, one drawing slot,
120s warmup, 600s requested observation, and 60s teardown. Other active slots
simulate; idle simulation slots use the existing parked cadence. Scheduling
counters are enabled; stack logging, verbose diagnostics and captures are off.
No builds, reviewers or intrusive allocation profiling overlapped the new runs.
Rust allocation counters are unavailable in this build; RSS, V8 and GPU values
overlap and must not be summed.

| Cell | Diagnostic directory | Qualification |
|:---|:---|:---|
| Idle32 | 20260906T122440Z_panel_n32_seeded-idle | Prior corrected control, ready32/active0 |
| Active32 A | 20260906T124019Z_panel_n32_active | Prior corrected run, gains48–92 |
| Active32 B | 20260906T141303Z_panel_n32_active | Repeat, ready32/active32, gains44–91 |
| Idle1 | 20260906T142954Z_panel_n1_seeded-idle | ready1/active0 |
| Active1 | 20260906T144306Z_panel_n1_active | ready1/active1, gain53 |

All cells qualified with empty error lists and exit0. New sampled observation
spans are 599.124s, 599.003s and 598.836s, with start/end proofs present.
Final teardown has clients ready, active0, live isolates0, V8 used0 and in-flight
snapshot bytes/capacity0. Retained RSS is measured with clients still connected.

## Measurements

MiB is 1,048,576 bytes. Peaks cover the entire process lifetime, including setup
and transitions; teardown is the median of its final 30 sampled seconds.

| Cell | Median RSS MiB | Lifetime peak MiB | Teardown MiB | CPU cores | Client ticks/slot/s | Mean client tick ms |
|:---|---:|---:|---:|---:|---:|---:|
| Idle32 | 1180.563 | 1186.641 | 1184.734 | 0.325 | 3.189 | 4.361 |
| Active32 A | 1618.656 | 1749.453 | 1419.438 | 0.826 | 41.186 | 0.850 |
| Active32 B | 1830.445 | 1937.531 | 1590.625 | 0.645 | 42.496 | 0.558 |
| Idle1 | 594.891 | 599.078 | 597.133 | 0.468 | 43.885 | 7.351 |
| Active1 | 633.328 | 642.422 | 626.742 | 0.376 | 43.347 | 6.291 |

Aggregate tick means at N1 and N32 weight drawing and simulation differently;
they are not directly comparable per-renderer latency. Idle1 CPU exceeding
Idle32 also cautions against deriving a CPU slope from these individual cells.
Active32 B simulation work averaged 0.414ms, interval 23.553ms; drawing work 4.905ms,
interval 22.881ms. There were three simulation work overruns across 788,750 cycles,
and zero drawing overruns; A had zero in both groups. Means and batched counters
do not establish tail-latency equivalence.

Compared with A, B's median RSS is 211.789MiB higher despite lower CPU/tick work.
Its first-to-last observation-minute RSS medians rise 41.250MiB, versus A's
75.328MiB decline. V8 used-heap medians are 154.150MiB and 178.521MiB respectively;
tracked GPU bytes are 18,433,784 and 18,550,568. These are separate gauges, not
an accounting decomposition of the RSS difference. Sampling/GC timing, activity,
scene state and allocator retention require evidence before assigning causality.

## Provisional growth per additional bot

Finite differences use `(RSS32 - RSS1) / 31` on observation medians, with the
same one-drawing-slot policy. Both sizes retain two navigation packs, so the
former per-runner pack duplication no longer contaminates this growth estimate.
Other benchmark-owned state remains included; these are not pure client-object
sizes or projections to 1000 bots.

- Idle: **19,810,370 bytes / 18.893MiB** per additional bot.
- Active: **33,328,756–40,492,527 bytes / 31.785–38.617MiB** per additional bot,
  pairing the single Active1 control with each Active32 run.
- After active Stop: **26,812,945–32,603,367 bytes / 25.571–31.093MiB** per
  additional connected bot, using teardown medians.

These ranges are observed finite differences, not confidence intervals. Each
one-bot control and Idle32 has only one corrected sample. TUI, repeated controls,
third Active32 and the lifecycle soak remain outstanding; N128 remains deferred
under the operator's direction.

## Remaining owners and next bounded investigation

1. **Client/scene and benchmark snapshot storage.** Idle growth remains 18.9MiB
   per bot. The earlier native report identified 32 client-construction blocks
   of 4,325,376 bytes each, plus scene and API allocations; exact fields remain
   unassigned. Completed seed runners also retain their `GameSnapshot`.
   Next collect a separate corrected idle allocation capture and attribute the
   largest live groups to concrete fields, distinguishing host, client and runner
   ownership. Preserve runner proof data. A standalone client-play comparison
   must match scene, rendering, memory and audio settings before interpreting it.
2. **Script snapshot churn.** Current `IsolateBuf::encode_snapshot_delta` still
   constructs an owned fingerprint every encode and `copy_finished` allocates a
   new output vector. Prior native evidence identifies a 512KiB builder and
   339,840-byte scene-fingerprint vector in the one-bot diagnostic. This supports
   the planned borrowed comparison and consumed-buffer pool as bounded candidates,
   but does not quantify their steady RSS benefit or current allocation rate.
   Profile allocation traffic separately; preserve exact delta semantics and
   packet lifetime tests before implementing each independently.
3. **Retention after activity.** Active32 teardown exceeds Idle32 by 234.703–
   405.891MiB despite zero isolate/in-flight gauges. Obtain successful paired
   native active/Stop allocation captures and then a lifecycle series to separate
   live owners from allocator-held memory. Earlier post-Stop stack capture timed
   out; these RSS measurements cannot replace it or establish a leak.
4. **Heightmap sharing remains a small concrete candidate.** The current nested
   `Vec` representation duplicates 4×105×105 `i32` values: 176,400 bytes (0.168MiB)
   of payload per client, plus vector/allocator overhead. Client clones initially
   and publishes through `world.groundh.clone_from` at the existing map boundary.
   Preserve that boundary with CoW and independent-client tests if pursued;
   the payload estimate makes clear it cannot explain most of the remaining RSS.

No owner optimization or runtime change is made in this reassessment. The next
useful step is allocation attribution, not further broad scale expansion.
No full campaign acceptance or whole-branch grok-4.6 approval is claimed.
