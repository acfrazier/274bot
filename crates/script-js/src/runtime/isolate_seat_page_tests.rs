//! `SeatPage` dispatch on the real isolate thread, with seat pages encoded as
//! the host encodes them and posted without a game tick. Each test commands
//! the dispatch itself, so it separates page application from command
//! execution (design `EVENT-RUNTIME-DESIGN-1022/design-r5.md`, S1).

use super::*;
use api::identity::RequestId;
use api::script_contract::{ChildOrigin, DecisionSequence};
use script::native::request::NativeKind;
use script::native::seat::{SeatEntry, SeatResult, SeatTerminal};

/// One live row of a posted seat page.
struct Live<'a> {
    id: RequestId,
    kind: NativeKind,
    consumed: u32,
    prompt: Option<(u32, &'a str, serde_json::Value)>,
}

fn live(id: RequestId, kind: NativeKind) -> Live<'static> {
    Live {
        id,
        kind,
        consumed: 0,
        prompt: None,
    }
}

fn prompted<'a>(
    id: RequestId,
    kind: NativeKind,
    sequence: u32,
    point: &'a str,
    args: serde_json::Value,
) -> Live<'a> {
    Live {
        id,
        kind,
        consumed: sequence - 1,
        prompt: Some((sequence, point, args)),
    }
}

fn result(id: RequestId, seq: u64, outcome: SeatTerminal) -> SeatResult {
    SeatResult {
        id,
        seq,
        outcome,
        prior_parent: None,
        session: None,
        authority_revoked: false,
    }
}

/// Encode and post one seat page. No tick follows: only a `SeatPage` command
/// executes work.
fn post_page(
    iso: &LoadIsolate,
    buffer: &mut crate::wire::IsolateBuf,
    tick: u64,
    rows: &[Live<'_>],
    results: &[SeatResult],
) {
    let encoded: Vec<Option<Vec<u8>>> = rows
        .iter()
        .map(|row| {
            row.prompt
                .as_ref()
                .map(|(_, _, args)| script::channel::encode(args).expect("bounded arguments"))
        })
        .collect();
    let entries: Vec<SeatEntry> = rows
        .iter()
        .map(|row| SeatEntry {
            id: row.id,
            parent: None,
            kind: row.kind,
            origin: ChildOrigin::Started,
            depth: 0,
            decisions_consumed: row.consumed,
            prompt: row.prompt.as_ref().map(|(sequence, _, _)| *sequence),
        })
        .collect();
    let details: Vec<crate::wire::SeatLiveDetails<'_>> = rows
        .iter()
        .zip(&encoded)
        .map(|(row, args)| crate::wire::SeatLiveDetails {
            id: row.id,
            phase: script::native::NativePhase::Waiting,
            prompt: row.prompt.as_ref().zip(args.as_ref()).map(
                |((sequence, point, _), args)| crate::wire::SeatPromptInput {
                    sequence: DecisionSequence::from_raw(*sequence).expect("nonzero sequence"),
                    point,
                    args,
                },
            ),
        })
        .collect();
    let page = crate::wire::SeatPageInput::new(&entries, results, &details, 0)
        .expect("valid seat page");
    let snapshot = crate::wire::tests::empty_input(tick);
    let bytes = buffer.encode_snapshot_with_native(
        &snapshot,
        crate::wire::NativeFactsInput {
            seat_page: Some(page),
            ..Default::default()
        },
    );
    assert!(iso.post_snapshot(bytes), "the page is accepted");
}

/// Queue one seat page dispatch and wait for the isolate thread to finish it.
fn dispatch(iso: &LoadIsolate, tick: u64, floor: u64) {
    iso.enqueue_seat_page_without_watchdog_for_test(tick, floor);
    iso.probe("true").unwrap();
}

/// The ids of the native starts of the first tick, in start order.
fn started(iso: &LoadIsolate) -> Vec<RequestId> {
    machine_tick(iso, 1);
    let batches = iso.drain_seat_batches();
    batches
        .iter()
        .flat_map(|batch| &batch.batch.starts)
        .map(|start| start.id)
        .collect()
}

fn settled(iso: &LoadIsolate) -> Vec<serde_json::Value> {
    iso.machine_host_for_test()["settled"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn row_of(iso: &LoadIsolate, id: RequestId) -> serde_json::Value {
    let handle = id.get() - RequestId::ISOLATE_FIRST + 1;
    iso.machine_host_for_test()["rows"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["handle"] == handle).cloned())
        .unwrap_or(serde_json::Value::Null)
}

fn steps_of(iso: &LoadIsolate, id: RequestId) -> u64 {
    row_of(iso, id)["steps"].as_u64().unwrap_or(0)
}

const THREE_DEPOSITS: &str = "globalThis.__got = {};
     const hooks = { match: () => false };
     for (const name of ['c', 'o', 'a']) {
         runMachine('bank_deposit', {}, hooks).then(
             value => { globalThis.__got[name] = { kind: 'done', value }; },
             error => { globalThis.__got[name] = { kind: 'threw' }; },
         );
     }";

#[test]
fn seat_answer_wake_releases_its_obligation_and_nothing_older() {
    let iso = spawn_bank_machine_card(THREE_DEPOSITS);
    let ids = started(&iso);
    assert_eq!(ids.len(), 3, "three native starts");
    let [c, o, a] = [ids[0], ids[1], ids[2]];
    let mut buffer = crate::wire::IsolateBuf::default();
    let rows = [
        live(c, NativeKind::BankPlan),
        live(o, NativeKind::BankPlan),
        live(a, NativeKind::BankPlan),
    ];
    post_page(&iso, &mut buffer, 1, &rows, &[]);
    dispatch(&iso, 1, 0);
    let _ = iso.drain_seat_batches();

    // P0 carries C's terminal (sequence 1), P1 carries O's (2) and stands for
    // a `Tick` page whose full dispatch the slow-tick skip dropped, P2 carries
    // A's (3). None of them is dispatched yet.
    let mut results = vec![result(c, 1, SeatTerminal::Done)];
    post_page(&iso, &mut buffer, 1, &rows[1..], &results);
    results.push(result(o, 2, SeatTerminal::Done));
    post_page(&iso, &mut buffer, 1, &rows[2..], &results);
    results.push(result(a, 3, SeatTerminal::Done));
    post_page(&iso, &mut buffer, 1, &[], &results);
    let stamps: Vec<_> = settled(&iso)
        .iter()
        .map(|entry| (entry["handle"].as_u64().unwrap(), entry["seat_seq"].as_u64().unwrap()))
        .collect();
    assert_eq!(
        stamps,
        vec![(1, 1), (2, 2), (3, 3)],
        "each page stamped its own terminal"
    );
    assert_eq!(
        iso.probe("Object.keys(globalThis.__got)").unwrap(),
        serde_json::json!([]),
        "page application releases nothing"
    );

    // The dispatch of the open obligation carries the floor 2: only A's
    // terminal is above it. C's and O's belong to obligations that earlier
    // accepted dispatches carried; they wait for the next global settle.
    dispatch(&iso, 1, 2);
    assert_eq!(
        iso.probe("Object.keys(globalThis.__got)").unwrap(),
        serde_json::json!(["a"])
    );
    let remaining: Vec<_> = settled(&iso)
        .iter()
        .map(|entry| entry["handle"].as_u64().unwrap())
        .collect();
    assert_eq!(remaining, vec![1, 2], "C and O keep their entries");
    // A second dispatch at the same floor is idempotent.
    dispatch(&iso, 1, 2);
    assert_eq!(
        iso.probe("Object.keys(globalThis.__got)").unwrap(),
        serde_json::json!(["a"])
    );
    // The next global settle releases what is left, whatever the stamp.
    machine_tick(&iso, 2);
    let mut got: Vec<String> = serde_json::from_value(iso.probe("Object.keys(globalThis.__got)").unwrap()).unwrap();
    got.sort();
    assert_eq!(got, vec!["a", "c", "o"]);
    iso.join();
}

#[test]
fn seat_page_stamps_a_pass_settled_native_outcome() {
    // X's match hook throws on its first visit inside the dispatch; U is a
    // second row whose terminal an earlier page already stamped.
    let iso = spawn_bank_machine_card(
        "globalThis.__thrown = { kind: 'b3' };
         globalThis.__got = {};
         const record = (name) => [
             value => { globalThis.__got[name] = { kind: 'done' }; },
             error => { globalThis.__got[name] = { kind: 'threw', same: error === globalThis.__thrown }; },
         ];
         runMachine('bank_deposit', {}, { match: () => { throw globalThis.__thrown; } })
             .then(...record('x'));
         runMachine('bank_deposit', {}, { match: () => false }).then(...record('u'));",
    );
    let ids = started(&iso);
    let [x, u] = [ids[0], ids[1]];
    let mut buffer = crate::wire::IsolateBuf::default();
    post_page(
        &iso,
        &mut buffer,
        1,
        &[live(x, NativeKind::BankPlan), live(u, NativeKind::BankPlan)],
        &[],
    );
    dispatch(&iso, 1, 0);
    let _ = iso.drain_seat_batches();
    // An earlier page stamps U with sequence 1.
    post_page(
        &iso,
        &mut buffer,
        1,
        &[prompted(x, NativeKind::BankPlan, 1, "match", serde_json::json!(["Item"]))],
        &[result(u, 1, SeatTerminal::Done)],
    );
    assert_eq!(
        settled(&iso)
            .iter()
            .map(|entry| (entry["handle"].as_u64().unwrap(), entry["seat_seq"].as_u64().unwrap()))
            .collect::<Vec<_>>(),
        vec![(2, 1)]
    );
    // The dispatch steps X (the prompt advanced), the hook throws and the row
    // ends inside the pass: the entry keeps its failure and has no stamp.
    dispatch(&iso, 1, 1);
    let batches = iso.drain_seat_batches();
    let failed = batches
        .iter()
        .flat_map(|batch| &batch.batch.machine_events)
        .any(|event| format!("{event:?}").contains("Failed"));
    assert!(failed, "the batch carries the Failed event: {batches:?}");
    let entries = settled(&iso);
    let x_entry = entries.iter().find(|entry| entry["handle"] == 1).expect("X's entry");
    assert_eq!(x_entry["kind"], "failed");
    assert_eq!(x_entry["seat_seq"], 0, "a pass settle carries no stamp yet");
    assert_eq!(
        iso.probe("globalThis.__got.x === undefined").unwrap(),
        serde_json::json!(true),
        "the await is not released by its own pass settle"
    );
    // The host publishes the seat's Failed result, and the page that carries
    // it stamps X's stored entry without changing its outcome.
    post_page(
        &iso,
        &mut buffer,
        1,
        &[],
        &[result(u, 1, SeatTerminal::Done), result(x, 2, SeatTerminal::Failed)],
    );
    let entries = settled(&iso);
    let x_entry = entries.iter().find(|entry| entry["handle"] == 1).expect("X's entry");
    assert_eq!(x_entry["kind"], "failed", "the stored outcome is kept");
    assert_eq!(x_entry["seat_seq"], 2);
    dispatch(&iso, 1, 1);
    assert_eq!(
        iso.probe("globalThis.__got.x").unwrap(),
        serde_json::json!({ "kind": "threw", "same": true }),
        "the rejection carries the thrown identity"
    );
    assert_eq!(
        iso.probe("globalThis.__got.u === undefined").unwrap(),
        serde_json::json!(true),
        "U's older terminal stays out of this release"
    );
    iso.join();
}

#[test]
fn seat_page_dispatches_carry_the_awaited_call_budget() {
    // `afterDeposit` is the awaited member of bank_nearest: each prompt is one
    // callback of the row's per-tick budget, and a seat page never refills it.
    let iso = spawn_bank_machine_card(
        "globalThis.__after_calls = 0;
         globalThis.__got = {};
         runMachine('bank_nearest', {}, {
             deposit: () => false,
             afterDeposit: () => { globalThis.__after_calls += 1; return Promise.resolve(true); },
             log: () => {},
         }).then(value => { globalThis.__got.n = 'done'; }, error => { globalThis.__got.n = 'threw'; });",
    );
    let ids = started(&iso);
    let id = ids[0];
    let mut buffer = crate::wire::IsolateBuf::default();
    post_page(&iso, &mut buffer, 1, &[live(id, NativeKind::BankNearest)], &[]);
    dispatch(&iso, 1, 0);
    for sequence in 1..=300u32 {
        post_page(
            &iso,
            &mut buffer,
            1,
            &[prompted(
                id,
                NativeKind::BankNearest,
                sequence,
                "afterDeposit",
                serde_json::json!(["Item"]),
            )],
            &[],
        );
        dispatch(&iso, 1, 0);
    }
    assert_eq!(
        iso.probe("globalThis.__after_calls").unwrap(),
        serde_json::json!(256),
        "the callback budget of one tick stops the row at 256 across seat pages"
    );
    let row = row_of(&iso, id);
    assert_eq!(row["calls"], 256);
    assert_eq!(row["seat_changed"], true, "the 257th prompt still waits for a step");
    // A full tick refills the budget and steps the row once more.
    machine_tick(&iso, 2);
    assert_eq!(
        iso.probe("globalThis.__after_calls").unwrap(),
        serde_json::json!(257)
    );
    iso.join();
}

#[test]
fn seat_dispatch_runs_ready_js_continuations_but_resumes_no_other_row() {
    // Row B waits on an awaited hook whose promise the script holds. Row A's
    // second visit resolves it. The microtask checkpoint of A's dispatch
    // runs B's JavaScript continuation, and B's Rust row is not resumed.
    let iso = spawn_bank_machine_card(
        "globalThis.__b_continued = 0;
         globalThis.__visits = 0;
         globalThis.__pB = new Promise(resolve => { globalThis.__resolveB = resolve; });
         runMachine('bank_deposit', {}, { match: () => {
             globalThis.__visits += 1;
             if (globalThis.__visits === 2) globalThis.__resolveB(true);
             return false;
         } });
         runMachine('bank_nearest', {}, {
             deposit: () => false,
             afterDeposit: () => globalThis.__pB.then(() => { globalThis.__b_continued += 1; return true; }),
             log: () => {},
         });",
    );
    let ids = started(&iso);
    let [a, b] = [ids[0], ids[1]];
    let mut buffer = crate::wire::IsolateBuf::default();
    post_page(
        &iso,
        &mut buffer,
        1,
        &[live(a, NativeKind::BankPlan), live(b, NativeKind::BankNearest)],
        &[],
    );
    dispatch(&iso, 1, 0);
    // B's prompt: its hook is called and its promise stays pending.
    post_page(
        &iso,
        &mut buffer,
        1,
        &[
            live(a, NativeKind::BankPlan),
            prompted(b, NativeKind::BankNearest, 1, "afterDeposit", serde_json::json!(["Item"])),
        ],
        &[],
    );
    dispatch(&iso, 1, 0);
    assert_eq!(row_of(&iso, b)["pending"], true);
    let b_steps = steps_of(&iso, b);
    let b_calls = row_of(&iso, b)["calls"].clone();
    // A's first visit, then its second.
    for sequence in 1..=2u32 {
        post_page(
            &iso,
            &mut buffer,
            1,
            &[
                prompted(a, NativeKind::BankPlan, sequence, "match", serde_json::json!(["Item"])),
                prompted(b, NativeKind::BankNearest, 1, "afterDeposit", serde_json::json!(["Item"])),
            ],
            &[],
        );
        dispatch(&iso, 1, 0);
    }
    assert_eq!(iso.probe("globalThis.__visits").unwrap(), serde_json::json!(2));
    assert_eq!(
        iso.probe("globalThis.__b_continued").unwrap(),
        serde_json::json!(1),
        "B's JavaScript continuation ran in a checkpoint of this dispatch"
    );
    let row = row_of(&iso, b);
    assert_eq!(row["pending"], true, "B's Rust row was not resumed");
    assert_eq!(row["calls"], b_calls);
    assert_eq!(steps_of(&iso, b), b_steps, "B was not stepped");
    iso.join();
}

#[test]
fn seat_answer_wake_steps_only_the_changed_row() {
    // A is the answered row. S is a second native row with no prompt, P a
    // snapshot-sensitive local row, and the script holds a parked wait.
    let iso = spawn_bank_machine_card(
        "globalThis.__visits = 0;
         globalThis.__wait_done = false;
         runMachine('bank_deposit', {}, { match: () => { globalThis.__visits += 1; return false; } });
         runMachine('bank_nearest', {}, { deposit: () => false, afterDeposit: () => true, log: () => {} });
         runMachine('probe', { button: 7, steps: 99 });
         Execution.delayTicks(50).then(() => { globalThis.__wait_done = true; });",
    );
    let ids = started(&iso);
    let [a, s] = [ids[0], ids[1]];
    let mut buffer = crate::wire::IsolateBuf::default();
    post_page(
        &iso,
        &mut buffer,
        1,
        &[live(a, NativeKind::BankPlan), live(s, NativeKind::BankNearest)],
        &[],
    );
    dispatch(&iso, 1, 0);
    let tick = iso.probe("__rs2b0t_host.tick").unwrap();
    let host = iso.machine_host_for_test();
    let probe_handle = host["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["family"] == "probe")
        .expect("the probe row")["handle"]
        .as_u64()
        .unwrap();
    let steps = |handle: u64| {
        iso.machine_host_for_test()["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["handle"] == handle)
            .map_or(0, |row| row["steps"].as_u64().unwrap())
    };
    let (s_before, p_before) = (steps(s.get() - RequestId::ISOLATE_FIRST + 1), steps(probe_handle));
    for sequence in 1..=5u32 {
        post_page(
            &iso,
            &mut buffer,
            1,
            &[
                prompted(a, NativeKind::BankPlan, sequence, "match", serde_json::json!(["Item"])),
                live(s, NativeKind::BankNearest),
            ],
            &[],
        );
        dispatch(&iso, 1, 0);
    }
    assert_eq!(iso.probe("globalThis.__visits").unwrap(), serde_json::json!(5));
    assert_eq!(steps(s.get() - RequestId::ISOLATE_FIRST + 1), s_before, "the sibling row was not stepped");
    assert_eq!(steps(probe_handle), p_before, "the snapshot-sensitive row was not stepped");
    assert_eq!(
        iso.probe("globalThis.__wait_done").unwrap(),
        serde_json::json!(false),
        "the parked wait is still parked"
    );
    assert_eq!(iso.probe("__rs2b0t_host.tick").unwrap(), tick, "no tick time moved");
    iso.join();
}

#[test]
fn destructive_reset_zeroes_stamps_and_the_first_new_result_qualifies() {
    // C's terminal carries a high stamp from a page that no dispatch carried.
    // A non-kept reset must clear it, so a new request's first result (whose
    // sequence restarts at 1) is above the new floor while C's old entry is not.
    let iso = spawn_bank_machine_card(
        "globalThis.__got = {};
         runMachine('bank_deposit', {}, { match: () => false }).then(
             v => { globalThis.__got.c = 'done'; }, e => { globalThis.__got.c = 'threw'; });
         runMachine('bank_deposit', {}, { match: () => false }).then(
             v => { globalThis.__got.n = 'done'; }, e => { globalThis.__got.n = 'threw'; });
         globalThis.__start_n3 = false;
         runMachine('bank_deposit', {}, { match: () => false }).then(
             v => { globalThis.__got.n3 = 'done'; }, e => { globalThis.__got.n3 = 'threw'; });",
    );
    let ids = started(&iso);
    let [c, n, n3] = [ids[0], ids[1], ids[2]];
    let mut buffer = crate::wire::IsolateBuf::default();
    let rows = [
        live(c, NativeKind::BankPlan),
        live(n, NativeKind::BankPlan),
        live(n3, NativeKind::BankPlan),
    ];
    post_page(&iso, &mut buffer, 1, &rows, &[]);
    dispatch(&iso, 1, 0);
    // Page P0 carries C's terminal with sequence 5 and nothing dispatches it.
    post_page(&iso, &mut buffer, 1, &rows[1..], &[result(c, 5, SeatTerminal::Done)]);
    let before: Vec<_> = settled(&iso)
        .iter()
        .map(|entry| (entry["handle"].as_u64().unwrap(), entry["seat_seq"].as_u64().unwrap()))
        .collect();
    assert_eq!(before, vec![(1, 5)]);
    iso.reset_session_work(None);
    iso.probe("true").unwrap();
    let entries = settled(&iso);
    let by_handle = |handle: u64| {
        entries
            .iter()
            .find(|entry| entry["handle"] == handle)
            .cloned()
            .unwrap_or_else(|| panic!("entry {handle}: {entries:?}"))
    };
    assert_eq!(by_handle(1)["seat_seq"], 0, "C's stamp is cleared");
    assert_eq!(by_handle(1)["kind"], "done", "C keeps its real outcome");
    assert_eq!(by_handle(2)["kind"], "aborted");
    assert_eq!(by_handle(2)["reason"], "reset");
    assert_eq!(by_handle(2)["seat_seq"], 0);
    let executed = iso.executed_seat_pages_for_test();
    // A SeatPage of the new generation, with the new seat's floor 0: nothing
    // of the old session qualifies, so no old await is released by it.
    dispatch(&iso, 1, 0);
    assert_eq!(iso.executed_seat_pages_for_test(), executed + 1);
    let released: Vec<String> =
        serde_json::from_value(iso.probe("Object.keys(globalThis.__got)").unwrap()).unwrap();
    // A v2 or compat reset settles the aborted awaits itself; the seat page
    // adds none of C's old outcome beyond what the reset already delivered.
    assert!(
        released.iter().all(|name| name != "c") || iso.probe("globalThis.__got.c").unwrap() == serde_json::json!("done"),
        "{released:?}"
    );
    iso.join();
}
