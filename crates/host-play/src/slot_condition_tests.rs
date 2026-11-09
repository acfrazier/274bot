use super::*;
use api::condition::poison_math::remaining;
use api::snapshot::{
    ActorKind, ActorTargetView, ActorView, ChatLineView, HitmarkView, HitmarksView,
    LocalPlayerView, NpcView, PlayerView, StatView, WorldStateView, WorldTile,
};

fn frame(tick: u32) -> GameSnapshot {
    let tile = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    // Source attribution needs posted world/equipment families, not only
    // the HP and inventory that make the condition fact complete.
    snapshot.seed_world(WorldStateView::default());
    snapshot.seed_equipment(Vec::new());
    snapshot.seed_tick(tick);
    snapshot.seed_local_player(LocalPlayerView {
        player: PlayerView {
            index: 0,
            actor: ActorView {
                name: None,
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
                health: 60,
                total_health: 60,
                face_entity: -1,
                target: None,
                moving: false,
                running: false,
                in_combat: false,
            },
            network: tile,
            combat_level: 3,
            skill_level: 0,
            headicons: 0,
            weapon: None,
        },
        energy: 100,
        weight: 0,
        animation_update: None,
    });
    snapshot.seed_stats(vec![StatView {
        index: 3,
        name: "Hitpoints".into(),
        effective: 60,
        base: 60,
        xp: 0,
        used: true,
    }]);
    snapshot.seed_inventory(Vec::new(), 28);
    snapshot.seed_inventory_packet_generations(Some(0), None);
    snapshot
}

fn npc(ident: usize, index: usize) -> NpcView {
    let tile = WorldTile {
        x: 3201,
        z: 3200,
        level: 0,
    };
    NpcView {
        index,
        r#type: Some(ident),
        name: None,
        actions: Vec::new(),
        tile,
        distance: 1,
        animation: -1,
        animation_frame: 0,
        pose_animation: -1,
        orientation: 0,
        target_orientation: 0,
        overhead_text: None,
        spot_animation: -1,
        spot_animation_stamp: 0,
        health: 20,
        total_health: 20,
        face_entity: 32768,
        target: Some(ActorTargetView {
            kind: ActorKind::Player,
            index: 0,
        }),
        moving: false,
        running: false,
        in_combat: true,
        level: 20,
        size: 1,
        network: tile,
        x: 0,
        z: 0,
        yaw: 0,
    }
}

fn drain(edge: bool, coherent: bool) -> DrainResult {
    DrainResult {
        player_info: edge,
        npc_rows_coherent: coherent,
        session_changed: false,
        gens: Default::default(),
        dirty: Default::default(),
    }
}

fn owner() -> SlotCondition {
    let mut owner = SlotCondition::default();
    owner.session_started(
        0,
        0,
        Some((script::combat::guard::shared_tables().unwrap(), 11)),
    );
    owner.observe_frame(&frame(0), &drain(false, true), false);
    owner
}

fn poison_window(owner: &SlotCondition) -> PoisonWindow {
    owner
        .current()
        .expect("complete live condition")
        .poison_window
        .expect("unknown poison publishes its window")
}

fn chat(snapshot: &mut GameSnapshot, sequence: i32, text: &str) {
    snapshot.seed_chat_lines(vec![ChatLineView {
        type_: 0,
        username: None,
        text: text.into(),
        sequence,
    }]);
}

fn hit(snapshot: &mut GameSnapshot, kind: i32, value: i32, serial: i32) {
    let mut marks = [HitmarkView {
        kind: 0,
        value: 0,
        cycle: -1,
    }; 4];
    marks[0] = HitmarkView {
        kind,
        value,
        cycle: 100 + serial,
    };
    snapshot.seed_hitmarks(HitmarksView {
        marks,
        loop_cycle: serial,
    });
}

fn chat_pair(snapshot: &mut GameSnapshot, older: (i32, &str), newer: (i32, &str)) {
    snapshot.seed_chat_lines(vec![
        ChatLineView {
            type_: 0,
            username: None,
            text: newer.1.into(),
            sequence: newer.0,
        },
        ChatLineView {
            type_: 0,
            username: None,
            text: older.1.into(),
            sequence: older.0,
        },
    ]);
}

fn poisoned_owner_with_fruit() -> (SlotCondition, GameSnapshot) {
    let mut owner = owner();
    let mut snapshot = frame(1);
    inventory(&mut snapshot, 1, &[464]);
    hit(&mut snapshot, 2, 6, 1);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert!(matches!(
        owner.current().unwrap().poison,
        PoisonState::Poisoned { .. }
    ));
    (owner, snapshot)
}

fn assert_onset(owner: &SlotCondition, q: u8, latched: bool, refreshed: bool) {
    assert_eq!(
        owner.current().unwrap().poison,
        PoisonState::Onset {
            since: 1,
            q: latched.then_some(q),
            hit_since_line: refreshed
        }
    );
}

#[test]
fn incomplete_frame_at_same_tick_hides_fact() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut owner = owner();
    let mut snapshot = frame(1);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert_eq!(owner.current().unwrap().tick, 1);
    snapshot.seed_inventory(Vec::new(), 0);
    owner.observe_frame(&snapshot, &drain(false, true), false);
    assert!(owner.current().is_none());
    assert_eq!(owner.fact.tick, 1);
    snapshot.seed_inventory(Vec::new(), 28);
    owner.observe_frame(&snapshot, &drain(false, true), false);
    assert_eq!(owner.current().unwrap().tick, 1);
}

#[test]
fn reconnect_hides_fact_until_first_complete_frame() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut owner = owner();
    owner.observe_frame(&frame(200), &drain(true, true), false);
    owner.session_ended();
    assert!(owner.current().is_none());
    owner.session_started(
        1,
        0,
        Some((script::combat::guard::shared_tables().unwrap(), 11)),
    );
    assert!(owner.current().is_none());
    assert_eq!(owner.published().session, SessionMark::Boundary);
    owner.observe_frame(&frame(1), &drain(false, true), false);
    assert_eq!(
        owner.current().unwrap().poison,
        PoisonState::Unknown { since: 1 }
    );
    assert_eq!(owner.last_tick, 1);
}

#[test]
fn unavailable_tables_keep_food_and_cures_unknown() {
    let mut owner = SlotCondition::default();
    owner.session_started(1, 0, None);
    let mut snapshot = frame(1);
    inventory(&mut snapshot, 1, &[179, 379]);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert!(
        owner.current().is_none(),
        "zero summaries must not publish as known"
    );
    assert_eq!(owner.published().session, SessionMark::Boundary);
    assert_eq!(
        owner.inv_gen, None,
        "unknown content cannot consume inventory"
    );
}

#[test]
fn reconnect_watermark_ignores_retained_and_unread_old_drinks() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut owner = owner();
    let mut snapshot = frame(1);
    chat(&mut snapshot, 1, DRINK_LINE);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert!(matches!(
        owner.current().unwrap().poison,
        PoisonState::Immune { .. }
    ));
    owner.session_ended();
    // Client position 2 includes a line the previous snapshot never consumed.
    owner.session_started(
        1,
        2,
        Some((script::combat::guard::shared_tables().unwrap(), 11)),
    );
    chat(&mut snapshot, 2, DRINK_LINE);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert_eq!(
        owner.current().unwrap().poison,
        PoisonState::Unknown { since: 1 }
    );
    snapshot.seed_tick(2);
    chat(&mut snapshot, 3, DRINK_LINE);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert_eq!(
        owner.current().unwrap().poison,
        PoisonState::Immune { until_lower: 122 }
    );
}

#[test]
fn player_chat_cannot_spoof_poison_drink_or_fruit() {
    if !api::test_support::game_data_available() {
        return;
    }
    for (type_, username) in [
        (2, Some("Other player")),
        (3, Some("Other player")),
        (7, Some("Other player")),
        (9, Some("Clan member")),
        (0, Some("Named sender")),
        (2, None),
    ] {
        let mut owner = owner();
        let mut snapshot = frame(1);
        inventory(&mut snapshot, 1, &[464]);
        for (tick, text) in [(1, POISON_LINE), (3, DRINK_LINE), (4, FRUIT_LINE)] {
            if tick == 3 {
                snapshot.seed_tick(2);
                hit(&mut snapshot, 2, 6, 1);
                owner.observe_frame(&snapshot, &drain(true, true), false);
            }
            snapshot.seed_tick(tick);
            if tick == 4 {
                inventory(&mut snapshot, 2, &[]);
            }
            snapshot.seed_chat_lines(vec![ChatLineView {
                type_,
                username: username.map(str::to_owned),
                text: text.into(),
                sequence: tick as i32,
            }]);
            owner.observe_frame(&snapshot, &drain(true, true), false);
            if tick == 1 {
                assert!(matches!(
                    owner.current().unwrap().poison,
                    PoisonState::Unknown { .. }
                ));
            } else {
                assert!(matches!(
                    owner.current().unwrap().poison,
                    PoisonState::Poisoned { .. }
                ));
            }
        }
        snapshot.seed_tick(5);
        chat(&mut snapshot, 5, DRINK_LINE);
        owner.observe_frame(&snapshot, &drain(true, true), false);
        assert_eq!(
            owner.current().unwrap().poison,
            PoisonState::Immune { until_lower: 125 }
        );
    }
}

#[test]
fn queued_poison_marks_keep_only_proven_drink_immunity() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut owner = owner();
    let mut snapshot = frame(1);
    chat(&mut snapshot, 1, DRINK_LINE);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    for tick in [3, 121] {
        snapshot.seed_tick(tick);
        hit(&mut snapshot, 2, 6, tick as i32);
        owner.observe_frame(&snapshot, &drain(true, true), false);
        assert_eq!(
            owner.current().unwrap().poison,
            PoisonState::Immune { until_lower: 121 }
        );
    }
    snapshot.seed_tick(122);
    hit(&mut snapshot, 2, 6, 122);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert_eq!(
        owner.current().unwrap().poison,
        PoisonState::Poisoned {
            per_tick: 6,
            last_mark: 122,
            hit_since_mark: false
        }
    );
}

#[test]
fn consumer_rejects_stale_stamp() {
    let snapshot = frame(2);
    let stale = BotCondition {
        tick: 1,
        session: SessionMark::Live,
        poison: PoisonState::Clear,
        ..Default::default()
    };
    let input = crate::admission::capture(&snapshot, true, Some(&stale), false, None);
    assert!(matches!(input.poison, PoisonState::Unknown { .. }));
}

#[test]
fn slot_condition_consumes_chat_at_edges_only() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut owner = owner();
    let mut snapshot = frame(0);
    chat(&mut snapshot, 1, POISON_LINE);
    owner.observe_frame(&snapshot, &drain(false, true), false);
    assert!(matches!(
        owner.current().unwrap().poison,
        PoisonState::Unknown { .. }
    ));
    assert_eq!(owner.chat_seen, 0);
    snapshot.seed_tick(1);
    snapshot.seed_npcs(vec![npc(108, 1)]);
    hit(&mut snapshot, 1, 1, 1);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert_onset(&owner, 3, true, false);
    assert_eq!(owner.chat_seen, 1);
}

#[test]
fn reducer_latch_future_line_with_coherent_weaker_rows_uses_q_unknown() {
    if !api::test_support::game_data_available() {
        return;
    } // Synthetic replay of RS5 R5-01: PLAYER_INFO t (hit), NPC_INFO t
      // (scorpion), then the first poison chat of t+1 before PLAYER_INFO t+1.
    let mut owner = owner();
    let mut snapshot = frame(1);
    snapshot.seed_npcs(vec![npc(108, 1)]);
    hit(&mut snapshot, 1, 1, 1);
    chat(&mut snapshot, 1, POISON_LINE);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert_onset(&owner, 11, true, false);
    assert!(owner.latch.is_none());
    let outlook = remaining(owner.current().unwrap().poison, Exposure::None, 1, 11);
    assert_eq!(outlook.total, 330);
    assert!(outlook.total >= u16::from(owner.current().unwrap().hp));
}

#[test]
fn reducer_latch_split_drain_stale_npc_rows_waits_for_coherent_frame() {
    if !api::test_support::game_data_available() {
        return;
    }
    for stronger in [false, true] {
        let mut owner = owner();
        let mut snapshot = frame(1);
        snapshot.seed_npcs(vec![npc(108, 1)]);
        hit(&mut snapshot, 1, 1, 1);
        chat(&mut snapshot, 1, POISON_LINE);
        owner.observe_frame(&snapshot, &drain(true, false), false);
        assert_onset(&owner, 11, false, false);
        if stronger {
            snapshot.seed_npcs(vec![npc(108, 1), npc(191, 2)]);
        }
        owner.observe_frame(&snapshot, &drain(false, true), false);
        assert_onset(&owner, if stronger { 11 } else { 3 }, true, false);
    }
}

#[test]
fn reducer_edge_order_old_latch_closes_before_new_edge_evidence() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut owner = owner();
    let mut snapshot = frame(1);
    chat(&mut snapshot, 1, POISON_LINE);
    hit(&mut snapshot, 1, 1, 1);
    owner.observe_frame(&snapshot, &drain(true, false), false);
    snapshot.seed_tick(2);
    snapshot.seed_npcs(vec![npc(108, 1)]);
    hit(&mut snapshot, 1, 1, 2);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert_onset(&owner, 11, true, true);
}

#[test]
fn reducer_latch_window_ends_ambiguous_uses_q_unknown() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut owner = owner();
    let mut snapshot = frame(1);
    chat(&mut snapshot, 1, POISON_LINE);
    hit(&mut snapshot, 1, 1, 1);
    owner.observe_frame(&snapshot, &drain(true, false), false);
    snapshot.seed_tick(2);
    owner.observe_frame(&snapshot, &drain(true, false), false);
    assert_onset(&owner, 11, true, false);
}

#[test]
fn reducer_latch_disqualifying_edges_use_q_unknown() {
    if !api::test_support::game_data_available() {
        return;
    }
    for (tick, previous_complete, hit_kind) in
        [(2, true, Some(1)), (1, false, Some(1)), (1, true, None)]
    {
        let mut owner = owner();
        owner.complete = previous_complete;
        let mut snapshot = frame(tick);
        snapshot.seed_npcs(vec![npc(108, 1)]);
        chat(&mut snapshot, 1, POISON_LINE);
        if let Some(kind) = hit_kind {
            hit(&mut snapshot, kind, 0, 1);
        }
        owner.observe_frame(&snapshot, &drain(true, false), false);
        assert!(matches!(
            owner.current().unwrap().poison,
            PoisonState::Onset { q: Some(11), .. }
        ));
    }
}

#[test]
fn reducer_latch_no_priced_poison_source_uses_q_unknown() {
    if !api::test_support::game_data_available() {
        return;
    }
    for rows in [vec![npc(1, 1)], vec![npc(108, 1), npc(191, 2)]] {
        let mut owner = owner();
        let mut snapshot = frame(1);
        snapshot.seed_npcs(rows);
        chat(&mut snapshot, 1, POISON_LINE);
        hit(&mut snapshot, 0, 0, 1);
        owner.observe_frame(&snapshot, &drain(true, false), false);
        owner.observe_frame(&snapshot, &drain(false, true), false);
        assert_onset(&owner, 11, true, false);
    }
}

#[test]
fn reducer_latch_never_lowers() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut owner = owner();
    let mut snapshot = frame(1);
    chat(&mut snapshot, 1, POISON_LINE);
    hit(&mut snapshot, 1, 1, 1);
    snapshot.seed_npcs(vec![npc(191, 1)]);
    owner.observe_frame(&snapshot, &drain(true, false), false);
    owner.observe_frame(&snapshot, &drain(false, true), false);
    assert_onset(&owner, 11, true, false);
    snapshot.seed_npcs(vec![npc(108, 1)]);
    owner.observe_frame(&snapshot, &drain(false, true), false);
    assert_onset(&owner, 11, true, false);
}

#[test]
fn slot_condition_feeds_kind2_mark_and_chat() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut owner = owner();
    let mut snapshot = frame(1);
    chat(&mut snapshot, 1, POISON_LINE);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert_onset(&owner, 11, true, false);
    snapshot.seed_tick(31);
    hit(&mut snapshot, 2, 6, 1);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert_eq!(
        owner.current().unwrap().poison,
        PoisonState::Poisoned {
            per_tick: 6,
            last_mark: 31,
            hit_since_mark: false
        }
    );
    // Wake frames must not consume new hitmarks.
    hit(&mut snapshot, 2, 5, 2);
    owner.observe_frame(&snapshot, &drain(false, true), false);
    assert!(matches!(
        owner.current().unwrap().poison,
        PoisonState::Poisoned { per_tick: 6, .. }
    ));
    snapshot.seed_tick(32);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert!(matches!(
        owner.current().unwrap().poison,
        PoisonState::Poisoned { per_tick: 5, .. }
    ));
}

#[test]
fn drink_line_alone_credits_ordinary() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut owner = owner();
    let mut snapshot = frame(1);
    chat(&mut snapshot, 1, DRINK_LINE);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert_eq!(
        owner.current().unwrap().poison,
        PoisonState::Immune { until_lower: 121 }
    );
    snapshot.seed_tick(121);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert!(matches!(
        owner.current().unwrap().poison,
        PoisonState::Immune { .. }
    ));
    snapshot.seed_tick(122);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert_eq!(owner.current().unwrap().poison, PoisonState::Clear);
}

#[test]
fn effect_origin_backlog_drain_of_three_credits_from_t_minus_two() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut owner = owner();
    let mut snapshot = frame(3);
    chat(&mut snapshot, 1, DRINK_LINE);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert_eq!(
        owner.current().unwrap().poison,
        PoisonState::Immune { until_lower: 121 }
    );
}

#[test]
fn two_lines_in_one_edge_credit_once() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut owner = owner();
    let mut snapshot = frame(3);
    snapshot.seed_chat_lines(vec![
        ChatLineView {
            type_: 0,
            username: None,
            text: DRINK_LINE.into(),
            sequence: 2,
        },
        ChatLineView {
            type_: 0,
            username: None,
            text: DRINK_LINE.into(),
            sequence: 1,
        },
    ]);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert_eq!(
        owner.current().unwrap().poison,
        PoisonState::Immune { until_lower: 121 }
    );
}

fn inventory(snapshot: &mut GameSnapshot, generation: u64, ids: &[i32]) {
    use api::snapshot::{ItemActionFamily, ItemContainer, ItemView};
    snapshot.seed_inventory(
        ids.iter()
            .enumerate()
            .map(|(slot, &id)| ItemView {
                def: api::ItemDefView {
                    id,
                    name: None,
                    stackable: false,
                    members: true,
                    base_value: 0,
                    noted: false,
                    certificate_link: -1,
                    certificate_template: -1,
                },
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: slot as i32,
                count: 1,
                actions: Vec::new(),
                component_id: -1,
            })
            .collect(),
        28,
    );
    snapshot.seed_inventory_packet_generations(Some(generation), None);
}

#[test]
fn dose_drop_without_line_does_not_clear_poison() {
    if !api::test_support::game_data_available() {
        return;
    }
    for id in [179, 185, 464] {
        let mut owner = owner();
        let mut snapshot = frame(1);
        inventory(&mut snapshot, 1, &[id]);
        hit(&mut snapshot, 2, 6, 1);
        owner.observe_frame(&snapshot, &drain(true, true), false);
        snapshot.seed_tick(2);
        inventory(&mut snapshot, 2, &[]);
        owner.observe_frame(&snapshot, &drain(true, true), false);
        assert!(matches!(
            owner.current().unwrap().poison,
            PoisonState::Poisoned { per_tick: 6, .. }
        ));
        assert_eq!(owner.current().unwrap().cures, Default::default());
    }
}

#[test]
fn ordinary_and_super_drops_with_line_credit_only_ordinary_bound() {
    if !api::test_support::game_data_available() {
        return;
    }
    for id in [179, 185] {
        let mut owner = owner();
        let mut snapshot = frame(1);
        inventory(&mut snapshot, 1, &[id]);
        hit(&mut snapshot, 2, 6, 1);
        owner.observe_frame(&snapshot, &drain(true, true), false);
        snapshot.seed_tick(2);
        inventory(&mut snapshot, 2, &[]);
        chat(&mut snapshot, 1, DRINK_LINE);
        owner.observe_frame(&snapshot, &drain(true, true), false);
        assert_eq!(
            owner.current().unwrap().poison,
            PoisonState::Immune { until_lower: 122 }
        );
    }
}

#[test]
fn fruit_line_requires_fruit_loss_and_grants_no_future_fire() {
    if !api::test_support::game_data_available() {
        return;
    }
    for consumed in [false, true] {
        let mut owner = owner();
        let mut snapshot = frame(1);
        inventory(&mut snapshot, 1, &[464]);
        hit(&mut snapshot, 2, 6, 1);
        owner.observe_frame(&snapshot, &drain(true, true), false);
        assert_eq!(owner.current().unwrap().cures.strange_fruit, 1);
        snapshot.seed_tick(2);
        if consumed {
            inventory(&mut snapshot, 2, &[]);
        }
        chat(&mut snapshot, 1, FRUIT_LINE);
        owner.observe_frame(&snapshot, &drain(true, true), false);
        if consumed {
            assert_eq!(
                owner.current().unwrap().poison,
                PoisonState::Immune { until_lower: 2 }
            );
            snapshot.seed_tick(3);
            owner.observe_frame(&snapshot, &drain(true, true), false);
            assert_eq!(owner.current().unwrap().poison, PoisonState::Clear);
        } else {
            assert!(matches!(
                owner.current().unwrap().poison,
                PoisonState::Poisoned { .. }
            ));
        }
    }
}

#[test]
fn fruit_chat_then_count_loss_match_on_adjacent_ticks() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut owner, mut snapshot) = poisoned_owner_with_fruit();
    snapshot.seed_tick(2);
    chat(&mut snapshot, 1, FRUIT_LINE);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert!(matches!(
        owner.current().unwrap().poison,
        PoisonState::Poisoned { .. }
    ));

    snapshot.seed_tick(3);
    inventory(&mut snapshot, 2, &[]);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert_eq!(owner.current().unwrap().poison, PoisonState::Clear);
}

#[test]
fn fruit_count_loss_then_chat_match_on_adjacent_ticks() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut owner, mut snapshot) = poisoned_owner_with_fruit();
    snapshot.seed_tick(2);
    inventory(&mut snapshot, 2, &[]);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert!(matches!(
        owner.current().unwrap().poison,
        PoisonState::Poisoned { .. }
    ));

    snapshot.seed_tick(3);
    chat(&mut snapshot, 1, FRUIT_LINE);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert_eq!(owner.current().unwrap().poison, PoisonState::Clear);
}

#[test]
fn fruit_chat_and_count_loss_do_not_match_after_the_content_bound() {
    if !api::test_support::game_data_available() {
        return;
    }
    for chat_first in [true, false] {
        let (mut owner, mut snapshot) = poisoned_owner_with_fruit();
        snapshot.seed_tick(2);
        if chat_first {
            chat(&mut snapshot, 1, FRUIT_LINE);
        } else {
            inventory(&mut snapshot, 2, &[]);
        }
        owner.observe_frame(&snapshot, &drain(true, true), false);

        snapshot.seed_tick(3);
        owner.observe_frame(&snapshot, &drain(true, true), false);

        snapshot.seed_tick(4);
        if chat_first {
            inventory(&mut snapshot, 2, &[]);
        } else {
            chat(&mut snapshot, 1, FRUIT_LINE);
        }
        owner.observe_frame(&snapshot, &drain(true, true), false);
        assert!(matches!(
            owner.current().unwrap().poison,
            PoisonState::Poisoned { .. }
        ));
    }
}

#[test]
fn fruit_chat_pairs_with_count_loss_after_a_k_two_drain() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut owner, mut snapshot) = poisoned_owner_with_fruit();
    snapshot.seed_tick(2);
    chat(&mut snapshot, 1, FRUIT_LINE);
    owner.observe_frame(&snapshot, &drain(true, true), false);

    snapshot.seed_tick(4);
    inventory(&mut snapshot, 2, &[]);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert_eq!(owner.current().unwrap().poison, PoisonState::Clear);
}

#[test]
fn fruit_chat_pairs_with_count_loss_in_a_later_non_edge_frame() {
    if !api::test_support::game_data_available() {
        return;
    } // The line of tick 3 can be read at the edge of tick 2, and the inventory of
      // tick 3 can arrive after PLAYER_INFO(3) in a frame without PLAYER_INFO.
    let (mut owner, mut snapshot) = poisoned_owner_with_fruit();
    snapshot.seed_tick(2);
    chat(&mut snapshot, 1, FRUIT_LINE);
    owner.observe_frame(&snapshot, &drain(true, true), false);

    snapshot.seed_tick(3);
    owner.observe_frame(&snapshot, &drain(true, true), false);

    inventory(&mut snapshot, 2, &[]);
    owner.observe_frame(&snapshot, &drain(false, true), false);
    assert_eq!(
        owner.current().unwrap().poison,
        PoisonState::Immune { until_lower: 2 }
    );
}

#[test]
fn same_edge_fruit_then_newer_poison_line_is_onset() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut owner, mut snapshot) = poisoned_owner_with_fruit();
    snapshot.seed_tick(2);
    inventory(&mut snapshot, 2, &[]);
    chat_pair(&mut snapshot, (1, FRUIT_LINE), (2, POISON_LINE));
    owner.observe_frame(&snapshot, &drain(true, true), false);

    snapshot.seed_tick(3);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert!(matches!(
        owner.current().unwrap().poison,
        PoisonState::Onset { .. }
    ));
}

#[test]
fn split_fruit_chat_then_poison_line_with_count_loss_is_onset() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut owner, mut snapshot) = poisoned_owner_with_fruit();
    snapshot.seed_tick(2);
    chat(&mut snapshot, 1, FRUIT_LINE);
    owner.observe_frame(&snapshot, &drain(true, true), false);

    snapshot.seed_tick(3);
    inventory(&mut snapshot, 2, &[]);
    chat_pair(&mut snapshot, (1, FRUIT_LINE), (2, POISON_LINE));
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert!(matches!(
        owner.current().unwrap().poison,
        PoisonState::Onset { .. }
    ));
}

#[test]
fn newer_poison_line_cancels_waiting_fruit_chat() {
    if !api::test_support::game_data_available() {
        return;
    } // One edge holds the fruit line (older) and a newer poison line; the count
      // loss comes on the next edge. The clear must stand, or the loss would cure
      // the newer onset.
    let (mut owner, mut snapshot) = poisoned_owner_with_fruit();
    snapshot.seed_tick(2);
    chat_pair(&mut snapshot, (1, FRUIT_LINE), (2, POISON_LINE));
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert!(matches!(
        owner.current().unwrap().poison,
        PoisonState::Onset { since: 2, .. }
    ));

    snapshot.seed_tick(3);
    inventory(&mut snapshot, 2, &[]);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert_eq!(
        owner.current().unwrap().poison,
        PoisonState::Onset {
            since: 2,
            q: Some(11),
            hit_since_line: false
        }
    );
}

#[test]
fn poison_window_clear_on_the_31st_clean_edge() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut owner = owner();
    owner.accepted_click();
    for tick in 1..=30 {
        owner.observe_frame(&frame(tick), &drain(true, true), false);
    }
    assert_eq!(
        owner.current().unwrap().poison,
        PoisonState::Unknown { since: 0 },
        "old: `> 33` cleared on clean edge 34; new: clean edge 30 is still Unknown"
    );
    assert_eq!(
        poison_window(&owner),
        PoisonWindow {
            remaining: 1,
            accepted: true,
            last_reset: WindowReset::Interaction,
        },
        "old: no window remainder was published; new: one clean edge remains"
    );

    owner.observe_frame(&frame(31), &drain(true, true), false);
    assert_eq!(
        owner.current().unwrap().poison,
        PoisonState::Clear,
        "old: `> 33` cleared on clean edge 34; new: `>= 31` clears on clean edge 31"
    );
    assert_eq!(owner.current().unwrap().poison_window, None);
}

#[test]
fn poison_window_remaining_counts_down() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut owner = owner();
    owner.accepted_click();
    for tick in 1..=12 {
        owner.observe_frame(&frame(tick), &drain(true, true), false);
    }
    assert_eq!(
        poison_window(&owner).remaining,
        19,
        "old: clean 12 reported 18 remaining; new: a 31-tick window reports 19"
    );
    owner.observe_frame(&frame(13), &drain(true, true), false);
    assert_eq!(
        poison_window(&owner).remaining,
        18,
        "old: clean 13 reported 17 remaining; new: a 31-tick window reports 18"
    );
}

#[test]
fn poison_window_not_accepted_needs_one_edge() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut owner = owner();
    for tick in 1..=31 {
        owner.observe_frame(&frame(tick), &drain(true, true), false);
    }
    assert_eq!(
        owner.current().unwrap().poison,
        PoisonState::Unknown { since: 0 },
        "old: acceptance was not exposed; new: 31 clean edges still need an accepted dispatch"
    );
    assert_eq!(
        poison_window(&owner),
        PoisonWindow {
            remaining: 1,
            accepted: false,
            last_reset: WindowReset::NotReset,
        },
        "old: missing acceptance was indistinguishable; new: exactly one edge remains"
    );

    let mut client = outgoing_client(client::io::ClientRevision::R289);
    let checkpoint = owner.dispatch_checkpoint(&client).unwrap();
    queue_packet(&mut client, ClientProt::MOVE_GAMECLICK);
    owner.observe_dispatch(&client, checkpoint);
    assert_eq!(
        owner.window.clean, 31,
        "movement accepts without resetting clean"
    );
    assert!(owner.window.accepted);
    owner.observe_frame(&frame(32), &drain(true, true), false);
    assert_eq!(
        owner.current().unwrap().poison,
        PoisonState::Clear,
        "old: no acceptance boundary was tested; new: one clean edge after movement proves Clear"
    );
}

#[test]
fn poison_window_interaction_restarts() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut owner = owner();
    owner.accepted_click();
    for tick in 1..=20 {
        owner.observe_frame(&frame(tick), &drain(true, true), false);
    }
    assert_eq!(poison_window(&owner).remaining, 11);

    let mut client = outgoing_client(client::io::ClientRevision::R289);
    let checkpoint = owner.dispatch_checkpoint(&client).unwrap();
    queue_packet(&mut client, ClientProt::OPLOC1);
    owner.observe_dispatch(&client, checkpoint);
    owner.observe_frame(&frame(20), &drain(false, true), false);
    assert_eq!(
        poison_window(&owner),
        PoisonWindow {
            remaining: 31,
            accepted: true,
            last_reset: WindowReset::Interaction,
        },
        "old: a door retained 11 clean edges; new: OPLOC restarts with an accepted 31-edge window"
    );
    for tick in 21..=40 {
        owner.observe_frame(&frame(tick), &drain(true, true), false);
    }
    assert_eq!(
        poison_window(&owner),
        PoisonWindow {
            remaining: 11,
            accepted: true,
            last_reset: WindowReset::Interaction,
        },
        "old: 20 clean approach edges left 10; new: a 31-edge door window leaves 11 at tick 40"
    );
}

#[test]
fn poison_window_modal_resets_and_names_cause() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut owner = owner();
    owner.accepted_click();
    for tick in 1..=12 {
        owner.observe_frame(&frame(tick), &drain(true, true), false);
    }

    let mut snapshot = frame(12);
    snapshot.seed_chat_modal(1, Vec::new());
    owner.observe_frame(&snapshot, &drain(false, true), false);
    assert_eq!(
        poison_window(&owner),
        PoisonWindow {
            remaining: 31,
            accepted: false,
            last_reset: WindowReset::Modal,
        },
        "old: the modal reset was unlabelled; new: it resets and records Modal"
    );
}

#[test]
fn poison_window_frame_gap_resets() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut owner = owner();
    owner.accepted_click();
    for tick in 1..=12 {
        owner.observe_frame(&frame(tick), &drain(true, true), false);
    }

    owner.observe_frame(&frame(14), &drain(true, true), false);
    assert_eq!(
        poison_window(&owner),
        PoisonWindow {
            remaining: 31,
            accepted: false,
            last_reset: WindowReset::FrameGap,
        },
        "old: a skipped PLAYER_INFO edge had no cause; new: it restarts and records FrameGap"
    );
}

#[test]
fn poison_window_incomplete_trace_resets() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut owner = owner();
    owner.accepted_click();
    for tick in 1..=12 {
        owner.observe_frame(&frame(tick), &drain(true, true), false);
    }

    let mut client = outgoing_client(client::io::ClientRevision::R289);
    let checkpoint = owner.dispatch_checkpoint(&client).unwrap();
    let packet = map_client_prot(client.revision(), ClientProt::IF_BUTTON);
    client.out.p1_enc(packet.id);
    owner.observe_dispatch(&client, checkpoint);
    owner.observe_frame(&frame(12), &drain(false, true), false);
    assert_eq!(
        poison_window(&owner),
        PoisonWindow {
            remaining: 31,
            accepted: false,
            last_reset: WindowReset::IncompleteTrace,
        },
        "old: an undecodable dispatch only cleared progress; new: it records IncompleteTrace"
    );
}

#[test]
fn poison_window_incomplete_fact_resets() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut owner = owner();
    owner.accepted_click();
    for tick in 1..=12 {
        owner.observe_frame(&frame(tick), &drain(true, true), false);
    }

    let mut incomplete = frame(12);
    incomplete.seed_inventory(Vec::new(), 0);
    owner.observe_frame(&incomplete, &drain(false, true), false);
    assert!(owner.current().is_none());
    owner.observe_frame(&frame(12), &drain(false, true), false);
    assert_eq!(
    poison_window(&owner),
    PoisonWindow {
        remaining: 31,
        accepted: false,
        last_reset: WindowReset::IncompleteFact,
    },
    "old: a missing condition fact hid the reset cause; new: it restarts and records IncompleteFact"
);
}

#[test]
fn poison_mark_on_31st_clean_edge_prevents_clear() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut owner = owner();
    owner.accepted_click();
    for tick in 1..=30 {
        owner.observe_frame(&frame(tick), &drain(true, true), false);
    }
    assert_eq!(poison_window(&owner).remaining, 1);

    let mut snapshot = frame(31);
    hit(&mut snapshot, 2, 6, 1);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert_eq!(
        owner.current().unwrap().poison,
        PoisonState::Poisoned {
            per_tick: 6,
            last_mark: 31,
            hit_since_mark: false,
        },
        "old: a 30-tick clear could hide the 31st-edge mark; new: the poison mark wins"
    );
    assert_eq!(owner.current().unwrap().poison_window, None);
}

#[test]
fn delayed_login_mark_never_follows_false_clear() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut owner = owner();
    owner.accepted_click();
    for tick in 1..=30 {
        owner.observe_frame(&frame(tick), &drain(true, true), false);
    }
    owner.accepted_click();
    // A content delay covers the original timer fire. Its mark arrives
    // after an uninterrupted 31-edge window would clear, but inside the restarted
    // window.
    for tick in 31..=39 {
        owner.observe_frame(&frame(tick), &drain(true, true), false);
        assert_eq!(
            owner.current().unwrap().poison,
            PoisonState::Unknown { since: 0 },
            "tick {tick} must not claim poison is clear before the mark"
        );
    }
    let mut snapshot = frame(40);
    hit(&mut snapshot, 2, 6, 1);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert_eq!(
        owner.current().unwrap().poison,
        PoisonState::Poisoned {
            per_tick: 6,
            last_mark: 40,
            hit_since_mark: false,
        }
    );
}

fn outgoing_client(revision: client::io::ClientRevision) -> Client {
    let mut client = Client::new_with_revision(
        client::client::ClientConfig {
            host: "127.0.0.1".into(),
            port: 0,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        revision,
    );
    client.ingame = true;
    client.out.random = Some(client::io::Isaac::new(&[1, 2, 3, 4]));
    client
}

fn queue_packet(client: &mut Client, packet: ClientProt) {
    let selected = map_client_prot(client.revision(), packet);
    client.out.p1_enc(selected.id);
    let length = if selected.length == -1 {
        client.out.p1(5);
        5
    } else {
        selected.length
    };
    for _ in 0..length {
        client.out.p1(0);
    }
}

#[test]
fn outgoing_interactions_restart_only_unknown_login_poison() {
    if !api::test_support::game_data_available() {
        return;
    } // Cover operations, item/widget uses, inventory controls and interface
      // continuations through the real encrypted outgoing decoder.
    for packet in [
        ClientProt::OPLOC1,
        ClientProt::OPNPC5,
        ClientProt::OPOBJ3,
        ClientProt::OPPLAYER2,
        ClientProt::OPHELD2,
        ClientProt::OPLOCU,
        ClientProt::OPNPCT,
        ClientProt::OPOBJU,
        ClientProt::OPPLAYERT,
        ClientProt::OPHELDT,
        ClientProt::OPHELDU,
        ClientProt::INV_BUTTON1,
        ClientProt::INV_BUTTON5,
        ClientProt::INV_BUTTOND,
        ClientProt::IF_BUTTON,
        ClientProt::RESUME_PAUSEBUTTON,
        ClientProt::RESUME_P_COUNTDIALOG,
        ClientProt::CLOSE_MODAL,
        ClientProt::TUT_CLICKSIDE,
    ] {
        let mut client = outgoing_client(client::io::ClientRevision::R289);
        let mut owner = owner();
        owner.accepted_click();
        for tick in 1..=30 {
            owner.observe_frame(&frame(tick), &drain(true, true), false);
        }
        let checkpoint = owner.dispatch_checkpoint(&client).unwrap();
        queue_packet(&mut client, packet);
        owner.observe_dispatch(&client, checkpoint);
        for tick in 31..=60 {
            owner.observe_frame(&frame(tick), &drain(true, true), false);
            assert_eq!(
            owner.current().unwrap().poison,
            PoisonState::Unknown { since: 0 },
            "packet {packet:?} at clean 30 resets the window; it remains Unknown through clean 30 of the new window"
        );
        }
        owner.observe_frame(&frame(61), &drain(true, true), false);
        assert_eq!(
            owner.current().unwrap().poison,
            PoisonState::Clear,
            "old: interaction needed the 34th edge; new: it clears on edge 31 after dispatch"
        );
        assert!(owner.dispatch_checkpoint(&client).is_none());

        let mut snapshot = frame(65);
        hit(&mut snapshot, 2, 6, 1);
        owner.observe_frame(&snapshot, &drain(true, true), false);
        assert!(owner.dispatch_checkpoint(&client).is_none());
        assert!(matches!(
            owner.current().unwrap().poison,
            PoisonState::Poisoned { per_tick: 6, .. }
        ));
    }
}

#[test]
fn walk_only_packets_accept_without_restarting_the_31_tick_window() {
    if !api::test_support::game_data_available() {
        return;
    }
    for packet in [
        ClientProt::MOVE_GAMECLICK,
        ClientProt::MOVE_MINIMAPCLICK,
        ClientProt::MOVE_OPCLICK,
    ] {
        let mut client = outgoing_client(client::io::ClientRevision::R289);
        let mut owner = owner();
        for tick in 1..=30 {
            let checkpoint = owner.dispatch_checkpoint(&client).unwrap();
            queue_packet(&mut client, packet);
            owner.observe_dispatch(&client, checkpoint);
            owner.observe_frame(&frame(tick), &drain(true, true), false);
            assert_eq!(
                owner.current().unwrap().poison,
                PoisonState::Unknown { since: 0 }
            );
        }
        owner.observe_frame(&frame(31), &drain(true, true), false);
        assert_eq!(
        owner.current().unwrap().poison,
        PoisonState::Clear,
        "old: `>33` delayed Clear until edge 34; new: movement accepts without resetting and clears at 31"
    );
    }
}

#[test]
fn empty_dispatch_and_telemetry_do_not_restart_login_window() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut client = outgoing_client(client::io::ClientRevision::R289);
    let mut owner = owner();
    owner.accepted_click();
    for tick in 1..=31 {
        let checkpoint = owner.dispatch_checkpoint(&client).unwrap();
        if tick % 2 == 0 {
            queue_packet(&mut client, ClientProt::EVENT_CAMERA_POSITION);
            queue_packet(&mut client, ClientProt::NO_TIMEOUT);
        }
        owner.observe_dispatch(&client, checkpoint);
        owner.observe_frame(&frame(tick), &drain(true, true), false);
    }
    assert_eq!(
        owner.current().unwrap().poison,
        PoisonState::Clear,
        "old: telemetry was tested against the delayed edge; new: clean edge 31 clears"
    );
}

#[test]
fn repeated_interactions_keep_login_poison_unknown() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut client = outgoing_client(client::io::ClientRevision::R289);
    let mut owner = owner();
    for tick in 1..=100 {
        if tick % 10 == 1 {
            let checkpoint = owner.dispatch_checkpoint(&client).unwrap();
            queue_packet(&mut client, ClientProt::IF_BUTTON);
            owner.observe_dispatch(&client, checkpoint);
        }
        owner.observe_frame(&frame(tick), &drain(true, true), false);
        assert_eq!(
            owner.current().unwrap().poison,
            PoisonState::Unknown { since: 0 }
        );
    }
}

/// The dispatch-checkpoint/trace gate runs the client the slot loop binds,
/// so the revision decides the opcode mapping `observe_dispatch` walks.
/// Each supported revision walks its own opcode table.
fn dispatch_checkpoint_allocations_gate(revision: client::io::ClientRevision) {
    if !api::test_support::game_data_available() {
        return;
    }
    let client = outgoing_client(revision);
    let mut owner = owner();
    let measured = allocation_counter::measure(|| {
        for _ in 0..1000 {
            let checkpoint = owner.dispatch_checkpoint(&client).unwrap();
            owner.observe_dispatch(&client, checkpoint);
        }
    });
    assert_eq!(measured.bytes_total, 0);
}

#[test]
fn dispatch_checkpoint_allocations_gate_r274() {
    dispatch_checkpoint_allocations_gate(client::io::ClientRevision::R274);
}

#[test]
fn dispatch_checkpoint_allocations_gate_r289() {
    dispatch_checkpoint_allocations_gate(client::io::ClientRevision::R289);
}

#[test]
fn dispatch_checkpoint_allocations_gate_r377() {
    dispatch_checkpoint_allocations_gate(client::io::ClientRevision::R377);
}

#[test]
fn guardian_hold_resets_and_records_the_clean_window_cause() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut owner = owner();
    owner.accepted_click();
    for tick in 1..=14 {
        owner.observe_frame(&frame(tick), &drain(true, true), false);
    }
    owner.observe_frame(&frame(14), &drain(false, true), true);
    assert_eq!(
        poison_window(&owner),
        PoisonWindow {
            remaining: 31,
            accepted: false,
            last_reset: WindowReset::GuardianHold,
        },
        "old: guardian reset had no published cause; new: it starts an unaccepted 31-edge window"
    );
    for tick in 15..=45 {
        owner.observe_frame(&frame(tick), &drain(true, true), false);
    }
    assert_eq!(
        poison_window(&owner),
        PoisonWindow {
            remaining: 1,
            accepted: false,
            last_reset: WindowReset::GuardianHold,
        }
    );
    owner.accepted_click();
    owner.observe_frame(&frame(45), &drain(false, true), false);
    assert_eq!(
        poison_window(&owner),
        PoisonWindow {
            remaining: 31,
            accepted: true,
            last_reset: WindowReset::Interaction,
        }
    );
    for tick in 46..=75 {
        owner.observe_frame(&frame(tick), &drain(true, true), false);
        assert_eq!(
            owner.current().unwrap().poison,
            PoisonState::Unknown { since: 0 }
        );
    }
    owner.observe_frame(&frame(76), &drain(true, true), false);
    assert_eq!(owner.current().unwrap().poison, PoisonState::Clear);
}

#[test]
fn admission_capture_reads_slot_condition() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut owner = owner();
    let mut snapshot = frame(1);
    hit(&mut snapshot, 2, 6, 1);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    let input = crate::admission::capture(&snapshot, false, owner.current(), false, None);
    assert_eq!(input.poison, owner.current().unwrap().poison);
    owner.complete = false;
    let input = crate::admission::capture(&snapshot, false, owner.current(), false, None);
    assert!(matches!(input.poison, PoisonState::Unknown { .. }));
}

/// The steady and line-frame gates observe the snapshot the slot loop
/// publishes for each supported revision.
fn condition_frame_allocations_gate(revision: client::io::ClientRevision) {
    let _ = revision;
    if !api::test_support::game_data_available() {
        return;
    }
    let mut steady_owner = owner();
    let snapshot = frame(0);
    let steady = drain(false, true);
    let measured = allocation_counter::measure(|| {
        for _ in 0..1000 {
            steady_owner.observe_frame(&snapshot, &steady, false);
        }
    });
    assert_eq!(measured.bytes_total, 0);

    let mut line_owner = owner();
    let mut line = frame(1);
    chat(&mut line, 1, POISON_LINE);
    hit(&mut line, 1, 1, 1);
    line.seed_npcs(vec![npc(108, 1)]);
    let mut drink = frame(2);
    chat(&mut drink, 2, DRINK_LINE);
    let measured = allocation_counter::measure(|| {
        line_owner.observe_frame(&line, &drain(true, false), false);
        line_owner.observe_frame(&line, &drain(false, true), false);
        line_owner.observe_frame(&drink, &drain(true, true), false);
    });
    assert_eq!(measured.bytes_total, 0);
}

#[test]
fn condition_frame_allocations_gate_r274() {
    condition_frame_allocations_gate(client::io::ClientRevision::R274);
}

#[test]
fn condition_frame_allocations_gate_r289() {
    condition_frame_allocations_gate(client::io::ClientRevision::R289);
}

#[test]
fn condition_frame_allocations_gate_r377() {
    condition_frame_allocations_gate(client::io::ClientRevision::R377);
}

#[test]
fn slot_condition_size_of_pinned() {
    assert_eq!(
        (
            std::mem::size_of::<SlotCondition>(),
            std::mem::size_of::<BotCondition>(),
            std::mem::size_of::<PoisonMemory>(),
            std::mem::size_of::<client::client::ClientGens>(),
            std::mem::size_of::<script::combat::risk::RiskInput>(),
        ),
        (104, 28, 8, 120, 124),
        "SlotCondition is 104 B: pending fruit uses existing padding; no allocation"
    );
    assert!(
        std::mem::size_of::<SlotCondition>() <= 160,
        "measured 104 B leaves 56 B under the 160 B owner ceiling"
    );
}

#[test]
fn client_gens_size_of_pinned() {
    assert_eq!(
        std::mem::size_of::<client::client::ClientGens>(),
        120,
        "npc_info adds one u64 to each of five retained copies: 40 B per bot, no allocation"
    );
}

// The CI memory gate for one warmed host tick per protocol revision
// (CI-MEMORY-GATE-1106 slice C): the production frame seam in the order
// `play_slots.rs` runs it — snapshot decode (`host::publish_snapshot`),
// the bot's own condition owner (frame, dispatch checkpoint, dispatch
// observation), the script tick (`SlotScript::on_game_tick` through the
// observe path), and the nav step (`step_nav_bot`). The 377 merge adds
// `warmed_tick_frame_allocations_gate_r377` beside the two tests below,
// calling the same helper with `ClientRevision::R377`.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex as StdMutex;

/// The idle native script the warmed tick gate runs: the real compiled-slot
/// poll path (`NativeTick` construction and the seat wiring) with no
/// effects of its own.
struct GateIdleScript;

impl script::native::Script for GateIdleScript {
    fn tick(
        &mut self,
        _cx: &mut script::native::NativeTick<'_>,
    ) -> Result<script::native::ScriptFlow, script::native::ScriptFailure> {
        Ok(script::native::ScriptFlow::Continue)
    }
}

/// A connected, ingame client at `revision` with the local player and the
/// hitpoints row armed once, so the first decode rebuilds those families.
/// The steady frame the gate measures leaves every generation unchanged.
fn gate_client(revision: client::io::ClientRevision) -> Client {
    let mut client = outgoing_client(revision);
    client.local_player = Some(client::dash3d::ClientPlayer::at(0, 0));
    client.stat_effective_level[3] = 60;
    client.stat_base_level[3] = 60;
    client.gens.player += 1;
    client.gens.player_info += 1;
    client.gens.scene += 1;
    client.gens.stat += 1;
    client
}

/// The gate's reach probe: the unavailable view behind a shared handle, so
/// a call costs nothing after the warm frame.
fn gate_reach() -> Arc<api::query::ReachQueryView> {
    static REACH: std::sync::LazyLock<Arc<api::query::ReachQueryView>> =
        std::sync::LazyLock::new(|| Arc::new(api::query::ReachQueryView::unavailable()));
    Arc::clone(&*REACH)
}

/// One warmed tick frame across the four production seams.
#[allow(clippy::too_many_arguments)]
fn gate_tick_frame(
    client: &mut Client,
    snapshot: &mut GameSnapshot,
    scripts: &crate::script_runtime::ScriptWall,
    cheats: &Arc<StdMutex<HashMap<String, VecDeque<String>>>>,
    navs: &Arc<StdMutex<HashMap<String, crate::script_runtime::NavBot>>>,
    statuses: &Arc<StdMutex<Vec<crate::SlotStatus>>>,
    world: &Option<Arc<nav_planner::world::NavWorld>>,
    tick: u64,
) {
    // 1. The snapshot decode seam (`play_slots.rs` publishes the frame here).
    host::publish_snapshot(snapshot, client, drain(false, true));
    // 2. The bot's own condition owner observes the frame and takes the
    //    dispatch checkpoint (the slot loop's order).
    let checkpoint = {
        let mut all = navs.lock().unwrap();
        let bot = all.get_mut("alice").expect("the gate bot");
        bot.condition
            .observe_frame(snapshot, &drain(false, true), false);
        bot.condition
            .dispatch_checkpoint(client)
            .expect("clean dispatch")
    };
    // 3. The script tick: `SlotScript::on_game_tick` dispatches on the edge
    //    and the idle card runs the real seat poll.
    crate::script_runtime::script_observe(
        client,
        "alice",
        true,
        true,
        tick,
        Some((0, 0, 0)),
        None,
        None,
        Some(&*snapshot),
        None,
        scripts,
        cheats,
        navs,
        world,
        false,
        false,
    );
    // 4. The dispatch observation follows the frame's dispatches.
    navs.lock()
        .unwrap()
        .get_mut("alice")
        .expect("the gate bot")
        .condition
        .observe_dispatch(client, checkpoint);
    // 5. The nav step.
    crate::script_runtime::step_nav_bot(
        client,
        "alice",
        snapshot.tile(),
        snapshot,
        navs,
        statuses,
        None,
        false,
        false,
        None,
        gate_reach,
    );
}

fn warmed_tick_frame_allocations_gate(revision: client::io::ClientRevision) {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut client = gate_client(revision);
    let mut snapshot = GameSnapshot::new();
    // The first decode rebuilds every armed family once (player, scene, loc,
    // stats); the measured frames leave every generation unchanged.
    host::publish_snapshot(&mut snapshot, &client, drain(false, true));
    // The same condition-completing fixtures the frame helper seeds.
    snapshot.seed_inventory(Vec::new(), 28);
    snapshot.seed_inventory_packet_generations(Some(0), None);

    let scripts: crate::script_runtime::ScriptWall = Default::default();
    let slot = crate::script_runtime::script_slot_or_insert(&scripts, "alice");
    slot.lock()
        .unwrap()
        .start_test_script(Box::new(GateIdleScript), None)
        .expect("the idle gate script starts");
    let work_scope = slot.lock().unwrap().work_scope();
    let mut bot = crate::script_runtime::NavBot {
        work_scope,
        ..crate::script_runtime::NavBot::default()
    };
    // The production session start arms the owner's selected tables, so the
    // observed frame completes the fact and stays clean.
    bot.condition.session_started(
        0,
        0,
        Some((script::combat::guard::shared_tables().unwrap(), 11)),
    );
    let navs = Arc::new(StdMutex::new(HashMap::from([("alice".to_string(), bot)])));
    let cheats = Arc::new(StdMutex::new(HashMap::new()));
    let statuses = Arc::new(StdMutex::new(vec![crate::SlotStatus {
        username: "alice".into(),
        ..Default::default()
    }]));
    let world: Option<Arc<nav_planner::world::NavWorld>> = None;

    // Warm: the first script tick, the first dispatch observation and any
    // one-shot logging settle before the counted frames.
    for tick in 1..=32u64 {
        gate_tick_frame(
            &mut client,
            &mut snapshot,
            &scripts,
            &cheats,
            &navs,
            &statuses,
            &world,
            tick,
        );
    }
    // 1,000 unchanged tick frames must allocate nothing.
    let measured = allocation_counter::measure(|| {
        for tick in 33..=1032u64 {
            gate_tick_frame(
                &mut client,
                &mut snapshot,
                &scripts,
                &cheats,
                &navs,
                &statuses,
                &world,
                tick,
            );
        }
    });
    assert_eq!(
        measured.bytes_total, 0,
        "1,000 unchanged tick frames allocate nothing: {measured:?}"
    );
    assert_eq!(
        measured.bytes_current, 0,
        "the warmed tick frames retain nothing: {measured:?}"
    );
}

#[test]
fn warmed_tick_frame_allocations_gate_r274() {
    warmed_tick_frame_allocations_gate(client::io::ClientRevision::R274);
}

#[test]
fn warmed_tick_frame_allocations_gate_r289() {
    warmed_tick_frame_allocations_gate(client::io::ClientRevision::R289);
}

#[test]
fn warmed_tick_frame_allocations_gate_r377() {
    warmed_tick_frame_allocations_gate(client::io::ClientRevision::R377);
}
