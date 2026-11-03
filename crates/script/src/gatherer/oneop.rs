use crate::interact::InteractReq;
use crate::native::{ActionContext, ActionError, NativeMachine};
use std::sync::Arc;
use std::task::Poll;

/// Observed ticks a survival drink waits for its dose drop before the runner
/// may click again. The engine runs a held op in the server tick that reads
/// it (`OpHeldHandler.ts:68`), and the antipoison effect replaces the dose in
/// that tick (`anti_poison.rs2:8`), so a processed drink is seen by the next
/// observed tick. The engine drops a held op without any reply while the
/// player is delayed (`OpHeldHandler.ts:16-19`), for example in the tick after
/// a mining success's `p_delay(0)` (`mining.rs2:139`, `PlayerOps.ts:377-378`).
/// This is the drink lock the walk guard and the combat schedule already use
/// (`combat/schedule.rs` `OpKind::Drink`), and it outlasts the drink's own
/// `p_delay(1)` (`consume_effects.rs2:124-126`), so a second click cannot land
/// inside the first drink's delayed ticks.
const DRINK_SETTLE_TICKS: u64 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OneOpKind {
    Wear {
        item_id: i32,
    },
    /// A bite settles on the held food's count drop alone. A multi-bite food
    /// changes item id, which also lowers the count of the id that was eaten.
    /// A Hitpoints rise is not a signal: natural regeneration restores one
    /// point on its own timer (`stat.rs2:58-60`), so a rise in the frame after
    /// the click can precede the bite's own count drop. The consume script
    /// deletes or replaces the item in the server tick that runs the click
    /// (`consume.rs2:122-127`), so the count drop is the click's tick.
    Eat {
        item_id: i32,
        before_count: i32,
    },
    Drink {
        item_id: i32,
        before_doses: i32,
    },
    /// Strange fruit: an `Eat` op that cures poison and heals nothing
    /// (`strange_fruit.rs2:1-12` deletes the slot and only restores energy), so
    /// it settles on the count drop alone, never on a hitpoints rise.
    CureFruit {
        item_id: i32,
        before_count: i32,
    },
    CloseModal,
    ContinueDialog,
}

#[derive(Debug, Clone)]
pub struct OneOpArgs {
    pub request: InteractReq,
    pub kind: OneOpKind,
    pub bound_ticks: u64,
}

pub struct OneOp {
    kind: OneOpKind,
    request_id: u64,
    deadline: u64,
    continued_root: Option<i32>,
}

impl OneOpArgs {
    /// Shared Gatherer policy for a stray skill/unlock page or main modal.
    pub(crate) fn stray_modal(snapshot: api::snapshot::SnapshotView<'_>) -> Option<Self> {
        if snapshot
            .chat_continue()
            .is_some_and(|button| button.value >= 0)
        {
            Some(Self::continue_dialog())
        } else if snapshot
            .main_modal()
            .is_some_and(|modal| modal.value.root >= 0)
        {
            Some(Self::close_modal())
        } else {
            None
        }
    }

    /// Close a page that blocks a survival held click. An enter-amount prompt
    /// counts: the client refuses every held click while it is open
    /// (`api::interact` `precondition`), and the engine refuses a protected
    /// script while the suspended count-dialog script keeps `protect` set
    /// (`Player.ts:2173-2178`, `:2213-2221`). The engine's `closeModal` clears
    /// `protect` before it checks for an open modal (`Player.ts:760-766`).
    pub fn close_modal_if_open(snapshot: api::snapshot::SnapshotView<'_>) -> Option<Self> {
        let main = snapshot
            .main_modal()
            .is_some_and(|modal| modal.value.root >= 0);
        let chat = snapshot
            .chat_modal()
            .is_some_and(|modal| modal.value.root >= 0);
        let count = snapshot.count_dialog_open().is_some_and(|open| open.value);
        (main || chat || count).then(Self::close_modal)
    }

    pub fn wear(name: Arc<str>, item_id: i32, action: &str) -> Self {
        Self {
            request: InteractReq::Held {
                name: name.to_string(),
                action: action.into(),
                slot: None,
                target_item_id: None,
            },
            kind: OneOpKind::Wear { item_id },
            bound_ticks: 8,
        }
    }

    pub fn eat(name: &str, item_id: i32, before_count: i32) -> Self {
        Self {
            request: InteractReq::Held {
                name: name.into(),
                action: "Eat".into(),
                slot: None,
                target_item_id: None,
            },
            kind: OneOpKind::Eat {
                item_id,
                before_count,
            },
            bound_ticks: 8,
        }
    }

    pub fn drink(name: &str, item_id: i32, before_doses: i32) -> Self {
        Self {
            request: InteractReq::Held {
                name: name.into(),
                action: "Drink".into(),
                slot: None,
                target_item_id: None,
            },
            kind: OneOpKind::Drink {
                item_id,
                before_doses,
            },
            bound_ticks: DRINK_SETTLE_TICKS,
        }
    }

    /// Eat a held Strange fruit as a poison cure. It shares the drink's
    /// settle bound: the fruit's slot is deleted in the server tick that runs
    /// the op (`strange_fruit.rs2:6`), and a dropped op is clicked again after
    /// the same lock.
    pub fn cure_fruit(name: &str, item_id: i32, before_count: i32) -> Self {
        Self {
            request: InteractReq::Held {
                name: name.into(),
                action: "Eat".into(),
                slot: None,
                target_item_id: None,
            },
            kind: OneOpKind::CureFruit {
                item_id,
                before_count,
            },
            bound_ticks: DRINK_SETTLE_TICKS,
        }
    }

    pub fn close_modal() -> Self {
        Self {
            request: InteractReq::CloseModal,
            kind: OneOpKind::CloseModal,
            bound_ticks: 8,
        }
    }

    pub fn continue_dialog() -> Self {
        Self {
            request: InteractReq::ContinueDialog { component_id: None },
            kind: OneOpKind::ContinueDialog,
            bound_ticks: 8,
        }
    }
}

impl NativeMachine for OneOp {
    type Args = OneOpArgs;
    type Output = bool;

    fn begin(args: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        let continued_root = if args.kind == OneOpKind::ContinueDialog {
            cx.snapshot().chat_modal().map(|modal| modal.value.root)
        } else {
            None
        };
        let request_id = cx.emit(args.request)?;
        Ok(Self {
            deadline: cx.evidence().tick.saturating_add(args.bound_ticks),
            kind: args.kind,
            request_id,
            continued_root,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        if let Some(receipt) = cx.interaction_receipt(self.request_id) {
            if !receipt.accepted {
                return Poll::Ready(Ok(false));
            }
        }
        let settled = match self.kind {
            OneOpKind::Wear { item_id } => cx
                .snapshot()
                .equipment()
                .is_some_and(|rows| rows.value.iter().any(|row| row.def.id == item_id)),
            OneOpKind::Eat {
                item_id,
                before_count,
            }
            | OneOpKind::Drink {
                item_id,
                before_doses: before_count,
            }
            | OneOpKind::CureFruit {
                item_id,
                before_count,
            } => cx.snapshot().inventory().is_some_and(|rows| {
                rows.value
                    .iter()
                    .filter(|row| row.def.id == item_id)
                    .map(|row| row.count)
                    .sum::<i32>()
                    < before_count
            }),
            OneOpKind::CloseModal => {
                let snapshot = cx.snapshot();
                snapshot
                    .main_modal()
                    .is_some_and(|modal| modal.value.root < 0)
                    && snapshot
                        .chat_modal()
                        .is_none_or(|modal| modal.value.root < 0)
                    && !snapshot.count_dialog_open().is_some_and(|open| open.value)
            }
            OneOpKind::ContinueDialog => cx.snapshot().chat_modal().is_some_and(|modal| {
                // A chained unlock page acknowledges this page; the runner must
                // issue a separate continue for the new modal.
                modal.value.root < 0
                    || self
                        .continued_root
                        .is_some_and(|root| modal.value.root != root)
            }),
        };
        if settled {
            return Poll::Ready(Ok(true));
        }
        if cx.evidence().tick >= self.deadline {
            return Poll::Ready(Ok(false));
        }
        Poll::Pending
    }

    fn cancel(&mut self) {}
}
