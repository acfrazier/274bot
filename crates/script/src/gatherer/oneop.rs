use crate::native::{ActionContext, ActionError, NativeMachine};
use crate::shim::InteractReq;
use std::sync::Arc;
use std::task::Poll;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OneOpKind {
    Wear { item_id: i32 },
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
}

impl OneOpArgs {
    pub fn wear(name: Arc<str>, item_id: i32) -> Self {
        Self {
            request: InteractReq::Held {
                name: name.to_string(),
                action: "Wear".into(),
                slot: None,
            },
            kind: OneOpKind::Wear { item_id },
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
        let request_id = cx.emit(args.request.clone())?;
        Ok(Self {
            deadline: cx.evidence().tick.saturating_add(args.bound_ticks),
            args,
            request_id,
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
            OneOpKind::CloseModal => cx
                .snapshot()
                .main_modal()
                .is_some_and(|modal| modal.value.root < 0),
            OneOpKind::ContinueDialog => cx
                .snapshot()
                .chat_modal()
                .is_some_and(|modal| modal.value.root < 0),
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
