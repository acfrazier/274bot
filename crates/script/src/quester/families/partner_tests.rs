use super::*;
use crate::native::{HostEffect, InteractionReceipt, WalkEnd};
use crate::quester::families::tests::{def, post_walk_receipt, with_tick};
use api::selected::{Knowledge, RunKey};

use api::snapshot::SnapshotView;
use api::snapshot::{GameSnapshot, ItemActionFamily, ItemContainer, TradeConfirmView};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

type Ledger = Option<Box<crate::native::ledger::Ledger>>;

// These gates stand for the host broker's independently tested two-screen
// barriers. Every other port operation is forbidden in this machine harness.
#[derive(Default)]
struct Port {
    offer: AtomicBool,
    confirm: AtomicBool,
    cancelled: AtomicBool,
    not_ready: AtomicBool,
    self_ref: std::sync::Mutex<Option<std::sync::Weak<Port>>>,
    action_revoker: std::sync::Mutex<Option<crate::native::ActionRevoker>>,
}

struct CommandPlan;
impl StepPlan for CommandPlan {
    fn begin(&self, _: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        unreachable!("the transfer machine does not dispatch its broker command")
    }
}

impl QuestPairPort for Port {
    fn shared(&self) -> Arc<dyn QuestPairPort> {
        self.self_ref
            .lock()
            .unwrap()
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
            .expect("test port shared before TradePlan begins")
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
        Ok(PairSettings {
            partner: Some(AccountKey(Arc::from("bob_1"))),
            gang: None,
        })
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
        Ok(pair_token())
    }
    fn register_action(
        &self,
        _: &PairToken,
        _: RunKey,
        action: crate::native::ActionRevoker,
    ) -> Result<(), PairError> {
        *self.action_revoker.lock().unwrap() = Some(action);
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

/// The offer screen as the client shows it after the mutual accept. The engine
/// stops the offer containers and the side pack (`trade.rs2:80-86`), the client
/// empties a stopped container (`client.rs:8389-8404`), and the main modal is
/// still the offer screen until `if_openmain(tradeconfirm)` (`trade.rs2:147`).
fn mutual_accept_offer_screen() -> TradeView {
    TradeView {
        my_offer: vec![],
        their_offer: vec![],
        side_pack: vec![],
        my_offer_stopped: true,
        their_offer_stopped: true,
        ..offer(0)
    }
}

/// The confirm screen as the client shows it: the offer containers stay
/// stopped and empty, and the engine's `tradeconfirm` lists carry the trade
/// (`trade.rs2:111-145`; rows are tagged with the offer they mirror).
fn confirm_screen() -> TradeView {
    TradeView {
        offer_open: false,
        confirm_open: true,
        accept_component_id: 12,
        decline_component_id: 18,
        confirm: Some(Box::new(TradeConfirmView {
            mine: vec![item(1, 2, ItemContainer::TradeMyOffer)],
            theirs: vec![item(2, 1, ItemContainer::TradeTheirOffer)],
        })),
        ..mutual_accept_offer_screen()
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

fn assert_trade_network_coords(
    local_rendered_x: i32,
    local_network_x: i32,
    partner_rendered_x: i32,
    partner_network_x: i32,
    expected: bool,
) {
    let at = |x| api::WorldTile {
        x,
        z: 100,
        level: 0,
    };
    let (mut snapshot, mut ledger, handle, _) = fixture(8);
    snapshot.seed_trade(api::snapshot::TradeView::default());
    snapshot.seed_local_player(crate::quester::families::tests::local_player_with_network(
        at(local_rendered_x),
        at(local_network_x),
    ));
    let mut partner = crate::quester::families::tests::local_player_with_network(
        at(partner_rendered_x),
        at(partner_network_x),
    )
    .player;
    partner.index = 1;
    partner.actor.name = Some("Bob 1".into());
    snapshot.seed_players(vec![partner]);
    assert!(poll(&snapshot, &mut ledger, &handle, 2, Instant::now()).is_pending());
    let requested = ledger.as_ref().unwrap().outbox.iter().any(|action| {
        matches!(
            &action.effect,
            HostEffect::Interaction(InteractReq::Player { .. })
        )
    });
    assert_eq!(
        requested, expected,
        "trade range must use both network actors"
    );
}

#[test]
fn network_coords_trade_uses_local_network_origin() {
    assert_trade_network_coords(110, 100, 101, 101, true);
}

#[test]
fn network_coords_trade_rejects_a_partner_only_rendered_nearby() {
    assert_trade_network_coords(100, 100, 101, 110, false);
}

#[test]
fn network_coords_trade_accepts_a_network_partner_while_rendering_trails() {
    assert_trade_network_coords(100, 100, 110, 101, true);
}

fn poll(
    snapshot: &GameSnapshot,
    ledger: &mut Ledger,
    handle: &ActionHandle<TradeMachine>,
    tick: u64,
    epoch: Instant,
) -> Poll<Result<StepOutcome, ActionError>> {
    with_tick(snapshot, ledger, tick, |frame| {
        // Both clocks stand still: the trade waits count observed game ticks only.
        frame.cx.wall_now = epoch;
        frame.cx.active_now = Duration::ZERO;
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
    if !api::test_support::game_data_available() {
        return;
    }
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
    if !api::test_support::game_data_available() {
        return;
    }
    let (snapshot, mut ledger, handle, port) = fixture(40);
    port.not_ready.store(true, Ordering::Relaxed);
    assert!(poll(&snapshot, &mut ledger, &handle, 2, Instant::now()).is_pending());
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
}

#[test]
fn trade_run_retains_args_for_each_transient_begin_error_then_starts() {
    if !api::test_support::game_data_available() {
        return;
    }
    for cause in 0..3 {
        let snapshot = trade_snapshot();
        let mut ledger = None;
        let port = Arc::new(Port::default());
        let mut run = TradeRun {
            walk: None,
            walk_retry: crate::native::WalkRetryState::default(),
            tile: api::WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
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
fn trade_run_retries_a_failed_approach_on_the_next_tick() {
    if !api::test_support::game_data_available() {
        return;
    }
    let snapshot = trade_snapshot();
    let tile = api::WorldTile {
        x: 12,
        z: 34,
        level: 0,
    };
    let mut ledger = None;
    let first_walk = with_tick(&snapshot, &mut ledger, 1, |tick| {
        PrereqWalk::begin(
            crate::quester::families::reach::walk_request(tile, 1, None, tick.cx.evidence()),
            tick.actions,
            &mut tick.cx,
        )
        .unwrap()
    });
    let port = Arc::new(Port::default());
    let mut walk_retry = crate::native::WalkRetryState::default();
    walk_retry.action_submitted();
    let mut run = TradeRun {
        walk: Some(first_walk),
        walk_retry,
        tile,
        trade: None,
        args: Some(trade_args(&port, 40)),
    };

    post_walk_receipt(
        &mut ledger,
        2,
        WalkEnd::NoPath,
        Some("partner approach failed"),
    );
    assert!(with_step(&snapshot, &mut ledger, 2, |cx| run.poll(cx)).is_pending());
    assert!(run.walk.is_none());

    assert!(with_step(&snapshot, &mut ledger, 3, |cx| run.poll(cx)).is_pending());
    let retry = ledger.as_ref().unwrap().outbox.last().unwrap();
    let HostEffect::Walk(request) = &retry.effect else {
        panic!("the retry should submit a fresh partner approach walk");
    };
    assert_eq!(request.target, tile);
    assert_eq!(request.required_after.tick, 3);
}

#[test]
fn trade_plan_registers_logical_pair_revoker_for_prerequisite_walk() {
    if !api::test_support::game_data_available() {
        return;
    }
    let snapshot = trade_snapshot();
    let mut ledger = None;
    static PORT: std::sync::LazyLock<Arc<Port>> = std::sync::LazyLock::new(|| {
        Arc::new_cyclic(|port| Port {
            self_ref: std::sync::Mutex::new(Some(port.clone())),
            ..Port::default()
        })
    });
    let port = &*PORT;
    let plan = TradePlan {
        phase: FactKey::new("arrav:key"),
        tile: api::WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        },
        give: Arc::from([Item { id: 1, qty: 2 }]),
        take: Arc::from([Item { id: 2, qty: 1 }]),
        budget: 40,
    };
    let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
        tick.pairs = Some(port.as_ref());
        let quests = api::quest_facts::QuestCatalog::empty();
        let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
        let choices = crate::quester::choices::QuestChoices::default();
        let required_after = tick.cx.evidence();
        plan.begin(&mut StepContext {
            tick,
            quests: &quests,
            progress: &[],
            required_after,
            banks: &banks,
            choices: &choices,
        })
        .unwrap()
    });

    assert!(ledger.as_ref().is_some_and(|ledger| {
        ledger
            .outbox
            .iter()
            .any(|operation| matches!(&operation.effect, HostEffect::Walk(_)))
    }));
    port.action_revoker
        .lock()
        .unwrap()
        .take()
        .expect("TradePlan registers the logical walk owner")
        .revoke();
    assert!(matches!(
        with_step(&snapshot, &mut ledger, 2, |cx| run.poll(cx)),
        Poll::Ready(Err(ActionError::Cancelled))
    ));
}

#[test]
fn failed_begin_missing_quest_item_declines_existing_trade_screen() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_inventory(vec![], 28);
    snapshot.seed_trade(offer(0));
    let mut ledger = None;
    let port = Arc::new(Port::default());
    let mut run = TradeRun {
        walk: None,
        trade: None,
        walk_retry: crate::native::WalkRetryState::default(),
        tile: api::WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        args: Some(trade_args(&port, 40)),
    };
    // Both clocks stand still: the trade waits count observed game ticks only.
    let epoch = Instant::now();

    assert!(with_step(&snapshot, &mut ledger, 2, |cx| {
        cx.tick.cx.wall_now = epoch;
        cx.tick.cx.active_now = Duration::ZERO;
        run.poll(cx)
    })
    .is_pending());
    assert_eq!(
        ack(&mut ledger, 2),
        InteractReq::CloseModal,
        "a permanent begin validation failure must close the existing trade"
    );

    snapshot.seed_trade(TradeView::default());
    assert!(with_step(&snapshot, &mut ledger, 3, |cx| {
        cx.tick.cx.wall_now = epoch;
        cx.tick.cx.active_now = Duration::ZERO;
        run.poll(cx)
    })
    .is_pending());
    assert!(matches!(
        with_step(&snapshot, &mut ledger, 4, |cx| {
            cx.tick.cx.wall_now = epoch;
            cx.tick.cx.active_now = Duration::ZERO;
            run.poll(cx)
        }),
        Poll::Ready(Err(ActionError::Blocked(reason)))
            if reason.as_ref() == "partner handoff quest item is missing"
    ));
}

#[test]
fn failed_trade_offer_mismatch_declines_and_waits_for_closed_screen() {
    if !api::test_support::game_data_available() {
        return;
    }
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
        InteractReq::CloseModal,
        "mismatched offer cleanup must send the packet the engine honours"
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
    if !api::test_support::game_data_available() {
        return;
    }
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
        InteractReq::CloseModal,
        "unexpected confirmation cleanup must send the packet the engine honours"
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
fn timed_out_trade_closes_the_screen_at_once_and_preserves_failure() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut snapshot, mut ledger, handle, _) = fixture(2);
    let epoch = Instant::now();
    assert!(poll(&snapshot, &mut ledger, &handle, 2, epoch).is_pending());
    assert!(poll(&snapshot, &mut ledger, &handle, 3, epoch).is_pending());
    assert_eq!(
        ack(&mut ledger, 3),
        InteractReq::CloseModal,
        "cleanup must send the packet the engine honours, not the inert Decline button"
    );
    snapshot.seed_trade(TradeView::default());
    assert!(poll(&snapshot, &mut ledger, &handle, 4, epoch).is_pending());
    assert!(matches!(
        poll(&snapshot, &mut ledger, &handle, 5, epoch),
        Poll::Ready(Err(ActionError::Blocked(reason)))
            if reason.as_ref() == "partner trade deadline exhausted"
    ));
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
}

#[test]
fn a_screen_that_stays_open_ends_cleanup_after_one_close_with_the_original_failure() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (snapshot, mut ledger, handle, _) = fixture(2);
    let epoch = Instant::now();
    assert!(poll(&snapshot, &mut ledger, &handle, 2, epoch).is_pending());
    assert!(poll(&snapshot, &mut ledger, &handle, 3, epoch).is_pending());
    assert_eq!(ack(&mut ledger, 3), InteractReq::CloseModal);
    // The close grace (5 observed ticks) passes with the screen still open.
    for tick in 4..8 {
        assert!(poll(&snapshot, &mut ledger, &handle, tick, epoch).is_pending());
        assert!(
            ledger.as_ref().unwrap().outbox.is_empty(),
            "one close request, never a repeat"
        );
    }
    assert!(matches!(
        poll(&snapshot, &mut ledger, &handle, 8, epoch),
        Poll::Ready(Err(ActionError::Blocked(reason)))
            if reason.as_ref() == "partner trade deadline exhausted"
    ));
}

#[test]
fn deadline_cleanup_supersedes_the_unacknowledged_accept_with_a_close() {
    if !api::test_support::game_data_available() {
        return;
    }
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
    // authority, not wait on it and leave the confirmation screen open. The run
    // is polled on every observed tick: a skipped tick is a held one.
    assert!(poll(&snapshot, &mut ledger, &handle, 4, epoch).is_pending());
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
        HostEffect::Interaction(InteractReq::CloseModal)
    ));
    // The close is not repeated while it is unacknowledged.
    assert!(poll(&snapshot, &mut ledger, &handle, 6, epoch).is_pending());
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
fn refused_trade_dispatch_still_closes_the_open_screen() {
    if !api::test_support::game_data_available() {
        return;
    }
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
        InteractReq::CloseModal,
        "refused accept cleanup must close the still-open trade"
    );
    snapshot.seed_trade(TradeView::default());
    assert!(poll(&snapshot, &mut ledger, &handle, 5, epoch).is_pending());
    assert!(matches!(
        poll(&snapshot, &mut ledger, &handle, 6, epoch),
        Poll::Ready(Err(ActionError::Blocked(reason)))
            if reason.as_ref() == "partner trade dispatch refused"
    ));
}

/// Drive a trade to the moment after the second accept: our accept click is
/// acknowledged and the client shows the stopped, emptied offer containers
/// under the still-open offer screen.
fn agreed_offers() -> (
    GameSnapshot,
    Ledger,
    ActionHandle<TradeMachine>,
    Arc<Port>,
    Instant,
) {
    let (mut snapshot, mut ledger, handle, port) = fixture(40);
    let epoch = Instant::now();
    assert!(poll(&snapshot, &mut ledger, &handle, 2, epoch).is_pending());
    snapshot.seed_trade(offer(2));
    port.offer.store(true, Ordering::Relaxed);
    assert!(poll(&snapshot, &mut ledger, &handle, 3, epoch).is_pending());
    assert!(matches!(
        ack(&mut ledger, 3),
        InteractReq::IfButton { component_id: 9 }
    ));
    // Neither side changed an offer: the engine stopped both containers.
    snapshot.seed_trade(mutual_accept_offer_screen());
    assert!(poll(&snapshot, &mut ledger, &handle, 4, epoch).is_pending());
    assert!(
        ledger.as_ref().unwrap().outbox.is_empty(),
        "stopped, emptied offers are not a changed offer: no cleanup may start"
    );
    (snapshot, ledger, handle, port, epoch)
}

#[test]
fn mutual_accept_stops_the_offer_containers_and_the_trade_waits_for_the_confirm_screen() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut snapshot, mut ledger, handle, _port, epoch) = agreed_offers();
    assert!(poll(&snapshot, &mut ledger, &handle, 5, epoch).is_pending());
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
    snapshot.seed_trade(confirm_screen());
    assert!(poll(&snapshot, &mut ledger, &handle, 6, epoch).is_pending());
    assert!(
        ledger.as_ref().unwrap().outbox.is_empty(),
        "the confirm screen opens without a cleanup or an early accept"
    );
}

#[test]
fn an_offer_that_changes_while_its_container_transmits_fails_after_agreement() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut snapshot, mut ledger, handle, port) = fixture(40);
    let epoch = Instant::now();
    assert!(poll(&snapshot, &mut ledger, &handle, 2, epoch).is_pending());
    snapshot.seed_trade(offer(2));
    port.offer.store(true, Ordering::Relaxed);
    assert!(poll(&snapshot, &mut ledger, &handle, 3, epoch).is_pending());
    assert!(matches!(
        ack(&mut ledger, 3),
        InteractReq::IfButton { component_id: 9 }
    ));

    // The partner pulled the item back: the container still transmits and the
    // engine reset the agreement (`trade.rs2:313,317`).
    let mut changed = offer(2);
    changed.their_offer.clear();
    snapshot.seed_trade(changed);
    assert!(poll(&snapshot, &mut ledger, &handle, 4, epoch).is_pending());
    assert_eq!(ack(&mut ledger, 4), InteractReq::CloseModal);
    snapshot.seed_trade(TradeView::default());
    assert!(poll(&snapshot, &mut ledger, &handle, 5, epoch).is_pending());
    assert!(matches!(
        poll(&snapshot, &mut ledger, &handle, 6, epoch),
        Poll::Ready(Err(ActionError::Blocked(reason)))
            if reason.as_ref() == "partner offers changed after agreement"
    ));
}

#[test]
fn the_confirm_screen_must_list_exactly_the_agreed_trade_before_the_final_accept() {
    if !api::test_support::game_data_available() {
        return;
    }
    for fault in 0..3 {
        let (mut snapshot, mut ledger, handle, port, epoch) = agreed_offers();
        port.confirm.store(true, Ordering::Relaxed);
        let mut confirm = confirm_screen();
        let lists = confirm.confirm.as_deref_mut().unwrap();
        match fault {
            0 => lists
                .theirs
                .push(item(99, 1, ItemContainer::TradeTheirOffer)),
            1 => lists.mine[0].count = 3,
            2 => lists.theirs[0].def.noted = true,
            _ => unreachable!(),
        }
        snapshot.seed_trade(confirm);
        // The first poll on the confirm screen ends the offer wait.
        assert!(poll(&snapshot, &mut ledger, &handle, 5, epoch).is_pending());
        assert!(poll(&snapshot, &mut ledger, &handle, 6, epoch).is_pending());
        assert_eq!(
            ack(&mut ledger, 6),
            InteractReq::CloseModal,
            "a confirm list that differs from the agreement must close, never accept (fault {fault})"
        );
        snapshot.seed_trade(TradeView::default());
        assert!(poll(&snapshot, &mut ledger, &handle, 7, epoch).is_pending());
        assert!(matches!(
            poll(&snapshot, &mut ledger, &handle, 8, epoch),
            Poll::Ready(Err(ActionError::Blocked(reason)))
                if reason.as_ref() == "partner confirmation does not match exact transfer"
        ));
    }
}

#[test]
fn a_confirm_list_that_has_not_arrived_waits_instead_of_accepting_or_failing() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut snapshot, mut ledger, handle, port, epoch) = agreed_offers();
    port.confirm.store(true, Ordering::Relaxed);
    let mut confirm = confirm_screen();
    confirm.confirm.as_deref_mut().unwrap().theirs.clear();
    snapshot.seed_trade(confirm);
    // The first poll on the confirm screen ends the offer wait.
    assert!(poll(&snapshot, &mut ledger, &handle, 5, epoch).is_pending());
    assert!(poll(&snapshot, &mut ledger, &handle, 6, epoch).is_pending());
    assert!(
        ledger.as_ref().unwrap().outbox.is_empty(),
        "an incomplete confirm list is neither accepted nor declined"
    );
    snapshot.seed_trade(confirm_screen());
    assert!(poll(&snapshot, &mut ledger, &handle, 7, epoch).is_pending());
    assert!(matches!(
        ack(&mut ledger, 7),
        InteractReq::IfButton { component_id: 12 }
    ));
}

#[test]
fn confirm_lists_emptied_by_the_closing_trade_do_not_fail_but_a_foreign_row_does() {
    if !api::test_support::game_data_available() {
        return;
    }
    for foreign in [false, true] {
        let (mut snapshot, mut ledger, handle, port, epoch) = agreed_offers();
        port.confirm.store(true, Ordering::Relaxed);
        snapshot.seed_trade(confirm_screen());
        // The first poll on the confirm screen ends the offer wait.
        assert!(poll(&snapshot, &mut ledger, &handle, 5, epoch).is_pending());
        assert!(poll(&snapshot, &mut ledger, &handle, 6, epoch).is_pending());
        assert!(matches!(
            ack(&mut ledger, 6),
            InteractReq::IfButton { component_id: 12 }
        ));
        let mut closing = confirm_screen();
        let lists = closing.confirm.as_deref_mut().unwrap();
        if foreign {
            lists
                .theirs
                .push(item(99, 1, ItemContainer::TradeTheirOffer));
        } else {
            // `[if_close,tradeconfirm]` stops the lists (`trade.rs2:279-283`).
            lists.mine.clear();
            lists.theirs.clear();
        }
        snapshot.seed_trade(closing);
        assert!(poll(&snapshot, &mut ledger, &handle, 7, epoch).is_pending());
        if foreign {
            assert_eq!(ack(&mut ledger, 7), InteractReq::CloseModal);
        } else {
            assert!(ledger.as_ref().unwrap().outbox.is_empty());
        }
    }
}

#[test]
fn transfer_requires_both_screens_then_closed_ui_and_fresh_inventory_delta() {
    if !api::test_support::game_data_available() {
        return;
    }
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
    snapshot.seed_trade(mutual_accept_offer_screen());
    assert!(poll(&snapshot, &mut ledger, &handle, 7, epoch).is_pending());
    snapshot.seed_trade(confirm_screen());
    assert!(poll(&snapshot, &mut ledger, &handle, 8, epoch).is_pending());
    assert!(poll(&snapshot, &mut ledger, &handle, 9, epoch).is_pending());
    assert!(
        ledger.as_ref().unwrap().outbox.is_empty(),
        "a local confirmation does not satisfy the mutual barrier"
    );
    port.confirm.store(true, Ordering::Relaxed);
    assert!(poll(&snapshot, &mut ledger, &handle, 10, epoch).is_pending());
    assert!(matches!(
        ack(&mut ledger, 10),
        InteractReq::IfButton { component_id: 12 }
    ));
    snapshot.seed_trade(TradeView::default());
    assert!(poll(&snapshot, &mut ledger, &handle, 11, epoch).is_pending());
    assert!(poll(&snapshot, &mut ledger, &handle, 12, epoch).is_pending());
    assert!(
        poll(&snapshot, &mut ledger, &handle, 13, epoch).is_pending(),
        "closed screens and accepts are not a transfer receipt"
    );
    snapshot.seed_inventory(vec![item(2, 1, ItemContainer::Inventory)], 28);

    with_tick(&snapshot, &mut ledger, 14, |frame| {
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
    match poll(&snapshot, &mut ledger, &handle, 15, epoch) {
        Poll::Ready(Ok(outcome)) => assert_eq!(outcome.evidence.tick, 15),
        _ => panic!("fresh exact inventory deltas must settle"),
    }
}

#[test]
fn trade_waits_count_observed_game_ticks_and_ignore_both_clocks() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (snapshot, mut ledger, handle, _) = fixture(2);
    let epoch = Instant::now();
    assert!(poll(&snapshot, &mut ledger, &handle, 2, epoch).is_pending());
    // An hour on both clocks, but no further game tick: nothing expires.
    with_tick(&snapshot, &mut ledger, 2, |frame| {
        frame.cx.wall_now = epoch + Duration::from_secs(3_600);
        frame.cx.active_now = Duration::from_secs(3_600);
        assert!(frame.actions.poll(&handle, &mut frame.cx).is_pending());
    });
    assert!(
        ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .all(|action| !matches!(
                action.effect,
                HostEffect::Interaction(InteractReq::CloseModal)
            )),
        "no observed tick passed, so the two-tick budget is not spent and nothing closes"
    );
    // One more observed tick spends the budget: the clocks are not consulted.
    assert!(poll(&snapshot, &mut ledger, &handle, 3, epoch).is_pending());
    assert!(ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .any(|action| matches!(
            action.effect,
            HostEffect::Interaction(InteractReq::CloseModal)
        )));
}

/// How the run is held: the frame is ineligible (guardian, readiness, welcome:
/// `ActionContext::eligible`), or the pair broker answers `NotReady`.
#[derive(Clone, Copy, Debug)]
enum Hold {
    Ineligible,
    PairNotReady,
}

/// Poll one observed tick, eligible or not, on frozen clocks.
fn poll_held(
    snapshot: &GameSnapshot,
    ledger: &mut Ledger,
    handle: &ActionHandle<TradeMachine>,
    tick: u64,
    ineligible: bool,
) -> Poll<Result<StepOutcome, ActionError>> {
    with_tick(snapshot, ledger, tick, |frame| {
        frame.cx.wall_now = Instant::now();
        frame.cx.active_now = Duration::ZERO;
        frame.cx.eligible = !ineligible;
        frame.actions.poll(handle, &mut frame.cx)
    })
}

/// Hold the run for `ticks` observed ticks from `from`; returns the next tick.
/// Nothing may be sent and nothing may end while the run is held.
fn hold_run(
    snapshot: &GameSnapshot,
    ledger: &mut Ledger,
    handle: &ActionHandle<TradeMachine>,
    port: &Port,
    (from, ticks): (u64, u64),
    hold: Hold,
) -> u64 {
    port.not_ready
        .store(matches!(hold, Hold::PairNotReady), Ordering::Relaxed);
    for tick in from..from + ticks {
        let polled = poll_held(
            snapshot,
            ledger,
            handle,
            tick,
            matches!(hold, Hold::Ineligible),
        );
        assert!(
            polled.is_pending(),
            "{hold:?} tick {tick} must stay pending"
        );
        assert!(
            ledger.as_ref().unwrap().outbox.is_empty(),
            "{hold:?} tick {tick}: a held run sends nothing"
        );
    }
    port.not_ready.store(false, Ordering::Relaxed);
    from + ticks
}

/// A whole trade that takes 14 eligible observed ticks from `begin` (a 30-tick
/// budget covers it), with a hold of 100 ticks at checkpoint `at`: after the
/// opening poll, after our offer accept, on the confirm screen before our
/// final accept, after our final accept, and with the screens closed.
fn complete_trade_across_a_hold(at: usize, hold: Hold) {
    const HOLD_TICKS: u64 = 100;
    let (mut snapshot, mut ledger, handle, port) = fixture(30);
    let epoch = Instant::now();
    let mut tick = 2;
    let held = |tick: u64, checkpoint: usize, ledger: &mut Ledger, snapshot: &GameSnapshot| {
        if checkpoint == at {
            hold_run(snapshot, ledger, &handle, &port, (tick, HOLD_TICKS), hold)
        } else {
            tick
        }
    };
    assert!(poll(&snapshot, &mut ledger, &handle, tick, epoch).is_pending());
    tick = held(tick + 1, 0, &mut ledger, &snapshot);
    for offered in [0, 1] {
        snapshot.seed_trade(offer(offered));
        assert!(poll(&snapshot, &mut ledger, &handle, tick, epoch).is_pending());
        let request = ack(&mut ledger, tick);
        assert!(
            matches!(request, InteractReq::InvButton { id: 1, .. }),
            "{hold:?} at checkpoint {at}, tick {tick}: {request:?}"
        );
        tick += 1;
    }
    snapshot.seed_trade(offer(2));
    port.offer.store(true, Ordering::Relaxed);
    assert!(poll(&snapshot, &mut ledger, &handle, tick, epoch).is_pending());
    assert!(matches!(
        ack(&mut ledger, tick),
        InteractReq::IfButton { component_id: 9 }
    ));
    tick = held(tick + 1, 1, &mut ledger, &snapshot);
    snapshot.seed_trade(mutual_accept_offer_screen());
    assert!(poll(&snapshot, &mut ledger, &handle, tick, epoch).is_pending());
    snapshot.seed_trade(confirm_screen());
    tick += 1;
    assert!(poll(&snapshot, &mut ledger, &handle, tick, epoch).is_pending());
    tick = held(tick + 1, 2, &mut ledger, &snapshot);
    port.confirm.store(true, Ordering::Relaxed);
    assert!(poll(&snapshot, &mut ledger, &handle, tick, epoch).is_pending());
    assert!(matches!(
        ack(&mut ledger, tick),
        InteractReq::IfButton { component_id: 12 }
    ));
    tick = held(tick + 1, 3, &mut ledger, &snapshot);
    snapshot.seed_trade(TradeView::default());
    assert!(poll(&snapshot, &mut ledger, &handle, tick, epoch).is_pending());
    tick += 1;
    assert!(poll(&snapshot, &mut ledger, &handle, tick, epoch).is_pending());
    tick = held(tick + 1, 4, &mut ledger, &snapshot);
    snapshot.seed_inventory(vec![item(2, 1, ItemContainer::Inventory)], 28);
    match poll(&snapshot, &mut ledger, &handle, tick, epoch) {
        Poll::Ready(Ok(outcome)) => assert_eq!(outcome.evidence.tick, tick),
        _ => panic!("{hold:?} at checkpoint {at}: the trade must complete"),
    }
}

#[test]
fn a_hold_longer_than_every_remaining_budget_spends_no_trade_tick_and_the_trade_completes() {
    if !api::test_support::game_data_available() {
        return;
    }
    for hold in [Hold::Ineligible, Hold::PairNotReady] {
        for at in 0..5 {
            complete_trade_across_a_hold(at, hold);
        }
    }
}

#[test]
fn a_trade_over_its_eligible_tick_budget_still_fails_with_cleanup_after_a_hold() {
    if !api::test_support::game_data_available() {
        return;
    }
    // Begun on tick 1 with a budget of 6 eligible ticks: with no hold the budget
    // is spent when the clock reads 7 and `CLOSE_MODAL` goes out.
    let (mut snapshot, mut ledger, handle, port) = fixture(6);
    let epoch = Instant::now();
    // Both offers are already exact and the partner has not accepted: the machine
    // waits at the barrier and sends nothing until its budget is spent.
    snapshot.seed_trade(offer(2));
    assert!(poll(&snapshot, &mut ledger, &handle, 2, epoch).is_pending());
    // 40 held ticks (3 to 42), then the run is eligible again on tick 43: the
    // clock reads 3 there, and reaches the budget of 7 on tick 47.
    let mut tick = hold_run(
        &snapshot,
        &mut ledger,
        &handle,
        &port,
        (3, 40),
        Hold::Ineligible,
    );
    assert_eq!(tick, 43);
    for _ in 0..4 {
        assert!(poll(&snapshot, &mut ledger, &handle, tick, epoch).is_pending());
        assert!(
            ledger.as_ref().unwrap().outbox.is_empty(),
            "tick {tick} is inside the eligible budget"
        );
        tick += 1;
    }
    // Tick 47 is the fifth eligible tick after the hold: the budget is spent.
    assert!(poll(&snapshot, &mut ledger, &handle, tick, epoch).is_pending());
    assert_eq!(ack(&mut ledger, tick), InteractReq::CloseModal);
    // The close grace is eligible ticks too: a hold inside it spends none of it.
    let close_tick = tick;
    tick = hold_run(
        &snapshot,
        &mut ledger,
        &handle,
        &port,
        (tick + 1, 100),
        Hold::PairNotReady,
    );
    for _ in 0..3 {
        assert!(
            poll(&snapshot, &mut ledger, &handle, tick, epoch).is_pending(),
            "tick {tick}: the screen is still open and the grace is not spent"
        );
        tick += 1;
    }
    snapshot.seed_trade(TradeView::default());
    assert!(poll(&snapshot, &mut ledger, &handle, tick, epoch).is_pending());
    assert!(matches!(
        poll(&snapshot, &mut ledger, &handle, tick + 1, epoch),
        Poll::Ready(Err(ActionError::Blocked(reason)))
            if reason.as_ref() == "partner trade deadline exhausted"
    ));
    assert!(close_tick < tick);
}

#[test]
fn the_offer_wait_expires_at_its_policy_budget_not_before() {
    if !api::test_support::game_data_available() {
        return;
    }
    use crate::trade_screen::TRADE_OFFER_WAIT_TICKS;
    let (mut snapshot, mut ledger, handle, port) = fixture(40);
    let epoch = Instant::now();
    assert!(poll(&snapshot, &mut ledger, &handle, 2, epoch).is_pending());
    snapshot.seed_trade(offer(2));
    port.offer.store(true, Ordering::Relaxed);
    assert!(poll(&snapshot, &mut ledger, &handle, 3, epoch).is_pending());
    assert!(matches!(
        ack(&mut ledger, 3),
        InteractReq::IfButton { component_id: 9 }
    ));
    // The partner never accepts: the screen stays on the offers.
    for tick in 4..3 + TRADE_OFFER_WAIT_TICKS {
        assert!(poll(&snapshot, &mut ledger, &handle, tick, epoch).is_pending());
        assert!(
            ledger.as_ref().unwrap().outbox.is_empty(),
            "tick {tick} is inside the wait"
        );
    }
    let expiry = 3 + TRADE_OFFER_WAIT_TICKS;
    assert!(poll(&snapshot, &mut ledger, &handle, expiry, epoch).is_pending());
    assert_eq!(ack(&mut ledger, expiry), InteractReq::CloseModal);
    snapshot.seed_trade(TradeView::default());
    assert!(poll(&snapshot, &mut ledger, &handle, expiry + 1, epoch).is_pending());
    assert!(matches!(
        poll(&snapshot, &mut ledger, &handle, expiry + 2, epoch),
        Poll::Ready(Err(ActionError::Blocked(reason)))
            if reason.as_ref() == "partner offer did not reach confirmation"
    ));
}

#[test]
fn the_confirm_wait_expires_at_its_policy_budget_not_before() {
    if !api::test_support::game_data_available() {
        return;
    }
    use crate::trade_screen::TRADE_CONFIRM_WAIT_TICKS;
    let (mut snapshot, mut ledger, handle, port, epoch) = agreed_offers();
    port.confirm.store(true, Ordering::Relaxed);
    snapshot.seed_trade(confirm_screen());
    assert!(poll(&snapshot, &mut ledger, &handle, 5, epoch).is_pending());
    assert!(poll(&snapshot, &mut ledger, &handle, 6, epoch).is_pending());
    assert!(matches!(
        ack(&mut ledger, 6),
        InteractReq::IfButton { component_id: 12 }
    ));
    // The partner never accepts: the screen stays on the confirmation.
    for tick in 7..6 + TRADE_CONFIRM_WAIT_TICKS {
        assert!(poll(&snapshot, &mut ledger, &handle, tick, epoch).is_pending());
        assert!(
            ledger.as_ref().unwrap().outbox.is_empty(),
            "tick {tick} is inside the wait"
        );
    }
    let expiry = 6 + TRADE_CONFIRM_WAIT_TICKS;
    assert!(poll(&snapshot, &mut ledger, &handle, expiry, epoch).is_pending());
    assert_eq!(ack(&mut ledger, expiry), InteractReq::CloseModal);
    snapshot.seed_trade(TradeView::default());
    assert!(poll(&snapshot, &mut ledger, &handle, expiry + 1, epoch).is_pending());
    assert!(matches!(
        poll(&snapshot, &mut ledger, &handle, expiry + 2, epoch),
        Poll::Ready(Err(ActionError::Blocked(reason)))
            if reason.as_ref() == "partner confirmation did not settle"
    ));
}

#[test]
fn counterpart_extra_noted_or_excessive_offer_blocks_before_accept() {
    if !api::test_support::game_data_available() {
        return;
    }
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
            InteractReq::CloseModal,
            "an invalid trade must close, never accept"
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
    if !api::test_support::game_data_available() {
        return;
    }
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
                InteractReq::CloseModal,
                "a locally expired trade must close, never accept"
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
fn gang_read_waits_for_foreign_main_and_keeps_identity_failures_terminal() {
    if !api::test_support::game_data_available() {
        return;
    }
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let facts =
        Arc::new(api::quest_facts::QuestCatalog::from_identity(data.quest_identity()).unwrap());
    static PORT: Port = Port {
        offer: AtomicBool::new(false),
        confirm: AtomicBool::new(false),
        cancelled: AtomicBool::new(false),
        not_ready: AtomicBool::new(false),
        self_ref: std::sync::Mutex::new(None),
        action_revoker: std::sync::Mutex::new(None),
    };
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_main_modal(777, vec![]);
    snapshot.seed_main_modal_texts(vec!["Trade confirmation".into()]);
    let mut reader = crate::quester::gang::GangRead::default();
    let mut ledger = None;

    for (index, active_ms) in [0, 7_999, 8_000, 15_999, 16_000].into_iter().enumerate() {
        let result = with_tick(&snapshot, &mut ledger, index as u64 + 1, |tick| {
            tick.cx.active_now = Duration::from_millis(active_ms);
            tick.pairs = Some(&PORT);
            reader.poll(tick, &facts)
        });
        if active_ms < 16_000 {
            assert!(result.is_pending(), "a foreign MAIN must wait before begin");
        } else {
            assert!(matches!(
                result,
                Poll::Ready(Err(ActionError::Blocked(reason)))
                    if reason.as_ref() == "owned Arrav admission journal could not be read"
            ));
        }
        assert!(ledger
            .as_ref()
            .is_none_or(|ledger| ledger.outbox.is_empty()));
        assert_eq!(snapshot.modals().main, 777, "never close an unowned window");
    }

    let mut no_modal = GameSnapshot::new();
    no_modal.seed_ingame(2);
    let mut unknown_reader = crate::quester::gang::GangRead::default();
    let empty_facts = Arc::new(api::quest_facts::QuestCatalog::empty());
    let unknown = with_tick(&no_modal, &mut ledger, 6, |tick| {
        tick.pairs = Some(&PORT);
        unknown_reader.poll(tick, &empty_facts)
    });
    assert!(matches!(
        unknown,
        Poll::Ready(Err(ActionError::Failed(reason))) if reason.as_ref() == "unknown journal quest"
    ));

    let mut wrong_title = GameSnapshot::new();
    wrong_title.seed_ingame(2);
    wrong_title.seed_main_modal(
        crate::quest_journal::ROOT_289,
        vec![crate::quest_journal::test_widget(
            8144,
            "@dre@Romeo & Juliet",
        )],
    );
    let mut wrong_title_reader = crate::quester::gang::GangRead::default();
    let wrong_title_result = with_tick(&wrong_title, &mut ledger, 7, |tick| {
        tick.pairs = Some(&PORT);
        wrong_title_reader.poll(tick, &facts)
    });
    assert!(matches!(
        wrong_title_result,
        Poll::Ready(Err(ActionError::Failed(reason)))
            if reason.as_ref() == "journal blocked by quest journal 'Romeo & Juliet'"
    ));
}
#[test]
fn recovery_item_fact_compiles_only_bounded_arrav_items_and_an_authored_pair_role() {
    if !api::test_support::game_data_available() {
        return;
    }
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
