//! Snapshot-driven bank core shared by compiled cards.
//!
//! Selection owns immutable [`NamedBankFacts`] explicitly. Operations never
//! trust dispatch receipts as transfers: every withdraw/deposit settles from a
//! newer inventory observation in the same bank session. Click choice,
//! completion and deposit settlement are the shared [`crate::bank::ops`]
//! kernel.
use crate::bank::npc;
use crate::bank::ops::{
    self, deposit_option, CloseBaseline, CloseScan, DepositKind, DepositScan, DepositSpec,
    NoteIntent, Progress, WithdrawGoal, MAX_DEPOSITS, MAX_MEMO,
};
use crate::interact::InteractReq;
use crate::native::{ActionContext, ActionError, BankDepositTrace, NativeMachine, WalkOptions};
use api::named_banks::{BankOperation, BankPreferences, NamedBank, NamedBankFacts};
use api::quest_progress::EvidenceStamp;
use api::selected::Truth;
use api::snapshot::{LocView, QuestListStatus, QuestStatusView, StatView, WorldTile};
use serde_json::{json, Value};
use std::num::NonZeroU64;
use std::sync::Arc;
use std::task::Poll;

/// Observed ticks a deposit waits for the bank side view after the open
/// session's table is up (the view the deposit click is read from, and the
/// pack it must agree with).
///
/// Bound from the engine and content, not from a clock: the `openbank` label
/// raises the side root and starts the backpack's transmission in the same
/// script run as the main modal (`bank.rs2:18-23`: `inv_transmit(inv,
/// bank_side:inv)`, `inv_transmit(bank, bank_main:inv)`,
/// `if_openmain_side(bank_main, bank_side)`). The engine writes every
/// first-seen or changed inventory listener in one pass at the end of the
/// server cycle (`NetworkPlayer.ts:332-377`, called from
/// `World.ts:1109-1116`), so the side backpack, the main table and each
/// deposit's change to both reach the client in one flush; the deposit script
/// applies a click inline (`bank_deposit.rs2:27`, `bank.rs2:102`). The side
/// view is therefore observed in the same observed tick as the table. The
/// bound allows the next observed tick for a flush that the host read in two
/// parts; a view still missing one full server cycle later was not sent, so
/// the request fails instead of waiting. Real-engine measurement of this
/// bound: `EVENT-RUNTIME-WAKE/r1/report.md` "Side view".
const SIDE_VIEW_TICKS: u64 = 1;
/// The window in which the host's bank pick job must publish a receipt, in
/// wall milliseconds. It is the host-job contract, written once here and read
/// by the host worker (`host-play` `script_bank.rs` `BANK_SELECTION_WINDOW`):
/// the worker ranks candidates under this bound and, when it is spent,
/// publishes its own air-distance fallback receipt of the same kind that
/// [`Select`] holds. The window is a host-job bound, so it is a wall time by
/// contract; the kernel counts it in observed ticks from its dispatch.
pub const BANK_PICK_HOST_WINDOW_MS: u64 = 5_000;
/// The observed ticks after its dispatch at which [`Select`] takes its own
/// air-distance fallback when no receipt came: the host window, rounded up to
/// whole production server ticks (`ops::SERVER_TICK_MS`), so the kernel never
/// gives up before the host job's own window has closed at the production
/// tick length. A receipt that arrives with the bound tick still wins.
const SELECT_FALLBACK_TICKS: u64 = ops::observed_ticks_for_millis(BANK_PICK_HOST_WINDOW_MS);
/// Re-opens of a lost session within one transfer request (deposit or
/// withdrawal), after the first open; exhausted, the loss is terminal with
/// its original message.
///
/// Bound from the engine: while the bank modal is up the player is `busy()`
/// (`Player.ts:821-823`), so normal queues, weak queues and normal timers
/// cannot run (`Player.ts:904`, `:919`, `:953`); only the player's own
/// client packets (`Op*Handler`/`MoveClickHandler` → `clearPendingAction`,
/// an `if_button` script with `if_close`) or a strong-queued script
/// (`Player.ts:874-885`) can close it. No content strong-queues a script on
/// a player banking without an action of their own (`grep strongqueue`
/// over `content/scripts`: combat/spec/greegree/quest/minigame triggers and
/// the run button, `player_controls.rs2:15,34`), and the kernel sends no
/// packet but its own transfer clicks while a transfer is open. The one
/// reachable trigger was the host's run press (`controls:com_5`), which
/// fires at most once per energy cycle: a press needs energy ≥ 20 with run
/// off, run is forced off only at 0, and 0 → 20 takes
/// `2000 / (floor(agility/6) + 8)` non-running ticks (`Player.ts:705-707`),
/// ≥ 84 ticks at agility 99, while one session attempt is bounded at
/// 8 + 8 + 8 observed ticks (open, table, transfer). So at most one close
/// per transfer comes from content; one re-open covers it. Measured live
/// with the host guard removed (HOTFIX-0.2.1 report-r2 §F4: 20 bots, 767
/// trips): six presses landed under the open bank, each followed by the
/// engine's stop-transmit and close within a tick; the one that cut a
/// transfer resumed on its first re-open, none needed a second. The second
/// is the margin for a close the content enumeration does not produce (a
/// reconnect, `Player.ts:562`; an operator cheat, `ClientCheatHandler.ts`).
pub const MAX_BANK_REOPENS: u8 = 2;

pub fn air_distance_squared(a: WorldTile, b: WorldTile) -> i64 {
    let dx = i64::from(a.x) - i64::from(b.x);
    let dz = i64::from(a.z) - i64::from(b.z);
    dx * dx + dz * dz
}

pub fn nearest_eligible_bank(
    facts: &NamedBankFacts,
    from: WorldTile,
    mut eligible: impl FnMut(&NamedBank) -> bool,
) -> Option<NamedBank> {
    facts
        .banks()
        .iter()
        .filter(|bank| eligible(bank))
        .min_by_key(|bank| air_distance_squared(from, bank.air_tile()))
        .copied()
}

pub fn bank_value(bank: &NamedBank) -> Value {
    let mut value = json!({"name": bank.name, "tile": {"x": bank.tile.x, "z": bank.tile.z, "level": bank.tile.level}});
    if let Some(definition) = bank.definition {
        if let Some(approach) = definition.approach {
            value["approach"] = json!({"x": approach.x, "z": approach.z, "level": approach.level});
        }
        if let Some(npc) = definition.npc {
            value["npcAccess"] = json!({"name": npc.name, "op": npc.op});
            if let Some(choose) = definition.choose {
                value["npcAccess"]["choose"] = json!(choose);
            }
        } else if let Some(object) = definition.object {
            value["access"] = json!({"name": object.name, "op": object.op});
            if let Some(open) = definition.open_first {
                value["access"]["openFirst"] = json!({"name": open.name, "op": open.op});
            }
        }
        if definition.skill.is_some() || definition.quest.is_some() || definition.setting.is_some()
        {
            value["requires"] = json!({});
            if let Some((_, level)) = definition.skill {
                value["requires"]["skill"] = json!({"name": "fishing", "level": level});
            }
            if let Some(quest) = definition.quest {
                value["requires"]["quest"] = json!(quest);
            }
            if let Some(setting) = definition.setting {
                value["requires"]["setting"] = json!(setting);
            }
        }
    }
    value
}

#[derive(Clone)]
pub struct BankSelector {
    facts: Arc<NamedBankFacts>,
    preferences: BankPreferences,
}

impl BankSelector {
    pub fn new(facts: Arc<NamedBankFacts>, preferences: BankPreferences) -> Self {
        Self { facts, preferences }
    }

    pub(crate) fn eligible(
        &self,
        bank: &NamedBank,
        stats: &[StatView],
        quests: &[QuestStatusView],
    ) -> bool {
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
    /// Air-ranked, with stable catalog ties, when `allow_air_fallback` is true.
    pub eligible: Arc<[u16]>,
    /// When present, no other eligible bank may be selected.
    pub explicit_bank: Option<u16>,
    /// Select-only compatibility may return an eligible air-ranked bank after
    /// bounded routing fails. Native callers leave this false.
    pub allow_air_fallback: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BankPickOutcome {
    Selected(SelectedBank),
    NeedsStep {
        gates: Arc<[api::prereq_steps::ActionableGate]>,
        allow: crate::native::PrereqAllow,
    },
    NoCandidate {
        detail: Arc<str>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankPickReceipt {
    pub request_id: u64,
    pub evidence: EvidenceStamp,
    pub outcome: BankPickOutcome,
}

/// Revision-bound service rows shared by every bot using the same nav world.
///
/// Bank placements come from the selected bank roster. Shop rows come from
/// pinned `keepers.npc` config/`owned_shop` joins and selected NPC placements.
#[derive(Debug, Clone)]
pub enum ServiceFacts {
    /// The existing selected-content named-bank roster.
    Bank(Arc<NamedBankFacts>),
    /// Pinned keeper placements with access rings resolved once at world bind.
    ShopKeeper(Arc<api::service_pick::ShopKeeperFacts>),
}

impl ServiceFacts {
    /// The row domain this shared selected-content owner contains.
    pub fn kind(&self) -> api::service_pick::ServiceKind {
        match self {
            Self::Bank(_) => api::service_pick::ServiceKind::Bank,
            Self::ShopKeeper(_) => api::service_pick::ServiceKind::ShopKeeper,
        }
    }
}

/// One off-pump route-cost pick over the complete account-eligible row domain.
#[derive(Debug, Clone)]
pub struct ServicePickRequest {
    /// Must match `facts.kind()`; a mismatch is not a selectable domain.
    pub kind: api::service_pick::ServiceKind,
    /// The player's observed world tile and level.
    pub from: WorldTile,
    /// The account's walk restrictions; service searches force teleports off.
    pub options: WalkOptions,
    /// The revision-bound shared rows; a request copies no access lists.
    pub facts: ServiceFacts,
    /// Eligible owner row indices, air-nearest first only for equal-cost ties.
    pub eligible: Arc<[u16]>,
    /// Break equal route costs by `eligible` order, never by a partial search.
    pub prefer_order: bool,
}

/// An established route result or an explicit undecided service pick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServicePickOutcome {
    /// The cheapest reachable access in the complete eligible target set.
    Routed {
        /// The chosen row in the bound facts owner.
        index: u16,
        /// A prevalidated stand outside the selected service footprint.
        access_tile: WorldTile,
        /// The router's accumulated tick cost.
        cost: u32,
    },
    /// No account-eligible row had usable access before searching.
    NoCandidate,
    /// The complete target search established absence of a permitted path.
    NoPath {
        /// Only still-unknown gates on the cheapest relaxed diagnosis route.
        unresolved: Option<Arc<[api::selected::QuestGate]>>,
    },
    /// The search work limit was reached; reachability remains undecided.
    BudgetExhausted,
    /// The selection window expired or work was refused; not proof of absence.
    NotSettled,
}

/// Run-fenced response to one [`ServicePickRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServicePickReceipt {
    /// The native action request this observation answers.
    pub request_id: u64,
    /// The requesting run and observed snapshot stamp.
    pub evidence: EvidenceStamp,
    /// The established or undecided search result.
    pub outcome: ServicePickOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Withdrawal {
    pub id: i32,
    pub name: Arc<str>,
    /// Exact final inventory count, not an amount to add.
    pub target: i32,
}

#[derive(Debug, Clone)]
pub struct SelectArgs {
    pub facts: Arc<NamedBankFacts>,
    pub from: WorldTile,
    pub preferences: BankPreferences,
    pub options: WalkOptions,
    pub explicit: Option<Arc<str>>,
    /// Opt in only for select-only contracts that promise an air fallback.
    pub allow_air_fallback: bool,
}

pub struct OpenArgs {
    pub access: Arc<BankStandAccess>,
}

pub struct DepositArgs {
    pub products: Arc<[i32]>,
    pub keep: Arc<[i32]>,
    /// The stand that opened this session; a session lost before the
    /// deposit settles is re-opened from it (`MAX_BANK_REOPENS`).
    pub access: Option<Arc<BankStandAccess>>,
}

pub struct WithdrawArgs {
    pub withdrawals: Arc<[Withdrawal]>,
    /// The stand that opened this session; see [`DepositArgs::access`].
    pub access: Option<Arc<BankStandAccess>>,
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
        /// One runtime-selected survival item preserved without copying `keep`.
        extra_keep: Option<i32>,
        slots: u8,
    },
    Close,
}

#[derive(Debug, Clone)]
pub struct BankRequest {
    pub bank: Option<NamedBank>,
    pub action: BankAction,
    pub partial_ok: bool,
    /// The stand that opened the session a transfer verb runs in. A
    /// transfer that loses its session re-opens the stand and resumes;
    /// without it the loss is terminal. Ignored by `OpenStand` (its own
    /// access), `Scan` and `Close`.
    pub access: Option<Arc<BankStandAccess>>,
}

/// One protected pack row's count before a deposit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BankCount {
    pub id: i32,
    pub count: i32,
}

const CAPACITY_PACK_SLOTS: usize = 28;

enum ProtectedCounts {
    Products(Vec<BankCount>),
    // Zero-count entries are empty. The complete keep set stays in the request.
    Capacity([BankCount; CAPACITY_PACK_SLOTS]),
}

// Measured at 232 B; the 64 B headroom retains the bounded inline 28-row
// protected-count snapshot without per-check allocation.
const _: () = assert!(std::mem::size_of::<ProtectedCounts>() <= 296);

/// What a bank action settled to. The bank's rows are not carried here: the
/// host observes the open table into the account's bank memory
/// (design-bank-snapshot §1.3), which every reader borrows through
/// `SnapshotView::stock()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankReceipt {
    pub complete: bool,
    /// Units whose deposit the bank machine confirmed in the observed rows.
    pub deposited: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AccessDialogue {
    Teller,
    Object,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ObjectDialogue {
    NotSent,
    Awaiting,
    /// A finished dialogue cannot restart while its modal remains open.
    Finished,
}

#[derive(Clone, Copy)]
enum Phase {
    Open,
    NpcAccess {
        core: npc::NpcAccess,
        dialogue: AccessDialogue,
        first_target: Option<(i32, WorldTile)>,
    },
    AwaitOpen {
        first_target: Option<(i32, WorldTile)>,
    },
    AwaitTable,
    AwaitNoteMode {
        request_id: u64,
        evidence: EvidenceStamp,
    },
    Act,
    AwaitTransfer {
        before: i32,
        before_slots: Option<u8>,
        item_id: i32,
        evidence: EvidenceStamp,
    },
    ReconcileDeposit {
        evidence: EvidenceStamp,
        reason: ops::DepositFailure,
    },
    AwaitClose {
        baseline: CloseBaseline,
        run: api::selected::RunKey,
    },
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct DepositObservationKey {
    main_now: Option<i32>,
    side_now: Option<i32>,
    bank_now: Option<i32>,
    main_revision: Option<u64>,
    side_revision: Option<u64>,
}

struct DepositLogState {
    request: u64,
    trace: Arc<BankDepositTrace>,
    last_observation: Option<DepositObservationKey>,
    last_logged_tick: u64,
    dispatch_evidence: Option<EvidenceStamp>,
    dispatch_accepted: Option<bool>,
    chat_since: Option<i32>,
}

/// A deposit click whose session was lost before its settlement was
/// observed: the clicked id's held count and (capacity) the pack's row
/// count before the click. Reconciled once against the next session's pack
/// before any further click, so what the lost session already applied is
/// credited and never repeated.
#[derive(Clone, Copy)]
struct InFlightDeposit {
    before: i32,
    before_slots: Option<u8>,
    item_id: i32,
}
/// The observed tick at which a bank wait gives up. It counts observed game
/// ticks only (operator 2026-10-19, `AGENTS.md` content-timed waits): no
/// elapsed-time floor and no wall clock. Every wait checks the awaited state
/// before its deadline, so a response that arrives with the tick that
/// reaches the deadline still settles the wait.
#[derive(Clone, Copy)]
struct BankDeadline {
    observed_tick: NonZeroU64,
}

impl BankDeadline {
    /// `tick_limit` observed ticks after `start_tick`.
    fn after(start_tick: u64, tick_limit: u64) -> Self {
        Self {
            observed_tick: NonZeroU64::new(start_tick.saturating_add(tick_limit).max(1))
                .expect("bank tick deadline is positive"),
        }
    }

    fn reached(self, tick: u64) -> bool {
        tick >= self.observed_tick.get()
    }
}

pub struct BankMachine {
    request: BankRequest,
    phase: Phase,
    /// Deadline for the active open, dialogue, table, or close wait.
    deadline: Option<BankDeadline>,
    session: u64,
    deposits: u8,
    withdraw_index: u8,
    protected_before: ProtectedCounts,
    deposited: u32,
    capacity_freed: u8,
    item_mode_ensured: bool,
    open_stage: u8,
    deposit_log: Option<Box<DepositLogState>>,
    object_dialogue: ObjectDialogue,
    /// The until-empty side-view bound is armed.
    view_armed: bool,
    /// The session generation when this machine sent its own bank-open request
    /// (stand, booth, bank object or teller). It is set only after that
    /// request was emitted. It stays empty while no such request went out.
    opened_from: Option<u64>,
    /// Re-opens taken after a lost session (`MAX_BANK_REOPENS`).
    reopens: u8,
    /// A lost session's stand click is due; a stale open without its table
    /// does not hold it back.
    reopen_pending: bool,
    /// The deposit in flight when the session was lost, until reconciled.
    in_flight: Option<InFlightDeposit>,
    /// One isolated no-op may be recovered within the entire deposit request.
    /// The live no-op fixture refuses exactly one transfer. Content applies
    /// each accepted deposit inline (`bank_deposit.rs2:27`, `bank.rs2:102`),
    /// and measured normal transfers settle at the next observation
    /// (USER-0.2.3-GATHERER/report.md §3). One fresh rescan/reissue covers
    /// that isolated fault, not persistent denial.
    /// MAX_DEPOSITS plus MAX_BANK_REOPENS plus this one
    /// recovery caps deposit sends at 35 per request.
    deposit_recovered: bool,
    /// Stop after the current transfer reaches an observed boundary.
    interrupted: bool,
}

const BANK_OBJECT_RADIUS: i32 = 4;

fn catalog_object_at<'a>(
    cx: &'a ActionContext<'_>,
    id: i32,
    tile: WorldTile,
    operation: BankOperation,
) -> Option<&'a LocView> {
    ops::booth_exact(
        cx.snapshot().locs()?.value,
        tile,
        id,
        operation.name,
        operation.op,
    )
}

/// Resolve a catalog object's live row around its declared access stand.
fn catalog_object_loc<'a>(
    cx: &'a ActionContext<'_>,
    stand_tile: WorldTile,
    operation: BankOperation,
) -> Option<&'a LocView> {
    ops::booth_catalog(
        cx.snapshot().locs()?.value,
        stand_tile,
        BANK_OBJECT_RADIUS,
        operation.name,
        operation.op,
    )
}

impl NativeMachine for BankMachine {
    type Args = BankRequest;
    type Output = BankReceipt;

    fn begin(request: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        let mut protected_before = ProtectedCounts::Products(Vec::new());
        match &request.action {
            BankAction::DepositProducts { products, keep } => {
                let mut before = Vec::with_capacity(keep.len());
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
                    if before.iter().any(|row: &BankCount| row.id == id) {
                        continue;
                    }
                    before.push(BankCount {
                        id,
                        count: ops::count_id(inventory.value, id),
                    });
                }
                protected_before = ProtectedCounts::Products(before);
            }
            BankAction::DepositCapacity {
                keep,
                extra_keep,
                slots,
            } => {
                if *slots == 0 || *slots > MAX_DEPOSITS {
                    return Err(ActionError::Unavailable(Arc::from(
                        "invalid exact bank capacity deposit plan",
                    )));
                }
                let inventory = cx
                    .snapshot()
                    .inventory()
                    .ok_or_else(|| ActionError::Unavailable(Arc::from("inventory unavailable")))?;
                if inventory.value.len() > CAPACITY_PACK_SLOTS {
                    return Err(ActionError::Unavailable(Arc::from(
                        "inventory exceeds bank capacity snapshot bound",
                    )));
                }
                let mut before = [BankCount { id: 0, count: 0 }; CAPACITY_PACK_SLOTS];
                let mut used = 0;
                for row in inventory.value {
                    let id = row.def.id;
                    if row.count <= 0
                        || !(keep.contains(&id) || *extra_keep == Some(id))
                        || before[..used].iter().any(|prior| prior.id == id)
                    {
                        continue;
                    }
                    before[used] = BankCount {
                        id,
                        count: ops::count_id(inventory.value, id),
                    };
                    used += 1;
                }
                protected_before = ProtectedCounts::Capacity(before);
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
        let open_started_tick = cx.evidence().tick;
        Ok(Self {
            request,
            phase: Phase::Open,
            deadline: Some(BankDeadline::after(
                open_started_tick,
                ops::TRANSFER_OBSERVED_TICK_LIMIT,
            )),
            session: 0,
            deposits: 0,
            withdraw_index: 0,
            protected_before,
            deposited: 0,
            capacity_freed: 0,
            item_mode_ensured: false,
            open_stage: 0,
            deposit_log: None,
            object_dialogue: ObjectDialogue::NotSent,
            view_armed: false,
            opened_from: None,
            reopens: 0,
            reopen_pending: false,
            in_flight: None,
            deposit_recovered: false,
            interrupted: false,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        loop {
            if self.interrupted && !self.transfer_in_flight() {
                return Poll::Ready(Err(ActionError::Cancelled));
            }
            let snapshot = cx.snapshot();
            let bank_open = snapshot
                .bank_session()
                .is_some_and(|session| session.value.open);
            let opening = !matches!(&self.request.action, BankAction::Close)
                && matches!(
                    self.phase,
                    Phase::Open | Phase::NpcAccess { .. } | Phase::AwaitOpen { .. }
                );
            // A session that appears after this machine's own open request is
            // that request's session, with or without its table yet. A session
            // seen in any later phase may be a player's reopen: it is not noted.
            if bank_open && (opening || matches!(self.phase, Phase::AwaitTable)) {
                let generation = open_generation(cx);
                if self.opened_from.is_some_and(|from| from != generation) {
                    cx.note_bank_opened_by_machine(generation);
                }
            }
            // A stale open whose table is down is a closed bank once a
            // transfer's reopen is due: the stand click goes out regardless
            // (the engine closes and re-opens on it, `OpLocHandler.ts:53`).
            if opening && bank_open && snapshot.bank().is_none() && !self.reopen_pending {
                self.arm_deadline(cx.evidence().tick, ops::TRANSFER_OBSERVED_TICK_LIMIT);
                self.phase = Phase::AwaitTable;
            }
            match self.phase {
                Phase::Open => {
                    if matches!(self.request.action, BankAction::Close) {
                        let Some(session) = cx.snapshot().bank_session().map(|s| s.value) else {
                            return Poll::Ready(Ok(self.receipt(true)));
                        };
                        if ops::close_begin(session.open).is_some() {
                            return Poll::Ready(Ok(self.receipt(true)));
                        }
                        let started_tick = cx.evidence().tick;
                        let run = cx.evidence().run;
                        std::task::ready!(crate::native::defer_budget(
                            cx.emit(InteractReq::Close)
                        ))?;
                        self.arm_deadline(started_tick, ops::TRANSFER_OBSERVED_TICK_LIMIT);
                        self.phase = Phase::AwaitClose {
                            baseline: CloseBaseline {
                                generation: session.generation,
                                side: session.side,
                            },
                            run,
                        };
                        return Poll::Pending;
                    }
                    let stand = self.stand_access().cloned();
                    let is_open_stand = matches!(self.request.action, BankAction::OpenStand { .. });
                    let teller = stand
                        .as_ref()
                        .is_some_and(|access| access.kind == AccessKind::Teller);
                    // A transfer verb re-opening through a teller goes to
                    // the dialogue only while the table is down.
                    if teller && (is_open_stand || cx.snapshot().bank().is_none()) {
                        // The teller attempt counts only once its core sends a
                        // request (`NpcAccess` phase below), not when it starts.
                        self.opened_from = None;
                        self.deadline = None;
                        self.phase = Phase::NpcAccess {
                            core: npc::NpcAccess::new(),
                            dialogue: AccessDialogue::Teller,
                            first_target: None,
                        };
                        continue;
                    }
                    if cx.snapshot().bank().is_some() {
                        self.session = cx
                            .snapshot()
                            .bank_session()
                            .map_or(0, |session| session.value.generation);
                        self.reopen_pending = false;
                        self.clear_wait_deadlines();
                        self.phase = Phase::Act;
                        continue;
                    }
                    if self.deadline_reached(cx.evidence().tick) {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "bank did not open",
                        ))));
                    }
                    let mut first_target = None;
                    if let Some(access) = stand.as_ref() {
                        let definition = access.bank.definition;
                        let object = definition.and_then(|definition| definition.object);
                        let open_first = definition.and_then(|definition| definition.open_first);
                        if object.is_some() || open_first.is_some() {
                            // Another player can leave the final bank target open.
                            // Probe it before the opener, as ObjectOpen does.
                            let target = object
                                .and_then(|operation| {
                                    catalog_object_loc(cx, access.stand_tile, operation)
                                        .map(|candidate| (operation, candidate, false))
                                })
                                .or_else(|| {
                                    open_first.filter(|_| self.open_stage == 0).and_then(
                                        |operation| {
                                            catalog_object_loc(cx, access.stand_tile, operation)
                                                .map(|candidate| (operation, candidate, true))
                                        },
                                    )
                                });
                            let Some((operation, candidate, opening_first)) = target else {
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
                            first_target = opening_first.then_some((loc.id, loc.tile));
                            std::task::ready!(crate::native::defer_budget(cx.emit(
                                InteractReq::Loc {
                                    x: loc.tile.x,
                                    z: loc.tile.z,
                                    level: loc.tile.level,
                                    action: operation.op.to_owned(),
                                    id: Some(loc.id),
                                },
                            )))?;
                            self.object_dialogue = ObjectDialogue::Awaiting;
                        } else {
                            let Some(locs) = cx.snapshot().locs() else {
                                return Poll::Pending;
                            };
                            let wanted_name = access.name.as_deref();
                            let Some(loc) =
                                ops::booth_at(locs.value, access.stand_tile, wanted_name)
                            else {
                                return Poll::Pending;
                            };
                            let stand_op = if access.stand_op > 0 {
                                Some(access.stand_op)
                            } else {
                                ops::op_slot(loc, "Use-quickly", ops::LabelMatch::Folded)
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
                        // The opener's click (a chest lid) does not open the bank.
                        self.opened_from = first_target.is_none().then(|| open_generation(cx));
                        self.reopen_pending = false;
                        self.phase = Phase::AwaitOpen { first_target };
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
                    let booth = ops::booth_nearest_action(
                        locs.value,
                        bank.tile.level,
                        wanted.map_or("Use-quickly", |wanted| wanted.op),
                    );
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
                    self.reopen_pending = false;
                    self.phase = Phase::AwaitOpen { first_target: None };
                    return Poll::Pending;
                }
                Phase::NpcAccess {
                    mut core,
                    dialogue,
                    first_target,
                } => {
                    let Some(access) = self.stand_access().cloned() else {
                        unreachable!("bank dialogue needs a stand access");
                    };
                    let intent = match dialogue {
                        AccessDialogue::Teller => npc_intent(&access)?,
                        AccessDialogue::Object => npc::Intent {
                            name: "",
                            op: npc::NpcOp::Index(-1),
                            choose: "",
                        },
                    };
                    let deadline = self.deadline;
                    let mut npc_cx = NativeNpcContext {
                        cx,
                        deadline: &mut self.deadline,
                        sent: false,
                    };
                    let polled = crate::native::defer_budget(core.step(intent, &mut npc_cx));
                    // A request that the teller core actually emitted is this
                    // machine's open attempt. A wait without a banker or a
                    // budget denial sends nothing and records nothing. An
                    // object's dialogue follows a click that already recorded
                    // (or, for a chest lid, must not record) the attempt.
                    if npc_cx.sent
                        && dialogue == AccessDialogue::Teller
                        && self.opened_from.is_none()
                    {
                        self.opened_from = Some(open_generation(cx));
                    }
                    let step = match polled {
                        Poll::Pending => {
                            self.deadline = deadline;
                            return Poll::Pending;
                        }
                        Poll::Ready(result) => result?,
                    };
                    match step {
                        npc::Step::Wait => {
                            self.phase = Phase::NpcAccess {
                                core,
                                dialogue,
                                first_target,
                            };
                            return Poll::Pending;
                        }
                        npc::Step::Note(npc::Note::Unloaded) => {
                            if cx
                                .snapshot()
                                .bank_session()
                                .is_some_and(|session| session.value.open)
                            {
                                if dialogue == AccessDialogue::Object {
                                    self.object_dialogue = ObjectDialogue::Finished;
                                }
                                self.phase = Phase::AwaitOpen { first_target };
                                continue;
                            }
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
                            self.phase = Phase::NpcAccess {
                                core,
                                dialogue,
                                first_target,
                            };
                            continue;
                        }
                        npc::Step::Done(false) => {
                            if cx
                                .snapshot()
                                .bank_session()
                                .is_some_and(|session| session.value.open)
                            {
                                if dialogue == AccessDialogue::Object {
                                    self.object_dialogue = ObjectDialogue::Finished;
                                }
                                self.phase = Phase::AwaitOpen { first_target };
                                continue;
                            }
                            if dialogue == AccessDialogue::Object {
                                self.arm_deadline(
                                    cx.evidence().tick,
                                    ops::TRANSFER_OBSERVED_TICK_LIMIT,
                                );
                                self.object_dialogue = ObjectDialogue::Finished;
                                self.phase = Phase::AwaitOpen { first_target };
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
                                self.phase = Phase::AwaitOpen { first_target };
                                continue;
                            }
                            self.session = session.value.generation;
                            self.clear_wait_deadlines();
                            self.phase = Phase::Act;
                            continue;
                        }
                    }
                }
                Phase::AwaitOpen { first_target } => {
                    if cx.snapshot().bank().is_some() {
                        self.session = cx
                            .snapshot()
                            .bank_session()
                            .map_or(0, |session| session.value.generation);
                        self.clear_wait_deadlines();
                        self.phase = Phase::Act;
                        continue;
                    }
                    let open_wait_expired = self.deadline_reached(cx.evidence().tick);
                    if open_wait_expired {
                        self.session_lost(cx, "bank item table unavailable")?;
                        continue;
                    }
                    let stand = self.stand_access().cloned();
                    let object_access = stand.as_ref().is_some_and(|access| {
                        access.kind == AccessKind::Booth
                            && access.bank.definition.is_some_and(|definition| {
                                definition.open_first.or(definition.object).is_some()
                            })
                    });
                    let chat_open = cx.snapshot().chat_modal().is_some_and(|chat| {
                        chat.value.root != -1 || chat.value.continue_component_id >= 0
                    });
                    if self.object_dialogue == ObjectDialogue::Awaiting
                        && object_access
                        && chat_open
                    {
                        self.deadline = None;
                        self.phase = Phase::NpcAccess {
                            core: npc::NpcAccess::new(),
                            dialogue: AccessDialogue::Object,
                            first_target,
                        };
                        continue;
                    }
                    if let Some(access) = stand.as_ref() {
                        if self.open_stage == 0 {
                            if let Some(open_first) = access
                                .bank
                                .definition
                                .and_then(|definition| definition.open_first)
                            {
                                if cx.snapshot().locs().is_none() {
                                    return Poll::Pending;
                                }
                                if let Some((id, tile)) = first_target {
                                    let still_closed =
                                        catalog_object_at(cx, id, tile, open_first).is_some();
                                    if !still_closed {
                                        self.open_stage = 1;
                                        self.phase = Phase::Open;
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                    return Poll::Pending;
                }
                Phase::AwaitTable => {
                    if cx.snapshot().bank().is_some() {
                        self.session = cx
                            .snapshot()
                            .bank_session()
                            .map_or(0, |session| session.value.generation);
                        self.clear_wait_deadlines();
                        self.phase = Phase::Act;
                        continue;
                    }
                    // The table and the open arrive in one server flush, the
                    // table first (`bank.rs2:21-23`, `World.ts:1116-1123`): an
                    // open whose session is gone again is a closed bank, not
                    // a slow table. After this request's own stand click the
                    // close is the engine's close-then-open on that click
                    // (`OpLocHandler.ts:53`, `Player.ts:760-814`), so the open
                    // wait resumes on its bound; a transfer that sent no click
                    // lost its session and re-opens.
                    if !bank_open {
                        if self.opened_from.is_some() {
                            self.phase = Phase::AwaitOpen { first_target: None };
                            return Poll::Pending;
                        }
                        // No access of this request is behind the gone modal
                        // (a transfer, or an open that adopted a modal left
                        // by an earlier session): access the stand now.
                        self.session_lost(cx, "bank closed before the item table loaded")?;
                        continue;
                    }
                    if self.deadline_reached(cx.evidence().tick) {
                        // A banker's dialogue is not driven while a bank modal
                        // is up (`bank/npc.rs:122`), so a teller open that
                        // adopted a modal cannot access its stand again until
                        // the modal is gone: it stays terminal for its caller.
                        let teller = self
                            .stand_access()
                            .is_some_and(|access| access.kind == AccessKind::Teller);
                        if teller && self.adopted_open_without_access() {
                            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                "bank item table unavailable",
                            ))));
                        }
                        self.session_lost(cx, "bank item table unavailable")?;
                        continue;
                    }
                    return Poll::Pending;
                }
                Phase::AwaitNoteMode {
                    request_id,
                    evidence,
                } => {
                    let current_evidence = cx.evidence();
                    let timed_out = ops::observed_wait_expired(
                        evidence.tick,
                        current_evidence.tick,
                        ops::TRANSFER_OBSERVED_TICK_LIMIT,
                    );
                    if current_evidence.run != evidence.run
                        || current_evidence.sequence <= evidence.sequence
                    {
                        if timed_out {
                            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                "bank item mode dispatch did not settle",
                            ))));
                        }
                        return Poll::Pending;
                    }
                    let Some(receipt) = cx.interaction_receipt(request_id) else {
                        if timed_out {
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
                    if !self.same_session(cx) {
                        self.session_lost(cx, "bank closed before item mode settled")?;
                        continue;
                    }
                    self.item_mode_ensured = true;
                    cx.note_bank_item_mode(self.session);
                    self.clear_wait_deadlines();
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
                                self.session_lost(cx, "bank closed during withdraw")?;
                                continue;
                            }
                            Progress::Incomplete | Progress::PackFull | Progress::OverTarget => {}
                        }
                        let reason =
                            match std::task::ready!(self.click_withdraw(cx, &goal, view.held))? {
                                Clicked::Sent => return Poll::Pending,
                                Clicked::Closed => {
                                    self.session_lost(cx, "bank closed during withdraw")?;
                                    continue;
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
                            self.session_lost(cx, "bank closed during tier withdraw")?;
                            continue;
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
                                self.session_lost(cx, "bank closed during tier withdraw")?;
                                continue;
                            }
                            Clicked::NoStock | Clicked::NoOp => {
                                return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                    "bank item has no compatible withdraw action",
                                ))));
                            }
                        }
                    }
                    BankAction::WithdrawTo { withdrawals } => {
                        let Some(withdrawal) = withdrawals.get(usize::from(self.withdraw_index))
                        else {
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
                                self.withdraw_index = self.withdraw_index.saturating_add(1);
                                continue;
                            }
                            Progress::OverTarget => {
                                return Poll::Ready(Ok(self.receipt(false)));
                            }
                            Progress::SessionGone => {
                                self.session_lost(cx, "bank closed during exact withdraw")?;
                                continue;
                            }
                            Progress::Incomplete | Progress::PackFull => {}
                        }
                        match std::task::ready!(self.click_withdraw(cx, &goal, view.held))? {
                            Clicked::Sent => return Poll::Pending,
                            Clicked::Closed => {
                                self.session_lost(cx, "bank closed during exact withdraw")?;
                                continue;
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
                        let snapshot = cx.snapshot();
                        let pack = snapshot.inventory().map(|rows| rows.value);
                        if matches!(
                            self.request.action,
                            BankAction::DepositProducts { .. } | BankAction::DepositCapacity { .. }
                        ) {
                            match self.protected_unchanged(pack) {
                                Some(true) => {}
                                Some(false) => {
                                    return Poll::Ready(Err(ActionError::Blocked(Arc::from(
                                        "protected inventory changed during bank deposit",
                                    ))));
                                }
                                None => return Poll::Pending,
                            }
                        }
                        // A deposit the lost session applied is credited
                        // from the new session's pack before this session
                        // chooses a click: the owed capacity delta and the
                        // deposited tally continue, never restart.
                        if self.in_flight.is_some() {
                            let Some(pack) = pack else {
                                return Poll::Pending;
                            };
                            self.reconcile_in_flight(cx, pack);
                        }
                        let side = snapshot.bank_side().map(|rows| rows.value);
                        if let BankAction::DepositCapacity { slots, .. } = &self.request.action {
                            if self.capacity_freed >= *slots {
                                return Poll::Ready(Ok(self.receipt(true)));
                            }
                        }
                        let view_wait_expired =
                            self.view_armed && self.deadline_reached(cx.evidence().tick);
                        let scan = if let BankAction::DepositCapacity {
                            keep, extra_keep, ..
                        } = &self.request.action
                        {
                            ops::deposit_capacity_next(
                                keep,
                                *extra_keep,
                                side,
                                pack,
                                self.same_session(cx),
                            )
                        } else {
                            let spec = deposit_spec(&self.request.action)
                                .expect("deposit actions have a deposit spec");
                            ops::deposit_next(
                                &spec,
                                side,
                                pack,
                                view_wait_expired,
                                self.same_session(cx),
                            )
                        };
                        match scan {
                            DepositScan::WaitView => {
                                if pack.is_some()
                                    && matches!(self.request.action, BankAction::DepositAll { .. })
                                {
                                    if !self.view_armed {
                                        self.view_armed = true;
                                        self.arm_deadline(cx.evidence().tick, SIDE_VIEW_TICKS);
                                    }
                                } else {
                                    if self.deadline.is_none() {
                                        self.arm_deadline(cx.evidence().tick, SIDE_VIEW_TICKS);
                                    }
                                    if self.deadline_reached(cx.evidence().tick) {
                                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                            "bank deposit view did not post",
                                        ))));
                                    }
                                }
                                return Poll::Pending;
                            }
                            DepositScan::ViewExpired => {
                                return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                    "bank deposit view did not post",
                                ))));
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
                                self.session_lost(cx, "bank closed during deposit")?;
                                continue;
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
                                let Some(before) = pack.map(|rows| ops::count_id(rows, click.id))
                                else {
                                    self.phase = Phase::Act;
                                    return Poll::Pending;
                                };
                                if before <= 0 {
                                    self.phase = Phase::Act;
                                    return Poll::Pending;
                                }
                                // Content rejects any positive addition to a
                                // saturated stack (`bank.rs2:88-92`). Other
                                // capacity refusals use its post-send message.
                                if snapshot.bank().is_some_and(|rows| {
                                    ops::count_id(rows.value, click.id) == i32::MAX
                                }) {
                                    return Poll::Ready(Err(ActionError::BankDeposit(
                                        ops::DepositFailure::FullStack,
                                    )));
                                }
                                let before_slots = matches!(
                                    self.request.action,
                                    BankAction::DepositCapacity { .. }
                                )
                                .then(|| pack.map_or(0, pack_rows));
                                let option = side
                                    .and_then(|side| deposit_option(side, click))
                                    .expect("a selected deposit has its posted operation label");
                                let trace = Arc::new(BankDepositTrace {
                                    generation: self.session,
                                    transfer: self.deposits + 1,
                                    click_tick: cx.evidence().tick,
                                    sequence: cx.evidence().sequence,
                                    id: click.id,
                                    slot: click.slot,
                                    component: click.component,
                                    option: Arc::from(option),
                                    operation: click.operation,
                                    main_before: before,
                                    side_before: side.map(|side| ops::count_id(side, click.id)),
                                    bank_before: snapshot
                                        .bank()
                                        .map(|rows| ops::count_id(rows.value, click.id)),
                                    main_revision: snapshot.inventory_packet_generation(),
                                    side_revision: snapshot.bank_side_packet_generation(),
                                });
                                let request = std::task::ready!(crate::native::defer_budget(
                                    cx.emit_bank_deposit(
                                        ops::deposit_req(click, self.session),
                                        &trace,
                                    )
                                ))?;
                                api::host_log!(
                                    api::hostlog::Category::BankOp,
                                    api::hostlog::Level::Debug,
                                    "bank-deposit-click run={:?} bank_generation={} transfer={} request={} click_tick={} sequence={} id={} slot={} component={} option={} operation={} main_before={} side_before={:?} bank_before={:?} main_revision={:?} side_revision={:?}",
                                    cx.evidence().run,
                                    trace.generation,
                                    trace.transfer,
                                    request,
                                    trace.click_tick,
                                    trace.sequence,
                                    trace.id,
                                    trace.slot,
                                    trace.component,
                                    trace.option,
                                    trace.operation,
                                    trace.main_before,
                                    trace.side_before,
                                    trace.bank_before,
                                    trace.main_revision,
                                    trace.side_revision,
                                );
                                self.deposit_log = Some(Box::new(DepositLogState {
                                    request,
                                    last_observation: None,
                                    last_logged_tick: trace.click_tick,
                                    trace: Arc::clone(&trace),
                                    dispatch_evidence: None,
                                    dispatch_accepted: None,
                                    chat_since: None,
                                }));
                                self.view_armed = false;
                                self.deadline = None;
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
                Phase::ReconcileDeposit { evidence, reason } => {
                    if !self.same_session(cx) {
                        if self.interrupted {
                            return Poll::Ready(Err(ActionError::Cancelled));
                        }
                        self.session_lost(cx, "bank closed before the transfer settled")?;
                        continue;
                    }
                    // An increased bank or decreased side count without the
                    // corresponding held-count fall is an applied transfer
                    // whose inventory packet is still delayed. This also
                    // covers notes, whose bank destination has a different id.
                    // Retain the existing transfer wait bound; never replay it.
                    let (bank_before, side_before) =
                        self.deposit_log.as_ref().map_or((None, None), |state| {
                            (state.trace.bank_before, state.trace.side_before)
                        });
                    let transfer_ahead = self.in_flight.is_some_and(|flight| {
                        let bank_ahead = bank_before.is_some_and(|before| {
                            snapshot.bank().is_some_and(|rows| {
                                ops::count_id(rows.value, flight.item_id) > before
                            })
                        });
                        let side_ahead = side_before.is_some_and(|before| {
                            snapshot.bank_side().is_some_and(|rows| {
                                ops::count_id(rows.value, flight.item_id) < before
                            })
                        });
                        (bank_ahead || side_ahead)
                            && snapshot.inventory().is_some_and(|rows| {
                                ops::count_id(rows.value, flight.item_id) >= flight.before
                            })
                    });
                    // Reconcile the old transfer once in Act, before rescanning.
                    if cx.evidence().run == evidence.run
                        && cx.evidence().sequence > evidence.sequence
                        && snapshot.inventory().is_some()
                        && snapshot.bank().is_some()
                        && !transfer_ahead
                    {
                        self.deposit_log = None;
                        self.phase = Phase::Act;
                        continue;
                    }
                    if ops::observed_wait_expired(
                        evidence.tick,
                        cx.evidence().tick,
                        ops::TRANSFER_OBSERVED_TICK_LIMIT,
                    ) {
                        return Poll::Ready(Err(if self.interrupted {
                            ActionError::Cancelled
                        } else {
                            ActionError::BankDeposit(reason)
                        }));
                    }
                    return Poll::Pending;
                }
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
                    if self.deposit_log.is_some() {
                        match self.await_deposit_transfer(
                            cx,
                            before,
                            before_slots,
                            item_id,
                            evidence,
                        ) {
                            Ok(TransferWait::Settled) => {
                                self.phase = Phase::Act;
                                continue;
                            }
                            Ok(TransferWait::Pending) => return Poll::Pending,
                            Ok(TransferWait::Retry(reason)) => {
                                if self.deposit_recovered {
                                    return Poll::Ready(Err(ActionError::BankDeposit(reason)));
                                }
                                self.deposit_recovered = true;
                                self.in_flight = Some(InFlightDeposit {
                                    before,
                                    before_slots,
                                    item_id,
                                });
                                api::host_log!(
                                    api::hostlog::Category::BankOp,
                                    api::hostlog::Level::Info,
                                    "bank-deposit-recover run={:?} tick={} id={} reason={reason} recovery=1/1",
                                    cx.evidence().run,
                                    cx.evidence().tick,
                                    item_id,
                                );
                                if let Some(state) = self.deposit_log.as_ref() {
                                    cx.cancel_request(state.request);
                                }
                                self.phase = Phase::ReconcileDeposit {
                                    evidence: cx.evidence(),
                                    reason,
                                };
                                return Poll::Pending;
                            }
                            Ok(TransferWait::SessionLost) => {
                                self.in_flight = Some(InFlightDeposit {
                                    before,
                                    before_slots,
                                    item_id,
                                });
                                if self.interrupted {
                                    return Poll::Ready(Err(ActionError::Cancelled));
                                }
                                self.session_lost(cx, "bank closed before the transfer settled")?;
                                continue;
                            }
                            Err(error) => return Poll::Ready(Err(error)),
                        }
                    }
                    let snapshot = cx.snapshot();
                    let pack = snapshot.inventory().map(|rows| rows.value);
                    let same_session = cx.evidence().run == evidence.run
                        && snapshot.bank_session().is_some_and(|session| {
                            session.value.open && session.value.generation == self.session
                        });
                    let withdrawing = matches!(
                        self.request.action,
                        BankAction::Withdraw { .. }
                            | BankAction::WithdrawAny { .. }
                            | BankAction::WithdrawTo { .. }
                    );
                    if !same_session {
                        // Withdrawals are held targets: the new session reads
                        // the held count again and owes only the remainder.
                        if !withdrawing {
                            self.in_flight = Some(InFlightDeposit {
                                before,
                                before_slots,
                                item_id,
                            });
                        }
                        if self.interrupted {
                            return Poll::Ready(Err(ActionError::Cancelled));
                        }
                        self.session_lost(cx, "bank closed before the transfer settled")?;
                        continue;
                    }
                    if matches!(
                        self.request.action,
                        BankAction::DepositProducts { .. } | BankAction::DepositCapacity { .. }
                    ) {
                        match self.protected_unchanged(pack) {
                            Some(true) => {}
                            Some(false) if self.interrupted => {
                                return Poll::Ready(Err(ActionError::Cancelled));
                            }
                            Some(false) => {
                                return Poll::Ready(Err(ActionError::Blocked(Arc::from(
                                    "protected inventory changed during bank deposit",
                                ))));
                            }
                            None => return Poll::Pending,
                        }
                    }
                    // The clicked id's held count: a withdraw raises it, a
                    // deposit lowers it.
                    let current = pack.map(|rows| ops::count_id(rows, item_id));
                    let moved = if withdrawing {
                        current.is_some_and(|now| now > before)
                    } else {
                        current.is_some_and(|now| now < before)
                    };
                    if moved {
                        if !withdrawing {
                            let after = current.unwrap_or(before);
                            self.deposited = self
                                .deposited
                                .saturating_add(before.saturating_sub(after).max(0) as u32);
                        }
                        if let (Some(before_slots), Some(pack)) = (before_slots, pack) {
                            let freed = before_slots.saturating_sub(pack_rows(pack));
                            self.capacity_freed = self.capacity_freed.saturating_add(freed);
                        }
                        self.phase = Phase::Act;
                        continue;
                    }
                    if ops::observed_wait_expired(
                        evidence.tick,
                        cx.evidence().tick,
                        ops::TRANSFER_OBSERVED_TICK_LIMIT,
                    ) {
                        return Poll::Ready(Err(if self.interrupted {
                            ActionError::Cancelled
                        } else {
                            ActionError::Failed(Arc::from("bank transfer did not settle"))
                        }));
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
                        self.deadline_reached(cx.evidence().tick),
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
    fn interrupt(&mut self) {
        BankMachine::interrupt(self);
    }
}

struct NativeNpcContext<'a, 'frame> {
    cx: &'a mut ActionContext<'frame>,
    deadline: &'a mut Option<BankDeadline>,
    /// The core emitted a request in this step.
    sent: bool,
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
        let (banker, action) = npc::banker_scan(npcs.value, name, op)?;
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
        let ticks = ops::observed_ticks_for_millis(millis);
        *self.deadline = Some(BankDeadline::after(self.cx.evidence().tick, ticks));
    }

    fn expired(&mut self) -> bool {
        (*self.deadline).is_some_and(|deadline| deadline.reached(self.cx.evidence().tick))
    }
    fn emit(&mut self, request: InteractReq) -> Result<(), Self::Error> {
        self.cx.emit(request)?;
        self.sent = true;
        Ok(())
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

/// What one deposit transfer wait observed this frame.
enum TransferWait {
    /// The held count fell: the click settled.
    Settled,
    /// Nothing terminal yet.
    Pending,
    /// The session closed or was replaced before the count fell.
    SessionLost,
    /// A refused or no-op dispatch needs a new observation and a fresh scan.
    Retry(ops::DepositFailure),
}

/// The bank session generation now, or `u64::MAX` before any session.
fn open_generation(cx: &ActionContext<'_>) -> u64 {
    cx.snapshot()
        .bank_session()
        .map_or(u64::MAX, |session| session.value.generation)
}

/// Occupied pack rows; the pack holds at most 28, so the count fits a byte.
fn pack_rows<R: ops::BankRow>(pack: &[R]) -> u8 {
    u8::try_from(pack.iter().filter(|row| row.count() > 0).count()).unwrap_or(u8::MAX)
}

impl BankMachine {
    fn arm_deadline(&mut self, start_tick: u64, tick_limit: u64) {
        self.deadline = Some(BankDeadline::after(start_tick, tick_limit));
    }

    fn deadline_reached(&self, tick: u64) -> bool {
        self.deadline.is_some_and(|deadline| deadline.reached(tick))
    }

    fn clear_wait_deadlines(&mut self) {
        self.deadline = None;
    }

    /// The stand a transfer verb can re-open its session from: an open
    /// request's own access, else the access its caller recorded.
    fn stand_access(&self) -> Option<&Arc<BankStandAccess>> {
        match &self.request.action {
            BankAction::OpenStand { access } => Some(access),
            _ => self.request.access.as_ref(),
        }
    }

    /// An open request that adopted an already-open modal and has sent no
    /// stand access of its own (every access it emits sets `opened_from`,
    /// except the opener's click on a chest lid, which is its own access and
    /// is waiting for the lid to open). That modal belongs to an earlier
    /// session or to another request, so its missing table says nothing about
    /// this request's access, and waiting on it again cannot recover: a soft
    /// reconnect (login response 15) keeps the client's modal fields but drops
    /// the item table (`client.rs:3735-3758`, `reset_packet_observations`).
    fn adopted_open_without_access(&self) -> bool {
        matches!(self.request.action, BankAction::OpenStand { .. })
            && self.opened_from.is_none()
            && !matches!(
                self.phase,
                Phase::AwaitOpen {
                    first_target: Some(_)
                }
            )
    }

    /// The open session is gone (closed, or replaced by a newer one) before
    /// this request settled. The engine closes the bank on any further
    /// client action packet or a strong-queued script (`Player.ts:760-814`,
    /// `:970-973`) and sends nothing a verb could resume on, so a transfer
    /// that knows its stand clicks it again and resumes from the live pack
    /// and table, bounded by [`MAX_BANK_REOPENS`]. Every verb reads its held
    /// counts again after the reopen: exact targets and product sweeps keep
    /// their meaning. A deposit click still in flight is kept as
    /// [`InFlightDeposit`] and credited from the new session's pack
    /// ([`Self::reconcile_in_flight`]) before any further click. An open
    /// request that sent its own access keeps its own semantics (its caller
    /// retries the stand); one that adopted a modal without sending an access
    /// clicks its stand once. That click sets `opened_from`, which bounds it.
    fn session_lost(
        &mut self,
        cx: &mut ActionContext<'_>,
        reason: &'static str,
    ) -> Result<(), ActionError> {
        let transfer = matches!(
            self.request.action,
            BankAction::Withdraw { .. }
                | BankAction::WithdrawAny { .. }
                | BankAction::WithdrawTo { .. }
                | BankAction::Deposit { .. }
                | BankAction::DepositAll { .. }
                | BankAction::DepositProducts { .. }
                | BankAction::DepositCapacity { .. }
        );
        let stand = if transfer || self.adopted_open_without_access() {
            self.stand_access().cloned()
        } else {
            None
        };
        let Some(stand) = stand else {
            return Err(ActionError::Failed(Arc::from(reason)));
        };
        if self.reopens >= MAX_BANK_REOPENS {
            api::host_log!(
                api::hostlog::Category::BankOp,
                api::hostlog::Level::Info,
                "bank-reopen run={:?} tick={} reason={reason} reopens={} limit={} result=exhausted",
                cx.evidence().run,
                cx.evidence().tick,
                self.reopens,
                MAX_BANK_REOPENS,
            );
            return Err(ActionError::Failed(Arc::from(reason)));
        }
        self.reopens = self.reopens.saturating_add(1);
        api::host_log!(
            api::hostlog::Category::BankOp,
            api::hostlog::Level::Info,
            "bank-reopen run={:?} tick={} reason={reason} reopen={}/{} bank_generation={} stand={},{},{} kind={} deposited={}",
            cx.evidence().run,
            cx.evidence().tick,
            self.reopens,
            MAX_BANK_REOPENS,
            self.session,
            stand.stand_tile.x,
            stand.stand_tile.z,
            stand.stand_tile.level,
            stand.kind.as_str(),
            self.deposited,
        );
        // Per-session state only; the request, its protected counts, the
        // deposited tally and the in-flight deposit belong to the whole
        // transfer.
        self.session = 0;
        self.deposit_log = None;
        self.view_armed = false;
        self.item_mode_ensured = false;
        self.open_stage = 0;
        self.object_dialogue = ObjectDialogue::NotSent;
        self.opened_from = None;
        self.reopen_pending = true;
        self.withdraw_index = 0;
        self.arm_deadline(cx.evidence().tick, ops::TRANSFER_OBSERVED_TICK_LIMIT);
        self.phase = Phase::Open;
        Ok(())
    }

    /// Credit what the lost session applied to the deposit that was in
    /// flight, exactly once, from the first pack of the new session: the
    /// clicked id's count fall is the deposited amount (the deposit's
    /// `UPDATE_INV` is flushed in the tick that applied it, before any later
    /// open, `World.ts:1116-1123`), and for a capacity deposit the pack's
    /// row count fall is the freed delta already delivered. A count that
    /// did not fall was not applied and the sweep resumes with nothing
    /// credited.
    fn reconcile_in_flight<R: ops::BankRow>(&mut self, cx: &ActionContext<'_>, pack: &[R]) {
        let Some(in_flight) = self.in_flight.take() else {
            return;
        };
        let after = ops::count_id(pack, in_flight.item_id);
        let fell = in_flight.before.saturating_sub(after).max(0);
        let freed = in_flight.before_slots.map_or(0, |before_slots| {
            before_slots.saturating_sub(pack_rows(pack))
        });
        if fell > 0 {
            self.deposited = self.deposited.saturating_add(fell as u32);
            self.capacity_freed = self.capacity_freed.saturating_add(freed);
            self.deposits = self.deposits.saturating_add(1);
        }
        api::host_log!(
            api::hostlog::Category::BankOp,
            api::hostlog::Level::Info,
            "bank-reopen-reconcile run={:?} tick={} bank_generation={} id={} before={} after={} credited={} freed={} deposited={} capacity_freed={}",
            cx.evidence().run,
            cx.evidence().tick,
            self.session,
            in_flight.item_id,
            in_flight.before,
            after,
            fell,
            freed,
            self.deposited,
            self.capacity_freed,
        );
    }
    fn await_deposit_transfer(
        &mut self,
        cx: &mut ActionContext<'_>,
        before: i32,
        before_slots: Option<u8>,
        item_id: i32,
        evidence: EvidenceStamp,
    ) -> Result<TransferWait, ActionError> {
        let (dispatched, accepted, chat_since) = {
            let Some(state) = self.deposit_log.as_mut() else {
                return Ok(TransferWait::Pending);
            };
            if state.dispatch_evidence.is_none() {
                if let Some(receipt) = cx.interaction_receipt(state.request).copied() {
                    state.dispatch_evidence = Some(receipt.evidence);
                    state.dispatch_accepted = Some(
                        receipt.accepted
                            && receipt.evidence.run == evidence.run
                            && receipt.evidence.sequence >= evidence.sequence,
                    );
                    state.chat_since = Some(receipt.chat_since);
                }
            }
            (
                state.dispatch_evidence.unwrap_or(evidence),
                state.dispatch_accepted,
                state.chat_since,
            )
        };
        let stamp = cx.evidence();
        let snapshot = cx.snapshot();
        let pack = snapshot.inventory().map(|rows| rows.value);
        let main_now = pack.map(|rows| ops::count_id(rows, item_id));
        let side_now = snapshot
            .bank_side()
            .map(|rows| ops::count_id(rows.value, item_id));
        let bank_now = snapshot
            .bank()
            .map(|rows| ops::count_id(rows.value, item_id));
        let main_revision = snapshot.inventory_packet_generation();
        let side_revision = snapshot.bank_side_packet_generation();
        let observation = DepositObservationKey {
            main_now,
            side_now,
            bank_now,
            main_revision,
            side_revision,
        };
        let same_session = stamp.run == evidence.run
            && snapshot.bank_session().is_some_and(|session| {
                session.value.open && session.value.generation == self.session
            });
        let protected = if matches!(
            self.request.action,
            BankAction::DepositProducts { .. } | BankAction::DepositCapacity { .. }
        ) {
            self.protected_unchanged(pack)
        } else {
            Some(true)
        };
        let moved = main_now.is_some_and(|now| now < before);
        let timed_out = ops::observed_wait_expired(
            dispatched.tick,
            stamp.tick,
            ops::TRANSFER_OBSERVED_TICK_LIMIT,
        );
        let click_main_revision = self
            .deposit_log
            .as_ref()
            .expect("deposit transfer carries its click trace")
            .trace
            .main_revision;
        let main_revision_advanced =
            click_main_revision.is_some_and(|click| main_revision.is_some_and(|now| now > click));
        let content_refusal = if accepted == Some(true) {
            chat_since.and_then(|since| {
                snapshot
                    .chat_lines(since)?
                    .value
                    .iter()
                    .filter(|line| line.type_ == 0 && line.username.is_none())
                    .find_map(|line| ops::deposit_content_refusal(&line.text))
            })
        } else {
            None
        };
        let decision = if !same_session {
            "session-gone"
        } else if protected == Some(false) {
            "protected-inventory-changed"
        } else if protected == Some(true) && moved {
            "settled-count-fell"
        } else if content_refusal.is_some() {
            "content-refused"
        } else if accepted == Some(false) {
            "dispatch-refused"
        } else if timed_out {
            "timeout-no-count-fall"
        } else if protected.is_none() || !main_revision_advanced {
            "pending-stale-evidence"
        } else {
            "pending-count-unchanged"
        };
        let terminal = !same_session
            || protected == Some(false)
            || timed_out
            || content_refusal.is_some()
            || accepted == Some(false)
            || (protected == Some(true) && moved);

        self.log_deposit_observation(stamp, observation, same_session, decision, terminal);
        if !same_session {
            self.log_deposit_result(
                stamp,
                observation,
                "aborted",
                "session-gone",
                api::hostlog::Level::Debug,
            );
            return Ok(TransferWait::SessionLost);
        }
        if protected == Some(false) {
            self.log_deposit_result(
                stamp,
                observation,
                "blocked",
                "protected-inventory-changed",
                api::hostlog::Level::Debug,
            );
            return Err(ActionError::Blocked(Arc::from(
                "protected inventory changed during bank deposit",
            )));
        }
        if protected == Some(true) && moved {
            self.log_deposit_result(
                stamp,
                observation,
                "settled",
                "main-count-fell",
                api::hostlog::Level::Debug,
            );
            let after = main_now.unwrap_or(before);
            self.deposited = self
                .deposited
                .saturating_add(before.saturating_sub(after).max(0) as u32);
            if let (Some(before_slots), Some(pack)) = (before_slots, pack) {
                let freed = before_slots.saturating_sub(pack_rows(pack));
                self.capacity_freed = self.capacity_freed.saturating_add(freed);
            }
            self.deposits = self.deposits.saturating_add(1);
            self.deposit_log = None;
            return Ok(TransferWait::Settled);
        }
        if let Some(reason) = content_refusal {
            self.log_deposit_result(
                stamp,
                observation,
                "blocked",
                "content-refused",
                api::hostlog::Level::Info,
            );
            return Err(ActionError::BankDeposit(reason));
        }
        if accepted == Some(false) {
            self.log_deposit_result(
                stamp,
                observation,
                "recover",
                "dispatch-refused",
                api::hostlog::Level::Info,
            );
            return Ok(TransferWait::Retry(ops::DepositFailure::DispatchRefused));
        }
        if timed_out {
            self.log_deposit_result(
                stamp,
                observation,
                "timeout",
                "no-main-count-fall",
                api::hostlog::Level::Info,
            );
            return Ok(TransferWait::Retry(if accepted.is_none() {
                ops::DepositFailure::DispatchMissing
            } else {
                ops::DepositFailure::NoCountFall
            }));
        }
        Ok(TransferWait::Pending)
    }

    fn log_deposit_observation(
        &mut self,
        stamp: EvidenceStamp,
        observation: DepositObservationKey,
        same_session: bool,
        decision: &str,
        terminal: bool,
    ) {
        let Some(state) = self.deposit_log.as_mut() else {
            return;
        };
        let fresh = stamp.tick > state.last_logged_tick;
        if !terminal && !fresh && state.last_observation == Some(observation) {
            return;
        }
        let trace = &state.trace;
        api::host_log!(
            api::hostlog::Category::BankOp,
            api::hostlog::Level::Debug,
            "bank-deposit-observe run={:?} bank_generation={} transfer={} request={} click_tick={} tick={} sequence={} fresh={} id={} main_before={} main_now={:?} side_now={:?} bank_now={:?} main_revision={:?} side_revision={:?} same_session={} elapsed_observed_ticks={} decision={}",
            stamp.run,
            trace.generation,
            trace.transfer,
            state.request,
            trace.click_tick,
            stamp.tick,
            stamp.sequence,
            fresh,
            trace.id,
            trace.main_before,
            observation.main_now,
            observation.side_now,
            observation.bank_now,
            observation.main_revision,
            observation.side_revision,
            same_session,
            stamp.tick.saturating_sub(trace.click_tick),
            decision,
        );
        state.last_observation = Some(observation);
        state.last_logged_tick = stamp.tick;
    }

    fn log_deposit_result(
        &self,
        stamp: EvidenceStamp,
        observation: DepositObservationKey,
        result: &str,
        reason: &str,
        level: api::hostlog::Level,
    ) {
        let Some(state) = self.deposit_log.as_ref() else {
            return;
        };
        let trace = &state.trace;
        api::host_log!(
            api::hostlog::Category::BankOp,
            level,
            "bank-deposit-result run={:?} bank_generation={} transfer={} request={} click_tick={} tick={} id={} slot={} component={} option={} operation={} main_before={} main_now={:?} side_now={:?} bank_now={:?} elapsed_observed_ticks={} result={} reason={}",
            stamp.run,
            trace.generation,
            trace.transfer,
            state.request,
            trace.click_tick,
            stamp.tick,
            trace.id,
            trace.slot,
            trace.component,
            trace.option,
            trace.operation,
            trace.main_before,
            observation.main_now,
            observation.side_now,
            observation.bank_now,
            stamp.tick.saturating_sub(trace.click_tick),
            result,
            reason,
        );
    }

    fn same_session(&self, cx: &ActionContext<'_>) -> bool {
        cx.snapshot()
            .bank_session()
            .is_some_and(|session| session.value.open && session.value.generation == self.session)
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
            let request_id = std::task::ready!(crate::native::defer_budget(
                cx.emit(ops::note_req(NoteIntent::Item))
            ))?;
            let evidence = cx.evidence();

            self.phase = Phase::AwaitNoteMode {
                request_id,
                evidence,
            };
            return Poll::Ready(Ok(Clicked::Sent));
        }
        std::task::ready!(crate::native::defer_budget(cx.emit(request)))?;

        self.phase = Phase::AwaitTransfer {
            before: held,
            before_slots: None,
            item_id: goal.lands_as_id,
            evidence: cx.evidence(),
        };
        Poll::Ready(Ok(Clicked::Sent))
    }

    fn protected_unchanged<R: ops::BankRow>(&self, inventory: Option<&[R]>) -> Option<bool> {
        let inventory = inventory?;
        Some(match &self.protected_before {
            ProtectedCounts::Products(before) => before
                .iter()
                .all(|before| ops::count_id(inventory, before.id) == before.count),
            ProtectedCounts::Capacity(before) => {
                let BankAction::DepositCapacity {
                    keep, extra_keep, ..
                } = &self.request.action
                else {
                    unreachable!("capacity counts belong to a capacity request");
                };
                before
                    .iter()
                    .filter(|row| row.count > 0)
                    .all(|row| ops::count_id(inventory, row.id) == row.count)
                    && inventory
                        .iter()
                        .filter(|row| {
                            row.count() > 0
                                && (keep.contains(&row.id()) || *extra_keep == Some(row.id()))
                        })
                        .all(|row| {
                            before
                                .iter()
                                .any(|prior| prior.count > 0 && prior.id == row.id())
                        })
            }
        })
    }

    pub(crate) fn interrupt(&mut self) {
        self.interrupted = true;
    }

    pub(crate) fn transfer_in_flight(&self) -> bool {
        matches!(
            self.phase,
            Phase::AwaitTransfer { .. } | Phase::ReconcileDeposit { .. }
        )
    }

    fn receipt(&self, complete: bool) -> BankReceipt {
        BankReceipt {
            complete,
            deposited: self.deposited,
        }
    }
}

pub struct Select {
    request: Option<BankPickRequest>,
    request_id: Option<u64>,
    immediate: Option<SelectedBank>,
    fallback: Option<SelectFallback>,
}

impl Select {
    pub(crate) fn prereq_allow(
        &self,
        cx: &ActionContext<'_>,
    ) -> Option<crate::native::PrereqAllow> {
        let receipt = cx.bank_pick_receipt(self.request_id?)?;
        match &receipt.outcome {
            BankPickOutcome::NeedsStep { allow, .. } => Some(*allow),
            BankPickOutcome::Selected(_) | BankPickOutcome::NoCandidate { .. } => None,
        }
    }

    pub(crate) fn no_candidate(&self, cx: &ActionContext<'_>) -> bool {
        self.fallback.is_none()
            && self
                .request_id
                .and_then(|id| cx.bank_pick_receipt(id))
                .is_some_and(|receipt| {
                    matches!(&receipt.outcome, BankPickOutcome::NoCandidate { .. })
                })
    }

    /// Keep the typed terminal until each caller applies its existing result contract.
    pub(crate) fn poll_outcome(
        &mut self,
        cx: &mut ActionContext<'_>,
    ) -> Poll<Result<BankPickOutcome, ActionError>> {
        if let Some(selected) = self.immediate.take() {
            return Poll::Ready(Ok(BankPickOutcome::Selected(selected)));
        }
        if let Some(receipt) = self.request_id.and_then(|id| cx.bank_pick_receipt(id)) {
            if let (BankPickOutcome::NoCandidate { .. }, Some(fallback)) =
                (&receipt.outcome, self.fallback.take())
            {
                return Poll::Ready(Ok(BankPickOutcome::Selected(fallback.selected)));
            }
            return Poll::Ready(Ok(receipt.outcome.clone()));
        }
        if self
            .fallback
            .as_ref()
            .and_then(|fallback| fallback.deadline)
            .is_some_and(|deadline| deadline.reached(cx.evidence().tick))
        {
            return Poll::Ready(Ok(BankPickOutcome::Selected(
                self.fallback.take().expect("checked fallback").selected,
            )));
        }
        if self.request_id.is_none() {
            let Some(request) = self.request.take() else {
                return Poll::Ready(Err(ActionError::Stale));
            };
            return match cx.bank_pick(request) {
                Ok(request_id) => {
                    self.request_id = Some(request_id);
                    // The bound counts from the dispatch, the host job's start.
                    if let Some(fallback) = self.fallback.as_mut() {
                        fallback.deadline = Some(BankDeadline::after(
                            cx.evidence().tick,
                            SELECT_FALLBACK_TICKS,
                        ));
                    }
                    Poll::Pending
                }
                Err(error) => Poll::Ready(Err(error)),
            };
        }
        Poll::Pending
    }
}

struct SelectFallback {
    selected: SelectedBank,
    /// Set when the pick request is dispatched; none before.
    deadline: Option<BankDeadline>,
}

impl Select {
    /// Begins a pick. `avoid` is a bank to leave out of the candidates
    /// (see [`SelectOtherArgs::avoid`]).
    fn start(
        args: SelectArgs,
        avoid: Option<u16>,
        cx: &mut ActionContext<'_>,
    ) -> Result<Self, ActionError> {
        let snapshot = cx.snapshot();
        let stats = snapshot.stats().map_or(&[][..], |rows| rows.value);
        let quests = snapshot.quest_statuses().map_or(&[][..], |rows| rows.value);
        let selector = BankSelector::new(Arc::clone(&args.facts), args.preferences);
        let mut eligible = selector.eligible_indices(stats, quests);
        let explicit_bank = args
            .explicit
            .as_deref()
            .and_then(|name| selector.named(name, stats, quests));
        if let Some(avoid) = avoid.filter(|_| explicit_bank.is_none()) {
            if eligible.iter().any(|&index| index != avoid) {
                eligible.retain(|&index| index != avoid);
            }
        }
        let explicit_unavailable = args.explicit.is_some() && explicit_bank.is_none();
        if args.allow_air_fallback {
            eligible.sort_by_key(|&index| {
                air_distance_squared(args.from, args.facts.banks()[usize::from(index)].air_tile())
            });
        }
        let eligible: Arc<[u16]> = Arc::from(eligible);
        if eligible.is_empty() || explicit_unavailable {
            return Ok(Self {
                request: None,
                request_id: None,
                immediate: Some(SelectedBank::no_candidate()),
                fallback: None,
            });
        }
        let fallback = if args.allow_air_fallback {
            explicit_bank
                .or_else(|| eligible.first().copied())
                .map(|bank_index| SelectFallback {
                    selected: SelectedBank {
                        bank_index,
                        access_tile: args.facts.banks()[usize::from(bank_index)].air_tile(),
                        kind: PickKind::AirFallback,
                        access: None,
                    },
                    deadline: None,
                })
        } else {
            None
        };
        Ok(Self {
            request: Some(BankPickRequest {
                facts: args.facts,
                from: args.from,
                preferences: args.preferences,
                options: args.options,
                eligible,
                explicit_bank,
                allow_air_fallback: args.allow_air_fallback,
            }),
            request_id: None,
            immediate: None,
            fallback,
        })
    }
}

impl NativeMachine for Select {
    type Args = SelectArgs;
    type Output = SelectedBank;

    fn begin(args: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        Self::start(args, None, cx)
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        self.poll_outcome(cx).map(|result| {
            result.and_then(|outcome| match outcome {
                BankPickOutcome::Selected(selected) => Ok(selected),
                BankPickOutcome::NeedsStep { gates, .. } => Err(ActionError::NeedsStep(gates)),
                BankPickOutcome::NoCandidate { detail } => Err(ActionError::Failed(detail)),
            })
        })
    }

    fn cancel(&mut self) {}
}

/// One typed off-pump service pick, without a service owner's retry policy.
///
/// The host applies the run/request fence and returns an explicit undecided
/// outcome on refusal or expiry. Owners decide whether that outcome merits a
/// retry; this machine never substitutes an air-nearest result.
pub struct SelectService {
    request: Option<ServicePickRequest>,
    request_id: Option<u64>,
    immediate: Option<ServicePickOutcome>,
}

impl NativeMachine for SelectService {
    type Args = ServicePickRequest;
    type Output = ServicePickOutcome;

    fn begin(request: Self::Args, _: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        let immediate = if request.kind != request.facts.kind() {
            Some(ServicePickOutcome::NotSettled)
        } else if request.eligible.is_empty() {
            Some(ServicePickOutcome::NoCandidate)
        } else {
            None
        };
        Ok(Self {
            request: immediate.is_none().then_some(request),
            request_id: None,
            immediate,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        if let Some(outcome) = self.immediate.take() {
            return Poll::Ready(Ok(outcome));
        }
        if let Some(receipt) = self.request_id.and_then(|id| cx.service_pick_receipt(id)) {
            return Poll::Ready(Ok(receipt.outcome.clone()));
        }
        if self.request_id.is_none() {
            let Some(request) = self.request.take() else {
                return Poll::Ready(Err(ActionError::Stale));
            };
            return match cx.service_pick(request) {
                Ok(request_id) => {
                    self.request_id = Some(request_id);
                    Poll::Pending
                }
                Err(error) => Poll::Ready(Err(error)),
            };
        }
        Poll::Pending
    }

    fn cancel(&mut self) {
        self.request = None;
        self.request_id = None;
        self.immediate = None;
    }
}

/// The arguments of a pick that leaves one bank out.
#[derive(Debug, Clone)]
pub struct SelectOtherArgs {
    pub select: SelectArgs,
    /// A bank whose access the owner could not walk to. The pick drops it
    /// from its candidates when another bank is eligible and the owner named
    /// no bank. Otherwise it keeps the bank, so the pick never ends empty
    /// because of this request.
    pub avoid: u16,
}

/// A [`Select`] that leaves out the bank whose access the owner could not
/// reach (the Gatherer's second access recovery).
pub struct SelectOther {
    pick: Select,
}

impl SelectOther {
    pub(crate) fn prereq_allow(
        &self,
        cx: &ActionContext<'_>,
    ) -> Option<crate::native::PrereqAllow> {
        self.pick.prereq_allow(cx)
    }
}

impl NativeMachine for SelectOther {
    type Args = SelectOtherArgs;
    type Output = SelectedBank;

    fn begin(args: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        Ok(Self {
            pick: Select::start(args.select, Some(args.avoid), cx)?,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        self.pick.poll(cx)
    }

    fn cancel(&mut self) {
        self.pick.cancel();
    }
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
                access: None,
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
                access: args.access,
            },
            cx,
        )?;
        Ok(Self { core })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        match self.core.poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(receipt)) => Poll::Ready(Ok(receipt.deposited)),
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
        }
    }

    fn cancel(&mut self) {
        self.core.cancel();
    }
    fn interrupt(&mut self) {
        self.core.interrupt();
    }
}

impl Deposit {
    pub(crate) fn transfer_in_flight(&self) -> bool {
        self.core.transfer_in_flight()
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
                access: args.access,
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

    fn interrupt(&mut self) {
        self.core.interrupt();
    }

    fn cancel(&mut self) {
        self.core.cancel();
    }
}

impl Withdraw {
    #[cfg(test)]
    pub(crate) fn transfer_in_flight(&self) -> bool {
        self.core.transfer_in_flight()
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
                access: None,
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
    mod capacity;

    use super::*;
    use crate::native::{ActionContext, HostEffect, InteractionReceipt};
    use crate::quester::families::tests::{
        local_player, with_tick, with_tick_reach, with_tick_snapshots,
    };
    use api::snapshot::{GameSnapshot, ItemActionFamily, ItemContainer, ItemView, SnapshotView};
    use std::time::Duration;

    fn catalog_bank(name: &str) -> NamedBank {
        let definition = api::named_banks::bank_catalog()
            .unwrap()
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

    type PickLedger = Option<Box<crate::native::ledger::Ledger>>;
    type PickHandle = crate::native::ActionHandle<Select>;

    fn select_waiter(allow_air_fallback: bool) -> (GameSnapshot, PickLedger, PickHandle) {
        let (snapshot, ledger, handle, _) = select_waiter_dispatched_at(allow_air_fallback, 1);
        (snapshot, ledger, handle)
    }

    /// A Select built at observed tick 1 whose request is dispatched by its
    /// first poll at `dispatch_tick`.
    fn select_waiter_dispatched_at(
        allow_air_fallback: bool,
        dispatch_tick: u64,
    ) -> (
        GameSnapshot,
        PickLedger,
        PickHandle,
        crate::native::HostAuthority,
    ) {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        let facts = Arc::new(NamedBankFacts::from_banks(vec![
            NamedBank::new(
                "far",
                WorldTile {
                    x: 10,
                    z: 5,
                    level: 0,
                },
            ),
            NamedBank::new(
                "first tie",
                WorldTile {
                    x: 6,
                    z: 5,
                    level: 3,
                },
            ),
            NamedBank::new(
                "second tie",
                WorldTile {
                    x: 4,
                    z: 5,
                    level: 0,
                },
            ),
        ]));
        let mut ledger = None;
        let handle = with_tick(&snapshot, &mut ledger, 1, |frame| {
            frame.cx.active_now = Duration::ZERO;
            frame
                .actions
                .begin::<Select>(
                    SelectArgs {
                        facts,
                        from: WorldTile {
                            x: 5,
                            z: 5,
                            level: 0,
                        },
                        preferences: BankPreferences {
                            use_mage_bank: false,
                            use_zanaris_bank: false,
                        },
                        options: WalkOptions::default(),
                        explicit: None,
                        allow_air_fallback,
                    },
                    &mut frame.cx,
                )
                .unwrap()
        });
        // The first poll, at its own observed tick, dispatches the request.
        with_tick(&snapshot, &mut ledger, dispatch_tick, |frame| {
            assert!(frame.actions.poll(&handle, &mut frame.cx).is_pending());
        });
        let pending = ledger.as_mut().unwrap();
        assert_eq!(pending.outbox.len(), 1);
        let HostEffect::BankPick(request) = &pending.outbox[0].effect else {
            panic!("Select must emit a real typed pick");
        };
        let expected_eligible: &[u16] = if allow_air_fallback {
            &[1, 2, 0]
        } else {
            &[0, 1, 2]
        };
        assert_eq!(
            request.eligible.as_ref(),
            expected_eligible,
            "selection applies its ranking to the eligible banks"
        );
        // Drop the real action without dispatch or receipt: the host never
        // admits it. Its authority stays for a test that answers it.
        let authority = pending.outbox[0].authority();
        pending.outbox.clear();
        (snapshot, ledger, handle, authority)
    }

    fn poll_select_waiter(
        snapshot: &GameSnapshot,
        ledger: &mut PickLedger,
        handle: &PickHandle,
        tick: u64,
        active: Duration,
    ) -> Poll<Result<SelectedBank, ActionError>> {
        with_tick(snapshot, ledger, tick, |frame| {
            frame.cx.active_now = active;
            frame.actions.poll(handle, &mut frame.cx)
        })
    }

    /// The banks a Select with these inputs lets the host choose from, and
    /// the explicit bank, as the typed pick request names them.
    fn pick_candidates(
        banks: &[(&'static str, i32)],
        explicit: Option<&str>,
        avoid: Option<u16>,
    ) -> (Vec<u16>, Option<u16>) {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        let facts = Arc::new(NamedBankFacts::from_banks(
            banks
                .iter()
                .map(|&(name, x)| NamedBank::new(name, WorldTile { x, z: 5, level: 0 }))
                .collect(),
        ));
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 1, |frame| {
            let args = SelectArgs {
                facts,
                from: WorldTile {
                    x: 5,
                    z: 5,
                    level: 0,
                },
                preferences: BankPreferences {
                    use_mage_bank: false,
                    use_zanaris_bank: false,
                },
                options: WalkOptions::default(),
                explicit: explicit.map(Arc::from),
                allow_air_fallback: false,
            };
            if let Some(avoid) = avoid {
                let handle = frame
                    .actions
                    .begin::<SelectOther>(
                        SelectOtherArgs {
                            select: args,
                            avoid,
                        },
                        &mut frame.cx,
                    )
                    .unwrap();
                assert!(frame.actions.poll(&handle, &mut frame.cx).is_pending());
            } else {
                let handle = frame.actions.begin::<Select>(args, &mut frame.cx).unwrap();
                assert!(frame.actions.poll(&handle, &mut frame.cx).is_pending());
            }
        });
        let pending = ledger.as_ref().unwrap();
        let HostEffect::BankPick(request) = &pending.outbox[0].effect else {
            panic!("Select must emit a real typed pick");
        };
        (request.eligible.to_vec(), request.explicit_bank)
    }

    /// A bank whose access walk failed is dropped from the next pick only
    /// when another bank is eligible and the owner named no bank; a named
    /// bank or the only bank stays, so the pick never ends empty because of it.
    #[test]
    fn select_avoid_drops_the_failed_bank_only_when_another_is_eligible() {
        if !api::test_support::game_data_available() {
            return;
        }
        let three = [("far", 10), ("near", 6), ("nearer", 4)];
        assert_eq!(pick_candidates(&three, None, None).0, [0, 1, 2]);
        assert_eq!(pick_candidates(&three, None, Some(1)).0, [0, 2]);
        assert_eq!(pick_candidates(&three, None, Some(0)).0, [1, 2]);
        assert_eq!(
            pick_candidates(&three, Some("far"), Some(0)),
            (vec![0, 1, 2], Some(0)),
            "a named bank is never dropped"
        );
        assert_eq!(
            pick_candidates(&[("only", 6)], None, Some(0)).0,
            [0],
            "the only bank stays"
        );
    }

    /// The waiter's request was dispatched at observed tick 1, so its
    /// fallback bound is `SELECT_FALLBACK_TICKS` (9) ticks later.
    #[test]
    fn select_only_dropped_request_settles_at_nine_observed_ticks_without_access() {
        if !api::test_support::game_data_available() {
            return;
        }
        for allow_air_fallback in [false, true] {
            let (mut snapshot, mut ledger, handle) = select_waiter(allow_air_fallback);
            snapshot.seed_tile(WorldTile {
                x: 10,
                z: 5,
                level: 0,
            });
            assert!(poll_select_waiter(
                &snapshot,
                &mut ledger,
                &handle,
                1 + SELECT_FALLBACK_TICKS - 1,
                Duration::ZERO,
            )
            .is_pending());
            let result = poll_select_waiter(
                &snapshot,
                &mut ledger,
                &handle,
                1 + SELECT_FALLBACK_TICKS,
                Duration::ZERO,
            );
            if allow_air_fallback {
                let Poll::Ready(Ok(selected)) = result else {
                    panic!("a dropped select-only request must settle at its bound: {result:?}");
                };
                assert_eq!(selected.bank_index, 1);
                assert_eq!(
                    selected.access_tile,
                    WorldTile {
                        x: 6,
                        z: 5,
                        level: 3
                    }
                );
                assert_eq!(selected.kind, PickKind::AirFallback);
                assert!(selected.access.is_none());
            } else {
                assert!(
                    result.is_pending(),
                    "native Select keeps its existing host-owned result"
                );
            }
        }
    }

    /// No clock decides the fallback: an hour of active time with too few
    /// observed ticks keeps waiting, and the bound tick settles with no
    /// active time at all.
    #[test]
    fn select_only_waiter_counts_observed_ticks_not_elapsed_time() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (snapshot, mut ledger, handle) = select_waiter(true);
        assert!(
            poll_select_waiter(
                &snapshot,
                &mut ledger,
                &handle,
                2,
                Duration::from_secs(3_600)
            )
            .is_pending(),
            "an hour of active time does not settle a wait of eight observed ticks"
        );
        let result = poll_select_waiter(
            &snapshot,
            &mut ledger,
            &handle,
            1 + SELECT_FALLBACK_TICKS,
            Duration::ZERO,
        );
        assert!(
            matches!(
                result,
                Poll::Ready(Ok(SelectedBank {
                    kind: PickKind::AirFallback,
                    access: None,
                    ..
                }))
            ),
            "the bound tick settles the captured fallback with no elapsed time: {result:?}"
        );
    }

    /// The select fallback is the host job's window in whole production
    /// ticks and no smaller: the kernel never gives up before the host job's
    /// own window has closed, and never waits a whole tick longer than it.
    #[test]
    fn the_select_fallback_never_gives_up_before_the_host_window_closes() {
        let window = BANK_PICK_HOST_WINDOW_MS;
        assert!(SELECT_FALLBACK_TICKS * ops::SERVER_TICK_MS >= window);
        assert!((SELECT_FALLBACK_TICKS - 1) * ops::SERVER_TICK_MS < window);
    }

    /// Size tripwire: a wait deadline is one observed tick (no clock), so a
    /// select fallback that holds one costs the deadline's own eight bytes.
    #[test]
    #[cfg(target_pointer_width = "64")]
    fn wait_deadlines_hold_one_tick_and_no_clock() {
        assert_eq!(std::mem::size_of::<BankDeadline>(), 8);
        assert_eq!(std::mem::size_of::<Option<BankDeadline>>(), 8);
        assert_eq!(
            std::mem::size_of::<SelectFallback>(),
            std::mem::size_of::<SelectedBank>() + 8
        );
    }

    /// The bound counts from the dispatch, not from construction: a request
    /// built at tick 1 and dispatched at tick 4 is still waiting at tick
    /// `1 + SELECT_FALLBACK_TICKS` (its old construction-anchored bound) and
    /// one tick short of its own, and settles on its own bound tick.
    #[test]
    fn select_fallback_bound_counts_from_the_dispatch_not_from_construction() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (snapshot, mut ledger, handle, _) = select_waiter_dispatched_at(true, 4);
        for tick in [1 + SELECT_FALLBACK_TICKS, 4 + SELECT_FALLBACK_TICKS - 1] {
            assert!(
                poll_select_waiter(&snapshot, &mut ledger, &handle, tick, Duration::ZERO)
                    .is_pending(),
                "tick {tick}"
            );
        }
        let result = poll_select_waiter(
            &snapshot,
            &mut ledger,
            &handle,
            4 + SELECT_FALLBACK_TICKS,
            Duration::ZERO,
        );
        assert!(
            matches!(
                result,
                Poll::Ready(Ok(SelectedBank {
                    kind: PickKind::AirFallback,
                    access: None,
                    ..
                }))
            ),
            "{result:?}"
        );
    }

    /// The host job's receipt is the signal: one that is published by the
    /// bound tick wins over the kernel's own fallback.
    #[test]
    fn select_receipt_published_by_the_bound_tick_beats_the_fallback() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (snapshot, mut ledger, handle, authority) = select_waiter_dispatched_at(true, 1);
        let selected = SelectedBank {
            bank_index: 0,
            access_tile: WorldTile {
                x: 10,
                z: 5,
                level: 0,
            },
            kind: PickKind::Reachable,
            access: None,
        };
        ledger.as_mut().unwrap().complete_bank_pick(
            &authority,
            BankPickReceipt {
                request_id: authority.request_id().get(),
                evidence: EvidenceStamp {
                    run: authority.run(),
                    tick: 1 + SELECT_FALLBACK_TICKS,
                    sequence: 1 + SELECT_FALLBACK_TICKS,
                },
                outcome: BankPickOutcome::Selected(selected.clone()),
            },
        );
        let result = poll_select_waiter(
            &snapshot,
            &mut ledger,
            &handle,
            1 + SELECT_FALLBACK_TICKS,
            Duration::ZERO,
        );
        assert!(
            matches!(&result, Poll::Ready(Ok(got)) if *got == selected),
            "{result:?}"
        );
    }
    #[test]
    fn select_needs_step_receipt_preserves_gates_and_original_grants() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (snapshot, mut ledger, handle, authority) = select_waiter_dispatched_at(false, 1);
        let at = WorldTile {
            x: 5,
            z: 5,
            level: 0,
        };
        let gates: Arc<[api::prereq_steps::ActionableGate]> =
            Arc::from([api::prereq_steps::ActionableGate {
                gate: api::selected::QuestGate::Complete(api::selected::FactKey::new("bank-gate")),
                truth: Truth::Unknown,
                leg: 0,
                crossing: api::prereq_steps::CrossingId {
                    loc_id: 7,
                    option: 0,
                    at,
                    takeoff: at,
                    to: WorldTile {
                        x: 6,
                        z: 5,
                        level: 0,
                    },
                    kind: api::prereq_steps::CrossingKind::Ladder,
                },
            }]);
        let allow = crate::native::PrereqAllow {
            dialogue: true,
            quest_start: false,
        };
        ledger.as_mut().unwrap().complete_bank_pick(
            &authority,
            BankPickReceipt {
                request_id: authority.request_id().get(),
                evidence: EvidenceStamp {
                    run: authority.run(),
                    tick: 2,
                    sequence: 2,
                },
                outcome: BankPickOutcome::NeedsStep {
                    gates: Arc::clone(&gates),
                    allow,
                },
            },
        );
        let observed_allow = with_tick(&snapshot, &mut ledger, 2, |frame| {
            handle.inspect_in(&mut frame.cx, |select, cx| select.prereq_allow(cx))
        })
        .flatten();
        assert_eq!(observed_allow, Some(allow));

        let result = poll_select_waiter(&snapshot, &mut ledger, &handle, 2, Duration::ZERO);
        assert!(
            matches!(&result, Poll::Ready(Err(ActionError::NeedsStep(found))) if found.as_ref() == gates.as_ref()),
            "{result:?}"
        );
    }

    #[test]
    fn select_no_candidate_preserves_failure_detail_and_air_fallback() {
        if !api::test_support::game_data_available() {
            return;
        }
        let detail: Arc<str> = Arc::from("NoPath: no route to an eligible bank");
        for allow_air_fallback in [false, true] {
            let (snapshot, mut ledger, handle, authority) =
                select_waiter_dispatched_at(allow_air_fallback, 1);
            ledger.as_mut().unwrap().complete_bank_pick(
                &authority,
                BankPickReceipt {
                    request_id: authority.request_id().get(),
                    evidence: EvidenceStamp {
                        run: authority.run(),
                        tick: 2,
                        sequence: 2,
                    },
                    outcome: BankPickOutcome::NoCandidate {
                        detail: Arc::clone(&detail),
                    },
                },
            );
            let no_candidate = with_tick(&snapshot, &mut ledger, 2, |frame| {
                handle.inspect_in(&mut frame.cx, |select, cx| select.no_candidate(cx))
            })
            .unwrap();
            assert_eq!(no_candidate, !allow_air_fallback);

            let result = poll_select_waiter(&snapshot, &mut ledger, &handle, 2, Duration::ZERO);
            if allow_air_fallback {
                assert!(
                    matches!(
                        &result,
                        Poll::Ready(Ok(SelectedBank {
                            kind: PickKind::AirFallback,
                            ..
                        }))
                    ),
                    "{result:?}"
                );
            } else {
                assert!(
                    matches!(&result, Poll::Ready(Err(ActionError::Failed(found))) if found == &detail),
                    "{result:?}"
                );
            }
        }
    }

    fn service_request(
        kind: api::service_pick::ServiceKind,
        eligible: &[u16],
    ) -> ServicePickRequest {
        ServicePickRequest {
            kind,
            from: WorldTile {
                x: 5,
                z: 5,
                level: 0,
            },
            options: WalkOptions::default(),
            facts: ServiceFacts::Bank(Arc::new(NamedBankFacts::from_banks(vec![NamedBank::new(
                "service",
                WorldTile {
                    x: 6,
                    z: 5,
                    level: 0,
                },
            )]))),
            eligible: Arc::from(eligible),
            prefer_order: false,
        }
    }

    #[test]
    fn service_select_refuses_empty_or_mismatched_domains_without_host_work() {
        if !api::test_support::game_data_available() {
            return;
        }
        use api::service_pick::ServiceKind;
        for (kind, eligible, expected) in [
            (ServiceKind::Bank, &[][..], ServicePickOutcome::NoCandidate),
            (
                ServiceKind::ShopKeeper,
                &[0][..],
                ServicePickOutcome::NotSettled,
            ),
        ] {
            let mut snapshot = GameSnapshot::new();
            snapshot.seed_ingame(2);
            let mut ledger = None;
            let result = with_tick(&snapshot, &mut ledger, 1, |frame| {
                let handle = frame
                    .actions
                    .begin::<SelectService>(service_request(kind, eligible), &mut frame.cx)
                    .unwrap();
                frame.actions.poll(&handle, &mut frame.cx)
            });
            assert_eq!(result, Poll::Ready(Ok(expected)));
            assert!(ledger.as_ref().unwrap().outbox.is_empty());
        }
    }

    #[test]
    fn service_select_waits_allocation_free_and_preserves_every_host_outcome() {
        if !api::test_support::game_data_available() {
            return;
        }
        use api::service_pick::ServiceKind;
        for expected in [
            ServicePickOutcome::Routed {
                index: 0,
                access_tile: WorldTile {
                    x: 5,
                    z: 5,
                    level: 0,
                },
                cost: 0,
            },
            ServicePickOutcome::NoCandidate,
            ServicePickOutcome::NoPath { unresolved: None },
            ServicePickOutcome::BudgetExhausted,
            ServicePickOutcome::NotSettled,
        ] {
            let mut snapshot = GameSnapshot::new();
            snapshot.seed_ingame(2);
            let mut ledger = None;
            let handle = with_tick(&snapshot, &mut ledger, 1, |frame| {
                let handle = frame
                    .actions
                    .begin::<SelectService>(service_request(ServiceKind::Bank, &[0]), &mut frame.cx)
                    .unwrap();
                assert!(frame.actions.poll(&handle, &mut frame.cx).is_pending());
                handle
            });
            let pending = ledger.as_mut().unwrap();
            assert_eq!(pending.outbox.len(), 1);
            let action = pending.outbox.remove(0);
            assert!(matches!(action.effect, HostEffect::ServicePick(_)));
            let allocations = allocation_counter::measure(|| {
                for tick in 2..=1001 {
                    with_tick(&snapshot, &mut ledger, tick, |frame| {
                        frame.cx.active_now = Duration::from_secs(3_600);
                        assert!(frame.actions.poll(&handle, &mut frame.cx).is_pending());
                    });
                }
            });
            assert_eq!(allocations, allocation_counter::AllocationInfo::default());
            assert!(ledger.as_ref().unwrap().outbox.is_empty());
            ledger.as_mut().unwrap().complete_service_pick(
                &action.authority(),
                ServicePickReceipt {
                    request_id: action.request_id.get(),
                    evidence: EvidenceStamp {
                        run: action.run(),
                        tick: 1002,
                        sequence: 1002,
                    },
                    outcome: expected.clone(),
                },
            );
            let result = with_tick(&snapshot, &mut ledger, 1002, |frame| {
                frame.actions.poll(&handle, &mut frame.cx)
            });
            assert_eq!(result, Poll::Ready(Ok(expected)));
            assert!(ledger.as_ref().unwrap().outbox.is_empty());
        }
    }

    #[test]
    fn cancelling_service_select_revokes_queued_work_and_rejects_its_late_result() {
        if !api::test_support::game_data_available() {
            return;
        }
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        let mut ledger = None;
        let handle = with_tick(&snapshot, &mut ledger, 1, |frame| {
            let handle = frame
                .actions
                .begin::<SelectService>(
                    service_request(api::service_pick::ServiceKind::Bank, &[0]),
                    &mut frame.cx,
                )
                .unwrap();
            assert!(frame.actions.poll(&handle, &mut frame.cx).is_pending());
            handle
        });
        let action = ledger.as_mut().unwrap().outbox.remove(0);
        assert!(action.live());
        drop(handle);
        assert!(!action.live(), "cancelled work cannot reach host admission");
        ledger.as_mut().unwrap().complete_service_pick(
            &action.authority(),
            ServicePickReceipt {
                request_id: action.request_id.get(),
                evidence: EvidenceStamp {
                    run: action.run(),
                    tick: 2,
                    sequence: 2,
                },
                outcome: ServicePickOutcome::NotSettled,
            },
        );
        assert!(ledger.as_ref().unwrap().pick.is_none());
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
            access: None,
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
        if !api::test_support::game_data_available() {
            return;
        }
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
        if !api::test_support::game_data_available() {
            return;
        }
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
    fn catalog_already_open_duel_chest_banks_without_an_opener() {
        if !api::test_support::game_data_available() {
            return;
        }
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
            3194,
            "Open chest",
            "Bank",
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
        assert_eq!(
            ledger.as_ref().unwrap().outbox.len(),
            1,
            "an already-open chest must receive the Bank action"
        );
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::Loc {
                action,
                id: Some(3194),
                ..
            }) if action == "Bank"
        ));
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), Vec::new());
        let result = with_tick_reach(&snapshot, &reach, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(matches!(result, Poll::Ready(Ok(receipt)) if receipt.complete));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }

    #[test]
    fn catalog_booth_opens_without_an_object_opener() {
        if !api::test_support::game_data_available() {
            return;
        }
        let bank = catalog_bank("Catherby");
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(bank.tile));
        snapshot.seed_locs(vec![bank_object_loc(
            2213,
            "Bank booth",
            "Use-quickly",
            bank.tile,
            0,
        )]);
        let access = Arc::new(BankStandAccess {
            bank,
            stand_tile: bank.tile,
            kind: AccessKind::Booth,
            stand_op: 1,
            name: Some(Arc::from("Bank booth")),
            choose: None,
        });
        let mut ledger = None;
        let handle = start_bank(&snapshot, &mut ledger, open_request(access));
        with_tick(&snapshot, &mut ledger, 1, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::OpenStand {
                kind,
                name: Some(name),
                stand_op: Some(1),
                ..
            }) if kind == "booth" && name == "Bank booth"
        ));
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), Vec::new());
        let result = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(matches!(result, Poll::Ready(Ok(receipt)) if receipt.complete));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }

    #[test]
    fn catalog_open_first_bank_resolves_live_loc_stages_near_stand() {
        if !api::test_support::game_data_available() {
            return;
        }
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
        assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
        assert!(matches!(
            acknowledge(&mut ledger, 3),
            HostEffect::Interaction(InteractReq::Loc {
                action,
                id: Some(4002),
                ..
            }) if action == "Bank"
        ));
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), Vec::new());
        let result = with_tick_reach(&snapshot, &reach, &mut ledger, 4, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(matches!(result, Poll::Ready(Ok(receipt)) if receipt.complete));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
        assert!(
            ledger
                .as_ref()
                .unwrap()
                .bank_session
                .is_some_and(|mark| mark.opened_by_machine),
            "the chest's Bank click is this machine's open"
        );
    }

    /// The chest lid click is not a bank-open request. A bank that opens
    /// after it (the player's) is not marked as this machine's open.
    #[test]
    fn a_bank_the_player_opens_after_the_chest_lid_click_is_not_marked() {
        if !api::test_support::game_data_available() {
            return;
        }
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
        // The lid stays shut. The player opens a bank.
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), Vec::new());
        let result = with_tick_reach(&snapshot, &reach, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(matches!(result, Poll::Ready(Ok(_))), "{result:?}");
        assert!(
            !ledger
                .as_ref()
                .unwrap()
                .bank_session
                .is_some_and(|mark| mark.opened_by_machine),
            "{:?}",
            ledger.as_ref().unwrap().bank_session
        );
    }

    #[test]
    fn catalog_object_stuck_continue_modal_uses_its_page_ack_bound() {
        if !api::test_support::game_data_available() {
            return;
        }
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

        let tick_number = 8;
        with_tick_reach(&snapshot, &reach, &mut ledger, tick_number, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        assert!(matches!(
            acknowledge(&mut ledger, tick_number),
            HostEffect::Interaction(InteractReq::ContinueDialog { component_id: None })
        ));

        let pending = with_tick_reach(&snapshot, &reach, &mut ledger, 9, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(
            pending.is_pending(),
            "the original open bound cannot expire an active dialogue: {pending:?}"
        );
        assert!(ledger.as_ref().unwrap().outbox.is_empty());

        let retried = with_tick_reach(&snapshot, &reach, &mut ledger, 13, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(retried.is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 13),
            HostEffect::Interaction(InteractReq::ContinueDialog { component_id: None })
        ));
    }

    #[test]
    fn catalog_object_finished_dialogue_does_not_restart_for_stuck_modal() {
        if !api::test_support::game_data_available() {
            return;
        }
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
        for tick_number in [2, 8, 9, 14, 20] {
            let pending = with_tick_reach(&snapshot, &reach, &mut ledger, tick_number, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            });
            assert!(
                pending.is_pending(),
                "active object dialogue remains inside its own bound at tick {tick_number}: {pending:?}"
            );
            assert!(ledger.as_ref().unwrap().outbox.is_empty());
        }

        assert!(with_tick_reach(&snapshot, &reach, &mut ledger, 27, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        assert!(matches!(
            with_tick_reach(&snapshot, &reach, &mut ledger, 28, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            }),
            Poll::Ready(Err(ActionError::Failed(message)))
                if message.as_ref() == "bank item table unavailable"
        ));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }

    #[test]
    fn catalog_open_first_missing_locs_respects_observed_tick_limit() {
        if !api::test_support::game_data_available() {
            return;
        }
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
        let result = with_tick(&missing_locs, &mut ledger, 9, |tick| {
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
        if !api::test_support::game_data_available() {
            return;
        }
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

        for tick_number in 2..=10 {
            let result = with_tick(&snapshot, &mut ledger, tick_number, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            });
            assert!(
                result.is_pending(),
                "Gundai's Talk-to remains inside its six-second dialogue bound at tick {tick_number}: {result:?}"
            );
        }

        // The Mage Arena teller has now used its full NPC_DIALOG_MS window;
        // the content issues @openbank only after the dialogue choice.
        snapshot.seed_chat_modal(1, vec!["First page".into()]);
        snapshot.seed_chat_options(Vec::new(), 99);
        with_tick(&snapshot, &mut ledger, 11, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        assert!(matches!(
            acknowledge(&mut ledger, 11),
            HostEffect::Interaction(InteractReq::ContinueDialog { component_id: None })
        ));

        snapshot.seed_chat_modal(2, vec!["Second page".into()]);
        with_tick(&snapshot, &mut ledger, 12, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
        with_tick(&snapshot, &mut ledger, 13, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        assert!(matches!(
            acknowledge(&mut ledger, 13),
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
        with_tick(&snapshot, &mut ledger, 14, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        with_tick(&snapshot, &mut ledger, 15, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        assert!(matches!(
            acknowledge(&mut ledger, 15),
            HostEffect::Interaction(InteractReq::Answer { option: 2 })
        ));

        snapshot.seed_chat_modal(-1, Vec::new());
        snapshot.seed_chat_options(Vec::new(), -1);
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), Vec::new());
        with_tick(&snapshot, &mut ledger, 16, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        with_tick(&snapshot, &mut ledger, 17, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        let receipt = with_tick(&snapshot, &mut ledger, 18, |tick| {
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
        if !api::test_support::game_data_available() {
            return;
        }
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
        if !api::test_support::game_data_available() {
            return;
        }
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
    fn native_teller_access_waits_for_loaded_stock_after_session_opens() {
        if !api::test_support::game_data_available() {
            return;
        }
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
        assert!(
            result.is_pending(),
            "an open bank session may still be loading its item table"
        );

        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), Vec::new());
        let result = with_tick(&snapshot, &mut ledger, 9, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(matches!(
            result,
            Poll::Ready(Ok(BankReceipt { complete: true, .. }))
        ));
    }

    /// A banker's dialogue is not driven while a bank modal is up, so the open
    /// that adopted this modal stays terminal at the table bound.
    #[test]
    fn native_teller_access_fails_when_open_session_never_loads_stock() {
        if !api::test_support::game_data_available() {
            return;
        }
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

        let result = with_tick(&snapshot, &mut ledger, 9, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(matches!(
            result,
            Poll::Ready(Err(ActionError::Failed(message)))
                if message.as_ref() == "bank item table unavailable"
        ));
    }

    #[test]
    fn bank_machine_without_interrupt_unchanged() {
        if !api::test_support::game_data_available() {
            return;
        }
        let item_id = 314;
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(vec![item(item_id, "Ore", 1, ItemContainer::Inventory)], 28);
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), Vec::new());
        let mut ledger = None;
        let request = BankRequest {
            bank: None,
            action: BankAction::DepositProducts {
                products: Arc::from([item_id]),
                keep: Arc::from([]),
            },
            partial_ok: false,
            access: None,
        };
        let (handle, result) = begin_bank(&snapshot, &mut ledger, request);
        assert!(result.is_pending());
        assert!(ledger.as_ref().unwrap().outbox.is_empty());

        let side = vec![row_with(
            item(item_id, "Ore", 1, ItemContainer::BankSide),
            &["Deposit-1", "Deposit-5", "Deposit-All"],
            0,
        )];
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), side);
        let result = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(result.is_pending());
        let generation = snapshot.bank_session_generation();
        assert!(matches!(
            acknowledge(&mut ledger, 2),
            HostEffect::Interaction(InteractReq::InvButton {
                id,
                slot: 0,
                component: 7,
                operation: 3,
                bank_generation,
            }) if id == item_id && bank_generation == generation
        ));

        snapshot.seed_inventory(Vec::new(), 28);
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), Vec::new());
        let result = with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(matches!(
            result,
            Poll::Ready(Ok(BankReceipt { complete: true, .. }))
        ));
    }

    #[test]
    fn deposit_products_succeeds_when_inventory_products_are_already_zero() {
        if !api::test_support::game_data_available() {
            return;
        }
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
                        access: None,
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
        if !api::test_support::game_data_available() {
            return;
        }
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
                        access: None,
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
        if !api::test_support::game_data_available() {
            return;
        }
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
                        access: None,
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
            access: None,
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
    fn begin_withdraw_after_item_mode(
        snapshot: &GameSnapshot,
        ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
        item_id: i32,
    ) -> crate::native::ActionHandle<BankMachine> {
        let (handle, started) = begin_bank(
            snapshot,
            ledger,
            bank_request(
                BankAction::Withdraw {
                    item: BankItem {
                        id: item_id,
                        name: Arc::from("Bait"),
                    },
                    qty: 2,
                },
                false,
            ),
        );
        assert!(started.is_pending());
        assert!(matches!(
            acknowledge(ledger, 2),
            HostEffect::Interaction(InteractReq::SetNoteMode { on: false })
        ));
        assert!(with_tick(snapshot, ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        assert!(matches!(
            ledger.as_ref().unwrap().outbox.as_slice(),
            [action]
                if matches!(
                    &action.effect,
                    HostEffect::Interaction(InteractReq::WithdrawX {
                        bank_item_id,
                        lands_as_id,
                        ..
                    }) if *bank_item_id == item_id && *lands_as_id == item_id
                )
        ));
        handle
    }

    /// P-ladder (native): a fixed-only row serves an exact 7 as 5 + 1 + 1;
    /// one landed click is never the request.
    #[test]
    fn exact_withdraw_clicks_the_fixed_ladder_until_the_final_count() {
        if !api::test_support::game_data_available() {
            return;
        }
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
        if !api::test_support::game_data_available() {
            return;
        }
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
        if !api::test_support::game_data_available() {
            return;
        }
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
                row_with(item(trout, "Trout", 2, ItemContainer::Inventory), &[], 0),
                row_with(item(noted, "Trout", 9, ItemContainer::Inventory), &[], 1),
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
        snapshot.seed_inventory(
            vec![row_with(
                item(trout, "Trout", 2, ItemContainer::Inventory),
                &[],
                0,
            )],
            28,
        );
        let poll = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(matches!(
            poll,
            Poll::Ready(Ok(BankReceipt { complete: true, .. }))
        ));
    }
    /// A main-first receipt after the first fish must skip that fish in the
    /// still-old side component and select the held next target instead.
    #[test]
    fn stale_side_after_first_fish_clicks_the_current_next_target() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (swordfish, tuna, net, beer, harpoon, coins) = (371, 359, 303, 1917, 311, 995);
        let side_row = |id, name, count, slot| {
            row_with(
                item(id, name, count, ItemContainer::BankSide),
                &["Deposit-All"],
                slot,
            )
        };
        let pack_row = |id, name, count, slot| {
            row_with(item(id, name, count, ItemContainer::Inventory), &[], slot)
        };
        let side = vec![
            side_row(swordfish, "Swordfish", 13, 0),
            side_row(tuna, "Tuna", 14, 1),
            side_row(net, "Net", 1, 2),
            side_row(beer, "Beer", 1, 3),
            side_row(harpoon, "Harpoon", 1, 4),
            side_row(coins, "Coins", 6_191, 5),
        ];
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(
            vec![
                pack_row(swordfish, "Swordfish", 13, 0),
                pack_row(tuna, "Tuna", 14, 1),
                pack_row(net, "Net", 1, 2),
                pack_row(beer, "Beer", 1, 3),
                pack_row(harpoon, "Harpoon", 1, 4),
                pack_row(coins, "Coins", 6_191, 5),
            ],
            28,
        );
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), side);
        let mut ledger = None;
        let request = bank_request(
            BankAction::DepositProducts {
                products: Arc::from([swordfish, tuna, net, beer]),
                keep: Arc::from([harpoon, coins]),
            },
            false,
        );
        let (handle, poll) = begin_bank(&snapshot, &mut ledger, request);
        assert!(poll.is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::InvButton { id, .. }) if id == swordfish
        ));

        snapshot.seed_inventory(
            vec![
                pack_row(tuna, "Tuna", 14, 1),
                pack_row(net, "Net", 1, 2),
                pack_row(beer, "Beer", 1, 3),
                pack_row(harpoon, "Harpoon", 1, 4),
                pack_row(coins, "Coins", 6_191, 5),
            ],
            28,
        );
        let poll = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(poll.is_pending());
        let action = ledger.as_ref().unwrap().outbox.first().unwrap();
        let trace = action.bank_deposit.as_ref().expect("deposit trace");
        assert_eq!(trace.id, tuna);
        assert_eq!(trace.main_before, 14);
        assert_eq!(trace.transfer, 2);
        assert_eq!(trace.option.as_ref(), "Deposit-All");
    }

    /// After the last fish leaves main inventory, a stale fish row cannot
    /// hide the held incidental targets; net and beer remain depositable.
    #[test]
    fn stale_last_fish_side_row_does_not_replace_net_or_beer() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (swordfish, tuna, net, beer, harpoon, coins) = (371, 359, 303, 1917, 311, 995);
        let side_row = |id, name, count, slot| {
            row_with(
                item(id, name, count, ItemContainer::BankSide),
                &["Deposit-All"],
                slot,
            )
        };
        let pack_row = |id, name, count, slot| {
            row_with(item(id, name, count, ItemContainer::Inventory), &[], slot)
        };
        let side = vec![
            side_row(swordfish, "Swordfish", 13, 0),
            side_row(tuna, "Tuna", 14, 1),
            side_row(net, "Net", 1, 2),
            side_row(beer, "Beer", 1, 3),
            side_row(harpoon, "Harpoon", 1, 4),
            side_row(coins, "Coins", 6_191, 5),
        ];
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(
            vec![
                pack_row(swordfish, "Swordfish", 13, 0),
                pack_row(tuna, "Tuna", 14, 1),
                pack_row(net, "Net", 1, 2),
                pack_row(beer, "Beer", 1, 3),
                pack_row(harpoon, "Harpoon", 1, 4),
                pack_row(coins, "Coins", 6_191, 5),
            ],
            28,
        );
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), side);
        let mut ledger = None;
        let request = || {
            bank_request(
                BankAction::DepositProducts {
                    products: Arc::from([swordfish, tuna, net, beer]),
                    keep: Arc::from([harpoon, coins]),
                },
                false,
            )
        };
        let (handle, poll) = begin_bank(&snapshot, &mut ledger, request());
        assert!(poll.is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::InvButton { id, .. }) if id == swordfish
        ));

        snapshot.seed_inventory(
            vec![
                pack_row(tuna, "Tuna", 14, 1),
                pack_row(net, "Net", 1, 2),
                pack_row(beer, "Beer", 1, 3),
                pack_row(harpoon, "Harpoon", 1, 4),
                pack_row(coins, "Coins", 6_191, 5),
            ],
            28,
        );
        let poll = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(poll.is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 2),
            HostEffect::Interaction(InteractReq::InvButton { id, .. }) if id == tuna
        ));

        snapshot.seed_inventory(
            vec![
                pack_row(net, "Net", 1, 2),
                pack_row(beer, "Beer", 1, 3),
                pack_row(harpoon, "Harpoon", 1, 4),
                pack_row(coins, "Coins", 6_191, 5),
            ],
            28,
        );
        let poll = with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(poll.is_pending());
        let action = ledger.as_ref().unwrap().outbox.first().unwrap();
        let trace = action.bank_deposit.as_ref().expect("deposit trace");
        assert_eq!(trace.id, net);
        assert_eq!(trace.main_before, 1);
        assert_eq!(trace.transfer, 3);
    }

    /// No current candidates means stale side rows are harmless; the kept
    /// method tool and coins remain protected.
    #[test]
    fn stale_side_rows_do_not_block_completion_with_only_protected_items() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (swordfish, tuna, net, beer, harpoon, coins) = (371, 359, 303, 1917, 311, 995);
        let side_row = |id, name, count, slot| {
            row_with(
                item(id, name, count, ItemContainer::BankSide),
                &["Deposit-All"],
                slot,
            )
        };
        let pack_row = |id, name, count, slot| {
            row_with(item(id, name, count, ItemContainer::Inventory), &[], slot)
        };
        let side = vec![
            side_row(swordfish, "Swordfish", 13, 0),
            side_row(tuna, "Tuna", 14, 1),
            side_row(net, "Net", 1, 2),
            side_row(beer, "Beer", 1, 3),
        ];
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(
            vec![
                pack_row(harpoon, "Harpoon", 1, 4),
                pack_row(coins, "Coins", 6_191, 5),
            ],
            28,
        );
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), side);
        let mut ledger = None;
        let (_, poll) = begin_bank(
            &snapshot,
            &mut ledger,
            bank_request(
                BankAction::DepositProducts {
                    products: Arc::from([swordfish, tuna, net, beer]),
                    keep: Arc::from([harpoon, coins]),
                },
                false,
            ),
        );
        assert!(matches!(
            poll,
            Poll::Ready(Ok(BankReceipt { complete: true, .. }))
        ));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }

    /// `BankMachine` owns transfers for both the native Gatherer and Quester.
    /// One delayed first observation must not spend the tick budget as wall time.
    #[test]
    fn transfer_is_pending_after_a_thirty_second_gap_and_settles_on_count_delta() {
        if !api::test_support::game_data_available() {
            return;
        }
        let id = 333;
        let side = vec![row_with(
            item(id, "Trout", 2, ItemContainer::BankSide),
            &["Deposit-1", "Deposit-5", "Deposit-All"],
            0,
        )];
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(vec![item(id, "Trout", 2, ItemContainer::Inventory)], 28);
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), side);
        let mut ledger = None;
        let (handle, poll) = begin_bank(
            &snapshot,
            &mut ledger,
            bank_request(
                BankAction::Deposit {
                    item: BankItem {
                        id,
                        name: Arc::from("Trout"),
                    },
                },
                false,
            ),
        );
        assert!(poll.is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::InvButton { id: clicked, .. }) if clicked == id
        ));

        let delayed = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.cx.active_now = Duration::from_secs(30);
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(
            delayed.is_pending(),
            "one unchanged observation after 30 wall seconds is not eight observed ticks: {delayed:?}"
        );

        snapshot.seed_inventory(Vec::new(), 28);
        let settled = with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(matches!(
            settled,
            Poll::Ready(Ok(BankReceipt { complete: true, .. }))
        ));
    }

    #[test]
    fn same_tick_transfer_repolls_remain_pending_after_wall_clock_gap() {
        if !api::test_support::game_data_available() {
            return;
        }
        let id = 333;
        let side = vec![row_with(
            item(id, "Trout", 2, ItemContainer::BankSide),
            &["Deposit-1", "Deposit-5", "Deposit-All"],
            0,
        )];
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(vec![item(id, "Trout", 2, ItemContainer::Inventory)], 28);
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), side);
        let mut ledger = None;
        let (handle, poll) = begin_bank(
            &snapshot,
            &mut ledger,
            bank_request(
                BankAction::Deposit {
                    item: BankItem {
                        id,
                        name: Arc::from("Trout"),
                    },
                },
                false,
            ),
        );
        assert!(poll.is_pending());
        let _ = acknowledge(&mut ledger, 1);

        for repoll in 1..=3 {
            let poll = with_tick(&snapshot, &mut ledger, 1, |tick| {
                tick.cx.active_now = Duration::from_secs(30);
                tick.actions.poll(&handle, &mut tick.cx)
            });
            assert!(
                poll.is_pending(),
                "same-tick repoll {repoll} cannot expire the transfer: {poll:?}"
            );
        }
        let poll = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.cx.active_now = Duration::from_secs(30);
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(
            poll.is_pending(),
            "one fresh tick after 30 wall seconds is still within the observed-tick bound: {poll:?}"
        );
    }

    #[test]
    fn transfer_expires_after_eight_fresh_ticks_and_success_wins_at_the_boundary() {
        if !api::test_support::game_data_available() {
            return;
        }
        let id = 333;
        let side = vec![row_with(
            item(id, "Trout", 2, ItemContainer::BankSide),
            &["Deposit-1", "Deposit-5", "Deposit-All"],
            0,
        )];
        let make_snapshot = || {
            let mut snapshot = GameSnapshot::new();
            snapshot.seed_ingame(2);
            snapshot.seed_inventory(vec![item(id, "Trout", 2, ItemContainer::Inventory)], 28);
            snapshot.seed_bank_observation(10, 1, Some(Vec::new()), side.clone());
            snapshot
        };
        let request = || {
            bank_request(
                BankAction::Deposit {
                    item: BankItem {
                        id,
                        name: Arc::from("Trout"),
                    },
                },
                false,
            )
        };

        let snapshot = make_snapshot();
        let mut ledger = None;
        let (handle, poll) = begin_bank(&snapshot, &mut ledger, request());
        assert!(poll.is_pending());
        let _ = acknowledge(&mut ledger, 1);
        for tick_number in 2..=8 {
            let poll = with_tick(&snapshot, &mut ledger, tick_number, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            });
            assert!(
                poll.is_pending(),
                "tick {tick_number} is within the eight-tick bound: {poll:?}"
            );
        }
        let expired = with_tick(&snapshot, &mut ledger, 9, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(expired.is_pending(), "the first expiry enters recovery");
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
        with_tick(&snapshot, &mut ledger, 10, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        let _ = acknowledge(&mut ledger, 10);
        for tick_number in 11..=17 {
            with_tick(&snapshot, &mut ledger, tick_number, |tick| {
                assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            });
        }
        let expired = with_tick(&snapshot, &mut ledger, 18, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(matches!(
            expired,
            Poll::Ready(Err(ActionError::BankDeposit(
                ops::DepositFailure::NoCountFall
            )))
        ));

        let mut snapshot = make_snapshot();
        let mut ledger = None;
        let (handle, poll) = begin_bank(&snapshot, &mut ledger, request());
        assert!(poll.is_pending());
        let _ = acknowledge(&mut ledger, 1);
        for tick_number in 2..=8 {
            let poll = with_tick(&snapshot, &mut ledger, tick_number, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            });
            assert!(poll.is_pending(), "tick {tick_number}: {poll:?}");
        }
        let settled_at_boundary = {
            snapshot.seed_inventory(Vec::new(), 28);
            with_tick(&snapshot, &mut ledger, 9, |tick| {
                tick.cx.active_now = Duration::from_secs(30);
                tick.actions.poll(&handle, &mut tick.cx)
            })
        };
        assert!(matches!(
            settled_at_boundary,
            Poll::Ready(Ok(BankReceipt { complete: true, .. }))
        ));
    }

    #[test]
    fn transfer_boundary_burst_expires_after_eight_observed_ticks_without_elapsed_time() {
        if !api::test_support::game_data_available() {
            return;
        }
        let id = 333;
        let side = vec![row_with(
            item(id, "Trout", 2, ItemContainer::BankSide),
            &["Deposit-1", "Deposit-5", "Deposit-All"],
            0,
        )];
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(vec![item(id, "Trout", 2, ItemContainer::Inventory)], 28);
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), side);
        let mut ledger = None;
        let (handle, started) = begin_bank(
            &snapshot,
            &mut ledger,
            bank_request(
                BankAction::Deposit {
                    item: BankItem {
                        id,
                        name: Arc::from("Trout"),
                    },
                },
                false,
            ),
        );
        assert!(started.is_pending());
        let _ = acknowledge(&mut ledger, 1);
        with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.cx.active_now = Duration::from_millis(600);
            tick.actions.interrupt(&handle);
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });

        // Keep the click's active time unchanged. Only eight observed ticks
        // can end this wait, including when observations arrive in a burst.
        for tick_number in 3..=8 {
            let poll = with_tick(&snapshot, &mut ledger, tick_number, |tick| {
                tick.cx.active_now = Duration::from_millis(600);
                tick.actions.poll(&handle, &mut tick.cx)
            });
            assert!(poll.is_pending(), "tick {tick_number}: {poll:?}");
        }

        let first_timeout = with_tick(&snapshot, &mut ledger, 9, |tick| {
            tick.cx.active_now = Duration::from_millis(600);
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(
            first_timeout.is_pending(),
            "the eighth observed tick enters one no-op reconciliation"
        );

        let cancelled = with_tick(&snapshot, &mut ledger, 10, |tick| {
            tick.cx.active_now = Duration::from_millis(600);
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(matches!(
            cancelled,
            Poll::Ready(Err(ActionError::Cancelled))
        ));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }

    #[test]
    fn note_mode_dispatch_expires_on_the_eighth_observed_tick_whatever_the_elapsed_time() {
        if !api::test_support::game_data_available() {
            return;
        }
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
        let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
            let handle = tick
                .actions
                .begin::<BankMachine>(
                    BankRequest {
                        bank: None,
                        action: BankAction::WithdrawTo {
                            withdrawals: Arc::from([Withdrawal {
                                id,
                                name: Arc::from("Bait"),
                                target: 2,
                            }]),
                        },
                        partial_ok: false,
                        access: None,
                    },
                    &mut tick.cx,
                )
                .unwrap();
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            handle
        });
        assert!(matches!(
            ledger.as_ref().unwrap().outbox.as_slice(),
            [action]
                if matches!(
                    &action.effect,
                    HostEffect::Interaction(InteractReq::SetNoteMode { on: false })
                )
        ));
        for tick_number in 2..=8 {
            let poll = with_tick(&snapshot, &mut ledger, tick_number, |tick| {
                // Two hours of active time do not expire the wait.
                tick.cx.active_now = Duration::from_secs(7_200);
                tick.actions.poll(&handle, &mut tick.cx)
            });
            assert!(
                poll.is_pending(),
                "observed tick {tick_number} is inside the eight-tick bound"
            );
        }
        let expired = with_tick(&snapshot, &mut ledger, 9, |tick| {
            // No active time at all: the eighth observed tick alone expires it.
            tick.cx.active_now = Duration::ZERO;
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(matches!(
            expired,
            Poll::Ready(Err(ActionError::Failed(message)))
                if message.as_ref() == "bank item mode dispatch did not settle"
        ));
    }
    #[test]
    fn withdraw_timeout_is_independent_of_elapsed_time() {
        if !api::test_support::game_data_available() {
            return;
        }
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
        let handle = begin_withdraw_after_item_mode(&snapshot, &mut ledger, item_id);

        for tick_number in 3..=9 {
            let pending = with_tick(&snapshot, &mut ledger, tick_number, |tick| {
                tick.cx.active_now = Duration::from_secs(7_200);
                tick.actions.poll(&handle, &mut tick.cx)
            });
            assert!(
                pending.is_pending(),
                "observed tick {tick_number} is inside the eight-tick bound"
            );
        }
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 10, |tick| {
                tick.cx.active_now = Duration::ZERO;
                tick.actions.poll(&handle, &mut tick.cx)
            }),
            Poll::Ready(Err(ActionError::Failed(message)))
                if message.as_ref() == "bank transfer did not settle"
        ));
    }

    #[test]
    fn withdraw_without_a_server_response_times_out_after_eight_observed_ticks() {
        if !api::test_support::game_data_available() {
            return;
        }
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
        let handle = begin_withdraw_after_item_mode(&snapshot, &mut ledger, item_id);

        for tick_number in 3..=9 {
            assert!(with_tick(&snapshot, &mut ledger, tick_number, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            })
            .is_pending());
        }
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 10, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            }),
            Poll::Ready(Err(ActionError::Failed(message)))
                if message.as_ref() == "bank transfer did not settle"
        ));
    }

    /// A deposit settles only in the bank session that sent it: a held count
    /// that falls while the bank closes, or closes and reopens as a new
    /// session, is never a completed deposit (Required or until-empty).
    #[test]
    fn deposit_does_not_settle_after_its_bank_session_is_gone() {
        if !api::test_support::game_data_available() {
            return;
        }
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
    /// At the transfer bound, unavailable inventory cannot mask timeout, and
    /// losing the bank session still takes precedence over that timeout.
    #[test]
    fn unavailable_protected_pack_preserves_timeout_and_session_ordering() {
        if !api::test_support::game_data_available() {
            return;
        }
        let trout = 333;
        let side = vec![row_with(
            item(trout, "Trout", 2, ItemContainer::BankSide),
            &["Deposit-All"],
            0,
        )];
        let request = || {
            bank_request(
                BankAction::DepositProducts {
                    products: Arc::from([trout]),
                    keep: Arc::from([311]),
                },
                false,
            )
        };
        let open = || {
            let mut snapshot = GameSnapshot::new();
            snapshot.seed_ingame(2);
            snapshot.seed_inventory(
                vec![
                    item(trout, "Trout", 2, ItemContainer::Inventory),
                    item(311, "Harpoon", 1, ItemContainer::Inventory),
                ],
                28,
            );
            snapshot.seed_bank_observation(10, 1, Some(Vec::new()), side.clone());
            snapshot
        };

        let snapshot = open();
        let mut ledger = None;
        let (handle, poll) = begin_bank(&snapshot, &mut ledger, request());
        assert!(poll.is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::InvButton { id, .. }) if id == trout
        ));
        let mut unavailable_open = GameSnapshot::new();
        unavailable_open.seed_ingame(2);
        unavailable_open.seed_bank_observation(10, 1, Some(Vec::new()), side.clone());
        for tick in 2..=8 {
            let poll = with_tick(&unavailable_open, &mut ledger, tick, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            });
            assert!(
                poll.is_pending(),
                "tick {tick} is within the eight-tick transfer bound: {poll:?}"
            );
        }
        let retry = with_tick(&unavailable_open, &mut ledger, 9, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(retry.is_pending(), "recovery re-observes the missing pack");
        for tick_number in 10..=16 {
            with_tick(&unavailable_open, &mut ledger, tick_number, |tick| {
                assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            });
        }
        let timed_out = with_tick(&unavailable_open, &mut ledger, 17, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(matches!(
            timed_out,
            Poll::Ready(Err(ActionError::BankDeposit(
                ops::DepositFailure::NoCountFall
            )))
        ));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());

        let snapshot = open();
        let mut ledger = None;
        let (handle, poll) = begin_bank(&snapshot, &mut ledger, request());
        assert!(poll.is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::InvButton { id, .. }) if id == trout
        ));
        let mut unavailable_closed = GameSnapshot::new();
        unavailable_closed.seed_ingame(2);
        unavailable_closed.seed_bank_observation(-1, 1, None, Vec::new());
        let closed = with_tick(&unavailable_closed, &mut ledger, 9, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(matches!(
            closed,
            Poll::Ready(Err(ActionError::Failed(message)))
                if message.as_ref() == "bank closed before the transfer settled"
        ));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }

    /// Until-empty waits for a usable side view; an empty or closed side root
    /// cannot stand in for held candidates or report success at the bound.
    #[test]
    fn deposit_all_fails_when_side_view_never_becomes_usable() {
        if !api::test_support::game_data_available() {
            return;
        }
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
        let (handle, poll) = begin_bank(&snapshot, &mut ledger, all());
        assert!(
            poll.is_pending(),
            "a posted empty side cannot hide a held candidate"
        );
        with_tick(&snapshot, &mut ledger, 1, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        let poll = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(matches!(
            poll,
            Poll::Ready(Err(ActionError::Failed(message)))
                if message.as_ref() == "bank deposit view did not post"
        ));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());

        snapshot.seed_side_modal(-1);
        let mut ledger = None;
        let (handle, poll) = begin_bank(&snapshot, &mut ledger, all());
        assert!(poll.is_pending(), "a side root still down is not posted");
        with_tick(&snapshot, &mut ledger, 1, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        let poll = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(matches!(
            poll,
            Poll::Ready(Err(ActionError::Failed(message)))
                if message.as_ref() == "bank deposit view did not post"
        ));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());

        // The required wait has the same bound: its view is the same flush.
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
        with_tick(&snapshot, &mut ledger, 1, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        let poll = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(matches!(
            poll,
            Poll::Ready(Err(ActionError::Failed(message)))
                if message.as_ref() == "bank deposit view did not post"
        ));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }

    /// A side view that posts with the bound tick still settles the wait: the
    /// awaited state is checked before the deadline.
    #[test]
    fn a_side_view_posting_on_the_bound_tick_settles_the_wait() {
        if !api::test_support::game_data_available() {
            return;
        }
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(vec![item(526, "Bones", 1, ItemContainer::Inventory)], 28);
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), Vec::new());
        let mut ledger = None;
        let (handle, poll) = begin_bank(
            &snapshot,
            &mut ledger,
            bank_request(
                BankAction::DepositAll {
                    keep: Arc::from([]),
                },
                false,
            ),
        );
        assert!(poll.is_pending(), "the view has not posted at the arm tick");
        snapshot.seed_bank_observation(
            10,
            1,
            Some(Vec::new()),
            vec![row_with(
                item(526, "Bones", 1, ItemContainer::BankSide),
                &["Deposit-1", "Deposit-All"],
                0,
            )],
        );
        let poll = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(
            poll.is_pending(),
            "the click went out instead of the wait failing: {poll:?}"
        );
        assert!(matches!(
            acknowledge(&mut ledger, 2),
            HostEffect::Interaction(InteractReq::InvButton { id: 526, .. })
        ));
    }

    /// P-close (native): already shut is true with no verb. A Close is done
    /// only once the bank is shut, its old side root released and the
    /// session generation newer; main shut with the side still up keeps
    /// waiting. A reopened session or eight still-open observed ticks fails it.
    #[test]
    fn close_settles_on_the_acknowledged_close_not_the_main_root() {
        if !api::test_support::game_data_available() {
            return;
        }
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

        // The close dispatch was observed at tick 1; allow eight newer ticks.
        let snapshot = open();
        let mut ledger = None;
        let (handle, _) = begin_bank(&snapshot, &mut ledger, close());
        let _ = acknowledge(&mut ledger, 1);
        assert!(poll_at(&snapshot, &mut ledger, &handle, 8).is_pending());
        assert!(matches!(
            poll_at(&snapshot, &mut ledger, &handle, 9),
            Poll::Ready(Err(ActionError::Failed(_)))
        ));
    }

    #[test]
    fn named_bank_selector_fails_closed_on_skill_quest_and_preference_gates() {
        if !api::test_support::game_data_available() {
            return;
        }
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
        if !api::test_support::game_data_available() {
            return;
        }
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
        if !api::test_support::game_data_available() {
            return;
        }
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
        if !api::test_support::game_data_available() {
            return;
        }
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
        if !api::test_support::game_data_available() {
            return;
        }
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
                access: None,
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
        if !api::test_support::game_data_available() {
            return;
        }
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
        if !api::test_support::game_data_available() {
            return;
        }
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
        if !api::test_support::game_data_available() {
            return;
        }
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
        if !api::test_support::game_data_available() {
            return;
        }
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

    /// The machine's first teller talk is refused by the tick budget, so no
    /// request went out. A bank that the player opens next is not marked as
    /// this machine's open.
    #[test]
    fn a_bank_the_player_opens_after_a_denied_first_teller_talk_is_not_marked() {
        if !api::test_support::game_data_available() {
            return;
        }
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_npcs(vec![npc_row(7, "Gundai", &["Talk-to"])]);
        let mut ledger = None;
        let handle = start_bank(
            &snapshot,
            &mut ledger,
            open_request(teller_access(
                catalog_bank("Mage Arena"),
                Some("Gundai"),
                0,
                None,
            )),
        );
        let denied = with_tick(&snapshot, &mut ledger, 2, |tick| {
            spend_interaction(&mut tick.cx);
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(denied.is_pending(), "budget denial must remain Pending");
        assert!(ledger.as_ref().unwrap().outbox.is_empty());

        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), Vec::new());
        let result = with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(matches!(result, Poll::Ready(Ok(_))), "{result:?}");
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
        assert!(
            !ledger
                .as_ref()
                .unwrap()
                .bank_session
                .is_some_and(|mark| mark.opened_by_machine),
            "{:?}",
            ledger.as_ref().unwrap().bank_session
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
        if !api::test_support::game_data_available() {
            return;
        }
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
        if !api::test_support::game_data_available() {
            return;
        }
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
    #[test]
    fn slow_host_keeps_waiting_for_an_open_bank_session() {
        if !api::test_support::game_data_available() {
            return;
        }
        let stand = WorldTile {
            x: 3208,
            z: 3220,
            level: 2,
        };
        let bank = NamedBank::new("Test bank", stand);
        let access = Arc::new(BankStandAccess {
            bank,
            stand_tile: stand,
            kind: AccessKind::Booth,
            stand_op: 1,
            name: Some(Arc::from("Bank booth")),
            choose: None,
        });
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(stand));
        snapshot.seed_locs(vec![bank_object_loc(
            2213,
            "Bank booth",
            "Use-quickly",
            stand,
            0,
        )]);
        snapshot.seed_inventory(Vec::new(), 28);
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
            HostEffect::Interaction(InteractReq::OpenStand { .. })
        ));

        snapshot.seed_bank_observation(10, 1, None, Vec::new());
        let opening = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.cx.active_now = Duration::from_secs(30);
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(
            opening.is_pending(),
            "a slow host must keep waiting while the bank session is open"
        );

        snapshot.seed_bank_observation(10, 2, Some(Vec::new()), Vec::new());
        let opened = with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.cx.active_now = Duration::from_secs(60);
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(matches!(
            opened,
            Poll::Ready(Ok(BankReceipt { complete: true, .. }))
        ));
    }
    #[test]
    fn open_session_without_an_item_table_fails_after_the_observed_bound() {
        if !api::test_support::game_data_available() {
            return;
        }
        let stand = WorldTile {
            x: 3208,
            z: 3220,
            level: 2,
        };
        let bank = NamedBank::new("Test bank", stand);
        let access = Arc::new(BankStandAccess {
            bank,
            stand_tile: stand,
            kind: AccessKind::Booth,
            stand_op: 1,
            name: Some(Arc::from("Bank booth")),
            choose: None,
        });
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(stand));
        snapshot.seed_locs(vec![bank_object_loc(
            2213,
            "Bank booth",
            "Use-quickly",
            stand,
            0,
        )]);
        snapshot.seed_inventory(Vec::new(), 28);
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
            HostEffect::Interaction(InteractReq::OpenStand { .. })
        ));

        snapshot.seed_bank_observation(10, 1, None, Vec::new());
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.cx.active_now = Duration::from_millis(1_000);
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());

        for tick_number in 3..=9 {
            let pending = with_tick(&snapshot, &mut ledger, tick_number, |tick| {
                // A slow host: hours of active time never expire the wait.
                tick.cx.active_now = Duration::from_secs(7_200);
                tick.actions.poll(&handle, &mut tick.cx)
            });
            assert!(pending.is_pending());
        }
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 10, |tick| {
                tick.cx.active_now = Duration::ZERO;
                tick.actions.poll(&handle, &mut tick.cx)
            }),
            Poll::Ready(Err(ActionError::Failed(message)))
                if message.as_ref() == "bank item table unavailable"
        ));
    }

    #[test]
    fn slow_host_keeps_waiting_for_a_bank_close_to_release_its_side_root() {
        if !api::test_support::game_data_available() {
            return;
        }
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(Vec::new(), 28);
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), Vec::new());
        let side = snapshot.modals().side;
        let mut ledger = None;
        let (handle, started) = begin_bank(
            &snapshot,
            &mut ledger,
            bank_request(BankAction::Close, false),
        );
        assert!(started.is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::Close)
        ));

        snapshot.seed_bank_observation(-1, 1, None, Vec::new());
        snapshot.seed_side_modal(side);
        let closing = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.cx.active_now = Duration::from_secs(20);
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(
            closing.is_pending(),
            "one observed server tick must not expire because the host was slow"
        );

        snapshot.seed_side_modal(-1);
        let closed = with_tick(&snapshot, &mut ledger, 10, |tick| {
            tick.cx.active_now = Duration::from_secs(50);
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(matches!(
            closed,
            Poll::Ready(Ok(BankReceipt { complete: true, .. }))
        ));
    }

    #[test]
    fn close_times_out_when_main_shuts_but_side_root_stays_held() {
        if !api::test_support::game_data_available() {
            return;
        }
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(Vec::new(), 28);
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), Vec::new());
        let side = snapshot.modals().side;
        let mut ledger = None;
        let (handle, started) = begin_bank(
            &snapshot,
            &mut ledger,
            bank_request(BankAction::Close, false),
        );
        assert!(started.is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::Close)
        ));

        snapshot.seed_bank_observation(-1, 1, None, Vec::new());
        snapshot.seed_side_modal(side);
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.cx.active_now = Duration::from_secs(20);
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 9, |tick| {
                tick.cx.active_now = Duration::from_secs(40);
                tick.actions.poll(&handle, &mut tick.cx)
            }),
            Poll::Ready(Err(ActionError::Failed(message)))
                if message.as_ref() == "bank did not close"
        ));
    }

    #[test]
    fn deposit_wait_counts_only_observed_ticks_after_the_click() {
        if !api::test_support::game_data_available() {
            return;
        }
        let item_id = 314;
        let side = row_with(
            item(item_id, "Ore", 1, ItemContainer::BankSide),
            &["Deposit-1", "Deposit-5", "Deposit-All"],
            0,
        );
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(vec![item(item_id, "Ore", 1, ItemContainer::Inventory)], 28);
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), vec![side]);
        let mut ledger = None;
        let (handle, sent) = begin_bank(
            &snapshot,
            &mut ledger,
            bank_request(
                BankAction::Deposit {
                    item: BankItem {
                        id: item_id,
                        name: Arc::from("Ore"),
                    },
                },
                false,
            ),
        );
        assert!(sent.is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::InvButton { id, .. }) if id == item_id
        ));

        for tick in 2..=9 {
            let pending = with_tick(&snapshot, &mut ledger, tick, |frame| {
                // No clock decides the wait: a slow host (an hour of active
                // time) and a fast one (none) see the same observed ticks.
                frame.cx.active_now = Duration::from_secs(3_600);
                frame.actions.poll(&handle, &mut frame.cx)
            });
            assert!(
                pending.is_pending(),
                "observed tick {tick} is inside the deposit wait"
            );
        }

        snapshot.seed_inventory(Vec::new(), 28);
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 10, |frame| {
                frame.cx.active_now = Duration::ZERO;
                frame.actions.poll(&handle, &mut frame.cx)
            }),
            Poll::Ready(Ok(BankReceipt { complete: true, .. }))
        ));
    }

    #[test]
    fn deposit_missing_receipt_recovers_after_observed_tick_bounds() {
        if !api::test_support::game_data_available() {
            return;
        }
        let item_id = 314;
        let side = row_with(
            item(item_id, "Ore", 1, ItemContainer::BankSide),
            &["Deposit-1", "Deposit-5", "Deposit-All"],
            0,
        );
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(vec![item(item_id, "Ore", 1, ItemContainer::Inventory)], 28);
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), vec![side]);
        let mut ledger = None;
        let (handle, sent) = begin_bank(
            &snapshot,
            &mut ledger,
            bank_request(
                BankAction::Deposit {
                    item: BankItem {
                        id: item_id,
                        name: Arc::from("Ore"),
                    },
                },
                false,
            ),
        );
        assert!(sent.is_pending());

        for tick in 2..=9 {
            let pending = with_tick(&snapshot, &mut ledger, tick, |frame| {
                frame.cx.active_now = Duration::from_secs(3_600);
                frame.actions.poll(&handle, &mut frame.cx)
            });
            assert!(
                pending.is_pending(),
                "the missing receipt stays pending before the observed-tick bound"
            );
        }

        assert!(with_tick(&snapshot, &mut ledger, 9, |frame| {
            frame.cx.active_now = Duration::ZERO;
            frame.actions.poll(&handle, &mut frame.cx)
        })
        .is_pending());
        assert!(
            ledger.as_ref().unwrap().outbox.is_empty(),
            "the unacknowledged original request was cancelled"
        );
        assert!(with_tick(&snapshot, &mut ledger, 10, |frame| {
            frame.actions.poll(&handle, &mut frame.cx)
        })
        .is_pending());
        assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 18, |frame| {
                frame.actions.poll(&handle, &mut frame.cx)
            }),
            Poll::Ready(Err(ActionError::BankDeposit(
                ops::DepositFailure::DispatchMissing
            )))
        ));
    }

    #[test]
    fn deposit_no_response_recovers_after_eight_post_dispatch_ticks() {
        if !api::test_support::game_data_available() {
            return;
        }
        let item_id = 314;
        let side = row_with(
            item(item_id, "Ore", 1, ItemContainer::BankSide),
            &["Deposit-1", "Deposit-5", "Deposit-All"],
            0,
        );
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(vec![item(item_id, "Ore", 1, ItemContainer::Inventory)], 28);
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), vec![side]);
        let mut ledger = None;
        let (handle, sent) = begin_bank(
            &snapshot,
            &mut ledger,
            bank_request(
                BankAction::Deposit {
                    item: BankItem {
                        id: item_id,
                        name: Arc::from("Ore"),
                    },
                },
                false,
            ),
        );
        assert!(sent.is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 2),
            HostEffect::Interaction(InteractReq::InvButton { id, .. }) if id == item_id
        ));

        assert!(with_tick(&snapshot, &mut ledger, 9, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        assert!(with_tick(&snapshot, &mut ledger, 10, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
        assert!(with_tick(&snapshot, &mut ledger, 11, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        acknowledge(&mut ledger, 11);
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 19, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            }),
            Poll::Ready(Err(ActionError::BankDeposit(
                ops::DepositFailure::NoCountFall
            )))
        ));
    }

    /// A Catherby-style booth trip: the player on the stand's access tile
    /// with the booth loc (Use-quickly) in view, and the packed access the
    /// Gatherer records for the trip.
    fn booth_trip(stand: WorldTile) -> (Arc<BankStandAccess>, Vec<api::snapshot::LocView>) {
        let access = Arc::new(BankStandAccess {
            bank: NamedBank::new("Catherby", stand),
            stand_tile: stand,
            kind: AccessKind::Booth,
            stand_op: 2,
            name: Some(Arc::from("Bank booth")),
            choose: None,
        });
        let locs = vec![bank_object_loc(2213, "Bank booth", "Use-quickly", stand, 0)];
        (access, locs)
    }

    /// The ledger names the session that appeared after this machine's own
    /// open request, so a reader can tell it from a player's bank.
    #[test]
    fn a_session_that_appears_after_the_machines_own_open_is_marked_as_its_own() {
        if !api::test_support::game_data_available() {
            return;
        }
        let stand = WorldTile {
            x: 2809,
            z: 3441,
            level: 0,
        };
        let (access, locs) = booth_trip(stand);
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(stand));
        snapshot.seed_locs(locs);
        let mut ledger = None;
        let (handle, poll) = begin_bank(
            &snapshot,
            &mut ledger,
            BankRequest {
                bank: None,
                action: BankAction::Scan,
                partial_ok: false,
                access: Some(access),
            },
        );
        assert!(poll.is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::OpenStand { .. })
        ));
        assert_eq!(ledger.as_ref().unwrap().bank_session, None);

        // A session opens, with its table not yet loaded.
        snapshot.seed_bank_observation(10, 2, None, Vec::new());
        let waiting = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(waiting.is_pending(), "{waiting:?}");
        let mark = ledger.as_ref().unwrap().bank_session.expect("open mark");
        assert_eq!(mark.generation, snapshot.bank_session_generation());
        assert!(mark.opened_by_machine && mark.item_mode);

        snapshot.seed_bank_observation(10, 3, Some(Vec::new()), Vec::new());
        let done = with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(matches!(done, Poll::Ready(Ok(_))), "{done:?}");
        assert_eq!(
            ledger.as_ref().unwrap().bank_session,
            Some(mark),
            "the table's arrival does not change whose open it was"
        );
    }

    /// A bank that a player left open, and a machine that only works in it,
    /// never make that session the machine's own open.
    #[test]
    fn a_session_found_already_open_is_not_marked_as_the_machines_own() {
        if !api::test_support::game_data_available() {
            return;
        }
        let stand = WorldTile {
            x: 2809,
            z: 3441,
            level: 0,
        };
        let (access, locs) = booth_trip(stand);
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(stand));
        snapshot.seed_locs(locs);
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), Vec::new());
        let mut ledger = None;
        let (_handle, poll) = begin_bank(
            &snapshot,
            &mut ledger,
            BankRequest {
                bank: None,
                action: BankAction::Scan,
                partial_ok: false,
                access: Some(access),
            },
        );
        assert!(matches!(poll, Poll::Ready(Ok(_))), "{poll:?}");
        assert_eq!(ledger.as_ref().unwrap().bank_session, None);

        // Pressing Item in the player's bank marks its mode, not its owner.
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
        let _handle = begin_withdraw_after_item_mode(&snapshot, &mut ledger, id);
        let mark = ledger.as_ref().unwrap().bank_session.expect("mode mark");
        assert_eq!(mark.generation, snapshot.bank_session_generation());
        assert!(mark.item_mode);
        assert!(!mark.opened_by_machine);
    }

    fn reopen_deposit_request(
        product: i32,
        tool: i32,
        access: &Arc<BankStandAccess>,
    ) -> BankRequest {
        BankRequest {
            bank: None,
            action: BankAction::DepositProducts {
                products: Arc::from([product]),
                keep: Arc::from([tool]),
            },
            partial_ok: false,
            access: Some(Arc::clone(access)),
        }
    }

    fn fish_pack(product: i32, count: i32, tool: i32) -> Vec<ItemView> {
        let mut rows = vec![item(tool, "Small fishing net", 1, ItemContainer::Inventory)];
        if count > 0 {
            rows.push(item(
                product,
                "Raw shrimps",
                count,
                ItemContainer::Inventory,
            ));
        }
        rows
    }

    fn fish_side(product: i32, count: i32) -> Vec<ItemView> {
        vec![row_with(
            item(product, "Raw shrimps", count, ItemContainer::BankSide),
            &["Deposit-1", "Deposit-5", "Deposit-10", "Deposit-All"],
            0,
        )]
    }

    fn failed_with(poll: &Poll<Result<BankReceipt, ActionError>>, expected: &str) -> bool {
        matches!(
            poll,
            Poll::Ready(Err(ActionError::Failed(message))) if message.as_ref() == expected
        )
    }

    /// HOTFIX-0.2.1: a deposit click is sent in session G and the next
    /// observation has the bank shut (stop-transmit, if_close) with the held
    /// count unchanged: the deposit was refused or never applied. A trip
    /// that knows its stand clicks the booth again and resumes the same
    /// products sweep in the new session; the receipt settles once the
    /// count falls there. (The recorded Catherby order, in which the count
    /// had already fallen when the close was observed, is
    /// `deposit_applied_before_the_close_is_credited_once_after_the_reopen`.)
    #[test]
    fn deposit_reopens_its_stand_when_the_session_closes_before_the_transfer_settles() {
        if !api::test_support::game_data_available() {
            return;
        }
        let shrimp = 317;
        let net = 303;
        let stand = WorldTile {
            x: 2809,
            z: 3441,
            level: 0,
        };
        let (access, locs) = booth_trip(stand);
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(stand));
        snapshot.seed_locs(locs);
        snapshot.seed_inventory(fish_pack(shrimp, 27, net), 28);
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), fish_side(shrimp, 27));
        let first_generation = snapshot.bank_session_generation();
        let mut ledger = None;
        let (handle, poll) = begin_bank(
            &snapshot,
            &mut ledger,
            reopen_deposit_request(shrimp, net, &access),
        );
        assert!(poll.is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::InvButton { id, bank_generation, .. })
                if id == shrimp && bank_generation == first_generation
        ));

        // Session G closes with the count unchanged: nothing was applied.
        snapshot.seed_bank_observation(-1, 1, None, Vec::new());
        let lost = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(
            lost.is_pending(),
            "a lost session with a known stand is not terminal: {lost:?}"
        );
        assert!(
            matches!(
                acknowledge(&mut ledger, 2),
                HostEffect::Interaction(InteractReq::OpenStand { x, z, stand_op: Some(2), .. })
                    if x == stand.x && z == stand.z
            ),
            "the trip's stand is clicked again"
        );

        // The engine opens a new session: table and side view transmitted,
        // then the interface (bank.rs2:21-23).
        snapshot.seed_bank_observation(10, 2, Some(Vec::new()), fish_side(shrimp, 27));
        let second_generation = snapshot.bank_session_generation();
        assert_ne!(second_generation, first_generation);
        let resumed = with_tick(&snapshot, &mut ledger, 4, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(resumed.is_pending(), "{resumed:?}");
        assert!(
            matches!(
                acknowledge(&mut ledger, 4),
                HostEffect::Interaction(InteractReq::InvButton { id, bank_generation, .. })
                    if id == shrimp && bank_generation == second_generation
            ),
            "the same products sweep resumes in the new session"
        );

        snapshot.seed_inventory(fish_pack(shrimp, 0, net), 28);
        snapshot.seed_bank_observation(
            10,
            3,
            Some(vec![item(shrimp, "Raw shrimps", 27, ItemContainer::Bank)]),
            Vec::new(),
        );
        let settled = with_tick(&snapshot, &mut ledger, 5, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(
            matches!(settled, Poll::Ready(Ok(BankReceipt { complete: true, .. }))),
            "{settled:?}"
        );
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }

    /// The reopen is bounded: after `MAX_BANK_REOPENS` re-opens the next
    /// loss is the terminal `bank closed before the transfer settled`, with
    /// no further click. Without a stand the first loss is terminal, as
    /// before.
    #[test]
    fn deposit_reopen_is_bounded_and_needs_a_stand() {
        if !api::test_support::game_data_available() {
            return;
        }
        let shrimp = 317;
        let net = 303;
        let stand = WorldTile {
            x: 2809,
            z: 3441,
            level: 0,
        };
        let (access, locs) = booth_trip(stand);
        let open = |generation: u64, snapshot: &mut GameSnapshot| {
            snapshot.seed_bank_observation(10, generation, Some(Vec::new()), fish_side(shrimp, 27));
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(stand));
        snapshot.seed_locs(locs.clone());
        snapshot.seed_inventory(fish_pack(shrimp, 27, net), 28);
        open(1, &mut snapshot);
        let mut ledger = None;
        let (handle, poll) = begin_bank(
            &snapshot,
            &mut ledger,
            reopen_deposit_request(shrimp, net, &access),
        );
        assert!(poll.is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::InvButton { id, .. }) if id == shrimp
        ));
        let mut tick = 1;
        for reopen in 1..=MAX_BANK_REOPENS {
            tick += 1;
            snapshot.seed_bank_observation(-1, 1, None, Vec::new());
            let lost = with_tick(&snapshot, &mut ledger, tick, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            });
            assert!(lost.is_pending(), "reopen {reopen}: {lost:?}");
            assert!(
                matches!(
                    acknowledge(&mut ledger, tick),
                    HostEffect::Interaction(InteractReq::OpenStand { .. })
                ),
                "reopen {reopen} clicks the stand"
            );
            tick += 1;
            open(u64::from(reopen) + 1, &mut snapshot);
            let resumed = with_tick(&snapshot, &mut ledger, tick, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            });
            assert!(resumed.is_pending(), "reopen {reopen}: {resumed:?}");
            assert!(
                matches!(
                    acknowledge(&mut ledger, tick),
                    HostEffect::Interaction(InteractReq::InvButton { id, .. }) if id == shrimp
                ),
                "reopen {reopen} resumes the deposit"
            );
        }
        tick += 1;
        snapshot.seed_bank_observation(-1, 1, None, Vec::new());
        let exhausted = with_tick(&snapshot, &mut ledger, tick, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(
            failed_with(&exhausted, "bank closed before the transfer settled"),
            "{exhausted:?}"
        );
        assert!(ledger.as_ref().unwrap().outbox.is_empty());

        // No stand recorded: the first loss stays terminal.
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(stand));
        snapshot.seed_locs(locs);
        snapshot.seed_inventory(fish_pack(shrimp, 27, net), 28);
        open(1, &mut snapshot);
        let mut ledger = None;
        let (handle, poll) = begin_bank(
            &snapshot,
            &mut ledger,
            BankRequest {
                access: None,
                ..reopen_deposit_request(shrimp, net, &access)
            },
        );
        assert!(poll.is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::InvButton { id, .. }) if id == shrimp
        ));
        snapshot.seed_bank_observation(-1, 1, None, Vec::new());
        let lost = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(
            failed_with(&lost, "bank closed before the transfer settled"),
            "{lost:?}"
        );
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }

    /// HOTFIX-0.2.1: the recorded Catherby order behind `bank item table
    /// unavailable` from a deposit. The deposit begins on an observation
    /// whose interface is still open but whose withdraw table has stopped
    /// transmitting (the engine's close runs `inv_stoptransmit` before its
    /// `IF_CLOSE` reaches the client, bank.rs2:26-27); the close follows.
    /// The old kernel waited the whole eight-tick table bound inside the
    /// closed bank and then failed; a trip that knows its stand clicks the
    /// booth as soon as the session is gone, and still re-opens at the bound
    /// when an open never loads.
    #[test]
    fn deposit_reopens_when_the_bank_closes_or_never_loads_before_its_table() {
        if !api::test_support::game_data_available() {
            return;
        }
        let shrimp = 317;
        let net = 303;
        let stand = WorldTile {
            x: 2809,
            z: 3441,
            level: 0,
        };
        let (access, locs) = booth_trip(stand);
        let begin_open_unloaded = |locs: Vec<api::snapshot::LocView>| {
            let mut snapshot = GameSnapshot::new();
            snapshot.seed_ingame(2);
            snapshot.seed_local_player(local_player(stand));
            snapshot.seed_locs(locs);
            snapshot.seed_inventory(fish_pack(shrimp, 27, net), 28);
            // Interface up, table not loaded: `rows: None`.
            snapshot.seed_bank_observation(10, 1, None, fish_side(shrimp, 27));
            snapshot
        };

        // Closed before the table: the reopen goes out on the next poll.
        let mut snapshot = begin_open_unloaded(locs.clone());
        let mut ledger = None;
        let (handle, poll) = begin_bank(
            &snapshot,
            &mut ledger,
            reopen_deposit_request(shrimp, net, &access),
        );
        assert!(poll.is_pending());
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
        snapshot.seed_bank_observation(-1, 1, None, Vec::new());
        let closed = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(closed.is_pending(), "{closed:?}");
        assert!(
            matches!(
                acknowledge(&mut ledger, 2),
                HostEffect::Interaction(InteractReq::OpenStand { x, z, .. })
                    if x == stand.x && z == stand.z
            ),
            "the stand is clicked as soon as the unloaded bank is gone"
        );
        snapshot.seed_bank_observation(10, 2, Some(Vec::new()), fish_side(shrimp, 27));
        let generation = snapshot.bank_session_generation();
        let resumed = with_tick(&snapshot, &mut ledger, 4, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(resumed.is_pending(), "{resumed:?}");
        assert!(matches!(
            acknowledge(&mut ledger, 4),
            HostEffect::Interaction(InteractReq::InvButton { id, bank_generation, .. })
                if id == shrimp && bank_generation == generation
        ));

        // Open but never loading: the stand is clicked at the table bound
        // (the engine closes and re-opens on that click), not failed.
        let snapshot = begin_open_unloaded(locs);
        let mut ledger = None;
        let (handle, poll) = begin_bank(
            &snapshot,
            &mut ledger,
            reopen_deposit_request(shrimp, net, &access),
        );
        assert!(poll.is_pending());
        for tick in 2..=8 {
            let waiting = with_tick(&snapshot, &mut ledger, tick, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            });
            assert!(waiting.is_pending(), "tick {tick}: {waiting:?}");
            assert!(ledger.as_ref().unwrap().outbox.is_empty(), "tick {tick}");
        }
        let bound = with_tick(&snapshot, &mut ledger, 9, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(bound.is_pending(), "{bound:?}");
        assert!(matches!(
            acknowledge(&mut ledger, 9),
            HostEffect::Interaction(InteractReq::OpenStand { .. })
        ));
    }

    /// The open request keeps its own contract: a bank that shuts while its
    /// table is awaited keeps waiting on the open bound and fails as before,
    /// so the Gatherer's bounded stand retry still owns that case.
    #[test]
    fn open_stand_still_fails_on_its_bound_when_the_table_never_arrives() {
        if !api::test_support::game_data_available() {
            return;
        }
        let stand = WorldTile {
            x: 2809,
            z: 3441,
            level: 0,
        };
        let (access, locs) = booth_trip(stand);
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(stand));
        snapshot.seed_locs(locs);
        snapshot.seed_inventory(Vec::new(), 28);
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
            HostEffect::Interaction(InteractReq::OpenStand { .. })
        ));
        snapshot.seed_bank_observation(10, 1, None, Vec::new());
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        snapshot.seed_bank_observation(-1, 1, None, Vec::new());
        for tick in 3..=9 {
            let waiting = with_tick(&snapshot, &mut ledger, tick, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            });
            assert!(waiting.is_pending(), "tick {tick}: {waiting:?}");
        }
        let bound = with_tick(&snapshot, &mut ledger, 10, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(
            failed_with(&bound, "bank item table unavailable"),
            "{bound:?}"
        );
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }

    /// A soft reconnect keeps the client's modal fields and drops the item
    /// table (`client.rs:3735-3758`): the new session's open request finds a modal
    /// that is up with no table and no access of its own behind it. It waits
    /// the existing table bound, then accesses the stand once (the engine
    /// closes and re-opens on that click, `OpLocHandler.ts:53`) instead of
    /// failing, and the fresh table settles it. 0.2.8 failed at the bound.
    #[test]
    fn open_stand_adopting_a_tableless_modal_accesses_the_stand_at_the_bound() {
        if !api::test_support::game_data_available() {
            return;
        }
        let stand = WorldTile {
            x: 2809,
            z: 3441,
            level: 0,
        };
        let (access, locs) = booth_trip(stand);
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(stand));
        snapshot.seed_locs(locs);
        snapshot.seed_inventory(Vec::new(), 28);
        // The new session's observation: interface up, table not loaded.
        snapshot.seed_bank_observation(10, 1, None, Vec::new());
        let mut ledger = None;
        let (handle, poll) = begin_bank(&snapshot, &mut ledger, open_request(access));
        assert!(poll.is_pending());
        for tick in 2..=8 {
            let waiting = with_tick(&snapshot, &mut ledger, tick, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            });
            assert!(waiting.is_pending(), "tick {tick}: {waiting:?}");
            assert!(
                ledger.as_ref().unwrap().outbox.is_empty(),
                "no click inside the table bound (tick {tick})"
            );
        }
        let bound = with_tick(&snapshot, &mut ledger, 9, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(
            bound.is_pending(),
            "an adopted modal without a table is recovered, not failed: {bound:?}"
        );
        assert!(
            matches!(
                acknowledge(&mut ledger, 9),
                HostEffect::Interaction(InteractReq::OpenStand { x, z, stand_op: Some(2), .. })
                    if x == stand.x && z == stand.z
            ),
            "the stand is accessed for real at the bound"
        );

        // The engine's close-then-open delivers a new session with its table.
        snapshot.seed_bank_observation(10, 2, Some(Vec::new()), Vec::new());
        let settled = with_tick(&snapshot, &mut ledger, 11, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(
            matches!(settled, Poll::Ready(Ok(BankReceipt { complete: true, .. }))),
            "{settled:?}"
        );
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }

    /// The same adopted modal, but the modal is gone before the bound (the
    /// login grant clears every modal id, `client.rs:3639-3674`): no access
    /// of this request is behind it, so the stand is accessed at once
    /// instead of waiting out the bound for a click nobody sent.
    #[test]
    fn open_stand_adopting_a_modal_that_closes_accesses_the_stand_at_once() {
        if !api::test_support::game_data_available() {
            return;
        }
        let stand = WorldTile {
            x: 2809,
            z: 3441,
            level: 0,
        };
        let (access, locs) = booth_trip(stand);
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(stand));
        snapshot.seed_locs(locs);
        snapshot.seed_inventory(Vec::new(), 28);
        snapshot.seed_bank_observation(10, 1, None, Vec::new());
        let mut ledger = None;
        let (handle, poll) = begin_bank(&snapshot, &mut ledger, open_request(access));
        assert!(poll.is_pending());
        snapshot.seed_bank_observation(-1, 1, None, Vec::new());
        let closed = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(closed.is_pending(), "{closed:?}");
        assert!(
            matches!(
                acknowledge(&mut ledger, 2),
                HostEffect::Interaction(InteractReq::OpenStand { x, z, .. })
                    if x == stand.x && z == stand.z
            ),
            "the stand is accessed as soon as the adopted modal is gone"
        );
    }

    /// The recovery is one access, not a loop: after the real access the
    /// open keeps its own contract, waits its bound and fails for its
    /// caller's bounded retry. Exactly one `OpenStand` effect is emitted.
    #[test]
    fn open_stand_adopting_a_tableless_modal_accesses_the_stand_once() {
        if !api::test_support::game_data_available() {
            return;
        }
        let stand = WorldTile {
            x: 2809,
            z: 3441,
            level: 0,
        };
        let (access, locs) = booth_trip(stand);
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(stand));
        snapshot.seed_locs(locs);
        snapshot.seed_inventory(Vec::new(), 28);
        snapshot.seed_bank_observation(10, 1, None, Vec::new());
        let mut ledger = None;
        let (handle, poll) = begin_bank(&snapshot, &mut ledger, open_request(access));
        assert!(poll.is_pending());
        let mut clicks = 0;
        let mut outcome = None;
        for tick in 2..=40 {
            let polled = with_tick(&snapshot, &mut ledger, tick, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            });
            if !ledger.as_ref().unwrap().outbox.is_empty() {
                assert!(matches!(
                    acknowledge(&mut ledger, tick),
                    HostEffect::Interaction(InteractReq::OpenStand { .. })
                ));
                clicks += 1;
                continue;
            }
            if !polled.is_pending() {
                outcome = Some(polled);
                break;
            }
        }
        assert_eq!(clicks, 1, "one real access, then the bound");
        assert!(
            failed_with(
                outcome.as_ref().expect("the open fails on its bound"),
                "bank item table unavailable"
            ),
            "{outcome:?}"
        );
    }

    fn begin_deposit(
        snapshot: &GameSnapshot,
        ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
        args: DepositArgs,
    ) -> (
        crate::native::ActionHandle<Deposit>,
        Poll<Result<u32, ActionError>>,
    ) {
        with_tick(snapshot, ledger, 1, |tick| {
            let handle = tick.actions.begin::<Deposit>(args, &mut tick.cx).unwrap();
            let poll = tick.actions.poll(&handle, &mut tick.cx);
            (handle, poll)
        })
    }

    fn slotted(item: ItemView, slot: i32) -> ItemView {
        ItemView { slot, ..item }
    }

    fn deposit_side_row(id: i32, name: &str, count: i32, slot: i32) -> ItemView {
        row_with(
            item(id, name, count, ItemContainer::BankSide),
            &["Deposit-1", "Deposit-5", "Deposit-10", "Deposit-All"],
            slot,
        )
    }

    /// HOTFIX-0.2.1 review F1: the recorded Catherby order behind `bank
    /// closed before the transfer settled` (`E/base/soak-d.log:1456-1478`).
    /// The Deposit-All click goes out in session G; the next observation
    /// still shows the interface with its table stopped (the engine's close
    /// writes `inv_stoptransmit` at once, bank.rs2:26-27) and the pack
    /// unchanged; the next one has the roots closed and the pack already
    /// net-only (the deposit's `UPDATE_INV` and the `IF_CLOSE` leave in one
    /// flush, `World.ts:1116-1123`). The transfer re-opens its stand and,
    /// in the new session, credits the 27 that left before the close exactly
    /// once, without another click.
    #[test]
    fn deposit_applied_before_the_close_is_credited_once_after_the_reopen() {
        if !api::test_support::game_data_available() {
            return;
        }
        let shrimp = 317;
        let net = 303;
        let stand = WorldTile {
            x: 2809,
            z: 3441,
            level: 0,
        };
        let (access, locs) = booth_trip(stand);
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(stand));
        snapshot.seed_locs(locs);
        snapshot.seed_inventory(fish_pack(shrimp, 27, net), 28);
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), fish_side(shrimp, 27));
        let mut ledger = None;
        let (handle, poll) = begin_deposit(
            &snapshot,
            &mut ledger,
            DepositArgs {
                products: Arc::from([shrimp]),
                keep: Arc::from([net]),
                access: Some(Arc::clone(&access)),
            },
        );
        assert!(poll.is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::InvButton { id, .. }) if id == shrimp
        ));

        // Stop-transmit observed with the interface still open, pack 28.
        snapshot.seed_bank_observation(10, 1, None, fish_side(shrimp, 27));
        let stopped = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(stopped.is_pending(), "{stopped:?}");
        assert!(ledger.as_ref().unwrap().outbox.is_empty());

        // IF_CLOSE observed, pack already net-only: the deposit applied.
        snapshot.seed_inventory(fish_pack(shrimp, 0, net), 28);
        snapshot.seed_bank_observation(-1, 1, None, Vec::new());
        let lost = with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(lost.is_pending(), "{lost:?}");
        assert!(matches!(
            acknowledge(&mut ledger, 3),
            HostEffect::Interaction(InteractReq::OpenStand { x, z, .. })
                if x == stand.x && z == stand.z
        ));

        // The new session's table holds the banked shrimp; nothing to sweep.
        snapshot.seed_bank_observation(
            10,
            2,
            Some(vec![item(shrimp, "Raw shrimps", 27, ItemContainer::Bank)]),
            Vec::new(),
        );
        let settled = with_tick(&snapshot, &mut ledger, 4, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(
            matches!(settled, Poll::Ready(Ok(27))),
            "the applied deposit is counted exactly once: {settled:?}"
        );
        assert!(
            ledger.as_ref().unwrap().outbox.is_empty(),
            "no second deposit click"
        );
    }

    /// A deposit the engine applied only in part before the close (the
    /// count fell 27 → 22) credits the 5 that left and resumes the sweep
    /// for the 22 still held; the output is the whole 27.
    #[test]
    fn deposit_partly_applied_before_the_close_resumes_the_remainder() {
        if !api::test_support::game_data_available() {
            return;
        }
        let shrimp = 317;
        let net = 303;
        let stand = WorldTile {
            x: 2809,
            z: 3441,
            level: 0,
        };
        let (access, locs) = booth_trip(stand);
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(stand));
        snapshot.seed_locs(locs);
        snapshot.seed_inventory(fish_pack(shrimp, 27, net), 28);
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), fish_side(shrimp, 27));
        let mut ledger = None;
        let (handle, poll) = begin_deposit(
            &snapshot,
            &mut ledger,
            DepositArgs {
                products: Arc::from([shrimp]),
                keep: Arc::from([net]),
                access: Some(Arc::clone(&access)),
            },
        );
        assert!(poll.is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::InvButton { id, .. }) if id == shrimp
        ));

        snapshot.seed_inventory(fish_pack(shrimp, 22, net), 28);
        snapshot.seed_bank_observation(-1, 1, None, Vec::new());
        let lost = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(lost.is_pending(), "{lost:?}");
        assert!(matches!(
            acknowledge(&mut ledger, 2),
            HostEffect::Interaction(InteractReq::OpenStand { .. })
        ));

        snapshot.seed_bank_observation(
            10,
            2,
            Some(vec![item(shrimp, "Raw shrimps", 5, ItemContainer::Bank)]),
            fish_side(shrimp, 22),
        );
        let generation = snapshot.bank_session_generation();
        let resumed = with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(resumed.is_pending(), "{resumed:?}");
        assert!(
            matches!(
                acknowledge(&mut ledger, 3),
                HostEffect::Interaction(InteractReq::InvButton { id, bank_generation, .. })
                    if id == shrimp && bank_generation == generation
            ),
            "the sweep resumes for the 22 still held"
        );

        snapshot.seed_inventory(fish_pack(shrimp, 0, net), 28);
        snapshot.seed_bank_observation(
            10,
            3,
            Some(vec![item(shrimp, "Raw shrimps", 27, ItemContainer::Bank)]),
            Vec::new(),
        );
        let settled = with_tick(&snapshot, &mut ledger, 4, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(
            matches!(settled, Poll::Ready(Ok(27))),
            "5 credited at the reopen plus 22 settled in the new session: {settled:?}"
        );
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }

    /// Two products (the Catherby net catch): the shrimp deposit applied
    /// before the close, the anchovies were still held. The reopen credits
    /// the shrimp and sweeps only the anchovies; the output is the whole
    /// catch.
    #[test]
    fn deposit_sweep_resumes_the_unapplied_product_after_the_reopen() {
        if !api::test_support::game_data_available() {
            return;
        }
        let shrimp = 317;
        let anchovy = 321;
        let net = 303;
        let stand = WorldTile {
            x: 2809,
            z: 3441,
            level: 0,
        };
        let (access, locs) = booth_trip(stand);
        let net_row = || {
            slotted(
                item(net, "Small fishing net", 1, ItemContainer::Inventory),
                0,
            )
        };
        let shrimp_row = || slotted(item(shrimp, "Raw shrimps", 20, ItemContainer::Inventory), 1);
        let anchovy_row = || {
            slotted(
                item(anchovy, "Raw anchovies", 7, ItemContainer::Inventory),
                2,
            )
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(stand));
        snapshot.seed_locs(locs);
        snapshot.seed_inventory(vec![net_row(), shrimp_row(), anchovy_row()], 28);
        snapshot.seed_bank_observation(
            10,
            1,
            Some(Vec::new()),
            vec![
                deposit_side_row(shrimp, "Raw shrimps", 20, 1),
                deposit_side_row(anchovy, "Raw anchovies", 7, 2),
            ],
        );
        let mut ledger = None;
        let (handle, poll) = begin_deposit(
            &snapshot,
            &mut ledger,
            DepositArgs {
                products: Arc::from([shrimp, anchovy]),
                keep: Arc::from([net]),
                access: Some(Arc::clone(&access)),
            },
        );
        assert!(poll.is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::InvButton { id, .. }) if id == shrimp
        ));

        snapshot.seed_inventory(vec![net_row(), anchovy_row()], 28);
        snapshot.seed_bank_observation(-1, 1, None, Vec::new());
        let lost = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(lost.is_pending(), "{lost:?}");
        assert!(matches!(
            acknowledge(&mut ledger, 2),
            HostEffect::Interaction(InteractReq::OpenStand { .. })
        ));

        snapshot.seed_bank_observation(
            10,
            2,
            Some(vec![item(shrimp, "Raw shrimps", 20, ItemContainer::Bank)]),
            vec![deposit_side_row(anchovy, "Raw anchovies", 7, 2)],
        );
        let resumed = with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(resumed.is_pending(), "{resumed:?}");
        assert!(
            matches!(
                acknowledge(&mut ledger, 3),
                HostEffect::Interaction(InteractReq::InvButton { id, .. }) if id == anchovy
            ),
            "only the anchovies are still to deposit"
        );

        snapshot.seed_inventory(vec![net_row()], 28);
        snapshot.seed_bank_observation(
            10,
            3,
            Some(vec![
                item(shrimp, "Raw shrimps", 20, ItemContainer::Bank),
                item(anchovy, "Raw anchovies", 7, ItemContainer::Bank),
            ]),
            Vec::new(),
        );
        let settled = with_tick(&snapshot, &mut ledger, 4, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(matches!(settled, Poll::Ready(Ok(27))), "{settled:?}");
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }

    /// HOTFIX-0.2.1 review F1: a capacity delta (`slots: 1`) whose Deposit-1
    /// applied before the close is already delivered. The reopen credits
    /// the freed row from the new session's pack and completes, instead of
    /// repeating the delta on the next row.
    #[test]
    fn capacity_deposit_applied_before_the_close_is_not_repeated() {
        if !api::test_support::game_data_available() {
            return;
        }
        let shrimp = 317;
        let anchovy = 321;
        let net = 303;
        let stand = WorldTile {
            x: 2809,
            z: 3441,
            level: 0,
        };
        let (access, locs) = booth_trip(stand);
        let net_row = || {
            slotted(
                item(net, "Small fishing net", 1, ItemContainer::Inventory),
                0,
            )
        };
        let shrimp_row = || slotted(item(shrimp, "Raw shrimps", 1, ItemContainer::Inventory), 1);
        let anchovy_row = || {
            slotted(
                item(anchovy, "Raw anchovies", 1, ItemContainer::Inventory),
                2,
            )
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(stand));
        snapshot.seed_locs(locs);
        snapshot.seed_inventory(vec![net_row(), shrimp_row(), anchovy_row()], 28);
        snapshot.seed_bank_observation(
            10,
            1,
            Some(Vec::new()),
            vec![
                deposit_side_row(shrimp, "Raw shrimps", 1, 1),
                deposit_side_row(anchovy, "Raw anchovies", 1, 2),
            ],
        );
        let mut ledger = None;
        let (handle, poll) = begin_bank(
            &snapshot,
            &mut ledger,
            BankRequest {
                bank: None,
                action: BankAction::DepositCapacity {
                    keep: Arc::from([net]),
                    extra_keep: None,
                    slots: 1,
                },
                partial_ok: false,
                access: Some(Arc::clone(&access)),
            },
        );
        assert!(poll.is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::InvButton { id, .. }) if id == shrimp
        ));

        // The shrimp row left the pack together with the close.
        snapshot.seed_inventory(vec![net_row(), anchovy_row()], 28);
        snapshot.seed_bank_observation(-1, 1, None, Vec::new());
        let lost = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(lost.is_pending(), "{lost:?}");
        assert!(matches!(
            acknowledge(&mut ledger, 2),
            HostEffect::Interaction(InteractReq::OpenStand { .. })
        ));

        snapshot.seed_bank_observation(
            10,
            2,
            Some(vec![item(shrimp, "Raw shrimps", 1, ItemContainer::Bank)]),
            vec![deposit_side_row(anchovy, "Raw anchovies", 1, 2)],
        );
        let settled = with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(
            matches!(settled, Poll::Ready(Ok(BankReceipt { complete: true, .. }))),
            "one row was already freed; the delta is delivered: {settled:?}"
        );
        assert!(
            ledger.as_ref().unwrap().outbox.is_empty(),
            "no second row is deposited"
        );
    }

    /// HOTFIX-0.2.1 review r2 N2: a two-slot capacity delta whose first
    /// Deposit-1 (shrimp) applied before the close. The reopen credits that
    /// freed row once: one of two rows is not the delta, so the anchovy row
    /// still goes out, and the delta completes only once that row has left.
    #[test]
    fn capacity_two_slots_credit_the_row_applied_before_the_close_once() {
        if !api::test_support::game_data_available() {
            return;
        }
        let shrimp = 317;
        let anchovy = 321;
        let net = 303;
        let stand = WorldTile {
            x: 2809,
            z: 3441,
            level: 0,
        };
        let (access, locs) = booth_trip(stand);
        let net_row = || {
            slotted(
                item(net, "Small fishing net", 1, ItemContainer::Inventory),
                0,
            )
        };
        let shrimp_row = || slotted(item(shrimp, "Raw shrimps", 1, ItemContainer::Inventory), 1);
        let anchovy_row = || {
            slotted(
                item(anchovy, "Raw anchovies", 1, ItemContainer::Inventory),
                2,
            )
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(stand));
        snapshot.seed_locs(locs);
        snapshot.seed_inventory(vec![net_row(), shrimp_row(), anchovy_row()], 28);
        snapshot.seed_bank_observation(
            10,
            1,
            Some(Vec::new()),
            vec![
                deposit_side_row(shrimp, "Raw shrimps", 1, 1),
                deposit_side_row(anchovy, "Raw anchovies", 1, 2),
            ],
        );
        let mut ledger = None;
        let (handle, poll) = begin_bank(
            &snapshot,
            &mut ledger,
            BankRequest {
                bank: None,
                action: BankAction::DepositCapacity {
                    keep: Arc::from([net]),
                    extra_keep: None,
                    slots: 2,
                },
                partial_ok: false,
                access: Some(Arc::clone(&access)),
            },
        );
        assert!(poll.is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::InvButton { id, .. }) if id == shrimp
        ));

        // The shrimp row left the pack together with the close.
        snapshot.seed_inventory(vec![net_row(), anchovy_row()], 28);
        snapshot.seed_bank_observation(-1, 1, None, Vec::new());
        let lost = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(lost.is_pending(), "{lost:?}");
        assert!(matches!(
            acknowledge(&mut ledger, 2),
            HostEffect::Interaction(InteractReq::OpenStand { .. })
        ));

        snapshot.seed_bank_observation(
            10,
            2,
            Some(vec![item(shrimp, "Raw shrimps", 1, ItemContainer::Bank)]),
            vec![deposit_side_row(anchovy, "Raw anchovies", 1, 2)],
        );
        let resumed = with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(
            resumed.is_pending(),
            "one freed row is not the requested two: {resumed:?}"
        );
        assert!(matches!(
            acknowledge(&mut ledger, 3),
            HostEffect::Interaction(InteractReq::InvButton { id, .. }) if id == anchovy
        ));

        snapshot.seed_inventory(vec![net_row()], 28);
        snapshot.seed_bank_observation(
            10,
            3,
            Some(vec![
                item(shrimp, "Raw shrimps", 1, ItemContainer::Bank),
                item(anchovy, "Raw anchovies", 1, ItemContainer::Bank),
            ]),
            Vec::new(),
        );
        let settled = with_tick(&snapshot, &mut ledger, 4, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(
            matches!(settled, Poll::Ready(Ok(BankReceipt { complete: true, .. }))),
            "the shrimp row and the anchovy row are the two slots: {settled:?}"
        );
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }

    /// An exact withdrawal (held target 2) applied before the close: the new
    /// session reads the held count again and owes nothing, so no second
    /// Withdraw-X goes out.
    #[test]
    fn exact_withdrawal_applied_before_the_close_is_not_repeated() {
        if !api::test_support::game_data_available() {
            return;
        }
        let bait = 314;
        let stand = WorldTile {
            x: 2809,
            z: 3441,
            level: 0,
        };
        let (access, locs) = booth_trip(stand);
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(stand));
        snapshot.seed_locs(locs);
        snapshot.seed_inventory(Vec::new(), 28);
        snapshot.seed_bank_observation(
            10,
            1,
            Some(vec![item(bait, "Bait", 5, ItemContainer::Bank)]),
            Vec::new(),
        );
        let mut ledger = None;
        let (handle, poll) = begin_bank(
            &snapshot,
            &mut ledger,
            BankRequest {
                bank: None,
                action: BankAction::WithdrawTo {
                    withdrawals: Arc::from([Withdrawal {
                        id: bait,
                        name: Arc::from("Bait"),
                        target: 2,
                    }]),
                },
                partial_ok: false,
                access: Some(Arc::clone(&access)),
            },
        );
        assert!(poll.is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::SetNoteMode { on: false })
        ));
        let clicked = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(clicked.is_pending(), "{clicked:?}");
        assert!(matches!(
            acknowledge(&mut ledger, 2),
            HostEffect::Interaction(InteractReq::WithdrawX { count: 2, bank_item_id, .. })
                if bank_item_id == bait
        ));

        // The withdrawal landed, then the bank closed.
        snapshot.seed_inventory(vec![item(bait, "Bait", 2, ItemContainer::Inventory)], 28);
        snapshot.seed_bank_observation(-1, 1, None, Vec::new());
        let lost = with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(lost.is_pending(), "{lost:?}");
        assert!(matches!(
            acknowledge(&mut ledger, 3),
            HostEffect::Interaction(InteractReq::OpenStand { .. })
        ));

        snapshot.seed_bank_observation(
            10,
            2,
            Some(vec![item(bait, "Bait", 3, ItemContainer::Bank)]),
            Vec::new(),
        );
        let settled = with_tick(&snapshot, &mut ledger, 4, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(
            matches!(settled, Poll::Ready(Ok(BankReceipt { complete: true, .. }))),
            "the held target is already met: {settled:?}"
        );
        assert!(
            ledger.as_ref().unwrap().outbox.is_empty(),
            "no second withdrawal delta"
        );
    }

    mod recovery {
        use super::*;

        const PRODUCT: i32 = 317;
        const TOOL: i32 = 303;
        type Ledger = Option<Box<crate::native::ledger::Ledger>>;
        type Handle = crate::native::ActionHandle<Deposit>;

        fn fixture() -> (GameSnapshot, Ledger, Handle) {
            let mut snapshot = GameSnapshot::new();
            snapshot.seed_ingame(2);
            snapshot.seed_inventory(fish_pack(PRODUCT, 2, TOOL), 28);
            snapshot.seed_inventory_packet_generations(Some(7), Some(7));
            snapshot.seed_bank_observation(10, 1, Some(Vec::new()), fish_side(PRODUCT, 2));
            let mut ledger = None;
            let (handle, poll) = begin_deposit(
                &snapshot,
                &mut ledger,
                DepositArgs {
                    products: Arc::from([PRODUCT]),
                    keep: Arc::from([TOOL]),
                    access: None,
                },
            );
            assert!(poll.is_pending());
            (snapshot, ledger, handle)
        }

        fn dispatch(ledger: &mut Ledger, tick: u64, accepted: bool) -> HostEffect {
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
                    accepted,
                    chat_since: 10,
                },
            );
            action.effect
        }

        fn poll(
            snapshot: &GameSnapshot,
            ledger: &mut Ledger,
            handle: &Handle,
            tick: u64,
        ) -> Poll<Result<u32, ActionError>> {
            with_tick(snapshot, ledger, tick, |frame| {
                frame.actions.poll(handle, &mut frame.cx)
            })
        }

        fn applied(snapshot: &mut GameSnapshot) {
            snapshot.seed_inventory(fish_pack(PRODUCT, 0, TOOL), 28);
            snapshot.seed_inventory_packet_generations(Some(8), Some(8));
            snapshot.seed_bank_observation(
                10,
                2,
                Some(vec![item(PRODUCT, "Raw shrimps", 2, ItemContainer::Bank)]),
                Vec::new(),
            );
        }

        #[test]
        fn refused_dispatch_recovers_immediately_without_crediting_acceptance() {
            if !api::test_support::game_data_available() {
                return;
            }
            let (mut snapshot, mut ledger, handle) = fixture();
            dispatch(&mut ledger, 1, false);
            assert!(poll(&snapshot, &mut ledger, &handle, 2).is_pending());
            assert!(ledger.as_ref().unwrap().outbox.is_empty());
            assert!(poll(&snapshot, &mut ledger, &handle, 3).is_pending());
            assert!(matches!(
                dispatch(&mut ledger, 3, true),
                HostEffect::Interaction(InteractReq::InvButton { id: PRODUCT, .. })
            ));
            assert!(
                poll(&snapshot, &mut ledger, &handle, 4).is_pending(),
                "an accepted retry is not a transfer"
            );
            applied(&mut snapshot);
            assert!(matches!(
                poll(&snapshot, &mut ledger, &handle, 5),
                Poll::Ready(Ok(2))
            ));
        }

        #[test]
        fn transfer_boundary_recovered_noop_returns_at_reconcile_end() {
            if !api::test_support::game_data_available() {
                return;
            }
            let (snapshot, mut ledger, handle) = fixture();
            assert!(matches!(
                dispatch(&mut ledger, 1, true),
                HostEffect::Interaction(InteractReq::InvButton { id: PRODUCT, .. })
            ));
            assert!(poll(&snapshot, &mut ledger, &handle, 9).is_pending());
            assert!(handle
                .inspect(|machine| machine.transfer_in_flight())
                .unwrap_or(false));
            with_tick(&snapshot, &mut ledger, 10, |tick| {
                tick.actions.interrupt(&handle);
            });
            assert!(matches!(
                poll(&snapshot, &mut ledger, &handle, 10),
                Poll::Ready(Err(ActionError::Cancelled))
            ));
            assert!(
                ledger.as_ref().unwrap().outbox.is_empty(),
                "the interrupted reconcile must not dispatch a recovered retry"
            );
        }

        #[test]
        fn accepted_noop_rescans_once_then_blocks_with_its_typed_reason() {
            if !api::test_support::game_data_available() {
                return;
            }
            let (snapshot, mut ledger, handle) = fixture();
            dispatch(&mut ledger, 1, true);
            assert!(poll(&snapshot, &mut ledger, &handle, 9).is_pending());
            assert!(ledger.as_ref().unwrap().outbox.is_empty());
            assert!(poll(&snapshot, &mut ledger, &handle, 10).is_pending());
            dispatch(&mut ledger, 10, true);
            assert!(matches!(
                poll(&snapshot, &mut ledger, &handle, 18),
                Poll::Ready(Err(ActionError::BankDeposit(
                    ops::DepositFailure::NoCountFall
                )))
            ));
            assert!(
                ledger.as_ref().unwrap().outbox.is_empty(),
                "no third attempt"
            );
        }

        #[test]
        fn delayed_inventory_revision_reconciles_before_retry_without_duplicate_credit() {
            if !api::test_support::game_data_available() {
                return;
            }
            for bank_first in [true, false] {
                let (mut snapshot, mut ledger, handle) = fixture();
                dispatch(&mut ledger, 1, true);
                // Either transfer table can arrive before the held inventory.
                let (bank, side) = if bank_first {
                    (
                        vec![item(PRODUCT, "Raw shrimps", 2, ItemContainer::Bank)],
                        fish_side(PRODUCT, 2),
                    )
                } else {
                    (Vec::new(), Vec::new())
                };
                snapshot.seed_bank_observation(10, 2, Some(bank), side);
                assert_eq!(snapshot.inventory_packet_generation(), Some(7));
                assert!(poll(&snapshot, &mut ledger, &handle, 9).is_pending());
                assert!(poll(&snapshot, &mut ledger, &handle, 10).is_pending());
                assert!(
                    ledger.as_ref().unwrap().outbox.is_empty(),
                    "a fresh snapshot with delayed held inventory cannot replay the old slot"
                );
                applied(&mut snapshot);
                assert!(matches!(
                    poll(&snapshot, &mut ledger, &handle, 11),
                    Poll::Ready(Ok(2))
                ));
                assert!(ledger.as_ref().unwrap().outbox.is_empty(), "no stale retry");
            }
        }

        #[test]
        fn earlier_transfer_succeeds_before_retry_and_next_product_is_rescanned() {
            if !api::test_support::game_data_available() {
                return;
            }
            let (mut snapshot, _, old_handle) = fixture();
            drop(old_handle);
            let second = 321;
            let mut pack = fish_pack(PRODUCT, 2, TOOL);
            pack.push(slotted(
                item(second, "Raw anchovies", 1, ItemContainer::Inventory),
                5,
            ));
            snapshot.seed_inventory(pack, 28);
            let mut side = fish_side(PRODUCT, 2);
            side.push(deposit_side_row(second, "Raw anchovies", 1, 5));
            snapshot.seed_bank_observation(10, 1, Some(Vec::new()), side);
            let mut ledger = None;
            let (handle, sent) = begin_deposit(
                &snapshot,
                &mut ledger,
                DepositArgs {
                    products: Arc::from([PRODUCT, second]),
                    keep: Arc::from([TOOL]),
                    access: None,
                },
            );
            assert!(sent.is_pending());
            dispatch(&mut ledger, 1, true);
            assert!(poll(&snapshot, &mut ledger, &handle, 9).is_pending());
            // The old transfer lands between timeout and the fresh observation.
            // A second requested product remains, at a different live slot.
            snapshot.seed_inventory(
                vec![
                    item(TOOL, "Net", 1, ItemContainer::Inventory),
                    slotted(
                        item(second, "Raw anchovies", 1, ItemContainer::Inventory),
                        5,
                    ),
                ],
                28,
            );
            snapshot.seed_bank_observation(
                10,
                2,
                Some(vec![item(PRODUCT, "Raw shrimps", 2, ItemContainer::Bank)]),
                vec![deposit_side_row(second, "Raw anchovies", 1, 5)],
            );
            assert!(poll(&snapshot, &mut ledger, &handle, 10).is_pending());
            assert!(matches!(dispatch(&mut ledger, 10, true),
                HostEffect::Interaction(InteractReq::InvButton { id, slot: 5, .. }) if id == second));
            snapshot.seed_inventory(fish_pack(PRODUCT, 0, TOOL), 28);
            snapshot.seed_bank_observation(
                10,
                3,
                Some(vec![
                    item(PRODUCT, "Raw shrimps", 2, ItemContainer::Bank),
                    item(second, "Raw anchovies", 1, ItemContainer::Bank),
                ]),
                Vec::new(),
            );
            assert!(matches!(
                poll(&snapshot, &mut ledger, &handle, 11),
                Poll::Ready(Ok(3))
            ));
            assert!(ledger.as_ref().unwrap().outbox.is_empty());
        }

        #[test]
        fn side_slot_movement_after_refusal_uses_current_slot() {
            if !api::test_support::game_data_available() {
                return;
            }
            let (mut snapshot, mut ledger, handle) = fixture();
            dispatch(&mut ledger, 1, false);
            assert!(poll(&snapshot, &mut ledger, &handle, 2).is_pending());
            snapshot.seed_inventory(
                vec![
                    item(TOOL, "Net", 1, ItemContainer::Inventory),
                    slotted(item(PRODUCT, "Raw shrimps", 2, ItemContainer::Inventory), 6),
                ],
                28,
            );
            snapshot.seed_bank_observation(
                10,
                2,
                Some(Vec::new()),
                vec![deposit_side_row(PRODUCT, "Raw shrimps", 2, 6)],
            );
            assert!(poll(&snapshot, &mut ledger, &handle, 3).is_pending());
            assert!(matches!(
                dispatch(&mut ledger, 3, true),
                HostEffect::Interaction(InteractReq::InvButton {
                    id: PRODUCT,
                    slot: 6,
                    ..
                })
            ));
            applied(&mut snapshot);
            assert!(matches!(
                poll(&snapshot, &mut ledger, &handle, 4),
                Poll::Ready(Ok(2))
            ));
        }

        #[test]
        fn refusal_recovery_reuses_close_reopen_reconciliation() {
            if !api::test_support::game_data_available() {
                return;
            }
            let (mut snapshot, _, old_handle) = fixture();
            // Add a real recorded bank stand to the same shared machine request.
            drop(old_handle);
            let stand = WorldTile {
                x: 2809,
                z: 3441,
                level: 0,
            };
            let (access, locs) = booth_trip(stand);
            snapshot.seed_local_player(local_player(stand));
            snapshot.seed_locs(locs);
            let mut ledger = None;
            let (handle, sent) = begin_deposit(
                &snapshot,
                &mut ledger,
                DepositArgs {
                    products: Arc::from([PRODUCT]),
                    keep: Arc::from([TOOL]),
                    access: Some(access),
                },
            );
            assert!(sent.is_pending());
            dispatch(&mut ledger, 1, false);
            assert!(poll(&snapshot, &mut ledger, &handle, 2).is_pending());
            assert!(poll(&snapshot, &mut ledger, &handle, 3).is_pending());
            assert!(matches!(
                dispatch(&mut ledger, 3, true),
                HostEffect::Interaction(InteractReq::InvButton { id: PRODUCT, .. })
            ));
            snapshot.seed_bank_observation(-1, 1, None, Vec::new());
            assert!(poll(&snapshot, &mut ledger, &handle, 4).is_pending());
            assert!(matches!(
                dispatch(&mut ledger, 4, true),
                HostEffect::Interaction(InteractReq::OpenStand { .. })
            ));
            snapshot.seed_bank_observation(10, 2, Some(Vec::new()), fish_side(PRODUCT, 2));
            assert!(poll(&snapshot, &mut ledger, &handle, 5).is_pending());
            assert!(matches!(
                dispatch(&mut ledger, 5, true),
                HostEffect::Interaction(InteractReq::InvButton { id: PRODUCT, .. })
            ));
            applied(&mut snapshot);
            assert!(matches!(
                poll(&snapshot, &mut ledger, &handle, 6),
                Poll::Ready(Ok(2))
            ));
        }

        #[test]
        fn permanent_content_refusals_block_without_reissue() {
            if !api::test_support::game_data_available() {
                return;
            }
            let mut wrong = Vec::new();
            for (message, reason) in [
                (
                    "You already have a full stack of that item in the bank.",
                    ops::DepositFailure::FullStack,
                ),
                (
                    "You don't have enough space in your bank account.",
                    ops::DepositFailure::BankFull,
                ),
                (
                    "You need to subscribe for a members account if you want to",
                    ops::DepositFailure::BankFull,
                ),
                (
                    "store any more items in the bank.",
                    ops::DepositFailure::BankFull,
                ),
                (
                    "A magical force prevents you from banking this item!",
                    ops::DepositFailure::Unbankable,
                ),
            ] {
                let (mut snapshot, mut ledger, handle) = fixture();
                dispatch(&mut ledger, 1, true);
                snapshot.seed_chat_lines(vec![api::snapshot::ChatLineView {
                    type_: 0,
                    username: None,
                    text: message.to_owned(),
                    sequence: 11,
                }]);
                let result = poll(&snapshot, &mut ledger, &handle, 2);
                if !matches!(result,
                    Poll::Ready(Err(ActionError::BankDeposit(actual))) if actual == reason)
                {
                    wrong.push(format!("{message}: {result:?}"));
                }
                assert!(ledger.as_ref().unwrap().outbox.is_empty());
            }
            assert!(wrong.is_empty(), "content refusals: {wrong:?}");
        }

        #[test]
        fn saturated_stack_is_explicit_without_sending_a_futile_click() {
            if !api::test_support::game_data_available() {
                return;
            }
            let (mut snapshot, _, old_handle) = fixture();
            drop(old_handle);
            snapshot.seed_bank_observation(
                10,
                2,
                Some(vec![item(
                    PRODUCT,
                    "Raw shrimps",
                    i32::MAX,
                    ItemContainer::Bank,
                )]),
                fish_side(PRODUCT, 2),
            );
            let mut ledger = None;
            let (_, poll) = begin_deposit(
                &snapshot,
                &mut ledger,
                DepositArgs {
                    products: Arc::from([PRODUCT]),
                    keep: Arc::from([TOOL]),
                    access: None,
                },
            );
            assert!(matches!(
                poll,
                Poll::Ready(Err(ActionError::BankDeposit(
                    ops::DepositFailure::FullStack
                )))
            ));
            assert!(ledger.as_ref().unwrap().outbox.is_empty());
        }

        #[test]
        fn stale_or_player_refusal_text_does_not_override_transient_recovery() {
            if !api::test_support::game_data_available() {
                return;
            }
            let (mut snapshot, mut ledger, handle) = fixture();
            dispatch(&mut ledger, 1, false);
            snapshot.seed_chat_lines(vec![
                api::snapshot::ChatLineView {
                    type_: 0,
                    username: None,
                    sequence: 10,
                    text: "You don't have enough space in your bank account.".to_owned(),
                },
                api::snapshot::ChatLineView {
                    type_: 2,
                    username: Some("another player".to_owned()),
                    sequence: 11,
                    text: "You already have a full stack of that item in the bank.".to_owned(),
                },
            ]);
            assert!(poll(&snapshot, &mut ledger, &handle, 2).is_pending());
            assert!(poll(&snapshot, &mut ledger, &handle, 3).is_pending());
            dispatch(&mut ledger, 3, true);
            applied(&mut snapshot);
            assert!(matches!(
                poll(&snapshot, &mut ledger, &handle, 4),
                Poll::Ready(Ok(2))
            ));
        }
    }
    #[test]
    fn deposit_and_withdraw_wrappers_forward_interrupt_and_transfer_in_flight() {
        if !api::test_support::game_data_available() {
            return;
        }
        let shrimp = 317;
        let net = 303;
        let mut deposit_snapshot = GameSnapshot::new();
        deposit_snapshot.seed_ingame(2);
        deposit_snapshot.seed_inventory(fish_pack(shrimp, 2, net), 28);
        deposit_snapshot.seed_bank_observation(10, 1, Some(Vec::new()), fish_side(shrimp, 2));
        let mut deposit_ledger = None;
        let (deposit, started) = begin_deposit(
            &deposit_snapshot,
            &mut deposit_ledger,
            DepositArgs {
                products: Arc::from([shrimp]),
                keep: Arc::from([net]),
                access: None,
            },
        );
        assert!(started.is_pending());
        assert!(matches!(
            acknowledge(&mut deposit_ledger, 1),
            HostEffect::Interaction(InteractReq::InvButton { id, .. }) if id == shrimp
        ));
        assert!(deposit
            .inspect(|machine| machine.transfer_in_flight())
            .unwrap_or(false));
        with_tick(&deposit_snapshot, &mut deposit_ledger, 2, |tick| {
            tick.actions.interrupt(&deposit);
        });
        assert!(deposit
            .inspect(|machine| machine.transfer_in_flight())
            .unwrap_or(false));
        assert!(
            with_tick(&deposit_snapshot, &mut deposit_ledger, 3, |tick| {
                tick.actions.poll(&deposit, &mut tick.cx)
            })
            .is_pending()
        );
        deposit_snapshot.seed_inventory(fish_pack(shrimp, 0, net), 28);
        deposit_snapshot.seed_bank_observation(
            10,
            1,
            Some(vec![item(shrimp, "Raw shrimps", 2, ItemContainer::Bank)]),
            Vec::new(),
        );
        assert!(matches!(
            with_tick(&deposit_snapshot, &mut deposit_ledger, 4, |tick| {
                tick.actions.poll(&deposit, &mut tick.cx)
            }),
            Poll::Ready(Err(ActionError::Cancelled))
        ));
        assert!(deposit_ledger.as_ref().unwrap().outbox.is_empty());

        let bait = 314;
        let mut withdraw_snapshot = GameSnapshot::new();
        withdraw_snapshot.seed_ingame(2);
        withdraw_snapshot.seed_inventory(Vec::new(), 28);
        withdraw_snapshot.seed_bank_observation(
            10,
            1,
            Some(vec![item(bait, "Bait", 5, ItemContainer::Bank)]),
            Vec::new(),
        );
        let mut withdraw_ledger = None;
        let withdraw = with_tick(&withdraw_snapshot, &mut withdraw_ledger, 1, |tick| {
            let handle = tick
                .actions
                .begin::<Withdraw>(
                    WithdrawArgs {
                        withdrawals: Arc::from([Withdrawal {
                            id: bait,
                            name: Arc::from("Bait"),
                            target: 2,
                        }]),
                        access: None,
                    },
                    &mut tick.cx,
                )
                .unwrap();
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            handle
        });
        assert!(matches!(
            acknowledge(&mut withdraw_ledger, 1),
            HostEffect::Interaction(InteractReq::SetNoteMode { on: false })
        ));
        with_tick(&withdraw_snapshot, &mut withdraw_ledger, 1, |tick| {
            assert!(tick.actions.poll(&withdraw, &mut tick.cx).is_pending());
        });
        assert!(
            with_tick(&withdraw_snapshot, &mut withdraw_ledger, 2, |tick| {
                tick.actions.poll(&withdraw, &mut tick.cx)
            })
            .is_pending()
        );
        assert!(matches!(
            acknowledge(&mut withdraw_ledger, 2),
            HostEffect::Interaction(InteractReq::WithdrawX {
                bank_item_id,
                count: 2,
                ..
            }) if bank_item_id == bait
        ));
        assert!(withdraw
            .inspect(|machine| machine.transfer_in_flight())
            .unwrap_or(false));
        with_tick(&withdraw_snapshot, &mut withdraw_ledger, 3, |tick| {
            tick.actions.interrupt(&withdraw);
        });
        assert!(withdraw
            .inspect(|machine| machine.transfer_in_flight())
            .unwrap_or(false));
        assert!(
            with_tick(&withdraw_snapshot, &mut withdraw_ledger, 4, |tick| {
                tick.actions.poll(&withdraw, &mut tick.cx)
            })
            .is_pending()
        );
        withdraw_snapshot.seed_inventory(vec![item(bait, "Bait", 2, ItemContainer::Inventory)], 28);
        withdraw_snapshot.seed_bank_observation(
            10,
            2,
            Some(vec![item(bait, "Bait", 3, ItemContainer::Bank)]),
            Vec::new(),
        );
        assert!(matches!(
            with_tick(&withdraw_snapshot, &mut withdraw_ledger, 5, |tick| {
                tick.actions.poll(&withdraw, &mut tick.cx)
            }),
            Poll::Ready(Err(ActionError::Cancelled))
        ));
        assert!(withdraw_ledger.as_ref().unwrap().outbox.is_empty());
    }

    #[test]
    fn transfer_boundary_deposit_capacity() {
        if !api::test_support::game_data_available() {
            return;
        }
        let shrimp = 317;
        let anchovy = 321;
        let net = 303;
        let net_row = || {
            slotted(
                item(net, "Small fishing net", 1, ItemContainer::Inventory),
                0,
            )
        };
        let shrimp_row = || slotted(item(shrimp, "Raw shrimps", 1, ItemContainer::Inventory), 1);
        let anchovy_row = || {
            slotted(
                item(anchovy, "Raw anchovies", 1, ItemContainer::Inventory),
                2,
            )
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(vec![net_row(), shrimp_row(), anchovy_row()], 28);
        snapshot.seed_bank_observation(
            10,
            1,
            Some(Vec::new()),
            vec![
                deposit_side_row(shrimp, "Raw shrimps", 1, 1),
                deposit_side_row(anchovy, "Raw anchovies", 1, 2),
            ],
        );
        let mut ledger = None;
        let (handle, started) = begin_bank(
            &snapshot,
            &mut ledger,
            BankRequest {
                bank: None,
                action: BankAction::DepositCapacity {
                    keep: Arc::from([net]),
                    extra_keep: None,
                    slots: 1,
                },
                partial_ok: false,
                access: None,
            },
        );
        assert!(started.is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::InvButton { id, .. }) if id == shrimp
        ));
        assert!(handle
            .inspect(|machine| machine.transfer_in_flight())
            .unwrap_or(false));
        with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.interrupt(&handle);
        });
        assert!(handle
            .inspect(|machine| machine.transfer_in_flight())
            .unwrap_or(false));

        snapshot.seed_inventory(vec![net_row(), anchovy_row()], 28);
        snapshot.seed_bank_observation(
            10,
            1,
            Some(vec![item(shrimp, "Raw shrimps", 1, ItemContainer::Bank)]),
            vec![deposit_side_row(anchovy, "Raw anchovies", 1, 2)],
        );
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 3, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            }),
            Poll::Ready(Err(ActionError::Cancelled))
        ));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }

    #[test]
    fn transfer_boundary_withdraw_list() {
        if !api::test_support::game_data_available() {
            return;
        }
        let bait = 314;
        let coin = 995;
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(Vec::new(), 28);
        snapshot.seed_bank_observation(
            10,
            1,
            Some(vec![
                item(bait, "Bait", 5, ItemContainer::Bank),
                item(coin, "Coins", 10, ItemContainer::Bank),
            ]),
            Vec::new(),
        );
        let mut ledger = None;
        let (handle, started) = begin_bank(
            &snapshot,
            &mut ledger,
            bank_request(
                BankAction::WithdrawTo {
                    withdrawals: Arc::from([
                        Withdrawal {
                            id: bait,
                            name: Arc::from("Bait"),
                            target: 2,
                        },
                        Withdrawal {
                            id: coin,
                            name: Arc::from("Coins"),
                            target: 5,
                        },
                    ]),
                },
                false,
            ),
        );
        assert!(started.is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 1),
            HostEffect::Interaction(InteractReq::SetNoteMode { on: false })
        ));
        with_tick(&snapshot, &mut ledger, 1, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 2),
            HostEffect::Interaction(InteractReq::WithdrawX {
                bank_item_id,
                count: 2,
                ..
            }) if bank_item_id == bait
        ));
        assert!(handle
            .inspect(|machine| machine.transfer_in_flight())
            .unwrap_or(false));
        with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.interrupt(&handle);
        });
        assert!(with_tick(&snapshot, &mut ledger, 4, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());

        snapshot.seed_inventory(vec![item(bait, "Bait", 2, ItemContainer::Inventory)], 28);
        snapshot.seed_bank_observation(
            10,
            2,
            Some(vec![
                item(bait, "Bait", 3, ItemContainer::Bank),
                item(coin, "Coins", 10, ItemContainer::Bank),
            ]),
            Vec::new(),
        );
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 5, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            }),
            Poll::Ready(Err(ActionError::Cancelled))
        ));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }

    #[test]
    fn transfer_boundary_starts_no_bank_action_in_same_poll() {
        if !api::test_support::game_data_available() {
            return;
        }
        let bait = 314;
        let coin = 995;
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(Vec::new(), 28);
        snapshot.seed_bank_observation(
            10,
            1,
            Some(vec![
                item(bait, "Bait", 5, ItemContainer::Bank),
                item(coin, "Coins", 10, ItemContainer::Bank),
            ]),
            Vec::new(),
        );
        let mut ledger = None;
        let (handle, started) = begin_bank(
            &snapshot,
            &mut ledger,
            bank_request(
                BankAction::WithdrawTo {
                    withdrawals: Arc::from([
                        Withdrawal {
                            id: bait,
                            name: Arc::from("Bait"),
                            target: 2,
                        },
                        Withdrawal {
                            id: coin,
                            name: Arc::from("Coins"),
                            target: 5,
                        },
                    ]),
                },
                false,
            ),
        );
        assert!(started.is_pending());
        let _ = acknowledge(&mut ledger, 1);
        with_tick(&snapshot, &mut ledger, 1, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        let _ = acknowledge(&mut ledger, 2);
        with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.interrupt(&handle);
        });
        snapshot.seed_inventory(vec![item(bait, "Bait", 2, ItemContainer::Inventory)], 28);
        snapshot.seed_bank_observation(
            10,
            2,
            Some(vec![
                item(bait, "Bait", 3, ItemContainer::Bank),
                item(coin, "Coins", 10, ItemContainer::Bank),
            ]),
            Vec::new(),
        );
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 4, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            }),
            Poll::Ready(Err(ActionError::Cancelled))
        ));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }
}
