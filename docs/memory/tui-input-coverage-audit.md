# TUI N16 input coverage audit (partial real-PTY probes)

Task: `t_b0bc0948`  
Branch/worktree: `codex/memory-diagnostics` / `t_a1f4796f`  
Scope: **read-only diagnosis**. Own only this report. No source changes, builds, tests, live, or server ops.

## Evidence corpus

| Item | Value |
|:---|:---|
| Batch | `docs/memory/diagnostics/instrumentation-overhead-20260907T064437Z/` |
| Protocol | OFF/ON/ON/OFF; `tui_input_probes: true` |
| Probe commit | `e62ea7e` (settings-overlay `o` via real PTY after observe-start, 1 s interval) |
| Profiled cells | `on_a` = `20260907T071537Z_tui_n16_active`, `on_b` = `20260907T072213Z_tui_n16_active` |
| Spec | TUI N=16 active, warmup 30 s, observe 120 s, scheduling + render + responsiveness (+ fine) |
| Bound sides | `on_a-bound.json`, `on_b-bound.json` — workload / native / managed_resources valid; probe `error: null`; 112 writes each ON cell |
| INPUT gate (bound) | status `available`, target `unproven`; **1/16** slot available each ON cell |

OFF cells also sent probes (113/112) but have no native latency histograms with profiles off — out of scope for latency; stimuli only.

## Native / harness semantics (inspected, not changed)

### Focus rotation (`crates/host-play/src/memory.rs`)

```text
focus_index = (started.elapsed().as_secs() / 30) % n   // unless render_policy.pins_focus()
```

TUI memory pump copies that index into UI focus and `play.focus(name)` (`crates/tui/src/bin.rs`). Panel-only render-policy env is rejected on TUI; default is rotating-all. Metadata may label TUI `render_policy: "none"`; runtime focus still rotates on the 30 s schedule. Full fleet focus period at N=16 is **480 s**; a **120 s** observe window covers **about four** consecutive ordinals (plus partial edges).

### Input producer (`crates/host/src/responsiveness_profile.rs` + TUI loop)

- `note_input_start(slot_id_for(focused_name), …)` only when a key/mouse press has a focused name.
- TUI complete path: `note_tui_draw_flush(focused_name)` after successful `terminal.draw`.
- Input capture mono brackets stamp on pending cut / complete / flush **only when input is touched** (`input_start_n` / complete / pending / …). Untouched slots keep **unset** `(0,0)` input capture brackets.
- Decode capture brackets update on ordinary local flush for every live slot — which is why decode is 16/16 while input is not.

### Probe (`docs/memory/tui_input_probe.py`)

- Writes begin only after native `observe-start`; 1 Hz `o`; restore toggle may fire **after** observe-end (outside observation).
- Writes are **stimuli only** — never treated as latency acks in this audit.

### Offline window selector (`docs/memory/reference_metrics.py` `_contained_responsiveness_pair` / `evaluate_input`)

1. Early: if observe boundary start **and** end rows have `input_start_n == 0` → `no_input_samples`.
2. Else: build a mono-contained span over **all** observe samples for `(slot_id, generation)`.
3. Per sample: require `responsiveness_clock` domain `responsiveness_process_mono`, sample/read brackets, **and** row `{input,decode}_capture_mono_ns_*`.
4. Capture parse `unset`/`missing` (including lo=hi=0) increments `clocks_absent` and drops clock fields for that row.
5. If **any** `clocks_absent` after the pass → **`publisher_clock_bracket_incomplete`** for the whole slot (no leading-unset trim; no favorable inner search).

## Recomputed per-slot totals (raw samples.jsonl)

Observe phase: **117** samples both ON runs. Sample-level `responsiveness_clock` present on **117/117**. Probe writes: **112** (`kind=write`); mono write span slightly wider than first/last observe sample (first write just before first observe row, last just after last) → ~1 write outside observe counter window.

### `on_a` (`20260907T071537Z`)

Focus proxy `floor(elapsed_s/30)%16` over observe samples: ordinals **4,5,6,7,8** (counts 18/30/29/29/11).

| ord | name | raw Δ start/complete (observe boundary, same gen) | unset-capture leading samples | set-capture samples | bound gate | bound selected Δ start/complete |
|---:|---|---:|---:|---:|---|---:|
| 0–3, 9–15 | (11 slots) | 0 / 0 | 117 | 0 | `no_input_samples` | — |
| 4 | livebd1e0_4 | 17 / 17 | 0 | 117 | **available** | **16 / 16** (p99 fine upper 1 ms; excluded before_start=1) |
| 5 | livebd1e0_5 | 28 / 28 | 19 | 98 | `publisher_clock_bracket_incomplete` | — |
| 6 | livebd1e0_6 | 28 / 28 | 48 | 69 | incomplete | — |
| 7 | livebd1e0_7 | 28 / 28 | 77 | 40 | incomplete | — |
| 8 | livebd1e0_8 | 10 / 10 | 107 | 10 | incomplete | — |

Sum raw observe-boundary Δ start = **111**; probe writes = **112**. Starts always equal completes per slot (pending/canceled/lost/dropped 0 on available path).

### `on_b` (`20260907T072213Z`)

Focus proxy ordinals **3,4,5,6,7** (7/29/29/30/22).

| ord | raw Δ S/C | unset leading | set | bound gate | bound selected Δ |
|---:|---:|---:|---:|---|---:|
| 0–2, 8–15 | 0/0 | 117 | 0 | `no_input_samples` | — |
| 3 | 6/6 | 0 | 117 | **available** | **5/5** (fine p99 upper 1 ms; before_start=1) |
| 4 | 28/28 | 7 | 110 | incomplete | — |
| 5 | 28/28 | 36 | 81 | incomplete | — |
| 6 | 28/28 | 66 | 51 | incomplete | — |
| 7 | 21/21 | 95 | 22 | incomplete | — |

Sum raw Δ start = **111** vs 112 writes. Same start≡complete pattern.

## What the absent slots mean

### `no_input_samples` (11/16 each ON cell) — **legitimately unfocused in observe**

Native starts only attach to the **currently focused** name. With 30 s rotation and 120 s observe, only ~4–5 ordinals receive PTY keys during observe. The other eleven keep `input_start_n == 0` on both observe boundary rows. That is expected under current stimulation + focus policy — **not** a publisher failure and **not** proof that those bots cannot take input.

Do **not** assume “all 16 slots must show input” for an N16 rotating TUI cell. Decode/scheduling already prove the other slots are live.

### `publisher_clock_bracket_incomplete` on positive-input slots — **reader cut, not missing clocks**

These slots **did** receive real native input:

- Counters rise in lockstep with focus windows (~1 start/complete per second while focused).
- After first input, every remaining observe sample has **set** input capture brackets and the process mono sample clock (117/117 sample clocks globally).
- Failure mode: **leading** observe samples still have **unset** input capture `(0,0)` because the producer only stamps input brackets once input is touched. The reader counts those unset rows as `clocks_absent` and then fails the **entire** slot as `publisher_clock_bracket_incomplete`, even though later rows are fully clocked and counters match.

The single **available** slot each run is exactly the ordinal that **already had input capture set at observe start** (focus carried in from late warmup / first observe second): zero leading-unset rows → contained mono path succeeds. Selected deltas are slightly below raw boundary deltas because the maximal contained span drops the first sample when prior capture lower sits outside observe mono lower (`coverage_excluded_edges.before_start_samples = 1`).

So: positive-input incompletes are **false incompletes under current cut rules**, not absent publisher mono and not failed PTY→native pairing.

## Probe writes vs native counters

| | on_a | on_b |
|---|---:|---:|
| PTY `write` events | 112 | 112 |
| Sum native observe-boundary Δ `input_start_n` | 111 | 111 |
| Sum native observe-boundary Δ `input_complete_n` | 111 | 111 |
| Bound available selected matched start/complete | 16 | 5 |

The 112 vs 111 gap is explained by write timestamps **outside** the first/last observe sample envelope (plus optional post-end restore, which is logged separately as `restore` and not counted in `sent`). **Do not** treat keystroke write counts as ack proof; the native start/complete equality is the authority, and it holds on every slot that saw input.

## Narrow correct next change

**Primary: offline reader** (`docs/memory/reference_metrics.py` `_contained_responsiveness_pair` for `prefix == "input"`, and any shared helper used the same way).

Required behavior (preserve fail-closed mono / epoch / counter rules):

1. When a row’s input capture bracket is **unset/missing** and input counters are still **all zero** (no start/complete/canceled/lost/dropped/pending), treat the row as **pre-activity** for that slot: omit it from the mono row list (leading trim only; do not invent clocks).
2. Once capture is set or any input counter is non-zero, require full sample/read/capture mono consistency as today.
3. Unset capture **with** non-zero counters → still fail closed (malformed / incomplete).
4. True mix of present vs absent **sample-level** `responsiveness_clock` on included rows → still `publisher_clock_bracket_incomplete` / missing.
5. After trim, keep **one** deterministic maximal contained span (observe mono bounds from first/last `elapsed_mono_ns`; capture ⊆ read ⊆ sample; no favorable inner search after outcome failure; no synthetic clocks; no weakening p99 / coverage / visible_ack gates).
6. Do **not** change aggregation to “all slots must input.” Gate may stay `unproven` when only a subset of fleet slots have input samples; that matches rotating focus. Optional later product policy can define fleet vs focused-seat input targets explicitly.

**Not the first fix:**

- **Producer** always stamping input capture on every flush without input would make capture “set” with zero counters and might pass today’s reader, but it expands claimed input capture over periods with no input events (weaker semantic of the input bracket). Only consider if reader trim is wrong for a documented reason.
- **Stimulation / harness** (pin focus for input cells, longer observe for full rotation, multi-seat key routing) improves **how many** seats are exercised; it does not fix the false incomplete on seats that already have native samples. Optional follow-up after reader fix if product wants multi-seat input p99 under rotation.

## Proposed regression (before implementation; do not implement here)

Add offline fixtures under `docs/memory/test_reference_metrics.py` (and tiny JSON under diagnostics if that is the house style):

1. **Leading unset then real input** — observe series with sample clocks on every row; slot capture unset + zero counters for samples 0..k-1; from k onward set capture and monotonic start/complete (+ matching latency hist). Expect **available** (or hist-available) with counter deltas from the trimmed span — **not** `publisher_clock_bracket_incomplete`.
2. **Unset capture but non-zero counters** — fail closed (not available).
3. **Missing sample clock on a post-activity row** — still incomplete/missing.
4. **Never-input slot** — still `no_input_samples`.
5. **Golden replay** (optional): freeze a trimmed excerpt of `on_a` ord 5 / `on_b` ord 4 progressive rows and assert the post-fix reason is no longer incomplete and selected start/complete match recomputed trimmed deltas (27/27 style from the hypothesis table below).

Hypothesis after leading-unset trim (same raw files; not executed as a product change):

| run | ord | clean span samples | raw trimmed Δ S/C |
|---|---:|---:|---:|
| on_a | 5 | 19..116 (98) | 27/27 |
| on_a | 6 | 48..116 (69) | 27/27 |
| on_a | 7 | 77..116 (40) | 27/27 |
| on_a | 8 | 107..116 (10) | 9/9 |
| on_b | 4 | 7..116 (110) | 27/27 |
| on_b | 5..7 | … | 27/27, 27/27, 20/20 |

Still expect **11** `no_input_samples` until stimulation/focus policy changes — that is correct.

## Conclusions

1. Partial INPUT coverage on these N16 profile cells is **two layered**: (a) focus rotation limits which seats receive PTY stimuli in 120 s; (b) the offline input contained-window selector rejects mid-window-first-input seats because pre-input unset capture brackets are scored as clock absence.
2. Real PTY probes **do** produce native start/complete pairs for the focused seat; 112 writes ≈ 111 observe-boundary native starts; available seat shows matched completes and sub-ms fine upper bound.
3. Narrow next implementation card: **reader pre-activity trim** for input capture unset+zero-counter leading rows; regressions above; no gate weakening; no “all slots must input” assumption; no synthetic clocks.
4. No performance acceptance from this audit.

## Files read (no code edits)

- `docs/memory/diagnostics/instrumentation-overhead-20260907T064437Z/{batch,cell-summary,on_a-bound,on_b-bound,on_a-spec}.json`
- `docs/memory/diagnostics/20260907T071537Z_tui_n16_active/{metadata,samples,input-probes}.json(l)`
- `docs/memory/diagnostics/20260907T072213Z_tui_n16_active/{metadata,samples,input-probes}.json(l)`
- `docs/memory/reference_metrics.py` (`_contained_responsiveness_pair`, `evaluate_input`)
- `docs/memory/tui_input_probe.py`, `tui-input-probe-report.md`
- `crates/host/src/responsiveness_profile.rs` (input start/complete/flush brackets)
- `crates/tui/src/bin.rs` (focus pump, note_input_start, note_tui_draw_flush)
- `crates/host-play/src/memory.rs` (`focus_index`, render policy)
