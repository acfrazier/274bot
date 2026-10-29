use super::*;
use crate::bank::{AccessKind, BankPickReceipt, BankStandAccess, PickKind, SelectedBank};
use crate::native::ledger::{HostEffect, Ledger};
use crate::native::{ActionError, InteractionReceipt};
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
    crate::quester::families::tests::with_tick_bank(snapshot, memory, ledger, tick_number, |tick| {
        run.poll(tick, gate)
    })
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
        HostEffect::Interaction(crate::shim::InteractReq::OpenStand { .. })
    )
}

fn is_close(effect: &HostEffect) -> bool {
    matches!(
        effect,
        HostEffect::Interaction(crate::shim::InteractReq::Close)
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
        HostEffect::Interaction(crate::shim::InteractReq::SetNoteMode { on: false, .. })
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
        HostEffect::Interaction(crate::shim::InteractReq::WithdrawX {
            bank_item_id: ITEM,
            ..
        })
    ));
    assert!(matches!(
        complete_interaction(&mut ledger, 2),
        HostEffect::Interaction(crate::shim::InteractReq::WithdrawX {
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
        HostEffect::Interaction(crate::shim::InteractReq::InvButton { id: PRODUCT, .. })
    ));
    assert!(matches!(
        complete_interaction(&mut ledger, 1),
        HostEffect::Interaction(crate::shim::InteractReq::InvButton { id: PRODUCT, .. })
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
                HostEffect::Interaction(crate::shim::InteractReq::SetNoteMode {
                    on: false,
                    ..
                }) => {
                    complete_interaction(&mut ledger, tick);
                }
                HostEffect::Interaction(crate::shim::InteractReq::WithdrawX {
                    bank_item_id: ITEM,
                    ..
                }) => {
                    assert!(matches!(
                        complete_interaction(&mut ledger, tick),
                        HostEffect::Interaction(crate::shim::InteractReq::WithdrawX {
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
fn live_replan_withdraws_an_alternative_by_its_selected_name_after_deposit() {
    const BRONZE_AXE: i32 = 1351;
    const RUNE_AXE: i32 = 1359;
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
        Some(vec![bank_row(BRONZE_AXE, "Bronze axe", 3)]),
        vec![side_row(PRODUCT, "Product", 2)],
    );
    let mut tool = need(
        RUNE_AXE,
        "Rune axe",
        1,
        1,
        Presence::Carried,
        NeedKind::Required,
        SourceSet::PACK | SourceSet::WORN | SourceSet::BANK,
    );
    tool.alternatives = Arc::from([RUNE_AXE, BRONZE_AXE]);
    let request = request(
        BankStage::Planned,
        vec![tool],
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
        HostEffect::Interaction(crate::shim::InteractReq::InvButton { id: PRODUCT, .. })
    ));
    snapshot.seed_inventory(Vec::new(), 28);
    open_bank(
        &mut snapshot,
        Some(vec![
            bank_row(BRONZE_AXE, "Bronze axe", 3),
            bank_row(PRODUCT, "Product", 2),
        ]),
        Vec::new(),
    );
    let mut withdrawal_tick = None;
    for tick in 2..=5 {
        assert!(poll_at(&mut run, &snapshot, None, &mut ledger, tick, OpenGate::Open).is_pending());
        if ledger.as_ref().unwrap().outbox.is_empty() {
            continue;
        }
        match complete_interaction(&mut ledger, tick) {
            HostEffect::Interaction(crate::shim::InteractReq::SetNoteMode {
                on: false, ..
            }) => {}
            HostEffect::Interaction(crate::shim::InteractReq::WithdrawX {
                name,
                bank_item_id,
                count,
                ..
            }) => {
                assert_eq!(bank_item_id, BRONZE_AXE);
                assert_eq!(
                    name, "Bronze axe",
                    "the host validates both the id and name"
                );
                assert_eq!(count, 1);
                withdrawal_tick = Some(tick);
                break;
            }
            _ => panic!("live re-plan dispatched an unexpected effect"),
        }
    }
    let withdrawal_tick = withdrawal_tick.expect("the alternative must be withdrawn");
    snapshot.seed_inventory(
        vec![item(
            BRONZE_AXE,
            "Bronze axe",
            1,
            ItemContainer::Inventory,
            &["Wield"],
        )],
        28,
    );
    let receipt = done(poll_at(
        &mut run,
        &snapshot,
        None,
        &mut ledger,
        withdrawal_tick + 1,
        OpenGate::Open,
    ));
    assert!(receipt.complete && receipt.withdrew_any && receipt.bank_open);
    assert_eq!(receipt.deposited, 2);
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
        matches!(
            first_effect(&ledger),
            HostEffect::Interaction(crate::shim::InteractReq::Held {
                action,
                slot: Some(0),
                target_item_id: Some(ITEM),
                ..
            }) if action.as_str() == "Wield"
        ),
        "wear must preserve the exact observed inventory id/slot for host resolution"
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
        HostEffect::Interaction(crate::shim::InteractReq::Held { .. })
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
fn shop_wear_resolution_uses_the_actual_tier_or_skips_a_carried_tool() {
    for wear in [Some(1351), None] {
        let (mut snapshot, _bank, banks) = fixture();
        let held_id = wear.unwrap_or(1359);
        let mut row = item(
            held_id,
            "Observed axe",
            1,
            ItemContainer::Inventory,
            &["Wield"],
        );
        row.slot = 7;
        snapshot.seed_inventory(vec![row], 28);
        let mut tool = need(
            1349,
            "Iron axe",
            1,
            1,
            Presence::Wielded,
            NeedKind::Required,
            SourceSet::PACK | SourceSet::WORN | SourceSet::SHOP,
        );
        tool.alternatives = Arc::from([1349, 1351, 1359]);
        let request = request(
            BankStage::Planned,
            vec![tool],
            None,
            None,
            false,
            false,
            CloseBank::LeaveOpen,
        );
        let mut plan = AcquirePlan::empty();
        plan.purchases[0] = Some(Purchase {
            need: 0,
            item: 1349,
            target: 1,
            minimum: 1,
        });
        plan.purchase_count = 1;
        let mut ledger = None;
        let mut run = crate::quester::families::tests::with_tick_bank(
            &snapshot,
            None,
            &mut ledger,
            1,
            |tick| AcquireRun::new(request, plan, &banks, &tick.cx),
        );
        assert!(matches!(
            run.wears.as_deref(),
            Some([WearStep::PendingPurchase(0)])
        ));
        // Resolve the same private outcome seam used after the routed shop run.
        run.settle_shop_wear(0, wear);
        run.purchase_index = 1;
        let outcome = poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open);
        if let Some(id) = wear {
            assert!(outcome.is_pending());
            assert!(matches!(
                first_effect(&ledger),
                HostEffect::Interaction(crate::shim::InteractReq::Held {
                    slot: Some(7),
                    target_item_id: Some(target),
                    ..
                }) if *target == id
            ));
            complete_interaction(&mut ledger, 1);
            snapshot.seed_inventory(Vec::new(), 28);
            snapshot.seed_equipment(vec![item(
                id,
                "Observed axe",
                1,
                ItemContainer::Equipment,
                &[],
            )]);
            assert!(
                done(poll_at(
                    &mut run,
                    &snapshot,
                    None,
                    &mut ledger,
                    2,
                    OpenGate::Open,
                ))
                .complete
            );
        } else {
            assert!(done(outcome).complete);
        }
        assert!(ledger
            .as_ref()
            .is_none_or(|ledger| ledger.outbox.is_empty()));
    }
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
fn chained_request_plans_from_open_rows_instead_of_stale_session_memory() {
    for (remembered, observed) in [(0, 7), (7, 0)] {
        let (mut snapshot, _bank, _banks) = fixture();
        open_bank(
            &mut snapshot,
            Some(if observed > 0 {
                vec![bank_row(ITEM, "Widget", observed)]
            } else {
                Vec::new()
            }),
            Vec::new(),
        );
        let memory = BankMemory::seeded(&[(ITEM, remembered)], Origin::Session);
        let request = request(
            BankStage::Planned,
            vec![need(
                ITEM,
                "Widget",
                5,
                1,
                Presence::Held,
                NeedKind::Required,
                SourceSet::PACK | SourceSet::BANK,
            )],
            None,
            None,
            false,
            true,
            CloseBank::LeaveOpen,
        );
        let mut ledger = None;
        crate::quester::families::tests::with_tick_bank(
            &snapshot,
            Some(&memory),
            &mut ledger,
            1,
            |tick| {
                let remembered_verdict =
                    plan::plan(&request, &tick.cx.snapshot().stock(), WorldFacts::default())
                        .unwrap();
                let current_verdict = plan_current(&request, &tick.cx).unwrap();
                if observed > 0 {
                    assert!(matches!(remembered_verdict, AcquireVerdict::Blocked(_)));
                    let AcquireVerdict::Plan(plan) = current_verdict else {
                        panic!("the open bank has the required item");
                    };
                    assert_eq!(plan.withdrawal_count, 1);
                    let row = plan.withdrawals[0].as_ref().unwrap();
                    assert_eq!((row.id, row.target), (ITEM, 5));
                } else {
                    assert!(matches!(remembered_verdict, AcquireVerdict::Plan(_)));
                    let AcquireVerdict::Blocked(block) = current_verdict else {
                        panic!("the open bank disproves the stale remembered stock");
                    };
                    assert_eq!(block.reason, BlockReason::SupplyMissing);
                    assert_eq!(block.looked.bank.unwrap().count, 0);
                }
            },
        );
        assert!(ledger.is_none(), "planning never emits an action");
    }
}

#[test]
fn wear_admission_keeps_the_gatherer_tool_failure_codes() {
    for (case, expected) in [
        (0, "tool-missing: the selected tool left the inventory"),
        (1, "tool-missing: the selected tool has no observed name"),
        (
            2,
            "tool-unusable: the selected tool has no observed equip action",
        ),
    ] {
        let (mut snapshot, _bank, banks) = fixture();
        let row = item(ITEM, "Widget", 1, ItemContainer::Inventory, &["Wield"]);
        snapshot.seed_inventory(vec![row.clone()], 28);
        let request = request(
            BankStage::Planned,
            vec![need(
                ITEM,
                "Widget",
                1,
                1,
                Presence::Wielded,
                NeedKind::Required,
                SourceSet::PACK | SourceSet::WORN,
            )],
            None,
            None,
            false,
            false,
            CloseBank::Close,
        );
        let mut ledger = None;
        let mut run = new_run(request, &snapshot, None, &mut ledger, &banks);
        let mut changed = row;
        let inventory = match case {
            0 => Vec::new(),
            1 => {
                changed.def.name = None;
                vec![changed]
            }
            2 => {
                changed.actions.clear();
                vec![changed]
            }
            _ => unreachable!(),
        };
        snapshot.seed_inventory(inventory, 28);
        assert!(matches!(
            poll_at(&mut run, &snapshot, None, &mut ledger, 2, OpenGate::Open),
            Poll::Ready(Err(ActionError::Failed(message))) if message.as_ref() == expected
        ));
        assert!(ledger.is_none(), "a missing equip action emits nothing");
    }
}

#[test]
fn shut_bank_with_stuck_side_root_fails_before_wear_or_completion() {
    let (mut snapshot, bank, banks) = fixture();
    open_bank(&mut snapshot, Some(Vec::new()), Vec::new());
    let side = snapshot.modals().side;
    let mut request = authored_request(Some(bank), false, Vec::new());
    request.close = CloseBank::Close;
    let mut ledger = None;
    let mut run = new_run(request, &snapshot, None, &mut ledger, &banks);
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open).is_pending());
    assert!(is_close(first_effect(&ledger)));
    complete_interaction(&mut ledger, 1);
    snapshot.seed_bank_observation(-1, 1, None, Vec::new());
    snapshot.seed_side_modal(side);
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 2, OpenGate::Open).is_pending());
    assert!(matches!(
        poll_at(&mut run, &snapshot, None, &mut ledger, 9, OpenGate::Open),
        Poll::Ready(Err(ActionError::Failed(message)))
            if message.as_ref() == "bank-close-failed: bank did not close"
    ));
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
}

#[test]
fn acquisition_owner_layout() {
    use std::mem::size_of;
    println!(
        "AcquireRun={} AcquireRequest={} AcquireReceipt={} BankReceipt={} Provisioner={} Quester={}",
        size_of::<AcquireRun>(),
        size_of::<AcquireRequest>(),
        size_of::<AcquireReceipt>(),
        size_of::<BankReceipt>(),
        size_of::<crate::quester::provision::Provisioner>(),
        size_of::<crate::quester::runner::Quester>(),
    );
    // First arm64 build: 712 bytes. Keep 56 bytes of explicit headroom.
    assert!(size_of::<AcquireRun>() <= 768);
    assert!(size_of::<WearStep>() <= 8);
    assert_eq!(size_of::<Option<Box<[WearStep]>>>(), 2 * size_of::<usize>());
    assert_eq!(size_of::<BankReceipt>(), 8);
}

#[test]
fn acquisition_receipt_preserves_more_than_255_landed_units() {
    let (snapshot, _bank, banks) = fixture();
    let mut ledger = None;
    let mut run =
        crate::quester::families::tests::with_tick_bank(&snapshot, None, &mut ledger, 1, |tick| {
            AcquireRun::new(
                request(
                    BankStage::Planned,
                    Vec::new(),
                    None,
                    None,
                    false,
                    false,
                    CloseBank::Close,
                ),
                AcquirePlan::empty(),
                &banks,
                &tick.cx,
            )
        });
    // The shop stage accumulates u32 landed counts. Receipt projection must
    // preserve those units rather than narrowing or saturating them at 255.
    run.bought = 300;
    let receipt = done(poll_at(
        &mut run,
        &snapshot,
        None,
        &mut ledger,
        2,
        OpenGate::Open,
    ));
    assert_eq!(receipt.bought, 300);
    assert!(receipt.complete);
    assert!(ledger
        .as_ref()
        .is_none_or(|ledger| ledger.outbox.is_empty()));
}
