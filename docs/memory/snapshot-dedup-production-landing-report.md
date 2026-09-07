# Snapshot-dedup production landing report

## Summary

Landed the approved A1 rebuild-edge exact family body dedup into production
`GameSnapshot` ownership paths under opt-in feature `snapshot-dedup`.

Feature-off keeps original `Vec` storage and rebuild behavior with no equality
or registry work. Feature-on stores Widgets / SideTabs / Loc as `Arc<Vec<_>>`,
interns completed rebuild candidates into a per-slot weak registry, and lets
multiple live owners (host SlotLoop, host-play nav_snapshot, panel nav_states)
share identical bodies by pointer.

## Storage and rebuild

- `GameSnapshot.widgets` / `side_tabs` / `loc`:
  - feature-off: `Vec`
  - feature-on: `Arc<Vec>`
- Getters still return `&[T]` (`as_slice()` when Arc).
- Rebuild paths still run the original gate + walk. On feature-on:
  - quiet gate miss → `quiet_skips++`, no equality
  - rebuild into a unique body (`Arc::get_mut` or fresh Arc)
  - after successful rebuild, if a `DedupHandle` is attached, take the body and
    `intern_*` into the slot registry
- Serde: `serde` feature `rc` so Arc serializes as the inner payload (same JSON
  shape as a bare Vec). Dedup handle/counters are `#[serde(skip)]`.

## Registry and owners

- `api::snapshot_dedup::SlotFamilyRegistry` — weak slots per registered cursor;
  exact equality only; overwrite on replace; unregister on drop.
- `process_slot_table()` — process-wide name→registry map (not a body cache).
- `attach_owner_for_slot(name)` — get-or-create registry + register cursor.
- Owner attach sites:
  - `host::Host::run_client` → `slot.snapshot.attach_dedup(...)`
  - `host-play` slot thread → `nav_snapshot.attach_dedup(...)`
  - `panel::session` nav_states insert → `snap.attach_dedup(...)`
- `Play::stop_slot` removes the name from the process table after join.
- `GameSnapshot::Drop` detaches/unregisters the handle.

## Feature plumbing

| crate | feature |
| --- | --- |
| api | `snapshot-dedup` |
| host | `snapshot-dedup = ["api/snapshot-dedup"]` |
| host-play | `snapshot-dedup = ["api/snapshot-dedup", "host/snapshot-dedup"]` |
| panel | `snapshot-dedup = ["api/…", "host/…", "host-play/…"]` |
| tui | same as panel |

Default builds leave the feature off.

## Tests run

```
cargo test -p api --test snapshot                                 # 60 ok (feature-off)
cargo test -p api --features snapshot-dedup --test snapshot       # 60 ok
cargo test -p api --test snapshot_dedup_proto                     # 12 ok
cargo test -p api --features snapshot-dedup --test snapshot_dedup # 7 ok
cargo check -p host -p host-play -p panel                         # ok
cargo check -p host -p host-play -p panel --features snapshot-dedup # ok
```

Production candidate coverage (`tests/snapshot_dedup.rs`, feature-gated):

1. Registry equal share / unequal no-share
2. Quiet rebuild: no extra walks or equality; quiet_skips advance
3. Two owners equal rebuild share widgets/side_tabs/loc Arcs
4. Two owners unequal widgets do not share
5. Owner teardown unregisters; re-attach works
6. Allocation accounting: unique body bytes < private per-owner sum; 1 unique Arc
7. Process table isolates slot names

## Scope limits (intentional)

- Only Widgets, SideTabs, Loc (A1 candidates). No other families.
- No global cross-client body cache; no strong history beyond live owners.
- No generation-only share stamp; no read-path deep compare.
- Diagnostics counters live on the snapshot when feature-on; optional for
  memory-profile consumers later (not required on quiet feature-on).
- Prototype module `snapshot_dedup_proto` retained for historical discriminator
  fixtures; production path is `snapshot_dedup` + GameSnapshot Arc storage.

## Files touched

- `crates/api/Cargo.toml`, `src/lib.rs`, `src/snapshot.rs`, `src/snapshot_dedup.rs` (new)
- `crates/api/tests/snapshot_dedup.rs` (new)
- `crates/host/Cargo.toml`, `src/lib.rs`
- `crates/host-play/Cargo.toml`, `src/lib.rs`
- `crates/panel/Cargo.toml`, `src/session.rs`
- `crates/tui/Cargo.toml`
- this report
