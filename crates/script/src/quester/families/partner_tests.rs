use super::*;
use crate::native::{HostEffect, InteractionReceipt};
use crate::quester::families::tests::{def, with_tick};
use api::selected::{Knowledge, RunKey};
#[cfg(feature = "load")]
use api::snapshot::SnapshotView;
use api::snapshot::{GameSnapshot, ItemActionFamily, ItemContainer};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

type Ledger = Option<Box<crate::native::ledger::Ledger>>;

// These gates stand for the host broker's independently tested two-screen
// barriers. Every other port operation is forbidden in this machine harness.
#[derive(Default)]
struct Port {
    offer: AtomicBool,
    confirm: AtomicBool,
    cancelled: AtomicBool,
    not_ready: AtomicBool,
}

struct CommandPlan;
impl StepPlan for CommandPlan {
    fn begin(&self, _: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        unreachable!("the transfer machine does not dispatch its broker command")
    }
}

impl QuestPairPort for Port {
    fn shared(&self) -> Arc<dyn QuestPairPort> {
        unreachable!()
    }
    fn observe(&self, _: PairRegistration, _: PairFrame<'_>) {
        unreachable!()
    }
    fn invalidate(&self, _: RunKey) {
        unreachable!()
    }
    fn busy(&self) -> bool {
        unreachable!()
    }
    fn world_changed(&self, _: &str, _: u16) {
        unreachable!()
    }
    fn settings(&self, _: RunKey) -> Result<PairSettings, PairError> {
        unreachable!()
    }
    fn observe_gang(
        &self,
        _: &api::quest_progress::JournalRead,
    ) -> Result<Knowledge<Option<Gang>>, PairError> {
        unreachable!()
    }
    fn gang(&self, _: RunKey) -> Result<(Knowledge<Option<Gang>>, EvidenceStamp), PairError> {
        unreachable!()
    }
    fn partner_item_count(&self, _: EvidenceStamp, _: &PairItemRequest) -> Result<i32, PairError> {
        unreachable!()
    }
    fn waiting(&self, _: RunKey) -> bool {
        unreachable!()
    }
    fn token(&self, _: RunKey, _: &FactKey) -> Result<PairToken, PairError> {
        unreachable!()
    }
    fn register_action(
        &self,
        _: &PairToken,
        _: RunKey,
        _: crate::native::ActionRevoker,
    ) -> Result<(), PairError> {
        Ok(())
    }
    fn begin(&self, _: PairRequest) -> Result<PairToken, PairError> {
        unreachable!()
    }
    fn poll(&self, token: &PairToken, caller: RunKey) -> Poll<Result<PairStep, PairError>> {
        assert_eq!(*token, pair_token());
        if self.not_ready.load(Ordering::Relaxed) {
            return Poll::Ready(Err(PairError::NotReady));
        }
        assert_eq!(caller, token.left);
        if self.cancelled.load(Ordering::Relaxed) {
            return Poll::Ready(Err(PairError::Cancelled));
        }
        Poll::Ready(Ok(PairStep::Act(RoleCommand {
            token: *token,
            recipient: caller,
            phase: FactKey::new("arrav:key"),
            command_id: 1,
            plan: Arc::new(CommandPlan),
        })))
    }
    fn report(&self, _: &PairToken, _: RoleReceipt) -> Result<(), PairError> {
        unreachable!()
    }
    fn trade_ready(
        &self,
        token: &PairToken,
        actor: RunKey,
        confirm: bool,
        evidence: EvidenceStamp,
    ) -> Result<bool, PairError> {
        assert_eq!(*token, pair_token());
        assert_eq!(actor, token.left);
        assert_eq!(evidence.run, actor);
        Ok(if confirm { &self.confirm } else { &self.offer }.load(Ordering::Relaxed))
    }
    fn gameplay_progress(&self, _: RunKey, _: EvidenceStamp, _: Instant) {
        unreachable!()
    }
    fn cancel(&self, _: &PairToken) {}
}

fn pair_token() -> PairToken {
    PairToken {
        id: 1,
        generation: 1,
        left: RunKey {
            slot: 1,
            run: 1,
            session: 1,
        },
        right: RunKey {
            slot: 2,
            run: 1,
            session: 1,
        },
    }
}

fn item(id: i32, qty: i32, container: ItemContainer) -> ItemView {
    ItemView {
        def: def(id, "quest item"),
        container,
        action_family: ItemActionFamily::Component,
        slot: 0,
        count: qty,
        actions: vec![],
        component_id: crate::trade_screen::OFFER_INV,
    }
}

fn offer(give: i32) -> TradeView {
    TradeView {
        offer_open: true,
        my_offer: (give > 0)
            .then(|| item(1, give, ItemContainer::TradeMyOffer))
            .into_iter()
            .collect(),
        their_offer: vec![item(2, 1, ItemContainer::TradeTheirOffer)],
        side_pack: vec![item(1, 2, ItemContainer::TradeSidePack)],
        partner: Some("Bob 1".into()),
        accept_component_id: 9,
        decline_component_id: 17,
        ..TradeView::default()
    }
}

fn fixture(budget: u16) -> (GameSnapshot, Ledger, ActionHandle<TradeMachine>, Arc<Port>) {
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_inventory(vec![item(1, 2, ItemContainer::Inventory)], 28);
    snapshot.seed_trade(offer(0));
    let mut ledger = None;
    let port = Arc::new(Port::default());
    let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<TradeMachine>(
                TradeArgs {
                    port: Arc::clone(&port) as Arc<dyn QuestPairPort>,
                    token: pair_token(),
                    partner: AccountKey(Arc::from("bob_1")),
                    give: Arc::from([Item { id: 1, qty: 2 }]),
                    take: Arc::from([Item { id: 2, qty: 1 }]),
                    budget,
                },
                &mut tick.cx,
            )
            .unwrap()
    });
    (snapshot, ledger, handle, port)
}

fn poll(
    snapshot: &GameSnapshot,
    ledger: &mut Ledger,
    handle: &ActionHandle<TradeMachine>,
    tick: u64,
    epoch: Instant,
) -> Poll<Result<StepOutcome, ActionError>> {
    with_tick(snapshot, ledger, tick, |frame| {
        frame.cx.wall_now = epoch + Duration::from_millis(tick * 600);
        frame.actions.poll(handle, &mut frame.cx)
    })
}

fn ack(ledger: &mut Ledger, tick: u64) -> InteractReq {
    ack_result(ledger, tick, true)
}

fn ack_result(ledger: &mut Ledger, tick: u64, accepted: bool) -> InteractReq {
    let ledger = ledger.as_mut().unwrap();
    let action = ledger.outbox.remove(0);
    ledger.complete_interaction(
        &action.authority(),
        InteractionReceipt {
            request_id: action.request_id.get(),
            evidence: EvidenceStamp {
                run: action.run(),
                tick,
                sequence: tick,
            },
            accepted,
            chat_since: 0,
        },
    );
    match action.effect {
        HostEffect::Interaction(request) => request,
        _ => panic!("expected owned trade interaction"),
    }
}

fn with_step<R>(
    snapshot: &GameSnapshot,
    ledger: &mut Ledger,
    tick: u64,
    f: impl FnOnce(&mut StepContext<'_, '_>) -> R,
) -> R {
    with_tick(snapshot, ledger, tick, |native| {
        let quests = api::quest_facts::QuestCatalog::empty();
        let required_after = native.cx.evidence();
        let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
        let choices = crate::quester::choices::QuestChoices::default();
        f(&mut StepContext {
            tick: native,
            quests: &quests,
            progress: &[],
            required_after,
            banks: &banks,
            choices: &choices,
        })
    })
}

struct PendingAction;
impl NativeMachine for PendingAction {
    type Args = ();
    type Output = ();

    fn begin(_: (), _: &mut crate::native::ActionContext<'_>) -> Result<Self, ActionError> {
        Ok(Self)
    }

    fn poll(&mut self, _: &mut crate::native::ActionContext<'_>) -> Poll<Result<(), ActionError>> {
        Poll::Pending
    }

    fn cancel(&mut self) {}
}

fn trade_snapshot() -> GameSnapshot {
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_inventory(vec![item(1, 2, ItemContainer::Inventory)], 28);
    snapshot.seed_trade(offer(0));
    snapshot
}

fn trade_args(port: &Arc<Port>, budget: u16) -> TradeArgs {
    TradeArgs {
        port: Arc::clone(port) as Arc<dyn QuestPairPort>,
        token: pair_token(),
        partner: AccountKey(Arc::from("bob_1")),
        give: Arc::from([Item { id: 1, qty: 2 }]),
        take: Arc::from([Item { id: 2, qty: 1 }]),
        budget,
    }
}

#[test]
fn partner_run_waits_for_a_transient_pair_not_ready_observation() {
    let snapshot = trade_snapshot();
    let mut ledger = None;
    let port = Arc::new(Port::default());
    port.not_ready.store(true, Ordering::Relaxed);
    let mut run = PartnerRun {
        port: Arc::clone(&port) as Arc<dyn QuestPairPort>,
        token: pair_token(),
        phase: FactKey::new("arrav:key"),
        command: None,
        active: None,
        reported: false,
        waiting: false,
    };

    assert!(with_step(&snapshot, &mut ledger, 2, |cx| run.poll(cx)).is_pending());
    assert!(run.waiting);
}

#[test]
fn trade_machine_waits_for_a_transient_pair_not_ready_observation() {
    let (snapshot, mut ledger, handle, port) = fixture(40);
    port.not_ready.store(true, Ordering::Relaxed);
    assert!(poll(&snapshot, &mut ledger, &handle, 2, Instant::now()).is_pending());
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
}

#[test]
fn trade_run_retains_args_for_each_transient_begin_error_then_starts() {
    for cause in 0..3 {
        let snapshot = trade_snapshot();
        let mut ledger = None;
        let port = Arc::new(Port::default());
        let mut run = TradeRun {
            walk: None,
            trade: None,
            args: Some(trade_args(&port, 40)),
        };
        let blocker = if cause == 0 {
            Some(with_tick(&snapshot, &mut ledger, 1, |tick| {
                tick.actions
                    .begin::<PendingAction>((), &mut tick.cx)
                    .unwrap()
            }))
        } else {
            None
        };

        let first = with_step(&snapshot, &mut ledger, 2, |cx| {
            match cause {
                0 => {}
                1 => cx.tick.cx.eligible = false,
                2 => {
                    for _ in 0..32 {
                        assert!(cx.tick.cx.budget.transition());
                    }
                }
                _ => unreachable!(),
            }
            run.poll(cx)
        });
        assert!(first.is_pending(), "transient begin cause {cause}");
        assert!(run.args.is_some(), "args must survive cause {cause}");
        assert!(run.trade.is_none());
        drop(blocker);

        assert!(with_step(&snapshot, &mut ledger, 3, |cx| run.poll(cx)).is_pending());
        assert!(run.args.is_none(), "successful begin consumes the args");
        assert!(
            run.trade.is_some(),
            "retry must start the native trade machine"
        );
    }
}

#[test]
fn failed_begin_missing_quest_item_declines_existing_trade_screen() {
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_inventory(vec![], 28);
    snapshot.seed_trade(offer(0));
    let mut ledger = None;
    let port = Arc::new(Port::default());
    let mut run = TradeRun {
        walk: None,
        trade: None,
        args: Some(trade_args(&port, 40)),
    };
    let epoch = Instant::now();

    assert!(with_step(&snapshot, &mut ledger, 2, |cx| {
        cx.tick.cx.wall_now = epoch + Duration::from_millis(1_200);
        run.poll(cx)
    })
    .is_pending());
    assert_eq!(
        ack(&mut ledger, 2),
        InteractReq::IfButton { component_id: 17 },
        "a permanent begin validation failure must decline the existing trade"
    );

    snapshot.seed_trade(TradeView::default());
    assert!(with_step(&snapshot, &mut ledger, 3, |cx| {
        cx.tick.cx.wall_now = epoch + Duration::from_millis(1_800);
        run.poll(cx)
    })
    .is_pending());
    assert!(matches!(
        with_step(&snapshot, &mut ledger, 4, |cx| {
            cx.tick.cx.wall_now = epoch + Duration::from_millis(2_400);
            run.poll(cx)
        }),
        Poll::Ready(Err(ActionError::Blocked(reason)))
            if reason.as_ref() == "partner handoff quest item is missing"
    ));
}

#[test]
fn failed_trade_offer_mismatch_declines_and_waits_for_closed_screen() {
    let (mut snapshot, mut ledger, handle, _) = fixture(40);
    let epoch = Instant::now();
    assert!(poll(&snapshot, &mut ledger, &handle, 2, epoch).is_pending());

    let mut mismatch = offer(2);
    mismatch
        .their_offer
        .push(item(99, 1, ItemContainer::TradeTheirOffer));
    snapshot.seed_trade(mismatch);
    assert!(poll(&snapshot, &mut ledger, &handle, 3, epoch).is_pending());
    assert_eq!(
        ack(&mut ledger, 3),
        InteractReq::IfButton { component_id: 17 },
        "mismatched offer cleanup must click the posted decline button"
    );

    snapshot.seed_trade(TradeView::default());
    assert!(poll(&snapshot, &mut ledger, &handle, 4, epoch).is_pending());
    assert!(matches!(
        poll(&snapshot, &mut ledger, &handle, 5, epoch),
        Poll::Ready(Err(ActionError::Blocked(reason)))
            if reason.as_ref() == "partner trade contains extra, noted or excessive items"
    ));
}

#[test]
fn unexpected_confirmation_declines_instead_of_leaving_the_trade_open() {
    let (mut snapshot, mut ledger, handle, _) = fixture(40);
    let epoch = Instant::now();
    let mut confirm = offer(0);
    confirm.offer_open = false;
    confirm.confirm_open = true;
    confirm.decline_component_id = 18;
    snapshot.seed_trade(confirm);

    assert!(poll(&snapshot, &mut ledger, &handle, 2, epoch).is_pending());
    assert_eq!(
        ack(&mut ledger, 2),
        InteractReq::IfButton { component_id: 18 },
        "unexpected confirmation cleanup must click its posted decline button"
    );
    snapshot.seed_trade(TradeView::default());
    assert!(poll(&snapshot, &mut ledger, &handle, 3, epoch).is_pending());
    assert!(matches!(
        poll(&snapshot, &mut ledger, &handle, 4, epoch),
        Poll::Ready(Err(ActionError::Blocked(reason)))
            if reason.as_ref() == "unexpected existing confirmation; reread both inventories"
    ));
}

#[test]
fn timed_out_trade_falls_back_to_close_modal_and_preserves_failure() {
    let (mut snapshot, mut ledger, handle, _) = fixture(2);
    let epoch = Instant::now();
    assert!(poll(&snapshot, &mut ledger, &handle, 2, epoch).is_pending());
    assert!(poll(&snapshot, &mut ledger, &handle, 3, epoch).is_pending());
    assert_eq!(
        ack(&mut ledger, 3),
        InteractReq::IfButton { component_id: 17 },
        "timeout cleanup must first click the posted decline button"
    );
    assert!(poll(&snapshot, &mut ledger, &handle, 5, epoch).is_pending());
    assert_eq!(
        ack(&mut ledger, 5),
        InteractReq::CloseModal,
        "an unclosed decline screen must fall back to the shared close path"
    );
    snapshot.seed_trade(TradeView::default());
    assert!(poll(&snapshot, &mut ledger, &handle, 6, epoch).is_pending());
    assert!(matches!(
        poll(&snapshot, &mut ledger, &handle, 7, epoch),
        Poll::Ready(Err(ActionError::Blocked(reason)))
            if reason.as_ref() == "partner trade deadline exhausted"
    ));
}

#[test]
fn deadline_cleanup_supersedes_unacknowledged_accept_and_decline() {
    let (mut snapshot, mut ledger, handle, port) = fixture(4);
    let epoch = Instant::now();
    assert!(poll(&snapshot, &mut ledger, &handle, 2, epoch).is_pending());
    snapshot.seed_trade(offer(2));
    port.offer.store(true, Ordering::Relaxed);
    assert!(poll(&snapshot, &mut ledger, &handle, 3, epoch).is_pending());
    assert!(matches!(
        &ledger.as_ref().unwrap().outbox[0].effect,
        HostEffect::Interaction(InteractReq::IfButton { component_id: 9 })
    ));
    // No dispatch receipt arrives. Failure cleanup must replace acceptance
    // authority, not wait on it and leave the confirmation screen open.
    assert!(poll(&snapshot, &mut ledger, &handle, 5, epoch).is_pending());
    let live: Vec<_> = ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .filter(|action| action.live())
        .collect();
    assert_eq!(live.len(), 1);
    assert!(matches!(
        &live[0].effect,
        HostEffect::Interaction(InteractReq::IfButton { component_id: 17 })
    ));
    assert!(poll(&snapshot, &mut ledger, &handle, 7, epoch).is_pending());
    let live: Vec<_> = ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .filter(|action| action.live())
        .collect();
    assert_eq!(live.len(), 1);
    assert!(matches!(
        &live[0].effect,
        HostEffect::Interaction(InteractReq::CloseModal)
    ));
    snapshot.seed_trade(TradeView::default());
    assert!(poll(&snapshot, &mut ledger, &handle, 8, epoch).is_pending());
    assert!(matches!(poll(&snapshot, &mut ledger, &handle, 9, epoch),
        Poll::Ready(Err(ActionError::Blocked(reason))) if reason.as_ref() == "partner trade deadline exhausted"));
}

#[test]
fn refused_trade_dispatch_still_declines_the_open_screen() {
    let (mut snapshot, mut ledger, handle, port) = fixture(40);
    let epoch = Instant::now();
    assert!(poll(&snapshot, &mut ledger, &handle, 2, epoch).is_pending());
    snapshot.seed_trade(offer(2));
    port.offer.store(true, Ordering::Relaxed);
    assert!(poll(&snapshot, &mut ledger, &handle, 3, epoch).is_pending());
    assert_eq!(
        ack_result(&mut ledger, 3, false),
        InteractReq::IfButton { component_id: 9 }
    );

    assert!(poll(&snapshot, &mut ledger, &handle, 4, epoch).is_pending());
    assert_eq!(
        ack(&mut ledger, 4),
        InteractReq::IfButton { component_id: 17 },
        "refused accept cleanup must decline the still-open trade"
    );
    snapshot.seed_trade(TradeView::default());
    assert!(poll(&snapshot, &mut ledger, &handle, 5, epoch).is_pending());
    assert!(matches!(
        poll(&snapshot, &mut ledger, &handle, 6, epoch),
        Poll::Ready(Err(ActionError::Blocked(reason)))
            if reason.as_ref() == "partner trade dispatch refused"
    ));
}

#[test]
fn transfer_requires_both_screens_then_closed_ui_and_fresh_inventory_delta() {
    let (mut snapshot, mut ledger, handle, port) = fixture(40);
    let epoch = Instant::now();
    assert!(poll(&snapshot, &mut ledger, &handle, 2, epoch).is_pending());
    for (tick, offered) in [(3, 0), (4, 1)] {
        snapshot.seed_trade(offer(offered));
        assert!(poll(&snapshot, &mut ledger, &handle, tick, epoch).is_pending());
        assert!(matches!(
            ack(&mut ledger, tick),
            InteractReq::InvButton {
                id: 1,
                operation: 1,
                component: crate::trade_screen::OFFER_INV,
                ..
            }
        ));
    }
    snapshot.seed_trade(offer(2));
    assert!(poll(&snapshot, &mut ledger, &handle, 5, epoch).is_pending());
    assert!(
        ledger.as_ref().unwrap().outbox.is_empty(),
        "a local exact offer does not satisfy the mutual barrier"
    );
    port.offer.store(true, Ordering::Relaxed);
    assert!(poll(&snapshot, &mut ledger, &handle, 6, epoch).is_pending());
    assert!(matches!(
        ack(&mut ledger, 6),
        InteractReq::IfButton { component_id: 9 }
    ));
    let mut confirm = offer(2);
    confirm.offer_open = false;
    confirm.confirm_open = true;
    confirm.accept_component_id = 12;
    snapshot.seed_trade(confirm);
    assert!(poll(&snapshot, &mut ledger, &handle, 7, epoch).is_pending());
    assert!(poll(&snapshot, &mut ledger, &handle, 8, epoch).is_pending());
    assert!(
        ledger.as_ref().unwrap().outbox.is_empty(),
        "a local confirmation does not satisfy the mutual barrier"
    );
    port.confirm.store(true, Ordering::Relaxed);
    assert!(poll(&snapshot, &mut ledger, &handle, 9, epoch).is_pending());
    assert!(matches!(
        ack(&mut ledger, 9),
        InteractReq::IfButton { component_id: 12 }
    ));
    snapshot.seed_trade(TradeView::default());
    assert!(poll(&snapshot, &mut ledger, &handle, 10, epoch).is_pending());
    assert!(poll(&snapshot, &mut ledger, &handle, 11, epoch).is_pending());
    assert!(
        poll(&snapshot, &mut ledger, &handle, 12, epoch).is_pending(),
        "closed screens and accepts are not a transfer receipt"
    );
    snapshot.seed_inventory(vec![item(2, 1, ItemContainer::Inventory)], 28);
    #[cfg(feature = "load")]
    with_tick(&snapshot, &mut ledger, 13, |frame| {
        let stale = EvidenceStamp {
            run: frame.cx.run(),
            tick: 1,
            sequence: 1,
        };
        frame.cx.snapshot = SnapshotView::new(frame.frame.snapshot, stale);
        assert!(
            frame.actions.poll(&handle, &mut frame.cx).is_pending(),
            "matching counts in stale inventory cannot settle"
        );
    });
    match poll(&snapshot, &mut ledger, &handle, 14, epoch) {
        Poll::Ready(Ok(outcome)) => assert_eq!(outcome.evidence.tick, 14),
        _ => panic!("fresh exact inventory deltas must settle"),
    }
}

#[test]
fn counterpart_extra_noted_or_excessive_offer_blocks_before_accept() {
    for fault in 0..4 {
        let (mut snapshot, mut ledger, handle, _) = fixture(40);
        let epoch = Instant::now();
        assert!(poll(&snapshot, &mut ledger, &handle, 2, epoch).is_pending());
        let mut trade = offer(2);
        match fault {
            0 => trade.partner = Some("mallory".into()),
            1 => trade
                .my_offer
                .push(item(99, 1, ItemContainer::TradeMyOffer)),
            2 => trade.their_offer[0].def.noted = true,
            3 => trade.their_offer[0].count = 2,
            _ => unreachable!(),
        }
        snapshot.seed_trade(trade);
        assert!(poll(&snapshot, &mut ledger, &handle, 3, epoch).is_pending());
        assert_eq!(
            ack(&mut ledger, 3),
            InteractReq::IfButton { component_id: 17 },
            "an invalid trade must decline, never accept"
        );
        snapshot.seed_trade(TradeView::default());
        assert!(poll(&snapshot, &mut ledger, &handle, 4, epoch).is_pending());
        let expected = if fault == 0 {
            "partner trade counterpart changed"
        } else {
            "partner trade contains extra, noted or excessive items"
        };
        assert!(matches!(
            poll(&snapshot, &mut ledger, &handle, 5, epoch),
            Poll::Ready(Err(ActionError::Blocked(reason))) if reason.as_ref() == expected
        ));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }
}

#[test]
fn cancelled_or_expired_phase_never_emits_an_accept() {
    let epoch = Instant::now();
    for cancelled in [false, true] {
        let (mut snapshot, mut ledger, handle, port) = fixture(if cancelled { 40 } else { 2 });
        assert!(poll(&snapshot, &mut ledger, &handle, 2, epoch).is_pending());
        snapshot.seed_trade(offer(2));
        port.offer.store(true, Ordering::Relaxed);
        port.cancelled.store(cancelled, Ordering::Relaxed);
        let result = poll(&snapshot, &mut ledger, &handle, 3, epoch);
        if cancelled {
            assert!(matches!(
                &result,
                Poll::Ready(Err(ActionError::Blocked(reason)))
                    if reason.as_ref() == "pair cancelled; Stop and freshly Start both accounts"
            ));
        } else {
            assert!(result.is_pending());
            assert_eq!(
                ack(&mut ledger, 3),
                InteractReq::IfButton { component_id: 17 },
                "a locally expired trade must decline, never accept"
            );
            snapshot.seed_trade(TradeView::default());
            assert!(poll(&snapshot, &mut ledger, &handle, 4, epoch).is_pending());
            assert!(matches!(
                poll(&snapshot, &mut ledger, &handle, 5, epoch),
                Poll::Ready(Err(ActionError::Blocked(reason)))
                    if reason.as_ref() == "partner trade deadline exhausted"
            ));
        }
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }
}

#[test]
fn recovery_item_fact_compiles_only_bounded_arrav_items_and_an_authored_pair_role() {
    use crate::quester::compile::{CompiledProgress, PairCompileContext};
    use crate::quester::loadouts::LoadoutOverlay;
    use api::quest_facts::QuestCatalog;
    use api::selected::ClientRevision;
    use std::collections::HashMap;

    let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let quests = QuestCatalog::from_identity(selected.quest_identity()).unwrap();
    let path = FactKey::new("blackarmgang");
    let role = FactKey::new("blackarm");
    let declaration = PartnerDeclaration {
        protocol: FactKey::new("arrav"),
        roles: [
            PartnerRole {
                id: FactKey::new("phoenix"),
                gang: Gang::Phoenix,
            },
            PartnerRole {
                id: role.clone(),
                gang: Gang::BlackArm,
            },
        ],
    };
    let pair = PairCompileContext {
        declaration: &declaration,
        role: &role,
        digest: [1; 32],
    };
    let progress = CompiledProgress {
        binding: FactKey::new("journal:blackarmgang"),
        role: Some(role.clone()),
        colour_not_started: FactKey::new("blackarmgang:0"),
        colour_in_progress: FactKey::new("blackarmgang:1"),
        colour_complete: FactKey::new("blackarmgang:4"),
        stage_keys: Arc::from([]),
        rules: Arc::from([]),
        flags: Arc::from([]),
        monotonic: false,
    };
    let areas = HashMap::new();
    let recipes = HashMap::new();
    let loadouts = LoadoutOverlay::new(Arc::from([]), Arc::from([]));
    let mut cx = CompileContext {
        path: &path,
        kind: crate::quester::path::PathKind::Quest,
        pair: Some(pair),
        progress: &progress,
        selected: &selected,
        quests: &quests,
        gathering: None,
        areas: &areas,
        recipes: &recipes,
        bank: None,
        bank_required: false,
        keep_ids: &[],
        loadouts: &loadouts,
        loadout_carry: crate::quester::compile::empty_loadout_carry(),
    };
    for obj in ["arravshield1", "obj:arravshield2", "arravcertificate"] {
        assert!(compile_partner_item_count(
            PartnerItemCountArgs {
                obj: obj.into(),
                qty: 1
            },
            &cx
        )
        .is_ok());
    }
    for qty in [0, 29] {
        let error = compile_partner_item_count(
            PartnerItemCountArgs {
                obj: "arravshield2".into(),
                qty,
            },
            &cx,
        )
        .err()
        .unwrap();
        assert_eq!(error.code.as_ref(), "partner-invalid-quantity");
    }
    let error = compile_partner_item_count(
        PartnerItemCountArgs {
            obj: "coins".into(),
            qty: 1,
        },
        &cx,
    )
    .err()
    .unwrap();
    assert_eq!(error.code.as_ref(), "partner-item-not-a-recovery-item");
    cx.pair = None;
    let error = compile_partner_item_count(
        PartnerItemCountArgs {
            obj: "arravshield2".into(),
            qty: 1,
        },
        &cx,
    )
    .err()
    .unwrap();
    assert_eq!(error.code.as_ref(), "partner-declaration-required");
    let foreign = FactKey::new("foreign");
    cx.pair = Some(PairCompileContext {
        role: &foreign,
        ..pair
    });
    let error = compile_partner_item_count(
        PartnerItemCountArgs {
            obj: "arravshield2".into(),
            qty: 1,
        },
        &cx,
    )
    .err()
    .unwrap();
    assert_eq!(error.code.as_ref(), "partner-invalid-role");
    let hero = FactKey::new("hero");
    cx.path = &hero;
    cx.pair = Some(pair);
    let error = compile_partner_item_count(
        PartnerItemCountArgs {
            obj: "arravshield2".into(),
            qty: 1,
        },
        &cx,
    )
    .err()
    .unwrap();
    assert_eq!(error.code.as_ref(), "partner-item-not-a-recovery-item");
}
