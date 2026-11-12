//! Content-timed shop purchasing with an ordered same-keeper page barrier.
//!
//! A server `IF_CLOSE` is not a purchase receipt. A roundtrip's Trade follows
//! the outstanding button on the same connection; its fresh page includes
//! the final pack state before another button is admitted.

use crate::interact::InteractReq;
use crate::native::walk::Walk;
use crate::native::{ActionContext, ActionError, NativeMachine, WalkRetryState};
use crate::quester::families::reach::walk_request;
use crate::reach_route;
use crate::shop_open::{self, Decision, Facts, Keeper, OpenCore, Session};
use api::selected::RunKey;
use std::sync::Arc;
use std::task::Poll;

/// Patience before requesting the ordered page roundtrip, in observed ticks.
///
/// The acquisition design's §4.8 E1–E7 establish input/output ordering:
/// Engine input is ordered and each admitted
/// buy runs synchronously (NetworkPlayer.ts:69–152, InvButtonHandler.ts:51–55,
/// Player.ts:2173–2185); pack output precedes modal output (World.ts:1110–1123).
/// Those facts establish settlement ordering, not a two-tick transit bound.
///
/// PROBE-BUY-REPLY on Engine L/R289 (2026-10-29) measured 300 held replies
/// and 100 counted-close replies at one edge each, plus 100 fresh refusals
/// at zero or one edge. K = 1 confirms two ticks under the K <= 2 rule.
/// Receipt run: GATHERER-SHOP/buy-reply/runs/20261029T163806Z-4016658-gsh.
///
/// This count can request a roundtrip, but can never settle a purchase.
pub const BUY_REPLY_TICKS: u64 = 2;

/// The Buy button batch for `remaining` wanted units against `stock` on the
/// page: the shop's Buy 10 and Buy 5 buttons while both still fit, else Buy 1.
pub(crate) fn buy_chunk(remaining: f64, stock: i32) -> i32 {
    if remaining >= 10.0 && stock >= 10 {
        10
    } else if remaining >= 5.0 && stock >= 5 {
        5
    } else {
        1
    }
}

/// The three refusal lines emitted by `shop.rs2:69–90`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PurchaseRefusal {
    Coins,
    Stock,
    Space,
}

impl PurchaseRefusal {
    fn from_line(text: &str) -> Option<Self> {
        match text {
            "You don't have enough coins." => Some(Self::Coins),
            "The shop has run out of stock." => Some(Self::Stock),
            "You don't have enough inventory space." => Some(Self::Space),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct RefusalSeen {
    kind: PurchaseRefusal,
    tick: u64,
    held: i32,
}

#[derive(Debug, Clone, Copy)]
struct RefusalLine {
    sequence: i32,
    kind: PurchaseRefusal,
}

#[derive(Debug, Clone, Copy)]
struct PurchaseObservation {
    run: RunKey,
    tick: u64,
    held: i32,
    refusal: Option<RefusalLine>,
}

/// Only the new page of an ordered, non-overlapping Trade is a roundtrip
/// witness. Raw server-close counts deliberately have no representation here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettlementWitness {
    Content,
    FreshPage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ResolvedDispatch {
    landed: u32,
    refusal: Option<PurchaseRefusal>,
    unanswered: bool,
}

/// One button. Removing it after resolution is the caller's accounting edge;
/// `resolved` also prevents repeated evidence polls from returning it twice.
struct Dispatch {
    run: RunKey,
    tick: u64,
    held_before: i32,
    chunk: i32,
    chat_seq: i32,
    closes_at: u64,
    stock_before: i32,
    refusal: Option<RefusalSeen>,
    resolved: bool,
}

impl Dispatch {
    fn observe(
        &mut self,
        observation: PurchaseObservation,
        witness: SettlementWitness,
    ) -> Result<Option<ResolvedDispatch>, crate::native::ActionError> {
        if observation.run != self.run {
            return Err(crate::native::ActionError::Stale);
        }
        if self.resolved {
            return Ok(None);
        }
        debug_assert!(self.chunk > 0 && self.chunk <= self.stock_before);
        let landed = observation.held.saturating_sub(self.held_before).max(0);
        // A complete chunk cannot produce a later refusal for this button:
        // buy_item returns only after its synchronous loop (shop.rs2:66–97).
        if landed >= self.chunk {
            return Ok(Some(self.resolve(landed, None, false)));
        }

        let line_was_known = self.refusal.is_some();
        if let Some(line) = observation
            .refusal
            .filter(|line| line.sequence > self.chat_seq)
        {
            self.chat_seq = line.sequence;
            if self.refusal.is_none() {
                self.refusal = Some(RefusalSeen {
                    kind: line.kind,
                    tick: observation.tick,
                    held: observation.held,
                });
            }
        }
        let content_final = self.refusal.is_some_and(|line| {
            // A partial update already present when the line first appeared
            // is not new transfer evidence on a repeated snapshot. Two later
            // PLAYER_INFO edges also put the cycle's pack flush behind us.
            line_was_known
                && (observation.held > line.held || observation.tick.saturating_sub(line.tick) >= 2)
        });
        if content_final || witness == SettlementWitness::FreshPage {
            let refusal = self.refusal.map(|line| line.kind);
            return Ok(Some(self.resolve(
                landed,
                refusal,
                landed == 0 && refusal.is_none(),
            )));
        }
        Ok(None)
    }

    fn resolve(
        &mut self,
        landed: i32,
        refusal: Option<PurchaseRefusal>,
        unanswered: bool,
    ) -> ResolvedDispatch {
        self.resolved = true;
        ResolvedDispatch {
            landed: u32::try_from(landed).unwrap_or(0),
            refusal,
            unanswered,
        }
    }

    fn roundtrip_due(&self, tick: u64, server_closes: u64, open: bool) -> bool {
        !self.resolved
            && (!open
                || server_closes > self.closes_at
                || (self.refusal.is_none() && tick.saturating_sub(self.tick) >= BUY_REPLY_TICKS))
    }
}

/// The last close is allowed only after every dispatch has been reconciled.
/// It uses the packet counter as a close observation, never as buy evidence.
struct CloseWait {
    closes_at: u64,
    since: u64,
    second_count: bool,
    sent: bool,
}

impl CloseWait {
    fn new(closes_at: u64, tick: u64) -> Self {
        Self {
            closes_at,
            since: tick,
            second_count: false,
            sent: false,
        }
    }

    fn observe(
        &mut self,
        server_closes: u64,
        tick: u64,
    ) -> Result<bool, crate::native::ActionError> {
        if server_closes > self.closes_at {
            return Ok(true);
        }
        if tick.saturating_sub(self.since) < BUY_REPLY_TICKS {
            return Ok(false);
        }
        if !self.second_count {
            self.second_count = true;
            self.since = tick;
            return Ok(false);
        }
        Err(crate::native::ActionError::Failed(std::sync::Arc::from(
            "shop did not close",
        )))
    }
}

#[derive(Clone, Copy)]
struct OpeningTrade {
    tick: u64,
    chat_mark: i32,
}

impl OpeningTrade {
    fn rejected(&self, facts: &Facts) -> bool {
        !facts.usable
            && facts.tick.is_some_and(|tick| tick > self.tick)
            && facts.cant_reach_seq > self.chat_mark
    }

    fn page_ready(&self, facts: &Facts) -> bool {
        facts.tick.is_some_and(|tick| tick > self.tick) && facts.usable && !facts.trade_taken
    }
}

/// One keeper type and one desired backpack count.
#[derive(Debug, Clone)]
pub struct BuyRequest {
    pub npc_id: i32,
    pub npc_name: Arc<str>,
    pub item_id: i32,
    pub item_name: Arc<str>,
    /// Held target, not the number of button presses or additional units.
    pub qty: i32,
}

/// Observed units only. A refusal applies to the still-missing held target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuyReceipt {
    pub held: i32,
    pub bought: u32,
    pub refusal: Option<PurchaseRefusal>,
    /// Latest observed stock on the owned page, for the caller's live quote.
    pub stock: i32,
    /// An increment observed on one owned page, never its initial stock post.
    pub restock_at: Option<u64>,
}

#[derive(Clone, Copy)]
enum ClosePurpose {
    Initial,
    Final,
}

enum Phase {
    Close(ClosePurpose, CloseWait),
    Open,
    Walk(Box<Walk>),
    /// A failed approach eligible for the shared bounded next-tick retry,
    /// replanned from the keeper's current observed tile.
    RetryWalk,
    Buy,
    AwaitBatch,
    /// Our local CloseModal clear must be observed before the new Trade.
    RoundtripClose(CloseWait),
}

#[derive(Clone, Copy)]
enum Finish {
    Complete,
    Refused(PurchaseRefusal),
    Unsettled,
}

/// Why the keeper walk now out was sent. A failed walk names it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ApproachCause {
    /// The core's walk to a keeper that the local route cannot reach.
    Unreachable,
    /// A recovery walk after the server's "I can't reach that!".
    Refusal,
}

pub struct BuyMachine {
    request: BuyRequest,
    run: RunKey,
    cancelled: bool,
    phase: Phase,
    open: OpenCore,
    walk_retry: WalkRetryState,
    /// The last keeper-walk receipt's own reason, kept across the shared
    /// retry budget so the terminal failure names how the walk failed.
    last_walk_failure: Option<Arc<str>>,
    opening_trade: Option<OpeningTrade>,
    trade_attempted: bool,
    /// Wander replans left for this machine
    /// ([`crate::quester::families::dialogue::npc_approach_replan_budget`]).
    /// As in Dialogue (`dialogue.rs:1376-1383`), only a keeper tile that
    /// changed since the last walk spends one.
    approach_replans_left: u16,
    approach_replans_total: u16,
    /// This open already sent its free recovery walk after a refusal
    /// (`dialogue.rs:885-899`).
    refusal_walked: bool,
    /// The keeper tile that the last walk of this open went to.
    walk_target: Option<api::WorldTile>,
    walk_cause: ApproachCause,
    reach_failure_pending: bool,
    keeper_index: Option<i32>,
    pending: Option<Dispatch>,
    reconciling: bool,
    bought: u32,
    held: i32,
    stock: i32,
    page_closes: u64,
    restock_at: Option<u64>,
    unanswered: u8,
    finish: Option<Finish>,
}

impl NativeMachine for BuyMachine {
    type Args = BuyRequest;
    type Output = BuyReceipt;

    fn begin(request: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        if request.qty < 1 {
            return Err(ActionError::Unavailable(Arc::from(
                "buy quantity must be positive",
            )));
        }
        let snapshot = cx.snapshot();
        let shop = snapshot.shop();
        let closes = shop.as_ref().map_or(0, |shop| shop.value.server_closes);
        let phase = if shop.is_some_and(|shop| shop.value.open) {
            Phase::Close(
                ClosePurpose::Initial,
                CloseWait::new(closes, cx.evidence().tick),
            )
        } else {
            Phase::Open
        };
        let approach_replans_left =
            crate::quester::families::dialogue::npc_approach_replan_budget(cx, request.npc_id);
        Ok(Self {
            request,
            run: cx.run(),
            cancelled: false,
            phase,
            open: OpenCore::default(),
            walk_retry: WalkRetryState::default(),
            last_walk_failure: None,
            opening_trade: None,
            trade_attempted: false,
            approach_replans_left,
            approach_replans_total: approach_replans_left,
            refusal_walked: false,
            walk_target: None,
            walk_cause: ApproachCause::Unreachable,
            reach_failure_pending: false,
            keeper_index: None,
            pending: None,
            reconciling: false,
            bought: 0,
            held: 0,
            stock: 0,
            page_closes: closes,
            restock_at: None,
            unanswered: 0,
            finish: None,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        if cx.run() != self.run {
            self.cancel();
            return Poll::Ready(Err(ActionError::Stale));
        }
        if self.cancelled {
            return Poll::Ready(Err(ActionError::Cancelled));
        }
        let Some(held) = cx.snapshot().stock().held(self.request.item_id) else {
            return Poll::Pending;
        };
        self.held = held;
        if let Some(shop) = cx.snapshot().shop().filter(|shop| shop.value.open) {
            let stock = shop
                .value
                .stock
                .iter()
                .find(|row| row.def.id == self.request.item_id)
                .map_or(0, |row| row.count);
            if matches!(self.phase, Phase::Buy | Phase::AwaitBatch)
                && shop.value.server_closes == self.page_closes
                && stock > self.stock
            {
                self.restock_at = Some(cx.evidence().tick);
            }
            self.stock = stock;
        }
        self.observe_dispatch(cx, SettlementWitness::Content)?;
        loop {
            // Keep phase changes within the tick's shared transition budget.
            if !cx.budget.transition() {
                return Poll::Pending;
            }
            match &mut self.phase {
                Phase::Close(purpose, wait) => {
                    let purpose = *purpose;
                    let snapshot = cx.snapshot();
                    let Some(shop) = snapshot.shop() else {
                        return Poll::Pending;
                    };
                    let (open, closes) = (shop.value.open, shop.value.server_closes);
                    if !wait.sent && open {
                        std::task::ready!(crate::native::defer_budget(
                            cx.emit(InteractReq::CloseModal)
                        ))?;
                        wait.closes_at = closes;
                        wait.since = cx.evidence().tick;
                        wait.sent = true;
                        return Poll::Pending;
                    }
                    let closed = wait.observe(closes, cx.evidence().tick)?;
                    if closed && open {
                        // A close-plus-open frame did not leave the old page
                        // absent. A raw count cannot bypass that condition.
                        wait.observe(wait.closes_at, cx.evidence().tick)?;
                    }
                    if !closed || open {
                        return Poll::Pending;
                    }
                    self.page_closes = closes;
                    match purpose {
                        ClosePurpose::Final => return Poll::Ready(self.receipt()),
                        ClosePurpose::Initial => {
                            self.opening_trade = None;
                            self.phase = Phase::Open;
                        }
                    }
                }
                Phase::Open => {
                    if cx.snapshot().npcs().is_none() {
                        return Poll::Pending;
                    }
                    let (facts, reach_failure) = self.open_facts(cx);
                    self.reach_failure_pending |= reach_failure;
                    let request = &self.request;
                    let keeper_index = self.keeper_index;
                    let mut selected_keeper_index = None;
                    let decision = self.open.tick(&facts, || {
                        let keeper = Self::keeper(request, keeper_index, cx);
                        selected_keeper_index = keeper.as_ref().map(|keeper| keeper.index);
                        keeper
                    });
                    if self.keeper_index.is_none() {
                        self.keeper_index = selected_keeper_index;
                    }
                    let refused = !matches!(decision, Decision::Wait)
                        && std::mem::take(&mut self.reach_failure_pending);
                    let decision = self.adapt(decision, refused, cx);
                    if std::task::ready!(self.act(decision, &facts, cx))? {
                        self.accept_page(cx)?;
                    } else {
                        return Poll::Pending;
                    }
                }
                Phase::RetryWalk => {
                    let (facts, _) = self.open_facts(cx);
                    if self
                        .opening_trade
                        .is_some_and(|trade| trade.page_ready(&facts))
                    {
                        self.accept_page(cx)?;
                        continue;
                    }
                    let tick = cx.evidence().tick;
                    if self.walk_retry.waiting_for_next_tick(tick) {
                        return Poll::Pending;
                    }
                    if cx.snapshot().npcs().is_none() {
                        return Poll::Pending;
                    }
                    let Some(keeper) = Self::keeper(&self.request, self.keeper_index, cx) else {
                        let reason = self.last_walk_failure.as_ref().map_or_else(
                            || Arc::from("the keeper disappeared during the approach"),
                            |detail| {
                                Arc::from(format!(
                                    "the keeper disappeared during the approach; last walk failure: {detail}"
                                ))
                            },
                        );
                        return Poll::Ready(Err(self.open_failure(reason)));
                    };
                    if !Self::keeper_needs_approach(keeper.route) {
                        if facts.trade_taken {
                            return Poll::Pending;
                        }
                        self.walk_retry.reset();
                        let opened =
                            std::task::ready!(self.act(Decision::Trade(keeper), &facts, cx,))?;
                        self.phase = Phase::Open;
                        if opened {
                            self.accept_page(cx)?;
                        }
                        continue;
                    }
                    let target = keeper.tile;
                    let walk = match crate::native::defer_budget(Walk::begin(
                        walk_request(target, 1, None, cx.evidence()),
                        cx,
                    )) {
                        Poll::Pending => {
                            self.walk_retry.defer_until_next_tick(tick);
                            return Poll::Pending;
                        }
                        Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                        Poll::Ready(Ok(walk)) => walk,
                    };
                    self.walk_retry.action_submitted();
                    self.walk_target = Some(target);
                    self.phase = Phase::Walk(Box::new(walk));
                    return Poll::Pending;
                }
                Phase::Walk(walk) => {
                    let receipt = match walk.poll(cx) {
                        Poll::Pending => return Poll::Pending,
                        Poll::Ready(Ok(receipt)) => Some(receipt),
                        Poll::Ready(Err(_)) => None,
                    };
                    let (facts, _) = self.open_facts(cx);
                    if self
                        .opening_trade
                        .is_some_and(|trade| trade.page_ready(&facts))
                    {
                        self.accept_page(cx)?;
                        continue;
                    }
                    if let Some(receipt) = receipt.as_ref() {
                        self.last_walk_failure =
                            if matches!(&receipt.end, crate::native::WalkEnd::Arrived) {
                                None
                            } else {
                                Some(receipt.failure_detail_or_fallback())
                            };
                    }
                    if receipt.as_ref().is_some_and(|receipt| {
                        self.walk_retry
                            .observe_approach_failure(receipt, cx.evidence().tick)
                    }) {
                        self.phase = Phase::RetryWalk;
                        return Poll::Pending;
                    }
                    let arrived = receipt.is_some_and(|receipt| receipt.into_arrival().is_ok());
                    if arrived {
                        self.walk_retry.reset();
                    }
                    let request = &self.request;
                    let keeper_index = self.keeper_index;
                    // A failed walk is terminal only once the shared budget is
                    // spent (or the failure is not retryable); its decision
                    // keeps the last receipt's own reason.
                    let decision = if arrived {
                        let decision = self
                            .open
                            .walked(true, || Self::keeper(request, keeper_index, cx));
                        self.adapt(decision, false, cx)
                    } else {
                        self.walk_failure()
                    };
                    self.phase = Phase::Open;
                    if std::task::ready!(self.act(decision, &facts, cx))? {
                        self.accept_page(cx)?;
                    } else {
                        return Poll::Pending;
                    }
                }
                Phase::Buy => {
                    if self.finish.is_some() || self.held >= self.request.qty {
                        self.finish.get_or_insert(Finish::Complete);
                        self.start_final_close(cx);
                        continue;
                    }
                    let snapshot = cx.snapshot();
                    let Some(shop) = snapshot.shop() else {
                        return Poll::Pending;
                    };
                    if !shop.value.open || shop.value.server_closes > self.page_closes {
                        self.start_roundtrip(cx);
                        continue;
                    }
                    let row = shop
                        .value
                        .stock
                        .iter()
                        .find(|row| row.def.id == self.request.item_id);
                    let Some(row) = row.filter(|row| row.count > 0) else {
                        self.stock = 0;
                        self.finish = Some(Finish::Refused(PurchaseRefusal::Stock));
                        self.start_final_close(cx);
                        continue;
                    };
                    self.stock = row.count;
                    let chunk = buy_chunk(f64::from(self.request.qty - self.held), row.count);
                    let (id, slot, component) = (row.def.id, row.slot, row.component_id);
                    let closes_at = shop.value.server_closes;
                    let stock_before = row.count;
                    let chat_seq = chat_watermark(cx);
                    std::task::ready!(crate::native::defer_budget(cx.emit(
                        InteractReq::ShopButton {
                            kind: "buy".into(),
                            name: self.request.item_name.to_string(),
                            id,
                            slot,
                            component,
                            chunk,
                        },
                    )))?;
                    self.pending = Some(Dispatch {
                        run: self.run,
                        tick: cx.evidence().tick,
                        held_before: self.held,
                        chunk,
                        chat_seq,
                        closes_at,
                        stock_before,
                        refusal: None,
                        resolved: false,
                    });
                    self.phase = Phase::AwaitBatch;
                    return Poll::Pending;
                }
                Phase::AwaitBatch => {
                    let Some(dispatch) = &self.pending else {
                        self.phase = Phase::Buy;
                        continue;
                    };
                    let snapshot = cx.snapshot();
                    let Some(shop) = snapshot.shop() else {
                        return Poll::Pending;
                    };
                    if dispatch.roundtrip_due(
                        cx.evidence().tick,
                        shop.value.server_closes,
                        shop.value.open,
                    ) {
                        self.start_roundtrip(cx);
                        continue;
                    }
                    return Poll::Pending;
                }
                Phase::RoundtripClose(wait) => {
                    let snapshot = cx.snapshot();
                    let Some(modals) = snapshot.modals() else {
                        return Poll::Pending;
                    };
                    let local_absent = [
                        modals.value.main,
                        modals.value.side,
                        modals.value.chat,
                        modals.value.tutorial,
                    ]
                    .into_iter()
                    .all(|root| root == -1);
                    if local_absent {
                        // Trade itself calls clearPendingAction -> closeModal
                        // on the server. A foreign replacement modal must first
                        // take our CloseModal path, just like the shop page.
                        self.open = OpenCore::default();
                        self.trade_attempted = false;
                        self.refusal_walked = false;
                        self.walk_target = None;
                        self.opening_trade = None;
                        self.phase = Phase::Open;
                        continue;
                    }
                    if !wait.sent {
                        std::task::ready!(crate::native::defer_budget(
                            cx.emit(InteractReq::CloseModal)
                        ))?;
                        wait.since = cx.evidence().tick;
                        wait.sent = true;
                        return Poll::Pending;
                    }
                    wait.observe(wait.closes_at, cx.evidence().tick)?;
                    return Poll::Pending;
                }
            }
        }
    }

    fn cancel(&mut self) {
        self.cancelled = true;
        if let Phase::Walk(walk) = &mut self.phase {
            walk.cancel();
        }
        self.pending = None;
    }
}

impl BuyMachine {
    fn keeper_needs_approach(route: shop_open::KeeperRoute) -> bool {
        match route {
            shop_open::KeeperRoute::Steps(steps) => steps > 0,
            shop_open::KeeperRoute::Unreachable => true,
            shop_open::KeeperRoute::Unknown(distance) => distance > 1,
        }
    }

    /// Apply the engine's reach rule to one core decision. A Trade sent from
    /// a distance makes the server walk the player and follow the keeper;
    /// only "I can't reach that!" shows that the keeper is unreachable
    /// (`client.rs:4244-4282`, `OpNpcHandler.ts:46-49`, `Player.ts:423`,
    /// `Player.ts:1255-1302`). `refused` is a fresh refusal of this open's
    /// Trade. Spent Trades and a missing keeper stay terminal.
    ///
    /// One open then sends at most [`shop_open::OPEN_ATTEMPTS`] Trades and
    /// at most three walk episodes: the core's walk to a keeper that is
    /// unreachable before the first Trade, the free recovery walk after the
    /// first refusal, and one wander replan after the second refusal, only
    /// when the keeper moved. The third refusal spends the last Trade, so
    /// no walk follows it. Each episode is at most
    /// [`crate::native::QUEST_WALK_MAX_ATTEMPTS`] walk requests.
    fn adapt(&mut self, decision: Decision, refused: bool, cx: &ActionContext<'_>) -> Decision {
        let trades_left = self.open.trades_left() > 0;
        let decision = match decision {
            Decision::Trade(_) | Decision::Walk(_) | Decision::Failed(_)
                if refused && trades_left =>
            {
                match Self::keeper(&self.request, self.keeper_index, cx) {
                    Some(keeper) if Self::keeper_needs_approach(keeper.route) => {
                        return self.recover_from_refusal(&keeper);
                    }
                    _ => decision,
                }
            }
            // A timeout is not proof that the server cannot path the player,
            // and neither is a local route that is still closed after a walk.
            // Send Trade to the keeper's current index instead of walking.
            Decision::Walk(_) | Decision::Failed(_) if self.trade_attempted && trades_left => {
                Self::keeper(&self.request, self.keeper_index, cx).map_or(decision, Decision::Trade)
            }
            decision => decision,
        };
        if matches!(decision, Decision::Walk(_)) {
            self.walk_cause = ApproachCause::Unreachable;
        }
        decision
    }

    /// The server refused this open's Trade with "I can't reach that!" and
    /// the keeper still needs an approach. As in Dialogue, the first refusal
    /// gets one recovery walk (`dialogue.rs:885-899`). A later refusal gets
    /// a walk only when the keeper's observed tile changed since the last
    /// walk, and that walk spends one wander replan
    /// (`dialogue.rs:1376-1383`). A refusal at an unchanged tile is terminal.
    fn recover_from_refusal(&mut self, keeper: &Keeper) -> Decision {
        let tile = keeper.tile;
        if !self.refusal_walked {
            self.refusal_walked = true;
            self.walk_cause = ApproachCause::Refusal;
            return Decision::Walk(tile);
        }
        match self.walk_target {
            Some(last) if last != tile => {
                if self.approach_replans_left == 0 {
                    return Decision::Failed(Arc::from(format!(
                        "the server refused the Trade again with \"I can't reach that!\" after the keeper moved from ({}, {}, level {}) to ({}, {}, level {}), route {:?}; no wander replan is left (the keeper's wander range allows {} in all)",
                        last.x,
                        last.z,
                        last.level,
                        tile.x,
                        tile.z,
                        tile.level,
                        keeper.route,
                        self.approach_replans_total,
                    )));
                }
                self.approach_replans_left -= 1;
                self.walk_cause = ApproachCause::Refusal;
                Decision::Walk(tile)
            }
            _ => Decision::Failed(Arc::from(format!(
                "the server refused the Trade again with \"I can't reach that!\" after the recovery walk; the keeper did not move from ({}, {}, level {}), route {:?}",
                tile.x, tile.z, tile.level, keeper.route,
            ))),
        }
    }

    /// The keeper walk failed and its bounded retries are spent. The text
    /// says why the walk went out and keeps the last receipt's reason.
    fn walk_failure(&self) -> Decision {
        match self.walk_cause {
            ApproachCause::Unreachable => OpenCore::walk_failed(self.last_walk_failure.as_deref()),
            ApproachCause::Refusal => {
                let target = self.walk_target.map_or_else(
                    || "an unobserved tile".to_owned(),
                    |tile| format!("({}, {}, level {})", tile.x, tile.z, tile.level),
                );
                Decision::Failed(Arc::from(format!(
                    "the recovery walk to the keeper at {target} after \"I can't reach that!\" failed; last walk failure: {}",
                    self.last_walk_failure.as_deref().unwrap_or("none")
                )))
            }
        }
    }

    fn open_failure(&self, reason: Arc<str>) -> ActionError {
        ActionError::Failed(Arc::from(format!(
            "shop did not open. Last detail: {reason}"
        )))
    }

    fn observe_dispatch(
        &mut self,
        cx: &ActionContext<'_>,
        witness: SettlementWitness,
    ) -> Result<(), ActionError> {
        let Some(dispatch) = &mut self.pending else {
            return Ok(());
        };
        let refusal = cx
            .snapshot()
            .chat_lines(dispatch.chat_seq)
            .and_then(|lines| {
                lines
                    .value
                    .iter()
                    .filter(|line| line.username.is_none() && line.type_ == 0)
                    .filter_map(|line| {
                        PurchaseRefusal::from_line(&line.text).map(|kind| RefusalLine {
                            sequence: line.sequence,
                            kind,
                        })
                    })
                    .min_by_key(|line| line.sequence)
            });
        let resolved = dispatch.observe(
            PurchaseObservation {
                run: cx.run(),
                tick: cx.evidence().tick,
                held: self.held,
                refusal,
            },
            witness,
        )?;
        if let Some(resolved) = resolved {
            self.pending = None;
            self.bought = self.bought.checked_add(resolved.landed).ok_or_else(|| {
                ActionError::Failed(Arc::from("observed purchase count exceeds u32"))
            })?;
            if resolved.unanswered {
                self.unanswered = self.unanswered.saturating_add(1);
                if self.unanswered >= 2 {
                    self.finish = Some(Finish::Unsettled);
                }
            } else {
                self.unanswered = 0;
            }
            if let Some(refusal) = resolved.refusal {
                self.finish = Some(Finish::Refused(refusal));
            }
        }
        Ok(())
    }

    fn start_roundtrip(&mut self, cx: &ActionContext<'_>) {
        let closes = cx
            .snapshot()
            .shop()
            .map_or(self.page_closes, |shop| shop.value.server_closes);
        self.reconciling = true;
        self.phase = Phase::RoundtripClose(CloseWait::new(closes, cx.evidence().tick));
    }

    fn start_final_close(&mut self, cx: &ActionContext<'_>) {
        let snapshot = cx.snapshot();
        let shop = snapshot.shop();
        let closes = shop
            .as_ref()
            .map_or(self.page_closes, |shop| shop.value.server_closes);
        let baseline = if shop.is_some_and(|shop| shop.value.open) {
            closes
        } else {
            self.page_closes
        };
        self.phase = Phase::Close(
            ClosePurpose::Final,
            CloseWait::new(baseline, cx.evidence().tick),
        );
    }

    fn accept_page(&mut self, cx: &ActionContext<'_>) -> Result<(), ActionError> {
        debug_assert!(self.opening_trade.is_some());
        self.walk_retry.reset();
        self.last_walk_failure = None;
        self.opening_trade = None;
        self.page_closes = cx
            .snapshot()
            .shop()
            .map_or(self.page_closes, |shop| shop.value.server_closes);
        self.stock = cx
            .snapshot()
            .shop()
            .and_then(|shop| {
                shop.value
                    .stock
                    .iter()
                    .find(|row| row.def.id == self.request.item_id)
            })
            .map_or(0, |row| row.count);
        if self.reconciling {
            self.observe_dispatch(cx, SettlementWitness::FreshPage)?;
            self.reconciling = false;
        }
        self.phase = Phase::Buy;
        Ok(())
    }

    fn receipt(&self) -> Result<BuyReceipt, ActionError> {
        debug_assert!(self.pending.is_none());
        if matches!(self.finish, Some(Finish::Unsettled)) {
            return Err(ActionError::Failed(Arc::from(
                "shop purchase did not settle",
            )));
        }
        Ok(BuyReceipt {
            held: self.held,
            bought: self.bought,
            refusal: match self.finish {
                Some(Finish::Refused(refusal)) => Some(refusal),
                _ => None,
            },
            stock: self.stock,
            restock_at: self.restock_at,
        })
    }

    fn open_facts(&mut self, cx: &ActionContext<'_>) -> (Facts, bool) {
        let mut facts = open_facts(cx);
        let reach_failure = self
            .opening_trade
            .is_some_and(|trade| trade.rejected(&facts));
        if reach_failure {
            // A fresh server refusal is the evidence that justifies an approach.
            self.opening_trade = None;
        }
        // Idle's ready path is not page adoption: only this opening Trade's
        // response may make the page usable to the stage.
        facts.usable &= self.opening_trade.is_some();
        (facts, reach_failure)
    }

    fn keeper(
        request: &BuyRequest,
        keeper_index: Option<i32>,
        cx: &ActionContext<'_>,
    ) -> Option<Keeper> {
        let snapshot = cx.snapshot();
        let npcs = snapshot.npcs()?;
        let here = snapshot
            .local_player()
            .map(|player| player.value.player.network);
        let view = reach_route::live(snapshot.reach().map(|reach| reach.value), here);
        let candidates = npcs
            .value
            .iter()
            .filter(|npc| npc.r#type == usize::try_from(request.npc_id).ok())
            .filter(|npc| keeper_index.is_none_or(|index| i32::try_from(npc.index) == Ok(index)))
            .map(|npc| shop_open::Candidate {
                name: npc.name.as_deref().unwrap_or(&request.npc_name),
                action: npc
                    .actions
                    .iter()
                    .flatten()
                    .find(|op| shop_open::is_trade_op(op))
                    .map(String::as_str),
                index: i32::try_from(npc.index).unwrap_or(-1),
                tile: if npc.size >= 1 { npc.network } else { npc.tile },
                size: npc.size,
                distance: npc.distance,
            });
        shop_open::choose_keeper(candidates, view.zip(here))
    }

    fn act(
        &mut self,
        decision: Decision,
        facts: &Facts,
        cx: &mut ActionContext<'_>,
    ) -> Poll<Result<bool, ActionError>> {
        match decision {
            Decision::Wait => Poll::Ready(Ok(false)),
            Decision::Opened => Poll::Ready(Ok(true)),
            Decision::Failed(reason) => Poll::Ready(Err(self.open_failure(reason))),
            Decision::Trade(keeper) => {
                std::task::ready!(crate::native::defer_budget(cx.emit(InteractReq::Npc {
                    name: keeper.name.clone(),
                    action: keeper.action.clone(),
                    index: Some(keeper.index),
                })))?;
                self.opening_trade = None;
                self.trade_attempted = true;
                self.open.traded(facts, &keeper);
                self.opening_trade = facts.tick.map(|tick| OpeningTrade {
                    tick,
                    chat_mark: facts.chat_seq,
                });
                Poll::Ready(Ok(false))
            }
            Decision::Walk(tile) => {
                let walk = Walk::begin(walk_request(tile, 1, None, cx.evidence()), cx)?;
                self.walk_retry.action_submitted();
                self.walk_target = Some(tile);
                self.phase = Phase::Walk(Box::new(walk));
                Poll::Ready(Ok(false))
            }
        }
    }
}
fn chat_watermark(cx: &ActionContext<'_>) -> i32 {
    cx.snapshot().chat_lines(0).map_or(0, |lines| {
        lines
            .value
            .iter()
            .map(|line| line.sequence)
            .max()
            .unwrap_or(0)
    })
}

fn open_facts(cx: &ActionContext<'_>) -> Facts {
    let evidence = cx.evidence();
    let (chat_seq, cant_reach_seq) = cx.snapshot().chat_lines(0).map_or((0, 0), |lines| {
        lines.value.iter().fold((0, 0), |(all, refused), line| {
            let refused_here =
                line.username.is_none() && line.type_ == 0 && shop_open::is_cant_reach(&line.text);
            (
                all.max(line.sequence),
                if refused_here {
                    refused.max(line.sequence)
                } else {
                    refused
                },
            )
        })
    });
    Facts {
        tick: Some(evidence.tick),
        session: Session {
            epoch: evidence.run.run,
            login: evidence.run.session,
        },
        chat_seq,
        cant_reach_seq,
        usable: cx
            .snapshot()
            .shop()
            .is_some_and(|shop| shop.value.open && !shop.value.stock.is_empty()),
        trade_taken: cx.interaction_event_spent(),
    }
}

#[cfg(test)]
#[path = "buy_protocol_tests.rs"]
mod protocol_tests;

#[cfg(test)]
#[path = "buy_ordering_tests.rs"]
mod ordering_tests;
