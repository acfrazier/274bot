use super::*;
use crate::file_journal::{PacketKinds, PacketRecord};
use serde_json::json;

#[test]
fn header_checks_all_roles_before_admitting_the_selected_case() {
    let header = json!({
        "name": "walk-check",
        "revision": 289,
        "cases": ["north", "south"],
        "accounts": {"main": {"mainland": true}, "peer": {"preamble": "tutorial"}},
        "proof": {"is": "bank_closed"},
        "seeds": "confirmed-only"
    });
    let plan = file_scenario(header.clone(), Some("south"), 289, false).unwrap();
    assert_eq!(plan.case, "south");
    assert_eq!(plan.deadline, Duration::from_secs(180));
    assert!(plan.roles["main"].mainland);
    assert_eq!(plan.roles["peer"].preamble, Some(FilePreamble::Tutorial));
    assert!(plan.confirmed_only);
    assert!(file_scenario(header.clone(), Some("east"), 289, false).is_err());
    assert!(file_scenario(header, Some("south"), 377, false).is_err());
}

#[test]
fn static_proof_rejects_unwitnessed_varps_table_state_and_unknown_keys() {
    for proof in [
        json!({"is": "script_idle"}),
        json!({"is": "varp", "id": 1, "min": 0}),
        json!({"is": "varp_exact", "id": 1, "value": 0}),
        json!({"is": "arrived", "x": 3220, "z": 3218, "level": 0, "wrong": 1}),
        json!({"is": "action_sent", "after": 1}),
        json!({"is": "all", "of": []}),
    ] {
        assert!(file_scenario(
            json!({"name": "walk-check", "proof": proof}),
            None,
            289,
            false
        )
        .is_err());
    }
}

#[test]
fn file_proof_uses_native_snapshot_predicates_without_a_table_runner() {
    let proof = json!({"is": "all", "of": [
        {"is": "arrived_ring", "x": 3220, "z": 3218, "level": 0, "min": 1, "max": 2},
        {"is": "bank_item_id", "id": 995, "count": 7},
        {"is": "npc_at", "type": 1, "x": 3220, "z": 3218},
        "tutorial_closed"
    ]});
    assert!(FileCheck::from_literal(proof).unwrap().terminal());
    assert!(FileCheck::from_literal(json!({
        "is": "arrived_ring", "x": 3220, "z": 3218, "level": 0, "min": 3, "max": 2
    }))
    .is_err());
}

#[test]
fn static_named_bank_proof_requires_exact_stand_and_distinct_totals() {
    let standing =
        json!({"is": "standing_bank_closed", "stand": {"x": 3221, "z": 3218, "level": 0}});
    let exact = json!({"is": "bank_exact", "expected": [{"id": 995, "count": 7}]});
    let plan = file_scenario(
        json!({"name": "bank-check", "proof": {"is": "all", "of": [standing, exact]}}),
        None,
        289,
        false,
    );
    assert!(plan.is_ok());
    for proof in [
        json!({"is": "standing_bank_closed", "stand": {"x": 3221, "z": 3218, "level": 4}}),
        json!({"is": "bank_exact", "expected": [{"id": 995, "count": 7}, {"id": 995, "count": 3}]}),
        json!({"is": "bank_exact", "expected": [{"id": 995, "count": 0}]}),
    ] {
        assert!(file_scenario(
            json!({"name": "bank-check", "proof": proof}),
            None,
            289,
            false
        )
        .is_err());
    }
}

#[test]
fn marks_do_not_accept_a_first_send_on_a_foreign_session() {
    let mut log = FileJournal::new();
    let snapshot = GameSnapshot::new();
    log.mark(1, &snapshot, 4, 83).unwrap();
    log.record_action(5, 90, true).unwrap();
    assert!(log.action_sent(1, 5).is_err());
    assert!(log.action_sent(1, 4).is_err());
    assert_eq!(log.mark_at(1).unwrap().source_serial, 83);
}

#[test]
fn action_proof_requires_exactly_one_root_that_actually_sent() {
    let mut log = FileJournal::new();
    log.mark(1, &GameSnapshot::new(), 2, 11).unwrap();
    assert!(!log.action_sent(1, 2).unwrap());
    log.record_action(2, 7, false).unwrap();
    assert!(!log.action_sent(1, 2).unwrap());
    log.note_sent(2, 7).unwrap();
    assert!(log.action_sent(1, 2).unwrap());
    log.mark(2, &GameSnapshot::new(), 2, 12).unwrap();
    log.record_action(2, 8, true).unwrap();
    assert!(log.action_sent(2, 2).unwrap());
    log.record_action(2, 9, true).unwrap();
    assert!(log.action_sent(2, 2).is_err());
}
#[test]
fn file_check_signatures_accept_declared_fields_and_reject_other_fields() {
    if !api::test_support::game_data_available() {
        return;
    }
    let unavailable = [
        "clue_protect_window",
        "logged_out",
        "login_packets",
        "script_idle",
        "script_receipt",
        "script_running",
        "varp",
        "varp_exact",
        "world_speed_changed",
    ];
    for signature in FILE_CHECK_SIGNATURES {
        let mut args = Map::new();
        for field in signature.fields {
            args.insert(field.name.into(), signature_sample(field.ty));
        }
        let parsed = FileCheck::parse(signature.name, args.clone());
        if unavailable.contains(&signature.name) {
            assert!(
                parsed.unwrap_err().contains(signature.name),
                "{} did not name its File-mode refusal",
                signature.name
            );
        } else {
            assert!(
                parsed.is_ok(),
                "{} rejected its declared fields",
                signature.name
            );
        }
        args.insert("__undeclared".into(), Value::Null);
        let parsed = FileCheck::parse(signature.name, args);
        if unavailable.contains(&signature.name) {
            assert!(
                parsed.unwrap_err().contains(signature.name),
                "{} did not retain its named refusal",
                signature.name
            );
        } else {
            assert!(
                parsed.is_err(),
                "{} accepted an undeclared field",
                signature.name
            );
        }
    }
}

fn signature_sample(ty: &str) -> Value {
    match ty {
        "integer" | "positive integer" | "skill" => json!(1),
        "string" | "text matcher" | "packet family" | "packet form" => json!("x"),
        "boolean" => json!(true),
        "amount" => json!({"eq": 1}),
        "container" => json!("inventory"),
        "exactness" => json!("value"),
        "to mark or label" => json!(2),
        "tile" => json!({"x": 1, "z": 1, "level": 0}),
        "integer list" => json!([1]),
        "string list" => json!(["x"]),
        "predicate list" => json!(["bank_closed"]),
        "expected item totals" => json!([]),
        "stat matchers" => json!([{"skill": 0, "xp": 1}]),
        "inventory matchers" => json!([{"container": "inventory"}]),
        "message matchers" => json!([{"type": 0, "text": "x"}]),
        "var matchers" => json!([{"id": 1, "value": 1}]),
        "record matcher" => json!({"kind": "message", "type": 0, "text": "x"}),
        "message filter" => json!(true),
        "dialogue lines" => json!(["line"]),
        "dialogue options" => json!(["option"]),
        "hint kind" => json!("tile"),
        "signal name" => json!("ready"),
        other => panic!("unknown signature type {other:?}"),
    }
}

fn selected_game_data() -> std::sync::Arc<SelectedGameData> {
    api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap()
}

#[test]
fn received_uses_distinct_records_and_bipartite_message_assignment() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut journal = FileJournal::new();
    let mut before = GameSnapshot::new();
    before.seed_ingame(2);
    before.seed_chat_lines(vec![api::snapshot::ChatLineView {
        type_: 0,
        username: None,
        text: "baseline".into(),
        sequence: 1,
    }]);
    journal.observe(&before, 4).unwrap();
    journal.mark(1, &before, 4, 0).unwrap();

    let mut after = GameSnapshot::new();
    after.seed_ingame(2);
    after.seed_chat_lines(vec![
        api::snapshot::ChatLineView {
            type_: 0,
            username: None,
            text: "alpha".into(),
            sequence: 2,
        },
        api::snapshot::ChatLineView {
            type_: 0,
            username: None,
            text: "beta".into(),
            sequence: 3,
        },
        api::snapshot::ChatLineView {
            type_: 0,
            username: None,
            text: "baseline".into(),
            sequence: 1,
        },
    ]);
    journal.observe(&after, 4).unwrap();

    let check = FileCheck::parse(
        "received",
        serde_json::from_value(json!({
            "after": 1,
            "messages": [
                {"type": 0, "text": {"contains": "a"}},
                {"type": 0, "text": "alpha"}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        check
            .check_matched(
                &after,
                &selected_game_data(),
                &journal,
                4,
                &CheckContext::default()
            )
            .unwrap(),
        Some(vec![1, 2])
    );
}

#[test]
fn received_requires_enough_records_and_exact_packet_provider() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut journal = FileJournal::new();
    let snapshot = GameSnapshot::new();
    journal.observe(&snapshot, 1).unwrap();
    journal.mark(1, &snapshot, 1, 0).unwrap();
    let check = FileCheck::parse(
        "received",
        serde_json::from_value(json!({
            "after": 1,
            "stats": [
                {"skill": 0, "effective": 2},
                {"skill": 0, "effective": 2}
            ],
            "exact": "packet"
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(check
        .admit(&journal)
        .unwrap_err()
        .contains("packet provider"));

    journal.set_packet_provider(PacketKinds {
        stats: true,
        inventories: false,
        vars: false,
    });
    check.admit(&journal).unwrap();
    journal
        .observe_packet(PacketRecord::Stat {
            skill: 0,
            level: 2,
            xp: 20,
            source_serial: 1,
            application_serial: 1,
            client_session: 1,
        })
        .unwrap();
    assert!(check
        .check_matched(
            &snapshot,
            &selected_game_data(),
            &journal,
            1,
            &CheckContext::default()
        )
        .unwrap()
        .is_none());
}

#[test]
fn silent_waits_for_full_window_and_fails_on_forbidden_change_or_session_reset() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut journal = FileJournal::new();
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_tick(0);
    snapshot.seed_stats(vec![api::snapshot::StatView {
        index: 0,
        name: "attack".into(),
        effective: 1,
        base: 1,
        xp: 0,
        used: true,
    }]);
    journal.observe(&snapshot, 2).unwrap();
    journal.mark(1, &snapshot, 2, 0).unwrap();
    let check = FileCheck::parse(
        "silent",
        serde_json::from_value(json!({
            "after": 1,
            "ticks": 2,
            "stats": [{"skill": 0}]
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(check
        .check_matched(
            &snapshot,
            &selected_game_data(),
            &journal,
            2,
            &CheckContext::default()
        )
        .unwrap()
        .is_none());
    snapshot.seed_tick(2);
    assert!(check
        .check_matched(
            &snapshot,
            &selected_game_data(),
            &journal,
            2,
            &CheckContext::default()
        )
        .unwrap()
        .is_some());
    assert!(check.limitation().is_some());

    let mut forbidden = GameSnapshot::new();
    forbidden.seed_ingame(2);
    forbidden.seed_stats(vec![api::snapshot::StatView {
        index: 0,
        name: "attack".into(),
        effective: 2,
        base: 1,
        xp: 0,
        used: true,
    }]);
    journal.observe(&forbidden, 2).unwrap();
    forbidden.seed_tick(1);
    assert!(check
        .check_matched(
            &forbidden,
            &selected_game_data(),
            &journal,
            2,
            &CheckContext::default()
        )
        .is_err());
    assert!(check
        .check_matched(
            &forbidden,
            &selected_game_data(),
            &journal,
            3,
            &CheckContext::default()
        )
        .is_err());
}

#[test]
fn count_and_order_use_exclusive_marks_and_reject_incomparable_labels() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut journal = FileJournal::new();

    journal.set_packet_provider(PacketKinds {
        stats: false,
        inventories: false,
        vars: true,
    });
    let snapshot = GameSnapshot::new();
    let from = journal.mark(1, &snapshot, 7, 0).unwrap();
    journal
        .observe_packet(PacketRecord::Varp {
            id: 8,
            value: 0,
            source_serial: 1,
            application_serial: 1,
            client_session: 7,
        })
        .unwrap();
    let to = journal.mark(2, &snapshot, 7, 0).unwrap();
    let count = FileCheck::parse(
        "count",
        serde_json::from_value(json!({
            "record": {"kind": "var", "id": 8},
            "from": 1,
            "to": 2,
            "amount": {"eq": 1},
            "exact": "packet"
        }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(from.serial, 0);
    assert_eq!(to.serial, 1);
    assert_eq!(
        count
            .check_matched(
                &snapshot,
                &selected_game_data(),
                &journal,
                7,
                &CheckContext::default()
            )
            .unwrap(),
        Some(vec![1])
    );

    let mut context = CheckContext::default();
    context.record("first", journal.actor_id(), 7, 1, vec![1]);
    context.record("then", journal.actor_id(), 7, 3, vec![2]);
    let order = FileCheck::parse(
        "order",
        serde_json::from_value(json!({
            "first": "first",
            "then": "then",
            "gap": {"min": 2}
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(order
        .check_matched(&snapshot, &selected_game_data(), &journal, 7, &context)
        .unwrap()
        .is_some());
    context.record("then", journal.actor_id() + 1, 7, 3, vec![2]);
    assert!(order
        .check_matched(&snapshot, &selected_game_data(), &journal, 7, &context)
        .unwrap_err()
        .contains("incomparable"));
}

#[test]
fn var_requires_a_packet_observation_even_for_snapshot_zero() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_varps(vec![api::snapshot::VarpView {
        index: 17,
        value: 0,
    }]);
    let mut journal = FileJournal::new();
    journal.observe(&snapshot, 3).unwrap();
    let check = FileCheck::parse(
        "var",
        serde_json::from_value(json!({"id": 17, "value": 0})).unwrap(),
    )
    .unwrap();
    assert!(check
        .admit(&journal)
        .unwrap_err()
        .contains("packet provider"));

    journal.set_packet_provider(PacketKinds {
        stats: false,
        inventories: false,
        vars: true,
    });
    check.admit(&journal).unwrap();
    assert!(check
        .check_matched(
            &snapshot,
            &selected_game_data(),
            &journal,
            3,
            &CheckContext::default()
        )
        .unwrap()
        .is_none());
    journal
        .observe_packet(PacketRecord::Varp {
            id: 17,
            value: 0,
            source_serial: 1,
            application_serial: 1,
            client_session: 3,
        })
        .unwrap();
    journal
        .observe_packet(PacketRecord::Varp {
            id: 17,
            value: 0,
            source_serial: 2,
            application_serial: 2,
            client_session: 3,
        })
        .unwrap();
    assert_eq!(journal.record_count(RecordKind::Varp), 2);
    assert!(check
        .check_matched(
            &snapshot,
            &selected_game_data(),
            &journal,
            3,
            &CheckContext::default()
        )
        .unwrap()
        .is_some());
}

#[test]
fn packet_history_gaps_reject_prior_marks_but_not_later_windows() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    let mut journal = FileJournal::new();
    journal.set_packet_provider(PacketKinds {
        stats: true,
        inventories: false,
        vars: false,
    });
    journal.observe(&snapshot, 7).unwrap();
    journal.mark(1, &snapshot, 7, 0).unwrap();
    journal.note_packet_gap(PacketKinds {
        stats: true,
        inventories: false,
        vars: false,
    });
    let args = json!({
        "after": 1,
        "stats": [{"skill": 0, "effective": 2}],
        "exact": "packet"
    });
    let before_gap =
        FileCheck::parse("received", serde_json::from_value(args.clone()).unwrap()).unwrap();
    assert!(before_gap.admit(&journal).is_ok());
    assert!(before_gap
        .check_matched(
            &snapshot,
            &selected_game_data(),
            &journal,
            7,
            &CheckContext::default()
        )
        .unwrap_err()
        .contains("packet history gap"));

    journal.mark(2, &snapshot, 7, 0).unwrap();
    journal
        .observe_packet(PacketRecord::Stat {
            skill: 0,
            level: 2,
            xp: 200,
            source_serial: 1,
            application_serial: 1,
            client_session: 7,
        })
        .unwrap();
    let after_gap = FileCheck::parse(
        "received",
        serde_json::from_value(json!({
            "after": 2,
            "stats": [{"skill": 0, "effective": 2}],
            "exact": "packet"
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(after_gap
        .check_matched(
            &snapshot,
            &selected_game_data(),
            &journal,
            7,
            &CheckContext::default()
        )
        .unwrap()
        .is_some());
}

#[test]
fn unavailable_file_predicates_have_named_refusals() {
    for (name, args, diagnostic) in [
        ("login_packets", json!({"family": "varp"}), "login_packets"),
        ("varp", json!({"id": 1, "min": 0}), "varp"),
        ("varp_exact", json!({"id": 1, "value": 0}), "varp_exact"),
        ("logged_out", json!({}), "runner-owned"),
        (
            "script_receipt",
            json!({"prefix": "receipt"}),
            "script_receipt",
        ),
        ("script_running", json!({}), "script_running"),
        ("script_idle", json!({}), "script_idle"),
        (
            "clue_protect_window",
            json!({"seeded": 1}),
            "clue_protect_window",
        ),
        (
            "world_speed_changed",
            json!({"ms": 600}),
            "world_speed_changed",
        ),
        ("hint", json!({"kind": "none"}), "not observable"),
    ] {
        let parsed = FileCheck::parse(name, serde_json::from_value(args).unwrap());
        assert!(
            parsed.unwrap_err().contains(diagnostic),
            "{name} did not name its unavailable source"
        );
    }
}

fn item_view(id: i32, count: i32, slot: i32, container: api::snapshot::ItemContainer) -> ItemView {
    ItemView {
        def: api::obj_names::ItemDefView {
            id,
            name: None,
            stackable: false,
            members: false,
            base_value: 0,
            noted: false,
            certificate_link: -1,
            certificate_template: -1,
        },
        container,
        action_family: api::snapshot::ItemActionFamily::Held,
        slot,
        count,
        actions: vec![],
        component_id: 149,
    }
}

fn npc_view(
    name: &str,
    kind: usize,
    tile: WorldTile,
    target: Option<api::snapshot::ActorTargetView>,
) -> api::snapshot::NpcView {
    api::snapshot::NpcView {
        index: 3,
        r#type: Some(kind),
        name: Some(name.into()),
        actions: vec![],
        tile,
        distance: 0,
        animation: -1,
        animation_frame: 0,
        pose_animation: -1,
        orientation: 0,
        target_orientation: 0,
        overhead_text: None,
        spot_animation: -1,
        spot_animation_stamp: 0,
        health: 10,
        total_health: 10,
        face_entity: -1,
        target,
        moving: false,
        running: false,
        in_combat: false,
        level: tile.level,
        size: 1,
        network: tile,
        x: tile.x * 128 + 64,
        z: tile.z * 128 + 64,
        yaw: 0,
    }
}

fn loc_view(id: i32, tile: WorldTile, actions: &[&str]) -> api::snapshot::LocView {
    api::snapshot::LocView {
        typecode: id,
        info: 0,
        id,
        name: None,
        description: None,
        actions: actions
            .iter()
            .map(|action| Some((*action).into()))
            .collect(),
        tile,
        distance: 0,
        layer: api::snapshot::LocLayer::Ground,
        shape: 10,
        angle: 0,
        width: 1,
        length: 1,
        footprint_width: 1,
        footprint_length: 1,
        block_walk: false,
        block_range: false,
        active: true,
        animation: -1,
        map_function: -1,
        map_scene: -1,
        force_approach: -1,
    }
}

fn quest_widget(text: &str) -> api::snapshot::WidgetView {
    api::snapshot::WidgetView {
        kind: api::snapshot::WidgetKind::Widget,
        component_id: 301,
        layer_id: 0,
        parent_id: -1,
        root_component_id: 297,
        root: api::snapshot::WidgetRoot::Main,
        type_: 0,
        button_type: 0,
        client_code: 0,
        x: 0,
        y: 0,
        width: 0,
        height: 0,
        scroll_height: 0,
        inventory_ready: false,
        scroll_position: 0,
        hidden: false,
        text: Some(text.into()),
        alternate_text: None,
        button_text: None,
        target_verb: None,
        target_base: None,
        target_mask: 0,
        model_type: 0,
        model_id: 0,
        alternate_model_type: 0,
        alternate_model_id: 0,
        scripts: None,
        script_comparators: None,
        script_operands: None,
        varp_bindings: vec![],
        colour: 0,
        actions: vec![],
        items: vec![],
    }
}

fn targeted_npc_snapshot(local_targets_npc: bool) -> GameSnapshot {
    use api::snapshot::{ActorKind, ActorTargetView, ActorView, LocalPlayerView, PlayerView};

    let tile = WorldTile {
        x: 3210,
        z: 3210,
        level: 0,
    };
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_tile(tile);
    let npc_target = (!local_targets_npc).then_some(ActorTargetView {
        kind: ActorKind::Player,
        index: 4,
    });
    snapshot.seed_npcs(vec![npc_view("Target", 1, tile, npc_target)]);
    let local_target = local_targets_npc.then_some(ActorTargetView {
        kind: ActorKind::Npc,
        index: 3,
    });
    snapshot.seed_local_player(LocalPlayerView {
        player: PlayerView {
            index: 4,
            actor: ActorView {
                name: Some("alice".into()),
                actions: vec![],
                tile,
                distance: 0,
                animation: -1,
                animation_frame: 0,
                pose_animation: -1,
                orientation: 0,
                target_orientation: 0,
                overhead_text: None,
                spot_animation: -1,
                spot_animation_stamp: 0,
                health: 10,
                total_health: 10,
                face_entity: -1,
                target: local_target,
                moving: false,
                running: false,
                in_combat: false,
            },
            network: tile,
            combat_level: 0,
            skill_level: 0,
            headicons: 0,
            weapon: None,
        },
        energy: 0,
        weight: 0,
        animation_update: None,
    });
    snapshot
}

#[test]
fn file_dialogue_npc_loc_and_hint_checks_match_only_selected_snapshot_fields() {
    if !api::test_support::game_data_available() {
        return;
    }
    let selected = selected_game_data();
    let mut dialogue = GameSnapshot::new();
    dialogue.seed_ingame(2);
    dialogue.seed_chat_modal(297, vec!["Choose a response".into()]);
    dialogue.seed_chat_options(
        vec![api::snapshot::ChatOptionView {
            component_id: 301,
            text: "Continue".into(),
        }],
        302,
    );
    let dialogue_check = FileCheck::parse(
        "dialogue",
        serde_json::from_value(json!({
            "open": true,
            "root": 297,
            "continue": 302,
            "lines": ["Choose a response"],
            "options": ["Continue"]
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(dialogue_check
        .check(&dialogue, &selected, &FileJournal::new(), 1)
        .unwrap());
    let wrong_dialogue = FileCheck::parse(
        "dialogue",
        serde_json::from_value(json!({"root": 298})).unwrap(),
    )
    .unwrap();
    assert!(!wrong_dialogue
        .check(&dialogue, &selected, &FileJournal::new(), 1)
        .unwrap());

    let tile = WorldTile {
        x: 10,
        z: 10,
        level: 0,
    };
    let mut world = GameSnapshot::new();
    world.seed_ingame(2);
    world.seed_tile(tile);
    let mut guide = npc_view("Guide", 1, WorldTile { x: 11, ..tile }, None);
    guide.actions = vec![Some("Talk-to".into())];
    world.seed_npcs(vec![guide]);
    let npc_check = FileCheck::parse(
        "npc",
        serde_json::from_value(json!({
            "type": 1,
            "near": {"x": 10, "z": 10, "level": 0},
            "radius": 1,
            "index": 3,
            "form": 1,
            "options": ["Talk-to"],
            "no_option": "Attack",
            "sequence": -1,
            "frame": 0
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(npc_check
        .check(&world, &selected, &FileJournal::new(), 1)
        .unwrap());
    let absent_npc =
        FileCheck::parse("npc", serde_json::from_value(json!({"type": 2})).unwrap()).unwrap();
    assert!(!absent_npc
        .check(&world, &selected, &FileJournal::new(), 1)
        .unwrap());

    let mut booth = loc_view(55, tile, &["Bank"]);
    booth.name = Some("Bank booth".into());
    world.seed_locs(vec![booth]);
    let loc_check = FileCheck::parse(
        "loc",
        serde_json::from_value(json!({
            "at": {"x": 10, "z": 10, "level": 0},
            "name": "Bank booth",
            "form": 55,
            "options": ["Bank"],
            "first_option": "Bank",
            "no_option": "Open"
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(loc_check
        .check(&world, &selected, &FileJournal::new(), 1)
        .unwrap());
    let wrong_loc = FileCheck::parse(
        "loc",
        serde_json::from_value(json!({
            "at": {"x": 10, "z": 10, "level": 0},
            "first_option": "Open"
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(!wrong_loc
        .check(&world, &selected, &FileJournal::new(), 1)
        .unwrap());

    let hint = api::snapshot::HintTileView {
        x: 11,
        z: 12,
        level: 0,
    };
    assert!(hint_matches(Some(&hint), 10, 10, 0, 2));
    assert!(!hint_matches(Some(&hint), 10, 10, 1, 2));
    let hint_check = FileCheck::parse(
        "hint",
        serde_json::from_value(json!({
            "kind": "tile",
            "at": {"x": 10, "z": 10, "level": 0},
            "radius": 2
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(!hint_check
        .check(&world, &selected, &FileJournal::new(), 1)
        .unwrap());
}

#[test]
fn file_gameplay_classification_excludes_observation_only_checks() {
    if !api::test_support::game_data_available() {
        return;
    }
    let checks = [
        FileCheck::parse(
            "action_sent",
            serde_json::from_value(json!({"after": 1})).unwrap(),
        )
        .unwrap(),
        FileCheck::parse(
            "signal",
            serde_json::from_value(json!({"name": "ready"})).unwrap(),
        )
        .unwrap(),
        FileCheck::Native(Proof::IngameScene2),
        FileCheck::Native(Proof::TutorialClosed),
        FileCheck::Native(Proof::RenderViewReady {
            x: 0,
            z: 0,
            level: 0,
            orbit_yaw: 0,
            orbit_pitch: 0,
        }),
        FileCheck::Native(Proof::LoggedOut),
    ];
    let signal = FileCheck::parse(
        "signal",
        serde_json::from_value(json!({"name": "ready"})).unwrap(),
    )
    .unwrap();
    let selected = selected_game_data();
    let journal = FileJournal::new();
    let mut context = CheckContext::default();
    assert!(signal
        .check_matched(&GameSnapshot::new(), &selected, &journal, 1, &context)
        .unwrap()
        .is_none());
    context.signal("ready");
    assert!(signal
        .check_matched(&GameSnapshot::new(), &selected, &journal, 1, &context)
        .unwrap()
        .is_some());
    assert!(checks.iter().all(|check| !check.is_gameplay()));
}

fn proof_holds(name: &str, args: Value, snapshot: &GameSnapshot, journal: &FileJournal) -> bool {
    FileCheck::parse(name, serde_json::from_value(args).unwrap())
        .unwrap()
        .check(snapshot, &selected_game_data(), journal, 1)
        .unwrap()
}

#[test]
fn appendix_a_file_proofs_pass_and_fail_against_observed_snapshots() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut journal = FileJournal::new();
    let mut inventory = GameSnapshot::new();
    inventory.seed_ingame(2);
    inventory.seed_inventory(
        vec![item_view(10, 2, 0, api::snapshot::ItemContainer::Inventory)],
        28,
    );
    inventory.seed_transmission_ready(true, false);
    assert!(proof_holds(
        "item_id_any",
        json!({"ids": [10, 11], "count": 2}),
        &inventory,
        &journal,
    ));
    assert!(!proof_holds(
        "item_id_any",
        json!({"ids": [11], "count": 1}),
        &inventory,
        &journal,
    ));

    let mut bank = GameSnapshot::new();
    bank.seed_ingame(2);
    bank.seed_bank_observation(
        601,
        1,
        Some(vec![item_view(
            10,
            2,
            0,
            api::snapshot::ItemContainer::Bank,
        )]),
        vec![],
    );
    assert!(proof_holds(
        "bank_item_id_any",
        json!({"ids": [10, 11], "count": 2}),
        &bank,
        &journal,
    ));
    let mut closed_bank = GameSnapshot::new();
    closed_bank.seed_ingame(2);
    assert!(!proof_holds(
        "bank_item_id_any",
        json!({"ids": [10], "count": 1}),
        &closed_bank,
        &journal,
    ));

    let mut quest = GameSnapshot::new();
    quest.seed_ingame(2);
    quest.seed_quest_statuses(
        vec![api::snapshot::QuestStatusView {
            component_id: 702,
            name: "Lost City".into(),
            colour: 0xF800,
        }],
        true,
    );
    assert!(proof_holds(
        "quest_done",
        json!({"name": "Lost City"}),
        &quest,
        &journal
    ));
    let mut unfinished = GameSnapshot::new();
    unfinished.seed_ingame(2);
    unfinished.seed_quest_statuses(
        vec![api::snapshot::QuestStatusView {
            component_id: 702,
            name: "Lost City".into(),
            colour: 0xF80000,
        }],
        true,
    );
    assert!(!proof_holds(
        "quest_done",
        json!({"name": "Lost City"}),
        &unfinished,
        &journal,
    ));

    let center = WorldTile {
        x: 10,
        z: 10,
        level: 0,
    };
    let mut npc_world = GameSnapshot::new();
    npc_world.seed_ingame(2);
    npc_world.seed_tile(center);
    npc_world.seed_npcs(vec![npc_view(
        "Guide",
        1,
        WorldTile { x: 11, ..center },
        None,
    )]);
    assert!(proof_holds(
        "npc_ids_absent_near",
        json!({"ids": [2], "radius": 2}),
        &npc_world,
        &journal,
    ));
    assert!(!proof_holds(
        "npc_ids_absent_near",
        json!({"ids": [1], "radius": 2}),
        &npc_world,
        &journal,
    ));

    let mut ground_clear = GameSnapshot::new();
    ground_clear.seed_ingame(2);
    ground_clear.seed_tile(center);
    assert!(proof_holds(
        "ground_item_ids_absent_near",
        json!({"ids": [42], "x": 10, "z": 10, "level": 0, "radius": 2}),
        &ground_clear,
        &journal,
    ));
    let mut ground_present = GameSnapshot::new();
    ground_present.seed_ingame(2);
    ground_present.seed_tile(center);
    ground_present.seed_ground_items(vec![api::snapshot::GroundItemView {
        def: api::obj_names::ItemDefView {
            id: 42,
            name: Some("Sample".into()),
            stackable: false,
            members: false,
            base_value: 0,
            noted: false,
            certificate_link: -1,
            certificate_template: -1,
        },
        count: 1,
        actions: vec![],
        tile: center,
        distance: 0,
    }]);
    assert!(!proof_holds(
        "ground_item_ids_absent_near",
        json!({"ids": [42], "x": 10, "z": 10, "level": 0, "radius": 2}),
        &ground_present,
        &journal,
    ));
    assert!(proof_holds(
        "npc_name_near",
        json!({"name": "Guide", "x": 10, "z": 10, "level": 0, "radius": 2}),
        &npc_world,
        &journal,
    ));
    assert!(!proof_holds(
        "npc_name_near",
        json!({"name": "Guard", "x": 10, "z": 10, "level": 0, "radius": 2}),
        &npc_world,
        &journal,
    ));

    let mut target_npc = targeted_npc_snapshot(false);
    assert!(proof_holds(
        "npc_name_targeting_local",
        json!({"name": "Target"}),
        &target_npc,
        &journal,
    ));
    assert!(!proof_holds(
        "local_targeting_npc_name",
        json!({"name": "Target"}),
        &target_npc,
        &journal,
    ));
    target_npc = targeted_npc_snapshot(true);
    assert!(!proof_holds(
        "npc_name_targeting_local",
        json!({"name": "Target"}),
        &target_npc,
        &journal,
    ));
    assert!(proof_holds(
        "local_targeting_npc_name",
        json!({"name": "Target"}),
        &target_npc,
        &journal,
    ));

    let mut guard = GameSnapshot::new();
    guard.seed_ingame(2);
    guard.seed_tile(WorldTile {
        x: 2,
        z: 5,
        level: 0,
    });
    guard.seed_scene(api::snapshot::SceneView {
        available: true,
        base_x: 0,
        base_z: 0,
        level: 0,
        width: 12,
        height: 12,
        collision_flags: vec![0; 144],
    });
    guard.seed_npcs(vec![npc_view(
        "Guard",
        1,
        WorldTile {
            x: 5,
            z: 5,
            level: 0,
        },
        None,
    )]);
    assert!(proof_holds(
        "npc_name_unengaged_in_sight",
        json!({"name": "Guard", "radius": 5}),
        &guard,
        &journal,
    ));
    let mut row = guard.npcs()[0].clone();
    row.in_combat = true;
    let mut busy_guard = GameSnapshot::new();
    busy_guard.seed_ingame(2);
    busy_guard.seed_tile(WorldTile {
        x: 2,
        z: 5,
        level: 0,
    });
    busy_guard.seed_scene(api::snapshot::SceneView {
        available: true,
        base_x: 0,
        base_z: 0,
        level: 0,
        width: 12,
        height: 12,
        collision_flags: vec![0; 144],
    });
    busy_guard.seed_npcs(vec![row]);
    assert!(!proof_holds(
        "npc_name_unengaged_in_sight",
        json!({"name": "Guard", "radius": 5}),
        &busy_guard,
        &journal,
    ));

    let mut locs = GameSnapshot::new();
    locs.seed_ingame(2);
    locs.seed_locs(vec![loc_view(55, WorldTile { x: 11, ..center }, &["Bank"])]);
    assert!(proof_holds(
        "loc_action_near",
        json!({"id": 55, "x": 10, "z": 10, "level": 0, "radius": 2, "action": "bank", "present": true}),
        &locs,
        &journal,
    ));
    assert!(!proof_holds(
        "loc_action_near",
        json!({"id": 55, "x": 10, "z": 10, "level": 0, "radius": 2, "action": "bank", "present": false}),
        &locs,
        &journal,
    ));

    let baseline_chat = |sequence| {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_chat_lines(vec![api::snapshot::ChatLineView {
            type_: 0,
            username: None,
            text: "old".into(),
            sequence,
        }]);
        snapshot
    };
    let before = baseline_chat(1);
    journal.mark(1, &before, 1, 0).unwrap();
    let mut fresh = baseline_chat(2);
    fresh.seed_chat_lines(vec![api::snapshot::ChatLineView {
        type_: 0,
        username: None,
        text: "get tutorial: 1000 and a fresh quest complete".into(),
        sequence: 2,
    }]);
    assert!(proof_holds(
        "fresh_tutorial",
        json!({"since": 1}),
        &fresh,
        &journal,
    ));
    assert!(proof_holds(
        "fresh_chat",
        json!({"needle": "fresh quest", "since": 1}),
        &fresh,
        &journal,
    ));
    assert!(proof_holds(
        "fresh_chat_since_previous_step",
        json!({"needle": "fresh quest", "since": 1}),
        &fresh,
        &journal,
    ));
    assert!(!proof_holds(
        "fresh_tutorial",
        json!({"since": 1}),
        &before,
        &journal,
    ));
    assert!(!proof_holds(
        "fresh_chat",
        json!({"needle": "fresh quest", "since": 1}),
        &before,
        &journal,
    ));
    assert!(proof_holds(
        "chat",
        json!({"needle": "fresh quest"}),
        &fresh,
        &journal,
    ));
    assert!(!proof_holds(
        "chat",
        json!({"needle": "absent"}),
        &fresh,
        &journal,
    ));

    let stat = |xp| api::snapshot::StatView {
        index: 0,
        name: "attack".into(),
        effective: 1,
        base: 1,
        xp,
        used: true,
    };
    let mut stat_before = GameSnapshot::new();
    stat_before.seed_ingame(2);
    stat_before.seed_stats(vec![stat(100)]);
    journal.mark(2, &stat_before, 1, 0).unwrap();
    let mut stat_after = GameSnapshot::new();
    stat_after.seed_ingame(2);
    stat_after.seed_stats(vec![stat(125)]);
    for name in ["stat_xp_gain", "fresh_stat_xp_gain", "xp_delta"] {
        assert!(proof_holds(
            name,
            json!({"id": 0, "min": 20, "since": 2}),
            &stat_after,
            &journal,
        ));
        assert!(!proof_holds(
            name,
            json!({"id": 0, "min": 30, "since": 2}),
            &stat_after,
            &journal,
        ));
    }

    let mut items_before = GameSnapshot::new();
    items_before.seed_ingame(2);
    items_before.seed_inventory(
        vec![item_view(20, 5, 0, api::snapshot::ItemContainer::Inventory)],
        28,
    );
    items_before.seed_transmission_ready(true, false);
    journal.mark(3, &items_before, 1, 0).unwrap();
    let mut product_after = GameSnapshot::new();
    product_after.seed_ingame(2);
    product_after.seed_inventory(
        vec![item_view(20, 7, 0, api::snapshot::ItemContainer::Inventory)],
        28,
    );
    product_after.seed_transmission_ready(true, false);
    let mut consumed_after = GameSnapshot::new();
    consumed_after.seed_ingame(2);
    consumed_after.seed_inventory(
        vec![item_view(20, 3, 0, api::snapshot::ItemContainer::Inventory)],
        28,
    );
    consumed_after.seed_transmission_ready(true, false);
    for name in ["product_delta", "input_consumed"] {
        let snapshot = if name == "product_delta" {
            &product_after
        } else {
            &consumed_after
        };
        assert!(proof_holds(
            name,
            json!({"id": 20, "count": 2, "since": 3}),
            snapshot,
            &journal,
        ));
        assert!(!proof_holds(
            name,
            json!({"id": 20, "count": 4, "since": 3}),
            snapshot,
            &journal,
        ));
    }

    let selected = selected_game_data();
    let clue = selected
        .items()
        .iter()
        .find(|item| item.name.as_deref() == Some("Clue scroll"))
        .unwrap()
        .id;
    let casket = selected
        .items()
        .iter()
        .find(|item| item.name.as_deref() == Some("Casket"))
        .unwrap()
        .id;
    let mut clue_before = GameSnapshot::new();
    clue_before.seed_ingame(2);
    clue_before.seed_inventory(
        vec![item_view(
            clue,
            1,
            0,
            api::snapshot::ItemContainer::Inventory,
        )],
        28,
    );
    clue_before.seed_transmission_ready(true, false);
    journal.mark(4, &clue_before, 1, 0).unwrap();
    let mut clue_after = GameSnapshot::new();
    clue_after.seed_ingame(2);
    clue_after.seed_inventory(
        vec![item_view(
            casket,
            1,
            0,
            api::snapshot::ItemContainer::Inventory,
        )],
        28,
    );
    clue_after.seed_transmission_ready(true, false);
    assert!(proof_holds(
        "clue_replaced",
        json!({"seeded": clue, "since": 4}),
        &clue_after,
        &journal,
    ));
    assert!(!proof_holds(
        "clue_replaced",
        json!({"seeded": clue, "since": 4}),
        &clue_before,
        &journal,
    ));

    let mut completion_before = baseline_chat(1);
    completion_before.seed_main_modal(297, vec![quest_widget("You completed the quest!")]);
    journal.mark(5, &completion_before, 1, 0).unwrap();
    let mut completion = baseline_chat(2);
    completion.seed_chat_lines(vec![api::snapshot::ChatLineView {
        type_: 0,
        username: None,
        text: "Congratulations! Quest complete!".into(),
        sequence: 2,
    }]);
    completion.seed_main_modal(297, vec![quest_widget("You completed the quest!")]);
    let completion_args = json!({
        "chat_needle": "Quest complete",
        "main_root_component_id": 297,
        "component_id": 301,
        "text": "You completed the quest!",
        "since": 5
    });
    assert!(proof_holds(
        "fresh_quest_completion",
        completion_args.clone(),
        &completion,
        &journal,
    ));
    assert!(!proof_holds(
        "fresh_quest_completion",
        completion_args,
        &completion_before,
        &journal,
    ));

    assert!(proof_holds(
        "visited_within",
        json!({"min_x": 10, "max_x": 12, "min_z": 9, "max_z": 11, "level": 0}),
        &npc_world,
        &journal,
    ));
    let mut outside = GameSnapshot::new();
    outside.seed_ingame(2);
    outside.seed_tile(WorldTile { x: 13, ..center });
    assert!(!proof_holds(
        "visited_within",
        json!({"min_x": 10, "max_x": 12, "min_z": 9, "max_z": 11, "level": 0}),
        &outside,
        &journal,
    ));
}

/// Appendix C writes `inventories = { "inventory", "worn" }`, `stats = { "firemaking" }`
/// and `inventories = true`; an unknown container or skill name stays refused.
#[test]
fn appendix_c_matcher_shorthands_parse_and_unknown_names_fail() {
    let parse = |name: &str, value: serde_json::Value| {
        FileCheck::parse(name, serde_json::from_value(value).unwrap())
    };
    assert!(parse(
        "received",
        json!({"after": 1, "inventories": ["inventory", "worn"]})
    )
    .is_ok());
    assert!(parse(
        "silent",
        json!({"after": 1, "ticks": 3, "stats": ["firemaking"], "inventories": true})
    )
    .is_ok());
    assert!(parse("received", json!({"after": 1, "inventories": ["pocket"]})).is_err());
    assert!(parse(
        "silent",
        json!({"after": 1, "ticks": 3, "stats": ["juggling"]})
    )
    .is_err());
    assert!(parse("silent", json!({"after": 1, "ticks": 3, "stats": true})).is_err());
}

/// Failure mode: a bank or explicit-component selector is admitted although
/// no provider observes that domain, so an absence check passes on empty
/// history.
#[test]
fn inventory_matchers_refuse_domains_no_provider_observes() {
    let parse = |name: &str, value: serde_json::Value| {
        FileCheck::parse(name, serde_json::from_value(value).unwrap())
    };
    for selector in [
        json!("bank"),
        json!({"container": "bank", "item": 995}),
        json!({"container": 149}),
    ] {
        let silent = parse(
            "silent",
            json!({"after": 1, "ticks": 3, "inventories": [selector.clone()]}),
        )
        .unwrap_err();
        assert!(
            silent.contains("container must be inventory or worn"),
            "{silent}"
        );
        let received = parse(
            "received",
            json!({"after": 1, "inventories": [selector.clone()], "exact": "packet"}),
        )
        .unwrap_err();
        assert!(
            received.contains("container must be inventory or worn"),
            "{received}"
        );
        let mut record = selector.as_object().cloned().unwrap_or_else(|| {
            serde_json::Map::from_iter([("container".to_owned(), selector.clone())])
        });
        record.insert("kind".into(), json!("inventory"));
        let count = parse(
            "count",
            json!({"record": record, "from": 1, "amount": {"eq": 0}, "exact": "packet"}),
        )
        .unwrap_err();
        assert!(
            count.contains("container must be inventory or worn"),
            "{count}"
        );
    }
    assert!(parse(
        "received",
        json!({"after": 1, "inventories": [{"component": 149, "item": 995}]})
    )
    .is_err());
    assert!(parse(
        "silent",
        json!({"after": 1, "ticks": 3, "inventories": [{"container": "worn", "slot": 0}]})
    )
    .is_ok());
}

fn inventory_marker(
    form: crate::file_journal::PacketInventoryForm,
    source_serial: u64,
) -> PacketRecord {
    PacketRecord::InventoryReset {
        carried: true,
        component: 149,
        form,
        source_serial,
        application_serial: source_serial,
        client_session: 7,
    }
}

fn inventory_packet(slot: i32, count: i32, source_serial: u64) -> PacketRecord {
    PacketRecord::InventorySlot {
        carried: true,
        component: 149,
        slot,
        id: Some(995),
        count,
        form: crate::file_journal::PacketInventoryForm::Partial,
        source_serial,
        application_serial: source_serial,
        client_session: 7,
    }
}

/// Failure modes: a packet slot event with no observed prior state reads as
/// an empty slot, so a repeated count of 10 satisfies `delta = 10`; a gap
/// keeps a pre-gap count as the baseline of the next event.
#[test]
fn packet_inventory_delta_needs_an_observed_same_epoch_baseline() {
    if !api::test_support::game_data_available() {
        return;
    }
    let selected = selected_game_data();
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    let mut journal = FileJournal::new();
    let inventories = PacketKinds {
        stats: false,
        inventories: true,
        vars: false,
    };
    journal.set_packet_provider(inventories);
    journal.observe(&snapshot, 7).unwrap();
    journal.mark(1, &snapshot, 7, 0).unwrap();
    let delta = |after: u64, amount: i64| {
        FileCheck::parse(
            "received",
            serde_json::from_value(json!({
                "after": after,
                "inventories": [{"item": 995, "delta": {"eq": amount}}],
                "exact": "packet"
            }))
            .unwrap(),
        )
        .unwrap()
    };
    let matched = |journal: &FileJournal, check: &FileCheck| {
        check
            .check_matched(&snapshot, &selected, journal, 7, &CheckContext::default())
            .unwrap()
            .is_some()
    };

    use crate::file_journal::PacketInventoryForm;

    // Mirror the real client->adapter stream: each Partial packet emits a
    // Partial marker before its updated slot records.
    journal
        .observe_packet(inventory_marker(PacketInventoryForm::Partial, 1))
        .unwrap();
    journal.observe_packet(inventory_packet(0, 10, 2)).unwrap();
    assert!(
        !matched(&journal, &delta(1, 10)),
        "first event has no baseline"
    );
    journal
        .observe_packet(inventory_marker(PacketInventoryForm::Partial, 3))
        .unwrap();
    journal.observe_packet(inventory_packet(0, 10, 4)).unwrap();
    assert!(
        !matched(&journal, &delta(1, 10)),
        "repeated 10 -> 10 is not +10"
    );
    assert!(matched(&journal, &delta(1, 0)));
    assert!(!matched(&journal, &delta(1, 3)));
    journal
        .observe_packet(inventory_marker(PacketInventoryForm::Partial, 5))
        .unwrap();
    journal.observe_packet(inventory_packet(0, 13, 6)).unwrap();
    assert!(
        matched(&journal, &delta(1, 3)),
        "observed 10 -> 13 across Partial marker is +3"
    );

    // A Full redraw clears the container's baselines.
    journal
        .observe_packet(inventory_marker(PacketInventoryForm::Full, 7))
        .unwrap();
    journal.mark(2, &snapshot, 7, 0).unwrap();
    journal
        .observe_packet(inventory_marker(PacketInventoryForm::Partial, 8))
        .unwrap();
    journal.observe_packet(inventory_packet(0, 16, 9)).unwrap();
    assert!(
        !matched(&journal, &delta(2, 3)),
        "13 is not a baseline after a Full redraw"
    );

    // A provider gap also clears baselines.
    journal.note_packet_gap(inventories);
    journal.mark(3, &snapshot, 7, 0).unwrap();
    journal
        .observe_packet(inventory_marker(PacketInventoryForm::Partial, 10))
        .unwrap();
    journal.observe_packet(inventory_packet(0, 19, 11)).unwrap();
    assert!(
        !matched(&journal, &delta(3, 3)),
        "16 is not a baseline across the gap"
    );
    journal
        .observe_packet(inventory_marker(PacketInventoryForm::Partial, 12))
        .unwrap();
    journal.observe_packet(inventory_packet(0, 22, 13)).unwrap();
    assert!(matched(&journal, &delta(3, 3)), "observed 19 -> 22 is +3");
}

/// Failure mode: after a seed releases the provider and rearms it, `var`
/// accepts the pre-gap varp value although unobserved packets may have
/// changed it.
#[test]
fn var_does_not_reuse_a_pre_gap_witness() {
    if !api::test_support::game_data_available() {
        return;
    }
    let selected = selected_game_data();
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    let mut journal = FileJournal::new();
    let vars = PacketKinds {
        stats: false,
        inventories: false,
        vars: true,
    };
    journal.set_packet_provider(vars);
    journal.observe(&snapshot, 3).unwrap();
    let var = |value: i32| {
        FileCheck::parse(
            "var",
            serde_json::from_value(json!({"id": 17, "value": value})).unwrap(),
        )
        .unwrap()
    };
    let matched = |journal: &FileJournal, check: &FileCheck| {
        check
            .check_matched(&snapshot, &selected, journal, 3, &CheckContext::default())
            .unwrap()
            .is_some()
    };
    let varp = |value: i32, serial: u64| PacketRecord::Varp {
        id: 17,
        value,
        source_serial: serial,
        application_serial: serial,
        client_session: 3,
    };

    journal.observe_packet(varp(5, 1)).unwrap();
    assert!(matched(&journal, &var(5)));

    // A seed releases the provider (gap) and rearms it.
    journal.set_packet_provider(PacketKinds::default());
    journal.note_packet_gap(vars);
    journal.set_packet_provider(vars);
    assert!(!matched(&journal, &var(5)), "pre-gap value is not current");

    journal.observe_packet(varp(9, 2)).unwrap();
    assert!(matched(&journal, &var(9)));
    assert!(!matched(&journal, &var(5)));
}
