# Publisher mono clock brackets + fine latency histograms

Task: `t_6670c1ec`  
Branch/worktree: `codex/memory-diagnostics` / `t_a1f4796f`

## What shipped

### Native (`crates/host/src/responsiveness_profile.rs`)
- Process mono origin pinned at `enable()` (`OnceLock` + `CLOCK_DOMAIN = "responsiveness_process_mono"`).
- Capture/read/sample brackets in **ns**; ms helpers use **floor lower / ceil upper** so ms still encloses the instant.
- **Separate** decode vs input capture brackets (decode flush does not retime input; input path does not retime decode).
- Input path stamps `t0` **before** `INPUT_PENDING` cut so the bracket encloses the deque mutation.
- **Local.flush** unions existing input capture bracket with the flush pending-cut/merge window when refreshing `input_pending_n` (operator dual-bracket note). Drop takes `t0` before pending retain so ended-row timestamps stay honest.
- Fine latency histograms (1..100 ms + overflow), opt-in via `enable_fine()` / `BOT_RESPONSIVENESS_FINE`; legacy coarse hist unchanged when fine off.
- Sibling `decode_unmatched_canceled_n` + `decode_accounting_exact`; **legacy `decode_coverage_complete` semantics unchanged**.

### Publisher (`crates/host-play/src/memory.rs`)
- `elapsed_mono_ns` stamped on the same `Instant` as `elapsed_s`.
- Emits mono clock + separate decode/input capture ns/ms + fine hist siblings + `decode_unmatched_canceled_n` + `decode_accounting_exact`.
- Enables profile + fine from env.

### Launcher (`docs/memory/run_diagnostic.py`)
- `--responsiveness-fine` requires `--responsiveness-profile`; env scrub + metadata provenance.

### Offline (`docs/memory/reference_metrics.py`)
- Contained window uses native mono when present (domain exact; elapsed-based observe bounds; read ⊆ sample; cap_hi ≤ read_hi; interior ended rejected).
- Without mono: still fail-closed with `publisher_clock_bracket_missing` after counter audit (age path).
- Decode identity: `edge = dispatch + canceled - unmatched + pending + dropped + lost`; pending is a gauge (not monotonic); unmatched required for decode only.
- Fine p99 sibling + paired ≤2 ms margin only when both coarse and fine bounds available.

## Tests run
- `cargo test -p host --lib responsiveness_profile -- --test-threads=1` → **21 passed**
  - Includes `flush_expands_input_bracket_over_fresh_pending_cut` (stale starts + changed pending + flush expand; bounds taken **after** second input start, not lifetime union).
  - Includes barrier concurrent input/decode flush.
- `cargo test -p host-play --lib -- --test-threads=1` → **114 passed**
- `python3 -m unittest test_reference_metrics test_run_diagnostic` → **82 passed**

## Operator notes honored
- Flush dual evidence: old input bracket ∪ fresh pending-cut/merge (not old alone).
- No perpetual input-event union (would pin warmup lower forever).
- Legacy coverage helper/field meaning preserved; exact accounting is a sibling.
- Separate decode/input fields + ns direction kept.
