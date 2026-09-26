//! Frozen `buryOneInFight` (`api/combat/fightUpkeep.ts:24-37`): skip only the
//! tick our swing began, bury during the cooldown, and report a burial only
//! once the backpack drops a slot within three ticks.

mod common;

use script::isolate_fb::{
    encode_snapshot_with_native, ItemRowInput, NativeFactsInput, SnapshotInput,
};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};
use serde_json::Value;

const SRC: &str = r#"
import { buryOneInFight } from '../../api/combat/fightUpkeep.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__buried = null;
        globalThis.__buried = await buryOneInFight('Bones');
    }
}
"#;

fn bones<'a>(ops: &'a [String], slot: i32) -> ItemRowInput<'a> {
    ItemRowInput {
        name: Some("Bones"),
        count: 1,
        id: 526,
        ops,
        noted: false,
        cert: -1,
        component_id: 3214,
        slot,
    }
}

fn post_tick(iso: &LoadIsolate, snap: &mut SnapshotInput<'_>, tick: u64) {
    snap.tick = tick;
    common::post_snapshot_input(iso, snap);
    iso.on_game_tick(tick);
    let _ = iso.probe("true");
}

fn bury() -> InteractReq {
    InteractReq::Held {
        name: "Bones".into(),
        action: "Bury".into(),
    }
}

#[test]
fn a_bury_counts_only_once_the_pack_drops_a_slot() {
    let ops = vec!["Bury".to_string()];
    let two = [bones(&ops, 0), bones(&ops, 1)];
    let one = [bones(&ops, 1)];
    let iso = LoadIsolate::spawn(SRC.into(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = common::ingame_snapshot();
    snap.inv = &two;
    post_tick(&iso, &mut snap, 1);
    assert_eq!(iso.drain_interacts(), vec![bury()]);
    assert_eq!(
        iso.probe("__buried").unwrap(),
        Value::Null,
        "a queued Bury is not a burial"
    );

    snap.inv = &one;
    post_tick(&iso, &mut snap, 2);
    assert_eq!(iso.probe("__buried").unwrap(), true);
    assert!(iso.drain_interacts().is_empty(), "one click per bury");
    iso.join();
}

#[test]
fn an_unconfirmed_bury_is_false_after_three_ticks() {
    let ops = vec!["Bury".to_string()];
    let two = [bones(&ops, 0), bones(&ops, 1)];
    let iso = LoadIsolate::spawn(SRC.into(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = common::ingame_snapshot();
    snap.inv = &two;
    post_tick(&iso, &mut snap, 1);
    assert_eq!(iso.drain_interacts(), vec![bury()]);
    for tick in 2..=3 {
        post_tick(&iso, &mut snap, tick);
        assert_eq!(iso.probe("__buried").unwrap(), Value::Null, "tick {tick}");
    }
    post_tick(&iso, &mut snap, 4);
    assert_eq!(iso.probe("__buried").unwrap(), false);
    assert!(iso.drain_interacts().is_empty(), "no second click");
    iso.join();
}

/// Calls `swingStartedThisTick()` every loop (recorded in `__swings`) and
/// `buryOneInFight` instead on the loop after the test sets `__bury`.
const CLOCK_SRC: &str = r#"
import { buryOneInFight, swingStartedThisTick } from '../../api/combat/fightUpkeep.js';
globalThis.__swings = [];
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__bury) {
            globalThis.__bury = false;
            globalThis.__buried = null;
            globalThis.__buried = await buryOneInFight('Bones');
            return;
        }
        globalThis.__swings.push(swingStartedThisTick());
    }
}
"#;

/// One post carrying the local player's animation id (`-1` idle).
fn post_anim(iso: &LoadIsolate, snap: &mut SnapshotInput<'_>, tick: u64, anim: i32) {
    snap.tick = tick;
    snap.animating = anim != -1;
    let native = NativeFactsInput {
        self_anim: Some(anim),
        ..NativeFactsInput::default()
    };
    iso.post_snapshot(encode_snapshot_with_native(snap, native));
    iso.on_game_tick(tick);
    let _ = iso.probe("true");
}

/// Frozen `AttackClock.observe` (`eatTiming.ts:31-39`): the first animation
/// id seen is a swing start, so a script started mid-swing does not bury on
/// that tick.
#[test]
fn a_script_started_mid_swing_skips_that_tick() {
    let ops = vec!["Bury".to_string()];
    let two = [bones(&ops, 0), bones(&ops, 1)];
    let iso = LoadIsolate::spawn(CLOCK_SRC.into(), LoadShape::CompatClass, vec![]).unwrap();
    let _ = iso.probe("globalThis.__bury = true");
    let mut snap = common::ingame_snapshot();
    snap.inv = &two;
    post_anim(&iso, &mut snap, 1, 390);
    assert_eq!(iso.probe("__buried").unwrap(), false);
    assert!(
        iso.drain_interacts().is_empty(),
        "the swing tick costs no bury"
    );
    iso.join();
}

/// A changed non-idle animation is a new swing even though the player never
/// stopped animating; the same id held, or a return to idle, is not.
#[test]
fn a_new_animation_while_animating_is_a_new_swing() {
    let iso = LoadIsolate::spawn(CLOCK_SRC.into(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = common::ingame_snapshot();
    for (tick, anim) in [(1, -1), (2, 390), (3, 390), (4, 391), (5, -1)] {
        post_anim(&iso, &mut snap, tick, anim);
    }
    assert_eq!(
        iso.probe("__swings").unwrap(),
        serde_json::json!([false, true, false, true, false])
    );
    iso.join();
}

#[test]
fn only_the_swing_start_tick_is_skipped() {
    let ops = vec!["Bury".to_string()];
    let two = [bones(&ops, 0), bones(&ops, 1)];

    // The tick our swing began: no click, false.
    let iso = LoadIsolate::spawn(CLOCK_SRC.into(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = common::ingame_snapshot();
    snap.inv = &two;
    post_anim(&iso, &mut snap, 1, -1);
    let _ = iso.probe("globalThis.__bury = true");
    post_anim(&iso, &mut snap, 2, 390);
    assert_eq!(iso.probe("__buried").unwrap(), false);
    assert!(
        iso.drain_interacts().is_empty(),
        "the swing tick costs no bury"
    );
    iso.join();

    // Still in that swing's animation (the cooldown): bury.
    let iso = LoadIsolate::spawn(CLOCK_SRC.into(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = common::ingame_snapshot();
    snap.inv = &two;
    post_anim(&iso, &mut snap, 1, 390);
    let _ = iso.probe("globalThis.__bury = true");
    post_anim(&iso, &mut snap, 2, 390);
    assert_eq!(
        iso.drain_interacts(),
        vec![bury()],
        "the cooldown ticks bury"
    );
    iso.join();
}

/// A script's own `new AttackClock()` (GreenDragon) keeps its own state and
/// observes the `anim` it is given (`eatTiming.ts:31-39`), not the posted
/// animation: here the posted id is 390 throughout while the script feeds
/// -1, -1, 500, 500 (then resets) and 500.
#[test]
fn an_attack_clock_instance_observes_the_anim_it_is_given() {
    let src = r#"
import { AttackClock } from '../../api/combat/eatTiming.js';
import { BotHost } from '../../runtime/BotHost.js';
const clock = new AttackClock();
const fed = { 1: -1, 2: -1, 3: 500, 4: 500, 5: 500 };
globalThis.__seen = [];
export default class T extends LoopingBot {
    loop() {
        const tick = BotHost.tickCount;
        clock.observe(fed[tick], tick);
        globalThis.__seen.push(clock.attackedThisTick(tick));
        if (tick === 4) clock.reset();
    }
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = common::ingame_snapshot();
    for tick in 1..=5 {
        post_anim(&iso, &mut snap, tick, 390);
    }
    // 500 is first seen on tick 3; after `reset` it is new again on tick 5.
    assert_eq!(
        iso.probe("__seen").unwrap(),
        serde_json::json!([false, false, true, false, true])
    );
    iso.join();
}

#[test]
fn a_bone_row_without_a_bury_op_is_false_without_a_click() {
    let ops = vec!["Use".to_string()];
    let inv = [bones(&ops, 0)];
    let iso = LoadIsolate::spawn(SRC.into(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = common::ingame_snapshot();
    snap.inv = &inv;
    post_tick(&iso, &mut snap, 1);
    assert_eq!(iso.probe("__buried").unwrap(), false);
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}
