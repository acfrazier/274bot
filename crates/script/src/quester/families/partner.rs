//! Finite reciprocal barriers and exact quest-item transfers, never generic trading.
use super::super::compile::{CompileContext, CompileError, CompiledPath, PredicateContext, PredicatePlan, StepContext, StepOutcome, StepPlan, StepRun};
use super::super::pair::*;
use crate::native::{ActionContext, ActionError, ActionHandle, NativeActions, NativeMachine};
use crate::native::walk::Walk;
use crate::shim::InteractReq;
use crate::trade_screen::{ScreenDriver, ScreenKind, WaitEnd, TRADE_CONFIRM_WAIT_MS, TRADE_OFFER_WAIT_MS};
use api::quest_progress::EvidenceStamp;
use api::selected::{FactKey, Truth};
use api::snapshot::{ItemView, TradeView};
use api::WorldTile;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct PartnerItemCountArgs {
    /// Unnoted Arrav shield-half or certificate alias.
    obj: String,
    /// Minimum quantity in the current reciprocal role's backpack, from 1 to 28.
    qty: i32,
}

pub(super) fn compile_partner_item_count(
    args: PartnerItemCountArgs,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    let pair = cx.pair.ok_or_else(|| CompileError::code("partner-declaration-required"))?;
    let obj = args.obj.strip_prefix("obj:").unwrap_or(&args.obj);
    if cx.path.0.as_ref() != "blackarmgang"
        || !matches!(obj, "arravshield1" | "arravshield2" | "arravcertificate") {
        return Err(CompileError::code("partner-item-not-a-recovery-item"));
    }
    if !(1..=28).contains(&args.qty) { return Err(CompileError::code("partner-invalid-quantity")); }
    let own = pair.declaration.roles.iter().position(|role| &role.id == pair.role)
        .ok_or_else(|| CompileError::code("partner-invalid-role"))?;
    Ok(Arc::new(PartnerItemCount {
        request: PairItemRequest {
            path: cx.path.clone(), protocol: pair.declaration.protocol.clone(),
            digest: pair.digest, own: pair.declaration.roles[own].clone(),
            peer: pair.declaration.roles[1 - own].clone(), obj: super::resolve_obj(cx, obj)?,
        },
        minimum: args.qty,
    }))
}

struct PartnerItemCount {
    request: PairItemRequest,
    minimum: i32,
}
impl PredicatePlan for PartnerItemCount {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        let Some(port) = cx.pairs else { return Truth::Unknown; };
        match port.partner_item_count(cx.cx.evidence(), &self.request) {
            Ok(count) if count >= self.minimum => Truth::True,
            Ok(_) => Truth::False,
            Err(_) => Truth::Unknown,
        }
    }
}

/// One mirrored phase; both roles author the same rendezvous and opposite items.
#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct Args {
    /// Stable protocol phase identity, distinct from admission.
    phase: String,
    /// Exact unnoted quest items this role gives.
    give: Vec<ItemArg>,
    /// Exact unnoted quest items this role receives.
    take: Vec<ItemArg>,
    /// Verified shared world rendezvous.
    rendezvous: Anchor,
    /// Optional finite local trade budget, in game ticks.
    #[serde(default)]
    timeout_ticks: Option<u16>,
}
#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct ItemArg {
    /// Selected object alias.
    obj: String,
    /// Exact quantity, never all held valuables.
    qty: i32,
}
#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct Anchor {
    /// World x, z and level.
    tile: [i32; 3],
    /// Content placement receipt.
    source: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Item {
    id: i32,
    qty: i32,
}
fn items(args: Vec<ItemArg>, cx: &CompileContext<'_>) -> Result<Arc<[Item]>, CompileError> {
    if args.len() > 4 { return Err(CompileError::code("partner-too-many-items")); }
    let mut out = Vec::with_capacity(args.len());
    for item in args {
        if !(1..=28).contains(&item.qty) { return Err(CompileError::code("partner-invalid-quantity")); }
        let id = super::resolve_obj(cx, item.obj.strip_prefix("obj:").unwrap_or(&item.obj))?;
        if out.iter().any(|row: &Item| row.id == id) { return Err(CompileError::code("partner-duplicate-item")); }
        out.push(Item { id, qty: item.qty });
    }
    out.sort_unstable_by_key(|row| row.id);
    Ok(out.into())
}
pub(super) fn compile(args: Args, cx: &CompileContext<'_>) -> Result<Arc<dyn StepPlan>, CompileError> {
    let pair = cx.pair.ok_or_else(|| CompileError::code("partner-declaration-required"))?;
    if args.phase.is_empty() || args.phase.ends_with(":admission") {
        return Err(CompileError::code("partner-invalid-phase"));
    }
    super::validate_tile(args.rendezvous.tile, &args.rendezvous.source)?;
    let role = pair.declaration.roles.iter().position(|role| &role.id == pair.role)
        .ok_or_else(|| CompileError::code("partner-invalid-role"))?;
    let give = items(args.give, cx)?;
    let take = items(args.take, cx)?;
    if give.is_empty() && take.is_empty() || give.iter().any(|row| take.iter().any(|other| row.id == other.id)) {
        return Err(CompileError::code("partner-unprovable-transfer"));
    }
    let budget = args.timeout_ticks.unwrap_or(100);
    if !(1..=6000).contains(&budget) { return Err(CompileError::code("partner-invalid-budget")); }
    let [x, z, level] = args.rendezvous.tile;
    let tile = WorldTile { x, z, level };
    let phase = FactKey::new(&args.phase);
    let (left_give, left_take) = if role == 0 { (&give, &take) } else { (&take, &give) };
    let mut hash = Sha256::new();
    hash.update(pair.digest);
    hash.update(args.phase.as_bytes());
    for value in [x, z, level, i32::from(budget)] { hash.update(value.to_le_bytes()); }
    for rows in [left_give, left_take] {
        hash.update((rows.len() as u32).to_le_bytes());
        for row in rows.iter() { hash.update(row.id.to_le_bytes()); hash.update(row.qty.to_le_bytes()); }
    }
    let forward: Arc<dyn StepPlan> = Arc::new(TradePlan {
        phase: phase.clone(), tile, give: Arc::clone(&give), take: Arc::clone(&take), budget,
    });
    let reverse: Arc<dyn StepPlan> = Arc::new(TradePlan {
        phase: phase.clone(), tile, give: take, take: give, budget,
    });
    let actions = if role == 0 { [forward, reverse] } else { [reverse, forward] };
    Ok(Arc::new(PartnerPlan {
        role: pair.role.clone(),
        plan: Arc::new(CompiledPairPlan {
            path: cx.path.clone(), protocol: pair.declaration.protocol.clone(), digest: pair.digest,
            phase, roles: pair.declaration.roles.clone(), signature: hash.finalize().into(), actions: Some(actions),
        }),
    }))
}

/// The runner performs this barrier before provisioning or either irreversible join.
pub(crate) fn admission(path: &CompiledPath) -> Result<Arc<dyn StepPlan>, ActionError> {
    let declaration = path.partner.as_ref().ok_or(ActionError::Stale)?;
    let role = path.role.as_ref().ok_or(ActionError::Stale)?.clone();
    let phase = FactKey::new(&format!("{}:admission", declaration.protocol.0));
    let mut hash = Sha256::new();
    hash.update(path.digest);
    hash.update(phase.0.as_bytes());
    Ok(Arc::new(PartnerPlan { role, plan: Arc::new(CompiledPairPlan {
        path: path.id.clone(), protocol: declaration.protocol.clone(), digest: path.digest,
        phase, roles: declaration.roles.clone(), signature: hash.finalize().into(), actions: None,
    }) }))
}
struct PartnerPlan {
    role: FactKey,
    plan: Arc<CompiledPairPlan>,
}
impl StepPlan for PartnerPlan {
    fn begin(&self, cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        let port = cx.tick.pairs.ok_or_else(|| ActionError::Unavailable(Arc::from("partner capability is not installed in this Play")))?;
        let settings = port.settings(cx.tick.cx.run()).map_err(PairError::action)?;
        let partner = settings.partner.ok_or(PairError::MissingPartner).map_err(PairError::action)?;
        let (observed_gang, owned) = port.gang(cx.tick.cx.run()).map_err(PairError::action)?;
        if owned.run != cx.tick.cx.run() { return Err(ActionError::Stale); }
        let token = port.begin(PairRequest {
            caller: cx.tick.cx.run(), partner, caller_role: self.role.clone(), phase: self.plan.phase.clone(),
            plan: Arc::clone(&self.plan), observed_gang, declared_gang: settings.gang, evidence: cx.tick.cx.evidence(),
        }).map_err(PairError::action)?;
        Ok(Box::new(PartnerRun {
            port: port.shared(), token, phase: self.plan.phase.clone(), command: None,
            active: None, reported: false, waiting: true,
        }))
    }
}
struct PartnerRun {
    port: Arc<dyn QuestPairPort>,
    token: PairToken,
    phase: FactKey,
    command: Option<u64>,
    active: Option<Box<dyn StepRun>>,
    reported: bool,
    waiting: bool,
}
impl Drop for PartnerRun {
    fn drop(&mut self) { self.port.cancel(&self.token); }
}
impl StepRun for PartnerRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        match self.port.poll(&self.token, cx.tick.cx.run()) {
            Poll::Pending => { self.waiting = true; return Poll::Pending; }
            Poll::Ready(Err(error)) => {
                self.active = None;
                return Poll::Ready(Err(error.action()));
            }
            Poll::Ready(Ok(PairStep::Done(receipt))) => {
                if receipt.token != self.token || receipt.phase != self.phase { return Poll::Ready(Err(ActionError::Stale)); }
                let side = usize::from(cx.tick.cx.run() == self.token.right);
                return Poll::Ready(Ok(StepOutcome { progress: None, evidence: receipt.evidence[side], receipt: None }));
            }
            Poll::Ready(Ok(PairStep::Waiting { .. })) => { self.waiting = true; return Poll::Pending; }
            Poll::Ready(Ok(PairStep::Act(command))) => {
                self.waiting = false;
                if command.recipient != cx.tick.cx.run() || command.phase != self.phase || command.token != self.token
                    || self.command.is_some_and(|id| id != command.command_id) || self.reported {
                    return Poll::Ready(Err(ActionError::Stale));
                }
                if self.active.is_none() {
                    match command.plan.begin(cx) {
                        Ok(run) => { self.active = Some(run); self.command = Some(command.command_id); }
                        Err(ActionError::Busy | ActionError::Held | ActionError::BudgetExhausted) => return Poll::Pending,
                        Err(error) => {
                            let detail = Arc::from(format!("partner role failed: {error:?}"));
                            let _ = self.port.report(&self.token, RoleReceipt { actor: cx.tick.cx.run(), command_id: command.command_id, outcome: Err(error) });
                            return Poll::Ready(Err(ActionError::Blocked(detail)));
                        }
                    }
                }
            }
        }
        match self.active.as_mut().unwrap().poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(outcome) => {
                let detail = outcome.as_ref().err().map(|error| Arc::from(format!("partner role failed: {error:?}")));
                self.active = None;
                let result = self.port.report(&self.token, RoleReceipt {
                    actor: cx.tick.cx.run(), command_id: self.command.unwrap(), outcome,
                });
                self.reported = true;
                if let Some(detail) = detail { return Poll::Ready(Err(ActionError::Blocked(detail))); }
                match result {
                    Ok(()) => { self.waiting = true; Poll::Pending }
                    Err(error) => Poll::Ready(Err(error.action())),
                }
            }
        }
    }
    fn cancel(&mut self, actions: &mut NativeActions) {
        if let Some(active) = self.active.as_mut() { active.cancel(actions); }
        self.active = None;
        self.port.cancel(&self.token);
    }
    fn waiting_for(&self) -> Option<(&'static str, &Arc<str>)> {
        if self.waiting { Some(("Partner barrier", &self.phase.0)) }
        else { self.active.as_ref().and_then(|run| run.waiting_for()) }
    }
}
struct TradePlan {
    phase: FactKey,
    tile: WorldTile,
    give: Arc<[Item]>,
    take: Arc<[Item]>,
    budget: u16,
}
impl StepPlan for TradePlan {
    fn anchor(&self) -> Option<WorldTile> { Some(self.tile) }
    fn begin(&self, cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        let port = cx.tick.pairs.ok_or(ActionError::Stale)?;
        let token = port.token(cx.tick.cx.run(), &self.phase).map_err(PairError::action)?;
        let partner = port.settings(cx.tick.cx.run()).map_err(PairError::action)?.partner
            .ok_or(PairError::MissingPartner).map_err(PairError::action)?;
        let walk = cx.tick.actions.begin::<Walk>(super::reach::walk_request(self.tile, 1, None, cx.required_after), &mut cx.tick.cx)?;
        port.register_action(&token, cx.tick.cx.run(), walk.revoker()).map_err(PairError::action)?;
        Ok(Box::new(TradeRun {
            walk: Some(walk), trade: None,
            args: Some(TradeArgs { port: port.shared(), token, partner, give: Arc::clone(&self.give), take: Arc::clone(&self.take), budget: self.budget }),
        }))
    }
}
struct TradeRun {
    walk: Option<ActionHandle<Walk>>,
    trade: Option<ActionHandle<TradeMachine>>,
    args: Option<TradeArgs>,
}
impl StepRun for TradeRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        if let Some(walk) = &self.walk {
            match cx.tick.actions.poll(walk, &mut cx.tick.cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(receipt)) => {
                    if let Err(error) = super::walk_step_evidence(receipt) { return Poll::Ready(Err(error)); }
                    self.walk = None;
                }
            }
        }
        if self.trade.is_none() {
            let Some(args) = self.args.take() else { return Poll::Ready(Err(ActionError::Stale)); };
            let port = Arc::clone(&args.port);
            let token = args.token;
            match cx.tick.actions.begin::<TradeMachine>(args, &mut cx.tick.cx) {
                Ok(handle) => {
                    if let Err(error) = port.register_action(&token, cx.tick.cx.run(), handle.revoker()) {
                        return Poll::Ready(Err(error.action()));
                    }
                    self.trade = Some(handle);
                }
                Err(error) => return Poll::Ready(Err(error)),
            }
        }
        cx.tick.actions.poll(self.trade.as_ref().unwrap(), &mut cx.tick.cx)
    }
    fn cancel(&mut self, _actions: &mut NativeActions) { self.walk = None; self.trade = None; self.args = None; }
}
struct TradeArgs {
    port: Arc<dyn QuestPairPort>,
    token: PairToken,
    partner: AccountKey,
    give: Arc<[Item]>,
    take: Arc<[Item]>,
    budget: u16,
}
#[derive(Clone, Copy, Default)]
struct InventoryCheck { id: i32, after: i32 }
enum TradePhase {
    Opening,
    Offering,
    OfferWait { driver: ScreenDriver, deadline: Duration },
    Confirm,
    ConfirmWait { driver: ScreenDriver, deadline: Duration },
    Transfer,
}
struct TradeMachine {
    args: TradeArgs,
    partner_id: u64,
    phase: TradePhase,
    checks: [InventoryCheck; 8],
    check_len: usize,
    before: EvidenceStamp,
    deadline: Duration,
    request: Option<(u64, EvidenceStamp)>,
    requested: bool,
    cancelled: bool,
}
fn count(rows: &[ItemView], id: i32) -> i32 {
    rows.iter().filter(|row| row.def.id == id && !row.def.noted)
        .fold(0i32, |total, row| total.saturating_add(row.count))
}
fn exact(rows: &[ItemView], wanted: &[Item]) -> bool {
    subset(rows, wanted) && wanted.iter().all(|item| count(rows, item.id) == item.qty)
}
fn subset(rows: &[ItemView], wanted: &[Item]) -> bool {
    rows.iter().all(|row| !row.def.noted && row.count > 0 && wanted.iter().any(|item| item.id == row.def.id))
        && wanted.iter().all(|item| count(rows, item.id) <= item.qty)
}
fn failure(message: &'static str) -> ActionError { ActionError::Blocked(Arc::from(message)) }
impl TradeMachine {
    fn counterpart(&self, trade: &TradeView) -> bool {
        trade.partner.as_deref().is_some_and(|name| api::snapshot::player_account_id(name) == Some(self.partner_id))
    }
    fn emit(&mut self, request: InteractReq, cx: &mut ActionContext<'_>) -> Result<(), ActionError> {
        self.request = Some((cx.emit(request)?, cx.evidence()));
        Ok(())
    }
}
impl NativeMachine for TradeMachine {
    type Args = TradeArgs;
    type Output = StepOutcome;
    fn begin(args: TradeArgs, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        let partner_id = api::snapshot::player_account_id(&args.partner.0)
            .ok_or_else(|| failure("partner account has no native player identity"))?;
        let inventory = cx.snapshot().inventory().ok_or(ActionError::Busy)?;
        let mut checks = [InventoryCheck::default(); 8];
        let mut check_len = 0;
        for (rows, giving) in [(&args.give, true), (&args.take, false)] {
            for item in rows.iter() {
                let before = count(inventory.value, item.id);
                if giving && before < item.qty { return Err(failure("partner handoff quest item is missing")); }
                let after = if giving { before - item.qty } else { before.checked_add(item.qty).ok_or(ActionError::Stale)? };
                checks[check_len] = InventoryCheck { id: item.id, after };
                check_len += 1;
            }
        }
        let deadline = cx.active_now() + Duration::from_millis(u64::from(args.budget) * 600);
        Ok(Self { args, partner_id, checks, check_len, deadline, before: cx.evidence(),
            phase: TradePhase::Opening, request: None, requested: false, cancelled: false })
    }
    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<StepOutcome, ActionError>> {
        if self.cancelled { return Poll::Ready(Err(ActionError::Cancelled)); }
        match self.args.port.poll(&self.args.token, cx.run()) {
            Poll::Ready(Err(error)) => return Poll::Ready(Err(error.action())),
            Poll::Ready(Ok(PairStep::Act(_))) => {}
            _ => return Poll::Ready(Err(ActionError::Stale)),
        }
        if cx.active_now() >= self.deadline { return Poll::Ready(Err(failure("partner trade deadline exhausted"))); }
        if let Some((request, before)) = self.request {
            let Some(receipt) = cx.interaction_receipt(request) else { return Poll::Pending; };
            if !receipt.accepted { return Poll::Ready(Err(failure("partner trade dispatch refused"))); }
            if !cx.evidence().meets(before) || cx.evidence() == before { return Poll::Pending; }
            self.request = None;
        }
        let Some(trade) = cx.snapshot().trade() else { return Poll::Pending; };
        let screen = ScreenKind::observed(trade.value.offer_open, trade.value.confirm_open);
        if screen.active() && !self.counterpart(trade.value) { return Poll::Ready(Err(failure("partner trade counterpart changed"))); }
        match &mut self.phase {
            TradePhase::Opening => {
                if screen == ScreenKind::Offer { self.phase = TradePhase::Offering; return Poll::Pending; }
                if screen == ScreenKind::Confirm { return Poll::Ready(Err(failure("unexpected existing confirmation; reread both inventories"))); }
                if !self.requested {
                    let Some(players) = cx.snapshot().players() else { return Poll::Pending; };
                    let Some(here) = cx.snapshot().here() else { return Poll::Pending; };
                    let name = players.value.iter().find_map(|player| {
                        let name = player.actor.name.as_deref()?;
                        (api::snapshot::player_account_id(name) == Some(self.partner_id)
                            && player.actor.tile.level == here.value.level
                            && (player.actor.tile.x - here.value.x).abs().max((player.actor.tile.z - here.value.z).abs()) <= 2)
                            .then_some(name)
                    });
                    let Some(name) = name else { return Poll::Pending; };
                    let request = InteractReq::Player { name: name.to_owned(), action: "Trade with".into() };
                    self.emit(request, cx)?;
                    self.requested = true;
                }
            }
            TradePhase::Offering => {
                if screen != ScreenKind::Offer { return Poll::Ready(Err(failure("partner offer screen closed before agreement"))); }
                if !subset(&trade.value.my_offer, &self.args.give) || !subset(&trade.value.their_offer, &self.args.take) {
                    return Poll::Ready(Err(failure("partner trade contains extra, noted or excessive items")));
                }
                if let Some(item) = self.args.give.iter().find(|item| count(&trade.value.my_offer, item.id) < item.qty) {
                    let row = trade.value.side_pack.iter().find(|row| row.def.id == item.id && !row.def.noted && row.count > 0)
                        .ok_or_else(|| failure("partner trade side quest item is missing"))?;
                    if row.component_id != crate::trade::OFFER_INV || row.slot < 0 { return Poll::Ready(Err(ActionError::Stale)); }
                    self.emit(InteractReq::InvButton { id: row.def.id, slot: row.slot, component: row.component_id, operation: 1, bank_generation: 0 }, cx)?;
                    return Poll::Pending;
                }
                if !exact(&trade.value.their_offer, &self.args.take) { return Poll::Pending; }
                if !self.args.port.trade_ready(&self.args.token, cx.run(), false, trade.stamp).map_err(PairError::action)? {
                    return Poll::Pending;
                }
                if trade.value.accept_component_id < 0 { return Poll::Ready(Err(ActionError::Stale)); }
                self.emit(InteractReq::IfButton { component_id: trade.value.accept_component_id }, cx)?;
                self.phase = TradePhase::OfferWait { driver: ScreenDriver::after_offer(), deadline: cx.active_now() + Duration::from_millis(TRADE_OFFER_WAIT_MS) };
            }
            TradePhase::OfferWait { driver, deadline } => {
                if screen.active() && (!exact(&trade.value.my_offer, &self.args.give) || !exact(&trade.value.their_offer, &self.args.take)) {
                    return Poll::Ready(Err(failure("partner offers changed after agreement")));
                }
                match driver.poll(screen, cx.wall_now(), cx.active_now() >= *deadline) {
                    WaitEnd::Confirm => self.phase = TradePhase::Confirm,
                    WaitEnd::Closed | WaitEnd::TimedOut => return Poll::Ready(Err(failure("partner offer did not reach confirmation"))),
                    WaitEnd::Waiting => {}
                }
            }
            TradePhase::Confirm => {
                if screen != ScreenKind::Confirm || !exact(&trade.value.my_offer, &self.args.give) || !exact(&trade.value.their_offer, &self.args.take) {
                    return Poll::Ready(Err(failure("partner confirmation does not match exact transfer")));
                }
                if !self.args.port.trade_ready(&self.args.token, cx.run(), true, trade.stamp).map_err(PairError::action)? { return Poll::Pending; }
                if trade.value.accept_component_id < 0 { return Poll::Ready(Err(ActionError::Stale)); }
                self.emit(InteractReq::IfButton { component_id: trade.value.accept_component_id }, cx)?;
                self.phase = TradePhase::ConfirmWait { driver: ScreenDriver::after_confirm(), deadline: cx.active_now() + Duration::from_millis(TRADE_CONFIRM_WAIT_MS) };
            }
            TradePhase::ConfirmWait { driver, deadline } => {
                if screen.active() && (!exact(&trade.value.my_offer, &self.args.give) || !exact(&trade.value.their_offer, &self.args.take)) {
                    return Poll::Ready(Err(failure("partner confirmation changed after acceptance")));
                }
                match driver.poll(screen, cx.wall_now(), cx.active_now() >= *deadline) {
                    WaitEnd::Closed => self.phase = TradePhase::Transfer,
                    WaitEnd::TimedOut => return Poll::Ready(Err(failure("partner confirmation did not settle"))),
                    WaitEnd::Confirm | WaitEnd::Waiting => {}
                }
            }
            TradePhase::Transfer => {
                let Some(inventory) = cx.snapshot().inventory() else { return Poll::Pending; };
                if inventory.stamp.meets(self.before) && inventory.stamp != self.before
                    && self.checks[..self.check_len].iter().all(|check| count(inventory.value, check.id) == check.after) {
                    return Poll::Ready(Ok(StepOutcome { progress: None, evidence: inventory.stamp, receipt: None }));
                }
            }
        }
        Poll::Pending
    }
    fn cancel(&mut self) { self.cancelled = true; }
}

#[cfg(test)]
#[path = "partner_tests.rs"]
mod tests;
