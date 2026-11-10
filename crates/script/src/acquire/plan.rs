//! Pure acquisition decisions over borrowed counts.

use super::*;
use api::shop_facts::{self, VendorFact};
use api::stock::Stock;

#[cfg(test)]
std::thread_local! {
    static PLAN_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn calls_for_test() -> usize {
    PLAN_CALLS.get()
}
pub trait Counts {
    fn held(&self, id: i32) -> Option<i32>;
    fn worn(&self, id: i32) -> Option<i32>;
    fn banked(&self, id: i32) -> Option<i32>;
    fn banked_origin(&self) -> Origin;
    fn bank_seen(&self) -> Option<BankSeen>;
    fn free_slots(&self) -> Option<i32>;
}

impl Counts for Stock<'_> {
    fn held(&self, id: i32) -> Option<i32> {
        Stock::held(self, id)
    }
    fn worn(&self, id: i32) -> Option<i32> {
        Stock::worn(self, id)
    }
    fn banked(&self, id: i32) -> Option<i32> {
        Stock::banked(self, id)
    }
    fn banked_origin(&self) -> Origin {
        Stock::banked_origin(self)
    }
    fn bank_seen(&self) -> Option<BankSeen> {
        let bank = self.bank?;
        Some(BankSeen {
            origin: bank.origin(),
            observed_at_unix: bank.observed_at()?.unix_secs,
            rows: u16::try_from(bank.rows().len()).unwrap_or(u16::MAX),
            count: 0,
        })
    }
    fn free_slots(&self) -> Option<i32> {
        Stock::free_slots(self)
    }
}

/// Producer-side detail for status and incremental slot-fit decisions.
#[derive(Clone)]
pub struct NeedObservation {
    pub id: i32,
    pub item: Arc<str>,
    pub need: i32,
    pub pack: i32,
    pub bank: Option<i32>,
    pub index: u8,
}

/// Why an optional need finished below its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnmetCause {
    /// The plan found nothing more to draw it from.
    NoSupply,
    /// The pack had no free slot for it, so it was left out of the plan.
    NoRoom,
}

impl UnmetCause {
    pub const fn describe(self) -> &'static str {
        match self {
            Self::NoSupply => "nothing more to draw it from",
            Self::NoRoom => "no free pack slot for it",
        }
    }
}

/// An optional need that stays below its target while the plan still succeeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnmetNeed {
    pub item: Arc<str>,
    pub need: i32,
    pub have: i32,
    pub cause: UnmetCause,
}

pub struct Planner<'a> {
    counts: &'a dyn Counts,
    data: Option<&'a SelectedGameData>,
    plan: AcquirePlan,
    first_withdrawal: Option<NeedObservation>,
    scan: Option<NeedObservation>,
    blocked: Option<NeedObservation>,
    blocked_reason: Option<BlockReason>,
    blocked_vendor_line: Option<Arc<str>>,
    child: Option<NeedObservation>,
    next_index: usize,
    coin_reserve: i64,
    unmet: Vec<UnmetNeed>,
}

enum TierDecision {
    Selected {
        id: i32,
        reserve: i64,
    },
    Scan {
        id: i32,
    },
    TooFewCoins {
        id: i32,
        needed: i64,
        held: i32,
        banked: i32,
    },
    NoVendor {
        id: i32,
        reason: Arc<str>,
    },
}

impl<'a> Planner<'a> {
    pub fn new(counts: &'a dyn Counts, world: WorldFacts<'a>) -> Self {
        Self {
            counts,
            data: world.data,
            plan: AcquirePlan::empty(),
            first_withdrawal: None,
            scan: None,
            blocked: None,
            blocked_reason: None,
            blocked_vendor_line: None,
            child: None,
            next_index: 0,
            coin_reserve: 0,
            unmet: Vec::new(),
        }
    }

    pub fn require(&mut self, need: &Need) -> Result<(), ActionError> {
        self.require_with_purchase(
            need,
            &VendorChoice::None,
            CoinPolicy::None,
            WorldFacts::default(),
        )
    }

    fn require_with_purchase(
        &mut self,
        need: &Need,
        vendor: &VendorChoice,
        coins: CoinPolicy,
        world: WorldFacts<'_>,
    ) -> Result<(), ActionError> {
        let index = u8::try_from(self.next_index)
            .map_err(|_| ActionError::Unavailable(Arc::from("acquisition has too many needs")))?;
        self.next_index += 1;
        if need.target <= 0 {
            return Ok(());
        }
        let existing = acceptable_ids(need)
            .find(|&id| present(self.counts, need, id) >= need.target)
            .or_else(|| acceptable_ids(need).find(|&id| present(self.counts, need, id) > 0))
            .or_else(|| {
                need.sources
                    .contains(SourceSet::BANK)
                    .then(|| {
                        acceptable_ids(need).find(|&id| self.counts.banked(id).unwrap_or(0) > 0)
                    })
                    .flatten()
            });
        let tiered_purchase = existing.is_none() && tiered_shop_need(need);
        let id = existing.unwrap_or(need.item);
        let held = held(self.counts, need, id);
        let present = present(self.counts, need, id);
        let worn = worn(self.counts, need, id);
        let banked = need
            .sources
            .contains(SourceSet::BANK)
            .then(|| self.counts.banked(id))
            .flatten();
        if should_wear(world.data, need, id) && held > 0 && worn == 0 {
            self.add_wear(id)?;
        }
        if present >= need.target {
            return Ok(());
        }
        let detail = observation(self.counts, world.data, need, index, id);
        if need.sources.contains(SourceSet::BANK) && banked.is_none() {
            self.plan.scan = true;
            if self.scan.is_none() {
                self.scan = Some(detail);
            }
            return Ok(());
        }
        let take = need
            .target
            .saturating_sub(present)
            .min(banked.unwrap_or(0).max(0));
        if take > 0 {
            let held_target = self
                .counts
                .held(id)
                .unwrap_or(0)
                .max(0)
                .saturating_add(take);
            self.add_withdrawal(id, need.item, &need.name, held_target)?;
            if self.first_withdrawal.is_none() {
                self.first_withdrawal = Some(detail.clone());
            }
            if should_wear(world.data, need, id) && worn == 0 {
                self.add_wear(id)?;
            }
        }
        let reach = present.saturating_add(take);
        if reach >= need.target {
            return Ok(());
        }
        if need.sources.contains(SourceSet::BANK) && self.counts.banked_origin() == Origin::Hint {
            self.plan.scan = true;
            if self.scan.is_none() {
                self.scan = Some(detail);
            }
            return Ok(());
        }
        if need.sources.contains(SourceSet::CHILD) {
            if self.child.is_none() {
                self.plan.child = Some(index);
                self.child = Some(detail);
            }
            return Ok(());
        }
        if need.sources.contains(SourceSet::SHOP) {
            let units = need.target.saturating_sub(reach).max(0);
            let minimum_units = need
                .minimum
                .saturating_sub(present)
                .saturating_sub(take)
                .max(0);
            if tiered_purchase {
                match self.select_shop_tier(need, vendor, world, coins, units, minimum_units) {
                    TierDecision::Selected { id, reserve } => {
                        let tier_worn = self::worn(self.counts, need, id);
                        self.add_purchase(Purchase {
                            need: index,
                            item: id,
                            target: need.target.saturating_sub(tier_worn).max(0),
                            minimum: need.minimum.saturating_sub(tier_worn).max(0),
                        })?;
                        self.coin_reserve = self.coin_reserve.saturating_add(reserve);
                        self.reserve_coins(coins)?;
                    }
                    TierDecision::Scan { id } => {
                        self.plan.scan = true;
                        if self.scan.is_none() {
                            self.scan = Some(observation(self.counts, world.data, need, index, id));
                        }
                    }
                    TierDecision::TooFewCoins {
                        id,
                        needed,
                        held,
                        banked,
                    } => {
                        if need.kind == NeedKind::Required {
                            let vendor_line = vendor_line_for_purchase(
                                world.data,
                                vendor,
                                id,
                                world.members,
                                held,
                                banked,
                            );
                            self.record_block(
                                observation(self.counts, world.data, need, index, id),
                                need,
                                id,
                                BlockReason::TooFewCoins {
                                    needed,
                                    held: i64::from(held),
                                    banked: i64::from(banked),
                                },
                                Some(Arc::from(vendor_line)),
                            );
                        } else {
                            self.note_short_target();
                        }
                    }
                    TierDecision::NoVendor { id, reason } => {
                        if need.kind == NeedKind::Required && reach < need.minimum {
                            self.record_block(
                                observation(self.counts, world.data, need, index, id),
                                need,
                                id,
                                BlockReason::NoVendor,
                                Some(reason),
                            );
                        } else {
                            self.note_short_target();
                        }
                    }
                }
                return Ok(());
            }
            let purchase_quote = shop_quote(world.data, vendor, id, world.members, units).and_then(
                |(_, reserve)| {
                    minimum_shop_quote(world.data, vendor, id, world.members, minimum_units)
                        .map(|minimum_quote| (reserve, minimum_quote))
                },
            );
            if let Some((reserve, minimum_quote)) = purchase_quote {
                let (held_coins, banked_coins, unknown_bank) = purchase_coins(self.counts, coins);
                if i64::from(held_coins).saturating_add(i64::from(banked_coins)) < minimum_quote {
                    if unknown_bank {
                        self.plan.scan = true;
                        if self.scan.is_none() {
                            self.scan = Some(detail.clone());
                        }
                    } else if need.kind == NeedKind::Required {
                        let vendor_line = vendor_line_for_purchase(
                            world.data,
                            vendor,
                            id,
                            world.members,
                            held_coins,
                            banked_coins,
                        );
                        self.record_block(
                            detail,
                            need,
                            id,
                            BlockReason::TooFewCoins {
                                needed: minimum_quote,
                                held: i64::from(held_coins),
                                banked: i64::from(banked_coins),
                            },
                            Some(Arc::from(vendor_line)),
                        );
                    } else {
                        self.note_short_target();
                    }
                    return Ok(());
                }
                self.add_purchase(Purchase {
                    need: index,
                    item: id,
                    target: need.target.saturating_sub(worn).max(0),
                    minimum: need.minimum.saturating_sub(worn).max(0),
                })?;
                if should_wear(world.data, need, id) {
                    self.add_wear(id)?;
                }
                self.coin_reserve = self.coin_reserve.saturating_add(reserve);
                self.reserve_coins(coins)?;
                return Ok(());
            }
            if need.kind == NeedKind::Required && reach < need.minimum {
                let vendor_line =
                    vendor_line_for_purchase(world.data, vendor, id, world.members, 0, 0);
                self.record_block(
                    detail,
                    need,
                    id,
                    BlockReason::NoVendor,
                    Some(Arc::from(vendor_line)),
                );
            } else {
                self.note_short_target();
            }
            return Ok(());
        }
        if reach >= need.minimum || need.kind == NeedKind::Optional {
            self.note_short_target();
            if need.kind == NeedKind::Optional {
                self.unmet.push(UnmetNeed {
                    item: Arc::clone(&need.name),
                    need: need.target,
                    have: reach,
                    cause: UnmetCause::NoSupply,
                });
            }
        } else if self.blocked.is_none() {
            self.record_block(detail, need, id, BlockReason::SupplyMissing, None);
        }
        Ok(())
    }

    fn select_shop_tier(
        &self,
        need: &Need,
        vendor: &VendorChoice,
        world: WorldFacts<'_>,
        coins: CoinPolicy,
        units: i32,
        minimum_units: i32,
    ) -> TierDecision {
        let (held_coins, banked_coins, unknown_bank) = purchase_coins(self.counts, coins);
        let budget = i64::from(held_coins).saturating_add(i64::from(banked_coins));
        let best = acceptable_ids(need)
            .find(|&id| {
                world
                    .data
                    .is_none_or(|data| wieldable_for_need(data, need, id))
            })
            .unwrap_or(need.item);
        let mut wearable_tier = false;
        let mut stocked_tier = false;
        let mut first_unaffordable = None;
        for id in acceptable_ids(need) {
            if !world
                .data
                .is_none_or(|data| wieldable_for_need(data, need, id))
            {
                continue;
            }
            wearable_tier = true;
            let Some(price) =
                minimum_shop_quote(world.data, vendor, id, world.members, minimum_units)
            else {
                stocked_tier |= eligible_shop_stock(world.data, vendor, id, world.members);
                continue;
            };
            stocked_tier = true;
            if budget < price {
                if unknown_bank && i64::from(held_coins) < price {
                    return TierDecision::Scan { id };
                }
                first_unaffordable.get_or_insert((id, price));
                continue;
            }
            if let Some((_, reserve)) = shop_quote(world.data, vendor, id, world.members, units) {
                return TierDecision::Selected { id, reserve };
            }
        }
        if let Some((id, needed)) = first_unaffordable {
            return TierDecision::TooFewCoins {
                id,
                needed,
                held: held_coins,
                banked: banked_coins,
            };
        }
        let name = item_name(world.data, need, best);
        let reason = if world.data.is_none() {
            format!("best tier {name} fails condition: pinned vendor price facts are unavailable")
        } else if !wearable_tier {
            format!(
                "best tier {name} fails condition: no tier meets the selected wield requirement"
            )
        } else if !stocked_tier {
            format!("best tier {name} fails condition: no eligible vendor stocks it")
        } else {
            format!("best tier {name} fails condition: no pinned purchase quote is available")
        };
        TierDecision::NoVendor {
            id: best,
            reason: Arc::from(reason),
        }
    }

    /// Records an optional need that the caller left out of the plan.
    pub fn note_unmet(&mut self, unmet: UnmetNeed) {
        self.unmet.push(unmet);
    }
    /// Optional needs that stay below their target.
    pub fn unmet(&self) -> &[UnmetNeed] {
        &self.unmet
    }
    pub fn withdrawal_count(&self) -> usize {
        self.plan.withdrawal_count
    }
    pub fn first_withdrawal(&self) -> Option<&NeedObservation> {
        self.first_withdrawal.as_ref()
    }
    pub fn scan(&self) -> Option<&NeedObservation> {
        self.scan.as_ref()
    }
    pub fn child(&self) -> Option<u8> {
        self.plan.child
    }
    pub fn child_detail(&self) -> Option<&NeedObservation> {
        self.child.as_ref()
    }
    pub fn blocked(&self) -> Option<&NeedObservation> {
        self.blocked.as_ref()
    }
    pub fn withdrawals(&self) -> impl Iterator<Item = &Withdrawal> {
        self.plan.withdrawals[..self.plan.withdrawal_count]
            .iter()
            .flatten()
    }
    pub fn take_withdrawals(&mut self) -> Arc<[Withdrawal]> {
        self.plan.withdrawals[..self.plan.withdrawal_count]
            .iter_mut()
            .filter_map(Option::take)
            .collect::<Vec<_>>()
            .into()
    }

    pub fn verdict(self, deposit: Option<&DepositSpec>) -> AcquireVerdict {
        if !self.plan.scan {
            if let Some(detail) = self.blocked {
                let reason = self.blocked_reason.unwrap_or(BlockReason::SupplyMissing);
                let item = Arc::clone(&detail.item);
                let vendor_line = self
                    .blocked_vendor_line
                    .as_deref()
                    .unwrap_or("nobody sells it");
                let message = block_message(
                    &item,
                    &reason,
                    &self.plan.looked,
                    vendor_line,
                    std::time::SystemTime::now(),
                    0,
                );
                return AcquireVerdict::Blocked(AcquireBlock {
                    item,
                    need: detail.index,
                    reason,
                    looked: self.plan.looked,
                    message,
                });
            }
        }
        if self.plan.withdrawal_count == 0
            && self.plan.purchase_count == 0
            && self.plan.wear_count == 0
            && !self.plan.scan
            && self.plan.child.is_none()
            && deposit.is_none()
        {
            AcquireVerdict::Satisfied
        } else {
            AcquireVerdict::Plan(self.plan)
        }
    }

    fn record_block(
        &mut self,
        detail: NeedObservation,
        need: &Need,
        id: i32,
        reason: BlockReason,
        vendor_line: Option<Arc<str>>,
    ) {
        if self.blocked.is_none() {
            self.plan.looked = Looked {
                short_of_target: self.plan.looked.short_of_target,
                ..looked(self.counts, need, id)
            };
            self.blocked = Some(detail);
            self.blocked_reason = Some(reason);
            self.blocked_vendor_line = vendor_line;
        }
    }

    fn note_short_target(&mut self) {
        self.plan.looked.short_of_target = self.plan.looked.short_of_target.saturating_add(1);
    }

    fn reserve_coins(&mut self, coins: CoinPolicy) -> Result<(), ActionError> {
        let CoinPolicy::ForPurchase { sources } = coins else {
            return Ok(());
        };
        if self.plan.purchase_count == 0 || !sources.contains(SourceSet::BANK) {
            return Ok(());
        }
        let (held_coins, banked_coins, unknown_bank) = purchase_coins(self.counts, coins);
        if unknown_bank {
            if self.coin_reserve > i64::from(held_coins) {
                self.plan.scan = true;
            }
            return Ok(());
        }
        let target = self
            .coin_reserve
            .min(i64::from(held_coins).saturating_add(i64::from(banked_coins)))
            .min(i64::from(i32::MAX));
        if target > i64::from(held_coins) {
            self.add_withdrawal(995, 995, &Arc::from("coins"), target as i32)?;
        }
        Ok(())
    }

    fn add_purchase(&mut self, purchase: Purchase) -> Result<(), ActionError> {
        if self.plan.purchase_count == MAX_MEMO {
            return Err(ActionError::Unavailable(Arc::from(
                "acquisition purchase batch is too large",
            )));
        }
        self.plan.purchases[self.plan.purchase_count] = Some(purchase);
        self.plan.purchase_count += 1;
        Ok(())
    }

    fn add_withdrawal(
        &mut self,
        id: i32,
        item: i32,
        name: &Arc<str>,
        target: i32,
    ) -> Result<(), ActionError> {
        if let Some(row) = self.plan.withdrawals[..self.plan.withdrawal_count]
            .iter_mut()
            .flatten()
            .find(|row| row.id == id)
        {
            row.target = row.target.max(target);
            return Ok(());
        }
        if self.plan.withdrawal_count == MAX_MEMO {
            return Err(ActionError::Unavailable(Arc::from(
                "provision withdrawal exceeds the bank batch",
            )));
        }
        // An admitted tier need keeps its primary name while live stock can
        // select another id. Dispatch validates the chosen id and name.
        let name = if id == item {
            Arc::clone(name)
        } else {
            let name = self
                .data
                .and_then(|data| data.item_by_id(id))
                .and_then(|item| item.name.as_deref())
                .filter(|name| !name.trim().is_empty())
                .ok_or_else(|| {
                    ActionError::Unavailable(Arc::from(format!(
                        "selected cache has no name for acquisition alternative {id}"
                    )))
                })?;
            Arc::from(name)
        };
        self.plan.withdrawals[self.plan.withdrawal_count] = Some(Withdrawal { id, name, target });
        self.plan.withdrawal_count += 1;
        Ok(())
    }

    fn add_wear(&mut self, id: i32) -> Result<(), ActionError> {
        if self.plan.wears[..self.plan.wear_count].contains(&Some(id)) {
            return Ok(());
        }
        if self.plan.wear_count == MAX_MEMO {
            return Err(ActionError::Unavailable(Arc::from(
                "acquisition wear batch is too large",
            )));
        }
        self.plan.wears[self.plan.wear_count] = Some(id);
        self.plan.wear_count += 1;
        Ok(())
    }
}

pub fn plan(
    request: &AcquireRequest,
    counts: &dyn Counts,
    world: WorldFacts<'_>,
) -> Result<AcquireVerdict, ActionError> {
    #[cfg(test)]
    PLAN_CALLS.set(PLAN_CALLS.get() + 1);
    if let BankStage::Authored(actions) = &request.stage {
        if let Some(message) = session_shortage(actions, request.partial_ok, counts) {
            return Ok(AcquireVerdict::Blocked(AcquireBlock {
                item: Arc::from(""),
                need: 0,
                reason: BlockReason::AuthoredShortage,
                looked: Looked::default(),
                message,
            }));
        }
        return Ok(AcquireVerdict::Plan(AcquirePlan::authored()));
    }
    let mut planner = Planner::new(counts, world);
    for need in request.needs.iter() {
        planner.require_with_purchase(need, &request.vendor, request.coins, world)?;
    }
    Ok(planner.verdict(request.deposit.as_ref()))
}

/// This query never plans work, changes a block, or trusts bank hints.
pub fn resolved(
    block: &AcquireBlock,
    request: &AcquireRequest,
    counts: &dyn Counts,
    _world: WorldFacts<'_>,
    now_tick: u64,
) -> bool {
    let Some(need) = request.needs.get(usize::from(block.need)) else {
        return false;
    };
    if acceptable_ids(need).any(|id| {
        let banked = if counts.banked_origin() == Origin::Session
            && need.sources.contains(SourceSet::BANK)
        {
            counts.banked(id).unwrap_or(0).max(0)
        } else {
            0
        };
        present(counts, need, id).saturating_add(banked) >= need.minimum
    }) {
        return true;
    }
    match block.reason {
        BlockReason::TooFewCoins { needed, .. } => {
            let banked = match request.coins {
                CoinPolicy::ForPurchase { sources }
                    if sources.contains(SourceSet::BANK)
                        && counts.banked_origin() == Origin::Session =>
                {
                    counts.banked(995).unwrap_or(0).max(0)
                }
                _ => 0,
            };
            i64::from(counts.held(995).unwrap_or(0).max(0)) + i64::from(banked) >= needed
        }
        BlockReason::NoStock { revisit_due, .. } => now_tick >= revisit_due,
        BlockReason::NoSpace { slots_needed } => {
            counts.free_slots().is_some_and(|free| free >= slots_needed)
        }
        _ => false,
    }
}

pub(crate) fn acceptable_ids(need: &Need) -> impl Iterator<Item = i32> + '_ {
    need.alternatives
        .iter()
        .copied()
        .chain((!need.alternatives.contains(&need.item)).then_some(need.item))
}

fn held(counts: &dyn Counts, need: &Need, id: i32) -> i32 {
    if need.sources.contains(SourceSet::PACK) {
        counts.held(id).unwrap_or(0).max(0)
    } else {
        0
    }
}

pub(super) fn worn(counts: &dyn Counts, need: &Need, id: i32) -> i32 {
    if need.presence != Presence::Held && need.sources.contains(SourceSet::WORN) {
        counts.worn(id).unwrap_or(0).max(0)
    } else {
        0
    }
}

pub(crate) fn present(counts: &dyn Counts, need: &Need, id: i32) -> i32 {
    held(counts, need, id).saturating_add(worn(counts, need, id))
}

pub(crate) fn looked(counts: &dyn Counts, need: &Need, id: i32) -> Looked {
    Looked {
        pack: held(counts, need, id),
        worn: worn(counts, need, id),
        bank: need
            .sources
            .contains(SourceSet::BANK)
            .then(|| counts.bank_seen())
            .flatten()
            .map(|mut bank| {
                bank.count = counts.banked(id).unwrap_or(0);
                bank
            }),
        ..Looked::default()
    }
}

pub(super) fn tiered_shop_need(need: &Need) -> bool {
    need.sources.contains(SourceSet::SHOP) && !need.alternatives.is_empty()
}

fn static_wield_attack(data: &api::game_data::SelectedGameData, id: i32) -> Option<Option<i32>> {
    api::gather_tools::AXES
        .iter()
        .chain(api::gather_tools::PICKAXES)
        .find_map(|tool| {
            data.item_by_alias(tool.alias)
                .filter(|item| item.id == id)
                .map(|_| tool.wield_attack)
        })
}

pub(super) fn wieldable_for_need(
    data: &api::game_data::SelectedGameData,
    need: &Need,
    id: i32,
) -> bool {
    if need.presence != Presence::Wielded {
        return true;
    }
    match (
        static_wield_attack(data, id),
        static_wield_attack(data, need.item),
    ) {
        (Some(candidate), Some(selected)) => {
            candidate.is_none_or(|candidate| selected.is_some_and(|selected| candidate <= selected))
        }
        _ => true,
    }
}

fn should_wear(data: Option<&api::game_data::SelectedGameData>, need: &Need, id: i32) -> bool {
    need.presence == Presence::Wielded && data.is_none_or(|data| wieldable_for_need(data, need, id))
}

/// Generated shop rows for one item. Rows the selected game data does not
/// carry yield no vendor, so an incomplete table fails closed.
fn vendors_for<'a>(
    data: &'a api::game_data::SelectedGameData,
    item_id: i32,
) -> impl Iterator<Item = VendorFact<'a>> + 'a {
    shop_facts::vendors_for(data, item_id).into_iter().flatten()
}

fn vendor_by_keeper_config<'a>(
    data: &'a api::game_data::SelectedGameData,
    keeper_config: &str,
    item_id: i32,
) -> Option<VendorFact<'a>> {
    shop_facts::vendor_by_keeper_config(data, keeper_config, item_id)
        .ok()
        .flatten()
}

fn minimum_fact_quote(fact: &VendorFact<'_>, units: i32) -> Option<i64> {
    (fact.item.baseline > 0)
        .then(|| shop_facts::quote(fact.record, fact.item, units, fact.item.baseline))
}

pub(super) fn minimum_shop_quote(
    data: Option<&api::game_data::SelectedGameData>,
    choice: &VendorChoice,
    item_id: i32,
    members: bool,
    units: i32,
) -> Option<i64> {
    let data = data?;
    match choice {
        VendorChoice::None => None,
        VendorChoice::Explicit { keeper_config, .. } => {
            let fact = vendor_by_keeper_config(data, keeper_config, item_id)?;
            vendor_eligible(&fact, members)
                .then(|| minimum_fact_quote(&fact, units))
                .flatten()
        }
        VendorChoice::Nearest => vendors_for(data, item_id)
            .filter(|fact| vendor_eligible(fact, members))
            .filter_map(|fact| minimum_fact_quote(&fact, units))
            .min(),
    }
}

pub(super) fn keeper_minimum_shop_quote(
    data: &api::game_data::SelectedGameData,
    keeper_config: &str,
    item_id: i32,
    members: bool,
    units: i32,
) -> Option<i64> {
    let fact = vendor_by_keeper_config(data, keeper_config, item_id)?;
    vendor_eligible(&fact, members)
        .then(|| minimum_fact_quote(&fact, units))
        .flatten()
}

fn eligible_shop_stock(
    data: Option<&api::game_data::SelectedGameData>,
    choice: &VendorChoice,
    item_id: i32,
    members: bool,
) -> bool {
    let Some(data) = data else {
        return false;
    };
    match choice {
        VendorChoice::None => false,
        VendorChoice::Explicit { keeper_config, .. } => {
            vendor_by_keeper_config(data, keeper_config, item_id)
                .is_some_and(|fact| vendor_eligible(&fact, members))
        }
        VendorChoice::Nearest => {
            vendors_for(data, item_id).any(|fact| vendor_eligible(&fact, members))
        }
    }
}

pub(super) fn item_name(
    data: Option<&api::game_data::SelectedGameData>,
    need: &Need,
    id: i32,
) -> Arc<str> {
    if id == need.item {
        return Arc::clone(&need.name);
    }
    data.and_then(|data| data.item_by_id(id))
        .and_then(|item| item.name.as_deref())
        .map(Arc::from)
        .unwrap_or_else(|| Arc::clone(&need.name))
}

fn observation(
    counts: &dyn Counts,
    data: Option<&api::game_data::SelectedGameData>,
    need: &Need,
    index: u8,
    id: i32,
) -> NeedObservation {
    NeedObservation {
        id,
        item: item_name(data, need, id),
        need: need.target,
        pack: present(counts, need, id),
        bank: need
            .sources
            .contains(SourceSet::BANK)
            .then(|| counts.banked(id))
            .flatten(),
        index,
    }
}

fn vendor_eligible(fact: &VendorFact<'_>, members: bool) -> bool {
    members || !fact.item.members
}

fn two_chunk_quote(fact: &VendorFact<'_>, units: i32) -> i64 {
    shop_facts::quote(fact.record, fact.item, units, fact.item.baseline).saturating_add(
        shop_facts::quote(
            fact.record,
            fact.item,
            units,
            fact.item.baseline.saturating_sub(units),
        ),
    )
}

fn shop_quote<'a>(
    data: Option<&'a api::game_data::SelectedGameData>,
    choice: &VendorChoice,
    item_id: i32,
    members: bool,
    units: i32,
) -> Option<(VendorFact<'a>, i64)> {
    let data = data?;
    match choice {
        VendorChoice::None => None,
        VendorChoice::Explicit { keeper_config, .. } => {
            let fact = vendor_by_keeper_config(data, keeper_config, item_id)?;
            if !vendor_eligible(&fact, members) {
                return None;
            }
            let reserve = two_chunk_quote(&fact, units);
            Some((fact, reserve))
        }
        VendorChoice::Nearest => {
            let mut best: Option<(VendorFact<'a>, i64)> = None;
            for fact in vendors_for(data, item_id) {
                if !vendor_eligible(&fact, members) {
                    continue;
                }
                let quote = two_chunk_quote(&fact, units);
                if match best.as_ref() {
                    None => true,
                    Some((_, current)) => quote > *current,
                } {
                    best = Some((fact, quote));
                }
            }
            best
        }
    }
}

pub(super) fn purchase_coins(counts: &dyn Counts, policy: CoinPolicy) -> (i32, i32, bool) {
    let held = counts.held(995).unwrap_or(0).max(0);
    let CoinPolicy::ForPurchase { sources } = policy else {
        return (held, 0, false);
    };
    if !sources.contains(SourceSet::BANK) {
        return (held, 0, false);
    }
    match counts.banked_origin() {
        Origin::Session => match counts.banked(995) {
            Some(banked) => (held, banked.max(0), false),
            None => (held, 0, true),
        },
        Origin::Unknown | Origin::Hint => (held, 0, true),
    }
}

fn vendor_line_for_purchase(
    data: Option<&api::game_data::SelectedGameData>,
    choice: &VendorChoice,
    item_id: i32,
    members: bool,
    held: i32,
    banked: i32,
) -> String {
    let Some(data) = data else {
        return "vendor price unavailable".to_string();
    };
    match choice {
        VendorChoice::None => "nobody sells it".to_string(),
        VendorChoice::Explicit { keeper_config, .. } => {
            let Some(fact) = vendor_by_keeper_config(data, keeper_config, item_id)
                .filter(|fact| vendor_eligible(fact, members))
            else {
                return "nobody sells it".to_string();
            };
            let price = shop_facts::quote(fact.record, fact.item, 1, fact.item.baseline);
            format!(
                "{} sells one for {price} gp at baseline stock; you hold {held} gp and the bank holds {banked} gp",
                fact.keeper_name
            )
        }
        VendorChoice::Nearest => {
            let mut names = String::with_capacity(128);
            let mut count = 0usize;
            let mut low = i64::MAX;
            let mut high = 0i64;
            for fact in vendors_for(data, item_id) {
                if !vendor_eligible(&fact, members) {
                    continue;
                }
                if count > 0 {
                    names.push_str(" and ");
                }
                names.push_str(fact.keeper_name);
                count += 1;
                let price = shop_facts::quote(fact.record, fact.item, 1, fact.item.baseline);
                low = low.min(price);
                high = high.max(price);
            }
            if count == 0 {
                return "nobody sells it".to_string();
            }
            let price_range = if low == high {
                low.to_string()
            } else {
                format!("{low}-{high}")
            };
            let (vendor_noun, verb) = if count == 1 {
                ("eligible vendor", "sells")
            } else {
                ("eligible vendors", "sell")
            };
            format!(
                "{vendor_noun} {names} {verb} one for {price_range} gp at baseline stock; you hold {held} gp and the bank holds {banked} gp"
            )
        }
    }
}

/// The bank clause of a block message, without its row count.
fn bank_seen_phrase(bank: BankSeen, now: std::time::SystemTime) -> String {
    let age = std::time::UNIX_EPOCH
        .checked_add(std::time::Duration::from_secs(bank.observed_at_unix))
        .and_then(|seen| now.duration_since(seen).ok());
    // A bank carried across a relog or loaded from the saved hint is not
    // "this session"; the status line labels it "Hint (saved)".
    let source = match bank.origin {
        Origin::Session | Origin::Unknown => "this session",
        Origin::Hint => "from a saved hint",
    };
    let seen = match age.map(|age| age.as_secs() / 60) {
        Some(0) => format!("less than a minute ago {source}"),
        Some(1) => format!("1 minute ago {source}"),
        Some(minutes) => format!("{minutes} minutes ago {source}"),
        None => source.to_string(),
    };
    format!("in the bank as seen {seen}")
}

/// Replace the bank clause of a retained block message after the bank memory's
/// provenance changes. A relog keeps the same observation but relabels it a
/// saved hint, so a parked reason must not keep claiming "this session".
pub(crate) fn rebank_message(
    message: &Arc<str>,
    bank: BankSeen,
    now: std::time::SystemTime,
) -> Arc<str> {
    const PREFIX: &str = "in the bank as seen ";
    let Some(start) = message.find(PREFIX) else {
        return Arc::clone(message);
    };
    let suffix = format!(" ({} rows)", bank.rows);
    let Some(end) = message[start..].find(&suffix).map(|at| at + start) else {
        return Arc::clone(message);
    };
    let phrase = bank_seen_phrase(bank, now);
    if phrase.as_str() == &message[start..end] {
        return Arc::clone(message);
    }
    Arc::from(format!("{}{phrase}{}", &message[..start], &message[end..]))
}

/// Format a generated block message from borrowed facts. Authored bank
/// refusals keep their existing text and must not call this helper.
pub(crate) fn block_message(
    item: &str,
    reason: &BlockReason,
    looked: &Looked,
    vendor_line: &str,
    now: std::time::SystemTime,
    now_tick: u64,
) -> Arc<str> {
    if *reason == BlockReason::AuthoredShortage {
        unreachable!("authored refusals preserve their existing message");
    }
    let bank = match looked.bank {
        Some(bank) => format!("{} ({} rows)", bank_seen_phrase(bank, now), bank.rows),
        None => "in the observed bank".to_string(),
    };
    let waiting = match reason {
        BlockReason::AuthoredShortage => unreachable!(),
        BlockReason::SupplyMissing | BlockReason::NoVendor => {
            format!("put a {item} in the pack")
        }
        BlockReason::TooFewCoins { .. } => format!("put a {item} or coins in the pack"),
        BlockReason::VendorUnreachable => "Stop, move closer, then Start".to_string(),
        BlockReason::VendorPickUndecided => {
            "Stop, allow the route in the card, then Start".to_string()
        }
        BlockReason::NoStock {
            restock_ticks,
            revisit_due,
            ..
        } => format!(
            "the shop restocks one every {restock_ticks} ticks, revisit in {} ticks",
            revisit_due.saturating_sub(now_tick)
        ),
        BlockReason::NoSpace { slots_needed } => format!(
            "free {slots_needed} pack {}",
            if *slots_needed == 1 { "slot" } else { "slots" }
        ),
    };
    Arc::from(format!(
        "no {item} in the pack, worn, or {bank}; {vendor_line}; waiting: {waiting}"
    ))
}

/// Preserve the authored refusal text and partial-withdraw rule exactly.
pub(crate) fn session_shortage(
    actions: &[BankAction],
    partial_ok: bool,
    counts: &dyn Counts,
) -> Option<Arc<str>> {
    if counts.banked_origin() != Origin::Session {
        return None;
    }
    let mut unobtainable = Vec::new();
    let mut obtainable = false;
    for action in actions {
        match action {
            BankAction::Withdraw { item, qty } => {
                let (Some(held), Some(banked)) = (counts.held(item.id), counts.banked(item.id))
                else {
                    obtainable = true;
                    continue;
                };
                let short = qty.saturating_sub(held).max(0);
                if short > banked.max(0) {
                    let need = format!("need {qty} {}; held {held}, banked {banked}", item.name);
                    if !partial_ok {
                        return Some(Arc::from(need));
                    }
                    if banked <= 0 {
                        unobtainable.push(need);
                    } else {
                        obtainable = true;
                    }
                } else {
                    obtainable = true;
                }
            }
            BankAction::WithdrawAny { items, qty }
                if !items.iter().any(|item| {
                    counts.held(item.id).is_some_and(|held| held >= *qty)
                        || counts
                            .held(item.id)
                            .zip(counts.worn(item.id))
                            .is_some_and(|(held, worn)| held.saturating_add(worn) >= *qty)
                        || counts.banked(item.id).is_some_and(|banked| banked > 0)
                }) =>
            {
                return Some(Arc::from(format!(
                    "no permitted tier carried or banked: {}",
                    items
                        .iter()
                        .map(|item| item.name.as_ref())
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
            _ => {}
        }
    }
    (!obtainable && !unobtainable.is_empty()).then(|| Arc::from(unobtainable.join("; ")))
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod planner_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_bank::BankItem;
    use api::bank_memory::BankMemory;
    use api::snapshot::{GameSnapshot, ItemActionFamily, ItemContainer, ItemView, SnapshotView};

    const COINS: i32 = 995;
    const SCIMITAR: i32 = 1333;
    const SWORD: i32 = 1289;

    fn row(id: i32, count: i32, container: ItemContainer) -> ItemView {
        ItemView {
            def: crate::quester::families::tests::def(id, "x"),
            container,
            action_family: ItemActionFamily::Held,
            slot: 0,
            count,
            actions: Vec::new(),
            component_id: 0,
        }
    }

    fn stamp() -> api::quest_progress::EvidenceStamp {
        api::quest_progress::EvidenceStamp {
            run: api::selected::RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 1,
            sequence: 1,
        }
    }

    fn item(id: i32, name: &str) -> BankItem {
        BankItem {
            id,
            name: Arc::from(name),
        }
    }

    fn withdraw(id: i32, name: &str, qty: i32) -> BankAction {
        BankAction::Withdraw {
            item: item(id, name),
            qty,
        }
    }

    /// The coin-float shape (`withdraw` 300 coins, `partial_ok`) and the
    /// loadout tier shape against every origin (design-bank-snapshot §2.4).
    #[test]
    fn only_a_session_shortage_refuses_and_a_partial_withdraw_only_when_nothing_is_banked() {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(Vec::new(), 28);
        snapshot.seed_equipment(Vec::new());
        let float = [withdraw(COINS, "Coins", 300)];
        fn stock<'a>(snapshot: &'a GameSnapshot, memory: &'a BankMemory) -> Stock<'a> {
            SnapshotView::new(Some(snapshot), stamp())
                .with_bank_memory(Some(memory))
                .stock()
        }

        let session_empty = BankMemory::seeded(&[], Origin::Session);
        assert_eq!(
            session_shortage(&float, true, &stock(&snapshot, &session_empty)).as_deref(),
            Some("need 300 Coins; held 0, banked 0"),
            "a bank seen empty this session refuses even a partial withdraw"
        );
        let session_some = BankMemory::seeded(&[(COINS, 40)], Origin::Session);
        assert_eq!(
            session_shortage(&float, true, &stock(&snapshot, &session_some)),
            None,
            "partial_ok takes the 40 the bank holds"
        );
        assert_eq!(
            session_shortage(&float, false, &stock(&snapshot, &session_some)).as_deref(),
            Some("need 300 Coins; held 0, banked 40"),
            "an exact withdraw the session bank cannot cover refuses"
        );
        for memory in [BankMemory::seeded(&[], Origin::Hint), BankMemory::default()] {
            assert_eq!(
                session_shortage(&float, true, &stock(&snapshot, &memory)),
                None,
                "{:?}: advisory or unknown, the trip verifies",
                memory.origin()
            );
        }

        let tiers = [BankAction::WithdrawAny {
            items: Arc::from([item(SCIMITAR, "Rune scimitar"), item(SWORD, "Rune sword")]),
            qty: 1,
        }];
        assert_eq!(
            session_shortage(&tiers, false, &stock(&snapshot, &session_empty)).as_deref(),
            Some("no permitted tier carried or banked: Rune scimitar, Rune sword")
        );
        let banked_sword = BankMemory::seeded(&[(SWORD, 1)], Origin::Session);
        assert_eq!(
            session_shortage(&tiers, false, &stock(&snapshot, &banked_sword)),
            None
        );
        snapshot.seed_equipment(vec![row(SWORD, 1, ItemContainer::Equipment)]);
        assert_eq!(
            session_shortage(&tiers, false, &stock(&snapshot, &session_empty)),
            None,
            "a worn legal tier covers the slot"
        );
    }

    /// REVIEW-D4-WITHDRAW-SKIPS: Imp Catcher's `withdraw-beads` is one
    /// `partial_ok` step over four bead colours. A Session bank holding two
    /// of them takes those two; the step refuses only when no listed item is
    /// obtainable at all.
    #[test]
    fn a_partial_multi_item_withdraw_takes_what_the_session_bank_holds() {
        if !api::test_support::game_data_available() {
            return;
        }
        let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let bead = |alias: &str| {
            let item = data.item_by_alias(alias).expect("bead");
            withdraw(item.id, alias, 1)
        };
        let beads = [
            bead("red_bead"),
            bead("yellow_bead"),
            bead("black_bead"),
            bead("white_bead"),
        ];
        let id = |alias: &str| data.item_by_alias(alias).unwrap().id;
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(Vec::new(), 28);
        snapshot.seed_equipment(Vec::new());
        let stock = |memory: &BankMemory| {
            session_shortage(
                &beads,
                true,
                &SnapshotView::new(Some(&snapshot), stamp())
                    .with_bank_memory(Some(memory))
                    .stock(),
            )
        };

        let two = BankMemory::seeded(
            &[(id("red_bead"), 1), (id("white_bead"), 1)],
            Origin::Session,
        );
        assert_eq!(stock(&two), None, "two banked colours are withdrawn");
        assert_eq!(
            session_shortage(
                &beads,
                false,
                &SnapshotView::new(Some(&snapshot), stamp())
                    .with_bank_memory(Some(&two))
                    .stock(),
            )
            .as_deref(),
            Some("need 1 yellow_bead; held 0, banked 0"),
            "an exact multi-item withdraw still refuses its first shortage"
        );
        let none = BankMemory::seeded(&[], Origin::Session);
        assert_eq!(
            stock(&none).as_deref(),
            Some(
                "need 1 red_bead; held 0, banked 0; need 1 yellow_bead; held 0, banked 0; \
                 need 1 black_bead; held 0, banked 0; need 1 white_bead; held 0, banked 0"
            ),
            "with nothing obtainable the partial step refuses, naming every item"
        );
    }
}
