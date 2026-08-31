// Task 8: snapshot families rebuild only the family whose gen moved.
// `Client::new` with a cache-less `/tmp` dir falls back to `Cache::default()`
// and never touches the network (same trick as `client/tests/gens.rs`).

use api::query::{npc_by_index, npcs_at};
use api::snapshot::{
    ActorKind, ActorTargetView, Family, GameSnapshot, ItemActionFamily, ItemContainer, LocLayer,
    ReadContext, VarpView, WidgetKind, WidgetRoot, WidgetVarpBindingView, WorldTile,
};
use client::client::{Client, ClientConfig, ClientNpc};
use client::config::if_type::{ButtonType, ComponentType, IfType, IfTypeMut};
use client::config::{LocType, NpcType, ObjType};
use client::dash3d::{ClientObj, ClientPlayer, CollisionFlag};
use client::datastruct::LinkList;
use client::io::ServerProt;
use std::sync::Arc;

fn cfg() -> ClientConfig {
    ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp".into(),
        members: true,
        lowmem: false,
    }
}

/// A client with one live NPC in slot 7 standing on (100, 200).
fn client_with_npc() -> Client {
    let mut c = Client::new(cfg());
    let mut npc = ClientNpc::default();
    npc.entity.x = 100;
    npc.entity.z = 200;
    npc.entity.yaw = 3;
    npc.entity.health = 7;
    npc.entity.total_health = 7;
    npc.r#type = Some(9);
    c.npc[7] = Some(Box::new(npc));
    c.npc_ids[0] = 7;
    c.npc_count = 1;
    c
}

/// An unchanged npc gen means `rebuild_family` copies nothing: the second
/// call returns the same slice pointer and the same views.
#[test]
fn unchanged_npc_gen_reuses_last_rebuild() {
    let mut c = client_with_npc();
    let mut snap = GameSnapshot::new();
    assert!(!snap.rebuild_family(&mut c, Family::Npc));

    let ptr = snap.npcs().as_ptr();
    assert!(!snap.rebuild_family(&mut c, Family::Npc));
    assert_eq!(snap.npcs().as_ptr(), ptr);
    assert!(snap.npcs().is_empty());
}

/// After `bump_gens(NPC_INFO)` the npc list is copied once, keyed by slot
/// index; a second rebuild with the same gen does not reallocate.
#[test]
fn npc_bump_copies_list_once_then_idempotent() {
    let mut c = client_with_npc();
    let mut snap = GameSnapshot::new();

    c.bump_gens(ServerProt::NPC_INFO);
    assert!(snap.rebuild_family(&mut c, Family::Npc));
    assert_eq!(snap.npcs().len(), 1);
    assert_eq!(snap.npcs()[0].index, 7);
    assert_eq!(snap.npcs()[0].x, 100);

    let ptr = snap.npcs().as_ptr();
    assert!(!snap.rebuild_family(&mut c, Family::Npc));
    assert_eq!(snap.npcs().as_ptr(), ptr);
    assert_eq!(snap.npcs().len(), 1);
}

/// Walking one npc (mutating `client.npc[i].x`) without a bump must not
/// require cloning the whole list for readers holding the previous
/// snapshot: identity (slot index) is preserved and data is the owned copy.
#[test]
fn walk_in_place_does_not_clone_for_previous_readers() {
    let mut c = client_with_npc();
    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::NPC_INFO);
    snap.rebuild_family(&mut c, Family::Npc);

    let old_ptr = snap.npcs().as_ptr();
    c.npc[7].as_mut().unwrap().entity.x = 999;
    c.npc[7].as_mut().unwrap().entity.yaw = 42;

    assert!(!snap.rebuild_family(&mut c, Family::Npc));
    assert_eq!(snap.npcs().as_ptr(), old_ptr);
    assert_eq!(snap.npcs()[0].index, 7);
    assert_eq!(snap.npcs()[0].x, 100);
    assert_eq!(snap.npcs()[0].yaw, 3);
}

/// A later npc gen bump copies the fresh values.
#[test]
fn second_bump_refreshes_copied_data() {
    let mut c = client_with_npc();
    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::NPC_INFO);
    snap.rebuild_family(&mut c, Family::Npc);

    c.bump_gens(ServerProt::NPC_INFO);
    c.npc[7].as_mut().unwrap().entity.x = 555;
    assert!(snap.rebuild_family(&mut c, Family::Npc));
    assert_eq!(snap.npcs()[0].x, 555);
}

/// An inv bump must not rebuild the npc family.
#[test]
fn other_family_bump_leaves_npc_rebuild_untouched() {
    let mut c = client_with_npc();
    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::NPC_INFO);
    snap.rebuild_family(&mut c, Family::Npc);

    let ptr = snap.npcs().as_ptr();
    c.bump_gens(ServerProt::UPDATE_INV_FULL);
    assert!(!snap.rebuild_family(&mut c, Family::Npc));
    assert_eq!(snap.npcs().as_ptr(), ptr);
    assert_eq!(snap.npcs().len(), 1);
}

/// `GameSnapshot` records the world generations it has been rebuilt up to.
#[test]
fn snapshot_tracks_family_generations() {
    let mut c = client_with_npc();
    let mut snap = GameSnapshot::new();

    assert!(!snap.rebuild_family(&mut c, Family::Inv));
    assert_eq!(snap.gens().inv, 0);

    c.bump_gens(ServerProt::VARP_SMALL);
    assert!(snap.rebuild_family(&mut c, Family::Varp));
    assert_eq!(snap.gens().varp, 1);
    assert_eq!(snap.gens().npc, 0);
}

/// An npc_ids index past `Client.npc` is skipped (`.get()`, no panic).
#[test]
fn npc_rebuild_skips_out_of_range_index() {
    let mut c = client_with_npc();
    c.npc_ids[0] = 99_999;
    c.npc_count = 1;
    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::NPC_INFO);
    snap.rebuild_family(&mut c, Family::Npc);
    assert!(snap.npcs().is_empty());
}

/// Stat rebuild copies run energy for auto-run's snapshot view.
#[test]
fn stat_rebuild_copies_runenergy() {
    let mut c = client_with_npc();
    c.runenergy = 20;
    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::UPDATE_RUNENERGY);
    assert!(snap.rebuild_family(&mut c, Family::Stat));
    assert_eq!(snap.runenergy(), 20);
    assert!(!snap.rebuild_family(&mut c, Family::Stat));
}

/// Queries borrow one family without allocating: by slot index or by tile.
#[test]
fn queries_borrow_by_index_and_tile() {
    let mut c = client_with_npc();
    c.npc[3] = Some(Box::new(ClientNpc::default()));
    c.npc_ids[1] = 3;
    c.npc_count = 2;
    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::NPC_INFO);
    snap.rebuild_family(&mut c, Family::Npc);

    let view = npc_by_index(snap.npcs(), 7).expect("slot 7 live");
    assert_eq!(view.x, 100);
    assert_eq!(view.r#type, Some(9));
    assert_eq!(npc_by_index(snap.npcs(), 3).unwrap().index, 3);
    assert!(npc_by_index(snap.npcs(), 999).is_none());

    assert_eq!(npcs_at(snap.npcs(), 100, 200).count(), 1);
    assert_eq!(npcs_at(snap.npcs(), 0, 0).count(), 1);
}

/// Player-family rebuild: the world build origin and the local player's
/// world tile (base + route head). No player decode yet → tile `None`.
#[test]
fn player_rebuild_records_base_and_tile() {
    let mut c = client_with_npc();
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::PLAYER_INFO);
    assert!(snap.rebuild_family(&mut c, Family::Player));
    assert_eq!(snap.base(), Some((3200, 3200)));
    assert_eq!(snap.tile(), None, "no local player decoded yet");

    c.local_player = Some(ClientPlayer::at(20, 12));
    c.bump_gens(ServerProt::PLAYER_INFO);
    assert!(snap.rebuild_family(&mut c, Family::Player));
    assert_eq!(snap.tile(), Some((3220, 3212, 0)));

    assert!(!snap.rebuild_family(&mut c, Family::Player));
}

/// Inv-family rebuild: zip the TYPE_INV iface's obj ids/counts. The iface
/// stores `obj_id + 1` (0 = empty), so the view carries the **real** obj
/// ids, matching `ItemView.def.id` and the `ObjNames`-resolved ids the
/// evidence / `Proof::Item` consumers compare against.
#[test]
fn inv_rebuild_reads_the_type_inv_iface() {
    let mut c = client_with_npc();
    match c.iface_id(|f| f.r#type == ComponentType::TYPE_INV) {
        Some(id) => {
            let inv = c.iface_mut(id).unwrap();
            inv.link_obj_type = Some(vec![526, 995]); // stored = obj id + 1
            inv.link_obj_number = Some(vec![1, 100]);
        }
        None => {
            let id = c.push_iface(IfType {
                r#type: ComponentType::TYPE_INV,
                ..Default::default()
            });
            c.set_iface_mut(
                id,
                IfTypeMut {
                    link_obj_type: Some(vec![526, 995]),
                    link_obj_number: Some(vec![1, 100]),
                    ..Default::default()
                },
            );
        }
    }
    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::UPDATE_INV_FULL);
    assert!(snap.rebuild_family(&mut c, Family::Inv));
    assert_eq!(snap.inv(), &[(525, 1), (994, 100)]);
    assert_eq!(snap.inv_count(525), 1);
    assert_eq!(snap.inv_count(994), 100);
    assert_eq!(snap.inv_count(526), 0, "the stored value is obj id + 1");
    assert_eq!(snap.inv_count(0), 0);
    assert!(!snap.rebuild_family(&mut c, Family::Inv));
}

/// The legacy `inv()` family and the iface-derived `inventory()` family
/// agree on obj ids: both decode the stored `obj_id + 1` convention to
/// real ids.
#[test]
fn inv_ids_match_inventory_def_ids() {
    let mut c = client_with_npc();
    plant_obj(&mut c, 3, "Coins");
    plant_obj(&mut c, 4, "Sword");
    // `rebuild_inv` reads the first TYPE_INV in the table; point the inv
    // tab (side tab 3) at the same component so both families agree.
    let inv_id = c
        .iface_id(|f| f.r#type == ComponentType::TYPE_INV)
        .unwrap_or_else(|| c.push_iface(IfType::default()));
    set_iface(
        &mut c,
        inv_id,
        IfType {
            id: inv_id as i32,
            r#type: ComponentType::TYPE_INV,
            obj_ops: true,
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        inv_id,
        IfTypeMut {
            link_obj_type: Some(vec![4, 5, 0]),
            link_obj_number: Some(vec![1, 100, 0]),
            ..Default::default()
        },
    );
    c.side_icon[3] = inv_id as i32;

    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::UPDATE_INV_FULL);
    assert!(snap.rebuild_family(&mut c, Family::Inv));
    assert!(snap.rebuild_family(&mut c, Family::Inventory));

    assert_eq!(snap.inv(), &[(3, 1), (4, 100)]);
    let inv_ids: Vec<i32> = snap.inv().iter().map(|(id, _)| *id).collect();
    let def_ids: Vec<i32> = snap.inventory().iter().map(|v| v.def.id).collect();
    assert_eq!(
        inv_ids, def_ids,
        "inv() ids and inventory() def.id(s) must both be real obj ids"
    );
}

/// The inv family reads the side-tab-3 container, not the first TYPE_INV
/// in the table (regression: a live client's earlier unrelated TYPE_INV
/// — a shop/trade modal — decodes empty, so `WorldState` gating failed
/// the cart fare closed while the coins sat in the real inv container).
#[test]
fn inv_rebuild_prefers_the_side_tab_container_over_an_earlier_type_inv() {
    let mut c = client_with_npc();
    // An earlier, unrelated TYPE_INV that never received an update.
    let decoy = c.push_iface(IfType {
        r#type: ComponentType::TYPE_INV,
        ..Default::default()
    });
    c.set_iface_mut(
        decoy,
        IfTypeMut {
            link_obj_type: Some(vec![0, 0, 0]),
            link_obj_number: Some(vec![0, 0, 0]),
            ..Default::default()
        },
    );
    // The real inv container, under the side-tab-3 root with obj_ops.
    let inv_id = c.push_iface(IfType {
        id: 0,
        r#type: ComponentType::TYPE_INV,
        obj_ops: true,
        ..Default::default()
    });
    c.set_iface_mut(
        inv_id,
        IfTypeMut {
            link_obj_type: Some(vec![526, 995]),
            link_obj_number: Some(vec![1, 100]),
            ..Default::default()
        },
    );
    c.side_icon[3] = inv_id as i32;

    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::UPDATE_INV_FULL);
    assert!(snap.rebuild_family(&mut c, Family::Inv));
    assert_eq!(
        snap.inv(),
        &[(525, 1), (994, 100)],
        "the side-tab container's slots, never the decoy's empty ones"
    );
}

/// Chat-family rebuild: the ring head (`chat_text[0]`) is the latest line.
#[test]
fn chat_rebuild_reads_the_ring_head() {
    let mut c = client_with_npc();
    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::MESSAGE_GAME);
    assert!(snap.rebuild_family(&mut c, Family::Chat));
    assert_eq!(snap.chat(), None, "empty ring head reads as none");

    c.chat_text[0] = "Welcome to RuneScape".into();
    c.bump_gens(ServerProt::MESSAGE_GAME);
    assert!(snap.rebuild_family(&mut c, Family::Chat));
    assert_eq!(snap.chat(), Some("Welcome to RuneScape"));
}

/// Scene-family rebuild: `ingame` + `scene_state`.
#[test]
fn scene_rebuild_records_ingame_and_scene_state() {
    let mut c = client_with_npc();
    c.ingame = true;
    c.scene_state = 2;
    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    assert!(snap.rebuild_family(&mut c, Family::Scene));
    assert!(snap.ingame());
    assert_eq!(snap.scene_state(), 2);
    assert!(!snap.rebuild_family(&mut c, Family::Scene));
}

/// The position of each `Family` variant in the enum. An exhaustive
/// match keeps this compile-checked against `ClientGens`' counters.
fn family_index(f: Family) -> usize {
    match f {
        Family::Npc => 1,
        Family::Player => 2,
        Family::Inv => 3,
        Family::Varp => 4,
        Family::Stat => 5,
        Family::Chat => 6,
        Family::Scene => 7,
        Family::Iface => 8,
        Family::Camera => 9,
        Family::MapFlag => 10,
        Family::World => 11,
        Family::Loc => 12,
        Family::GroundItem => 13,
        Family::Inventory => 14,
        Family::Equipment => 15,
        Family::Bank => 16,
        Family::BankSide => 17,
        Family::Trade => 18,
        Family::Widgets => 19,
        Family::SideTabs => 20,
        Family::ChatOptions => 21,
        Family::MakeProducts => 22,
        Family::QuestStatuses => 23,
        Family::Modals => 24,
        Family::Controls => 25,
        Family::Menu => 26,
    }
}

/// `Family` covers all 11 `ClientGens` counters, plus the two scene-derived
/// view families (loc/ground-item changes bump the scene gen, so both track
/// it with their own counter).
#[test]
fn family_enum_covers_all_client_gens() {
    assert_eq!(
        family_index(Family::World),
        11,
        "Family must mirror ClientGens' 11 counters"
    );

    let mut c = client_with_npc();
    let mut snap = GameSnapshot::new();

    assert!(!snap.rebuild_family(&mut c, Family::Iface));
    assert!(!snap.rebuild_family(&mut c, Family::Camera));
    assert!(!snap.rebuild_family(&mut c, Family::MapFlag));
    assert!(!snap.rebuild_family(&mut c, Family::World));
    assert!(!snap.rebuild_family(&mut c, Family::Loc));
    assert!(!snap.rebuild_family(&mut c, Family::GroundItem));
    assert_eq!(snap.gens().iface, 0);
    assert_eq!(snap.gens().camera, 0);
    assert_eq!(snap.gens().map_flag, 0);
    assert_eq!(snap.gens().world, 0);

    c.bump_gens(ServerProt::IF_SETPOSITION);
    c.bump_gens(ServerProt::CAM_LOOKAT);
    c.bump_gens(ServerProt::UNSET_MAP_FLAG);
    c.bump_gens(ServerProt::SET_MULTIWAY);

    assert!(snap.rebuild_family(&mut c, Family::Iface));
    assert!(snap.rebuild_family(&mut c, Family::Camera));
    assert!(snap.rebuild_family(&mut c, Family::MapFlag));
    assert!(snap.rebuild_family(&mut c, Family::World));
    assert_eq!(snap.gens().iface, 1);
    assert_eq!(snap.gens().camera, 1);
    assert_eq!(snap.gens().map_flag, 1);
    assert_eq!(snap.gens().world, 1);

    assert!(!snap.rebuild_family(&mut c, Family::Iface));
    assert!(!snap.rebuild_family(&mut c, Family::Camera));
    assert!(!snap.rebuild_family(&mut c, Family::MapFlag));
    assert!(!snap.rebuild_family(&mut c, Family::World));

    // Loc and ground-item changes arrive on scene-family packets.
    assert!(!snap.rebuild_family(&mut c, Family::Loc));
    assert!(!snap.rebuild_family(&mut c, Family::GroundItem));
    c.bump_gens(ServerProt::OBJ_ADD);
    assert!(snap.rebuild_family(&mut c, Family::Loc));
    assert!(snap.rebuild_family(&mut c, Family::GroundItem));
    assert!(!snap.rebuild_family(&mut c, Family::Loc));
    assert!(!snap.rebuild_family(&mut c, Family::GroundItem));
}

/// `GameSnapshot::rebuild` (the harness read) rebuilds every family and
/// reports whether any gen moved.
#[test]
fn rebuild_all_families_reports_dirty_once() {
    let mut c = client_with_npc();
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c.local_player = Some(ClientPlayer::at(20, 12));
    c.runenergy = 20;
    c.bump_gens(ServerProt::PLAYER_INFO);
    c.bump_gens(ServerProt::UPDATE_RUNENERGY);
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    let mut snap = GameSnapshot::new();
    assert!(snap.rebuild(&mut c));
    assert_eq!(snap.tile(), Some((3220, 3212, 0)));
    assert_eq!(snap.runenergy(), 20);
    assert!(!snap.rebuild(&mut c), "unchanged gens are not dirty again");
}

/// `check_scene` flips `scene_state = 2` on the SIM loop with no scene
/// gen bump; the snapshot must still see it (a gen-gated copy would pin
/// the harness in a stale "loading" state forever).
#[test]
fn scene_status_is_always_fresh_without_a_gen_bump() {
    let mut c = client_with_npc();
    c.ingame = true;
    c.scene_state = 1;
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&mut c);
    assert_eq!(snap.scene_state(), 1);

    // The scene completes with no packet behind it: no gen moves, but
    // the snapshot must read the live state.
    c.scene_state = 2;
    assert!(!snap.rebuild(&mut c), "no gen moved");
    assert_eq!(snap.scene_state(), 2, "scene status is always fresh");
    assert!(snap.ingame());
}

/// The tile key types are plain data: Copy, Eq and Hash so later
/// families can key per-tile caches (locs, ground items) on them.
#[test]
fn tile_types_are_plain_copy_data() {
    let t = api::snapshot::WorldTile {
        x: 3220,
        z: 3212,
        level: 1,
    };
    let t2 = t; // Copy
    assert_eq!(t, t2);
    let mut tiles = std::collections::HashSet::new();
    tiles.insert(t);
    assert!(tiles.contains(&t2));

    let l = api::snapshot::LocalTile { lx: 20, lz: 12 };
    let l2 = l;
    assert_eq!(l, l2);
    let mut locs = std::collections::HashSet::new();
    locs.insert(l);
    assert!(locs.contains(&l2));
}

/// Stat rebuild reads the full skill table into `StatView`s (all 25
/// skills) and keeps `runenergy()` as a derived convenience.
#[test]
fn stat_view_rebuild_reads_full_skills() {
    let mut c = client_with_npc();
    c.stat_effective_level[0] = 12;
    c.stat_base_level[0] = 12;
    c.stat_xp[0] = 1300;
    c.stat_effective_level[2] = 30;
    c.stat_base_level[2] = 30;
    c.stat_xp[2] = 13150;
    c.runenergy = 20;
    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::UPDATE_STAT);
    assert!(snap.rebuild_family(&mut c, Family::Stat));

    let stats = snap.stats();
    assert_eq!(stats.len(), 25);
    assert_eq!(stats[0].index, 0);
    assert_eq!(stats[0].name, "attack");
    assert_eq!(stats[0].effective, 12);
    assert_eq!(stats[0].base, 12);
    assert_eq!(stats[0].xp, 1300);
    assert!(stats[0].used);
    assert_eq!(stats[2].name, "strength");
    assert_eq!(stats[2].effective, 30);
    assert_eq!(stats[2].xp, 13150);
    assert_eq!(stats[18].name, "slayer");
    assert!(!stats[18].used, "the client table marks slayer unused");
    assert_eq!(stats[19].name, "-unused-");
    assert!(!stats[19].used, "the -unused- slot is not a skill");
    assert_eq!(stats[20].name, "runecraft");
    assert!(stats[20].used);
    assert_eq!(snap.runenergy(), 20);
    assert!(!snap.rebuild_family(&mut c, Family::Stat));
}

/// An npc rebuild copies the full actor view: tile, distance, anims,
/// health, face target, and the npc-type name/actions/level.
#[test]
fn npc_rebuild_reads_full_actor_view() {
    let mut c = client_with_npc();
    // Plant the npc type at the index `r#type` points at, so the fixture
    // works with or without a real /tmp cache.
    let mut planted_type = None;
    let npc = c.npc[7].as_mut().unwrap();
    if let Some(cache) = Arc::get_mut(&mut c.cache) {
        planted_type = Some(cache.npcs.len());
        npc.r#type = planted_type;
        cache.npcs.push(NpcType {
            id: 9,
            name: "Goblin".into(),
            op: vec![Some("Attack".into()), None],
            vislevel: 2,
            ..Default::default()
        });
    }
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c.minusedlevel = 1;
    c.local_player = Some(ClientPlayer::at(20, 12));
    {
        let e = &mut c.npc[7].as_mut().unwrap().entity;
        e.primary_anim = 809;
        e.secondary_anim = 808;
        e.yaw = 512;
        e.dst_yaw = 768;
        e.chat_message = Some("hi".into());
        e.spotanim_id = 99;
        e.health = 4;
        e.total_health = 8;
        e.face_entity = 3;
        e.route_length = 1;
        e.combat_cycle = 50;
        e.runanim = 810;
    }
    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::NPC_INFO);
    assert!(snap.rebuild_family(&mut c, Family::Npc));

    let v = &snap.npcs()[0];
    assert_eq!(v.index, 7);
    assert_eq!(v.r#type, planted_type);
    assert_eq!(v.name.as_deref(), Some("Goblin"));
    assert_eq!(v.actions, vec![Some("Attack".into()), None]);
    // entity pixel coords (100, 200), size 1 → world tile (3200, 3201)
    assert_eq!(
        v.tile,
        WorldTile {
            x: 3200,
            z: 3201,
            level: 1
        }
    );
    // chebyshev from the local player's world tile (3220, 3212)
    assert_eq!(v.distance, 20);
    assert_eq!(v.animation, 809);
    assert_eq!(v.pose_animation, 808);
    assert_eq!(v.orientation, 512);
    assert_eq!(v.target_orientation, 768);
    assert_eq!(v.overhead_text.as_deref(), Some("hi"));
    assert_eq!(v.spot_animation, 99);
    assert_eq!(v.health, 4);
    assert_eq!(v.total_health, 8);
    assert_eq!(v.face_entity, 3);
    assert_eq!(
        v.target,
        Some(ActorTargetView {
            kind: ActorKind::Npc,
            index: 3
        })
    );
    assert!(v.moving);
    assert!(v.in_combat);
    assert!(!v.running, "primary_anim 809 != runanim 810");
    assert_eq!(v.level, 2);
    assert_eq!(v.size, 1);
    // legacy position fields keep the existing consumers working
    assert_eq!(v.x, 100);
    assert_eq!(v.z, 200);
    assert_eq!(v.yaw, 512);
}

/// `face_entity` is decoded with the client's own scheme: NPC slots below
/// 32768, player slots above (offset by 32768).
#[test]
fn npc_face_target_encodes_npc_and_player_kinds() {
    let mut c = client_with_npc();
    c.npc[7].as_mut().unwrap().entity.face_entity = 3;
    let mut other = ClientNpc::default();
    other.entity.x = 300;
    other.entity.z = 100;
    other.r#type = Some(9);
    other.entity.face_entity = 7 + 32768;
    c.npc[3] = Some(Box::new(other));
    c.npc_ids[1] = 3;
    c.npc_count = 2;
    c.local_player = Some(ClientPlayer::at(20, 12));
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;

    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::NPC_INFO);
    snap.rebuild_family(&mut c, Family::Npc);

    assert_eq!(
        npc_by_index(snap.npcs(), 7).unwrap().target,
        Some(ActorTargetView {
            kind: ActorKind::Npc,
            index: 3
        })
    );
    assert_eq!(
        npc_by_index(snap.npcs(), 3).unwrap().target,
        Some(ActorTargetView {
            kind: ActorKind::Player,
            index: 7
        })
    );
}

/// Player rebuild reads the local player (`LocalPlayerView` with energy,
/// weight, distance 0) and the full remote `players` list.
#[test]
fn player_rebuild_reads_local_and_remote_players() {
    let mut c = client_with_npc();
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c.minusedlevel = 1;
    c.self_slot = 7;
    c.runenergy = 63;
    c.runweight = 24;
    c.player_op[0] = Some("Attack".into());
    c.player_op[2] = Some("Trade with".into());
    let mut local = ClientPlayer::at(20, 12);
    // rest pose: entity.x = route * 128 + size * 64, so the world-tile
    // conversion lands back on the route tile (base + route).
    local.entity.x = 20 * 128 + 64;
    local.entity.z = 12 * 128 + 64;
    local.name = Some("Zezima".into());
    local.combat_level = 126;
    local.skill_level = 99;
    local.entity.primary_anim = 808;
    local.entity.face_entity = 3; // facing an npc
    c.local_player = Some(local);
    let mut other = ClientPlayer::at(15, 16);
    other.entity.x = 100;
    other.entity.z = 150;
    other.name = Some("Other".into());
    other.combat_level = 3;
    other.skill_level = 5;
    other.entity.face_entity = 7 + 32768; // facing the local player
    c.players[3] = Some(Box::new(other));
    c.player_ids[0] = 3;
    c.player_count = 1;

    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::PLAYER_INFO);
    assert!(snap.rebuild_family(&mut c, Family::Player));

    let local_view = snap.local_player().expect("local player view");
    assert_eq!(local_view.player.index, 7);
    assert_eq!(local_view.player.actor.name.as_deref(), Some("Zezima"));
    assert_eq!(local_view.player.actor.distance, 0);
    assert_eq!(
        local_view.player.actor.tile,
        WorldTile {
            x: 3220,
            z: 3212,
            level: 1
        }
    );
    assert_eq!(
        (
            local_view.player.actor.tile.x,
            local_view.player.actor.tile.z
        ),
        (snap.tile().unwrap().0, snap.tile().unwrap().1),
        "the local actor tile matches the snapshot's world tile"
    );
    assert_eq!(
        local_view.player.actor.target,
        Some(ActorTargetView {
            kind: ActorKind::Npc,
            index: 3
        })
    );
    assert_eq!(local_view.player.actor.animation, 808);
    assert_eq!(local_view.player.combat_level, 126);
    assert_eq!(local_view.player.skill_level, 99);
    assert_eq!(local_view.energy, 63);
    assert_eq!(local_view.weight, 24);
    // actions come from the client's shared player menu ops table
    assert_eq!(
        local_view.player.actor.actions,
        vec![
            Some("Attack".into()),
            None,
            Some("Trade with".into()),
            None,
            None
        ]
    );

    let players = snap.players();
    assert_eq!(players.len(), 1);
    let p = &players[0];
    assert_eq!(p.index, 3);
    assert_eq!(p.actor.name.as_deref(), Some("Other"));
    // entity pixel coords (100, 150), size 1 → world tile (3200, 3200)
    assert_eq!(
        p.actor.tile,
        WorldTile {
            x: 3200,
            z: 3200,
            level: 1
        }
    );
    assert_eq!(p.actor.distance, 20); // chebyshev from (3220, 3212)
    assert_eq!(
        p.actor.target,
        Some(ActorTargetView {
            kind: ActorKind::Player,
            index: 7
        })
    );
    assert_eq!(p.combat_level, 3);
    assert_eq!(p.skill_level, 5);
    // The canonical tile carries the real scene level (`minusedlevel`).
    assert_eq!(snap.tile(), Some((3220, 3212, 1)));

    assert!(!snap.rebuild_family(&mut c, Family::Player));
}

/// Loc-family rebuild sweeps the sim world and resolves the loc's
/// name/description/actions and footprint from the loc table.
#[test]
fn loc_view_rebuild_reads_world_locs() {
    let mut c = client_with_npc();
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c.local_player = Some(ClientPlayer::at(20, 12));
    let id = {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        let id = cache.locs.len() as i32;
        cache.locs.push(LocType {
            id,
            name: "Large door".into(),
            desc: "A sturdy wooden door.".into(),
            op: vec![Some("Open".into()), None],
            width: 2,
            length: 3,
            blockwalk: false,
            blockrange: false,
            active: true,
            ..Default::default()
        });
        id
    };
    // A straight wall (shape 0) turned 90° (angle 1) at scene tile (3, 4).
    // The typecode packs the scene coords and the loc id; typecode2 the
    // shape/angle info byte.
    let typecode = 0x4000_0000 + (id << 14) + 3 + (4 << 7);
    c.world
        .set_wall(0, 3, 4, 0, 0, 0, typecode, 1 << 6, 0, 0, 0, 0);
    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    assert!(snap.rebuild_family(&mut c, Family::Loc));

    assert_eq!(snap.locs().len(), 1);
    let v = &snap.locs()[0];
    assert_eq!(v.typecode, typecode);
    assert_eq!(v.info, 1 << 6);
    assert_eq!(v.id, id);
    assert_eq!(v.layer, LocLayer::Wall);
    assert_eq!(v.shape, 0);
    assert_eq!(v.angle, 1);
    assert_eq!(v.name.as_deref(), Some("Large door"));
    assert_eq!(v.description.as_deref(), Some("A sturdy wooden door."));
    assert_eq!(v.actions, vec![Some("Open".into()), None]);
    // loc scene tiles are world tiles already (no pixel conversion)
    assert_eq!(
        v.tile,
        WorldTile {
            x: 3203,
            z: 3204,
            level: 0
        }
    );
    assert_eq!(v.distance, 17); // chebyshev from (3220, 3212)
    assert_eq!(v.width, 2);
    assert_eq!(v.length, 3);
    assert_eq!(v.footprint_width, 3, "angle 1 swaps the footprint");
    assert_eq!(v.footprint_length, 2);
    assert!(!v.block_walk);
    assert!(!v.block_range);
    assert!(v.active);
    assert_eq!(v.animation, -1);
    assert_eq!(v.map_function, -1);
    assert_eq!(v.map_scene, -1);
    assert_eq!(v.force_approach, 0);
    assert!(
        !snap.rebuild_family(&mut c, Family::Loc),
        "unchanged scene gen"
    );
}

/// Loc typecodes can change on the sim world after the observer already
/// consumed `gens.scene` (map restamp after REBUILD_NORMAL, a door
/// multiloc the packet applied before the gen latch). A gen-gated copy
/// would leave nav reading the previous build's door. Same pattern as
/// `scene_status_is_always_fresh_without_a_gen_bump`.
#[test]
fn loc_view_is_always_fresh_without_a_gen_bump() {
    let mut c = client_with_npc();
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c.minusedlevel = 0;
    let closed = 0x4000_0000 + (1530 << 14) + 3 + (4 << 7);
    let open = 0x4000_0000 + (1531 << 14) + 3 + (4 << 7);
    c.world
        .set_wall(0, 3, 4, 0, 0, 0, closed, 1 << 6, 0, 0, 0, 0);
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    let mut snap = GameSnapshot::new();
    assert!(snap.rebuild_family(&mut c, Family::Loc));
    assert_eq!(
        snap.locs()[0].id,
        1530,
        "first rebuild sees the closed door"
    );

    // The live loc flips with no packet / no scene gen — the snapshot
    // must still read the open leaf.
    c.world.set_wall(0, 3, 4, 0, 0, 0, open, 1 << 6, 0, 0, 0, 0);
    assert!(
        !snap.rebuild_family(&mut c, Family::Loc),
        "no scene gen moved"
    );
    assert_eq!(
        snap.locs()[0].id,
        1531,
        "loc view is always fresh, even when the scene gen did not bump"
    );
}

/// The loc sweep reads all four layers, each into its own `LocLayer`.
#[test]
fn loc_view_reads_all_four_layers() {
    let mut c = client_with_npc();
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    let wall = 0x4000_0000 + (1 << 14) + 3 + (4 << 7);
    c.world.set_wall(0, 3, 4, 0, 0, 0, wall, 1 << 6, 0, 0, 0, 0);
    let decor = 0x4000_0000 + (2 << 14) + 5 + (6 << 7);
    c.world
        .set_decor(0, 5, 6, 0, 0, 0, decor, (2 << 6) + 4, 0, 0, 0, 0, 0, 0);
    let scene = 0x4000_0000 + (3 << 14) + 7 + (8 << 7);
    c.world
        .add_scenery(0, 7, 8, 0, scene, (3 << 6) + 10, 1, 1, 0, 0, 0, 0, 0);
    let gd = 0x4000_0000 + (4 << 14) + 9 + (10 << 7);
    c.world
        .set_ground_decor(0, 9, 10, 0, gd, (3 << 6) + 22, 0, 0, 0, 0);

    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    assert!(snap.rebuild_family(&mut c, Family::Loc));

    let locs = snap.locs();
    assert_eq!(locs.len(), 4, "one view per non-empty layer");
    let wall_v = locs.iter().find(|v| v.layer == LocLayer::Wall).unwrap();
    assert_eq!(wall_v.id, 1);
    assert_eq!(
        wall_v.tile,
        WorldTile {
            x: 3203,
            z: 3204,
            level: 0
        }
    );
    let decor_v = locs
        .iter()
        .find(|v| v.layer == LocLayer::WallDecoration)
        .unwrap();
    assert_eq!(decor_v.id, 2);
    assert_eq!(decor_v.shape, 4);
    assert_eq!(decor_v.angle, 2);
    assert_eq!(
        decor_v.tile,
        WorldTile {
            x: 3205,
            z: 3206,
            level: 0
        }
    );
    let scene_v = locs.iter().find(|v| v.layer == LocLayer::Ground).unwrap();
    assert_eq!(scene_v.id, 3);
    assert_eq!(scene_v.shape, 10);
    assert_eq!(scene_v.angle, 3);
    assert_eq!(
        scene_v.tile,
        WorldTile {
            x: 3207,
            z: 3208,
            level: 0
        }
    );
    let gd_v = locs
        .iter()
        .find(|v| v.layer == LocLayer::GroundDecoration)
        .unwrap();
    assert_eq!(gd_v.id, 4);
    assert_eq!(gd_v.shape, 22);
    assert_eq!(gd_v.angle, 3);
    assert_eq!(
        gd_v.tile,
        WorldTile {
            x: 3209,
            z: 3210,
            level: 0
        }
    );
}

/// Loc ids outside the loaded loc table read the `LocType` defaults:
/// a 1×1 blocking loc with no name/description/actions.
#[test]
fn loc_view_unknown_id_reads_defaults() {
    let mut c = client_with_npc();
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    // The largest encodable loc id is beyond any loaded table.
    let id = 0x7fff;
    let typecode = 0x4000_0000 + (id << 14) + 1 + (2 << 7);
    c.world.set_wall(0, 1, 2, 0, 0, 0, typecode, 0, 0, 0, 0, 0);
    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    snap.rebuild_family(&mut c, Family::Loc);
    let v = &snap.locs()[0];
    assert_eq!(v.id, id);
    assert_eq!(v.name, None);
    assert_eq!(v.description, None);
    assert!(v.actions.is_empty());
    assert_eq!(v.width, 1);
    assert_eq!(v.length, 1);
    assert!(v.block_walk);
    assert!(v.block_range);
}

/// Ground-item rebuild reads each `ground_obj` list into a
/// `GroundItemView`: the obj's definition, the stack count, the menu ops
/// (a `Take` default fills an empty third slot) and the world tile.
#[test]
fn ground_item_view_rebuild_reads_ground_obj() {
    let mut c = client_with_npc();
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c.local_player = Some(ClientPlayer::at(20, 12));
    let (fish_id, bones_id) = {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        let fish_id = cache.objs.len() as i32;
        cache.objs.push(ObjType {
            id: fish_id,
            name: "Raw fish".into(),
            op: [None, None, Some("Cook".into()), None, None],
            ..Default::default()
        });
        let bones_id = cache.objs.len() as i32;
        cache.objs.push(ObjType {
            id: bones_id,
            name: "Bones".into(),
            ..Default::default()
        });
        (fish_id, bones_id)
    };
    let mut list = LinkList::new();
    list.push(ClientObj::new(fish_id, 1));
    list.push(ClientObj::new(bones_id, 2));
    c.ground_obj[0][10][12] = Some(Box::new(list));

    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    assert!(snap.rebuild_family(&mut c, Family::GroundItem));

    let items = snap.ground_items();
    assert_eq!(items.len(), 2, "one view per list node");
    assert_eq!(items[0].def.id, fish_id);
    assert_eq!(items[0].def.name.as_deref(), Some("Raw fish"));
    assert_eq!(items[0].count, 1);
    assert_eq!(
        items[0].actions[2].as_deref(),
        Some("Cook"),
        "a declared third op is kept"
    );
    assert_eq!(items[1].def.id, bones_id);
    assert_eq!(items[1].count, 2);
    assert_eq!(
        items[1].actions,
        vec![None, None, Some("Take".into()), None, None],
        "an empty third op defaults to Take"
    );
    assert_eq!(
        items[0].tile,
        WorldTile {
            x: 3210,
            z: 3212,
            level: 0
        }
    );
    assert_eq!(items[0].distance, 10); // chebyshev from (3220, 3212)
    assert!(!snap.rebuild_family(&mut c, Family::GroundItem));
}

/// Scene-family rebuild reads the collision map's per-tile flags, the
/// build base and the level into `SceneView`.
#[test]
fn scene_view_rebuild_reads_collision_flags() {
    let mut c = client_with_npc();
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c.minusedlevel = 1;
    // A straight wall at scene (5, 6) facing west: W_W lands on (5, 6)
    // and its east block on (4, 6).
    c.collision[1].add_wall(5, 6, 0, 0, false);

    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    assert!(snap.rebuild_family(&mut c, Family::Scene));

    let scene = snap.scene();
    assert!(scene.available);
    assert_eq!(scene.base_x, 3200);
    assert_eq!(scene.base_z, 3200);
    assert_eq!(scene.level, 1);
    assert_eq!(scene.width, 104);
    assert_eq!(scene.height, 104);
    assert_eq!(scene.collision_flags.len(), 104 * 104);
    let flag_at = |x: usize, z: usize| scene.collision_flags[x * 104 + z];
    assert_eq!(flag_at(5, 6), CollisionFlag::W_W);
    assert_eq!(flag_at(4, 6), CollisionFlag::W_E);
    assert_eq!(flag_at(50, 50), CollisionFlag::_OPEN, "open tile stays 0");
    assert!(!snap.rebuild_family(&mut c, Family::Scene));
}

/// World-state rebuild copies the client's world scalars.
#[test]
fn world_view_rebuild_reads_client_state() {
    let mut c = client_with_npc();
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c.minusedlevel = 2;
    c.members_account = 1;
    c.in_multizone = 1;
    c.player_count = 7;
    c.npc_count = 3;
    c.loop_cycle = 412;
    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::SET_MULTIWAY);
    assert!(snap.rebuild_family(&mut c, Family::World));

    let w = snap.world();
    assert_eq!(w.map_base_x, 3200);
    assert_eq!(w.map_base_z, 3200);
    assert_eq!(w.level, 2);
    assert!(w.members, "members_account != 0");
    assert!(w.multi_combat, "the SET_MULTIWAY in_multizone flag");
    assert_eq!(w.player_count, 7);
    assert_eq!(w.npc_count, 3);
    assert_eq!(w.cycle, 412);
    // Cheap scalars copy every rebuild, so counts stay fresh without a
    // world-gen bump (like the scene family's always-fresh status).
    c.player_count = 8;
    assert!(!snap.rebuild_family(&mut c, Family::World));
    assert_eq!(snap.world().player_count, 8);
}

/// Camera rebuild copies the follow camera, the orbit state and the
/// cinematic flag.
#[test]
fn camera_view_rebuild_reads_client_camera() {
    let mut c = client_with_npc();
    c.cam_x = 1;
    c.cam_y = 2;
    c.cam_z = 3;
    c.cam_pitch = 4;
    c.cam_yaw = 5;
    c.orbit_camera_pitch = 6;
    c.orbit_camera_yaw = 7;
    c.cinema_cam = true;
    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::CAM_LOOKAT);
    assert!(snap.rebuild_family(&mut c, Family::Camera));

    let cam = snap.camera();
    assert_eq!(cam.x, 1);
    assert_eq!(cam.y, 2);
    assert_eq!(cam.z, 3);
    assert_eq!(cam.pitch, 4);
    assert_eq!(cam.yaw, 5);
    assert_eq!(cam.orbit_pitch, 6);
    assert_eq!(cam.orbit_yaw, 7);
    assert!(cam.cinematic);
    // The follow camera eases every frame with no packet; always fresh.
    c.cam_y = 99;
    assert!(!snap.rebuild_family(&mut c, Family::Camera));
    assert_eq!(snap.camera().y, 99);
}

/// MapFlag reads the minimap flag as `Some` only while it is set.
#[test]
fn map_flag_view_reads_minimap_flag() {
    let mut c = client_with_npc();
    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::UNSET_MAP_FLAG);
    assert!(snap.rebuild_family(&mut c, Family::MapFlag));
    assert_eq!(snap.map_flag(), None, "flag 0 reads None");

    c.minimap_flag_x = 12;
    c.minimap_flag_z = 34;
    c.bump_gens(ServerProt::UNSET_MAP_FLAG);
    assert!(snap.rebuild_family(&mut c, Family::MapFlag));
    let flag = snap.map_flag().expect("flag set");
    assert_eq!(flag.lx, 12);
    assert_eq!(flag.lz, 34);

    // A minimap click sets the flag locally with no packet (and UNSET
    // clears it the same way); the view stays fresh like the scene status.
    c.minimap_flag_x = 0;
    c.minimap_flag_z = 0;
    assert!(
        !snap.rebuild_family(&mut c, Family::MapFlag),
        "no gen moved"
    );
    assert_eq!(snap.map_flag(), None, "the flag view is always fresh");
}

// ---- Task 4: iface-derived views (items, widgets, side tabs, trade,
// chat lines/options, make products, quests, controls, modals, menu) ----

/// Place a component in the iface table at `id` (growing the table).
fn set_iface(c: &mut Client, id: usize, com: IfType) {
    c.set_iface(id, com);
}

fn set_iface_mut(c: &mut Client, id: usize, m: IfTypeMut) {
    c.set_iface_mut(id, m);
}

/// Plant an obj def at `id` so def-name resolution reads it.
fn plant_obj(c: &mut Client, id: i32, name: &str) {
    let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
    if cache.objs.len() <= id as usize {
        cache.objs.resize(id as usize + 1, ObjType::default());
    }
    cache.objs[id as usize] = ObjType {
        id,
        name: name.into(),
        ..Default::default()
    };
}

/// The inventory tab TYPE_INV root (stored obj ids are `obj + 1`, 0
/// empty) and a bank iface under an open main modal; the new item
/// families read both. Held inventory ops come from the obj def's `iop`
/// with a `Drop` default; component ops come from the iface's own `iop`.
#[test]
fn item_view_reads_inventory_and_bank_containers() {
    let mut c = client_with_npc();
    plant_obj(&mut c, 3, "Coins");
    plant_obj(&mut c, 4, "Sword");
    plant_obj(&mut c, 5, "Lobster");
    set_iface(
        &mut c,
        500,
        IfType {
            id: 500,
            r#type: ComponentType::TYPE_INV,
            iop: [None, None, None, None, None],
            obj_ops: true,
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        500,
        IfTypeMut {
            link_obj_type: Some(vec![4, 5, 0]),
            link_obj_number: Some(vec![1, 100, 0]),
            ..Default::default()
        },
    );

    set_iface(
        &mut c,
        600,
        IfType {
            id: 600,
            layer_id: 600,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![601]),
            ..Default::default()
        },
    );
    set_iface(
        &mut c,
        601,
        IfType {
            id: 601,
            layer_id: 600,
            r#type: ComponentType::TYPE_INV,
            iop: [Some("Withdraw 1".into()), None, None, None, None],
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        601,
        IfTypeMut {
            link_obj_type: Some(vec![6, 0]),
            link_obj_number: Some(vec![2, 0]),
            ..Default::default()
        },
    );

    c.main_modal_id = 600;
    c.side_icon[3] = 500;

    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::UPDATE_INV_FULL);
    assert!(snap.rebuild_family(&mut c, Family::Inventory));
    assert!(snap.rebuild_family(&mut c, Family::Bank));

    let inv = snap.inventory();
    assert_eq!(inv.len(), 2);
    assert_eq!(inv[0].container, ItemContainer::Inventory);
    assert_eq!(inv[0].action_family, ItemActionFamily::Held);
    assert_eq!(inv[0].slot, 0);
    assert_eq!(inv[0].count, 1);
    assert_eq!(inv[0].def.id, 3, "stored 4 decodes to obj 3");
    assert_eq!(inv[0].def.name.as_deref(), Some("Coins"));
    assert_eq!(
        inv[0].actions,
        vec![None, None, None, None, Some("Drop".into())]
    );
    assert_eq!(inv[0].component_id, 500);
    assert_eq!(inv[1].slot, 1);
    assert_eq!(inv[1].count, 100);
    assert_eq!(inv[1].def.id, 4);
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        cache.objs[4].iop = [Some("Wield".into()), None, None, None, None];
    }
    c.bump_gens(ServerProt::UPDATE_INV_FULL);
    assert!(snap.rebuild_family(&mut c, Family::Inventory));
    assert!(
        snap.rebuild_family(&mut c, Family::Bank),
        "the inv gen moved"
    );
    assert_eq!(
        snap.inventory()[1].actions,
        vec![Some("Wield".into()), None, None, None, Some("Drop".into())],
        "held ops keep the Drop default in the fifth slot"
    );

    let bank = snap.bank();
    assert_eq!(bank.len(), 1);
    assert_eq!(bank[0].container, ItemContainer::Bank);
    assert_eq!(bank[0].action_family, ItemActionFamily::Component);
    assert_eq!(bank[0].slot, 0);
    assert_eq!(bank[0].count, 2);
    assert_eq!(bank[0].def.id, 5);
    assert_eq!(bank[0].def.name.as_deref(), Some("Lobster"));
    assert_eq!(bank[0].actions[0].as_deref(), Some("Withdraw 1"));
    assert_eq!(bank[0].component_id, 601);

    assert!(
        !snap.rebuild_family(&mut c, Family::Inventory),
        "unchanged gens"
    );
    assert!(!snap.rebuild_family(&mut c, Family::Bank));
}

/// The bank-side (deposit) container reads the side modal's `deposit`
/// component; the equipment container reads the worn-items tab.
#[test]
fn bank_side_and_equipment_read_their_components() {
    let mut c = client_with_npc();
    plant_obj(&mut c, 3, "Coins");
    plant_obj(&mut c, 6, "Bones");
    set_iface(
        &mut c,
        700,
        IfType {
            id: 700,
            layer_id: 700,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![701]),
            ..Default::default()
        },
    );
    set_iface(
        &mut c,
        701,
        IfType {
            id: 701,
            layer_id: 700,
            r#type: ComponentType::TYPE_INV,
            iop: [Some("Deposit All".into()), None, None, None, None],
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        701,
        IfTypeMut {
            link_obj_type: Some(vec![4, 0]),
            link_obj_number: Some(vec![7, 0]),
            ..Default::default()
        },
    );

    c.side_modal_id = 700;
    // worn-items tab 4: a layer wrapping a TYPE_INV `wear` component.
    set_iface(
        &mut c,
        710,
        IfType {
            id: 710,
            layer_id: 710,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![711]),
            ..Default::default()
        },
    );
    set_iface(
        &mut c,
        711,
        IfType {
            id: 711,
            layer_id: 710,
            r#type: ComponentType::TYPE_INV,
            iop: [Some("Remove".into()), None, None, None, None],
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        711,
        IfTypeMut {
            link_obj_type: Some(vec![7, 0]),
            link_obj_number: Some(vec![1, 0]),
            ..Default::default()
        },
    );

    c.side_icon[4] = 710;

    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::UPDATE_INV_FULL);
    assert!(snap.rebuild_family(&mut c, Family::BankSide));
    assert!(snap.rebuild_family(&mut c, Family::Equipment));

    let side = snap.bank_side();
    assert_eq!(side.len(), 1);
    assert_eq!(side[0].container, ItemContainer::BankSide);
    assert_eq!(side[0].def.id, 3);
    assert_eq!(side[0].count, 7);
    assert_eq!(side[0].actions[0].as_deref(), Some("Deposit All"));

    let worn = snap.equipment();
    assert_eq!(worn.len(), 1);
    assert_eq!(worn[0].container, ItemContainer::Equipment);
    assert_eq!(worn[0].component_id, 711);
    assert_eq!(worn[0].def.id, 6);
    assert_eq!(worn[0].actions[0].as_deref(), Some("Remove"));
}

/// A widget tree walk maps every IfType field onto the WidgetView, keeps
/// the accumulated position and the walk parent, and derives varp-bound
/// scripts (opcode 5) from `scripts` + `script_comparator`/`script_operand`.
#[test]
fn widget_view_reads_iface_fields_and_varp_bindings() {
    let mut c = client_with_npc();
    set_iface(
        &mut c,
        1000,
        IfType {
            id: 1000,
            layer_id: 1000,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![1001, 1002]),
            child_x: Some(vec![5, -3]),
            child_y: Some(vec![6, 4]),
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        1000,
        IfTypeMut {
            scroll_height: 300,
            ..Default::default()
        },
    );

    set_iface(
        &mut c,
        1001,
        IfType {
            id: 1001,
            layer_id: 1000,
            r#type: ComponentType::TYPE_TEXT,
            text2: "alt".into(),
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        1001,
        IfTypeMut {
            text: "Hello".into(),
            colour: 0x00FF00,
            ..Default::default()
        },
    );

    set_iface(
        &mut c,
        1002,
        IfType {
            id: 1002,
            layer_id: 1000,
            r#type: ComponentType::TYPE_GRAPHIC,
            client_code: 7,
            width: 36,
            height: 36,
            scripts: Some(vec![vec![5, 44], vec![7, 8, 9]]),
            script_comparator: Some(vec![1, 0]),
            script_operand: Some(vec![2, 0]),
            button_text: "Select".into(),
            target_verb: "Use".into(),
            target_base: "Fountain".into(),
            target_mask: 4,
            model2_type: 4,
            model2_id: 527,
            iop: [Some("Toggle".into()), None, None, None, None],
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        1002,
        IfTypeMut {
            button_type: ButtonType::BUTTON_SELECT,
            x: 9,
            y: 10,
            scroll_pos: 7,
            hide: true,
            model1_type: 4,
            model1_id: 526,
            colour: 0x123456,
            ..Default::default()
        },
    );

    c.main_modal_id = 1000;

    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::IF_SETTEXT);
    assert!(snap.rebuild_family(&mut c, Family::Widgets));

    let ws = snap.widgets();
    assert_eq!(ws.len(), 3, "root plus two children");
    let root = &ws[0];
    assert_eq!(root.component_id, 1000);
    assert_eq!(root.parent_id, -1);
    assert_eq!(root.root_component_id, 1000);
    assert_eq!(root.root, WidgetRoot::Main);
    assert_eq!(root.kind, WidgetKind::Widget);
    assert_eq!(root.layer_id, 1000);
    assert_eq!(root.type_, ComponentType::TYPE_LAYER);
    assert_eq!(root.scroll_height, 300);
    assert!(!root.hidden);

    let text = ws.iter().find(|w| w.component_id == 1001).unwrap();
    assert_eq!(text.parent_id, 1000);
    assert_eq!((text.x, text.y), (5, 6), "child offsets accumulate");
    assert_eq!(text.text.as_deref(), Some("Hello"));
    assert_eq!(text.alternate_text.as_deref(), Some("alt"));
    assert_eq!(text.colour, 0x00FF00);

    let btn = ws.iter().find(|w| w.component_id == 1002).unwrap();
    assert_eq!(btn.parent_id, 1000);
    assert_eq!((btn.x, btn.y), (-3, 4));
    assert_eq!(btn.type_, ComponentType::TYPE_GRAPHIC);
    assert_eq!(btn.button_type, ButtonType::BUTTON_SELECT);
    assert_eq!(btn.client_code, 7);
    assert_eq!((btn.width, btn.height), (36, 36));
    assert_eq!(btn.scroll_position, 7);
    assert!(btn.hidden);
    assert_eq!(btn.button_text.as_deref(), Some("Select"));
    assert_eq!(btn.target_verb.as_deref(), Some("Use"));
    assert_eq!(btn.target_base.as_deref(), Some("Fountain"));
    assert_eq!(btn.target_mask, 4);
    assert_eq!(btn.model_type, 4);
    assert_eq!(btn.model_id, 526);
    assert_eq!(btn.alternate_model_type, 4);
    assert_eq!(btn.alternate_model_id, 527);
    assert_eq!(
        btn.scripts,
        Some(vec![Some(vec![5, 44]), Some(vec![7, 8, 9])])
    );
    assert_eq!(btn.script_comparators, Some(vec![1, 0]));
    assert_eq!(btn.script_operands, Some(vec![2, 0]));
    assert_eq!(
        btn.varp_bindings,
        vec![WidgetVarpBindingView {
            script_index: 0,
            varp: 44,
            value: Some(2),
            comparator: Some(1),
        }],
        "only the opcode-5 script is varp-bound"
    );
    assert_eq!(btn.colour, 0x123456);
    assert_eq!(
        btn.actions,
        vec![Some("Toggle".into()), None, None, None, None]
    );

    assert!(
        !snap.rebuild_family(&mut c, Family::Widgets),
        "unchanged gens"
    );
}

/// TYPE_INV components inside a widget tree expose their slots as
/// widget-container items (component action family).
#[test]
fn widget_items_read_inv_component_slots() {
    let mut c = client_with_npc();
    plant_obj(&mut c, 3, "Coins");
    plant_obj(&mut c, 5, "Lobster");
    set_iface(
        &mut c,
        1000,
        IfType {
            id: 1000,
            layer_id: 1000,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![1003]),
            ..Default::default()
        },
    );
    set_iface(
        &mut c,
        1003,
        IfType {
            id: 1003,
            layer_id: 1000,
            r#type: ComponentType::TYPE_INV,
            iop: [Some("Use".into()), None, None, None, None],
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        1003,
        IfTypeMut {
            link_obj_type: Some(vec![4, 0, 6]),
            link_obj_number: Some(vec![1, 0, 2]),
            ..Default::default()
        },
    );

    c.main_modal_id = 1000;

    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::UPDATE_INV_FULL);
    assert!(snap.rebuild_family(&mut c, Family::Widgets));

    let items = &snap
        .widgets()
        .iter()
        .find(|w| w.component_id == 1003)
        .expect("the inv component is in the tree")
        .items;
    assert_eq!(items.len(), 2, "empty slots are skipped");
    assert_eq!(items[0].def.id, 3);
    assert_eq!(items[0].count, 1);
    assert_eq!(items[0].slot, 0);
    assert_eq!(items[0].container, ItemContainer::Widget);
    assert_eq!(items[0].action_family, ItemActionFamily::Component);
    assert_eq!(items[0].component_id, 1003);
    assert_eq!(items[0].actions[0].as_deref(), Some("Use"));
    assert_eq!(items[1].def.id, 5);
    assert_eq!(items[1].count, 2);
    assert_eq!(items[1].slot, 2);
}

/// Widget roots come from the open main modal/overlay, the side modal
/// (else the active tab), the chat modal and the tutorial overlay.
#[test]
fn widget_roots_cover_main_side_chat_and_tutorial() {
    let mut c = client_with_npc();
    for (id, label) in [
        (1000, "main"),
        (1100, "side"),
        (1200, "chat"),
        (1300, "tut"),
    ] {
        set_iface(
            &mut c,
            id as usize,
            IfType {
                id,
                layer_id: id,
                r#type: ComponentType::TYPE_LAYER,
                children: Some(vec![id + 1]),
                ..Default::default()
            },
        );
        set_iface(
            &mut c,
            (id + 1) as usize,
            IfType {
                id: id + 1,
                layer_id: id,
                r#type: ComponentType::TYPE_TEXT,
                ..Default::default()
            },
        );
        set_iface_mut(
            &mut c,
            (id + 1) as usize,
            IfTypeMut {
                text: label.into(),
                ..Default::default()
            },
        );
    }
    c.main_modal_id = 1000;
    c.side_modal_id = 1100;
    c.chat_modal_id = 1200;
    c.tut_com_id = 1300;

    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::IF_OPENMAIN_SIDE);
    assert!(snap.rebuild_family(&mut c, Family::Widgets));

    let root_of = |root: WidgetRoot| {
        snap.widgets()
            .iter()
            .find(|w| w.root == root)
            .expect("a widget under each open root")
            .root_component_id
    };
    assert_eq!(root_of(WidgetRoot::Main), 1000);
    assert_eq!(root_of(WidgetRoot::Side), 1100);
    assert_eq!(root_of(WidgetRoot::Chat), 1200);
    assert_eq!(root_of(WidgetRoot::Tutorial), 1300);

    // Closing the side modal falls back to the active tab's interface.
    c.side_modal_id = -1;
    c.side_icon[3] = 1100;
    c.active_icon = 3;
    c.bump_gens(ServerProt::IF_CLOSE);
    assert!(snap.rebuild_family(&mut c, Family::Widgets));
    assert!(snap
        .widgets()
        .iter()
        .any(|w| w.root == WidgetRoot::Side && w.root_component_id == 1100));
}

/// Side tabs report the 14 slots with available/active/visible state and
/// their widget tree; an open side modal hides the active tab.
#[test]
fn side_tabs_report_available_active_visible_and_widgets() {
    let mut c = client_with_npc();
    set_iface(
        &mut c,
        1100,
        IfType {
            id: 1100,
            layer_id: 1100,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![1101]),
            ..Default::default()
        },
    );
    set_iface(
        &mut c,
        1101,
        IfType {
            id: 1101,
            layer_id: 1100,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        1101,
        IfTypeMut {
            text: "abc".into(),
            ..Default::default()
        },
    );

    c.side_icon[3] = 1100;
    c.active_icon = 3;

    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::IF_SETICON);
    assert!(snap.rebuild_family(&mut c, Family::SideTabs));

    let tabs = snap.side_tabs();
    assert_eq!(tabs.len(), 14);
    let tab = &tabs[3];
    assert_eq!(tab.index, 3);
    assert_eq!(tab.root_component_id, 1100);
    assert!(tab.available);
    assert!(tab.active);
    assert!(tab.visible);
    assert_eq!(tab.widgets.len(), 2, "root + child");
    assert!(!tabs[0].available, "unset tabs stay closed");
    assert!(!tabs[0].active);

    c.side_modal_id = 1200;
    c.bump_gens(ServerProt::IF_OPENSIDE);
    assert!(snap.rebuild_family(&mut c, Family::SideTabs));
    assert!(snap.side_tabs()[3].active, "the tab stays selected");
    assert!(!snap.side_tabs()[3].visible, "a side modal hides the tab");
}

/// Trade views read the hardcoded 274 trade components (trademain 3323,
/// tradeconfirm 3443, inv 3415, otherinv 3416, otherplayer 3417, side
/// pack 3322) with the m8aq container/action families and partner
/// normalization.
#[test]
fn trade_view_reads_offer_confirm_and_containers() {
    let mut c = client_with_npc();
    plant_obj(&mut c, 3, "Coins");
    plant_obj(&mut c, 5, "Lobster");
    plant_obj(&mut c, 6, "Bones");
    set_iface(
        &mut c,
        3323,
        IfType {
            id: 3323,
            layer_id: 3323,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![3415, 3416, 3417]),
            ..Default::default()
        },
    );
    set_iface(
        &mut c,
        3415,
        IfType {
            id: 3415,
            layer_id: 3323,
            r#type: ComponentType::TYPE_INV,
            iop: [Some("Remove 1".into()), None, None, None, None],
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        3415,
        IfTypeMut {
            link_obj_type: Some(vec![4, 0]),
            link_obj_number: Some(vec![1, 0]),
            ..Default::default()
        },
    );

    set_iface(
        &mut c,
        3416,
        IfType {
            id: 3416,
            layer_id: 3323,
            r#type: ComponentType::TYPE_INV,
            iop: [None, None, None, None, None],
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        3416,
        IfTypeMut {
            link_obj_type: Some(vec![6, 0]),
            link_obj_number: Some(vec![3, 0]),
            ..Default::default()
        },
    );

    set_iface(
        &mut c,
        3417,
        IfType {
            id: 3417,
            layer_id: 3323,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        3417,
        IfTypeMut {
            text: "Trading With: Zezima".into(),
            ..Default::default()
        },
    );

    set_iface(
        &mut c,
        3321,
        IfType {
            id: 3321,
            layer_id: 3321,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![3322]),
            ..Default::default()
        },
    );
    set_iface(
        &mut c,
        3322,
        IfType {
            id: 3322,
            layer_id: 3321,
            r#type: ComponentType::TYPE_INV,
            iop: [Some("Offer".into()), None, None, None, None],
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        3322,
        IfTypeMut {
            link_obj_type: Some(vec![7, 0]),
            link_obj_number: Some(vec![2, 0]),
            ..Default::default()
        },
    );

    c.main_modal_id = 3323;

    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::IF_OPENMAIN_SIDE);
    assert!(snap.rebuild_family(&mut c, Family::Trade));

    let t = snap.trade();
    assert!(t.offer_open);
    assert!(!t.confirm_open);
    assert_eq!(t.partner.as_deref(), Some("Zezima"), "prefix stripped");
    assert_eq!(t.my_offer.len(), 1);
    assert_eq!(t.my_offer[0].container, ItemContainer::TradeMyOffer);
    assert_eq!(t.my_offer[0].action_family, ItemActionFamily::Component);
    assert_eq!(t.my_offer[0].def.id, 3);
    assert_eq!(t.my_offer[0].count, 1);
    assert_eq!(t.my_offer[0].component_id, 3415);
    assert_eq!(t.my_offer[0].actions[0].as_deref(), Some("Remove 1"));
    assert_eq!(t.their_offer.len(), 1);
    assert_eq!(t.their_offer[0].container, ItemContainer::TradeTheirOffer);
    assert_eq!(t.their_offer[0].action_family, ItemActionFamily::None);
    assert_eq!(t.their_offer[0].def.id, 5);
    assert_eq!(t.their_offer[0].count, 3);
    assert_eq!(t.side_pack.len(), 1);
    assert_eq!(t.side_pack[0].container, ItemContainer::TradeSidePack);
    assert_eq!(t.side_pack[0].def.id, 6);
    assert_eq!(t.side_pack[0].count, 2);
    assert_eq!(t.side_pack[0].actions[0].as_deref(), Some("Offer"));

    // The confirm screen is a different main modal.
    c.main_modal_id = 3443;
    c.bump_gens(ServerProt::IF_OPENMAIN_SIDE);
    assert!(snap.rebuild_family(&mut c, Family::Trade));
    assert!(snap.trade().confirm_open);
    assert!(!snap.trade().offer_open);

    // A partner label with no prefix keeps the raw name.
    c.set_iface(
        3417,
        IfType {
            ..c.if_(3417).unwrap().clone()
        },
    );
    c.set_iface_mut(
        3417,
        IfTypeMut {
            text: "  Smithy Bob".into(),
            ..Default::default()
        },
    );
    c.bump_gens(ServerProt::IF_SETTEXT);
    assert!(snap.rebuild_family(&mut c, Family::Trade));
    assert_eq!(snap.trade().partner.as_deref(), Some("Smithy Bob"));
}

/// Chat lines read the full ring (index 0 = newest) with a monotonic
/// sequence that bumps when the head moves.
#[test]
fn chat_lines_read_the_full_ring_in_order() {
    let mut c = client_with_npc();
    c.add_chat(4, "first", "Zezima");
    c.add_chat(0, "second", "");

    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::MESSAGE_GAME);
    assert!(snap.rebuild_family(&mut c, Family::Chat));

    let lines = snap.chat_lines();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].text, "second", "index 0 is the newest line");
    assert_eq!(lines[0].type_, 0);
    assert_eq!(lines[0].username, None, "server lines carry no sender");
    assert_eq!(lines[1].text, "first");
    assert_eq!(lines[1].type_, 4);
    assert_eq!(lines[1].username.as_deref(), Some("Zezima"));
    assert!(
        lines[0].sequence > lines[1].sequence,
        "newer lines have higher sequences"
    );

    let head_seq = lines[0].sequence;
    c.add_chat(0, "third", "");
    c.bump_gens(ServerProt::MESSAGE_GAME);
    assert!(snap.rebuild_family(&mut c, Family::Chat));
    let lines = snap.chat_lines();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0].text, "third");
    assert!(
        lines[0].sequence > head_seq,
        "a new head bumps the sequence"
    );
    assert_eq!(snap.chat(), Some("third"), "the head accessor stays");
}

/// Two messages added in a single gen bump get distinct, stable sequences:
/// the client bumps `chat_seq` once per `add_chat`, so every burst line
/// reads newer than the previous head's sequence and `since` queries miss
/// none of them.
#[test]
fn chat_burst_in_one_gen_gets_distinct_sequences() {
    let mut c = client_with_npc();
    c.add_chat(0, "one", "");
    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::MESSAGE_GAME);
    assert!(snap.rebuild_family(&mut c, Family::Chat));
    let last = snap.chat_lines()[0].sequence;

    // A burst of two messages before the next gen bump.
    c.add_chat(0, "two", "");
    c.add_chat(0, "three", "");
    c.bump_gens(ServerProt::MESSAGE_GAME);
    assert!(snap.rebuild_family(&mut c, Family::Chat));

    let lines = snap.chat_lines();
    assert_eq!(lines[0].text, "three", "index 0 is the newest line");
    assert_eq!(lines[1].text, "two");
    assert!(
        lines[0].sequence > last && lines[1].sequence > last,
        "every burst message is new since the previous head"
    );
    assert_ne!(
        lines[0].sequence, lines[1].sequence,
        "each burst message gets its own per-message sequence"
    );
    let (three_seq, two_seq) = (lines[0].sequence, lines[1].sequence);

    // Sequences are stable: a rebuild with no new chat keeps them.
    c.bump_gens(ServerProt::MESSAGE_GAME);
    assert!(snap.rebuild_family(&mut c, Family::Chat));
    let lines = snap.chat_lines();
    assert_eq!(lines[0].sequence, three_seq);
    assert_eq!(lines[1].sequence, two_seq);
}

/// `rebuild`/`rebuild_family` take `&Client`: a shared borrow suffices
/// now that the ground-item sweep uses the immutable `LinkList` iterator.
#[test]
fn rebuild_reads_a_shared_client_borrow() {
    let mut c = client_with_npc();
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    let client = &c;
    let mut snap = GameSnapshot::new();
    assert!(snap.rebuild(client));
    assert_eq!(snap.npcs().len(), 1);
    assert!(!snap.rebuild(client), "unchanged gens are not dirty again");
    assert!(!snap.rebuild_family(client, Family::Npc));
}

/// Chat options collect the BUTTON_OK labels of the chat modal; the
/// continue button is the modal's BUTTON_CONTINUE child (hidden while the
/// pause button is latched).
#[test]
fn chat_options_and_continue_read_the_chat_modal() {
    let mut c = client_with_npc();
    set_iface(
        &mut c,
        2000,
        IfType {
            id: 2000,
            layer_id: 2000,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![2001, 2002, 2003]),
            ..Default::default()
        },
    );
    set_iface(
        &mut c,
        2001,
        IfType {
            id: 2001,
            layer_id: 2000,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        2001,
        IfTypeMut {
            button_type: ButtonType::BUTTON_OK,
            text: "Yes".into(),
            ..Default::default()
        },
    );

    set_iface(
        &mut c,
        2002,
        IfType {
            id: 2002,
            layer_id: 2000,
            r#type: ComponentType::TYPE_TEXT,
            button_text: "No thanks".into(),
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        2002,
        IfTypeMut {
            button_type: ButtonType::BUTTON_OK,
            ..Default::default()
        },
    );

    set_iface(
        &mut c,
        2003,
        IfType {
            id: 2003,
            layer_id: 2000,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        2003,
        IfTypeMut {
            button_type: ButtonType::BUTTON_CONTINUE,
            ..Default::default()
        },
    );

    c.chat_modal_id = 2000;

    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::IF_OPENCHAT);
    assert!(snap.rebuild_family(&mut c, Family::ChatOptions));

    assert_eq!(snap.chat_continue_component_id(), 2003);
    let options = snap.chat_options();
    assert_eq!(options.len(), 2);
    assert_eq!(options[0].component_id, 2001);
    assert_eq!(options[0].text, "Yes");
    assert_eq!(options[1].component_id, 2002);
    assert_eq!(options[1].text, "No thanks");

    c.resumed_pause_button = true;
    c.bump_gens(ServerProt::IF_SETTEXT);
    assert!(snap.rebuild_family(&mut c, Family::ChatOptions));
    assert_eq!(
        snap.chat_continue_component_id(),
        -1,
        "latched pause hides continue"
    );
}

/// Make products come from the chat (or main) modal's obj-model
/// components; the make/smelt buttons are grouped four per product.
#[test]
fn make_products_read_the_make_modal() {
    let mut c = client_with_npc();
    plant_obj(&mut c, 800, "Pot");
    plant_obj(&mut c, 801, "Bowl");
    set_iface(
        &mut c,
        2100,
        IfType {
            id: 2100,
            layer_id: 2100,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![2110, 2111, 2120, 2121, 2130, 2131]),
            ..Default::default()
        },
    );
    set_iface(
        &mut c,
        2110,
        IfType {
            id: 2110,
            layer_id: 2100,
            r#type: ComponentType::TYPE_MODEL,
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        2110,
        IfTypeMut {
            model1_type: 4,
            model1_id: 800,
            ..Default::default()
        },
    );

    set_iface(
        &mut c,
        2111,
        IfType {
            id: 2111,
            layer_id: 2100,
            r#type: ComponentType::TYPE_MODEL,
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        2111,
        IfTypeMut {
            model1_type: 4,
            model1_id: 801,
            ..Default::default()
        },
    );

    for (id, label) in [
        (2120, "Make X"),
        (2121, "Make 10"),
        (2130, "Make 5"),
        (2131, "Make 1"),
    ] {
        set_iface(
            &mut c,
            id as usize,
            IfType {
                id,
                layer_id: 2100,
                r#type: ComponentType::TYPE_TEXT,
                button_text: label.into(),
                ..Default::default()
            },
        );
        set_iface_mut(
            &mut c,
            id as usize,
            IfTypeMut {
                button_type: ButtonType::BUTTON_OK,
                ..Default::default()
            },
        );
    }
    c.chat_modal_id = 2100;

    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::IF_OPENCHAT);
    assert!(snap.rebuild_family(&mut c, Family::MakeProducts));

    let products = snap.make_products();
    assert_eq!(products.len(), 2);
    assert_eq!(products[0].object_id, 800);
    assert_eq!(products[0].name, "Pot");
    assert_eq!(products[0].buttons.len(), 4);
    assert_eq!(products[0].buttons[0].quantity, -1, "Make X reads -1");
    assert_eq!(products[0].buttons[0].component_id, 2120);
    assert_eq!(products[0].buttons[1].quantity, 10);
    assert_eq!(products[0].buttons[2].quantity, 5);
    assert_eq!(products[0].buttons[3].quantity, 1);
    assert_eq!(products[1].object_id, 801);
    assert_eq!(products[1].name, "Bowl");
    assert!(
        products[1].buttons.is_empty(),
        "buttons are grouped per product"
    );
}

/// The mysterious-cube random event is a main modal with three TYPE_MODEL
/// obj displays and no Make/Smelt buttons. Walking it as a make-X tree
/// used to panic: `buttons[4..0]` on an empty slice.
#[test]
fn make_products_ignores_mysterious_cube_obj_models() {
    let mut c = client_with_npc();
    plant_obj(&mut c, 3063, "Red triangle");
    plant_obj(&mut c, 3069, "Red square");
    plant_obj(&mut c, 3081, "Red star");
    set_iface(
        &mut c,
        6554,
        IfType {
            id: 6554,
            layer_id: 6554,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![6555, 6557, 6559]),
            ..Default::default()
        },
    );
    for (id, obj) in [(6555, 3063), (6557, 3069), (6559, 3081)] {
        set_iface(
            &mut c,
            id as usize,
            IfType {
                id,
                layer_id: 6554,
                r#type: ComponentType::TYPE_MODEL,
                ..Default::default()
            },
        );
        set_iface_mut(
            &mut c,
            id as usize,
            IfTypeMut {
                model1_type: 4,
                model1_id: obj,
                ..Default::default()
            },
        );
    }
    c.main_modal_id = 6554;
    c.chat_modal_id = -1;

    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::IF_OPENMAIN);
    assert!(snap.rebuild_family(&mut c, Family::MakeProducts));
    assert!(
        snap.make_products().is_empty(),
        "cube obj models are not make-X products"
    );
}

/// Quest statuses read the quest tab's (side tab 2) TYPE_TEXT entries
/// with their colours.
#[test]
fn quest_statuses_read_the_quest_tab() {
    let mut c = client_with_npc();
    set_iface(
        &mut c,
        2200,
        IfType {
            id: 2200,
            layer_id: 2200,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![2201, 2202, 2203]),
            ..Default::default()
        },
    );
    set_iface(
        &mut c,
        2201,
        IfType {
            id: 2201,
            layer_id: 2200,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        2201,
        IfTypeMut {
            text: "Cook's Assistant".into(),
            colour: 0x123456,
            ..Default::default()
        },
    );

    set_iface(
        &mut c,
        2202,
        IfType {
            id: 2202,
            layer_id: 2200,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    set_iface(
        &mut c,
        2203,
        IfType {
            id: 2203,
            layer_id: 2200,
            r#type: ComponentType::TYPE_RECT,
            ..Default::default()
        },
    );
    c.side_icon[2] = 2200;

    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::IF_SETICON);
    assert!(snap.rebuild_family(&mut c, Family::QuestStatuses));

    let statuses = snap.quest_statuses();
    assert_eq!(
        statuses.len(),
        1,
        "empty text and non-text entries are skipped"
    );
    assert_eq!(statuses[0].component_id, 2201);
    assert_eq!(statuses[0].name, "Cook's Assistant");
    assert_eq!(statuses[0].colour, 0x123456);
}

/// The run/retaliate toggles come from the player-controls overlay: the
/// root with an "Auto retaliate" label; run sits at children 4/5
/// (off/on), retaliate at 2/3 (on/off) — the 274 `controls.if` buttons.
/// The fixture sits at root id 0 so the table scan finds it first even
/// when a real cache is present in the test cache dir.
#[test]
fn controls_read_the_player_controls_overlay() {
    let mut c = client_with_npc();
    let children: Vec<i32> = (1..=11).collect();
    set_iface(
        &mut c,
        0,
        IfType {
            id: 0,
            layer_id: 0,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(children.clone()),
            ..Default::default()
        },
    );
    for id in 1..=6 {
        set_iface(
            &mut c,
            id as usize,
            IfType {
                id,
                layer_id: 0,
                r#type: ComponentType::TYPE_GRAPHIC,
                ..Default::default()
            },
        );
        set_iface_mut(
            &mut c,
            id as usize,
            IfTypeMut {
                button_type: ButtonType::BUTTON_SELECT,
                ..Default::default()
            },
        );
    }
    set_iface(
        &mut c,
        7,
        IfType {
            id: 7,
            layer_id: 0,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        7,
        IfTypeMut {
            text: "Player controls".into(),
            ..Default::default()
        },
    );

    set_iface(
        &mut c,
        8,
        IfType {
            id: 8,
            layer_id: 0,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        8,
        IfTypeMut {
            text: "Auto retaliate".into(),
            ..Default::default()
        },
    );

    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::IF_SETTEXT);
    assert!(snap.rebuild_family(&mut c, Family::Controls));

    let run = snap.run_controls().expect("run toggle pair");
    assert_eq!(run.on_component_id, 6, "children[5] toggles run on");
    assert_eq!(run.off_component_id, 5, "children[4] toggles run off");
    let retaliate = snap.retaliate_controls().expect("retaliate toggle pair");
    assert_eq!(
        retaliate.on_component_id, 3,
        "children[2] toggles retaliate on"
    );
    assert_eq!(
        retaliate.off_component_id, 4,
        "children[3] toggles retaliate off"
    );

    assert!(
        !snap.rebuild_family(&mut c, Family::Controls),
        "unchanged gens"
    );
}

/// Modals read the open modal ids + the count-dialog/active-tab scalars
/// (always fresh, like the world family); the menu family reads the
/// minimenu entries and the login message.
#[test]
fn modals_menu_and_modal_texts_read_client_state() {
    let mut c = client_with_npc();
    set_iface(
        &mut c,
        600,
        IfType {
            id: 600,
            layer_id: 600,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![601, 602, 603]),
            ..Default::default()
        },
    );
    set_iface(
        &mut c,
        601,
        IfType {
            id: 601,
            layer_id: 600,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        601,
        IfTypeMut {
            text: "Line one".into(),
            ..Default::default()
        },
    );

    set_iface(
        &mut c,
        602,
        IfType {
            id: 602,
            layer_id: 600,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        602,
        IfTypeMut {
            text: "Line two".into(),
            ..Default::default()
        },
    );

    set_iface(
        &mut c,
        603,
        IfType {
            id: 603,
            layer_id: 600,
            r#type: ComponentType::TYPE_RECT,
            ..Default::default()
        },
    );
    c.main_modal_id = 600;
    c.dialog_input_open = true;
    c.active_icon = 3;
    c.login_mes1 = "Welcome".into();
    c.login_mes2 = "to RuneScape".into();
    c.menu_option = vec!["Cancel".into(), "Walk here".into(), "Attack Goblin".into()];
    c.menu_num_entries = 3;

    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::IF_OPENMAIN);
    assert!(snap.rebuild_family(&mut c, Family::Modals));
    assert!(snap.rebuild_family(&mut c, Family::Menu));

    let modals = snap.modals();
    assert_eq!(modals.main, 600);
    assert_eq!(modals.side, -1);
    assert_eq!(modals.chat, -1);
    assert_eq!(modals.tutorial, -1);
    assert!(snap.count_dialog_open());
    assert_eq!(snap.active_side_tab(), 3);
    assert_eq!(
        snap.main_modal_texts(),
        &["Line one".to_string(), "Line two".to_string()]
    );
    assert!(snap.chat_modal_texts().is_empty());
    assert_eq!(
        snap.menu_entries(),
        &[
            "Cancel".to_string(),
            "Walk here".to_string(),
            "Attack Goblin".to_string()
        ]
    );
    assert_eq!(snap.login_message(), "Welcome\nto RuneScape");

    // The scalars copy every rebuild like the world/camera families.
    c.dialog_input_open = false;
    c.active_icon = 2;
    c.menu_num_entries = 1;
    c.login_mes1 = String::new();
    assert!(
        !snap.rebuild_family(&mut c, Family::Modals),
        "no iface gen moved"
    );
    assert!(!snap.rebuild_family(&mut c, Family::Menu));
    assert!(!snap.count_dialog_open());
    assert_eq!(snap.active_side_tab(), 2);
    assert_eq!(snap.menu_entries(), &["Cancel".to_string()]);
    assert_eq!(snap.login_message(), "to RuneScape");
}

/// The new iface-derived families rebuild when the iface or inv gen moves
/// (item contents also track the inv gen), and stay put otherwise.
#[test]
fn iface_families_rebuild_on_iface_and_inv_gens() {
    let mut c = client_with_npc();
    let mut snap = GameSnapshot::new();

    for family in [
        Family::Inventory,
        Family::Equipment,
        Family::Bank,
        Family::BankSide,
        Family::Trade,
        Family::Widgets,
        Family::SideTabs,
        Family::ChatOptions,
        Family::MakeProducts,
        Family::QuestStatuses,
        Family::Modals,
        Family::Controls,
        Family::Menu,
    ] {
        assert!(
            !snap.rebuild_family(&mut c, family),
            "{family:?} starts clean"
        );
    }

    c.bump_gens(ServerProt::UPDATE_INV_FULL);
    for family in [
        Family::Inventory,
        Family::Equipment,
        Family::Bank,
        Family::BankSide,
        Family::Trade,
        Family::Widgets,
        Family::SideTabs,
    ] {
        assert!(
            snap.rebuild_family(&mut c, family),
            "{family:?} moves on inv"
        );
        assert!(
            !snap.rebuild_family(&mut c, family),
            "{family:?} then stays"
        );
    }

    c.bump_gens(ServerProt::IF_SETTEXT);
    for family in [
        Family::Inventory,
        Family::Trade,
        Family::Widgets,
        Family::ChatOptions,
        Family::MakeProducts,
        Family::QuestStatuses,
        Family::Modals,
        Family::Controls,
        Family::Menu,
    ] {
        assert!(
            snap.rebuild_family(&mut c, family),
            "{family:?} moves on iface"
        );
        assert!(
            !snap.rebuild_family(&mut c, family),
            "{family:?} then stays"
        );
    }
}

// ---- Task 5: full §3.1 assembly (tick/attached/self_slot/inventory_size/
// bank_component_id/varps) + ReadContext ----

/// The player family reads the local player's slot (`self_slot`, -1 before
/// any player rebuild) and counts each `PLAYER_INFO` (the game-tick edge,
/// the host's `should_emit_tick` rule) as one snapshot tick.
#[test]
fn player_rebuild_reads_self_slot_and_bumps_tick() {
    let mut c = client_with_npc();
    c.self_slot = 7;
    let mut snap = GameSnapshot::new();
    assert_eq!(snap.self_slot(), -1, "no player rebuild yet");
    assert_eq!(snap.tick(), 0);

    c.bump_gens(ServerProt::PLAYER_INFO);
    assert!(snap.rebuild_family(&mut c, Family::Player));
    assert_eq!(snap.self_slot(), 7);
    assert_eq!(snap.tick(), 1);

    c.bump_gens(ServerProt::PLAYER_INFO);
    assert!(snap.rebuild_family(&mut c, Family::Player));
    assert_eq!(snap.tick(), 2, "each PLAYER_INFO is one game tick");

    assert!(!snap.rebuild_family(&mut c, Family::Player));
    assert_eq!(snap.tick(), 2, "no player gen move: no tick");
}

/// The scene family reads `attached` from the socket (the §3.1 "socket
/// state" mapping): no stream → not attached; a connected `ClientStream`
/// → attached. The flag copies every rebuild like `ingame`/`scene_state`.
#[test]
fn scene_rebuild_reads_attached_from_the_socket() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let stream =
        client::io::ClientStream::connect(&addr.ip().to_string(), addr.port()).expect("connect");
    let mut c = client_with_npc();
    let mut snap = GameSnapshot::new();

    c.bump_gens(ServerProt::REBUILD_NORMAL);
    assert!(snap.rebuild_family(&mut c, Family::Scene));
    assert!(!snap.attached(), "no socket yet");

    c.stream = Some(stream);
    assert!(!snap.rebuild_family(&mut c, Family::Scene));
    assert!(
        snap.attached(),
        "a connected stream marks the slot attached"
    );
}

/// The varp family rebuilds the whole `client.var` table (one view per
/// definition; unset entries read 0), gated on the varp gen.
#[test]
fn varp_family_rebuild_reads_the_varp_table() {
    let mut c = client_with_npc();
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        cache.varps.clear();
        cache.varps.push(Default::default());
        cache.varps.push(Default::default());
    }
    c.var = vec![42, 0, 7]; // one beyond the definition count
    let mut snap = GameSnapshot::new();
    assert_eq!(snap.varps(), &[]);

    c.bump_gens(ServerProt::VARP_SMALL);
    assert!(snap.rebuild_family(&mut c, Family::Varp));
    assert_eq!(
        snap.varps(),
        &[
            VarpView {
                index: 0,
                value: 42
            },
            VarpView { index: 1, value: 0 }
        ],
        "one view per definition, unset values default to 0"
    );
    assert!(!snap.rebuild_family(&mut c, Family::Varp));
}

/// `inventory_size` is the inv tab component's slot count (0 until the
/// inv tab loads) and `bank_component_id` the open main modal's withdraw
/// component (-1 when no bank is open — never component 0).
#[test]
fn inventory_size_and_bank_component_id_derive_from_the_ifaces() {
    let mut c = client_with_npc();
    set_iface(
        &mut c,
        500,
        IfType {
            id: 500,
            r#type: ComponentType::TYPE_INV,
            obj_ops: true,
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        500,
        IfTypeMut {
            link_obj_type: Some(vec![4, 5, 0, 7]),
            link_obj_number: Some(vec![1, 100, 0, 2]),
            ..Default::default()
        },
    );

    c.side_icon[3] = 500;
    set_iface(
        &mut c,
        600,
        IfType {
            id: 600,
            layer_id: 600,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![601]),
            ..Default::default()
        },
    );
    set_iface(
        &mut c,
        601,
        IfType {
            id: 601,
            layer_id: 600,
            r#type: ComponentType::TYPE_INV,
            iop: [Some("Withdraw 1".into()), None, None, None, None],
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        601,
        IfTypeMut {
            link_obj_type: Some(vec![6, 0]),
            link_obj_number: Some(vec![2, 0]),
            ..Default::default()
        },
    );

    c.main_modal_id = 600;

    let mut snap = GameSnapshot::new();
    assert_eq!(snap.bank_component_id(), -1, "no bank open by default");
    c.bump_gens(ServerProt::UPDATE_INV_FULL);
    assert!(snap.rebuild_family(&mut c, Family::Inventory));
    assert!(snap.rebuild_family(&mut c, Family::Bank));
    assert_eq!(snap.inventory_size(), 4, "the inv component's slot count");
    assert_eq!(snap.bank_component_id(), 601);

    c.main_modal_id = -1;
    c.bump_gens(ServerProt::IF_OPENMAIN);
    assert!(snap.rebuild_family(&mut c, Family::Bank));
    assert_eq!(snap.bank_component_id(), -1, "no bank: -1, not component 0");
    assert!(snap.bank().is_empty());
}

/// `ReadContext` reads every §3.2 accessor from a rebuilt snapshot without
/// panicking and returns the planted values.
#[test]
fn read_context_round_trips_every_family() {
    let mut c = client_with_npc();
    // world / camera / scene scalars
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c.minusedlevel = 1;
    c.ingame = true;
    c.scene_state = 2;
    c.self_slot = 7;
    c.runenergy = 63;
    c.runweight = 24;
    c.members_account = 1;
    c.in_multizone = 1;
    c.loop_cycle = 412;
    c.cam_x = 1;
    c.cam_y = 2;
    c.cam_z = 3;
    c.cam_pitch = 4;
    c.cam_yaw = 5;
    c.orbit_camera_pitch = 6;
    c.orbit_camera_yaw = 7;
    c.cinema_cam = true;
    c.minimap_flag_x = 14;
    c.minimap_flag_z = 15;
    // player family: a local player with a decode-ready body
    let mut local = ClientPlayer::at(20, 12);
    local.entity.x = 20 * 128 + 64;
    local.entity.z = 12 * 128 + 64;
    local.name = Some("Zezima".into());
    c.local_player = Some(local);
    let mut other = ClientPlayer::at(15, 16);
    other.entity.x = 100;
    other.entity.z = 150;
    other.name = Some("Other".into());
    other.combat_level = 3;
    other.skill_level = 5;
    c.players[3] = Some(Box::new(other));
    c.player_ids[0] = 3;
    c.player_count = 1;
    // varps
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        cache.varps.clear();
        cache.varps.push(Default::default());
        cache.varps.push(Default::default());
    }
    c.var = vec![42, 0];
    // a loc (straight wall, angle 1) and a ground-item stack
    let typecode = 0x4000_0000 + (1 << 14) + 3 + (4 << 7);
    c.world
        .set_wall(1, 3, 4, 0, 0, 0, typecode, 1 << 6, 0, 0, 0, 0);
    plant_obj(&mut c, 3, "Coins");
    plant_obj(&mut c, 4, "Sword");
    let bones_id = {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        let id = cache.objs.len() as i32;
        cache.objs.push(ObjType {
            id,
            name: "Bones".into(),
            ..Default::default()
        });
        id
    };
    let mut list = LinkList::new();
    list.push(ClientObj::new(bones_id, 2));
    c.ground_obj[1][10][12] = Some(Box::new(list));
    // inventory (side tab 3) + equipment (side tab 4)
    set_iface(
        &mut c,
        500,
        IfType {
            id: 500,
            r#type: ComponentType::TYPE_INV,
            obj_ops: true,
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        500,
        IfTypeMut {
            link_obj_type: Some(vec![4, 5, 0]),
            link_obj_number: Some(vec![1, 100, 0]),
            ..Default::default()
        },
    );

    c.side_icon[3] = 500;
    set_iface(
        &mut c,
        710,
        IfType {
            id: 710,
            layer_id: 710,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![711]),
            ..Default::default()
        },
    );
    set_iface(
        &mut c,
        711,
        IfType {
            id: 711,
            layer_id: 710,
            r#type: ComponentType::TYPE_INV,
            iop: [Some("Remove".into()), None, None, None, None],
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        711,
        IfTypeMut {
            link_obj_type: Some(vec![5, 0]),
            link_obj_number: Some(vec![1, 0]),
            ..Default::default()
        },
    );

    c.side_icon[4] = 710;
    // bank (main modal) + bank-side (side modal), with a text row
    set_iface(
        &mut c,
        600,
        IfType {
            id: 600,
            layer_id: 600,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![601, 602]),
            ..Default::default()
        },
    );
    set_iface(
        &mut c,
        601,
        IfType {
            id: 601,
            layer_id: 600,
            r#type: ComponentType::TYPE_INV,
            iop: [Some("Withdraw 1".into()), None, None, None, None],
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        601,
        IfTypeMut {
            link_obj_type: Some(vec![6, 0]),
            link_obj_number: Some(vec![2, 0]),
            ..Default::default()
        },
    );

    set_iface(
        &mut c,
        602,
        IfType {
            id: 602,
            layer_id: 600,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        602,
        IfTypeMut {
            text: "Bank line".into(),
            ..Default::default()
        },
    );

    c.main_modal_id = 600;
    set_iface(
        &mut c,
        700,
        IfType {
            id: 700,
            layer_id: 700,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![701]),
            ..Default::default()
        },
    );
    set_iface(
        &mut c,
        701,
        IfType {
            id: 701,
            layer_id: 700,
            r#type: ComponentType::TYPE_INV,
            iop: [Some("Deposit All".into()), None, None, None, None],
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        701,
        IfTypeMut {
            link_obj_type: Some(vec![7, 0]),
            link_obj_number: Some(vec![7, 0]),
            ..Default::default()
        },
    );

    c.side_modal_id = 700;
    // trade containers (the packed 274 ids)
    set_iface(
        &mut c,
        3415,
        IfType {
            id: 3415,
            r#type: ComponentType::TYPE_INV,
            iop: [Some("Remove 1".into()), None, None, None, None],
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        3415,
        IfTypeMut {
            link_obj_type: Some(vec![4, 0]),
            link_obj_number: Some(vec![1, 0]),
            ..Default::default()
        },
    );

    set_iface(
        &mut c,
        3416,
        IfType {
            id: 3416,
            r#type: ComponentType::TYPE_INV,
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        3416,
        IfTypeMut {
            link_obj_type: Some(vec![6, 0]),
            link_obj_number: Some(vec![3, 0]),
            ..Default::default()
        },
    );

    set_iface(
        &mut c,
        3417,
        IfType {
            id: 3417,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        3417,
        IfTypeMut {
            text: "Trading With: Zezima".into(),
            ..Default::default()
        },
    );

    set_iface(
        &mut c,
        3322,
        IfType {
            id: 3322,
            r#type: ComponentType::TYPE_INV,
            iop: [Some("Offer".into()), None, None, None, None],
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        3322,
        IfTypeMut {
            link_obj_type: Some(vec![7, 0]),
            link_obj_number: Some(vec![2, 0]),
            ..Default::default()
        },
    );

    // chat modal: option, continue, a make product and its button
    set_iface(
        &mut c,
        2000,
        IfType {
            id: 2000,
            layer_id: 2000,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![2001, 2002, 2003, 2004]),
            ..Default::default()
        },
    );
    set_iface(
        &mut c,
        2001,
        IfType {
            id: 2001,
            layer_id: 2000,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        2001,
        IfTypeMut {
            button_type: ButtonType::BUTTON_OK,
            text: "Yes".into(),
            ..Default::default()
        },
    );

    set_iface(
        &mut c,
        2002,
        IfType {
            id: 2002,
            layer_id: 2000,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        2002,
        IfTypeMut {
            button_type: ButtonType::BUTTON_CONTINUE,
            ..Default::default()
        },
    );

    set_iface(
        &mut c,
        2003,
        IfType {
            id: 2003,
            layer_id: 2000,
            r#type: ComponentType::TYPE_MODEL,
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        2003,
        IfTypeMut {
            model1_type: 4,
            model1_id: bones_id,
            ..Default::default()
        },
    );

    set_iface(
        &mut c,
        2004,
        IfType {
            id: 2004,
            layer_id: 2000,
            r#type: ComponentType::TYPE_RECT,
            button_text: "Make X".into(),
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        2004,
        IfTypeMut {
            button_type: ButtonType::BUTTON_OK,
            ..Default::default()
        },
    );

    c.chat_modal_id = 2000;
    // chat ring + quest tab (side tab 2) + controls overlay (root 0)
    c.chat_text[0] = "hello".into();
    c.chat_type[0] = 0;
    set_iface(
        &mut c,
        2200,
        IfType {
            id: 2200,
            layer_id: 2200,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![2201]),
            ..Default::default()
        },
    );
    set_iface(
        &mut c,
        2201,
        IfType {
            id: 2201,
            layer_id: 2200,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        2201,
        IfTypeMut {
            text: "Cook's Assistant".into(),
            colour: 0x00ff00,
            ..Default::default()
        },
    );

    c.side_icon[2] = 2200;
    set_iface(
        &mut c,
        0,
        IfType {
            id: 0,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![1, 2, 3, 4, 5, 6]),
            ..Default::default()
        },
    );
    for id in 1..=6 {
        set_iface(
            &mut c,
            id as usize,
            IfType {
                id,
                r#type: ComponentType::TYPE_GRAPHIC,
                ..Default::default()
            },
        );
        set_iface_mut(
            &mut c,
            id as usize,
            IfTypeMut {
                button_type: ButtonType::BUTTON_SELECT,
                ..Default::default()
            },
        );
    }
    set_iface(
        &mut c,
        1,
        IfType {
            id: 1,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        1,
        IfTypeMut {
            text: "Player controls".into(),
            ..Default::default()
        },
    );

    set_iface(
        &mut c,
        2,
        IfType {
            id: 2,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        2,
        IfTypeMut {
            text: "Auto retaliate".into(),
            ..Default::default()
        },
    );

    // modals / menu scalars
    c.dialog_input_open = true;
    c.active_icon = 3;
    c.login_mes1 = "Welcome".into();
    c.menu_option = vec!["Walk here".into()];
    c.menu_num_entries = 1;

    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    assert!(snap.rebuild(&mut c));
    let ctx = ReadContext::new(&snap);

    // scalars + player family
    assert_eq!(ctx.tick(), 1, "one player-info in this run");
    assert!(!ctx.attached(), "no socket in this fixture");
    assert!(ctx.ingame());
    assert_eq!(ctx.scene_state(), 2);
    assert_eq!(ctx.self_slot(), 7);
    assert_eq!(ctx.local_player().map(|p| p.player.index), Some(7));
    assert_eq!(ctx.stats().len(), 25);
    assert_eq!(ctx.npcs().len(), 1);
    assert_eq!(ctx.npcs()[0].index, 7);
    assert_eq!(ctx.players().len(), 1);
    assert_eq!(ctx.players()[0].index, 3);
    assert_eq!(ctx.locs().len(), 1);
    assert_eq!(ctx.locs()[0].tile.x, 3203);
    assert_eq!(ctx.ground_items().len(), 1);
    assert_eq!(ctx.ground_items()[0].def.id, bones_id);
    // item containers
    assert_eq!(ctx.inventory().len(), 2);
    assert_eq!(ctx.inventory()[0].def.id, 3);
    assert_eq!(ctx.equipment().len(), 1);
    assert_eq!(ctx.equipment()[0].def.id, 4);
    assert_eq!(ctx.inventory_capacity(), 3);
    assert_eq!(ctx.bank().len(), 1);
    assert_eq!(ctx.bank()[0].def.id, 5);
    assert_eq!(ctx.bank_side_items().len(), 1);
    assert_eq!(ctx.bank_side_items()[0].def.id, 6);
    assert_eq!(ctx.bank_component_id(), 601);
    // chat + make + quests
    assert_eq!(ctx.chat().len(), 1);
    assert_eq!(ctx.chat()[0].text, "hello");
    assert_eq!(ctx.chat_options().len(), 2);
    assert_eq!(ctx.chat_options()[0].component_id, 2001);
    assert_eq!(ctx.chat_continue_component_id(), 2002);
    assert_eq!(ctx.make_products().len(), 1);
    assert_eq!(ctx.make_products()[0].object_id, bones_id);
    assert_eq!(ctx.make_products()[0].buttons.len(), 1);
    assert_eq!(ctx.make_products()[0].buttons[0].quantity, -1);
    assert_eq!(ctx.quest_statuses().len(), 1);
    assert_eq!(ctx.quest_statuses()[0].name, "Cook's Assistant");
    // widgets + side tabs + components
    assert_eq!(ctx.widgets().len(), 10, "main/side/chat root trees");
    assert_eq!(ctx.side_tabs().len(), 14);
    assert_eq!(ctx.component(601).map(|w| w.component_id), Some(601));
    assert!(ctx.component(999_999).is_none());
    assert_eq!(ctx.component_items(601).len(), 1);
    assert_eq!(ctx.component_items(601)[0].def.id, 5);
    assert_eq!(ctx.component_text(2001), Some("Yes"));
    assert_eq!(ctx.component_text(601), None);
    assert_eq!(ctx.component_model_obj_id(2003), Some(bones_id));
    assert_eq!(ctx.component_model_obj_id(601), None, "model type != 4");
    assert_eq!(ctx.side_tab_interface(2), 2200);
    assert_eq!(ctx.side_tab_interface(11), -1, "unbound tab");
    // varps + world + scene + camera
    assert_eq!(ctx.varps().len(), 2);
    assert_eq!(ctx.varps()[0].value, 42);
    assert_eq!(ctx.varp(1), 0);
    assert_eq!(ctx.varp(99), 0, "unknown varps read 0");
    assert_eq!(ctx.world().map_base_x, 3200);
    assert_eq!(ctx.world().cycle, 412);
    assert!(ctx.scene().available);
    assert_eq!(ctx.scene().collision_flags.len(), 104 * 104);
    assert_eq!(ctx.camera().x, 1);
    assert!(ctx.camera().cinematic);
    // trade containers
    assert_eq!(ctx.trade_my_offer().len(), 1);
    assert_eq!(ctx.trade_my_offer()[0].def.id, 3);
    assert_eq!(ctx.trade_their_offer().len(), 1);
    assert_eq!(ctx.trade_their_offer()[0].def.id, 5);
    assert_eq!(ctx.trade_side_pack().len(), 1);
    assert_eq!(ctx.trade_side_pack()[0].def.id, 6);
    // modals + menu
    assert_eq!(ctx.modals().main, 600);
    assert!(ctx.count_dialog_open());
    assert_eq!(ctx.active_side_tab(), 3);
    assert_eq!(ctx.login_message(), "Welcome");
    assert_eq!(ctx.menu_entries(), &["Walk here".to_string()]);
    assert_eq!(ctx.main_modal_texts(), &["Bank line".to_string()]);
    assert_eq!(ctx.chat_modal_texts(), &["Yes".to_string()]);
    // controls
    let run = ctx.run_controls().expect("run toggle pair");
    assert_eq!((run.on_component_id, run.off_component_id), (6, 5));
    let retaliate = ctx.retaliate_controls().expect("retaliate toggle pair");
    assert_eq!(
        (retaliate.on_component_id, retaliate.off_component_id),
        (3, 4)
    );
    // world tile comes from the local player view (real scene level)
    assert_eq!(
        ctx.world_tile(),
        Some(WorldTile {
            x: 3220,
            z: 3212,
            level: 1
        })
    );
}
