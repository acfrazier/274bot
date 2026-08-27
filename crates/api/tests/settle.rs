// Task 10: the pollable settle model — `Outcome`/`SettleOptions`/`Settle`
// and the eleven evidence predicates over `ReadContext` (the m8aq
// `Settle`/`Evidence`/`Outcome`). Fixture clients attach via a local
// socket pair (the interact.rs pattern) so `poll` sees `attached && ingame`
// unless a test drops the stream; `fresh_snap` bumps every gen and rebuilds
// every family into a new snapshot (tick 1), `bump_rebuild` advances an
// existing snapshot's tick.

use api::settle::{
    arrived, engaged, inventory_changed, item_delta, modal_closed, modal_opened, option_gone,
    said, scene_ready, server_refused, xp_gained, Evidence, Outcome, Settle, SettleOptions,
};
use api::snapshot::{
    ActorKind, ActorTargetView, GameSnapshot, ItemContainer, ReadContext, WorldTile,
};
use client::client::{Client, ClientConfig, ClientNpc, ClientPlayer};
use client::config::if_type::{ComponentType, IfType};
use client::config::LocType;
use client::io::{ClientStream, ServerProt};
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

/// An attached, ingame client with the scene built at base (3200, 3200).
fn scene_client() -> Client {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let stream = ClientStream::connect(&addr.ip().to_string(), addr.port()).expect("connect");
    // Keep the listener alive so the connect stays established.
    std::mem::forget(listener);
    let mut c = Client::new(cfg());
    c.stream = Some(stream);
    c.ingame = true;
    c.scene_state = 2;
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c
}

/// Plant the local player's route + body at scene (x, z); the actor's
/// world tile lands on (3200 + x, 3200 + z).
fn plant_player(c: &mut Client, x: i32, z: i32) {
    let mut lp = ClientPlayer::at(x, z);
    lp.entity.x = x * 128 + 64;
    lp.entity.z = z * 128 + 64;
    c.local_player = Some(lp);
}

/// Bump every gen and rebuild every family into a fresh snapshot (tick 1).
fn fresh_snap(c: &mut Client) -> GameSnapshot {
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    let mut snap = GameSnapshot::new();
    snap.rebuild(c);
    snap
}

/// Plant the player and take the fresh snapshot.
fn snap_at(c: &mut Client, x: i32, z: i32) -> GameSnapshot {
    plant_player(c, x, z);
    fresh_snap(c)
}

/// Bump every gen and rebuild into the existing snapshot (tick + 1).
fn bump_rebuild(c: &mut Client, snap: &mut GameSnapshot) {
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    snap.rebuild(c);
}

fn set_iface(c: &mut Client, id: usize, com: IfType) {
    if c.ifaces.len() <= id {
        c.ifaces.resize(id + 1, None);
    }
    c.ifaces[id] = Some(com);
}

/// The inventory tab (side tab 3) with `(stored, count)` slots; stored ids
/// are `obj id + 1` (0 = empty), so a stored 4 reads as obj id 3.
fn plant_inv(c: &mut Client, stored: &[i32], counts: &[i32]) {
    set_iface(
        c,
        500,
        IfType {
            id: 500,
            r#type: ComponentType::TYPE_INV,
            link_obj_type: Some(stored.to_vec()),
            link_obj_number: Some(counts.to_vec()),
            obj_ops: true,
            ..Default::default()
        },
    );
    c.side_icon[3] = 500;
}

/// The worn-items tab (side tab 4) with its slots (stored 7 = obj id 6).
fn plant_equipment(c: &mut Client, stored: &[i32], counts: &[i32]) {
    set_iface(
        c,
        711,
        IfType {
            id: 711,
            r#type: ComponentType::TYPE_INV,
            link_obj_type: Some(stored.to_vec()),
            link_obj_number: Some(counts.to_vec()),
            ..Default::default()
        },
    );
    c.side_icon[4] = 711;
}

/// A wall loc (id 1, "Door") at scene (3, 4) → world tile (3203, 3204).
fn plant_loc(c: &mut Client, op: Option<&str>) {
    let typecode = 0x4000_0000 + (1 << 14) + 3 + (4 << 7);
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        while cache.locs.len() <= 1 {
            cache.locs.push(LocType::default());
        }
        cache.locs[1] = LocType {
            id: 1,
            name: "Door".into(),
            op: vec![op.map(str::to_string), None, None, None, None],
            ..Default::default()
        };
    }
    c.world
        .set_wall(0, 3, 4, 0, 0, 0, typecode, 1 << 6, 0, 0, 0, 0);
}

/// One live npc (type 9) in slot 7 with the given health.
fn plant_npc(c: &mut Client, health: i32) {
    let mut npc = ClientNpc::default();
    npc.entity.x = 50 * 128;
    npc.entity.z = 50 * 128;
    npc.entity.health = health;
    npc.entity.total_health = health;
    npc.r#type = Some(9);
    c.npc[7] = Some(npc);
    c.npc_ids[0] = 7;
    c.npc_count = 1;
}

/// Evaluate one evidence predicate against `(now, before)` contexts.
fn evidence_holds(evidence: Evidence, now: &GameSnapshot, before: &GameSnapshot) -> bool {
    evidence(&ReadContext::new(now), &ReadContext::new(before))
}

/// `poll` watches: `None` while no arm fired and the budget holds, `Matched`
/// when an arm fires, `Expired` when the tick budget lapses.
#[test]
fn settle_poll_matches_arrived_then_expires() {
    let mut c = scene_client();
    let before = snap_at(&mut c, 10, 10);
    let mut now = snap_at(&mut c, 20, 12);
    bump_rebuild(&mut c, &mut now);
    let dest = WorldTile { x: 3220, z: 3212, level: 0 };

    let arms: [(&str, Evidence); 1] = [("arrived", arrived(dest, 0))];
    let options = SettleOptions {
        arms: &arms,
        since: None,
        budget_ticks: 3,
        budget_ms: None,
    };
    let mut settle = Settle::new(options, ReadContext::new(&before));
    assert!(
        settle.poll(ReadContext::new(&before)).is_none(),
        "still watching before arrival"
    );
    match settle.poll(ReadContext::new(&now)) {
        Some(Outcome::Matched {
            arm,
            tick,
            now: _,
            before: _,
        }) => {
            assert_eq!(arm, "arrived");
            assert_eq!(tick, now.tick() as u64);
        }
        _ => panic!("expected Matched on the arrival tick"),
    }

    // The same arm with a lapsed budget expires instead.
    let arms: [(&str, Evidence); 1] = [("arrived", arrived(dest, 0))];
    let options = SettleOptions {
        arms: &arms,
        since: None,
        budget_ticks: 0,
        budget_ms: None,
    };
    let mut settle = Settle::new(options, ReadContext::new(&before));
    match settle.poll(ReadContext::new(&before)) {
        Some(Outcome::Expired { tick, .. }) => assert_eq!(tick, before.tick() as u64),
        _ => panic!("expected Expired on the lapsed tick budget"),
    }
}

/// A disconnected / not-ingame snapshot expires the watch.
#[test]
fn settle_poll_expires_on_disconnect_or_not_ingame() {
    let mut c = scene_client();
    let before = snap_at(&mut c, 10, 10);
    c.stream = None;
    c.ingame = false;
    let now = snap_at(&mut c, 10, 10);
    let arms: [(&str, Evidence); 1] = [(
        "arrived",
        arrived(WorldTile { x: 3220, z: 3212, level: 0 }, 0),
    )];
    let options = SettleOptions {
        arms: &arms,
        since: None,
        budget_ticks: 5,
        budget_ms: None,
    };
    let mut settle = Settle::new(options, ReadContext::new(&before));
    match settle.poll(ReadContext::new(&now)) {
        Some(Outcome::Expired { .. }) => {}
        _ => panic!("expected Expired when disconnected"),
    }
}

/// The wall-clock backstop (the ms budget) expires the watch too.
#[test]
fn settle_poll_expires_on_the_ms_budget() {
    let mut c = scene_client();
    let before = snap_at(&mut c, 10, 10);
    let now = snap_at(&mut c, 10, 10);
    let arms: [(&str, Evidence); 1] = [(
        "arrived",
        arrived(WorldTile { x: 3220, z: 3212, level: 0 }, 0),
    )];
    let options = SettleOptions {
        arms: &arms,
        since: None,
        budget_ticks: u32::MAX,
        budget_ms: Some(0),
    };
    let mut settle = Settle::new(options, ReadContext::new(&before));
    match settle.poll(ReadContext::new(&now)) {
        Some(Outcome::Expired { .. }) => {}
        _ => panic!("expected Expired on the lapsed ms budget"),
    }
}

/// `arrived` needs the level and the Chebyshev radius to hold.
#[test]
fn arrived_gates_on_level_and_radius() {
    let mut c = scene_client();
    let far = snap_at(&mut c, 10, 10);
    let near = snap_at(&mut c, 11, 12);
    let dest = WorldTile { x: 3220, z: 3212, level: 0 };
    assert!(!evidence_holds(arrived(dest, 0), &far, &far), "not at the tile");
    assert!(
        !evidence_holds(arrived(dest, 8), &near, &far),
        "outside the radius"
    );
    assert!(evidence_holds(arrived(dest, 9), &near, &far), "within the radius");

    c.minusedlevel = 1;
    let upper = snap_at(&mut c, 20, 12);
    assert!(
        !evidence_holds(arrived(dest, 10), &upper, &far),
        "a level mismatch is never arrived"
    );
}

/// `item_delta` folds a container's `item_id` totals and compares the
/// signed move against `change`.
#[test]
fn item_delta_folds_container_totals() {
    let mut c = scene_client();
    plant_inv(&mut c, &[4, 0], &[1, 0]);
    let before = fresh_snap(&mut c);
    plant_inv(&mut c, &[4, 5, 0], &[2, 3, 0]);
    let now = fresh_snap(&mut c);
    // obj 3: 1 → 2; obj 4: 0 → 3.
    assert!(evidence_holds(
        item_delta(3, 1, ItemContainer::Inventory),
        &now,
        &before
    ));
    assert!(!evidence_holds(
        item_delta(3, 2, ItemContainer::Inventory),
        &now,
        &before
    ));
    assert!(evidence_holds(
        item_delta(4, 3, ItemContainer::Inventory),
        &now,
        &before
    ));
    assert!(!evidence_holds(
        item_delta(4, 4, ItemContainer::Inventory),
        &now,
        &before
    ));
    assert!(
        !evidence_holds(item_delta(3, -1, ItemContainer::Inventory), &now, &before),
        "a positive move never matches a negative change"
    );

    // A decrease matches a negative change (and not a positive one).
    assert!(evidence_holds(
        item_delta(3, -1, ItemContainer::Inventory),
        &before,
        &now
    ));
    assert!(!evidence_holds(
        item_delta(3, -2, ItemContainer::Inventory),
        &before,
        &now
    ));
    assert!(!evidence_holds(
        item_delta(3, 1, ItemContainer::Inventory),
        &before,
        &now
    ));

    // Containers are distinct reads.
    plant_equipment(&mut c, &[7], &[1]);
    let before_worn = fresh_snap(&mut c);
    plant_equipment(&mut c, &[7, 0], &[2, 0]);
    let now_worn = fresh_snap(&mut c);
    assert!(evidence_holds(
        item_delta(6, 1, ItemContainer::Equipment),
        &now_worn,
        &before_worn
    ));
    assert!(
        !evidence_holds(
            item_delta(6, 1, ItemContainer::Inventory),
            &now_worn,
            &before_worn
        ),
        "the equipment move is not an inventory move"
    );
}

/// `xp_gained` reads the skill's XP delta against `at_least`.
#[test]
fn xp_gained_reads_skill_xp() {
    let mut c = scene_client();
    c.stat_xp[0] = 100;
    let before = fresh_snap(&mut c);
    c.stat_xp[0] = 130;
    c.stat_xp[2] = 50;
    let now = fresh_snap(&mut c);
    assert!(evidence_holds(xp_gained(0, 30), &now, &before), "exactly at least");
    assert!(evidence_holds(xp_gained(0, 25), &now, &before));
    assert!(!evidence_holds(xp_gained(0, 31), &now, &before));
    assert!(
        !evidence_holds(xp_gained(1, 1), &now, &before),
        "skill 1 is unchanged"
    );
    assert!(evidence_holds(xp_gained(2, 50), &now, &before));
}

/// `engaged` fires on the local player's live target or on the target's
/// health dropping since `before`.
#[test]
fn engaged_detects_target_and_health_drop() {
    let mut c = scene_client();
    plant_npc(&mut c, 7);
    let before = snap_at(&mut c, 20, 20);
    let target = ActorTargetView {
        kind: ActorKind::Npc,
        index: 7,
    };
    assert!(
        !evidence_holds(engaged(target), &before, &before),
        "no target, same health"
    );

    // The local player is facing the npc.
    plant_player(&mut c, 20, 20);
    c.local_player.as_mut().unwrap().entity.face_entity = 7;
    let aimed = fresh_snap(&mut c);
    assert!(evidence_holds(engaged(target), &aimed, &before), "live target");

    // The npc loses health (player not facing it).
    plant_player(&mut c, 20, 20);
    c.npc[7].as_mut().unwrap().entity.health = 3;
    let wounded = fresh_snap(&mut c);
    assert!(
        evidence_holds(engaged(target), &wounded, &before),
        "health dropped"
    );
    assert!(
        !evidence_holds(engaged(target), &before, &wounded),
        "gaining health never engages"
    );
}

/// `modal_opened`/`modal_closed` match any root (or one named root).
#[test]
fn modal_opened_and_closed_match_roots() {
    let mut c = scene_client();
    let closed = snap_at(&mut c, 10, 10);
    c.main_modal_id = 100;
    let opened = snap_at(&mut c, 10, 10);
    assert!(evidence_holds(modal_opened(None), &opened, &closed), "any root");
    assert!(!evidence_holds(modal_opened(None), &closed, &closed));
    assert!(evidence_holds(modal_opened(Some(100)), &opened, &closed));
    assert!(!evidence_holds(modal_opened(Some(50)), &opened, &closed));
    assert!(!evidence_holds(modal_closed(None), &opened, &opened));
    assert!(evidence_holds(modal_closed(None), &closed, &closed));

    c.main_modal_id = -1;
    let closed_again = snap_at(&mut c, 10, 10);
    assert!(evidence_holds(modal_closed(Some(100)), &closed_again, &opened));
    assert!(
        !evidence_holds(modal_closed(Some(100)), &opened, &opened),
        "root 100 is still open"
    );
    assert!(
        evidence_holds(modal_closed(Some(50)), &opened, &opened),
        "root 50 is not among the open roots"
    );
}

/// `option_gone` matches when the loc (or its action) is gone from the
/// tile.
#[test]
fn option_gone_detects_loc_and_action_gone() {
    let mut c = scene_client();
    plant_loc(&mut c, Some("Open"));
    let before = fresh_snap(&mut c);
    let target = &before.locs()[0];
    assert_eq!(target.tile, WorldTile { x: 3203, z: 3204, level: 0 });

    let mut c2 = scene_client();
    plant_loc(&mut c2, Some("Open"));
    let now = fresh_snap(&mut c2);
    assert!(
        !evidence_holds(option_gone(target, "Open"), &now, &before),
        "action still present"
    );
    assert!(
        !evidence_holds(option_gone(target, "  open  "), &now, &before),
        "the match is trimmed case-insensitive"
    );

    let mut c3 = scene_client();
    plant_loc(&mut c3, Some("Use"));
    let now = fresh_snap(&mut c3);
    assert!(
        evidence_holds(option_gone(target, "Open"), &now, &before),
        "action gone"
    );

    let mut c4 = scene_client();
    let now = fresh_snap(&mut c4);
    assert!(
        evidence_holds(option_gone(target, "Open"), &now, &before),
        "loc gone entirely"
    );
}

/// `said` matches only chat lines newer than the `before` read.
#[test]
fn said_matches_new_chat_lines() {
    let mut c = scene_client();
    c.chat_text[0] = "first".into();
    let before = fresh_snap(&mut c);
    let mut now = fresh_snap(&mut c);
    c.chat_text[0] = "second".into();
    bump_rebuild(&mut c, &mut now);

    assert!(evidence_holds(said(&["sec"]), &now, &before), "new line");
    assert!(
        evidence_holds(said(&["SECOND", "hello"]), &now, &before),
        "case-insensitive, any phrase"
    );
    assert!(
        !evidence_holds(said(&["third"]), &now, &before),
        "no new line contains it"
    );
    assert!(
        !evidence_holds(said(&["first"]), &now, &before),
        "the before line is excluded"
    );
}

/// `server_refused` fires when the flag set at `before` drops without the
/// player standing on it.
#[test]
fn server_refused_detects_a_dropped_flag_off_destination() {
    let mut c = scene_client();
    c.minimap_flag_x = 14;
    c.minimap_flag_z = 15;
    let before = snap_at(&mut c, 10, 10);

    let mut c2 = scene_client();
    let now = snap_at(&mut c2, 10, 10);
    assert!(
        evidence_holds(server_refused(), &now, &before),
        "flag dropped off the destination"
    );

    let mut c3 = scene_client();
    let on_flag = snap_at(&mut c3, 14, 15);
    assert!(
        !evidence_holds(server_refused(), &on_flag, &before),
        "standing on the flag is not a refusal"
    );

    let mut c4 = scene_client();
    c4.minimap_flag_x = 14;
    c4.minimap_flag_z = 15;
    let still = snap_at(&mut c4, 10, 10);
    assert!(
        !evidence_holds(server_refused(), &still, &before),
        "flag still set"
    );

    let mut c5 = scene_client();
    let no_flag = snap_at(&mut c5, 10, 10);
    assert!(
        !evidence_holds(server_refused(), &no_flag, &no_flag),
        "no flag was set"
    );
}

/// `scene_ready` needs `scene_state == 2` and a nonzero build base.
#[test]
fn scene_ready_gates_on_state_and_base() {
    let mut c = scene_client();
    let ready = snap_at(&mut c, 10, 10);
    assert!(evidence_holds(scene_ready(), &ready, &ready));

    c.scene_state = 1;
    let loading = snap_at(&mut c, 10, 10);
    assert!(!evidence_holds(scene_ready(), &loading, &loading));

    c.scene_state = 2;
    c.map_build_base_x = 0;
    c.map_build_base_z = 0;
    let no_base = snap_at(&mut c, 10, 10);
    assert!(
        !evidence_holds(scene_ready(), &no_base, &no_base),
        "base 0 is never ready"
    );
}

/// `inventory_changed` fires on any slot/id/count delta.
#[test]
fn inventory_changed_detects_slot_moves() {
    let mut c = scene_client();
    plant_inv(&mut c, &[4, 5, 0], &[1, 2, 0]);
    let before = fresh_snap(&mut c);
    assert!(
        !evidence_holds(inventory_changed(), &before, &before),
        "same inventory"
    );

    plant_inv(&mut c, &[5, 4, 0], &[2, 1, 0]);
    let swapped = fresh_snap(&mut c);
    assert!(
        evidence_holds(inventory_changed(), &swapped, &before),
        "slots swapped"
    );

    plant_inv(&mut c, &[4, 5, 0], &[3, 2, 0]);
    let grew = fresh_snap(&mut c);
    assert!(
        evidence_holds(inventory_changed(), &grew, &before),
        "counts changed"
    );
}
