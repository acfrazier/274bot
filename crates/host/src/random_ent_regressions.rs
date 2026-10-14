// Included inside the existing `random/guardian.rs` tests module
// (`crates/host/src/random_tests.rs`) by the parent: this fragment relies on
// that module's private client/driver helpers (`new_client`, `ingame_scene`,
// `plant_player`, `plant_inv_named`, `plant_npc_typed_with_op`, `clear_npcs`,
// `tick_at`, `FakeDriver`) and on `Guardian`, `ProfileSettings`,
// `GameSnapshot` already in scope via `super::*`. The R1 replay fixtures below
// explicitly plant the local player's `p_opnpc` target. New cases cover visible
// foreign ownership and the stale-visible-slot 60-tick cap; stop retries stay
// fresh-only.
//
// Source: round 4 `guardian_skill_ent` t193 onset → t221 tool loss
// (`live/round4/guardian_skill_ent/result.json`): the trigger `~skill_macro_event 8`
// acked at t193, no hold rose, and the axe broke at t221 after six swings
// (`macro_event_ent.rs2:127-150`). The replay below plants the same shape
// (bronze axe 1351 held, nearby Ent NPC type 448/449 named after its tree
// with `Chop down`, `antimacro.npc:1279-1317`) for 28 ticks, then despawns.

/// Round-4 replay, oak Ent (type 448 `macro_ent_oak`, name `Oak`): the Guardian
/// must hold and stop the chop while the Ent stands, then release for resume
/// with the bronze axe still available.
#[test]
fn ent_oak_round4_replay_holds_stops_then_resumes_tool_kept() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_inv_named(&mut c, 1351, Some("Bronze axe"));
    plant_npc_typed_with_op(&mut c, 0, 448, "Oak", -1, None, "Chop down");
    c.local_player.as_mut().expect("local player").entity.face_entity = 0;
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    // Onset tick (round-4 t193): hold must rise and the chop must stop.
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(
        status.hold,
        "ent onset must hold the slot (round4 t193 had no hold)"
    );
    assert!(
        !drv.walks.is_empty(),
        "ent onset must send one stop walk to cancel the chop"
    );

    // t193→t221 (28 ticks to the round-4 tool loss): hold stays, tool stays.
    for tick in 1..28 {
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert!(
            status.hold,
            "ent must stay held at replay tick {tick} (tool lost at t221)"
        );
        assert!(
            snap.inventory().iter().any(|i| i.def.id == 1351),
            "bronze axe must stay available at replay tick {tick}"
        );
    }

    // Reversion/despawn (content 60-tick `npc_add`, `macro_event_ent.rs2:18`):
    // the slot clears and the hold releases for Gatherer resume.
    clear_npcs(&mut c);
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(
        !status.hold,
        "ent gone must release the hold so the Gatherer resumes"
    );
    assert!(
        snap.inventory().iter().any(|i| i.def.id == 1351),
        "bronze axe must still be available after the ent leaves"
    );
}

/// Second tree-name variant, willow Ent (type 449 `macro_ent_willow`, name
/// `Willow`): name alone is a tree name, so the same hold/stop/resume shape
/// must hold for every Ent type, not just oak.
#[test]
fn ent_willow_holds_stops_then_resumes_tool_kept() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_inv_named(&mut c, 1351, Some("Bronze axe"));
    plant_npc_typed_with_op(&mut c, 0, 449, "Willow", -1, None, "Chop down");
    c.local_player.as_mut().expect("local player").entity.face_entity = 0;

    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(status.hold, "willow ent onset must hold the slot");
    assert!(
        !drv.walks.is_empty(),
        "willow ent onset must send one stop walk to cancel the chop"
    );

    for tick in 1..28 {
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert!(
            status.hold,
            "willow ent must stay held at replay tick {tick}"
        );
    }
    assert!(
        snap.inventory().iter().any(|i| i.def.id == 1351),
        "bronze axe must stay available through the willow ent"
    );

    clear_npcs(&mut c);
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(
        !status.hold,
        "willow ent gone must release the hold so the Gatherer resumes"
    );
    assert!(
        snap.inventory().iter().any(|i| i.def.id == 1351),
        "bronze axe must still be available after the willow ent leaves"
    );
}

/// Distant Ent without a player target must not hold: the Ent at (0,0) is 50
/// tiles from the player at (50,50), beyond the measured fallback bound, and
/// the player targets nothing. Guards against holding another player's tree.
#[test]
fn ent_distant_foreign_ent_does_not_hold() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 50, 50);
    plant_inv_named(&mut c, 1351, Some("Bronze axe"));
    plant_npc_typed_with_op(&mut c, 0, 448, "Oak", -1, None, "Chop down");
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(
        !status.hold,
        "distant ent without a player target must not hold the slot"
    );
    assert!(drv.walks.is_empty(), "distant ent must send no stop walk");
}

/// A known target on a different actor suppresses fallback to a nearby Ent.
/// c3c8a1295 still claims the Ent by distance after failing the exact-target
/// comparison.
#[test]
fn ent_nearby_ent_is_not_claimed_when_local_targets_other_actor() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_inv_named(&mut c, 1351, Some("Bronze axe"));
    plant_npc_typed_with_op(&mut c, 0, 448, "Oak", -1, None, "Chop down");
    plant_npc_typed_with_op(&mut c, 1, 391, "River troll", -1, None, "Attack");
    c.local_player.as_mut().expect("local player").entity.face_entity = 1;
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(!status.hold, "a known non-Ent target must disable proximity fallback");
    assert!(drv.walks.is_empty(), "the unowned Ent must not send a stop walk");
}

/// A remote player's visible target identifies a nearby Ent as foreign even
/// when the local player's target is not yet visible.
#[test]
fn ent_nearby_remote_target_is_not_claimed() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_inv_named(&mut c, 1351, Some("Bronze axe"));
    plant_npc_typed_with_op(&mut c, 0, 448, "Oak", -1, None, "Chop down");
    let mut other = ClientPlayer::at(1, 0);
    other.entity.face_entity = 0;
    c.players[1] = Some(Box::new(other));
    c.player_ids[0] = 1;
    c.player_count = 1;
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    assert_eq!(snap.players().len(), 1);
    assert_eq!(snap.players()[0].actor.face_entity, 0, "foreign target must be observable");
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(!status.hold, "an Ent visibly targeted by another player must not be claimed");
    assert!(drv.walks.is_empty(), "the foreign Ent must not send a stop walk");
}

/// A slot that stays visible cannot extend the content lifetime or restart
/// the proximity hold after its 60 fresh ticks are exhausted.
#[test]
fn ent_stale_visible_slot_expires_after_60_ticks_without_reacquiring() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_inv_named(&mut c, 1351, Some("Bronze axe"));
    plant_npc_typed_with_op(&mut c, 0, 448, "Oak", -1, None, "Chop down");
    c.local_player.as_mut().expect("local player").entity.face_entity = 0;
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(status.hold, "the owned Ent holds on its onset tick");
    for tick in 1..60 {
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert!(status.hold, "Ent remains held through its 60-tick lifetime at {tick}");
    }

    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(!status.hold, "a visible stale slot must release after 60 held ticks");
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(!status.hold, "the expired visible slot must not be reacquired");
    assert_eq!(drv.walks.len(), 1, "the expired slot must not send another stop");
}


/// A refused stop walk retries only on fresh game ticks: non-fresh passes
/// still publish the hold without resending, and a sent stop never resends.
#[test]
fn ent_rejected_stop_retries_only_on_fresh() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_inv_named(&mut c, 1351, Some("Bronze axe"));
    plant_npc_typed_with_op(&mut c, 0, 448, "Oak", -1, None, "Chop down");
    c.local_player.as_mut().expect("local player").entity.face_entity = 0;

    let mut g = Guardian::new();
    let mut drv = FakeDriver {
        walk_ok: false,
        ..Default::default()
    };
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    // Fresh tick 1: stop attempted and refused, hold still rises.
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(status.hold, "ent must hold even when the stop is refused");
    assert_eq!(drv.walks.len(), 1, "first fresh tick sends the stop");

    // Non-fresh (same snapshot, no tick edge): hold without resending.
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(status.hold, "non-fresh must still publish the hold");
    assert_eq!(drv.walks.len(), 1, "non-fresh must not retry the stop");

    // Next fresh tick: refused stop retries.
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(status.hold, "ent must stay held while the stop is refused");
    assert_eq!(
        drv.walks.len(),
        2,
        "next fresh tick retries the refused stop"
    );

    // Accept the walk: one more send, then silence while held.
    drv.walk_ok = true;
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(status.hold, "ent stays held after the stop sends");
    assert_eq!(
        drv.walks.len(),
        3,
        "fresh retry after refusal must send again"
    );
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(status.hold, "ent stays held");
    assert_eq!(drv.walks.len(), 3, "a sent stop must not resend");
}
