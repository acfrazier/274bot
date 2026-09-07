# Reference metrics — bounded offline histogram / observation-window core

Task `t_135ceb9f` (recovery of timed-out `t_8bade183`). **No final performance
or lifecycle acceptance claim.**

## Contained responsiveness windows (follow-up)

Decode now has a narrow contained-window adapter for serializers that emit
`updated_ms`, `sample_age_ms`, `decode_edge_n`, and `dispatch_n`. It selects the
maximal timestamp-contained span first (publisher update after warmup, monotonic
sample order, stable `(slot_id, generation)`, unfinished boundaries rejected),
then validates the whole selected span. It differences decode edge/dispatch and
cancellation/lost/drop/pending counters, retains the stable edge-dispatch-cancel
accounting offset, and does not interpret cumulative `decode_coverage_complete`
as a window flag. It never searches for a favorable inner segment after a bad
middle; excluded boundary samples are reported explicitly. Input uses the same
publisher boundary proof with `input_start_n`/`input_complete_n`, but remains
unavailable when no input events exist. Missing, malformed, stale, reset,
restarted, or clock-misaligned publisher evidence fails closed.

The qualified control N=1 cell `20260907T004143Z_tui_n1_active` and candidate
N=1 cell `20260907T004608Z_tui_n1_active` each prove a contained decode span:
197 edge/dispatch events, zero cancellation/lost/drop/pending deltas, and
coarse p99 bound 25–40 ms (target 100 ms). Their lifetime cancellation counts
(11 and 14) remain visible and do not become window cancellations. Input remains
unavailable because `input_start_n=0`; this is not an input pass. The failed
N=16 run `20260907T005032Z_tui_n16_active` remains unqualified and cannot pass
overall; its partial decode observations are not acceptance evidence.

This adapter does not prove the requested 2 ms paired margin: the histogram
bounds remain coarse (25–40 ms), not a precise 2 ms measurement. Resource
strict eligibility remains unavailable without attributable sampler/serializer
overhead evidence. Hardware presentation/scanout timestamps, generated input
events, and target-hardware validation remain unavailable.

## Scope (this card only)

| Deliverable | Role |
|:---|:---|
| `docs/memory/reference_metrics.py` | Core + CLI |
| `docs/memory/test_reference_metrics.py` | Synthetic + one real cell |
| `docs/memory/reference-metrics-report.md` | This report |

No Rust, Docker, STATE, live runs, or builds. Workload qualification reuses
`qualify_control.py` via `--qualify` (not reimplemented). Resource/RSS rollups,
paint-cadence product gates, per-slot scheduling instrumentation, and a true
hardware-presentation endpoint remain outside this card.

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
   batch lag** → cannot prove per-slot or exact observation coverage; they remain
   diagnostic and cannot satisfy the per-slot gate.
   Per-slot scheduling is now adapted from `scheduling_slots`: the gate differences
   cumulative counters and histograms using each slot's start/end
   `last_cycle_mono_ms` endpoints. It reports the actual measured duration,
   observed iterations/fps, qualified worst slot, and full-fleet coverage. A
   slot's local monotonic origin is used only for its own duration; no absolute
   mono comparison across slots or against global `elapsed_s` is made. The
   lifetime `first_interval_*` fields are deliberately ignored because they
   generally include warmup. Coverage is checked as
   `cycle_delta = interval_delta + mode_break_delta + anchor_miss_delta`;
   `park_n_delta` is still reported, but parking clears the anchor without
   incrementing `cycle_n`; the resumed cycle is represented by `anchor_miss_delta`.
   resets, ended/lost rows, stale/missing snapshots, duplicate/missing expected
   slots, and incomplete coverage fail closed. `scene_transition_separation`
   remains unavailable, so this is not steady-scene-only proof.
   A paired candidate/reference p99 comparison is accepted only when the
   conservative bound `candidate_upper - reference_lower <= 2 ms`; otherwise
   it is `inconclusive`. No interpolation or exact percentile is claimed.
7. **Resources** — `evaluate_resources` consumes the host's sampled
   `process_cpu_user_s` + `process_cpu_system_s` counters and divides their
   endpoint delta by the actual `elapsed_s` monotonic-relative observation
   span. It reports process CPU cores, current resident RSS median/max, and
   lifetime/observation peak RSS separately; it never calls peak RSS steady
   resident memory. Missing, non-finite, non-monotonic, reset, or insufficient
   counters/boundaries are unavailable. The approved active N=1/N=16 TUI and
   both panel policies use the exact budgets in `performance-finish-plan.md`;
   unsupported scales/modes are unavailable. Numerical observations remain
   visible under `--inspect`, while resource status requires the real
   `qualify_control` result and active workload. Attributable instrumentation
   overhead evidence is not implemented, so the required resource gate remains
   unavailable (a caller cannot manufacture it with an `overhead` label), and
   complete provenance metadata is still required for matching:
   `nav_pack_sha256`, `nav_flags_sha256`, `renderer_settings`, `cache_settings`,
   `catalog_sha256`, `feature_flags`, `allocator_provenance`,
   `host_sources_sha256`, `client_sources_sha256`, and `binary_sha256`. Missing
   evidence remains unavailable while numerical observations stay visible under
   `--inspect`; contaminated windows are rejected.
8. **Matched comparisons** — `compare_matched_runs` requires available,
   uncontaminated runs with identical frontend, scale, workload, renderer/data
   settings, provenance and measured overhead metadata. It reports intended
   RSS and CPU deltas diagnostically, but a single pair is always
   `inconclusive`; mismatches and unknown profiles-on overhead never become
   accepted savings. P99 comparisons remain conservative bound comparisons.
9. **GPU product gate** — `--require gpu` is the **product frame/cadence** gate.
   The adapter now differences stable completion interval histograms,
   `stable_completed_n`, and registration/completion/lost/dropped/pending
   counters between the exact observe endpoints for every `(slot_id,
   generation)`. It requires a stable observed renderer identity, actual
   non-CPU backend residency, finite `sample_age_ms`, zero pending at both
   boundaries, complete registration/coverage flags, and no dropped/lost
   delta. Focused rows must show observed >=40 fps and p99 interval <=40 ms;
   background rows report expected 1 fps with an explicit ±0.25 fps tolerance.
   The adapter never lowers the configured cadence to make a row pass.
   `completion_latency_buckets` remain callback-delivery diagnostics only and
   cannot product-meet on their own. The product result still records that the
   endpoint is CPU callback delivery after a prior submit, not hardware timing
   or scanout, and never uses host paint as a proxy.
10. **CLI**
   - `--inspect` (default path): partial JSON, **exit 0**
   - `--require resources,scheduling,decode,input,gpu`: **exit 1** if any required gate is
     unavailable or target not `meet` (straddle = unproven → fail)
   - `-o` / `--output`: **exclusive create** (`open("x")`); existing path → exit 1,
     no overwrite (immutable evidence)
   - `final_acceptance_claim` always `false`

## Legacy cumulative end-snapshot cancel / pending (warmup limitation)

The legacy path and GPU adapter use **end-snapshot** counters
(`*_canceled_n`, `*_dropped_n`, `*_lost_n`, `*_pending_n`, coverage flags) on the
observe-end row — **not** observe-window deltas and **not** boundary pending
accounting. Startup warmup cancellations that remain in the cumulative end
snapshot can therefore **conservatively** make decode/input/gpu **unavailable**
even when the steady observe-window delta would be clean. This remains
fail-closed when publisher timestamps and accounting fields are absent. The
contained decode/input adapter above is the only exception, and only when its
raw boundary proof succeeds.

A later adapter may account for non-zero boundary pending without rejecting the
window. The GPU interval adapter currently requires both endpoint pending
snapshots to be zero and reports `boundary_accounted=false` otherwise; it does
not manufacture coverage across an unfinished boundary. Decode/input retain
the original conservative end-snapshot behavior.

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
→ 65 passed
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

## Explicit non-claims / remaining gaps

- Resource RSS/CPU results are bounded diagnostic gates only; no final
  performance acceptance or overhead-corrected saving is claimed.
- Legacy process-wide scheduling remains diagnostic; per-slot scheduling is
  qualified only when endpoint-stamped `scheduling_slots` rows are present.
- GPU interval evidence is callback-delivery cadence, not a true presentation
  or scanout endpoint; hardware/scanout completion remains unavailable.
- Per-slot scheduling p99 is bounded tooling evidence only; it does not provide
  scene separation, resource/RSS acceptance, or hardware presentation proof.
- No live paired overhead runs; no budget acceptance; no STATE update on this card.
- Flags-off reference cells cannot prove missing p99 — they correctly fail require.
- Resource/RSS rollups, overhead attribution, and true presentation timestamps
  remain diagnostic gaps for subsequent work.

## Example

```text
python3 docs/memory/reference_metrics.py \
  docs/memory/diagnostics/low-end-reference-screen-20260906T220129Z/tui_n1_active \
  --inspect --contamination-from 2026-09-06T22:21:17Z \
  --require resources,scheduling,decode,input,gpu
# exit 1; JSON gates all unavailable; final_acceptance_claim false
```
