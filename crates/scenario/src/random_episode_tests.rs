use super::*;

const FISHING: i32 = 10;
const HARPOON: i32 = 311;
const KEBAB: i32 = 1971;
const TROLL: RandomEventCell = RandomEventCell {
    event: "river troll",
    trigger: Trigger::Skill(2),
    oracle: Oracle::Evade { npc: "River troll" },
    skill: FISHING,
};
const SWARM: RandomEventCell = RandomEventCell {
    event: "swarm",
    trigger: Trigger::General(1),
    oracle: Oracle::Evade { npc: "Swarm" },
    skill: FISHING,
};
const DWARF: RandomEventCell = RandomEventCell {
    event: "drunken dwarf",
    trigger: Trigger::General(3),
    oracle: Oracle::Dialog {
        npc: "Drunken dwarf",
        reward: Reward::AnyOf(&[KEBAB]),
    },
    skill: FISHING,
};
const MAZE: RandomEventCell = RandomEventCell {
    event: "maze",
    trigger: Trigger::General(8),
    oracle: Oracle::Trap(TrapKind::Maze),
    skill: FISHING,
};
const BIG_FISH: RandomEventCell = RandomEventCell {
    event: "big fish",
    trigger: Trigger::Skill(1),
    oracle: Oracle::LostGear { gear_id: HARPOON },
    skill: FISHING,
};
const STRANGE_BOX: RandomEventCell = RandomEventCell {
    event: "strange box",
    trigger: Trigger::Seeded,
    oracle: Oracle::StrangeBoxHandoff,
    skill: FISHING,
};

/// A maze square tile (`api::random::trapped_area`).
fn maze_tile() -> (i32, i32, i32) {
    let tile = (2911, 4551, 0);
    assert_eq!(
        trapped_area(tile.0, tile.1, tile.2),
        Some(RandomKind::Maze),
        "fixture tile must be on the maze square"
    );
    tile
}

fn frame(tick: u32, x: i32, z: i32, hold: bool) -> Frame {
    Frame {
        tick,
        ready: true,
        tile: Some((x, z, 0)),
        fight_outcome: None,
        no_progress_ticks: None,
        hold,
        hp: Some(10),
        skill_xp: Some(1000),
        strength_xp: Some(0),
        chat: vec![(1, "Welcome".into())],
        npcs: Vec::new(),
        held: vec![(HARPOON, 1)],
        fruit: false,
        modal: false,
        targeting_npcs: Vec::new(),
        local_target: None,
    }
}

fn box_frame(tick: u32, hold: bool, count: i32) -> Frame {
    let mut f = frame(tick, 100, 100, hold);
    f.held.clear();
    if count > 0 {
        f.held.push((STRANGE_BOX_ID, count));
    }
    f
}

fn npc(x: i32, z: i32, targets_local: bool) -> FrameNpc {
    FrameNpc {
        index: 7,
        type_id: Some(391),
        tile: (x, z),
        targets_local,
        health: (30, 30),
    }
}

fn chat(mut f: Frame, seq: i32, text: &str) -> Frame {
    f.chat.insert(0, (seq, text.into()));
    f
}

/// Baseline, then the trigger send; returns the episode ready for onset.
fn armed(cell: RandomEventCell) -> RandomEventEpisode {
    let mut ep = RandomEventEpisode::new(cell).with_maxrange(15);
    assert_eq!(ep.observe(frame(1, 100, 100, false)), Verdict::Pending);
    let send = ep.next_send(&frame(2, 100, 100, false));
    assert_eq!(send, Some(cell.trigger.command()));
    assert_eq!(ep.observe(frame(2, 100, 100, false)), Verdict::Pending);
    ep
}

fn acked(cell: RandomEventCell, tick: u32) -> Frame {
    let id = match cell.trigger {
        Trigger::Skill(id) | Trigger::General(id) => id,
        Trigger::Seeded => unreachable!("seeded events have no cheat acknowledgement"),
    };
    chat(
        frame(tick, 100, 100, false),
        2,
        &format!("skill_macro_event {id}: {}.", cell.event),
    )
}

/// A correct evade: the bot flees from the attacker's start past
/// maxrange + 1 under a hold, the attacker disengages, the hold drops and
/// fishing XP resumes.
fn evade_pass_frames() -> Vec<Frame> {
    let mut frames = Vec::new();
    let mut f = acked(TROLL, 3);
    f.npcs = vec![npc(100, 101, true)];
    frames.push(f);
    for (i, z) in (83..=98).rev().enumerate() {
        let mut f = chat(
            frame(4 + i as u32, 100, z, true),
            2,
            "skill_macro_event 2: river troll.",
        );
        f.npcs = vec![npc(100, z + 1, true)];
        frames.push(f);
    }
    let mut f = frame(30, 100, 83, true);
    f.npcs = vec![npc(100, 98, false)];
    frames.push(f);
    let mut f = frame(31, 100, 83, false);
    f.npcs = vec![npc(100, 99, false)];
    frames.push(f);
    let mut f = frame(40, 100, 99, false);
    f.skill_xp = Some(1080);
    frames.push(f);
    frames
}

fn run(ep: &mut RandomEventEpisode, frames: Vec<Frame>) -> Verdict {
    let mut last = Verdict::Pending;
    for f in frames {
        last = ep.observe(f);
        if last != Verdict::Pending {
            return last;
        }
    }
    last
}

fn seeded_box_episode() -> RandomEventEpisode {
    let mut ep = RandomEventEpisode::new(STRANGE_BOX);
    let baseline = box_frame(1, false, 1);
    assert_eq!(ep.next_send(&baseline), None);
    assert_eq!(ep.observe(baseline), Verdict::Pending);
    assert_eq!(ep.next_send(&box_frame(2, false, 1)), None);
    ep
}

#[test]
fn seeded_strange_box_stays_script_owned_then_native_guardian_solves_it() {
    let mut ep = seeded_box_episode();
    for tick in 2..=6 {
        assert_eq!(ep.observe(box_frame(tick, false, 1)), Verdict::Pending);
    }
    assert_eq!(ep.script_owned, Some(6));
    assert_eq!(ep.observe(box_frame(7, true, 1)), Verdict::Pending);
    assert_eq!(ep.box_at_hold, Some(true));
    assert_eq!(ep.observe(box_frame(8, true, 0)), Verdict::Pending);
    assert_eq!(ep.item_cleared, Some(8));
    assert_eq!(ep.observe(box_frame(9, false, 0)), Verdict::Pass);
}

#[test]
fn seeded_strange_box_rejects_an_early_guardian_claim() {
    let mut ep = seeded_box_episode();
    assert!(matches!(
        ep.observe(box_frame(2, true, 1)),
        Verdict::Fail(message) if message.contains("retained the Strange box")
    ));
}

#[test]
fn seeded_strange_box_rejects_disappearance_before_guardian_claim() {
    let mut ep = seeded_box_episode();
    assert!(matches!(
        ep.observe(box_frame(2, false, 0)),
        Verdict::Fail(message) if message.contains("disappeared before native Guardian claimed")
    ));
}

#[test]
fn seeded_strange_box_rejects_release_before_consumption() {
    let mut ep = seeded_box_episode();
    for tick in 2..=6 {
        assert_eq!(ep.observe(box_frame(tick, false, 1)), Verdict::Pending);
    }
    assert_eq!(ep.observe(box_frame(7, true, 1)), Verdict::Pending);
    assert!(matches!(
        ep.observe(box_frame(8, false, 1)),
        Verdict::Fail(message) if message.contains("released the seeded Strange box")
    ));
}

#[test]
fn seeded_strange_box_rejects_a_claim_after_the_box_is_gone() {
    let mut ep = seeded_box_episode();
    for tick in 2..=6 {
        assert_eq!(ep.observe(box_frame(tick, false, 1)), Verdict::Pending);
    }
    assert!(matches!(
        ep.observe(box_frame(7, true, 0)),
        Verdict::Fail(message) if message.contains("not present when native Guardian claimed")
    ));
}

#[test]
fn seeded_strange_box_requires_the_item_at_baseline() {
    let mut ep = RandomEventEpisode::new(STRANGE_BOX);
    assert!(matches!(
        ep.observe(box_frame(1, false, 0)),
        Verdict::Fail(message) if message.contains("absent at the observation baseline")
    ));
}

#[test]
fn seeded_strange_box_claim_deadline_is_bounded_by_game_ticks() {
    let mut ep = seeded_box_episode();
    for tick in 2..=81 {
        assert_eq!(ep.observe(box_frame(tick, false, 1)), Verdict::Pending);
    }
    assert!(matches!(
        ep.observe(box_frame(82, false, 1)),
        Verdict::Fail(message) if message.contains("within 80 game ticks")
    ));
}

#[test]
fn evade_that_flees_past_chase_range_and_resumes_passes() {
    let mut ep = armed(TROLL);
    assert_eq!(run(&mut ep, evade_pass_frames()), Verdict::Pass);
    let receipt = ep.receipt();
    assert_eq!(receipt["escape"]["distance"], 17);
    assert_eq!(receipt["attacker"]["start"], json!([100, 101]));
    assert!(receipt["hold_rise"].is_number() && receipt["hold_fall"].is_number());
    assert_eq!(receipt["returned_inside_range"], 40);
}

#[test]
fn walking_back_toward_a_still_targeting_attacker_fails() {
    let mut ep = armed(TROLL);
    let mut frames: Vec<Frame> = evade_pass_frames().into_iter().take(10).collect();
    // Between hits (or after a miss) the bot heads back to its spot while
    // the troll still targets it.
    let mut back = frame(20, 100, 97, true);
    back.npcs = vec![npc(100, 96, true)];
    frames.push(back);
    match run(&mut ep, frames) {
        Verdict::Fail(message) => assert!(message.contains("walked back"), "{message}"),
        other => panic!("expected walk-back failure, got {other:?}"),
    }
}

#[test]
fn a_released_hold_inside_chase_range_is_a_give_up_failure() {
    let mut ep = armed(TROLL);
    let mut frames: Vec<Frame> = evade_pass_frames().into_iter().take(6).collect();
    let mut released = frame(20, 100, 95, false);
    released.npcs = vec![npc(100, 96, true)];
    frames.push(released);
    match run(&mut ep, frames) {
        Verdict::Fail(message) => assert!(message.contains("gave up"), "{message}"),
        other => panic!("expected give-up failure, got {other:?}"),
    }
}

const TROLL_FIGHT: RandomEventCell = RandomEventCell {
    event: "river troll",
    trigger: Trigger::Skill(2),
    oracle: Oracle::Fight { npc: "River troll" },
    skill: FISHING,
};

/// One observed tick of a fight: the hold, the troll at `(100, 101)` unless it
/// is `gone`, whether the bot targets it, and the troll's health so far (its
/// health bar totals 30).
fn fight_frame(tick: u32, hold: bool, gone: bool, fighting: bool, health: i32) -> Frame {
    let mut f = frame(tick, 100, 100, hold);
    if !gone {
        let mut troll = npc(100, 101, tick < 4);
        troll.health = (health, 30);
        f.npcs = vec![troll];
    }
    f.local_target = fighting.then(|| "River troll".to_string());
    f
}

/// A correct fight: the troll targets the bot, the Guardian holds, the bot
/// targets the troll and deals damage, the troll is gone for three ticks, the
/// hold drops after that and fishing XP resumes.
fn fight_pass_frames() -> Vec<Frame> {
    let mut frames = Vec::new();
    let mut acked = acked(TROLL_FIGHT, 3);
    acked.npcs = vec![npc(100, 101, true)];
    frames.push(acked);
    for tick in 4..=9 {
        frames.push(fight_frame(
            tick,
            true,
            false,
            true,
            if tick >= 6 { 24 } else { 30 },
        ));
    }
    for tick in 10..=12 {
        frames.push(fight_frame(tick, true, true, false, 0));
    }
    frames.push(fight_frame(13, false, true, false, 0));
    let mut resumed = fight_frame(20, false, true, false, 0);
    resumed.skill_xp = Some(1080);
    frames.push(resumed);
    frames
}

#[test]
fn a_fight_that_kills_the_actor_and_resumes_passes() {
    let mut ep = armed(TROLL_FIGHT);
    assert_eq!(run(&mut ep, fight_pass_frames()), Verdict::Pass);
    let receipt = ep.receipt();
    assert!(
        receipt["fight_started_tick"].is_number()
            && receipt["first_damage_observed_tick"].is_number()
    );
    assert!(
        receipt["escape"].is_null(),
        "a fight that ends in a kill never escaped"
    );
    assert!(receipt["hold_rise"].is_number() && receipt["hold_fall"].is_number());
}

#[test]
fn fight_receipt_records_host_outcome_stall_and_actual_combat_counts() {
    let mut ep = armed(TROLL_FIGHT);
    let mut frames = fight_pass_frames();
    for frame in &mut frames {
        match frame.tick {
            4 => {
                frame.fight_outcome = Some(FightOutcome::Pending);
                frame.no_progress_ticks = Some(1_001);
            }
            13 => {
                frame.fight_outcome = Some(FightOutcome::Killed);
                frame.no_progress_ticks = Some(42);
            }
            _ => {}
        }
    }
    assert_eq!(run(&mut ep, frames), Verdict::Pass);
    let receipt = ep.receipt();
    assert_eq!(receipt["fight_outcome"], "Killed");
    assert_eq!(receipt["guardian_no_progress_ticks_max"], 1_001);
    assert_eq!(receipt["guardian_no_progress_ticks_at_hold_release"], 42);
    assert_eq!(receipt["fight_target_ticks"], 6);
    assert_eq!(receipt["damage_event_count"], 1);
    assert_eq!(receipt["damage_hitpoints"], 6);
}

#[test]
fn fleeing_before_any_attack_fails_the_fight_oracle() {
    let mut ep = armed(TROLL_FIGHT);
    let mut frames: Vec<Frame> = Vec::new();
    let mut acked = acked(TROLL_FIGHT, 3);
    acked.npcs = vec![npc(100, 101, true)];
    frames.push(acked);
    for tick in 4..=6 {
        frames.push(fight_frame(tick, true, false, false, 30));
    }
    // The bot runs past the chase range with no attack and no damage: the
    // flee the Guardian had before it could fight.
    let mut fled = frame(20, 100, 83, true);
    fled.npcs = vec![npc(100, 98, true)];
    frames.push(fled);
    match run(&mut ep, frames) {
        Verdict::Fail(message) => assert!(message.contains("before it fought"), "{message}"),
        other => panic!("expected a flee failure, got {other:?}"),
    }
}

/// The hand over to Flee is a bound the product has: after an attack and
/// damage, the bot leaves the chase range under the hold, the troll lets go,
/// the hold drops and the fishing resumes.
#[test]
fn a_hand_over_to_flee_after_the_fight_passes() {
    let mut ep = armed(TROLL_FIGHT);
    let mut frames: Vec<Frame> = fight_pass_frames().into_iter().take(5).collect();
    let mut fled = frame(20, 100, 83, true);
    fled.npcs = vec![npc(100, 98, true)];
    frames.push(fled);
    let mut let_go = frame(21, 100, 83, true);
    let_go.npcs = vec![npc(100, 98, false)];
    frames.push(let_go);
    frames.push(frame(22, 100, 83, false));
    let mut resumed = frame(30, 100, 83, false);
    resumed.skill_xp = Some(1080);
    frames.push(resumed);
    assert_eq!(run(&mut ep, frames), Verdict::Pass);
    let receipt = ep.receipt();
    assert!(
        receipt["fight_started_tick"].is_number()
            && receipt["first_damage_observed_tick"].is_number()
    );
    assert!(
        receipt["escape"].is_object(),
        "the hand over left the chase range"
    );
}

/// A hold that drops while the troll still targets the bot is not an end:
/// the hand over waits until the actor lets go.
#[test]
fn a_hand_over_with_the_actor_still_on_the_bot_is_not_yet_an_end() {
    let mut ep = armed(TROLL_FIGHT);
    let mut frames: Vec<Frame> = fight_pass_frames().into_iter().take(5).collect();
    let mut fled = frame(20, 100, 83, true);
    fled.npcs = vec![npc(100, 98, true)];
    frames.push(fled);
    let mut dropped = frame(21, 100, 83, false);
    dropped.npcs = vec![npc(100, 98, true)];
    frames.push(dropped);
    let mut resumed = frame(30, 100, 83, false);
    resumed.skill_xp = Some(1080);
    resumed.npcs = vec![npc(100, 98, true)];
    frames.push(resumed);
    assert_eq!(run(&mut ep, frames), Verdict::Pending);
    assert!(ep.receipt()["resolved"].is_null());
}

#[test]
fn releasing_the_hold_while_the_actor_stands_fails_the_fight_oracle() {
    let mut ep = armed(TROLL_FIGHT);
    let mut frames: Vec<Frame> = fight_pass_frames().into_iter().take(5).collect();
    frames.push(fight_frame(20, false, false, true, 24));
    match run(&mut ep, frames) {
        Verdict::Fail(message) => assert!(message.contains("still stood"), "{message}"),
        other => panic!("expected a released-hold failure, got {other:?}"),
    }
}

/// A fight that outlasts the Guardian's whole-episode failure budget: the
/// hold is under it from the first targeting, with the bot attacking and the
/// troll damaged, until the tick `release`.
fn long_fight_frames(release: u32) -> Vec<Frame> {
    let mut frames = Vec::new();
    let mut acked = acked(TROLL_FIGHT, 3);
    acked.npcs = vec![npc(100, 101, true)];
    frames.push(acked);
    for tick in 4..release {
        frames.push(fight_frame(
            tick,
            true,
            false,
            true,
            if tick >= 6 { 24 } else { 30 },
        ));
    }
    frames.push(fight_frame(release, false, false, true, 24));
    frames
}

/// The Guardian lets go of an actor that still stands once its episode clock
/// ran out. With the fight proved, that release is an end the cell accepts,
/// whatever the roll of the fight was.
#[test]
fn the_episode_budget_release_with_the_actor_standing_passes() {
    let release = 3 + u32::from(api::random::EVADE_EPISODE_TICKS) - BUDGET_SLACK_TICKS;
    let mut ep = armed(TROLL_FIGHT);
    let mut frames = long_fight_frames(release);
    let mut resumed = fight_frame(release + 10, false, false, false, 24);
    resumed.skill_xp = Some(1080);
    frames.push(resumed);
    assert_eq!(run(&mut ep, frames), Verdict::Pass);
}

/// A release a tick short of the budget is the Guardian giving the fight up.
#[test]
fn a_release_short_of_the_episode_budget_is_a_give_up() {
    let release = 3 + u32::from(api::random::EVADE_EPISODE_TICKS) - BUDGET_SLACK_TICKS - 1;
    let mut ep = armed(TROLL_FIGHT);
    match run(&mut ep, long_fight_frames(release)) {
        Verdict::Fail(message) => assert!(message.contains("still stood"), "{message}"),
        other => panic!("expected a released-hold failure, got {other:?}"),
    }
}

#[test]
fn an_actor_gone_without_the_bot_having_fought_it_fails_the_fight_oracle() {
    let mut ep = armed(TROLL_FIGHT);
    let mut frames: Vec<Frame> = Vec::new();
    let mut acked = acked(TROLL_FIGHT, 3);
    acked.npcs = vec![npc(100, 101, true)];
    frames.push(acked);
    for tick in 4..=6 {
        frames.push(fight_frame(tick, true, false, false, 30));
    }
    // The troll vanishes (its lifetime ends) and the hold drops: no fight.
    for tick in 7..=9 {
        frames.push(fight_frame(tick, true, true, false, 0));
    }
    frames.push(fight_frame(10, false, true, false, 0));
    match run(&mut ep, frames) {
        Verdict::Fail(message) => assert!(message.contains("never fought"), "{message}"),
        other => panic!("expected a never-fought failure, got {other:?}"),
    }
}

#[test]
fn a_fight_oracle_run_without_damage_does_not_pass_on_a_target_alone() {
    let mut ep = armed(TROLL_FIGHT);
    let mut frames: Vec<Frame> = Vec::new();
    let mut acked = acked(TROLL_FIGHT, 3);
    acked.npcs = vec![npc(100, 101, true)];
    frames.push(acked);
    for tick in 4..=6 {
        frames.push(fight_frame(tick, true, false, true, 30));
    }
    for tick in 7..=9 {
        frames.push(fight_frame(tick, true, true, false, 0));
    }
    frames.push(fight_frame(10, false, true, false, 0));
    match run(&mut ep, frames) {
        Verdict::Fail(message) => assert!(message.contains("never fought"), "{message}"),
        other => panic!("expected a no-damage failure, got {other:?}"),
    }
}

#[test]
fn an_attacker_the_guardian_never_holds_for_fails() {
    let mut ep = armed(SWARM);
    let mut frames = Vec::new();
    for tick in 3..12 {
        let mut f = frame(tick, 100, 100, false);
        f.npcs = vec![npc(100, 101, true)];
        frames.push(f);
    }
    match run(&mut ep, frames) {
        Verdict::Fail(message) => assert!(message.contains("did not take the slot"), "{message}"),
        other => panic!("expected no-hold failure, got {other:?}"),
    }
}

#[test]
fn an_escape_without_resumed_work_stays_pending() {
    let mut ep = armed(TROLL);
    let mut frames = evade_pass_frames();
    frames.pop();
    assert_eq!(run(&mut ep, frames), Verdict::Pending);
    assert!(ep.receipt()["resolved"].is_object());
    assert!(ep.receipt()["resumed"].is_null());
}

#[test]
fn death_fails_whatever_the_class() {
    let mut ep = armed(SWARM);
    let mut f = frame(3, 100, 100, true);
    f.npcs = vec![npc(100, 101, true)];
    assert_eq!(ep.observe(f), Verdict::Pending);
    let dead = chat(frame(4, 3222, 3218, false), 9, "Oh dear you are dead!");
    match ep.observe(dead) {
        Verdict::Fail(message) => assert!(message.contains("died"), "{message}"),
        other => panic!("expected death failure, got {other:?}"),
    }
}

#[test]
fn a_failed_event_teleport_fails() {
    let mut ep = armed(DWARF);
    let mut f = frame(3, 100, 100, true);
    f.npcs = vec![npc(100, 101, false)];
    assert_eq!(ep.observe(f), Verdict::Pending);
    match ep.observe(frame(4, 400, 400, false)) {
        Verdict::Fail(message) => assert!(message.contains("teleported"), "{message}"),
        other => panic!("expected teleport failure, got {other:?}"),
    }
}

#[test]
fn a_silent_skill_cheat_is_unavailable_and_names_the_patch() {
    let mut ep = armed(TROLL);
    let mut verdict = Verdict::Pending;
    for tick in 3..20 {
        verdict = ep.observe(frame(tick, 100, 100, false));
        if verdict != Verdict::Pending {
            break;
        }
    }
    match verdict {
        Verdict::Unavailable(message) => {
            assert!(message.starts_with("UNAVAILABLE"), "{message}");
            assert!(message.contains(SKILL_EVENT_PATCH), "{message}");
        }
        other => panic!("expected UNAVAILABLE, got {other:?}"),
    }
}

#[test]
fn an_answered_skill_cheat_is_not_unavailable() {
    let mut ep = armed(TROLL);
    let mut verdict = ep.observe(acked(TROLL, 3));
    for tick in 4..40 {
        verdict = ep.observe(acked(TROLL, tick));
        if verdict != Verdict::Pending {
            break;
        }
    }
    // Answered but no troll ever appears: a plain failure, never UNAVAILABLE.
    match verdict {
        Verdict::Fail(message) => assert!(message.contains("never appeared"), "{message}"),
        other => panic!("expected onset failure, got {other:?}"),
    }
}

#[test]
fn a_refused_skill_cheat_is_a_fixture_failure() {
    let mut ep = armed(BIG_FISH);
    let refused = chat(
        frame(3, 100, 100, false),
        2,
        "skill_macro_event 1: refused, no fishing spot next to you.",
    );
    match ep.observe(refused) {
        Verdict::Fail(message) => assert!(message.starts_with("fixture:"), "{message}"),
        other => panic!("expected fixture failure, got {other:?}"),
    }
}

#[test]
fn a_busy_reply_resends_and_a_quiet_one_does_not() {
    let mut ep = armed(SWARM);
    assert_eq!(ep.next_send(&frame(3, 100, 100, false)), None);
    let busy = chat(frame(4, 100, 100, false), 2, BUSY_REPLY);
    assert_eq!(ep.next_send(&busy), Some("~macro_event 1".into()));
    assert_eq!(ep.observe(busy.clone()), Verdict::Pending);
    // The same busy line does not authorize another send.
    assert_eq!(ep.next_send(&busy), None);
}

#[test]
fn a_talked_through_dwarf_with_its_reward_passes() {
    let mut ep = armed(DWARF);
    let mut f = frame(3, 100, 100, false);
    f.npcs = vec![npc(101, 100, false)];
    assert_eq!(ep.observe(f), Verdict::Pending);
    let mut f = frame(4, 100, 100, true);
    f.npcs = vec![npc(101, 100, false)];
    f.held.push((KEBAB, 1));
    assert_eq!(ep.observe(f), Verdict::Pending);
    for tick in 5..9 {
        let mut f = frame(tick, 100, 100, tick < 8);
        f.held.push((KEBAB, 1));
        assert_eq!(ep.observe(f), Verdict::Pending);
    }
    let mut f = frame(9, 100, 100, false);
    f.held.push((KEBAB, 1));
    assert_eq!(ep.observe(f), Verdict::Pending);
    let mut f = frame(12, 100, 100, false);
    f.skill_xp = Some(1100);
    assert_eq!(ep.observe(f), Verdict::Pass);
}

#[test]
fn a_dialog_event_the_guardian_ignores_fails() {
    let mut ep = armed(DWARF);
    let mut verdict = Verdict::Pending;
    for tick in 3..30 {
        let mut f = frame(tick, 100, 100, false);
        f.npcs = vec![npc(101, 100, false)];
        verdict = ep.observe(f);
        if verdict != Verdict::Pending {
            break;
        }
    }
    match verdict {
        Verdict::Fail(message) => assert!(message.contains("never engaged"), "{message}"),
        other => panic!("expected never-engaged failure, got {other:?}"),
    }
}

#[test]
fn a_held_maze_visit_that_returns_and_resumes_passes() {
    let mut ep = armed(MAZE);
    let (mx, mz, ml) = maze_tile();
    let mut f = frame(4, mx, mz, true);
    f.tile = Some((mx, mz, ml));
    assert_eq!(ep.observe(f.clone()), Verdict::Pending);
    f.tick = 5;
    assert_eq!(ep.observe(f), Verdict::Pending);
    assert_eq!(ep.observe(frame(60, 100, 100, false)), Verdict::Pending);
    let mut back = frame(70, 100, 100, false);
    back.skill_xp = Some(1200);
    assert_eq!(ep.observe(back), Verdict::Pass);
}

#[test]
fn an_unheld_maze_visit_fails() {
    let mut ep = armed(MAZE);
    let (mx, mz, _) = maze_tile();
    let mut verdict = Verdict::Pending;
    for tick in 4..20 {
        verdict = ep.observe(frame(tick, mx, mz, false));
        if verdict != Verdict::Pending {
            break;
        }
    }
    match verdict {
        Verdict::Fail(message) => assert!(message.contains("never held"), "{message}"),
        other => panic!("expected never-held failure, got {other:?}"),
    }
}

#[test]
fn a_recovered_fishing_tool_passes_and_an_abandoned_one_never_does() {
    let lost = |tick| {
        let mut f = acked(BIG_FISH, tick);
        f.held.clear();
        f
    };
    let mut ep = armed(BIG_FISH);
    assert_eq!(ep.observe(lost(3)), Verdict::Pending);
    assert_eq!(ep.observe(lost(4)), Verdict::Pending);
    assert_eq!(ep.observe(frame(6, 101, 100, true)), Verdict::Pending);
    assert_eq!(ep.observe(frame(7, 100, 100, false)), Verdict::Pending);
    let mut f = frame(9, 100, 100, false);
    f.skill_xp = Some(1050);
    assert_eq!(ep.observe(f), Verdict::Pass);

    let mut abandoned = armed(BIG_FISH);
    for tick in 3..80 {
        let mut f = lost(tick);
        f.skill_xp = Some(1000 + tick as i32);
        assert_eq!(abandoned.observe(f), Verdict::Pending, "tick {tick}");
    }
}

#[test]
fn a_hit_from_the_still_targeting_attacker_after_release_fails() {
    // Diagnostic run diag3/tree_spirit-4: escape counted, hold released, the
    // spirit kept targeting and hitting the bot until it died.
    let mut ep = armed(TROLL);
    let mut frames: Vec<Frame> = evade_pass_frames().into_iter().take(17).collect();
    let mut released = frame(21, 100, 83, false);
    released.npcs = vec![npc(100, 84, true)];
    frames.push(released);
    let mut hit = frame(22, 100, 83, false);
    hit.npcs = vec![npc(100, 84, true)];
    hit.hp = Some(8);
    frames.push(hit);
    match run(&mut ep, frames) {
        Verdict::Fail(message) => assert!(message.contains("still targeting"), "{message}"),
        other => panic!("expected hit-after-release failure, got {other:?}"),
    }
}

#[test]
fn a_hit_after_release_from_a_disengaged_attacker_is_not_blamed_on_the_evade() {
    let mut ep = armed(TROLL);
    let mut frames = evade_pass_frames();
    let resumed = frames.pop().unwrap();
    let mut queued = frame(35, 100, 90, false);
    queued.npcs = vec![npc(100, 99, false)];
    queued.hp = Some(9);
    frames.push(queued);
    frames.push(resumed);
    assert_eq!(run(&mut ep, frames), Verdict::Pass);
}
