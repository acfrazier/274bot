# Cohort and capture integration review (Grok 4.6)

Not a per-task re-review and not campaign completion.

## Identity

| Field | Value |
|---|---|
| Reviewer profile | `branchreviewer` |
| Actual model / provider | `grok-4.6` / `xai-oauth` (no task overrides) |
| Frozen pair | `1ce9edb8252a0917c86e66ab5a31324426284af5` → `f9d729ce6e986a9effeac3134e262eeeb9209cb7` |
| Checkout HEAD at review | `7d0a2bdb30ba67a6d23d3b3891825510a35c7424` (STATE/manifest freeze only; source blobs match frozen head) |
| Client (unchanged) | `3456edc8dabf7b25ada78110ffa56327af9f67a4` |
| Manifest | `docs/memory/cohort-integration-review-manifest.json` |
| UTC | 2026-09-08T12:59:19Z |

All 14 listed source SHA-256 values at `f9d729c` match the manifest and the worktree. Contract/evidence hashes for execution.md, companion plan, named producer/publisher/reader/trace/capture/native reports, and the stimulus helper also match. Archive SHA-256 `8a9e5a1a9fc446a980ab579faf04516dd2ad45e94737c05cff96e1e148c7dbd5` matches.

## Verdict

**BOUNDED ACCEPT** for the next *source-justified one native input functional rerun* on capture-corrected binaries.

This does **not** release:

- rebuild/proof that the capture correction actually attaches `capture_tx` on native
- predeclared matched N=16 companion
- performance / p99 / RSS / lifecycle / scaling acceptance
- whole-campaign Grok 4.6

The original native diagnostic is evidence of a capture-lifecycle confounder on a pre-fix binary, plus structurally available decode cohort shape. It is not postfix capture proof and not input-population completeness.

## What was inspected (independent of prior Grok 4.5 task reviews)

Source (cold, then against the named contract):

- `crates/host/src/responsiveness_cohort.rs` producer journal
- `crates/host-play/src/memory.rs` publisher poll/arm/drain/finalize
- `docs/memory/cohort_reader.py` + additive `reference_metrics.py` wiring
- `crates/host/src/input_seam_trace.rs` and call sites (`window.rs`, `app.rs`, `session.rs`, `slot_io.rs`, `responsiveness_profile.rs`)
- `crates/panel/src/session.rs` `memory_focus_at` / `set_game_pane_open` / `stream_capture_for`
- surrounding `focus.rs`, `host/src/lib.rs`, `host-play/src/lib.rs`

Plan/contract: `docs/execution.md` integration-milestone cadence; companion plan §§3–7; named reports in the manifest.

Raw evidence: sidecar + qualification + `analyze_run` on `diagnostics/windows-input-trace-cohort-a-archive/latency-diagnostic-input-trace-20260908-a`, plus `stderr-after-pulses.log` stage counts. No native/SSH/workload/rerun.

## Contract mapping

### Producer identity / journal / loss

`EventId.sequence` (`State.next_sequence`) is distinct from `JournalEntry.journal_sequence` (`next_journal_sequence`). `extract_since` acknowledges the internal cursor only (`cursor == cursor_floor`); a forged cursor returns `InvalidCursor` without draining.

Half-open membership is `start_mono_ns ∈ [START, END)`. Completions after `END+tail` become `LateEvent` losses. `capacity_overflow` increments `losses_n` / `journal_overflow_n` without retaining a receipt. `finalize` converts leftover pending to `Incomplete` losses, then reports `pending_n = pending.len()` which is 0 after that drain — so pending0/complete≠coverage. `LossReceipt.generation` is `Option`; missing live generation is `None`, never invented 0.

`begin_armed` stores `OPT_IN` before sampling START under the ledger lock; publisher uses that path, not `begin`.

Disabled producer: `enabled()` atomic; `note_input_start` calls `live_generation_for` only when cohort is enabled.

### Publisher arm / observe / drain / terminal

Arm only if `responsiveness_profile::enabled()`. Sidecar `create_new`. `poll` checks process-mono `NOW >= END`, stamps `observe_end_elapsed_s` from harness `started.elapsed()` **before** `write_qualification("observe-end")`, then takes sample mono **after** that write — so the forced final observe row is expected wholly after END. Drain flips only after that sample (`enter_drain_after_sample`). Tail is absolute `END+DEFAULT_TAIL_NS` (5s), not the 60s teardown wait.

`durable_write_sample_line`: extract → attach cursor → writeln. Panel `ui_frame` uses `play.as_mut()` so `finish_cohort_shutdown` can `stop_slot` (which joins) before `process::exit`. `producers_joined` is set only after that stop/join loop. Terminal repeats the retained observe-end stamp.

Input population header: panel + `pins_focus()` → `{"kind":"focused-one","slots":[0]}`; `frontend=="tui"` → tui-endpoint note regardless of render policy; else all-run-slots. `focus_index()` returns 0 when `pins_focus()`.

No new CLI/env flags. Legacy sample fields unchanged; `cohort` is additive.

### Reader

Fail-closed offline reader. Legacy `decode`/`input` gates still run; missing sidecar → cohort unavailable, not a legacy rewrite. Quiet-inner-span fixture does not meet. Aggregate `pending==0` is not required.

Harness elapsed and process mono are distinct; observe-end qual `elapsed_s` may exceed retained `cohort.observe_end_elapsed_s`; terminal must equal the retained stamp (native: 314.0232895 vs 314.0232196, terminal matches retained). Final observe `elapsed_mono_ns >= END` is required; bracketing exact END is not.

Pre-arm seed generations ignored (no `cohort.present`). In-cohort reset/ended/disappeared fail closed. `losses_n` may exceed serialized receipts. Per-slot p99; `p99_ns` SHA-256 `ade46732007625b4ddd83f7895e38350646c7799879ed8f9030c8ceb41a9d8f3` (inspect.getsource, trailing newline).

`analyze_run` on this diagnostic directory returns `missing metadata.json` for both cohort gates. Direct structural metadata mapped from qualification settings is not canonical matched metadata.

### Input trace disabled path

`note_*` functions return on `!enabled()`. `enabled()` is atomic / cached env, not per-call `std::env::var`. App capture keeps `capture && is_item_hovered()` short-circuit when tracing is off. Stream traces before the `tx` None return (diagnostic), then no-ops send. Drain traces only after the slot is enabled and a key is applied. No new protocol opcode or synthetic input.

### Capture lifecycle

Reproduced confounder: `memory_draw_policy` writing `game_pane_open = true` then `set_game_pane_open(true)` seeing `was==true` skips `capture_on`. `memory_focus_at` rolls that flag back and calls `set_game_pane_open(true)` **only** when `forces_pane && !was_open`. Stable open frames add no setter wake. Pref-off stays off. RotatingAll does not force the pane.

This is the source justification for one postfix native input functional rerun. The archived native run used candidate `984c639` / binary `8247b703…` **before** `b9ea71e`/`f9d729c`.

## Independent tests run here

| Command | Result |
|---|---|
| `python3 -m unittest discover -s docs/memory -p 'test_cohort_reader.py' -q` | 60 passed, 0 skip |
| `python3 -m unittest discover -s docs/memory -p 'test_reference_metrics.py' -q` | 100 passed |
| `python3 -m unittest discover -s docs/memory -p 'test_matched_evidence_adapter.py' -q` | 46 passed |
| `cargo test -p host --lib responsiveness_cohort -- --test-threads=1` | 24 passed |
| `cargo test -p host --lib input_seam -- --test-threads=1` | 7 passed |
| `cargo test --locked -p panel --features memory-profile-no-alloc --lib -- capture memory_ --test-threads=1` | 25 passed, 363 filtered |
| `cargo test -p host-play --features memory-profile --lib cohort -- --test-threads=1` | 4 passed |
| `… --exact memory::tests::poll_forces_final_observe_sample_when_last_sample_recent_at_end` | pass |

No native launch. Instrumentation-overhead suite was not re-run (legacy; not required to re-prove this pair).

## Raw native evidence (pre-capture-fix binary)

Independent counts from `stderr-after-pulses.log`: `win_arrow=40`, `imgui_arrow=40`, `stream=40`, `channel=0=40`, `channel=1=0`, `drain=0`, `metric_start=0`. Sidecar: 300 unique Decode sequences 1..300, all starts in `[START,END)`, all completes `<= END+tail`, 0 Panel records, terminal `available=true` `records_n=300` `losses_n=0` `pending_n=0` `producers_joined=true`. Direct structural reader: decode available/meet (structural only); input `declared_population_incomplete`.

Tar listing: 36 files + 3 directories (39 members) vs manifest “35 files”. Archive hash still matches; this is a counting discrepancy in the freeze text, not a source defect.

Counterfactual qualification copy remains parser-diagnostic only.

## Findings

1. **Not blocking this gate — required before any performance/RSS cell.** `host::debug_enabled()` now calls `std::env::var("BOT_DEBUG")` when the `DEBUG` latch is false (`crates/host/src/lib.rs`). That sits on the observe hitch and `client_frame` paths. `input_seam_trace::enabled()` already caches; this does not. Uncached env lookup allocates on the ordinary debug-off frame path vs `1ce9edb`. Cache it the same way before N=16 companion / memory comparison. It does not break cohort membership, reader fail-closed rules, or the BOT_DEBUG=1 capture localization path.

2. **Non-blocking.** `Play::test_install_stoppable_slot` is `cfg(any(test, feature = "memory-profile"))`, so a dummy join helper is compiled into production memory-profile binaries. Unused on the harness path; do not treat as a new producer.

No material defect found in cohort identity/window/tail, publisher arm-before-start / forced final observe / finite drain / stop+join terminal, reader clocks/population/cursor/loss/provenance, disabled trace evaluation order, or the closed-to-open capture reattach.

No per-frame world copy, pending-gauge reinterpretation, or new input/protocol opcode in this pair.

## Release boundary

Allowed after this review: rebuild both native parent roles with this reviewed instrumentation **including** `f9d729c` capture gating; run **one** bounded native input functional proof that capture stays attached (`capture_tx=1` through pulses, drain/metric_start observed, slot-0 input starts published). Preserve incomplete input if it remains incomplete.

Not allowed: claiming the 2026-09-08-a archive as postfix proof; using qualifier-mapped meta as matched `metadata.json`; using the counterfactual qual copy as native evidence; executing the predeclared N=16 companion before that postfix functional proof; performance or campaign acceptance.

Root still reconciles this report against source. Final finish-plan / whole-branch Grok 4.6 remain open.
