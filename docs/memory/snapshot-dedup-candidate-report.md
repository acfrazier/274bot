# Snapshot-dedup candidate report (corrective ownership + census)

Task lineage: `t_cc437480` (slot-instance ownership) → `t_df016942` (real-owner census + honest scratch).

## Summary

The process-global `process_slot_table` / `attach_owner_for_slot(username)` path is gone. Ownership is **slot-instance tokens** in a **Play-local** `SlotDedupDirectory`. Host, host-play, and panel join the same per-slot instance for the slot lifetime. Memory-profile publishes bounded allocation-style diagnostics (not RSS).

**Census correction (`t_df016942`):** `account_registry_live` no longer synthesizes owners from `Arc::strong_count`. It upgrades each registered cursor's family weaks once under one locked snapshot, counts unique payload once per distinct body identity, and applies the private counterfactual once per real registered owner holding that identity. Measurement clones cannot fabricate overlap. Scratch peak is `Option` / JSON `null` when unmeasured, with `scratch_peak_available: false` on the memory-profile envelope. Slots and aggregate come from one `census_sample`.

## Three corrective roots

### 1. True slot-instance registry ownership

| Before | After |
| --- | --- |
| Process-wide `name → SlotFamilyRegistry` | `SlotDedupInstance` (token) + `SlotDedupDirectory` (Play-local map) |
| Same username across concurrent Plays collided / leaked | Each `Play` owns its own directory; same username on two Plays never shares |
| Stop removed process table entry only | `Play::stop_slot` removes directory entry; panel clears `nav_states` |

Wiring:

- **host-play**: on `spawn_slot`, mint `SlotDedupInstance`, `dedup_directory.install(username)`, pass clone into `spawn_slot_thread` → `Host::run_client(..., Some(instance), ...)` and `nav_snapshot.attach_dedup(instance.attach())`.
- **host**: `run_client` accepts optional `SlotDedupInstance`; attaches on the primary `GameSnapshot`. Non-owning call sites pass `None`.
- **panel**: shares the same Play directory Arc (created before `run_with_io`, injected via `Play::replace_dedup_directory`). Nav attach looks up `nav_dedup.get(name)` and joins; does not mint a global registry.

### 2. Bounded memory-profile diagnostics

Under `memory-profile` + `snapshot-dedup`, each sample line gains:

```json
"snapshot_dedup": {
  "scratch_peak_available": false,
  "slots": [ /* SlotDedupDiagnostic per live instance */ ],
  "aggregate": { /* AllocationAccount from the same census rows */ }
}
```

Fields include unique/duplicate nested payload bytes, live owner count, unique body count, registry metadata, Arc header estimate (`ArcInner` strong+weak), weak-slot bytes, equality hits/misses/work, walks, quiet skips, publishes.

Accounting rules:

- Exclusive unique payload uses capacity-aware `*_payload_bytes_vec` (spare `Vec` capacity counted once per unique body).
- Live census (`account_registry_live`) uses **registered cursor weak identity/occurrences** only — no strong-count owner synthesis.
- Arc header is layout estimate only — not RSS.
- No strong history; weak registry only.
- No process-global username walk.
- `scratch_peak_bytes` is `null` / `None` when unmeasured (never silent `0`). Harness sets `scratch_peak_available: false` until a bounded peak probe exists.
- `SlotDedupDirectory::census_sample` derives slots + aggregate from one sample.

### 3. Real multi-owner CX / lifecycle coverage

`cargo test -p api --features snapshot-dedup --test snapshot_dedup` including:

- two-owner share / miss
- side_tabs + loc share
- cursor unregister reclaim
- allocation account headers + duplicate savings
- directory isolation across Plays
- slot restart replace without leak
- diagnostic JSON shape + honest null scratch + single census
- **CX1–CX6** named tests
- **asymmetric census fixtures**: two distinct capacities → 0 duplicate; three-owner shared+distinct exact math; dropped cursor / dead weak cleanup

Proto suite remains separate historical CX fixtures: `cargo test -p api --features snapshot-dedup --test snapshot_dedup_proto`.

## Feature-off

Default build: no Arc family storage, no registry, no diagnostics key.

## Explicit non-claims

- No RSS / process RSS delta claims.
- No strong body history or cross-slot body cache.
- No 10 MiB live population run in this task (unit/mechanism coverage only); diagnostics are shaped for that discriminator when the harness is run with real population. Overlap evidence must come from real registered owners, not strong-count estimates.
- Scratch peak remains unmeasured in the harness (`scratch_peak_available: false`, JSON null) until a bounded peak probe is wired.

## Residual risk

- Panel attaches only if the instance is already installed at first nav entry; race if a frame ran before `spawn_slot` install is theoretical (spawn installs before thread start).
- Feature-on host/host-play lib tests do not assert Arc identity end-to-end through a full client frame; that remains the role of mechanism + proto tests and future live memory-profile samples.

## Changed files (census corrective)

- `crates/api/src/snapshot_dedup.rs` — real-owner census, `Option` scratch, `census_sample`
- `crates/api/tests/snapshot_dedup.rs` — asymmetric / three-owner / cleanup fixtures
- `crates/host-play/src/memory.rs` — single census + availability flag
- `docs/memory/snapshot-dedup-candidate-report.md` — this file
