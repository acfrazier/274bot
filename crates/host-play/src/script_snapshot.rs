use std::sync::Arc;

use api::snapshot::{GameSnapshot, WorldTile};
use nav::world::NavWorld;

use super::{route_inspect, script_slot, PostedWalkOutcome, ScriptWall};
/// The FlatBuffer snapshot blob posted into a Load isolate each
/// PLAYER_INFO (schema: `crates/script/schema/isolate.fbs`): `tick, here,
/// ingame, inv, stats, booths, banks, bank, bank_side, bank_open,
/// bank_loaded, hold, ours` — the exact fields the shim
/// Game/Inventory/Skills/Bank/Banking/EventSignal read, and nothing else
/// (no World clone). `here` is the local player's tile `{x, z, level}`
/// (absent when the body decoded none); `inv` rows carry the obj's
/// resolved name (`None` when the shared table has none — a name a script
/// queries never matches); `stats` rows carry the snapshot's stat
/// index/name/xp; `booths` are the scene locs whose actions include
/// `Use-quickly` (a name/action a script interacts with never appears
/// otherwise); `banks` are the packed bank stands (`{name, x, z, level,
/// kind: booth|npc, op, choose}`) the shim walks to; `bank`/`bank_side`
/// are the open bank's withdraw/deposit rows with the obj's resolved
/// name (`None` when the table has none — a deposit/withdraw by that name
/// never matches); `hold`/`ours` are the guardian's published status that
/// `EventSignal.pending()` reads.
///
/// Posts are DELTAS: `tick` is always carried; every other field only
/// when it changed vs `last` (the per-slot last-post fingerprint, `None`
/// right after Start — the first post is then the full keyframe). A 50+
/// isolate wall never resends unchanged inv/bank/stats/booths/packed
/// banks. Packed `banks` are additionally re-posted when `force_banks`
/// (the `NavWorld` identity changed) even though the stand list is
/// byte-identical. Returns the blob and the fingerprint to store as the
/// new last-post baseline.
/// Tests encode through a one-shot builder; the live observe path uses
/// [`with_script_snapshot_input`] + the slot's [`script::isolate_fb::IsolateBuf`].
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn script_snapshot_fb(
    last: Option<&script::isolate_fb::SnapshotFingerprint>,
    force_banks: bool,
    tick: u64,
    here: Option<(i32, i32, i32)>,
    ingame: bool,
    inv: Option<&[(i32, i32)]>,
    snapshot: Option<&GameSnapshot>,
    obj_names: Option<&api::obj_names::ObjNames>,
    world: Option<&NavWorld>,
    hold: bool,
    ours: bool,
    teleports_enabled: bool,
) -> (Vec<u8>, script::isolate_fb::SnapshotFingerprint) {
    with_script_snapshot_input(
        tick,
        here,
        ingame,
        inv,
        snapshot,
        obj_names,
        world,
        None,
        hold,
        ours,
        teleports_enabled,
        0,
        false,
        0,
        false,
        0,
        false,
        None,
        PostedWalkOutcome::default(),
        route_inspect::PostedInspect::default(),
        |input, native| {
            script::isolate_fb::encode_snapshot_delta_with_native(last, input, native, force_banks)
        },
    )
}

/// Note-mode landing id for an unnoted obj (`ObjNames` reverse cert), else
/// the snapshot def's raw `certlink`.
fn posted_cert(
    obj_names: Option<&api::obj_names::ObjNames>,
    def: &api::obj_names::ItemDefView,
) -> i32 {
    obj_names
        .and_then(|n| n.item(def.id))
        .map(|d| d.certificate_link)
        .filter(|&c| c >= 0)
        .unwrap_or(def.certificate_link)
}

fn posted_cert_id(obj_names: Option<&api::obj_names::ObjNames>, id: i32) -> i32 {
    obj_names
        .and_then(|n| n.item(id))
        .map(|d| d.certificate_link)
        .filter(|&c| c >= 0)
        .unwrap_or(-1)
}
pub(super) struct PackedReach {
    pub(super) view: Arc<api::query::ReachQueryView>,
    pub(super) flood: Option<Arc<api::query::ReachFlood>>,
    pub(super) stamp: u64,
}

pub(super) fn pack_cached_reach(
    cache: &mut api::query::ReachPackCache,
    snapshot: Option<&GameSnapshot>,
    here: Option<(i32, i32, i32)>,
    world: Option<&NavWorld>,
    canlight: Option<&[u64]>,
) -> PackedReach {
    let Some(s) = snapshot else {
        return PackedReach {
            view: Arc::new(api::query::ReachQueryView::unavailable()),
            flood: None,
            stamp: 0,
        };
    };
    let canlight_plane = canlight.and_then(|bits| {
        world.map(|w| api::query::CanlightPlane {
            bits,
            origin_x: w.collision.origin.x,
            origin_z: w.collision.origin.z,
            width: w.collision.width as i32,
            height: w.collision.height as i32,
        })
    });
    let here_tile = here.map(|(x, z, level)| WorldTile { x, z, level });
    let key = api::query::ReachCacheKey::from_parts(
        s.scene_generation(),
        s.loc_static_generation(),
        s.loc_model_stamp(),
        s.scene(),
        here_tile,
        canlight_plane,
    );
    let stamp = key.stamp();
    if cache.contains(&key) {
        return PackedReach {
            view: cache.view_arc(),
            flood: cache.flood_arc(),
            stamp,
        };
    }
    let flood = here_tile
        .and_then(|tile| {
            if !s.scene().available {
                return None;
            }
            api::query::SceneQuery::new(s.scene(), Some(tile)).flood_reach()
        })
        .map(Arc::new);
    PackedReach {
        view: cache.pack(key, s.scene(), flood.clone(), canlight_plane),
        flood,
        stamp,
    }
}

/// The slot's cached reach view for `here`: the follow's arrival probe.
/// Same cache and key as the isolate post, so a frame that already posted
/// is a hit; otherwise this one flood is the one the next post reuses.
/// No script slot (no script walk to settle) reads as an unavailable view.
pub(crate) fn slot_arrival_reach(
    scripts: &ScriptWall,
    name: &str,
    snapshot: &GameSnapshot,
    here: Option<(i32, i32, i32)>,
    world: Option<&NavWorld>,
    canlight: Option<&[u64]>,
) -> Arc<api::query::ReachQueryView> {
    let Some(slot) = script_slot(scripts, name) else {
        return Arc::new(api::query::ReachQueryView::unavailable());
    };
    let Ok(mut slot) = slot.lock() else {
        return Arc::new(api::query::ReachQueryView::unavailable());
    };
    pack_cached_reach(
        slot.reach_pack_cache(),
        Some(snapshot),
        here,
        world,
        canlight,
    )
    .view
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)] // test wrapper over shorts packer
pub(crate) fn with_script_snapshot_input<R>(
    tick: u64,
    here: Option<(i32, i32, i32)>,
    ingame: bool,
    inv: Option<&[(i32, i32)]>,
    snapshot: Option<&GameSnapshot>,
    obj_names: Option<&api::obj_names::ObjNames>,
    world: Option<&NavWorld>,
    npc_boxes: Option<&[script::isolate_fb::NpcBoxInput]>,
    hold: bool,
    ours: bool,
    teleports_enabled: bool,
    withdraw_x_result_seq: u64,
    withdraw_x_result: bool,
    withdraw_load_result_seq: u64,
    withdraw_load_result: bool,
    bank_op_result_seq: u64,
    bank_op_result: bool,
    canlight: Option<&[u64]>,
    walk_outcome: PostedWalkOutcome,
    inspect: route_inspect::PostedInspect,
    f: impl FnOnce(
        &script::isolate_fb::SnapshotInput<'_>,
        script::isolate_fb::NativeFactsInput<'_>,
    ) -> R,
) -> R {
    with_script_snapshot_input_shorts(
        tick,
        here,
        ingame,
        inv,
        snapshot,
        obj_names,
        world,
        npc_boxes,
        hold,
        ours,
        teleports_enabled,
        withdraw_x_result_seq,
        withdraw_x_result,
        withdraw_load_result_seq,
        withdraw_load_result,
        bank_op_result_seq,
        bank_op_result,
        canlight,
        walk_outcome,
        &[],
        inspect,
        None,
        None,
        0,
        f,
    )
}

/// The same pack with the walk outcome's navigator-named gate shorts: the live
/// observe path takes this entry point, and [`with_script_snapshot_input`] is
/// this one with none. The rows are a borrow of THIS frame and are handed to `f`
/// inside the facts it already receives, because a caller cannot attach them
/// through the closure itself: that parameter is higher-ranked, so a borrow of
/// the caller's frame could never satisfy it.
#[allow(clippy::too_many_arguments, unused_assignments)]
pub(crate) fn with_script_snapshot_input_shorts<R>(
    tick: u64,
    here: Option<(i32, i32, i32)>,
    ingame: bool,
    inv: Option<&[(i32, i32)]>,
    snapshot: Option<&GameSnapshot>,
    obj_names: Option<&api::obj_names::ObjNames>,
    world: Option<&NavWorld>,
    npc_boxes: Option<&[script::isolate_fb::NpcBoxInput]>,
    hold: bool,
    ours: bool,
    teleports_enabled: bool,
    withdraw_x_result_seq: u64,
    withdraw_x_result: bool,
    withdraw_load_result_seq: u64,
    withdraw_load_result: bool,
    bank_op_result_seq: u64,
    bank_op_result: bool,
    canlight: Option<&[u64]>,
    walk_outcome: PostedWalkOutcome,
    walk_missing_carry: &[script::isolate_fb::CarryInput<'_>],
    inspect: route_inspect::PostedInspect,
    precomputed_reach: Option<&api::query::ReachQueryView>,
    precomputed_flood: Option<&api::query::ReachFlood>,
    reach_stamp: u64,

    f: impl FnOnce(
        &script::isolate_fb::SnapshotInput<'_>,
        script::isolate_fb::NativeFactsInput<'_>,
    ) -> R,
) -> R {
    use script::isolate_fb::{
        BankApproachInput, BankStandInput, ChatLineInput, ChatOptionInput, CollisionViewInput,
        CombatStyleInput, ItemRowInput, MainModalTextsInput, MakeButtonInput, MakeProductInput,
        NativeFactsInput, NearestBoothInput, PuzzleBoardInput, QuestStatusInput, ReachViewInput,
        SceneEntityInput, SideTabIfaceInput, SnapshotInput, StatInput, TileInput, VarpInput,
        WidgetTextInput,
    };

    let owned_flood: Option<api::query::ReachFlood>;
    let flood: Option<&api::query::ReachFlood> = if precomputed_reach.is_some() {
        owned_flood = None;
        precomputed_flood
    } else {
        owned_flood = snapshot.and_then(|s| {
            let (x, z, level) = here?;
            if !s.scene().available {
                return None;
            }
            api::query::SceneQuery::new(s.scene(), Some(WorldTile { x, z, level })).flood_reach()
        });
        owned_flood.as_ref()
    };
    let owned_reach: Option<api::query::ReachQueryView>;
    let reach_pack: &api::query::ReachQueryView = match precomputed_reach {
        Some(view) => {
            owned_reach = None;
            view
        }
        None => {
            let canlight_plane = canlight.and_then(|bits| {
                world.map(|w| api::query::CanlightPlane {
                    bits,
                    origin_x: w.collision.origin.x,
                    origin_z: w.collision.origin.z,
                    width: w.collision.width as i32,
                    height: w.collision.height as i32,
                })
            });
            owned_reach = Some(
                snapshot
                    .map(|s| api::query::pack_reach_query_plane(s.scene(), flood, canlight_plane))
                    .unwrap_or_else(api::query::ReachQueryView::unavailable),
            );
            owned_reach.as_ref().unwrap()
        }
    };

    let reach = ReachViewInput {
        available: reach_pack.available,
        base_x: reach_pack.base_x,
        base_z: reach_pack.base_z,
        level: reach_pack.level,
        width: reach_pack.width,
        height: reach_pack.height,
        walkable: &reach_pack.walkable,
        reachable: &reach_pack.reachable,
        reachable_adj: &reach_pack.reachable_adj,
        exact_rank: &reach_pack.exact_rank,
        adjacent_rank: &reach_pack.adjacent_rank,
        step: &reach_pack.step,
        canlight: &reach_pack.canlight,
        stamp: reach_stamp,
    };
    let here = here.map(|(x, z, level)| TileInput { x, z, level });
    let entity_reach = |x: i32, z: i32, level: i32| -> (bool, bool) {
        let tile = WorldTile { x, z, level };
        (
            api::query::ReachQueryView::bit_at(
                &reach_pack.reachable,
                reach_pack.width,
                reach_pack.height,
                reach_pack.base_x,
                reach_pack.base_z,
                reach_pack.level,
                tile,
            ),
            api::query::ReachQueryView::bit_at(
                &reach_pack.reachable_adj,
                reach_pack.width,
                reach_pack.height,
                reach_pack.base_x,
                reach_pack.base_z,
                reach_pack.level,
                tile,
            ),
        )
    };
    let scene_entity_target = |target: Option<&api::snapshot::ActorTargetView>| -> (i32, i32) {
        match target {
            None => (0, -1),
            Some(t) => {
                let kind = match t.kind {
                    api::snapshot::ActorKind::Npc => 1,
                    api::snapshot::ActorKind::Player => 2,
                };
                (kind, t.index as i32)
            }
        }
    };
    let inv_ops_store: Vec<Vec<String>>;
    let inv: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        if s.inventory().is_empty() {
            inv_ops_store = Vec::new();
            inv.map(|rows| {
                rows.iter()
                    .map(|(id, count)| ItemRowInput {
                        name: obj_names.and_then(|names| names.name(*id)),
                        count: *count,
                        id: *id,
                        ops: &[],
                        noted: false,
                        cert: posted_cert_id(obj_names, *id),
                        component_id: -1,
                        slot: -1,
                    })
                    .collect()
            })
            .unwrap_or_default()
        } else {
            inv_ops_store = s
                .inventory()
                .iter()
                .map(|it| {
                    it.actions
                        .iter()
                        .filter_map(|a| a.as_deref().map(str::to_string))
                        .collect()
                })
                .collect();
            s.inventory()
                .iter()
                .enumerate()
                .map(|(i, it)| ItemRowInput {
                    name: obj_names
                        .and_then(|names| names.name(it.def.id))
                        .or(it.def.name.as_deref()),
                    count: it.count,
                    id: it.def.id,
                    ops: &inv_ops_store[i],
                    noted: it.def.noted,
                    cert: posted_cert(obj_names, &it.def),
                    component_id: -1,
                    slot: it.slot,
                })
                .collect()
        }
    } else {
        inv_ops_store = Vec::new();
        inv.map(|rows| {
            rows.iter()
                .map(|(id, count)| ItemRowInput {
                    name: obj_names.and_then(|names| names.name(*id)),
                    count: *count,
                    id: *id,
                    ops: &[],
                    noted: false,
                    cert: posted_cert_id(obj_names, *id),
                    component_id: -1,
                    slot: -1,
                })
                .collect()
        })
        .unwrap_or_default()
    };
    let stats = snapshot.map(|s| {
        s.stats()
            .iter()
            .map(|st| StatInput {
                index: st.index,
                name: &st.name,
                xp: st.xp,
                base: st.base,
                effective: st.effective,
            })
            .collect::<Vec<_>>()
    });
    let bank_ops_store: Vec<Vec<String>>;
    let bank: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        bank_ops_store = s
            .bank()
            .iter()
            .map(|it| {
                it.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect()
            })
            .collect();
        s.bank()
            .iter()
            .enumerate()
            .map(|(i, it)| ItemRowInput {
                name: obj_names
                    .and_then(|names| names.name(it.def.id))
                    .or(it.def.name.as_deref()),
                count: it.count,
                id: it.def.id,
                ops: &bank_ops_store[i],
                noted: it.def.noted,
                cert: posted_cert(obj_names, &it.def),
                component_id: it.component_id,
                slot: it.slot,
            })
            .collect()
    } else {
        bank_ops_store = Vec::new();
        Vec::new()
    };
    let bank_side_ops_store: Vec<Vec<String>>;
    let bank_side: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        bank_side_ops_store = s
            .bank_side()
            .iter()
            .map(|it| {
                it.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect()
            })
            .collect();
        s.bank_side()
            .iter()
            .enumerate()
            .map(|(i, it)| ItemRowInput {
                name: obj_names
                    .and_then(|names| names.name(it.def.id))
                    .or(it.def.name.as_deref()),
                count: it.count,
                id: it.def.id,
                ops: &bank_side_ops_store[i],
                noted: it.def.noted,
                cert: posted_cert(obj_names, &it.def),
                // Real deposit component (e.g. 2006 / fixture 701). Input.invButton
                // revalidates this id; posting -1 rejects every bank-side deposit.
                component_id: it.component_id,
                slot: it.slot,
            })
            .collect()
    } else {
        bank_side_ops_store = Vec::new();
        Vec::new()
    };
    let npc_action_store: Vec<Vec<String>>;
    let npcs: Vec<SceneEntityInput<'_>> = if let Some(s) = snapshot {
        npc_action_store = s
            .npcs()
            .iter()
            .map(|npc| {
                npc.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect::<Vec<String>>()
            })
            .collect();
        s.npcs()
            .iter()
            .enumerate()
            .map(|(i, npc)| {
                let (reachable, reachable_adj) =
                    entity_reach(npc.tile.x, npc.tile.z, npc.tile.level);
                let (target_kind, target_index) = scene_entity_target(npc.target.as_ref());
                SceneEntityInput {
                    index: npc.index as i32,
                    id: npc.r#type.map(|t| t as i32).unwrap_or(-1),
                    name: npc.name.as_deref(),
                    x: npc.tile.x,
                    z: npc.tile.z,
                    level: npc.tile.level,
                    distance: npc.distance,
                    health: npc.health,
                    max_health: npc.total_health,
                    in_combat: npc.in_combat,
                    animating: npc.moving || npc.animation != -1,
                    actions: &npc_action_store[i],
                    reachable,
                    reachable_adj,
                    combat_level: npc.level,
                    target_kind,
                    target_index,
                    size: npc.size,
                    nx: npc.network.x,
                    nz: npc.network.z,
                    shape: 0,
                    angle: 0,
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    let loc_action_store: Vec<Vec<String>>;
    let locs: Vec<SceneEntityInput<'_>> = if let Some(s) = snapshot {
        loc_action_store = s
            .locs()
            .iter()
            .map(|loc| {
                loc.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect::<Vec<String>>()
            })
            .collect();
        s.locs()
            .iter()
            .enumerate()
            .map(|(i, loc)| {
                let (reachable, reachable_adj) =
                    entity_reach(loc.tile.x, loc.tile.z, loc.tile.level);
                SceneEntityInput {
                    index: loc.id,
                    id: loc.id,
                    name: loc.name.as_deref(),
                    x: loc.tile.x,
                    z: loc.tile.z,
                    level: loc.tile.level,
                    distance: loc.distance,
                    health: -1,
                    max_health: -1,
                    in_combat: false,
                    animating: loc.animation != -1,
                    actions: &loc_action_store[i],
                    reachable,
                    reachable_adj,
                    combat_level: 0,
                    target_kind: 0,
                    target_index: -1,
                    size: 0,
                    nx: 0,
                    nz: 0,
                    shape: loc.shape,
                    angle: loc.angle,
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    let player_action_store: Vec<Vec<String>>;
    let players: Vec<SceneEntityInput<'_>> = if let Some(s) = snapshot {
        player_action_store = s
            .players()
            .iter()
            .map(|player| {
                player
                    .actor
                    .actions
                    .iter()
                    // FlatBuffers cannot carry null entries in a string
                    // vector. Keep each native op's position with the
                    // existing empty-string sentinel; the shim filters it
                    // from the public actions while opIndex still sees the
                    // original one-based slot numbers.
                    .map(|a| a.as_deref().unwrap_or_default().to_string())
                    .collect::<Vec<String>>()
            })
            .collect();
        s.players()
            .iter()
            .enumerate()
            .map(|(i, player)| {
                let (reachable, reachable_adj) = entity_reach(
                    player.actor.tile.x,
                    player.actor.tile.z,
                    player.actor.tile.level,
                );
                let (target_kind, target_index) = scene_entity_target(player.actor.target.as_ref());
                SceneEntityInput {
                    index: player.index as i32,
                    id: player.index as i32,
                    name: player.actor.name.as_deref(),
                    x: player.actor.tile.x,
                    z: player.actor.tile.z,
                    level: player.actor.tile.level,
                    distance: player.actor.distance,
                    health: player.actor.health,
                    max_health: player.actor.total_health,
                    in_combat: player.actor.in_combat,
                    animating: player.actor.moving || player.actor.animation != -1,
                    actions: &player_action_store[i],
                    reachable,
                    reachable_adj,
                    combat_level: player.combat_level,
                    target_kind,
                    target_index,
                    size: 0,
                    nx: 0,
                    nz: 0,
                    shape: 0,
                    angle: 0,
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    let ground_action_store: Vec<Vec<String>>;
    let ground: Vec<SceneEntityInput<'_>> = if let Some(s) = snapshot {
        ground_action_store = s
            .ground_items()
            .iter()
            .map(|item| {
                item.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect::<Vec<String>>()
            })
            .collect();
        s.ground_items()
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let (reachable, reachable_adj) =
                    entity_reach(item.tile.x, item.tile.z, item.tile.level);
                SceneEntityInput {
                    index: item.def.id,
                    id: item.def.id,
                    name: obj_names.and_then(|names| names.name(item.def.id)),
                    x: item.tile.x,
                    z: item.tile.z,
                    level: item.tile.level,
                    distance: item.distance,
                    health: -1,
                    max_health: -1,
                    in_combat: false,
                    animating: false,
                    actions: &ground_action_store[i],
                    reachable,
                    reachable_adj,
                    combat_level: 0,
                    target_kind: 0,
                    target_index: -1,
                    size: 0,
                    nx: 0,
                    nz: 0,
                    shape: 0,
                    angle: 0,
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    let equip_ops_store: Vec<Vec<String>>;
    let equipment: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        equip_ops_store = s
            .equipment()
            .iter()
            .map(|it| {
                it.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect()
            })
            .collect();
        s.equipment()
            .iter()
            .enumerate()
            .map(|(i, it)| ItemRowInput {
                name: obj_names
                    .and_then(|names| names.name(it.def.id))
                    .or(it.def.name.as_deref()),
                count: it.count,
                id: it.def.id,
                ops: &equip_ops_store[i],
                noted: it.def.noted,
                cert: posted_cert(obj_names, &it.def),
                component_id: -1,
                slot: it.slot,
            })
            .collect()
    } else {
        equip_ops_store = Vec::new();
        Vec::new()
    };
    let trade_mine_ops_store: Vec<Vec<String>>;
    let trade_mine: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        trade_mine_ops_store = s
            .trade()
            .my_offer
            .iter()
            .map(|it| {
                it.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect()
            })
            .collect();
        s.trade()
            .my_offer
            .iter()
            .enumerate()
            .map(|(i, it)| ItemRowInput {
                name: obj_names
                    .and_then(|names| names.name(it.def.id))
                    .or(it.def.name.as_deref()),
                count: it.count,
                id: it.def.id,
                ops: &trade_mine_ops_store[i],
                noted: it.def.noted,
                cert: posted_cert(obj_names, &it.def),
                component_id: it.component_id,
                slot: it.slot,
            })
            .collect()
    } else {
        trade_mine_ops_store = Vec::new();
        Vec::new()
    };
    let trade_theirs_ops_store: Vec<Vec<String>>;
    let trade_theirs: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        trade_theirs_ops_store = s
            .trade()
            .their_offer
            .iter()
            .map(|it| {
                it.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect()
            })
            .collect();
        s.trade()
            .their_offer
            .iter()
            .enumerate()
            .map(|(i, it)| ItemRowInput {
                name: obj_names
                    .and_then(|names| names.name(it.def.id))
                    .or(it.def.name.as_deref()),
                count: it.count,
                id: it.def.id,
                ops: &trade_theirs_ops_store[i],
                noted: it.def.noted,
                cert: posted_cert(obj_names, &it.def),
                component_id: it.component_id,
                slot: it.slot,
            })
            .collect()
    } else {
        trade_theirs_ops_store = Vec::new();
        Vec::new()
    };
    let trade_side_ops_store: Vec<Vec<String>>;
    let trade_side: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        trade_side_ops_store = s
            .trade()
            .side_pack
            .iter()
            .map(|it| {
                it.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect()
            })
            .collect();
        s.trade()
            .side_pack
            .iter()
            .enumerate()
            .map(|(i, it)| ItemRowInput {
                name: obj_names
                    .and_then(|names| names.name(it.def.id))
                    .or(it.def.name.as_deref()),
                count: it.count,
                id: it.def.id,
                ops: &trade_side_ops_store[i],
                noted: it.def.noted,
                cert: posted_cert(obj_names, &it.def),
                component_id: it.component_id,
                slot: it.slot,
            })
            .collect()
    } else {
        trade_side_ops_store = Vec::new();
        Vec::new()
    };
    let shop_player_ops_store: Vec<Vec<String>>;
    let shop_player_rows: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        shop_player_ops_store = s
            .shop()
            .player
            .iter()
            .map(|it| {
                it.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect()
            })
            .collect();
        s.shop()
            .player
            .iter()
            .enumerate()
            .map(|(i, it)| ItemRowInput {
                name: obj_names
                    .and_then(|names| names.name(it.def.id))
                    .or(it.def.name.as_deref()),
                count: it.count,
                id: it.def.id,
                ops: &shop_player_ops_store[i],
                noted: it.def.noted,
                cert: posted_cert(obj_names, &it.def),
                component_id: it.component_id,
                slot: it.slot,
            })
            .collect()
    } else {
        shop_player_ops_store = Vec::new();
        Vec::new()
    };
    // `None` = the shop side interface's player pack was not decoded: the
    // Sell path fails closed rather than acting on an empty stand-in.
    let shop_player = snapshot
        .filter(|s| s.shop().player_available)
        .map(|_| shop_player_rows.as_slice());
    let main_make_ops_store: Vec<Vec<String>>;
    let main_make_rows: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        main_make_ops_store = s
            .main_make()
            .iter()
            .map(|it| {
                it.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect()
            })
            .collect();
        s.main_make()
            .iter()
            .enumerate()
            .map(|(i, it)| ItemRowInput {
                name: obj_names
                    .and_then(|names| names.name(it.def.id))
                    .or(it.def.name.as_deref()),
                count: it.count,
                id: it.def.id,
                ops: &main_make_ops_store[i],
                noted: it.def.noted,
                cert: posted_cert(obj_names, &it.def),
                component_id: it.component_id,
                slot: it.slot,
            })
            .collect()
    } else {
        main_make_ops_store = Vec::new();
        Vec::new()
    };
    // Always decoded when a snapshot exists: empty means no Make TYPE_INV
    // on the open main modal, not an unpublished field.
    let main_make = snapshot.map(|_| main_make_rows.as_slice());
    let shop_stock_ops_store: Vec<Vec<String>>;
    let shop_stock: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        shop_stock_ops_store = s
            .shop()
            .stock
            .iter()
            .map(|it| {
                it.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect()
            })
            .collect();
        s.shop()
            .stock
            .iter()
            .enumerate()
            .map(|(i, it)| ItemRowInput {
                name: obj_names
                    .and_then(|names| names.name(it.def.id))
                    .or(it.def.name.as_deref()),
                count: it.count,
                id: it.def.id,
                ops: &shop_stock_ops_store[i],
                noted: it.def.noted,
                cert: posted_cert(obj_names, &it.def),
                component_id: it.component_id,
                slot: it.slot,
            })
            .collect()
    } else {
        shop_stock_ops_store = Vec::new();
        Vec::new()
    };
    let chat_options = snapshot.map(|s| {
        s.chat_options()
            .iter()
            .map(|o| ChatOptionInput { text: &o.text })
            .collect::<Vec<_>>()
    });
    let varps = snapshot.map(|s| {
        let magic = api::snapshot::ReadContext::new(s).varp(108);
        let energy = api::snapshot::ReadContext::new(s).varp(300);
        let armed = api::snapshot::ReadContext::new(s).varp(301);
        // attackstyle_magic is packed as varp 108 on both selected caches.
        // sa_energy/sa_attack are packed as 300/301. Always post them,
        // including 0, so Special.energy/armed and Autocast.armed/selected
        // observe real state instead of a missing-row default.
        let mut rows: Vec<VarpInput> = vec![
            VarpInput {
                index: 108,
                value: magic,
            },
            VarpInput {
                index: 300,
                value: energy,
            },
            VarpInput {
                index: 301,
                value: armed,
            },
        ];
        // Selected prayer overlays 83..=97 must ride the same vector,
        // including 0. Present snapshot rows only — do not invent 0 for
        // an unobserved index.
        let prayer0 = api::prayer::PRAYER_VARP0;
        let prayer_last = prayer0 + api::prayer::PRAYER_COUNT as i32 - 1;
        rows.extend(
            s.varps()
                .iter()
                .filter(|v| (prayer0..=prayer_last).contains(&v.index))
                .map(|v| VarpInput {
                    index: v.index,
                    value: v.value,
                }),
        );
        // Every other nonzero varp, so `reader.varp(i)` reads the client's
        // real value for any index (quest points 101, quest progress). A
        // missing row reads 0, which is the client's own unset value.
        rows.extend(
            s.varps()
                .iter()
                .filter(|v| {
                    v.value != 0
                        && v.index != 108
                        && v.index != 300
                        && v.index != 301
                        && !(prayer0..=prayer_last).contains(&v.index)
                })
                .map(|v| VarpInput {
                    index: v.index,
                    value: v.value,
                }),
        );
        rows
    });
    let combat_style_store = snapshot.map(|s| {
        let root = s
            .side_tabs()
            .iter()
            .find(|t| t.index == 0)
            .map(|t| t.root_component_id)
            .unwrap_or(-1);
        if root == -1 {
            Vec::new()
        } else {
            api::query::widget_search::combat_style_labels(s, root, 43)
        }
    });
    let combat_styles: Vec<CombatStyleInput<'_>> = combat_style_store
        .as_ref()
        .map(|labels| {
            labels
                .iter()
                .map(|l| CombatStyleInput {
                    mode: l.mode,
                    label: &l.label,
                    component_id: l.component_id,
                })
                .collect()
        })
        .unwrap_or_default();
    let local = snapshot.and_then(|s| s.local_player());
    let my_name = local.and_then(|lp| lp.player.actor.name.as_deref());
    let in_combat = local.is_some_and(|lp| lp.player.actor.in_combat);
    let attacked_by_player =
        local.is_some_and(|lp| api::snapshot::attacked_by_player(lp.player.actor.face_entity));
    let (self_target_kind, self_target_index) =
        scene_entity_target(local.and_then(|lp| lp.player.actor.target.as_ref()));
    let animating =
        local.is_some_and(|lp| lp.player.actor.moving || lp.player.actor.animation != -1);
    let modals = snapshot.map(|s| s.modals());
    // The scene bank booths: the openable locs (`Use-quickly` is the
    // bankbooth op the pack bakes from `scripts/interface_bank/configs/
    // bank_booth.loc`). Only the tile is posted — the shim never reads a
    // loc definition.
    let booths = snapshot.map(|s| {
        s.locs()
            .iter()
            .filter(|l| {
                l.actions.iter().any(|a| {
                    a.as_deref()
                        .is_some_and(|a| a.eq_ignore_ascii_case("Use-quickly"))
                })
            })
            .map(|l| TileInput {
                x: l.tile.x,
                z: l.tile.z,
                level: l.tile.level,
            })
            .collect::<Vec<_>>()
    });
    let nearest_booth = snapshot.and_then(|s| {
        s.nearest_use_quickly_booth().map(|loc| NearestBoothInput {
            x: loc.tile.x,
            z: loc.tile.z,
            level: loc.tile.level,
            id: loc.id,
            name: loc.name.as_deref().unwrap_or("Bank booth"),
            op: "Use-quickly",
        })
    });
    let banks = world.map(|w| {
        use nav::pack::BankAccess;
        w.banks()
            .iter()
            .map(|b| {
                let (kind, op, choose) = match &b.access {
                    BankAccess::Booth { op } => ("booth", *op, None),
                    BankAccess::Npc { op, choose, .. } => ("npc", *op, choose.as_deref()),
                };
                BankStandInput {
                    name: &b.name,
                    x: b.tile.x,
                    z: b.tile.z,
                    level: b.tile.level,
                    kind,
                    op,
                    choose,
                }
            })
            .collect::<Vec<_>>()
    });
    let mut make_button_store: Vec<Vec<MakeButtonInput>> = Vec::new();
    let make_products: Vec<MakeProductInput<'_>> = snapshot
        .map(|s| {
            make_button_store = s
                .make_products()
                .iter()
                .map(|p| {
                    p.buttons
                        .iter()
                        .map(|b| MakeButtonInput {
                            qty: b.quantity,
                            com_id: b.component_id,
                        })
                        .collect()
                })
                .collect();
            s.make_products()
                .iter()
                .enumerate()
                .map(|(i, p)| MakeProductInput {
                    object_id: p.object_id,
                    name: p.name.as_str(),
                    buttons: &make_button_store[i],
                })
                .collect()
        })
        .unwrap_or_default();
    let side_tab_ifaces: Vec<SideTabIfaceInput> = snapshot
        .map(|s| {
            s.side_tabs()
                .iter()
                .map(|t| SideTabIfaceInput {
                    index: t.index,
                    id: t.root_component_id,
                })
                .collect()
        })
        .unwrap_or_default();
    let spell_store = snapshot.map(|s| {
        let root = s
            .side_tabs()
            .iter()
            .find(|t| t.index == 6)
            .map(|t| t.root_component_id)
            .unwrap_or(-1);
        if root == -1 {
            Vec::new()
        } else {
            s.widgets()
                .iter()
                .chain(s.side_tabs().iter().flat_map(|t| t.widgets.iter()))
                .filter(|w| {
                    w.root_component_id == root
                        && w.button_type == 2
                        && w.target_base.as_deref().is_some_and(|t| !t.is_empty())
                })
                .map(|w| (w.target_base.clone().unwrap_or_default(), w.component_id))
                .collect::<Vec<_>>()
        }
    });
    let spell_buttons: Vec<CombatStyleInput<'_>> = spell_store
        .as_ref()
        .map(|rows| {
            rows.iter()
                .map(|(label, component_id)| CombatStyleInput {
                    mode: 0,
                    label,
                    component_id: *component_id,
                })
                .collect()
        })
        .unwrap_or_default();
    let chat_lines: Vec<ChatLineInput<'_>> = snapshot
        .map(|s| {
            s.chat_lines()
                .iter()
                .map(|l| ChatLineInput {
                    seq: l.sequence,
                    text: l.text.as_str(),
                    type_: l.type_,
                    username: l.username.as_deref(),
                })
                .collect()
        })
        .unwrap_or_default();
    let widget_store: Vec<(i32, String)> = snapshot
        .map(|s| {
            s.widgets()
                .iter()
                .filter_map(|w| w.text.as_ref().map(|text| (w.component_id, text.clone())))
                .collect()
        })
        .unwrap_or_default();
    let widgets: Vec<WidgetTextInput<'_>> = widget_store
        .iter()
        .map(|(component_id, text)| WidgetTextInput {
            component_id: *component_id,
            text,
        })
        .collect();
    // The open puzzle board, posted next to the widget-text map: the
    // identified component, its observed slot count, its rows mapped from
    // that widget's own `ItemView`s (bounded — one row per stored slot) and
    // the session generation. `None` only when there is no snapshot at all
    // (not supplied); a closed board is `Some` with component -1 and no
    // rows, so a close can never be mistaken for a delta keep.
    let puzzle_board_ops_store: Vec<Vec<String>>;
    let puzzle_board_items_store: Vec<ItemRowInput<'_>>;
    let puzzle_board = if let Some(s) = snapshot {
        let board = s.puzzle_board();
        puzzle_board_ops_store = board
            .items
            .iter()
            .map(|it| {
                it.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect()
            })
            .collect();
        puzzle_board_items_store = board
            .items
            .iter()
            .enumerate()
            .map(|(i, it)| ItemRowInput {
                name: obj_names
                    .and_then(|names| names.name(it.def.id))
                    .or(it.def.name.as_deref()),
                count: it.count,
                id: it.def.id,
                ops: &puzzle_board_ops_store[i],
                noted: it.def.noted,
                cert: posted_cert(obj_names, &it.def),
                component_id: it.component_id,
                slot: it.slot,
            })
            .collect();
        Some(PuzzleBoardInput {
            component_id: board.component_id,
            size: board.size,
            items: &puzzle_board_items_store,
            generation: board.generation,
        })
    } else {
        puzzle_board_ops_store = Vec::new();
        puzzle_board_items_store = Vec::new();
        None
    };
    let quest_statuses: Vec<QuestStatusInput<'_>> = snapshot
        .map(|s| {
            s.quest_statuses()
                .iter()
                .map(|quest| QuestStatusInput {
                    name: quest.name.as_str(),
                    status: quest.status().as_str(),
                    // The walked TYPE_TEXT id: the row's click target. The
                    // host walk always has one on a posted row.
                    component_id: Some(quest.component_id),
                })
                .collect()
        })
        .unwrap_or_default();
    let quest_statuses = snapshot
        .filter(|s| s.quest_statuses_available())
        .map(|_| quest_statuses.as_slice());
    // The main modal's paired TYPE_TEXT walk, copied from the same rebuild
    // that set `modals.main`: this root is the integer `main_modal_id`
    // already posts, and the lines are that root's walk. `None` (no
    // snapshot) is "not supplied" — not a closed modal. A closed modal is
    // `Some` with root -1 and no lines.
    let main_modal_texts = modals.map(|m| MainModalTextsInput {
        root: m.main,
        texts: snapshot.map(|s| s.main_modal_texts()).unwrap_or(&[]),
    });
    let input = SnapshotInput {
        tick,
        here,
        ingame,
        inv: &inv,
        // The inv tab's slot count (28 when bound, 0 while the side icons
        // stay tutorial-locked): the `reader.inventorySize()` gate a
        // script's onStart parks on.
        inv_size: snapshot.map_or(0, |s| s.inventory_size()),
        stats: stats.as_deref().unwrap_or(&[]),
        booths: booths.as_deref().unwrap_or(&[]),
        nearest_booth,
        banks: banks.as_deref().unwrap_or(&[]),
        bank: &bank,
        bank_side: &bank_side,
        bank_open: snapshot.is_some_and(|s| s.bank_component_id() != -1),
        bank_loaded: snapshot.is_some_and(GameSnapshot::bank_loaded),
        bank_generation: snapshot.map_or(0, GameSnapshot::bank_session_generation),
        count_dialog_open: snapshot.is_some_and(GameSnapshot::count_dialog_open),
        withdraw_x_result_seq,
        withdraw_x_result,
        withdraw_load_result_seq,
        withdraw_load_result,
        bank_op_result_seq,
        bank_op_result,
        hold,
        ours,
        npcs: &npcs,
        locs: &locs,
        players: &players,
        ground: &ground,
        equipment: &equipment,
        chat_open: modals.is_some_and(|m| m.chat != -1),
        chat_continue: snapshot.is_some_and(|s| s.chat_continue_component_id() != -1),
        chat_text: snapshot.and_then(|s| s.chat()),
        chat_options: chat_options.as_deref().unwrap_or(&[]),
        side_tab: snapshot.map(|s| s.active_side_tab()).unwrap_or(-1),
        varps: varps.as_deref().unwrap_or(&[]),
        combat_styles: &combat_styles,
        run_energy: snapshot.map(|s| s.runenergy()).unwrap_or(0),
        run_enabled: snapshot.is_some_and(|s| api::snapshot::ReadContext::new(s).varp(173) != 0),
        retaliate_enabled: snapshot
            .is_some_and(|s| api::snapshot::ReadContext::new(s).varp(172) == 0),
        my_name,
        in_combat,
        animating,
        main_modal_id: modals.map(|m| m.main).unwrap_or(-1),
        chat_modal_id: modals.map(|m| m.chat).unwrap_or(-1),
        make_products: &make_products,
        side_tab_ifaces: &side_tab_ifaces,
        spell_buttons: &spell_buttons,
        chat_lines: &chat_lines,
        bank_note_on: snapshot
            .and_then(|s| s.bank_note_controls())
            .map(|c| c.on_component_id)
            .unwrap_or(-1),
        bank_note_off: snapshot
            .and_then(|s| s.bank_note_controls())
            .map(|c| c.off_component_id)
            .unwrap_or(-1),
        scene_state: snapshot.map(|s| s.scene_state()).unwrap_or(0),
        weight: snapshot
            .and_then(|s| s.local_player())
            .map(|lp| lp.weight)
            .unwrap_or(0),
        combat_level: snapshot
            .and_then(|s| s.local_player())
            .map(|lp| lp.player.combat_level)
            .unwrap_or(0),
        camera_yaw: snapshot.map(|s| s.camera().orbit_yaw).unwrap_or(0),
        camera_pitch: snapshot.map(|s| s.camera().orbit_pitch).unwrap_or(0),
        teleports_enabled,
        self_slot: snapshot.map(|s| s.self_slot()).unwrap_or(-1),
        trade_offer_open: snapshot.is_some_and(|s| s.trade().offer_open),
        trade_confirm_open: snapshot.is_some_and(|s| s.trade().confirm_open),
        trade_partner: snapshot.and_then(|s| s.trade().partner.as_deref()),
        trade_mine: &trade_mine,
        trade_theirs: &trade_theirs,
        trade_side: &trade_side,
        trade_accept_id: snapshot
            .map(|s| s.trade().accept_component_id)
            .unwrap_or(-1),
        trade_decline_id: snapshot
            .map(|s| s.trade().decline_component_id)
            .unwrap_or(-1),
        shop_open: snapshot.is_some_and(|s| s.shop().open),
        shop_stock: &shop_stock,
        reach,
        attacked_by_player,
        self_target_kind,
        self_target_index,
        widgets: &widgets,
    };
    let bank_approach_store: Vec<BankApproachInput> = match (snapshot, here, flood.as_ref()) {
        (Some(s), Some(tile), Some(flood)) => s
            .locs()
            .iter()
            .filter_map(|loc| {
                let name = loc.name.as_deref()?;
                if !name
                    .as_bytes()
                    .windows(4)
                    .any(|word| word.eq_ignore_ascii_case(b"bank"))
                {
                    return None;
                }
                let approach = api::query::loc_approach::booth_approach(
                    loc,
                    s.scene(),
                    WorldTile {
                        x: tile.x,
                        z: tile.z,
                        level: tile.level,
                    },
                    flood,
                )?;
                Some(BankApproachInput {
                    loc_id: loc.id,
                    x: loc.tile.x,
                    z: loc.tile.z,
                    level: loc.tile.level,
                    can_operate: approach.can_operate,
                    dest_ok: approach.dest.is_some(),
                    dest_x: approach.dest.map(|t| t.x).unwrap_or(0),
                    dest_z: approach.dest.map(|t| t.z).unwrap_or(0),
                    dest_level: approach.dest.map(|t| t.level).unwrap_or(0),
                })
            })
            .collect(),
        _ => Vec::new(),
    };
    let latest_hops = inspect
        .latest
        .as_ref()
        .map(|t| route_inspect::hop_inputs(&t.hops))
        .unwrap_or_default();
    let prev_hops = inspect
        .prev
        .as_ref()
        .map(|t| route_inspect::hop_inputs(&t.hops))
        .unwrap_or_default();
    let latest_in = inspect
        .latest
        .as_ref()
        .map(|t| route_inspect::terminal_to_input(t, &latest_hops))
        .unwrap_or_default();
    let prev_in = inspect
        .prev
        .as_ref()
        .map(|t| route_inspect::terminal_to_input(t, &prev_hops))
        .unwrap_or_default();
    let collision = Some(match snapshot {
        Some(s) if s.ingame() && s.scene_state() == 2 && s.scene().available => {
            let sc = s.scene();
            CollisionViewInput {
                available: true,
                base_x: sc.base_x,
                base_z: sc.base_z,
                level: sc.level,
                width: sc.width,
                height: sc.height,
                flags: &sc.collision_flags,
            }
        }
        _ => CollisionViewInput::UNAVAILABLE,
    });
    let native = NativeFactsInput {
        self_chat: snapshot.and_then(GameSnapshot::local_overhead_text),
        hint_tile: snapshot
            .and_then(GameSnapshot::hint_tile)
            .map(|tile| (tile.x, tile.z)),
        retaliate_controls: snapshot
            .and_then(GameSnapshot::retaliate_controls)
            .map(|controls| (controls.on_component_id, controls.off_component_id)),
        quest_statuses,
        main_modal_texts,
        puzzle_board,
        npc_boxes,
        shop_player,
        main_make,
        bank_approaches: Some(&bank_approach_store),
        walk_outcome_seq: walk_outcome.seq,
        walk_outcome_generation: walk_outcome.generation,
        walk_outcome_request_id: walk_outcome.request_id,
        walk_outcome_failed: walk_outcome.failed,
        walk_outcome_x: walk_outcome.x,
        walk_outcome_z: walk_outcome.z,
        walk_outcome_level: walk_outcome.level,
        walk_outcome_radius: walk_outcome.radius,
        walk_outcome_allow_teleports: walk_outcome.allow_teleports,
        walk_missing_carry,
        route_inspect: script::isolate_fb::RouteInspectFactsInput {
            latest: latest_in,
            prev: prev_in,
            running_id: inspect.running_id,
            pending_id: inspect.pending_id,
            accepted_id: inspect.accepted_id,
            replaced_id: inspect.replaced_id,
            replaced_prev_id: inspect.replaced_prev_id,
            refused_id: inspect.refused_id,
            refused_id_2: inspect.refused_id_2,
            refused_id_3: inspect.refused_id_3,
            unobserved: inspect.unobserved,
        },
        collision,
        bank_selection: Default::default(),
        self_anim: Some(local.map_or(-1, |lp| lp.player.actor.animation)),
    };
    f(&input, native)
}
