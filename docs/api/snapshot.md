# Snapshot: families, gens, and reads

World reads come from `api::snapshot`/`api::query`. Reads borrow the last
rebuild — nothing deep-copies the world per read.

## Families and gens

`Family` mirrors the `ClientGens` counters on the client:

| Family | Bumped by (ServerProt) |
| --- | --- |
| `Npc` | `NPC_INFO` |
| `Player` | `PLAYER_INFO` |
| `Inv` | `UPDATE_INV_FULL`, `UPDATE_INV_PARTIAL`, `UPDATE_INV_STOP_TRANSMIT` |
| `Varp` | `VARP_SMALL`, `VARP_LARGE`, `VARP_SYNC` |
| `Stat` | `UPDATE_STAT`, `UPDATE_RUNENERGY`, `UPDATE_RUNWEIGHT` |
| `Chat` | `MESSAGE_GAME`, `MESSAGE_PRIVATE` |
| `Scene` | zone/loc/obj updates (`UPDATE_ZONE_*`, `LOC_*`, `OBJ_*`, `MAP_*`) |

`REBUILD_NORMAL` and `LOGOUT` bump **every** family (new scene / full reset);
unknown opcodes bump nothing.

## GameSnapshot

```rust
let mut snap = GameSnapshot::new();
snap.rebuild_family(&client, Family::Npc);   // true iff the gen moved
snap.npcs()                                  // &[NpcView], last rebuild
snap.gens()                                  // ClientGens this snapshot reflects
```

- `rebuild_family` copies only the family whose gen moved and returns true
  iff it did. Only `Npc` has a view cache today; the other families track
  their counter so a later view can detect movement.
- `NpcView` is an owned copy of one live NPC slot, keyed by its **slot index**
  in `Client.npc`. Identity is the slot index, stable across the client's
  in-place walk mutations, so a reader holding the previous slice is never
  invalidated.

## Queries (`api::query`)

Borrowing lookups over the last rebuild, no allocation:

```rust
npc_by_index(snap.npcs(), 7)        // Option<&NpcView> by slot index
npcs_at(snap.npcs(), x, z)          // impl Iterator<&NpcView> on a tile
```

## Settle evidence (`api::settle`)

`Settle` folds before/after deltas into evidence that an interaction landed:
`arrived`, `item_delta` (inv counts), `xp_gained` (positive-only skill gains),
`modal_opened`/`modal_closed`, within a tick/ms budget (defaults 10 ticks /
2000 ms). `Settle::done()` is true when some arm is armed **and** the budget
held.
