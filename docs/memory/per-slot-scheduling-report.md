# Per-slot scheduling interval measurements

Opt-in extension of `host::cadence` that records **bounded per-slot** tick
start-to-start intervals alongside the existing process-wide drawing /
non-drawing groups. Measurement prerequisite only — does **not** change the
20 ms sleep budget, sleep calls, park policy, socket wait, or gameplay.

Code review validates measurement semantics. It is **not** p99 budget acceptance
or overhead acceptance. Overhead is unmeasured here and must be a separate
matched enabled/disabled cell later.

## Enablement

- Default **off**. `Local::new` → `None`; `read` / `read_slots` → `None`; JSON
  `scheduling: null` and `scheduling_slots: null`.
- Env: `BOT_SCHEDULING_PROFILE=1` (launcher `--scheduling-profile`), same flag as
  legacy groups. `Run::prepare*` calls `host::cadence::enable()` before slots start.
- No separate flag: one enable arms both legacy groups and per-slot rows.

## What is measured

### Legacy groups (`scheduling` array) — unchanged

Process-wide drawing vs non-drawing aggregates:

| Field | Meaning |
|:---|:---|
| `cycles` / `work_ns` / sleep ns | Cumulative work and sleep |
| `work_overruns` | Work duration &gt; 20 ms budget |
| `intervals` / `interval_ns` | Same-drawing consecutive start-to-start (excess-oriented totals) |
| `sleep_excess_buckets` / `interval_excess_buckets` | Excess over requested sleep / over 20 ms; bounds 1,2,5,10,20 ms + overflow |

Thread-local batch still merges every **50** cycles (+ drop). Cumulative process
groups **may lag by up to 49 cycles per live slot**. Do not treat a single JSONL
row’s group totals as an exact per-slot boundary.

### Per-slot rows (`scheduling_slots` array)

| Field | Meaning |
|:---|:---|
| `slot_id` | FNV-1a 64 of username (stable identity; string not retained) |
| `generation` | Monotonic lifecycle id; restart = new generation, same `slot_id` |
| `updated_ms` / `sample_age_ms` / `ended` | Publish freshness; ended generations publish once then prune on read |
| `drawing` | Last recorded `client.draw` latch |
| `cycle_n` | Actual `record` iterations this generation |
| `drawing_cycle_n` / `non_drawing_cycle_n` | Split of `cycle_n` |
| `work_*` / `work_overrun_n` | Work/sleep totals and overrun count for this slot |
| `interval_n` / `interval_ns` / `interval_buckets` | **Absolute** start-to-start samples |
| `interval_bound_ms` | Fixed inclusive upper bounds (see below) |
| `interval_p99_upper_bound_ms` | Conservative p99: upper bound of the bucket that first reaches cum ≥ ceil(0.99·n); `null` if empty or overflow bucket |
| `interval_coverage_complete` | `cycle_n == interval_n + mode_break_n + anchor_miss_n` |
| `mode_break_n` | Drawing latch flipped while an anchor existed (excluded, not an interval) |
| `park_n` | `parked()` calls that cleared the start anchor |
| `anchor_miss_n` | Cycles with no prior anchor (first after start / park / prior break) |
| `first_*_ms` / `last_*_ms` | Wall timestamps of first/last cycle and first/last **interval sample** (0 if none) |
| `sleep_excess_buckets` / `interval_excess_buckets` | Same 6-bucket excess schema as legacy, on this slot |
| `ended_lost_n` | Process-wide count of ended rows discarded under the unread-ended cap |

**Interval pairing rules (steady vs excluded):**

1. Sample = duration from previous tick **start** Instant to this start.
2. Pair only when previous anchor exists **and** `drawing` matches.
3. `parked()` (idle park / focus sleep path in `run_client`) clears the anchor and
   increments `park_n`. The next cycle is an `anchor_miss`, never a park-spanning
   interval.
4. Drawing latch flip with an anchor present is a `mode_break` (focus / draw
   transition). The flip cycle becomes the new anchor for the new mode; it is
   not mixed into the previous mode’s histogram.
5. Scene transitions that do not change `client.draw` are **not** labeled
   separately here (unlike paint-cadence mode). Simulation start-to-start still
   pairs across scene rebuild if the slot stays on the 20 ms loop without park
   and without a drawing flip. Document scene-sensitive claims via
   `renderer_profile` / qualification, not by overloading these buckets.

**Not measured:** changing scheduler policy; GPU/present cadence; script
dispatch latency (see `responsiveness_profile`).

## Histogram bounds and quantile semantics

```text
INTERVAL_BOUNDS_MS =
  [5, 10, 15, 16, 17, 18, 19,
   20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30,
   31, 32, 33, 34, 35, 36, 37, 38, 39, 40,
   42, 45, 50, 100, 250, 500, 1000]
+ overflow (no finite upper bound)
```

- Membership uses full `Duration` comparison (not floored integer ms).  
  `40ms + 1ns` is **outside** the ≤40 ms bucket.
- 1 ms steps from 18–42 ms support the 20 ms target and the plan’s **p99 ≤ 40 ms**
  gate with ~2 ms paired-comparison headroom on the bound edge.
- `interval_p99_upper_bound_ms` is a **conservative bucket upper bound**, not an
  interpolated percentile. Overflow → `null` (unavailable precise p99).
- Observe-window deltas: subtract monotonic counters (`cycle_n`, `interval_n`,
  buckets) between two samples with `interval_coverage_complete` true on both
  ends when claiming full coverage. Use `first_interval_ms` /
  `last_interval_ms` for the elapsed wall span of interval samples only.

## Design bounds

| Bound | Value |
|:---|:---|
| Unread ended rows | `MAX_ENDED_UNREAD = 64`; excess dropped with `ended_lost_n` |
| Hot path | Local counters only; registry lock on ~1 s / dirty park-mode / every 50 cycles / drop — **not** every tick |
| History | No unbounded ring of intervals; fixed histogram + counters |
| Disabled | Cheap atomic false; no Local / no registry push |
| Scheduler | Unchanged 20 ms leftover sleep |

Legacy 49-cycle batch lag applies **only** to process-wide `scheduling` groups.
Per-slot published rows are whole `SlotObservation` clones (consistent
count/histogram snapshot). Do not claim group totals are exact per-slot at a
JSONL boundary.

## JSON endpoints

- `scheduling`: legacy array or `null` (schema/meaning preserved).
- `scheduling_slots`: array of per-slot objects or `null` while profiling off.

## Files

- `crates/host/src/cadence.rs` — legacy groups + per-slot registry/tests
- `crates/host/src/lib.rs` — `Local::new(slot_id_for(username))` hook (record/park unchanged structurally)
- `crates/host-play/src/memory.rs` — enable + `scheduling` / `scheduling_slots` emit
- `docs/memory/per-slot-scheduling-report.md` — this report

## Tests run (this task)

- `cargo test -p host --lib cadence -- --test-threads=1`
- `cargo test -p host --lib`
- `cargo test -p host-play --lib --features memory-profile-no-alloc`

Coverage in `cadence` unit tests: boundary buckets (incl. sub-20 ms and
`40ms+1ns`), park/resume and drawing-mode exclusion, restart generation + ended
prune, registry bound + `ended_lost_n`, disabled path, coverage identity,
legacy merge still works, p99 bucket upper bounds.

No live/native/acceptance runs (Linux compile active). No scheduler timing
change. Overhead and clean p99 budget proof are **out of scope** for this card.

## Needed next (not this card)

- Adapter/tooling support to read `scheduling_slots` and compute observe-window
  per-slot interval p99 upper bounds.
- Frozen candidate/reference enabled controls before clean paired runs.
- Separate matched enabled/disabled overhead measurement.
