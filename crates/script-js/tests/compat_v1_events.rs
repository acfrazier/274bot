//! Compat v1 Events census contracts and removal coverage.

mod common;

use script::native::snapshot_source::{SnapshotInput, StatInput};
use script_js::{LoadIsolate, LoadShape};
use serde_json::{json, Value};

const INDEX_DTS: &str = include_str!("../compat-js/index.d.ts");
const ABI_CONTRACT: &str = include_str!("../compat-js/abi-contract.json");

fn prayer_xp(xp: i32) -> [StatInput<'static>; 1] {
    [StatInput {
        index: 5,
        name: "prayer",
        xp,
        base: 2,
        effective: 2,
    }]
}

fn post_xp_tick(isolate: &LoadIsolate, tick: u64, xp: i32) {
    let stats = prayer_xp(xp);
    let snapshot = SnapshotInput {
        tick,
        stats: &stats,
        ..common::ingame_snapshot()
    };
    common::post_snapshot_input(isolate, &snapshot);
    isolate.on_game_tick(tick);
    let _ = isolate.probe("true");
}

fn probe(isolate: &LoadIsolate, expression: &str) -> Value {
    isolate
        .probe(expression)
        .unwrap_or_else(|error| panic!("event probe {expression} failed: {error}"))
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_events__events_on__contract() {
    let source = r#"
import { events } from '@rs2b0t/api';
export default class T extends LoopingBot {
    onStart() {
        globalThis.__order = [];
        globalThis.__payloads = [];
        const first = () => {
            globalThis.__order.push('first');
            throw new Error('isolated listener failure');
        };
        const second = (event) => {
            globalThis.__order.push('second');
            globalThis.__payloads.push({
                skill: event.skill,
                name: event.name,
                xp: event.xp,
                delta: event.delta,
            });
        };
        const instance = () => globalThis.__order.push('instance');
        globalThis.__unsubscribe = events.on('skill.xp', first);
        globalThis.__duplicate = events.on('skill.xp', first);
        events.on('skill.xp', second);
        this.on('skill.xp', second);
        this.on('skill.xp', instance);
        globalThis.__onShape = {
            ownOn: Object.hasOwn(events, 'on'),
            ownOff: Object.hasOwn(events, 'off'),
            on: typeof events.on === 'function',
            off: typeof events.off === 'function',
            unsubscribe: typeof globalThis.__unsubscribe === 'function',
            duplicate: typeof globalThis.__duplicate === 'function',
            instance: typeof this.on === 'function',
        };
    }
    loop() {}
}
"#;
    let isolate = LoadIsolate::spawn(source.into(), LoadShape::CompatClass, vec![])
        .expect("events.on source links");
    post_xp_tick(&isolate, 1, 112);
    assert_eq!(
        probe(&isolate, "globalThis.__onShape"),
        json!({
            "ownOn": true, "ownOff": true, "on": true, "off": true,
            "unsubscribe": true, "duplicate": true, "instance": true,
        })
    );

    post_xp_tick(&isolate, 2, 224);
    assert_eq!(
        probe(&isolate, "globalThis.__order"),
        json!(["first", "second", "instance"]),
        "duplicate callback identity is delivered once and module/instance subscriptions share ordered dispatch despite a throw"
    );
    assert_eq!(
        probe(&isolate, "globalThis.__payloads"),
        json!([{"skill": 5, "name": "prayer", "xp": 224, "delta": 112}])
    );
    assert_eq!(
        probe(&isolate, "(globalThis.__unsubscribe(), true)"),
        json!(true),
        "the returned unsubscriber can be invoked"
    );

    post_xp_tick(&isolate, 3, 336);
    assert_eq!(
        probe(&isolate, "globalThis.__order"),
        json!(["first", "second", "instance", "second", "instance"]),
        "the unsubscriber removes only its event/callback pair while instance listeners remain ordered"
    );
    assert_eq!(
        probe(&isolate, "globalThis.__payloads"),
        json!([
            {"skill": 5, "name": "prayer", "xp": 224, "delta": 112},
            {"skill": 5, "name": "prayer", "xp": 336, "delta": 112}
        ])
    );
    isolate.join();
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_events__events_off__contract() {
    let source = r#"
import { events } from '@rs2b0t/api';
export default class T extends LoopingBot {
    onStart() {
        globalThis.__count = 0;
        const listener = () => { globalThis.__count++; };
        events.on('skill.xp', listener);
        events.off('skill.xp', () => {});
        globalThis.__remove = () => events.off('skill.xp', listener) === undefined;
        globalThis.__restore = () => { events.on('skill.xp', listener); };
    }
    loop() {}
}
"#;
    let isolate = LoadIsolate::spawn(source.into(), LoadShape::CompatClass, vec![])
        .expect("events.off source links");
    post_xp_tick(&isolate, 1, 112);
    post_xp_tick(&isolate, 2, 224);
    assert_eq!(
        probe(&isolate, "globalThis.__count"),
        json!(1),
        "wrong callback is a no-op"
    );
    assert_eq!(probe(&isolate, "globalThis.__remove()"), json!(true));
    post_xp_tick(&isolate, 3, 336);
    assert_eq!(
        probe(&isolate, "globalThis.__count"),
        json!(1),
        "off removes the exact pair"
    );
    probe(&isolate, "globalThis.__restore() === undefined");
    post_xp_tick(&isolate, 4, 448);
    assert_eq!(
        probe(&isolate, "globalThis.__count"),
        json!(2),
        "removed listeners can be re-added"
    );
    isolate.join();
}

#[test]
#[expect(non_snake_case, reason = "the census names this removal test")]
fn compat_v1_events__eventsignal_ignoredrandoms__removed() {
    let declaration = INDEX_DTS
        .split("export const EventSignal: {")
        .nth(1)
        .expect("EventSignal declaration exists")
        .split("};")
        .next()
        .expect("EventSignal declaration closes");
    assert!(!declaration.contains("ignoredRandoms"));
    assert!(!ABI_CONTRACT.contains("EventSignal.ignoredRandoms"));
    assert!(!ABI_CONTRACT.contains("\"id\": \"event-ignored-randoms\""));
    assert!(
        INDEX_DTS.contains("ignoredRandoms(): string[];"),
        "LoopingBot sibling remains declared"
    );

    let source = r#"
import { EventSignal } from '../../api/execution/EventSignal.js';
const key = 'ignoredRandoms';
const read = (target, name) => {
    try { return { ok: true, value: target[name] }; }
    catch (error) { return { ok: false, message: error.message }; }
};
export default class T extends LoopingBot {
    loop() {
        EventSignal.setInterrupt(() => true);
        const interrupted = EventSignal.pending();
        EventSignal.setInterrupt(null);
        const plain = {};
        globalThis.__probe = {
            own: Object.hasOwn(EventSignal, key),
            descriptor: Object.getOwnPropertyDescriptor(EventSignal, key) === undefined,
            key: Object.keys(EventSignal).includes(key),
            read: read(EventSignal, key),
            unknown: read(EventSignal, '__compat_census_unknown'),
            plain: plain[key] === undefined,
            interrupted,
            cleared: EventSignal.pending() === false,
            siblings: typeof EventSignal.pending === 'function' && typeof EventSignal.setInterrupt === 'function',
        };
    }
}
"#;
    let isolate = LoadIsolate::spawn(source.into(), LoadShape::CompatClass, vec![])
        .expect("EventSignal removal probe links");
    let snapshot = common::ingame_snapshot();
    common::post_snapshot_input(&isolate, &snapshot);
    isolate.on_game_tick(1);
    let result = isolate.probe("globalThis.__probe");
    let logs = isolate.join();
    assert_eq!(
        result.unwrap_or_else(|error| panic!("EventSignal probe failed: {error}; logs: {logs:#?}")),
        json!({
            "own": false,
            "descriptor": true,
            "key": false,
            "read": {"ok": false, "message": "not impl: EventSignal.ignoredRandoms"},
            "unknown": {"ok": false, "message": "not impl: EventSignal.__compat_census_unknown"},
            "plain": true,
            "interrupted": true,
            "cleared": true,
            "siblings": true,
        })
    );
}
