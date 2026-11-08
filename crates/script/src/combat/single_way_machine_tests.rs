//! Single-way combat against an NPC. While an NPC attacked the bot less than
//! 8 ticks ago, the content refuses an attack on any other NPC with "I'm
//! already under attack" (`player_combat.rs2:112-118`; `npc_combat.rs2:367-370`
//! sets the attacker on every NPC attack). These tests pin the machine's
//! reaction: attack the NPC that is hitting the bot when the request may fight
//! it, otherwise hold the attack for the 8-tick window, and end the fight after
//! three refusals with no confirmed attack in between.
use super::*;
use api::snapshot::ChatLineView;

fn say(scene: &mut Scene, sequence: i32, text: &str) {
    scene.snapshot.seed_chat_lines(vec![ChatLineView {
        type_: 0,
        username: None,
        text: text.into(),
        sequence,
    }]);
}

fn attack_on(effect: Option<HostEffect>, index: i32) {
    assert!(
        matches!(&effect, Some(HostEffect::Interaction(InteractReq::Npc { action, index: Some(row), .. }))
            if action == "Attack" && *row == index),
        "expected an Attack on npc {index}, got {:?}",
        effect.map(|_| "another effect")
    );
}

fn has_attack(batch: &DrainedBatch) -> bool {
    (0..batch.len()).any(|index| {
        matches!(
            batch.get(index),
            Some(HostEffect::Interaction(InteractReq::Npc { action, .. })) if action == "Attack"
        )
    })
}

/// Poll once and take the batch; a fight that ended on this poll fails the
/// test with its end and tick instead of a bare "not pending".
fn pending_unless_ended(harness: &mut Harness, scene: &Scene, tick: u64) -> DrainedBatch {
    match harness.poll(&scene.snapshot, tick) {
        Poll::Pending => harness.drain(None),
        other => panic!(
            "the fight ended at tick {tick} after {} counted windows: {other:?}",
            harness.machine.single_way_lines
        ),
    }
}

fn faces_the_bot(row: &mut NpcView) {
    row.target = Some(ActorTargetView {
        kind: ActorKind::Player,
        index: 1,
    });
    row.in_combat = true;
}

/// Two imps: index 7 beside the bot and index 8 three tiles away, so the
/// nearest pick engages 7 first.
fn two_imps() -> Scene {
    let mut scene = Scene::new("imp");
    let mut far = scene.npcs[0].clone();
    far.index = 8;
    far.tile = tile(2603, 3200);
    far.network = far.tile;
    far.distance = 3;
    scene.npcs.push(far);
    scene.refresh();
    scene
}

const REFUSAL: &str = "I'm already under attack!";

#[test]
fn a_refusal_retargets_the_npc_that_is_hitting_the_bot() {
    if !api::test_support::game_data_available() {
        return;
    }
    // Revision 289 says it with "!", the revision-377 check and
    // `player_in_combat_check2` without a mark.
    for (sequence, text) in SINGLE_WAY_NPC_LINES.iter().enumerate() {
        let mut scene = two_imps();
        let mut harness = Harness::new(&scene, scene.request());
        attack_on(harness.pending(&scene, 1), 7);
        faces_the_bot(&mut scene.npcs[1]);
        scene.refresh();
        say(&mut scene, sequence as i32 + 1, text);
        attack_on(harness.pending(&scene, 2), 8);
        assert_eq!(
            harness.machine.engaged,
            Some(ActorRef {
                kind: ActorKind::Npc,
                index: 8
            }),
            "{text:?}"
        );
        assert!(harness.machine.end.is_none(), "{text:?}");
    }
}

/// The NPC in combat with the bot is the one the content allows, so it comes
/// before a nearer NPC that only faces the bot; a later refusal moves on to
/// the other attacker instead of repeating the refused one.
#[test]
fn a_refusal_prefers_the_npc_in_combat_and_then_moves_to_the_other_attacker() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = two_imps();
    let mut chaser = scene.npcs[0].clone();
    chaser.index = 9;
    chaser.tile = tile(2602, 3200);
    chaser.network = chaser.tile;
    chaser.distance = 2;
    scene.npcs.push(chaser);
    let mut harness = Harness::new(&scene, scene.request());
    attack_on(harness.pending(&scene, 1), 7);
    // 9 is nearer but only faces the bot; 8 is farther and fighting it.
    faces_the_bot(&mut scene.npcs[1]);
    scene.npcs[2].target = scene.npcs[1].target;
    scene.refresh();
    say(&mut scene, 1, REFUSAL);
    attack_on(harness.pending(&scene, 2), 8);
    say(&mut scene, 2, REFUSAL);
    attack_on(harness.pending(&scene, 3), 9);
    // The second refusal falls in the window the first one opened: it
    // retargets but does not count.
    assert_eq!(harness.machine.single_way_lines, 1);
}

/// The stand gates starting an engagement; an NPC already hitting the bot is
/// reached without it.
#[test]
fn a_refusal_retargets_even_when_the_bot_is_off_its_stand() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = two_imps();
    let mut request = scene.request();
    request.stand = Some(scene.local.player.network);
    let mut harness = Harness::new(&scene, request);
    attack_on(harness.pending(&scene, 1), 7);
    scene.local.player.network = tile(2604, 3200);
    scene.local.player.actor.tile = tile(2604, 3200);
    faces_the_bot(&mut scene.npcs[1]);
    scene.refresh();
    say(&mut scene, 1, REFUSAL);
    attack_on(harness.pending(&scene, 2), 8);
}

#[test]
fn a_refusal_with_no_attacker_holds_the_attack_for_the_eight_tick_window() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = Scene::new("imp");
    let mut harness = Harness::new(&scene, scene.request());
    attack_on(harness.pending(&scene, 1), 7);
    say(&mut scene, 1, REFUSAL);
    // The line is read at tick 2: no Attack row on ticks 2..=9.
    for tick in 2..=9 {
        let batch = harness.pending_batch(&scene, tick);
        assert!(
            !has_attack(&batch),
            "an attack inside the hold at tick {tick}"
        );
    }
    attack_on(harness.pending(&scene, 10), 7);
    assert!(harness.machine.end.is_none(), "the engagement is kept");
}

#[test]
fn an_attacker_the_request_may_not_fight_is_not_retargeted() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = Scene::new("imp");
    let mut other = scene.npcs[0].clone();
    other.index = 8;
    other.r#type = Some(scene.data.npc_by_config("cow").unwrap().id as usize);
    other.tile = tile(2599, 3200);
    other.network = other.tile;
    faces_the_bot(&mut other);
    scene.npcs.push(other);
    scene.refresh();
    let mut harness = Harness::new(&scene, scene.request());
    attack_on(harness.pending(&scene, 1), 7);
    say(&mut scene, 1, REFUSAL);
    for tick in 2..=9 {
        let batch = harness.pending_batch(&scene, tick);
        assert!(
            !has_attack(&batch),
            "an attack inside the hold at tick {tick}"
        );
    }
    assert_eq!(
        harness.machine.engaged,
        Some(ActorRef {
            kind: ActorKind::Npc,
            index: 7
        })
    );
}

#[test]
fn the_third_window_without_a_confirmed_attack_ends_the_fight() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = Scene::new("imp");
    let mut harness = Harness::new(&scene, scene.request());
    attack_on(harness.pending(&scene, 1), 7);
    // One refusal per hold window: ticks 2, 10 and 18 each open a window, and
    // the third one closes at tick 26.
    for (tick, sequence) in [(2, 1), (10, 2)] {
        say(&mut scene, sequence, REFUSAL);
        pending_unless_ended(&mut harness, &scene, tick);
        assert!(harness.machine.end.is_none(), "ended early at tick {tick}");
    }
    say(&mut scene, 3, REFUSAL);
    for tick in 18..26 {
        pending_unless_ended(&mut harness, &scene, tick);
        assert!(harness.machine.end.is_none(), "ended early at tick {tick}");
    }
    let report = match harness.poll(&scene.snapshot, 26) {
        Poll::Ready(Ok(report)) => report,
        other => panic!("the third window must end the fight, got {other:?}"),
    };
    assert_eq!(
        report.end,
        CombatEnd::Aborted(AbortReason::SingleWayBlocked)
    );
    assert_eq!(
        report.engaged_npc_type,
        scene.npcs[0].r#type.unwrap() as i32
    );
}

/// The live state: a refused click leaves the player's target on the engaged
/// NPC, and no hit lands. The target alone confirms nothing, so the third
/// refusal still ends the fight.
#[test]
fn the_third_window_ends_the_fight_while_the_player_target_stays_the_engaged_npc() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = Scene::new("imp");
    scene.local.player.actor.target = Some(ActorTargetView {
        kind: ActorKind::Npc,
        index: 7,
    });
    scene.refresh();
    let mut harness = Harness::new(&scene, scene.request());
    attack_on(harness.pending(&scene, 1), 7);
    for (tick, sequence) in [(2, 1), (10, 2)] {
        say(&mut scene, sequence, REFUSAL);
        pending_unless_ended(&mut harness, &scene, tick);
        assert!(harness.machine.end.is_none(), "ended early at tick {tick}");
        assert_eq!(harness.machine.single_way_lines, sequence as u8);
    }
    say(&mut scene, 3, REFUSAL);
    for tick in 18..26 {
        pending_unless_ended(&mut harness, &scene, tick);
        assert!(harness.machine.end.is_none(), "ended early at tick {tick}");
    }
    let report = match harness.poll(&scene.snapshot, 26) {
        Poll::Ready(Ok(report)) => report,
        other => panic!("the third window must end the fight, got {other:?}"),
    };
    assert_eq!(
        report.end,
        CombatEnd::Aborted(AbortReason::SingleWayBlocked)
    );
}

/// The same refusals with a confirmed attack between them: the count restarts
/// and the fight goes on. The first hit opens the NPC's health-bar window; the
/// second only lowers its health inside an open window.
#[test]
fn a_confirmed_attack_between_refusals_resets_the_count_and_the_fight_continues() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = Scene::new("imp");
    scene.local.player.actor.target = Some(ActorTargetView {
        kind: ActorKind::Npc,
        index: 7,
    });
    scene.refresh();
    let mut harness = Harness::new(&scene, scene.request());
    attack_on(harness.pending(&scene, 1), 7);
    say(&mut scene, 1, REFUSAL);
    harness.pending_batch(&scene, 2);
    assert_eq!(harness.machine.single_way_lines, 1);
    // The window opens and the first hit lands: the health bar opens.
    scene.npcs[0].in_combat = true;
    scene.npcs[0].health = 24;
    scene.refresh();
    harness.pending_batch(&scene, 3);
    assert_eq!(harness.machine.single_way_lines, 0);
    say(&mut scene, 2, REFUSAL);
    harness.pending_batch(&scene, 10);
    say(&mut scene, 3, REFUSAL);
    harness.pending_batch(&scene, 18);
    assert_eq!(harness.machine.single_way_lines, 2);
    assert!(harness.machine.end.is_none());
    // A later hit inside the open window lowers the health: the count restarts
    // again, so a fourth and fifth refusal do not end the fight.
    scene.npcs[0].health = 20;
    scene.refresh();
    harness.pending_batch(&scene, 19);
    assert_eq!(harness.machine.single_way_lines, 0);
    say(&mut scene, 4, REFUSAL);
    harness.pending_batch(&scene, 26);
    say(&mut scene, 5, REFUSAL);
    harness.pending_batch(&scene, 34);
    assert_eq!(harness.machine.single_way_lines, 2);
    assert!(harness.machine.end.is_none());
}

/// An open health-bar window with an unchanged health is not a new hit, and
/// neither is a hit seen while the bot targets another NPC.
#[test]
fn an_old_window_and_a_hit_on_a_target_the_bot_does_not_attack_confirm_nothing() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = Scene::new("imp");
    let mut harness = Harness::new(&scene, scene.request());
    attack_on(harness.pending(&scene, 1), 7);
    // The imp's health bar opens while the bot does not target it (another
    // hit, not ours); then the bot targets it and the first refusal arrives.
    scene.npcs[0].in_combat = true;
    scene.refresh();
    harness.pending_batch(&scene, 2);
    scene.local.player.actor.target = Some(ActorTargetView {
        kind: ActorKind::Npc,
        index: 7,
    });
    scene.refresh();
    say(&mut scene, 1, REFUSAL);
    harness.pending_batch(&scene, 3);
    harness.pending_batch(&scene, 4);
    assert_eq!(harness.machine.single_way_lines, 1);
    // Health drops, but the bot's target is another NPC.
    scene.local.player.actor.target = Some(ActorTargetView {
        kind: ActorKind::Npc,
        index: 8,
    });
    scene.npcs[0].health = 20;
    scene.refresh();
    harness.pending_batch(&scene, 5);
    assert_eq!(harness.machine.single_way_lines, 1);
}

#[test]
fn refusals_inside_a_hold_belong_to_the_attack_the_hold_answers() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = Scene::new("imp");
    let mut harness = Harness::new(&scene, scene.request());
    attack_on(harness.pending(&scene, 1), 7);
    // The engine can refuse an attack on several ticks of one window.
    for (tick, sequence) in (2..=9).zip(1..) {
        say(&mut scene, sequence, REFUSAL);
        harness.pending_batch(&scene, tick);
        assert!(harness.machine.end.is_none(), "ended at tick {tick}");
    }
    assert_eq!(harness.machine.single_way_lines, 1);
}

#[test]
fn a_hit_on_the_engaged_npc_ends_the_run_of_refusals() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = Scene::new("imp");
    let mut harness = Harness::new(&scene, scene.request());
    attack_on(harness.pending(&scene, 1), 7);
    say(&mut scene, 1, REFUSAL);
    harness.pending_batch(&scene, 2);
    assert_eq!(harness.machine.single_way_lines, 1);
    // The window lapsed and the attack landed: the player's target is the imp
    // and the imp's health bar opened.
    scene.install();
    scene.npcs[0].in_combat = true;
    scene.npcs[0].health = 26;
    scene.refresh();
    harness.pending_batch(&scene, 3);
    assert_eq!(harness.machine.single_way_lines, 0);
}

#[test]
fn a_refusal_line_is_matched_whole_and_the_player_line_is_not_an_npc_line() {
    if !api::test_support::game_data_available() {
        return;
    }
    for (sequence, text) in [
        // The PvP line has a period and belongs to a player fight.
        "I'm already under attack.",
        "I'm already under attack by a goblin",
        "Someone else is fighting that.",
    ]
    .into_iter()
    .enumerate()
    {
        let mut scene = two_imps();
        let mut harness = Harness::new(&scene, scene.request());
        attack_on(harness.pending(&scene, 1), 7);
        faces_the_bot(&mut scene.npcs[1]);
        scene.refresh();
        say(&mut scene, sequence as i32 + 1, text);
        harness.pending_batch(&scene, 2);
        assert_eq!(
            harness.machine.engaged,
            Some(ActorRef {
                kind: ActorKind::Npc,
                index: 7
            }),
            "{text:?} must not retarget"
        );
        assert_eq!(harness.machine.single_way_lines, 0, "{text:?}");
    }
}

#[test]
fn a_refusal_read_after_the_fight_ended_changes_nothing() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = two_imps();
    let mut harness = Harness::new(&scene, scene.request());
    attack_on(harness.pending(&scene, 1), 7);
    harness.machine.finish(CombatEnd::Killed, 1);
    faces_the_bot(&mut scene.npcs[1]);
    scene.refresh();
    say(&mut scene, 1, REFUSAL);
    // The wind-down poll may complete the run; the refusal is read either way.
    let _ = harness.poll(&scene.snapshot, 2);
    assert_eq!(harness.machine.single_way_lines, 0);
    assert_eq!(
        harness.machine.engaged,
        Some(ActorRef {
            kind: ActorKind::Npc,
            index: 7
        })
    );
}

/// Four knights hit the bot, as in live run 2 of round 2: the engine allows
/// only the one that attacked last, and that one changes from tick to tick.
fn four_attackers() -> Scene {
    let mut scene = two_imps();
    for (index, x) in [(9, 2602), (10, 2604)] {
        let mut extra = scene.npcs[0].clone();
        extra.index = index;
        extra.tile = tile(x, 3200);
        extra.network = extra.tile;
        extra.distance = (x - 2600).abs();
        scene.npcs.push(extra);
    }
    for row in &mut scene.npcs {
        faces_the_bot(row);
        row.health = 30;
        row.total_health = 30;
    }
    scene.refresh();
    scene
}

/// Refusals on consecutive ticks inside one engine window count once, and the
/// machine still retargets on each of them. Before the window rule the third
/// refusal (tick 4) ended the fight.
#[test]
fn refusals_on_consecutive_ticks_inside_one_window_count_once() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = four_attackers();
    let mut harness = Harness::new(&scene, scene.request());
    attack_on(harness.pending(&scene, 1), 7);
    // The first refusal is read at tick 2; its window runs through tick 9.
    for (tick, sequence) in (2..=9).zip(1..) {
        say(&mut scene, sequence, REFUSAL);
        let batch = pending_unless_ended(&mut harness, &scene, tick);
        assert!(
            harness.machine.end.is_none(),
            "the fight ended at tick {tick} after {} counted refusals",
            harness.machine.single_way_lines
        );
        assert!(has_attack(&batch), "no retarget attack at tick {tick}");
    }
    assert_eq!(
        harness.machine.single_way_lines, 1,
        "eight refusals in one window count once"
    );
}

/// Three windows in a row with a refusal and no landed hit end the fight, about
/// 24 ticks after the first refusal; refusals inside a window retarget and do
/// not count.
#[test]
fn three_windows_with_refusals_and_no_hit_end_as_single_way_blocked() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = four_attackers();
    let mut harness = Harness::new(&scene, scene.request());
    attack_on(harness.pending(&scene, 1), 7);
    let mut sequence = 0;
    // Windows open at ticks 2, 10 and 18 (each closes 8 ticks later); a second
    // refusal follows one tick after each opening.
    for tick in 2..26 {
        if matches!(tick, 2 | 3 | 10 | 11 | 18 | 19) {
            sequence += 1;
            say(&mut scene, sequence, REFUSAL);
        }
        pending_unless_ended(&mut harness, &scene, tick);
        assert!(
            harness.machine.end.is_none(),
            "the fight ended at tick {tick}, before the third window closed"
        );
    }
    assert_eq!(harness.machine.single_way_lines, 3);
    let report = match harness.poll(&scene.snapshot, 26) {
        Poll::Ready(Ok(report)) => report,
        other => panic!("the third window must end the fight at tick 26, got {other:?}"),
    };
    assert_eq!(
        report.end,
        CombatEnd::Aborted(AbortReason::SingleWayBlocked)
    );
    assert_eq!(
        report.engaged_npc_type,
        scene.npcs[0].r#type.unwrap() as i32
    );
}

/// A landed hit on the engaged NPC resets the count of windows: two windows,
/// a hit, then two more windows do not end the fight.
#[test]
fn a_landed_hit_resets_the_window_count() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = four_attackers();
    let mut harness = Harness::new(&scene, scene.request());
    attack_on(harness.pending(&scene, 1), 7);
    let mut sequence = 0;
    for tick in 2..=11 {
        if matches!(tick, 2 | 10) {
            sequence += 1;
            say(&mut scene, sequence, REFUSAL);
        }
        pending_unless_ended(&mut harness, &scene, tick);
        assert!(harness.machine.end.is_none(), "ended at tick {tick}");
    }
    assert_eq!(harness.machine.single_way_lines, 2);
    // The bot targets the engaged NPC and its health drops: a hit landed.
    let engaged = usize::from(harness.machine.engaged.unwrap().index);
    scene.local.player.actor.target = Some(ActorTargetView {
        kind: ActorKind::Npc,
        index: engaged,
    });
    scene
        .npcs
        .iter_mut()
        .find(|row| row.index == engaged)
        .unwrap()
        .health = 20;
    scene.refresh();
    harness.pending_batch(&scene, 12);
    assert_eq!(harness.machine.single_way_lines, 0);
    // Two more windows, with no hit, are one window short of the end.
    for tick in 13..=41 {
        if matches!(tick, 18 | 26) {
            sequence += 1;
            say(&mut scene, sequence, REFUSAL);
        }
        pending_unless_ended(&mut harness, &scene, tick);
        assert!(
            harness.machine.end.is_none(),
            "ended at tick {tick} with {} windows since the hit",
            harness.machine.single_way_lines
        );
    }
    assert_eq!(harness.machine.single_way_lines, 2);
}

/// In single-way combat the first pick takes the NPC that is in combat with
/// the bot over a nearer idle one: the engine would refuse the nearer one.
#[test]
fn the_first_pick_prefers_an_engaged_attacker_over_a_nearer_idle_npc() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = two_imps();
    // 8 is three tiles away and fights the bot; 7 is beside the bot and idle.
    faces_the_bot(&mut scene.npcs[1]);
    scene.refresh();
    let mut harness = Harness::new(&scene, scene.request());
    attack_on(harness.pending(&scene, 1), 8);
}

/// A multiway map has no such rule, so the nearest pick stays the nearest.
#[test]
fn the_first_pick_stays_nearest_on_a_multiway_map() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = two_imps();
    faces_the_bot(&mut scene.npcs[1]);
    scene.refresh();
    scene.snapshot.seed_world(WorldStateView {
        map_base_x: 2560,
        map_base_z: 3160,
        members: true,
        multi_combat: true,
        ..WorldStateView::default()
    });
    let mut harness = Harness::new(&scene, scene.request());
    attack_on(harness.pending(&scene, 1), 7);
}

/// An NPC that only faces the bot, with no health bar open, has not attacked
/// it; it does not outrank the nearest NPC.
#[test]
fn the_first_pick_ignores_an_npc_that_faces_the_bot_but_is_not_in_combat() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = two_imps();
    faces_the_bot(&mut scene.npcs[1]);
    scene.npcs[1].in_combat = false;
    scene.refresh();
    let mut harness = Harness::new(&scene, scene.request());
    attack_on(harness.pending(&scene, 1), 7);
}
