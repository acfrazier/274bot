//! Shared production selection and Make-X count sequencing for runtime adapters.

/// The exact native action for one fixed production quantity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MakeFixedAction {
    /// Press a chat Make-Products button.
    ChatButton {
        /// The posted button component.
        component_id: i32,
        /// The quantity exposed by this button.
        quantity: i32,
    },
    /// Apply an operation to one exact main-modal production row.
    MainMake {
        /// The selected item ID.
        item_id: i32,
        /// The selected inventory slot.
        slot: i32,
        /// The selected component ID.
        component_id: i32,
        /// The one-based operation index.
        operation: i32,
        /// The quantity exposed by this operation.
        quantity: i32,
    },
}

/// Result of selecting one posted fixed-quantity action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MakeFixedSelection {
    /// The selected product row is not posted.
    MenuMissing,
    /// The row has no action for the requested quantity.
    MissingButton,
    /// The requested quantity is not positive.
    InvalidCount,
    /// Press the selected product action.
    Select {
        /// The exact action and exposed quantity.
        action: MakeFixedAction,
    },
}

/// One product row from either runtime's posted make menu.
pub trait MakeXProduct {
    /// Return the selected-revision object ID.
    fn object_id(&self) -> i32;
    /// Return the posted Make-X button, if the row offers one.
    fn make_x_component_id(&self) -> Option<i32>;

    /// Return the fixed-quantity chat button for this exact count, if exposed.
    /// Adapters that only use Make-X may keep the default unavailable result.
    fn fixed_quantity_component_id(&self, _quantity: i32) -> Option<i32> {
        None
    }

    /// Return the exact action for this fixed quantity.
    fn fixed_quantity_action(&self, quantity: i32) -> Option<MakeFixedAction> {
        self.fixed_quantity_component_id(quantity)
            .map(|component_id| MakeFixedAction::ChatButton {
                component_id,
                quantity,
            })
    }

    /// Return the largest supported fixed action no greater than `maximum`.
    /// The default only selects an exact count.
    fn fixed_quantity_at_most_action(&self, maximum: i32) -> Option<MakeFixedAction> {
        self.fixed_quantity_action(maximum)
    }
}

/// Next action or terminal result of the shared Make-X sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MakeXStep {
    /// Keep waiting for the current phase's content signal.
    Wait,
    /// Answer the open count dialog once.
    AnswerCount {
        /// The requested production count.
        value: i32,
    },
    /// Begin waiting for the make menu to close.
    WaitMenuClose,
    /// The make menu has closed.
    Complete,
    /// The current phase's wait limit expired.
    TimedOut(
        /// The phase whose wait expired.
        MakeXPhase,
    ),
}
/// Current phase of a Make-X sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MakeXPhase {
    /// No product has been selected.
    Select,
    /// Waiting for the count dialog to open.
    WaitCountOpen,
    /// Waiting for the count dialog to close after answering it.
    WaitCountClose,
    /// Waiting for the make menu to close.
    WaitMenuClose,
}

/// Result of selecting a product row's Make-X button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MakeXSelection {
    /// The selected product row is not posted.
    MenuMissing,
    /// The row has no Make-X action.
    MissingButton,
    /// The requested count is not positive.
    InvalidCount,
    /// Press the selected Make-X action.
    Click {
        /// The posted button component.
        component_id: i32,
    },
}

/// Shared fixed-quantity/Make-X selection and count-dialog sequencing.
/// Runtime adapters supply their row view, count semantics and clock.
/// `Default` uses the same unselected state as [`Self::new`].
#[derive(Clone, Copy)]
pub struct MakeXCore {
    phase: MakeXPhase,
    count: i32,
}

impl Default for MakeXCore {
    fn default() -> Self {
        Self::new()
    }
}

impl MakeXCore {
    /// Create a sequence that has not selected a product.
    pub const fn new() -> Self {
        Self {
            phase: MakeXPhase::Select,
            count: 0,
        }
    }

    /// Return the current sequence phase.
    pub const fn phase(&self) -> MakeXPhase {
        self.phase
    }

    /// Resolve one posted object identity and start its Make-X button.
    pub fn select<P: MakeXProduct>(
        &mut self,
        products: Option<&[P]>,
        object_id: i32,
        count: Option<i32>,
    ) -> MakeXSelection {
        if self.phase != MakeXPhase::Select {
            return MakeXSelection::MenuMissing;
        }
        let Some(product) =
            products.and_then(|rows| rows.iter().find(|row| row.object_id() == object_id))
        else {
            return MakeXSelection::MenuMissing;
        };
        let Some(component_id) = product.make_x_component_id() else {
            return MakeXSelection::MissingButton;
        };
        let Some(count) = count else {
            return MakeXSelection::InvalidCount;
        };
        self.count = count;
        self.phase = MakeXPhase::WaitCountOpen;
        MakeXSelection::Click { component_id }
    }

    /// Resolve the exact fixed-quantity action for one posted product row.
    pub fn select_fixed<P: MakeXProduct>(
        &self,
        products: Option<&[P]>,
        object_id: i32,
        quantity: i32,
    ) -> MakeFixedSelection {
        self.select_fixed_with(products, object_id, quantity, false)
    }

    /// Resolve the largest supported fixed action no greater than the target.
    pub fn select_fixed_at_most<P: MakeXProduct>(
        &self,
        products: Option<&[P]>,
        object_id: i32,
        maximum: i32,
    ) -> MakeFixedSelection {
        self.select_fixed_with(products, object_id, maximum, true)
    }

    fn select_fixed_with<P: MakeXProduct>(
        &self,
        products: Option<&[P]>,
        object_id: i32,
        quantity: i32,
        at_most: bool,
    ) -> MakeFixedSelection {
        if self.phase != MakeXPhase::Select {
            return MakeFixedSelection::MenuMissing;
        }
        if quantity < 1 {
            return MakeFixedSelection::InvalidCount;
        }
        let Some(product) =
            products.and_then(|rows| rows.iter().find(|row| row.object_id() == object_id))
        else {
            return MakeFixedSelection::MenuMissing;
        };
        let action = if at_most {
            product.fixed_quantity_at_most_action(quantity)
        } else {
            product.fixed_quantity_action(quantity)
        };
        let Some(action) = action else {
            return MakeFixedSelection::MissingButton;
        };
        MakeFixedSelection::Select { action }
    }

    /// Advance the frozen open → one answer → close → menu-close order.
    /// `timed_out` belongs to the current phase's adapter-owned clock.
    pub fn step(
        &mut self,
        count_dialog_open: bool,
        make_menu_open: bool,
        timed_out: bool,
    ) -> MakeXStep {
        match self.phase {
            MakeXPhase::Select => {
                if timed_out {
                    MakeXStep::TimedOut(MakeXPhase::Select)
                } else {
                    MakeXStep::Wait
                }
            }
            MakeXPhase::WaitCountOpen if count_dialog_open => {
                self.phase = MakeXPhase::WaitCountClose;
                MakeXStep::AnswerCount { value: self.count }
            }
            MakeXPhase::WaitCountOpen if timed_out => {
                MakeXStep::TimedOut(MakeXPhase::WaitCountOpen)
            }
            MakeXPhase::WaitCountOpen => MakeXStep::Wait,
            MakeXPhase::WaitCountClose if !count_dialog_open => {
                self.phase = MakeXPhase::WaitMenuClose;
                MakeXStep::WaitMenuClose
            }
            MakeXPhase::WaitCountClose if timed_out => {
                MakeXStep::TimedOut(MakeXPhase::WaitCountClose)
            }
            MakeXPhase::WaitCountClose => MakeXStep::Wait,
            MakeXPhase::WaitMenuClose if !make_menu_open => MakeXStep::Complete,
            MakeXPhase::WaitMenuClose if timed_out => {
                MakeXStep::TimedOut(MakeXPhase::WaitMenuClose)
            }
            MakeXPhase::WaitMenuClose => MakeXStep::Wait,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Product {
        id: i32,
        buttons: &'static [(i32, i32)],
        make_x: Option<i32>,
    }

    impl MakeXProduct for Product {
        fn object_id(&self) -> i32 {
            self.id
        }

        fn make_x_component_id(&self) -> Option<i32> {
            self.make_x
        }

        fn fixed_quantity_component_id(&self, quantity: i32) -> Option<i32> {
            self.buttons
                .iter()
                .find(|(button_quantity, component_id)| {
                    *button_quantity == quantity && *component_id >= 0
                })
                .map(|(_, component_id)| *component_id)
        }

        fn fixed_quantity_at_most_action(&self, maximum: i32) -> Option<MakeFixedAction> {
            let &(quantity, component_id) = self
                .buttons
                .iter()
                .filter(|(quantity, component_id)| *quantity <= maximum && *component_id >= 0)
                .max_by_key(|(quantity, _)| quantity)?;
            Some(MakeFixedAction::ChatButton {
                component_id,
                quantity,
            })
        }
    }

    #[test]
    fn fixed_selection_requires_exact_product_and_quantity() {
        let products = [
            Product {
                id: 7,
                buttons: &[(1, 201), (5, 205), (10, 210)],
                make_x: Some(220),
            },
            Product {
                id: 8,
                buttons: &[(1, 301), (5, 305), (10, 310)],
                make_x: Some(320),
            },
        ];
        let core = MakeXCore::new();
        assert_eq!(
            core.select_fixed(Some(&products), 7, 5),
            MakeFixedSelection::Select {
                action: MakeFixedAction::ChatButton {
                    component_id: 205,
                    quantity: 5,
                }
            }
        );
        assert_eq!(
            core.select_fixed(Some(&products), 8, 10),
            MakeFixedSelection::Select {
                action: MakeFixedAction::ChatButton {
                    component_id: 310,
                    quantity: 10,
                }
            }
        );
        assert_eq!(
            core.select_fixed(Some(&products), 7, 20),
            MakeFixedSelection::MissingButton
        );
        assert_eq!(
            core.select_fixed(Some(&products), 9, 1),
            MakeFixedSelection::MenuMissing
        );
        assert_eq!(
            core.select_fixed(Some(&products), 7, 0),
            MakeFixedSelection::InvalidCount
        );
    }

    #[test]
    fn at_most_selection_uses_the_largest_supported_quantity_not_over_target() {
        let product = Product {
            id: 7,
            buttons: &[(1, 201), (5, 205), (10, 210)],
            make_x: Some(220),
        };
        assert_eq!(
            MakeXCore::new().select_fixed_at_most(Some(&[product]), 7, 7),
            MakeFixedSelection::Select {
                action: MakeFixedAction::ChatButton {
                    component_id: 205,
                    quantity: 5,
                }
            }
        );
    }
}
