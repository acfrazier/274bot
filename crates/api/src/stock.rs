//! The planning kernel over a frame's item pages (design-bank-snapshot §2).
//!
//! One exact-id count, one slot policy, one tri-state. Every page is a
//! borrow and `None` until the host has posted it; the bank is `None` until
//! the account's memory is known. A note is another obj id (§2.2), so every
//! count here is noted-exact by construction and there is no by-name count:
//! a by-name caller resolves ids first and picks the form with [`Stock::forms`].
//!
//! `script::bank::ops` stays the transfer kernel over borrowed `BankRow`s
//! (compat rows included); this is the read side every planner shares.

use crate::bank_memory::{BankMemory, Origin};
use crate::obj_names::ObjNames;
use crate::selected::Truth;
use crate::snapshot::{GameSnapshot, ItemView};

/// A frame's item facts, all borrowed. Each page is `None` until posted.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stock<'a> {
    /// The posted pack (`SnapshotView::inventory`).
    pub pack: Option<&'a [ItemView]>,
    /// The pack's slot count once posted (`SnapshotView::inventory_capacity`).
    pub capacity: Option<i32>,
    /// The posted worn table (`SnapshotView::equipment`).
    pub worn: Option<&'a [ItemView]>,
    /// The account's bank memory (`SnapshotView::bank_memory`).
    pub bank: Option<&'a BankMemory>,
}

/// One id's withdraw toward a held target: `take` more from the bank
/// reaches `target` held; `0` when the pack already holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WithdrawPlan {
    pub target: i32,
    pub take: i32,
}

/// Why one id has no withdraw plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortage {
    /// The pack is unposted or the bank was never observed.
    Unknown,
    /// The known bank cannot cover the gap. `origin` says how final that is
    /// (design-bank-snapshot §2.4): `Hint` is advisory, `Session` is final.
    Short {
        held: i32,
        banked: i32,
        origin: Origin,
    },
}

/// The obj ids one item can take: the bank stores `unnoted`, a note
/// withdrawal lands as `noted`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemForms {
    pub unnoted: i32,
    pub noted: Option<i32>,
}

/// Sum of `id` over posted rows, exact id. Posted rows are never empty;
/// a synthetic non-positive count contributes nothing.
fn count_id(rows: &[ItemView], id: i32) -> i32 {
    rows.iter()
        .filter(|row| row.def.id == id)
        .fold(0_i32, |total, row| total.saturating_add(row.count.max(0)))
}

/// Rows that take a slot: every row with a positive count.
fn occupied_rows(rows: &[ItemView]) -> i32 {
    i32::try_from(rows.iter().filter(|row| row.count > 0).count()).unwrap_or(i32::MAX)
}

fn truth(count: Option<i32>, qty: i32) -> Truth {
    match count {
        Some(count) if count >= qty => Truth::True,
        Some(_) => Truth::False,
        None => Truth::Unknown,
    }
}

impl<'a> Stock<'a> {
    /// The pack and worn pages of `snapshot` as posted, with no bank. This
    /// applies the page gates (`inventory_size > 0`, worn table posted) and
    /// nothing else; `SnapshotView::stock` adds the in-game gate and the
    /// bank memory.
    pub fn pages(snapshot: &'a GameSnapshot) -> Self {
        let capacity = snapshot.inventory_size();
        let posted = capacity > 0;
        Self {
            pack: posted.then(|| snapshot.inventory()),
            capacity: posted.then_some(capacity),
            worn: snapshot.equipment_posted().then(|| snapshot.equipment()),
            bank: None,
        }
    }

    // ---- counts: `None` is an unposted page / an unknown bank -------------

    /// Pack sum of the exact id.
    pub fn held(&self, id: i32) -> Option<i32> {
        self.pack.map(|rows| count_id(rows, id))
    }

    /// Worn sum of the exact id.
    pub fn worn(&self, id: i32) -> Option<i32> {
        self.worn.map(|rows| count_id(rows, id))
    }

    /// Held plus worn.
    pub fn carried(&self, id: i32) -> Option<i32> {
        Some(self.held(id)?.saturating_add(self.worn(id)?))
    }

    /// The bank memory's count: `None` while the bank is unknown, `Some(0)`
    /// for an id a known bank does not hold.
    pub fn banked(&self, id: i32) -> Option<i32> {
        self.bank?.count(id)
    }

    pub fn banked_origin(&self) -> Origin {
        self.bank.map_or(Origin::Unknown, BankMemory::origin)
    }

    /// Carried plus banked; `None` if any side is.
    pub fn total(&self, id: i32) -> Option<i32> {
        Some(self.carried(id)?.saturating_add(self.banked(id)?))
    }

    // ---- predicates: tri-state, never a guess -----------------------------

    /// Carried at least `qty`: kit, tools and quest items count worn.
    pub fn has(&self, id: i32, qty: i32) -> Truth {
        truth(self.carried(id), qty)
    }

    /// Held at least `qty`: consumables and hand-overs come from the pack.
    pub fn holds(&self, id: i32, qty: i32) -> Truth {
        truth(self.held(id), qty)
    }

    /// Any of `ids` worn.
    pub fn wears_any(&self, ids: &[i32]) -> Truth {
        match self.worn {
            Some(rows)
                if rows
                    .iter()
                    .any(|row| row.count > 0 && ids.contains(&row.def.id)) =>
            {
                Truth::True
            }
            Some(_) => Truth::False,
            None => Truth::Unknown,
        }
    }

    /// The known bank holds at least `qty`.
    pub fn bank_has(&self, id: i32, qty: i32) -> Truth {
        truth(self.banked(id), qty)
    }

    // ---- held-target planning --------------------------------------------

    /// What the pack is short of a final held count of `target`.
    pub fn shortfall(&self, id: i32, target: i32) -> Option<i32> {
        self.held(id).map(|held| target.saturating_sub(held).max(0))
    }

    /// The withdraw that reaches `target` held, or why there is none. A
    /// `Short` carries the bank's origin so the caller applies
    /// design-bank-snapshot §2.4 without a second lookup.
    pub fn plan_withdraw(&self, id: i32, target: i32) -> Result<WithdrawPlan, Shortage> {
        let (Some(held), Some(banked)) = (self.held(id), self.banked(id)) else {
            return Err(Shortage::Unknown);
        };
        let short = target.saturating_sub(held).max(0);
        let take = short.min(banked.max(0));
        if take < short {
            return Err(Shortage::Short {
                held,
                banked,
                origin: self.banked_origin(),
            });
        }
        Ok(WithdrawPlan { target, take })
    }

    // ---- slots: the one policy -------------------------------------------

    /// Taken pack slots: rows with a positive count.
    pub fn occupied(&self) -> Option<i32> {
        self.pack.map(occupied_rows)
    }

    pub fn free_slots(&self) -> Option<i32> {
        Some(self.capacity?.saturating_sub(self.occupied()?).max(0))
    }

    /// `None` while the capacity is unposted: an unknown capacity is neither
    /// full nor not full (the `ops::pack_full` guard).
    pub fn pack_full(&self) -> Option<bool> {
        Some(self.occupied()? >= self.capacity?)
    }

    /// Slots `count` of one item takes: one for any positive stackable
    /// count, one per item otherwise.
    pub fn slots_for(count: i32, stackable: bool) -> i32 {
        let count = count.max(0);
        if stackable {
            i32::from(count > 0)
        } else {
            count
        }
    }

    /// Slots a withdraw from `held` toward `target` adds to the pack, with
    /// the bank able to give at most `banked`.
    pub fn incoming_slots(held: i32, target: i32, stackable: bool, banked: i32) -> i32 {
        let take = target.saturating_sub(held).min(banked.max(0));
        Self::slots_for(held.saturating_add(take), stackable)
            .saturating_sub(Self::slots_for(held, stackable))
    }

    // ---- forms --------------------------------------------------------------

    /// Both obj ids of the item `id` names, whichever form `id` is. An id
    /// the table does not know is its own unnoted form with no note.
    pub fn forms(names: &ObjNames, id: i32) -> ItemForms {
        match names.item(id) {
            Some(def) if def.noted => ItemForms {
                unnoted: def.certificate_link,
                noted: Some(id),
            },
            Some(def) => ItemForms {
                unnoted: id,
                noted: (def.certificate_link >= 0).then_some(def.certificate_link),
            },
            None => ItemForms {
                unnoted: id,
                noted: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obj_names::ItemDefView;
    use crate::quest_progress::EvidenceStamp;
    use crate::selected::RunKey;
    use crate::snapshot::{ItemActionFamily, ItemContainer, SnapshotView};
    use client::config::ObjType;

    const LOBSTER: i32 = 379;
    const LOBSTER_NOTE: i32 = 380;
    const SCIMITAR: i32 = 1333;
    const COINS: i32 = 995;

    fn row(container: ItemContainer, id: i32, noted: bool, count: i32, slot: i32) -> ItemView {
        ItemView {
            def: ItemDefView {
                id,
                name: Some("fixture".into()),
                stackable: id == COINS || noted,
                members: false,
                base_value: 0,
                noted,
                certificate_link: -1,
                certificate_template: -1,
            },
            container,
            action_family: ItemActionFamily::Held,
            slot,
            count,
            actions: Vec::new(),
            component_id: 3214,
        }
    }

    fn held(id: i32, count: i32, slot: i32) -> ItemView {
        row(ItemContainer::Inventory, id, false, count, slot)
    }

    fn worn(id: i32, count: i32, slot: i32) -> ItemView {
        row(ItemContainer::Equipment, id, false, count, slot)
    }

    fn stamp() -> EvidenceStamp {
        EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 1,
            sequence: 1,
        }
    }

    #[test]
    fn counts_are_exact_id_so_a_noted_twin_never_folds_in() {
        let pack = [
            held(LOBSTER, 3, 0),
            row(ItemContainer::Inventory, LOBSTER_NOTE, true, 12, 1),
            held(LOBSTER, 2, 2),
            held(COINS, 0, 3),
        ];
        let stock = Stock {
            pack: Some(&pack),
            capacity: Some(28),
            worn: Some(&[]),
            bank: None,
        };
        assert_eq!(stock.held(LOBSTER), Some(5));
        assert_eq!(stock.held(LOBSTER_NOTE), Some(12));
        assert_eq!(
            stock.held(COINS),
            Some(0),
            "a non-positive row counts nothing"
        );
        assert_eq!(stock.held(SCIMITAR), Some(0));
        assert_eq!(stock.holds(LOBSTER, 5), Truth::True);
        assert_eq!(stock.holds(LOBSTER, 6), Truth::False);
        assert_eq!(
            stock.occupied(),
            Some(3),
            "the empty coin row takes no slot"
        );
    }

    #[test]
    fn worn_is_split_from_held_and_folds_into_carried() {
        let pack = [held(SCIMITAR, 1, 0), held(LOBSTER, 5, 1)];
        let kit = [worn(SCIMITAR, 1, 3)];
        let stock = Stock {
            pack: Some(&pack),
            capacity: Some(28),
            worn: Some(&kit),
            bank: None,
        };
        assert_eq!(stock.held(SCIMITAR), Some(1));
        assert_eq!(stock.worn(SCIMITAR), Some(1));
        assert_eq!(stock.carried(SCIMITAR), Some(2));
        assert_eq!(stock.has(SCIMITAR, 2), Truth::True);
        assert_eq!(stock.holds(SCIMITAR, 2), Truth::False, "worn is not held");
        assert_eq!(stock.has(LOBSTER, 5), Truth::True);
        assert_eq!(stock.worn(LOBSTER), Some(0));
        assert_eq!(stock.wears_any(&[LOBSTER, SCIMITAR]), Truth::True);
        assert_eq!(stock.wears_any(&[LOBSTER]), Truth::False);
        assert_eq!(stock.shortfall(LOBSTER, 8), Some(3));
        assert_eq!(stock.shortfall(LOBSTER, 5), Some(0));
        assert_eq!(
            stock.shortfall(SCIMITAR, 2),
            Some(1),
            "a worn tier does not fill a held target"
        );
    }

    #[test]
    fn unposted_pages_are_unknown_never_empty() {
        let none = Stock::default();
        assert_eq!(none.held(LOBSTER), None);
        assert_eq!(none.worn(LOBSTER), None);
        assert_eq!(none.carried(LOBSTER), None);
        assert_eq!(none.holds(LOBSTER, 1), Truth::Unknown);
        assert_eq!(none.has(LOBSTER, 1), Truth::Unknown);
        assert_eq!(none.wears_any(&[SCIMITAR]), Truth::Unknown);
        assert_eq!(none.shortfall(LOBSTER, 1), None);
        assert_eq!(none.occupied(), None);
        assert_eq!(none.free_slots(), None);
        assert_eq!(none.pack_full(), None);

        let pack = [held(LOBSTER, 1, 0)];
        let pack_only = Stock {
            pack: Some(&pack),
            capacity: None,
            worn: None,
            bank: None,
        };
        assert_eq!(pack_only.holds(LOBSTER, 1), Truth::True);
        assert_eq!(
            pack_only.has(LOBSTER, 1),
            Truth::Unknown,
            "no worn page: carried is unknown"
        );
        assert_eq!(pack_only.occupied(), Some(1));
        assert_eq!(
            pack_only.pack_full(),
            None,
            "an unposted capacity is neither full nor not"
        );
        assert_eq!(pack_only.free_slots(), None);
    }

    #[test]
    fn no_memory_is_an_unknown_bank() {
        let pack = [held(LOBSTER, 1, 0)];
        let stock = Stock {
            pack: Some(&pack),
            capacity: Some(28),
            worn: Some(&[]),
            bank: None,
        };
        assert_eq!(stock.banked(LOBSTER), None);
        assert_eq!(stock.banked_origin(), Origin::Unknown);
        assert_eq!(stock.bank_has(LOBSTER, 1), Truth::Unknown);
        assert_eq!(stock.total(LOBSTER), None);
        assert_eq!(stock.plan_withdraw(LOBSTER, 5), Err(Shortage::Unknown));

        let unknown = BankMemory::default();
        let stock = Stock {
            bank: Some(&unknown),
            ..stock
        };
        assert_eq!(stock.banked(LOBSTER), None);
        assert_eq!(stock.plan_withdraw(LOBSTER, 5), Err(Shortage::Unknown));

        let known = BankMemory::seeded(&[(COINS, 100)], Origin::Session);
        let unposted = Stock {
            pack: None,
            capacity: None,
            worn: None,
            bank: Some(&known),
        };
        assert_eq!(
            unposted.banked(COINS),
            Some(100),
            "the bank is known without a frame"
        );
        assert_eq!(unposted.bank_has(COINS, 100), Truth::True);
        assert_eq!(
            unposted.plan_withdraw(COINS, 5),
            Err(Shortage::Unknown),
            "no pack: no plan"
        );
    }

    #[test]
    fn withdraw_plans_are_held_target_and_a_shortage_carries_its_origin() {
        let pack = [held(LOBSTER, 3, 0)];
        for origin in [Origin::Hint, Origin::Session] {
            let memory = BankMemory::seeded(&[(LOBSTER, 4), (COINS, 50)], origin);
            let stock = Stock {
                pack: Some(&pack),
                capacity: Some(28),
                worn: Some(&[]),
                bank: Some(&memory),
            };
            assert_eq!(stock.banked_origin(), origin);
            assert_eq!(stock.total(LOBSTER), Some(7));
            assert_eq!(stock.bank_has(LOBSTER, 4), Truth::True);
            assert_eq!(stock.bank_has(LOBSTER, 5), Truth::False);
            assert_eq!(
                stock.plan_withdraw(LOBSTER, 5),
                Ok(WithdrawPlan { target: 5, take: 2 }),
                "3 held toward 5 takes 2, not 5"
            );
            assert_eq!(
                stock.plan_withdraw(LOBSTER, 3),
                Ok(WithdrawPlan { target: 3, take: 0 }),
                "the target is already held"
            );
            assert_eq!(
                stock.plan_withdraw(LOBSTER, 2),
                Ok(WithdrawPlan { target: 2, take: 0 })
            );
            assert_eq!(
                stock.plan_withdraw(LOBSTER, 8),
                Err(Shortage::Short {
                    held: 3,
                    banked: 4,
                    origin
                })
            );
            assert_eq!(
                stock.plan_withdraw(SCIMITAR, 1),
                Err(Shortage::Short {
                    held: 0,
                    banked: 0,
                    origin
                }),
                "a known bank without the id is a shortage, not unknown"
            );
            assert_eq!(
                stock.plan_withdraw(COINS, 50),
                Ok(WithdrawPlan {
                    target: 50,
                    take: 50
                })
            );
        }
    }

    #[test]
    fn slot_policy_matches_the_replaced_copies() {
        assert_eq!(Stock::slots_for(0, true), 0);
        assert_eq!(Stock::slots_for(1, true), 1);
        assert_eq!(Stock::slots_for(3_400, true), 1);
        assert_eq!(Stock::slots_for(0, false), 0);
        assert_eq!(Stock::slots_for(7, false), 7);
        assert_eq!(Stock::slots_for(-2, true), 0);
        assert_eq!(Stock::slots_for(-2, false), 0);

        // provision `incoming_slots(pack, target, stackable, bank_count)`.
        assert_eq!(Stock::incoming_slots(0, 5, false, 10), 5);
        assert_eq!(Stock::incoming_slots(3, 5, false, 10), 2);
        assert_eq!(
            Stock::incoming_slots(3, 5, false, 1),
            1,
            "the bank caps the take"
        );
        assert_eq!(
            Stock::incoming_slots(0, 5, false, 0),
            0,
            "an empty bank adds nothing"
        );
        assert_eq!(
            Stock::incoming_slots(0, 5, true, 10),
            1,
            "a new stack is one slot"
        );
        assert_eq!(
            Stock::incoming_slots(3, 5, true, 10),
            0,
            "topping a stack is free"
        );
        assert_eq!(Stock::incoming_slots(5, 5, false, 10), 0);
        assert_eq!(
            Stock::incoming_slots(7, 5, false, 10),
            -2,
            "over target is the verbatim provision result; callers only ask below target"
        );
        assert_eq!(
            Stock::incoming_slots(0, 5, false, -4),
            0,
            "a negative bank count is empty"
        );
        assert_eq!(Stock::incoming_slots(i32::MAX, i32::MAX, false, 1), 0);

        // `ops::pack_full` on a posted pack.
        let full: Vec<ItemView> = (0..28).map(|slot| held(LOBSTER, 1, slot)).collect();
        let stock = Stock {
            pack: Some(&full),
            capacity: Some(28),
            worn: None,
            bank: None,
        };
        assert_eq!(stock.occupied(), Some(28));
        assert_eq!(stock.free_slots(), Some(0));
        assert_eq!(stock.pack_full(), Some(true));
        let stock = Stock {
            pack: Some(&full[..27]),
            ..stock
        };
        assert_eq!(stock.free_slots(), Some(1));
        assert_eq!(stock.pack_full(), Some(false));
        let stock = Stock {
            capacity: Some(27),
            ..stock
        };
        assert_eq!(stock.pack_full(), Some(true));
        assert_eq!(stock.free_slots(), Some(0));
    }

    #[test]
    fn snapshot_view_serves_the_pages_under_its_gates() {
        let mut snapshot = GameSnapshot::new();
        let memory = BankMemory::seeded(&[(LOBSTER, 9)], Origin::Hint);
        let offline = SnapshotView::new(Some(&snapshot), stamp()).with_bank_memory(Some(&memory));
        let stock = offline.stock();
        assert_eq!(stock.held(LOBSTER), None, "not in game: no pack");
        assert_eq!(stock.worn(LOBSTER), None);
        assert_eq!(stock.capacity, None);
        assert_eq!(
            stock.banked(LOBSTER),
            Some(9),
            "the memory outlives the frame"
        );
        assert!(offline.bank_memory().is_some());

        snapshot.seed_ingame(2);
        let no_pages = SnapshotView::new(Some(&snapshot), stamp()).stock();
        assert_eq!(no_pages.held(LOBSTER), None, "inventory_size 0 is unposted");
        assert_eq!(
            no_pages.worn(LOBSTER),
            None,
            "the worn table has not posted"
        );
        assert_eq!(no_pages.banked(LOBSTER), None);
        assert_eq!(no_pages.banked_origin(), Origin::Unknown);

        snapshot.seed_inventory(vec![held(LOBSTER, 2, 0)], 28);
        snapshot.seed_equipment(vec![worn(SCIMITAR, 1, 3)]);
        let view = SnapshotView::new(Some(&snapshot), stamp()).with_bank_memory(Some(&memory));
        let stock = view.stock();
        assert_eq!(stock.held(LOBSTER), Some(2));
        assert_eq!(stock.worn(SCIMITAR), Some(1));
        assert_eq!(stock.capacity, Some(28));
        assert_eq!(stock.free_slots(), Some(27));
        assert_eq!(stock.total(LOBSTER), Some(11));
        assert_eq!(stock.banked_origin(), Origin::Hint);
        assert_eq!(view.inventory().map(|rows| rows.value.len()), Some(1));

        let pages = Stock::pages(&snapshot);
        assert_eq!(pages.held(LOBSTER), Some(2));
        assert_eq!(pages.pack_full(), Some(false));
        assert!(pages.bank.is_none());

        let empty = SnapshotView::new(None, stamp()).stock();
        assert_eq!(empty.held(LOBSTER), None);
        assert_eq!(empty.pack_full(), None);
    }

    #[test]
    fn forms_pair_an_item_with_its_note_either_way_round() {
        let objs = [
            ObjType {
                id: LOBSTER,
                name: "Lobster".into(),
                ..ObjType::default()
            },
            ObjType {
                id: LOBSTER_NOTE,
                name: "Lobster".into(),
                certlink: LOBSTER,
                certtemplate: 799,
                ..ObjType::default()
            },
            ObjType {
                id: COINS,
                name: "Coins".into(),
                stackable: true,
                ..ObjType::default()
            },
        ];
        let names = ObjNames::from_objs(&objs);
        let lobster = ItemForms {
            unnoted: LOBSTER,
            noted: Some(LOBSTER_NOTE),
        };
        assert_eq!(Stock::forms(&names, LOBSTER), lobster);
        assert_eq!(Stock::forms(&names, LOBSTER_NOTE), lobster);
        assert_eq!(
            Stock::forms(&names, COINS),
            ItemForms {
                unnoted: COINS,
                noted: None
            }
        );
        assert_eq!(
            Stock::forms(&names, 4_000),
            ItemForms {
                unnoted: 4_000,
                noted: None
            },
            "an unknown id is its own unnoted form"
        );
    }

    #[test]
    fn frame_borrows_stay_a_few_words() {
        let stock = std::mem::size_of::<Stock<'_>>();
        let view = std::mem::size_of::<SnapshotView<'_>>();
        let memory = std::mem::size_of::<BankMemory>();
        eprintln!("size_of Stock={stock} SnapshotView={view} BankMemory={memory}");
        assert!(stock <= 56, "Stock is {stock} bytes");
        assert!(view <= 72, "SnapshotView is {view} bytes");
    }
}
