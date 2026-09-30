//! Snapshot-driven Make/Make-X continuation shared by compiled cards.
//! The caller triggers the production loc/use-on first; this machine owns the
//! menu button, count-dialog latch and observed product-count settlement.
use crate::native::{ActionContext, ActionError, NativeMachine};
use crate::shim::InteractReq;
use api::snapshot::ItemView;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

const MENU_BOUND: Duration = Duration::from_secs(8);
const COUNT_BOUND: Duration = Duration::from_secs(4);
const PRODUCT_BOUND: Duration = Duration::from_secs(2 * 60);

#[derive(Debug, Clone)]
pub struct MakeRequest {
    pub product_id: i32,
    pub product_name: Arc<str>,
    /// Desired held count after this run.
    pub qty: i32,
    pub make_x: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MakeReceipt {
    pub held: i32,
}

enum Phase {
    WaitMenu,
    WaitCount,
    WaitCountClose,
    WaitProduct,
}

pub struct MakeMachine {
    request: MakeRequest,
    phase: Phase,
    deadline: Duration,
    answer: i32,
    before: i32,
}

impl NativeMachine for MakeMachine {
    type Args = MakeRequest;
    type Output = MakeReceipt;

    fn begin(request: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        if request.qty < 1 {
            return Err(ActionError::Unavailable(Arc::from(
                "make quantity must be positive",
            )));
        }
        let before = held(cx, request.product_id).unwrap_or(0);
        Ok(Self {
            request,
            phase: Phase::WaitMenu,
            deadline: cx.active_now().saturating_add(MENU_BOUND),
            answer: 0,
            before,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        loop {
            let now_held = held(cx, self.request.product_id);
            if now_held.is_some_and(|count| count >= self.request.qty) {
                return Poll::Ready(Ok(MakeReceipt {
                    held: now_held.unwrap_or_default(),
                }));
            }
            match self.phase {
                Phase::WaitMenu => {
                    let snapshot = cx.snapshot();
                    let Some(products) = snapshot.make_products() else {
                        return Poll::Pending;
                    };
                    let product = products.value.iter().find(|product| {
                        product
                            .name
                            .eq_ignore_ascii_case(&self.request.product_name)
                            || product
                                .name
                                .to_ascii_lowercase()
                                .contains(&self.request.product_name.to_ascii_lowercase())
                    });
                    let Some(product) = product else {
                        if cx.active_now() >= self.deadline {
                            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                "make menu did not expose the product",
                            ))));
                        }
                        return Poll::Pending;
                    };
                    let current = now_held.unwrap_or(0);
                    let need = (self.request.qty - current).max(1);
                    let component_id = if self.request.make_x {
                        let Some(button) = product
                            .buttons
                            .iter()
                            .find(|button| button.quantity == -1 && button.component_id >= 0)
                        else {
                            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                "make menu has no Make-X button",
                            ))));
                        };
                        button.component_id
                    } else {
                        let Some(button) = product
                            .buttons
                            .iter()
                            .filter(|button| button.quantity > 0)
                            .max_by_key(|button| button.quantity)
                        else {
                            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                "make menu has no fixed-quantity button",
                            ))));
                        };
                        button.component_id
                    };
                    cx.emit(InteractReq::IfButton { component_id })?;
                    if self.request.make_x {
                        self.answer = need;
                        self.deadline = cx.active_now().saturating_add(COUNT_BOUND);
                        self.phase = Phase::WaitCount;
                    } else {
                        self.deadline = cx.active_now().saturating_add(PRODUCT_BOUND);
                        self.phase = Phase::WaitProduct;
                    }
                    return Poll::Pending;
                }
                Phase::WaitCount => {
                    if cx
                        .snapshot()
                        .count_dialog_open()
                        .is_some_and(|open| open.value)
                    {
                        cx.emit(InteractReq::AnswerCount { value: self.answer })?;
                        self.deadline = cx.active_now().saturating_add(COUNT_BOUND);
                        self.phase = Phase::WaitCountClose;
                        return Poll::Pending;
                    }
                    if cx.active_now() >= self.deadline {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "Make-X count dialog did not open",
                        ))));
                    }
                    return Poll::Pending;
                }
                Phase::WaitCountClose => {
                    if cx
                        .snapshot()
                        .count_dialog_open()
                        .is_some_and(|open| !open.value)
                    {
                        self.deadline = cx.active_now().saturating_add(PRODUCT_BOUND);
                        self.phase = Phase::WaitProduct;
                        continue;
                    }
                    if cx.active_now() >= self.deadline {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "Make-X count dialog did not close",
                        ))));
                    }
                    return Poll::Pending;
                }
                Phase::WaitProduct => {
                    if now_held.is_some_and(|count| count > self.before) {
                        // A fixed button may make fewer than requested. The caller's
                        // authored settle sees the exact held count and re-selects.
                        return Poll::Ready(Ok(MakeReceipt {
                            held: now_held.unwrap_or_default(),
                        }));
                    }
                    if cx.active_now() >= self.deadline {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "production did not yield the product",
                        ))));
                    }
                    return Poll::Pending;
                }
            }
        }
    }

    fn cancel(&mut self) {}
}

fn held(cx: &ActionContext<'_>, id: i32) -> Option<i32> {
    cx.snapshot().inventory().map(|rows| count(rows.value, id))
}

fn count(rows: &[ItemView], id: i32) -> i32 {
    rows.iter()
        .filter(|row| row.def.id == id)
        .map(|row| row.count)
        .sum()
}
