//! Observed wear/remove core shared by compiled cards.
use crate::native::{defer_budget, ActionContext, ActionError, NativeMachine};
use crate::shim::InteractReq;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

const SETTLE_BOUND: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub enum EquipmentRequest {
    Wear { id: i32, name: Arc<str> },
    Unequip { id: i32, name: Arc<str> },
    Strip { keep: Arc<[i32]> },
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
                EquipmentRequest::Unequip { .. } | EquipmentRequest::Strip { .. } => {
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
                std::task::ready!(defer_budget(cx.emit(InteractReq::Wear {
                    name: name.to_string(),
                })))?;
                self.pending_id = Some(*id);
            }
            EquipmentRequest::Unequip { id, name } => {
                if !worn.value.iter().any(|row| row.def.id == *id) {
                    return Poll::Ready(Ok(EquipmentReceipt {
                        changed: self.changed,
                    }));
                }
                std::task::ready!(defer_budget(cx.emit(InteractReq::Unequip {
                    name: name.to_string(),
                })))?;
                self.pending_id = Some(*id);
            }
            EquipmentRequest::Strip { keep } => {
                let row = worn
                    .value
                    .iter()
                    .find(|row| row.count > 0 && !keep.contains(&row.def.id))
                    .and_then(|row| row.def.name.clone().map(|name| (row.def.id, name)));
                let Some((id, name)) = row else {
                    return Poll::Ready(Ok(EquipmentReceipt {
                        changed: self.changed,
                    }));
                };
                std::task::ready!(defer_budget(cx.emit(InteractReq::Unequip { name })))?;
                self.pending_id = Some(id);
            }
        }
        self.deadline = cx.active_now().saturating_add(SETTLE_BOUND);
        Poll::Pending
    }

    fn cancel(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::HostEffect;
    use crate::quester::families::tests::{def, with_tick, with_tick_snapshots};
    use api::snapshot::{GameSnapshot, ItemActionFamily, ItemContainer, ItemView};

    fn item(id: i32, container: ItemContainer) -> ItemView {
        ItemView {
            def: def(id, "Equipment"),
            container,
            action_family: ItemActionFamily::Held,
            slot: id,
            count: 1,
            actions: Vec::new(),
            component_id: -1,
        }
    }

    #[test]
    fn equipment_admissions_after_same_tick_close_retry_without_advancing_state() {
        for request in [
            EquipmentRequest::Wear {
                id: 1,
                name: Arc::from("Equipment"),
            },
            EquipmentRequest::Unequip {
                id: 1,
                name: Arc::from("Equipment"),
            },
            EquipmentRequest::Strip {
                keep: Arc::from([]),
            },
        ] {
            let mut snapshot = GameSnapshot::new();
            snapshot.seed_ingame(2);
            snapshot.seed_inventory(vec![item(1, ItemContainer::Inventory)], 28);
            snapshot.seed_equipment(if matches!(request, EquipmentRequest::Wear { .. }) {
                Vec::new()
            } else {
                vec![
                    item(1, ItemContainer::Equipment),
                    item(2, ItemContainer::Equipment),
                ]
            });
            let mut ledger = None;
            let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
                let handle = tick
                    .actions
                    .begin::<EquipmentMachine>(request.clone(), &mut tick.cx)
                    .unwrap();
                tick.cx.emit(InteractReq::CloseModal).unwrap();
                for _ in 0..3 {
                    assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
                    assert_eq!(handle.inspect(|machine| machine.pending_id), Some(None));
                    assert_eq!(tick.cx.ledger.as_ref().unwrap().outbox.len(), 1);
                }
                handle
            });
            let mut later = GameSnapshot::new();
            later.seed_ingame(2);
            later.seed_inventory(Vec::new(), 28);
            later.seed_equipment(if matches!(request, EquipmentRequest::Wear { .. }) {
                vec![item(1, ItemContainer::Equipment)]
            } else {
                vec![item(2, ItemContainer::Equipment)]
            });
            with_tick_snapshots(&snapshot, &later, &mut ledger, 2, |tick, later| {
                assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
                assert!(matches!(
                    &tick
                        .cx
                        .ledger
                        .as_ref()
                        .unwrap()
                        .outbox
                        .last()
                        .unwrap()
                        .effect,
                    HostEffect::Interaction(InteractReq::Wear { .. } | InteractReq::Unequip { .. })
                ));
                tick.cx.snapshot =
                    api::snapshot::SnapshotView::new(Some(later), tick.cx.evidence());
                let result = tick.actions.poll(&handle, &mut tick.cx);
                if matches!(request, EquipmentRequest::Strip { .. }) {
                    assert!(
                        result.is_pending(),
                        "strip's second item must wait: {result:?}"
                    );
                    assert_eq!(handle.inspect(|machine| machine.changed), Some(1));
                    assert_eq!(handle.inspect(|machine| machine.pending_id), Some(None));
                } else {
                    assert!(matches!(
                        result,
                        Poll::Ready(Ok(EquipmentReceipt { changed: 1 }))
                    ));
                }
            });
            if matches!(request, EquipmentRequest::Strip { .. }) {
                assert!(with_tick(&later, &mut ledger, 3, |tick| {
                    tick.actions.poll(&handle, &mut tick.cx)
                })
                .is_pending());
                assert_eq!(handle.inspect(|machine| machine.pending_id), Some(Some(2)));
            }
        }
    }
}
