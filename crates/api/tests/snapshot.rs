// Task 8: snapshot families rebuild only the family whose gen moved.
// `Client::new` with a cache-less `/tmp` dir falls back to `Cache::default()`
// and never touches the network (same trick as `client/tests/gens.rs`).

use api::query::{npc_by_index, npcs_at};
use api::snapshot::{ActorKind, ActorTargetView, Family, GameSnapshot, LocLayer, WorldTile};
use client::client::{Client, ClientConfig, ClientNpc};
use client::config::if_type::{ComponentType, IfType};
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
    c.npc[7] = Some(npc);
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
    c.npc[3] = Some(ClientNpc::default());
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

/// Inv-family rebuild: zip the TYPE_INV iface's obj ids/counts.
#[test]
fn inv_rebuild_reads_the_type_inv_iface() {
    let mut c = client_with_npc();
    match c.ifaces.iter_mut().flatten().find(|f| f.r#type == ComponentType::TYPE_INV) {
        Some(inv) => {
            inv.link_obj_type = Some(vec![526, 995]);
            inv.link_obj_number = Some(vec![1, 100]);
        }
        None => c.ifaces.push(Some(IfType {
            r#type: ComponentType::TYPE_INV,
            link_obj_type: Some(vec![526, 995]),
            link_obj_number: Some(vec![1, 100]),
            ..Default::default()
        })),
    }
    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::UPDATE_INV_FULL);
    assert!(snap.rebuild_family(&mut c, Family::Inv));
    assert_eq!(snap.inv(), &[(526, 1), (995, 100)]);
    assert_eq!(snap.inv_count(526), 1);
    assert_eq!(snap.inv_count(995), 100);
    assert_eq!(snap.inv_count(0), 0);
    assert!(!snap.rebuild_family(&mut c, Family::Inv));
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
    assert_eq!(v.tile, WorldTile { x: 3200, z: 3201, level: 1 });
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
        Some(ActorTargetView { kind: ActorKind::Npc, index: 3 })
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
    c.npc[3] = Some(other);
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
        Some(ActorTargetView { kind: ActorKind::Npc, index: 3 })
    );
    assert_eq!(
        npc_by_index(snap.npcs(), 3).unwrap().target,
        Some(ActorTargetView { kind: ActorKind::Player, index: 7 })
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
    c.players[3] = Some(other);
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
        WorldTile { x: 3220, z: 3212, level: 1 }
    );
    assert_eq!(
        (local_view.player.actor.tile.x, local_view.player.actor.tile.z),
        (snap.tile().unwrap().0, snap.tile().unwrap().1),
        "the local actor tile matches the snapshot's world tile"
    );
    assert_eq!(
        local_view.player.actor.target,
        Some(ActorTargetView { kind: ActorKind::Npc, index: 3 })
    );
    assert_eq!(local_view.player.actor.animation, 808);
    assert_eq!(local_view.player.combat_level, 126);
    assert_eq!(local_view.player.skill_level, 99);
    assert_eq!(local_view.energy, 63);
    assert_eq!(local_view.weight, 24);
    // actions come from the client's shared player menu ops table
    assert_eq!(
        local_view.player.actor.actions,
        vec![Some("Attack".into()), None, Some("Trade with".into()), None, None]
    );

    let players = snap.players();
    assert_eq!(players.len(), 1);
    let p = &players[0];
    assert_eq!(p.index, 3);
    assert_eq!(p.actor.name.as_deref(), Some("Other"));
    // entity pixel coords (100, 150), size 1 → world tile (3200, 3200)
    assert_eq!(p.actor.tile, WorldTile { x: 3200, z: 3200, level: 1 });
    assert_eq!(p.actor.distance, 20); // chebyshev from (3220, 3212)
    assert_eq!(
        p.actor.target,
        Some(ActorTargetView { kind: ActorKind::Player, index: 7 })
    );
    assert_eq!(p.combat_level, 3);
    assert_eq!(p.skill_level, 5);
    assert_eq!(snap.tile(), Some((3220, 3212, 0)));

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
    assert_eq!(v.tile, WorldTile { x: 3203, z: 3204, level: 0 });
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
    assert!(!snap.rebuild_family(&mut c, Family::Loc), "unchanged scene gen");
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
    assert_eq!(wall_v.tile, WorldTile { x: 3203, z: 3204, level: 0 });
    let decor_v = locs
        .iter()
        .find(|v| v.layer == LocLayer::WallDecoration)
        .unwrap();
    assert_eq!(decor_v.id, 2);
    assert_eq!(decor_v.shape, 4);
    assert_eq!(decor_v.angle, 2);
    assert_eq!(decor_v.tile, WorldTile { x: 3205, z: 3206, level: 0 });
    let scene_v = locs.iter().find(|v| v.layer == LocLayer::Ground).unwrap();
    assert_eq!(scene_v.id, 3);
    assert_eq!(scene_v.shape, 10);
    assert_eq!(scene_v.angle, 3);
    assert_eq!(scene_v.tile, WorldTile { x: 3207, z: 3208, level: 0 });
    let gd_v = locs
        .iter()
        .find(|v| v.layer == LocLayer::GroundDecoration)
        .unwrap();
    assert_eq!(gd_v.id, 4);
    assert_eq!(gd_v.shape, 22);
    assert_eq!(gd_v.angle, 3);
    assert_eq!(gd_v.tile, WorldTile { x: 3209, z: 3210, level: 0 });
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
    c.ground_obj[0][10][12] = Some(list);

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
    assert_eq!(items[0].tile, WorldTile { x: 3210, z: 3212, level: 0 });
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
    assert!(!snap.rebuild_family(&mut c, Family::MapFlag), "no gen moved");
    assert_eq!(snap.map_flag(), None, "the flag view is always fresh");
}
