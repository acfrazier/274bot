//! A kept reconnect holds the script and its parked waits, but it discards
//! gameplay requests from the previous session. The next session's own ticks
//! may produce fresh interactions; stale walks are never replayed outside
//! the seat.

use std::thread::sleep;
use std::time::{Duration, Instant};

use script::isolate_fb::{ChatLineInput, SceneEntityInput, SnapshotInput, TileInput};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};

mod common;
use common::{ingame_snapshot, post_snapshot_input};

#[path = "common/bank_slot.rs"]
mod bank_slot;

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

fn wait_for_execution(iso: &LoadIsolate, message: &str) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !iso.execution_active() {
        assert!(Instant::now() < deadline, "{message}");
        std::thread::yield_now();
    }
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

    iso.reconnect_session_work(None);
    let next = relog(&iso, &mut snap, 2, tile(3240, 3240));
    assert_eq!(
        value(&iso, "globalThis.__ok"),
        serde_json::Value::Null,
        "a reconnect does not end the walk"
    );
    assert!(
        walks(&iso.drain_interacts()).is_empty(),
        "the old-session walk is not emitted again"
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
        globalThis.__settled = 0;
        globalThis.__ok = await Banking.open({
            stand: { x: 3250, z: 3250, level: 0 },
            boothName: 'Bank chest',
            boothOp: 'Bank',
        });
        globalThis.__settled++;
    }
}
"#;
    use api::snapshot::{GameSnapshot, SceneView, WorldTile};
    use script::isolate_fb::NativeFactsInput;
    use script::native::HostEffect;

    fn bank_tick(slot: &mut bank_slot::BankSlot, snap: &SnapshotInput<'_>) {
        slot.tick_from_input(snap.tick, snap, NativeFactsInput::default());
    }

    let mut iso = bank_slot::BankSlot::new(
        src,
        LoadShape::CompatClass,
        None,
        std::sync::Arc::new(api::named_banks::NamedBankFacts::empty()),
    );
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
        size: 1,
        nx: 3251,
        nz: 3250,
        shape: 10,
        angle: 0,
    }];
    let mut snap = ingame_snapshot();
    snap.here = Some(tile(3222, 3222));
    snap.locs = &locs;
    bank_tick(&mut iso, &snap);
    let actions = iso.take_actions();
    assert!(
        matches!(actions.as_slice(), [action]
            if matches!(&action.effect, HostEffect::Walk(request)
                if request.target == (WorldTile { x: 3250, z: 3250, level: 0 }))),
        "Banking.open walks to the stand first"
    );
    let old = actions[0].authority();

    iso.reconnect_session_work();
    assert!(!old.live(), "the old session's native action is revoked");
    iso.slot.complete_native_walk(
        &old,
        script::native::WalkReceipt {
            request_id: old.request_id().get(),
            evidence: api::quest_progress::EvidenceStamp {
                run: old.run(),
                tick: 2,
                sequence: 2,
            },
            end: script::native::WalkEnd::Arrived,
            blocked: None,
            detail: None,
            refusal: None,
            assessment: None,
            escape: None,
        },
    );
    snap.here = Some(tile(3230, 3230));
    snap.hold = true;
    snap.tick = 2;
    bank_tick(&mut iso, &snap);
    assert!(
        iso.take_actions().is_empty(),
        "a held relog emits no follow"
    );
    snap.hold = false;
    snap.tick = 3;
    bank_tick(&mut iso, &snap);
    snap.tick = 4;
    bank_tick(&mut iso, &snap);
    assert_eq!(
        iso.probe("globalThis.__ok").unwrap(),
        serde_json::Value::Null
    );
    let resumed = iso.take_actions();
    let [follow] = resumed.as_slice() else {
        panic!("the saved bank walk is revalidated exactly once after the held relog");
    };
    assert!(matches!(
        &follow.effect,
        HostEffect::Walk(request)
            if request.target == (WorldTile { x: 3250, z: 3250, level: 0 })
    ));
    assert_ne!(follow.authority().run().session, old.run().session);
    assert!(
        !old.live(),
        "fresh follow authority cannot revive the old session"
    );

    // At the stand: fresh raw scene facts, not an echoed approach, open the chest.
    snap.tick = 5;
    snap.here = Some(tile(3250, 3250));
    let mut native = GameSnapshot::new();
    bank_slot::update_native_snapshot(&mut native, &snap, NativeFactsInput::default(), None);
    native.seed_scene(SceneView {
        available: true,
        base_x: 3250,
        base_z: 3250,
        level: 0,
        width: 2,
        height: 1,
        collision_flags: vec![0, 0],
    });
    iso.tick(5, &snap, NativeFactsInput::default(), &native);
    let opens = iso.take_actions();
    assert_eq!(opens.len(), 1, "exactly one chest open follows the relog");
    assert!(matches!(
        &opens[0].effect,
        HostEffect::Interaction(InteractReq::OpenBooth { id: 4483, .. })
    ));
    assert_ne!(opens[0].authority().run().session, old.run().session);
    snap.tick = 6;
    snap.bank_open = true;
    snap.bank_loaded = true;
    snap.bank_generation = 1;
    bank_tick(&mut iso, &snap);
    assert_eq!(iso.probe("globalThis.__ok").unwrap(), true);
    assert_eq!(iso.probe("globalThis.__settled").unwrap(), 1);
    snap.tick = 7;
    bank_tick(&mut iso, &snap);
    assert_eq!(iso.probe("globalThis.__settled").unwrap(), 1);
    assert!(iso.take_actions().is_empty());
    iso.stop();
}

#[test]
fn kept_bank_transfer_steps_take_the_closed_bank_path_without_replay() {
    use script::isolate_fb::{ItemRowInput, NativeFactsInput};
    use script::native::{HostEffect, InteractionReceipt};

    for (call, terminal_value) in [
        ("Bank.deposit('Bones')", serde_json::json!(false)),
        ("Bank.withdraw('Bones', 1)", serde_json::json!(false)),
        ("Bank.depositInventory()", serde_json::Value::Null),
    ] {
        let source = format!(
            r#"
import {{ Bank }} from '../../api/bank/Bank.js';
export default class T extends LoopingBot {{
    async loop() {{
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__settled = 0;
        const take = globalThis.__rs2b0t_machine_take;
        globalThis.__rs2b0t_machine_take = (handle) => {{
            const result = take(handle);
            if (result !== undefined) globalThis.__terminal = result;
            return result;
        }};
        globalThis.__ok = await {call};
        globalThis.__settled++;
    }}
}}
"#
        );
        let mut slot = bank_slot::BankSlot::new(
            source,
            LoadShape::CompatClass,
            None,
            std::sync::Arc::new(api::named_banks::NamedBankFacts::empty()),
        );
        let bank_ops = ["Withdraw-1".to_string()];
        let side_ops = ["Deposit-1".to_string(), "Deposit-All".to_string()];
        let bank = [ItemRowInput {
            id: 526,
            name: Some("Bones"),
            count: 40,
            ops: &bank_ops,
            noted: false,
            cert: -1,
            component_id: 3214,
            slot: 0,
        }];
        let side = [ItemRowInput {
            count: 1,
            ops: &side_ops,
            component_id: 701,
            ..bank[0]
        }];
        let mut snap = ingame_snapshot();
        snap.inv = &side;
        snap.inv_size = 28;
        snap.bank = &bank;
        snap.bank_side = &side;
        snap.bank_open = true;
        snap.bank_loaded = true;
        snap.bank_generation = 1;
        let native = NativeFactsInput {
            bank_snapshot_generation: Some(1),
            side_modal_id: Some(700),
            ..Default::default()
        };
        let mut actions = Vec::new();
        for tick in 1..=4 {
            snap.tick = tick;
            slot.tick_from_input(tick, &snap, native);
            actions.extend(slot.take_actions());
            if !actions.is_empty() {
                break;
            }
        }
        assert_eq!(actions.len(), 1, "{call}: one transfer is in flight");
        assert!(matches!(&actions[0].effect, HostEffect::Interaction(_)));
        assert_eq!(slot.probe("__settled").unwrap(), 0);
        let old = actions[0].authority();
        slot.reconnect_session_work();
        assert!(!old.live(), "{call}: a queued old effect cannot dispatch");
        slot.slot.complete_native_interaction(
            &old,
            InteractionReceipt {
                request_id: old.request_id().get(),
                evidence: api::quest_progress::EvidenceStamp {
                    run: old.run(),
                    tick: snap.tick + 1,
                    sequence: snap.tick + 1,
                },
                accepted: true,
                chat_since: -1,
            },
        );
        snap.bank_open = false;
        snap.bank_loaded = false;
        snap.bank = &[];
        snap.bank_side = &[];
        snap.hold = true;
        snap.tick += 1;
        slot.tick_from_input(snap.tick, &snap, Default::default());
        assert_eq!(slot.probe("__settled").unwrap(), 0);
        snap.hold = false;
        for _ in 0..4 {
            snap.tick += 1;
            slot.tick_from_input(snap.tick, &snap, Default::default());
            assert!(
                slot.take_actions().is_empty(),
                "{call}: no old verb is replayed"
            );
        }
        assert_eq!(slot.probe("__settled").unwrap(), 1, "{call}");
        assert_eq!(
            slot.probe("__terminal").unwrap(),
            serde_json::json!({"kind": "done", "value": terminal_value}),
            "{call}: the late accepted receipt cannot turn the closed-bank exit into success"
        );
        slot.stop();
    }
}

#[test]
fn non_kept_reset_aborts_bank_opening_and_settles_its_promise_once() {
    use script::native::HostEffect;

    let source = r#"
import { Banking } from '../../api/bank/Banking.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__settled = 0;
        const take = globalThis.__rs2b0t_machine_take;
        globalThis.__rs2b0t_machine_take = (handle) => {
            const result = take(handle);
            if (result !== undefined) globalThis.__terminal = result;
            return result;
        };
        globalThis.__ok = await Banking.open({
            stand: { x: 3250, z: 3250, level: 0 },
            boothName: 'Bank chest',
            boothOp: 'Bank',
        });
        globalThis.__settled++;
    }
}
"#;
    let mut slot = bank_slot::BankSlot::new(
        source,
        LoadShape::CompatClass,
        None,
        std::sync::Arc::new(api::named_banks::NamedBankFacts::empty()),
    );
    let mut snap = ingame_snapshot();
    snap.here = Some(tile(3222, 3222));
    slot.tick_from_input(1, &snap, Default::default());
    let actions = slot.take_actions();
    assert!(matches!(actions.as_slice(), [action] if matches!(action.effect, HostEffect::Walk(_))));
    let old = actions[0].authority();
    slot.reset_session_work();
    assert!(!old.live());
    for tick in 2..=5 {
        snap.tick = tick;
        slot.tick_from_input(tick, &snap, Default::default());
        assert!(
            slot.take_actions().is_empty(),
            "a non-kept opening must not resume"
        );
    }
    assert_eq!(
        slot.probe("__terminal").unwrap(),
        serde_json::json!({"kind": "aborted", "reason": "reset"})
    );
    assert_eq!(slot.probe("__ok").unwrap(), false);
    assert_eq!(slot.probe("__settled").unwrap(), 1);
    slot.stop();
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

    iso.reconnect_session_work(None);
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

    iso.reconnect_session_work(None);
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

/// Operator Pause and Resume while the reconnect still holds the script do
/// not restart its clocks: the hold alone keeps them stopped until the
/// relogged session's first tick.
#[test]
fn pause_and_resume_during_a_reconnect_hold_keep_the_wait_clock_stopped() {
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

    iso.reconnect_session_work(None);
    iso.pause();
    iso.resume();
    sleep(Duration::from_millis(900));
    let here = snap.here.unwrap();
    relog(&iso, &mut snap, 2, here);
    assert_eq!(
        value(&iso, "globalThis.__ok"),
        serde_json::Value::Null,
        "a Resume while still disconnected does not restart the clock"
    );
    iso.join();
}

const HELD_WALK: &str = r#"
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

/// A queued walk is discarded at a kept reconnect. The resumed wait makes
/// a fresh seat request, then settles on actual arrival.
#[test]
fn an_undrained_walk_is_discarded_on_kept_reconnect() {
    let iso = LoadIsolate::spawn(HELD_WALK.into(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = ingame_snapshot();
    snap.here = Some(tile(3222, 3222));
    tick(&iso, &snap);
    iso.reconnect_session_work(None);
    assert!(
        walks(&iso.drain_interacts()).is_empty(),
        "the old-session walk is discarded before the new tick"
    );
    let next = relog(&iso, &mut snap, 2, tile(3240, 3240));
    let after_relog = iso.drain_interacts();
    assert!(
        matches!(
            walks(&after_relog).as_slice(),
            [InteractReq::WalkNear {
                x: 3300,
                z: 3300,
                ..
            }]
        ),
        "the wait requests a fresh walk through the new seat: {after_relog:?}"
    );
    assert_eq!(value(&iso, "globalThis.__ok"), serde_json::Value::Null);
    snap.tick = next;
    snap.here = Some(tile(3300, 3301));
    tick(&iso, &snap);
    assert_eq!(value(&iso, "globalThis.__ok"), true, "the wait settles");
    iso.join();
}

/// A stale gameplay walk is discarded, but its AbortWalk release is folded
/// into the ordinary interaction drain so the host can stop its old follow.
#[test]
fn kept_reconnect_discards_stale_walk_and_drains_its_abort_release() {
    let src = r#"
import { EventSignal } from '../../api/execution/EventSignal.js';
import { Traversal } from '../../api/walking/Traversal.js';
export default class T extends LoopingBot {
    constructor() {
        super();
        globalThis.__ticks = 0;
        this.on('tick', () => { globalThis.__ticks += 1; });
        EventSignal.setInterrupt(() => globalThis.__ticks >= 2);
    }
    async loop() {
        if (globalThis.__ran) return;
        globalThis.__ran = true;
        globalThis.__result = await Traversal.walkTo(
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
    snap.tick = 2;
    tick(&iso, &snap);
    assert_eq!(value(&iso, "globalThis.__result"), false);

    iso.reconnect_session_work(None);
    relog(&iso, &mut snap, 3, tile(3240, 3240));
    let after_relog = iso.drain_interacts();
    assert!(
        matches!(
            after_relog.as_slice(),
            [InteractReq::AbortWalk { request_id }] if *request_id != 0
        ),
        "the prior-session walk is gone but its release reaches the normal host drain: \
         {after_relog:?}"
    );
    iso.join();
}

/// The first session's tick is still queued when a new session begins. The
/// bot's next tick is a fresh request and remains eligible for dispatch.
#[test]
fn a_fresh_walk_emitted_after_relog_is_admitted() {
    for _ in 0..8 {
        let iso = LoadIsolate::spawn(HELD_WALK.into(), LoadShape::CompatClass, vec![]).unwrap();
        let mut snap = ingame_snapshot();
        snap.here = Some(tile(3222, 3222));
        post_snapshot_input(&iso, &snap);
        iso.on_game_tick(1);
        iso.reconnect_session_work(None);
        relog(&iso, &mut snap, 2, tile(3240, 3240));
        let after_relog = iso.drain_interacts();
        assert!(
            matches!(
                walks(&after_relog).as_slice(),
                [InteractReq::WalkNear { x: 3300, z: 3300, request_id, .. }]
                    if *request_id != 0
            ),
            "a current-session walk remains admitted: {after_relog:?}"
        );
        iso.join();
    }
}

/// Two boundaries of one disconnect queued behind a busy isolate restate
/// the parked wait once, not once per reset.
#[test]
fn queued_resets_of_one_disconnect_restate_a_parked_wait_once() {
    let src = r#"
import { Execution } from '../../api/execution/Execution.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__ran) return;
        globalThis.__ran = true;
        await Execution.delayUntil(() => false, 0);
    }
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = ingame_snapshot();
    tick(&iso, &snap);
    let enqueued = |facts: &[InteractReq]| {
        facts
            .iter()
            .filter(|fact| matches!(fact, InteractReq::WaitEnqueued))
            .count()
    };
    assert_eq!(enqueued(&iso.drain_lifecycle()), 1, "one wait parked");

    std::thread::scope(|scope| {
        let busy = scope.spawn(|| {
            iso.probe("(() => { const t = Date.now() + 300; while (Date.now() < t) {} })()")
        });
        sleep(Duration::from_millis(50));
        iso.reconnect_session_work(None);
        iso.reconnect_session_work(None);
        busy.join().unwrap().unwrap();
    });
    let here = snap.here.unwrap();
    relog(&iso, &mut snap, 2, here);
    assert_eq!(
        enqueued(&iso.drain_lifecycle()),
        1,
        "the one parked wait is counted once"
    );
    iso.join();
}

/// A late old-generation tick is discarded. Its still-live walk wait issues
/// a new seat request rather than replaying the stale batch.
#[test]
fn a_walk_emitted_late_by_a_tick_crossing_the_drop_is_discarded() {
    let src = r#"
import { Traversal } from '../../api/walking/Traversal.js';
export default class T extends LoopingBot {
    async loop() {
        if (!globalThis.__armed) { globalThis.__armed = true; return; }
        if (globalThis.__ran) return;
        globalThis.__ran = true;
        const until = Date.now() + 300;
        while (Date.now() < until) {}
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
    assert!(walks(&iso.drain_interacts()).is_empty());

    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(2);
    wait_for_execution(&iso, "old-session tick never entered execution");
    iso.reconnect_session_work(None);
    let before_tick_finishes = iso.drain_interacts();
    assert!(
        walks(&before_tick_finishes).is_empty(),
        "the release fence does not expose old gameplay: {before_tick_finishes:?}"
    );

    let next = relog(&iso, &mut snap, 3, tile(3240, 3240));
    let mut after_relog = iso.drain_interacts();
    for offset in 0..4 {
        if !walks(&after_relog).is_empty() {
            break;
        }
        snap.tick = next + offset;
        tick(&iso, &snap);
        after_relog.extend(iso.drain_interacts());
    }
    assert!(
        matches!(
            walks(&after_relog).as_slice(),
            [InteractReq::WalkNear {
                x: 3300,
                z: 3300,
                ..
            }]
        ),
        "only the fresh new-session walk enters the seat: {after_relog:?}"
    );
    snap.tick = next + 4;
    snap.here = Some(tile(3300, 3301));
    tick(&iso, &snap);
    assert_eq!(value(&iso, "globalThis.__ok"), true);
    iso.join();
}
