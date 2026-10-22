// GUARDIAN-FIX-R1: reduced tick replays of the retained live receipts.

#[test]
fn guardian_r1_tree_spirit_still_targeting_after_apparent_escape_stays_held() {
    // diag3/guardian_skill_tree_spirit-4: first targeted at t99 from
    // (3109,3238), apparent escape at (3109,3245) t104, release t105,
    // then targeted hits t108,112,116,120,128. Translate to scene tiles.
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 13, 8);
    plant_npc_typed_with_op(&mut c, 0, 438, "Tree spirit", 32768, None, "Attack");
    set_flee_npc(&mut c, 10, 10);
    let data = event_game_data();
    let mut guardian = Guardian::new();
    let mut driver = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();
    tick_at(&mut c, &mut snap);
    assert!(guardian_tick_with_data(&mut guardian, &mut driver, &snap, &settings, &data).hold);
    set_flee_player(&mut c, 10, 17, false);
    set_flee_npc(&mut c, 10, 14);
    for receipt_tick in [104, 105, 108, 112] {
        tick_at(&mut c, &mut snap);
        let status = guardian_tick_with_data(&mut guardian, &mut driver, &snap, &settings, &data);
        assert_eq!(
            status.kind,
            Some(RandomKind::Evade),
            "attacker remains visible at t{receipt_tick}"
        );
        assert!(
            status.hold,
            "a still-targeting spirit must not release at t{receipt_tick}"
        );
    }
    assert!(
        driver.walks.iter().all(|tile| *tile != (13, 8)),
        "never return into the spirit"
    );
}

#[test]
fn guardian_r1_tree_spirit_anchor_is_observed_before_targeting() {
    // dryad spawn delays its attack by one tick. A spawn sighting must
    // survive the later first-targeting sighting after movement.
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 9, 10);
    plant_npc_typed_with_op(&mut c, 0, 438, "Tree spirit", -1, None, "Attack");
    set_flee_npc(&mut c, 9, 11);
    let data = event_game_data();
    let mut guardian = Guardian::new();
    let mut driver = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();
    tick_at(&mut c, &mut snap);
    assert!(!guardian_tick_with_data(&mut guardian, &mut driver, &snap, &settings, &data).hold);
    set_flee_npc(&mut c, 9, 9);
    c.npc[0].as_mut().expect("spirit").entity.face_entity = 32768;
    tick_at(&mut c, &mut snap);
    assert!(guardian_tick_with_data(&mut guardian, &mut driver, &snap, &settings, &data).hold);
    let target = *driver.walks.last().expect("flee sends a route");
    assert!(
        outside_chase_range(target, (9, 11), 5),
        "flee must clear the earlier spawn anchor, not just the moved NPC"
    );
}

#[test]
fn guardian_r1_disengaged_attacker_holds_until_observed_despawn() {
    // round4 troll: escape t76, release t77, return t78 while present.
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 10, 10);
    plant_npc_typed_with_op(&mut c, 0, 391, "River troll", 32768, None, "Attack");
    set_flee_npc(&mut c, 9, 10);
    let data = event_game_data();
    let mut guardian = Guardian::new();
    let mut driver = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();
    tick_at(&mut c, &mut snap);
    assert!(guardian_tick_with_data(&mut guardian, &mut driver, &snap, &settings, &data).hold);
    set_flee_player(&mut c, 16, 10, false);
    c.npc[0].as_mut().expect("troll").entity.face_entity = -1;
    for _ in 0..5 {
        tick_at(&mut c, &mut snap);
        let status = guardian_tick_with_data(&mut guardian, &mut driver, &snap, &settings, &data);
        assert!(status.hold, "the visible attacker has not despawned");
        assert!(
            !driver.walks.contains(&(10, 10)),
            "return only after despawn"
        );
    }
    clear_npcs(&mut c);
    tick_at(&mut c, &mut snap);
    let status = guardian_tick_with_data(&mut guardian, &mut driver, &snap, &settings, &data);
    assert!(!status.hold);
    assert_eq!(driver.walks.last(), Some(&(10, 10)));
}

#[test]
fn guardian_r1_lost_pickaxe_refused_send_does_not_abandon_recovery() {
    // diag-pickaxe/lost_pickaxe-8: Remove sent at client t51, then
    // InvalidAction at t52 while the worn handle/head remain visible.
    // Receipt t65 raises the hold, t67 releases it, and no tool returns.
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_inv_named(&mut c, 466, Some("Pickaxe handle"));
    clear_inv(&mut c);
    c.side_icon[4] = 710;
    c.set_iface(
        710,
        IfType {
            id: 710,
            layer_id: 710,
            children: Some(vec![711]),
            ..Default::default()
        },
    );
    let set_worn = |c: &mut Client, remove: bool| {
        c.set_iface(
            711,
            IfType {
                id: 711,
                layer_id: 710,
                r#type: ComponentType::TYPE_INV,
                iop: [remove.then(|| "Remove".into()), None, None, None, None],
                ..Default::default()
            },
        );
        c.set_iface_mut(
            711,
            IfTypeMut {
                link_obj_type: Some(vec![0, 0, 0, 467]),
                link_obj_number: Some(vec![0, 0, 0, 1]),
                ..Default::default()
            },
        );
        c.gens.iface += 1;
        c.bump_gens(client::io::ServerProt::UPDATE_INV_FULL);
    };
    set_worn(&mut c, true);
    open_chat(&mut c);
    let mut guardian = Guardian::new();
    let mut driver = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();
    tick_at(&mut c, &mut snap);
    assert!(guardian.tick(&mut driver, &snap, &settings, 0, None).hold);
    plant_ground_obj(&mut c, 1, 0, 490, Some("Pickaxe head"));
    assert_eq!(driver.menus[0].1, MiniMenuAction::INV_BUTTON1);

    set_worn(&mut c, false);
    tick_at(&mut c, &mut snap);
    driver.menus.clear();
    assert!(
        guardian.tick(&mut driver, &snap, &settings, 600, None).hold,
        "a transient missing Remove action must preserve recovery"
    );
    assert!(
        driver.menus.is_empty(),
        "the refused operation sends nothing"
    );
    assert!(guardian.tick(&mut driver, &snap, &settings, 600, None).hold);
    assert!(
        driver.menus.is_empty(),
        "never retry in the same snapshot tick"
    );

    // The engine's removal settles, and the message closes. Recovery
    // must continue, rather than releasing the Gatherer to its bank.
    close_chat(&mut c);
    c.set_iface_mut(711, IfTypeMut::default());
    plant_inv_named(&mut c, 466, Some("Pickaxe handle"));
    tick_at(&mut c, &mut snap);
    assert!(
        guardian
            .tick(&mut driver, &snap, &settings, 1200, None)
            .hold
    );
    assert_eq!(driver.menus[0].1, MiniMenuAction::OP_OBJ3);

    c.ground_obj[0][1][0] = None;
    plant_inv_named(&mut c, 490, Some("Pickaxe head"));
    c.set_iface_mut(
        301,
        IfTypeMut {
            link_obj_type: Some(vec![467, 491]),
            link_obj_number: Some(vec![1, 1]),
            ..Default::default()
        },
    );
    c.bump_gens(client::io::ServerProt::UPDATE_INV_FULL);
    driver.menus.clear();
    tick_at(&mut c, &mut snap);
    assert!(
        guardian
            .tick(&mut driver, &snap, &settings, 1800, None)
            .hold
    );
    assert_eq!(driver.menus[0].1, MiniMenuAction::USEHELD_START);
    assert_eq!(driver.menus[1].1, MiniMenuAction::USEHELD_ONHELD);

    plant_inv_ops(
        &mut c,
        1265,
        Some("Iron pickaxe"),
        [Some("Wield"), None, None, None, None],
    );
    driver.menus.clear();
    tick_at(&mut c, &mut snap);
    let status = guardian.tick(&mut driver, &snap, &settings, 2400, None);
    assert!(
        !status.hold && !status.ours,
        "release only after the complete pickaxe returns"
    );
    assert_eq!(
        driver.menus[0].1,
        MiniMenuAction::OP_HELD1,
        "restore the formerly worn tool"
    );
}

/// A partial Swarm route can end before the 19-tile flee ring, beyond the
/// 15-tile NPC view. Keep walking; outside chase range the hostile cleanup
/// (`macro_events.rs2:166-173`, `antimacro.npc:130`) still bounds the wait.
#[test]
fn guardian_r2_missing_swarm_continues_flee_and_finishes_cleanup() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc_typed_with_op(&mut c, 0, 411, "Swarm", 32768, None, "Attack");
    set_flee_npc(&mut c, 1, 0);
    let data = event_game_data();
    let mut guardian = Guardian::new();
    let mut driver = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();
    tick_at(&mut c, &mut snap);
    assert!(guardian_tick_with_data(&mut guardian, &mut driver, &snap, &settings, &data).hold);
    assert_eq!(driver.walks, vec![(-18, 0)]);

    set_flee_player(&mut c, -16, 0, false);
    clear_npcs(&mut c);
    tick_at(&mut c, &mut snap);
    assert!(guardian_tick_with_data(&mut guardian, &mut driver, &snap, &settings, &data).hold);
    assert_eq!(
        driver.walks,
        vec![(-18, 0), (-18, 0)],
        "finish the outward route even without the NPC in view"
    );
    // A refused/partial route need not reach the full wandering-clearance
    // ring: missing outside chase range is bounded by 20+2 cleanup ticks.
    for missing_tick in 2..=22 {
        tick_at(&mut c, &mut snap);
        let status = guardian_tick_with_data(&mut guardian, &mut driver, &snap, &settings, &data);
        assert_eq!(status.hold, missing_tick < 22);
    }
    assert_eq!(driver.walks.last(), Some(&(0, 0)));
}

/// A disengaged attacker retains the normal spawn-lifetime budget even if
/// no route is possible. Active attacks have a separate shorter bound.
#[test]
fn guardian_r2_disengaged_attacker_gives_up_without_return_or_reacquisition() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc_typed_with_op(&mut c, 0, 391, "River troll", 32768, None, "Attack");
    let data = event_game_data();
    let mut guardian = Guardian::new();
    let mut driver = FakeDriver {
        walk_ok: false,
        walk_nearest_ok: false,
        ..Default::default()
    };
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();
    for elapsed in 0..=2_000 {
        if elapsed == 1 {
            c.npc[0].as_mut().expect("troll").entity.face_entity = -1;
        }
        tick_at(&mut c, &mut snap);
        let status = guardian_tick_with_data(&mut guardian, &mut driver, &snap, &settings, &data);
        assert_eq!(status.kind, Some(RandomKind::Evade));
        assert_eq!(status.hold, elapsed < 1_002, "elapsed={elapsed}");
        assert_eq!(status.ours, elapsed < 1_002, "elapsed={elapsed}");
        if elapsed == 1_002 {
            let sends = driver.walks.len();
            // Neither repaint nor subsequent ticks may retake this actor.
            let repeat = guardian_tick_with_data(&mut guardian, &mut driver, &snap, &settings, &data);
            assert!(!repeat.hold && !repeat.ours);
            assert_eq!(driver.walks.len(), sends);
        }
    }
    assert!(!driver.walks.contains(&(0, 0)), "give-up is not a walk back");
}

/// World zombie_unarmed (73) is not a macro event, even when it attacks us.
/// `content/pack/npc.pack:74` versus macro zombies at :420-425.
#[test]
fn guardian_r2_world_zombie_73_never_starts_or_sustains_evade() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc_typed_with_op(&mut c, 0, 73, "Zombie", 32768, None, "Attack");
    let data = event_game_data();
    let mut guardian = Guardian::new();
    let mut driver = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();
    for _ in 0..2 {
        tick_at(&mut c, &mut snap);
        let status = guardian_tick_with_data(&mut guardian, &mut driver, &snap, &settings, &data);
        assert_eq!(status.kind, None);
        assert!(!status.hold && !status.ours);
    }
    assert!(driver.walks.is_empty());

    clear_npcs(&mut c);
    plant_npc_typed_with_op(&mut c, 0, 419, "Zombie", 32768, None, "Attack");
    tick_at(&mut c, &mut snap);
    assert!(guardian_tick_with_data(&mut guardian, &mut driver, &snap, &settings, &data).hold);
    // Reusing an event slot with a same-name world type must end ownership.
    clear_npcs(&mut c);
    plant_npc_typed_with_op(&mut c, 0, 73, "Zombie", 32768, None, "Attack");
    tick_at(&mut c, &mut snap);
    let status = guardian_tick_with_data(&mut guardian, &mut driver, &snap, &settings, &data);
    assert_eq!(status.kind, None);
    assert!(!status.hold && !status.ours);
}

#[test]
fn guardian_r2_visible_former_attacker_does_not_mask_strange_box() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc_typed_with_op(&mut c, 0, 391, "River troll", 32768, None, "Attack");
    let data = event_game_data();
    let mut guardian = Guardian::new();
    let mut driver = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();
    tick_at(&mut c, &mut snap);
    assert!(guardian_tick_with_data(&mut guardian, &mut driver, &snap, &settings, &data).hold);
    c.npc[0].as_mut().expect("troll").entity.face_entity = -1;
    plant_inv_named(&mut c, 3062, Some("Strange box"));
    tick_at(&mut c, &mut snap);
    let status = guardian_tick_with_data(&mut guardian, &mut driver, &snap, &settings, &data);
    assert_eq!(status.kind, Some(RandomKind::Box));
    assert!(status.ours && status.hold);
}

/// A nearest route can be accepted while ending on the boxed-in bot's tile.
/// Repeated walk acceptance is not distance progress, and must not suppress
/// retaliation or script eating for the full missing-attacker lifetime.
fn replay_boxed_targeting_attacker(type_id: usize, name: &str, radius: i32, bound: u16) {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", -radius, 0);
    plant_npc_typed_with_op(&mut c, 0, type_id, name, 32768, None, "Attack");
    set_flee_npc(&mut c, 0, 0);
    let data = event_game_data();
    let mut guardian = Guardian::new();
    let mut driver = FakeDriver {
        reach: Some(((0, 0), radius)),
        ..Default::default()
    };
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();
    tick_at(&mut c, &mut snap);
    assert!(guardian_tick_with_data(&mut guardian, &mut driver, &snap, &settings, &data).hold);
    set_flee_npc(&mut c, -radius + 1, 0);
    let mut attempts_at_release = None;
    for elapsed in 1..=2_000 {
        // More host frames must neither consume fresh ticks nor retry walks.
        let attempts = driver.walks.len();
        for _ in 0..3 {
            let repaint =
                guardian_tick_with_data(&mut guardian, &mut driver, &snap, &settings, &data);
            assert_eq!(repaint.hold, elapsed <= bound, "repaint at {elapsed}");
            assert_eq!(driver.walks.len(), attempts, "no walk on repaint");
        }
        plant_positive_hit(&mut c);
        tick_at(&mut c, &mut snap);
        let status = guardian_tick_with_data(&mut guardian, &mut driver, &snap, &settings, &data);
        assert_eq!(status.kind, Some(RandomKind::Evade));
        assert_eq!(status.hold, elapsed < bound, "{name} elapsed={elapsed}");
        assert_eq!(status.ours, elapsed < bound, "{name} elapsed={elapsed}");
        if elapsed == bound {
            assert_eq!(driver.walks.len(), attempts, "release sends no further walk");
            attempts_at_release = Some(attempts);
            assert!(
                !driver.sent_walks.is_empty(),
                "the boxed route must accept nearest walks before release"
            );
        }
        if let Some(attempts) = attempts_at_release {
            assert_eq!(driver.walks.len(), attempts, "no walk or reacquisition after release");
        }
    }
}

#[test]
fn guardian_r3_boxed_targeting_swarm_releases_after_21_fresh_ticks() {
    replay_boxed_targeting_attacker(411, "Swarm", 10, 21);
}

#[test]
fn guardian_r3_boxed_targeting_tree_spirit_releases_after_9_fresh_ticks() {
    replay_boxed_targeting_attacker(438, "Tree spirit", 5, 9);
}

#[test]
fn guardian_r3_only_new_furthest_distance_resets_engaged_budget() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", -1, 0);
    plant_npc_typed_with_op(&mut c, 0, 438, "Tree spirit", 32768, None, "Attack");
    set_flee_npc(&mut c, 0, 0);
    let data = event_game_data();
    let mut guardian = Guardian::new();
    let mut driver = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();
    for elapsed in 0..=16 {
        // Eight stalled ticks, then genuine outward progress buys a new
        // budget. Retreating and regaining old ground cannot reset it.
        let distance = match elapsed {
            8 | 10 | 12 | 14 | 16 => 2,
            _ => 1,
        };
        set_flee_player(&mut c, -distance, 0, true);
        tick_at(&mut c, &mut snap);
        assert!(
            guardian_tick_with_data(&mut guardian, &mut driver, &snap, &settings, &data).hold,
            "progress at tick 8 must preserve the hold at {elapsed}"
        );
    }
    tick_at(&mut c, &mut snap);
    let attempts = driver.walks.len();
    let status = guardian_tick_with_data(&mut guardian, &mut driver, &snap, &settings, &data);
    assert!(!status.hold && !status.ours, "nine ticks without a new maximum releases");
    assert_eq!(driver.walks.len(), attempts, "give-up sends no walk");
}

#[test]
fn guardian_evade_stunned_ticks_do_not_consume_the_stalled_budget() {
    // FIX-025-F-WATCHMAN recorded sequence: the Watchman is summoned, the
    // bot fails a pickpocket attempt on a Man and the content applies its
    // 8-tick stun (`pickpocket.dbrow:10`). The `stunned` walktrigger then
    // cancels every flee walk and answers "You're stunned!" per attempt
    // (`movecheck.rs2:1-6`), so the tile cannot change until the stun ends.
    // Those server-blocked ticks must not spend the 9-tick stalled budget
    // (Watchman `flee_distance` 7 + 2).
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", -1, 0);
    plant_npc_typed_with_op(&mut c, 0, 431, "Watchman", 32768, None, "Attack");
    set_flee_npc(&mut c, 0, 0);
    let data = event_game_data();
    let mut guardian = Guardian::new();
    let mut driver = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();
    tick_at(&mut c, &mut snap);
    assert!(guardian_tick_with_data(&mut guardian, &mut driver, &snap, &settings, &data).hold);
    let mut walks = driver.walks.len();
    for stunned_tick in 1..=12 {
        let line = if stunned_tick == 1 {
            "You've been stunned!"
        } else {
            "You're stunned!"
        };
        c.add_chat(0, line, "");
        tick_at(&mut c, &mut snap);
        let status = guardian_tick_with_data(&mut guardian, &mut driver, &snap, &settings, &data);
        assert!(
            status.hold && status.ours,
            "a server-blocked flee must not give up at stunned tick {stunned_tick}"
        );
        assert!(
            driver.walks.len() > walks,
            "the flee keeps attempting walks while stunned at tick {stunned_tick}"
        );
        walks = driver.walks.len();
    }
    // The stun has ended and the bot still makes no progress: the normal
    // 9-tick stalled bound applies from here, unspent and unwidened.
    for stalled_tick in 1..=9 {
        tick_at(&mut c, &mut snap);
        let status = guardian_tick_with_data(&mut guardian, &mut driver, &snap, &settings, &data);
        assert_eq!(
            status.hold,
            stalled_tick < 9,
            "post-stun stalled tick {stalled_tick}"
        );
    }
}
