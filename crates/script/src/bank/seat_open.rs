//! Native bank opening and bank selection for I-family seat requests.
//!
//! This owner keeps the frozen access order while reading only the current
//! native snapshot and sending effects through the slot's real action context.

use crate::bank::compat::{BANK_READY_MS, WALK_BOUND_MS};
use crate::bank::npc;
use crate::bank::ops;
use crate::bank::request::{
    BankSeatArgs, BankSeatDestination, BankSeatNpcAccess, BankSeatObjectAccess, BankSeatOperation,
    HOOK_LOG,
};
use crate::bank::seat_transfer::{TransferAnswer, TransferCall, TransferHooks};
use crate::native::{
    ActionContext, ActionError, NativeMachine, WalkAllow, WalkBit, WalkEnd, WalkOptions,
    WalkRequest,
};
use crate::native_bank::{
    nearest_eligible_bank as select_nearest_eligible_bank, BankSelector, PickKind, Select,
    SelectArgs, SelectedBank,
};
use crate::shim::InteractReq;
use api::named_banks::{BankPreferences, NamedBank, NamedBankFacts};
use api::snapshot::{LocView, WorldTile};
use nav_planner::contract::ArrivalKind;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::Duration;

const READY_TIMEOUT_MS: u64 = 4_000;
const NEAR_WALK_TIMEOUT_MS: u64 = 15_000;
const WORLD_WALK_TIMEOUT_MS: u64 = 120_000;
const OPENER_INTERACT_TIMEOUT_MS: u64 = 8_000;
const OBJECT_INTERACT_TIMEOUT_MS: u64 = 8_000;
const ADJACENT_INTERACT_TIMEOUT_MS: u64 = 4_000;
const DIALOG_OPEN_TIMEOUT_MS: u64 = 4_000;

/// The request and immutable content facts needed by the opening owner.
pub struct OpeningArgs {
    pub request: BankSeatArgs,
    pub facts: Arc<NamedBankFacts>,
    pub preferences: BankPreferences,
    pub hooks: Arc<Mutex<TransferHooks>>,
}

/// Result returned by low-level opening and selection operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpeningValue {
    Bool(bool),
    Selected {
        bank: Option<NamedBank>,
        selected: SelectedBank,
    },
}

/// One native owner for every I-family bank-opening operation.
pub struct Opening {
    core: Core,
    facts: Arc<NamedBankFacts>,
    hooks: Arc<Mutex<TransferHooks>>,
    log: LogCall,
    log_enabled: bool,
}

#[expect(
    clippy::large_enum_variant,
    reason = "the bounded bank row keeps its opening phases inline instead of allocating a nested box per opening"
)]
enum Core {
    Immediate(OpeningValue),
    Booth(BoothOpen),
    Object(ObjectOpen),
    Npc(NpcOpen),
    Access(AccessOpen),
    Banking(BankingOpen),
    Select(SelectorCore),
}

impl NativeMachine for Opening {
    type Args = OpeningArgs;
    type Output = OpeningValue;

    fn begin(args: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        let log_enabled = args.request.hooks_present & HOOK_LOG != 0;
        let core = match args.request.operation {
            BankSeatOperation::OpenBooth => Core::Booth(BoothOpen::new(
                BoothMode::Booth,
                args.request.stand,
                args.request.object_access.as_ref().map(object_selector),
            )),
            BankSeatOperation::OpenNearest => {
                match args.request.object_access.as_ref().map(object_access) {
                    Some(access) => {
                        Core::Booth(BoothOpen::new(BoothMode::Named, None, Some(access)))
                    }
                    None => Core::Immediate(OpeningValue::Bool(false)),
                }
            }
            BankSeatOperation::OpenNearestWorld => {
                Core::Banking(BankingOpen::new(&args.request, args.preferences))
            }
            BankSeatOperation::OpenObject => match args.request.object_access.as_ref() {
                Some(access) => Core::Object(ObjectOpen::new(object_access(access))),
                None => Core::Immediate(OpeningValue::Bool(false)),
            },
            BankSeatOperation::OpenNpc => match args.request.npc_access.as_ref() {
                Some(access) => Core::Npc(NpcOpen::new(npc_access(access))),
                None => Core::Immediate(OpeningValue::Bool(false)),
            },
            BankSeatOperation::OpenAccess => Core::Access(AccessOpen::new(
                args.request.npc_access.as_ref().map(npc_access),
                args.request.object_access.as_ref().map(object_access),
                args.request.fallback_access.as_ref().map(object_access),
                default_access("Bank booth", "Use-quickly"),
            )),
            BankSeatOperation::Open => {
                Core::Banking(BankingOpen::new(&args.request, args.preferences))
            }
            BankSeatOperation::Select => {
                let preferences = effective_preferences(&args.request, args.preferences);
                match SelectorCore::begin(
                    Arc::clone(&args.facts),
                    preferences,
                    args.request.stand,
                    args.request.allow_wilderness,
                    true,
                    cx,
                )? {
                    Some(select) => Core::Select(select),
                    None => Core::Immediate(OpeningValue::Selected {
                        bank: None,
                        selected: no_candidate_selection(),
                    }),
                }
            }
            _ => {
                return Err(ActionError::Unavailable(Arc::from(
                    "bank opening operation unavailable",
                )));
            }
        };
        Ok(Self {
            core,
            facts: args.facts,
            hooks: args.hooks,
            log: LogCall::default(),
            log_enabled,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        match self.log.poll(&self.hooks) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
            Poll::Ready(Ok(())) => {}
        }
        match &mut self.core {
            Core::Immediate(value) => Poll::Ready(Ok(value.clone())),
            Core::Booth(open) => map_bool(open.poll(cx, &mut self.log, self.log_enabled)),
            Core::Object(open) => map_bool(open.poll(cx, &mut self.log, self.log_enabled)),
            Core::Npc(open) => map_bool(open.poll(cx, &mut self.log, self.log_enabled)),
            Core::Access(open) => map_bool(open.poll(cx, &mut self.log, self.log_enabled)),
            Core::Banking(open) => {
                map_bool(open.poll(cx, &self.facts, &mut self.log, self.log_enabled))
            }
            Core::Select(select) => match select.poll(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
                Poll::Ready(Ok(selection)) => {
                    let (bank, selected) = selection.map_or_else(
                        || (None, no_candidate_selection()),
                        |selection| (Some(selection.bank), selection.selected),
                    );
                    Poll::Ready(Ok(OpeningValue::Selected { bank, selected }))
                }
            },
        }
    }
}

fn map_bool(poll: Poll<Result<bool, ActionError>>) -> Poll<Result<OpeningValue, ActionError>> {
    match poll {
        Poll::Pending => Poll::Pending,
        Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
        Poll::Ready(Ok(value)) => Poll::Ready(Ok(OpeningValue::Bool(value))),
    }
}

// `encode_arguments([String])` adds an array tag/u16 length and a string tag/u16 length.
const MAX_LOG_TEXT_BYTES: usize = api::script_contract::PROMPT_ARGS_MAX_BYTES - 6;

fn truncate_log_text(line: &mut String) {
    if line.len() > MAX_LOG_TEXT_BYTES {
        let mut end = MAX_LOG_TEXT_BYTES;
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        line.truncate(end);
    }
}

#[derive(Default)]
struct LogCall {
    line: Option<String>,
    waiting: bool,
}

impl LogCall {
    fn queue(&mut self, mut line: String, enabled: bool) {
        if enabled {
            truncate_log_text(&mut line);
            self.line = Some(line);
        }
    }

    fn poll(&mut self, hooks: &Arc<Mutex<TransferHooks>>) -> Poll<Result<(), ActionError>> {
        let Some(line) = self.line.as_ref() else {
            return Poll::Ready(Ok(()));
        };
        let mut mailbox = lock_hooks(hooks);
        if self.waiting {
            let Some(answer) = mailbox.answer.take() else {
                return Poll::Pending;
            };
            mailbox.pending.take();
            self.waiting = false;
            self.line.take();
            return match answer {
                TransferAnswer::Value(_) => Poll::Ready(Ok(())),
                TransferAnswer::Failed => {
                    Poll::Ready(Err(ActionError::Failed(Arc::from("bank callback failed"))))
                }
            };
        }
        if mailbox.pending.is_some() || mailbox.answer.is_some() {
            return Poll::Pending;
        }
        mailbox.pending = Some(TransferCall {
            hook: "log",
            args: vec![Value::String(line.clone())],
        });
        self.waiting = true;
        Poll::Pending
    }
}

fn lock_hooks(hooks: &Mutex<TransferHooks>) -> std::sync::MutexGuard<'_, TransferHooks> {
    hooks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn unavailable(message: &'static str) -> ActionError {
    ActionError::Unavailable(Arc::from(message))
}

fn failed(message: &'static str) -> ActionError {
    ActionError::Failed(Arc::from(message))
}

#[derive(Clone, Copy)]
struct TimeBound {
    deadline: Duration,
}

impl TimeBound {
    fn new(cx: &ActionContext<'_>, millis: u64) -> Self {
        Self {
            deadline: cx
                .active_now()
                .saturating_add(Duration::from_millis(millis)),
        }
    }

    fn reached(self, cx: &ActionContext<'_>) -> bool {
        cx.active_now() >= self.deadline
    }
}

#[derive(Clone, Copy)]
struct TickBound {
    tick_deadline: u64,
    active_deadline: Duration,
}

impl TickBound {
    fn new(cx: &ActionContext<'_>, millis: u64) -> Self {
        let ticks = ops::observed_ticks_for_millis(millis);
        let active_ms = ticks.saturating_mul(ops::SERVER_TICK_MS);
        Self {
            tick_deadline: cx.evidence().tick.saturating_add(ticks),
            active_deadline: cx
                .active_now()
                .saturating_add(Duration::from_millis(active_ms)),
        }
    }

    fn reached(self, cx: &ActionContext<'_>) -> bool {
        cx.evidence().tick >= self.tick_deadline && cx.active_now() >= self.active_deadline
    }
}

fn effective_preferences(request: &BankSeatArgs, defaults: BankPreferences) -> BankPreferences {
    BankPreferences {
        use_mage_bank: request.use_mage_bank.unwrap_or(defaults.use_mage_bank),
        use_zanaris_bank: request
            .use_zanaris_bank
            .unwrap_or(defaults.use_zanaris_bank),
    }
}

fn walk_request(
    cx: &ActionContext<'_>,
    target: WorldTile,
    radius: i32,
    options: WalkOptions,
) -> WalkRequest {
    build_walk_request(cx.evidence(), target, radius, options)
}

/// Pause ends the host follow; a kept relog revokes its old session.
/// The retained bank cursor revalidates that destination under fresh request
/// authority without resetting its phase's active-time deadline.
struct OpeningWalk {
    request_id: u64,
    session: u64,
}

impl OpeningWalk {
    fn new(cx: &ActionContext<'_>, request_id: u64) -> Self {
        Self {
            request_id,
            session: cx.run().session,
        }
    }

    fn resume(
        &mut self,
        cx: &mut ActionContext<'_>,
        target: WorldTile,
        radius: i32,
        options: WalkOptions,
    ) -> Result<(), ActionError> {
        if self.session != cx.run().session
            || cx
                .walk_receipt(self.request_id)
                .is_some_and(|receipt| receipt.end == WalkEnd::Cancelled)
        {
            let request_id = cx.walk(walk_request(cx, target, radius, options))?;
            *self = Self::new(cx, request_id);
        }
        Ok(())
    }
}

fn build_walk_request(
    required_after: api::quest_progress::EvidenceStamp,
    target: WorldTile,
    radius: i32,
    options: WalkOptions,
) -> WalkRequest {
    WalkRequest {
        target,
        candidate_goals: None,
        loc_id: None,
        radius: u16::try_from(radius).unwrap_or(u16::MAX),
        arrival: ArrivalKind::Reach,
        options,
        required_after,
        evidence: None,
        cross: Box::default(),
        protect: false,
        food_guard: false,
        allow: WalkAllow::default(),
    }
}

fn access_walk_options() -> WalkOptions {
    WalkOptions {
        allow_teleports: WalkBit::Forbid,
        allow_wilderness: WalkBit::Allow,
        allow_danger_zones: WalkBit::Inherit,
    }
}

fn world_walk_options() -> WalkOptions {
    WalkOptions {
        allow_teleports: WalkBit::Forbid,
        allow_wilderness: WalkBit::Inherit,
        allow_danger_zones: WalkBit::Inherit,
    }
}

fn current_tile(cx: &ActionContext<'_>) -> Option<WorldTile> {
    cx.snapshot().here().map(|here| here.value)
}

fn in_game(cx: &ActionContext<'_>) -> bool {
    cx.snapshot().bank_session().is_some()
}

fn bank_state(cx: &ActionContext<'_>) -> (bool, bool, u64) {
    let snapshot = cx.snapshot();
    let session = snapshot.bank_session();
    let open = session.as_ref().is_some_and(|session| session.value.open);
    let generation = session
        .as_ref()
        .map_or(u64::MAX, |session| session.value.generation);
    (open, snapshot.bank().is_some(), generation)
}

fn bank_near(here: Option<WorldTile>, target: WorldTile, radius: i32) -> bool {
    here.is_some_and(|here| {
        here.level == target.level
            && here.x.abs_diff(target.x).max(here.z.abs_diff(target.z))
                <= u32::try_from(radius).unwrap_or(u32::MAX)
    })
}

fn walk_arrived(cx: &ActionContext<'_>, target: WorldTile, radius: i32) -> bool {
    let snapshot = cx.snapshot();
    let Some(here) = snapshot.here() else {
        return false;
    };
    // Frozen bank walks use anchor arrival even when reach is unknown.
    // A booth click still needs the separate live operation-approach check.
    let unavailable = api::query::ReachQueryView::unavailable();
    api::query::is_arrived(here.value, target, radius, || {
        snapshot.reach().map_or(&unavailable, |reach| reach.value)
    })
}

fn lower_eq(left: &str, right: &str) -> bool {
    left.chars()
        .flat_map(char::to_lowercase)
        .eq(right.chars().flat_map(char::to_lowercase))
}

fn object_access(access: &BankSeatObjectAccess) -> ObjectAccess {
    ObjectAccess {
        name: access.name.clone(),
        op: access.op.clone(),
        open_first: match (&access.open_first_name, &access.open_first_op) {
            (Some(name), Some(op)) => Some((name.clone(), op.clone())),
            _ => None,
        },
    }
}

fn object_selector(access: &BankSeatObjectAccess) -> ObjectAccess {
    ObjectAccess {
        name: access.name.clone(),
        op: access.op.clone(),
        open_first: None,
    }
}

fn npc_access(access: &BankSeatNpcAccess) -> NpcAccessArgs {
    NpcAccessArgs {
        name: access.name.clone(),
        op: access.op.clone(),
        choose: access.choose.clone().unwrap_or_default(),
    }
}

fn default_access(name: &str, op: &str) -> ObjectAccess {
    ObjectAccess {
        name: name.to_owned(),
        op: op.to_owned(),
        open_first: None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ObjectAccess {
    name: String,
    op: String,
    open_first: Option<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NpcAccessArgs {
    name: String,
    op: String,
    choose: String,
}

struct AccessOpen {
    inner: AccessMachine,
}

enum AccessMachine {
    Object(ObjectOpen),
    Npc(NpcOpen),
}

impl AccessOpen {
    fn new(
        npc: Option<NpcAccessArgs>,
        object: Option<ObjectAccess>,
        fallback: Option<ObjectAccess>,
        default: ObjectAccess,
    ) -> Self {
        let inner = match npc {
            Some(npc) => AccessMachine::Npc(NpcOpen::new(npc)),
            None => AccessMachine::Object(ObjectOpen::new(object.or(fallback).unwrap_or(default))),
        };
        Self { inner }
    }

    fn poll(
        &mut self,
        cx: &mut ActionContext<'_>,
        log: &mut LogCall,
        log_enabled: bool,
    ) -> Poll<Result<bool, ActionError>> {
        match &mut self.inner {
            AccessMachine::Object(open) => open.poll(cx, log, log_enabled),
            AccessMachine::Npc(open) => open.poll(cx, log, log_enabled),
        }
    }
}

struct NpcOpen {
    args: NpcAccessArgs,
    core: npc::NpcAccess,
    deadline: Option<TickBound>,
}

impl NpcOpen {
    fn new(args: NpcAccessArgs) -> Self {
        Self {
            args,
            core: npc::NpcAccess::new(),
            deadline: None,
        }
    }

    fn poll(
        &mut self,
        cx: &mut ActionContext<'_>,
        log: &mut LogCall,
        log_enabled: bool,
    ) -> Poll<Result<bool, ActionError>> {
        loop {
            let intent = npc::Intent {
                name: &self.args.name,
                op: npc::NpcOp::Name(&self.args.op),
                choose: &self.args.choose,
            };
            let mut context = NativeNpcContext {
                cx,
                deadline: &mut self.deadline,
            };
            match self.core.step(intent, &mut context) {
                Ok(npc::Step::Wait) => return Poll::Pending,
                Ok(npc::Step::Done(value)) => return Poll::Ready(Ok(value)),
                Ok(npc::Step::Note(note)) => {
                    if log_enabled {
                        log.queue(note.message(&self.args.name), true);
                        return Poll::Pending;
                    }
                }
                Err(ActionError::BudgetExhausted) => return Poll::Pending,
                Err(error) => return Poll::Ready(Err(error)),
            }
        }
    }
}

struct NativeNpcContext<'a, 'frame> {
    cx: &'a mut ActionContext<'frame>,
    deadline: &'a mut Option<TickBound>,
}

impl npc::Context for NativeNpcContext<'_, '_> {
    type Error = ActionError;

    fn bank_open(&self) -> bool {
        bank_state(self.cx).0
    }

    fn bank_loaded(&self) -> bool {
        bank_state(self.cx).1
    }

    fn chat(&self) -> npc::Chat {
        let snapshot = self.cx.snapshot();
        npc::Chat {
            tick: self.cx.evidence().tick,
            modal: snapshot.chat_modal().map_or(-1, |modal| modal.value.root),
            can_continue: snapshot
                .chat_continue()
                .is_some_and(|continue_id| continue_id.value >= 0),
        }
    }

    fn banker(&self, name: &str, op: npc::NpcOp<'_>) -> Option<npc::Banker> {
        let npcs = self.cx.snapshot().npcs()?.value;
        let (row, action) = npc::banker_scan(npcs, name, op)?;
        Some(npc::Banker {
            name: row.name.as_deref()?.to_owned(),
            action: action.to_owned(),
            index: i32::try_from(row.index).ok()?,
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
        *self.deadline = Some(TickBound::new(self.cx, millis));
    }

    fn expired(&mut self) -> bool {
        self.deadline
            .is_some_and(|deadline| deadline.reached(self.cx))
    }

    fn emit(&mut self, request: InteractReq) -> Result<(), Self::Error> {
        self.cx.emit(request).map(|_| ())
    }
}

#[derive(Clone, Debug)]
struct AccessLoc {
    id: i32,
    tile: WorldTile,
    distance: i32,
    actions: Vec<String>,
}

impl AccessLoc {
    fn from_row(row: &LocView) -> Self {
        Self {
            id: row.id,
            tile: row.tile,
            distance: row.distance,
            actions: row
                .actions
                .iter()
                .filter_map(|action| action.as_deref())
                .filter(|action| !action.is_empty() && *action != "hidden")
                .map(str::to_owned)
                .collect(),
        }
    }

    fn interact(&self, action: &str) -> InteractReq {
        InteractReq::Loc {
            x: self.tile.x,
            z: self.tile.z,
            level: self.tile.level,
            action: action.to_owned(),
            id: Some(self.id),
        }
    }
}

fn loc_name_matches(got: &str, wanted: &str) -> bool {
    lower_eq(got.trim(), wanted.trim())
}

fn loc_with_action(cx: &ActionContext<'_>, name: &str, op: &str) -> Option<AccessLoc> {
    let rows = cx.snapshot().locs()?.value;
    let wanted = op.to_lowercase();
    rows.iter()
        .filter(|row| {
            row.name
                .as_deref()
                .is_some_and(|got| loc_name_matches(got, name))
                && row.actions.iter().any(|action| {
                    action.as_deref().is_some_and(|action| {
                        !action.is_empty() && action != "hidden" && lower_eq(action, &wanted)
                    })
                })
        })
        .min_by_key(|row| row.distance)
        .map(AccessLoc::from_row)
}

fn usable_loc(cx: &ActionContext<'_>, name: &str, adjacent: bool) -> Option<AccessLoc> {
    cx.snapshot()
        .locs()?
        .value
        .iter()
        .filter(|row| {
            row.name
                .as_deref()
                .is_some_and(|got| loc_name_matches(got, name))
                && row.actions.iter().any(|action| {
                    action
                        .as_deref()
                        .is_some_and(|action| !action.is_empty() && action != "hidden")
                })
                && (!adjacent || row.distance <= 1)
        })
        .min_by_key(|row| row.distance)
        .map(AccessLoc::from_row)
}

fn pick_action(actions: &[String], wanted: &str) -> Option<String> {
    actions
        .iter()
        .find(|action| action.eq_ignore_ascii_case(wanted))
        .or_else(|| {
            actions.iter().find(|action| {
                action
                    .get(..3)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("use"))
                    || action
                        .get(..4)
                        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("bank"))
            })
        })
        .or_else(|| actions.first())
        .cloned()
}

enum AfterContinue {
    Approach(AccessLoc),
    Next,
}

enum ObjectPhase {
    Start,
    BoothWalk {
        walk: OpeningWalk,
        target: WorldTile,
        deadline: TimeBound,
    },
    Booth(Box<BoothOpen>),
    OpenerFind(u8),
    OpenerWait {
        attempt: u8,
        deadline: TimeBound,
    },
    OpenerDelay {
        attempt: u8,
        until: u64,
    },
    OpenerGate,
    NearFind(u32),
    NearWait {
        attempt: u32,
        loc: AccessLoc,
        first: bool,
        deadline: TimeBound,
    },
    NearApproach {
        attempt: u32,
        loc: AccessLoc,
    },
    NearRetry {
        attempt: u32,
        loc: AccessLoc,
    },
    NearWalk {
        walk: OpeningWalk,
        attempt: u32,
        loc: AccessLoc,
        target: WorldTile,
        deadline: TimeBound,
    },
    Continue {
        attempt: u32,
        press: DialogPress,
        after: AfterContinue,
    },
    ContinueOpen {
        attempt: u32,
        after: AfterContinue,
        deadline: TickBound,
    },
    Ready(Option<TickBound>),
    Finish(bool),
}
fn object_start_phase(args: &ObjectAccess, bank_open: bool, target_present: bool) -> ObjectPhase {
    if bank_open {
        ObjectPhase::Ready(None)
    } else if args.open_first.is_some() && !target_present {
        ObjectPhase::OpenerFind(0)
    } else {
        ObjectPhase::NearFind(0)
    }
}
fn opener_gate_phase(bank_open: bool, target_present: bool) -> ObjectPhase {
    if bank_open {
        ObjectPhase::Ready(None)
    } else if target_present {
        ObjectPhase::NearFind(0)
    } else {
        ObjectPhase::Finish(false)
    }
}

struct ObjectOpen {
    args: ObjectAccess,
    phase: ObjectPhase,
}

impl ObjectOpen {
    fn new(args: ObjectAccess) -> Self {
        Self {
            args,
            phase: ObjectPhase::Start,
        }
    }

    fn default_booth(&self) -> bool {
        self.args.open_first.is_none()
            && self.args.name.eq_ignore_ascii_case("bank booth")
            && self.args.op.eq_ignore_ascii_case("use-quickly")
    }

    fn poll(
        &mut self,
        cx: &mut ActionContext<'_>,
        log: &mut LogCall,
        log_enabled: bool,
    ) -> Poll<Result<bool, ActionError>> {
        loop {
            let phase = std::mem::replace(&mut self.phase, ObjectPhase::Finish(false));
            match phase {
                ObjectPhase::Start => {
                    if bank_state(cx).0 {
                        self.phase = ObjectPhase::Ready(None);
                    } else if self.default_booth() {
                        let Some(booth) = BoothOpen::new(BoothMode::Booth, None, None).select(cx)
                        else {
                            self.queue_log(
                                format!("no usable '{}' in the scene", self.args.name),
                                ObjectPhase::Finish(false),
                                log,
                                log_enabled,
                            );
                            if log_enabled {
                                return Poll::Pending;
                            }
                            continue;
                        };
                        if walk_arrived(cx, booth.tile, 1) {
                            self.phase = ObjectPhase::Booth(Box::new(BoothOpen::new(
                                BoothMode::Booth,
                                None,
                                None,
                            )));
                            continue;
                        }
                        match cx.walk(walk_request(cx, booth.tile, 1, access_walk_options())) {
                            Ok(request_id) => {
                                self.phase = ObjectPhase::BoothWalk {
                                    walk: OpeningWalk::new(cx, request_id),
                                    target: booth.tile,
                                    deadline: TimeBound::new(cx, WALK_BOUND_MS),
                                };
                                return Poll::Pending;
                            }
                            Err(ActionError::BudgetExhausted) => {
                                self.phase = ObjectPhase::Start;
                                return Poll::Pending;
                            }
                            Err(error) => return Poll::Ready(Err(error)),
                        }
                    } else {
                        // The opener's presence does not mean its bank target is usable.
                        // 289 duel chest content opens Closed chest before Open chest/Bank.
                        let target_present =
                            loc_with_action(cx, &self.args.name, &self.args.op).is_some();
                        self.phase = object_start_phase(&self.args, false, target_present);
                    }
                }
                ObjectPhase::BoothWalk {
                    mut walk,
                    target,
                    deadline,
                } => {
                    if walk_arrived(cx, target, 1) {
                        self.phase = ObjectPhase::Booth(Box::new(BoothOpen::new(
                            BoothMode::Booth,
                            None,
                            None,
                        )));
                    } else if deadline.reached(cx) {
                        return Poll::Ready(Ok(false));
                    } else {
                        if let Err(error) = walk.resume(cx, target, 1, access_walk_options()) {
                            if error != ActionError::BudgetExhausted {
                                return Poll::Ready(Err(error));
                            }
                        }
                        self.phase = ObjectPhase::BoothWalk {
                            walk,
                            target,
                            deadline,
                        };
                        return Poll::Pending;
                    }
                }
                ObjectPhase::Booth(mut open) => match open.poll(cx, log, log_enabled) {
                    Poll::Pending => {
                        self.phase = ObjectPhase::Booth(open);
                        return Poll::Pending;
                    }
                    Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                    Poll::Ready(Ok(value)) => return Poll::Ready(Ok(value)),
                },
                ObjectPhase::OpenerFind(attempt) => {
                    if attempt >= 3 || bank_state(cx).0 {
                        self.phase = ObjectPhase::OpenerGate;
                        continue;
                    }
                    let (name, op) = self.args.open_first.as_ref().expect("opener phase");
                    let Some(loc) = loc_with_action(cx, name, op) else {
                        let until = cx.evidence().tick.saturating_add(1);
                        self.queue_log(
                            format!("no '{name}' with '{op}' in the scene"),
                            ObjectPhase::OpenerDelay { attempt, until },
                            log,
                            log_enabled,
                        );
                        if log_enabled {
                            return Poll::Pending;
                        }
                        continue;
                    };
                    let action = pick_action(&loc.actions, op).unwrap_or_else(|| op.clone());
                    match cx.emit(loc.interact(&action)) {
                        Ok(_) => {
                            self.queue_log(
                                format!("opening '{name}' before banking"),
                                ObjectPhase::OpenerWait {
                                    attempt,
                                    deadline: TimeBound::new(cx, OPENER_INTERACT_TIMEOUT_MS),
                                },
                                log,
                                log_enabled,
                            );
                            if log_enabled {
                                return Poll::Pending;
                            }
                        }
                        Err(ActionError::BudgetExhausted) => {
                            self.phase = ObjectPhase::OpenerFind(attempt);
                            return Poll::Pending;
                        }
                        Err(error) => return Poll::Ready(Err(error)),
                    }
                }
                ObjectPhase::OpenerWait { attempt, deadline } => {
                    if bank_state(cx).0
                        || loc_with_action(cx, &self.args.name, &self.args.op).is_some()
                    {
                        self.phase = ObjectPhase::OpenerGate;
                    } else if deadline.reached(cx) {
                        self.phase = ObjectPhase::OpenerFind(attempt.saturating_add(1));
                    } else {
                        self.phase = ObjectPhase::OpenerWait { attempt, deadline };
                        return Poll::Pending;
                    }
                }
                ObjectPhase::OpenerDelay { attempt, until } => {
                    if cx.evidence().tick < until {
                        self.phase = ObjectPhase::OpenerDelay { attempt, until };
                        return Poll::Pending;
                    }
                    self.phase = ObjectPhase::OpenerFind(attempt.saturating_add(1));
                }
                ObjectPhase::OpenerGate => {
                    if bank_state(cx).0 {
                        self.phase = opener_gate_phase(true, false);
                    } else if loc_with_action(cx, &self.args.name, &self.args.op).is_none() {
                        self.queue_log(
                            format!("'{}' never became usable", self.args.name),
                            opener_gate_phase(false, false),
                            log,
                            log_enabled,
                        );
                        if log_enabled {
                            return Poll::Pending;
                        }
                    } else {
                        self.phase = opener_gate_phase(false, true);
                    }
                }
                ObjectPhase::NearFind(attempt) => {
                    if attempt >= 6 || bank_state(cx).0 {
                        self.phase = ObjectPhase::Ready(None);
                        continue;
                    }
                    let Some(loc) = usable_loc(cx, &self.args.name, false) else {
                        self.queue_log(
                            format!("no usable '{}' in the scene", self.args.name),
                            ObjectPhase::Finish(false),
                            log,
                            log_enabled,
                        );
                        if log_enabled {
                            return Poll::Pending;
                        }
                        continue;
                    };
                    let Some(action) = pick_action(&loc.actions, &self.args.op) else {
                        self.phase = ObjectPhase::NearApproach { attempt, loc };
                        continue;
                    };
                    match cx.emit(loc.interact(&action)) {
                        Ok(_) => {
                            self.phase = ObjectPhase::NearWait {
                                attempt,
                                loc,
                                first: true,
                                deadline: TimeBound::new(cx, OBJECT_INTERACT_TIMEOUT_MS),
                            };
                            return Poll::Pending;
                        }
                        Err(ActionError::BudgetExhausted) => {
                            self.phase = ObjectPhase::NearFind(attempt);
                            return Poll::Pending;
                        }
                        Err(error) => return Poll::Ready(Err(error)),
                    }
                }
                ObjectPhase::NearWait {
                    attempt,
                    loc,
                    first,
                    deadline,
                } => {
                    let (open, _, _) = bank_state(cx);
                    let chat = native_chat(cx);
                    if chat.can_continue {
                        match DialogPress::send(cx, false, chat) {
                            Ok(press) => {
                                let after = if first {
                                    AfterContinue::Approach(loc)
                                } else {
                                    AfterContinue::Next
                                };
                                self.phase = ObjectPhase::Continue {
                                    attempt,
                                    press,
                                    after,
                                };
                                return Poll::Pending;
                            }
                            Err(ActionError::BudgetExhausted) => {
                                self.phase = ObjectPhase::NearWait {
                                    attempt,
                                    loc,
                                    first,
                                    deadline,
                                };
                                return Poll::Pending;
                            }
                            Err(error) => return Poll::Ready(Err(error)),
                        }
                    }
                    if open {
                        self.phase = ObjectPhase::Ready(None);
                    } else if deadline.reached(cx) {
                        self.phase = if first {
                            ObjectPhase::NearApproach { attempt, loc }
                        } else {
                            ObjectPhase::NearFind(attempt.saturating_add(1))
                        };
                    } else {
                        self.phase = ObjectPhase::NearWait {
                            attempt,
                            loc,
                            first,
                            deadline,
                        };
                        return Poll::Pending;
                    }
                }
                ObjectPhase::NearApproach { attempt, loc } => {
                    if loc.distance <= 1 {
                        self.retry_beside(attempt, Some(loc), cx);
                        continue;
                    }
                    let (line, target) = match bank_stand(cx, loc.tile) {
                        Some(stand) => (
                            format!(
                                "booth didn't open — stepping to the bank counter at ({}, {})",
                                stand.x, stand.z
                            ),
                            stand,
                        ),
                        None => (
                            format!(
                                "no reachable tile beside '{}' yet — closing in",
                                self.args.name
                            ),
                            loc.tile,
                        ),
                    };
                    match cx.walk(walk_request(cx, target, 1, access_walk_options())) {
                        Ok(request_id) => {
                            let deadline = TimeBound::new(cx, NEAR_WALK_TIMEOUT_MS);
                            self.queue_log(
                                line,
                                ObjectPhase::NearWalk {
                                    walk: OpeningWalk::new(cx, request_id),
                                    attempt,
                                    loc,
                                    target,
                                    deadline,
                                },
                                log,
                                log_enabled,
                            );
                            if log_enabled {
                                return Poll::Pending;
                            }
                        }
                        Err(ActionError::BudgetExhausted) => {
                            self.phase = ObjectPhase::NearApproach { attempt, loc };
                            return Poll::Pending;
                        }
                        Err(error) => return Poll::Ready(Err(error)),
                    }
                }
                ObjectPhase::NearWalk {
                    mut walk,
                    attempt,
                    loc,
                    target,
                    deadline,
                } => {
                    if walk_arrived(cx, target, 1) || deadline.reached(cx) {
                        self.retry_beside(attempt, Some(loc), cx);
                    } else {
                        if let Err(error) = walk.resume(cx, target, 1, access_walk_options()) {
                            if error != ActionError::BudgetExhausted {
                                return Poll::Ready(Err(error));
                            }
                        }
                        self.phase = ObjectPhase::NearWalk {
                            walk,
                            attempt,
                            loc,
                            target,
                            deadline,
                        };
                        return Poll::Pending;
                    }
                }
                ObjectPhase::NearRetry { attempt, loc } => {
                    self.retry_beside(attempt, Some(loc), cx);
                }
                ObjectPhase::Continue {
                    attempt,
                    mut press,
                    after,
                } => match press.poll(cx) {
                    None => {
                        self.phase = ObjectPhase::Continue {
                            attempt,
                            press,
                            after,
                        };
                        return Poll::Pending;
                    }
                    Some(true) => {
                        self.phase = ObjectPhase::ContinueOpen {
                            attempt,
                            after,
                            deadline: TickBound::new(cx, DIALOG_OPEN_TIMEOUT_MS),
                        };
                    }
                    Some(false) => {
                        let next = after_continue(attempt, after);
                        self.queue_log(
                            "bank: failed to Continue the object dialogue".into(),
                            next,
                            log,
                            log_enabled,
                        );
                        if log_enabled {
                            return Poll::Pending;
                        }
                    }
                },
                ObjectPhase::ContinueOpen {
                    attempt,
                    after,
                    deadline,
                } => {
                    if bank_state(cx).0 {
                        self.phase = ObjectPhase::Ready(None);
                    } else if deadline.reached(cx) {
                        let next = after_continue(attempt, after);
                        self.queue_log(
                            "bank: object dialogue completed without opening the bank".into(),
                            next,
                            log,
                            log_enabled,
                        );
                        if log_enabled {
                            return Poll::Pending;
                        }
                    } else {
                        self.phase = ObjectPhase::ContinueOpen {
                            attempt,
                            after,
                            deadline,
                        };
                        return Poll::Pending;
                    }
                }
                ObjectPhase::Ready(deadline) => {
                    let (open, loaded, _) = bank_state(cx);
                    if !open {
                        return Poll::Ready(Ok(false));
                    }
                    if loaded {
                        return Poll::Ready(Ok(true));
                    }
                    let deadline = deadline.unwrap_or_else(|| TickBound::new(cx, READY_TIMEOUT_MS));
                    if deadline.reached(cx) {
                        self.queue_log(
                            "bank: opened but the item list never arrived".into(),
                            ObjectPhase::Finish(true),
                            log,
                            log_enabled,
                        );
                        if log_enabled {
                            return Poll::Pending;
                        }
                    } else {
                        self.phase = ObjectPhase::Ready(Some(deadline));
                        return Poll::Pending;
                    }
                }
                ObjectPhase::Finish(value) => return Poll::Ready(Ok(value)),
            }
        }
    }

    fn queue_log(&mut self, line: String, next: ObjectPhase, log: &mut LogCall, log_enabled: bool) {
        self.phase = next;
        log.queue(line, log_enabled);
    }

    fn retry_beside(&mut self, attempt: u32, found: Option<AccessLoc>, cx: &mut ActionContext<'_>) {
        let Some(loc) = usable_loc(cx, &self.args.name, true).or(found) else {
            self.phase = ObjectPhase::NearFind(attempt.saturating_add(1));
            return;
        };
        let Some(action) = pick_action(&loc.actions, &self.args.op) else {
            self.phase = ObjectPhase::NearFind(attempt.saturating_add(1));
            return;
        };
        match cx.emit(loc.interact(&action)) {
            Ok(_) => {
                self.phase = ObjectPhase::NearWait {
                    attempt,
                    loc,
                    first: false,
                    deadline: TimeBound::new(cx, ADJACENT_INTERACT_TIMEOUT_MS),
                };
            }
            Err(ActionError::BudgetExhausted) => {
                self.phase = ObjectPhase::NearRetry { attempt, loc };
            }
            Err(_) => self.phase = ObjectPhase::Finish(false),
        }
    }
}

fn after_continue(attempt: u32, after: AfterContinue) -> ObjectPhase {
    match after {
        AfterContinue::Approach(loc) => ObjectPhase::NearApproach { attempt, loc },
        AfterContinue::Next => ObjectPhase::NearFind(attempt.saturating_add(1)),
    }
}

#[derive(Clone, Copy)]
struct NativeChat {
    tick: u64,
    modal: i32,
    can_continue: bool,
}

fn native_chat(cx: &ActionContext<'_>) -> NativeChat {
    let snapshot = cx.snapshot();
    NativeChat {
        tick: cx.evidence().tick,
        modal: snapshot.chat_modal().map_or(-1, |modal| modal.value.root),
        can_continue: snapshot
            .chat_continue()
            .is_some_and(|continue_id| continue_id.value >= 0),
    }
}

struct DialogPress {
    choice: bool,
    ack_modal: i32,
    deadline: TickBound,
    due: Option<u64>,
}

impl DialogPress {
    fn send(
        cx: &mut ActionContext<'_>,
        choice: bool,
        chat: NativeChat,
    ) -> Result<Self, ActionError> {
        let request = if choice {
            InteractReq::Answer { option: 0 }
        } else {
            InteractReq::ContinueDialog { component_id: None }
        };
        // `choice` is not currently sent through this object-access branch.
        if choice {
            return Err(unavailable("bank object choice unavailable"));
        }
        cx.emit(request)?;
        Ok(Self {
            choice,
            ack_modal: chat.modal,
            deadline: TickBound::new(cx, npc::PAGE_ACK_MS),
            due: None,
        })
    }

    fn poll(&mut self, cx: &ActionContext<'_>) -> Option<bool> {
        let chat = native_chat(cx);
        if let Some(due) = self.due {
            return (chat.tick >= due).then_some(true);
        }
        let acked = chat.modal != self.ack_modal
            || if self.choice {
                chat.can_continue
            } else {
                !chat.can_continue
            };
        if acked {
            let ticks = if self.choice {
                npc::CHOICE_TICKS
            } else {
                npc::CONTINUE_TICKS
            };
            self.due = Some(chat.tick.saturating_add(ticks));
            return None;
        }
        self.deadline.reached(cx).then_some(false)
    }
}

fn bank_stand(cx: &ActionContext<'_>, loc: WorldTile) -> Option<WorldTile> {
    let here = current_tile(cx);
    let snapshot = cx.snapshot();
    let reach = snapshot.reach()?;
    let options = api::query::SceneReachOptions {
        max_steps: Some(400),
        adjacent_ok: false,
    };
    nearest_reachable_stand(loc, here, |tile| reach.value.can_reach(tile, &options))
}

fn nearest_reachable_stand(
    loc: WorldTile,
    here: Option<WorldTile>,
    mut can_reach: impl FnMut(WorldTile) -> bool,
) -> Option<WorldTile> {
    let mut best: Option<(u32, usize, WorldTile)> = None;
    for (index, (dx, dz)) in [(1, 0), (-1, 0), (0, 1), (0, -1)].into_iter().enumerate() {
        let tile = WorldTile {
            x: loc.x + dx,
            z: loc.z + dz,
            level: loc.level,
        };
        if !can_reach(tile) {
            continue;
        }
        let distance = here.map_or(0, |here| {
            here.x.abs_diff(tile.x).max(here.z.abs_diff(tile.z))
        });
        if best.is_none_or(|current| (distance, index) < (current.0, current.1)) {
            best = Some((distance, index, tile));
        }
    }
    best.map(|(_, _, tile)| tile)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BoothSelection {
    id: i32,
    tile: WorldTile,
    name: Option<String>,
    action: Option<String>,
}
enum BoothApproach {
    Operate,
    Walk(WorldTile),
    Unavailable,
}

fn booth_approach(can_operate: bool, destination: Option<WorldTile>) -> BoothApproach {
    if can_operate {
        BoothApproach::Operate
    } else {
        destination.map_or(BoothApproach::Unavailable, BoothApproach::Walk)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BoothMode {
    Booth,
    Named,
}

struct BoothOpen {
    mode: BoothMode,
    stand: Option<WorldTile>,
    wanted: Option<ObjectAccess>,
    phase: BoothPhase,
    selected: Option<BoothSelection>,
    last_approach_dest: Option<WorldTile>,
    walk: Option<OpeningWalk>,
    open_generation: u64,
}

enum BoothPhase {
    Begin,
    WaitStand { deadline: TimeBound },
    WaitSelected { deadline: TimeBound },
    WaitFresh { deadline: TimeBound },
    WaitReady { deadline: TimeBound },
}

impl BoothOpen {
    fn new(mode: BoothMode, stand: Option<WorldTile>, wanted: Option<ObjectAccess>) -> Self {
        Self {
            mode,
            stand,
            wanted,
            phase: BoothPhase::Begin,
            selected: None,
            last_approach_dest: None,
            walk: None,
            open_generation: u64::MAX,
        }
    }

    fn poll(
        &mut self,
        cx: &mut ActionContext<'_>,
        _log: &mut LogCall,
        _log_enabled: bool,
    ) -> Poll<Result<bool, ActionError>> {
        loop {
            if !in_game(cx) {
                return Poll::Ready(Ok(false));
            }
            let phase = std::mem::replace(&mut self.phase, BoothPhase::Begin);
            match phase {
                BoothPhase::Begin => {
                    let (open, loaded, generation) = bank_state(cx);
                    if open {
                        if loaded {
                            return Poll::Ready(Ok(true));
                        }
                        self.phase = BoothPhase::WaitReady {
                            deadline: TimeBound::new(cx, BANK_READY_MS),
                        };
                        return Poll::Pending;
                    }
                    if let Some(stand) = self.stand {
                        if !bank_near(current_tile(cx), stand, 1) {
                            let walk = cx.walk(walk_request(cx, stand, 1, access_walk_options()));
                            match walk {
                                Ok(request_id) => {
                                    self.walk = Some(OpeningWalk::new(cx, request_id));
                                    self.phase = BoothPhase::WaitStand {
                                        deadline: TimeBound::new(cx, WALK_BOUND_MS),
                                    };
                                    return Poll::Pending;
                                }
                                Err(ActionError::BudgetExhausted) => {
                                    self.phase = BoothPhase::Begin;
                                    return Poll::Pending;
                                }
                                Err(error) => return Poll::Ready(Err(error)),
                            }
                        }
                    }
                    let Some(selected) = self.select(cx) else {
                        return Poll::Ready(Ok(false));
                    };
                    self.selected = Some(selected);
                    self.open_generation = generation;
                    match self.approach_or_open(cx) {
                        Poll::Pending => return Poll::Pending,
                        Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                        Poll::Ready(Ok(Some(value))) => return Poll::Ready(Ok(value)),
                        Poll::Ready(Ok(None)) => continue,
                    }
                }
                BoothPhase::WaitStand { deadline } => {
                    let (open, loaded, generation) = bank_state(cx);
                    if open {
                        if loaded {
                            return Poll::Ready(Ok(true));
                        }
                        self.phase = BoothPhase::WaitReady {
                            deadline: TimeBound::new(cx, BANK_READY_MS),
                        };
                        return Poll::Pending;
                    }
                    if bank_near(current_tile(cx), self.stand.expect("stand walk"), 1) {
                        self.open_generation = generation;
                        let Some(selected) = self.select(cx) else {
                            return Poll::Ready(Ok(false));
                        };
                        self.selected = Some(selected);
                        match self.approach_or_open(cx) {
                            Poll::Pending => return Poll::Pending,
                            Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                            Poll::Ready(Ok(Some(value))) => return Poll::Ready(Ok(value)),
                            Poll::Ready(Ok(None)) => continue,
                        }
                    }
                    if deadline.reached(cx) {
                        return Poll::Ready(Ok(false));
                    }
                    if let Some(walk) = self.walk.as_mut() {
                        if let Err(error) = walk.resume(
                            cx,
                            self.stand.expect("stand walk"),
                            1,
                            access_walk_options(),
                        ) {
                            if error != ActionError::BudgetExhausted {
                                return Poll::Ready(Err(error));
                            }
                        }
                    }
                    self.phase = BoothPhase::WaitStand { deadline };
                    return Poll::Pending;
                }
                BoothPhase::WaitSelected { deadline } => {
                    let (open, loaded, _) = bank_state(cx);
                    if open {
                        if loaded {
                            return Poll::Ready(Ok(true));
                        }
                        self.phase = BoothPhase::WaitReady {
                            deadline: TimeBound::new(cx, BANK_READY_MS),
                        };
                        return Poll::Pending;
                    }
                    if !self.selected_still_present(cx) {
                        return Poll::Ready(Ok(false));
                    }
                    let Some(approach) = self.live_approach(cx) else {
                        return Poll::Ready(Ok(false));
                    };
                    match approach {
                        BoothApproach::Operate => match self.send_open(cx) {
                            Ok(()) => return Poll::Pending,
                            Err(ActionError::BudgetExhausted) => {
                                self.phase = BoothPhase::WaitSelected { deadline };
                                return Poll::Pending;
                            }
                            Err(error) => return Poll::Ready(Err(error)),
                        },
                        BoothApproach::Unavailable => return Poll::Ready(Ok(false)),
                        BoothApproach::Walk(destination) => {
                            if deadline.reached(cx) {
                                return Poll::Ready(Ok(false));
                            }
                            match self.start_approach(cx, destination, deadline) {
                                Poll::Pending => return Poll::Pending,
                                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                                Poll::Ready(Ok(value)) => return Poll::Ready(Ok(value)),
                            }
                        }
                    }
                }
                BoothPhase::WaitFresh { deadline } => {
                    let (open, loaded, generation) = bank_state(cx);
                    if open && loaded && generation > self.open_generation {
                        return Poll::Ready(Ok(true));
                    }
                    if deadline.reached(cx) {
                        return Poll::Ready(Ok(false));
                    }
                    self.phase = BoothPhase::WaitFresh { deadline };
                    return Poll::Pending;
                }
                BoothPhase::WaitReady { deadline } => {
                    let (open, loaded, _) = bank_state(cx);
                    if open && loaded {
                        return Poll::Ready(Ok(true));
                    }
                    if deadline.reached(cx) {
                        return Poll::Ready(Ok(false));
                    }
                    self.phase = BoothPhase::WaitReady { deadline };
                    return Poll::Pending;
                }
            }
        }
    }

    fn select(&self, cx: &ActionContext<'_>) -> Option<BoothSelection> {
        let rows = cx.snapshot().locs()?.value;
        if let Some(wanted) = self.wanted.as_ref() {
            let level = current_tile(cx).map(|tile| tile.level);
            let row = ops::booth_named(rows, &wanted.name, &wanted.op, level)?;
            return Some(BoothSelection {
                id: row.id,
                tile: row.tile,
                name: Some(wanted.name.clone()),
                action: Some(wanted.op.clone()),
            });
        }
        if self.mode == BoothMode::Named {
            return None;
        }
        let level = current_tile(cx)?.level;
        let row = nearest_use_quickly(rows, current_tile(cx)?, level)?;
        Some(BoothSelection {
            id: row.id,
            tile: row.tile,
            name: None,
            action: None,
        })
    }

    fn selected_still_present(&self, cx: &ActionContext<'_>) -> bool {
        let Some(selected) = self.selected.as_ref() else {
            return false;
        };
        let Some(rows) = cx.snapshot().locs() else {
            return false;
        };
        Self::selected_booth_still_present(
            selected,
            rows.value,
            current_tile(cx).map(|here| here.level),
        )
    }
    fn selected_booth_still_present(
        selected: &BoothSelection,
        rows: &[LocView],
        current_level: Option<i32>,
    ) -> bool {
        if selected.name.is_none() && selected.action.is_none() {
            return rows
                .iter()
                .any(|row| row.id == selected.id && row.tile == selected.tile);
        }
        rows.iter().any(|row| {
            row.id == selected.id
                && row.tile == selected.tile
                && row.name.as_deref() == selected.name.as_deref()
                && selected.action.as_deref().is_some_and(|action| {
                    row.actions.iter().any(|got| {
                        got.as_deref()
                            .is_some_and(|got| got.eq_ignore_ascii_case(action))
                    })
                })
                && current_level.is_none_or(|level| level == row.tile.level)
        })
    }

    fn live_approach(&self, cx: &ActionContext<'_>) -> Option<BoothApproach> {
        let selected = self.selected.as_ref()?;
        let snapshot = cx.snapshot();
        let locs = snapshot.locs()?;
        let loc = locs
            .value
            .iter()
            .find(|row| row.id == selected.id && row.tile == selected.tile)?;
        let approach = snapshot.booth_approach(loc)?.value;
        Some(booth_approach(approach.can_operate, approach.dest))
    }

    fn approach_or_open(
        &mut self,
        cx: &mut ActionContext<'_>,
    ) -> Poll<Result<Option<bool>, ActionError>> {
        match self.live_approach(cx) {
            Some(BoothApproach::Operate) => match self.send_open(cx) {
                Ok(()) => Poll::Pending,
                Err(ActionError::BudgetExhausted) => Poll::Pending,
                Err(error) => Poll::Ready(Err(error)),
            },
            Some(BoothApproach::Walk(destination)) => {
                let deadline = TimeBound::new(cx, WALK_BOUND_MS);
                self.start_approach(cx, destination, deadline)
                    .map(|result| result.map(Some))
            }
            Some(BoothApproach::Unavailable) | None => Poll::Ready(Ok(Some(false))),
        }
    }

    fn start_approach(
        &mut self,
        cx: &mut ActionContext<'_>,
        destination: WorldTile,
        deadline: TimeBound,
    ) -> Poll<Result<bool, ActionError>> {
        if self.last_approach_dest == Some(destination) {
            if let Some(walk) = self.walk.as_mut() {
                if let Err(error) = walk.resume(cx, destination, 0, access_walk_options()) {
                    if error != ActionError::BudgetExhausted {
                        return Poll::Ready(Err(error));
                    }
                }
            }
            self.phase = BoothPhase::WaitSelected { deadline };
            return Poll::Pending;
        }
        match cx.walk(walk_request(cx, destination, 0, access_walk_options())) {
            Ok(request_id) => {
                self.walk = Some(OpeningWalk::new(cx, request_id));
                self.last_approach_dest = Some(destination);
                self.phase = BoothPhase::WaitSelected { deadline };
                Poll::Pending
            }
            Err(ActionError::BudgetExhausted) => {
                self.phase = BoothPhase::WaitSelected { deadline };
                Poll::Pending
            }
            Err(error) => Poll::Ready(Err(error)),
        }
    }

    fn send_open(&mut self, cx: &mut ActionContext<'_>) -> Result<(), ActionError> {
        let Some(selected) = self.selected.as_ref() else {
            return Err(unavailable("bank booth selection unavailable"));
        };
        cx.emit(InteractReq::OpenBooth {
            x: selected.tile.x,
            z: selected.tile.z,
            level: selected.tile.level,
            id: selected.id,
            name: selected.name.clone(),
            action: selected.action.clone(),
        })?;
        self.open_generation = bank_state(cx).2;
        self.phase = BoothPhase::WaitFresh {
            deadline: TimeBound::new(cx, BANK_READY_MS),
        };
        Ok(())
    }
}

fn nearest_use_quickly(rows: &[LocView], here: WorldTile, level: i32) -> Option<&LocView> {
    rows.iter()
        .filter(|row| {
            row.tile.level == level
                && row.actions.iter().any(|action| {
                    action
                        .as_deref()
                        .is_some_and(|action| action.eq_ignore_ascii_case("Use-quickly"))
                })
        })
        .min_by_key(|row| row.tile.x.abs_diff(here.x).max(row.tile.z.abs_diff(here.z)))
}

fn nearest_scene_booth_distance(
    cx: &ActionContext<'_>,
    booth_name: &str,
    booth_op: &str,
) -> Option<i32> {
    let snapshot = cx.snapshot();
    let rows = snapshot.locs()?.value;
    let here = current_tile(cx);
    let row_distance = rows
        .iter()
        .filter(|row| {
            row.name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(booth_name))
                && row.actions.iter().any(|action| {
                    action
                        .as_deref()
                        .is_some_and(|op| !op.is_empty() && op != "hidden")
                })
        })
        .map(|row| row.distance)
        .min();
    let nearest = here.and_then(|here| {
        nearest_use_quickly(rows, here, here.level)
            .filter(|row| {
                row.name
                    .as_deref()
                    .is_some_and(|name| name.eq_ignore_ascii_case(booth_name))
                    && row.actions.iter().any(|op| {
                        op.as_deref()
                            .is_some_and(|op| op.eq_ignore_ascii_case(booth_op))
                    })
            })
            .map(|row| row.tile.x.abs_diff(here.x).max(row.tile.z.abs_diff(here.z)) as i32)
    });
    row_distance.into_iter().chain(nearest).min()
}

struct SelectionResult {
    bank: NamedBank,
    tile: WorldTile,
    selected: SelectedBank,
}

fn no_candidate_selection() -> SelectedBank {
    SelectedBank {
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

enum WorldTarget {
    Access(Option<AccessSpec>),
    Preset(WorldTile),
}

#[derive(Clone)]
struct AccessSpec {
    npc: Option<NpcAccessArgs>,
    object: Option<ObjectAccess>,
}

impl AccessSpec {
    fn from_wire(destination: &BankSeatDestination) -> Self {
        Self {
            npc: destination.npc_access.as_ref().map(npc_access),
            object: destination.object_access.as_ref().map(object_access),
        }
    }

    fn from_bank(bank: NamedBank) -> Self {
        let definition = bank.definition;
        let npc = definition.and_then(|definition| {
            definition.npc.map(|npc| NpcAccessArgs {
                name: npc.name.to_owned(),
                op: npc.op.to_owned(),
                choose: definition.choose.unwrap_or_default().to_owned(),
            })
        });
        let object = definition.and_then(|definition| {
            if definition.npc.is_some() {
                return None;
            }
            definition.object.map(|object| ObjectAccess {
                name: object.name.to_owned(),
                op: object.op.to_owned(),
                open_first: definition
                    .open_first
                    .map(|opener| (opener.name.to_owned(), opener.op.to_owned())),
            })
        });
        Self { npc, object }
    }
}

struct WorldWalk {
    follow: OpeningWalk,
    target: WorldTile,
    radius: i32,
    deadline: TimeBound,
    next: WorldTarget,
}

enum BankingPhase {
    Choose,
    Select(SelectorCore),
    WalkStart {
        target: WorldTile,
        radius: i32,
        next: WorldTarget,
    },
    Walk(WorldWalk),
    Access(AccessOpen),
    Booth(BoothOpen),
    Ready(TimeBound),
    Done(bool),
}

struct BankingOpen {
    phase: BankingPhase,
    stand: Option<WorldTile>,
    destination: Option<BankSeatDestination>,
    booth_name: String,
    booth_op: String,
    prefer_nearby: bool,
    nearby_radius: f64,
    unsupported_obstacles: bool,
    preferences: BankPreferences,
}

impl BankingOpen {
    fn new(request: &BankSeatArgs, preferences: BankPreferences) -> Self {
        let (booth_name, booth_op) = request.object_access.as_ref().map_or_else(
            || ("Bank booth".to_owned(), "Use-quickly".to_owned()),
            |access| (access.name.clone(), access.op.clone()),
        );
        Self {
            phase: BankingPhase::Choose,
            stand: request.stand,
            destination: request.destination.clone(),
            booth_name,
            booth_op,
            prefer_nearby: request.prefer_nearby != Some(false),
            nearby_radius: request.nearby_radius.unwrap_or(14.0),
            unsupported_obstacles: request.obstacles.iter().any(|name| !name.trim().is_empty()),
            preferences,
        }
    }

    fn poll(
        &mut self,
        cx: &mut ActionContext<'_>,
        facts: &Arc<NamedBankFacts>,
        log: &mut LogCall,
        log_enabled: bool,
    ) -> Poll<Result<bool, ActionError>> {
        loop {
            let phase = std::mem::replace(&mut self.phase, BankingPhase::Choose);
            match phase {
                BankingPhase::Choose => match self.choose(cx, facts) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                    Poll::Ready(Ok(())) => continue,
                },
                BankingPhase::Select(mut select) => match select.poll(cx) {
                    Poll::Pending => {
                        self.phase = BankingPhase::Select(select);
                        return Poll::Pending;
                    }
                    Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                    Poll::Ready(Ok(None)) => {
                        api::host_log!(
                            api::hostlog::Category::BankOp,
                            api::hostlog::Level::Warn,
                            "[DEBUG-cbf] selection none tick={} here={:?}",
                            cx.evidence().tick, current_tile(cx),
                        );
                        self.phase = BankingPhase::Access(self.default_access());
                    }
                    Poll::Ready(Ok(Some(selection))) => {
                        api::host_log!(
                            api::hostlog::Category::BankOp,
                            api::hostlog::Level::Warn,
                            "[DEBUG-cbf] selection bank={} tile={:?} kind={:?} tick={} here={:?}",
                            selection.bank.name, selection.tile, selection.selected.kind,
                            cx.evidence().tick, current_tile(cx),
                        );
                        self.walk_to(
                            cx,
                            selection.tile,
                            4,
                            WorldTarget::Access(Some(AccessSpec::from_bank(selection.bank))),
                        );
                    }
                },
                BankingPhase::WalkStart {
                    target,
                    radius,
                    next,
                } => {
                    if walk_arrived(cx, target, radius) {
                        self.open_target(next);
                        continue;
                    }
                    match cx.walk(walk_request(cx, target, radius, world_walk_options())) {
                        Ok(request_id) => {
                            self.phase = BankingPhase::Walk(WorldWalk {
                                follow: OpeningWalk::new(cx, request_id),
                                target,
                                radius,
                                deadline: TimeBound::new(cx, WORLD_WALK_TIMEOUT_MS),
                                next,
                            });
                            return Poll::Pending;
                        }
                        Err(ActionError::BudgetExhausted) => {
                            self.phase = BankingPhase::WalkStart {
                                target,
                                radius,
                                next,
                            };
                            return Poll::Pending;
                        }
                        Err(_) => {
                            self.open_target(next);
                            continue;
                        }
                    }
                }
                BankingPhase::Walk(mut walk) => {
                    let terminal = walk.follow.session == cx.run().session
                        && cx
                            .walk_receipt(walk.follow.request_id)
                            .is_some_and(|receipt| receipt.end != WalkEnd::Cancelled);
                    if walk_arrived(cx, walk.target, walk.radius)
                        || terminal
                        || walk.deadline.reached(cx)
                    {
                        if walk.deadline.reached(cx) {
                            cx.cancel_request(walk.follow.request_id);
                        }
                        self.open_target(walk.next);
                        continue;
                    }
                    if let Err(error) =
                        walk.follow
                            .resume(cx, walk.target, walk.radius, world_walk_options())
                    {
                        if error != ActionError::BudgetExhausted {
                            return Poll::Ready(Err(error));
                        }
                    }
                    self.phase = BankingPhase::Walk(walk);
                    return Poll::Pending;
                }
                BankingPhase::Access(mut open) => match open.poll(cx, log, log_enabled) {
                    Poll::Pending => {
                        self.phase = BankingPhase::Access(open);
                        return Poll::Pending;
                    }
                    Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                    Poll::Ready(Ok(value)) => return Poll::Ready(Ok(value)),
                },
                BankingPhase::Booth(mut open) => match open.poll(cx, log, log_enabled) {
                    Poll::Pending => {
                        self.phase = BankingPhase::Booth(open);
                        return Poll::Pending;
                    }
                    Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                    Poll::Ready(Ok(value)) => return Poll::Ready(Ok(value)),
                },
                BankingPhase::Ready(deadline) => {
                    let (open, loaded, _) = bank_state(cx);
                    if open && loaded {
                        return Poll::Ready(Ok(true));
                    }
                    if deadline.reached(cx) {
                        return Poll::Ready(Ok(false));
                    }
                    self.phase = BankingPhase::Ready(deadline);
                    return Poll::Pending;
                }
                BankingPhase::Done(value) => return Poll::Ready(Ok(value)),
            }
        }
    }

    fn choose(
        &mut self,
        cx: &mut ActionContext<'_>,
        facts: &Arc<NamedBankFacts>,
    ) -> Poll<Result<(), ActionError>> {
        let (open, loaded, _) = bank_state(cx);
        if open {
            if loaded {
                self.phase = BankingPhase::Done(true);
                return Poll::Ready(Ok(()));
            }
            self.phase = BankingPhase::Ready(TimeBound::new(cx, BANK_READY_MS));
            return Poll::Pending;
        }
        let here = current_tile(cx);
        let scene_booth = nearest_scene_booth_distance(cx, &self.booth_name, &self.booth_op);
        let nearest =
            here.and_then(|here| nearest_eligible_bank(facts, self.preferences, cx, here));
        api::host_log!(
            api::hostlog::Category::BankOp,
            api::hostlog::Level::Warn,
            "[DEBUG-cbf] choose tick={} here={here:?} scene_booth={scene_booth:?} nearest={nearest:?}",
            cx.evidence().tick,
        );
        if self.prefer_nearby
            && scene_booth.is_some_and(|distance| f64::from(distance) <= self.nearby_radius)
        {
            self.phase = BankingPhase::Access(self.default_access());
        } else if let Some(bank) = nearest.filter(|bank| {
            self.prefer_nearby
                && here.is_some_and(|here| {
                    within_radius(here, bank.tile, self.nearby_radius)
                        && self
                            .stand
                            .is_none_or(|stand| !within_radius(here, stand, self.nearby_radius))
                })
        }) {
            self.walk_to(
                cx,
                bank.tile,
                4,
                WorldTarget::Access(Some(AccessSpec::from_bank(bank))),
            );
        } else if let Some(stand) = self.stand {
            if self.unsupported_obstacles {
                return Poll::Ready(Err(failed("not impl: Banking.open.obstacles")));
            }
            let known = nearest_eligible_bank(facts, self.preferences, cx, stand)
                .filter(|bank| within_radius(stand, bank.tile, self.nearby_radius));
            let access = known.map(AccessSpec::from_bank);
            let target = if access
                .as_ref()
                .is_some_and(|access| access.npc.is_some() || access.object.is_some())
            {
                WorldTarget::Access(access)
            } else {
                WorldTarget::Preset(stand)
            };
            self.walk_to(cx, stand, 2, target);
        } else if scene_booth.is_some() {
            self.phase = BankingPhase::Access(self.default_access());
        } else if let Some(destination) = self.destination.take() {
            let tile = destination.tile;
            self.walk_to(
                cx,
                tile,
                4,
                WorldTarget::Access(Some(AccessSpec::from_wire(&destination))),
            );
        } else if let Some(from) = here {
            match SelectorCore::begin(
                Arc::clone(facts),
                self.preferences,
                Some(from),
                true,
                false,
                cx,
            )? {
                Some(select) => self.phase = BankingPhase::Select(select),
                None => self.phase = BankingPhase::Access(self.default_access()),
            }
        } else {
            self.phase = BankingPhase::Access(self.default_access());
        }
        Poll::Ready(Ok(()))
    }

    fn default_access(&self) -> AccessOpen {
        AccessOpen::new(
            None,
            None,
            None,
            default_access(&self.booth_name, &self.booth_op),
        )
    }

    fn walk_to(
        &mut self,
        _cx: &mut ActionContext<'_>,
        target: WorldTile,
        radius: i32,
        next: WorldTarget,
    ) {
        self.phase = BankingPhase::WalkStart {
            target,
            radius,
            next,
        };
    }

    fn open_target(&mut self, target: WorldTarget) {
        self.phase = match target {
            WorldTarget::Access(access) => {
                let (npc, object) = access
                    .map(|access| (access.npc, access.object))
                    .unwrap_or((None, None));
                BankingPhase::Access(AccessOpen::new(
                    npc,
                    object,
                    None,
                    default_access(&self.booth_name, &self.booth_op),
                ))
            }
            WorldTarget::Preset(stand) => BankingPhase::Booth(BoothOpen::new(
                BoothMode::Booth,
                Some(stand),
                Some(default_access(&self.booth_name, &self.booth_op)),
            )),
        };
    }
}

fn within_radius(a: WorldTile, b: WorldTile, radius: f64) -> bool {
    if radius < 0.0 {
        return false;
    }
    let dx = (i64::from(a.x) - i64::from(b.x)) as f64;
    let dz = (i64::from(a.z) - i64::from(b.z)) as f64;
    dx * dx + dz * dz <= radius * radius
}

fn nearest_eligible_bank(
    facts: &Arc<NamedBankFacts>,
    preferences: BankPreferences,
    cx: &ActionContext<'_>,
    from: WorldTile,
) -> Option<NamedBank> {
    let snapshot = cx.snapshot();
    let stats = snapshot.stats().map_or(&[][..], |rows| rows.value);
    let quests = snapshot.quest_statuses().map_or(&[][..], |rows| rows.value);
    nearest_eligible_bank_from_rows(facts, preferences, stats, quests, from)
}

fn nearest_eligible_bank_from_rows(
    facts: &Arc<NamedBankFacts>,
    preferences: BankPreferences,
    stats: &[api::snapshot::StatView],
    quests: &[api::snapshot::QuestStatusView],
    from: WorldTile,
) -> Option<NamedBank> {
    let selector = BankSelector::new(Arc::clone(facts), preferences);
    select_nearest_eligible_bank(facts.as_ref(), from, |bank| {
        selector.eligible(bank, stats, quests)
    })
}

struct SelectorCore {
    machine: Select,
    facts: Arc<NamedBankFacts>,
}

impl SelectorCore {
    fn begin(
        facts: Arc<NamedBankFacts>,
        preferences: BankPreferences,
        start: Option<WorldTile>,
        allow_wilderness: bool,
        allow_air_fallback: bool,
        cx: &mut ActionContext<'_>,
    ) -> Result<Option<Self>, ActionError> {
        let Some(from) = start.or_else(|| current_tile(cx)) else {
            return Ok(None);
        };
        let options = WalkOptions {
            allow_teleports: WalkBit::Forbid,
            allow_wilderness: if allow_wilderness {
                WalkBit::Allow
            } else {
                WalkBit::Forbid
            },
            allow_danger_zones: WalkBit::Inherit,
        };
        let machine = Select::begin(
            SelectArgs {
                facts: Arc::clone(&facts),
                from,
                preferences,
                options,
                explicit: None,
                allow_air_fallback,
            },
            cx,
        )?;
        Ok(Some(Self { machine, facts }))
    }

    fn poll(
        &mut self,
        cx: &mut ActionContext<'_>,
    ) -> Poll<Result<Option<SelectionResult>, ActionError>> {
        self.machine
            .poll(cx)
            .map(|result| result.map(|selected| selected_bank(&self.facts, selected)))
    }
}

fn selected_bank(facts: &NamedBankFacts, selected: SelectedBank) -> Option<SelectionResult> {
    if selected.kind == PickKind::NoCandidate {
        return None;
    }
    facts
        .banks()
        .get(usize::from(selected.bank_index))
        .copied()
        .map(|bank| SelectionResult {
            bank,
            tile: selected.access_tile,
            selected,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::{ledger, HostEffect};
    use crate::quester::families::tests::with_tick;
    use api::snapshot::GameSnapshot;

    fn duel_chest(open: bool) -> LocView {
        LocView {
            typecode: 0,
            info: 0,
            id: if open { 3194 } else { 3193 },
            name: Some(if open { "Open chest" } else { "Closed chest" }.into()),
            description: None,
            actions: if open {
                vec![None, Some("Bank".into())]
            } else {
                vec![Some("Open".into())]
            },
            tile: tile(3382, 3268),
            distance: 1,
            layer: api::snapshot::LocLayer::Ground,
            shape: 10,
            angle: 0,
            width: 1,
            length: 1,
            footprint_width: 1,
            footprint_length: 1,
            block_walk: true,
            block_range: false,
            active: true,
            animation: -1,
            map_function: -1,
            map_scene: -1,
            force_approach: 0,
        }
    }

    fn duel_chest_opening() -> OpeningArgs {
        // 289 game_duelarena/configs/duelarena.loc:70-83 and
        // scripts/misc_locs.rs2:6-13 require Closed chest/Open, then Open chest/Bank.
        let mut request = BankSeatArgs::new(BankSeatOperation::OpenObject);
        request.object_access = Some(BankSeatObjectAccess {
            name: "Open chest".into(),
            op: "Bank".into(),
            open_first_name: Some("Closed chest".into()),
            open_first_op: Some("Open".into()),
        });
        OpeningArgs {
            request,
            facts: Arc::new(NamedBankFacts::from_banks(Vec::new())),
            preferences: BankPreferences::default(),
            hooks: Arc::new(Mutex::new(TransferHooks::default())),
        }
    }

    fn take_chest_action(ledger: &mut Option<Box<ledger::Ledger>>) -> (i32, String) {
        let effects = &mut ledger.as_mut().unwrap().outbox;
        assert_eq!(effects.len(), 1, "exactly one chest action");
        let HostEffect::Interaction(InteractReq::Loc { id, action, .. }) = effects.remove(0).effect
        else {
            panic!("expected a chest interaction");
        };
        (id.unwrap(), action)
    }

    #[test]
    fn closed_duel_chest_opens_before_banking_and_waits_for_the_new_state() {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_tile(tile(3382, 3269));
        snapshot.seed_locs(vec![duel_chest(false)]);
        let mut ledger = None;
        let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
            let handle = tick
                .actions
                .begin::<Opening>(duel_chest_opening(), &mut tick.cx)
                .unwrap();
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            handle
        });
        assert_eq!(take_chest_action(&mut ledger), (3193, "Open".into()));
        with_tick(&snapshot, &mut ledger, 2, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
        snapshot.seed_locs(vec![duel_chest(true)]);
        with_tick(&snapshot, &mut ledger, 3, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        assert_eq!(take_chest_action(&mut ledger), (3194, "Bank".into()));
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), Vec::new());
        with_tick(&snapshot, &mut ledger, 4, |tick| {
            assert_eq!(
                tick.actions.poll(&handle, &mut tick.cx),
                Poll::Ready(Ok(OpeningValue::Bool(true)))
            );
        });
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }

    #[test]
    fn already_open_duel_chest_skips_the_opener() {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_tile(tile(3382, 3269));
        snapshot.seed_locs(vec![duel_chest(true)]);
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 1, |tick| {
            let handle = tick
                .actions
                .begin::<Opening>(duel_chest_opening(), &mut tick.cx)
                .unwrap();
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        });
        assert_eq!(take_chest_action(&mut ledger), (3194, "Bank".into()));
    }

    #[test]
    fn already_loaded_bank_skips_all_chest_actions() {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_locs(vec![duel_chest(false)]);
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), Vec::new());
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 1, |tick| {
            let handle = tick
                .actions
                .begin::<Opening>(duel_chest_opening(), &mut tick.cx)
                .unwrap();
            assert_eq!(
                tick.actions.poll(&handle, &mut tick.cx),
                Poll::Ready(Ok(OpeningValue::Bool(true)))
            );
        });
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }

    fn tile(x: i32, z: i32) -> WorldTile {
        WorldTile { x, z, level: 0 }
    }

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

    #[test]
    fn banking_open_preferences_gate_the_closer_mage_arena_choice() {
        let mage = catalog_bank("Mage Arena");
        let public = NamedBank::new("Public", tile(mage.air_tile().x + 1, mage.air_tile().z));
        let facts = Arc::new(NamedBankFacts::from_banks(vec![public, mage]));
        let stats = [];
        let quests = [];
        let from = mage.air_tile();
        let ordinary_bank = nearest_eligible_bank_from_rows(
            &facts,
            BankPreferences {
                use_mage_bank: false,
                use_zanaris_bank: false,
            },
            &stats,
            &quests,
            from,
        );
        let opted_bank = nearest_eligible_bank_from_rows(
            &facts,
            BankPreferences {
                use_mage_bank: true,
                use_zanaris_bank: false,
            },
            &stats,
            &quests,
            from,
        );
        assert_eq!(ordinary_bank.map(|bank| bank.name), Some("Public"));
        assert_eq!(opted_bank.map(|bank| bank.name), Some("Mage Arena"));
    }

    #[test]
    fn opener_precedes_target_and_destination_access_keeps_npc_priority() {
        let request = BankSeatObjectAccess {
            name: "Bank chest".into(),
            op: "Use".into(),
            open_first_name: Some("Door".into()),
            open_first_op: Some("Open".into()),
        };
        let access = object_access(&request);
        assert_eq!(access.open_first, Some(("Door".into(), "Open".into())));
        assert!(matches!(
            object_start_phase(&access, false, false),
            ObjectPhase::OpenerFind(0)
        ));
        assert!(matches!(
            opener_gate_phase(false, true),
            ObjectPhase::NearFind(0)
        ));
        let npc = NpcAccessArgs {
            name: "Banker".into(),
            op: "Bank".into(),
            choose: "Bank".into(),
        };
        let open = AccessOpen::new(
            Some(npc.clone()),
            Some(access),
            None,
            default_access("Bank booth", "Use-quickly"),
        );
        assert!(matches!(open.inner, AccessMachine::Npc(_)));
        assert_eq!(npc.choose, "Bank");
    }

    #[test]
    fn booth_selection_requires_current_plane_and_preserves_named_identity() {
        let row = LocView {
            typecode: 0,
            info: 0,
            id: 12,
            name: Some("Bank booth".into()),
            description: None,
            actions: vec![Some("Use-quickly".into())],
            tile: tile(5, 5),
            distance: 2,
            layer: api::snapshot::LocLayer::Ground,
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
        };
        let rows = [row.clone()];
        let selected = ops::booth_named(&rows, "bank booth", "use-quickly", Some(0)).unwrap();
        assert_eq!(selected.id, 12);
        let selected = BoothSelection {
            id: 12,
            tile: row.tile,
            name: row.name.clone(),
            action: Some("Use-quickly".into()),
        };
        assert!(BoothOpen::selected_booth_still_present(
            &selected,
            &rows,
            Some(0)
        ));
        assert!(!BoothOpen::selected_booth_still_present(
            &selected,
            &rows,
            Some(1)
        ));
        assert_eq!(nearest_use_quickly(&rows, tile(4, 5), 0).unwrap().id, 12);
        assert!(nearest_use_quickly(
            &rows,
            WorldTile {
                level: 1,
                ..tile(4, 5)
            },
            1
        )
        .is_none());
    }
    #[test]
    fn approach_uses_reachable_counter_tile_and_preserves_route_arrival() {
        let loc = tile(10, 10);
        let here = Some(tile(9, 9));
        let stand = nearest_reachable_stand(loc, here, |candidate| {
            candidate == tile(9, 10) || candidate == tile(10, 9)
        });
        assert_eq!(stand, Some(tile(9, 10)));

        let evidence = api::quest_progress::EvidenceStamp {
            run: api::selected::RunKey {
                slot: 0,
                run: 0,
                session: 0,
            },
            tick: 7,
            sequence: 3,
        };
        let request = build_walk_request(evidence, stand.unwrap(), 1, access_walk_options());
        assert_eq!(request.target, tile(9, 10));
        assert_eq!(request.radius, 1);
        assert_eq!(request.arrival, ArrivalKind::Reach);
        assert_eq!(request.required_after, evidence);
        assert_eq!(request.options.allow_teleports, WalkBit::Forbid);
        assert_eq!(request.options.allow_wilderness, WalkBit::Allow);
        assert_eq!(request.options.allow_danger_zones, WalkBit::Inherit);

        assert!(matches!(
            booth_approach(false, Some(tile(8, 8))),
            BoothApproach::Walk(WorldTile {
                x: 8,
                z: 8,
                level: 0
            })
        ));
        assert!(matches!(booth_approach(true, None), BoothApproach::Operate));
        assert!(matches!(
            booth_approach(false, None),
            BoothApproach::Unavailable
        ));
    }

    struct FakeNpcContext {
        open: bool,
        loaded: bool,
        tick: u64,
        chat: npc::Chat,
        banker: bool,
        choice: Option<i32>,
        deadline: Option<TickBound>,
        emitted: Vec<InteractReq>,
        cx_time: Duration,
    }

    impl Default for FakeNpcContext {
        fn default() -> Self {
            Self {
                open: false,
                loaded: false,
                tick: 0,
                chat: npc::Chat {
                    tick: 0,
                    modal: -1,
                    can_continue: false,
                },
                banker: false,
                choice: None,
                deadline: None,
                emitted: Vec::new(),
                cx_time: Duration::ZERO,
            }
        }
    }
    fn bank_npc_intent() -> npc::Intent<'static> {
        npc::Intent {
            name: "Banker",
            op: npc::NpcOp::Name("Bank"),
            choose: "Bank",
        }
    }

    impl npc::Context for FakeNpcContext {
        type Error = ();

        fn bank_open(&self) -> bool {
            self.open
        }
        fn bank_loaded(&self) -> bool {
            self.loaded
        }
        fn chat(&self) -> npc::Chat {
            npc::Chat {
                tick: self.tick,
                ..self.chat
            }
        }
        fn banker(&self, _name: &str, _op: npc::NpcOp<'_>) -> Option<npc::Banker> {
            self.banker.then(|| npc::Banker {
                name: "Banker".into(),
                action: "Bank".into(),
                index: 3,
            })
        }
        fn choice(&self, _choose: &str) -> Option<i32> {
            self.choice
        }
        fn arm(&mut self, millis: u64) {
            self.deadline = Some(TickBound {
                tick_deadline: self
                    .tick
                    .saturating_add(ops::observed_ticks_for_millis(millis)),
                active_deadline: self.cx_time.saturating_add(Duration::from_millis(millis)),
            });
        }
        fn expired(&mut self) -> bool {
            self.deadline.is_some_and(|deadline| {
                self.tick >= deadline.tick_deadline && self.cx_time >= deadline.active_deadline
            })
        }
        fn emit(&mut self, request: InteractReq) -> Result<(), Self::Error> {
            self.emitted.push(request);
            Ok(())
        }
    }

    #[test]
    fn npc_access_uses_fixed_choice() {
        let mut access = npc::NpcAccess::new();
        let mut context = FakeNpcContext {
            banker: true,
            choice: Some(2),
            ..FakeNpcContext::default()
        };
        assert!(matches!(
            access.step(bank_npc_intent(), &mut context),
            Ok(npc::Step::Wait)
        ));
        assert!(matches!(
            context.emitted.first(),
            Some(InteractReq::Npc { .. })
        ));
        context.chat = npc::Chat {
            tick: 1,
            modal: 5,
            can_continue: true,
        };
        assert!(matches!(
            access.step(bank_npc_intent(), &mut context),
            Ok(npc::Step::Wait)
        ));
        assert!(matches!(
            context.emitted.last(),
            Some(InteractReq::Answer { option: 2 })
        ));
    }

    #[test]
    fn npc_dialogue_timeout_requires_tick_and_active_time() {
        let mut access = npc::NpcAccess::new();
        let mut context = FakeNpcContext {
            banker: true,
            ..FakeNpcContext::default()
        };
        assert!(matches!(
            access.step(bank_npc_intent(), &mut context),
            Ok(npc::Step::Wait)
        ));
        context.cx_time = Duration::from_millis(npc::NPC_DIALOG_MS);
        context.tick = ops::observed_ticks_for_millis(npc::NPC_DIALOG_MS).saturating_sub(1);
        assert!(matches!(
            access.step(bank_npc_intent(), &mut context),
            Ok(npc::Step::Wait)
        ));
        context.tick = context.tick.saturating_add(1);
        assert!(matches!(
            access.step(bank_npc_intent(), &mut context),
            Ok(npc::Step::Note(npc::Note::NoDialogue))
        ));
        assert_eq!(context.emitted.len(), 1);
    }

    #[test]
    fn native_walk_requests_use_reach_arrival_and_preserve_the_target() {
        let evidence = api::quest_progress::EvidenceStamp {
            run: api::selected::RunKey {
                slot: 0,
                run: 0,
                session: 0,
            },
            tick: 0,
            sequence: 0,
        };
        let request = build_walk_request(evidence, tile(10, 20), 1, access_walk_options());
        assert_eq!(request.target, tile(10, 20));
        assert_eq!(request.radius, 1);
        assert_eq!(request.arrival, ArrivalKind::Reach);
        assert_eq!(request.required_after, evidence);
        assert_eq!(request.options.allow_wilderness, WalkBit::Allow);
    }
    #[test]
    fn opening_log_text_stays_inside_prompt_budget_and_utf8_boundary() {
        fn encoded_len(line: &str) -> usize {
            let args = [Value::String(line.to_owned())];
            crate::channel::encode_arguments(&args)
                .expect("opening log text should fit the prompt budget")
                .len()
        }

        let mut log = LogCall::default();
        let name = "n".repeat(300);
        assert_eq!(name.len(), 300);
        log.queue(format!("no usable '{name}' in the scene"), true);
        let line = log
            .line
            .as_deref()
            .expect("the opening log should be queued");
        assert_eq!(line.len(), MAX_LOG_TEXT_BYTES);
        assert_eq!(
            encoded_len(line),
            api::script_contract::PROMPT_ARGS_MAX_BYTES
        );

        let multibyte = format!("{}é", "x".repeat(MAX_LOG_TEXT_BYTES - 1));
        log.queue(multibyte, true);
        let line = log
            .line
            .as_deref()
            .expect("the opening log should be queued");
        assert_eq!(line.len(), MAX_LOG_TEXT_BYTES - 1);
        assert!(line.is_char_boundary(line.len()));
        assert!(encoded_len(line) <= api::script_contract::PROMPT_ARGS_MAX_BYTES);
    }
}
