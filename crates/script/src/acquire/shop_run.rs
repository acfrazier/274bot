//! One active shop need, with routing outside the content-timed buyer.

use super::buy::{BuyMachine, BuyReceipt, BuyRequest, PurchaseRefusal};
use super::vendor::{self, VendorDecision};
use super::*;
use crate::native::walk::Walk;
use crate::native::{ActionHandle, NativeActions, NativeTick, WalkRequest};
use crate::native_bank::{SelectService, ServiceFacts, ServicePickRequest};
use api::service_pick::{ServiceKind, ShopKeeperFacts};
use api::shop_facts::{self, KeeperFact};
use api::stock::Stock;
use std::fmt::Write;
use std::task::Poll;

#[derive(Clone, Copy)]
enum Phase {
    Select,
    Walk,
    Buy,
}

#[derive(Default)]
enum Active {
    #[default]
    None,
    Select(ActionHandle<SelectService>),
    Walk(ActionHandle<Walk>),
    Buy(ActionHandle<BuyMachine>),
}

pub(super) enum Outcome {
    Complete { bought: u32, short: bool },
    Blocked(AcquireBlock),
}

pub(super) struct ShopRun {
    purchase: Purchase,
    phase: Phase,
    active: Active,
    facts: Option<Arc<ShopKeeperFacts>>,
    eligible: Option<Arc<[u16]>>,
    keeper: Option<KeeperFact<'static>>,
    target: Option<WorldTile>,
    nearest: bool,
    retry_used: bool,
    looked: Looked,
    bought: u32,
}

impl ShopRun {
    pub(super) fn new(
        request: &AcquireRequest,
        purchase: Purchase,
        cx: &crate::native::ActionContext<'_>,
    ) -> Result<Self, ActionError> {
        let data = api::game_data::for_revision(cx.pin().revision)
            .map_err(|error| ActionError::Unavailable(Arc::from(error)))?;
        let snapshot = cx.snapshot();
        let need = &request.needs[usize::from(purchase.need)];
        let looked = plan::looked(&snapshot.stock(), need, purchase.item);
        let mut run = Self {
            purchase,
            phase: Phase::Buy,
            active: Active::None,
            facts: None,
            eligible: None,
            keeper: None,
            target: None,
            nearest: false,
            retry_used: false,
            looked,
            bought: 0,
        };
        match &request.vendor {
            VendorChoice::None => {}
            VendorChoice::Explicit {
                keeper_config,
                anchor,
            } => {
                run.keeper = shop_facts::keepers(&data)
                    .find(|keeper| keeper.keeper_config == keeper_config.as_ref());
                run.target = *anchor;
                if anchor.is_some() {
                    run.phase = Phase::Walk;
                }
            }
            VendorChoice::Nearest => {
                let facts = cx.shop_keepers.ok_or_else(|| {
                    ActionError::Unavailable(Arc::from("bound shop keeper facts unavailable"))
                })?;
                let from = snapshot
                    .here()
                    .ok_or_else(|| {
                        ActionError::Unavailable(Arc::from(
                            "player position unavailable for vendor selection",
                        ))
                    })?
                    .value;
                let members = snapshot.world().is_some_and(|world| world.value.members);
                // Entry gates belong to the complete nav search. No area or
                // keeper-config gate table is maintained by acquisition.
                run.eligible = Some(vendor::eligible_indices(
                    facts,
                    &data,
                    purchase.item,
                    members,
                    from,
                    |_| true,
                ));
                run.facts = Some(Arc::clone(facts));
                run.nearest = true;
                run.phase = Phase::Select;
            }
        }
        Ok(run)
    }

    pub(super) fn cancel(&mut self, actions: &mut NativeActions) {
        match std::mem::take(&mut self.active) {
            Active::None => {}
            Active::Select(handle) => actions.cancel(handle),
            Active::Walk(handle) => actions.cancel(handle),
            Active::Buy(handle) => actions.cancel(handle),
        }
    }

    pub(super) fn poll(
        &mut self,
        request: &AcquireRequest,
        tick: &mut NativeTick<'_>,
        gate: OpenGate,
    ) -> Poll<Result<Outcome, ActionError>> {
        loop {
            match std::mem::take(&mut self.active) {
                Active::None => {}
                Active::Select(handle) => match tick.actions.poll(&handle, &mut tick.cx) {
                    Poll::Pending => {
                        self.active = Active::Select(handle);
                        return Poll::Pending;
                    }
                    Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                    Poll::Ready(Ok(outcome)) => {
                        let facts = self.facts.as_ref().expect("nearest facts retained");
                        let eligible = self.eligible.as_deref().expect("nearest indices retained");
                        match vendor::choose_vendor(
                            facts,
                            eligible,
                            &outcome,
                            &mut self.retry_used,
                            &mut self.looked,
                        )? {
                            VendorDecision::Go {
                                index, access_tile, ..
                            } => {
                                self.keeper = Some(facts.rows()[usize::from(index)].keeper);
                                self.target = Some(access_tile);
                                self.phase = Phase::Walk;
                            }
                            VendorDecision::Retry => self.phase = Phase::Select,
                            VendorDecision::Blocked(reason) => {
                                return Poll::Ready(Ok(self.block(request, reason, tick)));
                            }
                        }
                    }
                },
                Active::Walk(handle) => match tick.actions.poll(&handle, &mut tick.cx) {
                    Poll::Pending => {
                        self.active = Active::Walk(handle);
                        return Poll::Pending;
                    }
                    Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                    Poll::Ready(Ok(receipt)) => {
                        receipt.into_arrival()?;
                        self.phase = Phase::Buy;
                    }
                },
                Active::Buy(handle) => match tick.actions.poll(&handle, &mut tick.cx) {
                    Poll::Pending => {
                        self.active = Active::Buy(handle);
                        return Poll::Pending;
                    }
                    Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                    Poll::Ready(Ok(receipt)) => {
                        if let Some(outcome) = self.bought(request, receipt, tick)? {
                            return Poll::Ready(Ok(outcome));
                        }
                    }
                },
            }
            if !tick.cx.may_continue_this_tick() {
                return Poll::Pending;
            }
            match self.phase {
                Phase::Select => {
                    let facts = self.facts.as_ref().expect("nearest facts retained");
                    let eligible = self.eligible.as_ref().expect("nearest indices retained");
                    if eligible.is_empty() {
                        return Poll::Ready(Ok(self.block(request, BlockReason::NoVendor, tick)));
                    }
                    let Some(from) = tick.cx.snapshot().here().map(|here| here.value) else {
                        return Poll::Pending;
                    };
                    self.active = Active::Select(tick.actions.begin::<SelectService>(
                        ServicePickRequest {
                            kind: ServiceKind::ShopKeeper,
                            from,
                            options: request.walk,
                            facts: ServiceFacts::ShopKeeper(Arc::clone(facts)),
                            eligible: Arc::clone(eligible),
                            prefer_order: true,
                        },
                        &mut tick.cx,
                    )?);
                }
                Phase::Walk => {
                    let Some(target) = self.target else {
                        self.phase = Phase::Buy;
                        continue;
                    };
                    self.active = Active::Walk(tick.actions.begin::<Walk>(
                        WalkRequest {
                            target,
                            loc_id: None,
                            radius: if self.nearest { 0 } else { 6 },
                            arrival: nav_planner::contract::ArrivalKind::Reach,
                            options: request.walk,
                            required_after: request.required_after,
                            evidence: None,
                            cross: Box::default(),
                            protect: false,
                            food_guard: false,
                            allow: Default::default(),
                        },
                        &mut tick.cx,
                    )?);
                }
                Phase::Buy => {
                    if gate == OpenGate::Hold {
                        return Poll::Pending;
                    }
                    let Some(keeper) = self.keeper else {
                        return Poll::Ready(Ok(self.block(request, BlockReason::NoVendor, tick)));
                    };
                    self.active = Active::Buy(tick.actions.begin::<BuyMachine>(
                        BuyRequest {
                            npc_id: keeper.keeper_id,
                            npc_name: Arc::from(keeper.keeper_name),
                            item_id: self.purchase.item,
                            item_name: Arc::clone(
                                &request.needs[usize::from(self.purchase.need)].name,
                            ),
                            qty: self.purchase.target,
                        },
                        &mut tick.cx,
                    )?);
                }
            }
        }
    }

    fn bought(
        &mut self,
        request: &AcquireRequest,
        receipt: BuyReceipt,
        tick: &NativeTick<'_>,
    ) -> Result<Option<Outcome>, ActionError> {
        self.bought = self
            .bought
            .checked_add(receipt.bought)
            .ok_or_else(|| ActionError::Failed(Arc::from("observed purchase count exceeds u32")))?;
        let need = &request.needs[usize::from(self.purchase.need)];
        if receipt.held >= self.purchase.minimum || need.kind == NeedKind::Optional {
            return Ok(Some(Outcome::Complete {
                bought: self.bought,
                short: receipt.held < self.purchase.target,
            }));
        }
        let keeper = self.keeper.expect("buyer retained exact keeper");
        let data = api::game_data::for_revision(tick.cx.pin().revision)
            .map_err(|error| ActionError::Unavailable(Arc::from(error)))?;
        let fact =
            shop_facts::vendor_by_keeper_config(&data, keeper.keeper_config, self.purchase.item)
                .ok_or(ActionError::Stale)?;
        let reason = match receipt.refusal.ok_or(ActionError::Stale)? {
            PurchaseRefusal::Coins => {
                let stock = tick.cx.snapshot().stock();
                let banked = if stock.banked_origin() == Origin::Session {
                    stock.banked(995).unwrap_or(0)
                } else {
                    0
                };
                BlockReason::TooFewCoins {
                    needed: shop_facts::quote(
                        fact.record,
                        fact.item,
                        self.purchase.minimum.saturating_sub(receipt.held),
                        receipt.stock,
                    ),
                    held: i64::from(stock.held(995).unwrap_or(0)),
                    banked: i64::from(banked),
                }
            }
            PurchaseRefusal::Space => BlockReason::NoSpace {
                slots_needed: Stock::incoming_slots(
                    receipt.held,
                    self.purchase.minimum,
                    fact.item.stackable,
                    i32::MAX,
                ),
            },
            PurchaseRefusal::Stock => {
                vendor::record_vendor(
                    &mut self.looked,
                    keeper.keeper_id,
                    VendorOutcome::OutOfStock,
                );
                if let (Some(facts), Some(eligible)) = (&self.facts, &self.eligible) {
                    let count = eligible
                        .iter()
                        .filter(|&&index| {
                            facts.rows()[usize::from(index)].keeper.keeper_id != keeper.keeper_id
                        })
                        .count();
                    if count > 0 {
                        let mut remaining = eligible.iter().copied().filter(|&index| {
                            facts.rows()[usize::from(index)].keeper.keeper_id != keeper.keeper_id
                        });
                        self.eligible = Some(
                            (0..count)
                                .map(|_| remaining.next().expect("counted remaining vendor"))
                                .collect(),
                        );
                        self.keeper = None;
                        self.target = None;
                        self.retry_used = false;
                        self.phase = Phase::Select;
                        return Ok(None);
                    }
                }
                let tick_now = tick.cx.evidence().tick;
                let rate = u64::try_from(fact.item.restock_ticks)
                    .ok()
                    .filter(|&rate| rate > 0)
                    .ok_or_else(|| {
                        ActionError::Unavailable(Arc::from("shop has no positive restock interval"))
                    })?;
                let revisit_due = receipt.restock_at.map_or_else(
                    || tick_now.saturating_add(rate),
                    |edge| {
                        edge.saturating_add(
                            tick_now
                                .saturating_sub(edge)
                                .checked_div(rate)
                                .unwrap_or(0)
                                .saturating_add(1)
                                .saturating_mul(rate),
                        )
                    },
                );
                BlockReason::NoStock {
                    restock_ticks: fact.item.restock_ticks,
                    refused_at_tick: tick_now,
                    aligned_to: receipt.restock_at,
                    revisit_due,
                }
            }
        };
        Ok(Some(self.block(request, reason, tick)))
    }

    fn block(
        &mut self,
        request: &AcquireRequest,
        reason: BlockReason,
        tick: &NativeTick<'_>,
    ) -> Outcome {
        let need = &request.needs[usize::from(self.purchase.need)];
        let stock = tick.cx.snapshot().stock();
        let held = stock.held(self.purchase.item).unwrap_or(0);
        if held >= self.purchase.minimum || need.kind == NeedKind::Optional {
            return Outcome::Complete {
                bought: self.bought,
                short: held < self.purchase.target,
            };
        }
        let mut looked = plan::looked(&stock, need, self.purchase.item);
        looked.vendors = self.looked.vendors;
        looked.more_vendors = self.looked.more_vendors;
        let line = match &reason {
            BlockReason::NoVendor => "nobody sells it in the pinned eligible set".to_string(),
            BlockReason::TooFewCoins { needed, held, banked } => format!(
                "{} asks {needed} gp for the usable remainder at live stock; you hold {held} gp and the bank holds {banked} gp",
                self.keeper.expect("live buyer keeper").keeper_name,
            ),
            BlockReason::NoSpace { .. } => format!("{} could not add the usable minimum to the pack", self.keeper.expect("live buyer keeper").keeper_name),
            BlockReason::NoStock { .. } => format!("{} had none (seen now)", self.keeper.expect("live buyer keeper").keeper_name),
            BlockReason::VendorUnreachable | BlockReason::VendorPickUndecided => {
                let mut names = String::new();
                if let Some(facts) = &self.facts {
                    for (position, tried) in looked.vendors.iter().flatten().enumerate() {
                        if let Some(row) = facts.rows().iter().find(|row| row.keeper.keeper_id == tried.keeper_id) {
                            if position > 0 { names.push_str(", "); }
                            names.push_str(row.keeper.keeper_name);
                        }
                    }
                }
                if looked.more_vendors > 0 {
                    let _ = write!(names, " and {} more keepers", looked.more_vendors);
                }
                if reason == BlockReason::VendorUnreachable {
                    format!("{names} cannot be reached from here")
                } else {
                    format!("the route to {names} is undecided")
                }
            }
            BlockReason::AuthoredShortage | BlockReason::SupplyMissing => unreachable!("shop refusal reason"),
        };
        let message = plan::block_message(
            &need.name,
            &reason,
            &looked,
            &line,
            std::time::SystemTime::now(),
            tick.cx.evidence().tick,
        );
        Outcome::Blocked(AcquireBlock {
            item: Arc::clone(&need.name),
            need: self.purchase.need,
            reason,
            looked,
            message,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::ledger::{HostEffect, Ledger};
    use crate::quester::families::tests::{with_tick_bank, with_tick_shop};
    use api::bank_memory::BankMemory;
    use api::service_pick::ResolvedShopKeeper;
    use api::snapshot::{GameSnapshot, ItemActionFamily, ItemContainer, ItemView};

    fn tile(x: i32) -> WorldTile {
        WorldTile {
            x,
            z: 3200,
            level: 0,
        }
    }

    fn fixture() -> (AcquireRequest, Purchase, GameSnapshot, Arc<ShopKeeperFacts>) {
        let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let item = data.item_by_alias("harpoon").unwrap().id;
        let gerrant = shop_facts::keepers(&data)
            .find(|row| row.keeper_config == "gerrant")
            .unwrap();
        let harry = shop_facts::keepers(&data)
            .find(|row| row.keeper_config == "harry")
            .unwrap();
        let facts = Arc::new(ShopKeeperFacts::from_rows(
            vec![
                ResolvedShopKeeper::new(0, gerrant, tile(3202), [tile(3201)]),
                ResolvedShopKeeper::new(1, harry, tile(3210), [tile(3209)]),
                ResolvedShopKeeper::new(2, gerrant, tile(3220), [tile(3219)]),
            ]
            .into_boxed_slice(),
        ));
        let request = AcquireRequest {
            needs: Arc::from([Need {
                item,
                name: Arc::from("Harpoon"),
                target: 5,
                minimum: 3,
                presence: Presence::Held,
                kind: NeedKind::Required,
                sources: SourceSet::PACK | SourceSet::SHOP,
                alternatives: Arc::from([]),
            }]),
            deposit: None,
            bank: BankChoice {
                bank: None,
                explicit: None,
                required: false,
                preferences: Default::default(),
            },
            stage: BankStage::Planned,
            partial_ok: true,
            close: CloseBank::Close,
            vendor: VendorChoice::Explicit {
                keeper_config: Arc::from("gerrant"),
                anchor: None,
            },
            coins: CoinPolicy::ForPurchase {
                sources: SourceSet::PACK | SourceSet::BANK,
            },
            walk: WalkOptions::default(),
            required_after: api::quest_progress::EvidenceStamp {
                run: api::selected::RunKey {
                    slot: 1,
                    run: 1,
                    session: 1,
                },
                tick: 0,
                sequence: 0,
            },
        };
        let purchase = Purchase {
            need: 0,
            item,
            target: 5,
            minimum: 3,
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(crate::quester::families::tests::local_player(tile(3200)));
        snapshot.seed_inventory(Vec::new(), 28);
        snapshot.seed_equipment(Vec::new());
        (request, purchase, snapshot, facts)
    }

    fn row(id: i32, count: i32) -> ItemView {
        ItemView {
            def: crate::quester::families::tests::def(id, "Observed item"),
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot: 0,
            count,
            actions: Vec::new(),
            component_id: 3214,
        }
    }

    fn start(
        request: &AcquireRequest,
        purchase: Purchase,
        snapshot: &GameSnapshot,
        facts: &Arc<ShopKeeperFacts>,
        ledger: &mut Option<Box<Ledger>>,
    ) -> ShopRun {
        with_tick_shop(snapshot, facts, None, ledger, 1, |tick| {
            ShopRun::new(request, purchase, &tick.cx).unwrap()
        })
    }

    fn receipt(held: i32, bought: u32, refusal: PurchaseRefusal) -> BuyReceipt {
        BuyReceipt {
            held,
            bought,
            refusal: Some(refusal),
            stock: 0,
            restock_at: None,
        }
    }

    #[test]
    fn explicit_anchor_queues_only_the_authored_walk_without_a_vendor_pick() {
        let (mut request, purchase, snapshot, facts) = fixture();
        request.vendor = VendorChoice::Explicit {
            keeper_config: Arc::from("gerrant"),
            anchor: Some(tile(3300)),
        };
        let mut ledger = None;
        let mut run = start(&request, purchase, &snapshot, &facts, &mut ledger);
        assert!(!run.nearest);
        assert!(run.facts.is_none());
        with_tick_bank(&snapshot, None, &mut ledger, 2, |tick| {
            assert!(run.poll(&request, tick, OpenGate::Open).is_pending());
        });
        let outbox = &ledger.as_ref().unwrap().outbox;
        assert_eq!(outbox.len(), 1);
        assert!(matches!(&outbox[0].effect, HostEffect::Walk(walk)
            if walk.target == tile(3300) && walk.radius == 6 && walk.options == request.walk));
    }

    #[test]
    fn nearest_pick_carries_one_shared_domain_and_cancellation_revokes_it() {
        let (mut request, purchase, snapshot, facts) = fixture();
        request.vendor = VendorChoice::Nearest;
        let mut ledger = None;
        assert!(with_tick_bank(&snapshot, None, &mut ledger, 1, |tick| {
            ShopRun::new(&request, purchase, &tick.cx)
        })
        .is_err());
        let mut run = start(&request, purchase, &snapshot, &facts, &mut ledger);
        assert_eq!(Arc::strong_count(&facts), 2);
        with_tick_bank(&snapshot, None, &mut ledger, 2, |tick| {
            assert!(run.poll(&request, tick, OpenGate::Open).is_pending());
        });
        let action = &ledger.as_ref().unwrap().outbox[0];
        let authority = action.authority();
        match &action.effect {
            HostEffect::ServicePick(pick) => {
                assert_eq!(pick.kind, ServiceKind::ShopKeeper);
                assert_eq!(pick.options, request.walk);
                assert_eq!(&*pick.eligible, &[0, 1, 2]);
                assert!(pick.prefer_order);
                assert!(
                    matches!(&pick.facts, ServiceFacts::ShopKeeper(shared) if Arc::ptr_eq(shared, &facts))
                );
            }
            _ => panic!("nearest must submit a complete service domain"),
        }
        with_tick_bank(&snapshot, None, &mut ledger, 3, |tick| {
            run.cancel(tick.actions)
        });
        assert!(!authority.live());
        assert!(ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .all(|action| !action.live()));
    }

    #[test]
    fn held_minimum_and_optional_shortage_continue_with_exact_observed_units() {
        let (mut request, purchase, mut snapshot, facts) = fixture();
        let mut ledger = None;
        let mut run = start(&request, purchase, &snapshot, &facts, &mut ledger);
        snapshot.seed_inventory(vec![row(purchase.item, 3)], 25);
        let outcome = with_tick_bank(&snapshot, None, &mut ledger, 8, |tick| {
            run.bought(&request, receipt(3, 300, PurchaseRefusal::Coins), tick)
                .unwrap()
                .unwrap()
        });
        assert!(matches!(
            outcome,
            Outcome::Complete {
                bought: 300,
                short: true
            }
        ));

        Arc::get_mut(&mut request.needs).unwrap()[0].kind = NeedKind::Optional;
        let mut run = start(&request, purchase, &snapshot, &facts, &mut ledger);
        let outcome = with_tick_bank(&snapshot, None, &mut ledger, 9, |tick| {
            run.bought(&request, receipt(1, 1, PurchaseRefusal::Space), tick)
                .unwrap()
                .unwrap()
        });
        assert!(matches!(
            outcome,
            Outcome::Complete {
                bought: 1,
                short: true
            }
        ));
    }

    #[test]
    fn live_coins_and_space_refusals_use_the_missing_minimum_not_the_target() {
        let (request, purchase, mut snapshot, facts) = fixture();
        snapshot.seed_inventory(vec![row(purchase.item, 1), row(995, 7)], 26);
        let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let fact = shop_facts::vendor_by_keeper_config(&data, "gerrant", purchase.item).unwrap();
        let expected_quote = shop_facts::quote(fact.record, fact.item, 2, 0);
        for origin in [Origin::Session, Origin::Hint] {
            let bank = BankMemory::seeded(&[(995, 80)], origin);
            let mut ledger = None;
            let mut run = start(&request, purchase, &snapshot, &facts, &mut ledger);
            let outcome = with_tick_bank(&snapshot, Some(&bank), &mut ledger, 8, |tick| {
                run.bought(&request, receipt(1, 1, PurchaseRefusal::Coins), tick)
                    .unwrap()
                    .unwrap()
            });
            match outcome {
                Outcome::Blocked(block) => {
                    assert_eq!(
                        block.reason,
                        BlockReason::TooFewCoins {
                            needed: expected_quote,
                            held: 7,
                            banked: if origin == Origin::Session { 80 } else { 0 },
                        }
                    );
                    assert!(block
                        .message
                        .starts_with("no Harpoon in the pack, worn, or "));
                    assert!(block.message.contains("Gerrant asks"));
                }
                _ => panic!("required minimum must block"),
            }
        }
        let mut ledger = None;
        let mut run = start(&request, purchase, &snapshot, &facts, &mut ledger);
        let outcome = with_tick_bank(&snapshot, None, &mut ledger, 8, |tick| {
            run.bought(&request, receipt(1, 1, PurchaseRefusal::Space), tick)
                .unwrap()
                .unwrap()
        });
        assert!(matches!(
            outcome,
            Outcome::Blocked(AcquireBlock {
                reason: BlockReason::NoSpace { slots_needed: 2 },
                ..
            })
        ));
    }

    #[test]
    fn exhausted_stock_excludes_every_placement_before_the_next_complete_pick() {
        let (mut request, purchase, snapshot, facts) = fixture();
        request.vendor = VendorChoice::Nearest;
        let mut ledger = None;
        let mut run = start(&request, purchase, &snapshot, &facts, &mut ledger);
        run.keeper = Some(facts.rows()[0].keeper);
        run.retry_used = true;
        let result = with_tick_bank(&snapshot, None, &mut ledger, 8, |tick| {
            run.bought(&request, receipt(1, 1, PurchaseRefusal::Stock), tick)
                .unwrap()
        });
        assert!(result.is_none());
        assert_eq!(run.eligible.as_deref(), Some(&[1_u16][..]));
        assert!(run.keeper.is_none());
        assert!(run.target.is_none());
        assert!(!run.retry_used);
        assert_eq!(
            run.looked.vendors[0].unwrap().outcome,
            VendorOutcome::OutOfStock
        );
        with_tick_bank(&snapshot, None, &mut ledger, 9, |tick| {
            assert!(run.poll(&request, tick, OpenGate::Open).is_pending());
        });
        assert!(matches!(&ledger.as_ref().unwrap().outbox[0].effect,
            HostEffect::ServicePick(pick) if *pick.eligible == [1]));
    }

    #[test]
    fn final_stock_refusal_reports_the_next_observed_restock_edge() {
        let (request, purchase, snapshot, facts) = fixture();
        let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let fact = shop_facts::vendor_by_keeper_config(&data, "gerrant", purchase.item).unwrap();
        let rate = u64::try_from(fact.item.restock_ticks).unwrap();
        for aligned_to in [None, Some(4)] {
            let now = 4 + rate * 2;
            let mut ledger = None;
            let mut run = start(&request, purchase, &snapshot, &facts, &mut ledger);
            let mut result = receipt(0, 0, PurchaseRefusal::Stock);
            result.restock_at = aligned_to;
            let outcome = with_tick_bank(&snapshot, None, &mut ledger, now, |tick| {
                run.bought(&request, result, tick).unwrap().unwrap()
            });
            match outcome {
                Outcome::Blocked(block) => {
                    assert_eq!(
                        block.reason,
                        BlockReason::NoStock {
                            restock_ticks: fact.item.restock_ticks,
                            refused_at_tick: now,
                            aligned_to,
                            revisit_due: now + rate,
                        }
                    );
                    assert_eq!(
                        block.looked.vendors[0].unwrap().keeper_id,
                        facts.rows()[0].keeper.keeper_id
                    );
                }
                _ => panic!("no remaining vendor must block"),
            }
        }
    }
}
