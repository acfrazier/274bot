# Responsiveness mono clock brackets + fine latency histograms

Task: `t_6670c1ec`  
Branch/worktree: `codex/memory-diagnostics` / `t_a1f4796f`  
Scope: instrumentation + offline adapter only. **No live/performance acceptance, no measured speedup, no latency pass from unit fixtures.**

## Design

### Native (`crates/host/src/responsiveness_profile.rs`)
- Process mono origin pinned at `enable()` (`OnceLock`, `CLOCK_DOMAIN = "responsiveness_process_mono"`).
- Capture/read/sample brackets in **ns**; ms helpers **floor lower / ceil upper**.
- **Separate** decode vs input capture brackets (flush does not retime input; input does not retime decode).
- Input stamps `t0` **before** `INPUT_PENDING` cut; flush unions retained input bracket with fresh pending-cut/merge (not lifetime input-event union).
- Fine latency histograms (1..100 ms + overflow), opt-in `enable_fine()` / `BOT_RESPONSIVENESS_FINE` with launcher `--responsiveness-fine` provenance. Legacy coarse hist unchanged when fine off.
- Sibling `decode_unmatched_canceled_n` + `decode_accounting_exact`; **legacy `decode_coverage_complete` semantics unchanged**.

### Publisher (`crates/host-play/src/memory.rs`)
- `elapsed_mono_ns` stamped on the same `Instant` as `elapsed_s`.
- Emits mono clock + separate decode/input capture ns/ms + fine hist siblings.
- Responsiveness slot JSON built in **two layers** (core row + fine/ack inserts) under serde_json recursion limit; fine arrays/bounds serialized via `.as_slice()`.

### Offline (`docs/memory/reference_metrics.py`)
- Contained window uses native mono only (domain exact; observe bounds from first/last `elapsed_mono_ns`; read ⊆ sample; cap_hi ≤ read_hi; interior ended rejected).
- Without mono: fail-closed `publisher_clock_bracket_missing` after counter audit.
- Decode identity: `edge = dispatch + canceled - unmatched + pending + dropped + lost`; pending is a gauge.
- **Both** decode and input run interior histogram + `latency_n` monotonic checks before accepting a span (closes recovered hist-reset false pass).
- Hist conservation: coarse sum == `latency_n` delta == dispatch (decode) / complete (input) delta when those fields are present; when fine is present, fine sum matches and rolls into coarse ≤100 bins with overflow equality.
- Single-run emits coarse/fine p99 bounds only. **No same-run `fine_paired_within_2ms`.**
- `paired_fine_p99_margin` is a **diagnostic** bound-difference helper only: finite
  nonnegative lower≤upper bounds may yield `diagnostic_margin_ms`, but status stays
  `unavailable` / `paired_matched_run_evidence_pending` with **no**
  `paired_within_margin` pass boolean. True paired ≤2 ms evidence adapter (artifact
  binding, slot→run, gate/endpoint match) remains an explicit remaining gap.

## Bounded per-slot storage / work (extra vs pre-task)

| Item | Approx size / work |
|:---|:---|
| `decode_fine_latency_buckets` `[u64; 101]` | 808 B |
| `input_fine_latency_buckets` `[u64; 101]` | 808 B |
| Separate decode/input capture mono brackets (4×u64 ns) | 32 B |
| `decode_unmatched_canceled_n` | 8 B |
| Fine hist increment on record path | +1 bin index + optional add when fine on |
| Fine off | null siblings / no fine bin work |

Exact live on/off overhead measurement remains **missing** (resource overhead proof still unlocked-blocked).

## Tests run (this revision)

| Command | Result |
|:---|:---|
| `cargo test -p host --lib responsiveness_profile -- --test-threads=1` | **21 passed** |
| `cargo test -p host-play --lib --features memory-profile -- --test-threads=1` | **146 passed** |
| `cargo check -p tui --features memory-profile` | **ok** |
| `python3 -m unittest test_reference_metrics test_run_diagnostic` | **85 passed** (after paired provenance tests) |

Adversarial fixtures under `docs/memory/diagnostics/clock-adapter-review-20260907T022630Z/`:
- `input-recovered-histogram-reset.json` → `unavailable` / `counter_reset`
- `same-run-false-paired-proof.json` → no same-run paired claim; conservation rejects inconsistent fine/coarse

## Limitations (explicit)

- No live matched-run performance or latency acceptance from this card.
- No on/off overhead measurement yet; resource overhead proof remains missing.
- Legacy JSONL without mono stays `publisher_clock_bracket_missing` (not metadata opt-in).
- Unit fixtures prove adapter fail-closed / clock enclosure semantics only — not product latency.
- Endpoints remain truthful: TUI draw flush / GPU callback, not physical display scanout.
- Process-local clocks are not comparable across runs except durations/distributions.
- Paired ≤2 ms fine margin: diagnostic arithmetic only; true matched-run evidence
  adapter remains missing (no eligibility pass from this card).

## Files

- `crates/host/src/responsiveness_profile.rs`
- `crates/host-play/src/memory.rs`
- `docs/memory/reference_metrics.py`
- `docs/memory/test_reference_metrics.py`
- `docs/memory/run_diagnostic.py` / `test_run_diagnostic.py` (fine flag provenance)
- `docs/memory/responsiveness-clock-bracket-report.md` (this file)
- `docs/memory/reference-metrics-report.md` (adapter semantics note)
- Alias: `docs/memory/publisher-clock-fine-hist-report.md` → points here
