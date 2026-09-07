# Input pre-activity trim (contained latency reader)

Task: `t_fb472c0c`  
Branch/worktree: `codex/memory-diagnostics` / `t_a1f4796f`  
Diagnosis: `docs/memory/tui-input-coverage-audit.md` (36757e2, Grok-approved `t_b0bc0948`)

## Scope

Owned only:

- `docs/memory/reference_metrics.py`
- `docs/memory/test_reference_metrics.py`
- `docs/memory/input-preactivity-trim-report.md` (this file)

No Rust / build / live / server changes. Original bound JSON and raw runs preserved.

## Change

In `_contained_responsiveness_pair` for `prefix == "input"` only:

1. After sample/read/`elapsed_mono_ns` validation, leading rows with **unset/missing** input capture and **genuine all-zero** native input state are omitted from the mono selection list (pre-activity trim).
2. Zero-state proof (`_input_row_preactivity_ok`): required counters `start/complete/canceled/lost/dropped/pending` present and typed zero; optional `input_latency_n` / `input_latency_ns` zero if present; present coarse/fine hists all zero / well-formed.
3. Once capture is set (or activity would have been seen), any later unset/missing capture still fails closed (`publisher_clock_bracket_incomplete`).
4. Unset + non-zero counter or non-zero/malformed hist → incomplete (not trimmed).
5. Missing sample clock on a pre-activity or post-activity row still fails incomplete (trim does not hide bad global clocks).
6. Observe mono bounds still come from the **full** observe series of validated sample clocks (including trimmed pre-activity rows), not from the first post-trim row.
7. After trim, existing deterministic maximal contained span (capture ⊆ observe mono; list positions; contiguous sample indices; no favorable inner search; no synthetic clocks).
8. Meta: `input_preactivity_rows_trimmed` on successful input contained windows.
9. Decode path unchanged. Never-input slots still `no_input_samples` via boundary `input_start_n == 0`.

## Regressions (RED then GREEN)

`InputPreactivityTrimTests` in `test_reference_metrics.py`:

| Test | Expectation |
|:---|:---|
| leading unset zeros + valid clocks then real input | `available`, deltas from trimmed span, `input_preactivity_rows_trimmed=3` |
| unset + non-zero counter | `publisher_clock_bracket_incomplete` |
| unset zero counters + non-zero hist | incomplete |
| missing sample clock on pre-activity prefix | incomplete |
| missing sample clock post-activity | incomplete |
| internal unset after activity | incomplete |
| never-input | `no_input_samples` |

## Verification

```text
python3 -m unittest docs.memory.test_reference_metrics -q   # 88 OK
python3 -m unittest docs.memory.test_matched_evidence_adapter -q  # 38 OK
```

## Real corpus replay

Corpus: `diagnostics/instrumentation-overhead-20260907T064437Z`  
Runs: `20260907T071537Z_tui_n16_active` (ON_A), `20260907T072213Z_tui_n16_active` (ON_B)

### New derived artifacts (originals untouched)

| Artifact | Role |
|:---|:---|
| `…/on_a-analyze-posttrim.json` | `analyze_run` post-trim |
| `…/on_b-analyze-posttrim.json` | `analyze_run` post-trim |
| `…/on_a-bound-posttrim.json` | `bind_side` post-trim |
| `…/on_b-bound-posttrim.json` | `bind_side` post-trim |
| `…/input-preactivity-trim-replay.json` | machine summary |

Preserved: `on_a-bound.json`, `on_b-bound.json`, raw `samples.jsonl` / receipts.

### INPUT gate post-trim (both analyze_run and bind_side)

| Run | available | no_input_samples | other incomplete | gate status | target_verdict | Σ selected start/complete |
|:---|---:|---:|---:|:---|:---|---:|
| ON_A | **5** | **11** | 0 | available | unproven | **106 / 106** |
| ON_B | **5** | **11** | 0 | available | unproven | **106 / 106** |

Fleet target stays `unproven` because only a subset of seats have input samples under 30 s focus rotation + 120 s observe — **not** forced to fleet pass. Matches audit: expect 5 available / 11 no_input, not full-gate pass.

### Per available slot (ON_A analyze)

| gen | trimmed | before_start | Δ start/complete | coarse p99 upper | fine upper | verdict |
|---:|---:|---:|---:|---:|---:|:---|
| 20 | 107 | 107 | 9 / 9 | 5 | 1 | meet |
| 22 | 0 | 1 | 16 / 16 | 5 | 1 | meet |
| 25 | 77 | 77 | 27 / 27 | 5 | 2 | meet |
| 26 | 19 | 19 | 27 / 27 | 5 | 1 | meet |
| 27 | 48 | 48 | 27 / 27 | 100 | 61 | meet |

### Per available slot (ON_B analyze)

| gen | trimmed | before_start | Δ start/complete | coarse p99 upper | fine upper | verdict |
|---:|---:|---:|---:|---:|---:|:---|
| 20 | 95 | 95 | 20 / 20 | 5 | 1 | meet |
| 21 | 36 | 36 | 27 / 27 | 100 | 54 | meet |
| 23 | 0 | 1 | 5 / 5 | 5 | 1 | meet |
| 31 | 7 | 7 | 27 / 27 | 5 | 2 | meet |
| 32 | 66 | 66 | 27 / 27 | 40 | 35 | meet |

### Counter-cut exclusions (not “all 112 writes”)

- Probe writes per ON cell: **112** (stimuli only; not acks).
- Pre-trim bound available matched start/complete: ON_A **16**, ON_B **5** (single seat that already had capture at observe start).
- Post-trim selected Σ start/complete: **106** each ON cell.
- Excluded relative to 112 writes: **6** (writes outside observe counter envelope / restore / edge samples not in maximal contained span) — do **not** claim all 112 as selected latency events.
- Pre-activity rows trimmed equal leading unset-capture counts from the audit (e.g. ON_A 19/48/77/107 on progressive seats; 0 on the seat that entered observe already clocked).
- `before_start_samples` equals trimmed count when the first post-trim row is also the first contained-span endpoint (no extra capture-before-observe drop); the already-clocked seat keeps `before_start=1` with `trimmed=0` as before.

### Original bound (pre-trim) for contrast

From preserved `on_*-bound.json`: INPUT status available with **1/16** slot available each ON cell; remaining positive-input seats were `publisher_clock_bracket_incomplete`. Post-trim those false incompletes become available without weakening p99 / coverage / visible_ack rules.

## Non-goals confirmed

- No producer always-stamp of input capture without input.
- No stimulation / focus-policy change.
- No synthetic clock stamps.
- No threshold / coverage / visible_ack weakening.
- No global aggregation change to force fleet INPUT pass.
