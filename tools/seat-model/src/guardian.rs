//! C6: ownership of one Guardian event, composed with the seat's guardian hold.
//!
//! The model owns the family-independent part of the Guardian: who owns a
//! detected event (the host solver or the running script), the bounded wait of
//! the host work in flight, the stalled and released outcomes, and the
//! publications (`claim`, `handling`, `ours`, `hold`) derived from them. It does
//! not model any family's puzzle. A family is abstracted to how its host work
//! behaves: a dialog handle, a solver with one operation and a bounded wait, or
//! a trap square that holds while the host walks out.
//!
//! The model changes in `Tick`/`Frame` only. Every other step is the world
//! changing between Guardian passes (an event appears, a script pauses, the
//! toggle flips). Invariants that read the world are checked on the state a
//! fresh `Tick` leaves ("within one tick"), the others after every step.
//!
//! Each rejected design and each defect the audit found is a
//! [`GuardianMutation`] that an invariant must keep catching (ADR 0017).
//! Dispatch exclusion composes with the seat: the published `hold` drives the
//! seat's `SessionHeld` / `SessionResumed`. A host operation never goes through
//! the seat, so the model checks it against the owner directly (the seat's own
//! `HostAction` counters are unbounded and stay out of the composition).
use crate::explorer::{CapKind, ExploreLimits, state_heap_bytes};
use crate::{Bounds, Class, Rules, SessionFamily, State, Step, Violation};
use std::collections::{HashMap, VecDeque};
use std::mem::size_of;
use std::time::{Duration, Instant};

/// How a Guardian family behaves toward the owner. Concrete solvers are
/// exercised by replay tests against the real code.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Family {
    /// A Talk-to handle: sent once, continued until the chat closes or the
    /// continue cap releases it. The handle outlives a missing NPC frame.
    Dialog,
    /// One operation, then a bounded wait for its outcome (Open, Rub, a
    /// flee walk, a tool refit). Exhausted, it stalls and releases the slot.
    Solver,
    /// A trap square: the hold follows the detection while the host walks out;
    /// a bounded number of solver ticks gives the visit up.
    Trap,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ScriptExit {
    Pause,
    Stop,
    Fail,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Script {
    Stopped,
    Paused,
    Running,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Owner {
    Host,
    Script,
}

/// Permanent seeded regressions. Production exploration uses `None`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum GuardianMutation {
    #[default]
    None,
    /// 0.2.4: the toggle off still holds the slot (stuck Gatherer).
    StuckGathererHold,
    /// A stalled family keeps publishing hold.
    StalledHolds,
    /// 0.2.5: a tick the server blocked (a stun) spends the flee budget.
    FleeInterrupted,
    /// 0.2.7: a script that pauses, stops or fails keeps its claim.
    ClaimKeptOnExit(ScriptExit),
    /// 0.2.7: a missing Open response is waited for forever.
    MissingOpenWaitsForever,
    /// Time while a script owns the event spends the host's wait.
    ScriptTimeCountsWait,
    /// An old or player chat line re-arms an exhausted box.
    StaleChatRearms,
    /// A late result of an earlier operation settles the current one.
    WrongTokenResult,
    /// A script exit releases the seat hold of a host-owned event.
    GlobalUnhold,
    /// `hold` is kept beside the active event as its own latch.
    IndependentHold,
    /// A script claim takes an event that is forced to the host.
    ForcedHostIgnored,
    /// A stalled episode restarts its budget on the next tick.
    ReleasedActorRestarts,
    /// The host acts on an event a script owns.
    DispatchUnderScript,
    /// The published hold never reaches the seat.
    HoldNotAppliedToSeat,
    /// An exhausted episode masks an independent event.
    OverlapMasksBox,
    /// A repeated frame poll spends a wait tick.
    FrameConsumesTick,
    /// A replaced session keeps the previous session's active event.
    SessionKeepsActive,
    /// The toggle off still publishes `ours`.
    ToggleOffKeepsOurs,
    /// The host re-sends a pending operation when it regains the claim.
    DuplicateOpOnReclaim,
    /// A regained claim replays the rising-edge callback.
    SuccessorCallbackReplay,
}

impl GuardianMutation {
    /// Every mutation, with the invariant that must catch it.
    pub const ALL: [(Self, &'static str); 22] = [
        (Self::StuckGathererHold, "toggle-off-publishes"),
        (Self::StalledHolds, "stalled-hold"),
        (Self::FleeInterrupted, "wait-early"),
        (
            Self::ClaimKeptOnExit(ScriptExit::Pause),
            "owner-without-script",
        ),
        (
            Self::ClaimKeptOnExit(ScriptExit::Stop),
            "owner-without-script",
        ),
        (
            Self::ClaimKeptOnExit(ScriptExit::Fail),
            "owner-without-script",
        ),
        (Self::MissingOpenWaitsForever, "wait-unbounded"),
        (Self::ScriptTimeCountsWait, "script-time-counted"),
        (Self::StaleChatRearms, "stale-rearm"),
        (Self::WrongTokenResult, "wrong-token"),
        (Self::GlobalUnhold, "seat-hold-mismatch"),
        (Self::IndependentHold, "publication-derived"),
        (Self::ForcedHostIgnored, "owner-forced-host"),
        (Self::ReleasedActorRestarts, "released-actor-restart"),
        (Self::DispatchUnderScript, "dispatch-exclusion"),
        (Self::HoldNotAppliedToSeat, "seat-hold-mismatch"),
        (Self::OverlapMasksBox, "unowned-event"),
        (Self::FrameConsumesTick, "frame-advances"),
        (Self::SessionKeepsActive, "stale-session"),
        (Self::ToggleOffKeepsOurs, "toggle-off-publishes"),
        (Self::DuplicateOpOnReclaim, "duplicate-op"),
        (Self::SuccessorCallbackReplay, "duplicate-callback"),
    ];
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GuardianBounds {
    /// Host ticks a bounded wait may last (the model's stand-in for a
    /// deadline; a long deadline is before, due or after it).
    pub wait: u8,
    /// Distinct event episodes issued.
    pub ids: u8,
    /// Slot sessions.
    pub sessions: u8,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Phase {
    /// No host work in flight.
    Idle,
    /// Host work in flight. `wait` is the Guardian's counter; `unblocked` is
    /// the ticks that could progress, kept by the monitor.
    Pending { wait: u8, unblocked: u8 },
    /// The bounded wait ended with a logged failure; the slot is released.
    Stalled { logged: bool, unblocked: u8 },
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct Event {
    id: u8,
    family: Family,
    /// The policy forces the host to keep the event (Swarm, the angry plant).
    forced_host: bool,
}

/// One owner of one episode and the host work it has in flight.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct Active {
    key: u8,
    session: u8,
    family: Family,
    owner: Owner,
    phase: Phase,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct Publication {
    pub claim_script: bool,
    pub handling: bool,
    pub ours: bool,
    pub hold: bool,
}

/// The inputs of a derivation, as the real Guardian passes them.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PublicationInputs {
    pub toggle: bool,
    /// The detector reports an event at all.
    pub detected: bool,
    pub claim_script: bool,
    pub dialog_in_flight: bool,
    pub solver_in_flight: bool,
    /// The family is stalled (inert): detect-only, never holds.
    pub stalled: bool,
    pub trapped: bool,
}

/// The one derivation of the ownership publications. The real Guardian's
/// `derive_publication` computes the same function (conformance asserts it at
/// every step).
pub fn derive(input: PublicationInputs) -> Publication {
    let handling = input.dialog_in_flight;
    Publication {
        claim_script: input.claim_script,
        handling,
        ours: input.toggle && input.detected && !input.stalled,
        hold: input.toggle
            && !input.claim_script
            && !input.stalled
            && (handling || input.solver_in_flight || (input.trapped && input.detected)),
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum GuardianStep {
    Appear {
        family: Family,
        forced_host: bool,
    },
    Replace {
        family: Family,
        forced_host: bool,
    },
    Disappear,
    ScriptStart {
        wants: bool,
        live: bool,
    },
    ScriptExit(ScriptExit),
    ScriptResume,
    ScriptWants(bool),
    SetEnabled(bool),
    SessionReplace,
    ChatClose,
    Replicate {
        fresh: bool,
    },
    LateResult,
    Tick {
        blocked: bool,
    },
    Frame,
    /// A seat step of the composition (script dispatch, emissions, takes).
    Seat(Step),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct GuardianState {
    bounds: GuardianBounds,
    mutation: GuardianMutation,
    // The world.
    session: u8,
    enabled: bool,
    event: Option<Event>,
    /// The event as the last fresh pass saw it: a repeated frame polls the
    /// same snapshot and cannot see a later change.
    observed: Option<Event>,
    script: Script,
    script_exit: Option<ScriptExit>,
    wants: bool,
    live: bool,
    chat_open: bool,
    next_id: u8,
    // The Guardian.
    last_sig: Option<u8>,
    knocks: u8,
    active: Option<Active>,
    stale_hold: bool,
    // What the last pass published.
    out: Publication,
    emitted: bool,
    resolved_now: bool,
    out_fresh: bool,
    settled: bool,
    // Composition.
    seat: Option<Box<State>>,
    violation: Option<(&'static str, String)>,
}

impl GuardianState {
    pub fn new(bounds: GuardianBounds, mutation: GuardianMutation) -> Self {
        Self {
            bounds,
            mutation,
            session: 0,
            enabled: true,
            event: None,
            observed: None,
            script: Script::Stopped,
            script_exit: None,
            wants: false,
            live: false,
            chat_open: false,
            next_id: 0,
            last_sig: None,
            knocks: 0,
            active: None,
            stale_hold: false,
            out: Publication::default(),
            emitted: false,
            resolved_now: false,
            out_fresh: false,
            settled: false,
            seat: None,
            violation: None,
        }
    }

    /// Compose with a seat that holds one admitted Gather session with queued
    /// gameplay and a kept release, so a hold has custody to keep.
    pub fn composed(bounds: GuardianBounds, mutation: GuardianMutation) -> Self {
        let seat_bounds = Bounds {
            roots: 1,
            live: 1,
            depth: 1,
            credits: 2,
            pending: 1,
            history: 1,
            rows: 3,
            gameplay: 1,
            ids: 1,
            batches: 1,
        };
        let mut seat = State::new(Rules::Chosen, seat_bounds);
        for step in [
            Step::Start {
                parent: None,
                local_parent: None,
                class: Class::Session(SessionFamily::Gather),
                group: 2,
                bytes: 0,
                hooks: 0,
            },
            Step::Admit(1),
            Step::Emit {
                of: Some(1),
                release: false,
            },
        ] {
            seat.apply(&step).expect("composition prefix is enabled");
        }
        let mut state = Self::new(bounds, mutation);
        state.seat = Some(Box::new(seat));
        state
    }

    pub fn mutation(&self) -> GuardianMutation {
        self.mutation
    }

    pub fn publication(&self) -> Publication {
        self.out
    }

    pub fn seat(&self) -> Option<&State> {
        self.seat.as_deref()
    }

    pub fn emitted(&self) -> bool {
        self.emitted
    }

    /// The episode id of the detected event.
    pub fn event_id(&self) -> Option<u8> {
        self.event.map(|event| event.id)
    }

    fn owner(&self) -> Owner {
        self.active.map_or(Owner::Host, |active| active.owner)
    }

    fn set_owner(&mut self, owner: Owner) {
        if owner == Owner::Host && self.active.is_none() {
            return;
        }
        let previous = self.owner();
        match self.active.as_mut() {
            Some(active) => active.owner = owner,
            None => {
                self.active = Some(Active {
                    key: self.last_sig.unwrap_or(0),
                    session: self.session,
                    family: self.observed.map_or(Family::Solver, |event| event.family),
                    owner,
                    phase: Phase::Idle,
                });
            }
        }
        if previous == Owner::Host
            && owner == Owner::Script
            && self.mutation == GuardianMutation::SuccessorCallbackReplay
        {
            self.knocks = self.knocks.saturating_add(1);
        }
        self.settle();
    }

    /// The Host owns the episode and has nothing in flight: no active event.
    fn settle(&mut self) {
        if self
            .active
            .is_some_and(|active| active.owner == Owner::Host && active.phase == Phase::Idle)
        {
            self.active = None;
        }
    }

    fn claim_returns(&self) -> bool {
        match (self.mutation, self.script_exit) {
            (GuardianMutation::ClaimKeptOnExit(kept), Some(exit)) => kept != exit,
            _ => true,
        }
    }

    fn script_claims(&self) -> bool {
        self.script == Script::Running && self.wants
    }

    pub fn apply(&mut self, step: &GuardianStep) -> Result<(), String> {
        self.violation = None;
        let pre = self.active;
        if !matches!(step, GuardianStep::Seat(_)) {
            self.out_fresh = false;
            self.settled = false;
            self.emitted = false;
            self.resolved_now = false;
        }
        match step {
            GuardianStep::Appear {
                family,
                forced_host,
            } => {
                if self.event.is_some() || self.next_id >= self.bounds.ids {
                    return Err("an event is present or the ids are spent".into());
                }
                self.event = Some(Event {
                    id: self.next_id,
                    family: *family,
                    forced_host: *forced_host,
                });
                self.next_id += 1;
            }
            GuardianStep::Replace {
                family,
                forced_host,
            } => {
                if self.event.is_none() || self.next_id >= self.bounds.ids {
                    return Err("no event to replace or the ids are spent".into());
                }
                self.event = Some(Event {
                    id: self.next_id,
                    family: *family,
                    forced_host: *forced_host,
                });
                self.next_id += 1;
            }
            GuardianStep::Disappear => {
                if self.event.is_none() {
                    return Err("no event".into());
                }
                self.event = None;
            }
            GuardianStep::ScriptStart { wants, live } => {
                if self.script != Script::Stopped {
                    return Err("a script is already started".into());
                }
                self.script = Script::Running;
                self.script_exit = None;
                self.wants = *wants;
                self.live = *live;
            }
            GuardianStep::ScriptExit(exit) => {
                let allowed = match exit {
                    ScriptExit::Pause => self.script == Script::Running,
                    ScriptExit::Stop | ScriptExit::Fail => self.script != Script::Stopped,
                };
                if !allowed {
                    return Err("the script cannot leave that way".into());
                }
                self.script = match exit {
                    ScriptExit::Pause => Script::Paused,
                    ScriptExit::Stop | ScriptExit::Fail => Script::Stopped,
                };
                self.script_exit =
                    matches!(self.mutation, GuardianMutation::ClaimKeptOnExit(_)).then_some(*exit);
                if self.mutation == GuardianMutation::GlobalUnhold
                    && let Some(seat) = self.seat.as_mut()
                {
                    let _ = seat.apply(&Step::SessionResumed);
                }
            }
            GuardianStep::ScriptResume => {
                if self.script != Script::Paused {
                    return Err("the script is not paused".into());
                }
                self.script = Script::Running;
            }
            GuardianStep::ScriptWants(wants) => {
                if self.script != Script::Running || !self.live || self.wants == *wants {
                    return Err("no live membership change".into());
                }
                self.wants = *wants;
            }
            GuardianStep::SetEnabled(enabled) => {
                if self.enabled == *enabled {
                    return Err("the toggle already has that value".into());
                }
                self.enabled = *enabled;
            }
            GuardianStep::SessionReplace => {
                if self.session + 1 >= self.bounds.sessions {
                    return Err("sessions are spent".into());
                }
                self.session += 1;
                self.script = Script::Stopped;
                self.script_exit = matches!(self.mutation, GuardianMutation::ClaimKeptOnExit(_))
                    .then_some(ScriptExit::Stop);
                self.chat_open = false;
                // The new session's snapshot has not seen the world yet.
                self.observed = None;
            }
            GuardianStep::ChatClose => {
                if !self.chat_open {
                    return Err("no chat".into());
                }
                self.chat_open = false;
            }
            GuardianStep::Replicate { fresh } => {
                let Some(active) = self.active.as_mut() else {
                    return Err("nothing to re-arm".into());
                };
                if !matches!(active.phase, Phase::Stalled { .. }) {
                    return Err("not stalled".into());
                }
                if *fresh || self.mutation == GuardianMutation::StaleChatRearms {
                    active.phase = Phase::Idle;
                }
            }
            GuardianStep::LateResult => {
                let Some(active) = self.active.as_mut() else {
                    return Err("no active event".into());
                };
                if !matches!(active.phase, Phase::Pending { .. }) {
                    return Err("no pending operation".into());
                }
                if self.mutation == GuardianMutation::WrongTokenResult {
                    active.phase = Phase::Idle;
                }
            }
            GuardianStep::Tick { blocked } => self.tick(true, *blocked),
            GuardianStep::Frame => self.tick(false, false),
            GuardianStep::Seat(step) => {
                let Some(seat) = self.seat.as_mut() else {
                    return Err("no seat in this composition".into());
                };
                seat.apply(step)?;
            }
        }
        self.settle();
        // Spec: only a fresh server signal re-arms a stalled family, and only
        // an operation's own result settles it.
        let stalled_before = matches!(pre.map(|active| active.phase), Some(Phase::Stalled { .. }));
        let pending_before = matches!(pre.map(|active| active.phase), Some(Phase::Pending { .. }));
        let same_key = pre.is_some_and(|pre| self.active.is_some_and(|now| now.key == pre.key));
        let stalled_now = same_key
            && matches!(
                self.active.map(|active| active.phase),
                Some(Phase::Stalled { .. })
            );
        let pending_now = same_key
            && matches!(
                self.active.map(|active| active.phase),
                Some(Phase::Pending { .. })
            );
        match step {
            GuardianStep::Replicate { fresh: false } if stalled_before && !stalled_now => {
                self.violation = Some((
                    "stale-rearm",
                    "a line that is not a fresh server signal re-armed the stalled family".into(),
                ));
            }
            GuardianStep::LateResult if pending_before && !pending_now => {
                self.violation = Some((
                    "wrong-token",
                    "a result carrying an earlier token settled the pending operation".into(),
                ));
            }
            _ => {}
        }
        Ok(())
    }

    fn tick(&mut self, fresh: bool, blocked: bool) {
        let before = self.active;
        self.settled = fresh;
        // A new session's pass never owns the previous session's event.
        if self
            .active
            .is_some_and(|active| active.session != self.session)
            && self.mutation != GuardianMutation::SessionKeepsActive
        {
            self.active = None;
            self.last_sig = None;
            self.knocks = 0;
        }
        if fresh {
            self.observed = self.event;
        }
        let event = self.observed;
        if fresh {
            // `dialog_done`: a finished handle clears before the claim and the
            // act, whoever owns the event and whatever the toggle says.
            if let Some(Active {
                family: Family::Dialog,
                phase: Phase::Pending { .. },
                key,
                ..
            }) = self.active
                && !self.chat_open
                && event.map(|event| event.id) != Some(key)
            {
                self.finish(Phase::Idle);
            }
        }
        if fresh {
            let sig = event.map(|event| event.id);
            if sig != self.last_sig {
                self.last_sig = sig;
                self.knocks = 0;
                if let Some(active) = self.active.as_mut() {
                    active.owner = Owner::Host;
                }
                let claim = if event.is_some() && self.script_claims() {
                    Owner::Script
                } else {
                    Owner::Host
                };
                self.set_owner(claim);
                if claim == Owner::Script {
                    self.knocks = 1;
                }
            }
            if event.is_some() {
                if self.live {
                    let claim = if self.script_claims() {
                        Owner::Script
                    } else {
                        Owner::Host
                    };
                    if claim == Owner::Script || self.claim_returns() {
                        self.set_owner(claim);
                    }
                } else if self.script != Script::Running && self.claim_returns() {
                    self.set_owner(Owner::Host);
                }
            }
        }
        if event.is_some_and(|event| event.forced_host)
            && self.mutation != GuardianMutation::ForcedHostIgnored
        {
            self.set_owner(Owner::Host);
        }

        if fresh {
            self.host_pass(blocked);
        } else if self.mutation == GuardianMutation::FrameConsumesTick
            && let Some(Active {
                phase: Phase::Pending { wait, .. },
                ..
            }) = self.active.as_mut()
        {
            *wait = wait.saturating_add(1);
        }
        if self.mutation == GuardianMutation::DuplicateOpOnReclaim
            && before.is_some_and(|active| active.owner == Owner::Script)
            && self.owner() == Owner::Host
            && self.active.is_some_and(|active| {
                active.family == Family::Solver && matches!(active.phase, Phase::Pending { .. })
            })
        {
            // The pending operation is sent again when the claim comes back.
            self.emitted = true;
        }

        // Spec checks over the transition.
        if let Some((was, now)) = before
            .zip(self.active)
            .filter(|(was, now)| was.key == now.key && was.session == now.session)
        {
            if let (
                Phase::Pending {
                    wait: before_wait, ..
                },
                Phase::Pending {
                    wait: after_wait, ..
                },
            ) = (was.phase, now.phase)
            {
                if !fresh && after_wait != before_wait {
                    self.violation = Some((
                        "frame-advances",
                        "a repeated frame poll spent a wait tick".into(),
                    ));
                }
                if fresh && now.owner == Owner::Script && after_wait != before_wait {
                    self.violation = Some((
                        "script-time-counted",
                        "time under a script claim spent the host's wait".into(),
                    ));
                }
                if now.family == Family::Solver && self.emitted {
                    self.violation = Some((
                        "duplicate-op",
                        "the host sent a pending operation a second time".into(),
                    ));
                }
            }
            if matches!(was.phase, Phase::Stalled { .. })
                && matches!(now.phase, Phase::Pending { .. })
            {
                self.violation = Some((
                    "released-actor-restart",
                    "a stalled episode started a second budget".into(),
                ));
            }
        }

        // Publish: one derivation from the active event and the live inputs.
        let publication = self.publish();
        self.out = publication;
        self.out_fresh = true;
        if let Some(Active {
            phase: Phase::Pending { .. },
            ..
        }) = self.active
        {
            if self.mutation == GuardianMutation::IndependentHold && self.emitted {
                self.stale_hold = true;
            }
        } else if self.active.is_none() {
            self.stale_hold = false;
        }
        if self.mutation == GuardianMutation::IndependentHold && self.stale_hold {
            self.out.hold = true;
        }
        if let Some(seat) = self.seat.as_mut()
            && self.mutation != GuardianMutation::HoldNotAppliedToSeat
            && self.out.hold != seat.session_held()
        {
            let step = if self.out.hold {
                Step::SessionHeld
            } else {
                Step::SessionResumed
            };
            seat.apply(&step).expect("hold edges are always enabled");
        }
    }

    /// The host's act for this fresh tick.
    fn host_pass(&mut self, blocked: bool) {
        let event = self.observed;
        let enabled = self.enabled;
        // Episode state follows its event, whoever holds the claim: a stall
        // latch ends with its event, and a replaced event's idle shell ends.
        if let Some(active) = self.active {
            let gone = event.map(|event| event.id) != Some(active.key);
            if gone && active.family != Family::Dialog {
                match active.phase {
                    Phase::Stalled { .. } if self.mutation == GuardianMutation::OverlapMasksBox => {
                    }
                    Phase::Stalled { .. } | Phase::Idle => {
                        // The episode's work ends; the owner stays.
                        if let Some(active) = self.active.as_mut() {
                            active.phase = Phase::Idle;
                        }
                        self.settle();
                    }
                    Phase::Pending { .. } => {}
                }
            }
        }
        if !enabled || self.owner() != Owner::Host {
            // A script owns the event, or the toggle is off: the host's work
            // in flight keeps its place and spends nothing. Under a script
            // claim, the mutation lets time count anyway.
            if self.owner() == Owner::Script
                && self.mutation == GuardianMutation::ScriptTimeCountsWait
                && let Some(Active {
                    phase: Phase::Pending { wait, .. },
                    ..
                }) = self.active.as_mut()
            {
                *wait = wait.saturating_add(1);
            }
            if self.mutation == GuardianMutation::DispatchUnderScript
                && enabled
                && self.owner() == Owner::Script
                && self
                    .active
                    .is_some_and(|active| active.phase == Phase::Idle)
                && let Some(event) = event
            {
                self.start(event);
            }
            return;
        }
        let Some(active) = self.active else {
            if let Some(event) = event {
                self.start(event);
            }
            return;
        };
        let gone = event.map(|event| event.id) != Some(active.key);
        match active.phase {
            Phase::Pending { wait, unblocked } => {
                // Work for an event that is gone or replaced resolves; the next
                // pass starts the successor.
                if active.family != Family::Dialog && gone {
                    self.finish(Phase::Idle);
                    return;
                }
                if active.family == Family::Dialog && !self.chat_open {
                    // A continue the closed chat refuses: one tick of grace
                    // after the Talk-to, then the handle clears.
                    if wait == 0 {
                        if let Some(active) = self.active.as_mut() {
                            active.phase = Phase::Pending {
                                wait: 1,
                                unblocked: unblocked + 1,
                            };
                        }
                    } else {
                        self.finish(Phase::Idle);
                    }
                    return;
                }
                let counts = !blocked
                    || active.family != Family::Solver
                    || self.mutation == GuardianMutation::FleeInterrupted;
                let next_wait = if counts { wait + 1 } else { wait };
                let next_unblocked = if blocked && active.family == Family::Solver {
                    unblocked
                } else {
                    unblocked + 1
                };
                if self.mutation == GuardianMutation::MissingOpenWaitsForever
                    && active.family == Family::Solver
                {
                    // The wait never ends: the counter saturates below the bound.
                    let capped = next_wait.min(self.bounds.wait.saturating_sub(1));
                    if let Some(active) = self.active.as_mut() {
                        active.phase = Phase::Pending {
                            wait: capped,
                            unblocked: next_unblocked,
                        };
                    }
                    return;
                }
                if next_wait >= self.bounds.wait {
                    let ended = if active.family == Family::Dialog {
                        Phase::Idle
                    } else {
                        Phase::Stalled {
                            logged: true,
                            unblocked: next_unblocked,
                        }
                    };
                    self.finish(ended);
                } else if let Some(active) = self.active.as_mut() {
                    active.phase = Phase::Pending {
                        wait: next_wait,
                        unblocked: next_unblocked,
                    };
                    if active.family == Family::Trap {
                        self.emitted = true;
                    }
                }
            }
            Phase::Stalled { .. } => {
                // Inert while the event stays. The mutation restarts the budget.
                if self.mutation == GuardianMutation::ReleasedActorRestarts
                    && !gone
                    && let Some(event) = event
                {
                    self.start(event);
                }
            }
            Phase::Idle => {
                if let Some(event) = event {
                    self.start(event);
                }
            }
        }
    }

    fn finish(&mut self, phase: Phase) {
        if let Some(active) = self.active.as_mut() {
            active.phase = phase;
        }
        self.resolved_now = true;
        self.settle();
    }

    fn start(&mut self, event: Event) {
        let owner = self.owner();
        self.active = Some(Active {
            key: event.id,
            session: self.session,
            family: event.family,
            owner,
            phase: Phase::Pending {
                wait: 0,
                unblocked: 0,
            },
        });
        if event.family == Family::Dialog {
            self.chat_open = true;
        }
        self.emitted = true;
        if owner == Owner::Script {
            self.violation = Some((
                "dispatch-exclusion",
                "the host acted on an event a script owns".into(),
            ));
        }
    }

    fn inputs(&self) -> PublicationInputs {
        let event = self.observed;
        let active = self.active;
        let matches_event = |active: &Active| event.is_some_and(|event| event.id == active.key);
        PublicationInputs {
            toggle: self.enabled,
            detected: event.is_some(),
            claim_script: active.is_some_and(|active| active.owner == Owner::Script),
            dialog_in_flight: active.is_some_and(|active| {
                active.family == Family::Dialog && matches!(active.phase, Phase::Pending { .. })
            }),
            solver_in_flight: active.is_some_and(|active| {
                active.family != Family::Dialog && matches!(active.phase, Phase::Pending { .. })
            }),
            stalled: active.is_some_and(|active| {
                matches_event(&active) && matches!(active.phase, Phase::Stalled { .. })
            }),
            trapped: event.is_some_and(|event| event.family == Family::Trap),
        }
    }

    fn publish(&self) -> Publication {
        let input = self.inputs();
        let mut publication = derive(input);
        if self.mutation == GuardianMutation::StuckGathererHold && !input.toggle {
            publication.hold = !input.claim_script
                && (publication.handling || input.solver_in_flight || input.trapped);
        }
        if self.mutation == GuardianMutation::StalledHolds && input.stalled && input.toggle {
            publication.hold = !input.claim_script;
        }
        if self.mutation == GuardianMutation::ToggleOffKeepsOurs {
            publication.ours = input.detected && !input.stalled;
        }
        publication
    }

    pub fn check(&self) -> Result<(), Violation> {
        let fail = |name: &'static str, detail: String| Err(Violation { name, detail });
        if let Some((name, detail)) = &self.violation {
            return fail(name, detail.clone());
        }
        if let Some(seat) = &self.seat {
            seat.check()?;
            if seat.session_held() != self.out.hold {
                return fail(
                    "seat-hold-mismatch",
                    format!(
                        "the seat is held={} but the Guardian published hold={}",
                        seat.session_held(),
                        self.out.hold
                    ),
                );
            }
        }
        if let Some(active) = self.active {
            if (self.settled || self.out_fresh) && active.session != self.session {
                return fail(
                    "stale-session",
                    format!(
                        "the active event belongs to session {} in session {}",
                        active.session, self.session
                    ),
                );
            }
            if let Phase::Pending { unblocked, .. } = active.phase
                && self.settled
                && unblocked >= self.bounds.wait
            {
                return fail(
                    "wait-unbounded",
                    format!(
                        "a wait still pending after {unblocked} ticks that could progress (bound {})",
                        self.bounds.wait
                    ),
                );
            }
            if let Phase::Stalled { logged, unblocked } = active.phase {
                if !logged {
                    return fail(
                        "wait-unlogged",
                        "a wait ended without a logged failure".into(),
                    );
                }
                if unblocked < self.bounds.wait && active.family == Family::Solver {
                    return fail(
                        "wait-early",
                        format!(
                            "the family gave up after {unblocked} progressing ticks (bound {})",
                            self.bounds.wait
                        ),
                    );
                }
            }
        }
        if self.knocks > 1 {
            return fail(
                "duplicate-callback",
                format!(
                    "the rising-edge callback ran {} times for one event",
                    self.knocks
                ),
            );
        }
        if self.out_fresh {
            if !self.enabled && (self.out.hold || self.out.ours) {
                return fail(
                    "toggle-off-publishes",
                    format!(
                        "the toggle is off but the Guardian published {:?}",
                        self.out
                    ),
                );
            }
            let input = self.inputs();
            if input.stalled && self.out.hold {
                return fail("stalled-hold", "a stalled family published hold".into());
            }
            if self.out != derive(input) {
                return fail(
                    "publication-derived",
                    format!(
                        "published {:?} differs from the derivation {:?}",
                        self.out,
                        derive(input)
                    ),
                );
            }
            if self.event.is_some_and(|event| event.forced_host)
                && self.settled
                && self.out.claim_script
            {
                return fail(
                    "owner-forced-host",
                    "a script owns an event the policy forces to the host".into(),
                );
            }
        }
        if self.settled {
            if self.script != Script::Running && self.out.claim_script {
                return fail(
                    "owner-without-script",
                    format!("the script is {:?} but still owns the event", self.script),
                );
            }
            if self.out.claim_script && !self.script_claims() && self.live {
                return fail(
                    "owner-without-script",
                    "a live script no longer claims the event but owns it".into(),
                );
            }
            if let Some(event) = self.event {
                let owned = self
                    .active
                    .is_some_and(|active| active.owner == Owner::Script);
                // A dialog handle keeps its own NPC: a replacement waits for the
                // handle's bounded end, and the host stays engaged on it.
                let host_engaged = self.active.is_some_and(|active| {
                    active.owner == Owner::Host
                        && ((active.key == event.id && !matches!(active.phase, Phase::Idle))
                            || (active.family == Family::Dialog
                                && matches!(active.phase, Phase::Pending { .. })))
                });
                let stalled = self.active.is_some_and(|active| {
                    active.key == event.id && matches!(active.phase, Phase::Stalled { .. })
                });
                if self.enabled && !owned && !host_engaged && !stalled && !self.resolved_now {
                    return fail(
                        "unowned-event",
                        format!(
                            "event {} is detected and enabled but neither the host nor a script owns it ({:?})",
                            event.id, self.active
                        ),
                    );
                }
            }
        }
        Ok(())
    }

    pub fn describe(&self) -> String {
        format!(
            "event={:?} script={:?} enabled={} session={} active={:?} out={:?} knocks={}",
            self.event, self.script, self.enabled, self.session, self.active, self.out, self.knocks
        )
    }
}

fn alphabet(state: &GuardianState) -> Vec<GuardianStep> {
    let mut out = Vec::new();
    for (family, forced_host) in [
        (Family::Dialog, false),
        (Family::Solver, false),
        (Family::Solver, true),
        (Family::Trap, false),
    ] {
        out.push(GuardianStep::Appear {
            family,
            forced_host,
        });
        out.push(GuardianStep::Replace {
            family,
            forced_host,
        });
    }
    out.push(GuardianStep::Disappear);
    for wants in [false, true] {
        for live in [false, true] {
            out.push(GuardianStep::ScriptStart { wants, live });
        }
    }
    out.extend([
        GuardianStep::ScriptExit(ScriptExit::Pause),
        GuardianStep::ScriptExit(ScriptExit::Stop),
        GuardianStep::ScriptExit(ScriptExit::Fail),
        GuardianStep::ScriptResume,
        GuardianStep::ScriptWants(false),
        GuardianStep::ScriptWants(true),
        GuardianStep::SetEnabled(false),
        GuardianStep::SetEnabled(true),
        GuardianStep::SessionReplace,
        GuardianStep::ChatClose,
        GuardianStep::Replicate { fresh: true },
        GuardianStep::Replicate { fresh: false },
        GuardianStep::LateResult,
        GuardianStep::Tick { blocked: false },
        GuardianStep::Tick { blocked: true },
        GuardianStep::Frame,
    ]);
    if state.seat.is_some() {
        out.extend([
            GuardianStep::Seat(Step::Take),
            GuardianStep::Seat(Step::Dispatch),
        ]);
    }
    out
}

/// A named bounded exploration of the ownership model.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GuardianProfile {
    /// The ownership model alone.
    Ownership,
    /// Ownership composed with the seat's guardian hold.
    OwnershipSeat,
    /// A deeper bound for the nightly Linux lane (16 GiB scope).
    Nightly,
}

pub const GUARDIAN_PROFILES: [GuardianProfile; 3] = [
    GuardianProfile::Ownership,
    GuardianProfile::OwnershipSeat,
    GuardianProfile::Nightly,
];

impl GuardianProfile {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Ownership => "c6-ownership",
            Self::OwnershipSeat => "c6-ownership-seat",
            Self::Nightly => "c6-ownership-nightly",
        }
    }

    /// Transitions explored beyond the empty start state. The graphs close
    /// well inside the horizon: a closed run is exhaustive, not truncated.
    pub const fn horizon(self) -> usize {
        match self {
            Self::Ownership | Self::OwnershipSeat => 32,
            Self::Nightly => 48,
        }
    }

    pub fn bounds(self) -> GuardianBounds {
        match self {
            Self::Ownership | Self::OwnershipSeat => GuardianBounds {
                wait: 2,
                ids: 2,
                sessions: 2,
            },
            Self::Nightly => GuardianBounds {
                wait: 4,
                ids: 2,
                sessions: 3,
            },
        }
    }

    pub fn initial(self, mutation: GuardianMutation) -> GuardianState {
        match self {
            Self::Ownership => GuardianState::new(self.bounds(), mutation),
            Self::OwnershipSeat | Self::Nightly => GuardianState::composed(self.bounds(), mutation),
        }
    }
}

pub fn guardian_profile(name: &str) -> Option<GuardianProfile> {
    GUARDIAN_PROFILES
        .into_iter()
        .find(|profile| profile.name() == name)
}

#[derive(Clone, Debug)]
pub struct GuardianExploration {
    pub bounds: GuardianBounds,
    pub horizon: usize,
    pub state_cap: usize,
    pub frontier_cap: usize,
    pub byte_cap: usize,
    pub states: usize,
    pub transitions: usize,
    pub max_frontier: usize,
    /// The deepest level that still produced a new state.
    pub depth_reached: usize,
    pub estimated_peak_bytes: usize,
    pub elapsed: Duration,
    pub failure: Option<(Violation, Vec<GuardianStep>)>,
    pub cap_reached: Option<CapKind>,
    /// The state graph closed inside the horizon, without a failure or a cap.
    pub complete: bool,
}

pub fn explore_guardian_profile(
    profile: GuardianProfile,
    mutation: GuardianMutation,
    limits: ExploreLimits,
) -> GuardianExploration {
    explore_guardian(
        profile.initial(mutation),
        profile.horizon(),
        profile.bounds(),
        limits,
    )
}

fn node_bytes(state: &GuardianState) -> usize {
    size_of::<GuardianState>()
        + state.seat.as_deref().map_or(0, state_heap_bytes)
        + size_of::<Node>()
        + 64
}

/// A reached state's parent link; the state itself is the `seen` key.
struct Node {
    parent: Option<usize>,
    via: Option<GuardianStep>,
}

fn trace(nodes: &[Node], mut index: usize) -> Vec<GuardianStep> {
    let mut steps = Vec::new();
    while let Some(parent) = nodes[index].parent {
        steps.push(nodes[index].via.clone().expect("a child has an edge"));
        index = parent;
    }
    steps.reverse();
    steps
}

pub fn explore_guardian(
    initial: GuardianState,
    horizon: usize,
    bounds: GuardianBounds,
    limits: ExploreLimits,
) -> GuardianExploration {
    let started = Instant::now();
    let mut result = GuardianExploration {
        bounds,
        horizon,
        state_cap: limits.state_cap,
        frontier_cap: limits.frontier_cap,
        byte_cap: limits.byte_cap,
        states: 0,
        transitions: 0,
        max_frontier: 0,
        depth_reached: 0,
        estimated_peak_bytes: 0,
        elapsed: Duration::ZERO,
        failure: None,
        cap_reached: None,
        complete: false,
    };
    let mut seen: HashMap<GuardianState, usize> = HashMap::new();
    let mut nodes: Vec<Node> = Vec::new();
    let mut frontier: VecDeque<(usize, usize, GuardianState)> = VecDeque::new();
    let mut bytes = 0usize;
    if let Err(violation) = initial.check() {
        result.failure = Some((violation, Vec::new()));
        result.elapsed = started.elapsed();
        return result;
    }
    bytes += node_bytes(&initial);
    seen.insert(initial.clone(), 0);
    nodes.push(Node {
        parent: None,
        via: None,
    });
    frontier.push_back((0, 0, initial));
    let mut truncated = false;
    while let Some((index, depth, state)) = frontier.pop_front() {
        result.max_frontier = result.max_frontier.max(frontier.len() + 1);
        if depth >= horizon {
            truncated = true;
            continue;
        }
        for step in alphabet(&state) {
            let mut next = state.clone();
            if next.apply(&step).is_err() {
                continue;
            }
            result.transitions += 1;
            if let Err(violation) = next.check() {
                let mut path = trace(&nodes, index);
                path.push(step);
                result.failure = Some((violation, path));
                result.states = seen.len();
                result.elapsed = started.elapsed();
                return result;
            }
            if seen.contains_key(&next) {
                continue;
            }
            if seen.len() >= limits.state_cap {
                result.cap_reached = Some(CapKind::States);
                break;
            }
            if frontier.len() >= limits.frontier_cap {
                result.cap_reached = Some(CapKind::Frontier);
                break;
            }
            bytes += node_bytes(&next);
            result.estimated_peak_bytes = result.estimated_peak_bytes.max(
                bytes
                    + seen.capacity() * size_of::<(GuardianState, usize)>()
                    + nodes.capacity() * size_of::<Node>()
                    + frontier.capacity() * size_of::<(usize, usize, GuardianState)>(),
            );
            if result.estimated_peak_bytes > limits.byte_cap {
                result.cap_reached = Some(CapKind::Bytes);
                break;
            }
            result.depth_reached = result.depth_reached.max(depth + 1);
            seen.insert(next.clone(), nodes.len());
            frontier.push_back((nodes.len(), depth + 1, next));
            nodes.push(Node {
                parent: Some(index),
                via: Some(step),
            });
        }
        if result.cap_reached.is_some() {
            break;
        }
    }
    result.states = seen.len();
    result.elapsed = started.elapsed();
    if let Some(cap) = result.cap_reached {
        result.failure = Some((
            Violation {
                name: "exploration-cap",
                detail: format!("{cap:?} cap reached; the exploration is incomplete"),
            },
            Vec::new(),
        ));
    } else {
        result.complete = !truncated;
        if truncated {
            result.failure = Some((
                Violation {
                    name: "exploration-horizon",
                    detail: format!(
                        "the horizon {horizon} cut the graph before it closed; raise it"
                    ),
                },
                Vec::new(),
            ));
        }
    }
    result
}

/// Replay `steps` from the profile's start state, checking every invariant.
pub fn run_guardian_trace(
    profile: GuardianProfile,
    mutation: GuardianMutation,
    steps: &[GuardianStep],
) -> Result<GuardianState, (usize, Violation)> {
    let mut state = profile.initial(mutation);
    for (index, step) in steps.iter().enumerate() {
        state.apply(step).map_err(|error| {
            (
                index,
                Violation {
                    name: "disabled-step",
                    detail: format!("{step:?}: {error}"),
                },
            )
        })?;
        state.check().map_err(|violation| (index, violation))?;
    }
    Ok(state)
}
