// Task 8: snapshot families rebuild only the family whose gen moved.
// `Client::new` with a cache-less `/tmp` dir falls back to `Cache::default()`
// and never touches the network (same trick as `client/tests/gens.rs`).

use api::query::{npc_by_index, npcs_at};
use api::snapshot::{ActorKind, ActorTargetView, Family, GameSnapshot, WorldTile};
use client::client::{Client, ClientConfig, ClientNpc};
use client::config::if_type::{ComponentType, IfType};
use client::config::NpcType;
use client::dash3d::ClientPlayer;
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
    let c = client_with_npc();
    let mut snap = GameSnapshot::new();
    assert!(!snap.rebuild_family(&c, Family::Npc));

    let ptr = snap.npcs().as_ptr();
    assert!(!snap.rebuild_family(&c, Family::Npc));
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
    assert!(snap.rebuild_family(&c, Family::Npc));
    assert_eq!(snap.npcs().len(), 1);
    assert_eq!(snap.npcs()[0].index, 7);
    assert_eq!(snap.npcs()[0].x, 100);

    let ptr = snap.npcs().as_ptr();
    assert!(!snap.rebuild_family(&c, Family::Npc));
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
    snap.rebuild_family(&c, Family::Npc);

    let old_ptr = snap.npcs().as_ptr();
    c.npc[7].as_mut().unwrap().entity.x = 999;
    c.npc[7].as_mut().unwrap().entity.yaw = 42;

    assert!(!snap.rebuild_family(&c, Family::Npc));
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
    snap.rebuild_family(&c, Family::Npc);

    c.bump_gens(ServerProt::NPC_INFO);
    c.npc[7].as_mut().unwrap().entity.x = 555;
    assert!(snap.rebuild_family(&c, Family::Npc));
    assert_eq!(snap.npcs()[0].x, 555);
}

/// An inv bump must not rebuild the npc family.
#[test]
fn other_family_bump_leaves_npc_rebuild_untouched() {
    let mut c = client_with_npc();
    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::NPC_INFO);
    snap.rebuild_family(&c, Family::Npc);

    let ptr = snap.npcs().as_ptr();
    c.bump_gens(ServerProt::UPDATE_INV_FULL);
    assert!(!snap.rebuild_family(&c, Family::Npc));
    assert_eq!(snap.npcs().as_ptr(), ptr);
    assert_eq!(snap.npcs().len(), 1);
}

/// `GameSnapshot` records the world generations it has been rebuilt up to.
#[test]
fn snapshot_tracks_family_generations() {
    let mut c = client_with_npc();
    let mut snap = GameSnapshot::new();

    assert!(!snap.rebuild_family(&c, Family::Inv));
    assert_eq!(snap.gens().inv, 0);

    c.bump_gens(ServerProt::VARP_SMALL);
    assert!(snap.rebuild_family(&c, Family::Varp));
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
    snap.rebuild_family(&c, Family::Npc);
    assert!(snap.npcs().is_empty());
}

/// Stat rebuild copies run energy for auto-run's snapshot view.
#[test]
fn stat_rebuild_copies_runenergy() {
    let mut c = client_with_npc();
    c.runenergy = 20;
    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::UPDATE_RUNENERGY);
    assert!(snap.rebuild_family(&c, Family::Stat));
    assert_eq!(snap.runenergy(), 20);
    assert!(!snap.rebuild_family(&c, Family::Stat));
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
    snap.rebuild_family(&c, Family::Npc);

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
    assert!(snap.rebuild_family(&c, Family::Player));
    assert_eq!(snap.base(), Some((3200, 3200)));
    assert_eq!(snap.tile(), None, "no local player decoded yet");

    c.local_player = Some(ClientPlayer::at(20, 12));
    c.bump_gens(ServerProt::PLAYER_INFO);
    assert!(snap.rebuild_family(&c, Family::Player));
    assert_eq!(snap.tile(), Some((3220, 3212, 0)));

    assert!(!snap.rebuild_family(&c, Family::Player));
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
    assert!(snap.rebuild_family(&c, Family::Inv));
    assert_eq!(snap.inv(), &[(526, 1), (995, 100)]);
    assert_eq!(snap.inv_count(526), 1);
    assert_eq!(snap.inv_count(995), 100);
    assert_eq!(snap.inv_count(0), 0);
    assert!(!snap.rebuild_family(&c, Family::Inv));
}

/// Chat-family rebuild: the ring head (`chat_text[0]`) is the latest line.
#[test]
fn chat_rebuild_reads_the_ring_head() {
    let mut c = client_with_npc();
    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::MESSAGE_GAME);
    assert!(snap.rebuild_family(&c, Family::Chat));
    assert_eq!(snap.chat(), None, "empty ring head reads as none");

    c.chat_text[0] = "Welcome to RuneScape".into();
    c.bump_gens(ServerProt::MESSAGE_GAME);
    assert!(snap.rebuild_family(&c, Family::Chat));
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
    assert!(snap.rebuild_family(&c, Family::Scene));
    assert!(snap.ingame());
    assert_eq!(snap.scene_state(), 2);
    assert!(!snap.rebuild_family(&c, Family::Scene));
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
    }
}

/// `Family` covers all 11 `ClientGens` counters, and the four new
/// families track their gen (no view yet) so a later view can detect
/// movement.
#[test]
fn family_enum_covers_all_client_gens() {
    assert_eq!(
        family_index(Family::World),
        11,
        "Family must mirror ClientGens' 11 counters"
    );

    let mut c = client_with_npc();
    let mut snap = GameSnapshot::new();

    assert!(!snap.rebuild_family(&c, Family::Iface));
    assert!(!snap.rebuild_family(&c, Family::Camera));
    assert!(!snap.rebuild_family(&c, Family::MapFlag));
    assert!(!snap.rebuild_family(&c, Family::World));
    assert_eq!(snap.gens().iface, 0);
    assert_eq!(snap.gens().camera, 0);
    assert_eq!(snap.gens().map_flag, 0);
    assert_eq!(snap.gens().world, 0);

    c.bump_gens(ServerProt::IF_SETPOSITION);
    c.bump_gens(ServerProt::CAM_LOOKAT);
    c.bump_gens(ServerProt::UNSET_MAP_FLAG);
    c.bump_gens(ServerProt::SET_MULTIWAY);

    assert!(snap.rebuild_family(&c, Family::Iface));
    assert!(snap.rebuild_family(&c, Family::Camera));
    assert!(snap.rebuild_family(&c, Family::MapFlag));
    assert!(snap.rebuild_family(&c, Family::World));
    assert_eq!(snap.gens().iface, 1);
    assert_eq!(snap.gens().camera, 1);
    assert_eq!(snap.gens().map_flag, 1);
    assert_eq!(snap.gens().world, 1);

    assert!(!snap.rebuild_family(&c, Family::Iface));
    assert!(!snap.rebuild_family(&c, Family::Camera));
    assert!(!snap.rebuild_family(&c, Family::MapFlag));
    assert!(!snap.rebuild_family(&c, Family::World));
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
    assert!(snap.rebuild(&c));
    assert_eq!(snap.tile(), Some((3220, 3212, 0)));
    assert_eq!(snap.runenergy(), 20);
    assert!(!snap.rebuild(&c), "unchanged gens are not dirty again");
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
    snap.rebuild(&c);
    assert_eq!(snap.scene_state(), 1);

    // The scene completes with no packet behind it: no gen moves, but
    // the snapshot must read the live state.
    c.scene_state = 2;
    assert!(!snap.rebuild(&c), "no gen moved");
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
    assert!(snap.rebuild_family(&c, Family::Stat));

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
    assert!(!snap.rebuild_family(&c, Family::Stat));
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
    assert!(snap.rebuild_family(&c, Family::Npc));

    let v = &snap.npcs()[0];
    assert_eq!(v.index, 7);
    assert_eq!(v.r#type, planted_type);
    assert_eq!(v.name.as_deref(), Some("Goblin"));
    assert_eq!(v.actions, vec![Some("Attack".into()), None]);
    assert_eq!(v.tile, WorldTile { x: 100, z: 200, level: 1 });
    // chebyshev from the local player's world tile (3220, 3212)
    assert_eq!(v.distance, 3120);
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
    snap.rebuild_family(&c, Family::Npc);

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
    local.entity.x = 100;
    local.entity.z = 200;
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
    assert!(snap.rebuild_family(&c, Family::Player));

    let local_view = snap.local_player().expect("local player view");
    assert_eq!(local_view.player.index, 7);
    assert_eq!(local_view.player.actor.name.as_deref(), Some("Zezima"));
    assert_eq!(local_view.player.actor.distance, 0);
    assert_eq!(
        local_view.player.actor.tile,
        WorldTile { x: 100, z: 200, level: 1 }
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
    assert_eq!(p.actor.distance, 3120); // chebyshev from (3220, 3212)
    assert_eq!(
        p.actor.target,
        Some(ActorTargetView { kind: ActorKind::Player, index: 7 })
    );
    assert_eq!(p.combat_level, 3);
    assert_eq!(p.skill_level, 5);
    assert_eq!(snap.tile(), Some((3220, 3212, 0)));

    assert!(!snap.rebuild_family(&c, Family::Player));
}
