//! Frozen `buryOneInFight` (`api/combat/fightUpkeep.ts:24-37`): skip only the
//! tick our swing began, bury during the cooldown, and report a burial only
//! once the backpack drops a slot within three ticks.

mod common;

use script::isolate_fb::{ItemRowInput, SnapshotInput};
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

#[test]
fn only_the_swing_start_tick_is_skipped() {
    let ops = vec!["Bury".to_string()];
    let two = [bones(&ops, 0), bones(&ops, 1)];

    // The tick our swing began: no click, false.
    let iso = LoadIsolate::spawn(SRC.into(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = common::ingame_snapshot();
    snap.inv = &two;
    snap.animating = false;
    common::post_snapshot_input(&iso, &snap);
    snap.animating = true;
    post_tick(&iso, &mut snap, 2);
    assert_eq!(iso.probe("__buried").unwrap(), false);
    assert!(
        iso.drain_interacts().is_empty(),
        "the swing tick costs no bury"
    );
    iso.join();

    // Still animating after the swing began (the cooldown): bury.
    let iso = LoadIsolate::spawn(SRC.into(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = common::ingame_snapshot();
    snap.inv = &two;
    snap.animating = true;
    snap.tick = 1;
    common::post_snapshot_input(&iso, &snap);
    post_tick(&iso, &mut snap, 2);
    assert_eq!(
        iso.drain_interacts(),
        vec![bury()],
        "the cooldown ticks bury"
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
