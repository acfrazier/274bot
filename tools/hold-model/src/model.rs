//! The auto-run hold: state, transitions and frozen invariants.
//!
//! One slot, one script, at most one wait lease. The lease kinds are the
//! native `Dialogue` modal wait, the native quest-journal quiet read and the
//! compatibility (JS) quest-journal read. Time is abstract: the owner's
//! active clock and the wall clock are small counters, game ticks are a
//! counter, and every bound is a relation on those counters (see README.md,
//! "Abstractions").
//!
//! The transitions follow the product's frame order (`crates/host/src/lib.rs`
//! `client_frame_observed`): the drain, the host's session-boundary release,
//! the observe hook (session reset, clock, the owner's poll), the host's
//! sample of the lease, the host's wait policy and then the press decision.

use crate::explorer::{Machine, Violation};
use std::fmt::Write as _;

/// `host::SCRIPT_WAIT_MARGIN_TICKS`: the invariants are stated with the real
/// value, even for a mutation that changes the margin of the transitions.
pub const MARGIN_TICKS: u8 = 1;
/// The fixed terminal bound of mutation M3, in model ticks. The product's
/// earlier design used 25 ticks, shorter than the longest authored wait (30).
/// The model's waits and ticks are small, so this is a bound that is shorter
/// than a legal wait here too.
pub const M3_FIXED_TERMINAL_TICKS: u8 = 3;

/// Permanent seeded regressions. Production exploration uses `None`. The
/// `Check*` mutations are the model builder's own checks that each frozen
/// invariant can fail; they are not earlier designs of the product.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Mutation {
    #[default]
    None,
    /// The host ends a hold at the deadline, with no margin.
    CheckMarginZero,
    /// The host ends a hold two ticks after the deadline.
    CheckMarginTwo,
    /// Stop leaves the native lease alive.
    CheckStopKeepsLease,
    /// The compat click does not take a fresh hold bound.
    CheckCompatClickNoRenew,
    /// A new send keeps the key of the lease it replaces. The head host cannot
    /// tell such a send from a renewal of the same key (both move the deadline
    /// later), so this check runs on the earlier host
    /// (`F1RenewalKeepsRelease`): it shows that S4 sees a released state
    /// carried to a new transaction.
    CheckSendKeepsKey,
    /// The host keeps its hold when the sample returns no wait.
    CheckStaleHold,
    /// The journal quiet lease has no wall bound.
    CheckQuietNoWall,
    /// M1 (round 2): no leases. The host ends its hold when a modal opens, or
    /// 2 observed ticks after the owner's click, whichever is first.
    M1HoldTwoTicks,
    /// M2 (round 3): as M1 with 3 ticks.
    M2HoldThreeTicks,
    /// M3 (round 4): the host's terminal release is a fixed number of ticks
    /// from the first sample of the lease key, whatever the owner's deadline.
    M3FixedTerminal,
    /// M4 (round 4): every compat journal lease has one constant key. The host
    /// keeps its start tick and released flag while the key does not change.
    M4CompatConstantKey,
    /// M5 (round 5): an active-clock deadline is converted once, when it is
    /// installed, to a fixed wall deadline and never recomputed.
    M5ActiveToWall,
    /// M6 (round 5): the compat hold has one wall bound from the begin; the
    /// click does not renew it.
    M6CompatNoRenew,
    /// F1 (the defect of `HOLD-MODEL-AB`, fixed in `host`): the host's hold of
    /// a key that it ended as the terminal failure stays ended when the owner
    /// renews the same key to a later deadline. This is the host as it was at
    /// `0a3e700f8`; the head model re-arms the hold at such a renewal.
    F1RenewalKeepsRelease,
}

impl Mutation {
    pub const ALL: [Mutation; 15] = [
        Mutation::None,
        Mutation::CheckMarginZero,
        Mutation::CheckMarginTwo,
        Mutation::CheckStopKeepsLease,
        Mutation::CheckCompatClickNoRenew,
        Mutation::CheckSendKeepsKey,
        Mutation::CheckStaleHold,
        Mutation::CheckQuietNoWall,
        Mutation::M1HoldTwoTicks,
        Mutation::M2HoldThreeTicks,
        Mutation::M3FixedTerminal,
        Mutation::M4CompatConstantKey,
        Mutation::M5ActiveToWall,
        Mutation::M6CompatNoRenew,
        Mutation::F1RenewalKeepsRelease,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Mutation::None => "none",
            Mutation::CheckMarginZero => "check-margin-zero",
            Mutation::CheckMarginTwo => "check-margin-two",
            Mutation::CheckStopKeepsLease => "check-stop-keeps-lease",
            Mutation::CheckCompatClickNoRenew => "check-compat-click-no-renew",
            Mutation::CheckSendKeepsKey => "check-send-keeps-key",
            Mutation::CheckStaleHold => "check-stale-hold",
            Mutation::CheckQuietNoWall => "check-quiet-no-wall",
            Mutation::M1HoldTwoTicks => "m1-hold-two-ticks",
            Mutation::M2HoldThreeTicks => "m2-hold-three-ticks",
            Mutation::M3FixedTerminal => "m3-fixed-terminal",
            Mutation::M4CompatConstantKey => "m4-compat-constant-key",
            Mutation::M5ActiveToWall => "m5-active-to-wall",
            Mutation::M6CompatNoRenew => "m6-compat-no-renew",
            Mutation::F1RenewalKeepsRelease => "f1-renewal-keeps-release",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Kind {
    Modal,
    Quiet,
    Compat,
}

/// Why a lease ended, or why the host stopped holding.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Cause {
    /// The owner saw its page, its acknowledgement, or finished its gap.
    Page,
    /// The owner's own bound expired and the owner ended the wait.
    Bound,
    /// The owner dropped the lease (cancelled its step).
    Dropped,
    /// Stop.
    Stop,
    /// Relog, disconnect or reconnect.
    Session,
    /// The runner cancelled its step after a death.
    Death,
    /// The lease's own wall bound passed.
    Wall,
    /// The host released after the deadline plus the margin.
    HostTerminal,
}

/// Events that the environment may offer; a profile enables a subset.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Env {
    pub pause: bool,
    pub stop: bool,
    /// A relog in one drain (a session change).
    pub session: bool,
    /// The connection drops and the session connects again.
    pub disconnect: bool,
    pub hold: bool,
    pub cleanup: bool,
    pub death: bool,
    pub run_off: bool,
    pub page_change: bool,
}

impl Env {
    pub const NONE: Env = Env {
        pause: false,
        stop: false,
        session: false,
        disconnect: false,
        hold: false,
        cleanup: false,
        death: false,
        run_off: false,
        page_change: false,
    };
    pub const ALL: Env = Env {
        pause: true,
        stop: true,
        session: true,
        disconnect: true,
        hold: true,
        cleanup: true,
        death: true,
        run_off: true,
        page_change: true,
    };
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Bounds {
    /// Longest trace, in actions.
    pub horizon: usize,
    /// Observed game ticks; further tick edges are not offered.
    pub max_tick: u8,
    /// Wall units; the active clock never exceeds it.
    pub max_wall: u8,
    /// Lease, action and token numbers.
    pub max_ids: u8,
    /// Page changes (a modal opening, closing or being replaced).
    pub max_pages: u8,
    /// Runtime starts and work generations.
    pub max_epochs: u8,
    /// Dialogue page wait, in active units (the product's 8 s).
    pub page_active: u8,
    /// Dialogue acknowledgement wait, in active units (the product's 3 s).
    pub ack_active: u8,
    /// Journal window, in observed ticks (the product's 5).
    pub quiet_window: u8,
    /// Journal whole-read bound, in active units (the product's 8 s).
    pub quiet_overall: u8,
    /// Journal quiet lease wall bound (the product's 10 s).
    pub quiet_wall: u8,
    /// Compat hold bound and paint cap, in wall units (the product's 10 s).
    pub compat_hold: u8,
    /// Compat acquisition and close window, in active units (the product's 3 s).
    pub compat_acquire: u8,
    /// Longest end gap, in ticks (the product allows up to 30).
    pub max_gap: u8,
    /// Gap re-arms one dialogue may take.
    pub rearms: u8,
    pub modal: bool,
    pub quiet: bool,
    pub compat: bool,
    pub env: Env,
    /// Offer drift only in frames where the owner polls. The conformance
    /// test uses this: the product's poll is where a test can pass time.
    pub drift_polls_only: bool,
    /// Offer a begin in a frame whose session boundary revokes the owner's
    /// lease: the product's hook resets the session before the poll, so an
    /// owner can begin a new read in that same poll. The frozen model offers
    /// the acts against the lease of the previous frame; this flag removes
    /// that abstraction. Added in phase 2 (default off, the head runs are
    /// unchanged).
    pub same_frame_begin: bool,
}

impl Bounds {
    /// The default bounds: every kind and every event, relation-scale numbers.
    pub const fn standard() -> Self {
        Bounds {
            horizon: 12,
            max_tick: 4,
            max_wall: 5,
            max_ids: 4,
            max_pages: 4,
            max_epochs: 2,
            page_active: 3,
            ack_active: 1,
            quiet_window: 2,
            quiet_overall: 3,
            quiet_wall: 4,
            compat_hold: 4,
            compat_acquire: 1,
            max_gap: 2,
            rearms: 1,
            modal: true,
            quiet: true,
            compat: true,
            env: Env::ALL,
            drift_polls_only: false,
            same_frame_begin: false,
        }
    }
}

/// A lease key as the host sees it. The model keeps the three namespaces
/// apart, as the product's tag bits do.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Key {
    Modal(u8),
    Quiet(u8),
    Compat { token: u8, generation: u8 },
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LedgerBound {
    Tick(u8),
    Active(u8),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ModalPhase {
    /// Waits for the first page of a Talk.
    Page { until: u8 },
    /// Waits for the acknowledgement of a Continue or an answer.
    Ack { base: u8, until: u8 },
    /// The quiet end gap after the chat closed.
    Gap { due: u8 },
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Modal {
    pub id: u8,
    pub txn: u8,
    pub phase: ModalPhase,
    /// What the ledger reports for this lease.
    pub ledger: LedgerBound,
    /// `gap_rearms_left`.
    pub rearms: u8,
    pub rearm_tick: Option<u8>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum QuietPhase {
    Click,
    Acquire,
    Close,
    Closing,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Quiet {
    pub action: u8,
    pub txn: u8,
    /// Wall unit of the begin.
    pub since: u8,
    /// The ledger still holds the lease (the wall bound has not released it).
    pub held: bool,
    /// The owner's window, in ticks.
    pub window: u8,
    /// The window the ledger reports.
    pub ledger_tick: u8,
    /// The owner's whole-read bound on the active clock.
    pub overall: u8,
    pub phase: QuietPhase,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CompatPhase {
    Ready,
    Awaiting,
    Acquired { page: u8 },
    Closing { page: u8 },
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Compat {
    pub token: u8,
    pub generation: u8,
    pub txn: u8,
    pub since: u8,
    pub hold_until: u8,
    /// The paint-hide cap: from the begin, never renewed.
    pub paint_until: u8,
    pub phase: CompatPhase,
    /// The owner's acquisition or close window on the active clock.
    pub acquire: u8,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Lease {
    None,
    Modal(Modal),
    Quiet(Quiet),
    Compat(Compat),
}

impl Lease {
    fn txn(self) -> Option<u8> {
        match self {
            Lease::None => None,
            Lease::Modal(m) => Some(m.txn),
            Lease::Quiet(q) => Some(q.txn),
            Lease::Compat(c) => Some(c.txn),
        }
    }

    pub fn kind(self) -> Option<Kind> {
        match self {
            Lease::None => None,
            Lease::Modal(_) => Some(Kind::Modal),
            Lease::Quiet(_) => Some(Kind::Quiet),
            Lease::Compat(_) => Some(Kind::Compat),
        }
    }
}

/// `host::ScriptWaitHold`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct HostHold {
    pub key: Key,
    pub released: bool,
    /// A session boundary ended the hold (`released` is set too). Only a new
    /// key holds again; the terminal release is the one that a renewal undoes.
    pub boundary: bool,
    /// The deadline of the latest sample of this key (`ScriptWaitHold::
    /// deadline_tick`). A later one after the terminal release is a renewal.
    /// The earlier host (`F1RenewalKeepsRelease`) did not keep it: it stays 0.
    pub deadline: u8,
    /// The tick at which this hold began: the first sample of its key, or the
    /// renewal that re-armed it. Only the earlier designs M1 to M4 read it.
    pub start: u8,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct State {
    // The world.
    pub tick: u8,
    pub wall: u8,
    pub modal: bool,
    /// Counts page changes; an acknowledgement is a page that differs.
    pub page: u8,
    pub up: bool,
    pub session_pending: bool,
    pub hp_zero: bool,
    pub death_msg: bool,
    /// Guardian or not-ready hold: the clock stops and no owner polls.
    pub hold: bool,
    /// Prayer cleanup or similar: no owner polls, the clock runs.
    pub cleanup: bool,
    // The slot's script.
    /// A runtime exists (Starting, Running or Paused); Stop clears it.
    pub runtime_live: bool,
    /// The runtime generation; each Start takes the next one.
    pub rgen: u8,
    pub want_run: bool,
    pub active: u8,
    pub eligible: bool,
    pub next_id: u8,
    /// The compat work generation; each session boundary takes the next one.
    pub work_gen: u8,
    pub lease: Lease,
    // The host.
    pub host_generation: Option<u8>,
    pub hold_rec: Option<HostHold>,
    pub run_on: bool,
    // Ghosts: written by the transitions, read only by the invariants.
    pub txn: u8,
    /// The tick at which the live lease's owner deadline first passed.
    pub due: Option<u8>,
    /// The transaction that the host's terminal release ended.
    pub released_txn: Option<u8>,
    pub stopped_since_frame: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Begin {
    Modal,
    Quiet,
    Compat,
}

/// What the owner does at a poll.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum OwnerAct {
    /// The owner does not poll in this frame.
    NoPoll,
    /// The owner polls and changes nothing (budget spent, or still pending).
    Stall,
    Begin(Begin),
    /// The owner's poll: it ends the wait when it sees its page or when its
    /// own bound expired, and moves a journal or compat read one phase.
    Poll,
    /// A dialogue sends again: a new acknowledgement lease.
    Send,
    /// A dialogue's acknowledged page closed: the same lease becomes the gap.
    ToGap(u8),
    /// A dialogue re-arms its gap: a new lease.
    Rearm(u8),
    /// The owner cancels its step.
    Drop,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FrameInput {
    /// The drain carries a PLAYER_INFO (a game tick).
    pub tick: bool,
    /// Packet families are dirty: an evidence wake polls without a tick.
    pub wake: bool,
    /// Time passes between the owner's poll and the host's sample.
    pub drift: bool,
    pub act: OwnerAct,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Action {
    /// One unit of time passes.
    Wait,
    Frame(FrameInput),
    Pause,
    Resume,
    Stop,
    Start,
    Disconnect,
    Connect,
    Relog,
    ModalOpen,
    ModalClose,
    ModalNext,
    HoldOn,
    HoldOff,
    CleanupOn,
    CleanupOff,
    Die,
    DeathMsg,
    RunOff,
}

/// What a transition did, for traces and goals; never part of the state.
#[derive(Clone, Debug, Default)]
pub struct Info {
    pub releases: Vec<(Kind, Cause)>,
    pub pressed: bool,
    pub deferred: bool,
    pub polled: bool,
    pub boundary: bool,
    pub expired: Option<(Key, u8, u8)>,
    pub wait: Option<(Key, u8)>,
    /// The owner sent a verb this frame (a Talk, Continue, answer, click or close).
    pub emitted: bool,
}

struct Wait {
    key: Key,
    deadline: u8,
}

pub struct Model {
    pub bounds: Bounds,
    pub mutation: Mutation,
}

impl Model {
    pub fn new(bounds: Bounds, mutation: Mutation) -> Self {
        Self { bounds, mutation }
    }

    pub fn initial_state(&self) -> State {
        State {
            tick: 0,
            wall: 0,
            modal: false,
            page: 0,
            up: true,
            session_pending: false,
            hp_zero: false,
            death_msg: false,
            hold: false,
            cleanup: false,
            runtime_live: true,
            rgen: 1,
            want_run: true,
            active: 0,
            eligible: false,
            next_id: 1,
            work_gen: 1,
            lease: Lease::None,
            host_generation: None,
            hold_rec: None,
            run_on: false,
            txn: 0,
            due: None,
            released_txn: None,
            stopped_since_frame: false,
        }
    }

    fn running(s: &State) -> bool {
        s.runtime_live && s.want_run && s.up
    }

    fn live(s: &State) -> bool {
        match s.lease {
            Lease::None => false,
            Lease::Modal(_) => true,
            Lease::Quiet(q) => q.held,
            Lease::Compat(c) => s.wall < c.hold_until,
        }
    }

    /// Whether the owner's own deadline, in the owner's own clock, has passed.
    fn passed(s: &State) -> bool {
        match s.lease {
            Lease::None => false,
            Lease::Modal(m) => match m.phase {
                ModalPhase::Page { until } | ModalPhase::Ack { until, .. } => s.active >= until,
                ModalPhase::Gap { due } => s.tick >= due,
            },
            Lease::Quiet(q) => s.tick >= q.window,
            Lease::Compat(c) => s.wall >= c.hold_until,
        }
    }

    fn key_of(lease: Lease) -> Option<Key> {
        match lease {
            Lease::None => None,
            Lease::Modal(m) => Some(Key::Modal(m.id)),
            Lease::Quiet(q) => Some(Key::Quiet(q.action)),
            Lease::Compat(c) => Some(Key::Compat {
                token: c.token,
                generation: c.generation,
            }),
        }
    }

    fn end_lease(s: &mut State, info: &mut Info, cause: Cause) {
        if let Some(kind) = s.lease.kind() {
            info.releases.push((kind, cause));
        }
        s.lease = Lease::None;
        s.due = None;
    }

    fn fresh_txn(s: &mut State) -> u8 {
        s.txn = s.txn.saturating_add(1);
        s.txn
    }

    fn fresh_id(s: &mut State) -> u8 {
        let id = s.next_id;
        s.next_id = s.next_id.saturating_add(1);
        id
    }

    /// A new connection starts with no modal and a living player.
    fn client_reset(s: &mut State) {
        if s.modal {
            s.modal = false;
            s.page += 1;
        }
        s.hp_zero = false;
        s.death_msg = false;
    }

    // ---- the owner's side ----

    fn begin(&self, s: &mut State, kind: Begin, info: &mut Info) {
        let b = &self.bounds;
        match kind {
            Begin::Modal => {
                info.emitted = true;
                let id = Self::fresh_id(s);
                let until = s.active + b.page_active;
                s.lease = Lease::Modal(Modal {
                    id,
                    txn: Self::fresh_txn(s),
                    phase: ModalPhase::Page { until },
                    ledger: Self::ledger_for(self.mutation, s.wall, s.active, until),
                    rearms: b.rearms,
                    rearm_tick: None,
                });
            }
            Begin::Quiet => {
                let action = Self::fresh_id(s);
                let window = s.tick + b.quiet_window;
                s.lease = Lease::Quiet(Quiet {
                    action,
                    txn: Self::fresh_txn(s),
                    since: s.wall,
                    held: true,
                    window,
                    ledger_tick: window,
                    overall: s.active + b.quiet_overall,
                    phase: QuietPhase::Click,
                });
            }
            Begin::Compat => {
                let token = Self::fresh_id(s);
                s.lease = Lease::Compat(Compat {
                    token,
                    generation: s.work_gen,
                    txn: Self::fresh_txn(s),
                    since: s.wall,
                    hold_until: s.wall + b.compat_hold,
                    paint_until: s.wall + b.compat_hold,
                    phase: CompatPhase::Ready,
                    acquire: 0,
                });
            }
        }
    }

    /// The owner's poll of its own lease.
    fn poll(&self, s: &mut State, info: &mut Info) {
        match s.lease {
            Lease::None => {}
            Lease::Modal(m) => {
                let ended = match m.phase {
                    ModalPhase::Page { until } => {
                        if s.modal {
                            Some(Cause::Page)
                        } else if s.active >= until {
                            Some(Cause::Bound)
                        } else {
                            None
                        }
                    }
                    ModalPhase::Ack { base, until } => {
                        if s.page != base {
                            Some(Cause::Page)
                        } else if s.active >= until {
                            Some(Cause::Bound)
                        } else {
                            None
                        }
                    }
                    ModalPhase::Gap { due } => {
                        if s.modal {
                            Some(Cause::Page)
                        } else if s.tick >= due {
                            Some(Cause::Bound)
                        } else {
                            None
                        }
                    }
                };
                if let Some(cause) = ended {
                    Self::end_lease(s, info, cause);
                }
            }
            Lease::Quiet(mut q) => {
                if !q.held {
                    Self::end_lease(s, info, Cause::Wall);
                } else if s.active >= q.overall || s.tick >= q.window {
                    Self::end_lease(s, info, Cause::Bound);
                } else {
                    let renew = |q: &mut Quiet, tick: u8, window: u8| {
                        q.window = tick + window;
                        q.ledger_tick = q.window;
                    };
                    let window = self.bounds.quiet_window;
                    match q.phase {
                        QuietPhase::Click => {
                            if s.modal {
                                Self::end_lease(s, info, Cause::Bound);
                                return;
                            }
                            q.phase = QuietPhase::Acquire;
                            info.emitted = true;
                            renew(&mut q, s.tick, window);
                        }
                        QuietPhase::Acquire => {
                            if s.modal {
                                q.phase = QuietPhase::Close;
                                renew(&mut q, s.tick, window);
                            }
                        }
                        QuietPhase::Close => {
                            if s.modal {
                                q.phase = QuietPhase::Closing;
                                info.emitted = true;
                                renew(&mut q, s.tick, window);
                            } else {
                                Self::end_lease(s, info, Cause::Bound);
                                return;
                            }
                        }
                        QuietPhase::Closing => {
                            if !s.modal {
                                Self::end_lease(s, info, Cause::Page);
                                return;
                            }
                        }
                    }
                    s.lease = Lease::Quiet(q);
                }
            }
            Lease::Compat(mut c) => {
                let acquire = self.bounds.compat_acquire;
                match c.phase {
                    CompatPhase::Ready => {
                        // The click takes a fresh hold bound; the paint cap stays.
                        if !matches!(
                            self.mutation,
                            Mutation::CheckCompatClickNoRenew | Mutation::M6CompatNoRenew
                        ) {
                            c.hold_until = s.wall + self.bounds.compat_hold;
                        }
                        info.emitted = true;
                        c.phase = CompatPhase::Awaiting;
                        c.acquire = s.active + acquire;
                    }
                    _ if s.wall >= c.hold_until => {
                        Self::end_lease(s, info, Cause::Bound);
                        return;
                    }
                    CompatPhase::Awaiting => {
                        if s.modal {
                            c.phase = CompatPhase::Acquired { page: s.page };
                        } else if s.active >= c.acquire {
                            Self::end_lease(s, info, Cause::Bound);
                            return;
                        }
                    }
                    CompatPhase::Acquired { page } => {
                        if s.modal && s.page == page {
                            c.phase = CompatPhase::Closing { page };
                            info.emitted = true;
                            c.acquire = s.active + acquire;
                        } else {
                            Self::end_lease(s, info, Cause::Bound);
                            return;
                        }
                    }
                    CompatPhase::Closing { page } => {
                        if !s.modal {
                            Self::end_lease(s, info, Cause::Page);
                            return;
                        } else if s.page != page || s.active >= c.acquire {
                            Self::end_lease(s, info, Cause::Bound);
                            return;
                        }
                    }
                }
                s.lease = Lease::Compat(c);
            }
        }
    }

    fn send(&self, s: &mut State, info: &mut Info) {
        let Lease::Modal(old) = s.lease else {
            return;
        };
        info.emitted = true;
        let id = if self.mutation == Mutation::CheckSendKeepsKey {
            old.id
        } else {
            Self::fresh_id(s)
        };
        let until = s.active + self.bounds.ack_active;
        s.lease = Lease::Modal(Modal {
            id,
            txn: Self::fresh_txn(s),
            phase: ModalPhase::Ack {
                base: s.page,
                until,
            },
            ledger: Self::ledger_for(self.mutation, s.wall, s.active, until),
            rearms: old.rearms,
            rearm_tick: old.rearm_tick,
        });
        s.due = None;
    }

    fn to_gap(s: &mut State, gap: u8) {
        let Lease::Modal(mut m) = s.lease else {
            return;
        };
        let due = s.tick + gap;
        m.phase = ModalPhase::Gap { due };
        // The same lease, renewed: `renew_modal_wait`.
        m.ledger = LedgerBound::Tick(due);
        m.txn = Self::fresh_txn(s);
        s.lease = Lease::Modal(m);
        s.due = None;
    }

    fn rearm(&self, s: &mut State, gap: u8) {
        let Lease::Modal(old) = s.lease else {
            return;
        };
        let id = Self::fresh_id(s);
        let due = s.tick + gap;
        s.lease = Lease::Modal(Modal {
            id,
            txn: Self::fresh_txn(s),
            phase: ModalPhase::Gap { due },
            ledger: LedgerBound::Tick(due),
            rearms: old.rearms - 1,
            rearm_tick: Some(s.tick),
        });
        s.due = None;
    }

    fn owner_acts(&self, s: &State, tick: u8, out: &mut Vec<OwnerAct>) {
        let b = &self.bounds;
        if s.death_msg {
            out.push(OwnerAct::Poll);
            return;
        }
        out.push(OwnerAct::Stall);
        if s.hp_zero {
            return;
        }
        let lease = if b.same_frame_begin && s.session_pending {
            Lease::None
        } else {
            s.lease
        };
        match lease {
            Lease::None => {
                if s.next_id < b.max_ids {
                    if b.modal && !s.modal {
                        out.push(OwnerAct::Begin(Begin::Modal));
                    }
                    if b.quiet && !s.modal {
                        out.push(OwnerAct::Begin(Begin::Quiet));
                    }
                    if b.compat && !s.modal {
                        out.push(OwnerAct::Begin(Begin::Compat));
                    }
                }
            }
            Lease::Modal(m) => {
                out.push(OwnerAct::Poll);
                out.push(OwnerAct::Drop);
                let sees_page = match m.phase {
                    ModalPhase::Page { .. } | ModalPhase::Gap { .. } => s.modal,
                    ModalPhase::Ack { base, .. } => s.page != base && s.modal,
                };
                if sees_page && s.next_id < b.max_ids {
                    out.push(OwnerAct::Send);
                }
                if let ModalPhase::Ack { base, .. } = m.phase
                    && s.page != base
                    && !s.modal
                {
                    for gap in 1..=b.max_gap {
                        out.push(OwnerAct::ToGap(gap));
                    }
                }
                if matches!(m.phase, ModalPhase::Gap { .. })
                    && !s.modal
                    && m.rearms > 0
                    && m.rearm_tick != Some(tick)
                    && s.next_id < b.max_ids
                {
                    for gap in 1..=b.max_gap {
                        out.push(OwnerAct::Rearm(gap));
                    }
                }
            }
            Lease::Quiet(_) | Lease::Compat(_) => {
                out.push(OwnerAct::Poll);
                out.push(OwnerAct::Drop);
            }
        }
    }

    // ---- the host's side ----

    /// M5: the ledger copy of an active-clock deadline. The head design keeps
    /// the owner's active-clock deadline. M5 converts it once, when it is
    /// installed, to a wall deadline (`wall_now + (deadline - active_now)`),
    /// kept in the same field.
    fn ledger_for(mutation: Mutation, wall: u8, active: u8, until: u8) -> LedgerBound {
        if mutation == Mutation::M5ActiveToWall {
            LedgerBound::Active(wall.saturating_add(until.saturating_sub(active)))
        } else {
            LedgerBound::Active(until)
        }
    }

    /// Whether an active-clock ledger bound is not yet reached at the sample.
    fn active_before(&self, s: &State, at: u8) -> bool {
        if self.mutation == Mutation::M5ActiveToWall {
            s.wall < at
        } else {
            s.active < at
        }
    }

    /// M1 and M2: the host has no leases. Its hold starts at the owner's click
    /// and ends when a modal opens or `ticks` observed ticks after the click.
    fn host_without_leases(&self, s: &mut State, info: &mut Info, ticks: u8) {
        if info.emitted {
            s.hold_rec = Some(HostHold {
                key: Key::Modal(0),
                released: false,
                boundary: false,
                deadline: 0,
                start: s.tick,
            });
        }
        if let Some(h) = s.hold_rec
            && !h.released
            && (s.modal || s.tick >= h.start.saturating_add(ticks))
        {
            s.hold_rec = None;
        }
        info.wait = s.hold_rec.map(|h| (h.key, h.start));
    }

    /// `quiet_read(now)`: the lease's own wall bound. True when it released.
    fn quiet_wall_release(&self, s: &mut State) -> bool {
        if let Lease::Quiet(mut q) = s.lease
            && q.held
            && self.mutation != Mutation::CheckQuietNoWall
            && s.wall.saturating_sub(q.since) >= self.bounds.quiet_wall
        {
            q.held = false;
            s.lease = Lease::Quiet(q);
            return true;
        }
        false
    }

    /// `SlotScript::script_interaction_wait`: the three leases in order. It
    /// reads the owner's clocks at the sample and converts an active-clock
    /// bound to a tick when it first sees it reached (`WaitDeadline::owner_tick`).
    fn sample(&self, s: &mut State, info: &mut Info) -> Option<Wait> {
        if !s.runtime_live {
            return None;
        }
        if self.quiet_wall_release(s) {
            info.releases.push((Kind::Quiet, Cause::Wall));
        }
        match s.lease {
            Lease::None => None,
            Lease::Modal(mut m) => {
                let deadline = match m.ledger {
                    LedgerBound::Tick(d) => d,
                    LedgerBound::Active(at) if self.active_before(s, at) => s.tick,
                    LedgerBound::Active(_) => {
                        m.ledger = LedgerBound::Tick(s.tick);
                        s.tick
                    }
                };
                s.lease = Lease::Modal(m);
                Some(Wait {
                    key: Key::Modal(m.id),
                    deadline,
                })
            }
            Lease::Quiet(q) => q.held.then_some(Wait {
                key: Key::Quiet(q.action),
                deadline: q.ledger_tick,
            }),
            Lease::Compat(c) => {
                if s.wall < c.hold_until {
                    Some(Wait {
                        key: if self.mutation == Mutation::M4CompatConstantKey {
                            Key::Compat {
                                token: u8::MAX,
                                generation: u8::MAX,
                            }
                        } else {
                            Key::Compat {
                                token: c.token,
                                generation: c.generation,
                            }
                        },
                        // `self_ending_tick`: a live read is not yet due.
                        deadline: s.tick,
                    })
                } else {
                    if s.hold_rec.is_some_and(|h| !h.released) {
                        info.releases.push((Kind::Compat, Cause::Wall));
                    }
                    None
                }
            }
        }
    }

    /// `ScriptRunPolicy::observe_script_wait`: the expiry it returns, as the
    /// key, the reported deadline and the tick.
    fn observe_wait(&self, s: &mut State, wait: Option<&Wait>) -> Option<(Key, u8, u8)> {
        // The host as it was at `0a3e700f8`: no re-arm at a renewal.
        let f1 = matches!(
            self.mutation,
            Mutation::F1RenewalKeepsRelease | Mutation::CheckSendKeepsKey
        );
        // Only the earlier designs read the start tick; the head design keeps
        // it at 0 so that its state space is unchanged.
        let start = if self.mutation == Mutation::M3FixedTerminal {
            s.tick
        } else {
            0
        };
        s.hold_rec = match (wait, s.hold_rec) {
            (None, held) if self.mutation == Mutation::CheckStaleHold => held,
            (None, _) => None,
            (Some(w), Some(mut h)) if h.key == w.key => {
                // A renewal that moves the deadline later after the terminal
                // release is a wait of its own: it holds again. A boundary
                // release and a repeat of the deadline do not.
                if !f1 && h.released && !h.boundary && w.deadline > h.deadline {
                    h.released = false;
                    h.start = start;
                }
                if !f1 {
                    h.deadline = w.deadline;
                }
                Some(h)
            }
            (Some(w), _) => Some(HostHold {
                key: w.key,
                released: false,
                boundary: false,
                deadline: if f1 { 0 } else { w.deadline },
                start,
            }),
        };
        let (Some(w), Some(h)) = (wait, s.hold_rec.as_mut()) else {
            return None;
        };
        let margin = match self.mutation {
            Mutation::CheckMarginZero => 0,
            Mutation::CheckMarginTwo => 2,
            _ => MARGIN_TICKS,
        };
        let early = if self.mutation == Mutation::M3FixedTerminal {
            s.tick < h.start.saturating_add(M3_FIXED_TERMINAL_TICKS)
        } else {
            s.tick < w.deadline.saturating_add(margin)
        };
        if h.released || early {
            return None;
        }
        h.released = true;
        s.released_txn = s.lease.txn();
        Some((w.key, w.deadline, s.tick))
    }

    fn deferred(s: &State) -> bool {
        s.hold_rec.is_some_and(|h| !h.released)
    }

    /// Keep the ghost deadline tick: the first frame at which the owner's own
    /// deadline, in its own clock, is seen passed.
    fn update_due(s: &mut State) {
        s.due = if Self::live(s) && Self::passed(s) {
            s.due.or(Some(s.tick))
        } else {
            None
        };
    }

    fn violation(name: &'static str, detail: String) -> Violation {
        Violation { name, detail }
    }

    /// The frozen invariants, over the state after a frame. `old_txn` is the
    /// newest transaction that existed before the frame; `owed_end` says that
    /// Stop, a session boundary or a death cancel ended every older transaction.
    fn check_frame(
        &self,
        post: &State,
        info: &Info,
        old_txn: u8,
        owed_end: bool,
    ) -> Result<(), Violation> {
        let live = Self::live(post);
        let passed = Self::passed(post);
        // S1: no press while a lease is live and its owner's deadline has not passed.
        if info.pressed && live && !passed {
            return Err(Self::violation(
                "S1-press-while-live",
                format!(
                    "the host pressed run while {:?} was live and its own deadline had not passed (tick {}, active {}, wall {})",
                    post.lease, post.tick, post.active, post.wall
                ),
            ));
        }
        // S6: a verb that the owner sends is never followed by a press in the
        // same frame: the press would close the page the verb opens.
        if info.emitted && info.pressed {
            return Err(Self::violation(
                "S6-press-after-send",
                format!(
                    "the owner sent a verb in this frame and the host pressed run after it (lease {:?}, host {:?})",
                    post.lease, post.hold_rec
                ),
            ));
        }
        // S3: a terminal release comes only after the owner's deadline plus the margin.
        if let Some((key, deadline, tick)) = info.expired {
            let early = match post.due {
                Some(due) => tick < due.saturating_add(MARGIN_TICKS),
                None => true,
            };
            if early {
                return Err(Self::violation(
                    "S3-early-terminal-release",
                    format!(
                        "the host ended the hold of {key:?} at tick {tick} (reported deadline {deadline}) but the owner's deadline passed at tick {:?}",
                        post.due
                    ),
                ));
            }
        }
        // L: bounded liveness: no hold outlives its owner's deadline plus the margin.
        if info.deferred
            && let Some(due) = post.due
            && post.tick >= due.saturating_add(MARGIN_TICKS)
        {
            return Err(Self::violation(
                "L-hold-outlives-deadline",
                format!(
                    "the hold is live at tick {}: its owner's deadline passed at tick {due} (margin {MARGIN_TICKS})",
                    post.tick
                ),
            ));
        }
        // S0: no stale hold: a hold is live only while the sample returned a wait.
        if info.deferred && info.wait.is_none() {
            return Err(Self::violation(
                "S0-hold-without-wait",
                "the host holds the press but the sample returned no wait".to_string(),
            ));
        }
        // S4: a released state never carries over to a new transaction.
        if let (Some(h), Some(txn)) = (post.hold_rec, post.lease.txn())
            && h.released
            && live
            && Self::key_of(post.lease) == Some(h.key)
            && post.released_txn != Some(txn)
        {
            return Err(Self::violation(
                "S4-released-state-carried",
                format!(
                    "the host's released flag for {:?} belongs to transaction {:?}, but the live lease is transaction {txn}",
                    h.key, post.released_txn
                ),
            ));
        }
        // S2: after Stop, a session boundary or a death, no older hold stays.
        if owed_end && post.lease.txn().is_some_and(|txn| txn <= old_txn) {
            return Err(Self::violation(
                "S2-hold-after-end",
                format!(
                    "after Stop, a session boundary or a death cancel an older lease stays: lease {:?}, host {:?}",
                    post.lease, post.hold_rec
                ),
            ));
        }
        // S5: the page stops being hidden at the lease's wall cap from its begin.
        let (hidden, since, cap) = match post.lease {
            Lease::Quiet(q) => (q.held, q.since, self.bounds.quiet_wall),
            Lease::Compat(c) => (post.wall < c.paint_until, c.since, self.bounds.compat_hold),
            _ => (false, 0, 0),
        };
        if hidden && post.wall.saturating_sub(since) >= cap {
            return Err(Self::violation(
                "S5-paint-hidden-past-cap",
                format!(
                    "the journal page is still hidden at wall {} for a read begun at wall {since} (cap {cap})",
                    post.wall
                ),
            ));
        }
        Ok(())
    }

    // ---- one frame ----

    fn frame(&self, pre: &State, f: FrameInput, info: &mut Info) -> (State, Option<Violation>) {
        let mut s = pre.clone();
        let owed_stop = std::mem::take(&mut s.stopped_since_frame);
        let old_txn = s.txn;
        if f.tick {
            s.tick += 1;
        }
        // The host's drain: `after_drain` releases at a session boundary.
        if s.session_pending {
            if let Some(h) = s.hold_rec.as_mut() {
                h.released = true;
                h.boundary = true;
                s.released_txn = s.lease.txn();
            }
            s.run_on = false;
        }
        // The observe hook: the session reset, then the clock and the lease's wall bound.
        let boundary = s.session_pending;
        if boundary {
            if s.lease != Lease::None {
                Self::end_lease(&mut s, info, Cause::Session);
            }
            s.work_gen = s.work_gen.saturating_add(1);
            s.eligible = false;
            s.session_pending = false;
            info.boundary = true;
        }
        if self.quiet_wall_release(&mut s) {
            info.releases.push((Kind::Quiet, Cause::Wall));
        }
        let running = Self::running(&s);
        s.eligible = running && !s.hold;
        // The owner's poll.
        let poll_enabled = running && !s.hold && !s.cleanup && (f.tick || (f.wake && s.tick > 0));
        let mut death_cancel = false;
        if poll_enabled {
            info.polled = true;
            if s.death_msg {
                // The runner cancels its step when it reads the death line.
                if s.lease != Lease::None {
                    Self::end_lease(&mut s, info, Cause::Death);
                }
                s.hp_zero = false;
                s.death_msg = false;
                death_cancel = true;
            } else {
                match f.act {
                    OwnerAct::NoPoll | OwnerAct::Stall => {}
                    OwnerAct::Begin(kind) => self.begin(&mut s, kind, info),
                    OwnerAct::Poll => self.poll(&mut s, info),
                    OwnerAct::Send => self.send(&mut s, info),
                    OwnerAct::ToGap(gap) => Self::to_gap(&mut s, gap),
                    OwnerAct::Rearm(gap) => self.rearm(&mut s, gap),
                    OwnerAct::Drop => Self::end_lease(&mut s, info, Cause::Dropped),
                }
            }
        }
        // Time passes between the poll and the sample.
        if f.drift {
            s.wall = s.wall.saturating_add(1);
            if s.eligible {
                s.active = s.active.saturating_add(1);
            }
        }
        // The host samples the lease (also while paused) and applies its policy.
        let ticks_without_leases = match self.mutation {
            Mutation::M1HoldTwoTicks => Some(2),
            Mutation::M2HoldThreeTicks => Some(3),
            _ => None,
        };
        let wait = if ticks_without_leases.is_some() {
            None
        } else {
            self.sample(&mut s, info)
        };
        let generation = s.runtime_live.then_some(s.rgen);
        if s.host_generation != generation {
            s.host_generation = generation;
            s.hold_rec = None;
        }
        Self::update_due(&mut s);
        if let Some(ticks) = ticks_without_leases {
            self.host_without_leases(&mut s, info, ticks);
        } else {
            info.wait = wait.as_ref().map(|w| (w.key, w.deadline));
            if let Some((key, deadline, tick)) = self.observe_wait(&mut s, wait.as_ref()) {
                info.expired = Some((key, deadline, tick));
                if let Some(kind) = s.lease.kind() {
                    info.releases.push((kind, Cause::HostTerminal));
                }
            }
        }
        let defer = Self::deferred(&s);
        info.deferred = defer;
        if s.up && !s.modal && !s.run_on && !defer {
            s.run_on = true;
            info.pressed = true;
        }
        let violation = self
            .check_frame(&s, info, old_txn, owed_stop || boundary || death_cancel)
            .err();
        (s, violation)
    }

    // ---- the other actions ----

    fn other(&self, pre: &State, action: Action, info: &mut Info) -> State {
        let mut s = pre.clone();
        match action {
            Action::Frame(_) => unreachable!("frames are handled by `frame`"),
            Action::Wait => {
                s.wall += 1;
                if s.eligible {
                    s.active += 1;
                }
            }
            Action::Pause => {
                s.want_run = false;
                s.eligible = false;
            }
            Action::Resume => s.want_run = true,
            Action::Stop => {
                if s.lease != Lease::None && self.mutation != Mutation::CheckStopKeepsLease {
                    Self::end_lease(&mut s, info, Cause::Stop);
                }
                s.runtime_live = false;
                s.want_run = false;
                s.eligible = false;
                s.stopped_since_frame = true;
            }
            Action::Start => {
                s.runtime_live = true;
                s.rgen = s.rgen.saturating_add(1);
                s.want_run = true;
            }
            Action::Disconnect => {
                s.up = false;
                s.session_pending = true;
                Self::client_reset(&mut s);
            }
            Action::Connect => {
                s.up = true;
                s.session_pending = true;
            }
            Action::Relog => {
                s.session_pending = true;
                Self::client_reset(&mut s);
            }
            Action::ModalOpen => {
                s.modal = true;
                s.page += 1;
            }
            Action::ModalClose => {
                s.modal = false;
                s.page += 1;
            }
            Action::ModalNext => s.page += 1,
            Action::HoldOn => s.hold = true,
            Action::HoldOff => s.hold = false,
            Action::CleanupOn => s.cleanup = true,
            Action::CleanupOff => s.cleanup = false,
            Action::Die => s.hp_zero = true,
            Action::DeathMsg => s.death_msg = true,
            Action::RunOff => s.run_on = false,
        }
        s
    }

    /// One transition with its observation record, and the violation that it
    /// proves, if any. A violating transition still returns its state and
    /// record: the conformance test needs the model's decision at the
    /// frame of a counterexample.
    pub fn step_unchecked(&self, pre: &State, action: &Action) -> (State, Info, Option<Violation>) {
        let mut info = Info::default();
        let (next, violation) = match action {
            Action::Frame(f) => self.frame(pre, *f, &mut info),
            other => (self.other(pre, *other, &mut info), None),
        };
        (next, info, violation)
    }

    /// One transition with its observation record.
    pub fn step_detail(&self, pre: &State, action: &Action) -> Result<(State, Info), Violation> {
        match self.step_unchecked(pre, action) {
            (next, info, None) => Ok((next, info)),
            (_, _, Some(violation)) => Err(violation),
        }
    }

    fn frames(&self, s: &State, out: &mut Vec<Action>) {
        let b = &self.bounds;
        let running = Self::running(s);
        let poll_possible = running && !s.hold && !s.cleanup;
        let may_drift = s.wall < b.max_wall;
        let drifts: &[bool] = if may_drift { &[false, true] } else { &[false] };
        let ticks: &[bool] = if s.tick < b.max_tick {
            &[false, true]
        } else {
            &[false]
        };
        let mut acts = Vec::new();
        for &tick in ticks {
            let wakes: &[bool] = if poll_possible && !tick && s.tick > 0 {
                &[false, true]
            } else {
                &[false]
            };
            for &wake in wakes {
                acts.clear();
                if poll_possible && (tick || wake) {
                    self.owner_acts(s, s.tick + u8::from(tick), &mut acts);
                } else {
                    acts.push(OwnerAct::NoPoll);
                }
                for &drift in drifts {
                    for &act in &acts {
                        let polls = !matches!(act, OwnerAct::NoPoll);
                        if drift
                            && ((s.lease == Lease::None && !matches!(act, OwnerAct::Begin(_)))
                                || (b.drift_polls_only && !polls))
                        {
                            continue;
                        }
                        out.push(Action::Frame(FrameInput {
                            tick,
                            wake,
                            drift,
                            act,
                        }));
                    }
                }
            }
        }
    }
}

impl Machine for Model {
    type State = State;
    type Action = Action;

    fn initial(&self) -> State {
        self.initial_state()
    }

    fn actions(&self, s: &State, out: &mut Vec<Action>) {
        let b = &self.bounds;
        self.frames(s, out);
        if s.wall < b.max_wall {
            out.push(Action::Wait);
        }
        let env = &b.env;
        if env.pause && s.runtime_live {
            out.push(if s.want_run {
                Action::Pause
            } else {
                Action::Resume
            });
        }
        if env.stop {
            if s.runtime_live {
                out.push(Action::Stop);
            } else if s.rgen <= b.max_epochs {
                out.push(Action::Start);
            }
        }
        if env.session && s.work_gen <= b.max_epochs {
            if s.up {
                if env.disconnect {
                    out.push(Action::Disconnect);
                }
                out.push(Action::Relog);
            } else {
                out.push(Action::Connect);
            }
        }
        if s.up && s.page < b.max_pages {
            if s.modal {
                out.push(Action::ModalClose);
                if env.page_change {
                    out.push(Action::ModalNext);
                }
            } else {
                out.push(Action::ModalOpen);
            }
        }
        if env.hold {
            out.push(if s.hold {
                Action::HoldOff
            } else {
                Action::HoldOn
            });
        }
        if env.cleanup {
            out.push(if s.cleanup {
                Action::CleanupOff
            } else {
                Action::CleanupOn
            });
        }
        if env.death && s.up {
            if !s.hp_zero {
                out.push(Action::Die);
            } else if !s.death_msg {
                out.push(Action::DeathMsg);
            }
        }
        if env.run_off && s.run_on {
            out.push(Action::RunOff);
        }
    }

    fn step(&self, s: &State, a: &Action) -> Result<State, Violation> {
        self.step_detail(s, a).map(|(next, _)| next)
    }
}

// ---- traces in the product's terms ----

fn lease_text(lease: Lease) -> String {
    match lease {
        Lease::None => "no lease".to_string(),
        Lease::Modal(m) => {
            let phase = match m.phase {
                ModalPhase::Page { until } => format!("page wait until active {until}"),
                ModalPhase::Ack { base, until } => {
                    format!("ack wait (page {base}) until active {until}")
                }
                ModalPhase::Gap { due } => format!("end gap due tick {due}"),
            };
            format!(
                "Dialogue modal wait #{} {phase}, ledger {:?}",
                m.id, m.ledger
            )
        }
        Lease::Quiet(q) => format!(
            "journal quiet read action {} {:?}, window tick {}, ledger {}, held {}",
            q.action, q.phase, q.window, q.ledger_tick, q.held
        ),
        Lease::Compat(c) => format!(
            "compat journal read token {} {:?}, hold until wall {}",
            c.token, c.phase, c.hold_until
        ),
    }
}

impl State {
    /// One line of the world, the lease and the host.
    pub fn summary(&self) -> String {
        let mut out = String::new();
        let _ = write!(
            out,
            "tick {} wall {} active {}{} modal {}(page {}) | {} | host {}",
            self.tick,
            self.wall,
            self.active,
            if self.eligible { "+" } else { "=" },
            if self.modal { "open" } else { "closed" },
            self.page,
            lease_text(self.lease),
            match self.hold_rec {
                None => "no hold".to_string(),
                Some(h) if h.released => format!("released {:?}", h.key),
                Some(h) => format!("holds {:?}", h.key),
            }
        );
        out
    }
}

impl Action {
    pub fn text(&self) -> String {
        match self {
            Action::Wait => "time passes (one unit)".to_string(),
            Action::Frame(f) => {
                let mut parts = vec![String::from("frame")];
                if f.tick {
                    parts.push("with a game tick".to_string());
                } else if f.wake {
                    parts.push("with an evidence wake".to_string());
                }
                if f.drift {
                    parts.push("(time passes between poll and sample)".to_string());
                }
                parts.push(match f.act {
                    OwnerAct::NoPoll => "no owner poll".to_string(),
                    OwnerAct::Stall => "owner polls, changes nothing".to_string(),
                    OwnerAct::Begin(Begin::Modal) => {
                        "owner begins a Dialogue modal wait".to_string()
                    }
                    OwnerAct::Begin(Begin::Quiet) => {
                        "owner begins a journal quiet read".to_string()
                    }
                    OwnerAct::Begin(Begin::Compat) => {
                        "owner begins a compat journal read".to_string()
                    }
                    OwnerAct::Poll => "owner polls".to_string(),
                    OwnerAct::Send => "owner sends again (new acknowledgement wait)".to_string(),
                    OwnerAct::ToGap(g) => {
                        format!("owner renews its wait as an end gap of {g} tick(s)")
                    }
                    OwnerAct::Rearm(g) => {
                        format!("owner re-arms its gap by {g} tick(s) (new lease)")
                    }
                    OwnerAct::Drop => "owner drops its lease".to_string(),
                });
                parts.join(", ")
            }
            Action::Pause => "operator Pause".to_string(),
            Action::Resume => "operator Resume".to_string(),
            Action::Stop => "operator Stop".to_string(),
            Action::Start => "operator Start".to_string(),
            Action::Disconnect => "connection drops".to_string(),
            Action::Connect => "session connects".to_string(),
            Action::Relog => "relog in one drain (session changes)".to_string(),
            Action::ModalOpen => "server opens a modal".to_string(),
            Action::ModalClose => "server closes the modal".to_string(),
            Action::ModalNext => "server replaces the open page".to_string(),
            Action::HoldOn => "guardian hold on".to_string(),
            Action::HoldOff => "guardian hold off".to_string(),
            Action::CleanupOn => "prayer cleanup hold on".to_string(),
            Action::CleanupOff => "prayer cleanup hold off".to_string(),
            Action::Die => "player hitpoints reach zero".to_string(),
            Action::DeathMsg => "server sends the death message".to_string(),
            Action::RunOff => "run turns off (energy or echo)".to_string(),
        }
    }
}

/// Replay `trace` and print each step with its effect, in the product's terms.
pub fn describe_trace(model: &Model, trace: &[Action]) -> String {
    let mut out = String::new();
    let mut state = model.initial_state();
    let _ = writeln!(out, "  start: {}", state.summary());
    for (i, action) in trace.iter().enumerate() {
        let _ = writeln!(out, "  {:>2}. {}", i + 1, action.text());
        match model.step_detail(&state, action) {
            Ok((next, info)) => {
                for (kind, cause) in &info.releases {
                    let _ = writeln!(out, "        released: {kind:?} lease, {cause:?}");
                }
                if info.pressed {
                    let _ = writeln!(out, "        the host PRESSED run");
                } else if info.deferred {
                    let _ = writeln!(out, "        the host holds the press");
                }
                let _ = writeln!(out, "        {}", next.summary());
                state = next;
            }
            Err(violation) => {
                let _ = writeln!(
                    out,
                    "        VIOLATION {}: {}",
                    violation.name, violation.detail
                );
                break;
            }
        }
    }
    out
}
