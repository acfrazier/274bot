# Reference metrics — bounded offline histogram / observation-window core

Task `t_135ceb9f` (recovery of timed-out `t_8bade183`). **No final performance
or lifecycle acceptance claim.**

## Scope (this card only)

| Deliverable | Role |
|:---|:---|
| `docs/memory/reference_metrics.py` | Core + CLI |
| `docs/memory/test_reference_metrics.py` | Synthetic + one real cell |
| `docs/memory/reference-metrics-report.md` | This report |

No Rust, Docker, STATE, live runs, or builds. Workload qualification reuses
`qualify_control.py` via `--qualify` (not reimplemented). Resource/renderer
**expanded** adapters (RSS rollups, paint-cadence product gates, GPU *interval*
histogram productization, etc.) are **next card** work — not fabricated here.

## What the core does

1. **Bounded JSONL** — metadata + line-iterated `samples.jsonl` (no full schema dumps).
2. **Observation window** — `started_unix + elapsed_s` on first/last `phase=observe`
   samples; optional `--contamination-from` intersection with `[obs_start, obs_end]`.
3. **Cumulative histogram subtraction** — observe-end − observe-start; rejects
   counter resets (any bucket drop), `slot_id` / `generation` mismatches, missing
   start rows, **duplicate (slot_id, generation)** rows, and **disappearing** slots
   present at observe-start but absent at observe-end.
4. **p99 lower/upper** — same host rule as `p99_upper_bound_ms` (`cum*100 >= n*99`);
   reports the coarse bucket edges, never a precise percentile. Empty, width
   mismatch, negative counts, and **overflow** → `status=unavailable`.
5. **Fail-closed integrity** — drops / cancels / lost / pending / coverage flags
   cannot produce a gate pass. **Absent** coverage flags → `coverage_flag_absent`
   (unavailable), not default-true. **Input `visible_ack`** must be an explicit
   end-row dict with `available is True`; absent/malformed → `visible_ack_absent`,
   `available` not True → `visible_ack_unavailable` (no false-pass). **Freshness**
   field enforced: host `sample_age_ms` on end responsiveness / renderer slot
   rows must be present and finite; absent/null/non-finite →
   `sample_age_ms_absent` / `sample_age_ms_invalid` (GPU diagnostic uses the
   parent `renderer_profile` row age; nested `gpu_completion` has no age field).
   Disabled profiles and null input → unavailable.
6. **Scheduling** — excess bins `[1,2,5,10,20]` ms + overflow; absolute interval
   mapping uses fixed **20 ms** budget. **First excess bucket lower absolute bound
   is 0**, not 20: `interval.saturating_sub(budget)` puts sub-budget intervals in
   bucket 0, so ≥20 ms cannot be proven from that bucket. Upper `20 + excess_upper`
   is sound. Groups are **process-wide** drawing/non-drawing with up to **49-cycle
   batch lag** → cannot prove per-slot or exact observation coverage; gate stays
   `unavailable` / `target_verdict=unavailable` even when diagnostic bounds exist.
7. **GPU product gate** — `--require gpu` is the **product frame/cadence** gate.
   It stays `status=unavailable` /
   `reason=product_gpu_frame_cadence_requires_stable_completion_interval` until a
   stable completion **interval**/FPS adapter exists. Fast
   `completion_latency_buckets` (mainredraw→callback delivery) **cannot** product-meet
   40 fps. Latency is scored only under nested
   `diagnostic_completion_latency` with `metric=gpu_completion_latency` — distinct
   from product `gpu`, and **not** accepted by `gate_satisfies_require` /
   `--require gpu`. Never uses host paint as proxy. Serializer
   `stable_completion_interval_buckets` are **raw present**, adapter **deferred**.
8. **CLI**
   - `--inspect` (default path): partial JSON, **exit 0**
   - `--require scheduling,decode,input,gpu`: **exit 1** if any required gate is
     unavailable or target not `meet` (straddle = unproven → fail)
   - `-o` / `--output`: **exclusive create** (`open("x")`); existing path → exit 1,
     no overwrite (immutable evidence)
   - `final_acceptance_claim` always `false`

## Cumulative end-snapshot cancel / pending (warmup limitation)

Core integrity for decode / input / gpu uses **end-snapshot** counters
(`*_canceled_n`, `*_dropped_n`, `*_lost_n`, `*_pending_n`, coverage flags) on the
observe-end row — **not** observe-window deltas and **not** boundary pending
accounting. Startup warmup cancellations that remain in the cumulative end
snapshot can therefore **conservatively** make decode/input/gpu **unavailable**
even when the steady observe-window delta would be clean. This is intentional
fail-closed for the narrow core.

A later window adapter should compare observation deltas and account for
boundary pending rather than reject all valid steady data solely because of
canceled startup. Until that adapter exists, canceled/pending end-snapshot
counts remain a hard unavailable path.

## Target verdicts

| Verdict | Meaning |
|:---|:---|
| `meet` | available and `upper_ms <= target` |
| `miss` | available and `lower_ms > target` |
| `unproven` | available but target lies inside `[lower, upper]` or no target |
| `unavailable` | no honest bound / integrity failure / wrong scope |

`--require` accepts only `status=available` **and** `target_verdict=meet`.
Product `gpu` never reaches that path on completion-latency alone.

## Verification

```text
python3 docs/memory/test_reference_metrics.py
→ 42 passed
```

Coverage includes: empty/overflow/missing histograms; counter reset; generation
mismatch; duplicate/disappearing slots; absent coverage flags; visible_ack
absent/malformed/False; sample_age_ms absent/null; scheduling first-bucket
lower=0; process-wide cannot meet per-slot; decode/input coverage-lost and
no-input; GPU disabled ≠ paint proxy; GPU product gate not meet on completion
latency (diagnostic only); GPU lost; exclusive `--output`; contamination
intersection; CLI inspect exit 0; require exit 1; **one real reviewed**
flags-off cell
`diagnostics/low-end-reference-screen-20260906T220129Z/tui_n1_active`
(`scheduling_profile` / `responsiveness_profile` / `gpu_completion_profile` false)
proves `--require scheduling,decode,input,gpu` **exit 1** with all four gates
`unavailable` / `profile_disabled` (scheduling + responsiveness required gates
fail as required).

Profiles-on overhead remains **unknown / diagnostic** (not measured here; never
grants acceptance).

## Explicit non-claims / next card

- No RSS/CPU resource gate productization.
- No per-slot scheduling instrumentation (host emits process-wide only today).
- GPU completion **interval** histograms: in serializer, not adapted in this core
  (product `--require gpu` remains unavailable until that adapter).
- No live paired overhead runs; no budget acceptance; no STATE update on this card.
- Flags-off reference cells cannot prove missing p99 — they correctly fail require.
- No observe-window delta / boundary-pending cancel accounting yet (see warmup
  limitation above).

## Example

```text
python3 docs/memory/reference_metrics.py \
  docs/memory/diagnostics/low-end-reference-screen-20260906T220129Z/tui_n1_active \
  --inspect --contamination-from 2026-09-06T22:21:17Z \
  --require scheduling,decode,input,gpu
# exit 1; JSON gates all unavailable; final_acceptance_claim false
```
