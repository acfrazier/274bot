use crate::observed::{EntityRow, ItemRow, SceneRow};
use api::snapshot::WorldTile;
use script::bank::ops::{op_index, BankRow, LocRow, NpcRow, OpLabels};

/// The host posts a native action hole as an empty label in its slot, so
/// the 1-based positions match the native row.
impl BankRow for ItemRow {
    fn id(&self) -> i32 {
        self.id
    }

    fn count(&self) -> i32 {
        self.count
    }

    fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    fn slot(&self) -> Option<i32> {
        self.slot
    }

    fn component_id(&self) -> Option<i32> {
        self.component_id
    }

    fn op_len(&self) -> i32 {
        i32::try_from(self.ops.len()).unwrap_or(i32::MAX)
    }

    fn op_label(&self, one_based: i32) -> Option<&str> {
        self.ops
            .get(op_index(one_based)?)
            .map(|label| &**label)
            .filter(|label| !label.is_empty())
    }
}

impl OpLabels for SceneRow {
    fn op_len(&self) -> i32 {
        i32::try_from(self.actions.len()).unwrap_or(i32::MAX)
    }

    fn op_label(&self, one_based: i32) -> Option<&str> {
        self.actions.get(op_index(one_based)?).map(|label| &**label)
    }
}

impl LocRow for SceneRow {
    fn id(&self) -> i32 {
        self.id
    }

    fn tile(&self) -> WorldTile {
        WorldTile {
            x: self.x,
            z: self.z,
            level: self.level,
        }
    }

    fn distance(&self) -> i32 {
        self.distance
    }

    fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

impl OpLabels for EntityRow {
    fn op_len(&self) -> i32 {
        i32::try_from(self.actions.len()).unwrap_or(i32::MAX)
    }

    fn op_label(&self, one_based: i32) -> Option<&str> {
        self.actions.get(op_index(one_based)?).map(|label| &**label)
    }
}

impl NpcRow for EntityRow {
    fn tile(&self) -> WorldTile {
        WorldTile {
            x: self.x,
            z: self.z,
            level: self.level,
        }
    }

    fn distance(&self) -> i32 {
        self.distance
    }

    fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::obj_names::ItemDefView;
    use api::snapshot::{ItemActionFamily, ItemContainer, ItemView};
    use script::bank::ops::{
        action_op, deposit_capacity_next, deposit_click, deposit_next, deposit_req, withdraw_click,
        DepositClick, DepositKind, DepositRequest, DepositScan, DepositSpec, WithdrawGoal,
    };
    use script::interact::InteractReq;
    use std::sync::Arc;

    const COINS: i32 = 995;
    const TROUT: i32 = 333;

    fn native(
        id: i32,
        name: &str,
        count: i32,
        slot: i32,
        component_id: i32,
        actions: &[Option<&str>],
    ) -> ItemView {
        ItemView {
            def: ItemDefView {
                id,
                name: Some(name.to_owned()),
                stackable: false,
                members: false,
                base_value: 0,
                noted: false,
                certificate_link: -1,
                certificate_template: -1,
            },
            container: ItemContainer::Bank,
            action_family: ItemActionFamily::Component,
            slot,
            count,
            actions: actions
                .iter()
                .map(|action| action.map(str::to_owned))
                .collect(),
            component_id,
        }
    }

    /// A pack row: no ops, only id and count matter to the kernel.
    fn held(id: i32, count: i32) -> ItemView {
        native(id, "Held", count, 0, 3214, &[])
    }

    fn held_at(id: i32, count: i32, slot: i32) -> ItemView {
        ItemView {
            slot,
            ..held(id, count)
        }
    }

    fn exact(final_count: i32, available: i32, baseline: i32) -> WithdrawGoal {
        WithdrawGoal::exact_at_least(
            COINS,
            Arc::from("Coins"),
            COINS,
            final_count,
            available,
            baseline,
            9,
        )
    }

    /// The isolate row the host posts for the same facts: an action hole is an
    /// empty label in its slot.
    fn compat(
        id: i32,
        name: &str,
        count: i32,
        slot: Option<i32>,
        component_id: Option<i32>,
        actions: &[Option<&str>],
    ) -> ItemRow {
        use std::rc::Rc;
        ItemRow {
            id,
            count,
            name: Some(Rc::from(name)),
            ops: actions
                .iter()
                .map(|action| Rc::from(action.unwrap_or_default()))
                .collect::<crate::observed::Ops>(),
            component_id,
            slot,
            ..ItemRow::default()
        }
    }

    /// P-row: one kernel decides on a native row and on the isolate row the
    /// host posts for it, holes and positions included. Nothing converts one
    /// representation into the other.
    #[test]
    fn p_row_both_row_types_make_the_same_decisions() {
        let withdraw_ops = [
            Some("Withdraw-1"),
            Some("Withdraw-5"),
            None,
            Some("Withdraw-X"),
        ];
        let bank_native = native(COINS, "Coins", 50, 4, 5382, &withdraw_ops);
        let bank_compat = compat(COINS, "Coins", 50, Some(4), Some(5382), &withdraw_ops);
        assert_eq!(action_op(&bank_native, "Withdraw-X"), Some(4));
        assert_eq!(action_op(&bank_compat, "Withdraw-X"), Some(4));
        assert_eq!(action_op(&bank_native, "withdraw x"), Some(4));
        let goal = exact(7, 50, 0);
        for remaining in [1, 5, 7, 10, 60] {
            assert_eq!(
                withdraw_click(&bank_native, remaining, &goal),
                withdraw_click(&bank_compat, remaining, &goal),
                "remaining {remaining}"
            );
        }
        assert_eq!(
            withdraw_click(&bank_compat, 7, &goal),
            Some(InteractReq::WithdrawX {
                name: "Coins".into(),
                count: 7,
                bank_item_id: COINS,
                lands_as_id: COINS,
                action: "Withdraw-X".into(),
                bank_generation: 9,
            })
        );

        // A hole before the bulk op: the sweep and the labelled op name the
        // same 1-based slot in both representations.
        let side_ops = [Some("Deposit-1"), None, Some("Deposit-All")];
        let side_native = [native(TROUT, "Trout", 3, 6, 2006, &side_ops)];
        let side_compat = [compat(TROUT, "Trout", 3, Some(6), Some(2006), &side_ops)];
        let want = DepositClick {
            id: TROUT,
            slot: 6,
            component: 2006,
            operation: 3,
        };
        assert_eq!(
            deposit_click(&side_native[0], DepositRequest::Sweep),
            Some(want)
        );
        assert_eq!(
            deposit_click(&side_compat[0], DepositRequest::Sweep),
            Some(want)
        );
        let one = DepositClick {
            operation: 1,
            ..want
        };
        assert_eq!(
            deposit_click(&side_native[0], DepositRequest::Label("Deposit-1")),
            Some(one)
        );
        assert_eq!(
            deposit_click(&side_compat[0], DepositRequest::Label("Deposit-1")),
            Some(one)
        );
        assert_eq!(
            deposit_click(&side_compat[0], DepositRequest::Label("Deposit-5")),
            None,
            "a missing label is not an All substitute"
        );
        let spec = DepositSpec {
            kind: DepositKind::UntilEmpty,
            keep: &[],
            only: None,
        };
        let pack_native = [held_at(TROUT, 3, 6)];
        let pack_compat = [compat(TROUT, "Trout", 3, None, None, &[])];
        assert_eq!(
            deposit_next(
                &spec,
                Some(&side_native[..]),
                Some(&pack_native[..]),
                false,
                true
            ),
            DepositScan::Click(want)
        );
        assert_eq!(
            deposit_next(
                &spec,
                Some(&side_compat[..]),
                Some(&pack_compat[..]),
                false,
                true
            ),
            DepositScan::Click(want)
        );
        assert_eq!(
            deposit_req(want, 9),
            InteractReq::InvButton {
                id: TROUT,
                slot: 6,
                component: 2006,
                operation: 3,
                bank_generation: 9,
            }
        );

        // Op 0 and out-of-range slots are never ops; holes are not labels.
        for row in [&side_compat[0] as &dyn RowProbe, &side_native[0]] {
            assert_eq!(row.label(0), None);
            assert_eq!(row.label(-1), None);
            assert_eq!(row.label(2), None);
            assert_eq!(row.label(4), None);
            assert_eq!(row.label(3), Some("Deposit-All"));
        }

        // A row the isolate posted without slot or component cannot be clicked.
        let unplaced = compat(TROUT, "Trout", 3, None, Some(2006), &side_ops);
        assert_eq!(deposit_click(&unplaced, DepositRequest::Sweep), None);
        let unplaced = compat(TROUT, "Trout", 3, Some(6), None, &side_ops);
        assert_eq!(deposit_click(&unplaced, DepositRequest::Sweep), None);
        let negative = native(TROUT, "Trout", 3, -1, 2006, &side_ops);
        assert_eq!(deposit_click(&negative, DepositRequest::Sweep), None);
    }

    /// Object-safe probe so one assertion loop covers both row types.
    trait RowProbe {
        fn label(&self, op: i32) -> Option<&str>;
    }

    impl<R: BankRow> RowProbe for R {
        fn label(&self, op: i32) -> Option<&str> {
            self.op_label(op)
        }
    }

    #[test]
    fn capacity_deposit_fails_closed_when_stackability_is_unknown() {
        let side = [compat(
            500,
            "Stack",
            1,
            Some(0),
            Some(2000),
            &[Some("Deposit-All")],
        )];
        let pack = [compat(500, "Stack", 4, Some(0), Some(2001), &[])];
        assert_eq!(
            deposit_capacity_next(&[], Some(&side[..]), Some(&pack[..]), true),
            DepositScan::MissingRequired
        );
    }
}
