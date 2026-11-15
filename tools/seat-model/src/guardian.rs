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
    /// tool refit). Exhausted, it stalls and releases the slot. A replaced
    /// event releases it.
    Solver,
    /// A flee from a hostile actor: a solver whose act the next hostile actor
    /// inherits. The replacement keeps the pending act, and its hold, without
    /// a release tick, and starts its own escape budget.
    ///
    /// The escape budget runs on the content's clock, not on the host's
    /// passes: every fresh tick the server did not block, from the episode's
    /// first observation, whoever owns the event and whatever the toggle
    /// says. The engaged actor keeps its target and attacks a player who
    /// makes no outward progress from its spawn anchor, whoever moves the
    /// player (`Npc.ts:77-85,661-670`; the hostile lifetime is
    /// `macro_events.rs2:36`). A stun blocks the walk at the server
    /// (`movecheck.rs2:1-6`) and spends nothing. A spent budget makes the
    /// episode detect-only for any owner until the actor is gone.
    Evade,
    /// A trap square: the hold follows the detection while the host walks out;
    /// a bounded number of solver ticks gives the visit up.
    Trap,
    /// One operation whose outcome must arrive inside a content window. The
    /// lost tool's head lands within two content ticks of the loss, plus one
    /// delivery tick (`macro_event_lost_axe.rs2:18,25-27`;
    /// `macro_event_lost_pickaxe.rs2:20,25-29`; `projectile.rs2:31-35`). The
    /// window runs from the episode's first observation on every fresh tick,
    /// whoever owns the event and whatever the toggle says; the host that
    /// finds it closed lets go, also when it begins after the window closed.
    /// A pass that sees the toggle off re-arms a spent window once.
    Window,
}

impl Family {
    /// One operation in flight and a bounded wait for its outcome.
    fn is_solver(self) -> bool {
        matches!(self, Self::Solver | Self::Evade | Self::Window)
    }

    /// The family's wait is the content's clock of the episode, not the
    /// host's passes.
    fn has_clock(self) -> bool {
        matches!(self, Self::Evade | Self::Window)
    }
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
    /// A replaced hostile actor's successor does not inherit the pending
    /// act: the host releases the slot for a tick between two attackers.
    EvadeReleasesOnReplace,
    /// The Ent guard holds past its bounded lifetime.
    GuardNeverExpires,
    /// A replaced session keeps the previous session's active event.
    SessionKeepsActive,
    /// The toggle off still publishes `ours`.
    ToggleOffKeepsOurs,
    /// The host re-sends a pending operation when it regains the claim.
    DuplicateOpOnReclaim,
    /// A regained claim replays the rising-edge callback.
    SuccessorCallbackReplay,
    /// A replacement script runtime inherits the claim of the runtime before
    /// it (round-2 review probe: a compiled script that never claims the event
    /// keeps its predecessor's `Handle`).
    ClaimKeptAcrossRuntime,
}

impl GuardianMutation {
    /// Every mutation, with the invariant that must catch it.
    pub const ALL: [(Self, &'static str); 25] = [
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
        (Self::EvadeReleasesOnReplace, "evade-handoff"),
        (Self::GuardNeverExpires, "guard-unbounded"),
        (Self::SessionKeepsActive, "stale-session"),
        (Self::ToggleOffKeepsOurs, "toggle-off-publishes"),
        (Self::DuplicateOpOnReclaim, "duplicate-op"),
        (Self::SuccessorCallbackReplay, "duplicate-callback"),
        (Self::ClaimKeptAcrossRuntime, "stale-runtime"),
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
    /// The alphabet carries the Ent guard's steps and a reduced set of event
    /// families, so the state graph stays inside the profile caps.
    pub ent: bool,
    /// The alphabet carries the content-window family. The seat composition
    /// leaves it out to stay inside the profile caps: its publications are a
    /// solver's, and `c6-ownership` explores it.
    pub window: bool,
    /// Fresh ticks the Ent guard may hold (the content gives an Ent 60
    /// ticks; profiles bound it like a wait).
    pub ent_hold: u8,
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

/// One owner of one episode and the host work it has in flight. `guard` is the
/// Ent guard's hold (a bounded host guard that needs no detected event). The
/// real Guardian keeps the Ent's live latch in its active event and only the
/// expired-slot exclusion beside it; the model's [`EntLatch`] is that latch.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct Active {
    key: u8,
    session: u8,
    family: Family,
    owner: Owner,
    phase: Phase,
    guard: bool,
    /// With a Script owner: the script runtime that made the claim has ended
    /// (Stop, failure, a new Start, a session replacement). The real Guardian
    /// compares the runtime generation of the claim with the current one; this
    /// is that comparison. Pause and Resume keep the runtime.
    stale: bool,
}

/// The content clock of the observed episode of a clock family: fresh ticks
/// since its first observation that spend it (`spent`), and whether one of
/// them was a tick the server blocked (`blocked`, only under the
/// `FleeInterrupted` mutation). It is the Guardian's tracking of the world,
/// not host work. `rearmed`: the operator's toggle-off re-armed a spent
/// Window, and the next host begin opens a new window.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct EpisodeClock {
    id: u8,
    spent: u8,
    blocked: bool,
    rearmed: bool,
}

impl EpisodeClock {
    /// The spent ticks that could make progress.
    fn unblocked(self) -> u8 {
        self.spent.saturating_sub(u8::from(self.blocked))
    }
}

/// The Ent guard's latch: its slot is held for a bounded number of fresh
/// ticks, then excluded until it is observed gone.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum EntLatch {
    Absent,
    Held { ticks: u8 },
    Expired,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct Publication {
    pub claim_script: bool,
    pub handling: bool,
    pub acting: bool,
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
    /// The Ent guard holds the slot.
    pub guard: bool,
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
        acting: input.solver_in_flight,
        ours: input.toggle && input.detected && !input.stalled,
        hold: input.toggle
            && !input.claim_script
            && !input.stalled
            && (handling || input.solver_in_flight || input.guard || input.trapped),
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
    /// An Ent stands in the world. It is host work with no detected event.
    EntAppear,
    /// The Ent is gone (the tree reverted).
    EntRemove,
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
    /// An Ent stands in the world, and as the last fresh pass saw it.
    ent_world: bool,
    ent_observed: bool,
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
    clock: Option<EpisodeClock>,
    ent: EntLatch,
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
            ent_world: false,
            ent_observed: false,
            script: Script::Stopped,
            script_exit: None,
            wants: false,
            live: false,
            chat_open: false,
            next_id: 0,
            last_sig: None,
            knocks: 0,
            active: None,
            clock: None,
            ent: EntLatch::Absent,
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

    /// A Host-owned shell for the current episode with nothing in flight.
    fn shell(&self, owner: Owner) -> Active {
        Active {
            key: self.last_sig.unwrap_or(0),
            session: self.session,
            family: self.observed.map_or(Family::Solver, |event| event.family),
            owner,
            phase: Phase::Idle,
            guard: false,
            stale: false,
        }
    }

    fn set_owner(&mut self, owner: Owner) {
        if owner == Owner::Host && self.active.is_none() {
            return;
        }
        let previous = self.owner();
        match self.active.as_mut() {
            Some(active) => {
                active.owner = owner;
                // A claim made now is the current runtime's; a Host owner has
                // no runtime.
                active.stale = false;
            }
            None => self.active = Some(self.shell(owner)),
        }
        if previous == Owner::Host
            && owner == Owner::Script
            && self.mutation == GuardianMutation::SuccessorCallbackReplay
        {
            self.knocks = self.knocks.saturating_add(1);
        }
        self.settle();
    }

    /// The Ent guard asks for the hold this pass, or no longer does.
    fn set_guard(&mut self, guard: bool) {
        if guard && self.active.is_none() {
            self.active = Some(self.shell(Owner::Host));
        }
        if let Some(active) = self.active.as_mut() {
            active.guard = guard;
        }
        self.settle();
    }

    /// The Host owns the episode and has nothing in flight: no active event.
    fn settle(&mut self) {
        if self.active.is_some_and(|active| {
            active.owner == Owner::Host && active.phase == Phase::Idle && !active.guard
        }) {
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

    /// The script runtime ended or a new one started: a claim the old runtime
    /// made is no longer the current runtime's.
    fn end_runtime(&mut self) {
        if let Some(active) = self.active.as_mut()
            && active.owner == Owner::Script
        {
            active.stale = true;
        }
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
                // A new runtime: the previous runtime's claim is not its claim.
                self.end_runtime();
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
                if *exit != ScriptExit::Pause {
                    self.end_runtime();
                }
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
                self.end_runtime();
                self.chat_open = false;
                // The new session's snapshot has not seen the world yet, and
                // the new Guardian has tracked nothing: no signature, no
                // callback, no clock.
                self.observed = None;
                self.last_sig = None;
                self.knocks = 0;
                self.clock = None;
                self.ent_observed = false;
                self.ent = EntLatch::Absent;
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
            GuardianStep::EntAppear => {
                if self.ent_world {
                    return Err("an Ent already stands".into());
                }
                self.ent_world = true;
            }
            GuardianStep::EntRemove => {
                if !self.ent_world {
                    return Err("no Ent stands".into());
                }
                self.ent_world = false;
            }
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
            self.ent_observed = self.ent_world;
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
            // A claim belongs to the runtime that made it. A runtime that
            // ended or was replaced returns it within this tick, and its
            // successor is not called back (design 3.3).
            if self.claim_returns()
                && self.mutation != GuardianMutation::ClaimKeptAcrossRuntime
                && self
                    .active
                    .is_some_and(|active| active.owner == Owner::Script && active.stale)
            {
                self.set_owner(Owner::Host);
            }
            let sig = event.map(|event| event.id);
            if sig != self.last_sig {
                self.last_sig = sig;
                self.knocks = 0;
                if let Some(active) = self.active.as_mut() {
                    active.owner = Owner::Host;
                    active.stale = false;
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
            self.content_clock(blocked);
        }
        // The episode the host pass sees as stalled: a stall the content clock
        // made on this tick counts like one from an earlier tick.
        let stalled_for_host = self
            .active
            .is_some_and(|active| matches!(active.phase, Phase::Stalled { .. }));

        let guard = self.ent_pass(fresh);
        self.set_guard(guard);
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
                active.family.is_solver() && matches!(active.phase, Phase::Pending { .. })
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
                if now.family.is_solver() && self.emitted {
                    self.violation = Some((
                        "duplicate-op",
                        "the host sent a pending operation a second time".into(),
                    ));
                }
            }
            if (matches!(was.phase, Phase::Stalled { .. }) || stalled_for_host)
                && matches!(now.phase, Phase::Pending { .. })
            {
                self.violation = Some((
                    "released-actor-restart",
                    "a stalled episode started a second budget".into(),
                ));
            }
        }
        // The next hostile actor inherits a pending flee: it is not released
        // for a tick between two attackers.
        if fresh
            && let Some(was) = before
            && was.family == Family::Evade
            && was.owner == Owner::Host
            && was.session == self.session
            && matches!(was.phase, Phase::Pending { .. })
            && event.is_some_and(|event| event.family == Family::Evade && event.id != was.key)
            && !matches!(
                self.active.map(|active| active.phase),
                Some(Phase::Pending { .. })
            )
        {
            self.violation = Some((
                "evade-handoff",
                "a replaced hostile actor's successor did not inherit the pending flee".into(),
            ));
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
        // The Ent guard holds the slot: like a script claim or a toggle off,
        // the host's work in flight keeps its place and spends nothing.
        let guarded = self.active.is_some_and(|active| active.guard);
        if !enabled || guarded || self.owner() != Owner::Host {
            // A script owns the event, or the toggle is off: the host's work
            // in flight keeps its place and spends nothing.
            //
            // A pass that sees the toggle off clears a Window's stall latch:
            // the operator's toggle re-arms one new window (the real
            // Guardian's `tool_stalled` reset on a toggle-off pass).
            if !enabled
                && let Some(active) = self.active
                && active.family == Family::Window
                && matches!(active.phase, Phase::Stalled { .. })
                && event.is_some_and(|event| event.id == active.key)
            {
                if let Some(active) = self.active.as_mut() {
                    active.phase = Phase::Idle;
                }
                if let Some(clock) = self.clock.as_mut() {
                    clock.rearmed = true;
                }
                self.settle();
            }
            // Under a script claim, the mutation lets time count anyway.
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
                self.begin(event);
            }
            return;
        };
        let gone = event.map(|event| event.id) != Some(active.key);
        match active.phase {
            Phase::Pending { wait, unblocked } => {
                // Work for an event that is gone or replaced resolves; the next
                // pass starts the successor.
                if active.family != Family::Dialog && gone {
                    // A hostile actor's successor inherits the flee; any other
                    // replacement releases the work and the next pass starts
                    // its own.
                    if active.family == Family::Evade
                        && self.mutation != GuardianMutation::EvadeReleasesOnReplace
                        && let Some(event) = event.filter(|event| event.family == Family::Evade)
                    {
                        self.active = Some(Active {
                            key: event.id,
                            phase: Phase::Pending {
                                wait: 0,
                                unblocked: 0,
                            },
                            ..active
                        });
                        return;
                    }
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
                // A clock family's wait is the episode's content clock
                // (`content_clock`), not this pass: an Evade budget ends there,
                // and a Window ends at the first host pass that finds it
                // closed.
                if active.family == Family::Window {
                    if self.window_closed(active.key) {
                        self.finish(Phase::Stalled {
                            logged: true,
                            unblocked: self.bounds.wait,
                        });
                    }
                    return;
                }
                if active.family == Family::Evade {
                    return;
                }
                let counts = !blocked
                    || !active.family.is_solver()
                    || self.mutation == GuardianMutation::FleeInterrupted;
                let next_wait = if counts { wait + 1 } else { wait };
                let next_unblocked = if blocked && active.family.is_solver() {
                    unblocked
                } else {
                    unblocked + 1
                };
                if self.mutation == GuardianMutation::MissingOpenWaitsForever
                    && active.family.is_solver()
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
                    self.begin(event);
                }
            }
        }
    }

    /// One pass of the Ent guard: whether it holds the slot. Observing the Ent
    /// latches it for a bounded number of fresh ticks while the host may act,
    /// then excludes it until it is observed gone.
    fn ent_pass(&mut self, fresh: bool) -> bool {
        if !self.ent_observed {
            self.ent = EntLatch::Absent;
            return false;
        }
        let enabled = self.enabled && self.owner() == Owner::Host;
        match self.ent {
            EntLatch::Expired => false,
            EntLatch::Held { ticks } => {
                if fresh
                    && ticks >= self.bounds.ent_hold
                    && self.mutation != GuardianMutation::GuardNeverExpires
                {
                    self.ent = EntLatch::Expired;
                    return false;
                }
                if fresh {
                    self.ent = EntLatch::Held {
                        ticks: ticks.saturating_add(1),
                    };
                }
                enabled
            }
            EntLatch::Absent if enabled => {
                self.ent = EntLatch::Held {
                    ticks: u8::from(fresh),
                };
                true
            }
            EntLatch::Absent => false,
        }
    }

    fn finish(&mut self, phase: Phase) {
        if let Some(active) = self.active.as_mut() {
            active.phase = phase;
        }
        self.resolved_now = true;
        self.settle();
    }

    /// The host begins the work of `event`. A spent Evade budget is not
    /// begun again. A Window keeps the loss's first observation: a host that
    /// begins after it closed finds no head to wait for and lets go at once,
    /// unless the operator re-armed it.
    fn begin(&mut self, event: Event) {
        if self.evade_exhausted() {
            return;
        }
        if event.family == Family::Window
            && let Some(clock) = self.clock.as_mut().filter(|clock| clock.id == event.id)
            && clock.rearmed
        {
            clock.spent = 0;
            clock.rearmed = false;
        }
        self.start(event);
        if event.family == Family::Window && self.window_closed(event.id) {
            self.finish(Phase::Stalled {
                logged: true,
                unblocked: self.bounds.wait,
            });
        }
    }

    /// The content clock of the observed episode of a clock family. It runs on
    /// every fresh tick after the first observation, whoever owns the event
    /// and whatever the toggle says. A spent Evade budget releases the
    /// episode for any owner.
    fn content_clock(&mut self, blocked: bool) {
        let Some(event) = self.observed.filter(|event| event.family.has_clock()) else {
            self.clock = None;
            return;
        };
        let limit = self.bounds.wait;
        let clock = match self.clock {
            Some(clock) if clock.id == event.id => {
                let window = event.family == Family::Window;
                let counts =
                    !blocked || window || self.mutation == GuardianMutation::FleeInterrupted;
                EpisodeClock {
                    id: event.id,
                    spent: if counts {
                        clock.spent.saturating_add(1).min(limit)
                    } else {
                        clock.spent
                    },
                    blocked: clock.blocked || (counts && blocked && !window),
                    rearmed: clock.rearmed,
                }
            }
            // The first observation of the episode.
            _ => EpisodeClock {
                id: event.id,
                spent: 0,
                blocked: false,
                rearmed: false,
            },
        };
        self.clock = Some(clock);
        if self.evade_exhausted() {
            self.exhaust(event, clock.unblocked());
        }
    }

    /// The observed Evade episode spent its budget.
    fn evade_exhausted(&self) -> bool {
        self.observed.is_some_and(|event| {
            event.family == Family::Evade
                && self
                    .clock
                    .is_some_and(|clock| clock.id == event.id && clock.spent >= self.bounds.wait)
        })
    }

    /// The Window of episode `id` has closed.
    fn window_closed(&self, id: u8) -> bool {
        self.clock
            .is_some_and(|clock| clock.id == id && clock.spent >= self.bounds.wait)
    }

    /// A spent Evade budget: the episode is released with a logged failure,
    /// whoever owns it. Another episode's work in flight (a dialog handle)
    /// keeps its place; the spent episode publishes through its clock.
    fn exhaust(&mut self, event: Event, unblocked: u8) {
        let stalled = Phase::Stalled {
            logged: true,
            unblocked,
        };
        match self.active {
            Some(active) if active.key == event.id => {
                if matches!(active.phase, Phase::Stalled { .. }) {
                    return;
                }
                if matches!(active.phase, Phase::Pending { .. }) {
                    self.resolved_now = true;
                }
                if let Some(active) = self.active.as_mut() {
                    active.phase = stalled;
                }
            }
            None => {
                let mut shell = self.shell(Owner::Host);
                shell.key = event.id;
                shell.family = event.family;
                shell.phase = stalled;
                self.active = Some(shell);
            }
            Some(_) => {}
        }
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
            guard: self.active.is_some_and(|active| active.guard),
            stale: owner == Owner::Script && self.active.is_some_and(|active| active.stale),
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
            // A spent Evade budget is detected but no longer owned work.
            detected: event.is_some() && !self.evade_exhausted(),
            claim_script: active.is_some_and(|active| active.owner == Owner::Script),
            dialog_in_flight: active.is_some_and(|active| {
                active.family == Family::Dialog && matches!(active.phase, Phase::Pending { .. })
            }),
            solver_in_flight: active.is_some_and(|active| {
                active.family != Family::Dialog && matches!(active.phase, Phase::Pending { .. })
            }),
            guard: active.is_some_and(|active| active.guard),
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
                if unblocked < self.bounds.wait && active.family.is_solver() {
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
        if let EntLatch::Held { ticks } = self.ent
            && ticks > self.bounds.ent_hold
        {
            return fail(
                "guard-unbounded",
                format!(
                    "the Ent guard held {ticks} fresh ticks (bound {})",
                    self.bounds.ent_hold
                ),
            );
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
            if self.out.claim_script
                && self
                    .active
                    .is_some_and(|active| active.owner == Owner::Script && active.stale)
            {
                return fail(
                    "stale-runtime",
                    "a script runtime that ended or was replaced still owns the event".into(),
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
                        && (active.guard
                            || (active.key == event.id && !matches!(active.phase, Phase::Idle))
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
            "event={:?} ent={:?}/{:?} script={:?} enabled={} session={} active={:?} clock={:?} out={:?} knocks={}",
            self.event,
            self.ent_world,
            self.ent,
            self.script,
            self.enabled,
            self.session,
            self.active,
            self.clock,
            self.out,
            self.knocks
        )
    }
}

fn alphabet(state: &GuardianState) -> Vec<GuardianStep> {
    let mut out = Vec::new();
    let families: &[(Family, bool)] = if state.bounds.ent {
        &[(Family::Solver, false), (Family::Trap, false)]
    } else {
        &[
            (Family::Dialog, false),
            (Family::Solver, false),
            (Family::Solver, true),
            (Family::Evade, false),
            (Family::Evade, true),
            (Family::Trap, false),
        ]
    };
    let window: &[(Family, bool)] = if state.bounds.window && !state.bounds.ent {
        &[(Family::Window, false)]
    } else {
        &[]
    };
    for &(family, forced_host) in families.iter().chain(window) {
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
    if state.bounds.ent {
        out.extend([GuardianStep::EntAppear, GuardianStep::EntRemove]);
    }
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
    /// Ownership with the Ent guard, over a reduced set of event families.
    OwnershipEnt,
    /// A deeper bound for the nightly Linux lane (16 GiB scope).
    Nightly,
}

pub const GUARDIAN_PROFILES: [GuardianProfile; 4] = [
    GuardianProfile::Ownership,
    GuardianProfile::OwnershipSeat,
    GuardianProfile::OwnershipEnt,
    GuardianProfile::Nightly,
];

impl GuardianProfile {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Ownership => "c6-ownership",
            Self::OwnershipSeat => "c6-ownership-seat",
            Self::OwnershipEnt => "c6-ownership-ent",
            Self::Nightly => "c6-ownership-nightly",
        }
    }

    /// Transitions explored beyond the empty start state. The graphs close
    /// well inside the horizon: a closed run is exhaustive, not truncated.
    pub const fn horizon(self) -> usize {
        match self {
            Self::Ownership | Self::OwnershipSeat | Self::OwnershipEnt => 32,
            Self::Nightly => 48,
        }
    }

    pub fn bounds(self) -> GuardianBounds {
        match self {
            Self::Ownership => GuardianBounds {
                wait: 2,
                ids: 2,
                sessions: 2,
                ent: false,
                window: true,
                ent_hold: 2,
            },
            Self::OwnershipSeat => GuardianBounds {
                wait: 2,
                ids: 2,
                sessions: 2,
                ent: false,
                window: false,
                ent_hold: 2,
            },
            Self::OwnershipEnt => GuardianBounds {
                wait: 2,
                ids: 2,
                sessions: 2,
                ent: true,
                window: false,
                ent_hold: 2,
            },
            Self::Nightly => GuardianBounds {
                wait: 4,
                ids: 2,
                sessions: 3,
                ent: false,
                window: true,
                ent_hold: 4,
            },
        }
    }

    pub fn initial(self, mutation: GuardianMutation) -> GuardianState {
        match self {
            Self::Ownership | Self::OwnershipEnt => GuardianState::new(self.bounds(), mutation),
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
