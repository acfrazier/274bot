# Direct per-bot owner capture — implementer report (t_ce3310e4)

## Summary

Implemented opt-in feature `memory-owner-capture` (schema `direct-owner-v1`) across api → client → host → script → host-play → tui. Default-off; runtime gate `BOT_MEMORY_OWNER_CAPTURE=1`. No product ownership retention; walkers emit capacity/length scalars into a nonblocking mailbox.

## Feature wiring

| Crate | Feature |
|-------|---------|
| api | `memory-owner-capture = []` |
| client (vendor) | `memory-owner-capture = []` |
| host | `memory-owner-capture = ["api/memory-owner-capture", "client/memory-owner-capture"]` |
| script | `memory-owner-capture = ["api/memory-owner-capture", "load"]` |
| host-play | `memory-owner-capture = ["memory-profile-no-alloc", api/host/script/client features]` |
| tui | forwards via `host-play/memory-owner-capture` |

## Modules / seams

**API**
- `crates/api/src/owner_capture.rs` — Budget, FieldRow, OwnerFragment, capacity helpers, schema constant
- `crates/api/src/snapshot_owner_capture.rs` — GameSnapshot capacity census (child of `snapshot` via `#[path]`)

**Client (vendor submodule, HEAD `3456edc8dabf7b25ada78110ffa56327af9f67a4`)**

Frozen-overlay applicability includes **added** helper files (not tracked-only diff).

| Path | role | bytes | sha256 |
|------|------|------:|--------|
| `vendor/fr-client-rust/crates/client/Cargo.toml` | feature flag | 929 | `4197688f08ed259ad96f2dc43dda6aa8433960769d30b3e4be51c8f1fd3ce26e` |
| `vendor/fr-client-rust/crates/client/src/io/packet.rs` | `Packet::owner_data_capacity` | 9976 | `7485b52922b60ec17f00e01e3dbd90280dfebdd3dd0021bf1b553ecfa361e718` |
| `vendor/fr-client-rust/crates/client/src/core/world.rs` | gate `mod world_owner_capture` | 43176 | `bae921d7ffd49c368af62feb736ca1a44c897ab9dff0f6ab7ee6091b8a7cb13d` |
| `vendor/fr-client-rust/crates/client/src/core/world_owner_capture.rs` | **NEW** `World::owner_payload` | 14250 | `04fd47fbd6cdd93642aa9ce61e07ca02b0e2ef2f8697b1f9a5122cdacc1e1cda` |

Submodule working tree status at report time:
- modified: Cargo.toml, world.rs, packet.rs
- untracked: world_owner_capture.rs
- branch tip: `3456edc8…` (original3456 HEAD baseline for root verification before submodule commit)

**Host**
- `crates/host/src/owner_capture.rs` — pre-observe staging, mailbox, Client public-table walker, world row map
- `crates/host/src/lib.rs` — pre-observe hook immediately before `observe`

**Script**
- `crates/script/src/fingerprint_owner_capture.rs` — SnapshotFingerprint capacity walker
- `crates/script/src/slot.rs` — `SlotScript::owner_payload` under existing lock after `on_is_up`

**Host-play**
- `crates/host-play/src/owner_capture.rs` — COW ifaces census, nav shell join, encoded buf note, phase publisher
- Seams: observe-closure entry (nav_snapshot), script_observe after on_is_up, encoded Vec before `post_snapshot`, memory harness init + phase A/B/C requests

**Tooling**
- `scripts/validate_owner_capture_jsonl.py` — schema/`direct-owner-v1` JSONL validator

## Tests run (this worktree)

```
cargo test -p api --features memory-owner-capture --lib owner_capture
  → 5 passed
cargo test -p host --features memory-owner-capture --lib owner_capture
  → 3 passed
cargo test -p script --features memory-owner-capture,load --lib fingerprint_owner
  → 2 passed
cargo test -p host-play --features memory-profile,memory-owner-capture --lib owner_capture
  → 4 passed
cargo test -p api --lib   # feature off
  → 5 content tests passed (owner_capture module absent)
```

Default-off: host/host-play without `memory-owner-capture` filter to 0 owner_capture tests (module cfg'd out).

## Known gaps / follow-ups for root

1. **Submodule commit not performed here** — client overlay remains dirty/untracked in `vendor/fr-client-rust`; root must verify all four client members (hashes above) before submodule commit + pin.
2. Full end-to-end JSONL emission under live harness not run in this task (unit/COW/mailbox only).
3. `Packet::owner_data_capacity` is available; host walker does not yet surface every optional packet buffer row in every path (world + client public tables + snapshot + fingerprint + COW are primary).
4. Phase requests currently use `SlotToken(0)` placeholder at harness boundaries; per-slot token bind is present for observe path.

## Acceptance map (plan §6)

| Criterion | Status |
|-----------|--------|
| Feature default-off | yes |
| Schema direct-owner-v1 | yes |
| Capacity formulas (Vec/String/grid/collision) | yes in api helpers |
| Both snapshot shells (host snapshot + nav) | yes |
| Fingerprint walker | yes |
| COW ifaces private vs shared | yes (unit tests) |
| Nonblocking mailbox + drop on full | yes (unit test) |
| Runtime off skips work | yes (unit test) |
| Unknown ≠ zero capacity | yes (Reason::OpaqueUnknown / incomplete) |
| Validator script | yes |
| Client new-file evidence | hashes above |

## Host branch

- Branch: `codex/memory-diagnostics`
- Task: `t_ce3310e4`
