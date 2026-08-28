# Snapshot: the full world read model

World reads come from `api::snapshot` (the gen-stamped `GameSnapshot`) and
`api::snapshot::ReadContext` (the query-facing read surface). Reads borrow
the last rebuild — nothing deep-copies the world per read.

## Families and gens

`Family` mirrors the `ClientGens` counters on the client. The first 11 map
1:1; the rest are additive and gate on the same underlying gens.

| Family | Bumped by (ServerProt) |
| --- | --- |
| `Npc` | `NPC_INFO` |
| `Player` | `PLAYER_INFO` |
| `Inv` | `UPDATE_INV_FULL/PARTIAL/STOP_TRANSMIT` |
| `Varp` | `VARP_SMALL/LARGE/SYNC` |
| `Stat` | `UPDATE_STAT`, `UPDATE_RUNENERGY`, `UPDATE_RUNWEIGHT` |
| `Chat` | `MESSAGE_GAME`, `MESSAGE_PRIVATE` |
| `Scene` | zone/loc/obj updates (`UPDATE_ZONE_*`, `LOC_*`, `OBJ_*`, `MAP_*`) |
| `Iface` | every `IF_*` packet + `P_COUNTDIALOG` |
| `Camera` | `CAM_LOOKAT/SHAKE/MOVETO/RESET` |
| `MapFlag` | `UNSET_MAP_FLAG` |
| `World` | `SET_MULTIWAY` |

`REBUILD_NORMAL` and `LOGOUT` bump every family. The additive iface-derived
families (`Loc`, `GroundItem`, `Equipment`, `Bank`, `Widgets`, `SideTabs`,
`Trade`, `ChatOptions`, `MakeProducts`, `QuestStatuses`, `Controls`,
`Modals`, `Menu`) rebuild on `Iface`/`Inv`/`Scene` gen movement via private
per-family gates. All 25 families are rebuilt by `rebuild`/`rebuild_family`.

## GameSnapshot

```rust
let mut snap = GameSnapshot::new();
snap.rebuild(&client);                       // rebuild every dirty family
snap.tile()                                  // Option<(x,z,level)> route-based world tile
snap.inv() / snap.inv_count(id)              // real obj ids (stored - 1)
snap.local_player() / snap.npcs() / snap.locs() / snap.ground_items()
snap.inventory() / snap.equipment() / snap.bank() / snap.widgets() / snap.side_tabs()
snap.stats() / snap.varps() / snap.chat() / snap.world() / snap.scene() / snap.camera()
```

- `rebuild(&mut self, client: &Client)` takes an immutable `&Client` (the
  ground-item sweep uses the client's immutable `LinkList` iterator).
- World-derived families (npc, player, loc, ground item, scene, world) are
  gen-gated; iface-derived families re-read the materialized `client.ifaces`
  on their gate.
- `snap.tile()` is the canonical route-based world tile
  (`base + route_x[0]`, level = `minusedlevel`). Entity-pixel tiles remain
  available on `local_player().actor.tile` for visuals.

## View structs

`api::snapshot` owns the view structs the query DSL and scripts consume:
`NpcView`, `PlayerView`, `LocalPlayerView` (energy/weight), `LocView`
(typecode/layer/shape/angle + resolved name/actions), `GroundItemView`,
`ItemView` (`container`/`action_family`/`slot`/`count`/`def`),
`WidgetView`, `SideTabView`, `StatView`, `VarpView`, `ChatLineView`,
`SceneView` (collision flags), `WorldStateView`, `CameraView`,
`MapFlagView`, `TradeView`, `ModalView`, `QuestStatusView`,
`MakeProductView`, `ToggleControlsView`, plus `WorldTile`/`LocalTile` and
`ItemDefView`/`LocDefView` (from `api::obj_names`).

## ReadContext

`ReadContext<'a>` wraps `&'a GameSnapshot` and exposes the query-facing
surface: `tick`, `attached`, `ingame`, `scene_state`, `local_player`,
`self_slot`, `stats`, `npcs`, `players`, `locs`, `ground_items`,
`inventory`, `equipment`, `inventory_capacity`, `bank`, `bank_side_items`,
`bank_component_id`, `chat`, `chat_options`, `chat_continue_component_id`,
`make_products`, `quest_statuses`, `widgets`, `side_tabs`, `component`,
`varps`, `world`, `scene`, `camera`, the three trade containers, `modals`,
`count_dialog_open`, `active_side_tab`, `login_message`, `menu_entries`,
`main_modal_texts`, `chat_modal_texts`, `run_controls`, `retaliate_controls`,
`world_tile`, `varp`, `component_items`, `component_text`,
`component_model_obj_id`, `side_tab_interface`. `ReadContext` is `Copy`
(a single reference).
