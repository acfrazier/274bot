//! Intent and session vocabulary (F0 item 2: contract D6, B03 "Intent
//! semantics", §20 L4/L8 as the starting design, §26 §1 item 2).
//!
//! Frozen here:
//!
//! - the [`IntentMailbox`] row matrix: every operator, script or lifecycle
//!   input an owner reduces ([`Intent`] and [`LifecycleFact`]), each with its
//!   frozen [`RowMeaning`] (what it does to the session and login policy,
//!   what it does to the run, and what it does to an in-flight attempt);
//! - the generation-stamped mailbox itself, a pure reducer of operator intent
//!   with today's semantics (`SlotIntent`, `crates/host-play/src/play_login.rs:104-125`):
//!   every command advances the generation, only attempt-invalidating rows
//!   advance the cancellation fence, agreeing refreshes create no second
//!   one-shot, a clean Logout latches automatic login off until an explicit
//!   rearm, acknowledgements from an older generation apply only while the
//!   current intent still asks for the same outcome, a turnover (Relog now,
//!   Switch world) owes its login half only until a stronger row supersedes
//!   it, and Remove or ProcessShutdown closes the mailbox's slot lifetime for
//!   good (today the sticky `SlotArm::stop`, `play_login.rs:807-817,945-949`);
//!   no generation wraps ([`MailboxError::Exhausted`]);
//! - [`SessionEventOut`], the single session boundary every consumer reads,
//!   carried in B03's identity envelope ([`crate::identity::Event`]) with a
//!   slot or session scope, and the [`SessionGate`] that derives "the script
//!   may tick" from it behind the owner-authorized session fence;
//! - the run-side hold vocabulary: [`SessionHeld`] is not operator pause, and
//!   only Stop ends a run ([`RunEffect::Terminate`]). The run reducer itself
//!   (today `script::RunState` under `SlotScript`) is characterized at its
//!   own seam by F0-3/T2; nothing here authorizes work.
//!
//! The first rows are the merged relog and login-pacing rules, characterized
//! from the current code they describe (see each row's documentation).
//! Reducer and caller activation remain T1.3; nothing here is wired into a
//! slot.
//!
//! Owner-only obligations the mailbox does not carry (they need the owner's
//! clock or transport evidence): the repeat-guard history
//! (`record_unexpected_logout_at`, `play_login.rs:744-778`; an explicit
//! Login clears it, `:378`; an exit is counted only when
//! [`IntentSnapshot::logout_wanted`] was false before the exit was observed,
//! `:749`); the clean-release evidence (`CleanExit`, `:819-885`), which
//! every row that moves the cancellation fence also invalidates; and the
//! process-condition ledger (`crate::process_condition::ProcessConditions`),
//! consulted before every attempt so a login the intent still wants is
//! refused, not retried, while its capability is unavailable.

use std::time::Duration;

use crate::identity::{Deadline, Fence, SessionKey, SlotKey};
use crate::process_condition::{ConditionId, ProcessCapability};

/// Client memory mode queued for the next fresh handshake (today
/// `SlotArm::lowmem_handshake`, `play_login.rs:195`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryMode {
    /// High-detail client.
    High,
    /// Low-memory client.
    Low,
}

/// Which world the next handshake selects (today `SlotArm::world`,
/// `play_login.rs:172`: `None` keeps automatic fallback).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorldChoice {
    /// Automatic fallback through the public world round.
    Automatic,
    /// A specific public world.
    World(u16),
}

/// An operator- or script-originated row of the intent matrix: what may be
/// posted to the [`IntentMailbox`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Intent {
    /// Explicit operator "Log in": arms one login even if auto-login is off,
    /// lifts every latch and hold and supersedes a pending turnover (today
    /// `SlotArm::arm_explicit_login`, `play_login.rs:370-381`, and the
    /// frontend's `arm_login`, `crates/frontend-core/src/session.rs:941-946`).
    Login,
    /// The persistent auto-login preference (today `SlotArm::set_auto_login`,
    /// `play_login.rs:518-536`).
    AutoLogin(bool),
    /// Clean operator Logout: cancels a queued or handshaking attempt or
    /// sends the clean game logout, latches automatic and script-triggered
    /// login off until an explicit rearm, and outranks a pending turnover
    /// (today `SlotArm::request_logout`, `play_login.rs:384-403`, and
    /// `logout_member`, `session.rs:1012-1027`).
    Logout,
    /// A persisted logged-out hold restored at spawn: latched like a Logout
    /// without requesting one from the title screen (today
    /// `SlotArm::hold_logged_out`, `play_login.rs:407-418`).
    HoldLoggedOut,
    /// Withdraw the active login intent without changing the saved policy
    /// (today `SlotArm::withdraw_login`, `play_login.rs:540-549`); a pending
    /// turnover's login half is withdrawn with it.
    WithdrawLogin,
    /// Relog now: explicit clean session turnover followed by a login, not
    /// Logout's park latch. Today the memory-mode relog composes it from the
    /// owner's Logout and Login: the logout half goes out while online and
    /// the login half is armed once the slot is parked
    /// (`request_memory_relog`/`complete_memory_relogs`,
    /// `session.rs:1102-1155`); with no live session it is a plain login
    /// (`session.rs:1107-1109`).
    RelogNow,
    /// World preference save: changes the next handshake's selection and
    /// never disconnects an online bot (today the world branch of
    /// `sync_profile_arm`, `play_login.rs:921-932`).
    WorldPreference(WorldChoice),
    /// Explicit switch world: cancel the old attempt or cleanly exit the
    /// online session, then log in to the requested world. Today's product
    /// has no single command for it: the row is the contract's (B03), and
    /// its current evidence is the composition Logout → world save
    /// (`sync_profile_arm`) → Login, sequenced by the frontend the way the
    /// memory relog is. Operational Switch World waits for T1.3's caller
    /// integration.
    SwitchWorld(u16),
    /// Memory mode for the next fresh handshake; a dirty reconnect keeps the
    /// server session's applied mode (today `set_lowmem_handshake` and
    /// `login_applies_memory`, `play_login.rs:437-457`).
    MemoryMode(MemoryMode),
    /// The script seat's level-triggered session need: a running or paused
    /// run wants a login whatever the auto-login preference says (today
    /// `SlotArm::set_script_active`, `play_login.rs:597-604`). The end of
    /// the need (`false` after `true`) also invalidates the in-flight
    /// attempt's outcome (today `invalidate_clean_exit`, `:600,881-885`).
    ScriptNeedsSession(bool),
    /// The login identity (profile uid) changed under a running slot: the
    /// next handshake uses it and nothing in flight is current any more
    /// (today `invalidate_login_identity` from `sync_profile_arm`,
    /// `play_login.rs:887-894,906,913-915`).
    LoginIdentityChanged,
    /// Operator Start of a script.
    StartScript,
    /// Operator Pause: the run keeps its identity and holds its clocks.
    PauseScript,
    /// Operator Resume of an operator pause.
    ResumeScript,
    /// Operator Stop: ends the run; it does not imply an account logout.
    StopScript,
    /// The watchdog's restart request: stop and recreate the execution,
    /// keep the operator run and progress identity.
    WatchdogRestart,
    /// Remove the slot: closes this mailbox's slot lifetime for good (today
    /// `SlotArm::stop`, `play_login.rs:156`, set by `signal_slot_stop`,
    /// `play_slots.rs:172-194`); a connected slot first gets the clean
    /// logout (`remove`, `session.rs:1228-1231`). A replacement is a new
    /// slot lifetime with a new mailbox.
    RemoveSlot,
    /// Process shutdown: closes the lifetime like [`Intent::RemoveSlot`].
    ProcessShutdown,
}

impl Intent {
    /// Every row, for exhaustive checks.
    pub const ALL: [Intent; 20] = [
        Intent::Login,
        Intent::AutoLogin(true),
        Intent::AutoLogin(false),
        Intent::Logout,
        Intent::HoldLoggedOut,
        Intent::WithdrawLogin,
        Intent::RelogNow,
        Intent::WorldPreference(WorldChoice::Automatic),
        Intent::SwitchWorld(1),
        Intent::MemoryMode(MemoryMode::Low),
        Intent::ScriptNeedsSession(true),
        Intent::ScriptNeedsSession(false),
        Intent::LoginIdentityChanged,
        Intent::StartScript,
        Intent::PauseScript,
        Intent::ResumeScript,
        Intent::StopScript,
        Intent::WatchdogRestart,
        Intent::RemoveSlot,
        Intent::ProcessShutdown,
    ];

    /// The frozen meaning of this row.
    pub const fn meaning(self) -> RowMeaning {
        match self {
            Intent::Login => RowMeaning {
                session: SessionEffect::ArmLogin,
                run: RunEffect::ResumeHeld,
                attempt: AttemptCancellation::Never,
            },
            Intent::AutoLogin(true) => RowMeaning {
                session: SessionEffect::PolicyOnly,
                run: RunEffect::None,
                attempt: AttemptCancellation::Never,
            },
            Intent::AutoLogin(false) => RowMeaning {
                session: SessionEffect::PolicyOnly,
                run: RunEffect::None,
                attempt: AttemptCancellation::IfAutoDerived,
            },
            Intent::Logout => RowMeaning {
                session: SessionEffect::CleanLogoutAndLatch,
                run: RunEffect::HoldWork,
                attempt: AttemptCancellation::Always,
            },
            Intent::HoldLoggedOut => RowMeaning {
                session: SessionEffect::LatchOnly,
                run: RunEffect::HoldWork,
                attempt: AttemptCancellation::Always,
            },
            Intent::WithdrawLogin => RowMeaning {
                session: SessionEffect::WithdrawLogin,
                run: RunEffect::None,
                attempt: AttemptCancellation::Always,
            },
            Intent::RelogNow => RowMeaning {
                session: SessionEffect::Turnover,
                run: RunEffect::RevalidateUnderNewSession,
                attempt: AttemptCancellation::Never,
            },
            Intent::WorldPreference(_) => RowMeaning {
                session: SessionEffect::NextHandshakeOnly,
                run: RunEffect::None,
                attempt: AttemptCancellation::Never,
            },
            Intent::SwitchWorld(_) => RowMeaning {
                session: SessionEffect::SwitchWorld,
                run: RunEffect::RevalidateUnderNewSession,
                attempt: AttemptCancellation::Always,
            },
            Intent::MemoryMode(_) => RowMeaning {
                session: SessionEffect::NextHandshakeOnly,
                run: RunEffect::None,
                attempt: AttemptCancellation::Never,
            },
            Intent::ScriptNeedsSession(true) => RowMeaning {
                session: SessionEffect::PolicyOnly,
                run: RunEffect::None,
                attempt: AttemptCancellation::Never,
            },
            Intent::ScriptNeedsSession(false) => RowMeaning {
                session: SessionEffect::PolicyOnly,
                run: RunEffect::None,
                attempt: AttemptCancellation::IfScriptNeedEnds,
            },
            Intent::LoginIdentityChanged => RowMeaning {
                session: SessionEffect::NextHandshakeOnly,
                run: RunEffect::None,
                attempt: AttemptCancellation::Always,
            },
            Intent::StartScript => RowMeaning {
                session: SessionEffect::None,
                run: RunEffect::Start,
                attempt: AttemptCancellation::Never,
            },
            Intent::PauseScript => RowMeaning {
                session: SessionEffect::None,
                run: RunEffect::Pause,
                attempt: AttemptCancellation::Never,
            },
            Intent::ResumeScript => RowMeaning {
                session: SessionEffect::None,
                run: RunEffect::Resume,
                attempt: AttemptCancellation::Never,
            },
            Intent::StopScript => RowMeaning {
                session: SessionEffect::None,
                run: RunEffect::Terminate,
                attempt: AttemptCancellation::Never,
            },
            Intent::WatchdogRestart => RowMeaning {
                session: SessionEffect::None,
                run: RunEffect::RecreateExecution,
                attempt: AttemptCancellation::Never,
            },
            Intent::RemoveSlot | Intent::ProcessShutdown => RowMeaning {
                session: SessionEffect::Teardown,
                run: RunEffect::Terminate,
                attempt: AttemptCancellation::Always,
            },
        }
    }
}

/// A lifecycle fact the owner reduces alongside intents: not posted by an
/// operator, observed from the transport, client or guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LifecycleFact {
    /// The handshake succeeded and a session is live (today
    /// `on_login_success`, `play_login.rs:1080-1088`): the attempt that
    /// completed is settled, so its command stamp is no longer current.
    LoginSucceeded,
    /// The session ended. `clean` is true only for a server logout (today
    /// `tick_flags`, `play_login.rs:1101-1103`); `resumable` is whether the
    /// client kept its world, which selects reconnect (opcode 18) over a
    /// fresh login (16) (`play_login.rs:1110-1119`). Operator Login intent
    /// never turns a dirty reconnect into a clean login. An exit while a
    /// turnover's logout half is pending settles that half: the session is
    /// gone, so the login half is armed (today `complete_memory_relogs`
    /// fires it once the slot is parked, `session.rs:1124-1155`).
    SessionExited {
        /// The exit was a server logout (operator logout, world change or
        /// idle), never a drop.
        clean: bool,
        /// The client kept its world, so the next handshake may reconnect.
        resumable: bool,
    },
    /// The repeat guard tripped: `count` unexpected exits within the window
    /// (today `record_unexpected_logout_at`, `play_login.rs:744-778`:
    /// 3 within 600 s). It parks automatic retries with an explicit reason
    /// and retains recoverable run work; an explicit Login may resume it.
    RepeatGuardTripped {
        /// Unexpected exits counted in the window.
        count: u8,
        /// The window, in seconds.
        window_seconds: u64,
    },
    /// A world-preference error withdrew the login, including a script's
    /// (today `hold_login_on_error`, `play_login.rs:554-564`); the slot parks
    /// until the operator arms a login or changes auto-login or the world.
    /// A pending turnover ends with it (today `finish_memory_relogs`,
    /// `session.rs:1160-1173`).
    WorldPreferenceFailed,
}

impl LifecycleFact {
    /// Every fact row (with representative values), for exhaustive checks.
    pub const ALL: [LifecycleFact; 5] = [
        LifecycleFact::LoginSucceeded,
        LifecycleFact::SessionExited {
            clean: true,
            resumable: false,
        },
        LifecycleFact::SessionExited {
            clean: false,
            resumable: true,
        },
        LifecycleFact::RepeatGuardTripped {
            count: 3,
            window_seconds: 600,
        },
        LifecycleFact::WorldPreferenceFailed,
    ];

    /// The frozen meaning of this row.
    pub const fn meaning(self) -> RowMeaning {
        match self {
            LifecycleFact::LoginSucceeded => RowMeaning {
                session: SessionEffect::None,
                run: RunEffect::RevalidateUnderNewSession,
                attempt: AttemptCancellation::Settled,
            },
            LifecycleFact::SessionExited { clean: true, .. } => RowMeaning {
                session: SessionEffect::None,
                run: RunEffect::HoldWork,
                attempt: AttemptCancellation::Never,
            },
            LifecycleFact::SessionExited { clean: false, .. } => RowMeaning {
                session: SessionEffect::ReconnectIfWanted,
                run: RunEffect::HoldWork,
                attempt: AttemptCancellation::Never,
            },
            LifecycleFact::RepeatGuardTripped { .. } => RowMeaning {
                session: SessionEffect::LatchOnly,
                run: RunEffect::HoldWork,
                attempt: AttemptCancellation::Always,
            },
            LifecycleFact::WorldPreferenceFailed => RowMeaning {
                session: SessionEffect::WithdrawLogin,
                run: RunEffect::HoldWork,
                attempt: AttemptCancellation::Always,
            },
        }
    }
}

/// What a row does to the session and the login policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionEffect {
    /// Arm one login regardless of auto-login; lift the latch and every hold.
    ArmLogin,
    /// Change policy or need for later exits; a healthy session stays up.
    PolicyOnly,
    /// Cancel a queued or handshaking attempt or send a clean logout; latch
    /// automatic and script-triggered login off until an explicit rearm.
    CleanLogoutAndLatch,
    /// Latch login off without requesting a logout.
    LatchOnly,
    /// Withdraw the login want; the saved policy stays.
    WithdrawLogin,
    /// Clean turnover then login; not Logout's park latch.
    Turnover,
    /// Next handshake selection only; an online bot is never disconnected.
    NextHandshakeOnly,
    /// Cancel the old attempt or cleanly exit, then log in to the requested
    /// world; a failure holds with a typed reason.
    SwitchWorld,
    /// Reconnect (opcode 18) or fresh login chosen from the exit facts, only
    /// while a login is still wanted and nothing latched the slot.
    ReconnectIfWanted,
    /// No session or policy change.
    None,
    /// Stop producers; cancel login, transport and jobs; close the lifetime.
    Teardown,
}

/// What a row does to the run and its action authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunEffect {
    /// Resume held surviving work; invent no new Start.
    ResumeHeld,
    /// No run or effect change.
    None,
    /// Clear session work and effect authority, including pending action and
    /// callback requests; keep run, execution and progress identity. Running
    /// stays Running plus [`SessionHeld`], Paused stays Paused plus
    /// [`SessionHeld`]; clocks and effects stay held until the next Ready.
    HoldWork,
    /// Revoke execution authority, cancel and settle callbacks and native
    /// requests, then bounded stop and disposal; no later game action from
    /// this execution.
    Terminate,
    /// Stop and recreate the execution with a new generation; keep the run
    /// and progress identity; reset held action authority.
    RecreateExecution,
    /// Retain run intent; invalidate every old-session target, route and
    /// channel delivery; resume only under the permitted world identity.
    RevalidateUnderNewSession,
    /// Start a run.
    Start,
    /// Operator pause: hold clocks, keep identity.
    Pause,
    /// Operator resume.
    Resume,
}

/// What a row does to an in-flight login attempt (§20 L4 item 1: the
/// producer of a session-ending intent fires the attempt itself). Every
/// variant but `Never` moves the cancellation fence, so the attempt's later
/// outcome is stale ([`IntentMailbox::attempt_is_current`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AttemptCancellation {
    /// Always cancels.
    Always,
    /// Never cancels; a later world fence may still make the attempt's
    /// outcome stale.
    Never,
    /// Cancels only an attempt the auto-login preference derived.
    IfAutoDerived,
    /// Cancels only when the script's session need was on: the end of the
    /// need invalidates the attempt it may have derived.
    IfScriptNeedEnds,
    /// The attempt completed under this row: nothing is fired, but its
    /// command stamp is retired.
    Settled,
}

/// The frozen meaning of one matrix row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RowMeaning {
    session: SessionEffect,
    run: RunEffect,
    attempt: AttemptCancellation,
}

impl RowMeaning {
    /// Session and login-policy column.
    pub const fn session(self) -> SessionEffect {
        self.session
    }

    /// Run and action column.
    pub const fn run(self) -> RunEffect {
        self.run
    }

    /// In-flight attempt column.
    pub const fn attempt(self) -> AttemptCancellation {
        self.attempt
    }
}

/// Which mailbox generation refused to advance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MailboxGeneration {
    /// The command generation, advanced by every command.
    Command,
    /// The cancellation fence, advanced by attempt-invalidating rows.
    Cancellation,
    /// The world fence, advanced by every world change.
    World,
}

/// Why the mailbox refused a row or fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MailboxError {
    /// Remove or ProcessShutdown closed this slot lifetime; nothing re-arms
    /// it, and a replacement needs a new slot and a new mailbox.
    #[error("slot lifetime closed: a removed or shut-down slot cannot be re-armed")]
    Closed,
    /// A generation has no unused value left; the row was not applied and
    /// nothing changed.
    #[error("mailbox {0:?} generation exhausted: refusing to reuse a generation")]
    Exhausted(MailboxGeneration),
}

/// A mailbox generation: advanced by every command (today
/// `SlotIntent::generation`, `play_login.rs:110`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct IntentGeneration(u64);

impl IntentGeneration {
    /// The raw generation.
    pub const fn get(self) -> u64 {
        self.0
    }

    const fn next(self, which: MailboxGeneration) -> Result<Self, MailboxError> {
        match self.0.checked_add(1) {
            Some(next) => Ok(Self(next)),
            None => Err(MailboxError::Exhausted(which)),
        }
    }
}

/// A level-triggered one-shot command read from the mailbox, carrying the
/// generation it was read at and the cancellation fence (today
/// `IntentCommand`, `play_login.rs:127-131`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IntentCommand {
    generation: IntentGeneration,
    cancellation: IntentGeneration,
}

impl IntentCommand {
    /// The generation the command was read at.
    pub const fn generation(self) -> IntentGeneration {
        self.generation
    }

    /// The cancellation fence the command was read under.
    pub const fn cancellation(self) -> IntentGeneration {
        self.cancellation
    }
}

/// The current login want (today `want_login` with `auto_intent`,
/// `play_login.rs:115,118`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LoginIntent {
    /// No login wanted by intent (a script need may still want one).
    Withdrawn,
    /// One explicit operator login.
    Once,
    /// A login derived from the auto-login preference.
    Auto,
}

/// Why automatic and script-triggered login is latched off (today
/// `LoginLatchReason`, `play_login.rs:55-59`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LoginLatch {
    /// A clean operator Logout.
    OperatorLogout,
    /// A persisted logged-out hold.
    PersistedLogout,
    /// The repeat guard.
    RepeatGuard {
        /// Unexpected exits counted in the window.
        count: u8,
        /// The window, in seconds.
        window_seconds: u64,
    },
}

/// What a posted row did, for the poster's acknowledgement bookkeeping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PostReceipt {
    generation: IntentGeneration,
    invalidated_attempt: bool,
}

impl PostReceipt {
    /// The generation after the post; [`SessionEventOut::IntentApplied`]
    /// acknowledges it.
    pub const fn generation(self) -> IntentGeneration {
        self.generation
    }

    /// Whether the row advanced the cancellation fence, so an attempt read
    /// before it is no longer current. For an attempt-cancelling intent the
    /// poster fires the attempt it holds (§20 L4 item 1); for
    /// [`LifecycleFact::LoginSucceeded`] the attempt completed and is merely
    /// retired.
    pub const fn invalidated_attempt(self) -> bool {
        self.invalidated_attempt
    }
}

/// Whether an acknowledgement applied or was stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Acknowledgement {
    /// The command's outcome was recorded.
    Applied,
    /// A newer command changed the intent, or the lifetime is closed;
    /// nothing was recorded.
    Stale,
}

/// A read-only view of the mailbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntentSnapshot {
    generation: IntentGeneration,
    cancellation: IntentGeneration,
    login: LoginIntent,
    logout_wanted: bool,
    turnover: bool,
    latch: Option<LoginLatch>,
    error_hold: bool,
    auto_login: bool,
    script_needs_session: bool,
    world: WorldChoice,
    world_generation: u64,
    memory: Option<MemoryMode>,
    closed: bool,
}

impl IntentSnapshot {
    /// The generation of the last command.
    pub const fn generation(&self) -> IntentGeneration {
        self.generation
    }

    /// The cancellation fence.
    pub const fn cancellation(&self) -> IntentGeneration {
        self.cancellation
    }

    /// The login want by intent.
    pub const fn login(&self) -> LoginIntent {
        self.login
    }

    /// Whether a clean logout is wanted.
    pub const fn logout_wanted(&self) -> bool {
        self.logout_wanted
    }

    /// Whether a turnover's login half is still owed after its logout half.
    pub const fn turnover_pending(&self) -> bool {
        self.turnover
    }

    /// Why automatic login is latched off, if it is.
    pub const fn latch(&self) -> Option<LoginLatch> {
        self.latch
    }

    /// Whether a world-preference error holds the login.
    pub const fn error_hold(&self) -> bool {
        self.error_hold
    }

    /// The saved auto-login preference.
    pub const fn auto_login(&self) -> bool {
        self.auto_login
    }

    /// Whether the script seat needs a session.
    pub const fn script_needs_session(&self) -> bool {
        self.script_needs_session
    }

    /// The next handshake's world choice.
    pub const fn world(&self) -> WorldChoice {
        self.world
    }

    /// The world fence: advanced by every world-preference change.
    pub const fn world_generation(&self) -> u64 {
        self.world_generation
    }

    /// The queued memory mode, if a surface seeded one.
    pub const fn memory(&self) -> Option<MemoryMode> {
        self.memory
    }

    /// Whether Remove or ProcessShutdown closed this slot lifetime.
    pub const fn closed(&self) -> bool {
        self.closed
    }

    /// The level-triggered login want (today `SlotArm::wanted`,
    /// `play_login.rs:638-642`): the lifetime is open, nothing latched it,
    /// and either a login intent or a script need that no error hold
    /// suppresses.
    pub const fn login_wanted(&self) -> bool {
        !self.closed
            && self.latch.is_none()
            && (!matches!(self.login, LoginIntent::Withdrawn)
                || (!self.error_hold && self.script_needs_session))
    }

    /// Why the slot parks instead of logging in (today `relog_park_reason`,
    /// `play_login.rs:482-493`), or `None` while a login is wanted.
    pub const fn park_reason(&self) -> Option<ParkReason> {
        match self.latch {
            Some(LoginLatch::OperatorLogout) => Some(ParkReason::OperatorLogout),
            Some(LoginLatch::PersistedLogout) => Some(ParkReason::PersistedLogout),
            Some(LoginLatch::RepeatGuard { .. }) => Some(ParkReason::RepeatGuard),
            None if self.error_hold => Some(ParkReason::WorldError),
            None if self.login_wanted() => None,
            None => Some(ParkReason::NoIntent),
        }
    }
}

/// Why a slot sits on the title screen (today `RelogParkReason`,
/// `play_login.rs:84-90`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParkReason {
    /// Nothing wants a login.
    NoIntent,
    /// A clean operator Logout latched login off.
    OperatorLogout,
    /// A persisted logged-out hold.
    PersistedLogout,
    /// The repeat guard parked automatic retries.
    RepeatGuard,
    /// A world-preference error holds the login.
    WorldError,
}

/// The generation-stamped operator-intent mailbox: one per slot lifetime,
/// written by control threads and read by the slot, a pure reducer with the
/// row semantics of [`Intent::meaning`]. It holds no lock of another owner,
/// no attempt token and no clock; the owner that posts an attempt-cancelling
/// row fires the attempt it holds, and the owner's clock counts the repeat
/// guard. A refused row or fact changes nothing.
#[derive(Debug)]
pub struct IntentMailbox {
    generation: IntentGeneration,
    cancellation: IntentGeneration,
    login: LoginIntent,
    logout_wanted: bool,
    /// A turnover (Relog now, Switch world) owes its login half once its
    /// logout half is acknowledged or the session exits; a stronger row
    /// supersedes it.
    turnover: bool,
    latch: Option<LoginLatch>,
    error_hold: bool,
    auto_login: bool,
    script_needs_session: bool,
    world: WorldChoice,
    world_generation: u64,
    memory: Option<MemoryMode>,
    session_online: bool,
    work_reset_pending: bool,
    reconnect: bool,
    last_exit_clean: bool,
    closed: bool,
}

impl IntentMailbox {
    /// A mailbox for a spawned slot: `auto_login` is the saved preference and
    /// is also the spawn-time login want (today `SlotArm::new`,
    /// `play_login.rs:224-248`).
    pub const fn new(auto_login: bool, world: WorldChoice) -> Self {
        Self {
            generation: IntentGeneration(0),
            cancellation: IntentGeneration(0),
            login: if auto_login {
                LoginIntent::Auto
            } else {
                LoginIntent::Withdrawn
            },
            logout_wanted: false,
            turnover: false,
            latch: None,
            error_hold: false,
            auto_login,
            script_needs_session: false,
            world,
            world_generation: 0,
            memory: None,
            session_online: false,
            work_reset_pending: false,
            reconnect: false,
            last_exit_clean: true,
            closed: false,
        }
    }

    /// A read-only view.
    pub const fn snapshot(&self) -> IntentSnapshot {
        IntentSnapshot {
            generation: self.generation,
            cancellation: self.cancellation,
            login: self.login,
            logout_wanted: self.logout_wanted,
            turnover: self.turnover,
            latch: self.latch,
            error_hold: self.error_hold,
            auto_login: self.auto_login,
            script_needs_session: self.script_needs_session,
            world: self.world,
            world_generation: self.world_generation,
            memory: self.memory,
            closed: self.closed,
        }
    }

    /// Advance the command generation and, if `cancels`, the cancellation
    /// fence: both are computed before either is stored, so a refusal
    /// leaves both as they were.
    fn command(&mut self, cancels: bool) -> Result<(), MailboxError> {
        let generation = self.generation.next(MailboxGeneration::Command)?;
        let cancellation = if cancels {
            self.cancellation.next(MailboxGeneration::Cancellation)?
        } else {
            self.cancellation
        };
        self.generation = generation;
        self.cancellation = cancellation;
        Ok(())
    }

    /// Advance only the cancellation fence (today `invalidate_clean_exit`,
    /// `play_login.rs:881-885`).
    fn invalidate_attempt(&mut self) -> Result<(), MailboxError> {
        self.cancellation = self.cancellation.next(MailboxGeneration::Cancellation)?;
        Ok(())
    }

    const fn next_world_generation(&self) -> Result<u64, MailboxError> {
        match self.world_generation.checked_add(1) {
            Some(next) => Ok(next),
            None => Err(MailboxError::Exhausted(MailboxGeneration::World)),
        }
    }

    /// Re-derive the auto-login want after a hold lifts (today
    /// `release_error_hold`, `play_login.rs:575-589`).
    fn rederive_auto_want(&mut self) {
        if self.auto_login && self.latch.is_none() && matches!(self.login, LoginIntent::Withdrawn) {
            self.login = LoginIntent::Auto;
        }
    }

    /// Arm one explicit login and lift every latch, hold and pending
    /// turnover half (today `arm_explicit_login`, `play_login.rs:370-381`).
    fn arm_login(&mut self) {
        self.error_hold = false;
        self.latch = None;
        self.login = LoginIntent::Once;
        self.logout_wanted = false;
        self.turnover = false;
    }

    /// Begin a turnover: the logout half only while a session is online,
    /// then the login half; with no session it is the login alone.
    fn begin_turnover(&mut self) {
        self.arm_login();
        if self.session_online {
            self.logout_wanted = true;
            self.turnover = true;
        }
    }

    const fn receipt(&self, before: IntentGeneration) -> PostReceipt {
        PostReceipt {
            generation: self.generation,
            invalidated_attempt: self.cancellation.0 != before.0,
        }
    }

    /// Post one row. Session-side script rows (`StartScript`, `PauseScript`,
    /// `ResumeScript`, `StopScript`, `WatchdogRestart`) change no login
    /// intent: they are routed to the run owner and only acknowledged here.
    /// Once the lifetime is closed only another Remove or ProcessShutdown is
    /// accepted (and changes nothing); every other row is refused.
    pub fn post(&mut self, intent: Intent) -> Result<PostReceipt, MailboxError> {
        let before = self.cancellation;
        if self.closed {
            return match intent {
                Intent::RemoveSlot | Intent::ProcessShutdown => Ok(self.receipt(before)),
                _ => Err(MailboxError::Closed),
            };
        }
        match intent {
            Intent::Login => {
                self.command(false)?;
                self.arm_login();
            }
            Intent::AutoLogin(enabled) => {
                let withdraws = !enabled && self.login == LoginIntent::Auto;
                self.command(withdraws)?;
                self.auto_login = enabled;
                self.error_hold = false;
                if enabled {
                    self.rederive_auto_want();
                } else if withdraws {
                    self.login = LoginIntent::Withdrawn;
                }
            }
            Intent::Logout => {
                self.command(true)?;
                self.latch = Some(LoginLatch::OperatorLogout);
                self.logout_wanted = true;
                self.turnover = false;
                self.login = LoginIntent::Withdrawn;
                // A connected exit reads OperatorLogout from the latch; only
                // an already-offline command owes a held-work reset, and an
                // older unconsumed reset is kept.
                if !self.session_online {
                    self.work_reset_pending = true;
                }
            }
            Intent::HoldLoggedOut => {
                self.command(true)?;
                self.latch = Some(LoginLatch::PersistedLogout);
                self.login = LoginIntent::Withdrawn;
                self.logout_wanted = false;
                self.turnover = false;
            }
            Intent::WithdrawLogin => {
                self.command(true)?;
                self.login = LoginIntent::Withdrawn;
                self.turnover = false;
            }
            Intent::RelogNow => {
                self.command(false)?;
                self.begin_turnover();
            }
            Intent::WorldPreference(world) => {
                if self.world != world {
                    let world_generation = self.next_world_generation()?;
                    if self.error_hold {
                        self.command(false)?;
                    }
                    self.world = world;
                    self.world_generation = world_generation;
                    if self.error_hold {
                        self.error_hold = false;
                        self.rederive_auto_want();
                    }
                }
            }
            Intent::SwitchWorld(world) => {
                let world_generation = self.next_world_generation()?;
                self.command(true)?;
                self.world = WorldChoice::World(world);
                self.world_generation = world_generation;
                self.begin_turnover();
            }
            Intent::MemoryMode(mode) => {
                self.memory = Some(mode);
            }
            Intent::ScriptNeedsSession(needs) => {
                if needs != self.script_needs_session {
                    if !needs {
                        self.invalidate_attempt()?;
                    }
                    self.script_needs_session = needs;
                }
            }
            Intent::LoginIdentityChanged => {
                self.command(true)?;
            }
            Intent::StartScript
            | Intent::PauseScript
            | Intent::ResumeScript
            | Intent::StopScript
            | Intent::WatchdogRestart => {
                self.command(false)?;
            }
            Intent::RemoveSlot | Intent::ProcessShutdown => {
                self.command(true)?;
                self.closed = true;
                self.login = LoginIntent::Withdrawn;
                self.script_needs_session = false;
                self.turnover = false;
                // A connected slot gets the clean logout first; the owner
                // bounds that window and stops after the exit or its deadline.
                self.logout_wanted = self.session_online;
            }
        }
        Ok(self.receipt(before))
    }

    /// Reduce a lifecycle fact. Session facts are recorded for a closed
    /// lifetime too (its teardown reads them); guard and world facts no
    /// longer change a closed lifetime.
    pub fn observe(&mut self, fact: LifecycleFact) -> Result<PostReceipt, MailboxError> {
        let before = self.cancellation;
        match fact {
            LifecycleFact::LoginSucceeded => {
                self.invalidate_attempt()?;
                self.session_online = true;
                self.reconnect = true;
            }
            LifecycleFact::SessionExited { clean, resumable } => {
                self.session_online = false;
                self.last_exit_clean = clean;
                self.reconnect = resumable;
                if self.turnover {
                    // The logout half is done with the session; the login
                    // half is owed now.
                    self.turnover = false;
                    self.logout_wanted = false;
                    self.login = LoginIntent::Once;
                }
            }
            LifecycleFact::RepeatGuardTripped {
                count,
                window_seconds,
            } => {
                if !self.closed && self.latch.is_none() && !self.logout_wanted {
                    self.command(true)?;
                    self.latch = Some(LoginLatch::RepeatGuard {
                        count,
                        window_seconds,
                    });
                    self.login = LoginIntent::Withdrawn;
                }
            }
            LifecycleFact::WorldPreferenceFailed => {
                if !self.closed {
                    self.command(true)?;
                    self.login = LoginIntent::Withdrawn;
                    self.error_hold = true;
                    self.turnover = false;
                }
            }
        }
        Ok(self.receipt(before))
    }

    /// Whether a live session owns its one boundary decision.
    pub const fn session_online(&self) -> bool {
        self.session_online
    }

    /// Whether the next handshake reconnects (opcode 18) rather than logging
    /// in fresh (16): true after a grant until an exit that did not keep the
    /// world. A clean logout always leads to a fresh login.
    pub const fn reconnects(&self) -> bool {
        self.reconnect
    }

    /// Whether the next handshake may apply the queued memory mode: a dirty
    /// reconnect preserves the server session's applied mode (today
    /// `login_applies_memory`, `play_login.rs:455-457`).
    pub const fn login_applies_memory(&self) -> bool {
        !self.reconnect || self.last_exit_clean
    }

    /// Consume the held-work reset an offline Logout owes, once (today
    /// `take_logout_work_reset`, `play_login.rs:463-465`).
    pub fn take_work_reset(&mut self) -> bool {
        std::mem::replace(&mut self.work_reset_pending, false)
    }

    /// The login command while on the title screen, if a login is wanted
    /// (today `login_command`, `play_login.rs:717-723`, behind the sticky
    /// stop of `granted_permit_may_start_login`, `:945-949`).
    pub fn login_command(&self, online: bool) -> Option<IntentCommand> {
        (!online && self.snapshot().login_wanted()).then_some(IntentCommand {
            generation: self.generation,
            cancellation: self.cancellation,
        })
    }

    /// The logout command while in game, if a clean logout is wanted (today
    /// `logout_command`, `play_login.rs:725-731`); a closed lifetime still
    /// presses the clean logout its teardown wants.
    pub fn logout_command(&self, online: bool) -> Option<IntentCommand> {
        (online && self.logout_wanted).then_some(IntentCommand {
            generation: self.generation,
            cancellation: self.cancellation,
        })
    }

    /// Whether an in-flight attempt read as `command` is still current: the
    /// lifetime is open, a login is still wanted, and neither the
    /// cancellation nor the world fence moved (today
    /// `login_attempt_is_current`, `play_login.rs:807-817`).
    pub fn attempt_is_current(&self, command: IntentCommand, world_generation: u64) -> bool {
        self.snapshot().login_wanted()
            && self.cancellation == command.cancellation
            && self.world_generation == world_generation
    }

    /// Acknowledge a login the slot completed under `command`: an older
    /// generation applies only while the current intent still asks for a
    /// login; the want survives only as the auto-login preference (today
    /// `acknowledge_login`, `play_login.rs:793-803`). A closed lifetime
    /// records nothing.
    pub fn acknowledge_login(&mut self, command: IntentCommand) -> Acknowledgement {
        if self.closed {
            return Acknowledgement::Stale;
        }
        if self.generation != command.generation
            && (self.login == LoginIntent::Withdrawn || self.latch.is_some() || self.logout_wanted)
        {
            return Acknowledgement::Stale;
        }
        self.login = if self.auto_login && self.latch.is_none() {
            LoginIntent::Auto
        } else {
            LoginIntent::Withdrawn
        };
        Acknowledgement::Applied
    }

    /// Acknowledge a logout the slot pressed under `command`: an older
    /// generation applies only while a logout is still wanted. A Logout
    /// installs the OperatorLogout latch; a turnover whose login half was
    /// not superseded keeps its login armed instead (today
    /// `acknowledge_logout`, `play_login.rs:733-742`, and the login half of
    /// the memory relog, `session.rs:1139-1150`).
    pub fn acknowledge_logout(&mut self, command: IntentCommand) -> Acknowledgement {
        if self.generation != command.generation && !self.logout_wanted {
            return Acknowledgement::Stale;
        }
        self.logout_wanted = false;
        if std::mem::replace(&mut self.turnover, false) {
            self.login = LoginIntent::Once;
        } else {
            self.latch = Some(LoginLatch::OperatorLogout);
            self.login = LoginIntent::Withdrawn;
        }
        Acknowledgement::Applied
    }

    /// Whether a dropped connection is relogged: a login is still wanted and
    /// no logout is pending (today `relogs_after_drop`,
    /// `play_login.rs:609-612`).
    pub fn relogs_after_drop(&self) -> bool {
        !self.logout_wanted && self.snapshot().login_wanted()
    }

    /// Whether an exit keeps a running or paused run's session work for a
    /// later explicit Login: the script needs a session, no logout is
    /// pending, and neither an operator nor a persisted logout latched the
    /// slot; the repeat guard keeps the work (today
    /// `keeps_script_work_after_exit`, `play_login.rs:617-625`).
    pub fn keeps_run_work_after_exit(&self) -> bool {
        self.script_needs_session
            && !self.logout_wanted
            && !matches!(
                self.latch,
                Some(LoginLatch::OperatorLogout | LoginLatch::PersistedLogout)
            )
    }
}

/// The handshake a login attempt sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HandshakeKind {
    /// A fresh login (opcode 16).
    Fresh,
    /// A reconnect into a kept world (opcode 18).
    Reconnect,
}

/// Why a queued slot is not granted yet (today `host::login_queue::AdmitBlocker`
/// with `Instant`s; §20 L6 adds `Retiring`). The rows carry the merged
/// login-pacing rules as current facts; the numbers are T1's policy, not
/// part of the frozen vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QueueBlocker {
    /// Another bot's handshake has not returned: one handshake in flight at
    /// a time (today `max_in_flight: 1`, `login_queue.rs:186-195`).
    Handshake,
    /// The minimum gap after a prior login start (today `LOGIN_START_SPACING`
    /// = 2 server ticks = 1200 ms from the attempt's connect,
    /// `login_queue.rs:75-84`) or this slot's own clean-release wait (today
    /// a 5 s probe cadence inside a 30 s budget after its own clean logout,
    /// `play_login.rs:36-43`).
    Spacing {
        /// When the gap ends.
        until: Deadline,
    },
    /// The address reached the server's attempt limit (today 29 attempts
    /// per 60 s idle window, `login_queue.rs:614-620`).
    AddressBudget {
        /// Attempts counted against the limit.
        attempts: u32,
        /// When the budget frees, if known.
        until: Option<Deadline>,
    },
    /// This device id reached the server's per-device limit (today 4 grants
    /// then the remaining 15 s TTL, `login_queue.rs:103-109`).
    DeviceBudget {
        /// When the budget frees, if known.
        until: Option<Deadline>,
    },
    /// A previous attempt's socket is not yet acknowledged closed; no grant
    /// until it is (one handshake at a time).
    Retiring,
}

/// Why an attempt ended without a handshake outcome (§20 L7/L8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AbortedAttempt {
    /// The server answered after the lease ended; the slot rejected it.
    LateSuccess,
    /// An attempt-cancelling row fired.
    Cancelled,
    /// The absolute deadline passed.
    Deadline,
    /// Name resolution failed or timed out.
    Resolve,
}

/// Why a session ended, as a closed classification of the client's exit
/// reason (`client::client::client::SessionExitReason`) plus the two ends
/// the owner produces itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionEndCause {
    /// The server logged the client out: operator logout, world change or
    /// idle. The only clean end.
    ServerLogout,
    /// The read side reached end of stream.
    ReadEof,
    /// A read error.
    ReadError,
    /// A write error.
    WriteError,
    /// The WebSocket peer closed.
    WebSocketClose,
    /// A TLS error.
    TlsError,
    /// The dead-server deadline passed with no traffic.
    DeadServerDeadline,
    /// A T1 (unknown opcode) or T2 (decode) protocol error.
    ProtocolError,
    /// The connection was lost.
    ConnectionLost,
    /// The slot was removed or the process shut down.
    Removed,
    /// The process service the session depended on became unavailable
    /// (§26 §3.2 "Reactor becomes unavailable with a live session"): its
    /// network and session effect authority is invalidated and the condition
    /// is projected instead of Ready or recovery. Not clean, not resumable,
    /// never counted by the repeat guard: the retry is refused at admission
    /// until the process restarts, while the login intent is preserved.
    ServiceUnavailable {
        /// The latched condition.
        condition: ConditionId,
    },
}

impl SessionEndCause {
    /// Classify the client's exit reason (today `tick_flags`,
    /// `play_login.rs:1101-1103`: clean iff `ServerLogout`).
    pub fn classify(reason: &client::client::client::SessionExitReason) -> Self {
        use client::client::client::SessionExitReason as Exit;
        match reason {
            Exit::ServerLogout { .. } => Self::ServerLogout,
            Exit::ReadEof => Self::ReadEof,
            Exit::ReadError { .. } => Self::ReadError,
            Exit::WriteError { .. } => Self::WriteError,
            Exit::WebSocketClose { .. } => Self::WebSocketClose,
            Exit::TlsError { .. } => Self::TlsError,
            Exit::DeadServerDeadline { .. } => Self::DeadServerDeadline,
            Exit::ProtocolError { .. } => Self::ProtocolError,
            Exit::ConnectionLost => Self::ConnectionLost,
        }
    }

    /// Whether this end is clean: a server logout or the owner's own
    /// removal. Every other end is unexpected.
    pub const fn is_clean(self) -> bool {
        matches!(self, Self::ServerLogout | Self::Removed)
    }

    /// Whether the owner's repeat guard counts this end: every unexpected
    /// transport end does; a removal or a process-service outage never does.
    pub const fn counts_toward_repeat_guard(self) -> bool {
        !matches!(
            self,
            Self::ServerLogout | Self::Removed | Self::ServiceUnavailable { .. }
        )
    }
}

/// Why the slot holds its script while the session is otherwise usable.
/// Holds are a set: each reason is held once and released by its own
/// [`SessionEventOut::Released`], so a guardian hold outlives every other
/// release and no readiness event (§27 §6: the guardian hold takes
/// precedence, whatever owns the foreground).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HoldReason {
    /// The welcome modal is up.
    WelcomeModal,
    /// The random-event guardian acts (§27 §6: it freezes the seat whatever
    /// owns the foreground).
    Guardian,
    /// A session boundary is being applied.
    SessionBoundary,
}

impl HoldReason {
    /// Every reason, for exhaustive checks.
    pub const ALL: [HoldReason; 3] = [
        HoldReason::WelcomeModal,
        HoldReason::Guardian,
        HoldReason::SessionBoundary,
    ];

    const fn bit(self) -> u8 {
        match self {
            Self::WelcomeModal => 1,
            Self::Guardian => 2,
            Self::SessionBoundary => 4,
        }
    }
}

/// The one session boundary event every consumer reads (A D4 with B03's
/// envelope; §20 L8 `HandshakeAborted`; §26 §3.2 `Unavailable`). Carried as
/// [`crate::identity::Event`] with a slot scope, or a session scope for
/// `SceneReady`, `SessionEnded`, `Held` and `Released`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEventOut {
    /// Assets and cache prepared; the slot may queue.
    Prepared,
    /// The slot holds a FIFO place.
    Queued {
        /// 1-based place.
        place: u16,
        /// Queue length.
        total: u16,
        /// Why not granted yet, if blocked.
        blocker: Option<QueueBlocker>,
    },
    /// A handshake attempt started.
    AttemptStarted {
        /// Fresh login or reconnect.
        kind: HandshakeKind,
        /// The world selected, if pinned.
        world: Option<u16>,
    },
    /// The server accepted the handshake.
    HandshakeOk {
        /// Fresh login or reconnect.
        kind: HandshakeKind,
    },
    /// The server refused the handshake.
    HandshakeRefused {
        /// The login response code.
        code: u8,
        /// The retry the policy selected, if any.
        retry_after: Option<Duration>,
    },
    /// The attempt ended without a handshake outcome.
    HandshakeAborted {
        /// Why.
        reason: AbortedAttempt,
    },
    /// The slot's next step needs a capability the process lost (§26 §3.2):
    /// no attempt is started and none is retried before the process
    /// restarts; the login intent is preserved. Slot-scoped.
    Unavailable {
        /// The capability the step needed.
        capability: ProcessCapability,
        /// The latched condition that refused it.
        condition: ConditionId,
    },
    /// The scene is built and the local player seen: the one event that lets
    /// scripts tick. Session-scoped; idempotent for the current session and
    /// stale for any other.
    SceneReady,
    /// The session ended. Session-scoped.
    SessionEnded {
        /// Why.
        cause: SessionEndCause,
        /// The client kept its world, so the next handshake may reconnect.
        resumable: bool,
    },
    /// The script is held while the session stays up. Session-scoped.
    Held {
        /// Why.
        why: HoldReason,
    },
    /// A hold lifted: only the hold of the same reason. Session-scoped.
    Released {
        /// Which hold.
        why: HoldReason,
    },
    /// The owner applied the mailbox generation; the frontend settles its
    /// pending operation on this acknowledgement.
    IntentApplied {
        /// The acknowledged generation.
        generation: IntentGeneration,
    },
}

/// Why the session gate refused to begin a session or observe an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SessionGateError {
    /// The session belongs to another slot.
    #[error("session {offered:?} belongs to another slot; this gate serves {expected:?}")]
    WrongSlot {
        /// The slot this gate serves.
        expected: SlotKey,
        /// The session offered.
        offered: SessionKey,
    },
    /// The session's epoch is not newer than the newest one begun: an ended
    /// or superseded session never begins again.
    #[error("session {offered:?} is not newer than {latest:?}; an ended or superseded epoch cannot begin again")]
    ObsoleteEpoch {
        /// The newest session begun.
        latest: SessionKey,
        /// The session offered.
        offered: SessionKey,
    },
    /// The event is stamped with a session that is not the current one
    /// (ended, superseded, never begun or still to begin); it changed
    /// nothing.
    #[error("event stamped {stamped:?} is stale; the current session is {current:?}")]
    StaleSession {
        /// The current session, if one is begun and not ended.
        current: Option<SessionKey>,
        /// The session the event carried.
        stamped: SessionKey,
    },
}

/// Derives "the script may tick" from session events: only between
/// `SceneReady` and `SessionEnded` of the session the owner authorized with
/// [`begin`](Self::begin), and only while no hold is set. The owner's
/// authorization is the fence ([`crate::identity::Fence`] over
/// [`SessionKey`]); no arriving event changes which session is current, and
/// an event stamped with any other session is stale and changes nothing.
#[derive(Debug)]
pub struct SessionGate {
    slot: SlotKey,
    /// The newest session ever begun: an ended or superseded epoch never
    /// begins again.
    latest: Option<SessionKey>,
    current: Fence<SessionKey>,
    ready: bool,
    holds: u8,
}

impl SessionGate {
    /// A gate for `slot` with no session begun.
    pub const fn new(slot: SlotKey) -> Self {
        Self {
            slot,
            latest: None,
            current: Fence::new(),
            ready: false,
            holds: 0,
        }
    }

    /// The slot this gate serves.
    pub const fn slot(&self) -> SlotKey {
        self.slot
    }

    /// Authorize `session` as the current one: the owner calls this at the
    /// session boundary that minted the key ([`SessionKey::next`]). Refused
    /// for another slot or an epoch not newer than the newest begun. A
    /// session still current is superseded (its late events become stale);
    /// readiness and holds start over.
    pub fn begin(&mut self, session: SessionKey) -> Result<(), SessionGateError> {
        if session.slot() != self.slot {
            return Err(SessionGateError::WrongSlot {
                expected: self.slot,
                offered: session,
            });
        }
        if let Some(latest) = self.latest {
            if session.epoch() <= latest.epoch() {
                return Err(SessionGateError::ObsoleteEpoch {
                    latest,
                    offered: session,
                });
            }
        }
        self.latest = Some(session);
        self.current.arm(session);
        self.ready = false;
        self.holds = 0;
        Ok(())
    }

    /// Reduce one event stamped with `session`. Only the current session's
    /// events apply: `SceneReady` makes it ready (again, without touching
    /// holds); `SessionEnded` ends it for good; `Held`/`Released` set and
    /// clear the hold of their reason; every other event changes nothing.
    pub fn observe(
        &mut self,
        session: SessionKey,
        event: &SessionEventOut,
    ) -> Result<(), SessionGateError> {
        self.current
            .admit(session)
            .map_err(|rejected| SessionGateError::StaleSession {
                current: rejected.current(),
                stamped: session,
            })?;
        match event {
            SessionEventOut::SceneReady => self.ready = true,
            SessionEventOut::SessionEnded { .. } => {
                self.current.revoke();
                self.ready = false;
                self.holds = 0;
            }
            SessionEventOut::Held { why } => self.holds |= why.bit(),
            SessionEventOut::Released { why } => self.holds &= !why.bit(),
            SessionEventOut::Prepared
            | SessionEventOut::Queued { .. }
            | SessionEventOut::AttemptStarted { .. }
            | SessionEventOut::HandshakeOk { .. }
            | SessionEventOut::HandshakeRefused { .. }
            | SessionEventOut::HandshakeAborted { .. }
            | SessionEventOut::Unavailable { .. }
            | SessionEventOut::IntentApplied { .. } => {}
        }
        Ok(())
    }

    /// The session the owner authorized and has not ended, ready or not.
    pub const fn current(&self) -> Option<SessionKey> {
        self.current.current()
    }

    /// The session scripts may tick in, if any.
    pub const fn ready(&self) -> Option<SessionKey> {
        if self.ready && self.holds == 0 {
            self.current.current()
        } else {
            None
        }
    }

    /// Whether the current session is held for any reason.
    pub const fn held(&self) -> bool {
        self.current.current().is_some() && self.holds != 0
    }

    /// Whether the current session is held for `why`.
    pub const fn held_by(&self, why: HoldReason) -> bool {
        self.current.current().is_some() && self.holds & why.bit() != 0
    }
}

/// The run's phase (today `script::RunState`, `crates/script/src/slot.rs:66-73`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunPhase {
    /// No run.
    Idle,
    /// Preparation or Load setup.
    Starting,
    /// Ticking when the session gate allows.
    Running,
    /// Operator pause.
    Paused,
    /// Bounded stop and disposal.
    Stopping,
    /// A failed run retained for its diagnostic.
    Error,
}

/// The session hold on a run: distinct from operator pause. Running stays
/// Running plus `SessionHeld`, Paused stays Paused plus `SessionHeld`; both
/// clocks and effects stay held until the next Ready. Only Stop ends the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionHeld {
    since: Option<SessionKey>,
}

impl SessionHeld {
    /// A hold since the end of `since`, if the session is known.
    pub const fn new(since: Option<SessionKey>) -> Self {
        Self { since }
    }

    /// The session whose end holds the run, if it is known.
    pub const fn since(self) -> Option<SessionKey> {
        self.since
    }
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
