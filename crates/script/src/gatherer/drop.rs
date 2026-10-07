use crate::native::{defer_budget, ActionContext, ActionError, NativeMachine};
use crate::shim::InteractReq;
use api::snapshot::ItemView;
use std::sync::Arc;
use std::task::Poll;

const MAX_PROTECTED: usize = 8;
const MAX_BATCH: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropBatchArgs {
    pub products: Arc<[i32]>,
    pub protected: [i32; MAX_PROTECTED],
    pub protected_len: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropEnd {
    Cleared,
    Blocked { remaining: u16 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DropResult {
    pub end: DropEnd,
    pub dropped: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Entry {
    slot: i32,
    expected: i32,
    request: Option<u64>,
    sent_tick: u64,
}

pub struct DropBatch {
    args: DropBatchArgs,
    entries: [Option<Entry>; MAX_BATCH],
    tick: u64,
    packets: u8,
    close_request: Option<u64>,
    close_tick: u64,
    rounds_without_settle: u8,
    dropped: u32,
}

impl NativeMachine for DropBatch {
    type Args = DropBatchArgs;
    type Output = DropResult;

    fn begin(args: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        Ok(Self {
            args,
            entries: [None; MAX_BATCH],
            tick: cx.evidence().tick,
            packets: 0,
            close_request: None,
            close_tick: 0,
            rounds_without_settle: 0,
            dropped: 0,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        if self.tick != cx.evidence().tick {
            self.tick = cx.evidence().tick;
            self.packets = 0;
        }
        let snapshot = cx.snapshot();
        let Some(inventory) = snapshot.inventory() else {
            return Poll::Pending;
        };
        let mut settled = false;
        let previous_dropped = self.dropped;
        let retry_round = self.reconcile(cx, inventory.value, &mut settled);
        let confirmed = self.dropped.saturating_sub(previous_dropped);
        if confirmed != 0 {
            let retained = cx.retained().gather();
            retained.dropped = retained.dropped.saturating_add(confirmed);
        }
        if settled {
            self.rounds_without_settle = 0;
        } else if retry_round {
            self.rounds_without_settle = self.rounds_without_settle.saturating_add(1);
        }
        if self.rounds_without_settle >= 3 {
            return Poll::Ready(Ok(DropResult {
                end: DropEnd::Blocked {
                    remaining: self.remaining(inventory.value),
                },
                dropped: self.dropped,
            }));
        }

        // Mining's full-pack message is an ordinary modal packet. It shares
        // the same five-packet fence, so at most four drops follow it.
        if self.close_request.is_none()
            && self.packets < MAX_BATCH as u8
            && (snapshot
                .main_modal()
                .is_some_and(|modal| modal.value.root >= 0)
                || snapshot
                    .chat_modal()
                    .is_some_and(|modal| modal.value.root >= 0))
        {
            let request = std::task::ready!(defer_budget(cx.emit(InteractReq::CloseModal)))?;
            self.close_request = Some(request);
            self.close_tick = cx.evidence().tick;
            self.packets += 1;
        }

        while self.packets < MAX_BATCH as u8 {
            let Some(row) = self.next_candidate(inventory.value) else {
                break;
            };
            let Some(name) = row.def.name.as_deref() else {
                // A row without an object name cannot be dispatched safely.
                break;
            };
            let request = InteractReq::Held {
                name: name.to_string(),
                action: "Drop".into(),
                slot: Some(row.slot),
                target_item_id: None,
            };
            match cx.emit_disposal(request) {
                Ok(request_id) => {
                    self.insert(Entry {
                        slot: row.slot,
                        expected: row.def.id,
                        request: Some(request_id),
                        sent_tick: cx.evidence().tick,
                    });
                    self.packets += 1;
                }
                Err(ActionError::BudgetExhausted) => break,
                Err(error) => return Poll::Ready(Err(error)),
            }
        }

        let remaining = self.remaining(inventory.value);
        if remaining == 0 && self.entries.iter().all(Option::is_none) {
            return Poll::Ready(Ok(DropResult {
                end: DropEnd::Cleared,
                dropped: self.dropped,
            }));
        }
        Poll::Pending
    }

    fn cancel(&mut self) {}
}

impl DropBatch {
    fn can_track(&self, slot: i32) -> bool {
        self.has_slot(slot) || self.entries.iter().any(Option::is_none)
    }
    fn insert(&mut self, entry: Entry) {
        if let Some(slot) = self
            .entries
            .iter_mut()
            .find(|row| row.is_some_and(|row| row.slot == entry.slot))
        {
            *slot = Some(entry);
        } else if let Some(slot) = self.entries.iter_mut().find(|entry| entry.is_none()) {
            *slot = Some(entry);
        }
    }

    fn has_slot(&self, slot: i32) -> bool {
        self.entries
            .iter()
            .flatten()
            .any(|entry| entry.slot == slot)
    }

    fn reconcile(&mut self, cx: &ActionContext<'_>, rows: &[ItemView], settled: &mut bool) -> bool {
        let mut retry_round = false;
        for index in 0..self.entries.len() {
            let Some(entry) = self.entries[index] else {
                continue;
            };
            let row = rows.iter().find(|row| row.slot == entry.slot);
            match row {
                None => {
                    // The only positive proof is an observed empty slot. A
                    // different object is invalidation, not a successful drop.
                    self.dropped = self.dropped.saturating_add(1);
                    self.entries[index] = None;
                    *settled = true;
                }
                Some(row) if row.def.id != entry.expected => {
                    self.entries[index] = None;
                    *settled = true;
                }
                Some(_) => {
                    let refused = entry
                        .request
                        .and_then(|request| cx.interaction_receipt(request))
                        .is_some_and(|receipt| !receipt.accepted);
                    if entry.request.is_some()
                        && cx.evidence().tick > entry.sent_tick
                        && (refused || cx.evidence().tick >= entry.sent_tick.saturating_add(2))
                    {
                        // Keep observing the slot until it is re-sent. A late
                        // empty observation still confirms the original drop.
                        self.entries[index].as_mut().unwrap().request = None;
                        retry_round = true;
                    }
                }
            }
        }
        retry_round
    }

    fn next_candidate<'a>(&self, rows: &'a [ItemView]) -> Option<&'a ItemView> {
        rows.iter().find(|row| {
            self.is_product(row.def.id)
                && !self.is_protected(row.def.id)
                && self.can_track(row.slot)
                && !self
                    .entries
                    .iter()
                    .flatten()
                    .any(|entry| entry.slot == row.slot && entry.request.is_some())
                && row.count > 0
                && row.def.name.is_some()
        })
    }

    fn is_product(&self, id: i32) -> bool {
        self.args.products.contains(&id)
    }

    fn is_protected(&self, id: i32) -> bool {
        self.args.protected[..self.args.protected_len as usize].contains(&id)
    }

    fn remaining(&self, rows: &[ItemView]) -> u16 {
        rows.iter()
            .filter(|row| {
                row.count > 0 && self.is_product(row.def.id) && !self.is_protected(row.def.id)
            })
            .count() as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::HostEffect;
    use crate::quester::families::tests::{def, with_tick, with_tick_snapshots};
    use api::snapshot::{GameSnapshot, ItemActionFamily, ItemContainer};

    #[test]
    fn same_tick_modal_after_disposals_defers_close_until_next_tick() {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(
            vec![ItemView {
                def: def(1511, "Logs"),
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: 0,
                count: 1,
                actions: vec![Some("Drop".into())],
                component_id: -1,
            }],
            28,
        );
        let mut later = GameSnapshot::new();
        later.seed_ingame(2);
        later.seed_inventory(Vec::new(), 28);
        later.seed_chat_modal(100, vec!["Your inventory is full.".into()]);
        let mut ledger = None;
        let handle = with_tick_snapshots(&snapshot, &later, &mut ledger, 1, |tick, later| {
            let handle = tick
                .actions
                .begin::<DropBatch>(
                    DropBatchArgs {
                        products: Arc::from([1511]),
                        protected: [0; MAX_PROTECTED],
                        protected_len: 0,
                    },
                    &mut tick.cx,
                )
                .unwrap();
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            tick.cx.snapshot = api::snapshot::SnapshotView::new(Some(later), tick.cx.evidence());
            for _ in 0..3 {
                assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            }
            assert_eq!(tick.cx.ledger.as_ref().unwrap().outbox.len(), 1);
            handle
        });
        let result = with_tick(&later, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(matches!(
            result,
            Poll::Ready(Ok(DropResult {
                end: DropEnd::Cleared,
                dropped: 1
            }))
        ));
        assert!(matches!(
            &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
            HostEffect::Interaction(InteractReq::CloseModal)
        ));
    }
}
