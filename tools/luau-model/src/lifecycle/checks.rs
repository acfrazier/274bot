//! The checks of the model, by name (ADR 0017: the invariants are explicit and
//! frozen, and a mutation must make one of them fail).
//!
//! Each check belongs to one invariant (`I1` to `I13`) or progress property
//! (`P1`, `P2`) of LD1 section 6.2, or to one rule of the runner side of LD2
//! (`D` for the dock, `F` for the entry fence, `L` for the ledger and the
//! final gate). A check is raised either by the step that breaks it, or by
//! `check_state`, which reads the state after every step.
//!
//! `Lifecycle::violation` accepts no name that is not in [`CHECKS`] (a debug
//! assertion). `tests/model.rs` requires that a mutation fails each check, and
//! that README.md names each one.

/// One check of the model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Check {
    pub name: &'static str,
    /// The invariant or rule: `I1` to `I13` with the part (`I7 a`), `P1`,
    /// `P2`, `D`, `F` or `L`.
    pub rule: &'static str,
    /// The condition that the check refuses.
    pub refuses: &'static str,
}

const fn check(name: &'static str, rule: &'static str, refuses: &'static str) -> Check {
    Check {
        name,
        rule,
        refuses,
    }
}

pub const CHECKS: [Check; 61] = [
    check(
        "i1-two-vms",
        "I1",
        "More than one VM exists: the run's, the Load entry's and any that a failed spawn kept.",
    ),
    check(
        "i1-vm-outlives-run",
        "I1",
        "A failed spawn returned its error and kept its VM.",
    ),
    check(
        "i2-entry-while-paused",
        "I2",
        "A Tick or a Paint entry started while the run is paused.",
    ),
    check(
        "i3-completion-lost",
        "I3",
        "The completion of the awaited request arrived and neither the host nor the run holds it, or the host acknowledged a completion that the run neither stored nor used.",
    ),
    check(
        "i4-done-count",
        "I4",
        "`stop` sent a number of messages on `done` other than one.",
    ),
    check(
        "i4-entry-after-stop",
        "I4",
        "A Tick or a Paint entry started after `stop` began.",
    ),
    check(
        "i4-row-after-stop",
        "I4",
        "A row of the run reached the seat after the Stop began.",
    ),
    check(
        "i4-vm-after-stop",
        "I4",
        "The slot is gone and a run, a VM or a section is left.",
    ),
    check("i5-hook-after-trip", "I5", "`onStop` ran after a trip."),
    check(
        "i5-hook-in-dispatch",
        "I5",
        "`api.stop` ran `onStop` inside the dispatch.",
    ),
    check(
        "i5-hook-skipped",
        "I5",
        "`onStop` did not run for a script that loaded, has the entry and did not trip or fail.",
    ),
    check(
        "i5-hook-twice",
        "I5",
        "`onStop` started more than once for a run.",
    ),
    check(
        "i5-hook-unexpected",
        "I5",
        "`onStop` ran for a script that has no such entry.",
    ),
    check(
        "i6-bare-completion",
        "I6",
        "An awaited family request resumed with the bare seat end and no typed outcome.",
    ),
    check(
        "i6-paint-suspends",
        "I6",
        "A waiting binding in a Paint entry started a request and yielded (finding F1).",
    ),
    check("i6-resume-twice", "I6", "One completion resumed two waits."),
    check(
        "i6-resume-without-completion",
        "I6",
        "A dispatch resumed an invocation with no completion due.",
    ),
    check(
        "i6-two-invocations",
        "I6",
        "A game tick started `tick` while an invocation is suspended.",
    ),
    check(
        "i7a-late-accept",
        "I7 a",
        "A Tick or a Paint entry that passed its deadline was accepted.",
    ),
    check(
        "i7a-late-hook",
        "I7 a",
        "A Hook entry that passed its deadline was accepted.",
    ),
    check(
        "i7a-late-load",
        "I7 a",
        "A Load entry that passed its deadline was accepted.",
    ),
    check(
        "i7b-effect-after-trip",
        "I7 b",
        "A binding had an effect after a trip, after the cap, or after the deadline.",
    ),
    check(
        "i7b-rows-of-failed-dispatch-admitted",
        "I7 b",
        "The slot drained the rows of a dispatch before it handled `Failed`.",
    ),
    check(
        "i7c-gap-bound",
        "I7 c",
        "More script operations ran with no check point than the residual `R_gap`.",
    ),
    check(
        "i7d-deadline-outlives-owner",
        "I7 d",
        "A deadline is armed and no entry runs.",
    ),
    check(
        "i8b-cap-event-untripped",
        "I8 b",
        "An entry saw a refused allocation or a lost error and the latch stayed empty.",
    ),
    check(
        "i9-idle-with-run",
        "I9",
        "The slot is Idle and still owns a run.",
    ),
    check(
        "i10-entry-not-allowed",
        "I10",
        "A Tick or a Paint entry started with a written latch, or with the slot in a state other than Running.",
    ),
    check(
        "i11a-failure-pending",
        "I11 a",
        "A dispatch returned Accepted and the run keeps its failure for a later read.",
    ),
    check(
        "i11b-latch-overwritten",
        "I11 b",
        "A trip replaced a written latch.",
    ),
    check(
        "i11b-selfstop-overridden",
        "I11 b",
        "The progress watchdog restarted a run that called `api.stop` and the replacement runs (finding F3).",
    ),
    check(
        "i11b-wrong-receipt",
        "I11 b",
        "The receipt of a Stop does not follow the first terminal cause of the run (finding F2).",
    ),
    check(
        "i11c-effect-after-stop",
        "I11 c",
        "A binding had an effect after `api.stop`.",
    ),
    check(
        "i12-failure-lost",
        "I12",
        "The failure of `onStop` is not in the completion that `stop` sent.",
    ),
    check(
        "i12-receipt-replaced",
        "I12",
        "The failure of `onStop` changed the receipt.",
    ),
    check(
        "i13-held-not-stored",
        "I13",
        "A held tick was accepted and the run did not store the completion.",
    ),
    check(
        "i13-held-resumes",
        "I13",
        "A held tick resumed the invocation.",
    ),
    check(
        "p1-queued-start-lost",
        "P1",
        "`complete_stop` ended Idle with no diagnostic although a Start was queued behind the Stop.",
    ),
    check(
        "p1-stop-kept-queued-start",
        "P1",
        "A Stop while the slot is Stopping left the queued Start, and the replacement will start.",
    ),
    check(
        "p1-stuck-stopping",
        "P1",
        "The slot is Stopping and no completion is waiting for the observe step.",
    ),
    check(
        "p2-completion-not-delivered",
        "P2",
        "The completion of the awaited request was in the delivery of a wake and the run did not resume.",
    ),
    check(
        "d-call-after-close",
        "D",
        "The runner completed a call on a dock that was already closed.",
    ),
    check(
        "d-completion-overwritten",
        "D",
        "`complete` wrote over a payload that was in the dock.",
    ),
    check(
        "d-consume-not-allowed",
        "D",
        "A paused run consumed the extension payload.",
    ),
    check(
        "d-consume-twice",
        "D",
        "The payload was delivered and the dock still offers it.",
    ),
    check(
        "d-dock-shared",
        "D",
        "A restart claimed the dock of the old run again.",
    ),
    check(
        "d-end-lacks-hook-failure",
        "D",
        "The end record of the dock does not carry the failure of `onStop`.",
    ),
    check(
        "d-failed-dispatch-committed",
        "D",
        "A staged call of a failed dispatch was committed to the dock queue.",
    ),
    check(
        "d-first-fault-lost",
        "D",
        "The end record of the dock lost the first fault that an entry latched.",
    ),
    check(
        "d-payload-lost",
        "D",
        "The call is Completed and its payload is gone before the resume.",
    ),
    check(
        "d-refusal-not-latched",
        "D",
        "An extension call in `onStop` was refused and left no first fault.",
    ),
    check(
        "d-revoke-keeps-payload",
        "D",
        "The dock was revoked and its completion payload is still there.",
    ),
    check(
        "d-session-cancels-extension-wait",
        "D",
        "A session reset ended the extension wait of the run.",
    ),
    check(
        "f-entry-after-fence",
        "F",
        "A Tick or a Paint entry started after the entry fence was published or the dock was revoked.",
    ),
    check(
        "f-lock-wait",
        "F",
        "The fence waited for a busy slot while the runner lock was held.",
    ),
    check(
        "f-row-after-fence",
        "F",
        "The seat admitted a row of a run whose entry fence is published.",
    ),
    check(
        "l-ledger-incomplete",
        "L",
        "An admitted Start has no entry in the execution ledger.",
    ),
    check(
        "l-load-under-admission",
        "L",
        "The Load entry of a File Start ran under the shared admission guard.",
    ),
    check(
        "l-pass-unsound",
        "L",
        "The final gate wrote PASS while a planned execution was not retired by the runner's own Stop and clean, or an admission outside the plan happened, or the slot is not Idle.",
    ),
    check(
        "l-stop-unowned",
        "L",
        "The runner's Stop for one execution ended another execution.",
    ),
    check(
        "l-vm-under-runner-lock",
        "L",
        "A VM section began inside the runner's closure, under the runner lock.",
    ),
];

/// The check with this name.
pub fn find(name: &str) -> Option<&'static Check> {
    CHECKS.iter().find(|check| check.name == name)
}
