//! Snapshot-driven production adapter shared with compat Make-X sequencing.
//! The caller triggers the production loc/use-on first; this machine adapts
//! target-held quantities and observes output settlement.
use crate::native::{ActionContext, ActionError, NativeMachine};
use crate::production::{MakeXCore, MakeXPhase, MakeXProduct, MakeXSelection, MakeXStep};
use crate::shim::InteractReq;
use api::snapshot::MakeProductView;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

const PRODUCT_BOUND: Duration = Duration::from_secs(2 * 60);

impl MakeXProduct for MakeProductView {
    fn object_id(&self) -> i32 {
        self.object_id
    }

    fn make_x_component_id(&self) -> Option<i32> {
        self.buttons
            .iter()
            .find(|button| button.quantity == -1 && button.component_id >= 0)
            .map(|button| button.component_id)
    }
}

#[derive(Debug, Clone)]
pub struct MakeRequest {
    pub product_id: i32,
    /// Object represented by the menu row; it may be the input, not the output.
    pub menu_id: i32,
    /// Desired held count after this run.
    pub qty: i32,
    pub make_x: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MakeReceipt {
    pub held: i32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    MakeX,
    FixedMenu,
    Product,
}

pub struct MakeMachine {
    request: MakeRequest,
    phase: Phase,
    make_x: MakeXCore,
    deadline: Duration,
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
        let make_x = MakeXCore::new();
        let phase = if request.make_x {
            Phase::MakeX
        } else {
            Phase::FixedMenu
        };
        let deadline = cx
            .active_now()
            .saturating_add(Duration::from_millis(make_x.timeout_ms()));
        Ok(Self {
            request,
            phase,
            make_x,
            deadline,
            before,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        loop {
            let now = cx.active_now();
            let now_held = held(cx, self.request.product_id);
            if now_held.is_some_and(|count| count >= self.request.qty)
                && (self.phase != Phase::MakeX || self.make_x.phase() == MakeXPhase::Select)
            {
                return Poll::Ready(Ok(MakeReceipt {
                    held: now_held.unwrap_or_default(),
                }));
            }
            match self.phase {
                Phase::MakeX => {
                    let current = now_held.unwrap_or(0);
                    let answer = (self.request.qty - current).max(1);
                    let (selection, count_dialog_open, make_menu_open) = {
                        let snapshot = cx.snapshot();
                        let products = snapshot.make_products();
                        let selection = if self.make_x.phase() == MakeXPhase::Select {
                            Some(self.make_x.select(
                                products.as_ref().map(|rows| rows.value),
                                self.request.menu_id,
                                Some(answer),
                            ))
                        } else {
                            None
                        };
                        let count_dialog_open =
                            snapshot.count_dialog_open().is_some_and(|open| open.value);
                        let make_menu_open =
                            products.as_ref().is_none_or(|rows| !rows.value.is_empty());
                        (selection, count_dialog_open, make_menu_open)
                    };
                    if let Some(selection) = selection {
                        match selection {
                            MakeXSelection::MenuMissing => {
                                if now >= self.deadline {
                                    return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                        "make menu did not expose the product",
                                    ))));
                                }
                                return Poll::Pending;
                            }
                            MakeXSelection::MissingButton => {
                                return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                    "make menu has no Make-X button",
                                ))));
                            }
                            MakeXSelection::InvalidCount => {
                                return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                    "make quantity must be positive",
                                ))));
                            }
                            MakeXSelection::Click { component_id } => {
                                cx.emit(InteractReq::IfButton { component_id })?;
                                self.deadline = now.saturating_add(Duration::from_millis(
                                    self.make_x.timeout_ms(),
                                ));
                                return Poll::Pending;
                            }
                        }
                    }
                    match self
                        .make_x
                        .step(count_dialog_open, make_menu_open, now >= self.deadline)
                    {
                        MakeXStep::Wait => return Poll::Pending,
                        MakeXStep::AnswerCount { value } => {
                            cx.emit(InteractReq::AnswerCount { value })?;
                            self.deadline =
                                now.saturating_add(Duration::from_millis(self.make_x.timeout_ms()));
                            return Poll::Pending;
                        }
                        MakeXStep::WaitMenuClose => {
                            self.deadline =
                                now.saturating_add(Duration::from_millis(self.make_x.timeout_ms()));
                            return Poll::Pending;
                        }
                        MakeXStep::Complete => {
                            self.phase = Phase::Product;
                            self.deadline = now.saturating_add(PRODUCT_BOUND);
                            continue;
                        }
                        MakeXStep::TimedOut(phase) => {
                            let message = match phase {
                                MakeXPhase::Select => "make menu did not expose the product",
                                MakeXPhase::WaitCountOpen => "Make-X count dialog did not open",
                                MakeXPhase::WaitCountClose => "Make-X count dialog did not close",
                                MakeXPhase::WaitMenuClose => "Make-X menu did not close",
                            };
                            return Poll::Ready(Err(ActionError::Failed(Arc::from(message))));
                        }
                    }
                }
                Phase::FixedMenu => {
                    let snapshot = cx.snapshot();
                    let Some(products) = snapshot.make_products() else {
                        return Poll::Pending;
                    };
                    let product = products
                        .value
                        .iter()
                        .find(|product| product.object_id == self.request.menu_id);
                    let Some(product) = product else {
                        if now >= self.deadline {
                            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                "make menu did not expose the product",
                            ))));
                        }
                        return Poll::Pending;
                    };
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
                    cx.emit(InteractReq::IfButton {
                        component_id: button.component_id,
                    })?;
                    self.phase = Phase::Product;
                    self.deadline = now.saturating_add(PRODUCT_BOUND);
                    return Poll::Pending;
                }
                Phase::Product => {
                    if now_held.is_some_and(|count| count > self.before) {
                        // A fixed button may make fewer than requested. The caller's
                        // authored settle sees the exact held count and re-selects.
                        return Poll::Ready(Ok(MakeReceipt {
                            held: now_held.unwrap_or_default(),
                        }));
                    }
                    if now >= self.deadline {
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
    cx.snapshot().stock().held(id)
}
