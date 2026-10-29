//! Shared production selection and Make-X count sequencing for runtime adapters.

/// Frozen family-1 count-dialog open wait.
pub const COUNT_OPEN_MS: u64 = 3_000;
/// Frozen count-dialog close wait after Answer-Count.
pub const COUNT_CLOSE_MS: u64 = 3_000;
/// Frozen chat make-menu close wait after the count dialog drops.
pub const MAKE_MENU_MS: u64 = 5_000;

/// One product row from either runtime's posted make menu.
pub trait MakeXProduct {
    /// Return the selected-revision object ID.
    fn object_id(&self) -> i32;
    /// Return the posted Make-X button, if the row offers one.
    fn make_x_component_id(&self) -> Option<i32>;

    /// Return the fixed-quantity button for this exact count when exposed.
    /// Adapters that only use Make-X may keep the default unavailable result.
    fn fixed_quantity_component_id(&self, _quantity: i32) -> Option<i32> {
        None
    }
}

/// Current phase of the shared Make-X sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MakeXPhase {
    /// Select the product and its Make-X button.
    Select,
    /// Wait for the count dialog to open.
    WaitCountOpen,
    /// Wait for the answered count dialog to close.
    WaitCountClose,
    /// Wait for the make menu to close.
    WaitMenuClose,
}

/// Result of selecting one posted product and quantity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MakeXSelection {
    /// The sequence cannot select from the current menu.
    MenuMissing,
    /// The product has no button for the requested selection.
    MissingButton,
    /// The caller supplied no valid count.
    InvalidCount,
    /// Click the selected product button.
    Click {
        /// The posted button component ID.
        component_id: i32,
    },
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

    /// Return the current phase's wait limit, in milliseconds.
    pub const fn timeout_ms(&self) -> u64 {
        match self.phase {
            MakeXPhase::Select => MAKE_MENU_MS,
            MakeXPhase::WaitCountOpen => COUNT_OPEN_MS,
            MakeXPhase::WaitCountClose => COUNT_CLOSE_MS,
            MakeXPhase::WaitMenuClose => MAKE_MENU_MS,
        }
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

    /// Resolve the exact fixed-quantity button for one posted product row.
    pub fn select_fixed<P: MakeXProduct>(
        &self,
        products: Option<&[P]>,
        object_id: i32,
        quantity: i32,
    ) -> MakeXSelection {
        if self.phase != MakeXPhase::Select {
            return MakeXSelection::MenuMissing;
        }
        if quantity < 1 {
            return MakeXSelection::InvalidCount;
        }
        let Some(product) =
            products.and_then(|rows| rows.iter().find(|row| row.object_id() == object_id))
        else {
            return MakeXSelection::MenuMissing;
        };
        let Some(component_id) = product.fixed_quantity_component_id(quantity) else {
            return MakeXSelection::MissingButton;
        };
        MakeXSelection::Click { component_id }
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
            MakeXSelection::Click { component_id: 205 }
        );
        assert_eq!(
            core.select_fixed(Some(&products), 8, 10),
            MakeXSelection::Click { component_id: 310 }
        );
        assert_eq!(
            core.select_fixed(Some(&products), 7, 20),
            MakeXSelection::MissingButton
        );
        assert_eq!(
            core.select_fixed(Some(&products), 9, 1),
            MakeXSelection::MenuMissing
        );
        assert_eq!(
            core.select_fixed(Some(&products), 7, 0),
            MakeXSelection::InvalidCount
        );
    }
}
