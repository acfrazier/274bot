use crate::native::{ActionContext, ActionError, NativeMachine};
use crate::shim::InteractReq;
use std::sync::Arc;
use std::task::Poll;

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
    args: OneOpArgs,
    request_id: u64,
    deadline: u64,
    continued_root: Option<i32>,
}

impl OneOpArgs {
    pub fn wear(name: Arc<str>, item_id: i32, action: &str) -> Self {
        Self {
            request: InteractReq::Held {
                name: name.to_string(),
                action: action.into(),
                slot: None,
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
            },
            kind: OneOpKind::Eat {
                item_id,
                before_count,
                before_hp,
            },
            bound_ticks: 8,
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
            request: InteractReq::ContinueDialog,
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
        let request_id = cx.emit(args.request.clone())?;
        Ok(Self {
            deadline: cx.evidence().tick.saturating_add(args.bound_ticks),
            args,
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
        let settled = match self.args.kind {
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
            OneOpKind::CloseModal => cx
                .snapshot()
                .main_modal()
                .is_some_and(|modal| modal.value.root < 0),
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
