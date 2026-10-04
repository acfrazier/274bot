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
    let chat_options = [ChatOptionInput {
        text: "Continue",
        com_id: 0,
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
    input.chat_options = &chat_options;
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
    let chat_options = snapshot.chat_options().expect("posted chat options");
    let chat_option = chat_options.get(0);
    assert_eq!(chat_option.text(), Some("Continue"));
    assert_eq!(chat_option.com_id(), 0);
    assert_ne!(
        chat_option._tab.vtable().get(ChatOption::VT_COM_ID),
        0,
        "default-valued ChatOption.com_id remains physically present"
    );

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
    let options_before = [ChatOptionInput {
        text: "Continue",
        com_id: 7,
    }];
    let options_after = [ChatOptionInput {
        text: "Continue",
        com_id: 0,
    }];
    let mut before = empty_input(100);
    before.chat_options = &options_before;
    before.run_enabled = true;
    before.run_energy = 42;
    before.self_target_index = 9;
    let mut after = empty_input(101);
    after.chat_options = &options_after;
    let mut delta_buf = IsolateBuf::new();
    let (_, before_fp) = delta_buf.encode_snapshot_delta(None, &before, false);
    let (delta_bytes, _) = delta_buf.encode_snapshot_delta(Some(&before_fp), &after, false);
    let delta = decode_snapshot(&delta_bytes).expect("default-valued delta verifies");
    assert!(delta.has_run_enabled());
    assert!(!delta.run_enabled());
    assert!(delta.has_run_energy());
    assert_eq!(delta.run_energy(), 0);
    assert!(delta.has_self_target_index());
    assert_eq!(delta.self_target_index(), -1);
    assert!(!delta.has_bank_note_on());
    assert_eq!(delta.bank_note_on(), -1);
    let delta_option = delta
        .chat_options()
        .expect("changed chat options are present")
        .get(0);
    assert_eq!(delta_option.com_id(), 0);
    assert_ne!(
        delta_option._tab.vtable().get(ChatOption::VT_COM_ID),
        0,
        "force_defaults keeps ChatOption.com_id=0 present in a delta"
    );
}

#[test]
fn absent_quest_status_defaults_to_unknown() {
    let mut b = flatbuffers::FlatBufferBuilder::new();
    let name = b.create_string("Mystery Quest");
    let status = {
        let mut row = QuestStatusBuilder::new(&mut b);
        row.add_name(name);
        row.finish()
    };
    let statuses = b.create_vector(&[status]);
    let root = {
        let mut snapshot = SnapshotBuilder::new(&mut b);
        snapshot.add_tick(1);
        snapshot.add_quest_statuses(statuses);
        snapshot.add_quest_statuses_available(true);
        snapshot.finish()
    };
    b.finish(root, None);

    let snapshot = decode_snapshot(b.finished_data()).expect("snapshot verifies");
    let row = snapshot
        .quest_statuses()
        .expect("quest statuses")
        .get(0);
    assert_eq!(row.status(), None, "fixture omits the status slot");

    crate::observed::on_reset();
    crate::observed::apply(&snapshot);
    let observed_status = crate::observed::with(|scene| match scene.latest().quest_statuses() {
        Some(crate::observed::QuestTab::Bound(rows)) => rows[0].status.to_string(),
        _ => panic!("quest statuses were not applied"),
    });
    crate::observed::on_reset();
    assert_eq!(observed_status, "unknown");
}

#[test]
fn older_snapshot_without_chat_page_fingerprint_reads_as_default() {
    let mut b = flatbuffers::FlatBufferBuilder::new();
    let root = {
        let mut snapshot = SnapshotBuilder::new(&mut b);
        snapshot.add_tick(1);
        snapshot.finish()
    };
    b.finish(root, None);

    let snapshot = decode_snapshot(b.finished_data()).expect("older snapshot verifies");
    assert!(
        !snapshot.has_chat_page_fingerprint(),
        "older wire data has no fingerprint slot"
    );
    assert_eq!(snapshot.chat_page_fingerprint(), 0);
}

/// A buffer from before slot 284 carries no side root: it reads absent
/// (`-1`), so an open bank's side is never mistaken for a posted pack.
#[test]
fn older_snapshot_without_side_modal_id_reads_absent() {
    let mut b = flatbuffers::FlatBufferBuilder::new();
    let root = {
        let mut snapshot = SnapshotBuilder::new(&mut b);
        snapshot.add_tick(1);
        snapshot.add_bank_open(true);
        snapshot.add_chat_page_fingerprint(7);
        snapshot.finish()
    };
    b.finish(root, None);

    let snapshot = decode_snapshot(b.finished_data()).expect("older snapshot verifies");
    assert!(!snapshot.has_side_modal_id());
    assert_eq!(snapshot.side_modal_id(), -1);
    crate::observed::on_reset();
    crate::observed::apply(&snapshot);
    let posted = crate::observed::with(|scene| {
        let session = scene.since_login();
        crate::bank::ops::side_observation(
            session.bank_open().unwrap_or(false),
            session.side_modal_id().unwrap_or(-1),
            session.bank_side().map(Vec::as_slice).unwrap_or_default(),
        )
        .is_some()
    });
    crate::observed::on_reset();
    assert!(!posted, "an old buffer's side is not posted");
    assert_eq!(
        Snapshot::VT_SIDE_MODAL_ID,
        Snapshot::VT_CHAT_PAGE_FINGERPRINT + 2,
        "appended after the last deployed slot"
    );
}
