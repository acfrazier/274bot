# Latency cohort producer report

## API / schema

`host::responsiveness_cohort` is an additive process-local producer journal (schema_version = 1). Explicit Rust opt-in only — no CLI/env.

- `begin(Boundaries { start_mono_ns, end_mono_ns, tail_ns }, capacity)` / `begin_default`: one immutable half-open window `[start, end)` plus finite tail. Rejects capacity 0, malformed bounds, already-active, already-finalized, stale/overlapping vs `last_end`.
- `enabled()`: cheap `AtomicBool` gate; disabled producer paths take no cohort lock and (for input) no new REGISTRY lookup beyond prior behavior.
- Identities: `EventId { slot_id, generation, sequence, start_mono_ns, surface }` with `Surface::{Decode,Panel,Tui}`.
- Terminals: `complete` / `cancel` / `lost` / `dropped` / `dropped_start` / `generation_lost` → `EventRecord` or surface-bearing `LossReceipt`.
- `extract_since(cursor)`: acknowledgement protocol — cursor must equal live `cursor_floor` or `InvalidCursor` (journal preserved). Drains undrained journal in publication order; independent `journal_sequence` so long-pending starts cannot wedge later terminals.
- `acknowledge_producers_closed()`: nonblocking barrier the consumer must set only after producers stop publishing. `finalize(now)` refuses before `end+tail` or without the barrier (`ProducersNotClosed` / `Terminal`). Incomplete pending → `Incomplete` losses; cancel/lost/capacity/late make `available=false`.
- Serializable records: `EventRecord`, `LossReceipt`, `CohortBatch`, `TerminalSummary`, `Boundaries`, `EventId` (serde 1.0.219, already locked).

## Capacity / overhead

Default capacity 512. **Shared bound**: `pending.len() + journal.len() <= capacity` on every retain path (`start`, `append` used by terminal records and losses, and `dropped_start`). When the bound is full, producers count `journal_overflow_n` + `losses_n` without retaining an extra slot (`capacity_overflow`). Extraction frees journal slots. No unbounded loss vectors. Cohort accounting is separate from legacy aggregate gauges.

Disabled path overhead: one atomic load on gated call sites; `note_input_start` does not call `live_generation_for` unless cohort `enabled()`.

## Finalization safety argument

Timestamps alone never prove pre-end producers have finished publishing. Host-play must stop/join (or otherwise synchronize) every generation that can call start/terminal APIs, then `acknowledge_producers_closed()`, then `finalize(end+tail)`. A start that still arrives after the barrier with in-window mono is recorded as `LateEvent` (not silently dropped) and fails the cohort closed. Gameplay paths never block on the barrier.

## Requirement → test table

| Requirement | Test |
|---|---|
| Half-open window, pre-start exclude, post-end exclude, complete-in-tail, barrier before finalize | `window_tail_and_barrier` |
| Post-end exclude via real `Local::note_decode_edge` | `post_end_start_excluded_on_real_decode_entrypoint` |
| Bounded capacity; overflow counters; drain | `extraction_drains_and_capacity_loss_is_bounded` |
| drop-before-admit at full pending does not exceed shared cap | `drop_before_admit_at_full_pending_does_not_exceed_capacity` |
| Out-of-order complete / independent journal cursor | `extraction_does_not_wedge_on_long_pending_first_event` |
| Cancel → unavailable | `incomplete_and_noncompleted_fail_closed` |
| Forged cursor preserves journal | `forged_cursor_preserves_unacknowledged_journal` |
| Cohort off: real decode+input entrypoints, no cohort mutation | `disabled_legacy_entrypoints_do_not_mutate_cohort` |
| Real decode Local edge+dispatch | `existing_decode_entrypoints_publish_cohort_identity` |
| Real TUI input complete + `note_input_canceled` | `actual_input_tui_complete_and_cancel_entrypoints` |
| Real panel `note_input_start` + bind + `note_panel_present` | `actual_panel_present_entrypoint_records_surface` |
| Real `Local` Drop → lost (decode + input) | `local_drop_teardown_publishes_lost_via_actual_drop` |
| Finalize barrier race: decode after `acknowledge_producers_closed` → LateEvent | `finalize_race_after_barrier_records_late_producer_start` |
| Missing generation input: legacy admit, no cohort identity | `missing_generation_input_admits_legacy_without_cohort_identity` |

Isolation: cohort tests call `responsiveness_profile::tests::lock_tests()` (clears registry/input/bridge + `test_reset` cohort) and `test_reset()` at end so OPT_IN cannot leak into later profile tests.

## Commands / counts

```text
cargo test -p host --lib responsiveness_cohort -- --test-threads=1
→ 14 passed

cargo test -p host --lib -- --test-threads=1
→ 193 passed; 1 ignored (gpu_real_queue…, pre-existing)

cargo test -p host --lib
→ 193 passed; 1 ignored (default thread pool)
```

## Unsupported / out of scope (honest)

- No host-play wiring, drain phase rows, reader Python changes, CLI/env flags, native/SSH/live/performance runs.
- No claim that old binaries emit schema 1.
- Producer-close is caller-owned; this crate does not discover live producer threads.
- Empty/no-input population is valid (`available` can remain true with zero records if no losses).
- This producer task alone is not complete latency instrumentation or p99 acceptance.

## Handoff (host-play / reader)

1. `begin` with observe mono bounds + chosen capacity (default 512) and tail.
2. Existing producer entrypoints only.
3. Stop/join producers → `acknowledge_producers_closed()`.
4. Periodic `extract_since(cursor)` starting at 0; next call uses returned `next_cursor`.
5. After `end+tail`, `finalize(now_mono_ns)`.
6. Reader: require `schema_version == 1`, validate identities/bounds, treat any loss / `!available` / overflow as gate-unavailable; do not reinterpret legacy `pending` gauges as cohort closure.
