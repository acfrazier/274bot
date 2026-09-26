//! Frozen `reader.selfAnim()` is the local player's primary animation id
//! (`adapter/ClientAdapter.ts:492-494`) and `Game.animating()` is
//! `selfAnim() !== -1` (`api/game/Game.ts:112-114`): walking alone is not
//! animating, and `lightFire` (`api/firemaking/LightFire.ts:36-39`) does not
//! count a walk as the fire starting.

mod common;

use script::isolate_fb::{
    encode_snapshot_with_native, ItemRowInput, NativeFactsInput, SnapshotInput, StatInput,
};
use script::{LoadIsolate, LoadShape};
use serde_json::{json, Value};

/// One post: `moving` is the host's walking-or-animating flag, `anim` the
/// posted primary animation id.
fn post(iso: &LoadIsolate, snap: &mut SnapshotInput<'_>, tick: u64, moving: bool, anim: i32) {
    snap.tick = tick;
    snap.animating = moving;
    let native = NativeFactsInput {
        self_anim: Some(anim),
        ..NativeFactsInput::default()
    };
    iso.post_snapshot(encode_snapshot_with_native(snap, native));
    iso.on_game_tick(tick);
    let _ = iso.probe("true");
}

#[test]
fn walking_is_not_animating_and_self_anim_is_the_animation_id() {
    let src = r#"
import { reader } from '../../adapter/ClientAdapter.js';
import { Game } from '../../api/game/Game.js';
globalThis.__seen = [];
export default class T extends LoopingBot {
    loop() { globalThis.__seen.push([reader.selfAnim(), Game.animating()]); }
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = common::ingame_snapshot();
    post(&iso, &mut snap, 1, true, -1);
    post(&iso, &mut snap, 2, true, 390);
    post(&iso, &mut snap, 3, false, -1);
    assert_eq!(
        iso.probe("__seen").unwrap(),
        json!([[-1, false], [390, true], [-1, false]])
    );
    iso.join();
}

#[test]
fn a_walk_is_not_the_fire_starting() {
    let src = r#"
import { lightFire } from '../../api/firemaking/LightFire.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        globalThis.__ok = await lightFire('Logs');
    }
}
"#;
    let row = |name, id, slot| ItemRowInput {
        name: Some(name),
        count: 1,
        id,
        ops: &[],
        noted: false,
        cert: -1,
        component_id: -1,
        slot,
    };
    let inv = [row("Tinderbox", 590, 0), row("Logs", 1511, 1)];
    let stats = [StatInput {
        index: 11,
        name: "firemaking",
        xp: 0,
        base: 1,
        effective: 1,
    }];
    let iso = LoadIsolate::spawn(src.into(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = common::ingame_snapshot();
    snap.inv = &inv;
    snap.stats = &stats;
    post(&iso, &mut snap, 1, false, -1);
    assert_eq!(iso.drain_interacts().len(), 1, "the tinderbox use-on");
    // Walking the whole start window (FIRE_START_TICKS = 14) with no log
    // used, no chat and no animation: the attempt never started.
    for tick in 2..=15 {
        post(&iso, &mut snap, tick, true, -1);
    }
    assert_eq!(iso.probe("__ok").unwrap(), Value::from("stalled"));
    iso.join();
}
