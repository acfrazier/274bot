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
    post_page_in(
        iso,
        buffer,
        crate::wire::tests::empty_input(tick),
        rows,
        results,
    );
}

/// `post_page` over explicit facts, for a trace whose rows read them.
fn post_page_in(
    iso: &LoadIsolate,
    buffer: &mut crate::wire::IsolateBuf,
    snapshot: script::native::snapshot_source::SnapshotInput<'_>,
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
    let details: Vec<script::native::seat_rows::SeatLiveDetails<'_>> =
        rows.iter()
            .zip(&encoded)
            .map(|(row, args)| script::native::seat_rows::SeatLiveDetails {
                id: row.id,
                phase: script::native::NativePhase::Waiting,
                prompt: row.prompt.as_ref().zip(args.as_ref()).map(
                    |((sequence, point, _), args)| script::native::seat_rows::SeatPromptInput {
                        sequence: DecisionSequence::from_raw(*sequence).expect("nonzero sequence"),
                        point,
                        args,
                    },
                ),
            })
            .collect();
    let page = script::native::seat_rows::SeatPageInput::new(&entries, results, &details, 0)
        .expect("valid seat page");
    let bytes = buffer.encode_snapshot_with_native(
        &snapshot,
        script::native::snapshot_source::NativeFactsInput {
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

/// Three native rows that one script awaits: C is a `bank_nearest`, O a
/// `bank_wait` and A a `bank_deposit`. One transfer at a time may run, so the
/// rows are of different families.
const THREE_ROWS: &str = "globalThis.__got = {};
     const record = (name) => [
         value => { globalThis.__got[name] = { kind: 'done', value }; },
         error => { globalThis.__got[name] = { kind: 'threw' }; },
     ];
     runMachine('bank_nearest', {}, { deposit: () => false, afterDeposit: () => true, log: () => {} })
         .then(...record('c'));
     runMachine('bank_wait', { kind: 'ready' }, { log: () => {} }).then(...record('o'));
     runMachine('bank_deposit', {}, { match: () => false }).then(...record('a'));";

#[test]
fn seat_answer_wake_releases_its_obligation_and_nothing_older() {
    let iso = spawn_bank_machine_card(THREE_ROWS);
    let ids = started(&iso);
    assert_eq!(ids.len(), 3, "three native starts");
    let [c, o, a] = [ids[0], ids[1], ids[2]];
    let mut buffer = crate::wire::IsolateBuf::default();
    let rows = [
        live(c, NativeKind::BankNearest),
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
        .map(|entry| {
            (
                entry["handle"].as_u64().unwrap(),
                entry["seat_seq"].as_u64().unwrap(),
            )
        })
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
    let mut got: Vec<String> =
        serde_json::from_value(iso.probe("Object.keys(globalThis.__got)").unwrap()).unwrap();
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
         runMachine('bank_nearest', {}, { deposit: () => false, afterDeposit: () => true, log: () => {} })
             .then(...record('u'));",
    );
    let ids = started(&iso);
    let [x, u] = [ids[0], ids[1]];
    let mut buffer = crate::wire::IsolateBuf::default();
    post_page(
        &iso,
        &mut buffer,
        1,
        &[
            live(x, NativeKind::BankPlan),
            live(u, NativeKind::BankNearest),
        ],
        &[],
    );
    dispatch(&iso, 1, 0);
    let _ = iso.drain_seat_batches();
    // An earlier page stamps U with sequence 1.
    post_page(
        &iso,
        &mut buffer,
        1,
        &[prompted(
            x,
            NativeKind::BankPlan,
            1,
            "match",
            serde_json::json!(["Item"]),
        )],
        &[result(u, 1, SeatTerminal::Done)],
    );
    assert_eq!(
        settled(&iso)
            .iter()
            .map(|entry| (
                entry["handle"].as_u64().unwrap(),
                entry["seat_seq"].as_u64().unwrap()
            ))
            .collect::<Vec<_>>(),
        vec![(2, 1)]
    );
    // The dispatch steps X (the prompt advanced), the hook throws and the row
    // ends inside the pass: the entry keeps its failure and has no stamp.
    dispatch(&iso, 1, 1);
    let batches = iso.drain_seat_batches();
    let failed = batches
        .iter()
        .flat_map(|batch| &batch.machine_events)
        .any(|event| format!("{event:?}").contains("Failed"));
    assert!(failed, "the batch carries the Failed event: {batches:?}");
    let entries = settled(&iso);
    let x_entry = entries
        .iter()
        .find(|entry| entry["handle"] == 1)
        .expect("X's entry");
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
        &[
            result(u, 1, SeatTerminal::Done),
            result(x, 2, SeatTerminal::Failed),
        ],
    );
    let entries = settled(&iso);
    let x_entry = entries
        .iter()
        .find(|entry| entry["handle"] == 1)
        .expect("X's entry");
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
    post_page(
        &iso,
        &mut buffer,
        1,
        &[live(id, NativeKind::BankNearest)],
        &[],
    );
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
    assert_eq!(
        row["seat_changed"], true,
        "the 257th prompt still waits for a step"
    );
    // Full ticks refill the budget: the first consumes the 256th reply, the
    // next calls the hook for the prompt that waited.
    machine_tick(&iso, 2);
    machine_tick(&iso, 3);
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
        &[
            live(a, NativeKind::BankPlan),
            live(b, NativeKind::BankNearest),
        ],
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
            prompted(
                b,
                NativeKind::BankNearest,
                1,
                "afterDeposit",
                serde_json::json!(["Item"]),
            ),
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
                prompted(
                    a,
                    NativeKind::BankPlan,
                    sequence,
                    "match",
                    serde_json::json!(["Item"]),
                ),
                prompted(
                    b,
                    NativeKind::BankNearest,
                    1,
                    "afterDeposit",
                    serde_json::json!(["Item"]),
                ),
            ],
            &[],
        );
        dispatch(&iso, 1, 0);
    }
    assert_eq!(
        iso.probe("globalThis.__visits").unwrap(),
        serde_json::json!(2)
    );
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
        &[
            live(a, NativeKind::BankPlan),
            live(s, NativeKind::BankNearest),
        ],
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
    let (s_before, p_before) = (
        steps(s.get() - RequestId::ISOLATE_FIRST + 1),
        steps(probe_handle),
    );
    for sequence in 1..=5u32 {
        post_page(
            &iso,
            &mut buffer,
            1,
            &[
                prompted(
                    a,
                    NativeKind::BankPlan,
                    sequence,
                    "match",
                    serde_json::json!(["Item"]),
                ),
                live(s, NativeKind::BankNearest),
            ],
            &[],
        );
        dispatch(&iso, 1, 0);
    }
    assert_eq!(
        iso.probe("globalThis.__visits").unwrap(),
        serde_json::json!(5)
    );
    assert_eq!(
        steps(s.get() - RequestId::ISOLATE_FIRST + 1),
        s_before,
        "the sibling row was not stepped"
    );
    assert_eq!(
        steps(probe_handle),
        p_before,
        "the snapshot-sensitive row was not stepped"
    );
    assert_eq!(
        iso.probe("globalThis.__wait_done").unwrap(),
        serde_json::json!(false),
        "the parked wait is still parked"
    );
    assert_eq!(
        iso.probe("__rs2b0t_host.tick").unwrap(),
        tick,
        "no tick time moved"
    );
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
         runMachine('bank_nearest', {}, { deposit: () => false, afterDeposit: () => true, log: () => {} }).then(
             v => { globalThis.__got.n = 'done'; }, e => { globalThis.__got.n = 'threw'; });
         runMachine('bank_wait', { kind: 'ready' }, { log: () => {} }).then(
             v => { globalThis.__got.n3 = 'done'; }, e => { globalThis.__got.n3 = 'threw'; });",
    );
    let ids = started(&iso);
    let [c, n, n3] = [ids[0], ids[1], ids[2]];
    let mut buffer = crate::wire::IsolateBuf::default();
    let rows = [
        live(c, NativeKind::BankPlan),
        live(n, NativeKind::BankNearest),
        live(n3, NativeKind::BankPlan),
    ];
    post_page(&iso, &mut buffer, 1, &rows, &[]);
    dispatch(&iso, 1, 0);
    // Page P0 carries C's terminal with sequence 5 and nothing dispatches it.
    post_page(
        &iso,
        &mut buffer,
        1,
        &rows[1..],
        &[result(c, 5, SeatTerminal::Done)],
    );
    let before: Vec<_> = settled(&iso)
        .iter()
        .map(|entry| {
            (
                entry["handle"].as_u64().unwrap(),
                entry["seat_seq"].as_u64().unwrap(),
            )
        })
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
    // Every stamp is cleared, so the seat page releases nothing of the old
    // session: a compat script's awaits settle at the next global settle.
    let keys = |iso: &LoadIsolate| iso.probe("Object.keys(globalThis.__got).sort()").unwrap();
    let after_page = keys(&iso);
    assert_eq!(after_page, keys(&iso));
    assert_eq!(
        iso.machine_host_for_test()["settled"]
            .as_array()
            .map(Vec::len),
        Some(3),
        "C, N and N3 still wait for the global settle"
    );
    iso.join();
}

/// The `Tick` variant carries a mode instead of a flag, and `IsolateCmd` is
/// still 80 B, as on `d65c6bdd8` (ADR 0017 section 7.1).
#[test]
#[cfg(target_pointer_width = "64")]
fn isolate_cmd_size_is_pinned() {
    assert_eq!(std::mem::size_of::<IsolateCmd>(), 80);
}

/// The names of the hooks a script recorded, in call order.
fn called(iso: &LoadIsolate) -> Vec<serde_json::Value> {
    iso.probe("globalThis.__calls")
        .unwrap()
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn family_row(iso: &LoadIsolate, family: &str) -> serde_json::Value {
    iso.machine_host_for_test()["rows"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["family"] == family).cloned())
        .unwrap_or_else(|| panic!("a live {family} row"))
}

/// ADR 0017 N10 (Main's ruling), `periodic_bank`: a seat page that advances
/// the prompt of the embedded child steps the row that owns it, through
/// `TaskPhase::Run` and the child alone. After the child's terminal the same
/// step continues into the family's `Log` hook and nothing else. An unrelated
/// local row, the parked wait and the game tick stay where they were.
#[test]
fn an_embedded_periodic_bank_child_steps_only_its_owner_and_continues_into_the_log() {
    let iso = spawn_bank_machine_card(
        "globalThis.__calls = [];
         globalThis.__wait_done = false;
         const rec = (name) => (...args) => { globalThis.__calls.push([name, ...args]); };
         runMachine('periodic_bank', {}, {
             setStatus: rec('setStatus'),
             deposit: (name) => { globalThis.__calls.push(['deposit', name]); return false; },
             afterDeposit: rec('afterDeposit'),
             destination: rec('destination'),
             commonJunk: rec('commonJunk'),
             returnTo: rec('returnTo'),
             log: rec('log'),
         });
         runMachine('probe', { button: 7, steps: 99 });
         Execution.delayTicks(50).then(() => { globalThis.__wait_done = true; });",
    );
    let ids = started(&iso);
    assert_eq!(ids.len(), 1, "the embedded child is the only seat start");
    let child = ids[0];
    let mut buffer = crate::wire::IsolateBuf::default();
    let probe_row = family_row(&iso, "probe")["handle"].as_u64().unwrap();
    let steps_of_probe = |iso: &LoadIsolate| {
        iso.machine_host_for_test()["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["handle"] == probe_row)
            .map_or(0, |row| row["steps"].as_u64().unwrap())
    };
    let startup = called(&iso).len();
    let tick = iso.probe("__rs2b0t_host.tick").unwrap();

    post_page(
        &iso,
        &mut buffer,
        1,
        &[live(child, NativeKind::BankNearest)],
        &[],
    );
    dispatch(&iso, 1, 0);
    assert_eq!(called(&iso).len(), startup, "admission calls no hook");

    let probe_steps = steps_of_probe(&iso);
    let owner_steps = family_row(&iso, "periodic_bank")["steps"].as_u64().unwrap();
    post_page(
        &iso,
        &mut buffer,
        1,
        &[prompted(
            child,
            NativeKind::BankNearest,
            1,
            "deposit",
            serde_json::json!(["Item"]),
        )],
        &[],
    );
    dispatch(&iso, 1, 0);
    assert_eq!(
        called(&iso)[startup..],
        [serde_json::json!(["deposit", "Item"])],
        "the next embedded prompt executes once and nothing else runs"
    );
    assert!(
        family_row(&iso, "periodic_bank")["steps"].as_u64().unwrap() > owner_steps,
        "the owning row stepped"
    );
    assert_eq!(
        steps_of_probe(&iso),
        probe_steps,
        "the unrelated row stayed inactive"
    );

    // The terminal: the owner continues into its own `log` hook, once.
    let after_prompt = called(&iso).len();
    post_page(
        &iso,
        &mut buffer,
        1,
        &[],
        &[result(child, 1, SeatTerminal::Done)],
    );
    dispatch(&iso, 1, 0);
    let delta = &called(&iso)[after_prompt..];
    assert_eq!(delta.len(), 1, "one continuation hook: {delta:?}");
    assert_eq!(delta[0][0], "log");
    assert_eq!(steps_of_probe(&iso), probe_steps, "still inactive");
    assert_eq!(
        iso.probe("globalThis.__wait_done").unwrap(),
        serde_json::json!(false)
    );
    assert_eq!(
        iso.probe("__rs2b0t_host.tick").unwrap(),
        tick,
        "no tick time moved"
    );
    iso.join();
}

/// ADR 0017 N10, `bank_withdraw_to`: its children carry no prompts. While the
/// child is live a seat page steps nothing. The child's terminal steps the
/// owner once, and it continues into its own `count` read and nothing else.
#[test]
fn an_embedded_withdraw_child_delivers_its_terminal_to_its_owner_only() {
    let iso = spawn_bank_machine_card(
        "globalThis.__calls = [];
         runMachine('bank_withdraw_to', { name: 'Shark', target: 5 }, {
             count: () => { globalThis.__calls.push(['count']); return 0; },
         });
         runMachine('probe', { button: 7, steps: 99 });",
    );
    let ids = started(&iso);
    assert_eq!(ids.len(), 1, "the withdraw child is the only seat start");
    let child = ids[0];
    let mut buffer = crate::wire::IsolateBuf::default();
    let probe_row = family_row(&iso, "probe")["handle"].as_u64().unwrap();
    let probe_steps = |iso: &LoadIsolate| {
        iso.machine_host_for_test()["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["handle"] == probe_row)
            .map_or(0, |row| row["steps"].as_u64().unwrap())
    };
    let startup = called(&iso).len();
    assert_eq!(
        startup, 3,
        "the start phases Start, Check and Before each read the count"
    );
    post_page(
        &iso,
        &mut buffer,
        1,
        &[live(child, NativeKind::BankPlan)],
        &[],
    );
    dispatch(&iso, 1, 0);
    assert_eq!(
        called(&iso).len(),
        startup,
        "a live child with no prompt steps nothing"
    );
    let before = probe_steps(&iso);
    post_page(
        &iso,
        &mut buffer,
        1,
        &[],
        &[result(child, 1, SeatTerminal::Done)],
    );
    dispatch(&iso, 1, 0);
    assert_eq!(
        called(&iso).len(),
        startup + 1,
        "the owner stepped once and read its count: {:?}",
        called(&iso)
    );
    assert_eq!(
        probe_steps(&iso),
        before,
        "the unrelated row stayed inactive"
    );
    iso.join();
}

/// ADR 0017 N10, `hunt` (the bank run embeds a native `bank_open` child):
/// the public `bankRun` path on frozen ticks and frozen facts. While the
/// child is live the owning row steps through the child alone: no hook runs
/// and no other row moves. After the child's terminal the same step runs the
/// family's own continuation (here: the stepper's `log` note and its own
/// end), and nothing else.
#[test]
fn an_embedded_hunt_child_steps_only_its_owner_and_continues_into_the_family() {
    let iso = spawn_bank_machine_card(
        "globalThis.__calls = [];
         globalThis.__wait_done = false;
         const rec = (name) => (...args) => { globalThis.__calls.push([name, ...args]); };
         runMachine('hunt-bank', {
             site: { key: 'heroes-blue', keyItem: null, boxes: [], bank: { x: 80, z: 80, level: 0 }, style: 'range' },
         }, {
             log: rec('log'), vlog: rec('vlog'), setStatus: rec('setStatus'), eatOnce: rec('eatOnce'),
             parkFor: rec('parkFor'), countBankTrip: rec('countBankTrip'), setTarget: rec('setTarget'),
             hasFood: rec('hasFood'), needEat: rec('needEat'), leave: rec('leave'), sustain: rec('sustain'),
         }).then(v => { globalThis.__done = v; }, e => { globalThis.__done = { threw: String(e) }; });
         runMachine('probe', { button: 7, steps: 99 });
         Execution.delayTicks(50).then(() => { globalThis.__wait_done = true; });",
    );
    let mut buffer = crate::wire::IsolateBuf::default();
    // The facts stay as they are for the whole trace: the player stands at
    // the bank, the bank is closed. Only the seat pages change.
    let standing = || {
        let mut input = crate::wire::tests::empty_input(1);
        input.here = Some(script::native::snapshot_source::TileInput {
            x: 80,
            z: 80,
            level: 0,
        });
        input
    };
    let first = buffer.encode_snapshot_with_native(&standing(), Default::default());
    assert!(iso.post_snapshot(first));
    let ids = started(&iso);
    assert_eq!(ids.len(), 1, "the embedded opener is the only seat start");
    let child = ids[0];
    let probe_row = family_row(&iso, "probe")["handle"].as_u64().unwrap();
    let steps_of_probe = |iso: &LoadIsolate| {
        iso.machine_host_for_test()["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["handle"] == probe_row)
            .map_or(0, |row| row["steps"].as_u64().unwrap())
    };
    let startup = called(&iso).len();
    let probe_steps = steps_of_probe(&iso);
    let owner_handle = family_row(&iso, "hunt-bank")["handle"].as_u64().unwrap();
    let tick = iso.probe("__rs2b0t_host.tick").unwrap();
    assert_eq!(
        iso.probe("typeof globalThis.__done").unwrap(),
        serde_json::json!("undefined"),
        "the run is parked on its child"
    );

    // The child is live and nothing about it changed: the owner steps through
    // the child alone and nothing is called.
    post_page_in(
        &iso,
        &mut buffer,
        standing(),
        &[live(child, NativeKind::BankPlan)],
        &[],
    );
    dispatch(&iso, 1, 0);
    assert_eq!(called(&iso).len(), startup, "a live child runs no hook");
    assert_eq!(
        steps_of_probe(&iso),
        probe_steps,
        "the unrelated row is inactive"
    );
    assert_eq!(
        iso.probe("typeof globalThis.__done").unwrap(),
        serde_json::json!("undefined"),
        "the family has not continued"
    );

    // The child's terminal: the same step continues into the family.
    let after_live = called(&iso).len();
    post_page_in(
        &iso,
        &mut buffer,
        standing(),
        &[],
        &[result(child, 1, SeatTerminal::Done)],
    );
    dispatch(&iso, 1, 0);
    let delta = &called(&iso)[after_live..];
    assert_eq!(
        delta,
        [serde_json::json!([
            "log",
            "could not open the bank. Will retry."
        ])],
        "exactly the family's own continuation ran"
    );
    let ended = settled(&iso)
        .into_iter()
        .find(|entry| entry["handle"] == owner_handle)
        .unwrap_or_else(|| panic!("the owner settled: {:?}", settled(&iso)));
    assert_eq!(ended["kind"], "done", "the family ended on its own stepper");
    assert_eq!(steps_of_probe(&iso), probe_steps, "still inactive");
    assert_eq!(
        iso.probe("globalThis.__wait_done").unwrap(),
        serde_json::json!(false),
        "the parked wait stayed parked"
    );
    assert_eq!(
        iso.probe("__rs2b0t_host.tick").unwrap(),
        tick,
        "no tick time moved"
    );
    iso.join();
}

/// Sol N1 and Grok F1: a slow tick makes the thread skip every queued game
/// tick and evidence wake of its window, but never an accepted seat page. The
/// retained page runs after the slow tick, with its own floor, and its paired
/// snapshot applied first.
#[test]
fn slow_tick_skip_keeps_the_queued_seat_page() {
    let iso = spawn_bank_machine_card(
        "globalThis.__visits = 0;
         runMachine('bank_deposit', {}, { match: () => { globalThis.__visits += 1; return false; } });
         const spun = Date.now();
         while (Date.now() - spun < 700) {}",
    );
    let id = RequestId::isolate(1).expect("the first machine handle");
    let mut buffer = crate::wire::IsolateBuf::default();
    // The slow tick is running (or about to). Everything below queues behind it,
    // in the order of one host frame: the seat page with prompt 1 and its
    // dispatch, then the next tick's snapshot and its game tick.
    iso.on_game_tick(1);
    post_page(
        &iso,
        &mut buffer,
        1,
        &[prompted(
            id,
            NativeKind::BankPlan,
            1,
            "match",
            serde_json::json!(["Item"]),
        )],
        &[],
    );
    iso.enqueue_seat_page_without_watchdog_for_test(1, 0);
    let tick_two = buffer.encode_snapshot_with_native(
        &crate::wire::tests::empty_input(2),
        script::native::snapshot_source::NativeFactsInput::default(),
    );
    assert!(iso.post_snapshot(tick_two));
    iso.enqueue_tick_without_watchdog_for_test(2);
    assert_eq!(
        iso.probe("globalThis.__visits").unwrap(),
        serde_json::json!(1),
        "the retained seat page ran its prompt hook once, after the slow tick"
    );
    let logs = iso.drain_logs();
    assert!(
        logs.iter().any(|line| line == "skipped stale ticks -> 2"),
        "{logs:?}"
    );
    assert_eq!(
        iso.probe("__rs2b0t_host.tick").unwrap(),
        serde_json::json!(1),
        "tick 2 was skipped as stale, so it did not run"
    );
    assert_eq!(
        iso.probe("__rs2b0t_host.snapshot.tick").unwrap(),
        serde_json::json!(2),
        "its snapshot still applied"
    );
    assert_eq!(iso.executed_seat_pages_for_test(), 1);
    machine_tick(&iso, 3);
    assert_eq!(
        iso.probe("__rs2b0t_host.tick").unwrap(),
        serde_json::json!(3),
        "a fresh tick runs"
    );
    iso.join();
}
