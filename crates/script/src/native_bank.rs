//! Snapshot-driven bank core shared by compiled cards.
//!
//! Selection owns immutable [`NamedBankFacts`] explicitly. Operations never
//! trust dispatch receipts as transfers: every withdraw/deposit settles from a
//! newer inventory or bank-side observation in the same bank session.
use crate::native::{ActionContext, ActionError, NativeMachine};
use crate::shim::InteractReq;
use api::named_banks::{BankPreferences, NamedBank, NamedBankFacts};
use api::snapshot::{ItemView, WorldTile};
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

const OPEN_BOUND: Duration = Duration::from_secs(12);
const TRANSFER_BOUND: Duration = Duration::from_secs(4);
const MAX_MEMO: usize = 64;
const MAX_DEPOSITS: u8 = 32;

#[derive(Clone)]
pub struct BankSelector {
    facts: Arc<NamedBankFacts>,
    preferences: BankPreferences,
}

impl BankSelector {
    pub fn new(facts: Arc<NamedBankFacts>, preferences: BankPreferences) -> Self {
        Self { facts, preferences }
    }

    pub fn named(&self, name: &str) -> Option<NamedBank> {
        self.facts
            .banks()
            .iter()
            .find(|bank| bank.name.eq_ignore_ascii_case(name) && self.eligible(bank))
            .copied()
    }

    pub fn at(&self, tile: WorldTile) -> Option<NamedBank> {
        self.facts
            .banks()
            .iter()
            .find(|bank| bank.tile == tile && self.eligible(bank))
            .copied()
    }

    pub fn nearest(&self, from: WorldTile) -> Option<NamedBank> {
        self.facts
            .banks()
            .iter()
            .filter(|bank| bank.routable && self.eligible(bank))
            .min_by_key(|bank| distance_squared(from, bank.air_tile()))
            .copied()
    }

    fn eligible(&self, bank: &NamedBank) -> bool {
        bank.eligible(|_| None, |_| false, self.preferences)
    }
}

fn distance_squared(a: WorldTile, b: WorldTile) -> i64 {
    let dx = i64::from(a.x) - i64::from(b.x);
    let dz = i64::from(a.z) - i64::from(b.z);
    dx * dx + dz * dz
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankItem {
    pub id: i32,
    pub name: Arc<str>,
}

#[derive(Debug, Clone)]
pub enum BankAction {
    Scan,
    Withdraw {
        item: BankItem,
        qty: i32,
    },
    /// First listed, held-or-banked alternative. Callers order strongest to
    /// weakest after applying stat/quest gates.
    WithdrawAny {
        items: Arc<[BankItem]>,
        qty: i32,
    },
    Deposit {
        item: BankItem,
    },
    DepositAll {
        keep: Arc<[i32]>,
    },
    Close,
}

#[derive(Debug, Clone)]
pub struct BankRequest {
    pub bank: NamedBank,
    pub action: BankAction,
    /// Only these active-Path items enter the runner memo.
    pub memo_ids: Arc<[i32]>,
    pub partial_ok: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BankCount {
    pub id: i32,
    pub count: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankReceipt {
    pub counts: Vec<BankCount>,
    pub complete: bool,
}

enum Phase {
    Open,
    AwaitOpen,
    Act,
    AwaitTransfer { before: i32 },
    AwaitClose,
}

pub struct BankMachine {
    request: BankRequest,
    phase: Phase,
    deadline: Duration,
    session: u64,
    deposits: u8,
}

impl NativeMachine for BankMachine {
    type Args = BankRequest;
    type Output = BankReceipt;

    fn begin(request: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        if request.memo_ids.len() > MAX_MEMO {
            return Err(ActionError::Unavailable(Arc::from(
                "bank memo exceeds 64 items",
            )));
        }
        Ok(Self {
            request,
            phase: Phase::Open,
            deadline: cx.active_now().saturating_add(OPEN_BOUND),
            session: 0,
            deposits: 0,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        loop {
            match self.phase {
                Phase::Open => {
                    if matches!(self.request.action, BankAction::Close) {
                        let open = cx
                            .snapshot()
                            .bank_session()
                            .is_some_and(|session| session.value.open);
                        if !open {
                            return Poll::Ready(Ok(self.receipt(cx, true)));
                        }
                        cx.emit(InteractReq::Close)?;
                        self.deadline = cx.active_now().saturating_add(TRANSFER_BOUND);
                        self.phase = Phase::AwaitClose;
                        return Poll::Pending;
                    }
                    if cx.snapshot().bank().is_some() {
                        self.session = cx
                            .snapshot()
                            .bank_session()
                            .map_or(0, |session| session.value.generation);
                        self.phase = Phase::Act;
                        continue;
                    }
                    if cx.active_now() >= self.deadline {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "bank did not open",
                        ))));
                    }
                    let Some(locs) = cx.snapshot().locs() else {
                        return Poll::Pending;
                    };
                    let wanted = self
                        .request
                        .bank
                        .definition
                        .and_then(|definition| definition.object);
                    let booth = locs
                        .value
                        .iter()
                        .filter(|loc| {
                            loc.tile.level == self.request.bank.tile.level
                                && loc.actions.iter().flatten().any(|action| {
                                    wanted.map_or_else(
                                        || action.eq_ignore_ascii_case("Use-quickly"),
                                        |wanted| action.eq_ignore_ascii_case(wanted.op),
                                    )
                                })
                        })
                        .min_by_key(|loc| loc.distance);
                    let Some(booth) = booth else {
                        return Poll::Pending;
                    };
                    let action = wanted.map(|row| row.op.to_string());
                    let name = wanted.map(|row| row.name.to_string());
                    cx.emit(InteractReq::OpenBooth {
                        x: booth.tile.x,
                        z: booth.tile.z,
                        level: booth.tile.level,
                        id: booth.id,
                        name,
                        action,
                    })?;
                    self.phase = Phase::AwaitOpen;
                    return Poll::Pending;
                }
                Phase::AwaitOpen => {
                    if let Some(bank) = cx.snapshot().bank() {
                        self.session = cx
                            .snapshot()
                            .bank_session()
                            .map_or(0, |session| session.value.generation);
                        let _ = bank;
                        self.phase = Phase::Act;
                        continue;
                    }
                    if cx.active_now() >= self.deadline {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "bank item table unavailable",
                        ))));
                    }
                    return Poll::Pending;
                }
                Phase::Act => match &self.request.action {
                    BankAction::Scan => return Poll::Ready(Ok(self.receipt(cx, true))),
                    BankAction::Withdraw { item, qty } => {
                        let snapshot = cx.snapshot();
                        let Some(inv) = snapshot.inventory() else {
                            return Poll::Pending;
                        };
                        let held = count(inv.value, item.id);
                        if held >= *qty {
                            return Poll::Ready(Ok(self.receipt(cx, true)));
                        }
                        let Some(bank) = snapshot.bank() else {
                            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                "bank closed during withdraw",
                            ))));
                        };
                        let Some(row) = bank.value.iter().find(|row| row.def.id == item.id) else {
                            let receipt = self.receipt(cx, false);
                            return if self.request.partial_ok {
                                Poll::Ready(Ok(receipt))
                            } else {
                                Poll::Ready(Err(ActionError::Failed(Arc::from(
                                    "bank lacks requested item",
                                ))))
                            };
                        };
                        let bank_item_id = row.def.id;
                        let need = (*qty - held).min(row.count).max(0);
                        if need == 0 {
                            return Poll::Ready(Ok(self.receipt(cx, self.request.partial_ok)));
                        }
                        let request = if need > 10 {
                            InteractReq::WithdrawX {
                                name: item.name.to_string(),
                                count: need,
                                bank_item_id,
                                lands_as_id: item.id,
                                action: "Withdraw-X".into(),
                                bank_generation: self.session,
                            }
                        } else {
                            let action = if need >= 10 {
                                "Withdraw-10"
                            } else if need >= 5 {
                                "Withdraw-5"
                            } else {
                                "Withdraw-1"
                            };
                            InteractReq::Withdraw {
                                name: item.name.to_string(),
                                action: action.into(),
                            }
                        };
                        cx.emit(request)?;
                        self.deadline = cx.active_now().saturating_add(TRANSFER_BOUND);
                        self.phase = Phase::AwaitTransfer { before: held };
                        return Poll::Pending;
                    }
                    BankAction::WithdrawAny { items, qty } => {
                        let snapshot = cx.snapshot();
                        let Some(inv) = snapshot.inventory() else {
                            return Poll::Pending;
                        };
                        if items.iter().any(|item| count(inv.value, item.id) >= *qty) {
                            return Poll::Ready(Ok(self.receipt(cx, true)));
                        }
                        let Some(bank) = snapshot.bank() else {
                            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                "bank closed during tier withdraw",
                            ))));
                        };
                        let Some((item, bank_item_id, available)) = items.iter().find_map(|item| {
                            bank.value
                                .iter()
                                .find(|row| row.def.id == item.id && row.count > 0)
                                .map(|row| (item, row.def.id, row.count))
                        }) else {
                            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                "bank lacks every permitted loadout tier",
                            ))));
                        };
                        let before = count(inv.value, item.id);
                        let need = (*qty - before).min(available).max(0);
                        let request = if need > 10 {
                            InteractReq::WithdrawX {
                                name: item.name.to_string(),
                                count: need,
                                bank_item_id,
                                lands_as_id: item.id,
                                action: "Withdraw-X".into(),
                                bank_generation: self.session,
                            }
                        } else {
                            let action = if need >= 10 {
                                "Withdraw-10"
                            } else if need >= 5 {
                                "Withdraw-5"
                            } else {
                                "Withdraw-1"
                            };
                            InteractReq::Withdraw {
                                name: item.name.to_string(),
                                action: action.into(),
                            }
                        };
                        cx.emit(request)?;
                        self.deadline = cx.active_now().saturating_add(TRANSFER_BOUND);
                        self.phase = Phase::AwaitTransfer { before };
                        return Poll::Pending;
                    }
                    BankAction::Deposit { item } => {
                        let snapshot = cx.snapshot();
                        let Some(side) = snapshot.bank_side() else {
                            return Poll::Pending;
                        };
                        let held = count(side.value, item.id);
                        if held == 0 {
                            return Poll::Ready(Ok(self.receipt(cx, true)));
                        }
                        cx.emit(InteractReq::Deposit {
                            name: item.name.to_string(),
                        })?;
                        self.deadline = cx.active_now().saturating_add(TRANSFER_BOUND);
                        self.phase = Phase::AwaitTransfer { before: held };
                        return Poll::Pending;
                    }
                    BankAction::DepositAll { keep } => {
                        if self.deposits >= MAX_DEPOSITS {
                            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                "deposit-all exceeded 32 item rows",
                            ))));
                        }
                        let snapshot = cx.snapshot();
                        let Some(side) = snapshot.bank_side() else {
                            return Poll::Pending;
                        };
                        let row = side
                            .value
                            .iter()
                            .find(|row| row.count > 0 && !keep.iter().any(|id| *id == row.def.id));
                        let Some(row) = row else {
                            return Poll::Ready(Ok(self.receipt(cx, true)));
                        };
                        let Some(name) = row.def.name.clone() else {
                            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                "deposit row has no resolved name",
                            ))));
                        };
                        let before = side
                            .value
                            .iter()
                            .filter(|row| !keep.iter().any(|id| *id == row.def.id))
                            .map(|row| row.count)
                            .sum();
                        cx.emit(InteractReq::Deposit { name })?;
                        self.deposits += 1;
                        self.deadline = cx.active_now().saturating_add(TRANSFER_BOUND);
                        self.phase = Phase::AwaitTransfer { before };
                        return Poll::Pending;
                    }
                    BankAction::Close => unreachable!("close is handled before opening"),
                },
                Phase::AwaitTransfer { before } => {
                    let current = match &self.request.action {
                        BankAction::Withdraw { item, .. } => cx
                            .snapshot()
                            .inventory()
                            .map(|rows| count(rows.value, item.id)),
                        BankAction::WithdrawAny { items, .. } => {
                            cx.snapshot().inventory().map(|rows| {
                                items
                                    .iter()
                                    .map(|item| count(rows.value, item.id))
                                    .max()
                                    .unwrap_or(0)
                            })
                        }
                        BankAction::Deposit { item } => cx
                            .snapshot()
                            .bank_side()
                            .map(|rows| count(rows.value, item.id)),
                        BankAction::DepositAll { keep } => cx.snapshot().bank_side().map(|rows| {
                            rows.value
                                .iter()
                                .filter(|row| !keep.iter().any(|id| *id == row.def.id))
                                .map(|row| row.count)
                                .sum()
                        }),
                        BankAction::Scan | BankAction::Close => Some(before),
                    };
                    let moved = match &self.request.action {
                        BankAction::Withdraw { .. } | BankAction::WithdrawAny { .. } => {
                            current.is_some_and(|now| now > before)
                        }
                        _ => current.is_some_and(|now| now < before),
                    };
                    if moved {
                        self.phase = Phase::Act;
                        continue;
                    }
                    let same_session = cx.snapshot().bank_session().is_some_and(|session| {
                        session.value.open && session.value.generation == self.session
                    });
                    if !same_session || cx.active_now() >= self.deadline {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "bank transfer did not settle",
                        ))));
                    }
                    return Poll::Pending;
                }
                Phase::AwaitClose => {
                    let open = cx
                        .snapshot()
                        .bank_session()
                        .is_some_and(|session| session.value.open);
                    if !open {
                        return Poll::Ready(Ok(self.receipt(cx, true)));
                    }
                    if cx.active_now() >= self.deadline {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "bank did not close",
                        ))));
                    }
                    return Poll::Pending;
                }
            }
        }
    }

    fn cancel(&mut self) {}
}

impl BankMachine {
    fn receipt(&self, cx: &ActionContext<'_>, complete: bool) -> BankReceipt {
        let snapshot = cx.snapshot();
        let bank = snapshot.bank();
        let counts = self
            .request
            .memo_ids
            .iter()
            .map(|id| BankCount {
                id: *id,
                count: bank.map_or(0, |rows| count(rows.value, *id)),
            })
            .collect();
        BankReceipt { counts, complete }
    }
}

fn count(rows: &[ItemView], id: i32) -> i32 {
    rows.iter()
        .filter(|row| row.def.id == id)
        .map(|row| row.count)
        .sum()
}
