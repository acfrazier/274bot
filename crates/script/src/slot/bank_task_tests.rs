//! Contract tests for the native bank tasks behind `bankPace`,
//! `Bank.backpackReady`, `withdrawCoins` and `purgePackAtBank`
//! (COMPAT-FINISH-029 census rows `debt:004-006`, `earlier-census:001`).

use super::*;
use crate::quester::families::tests::with_tick;
use api::obj_names::ItemDefView;
use api::selected::RunKey;
use api::snapshot::{GameSnapshot, ItemActionFamily, ItemContainer};

fn item(id: i32, name: &str, count: i32, slot: i32, container: ItemContainer) -> ItemView {
    ItemView {
        def: ItemDefView {
            id,
            name: Some(name.to_owned()),
            stackable: id == 995,
            members: false,
            base_value: 0,
            noted: false,
            certificate_link: -1,
            certificate_template: -1,
        },
        container,
        action_family: if container == ItemContainer::Inventory {
            ItemActionFamily::Held
        } else {
            ItemActionFamily::Component
        },
        slot,
        count,
        actions: Vec::new(),
        component_id: 7,
    }
}

fn pack(id: i32, name: &str, count: i32, slot: i32) -> ItemView {
    item(id, name, count, slot, ItemContainer::Inventory)
}

fn side(id: i32, name: &str, count: i32, slot: i32) -> ItemView {
    item(id, name, count, slot, ItemContainer::BankSide)
}

fn banked(id: i32, name: &str, count: i32, slot: i32) -> ItemView {
    item(id, name, count, slot, ItemContainer::Bank)
}

fn closed(pack_rows: Vec<ItemView>) -> GameSnapshot {
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_inventory(pack_rows, 28);
    snapshot
}

fn open(
    pack_rows: Vec<ItemView>,
    bank_rows: Vec<ItemView>,
    side_rows: Vec<ItemView>,
    side_ready: bool,
) -> GameSnapshot {
    let mut snapshot = closed(pack_rows);
    snapshot.seed_bank_observation(10, 41, Some(bank_rows), side_rows);
    snapshot.seed_transmission_ready(true, side_ready);
    snapshot
}

fn row(request: BankSeatArgs) -> BankRow {
    BankRow {
        id: RequestId::isolate(1).expect("test request id"),
        runtime_generation: 1,
        selection_generation: None,
        selected_bank: None,
        family: family(request.operation),
        job: Job::Start(request),
        trip: None,
        task: None,
        hooks: Arc::new(Mutex::new(TransferHooks::default())),
        prompt_posted: false,
    }
}

fn with_log(mut request: BankSeatArgs) -> BankSeatArgs {
    request.hooks_present = HOOK_LOG;
    request
}

fn poll_row(
    row: &mut BankRow,
    snapshot: &GameSnapshot,
    ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
    tick: u64,
) -> Poll<Result<Value, ActionError>> {
    let facts = Arc::new(NamedBankFacts::empty());
    let preferences = BankPreferences {
        use_mage_bank: false,
        use_zanaris_bank: false,
    };
    with_tick(snapshot, ledger, tick, |native| {
        let mut note = None;
        row.poll(
            native.actions,
            &mut native.cx,
            None,
            &facts,
            preferences,
            &mut note,
            None,
        )
    })
}

/// Answer the pending `log` call and return its message.
fn answer_log(row: &BankRow) -> String {
    let mut hooks = row.hooks.lock().expect("hooks");
    let call = hooks.pending.take().expect("a pending log call");
    assert_eq!(call.hook, "log");
    hooks.answer = Some(TransferAnswer::Value(Value::Null));
    call.args[0].as_str().expect("log message").to_owned()
}

fn settled(job: &Job) -> Option<&Value> {
    match job {
        Job::Settled(value) => Some(value),
        _ => None,
    }
}

fn logged(job: &Job) -> Option<(&str, &Job)> {
    match job {
        Job::Log { message, then, .. } => Some((message.as_str(), then)),
        _ => None,
    }
}

fn started(job: &Job) -> Option<&BankSeatArgs> {
    match job {
        Job::Start(request) => Some(request),
        _ => None,
    }
}

fn start_task(row: &mut BankRow, snapshot: &GameSnapshot, request: BankSeatArgs) -> Job {
    let mut ledger = None;
    with_tick(snapshot, &mut ledger, 1, |native| {
        row.start_task(request, &native.cx)
    })
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_bank__bankpace__contract() {
    let snapshot = closed(Vec::new());
    let mut ledger = None;
    let mut pace = row(BankSeatArgs::new(BankSeatOperation::Pace));
    // The first poll accepts on the current PLAYER_INFO tick and waits.
    assert!(poll_row(&mut pace, &snapshot, &mut ledger, 5).is_pending());
    // Further frames of the same tick do not spend game progress, however
    // much host time passes: there is no elapsed-time floor.
    assert!(poll_row(&mut pace, &snapshot, &mut ledger, 5).is_pending());
    assert!(poll_row(&mut pace, &snapshot, &mut ledger, 5).is_pending());
    // Exactly the next observed tick settles it.
    assert!(matches!(
        poll_row(&mut pace, &snapshot, &mut ledger, 6),
        Poll::Ready(Ok(Value::Null))
    ));

    // Another run never inherits an accepted pause.
    let mut replaced = row(BankSeatArgs::new(BankSeatOperation::Pace));
    replaced.job = Job::Pace {
        accepted: Some(EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 0,
                session: 1,
            },
            tick: 5,
            sequence: 5,
        }),
    };
    assert!(matches!(
        poll_row(&mut replaced, &snapshot, &mut ledger, 6),
        Poll::Ready(Err(ActionError::Cancelled))
    ));
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_bank__bank_backpackready__contract() {
    let before = vec![pack(1511, "Logs", 1, 0), pack(995, "Coins", 25, 3)];
    let posted = vec![side(1511, "Logs", 1, 0), side(995, "Coins", 25, 3)];
    let expected = vec![
        BankPackRow {
            slot: 0,
            id: 1511,
            count: 1,
        },
        BankPackRow {
            slot: 3,
            id: 995,
            count: 25,
        },
    ];
    let mut ledger = None;

    // Readiness only: a ready side pack of this session answers true.
    let ready = open(before.clone(), Vec::new(), posted.clone(), true);
    let mut plain = row(BankSeatArgs::new(BankSeatOperation::BackpackReady));
    assert!(matches!(
        poll_row(&mut plain, &ready, &mut ledger, 3),
        Poll::Ready(Ok(Value::Bool(true)))
    ));

    // The same expectation matches slot, id and count.
    let mut request = with_log(BankSeatArgs::new(BankSeatOperation::BackpackReady));
    request.expected_pack = Some(expected.clone());
    let mut matching = row(request.clone());
    assert!(matches!(
        poll_row(&mut matching, &ready, &mut ledger, 3),
        Poll::Ready(Ok(Value::Bool(true)))
    ));

    // An open bank modal whose side pack has not been transmitted for this
    // session stays pending; the matching transmission then completes true.
    let stale = open(before.clone(), Vec::new(), posted.clone(), false);
    let mut waiting = row(request.clone());
    assert!(poll_row(&mut waiting, &stale, &mut ledger, 3).is_pending());
    assert!(poll_row(&mut waiting, &stale, &mut ledger, 4).is_pending());
    assert!(matches!(
        poll_row(&mut waiting, &ready, &mut ledger, 5),
        Poll::Ready(Ok(Value::Bool(true)))
    ));

    // A different count never matches; after the observed-tick budget the
    // contract log runs once and the answer is false.
    let changed = open(
        before.clone(),
        Vec::new(),
        vec![side(1511, "Logs", 1, 0), side(995, "Coins", 24, 3)],
        true,
    );
    let mut mismatch = row(request.clone());
    assert!(poll_row(&mut mismatch, &changed, &mut ledger, 10).is_pending());
    assert!(poll_row(
        &mut mismatch,
        &changed,
        &mut ledger,
        10 + TRANSFER_OBSERVED_TICK_LIMIT - 1
    )
    .is_pending());
    assert!(poll_row(
        &mut mismatch,
        &changed,
        &mut ledger,
        10 + TRANSFER_OBSERVED_TICK_LIMIT
    )
    .is_pending());
    assert_eq!(
        answer_log(&mismatch),
        "bank: side backpack did not match the pre-open backpack"
    );
    assert!(matches!(
        poll_row(&mut mismatch, &changed, &mut ledger, 20),
        Poll::Ready(Ok(Value::Bool(false)))
    ));

    // A closed bank fails the readiness stage with its own log line.
    let shut = closed(before);
    let mut closed_bank = row(request);
    assert!(poll_row(&mut closed_bank, &shut, &mut ledger, 3).is_pending());
    assert_eq!(
        answer_log(&closed_bank),
        "bank: side backpack modal did not become ready"
    );
    assert!(matches!(
        poll_row(&mut closed_bank, &shut, &mut ledger, 3),
        Poll::Ready(Ok(Value::Bool(false)))
    ));
}

#[test]
fn backpack_expectations_compare_like_upstream_same_backpack() {
    let posted = [side(1, "A", 1, 0), side(2, "B", 5, 1)];
    let exact = [
        BankPackRow {
            slot: 1,
            id: 2,
            count: 5,
        },
        BankPackRow {
            slot: 0,
            id: 1,
            count: 1,
        },
    ];
    assert!(same_backpack(&posted, &exact), "order is not significant");
    assert!(!same_backpack(&posted, &exact[..1]), "length must match");
    assert!(!same_backpack(
        &posted,
        &[exact[0], crate::bank::request::IMPOSSIBLE_PACK_ROW]
    ));
    assert!(
        same_backpack(&[], &[]),
        "an empty pack matches an empty expectation"
    );
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_bank__withdrawcoins__contract() {
    let coins = |need: i64| {
        let mut request = with_log(BankSeatArgs::new(BankSeatOperation::WithdrawCoins));
        request.item_name = Some("Coins".into());
        request.quantity = Some(need);
        request
    };
    let held = vec![pack(995, "Coins", 100, 0)];

    // `need` is a held target: holding enough already answers true.
    let enough = open(
        held.clone(),
        vec![banked(995, "Coins", 1_000, 0)],
        Vec::new(),
        true,
    );
    let mut done = row(coins(100));
    assert_eq!(
        settled(&start_task(&mut done, &enough, coins(100))),
        Some(&json!(true))
    );

    // A closed bank answers false without an action.
    let shut = closed(held.clone());
    let mut closed_bank = row(coins(500));
    assert_eq!(
        settled(&start_task(&mut closed_bank, &shut, coins(500))),
        Some(&json!(false))
    );

    // No banked coins logs the contract line, then answers false.
    let empty = open(held.clone(), Vec::new(), Vec::new(), true);
    let mut none = row(coins(500));
    let job = start_task(&mut none, &empty, coins(500));
    let (message, then) = logged(&job).expect("log before false");
    assert_eq!(message, "acquire: no coins in bank (need 500gp)");
    assert_eq!(settled(then), Some(&json!(false)));

    // The deficit is withdrawn as one delta of the held target's shortfall.
    let stocked = open(
        held.clone(),
        vec![banked(995, "Coins", 1_000, 0)],
        Vec::new(),
        true,
    );
    let mut full = row(coins(500));
    let job = start_task(&mut full, &stocked, coins(500));
    let (message, then) = logged(&job).expect("log before the withdrawal");
    assert_eq!(message, "acquire: withdraw 400gp");
    let withdraw = started(then).expect("a typed withdrawal");
    assert_eq!(withdraw.operation, BankSeatOperation::WithdrawDelta);
    assert_eq!(withdraw.item_name.as_deref(), Some("Coins"));
    assert_eq!(withdraw.quantity, Some(400));
    let mut ledger = None;
    let after = open(
        vec![pack(995, "Coins", 500, 0)],
        Vec::new(),
        Vec::new(),
        true,
    );
    let short = open(
        vec![pack(995, "Coins", 400, 0)],
        Vec::new(),
        Vec::new(),
        true,
    );
    with_tick(&after, &mut ledger, 2, |native| {
        let job = full
            .task_transfer_done(Ok(TransferValue::Bool(true)), &native.cx)
            .expect("coins task")
            .expect("no error");
        assert_eq!(settled(&job), Some(&json!(true)));
    });
    with_tick(&short, &mut ledger, 2, |native| {
        let job = full
            .task_transfer_done(Ok(TransferValue::Bool(true)), &native.cx)
            .expect("coins task")
            .expect("no error");
        assert_eq!(
            settled(&job),
            Some(&json!(false)),
            "the fresh count decides"
        );
        let job = full
            .task_transfer_done(Ok(TransferValue::Bool(false)), &native.cx)
            .expect("coins task")
            .expect("no error");
        assert_eq!(settled(&job), Some(&json!(false)));
    });

    // A partial bank withdraws what it has and succeeds against
    // `min(need, held + withdrawn)`.
    let partial = open(held, vec![banked(995, "Coins", 150, 0)], Vec::new(), true);
    let mut some = row(coins(500));
    let job = start_task(&mut some, &partial, coins(500));
    let (message, then) = logged(&job).expect("log before the withdrawal");
    assert_eq!(message, "acquire: withdraw 150gp");
    assert_eq!(
        started(then).and_then(|request| request.quantity),
        Some(150)
    );
    let partial_after = open(
        vec![pack(995, "Coins", 250, 0)],
        Vec::new(),
        Vec::new(),
        true,
    );
    with_tick(&partial_after, &mut ledger, 3, |native| {
        let job = some
            .task_transfer_done(Ok(TransferValue::Bool(true)), &native.cx)
            .expect("coins task")
            .expect("no error");
        assert_eq!(settled(&job), Some(&json!(true)));
    });
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_bank__purgepackatbank__contract() {
    let purge = |keep: &[&str]| {
        let mut request = with_log(BankSeatArgs::new(BankSeatOperation::PurgePack));
        request.keep_names = keep.iter().map(|name| (*name).to_owned()).collect();
        request.obstacles = vec!["door".into()];
        request.prefer_nearby = Some(true);
        request
    };

    // A clean pack (keep names are case-insensitive) needs no bank.
    let clean = closed(vec![pack(1265, "Bronze pickaxe", 1, 0)]);
    let mut nothing = row(purge(&["bronze PICKAXE"]));
    assert_eq!(
        settled(&start_task(
            &mut nothing,
            &clean,
            purge(&["bronze PICKAXE"])
        )),
        Some(&json!(true))
    );

    // Junk logs the contract line and opens with the caller's bank options.
    let junk = closed(vec![
        pack(1265, "Bronze pickaxe", 1, 0),
        pack(436, "Copper ore", 1, 1),
        pack(438, "Tin ore", 1, 2),
    ]);
    let mut purging = row(purge(&["Bronze pickaxe"]));
    let job = start_task(&mut purging, &junk, purge(&["Bronze pickaxe"]));
    let (message, then) = logged(&job).expect("log before opening");
    assert_eq!(
        message,
        "start purge: banking 2 stack(s) (keep: Bronze pickaxe)"
    );
    let opening = started(then).expect("a typed open");
    assert_eq!(opening.operation, BankSeatOperation::Open);
    assert_eq!(opening.obstacles, vec!["door".to_owned()]);
    assert_eq!(opening.prefer_nearby, Some(true));
    assert_eq!(opening.hooks_present, HOOK_LOG);

    // A failed open leaves the pack and answers false after its log.
    assert!(purging.after_open(false).is_none());
    let (message, then) = logged(&purging.job).expect("log after a failed open");
    assert_eq!(
        message,
        "start purge: could not open bank — leaving pack as-is"
    );
    assert_eq!(settled(then), Some(&json!(false)));

    // An open bank deposits every unkept row by the kept names.
    let mut opened = row(purge(&["Bronze pickaxe"]));
    let _ = start_task(&mut opened, &junk, purge(&["Bronze pickaxe"]));
    assert!(opened.after_open(true).is_none());
    let deposit = started(&opened.job).expect("a typed deposit");
    assert_eq!(deposit.operation, BankSeatOperation::DepositKeep);
    assert_eq!(deposit.keep_names, vec!["Bronze pickaxe".to_owned()]);

    // After the deposit the bank closes; the close result is ignored and the
    // fresh pack decides.
    let mut ledger = None;
    with_tick(&junk, &mut ledger, 4, |native| {
        let job = opened
            .task_transfer_done(Ok(TransferValue::Void), &native.cx)
            .expect("purge task")
            .expect("no error");
        let close = started(&job).expect("a typed close");
        assert_eq!(close.operation, BankSeatOperation::Close);
        let left = opened
            .task_transfer_done(Err(ActionError::Cancelled), &native.cx)
            .expect("purge task")
            .expect("close errors are ignored");
        let (message, then) = logged(&left).expect("log after the check");
        assert_eq!(
            message,
            "start purge: still holding 2 junk stack(s) after deposit"
        );
        assert_eq!(settled(then), Some(&json!(false)));
    });
    let mut cleared = row(purge(&["Bronze pickaxe"]));
    let _ = start_task(&mut cleared, &junk, purge(&["Bronze pickaxe"]));
    assert!(cleared.after_open(true).is_none());
    with_tick(&clean, &mut ledger, 5, |native| {
        let _ = cleared
            .task_transfer_done(Ok(TransferValue::Void), &native.cx)
            .expect("purge task");
        let done = cleared
            .task_transfer_done(Ok(TransferValue::Bool(true)), &native.cx)
            .expect("purge task")
            .expect("no error");
        let (message, then) = logged(&done).expect("log after the check");
        assert_eq!(message, "start purge: pack cleared");
        assert_eq!(settled(then), Some(&json!(true)));
    });
}

/// Poll `row` on a frame of the same run whose session is `session` (a kept
/// reconnect re-keys the run's session, not the run).
fn poll_row_in_session(
    row: &mut BankRow,
    snapshot: &GameSnapshot,
    ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
    tick: u64,
    session: u64,
) -> Poll<Result<Value, ActionError>> {
    let facts = Arc::new(NamedBankFacts::empty());
    let preferences = BankPreferences {
        use_mage_bank: false,
        use_zanaris_bank: false,
    };
    with_tick(snapshot, ledger, tick, |native| {
        native.cx.evidence.run.session = session;
        let mut note = None;
        row.poll(
            native.actions,
            &mut native.cx,
            None,
            &facts,
            preferences,
            &mut note,
            None,
        )
    })
}

/// Contract v5 R5-7 / R5-8.5: a kept reconnect holds request state. A sort
/// that already sent a drag resumes on the new session and reports that
/// progress through the closed-bank round, not an invented empty result.
#[test]
fn a_kept_reconnect_holds_sort_progress() {
    let mut snapshot = open(
        Vec::new(),
        vec![
            banked(438, "Tin ore", 1, 0),
            banked(436, "Copper ore", 1, 1),
        ],
        Vec::new(),
        true,
    );
    snapshot.seed_varps(vec![api::snapshot::VarpView {
        index: crate::bank::seat_sort::BANK_INSERT_VARP,
        value: 0,
    }]);
    let mut ledger = None;
    let mut sort = row(BankSeatArgs::new(BankSeatOperation::Sort));
    assert!(poll_row(&mut sort, &snapshot, &mut ledger, 2).is_pending());
    assert!(poll_row(&mut sort, &snapshot, &mut ledger, 3).is_pending());
    assert!(matches!(sort.job, Job::Sort(_)), "one drag is in flight");

    let job = std::mem::replace(&mut sort.job, Job::Finished);
    sort.job = job.held_across_kept_reconnect(NativeKind::BankPlan);
    assert!(matches!(sort.job, Job::ResumeSort { .. }));
    ledger
        .as_mut()
        .expect("the sort's ledger")
        .revoke_foreground();

    let reconnected = closed(Vec::new());
    match poll_row_in_session(&mut sort, &reconnected, &mut ledger, 4, 2) {
        Poll::Ready(Ok(value)) => assert_eq!(
            value,
            json!({
                "sorted": false,
                "moves": 1,
                "mode": "swap",
                "unmatched": [],
                "reason": "bank closed",
            })
        ),
        other => panic!(
            "the resumed sort settles, got pending={}",
            other.is_pending()
        ),
    }
}

/// A kept reconnect holds a pause: it restarts on the new session's own
/// observed ticks instead of ending.
#[test]
fn a_kept_reconnect_holds_a_pause() {
    let snapshot = closed(Vec::new());
    let mut ledger = None;
    let mut pace = row(BankSeatArgs::new(BankSeatOperation::Pace));
    assert!(poll_row(&mut pace, &snapshot, &mut ledger, 5).is_pending());
    assert!(poll_row_in_session(&mut pace, &snapshot, &mut ledger, 1, 2).is_pending());
    assert!(matches!(
        poll_row_in_session(&mut pace, &snapshot, &mut ledger, 2, 2),
        Poll::Ready(Ok(Value::Null))
    ));
}
