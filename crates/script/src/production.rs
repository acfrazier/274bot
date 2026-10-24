//! Shared Make-X selection and count sequencing for runtime adapters.

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
    /// The product has no Make-X button.
    MissingButton,
    /// The caller supplied no valid count.
    InvalidCount,
    /// Click the selected product's Make-X button.
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

/// Shared Make-X selection, count-dialog latch and make-menu close sequence.
/// Runtime adapters supply their row view, count semantics and clock.
#[derive(Clone, Copy)]
pub struct MakeXCore {
    phase: MakeXPhase,
    count: i32,
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
