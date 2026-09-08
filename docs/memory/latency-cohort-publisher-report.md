# Latency cohort publisher — implementer report

Task: `t_5992fd7a`  
Branch/worktree: `codex/latency-cohort-publisher`  
Companion: `docs/memory/current-latency-companion-plan.md`, `docs/memory/latency-cohort-producer-report.md`

## Summary

Additive fixed-window cohort journal publisher on the memory harness path. Active only when the existing responsiveness profile is already enabled at observe-start (no new CLI/env flags). `begin_armed` activation order fixed (OPT_IN before START under ledger lock). Operator integration-review checklist resolved, including a production poll scheduling gap for the final observe row.

## Changes

| Area | Path | What |
| --- | --- | --- |
| Activation | `crates/host/src/responsiveness_cohort.rs` | `begin_armed`: enable OPT_IN before START sample under lock; harness ledger reset surface; test `begin_armed_enables_before_start_sample` |
| Publisher | `crates/host-play/src/memory.rs` | `CohortPublisher` on `Run`; arm/drain/attach/finish; production helpers |
| Poll mut | `memory.rs`, `crates/panel/src/app.rs`, `crates/tui/src/bin.rs` | `poll(&mut self, play: &mut Play)`; panel/tui `play.as_mut()` so stop_slot/join can run before `process::exit` |
| Play test helper | `crates/host-play/src/lib.rs` | `#[cfg(test)] test_install_stoppable` dummy slot for join path |

## Operator checklist (codex-orch)

1. **attach before durable write** — `durable_write_sample_line`: drain journal → `attach_cohort_sample_refs` → `writeln`. Qualification path attaches before write.
2. **Same immutable START/END as activation** — `arm_cohort_at_observe_start` stores `begin_armed` boundaries; phase/tail use those fields only (not a later poll-entry now + re-begin).
3. **Observe-end final row before drain** — `enter_drain_after_sample` defers drain flip until after the sample write; `sample_due` includes that flag so a recent `<1s` last_sample still forces the final phase=`"observe"` row. Drain is set only after that write.
4. **Stop at predeclared absolute END+tail** — `cohort_tail_complete(boundaries, now_mono)`: `now_mono >= end_mono_ns + tail_ns` (`DEFAULT_TAIL_NS = 5s`). Not Instant-relative drain or 60s teardown as the finite tail.
5. **Cohort-off preserved** — arm only if `responsiveness_profile::enabled()`; child-process `--exact` test.

## Drain / loss counters

`drain_cohort_journal` tracks `last_loss_count` / `last_journal_overflow_n` and emits counter-only batches when loss/overflow advances with an empty event payload.

## Tests run (verified this handoff)

```text
cargo test -p host-play --features memory-profile --lib \
  -- --exact memory::tests::poll_forces_final_observe_sample_when_last_sample_recent_at_end \
  --test-threads=1
→ 1 passed (operator-confirmed session55582; re-run this session exit 0)

cargo test -p host-play --features memory-profile --lib cohort -- --test-threads=1
→ 4 passed (lifecycle, skips-off child, tail absolute, finish join)

cargo test -p host --lib responsiveness_cohort -- --test-threads=1
→ 24 passed

cargo check -p panel --features memory-profile
cargo check -p tui --features memory-profile
→ ok
```

Production-path coverage (not mirrored helpers alone):

- `cohort_publisher_production_lifecycle_cursor_drain_finalize` — arm → complete → durable_write drain → cursor progress → absolute END+tail finish
- `poll_forces_final_observe_sample_when_last_sample_recent_at_end` — **production `Run::poll`**: last_sample recent at mono END forces final observe row then drain (closes the 1 Hz gate gap)
- `finish_cohort_shutdown_joins_slot_before_terminal` — stop_slot + join via real Play path
- `cohort_tail_complete_uses_absolute_end_plus_tail_not_relative`
- `cohort_publisher_skips_when_responsiveness_profile_off` — fresh child `--exact` (process-global profile)

## Not claimed

- Native/live acceptance of a full memory harness run is out of this unit-test handoff.
- No new CLI/env flags for cohort.

## Handoff

Ready for profile `reviewer` same-card review.
