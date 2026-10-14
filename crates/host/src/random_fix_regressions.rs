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
    plant_ground_obj(&mut c, 1, 0, 490, Some("Pickaxe head"));
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
