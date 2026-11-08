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
    assert_eq!(harness.machine.single_way_lines, 2);
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
fn the_third_refusal_without_a_confirmed_attack_ends_the_fight() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = Scene::new("imp");
    let mut harness = Harness::new(&scene, scene.request());
    attack_on(harness.pending(&scene, 1), 7);
    // One refusal per hold window: ticks 2 and 10 hold, tick 18 ends.
    for (tick, sequence) in [(2, 1), (10, 2)] {
        say(&mut scene, sequence, REFUSAL);
        harness.pending_batch(&scene, tick);
        assert!(harness.machine.end.is_none(), "ended early at tick {tick}");
    }
    say(&mut scene, 3, REFUSAL);
    let report = match harness.poll(&scene.snapshot, 18) {
        Poll::Ready(Ok(report)) => report,
        other => panic!("the third refusal must end the fight, got {other:?}"),
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
fn the_third_refusal_ends_the_fight_while_the_player_target_stays_the_engaged_npc() {
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
        harness.pending_batch(&scene, tick);
        assert!(harness.machine.end.is_none(), "ended early at tick {tick}");
        assert_eq!(harness.machine.single_way_lines, sequence as u8);
    }
    say(&mut scene, 3, REFUSAL);
    let report = match harness.poll(&scene.snapshot, 18) {
        Poll::Ready(Ok(report)) => report,
        other => panic!("the third refusal must end the fight, got {other:?}"),
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
