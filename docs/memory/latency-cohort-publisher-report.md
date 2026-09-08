# Latency cohort publisher — implementer report

Task: `t_5992fd7a`  
Branch/worktree: `codex/latency-cohort-publisher`  
Companion: `docs/memory/current-latency-companion-plan.md`, `docs/memory/latency-cohort-producer-report.md`

## Summary

Additive fixed-window cohort journal publisher on the memory harness path. Active only when the existing responsiveness profile is already enabled at observe-start (no new CLI/env flags). `begin_armed` activation order fixed (OPT_IN before START under ledger lock). Operator integration-review checklist resolved, including a production poll scheduling gap for the final observe row. Round-1 review corrections: concrete JSON envelope freeze below; `observe_end_elapsed_s` stamped before observe-end qualification write; report provenance matches commit file list. Round-2: envelope field meanings aligned to source (`complete`/`next_cursor`/`pending_n`/`records_n`/finalize endpoint); production `Run::poll` asserts poll-written observe-end qual meta.

## Changes

| Area | Path | What |
| --- | --- | --- |
| Activation | `crates/host/src/responsiveness_cohort.rs` | `begin_armed`: enable OPT_IN before START sample under lock; harness ledger reset surface; concurrent enable-before-START test |
| Publisher | `crates/host-play/src/memory.rs` | `CohortPublisher` on `Run`; arm/drain/attach/finish; production helpers |
| Poll mut | `memory.rs`, `crates/panel/src/app.rs` | `poll(&mut self, play: &mut Play)`; panel `play.as_mut()` so stop_slot/join can run before `process::exit` |
| Play test helper | `crates/host-play/src/lib.rs` | `#[cfg(test)] test_install_stoppable` dummy slot for join path |
| Report | `docs/memory/latency-cohort-publisher-report.md` | this file |

**Not in bf614b9 / this publisher commit:** `crates/tui/src/bin.rs` already called `poll(play.as_mut())` before this work; no TUI bin edit was required. `cargo check -p tui --features memory-profile` still verifies the mut Play path compiles.

## Operator checklist (codex-orch)

1. **attach before durable write** — `durable_write_sample_line`: drain journal → `attach_cohort_sample_refs` → `writeln`. Qualification path attaches before write.
2. **Same immutable START/END as activation** — `arm_cohort_at_observe_start` stores `begin_armed` boundaries; phase/tail use those fields only (not a later poll-entry now + re-begin).
3. **Observe-end final row before drain** — `enter_drain_after_sample` defers drain flip until after the sample write; `sample_due` includes that flag so a recent `<1s` last_sample still forces the final phase=`"observe"` row. Drain is set only after that write.
4. **Stop at predeclared absolute END+tail** — `cohort_tail_complete(boundaries, now_mono)`: `now_mono >= end_mono_ns + tail_ns` (equality OK; `finalize` rejects only `<`). Named `DEFAULT_TAIL_NS = 5s`. Not Instant-relative drain or 60s teardown as the finite tail.
5. **Cohort-off preserved** — arm only if `responsiveness_profile::enabled()`; child-process `--exact` test.
6. **observe-end meta** — stamp `CohortPublisher.observe_end_elapsed_s` from harness `started.elapsed()` **before** `write_qualification(..., "observe-end")` so additive `cohort.observe_end_elapsed_s` is non-null on that row.

## Frozen JSON envelope (reader-owner contract)

Schema version: `host::responsiveness_cohort::SCHEMA_VERSION` = **1**.  
Clock domain: `host::responsiveness_profile::CLOCK_DOMAIN` = `"responsiveness_process_mono"`.  
Default finite tail: name `DEFAULT_TAIL_NS`, value `5_000_000_000` ns (5s), recorded in header/terminal before results.  
Membership: half-open `[START, END)` on start_mono_ns; included events may complete through `END+tail`, including equality (the producer rejects completion timestamps strictly greater than that deadline).

### Sidecar path

- Derived from the memory samples path: `output_path.with_extension("cohort.jsonl")`.
  - Example: `…/samples.jsonl` → `…/samples.cohort.jsonl` (Path::with_extension replaces the final extension).
- Opened with `create_new(true)` at arm time. **Failure to create** (exists, permissions, etc.) fails the arm with `cohort sidecar <path>: …` and leaves no cohort publisher.
- **Missing sidecar at read time:** if cohort mode never armed (profile off), no sidecar is expected. If arm succeeded, sidecar must exist; reader treats absence as incomplete publication.
- Writes use `writeln!` + `sync_all` on header, each batch, and terminal. **Write/sync failure** returns `Err` (fail-closed); no silent drop of durable publication loss.

### Record order (sidecar JSONL)

Exactly one line per record, UTF-8 JSON objects, newline-terminated:

1. **`cohort-header`** — once, at arm (observe-start), before any batch.
2. **`cohort-batch`** — zero or more, at 1 Hz sample cadence and once more at finalize (final extract after barrier).
3. **`cohort-terminal`** — once, after producers joined + `acknowledge_producers_closed` + `finalize(now_mono ≥ END+tail)` (equality permitted; source rejects only `now_mono < END+tail`).

No other record types. Batches may be **counter-only** (empty `records`/`losses`) when `loss_count` or `journal_overflow_n` advanced.

### `cohort-header` fields

| Field | Type | Meaning |
| --- | --- | --- |
| `record` | `"cohort-header"` | discriminant |
| `schema_version` | u16 | `1` |
| `tail_name` | string | `"DEFAULT_TAIL_NS"` |
| `tail_ns` | u64 | absolute tail duration |
| `observe_ns` | u64 | observe window duration used to build END |
| `capacity` | usize | journal capacity (`DEFAULT_CAPACITY` = 512) |
| `frontend` | string | `"panel"` / `"tui"` / harness frontend label |
| `input_population` | object | see below |
| `decode_population` | object | `{"kind":"all-run-slots","n":N}` |
| `boundaries` | object | `{start_mono_ns, end_mono_ns, tail_ns}` immutable |
| `clock_domain` | string | `"responsiveness_process_mono"` |
| `sidecar_path` | string | absolute/display path of this file |

`input_population` kinds:

- Panel focused-one: `{"kind":"focused-one","slots":[0]}`
- TUI: `{"kind":"tui-endpoint","note":"TUI flush endpoint; not panel texture present"}`
- Else: `{"kind":"all-run-slots","n":N}`

### `cohort-batch` fields

| Field | Type | Meaning |
| --- | --- | --- |
| `record` | `"cohort-batch"` | discriminant |
| `schema_version` | u16 | from `CohortBatch` |
| `boundaries` | Boundaries | same window |
| `records` | EventRecord[] | EventRecords extracted from journal entries after the previous cursor, with any `Outcome`. EventId.sequence is a start identity, not a journal cursor. |
| `losses` | LossReceipt[] | Retained loss receipts extracted from journal entries after the previous cursor. LossReceipt.sequence is an event identity, not a journal cursor. |
| `next_cursor` | u64 | **Inclusive** high-water: largest extracted internal `JournalEntry.journal_sequence`, unchanged for an empty batch. This internal append-order sequence is distinct from EventId.sequence and LossReceipt.sequence and is not serialized per entry. Pass the returned cursor unchanged to the next extraction. Never compute it from event IDs or filter records by comparing their event sequence with it. |
| `complete` | bool | **`State.finalized` only** (source: `complete: s.finalized`). Not proof of producer barrier success, empty pending, or lossless population. Can be `true` on an unavailable final batch after finalize. |
| `loss_count` | u64 | cumulative losses |
| `journal_overflow_n` | u64 | cumulative capacity rejects |
| `available` | bool | **only when** `journal_overflow_n > 0` → `false` (publisher adds this on overflow drain) |
| `unavailable_reason` | string | **only when** overflow → `"journal_overflow"` |

EventRecord: `{id: EventId, complete_mono_ns: Option<u64>, outcome: Outcome}`.  
EventId: `{slot_id, generation, sequence, start_mono_ns, surface}` with `surface` ∈ `Decode|Panel|Tui`.  
Outcome ∈ `Completed|Canceled|Lost|Dropped`.  
LossReceipt: `{sequence, slot_id, generation: Option<u64>, start_mono_ns: Option<u64>, surface, outcome, reason}` with reason ∈ `Capacity|LateEvent|GenerationMismatch|MissingIdentity|Incomplete|MalformedTimestamp`. Generation is never invented as 0 when missing.

**Counter-only rule:** if vectors empty but `loss_count` or `journal_overflow_n` changed vs last drain, still emit a batch (and retain overflow reason). Overflow on any batch → publisher returns `Err` after durable write (fail-closed).

### `cohort-terminal` fields

| Field | Type | Meaning |
| --- | --- | --- |
| `record` | `"cohort-terminal"` | discriminant |
| `schema_version` | u16 | from `TerminalSummary` |
| `boundaries` | Boundaries | same window |
| `terminal` | bool | summary.terminal (always true on success path) |
| `available` | bool | `s.available && s.pending.is_empty()` after finalize drain; false on source losses / incomplete / overflow path |
| `records_n` | u64 | count of **all** EventRecords successfully appended via the terminal path (`records_n` increments for any `Outcome` that becomes an EventRecord). **Not** “Completed latency samples only”. Inspect `outcome` on records; losses are separate (`losses_n` / LossReceipt). |
| `losses_n` | u64 | Cumulative counted losses, including capacity losses whose receipts could not be retained. It can exceed the number of serialized LossReceipts; inspect journal_overflow_n and preserve unavailable status. |
| `pending_n` | u64 | `s.pending.len()` **after** finalize drains prior pending into Incomplete LossReceipts. Typically **0 even when incomplete work existed**. Reader must use `losses_n` / `available` / loss reasons — **never** equate `pending_n == 0` with closed success. |
| `tail_name` | string | `"DEFAULT_TAIL_NS"` |
| `producers_joined` | bool | publisher proved stop_slot/join path |
| `observe_end_elapsed_s` | f64 \| null | harness elapsed stamped at observe-end |
| `frontend` | string | same as header |

**Finalize / completion deadline (source-aligned):**

- `finalize(now_mono_ns)` rejects only when `now_mono_ns < end_mono_ns + tail_ns` (or producers not closed). **Equality is permitted.**
- Harness `cohort_tail_complete` is `now_mono >= end_mono_ns + tail_ns` (same inclusive endpoint).

**Fail-closed terminal:** `available == false` → `finish_cohort_journal_after_barrier` returns `Err` (`cohort terminal unavailable: …`). Overflow on final batch also Err. Finalize with `now_mono < END+tail` is Err. Incomplete producer barrier is Err via `acknowledge_producers_closed` / finalize.

### Additive refs on legacy samples / qualification (not sidecar)

Samples (`durable_write_sample_line`) gain object `cohort`:

```text
schema_version, present=true, sidecar_path, cursor,
boundaries{start_mono_ns,end_mono_ns,tail_ns}, tail_name,
terminal: null | {available, records_n, losses_n, pending_n}
```

Qualification rows gain object `cohort`:

```text
schema_version, present=true, phase_tag, sidecar_path,
boundaries{…}, tail_name, observe_end_elapsed_s,
qualification_elapsed_s_is_harness_not_cohort_mono=true
```

Legacy sample/qualification field meanings and counters are unchanged; `cohort` is additive only. Old binaries/archives without sidecar remain valid.

### Missing / failed-write behavior (summary)

| Condition | Behavior |
| --- | --- |
| Profile off at observe-start | no arm, no sidecar, no `cohort` keys |
| Sidecar `create_new` fails | arm Err; run fails closed |
| Batch/header/terminal writeln or sync fails | Err; no continue-as-success |
| `journal_overflow_n > 0` on drain | durable counter batch + `unavailable_reason`, then Err |
| Terminal `available=false` | durable terminal row, then Err → harness exit failure path |
| process::exit path | panel/TUI must `poll(&mut Play)` so `finish_cohort_shutdown` stop_slot/join runs before exit |

## Drain / loss counters

`drain_cohort_journal` tracks `last_loss_count` / `last_journal_overflow_n` and emits counter-only batches when loss/overflow advances with an empty event payload.

## Tests run (this re-review handoff)

```text
cargo test -p host-play --features memory-profile --lib \
  -- --exact memory::tests::poll_forces_final_observe_sample_when_last_sample_recent_at_end \
  --test-threads=1

cargo test -p host-play --features memory-profile --lib \
  -- --exact memory::tests::observe_end_qualification_meta_includes_elapsed_when_stamped_before_write \
  --test-threads=1

cargo test -p host-play --features memory-profile --lib cohort -- --test-threads=1

cargo test -p host --lib responsiveness_cohort -- --test-threads=1

cargo check -p panel --features memory-profile
cargo check -p tui --features memory-profile
```

Production-path coverage (not mirrored helpers alone):

- `cohort_publisher_production_lifecycle_cursor_drain_finalize` — arm → complete → durable_write drain → cursor progress → absolute END+tail finish
- `poll_forces_final_observe_sample_when_last_sample_recent_at_end` — **production `Run::poll`**: last_sample recent at mono END forces final observe row then drain; **also** asserts poll-written observe-end qualification has non-null `cohort.observe_end_elapsed_s` (catches stamp-after-write regressions)
- `observe_end_qualification_meta_includes_elapsed_when_stamped_before_write` — write_qualification + unstamped attach null check (supplementary; not sole coverage)
- `finish_cohort_shutdown_joins_slot_before_terminal` — stop_slot + join via real Play path
- `cohort_tail_complete_uses_absolute_end_plus_tail_not_relative`
- `cohort_publisher_skips_when_responsiveness_profile_off` — fresh child `--exact`

## Inherited suite note (not this diff)

Full `cargo test -p host-play --features memory-profile --lib` can report 4 `prepare_*` failures from `mint_live_names` PID/serial truncation collisions when many tests share one process. Same failures on pre-publisher baseline. Isolated `--exact` runs of those four pass. Do not “fix” naming in this card; do not claim whole-suite green.

## Not claimed

- Native/live acceptance of a full memory harness run is out of this unit-test handoff.
- No new CLI/env flags for cohort.
- Python reader ownership is a later card; this freeze is the publisher contract only.

## Handoff

Ready for profile `reviewer` same-card re-review after round-2 envelope + production observe-end poll coverage.
