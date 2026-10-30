use crate::native::{ActionContext, ActionError, NativeMachine};
use crate::shim::InteractReq;
use std::sync::Arc;
use std::task::Poll;

/// Observed ticks a survival drink waits for its dose drop before the runner
/// may click again. The engine runs a held op in the server tick that reads
/// it (`OpHeldHandler.ts:68`), and the antipoison effect replaces the dose in
/// that tick (`anti_poison.rs2:8`), so a processed drink is seen by the next
/// observed tick. The engine drops a held op without any reply while the
/// player is delayed (`OpHeldHandler.ts:16-19`), for example in the tick after
/// a mining success's `p_delay(0)` (`mining.rs2:138`, `PlayerOps.ts:377-378`).
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
    Eat {
        item_id: i32,
        before_count: i32,
        before_hp: i32,
    },
    Drink {
        item_id: i32,
        before_doses: i32,
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

    /// Close a page that blocks a survival held click.
    pub fn close_modal_if_open(snapshot: api::snapshot::SnapshotView<'_>) -> Option<Self> {
        let main = snapshot
            .main_modal()
            .is_some_and(|modal| modal.value.root >= 0);
        let chat = snapshot
            .chat_modal()
            .is_some_and(|modal| modal.value.root >= 0);
        (main || chat).then(Self::close_modal)
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

    pub fn eat(name: &str, item_id: i32, before_count: i32, before_hp: i32) -> Self {
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
                before_hp,
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
                before_hp,
            } => {
                let snapshot = cx.snapshot();
                snapshot.inventory().is_some_and(|rows| {
                    rows.value
                        .iter()
                        .filter(|row| row.def.id == item_id)
                        .map(|row| row.count)
                        .sum::<i32>()
                        < before_count
                }) || snapshot.stats().is_some_and(|stats| {
                    stats
                        .value
                        .iter()
                        .any(|stat| stat.index == 3 && stat.effective > before_hp)
                })
            }
            OneOpKind::Drink {
                item_id,
                before_doses,
            } => cx.snapshot().inventory().is_some_and(|rows| {
                rows.value
                    .iter()
                    .filter(|row| row.def.id == item_id)
                    .map(|row| row.count)
                    .sum::<i32>()
                    < before_doses
            }),
            OneOpKind::CloseModal => {
                let snapshot = cx.snapshot();
                snapshot
                    .main_modal()
                    .is_some_and(|modal| modal.value.root < 0)
                    && snapshot
                        .chat_modal()
                        .is_none_or(|modal| modal.value.root < 0)
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
