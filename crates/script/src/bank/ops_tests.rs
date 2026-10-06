use super::*;
use api::obj_names::ItemDefView;
use api::snapshot::{ItemActionFamily, ItemContainer};

const COINS: i32 = 995;
const TROUT: i32 = 333;
const TROUT_NOTE: i32 = 334;

fn native(
    id: i32,
    name: &str,
    count: i32,
    slot: i32,
    component_id: i32,
    actions: &[Option<&str>],
) -> ItemView {
    ItemView {
        def: ItemDefView {
            id,
            name: Some(name.to_owned()),
            stackable: false,
            members: false,
            base_value: 0,
            noted: false,
            certificate_link: -1,
            certificate_template: -1,
        },
        container: ItemContainer::Bank,
        action_family: ItemActionFamily::Component,
        slot,
        count,
        actions: actions
            .iter()
            .map(|action| action.map(str::to_owned))
            .collect(),
        component_id,
    }
}

/// A pack row: no ops, only id and count matter to the kernel.
fn held(id: i32, count: i32) -> ItemView {
    native(id, "Held", count, 0, 3214, &[])
}

/// The isolate row the host posts for the same facts: an action hole is an
/// empty label in its slot.
#[cfg(feature = "load")]
fn compat(
    id: i32,
    name: &str,
    count: i32,
    slot: Option<i32>,
    component_id: Option<i32>,
    actions: &[Option<&str>],
) -> ItemRow {
    use std::rc::Rc;
    ItemRow {
        id,
        count,
        name: Some(Rc::from(name)),
        ops: actions
            .iter()
            .map(|action| Rc::from(action.unwrap_or_default()))
            .collect::<crate::observed::Ops>(),
        component_id,
        slot,
        ..ItemRow::default()
    }
}

const LADDER: [Option<&str>; 5] = [
    Some("Withdraw-1"),
    Some("Withdraw-5"),
    Some("Withdraw-10"),
    Some("Withdraw-All"),
    Some("Withdraw-X"),
];
const FIXED_ONLY: [Option<&str>; 3] = [Some("Withdraw-1"), Some("Withdraw-5"), Some("Withdraw-10")];

fn exact(final_count: i32, available: i32, baseline: i32) -> WithdrawGoal {
    WithdrawGoal::exact_at_least(
        COINS,
        Arc::from("Coins"),
        COINS,
        final_count,
        available,
        baseline,
        9,
    )
}

fn clipped(requested: i32, available: i32, baseline: i32) -> WithdrawGoal {
    WithdrawGoal::available_limited(
        COINS,
        Arc::from("Coins"),
        COINS,
        requested,
        available,
        baseline,
        9,
    )
}

fn clicked(req: Option<InteractReq>) -> (String, i32) {
    match req {
        Some(InteractReq::WithdrawX { action, count, .. }) => (action, count),
        other => panic!("expected one Withdraw-X click, got {other:?}"),
    }
}

/// P-row: one kernel decides on a native row and on the isolate row the
/// host posts for it, holes and positions included. Nothing converts one
/// representation into the other.
#[cfg(feature = "load")]
#[test]
fn p_row_both_row_types_make_the_same_decisions() {
    let withdraw_ops = [
        Some("Withdraw-1"),
        Some("Withdraw-5"),
        None,
        Some("Withdraw-X"),
    ];
    let bank_native = native(COINS, "Coins", 50, 4, 5382, &withdraw_ops);
    let bank_compat = compat(COINS, "Coins", 50, Some(4), Some(5382), &withdraw_ops);
    assert_eq!(action_op(&bank_native, "Withdraw-X"), Some(4));
    assert_eq!(action_op(&bank_compat, "Withdraw-X"), Some(4));
    assert_eq!(action_op(&bank_native, "withdraw x"), Some(4));
    let goal = exact(7, 50, 0);
    for remaining in [1, 5, 7, 10, 60] {
        assert_eq!(
            withdraw_click(&bank_native, remaining, &goal),
            withdraw_click(&bank_compat, remaining, &goal),
            "remaining {remaining}"
        );
    }
    assert_eq!(
        withdraw_click(&bank_compat, 7, &goal),
        Some(InteractReq::WithdrawX {
            name: "Coins".into(),
            count: 7,
            bank_item_id: COINS,
            lands_as_id: COINS,
            action: "Withdraw-X".into(),
            bank_generation: 9,
        })
    );

    // A hole before the bulk op: the sweep and the labelled op name the
    // same 1-based slot in both representations.
    let side_ops = [Some("Deposit-1"), None, Some("Deposit-All")];
    let side_native = [native(TROUT, "Trout", 3, 6, 2006, &side_ops)];
    let side_compat = [compat(TROUT, "Trout", 3, Some(6), Some(2006), &side_ops)];
    let want = DepositClick {
        id: TROUT,
        slot: 6,
        component: 2006,
        operation: 3,
    };
    assert_eq!(
        deposit_click(&side_native[0], DepositRequest::Sweep),
        Some(want)
    );
    assert_eq!(
        deposit_click(&side_compat[0], DepositRequest::Sweep),
        Some(want)
    );
    let one = DepositClick {
        operation: 1,
        ..want
    };
    assert_eq!(
        deposit_click(&side_native[0], DepositRequest::Label("Deposit-1")),
        Some(one)
    );
    assert_eq!(
        deposit_click(&side_compat[0], DepositRequest::Label("Deposit-1")),
        Some(one)
    );
    assert_eq!(
        deposit_click(&side_compat[0], DepositRequest::Label("Deposit-5")),
        None,
        "a missing label is not an All substitute"
    );
    let spec = DepositSpec {
        kind: DepositKind::UntilEmpty,
        keep: &[],
        only: None,
    };
    let pack_native = [held(TROUT, 3)];
    let pack_compat = [compat(TROUT, "Trout", 3, None, None, &[])];
    assert_eq!(
        deposit_next(
            &spec,
            Some(&side_native[..]),
            Some(&pack_native[..]),
            false,
            true
        ),
        DepositScan::Click(want)
    );
    assert_eq!(
        deposit_next(
            &spec,
            Some(&side_compat[..]),
            Some(&pack_compat[..]),
            false,
            true
        ),
        DepositScan::Click(want)
    );
    assert_eq!(
        deposit_req(want, 9),
        InteractReq::InvButton {
            id: TROUT,
            slot: 6,
            component: 2006,
            operation: 3,
            bank_generation: 9,
        }
    );

    // Op 0 and out-of-range slots are never ops; holes are not labels.
    for row in [&side_compat[0] as &dyn RowProbe, &side_native[0]] {
        assert_eq!(row.label(0), None);
        assert_eq!(row.label(-1), None);
        assert_eq!(row.label(2), None);
        assert_eq!(row.label(4), None);
        assert_eq!(row.label(3), Some("Deposit-All"));
    }

    // A row the isolate posted without slot or component cannot be clicked.
    let unplaced = compat(TROUT, "Trout", 3, None, Some(2006), &side_ops);
    assert_eq!(deposit_click(&unplaced, DepositRequest::Sweep), None);
    let unplaced = compat(TROUT, "Trout", 3, Some(6), None, &side_ops);
    assert_eq!(deposit_click(&unplaced, DepositRequest::Sweep), None);
    let negative = native(TROUT, "Trout", 3, -1, 2006, &side_ops);
    assert_eq!(deposit_click(&negative, DepositRequest::Sweep), None);
}

/// Object-safe probe so one assertion loop covers both row types.
#[cfg(feature = "load")]
trait RowProbe {
    fn label(&self, op: i32) -> Option<&str>;
}

#[cfg(feature = "load")]
impl<R: BankRow> RowProbe for R {
    fn label(&self, op: i32) -> Option<&str> {
        self.op_label(op)
    }
}

/// P-ladder: the click is not the request. Fixed-only 7 is 5 + 1 + 1, and
/// only the caller-specific policy decides completion.
#[test]
fn p_ladder_click_and_completion_are_separate() {
    let full = native(COINS, "Coins", 50, 0, 5382, &LADDER);
    assert_eq!(
        clicked(withdraw_click(&full, 7, &exact(7, 50, 0))),
        ("Withdraw-X".into(), 7)
    );
    assert_eq!(
        clicked(withdraw_click(&full, 5, &exact(5, 50, 0))),
        ("Withdraw-5".into(), 5)
    );

    // Fixed-only 7: Withdraw-5, still incomplete at 5 (not pack-full),
    // then Withdraw-1 twice.
    let fixed = native(COINS, "Coins", 50, 0, 5382, &FIXED_ONLY);
    let goal = exact(7, 50, 0);
    let mut now = 0;
    let mut clicks = Vec::new();
    while withdraw_progress(&goal, now, false, true) == Progress::Incomplete {
        let (action, count) = clicked(withdraw_click(
            &fixed,
            withdraw_remaining(&goal, now),
            &goal,
        ));
        clicks.push(action);
        now += count;
        assert!(clicks.len() <= 3, "{clicks:?}");
    }
    assert_eq!(clicks, ["Withdraw-5", "Withdraw-1", "Withdraw-1"]);
    assert_eq!(now, 7);
    assert_eq!(
        withdraw_progress(&goal, 5, false, true),
        Progress::Incomplete
    );
    assert_eq!(withdraw_progress(&goal, 7, false, true), Progress::Complete);

    // Separator-insensitive native labels.
    let underscored = native(COINS, "Coins", 50, 0, 5382, &[Some("Withdraw_1")]);
    assert_eq!(
        clicked(withdraw_click(&underscored, 1, &exact(1, 50, 0))),
        ("Withdraw_1".into(), 1)
    );
    assert_eq!(action_op(&underscored, "Withdraw-1"), Some(1));
    assert!(withdraw_click(
        &native(COINS, "Coins", 50, 0, 5382, &[]),
        3,
        &exact(3, 50, 0)
    )
    .is_none());

    // Native exact 7 with stock 5: the target stays 7, live stock bounds
    // the click, held 5 is incomplete even with a full pack.
    for goal in [
        exact(7, 5, 0),
        WithdrawGoal::exact_equal(COINS, Arc::from("Coins"), COINS, 7, 5, 0, 9),
    ] {
        assert_eq!(goal.target, 7);
        let stock5 = native(COINS, "Coins", 5, 0, 5382, &LADDER);
        assert_eq!(
            clicked(withdraw_click(&stock5, withdraw_remaining(&goal, 0), &goal)),
            ("Withdraw-5".into(), 5)
        );
        assert_eq!(
            withdraw_progress(&goal, 5, false, true),
            Progress::Incomplete
        );
        assert_eq!(
            withdraw_progress(&goal, 5, true, true),
            Progress::Incomplete
        );
    }

    // Frozen withdrawX with stock 5 clips once: complete at 5.
    let compat_goal = clipped(7, 5, 0);
    assert_eq!(compat_goal.target, 5);
    assert_eq!(
        withdraw_progress(&compat_goal, 5, false, true),
        Progress::Complete
    );
    // Progressed, then full, below the clipped target: PackFull. Full
    // without progress is not.
    let big = clipped(20, 50, 0);
    assert_eq!(withdraw_progress(&big, 6, true, true), Progress::PackFull);
    assert_eq!(withdraw_progress(&big, 0, true, true), Progress::Incomplete);
    assert_eq!(
        withdraw_progress(&big, 6, false, true),
        Progress::Incomplete
    );
}

/// Completion edges: overshoot, nonzero baseline, stock moving after begin,
/// session replacement.
#[test]
fn withdraw_completion_edges() {
    let equal = WithdrawGoal::exact_equal(COINS, Arc::from("Coins"), COINS, 7, 50, 0, 9);
    assert_eq!(
        withdraw_progress(&equal, 7, false, true),
        Progress::Complete
    );
    assert_eq!(
        withdraw_progress(&equal, 8, false, true),
        Progress::OverTarget
    );
    assert_eq!(
        withdraw_progress(&exact(7, 50, 0), 8, false, true),
        Progress::Complete,
        "at-least accepts more"
    );

    // withdrawX of 7 on top of 3 held, 5 in stock: target 3 + 5.
    let goal = clipped(7, 5, 3);
    assert_eq!(goal.target, 8);
    assert_eq!(withdraw_remaining(&goal, 3), 5);
    assert_eq!(
        withdraw_progress(&goal, 7, false, true),
        Progress::Incomplete
    );
    assert_eq!(withdraw_progress(&goal, 8, false, true), Progress::Complete);
    assert_eq!(withdraw_progress(&goal, 4, true, true), Progress::PackFull);
    assert_eq!(
        withdraw_progress(&goal, 3, true, true),
        Progress::Incomplete
    );

    // Stock drops after begin: live count bounds the click, not the goal.
    let shrunk = native(COINS, "Coins", 2, 0, 5382, &LADDER);
    assert_eq!(
        clicked(withdraw_click(&shrunk, withdraw_remaining(&goal, 3), &goal)),
        ("Withdraw-X".into(), 2)
    );
    assert_eq!(goal.target, 8);
    let empty = native(COINS, "Coins", 0, 0, 5382, &LADDER);
    assert!(withdraw_click(&empty, 5, &goal).is_none());
    let other = native(TROUT, "Trout", 50, 0, 5382, &LADDER);
    assert!(
        withdraw_click(&other, 5, &goal).is_none(),
        "another item's row"
    );
    assert!(
        withdraw_click(&shrunk, 0, &goal).is_none(),
        "nothing remains"
    );

    for goal in [exact(7, 50, 0), equal, clipped(7, 50, 0)] {
        assert_eq!(
            withdraw_progress(&goal, 7, true, false),
            Progress::SessionGone
        );
    }

    // A clip that leaves the target at the baseline is complete only for
    // that zero-stock goal; admission refuses it before a goal exists.
    assert_eq!(clipped(7, 0, 3).target, 3);
}

/// P-match: identity is the obj id. A noted and an unnoted row of one name
/// are two ids; an id keep keeps one, a name keep resolves to both.
#[test]
fn p_match_identity_is_the_obj_id() {
    let side = [
        native(
            TROUT,
            "Trout",
            3,
            0,
            2006,
            &[Some("Deposit-1"), Some("Deposit-All")],
        ),
        native(
            TROUT_NOTE,
            "Trout",
            12,
            1,
            2006,
            &[Some("Deposit-1"), Some("Deposit-All")],
        ),
    ];
    let pack = [held(TROUT, 3), held(TROUT_NOTE, 12)];
    let keep_item = DepositSpec {
        kind: DepositKind::UntilEmpty,
        keep: &[TROUT],
        only: None,
    };
    let click = DepositClick {
        id: TROUT_NOTE,
        slot: 1,
        component: 2006,
        operation: 2,
    };
    assert_eq!(
        deposit_next(&keep_item, Some(&side[..]), Some(&pack[..]), false, true),
        DepositScan::Click(click)
    );
    assert_eq!(
        deposit_req(click, 4),
        InteractReq::InvButton {
            id: TROUT_NOTE,
            slot: 1,
            component: 2006,
            operation: 2,
            bank_generation: 4,
        },
        "the click names the noted row, not a name"
    );

    let named: Vec<i32> = ids_named(&side, "trout").collect();
    assert_eq!(named, [TROUT, TROUT_NOTE]);
    let keep_name = DepositSpec {
        kind: DepositKind::UntilEmpty,
        keep: &named,
        only: None,
    };
    assert_eq!(
        deposit_next(&keep_name, Some(&side[..]), Some(&pack[..]), false, true),
        DepositScan::Done
    );
    assert!(same_name("Trout", "tROUT"));
    assert_eq!(count_id(&pack, TROUT_NOTE), 12);

    // A nameless (cache-miss) row still deposits by identity.
    let nameless = ItemView {
        def: ItemDefView {
            name: None,
            ..side[0].def.clone()
        },
        ..side[0].clone()
    };
    let spec = DepositSpec {
        kind: DepositKind::UntilEmpty,
        keep: &[],
        only: None,
    };
    assert!(matches!(
        deposit_next(&spec, Some(&[nameless][..]), Some(&pack[..]), false, true),
        DepositScan::Click(DepositClick { id: TROUT, .. })
    ));
}

/// P-settle: a posted empty side is not an absent side, and only
/// UntilEmpty may settle on the view bound.
#[test]
fn p_settle_posted_empty_is_not_absent() {
    let product = 436;
    let required = DepositSpec {
        kind: DepositKind::Required,
        keep: &[],
        only: Some(&[product][..]),
    };
    let until_empty = DepositSpec {
        kind: DepositKind::UntilEmpty,
        keep: &[],
        only: None,
    };
    let holding = [held(product, 1)];
    let empty: [ItemView; 0] = [];

    assert_eq!(
        deposit_next(&required, Some(&empty[..]), Some(&holding[..]), false, true),
        DepositScan::MissingRequired
    );
    assert_eq!(
        deposit_next(&required, Some(&empty[..]), Some(&empty[..]), false, true),
        DepositScan::Done
    );
    assert_eq!(
        deposit_next(&required, None, Some(&holding[..]), false, true),
        DepositScan::WaitView
    );
    assert_eq!(
        deposit_next(&required, None, Some(&holding[..]), true, true),
        DepositScan::WaitView,
        "the view bound never manufactures Required success"
    );
    assert_eq!(
        deposit_next(
            &until_empty,
            Some(&empty[..]),
            Some(&holding[..]),
            false,
            true
        ),
        DepositScan::Done,
        "a settled empty side is not a wait"
    );
    assert_eq!(
        deposit_next(&until_empty, None, Some(&holding[..]), false, true),
        DepositScan::WaitView
    );
    assert_eq!(
        deposit_next(&until_empty, None, Some(&holding[..]), true, true),
        DepositScan::Done
    );
    for spec in [&required, &until_empty] {
        assert_eq!(
            deposit_next::<ItemView>(spec, Some(&empty[..]), None, true, true),
            DepositScan::WaitView,
            "an unposted pack never settles"
        );
    }

    // Required: a side row for another id does not satisfy the product.
    let other = [native(1, "Other", 1, 0, 2006, &[Some("Deposit-All")])];
    assert_eq!(
        deposit_next(&required, Some(&other[..]), Some(&holding[..]), false, true),
        DepositScan::MissingRequired
    );
    // Keep wins over only.
    let kept = DepositSpec {
        keep: &[product],
        ..required
    };
    assert_eq!(
        deposit_next(&kept, None, Some(&holding[..]), false, true),
        DepositScan::Done
    );

    // Outside the request's bank session nothing clicks or completes, the
    // settled-empty and view-bound completions included.
    for spec in [&required, &until_empty] {
        for (side, pack, wait_done) in [
            (Some(&empty[..]), Some(&empty[..]), false),
            (Some(&empty[..]), Some(&holding[..]), false),
            (None, Some(&holding[..]), true),
            (Some(&other[..]), Some(&holding[..]), false),
        ] {
            assert_eq!(
                deposit_next(spec, side, pack, wait_done, false),
                DepositScan::SessionGone
            );
        }
    }
}

#[test]
fn capacity_deposit_requires_exact_stackability_and_op() {
    let side = [
        native(
            500,
            "Stack",
            1,
            0,
            2000,
            &[Some("Deposit-1"), Some("Deposit-All")],
        ),
        native(
            501,
            "Singles",
            1,
            1,
            2001,
            &[Some("Deposit-1"), Some("Deposit-All")],
        ),
    ];
    let mut pack = [held(500, 4), held(501, 1), held(502, 1)];
    pack[0].def.stackable = true;
    let keep = [502];
    assert_eq!(
        deposit_capacity_next(&keep, Some(&side[..]), Some(&pack[..]), true),
        DepositScan::Click(DepositClick {
            id: 500,
            slot: 0,
            component: 2000,
            operation: 2,
        })
    );
    assert_eq!(
        deposit_capacity_next(&keep, Some(&side[..]), Some(&pack[1..]), true),
        DepositScan::Click(DepositClick {
            id: 501,
            slot: 1,
            component: 2001,
            operation: 1,
        })
    );

    let wrong_op = [native(501, "Singles", 1, 1, 2001, &[Some("Deposit-All")])];
    assert_eq!(
        deposit_capacity_next(&[], Some(&wrong_op[..]), Some(&pack[1..2]), true),
        DepositScan::MissingRequired,
        "an unstackable must not fall back from missing Deposit-1 to Deposit-All"
    );
    assert_eq!(
        deposit_capacity_next::<ItemView>(&[], None, Some(&pack[..1]), true),
        DepositScan::WaitView
    );
    let empty_side: [ItemView; 0] = [];
    assert_eq!(
        deposit_capacity_next(
            &[500, 501, 502],
            Some(&empty_side[..]),
            Some(&pack[..]),
            true,
        ),
        DepositScan::Done
    );
    assert_eq!(
        deposit_capacity_next(&[], Some(&side[..]), Some(&pack[..]), false),
        DepositScan::SessionGone
    );
}

#[cfg(feature = "load")]
#[test]
fn capacity_deposit_fails_closed_when_stackability_is_unknown() {
    let side = [compat(
        500,
        "Stack",
        1,
        Some(0),
        Some(2000),
        &[Some("Deposit-All")],
    )];
    let pack = [compat(500, "Stack", 4, Some(0), Some(2001), &[])];
    assert_eq!(
        deposit_capacity_next(&[], Some(&side[..]), Some(&pack[..]), true),
        DepositScan::MissingRequired
    );
}

/// The side-root fact, not vector length, decides whether a side posted.
#[test]
fn side_observation_reads_the_root_not_the_length() {
    let empty: [ItemView; 0] = [];
    assert_eq!(side_observation(true, -1, &empty), None);
    assert_eq!(side_observation(true, 700, &empty), Some(&empty[..]));
    assert_eq!(side_observation(false, 700, &empty), None);
    let rows = [held(TROUT, 1)];
    assert_eq!(side_observation(true, -1, &rows), None);
    assert_eq!(side_observation(true, 0, &rows).map(<[_]>::len), Some(1));
}

/// P-note (kernel half): intent is per open bank session.
#[test]
fn p_note_intent_is_per_bank_session() {
    assert_eq!(NoteIntent::default(), NoteIntent::Item);
    assert_eq!(note_intent(4, NoteIntent::Noted, 4), NoteIntent::Noted);
    assert_eq!(note_intent(4, NoteIntent::Noted, 5), NoteIntent::Item);
    assert_eq!(
        note_req(NoteIntent::Item),
        InteractReq::SetNoteMode { on: false }
    );
    assert_eq!(
        note_req(NoteIntent::Noted),
        InteractReq::SetNoteMode { on: true }
    );
    // An explicit landing id is the observation identity, never the source.
    let goal = WithdrawGoal::available_limited(TROUT, Arc::from("Trout"), TROUT_NOTE, 5, 9, 0, 1);
    let row = native(TROUT, "Trout", 9, 0, 5382, &LADDER);
    assert!(matches!(
        withdraw_click(&row, 5, &goal),
        Some(InteractReq::WithdrawX {
            bank_item_id: TROUT,
            lands_as_id: TROUT_NOTE,
            ..
        })
    ));
}

#[test]
fn load_click_prefers_the_rows_all_op() {
    let goal = clipped(3, 50, 0);
    let row = native(COINS, "Coins", 50, 2, 5382, &LADDER);
    assert_eq!(
        load_click(&row, 3, &goal),
        Some(LoadClick::All(InteractReq::InvButton {
            id: COINS,
            slot: 2,
            component: 5382,
            operation: 4,
            bank_generation: 9,
        }))
    );
    let fixed = native(COINS, "Coins", 50, 2, 5382, &FIXED_ONLY);
    assert!(matches!(
        load_click(&fixed, 3, &goal),
        Some(LoadClick::Fill(InteractReq::WithdrawX { count: 1, .. }))
    ));
    assert_eq!(load_all_progress(4, 5, false, 9, true), Progress::Complete);
    assert_eq!(load_all_progress(4, 4, true, 9, true), Progress::Complete);
    assert_eq!(load_all_progress(4, 4, false, 0, true), Progress::Complete);
    assert_eq!(
        load_all_progress(4, 4, false, 9, true),
        Progress::Incomplete
    );
    assert_eq!(
        load_all_progress(4, 5, false, 9, false),
        Progress::SessionGone
    );
}

#[test]
fn close_scan_needs_shut_main_released_side_and_new_session() {
    assert_eq!(close_deadline(None), TRANSFER_BOUND);
    assert_eq!(close_deadline(Some(1_500)), Duration::from_millis(1_500));
    assert_eq!(close_begin(false), Some(CloseScan::AlreadyShut));
    assert_eq!(close_begin(true), None);
    let base = CloseBaseline {
        generation: 3,
        side: 700,
    };
    assert_eq!(
        close_progress(true, 700, 3, base, true, false),
        CloseScan::Waiting
    );
    assert_eq!(
        close_progress(true, 700, 3, base, true, true),
        CloseScan::TimedOut
    );
    assert_eq!(
        close_progress(false, 700, 4, base, true, false),
        CloseScan::SideHeld
    );
    assert_eq!(
        close_progress(false, 700, 4, base, true, true),
        CloseScan::TimedOut
    );
    assert_eq!(
        close_progress(false, -1, 4, base, true, false),
        CloseScan::Complete
    );
    assert_eq!(
        close_progress(false, -1, 3, base, true, false),
        CloseScan::Waiting
    );
    assert_eq!(
        close_progress(true, 700, 5, base, true, false),
        CloseScan::SessionReplaced
    );
    assert_eq!(
        close_progress(false, -1, 4, base, false, false),
        CloseScan::SessionReplaced
    );
}

#[test]
fn pack_full_counts_occupied_slots() {
    let pack: Vec<ItemView> = (0..28).map(|slot| held(slot, 1)).collect();
    assert!(pack_full(&pack, 28));
    assert!(!pack_full(&pack[..27], 28));
    assert!(!pack_full(&pack, 0), "an unposted size is never full");
}
