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

fn assert_onset(owner: &SlotCondition, q: u8, latched: bool, refreshed: bool) {
    assert_eq!(
        owner.current().unwrap().poison,
        PoisonState::Onset {
            since: 1,
            q,
            latched,
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
    owner.session_ended();
    assert!(owner.current().is_none());
    owner.session_started(1, None);
    assert!(owner.current().is_none());
    assert_eq!(owner.published().session, SessionMark::Boundary);
    owner.observe_frame(&frame(1), &drain(false, true), false);
    assert!(matches!(
        owner.current().unwrap().poison,
        PoisonState::Unknown { .. }
    ));
    assert_eq!(owner.unknown_pulse(), u8::MAX);
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
            PoisonState::Onset {
                q: 11,
                latched: true,
                ..
            }
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
fn accepted_click_settle_needs_thirty_consecutive_clean_edges() {
    let mut owner = owner();
    owner.accepted_click();
    for tick in 1..30 {
        owner.observe_frame(&frame(tick), &drain(true, true), false);
        assert!(matches!(
            owner.current().unwrap().poison,
            PoisonState::Unknown { .. }
        ));
    }
    owner.observe_frame(&frame(30), &drain(true, true), false);
    assert_eq!(owner.current().unwrap().poison, PoisonState::Clear);
}

#[test]
fn guardian_hold_on_wake_resets_clean_window_and_click() {
    let mut owner = owner();
    owner.accepted_click();
    for tick in 1..=14 {
        owner.observe_frame(&frame(tick), &drain(true, true), false);
    }
    owner.observe_frame(&frame(14), &drain(false, true), true);
    for tick in 15..=45 {
        owner.observe_frame(&frame(tick), &drain(true, true), false);
    }
    assert!(matches!(
        owner.current().unwrap().poison,
        PoisonState::Unknown { .. }
    ));
    owner.accepted_click();
    owner.observe_frame(&frame(46), &drain(true, true), false);
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
        "SlotCondition is 104 B: the inline fact, reducer, cursors and latch; no inventory witness or allocation"
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
