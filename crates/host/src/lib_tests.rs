use super::*;
use api::interact::RUN_ORB_IFACE;
use client::dash3d::TerrainOverlayShape;
use client::io::{ClientProt, ClientProt289};
use std::sync::OnceLock;
use std::time::Instant;

/// Force the wgpu backend's process-wide init to fail so every
/// renderer this test process constructs lands on `CpuBackend` (the
/// client's documented `R274_TEST_FORCE_NO_GPU` test hook): the host's
/// GPU-first default must not open a real device inside
/// `cargo test -p host`. Once set, the client caches the failure, so
/// the first renderer construction wins — call before any test
/// constructs one.
static FORCE_CPU_BACKEND: OnceLock<()> = OnceLock::new();
fn force_cpu_backend() {
    FORCE_CPU_BACKEND.get_or_init(|| {
        std::env::set_var("R274_TEST_FORCE_NO_GPU", "1");
        Renderer::set_prefer_gpu(true);
    });
}

fn cfg() -> ClientConfig {
    ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp".into(),
        members: true,
        lowmem: true,
    }
}

#[test]
fn prepare_client_shares_arc_and_clears_error_loading() {
    let cache = Arc::new(Cache::default());
    let a = prepare_client(cfg(), 1, Arc::clone(&cache), Arc::new(vec![]), Vec::new());
    let b = prepare_client(cfg(), 2, Arc::clone(&cache), Arc::new(vec![]), Vec::new());
    assert!(Arc::ptr_eq(&a.cache, &b.cache));
    assert!(Arc::ptr_eq(&a.cache, &cache));
    assert!(!a.error_loading);
    assert!(!b.error_loading);
    assert_eq!(a.login_uid, 1);
    assert_eq!(b.login_uid, 2);
}

#[test]
fn slot_rebuilds_from_drain_dirty_not_post_drain_pump_dirty() {
    let mut client = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let mut slot = SlotLoop::new();
    client.gens.npc = 1;
    let result = slot.after_drain(&mut client);
    assert!(result.dirty.npc);
    assert_eq!(slot.snapshot.gens().npc, 1);
    assert_eq!(slot.pump.dirty(client.gens), DirtyFamilies::default());
}

/// The drain's dirty flags must feed `publish_snapshot` for the four new
/// families too, or `snapshot.gens().{iface,camera,map_flag,world}`
/// stay permanently 0 (the snapshot views rely on this path).
#[test]
fn drain_rebuilds_the_four_new_families() {
    let mut client = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let mut slot = SlotLoop::new();

    client.gens.iface = 1;
    let result = slot.after_drain(&mut client);
    assert!(result.dirty.iface);
    assert_eq!(
        slot.snapshot.gens().iface,
        1,
        "drain must rebuild the iface family"
    );

    client.gens.camera = 1;
    let result = slot.after_drain(&mut client);
    assert!(result.dirty.camera);
    assert_eq!(
        slot.snapshot.gens().camera,
        1,
        "drain must rebuild the camera family"
    );

    client.gens.map_flag = 1;
    let result = slot.after_drain(&mut client);
    assert!(result.dirty.map_flag);
    assert_eq!(
        slot.snapshot.gens().map_flag,
        1,
        "drain must rebuild the map_flag family"
    );

    client.gens.world = 1;
    let result = slot.after_drain(&mut client);
    assert!(result.dirty.world);
    assert_eq!(
        slot.snapshot.gens().world,
        1,
        "drain must rebuild the world family"
    );
}

/// The drain's iface/inv flags must rebuild the iface-derived
/// snapshot families too, or the host path (as opposed to the scenario
/// runner's `GameSnapshot::rebuild`) would keep their views
/// permanently empty.
#[test]
fn drain_rebuilds_the_iface_derived_families() {
    use client::config::if_type::ComponentType;

    let mut ifaces = vec![None; 1000];
    ifaces[500] = Some(Box::new(IfType {
        id: 500,
        r#type: ComponentType::TYPE_INV,
        obj_ops: true,
        ..IfType::default()
    }));
    let mut ifaces_mut = vec![None; 1000];
    ifaces_mut[500] = Some(Arc::new(IfTypeMut {
        link_obj_type: Some(vec![4, 5, 0]),
        link_obj_number: Some(vec![1, 100, 0]),
        ..IfTypeMut::default()
    }));
    let mut client = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(ifaces),
        ifaces_mut,
    );
    client.side_icon[3] = 500;
    client.ingame = true;
    let mut slot = SlotLoop::new();

    client.gens.iface = 1;
    client.gens.inv = 1;
    let result = slot.after_drain(&mut client);
    assert!(result.dirty.iface);
    assert!(result.dirty.inv);
    assert_eq!(
        slot.snapshot.inventory().len(),
        2,
        "an iface/inv drain must rebuild the inventory family"
    );
    assert_eq!(slot.snapshot.inventory_size(), 3);

    // A quiet drain leaves the snapshot gens alone: unchanged gens do not
    // re-mark anything dirty.
    let result = slot.after_drain(&mut client);
    assert!(!result.dirty.any());
}

#[test]
fn logout_drain_resets_snapshot_instead_of_republishing_retained_client_state() {
    use client::client::{ClientNpc, ClientPlayer};

    let mut client = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    client.ingame = true;
    client.scene_state = 2;
    client.local_player = Some(ClientPlayer::at(5, 6));
    client.npc[7] = Some(Box::new(ClientNpc::default()));
    client.npc_ids[0] = 7;
    client.npc_count = 1;
    client.bump_gens(client::io::ServerProt::REBUILD_NORMAL);

    let mut slot = SlotLoop::new();
    slot.after_drain(&mut client);
    assert_eq!(slot.snapshot.npcs().len(), 1);
    assert!(slot.snapshot.local_player().is_some());
    assert!(slot.snapshot.ingame());

    client.logout();
    let reset_gens = client.gens;
    assert!(
        client.npc[7].is_some(),
        "logout retains the client actor table"
    );
    let result = slot.after_drain(&mut client);

    assert!(result.dirty.npc && result.dirty.player && result.dirty.inv);
    assert_eq!(slot.snapshot.gens().npc, reset_gens.npc);
    assert_eq!(slot.snapshot.gens().player, reset_gens.player);
    assert!(slot.snapshot.npcs().is_empty());
    assert!(slot.snapshot.local_player().is_none());
    assert!(slot.snapshot.players().is_empty());
    assert!(slot.snapshot.inv().is_empty());
    assert_eq!(slot.snapshot.tick(), 0);
    assert!(!slot.snapshot.ingame());
    assert!(!slot.snapshot.attached());

    // A reconnect grant alone does not republish retained tables. Only a
    // packet beyond the reset watermark may make its family current.
    client.ingame = true;
    client.scene_state = 2;
    let quiet = slot.after_drain(&mut client);
    assert!(!quiet.dirty.any());
    assert!(slot.snapshot.npcs().is_empty());
    assert!(slot.snapshot.local_player().is_none());

    let mut player = client::io::Packet::new(vec![0xe0, 0x50, 0xc0, 0]);
    client.psize = 4;
    client.handle_packet(client::io::ServerProt::PLAYER_INFO, &mut player);
    let fresh = slot.after_drain(&mut client);
    assert!(fresh.player_info);
    assert!(slot.snapshot.local_player().is_some());
    assert!(slot.snapshot.npcs().is_empty());
    assert_eq!(slot.snapshot.tick(), 1);
}

/// Drive the real handshake so the client records its grant watermark.
fn reconnect_grant(client: &mut Client) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    client.config.port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut header = [0; 2];
        stream.read_exact(&mut header).unwrap();
        assert_eq!(header[0], 14);
        stream.write_all(&[0; 17]).unwrap();
        stream.read_exact(&mut header).unwrap();
        assert_eq!(header[0], 18);
        let mut login = vec![0; header[1] as usize];
        stream.read_exact(&mut login).unwrap();
        stream.write_all(&[15]).unwrap();
    });
    client.login("snapshot", "test", true).unwrap();
    server.join().unwrap();
}

#[test]
fn successful_reconnect_generation_resets_snapshot_while_ingame_stays_true() {
    use client::client::{ClientNpc, ClientPlayer};
    use client::io::ServerProt;

    let mut client = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    client.ingame = true;
    client.scene_state = 2;
    client.local_player = Some(ClientPlayer::at(5, 6));
    client.npc[7] = Some(Box::new(ClientNpc::default()));
    client.npc_ids[0] = 7;
    client.npc_count = 1;
    client.map_build_base_x = 3200;
    client.map_build_base_z = 3201;
    client.minusedlevel = 1;
    client.collision[1].add_wall(5, 6, 0, 0, false);
    install_revision_snapshot_ifaces(&mut client);
    dispatch_packet(
        &mut client,
        ServerProt::UPDATE_INV_FULL,
        vec![0, 3, 1, 0, 1, 2],
    );
    client.bump_gens(ServerProt::REBUILD_NORMAL);

    let mut pump = Pump::new();
    let mut snapshot = GameSnapshot::new();
    publish_snapshot(&mut snapshot, &client, pump.drain_client(&client));
    assert_eq!(snapshot.npcs().len(), 1);
    assert!(snapshot.local_player().is_some());
    assert_eq!(snapshot.inv_count(0), 2);
    assert!(snapshot.scene().available);

    // A response-15 reconnect does not set ingame false or clear actor
    // tables. The successful-session generation is the only reliable
    // boundary available to the host.
    let scene_gen = client.gens.scene;
    reconnect_grant(&mut client);
    let result = pump.drain_client(&client);
    publish_snapshot(&mut snapshot, &client, result);

    assert!(result.session_changed);
    assert_eq!(
        client.gens.scene, scene_gen,
        "grant must not invent a scene gen"
    );
    assert!(snapshot.npcs().is_empty());
    assert!(snapshot.local_player().is_none());
    assert!(snapshot.inv().is_empty());
    assert_eq!(snapshot.tick(), 0);
    assert!(snapshot.ingame());
    assert_eq!(snapshot.gens().session, client.gens.session);

    let scene = snapshot.scene();
    assert!(
        scene.available,
        "the retained current collision map is usable"
    );
    assert_eq!((scene.base_x, scene.base_z, scene.level), (3200, 3201, 1));
    assert_eq!(
        scene.collision_flags,
        client.collision[1]
            .flags
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>()
    );
}

#[test]
fn rebuild_invalidation_refreshes_player_view_without_inventing_tick() {
    use client::client::ClientPlayer;
    use client::io::{Packet, ServerProt};

    let mut client = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    client.ingame = true;
    client.local_player = Some(ClientPlayer::at(10, 10));
    let mut rebuild = Packet::new(vec![0, 50, 0, 50]);
    client.psize = 4;
    client.handle_packet(ServerProt::REBUILD_NORMAL, &mut rebuild);

    let mut slot = SlotLoop::new();
    let result = slot.after_drain(&mut client);

    assert!(result.dirty.player && result.dirty.scene);
    assert!(!result.player_info);
    assert_eq!(slot.snapshot.base(), Some((352, 352)));
    assert_eq!(slot.snapshot.tick(), 0);
}

fn bit_packet(fields: &[(usize, u32)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut position = 0;
    for &(width, value) in fields {
        assert!(value < (1 << width));
        for shift in (0..width).rev() {
            if position % 8 == 0 {
                bytes.push(0);
            }
            *bytes.last_mut().unwrap() |= (((value >> shift) & 1) as u8) << (7 - position % 8);
            position += 1;
        }
    }
    bytes
}

fn dispatch_packet(client: &mut Client, opcode: i32, bytes: Vec<u8>) {
    use client::io::Packet;

    client.psize = bytes.len() as i32;
    let mut packet = Packet::new(bytes);
    packet.set_frame_end(client.psize as usize);
    client.handle_packet(opcode, &mut packet);
    assert_eq!(packet.pos, client.psize as usize);
    assert!(client.ingame, "fixture packet must not T2/logout");
}

fn install_revision_snapshot_ifaces(client: &mut Client) {
    use client::config::if_type::ComponentType;

    client.set_iface(
        3,
        IfType {
            id: 3,
            r#type: ComponentType::TYPE_INV,
            obj_ops: true,
            ..IfType::default()
        },
    );
    client.set_iface_mut(
        3,
        IfTypeMut {
            link_obj_type: Some(vec![0; 260]),
            link_obj_number: Some(vec![0; 260]),
            ..IfTypeMut::default()
        },
    );
    client.side_icon[3] = 3;
    client.set_iface(
        11,
        IfType {
            id: 11,
            layer_id: 11,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![13]),
            ..IfType::default()
        },
    );
    client.set_iface(12, IfType::default());
    client.set_iface(
        13,
        IfType {
            id: 13,
            layer_id: 11,
            r#type: ComponentType::TYPE_INV,
            iop: [Some("Withdraw 1".into()), None, None, None, None],
            ..IfType::default()
        },
    );
    client.set_iface_mut(
        13,
        IfTypeMut {
            link_obj_type: Some(vec![0; 4]),
            link_obj_number: Some(vec![0; 4]),
            ..IfTypeMut::default()
        },
    );
}

fn qualify_decoder_to_host_snapshot(revision: client::client::ClientRevision) {
    use client::client::{ClientNpc, ClientPlayer};
    use client::io::{ServerProt, ServerProt289};

    let mut client = Client::new_with_revision(cfg(), revision);
    client.ingame = true;
    client.scene_state = 2;
    client.self_slot = 5;
    client.local_player = Some(ClientPlayer::at(10, 10));
    client.players[2047] = Some(Box::new(ClientPlayer::at(10, 10)));
    install_revision_snapshot_ifaces(&mut client);
    let mut slot = SlotLoop::new();

    let (inv_full, inv_partial, inv_full_opcode, inv_partial_opcode) = if revision.is_289() {
        (
            vec![0, 3, 0, 2, 0, 1, 2, 0, 3, 3],
            vec![0, 3, 0x80, 0x80, 0, 5, 9],
            ServerProt289::UPDATE_INV_FULL,
            ServerProt289::UPDATE_INV_PARTIAL,
        )
    } else {
        (
            vec![0, 3, 2, 0, 1, 2, 0, 3, 3],
            vec![0, 3, 1, 0, 5, 9],
            ServerProt::UPDATE_INV_FULL,
            ServerProt::UPDATE_INV_PARTIAL,
        )
    };
    dispatch_packet(&mut client, inv_full_opcode, inv_full);
    slot.after_drain(&mut client);
    assert_eq!(slot.snapshot.inv_count(0), 2);
    assert_eq!(slot.snapshot.inv_count(2), 3);

    dispatch_packet(&mut client, inv_partial_opcode, inv_partial);
    slot.after_drain(&mut client);
    assert_eq!(slot.snapshot.inv_count(4), 9);
    if revision.is_289() {
        assert_eq!(slot.snapshot.inventory()[2].slot, 128);
    } else {
        assert_eq!(slot.snapshot.inventory()[1].slot, 1);
    }

    let (open_opcode, bank_full_opcode, bank_full) = if revision.is_289() {
        (
            ServerProt289::IF_OPENMAIN_SIDE,
            ServerProt289::UPDATE_INV_FULL,
            vec![0, 13, 0, 1, 0, 6, 20],
        )
    } else {
        (
            ServerProt::IF_OPENMAIN_SIDE,
            ServerProt::UPDATE_INV_FULL,
            vec![0, 13, 1, 0, 6, 20],
        )
    };
    dispatch_packet(&mut client, open_opcode, vec![0, 11, 0, 12]);
    dispatch_packet(&mut client, bank_full_opcode, bank_full);
    slot.after_drain(&mut client);
    assert_eq!((client.main_modal_id, client.side_modal_id), (11, 12));
    assert_eq!(slot.snapshot.modals().main, 11);
    assert_eq!(slot.snapshot.bank_component_id(), 13);
    assert_eq!(slot.snapshot.bank()[0].def.id, 5);
    assert_eq!(slot.snapshot.bank()[0].count, 20);

    let rebuild_opcode = if revision.is_289() {
        ServerProt289::REBUILD_NORMAL
    } else {
        ServerProt::REBUILD_NORMAL
    };
    let rebuild = if revision.is_289() {
        vec![0, 16, 0, 32]
    } else {
        vec![0, 50, 0, 50]
    };
    dispatch_packet(&mut client, rebuild_opcode, rebuild);
    let rebuild_result = slot.after_drain(&mut client);
    assert!(!rebuild_result.player_info);
    assert!(rebuild_result.dirty.scene && rebuild_result.dirty.player);
    assert_eq!(
        slot.snapshot.base(),
        Some(if revision.is_289() {
            (80, 208)
        } else {
            (352, 352)
        })
    );
    assert_eq!(slot.snapshot.tick(), 0);

    client.npc[3] = Some(Box::new(ClientNpc::default()));
    client.npc_ids[0] = 3;
    client.npc_count = 1;
    let npc_opcode = if revision.is_289() {
        ServerProt289::NPC_INFO
    } else {
        ServerProt::NPC_INFO
    };
    dispatch_packet(&mut client, npc_opcode, bit_packet(&[(8, 1), (1, 0)]));
    slot.after_drain(&mut client);
    assert_eq!(slot.snapshot.npcs()[0].index, 3);

    let (player_opcode, player_frame, expected_tile) = if revision.is_289() {
        (
            ServerProt289::PLAYER_INFO,
            bit_packet(&[
                (1, 1),
                (2, 3),
                (2, 2),
                (7, 80),
                (7, 81),
                (1, 1),
                (1, 0),
                (8, 0),
            ]),
            (160, 289, 2),
        )
    } else {
        (
            ServerProt::PLAYER_INFO,
            vec![0xe0, 0x50, 0xc0, 0],
            (357, 358, 0),
        )
    };
    dispatch_packet(&mut client, player_opcode, player_frame);
    let tick_result = slot.after_drain(&mut client);
    assert!(tick_result.player_info);
    assert!(should_emit_tick(tick_result.player_info));
    assert_eq!(slot.snapshot.tick(), 1);
    assert_eq!(slot.snapshot.tile(), Some(expected_tile));
    assert_eq!(
        slot.snapshot.local_player().map(|p| p.player.index),
        Some(5)
    );
}

#[test]
fn revision_274_decoder_packets_publish_host_snapshot() {
    qualify_decoder_to_host_snapshot(client::client::ClientRevision::R274);
}

#[test]
fn revision_289_decoder_packets_publish_host_snapshot() {
    qualify_decoder_to_host_snapshot(client::client::ClientRevision::R289);
}

#[test]
fn session_change_same_drain_publishes_fresh_packet_families() {
    use client::client::ClientPlayer;
    use client::io::ServerProt;

    let mut client = Client::new(cfg());
    client.ingame = true;
    client.scene_state = 2;
    client.self_slot = 5;
    client.local_player = Some(ClientPlayer::at(10, 10));
    client.players[2047] = Some(Box::new(ClientPlayer::at(10, 10)));
    install_revision_snapshot_ifaces(&mut client);

    let mut pump = Pump::new();
    pump.drain(client.gens);
    let mut snapshot = GameSnapshot::new();

    reconnect_grant(&mut client);
    dispatch_packet(
        &mut client,
        ServerProt::UPDATE_INV_FULL,
        vec![0, 3, 2, 0, 1, 2, 0, 3, 3],
    );
    dispatch_packet(
        &mut client,
        ServerProt::IF_OPENMAIN_SIDE,
        vec![0, 11, 0, 12],
    );
    dispatch_packet(
        &mut client,
        ServerProt::PLAYER_INFO,
        vec![0xe0, 0x50, 0xc0, 0],
    );

    let result = pump.drain_client(&client);
    assert!(result.session_changed && result.player_info);
    publish_snapshot(&mut snapshot, &client, result);

    assert_eq!(snapshot.tick(), 1);
    assert!(snapshot.local_player().is_some());
    assert_eq!(snapshot.inv_count(0), 2);
    assert_eq!(snapshot.inventory()[0].count, 2);
    assert_eq!(snapshot.modals().main, 11);
}

#[test]
fn reconnect_drain_discards_old_packets_and_invalidation_but_keeps_fresh_families() {
    use client::client::{ClientNpc, ClientPlayer, ClientRevision};
    use client::io::{ServerProt, ServerProt289};
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let mut client = Client::new_with_revision(cfg(), revision);
        client.ingame = true;
        client.scene_state = 2;
        client.local_player = Some(ClientPlayer::at(10, 10));
        client.players[2047] = Some(Box::new(ClientPlayer::at(10, 10)));
        client.npc[7] = Some(Box::new(ClientNpc::default()));
        client.npc_ids[0] = 7;
        client.npc_count = 1;
        install_revision_snapshot_ifaces(&mut client);
        let mut pump = Pump::new();
        pump.drain_client(&client);
        let mut snapshot = GameSnapshot::new();
        let (inv, player, rebuild, iface, inv_body, player_body) = if revision.is_289() {
            (
                ServerProt289::UPDATE_INV_FULL,
                ServerProt289::PLAYER_INFO,
                ServerProt289::REBUILD_NORMAL,
                ServerProt289::IF_OPENMAIN_SIDE,
                vec![0, 3, 0, 2, 0, 1, 2, 0, 3, 3],
                bit_packet(&[
                    (1, 1),
                    (2, 3),
                    (2, 0),
                    (7, 10),
                    (7, 12),
                    (1, 1),
                    (1, 0),
                    (8, 0),
                ]),
            )
        } else {
            (
                ServerProt::UPDATE_INV_FULL,
                ServerProt::PLAYER_INFO,
                ServerProt::REBUILD_NORMAL,
                ServerProt::IF_OPENMAIN_SIDE,
                vec![0, 3, 2, 0, 1, 2, 0, 3, 3],
                vec![0xe0, 0x50, 0xc0, 0],
            )
        };
        // The observer has not drained these previous-session packets.
        dispatch_packet(&mut client, inv, inv_body.clone());
        dispatch_packet(&mut client, player, player_body.clone());
        reconnect_grant(&mut client);
        // A real new-session scene invalidation must not revive inventory
        // or actors retained by response 15.
        dispatch_packet(&mut client, rebuild, vec![0, 50, 0, 50]);
        let drain = pump.drain_client(&client);
        assert!(!drain.player_info);
        publish_snapshot(&mut snapshot, &client, drain);
        assert!(snapshot.local_player().is_none());
        assert!(snapshot.npcs().is_empty());
        assert!(snapshot.inv().is_empty());
        dispatch_packet(&mut client, player, player_body);
        dispatch_packet(&mut client, iface, vec![0, 11, 0, 12]);
        publish_snapshot(&mut snapshot, &client, pump.drain_client(&client));
        assert_eq!(snapshot.tick(), 1);
        assert!(snapshot.local_player().is_some());
        assert!(snapshot.npcs().is_empty());
        assert!(snapshot.inventory().is_empty());
        dispatch_packet(&mut client, inv, inv_body);
        publish_snapshot(&mut snapshot, &client, pump.drain_client(&client));
        assert_eq!(snapshot.inv_count(0), 2);
        assert_eq!(snapshot.inventory()[0].count, 2);
    }
}

#[test]
fn quiet_drain_refreshes_scene_ready_scalar_without_grid_rebuild() {
    let mut client = Client::new(cfg());
    client.ingame = true;
    client.scene_state = 1;
    client.gens.scene = 1;
    let mut pump = Pump::new();
    let mut snapshot = GameSnapshot::new();
    publish_snapshot(&mut snapshot, &client, pump.drain(client.gens));
    assert_eq!(snapshot.scene_state(), 1);
    assert!(
        !snapshot.scene().available,
        "scene_state 1 must not publish a collision grid"
    );

    // check_scene performs this transition locally without moving a
    // packet-family generation. The host copies the scalar every quiet
    // drain; an unavailable grid is materialized once when the client
    // becomes scene-ready (retained collision after a session reset).
    client.scene_state = 2;
    let quiet = pump.drain(client.gens);
    assert!(!quiet.dirty.any());
    publish_snapshot(&mut snapshot, &client, quiet);

    assert_eq!(snapshot.scene_state(), 2);
    assert!(
        snapshot.scene().available,
        "scene-ready must materialize the missing host collision view once"
    );
    let flags_before = snapshot.scene().collision_flags.clone();
    client.collision[0].flags[0][0] ^= 1;
    let quiet_again = pump.drain(client.gens);
    assert!(!quiet_again.dirty.any());
    publish_snapshot(&mut snapshot, &client, quiet_again);
    assert_eq!(
        snapshot.scene().collision_flags,
        flags_before,
        "a later quiet drain must not copy the grid without a scene gen or identity change"
    );
}

fn ingame_scene2(client: &mut Client) {
    client.ingame = true;
    client.scene_state = 2;
    client.local_player = Some(client::client::ClientPlayer::at(10, 10));
    client.gens.player += 1;
    client.gens.player_info += 1;
}

#[test]
fn auto_run_does_not_send_on_the_title() {
    let mut client = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let mut slot = SlotLoop::new();
    client.runenergy = 100;
    client.gens.stat = 1;
    slot.after_drain(&mut client);
    assert_eq!(
        slot.run_sends, 0,
        "title IF_BUTTON is ignored; sending here sticks run_on"
    );
    assert!(!slot.run_on);

    ingame_scene2(&mut client);
    slot.after_drain(&mut client);
    assert_eq!(slot.run_sends, 1);
    assert!(slot.run_on);
}

#[test]
fn auto_run_20_0_20_sends_twice() {
    let mut client = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let mut slot = SlotLoop::new();
    ingame_scene2(&mut client);
    client.runenergy = 20;
    client.gens.stat = 1;
    slot.after_drain(&mut client);
    assert_eq!(slot.run_sends, 1);
    assert_eq!(client.out.data()[0], ClientProt::IF_BUTTON.id as u8);
    let iface = u16::from_be_bytes([client.out.data()[1], client.out.data()[2]]);
    assert_eq!(iface, RUN_ORB_IFACE as u16);

    client.out.pos = 0;
    client.runenergy = 0;
    client.gens.stat = 2;
    slot.after_drain(&mut client);
    assert!(!slot.run_on);
    assert_eq!(slot.run_sends, 1);

    client.runenergy = 20;
    client.gens.stat = 3;
    slot.after_drain(&mut client);
    assert_eq!(slot.run_sends, 2);
    assert!(slot.run_on);
}

#[test]
fn script_run_override_applies_until_shared_cell_is_cleared() {
    let mut client = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let mut slot = SlotLoop::new();
    ingame_scene2(&mut client);
    client.runenergy = 20;
    client.gens.stat = 1;

    slot.run_policy_override
        .set(Some(api::run_policy::RunPolicyOverride {
            run_auto: None,
            energy_min: Some(api::run_policy::RunEnergyMin::Floor(80)),
        }));
    slot.after_drain(&mut client);
    assert_eq!(
        slot.run_sends, 0,
        "session threshold overrides host default 20"
    );

    slot.run_policy_override.clear();
    slot.after_drain(&mut client);
    assert_eq!(
        slot.run_sends, 1,
        "clear falls back to unchanged host default 20"
    );
}

#[test]
fn already_running_echo_does_not_send() {
    let mut ifaces = vec![None; 154];
    ifaces[152] = Some(Box::new(IfType::default()));
    ifaces[153] = Some(Box::new(IfType::default()));
    let mut ifaces_mut = vec![None; 154];
    ifaces_mut[152] = Some(Arc::new(IfTypeMut {
        hide: false,
        ..IfTypeMut::default()
    }));
    ifaces_mut[153] = Some(Arc::new(IfTypeMut {
        hide: true,
        ..IfTypeMut::default()
    }));
    let mut client = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(ifaces),
        ifaces_mut,
    );
    client.runenergy = 20;
    client.gens.stat = 1;
    ingame_scene2(&mut client);
    let mut slot = SlotLoop::new();
    slot.after_drain(&mut client);
    assert_eq!(slot.run_sends, 0, "already on → no extra send");
    assert!(slot.run_on);
}

#[test]
fn unpacked_ifaces_both_visible_still_sends() {
    let mut ifaces = vec![None; 154];
    ifaces[152] = Some(Box::new(IfType::default()));
    ifaces[153] = Some(Box::new(IfType::default()));
    let mut client = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(ifaces),
        Vec::new(),
    );
    client.runenergy = 20;
    client.gens.stat = 1;
    ingame_scene2(&mut client);
    let mut slot = SlotLoop::new();
    slot.after_drain(&mut client);
    assert_eq!(slot.run_sends, 1);
}

#[test]
fn client_frame_applies_click_only_when_input_enabled() {
    let mut c = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let inp = SlotInput::new();
    let (tx, rx) = std::sync::mpsc::channel();
    inp.connect_rx(rx);
    tx.send(InputEv::Down {
        button: 1,
        x: 20,
        y: 20,
    })
    .unwrap();
    let mut slot = SlotLoop::new();
    let mut sends = 0u32;
    inp.set_enabled(false);
    Host::client_frame(&mut c, &mut slot, "t", Some(&inp), None, &mut sends, None);
    assert_eq!(c.shell.mouse_click_button, 0);
    inp.set_enabled(true);
    Host::client_frame(&mut c, &mut slot, "t", Some(&inp), None, &mut sends, None);
    assert_eq!(c.shell.mouse_click_button, 1);
}

#[test]
fn client_frame_consumes_script_mouse_with_capture_off() {
    let mut c = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let inp = SlotInput::new();
    inp.authority().publish_live();
    inp.set_enabled(false);
    inp.enqueue_script_mouse(true, 382.5, 251.5, 0);
    let mut slot = SlotLoop::new();
    let mut sends = 0u32;
    Host::client_frame(&mut c, &mut slot, "t", Some(&inp), None, &mut sends, None);
    assert_eq!(
        (
            c.shell.mouse_click_button,
            c.shell.mouse_click_x,
            c.shell.mouse_click_y,
            c.shell.mouse_button
        ),
        (1, 382, 251, 1)
    );
}

#[test]
fn client_frame_skips_frame_store_when_draw_off() {
    force_cpu_backend();
    let mut c = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let buf = FrameBuf::new();
    let mut slot = SlotLoop::new();
    let mut sends = 0u32;
    // Renderer off: no frame is rendered and nothing is stored.
    c.set_draw(false);
    Host::client_frame(&mut c, &mut slot, "t", None, Some(&buf), &mut sends, None);
    assert!(buf.snapshot().is_empty(), "draw off must not store pixels");
    // Renderer on: the first (rising-edge) tick paints a full applet
    // into the buffer (with no title assets in this test the paint is
    // empty, but the frame still packs a full applet; the non-zero
    // paint is proven live by the panel_view e2e).
    c.set_draw(true);
    Host::client_frame(&mut c, &mut slot, "t", None, Some(&buf), &mut sends, None);
    assert_eq!(
        buf.snapshot().len(),
        (client::client::APPLET_W * client::client::APPLET_H) as usize
    );
}

#[test]
fn headless_slot_constructs_no_renderer_and_never_draws() {
    let mut c = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let mut slot = SlotLoop::new();
    let mut sends = 0u32;
    // `client.draw` defaults false: a headless slot never paints. The
    // check is slot-local (not the global `Renderer::constructed()`
    // counter, which other tests' renderers bump concurrently).
    for _ in 0..3 {
        Host::client_frame(&mut c, &mut slot, "t", None, None, &mut sends, None);
    }
    assert!(
        slot.renderer.is_none(),
        "draw off must not construct a Renderer"
    );
    assert_eq!(slot.skip_n, 3);
    assert_eq!(slot.paint_n, 0);
    assert!(slot.loop_ns > 0, "mainloop still ran");
}

#[test]
fn draw_off_drops_renderer_draw_on_reattaches() {
    force_cpu_backend();
    let mut c = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let mut slot = SlotLoop::new();
    let mut sends = 0u32;
    c.set_draw(true);
    Host::client_frame(&mut c, &mut slot, "t", None, None, &mut sends, None);
    assert!(slot.renderer.is_some());
    let loop_after_on = slot.loop_ns;
    c.set_draw(false);
    Host::client_frame(&mut c, &mut slot, "t", None, None, &mut sends, None);
    assert!(slot.renderer.is_none(), "unheaded must drop headed data");
    assert!(slot.loop_ns > loop_after_on, "sim still ticks");
    c.set_draw(true);
    Host::client_frame(&mut c, &mut slot, "t", None, None, &mut sends, None);
    assert!(slot.renderer.is_some(), "attach at any time");
}

/// Draw-off (only-render-selected, 49 heads dropping) must drain the
/// mailbox so a `FrameOutput::Texture` cannot outlive `frame_texture`.
#[test]
fn draw_off_drains_the_frame_mailbox_before_dropping_the_head() {
    force_cpu_backend();
    let mut c = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let buf = FrameBuf::new();
    let mut slot = SlotLoop::new();
    let mut sends = 0u32;
    c.set_draw(true);
    Host::client_frame(&mut c, &mut slot, "t", None, Some(&buf), &mut sends, None);
    assert!(buf.take().is_some(), "a paint tick stores a frame");
    c.set_draw(true);
    Host::client_frame(&mut c, &mut slot, "t", None, Some(&buf), &mut sends, None);
    assert!(slot.renderer.is_some());
    c.set_draw(false);
    Host::client_frame(&mut c, &mut slot, "t", None, Some(&buf), &mut sends, None);
    assert!(slot.renderer.is_none());
    assert!(
        buf.take().is_none(),
        "draw-off must take the mailbox before dropping the renderer"
    );
}

/// Final-review fix: a draw-off detach must dematerialize the headed
/// overlay meshes (`Square.ground`/`quick_ground`) from the sim — keep
/// the compact `overlay_stamp` typecodes and re-arm the attach
/// materialize — so an unheaded slot never holds headed data, and the
/// next attach restores the mesh without a rebuild.
#[test]
fn draw_off_dematerializes_overlay_draw_on_restores_mesh() {
    force_cpu_backend();
    let mut c = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let mut slot = SlotLoop::new();
    let mut sends = 0u32;
    // A headed overlay mesh, planted on the sim directly (no full
    // map_build needed to exercise the hook).
    c.world.fill_base_level(0);
    c.set_draw(true);
    c.world.set_ground(
        0,
        2,
        2,
        TerrainOverlayShape::TRAPEZIUM,
        1,
        7,
        1000,
        1004,
        1010,
        1006,
        0x111111,
        0x222222,
        0x333333,
        0x444444,
        0x555555,
        0x666666,
        0x777777,
        0x888888,
        0x99,
        0xaa,
    );
    Host::client_frame(&mut c, &mut slot, "t", None, None, &mut sends, None);
    assert!(
        c.world
            .square(0, 2, 2)
            .and_then(|s| s.ground.as_ref())
            .is_some(),
        "headed draw must build the overlay mesh"
    );

    // Draw off drops the head and the headed mesh; the stamp survives
    // and the next attach is re-armed.
    c.set_draw(false);
    Host::client_frame(&mut c, &mut slot, "t", None, None, &mut sends, None);
    assert!(slot.renderer.is_none());
    let sq = c.world.square(0, 2, 2).expect("stamped tile");
    assert!(sq.ground.is_none(), "draw off must drop the overlay mesh");
    assert!(sq.quick_ground.is_none());
    assert!(
        sq.overlay_stamp.is_some(),
        "the stamp must survive a detach"
    );
    assert!(
        c.world.overlay_pending,
        "a detach must re-arm the attach materialize"
    );
    assert!(
        c.world.share_light_pending,
        "a detach must re-arm share_light: loc models died with the head"
    );
    assert_eq!(
        c.minimap_level, -1,
        "a detach must dirty the minimap so the next head recomposes it"
    );

    // Draw on: the reattached head's first paint consumes the pending
    // flag and restores the mesh from the stamp — no map_build.
    c.ingame = true;
    c.scene_state = 2;
    c.set_draw(true);
    Host::client_frame(&mut c, &mut slot, "t", None, None, &mut sends, None);
    assert!(slot.renderer.is_some());
    assert!(
        c.world
            .square(0, 2, 2)
            .and_then(|s| s.ground.as_ref())
            .is_some(),
        "attach must rematerialize the overlay mesh"
    );
    assert!(
        !c.world.overlay_pending,
        "the first paint must consume the attach materialize"
    );
}

#[test]
fn backend_flip_rearms_share_light_without_dropping_overlay() {
    let mut c = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    c.world.share_light_pending = false;
    c.world.overlay_pending = false;
    c.minimap_level = 0;
    rearm_after_head_drop(&mut c, false, true);
    assert!(
        c.world.share_light_pending,
        "GPU↔CPU / mem flip must re-arm share_light; loc models died with the head"
    );
    assert!(
        !c.world.overlay_pending,
        "a backend flip must not dematerialize overlay meshes still on the sim"
    );
    assert_eq!(c.minimap_level, -1);
}

#[test]
fn prefer_cpu_rebuilds_renderer_not_client() {
    force_cpu_backend();
    let mut c = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let inp = SlotInput::new();
    inp.set_prefer_cpu(false);
    let mut slot = SlotLoop::new();
    let mut sends = 0u32;
    c.set_draw(true);
    Host::client_frame(&mut c, &mut slot, "t", Some(&inp), None, &mut sends, None);
    let uid = c.login_uid;
    inp.set_prefer_cpu(true);
    Host::client_frame(&mut c, &mut slot, "t", Some(&inp), None, &mut sends, None);
    assert_eq!(c.login_uid, uid);
    assert!(slot.renderer.is_some());
    // Under `force_cpu_backend` both preferences land on
    // `BackendKind::Cpu`, so the kind alone cannot prove a rebuild:
    // the head must carry the *new* `prefer_cpu` request, and the flip
    // tick must have repainted with it.
    assert_eq!(
        slot.renderer_prefer_cpu,
        Some(true),
        "the flip must rebuild the head for the new prefer_cpu"
    );
    assert_eq!(slot.paint_n, 2, "the flip tick must repaint the new head");
    assert_eq!(
        slot.renderer.as_ref().unwrap().backend_kind(),
        client::render::backend::BackendKind::Cpu
    );
}

#[test]
fn bot_cpu_env_forces_cpu_even_when_slot_input_prefers_gpu() {
    force_cpu_backend();
    let prev = std::env::var("BOT_CPU").ok();
    std::env::set_var("BOT_CPU", "1");
    let mut c = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let inp = SlotInput::new();
    inp.set_prefer_cpu(false);
    let mut slot = SlotLoop::new();
    let mut sends = 0u32;
    c.set_draw(true);
    Host::client_frame(&mut c, &mut slot, "t", Some(&inp), None, &mut sends, None);
    match prev {
        Some(v) => std::env::set_var("BOT_CPU", v),
        None => std::env::remove_var("BOT_CPU"),
    }
    assert_eq!(
        slot.renderer_prefer_cpu,
        Some(true),
        "BOT_CPU=1 must force CpuPix3D even when SlotInput.prefer_cpu is false"
    );
}

#[test]
fn lowmem_flip_rebuilds_renderer_not_client() {
    force_cpu_backend();
    let mut c = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let mut slot = SlotLoop::new();
    let mut sends = 0u32;
    c.set_draw(true);
    assert!(c.config.lowmem, "cfg() spawns lowmem");
    Host::client_frame(&mut c, &mut slot, "t", None, None, &mut sends, None);
    assert_eq!(slot.renderer_lowmem, Some(true));
    let uid = c.login_uid;
    c.set_lowmem(false);
    Host::client_frame(&mut c, &mut slot, "t", None, None, &mut sends, None);
    assert_eq!(c.login_uid, uid);
    assert!(slot.renderer.is_some());
    assert_eq!(
        slot.renderer_lowmem,
        Some(false),
        "a mem flip must drop the head so the next paint reads the new config.lowmem"
    );
}

#[test]
fn client_frame_keeps_loop_counters_slot_local() {
    let mut c = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let mut slot = SlotLoop::new();
    let mut sends = 0u32;
    c.set_draw(false);
    Host::client_frame(&mut c, &mut slot, "t", None, None, &mut sends, None);
    assert_eq!(slot.skip_n, 1);
    assert_eq!(slot.paint_n, 0);
    assert!(slot.loop_ns > 0);
}

#[test]
fn client_frame_draw_on_paints_this_tick() {
    force_cpu_backend();
    let mut c = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let mut slot = SlotLoop::new();
    let mut sends = 0u32;
    c.set_draw(true);
    Host::client_frame(&mut c, &mut slot, "t", None, None, &mut sends, None);
    assert_eq!(slot.paint_n, 1);
    assert_eq!(slot.skip_n, 0);
}

#[test]
fn mainredraw_runs_check_minimap_on_a_paint_tick() {
    force_cpu_backend();
    let mut c = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let buf = FrameBuf::new();
    let mut slot = SlotLoop::new();
    let mut sends = 0u32;
    c.set_draw(true);
    c.ingame = true;
    c.scene_state = 2;
    // A fresh client starts with `minimap_level = -1` (reset by login)
    // while `minusedlevel` is 0. Only `Renderer::mainredraw`'s
    // `check_minimap` render half brings `minimap_level` up to
    // `minusedlevel`; the raw `game_draw` stage does not, so this
    // pins the render dispatch to `mainredraw`.
    assert_eq!(c.minimap_level, -1);
    assert_ne!(c.minimap_level, c.minusedlevel);
    Host::client_frame(&mut c, &mut slot, "t", None, Some(&buf), &mut sends, None);
    assert_eq!(
        c.minimap_level, c.minusedlevel,
        "mainredraw must run the check_minimap render half"
    );
    assert_eq!(slot.paint_n, 1);
    assert_eq!(
        buf.snapshot().len(),
        (client::client::APPLET_W * client::client::APPLET_H) as usize
    );
}

#[test]
fn client_tick_observe_runs_before_the_frame_paint() {
    force_cpu_backend();
    let mut c = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let buf = FrameBuf::new();
    let mut slot = SlotLoop::new();
    let mut sends = 0u32;
    assert!(!c.draw, "slots start with the renderer off");
    // A drawing slot paints this tick; an observe-after-frame would
    // skip it, so observe-before must run first.
    c.set_draw(true);
    let observed = AtomicBool::new(false);
    Host::client_tick(
        &mut c,
        &mut slot,
        "t",
        None,
        Some(&buf),
        &mut sends,
        &mut |_, _, _, _| {
            observed.store(true, Ordering::Relaxed);
            false
        },
        None,
        &RandomStatus::default(),
    );
    assert!(observed.load(Ordering::Relaxed));
    assert_eq!(
        buf.snapshot().len(),
        (client::client::APPLET_W * client::client::APPLET_H) as usize
    );
    let gen = buf.generation();
    c.set_draw(false);
    Host::client_tick(
        &mut c,
        &mut slot,
        "t",
        None,
        Some(&buf),
        &mut sends,
        &mut |_, _, _, _| false,
        None,
        &RandomStatus::default(),
    );
    assert!(!c.draw);
    assert_eq!(
        buf.generation(),
        gen,
        "renderer off must not store a frame this tick"
    );
}

#[test]
fn debug_window_delta_is_since_last_summary_not_lifetime() {
    let prev = DebugSnap {
        loop_ns: 1_000_000,
        raster_ns: 2_000_000,
        observe_ns: 100_000,
        paint_n: 50,
        skip_n: 10,
        tick_n: 3,
    };
    let now = DebugSnap {
        loop_ns: 1_500_000,
        raster_ns: 2_400_000,
        observe_ns: 400_000,
        paint_n: 100,
        skip_n: 10,
        tick_n: 5,
    };
    assert_eq!(
        debug_window_delta(prev, now),
        (500, 400, 300, 50, 0, 2),
        "summary must be a window, not cumulative totals"
    );
}

#[test]
fn debug_hitch_is_one_frame_over_50ms_not_every_tick() {
    assert_eq!(debug_hitch_us(20_000), None);
    assert_eq!(debug_hitch_us(49_999), None);
    assert_eq!(debug_hitch_us(50_000), Some(50_000));
    assert_eq!(debug_hitch_us(120_000), Some(120_000));
}

#[test]
fn debug_observe_hitch_is_one_missed_frame_not_every_tick() {
    assert_eq!(debug_observe_hitch_us(15_999), None);
    assert_eq!(debug_observe_hitch_us(16_000), Some(16_000));
    assert_eq!(debug_observe_hitch_us(40_000), Some(40_000));
}

#[test]
fn raster_this_tick_watch_is_wall_clock_one_fps_capture_is_every_tick() {
    let t0 = Instant::now();
    let mut last = None;
    let mut on = false;
    assert!(!raster_this_tick(false, false, t0, &mut last, &mut on));
    assert!(
        raster_this_tick(true, false, t0, &mut last, &mut on),
        "rising edge paints now"
    );
    // Sub-second wakes (the parked slot's cadence) stay quiet…
    assert!(!raster_this_tick(
        true,
        false,
        t0 + Duration::from_millis(500),
        &mut last,
        &mut on
    ));
    // …and the paint lands once a wall-clock second elapses.
    assert!(raster_this_tick(
        true,
        false,
        t0 + Duration::from_secs(1),
        &mut last,
        &mut on
    ));
    assert!(!raster_this_tick(
        true,
        false,
        t0 + Duration::from_secs(1) + Duration::from_millis(100),
        &mut last,
        &mut on
    ));
    // Capture paints every tick (minimenu / TV static / full-rate).
    assert!(raster_this_tick(
        true,
        true,
        t0 + Duration::from_secs(1),
        &mut last,
        &mut on
    ));
    assert!(raster_this_tick(
        true,
        true,
        t0 + Duration::from_secs(1),
        &mut last,
        &mut on
    ));
    on = false;
    assert!(
        raster_this_tick(true, false, t0 + Duration::from_secs(1), &mut last, &mut on),
        "draw rising after off paints immediately"
    );
}

#[test]
fn full_rate_title_paints_every_host_tick() {
    force_cpu_backend();
    let mut c = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let buf = FrameBuf::new();
    let mut slot = SlotLoop::new();
    let mut sends = 0u32;
    let inp = SlotInput::new();
    inp.set_full_rate(true);
    c.set_draw(true);
    c.ingame = false;

    Host::client_frame(
        &mut c,
        &mut slot,
        "t",
        Some(&inp),
        Some(&buf),
        &mut sends,
        None,
    );
    Host::client_frame(
        &mut c,
        &mut slot,
        "t",
        Some(&inp),
        Some(&buf),
        &mut sends,
        None,
    );

    assert_eq!(buf.generation(), 2, "a full-rate title paints every tick");
}

#[test]
fn watch_rate_title_catches_up_flames_without_extra_paints() {
    force_cpu_backend();
    let mut c = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let buf = FrameBuf::new();
    let mut slot = SlotLoop::new();
    let mut sends = 0u32;
    let inp = SlotInput::new();
    c.set_draw(true);
    c.ingame = false;

    Host::client_frame(
        &mut c,
        &mut slot,
        "t",
        Some(&inp),
        Some(&buf),
        &mut sends,
        None,
    );
    let first_cycle = slot
        .renderer
        .as_ref()
        .and_then(|r| r.title_flames.as_ref())
        .expect("title flames")
        .cycle;
    Host::client_frame(
        &mut c,
        &mut slot,
        "t",
        Some(&inp),
        Some(&buf),
        &mut sends,
        None,
    );
    assert_eq!(buf.generation(), 1, "watch title stays at one fps");

    thread::sleep(Duration::from_secs(1) + Duration::from_millis(50));
    Host::client_frame(
        &mut c,
        &mut slot,
        "t",
        Some(&inp),
        Some(&buf),
        &mut sends,
        None,
    );
    let second_cycle = slot
        .renderer
        .as_ref()
        .and_then(|r| r.title_flames.as_ref())
        .expect("title flames")
        .cycle;
    assert_eq!(buf.generation(), 2, "watch title repaints after one second");
    assert!(
        second_cycle - first_cycle > 1,
        "one watch paint catches up multiple 35 ms flame frames"
    );
}

#[test]
fn watch_only_paints_first_tick_then_once_per_second() {
    force_cpu_backend();
    let mut c = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let buf = FrameBuf::new();
    let mut slot = SlotLoop::new();
    let mut sends = 0u32;
    c.set_draw(true);
    c.ingame = true;
    c.scene_state = 2;
    Host::client_frame(&mut c, &mut slot, "t", None, Some(&buf), &mut sends, None);
    assert_eq!(buf.generation(), 1);
    // A fast second tick (the parked slot draining a burst) must not
    // repaint before a wall-clock second elapses.
    Host::client_frame(&mut c, &mut slot, "t", None, Some(&buf), &mut sends, None);
    assert_eq!(buf.generation(), 1);
    thread::sleep(Duration::from_secs(1) + Duration::from_millis(50));
    Host::client_frame(&mut c, &mut slot, "t", None, Some(&buf), &mut sends, None);
    assert_eq!(buf.generation(), 2, "watch-only repaints after 1 s");
}

#[test]
fn full_rate_paints_every_tick_after_scene_ready() {
    force_cpu_backend();
    let mut c = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let buf = FrameBuf::new();
    let mut slot = SlotLoop::new();
    let mut sends = 0u32;
    c.set_draw(true);
    // The sidecar-50 pref drives the frame cadence through the shared
    // SlotInput, not a slot-local field.
    let inp = SlotInput::new();
    inp.set_full_rate(true);
    c.ingame = true;
    c.scene_state = 2;
    Host::client_frame(
        &mut c,
        &mut slot,
        "t",
        Some(&inp),
        Some(&buf),
        &mut sends,
        None,
    );
    assert_eq!(buf.generation(), 1);
    Host::client_frame(
        &mut c,
        &mut slot,
        "t",
        Some(&inp),
        Some(&buf),
        &mut sends,
        None,
    );
    assert_eq!(
        buf.generation(),
        2,
        "TV full_rate must redraw 2D+3D every 20 ms, not 1 fps watch"
    );
    // Clearing the latch drops back to the 1 fps watch cadence.
    inp.set_full_rate(false);
    Host::client_frame(
        &mut c,
        &mut slot,
        "t",
        Some(&inp),
        Some(&buf),
        &mut sends,
        None,
    );
    assert_eq!(
        buf.generation(),
        2,
        "full_rate off must not paint sub-second"
    );
}

#[test]
fn loading_scene_paints_every_tick_for_tv_static() {
    force_cpu_backend();
    let mut c = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let buf = FrameBuf::new();
    let mut slot = SlotLoop::new();
    let mut sends = 0u32;
    c.set_draw(true);
    c.ingame = true;
    c.scene_state = 1;
    Host::client_frame(&mut c, &mut slot, "t", None, Some(&buf), &mut sends, None);
    assert_eq!(buf.generation(), 1);
    Host::client_frame(&mut c, &mut slot, "t", None, Some(&buf), &mut sends, None);
    assert_eq!(
        buf.generation(),
        2,
        "scene_state != 2 must re-roll static every 20 ms"
    );
}

#[test]
fn capture_draw_copies_every_tick() {
    force_cpu_backend();
    let mut c = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let buf = FrameBuf::new();
    let inp = SlotInput::new();
    inp.set_full_rate(true);
    let mut slot = SlotLoop::new();
    let mut sends = 0u32;
    c.set_draw(true);
    Host::client_frame(
        &mut c,
        &mut slot,
        "t",
        Some(&inp),
        Some(&buf),
        &mut sends,
        None,
    );
    Host::client_frame(
        &mut c,
        &mut slot,
        "t",
        Some(&inp),
        Some(&buf),
        &mut sends,
        None,
    );
    assert_eq!(buf.generation(), 2);
}

/// Observe mirror: the shared handle the slot thread's observe hook
/// updates, so a `run_client` thread is observable without owning the
/// client. `(loop_cycle, reboot_timer)` for packet tests.
type Seen = std::sync::Arc<std::sync::Mutex<(i32, i32)>>;

fn seen() -> (Seen, Seen) {
    let s = std::sync::Arc::new(std::sync::Mutex::new((0, 0)));
    (std::sync::Arc::clone(&s), s)
}

// `Client` is !Send (its `present` target is a `Box<dyn PresentTarget>`),
// so like host-play's slot threads the tests build the client *inside*
// the spawned closure and only move Send handles across the boundary.

#[test]
fn idle_slot_parks_between_packets_and_wakes_on_one() {
    use std::io::Write;
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (wake, park) = crate::slot_io::wake_channel();
    let (mirror, seen) = seen();
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = Arc::clone(&stop);
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let handle = thread::spawn(move || {
        let mut c = prepare_client(
            cfg(),
            1,
            Arc::new(Cache::default()),
            Arc::new(vec![]),
            Vec::new(),
        );
        let stream =
            client::io::ClientStream::connect(&addr.ip().to_string(), addr.port()).unwrap();
        c.stream = Some(stream);
        c.ingame = true;
        // The login handshake leaves the packet decoder mid-frame
        // (`ptype == -1` → read the next header byte); without it a
        // fresh client misreads the first socket byte as a header.
        c.ptype = -1;
        Host::run_client(
            &mut c,
            "idle",
            ProfileSettings::default(),
            Arc::new(AtomicBool::new(true)),
            Arc::new(AtomicBool::new(true)),
            Arc::new(Mutex::new("strength".to_string())),
            None,
            None,
            Some(Arc::new(park)),
            Arc::new(api::run_policy::RunPolicyOverrideCell::new()),
            |c, _, _, _| {
                let mut v = mirror.lock().unwrap();
                v.0 = c.loop_cycle;
                v.1 = c.reboot_timer;
                false
            },
            |_| stop2.load(Ordering::Relaxed),
            |_| RandomClaim::Host,
        );
        done_tx.send(()).unwrap();
    });
    let (mut server, _) = listener.accept().unwrap();

    // First park happens after the opening tick; a quiet window must
    // not advance mainloop at all (no packet, no control, no timer yet).
    thread::sleep(Duration::from_millis(120));
    let before = *seen.lock().unwrap();
    thread::sleep(Duration::from_millis(120));
    let after = *seen.lock().unwrap();
    assert_eq!(
        after, before,
        "idle slot must not call mainloop between packet arrivals"
    );

    // A server packet (UPDATE_REBOOT_TIMER, two payload bytes) must
    // wake the park and apply within a frame.
    server.write_all(&[89, 0, 10]).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let (_, reboot) = *seen.lock().unwrap();
        if reboot > 0 {
            break;
        }
        assert!(Instant::now() < deadline, "packet never applied");
        thread::sleep(Duration::from_millis(5));
    }

    stop.store(true, Ordering::Relaxed);
    wake.wake();
    done_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("stop control must return a parked slot");
    handle.join().unwrap();
}

#[test]
fn focused_slot_keeps_the_twenty_ms_cadence() {
    force_cpu_backend();
    let (mirror, seen) = seen();
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = Arc::clone(&stop);
    let inp = SlotInput::new();
    inp.set_enabled(true);
    let handle = thread::spawn(move || {
        let mut c = prepare_client(
            cfg(),
            1,
            Arc::new(Cache::default()),
            Arc::new(vec![]),
            Vec::new(),
        );
        c.set_draw(true);
        Host::run_client(
            &mut c,
            "focused",
            ProfileSettings::default(),
            Arc::new(AtomicBool::new(true)),
            Arc::new(AtomicBool::new(true)),
            Arc::new(Mutex::new("strength".to_string())),
            Some(inp),
            None,
            None,
            Arc::new(api::run_policy::RunPolicyOverrideCell::new()),
            |c, _, _, _| {
                mirror.lock().unwrap().0 = c.loop_cycle;
                false
            },
            |_| stop2.load(Ordering::Relaxed),
            |_| RandomClaim::Host,
        );
    });
    // Gate on the count, not a fixed sleep: a parked slot (600 ms
    // timeout, no socket/control) manages ≤8 ticks in 5 s, so reaching
    // 12 proves the frame loop is running no matter how contended the
    // test machine is.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let cycles = seen.lock().unwrap().0;
        if cycles >= 12 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "focused slot never entered the frame loop, {cycles} ticks in 5 s"
        );
        thread::sleep(Duration::from_millis(10));
    }
    // Then the rate: the 20 ms cadence yields dozens per 300 ms; a
    // parked slot would yield ≤1.
    let t1 = seen.lock().unwrap().0;
    thread::sleep(Duration::from_millis(300));
    let t2 = seen.lock().unwrap().0;
    assert!(
        t2 >= t1 + 3,
        "focused slot must tick at ~20 ms, {t1} -> {t2} over 300 ms"
    );
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
}

#[test]
fn busy_observe_keeps_the_slot_on_the_frame_loop() {
    let (mirror, seen) = seen();
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = Arc::clone(&stop);
    let handle = thread::spawn(move || {
        let mut c = prepare_client(
            cfg(),
            1,
            Arc::new(Cache::default()),
            Arc::new(vec![]),
            Vec::new(),
        );
        Host::run_client(
            &mut c,
            "scripted",
            ProfileSettings::default(),
            Arc::new(AtomicBool::new(true)),
            Arc::new(AtomicBool::new(true)),
            Arc::new(Mutex::new("strength".to_string())),
            None,
            None,
            None,
            Arc::new(api::run_policy::RunPolicyOverrideCell::new()),
            |c, _, _, _| {
                mirror.lock().unwrap().0 = c.loop_cycle;
                true // script/cheat/nav work due: never park
            },
            |_| stop2.load(Ordering::Relaxed),
            |_| RandomClaim::Host,
        );
    });
    // Same gate as the focused test: ≥12 ticks in 5 s is unreachable
    // for a parked slot (≤8), so this proves a busy slot never parks.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let cycles = seen.lock().unwrap().0;
        if cycles >= 12 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "a busy (scripted) slot must keep ticking, {cycles} ticks in 5 s"
        );
        thread::sleep(Duration::from_millis(10));
    }
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
}

#[test]
fn watch_only_sidecar_parks_wakes_once_per_second_and_paints() {
    force_cpu_backend();
    let (wake, park) = crate::slot_io::wake_channel();
    let (mirror, seen) = seen();
    let buf = FrameBuf::new();
    let buf2 = Arc::clone(&buf);
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = Arc::clone(&stop);
    let handle = thread::spawn(move || {
        let mut c = prepare_client(
            cfg(),
            1,
            Arc::new(Cache::default()),
            Arc::new(vec![]),
            Vec::new(),
        );
        c.set_draw(true);
        c.ingame = true;
        c.scene_state = 2;
        Host::run_client(
            &mut c,
            "sidecar",
            ProfileSettings::default(),
            Arc::new(AtomicBool::new(true)),
            Arc::new(AtomicBool::new(true)),
            Arc::new(Mutex::new("strength".to_string())),
            None,
            Some(buf2),
            Some(Arc::new(park)),
            Arc::new(api::run_policy::RunPolicyOverrideCell::new()),
            |c, _, _, _| {
                mirror.lock().unwrap().0 = c.loop_cycle;
                false
            },
            |_| stop2.load(Ordering::Relaxed),
            |_| RandomClaim::Host,
        );
    });
    // The rising edge paints the first frame, then the slot parks.
    // Poll for the first paint — no fixed sleep, because under parallel
    // `--workspace` load the slot thread can be delayed arbitrarily.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if buf.generation() >= 1 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "watch-only sidecar never painted its first frame"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let g0 = buf.generation();
    assert!(g0 >= 1, "watch-only rising edge must paint the first frame");
    // The second paint is gated on a wall-clock second since the first
    // (the elapsed-time paint decision), so it must still arrive on the
    // 1 s park — poll for it with a generous deadline.
    let second = Instant::now();
    let deadline = second + Duration::from_secs(10);
    loop {
        if buf.generation() >= g0 + 2 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "watch-only sidecar never repainted (gen stuck at {})",
            buf.generation()
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        second.elapsed() >= Duration::from_millis(900),
        "watch-only repaint must wait a wall-clock second, not every tick"
    );
    // Parking proof: a 20 ms loop would tick ~100× in 2.2 s; the 1 s
    // park stays near 2–3 wakes.
    let t1 = seen.lock().unwrap().0;
    thread::sleep(Duration::from_secs(2) + Duration::from_millis(200));
    let t2 = seen.lock().unwrap().0;
    let ticks = t2 - t1;
    assert!(
        (1..=5).contains(&ticks),
        "watch-only sidecar must wake ~1×/s, not 50×/s: ticks {t1} -> {t2} ({ticks} in 2.2 s)"
    );
    stop.store(true, Ordering::Relaxed);
    wake.wake();
    handle.join().unwrap();
}

#[test]
fn stop_control_wakes_a_parked_slot_and_returns() {
    let (wake, park) = crate::slot_io::wake_channel();
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = Arc::clone(&stop);
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let handle = thread::spawn(move || {
        let mut c = prepare_client(
            cfg(),
            1,
            Arc::new(Cache::default()),
            Arc::new(vec![]),
            Vec::new(),
        );
        Host::run_client(
            &mut c,
            "parked",
            ProfileSettings::default(),
            Arc::new(AtomicBool::new(true)),
            Arc::new(AtomicBool::new(true)),
            Arc::new(Mutex::new("strength".to_string())),
            None,
            None,
            Some(Arc::new(park)),
            Arc::new(api::run_policy::RunPolicyOverrideCell::new()),
            |_, _, _, _| false,
            |_| stop2.load(Ordering::Relaxed),
            |_| RandomClaim::Host,
        );
        done_tx.send(()).unwrap();
    });
    // Let the slot park, then stop + kick must return it promptly.
    thread::sleep(Duration::from_millis(100));
    stop.store(true, Ordering::Relaxed);
    wake.wake();
    done_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("a stop control must wake a parked slot");
    handle.join().unwrap();
}

#[test]
fn draw_kick_wakes_a_parked_slot_into_the_frame_loop() {
    force_cpu_backend();
    let (wake, park) = crate::slot_io::wake_channel();
    let (mirror, seen) = seen();
    let inp = SlotInput::new();
    // The panel mirrors focus into the slot thread via the observe
    // hook (per_frame → `set_draw`, capture → `input.set_enabled`).
    // The kick flips the slot from draw-off to focused+capture, which
    // must wake the park into the 20 ms frame loop.
    let want = Arc::new(AtomicBool::new(false));
    let want2 = Arc::clone(&want);
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = Arc::clone(&stop);
    let handle = thread::spawn(move || {
        let mut c = prepare_client(
            cfg(),
            1,
            Arc::new(Cache::default()),
            Arc::new(vec![]),
            Vec::new(),
        );
        Host::run_client(
            &mut c,
            "kicked",
            ProfileSettings::default(),
            Arc::new(AtomicBool::new(true)),
            Arc::new(AtomicBool::new(true)),
            Arc::new(Mutex::new("strength".to_string())),
            Some(Arc::clone(&inp)),
            None,
            Some(Arc::new(park)),
            Arc::new(api::run_policy::RunPolicyOverrideCell::new()),
            |c, _, _, _| {
                let on = want2.load(Ordering::Relaxed);
                c.set_draw(on);
                inp.set_enabled(on);
                mirror.lock().unwrap().0 = c.loop_cycle;
                false
            },
            |_| stop2.load(Ordering::Relaxed),
            |_| RandomClaim::Host,
        );
    });
    // Opening tick, then parked (draw off): a quiet window must not
    // advance the tick count.
    thread::sleep(Duration::from_millis(80));
    let before = seen.lock().unwrap().0;
    assert!(before <= 1, "draw-off slot must park, got {before} ticks");
    // The panel flips its draw intent and kicks the parked thread.
    want.store(true, Ordering::Relaxed);
    wake.wake();
    // Gate on the tick count, not a fixed deadline: under parallel load
    // the kicked slot can take a while to run 8 ticks, so give it a
    // generous window (a still-parked slot would be far slower).
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let t = seen.lock().unwrap().0;
        if t >= before + 8 {
            break; // the kicked slot is ticking again
        }
        assert!(
            Instant::now() < deadline,
            "kicked slot never resumed ticking, {t} ticks in 10 s"
        );
        thread::sleep(Duration::from_millis(10));
    }
    // Then the rate: ≥3 ticks per 300 ms is the ~20 ms cadence (a
    // parked slot yields ≤1), so the kick really re-entered the frame
    // loop instead of just ticking slowly.
    let t1 = seen.lock().unwrap().0;
    thread::sleep(Duration::from_millis(300));
    let t2 = seen.lock().unwrap().0;
    assert!(
        t2 >= t1 + 3,
        "kicked slot must tick at ~20 ms, {t1} -> {t2} over 300 ms"
    );
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
}

#[test]
fn spurious_kick_does_not_busy_loop_a_parked_slot() {
    let (wake, park) = crate::slot_io::wake_channel();
    let (mirror, seen) = seen();
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = Arc::clone(&stop);
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let handle = thread::spawn(move || {
        let mut c = prepare_client(
            cfg(),
            1,
            Arc::new(Cache::default()),
            Arc::new(vec![]),
            Vec::new(),
        );
        Host::run_client(
            &mut c,
            "kicked-idle",
            ProfileSettings::default(),
            Arc::new(AtomicBool::new(true)),
            Arc::new(AtomicBool::new(true)),
            Arc::new(Mutex::new("strength".to_string())),
            None,
            None,
            Some(Arc::new(park)),
            Arc::new(api::run_policy::RunPolicyOverrideCell::new()),
            |c, _, _, _| {
                mirror.lock().unwrap().0 = c.loop_cycle;
                false // stays idle after the kick
            },
            |_| stop2.load(Ordering::Relaxed),
            |_| RandomClaim::Host,
        );
        done_tx.send(()).unwrap();
    });
    thread::sleep(Duration::from_millis(80));
    let before = seen.lock().unwrap().0;
    // Two kicks while the slot stays idle: each must wake it once and
    // re-park — an undrained control fd would re-fire every park and
    // spin the thread.
    wake.wake();
    wake.wake();
    thread::sleep(Duration::from_millis(120));
    let after = seen.lock().unwrap().0;
    assert!(
        after <= before + 2,
        "a spurious kick must not busy-loop a parked slot, ticks {before} -> {after}"
    );
    stop.store(true, Ordering::Relaxed);
    wake.wake();
    done_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("stop must still return the slot");
    handle.join().unwrap();
}

#[test]
fn park_prefers_socket_when_control_and_socket_both_ready() {
    use std::io::Write;
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let mut c = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let stream = client::io::ClientStream::connect(&addr.ip().to_string(), addr.port()).unwrap();
    c.stream = Some(stream);
    let (mut server, _) = listener.accept().unwrap();
    let (wake, park_end) = crate::slot_io::wake_channel();

    server.write_all(&[1]).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let h = stream_wait_handle(c.stream.as_ref().unwrap());
        if slot_io::wait_readable(&[h], Duration::from_millis(0))[0] {
            break;
        }
        assert!(Instant::now() < deadline, "socket never ready");
        thread::sleep(Duration::from_millis(5));
    }
    wake.wake();
    assert_eq!(
        park(&c, Some(&park_end), true, Duration::from_millis(1000)),
        ParkWake::Socket,
        "socket must win when both control and socket are ready"
    );
}

#[test]
fn park_suppresses_socket_when_poll_socket_false() {
    use std::io::Write;
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let mut c = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let stream = client::io::ClientStream::connect(&addr.ip().to_string(), addr.port()).unwrap();
    c.stream = Some(stream);
    let (mut server, _) = listener.accept().unwrap();
    let (wake, park_end) = crate::slot_io::wake_channel();

    server.write_all(&[1]).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let h = stream_wait_handle(c.stream.as_ref().unwrap());
        if slot_io::wait_readable(&[h], Duration::from_millis(0))[0] {
            break;
        }
        assert!(Instant::now() < deadline, "socket never ready");
        thread::sleep(Duration::from_millis(5));
    }
    // Socket is readable, but poll_socket=false (stalled path): only control
    // is waited. Without a kick, park must time out instead of busy-spinning.
    let start = Instant::now();
    assert_eq!(
        park(&c, Some(&park_end), false, Duration::from_millis(50)),
        ParkWake::Timeout
    );
    assert!(start.elapsed() >= Duration::from_millis(30));
    wake.wake();
    assert_eq!(
        park(&c, Some(&park_end), false, Duration::from_millis(1000)),
        ParkWake::Control,
        "with socket suppressed, a control kick must still wake as Control"
    );
}

#[test]
fn park_no_fds_sleeps_timeout() {
    let c = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let start = Instant::now();
    assert_eq!(
        park(&c, None, true, Duration::from_millis(60)),
        ParkWake::Timeout
    );
    assert!(start.elapsed() >= Duration::from_millis(40));
}

/// The guardian kicks from `client_frame` right after `after_drain`
/// (Task 4): a dialog NPC on a fresh snapshot gets a Talk-to through
/// the real `Client` driver, and the slot latches `in_flight`.
#[test]
fn client_frame_kicks_guardian_after_drain() {
    use client::client::{ClientNpc, ClientPlayer};
    use client::config::NpcType;

    let mut c = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    // Attached + ingame scene-2 so the `Interactions` preconditions
    // pass; the guardian then sends through the real client driver.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let stream =
        client::io::ClientStream::connect(&addr.ip().to_string(), addr.port()).expect("connect");
    std::mem::forget(listener);
    c.stream = Some(stream);
    c.ingame = true;
    c.scene_state = 2;
    c.map_build_base_x = 0;
    c.map_build_base_z = 0;
    c.self_slot = 0;

    // The local player and a dialog NPC that owns us (overhead name).
    let mut lp = ClientPlayer::at(0, 0);
    lp.name = Some("Test".to_string());
    c.local_player = Some(lp);
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        cache.npcs.push(NpcType {
            id: 0,
            name: "Genie".to_string(),
            op: vec![Some("Talk-to".to_string())],
            ..Default::default()
        });
    }
    let mut npc = ClientNpc::at(0, 0);
    npc.r#type = Some(0);
    npc.entity.chat_message = Some("Greetings Test!".to_string());
    c.npc[0] = Some(Box::new(npc));
    c.npc_ids[0] = 0;
    c.npc_count = 1;
    // Dirty families so `after_drain` rebuilds the snapshot.
    c.gens.npc = 1;
    c.gens.player = 1;
    c.gens.player_info = 1;
    c.gens.scene = 1;

    let mut slot = SlotLoop::new();
    let mut sends = 0u32;
    Host::client_frame(&mut c, &mut slot, "t", None, None, &mut sends, None);
    assert!(
        slot.guardian.in_flight,
        "client_frame must run the guardian on a dialog NPC"
    );
    assert!(
        c.out.pos > 0,
        "the Talk-to went out on the real client driver"
    );
}

fn maze_client(revision: client::client::ClientRevision) -> Client {
    let mut client = Client::from_shared_with_revision(
        cfg(),
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
        revision,
    );
    ingame_scene2(&mut client);
    client.map_build_base_x = 45 * 64 - 10;
    client.map_build_base_z = 71 * 64 - 10;
    client.gens.scene += 1;
    client
}

fn establish_maze_hold(client: &mut Client, slot: &mut SlotLoop, sends: &mut u32) {
    let status = Host::client_frame(client, slot, "maze", None, None, sends, None);
    assert_eq!(status.kind, Some(RandomKind::Maze));
    assert_eq!(status.claim, RandomClaim::Host);
    assert!(status.hold, "enabled host-owned maze must hold");
    client.out.pos = 0;
    client.logout_timer = 0;
}

#[test]
fn held_host_owned_289_random_event_interrupts_idle_timer_before_threshold() {
    let mut client = maze_client(client::client::ClientRevision::R289);
    let mut slot = SlotLoop::new();
    let mut sends = 0;
    establish_maze_hold(&mut client, &mut slot, &mut sends);

    client.shell.idle_cycles = 4500;
    Host::client_frame(&mut client, &mut slot, "maze", None, None, &mut sends, None);

    assert_eq!(client.shell.idle_cycles, 1);
    assert_eq!(client.logout_timer, 0);
    assert_eq!(client.out.pos, 0, "held frame must not emit IDLE_TIMER");
}

#[test]
fn disabled_released_and_ordinary_289_idle_still_cross_threshold() {
    fn assert_idle_timer(client: &Client) {
        assert_eq!(client.shell.idle_cycles, 4001);
        assert_eq!(client.logout_timer, 250);
        assert_eq!(client.out.pos, 1);
        assert_eq!(
            client.out.data()[0],
            ClientProt289::IDLE_TIMER.id as u8,
            "289 IDLE_TIMER opcode"
        );
    }

    let mut disabled = maze_client(client::client::ClientRevision::R289);
    let mut disabled_slot = SlotLoop::new();
    let mut sends = 0;
    establish_maze_hold(&mut disabled, &mut disabled_slot, &mut sends);
    disabled_slot.random_events.store(false, Ordering::Relaxed);
    disabled.shell.idle_cycles = 4500;
    Host::client_frame(
        &mut disabled,
        &mut disabled_slot,
        "disabled",
        None,
        None,
        &mut sends,
        None,
    );
    assert_idle_timer(&disabled);

    let mut released = maze_client(client::client::ClientRevision::R289);
    let mut released_slot = SlotLoop::new();
    establish_maze_hold(&mut released, &mut released_slot, &mut sends);
    released.map_build_base_x = 0;
    released.map_build_base_z = 0;
    released.gens.player += 1;
    released.gens.player_info += 1;
    released.shell.idle_cycles = 4500;
    let released_status = Host::client_frame(
        &mut released,
        &mut released_slot,
        "released",
        None,
        None,
        &mut sends,
        None,
    );
    assert_idle_timer(&released);
    assert!(!released_status.hold, "released maze must clear the hold");

    let mut relogged = maze_client(client::client::ClientRevision::R289);
    let mut relogged_slot = SlotLoop::new();
    establish_maze_hold(&mut relogged, &mut relogged_slot, &mut sends);
    relogged.gens.session += 1;
    relogged.map_build_base_x = 0;
    relogged.map_build_base_z = 0;
    relogged.gens.player += 1;
    relogged.gens.player_info += 1;
    relogged.shell.idle_cycles = 4500;
    let relogged_status = Host::client_frame(
        &mut relogged,
        &mut relogged_slot,
        "relogged",
        None,
        None,
        &mut sends,
        None,
    );
    assert_idle_timer(&relogged);
    assert!(!relogged_status.hold, "a new session must not inherit hold");

    let mut ordinary = Client::from_shared_with_revision(
        cfg(),
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
        client::client::ClientRevision::R289,
    );
    ingame_scene2(&mut ordinary);
    ordinary.shell.idle_cycles = 4500;
    let mut ordinary_slot = SlotLoop::new();
    Host::client_frame(
        &mut ordinary,
        &mut ordinary_slot,
        "ordinary",
        None,
        None,
        &mut sends,
        None,
    );
    assert_idle_timer(&ordinary);
}

#[test]
fn held_274_random_event_leaves_native_idle_state_untouched() {
    let mut client = maze_client(client::client::ClientRevision::R274);
    let mut slot = SlotLoop::new();
    let mut sends = 0;
    establish_maze_hold(&mut client, &mut slot, &mut sends);

    client.shell.idle_cycles = 4500;
    Host::client_frame(
        &mut client,
        &mut slot,
        "maze-274",
        None,
        None,
        &mut sends,
        None,
    );

    assert_eq!(client.shell.idle_cycles, 4500);
    assert_eq!(client.logout_timer, 0);
    assert_eq!(client.out.pos, 0);
}

/// Lamp auto is live-mirrored like `random_events`: spawn with auto
/// off, flip the shared atomic mid-session, and the next guardian
/// tick rubs without a respawn.
#[test]
fn lamp_auto_live_mirror_reaches_guardian_without_respawn() {
    const LAMP_OBJ: i32 = 2528;

    let mut c = prepare_client(
        cfg(),
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let stream =
        client::io::ClientStream::connect(&addr.ip().to_string(), addr.port()).expect("connect");
    std::mem::forget(listener);
    c.stream = Some(stream);
    ingame_scene2(&mut c);
    c.map_build_base_x = 0;
    c.map_build_base_z = 0;
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        while cache.objs.len() <= LAMP_OBJ as usize {
            cache.objs.push(client::config::ObjType::default());
        }
        cache.objs[LAMP_OBJ as usize] = client::config::ObjType {
            id: LAMP_OBJ,
            iop: [None, None, None, Some("Rub".into()), None],
            ..Default::default()
        };
    }
    c.side_icon[3] = 300;
    c.set_iface(
        300,
        IfType {
            id: 300,
            layer_id: 300,
            children: Some(vec![301]),
            ..Default::default()
        },
    );
    c.set_iface(
        301,
        IfType {
            id: 301,
            layer_id: 300,
            r#type: client::config::if_type::ComponentType::TYPE_INV,
            obj_ops: true,
            ..Default::default()
        },
    );
    c.set_iface_mut(
        301,
        IfTypeMut {
            link_obj_type: Some(vec![LAMP_OBJ + 1, 0]),
            link_obj_number: Some(vec![1, 0]),
            ..Default::default()
        },
    );
    c.gens.inv = 1;
    c.gens.iface = 1;
    c.gens.scene = 1;
    c.gens.player = 1;
    c.gens.player_info = 1;

    let lamp_auto = Arc::new(AtomicBool::new(false));
    let lamp_skill = Arc::new(Mutex::new("strength".to_string()));
    let mut slot = SlotLoop {
        settings: ProfileSettings {
            lamp_auto: false,
            ..ProfileSettings::default()
        },
        random_events: Arc::new(AtomicBool::new(true)),
        lamp_auto: Arc::clone(&lamp_auto),
        lamp_skill: Arc::clone(&lamp_skill),
        ..SlotLoop::new()
    };
    let mut sends = 0u32;
    Host::client_frame(&mut c, &mut slot, "t", None, None, &mut sends, None);
    assert!(
        c.out.pos == 0,
        "lamp_auto off at spawn: guardian must not rub"
    );

    lamp_auto.store(true, Ordering::Relaxed);
    c.gens.player = 2;
    c.gens.player_info = 2;
    Host::client_frame(&mut c, &mut slot, "t", None, None, &mut sends, None);
    assert!(
        c.out.pos > 0,
        "lamp_auto flipped live: next guardian tick must rub"
    );
}
