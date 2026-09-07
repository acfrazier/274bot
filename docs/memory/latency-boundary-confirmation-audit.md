# Latency boundary confirmation audit (600s companions)

**Scope:** read-only diagnostic audit. No source, build, live, benchmark, or metric-gate changes.
**Branch:** `codex/memory-diagnostics` (not `main`).
**Owned file:** this report only.
**Task:** `t_da2f4634`.
**Out of scope:** re-audit of input pre-activity trim (`f306183`); overall confirmation narrative (root owns `shared-nav-confirmation-report.md`).

## Evidence set

| Item | Path / value |
|:---|:---|
| Batch | `docs/memory/diagnostics/shared-nav-confirmation-20260907T083318Z/batch.json` |
| Candidate latency bound | `.../candidate_latency-bound.json` → run `20260907T091408Z_tui_n16_active` |
| Reference latency bound | `.../reference_latency-bound.json` → run `20260907T092908Z_tui_n16_active` |
| Reader | `docs/memory/reference_metrics.py` (`_contained_responsiveness_pair`, `evaluate_input`, `evaluate_decode`, `p99_lower_upper_ms`) |
| Observe | phase `observe`, N=16, warmup 120s, observe 600s, responsiveness + fine ON |
| Recompute | Offline re-call of `evaluate_input` / `evaluate_decode` on raw `metadata.json` + `samples.jsonl` bit-matches bound slot reasons |

Gate slot arrays are ordered by `(slot_id, generation)` from `_pair_slot_maps`, **not** by workload ordinal. All tables below map failures through `ordinal_mapping`.

## Reader rule under test

`boundary_pending_incomplete` is raised only after a deterministic maximal publisher-contained mono span is selected, when either edge has non-zero pending gauge:

- Input: `reference_metrics.py` ~1350–1351 — `parsed[0]["pending"] != 0 or parsed[-1]["pending"] != 0`
- Decode: ~1422–1423 — same on `decode_pending_n`

Selection does **not** search for a quieter inner window after a failed validation (function docstring ~1163–1164). Pending is a gauge; mono counters must still identity-check.

`overflow_bucket` is raised by `p99_lower_upper_ms` when the cumulative p99 mass lands in the final open bucket (~140–147): returns `unavailable` with `overflow_count`, `lower_ms` = last finite bound, `upper_ms=None`. Coarse bounds default `(5,10,20,25,40,50,100,250,500,1000)`; fine `1..100` + overflow.

## Headline counts (preserve existing failed classifications)

| Role | Gate | Available | `boundary_pending_incomplete` | `overflow_bucket` |
|:---|:---|---:|---:|---:|
| Candidate | input | 13/16 | **2** | **1** |
| Candidate | decode | 14/16 | **2** | 0 |
| Reference | input | 13/16 | **3** | 0 |
| Reference | decode | 12/16 | **4** | 0 |

These match the confirmation report’s ordinal-level summary and offline recompute.

## Ordinal map of failures

### Candidate (`liveeb0f0_*`, run `20260907T091408Z`)

| Gate idx | Ordinal | Name | gen | slot_id | Reason | Edge with pending≠0 |
|:---:|---:|:---|---:|---:|:---|:---|
| input[8] | **6** | liveeb0f0_6 | 25 | 15394190323276445915 | boundary_pending_incomplete | **end** = 1 (start = 0) |
| input[15] | **5** | liveeb0f0_5 | 32 | 15394191422788074126 | boundary_pending_incomplete | **end** = 1 (start = 0) |
| input[9] | **9** | liveeb0f0_9 | 26 | 15394187024741561282 | overflow_bucket | edges quiet (pending 0/0) |
| decode[5] | **15** | liveeb0f0_15 | 22 | 12269047437541535645 | boundary_pending_incomplete | **start** = 1 (end = 0) |
| decode[14] | **8** | liveeb0f0_8 | 31 | 15394188124253189493 | boundary_pending_incomplete | **start** = 1 (end = 0) |

### Reference (`liveef8d0_*`, run `20260907T092908Z`)

| Gate idx | Ordinal | Name | gen | slot_id | Reason | Edge with pending≠0 |
|:---:|---:|:---|---:|---:|:---|:---|
| input[11] | **5** | liveef8d0_5 | 28 | 3197829328183803740 | boundary_pending_incomplete | **end** = 1 |
| input[10] | **6** | liveef8d0_6 | 27 | 3197832626718688373 | boundary_pending_incomplete | **end** = 1 |
| input[4] | **7** | liveef8d0_7 | 21 | 3197831527207060162 | boundary_pending_incomplete | **end** = 1 |
| decode[15] | **4** | liveef8d0_4 | 32 | 3197830427695431951 | boundary_pending_incomplete | **end** = 1 |
| decode[10] | **6** | liveef8d0_6 | 27 | 3197832626718688373 | boundary_pending_incomplete | **end** = 1 |
| decode[7] | **9** | liveef8d0_9 | 24 | 3197833726230316584 | boundary_pending_incomplete | **end** = 1 |
| decode[5] | **14** | liveef8d0_14 | 22 | 2411864026396081836 | boundary_pending_incomplete | **end** = 1 |

Note: reference ordinal 6 fails **both** input and decode on the same `slot_id` / generation pair (shared bot). Candidate ordinals for input vs decode failures are disjoint.

## Classification of each failure mode

### A. Input `boundary_pending_incomplete` — real nonzero pending at selected **end**

Applies to candidate ordinals 5 & 6 and reference ordinals 5, 6, 7.

**Reader path:** not a metrics-reader bug, lifecycle `ended` loss, cancel/lost/dropped coverage hole, or publisher/counter epoch mismatch on the published gauges. Gauges at the selected edges are real and identity-matched. This ruling does **not** clear the TUI producer path (see §A.1 / ruling matrix).

Per-slot facts (selected maximal contained span; identity `start − complete − canceled − lost − dropped == pending` holds at both edges):

| Role | Ord | Selected start elapsed_s | Selected end elapsed_s | Start counters (start/complete/pending) | End counters | Terminal pending streak wall_s (lower bound) | Last complete advance elapsed_s | Streak onset ≈ 30s boundary |
|:---|---:|---:|---:|:---|:---|---:|---:|:---|
| C | 6 | 193.869 | 792.340 | 2/2/0 | 45/44/1 | **102.223** (samples 486→586) | 689.081 | **~690 s** |
| C | 5 | 631.927 | 792.340 | 1/1/0 | 28/27/1 | **131.817** (457→586) | 659.498 | **~660 s** |
| R | 7 | 211.603 | 809.652 | 2/2/0 | 56/55/1 | **88.761** (499→586) | 719.842 | **~720 s** |
| R | 6 | 661.697 | 809.652 | 1/1/0 | 28/27/1 | **119.427** (469→586) | 689.200 | **~690 s** |
| R | 5 | 631.952 | 809.652 | 1/1/0 | 28/27/1 | **148.958** (440→586) | 659.677 | **~660 s** |

Common pattern (published series):

1. Observe series length 587 samples both roles; clock domain `responsiveness_process_mono`.
2. One input **starts** near the terminal streak (`Δstart` leaves `pending=1`) and **never completes** through the last observe sample.
3. `ended=false`; canceled/lost/dropped remain 0 across the span.
4. Capture brackets remain set; `cap_lo` freezes at the open input’s capture lower while `cap_hi` keeps advancing with later registry reads — consistent with an open in-flight input on the published counters, not a missing publisher epoch.
5. Reader correctly refuses histogram p99: an open completion at the end edge means the window is not a closed accounting interval.

These are **long-lived open pending gauges** (order 10² s of observe wall time still pending), not single-sample edge jitter. Do not reclassify as pass by trimming the end or picking an earlier quiet sample (forbidden favorable inner window). Whether the underlying input *should* have completed earlier is a separate producer question (§A.1), not a reader reclassification.

#### A.1 Unresolved TUI producer focus/flush correlation risk (hypothesis — not proven RC here)

Distinguish two layers:

| Layer | Status in this audit |
|:---|:---|
| **(a) Metrics reader** | Correctly fail-closes on open gauges at the selected edges. Recompute matches bound reasons. No reader bug proven. |
| **(b) TUI producer correlation** | **Unresolved risk.** Evidence does **not** clear instrumentation on the TUI start/flush path. Not proven as root cause in this audit; root owns a separate bounded proof/fix. |

Producer path (cited, not fixed here):

1. `crates/tui/src/bin.rs` `run_loop`: each iteration calls `session.pump(&mut app)` **before** `terminal.draw` (~1304–1316). Under `memory-profile`, `Session::pump` sets `app.focused = Some(run.focus_index())` and `play.focus(...)` (~943–945).
2. After a successful draw, `note_tui_draw_flush` runs only for the **post-pump** `app.focused_name()` (~1318–1322).
3. Host `note_tui_draw_flush` (`crates/host/src/responsiveness_profile.rs` ~871–877) completes pending only when `p.surface == Tui && p.slot_id == flush slot_id`.
4. Input starts also bind `focused_name` at key/mouse time (~1328–1333, ~1345–1350).
5. Focus rotation: `host-play` `MemoryRun::focus_index` = `(started.elapsed().as_secs() / 30) % n` unless `render_policy.pins_focus()` (`crates/host-play/src/memory.rs` ~1592–1598; default panel benchmark comment: rotate every 30s).

**Time tie-in (consistent with risk, not proof):** terminal input pending-streak onsets cluster on **30 s boundaries** across cand/ref — ~660 / ~690 / ~720 s elapsed (table above). A plausible mechanism is focus-switch mis-attribution: start recorded against slot A, then pump advances focus before draw so flush completes only the **new** focused slot, leaving A’s pending open until a later focus return (or never within observe). Candidate input **ordinal 9 overflow** may be a long-but-eventually-completed instance of the same class (edges quiet; one completion >1000 ms coarse). Mark as **hypothesis/risk for separate root proof** — not proven root cause here, and **not** a reader defect.

Do **not** globally assert “no instrumentation bug,” “not a serializer glitch,” or “pure harness teardown workload.” Those claims overreach: (a) is cleared; (b) is not.

### B. Decode `boundary_pending_incomplete` — two distinct edge geometries

#### B1. Candidate: nonzero pending at selected **start** only (ordinals 15 & 8)

| Ord | Selected start elapsed_s | start pending | start sample_age_ms | end pending | Clears by |
|---:|---:|---:|---:|---:|:---|
| 15 | 193.869 | **1** | 5 | 0 | next sample (~+1.026 s) |
| 8 | 193.869 | **1** | 4 | 0 | next sample (~+1.026 s) |

Mechanism:

- Full observe mono window uses first/last `elapsed_mono_ns` (`observe_lb`…`observe_ub`).
- First observe row’s decode capture lower is **below** `observe_lb`, so `start_pos` advances to sample index 1.
- That first *contained* capture sample happens to publish `decode_pending_n=1` with accounting identity intact (`edge == dispatch + canceled − unmatched + pending + dropped + lost`).
- End edge is quiet (`pending=0`). Canceled/lost/dropped deltas across the would-be span are 0; unmatched stays flat.

So: **real gauge pending=1 at the algorithmically selected start edge**, not a counter reset, not lifecycle end, not epoch mismatch. The in-flight decode clears on the following sample; the reader still fail-closes because it will not slide the start forward to the first quiet row.

#### B2. Reference: nonzero pending at selected **end** only (ordinals 4, 6, 9, 14)

All four: selected start pending=0, selected end pending=1, identity holds, end `sample_age_ms` ≈ 25–27 ms on the final row. Final counters show `edge − (dispatch + canceled − unmatched + dropped + lost) = pending = 1`. This is the decode analogue of the input end-open case, but the open work is **short-lived relative to input** (appears only on the last sample in the terminal streak list for these slots — a decode in flight at observe teardown), not a multi-minute hang.

Still correctly classified: end edge not quiet ⇒ no closed decode latency span.

### C. Candidate input ordinal 9 — `overflow_bucket` (not boundary pending)

Contained window **succeeds**:

| Field | Value |
|:---|:---|
| selected_start_elapsed_s | 271.665746958 |
| selected_end_elapsed_s | 792.339612291 |
| input_preactivity_rows_trimmed | 77 |
| counter_deltas | start=55, complete=55, canceled=lost=dropped=pending=0 |
| clock_domain | responsiveness_process_mono |
| contained_window | true |

Histogram deltas on that span (`input_latency_buckets` / `input_fine_latency_buckets`):

**Coarse** (bounds ms `[5,10,20,25,40,50,100,250,500,1000]` + overflow):

| Bucket | Range (ms) | Count |
|---:|:---|---:|
| 0 | (0, 5] | 52 |
| 5 | (40, 50] | 1 |
| 6 | (50, 100] | 1 |
| 10 (overflow) | **(1000, ∞)** | **1** |
| sum | | **55** |

**Fine** (bounds 1..100 ms + overflow):

| Bucket | Range (ms) | Count |
|---:|:---|---:|
| 0 | (0, 1] | 50 |
| 1 | (1, 2] | 2 |
| 40 | (40, 41] | 1 |
| 61 | (61, 62] | 1 |
| 100 (overflow) | **(100, ∞)** | **1** |
| sum | | **55** |

p99 decision: with n=55, cum·100 ≥ n·99 requires the overflow bucket on both hist widths ⇒ `reason=overflow_bucket`, `sample_n=55`, `overflow_count=1`.

**Lower bound only (no invented precise duration):**

- Coarse: the overflowing completion is **> 1000 ms** (`lower_ms=1000`, `upper_ms=null`).
- Fine: same completion is **> 100 ms** (`lower_ms=100`); this is a weaker bound than coarse and does not tighten it.
- 54/55 completions sit in finite buckets; one sample forces p99 into overflow. Status remains `unavailable` / `target_verdict=unavailable`. Do not treat as a numeric p99 pass or miss against the 100 ms input target.

## Ruling matrix (task questions)

| Hypothesis | Verdict |
|:---|:---|
| Real nonzero pending at selected edges | **Yes** for all `boundary_pending_incomplete` slots (input end-open; cand decode start-open; ref decode end-open). Gauges match accounting identity. |
| Lifecycle loss (`ended`, cancel/lost/drop) | **No** on these failing slots’ selected edges / deltas. |
| Publisher/counter epoch mismatch | **No** evidence on published counters: mono non-decreasing, identities hold, capture brackets ordered, clock domain constant. |
| Reader bug | **Not proven.** Failures reproduce from raw rows through current `_contained_responsiveness_pair` + pending edge rule. Recompute matches bound reasons exactly. Reader correctly fail-closes. |
| TUI / instrumentation producer bug | **Not cleared.** Unresolved focus/flush correlation risk (§A.1): pump-before-draw + flush only for post-pump focused slot + 30s rotation; input streak onsets align ~660/690/720 s. Hypothesis only — not proven RC in this audit; raw classifications unchanged. |
| “No instrumentation bug” / pure workload teardown | **Do not claim.** Overreaches beyond reader-only evidence. |
| Favorable inner window would “fix” | Possibly for cand decode start-open (next sample quiet) and maybe ref decode last-sample open — **explicitly disallowed** by reader policy; must not be counted as pass. |
| Unavailable counted as pass | **No** in bound or recompute. |

## One concrete confounder / honest limitation

**Open gauges at the observe edge under a maximal contained span pinned to observe mono bounds (no drain-to-quiet), plus an unresolved TUI producer focus/flush correlation risk that could leave prior-focus slots pending across 30 s rotations.**

- **(a) Reader / window:** For **input**, several bots publish a single incomplete start for ~90–150 s through end of observe; until completion (or cancel/lost with coverage) lands, input p99 for that slot is unprovable under the closed-span rule. For **decode**, high-rate edges mean the first capture with `cap_lo ≥ observe_lb` (candidate) or the last sample before observe ends (reference) often coincides with `pending=1` for a few milliseconds of sample age. Reader refusal is intentional fail-closed; **600 s observation without end-of-window quiet/drain** leaves a minority of slots without a closed span even when interior traffic looks healthy. No favorable-window or gate relaxation.
- **(b) Producer risk (unproven RC):** Input end-open streak starts cluster on 30 s focus boundaries (660/690/720 s) on both roles — consistent with `session.pump` advancing `focus_index` before `note_tui_draw_flush` attributes completion only to the new focused `slot_id` (`bin.rs` ~943–945, ~1304–1322; `responsiveness_profile.rs` ~871–877). Overflow ordinal 9 may be the same class with a late completion. This is **not** a workload-only explanation and **not** a proven root cause here; it is the honest instrumentation limitation alongside (a).

This pair explains why both roles show incomplete input boundaries on nearby ordinals (5–7) and why decode incompletes differ in edge geometry between candidate and reference **without** implying a metrics-reader defect, and **without** asserting the producer path is clean.

## Bounded correction proposal

**No source correction in this task.** No reader bug was proven; producer focus/flush ordering is **not** implemented or fixed here (root-owned if authorized). Bound artifacts and unavailable classifications stay as-is.

Out-of-scope notes only (operational / future authorized work — **not** gate relaxation):

1. **Producer ordering proof/fix (root-owned, separate task):** if authorized, prove or fix pump-vs-draw / focus-slot flush attribution so starts and flushes cannot desync across 30 s rotations. Out of scope for this audit.
2. **Drain / quiet tail (harness):** after observe_s, optionally wait until per-slot input/decode pending return to 0 (with timeout → keep `boundary_pending_incomplete`) before stopping samples — would address end-open cases without changing p99 math.
3. **Do not** auto-advance start/end to the nearest quiet sample inside the reader (that is favorable window selection).
4. **Do not** treat `overflow_bucket` as a finite p99 or soften overflow to a synthetic upper bound.
5. Preserve all current unavailable classifications on these artifacts.

## What this audit does not claim

- No latency non-regression between candidate and reference.
- No final acceptance, no metric gate change, no claim that interior “most slots meet 100 ms” substitutes for full 16/16 closed spans.
- No precise duration for the overflowing input sample beyond **> 1000 ms** coarse lower bound.
- No re-litigation of `f306183` pre-activity trim (already reviewed).
- No clearance of TUI producer instrumentation; §A.1 is hypothesis/risk only, not a proven root cause and not a source fix.
- No change to raw bound classifications or gate thresholds.

## Method note

Offline scripts used for this report lived only as untracked scratch under `docs/memory/_tmp_latency_boundary_*.py` and were not committed. Authoritative inputs remain the batch bounds, receipts, and raw run directories cited above.
