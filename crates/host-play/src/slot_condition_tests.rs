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

fn chat_pair(
    snapshot: &mut GameSnapshot,
    older: (i32, &str),
    newer: (i32, &str),
) {
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
    let input = crate::admission::capture(&snapshot, true, Some(&stale), false);
    assert!(matches!(input.poison, PoisonState::Unknown { .. }));
}

#[test]
fn slot_condition_consumes_chat_at_edges_only() {
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
    // Synthetic replay of RS5 R5-01: PLAYER_INFO t (hit), NPC_INFO t
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
fn review_probe_same_edge_fruit_then_newer_poison_line_is_onset() {
    let (mut owner, mut snapshot) = poisoned_owner_with_fruit();
    snapshot.seed_tick(2);
    inventory(&mut snapshot, 2, &[]);
    chat_pair(
        &mut snapshot,
        (1, FRUIT_LINE),
        (2, POISON_LINE),
    );
    owner.observe_frame(&snapshot, &drain(true, true), false);

    snapshot.seed_tick(3);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert!(matches!(
        owner.current().unwrap().poison,
        PoisonState::Onset { .. }
    ));
}

#[test]
fn review_probe_split_fruit_chat_then_poison_line_with_count_loss_is_onset() {
    let (mut owner, mut snapshot) = poisoned_owner_with_fruit();
    snapshot.seed_tick(2);
    chat(&mut snapshot, 1, FRUIT_LINE);
    owner.observe_frame(&snapshot, &drain(true, true), false);

    snapshot.seed_tick(3);
    inventory(&mut snapshot, 2, &[]);
    chat_pair(
        &mut snapshot,
        (1, FRUIT_LINE),
        (2, POISON_LINE),
    );
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert!(matches!(
        owner.current().unwrap().poison,
        PoisonState::Onset { .. }
    ));
}

#[test]
fn newer_poison_line_clears_waiting_fruit_chat() {
    let (mut owner, mut snapshot) = poisoned_owner_with_fruit();
    snapshot.seed_tick(2);
    chat(&mut snapshot, 1, FRUIT_LINE);
    owner.observe_frame(&snapshot, &drain(true, true), false);

    snapshot.seed_tick(3);
    chat_pair(
        &mut snapshot,
        (1, FRUIT_LINE),
        (2, POISON_LINE),
    );
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert!(matches!(owner.pending_fruit, PendingFruit::None));
    assert!(matches!(
        owner.current().unwrap().poison,
        PoisonState::Onset { .. }
    ));
}

#[test]
fn accepted_click_settle_covers_last_login_mark_tick_and_lag() {
    // Login is session tick 1. The timer fires 30 ticks later, damage is
    // queued for the following tick, and LAG retains two observation ticks.
    let mut owner = SlotCondition::default();
    owner.session_started(
        1,
        0,
        Some((script::combat::guard::shared_tables().unwrap(), 11)),
    );
    owner.observe_frame(&frame(1), &drain(true, true), false);
    owner.accepted_click();
    for tick in 2..=34 {
        owner.observe_frame(&frame(tick), &drain(true, true), false);
        assert_eq!(
            owner.current().unwrap().poison,
            PoisonState::Unknown { since: 1 },
            "tick {tick} must still cover the login fire's possible mark"
        );
    }
    owner.observe_frame(&frame(35), &drain(true, true), false);
    assert_eq!(owner.current().unwrap().poison, PoisonState::Clear);
}

#[test]
fn login_poison_mark_at_last_settle_tick_prevents_clear() {
    let mut owner = SlotCondition::default();
    owner.session_started(
        1,
        0,
        Some((script::combat::guard::shared_tables().unwrap(), 11)),
    );
    owner.observe_frame(&frame(1), &drain(true, true), false);
    owner.accepted_click();
    for tick in 2..34 {
        owner.observe_frame(&frame(tick), &drain(true, true), false);
    }
    let mut snapshot = frame(34);
    hit(&mut snapshot, 2, 6, 1);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    assert_eq!(
        owner.current().unwrap().poison,
        PoisonState::Poisoned {
            per_tick: 6,
            last_mark: 34,
            hit_since_mark: false
        }
    );
}

#[test]
fn interaction_at_clean_tick_30_restarts_login_settle() {
    let mut owner = owner();
    owner.accepted_click();
    for tick in 1..=30 {
        owner.observe_frame(&frame(tick), &drain(true, true), false);
    }
    owner.accepted_click();
    for tick in 31..=63 {
        owner.observe_frame(&frame(tick), &drain(true, true), false);
        assert_eq!(
            owner.current().unwrap().poison,
            PoisonState::Unknown { since: 0 },
            "tick {tick} must cover the delayed login fire"
        );
    }
    owner.observe_frame(&frame(64), &drain(true, true), false);
    assert_eq!(owner.current().unwrap().poison, PoisonState::Clear);
}

#[test]
fn delayed_login_mark_never_follows_false_clear() {
    let mut owner = owner();
    owner.accepted_click();
    for tick in 1..=30 {
        owner.observe_frame(&frame(tick), &drain(true, true), false);
    }
    owner.accepted_click();
    // A content delay covers the original timer fire. Its mark arrives
    // after the old window would clear, but inside the restarted window.
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
    // Cover operations, item/widget uses, inventory controls and interface
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
        for tick in 31..=63 {
            owner.observe_frame(&frame(tick), &drain(true, true), false);
            assert_eq!(
                owner.current().unwrap().poison,
                PoisonState::Unknown { since: 0 },
                "packet {packet:?} at tick 30 must extend the window through {tick}"
            );
        }
        owner.observe_frame(&frame(64), &drain(true, true), false);
        assert_eq!(owner.current().unwrap().poison, PoisonState::Clear);
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
fn walk_only_packets_finish_login_settle_without_restarting() {
    for packet in [
        ClientProt::MOVE_GAMECLICK,
        ClientProt::MOVE_MINIMAPCLICK,
        ClientProt::MOVE_OPCLICK,
    ] {
        let mut client = outgoing_client(client::io::ClientRevision::R289);
        let mut owner = owner();
        for tick in 1..=33 {
            let checkpoint = owner.dispatch_checkpoint(&client).unwrap();
            queue_packet(&mut client, packet);
            owner.observe_dispatch(&client, checkpoint);
            owner.observe_frame(&frame(tick), &drain(true, true), false);
            assert_eq!(
                owner.current().unwrap().poison,
                PoisonState::Unknown { since: 0 }
            );
        }
        owner.observe_frame(&frame(34), &drain(true, true), false);
        assert_eq!(
            owner.current().unwrap().poison,
            PoisonState::Clear,
            "movement {packet:?} must not postpone the clean run"
        );
    }
}

#[test]
fn empty_dispatch_and_telemetry_do_not_restart_login_settle() {
    let mut client = outgoing_client(client::io::ClientRevision::R289);
    let mut owner = owner();
    owner.accepted_click();
    for tick in 1..=34 {
        let checkpoint = owner.dispatch_checkpoint(&client).unwrap();
        if tick % 2 == 0 {
            queue_packet(&mut client, ClientProt::EVENT_CAMERA_POSITION);
            queue_packet(&mut client, ClientProt::NO_TIMEOUT);
        }
        owner.observe_dispatch(&client, checkpoint);
        owner.observe_frame(&frame(tick), &drain(true, true), false);
    }
    assert_eq!(owner.current().unwrap().poison, PoisonState::Clear);
}

#[test]
fn repeated_interactions_keep_login_poison_unknown() {
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

#[test]
fn condition_dispatch_checkpoint_and_trace_allocate_nothing() {
    let client = outgoing_client(client::io::ClientRevision::R289);
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
fn guardian_hold_on_wake_resets_clean_window_and_click() {
    let mut owner = owner();
    owner.accepted_click();
    for tick in 1..=14 {
        owner.observe_frame(&frame(tick), &drain(true, true), false);
    }
    owner.observe_frame(&frame(14), &drain(false, true), true);
    for tick in 15..=48 {
        owner.observe_frame(&frame(tick), &drain(true, true), false);
    }
    assert!(matches!(
        owner.current().unwrap().poison,
        PoisonState::Unknown { .. }
    ));
    owner.accepted_click();
    owner.observe_frame(&frame(49), &drain(true, true), false);
    assert!(matches!(
        owner.current().unwrap().poison,
        PoisonState::Unknown { .. }
    ));
    // The new interaction also restarts the run after the hold.
    for tick in 50..=81 {
        owner.observe_frame(&frame(tick), &drain(true, true), false);
        assert!(matches!(
            owner.current().unwrap().poison,
            PoisonState::Unknown { .. }
        ));
    }
    owner.observe_frame(&frame(82), &drain(true, true), false);
    assert_eq!(owner.current().unwrap().poison, PoisonState::Clear);
}

#[test]
fn admission_capture_reads_slot_condition() {
    let mut owner = owner();
    let mut snapshot = frame(1);
    hit(&mut snapshot, 2, 6, 1);
    owner.observe_frame(&snapshot, &drain(true, true), false);
    let input = crate::admission::capture(&snapshot, false, owner.current(), false);
    assert_eq!(input.poison, owner.current().unwrap().poison);
    owner.complete = false;
    let input = crate::admission::capture(&snapshot, false, owner.current(), false);
    assert!(matches!(input.poison, PoisonState::Unknown { .. }));
}

#[test]
fn slot_condition_observe_frame_allocates_nothing() {
    let mut owner = owner();
    let snapshot = frame(0);
    let drain = drain(false, true);
    let measured = allocation_counter::measure(|| {
        for _ in 0..1000 {
            owner.observe_frame(&snapshot, &drain, false);
        }
    });
    assert_eq!(measured.bytes_total, 0);
}

#[test]
fn slot_condition_line_frame_allocates_nothing() {
    let mut owner = owner();
    let mut line = frame(1);
    chat(&mut line, 1, POISON_LINE);
    hit(&mut line, 1, 1, 1);
    line.seed_npcs(vec![npc(108, 1)]);
    let mut drink = frame(2);
    chat(&mut drink, 2, DRINK_LINE);
    let measured = allocation_counter::measure(|| {
        owner.observe_frame(&line, &drain(true, false), false);
        owner.observe_frame(&line, &drain(false, true), false);
        owner.observe_frame(&drink, &drain(true, true), false);
    });
    assert_eq!(measured.bytes_total, 0);
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
        (104, 28, 8, 120, 120),
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
