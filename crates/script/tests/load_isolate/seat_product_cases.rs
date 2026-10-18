use super::*;
use api::script_contract::{AbortReason, CallbackCancelReason, ChildOrigin};
use script::api_progress::ProgressPage;
use script::isolate_fb::{
    NativeFactsInput, SeatLiveDetails, SeatPageInput, SeatResultValue, SeatStartArgs,
    SeatStartInput, TileInput,
};
use script::load::QueuedSeatBatch;
use script::native::request::NativeKind;
use script::native::seat::{SeatEntry, SeatResult, SeatTerminal};
use script::shim::InteractReq;
use std::sync::Arc;
fn seat_execution() -> api::identity::ExecutionId {
    api::identity::ExecutionId::first(api::selected::RunKey {
        slot: 94,
        run: 1,
        session: 1,
    })
}

fn spawn_owned_ready(js: String, shape: LoadShape, siblings: Vec<(String, String)>) -> LoadIsolate {
    let iso = LoadIsolate::spawn_with_execution(js, shape, siblings, seat_execution()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match iso.poll_ready() {
            script::Ready::Ready => return iso,
            script::Ready::Failed(error) => panic!("owner-bound isolate setup failed: {error}"),
            script::Ready::Pending if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(2));
            }
            script::Ready::Pending => panic!("owner-bound isolate setup did not finish"),
        }
    }
}
fn root_seat_entry(id: api::identity::RequestId, kind: NativeKind) -> SeatEntry {
    SeatEntry {
        id,
        parent: None,
        kind,
        origin: ChildOrigin::Started,
        depth: 1,
        decisions_consumed: 0,
        prompt: None,
    }
}

fn root_seat_details(id: api::identity::RequestId) -> SeatLiveDetails<'static> {
    SeatLiveDetails {
        id,
        phase: script::native::NativePhase::Preparing,
        prompt: None,
    }
}

fn post_test_tick_with_native(
    iso: &LoadIsolate,
    snapshot: &mut script::isolate_fb::SnapshotInput<'_>,
    tick: u64,
    native: NativeFactsInput<'_>,
) {
    snapshot.tick = tick;
    iso.post_snapshot(script::isolate_fb::encode_snapshot_with_native(
        snapshot, native,
    ));
    iso.on_game_tick(tick);
    let _ = iso
        .probe("true")
        .expect("posted owner-bound product tick completes");
}

fn post_test_tick(
    iso: &LoadIsolate,
    snapshot: &mut script::isolate_fb::SnapshotInput<'_>,
    tick: u64,
) {
    snapshot.tick = tick;
    post_snapshot_input(iso, snapshot);
    iso.on_game_tick(tick);
    let _ = iso.probe("true").expect("posted product tick completes");
}

fn seat_reqs(
    batches: &[QueuedSeatBatch],
) -> impl Iterator<Item = &script::isolate_fb::SeatInteractDecoded> {
    batches.iter().flat_map(|batch| batch.batch.reqs.iter())
}

fn seat_starts(batches: &[QueuedSeatBatch]) -> impl Iterator<Item = &SeatStartInput> {
    batches.iter().flat_map(|batch| batch.batch.starts.iter())
}

fn assert_bank_start_operation(start: &SeatStartInput, operation: &str) {
    let SeatStartArgs::Bank(args) = &start.args else {
        panic!("bank starts require typed bank arguments: {:?}", start.args);
    };
    assert!(
        format!("{args:?}").contains(&format!("operation: {operation},")),
        "expected bank operation {operation}, got {args:?}"
    );
}

#[test]
fn case_02_06_07_21_leave_awaits_real_walk_and_bank_children() {
    let src = r#"
import { bankRoutine } from '../../api/combat/hunting/combat.js';
import { Banking } from '../../api/bank/Banking.js';
import { Execution } from '../../api/execution/Execution.js';
import { Traversal } from '../../api/walking/Traversal.js';

export const apiVersion = 2;
const site = { boxes: [{ minX: 3220, maxX: 3224, minZ: 3220, maxZ: 3224, level: 0 }] };
const contextValue = new Deno.core.AsyncVariable();
const target = { x: 3300, z: 3300, level: 0 };
let started = false;
globalThis.__leaveOrder = [];
globalThis.__rootDone = false;
globalThis.__siteBefore = JSON.stringify(site);
globalThis.__bankAndWalk = null;
globalThis.__contextAfterDelay = null;
globalThis.__contextAfterBank = null;
globalThis.__siteUnchanged = null;

export function tick(api) {
    if (started) return;
    started = true;
    const root = bankRoutine(globalThis.__rs2b0t_host, site, {
        leave: () => {
            const previous = Deno.core.getAsyncContext();
            const derived = Object.assign(Object.create(null), previous ?? {});
            Deno.core.scopeAsyncContext(derived);
            contextValue.enter('leave-context');
            let pending;
            try {
                pending = (async () => {
                    globalThis.__leaveOrder.push('leave');
                    await Execution.delayTicks(1);
                    globalThis.__leaveOrder.push('after-delay');
                    globalThis.__contextAfterDelay = contextValue.get() ?? null;
                    Execution.noteProgress();

                    const walk = Traversal.walkResilient(target, { radius: 1, timeoutMs: 300000 })
                        .then((arrived) => {
                            globalThis.__leaveOrder.push('walk');
                            return arrived;
                        });
                    const bank = Banking.open()
                        .then((opened) => {
                            globalThis.__leaveOrder.push('bank');
                            globalThis.__contextAfterBank = contextValue.get() ?? null;
                            Execution.noteProgress();
                            return opened;
                        });
                    const results = await Promise.all([walk, bank]);
                    globalThis.__leaveOrder.push('all');
                    globalThis.__bankAndWalk = results;
                    globalThis.__siteUnchanged = JSON.stringify(site) === globalThis.__siteBefore;
                    return results.every(Boolean) && contextValue.get() === 'leave-context';
                })();
            } finally {
                Deno.core.setAsyncContext(previous);
            }
            return pending;
        },
    });
    root.then(() => {
        globalThis.__leaveOrder.push('root');
        globalThis.__rootDone = true;
    });
}
"#;
    let iso = spawn_owned_ready(src.to_string(), LoadShape::NativeTick, vec![]);
    let mut snapshot = ready_snapshot();
    snapshot.bank_open = true;
    snapshot.bank_loaded = true;
    snapshot.bank_generation = 1;

    // These protocol, lineage, and async-context fixtures acknowledge host pages explicitly.
    // Bank execution behavior is covered separately by the BankSlot fixtures.
    let mut bank_open_id = None;
    let mut bank_live = false;
    let mut bank_completed = false;
    for tick in 1..=12 {
        if tick >= 4 {
            snapshot.here = Some(TileInput {
                x: 3300,
                z: 3300,
                level: 0,
            });
        }
        if let Some(id) = bank_open_id.filter(|_| !bank_live) {
            let live = [root_seat_entry(id, NativeKind::BankPlan)];
            let details = [SeatLiveDetails {
                id,
                phase: script::native::NativePhase::Waiting,
                prompt: None,
            }];
            let seat_page = SeatPageInput::new(&live, &[], &details, 0).unwrap();
            post_test_tick_with_native(
                &iso,
                &mut snapshot,
                tick,
                NativeFactsInput {
                    seat_page: Some(seat_page),
                    ..NativeFactsInput::default()
                },
            );
            bank_live = true;
        } else if let Some(id) = bank_open_id.filter(|_| !bank_completed) {
            let terminal = [SeatResult {
                id,
                seq: 1,
                outcome: SeatTerminal::Done,
                prior_parent: None,
                session: Some(id),
                authority_revoked: false,
            }];
            let value = serde_json::Value::Bool(true);
            let encoded_value = script::channel::encode(&value).unwrap();
            let values = [SeatResultValue {
                id,
                value: &encoded_value,
            }];
            let seat_page = SeatPageInput::new(&[], &terminal, &[], 0)
                .unwrap()
                .with_result_values(&values)
                .unwrap();
            post_test_tick_with_native(
                &iso,
                &mut snapshot,
                tick,
                NativeFactsInput {
                    seat_page: Some(seat_page),
                    ..NativeFactsInput::default()
                },
            );
            bank_completed = true;
        } else {
            post_test_tick(&iso, &mut snapshot, tick);
        }
        let tick_batches = iso.drain_seat_batches();
        if bank_open_id.is_none() {
            if let Some(start) = seat_starts(&tick_batches)
                .find(|start| start.kind == NativeKind::BankPlan)
            {
                assert_bank_start_operation(start, "Open");
                bank_open_id = Some(start.id);
            }
        }
        batches.extend(tick_batches);
        if iso.probe("globalThis.__rootDone").unwrap() == true {
            break;
        }
    }

    assert_eq!(iso.probe("globalThis.__rootDone").unwrap(), true);
    assert_eq!(
        iso.probe("globalThis.__bankAndWalk").unwrap(),
        serde_json::json!([true, true])
    );
    assert_eq!(
        iso.probe("globalThis.__contextAfterDelay").unwrap(),
        "leave-context"
    );
    assert_eq!(
        iso.probe("globalThis.__contextAfterBank").unwrap(),
        "leave-context"
    );
    assert_eq!(iso.probe("globalThis.__siteUnchanged").unwrap(), true);
    let bank_open = seat_starts(&batches)
        .find(|start| start.kind == NativeKind::BankPlan)
        .expect("Banking.open starts a typed bank seat");
    assert_bank_start_operation(bank_open, "Open");
    let order = iso.probe("globalThis.__leaveOrder").unwrap();
    let order = order.as_array().unwrap();
    let position = |name: &str| order.iter().position(|item| item == name).unwrap();
    assert!(position("after-delay") < position("bank"));
    assert!(position("bank") < position("walk"));
    assert!(position("walk") < position("all"));
    assert!(position("all") < position("root"));

    let note_rows: Vec<_> = seat_reqs(&batches)
        .filter(|row| matches!(&row.request, InteractReq::NoteProgress))
        .collect();
    assert_eq!(
        note_rows.len(),
        2,
        "the hook pushes before and after Banking.open"
    );
    assert!(note_rows.iter().all(|row| row.lineage.is_none()));
    let owner = note_rows[0]
        .mirror_owner
        .expect("the hook's real product pushes retain their local mirror owner");
    assert!(owner.get() > 0);
    assert_eq!(note_rows[1].mirror_owner, Some(owner));

    let walk = seat_reqs(&batches)
        .find(|row| {
            matches!(
                &row.request,
                InteractReq::WalkNear {
                    x: 3300,
                    z: 3300,
                    ..
                }
            )
        })
        .expect("Traversal.walkResilient emits its real WalkNear row");
    assert!(walk.lineage.is_none());
    assert!(walk.mirror_owner.is_some());
    iso.join();
}

#[test]
fn case_08_17_native_root_tick_listener_delay_until_and_import_stay_rooted() {
    let src = r#"
import { BotHost } from '../../runtime/BotHost.js';
import { Execution } from '../../api/execution/Execution.js';
export const apiVersion = 2;
let started = false;
globalThis.__listenerRan = false;
globalThis.__delayTickRan = false;
globalThis.__delayUntilRan = false;
globalThis.__rootPushes = [];
globalThis.__importStarted = false;
globalThis.__rootProgress = null;

function push(api, name) {
    api.request({ op: 'held', name, action: 'Bury' });
    globalThis.__rootPushes.push(name);
}

export function tick(api) {
    if (started) return;
    started = true;
    push(api, 'tick-root');
    globalThis.__rootProgress = api.questProgress({ quest: 'tick-root' });
    BotHost.addTickListener(() => {
        if (!globalThis.__listenerRan && api.tick >= 2) {
            globalThis.__listenerRan = true;
            push(api, 'listener-root');
        }
    });
    Execution.delayTicks(1).then(() => {
        globalThis.__delayTickRan = true;
        push(api, 'delay-ticks-root');
    });
    Execution.delayUntil(() => api.tick >= 3, 5000).then(() => {
        globalThis.__delayUntilRan = true;
        push(api, 'delay-until-root');
    });
    import('../../api/bank/Banking.js').then(({ Banking }) => {
        globalThis.__importStarted = typeof Banking.open === 'function';
        push(api, 'import-root');
    });
}
"#;
    let iso = spawn_owned_ready(src.to_string(), LoadShape::NativeTick, vec![]);
    let mut snapshot = ready_snapshot();
    snapshot.bank_open = true;
    snapshot.bank_loaded = true;
    snapshot.bank_generation = 1;
    let mut batches = Vec::new();
    post_test_tick(&iso, &mut snapshot, 1);
    batches.extend(iso.drain_seat_batches());
    let progress_id = seat_starts(&batches)
        .find(|start| start.kind == NativeKind::QuestProgress)
        .expect("the root API call creates a typed seat start")
        .id;
    let live = [root_seat_entry(progress_id, NativeKind::QuestProgress)];
    let details = [root_seat_details(progress_id)];
    let seat_page = SeatPageInput::new(&live, &[], &details, 0).unwrap();
    post_test_tick_with_native(
        &iso,
        &mut snapshot,
        2,
        NativeFactsInput {
            seat_page: Some(seat_page),
            ..NativeFactsInput::default()
        },
    );
    batches.extend(iso.drain_seat_batches());
    for tick in 3..=5 {
        post_test_tick(&iso, &mut snapshot, tick);
        batches.extend(iso.drain_seat_batches());
    }

    assert_eq!(iso.probe("globalThis.__importStarted").unwrap(), true);
    let pushes = iso.probe("globalThis.__rootPushes").unwrap();
    let pushes = pushes.as_array().unwrap();
    assert_eq!(pushes.len(), 5, "root pushes: {pushes:?}");
    for name in [
        "tick-root",
        "listener-root",
        "delay-ticks-root",
        "delay-until-root",
        "import-root",
    ] {
        assert!(
            pushes.contains(&serde_json::json!(name)),
            "missing root push {name}: {pushes:?}"
        );
    }
    assert_eq!(
        iso.probe(
            "globalThis.__listenerRan && globalThis.__delayTickRan && globalThis.__delayUntilRan"
        )
        .unwrap(),
        true
    );

    let root_rows: Vec<_> = seat_reqs(&batches)
        .filter(|row| matches!(&row.request, InteractReq::Held { .. }))
        .collect();
    assert_eq!(root_rows.len(), 5);
    for row in &root_rows {
        assert_eq!(
            row.lineage, None,
            "root tick continuations never inherit a seat parent"
        );
        assert_eq!(
            row.mirror_owner, None,
            "direct API pushes have no local machine owner"
        );
    }
    let progress = seat_starts(&batches)
        .find(|start| start.kind == NativeKind::QuestProgress)
        .expect("the real root questProgress API starts its native family");
    assert_eq!(progress.parent, None);
    assert!(matches!(&progress.args, SeatStartArgs::QuestProgress(name) if name == "tick-root"));
    let progress_id = progress.id;
    assert!(seat_starts(&batches)
        .filter(|start| start.kind == NativeKind::QuestProgress)
        .all(|start| start.parent.is_none()));
    assert!(progress_id.get() > 0);
    iso.join();
}

#[test]
fn case_18_async_variable_scope_and_foreign_replacement_are_observed_once() {
    let src = r#"
import { bankRoutine } from '../../api/combat/hunting/combat.js';
import { Banking } from '../../api/bank/Banking.js';
import { Execution } from '../../api/execution/Execution.js';
export const apiVersion = 2;
const site = { boxes: [{ minX: 3220, maxX: 3224, minZ: 3220, maxZ: 3224, level: 0 }] };
const value = new Deno.core.AsyncVariable();
let started = false;
globalThis.__asyncAfterDelay = null;
globalThis.__foreignValue = 'unset';
globalThis.__restoredValue = null;
globalThis.__foreignCalls = null;
export function tick(api) {
    if (started) return;
    started = true;
    bankRoutine(globalThis.__rs2b0t_host, site, {
        leave: () => {
            const previous = Deno.core.getAsyncContext();
            const derived = Object.assign(Object.create(null), previous ?? {});
            Deno.core.scopeAsyncContext(derived);
            value.enter('derived-hook');
            let pending;
            try {
                pending = (async () => {
                    await Execution.delayTicks(1);
                    globalThis.__asyncAfterDelay = value.get() ?? null;
                    Execution.noteProgress();
                    const inherited = Deno.core.getAsyncContext();
                    Deno.core.setAsyncContext(Object.create(null));
                    globalThis.__foreignValue = value.get() ?? null;
                    const first = await Banking.open();
                    const second = await Banking.open();
                    Deno.core.setAsyncContext(inherited);
                    globalThis.__restoredValue = value.get() ?? null;
                    globalThis.__foreignCalls = [first, second];
                    return true;
                })();
            } finally {
                Deno.core.setAsyncContext(previous);
            }
            return pending;
        },
    });
}
"#;
    let iso = spawn_owned_ready(src.to_string(), LoadShape::NativeTick, vec![]);
    let mut snapshot = ready_snapshot();
    snapshot.bank_open = true;
    snapshot.bank_loaded = true;
    snapshot.bank_generation = 1;
    // This protocol and async-context fixture acknowledges host pages explicitly.
    // Bank execution behavior is covered separately by the BankSlot fixtures.
    let mut bank_open_id = None;
    let mut bank_live = false;
    let mut next_terminal_seq = 1;
    for tick in 1..=8 {
        if let Some(id) = bank_open_id {
            if !bank_live {
                let live = [root_seat_entry(id, NativeKind::BankPlan)];
                let details = [SeatLiveDetails {
                    id,
                    phase: script::native::NativePhase::Waiting,
                    prompt: None,
                }];
                let seat_page = SeatPageInput::new(&live, &[], &details, 0).unwrap();
                post_test_tick_with_native(
                    &iso,
                    &mut snapshot,
                    tick,
                    NativeFactsInput {
                        seat_page: Some(seat_page),
                        ..NativeFactsInput::default()
                    },
                );
                bank_live = true;
            } else {
                let terminal = [SeatResult {
                    id,
                    seq: next_terminal_seq,
                    outcome: SeatTerminal::Done,
                    prior_parent: None,
                    session: Some(id),
                    authority_revoked: false,
                }];
                let value = serde_json::Value::Bool(true);
                let encoded_value = script::channel::encode(&value).unwrap();
                let values = [SeatResultValue {
                    id,
                    value: &encoded_value,
                }];
                let seat_page = SeatPageInput::new(&[], &terminal, &[], 0)
                    .unwrap()
                    .with_result_values(&values)
                    .unwrap();
                post_test_tick_with_native(
                    &iso,
                    &mut snapshot,
                    tick,
                    NativeFactsInput {
                        seat_page: Some(seat_page),
                        ..NativeFactsInput::default()
                    },
                );
                bank_open_id = None;
                bank_live = false;
                next_terminal_seq += 1;
            }
        } else {
            post_test_tick(&iso, &mut snapshot, tick);
        }
        let tick_batches = iso.drain_seat_batches();
        if bank_open_id.is_none() {
            if let Some(start) = seat_starts(&tick_batches)
                .find(|start| start.kind == NativeKind::BankPlan)
            {
                assert_bank_start_operation(start, "Open");
                bank_open_id = Some(start.id);
            }
        }
        batches.extend(tick_batches);
    }

    assert_eq!(
        iso.probe("globalThis.__asyncAfterDelay").unwrap(),
        "derived-hook"
    );
    assert_eq!(
        iso.probe("globalThis.__foreignValue").unwrap(),
        serde_json::Value::Null
    );
    assert_eq!(
        iso.probe("globalThis.__restoredValue").unwrap(),
        "derived-hook"
    );
    assert_eq!(
        iso.probe("globalThis.__foreignCalls").unwrap(),
        serde_json::json!([true, true])
    );
    let notes: Vec<_> = seat_reqs(&batches)
        .filter(|row| matches!(&row.request, InteractReq::NoteProgress))
        .collect();
    assert_eq!(notes.len(), 1);
    assert!(notes[0].lineage.is_none());
    assert!(notes[0].mirror_owner.is_some());

    let foreign_log = "foreign async context encountered; interact ancestry falls back to the active hook or root";
    let count = iso
        .drain_logs()
        .iter()
        .filter(|line| line.as_str() == foreign_log)
        .count();
    assert_eq!(
        count, 1,
        "Foreign replacement is diagnosed once per isolate run"
    );
    iso.join();
}

#[test]
fn case_19_hook_microtask_keeps_local_mirror_while_loop_microtask_is_root() {
    let src = r#"
import { bankRoutine } from '../../api/combat/hunting/combat.js';
import { Execution } from '../../api/execution/Execution.js';
export const apiVersion = 2;
const site = { boxes: [{ minX: 3220, maxX: 3224, minZ: 3220, maxZ: 3224, level: 0 }] };
let started = false;
globalThis.__loopMicrotask = false;
globalThis.__hookMicrotask = false;
export function tick(api) {
    if (started) return;
    started = true;
    Execution.noteProgress();
    queueMicrotask(() => {
        Execution.noteProgress();
        globalThis.__loopMicrotask = true;
    });
    bankRoutine(globalThis.__rs2b0t_host, site, {
        leave: () => new Promise((resolve) => {
            queueMicrotask(() => {
                Execution.noteProgress();
                globalThis.__hookMicrotask = true;
                resolve(true);
            });
        }),
    });
}
"#;
    let iso = spawn_owned_ready(src.to_string(), LoadShape::NativeTick, vec![]);
    let mut snapshot = ready_snapshot();
    post_test_tick(&iso, &mut snapshot, 1);
    let batches = iso.drain_seat_batches();

    assert_eq!(
        iso.probe("globalThis.__loopMicrotask && globalThis.__hookMicrotask")
            .unwrap(),
        true
    );
    let notes: Vec<_> = seat_reqs(&batches)
        .filter(|row| matches!(&row.request, InteractReq::NoteProgress))
        .collect();
    assert_eq!(notes.len(), 3);
    let root = notes
        .iter()
        .filter(|row| row.lineage.is_none() && row.mirror_owner.is_none())
        .count();
    let hook: Vec<_> = notes
        .iter()
        .filter(|row| row.lineage.is_none() && row.mirror_owner.is_some())
        .collect();
    assert_eq!(root, 2, "sync and queued pushes from loop() stay root");
    assert_eq!(
        hook.len(),
        1,
        "the queued leave-hook push retains its local mirror"
    );
    assert!(hook[0].mirror_owner.unwrap().get() > 0);
    iso.join();
}

#[test]
fn case_22_bank_nearest_child_promise_survives_fight_completion_until_cancel() {
    let src = r#"
import { Banking } from '../../api/bank/Banking.js';
export const apiVersion = 2;
const site = {
    key: 't',
    target: 'Goblin',
    safespots: [{ x: 3222, z: 3222, level: 0 }],
    meleeAnchor: { x: 3222, z: 3222, level: 0 },
    boxes: [{ minX: 3220, maxX: 3224, minZ: 3220, maxZ: 3224, level: 0 }],
};
let started = false;
globalThis.__completionOrder = [];
globalThis.__parentSettled = false;
globalThis.__childStarted = false;
globalThis.__childSettled = false;
globalThis.__childResult = null;

export function tick(api) {
    if (started) return;
    started = true;
    const token = api.fightBegin(site).value.token;
    const parent = api.fightRun({ token }, {
        died: () => {
            const child = Banking.bankNearest();
            globalThis.__childStarted = true;
            child.then(
                (value) => {
                    globalThis.__childResult = value;
                    globalThis.__childSettled = true;
                    globalThis.__completionOrder.push('child');
                },
                (error) => {
                    globalThis.__childResult = { error: String(error) };
                    globalThis.__childSettled = true;
                    globalThis.__completionOrder.push('child');
                },
            );
            return true;
        },
    });
    parent.then(() => {
        globalThis.__parentSettled = true;
        globalThis.__completionOrder.push('parent');
    });
}
"#;
    let iso = spawn_owned_ready(src.to_string(), LoadShape::NativeTick, vec![]);
    let mut snapshot = ready_snapshot();
    let mut batches = Vec::new();
    post_test_tick(&iso, &mut snapshot, 1);
    batches.extend(iso.drain_seat_batches());

    let parent_id = {
        let parent = seat_starts(&batches)
            .find(|start| start.kind == NativeKind::Hunt)
            .expect("fightRun starts a real typed Hunt seat");
        assert_eq!(parent.parent, None);
        assert_eq!(parent.args, SeatStartArgs::Empty);
        parent.id
    };
    let live = [root_seat_entry(parent_id, NativeKind::Hunt)];
    let details = [root_seat_details(parent_id)];
    let live_page = SeatPageInput::new(&live, &[], &details, 0).unwrap();
    post_test_tick_with_native(
        &iso,
        &mut snapshot,
        2,
        NativeFactsInput {
            seat_page: Some(live_page),
            ..NativeFactsInput::default()
        },
    );
    batches.extend(iso.drain_seat_batches());

    for tick in 3..=5 {
        if iso.probe("globalThis.__childStarted").unwrap() == true {
            break;
        }
        post_test_tick(&iso, &mut snapshot, tick);
        batches.extend(iso.drain_seat_batches());
    }

    assert_eq!(iso.probe("globalThis.__childStarted").unwrap(), true);
    assert_eq!(iso.probe("globalThis.__childSettled").unwrap(), false);
    let child_id = {
        let child = seat_starts(&batches)
            .find(|start| start.kind == NativeKind::BankNearest)
            .expect("Banking.bankNearest starts a real typed Capability child");
        assert_eq!(child.parent, Some(parent_id));
        assert_bank_start_operation(child, "BankNearest");
        child.id
    };

    assert!(
        !batches
            .iter()
            .flat_map(|batch| batch.batch.mirror_revokes.iter())
            .any(|revoke| revoke.owner == parent_id),
        "ordinary Hunt completion rebases its Capability child, never revokes its mirror"
    );
    let promoted = [root_seat_entry(child_id, NativeKind::BankNearest)];
    let details = [SeatLiveDetails {
        id: child_id,
        phase: script::native::NativePhase::Waiting,
        prompt: None,
    }];
    let parent_result = [SeatResult {
        id: parent_id,
        seq: 1,
        outcome: SeatTerminal::Done,
        prior_parent: None,
        session: Some(parent_id),
        authority_revoked: false,
    }];
    let promoted_page = SeatPageInput::new(&promoted, &parent_result, &details, 0).unwrap();
    post_test_tick_with_native(
        &iso,
        &mut snapshot,
        6,
        NativeFactsInput {
            seat_page: Some(promoted_page),
            ..NativeFactsInput::default()
        },
    );
    batches.extend(iso.drain_seat_batches());
    assert_eq!(iso.probe("globalThis.__parentSettled").unwrap(), true);
    assert_eq!(iso.probe("globalThis.__childSettled").unwrap(), false);

    let terminal = [SeatResult {
        id: child_id,
        seq: 2,
        outcome: SeatTerminal::Aborted(AbortReason::UserInput),
        prior_parent: None,
        session: None,
        authority_revoked: true,
    }];
    let terminal_page = SeatPageInput::new(&[], &terminal, &[], 0).unwrap();
    post_test_tick_with_native(
        &iso,
        &mut snapshot,
        7,
        NativeFactsInput {
            seat_page: Some(terminal_page),
            ..NativeFactsInput::default()
        },
    );
    batches.extend(iso.drain_seat_batches());

    assert_eq!(iso.probe("globalThis.__childSettled").unwrap(), true);
    assert_eq!(
        iso.probe("globalThis.__completionOrder").unwrap(),
        serde_json::json!(["parent", "child"])
    );
    assert_eq!(
        batches.last().unwrap().batch.consumed_terminal_seq,
        2,
        "the terminal seat page is acknowledged by the next isolate batch"
    );
    iso.join();
}

#[test]
fn case_26_bank_nearest_reachable_and_progress_refusal_settle_inside_leave() {
    let src = r#"
import { bankRoutine } from '../../api/combat/hunting/combat.js';
export const apiVersion = 2;
const site = { boxes: [{ minX: 3220, maxX: 3224, minZ: 3220, maxZ: 3224, level: 0 }] };
let started = false;
globalThis.__leaveDone = false;
globalThis.__leaveValues = null;
globalThis.__rootDone = false;

export function tick(api) {
    if (started) return;
    started = true;
    const root = bankRoutine(globalThis.__rs2b0t_host, site, {
        leave: async () => {
            const bank = api.bankNearestReachable({ from: { x: 3222, z: 3222, level: 0 } });
            const progress = api.questProgress({ quest: 'unlisted-product-fixture' });
            const values = await Promise.all([bank, progress]);
            globalThis.__leaveValues = values;
            globalThis.__leaveDone = true;
            return values[0].ok === true && values[1].kind === 'done'
                && values[1].value.end === 'refused';
        },
    });
    root.then(() => { globalThis.__rootDone = true; });
}
"#;
    let iso = spawn_owned_ready(src.to_string(), LoadShape::NativeTick, vec![]);
    let mut snapshot = ready_snapshot();
    let mut batches = Vec::new();
    post_test_tick(&iso, &mut snapshot, 1);
    batches.extend(iso.drain_seat_batches());

    let bank = seat_starts(&batches)
        .find(|start| start.kind == NativeKind::BankPlan)
        .expect("bankNearestReachable starts a typed bank Select seat");
    assert_eq!(bank.parent, None);
    assert_bank_start_operation(bank, "Select");
    let bank_id = bank.id;
    assert!(
        !seat_reqs(&batches).any(|row| matches!(&row.request, InteractReq::SelectBank { .. })),
        "bank selection runs in the typed bank seat, not an isolate-owned interaction"
    );
    let progress = seat_starts(&batches)
        .find(|start| start.kind == NativeKind::QuestProgress)
        .expect("questProgress is carried as its typed native family start");
    assert_eq!(
        progress.parent, None,
        "a local Hunt hook is not a seat parent"
    );
    assert!(
        matches!(&progress.args, SeatStartArgs::QuestProgress(name) if name == "unlisted-product-fixture")
    );
    let progress_id = progress.id;
    let progress_token = progress_id.get();

    let live = [
        root_seat_entry(bank_id, NativeKind::BankPlan),
        root_seat_entry(progress_id, NativeKind::QuestProgress),
    ];
    let bank_details = SeatLiveDetails {
        id: bank_id,
        phase: script::native::NativePhase::Waiting,
        prompt: None,
    };
    let progress_details = root_seat_details(progress_id);
    let details = [bank_details, progress_details];
    let live_page = SeatPageInput::new(&live, &[], &details, 0).unwrap();
    post_test_tick_with_native(
        &iso,
        &mut snapshot,
        2,
        NativeFactsInput {
            seat_page: Some(live_page),
            ..NativeFactsInput::default()
        },
    );
    batches.extend(iso.drain_seat_batches());
    assert_eq!(iso.probe("globalThis.__leaveDone").unwrap(), false);

    let progress_page = ProgressPage::Refused {
        token: progress_token,
        reason: Arc::from("unknown-path"),
    };
    let terminal = [
        SeatResult {
            id: bank_id,
            seq: 1,
            outcome: SeatTerminal::Done,
            prior_parent: None,
            session: Some(bank_id),
            authority_revoked: false,
        },
        SeatResult {
            id: progress_id,
            seq: 2,
            outcome: SeatTerminal::Failed,
            prior_parent: None,
            session: Some(progress_id),
            authority_revoked: false,
        },
    ];
    let bank_value_json = serde_json::Value::Null;
    let bank_value_bytes = script::channel::encode(&bank_value_json).unwrap();
    let bank_value = [SeatResultValue {
        id: bank_id,
        value: &bank_value_bytes,
    }];
    let terminal_page = SeatPageInput::new(&[], &terminal, &[], 0)
        .unwrap()
        .with_result_values(&bank_value)
        .unwrap();
    post_test_tick_with_native(
        &iso,
        &mut snapshot,
        3,
        NativeFactsInput {
            api_progress: Some(&progress_page),
            seat_page: Some(terminal_page),
            ..NativeFactsInput::default()
        },
    );
    batches.extend(iso.drain_seat_batches());

    assert_eq!(iso.probe("globalThis.__leaveDone").unwrap(), true);
    assert_eq!(iso.probe("globalThis.__rootDone").unwrap(), true);
    let values = iso.probe("globalThis.__leaveValues").unwrap();
    assert_eq!(values[0]["ok"], true);
    assert_eq!(values[0]["value"], serde_json::Value::Null);
    assert_eq!(values[1]["kind"], "done");
    assert_eq!(values[1]["value"]["end"], "refused");
    assert_eq!(values[1]["value"]["reason"], "unknown-path");
    assert_eq!(batches.last().unwrap().batch.consumed_terminal_seq, 2);
    iso.join();
}

/// §28.7 case 27: the real Hunt machine is a Session; its in-isolate Walk
/// mirror emits through that Session, while bankNearest crosses into the
/// native seat as a Capability child. A user movement produces a fenced
/// AbortWalk release without losing the still-pending bank child.
#[test]
fn case_27_hunt_session_walk_mirror_bank_child_and_manual_release() {
    let src = r#"
import { Fight } from '../../api/combat/hunting/combat.js';
import { Traversal } from '../../api/walking/Traversal.js';
import { Banking } from '../../api/bank/Banking.js';
const site = {
    key: 'hybrid-hunt',
    target: 'Chicken',
    boxes: [{ minX: 3210, maxX: 3230, minZ: 3210, maxZ: 3230, level: 0 }],
    meleeAnchor: { x: 3222, z: 3222, level: 0 },
};
globalThis.__armStarted = false;
globalThis.__walkFinished = null;
globalThis.__bankFinished = null;
globalThis.__fightStarted = false;
globalThis.__fightError = null;
export default class T extends LoopingBot {
    loop() {
        if (globalThis.__fightStarted) return;
        globalThis.__fightStarted = true;
        const host = {
            hpFraction() { return 1; },
            hasFood() { return true; },
            needEat() { return false; },
            countKill() {},
            armSpecial() {
                if (globalThis.__armStarted) return false;
                globalThis.__armStarted = true;
                const walk = Traversal.walkResilient(
                    { x: 3300, z: 3300, level: 0 },
                    { radius: 1, timeoutMs: 300000 },
                );
                const bank = Banking.bankNearest();
                return walk.then((moved) => {
                    globalThis.__walkFinished = moved;
                    return bank;
                }).then((value) => {
                    globalThis.__bankFinished = value;
                    return false;
                });
            },
        };
        new Fight(host, site).execute().catch((error) => {
            globalThis.__fightError = String(error && error.message || error);
        });
    }
}
"#;
    let iso = spawn_owned_ready(src.to_string(), LoadShape::CompatClass, vec![]);
    let actions = ["Attack".to_string()];
    let chickens = [case23_chicken(&actions, 7)];
    let mut snapshot = ready_snapshot();
    snapshot.npcs = &chickens;
    let mut batches = Vec::new();
    post_test_tick(&iso, &mut snapshot, 1);
    batches.extend(iso.drain_seat_batches());
    let hunt = seat_starts(&batches)
        .find(|start| start.kind == NativeKind::Hunt)
        .expect("Fight begins a real Hunt Session");
    assert_eq!(hunt.parent, None);
    let hunt_id = hunt.id;
    let hunt_live = [root_seat_entry(hunt_id, NativeKind::Hunt)];
    let hunt_details = [root_seat_details(hunt_id)];
    let hunt_page = SeatPageInput::new(&hunt_live, &[], &hunt_details, 0).unwrap();
    post_test_tick_with_native(
        &iso,
        &mut snapshot,
        2,
        NativeFactsInput {
            seat_page: Some(hunt_page),
            ..NativeFactsInput::default()
        },
    );
    batches.extend(iso.drain_seat_batches());

    let mut armed_at = None;
    for tick in 3..=24 {
        post_test_tick(&iso, &mut snapshot, tick);
        batches.extend(iso.drain_seat_batches());
        if iso.probe("globalThis.__armStarted").unwrap() == true
            && seat_starts(&batches).any(|start| start.kind == NativeKind::BankNearest)
            && seat_reqs(&batches).any(|row| matches!(&row.request, InteractReq::WalkNear { .. }))
        {
            armed_at = Some(tick);
            break;
        }
    }
    let armed_at = armed_at.expect("Hunt hook arms local Walk and typed bank child");
    let bank = seat_starts(&batches)
        .find(|start| start.kind == NativeKind::BankNearest)
        .unwrap();
    assert_eq!(
        bank.parent,
        Some(hunt_id),
        "local Walk is a mirror, not a seat parent"
    );
    assert_bank_start_operation(bank, "BankNearest");
    let bank_id = bank.id;
    let (walk_id, mirror_owner) = seat_reqs(&batches)
        .find_map(|row| match &row.request {
            InteractReq::WalkNear {
                request_id,
                x: 3300,
                z: 3300,
                ..
            } => {
                assert_eq!(row.lineage, Some(hunt_id));
                Some((*request_id, row.mirror_owner))
            }
            _ => None,
        })
        .expect("WalkResilient emits a real host walk request");
    assert_ne!(walk_id, 0);
    assert!(mirror_owner.is_some_and(|owner| owner != hunt_id));
    assert_eq!(
        iso.probe("globalThis.__walkFinished").unwrap(),
        serde_json::Value::Null
    );
    assert_eq!(
        iso.probe("globalThis.__bankFinished").unwrap(),
        serde_json::Value::Null
    );

    let live = [
        root_seat_entry(hunt_id, NativeKind::Hunt),
        SeatEntry {
            id: bank_id,
            parent: Some(hunt_id),
            kind: NativeKind::BankNearest,
            origin: ChildOrigin::Started,
            depth: 2,
            decisions_consumed: 0,
            prompt: None,
        },
    ];
    let details = [root_seat_details(hunt_id), root_seat_details(bank_id)];
    let live_page = SeatPageInput::new(&live, &[], &details, 0).unwrap();
    post_test_tick_with_native(
        &iso,
        &mut snapshot,
        armed_at + 1,
        NativeFactsInput {
            seat_page: Some(live_page),
            ..NativeFactsInput::default()
        },
    );
    batches.extend(iso.drain_seat_batches());
    snapshot.user_move_intent_seq = 1;
    for tick in armed_at + 2..=armed_at + 6 {
        post_test_tick(&iso, &mut snapshot, tick);
        batches.extend(iso.drain_seat_batches());
        if seat_reqs(&batches).any(|row| {
            matches!(
                &row.request,
                InteractReq::AbortWalk { request_id } if *request_id == walk_id
            )
        }) {
            break;
        }
    }
    let release = seat_reqs(&batches)
        .find(|row| {
            matches!(
                &row.request,
                InteractReq::AbortWalk { request_id } if *request_id == walk_id
            )
        })
        .expect("manual movement emits the same armed walk's fenced AbortWalk");
    assert_eq!(release.lineage, Some(hunt_id));
    assert_eq!(release.mirror_owner, mirror_owner);
    assert_eq!(iso.probe("globalThis.__walkFinished").unwrap(), false);
    assert_eq!(
        iso.probe("globalThis.__bankFinished").unwrap(),
        serde_json::Value::Null
    );
    assert_eq!(
        iso.probe("globalThis.__fightError").unwrap(),
        serde_json::Value::Null
    );
    assert!(
        iso.drain_interacts_stamped().is_empty(),
        "owner-bound hybrid output must not escape into the legacy interact queue"
    );
    iso.join();
}

/// §28.7 case 27 reverse cutover: an in-isolate exclusive Reach owns the
/// synchronous Sustain hook that starts a seat-family bankNearest. The child
/// has no seat parent, but its acknowledged local-parent link still cancels
/// the child when another Reach supersedes that local row.
#[test]
fn case_27_reverse_local_parent_supersession_cancels_admitted_seat_child() {
    let src = r#"
import { Reach } from '../../api/walking/Reach.js';
import { Sustain } from '../../api/sustain/Sustain.js';
import { Banking } from '../../api/bank/Banking.js';
const options = { name: 'Banker', near: { x: 3300, z: 3300, level: 0 } };
globalThis.__reverseStarted = false;
globalThis.__reverseChildStarted = false;
globalThis.__reverseChildSettles = 0;
globalThis.__reverseChildValue = null;
globalThis.__replaceReach = () => {
    Reach.npcDialog(options);
};
export function tick() {
    if (globalThis.__reverseStarted) return;
    globalThis.__reverseStarted = true;
    Sustain.set(() => {
        if (!globalThis.__reverseChildStarted) {
            globalThis.__reverseChildStarted = true;
            Banking.bankNearest().then((value) => {
                globalThis.__reverseChildValue = value;
                globalThis.__reverseChildSettles++;
            });
        }
        return new Promise(() => {});
    });
    Reach.npcDialog(options);
}
"#;
    let iso = spawn_owned_ready(src.to_string(), LoadShape::NativeTick, vec![]);
    let mut snapshot = ready_snapshot();
    let mut batches = Vec::new();
    let mut started_at = None;
    for tick in 1..=12 {
        post_test_tick(&iso, &mut snapshot, tick);
        batches.extend(iso.drain_seat_batches());
        if seat_starts(&batches).any(|start| start.kind == NativeKind::BankNearest) {
            started_at = Some(tick);
            break;
        }
    }
    let started_at = started_at.expect("Reach Sustain hook starts a typed bank child");
    let child = seat_starts(&batches)
        .find(|start| start.kind == NativeKind::BankNearest)
        .unwrap();
    assert_eq!(
        child.parent, None,
        "an in-isolate parent cannot invent a seat ancestor"
    );
    assert_bank_start_operation(child, "BankNearest");
    let child_id = child.id;
    let live = [root_seat_entry(child_id, NativeKind::BankNearest)];
    let details = [root_seat_details(child_id)];
    let live_page = SeatPageInput::new(&live, &[], &details, 0).unwrap();
    post_test_tick_with_native(
        &iso,
        &mut snapshot,
        started_at + 1,
        NativeFactsInput {
            seat_page: Some(live_page),
            ..NativeFactsInput::default()
        },
    );
    batches.extend(iso.drain_seat_batches());
    iso.probe("globalThis.__replaceReach(); true").unwrap();
    for tick in started_at + 2..=started_at + 5 {
        post_test_tick(&iso, &mut snapshot, tick);
        batches.extend(iso.drain_seat_batches());
        if batches
            .iter()
            .flat_map(|batch| batch.batch.cancels.iter())
            .any(|cancel| {
                cancel.of == child_id && cancel.reason == CallbackCancelReason::ParentAborted
            })
        {
            break;
        }
    }
    assert!(
        batches
            .iter()
            .flat_map(|batch| batch.batch.cancels.iter())
            .any(|cancel| {
                cancel.of == child_id && cancel.reason == CallbackCancelReason::ParentAborted
            }),
        "superseding a local parent carries an admitted child's ParentAborted control"
    );
    assert_eq!(iso.probe("globalThis.__reverseChildSettles").unwrap(), 0);

    let terminal = [SeatResult {
        id: child_id,
        seq: 1,
        outcome: SeatTerminal::Aborted(AbortReason::ParentAborted),
        prior_parent: None,
        session: None,
        authority_revoked: true,
    }];
    let terminal_page = SeatPageInput::new(&[], &terminal, &[], 0).unwrap();
    post_test_tick_with_native(
        &iso,
        &mut snapshot,
        started_at + 6,
        NativeFactsInput {
            seat_page: Some(terminal_page),
            ..NativeFactsInput::default()
        },
    );
    batches.extend(iso.drain_seat_batches());
    assert_eq!(iso.probe("globalThis.__reverseChildSettles").unwrap(), 1);
    assert_eq!(iso.probe("globalThis.__reverseChildValue").unwrap(), false);
    assert_eq!(
        batches.last().unwrap().batch.consumed_terminal_seq,
        1,
        "terminal receipt retires the carried cancellation"
    );
    assert!(iso.drain_interacts_stamped().is_empty());
    iso.join();
}
