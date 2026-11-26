use super::*;
use api::obj_names::ItemDefView;
use api::snapshot::{
    ActorView, ChatLineView, ChatOptionView, ItemActionFamily, ItemContainer, ItemView, LocLayer,
    LocView, LocalPlayerView, NpcView, PlayerView, QuestStatusView, StatView, VarpView, WidgetKind,
    WidgetRoot, WidgetView, WorldTile,
};
use client::client::skill::Skill;
use client::client::{Client, ClientConfig};
use std::collections::BTreeSet;

fn item_view(id: i32, name: Option<&str>, count: i32, container: ItemContainer) -> ItemView {
    ItemView {
        def: ItemDefView {
            id,
            name: name.map(str::to_owned),
            stackable: false,
            members: false,
            base_value: 0,
            noted: false,
            certificate_link: -1,
            certificate_template: -1,
        },
        container,
        action_family: ItemActionFamily::Held,
        slot: 0,
        count,
        actions: Vec::new(),
        component_id: -1,
    }
}

fn stat_view(index: i32, base: i32, effective: i32, xp: i32) -> StatView {
    StatView {
        index,
        name: Skill::names
            .get(index as usize)
            .copied()
            .unwrap_or("")
            .to_owned(),
        effective,
        base,
        xp,
        used: true,
    }
}

fn actor_view(name: &str, tile: WorldTile) -> ActorView {
    ActorView {
        name: Some(name.to_owned()),
        actions: Vec::new(),
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
        health: 0,
        total_health: 0,
        face_entity: -1,
        target: None,
        moving: false,
        running: false,
        in_combat: false,
    }
}

fn player_view(index: usize, name: &str, tile: WorldTile) -> PlayerView {
    PlayerView {
        index,
        actor: actor_view(name, tile),
        network: tile,
        combat_level: 126,
        skill_level: 0,
        headicons: 0,
        weapon: None,
    }
}

fn local_player_view(name: &str, tile: WorldTile) -> LocalPlayerView {
    LocalPlayerView {
        player: player_view(0, name, tile),
        energy: 100,
        weight: 0,
        animation_update: None,
    }
}

fn widget_view(component_id: i32, text: &str) -> WidgetView {
    WidgetView {
        kind: WidgetKind::Widget,
        component_id,
        layer_id: -1,
        parent_id: -1,
        root_component_id: 6575,
        root: WidgetRoot::Main,
        type_: 4,
        button_type: 0,
        client_code: 0,
        x: 0,
        y: 0,
        width: 10,
        height: 10,
        scroll_height: 0,
        scroll_position: 0,
        inventory_ready: false,
        hidden: false,
        text: Some(text.to_owned()),
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
        varp_bindings: Vec::new(),
        colour: 0,
        actions: Vec::new(),
        items: Vec::new(),
    }
}

fn loc_view(id: i32, name: &str, tile: WorldTile, action: &str) -> LocView {
    LocView {
        typecode: 0,
        info: 0,
        id,
        name: Some(name.to_owned()),
        description: None,
        actions: vec![Some(action.to_owned())],
        tile,
        distance: 0,
        layer: LocLayer::Ground,
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
        force_approach: 0,
    }
}

fn npc_view(r#type: usize, tile: WorldTile, size: i32, health: i32, total_health: i32) -> NpcView {
    NpcView {
        index: 1,
        r#type: Some(r#type),
        name: Some("Kalphite Queen".into()),
        actions: vec![Some("Attack".into())],
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
        health,
        total_health,
        face_entity: -1,
        target: None,
        moving: false,
        running: false,
        in_combat: health > 0,
        level: 333,
        size,
        network: tile,
        x: 0,
        z: 0,
        yaw: 0,
    }
}

fn ingame_snapshot(tile: WorldTile, tick: u32) -> GameSnapshot {
    let mut snap = GameSnapshot::new();
    snap.seed_ingame(2);
    snap.seed_tick(tick);
    snap.seed_tile(tile);
    snap.seed_bank_observation(-1, 1, None, Vec::new());
    snap
}

fn shop_open_snapshot(tile: WorldTile, tick: u32) -> GameSnapshot {
    let mut client = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: crate::NO_CACHE_DIR.into(),
        members: true,
        lowmem: false,
    });
    client.ingame = true;
    client.scene_state = 2;
    client.main_modal_id = 3824;
    client.gens.iface = 1;
    let mut snap = GameSnapshot::new();
    snap.rebuild(&client);
    snap.seed_ingame(2);
    snap.seed_tick(tick);
    snap.seed_tile(tile);
    snap.seed_bank_observation(-1, 1, None, Vec::new());
    snap
}

#[test]
fn registry_covers_all_24_inventory_rows_and_duel_round_trip() {
    let descriptors = all_descriptors();
    assert_eq!(descriptors.len(), 25);
    let mut seen_names = BTreeSet::new();
    let mut seen_rows = BTreeSet::new();
    for desc in descriptors {
        assert!(seen_names.insert(desc.name), "duplicate name {}", desc.name);
        if let Some(row) = desc.inventory_row {
            assert!(seen_rows.insert(row), "duplicate inventory row {row}");
        }
        assert_eq!(lookup(desc.name).expect("lookup succeeds"), *desc);
    }
    let expected_rows: BTreeSet<u8> = (1..=24).collect();
    assert_eq!(seen_rows, expected_rows);
    assert_eq!(lookup("duel_round_trip").unwrap().inventory_row, None);
}

#[test]
fn unknown_names_and_non_callable_summaries_fail_closed() {
    assert_eq!(
        lookup("nonexistent_predicate"),
        Err(NamedPredicateError::UnknownPredicate {
            name: "nonexistent_predicate".into()
        })
    );
    assert_eq!(
        NamedPredicateGate::bind(
            "nonexistent_predicate",
            NamedPredicateParams::None,
            PredicateAuthority::Table,
        )
        .unwrap_err(),
        NamedPredicateError::UnknownPredicate {
            name: "nonexistent_predicate".into()
        }
    );
    assert_eq!(
        NamedArmingStep::bind("nonexistent_predicate", NamedPredicateParams::None).unwrap_err(),
        NamedPredicateError::UnknownPredicate {
            name: "nonexistent_predicate".into()
        }
    );

    // Row 9 and Row 18 are Rust terminal_summary hooks, not script-selectable predicates.
    for (summary_name, row) in [("clue_family_summary", 9), ("kq_family_summary", 18)] {
        let desc = lookup(summary_name).unwrap();
        assert_eq!(desc.kind, NamedPredicateKind::TerminalSummary);
        assert_eq!(desc.inventory_row, Some(row));
        for authority in [PredicateAuthority::Script, PredicateAuthority::Table] {
            assert_eq!(
                NamedPredicateGate::bind(summary_name, NamedPredicateParams::None, authority)
                    .unwrap_err(),
                NamedPredicateError::NonCallableSummary {
                    name: summary_name,
                    inventory_row: row,
                }
            );
        }
        assert_eq!(
            NamedArmingStep::bind(summary_name, NamedPredicateParams::None).unwrap_err(),
            NamedPredicateError::NonCallableSummary {
                name: summary_name,
                inventory_row: row,
            }
        );
    }
}

#[test]
fn companions_prepared_passes_fails_and_preserves_three_evidence_labels() {
    assert!(CompanionPrepBarrier::try_new(0).is_err());
    assert!(CompanionPrepBarrier::try_new(8).is_err());

    // Empty evidence label fails closed.
    let err = NamedPredicateGate::bind(
        "companions_prepared",
        NamedPredicateParams::CompanionsPrepared(CompanionsPreparedParams {
            evidence: Cow::Borrowed("   "),
            barrier: CompanionPrepBarrier::try_new(1).unwrap(),
        }),
        PredicateAuthority::Script,
    )
    .unwrap_err();
    assert!(matches!(err, NamedPredicateError::InvalidParams { .. }));

    let snap = GameSnapshot::new();
    for (companions, evidence) in [
        (1usize, EVIDENCE_FLAX_COMPANIONS_PREPARED),
        (1usize, EVIDENCE_CLUE_COMPANIONS_PREPARED),
        (3usize, EVIDENCE_KQ_COMPANIONS_PREPARED),
    ] {
        let barrier = CompanionPrepBarrier::try_new(companions).unwrap();
        let gate = NamedPredicateGate::bind(
            "companions_prepared",
            NamedPredicateParams::CompanionsPrepared(CompanionsPreparedParams {
                evidence: Cow::Borrowed(evidence),
                barrier: barrier.clone(),
            }),
            PredicateAuthority::Script,
        )
        .unwrap();
        assert_eq!(gate.evidence(), evidence);

        let fail = gate.evaluate(PredicateContext::snapshot_only(&snap));
        assert!(!fail.is_pass());
        assert!(fail.failure_reason().unwrap().contains("barrier not ready"));

        for slot in 1..companions {
            barrier.mark_ready(slot);
            assert!(!gate.check(PredicateContext::snapshot_only(&snap)));
        }
        barrier.mark_ready(companions);
        assert_eq!(
            gate.evaluate(PredicateContext::snapshot_only(&snap)),
            PredicateVerdict::Pass
        );
    }
}

#[test]
fn quester_start_exact_pass_and_fail_cases() {
    let stand = WorldTile {
        x: 3209,
        z: 3215,
        level: 0,
    };
    let make_params = |hitpoints_may_be_hurt| QuesterStartExactParams {
        stand,
        stats: vec![
            SkillMinimum {
                skill: 0,
                level: 40,
            },
            SkillMinimum {
                skill: HITPOINTS_SKILL_INDEX,
                level: 40,
            },
        ],
        inventory: vec![(995, 1000)],
        equipment: vec![1333],
        hitpoints_may_be_hurt,
    };

    // Invalid negative inventory count fails closed at bind time.
    assert!(matches!(
        NamedPredicateGate::bind(
            "quester_start_exact",
            NamedPredicateParams::QuesterStartExact(QuesterStartExactParams {
                stand,
                stats: vec![],
                inventory: vec![(995, -1)],
                equipment: vec![],
                hitpoints_may_be_hurt: false,
            }),
            PredicateAuthority::Script,
        ),
        Err(NamedPredicateError::InvalidParams { .. })
    ));

    let gate = NamedPredicateGate::bind(
        "quester_start_exact",
        NamedPredicateParams::QuesterStartExact(make_params(false)),
        PredicateAuthority::Script,
    )
    .unwrap();
    assert_eq!(gate.evidence(), EVIDENCE_QUESTER_START_EXACT);

    // Offline fails closed.
    let offline = GameSnapshot::new();
    assert!(gate
        .evaluate(PredicateContext::snapshot_only(&offline))
        .failure_reason()
        .unwrap()
        .contains("ingame && scene_state == 2"));

    let make_valid = || {
        let mut snap = ingame_snapshot(stand, 1);
        snap.seed_stats(vec![stat_view(0, 40, 40, 0), stat_view(3, 40, 40, 0)]);
        snap.seed_inventory(
            vec![item_view(
                995,
                Some("Coins"),
                1000,
                ItemContainer::Inventory,
            )],
            28,
        );
        snap.seed_equipment(vec![item_view(
            1333,
            Some("Rune scimitar"),
            1,
            ItemContainer::Equipment,
        )]);
        snap
    };
    assert_eq!(
        gate.evaluate(PredicateContext::snapshot_only(&make_valid())),
        PredicateVerdict::Pass
    );

    // Wrong stand fails.
    let mut wrong_stand = make_valid();
    wrong_stand.seed_tile(WorldTile {
        x: 3220,
        z: 3220,
        level: 0,
    });
    assert!(gate
        .evaluate(PredicateContext::snapshot_only(&wrong_stand))
        .failure_reason()
        .unwrap()
        .contains("fixture stand expected"));

    // Missing inventory fails.
    let mut short_inv = make_valid();
    short_inv.seed_inventory(
        vec![item_view(995, Some("Coins"), 500, ItemContainer::Inventory)],
        28,
    );
    assert!(gate
        .evaluate(PredicateContext::snapshot_only(&short_inv))
        .failure_reason()
        .unwrap()
        .contains("fixture item 995 expected 1000"));

    // Missing equipment fails.
    let mut missing_eq = make_valid();
    missing_eq.seed_equipment(vec![]);
    assert!(gate
        .evaluate(PredicateContext::snapshot_only(&missing_eq))
        .failure_reason()
        .unwrap()
        .contains("fixture equipment 1333 was not worn"));

    // Hurt HP fails exact gate, passes when hitpoints_may_be_hurt is true.
    let mut hurt_hp = make_valid();
    hurt_hp.seed_stats(vec![stat_view(0, 40, 40, 0), stat_view(3, 40, 31, 0)]);
    assert!(gate
        .evaluate(PredicateContext::snapshot_only(&hurt_hp))
        .failure_reason()
        .unwrap()
        .contains("fixture stat 3 expected 40"));
    let hurt_gate = NamedPredicateGate::bind(
        "quester_start_exact",
        NamedPredicateParams::QuesterStartExact(make_params(true)),
        PredicateAuthority::Script,
    )
    .unwrap();
    assert_eq!(
        hurt_gate.evaluate(PredicateContext::snapshot_only(&hurt_hp)),
        PredicateVerdict::Pass
    );
}

#[test]
fn clue_duel_named_predicates_pass_and_fail_through_full_lifecycle() {
    let witness = ClueFleetWitnessHandle::new();
    let lobby = WorldTile {
        x: 3366,
        z: 3268,
        level: 0,
    };
    let pen = WorldTile {
        x: 3370,
        z: 3250,
        level: 0,
    };

    let identities = NamedPredicateGate::bind(
        "clue_duel_identities",
        NamedPredicateParams::ClueWitness(witness.clone()),
        PredicateAuthority::Script,
    )
    .unwrap();
    let handshake = NamedPredicateGate::bind(
        "clue_duel_handshake",
        NamedPredicateParams::ClueWitness(witness.clone()),
        PredicateAuthority::Script,
    )
    .unwrap();
    let trail = NamedPredicateGate::bind(
        "clue_duel_trail",
        NamedPredicateParams::ClueWitness(witness.clone()),
        PredicateAuthority::Script,
    )
    .unwrap();
    let reward_banked = NamedPredicateGate::bind(
        "clue_duel_reward_banked",
        NamedPredicateParams::ClueWitness(witness.clone()),
        PredicateAuthority::Script,
    )
    .unwrap();
    let status_settled = NamedPredicateGate::bind(
        "clue_duel_status_settled",
        NamedPredicateParams::ClueDuelStatusSettled(
            ClueDuelStatusSettledParams::with_default_ticks(witness.clone()),
        ),
        PredicateAuthority::Script,
    )
    .unwrap();

    // Zero settle ticks fails closed at bind time.
    assert!(matches!(
        NamedPredicateGate::bind(
            "clue_duel_status_settled",
            NamedPredicateParams::ClueDuelStatusSettled(ClueDuelStatusSettledParams {
                witness: witness.clone(),
                ticks: 0,
            }),
            PredicateAuthority::Script,
        ),
        Err(NamedPredicateError::InvalidParams { .. })
    ));

    let clue_slot = |name: &str, peer: &str, tile: WorldTile, hp: i32| {
        let mut s = ingame_snapshot(tile, 1);
        s.seed_local_player(local_player_view(name, tile));
        s.seed_players(vec![player_view(1, peer, tile)]);
        s.seed_stats(vec![stat_view(3, 99, hp, 0)]);
        s
    };

    // Row 3: clue_duel_identities fails with only one slot / stranger, passes when both see each other.
    let mut s0 = clue_slot("alice", "stranger", lobby, 99);
    s0.seed_inv(vec![(3554, 1)]);
    assert!(!identities.check(PredicateContext::snapshot_only(&s0)));

    let s1 = clue_slot("bob", "alice", lobby, 99);
    witness.observe(1, &s1);
    assert!(!identities.check(PredicateContext::snapshot_only(&s0)));

    s0.seed_players(vec![player_view(1, "bob", lobby)]);
    assert_eq!(
        identities.evaluate(PredicateContext::snapshot_only(&s0)),
        PredicateVerdict::Pass
    );

    // Row 4: clue_duel_handshake fails before offer/confirm/pen, passes once both complete all stages.
    assert!(!handshake.check(PredicateContext::snapshot_only(&s0)));
    for (slot, name, peer) in [(0usize, "alice", "bob"), (1usize, "bob", "alice")] {
        let mut offer = clue_slot(name, peer, lobby, 99);
        offer.seed_varps(vec![VarpView {
            index: 286,
            value: 1024,
        }]);
        offer.seed_main_modal(6575, vec![widget_view(6671, "Opponent")]);
        witness.observe(slot, &offer);

        let mut confirm = clue_slot(name, peer, lobby, 99);
        confirm.seed_varps(vec![VarpView {
            index: 286,
            value: 1024,
        }]);
        confirm.seed_main_modal(6412, Vec::new());
        witness.observe(slot, &confirm);

        let in_pen = clue_slot(name, peer, pen, 99);
        witness.observe(slot, &in_pen);
    }
    assert_eq!(
        handshake.evaluate(PredicateContext::snapshot_only(&s0)),
        PredicateVerdict::Pass
    );

    // Row 5: clue_duel_trail fails before casket/forfeit/win/return/reward, passes after.
    assert!(!trail.check(PredicateContext::snapshot_only(&s0)));
    let mut s0_casket = clue_slot("alice", "bob", pen, 99);
    s0_casket.seed_inv(vec![(3555, 1)]);
    s0_casket.seed_chat_options(
        vec![ChatOptionView {
            component_id: 2461,
            text: "Yes".into(),
        }],
        -1,
    );
    witness.observe(0, &s0_casket);
    let mut s0_reward = clue_slot("alice", "bob", lobby, 99);
    s0_reward.seed_inv(vec![(861, 1)]);
    witness.observe(0, &s0_reward);
    assert!(
        !trail.check(PredicateContext::snapshot_only(&s0_reward)),
        "helper has not won and returned yet"
    );

    let mut s1_win = clue_slot("bob", "alice", lobby, 99);
    s1_win.seed_main_modal(6733, Vec::new());
    witness.observe(1, &s1_win);
    assert_eq!(
        trail.evaluate(PredicateContext::snapshot_only(&s0_reward)),
        PredicateVerdict::Pass
    );

    // Row 6: clue_duel_reward_banked fails before banking, passes once reward is banked with HP intact.
    assert!(!reward_banked.check(PredicateContext::snapshot_only(&s0_reward)));
    let mut s0_bank = clue_slot("alice", "bob", lobby, 99);
    s0_bank.seed_tick(10);
    s0_bank.seed_inv(vec![(861, 1)]);
    s0_bank.seed_bank_observation(
        5382,
        2,
        Some(vec![item_view(
            861,
            Some("Magic shortbow"),
            1,
            ItemContainer::Bank,
        )]),
        Vec::new(),
    );
    assert_eq!(
        reward_banked.evaluate(PredicateContext::snapshot_only(&s0_bank)),
        PredicateVerdict::Pass
    );

    // Verify HP drift fails clue_duel_reward_banked cleanly.
    let s1_hurt = clue_slot("bob", "alice", lobby, 90);
    witness.observe(1, &s1_hurt);
    assert!(!reward_banked.check(PredicateContext::snapshot_only(&s0_bank)));

    // Row 8: clue_duel_status_settled fails until `from + 3` observed ticks.
    let mut t20 = ingame_snapshot(lobby, 20);
    assert!(!status_settled.check(PredicateContext::snapshot_only(&t20)));
    t20.seed_tick(22);
    assert!(!status_settled.check(PredicateContext::snapshot_only(&t20)));
    t20.seed_tick(23);
    assert_eq!(
        status_settled.evaluate(PredicateContext::snapshot_only(&t20)),
        PredicateVerdict::Pass
    );
}

#[test]
fn bank_closed_alias_pass_and_fail_cases() {
    let gate = NamedPredicateGate::bind(
        "bank_closed",
        NamedPredicateParams::None,
        PredicateAuthority::Script,
    )
    .unwrap();
    assert_eq!(gate.descriptor().kind, NamedPredicateKind::ProofAlias);
    assert_eq!(gate.evidence(), EVIDENCE_BANK_CLOSED);

    // Mismatched parameter fails closed.
    assert!(matches!(
        NamedPredicateGate::bind(
            "bank_closed",
            NamedPredicateParams::StandingBankClosed(StandingBankClosedParams {
                stand: WorldTile {
                    x: 3200,
                    z: 3200,
                    level: 0,
                }
            }),
            PredicateAuthority::Script,
        ),
        Err(NamedPredicateError::InvalidParams { .. })
    ));

    let offline = GameSnapshot::new();
    assert!(!gate.check(PredicateContext::snapshot_only(&offline)));

    let mut open = ingame_snapshot(
        WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        },
        1,
    );
    open.seed_bank_observation(5382, 1, Some(Vec::new()), Vec::new());
    let fail = gate.evaluate(PredicateContext::snapshot_only(&open));
    assert!(!fail.is_pass());
    assert!(fail.failure_reason().unwrap().contains("still open"));

    let closed = ingame_snapshot(
        WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        },
        2,
    );
    assert_eq!(
        gate.evaluate(PredicateContext::snapshot_only(&closed)),
        PredicateVerdict::Pass
    );
}

#[test]
fn kq_arming_step_is_distinct_from_gates_and_kq_gates_pass_and_fail() {
    let witness = KqFleetWitnessHandle::new();

    // Row 11: kq_armed cannot be bound as a gate.
    assert_eq!(
        NamedPredicateGate::bind(
            "kq_armed",
            NamedPredicateParams::KqWitness(witness.clone()),
            PredicateAuthority::Table,
        )
        .unwrap_err(),
        NamedPredicateError::ArmingStepNotAGate { name: "kq_armed" }
    );
    // A gate cannot be bound as an arming step.
    assert_eq!(
        NamedArmingStep::bind("bank_closed", NamedPredicateParams::None).unwrap_err(),
        NamedPredicateError::NotAnArmingStep {
            name: "bank_closed",
            kind: NamedPredicateKind::ProofAlias,
        }
    );

    assert!(!witness.is_started());
    execute_arming_by_name("kq_armed", NamedPredicateParams::KqWitness(witness.clone())).unwrap();
    assert!(witness.is_started());

    let identities = NamedPredicateGate::bind(
        "kq_identities",
        NamedPredicateParams::KqWitness(witness.clone()),
        PredicateAuthority::Script,
    )
    .unwrap();
    let first_descent = NamedPredicateGate::bind(
        "kq_first_descent",
        NamedPredicateParams::KqWitness(witness.clone()),
        PredicateAuthority::Script,
    )
    .unwrap();
    let first_combat = NamedPredicateGate::bind(
        "kq_first_combat",
        NamedPredicateParams::KqWitness(witness.clone()),
        PredicateAuthority::Script,
    )
    .unwrap();
    let first_kill_banked = NamedPredicateGate::bind(
        "kq_first_kill_banked",
        NamedPredicateParams::KqWitness(witness.clone()),
        PredicateAuthority::Script,
    )
    .unwrap();
    let second_descent = NamedPredicateGate::bind(
        "kq_second_descent",
        NamedPredicateParams::KqWitness(witness.clone()),
        PredicateAuthority::Script,
    )
    .unwrap();
    let safe_complete = NamedPredicateGate::bind(
        "kq_safe_complete",
        NamedPredicateParams::KqWitness(witness.clone()),
        PredicateAuthority::Script,
    )
    .unwrap();
    let finish_ready = NamedPredicateGate::bind(
        "kq_finish_ready",
        NamedPredicateParams::KqWitness(witness.clone()),
        PredicateAuthority::Script,
    )
    .unwrap();

    let bank_tile = WorldTile {
        x: 3308,
        z: 3120,
        level: 0,
    };
    let surface_tile = WorldTile {
        x: 3226,
        z: 3108,
        level: 0,
    };
    let upper_tile = WorldTile {
        x: 3508,
        z: 9497,
        level: 2,
    };
    let lair_tile = WorldTile {
        x: 3508,
        z: 9493,
        level: 0,
    };
    let camelot_tile = WorldTile {
        x: 2757,
        z: 3478,
        level: 0,
    };
    let duel_tile = WorldTile {
        x: 3315,
        z: 3235,
        level: 0,
    };

    let names = ["alice", "bob", "carol", "dave"];
    let kit_items = || {
        let mut eq = vec![item_view(
            892,
            Some("Rune arrow"),
            250,
            ItemContainer::Equipment,
        )];
        for id in [1434, 1163, 2503, 2497, 2491, 1731, 1061, 2550] {
            eq.push(item_view(id, None, 1, ItemContainer::Equipment));
        }
        eq
    };
    let pack_items = |slot: usize, sharks: i32, ropes: i32| {
        let mut inv = vec![item_view(
            1854,
            Some("Shantay pass"),
            1,
            ItemContainer::Inventory,
        )];
        if sharks > 0 {
            inv.push(item_view(
                385,
                Some("Shark"),
                sharks,
                ItemContainer::Inventory,
            ));
        }
        if ropes > 0 {
            inv.push(item_view(
                954,
                Some("Rope"),
                ropes,
                ItemContainer::Inventory,
            ));
        }
        let _ = slot;
        inv
    };
    let base_stats = |str_xp: i32, rng_xp: i32, hp: i32, boosted: bool| {
        let eff = if boosted { 118 } else { 99 };
        vec![
            stat_view(0, 99, eff, 14_000_000),
            stat_view(1, 99, eff, 14_000_000),
            stat_view(2, 99, eff, str_xp),
            stat_view(3, 99, hp, 14_000_000),
            stat_view(4, 99, eff, rng_xp),
        ]
    };
    let three_peers = |tile: WorldTile| {
        vec![
            player_view(1, "p1", tile),
            player_view(2, "p2", tile),
            player_view(3, "p3", tile),
        ]
    };

    // Row 10: kq_identities fails with 3 slots, passes once all 4 distinct slots are observed.
    for (slot, &name) in names.iter().enumerate().skip(1) {
        let mut s = if slot <= 1 {
            let mut snap = ingame_snapshot(bank_tile, 1);
            snap.seed_bank_observation(
                5382,
                1,
                Some(vec![item_view(
                    1854,
                    Some("Shantay pass"),
                    2,
                    ItemContainer::Bank,
                )]),
                Vec::new(),
            );
            snap
        } else {
            shop_open_snapshot(bank_tile, 1)
        };
        s.seed_local_player(local_player_view(name, bank_tile));
        s.seed_stats(base_stats(14_000_000, 14_000_000, 99, false));
        s.seed_equipment(kit_items());
        s.seed_inventory(pack_items(slot, 16, 0), 28);
        witness.observe(slot, &s);
    }
    let s0_unnamed = ingame_snapshot(bank_tile, 1);
    assert!(
        !identities.check(PredicateContext::snapshot_only(&s0_unnamed)),
        "kq_identities must fail while only 3 of 4 distinct accounts are observed"
    );
    let mut s0_bank = ingame_snapshot(bank_tile, 1);
    s0_bank.seed_local_player(local_player_view("alice", bank_tile));
    s0_bank.seed_stats(base_stats(14_000_000, 14_000_000, 99, false));
    s0_bank.seed_equipment(kit_items());
    s0_bank.seed_inventory(pack_items(0, 14, 2), 28);
    s0_bank.seed_bank_observation(
        5382,
        1,
        Some(vec![item_view(
            1854,
            Some("Shantay pass"),
            2,
            ItemContainer::Bank,
        )]),
        Vec::new(),
    );
    assert!(!first_descent.check(PredicateContext::snapshot_only(&s0_bank)));
    assert_eq!(
        identities.evaluate(PredicateContext::snapshot_only(&s0_bank)),
        PredicateVerdict::Pass
    );

    // Row 12: kq_first_descent — all 4 visit surface + upper with party/ropes and enter lair.
    for slot in 0..4 {
        let ropes_surface = if slot == 0 { 1 } else { 0 };
        let sharks = if slot == 0 { 14 } else { 16 };
        let mut surf = ingame_snapshot(surface_tile, 5);
        surf.seed_stats(base_stats(14_000_000, 14_000_000, 99, false));
        surf.seed_inventory(pack_items(slot, sharks, ropes_surface), 28);
        surf.seed_players(three_peers(surface_tile));
        surf.seed_locs(vec![loc_view(3828, "Tunnel", surface_tile, "Climb-down")]);
        witness.observe(slot, &surf);
        witness.observe(slot, &surf);

        let mut upper = ingame_snapshot(upper_tile, 6);
        upper.seed_stats(base_stats(14_000_000, 14_000_000, 99, false));
        upper.seed_inventory(pack_items(slot, sharks, 0), 28);
        upper.seed_players(three_peers(upper_tile));
        upper.seed_locs(vec![loc_view(3831, "Tunnel", upper_tile, "Climb-down")]);
        witness.observe(slot, &upper);
        witness.observe(slot, &upper);

        let mut enter = ingame_snapshot(lair_tile, 7);
        enter.seed_stats(base_stats(14_000_000, 14_000_000, 99, false));
        enter.seed_inventory(pack_items(slot, sharks, 0), 28);
        witness.observe(slot, &enter);
    }
    let s0_lair = ingame_snapshot(lair_tile, 8);
    assert_eq!(
        first_descent.evaluate(PredicateContext::snapshot_only(&s0_lair)),
        PredicateVerdict::Pass
    );

    // Row 13: kq_first_combat fails before combat/formations/food, passes once met.
    assert!(!first_combat.check(PredicateContext::snapshot_only(&s0_lair)));
    let queen_tile = WorldTile {
        x: 3508,
        z: 9493,
        level: 0,
    };
    // Queen size 3 => centre (3509, 9494), melee along == 2, ranged along in 3..=9.
    let melee_tiles = [
        WorldTile {
            x: 3507,
            z: 9494,
            level: 0,
        },
        WorldTile {
            x: 3511,
            z: 9494,
            level: 0,
        },
        WorldTile {
            x: 3509,
            z: 9496,
            level: 0,
        },
        WorldTile {
            x: 3509,
            z: 9492,
            level: 0,
        },
    ];
    let ranged_tiles = [
        WorldTile {
            x: 3505,
            z: 9494,
            level: 0,
        },
        WorldTile {
            x: 3513,
            z: 9494,
            level: 0,
        },
        WorldTile {
            x: 3509,
            z: 9498,
            level: 0,
        },
        WorldTile {
            x: 3509,
            z: 9490,
            level: 0,
        },
    ];
    for slot in 0..4 {
        let mut melee = ingame_snapshot(melee_tiles[slot], 10);
        melee.seed_varps(vec![VarpView {
            index: 95,
            value: 1,
        }]);
        melee.seed_stats(base_stats(14_000_100, 14_000_000, 99, true));
        melee.seed_inventory(pack_items(slot, 12, 0), 28);
        melee.seed_npcs(vec![npc_view(1158, queen_tile, 3, 255, 255)]);
        witness.observe(slot, &melee);

        let mut ranged = ingame_snapshot(ranged_tiles[slot], 11);
        ranged.seed_varps(vec![VarpView {
            index: 95,
            value: 1,
        }]);
        ranged.seed_stats(base_stats(14_000_200, 14_000_100, 99, true));
        ranged.seed_inventory(pack_items(slot, 12, 0), 28);
        ranged.seed_npcs(vec![npc_view(1160, queen_tile, 3, 255, 255)]);
        witness.observe(slot, &ranged);
    }
    assert_eq!(
        first_combat.evaluate(PredicateContext::snapshot_only(&s0_lair)),
        PredicateVerdict::Pass
    );

    // Row 14: kq_first_kill_banked fails before kill + bank return, passes after.
    assert!(!first_kill_banked.check(PredicateContext::snapshot_only(&s0_lair)));
    for slot in 0..4 {
        let mut zero = ingame_snapshot(lair_tile, 12);
        zero.seed_npcs(vec![npc_view(1160, queen_tile, 3, 0, 255)]);
        witness.observe(slot, &zero);
        let empty = ingame_snapshot(lair_tile, 13);
        witness.observe(slot, &empty);
        let mut bank_ret = ingame_snapshot(bank_tile, 14);
        bank_ret.seed_bank_observation(5382, 2, Some(Vec::new()), Vec::new());
        witness.observe(slot, &bank_ret);
    }
    let mut s0_bank_ret = ingame_snapshot(bank_tile, 15);
    s0_bank_ret.seed_bank_observation(5382, 2, Some(Vec::new()), Vec::new());
    assert_eq!(
        first_kill_banked.evaluate(PredicateContext::snapshot_only(&s0_bank_ret)),
        PredicateVerdict::Pass
    );

    // Row 15: kq_second_descent fails before 2nd entry + respawn attack, passes after.
    assert!(!second_descent.check(PredicateContext::snapshot_only(&s0_bank_ret)));
    for slot in 0..4 {
        let mut enter2 = ingame_snapshot(lair_tile, 20);
        enter2.seed_stats(base_stats(14_000_200, 14_000_100, 99, true));
        enter2.seed_npcs(vec![npc_view(1160, queen_tile, 3, 255, 255)]);
        witness.observe(slot, &enter2);

        let mut hit2 = ingame_snapshot(lair_tile, 21);
        hit2.seed_stats(base_stats(14_000_300, 14_000_100, 99, true));
        hit2.seed_npcs(vec![npc_view(1160, queen_tile, 3, 0, 255)]);
        witness.observe(slot, &hit2);
    }
    let s0_hit2 = ingame_snapshot(lair_tile, 22);
    assert_eq!(
        second_descent.evaluate(PredicateContext::snapshot_only(&s0_hit2)),
        PredicateVerdict::Pass
    );

    // Row 16: kq_safe_complete fails before escapes + 2nd kill + safe bank close, passes after.
    assert!(!safe_complete.check(PredicateContext::snapshot_only(&s0_hit2)));
    for slot in 0..4 {
        // Disappearance -> 2nd kill.
        witness.observe(slot, &ingame_snapshot(lair_tile, 23));
        // Camelot + Duel escape with spent ring (2554).
        witness.observe(slot, &ingame_snapshot(camelot_tile, 24));
        let mut duel_esc = ingame_snapshot(duel_tile, 25);
        duel_esc.seed_inv(vec![(2554, 1)]);
        witness.observe(slot, &duel_esc);
        // Second fresh bank open then closed at bank.
        let mut bank2 = ingame_snapshot(bank_tile, 26);
        bank2.seed_bank_observation(5382, 3, Some(Vec::new()), Vec::new());
        witness.observe(slot, &bank2);
        witness.observe(slot, &ingame_snapshot(bank_tile, 27));
    }
    let s0_safe = ingame_snapshot(bank_tile, 28);
    assert_eq!(
        safe_complete.evaluate(PredicateContext::snapshot_only(&s0_safe)),
        PredicateVerdict::Pass
    );

    // Row 17: kq_finish_ready fails before begin_finish, fails when companion bank opens, passes when all closed.
    assert!(!finish_ready.check(PredicateContext::snapshot_only(&s0_safe)));
    witness.begin_finish();
    for slot in 1..4 {
        witness.observe(slot, &ingame_snapshot(bank_tile, 29));
    }
    assert_eq!(
        finish_ready.evaluate(PredicateContext::snapshot_only(&ingame_snapshot(
            bank_tile, 29
        ))),
        PredicateVerdict::Pass
    );
    // Reopening a companion bank revokes finish_ready.
    let mut comp_open = ingame_snapshot(bank_tile, 30);
    comp_open.seed_bank_observation(5382, 4, Some(Vec::new()), Vec::new());
    witness.observe(2, &comp_open);
    assert!(
        !finish_ready.check(PredicateContext::snapshot_only(&ingame_snapshot(
            bank_tile, 30
        )))
    );
}

#[test]
fn gnome_radius_respected_pass_and_fail_cases() {
    let witness_tile = crate::scenarios::production::GNOME_RADIUS_WITNESS_TILE;
    let ticks_needed = crate::scenarios::production::GNOME_RADIUS_WITNESS_GAME_TICKS;
    let log_beyond = loc_view(
        2295,
        "Log balance",
        WorldTile {
            x: 2474,
            z: 3435,
            level: 0,
        },
        "Walk-across",
    );

    let make_tick = |tick: u32, tile: WorldTile, xp: i32, locs: Vec<LocView>| {
        let mut snap = ingame_snapshot(tile, tick);
        snap.seed_stats(vec![stat_view(16, 1, 1, xp)]);
        snap.seed_locs(locs);
        snap
    };

    // Pass path: 12 consecutive observed game ticks on witness tile with log beyond radius 10.
    let gate = NamedPredicateGate::bind(
        "gnome_radius_respected",
        NamedPredicateParams::GnomeRadius(GnomeRadiusWitnessHandle::new()),
        PredicateAuthority::Script,
    )
    .unwrap();
    assert_eq!(gate.evidence(), EVIDENCE_GNOME_RADIUS_RESPECTED);
    for tick in 1..ticks_needed {
        let snap = make_tick(tick, witness_tile, 0, vec![log_beyond.clone()]);
        assert!(!gate.check(PredicateContext::snapshot_only(&snap)));
    }
    let final_snap = make_tick(ticks_needed, witness_tile, 0, vec![log_beyond.clone()]);
    assert_eq!(
        gate.evaluate(PredicateContext::snapshot_only(&final_snap)),
        PredicateVerdict::Pass
    );

    // Fail path 1: bot moves off witness tile after watch begins -> permanently refused even if returning.
    let moved_gate = NamedPredicateGate::bind(
        "gnome_radius_respected",
        NamedPredicateParams::GnomeRadius(GnomeRadiusWitnessHandle::new()),
        PredicateAuthority::Script,
    )
    .unwrap();
    let _ = moved_gate.evaluate(PredicateContext::snapshot_only(&make_tick(
        1,
        witness_tile,
        0,
        vec![log_beyond.clone()],
    )));
    let off_tile = WorldTile {
        x: witness_tile.x,
        z: witness_tile.z - 1,
        level: 0,
    };
    let moved_fail = moved_gate.evaluate(PredicateContext::snapshot_only(&make_tick(
        2,
        off_tile,
        0,
        vec![log_beyond.clone()],
    )));
    assert!(moved_fail
        .failure_reason()
        .unwrap()
        .contains("refused after moving"));
    for tick in 3..=(ticks_needed + 5) {
        assert!(
            !moved_gate.check(PredicateContext::snapshot_only(&make_tick(
                tick,
                witness_tile,
                0,
                vec![log_beyond.clone()],
            )))
        );
    }

    // Fail path 2: Agility XP increases -> permanently refused.
    let xp_gate = NamedPredicateGate::bind(
        "gnome_radius_respected",
        NamedPredicateParams::GnomeRadius(GnomeRadiusWitnessHandle::new()),
        PredicateAuthority::Script,
    )
    .unwrap();
    let _ = xp_gate.evaluate(PredicateContext::snapshot_only(&make_tick(
        1,
        witness_tile,
        100,
        vec![log_beyond.clone()],
    )));
    let xp_fail = xp_gate.evaluate(PredicateContext::snapshot_only(&make_tick(
        2,
        witness_tile,
        108,
        vec![log_beyond],
    )));
    assert!(xp_fail
        .failure_reason()
        .unwrap()
        .contains("agility XP rose"));
}

#[test]
fn quest_seed_exact_enforces_listed_stats_hitpoints_inventory_equipment_colours_and_optional_qp() {
    let stand = WorldTile {
        x: 3481,
        z: 3279,
        level: 0,
    };
    let base_params = |qp_varp: Option<i32>| QuestSeedExactParams {
        stand,
        stats: vec![
            QuestSeedStat {
                index: 0,
                level: 50,
                listed: true,
            },
            QuestSeedStat {
                index: 3,
                level: 50,
                listed: true,
            },
            QuestSeedStat {
                index: 15,
                level: 1,
                listed: false,
            },
        ],
        hitpoints_index: Some(3),
        inventory: BTreeMap::from([(379, 16), (995, 500)]),
        equipment: vec![1333],
        completed_quests: vec!["Priest in Peril".into()],
        current_quest: "Shades of Mortton".into(),
        quest_points: 1,
        quest_points_varp: qp_varp,
    };

    // Invalid empty current_quest fails closed.
    let mut bad_params = base_params(Some(101));
    bad_params.current_quest = " ".into();
    assert!(matches!(
        NamedPredicateGate::bind(
            "quest_seed_exact",
            NamedPredicateParams::QuestSeedExact(bad_params),
            PredicateAuthority::Script,
        ),
        Err(NamedPredicateError::InvalidParams { .. })
    ));

    let gate = NamedPredicateGate::bind(
        "quest_seed_exact",
        NamedPredicateParams::QuestSeedExact(base_params(Some(101))),
        PredicateAuthority::Script,
    )
    .unwrap();
    assert_eq!(gate.evidence(), EVIDENCE_QUEST_SEED_EXACT);

    let make_valid = || {
        let mut snap = ingame_snapshot(stand, 1);
        // Unlisted stat 15 intentionally omitted to verify missing unlisted stat is accepted.
        // Hitpoints hurt to 42 (> 0 && <= 50) is accepted.
        snap.seed_stats(vec![stat_view(0, 50, 50, 0), stat_view(3, 50, 42, 0)]);
        snap.seed_inventory(
            vec![
                item_view(379, Some("Lobster"), 10, ItemContainer::Inventory),
                item_view(379, Some("Lobster"), 6, ItemContainer::Inventory),
                item_view(995, Some("Coins"), 500, ItemContainer::Inventory),
            ],
            28,
        );
        snap.seed_equipment(vec![item_view(
            1333,
            Some("Rune scimitar"),
            1,
            ItemContainer::Equipment,
        )]);
        snap.seed_quest_statuses(
            vec![
                QuestStatusView {
                    component_id: 1,
                    name: "priest in peril".into(),
                    colour: QUEST_COLOUR_COMPLETE,
                },
                QuestStatusView {
                    component_id: 2,
                    name: "Shades of Mortton".into(),
                    colour: QUEST_COLOUR_NOT_STARTED,
                },
            ],
            true,
        );
        snap.seed_varps(vec![VarpView {
            index: 101,
            value: 1,
        }]);
        snap
    };

    assert_eq!(
        gate.evaluate(PredicateContext::snapshot_only(&make_valid())),
        PredicateVerdict::Pass
    );

    // Missing listed stat fails.
    let mut missing_listed = make_valid();
    missing_listed.seed_stats(vec![stat_view(3, 50, 50, 0)]);
    assert!(gate
        .evaluate(PredicateContext::snapshot_only(&missing_listed))
        .failure_reason()
        .unwrap()
        .contains("listed stat 0 is unobserved"));

    // Hitpoints == 0 fails.
    let mut zero_hp = make_valid();
    zero_hp.seed_stats(vec![stat_view(0, 50, 50, 0), stat_view(3, 50, 0, 0)]);
    assert!(!gate.check(PredicateContext::snapshot_only(&zero_hp)));

    // Hitpoints > target fails.
    let mut over_hp = make_valid();
    over_hp.seed_stats(vec![stat_view(0, 50, 50, 0), stat_view(3, 50, 51, 0)]);
    assert!(!gate.check(PredicateContext::snapshot_only(&over_hp)));

    // Non-HP effective mismatch fails.
    let mut hurt_atk = make_valid();
    hurt_atk.seed_stats(vec![stat_view(0, 50, 49, 0), stat_view(3, 50, 50, 0)]);
    assert!(!gate.check(PredicateContext::snapshot_only(&hurt_atk)));

    // Extra inventory item fails (exact totals required).
    let mut extra_inv = make_valid();
    extra_inv.seed_inventory(
        vec![
            item_view(379, Some("Lobster"), 16, ItemContainer::Inventory),
            item_view(995, Some("Coins"), 500, ItemContainer::Inventory),
            item_view(554, Some("Fire rune"), 1, ItemContainer::Inventory),
        ],
        28,
    );
    assert!(gate
        .evaluate(PredicateContext::snapshot_only(&extra_inv))
        .failure_reason()
        .unwrap()
        .contains("inventory expected"));

    // Extra worn equipment fails (exact worn set required).
    let mut extra_eq = make_valid();
    extra_eq.seed_equipment(vec![
        item_view(1333, Some("Rune scimitar"), 1, ItemContainer::Equipment),
        item_view(1163, Some("Rune full helm"), 1, ItemContainer::Equipment),
    ]);
    assert!(gate
        .evaluate(PredicateContext::snapshot_only(&extra_eq))
        .failure_reason()
        .unwrap()
        .contains("worn equipment expected"));

    // Wrong completed quest colour fails.
    let mut bad_completed = make_valid();
    bad_completed.seed_quest_statuses(
        vec![
            QuestStatusView {
                component_id: 1,
                name: "Priest in Peril".into(),
                colour: QUEST_COLOUR_NOT_STARTED,
            },
            QuestStatusView {
                component_id: 2,
                name: "Shades of Mortton".into(),
                colour: QUEST_COLOUR_NOT_STARTED,
            },
        ],
        true,
    );
    assert!(gate
        .evaluate(PredicateContext::snapshot_only(&bad_completed))
        .failure_reason()
        .unwrap()
        .contains("completed quest"));

    // Wrong current quest colour fails.
    let mut bad_current = make_valid();
    bad_current.seed_quest_statuses(
        vec![
            QuestStatusView {
                component_id: 1,
                name: "Priest in Peril".into(),
                colour: QUEST_COLOUR_COMPLETE,
            },
            QuestStatusView {
                component_id: 2,
                name: "Shades of Mortton".into(),
                colour: QUEST_COLOUR_COMPLETE,
            },
        ],
        true,
    );
    assert!(gate
        .evaluate(PredicateContext::snapshot_only(&bad_current))
        .failure_reason()
        .unwrap()
        .contains("current quest"));

    // Wrong QP fails when varp is Some(101), but passes when quest_points_varp is None.
    let mut wrong_qp = make_valid();
    wrong_qp.seed_varps(vec![VarpView {
        index: 101,
        value: 2,
    }]);
    assert!(gate
        .evaluate(PredicateContext::snapshot_only(&wrong_qp))
        .failure_reason()
        .unwrap()
        .contains("quest points varp 101"));

    let no_qp_gate = NamedPredicateGate::bind(
        "quest_seed_exact",
        NamedPredicateParams::QuestSeedExact(base_params(None)),
        PredicateAuthority::Script,
    )
    .unwrap();
    assert_eq!(
        no_qp_gate.evaluate(PredicateContext::snapshot_only(&wrong_qp)),
        PredicateVerdict::Pass
    );
}

#[test]
fn standing_bank_closed_pass_and_fail_cases() {
    let stand = WorldTile {
        x: 3243,
        z: 3206,
        level: 0,
    };
    let gate = NamedPredicateGate::bind(
        "standing_bank_closed",
        NamedPredicateParams::StandingBankClosed(StandingBankClosedParams { stand }),
        PredicateAuthority::Script,
    )
    .unwrap();
    assert_eq!(gate.evidence(), EVIDENCE_STANDING_BANK_CLOSED);

    let ok = ingame_snapshot(stand, 1);
    assert_eq!(
        gate.evaluate(PredicateContext::snapshot_only(&ok)),
        PredicateVerdict::Pass
    );

    let mut wrong_scene = ingame_snapshot(stand, 1);
    wrong_scene.seed_ingame(1);
    assert!(gate
        .evaluate(PredicateContext::snapshot_only(&wrong_scene))
        .failure_reason()
        .unwrap()
        .contains("scene_state == 2"));

    let wrong_tile = ingame_snapshot(
        WorldTile {
            x: 3244,
            z: 3206,
            level: 0,
        },
        1,
    );
    assert!(gate
        .evaluate(PredicateContext::snapshot_only(&wrong_tile))
        .failure_reason()
        .unwrap()
        .contains("stand expected"));

    let mut bank_open = ingame_snapshot(stand, 1);
    bank_open.seed_bank_observation(5382, 1, Some(Vec::new()), Vec::new());
    assert!(gate
        .evaluate(PredicateContext::snapshot_only(&bank_open))
        .failure_reason()
        .unwrap()
        .contains("bank component 5382 is open"));
}

#[test]
fn bank_exact_pass_and_fail_cases() {
    let expected = BTreeMap::from([(2347, 1), (960, 50)]);
    let gate = NamedPredicateGate::bind(
        "bank_exact",
        NamedPredicateParams::BankExact(BankExactParams { expected }),
        PredicateAuthority::Script,
    )
    .unwrap();
    assert_eq!(gate.evidence(), EVIDENCE_BANK_EXACT);

    // Zero or negative count fails closed at bind time.
    assert!(matches!(
        NamedPredicateGate::bind(
            "bank_exact",
            NamedPredicateParams::BankExact(BankExactParams {
                expected: BTreeMap::from([(2347, 0)]),
            }),
            PredicateAuthority::Script,
        ),
        Err(NamedPredicateError::InvalidParams { .. })
    ));

    let tile = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    let unloaded = ingame_snapshot(tile, 1);
    assert!(gate
        .evaluate(PredicateContext::snapshot_only(&unloaded))
        .failure_reason()
        .unwrap()
        .contains("not loaded"));

    let mut exact = ingame_snapshot(tile, 1);
    exact.seed_bank_observation(
        5382,
        1,
        Some(vec![
            item_view(2347, Some("Hammer"), 1, ItemContainer::Bank),
            item_view(960, Some("Plank"), 50, ItemContainer::Bank),
        ]),
        Vec::new(),
    );
    assert_eq!(
        gate.evaluate(PredicateContext::snapshot_only(&exact)),
        PredicateVerdict::Pass
    );

    // Missing item fails.
    let mut missing = ingame_snapshot(tile, 1);
    missing.seed_bank_observation(
        5382,
        1,
        Some(vec![item_view(
            2347,
            Some("Hammer"),
            1,
            ItemContainer::Bank,
        )]),
        Vec::new(),
    );
    assert!(!gate.check(PredicateContext::snapshot_only(&missing)));

    // Extra item fails.
    let mut extra = ingame_snapshot(tile, 1);
    extra.seed_bank_observation(
        5382,
        1,
        Some(vec![
            item_view(2347, Some("Hammer"), 1, ItemContainer::Bank),
            item_view(960, Some("Plank"), 50, ItemContainer::Bank),
            item_view(995, Some("Coins"), 1, ItemContainer::Bank),
        ]),
        Vec::new(),
    );
    assert!(!gate.check(PredicateContext::snapshot_only(&extra)));
}

#[test]
fn world_speed_changed_requires_table_authority_and_fresh_exact_confirmation() {
    // Script authority is refused fail-closed.
    assert_eq!(
        NamedPredicateGate::bind(
            "world_speed_changed",
            NamedPredicateParams::WorldSpeedChanged(WorldSpeedChangedParams {
                expected_ms: 300,
                baseline: WorldSpeedBaseline::fixed(10),
            }),
            PredicateAuthority::Script,
        )
        .unwrap_err(),
        NamedPredicateError::TableOnlyAuthorityRequired {
            name: "world_speed_changed",
        }
    );

    // Out-of-range speed fails closed.
    for bad_ms in [49, 5001] {
        assert!(matches!(
            NamedPredicateGate::bind(
                "world_speed_changed",
                NamedPredicateParams::WorldSpeedChanged(WorldSpeedChangedParams {
                    expected_ms: bad_ms,
                    baseline: WorldSpeedBaseline::fixed(0),
                }),
                PredicateAuthority::Table,
            ),
            Err(NamedPredicateError::InvalidParams { .. })
        ));
    }

    let baseline = WorldSpeedBaseline::unset();
    let gate = NamedPredicateGate::bind(
        "world_speed_changed",
        NamedPredicateParams::WorldSpeedChanged(WorldSpeedChangedParams {
            expected_ms: 300,
            baseline: baseline.clone(),
        }),
        PredicateAuthority::Table,
    )
    .unwrap();
    assert_eq!(gate.evidence(), EVIDENCE_WORLD_SPEED_CHANGED);

    let mut snap = GameSnapshot::new();
    snap.seed_chat_lines(vec![ChatLineView {
        type_: 0,
        username: None,
        text: "World speed was changed to 300ms".into(),
        sequence: 12,
    }]);

    // Absent baseline fails closed even if matching line is present.
    let absent = gate.evaluate(PredicateContext::snapshot_only(&snap));
    assert!(absent
        .failure_reason()
        .unwrap()
        .contains("baseline is absent"));

    // Capturing baseline at sequence 12 makes sequence 12 stale (must fail).
    assert_eq!(baseline.capture(&snap), 12);
    assert!(!gate.check(PredicateContext::snapshot_only(&snap)));

    // Wrong speed newer than baseline fails.
    snap.seed_chat_lines(vec![ChatLineView {
        type_: 0,
        username: None,
        text: "World speed was changed to 600ms".into(),
        sequence: 13,
    }]);
    let wrong_speed = gate.evaluate(PredicateContext::snapshot_only(&snap));
    assert!(wrong_speed
        .failure_reason()
        .unwrap()
        .contains("observed 600ms"));

    // Player chat impersonating system message fails.
    snap.seed_chat_lines(vec![ChatLineView {
        type_: 0,
        username: Some("alice".into()),
        text: "World speed was changed to 300ms".into(),
        sequence: 14,
    }]);
    assert!(!gate.check(PredicateContext::snapshot_only(&snap)));

    // Fresh exact system message passes.
    snap.seed_chat_lines(vec![ChatLineView {
        type_: 0,
        username: None,
        text: "World speed was changed to 300ms".into(),
        sequence: 15,
    }]);
    assert_eq!(
        gate.evaluate(PredicateContext::snapshot_only(&snap)),
        PredicateVerdict::Pass
    );
}

#[test]
fn revision_landing_idle_pass_and_fail_cases() {
    // Empty accepted_tiles fails closed.
    assert!(matches!(
        NamedPredicateGate::bind(
            "revision_landing_idle",
            NamedPredicateParams::RevisionLandingIdle(RevisionLandingIdleParams {
                expected_revision: ClientRevision::R377,
                accepted_tiles: Vec::new(),
            }),
            PredicateAuthority::Script,
        ),
        Err(NamedPredicateError::InvalidParams { .. })
    ));

    let params = RevisionLandingIdleParams {
        expected_revision: ClientRevision::R377,
        accepted_tiles: vec![
            WorldTile {
                x: 3220,
                z: 3220,
                level: 0,
            },
            WorldTile {
                x: 3220,
                z: 3222,
                level: 0,
            },
        ],
    };
    let gate = NamedPredicateGate::bind(
        "revision_landing_idle",
        NamedPredicateParams::RevisionLandingIdle(params),
        PredicateAuthority::Script,
    )
    .unwrap();
    assert_eq!(gate.evidence(), EVIDENCE_REVISION_LANDING_IDLE);

    let snap = ingame_snapshot(
        WorldTile {
            x: 3220,
            z: 3222,
            level: 0,
        },
        1,
    );

    // Missing revision fails closed.
    assert!(gate
        .evaluate(PredicateContext::snapshot_only(&snap))
        .failure_reason()
        .unwrap()
        .contains("revision is unavailable"));

    // Wrong revision fails.
    for wrong_rev in [ClientRevision::R274, ClientRevision::R289] {
        assert!(gate
            .evaluate(PredicateContext::with_revision(&snap, wrong_rev))
            .failure_reason()
            .unwrap()
            .contains("expected revision R377"));
    }

    // Matching revision + accepted tile + no active continue passes.
    assert_eq!(
        gate.evaluate(PredicateContext::with_revision(&snap, ClientRevision::R377)),
        PredicateVerdict::Pass
    );

    // Unaccepted tile fails.
    let bad_tile = ingame_snapshot(
        WorldTile {
            x: 3220,
            z: 3216,
            level: 0,
        },
        1,
    );
    assert!(gate
        .evaluate(PredicateContext::with_revision(
            &bad_tile,
            ClientRevision::R377
        ))
        .failure_reason()
        .unwrap()
        .contains("not in accepted landing tiles"));

    // Active continue fails.
    let mut with_continue = ingame_snapshot(
        WorldTile {
            x: 3220,
            z: 3222,
            level: 0,
        },
        1,
    );
    with_continue.seed_chat_options(Vec::new(), 4886);
    assert!(gate
        .evaluate(PredicateContext::with_revision(
            &with_continue,
            ClientRevision::R377
        ))
        .failure_reason()
        .unwrap()
        .contains("active continue dialogue"));
}

#[test]
fn duel_round_trip_pass_and_fail_cases() {
    let gate = NamedPredicateGate::bind(
        "duel_round_trip",
        NamedPredicateParams::DuelRoundTrip(DuelRoundTripHandle::new()),
        PredicateAuthority::Script,
    )
    .unwrap();
    assert_eq!(gate.evidence(), EVIDENCE_DUEL_ROUND_TRIP);

    let lobby = WorldTile {
        x: 3366,
        z: 3268,
        level: 0,
    };
    let pen = WorldTile {
        x: 3340,
        z: 3250,
        level: 0,
    };
    let lumbridge = WorldTile {
        x: 3222,
        z: 3218,
        level: 0,
    };

    // Lobby before entering any fight pen fails.
    assert!(gate
        .evaluate(PredicateContext::snapshot_only(&ingame_snapshot(lobby, 1)))
        .failure_reason()
        .unwrap()
        .contains("fight pen not yet entered"));

    // Wrong level fails.
    assert!(gate
        .evaluate(PredicateContext::snapshot_only(&ingame_snapshot(
            WorldTile {
                x: pen.x,
                z: pen.z,
                level: 1,
            },
            2,
        )))
        .failure_reason()
        .unwrap()
        .contains("expected level 0"));

    // Inside fight pen latches `fought` but does not pass yet.
    assert!(gate
        .evaluate(PredicateContext::snapshot_only(&ingame_snapshot(pen, 3)))
        .failure_reason()
        .unwrap()
        .contains("in duel fight pen"));

    // Outside duel zone after fight fails.
    assert!(gate
        .evaluate(PredicateContext::snapshot_only(&ingame_snapshot(
            lumbridge, 4
        )))
        .failure_reason()
        .unwrap()
        .contains("outside duel zone"));

    // Returning to Duel Zone lobby passes.
    assert_eq!(
        gate.evaluate(PredicateContext::snapshot_only(&ingame_snapshot(lobby, 5))),
        PredicateVerdict::Pass
    );
}

#[test]
fn evaluate_by_name_convenience_dispatches_and_fails_closed() {
    let stand = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    let snap = ingame_snapshot(stand, 1);
    let ok = evaluate_by_name(
        "standing_bank_closed",
        NamedPredicateParams::StandingBankClosed(StandingBankClosedParams { stand }),
        PredicateAuthority::Script,
        PredicateContext::snapshot_only(&snap),
    )
    .unwrap();
    assert_eq!(ok, PredicateVerdict::Pass);

    let err = evaluate_by_name(
        "clue_family_summary",
        NamedPredicateParams::None,
        PredicateAuthority::Table,
        PredicateContext::snapshot_only(&snap),
    )
    .unwrap_err();
    assert_eq!(
        err,
        NamedPredicateError::NonCallableSummary {
            name: "clue_family_summary",
            inventory_row: 9,
        }
    );
}
