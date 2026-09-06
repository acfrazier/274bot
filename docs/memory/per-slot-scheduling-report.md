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
| `work_overruns` | Work duration > 20 ms budget |
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
| `updated_ms` / `sample_age_ms` / `ended` | **Publish freshness only** (registry flush wall). Not a sample endpoint. |
| `drawing` | Last recorded `client.draw` latch |
| `cycle_n` | Actual `record` iterations this generation (flush-consistent snapshot) |
| `drawing_cycle_n` / `non_drawing_cycle_n` | Split of `cycle_n` |
| `work_*` / `work_overrun_n` | Work/sleep totals and overrun count for this slot |
| `interval_n` / `interval_ns` / `interval_buckets` | **Absolute** start-to-start samples |
| `interval_bound_ms` | Fixed inclusive upper bounds (see below) |
| `interval_p99_upper_bound_ms` | Conservative p99: upper bound of the bucket that first reaches cum ≥ ceil(0.99·n); `null` if empty or overflow bucket |
| `interval_coverage_complete` | `cycle_n == interval_n + mode_break_n + anchor_miss_n` |
| `mode_break_n` | Drawing latch flipped while an anchor existed (excluded, not an interval) |
| `park_n` | `parked()` calls that cleared the start anchor |
| `anchor_miss_n` | Cycles with no prior anchor (first after start / park / prior break) |
| `first_interval_ms` | Wall ms of the **start Instant** of the first sampled interval (opening tick start) |
| `last_interval_ms` | Wall ms of the **end Instant** of the last sampled interval (= closing tick start) |
| `first_interval_start_mono_ms` / `last_interval_end_mono_ms` | Local-origin monotonic ms of those same Instants (precise Instant math) |
| `first_cycle_ms` / `last_cycle_ms` | Wall ms of first/last tick **start** Instant (not post-sleep `record` entry) |
| `first_cycle_mono_ms` / `last_cycle_mono_ms` | Local-origin mono of first/last tick start |
| `sleep_excess_buckets` / `interval_excess_buckets` | Same 6-bucket excess schema as legacy, on this slot |
| `ended_lost_n` | Process-wide count of ended rows discarded under the unread-ended cap |
| `flush_lag_max_cycles` / `flush_lag_max_ms` | Published bounds: 50 cycles / 1000 ms (also dirty on park/mode) |
| `scene_transition_separation` | Always `"unavailable"` (fail-closed) |
| `jsonl_row_time_is_not_interval_endpoint` | `true` — never treat JSONL row time as sample span |

**Interval endpoint rules (precise observe windows):**

1. Host `run_client` calls `record(start, …)` **after** work + leftover sleep.
2. Endpoints are still the tick **`start` Instant** values passed into `record`,
   not the wall clock at `record` entry.
3. Wall endpoint ≈ `wall_now − (Instant::now() − start)` at `record` entry
   (backdated). Mono endpoint = `start.duration_since(Local.origin)` in ms.
4. For an interval sample pairing previous start P with current start C:
   - interval length = `C − P` (Instant Duration → histogram)
   - sample window start endpoint = P (wall + mono stored when P was recorded)
   - sample window end endpoint = C (wall + mono of current start)
5. `first_interval_ms` = wall of the first sample’s opening tick start.
   `last_interval_ms` = wall of the last sample’s closing tick start.
6. `updated_ms` is set only on registry publish and must stay distinct from (5).
7. **JSONL row wall time ≠ interval endpoints.** Adapters must not use
   first/last JSONL timestamps or row Δt as the elapsed sample span.

**Wholly-inside vs boundary-crossing:**

- Prefer mono endpoints when available: an observation window `[W0, W1]` (mono
  or aligned wall) contains a sample set only when
  `first_interval_start_mono_ms ≥ W0` and `last_interval_end_mono_ms ≤ W1`
  on the same generation, using counter deltas between two flush-consistent
  snapshots.
- Intervals that open before W0 or close after W1 are boundary-crossing —
  exclude them from the observe-window claim (do not invent partial intervals).

**Flush lag vs observation deltas:**

- Local counters may advance up to **`flush_lag_max_cycles` (50)** or
  **`flush_lag_max_ms` (1000)** before the next registry publish, and also flush
  immediately on park/mode dirty and drop.
- Published rows are whole `SlotObservation` clones (consistent counters +
  histograms + endpoints together). Unread local progress is bounded by the
  lag constants above; it is **not** visible until flush.
- Derive observe deltas from monotonic counters (`cycle_n`, `interval_n`,
  buckets) between two reads **with** the endpoint stamps on those snapshots.
  **Never** treat nominal JSONL Δt / first–last JSONL row times as the
  observation window.

**Interval pairing rules (steady vs excluded):**

1. Sample = duration from previous tick **start** Instant to this start.
2. Pair only when previous anchor exists **and** `drawing` matches.
3. `parked()` (idle park / focus sleep path in `run_client`) clears the anchor and
   increments `park_n`. The next cycle is an `anchor_miss`, never a park-spanning
   interval. Interval endpoints do not advance across the park gap.
4. Drawing latch flip with an anchor present is a `mode_break` (focus / draw
   transition). The flip cycle becomes the new anchor for the new mode; it is
   not mixed into the previous mode’s histogram.
5. **Scene transition separation: unavailable.** `record` only receives the
   drawing bool — not `scene_state`. Scene rebuilds that leave `client.draw`
   unchanged can still pair intervals. JSON always emits
   `scene_transition_separation: "unavailable"`. Do **not** treat buckets as
   steady-scene proof; qualify scene-sensitive claims via `renderer_profile` /
   other signals, not these histograms.

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

## Design bounds

| Bound | Value |
|:---|:---|
| Unread ended rows | `MAX_ENDED_UNREAD = 64`; excess dropped with `ended_lost_n` |
| Flush lag | `FLUSH_LAG_MAX_CYCLES = 50`, `FLUSH_LAG_MAX_MS = 1000`, plus park/mode dirty |
| Hot path | Local counters only; registry lock on flush cadence above — **not** every tick |
| History | No unbounded ring of intervals; fixed histogram + counters |
| Disabled | Cheap atomic false; no Local / no registry push |
| Scheduler | Unchanged 20 ms leftover sleep |

Legacy 49-cycle batch lag applies **only** to process-wide `scheduling` groups.
Per-slot published rows are whole `SlotObservation` clones (consistent
count/histogram/endpoint snapshot). Do not claim group totals are exact
per-slot at a JSONL boundary.

## JSON endpoints

- `scheduling`: legacy array or `null` (schema/meaning preserved).
- `scheduling_slots`: array of per-slot objects or `null` while profiling off.

## Files

- `crates/host/src/cadence.rs` — legacy groups + per-slot registry/tests
- `crates/host/src/lib.rs` — `Local::new(slot_id_for(username))` hook (record after work+sleep; endpoints still use tick `start`)
- `crates/host-play/src/memory.rs` — enable + `scheduling` / `scheduling_slots` emit
- `docs/memory/per-slot-scheduling-report.md` — this report

## Tests run (this task)

- `cargo test -p host --lib cadence -- --test-threads=1`
- `cargo test -p host --lib`
- `cargo test -p host-play --lib --features memory-profile-no-alloc`

Coverage in `cadence` unit tests: boundary buckets (incl. sub-20 ms and
`40ms+1ns`), park/resume and drawing-mode exclusion, **mono/wall endpoint
alignment with synthetic start Instants**, park/mode not extending endpoints
across gaps, `updated_ms` publish freshness vs sample endpoints, restart
generation + ended prune, registry bound + `ended_lost_n`, disabled path,
coverage identity, legacy merge still works, p99 bucket upper bounds,
scene-separation constant fail-closed.

No live/native/acceptance runs (Linux compile active). No scheduler timing
change. Overhead and clean p99 budget proof are **out of scope** for this card.

## Needed next (not this card)

- Adapter/tooling support to read `scheduling_slots` and compute observe-window
  per-slot interval p99 upper bounds using counter deltas + mono/wall endpoints
  (never JSONL Δt).
- Frozen candidate/reference enabled controls before clean paired runs.
- Separate matched enabled/disabled overhead measurement.
