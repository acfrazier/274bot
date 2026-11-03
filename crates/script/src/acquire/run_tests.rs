use super::*;
use crate::bank::{AccessKind, BankPickReceipt, BankStandAccess, PickKind, SelectedBank};
use crate::native::ledger::{HostEffect, Ledger};
use crate::native::{ActionError, InteractionReceipt, WalkEnd};
use crate::native_bank::{BankAction, BankItem};
use api::bank_memory::{BankMemory, Origin};
use api::named_banks::{BankPreferences, NamedBank};
use api::quest_progress::EvidenceStamp;
use api::selected::RunKey;
use api::snapshot::{
    GameSnapshot, ItemActionFamily, ItemContainer, ItemView, LocLayer, LocView, WorldTile,
};
use std::sync::Arc;
use std::task::Poll;

const ITEM: i32 = 42;
const PRODUCT: i32 = 43;
const OTHER: i32 = 44;
const BANK_COMPONENT: i32 = 10;
const STAND: WorldTile = WorldTile {
    x: 3210,
    z: 3210,
    level: 0,
};

type TestLedger = Option<Box<Ledger>>;

fn stamp(tick: u64) -> EvidenceStamp {
    EvidenceStamp {
        run: RunKey {
            slot: 1,
            run: 1,
            session: 1,
        },
        tick,
        sequence: tick,
    }
}

fn item(id: i32, name: &str, count: i32, container: ItemContainer, actions: &[&str]) -> ItemView {
    let component = matches!(container, ItemContainer::Bank | ItemContainer::BankSide);
    ItemView {
        def: crate::quester::families::tests::def(id, name),
        container,
        action_family: if component {
            ItemActionFamily::Component
        } else {
            ItemActionFamily::Held
        },
        slot: 0,
        count,
        actions: actions
            .iter()
            .map(|action| Some((*action).to_owned()))
            .collect(),
        component_id: if component { 7 } else { 3214 },
    }
}

fn bank_row(id: i32, name: &str, count: i32) -> ItemView {
    item(
        id,
        name,
        count,
        ItemContainer::Bank,
        &["Withdraw-X", "Withdraw-1"],
    )
}

fn side_row(id: i32, name: &str, count: i32) -> ItemView {
    item(
        id,
        name,
        count,
        ItemContainer::BankSide,
        &["Deposit-1", "Deposit-5", "Deposit-10", "Deposit-All"],
    )
}

fn fixture() -> (
    GameSnapshot,
    NamedBank,
    Arc<api::named_banks::NamedBankFacts>,
) {
    let bank = NamedBank::new("Test bank", STAND);
    let banks = Arc::new(api::named_banks::NamedBankFacts::from_banks(vec![bank]));
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_local_player(crate::quester::families::tests::local_player(STAND));
    snapshot.seed_locs(vec![LocView {
        id: 2213,
        name: Some("Bank booth".into()),
        actions: vec![Some("Use-quickly".into())],
        tile: STAND,
        distance: 0,
        typecode: 0,
        info: 0,
        description: None,
        layer: LocLayer::GroundDecoration,
        shape: 0,
        angle: 0,
        width: 1,
        length: 1,
        footprint_width: 1,
        footprint_length: 1,
        block_walk: false,
        block_range: false,
        active: true,
        animation: -1,
        map_function: -1,
        map_scene: -1,
        force_approach: 0,
    }]);
    snapshot.seed_inventory(Vec::new(), 28);
    snapshot.seed_equipment(Vec::new());
    (snapshot, bank, banks)
}

fn open_bank(snapshot: &mut GameSnapshot, rows: Option<Vec<ItemView>>, side: Vec<ItemView>) {
    snapshot.seed_bank_observation(BANK_COMPONENT, 1, rows, side);
}

fn request(
    stage: BankStage,
    needs: Vec<Need>,
    deposit: Option<DepositSpec>,
    bank: Option<NamedBank>,
    required: bool,
    partial_ok: bool,
    close: CloseBank,
) -> AcquireRequest {
    AcquireRequest {
        needs: needs.into(),
        deposit,
        bank: BankChoice {
            bank,
            explicit: None,
            required,
            preferences: BankPreferences::default(),
        },
        stage,
        partial_ok,
        close,
        vendor: VendorChoice::None,
        coins: CoinPolicy::None,
        walk: WalkOptions::default(),
        required_after: stamp(0),
    }
}

fn authored_request(
    bank: Option<NamedBank>,
    required: bool,
    actions: Vec<BankAction>,
) -> AcquireRequest {
    request(
        BankStage::Authored(actions.into()),
        Vec::new(),
        None,
        bank,
        required,
        true,
        CloseBank::LeaveOpen,
    )
}

fn need(
    id: i32,
    name: &str,
    target: i32,
    minimum: i32,
    presence: Presence,
    kind: NeedKind,
    sources: SourceSet,
) -> Need {
    Need {
        item: id,
        name: Arc::from(name),
        target,
        minimum,
        presence,
        kind,
        sources,
        alternatives: Arc::from([]),
    }
}

fn new_run(
    request: AcquireRequest,
    snapshot: &GameSnapshot,
    memory: Option<&BankMemory>,
    ledger: &mut TestLedger,
    banks: &Arc<api::named_banks::NamedBankFacts>,
) -> AcquireRun {
    crate::quester::families::tests::with_tick_bank(snapshot, memory, ledger, 1, |tick| {
        let plan = match plan::plan(&request, &tick.cx.snapshot().stock(), WorldFacts::default())
            .expect("fixture acquisition request plans")
        {
            AcquireVerdict::Plan(plan) => plan,
            AcquireVerdict::Satisfied => panic!("fixture request must produce a run plan"),
            AcquireVerdict::Blocked(block) => {
                panic!("fixture request unexpectedly blocked: {}", block.message)
            }
        };
        AcquireRun::new(request, plan, banks, &tick.cx)
    })
}

fn poll_at(
    run: &mut AcquireRun,
    snapshot: &GameSnapshot,
    memory: Option<&BankMemory>,
    ledger: &mut TestLedger,
    tick_number: u64,
    gate: OpenGate,
) -> Poll<Result<AcquireOutcome, ActionError>> {
    crate::quester::families::tests::with_tick_scene_bank(
        snapshot,
        memory,
        ledger,
        tick_number,
        |tick| run.poll(tick, gate),
    )
}

fn first_effect(ledger: &TestLedger) -> &HostEffect {
    &ledger
        .as_ref()
        .expect("native action created ledger")
        .outbox[0]
        .effect
}

fn complete_pick(ledger: &mut TestLedger, bank: NamedBank, tick: u64) {
    let ledger = ledger.as_mut().expect("bank selection queued a pick");
    let action = ledger.outbox.remove(0);
    assert!(matches!(&action.effect, HostEffect::BankPick(_)));
    let authority = action.authority();
    ledger.complete_bank_pick(
        &authority,
        BankPickReceipt {
            request_id: authority.request_id().get(),
            evidence: EvidenceStamp {
                run: authority.run(),
                tick,
                sequence: tick,
            },
            selected: SelectedBank {
                bank_index: 0,
                access_tile: STAND,
                kind: PickKind::Reachable,
                access: Some(Arc::new(BankStandAccess {
                    bank,
                    stand_tile: STAND,
                    kind: AccessKind::Booth,
                    stand_op: 1,
                    name: None,
                    choose: None,
                })),
            },
        },
    );
}

fn complete_interaction(ledger: &mut TestLedger, tick: u64) -> HostEffect {
    let ledger = ledger.as_mut().expect("native interaction created ledger");
    let action = ledger.outbox.remove(0);
    assert!(matches!(&action.effect, HostEffect::Interaction(_)));
    let authority = action.authority();
    ledger.complete_interaction(
        &authority,
        InteractionReceipt {
            request_id: authority.request_id().get(),
            evidence: EvidenceStamp {
                run: authority.run(),
                tick,
                sequence: tick,
            },
            accepted: true,
            chat_since: 0,
        },
    );
    action.effect
}

fn done(result: Poll<Result<AcquireOutcome, ActionError>>) -> AcquireReceipt {
    match result {
        Poll::Ready(Ok(AcquireOutcome::Done(receipt))) => receipt,
        _ => panic!("acquisition did not finish successfully"),
    }
}

fn is_open_stand(effect: &HostEffect) -> bool {
    matches!(
        effect,
        HostEffect::Interaction(crate::interact::InteractReq::OpenStand { .. })
    )
}

fn is_close(effect: &HostEffect) -> bool {
    matches!(
        effect,
        HostEffect::Interaction(crate::interact::InteractReq::Close)
    )
}

#[test]
fn authored_empty_actions_open_required_bank_and_leave_it_open() {
    let (mut snapshot, bank, banks) = fixture();
    let mut ledger = None;
    let mut run = new_run(
        authored_request(Some(bank), true, Vec::new()),
        &snapshot,
        None,
        &mut ledger,
        &banks,
    );

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open).is_pending());
    assert!(matches!(first_effect(&ledger), HostEffect::BankPick(_)));
    complete_pick(&mut ledger, bank, 1);

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 2, OpenGate::Open).is_pending());
    assert!(is_open_stand(first_effect(&ledger)));
    assert!(is_open_stand(&complete_interaction(&mut ledger, 2)));
    open_bank(&mut snapshot, Some(Vec::new()), Vec::new());

    let receipt = done(poll_at(
        &mut run,
        &snapshot,
        None,
        &mut ledger,
        3,
        OpenGate::Open,
    ));
    assert!(receipt.complete);
    assert!(receipt.bank_open);
    assert!(receipt.last_bank.is_none());
    assert!(ledger
        .as_ref()
        .is_none_or(|ledger| ledger.outbox.is_empty()));
}

#[test]
fn quester_bank_access_retries_a_failed_walk_on_the_next_tick() {
    let (mut snapshot, bank, banks) = fixture();
    let start = WorldTile {
        x: STAND.x + 12,
        z: STAND.z,
        level: STAND.level,
    };
    snapshot.seed_local_player(crate::quester::families::tests::local_player(start));
    let mut ledger = None;
    let mut run = new_run(
        authored_request(Some(bank), true, Vec::new()),
        &snapshot,
        None,
        &mut ledger,
        &banks,
    )
    .with_quester_walk_retries();

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open).is_pending());
    assert!(matches!(first_effect(&ledger), HostEffect::BankPick(_)));
    complete_pick(&mut ledger, bank, 1);

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 2, OpenGate::Open).is_pending());
    assert!(matches!(
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        HostEffect::Walk(_)
    ));
    let first_request_id = ledger
        .as_ref()
        .unwrap()
        .outbox
        .last()
        .unwrap()
        .request_id
        .get();
    assert_eq!(
        crate::quester::families::tests::post_walk_receipt(
            &mut ledger,
            3,
            WalkEnd::NoPath,
            Some("first bank route failed"),
        ),
        first_request_id
    );
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 3, OpenGate::Open).is_pending());
    let walk_count = |ledger: &TestLedger| {
        ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .filter(|action| matches!(&action.effect, HostEffect::Walk(_)))
            .count()
    };
    assert_eq!(walk_count(&ledger), 1);

    let moved = WorldTile {
        x: start.x - 1,
        ..start
    };
    snapshot.seed_local_player(crate::quester::families::tests::local_player(moved));
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 4, OpenGate::Open).is_pending());
    assert_ne!(
        ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .rev()
            .find(|action| matches!(&action.effect, HostEffect::Walk(_)))
            .unwrap()
            .request_id
            .get(),
        first_request_id
    );
    let retry = ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .rev()
        .find(|action| matches!(&action.effect, HostEffect::Walk(_)))
        .unwrap();
    let HostEffect::Walk(request) = &retry.effect else {
        unreachable!();
    };
    assert_eq!(request.target, STAND);
    assert_eq!(request.required_after.tick, 4);

    snapshot.seed_local_player(crate::quester::families::tests::local_player(STAND));
    crate::quester::families::tests::post_walk_receipt(&mut ledger, 5, WalkEnd::Arrived, None);
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 5, OpenGate::Open).is_pending());
    assert!(matches!(
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        HostEffect::Interaction(crate::interact::InteractReq::OpenStand { .. })
    ));
}

#[test]
fn hold_blocks_initial_open_and_does_not_advance_active_open() {
    let (mut snapshot, bank, banks) = fixture();
    let mut ledger = None;
    let mut run = new_run(
        authored_request(Some(bank), true, Vec::new()),
        &snapshot,
        None,
        &mut ledger,
        &banks,
    );

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Hold).is_pending());
    assert!(matches!(first_effect(&ledger), HostEffect::BankPick(_)));
    complete_pick(&mut ledger, bank, 1);

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 2, OpenGate::Hold).is_pending());
    assert!(
        ledger.as_ref().unwrap().outbox.is_empty(),
        "Hold must not dispatch OpenStand"
    );
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 3, OpenGate::Open).is_pending());
    assert!(is_open_stand(first_effect(&ledger)));
    complete_interaction(&mut ledger, 3);
    open_bank(&mut snapshot, Some(Vec::new()), Vec::new());

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 4, OpenGate::Hold).is_pending());
    assert!(
        ledger.as_ref().unwrap().outbox.is_empty(),
        "Hold must leave the active Open unpolled"
    );
    let receipt = done(poll_at(
        &mut run,
        &snapshot,
        None,
        &mut ledger,
        5,
        OpenGate::Open,
    ));
    assert!(receipt.bank_open);
}

#[test]
fn authored_partial_withdrawal_transfers_observed_available_count() {
    let (mut snapshot, bank, banks) = fixture();
    open_bank(
        &mut snapshot,
        Some(vec![bank_row(ITEM, "Widget", 2)]),
        Vec::new(),
    );
    let memory = BankMemory::seeded(&[(ITEM, 2)], Origin::Session);
    let action = BankAction::Withdraw {
        item: BankItem {
            id: ITEM,
            name: Arc::from("Widget"),
        },
        qty: 5,
    };
    let strict = authored_request(Some(bank), false, vec![action.clone()]);
    let strict = request(
        strict.stage,
        Vec::new(),
        None,
        Some(bank),
        false,
        false,
        CloseBank::LeaveOpen,
    );
    let mut ledger = None;
    let strict_verdict = crate::quester::families::tests::with_tick_bank(
        &snapshot,
        Some(&memory),
        &mut ledger,
        1,
        |tick| plan::plan(&strict, &tick.cx.snapshot().stock(), WorldFacts::default()).unwrap(),
    );
    assert!(matches!(
        strict_verdict,
        AcquireVerdict::Blocked(AcquireBlock {
            reason: BlockReason::AuthoredShortage,
            ..
        })
    ));

    let partial = request(
        BankStage::Authored(vec![action].into()),
        Vec::new(),
        None,
        Some(bank),
        false,
        true,
        CloseBank::LeaveOpen,
    );
    let mut run = new_run(partial, &snapshot, Some(&memory), &mut ledger, &banks);
    assert!(poll_at(
        &mut run,
        &snapshot,
        Some(&memory),
        &mut ledger,
        1,
        OpenGate::Open
    )
    .is_pending());
    assert!(matches!(
        first_effect(&ledger),
        HostEffect::Interaction(crate::interact::InteractReq::SetNoteMode { on: false, .. })
    ));
    complete_interaction(&mut ledger, 1);
    assert!(poll_at(
        &mut run,
        &snapshot,
        Some(&memory),
        &mut ledger,
        2,
        OpenGate::Open
    )
    .is_pending());
    assert!(matches!(
        first_effect(&ledger),
        HostEffect::Interaction(crate::interact::InteractReq::WithdrawX {
            bank_item_id: ITEM,
            ..
        })
    ));
    assert!(matches!(
        complete_interaction(&mut ledger, 2),
        HostEffect::Interaction(crate::interact::InteractReq::WithdrawX {
            bank_item_id: ITEM,
            ..
        })
    ));

    snapshot.seed_inventory(
        vec![item(ITEM, "Widget", 2, ItemContainer::Inventory, &["Use"])],
        28,
    );
    open_bank(
        &mut snapshot,
        Some(vec![bank_row(ITEM, "Widget", 0)]),
        Vec::new(),
    );
    let receipt = done(poll_at(
        &mut run,
        &snapshot,
        Some(&memory),
        &mut ledger,
        3,
        OpenGate::Open,
    ));
    assert!(
        !receipt.complete,
        "partial fulfillment must remain incomplete"
    );
    assert!(receipt.withdrew_any);
    assert_eq!(
        receipt.last_bank.as_ref().map(|receipt| receipt.complete),
        Some(false)
    );
    assert_eq!(receipt.deposited, 0);
    assert!(receipt.bank_open);
}

#[test]
fn generated_deposit_precedes_live_replan_and_receipts_exact_count() {
    let (mut snapshot, bank, banks) = fixture();
    snapshot.seed_inventory(
        vec![item(
            PRODUCT,
            "Product",
            2,
            ItemContainer::Inventory,
            &["Use"],
        )],
        28,
    );
    open_bank(
        &mut snapshot,
        Some(vec![bank_row(ITEM, "Widget", 2)]),
        vec![side_row(PRODUCT, "Product", 2)],
    );
    let need = need(
        ITEM,
        "Widget",
        2,
        2,
        Presence::Held,
        NeedKind::Required,
        SourceSet::PACK | SourceSet::BANK,
    );
    let request = request(
        BankStage::Planned,
        vec![need],
        Some(DepositSpec::Ids {
            products: Arc::from([PRODUCT]),
            keep: Arc::from([]),
        }),
        Some(bank),
        false,
        false,
        CloseBank::LeaveOpen,
    );
    let mut ledger = None;
    let mut run = new_run(request, &snapshot, None, &mut ledger, &banks);

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open).is_pending());
    assert!(matches!(
        first_effect(&ledger),
        HostEffect::Interaction(crate::interact::InteractReq::InvButton { id: PRODUCT, .. })
    ));
    assert!(matches!(
        complete_interaction(&mut ledger, 1),
        HostEffect::Interaction(crate::interact::InteractReq::InvButton { id: PRODUCT, .. })
    ));
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 2, OpenGate::Open).is_pending());
    assert!(
        ledger.as_ref().unwrap().outbox.is_empty(),
        "re-plan must wait for observed deposit effects"
    );

    snapshot.seed_inventory(Vec::new(), 28);
    open_bank(
        &mut snapshot,
        Some(vec![
            bank_row(ITEM, "Widget", 2),
            bank_row(PRODUCT, "Product", 2),
        ]),
        Vec::new(),
    );
    let mut withdrawal_tick = None;
    for tick in 3..=6 {
        assert!(poll_at(&mut run, &snapshot, None, &mut ledger, tick, OpenGate::Open).is_pending());
        if ledger
            .as_ref()
            .is_some_and(|ledger| !ledger.outbox.is_empty())
        {
            match first_effect(&ledger) {
                HostEffect::Interaction(crate::interact::InteractReq::SetNoteMode {
                    on: false,
                    ..
                }) => {
                    complete_interaction(&mut ledger, tick);
                }
                HostEffect::Interaction(crate::interact::InteractReq::WithdrawX {
                    bank_item_id: ITEM,
                    ..
                }) => {
                    assert!(matches!(
                        complete_interaction(&mut ledger, tick),
                        HostEffect::Interaction(crate::interact::InteractReq::WithdrawX {
                            bank_item_id: ITEM,
                            ..
                        })
                    ));
                    withdrawal_tick = Some(tick);
                    break;
                }
                _ => panic!("live re-plan dispatched an unexpected effect"),
            }
        }
    }
    let withdrawal_tick =
        withdrawal_tick.expect("live re-plan must withdraw the newly banked need");
    snapshot.seed_inventory(
        vec![item(ITEM, "Widget", 2, ItemContainer::Inventory, &["Use"])],
        28,
    );
    open_bank(
        &mut snapshot,
        Some(vec![bank_row(PRODUCT, "Product", 2)]),
        Vec::new(),
    );
    let receipt = done(poll_at(
        &mut run,
        &snapshot,
        None,
        &mut ledger,
        withdrawal_tick + 1,
        OpenGate::Open,
    ));
    assert!(receipt.complete);
    assert!(receipt.withdrew_any);
    assert_eq!(receipt.deposited, 2);
    assert!(receipt.bank_open);
}

#[test]
fn quester_interrupts_acquire_run_at_transfer_boundary() {
    let (mut snapshot, bank, banks) = fixture();
    snapshot.seed_inventory(
        vec![item(
            PRODUCT,
            "Product",
            2,
            ItemContainer::Inventory,
            &["Use"],
        )],
        28,
    );
    open_bank(
        &mut snapshot,
        Some(vec![bank_row(ITEM, "Widget", 2)]),
        vec![side_row(PRODUCT, "Product", 2)],
    );
    let need = need(
        ITEM,
        "Widget",
        2,
        2,
        Presence::Held,
        NeedKind::Required,
        SourceSet::PACK | SourceSet::BANK,
    );
    let request = request(
        BankStage::Planned,
        vec![need],
        Some(DepositSpec::Ids {
            products: Arc::from([PRODUCT]),
            keep: Arc::from([]),
        }),
        Some(bank),
        false,
        false,
        CloseBank::LeaveOpen,
    );
    let mut ledger = None;
    let mut run = new_run(request, &snapshot, None, &mut ledger, &banks);
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open).is_pending());
    assert!(matches!(
        complete_interaction(&mut ledger, 1),
        HostEffect::Interaction(crate::interact::InteractReq::InvButton { id: PRODUCT, .. })
    ));
    assert!(run.transfer_in_flight());
    crate::quester::families::tests::with_tick_bank(&snapshot, None, &mut ledger, 2, |tick| {
        run.interrupt_transfer(tick.actions);
    });
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 3, OpenGate::Open).is_pending());
    assert!(ledger.as_ref().unwrap().outbox.is_empty());

    snapshot.seed_inventory(Vec::new(), 28);
    open_bank(
        &mut snapshot,
        Some(vec![
            bank_row(ITEM, "Widget", 2),
            bank_row(PRODUCT, "Product", 2),
        ]),
        Vec::new(),
    );
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 4, OpenGate::Open).is_pending());
    assert!(!run.transfer_in_flight());
    assert!(matches!(run.active, Active::None));
    assert!(
        ledger.as_ref().unwrap().outbox.is_empty(),
        "the boundary poll must not begin a later bank action"
    );

    let (mut snapshot, bank, banks) = fixture();
    open_bank(
        &mut snapshot,
        Some(vec![
            bank_row(ITEM, "Widget", 3),
            bank_row(OTHER, "Other", 1),
        ]),
        Vec::new(),
    );
    let request = authored_request(
        Some(bank),
        false,
        vec![BankAction::WithdrawTo {
            withdrawals: Arc::from([
                crate::native_bank::Withdrawal {
                    id: ITEM,
                    name: Arc::from("Widget"),
                    target: 2,
                },
                crate::native_bank::Withdrawal {
                    id: OTHER,
                    name: Arc::from("Other"),
                    target: 1,
                },
            ]),
        }],
    );
    let mut ledger = None;
    let mut run = new_run(request, &snapshot, None, &mut ledger, &banks);
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open).is_pending());
    assert!(matches!(
        complete_interaction(&mut ledger, 1),
        HostEffect::Interaction(crate::interact::InteractReq::SetNoteMode { on: false, .. })
    ));
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open).is_pending());
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 2, OpenGate::Open).is_pending());
    assert!(matches!(
        complete_interaction(&mut ledger, 2),
        HostEffect::Interaction(crate::interact::InteractReq::WithdrawX {
            bank_item_id: ITEM,
            ..
        })
    ));
    assert!(run.transfer_in_flight());
    crate::quester::families::tests::with_tick_bank(&snapshot, None, &mut ledger, 3, |tick| {
        run.interrupt_transfer(tick.actions);
    });
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 4, OpenGate::Open).is_pending());
    snapshot.seed_inventory(
        vec![item(ITEM, "Widget", 2, ItemContainer::Inventory, &["Use"])],
        28,
    );
    open_bank(
        &mut snapshot,
        Some(vec![
            bank_row(ITEM, "Widget", 1),
            bank_row(OTHER, "Other", 1),
        ]),
        Vec::new(),
    );
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 5, OpenGate::Open).is_pending());
    assert!(!run.transfer_in_flight());
    assert!(matches!(run.active, Active::None));
    assert!(
        ledger.as_ref().unwrap().outbox.is_empty(),
        "the boundary poll must not withdraw the second requested item"
    );
}

#[test]
fn acquire_run_active_match_unchanged() {
    let (mut snapshot, bank, banks) = fixture();
    let mut net = item(
        ITEM,
        "Net",
        1,
        ItemContainer::Inventory,
        &["Deposit-1", "Deposit-All"],
    );
    net.slot = 0;
    let mut shrimp = item(
        PRODUCT,
        "Shrimp",
        1,
        ItemContainer::Inventory,
        &["Deposit-1", "Deposit-All"],
    );
    shrimp.slot = 1;
    let mut anchovy = item(
        OTHER,
        "Anchovy",
        1,
        ItemContainer::Inventory,
        &["Deposit-1", "Deposit-All"],
    );
    anchovy.slot = 2;
    snapshot.seed_inventory(vec![net, shrimp, anchovy], 28);

    let mut shrimp_side = side_row(PRODUCT, "Shrimp", 1);
    shrimp_side.slot = 1;
    let mut anchovy_side = side_row(OTHER, "Anchovy", 1);
    anchovy_side.slot = 2;
    open_bank(
        &mut snapshot,
        Some(Vec::new()),
        vec![shrimp_side, anchovy_side],
    );

    let request = request(
        BankStage::Planned,
        Vec::new(),
        Some(DepositSpec::Capacity {
            keep: Arc::from([ITEM]),
            slots: 1,
        }),
        Some(bank),
        false,
        false,
        CloseBank::LeaveOpen,
    );
    let mut ledger = None;
    let mut run = new_run(request, &snapshot, None, &mut ledger, &banks);
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open).is_pending());
    assert!(
        run.transfer_in_flight(),
        "capacity deposits use Active::Machine"
    );
    assert!(matches!(
        complete_interaction(&mut ledger, 1),
        HostEffect::Interaction(crate::interact::InteractReq::InvButton { id: PRODUCT, .. })
    ));

    crate::quester::families::tests::with_tick_bank(&snapshot, None, &mut ledger, 2, |tick| {
        run.interrupt_transfer(tick.actions)
    });
    assert!(run.transfer_in_flight());

    let mut net = item(
        ITEM,
        "Net",
        1,
        ItemContainer::Inventory,
        &["Deposit-1", "Deposit-All"],
    );
    net.slot = 0;
    let mut anchovy = item(
        OTHER,
        "Anchovy",
        1,
        ItemContainer::Inventory,
        &["Deposit-1", "Deposit-All"],
    );
    anchovy.slot = 2;
    snapshot.seed_inventory(vec![net, anchovy], 28);
    let mut anchovy_side = side_row(OTHER, "Anchovy", 1);
    anchovy_side.slot = 2;
    open_bank(
        &mut snapshot,
        Some(vec![bank_row(PRODUCT, "Shrimp", 1)]),
        vec![anchovy_side],
    );
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 3, OpenGate::Open).is_pending());
    assert!(!run.transfer_in_flight());
    assert!(matches!(run.active, Active::None));
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
}

#[test]
fn wear_only_run_waits_for_worn_evidence_without_walk_or_bank_effects() {
    let (mut snapshot, _bank, banks) = fixture();
    snapshot.seed_inventory(
        vec![item(
            ITEM,
            "Widget",
            1,
            ItemContainer::Inventory,
            &["Wield"],
        )],
        28,
    );
    let request = request(
        BankStage::Planned,
        vec![need(
            ITEM,
            "Widget",
            1,
            1,
            Presence::Wielded,
            NeedKind::Required,
            SourceSet::PACK,
        )],
        None,
        None,
        false,
        false,
        CloseBank::LeaveOpen,
    );
    let mut ledger = None;
    let mut run = new_run(request, &snapshot, None, &mut ledger, &banks);

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open).is_pending());
    assert!(
        matches!(first_effect(&ledger), HostEffect::Interaction(crate::interact::InteractReq::Held { action, .. }) if action.as_str() == "Wield")
    );
    assert!(!ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .any(|action| matches!(
            &action.effect,
            HostEffect::BankPick(_) | HostEffect::Walk(_)
        )));
    assert!(matches!(
        complete_interaction(&mut ledger, 1),
        HostEffect::Interaction(crate::interact::InteractReq::Held { .. })
    ));

    assert!(
        poll_at(&mut run, &snapshot, None, &mut ledger, 2, OpenGate::Open).is_pending(),
        "accepted click is not worn evidence"
    );
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
    snapshot.seed_inventory(Vec::new(), 28);
    snapshot.seed_equipment(vec![item(ITEM, "Widget", 1, ItemContainer::Equipment, &[])]);
    let receipt = done(poll_at(
        &mut run,
        &snapshot,
        None,
        &mut ledger,
        3,
        OpenGate::Open,
    ));
    assert!(receipt.complete);
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
}

#[test]
fn child_due_reports_need_index_then_resume_replans_from_new_inventory() {
    let (mut snapshot, _bank, banks) = fixture();
    snapshot.seed_inventory(
        vec![item(ITEM, "Widget", 1, ItemContainer::Inventory, &["Use"])],
        28,
    );
    let request = request(
        BankStage::Planned,
        vec![
            need(
                ITEM,
                "Widget",
                1,
                1,
                Presence::Held,
                NeedKind::Required,
                SourceSet::PACK,
            ),
            need(
                OTHER,
                "Component",
                1,
                1,
                Presence::Held,
                NeedKind::Required,
                SourceSet::PACK | SourceSet::CHILD,
            ),
        ],
        None,
        None,
        false,
        false,
        CloseBank::LeaveOpen,
    );
    let mut ledger = None;
    let mut run = new_run(request, &snapshot, None, &mut ledger, &banks);

    assert!(matches!(
        poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open),
        Poll::Ready(Ok(AcquireOutcome::ChildDue { need: 1 }))
    ));
    assert!(ledger
        .as_ref()
        .is_none_or(|ledger| ledger.outbox.is_empty()));
    assert!(matches!(
        poll_at(&mut run, &snapshot, None, &mut ledger, 2, OpenGate::Open),
        Poll::Ready(Ok(AcquireOutcome::ChildDue { need: 1 }))
    ));

    run.resume_after_child();
    snapshot.seed_inventory(
        vec![
            item(ITEM, "Widget", 1, ItemContainer::Inventory, &["Use"]),
            item(OTHER, "Component", 1, ItemContainer::Inventory, &["Use"]),
        ],
        28,
    );
    let receipt = done(poll_at(
        &mut run,
        &snapshot,
        None,
        &mut ledger,
        3,
        OpenGate::Open,
    ));
    assert!(receipt.complete);
    assert!(ledger
        .as_ref()
        .is_none_or(|ledger| ledger.outbox.is_empty()));
}

#[test]
fn explicit_cancel_revokes_queued_effect_and_is_terminal() {
    let (snapshot, bank, banks) = fixture();
    let mut ledger = None;
    let mut run = new_run(
        authored_request(Some(bank), true, Vec::new()),
        &snapshot,
        None,
        &mut ledger,
        &banks,
    );

    crate::quester::families::tests::with_tick_bank(&snapshot, None, &mut ledger, 1, |tick| {
        assert!(run.poll(tick, OpenGate::Open).is_pending());
    });
    assert!(matches!(first_effect(&ledger), HostEffect::BankPick(_)));
    assert!(ledger.as_ref().unwrap().outbox[0].live());
    crate::quester::families::tests::with_tick_bank(&snapshot, None, &mut ledger, 2, |tick| {
        run.cancel(tick.actions)
    });
    assert!(
        !ledger.as_ref().unwrap().outbox[0].live(),
        "cancel must revoke the queued BankPick before host dispatch"
    );
    assert!(matches!(
        poll_at(&mut run, &snapshot, None, &mut ledger, 3, OpenGate::Open),
        Poll::Ready(Err(ActionError::Cancelled))
    ));
    assert!(!ledger.as_ref().unwrap().outbox[0].live());
}

#[test]
fn stale_run_key_cancels_and_prevents_later_dispatch() {
    let (snapshot, bank, banks) = fixture();
    let mut ledger = None;
    let mut run = new_run(
        authored_request(Some(bank), true, Vec::new()),
        &snapshot,
        None,
        &mut ledger,
        &banks,
    );
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open).is_pending());
    assert!(ledger.as_ref().unwrap().outbox[0].live());

    let stale =
        crate::quester::families::tests::with_tick_bank(&snapshot, None, &mut ledger, 2, |tick| {
            tick.cx.evidence.run.run += 1;
            run.poll(tick, OpenGate::Open)
        });
    assert!(matches!(stale, Poll::Ready(Err(ActionError::Stale))));
    assert!(!ledger.as_ref().unwrap().outbox[0].live());
    assert!(matches!(
        poll_at(&mut run, &snapshot, None, &mut ledger, 3, OpenGate::Open),
        Poll::Ready(Err(ActionError::Cancelled))
    ));
}

#[test]
fn unchanged_pending_poll_uses_zero_allocations_after_warmup() {
    let (snapshot, bank, banks) = fixture();
    let mut ledger = None;
    let mut run = new_run(
        authored_request(Some(bank), true, Vec::new()),
        &snapshot,
        None,
        &mut ledger,
        &banks,
    );

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open).is_pending());
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 2, OpenGate::Open).is_pending());
    let allocations = allocation_counter::measure(|| {
        for tick in 3..=1002 {
            assert!(
                poll_at(&mut run, &snapshot, None, &mut ledger, tick, OpenGate::Open).is_pending()
            );
        }
    });
    assert_eq!(
        allocations,
        allocation_counter::AllocationInfo::default(),
        "unchanged pending polls must allocate nothing after warmup"
    );
    assert!(matches!(first_effect(&ledger), HostEffect::BankPick(_)));
}

#[test]
fn unchanged_blocked_polls_retain_one_block_without_allocations() {
    let (mut snapshot, bank, banks) = fixture();
    open_bank(&mut snapshot, Some(Vec::new()), Vec::new());
    let mut ledger = None;
    let request = request(
        BankStage::Planned,
        vec![need(
            ITEM,
            "Widget",
            1,
            1,
            Presence::Held,
            NeedKind::Required,
            SourceSet::PACK | SourceSet::BANK,
        )],
        None,
        Some(bank),
        false,
        false,
        CloseBank::LeaveOpen,
    );
    let mut run = new_run(request, &snapshot, None, &mut ledger, &banks);
    let message = match poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open) {
        Poll::Ready(Err(ActionError::Blocked(message))) => message,
        _ => panic!("the observed empty bank must block the required need"),
    };
    let block = std::ptr::from_ref(run.block.as_deref().expect("one retained block"));
    let allocations = allocation_counter::measure(|| {
        for tick in 2..=1001 {
            match poll_at(&mut run, &snapshot, None, &mut ledger, tick, OpenGate::Open) {
                Poll::Ready(Err(ActionError::Blocked(current))) => {
                    assert!(Arc::ptr_eq(&current, &message));
                }
                _ => panic!("the unchanged block must remain terminal"),
            }
            assert_eq!(run.block.as_deref().map(std::ptr::from_ref), Some(block));
        }
    });
    assert_eq!(allocations, allocation_counter::AllocationInfo::default());
    assert!(ledger
        .as_ref()
        .is_none_or(|ledger| ledger.outbox.is_empty()));
}

#[test]
fn open_attempt_cap_uses_observed_tick_deadlines_and_real_interactions() {
    let (snapshot, bank, banks) = fixture();
    let mut ledger = None;
    let mut run = new_run(
        request(
            BankStage::Planned,
            vec![need(
                ITEM,
                "Tool",
                1,
                1,
                Presence::Held,
                NeedKind::Required,
                SourceSet::PACK | SourceSet::BANK,
            )],
            None,
            Some(bank),
            true,
            false,
            CloseBank::LeaveOpen,
        ),
        &snapshot,
        None,
        &mut ledger,
        &banks,
    );
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open).is_pending());
    complete_pick(&mut ledger, bank, 1);

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 2, OpenGate::Open).is_pending());
    assert!(is_open_stand(first_effect(&ledger)));
    complete_interaction(&mut ledger, 2);

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 10, OpenGate::Open).is_pending());
    assert!(is_open_stand(first_effect(&ledger)));
    complete_interaction(&mut ledger, 10);

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 18, OpenGate::Open).is_pending());
    assert!(is_open_stand(first_effect(&ledger)));
    complete_interaction(&mut ledger, 18);

    assert!(matches!(
        poll_at(&mut run, &snapshot, None, &mut ledger, 26, OpenGate::Open),
        Poll::Ready(Err(ActionError::Failed(message)))
            if message.as_ref() == "bank-unavailable: bank did not open after 3 attempts"
    ));
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
}

#[test]
fn close_attempt_cap_uses_observed_tick_deadlines_and_real_interactions() {
    let (mut snapshot, bank, banks) = fixture();
    open_bank(&mut snapshot, Some(Vec::new()), Vec::new());
    let mut request = authored_request(Some(bank), false, Vec::new());
    request.close = CloseBank::Close;
    let mut ledger = None;
    let mut run = new_run(request, &snapshot, None, &mut ledger, &banks);

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open).is_pending());
    assert!(is_close(first_effect(&ledger)));
    complete_interaction(&mut ledger, 1);

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 9, OpenGate::Open).is_pending());
    assert!(is_close(first_effect(&ledger)));
    complete_interaction(&mut ledger, 9);

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 17, OpenGate::Open).is_pending());
    assert!(is_close(first_effect(&ledger)));
    complete_interaction(&mut ledger, 17);

    assert!(matches!(
        poll_at(&mut run, &snapshot, None, &mut ledger, 25, OpenGate::Open),
        Poll::Ready(Err(ActionError::Failed(message)))
            if message.as_ref() == "bank-close-failed: bank did not close after 3 attempts"
    ));
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
    assert!(crate::quester::families::tests::with_tick_bank(
        &snapshot,
        None,
        &mut ledger,
        26,
        |tick| bank_open(&tick.cx),
    ));
}

#[test]
fn bank_machine_and_acquire_run_size_of_pinned() {
    use std::mem::size_of;
    println!(
        "BankMachine={} AcquireRun={} AcquireRequest={} BankReceipt={} Provisioner={} Gatherer={} Quester={} SurvivalStep={}",
        size_of::<crate::native_bank::BankMachine>(),
        size_of::<AcquireRun>(),
        size_of::<AcquireRequest>(),
        size_of::<BankReceipt>(),
        size_of::<crate::quester::provision::Provisioner>(),
        size_of::<crate::gatherer::Gatherer>(),
        size_of::<crate::quester::runner::Quester>(),
        size_of::<crate::survival::SurvivalStep>(),
    );
    assert!(size_of::<crate::native_bank::BankMachine>() <= 768);
    assert!(size_of::<AcquireRun>() <= 768);
    assert_eq!(size_of::<BankReceipt>(), 8);
    assert!(size_of::<crate::survival::SurvivalStep>() <= 192);
}

#[test]
fn bank_walk_prerequisite_handoff_reissues_once_and_cancel_keeps_run_state() {
    let (mut snapshot, bank, banks) = fixture();
    let data =
        api::game_data::for_revision(api::selected::ClientRevision::R289).expect("R289 data");
    let catalog = api::selected::FamilyPreparation::run(move |worker| {
        data.prepare_prereq_steps(worker)
            .expect("R289 prerequisite catalog")
    })
    .expect("family preparation worker")
    .join()
    .expect("prerequisite preparation");
    let (_, row) = catalog
        .iter()
        .find(|(_, row)| row.crossings.iter().any(|entry| entry.crossing.option > 0))
        .expect("R289 has a prerequisite crossing");
    let entry = row
        .crossings
        .iter()
        .find(|entry| entry.crossing.option > 0)
        .expect("selected row has a supported operation");
    let crossing = entry.crossing;
    let gate = api::prereq_steps::ActionableGate {
        // The dialogue fixture needs its actual barrier promise, not any journal interval.
        gate: api::selected::QuestGate::Window(row.establishes.clone()),
        truth: api::selected::Truth::Unknown,
        leg: 0,
        crossing,
    };
    let mut player = snapshot
        .local_player()
        .expect("fixture has a local player")
        .clone();
    player.player.network = crossing.takeoff;
    player.player.actor.tile = crossing.takeoff;
    player.player.actor.moving = false;
    snapshot.seed_local_player(player);
    crate::quester::families::tests::seed_prereq_crossing(&mut snapshot, row, &crossing);
    let target = WorldTile {
        x: crossing.at.x + 80,
        z: crossing.at.z + 80,
        level: crossing.at.level,
    };

    let mut ledger = None;
    let mut run = new_run(
        authored_request(Some(bank), true, Vec::new()),
        &snapshot,
        None,
        &mut ledger,
        &banks,
    );
    run.phase = Phase::Walk;
    run.target = Some(target);
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open).is_pending());
    let (outer_id, outer_run) = {
        let action = &ledger.as_ref().unwrap().outbox[0];
        (action.request_id.get(), action.run())
    };
    ledger.as_mut().unwrap().walk = Some(crate::native::WalkReceipt {
        goal: None,
        request_id: outer_id,
        evidence: api::quest_progress::EvidenceStamp {
            run: outer_run,
            tick: 2,
            sequence: 2,
        },
        end: crate::native::WalkEnd::NeedsStep(Arc::from([gate])),
        blocked: None,
        detail: None,
        refusal: None,
        assessment: None,
        escape: None,
        prereq_allow: Some(crate::native::PrereqAllow {
            dialogue: true,
            quest_start: true,
        }),
    });
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 2, OpenGate::Open).is_pending());
    let (probe_authority, probe_id, probe_run) = {
        let probe = ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .find(|action| {
                matches!(
                    &action.effect,
                    HostEffect::Interaction(crate::interact::InteractReq::Loc {
                        id: Some(id),
                        ..
                    }) if *id == crossing.loc_id
                )
            })
            .expect("prerequisite probe was submitted");
        (probe.authority(), probe.request_id.get(), probe.run())
    };
    ledger.as_mut().unwrap().complete_interaction(
        &probe_authority,
        crate::native::InteractionReceipt {
            request_id: probe_id,
            evidence: api::quest_progress::EvidenceStamp {
                run: probe_run,
                tick: 2,
                sequence: 2,
            },
            accepted: true,
            chat_since: 0,
        },
    );
    let mut player = snapshot
        .local_player()
        .expect("fixture has a local player")
        .clone();
    player.player.network = crossing.to;
    player.player.actor.tile = crossing.to;
    snapshot.seed_local_player(player);
    snapshot.seed_chat_lines(vec![api::snapshot::ChatLineView {
        sequence: 1,
        type_: 0,
        username: None,
        text: entry.pass.message.to_string(),
    }]);
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 3, OpenGate::Open).is_pending());

    let walk_ids: Vec<_> = ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .filter_map(|action| match &action.effect {
            HostEffect::Walk(request) => Some((action.request_id.get(), request.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(walk_ids.len(), 1, "only the live reissue remains queued");
    let (reissued_id, reissued) = &walk_ids[0];
    assert!(
        *reissued_id > outer_id,
        "the outer request was replaced once"
    );
    assert_eq!(reissued.target, target);
    assert_eq!(reissued.radius, 0);
    assert_eq!(reissued.required_after.tick, 3);
    assert!(reissued.cross.is_empty());
    assert!(reissued.avoid.is_empty());
    assert!(!reissued.protect);
    assert!(!reissued.food_guard);
    assert_eq!(run.target, Some(target));
    assert_eq!(run.run, outer_run);

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 4, OpenGate::Open).is_pending());
    assert_eq!(
        ledger.as_ref().unwrap().outbox.len(),
        1,
        "pending wrapper polls keep only their one live reissue"
    );
    crate::quester::families::tests::with_tick_bank(&snapshot, None, &mut ledger, 5, |tick| {
        run.cancel(tick.actions)
    });
    assert!(!ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .find(|action| action.request_id.get() == *reissued_id)
        .expect("reissued walk remains recorded")
        .live());
    assert_eq!(
        run.target,
        Some(target),
        "cancel preserves the selected bank target"
    );
    assert!(matches!(
        poll_at(&mut run, &snapshot, None, &mut ledger, 6, OpenGate::Open),
        Poll::Ready(Err(ActionError::Cancelled))
    ));
    assert_eq!(run.target, Some(target));
}
