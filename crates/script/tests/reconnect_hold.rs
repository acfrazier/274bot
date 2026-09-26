//! A reconnect the host relogs through holds the whole script. Frozen
//! AutoRelogin pauses the script on the disconnect and resumes it on the new
//! session's scene 2 (`AutoRelogin.ts:180-190`, `159-163`), with every parked
//! waiter shifted by the paused span (`ScriptContext.ts:101-117`): the chain
//! the script had in flight carries on as one, and nothing else starts.
//!
//! The host re-arms the script walk it was following under the same request
//! id (host-play `hold_script_nav`); here the isolate side is driven alone,
//! so arrival settles the held walk.

use std::thread::sleep;
use std::time::Duration;

use script::isolate_fb::{ChatLineInput, SceneEntityInput, SnapshotInput, TileInput};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};

mod common;
use common::{ingame_snapshot, post_snapshot_input};

fn tile(x: i32, z: i32) -> TileInput {
    TileInput { x, z, level: 0 }
}

/// Post `snap`, run its tick and wait for it (the probe orders after it).
fn tick(iso: &LoadIsolate, snap: &SnapshotInput<'_>) {
    post_snapshot_input(iso, snap);
    iso.on_game_tick(snap.tick);
    let _ = iso.probe("true");
}

fn value(iso: &LoadIsolate, expr: &str) -> serde_json::Value {
    iso.probe(expr).unwrap()
}

fn walks(reqs: &[InteractReq]) -> Vec<&InteractReq> {
    reqs.iter()
        .filter(|req| matches!(req, InteractReq::Walk { .. } | InteractReq::WalkNear { .. }))
        .collect()
}

/// The relogged session as the host posts it once the slot runs again: the
/// scene still held (the welcome screen), then play, at `here`. Returns the
/// next free tick. Nothing may go out and nothing may settle meanwhile.
fn relog(iso: &LoadIsolate, snap: &mut SnapshotInput<'_>, from: u64, here: TileInput) -> u64 {
    snap.here = Some(here);
    snap.hold = true;
    snap.tick = from;
    tick(iso, snap);
    snap.hold = false;
    snap.tick = from + 1;
    tick(iso, snap);
    snap.tick = from + 2;
    tick(iso, snap);
    from + 3
}

#[test]
fn a_walk_in_flight_across_a_reconnect_settles_after_the_relog() {
    let src = r#"
import { Traversal } from '../../api/walking/Traversal.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__ran) return;
        globalThis.__ran = true;
        globalThis.__ok = null;
        globalThis.__ok = await Traversal.walkResilient(
            { x: 3300, z: 3300, level: 0 },
            { radius: 1, timeoutMs: 300000 },
        );
    }
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = ingame_snapshot();
    snap.here = Some(tile(3222, 3222));
    tick(&iso, &snap);
    assert_eq!(walks(&iso.drain_interacts()).len(), 1, "the walk is out");

    iso.reconnect_session_work();
    let next = relog(&iso, &mut snap, 2, tile(3240, 3240));
    assert_eq!(
        value(&iso, "globalThis.__ok"),
        serde_json::Value::Null,
        "a reconnect does not end the walk"
    );
    assert!(
        walks(&iso.drain_interacts()).is_empty(),
        "the held walk issues no second walk (the host re-arms the first)"
    );

    snap.tick = next;
    snap.here = Some(tile(3300, 3301));
    tick(&iso, &snap);
    assert_eq!(
        value(&iso, "globalThis.__ok"),
        true,
        "the held walk arrives"
    );
    iso.join();
}

#[test]
fn a_bank_walk_across_a_reconnect_goes_on_to_open_the_bank() {
    let src = r#"
import { Banking } from '../../api/bank/Banking.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        globalThis.__ok = await Banking.open({
            stand: { x: 3250, z: 3250, level: 0 },
            boothName: 'Bank chest',
            boothOp: 'Bank',
        });
    }
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::CompatClass, vec![]).unwrap();
    let bank = ["Bank".to_string()];
    let locs = [SceneEntityInput {
        index: 0,
        id: 4483,
        name: Some("Bank chest"),
        x: 3251,
        z: 3250,
        level: 0,
        distance: 30,
        health: -1,
        max_health: -1,
        in_combat: false,
        animating: false,
        actions: &bank,
        reachable: false,
        reachable_adj: false,
        combat_level: 0,
        target_kind: 0,
        target_index: -1,
        size: 0,
        nx: 0,
        nz: 0,
        shape: 0,
        angle: 0,
    }];
    let mut snap = ingame_snapshot();
    snap.here = Some(tile(3222, 3222));
    snap.locs = &locs;
    tick(&iso, &snap);
    assert!(
        matches!(
            walks(&iso.drain_interacts()).as_slice(),
            [InteractReq::WalkNear {
                x: 3250,
                z: 3250,
                ..
            }]
        ),
        "Banking.open walks to the stand first"
    );

    iso.reconnect_session_work();
    let next = relog(&iso, &mut snap, 2, tile(3230, 3230));
    assert_eq!(value(&iso, "globalThis.__ok"), serde_json::Value::Null);
    assert!(walks(&iso.drain_interacts()).is_empty());

    // At the stand: the held Banking.open carries on to the booth.
    snap.tick = next;
    snap.here = Some(tile(3250, 3250));
    let approaches = [script::isolate_fb::BankApproachInput {
        loc_id: 4483,
        x: 3251,
        z: 3250,
        level: 0,
        can_operate: true,
        dest_ok: true,
        dest_x: 3250,
        dest_z: 3250,
        dest_level: 0,
    }];
    iso.post_snapshot(script::isolate_fb::encode_snapshot_with_native(
        &snap,
        script::isolate_fb::NativeFactsInput {
            bank_approaches: Some(&approaches),
            ..Default::default()
        },
    ));
    iso.on_game_tick(next);
    let _ = iso.probe("true");
    assert!(
        iso.drain_interacts()
            .iter()
            .any(|req| matches!(req, InteractReq::OpenBooth { id: 4483, .. })),
        "the resumed Banking.open opens the booth"
    );
    snap.tick = next + 1;
    snap.bank_open = true;
    snap.bank_loaded = true;
    snap.bank_generation = 1;
    tick(&iso, &snap);
    assert_eq!(value(&iso, "globalThis.__ok"), true);
    iso.join();
}

fn line(seq: i32, text: &'static str) -> ChatLineInput<'static> {
    ChatLineInput {
        seq,
        text,
        type_: 0,
        username: None,
    }
}

/// HillGiant / WildyAgility / RoguesPurse shape: DeathRecovery's `walkBack`
/// awaits its own walk, and another task walks once it may run. Across a
/// reconnect mid-`walkBack` the recovery row, its callback's walk and the
/// loop parked on them stay one chain: the other task does not start and no
/// second walk goes out until `walkBack` has finished.
#[test]
fn a_death_recovery_walk_across_a_reconnect_stays_the_only_chain() {
    let src = r#"
import { DeathRecovery } from '../../api/tasks/DeathRecovery.js';
import { Traversal } from '../../api/walking/Traversal.js';
class Wander {
    validate() { return (globalThis.__deaths || 0) > 0; }
    async execute() {
        globalThis.__wander = (globalThis.__wander || 0) + 1;
        await Traversal.walkResilient({ x: 3400, z: 3400, level: 0 }, { radius: 1, timeoutMs: 300000 });
    }
}
export default class HillGiantShaped extends TaskBot {
    onStart() {
        this.add(new DeathRecovery(this, {
            anchor: { x: 3300, z: 3300, level: 0 },
            radius: 6,
            onDeath: () => { globalThis.__deaths = (globalThis.__deaths || 0) + 1; },
            onRecovered: () => { globalThis.__recovered = (globalThis.__recovered || 0) + 1; },
            walkBack: async () => {
                const ok = await Traversal.walkResilient(
                    { x: 3300, z: 3300, level: 0 },
                    { radius: 1, timeoutMs: 300000 },
                );
                globalThis.__wbSettled = (globalThis.__wbSettled || 0) + 1;
                globalThis.__wbLast = ok;
            },
        }));
        this.add(new Wander());
    }
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::CompatClass, vec![]).unwrap();
    let welcome = [line(1, "Welcome to RuneScape")];
    let died = [
        line(2, "Oh dear, you are dead!"),
        line(1, "Welcome to RuneScape"),
    ];
    let mut snap = ingame_snapshot();
    snap.here = Some(tile(3222, 3218));
    snap.chat_lines = &welcome;
    tick(&iso, &snap);
    snap.chat_lines = &died;
    let mut n = 2;
    let mut out = Vec::new();
    while n < 40 && walks(&out).is_empty() {
        snap.tick = n;
        tick(&iso, &snap);
        out = iso.drain_interacts();
        n += 1;
    }
    assert!(
        matches!(
            walks(&out).as_slice(),
            [InteractReq::WalkNear {
                x: 3300,
                z: 3300,
                ..
            }]
        ),
        "walkBack's walk is out: {out:?}"
    );

    iso.reconnect_session_work();
    let mut next = relog(&iso, &mut snap, n, tile(3230, 3230));
    for _ in 0..5 {
        snap.tick = next;
        tick(&iso, &snap);
        next += 1;
    }
    assert_eq!(
        value(&iso, "globalThis.__wbSettled ?? null"),
        serde_json::Value::Null,
        "walkBack's walk is still in flight"
    );
    assert_eq!(
        value(&iso, "globalThis.__wander ?? null"),
        serde_json::Value::Null,
        "no other task runs beside the held recovery"
    );
    assert!(
        walks(&iso.drain_interacts()).is_empty(),
        "no second walk goes out"
    );

    // walkBack arrives: the recovery ends, and only then does the next task run.
    snap.here = Some(tile(3300, 3300));
    for _ in 0..6 {
        snap.tick = next;
        tick(&iso, &snap);
        next += 1;
    }
    assert_eq!(value(&iso, "globalThis.__wbSettled"), 1);
    assert_eq!(value(&iso, "globalThis.__wbLast"), true);
    assert_eq!(value(&iso, "globalThis.__recovered"), 1);
    assert_eq!(value(&iso, "globalThis.__wander"), 1, "then the next task");
    iso.join();
}

/// A timed wait parked across a reconnect keeps the time it had left:
/// frozen `ScriptContext.resume` shifts `timeoutAt` by the paused span.
#[test]
fn a_wait_timeout_across_a_reconnect_keeps_the_time_it_had_left() {
    let src = r#"
import { Execution } from '../../api/execution/Execution.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__ran) return;
        globalThis.__ran = true;
        globalThis.__ok = null;
        globalThis.__ok = await Execution.delayUntil(() => false, 600);
    }
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = ingame_snapshot();
    tick(&iso, &snap);

    iso.reconnect_session_work();
    sleep(Duration::from_millis(900));
    let here = snap.here.unwrap();
    let next = relog(&iso, &mut snap, 2, here);
    assert_eq!(
        value(&iso, "globalThis.__ok"),
        serde_json::Value::Null,
        "the time disconnected does not count against the wait"
    );

    sleep(Duration::from_millis(800));
    snap.tick = next;
    tick(&iso, &snap);
    assert_eq!(value(&iso, "globalThis.__ok"), false, "the rest of it does");
    iso.join();
}
