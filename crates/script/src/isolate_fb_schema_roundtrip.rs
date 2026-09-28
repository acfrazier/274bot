#[test]
fn generated_bindings_round_trip_domain_payload_and_presence() {
    let item_ops = vec!["Wear".to_string(), "Drop".to_string()];
    let inventory = [ItemRowInput {
        name: Some("Rune platebody"),
        count: 1,
        id: 1127,
        ops: &item_ops,
        noted: false,
        cert: -1,
        component_id: 3214,
        slot: 3,
    }];
    let stats = [StatInput {
        index: 3,
        name: "hitpoints",
        xp: 12_345,
        base: 20,
        effective: 17,
    }];
    let npc_actions = vec!["Talk-to".to_string(), "Attack".to_string()];
    let npcs = [SceneEntityInput {
        index: 9,
        id: 1,
        name: Some("Man"),
        x: 3_208,
        z: 3_210,
        level: 0,
        distance: 2,
        health: 5,
        max_health: 7,
        in_combat: true,
        animating: false,
        actions: &npc_actions,
        reachable: true,
        reachable_adj: true,
        combat_level: 2,
        target_kind: 2,
        target_index: 4,
        size: 1,
        nx: 3_208,
        nz: 3_210,
        shape: 0,
        angle: 0,
    }];
    let booths = [TileInput {
        x: 3_209,
        z: 3_211,
        level: 0,
    }];
    let mut input = empty_input(41);
    input.here = Some(TileInput {
        x: 3_207,
        z: 3_209,
        level: 0,
    });
    input.inv = &inventory;
    input.stats = &stats;
    input.npcs = &npcs;
    input.booths = &booths;
    input.my_name = Some("Flat C");
    input.self_target_kind = 1;
    input.self_target_index = 9;

    let quest_rows = [
        QuestStatusInput {
            name: "Waterfall Quest",
            status: "complete",
            component_id: Some(0),
        },
        QuestStatusInput {
            name: "Cook's Assistant",
            status: "started",
            component_id: None,
        },
    ];
    let collision_flags = [0x100_i32, -1, 0x4000];
    let bytes = encode_snapshot_with_native(
        &input,
        NativeFactsInput {
            quest_statuses: Some(&quest_rows),
            collision: Some(CollisionViewInput {
                available: true,
                base_x: 3_200,
                base_z: 3_200,
                level: 0,
                width: 3,
                height: 1,
                flags: &collision_flags,
            }),
            ..NativeFactsInput::default()
        },
    );

    let snapshot = decode_snapshot(&bytes).expect("generated snapshot verifies");
    assert_eq!(snapshot.tick(), 41);
    assert_eq!(snapshot.my_name(), Some("Flat C"));
    assert_eq!(snapshot.self_target_kind(), 1);
    assert_eq!(snapshot.self_target_index(), 9);
    let here = snapshot.here().expect("posted world tile");
    assert_eq!((here.x(), here.z(), here.level()), (3_207, 3_209, 0));

    let inventory = snapshot.inv().expect("posted inventory");
    assert_eq!(inventory.len(), 1);
    let item = inventory.get(0);
    assert_eq!(item.name(), Some("Rune platebody"));
    assert_eq!((item.id(), item.count(), item.slot()), (1127, 1, 3));
    assert_eq!(
        item.ops()
            .expect("posted item operations")
            .iter()
            .collect::<Vec<_>>(),
        vec!["Wear", "Drop"]
    );

    let stats = snapshot.stats().expect("posted stats");
    let hitpoints = stats.get(0);
    assert_eq!(hitpoints.name(), Some("hitpoints"));
    assert_eq!(
        (hitpoints.xp(), hitpoints.base(), hitpoints.effective()),
        (12_345, 20, 17)
    );

    let npc = snapshot.npcs().expect("posted npcs").get(0);
    assert_eq!(npc.name(), Some("Man"));
    assert_eq!((npc.index(), npc.id(), npc.size()), (9, 1, 1));
    assert_eq!(
        npc.actions()
            .expect("posted npc actions")
            .iter()
            .collect::<Vec<_>>(),
        vec!["Talk-to", "Attack"]
    );
    assert_eq!((npc.target_kind(), npc.target_index()), (2, 4));

    let booth = snapshot.booths().expect("posted booth").get(0);
    assert_eq!((booth.x(), booth.z(), booth.level()), (3_209, 3_211, 0));

    let quests = snapshot.quest_statuses().expect("posted quest rows");
    assert_eq!(quests.len(), 2);
    assert_eq!(quests.get(0).component_id(), Some(0));
    assert_eq!(quests.get(1).component_id(), None);

    let collision = snapshot.collision().expect("posted collision view");
    assert!(collision.available());
    assert_eq!(
        collision
            .flags()
            .expect("posted collision flags")
            .iter()
            .collect::<Vec<_>>(),
        collision_flags
    );

    assert!(snapshot.has_self_chat());
    assert_eq!(snapshot.self_chat(), Some(""));
}
