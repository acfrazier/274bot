#![cfg(feature = "load")]

//! Locked V8 and Deno characterization for the current isolate.
//!
//! The Symbol in this fixture is a test-local context marker. Production does
//! not install a seat-lineage carrier, so these assertions prove CPED and
//! AsyncContext propagation, not product lineage attribution. Current API
//! session outcomes remain covered by the real LoadIsolate tests in
//! `api_gather_runtime.rs`.

use script::isolate_fb::{ChatLineInput, SnapshotInput, TileInput};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};

mod common;
use common::{ingame_snapshot, post_snapshot_input};

fn tile(x: i32, z: i32) -> TileInput {
    TileInput { x, z, level: 0 }
}

fn line(seq: i32, text: &'static str) -> ChatLineInput<'static> {
    ChatLineInput {
        seq,
        text,
        type_: 0,
        username: None,
    }
}

fn tick(iso: &LoadIsolate, snap: &SnapshotInput<'_>) {
    post_snapshot_input(iso, snap);
    iso.on_game_tick(snap.tick);
    let _ = iso.probe("true").unwrap();
}

fn value(iso: &LoadIsolate, expr: &str) -> serde_json::Value {
    iso.probe(expr).unwrap()
}

fn walks(reqs: &[InteractReq]) -> Vec<&InteractReq> {
    reqs.iter()
        .filter(|req| matches!(req, InteractReq::Walk { .. } | InteractReq::WalkNear { .. }))
        .collect()
}

fn wait_for_walk_back_child(
    iso: &LoadIsolate,
    snap: &mut SnapshotInput<'_>,
) -> (u64, Vec<InteractReq>) {
    let mut tick_id = 2;
    let mut out = Vec::new();
    while tick_id < 40 && walks(&out).is_empty() {
        snap.tick = tick_id;
        tick(iso, snap);
        out.extend(iso.drain_interacts());
        tick_id += 1;
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
        "the async hook starts one real child walk: {out:?}"
    );
    (tick_id, out)
}

fn relog(iso: &LoadIsolate, snap: &mut SnapshotInput<'_>, from: u64) -> u64 {
    snap.here = Some(tile(3230, 3230));
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
fn real_hook_continuation_keeps_derived_async_context_across_delay_child_and_reconnect() {
    let src = r#"
import { DeathRecovery } from '../../api/tasks/DeathRecovery.js';
import { Execution } from '../../api/execution/Execution.js';
import { Traversal } from '../../api/walking/Traversal.js';

const lineageKey = Symbol('test-only-lineage');
const parentContext = new Deno.core.AsyncVariable();
const nestedContext = new Deno.core.AsyncVariable();
globalThis.__testLineageKey = lineageKey;
globalThis.__testParentContext = parentContext;
globalThis.__testNestedContext = nestedContext;
globalThis.__sentinel = { kind: 'test-thrown-value' };
globalThis.__deaths = 0;
globalThis.__afterDelay = null;
globalThis.__predicate = null;
globalThis.__afterPredicate = null;
globalThis.__beforeChild = null;
globalThis.__afterChild = null;
globalThis.__sameThrown = false;
globalThis.__machineSameThrown = false;
globalThis.__recovered = false;

function readContext() {
    const current = Deno.core.getAsyncContext();
    return {
        carrier: current?.[lineageKey] ?? null,
        parent: parentContext.get() ?? null,
        nested: nestedContext.get() ?? null,
        nullPrototype: current != null && Object.getPrototypeOf(current) === null,
    };
}

function walkBack() {
    const previous = Deno.core.getAsyncContext();
    const derived = Object.assign(
        Object.create(null),
        previous ?? {},
        { [lineageKey]: 'test-hook-token' },
    );
    Deno.core.scopeAsyncContext(derived);
    const previousParent = parentContext.enter('outer-hook');
    const previousNested = nestedContext.enter('nested-hook');

    let pending;
    try {
        pending = (async () => {
            try {
                await Execution.delayTicks(1);
                globalThis.__afterDelay = readContext();

                await Execution.delayUntil(() => {
                    globalThis.__predicate = readContext();
                    return true;
                }, 3000);
                globalThis.__afterPredicate = readContext();

                globalThis.__beforeChild = readContext();
                const arrived = await Traversal.walkResilient(
                    { x: 3300, z: 3300, level: 0 },
                    { radius: 1, timeoutMs: 300000 },
                );
                globalThis.__afterChild = { arrived, ...readContext() };
                throw globalThis.__sentinel;
            } catch (thrown) {
                globalThis.__sameThrown = thrown === globalThis.__sentinel;
                throw thrown;
            } finally {
                Deno.core.setAsyncContext(previousNested);
                Deno.core.setAsyncContext(previousParent);
                Deno.core.setAsyncContext(previous);
            }
        })();
    } finally {
        // Leave the synchronous hook call as a root context. The wait and
        // async-function reactions already captured the derived mapping.
        Deno.core.setAsyncContext(previous);
    }
    return pending;
}

export default class T extends TaskBot {
    onStart() {
        const recovery = new DeathRecovery(this, {
            anchor: { x: 3300, z: 3300, level: 0 },
            radius: 6,
            onDeath: () => { globalThis.__deaths += 1; },
            onRecovered: () => { globalThis.__recovered = true; },
            walkBack,
        });
        const execute = recovery.execute.bind(recovery);
        recovery.execute = async (...args) => {
            try {
                return await execute(...args);
            } catch (thrown) {
                globalThis.__machineSameThrown = thrown === globalThis.__sentinel;
                throw thrown;
            }
        };
        this.add(recovery);
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
    let (last_tick, mut out) = wait_for_walk_back_child(&iso, &mut snap);
    assert_eq!(
        value(&iso, "globalThis.__afterDelay"),
        serde_json::json!({
            "carrier": "test-hook-token",
            "parent": "outer-hook",
            "nested": "nested-hook",
            "nullPrototype": true,
        }),
        "the actual Execution.delayTicks continuation retains the derived Deno mapping"
    );
    assert_eq!(
        value(&iso, "globalThis.__predicate"),
        serde_json::json!({
            "carrier": null,
            "parent": null,
            "nested": null,
            "nullPrototype": false,
        }),
        "the scheduler's synchronous delayUntil predicate runs outside the hook continuation"
    );
    assert_eq!(
        value(&iso, "globalThis.__afterPredicate"),
        serde_json::json!({
            "carrier": "test-hook-token",
            "parent": "outer-hook",
            "nested": "nested-hook",
            "nullPrototype": true,
        }),
        "the hook continuation restores its captured Deno mapping after the predicate"
    );
    assert_eq!(
        value(&iso, "globalThis.__beforeChild"),
        value(&iso, "globalThis.__afterPredicate")
    );

    // This is a real current in-isolate machine child, not a seat admission.
    // Keep the callback and child walk pending across the actual kept reset.
    iso.reconnect_session_work(None);
    let next = relog(&iso, &mut snap, last_tick + 1);
    assert!(
        walks(&iso.drain_interacts()).is_empty(),
        "kept child walk is not restarted"
    );
    assert_eq!(
        value(&iso, "globalThis.__afterChild"),
        serde_json::Value::Null
    );

    snap.here = Some(tile(3300, 3300));
    snap.tick = next;
    tick(&iso, &snap);
    for offset in 1..=4 {
        snap.tick = next + offset;
        tick(&iso, &snap);
        out.extend(iso.drain_interacts());
    }
    assert_eq!(
        walks(&out).len(),
        1,
        "the child walk was emitted once: {out:?}"
    );
    assert_eq!(
        value(&iso, "globalThis.__afterChild"),
        serde_json::json!({
            "arrived": true,
            "carrier": "test-hook-token",
            "parent": "outer-hook",
            "nested": "nested-hook",
            "nullPrototype": true,
        }),
        "the real runMachine child continuation retains the test mapping through reconnect"
    );
    assert_eq!(value(&iso, "globalThis.__sameThrown"), true);
    assert_eq!(value(&iso, "globalThis.__deaths"), 1);
    assert_eq!(
        value(&iso, "globalThis.__machineSameThrown"),
        true,
        "the actual DeathRecovery execute rejects with the awaited hook's thrown value"
    );
    assert_eq!(
        value(&iso, "globalThis.__recovered"),
        true,
        "onRecovered is an independent validate observation of the actual arrival, not execute success"
    );
    assert_eq!(
        value(
            &iso,
            "({ carrier: Deno.core.getAsyncContext()?.[globalThis.__testLineageKey] ?? null, parent: globalThis.__testParentContext.get() ?? null, nested: globalThis.__testNestedContext.get() ?? null })",
        ),
        serde_json::json!({ "carrier": null, "parent": null, "nested": null }),
        "the explicit async scope restores the root probe after the rejection"
    );
    iso.join();
}
