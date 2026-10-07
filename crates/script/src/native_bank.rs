//! Snapshot-driven bank core shared by compiled cards.
//!
//! Selection owns immutable [`NamedBankFacts`] explicitly. Operations never
//! trust dispatch receipts as transfers: every withdraw/deposit settles from a
//! newer inventory observation in the same bank session. Click choice,
//! completion and deposit settlement are the shared [`crate::bank::ops`]
//! kernel.
use crate::bank::npc;
use crate::bank::ops::{
    self, CloseBaseline, CloseScan, DepositKind, DepositScan, DepositSpec, NoteIntent, Progress,
    WithdrawGoal, DEPOSIT_VIEW_MS, MAX_DEPOSITS, MAX_MEMO, TRANSFER_BOUND,
};
use crate::native::{ActionContext, ActionError, NativeMachine, WalkOptions};
use crate::shim::InteractReq;
use api::named_banks::{BankOperation, BankPreferences, NamedBank, NamedBankFacts};
use api::quest_progress::EvidenceStamp;
use api::selected::Truth;
use api::snapshot::{LocView, QuestListStatus, QuestStatusView, StatView, WorldTile};
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

const OPEN_BOUND: Duration = Duration::from_secs(12);

#[derive(Clone)]
pub struct BankSelector {
    facts: Arc<NamedBankFacts>,
    preferences: BankPreferences,
}

impl BankSelector {
    pub fn new(facts: Arc<NamedBankFacts>, preferences: BankPreferences) -> Self {
        Self { facts, preferences }
    }

    fn eligible(&self, bank: &NamedBank, stats: &[StatView], quests: &[QuestStatusView]) -> bool {
        bank.eligible(
            |id| {
                stats
                    .iter()
                    .find(|stat| stat.index == id)
                    .map(|stat| stat.base)
            },
            |name| {
                quests.iter().any(|quest| {
                    quest.name.eq_ignore_ascii_case(name)
                        && quest.status() == QuestListStatus::Complete
                })
            },
            self.preferences,
        )
    }

    /// Return eligible roster indices in catalog order. Unknown live skill or
    /// quest facts fail only the banks gated by those facts; preference gates
    /// are always evaluated explicitly.
    pub fn eligible_indices(&self, stats: &[StatView], quests: &[QuestStatusView]) -> Vec<u16> {
        self.facts
            .banks()
            .iter()
            .enumerate()
            .filter_map(|(index, bank)| {
                let index = u16::try_from(index)
                    .ok()
                    .filter(|index| *index != u16::MAX)?;
                self.eligible(bank, stats, quests).then_some(index)
            })
            .collect()
    }

    pub fn named(&self, name: &str, stats: &[StatView], quests: &[QuestStatusView]) -> Option<u16> {
        self.facts
            .banks()
            .iter()
            .enumerate()
            .find_map(|(index, bank)| {
                if !bank.name.eq_ignore_ascii_case(name) || !self.eligible(bank, stats, quests) {
                    return None;
                }
                u16::try_from(index).ok().filter(|index| *index != u16::MAX)
            })
    }

    pub fn facts(&self) -> &NamedBankFacts {
        &self.facts
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PickKind {
    NearShortcut = 1,
    Reachable = 2,
    AirFallback = 3,
    NoCandidate = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AccessKind {
    Booth = 1,
    Teller = 2,
}

impl AccessKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Booth => "booth",
            Self::Teller => "npc",
        }
    }
}

/// The packed M-279 stand selected by the host worker, plus its access tile.
/// It is shared between the pick receipt and the subsequent Open machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankStandAccess {
    pub bank: NamedBank,
    pub stand_tile: WorldTile,
    pub kind: AccessKind,
    pub stand_op: i32,
    pub name: Option<Arc<str>>,
    pub choose: Option<Arc<str>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedBank {
    pub bank_index: u16,
    pub access_tile: WorldTile,
    pub kind: PickKind,
    pub access: Option<Arc<BankStandAccess>>,
}

impl SelectedBank {
    fn no_candidate() -> Self {
        Self {
            bank_index: u16::MAX,
            access_tile: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            kind: PickKind::NoCandidate,
            access: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BankPickRequest {
    pub facts: Arc<NamedBankFacts>,
    pub from: WorldTile,
    pub preferences: BankPreferences,
    pub options: WalkOptions,
    /// Roster indices already proven eligible from this run's live facts.
    pub eligible: Arc<[u16]>,
    /// When present, no other eligible bank may be selected.
    pub explicit_bank: Option<u16>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankPickReceipt {
    pub request_id: u64,
    pub evidence: EvidenceStamp,
    pub selected: SelectedBank,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Withdrawal {
    pub id: i32,
    pub name: Arc<str>,
    /// Exact final inventory count, not an amount to add.
    pub target: i32,
}

pub struct SelectArgs {
    pub facts: Arc<NamedBankFacts>,
    pub from: WorldTile,
    pub preferences: BankPreferences,
    pub options: WalkOptions,
    pub explicit: Option<Arc<str>>,
}

pub struct OpenArgs {
    pub access: Arc<BankStandAccess>,
}

pub struct DepositArgs {
    pub products: Arc<[i32]>,
    pub keep: Arc<[i32]>,
}

pub struct WithdrawArgs {
    pub withdrawals: Arc<[Withdrawal]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankItem {
    pub id: i32,
    pub name: Arc<str>,
}

#[derive(Debug, Clone)]
pub enum BankAction {
    Scan,
    OpenStand {
        access: Arc<BankStandAccess>,
    },
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
    WithdrawTo {
        withdrawals: Arc<[Withdrawal]>,
    },
    Deposit {
        item: BankItem,
    },
    DepositAll {
        keep: Arc<[i32]>,
    },
    DepositProducts {
        products: Arc<[i32]>,
        keep: Arc<[i32]>,
    },
    /// Deposit exactly `slots` safe inventory rows, preserving all `keep` ids.
    DepositCapacity {
        keep: Arc<[i32]>,
        slots: u8,
    },
    Close,
}

#[derive(Debug, Clone)]
pub struct BankRequest {
    pub bank: Option<NamedBank>,
    pub action: BankAction,
    pub partial_ok: bool,
}

/// One protected pack row's count before a deposit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BankCount {
    pub id: i32,
    pub count: i32,
}

/// What a bank action settled to. The bank's rows are not carried here: the
/// host observes the open table into the account's bank memory
/// (design-bank-snapshot §1.3), which every reader borrows through
/// `SnapshotView::stock()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankReceipt {
    pub complete: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AccessDialogue {
    Teller,
    Object,
}

#[derive(Clone, Copy)]
enum Phase {
    Open,
    NpcAccess {
        core: npc::NpcAccess,
        dialogue: AccessDialogue,
    },
    AwaitOpen,
    AwaitNoteMode {
        request_id: u64,
        evidence: EvidenceStamp,
    },
    Act,
    AwaitTransfer {
        before: i32,
        before_slots: Option<usize>,
        item_id: i32,
        evidence: EvidenceStamp,
    },
    /// The Close verb's bank session and side root, and the login run
    /// that sent it.
    AwaitClose {
        baseline: CloseBaseline,
        run: api::selected::RunKey,
    },
}

pub struct BankMachine {
    request: BankRequest,
    phase: Phase,
    deadline: Duration,
    /// Fixed opening bound, separate from the per-step `deadline` armed by dialogue.
    open_deadline: Duration,
    session: u64,
    deposits: u8,
    withdraw_index: usize,
    protected_before: Vec<BankCount>,
    deposited: u32,
    capacity_freed: u8,
    item_mode_ensured: bool,
    open_stage: u8,
    open_first_target: Option<(i32, WorldTile)>,
    object_interaction_sent: bool,
    /// A completed object dialogue cannot be restarted just because its modal remains open.
    object_dialogue_finished: bool,
    /// The until-empty side-view bound is armed in `deadline`.
    view_armed: bool,
    /// The bank session generation seen when this machine sent its open
    /// (`u64::MAX` before any session). A later generation is a session this
    /// open created, which the server starts in Item mode.
    opened_from: Option<u64>,
}

const BANK_OBJECT_RADIUS: i32 = 4;
fn catalog_object_matches(loc: &LocView, operation: BankOperation) -> bool {
    loc.name
        .as_deref()
        .is_some_and(|name| name.eq_ignore_ascii_case(operation.name))
        && loc
            .actions
            .iter()
            .flatten()
            .any(|action| action.eq_ignore_ascii_case(operation.op))
}

fn catalog_object_at<'a>(
    cx: &'a ActionContext<'_>,
    id: i32,
    tile: WorldTile,
    operation: BankOperation,
) -> Option<&'a LocView> {
    cx.snapshot()
        .locs()?
        .value
        .iter()
        .find(|loc| loc.id == id && loc.tile == tile && catalog_object_matches(loc, operation))
}

/// Resolve a catalog object's live row around its declared access stand.
fn catalog_object_loc<'a>(
    cx: &'a ActionContext<'_>,
    stand_tile: WorldTile,
    operation: BankOperation,
) -> Option<&'a LocView> {
    cx.snapshot()
        .locs()?
        .value
        .iter()
        .filter(|loc| {
            loc.tile.level == stand_tile.level
                && loc
                    .tile
                    .x
                    .abs_diff(stand_tile.x)
                    .max(loc.tile.z.abs_diff(stand_tile.z))
                    <= BANK_OBJECT_RADIUS as u32
                && catalog_object_matches(loc, operation)
        })
        .min_by_key(|loc| {
            (
                loc.tile
                    .x
                    .abs_diff(stand_tile.x)
                    .max(loc.tile.z.abs_diff(stand_tile.z)),
                loc.id,
            )
        })
}

impl NativeMachine for BankMachine {
    type Args = BankRequest;
    type Output = BankReceipt;

    fn begin(request: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        let protected_capacity = match &request.action {
            BankAction::DepositProducts { keep, .. } | BankAction::DepositCapacity { keep, .. } => {
                keep.len()
            }
            _ => 0,
        };
        let mut protected_before = Vec::with_capacity(protected_capacity);
        match &request.action {
            BankAction::DepositProducts { products, keep } => {
                if products.len() + keep.len() > MAX_MEMO {
                    return Err(ActionError::Unavailable(Arc::from(
                        "bank product/keep set exceeds 64 items",
                    )));
                }
                let inventory = cx
                    .snapshot()
                    .inventory()
                    .ok_or_else(|| ActionError::Unavailable(Arc::from("inventory unavailable")))?;
                for id in keep.iter().copied() {
                    if protected_before.iter().any(|row: &BankCount| row.id == id) {
                        continue;
                    }
                    protected_before.push(BankCount {
                        id,
                        count: ops::count_id(inventory.value, id),
                    });
                }
            }
            BankAction::DepositCapacity { keep, slots } => {
                if keep.len() > MAX_MEMO || *slots == 0 || *slots > MAX_DEPOSITS {
                    return Err(ActionError::Unavailable(Arc::from(
                        "invalid exact bank capacity deposit plan",
                    )));
                }
                let inventory = cx
                    .snapshot()
                    .inventory()
                    .ok_or_else(|| ActionError::Unavailable(Arc::from("inventory unavailable")))?;
                for id in keep.iter().copied() {
                    if protected_before.iter().any(|row: &BankCount| row.id == id) {
                        continue;
                    }
                    protected_before.push(BankCount {
                        id,
                        count: ops::count_id(inventory.value, id),
                    });
                }
            }
            BankAction::WithdrawTo { withdrawals } => {
                if withdrawals.len() > MAX_MEMO
                    || withdrawals
                        .iter()
                        .any(|row| row.target < 0 || row.name.is_empty())
                {
                    return Err(ActionError::Unavailable(Arc::from(
                        "invalid exact bank withdrawal plan",
                    )));
                }
                if withdrawals
                    .iter()
                    .enumerate()
                    .any(|(index, row)| withdrawals[..index].iter().any(|prior| prior.id == row.id))
                {
                    return Err(ActionError::Unavailable(Arc::from(
                        "duplicate item in exact bank withdrawal plan",
                    )));
                }
            }
            BankAction::DepositAll { keep } if keep.len() > MAX_MEMO => {
                return Err(ActionError::Unavailable(Arc::from(
                    "bank keep set exceeds 64 items",
                )));
            }
            _ => {}
        }
        let open_deadline = cx.active_now().saturating_add(OPEN_BOUND);
        Ok(Self {
            request,
            phase: Phase::Open,
            deadline: open_deadline,
            open_deadline,
            session: 0,
            deposits: 0,
            withdraw_index: 0,
            protected_before,
            deposited: 0,
            capacity_freed: 0,
            item_mode_ensured: false,
            open_stage: 0,
            open_first_target: None,
            object_interaction_sent: false,
            object_dialogue_finished: false,
            view_armed: false,
            opened_from: None,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        loop {
            match self.phase {
                Phase::Open => {
                    if matches!(self.request.action, BankAction::Close) {
                        let Some(session) = cx.snapshot().bank_session().map(|s| s.value) else {
                            return Poll::Ready(Ok(self.receipt(true)));
                        };
                        if ops::close_begin(session.open).is_some() {
                            return Poll::Ready(Ok(self.receipt(true)));
                        }
                        let run = cx.evidence().run;
                        std::task::ready!(crate::native::defer_budget(
                            cx.emit(InteractReq::Close)
                        ))?;
                        self.deadline = cx.active_now().saturating_add(ops::close_deadline(None));
                        self.phase = Phase::AwaitClose {
                            baseline: CloseBaseline {
                                generation: session.generation,
                                side: session.side,
                            },
                            run,
                        };
                        return Poll::Pending;
                    }
                    if matches!(
                        &self.request.action,
                        BankAction::OpenStand { access } if access.kind == AccessKind::Teller
                    ) {
                        self.opened_from = Some(open_generation(cx));
                        self.phase = Phase::NpcAccess {
                            core: npc::NpcAccess::new(),
                            dialogue: AccessDialogue::Teller,
                        };
                        continue;
                    }
                    if cx.snapshot().bank().is_some() {
                        self.session = cx
                            .snapshot()
                            .bank_session()
                            .map_or(0, |session| session.value.generation);
                        self.phase = Phase::Act;
                        continue;
                    }
                    if cx
                        .snapshot()
                        .bank_session()
                        .is_some_and(|session| session.value.open)
                    {
                        self.phase = Phase::AwaitOpen;
                        continue;
                    }
                    if cx.active_now() >= self.open_deadline {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "bank did not open",
                        ))));
                    }
                    if let BankAction::OpenStand { access } = &self.request.action {
                        let open_first = access
                            .bank
                            .definition
                            .and_then(|definition| definition.open_first);
                        let opening_first = self.open_stage == 0 && open_first.is_some();
                        let operation =
                            (if opening_first { open_first } else { None }).or_else(|| {
                                access
                                    .bank
                                    .definition
                                    .and_then(|definition| definition.object)
                            });
                        if let Some(operation) = operation {
                            let Some(candidate) =
                                catalog_object_loc(cx, access.stand_tile, operation)
                            else {
                                return Poll::Pending;
                            };
                            // A player-radius area and strict reachability
                            // keep the old `reachable_only` object probe.
                            let Some(loc) = crate::quester::families::reach::choose_loc(
                                cx,
                                Some(candidate.id),
                                None,
                                Some(operation.op),
                                crate::quester::families::reach::Area::new(
                                    None,
                                    BANK_OBJECT_RADIUS,
                                ),
                                Some(candidate.tile),
                                &crate::quester::families::reach::Avoid::default(),
                            )
                            .reachable() else {
                                return Poll::Pending;
                            };
                            let first_target = opening_first.then_some((loc.id, loc.tile));
                            std::task::ready!(crate::native::defer_budget(cx.emit(
                                InteractReq::Loc {
                                    x: loc.tile.x,
                                    z: loc.tile.z,
                                    level: loc.tile.level,
                                    action: operation.op.to_owned(),
                                    id: Some(loc.id),
                                },
                            )))?;
                            if let Some(target) = first_target {
                                self.open_first_target = Some(target);
                            }
                            self.object_interaction_sent = true;
                            self.object_dialogue_finished = false;
                        } else {
                            let Some(locs) = cx.snapshot().locs() else {
                                return Poll::Pending;
                            };
                            let wanted_name = access.name.as_deref();
                            let Some(loc) = locs.value.iter().find(|loc| {
                                loc.tile == access.stand_tile
                                    && wanted_name.is_none_or(|wanted| {
                                        loc.name.as_deref().is_some_and(|actual| {
                                            actual.eq_ignore_ascii_case(wanted)
                                        })
                                    })
                            }) else {
                                return Poll::Pending;
                            };
                            let stand_op = if access.stand_op > 0 {
                                Some(access.stand_op)
                            } else {
                                loc.actions
                                    .iter()
                                    .position(|action| {
                                        action.as_deref().is_some_and(|action| {
                                            action.eq_ignore_ascii_case("Use-quickly")
                                        })
                                    })
                                    .and_then(|index| i32::try_from(index + 1).ok())
                            };
                            let Some(stand_op) = stand_op else {
                                return Poll::Pending;
                            };
                            std::task::ready!(crate::native::defer_budget(cx.emit(
                                InteractReq::OpenStand {
                                    x: loc.tile.x,
                                    z: loc.tile.z,
                                    level: loc.tile.level,
                                    kind: access.kind.as_str().to_owned(),
                                    name: loc.name.as_deref().map(str::to_owned),
                                    stand_op: Some(stand_op),
                                    choose: access.choose.as_deref().map(str::to_owned),
                                },
                            )))?;
                        }
                        self.opened_from = Some(open_generation(cx));
                        self.phase = Phase::AwaitOpen;
                        return Poll::Pending;
                    }
                    let Some(bank) = self.request.bank else {
                        return Poll::Ready(Err(ActionError::Unavailable(Arc::from(
                            "bank open requires a selected stand",
                        ))));
                    };
                    let Some(locs) = cx.snapshot().locs() else {
                        return Poll::Pending;
                    };
                    let wanted = bank.definition.and_then(|definition| definition.object);
                    let booth = locs
                        .value
                        .iter()
                        .filter(|loc| {
                            loc.tile.level == bank.tile.level
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
                    std::task::ready!(crate::native::defer_budget(cx.emit(
                        InteractReq::OpenBooth {
                            x: booth.tile.x,
                            z: booth.tile.z,
                            level: booth.tile.level,
                            id: booth.id,
                            name,
                            action,
                        },
                    )))?;
                    self.opened_from = Some(open_generation(cx));
                    self.phase = Phase::AwaitOpen;
                    return Poll::Pending;
                }
                Phase::NpcAccess { mut core, dialogue } => {
                    if dialogue == AccessDialogue::Object
                        && cx.active_now() >= self.open_deadline
                        && cx.snapshot().bank().is_none()
                    {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "bank item table unavailable",
                        ))));
                    }
                    let BankAction::OpenStand { access } = &self.request.action else {
                        unreachable!("bank dialogue only belongs to OpenStand");
                    };
                    let intent = match dialogue {
                        AccessDialogue::Teller => npc_intent(access)?,
                        AccessDialogue::Object => npc::Intent {
                            name: "",
                            op: npc::NpcOp::Index(-1),
                            choose: "",
                        },
                    };
                    let deadline = self.deadline;
                    let step = match crate::native::defer_budget(core.step(
                        intent,
                        &mut NativeNpcContext {
                            cx,
                            deadline: &mut self.deadline,
                        },
                    )) {
                        Poll::Pending => {
                            self.deadline = deadline;
                            return Poll::Pending;
                        }
                        Poll::Ready(result) => result?,
                    };
                    match step {
                        npc::Step::Wait => {
                            self.phase = Phase::NpcAccess { core, dialogue };
                            return Poll::Pending;
                        }
                        npc::Step::Note(npc::Note::Unloaded) => {
                            let message = if dialogue == AccessDialogue::Teller {
                                "bank teller opened without a loaded item list"
                            } else {
                                "bank object opened without a loaded item list"
                            };
                            return Poll::Ready(Err(ActionError::Unavailable(Arc::from(message))));
                        }
                        npc::Step::Note(
                            npc::Note::NoBanker | npc::Note::NoDialogue | npc::Note::NotOpened,
                        ) => {
                            self.phase = Phase::NpcAccess { core, dialogue };
                            continue;
                        }
                        npc::Step::Done(false) => {
                            if dialogue == AccessDialogue::Object {
                                self.object_dialogue_finished = true;
                                self.phase = Phase::AwaitOpen;
                                continue;
                            }
                            return Poll::Ready(Err(ActionError::Unavailable(Arc::from(
                                "bank teller did not open the bank",
                            ))));
                        }
                        npc::Step::Done(true) => {
                            let snapshot = cx.snapshot();
                            let Some(session) = snapshot.bank_session() else {
                                let message = if dialogue == AccessDialogue::Teller {
                                    "bank teller did not establish an open session"
                                } else {
                                    "bank object did not establish an open session"
                                };
                                return Poll::Ready(Err(ActionError::Unavailable(Arc::from(
                                    message,
                                ))));
                            };
                            if !session.value.open {
                                let message = if dialogue == AccessDialogue::Teller {
                                    "bank teller did not establish an open session"
                                } else {
                                    "bank object did not establish an open session"
                                };
                                return Poll::Ready(Err(ActionError::Unavailable(Arc::from(
                                    message,
                                ))));
                            }
                            if snapshot.bank().is_none() {
                                let message = if dialogue == AccessDialogue::Teller {
                                    "bank teller opened without a loaded item list"
                                } else {
                                    "bank object opened without a loaded item list"
                                };
                                return Poll::Ready(Err(ActionError::Unavailable(Arc::from(
                                    message,
                                ))));
                            }
                            self.session = session.value.generation;
                            self.note_fresh_session(cx);
                            self.phase = Phase::Act;
                            continue;
                        }
                    }
                }
                Phase::AwaitOpen => {
                    if cx.snapshot().bank().is_some() {
                        self.session = cx
                            .snapshot()
                            .bank_session()
                            .map_or(0, |session| session.value.generation);
                        self.note_fresh_session(cx);
                        self.phase = Phase::Act;
                        continue;
                    }
                    if cx.active_now() >= self.open_deadline {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "bank item table unavailable",
                        ))));
                    }
                    let object_access = matches!(
                        &self.request.action,
                        BankAction::OpenStand { access }
                            if access.kind == AccessKind::Booth
                                && access.bank.definition.is_some_and(|definition| {
                                    definition.open_first.or(definition.object).is_some()
                                })
                    );
                    let chat_open = cx.snapshot().chat_modal().is_some_and(|chat| {
                        chat.value.root != -1 || chat.value.continue_component_id >= 0
                    });
                    if self.object_interaction_sent
                        && !self.object_dialogue_finished
                        && object_access
                        && chat_open
                    {
                        self.phase = Phase::NpcAccess {
                            core: npc::NpcAccess::new(),
                            dialogue: AccessDialogue::Object,
                        };
                        continue;
                    }
                    if let BankAction::OpenStand { access } = &self.request.action {
                        if self.open_stage == 0 {
                            if let Some(open_first) = access
                                .bank
                                .definition
                                .and_then(|definition| definition.open_first)
                            {
                                if cx.snapshot().locs().is_none() {
                                    return Poll::Pending;
                                }
                                if let Some((id, tile)) = self.open_first_target {
                                    let still_closed =
                                        catalog_object_at(cx, id, tile, open_first).is_some();
                                    if !still_closed {
                                        self.open_stage = 1;
                                        self.open_first_target = None;
                                        self.phase = Phase::Open;
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                    return Poll::Pending;
                }
                Phase::AwaitNoteMode {
                    request_id,
                    evidence,
                } => {
                    if cx.evidence().run != evidence.run
                        || cx.evidence().sequence <= evidence.sequence
                    {
                        if cx.active_now() >= self.deadline {
                            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                "bank item mode dispatch did not settle",
                            ))));
                        }
                        return Poll::Pending;
                    }
                    let Some(receipt) = cx.interaction_receipt(request_id) else {
                        if cx.active_now() >= self.deadline {
                            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                "bank item mode dispatch did not settle",
                            ))));
                        }
                        return Poll::Pending;
                    };
                    if !receipt.accepted
                        || receipt.evidence.run != evidence.run
                        || receipt.evidence.sequence < evidence.sequence
                    {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "bank item mode could not be set to unnoted",
                        ))));
                    }
                    if !cx.snapshot().bank_session().is_some_and(|session| {
                        session.value.open && session.value.generation == self.session
                    }) {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "bank closed before item mode settled",
                        ))));
                    }
                    self.item_mode_ensured = true;
                    cx.note_bank_item_mode(self.session);
                    self.phase = Phase::Act;
                    continue;
                }
                Phase::Act => match &self.request.action {
                    BankAction::Scan => return Poll::Ready(Ok(self.receipt(true))),
                    BankAction::OpenStand { .. } => {
                        return Poll::Ready(Ok(self.receipt(true)));
                    }
                    BankAction::Withdraw { item, qty } => {
                        let Some(view) = HeldView::read(cx, item.id) else {
                            return Poll::Pending;
                        };
                        let goal = WithdrawGoal::exact_at_least(
                            item.id,
                            Arc::clone(&item.name),
                            item.id,
                            *qty,
                            view.available,
                            view.held,
                            self.session,
                        );
                        match ops::withdraw_progress(
                            &goal,
                            view.held,
                            view.full,
                            self.same_session(cx),
                        ) {
                            Progress::Complete => {
                                return Poll::Ready(Ok(self.receipt(true)));
                            }
                            Progress::SessionGone => {
                                return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                    "bank closed during withdraw",
                                ))));
                            }
                            Progress::Incomplete | Progress::PackFull | Progress::OverTarget => {}
                        }
                        let reason =
                            match std::task::ready!(self.click_withdraw(cx, &goal, view.held))? {
                                Clicked::Sent => return Poll::Pending,
                                Clicked::Closed => {
                                    return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                        "bank closed during withdraw",
                                    ))));
                                }
                                Clicked::NoStock => "bank lacks requested item",
                                Clicked::NoOp => "bank item has no compatible withdraw action",
                            };
                        // `partial_ok` takes the incomplete receipt; an unmet
                        // exact target is never reported complete.
                        return if self.request.partial_ok {
                            Poll::Ready(Ok(self.receipt(false)))
                        } else {
                            Poll::Ready(Err(ActionError::Failed(Arc::from(reason))))
                        };
                    }
                    BankAction::WithdrawAny { items, qty } => {
                        let snapshot = cx.snapshot();
                        // A legal tier already carried — worn or held — is
                        // the loadout satisfied; never withdraw a second one
                        // (design-bank-snapshot §6 bug 15).
                        let stock = snapshot.stock();
                        if stock.pack.is_none() {
                            return Poll::Pending;
                        }
                        if items.iter().any(|item| {
                            stock.holds(item.id, *qty) == Truth::True
                                || stock.has(item.id, *qty) == Truth::True
                        }) {
                            return Poll::Ready(Ok(self.receipt(true)));
                        }
                        let Some(bank) = snapshot.bank() else {
                            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                "bank closed during tier withdraw",
                            ))));
                        };
                        let Some(item) = items.iter().find(|item| {
                            bank.value
                                .iter()
                                .any(|row| row.def.id == item.id && row.count > 0)
                        }) else {
                            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                "bank lacks every permitted loadout tier",
                            ))));
                        };
                        let Some(view) = HeldView::read(cx, item.id) else {
                            return Poll::Pending;
                        };
                        let goal = WithdrawGoal::exact_at_least(
                            item.id,
                            Arc::clone(&item.name),
                            item.id,
                            *qty,
                            view.available,
                            view.held,
                            self.session,
                        );
                        if ops::withdraw_progress(
                            &goal,
                            view.held,
                            view.full,
                            self.same_session(cx),
                        ) == Progress::SessionGone
                        {
                            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                "bank closed during tier withdraw",
                            ))));
                        }
                        match std::task::ready!(self.click_withdraw(cx, &goal, view.held))? {
                            Clicked::Sent => return Poll::Pending,
                            Clicked::Closed => {
                                return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                    "bank closed during tier withdraw",
                                ))));
                            }
                            Clicked::NoStock | Clicked::NoOp => {
                                return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                    "bank item has no compatible withdraw action",
                                ))));
                            }
                        }
                    }
                    BankAction::WithdrawTo { withdrawals } => {
                        let Some(withdrawal) = withdrawals.get(self.withdraw_index) else {
                            return Poll::Ready(Ok(self.receipt(true)));
                        };
                        let Some(view) = HeldView::read(cx, withdrawal.id) else {
                            return Poll::Pending;
                        };
                        let goal = WithdrawGoal::exact_equal(
                            withdrawal.id,
                            Arc::clone(&withdrawal.name),
                            withdrawal.id,
                            withdrawal.target,
                            view.available,
                            view.held,
                            self.session,
                        );
                        match ops::withdraw_progress(
                            &goal,
                            view.held,
                            view.full,
                            self.same_session(cx),
                        ) {
                            Progress::Complete => {
                                self.withdraw_index += 1;
                                continue;
                            }
                            Progress::OverTarget => {
                                return Poll::Ready(Ok(self.receipt(false)));
                            }
                            Progress::SessionGone => {
                                return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                    "bank closed during exact withdraw",
                                ))));
                            }
                            Progress::Incomplete | Progress::PackFull => {}
                        }
                        match std::task::ready!(self.click_withdraw(cx, &goal, view.held))? {
                            Clicked::Sent => return Poll::Pending,
                            Clicked::Closed => {
                                return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                    "bank closed during exact withdraw",
                                ))));
                            }
                            Clicked::NoStock | Clicked::NoOp => {
                                return Poll::Ready(Ok(self.receipt(false)));
                            }
                        }
                    }
                    BankAction::Deposit { .. }
                    | BankAction::DepositAll { .. }
                    | BankAction::DepositProducts { .. }
                    | BankAction::DepositCapacity { .. } => {
                        if matches!(
                            self.request.action,
                            BankAction::DepositProducts { .. } | BankAction::DepositCapacity { .. }
                        ) {
                            match self.protected_unchanged(cx) {
                                Some(true) => {}
                                Some(false) => {
                                    return Poll::Ready(Err(ActionError::Blocked(Arc::from(
                                        "protected inventory changed during bank deposit",
                                    ))));
                                }
                                None => return Poll::Pending,
                            }
                        }
                        let now = cx.active_now();
                        let snapshot = cx.snapshot();
                        let pack = snapshot.inventory().map(|rows| rows.value);
                        let side = snapshot.bank_side().map(|rows| rows.value);
                        if let BankAction::DepositCapacity { slots, .. } = &self.request.action {
                            if self.capacity_freed >= *slots {
                                return Poll::Ready(Ok(self.receipt(true)));
                            }
                        }
                        let wait_done = self.view_armed && now >= self.deadline;
                        let scan = if let BankAction::DepositCapacity { keep, .. } =
                            &self.request.action
                        {
                            ops::deposit_capacity_next(keep, side, pack, self.same_session(cx))
                        } else {
                            let spec = deposit_spec(&self.request.action)
                                .expect("deposit actions have a deposit spec");
                            ops::deposit_next(&spec, side, pack, wait_done, self.same_session(cx))
                        };
                        match scan {
                            DepositScan::WaitView => {
                                if pack.is_some()
                                    && matches!(self.request.action, BankAction::DepositAll { .. })
                                {
                                    if !self.view_armed {
                                        self.view_armed = true;
                                        self.deadline = now
                                            .saturating_add(Duration::from_millis(DEPOSIT_VIEW_MS));
                                    }
                                } else if now >= self.deadline {
                                    return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                        "bank deposit view did not post",
                                    ))));
                                }
                                return Poll::Pending;
                            }
                            DepositScan::Done => {
                                let complete = match &self.request.action {
                                    BankAction::DepositCapacity { slots, .. } => {
                                        self.capacity_freed >= *slots
                                    }
                                    _ => true,
                                };
                                return Poll::Ready(Ok(self.receipt(complete)));
                            }
                            DepositScan::SessionGone => {
                                return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                    "bank closed during deposit",
                                ))));
                            }
                            DepositScan::MissingRequired => {
                                return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                    "inventory products remain but matching bank side row is missing",
                                ))));
                            }
                            DepositScan::Click(click) => {
                                if self.deposits >= MAX_DEPOSITS {
                                    return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                        "bank deposit exceeded 32 item rows",
                                    ))));
                                }
                                let before = pack.map_or(0, |rows| ops::count_id(rows, click.id));
                                let before_slots = matches!(
                                    self.request.action,
                                    BankAction::DepositCapacity { .. }
                                )
                                .then(|| {
                                    pack.map_or(0, |rows| {
                                        rows.iter().filter(|row| row.count > 0).count()
                                    })
                                });
                                std::task::ready!(crate::native::defer_budget(
                                    cx.emit(ops::deposit_req(click, self.session))
                                ))?;
                                self.deposits += 1;
                                self.view_armed = false;
                                self.deadline = cx.active_now().saturating_add(TRANSFER_BOUND);
                                self.phase = Phase::AwaitTransfer {
                                    before,
                                    before_slots,
                                    item_id: click.id,
                                    evidence: cx.evidence(),
                                };
                                return Poll::Pending;
                            }
                        }
                    }
                    BankAction::Close => unreachable!("close is handled before opening"),
                },
                Phase::AwaitTransfer {
                    before,
                    before_slots,
                    item_id,
                    evidence,
                } => {
                    if cx.evidence().run != evidence.run
                        || cx.evidence().sequence <= evidence.sequence
                    {
                        return Poll::Pending;
                    }
                    if matches!(
                        self.request.action,
                        BankAction::DepositProducts { .. } | BankAction::DepositCapacity { .. }
                    ) {
                        match self.protected_unchanged(cx) {
                            Some(true) => {}
                            Some(false) => {
                                return Poll::Ready(Err(ActionError::Blocked(Arc::from(
                                    "protected inventory changed during bank deposit",
                                ))));
                            }
                            None => return Poll::Pending,
                        }
                    }
                    // A count change settles only in the bank session that
                    // sent the click; a closed or replaced session never
                    // completes it, whichever way the count moved.
                    if !self.same_session(cx) {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "bank closed before the transfer settled",
                        ))));
                    }
                    // The clicked id's held count: a withdraw raises it, a
                    // deposit lowers it.
                    let current = cx
                        .snapshot()
                        .inventory()
                        .map(|rows| ops::count_id(rows.value, item_id));
                    let withdrawing = matches!(
                        self.request.action,
                        BankAction::Withdraw { .. }
                            | BankAction::WithdrawAny { .. }
                            | BankAction::WithdrawTo { .. }
                    );
                    let moved = if withdrawing {
                        current.is_some_and(|now| now > before)
                    } else {
                        current.is_some_and(|now| now < before)
                    };
                    if moved {
                        if let BankAction::DepositProducts { .. } = &self.request.action {
                            let after = current.unwrap_or(before);
                            self.deposited = self
                                .deposited
                                .saturating_add(before.saturating_sub(after).max(0) as u32);
                        }
                        if let Some(before_slots) = before_slots {
                            if let Some(after_slots) = cx
                                .snapshot()
                                .inventory()
                                .map(|rows| rows.value.iter().filter(|row| row.count > 0).count())
                            {
                                let freed = before_slots.saturating_sub(after_slots);
                                self.capacity_freed = self
                                    .capacity_freed
                                    .saturating_add(u8::try_from(freed).unwrap_or(u8::MAX));
                            }
                        }
                        self.phase = Phase::Act;
                        continue;
                    }
                    if cx.active_now() >= self.deadline {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "bank transfer did not settle",
                        ))));
                    }
                    return Poll::Pending;
                }
                Phase::AwaitClose { baseline, run } => {
                    let session = cx.snapshot().bank_session().map(|s| s.value);
                    // Logged out: no session to read; the replaced login
                    // settles it whatever the other facts say.
                    let same_login = session.is_some() && cx.evidence().run == run;
                    match ops::close_progress(
                        session.is_some_and(|s| s.open),
                        session.map_or(-1, |s| s.side),
                        session.map_or(baseline.generation, |s| s.generation),
                        baseline,
                        same_login,
                        cx.active_now() >= self.deadline,
                    ) {
                        CloseScan::Complete | CloseScan::AlreadyShut => {
                            return Poll::Ready(Ok(self.receipt(true)));
                        }
                        CloseScan::Waiting | CloseScan::SideHeld => return Poll::Pending,
                        CloseScan::SessionReplaced => {
                            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                "bank session replaced before the close settled",
                            ))));
                        }
                        CloseScan::TimedOut => {
                            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                "bank did not close",
                            ))));
                        }
                    }
                }
            }
        }
    }

    fn cancel(&mut self) {}
}

struct NativeNpcContext<'a, 'frame> {
    cx: &'a mut ActionContext<'frame>,
    deadline: &'a mut Duration,
}

impl npc::Context for NativeNpcContext<'_, '_> {
    type Error = ActionError;

    fn bank_open(&self) -> bool {
        self.cx
            .snapshot()
            .bank_session()
            .is_some_and(|session| session.value.open)
    }

    fn bank_loaded(&self) -> bool {
        self.cx.snapshot().bank().is_some()
    }

    fn chat(&self) -> npc::Chat {
        let snapshot = self.cx.snapshot();
        let tick = self.cx.evidence().tick;
        match snapshot.chat_modal() {
            Some(chat) => npc::Chat {
                tick,
                modal: chat.value.root,
                can_continue: chat.value.continue_component_id >= 0,
            },
            None => npc::Chat {
                tick,
                modal: -1,
                can_continue: false,
            },
        }
    }

    fn banker(&self, name: &str, op: npc::NpcOp<'_>) -> Option<npc::Banker> {
        let npcs = self.cx.snapshot().npcs()?;
        let (banker, action) = npcs
            .value
            .iter()
            .filter(|banker| {
                banker
                    .name
                    .as_deref()
                    .is_some_and(|actual| npc::name_matches(actual, name))
            })
            .filter_map(|banker| {
                let action = banker
                    .actions
                    .iter()
                    .enumerate()
                    .find_map(|(index, action)| {
                        let action = action.as_deref()?;
                        npc::action_matches(action, index, op).then_some(action)
                    })?;
                Some((banker, action))
            })
            .min_by_key(|(banker, _)| banker.distance)?;
        Some(npc::Banker {
            name: banker.name.as_ref()?.clone(),
            action: action.to_owned(),
            index: i32::try_from(banker.index).ok()?,
        })
    }

    fn choice(&self, choose: &str) -> Option<i32> {
        self.cx
            .snapshot()
            .chat_options()?
            .value
            .iter()
            .position(|option| npc::option_matches(&option.text, choose))
            .and_then(|index| i32::try_from(index + 1).ok())
    }

    fn arm(&mut self, millis: u64) {
        *self.deadline = self
            .cx
            .active_now()
            .saturating_add(Duration::from_millis(millis));
    }

    fn expired(&mut self) -> bool {
        self.cx.active_now() >= *self.deadline
    }

    fn emit(&mut self, request: InteractReq) -> Result<(), Self::Error> {
        self.cx.emit(request).map(|_| ())
    }
}

fn npc_intent(access: &BankStandAccess) -> Result<npc::Intent<'_>, ActionError> {
    let declared = access.bank.definition.and_then(|definition| definition.npc);
    let name: &str = match declared {
        Some(npc) => npc.name,
        None => access.name.as_deref().unwrap_or(""),
    };
    if name.trim().is_empty() {
        return Err(ActionError::Unavailable(Arc::from(
            "bank teller has no declared or packed NPC name",
        )));
    }
    let op = match declared {
        Some(npc) if !npc.op.trim().is_empty() => npc::NpcOp::Name(npc.op),
        Some(_) => {
            return Err(ActionError::Unavailable(Arc::from(
                "bank teller has no declared NPC operation",
            )));
        }
        None if access.stand_op > 0 => npc::NpcOp::Index(access.stand_op),
        None => {
            return Err(ActionError::Unavailable(Arc::from(
                "bank teller has no packed NPC action index",
            )));
        }
    };
    let choose: &str = match access
        .bank
        .definition
        .and_then(|definition| definition.choose)
    {
        Some(choose) => choose,
        None => access.choose.as_deref().unwrap_or(""),
    };
    Ok(npc::Intent { name, op, choose })
}

/// The kernel deposit spec of a deposit action, borrowed from its request:
/// a one-row deposit must move that id, deposit-all sweeps everything not
/// kept, and products must leave the pack.
fn deposit_spec(action: &BankAction) -> Option<DepositSpec<'_>> {
    match action {
        BankAction::Deposit { item } => Some(DepositSpec {
            kind: DepositKind::Required,
            keep: &[],
            only: Some(std::slice::from_ref(&item.id)),
        }),
        BankAction::DepositAll { keep } => Some(DepositSpec {
            kind: DepositKind::UntilEmpty,
            keep,
            only: None,
        }),
        BankAction::DepositProducts { products, keep } => Some(DepositSpec {
            kind: DepositKind::Required,
            keep,
            only: Some(products),
        }),
        _ => None,
    }
}

/// The held count, pack fullness and bank stock of one id this frame.
struct HeldView {
    held: i32,
    full: bool,
    available: i32,
}

impl HeldView {
    /// `None` until the inventory has posted.
    fn read(cx: &ActionContext<'_>, id: i32) -> Option<Self> {
        let snapshot = cx.snapshot();
        let inventory = snapshot.inventory()?.value;
        let size = snapshot
            .inventory_capacity()
            .map_or(0, |capacity| i32::from(capacity.value));
        Some(Self {
            held: ops::count_id(inventory, id),
            full: ops::pack_full(inventory, size),
            available: snapshot
                .bank()
                .map_or(0, |bank| ops::count_id(bank.value, id)),
        })
    }
}

/// What one withdraw click attempt did.
enum Clicked {
    /// The click (or the Item-mode step before it) was sent.
    Sent,
    /// The bank item table is gone.
    Closed,
    /// No bank row of the item has stock.
    NoStock,
    /// The row has no compatible withdraw op.
    NoOp,
}

/// The bank session generation now, or `u64::MAX` before any session.
fn open_generation(cx: &ActionContext<'_>) -> u64 {
    cx.snapshot()
        .bank_session()
        .map_or(u64::MAX, |session| session.value.generation)
}

impl BankMachine {
    fn same_session(&self, cx: &ActionContext<'_>) -> bool {
        cx.snapshot()
            .bank_session()
            .is_some_and(|session| session.value.open && session.value.generation == self.session)
    }

    /// A session this machine's own open created starts in Item mode
    /// (`bank.rs2:18-19` resets `%bankcert`); later verbs read it from the
    /// ledger instead of pressing Item again (TICK-FIX #11, D3).
    fn note_fresh_session(&self, cx: &mut ActionContext<'_>) {
        if self.opened_from.is_some_and(|from| from != self.session) {
            cx.note_bank_item_mode(self.session);
        }
    }

    /// Send the kernel's next click toward `goal`, after selecting Item mode
    /// once in a session not already known to be in Item mode.
    fn click_withdraw(
        &mut self,
        cx: &mut ActionContext<'_>,
        goal: &WithdrawGoal,
        held: i32,
    ) -> Poll<Result<Clicked, ActionError>> {
        let request = {
            let snapshot = cx.snapshot();
            let Some(bank) = snapshot.bank() else {
                return Poll::Ready(Ok(Clicked::Closed));
            };
            let Some(row) = bank
                .value
                .iter()
                .find(|row| row.def.id == goal.bank_item_id && row.count > 0)
            else {
                return Poll::Ready(Ok(Clicked::NoStock));
            };
            let Some(request) = ops::withdraw_click(row, ops::withdraw_remaining(goal, held), goal)
            else {
                return Poll::Ready(Ok(Clicked::NoOp));
            };
            request
        };
        if !self.item_mode_ensured && cx.bank_item_mode(self.session) {
            self.item_mode_ensured = true;
        }
        if !self.item_mode_ensured {
            let evidence = cx.evidence();
            let request_id = std::task::ready!(crate::native::defer_budget(
                cx.emit(ops::note_req(NoteIntent::Item))
            ))?;
            self.deadline = cx.active_now().saturating_add(TRANSFER_BOUND);
            self.phase = Phase::AwaitNoteMode {
                request_id,
                evidence,
            };
            return Poll::Ready(Ok(Clicked::Sent));
        }
        std::task::ready!(crate::native::defer_budget(cx.emit(request)))?;
        self.deadline = cx.active_now().saturating_add(TRANSFER_BOUND);
        self.phase = Phase::AwaitTransfer {
            before: held,
            before_slots: None,
            item_id: goal.lands_as_id,
            evidence: cx.evidence(),
        };
        Poll::Ready(Ok(Clicked::Sent))
    }

    fn protected_unchanged(&self, cx: &ActionContext<'_>) -> Option<bool> {
        let snapshot = cx.snapshot();
        let inventory = snapshot.inventory()?;
        Some(
            self.protected_before
                .iter()
                .all(|before| ops::count_id(inventory.value, before.id) == before.count),
        )
    }

    fn receipt(&self, complete: bool) -> BankReceipt {
        BankReceipt { complete }
    }
}

pub struct Select {
    request: Option<BankPickRequest>,
    request_id: Option<u64>,
    immediate: Option<SelectedBank>,
}

impl NativeMachine for Select {
    type Args = SelectArgs;
    type Output = SelectedBank;

    fn begin(args: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        let snapshot = cx.snapshot();
        let stats = snapshot.stats().map_or(&[][..], |rows| rows.value);
        let quests = snapshot.quest_statuses().map_or(&[][..], |rows| rows.value);
        let selector = BankSelector::new(Arc::clone(&args.facts), args.preferences);
        let eligible = selector.eligible_indices(stats, quests);
        let explicit_bank = args
            .explicit
            .as_deref()
            .and_then(|name| selector.named(name, stats, quests));
        let explicit_unavailable = args.explicit.is_some() && explicit_bank.is_none();
        let eligible: Arc<[u16]> = Arc::from(eligible);
        if eligible.is_empty() || explicit_unavailable {
            return Ok(Self {
                request: None,
                request_id: None,
                immediate: Some(SelectedBank::no_candidate()),
            });
        }
        Ok(Self {
            request: Some(BankPickRequest {
                facts: args.facts,
                from: args.from,
                preferences: args.preferences,
                options: args.options,
                eligible,
                explicit_bank,
            }),
            request_id: None,
            immediate: None,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        if let Some(selected) = self.immediate.take() {
            return Poll::Ready(Ok(selected));
        }
        let Some(request_id) = self.request_id else {
            let Some(request) = self.request.take() else {
                return Poll::Ready(Err(ActionError::Stale));
            };
            return match cx.bank_pick(request) {
                Ok(request_id) => {
                    self.request_id = Some(request_id);
                    Poll::Pending
                }
                Err(error) => Poll::Ready(Err(error)),
            };
        };
        match cx.bank_pick_receipt(request_id) {
            Some(receipt) => Poll::Ready(Ok(receipt.selected.clone())),
            None => Poll::Pending,
        }
    }

    fn cancel(&mut self) {}
}

pub struct Open {
    core: BankMachine,
}

impl NativeMachine for Open {
    type Args = OpenArgs;
    type Output = ();

    fn begin(args: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        let bank = args.access.bank;
        let core = BankMachine::begin(
            BankRequest {
                bank: Some(bank),
                action: BankAction::OpenStand {
                    access: args.access,
                },
                partial_ok: false,
            },
            cx,
        )?;
        Ok(Self { core })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        match self.core.poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(_)) => Poll::Ready(Ok(())),
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
        }
    }

    fn cancel(&mut self) {
        self.core.cancel();
    }
}

pub struct Deposit {
    core: BankMachine,
}

impl NativeMachine for Deposit {
    type Args = DepositArgs;
    type Output = u32;

    fn begin(args: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        let core = BankMachine::begin(
            BankRequest {
                bank: None,
                action: BankAction::DepositProducts {
                    products: Arc::clone(&args.products),
                    keep: args.keep,
                },
                partial_ok: false,
            },
            cx,
        )?;
        Ok(Self { core })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        match self.core.poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(_)) => Poll::Ready(Ok(self.core.deposited)),
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
        }
    }

    fn cancel(&mut self) {
        self.core.cancel();
    }
}

pub struct Withdraw {
    core: BankMachine,
}

impl NativeMachine for Withdraw {
    type Args = WithdrawArgs;
    type Output = bool;

    fn begin(args: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        let core = BankMachine::begin(
            BankRequest {
                bank: None,
                action: BankAction::WithdrawTo {
                    withdrawals: args.withdrawals,
                },
                partial_ok: false,
            },
            cx,
        )?;
        Ok(Self { core })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        match self.core.poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(receipt)) => Poll::Ready(Ok(receipt.complete)),
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
        }
    }

    fn cancel(&mut self) {
        self.core.cancel();
    }
}

pub struct Close {
    core: BankMachine,
}

impl NativeMachine for Close {
    type Args = ();
    type Output = ();

    fn begin(_: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        let core = BankMachine::begin(
            BankRequest {
                bank: None,
                action: BankAction::Close,
                partial_ok: false,
            },
            cx,
        )?;
        Ok(Self { core })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        match self.core.poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(_)) => Poll::Ready(Ok(())),
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
        }
    }

    fn cancel(&mut self) {
        self.core.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::{ActionContext, HostEffect, InteractionReceipt};
    use crate::quester::families::tests::{
        local_player, with_tick, with_tick_reach, with_tick_snapshots,
    };
    use api::snapshot::{GameSnapshot, ItemActionFamily, ItemContainer, ItemView, SnapshotView};

    fn catalog_bank(name: &str) -> NamedBank {
        let definition = api::named_banks::BANK_CATALOG
            .iter()
            .find(|definition| definition.name == name)
            .unwrap();
        NamedBank {
            name: definition.name,
            tile: definition.tile,
            definition: Some(definition),
            routable: true,
        }
    }

    fn fishing(base: i32) -> [StatView; 1] {
        [StatView {
            index: 10,
            name: "Fishing".to_owned(),
            effective: base,
            base,
            xp: 0,
            used: true,
        }]
    }

    fn item(id: i32, name: &str, count: i32, container: ItemContainer) -> ItemView {
        ItemView {
            def: api::obj_names::ItemDefView {
                id,
                name: Some(name.to_owned()),
                stackable: false,
                members: false,
                base_value: 0,
                noted: false,
                certificate_link: -1,
                certificate_template: -1,
            },
            container,
            action_family: if container == ItemContainer::Bank {
                ItemActionFamily::Component
            } else {
                ItemActionFamily::Held
            },
            slot: 0,
            count,
            actions: if container == ItemContainer::Bank {
                vec![Some("Withdraw-X".to_owned()), Some("Withdraw-1".to_owned())]
            } else {
                Vec::new()
            },
            component_id: 7,
        }
    }

    fn acknowledge(
        ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
        tick: u64,
    ) -> HostEffect {
        let ledger = ledger.as_mut().unwrap();
        let action = ledger.outbox.remove(0);
        ledger.complete_interaction(
            &action.authority(),
            InteractionReceipt {
                request_id: action.request_id.get(),
                evidence: EvidenceStamp {
                    run: action.run(),
                    tick,
                    sequence: tick,
                },
                accepted: true,
                chat_since: 0,
            },
        );
        action.effect
    }
    fn spend_interaction(cx: &mut ActionContext<'_>) {
        cx.action_id = cx.ledger.as_ref().unwrap().owner.as_ref().unwrap().id.get();
        let request_id = cx
            .emit(InteractReq::ContinueDialog { component_id: None })
            .unwrap();
        let evidence = cx.evidence();
        let ledger = cx.ledger.as_mut().unwrap();
        let action = ledger.outbox.pop().expect("budget-filling interaction");
        assert_eq!(action.request_id.get(), request_id);
        ledger.complete_interaction(
            &action.authority(),
            InteractionReceipt {
                request_id,
                evidence,
                accepted: true,
                chat_since: 0,
            },
        );
    }

    fn spend_batch_event(cx: &mut ActionContext<'_>) {
        cx.action_id = cx.ledger.as_ref().unwrap().owner.as_ref().unwrap().id.get();
        let request_id = cx
            .emit_batch([
                Some(InteractReq::SideTab { tab: 0 }),
                None,
                None,
                None,
                None,
            ])
            .unwrap();
        let evidence = cx.evidence();
        let ledger = cx.ledger.as_mut().unwrap();
        let action = ledger.outbox.pop().expect("budget-filling batch");
        assert_eq!(action.request_id.get(), request_id);
        ledger.complete_interaction(
            &action.authority(),
            InteractionReceipt {
                request_id,
                evidence,
                accepted: true,
                chat_since: 0,
            },
        );
    }

    fn acknowledge_current(cx: &mut ActionContext<'_>) -> HostEffect {
        let evidence = cx.evidence();
        let ledger = cx.ledger.as_mut().unwrap();
        let action = ledger.outbox.remove(0);
        ledger.complete_interaction(
            &action.authority(),
            InteractionReceipt {
                request_id: action.request_id.get(),
                evidence,
                accepted: true,
                chat_since: 0,
            },
        );
        action.effect
    }

    fn start_bank(
        snapshot: &GameSnapshot,
        ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
        request: BankRequest,
    ) -> crate::native::ActionHandle<BankMachine> {
        with_tick(snapshot, ledger, 1, |tick| {
            tick.actions
                .begin::<BankMachine>(request, &mut tick.cx)
                .unwrap()
        })
    }

    fn assert_bank_emit_deferred(
        snapshot: &GameSnapshot,
        request: BankRequest,
        assert_emitted: impl FnOnce(&HostEffect),
    ) {
        let mut ledger = None;
        let handle = start_bank(snapshot, &mut ledger, request);
        let denied = with_tick(snapshot, &mut ledger, 2, |tick| {
            spend_interaction(&mut tick.cx);
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(denied.is_pending(), "budget denial must remain Pending");
        assert!(ledger.as_ref().unwrap().outbox.is_empty());

        let retry = with_tick(snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(retry.is_pending(), "admitted interaction remains in flight");
        let ledger = ledger.as_ref().unwrap();
        assert_eq!(ledger.outbox.len(), 1);
        assert_emitted(&ledger.outbox[0].effect);
    }

    fn teller_access(
        bank: NamedBank,
        name: Option<&str>,
        stand_op: i32,
        choose: Option<&str>,
    ) -> Arc<BankStandAccess> {
        Arc::new(BankStandAccess {
            bank,
            stand_tile: bank.tile,
            kind: AccessKind::Teller,
            stand_op,
            name: name.map(Arc::from),
            choose: choose.map(Arc::from),
        })
    }

    fn open_request(access: Arc<BankStandAccess>) -> BankRequest {
        BankRequest {
            bank: Some(access.bank),
            action: BankAction::OpenStand { access },
            partial_ok: false,
        }
    }
    fn bank_object_loc(
        id: i32,
        name: &str,
        action: &str,
        tile: WorldTile,
        distance: i32,
    ) -> api::snapshot::LocView {
        api::snapshot::LocView {
            typecode: id,
            info: 0,
            id,
            name: Some(name.to_owned()),
            description: None,
            actions: vec![Some(action.to_owned())],
            tile,
            distance,
            layer: api::snapshot::LocLayer::GroundDecoration,
            shape: 0,
            angle: 0,
            width: 1,
            length: 1,
            footprint_width: 1,
            footprint_length: 1,
            block_walk: false,
            block_range: false,
            active: true,
            animation: -1,
            map_function: -1,
            map_scene: -1,
            force_approach: 0,
        }
    }

    fn object_reach_view(stand: WorldTile, loc: WorldTile) -> api::query::ReachQueryView {
        let base_x = stand.x - 1;
        let base_z = stand.z - 1;
        let width = 3;
        let height = 3;
        let index = |tile: WorldTile| ((tile.x - base_x) * height + (tile.z - base_z)) as usize;
        let tile_count = (width * height) as usize;
        let mut reachable = vec![0; 1];
        let mut reachable_adj = vec![0; 1];
        let mut exact_rank = vec![u16::MAX; tile_count];
        let mut adjacent_rank = vec![u16::MAX; tile_count];
        let stand_index = index(stand);
        let loc_index = index(loc);
        reachable[stand_index / 32] |= 1 << (stand_index % 32);
        reachable_adj[stand_index / 32] |= 1 << (stand_index % 32);
        reachable_adj[loc_index / 32] |= 1 << (loc_index % 32);
        exact_rank[stand_index] = 0;
        adjacent_rank[stand_index] = 0;
        adjacent_rank[loc_index] = 1;
        api::query::ReachQueryView {
            available: true,
            base_x,
            base_z,
            level: stand.level,
            width,
            height,
            walkable: vec![0; 1],
            reachable,
            reachable_adj,
            exact_rank,
            adjacent_rank,
            step: vec![0; tile_count],
            canlight: Vec::new(),
        }
    }

    #[test]
    fn catalog_object_access_opens_live_loc_from_adjacent_bank_stand() {
        let bank = catalog_bank("Shantay Pass");
        let loc_tile = WorldTile {
            x: bank.tile.x + 1,
            z: bank.tile.z,
            level: bank.tile.level,
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        // The shared chooser trusts a flood only from the live player tile.
        snapshot.seed_local_player(local_player(bank.tile));
        snapshot.seed_locs(vec![
            bank_object_loc(2694, "Shantay chest", "Bank", bank.tile, 0),
            bank_object_loc(2693, "Shantay chest", "Open", loc_tile, 1),
        ]);
        let reach = object_reach_view(bank.tile, loc_tile);
        let access = Arc::new(BankStandAccess {
            bank,
            stand_tile: bank.tile,
            kind: AccessKind::Booth,
            stand_op: 0,
            name: Some(Arc::from("Shantay chest")),
            choose: None,
        });
        let mut ledger = None;
        with_tick_reach(&snapshot, &reach, &mut ledger, 1, |tick| {
            let handle = tick
                .actions
                .begin::<BankMachine>(open_request(access), &mut tick.cx)
                .unwrap();
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        assert!(matches!(
            ledger
                .as_ref()
                .and_then(|ledger| ledger.outbox.first())
                .map(|action| &action.effect),
            Some(HostEffect::Interaction(InteractReq::Loc {
                x,
                z,
                level,
                action,
                id: Some(2693),
            })) if *x == loc_tile.x && *z == loc_tile.z && *level == loc_tile.level && action.as_str() == "Open"
        ));
    }
    #[test]
    fn catalog_object_dialogue_continues_and_settles_loaded_bank() {
        let bank = catalog_bank("Shantay Pass");
        let loc_tile = WorldTile {
            x: bank.tile.x + 1,
            z: bank.tile.z,
            level: bank.tile.level,
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(bank.tile));
        snapshot.seed_locs(vec![bank_object_loc(
            2693,
            "Shantay chest",
            "Open",
            loc_tile,
            1,
        )]);
        let reach = object_reach_view(bank.tile, loc_tile);
        let access = Arc::new(BankStandAccess {
            bank,
            stand_tile: bank.tile,
            kind: AccessKind::Booth,
            stand_op: 0,
            name: Some(Arc::from("Shantay chest")),
            choose: None,
        });
        let mut ledger = None;
        let handle = with_tick_reach(&snapshot, &reach, &mut ledger, 1, |tick| {
            let handle = tick
                .actions
                .begin::<BankMachine>(open_request(access), &mut tick.cx)
                .unwrap();
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            handle
        });
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::Loc {
                action,
                id: Some(2693),
                ..
            }) if action == "Open"
        ));

        snapshot.seed_chat_modal(1, vec!["You open the bank.".into()]);
        snapshot.seed_chat_options(Vec::new(), 99);
        with_tick_reach(&snapshot, &reach, &mut ledger, 2, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        assert!(matches!(
            ledger
                .as_ref()
                .and_then(|ledger| ledger.outbox.first())
                .map(|action| &action.effect),
            Some(HostEffect::Interaction(InteractReq::ContinueDialog {
                component_id: None
            }))
        ));
        assert!(matches!(
            acknowledge(&mut ledger, 2),
            HostEffect::Interaction(InteractReq::ContinueDialog { component_id: None })
        ));

        snapshot.seed_chat_modal(-1, Vec::new());
        snapshot.seed_chat_options(Vec::new(), -1);
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), Vec::new());
        with_tick_reach(&snapshot, &reach, &mut ledger, 3, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        let receipt = with_tick_reach(&snapshot, &reach, &mut ledger, 4, |tick| {
            match tick.actions.poll(&handle, &mut tick.cx) {
                Poll::Ready(Ok(receipt)) => receipt,
                other => panic!("loaded object bank should settle after dialogue: {other:?}"),
            }
        });
        assert!(receipt.complete);
    }

    #[test]
    fn catalog_open_first_bank_resolves_live_loc_stages_near_stand() {
        let bank = catalog_bank("Duel Arena");
        let loc_tile = WorldTile {
            x: bank.tile.x - 1,
            z: bank.tile.z,
            level: bank.tile.level,
        };
        let other_loc_tile = WorldTile {
            x: bank.tile.x,
            z: bank.tile.z + 1,
            level: bank.tile.level,
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(bank.tile));
        snapshot.seed_locs(vec![
            bank_object_loc(4001, "Closed chest", "Open", loc_tile, 1),
            bank_object_loc(4001, "Closed chest", "Open", other_loc_tile, 1),
        ]);
        let reach = object_reach_view(bank.tile, loc_tile);
        let access = Arc::new(BankStandAccess {
            bank,
            stand_tile: bank.tile,
            kind: AccessKind::Booth,
            stand_op: 0,
            name: Some(Arc::from("Open chest")),
            choose: None,
        });
        let mut ledger = None;
        let handle = with_tick_reach(&snapshot, &reach, &mut ledger, 1, |tick| {
            let handle = tick
                .actions
                .begin::<BankMachine>(open_request(access), &mut tick.cx)
                .unwrap();
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            handle
        });
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::Loc {
                action,
                id: Some(4001),
                ..
            }) if action == "Open"
        ));
        with_tick_reach(&snapshot, &reach, &mut ledger, 2, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        snapshot.seed_locs(vec![
            bank_object_loc(4002, "Open chest", "Bank", loc_tile, 1),
            bank_object_loc(4001, "Closed chest", "Open", other_loc_tile, 1),
        ]);
        with_tick_reach(&snapshot, &reach, &mut ledger, 3, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        assert!(matches!(
            ledger
                .as_ref()
                .and_then(|ledger| ledger.outbox.first())
                .map(|action| &action.effect),
            Some(HostEffect::Interaction(InteractReq::Loc {
                action,
                id: Some(4002),
                ..
            })) if action.as_str() == "Bank"
        ));
    }

    #[test]
    fn catalog_object_stuck_continue_modal_fails_at_open_deadline() {
        let bank = catalog_bank("Shantay Pass");
        let loc_tile = WorldTile {
            x: bank.tile.x + 1,
            z: bank.tile.z,
            level: bank.tile.level,
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(bank.tile));
        snapshot.seed_locs(vec![bank_object_loc(
            2693,
            "Shantay chest",
            "Open",
            loc_tile,
            1,
        )]);
        let reach = object_reach_view(bank.tile, loc_tile);
        let access = Arc::new(BankStandAccess {
            bank,
            stand_tile: bank.tile,
            kind: AccessKind::Booth,
            stand_op: 0,
            name: Some(Arc::from("Shantay chest")),
            choose: None,
        });
        let mut ledger = None;
        let handle = with_tick_reach(&snapshot, &reach, &mut ledger, 1, |tick| {
            let handle = tick
                .actions
                .begin::<BankMachine>(open_request(access), &mut tick.cx)
                .unwrap();
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            handle
        });
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::Loc {
                action,
                id: Some(2693),
                ..
            }) if action == "Open"
        ));

        snapshot.seed_chat_modal(1, vec!["The bank remains open.".into()]);
        snapshot.seed_chat_options(Vec::new(), 99);
        with_tick_reach(&snapshot, &reach, &mut ledger, 2, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        assert!(matches!(
            acknowledge(&mut ledger, 2),
            HostEffect::Interaction(InteractReq::ContinueDialog { component_id: None })
        ));

        for tick_number in [7, 12, 17] {
            with_tick_reach(&snapshot, &reach, &mut ledger, tick_number, |tick| {
                assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            });
            assert!(matches!(
                acknowledge(&mut ledger, tick_number),
                HostEffect::Interaction(InteractReq::ContinueDialog { component_id: None })
            ));
        }

        let result = with_tick_reach(&snapshot, &reach, &mut ledger, 21, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(matches!(
            result,
            Poll::Ready(Err(ActionError::Failed(message)))
                if message.as_ref() == "bank item table unavailable"
        ));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }

    #[test]
    fn catalog_object_finished_dialogue_does_not_restart_for_stuck_modal() {
        let bank = catalog_bank("Shantay Pass");
        let loc_tile = WorldTile {
            x: bank.tile.x + 1,
            z: bank.tile.z,
            level: bank.tile.level,
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(bank.tile));
        snapshot.seed_locs(vec![bank_object_loc(
            2693,
            "Shantay chest",
            "Open",
            loc_tile,
            1,
        )]);
        let reach = object_reach_view(bank.tile, loc_tile);
        let access = Arc::new(BankStandAccess {
            bank,
            stand_tile: bank.tile,
            kind: AccessKind::Booth,
            stand_op: 0,
            name: Some(Arc::from("Shantay chest")),
            choose: None,
        });
        let mut ledger = None;
        let handle = with_tick_reach(&snapshot, &reach, &mut ledger, 1, |tick| {
            let handle = tick
                .actions
                .begin::<BankMachine>(open_request(access), &mut tick.cx)
                .unwrap();
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            handle
        });
        let _ = acknowledge(&mut ledger, 1);

        snapshot.seed_chat_modal(1, vec!["The bank remains open.".into()]);
        snapshot.seed_chat_options(Vec::new(), -1);
        for tick_number in [2, 7, 12, 17] {
            with_tick_reach(&snapshot, &reach, &mut ledger, tick_number, |tick| {
                assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            });
            assert!(ledger.as_ref().unwrap().outbox.is_empty());
        }

        let result = with_tick_reach(&snapshot, &reach, &mut ledger, 21, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(matches!(
            result,
            Poll::Ready(Err(ActionError::Failed(message)))
                if message.as_ref() == "bank item table unavailable"
        ));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }

    #[test]
    fn catalog_open_first_missing_locs_respects_open_deadline() {
        let bank = catalog_bank("Duel Arena");
        let loc_tile = WorldTile {
            x: bank.tile.x - 1,
            z: bank.tile.z,
            level: bank.tile.level,
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(bank.tile));
        snapshot.seed_locs(vec![bank_object_loc(
            4001,
            "Closed chest",
            "Open",
            loc_tile,
            1,
        )]);
        let reach = object_reach_view(bank.tile, loc_tile);
        let access = Arc::new(BankStandAccess {
            bank,
            stand_tile: bank.tile,
            kind: AccessKind::Booth,
            stand_op: 0,
            name: Some(Arc::from("Open chest")),
            choose: None,
        });
        let mut ledger = None;
        let handle = with_tick_reach(&snapshot, &reach, &mut ledger, 1, |tick| {
            let handle = tick
                .actions
                .begin::<BankMachine>(open_request(access), &mut tick.cx)
                .unwrap();
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            handle
        });
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::Loc {
                action,
                id: Some(4001),
                ..
            }) if action == "Open"
        ));

        let mut missing_locs = GameSnapshot::new();
        missing_locs.seed_ingame(1);
        let result = with_tick(&missing_locs, &mut ledger, 21, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(matches!(
            result,
            Poll::Ready(Err(ActionError::Failed(message)))
                if message.as_ref() == "bank item table unavailable"
        ));
    }

    fn npc_row(index: usize, name: &str, actions: &[&str]) -> api::snapshot::NpcView {
        let tile = WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        };
        api::snapshot::NpcView {
            index,
            r#type: None,
            name: Some(name.to_owned()),
            actions: actions
                .iter()
                .map(|action| Some((*action).to_owned()))
                .collect(),
            tile,
            distance: 2,
            animation: -1,
            animation_frame: 0,
            pose_animation: -1,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            spot_animation_stamp: -1,
            health: 0,
            total_health: 0,
            face_entity: -1,
            target: None,
            moving: false,
            running: false,
            in_combat: false,
            level: 0,
            size: 1,
            network: tile,
            x: tile.x,
            z: tile.z,
            yaw: 0,
        }
    }

    #[test]
    fn teller_access_drives_multiple_dialogue_pages_from_declared_metadata() {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_npcs(vec![npc_row(7, "Gundai", &["Talk-to"])]);
        let access = teller_access(
            catalog_bank("Mage Arena"),
            Some("wrong packed name"),
            0,
            None,
        );
        let mut ledger = None;
        let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
            let handle = tick
                .actions
                .begin::<BankMachine>(open_request(Arc::clone(&access)), &mut tick.cx)
                .unwrap();
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            handle
        });
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::Npc {
                ref name,
                ref action,
                index: Some(7),
            }) if name == "Gundai" && action == "Talk-to"
        ));

        snapshot.seed_chat_modal(1, vec!["First page".into()]);
        snapshot.seed_chat_options(Vec::new(), 99);
        with_tick(&snapshot, &mut ledger, 2, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        assert!(matches!(
            acknowledge(&mut ledger, 2),
            HostEffect::Interaction(InteractReq::ContinueDialog { component_id: None })
        ));

        snapshot.seed_chat_modal(2, vec!["Second page".into()]);
        with_tick(&snapshot, &mut ledger, 3, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
        with_tick(&snapshot, &mut ledger, 4, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        assert!(matches!(
            acknowledge(&mut ledger, 4),
            HostEffect::Interaction(InteractReq::ContinueDialog { component_id: None })
        ));

        snapshot.seed_chat_modal(3, vec!["Final page".into()]);
        snapshot.seed_chat_options(
            vec![
                api::snapshot::ChatOptionView {
                    component_id: 1,
                    text: "Not now".into(),
                },
                api::snapshot::ChatOptionView {
                    component_id: 2,
                    text: "I'd like to access my bank account".into(),
                },
            ],
            -1,
        );
        with_tick(&snapshot, &mut ledger, 5, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        with_tick(&snapshot, &mut ledger, 6, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        assert!(matches!(
            acknowledge(&mut ledger, 6),
            HostEffect::Interaction(InteractReq::Answer { option: 2 })
        ));

        snapshot.seed_chat_modal(-1, Vec::new());
        snapshot.seed_chat_options(Vec::new(), -1);
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), Vec::new());
        with_tick(&snapshot, &mut ledger, 7, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        with_tick(&snapshot, &mut ledger, 8, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        let receipt = with_tick(&snapshot, &mut ledger, 9, |tick| {
            match tick.actions.poll(&handle, &mut tick.cx) {
                Poll::Ready(Ok(receipt)) => receipt,
                other => {
                    panic!("loaded teller bank should open after all dialogue pages: {other:?}")
                }
            }
        });
        assert!(receipt.complete);
    }

    #[test]
    fn packed_teller_uses_the_declared_action_index_fallback() {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_npcs(vec![npc_row(12, "Packed Teller", &["Talk-to", "Bank"])]);
        let bank = NamedBank::new(
            "Packed teller bank",
            WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
        );
        let access = teller_access(bank, Some("Packed Teller"), 2, None);
        let mut ledger = None;
        let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
            let handle = tick
                .actions
                .begin::<BankMachine>(open_request(access), &mut tick.cx)
                .unwrap();
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            handle
        });
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::Npc {
                ref name,
                ref action,
                index: Some(12),
            }) if name == "Packed Teller" && action == "Bank"
        ));
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), Vec::new());
        with_tick(&snapshot, &mut ledger, 2, |tick| {
            assert!(matches!(
                tick.actions.poll(&handle, &mut tick.cx),
                Poll::Ready(Ok(receipt)) if receipt.complete
            ));
        });
    }

    #[test]
    fn native_teller_access_fails_when_unopened() {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_npcs(Vec::new());
        let access = teller_access(catalog_bank("Mage Arena"), Some("Gundai"), 1, None);
        let mut ledger = None;
        let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
            let handle = tick
                .actions
                .begin::<BankMachine>(open_request(access), &mut tick.cx)
                .unwrap();
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            handle
        });
        for tick_number in 2..=4 {
            let result = with_tick(&snapshot, &mut ledger, tick_number, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            });
            if tick_number < 4 {
                assert!(result.is_pending());
            } else {
                assert!(matches!(
                    result,
                    Poll::Ready(Err(ActionError::Unavailable(_)))
                ));
            }
        }
    }

    #[test]
    fn native_teller_access_rejects_an_open_bank_without_loaded_stock() {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_bank_observation(10, 1, None, Vec::new());
        let access = teller_access(catalog_bank("Mage Arena"), Some("Gundai"), 1, None);
        let mut ledger = None;
        let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
            let handle = tick
                .actions
                .begin::<BankMachine>(open_request(access), &mut tick.cx)
                .unwrap();
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            handle
        });
        let result = with_tick(&snapshot, &mut ledger, 8, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(matches!(
            result,
            Poll::Ready(Err(ActionError::Unavailable(_)))
        ));
    }

    #[test]
    fn deposit_products_fails_when_inventory_products_are_missing_from_loaded_side() {
        let item_id = 314;
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(vec![item(item_id, "Ore", 1, ItemContainer::Inventory)], 28);
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), Vec::new());
        let mut ledger = None;

        let result = with_tick(&snapshot, &mut ledger, 1, |tick| {
            let handle = tick
                .actions
                .begin::<BankMachine>(
                    BankRequest {
                        bank: None,
                        action: BankAction::DepositProducts {
                            products: Arc::from([item_id]),
                            keep: Arc::from([]),
                        },
                        partial_ok: false,
                    },
                    &mut tick.cx,
                )
                .unwrap();
            tick.actions.poll(&handle, &mut tick.cx)
        });

        assert!(matches!(
            result,
            Poll::Ready(Err(ActionError::Failed(message)))
                if message.as_ref()
                    == "inventory products remain but matching bank side row is missing"
        ));
    }

    #[test]
    fn deposit_products_succeeds_when_inventory_products_are_already_zero() {
        let item_id = 314;
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(Vec::new(), 28);
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), Vec::new());
        let mut ledger = None;

        let result = with_tick(&snapshot, &mut ledger, 1, |tick| {
            let handle = tick
                .actions
                .begin::<BankMachine>(
                    BankRequest {
                        bank: None,
                        action: BankAction::DepositProducts {
                            products: Arc::from([item_id]),
                            keep: Arc::from([]),
                        },
                        partial_ok: false,
                    },
                    &mut tick.cx,
                )
                .unwrap();
            tick.actions.poll(&handle, &mut tick.cx)
        });

        assert!(matches!(
            result,
            Poll::Ready(Ok(BankReceipt { complete: true, .. }))
        ));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }

    /// A session this machine did not open (here: adopted already open) has an
    /// unknown `%bankcert`, so the first withdraw still presses Item. A
    /// session a native machine opened skips it (TICK-FIX #11, below).
    #[test]
    fn exact_withdraw_in_an_adopted_session_selects_item_mode_before_id_fenced_transfer() {
        let item_id = 314;
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(Vec::new(), 28);
        snapshot.seed_bank_observation(
            10,
            1,
            Some(vec![item(item_id, "Bait", 5, ItemContainer::Bank)]),
            Vec::new(),
        );
        let mut ledger = None;
        let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
            let handle = tick
                .actions
                .begin::<BankMachine>(
                    BankRequest {
                        bank: None,
                        action: BankAction::WithdrawTo {
                            withdrawals: Arc::from([Withdrawal {
                                id: item_id,
                                name: Arc::from("Bait"),
                                target: 2,
                            }]),
                        },
                        partial_ok: false,
                    },
                    &mut tick.cx,
                )
                .unwrap();
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            handle
        });
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::SetNoteMode { on: false })
        ));
        with_tick(&snapshot, &mut ledger, 1, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        assert!(
            ledger.as_ref().unwrap().outbox.is_empty(),
            "same-frame dispatch receipt must wait for a fresh bank observation"
        );

        with_tick(&snapshot, &mut ledger, 2, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending())
        });
        assert!(matches!(
            acknowledge(&mut ledger, 2),
            HostEffect::Interaction(InteractReq::WithdrawX {
                count: 2,
                bank_item_id,
                lands_as_id,
                ref action,
                ..
            }) if bank_item_id == item_id && lands_as_id == item_id && action == "Withdraw-X"
        ));

        snapshot.seed_inventory(vec![item(item_id, "Bait", 2, ItemContainer::Inventory)], 28);
        snapshot.seed_bank_observation(
            10,
            2,
            Some(vec![item(item_id, "Bait", 3, ItemContainer::Bank)]),
            Vec::new(),
        );
        let receipt = with_tick(&snapshot, &mut ledger, 3, |tick| {
            match tick.actions.poll(&handle, &mut tick.cx) {
                Poll::Ready(Ok(receipt)) => receipt,
                other => panic!("exact withdrawal should settle from its item delta: {other:?}"),
            }
        });
        assert!(receipt.complete);
    }

    /// TICK-FIX #11 (D3): the open label resets `%bankcert` to Item
    /// (`content/scripts/interface_bank/scripts/bank.rs2:18-19`), so in a
    /// session this account's kernel opened, a later withdraw verb clicks the
    /// item at once instead of pressing Item a tick first.
    #[test]
    fn exact_withdraw_in_a_session_the_kernel_opened_skips_the_item_press() {
        let item_id = 314;
        let stand = WorldTile {
            x: 3208,
            z: 3220,
            level: 2,
        };
        let access = Arc::new(BankStandAccess {
            bank: NamedBank::new("Test bank", stand),
            stand_tile: stand,
            kind: AccessKind::Booth,
            stand_op: 1,
            name: None,
            choose: None,
        });
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(stand));
        snapshot.seed_inventory(Vec::new(), 28);
        snapshot.seed_locs(vec![bank_object_loc(
            2213,
            "Bank booth",
            "Use-quickly",
            stand,
            0,
        )]);
        let mut ledger = None;
        let open = with_tick(&snapshot, &mut ledger, 1, |tick| {
            let handle = tick
                .actions
                .begin::<BankMachine>(open_request(access), &mut tick.cx)
                .unwrap();
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            handle
        });
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::OpenStand { .. })
        ));
        snapshot.seed_bank_observation(
            10,
            2,
            Some(vec![item(item_id, "Bait", 5, ItemContainer::Bank)]),
            Vec::new(),
        );
        with_tick(&snapshot, &mut ledger, 2, |tick| {
            assert!(matches!(
                tick.actions.poll(&open, &mut tick.cx),
                Poll::Ready(Ok(_))
            ));
            let withdraw = tick
                .actions
                .begin::<BankMachine>(
                    BankRequest {
                        bank: None,
                        action: BankAction::WithdrawTo {
                            withdrawals: Arc::from([Withdrawal {
                                id: item_id,
                                name: Arc::from("Bait"),
                                target: 2,
                            }]),
                        },
                        partial_ok: false,
                    },
                    &mut tick.cx,
                )
                .unwrap();
            assert!(tick.actions.poll(&withdraw, &mut tick.cx).is_pending());
        });
        assert!(
            matches!(
                acknowledge(&mut ledger, 2),
                HostEffect::Interaction(InteractReq::WithdrawX {
                    count: 2,
                    bank_item_id,
                    ..
                }) if bank_item_id == item_id
            ),
            "the first click in a session the kernel opened is the withdraw itself"
        );
    }

    fn row_with(item: ItemView, actions: &[&str], slot: i32) -> ItemView {
        ItemView {
            actions: actions.iter().map(|a| Some((*a).to_owned())).collect(),
            slot,
            ..item
        }
    }

    fn bank_request(action: BankAction, partial_ok: bool) -> BankRequest {
        BankRequest {
            bank: None,
            action,
            partial_ok,
        }
    }

    fn begin_bank(
        snapshot: &GameSnapshot,
        ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
        request: BankRequest,
    ) -> (
        crate::native::ActionHandle<BankMachine>,
        Poll<Result<BankReceipt, ActionError>>,
    ) {
        with_tick(snapshot, ledger, 1, |tick| {
            let handle = tick
                .actions
                .begin::<BankMachine>(request, &mut tick.cx)
                .unwrap();
            let poll = tick.actions.poll(&handle, &mut tick.cx);
            (handle, poll)
        })
    }

    /// P-ladder (native): a fixed-only row serves an exact 7 as 5 + 1 + 1;
    /// one landed click is never the request.
    #[test]
    fn exact_withdraw_clicks_the_fixed_ladder_until_the_final_count() {
        let id = 995;
        let bank = |count| {
            Some(vec![row_with(
                item(id, "Coins", count, ItemContainer::Bank),
                &["Withdraw-1", "Withdraw-5", "Withdraw-10"],
                0,
            )])
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(Vec::new(), 28);
        snapshot.seed_bank_observation(10, 1, bank(50), Vec::new());
        let mut ledger = None;
        let request = bank_request(
            BankAction::WithdrawTo {
                withdrawals: Arc::from([Withdrawal {
                    id,
                    name: Arc::from("Coins"),
                    target: 7,
                }]),
            },
            false,
        );
        let (handle, poll) = begin_bank(&snapshot, &mut ledger, request);
        assert!(poll.is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::SetNoteMode { on: false })
        ));
        let mut clicks = Vec::new();
        let mut held = 0;
        for tick_number in 2..=4 {
            with_tick(&snapshot, &mut ledger, tick_number, |tick| {
                assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            });
            match acknowledge(&mut ledger, tick_number) {
                HostEffect::Interaction(InteractReq::WithdrawX {
                    action,
                    count,
                    bank_item_id,
                    lands_as_id,
                    ..
                }) => {
                    assert_eq!((bank_item_id, lands_as_id), (id, id));
                    clicks.push(action);
                    held += count;
                }
                _ => panic!("expected a withdraw click"),
            }
            snapshot.seed_inventory(vec![item(id, "Coins", held, ItemContainer::Inventory)], 28);
            snapshot.seed_bank_observation(10, 1, bank(50 - held), Vec::new());
        }
        assert_eq!(clicks, ["Withdraw-5", "Withdraw-1", "Withdraw-1"]);
        let receipt = with_tick(&snapshot, &mut ledger, 5, |tick| {
            match tick.actions.poll(&handle, &mut tick.cx) {
                Poll::Ready(Ok(receipt)) => receipt,
                other => panic!("held 7 completes the request: {other:?}"),
            }
        });
        assert!(receipt.complete);
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }

    /// `partial_ok` returns the incomplete receipt; an unmet exact target is
    /// never reported complete.
    #[test]
    fn partial_withdraw_of_missing_stock_is_incomplete() {
        let id = 314;
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(Vec::new(), 28);
        snapshot.seed_bank_observation(
            10,
            1,
            Some(vec![item(id, "Bait", 0, ItemContainer::Bank)]),
            Vec::new(),
        );
        let mut ledger = None;
        let request = bank_request(
            BankAction::Withdraw {
                item: BankItem {
                    id,
                    name: Arc::from("Bait"),
                },
                qty: 5,
            },
            true,
        );
        let (_, poll) = begin_bank(&snapshot, &mut ledger, request);
        assert!(matches!(
            poll,
            Poll::Ready(Ok(BankReceipt {
                complete: false,
                ..
            }))
        ));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }

    /// A one-row deposit presses that exact side row by id/slot/component;
    /// a same-named noted row is never re-found by name.
    #[test]
    fn deposit_presses_the_exact_side_row() {
        let (trout, noted) = (333, 334);
        let deposit_ops = ["Deposit-1", "Deposit-5", "Deposit-All"];
        let side = vec![
            row_with(
                item(trout, "Trout", 2, ItemContainer::BankSide),
                &deposit_ops,
                0,
            ),
            row_with(
                item(noted, "Trout", 9, ItemContainer::BankSide),
                &deposit_ops,
                1,
            ),
        ];
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(
            vec![
                item(trout, "Trout", 2, ItemContainer::Inventory),
                item(noted, "Trout", 9, ItemContainer::Inventory),
            ],
            28,
        );
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), side.clone());
        let mut ledger = None;
        let request = bank_request(
            BankAction::Deposit {
                item: BankItem {
                    id: noted,
                    name: Arc::from("Trout"),
                },
            },
            false,
        );
        let (handle, poll) = begin_bank(&snapshot, &mut ledger, request);
        assert!(poll.is_pending());
        let generation = snapshot.bank_session_generation();
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::InvButton {
                id,
                slot: 1,
                component: 7,
                operation: 3,
                bank_generation,
            }) if id == noted && bank_generation == generation
        ));
        snapshot.seed_inventory(vec![item(trout, "Trout", 2, ItemContainer::Inventory)], 28);
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), side[..1].to_vec());
        let poll = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(matches!(
            poll,
            Poll::Ready(Ok(BankReceipt { complete: true, .. }))
        ));
    }

    /// A deposit settles only in the bank session that sent it: a held count
    /// that falls while the bank closes, or closes and reopens as a new
    /// session, is never a completed deposit (Required or until-empty).
    #[test]
    fn deposit_does_not_settle_after_its_bank_session_is_gone() {
        let trout = 333;
        let side = vec![row_with(
            item(trout, "Trout", 2, ItemContainer::BankSide),
            &["Deposit-1", "Deposit-5", "Deposit-All"],
            0,
        )];
        let required = || {
            bank_request(
                BankAction::Deposit {
                    item: BankItem {
                        id: trout,
                        name: Arc::from("Trout"),
                    },
                },
                false,
            )
        };
        let until_empty = || {
            bank_request(
                BankAction::DepositAll {
                    keep: Arc::from([]),
                },
                false,
            )
        };
        let mut wrong = Vec::new();
        for (label, request, reopen) in [
            ("required, closed", required(), false),
            ("required, reopened", required(), true),
            ("until-empty, closed", until_empty(), false),
            ("until-empty, reopened", until_empty(), true),
        ] {
            let mut snapshot = GameSnapshot::new();
            snapshot.seed_ingame(2);
            snapshot.seed_inventory(vec![item(trout, "Trout", 2, ItemContainer::Inventory)], 28);
            snapshot.seed_bank_observation(10, 1, Some(Vec::new()), side.clone());
            let generation = snapshot.bank_session_generation();
            let mut ledger = None;
            let (handle, poll) = begin_bank(&snapshot, &mut ledger, request);
            assert!(poll.is_pending(), "{label}: {poll:?}");
            assert!(
                matches!(
                    acknowledge(&mut ledger, 1),
                    HostEffect::Interaction(InteractReq::InvButton { id, bank_generation, .. })
                        if id == trout && bank_generation == generation
                ),
                "{label}: the deposit click names its session"
            );

            // The held count drops in the same observation that loses G.
            snapshot.seed_inventory(Vec::new(), 28);
            snapshot.seed_bank_observation(-1, 1, None, Vec::new());
            if reopen {
                snapshot.seed_bank_observation(10, 1, Some(Vec::new()), Vec::new());
                assert_ne!(snapshot.bank_session_generation(), generation);
            }
            let poll = with_tick(&snapshot, &mut ledger, 2, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            });
            let refused = matches!(poll, Poll::Ready(Err(ActionError::Failed(_))))
                && ledger.as_ref().unwrap().outbox.is_empty();
            if !refused {
                wrong.push(format!("{label}: {poll:?}"));
            }
        }
        assert!(
            wrong.is_empty(),
            "a deposit outside its bank session must fail without another click: {wrong:#?}"
        );
    }

    /// Until-empty deposit: a posted empty side settles at once; a side
    /// root still down is not an empty side and waits the 1.2 s view bound.
    #[test]
    fn deposit_all_reads_the_side_root_not_the_list_length() {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(vec![item(526, "Bones", 1, ItemContainer::Inventory)], 28);
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), Vec::new());
        let all = || {
            bank_request(
                BankAction::DepositAll {
                    keep: Arc::from([]),
                },
                false,
            )
        };
        let mut ledger = None;
        let (_, poll) = begin_bank(&snapshot, &mut ledger, all());
        assert!(matches!(
            poll,
            Poll::Ready(Ok(BankReceipt { complete: true, .. }))
        ));

        snapshot.seed_side_modal(-1);
        let mut ledger = None;
        let (handle, poll) = begin_bank(&snapshot, &mut ledger, all());
        assert!(poll.is_pending(), "a side root still down is not posted");
        with_tick(&snapshot, &mut ledger, 2, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        let poll = with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(
            matches!(poll, Poll::Ready(Ok(BankReceipt { complete: true, .. }))),
            "the until-empty view bound settles: {poll:?}"
        );
        assert!(ledger.as_ref().unwrap().outbox.is_empty());

        // Required never settles on that bound.
        let mut ledger = None;
        let (handle, poll) = begin_bank(
            &snapshot,
            &mut ledger,
            bank_request(
                BankAction::DepositProducts {
                    products: Arc::from([526]),
                    keep: Arc::from([]),
                },
                false,
            ),
        );
        assert!(poll.is_pending());
        with_tick(&snapshot, &mut ledger, 3, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
    }

    /// P-close (native): already shut is true with no verb. A Close is done
    /// only once the bank is shut, its old side root released and the
    /// session generation newer; main shut with the side still up keeps
    /// waiting. A reopened session or the 4 s bound fails it.
    #[test]
    fn close_settles_on_the_acknowledged_close_not_the_main_root() {
        let open = || {
            let mut snapshot = GameSnapshot::new();
            snapshot.seed_ingame(2);
            snapshot.seed_inventory(Vec::new(), 28);
            snapshot.seed_bank_observation(10, 1, Some(Vec::new()), Vec::new());
            snapshot
        };
        let close = || bank_request(BankAction::Close, false);
        let poll_at = |snapshot: &GameSnapshot,
                       ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
                       handle: &crate::native::ActionHandle<BankMachine>,
                       tick: u64| {
            with_tick(snapshot, ledger, tick, |t| {
                t.actions.poll(handle, &mut t.cx)
            })
        };

        let mut shut = GameSnapshot::new();
        shut.seed_ingame(2);
        let mut ledger = None;
        let (_, poll) = begin_bank(&shut, &mut ledger, close());
        assert!(matches!(
            poll,
            Poll::Ready(Ok(BankReceipt { complete: true, .. }))
        ));
        assert!(ledger
            .as_ref()
            .is_none_or(|ledger| ledger.outbox.is_empty()));

        let mut snapshot = open();
        let side = snapshot.modals().side;
        assert!(side >= 0, "the fixture raises a side root");
        let mut ledger = None;
        let (handle, poll) = begin_bank(&snapshot, &mut ledger, close());
        assert!(poll.is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::Close)
        ));
        snapshot.seed_bank_observation(-1, 1, None, Vec::new());
        snapshot.seed_side_modal(side);
        assert!(
            poll_at(&snapshot, &mut ledger, &handle, 2).is_pending(),
            "main shut, old side root still up"
        );
        snapshot.seed_side_modal(-1);
        assert!(matches!(
            poll_at(&snapshot, &mut ledger, &handle, 3),
            Poll::Ready(Ok(BankReceipt { complete: true, .. }))
        ));

        // Reopened as another session before the close settled.
        let mut snapshot = open();
        let mut ledger = None;
        let (handle, _) = begin_bank(&snapshot, &mut ledger, close());
        let _ = acknowledge(&mut ledger, 1);
        snapshot.seed_bank_observation(-1, 1, None, Vec::new());
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), Vec::new());
        assert!(matches!(
            poll_at(&snapshot, &mut ledger, &handle, 2),
            Poll::Ready(Err(ActionError::Failed(_)))
        ));

        // Still open at the 4 s bound (tick 1 + 4 s is tick 8 at 600 ms).
        let snapshot = open();
        let mut ledger = None;
        let (handle, _) = begin_bank(&snapshot, &mut ledger, close());
        let _ = acknowledge(&mut ledger, 1);
        assert!(poll_at(&snapshot, &mut ledger, &handle, 7).is_pending());
        assert!(matches!(
            poll_at(&snapshot, &mut ledger, &handle, 8),
            Poll::Ready(Err(ActionError::Failed(_)))
        ));
    }

    #[test]
    fn named_bank_selector_fails_closed_on_skill_quest_and_preference_gates() {
        let public = NamedBank::new(
            "Public",
            WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
        );
        let facts = Arc::new(NamedBankFacts::from_banks(vec![
            public,
            catalog_bank("Fishing Guild"),
            catalog_bank("Zanaris"),
        ]));
        let quest = [QuestStatusView {
            component_id: 0,
            name: "Lost City".to_owned(),
            colour: 0x00F800,
        }];
        let opted_in = BankSelector::new(
            Arc::clone(&facts),
            BankPreferences {
                use_mage_bank: false,
                use_zanaris_bank: true,
            },
        );
        assert_eq!(opted_in.eligible_indices(&fishing(67), &quest), vec![0, 2]);
        assert_eq!(opted_in.named("Fishing Guild", &fishing(67), &quest), None);
        assert_eq!(opted_in.eligible_indices(&fishing(68), &[]), vec![0, 1]);
        assert_eq!(opted_in.named("Zanaris", &fishing(68), &[]), None);

        let defaults = BankSelector::new(
            facts,
            BankPreferences {
                use_mage_bank: false,
                use_zanaris_bank: false,
            },
        );
        assert_eq!(defaults.eligible_indices(&fishing(68), &quest), vec![0, 1]);
    }

    #[test]
    fn native_bank_close_budget_denial_retries_next_tick() {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(Vec::new(), 28);
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), Vec::new());
        assert_bank_emit_deferred(
            &snapshot,
            bank_request(BankAction::Close, false),
            |effect| {
                assert!(matches!(
                    effect,
                    HostEffect::Interaction(InteractReq::Close)
                ))
            },
        );
    }

    #[test]
    fn native_bank_catalog_loc_budget_denial_retries_next_tick() {
        let bank = catalog_bank("Shantay Pass");
        let loc_tile = WorldTile {
            x: bank.tile.x + 1,
            z: bank.tile.z,
            level: bank.tile.level,
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(bank.tile));
        snapshot.seed_locs(vec![bank_object_loc(
            2693,
            "Shantay chest",
            "Open",
            loc_tile,
            1,
        )]);
        let reach = object_reach_view(bank.tile, loc_tile);
        let access = Arc::new(BankStandAccess {
            bank,
            stand_tile: loc_tile,
            kind: AccessKind::Booth,
            stand_op: 0,
            name: Some(Arc::from("Shantay chest")),
            choose: None,
        });
        let mut ledger = None;
        let handle = with_tick_reach(&snapshot, &reach, &mut ledger, 1, |tick| {
            tick.actions
                .begin::<BankMachine>(open_request(access), &mut tick.cx)
                .unwrap()
        });
        let denied = with_tick_reach(&snapshot, &reach, &mut ledger, 2, |tick| {
            spend_interaction(&mut tick.cx);
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(denied.is_pending(), "budget denial must remain Pending");
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
        let retry = with_tick_reach(&snapshot, &reach, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(retry.is_pending());
        assert!(matches!(
            ledger.as_ref().unwrap().outbox[0].effect,
            HostEffect::Interaction(InteractReq::Loc { id: Some(2693), .. })
        ));
    }

    #[test]
    fn native_bank_open_stand_budget_denial_retries_next_tick() {
        let tile = WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        };
        let bank = NamedBank::new("Fixture bank", tile);
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_locs(vec![bank_object_loc(
            4010,
            "Fixture booth",
            "Use-quickly",
            tile,
            0,
        )]);
        let access = Arc::new(BankStandAccess {
            bank,
            stand_tile: tile,
            kind: AccessKind::Booth,
            stand_op: 1,
            name: Some(Arc::from("Fixture booth")),
            choose: None,
        });
        assert_bank_emit_deferred(&snapshot, open_request(access), |effect| {
            assert!(matches!(
                effect,
                HostEffect::Interaction(InteractReq::OpenStand {
                    stand_op: Some(1),
                    ..
                })
            ));
        });
    }

    #[test]
    fn native_bank_open_booth_budget_denial_retries_next_tick() {
        let tile = WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        };
        let bank = NamedBank::new("Fixture bank", tile);
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_locs(vec![bank_object_loc(
            4011,
            "Fixture booth",
            "Use-quickly",
            tile,
            0,
        )]);
        assert_bank_emit_deferred(
            &snapshot,
            BankRequest {
                bank: Some(bank),
                action: BankAction::Scan,
                partial_ok: false,
            },
            |effect| {
                assert!(matches!(
                    effect,
                    HostEffect::Interaction(InteractReq::OpenBooth { id: 4011, .. })
                ));
            },
        );
    }

    #[test]
    fn native_bank_deposit_budget_denial_retries_next_tick() {
        let id = 333;
        let side = row_with(
            item(id, "Trout", 2, ItemContainer::BankSide),
            &["Deposit-1", "Deposit-5", "Deposit-All"],
            0,
        );
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(vec![item(id, "Trout", 2, ItemContainer::Inventory)], 28);
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), vec![side]);
        assert_bank_emit_deferred(
            &snapshot,
            bank_request(
                BankAction::Deposit {
                    item: BankItem {
                        id,
                        name: Arc::from("Trout"),
                    },
                },
                false,
            ),
            |effect| {
                assert!(matches!(
                    effect,
                    HostEffect::Interaction(InteractReq::InvButton { id: button_id, .. })
                        if *button_id == id
                ));
            },
        );
    }

    #[test]
    fn native_bank_note_mode_budget_denial_retries_next_tick() {
        let id = 314;
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(Vec::new(), 28);
        snapshot.seed_bank_observation(
            10,
            1,
            Some(vec![item(id, "Bait", 5, ItemContainer::Bank)]),
            Vec::new(),
        );
        assert_bank_emit_deferred(
            &snapshot,
            bank_request(
                BankAction::WithdrawTo {
                    withdrawals: Arc::from([Withdrawal {
                        id,
                        name: Arc::from("Bait"),
                        target: 2,
                    }]),
                },
                false,
            ),
            |effect| {
                assert!(matches!(
                    effect,
                    HostEffect::Interaction(InteractReq::SetNoteMode { on: false })
                ));
            },
        );
    }

    #[test]
    fn native_bank_withdraw_click_budget_denial_retries_next_tick() {
        let id = 314;
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(Vec::new(), 28);
        snapshot.seed_bank_observation(
            10,
            1,
            Some(vec![item(id, "Bait", 5, ItemContainer::Bank)]),
            Vec::new(),
        );
        let mut ledger = None;
        let handle = start_bank(
            &snapshot,
            &mut ledger,
            bank_request(
                BankAction::WithdrawTo {
                    withdrawals: Arc::from([Withdrawal {
                        id,
                        name: Arc::from("Bait"),
                        target: 2,
                    }]),
                },
                false,
            ),
        );
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 2),
            HostEffect::Interaction(InteractReq::SetNoteMode { on: false })
        ));

        let denied = with_tick(&snapshot, &mut ledger, 3, |tick| {
            spend_batch_event(&mut tick.cx);
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(denied.is_pending(), "budget denial must remain Pending");
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
        let retry = with_tick(&snapshot, &mut ledger, 4, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(retry.is_pending());
        assert!(matches!(
            ledger.as_ref().unwrap().outbox[0].effect,
            HostEffect::Interaction(InteractReq::WithdrawX { count: 2, .. })
        ));
    }

    #[test]
    fn native_teller_talk_budget_denial_retries_next_tick() {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_npcs(vec![npc_row(7, "Gundai", &["Talk-to"])]);
        assert_bank_emit_deferred(
            &snapshot,
            open_request(teller_access(
                catalog_bank("Mage Arena"),
                Some("Gundai"),
                0,
                None,
            )),
            |effect| {
                assert!(matches!(
                    effect,
                    HostEffect::Interaction(InteractReq::Npc {
                        name,
                        index: Some(7),
                        ..
                    }) if name == "Gundai"
                ));
            },
        );
    }

    fn assert_teller_page_deferral(
        page: &GameSnapshot,
        access: Arc<BankStandAccess>,
        assert_emitted: impl FnOnce(&HostEffect),
    ) {
        let mut initial = GameSnapshot::new();
        initial.seed_ingame(2);
        initial.seed_npcs(vec![npc_row(7, "Gundai", &["Talk-to"])]);
        let mut ledger = None;
        let handle = with_tick_snapshots(&initial, page, &mut ledger, 1, |tick, next| {
            let handle = tick
                .actions
                .begin::<BankMachine>(open_request(access), &mut tick.cx)
                .unwrap();
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            assert!(matches!(
                acknowledge_current(&mut tick.cx),
                HostEffect::Interaction(InteractReq::Npc { .. })
            ));

            let mut evidence = tick.cx.evidence();
            evidence.sequence += 1;
            tick.cx.evidence = evidence;
            tick.cx.snapshot = SnapshotView::new(Some(next), evidence);
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            assert!(
                tick.cx.ledger.as_ref().unwrap().outbox.is_empty(),
                "same-tick page wake must defer its denied action"
            );
            handle
        });
        let retry = with_tick(page, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(retry.is_pending());
        assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
        assert_emitted(&ledger.as_ref().unwrap().outbox[0].effect);
    }

    #[test]
    fn native_teller_continue_page_budget_denial_retries_next_tick() {
        let mut page = GameSnapshot::new();
        page.seed_ingame(2);
        page.seed_chat_modal(1, vec!["Bank access page".into()]);
        page.seed_chat_options(Vec::new(), 99);
        assert_teller_page_deferral(
            &page,
            teller_access(catalog_bank("Mage Arena"), Some("Gundai"), 0, None),
            |effect| {
                assert!(matches!(
                    effect,
                    HostEffect::Interaction(InteractReq::ContinueDialog { component_id: None })
                ));
            },
        );
    }

    #[test]
    fn native_teller_choice_page_budget_denial_retries_next_tick() {
        let mut page = GameSnapshot::new();
        page.seed_ingame(2);
        page.seed_chat_modal(1, vec!["Bank access page".into()]);
        page.seed_chat_options(
            vec![
                api::snapshot::ChatOptionView {
                    component_id: 1,
                    text: "Not now".into(),
                },
                api::snapshot::ChatOptionView {
                    component_id: 2,
                    text: "I'd like to access my bank account".into(),
                },
            ],
            -1,
        );
        assert_teller_page_deferral(
            &page,
            teller_access(catalog_bank("Mage Arena"), Some("Gundai"), 0, None),
            |effect| {
                assert!(matches!(
                    effect,
                    HostEffect::Interaction(InteractReq::Answer { option: 2 })
                ));
            },
        );
    }
}
