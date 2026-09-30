//! Snapshot-driven shop purchase machine shared by compiled cards.
//! Queued buttons are not receipts: each batch settles only after the held
//! item count grows.
use crate::native::{ActionContext, ActionError, NativeMachine};
use crate::shim::InteractReq;
use api::snapshot::ItemView;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

const OPEN_BOUND: Duration = Duration::from_secs(10);
const SETTLE_BOUND: Duration = Duration::from_secs(4);
const CLOSE_BOUND: Duration = Duration::from_secs(3);

#[derive(Debug, Clone)]
pub struct BuyRequest {
    pub npc_id: i32,
    pub npc_name: Arc<str>,
    pub item_id: i32,
    pub item_name: Arc<str>,
    /// Desired held count, not an unobserved click count.
    pub qty: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuyReceipt {
    pub held: i32,
}

enum Phase {
    Open,
    AwaitOpen,
    Buy,
    AwaitBatch { before: i32 },
    Close,
    AwaitClose,
}

pub struct BuyMachine {
    request: BuyRequest,
    phase: Phase,
    deadline: Duration,
}

impl NativeMachine for BuyMachine {
    type Args = BuyRequest;
    type Output = BuyReceipt;

    fn begin(request: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        if request.qty < 1 {
            return Err(ActionError::Unavailable(Arc::from("buy quantity must be positive")));
        }
        Ok(Self {
            request,
            phase: Phase::Open,
            deadline: cx.active_now().saturating_add(OPEN_BOUND),
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        loop {
            match self.phase {
                Phase::Open => {
                    if cx.snapshot().shop().is_some_and(|shop| shop.value.open) {
                        self.phase = Phase::Buy;
                        continue;
                    }
                    let Some(npcs) = cx.snapshot().npcs() else {
                        return Poll::Pending;
                    };
                    let npc = npcs.value.iter().filter(|npc| {
                        npc.r#type == usize::try_from(self.request.npc_id).ok()
                            || npc.name.as_deref().is_some_and(|name| {
                                name.eq_ignore_ascii_case(&self.request.npc_name)
                            })
                    }).min_by_key(|npc| npc.distance);
                    let Some(npc) = npc else {
                        return Poll::Pending;
                    };
                    let trade = npc.actions.iter().flatten().find(|action| {
                        action.eq_ignore_ascii_case("Trade")
                            || action.eq_ignore_ascii_case("Trade-with")
                    });
                    let Some(trade) = trade else {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "shop NPC has no Trade action",
                        ))));
                    };
                    cx.emit(InteractReq::Npc {
                        name: npc.name.clone().unwrap_or_else(|| self.request.npc_name.to_string()),
                        action: trade.clone(),
                        index: i32::try_from(npc.index).ok(),
                    })?;
                    self.phase = Phase::AwaitOpen;
                    return Poll::Pending;
                }
                Phase::AwaitOpen => {
                    if cx.snapshot().shop().is_some_and(|shop| shop.value.open) {
                        self.phase = Phase::Buy;
                        continue;
                    }
                    if cx.active_now() >= self.deadline {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "shop did not open",
                        ))));
                    }
                    return Poll::Pending;
                }
                Phase::Buy => {
                    let Some(inventory) = cx.snapshot().inventory() else {
                        return Poll::Pending;
                    };
                    let held = count(inventory.value, self.request.item_id);
                    if held >= self.request.qty {
                        self.phase = Phase::Close;
                        continue;
                    }
                    let Some(shop) = cx.snapshot().shop() else {
                        return Poll::Pending;
                    };
                    if !shop.value.open {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "shop closed during purchase",
                        ))));
                    }
                    let Some(row) = shop
                        .value
                        .stock
                        .iter()
                        .find(|row| row.def.id == self.request.item_id && row.count > 0)
                    else {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "shop lacks requested item",
                        ))));
                    };
                    let remaining = self.request.qty - held;
                    let chunk = if remaining >= 10 && row.count >= 10 {
                        10
                    } else if remaining >= 5 && row.count >= 5 {
                        5
                    } else {
                        1
                    };
                    cx.emit(InteractReq::ShopButton {
                        kind: "buy".into(),
                        name: self.request.item_name.to_string(),
                        id: row.def.id,
                        slot: row.slot,
                        component: row.component_id,
                        chunk,
                    })?;
                    self.deadline = cx.active_now().saturating_add(SETTLE_BOUND);
                    self.phase = Phase::AwaitBatch { before: held };
                    return Poll::Pending;
                }
                Phase::AwaitBatch { before } => {
                    let held = cx
                        .snapshot()
                        .inventory()
                        .map(|inventory| count(inventory.value, self.request.item_id));
                    if held.is_some_and(|held| held > before) {
                        self.phase = Phase::Buy;
                        continue;
                    }
                    if cx.active_now() >= self.deadline {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "shop purchase did not settle",
                        ))));
                    }
                    return Poll::Pending;
                }
                Phase::Close => {
                    if !cx.snapshot().shop().is_some_and(|shop| shop.value.open) {
                        return Poll::Ready(Ok(self.receipt(cx)));
                    }
                    cx.emit(InteractReq::CloseModal)?;
                    self.deadline = cx.active_now().saturating_add(CLOSE_BOUND);
                    self.phase = Phase::AwaitClose;
                    return Poll::Pending;
                }
                Phase::AwaitClose => {
                    if !cx.snapshot().shop().is_some_and(|shop| shop.value.open) {
                        return Poll::Ready(Ok(self.receipt(cx)));
                    }
                    if cx.active_now() >= self.deadline {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "shop did not close",
                        ))));
                    }
                    return Poll::Pending;
                }
            }
        }
    }

    fn cancel(&mut self) {}
}

impl BuyMachine {
    fn receipt(&self, cx: &ActionContext<'_>) -> BuyReceipt {
        BuyReceipt {
            held: cx
                .snapshot()
                .inventory()
                .map_or(0, |rows| count(rows.value, self.request.item_id)),
        }
    }
}

fn count(rows: &[ItemView], id: i32) -> i32 {
    rows.iter()
        .filter(|row| row.def.id == id)
        .map(|row| row.count)
        .sum()
}
