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
        hold,
        hp: Some(10),
        skill_xp: Some(1000),
        strength_xp: Some(0),
        chat: vec![(1, "Welcome".into())],
        npcs: Vec::new(),
        held: vec![(HARPOON, 1)],
        fruit: false,
    }
}

fn npc(x: i32, z: i32, targets_local: bool) -> FrameNpc {
    FrameNpc {
        index: 7,
        type_id: Some(391),
        tile: (x, z),
        targets_local,
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

#[test]
fn evade_that_flees_past_chase_range_and_resumes_passes() {
    let mut ep = armed(TROLL);
    assert_eq!(run(&mut ep, evade_pass_frames()), Verdict::Pass);
    let receipt = ep.receipt();
    assert_eq!(receipt["escape"]["distance"], 17);
    assert_eq!(receipt["attacker"]["start"], json!([100, 101]));
    assert!(receipt["hold_rise"].is_number() && receipt["hold_fall"].is_number());
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
