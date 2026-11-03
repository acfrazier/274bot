//! Content-witnessed prerequisite execution under the existing native owners.
use crate::dialogue_outcome::DialogueOutcome;
use crate::local_state::SlotState;
use crate::native::walk::Walk;
use crate::native::{
    ActionContext, ActionError, ActionHandle, NativeActions, NativeMachine, OuterWalk, PrereqAllow,
    PrereqFailure, PrereqMissing, PrereqRefusal, PrereqStepIdentity, PrereqSwitch, PrereqWaitName,
    TalkReachCause, WalkAllow, WalkEnd, WalkGoals, WalkOptions, WalkRequest,
};
use crate::quest_journal::{JournalMachine, JournalRequest};
use crate::quester::families::dialogue::{
    Dialogue, DialogueArgs, DialogueOptions, DialogueSend, DialogueSendKind, DialogueTarget,
    LineRule, PageAcknowledgement, DIALOG_GAP_TICKS,
};
use crate::quester::families::{reach, InteractRun};
use crate::shim::InteractReq;
use api::prereq_knowledge::Observation;
use api::prereq_knowledge::{Fact, FactSource, RefusalObservation, SessionMemory};
use api::prereq_steps::{
    ActionableGate, BranchOutcome, ConsentSwitch, OpenerRequirement, PrereqCatalog, PrereqJournal,
    PrereqPrecondition, PrereqStep, RefusalThen,
};
use api::quest_progress::{EvidenceProvider, EvidenceStamp};
use api::selected::{InclusiveRange, QuestGate, Truth};
use api::snapshot::chat_page_matches;
use api::WorldTile;
use std::sync::Arc;
use std::task::Poll;

// Engine: OpLocHandler.ts:52-56, Player.ts:1287-1295 (one stationary op cycle).
pub(crate) const OP_TICKS: u16 = 1;
// Content: chat.rs2:269-279,323-332; engine: ResumePauseButtonHandler.ts:7-13.
pub(crate) const PAGE_TICKS: u16 = 1;
// Engine: PathingEntity.ts:135-151, World.ts:367,378 (leash plus NPC movement).
pub(crate) const NPC_STEP_TICKS: u16 = 2;
// Accepted send allowance, not a content guarantee. Live cells pin observed counts.
pub(crate) const SEND_SLACK_TICKS: u16 = 2;
// One retry covers the host run-press race after a journal modal timeout.
// The host run-on latch remains set until energy reaches 0 or the session changes.
// quest_journal.rs2:1-55 has no delay. Retry only on the next observed tick
// after the reader's five-tick window, never again within that same tick.
const JOURNAL_TIMEOUT_RETRIES: u8 = 1;
// A Talk the engine cancels ("I can't reach that!", Player.ts:1292-1295) is not a
// slow page: the engine clears the interaction in the cycle after the send, so
// waiting longer cannot recover it. The runner reads both tiles again on a later
// observed tick (Drezel starts a wander on one tick in eight, Npc.ts:723, range 5,
// NpcType.ts:101,110; the wait of a cancelled Talk ends after one tick), walks to
// the NPC if it is more than two tiles away, and sends the Talk again. A
// re-approach that ends while the NPC is not ready to talk is a failed attempt
// of the same budget. The budget is per conversation (one `ChatDrive`): the
// first attempt and three more, so at most four Talks per conversation.
// `PrereqBudget::talks` counts the conversation once, so a later conversation of
// the same step starts a new count. An allowance, not a content guarantee. Then
// the step fails with the last failure's detail (`PrereqFailure::TalkUnreachable`).
const TALK_REACH_RETRIES: u8 = 3;

/// Repeated polls and scene changes cannot spend this wait: only observed ticks do.
#[derive(Clone, Copy)]
pub(crate) struct TickWait {
    since: u64,
    bound: u16,
    chat_mark: i32,
    page: PageAcknowledgement,
    here: Option<WorldTile>,
}
impl TickWait {
    fn new(cx: &ActionContext<'_>, bound: u16) -> Self {
        Self {
            since: cx.evidence().tick,
            bound,
            chat_mark: reach::last_chat_seq(cx),
            page: page(cx),
            here: cx.snapshot().here().map(|here| here.value),
        }
    }
    fn started(self, name: PrereqWaitName) -> Self {
        api::host_log!(
            api::hostlog::Category::NavEvent,
            api::hostlog::Level::Info,
            "prereq wait-start={name:?} since={} bound={} from={:?}",
            self.since,
            self.bound,
            self.here
        );
        self
    }
    fn spent(self, tick: u64) -> bool {
        tick.saturating_sub(self.since) > u64::from(self.bound)
    }
    fn failure(self, wait: PrereqWaitName, tick: u64) -> PrereqFailure {
        wait_log(wait, self, tick, "spent");
        PrereqFailure::WaitSpent {
            wait,
            since: self.since,
            at: tick,
            bound: self.bound,
        }
    }
    fn changed(self, cx: &ActionContext<'_>) -> bool {
        let now = page(cx);
        self.page != now
    }
}

fn wait_log(name: PrereqWaitName, wait: TickWait, at: u64, end: &str) {
    api::host_log!(
        api::hostlog::Category::NavEvent,
        api::hostlog::Level::Info,
        "prereq wait={name:?} since={} at={at} count={} bound={} end={end} from={:?}",
        wait.since,
        at.saturating_sub(wait.since),
        wait.bound,
        wait.here
    );
}

/// Borrow the original policy only while polling. The runner retains no
/// second request or permission box between ticks.
#[derive(Clone, Copy, Default)]
pub(crate) struct RepairWalk<'a> {
    pub(super) options: WalkOptions,
    pub(super) protect: bool,
    pub(super) food_guard: bool,
    pub(super) allow: WalkAllow,
    pub(super) cross: &'a [Arc<str>],
    pub(super) avoid: &'a [Arc<str>],
    pub(super) evidence: Option<&'a Arc<dyn EvidenceProvider>>,
}
impl<'a> RepairWalk<'a> {
    #[cfg(test)]
    pub(crate) fn from_request(request: &'a WalkRequest) -> Self {
        Self {
            options: request.options,
            protect: request.protect,
            food_guard: request.food_guard,
            allow: request.allow,
            cross: &request.cross,
            avoid: &request.avoid,
            evidence: request.evidence.as_ref(),
        }
    }
    pub(crate) fn from_outer(outer: &'a OuterWalk) -> Self {
        Self {
            options: outer.options,
            cross: &outer.cross,
            avoid: &outer.avoid,
            ..Self::default()
        }
    }
    fn request(self, target: WorldTile, radius: u16, observed: EvidenceStamp) -> WalkRequest {
        let mut request = reach::walk_request(target, radius, None, observed);
        request.options = self.options;
        request.protect = self.protect;
        request.food_guard = self.food_guard;
        request.allow = self.allow;
        request.cross = self.cross.into();
        request.avoid = self.avoid.into();
        request.evidence = self.evidence.cloned();
        request
    }
    fn request_goals(
        self,
        target: WorldTile,
        goals: WalkGoals,
        observed: EvidenceStamp,
    ) -> WalkRequest {
        let mut request = self.request(target, 0, observed);
        request.candidate_goals = Some(goals);
        request.arrival = nav_planner::contract::ArrivalKind::Area;
        request
    }
}

#[derive(Default)]
pub struct PrereqBudget {
    rows_run: u8,
    talks: u8,
    probes: u8,
    pub re_issues: u8,
    visited: [u16; 3],
    visited_len: u8,
}
impl PrereqBudget {
    pub const MAX_RE_ISSUES: u8 = 2;
    fn row(&mut self, row: u16) -> bool {
        if self.rows_run >= 3 || self.visited[..usize::from(self.visited_len)].contains(&row) {
            return false;
        }
        self.visited[usize::from(self.visited_len)] = row;
        self.visited_len += 1;
        self.rows_run += 1;
        true
    }
}

pub enum RunEntry {
    NeedsStep,
    GateRefused(Arc<RefusalObservation>),
}
pub struct Done {
    pub crossed: bool,
}

/// A read-only view for live proof receipts; no history is retained by the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JournalAttempt {
    /// Journal owners that began, including an owner that timed out.
    pub attempts_started: u8,
    pub owner_active: bool,
    /// The observed tick of the retryable timeout, if one occurred.
    pub retry_after: Option<u64>,
}

enum Phase {
    Resolve,
    Journal {
        handle: Option<ActionHandle<JournalMachine>>,
        retries_left: u8,
        retry_after: Option<u64>,
    },
    ProbeApproach(Option<ActionHandle<Walk>>),
    Probe {
        run: Box<InteractRun>,
        wait: Option<TickWait>,
    },
    ProbeDrain(ChatDrive),
    Preconditions(usize),
    Child {
        run: Box<PrereqRun>,
        next: usize,
    },
    Approach(Option<ActionHandle<Walk>>),
    TalkStart,
    Talk(ChatDrive),
    CloseStart(CloseResult),
    Close {
        handle: ActionHandle<ClosePage>,
        result: CloseResult,
    },
    Complete,
}
enum CloseResult {
    Done,
    Refusal(PrereqRefusal),
}

#[cfg(feature = "test-hooks")]
const JOURNAL_TRACE_ATTEMPTS: usize = JOURNAL_TIMEOUT_RETRIES as usize + 1;

#[cfg(feature = "test-hooks")]
#[derive(Default)]
struct JournalAttemptTrace {
    started_tick: u64,
    terminal_tick: Option<u64>,
    failure: Option<Arc<str>>,
}

pub struct PrereqRun {
    gate: ActionableGate,
    entry: RunEntry,
    allow: PrereqAllow,
    catalog: Option<Arc<PrereqCatalog>>,
    phase: Phase,
    depth: u8,
    repeats_left: Option<u8>,
    required_after: Option<EvidenceStamp>,
    explicit: Option<Arc<dyn EvidenceProvider>>,
    #[cfg(feature = "test-hooks")]
    journal_attempts: [Option<JournalAttemptTrace>; JOURNAL_TRACE_ATTEMPTS],
    #[cfg(feature = "test-hooks")]
    journal_attempts_len: usize,
}
impl PrereqRun {
    pub fn begin(gate: ActionableGate, entry: RunEntry, allow: PrereqAllow) -> Self {
        Self {
            gate,
            entry,
            allow,
            catalog: None,
            phase: Phase::Resolve,
            depth: 0,
            repeats_left: None,
            required_after: None,
            explicit: None,
            #[cfg(feature = "test-hooks")]
            journal_attempts: std::array::from_fn(|_| None),
            #[cfg(feature = "test-hooks")]
            journal_attempts_len: 0,
        }
    }
    pub fn with_evidence(mut self, evidence: Option<Arc<dyn EvidenceProvider>>) -> Self {
        self.explicit = evidence;
        self
    }

    /// Return the bounded journal-attempt trace used by test receipts.
    #[cfg(feature = "test-hooks")]
    pub fn journal_attempts(
        &self,
    ) -> impl Iterator<Item = (u8, u64, Option<u64>, Option<&str>)> + '_ {
        self.journal_attempts
            .iter()
            .take(self.journal_attempts_len)
            .enumerate()
            .filter_map(|(index, attempt)| {
                attempt.as_ref().map(|attempt| {
                    (
                        index as u8 + 1,
                        attempt.started_tick,
                        attempt.terminal_tick,
                        attempt.failure.as_deref(),
                    )
                })
            })
    }
    pub fn first_supported(
        gates: &[ActionableGate],
        entry: &RunEntry,
        cx: &ActionContext<'_>,
    ) -> Option<ActionableGate> {
        let catalog = catalog(cx)?;
        match entry {
            RunEntry::NeedsStep => gates
                .iter()
                .find(|gate| {
                    catalog.for_journal(&gate.gate).is_some()
                        || catalog.for_gate(&gate.gate, &gate.crossing).is_some()
                })
                .cloned(),
            RunEntry::GateRefused(observation) => {
                let (_, crossing) =
                    catalog.reference(observation.matched.row, observation.matched.entry)?;
                if crossing.crossing != observation.crossing {
                    return None;
                }
                gates
                    .iter()
                    .find(|gate| {
                        gate.leg == observation.leg
                            && gate.crossing == observation.crossing
                            && catalog
                                .for_gate(&gate.gate, &gate.crossing)
                                .is_some_and(|(row, _)| row == observation.matched.row)
                    })
                    .cloned()
            }
        }
    }
    pub fn identity(
        gate: &ActionableGate,
        cx: &ActionContext<'_>,
    ) -> Result<PrereqStepIdentity, ActionError> {
        let catalog = catalog(cx).ok_or_else(|| {
            ActionError::Unavailable("prerequisite facts are not prepared".into())
        })?;
        if let Some((_, journal)) = catalog.for_journal(&gate.gate) {
            return Ok(journal_identity(journal));
        }
        let (_, row) = catalog
            .for_gate(&gate.gate, &gate.crossing)
            .ok_or_else(|| ActionError::Unavailable("no matching prerequisite row".into()))?;
        Ok(step_identity(row, &gate.crossing))
    }
    pub fn walk_handle(&self) -> Option<&ActionHandle<Walk>> {
        match &self.phase {
            Phase::ProbeApproach(handle) | Phase::Approach(handle) => handle.as_ref(),
            Phase::Child { run, .. } => run.walk_handle(),
            _ => None,
        }
    }
    /// Observe this run's journal attempt without changing its owner or budget.
    pub fn journal_attempt(&self) -> Option<JournalAttempt> {
        let Phase::Journal {
            handle,
            retries_left,
            retry_after,
        } = &self.phase
        else {
            return None;
        };
        Some(JournalAttempt {
            attempts_started: JOURNAL_TIMEOUT_RETRIES - retries_left + u8::from(handle.is_some()),
            owner_active: handle
                .as_ref()
                .is_some_and(|handle| handle.inspect(|_| ()).is_some()),
            retry_after: *retry_after,
        })
    }
    fn step(&self, row: &PrereqStep) -> PrereqStepIdentity {
        step_identity(row, &self.gate.crossing)
    }

    pub(crate) fn revoker(&self) -> Option<crate::native::ActionRevoker> {
        match &self.phase {
            Phase::ProbeApproach(handle) | Phase::Approach(handle) => {
                handle.as_ref().map(ActionHandle::revoker)
            }
            Phase::Probe { run, .. } => run.prereq_revoker(),
            Phase::ProbeDrain(drive) | Phase::Talk(drive) => {
                drive.handle.as_ref().map(ActionHandle::revoker)
            }
            Phase::Child { run, .. } => run.revoker(),
            Phase::Close { handle, .. } => Some(handle.revoker()),
            Phase::Journal { handle, .. } => handle.as_ref().map(ActionHandle::revoker),
            _ => None,
        }
    }

    pub fn poll(
        &mut self,
        actions: &mut NativeActions,
        cx: &mut ActionContext<'_>,
        state: Option<&SlotState>,
        budget: &mut PrereqBudget,
    ) -> Poll<Result<Done, PrereqRefusal>> {
        self.poll_with_walk(actions, cx, state, budget, RepairWalk::default())
    }

    pub(crate) fn poll_with_walk(
        &mut self,
        actions: &mut NativeActions,
        cx: &mut ActionContext<'_>,
        state: Option<&SlotState>,
        budget: &mut PrereqBudget,
        repair: RepairWalk<'_>,
    ) -> Poll<Result<Done, PrereqRefusal>> {
        if !cx.eligible() {
            return Poll::Pending;
        }
        if self.catalog.is_none() {
            self.catalog = catalog(cx);
        }
        let Some(catalog) = self.catalog.as_ref() else {
            return Poll::Ready(Err(PrereqRefusal::NoRow {
                gate: self.gate.gate.clone(),
            }));
        };
        if let Some((ordinal, journal)) = catalog.for_journal(&self.gate.gate) {
            let step = journal_identity(journal);
            let floor = *self.required_after.get_or_insert(cx.evidence());
            if matches!(self.phase, Phase::Resolve) {
                if !budget.row(ordinal) {
                    return Poll::Ready(Err(PrereqRefusal::StepRepeated {
                        gate: self.gate.gate.clone(),
                    }));
                }
                self.phase = Phase::Journal {
                    handle: None,
                    retries_left: JOURNAL_TIMEOUT_RETRIES,
                    retry_after: None,
                };
            }
            let Phase::Journal {
                handle,
                retries_left,
                retry_after,
            } = &mut self.phase
            else {
                return Poll::Ready(Err(PrereqRefusal::StepUnwitnessed { step }));
            };
            if retry_after.is_some_and(|tick| cx.evidence().tick <= tick) {
                return Poll::Pending;
            }
            if handle.is_none() {
                if !cx.may_begin_this_tick() {
                    return Poll::Pending;
                }
                let data = api::game_data::for_revision(cx.pin().revision)
                    .expect("catalog revision was validated");
                let Some(facts) = data.try_quests() else {
                    return Poll::Ready(Err(failed(
                        step,
                        PrereqFailure::Action(Arc::from("journal facts are not prepared")),
                    )));
                };
                match actions.begin::<JournalMachine>(
                    JournalRequest {
                        quest: journal.quest.clone(),
                        facts,
                    },
                    cx,
                ) {
                    Ok(read) => {
                        #[cfg(feature = "test-hooks")]
                        journal_attempt_started(
                            &mut self.journal_attempts,
                            &mut self.journal_attempts_len,
                            cx.evidence().tick,
                        );
                        *handle = Some(read);
                    }
                    Err(ActionError::BudgetExhausted) => return Poll::Pending,
                    Err(error) => {
                        #[cfg(feature = "test-hooks")]
                        {
                            journal_attempt_started(
                                &mut self.journal_attempts,
                                &mut self.journal_attempts_len,
                                cx.evidence().tick,
                            );
                            journal_attempt_finished(
                                &mut self.journal_attempts,
                                self.journal_attempts_len,
                                cx.evidence().tick,
                                Some(Arc::<str>::from(error.to_string())),
                            );
                        }
                        return Poll::Ready(Err(failed(step, action_failure(error))));
                    }
                }
            }
            let read = match actions.poll(handle.as_ref().expect("journal owner"), cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(ActionError::Failed(reason)))
                    if reason.as_ref() == "journal modal timeout" && *retries_left > 0 =>
                {
                    #[cfg(feature = "test-hooks")]
                    journal_attempt_finished(
                        &mut self.journal_attempts,
                        self.journal_attempts_len,
                        cx.evidence().tick,
                        Some(Arc::<str>::from(
                            ActionError::Failed(Arc::clone(&reason)).to_string(),
                        )),
                    );
                    *retries_left -= 1;
                    *handle = None;
                    *retry_after = Some(cx.evidence().tick);
                    return Poll::Pending;
                }
                Poll::Ready(Err(error)) => {
                    #[cfg(feature = "test-hooks")]
                    journal_attempt_finished(
                        &mut self.journal_attempts,
                        self.journal_attempts_len,
                        cx.evidence().tick,
                        Some(Arc::<str>::from(error.to_string())),
                    );
                    self.phase = Phase::Complete;
                    return Poll::Ready(Err(failed(step, action_failure(error))));
                }
                Poll::Ready(Ok(read)) => {
                    #[cfg(feature = "test-hooks")]
                    journal_attempt_finished(
                        &mut self.journal_attempts,
                        self.journal_attempts_len,
                        cx.evidence().tick,
                        None,
                    );
                    read
                }
            };
            self.phase = Phase::Complete;
            if read.quest != journal.quest
                || read.pin.as_ref() != cx.pin()
                || !read.acquired.meets(floor)
                || !read.closed.meets(read.acquired)
                || !cx.evidence().meets(read.closed)
            {
                return Poll::Ready(Err(PrereqRefusal::StepUnwitnessed { step }));
            }
            let text = super::super::progress::normalize_journal(&read.lines);
            let mut matched =
                journal.rules.iter().enumerate().filter(|(_, rule)| {
                    rule.all.iter().all(|needle| text.contains(needle.as_ref()))
                });
            let Some((line, rule)) = matched.next() else {
                return Poll::Ready(Err(PrereqRefusal::StepUnwitnessed { step }));
            };
            if matched.next().is_some() {
                return Poll::Ready(Err(PrereqRefusal::StepUnwitnessed { step }));
            }
            let after = journal
                .after
                .iter()
                .find(|after| after.proves == rule.proves)
                .expect("journal preservation was validated")
                .after;
            if let Some(state) = state {
                state.record_prereq_fact(Fact {
                    quest: journal.establishes.quest.clone(),
                    signal: journal.establishes.signal.clone(),
                    possible: rule.proves,
                    after,
                    refused: None,
                    source: FactSource::Page {
                        row: ordinal,
                        line: u16::try_from(line).expect("journal rule count was checked"),
                    },
                    observed: read.acquired,
                });
            }
            let QuestGate::Window(window) = &self.gate.gate else {
                return Poll::Ready(Err(PrereqRefusal::StepUnwitnessed { step }));
            };
            return Poll::Ready(if window.values.test(&rule.proves) == Truth::True {
                Ok(Done { crossed: false })
            } else {
                Err(PrereqRefusal::PreconditionMissing {
                    step,
                    missing: PrereqMissing::Quest(self.gate.gate.clone()),
                })
            });
        }
        let Some((ordinal, row)) = catalog.for_gate(&self.gate.gate, &self.gate.crossing) else {
            return Poll::Ready(Err(PrereqRefusal::NoRow {
                gate: self.gate.gate.clone(),
            }));
        };
        self.required_after.get_or_insert(cx.evidence());
        loop {
            let phase = std::mem::replace(&mut self.phase, Phase::Complete);
            match phase {
                Phase::Resolve => {
                    // No handle, precondition or probe exists before consent.
                    let switch = match row.consent {
                        ConsentSwitch::PrereqDialogue => PrereqSwitch::Dialogue,
                        ConsentSwitch::QuestStart => PrereqSwitch::QuestStart,
                    };
                    if !(match switch {
                        PrereqSwitch::Dialogue => self.allow.dialogue,
                        PrereqSwitch::QuestStart => self.allow.quest_start,
                    }) {
                        return Poll::Ready(Err(PrereqRefusal::NotAllowed {
                            step: self.step(row),
                            switch,
                        }));
                    }
                    if !budget.row(ordinal) {
                        return Poll::Ready(Err(PrereqRefusal::StepRepeated {
                            gate: self.gate.gate.clone(),
                        }));
                    }
                    self.repeats_left = Some(
                        row.talk
                            .repeat_after
                            .as_ref()
                            .map_or(0, |repeat| repeat.reclaimed),
                    );
                    self.phase = if matches!(self.entry, RunEntry::GateRefused(_))
                        || self.gate.truth == Truth::False
                    {
                        Phase::Preconditions(0)
                    } else {
                        Phase::ProbeApproach(None)
                    };
                }
                Phase::ProbeApproach(mut handle) => {
                    if budget.probes >= 2 {
                        return Poll::Ready(Err(failed(
                            self.step(row),
                            PrereqFailure::ProbeUnacknowledged,
                        )));
                    }
                    let alternatives = self.depth > 0
                        && row
                            .crossings
                            .iter()
                            .any(|entry| entry.crossing.takeoff != self.gate.crossing.takeoff);
                    let here = cx.snapshot().here().map(|here| here.value);
                    // Settle an in-flight candidate walk before binding its receipt goal.
                    let crossing = if alternatives && handle.is_none() {
                        here.and_then(|here| {
                            row.crossings
                                .iter()
                                .find(|entry| entry.crossing.takeoff == here)
                        })
                    } else if alternatives {
                        None
                    } else {
                        here.filter(|here| *here == self.gate.crossing.takeoff)
                            .and_then(|_| {
                                row.crossing(&self.gate.crossing)
                                    .map(|(_, crossing)| crossing)
                            })
                    };
                    if let Some(crossing) = crossing {
                        drop(handle);
                        self.gate.crossing = crossing.crossing;
                        let Some((tile, op)) = probe_target(cx, &self.gate.crossing) else {
                            return Poll::Ready(Err(failed(
                                self.step(row),
                                PrereqFailure::Action(
                                    "the crossing loc or its authored option was not observed"
                                        .into(),
                                ),
                            )));
                        };
                        budget.probes += 1;
                        self.phase = Phase::Probe {
                            run: Box::new(InteractRun::prereq_probe(&self.gate.crossing, tile, op)),
                            wait: None,
                        };
                        continue;
                    }
                    if handle.is_none() {
                        if !cx.may_begin_this_tick() {
                            self.phase = Phase::ProbeApproach(handle);
                            return Poll::Pending;
                        }
                        let request = if alternatives {
                            let mut goals = Vec::with_capacity(row.crossings.len());
                            goals.extend(
                                row.crossings
                                    .iter()
                                    .map(|crossing| crossing.crossing.takeoff),
                            );
                            let goals = match WalkGoals::new(goals) {
                                Ok(goals) => goals,
                                Err(error) => {
                                    return Poll::Ready(Err(failed(
                                        self.step(row),
                                        action_failure(ActionError::WalkGoals(error)),
                                    )))
                                }
                            };
                            repair.request_goals(self.gate.crossing.takeoff, goals, cx.evidence())
                        } else {
                            repair.request(self.gate.crossing.takeoff, 0, cx.evidence())
                        };
                        match actions.begin::<Walk>(request, cx) {
                            Ok(walk) => handle = Some(walk),
                            Err(ActionError::BudgetExhausted) => {
                                self.phase = Phase::ProbeApproach(handle);
                                return Poll::Pending;
                            }
                            Err(error) => {
                                return Poll::Ready(Err(failed(
                                    self.step(row),
                                    action_failure(error),
                                )))
                            }
                        }
                    }
                    match actions.poll(handle.as_ref().expect("probe approach owner"), cx) {
                        Poll::Pending => {
                            self.phase = Phase::ProbeApproach(handle);
                            return Poll::Pending;
                        }
                        Poll::Ready(Ok(receipt)) if receipt.end == WalkEnd::Arrived => {
                            if alternatives {
                                let Some(goal) = receipt.goal else {
                                    return Poll::Ready(Err(failed(
                                        self.step(row),
                                        PrereqFailure::Action(
                                            "candidate walk arrived without reporting its chosen goal"
                                                .into(),
                                        ),
                                    )));
                                };
                                let Some(crossing) = row
                                    .crossings
                                    .iter()
                                    .find(|entry| entry.crossing.takeoff == goal)
                                else {
                                    return Poll::Ready(Err(failed(
                                        self.step(row),
                                        PrereqFailure::Action(
                                            "candidate walk selected a tile outside the prerequisite row"
                                                .into(),
                                        ),
                                    )));
                                };
                                self.gate.crossing = crossing.crossing;
                            }
                            self.phase = Phase::ProbeApproach(None)
                        }
                        Poll::Ready(Ok(receipt)) => {
                            return Poll::Ready(Err(PrereqRefusal::NoRouteToStep {
                                step: self.step(row),
                                end: receipt.end,
                            }))
                        }
                        Poll::Ready(Err(error)) => {
                            return Poll::Ready(Err(failed(self.step(row), action_failure(error))))
                        }
                    }
                }
                Phase::Probe { mut run, mut wait } => {
                    let (_, crossing) = row
                        .crossing(&self.gate.crossing)
                        .expect("resolved crossing");
                    if wait.is_none() {
                        match run.poll_prereq_click(actions, cx) {
                            Poll::Pending => {
                                self.phase = Phase::Probe { run, wait };
                                return Poll::Pending;
                            }
                            Poll::Ready(Err(error)) => {
                                return Poll::Ready(Err(failed(
                                    self.step(row),
                                    action_failure(error),
                                )))
                            }
                            Poll::Ready(Ok(())) => {
                                wait = Some(
                                    TickWait::new(
                                        cx,
                                        OP_TICKS
                                            .saturating_add(crossing.delay.ticks)
                                            .saturating_add(SEND_SLACK_TICKS),
                                    )
                                    .started(PrereqWaitName::W2),
                                );
                            }
                        }
                    }
                    let mut current = wait.expect("probe send has a wait");
                    let Some((_, receipt)) = run.prereq_send(cx) else {
                        return Poll::Ready(Err(failed(
                            self.step(row),
                            PrereqFailure::ProbeUnacknowledged,
                        )));
                    };
                    let accepted = if let Some(receipt) = receipt {
                        if !receipt.accepted {
                            return Poll::Ready(Err(failed(
                                self.step(row),
                                PrereqFailure::ProbeUnacknowledged,
                            )));
                        }
                        current.since = receipt.evidence.tick;
                        current.chat_mark = receipt.chat_since;
                        cx.evidence().tick > receipt.evidence.tick
                    } else {
                        false
                    };
                    if accepted {
                        let message =
                            |text: &str| reach::saw_game_message(cx, current.chat_mark, text);
                        if message(&crossing.pass.message)
                            && cx
                                .snapshot()
                                .here()
                                .is_some_and(|here| here.value == crossing.pass.landing)
                        {
                            record(
                                state,
                                row,
                                crossing.pass.proves,
                                None,
                                FactSource::Crossing {
                                    loc_id: self.gate.crossing.loc_id,
                                    option: self.gate.crossing.option,
                                    passed: true,
                                },
                                cx.evidence(),
                            );
                            wait_log(PrereqWaitName::W2, current, cx.evidence().tick, "pass");
                            return Poll::Ready(Ok(Done { crossed: true }));
                        }
                        if let Some(other) = row.crossings.iter().find(|other| {
                            other.pass.message != crossing.pass.message
                                && message(&other.pass.message)
                        }) {
                            record(
                                state,
                                row,
                                other.pass.proves,
                                None,
                                FactSource::Crossing {
                                    loc_id: self.gate.crossing.loc_id,
                                    option: self.gate.crossing.option,
                                    passed: true,
                                },
                                cx.evidence(),
                            );
                            return Poll::Ready(Err(failed(
                                self.step(row),
                                PrereqFailure::ProbeDirection {
                                    expected: Arc::clone(&crossing.pass.message),
                                    seen: Arc::clone(&other.pass.message),
                                },
                            )));
                        }
                        if reach::saw_cant_reach(cx, current.chat_mark) {
                            return Poll::Ready(Err(failed(
                                self.step(row),
                                PrereqFailure::ProbeUnacknowledged,
                            )));
                        }
                        if current.changed(cx) && chat_open(cx) {
                            wait_log(PrereqWaitName::W2, current, cx.evidence().tick, "page");
                            drop(run);
                            match ChatDrive::begin(actions, cx, row, true) {
                                Ok(drive) => self.phase = Phase::ProbeDrain(drive),
                                Err(ActionError::BudgetExhausted) => {
                                    // The op was accepted; never click it again on a transition limit.
                                    self.phase = Phase::ProbeDrain(ChatDrive::unstarted(true));
                                    return Poll::Pending;
                                }
                                Err(error) => {
                                    return Poll::Ready(Err(failed(
                                        self.step(row),
                                        action_failure(error),
                                    )))
                                }
                            }
                            continue;
                        }
                    }
                    if current.spent(cx.evidence().tick) {
                        wait_log(
                            PrereqWaitName::W2,
                            current,
                            cx.evidence().tick,
                            "unacknowledged",
                        );
                        return Poll::Ready(Err(failed(
                            self.step(row),
                            PrereqFailure::ProbeUnacknowledged,
                        )));
                    }
                    // Movement alone is not pass evidence; retain its initial tile watermark.
                    self.phase = Phase::Probe {
                        run,
                        wait: Some(current),
                    };
                    return Poll::Pending;
                }
                Phase::ProbeDrain(mut drive) => {
                    match drive.poll(actions, cx, state, row, ordinal, &self.gate.gate) {
                        Poll::Pending => {
                            self.phase = Phase::ProbeDrain(drive);
                            return Poll::Pending;
                        }
                        Poll::Ready(Err(why)) => {
                            return Poll::Ready(Err(failed(self.step(row), why)))
                        }
                        Poll::Ready(Ok(ChatDone::Drained {
                            refusal: Some(index),
                            ..
                        })) => match &row.probe.refuse[index].then {
                            RefusalThen::Talk => self.phase = Phase::Preconditions(0),
                            RefusalThen::Precondition(gate) => {
                                return Poll::Ready(Err(PrereqRefusal::PreconditionMissing {
                                    step: self.step(row),
                                    missing: PrereqMissing::Quest(gate.clone()),
                                }))
                            }
                        },
                        Poll::Ready(Ok(_)) => {
                            return Poll::Ready(Err(PrereqRefusal::StepUnwitnessed {
                                step: self.step(row),
                            }))
                        }
                    }
                }
                Phase::Preconditions(index) => {
                    let Some(precondition) = row.preconditions.get(index) else {
                        self.phase = Phase::Approach(None);
                        continue;
                    };
                    match precondition {
                        PrereqPrecondition::Gate { gate, .. } => {
                            let truth = self.truth(gate, cx, state);
                            let child = catalog
                                .iter()
                                .find(|(_, row)| api::prereq_steps::proves(&row.establishes, gate));
                            if truth != Truth::True && self.depth < 2 {
                                if let Some((_, child_row)) = child {
                                    // This identity locates the child row; its approach
                                    // selects among all authored takeoffs in ProbeApproach.
                                    if let Some(crossing) = child_row.crossings.first() {
                                        let mut run = Self::begin(
                                            ActionableGate {
                                                gate: gate.clone(),
                                                truth,
                                                leg: self.gate.leg,
                                                crossing: crossing.crossing,
                                            },
                                            RunEntry::NeedsStep,
                                            self.allow,
                                        )
                                        .with_evidence(self.explicit.clone());
                                        run.catalog = Some(Arc::clone(catalog));
                                        run.depth = self.depth + 1;
                                        self.phase = Phase::Child {
                                            run: Box::new(run),
                                            next: index + 1,
                                        };
                                        continue;
                                    }
                                }
                            }
                            if truth == Truth::False || (child.is_some() && truth != Truth::True) {
                                return Poll::Ready(Err(PrereqRefusal::PreconditionMissing {
                                    step: self.step(row),
                                    missing: PrereqMissing::Quest(gate.clone()),
                                }));
                            }
                        }
                        // The server chooses the reclaim branch using inventory, worn and bank.
                        // Unknown bank ownership cannot refuse an otherwise safe conversation.
                        PrereqPrecondition::OwnedAnywhere { .. } => {}
                    }
                    self.phase = Phase::Preconditions(index + 1);
                }
                Phase::Child { mut run, next } => {
                    match run.poll_with_walk(actions, cx, state, budget, repair) {
                        Poll::Pending => {
                            self.phase = Phase::Child { run, next };
                            return Poll::Pending;
                        }
                        Poll::Ready(Err(refusal)) => return Poll::Ready(Err(refusal)),
                        Poll::Ready(Ok(_)) => self.phase = Phase::Preconditions(next),
                    }
                }
                Phase::Approach(mut handle) => {
                    if handle.is_none() {
                        if !cx.may_begin_this_tick() {
                            self.phase = Phase::Approach(handle);
                            return Poll::Pending;
                        }
                        match actions.begin::<Walk>(
                            repair.request(row.target.anchor, row.target.leash, cx.evidence()),
                            cx,
                        ) {
                            Ok(walk) => handle = Some(walk),
                            Err(ActionError::BudgetExhausted) => {
                                self.phase = Phase::Approach(handle);
                                return Poll::Pending;
                            }
                            Err(error) => {
                                return Poll::Ready(Err(failed(
                                    self.step(row),
                                    action_failure(error),
                                )))
                            }
                        }
                    }
                    match actions.poll(handle.as_ref().expect("step approach owner"), cx) {
                        Poll::Pending => {
                            self.phase = Phase::Approach(handle);
                            return Poll::Pending;
                        }
                        Poll::Ready(Ok(receipt)) if receipt.end == WalkEnd::Arrived => {
                            self.phase = Phase::TalkStart
                        }
                        Poll::Ready(Ok(receipt)) => {
                            return Poll::Ready(Err(PrereqRefusal::NoRouteToStep {
                                step: self.step(row),
                                end: receipt.end,
                            }))
                        }
                        Poll::Ready(Err(error)) => {
                            return Poll::Ready(Err(failed(self.step(row), action_failure(error))))
                        }
                    }
                }
                Phase::TalkStart => {
                    if budget.talks >= 4 {
                        return Poll::Ready(Err(failed(
                            self.step(row),
                            PrereqFailure::ReclaimLoop,
                        )));
                    }
                    match ChatDrive::begin(actions, cx, row, false) {
                        Ok(drive) => {
                            budget.talks += 1;
                            self.phase = Phase::Talk(drive);
                        }
                        Err(ActionError::BudgetExhausted) => {
                            self.phase = Phase::TalkStart;
                            return Poll::Pending;
                        }
                        Err(error) => {
                            return Poll::Ready(Err(failed(self.step(row), action_failure(error))))
                        }
                    }
                }
                Phase::Talk(mut drive) => {
                    match drive.poll(actions, cx, state, row, ordinal, &self.gate.gate) {
                        Poll::Pending => {
                            self.phase = Phase::Talk(drive);
                            return Poll::Pending;
                        }
                        Poll::Ready(Err(why)) => {
                            return Poll::Ready(Err(failed(self.step(row), why)))
                        }
                        Poll::Ready(Ok(ChatDone::Close { missing, witnessed })) => {
                            drop(drive);
                            let result = if let Some(required) = missing {
                                CloseResult::Refusal(PrereqRefusal::PreconditionMissing {
                                    step: self.step(row),
                                    missing: PrereqMissing::FreeSlots { required },
                                })
                            } else if witnessed {
                                CloseResult::Done
                            } else {
                                CloseResult::Refusal(PrereqRefusal::StepUnwitnessed {
                                    step: self.step(row),
                                })
                            };
                            self.phase = Phase::CloseStart(result);
                        }
                        Poll::Ready(Ok(ChatDone::Drained {
                            branch: Some(index),
                            ..
                        })) => {
                            if row.talk.branches[index].outcome == BranchOutcome::Reclaimed {
                                let left =
                                    self.repeats_left.as_mut().expect("resolved repeat bound");
                                if *left == 0 {
                                    return Poll::Ready(Err(failed(
                                        self.step(row),
                                        PrereqFailure::ReclaimLoop,
                                    )));
                                }
                                *left -= 1;
                                self.phase = Phase::TalkStart;
                            } else {
                                return Poll::Ready(Ok(Done { crossed: false }));
                            }
                        }
                        Poll::Ready(Ok(_)) => {
                            return Poll::Ready(Err(PrereqRefusal::StepUnwitnessed {
                                step: self.step(row),
                            }))
                        }
                    }
                }
                Phase::CloseStart(result) => match actions.begin::<ClosePage>((), cx) {
                    Ok(handle) => self.phase = Phase::Close { handle, result },
                    Err(ActionError::BudgetExhausted) => {
                        self.phase = Phase::CloseStart(result);
                        return Poll::Pending;
                    }
                    Err(error) => {
                        return Poll::Ready(Err(failed(self.step(row), action_failure(error))))
                    }
                },
                Phase::Close { handle, result } => match actions.poll(&handle, cx) {
                    Poll::Pending => {
                        self.phase = Phase::Close { handle, result };
                        return Poll::Pending;
                    }
                    Poll::Ready(Ok(Ok(()))) => {
                        return Poll::Ready(match result {
                            CloseResult::Done => Ok(Done { crossed: false }),
                            CloseResult::Refusal(refusal) => Err(refusal),
                        })
                    }
                    Poll::Ready(Ok(Err(why))) => {
                        return Poll::Ready(Err(failed(self.step(row), why)))
                    }
                    Poll::Ready(Err(error)) => {
                        return Poll::Ready(Err(failed(self.step(row), action_failure(error))))
                    }
                },
                Phase::Journal { .. } => {
                    return Poll::Ready(Err(PrereqRefusal::StepUnwitnessed {
                        step: self.step(row),
                    }));
                }
                Phase::Complete => {
                    return Poll::Ready(Err(failed(
                        self.step(row),
                        PrereqFailure::Action("prerequisite was already completed".into()),
                    )))
                }
            }
        }
    }

    fn truth(&self, gate: &QuestGate, cx: &ActionContext<'_>, state: Option<&SlotState>) -> Truth {
        let floor = self.required_after.unwrap_or_else(|| cx.evidence());
        let ordinary = ObservationProvider(frame_observation(gate, cx, floor));
        let compose = |memory: &SessionMemory| {
            api::prereq_knowledge::compose(gate, floor, memory, self.explicit.as_deref(), &ordinary)
                .truth
        };
        state.map_or_else(
            || compose(&SessionMemory::default()),
            |state| state.with_prereq_memory(compose),
        )
    }
}

#[cfg(feature = "test-hooks")]
fn journal_attempt_started(
    attempts: &mut [Option<JournalAttemptTrace>; JOURNAL_TRACE_ATTEMPTS],
    attempts_len: &mut usize,
    tick: u64,
) {
    let index = *attempts_len;
    assert!(
        index < attempts.len(),
        "journal retry bound exceeds trace capacity"
    );
    attempts[index] = Some(JournalAttemptTrace {
        started_tick: tick,
        ..JournalAttemptTrace::default()
    });
    *attempts_len += 1;
}

#[cfg(feature = "test-hooks")]
fn journal_attempt_finished(
    attempts: &mut [Option<JournalAttemptTrace>; JOURNAL_TRACE_ATTEMPTS],
    attempts_len: usize,
    tick: u64,
    failure: Option<Arc<str>>,
) {
    let attempt = attempts
        .get_mut(
            attempts_len
                .checked_sub(1)
                .expect("journal attempt started"),
        )
        .and_then(Option::as_mut)
        .expect("journal attempt trace exists");
    attempt.terminal_tick = Some(tick);
    attempt.failure = failure;
}

fn catalog(cx: &ActionContext<'_>) -> Option<Arc<PrereqCatalog>> {
    let data = api::game_data::for_revision(cx.pin().revision).ok()?;
    if data.selected_pin().ok()?.as_ref() != cx.pin() {
        return None;
    }
    data.try_prereq_steps()
}
fn step_identity(row: &PrereqStep, crossing: &api::prereq_steps::CrossingId) -> PrereqStepIdentity {
    PrereqStepIdentity {
        id: Arc::clone(&row.id),
        source: Arc::clone(&row.crossing(crossing).expect("matched crossing").1.source.0),
    }
}
fn journal_identity(journal: &PrereqJournal) -> PrereqStepIdentity {
    PrereqStepIdentity {
        id: Arc::clone(&journal.id),
        source: Arc::clone(&journal.source.0),
    }
}
fn failed(step: PrereqStepIdentity, why: PrereqFailure) -> PrereqRefusal {
    PrereqRefusal::StepFailed { step, why }
}
fn action_failure(error: ActionError) -> PrereqFailure {
    match error {
        ActionError::CombatInterrupted => PrereqFailure::CombatInterrupted,
        error => PrereqFailure::Action(Arc::from(error.to_string())),
    }
}
fn record(
    state: Option<&SlotState>,
    row: &PrereqStep,
    possible: InclusiveRange,
    refused: Option<InclusiveRange>,
    source: FactSource,
    observed: EvidenceStamp,
) {
    let Some(after) = row
        .after
        .iter()
        .find(|rule| rule.proves == possible)
        .map(|rule| rule.after)
    else {
        return;
    };
    if let Some(state) = state {
        if state.record_prereq_fact(Fact {
            quest: row.establishes.quest.clone(),
            signal: row.establishes.signal.clone(),
            possible,
            after,
            refused,
            source,
            observed,
        }) {
            api::host_log!(
                api::hostlog::Category::NavEvent,
                api::hostlog::Level::Info,
                "prereq fact row={} possible={possible:?} after={after:?} observed={observed:?}",
                row.id
            );
        }
    }
}
struct ObservationProvider(Option<Observation>);
impl EvidenceProvider for ObservationProvider {
    fn observe_gate(&self, _: &QuestGate, _: EvidenceStamp) -> Option<Observation> {
        self.0
    }
}
fn frame_observation(
    gate: &QuestGate,
    cx: &ActionContext<'_>,
    floor: EvidenceStamp,
) -> Option<Observation> {
    let data = api::game_data::for_revision(cx.pin().revision).ok()?;
    if data.selected_pin().ok()?.as_ref() != cx.pin() {
        return None;
    }
    api::quest_progress::observe_quest_gate(data.try_quests()?.as_ref(), gate, cx.snapshot(), floor)
}
fn page(cx: &ActionContext<'_>) -> PageAcknowledgement {
    cx.snapshot()
        .chat_modal()
        .map_or_else(PageAcknowledgement::default, |chat| {
            PageAcknowledgement::capture(
                chat.value.root,
                chat.value.continue_component_id >= 0,
                api::snapshot::chat_page_fingerprint(
                    chat.value.texts,
                    chat.value
                        .options
                        .iter()
                        .map(|option| (option.component_id, option.text.as_str())),
                ),
            )
        })
}
fn chat_open(cx: &ActionContext<'_>) -> bool {
    cx.snapshot()
        .chat_modal()
        .is_some_and(|chat| chat.value.root != -1 || chat.value.continue_component_id >= 0)
}
fn contains(cx: &ActionContext<'_>, text: &str) -> bool {
    cx.snapshot()
        .chat_modal()
        .is_some_and(|chat| chat_page_matches(chat.value.texts, text))
}

fn probe_target(
    cx: &ActionContext<'_>,
    crossing: &api::prereq_steps::CrossingId,
) -> Option<(WorldTile, Arc<str>)> {
    let option = usize::from(crossing.option.checked_sub(1)?);
    let locs = cx.snapshot().locs()?;
    locs.value
        .iter()
        .find(|loc| loc.id == crossing.loc_id && loc.tile == crossing.at)
        .and_then(|loc| Some((loc.tile, Arc::from(loc.actions.get(option)?.as_deref()?))))
}

fn talk_allowance_ticks(row: &PrereqStep) -> u16 {
    row.target
        .leash
        .saturating_add(NPC_STEP_TICKS)
        .saturating_add(OP_TICKS)
        .saturating_add(SEND_SLACK_TICKS)
}

struct PendingPage {
    send: DialogueSend,
    wait: TickWait,
    name: PrereqWaitName,
    effect: bool,
    terminal: bool,
    effect_written: bool,
}
impl PendingPage {
    fn new(
        send: DialogueSend,
        cx: &ActionContext<'_>,
        row: &PrereqStep,
        effect: bool,
        terminal: bool,
    ) -> Self {
        let name = if send.kind == DialogueSendKind::Talk {
            PrereqWaitName::W5
        } else if terminal {
            PrereqWaitName::W4
        } else {
            PrereqWaitName::W3
        };
        let bound = if send.kind == DialogueSendKind::Talk {
            talk_allowance_ticks(row)
        } else {
            PAGE_TICKS + SEND_SLACK_TICKS
        };
        if send.kind == DialogueSendKind::Talk {
            talk_log(cx, row, "send", send.evidence.tick);
        }
        Self {
            send,
            wait: TickWait {
                since: send.evidence.tick,
                page: send.page,
                ..TickWait::new(cx, bound)
            }
            .started(name),
            name,
            effect,
            terminal,
            effect_written: false,
        }
    }
}

/// The nearest observed NPC of this type: its rendered tile and distance from
/// the player, and its server-side (path-head) tile.
fn nearest_npc(cx: &ActionContext<'_>, id: i32) -> Option<(WorldTile, i32, WorldTile)> {
    let wanted = usize::try_from(id).ok()?;
    cx.snapshot()
        .npcs()?
        .value
        .iter()
        .filter(|npc| npc.r#type == Some(wanted))
        .min_by_key(|npc| npc.distance)
        .map(|npc| (npc.tile, npc.distance, npc.network))
}

fn talk_log(cx: &ActionContext<'_>, row: &PrereqStep, what: &str, tick: u64) {
    api::host_log!(
        api::hostlog::Category::NavEvent,
        api::hostlog::Level::Info,
        "prereq talk {what} npc_type={} tick={tick} from={:?} from_network={:?} npc(tile,distance,network)={:?}",
        row.target.npc,
        cx.snapshot().here().map(|here| here.value),
        cx.snapshot()
            .local_player()
            .map(|player| player.value.player.network),
        nearest_npc(cx, row.target.npc)
    );
}
struct ChatDrive {
    handle: Option<ActionHandle<Dialogue>>,
    probe: bool,
    branch: Option<usize>,
    refusal: Option<usize>,
    pending: Option<PendingPage>,
    last_send: u64,
    effect_seen: bool,
    terminal_seen: bool,
    /// Failed reach attempts of this conversation: Talks the engine cancelled
    /// with "I can't reach that!", and failed re-approaches after a cancel.
    reach_failures: u8,
    /// A failed attempt is retried only on a later observed tick.
    retry_after: Option<u64>,
    /// Observed tick on which the current dialogue began.
    attempt_since: u64,
}
enum ChatDone {
    Close {
        missing: Option<u16>,
        witnessed: bool,
    },
    Drained {
        branch: Option<usize>,
        refusal: Option<usize>,
    },
}
impl ChatDrive {
    fn unstarted(probe: bool) -> Self {
        Self {
            handle: None,
            probe,
            branch: None,
            refusal: None,
            pending: None,
            last_send: 0,
            effect_seen: false,
            terminal_seen: false,
            reach_failures: 0,
            retry_after: None,
            attempt_since: 0,
        }
    }
    fn begin(
        actions: &mut NativeActions,
        cx: &mut ActionContext<'_>,
        row: &PrereqStep,
        probe: bool,
    ) -> Result<Self, ActionError> {
        let mut drive = Self::unstarted(probe);
        drive.start(actions, cx, row)?;
        Ok(drive)
    }
    fn start(
        &mut self,
        actions: &mut NativeActions,
        cx: &mut ActionContext<'_>,
        row: &PrereqStep,
    ) -> Result<(), ActionError> {
        if !cx.may_begin_this_tick() {
            return Err(ActionError::BudgetExhausted);
        }
        let options = if self.probe {
            DialogueOptions::continue_only()
        } else {
            DialogueOptions {
                strict: true,
                first_page_allowance_ticks: Some(talk_allowance_ticks(row)),
                prefer: Arc::clone(&row.talk.prefer),
                line_rules: if row.talk.line_rules.is_empty() {
                    DialogueOptions::default().line_rules
                } else {
                    row.talk
                        .line_rules
                        .iter()
                        .map(|rule| LineRule {
                            when_line: Arc::clone(&rule.page),
                            choose: Arc::clone(&rule.choice),
                        })
                        .collect()
                },
                ..DialogueOptions::default()
            }
        };
        let target = if self.probe {
            DialogueTarget::Continuation
        } else {
            let data = api::game_data::for_revision(cx.pin().revision).map_err(|_| {
                ActionError::Unavailable("selected NPC facts are unavailable".into())
            })?;
            let name = data
                .npc_name(row.target.npc)
                .and_then(|npc| npc.display.as_deref())
                .ok_or_else(|| {
                    ActionError::Unavailable("prerequisite NPC name is unavailable".into())
                })?;
            DialogueTarget::Npc {
                id: row.target.npc,
                name: Arc::from(name),
            }
        };
        let handle = actions.begin::<Dialogue>(DialogueArgs { target, options }, cx)?;
        self.attempt_since = cx.evidence().tick;
        if let Some(send) = handle.inspect(Dialogue::last_send).flatten() {
            self.last_send = send.request_id;
            self.pending = Some(PendingPage::new(send, cx, row, false, false));
        }
        self.handle = Some(handle);
        Ok(())
    }
    /// The engine sent "I can't reach that!" and cleared the Talk
    /// (`Player.ts:1292-1295`). The wait ends at once as a reach failure: a
    /// cleared interaction never opens a page, so waiting longer cannot help.
    fn talk_cancelled(
        &mut self,
        cx: &ActionContext<'_>,
        row: &PrereqStep,
    ) -> Poll<Result<ChatDone, PrereqFailure>> {
        let pending = self.pending.take().expect("a pending Talk was cancelled");
        let tick = cx.evidence().tick;
        wait_log(pending.name, pending.wait, tick, "unreachable");
        talk_log(cx, row, "cancelled", tick);
        self.reach_failed(cx, row, TalkReachCause::Cancelled, pending.wait.since)
    }
    /// One reach attempt failed: a cancelled Talk, or a re-approach that ended
    /// while the NPC was not ready to talk. A failed walk is not terminal
    /// until its bounded retries are spent (AGENTS.md): a new dialogue reads
    /// both tiles again on a later observed tick, within
    /// [`TALK_REACH_RETRIES`]. The last failure's detail is kept.
    fn reach_failed(
        &mut self,
        cx: &ActionContext<'_>,
        row: &PrereqStep,
        cause: TalkReachCause,
        since: u64,
    ) -> Poll<Result<ChatDone, PrereqFailure>> {
        let tick = cx.evidence().tick;
        self.reach_failures += 1;
        if self.reach_failures > TALK_REACH_RETRIES {
            return Poll::Ready(Err(PrereqFailure::TalkUnreachable {
                attempts: self.reach_failures,
                since,
                at: tick,
                from: cx.snapshot().here().map(|here| here.value),
                npc: nearest_npc(cx, row.target.npc).map(|(tile, _, _)| tile),
                cause,
            }));
        }
        // Dropping the owner revokes the finished attempt's dialogue; the next
        // `start` approaches from the tiles observed on a later tick.
        self.handle = None;
        self.pending = None;
        self.retry_after = Some(tick);
        Poll::Pending
    }
    fn poll(
        &mut self,
        actions: &mut NativeActions,
        cx: &mut ActionContext<'_>,
        state: Option<&SlotState>,
        row: &PrereqStep,
        ordinal: u16,
        gate: &QuestGate,
    ) -> Poll<Result<ChatDone, PrereqFailure>> {
        if self.handle.is_none() {
            if self
                .retry_after
                .is_some_and(|tick| cx.evidence().tick <= tick)
            {
                return Poll::Pending;
            }
            match self.start(actions, cx, row) {
                Ok(()) => {}
                Err(ActionError::BudgetExhausted) => return Poll::Pending,
                Err(error) => return Poll::Ready(Err(action_failure(error))),
            }
        }
        let handle = self.handle.as_ref().expect("dialogue owner");
        let (owned, receipt) = handle
            .inspect_in(cx, |dialogue, cx| {
                (
                    dialogue.owned_chat_page(cx).map(|page| {
                        row.talk
                            .branches
                            .iter()
                            .position(|branch| {
                                branch.opener.as_ref() != "*"
                                    && chat_page_matches(page.texts, &branch.opener)
                            })
                            .or_else(|| {
                                row.talk
                                    .branches
                                    .iter()
                                    .position(|branch| branch.opener.as_ref() == "*")
                            })
                    }),
                    self.pending.as_ref().and_then(|pending| {
                        cx.interaction_receipt(pending.send.request_id).copied()
                    }),
                )
            })
            .unwrap_or((None, None));
        if let Some(pending) = &mut self.pending {
            if let Some(receipt) = receipt {
                if !receipt.accepted {
                    return Poll::Ready(Err(PrereqFailure::Action(
                        "dialogue send was rejected".into(),
                    )));
                }
                pending.wait.since = receipt.evidence.tick;
                pending.wait.chat_mark = receipt.chat_since;
                if cx.evidence().tick > receipt.evidence.tick
                    && pending.wait.changed(cx)
                    && (pending.send.kind != DialogueSendKind::Talk || owned.is_some())
                {
                    if pending.effect && !pending.effect_written {
                        if let Some(index) = self.branch {
                            record(
                                state,
                                row,
                                row.talk.branches[index].proves,
                                None,
                                FactSource::Page {
                                    row: ordinal,
                                    line: index as u16,
                                },
                                cx.evidence(),
                            );
                            self.effect_seen = true;
                            pending.effect_written = true;
                        }
                    }
                    if !pending.terminal || !chat_open(cx) {
                        self.terminal_seen |= pending.terminal;
                        wait_log(
                            pending.name,
                            pending.wait,
                            cx.evidence().tick,
                            "acknowledged",
                        );
                        self.pending = None;
                    }
                }
            }
            let cancelled = self.pending.as_ref().is_some_and(|pending| {
                receipt.is_some()
                    && pending.send.kind == DialogueSendKind::Talk
                    && reach::saw_cant_reach(cx, pending.wait.chat_mark)
            });
            if cancelled {
                return self.talk_cancelled(cx, row);
            }
            if let Some(pending) = &self.pending {
                if pending.wait.spent(cx.evidence().tick) {
                    return Poll::Ready(Err(pending
                        .wait
                        .failure(pending.name, cx.evidence().tick)));
                }
            }
        }
        if !self.probe && self.branch.is_none() {
            if let Some(index) = owned {
                let Some(index) = index else {
                    return Poll::Ready(Ok(ChatDone::Close {
                        missing: None,
                        witnessed: false,
                    }));
                };
                self.branch = Some(index);
                let branch = &row.talk.branches[index];
                if let Some(OpenerRequirement::FreeSlots { count, .. }) = &branch.requires_at_opener
                {
                    let free = cx.snapshot().inventory().map(|inventory| {
                        28usize.saturating_sub(
                            inventory
                                .value
                                .iter()
                                .filter(|item| item.def.id > 0 && item.count > 0)
                                .count(),
                        )
                    });
                    if free.is_none_or(|free| free < usize::from(*count)) {
                        return Poll::Ready(Ok(ChatDone::Close {
                            missing: Some(u16::from(*count)),
                            witnessed: false,
                        }));
                    }
                }
                if branch.outcome == BranchOutcome::Satisfied {
                    record(
                        state,
                        row,
                        branch.proves,
                        None,
                        FactSource::Page {
                            row: ordinal,
                            line: index as u16,
                        },
                        cx.evidence(),
                    );
                    return Poll::Ready(Ok(ChatDone::Close {
                        missing: None,
                        witnessed: true,
                    }));
                }
            } else {
                // An existing page is never an opener owned by this Talk-to.
                if chat_open(cx) && handle.inspect(Dialogue::last_send).flatten().is_none() {
                    return Poll::Ready(Ok(ChatDone::Close {
                        missing: None,
                        witnessed: false,
                    }));
                }
                if !chat_open(cx) {
                    // After a cancel, a failed re-approach is a reach failure
                    // of the same budget, not the end of the conversation.
                    // The walk receipt is read before the poll: a failed
                    // machine is dropped with its walk request.
                    let approach = if self.reach_failures > 0 {
                        handle
                            .inspect_in(cx, |dialogue, cx| dialogue.approach_walk(cx))
                            .flatten()
                    } else {
                        None
                    };
                    let result = actions.poll(handle, cx);
                    if let Some(send) = handle
                        .inspect(Dialogue::last_send)
                        .flatten()
                        .filter(|send| send.request_id != self.last_send)
                    {
                        self.last_send = send.request_id;
                        self.pending = Some(PendingPage::new(send, cx, row, false, false));
                    }
                    match result {
                        Poll::Ready(Err(error)) => return Poll::Ready(Err(action_failure(error))),
                        Poll::Ready(Ok(DialogueOutcome::Failed)) if approach.is_some() => {
                            let walk = approach.flatten();
                            let tick = cx.evidence().tick;
                            talk_log(cx, row, &format!("approach-failed walk={walk:?}"), tick);
                            let since = self.attempt_since;
                            return self.reach_failed(
                                cx,
                                row,
                                TalkReachCause::Approach { walk },
                                since,
                            );
                        }
                        Poll::Ready(Ok(_)) => {
                            return Poll::Ready(Err(PrereqFailure::DialogueFailed))
                        }
                        Poll::Pending => {}
                    }
                }
                return Poll::Pending;
            }
        }
        if self.probe && self.refusal.is_none() {
            if let Some((index, refusal)) = row
                .probe
                .refuse
                .iter()
                .enumerate()
                .find(|(_, refusal)| contains(cx, &refusal.page))
            {
                self.refusal = Some(index);
                let refused = match gate {
                    QuestGate::Window(window) => Some(window.values),
                    _ => None,
                };
                record(
                    state,
                    row,
                    refusal.proves,
                    refused,
                    FactSource::Refusal {
                        row: ordinal,
                        entry: index as u8,
                    },
                    cx.evidence(),
                );
            }
        }
        let effect = self.branch.is_some_and(|index| {
            row.talk.branches[index]
                .effect_page
                .as_deref()
                .is_some_and(|text| contains(cx, text))
        });
        let terminal = if self.probe {
            self.refusal
                .is_some_and(|index| contains(cx, &row.probe.refuse[index].page))
        } else {
            self.branch.is_some_and(|index| {
                row.talk.branches[index]
                    .terminal
                    .as_deref()
                    .is_some_and(|text| contains(cx, text))
            })
        };
        let result = actions.poll(handle, cx);
        if let Some(send) = handle
            .inspect(Dialogue::last_send)
            .flatten()
            .filter(|send| send.request_id != self.last_send)
        {
            self.last_send = send.request_id;
            self.pending = Some(PendingPage::new(send, cx, row, effect, terminal));
        }
        match result {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => Poll::Ready(Err(action_failure(error))),
            Poll::Ready(Ok(DialogueOutcome::Failed)) => {
                Poll::Ready(Err(PrereqFailure::DialogueFailed))
            }
            Poll::Ready(Ok(DialogueOutcome::Completed))
                if self.probe || (self.effect_seen && self.terminal_seen && !chat_open(cx)) =>
            {
                Poll::Ready(Ok(ChatDone::Drained {
                    branch: self.branch,
                    refusal: self.refusal,
                }))
            }
            Poll::Ready(Ok(_)) => Poll::Ready(Err(PrereqFailure::DialogueFailed)),
        }
    }
}

/// Closing is an owned action, not an emission made after dropping a dialogue.
struct ClosePage {
    request_id: u64,
    wait: TickWait,
    closed_at: Option<u64>,
}
impl NativeMachine for ClosePage {
    type Args = ();
    type Output = Result<(), PrereqFailure>;
    fn begin(_: (), cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        let wait = TickWait::new(cx, PAGE_TICKS + SEND_SLACK_TICKS).started(PrereqWaitName::W4);
        let request_id = cx.emit(InteractReq::CloseModal)?;
        Ok(Self {
            request_id,
            wait,
            closed_at: None,
        })
    }
    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        if let Some(receipt) = cx.interaction_receipt(self.request_id) {
            if !receipt.accepted {
                return Poll::Ready(Err(ActionError::Failed(
                    "close modal dispatch rejected".into(),
                )));
            }
            self.wait.since = receipt.evidence.tick;
            if cx.evidence().tick > receipt.evidence.tick && !chat_open(cx) {
                let closed = *self.closed_at.get_or_insert_with(|| {
                    wait_log(PrereqWaitName::W4, self.wait, cx.evidence().tick, "closed");
                    cx.evidence().tick
                });
                if cx.evidence().tick.saturating_sub(closed) >= DIALOG_GAP_TICKS {
                    return Poll::Ready(Ok(Ok(())));
                }
            } else if chat_open(cx) {
                self.closed_at = None;
            }
        }
        if self.closed_at.is_none() && self.wait.spent(cx.evidence().tick) {
            return Poll::Ready(Ok(Err(self
                .wait
                .failure(PrereqWaitName::W4, cx.evidence().tick))));
        }
        Poll::Pending
    }
    fn cancel(&mut self) {}
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
