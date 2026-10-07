//! One bank trip for the Quester: select, walk, open, then run the bank
//! machine over each authored action (design-bank-snapshot §4 F3).
//!
//! Both the provisioner and the authored `bank`/`loadout` steps use this
//! run. A bank that is already open is reused when the Path does not pin a
//! required bank: no `Select`, no `Walk`, no `Open` — the machine acts at the
//! open table. The host observes that table into the account's bank memory
//! frame by frame, so a trip that only needs to look (`BankAction::Scan`)
//! learns the whole bank by opening it. Live verification stays inside
//! `BankMachine`.
use super::compile::StepContext;
use super::families;
use crate::bank::{BankStandAccess, Open, OpenArgs, PickKind, Select, SelectArgs};
use crate::native::walk::Walk;
use crate::native::{ActionError, ActionHandle, NativeActions, WalkOptions};
use crate::native_bank::{BankAction, BankMachine, BankReceipt, BankRequest};
use api::bank_memory::Origin;
use api::named_banks::NamedBank;
use api::selected::Truth;
use api::snapshot::WorldTile;
use api::stock::{Shortage, Stock};
use std::sync::Arc;
use std::task::Poll;

pub(super) struct BankRun {
    bank: Option<NamedBank>,
    explicit: Option<Arc<str>>,
    required_bank_missing: bool,
    /// A `Session`-origin shortage the first poll fails on, in place.
    refused: Option<Arc<str>>,
    selection: Option<ActionHandle<Select>>,
    picked: bool,
    access: Option<Arc<BankStandAccess>>,
    target: Option<WorldTile>,
    walk: Option<ActionHandle<Walk>>,
    walk_started: bool,
    opening: Option<ActionHandle<Open>>,
    open_started: bool,
    machine: Option<ActionHandle<BankMachine>>,
    actions: Arc<[BankAction]>,
    partial_ok: bool,
    index: usize,
    /// The receipt of the last action that was not a `Close`.
    last: Option<BankReceipt>,
}

/// The D4 rule for an authored withdraw (design-bank-snapshot §2.4): a
/// shortage the bank memory observed **this session** is final, so the trip
/// is refused before any walk with the counts that refuse it. A `Hint`
/// shortage is advisory and costs the one verifying trip; an `Unknown` bank
/// learns. With `partial_ok` the author accepts fewer, so the step refuses
/// only when none of its withdrawn items is obtainable: each is short with
/// nothing banked (REVIEW-D4-WITHDRAW-SKIPS; a single-item step therefore
/// refuses exactly when that item has none banked).
fn session_shortage(
    actions: &[BankAction],
    partial_ok: bool,
    stock: &Stock<'_>,
) -> Option<Arc<str>> {
    if stock.banked_origin() != Origin::Session {
        return None;
    }
    let mut unobtainable = Vec::new();
    let mut obtainable = false;
    for action in actions {
        match action {
            BankAction::Withdraw { item, qty } => match stock.plan_withdraw(item.id, *qty) {
                Err(Shortage::Short { held, banked, .. }) => {
                    let need = format!("need {qty} {}; held {held}, banked {banked}", item.name);
                    if !partial_ok {
                        return Some(Arc::from(need));
                    }
                    if banked <= 0 {
                        unobtainable.push(need);
                    } else {
                        obtainable = true;
                    }
                }
                Ok(_) | Err(Shortage::Unknown) => obtainable = true,
            },
            BankAction::WithdrawAny { items, qty } => {
                let covered = items.iter().any(|item| {
                    stock.has(item.id, *qty) == Truth::True
                        || stock.holds(item.id, *qty) == Truth::True
                        || stock.bank_has(item.id, 1) == Truth::True
                });
                if !covered {
                    return Some(Arc::from(format!(
                        "no permitted tier carried or banked: {}",
                        items
                            .iter()
                            .map(|item| item.name.as_ref())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )));
                }
            }
            _ => {}
        }
    }
    (!obtainable && !unobtainable.is_empty()).then(|| Arc::from(unobtainable.join("; ")))
}

impl BankRun {
    /// A trip to `path_bank` (or the nearest eligible bank when `None`),
    /// reusing an already open bank when `bank_required` is false.
    pub(super) fn new(
        path_bank: Option<NamedBank>,
        bank_required: bool,
        actions: Arc<[BankAction]>,
        partial_ok: bool,
        cx: &StepContext<'_, '_>,
    ) -> Self {
        let already_open = cx
            .tick
            .cx
            .snapshot()
            .bank_session()
            .is_some_and(|session| session.value.open);
        let reuse_open = already_open && !bank_required;
        let explicit: Option<Arc<str>> = if bank_required {
            path_bank.and_then(|wanted| {
                cx.banks
                    .banks()
                    .iter()
                    .find(|candidate| candidate.tile == wanted.tile)
                    .map(|candidate| Arc::from(candidate.name))
            })
        } else {
            None
        };
        let required_bank_missing = bank_required && explicit.is_none();
        let refused = session_shortage(&actions, partial_ok, &cx.tick.cx.snapshot().stock());
        Self {
            bank: if reuse_open { path_bank } else { None },
            explicit,
            required_bank_missing,
            refused,
            selection: None,
            picked: reuse_open,
            access: None,
            target: None,
            walk: None,
            walk_started: false,
            opening: None,
            open_started: false,
            machine: None,
            actions,
            partial_ok,
            index: 0,
            last: None,
        }
    }

    pub(super) fn poll(
        &mut self,
        cx: &mut StepContext<'_, '_>,
    ) -> Poll<Result<BankReceipt, ActionError>> {
        if self.required_bank_missing {
            return Poll::Ready(Err(ActionError::Unavailable(Arc::from(
                "required bank is not in the bank catalog",
            ))));
        }
        if let Some(shortage) = self.refused.take() {
            return Poll::Ready(Err(ActionError::Blocked(shortage)));
        }
        if !self.picked {
            if let Some(handle) = self.selection.as_ref() {
                match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                    Poll::Ready(Ok(selected)) => {
                        self.selection = None;
                        if selected.kind == PickKind::NoCandidate {
                            return Poll::Ready(Err(ActionError::Unavailable(Arc::from(
                                "no eligible bank",
                            ))));
                        }
                        let Some(access) = selected.access else {
                            return Poll::Ready(Err(ActionError::Unavailable(Arc::from(
                                "selected bank has no access stand",
                            ))));
                        };
                        let Some(bank) = cx
                            .banks
                            .banks()
                            .get(usize::from(selected.bank_index))
                            .copied()
                        else {
                            return Poll::Ready(Err(ActionError::Stale));
                        };
                        self.bank = Some(bank);
                        self.target = Some(selected.access_tile);
                        self.access = Some(access);
                        self.picked = true;
                    }
                }
            }
            if !self.picked {
                let Some(from) = cx.tick.cx.snapshot().here() else {
                    return Poll::Ready(Err(ActionError::Unavailable(Arc::from(
                        "player position unavailable for bank selection",
                    ))));
                };
                self.selection = Some(
                    match cx.tick.actions.begin::<Select>(
                        SelectArgs {
                            facts: Arc::clone(cx.banks),
                            from: from.value,
                            preferences: api::named_banks::BankPreferences::default(),
                            options: WalkOptions::default(),
                            explicit: self.explicit.clone(),
                        },
                        &mut cx.tick.cx,
                    ) {
                        Ok(handle) => handle,
                        Err(error) => return Poll::Ready(Err(error)),
                    },
                );
                return Poll::Pending;
            }
        }

        if let Some(handle) = self.walk.as_ref() {
            match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(receipt)) => {
                    if let Err(error) = receipt.into_arrival() {
                        return Poll::Ready(Err(error));
                    }
                    self.walk = None;
                }
            }
        }
        if !self.walk_started {
            self.walk_started = true;
            if let Some(target) = self.target {
                let near = cx
                    .tick
                    .cx
                    .snapshot()
                    .here()
                    .is_some_and(|here| families::reach::within(here.value, target, 0));
                if !near {
                    self.walk = Some(
                        match cx.tick.actions.begin::<Walk>(
                            families::reach::walk_request(target, 0, None, cx.required_after),
                            &mut cx.tick.cx,
                        ) {
                            Ok(handle) => handle,
                            Err(error) => return Poll::Ready(Err(error)),
                        },
                    );
                    return Poll::Pending;
                }
            }
        }

        if let Some(handle) = self.opening.as_ref() {
            match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(())) => self.opening = None,
            }
        }
        if !self.open_started {
            self.open_started = true;
            if let Some(access) = self.access.as_ref() {
                self.opening = Some(
                    match cx.tick.actions.begin::<Open>(
                        OpenArgs {
                            access: Arc::clone(access),
                        },
                        &mut cx.tick.cx,
                    ) {
                        Ok(handle) => handle,
                        Err(error) => return Poll::Ready(Err(error)),
                    },
                );
                return Poll::Pending;
            }
        }

        if let Some(handle) = self.machine.as_ref() {
            match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(receipt)) => {
                    if !matches!(self.actions.get(self.index), Some(BankAction::Close)) {
                        self.last = Some(receipt);
                    }
                    self.machine = None;
                    self.index += 1;
                }
            }
        }
        if let Some(action) = self.actions.get(self.index).cloned() {
            self.machine = Some(
                match cx.tick.actions.begin::<BankMachine>(
                    BankRequest {
                        bank: self.bank,
                        action,
                        partial_ok: self.partial_ok,
                    },
                    &mut cx.tick.cx,
                ) {
                    Ok(handle) => handle,
                    Err(error) => return Poll::Ready(Err(error)),
                },
            );
            return Poll::Pending;
        }
        Poll::Ready(Ok(self
            .last
            .take()
            .unwrap_or(BankReceipt { complete: true })))
    }

    pub(super) fn cancel(&mut self, actions: &mut NativeActions) {
        if let Some(handle) = self.selection.take() {
            actions.cancel(handle);
        }
        if let Some(handle) = self.walk.take() {
            actions.cancel(handle);
        }
        if let Some(handle) = self.opening.take() {
            actions.cancel(handle);
        }
        if let Some(handle) = self.machine.take() {
            actions.cancel(handle);
        }
    }
}

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
