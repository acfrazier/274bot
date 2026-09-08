# Latency cohort producer report

## API / schema

`host::responsiveness_cohort` is an additive process-local producer journal (schema_version = 1). Explicit Rust opt-in only — no CLI/env.

- `begin(Boundaries { start_mono_ns, end_mono_ns, tail_ns }, capacity)` / `begin_default`: one immutable half-open window `[start, end)` plus finite tail. Rejects capacity 0, malformed bounds, already-active, already-finalized, stale/overlapping vs `last_end`.
- `enabled()`: cheap `AtomicBool` gate; disabled producer paths take no cohort lock and (for input) no new REGISTRY lookup beyond prior behavior.
- Identities: `EventId { slot_id, generation, sequence, start_mono_ns, surface }` with `Surface::{Decode,Panel,Tui}`.
- Terminals: `complete` / `cancel` / `lost` / `dropped` / `dropped_start` / `missing_generation_start` / `generation_lost` → `EventRecord` or surface-bearing `LossReceipt`.
- `LossReceipt.generation` is `Option<u64>`: known gens are `Some`; missing live registry generation is explicit `None` (never invents valid `0`).
- `extract_since(cursor)`: acknowledgement protocol — cursor must equal live `cursor_floor` or `InvalidCursor` (journal preserved). Drains undrained journal in publication order; independent `journal_sequence` so long-pending starts cannot wedge later terminals.
- `acknowledge_producers_closed()`: nonblocking barrier the consumer must set only after producers stop publishing. `finalize(now)` refuses before `end+tail` or without the barrier (`ProducersNotClosed` / `Terminal`). Incomplete pending → `Incomplete` losses; cancel/lost/capacity/late/missing-identity make `available=false`.
- Serializable records: `EventRecord`, `LossReceipt`, `CohortBatch`, `TerminalSummary`, `Boundaries`, `EventId` (serde 1.0.219, already locked).

## Capacity / overhead

Default capacity 512. **Shared bound**: `pending.len() + journal.len() <= capacity` on every retain path (`start`, `append` used by terminal records and losses, `dropped_start`, `missing_generation_start`). When the bound is full, producers count `journal_overflow_n` + `losses_n` without retaining an extra slot (`capacity_overflow`). Extraction frees journal slots. No unbounded loss vectors. Cohort accounting is separate from legacy aggregate gauges.

Disabled path overhead: one atomic load on gated call sites; `note_input_start` does not call `live_generation_for` unless cohort `enabled()`.

## Missing-generation contract (input)

In-window actionable `note_input_start` with cohort enabled and no live registry generation:

- **Preserves** every legacy return/counter/queue path (admit returns `true` and enqueues; full-queue drop-before-admit returns `false` and bumps drop counters).
- **Records** a bounded cohort loss: known `slot_id` / `start_mono_ns` / `surface`, `generation: None`, reason `MissingIdentity` (or `LateEvent` if barrier/finalized already closed), `available=false`.
- Full shared capacity still only counts a gap (`capacity_overflow`) — no extra retained slot.
- Pre-start / post-end mono is excluded (no loss, no record), same half-open window as other starts.

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
| Incomplete pending at finalize → Incomplete loss, unavailable | `incomplete_pending_at_finalize_records_incomplete_loss` |
| Generation mismatch terminal → GenerationMismatch loss | `generation_mismatch_terminal_fails_closed` |
| Forged cursor preserves journal | `forged_cursor_preserves_unacknowledged_journal` |
| Cohort off: real decode+input entrypoints, no cohort mutation | `disabled_legacy_entrypoints_do_not_mutate_cohort` |
| Real decode Local edge+dispatch | `existing_decode_entrypoints_publish_cohort_identity` |
| Real TUI input complete + `note_input_canceled` | `actual_input_tui_complete_and_cancel_entrypoints` |
| Real panel `note_input_start` + bind + `note_panel_present` | `actual_panel_present_entrypoint_records_surface` |
| Real `Local` Drop → lost (decode + input) | `local_drop_teardown_publishes_lost_via_actual_drop` |
| Sequential post-barrier late start (same-thread) | `sequential_finalize_race_after_barrier_records_late_producer_start` |
| Concurrent producer start after barrier → LateEvent | `concurrent_producer_start_after_barrier_records_late_event` |
| Missing generation input: legacy admit + identity loss + finalize unavailable | `missing_generation_input_admits_legacy_and_records_identity_loss` |
| Missing generation full-queue drop-before-admit + bounded overflow | `missing_generation_drop_before_admit_records_identity_loss` |
| Missing generation post-end excluded | `missing_generation_out_of_window_excluded` |

Isolation: cohort tests call `responsiveness_profile::tests::lock_tests()` (clears registry/input/bridge + `test_reset` cohort) and `test_reset()` at end so OPT_IN cannot leak into later profile tests.

## Commands / counts

```text
cargo test -p host --lib responsiveness_cohort -- --test-threads=1
→ 19 passed; 0 failed; 0 ignored (180 filtered out)

cargo test -p host --lib -- --test-threads=1
→ 198 passed; 0 failed; 1 ignored (gpu_real_queue…, pre-existing)

cargo test -p host --lib
→ 198 passed; 0 failed; 1 ignored (default thread pool)
```

## Unsupported / out of scope (honest)

- No host-play wiring, drain phase rows, reader Python changes, CLI/env flags, native/SSH/live/performance runs.
- No claim that old binaries emit schema 1.
- Producer-close is caller-owned; this crate does not discover live producer threads.
- Empty/no-input population is valid (`available` can remain true with zero records if no losses).
- This producer task alone is not complete latency instrumentation or p99 acceptance.
- Root nativeinputzero-host audit is separate; this corrective only closes missing-generation cohort accounting.

## Handoff (host-play / reader)

1. `begin` with observe mono bounds + chosen capacity (default 512) and tail.
2. Existing producer entrypoints only.
3. Stop/join producers → `acknowledge_producers_closed()`.
4. Periodic `extract_since(cursor)` starting at 0; next call uses returned `next_cursor`.
5. After `end+tail`, `finalize(now_mono_ns)`.
6. Reader: require `schema_version == 1`, validate identities/bounds, treat any loss / `!available` / overflow / `generation: null` MissingIdentity as gate-unavailable; do not reinterpret legacy `pending` gauges as cohort closure.
