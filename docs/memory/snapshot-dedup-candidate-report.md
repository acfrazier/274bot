# Snapshot-dedup candidate report (corrective ownership)

Task: `t_cc437480` (corrective follow-up to production landing `t_b1e57f6d` / `7c05f5c`).

## Summary

The process-global `process_slot_table` / `attach_owner_for_slot(username)` path is gone. Ownership is now **slot-instance tokens** installed in a **Play-local** `SlotDedupDirectory`. Host, host-play, and panel join the same per-slot instance for the slot lifetime. Memory-profile publishes bounded allocation-style diagnostics (not RSS). CX1–CX6 style coverage lives in `crates/api/tests/snapshot_dedup.rs` plus existing proto CX suite.

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
  "slots": [ /* SlotDedupDiagnostic per live instance */ ],
  "aggregate": { /* AllocationAccount sum */ }
}
```

Fields include unique/duplicate nested payload bytes, live owner count, unique body count, registry metadata, Arc header estimate (`ArcInner` strong+weak), weak-slot bytes, equality hits/misses/work, walks, quiet skips, publishes, scratch peak (caller-supplied; harness currently passes `0` until a peak probe is wired).

Accounting rules:

- Exclusive unique payload uses capacity-aware `*_payload_bytes_vec` (spare `Vec` capacity counted once per unique body).
- Arc header is layout estimate only — not RSS.
- No strong history; weak registry only.
- No process-global username walk.

### 3. Real multi-owner CX / lifecycle coverage

`cargo test -p api --features snapshot-dedup --test snapshot_dedup` — 15 tests including:

- two-owner share / miss
- side_tabs + loc share
- cursor unregister reclaim
- allocation account headers + duplicate savings
- directory isolation across Plays
- slot restart replace without leak
- diagnostic JSON shape
- **CX1–CX6** named tests (savings, partial share, multi-family, miss, lifetime boundary, capacity/headers/no-RSS)

Proto suite remains green: `cargo test -p api --features snapshot-dedup --test snapshot_dedup_proto` (12 tests, CX1–CX6 gate-history cases).

## Feature-off

Default build: no Arc family storage, no registry, no diagnostics key. Verified:

- `cargo test -p api --test snapshot_dedup` → 0 tests (cfg-gated)
- `cargo check -p host -p host-play -p panel -p api`
- `cargo test -p host --lib` (179 pass)
- `cargo test -p host-play --lib` (115 pass)

## Feature-on verification

- `cargo test -p api --features snapshot-dedup --test snapshot_dedup` — 15 pass
- `cargo test -p api --features snapshot-dedup --test snapshot_dedup_proto` — 12 pass
- `cargo check -p host --features snapshot-dedup`
- `cargo check -p host-play --features 'snapshot-dedup,memory-profile'`
- `cargo check -p panel --features snapshot-dedup`
- `cargo test -p host --features snapshot-dedup --lib` — 179 pass
- `cargo test -p host-play --features 'snapshot-dedup,memory-profile' --lib` — 176 pass

## Changed files (primary)

- `crates/api/src/snapshot_dedup.rs` — instance/directory model, allocation account, diagnostics
- `crates/api/src/snapshot.rs` — attach path, quiet-skip registry bumps, intern walk dual-write, test intern helpers
- `crates/api/tests/snapshot_dedup.rs` — ownership / CX / diagnostic tests
- `crates/api/Cargo.toml` — `serde_json` dev-dep for diagnostic serialize tests
- `crates/host/src/lib.rs` — optional instance on `run_client`
- `crates/host-play/src/lib.rs` — Play directory, spawn install, stop remove, accessors
- `crates/host-play/src/memory.rs` — sample `snapshot_dedup` JSON
- `crates/panel/src/session.rs` — shared directory attach + stop clears nav_states
- `docs/memory/snapshot-dedup-candidate-report.md` — this file

## Explicit non-claims

- No RSS / process RSS delta claims.
- No strong body history or cross-slot body cache.
- No 10 MiB live population run in this task (unit/mechanism coverage only); diagnostics are shaped for that discriminator when the harness is run with real population.
- Scratch peak is plumbed (`scratch_peak_bytes` arg) but harness currently samples with `0` until a peak probe is added.

## Residual risk

- Panel attaches only if the instance is already installed at first nav entry; race if a frame ran before `spawn_slot` install is theoretical (spawn installs before thread start).
- Diagnostic holder synthesis from unique bodies × strong_count is an estimate when external owner lists are absent; multi-owner live samples still report positive duplicate savings when Arcs are shared.
- Feature-on host/host-play lib tests do not assert Arc identity end-to-end through a full client frame; that remains the role of mechanism + proto tests and future live memory-profile samples.
