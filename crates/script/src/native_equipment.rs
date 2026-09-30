//! Observed wear/remove core shared by compiled cards.
use crate::native::{ActionContext, ActionError, NativeMachine};
use crate::shim::InteractReq;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

const SETTLE_BOUND: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub enum EquipmentRequest {
    Wear { id: i32, name: Arc<str> },
    Unequip { id: i32, name: Arc<str> },
    Strip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EquipmentReceipt {
    pub changed: u8,
}

pub struct EquipmentMachine {
    request: EquipmentRequest,
    pending_id: Option<i32>,
    deadline: Duration,
    changed: u8,
}

impl NativeMachine for EquipmentMachine {
    type Args = EquipmentRequest;
    type Output = EquipmentReceipt;

    fn begin(request: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        Ok(Self {
            request,
            pending_id: None,
            deadline: cx.active_now().saturating_add(SETTLE_BOUND),
            changed: 0,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        let snapshot = cx.snapshot();
        let Some(worn) = snapshot.equipment() else {
            return Poll::Pending;
        };
        if let Some(id) = self.pending_id {
            let landed = match self.request {
                EquipmentRequest::Wear { .. } => worn.value.iter().any(|row| row.def.id == id),
                EquipmentRequest::Unequip { .. } | EquipmentRequest::Strip => {
                    !worn.value.iter().any(|row| row.def.id == id)
                }
            };
            if landed {
                self.pending_id = None;
                self.changed = self.changed.saturating_add(1);
            } else if cx.active_now() >= self.deadline {
                return Poll::Ready(Err(ActionError::Failed(Arc::from(
                    "equipment change did not settle",
                ))));
            } else {
                return Poll::Pending;
            }
        }
        match &self.request {
            EquipmentRequest::Wear { id, name } => {
                if worn.value.iter().any(|row| row.def.id == *id) {
                    return Poll::Ready(Ok(EquipmentReceipt {
                        changed: self.changed,
                    }));
                }
                let held = cx.snapshot().inventory().is_some_and(|rows| {
                    rows.value
                        .iter()
                        .any(|row| row.def.id == *id && row.count > 0)
                });
                if !held {
                    return Poll::Ready(Err(ActionError::Failed(Arc::from(
                        "equipment item is not held",
                    ))));
                }
                cx.emit(InteractReq::Wear {
                    name: name.to_string(),
                })?;
                self.pending_id = Some(*id);
            }
            EquipmentRequest::Unequip { id, name } => {
                if !worn.value.iter().any(|row| row.def.id == *id) {
                    return Poll::Ready(Ok(EquipmentReceipt {
                        changed: self.changed,
                    }));
                }
                cx.emit(InteractReq::Unequip {
                    name: name.to_string(),
                })?;
                self.pending_id = Some(*id);
            }
            EquipmentRequest::Strip => {
                let row = worn
                    .value
                    .iter()
                    .find(|row| row.count > 0)
                    .and_then(|row| row.def.name.clone().map(|name| (row.def.id, name)));
                let Some((id, name)) = row else {
                    return Poll::Ready(Ok(EquipmentReceipt {
                        changed: self.changed,
                    }));
                };
                cx.emit(InteractReq::Unequip { name })?;
                self.pending_id = Some(id);
            }
        }
        self.deadline = cx.active_now().saturating_add(SETTLE_BOUND);
        Poll::Pending
    }

    fn cancel(&mut self) {}
}
