//! Pure acquisition decisions over borrowed counts.

use super::*;
use api::stock::Stock;

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

pub struct Planner<'a> {
    counts: &'a dyn Counts,
    plan: AcquirePlan,
    first_withdrawal: Option<NeedObservation>,
    scan: Option<NeedObservation>,
    blocked: Option<NeedObservation>,
    child: Option<NeedObservation>,
    next_index: usize,
}

impl<'a> Planner<'a> {
    pub fn new(counts: &'a dyn Counts, _world: WorldFacts<'_>) -> Self {
        Self {
            counts,
            plan: AcquirePlan::empty(),
            first_withdrawal: None,
            scan: None,
            blocked: None,
            child: None,
            next_index: 0,
        }
    }

    pub fn require(&mut self, need: &Need) -> Result<(), ActionError> {
        let index = u8::try_from(self.next_index)
            .map_err(|_| ActionError::Unavailable(Arc::from("acquisition has too many needs")))?;
        self.next_index += 1;
        if need.target <= 0 {
            return Ok(());
        }
        if need.sources.contains(SourceSet::SHOP) {
            return Err(ActionError::Unavailable(Arc::from(
                "shop acquisition is not available",
            )));
        }
        let id = acceptable_ids(need)
            .find(|&id| present(self.counts, need, id) >= need.target)
            .or_else(|| acceptable_ids(need).find(|&id| present(self.counts, need, id) > 0))
            .or_else(|| {
                need.sources
                    .contains(SourceSet::BANK)
                    .then(|| {
                        acceptable_ids(need).find(|&id| self.counts.banked(id).unwrap_or(0) > 0)
                    })
                    .flatten()
            })
            .unwrap_or(need.item);
        let held = held(self.counts, need, id);
        let present = present(self.counts, need, id);
        let banked = need
            .sources
            .contains(SourceSet::BANK)
            .then(|| self.counts.banked(id))
            .flatten();
        if need.presence == Presence::Wielded && held > 0 && worn(self.counts, need, id) == 0 {
            self.add_wear(id)?;
        }
        if present >= need.target {
            return Ok(());
        }
        let detail = NeedObservation {
            id,
            item: Arc::clone(&need.name),
            need: need.target,
            pack: present,
            bank: banked,
            index,
        };
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
            self.add_withdrawal(id, &need.name, held_target)?;
            if self.first_withdrawal.is_none() {
                self.first_withdrawal = Some(detail.clone());
            }
            if need.presence == Presence::Wielded && worn(self.counts, need, id) == 0 {
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
        } else if reach >= need.minimum || need.kind == NeedKind::Optional {
            self.plan.looked.short_of_target = self.plan.looked.short_of_target.saturating_add(1);
        } else if self.blocked.is_none() {
            self.blocked = Some(detail);
            self.plan.looked = Looked {
                short_of_target: self.plan.looked.short_of_target,
                ..looked(self.counts, need, id)
            };
        }
        Ok(())
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
                let message = supply_message(&detail.item, &self.plan.looked);
                return AcquireVerdict::Blocked(AcquireBlock {
                    item: detail.item,
                    need: detail.index,
                    reason: BlockReason::SupplyMissing,
                    looked: self.plan.looked,
                    message,
                });
            }
        }
        if self.plan.withdrawal_count == 0
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

    fn add_withdrawal(&mut self, id: i32, name: &Arc<str>, target: i32) -> Result<(), ActionError> {
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
        self.plan.withdrawals[self.plan.withdrawal_count] = Some(Withdrawal {
            id,
            name: Arc::clone(name),
            target,
        });
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
        planner.require(need)?;
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

fn worn(counts: &dyn Counts, need: &Need, id: i32) -> i32 {
    if need.presence != Presence::Held && need.sources.contains(SourceSet::WORN) {
        counts.worn(id).unwrap_or(0).max(0)
    } else {
        0
    }
}

pub(crate) fn present(counts: &dyn Counts, need: &Need, id: i32) -> i32 {
    held(counts, need, id).saturating_add(worn(counts, need, id))
}

fn looked(counts: &dyn Counts, need: &Need, id: i32) -> Looked {
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

fn supply_message(item: &str, looked: &Looked) -> Arc<str> {
    let bank = match looked.bank {
        Some(bank) => format!(
            "in the bank as seen at {} this session ({} rows)",
            local_clock(bank.observed_at_unix),
            bank.rows
        ),
        None => "in the observed bank".to_string(),
    };
    Arc::from(format!("no {item} in the pack, worn, or {bank}; nobody sells it; waiting: put a {item} in the pack"))
}

#[cfg(unix)]
fn local_clock(seconds: u64) -> String {
    let Ok(seconds) = libc::time_t::try_from(seconds) else {
        return "unknown".to_string();
    };
    let mut calendar = std::mem::MaybeUninit::<libc::tm>::uninit();
    // SAFETY: localtime_r reads the valid time value and initializes the owned
    // output when it returns a non-null pointer. No borrowed output escapes.
    let result = unsafe { libc::localtime_r(&seconds, calendar.as_mut_ptr()) };
    if result.is_null() {
        return "unknown".to_string();
    }
    // SAFETY: the successful localtime_r call initialized calendar above.
    let calendar = unsafe { calendar.assume_init() };
    format!("{:02}:{:02}", calendar.tm_hour, calendar.tm_min)
}

#[cfg(windows)]
fn local_clock(seconds: u64) -> String {
    #[repr(C)]
    struct Calendar {
        second: i32,
        minute: i32,
        hour: i32,
        day: i32,
        month: i32,
        year: i32,
        weekday: i32,
        yearday: i32,
        daylight: i32,
    }
    extern "C" {
        fn _localtime64_s(result: *mut Calendar, seconds: *const i64) -> i32;
    }
    let Ok(seconds) = i64::try_from(seconds) else {
        return "unknown".to_string();
    };
    let mut calendar = std::mem::MaybeUninit::<Calendar>::uninit();
    // SAFETY: the CRT writes its documented nine-int tm into owned storage.
    // The time input is valid, and success initializes every field.
    if unsafe { _localtime64_s(calendar.as_mut_ptr(), &seconds) } != 0 {
        return "unknown".to_string();
    }
    // SAFETY: the successful CRT call initialized calendar above.
    let calendar = unsafe { calendar.assume_init() };
    format!("{:02}:{:02}", calendar.hour, calendar.minute)
}

#[cfg(not(any(unix, windows)))]
fn local_clock(_seconds: u64) -> String {
    "unknown".to_string()
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
