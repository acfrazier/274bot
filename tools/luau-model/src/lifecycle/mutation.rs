//! Seeded earlier designs, found defects and self-checks (ADR 0017 point 2).
//! The head is `None`. A mutation changes transitions only: the invariants
//! are frozen, and each mutation must make one of them fail
//! (`Mutation::caught_by`, tested in `tests/model.rs`).
//!
//! * `LD1 6.4` rows: the earlier designs that LD1's reviews rejected. Round
//!   one and round two are in the order of that table; the last ten are the
//!   third set (the reviews of round two).
//! * `LD1 rules`: a rule of LD1 that 6.4 has no row for (rule X8 step 2, the
//!   refusal of a self-stopped run, nothing due, the `onStop` entry). Each
//!   mutation breaks the rule in the way a defect would.
//! * `LD2`: the rejected designs of the runner side that touch the run
//!   lifecycle (the dock, the entry fence, the execution ledger).
//! * `F*`: a gap that this model found in LD1 (README, "Findings"): the
//!   design taken as written, which the head resolves.
//! * `Check*`: a builder's check that an invariant can fail.

/// One mutation. The doc line says what the earlier design did.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Mutation {
    #[default]
    None,

    // ---- LD1 6.4 ----
    /// `stop` runs `onStop` after a trip.
    OnStopAfterTrip,
    /// `api.stop` runs `onStop` at once, inside the dispatch.
    OnStopInDispatch,
    /// A seat-page dispatch resumes a paused run.
    SeatPageIgnoresPause,
    /// The deadline is not cleared when its owner ends: a Hook entry or a
    /// later dispatch gets the old deadline.
    DeadlineCarries,
    /// The run fails only when the error reaches the host. A script catches
    /// it with `pcall` and goes on.
    VerdictFromError,
    /// A tick dispatch and a seat-page dispatch both deliver one completion.
    ResumeTwice,
    /// A completion that arrives during Pause is dropped.
    LostWake,
    /// Rows of the last dispatch are drained after the Stop.
    RowsAfterStop,
    /// A game tick starts `tick` while an invocation is suspended.
    ReenterTick,
    /// `done` gets a message before `onStop` and one after it.
    DoneTwice,
    /// A slot restart spawns the new VM before the old `stop` returned.
    SpawnBeforeRetire,
    /// The interrupt reads the clock at every 256th call only; no exit
    /// check. An entry passes its deadline and returns before the next read.
    SampledClock,
    /// No check at the exit. An entry passes its deadline inside one
    /// interrupt-free call and then ends with an error.
    NoExitCheck,
    /// A binding queues a row after the latch is set, in a script that
    /// caught the trip error.
    EffectAfterTrip,
    /// The slot drains the rows of a dispatch before it handles `Failed`.
    DrainBeforeFailure,
    /// A refused allocation sets the latch only when the error reaches the
    /// host. A script catches it in `pcall`.
    MemoryVerdictFromError,
    /// The run keeps a dispatch failure for a later read. A Stop comes
    /// first, and the failure is lost.
    PendingFailure,
    /// `stop` sends only log lines. A trip inside `onStop` leaves no trace.
    HookFailureLost,
    /// A trip inside `onStop` writes a `Failed` receipt over the `Stopped`
    /// receipt of `api.stop`.
    HookFailureReplacesReceipt,
    /// A Stop of a paused run does not run `onStop`.
    HookSkippedWhilePaused,
    /// A paused run runs `onPaint`.
    PausedPaint,
    /// The top level of the file runs with no deadline and no cap.
    LoadUngated,
    /// A failed load returns its error and keeps the VM.
    FailedLoadLeak,
    /// A held tick resumes the invocation.
    HeldTickResumes,
    /// A held tick is accepted and the run does not store the completion.
    AckWithoutStore,
    /// The host acknowledges a delivery that the run refused.
    AckOnRefusal,
    /// The lent path passes the host-built facts without the slot pages.
    LentSkipsSeatPage,
    /// The Paint entry arms a new deadline. A Tick entry and a Paint entry
    /// that each stay inside the budget are both accepted.
    PaintRearmsDeadline,
    /// The check runs only at an interrupt. An entry passes its deadline in
    /// straight-line code and calls a binding before the next interrupt.
    GapEffect,
    /// The guarded `xpcall` wraps the handler of the VM's `xpcall`: a refusal
    /// at the C-call limit is replaced by "error in error handling".
    LostRefusal,
    /// The run enters a Paint entry through a protected call with an error
    /// function, not through a resume.
    EntryThroughCall,
    /// A binding returns the lost-error value of `mlua` to the script as an
    /// ordinary error.
    LostErrorIgnored,
    /// `api.stop` sets the fact and returns to the script.
    StopContinues,
    /// A watchdog trip after `api.stop` in the same entry replaces the
    /// self-stop in the latch.
    LatchOverwritten,
    /// `complete_stop` ignores a Start that was queued while the slot was
    /// `Stopping`.
    QueuedStartLost,
    /// A second Stop while the slot is `Stopping` does not drop the queued
    /// Start.
    StopKeepsQueuedStart,
    /// An awaited family request resumes with the seat end and no typed
    /// outcome.
    BareSeatEnd,

    // ---- LD1 rules ----
    /// An action binding is legal in `onStop` (rule X8 step 2 makes it an
    /// error): its row is queued after the Stop began.
    HookActionLegal,
    /// A run that called `api.stop` accepts the next dispatch (I10: it
    /// refuses each dispatch until the slot stops it).
    SelfStopAcceptsDispatch,
    /// A game tick resumes a suspended invocation whether or not its
    /// completion is in the delivery (I6: a dispatch with nothing due does not
    /// enter the VM).
    ResumeWithoutCompletion,
    /// `stop` runs `onStop` for a script that has no such entry (I5).
    HookRunsWithoutEntry,
    /// `stop` runs `onStop` without the slot mutex (rule X2 makes every step
    /// atomic): a dispatch enters the VM while the hook runs.
    HookOutsideMutex,
    /// The Hook entry arms no deadline of its own (I7 a).
    HookUngated,

    // ---- found by this model ----
    /// F1: a waiting binding in a Paint entry starts its request and yields.
    /// LD1 6.1 lists "yield at a wait" for a Paint entry and no rule says what
    /// the run does with it. The head refuses the wait in a Paint entry.
    F1PaintWaitSuspends,
    /// F2: an operator Stop does not read the latch of a run that called
    /// `api.stop`; only the log drain writes the `Stopped` receipt.
    F2StopIgnoresSelfStop,
    /// F3: the progress watchdog restarts a run that called `api.stop` and
    /// the replacement starts.
    F3RestartIgnoresSelfStop,

    // ---- LD2: the dock, the fence, the ledger ----
    /// A retained Start used by a watchdog restart claims the old dock again.
    SharedRestartDock,
    /// `complete` writes over a payload that is in the dock.
    CompletionOverwrite,
    /// One payload resumes two waits.
    ConsumeTwice,
    /// A paused run consumes the extension payload.
    ConsumeWhilePaused,
    /// A held dispatch consumes the extension payload.
    HeldConsumes,
    /// A refused dispatch drops the extension payload.
    RefusedDispatchLosesPayload,
    /// A failed dispatch commits its staged calls.
    CommitFailedDispatch,
    /// A session reset ends the extension wait.
    SessionCancelsExtensionWait,
    /// The dock admits a call after it closed.
    CallsAfterClose,
    /// The end record leaves out the failure of `onStop`.
    HookFailureNotInEnd,
    /// A refused extension call in `onStop` leaves no first fault when the
    /// script catches the error.
    CaughtHookRefusalLost,
    /// `close` builds the end record without the first fault.
    UnobservedRefusalLost,
    /// Revocation leaves the completion payload in the dock.
    RevokeKeepsPayload,
    /// The runner decides that a Stop is its own from `generation == old + 1`.
    GuessedStopGeneration,
    /// The final gate accepts a replacement run on the old run's clean end.
    ReplacementWithOldEnd,
    /// The final gate compares the live generation with the ticket's. A
    /// failed queued setup gives the generation back.
    FailedReplacementRestoresAuthorization,
    /// The ledger is one mutable field: a later admission forgets the
    /// earlier execution.
    ForgetEarlierExecution,
    /// The runner issues a second Stop for an execution that a ticketed Stop
    /// already retired. It lands on the replacement.
    RepeatLastTableStop,
    /// The ledger records an admission at Ready, not at the Start.
    LoadingMissingFromLedger,
    /// An entry enters between the published fence and the Stop.
    WakeBetweenFreezeAndStop,
    /// The seat admits a row of a fenced run.
    PostFreezeAction,
    /// The fence waits for a busy slot mutex while it holds the runner lock.
    BlockingFleetLock,
    /// The runner issues the Stop inside its own closure, under its lock.
    StopUnderRunnerLock,
    /// File Start, compile and Load run under the runner lock.
    FileCompileUnderRunner,
    /// File Start, compile and Load hold the shared admission guard.
    FileCompileHoldsSharedAdmission,

    // ---- self-checks ----
    /// Check: a gap longer than `R_gap` is allowed.
    CheckGapUnbounded,
    /// Check: a second `onStop` can start.
    CheckHookTwice,
    /// Check: a Pause does not stop the dispatch from entering the VM.
    CheckPauseNoRefusal,
    /// Check: the completion that `stop` sends never reaches the slot, so the
    /// slot stays Stopping.
    CheckDoneDropped,
    /// Check: a stopped run stays in the slot.
    CheckStopKeepsRun,
    /// Check: a removed slot keeps the VM of its run.
    CheckRemoveKeepsVm,
}

impl Mutation {
    pub const ALL: [Mutation; 78] = [
        Mutation::None,
        Mutation::OnStopAfterTrip,
        Mutation::OnStopInDispatch,
        Mutation::SeatPageIgnoresPause,
        Mutation::DeadlineCarries,
        Mutation::VerdictFromError,
        Mutation::ResumeTwice,
        Mutation::LostWake,
        Mutation::RowsAfterStop,
        Mutation::ReenterTick,
        Mutation::DoneTwice,
        Mutation::SpawnBeforeRetire,
        Mutation::SampledClock,
        Mutation::NoExitCheck,
        Mutation::EffectAfterTrip,
        Mutation::DrainBeforeFailure,
        Mutation::MemoryVerdictFromError,
        Mutation::PendingFailure,
        Mutation::HookFailureLost,
        Mutation::HookFailureReplacesReceipt,
        Mutation::HookSkippedWhilePaused,
        Mutation::PausedPaint,
        Mutation::LoadUngated,
        Mutation::FailedLoadLeak,
        Mutation::HeldTickResumes,
        Mutation::AckWithoutStore,
        Mutation::AckOnRefusal,
        Mutation::LentSkipsSeatPage,
        Mutation::PaintRearmsDeadline,
        Mutation::GapEffect,
        Mutation::LostRefusal,
        Mutation::EntryThroughCall,
        Mutation::LostErrorIgnored,
        Mutation::StopContinues,
        Mutation::LatchOverwritten,
        Mutation::QueuedStartLost,
        Mutation::StopKeepsQueuedStart,
        Mutation::BareSeatEnd,
        Mutation::HookActionLegal,
        Mutation::F1PaintWaitSuspends,
        Mutation::F2StopIgnoresSelfStop,
        Mutation::F3RestartIgnoresSelfStop,
        Mutation::SharedRestartDock,
        Mutation::CompletionOverwrite,
        Mutation::ConsumeTwice,
        Mutation::ConsumeWhilePaused,
        Mutation::HeldConsumes,
        Mutation::RefusedDispatchLosesPayload,
        Mutation::CommitFailedDispatch,
        Mutation::SessionCancelsExtensionWait,
        Mutation::CallsAfterClose,
        Mutation::HookFailureNotInEnd,
        Mutation::CaughtHookRefusalLost,
        Mutation::UnobservedRefusalLost,
        Mutation::RevokeKeepsPayload,
        Mutation::GuessedStopGeneration,
        Mutation::ReplacementWithOldEnd,
        Mutation::FailedReplacementRestoresAuthorization,
        Mutation::ForgetEarlierExecution,
        Mutation::RepeatLastTableStop,
        Mutation::LoadingMissingFromLedger,
        Mutation::WakeBetweenFreezeAndStop,
        Mutation::PostFreezeAction,
        Mutation::BlockingFleetLock,
        Mutation::StopUnderRunnerLock,
        Mutation::FileCompileUnderRunner,
        Mutation::FileCompileHoldsSharedAdmission,
        Mutation::CheckGapUnbounded,
        Mutation::CheckHookTwice,
        Mutation::CheckPauseNoRefusal,
        Mutation::SelfStopAcceptsDispatch,
        Mutation::ResumeWithoutCompletion,
        Mutation::HookRunsWithoutEntry,
        Mutation::HookOutsideMutex,
        Mutation::HookUngated,
        Mutation::CheckDoneDropped,
        Mutation::CheckStopKeepsRun,
        Mutation::CheckRemoveKeepsVm,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Mutation::None => "none",
            Mutation::OnStopAfterTrip => "on-stop-after-trip",
            Mutation::OnStopInDispatch => "on-stop-in-dispatch",
            Mutation::SeatPageIgnoresPause => "seat-page-ignores-pause",
            Mutation::DeadlineCarries => "deadline-carries",
            Mutation::VerdictFromError => "verdict-from-error",
            Mutation::ResumeTwice => "resume-twice",
            Mutation::LostWake => "lost-wake",
            Mutation::RowsAfterStop => "rows-after-stop",
            Mutation::ReenterTick => "reenter-tick",
            Mutation::DoneTwice => "done-twice",
            Mutation::SpawnBeforeRetire => "spawn-before-retire",
            Mutation::SampledClock => "sampled-clock",
            Mutation::NoExitCheck => "no-exit-check",
            Mutation::EffectAfterTrip => "effect-after-trip",
            Mutation::DrainBeforeFailure => "drain-before-failure",
            Mutation::MemoryVerdictFromError => "memory-verdict-from-error",
            Mutation::PendingFailure => "pending-failure",
            Mutation::HookFailureLost => "hook-failure-lost",
            Mutation::HookFailureReplacesReceipt => "hook-failure-replaces-receipt",
            Mutation::HookSkippedWhilePaused => "hook-skipped-while-paused",
            Mutation::PausedPaint => "paused-paint",
            Mutation::LoadUngated => "load-ungated",
            Mutation::FailedLoadLeak => "failed-load-leak",
            Mutation::HeldTickResumes => "held-tick-resumes",
            Mutation::AckWithoutStore => "ack-without-store",
            Mutation::AckOnRefusal => "ack-on-refusal",
            Mutation::LentSkipsSeatPage => "lent-skips-seat-page",
            Mutation::PaintRearmsDeadline => "paint-rearms-deadline",
            Mutation::GapEffect => "gap-effect",
            Mutation::LostRefusal => "lost-refusal",
            Mutation::EntryThroughCall => "entry-through-call",
            Mutation::LostErrorIgnored => "lost-error-ignored",
            Mutation::StopContinues => "stop-continues",
            Mutation::LatchOverwritten => "latch-overwritten",
            Mutation::QueuedStartLost => "queued-start-lost",
            Mutation::StopKeepsQueuedStart => "stop-keeps-queued-start",
            Mutation::BareSeatEnd => "bare-seat-end",
            Mutation::HookActionLegal => "hook-action-legal",
            Mutation::F1PaintWaitSuspends => "f1-paint-wait-suspends",
            Mutation::F2StopIgnoresSelfStop => "f2-stop-ignores-self-stop",
            Mutation::F3RestartIgnoresSelfStop => "f3-restart-ignores-self-stop",
            Mutation::SharedRestartDock => "shared-restart-dock",
            Mutation::CompletionOverwrite => "completion-overwrite",
            Mutation::ConsumeTwice => "consume-twice",
            Mutation::ConsumeWhilePaused => "consume-while-paused",
            Mutation::HeldConsumes => "held-consumes",
            Mutation::RefusedDispatchLosesPayload => "refused-dispatch-loses-payload",
            Mutation::CommitFailedDispatch => "commit-failed-dispatch",
            Mutation::SessionCancelsExtensionWait => "session-cancels-extension-wait",
            Mutation::CallsAfterClose => "calls-after-close",
            Mutation::HookFailureNotInEnd => "hook-failure-not-in-end",
            Mutation::CaughtHookRefusalLost => "caught-hook-refusal-lost",
            Mutation::UnobservedRefusalLost => "unobserved-refusal-lost",
            Mutation::RevokeKeepsPayload => "revoke-keeps-payload",
            Mutation::GuessedStopGeneration => "guessed-stop-generation",
            Mutation::ReplacementWithOldEnd => "replacement-with-old-end",
            Mutation::FailedReplacementRestoresAuthorization => {
                "failed-replacement-restores-authorization"
            }
            Mutation::ForgetEarlierExecution => "forget-earlier-execution",
            Mutation::RepeatLastTableStop => "repeat-last-table-stop",
            Mutation::LoadingMissingFromLedger => "loading-missing-from-ledger",
            Mutation::WakeBetweenFreezeAndStop => "wake-between-freeze-and-stop",
            Mutation::PostFreezeAction => "post-freeze-action",
            Mutation::BlockingFleetLock => "blocking-fleet-lock",
            Mutation::StopUnderRunnerLock => "stop-under-runner-lock",
            Mutation::FileCompileUnderRunner => "file-compile-under-runner",
            Mutation::FileCompileHoldsSharedAdmission => "file-compile-holds-shared-admission",
            Mutation::CheckGapUnbounded => "check-gap-unbounded",
            Mutation::CheckHookTwice => "check-hook-twice",
            Mutation::CheckPauseNoRefusal => "check-pause-no-refusal",
            Mutation::SelfStopAcceptsDispatch => "self-stop-accepts-dispatch",
            Mutation::ResumeWithoutCompletion => "resume-without-completion",
            Mutation::HookRunsWithoutEntry => "hook-runs-without-entry",
            Mutation::HookOutsideMutex => "hook-outside-mutex",
            Mutation::HookUngated => "hook-ungated",
            Mutation::CheckDoneDropped => "check-done-dropped",
            Mutation::CheckStopKeepsRun => "check-stop-keeps-run",
            Mutation::CheckRemoveKeepsVm => "check-remove-keeps-vm",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }

    /// The invariant that must catch this mutation.
    pub fn caught_by(self) -> Option<&'static str> {
        Some(match self {
            Mutation::None => return None,
            Mutation::OnStopAfterTrip => "i5-hook-after-trip",
            Mutation::OnStopInDispatch => "i5-hook-in-dispatch",
            Mutation::SeatPageIgnoresPause => "i2-entry-while-paused",
            Mutation::DeadlineCarries => "i7d-deadline-outlives-owner",
            Mutation::VerdictFromError => "i7a-late-accept",
            Mutation::ResumeTwice => "i6-resume-twice",
            Mutation::LostWake => "i3-completion-lost",
            Mutation::RowsAfterStop => "i4-row-after-stop",
            Mutation::ReenterTick => "i6-two-invocations",
            Mutation::DoneTwice => "i4-done-count",
            Mutation::SpawnBeforeRetire => "i1-two-vms",
            Mutation::SampledClock => "i7a-late-accept",
            Mutation::NoExitCheck => "i7a-late-accept",
            Mutation::EffectAfterTrip => "i7b-effect-after-trip",
            Mutation::DrainBeforeFailure => "i7b-rows-of-failed-dispatch-admitted",
            Mutation::MemoryVerdictFromError => "i8b-cap-event-untripped",
            Mutation::PendingFailure => "i11a-failure-pending",
            Mutation::HookFailureLost => "i12-failure-lost",
            Mutation::HookFailureReplacesReceipt => "i12-receipt-replaced",
            Mutation::HookSkippedWhilePaused => "i5-hook-skipped",
            Mutation::PausedPaint => "i2-entry-while-paused",
            Mutation::LoadUngated => "i7a-late-load",
            Mutation::FailedLoadLeak => "i1-vm-outlives-run",
            Mutation::HeldTickResumes => "i13-held-resumes",
            Mutation::AckWithoutStore => "i13-held-not-stored",
            Mutation::AckOnRefusal => "i3-completion-lost",
            Mutation::LentSkipsSeatPage => "p2-completion-not-delivered",
            Mutation::PaintRearmsDeadline => "i7a-late-accept",
            Mutation::GapEffect => "i7b-effect-after-trip",
            Mutation::LostRefusal => "i8b-cap-event-untripped",
            Mutation::EntryThroughCall => "i8b-cap-event-untripped",
            Mutation::LostErrorIgnored => "i8b-cap-event-untripped",
            Mutation::StopContinues => "i11c-effect-after-stop",
            Mutation::LatchOverwritten => "i11b-latch-overwritten",
            Mutation::QueuedStartLost => "p1-queued-start-lost",
            Mutation::StopKeepsQueuedStart => "p1-stop-kept-queued-start",
            Mutation::BareSeatEnd => "i6-bare-completion",
            Mutation::HookActionLegal => "i4-row-after-stop",
            Mutation::F1PaintWaitSuspends => "i6-paint-suspends",
            Mutation::F2StopIgnoresSelfStop => "i11b-wrong-receipt",
            Mutation::F3RestartIgnoresSelfStop => "i11b-selfstop-overridden",
            Mutation::SharedRestartDock => "d-dock-shared",
            Mutation::CompletionOverwrite => "d-completion-overwritten",
            Mutation::ConsumeTwice => "d-consume-twice",
            Mutation::ConsumeWhilePaused => "d-consume-not-allowed",
            Mutation::HeldConsumes => "i13-held-resumes",
            Mutation::RefusedDispatchLosesPayload => "d-payload-lost",
            Mutation::CommitFailedDispatch => "d-failed-dispatch-committed",
            Mutation::SessionCancelsExtensionWait => "d-session-cancels-extension-wait",
            Mutation::CallsAfterClose => "d-call-after-close",
            Mutation::HookFailureNotInEnd => "d-end-lacks-hook-failure",
            Mutation::CaughtHookRefusalLost => "d-refusal-not-latched",
            Mutation::UnobservedRefusalLost => "d-first-fault-lost",
            Mutation::RevokeKeepsPayload => "d-revoke-keeps-payload",
            Mutation::GuessedStopGeneration => "l-pass-unsound",
            Mutation::ReplacementWithOldEnd => "l-pass-unsound",
            Mutation::FailedReplacementRestoresAuthorization => "l-pass-unsound",
            Mutation::ForgetEarlierExecution => "l-ledger-incomplete",
            Mutation::RepeatLastTableStop => "l-stop-unowned",
            Mutation::LoadingMissingFromLedger => "l-ledger-incomplete",
            Mutation::WakeBetweenFreezeAndStop => "f-entry-after-fence",
            Mutation::PostFreezeAction => "f-row-after-fence",
            Mutation::BlockingFleetLock => "f-lock-wait",
            Mutation::StopUnderRunnerLock => "l-vm-under-runner-lock",
            Mutation::FileCompileUnderRunner => "l-vm-under-runner-lock",
            Mutation::FileCompileHoldsSharedAdmission => "l-load-under-admission",
            Mutation::CheckGapUnbounded => "i7c-gap-bound",
            Mutation::CheckHookTwice => "i5-hook-twice",
            Mutation::CheckPauseNoRefusal => "i2-entry-while-paused",
            Mutation::SelfStopAcceptsDispatch => "i10-entry-not-allowed",
            Mutation::ResumeWithoutCompletion => "i6-resume-without-completion",
            Mutation::HookRunsWithoutEntry => "i5-hook-unexpected",
            Mutation::HookOutsideMutex => "i4-entry-after-stop",
            Mutation::HookUngated => "i7a-late-hook",
            Mutation::CheckDoneDropped => "p1-stuck-stopping",
            Mutation::CheckStopKeepsRun => "i9-idle-with-run",
            Mutation::CheckRemoveKeepsVm => "i4-vm-after-stop",
        })
    }
}
