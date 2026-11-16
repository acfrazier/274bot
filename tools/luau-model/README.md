# luau-model

A checked model of the Luau run lifecycle (ADR 0017): one slot, one run at a
time, the VM that runs script code only inside an entry, and the actors that
meet at the slot mutex. It is a developer tool. No product crate depends on
it, except as a dev-dependency of the conformance tests in `crates/script-luau`
(slice L2b, not yet written).

It models LD1 (the Luau surface design, section 6: lifecycle invariants), and
the runner side of LD2 (the scenario design: the dock, the entry fence, the
execution ledger, the retirement tickets and the final gate) as far as the
run lifecycle has to stay compatible with it. It is a new model, beside
`hold-model`, `seat-model` and `js-teardown-model`: the Luau machine has no
isolate thread and no supervisor, so it does not extend the JS model. The
explorer (`src/explorer.rs`) is a copy of the explorer of
`tools/js-teardown-model`, which is a copy of the one in `tools/hold-model`.

The crate has no workspace dependency. A search runs inside four caps (a
horizon, a state count, a frontier size and a byte estimate) and reports the
shortest trace of every violation it finds.

## Checks

A check is a function of one step, or of the state after every step
(`check_state`). Each belongs to one invariant of LD1 6.2, or to one rule of
the runner side of LD2. The text of each invariant is short here; the names
are in `src/lifecycle/checks.rs`, and `luau-model --list-checks` prints them.

| Rule | What it says |
|---|---|
| I1 | One VM, only after a Start. A failed `spawn` drops its VM before it returns. |
| I2 | After `pause` returns, no Tick or Paint entry runs until `resume`. A suspended invocation survives. A Stop of a paused run still runs `onStop` once. |
| I3 | `resume` does not enter the VM. A completion that arrived during Pause is delivered once, by the first accepted dispatch after Resume. |
| I4 | `stop` is accepted in every state. When it returns the VM is gone, `done` got one message, no later call enters the VM, and no row of the run reaches the seat after the Stop began. |
| I5 | `onStop` runs at most once, only inside `stop`, never in a dispatch, never after a trip or an internal failure, never for a script that did not load or has no such entry. A pause does not change that. |
| I6 | At most one invocation of `tick` is live. A game tick starts one only when none is live and the tick is not held. A suspended invocation resumes once per wait, in a dispatch that is not held and carries its completion. A dispatch with nothing due does not enter the VM. |
| I7 | a: each entry has its deadline (one for a whole dispatch), and no entry that passed its deadline is accepted. b: after a trip no new entry starts and no binding has an effect. c: script code after the deadline is bounded (`R_gap`, `R_prop`). d: a deadline is cleared when its owner ends. |
| I8 | b: each cap event trips the run (E1, E2: memory; E3: internal) unless an earlier cause set the latch. c: the verdict does not depend on the error value that the script sees. |
| I9 | Every end of a run (Stop, removal, `api.stop`, a trip, an internal failure, a restart, the drop of the slot) drops the VM once, discards the queued rows and leaves `Stopping`. |
| I10 | Script code runs only in the Load entry of `spawn`, in a Tick or Paint entry of a dispatch (the slot is `Running`, the run is not paused, tripped or self-stopped) and in the Hook entry of `stop`. |
| I11 | a: a failure of a dispatch is its return value; none waits in the run. b: the latch is written once and the first terminal cause decides the receipt. c: `api.stop` ends its entry at the call. |
| I12 | A failure inside `onStop` ends the hook, reaches the log through the completion, and does not change the receipt. |
| I13 | A held dispatch stores the completion and may run the Paint entry. It does not start or resume `tick`. A completion that the host acknowledged is never lost. |
| P1 | After a Stop the slot retires the run within a fixed number of observe steps. A queued Start or restart spawns, or ends `Idle` with a diagnostic. |
| P2 | A completion in a delivery, a `Running` slot with `want_run`, a run that is not paused, tripped or stopped, and a dispatch that is not held: that dispatch resumes the invocation. |
| D | The scenario dock (LD2 2.2 to 2.4): one claim, one awaited call, one completion payload, a durable first fault, an end record. |
| F | The entry fence (LD2 5.1 phase A): one try-lock publication that never waits, and no entry after it. |
| L | The execution ledger, the retirement tickets, the final gate (with a plan that requires the natural self-stop, and its housekeeping Stop), and the rule that the runner issues no Stop and loads no file under its own lock (LD2 3.2, 3.5, 4.1, 5.4). |

**TA**, the termination assumption of LD1 6.2, is not a check. The model
treats every section that holds the slot mutex as ending, and it has no wall
clock. The test `a_section_can_always_end_and_a_completion_can_always_be_read`
checks the half that a model can: no reachable state inside a section lacks a
step, and a `Stopping` slot always offers the step that retires it. L2p proves
TA on the real VM.

I8 a (used bytes never exceed `H + E`) is not modelled: the model has the
three memory events and no byte count.

| Rule | Check | What it refuses | Mutations that fail it |
|---|---|---|---|
| I1 | `i1-two-vms` | More than one VM exists: the run's, the Load entry's and any that a failed spawn kept. | `spawn-before-retire` |
| I1 | `i1-vm-outlives-run` | A failed spawn returned its error and kept its VM. | `failed-load-leak` |
| I2 | `i2-entry-while-paused` | A Tick or a Paint entry started while the run is paused. | `check-pause-no-refusal`, `paused-paint`, `seat-page-ignores-pause` |
| I3 | `i3-completion-lost` | The completion of the awaited request arrived and neither the host nor the run holds it, or the host acknowledged a completion that the run neither stored nor used. | `ack-on-refusal`, `lost-wake` |
| I4 | `i4-done-count` | `stop` sent a number of messages on `done` other than one. | `done-twice` |
| I4 | `i4-entry-after-stop` | A Tick or a Paint entry started after `stop` began. | `hook-outside-mutex` |
| I4 | `i4-row-after-stop` | A row of the run reached the seat after the Stop began. | `hook-action-legal`, `rows-after-stop` |
| I4 | `i4-vm-after-stop` | The slot is gone and a run, a VM or a section is left. | `check-remove-keeps-vm` |
| I5 | `i5-hook-after-trip` | `onStop` ran after a trip. | `on-stop-after-trip` |
| I5 | `i5-hook-in-dispatch` | `api.stop` ran `onStop` inside the dispatch. | `on-stop-in-dispatch` |
| I5 | `i5-hook-skipped` | `onStop` did not run for a script that loaded, has the entry and did not trip or fail. | `hook-skipped-while-paused` |
| I5 | `i5-hook-twice` | `onStop` started more than once for a run. | `check-hook-twice` |
| I5 | `i5-hook-unexpected` | `onStop` ran for a script that has no such entry. | `hook-runs-without-entry` |
| I6 | `i6-bare-completion` | An awaited family request resumed with the bare seat end and no typed outcome. | `bare-seat-end` |
| I6 | `i6-paint-suspends` | A waiting binding in a Paint entry started a request and yielded (finding F1). | `f1-paint-wait-suspends` |
| I6 | `i6-resume-twice` | One completion resumed two waits. | `resume-twice` |
| I6 | `i6-resume-without-completion` | A dispatch resumed an invocation with no completion due. | `resume-without-completion` |
| I6 | `i6-two-invocations` | A game tick started `tick` while an invocation is suspended. | `reenter-tick` |
| I7 a | `i7a-late-accept` | A Tick or a Paint entry that passed its deadline was accepted. | `no-exit-check`, `paint-rearms-deadline`, `sampled-clock`, `verdict-from-error` |
| I7 a | `i7a-late-hook` | A Hook entry that passed its deadline was accepted. | `hook-ungated` |
| I7 a | `i7a-late-load` | A Load entry that passed its deadline was accepted. | `load-ungated` |
| I7 b | `i7b-effect-after-trip` | A binding had an effect after a trip, after the cap, or after the deadline. | `effect-after-trip`, `gap-effect` |
| I7 b | `i7b-rows-of-failed-dispatch-admitted` | The slot drained the rows of a dispatch before it handled `Failed`. | `drain-before-failure` |
| I7 c | `i7c-gap-bound` | More script operations ran with no check point than the residual `R_gap`. | `check-gap-unbounded` |
| I7 d | `i7d-deadline-outlives-owner` | A deadline is armed and no entry runs. | `deadline-carries` |
| I8 b | `i8b-cap-event-untripped` | An entry saw a refused allocation or a lost error and the latch stayed empty. | `entry-through-call`, `lost-error-ignored`, `lost-refusal`, `memory-verdict-from-error` |
| I9 | `i9-idle-with-run` | The slot is Idle and still owns a run. | `check-stop-keeps-run` |
| I10 | `i10-entry-not-allowed` | A Tick or a Paint entry started with a written latch, or with the slot in a state other than Running. | `self-stop-accepts-dispatch` |
| I11 a | `i11a-failure-pending` | A dispatch returned Accepted and the run keeps its failure for a later read. | `pending-failure` |
| I11 b | `i11b-latch-overwritten` | A trip replaced a written latch. | `latch-overwritten` |
| I11 b | `i11b-selfstop-overridden` | The progress watchdog restarted a run that called `api.stop` and the replacement runs (finding F3). | `f3-restart-ignores-self-stop` |
| I11 b | `i11b-wrong-receipt` | The receipt of a Stop does not follow the first terminal cause of the run (finding F2). | `f2-stop-ignores-self-stop` |
| I11 c | `i11c-effect-after-stop` | A binding had an effect after `api.stop`. | `stop-continues` |
| I12 | `i12-failure-lost` | The failure of `onStop` is not in the completion that `stop` sent. | `hook-failure-lost` |
| I12 | `i12-receipt-replaced` | The failure of `onStop` changed the receipt. | `hook-failure-replaces-receipt` |
| I13 | `i13-held-not-stored` | A held tick was accepted and the run did not store the completion. | `ack-without-store` |
| I13 | `i13-held-resumes` | A held tick resumed the invocation. | `held-consumes`, `held-tick-resumes` |
| P1 | `p1-queued-start-lost` | `complete_stop` ended Idle with no diagnostic although a Start was queued behind the Stop. | `queued-start-lost` |
| P1 | `p1-stop-kept-queued-start` | A Stop while the slot is Stopping left the queued Start, and the replacement will start. | `stop-keeps-queued-start` |
| P1 | `p1-stuck-stopping` | The slot is Stopping and no completion is waiting for the observe step. | `check-done-dropped` |
| P2 | `p2-completion-not-delivered` | The completion of the awaited request was in the delivery of a wake and the run did not resume. | `lent-skips-seat-page` |
| D | `d-call-after-close` | The runner completed a call on a dock that was already closed. | `calls-after-close` |
| D | `d-completion-overwritten` | `complete` wrote over a payload that was in the dock. | `completion-overwrite` |
| D | `d-consume-not-allowed` | A paused run consumed the extension payload. | `consume-while-paused` |
| D | `d-consume-twice` | The payload was delivered and the dock still offers it. | `consume-twice` |
| D | `d-dock-shared` | A restart claimed the dock of the old run again. | `shared-restart-dock` |
| D | `d-end-lacks-hook-failure` | The end record of the dock does not carry the failure of `onStop`. | `hook-failure-not-in-end` |
| D | `d-failed-dispatch-committed` | A staged call of a failed dispatch was committed to the dock queue. | `commit-failed-dispatch` |
| D | `d-first-fault-lost` | The end record of the dock lost the first fault that an entry latched. | `unobserved-refusal-lost` |
| D | `d-payload-lost` | The call is Completed and its payload is gone before the resume. | `refused-dispatch-loses-payload` |
| D | `d-refusal-not-latched` | An extension call in `onStop` was refused and left no first fault. | `caught-hook-refusal-lost` |
| D | `d-revoke-keeps-payload` | The dock was revoked and its completion payload is still there. | `revoke-keeps-payload` |
| D | `d-session-cancels-extension-wait` | A session reset ended the extension wait of the run. | `session-cancels-extension-wait` |
| F | `f-entry-after-fence` | A Tick or a Paint entry started after the entry fence was published or the dock was revoked. | `wake-between-freeze-and-stop` |
| F | `f-lock-wait` | The fence waited for a busy slot while the runner lock was held. | `blocking-fleet-lock` |
| F | `f-row-after-fence` | The seat admitted a row of a run whose entry fence is published. | `post-freeze-action` |
| L | `l-ledger-incomplete` | An admitted Start has no entry in the execution ledger. | `forget-earlier-execution`, `loading-missing-from-ledger` |
| L | `l-load-under-admission` | The Load entry of a File Start ran under the shared admission guard. | `file-compile-holds-shared-admission` |
| L | `l-pass-unsound` | The final gate wrote PASS while a planned execution was not retired by the runner's own ticketed Stop and clean (or, when the plan requires the natural self-stop, was not ended by its own clean `api.stop`), or an admission outside the plan happened, or the slot is not Idle. | `failed-replacement-restores-authorization`, `forced-stop-satisfies-natural-stop`, `guessed-stop-generation`, `replacement-with-old-end` |
| L | `l-stop-unowned` | The runner's Stop for one execution ended another execution. | `repeat-last-table-stop` |
| L | `l-vm-under-runner-lock` | A VM section began inside the runner's closure, under the runner lock. | `file-compile-under-runner`, `stop-under-runner-lock` |

A test fails when a check cannot fail: `every_mutation_fails_the_invariant_that_must_catch_it`
runs each mutation and requires that every check of this table fires in some
run.

## What the model covers

Five kinds of actor. Every action is one critical section of the slot mutex
(LD1 rule X2), or one step that the product takes outside it. A call that
enters the VM (a Start, a dispatch, a Stop with a hook) is a *section*: while
it runs, only script operations and the clock move.

| Model action | Product function (today `crates/script/src/slot.rs`; the run is LD1's `LuauSurfaceRun`) |
|---|---|
| `Start`, `Vm(Return(Table))` | `start_surface` with `spawn`: the Load entry (rule X9). A Start while the slot is `Stopping` is the queued `AfterStop::Start` |
| `Pause`, `Resume` | `SlotScript::pause`, `resume` |
| `Stop`, `Remove`, `DropSlot` | `stop_with_reason`, `begin_async_stop` and the run's `stop` (rule X8): receipt, `onStop`, drop of the VM, the message on `done`. `Drop` of the slot |
| `Dispatch(Tick, Evidence, SeatPage)` | the lent dispatch of one wake (LD1 4.2): the Tick entry, the Paint entry, one deadline, the outcome that the slot reads in the same call (4.3) |
| `Observe` | `observe_lifecycle`: `observe_surface_lifecycle` (`poll_ready`, `stop_rx`, `complete_stop`) and `drain_logs` |
| `Restart` | `restart_from_identity`, the progress watchdog's restart |
| `ClientDown`, `ClientUp` | the client-up gate |
| `HoldOn`, `HoldOff`, `Arrive`, `SessionReset` | the host's hold, the arrival of the awaited completion, the seat's session reset |
| `Advance`, `Next` | one unit of time; the dispatch goes from its Tick entry to its Paint entry |
| `Vm(op)` | one script operation inside an entry: the table of LD1 6.1 |
| `Fence`, `FenceWait`, `Revoke` | the runner's entry fence, its blocking form (only a mutation offers it) and the revocation of the dock |
| `Post`, `Pump`, `Take`, `Complete`, `FinalGate` | the runner's owner intent, the role pump that executes it outside the runner lock, the runner's take and complete of the dock call, the final gate |

### Abstractions

Each is on purpose.

1. **Every section is atomic, and ends.** The model states TA. It has no wall
   clock. Time is a counter: one unit is a part of a budget. The dispatch has
   one budget for its Tick entry and its Paint entry together (`b_dispatch`,
   two units by default), the Load and Hook entries one each (`b_load`,
   `b_hook`). `Advance` takes one unit from the deadline that the run armed
   and from `truth`, a ghost of the real deadline that a mutation cannot
   change. A comparison in the product is `<=` or `>=` between a start and a
   bound, so the order of events is kept and the values are not.
2. **Script code is a sequence of operations.** The model has no Luau. An
   entry runs operations chosen from the table of LD1 6.1: a straight-line
   gap, an interrupt (the check point), a binding (a row, a log line, a
   waiting binding, `api.stop`, an extension call), a return, an uncaught
   error, a `pcall` that catches a propagating error, an allocation event, a
   panic. `R_gap` and `R_prop` bound the gap after a deadline and the number
   of errors that a script can swallow (`r_gap`, `r_prop`, one by default).
3. **One invocation of `tick`.** Its states are `Idle`, `Running` and
   `Suspended(wait)`. A wait is a seat request, a typed family request, a
   walk, a tick wait, or an extension call of the dock. The completion is a
   fact in the host (`Comp`) and a ghost that records that it arrived and
   that it was used.
4. **Memory is three events.** E1 (used bytes reach `S`), E2 (the allocator
   refuses at the host, at a binding or at a guarded call) and E3 (a lost
   error). The model has no byte counts (I8 a is not modelled).
5. **A row is a counter.** A binding queues a row. The dispatch admits it at
   its end, and a failed dispatch discards it. The seat is not modelled
   beyond that.
6. **One slot, one run at a time.** The slot fields are those that the
   lifecycle reads: `RunState`, `want_run`, `AfterStop`, the generation, the
   generation base that a failed setup gives back, `ready`, the channel of
   the stop completion, the receipt, the card text, the Start serial and the
   Stop ordinal. A Start, a queued Start and a restart each admit one run
   (`starts` is the budget). The observe step is one step: `poll_ready`, the
   log drain and the stop completion.
7. **The host is a hold, a completion and a gate.** A hold, the one
   completion fact that a delivery carries, a session reset that aborts the
   wait, and the client gate. A reconnect is not modelled.
8. **The LD2 layers are optional.** A profile turns on the dock, the fence
   and the runner. The dock has one claim, one call slot with its states
   (`Staged`, `Queued`, `Taken`, `Completed`, `Consumed`, `Done`,
   `Discarded`, `Cancelled`), one completion payload, a first-fault latch and
   an end record. The runner has a ledger of up to three admitted Starts
   (serial, planned or foreign, state, clean, natural witness, generation),
   the retirement ticket that the slot issues for a Stop of the runner (and,
   when the plan requires the natural self-stop, for the one housekeeping Stop
   of the idle slot after the script stopped itself), the posted intent and
   the verdict. The runner never runs script code: it posts an intent, and the
   role pump executes it.
9. **Counts are budgets.** Every event has a count in the profile: Starts,
   ticks (and how many are held), wakes, waits, control operations, restarts,
   gate changes, holds, session resets, memory events, panics. A profile
   explores every order of its events and no more.
10. **Safety, and the bounded progress of P1 and P2 as checks.** The explorer
    does not check liveness. P1 and P2 are checks on states and on steps
    (`p1-stuck-stopping`, `p2-completion-not-delivered`), with no fairness
    assumption.

## Mutations

Each earlier design that a review rejected, and each defect that the model
found, is a `Mutation` that must make one named check fail
(`Mutation::caught_by`, tested in `tests/model.rs`). The head is `none`. A
mutation changes transitions only. The checks are frozen. The Steps column is
the length of the shortest trace to the named check, on the profile of the
`Profile` column (the smallest profile that catches it; every profile is
exhausted, so the choice only keeps the test lane short).

Groups: LD1 6.4 is the table of LD1 (round 1 to round 3 of its reviews). LD1
rules are rules of LD1 that its table has no row for. LD2 is the runner side.
The self-checks are the builder's proof that a check can fail. Seventy-eight
mutations, none uncaught.

#### LD1 6.4

| Mutation | What it is | Caught by | Profile | Steps |
|---|---|---|---|---|
| `on-stop-after-trip` | `stop` runs `onStop` after a trip. | `i5-hook-after-trip` | `smoke` | 6 |
| `on-stop-in-dispatch` | `api.stop` runs `onStop` at once, inside the dispatch. | `i5-hook-in-dispatch` | `smoke` | 4 |
| `seat-page-ignores-pause` | A seat-page dispatch resumes a paused run. | `i2-entry-while-paused` | `waits` | 9 |
| `deadline-carries` | The deadline is not cleared when its owner ends: a Hook entry or a later dispatch gets the old deadline. | `i7d-deadline-outlives-owner` | `smoke` | 2 |
| `verdict-from-error` | The run fails only when the error reaches the host. A script catches it with `pcall` and goes on. | `i7a-late-accept` | `smoke` | 6 |
| `resume-twice` | A tick dispatch and a seat-page dispatch both deliver one completion. | `i6-resume-twice` | `waits` | 10 |
| `lost-wake` | A completion that arrives during Pause is dropped. | `i3-completion-lost` | `waits` | 8 |
| `rows-after-stop` | Rows of the last dispatch are drained after the Stop. | `i4-row-after-stop` | `smoke` | 9 |
| `reenter-tick` | A game tick starts `tick` while an invocation is suspended. | `i6-two-invocations` | `waits` | 7 |
| `done-twice` | `done` gets a message before `onStop` and one after it. | `i4-done-count` | `smoke` | 4 |
| `spawn-before-retire` | A slot restart spawns the new VM before the old `stop` returned. | `i1-two-vms` | `control` | 3 |
| `sampled-clock` | The interrupt reads the clock at every 256th call only; no exit check. An entry passes its deadline and returns before the next read. | `i7a-late-accept` | `smoke` | 6 |
| `no-exit-check` | No check at the exit. An entry passes its deadline inside one interrupt-free call and then ends with an error. | `i7a-late-accept` | `smoke` | 6 |
| `effect-after-trip` | A binding queues a row after the latch is set, in a script that caught the trip error. | `i7b-effect-after-trip` | `smoke` | 3 |
| `drain-before-failure` | The slot drains the rows of a dispatch before it handles `Failed`. | `i7b-rows-of-failed-dispatch-admitted` | `smoke` | 7 |
| `memory-verdict-from-error` | A refused allocation sets the latch only when the error reaches the host. A script catches it in `pcall`. | `i8b-cap-event-untripped` | `memory` | 3 |
| `pending-failure` | The run keeps a dispatch failure for a later read. A Stop comes first, and the failure is lost. | `i11a-failure-pending` | `smoke` | 6 |
| `hook-failure-lost` | `stop` sends only log lines. A trip inside `onStop` leaves no trace. | `i12-failure-lost` | `smoke` | 5 |
| `hook-failure-replaces-receipt` | A trip inside `onStop` writes a `Failed` receipt over the `Stopped` receipt of `api.stop`. | `i12-receipt-replaced` | `smoke` | 6 |
| `hook-skipped-while-paused` | A Stop of a paused run does not run `onStop`. | `i5-hook-skipped` | `smoke` | 4 |
| `paused-paint` | A paused run runs `onPaint`. | `i2-entry-while-paused` | `smoke` | 4 |
| `load-ungated` | The top level of the file runs with no deadline and no cap. | `i7a-late-load` | `smoke` | 3 |
| `failed-load-leak` | A failed load returns its error and keeps the VM. | `i1-vm-outlives-run` | `smoke` | 2 |
| `held-tick-resumes` | A held tick resumes the invocation. | `i13-held-resumes` | `waits` | 8 |
| `ack-without-store` | A held tick is accepted and the run does not store the completion. | `i13-held-not-stored` | `waits` | 10 |
| `ack-on-refusal` | The host acknowledges a delivery that the run refused. | `i3-completion-lost` | `waits` | 9 |
| `lent-skips-seat-page` | The lent path passes the host-built facts without the slot pages. | `p2-completion-not-delivered` | `waits` | 8 |
| `paint-rearms-deadline` | The Paint entry arms a new deadline. A Tick entry and a Paint entry that each stay inside the budget are both accepted. | `i7a-late-accept` | `smoke` | 8 |
| `gap-effect` | The check runs only at an interrupt. An entry passes its deadline in straight-line code and calls a binding before the next interrupt. | `i7b-effect-after-trip` | `smoke` | 3 |
| `lost-refusal` | The guarded `xpcall` wraps the handler of the VM's `xpcall`: a refusal at the C-call limit is replaced by "error in error handling". | `i8b-cap-event-untripped` | `memory` | 3 |
| `entry-through-call` | The run enters a Paint entry through a protected call with an error function, not through a resume. | `i8b-cap-event-untripped` | `memory` | 7 |
| `lost-error-ignored` | A binding returns the lost-error value of `mlua` to the script as an ordinary error. | `i8b-cap-event-untripped` | `memory` | 3 |
| `stop-continues` | `api.stop` sets the fact and returns to the script. | `i11c-effect-after-stop` | `smoke` | 5 |
| `latch-overwritten` | A watchdog trip after `api.stop` in the same entry replaces the self-stop in the latch. | `i11b-latch-overwritten` | `smoke` | 7 |
| `queued-start-lost` | `complete_stop` ignores a Start that was queued while the slot was `Stopping`. | `p1-queued-start-lost` | `control` | 5 |
| `stop-keeps-queued-start` | A second Stop while the slot is `Stopping` does not drop the queued Start. | `p1-stop-kept-queued-start` | `control` | 4 |
| `bare-seat-end` | An awaited family request resumes with the seat end and no typed outcome. | `i6-bare-completion` | `waits` | 8 |

#### LD1 rules

| Mutation | What it is | Caught by | Profile | Steps |
|---|---|---|---|---|
| `hook-action-legal` | An action binding is legal in `onStop` (rule X8 step 2 makes it an error): its row is queued after the Stop began. | `i4-row-after-stop` | `smoke` | 4 |
| `self-stop-accepts-dispatch` | A run that called `api.stop` accepts the next dispatch (I10: it refuses each dispatch until the slot stops it). | `i10-entry-not-allowed` | `control` | 6 |
| `resume-without-completion` | A game tick resumes a suspended invocation whether or not its completion is in the delivery (I6: a dispatch with nothing due does not enter the VM). | `i6-resume-without-completion` | `waits` | 7 |
| `hook-runs-without-entry` | `stop` runs `onStop` for a script that has no such entry (I5). | `i5-hook-unexpected` | `control` | 3 |
| `hook-outside-mutex` | `stop` runs `onStop` without the slot mutex (rule X2 makes every step atomic): a dispatch enters the VM while the hook runs. | `i4-entry-after-stop` | `smoke` | 4 |
| `hook-ungated` | The Hook entry arms no deadline of its own (I7 a). | `i7a-late-hook` | `smoke` | 5 |

#### found by this model

| Mutation | What it is | Caught by | Profile | Steps |
|---|---|---|---|---|
| `f1-paint-wait-suspends` | F1: a waiting binding in a Paint entry starts its request and yields. LD1 6.1 lists "yield at a wait" for a Paint entry and no rule says what the run does with it. The head refuses the wait in a Paint entry. | `i6-paint-suspends` | `waits` | 5 |
| `f2-stop-ignores-self-stop` | F2: an operator Stop does not read the latch of a run that called `api.stop`; only the log drain writes the `Stopped` receipt. | `i11b-wrong-receipt` | `smoke` | 6 |
| `f3-restart-ignores-self-stop` | F3: the progress watchdog restarts a run that called `api.stop` and the replacement starts. | `i11b-selfstop-overridden` | `control` | 6 |

#### LD2: the dock, the fence, the ledger

| Mutation | What it is | Caught by | Profile | Steps |
|---|---|---|---|---|
| `shared-restart-dock` | A retained Start used by a watchdog restart claims the old dock again. | `d-dock-shared` | `dock-restart` | 5 |
| `completion-overwrite` | `complete` writes over a payload that is in the dock. | `d-completion-overwritten` | `fence` | 9 |
| `consume-twice` | One payload resumes two waits. | `d-consume-twice` | `fence` | 9 |
| `consume-while-paused` | A paused run consumes the extension payload. | `d-consume-not-allowed` | `dock` | 10 |
| `held-consumes` | A held dispatch consumes the extension payload. | `i13-held-resumes` | `dock` | 10 |
| `refused-dispatch-loses-payload` | A refused dispatch drops the extension payload. | `d-payload-lost` | `dock` | 10 |
| `commit-failed-dispatch` | A failed dispatch commits its staged calls. | `d-failed-dispatch-committed` | `dock-trips` | 5 |
| `session-cancels-extension-wait` | A session reset ends the extension wait. | `d-session-cancels-extension-wait` | `dock` | 7 |
| `calls-after-close` | The dock admits a call after it closed. | `d-call-after-close` | `fence` | 3 |
| `hook-failure-not-in-end` | The end record leaves out the failure of `onStop`. | `d-end-lacks-hook-failure` | `dock-trips` | 4 |
| `caught-hook-refusal-lost` | A refused extension call in `onStop` leaves no first fault when the script catches the error. | `d-refusal-not-latched` | `fence` | 4 |
| `unobserved-refusal-lost` | `close` builds the end record without the first fault. | `d-first-fault-lost` | `fence` | 3 |
| `revoke-keeps-payload` | Revocation leaves the completion payload in the dock. | `d-revoke-keeps-payload` | `fence` | 9 |
| `guessed-stop-generation` | The runner decides that a Stop is its own from `generation == old + 1`. | `l-pass-unsound` | `runner` | 7 |
| `replacement-with-old-end` | The final gate accepts a replacement run on the old run's clean end. | `l-pass-unsound` | `runner` | 4 |
| `failed-replacement-restores-authorization` | The final gate compares the live generation with the ticket's. A failed queued setup gives the generation back. | `l-pass-unsound` | `runner` | 11 |
| `forget-earlier-execution` | The ledger is one mutable field: a later admission forgets the earlier execution. | `l-ledger-incomplete` | `runner` | 3 |
| `repeat-last-table-stop` | The runner issues a second Stop for an execution that a ticketed Stop already retired. It lands on the replacement. | `l-stop-unowned` | `runner` | 10 |
| `loading-missing-from-ledger` | The ledger records an admission at Ready, not at the Start. | `l-ledger-incomplete` | `runner` | 1 |
| `wake-between-freeze-and-stop` | An entry enters between the published fence and the Stop. | `f-entry-after-fence` | `fence` | 4 |
| `post-freeze-action` | The seat admits a row of a fenced run. | `f-row-after-fence` | `fence` | 8 |
| `blocking-fleet-lock` | The fence waits for a busy slot mutex while it holds the runner lock. | `f-lock-wait` | `fence` | 2 |
| `stop-under-runner-lock` | The runner issues the Stop inside its own closure, under its lock. | `l-vm-under-runner-lock` | `runner` | 5 |
| `file-compile-under-runner` | File Start, compile and Load run under the runner lock. | `l-vm-under-runner-lock` | `runner` | 2 |
| `file-compile-holds-shared-admission` | File Start, compile and Load hold the shared admission guard. | `l-load-under-admission` | `runner` | 2 |
| `forced-stop-satisfies-natural-stop` | A plan that requires the natural self-stop accepts the Stop that the runner forced: the ticket and a clean end stand for the witness. | `l-pass-unsound` | `runner-natural` | 9 |

#### self-checks

| Mutation | What it is | Caught by | Profile | Steps |
|---|---|---|---|---|
| `check-gap-unbounded` | Check: a gap longer than `R_gap` is allowed. | `i7c-gap-bound` | `smoke` | 3 |
| `check-hook-twice` | Check: a second `onStop` can start. | `i5-hook-twice` | `smoke` | 4 |
| `check-pause-no-refusal` | Check: a Pause does not stop the dispatch from entering the VM. | `i2-entry-while-paused` | `smoke` | 4 |
| `check-done-dropped` | Check: the completion that `stop` sends never reaches the slot, so the slot stays Stopping. | `p1-stuck-stopping` | `smoke` | 4 |
| `check-stop-keeps-run` | Check: a stopped run stays in the slot. | `i9-idle-with-run` | `smoke` | 5 |
| `check-remove-keeps-vm` | Check: a removed slot keeps the VM of its run. | `i4-vm-after-stop` | `control` | 3 |

Why some LD1 and LD2 mutations are not in this model, and what LD2 rows
remain: see "Not modelled".

## Findings

LD1 and LD2, taken as written, leave three behaviours open that the model
reached. Each has a mutation that keeps the design as written, with its
shortest trace. The head resolves each one with a rule that LD1 should state.

**F1. A wait in a Paint entry has no resumer** (`f1-paint-wait-suspends`,
profile `waits`, 5 steps). LD1 6.1 lets a script "yield at a wait" in a Tick
or a Paint entry, and the only coroutine that a dispatch resumes is the
invocation of `tick`. Trace: Start; the file returns a table with `onPaint`;
the host holds the frame; a held tick dispatch runs the Paint entry only; the
Paint entry calls a waiting binding, which starts its request and yields. No
state of the run can resume that coroutine. The head makes a waiting binding
an error to the script in a Paint entry (and in a Load and a Hook entry),
like an action binding in `onStop`.

**F2. An operator Stop can overtake the report of `api.stop`**
(`f2-stop-ignores-self-stop`, profile `smoke`, 6 steps). LD1 4.3 writes the
`Stopped` receipt where the slot's log drain reads the stop request of the
script. Trace: Start; a tick dispatch; the script calls `api.stop` and its
entry ends; the operator's Stop arrives before the next observe step. A Stop
that reads only its own reason writes the receipt and the card text of an
operator Stop, and not those of the first terminal cause (`api.stop`). The
head reads the latch of the run in every Stop (I11 b).

**F3. The progress watchdog can restart a run that asked to stop**
(`f3-restart-ignores-self-stop`, profile `control`, 6 steps). Trace: Start; a
tick dispatch; the script calls `api.stop` and its entry ends; the progress
watchdog restarts the run before the slot observed the request. The
replacement runs although the script ended itself. The head does not offer
`restart_from_identity` for a run with a self-stop latch.

LD2 observations. A retained Start that a watchdog restart reuses cannot claim
a dock a second time (`shared-restart-dock`, 5 steps): the spawn fails. LD2
2.2 says the same, and the model confirms that no fresh dock exists for a
retained Start. A revoked run sets the runner's failed flag, so its final gate
cannot pass (LD2 5.3, row V08).

## Run it

Every run is bounded by a horizon, a state cap (115 000 000), a frontier cap
(60 000 000) and a byte cap (an estimate, 12 GiB). Every profile ends before
its horizon (`deepest < horizon`, asserted in `tests/model.rs`): the horizon
does not bound what a profile covers.

```sh
cargo run --release -p luau-model -- --profile all-profiles
cargo run --release -p luau-model -- --profile control --mutation f3-restart-ignores-self-stop
cargo run --release -p luau-model -- --list              # profiles, lane, horizon
cargo run --release -p luau-model -- --list-mutations
cargo run --release -p luau-model -- --list-checks
```

Options: `--profile NAME|all-profiles`, `--mutation NAME`, `--horizon N`,
`--state-cap N`, `--frontier-cap N`, `--byte-cap N`, `--all-violations` (one
shortest trace for each violation name, not only the first). The exit code is
0 for a complete clean run, 1 for a violation, 3 when a cap stopped the run
and 2 for a bad argument.

The head is clean and exhausted in every profile. The counts are the same on
every machine.

| Lane | Profile | What it adds | Horizon | States | Deepest |
|---|---|---|---|---|---|
| unit | `smoke` | one run: Start, Stop, the watchdog, `api.stop` | 28 | 288 | 13 |
| unit | `control` | two Starts, Pause, Resume, Stop, removal, the progress watchdog's restart, the client gate, scripts with and without hooks | 40 | 7 995 | 17 |
| unit | `waits` | seat, family, walk and tick waits; a hold; a session reset; evidence and seat-page wakes | 44 | 31 674 | 23 |
| unit | `watchdog` | one deadline for the Tick and the Paint entry, trips and panics in every entry kind | 44 | 579 | 17 |
| unit | `memory` | the memory events E1, E2 (three sites), E3 | 40 | 1 120 | 17 |
| unit | `dock` | the dock: extension calls, the runner's take and complete, revocation, a hold, a reset | 44 | 63 350 | 27 |
| unit | `dock-trips` | the dock with trips and internal failures | 44 | 8 935 | 23 |
| unit | `dock-restart` | a retained Start after a restart gets a fresh dock | 44 | 3 767 | 21 |
| unit | `fence` | the entry fence and the revocation of the dock | 40 | 3 779 | 21 |
| unit | `runner` | the ledger, the retirement tickets and the final gate, two Starts | 44 | 49 214 | 23 |
| unit | `runner-natural` | a plan that requires the natural self-stop: the script's own `api.stop`, then one housekeeping Stop of the idle slot; a forced Stop is not that witness | 44 | 54 808 | 23 |
| unit | `clock` | a longer clock (two units for Load and Hook, three for the dispatch), three ticks, a memory event, a panic | 52 | 15 368 | 24 |
| nightly | `runner-restart` | `runner` with a restart, three Starts all planned, two ticks, `api.stop`, four control operations | 52 | 15 119 060 | 40 |
| nightly | `full` | everything in small counts with the longer clock: two runs and a restart, held and evidence ticks, a wait, a reset, a hold, the client gate, one memory event, one panic | 64 | 22 244 068 | 31 |
| nightly | `suggested` | the suggested start of LD1 6.5: two runs, four ticks (one held), two waits, three control operations, the watchdog, one memory event and one panic, with the seat pages, reset, hold and client gate of `waits` | 64 | 32 721 008 | 36 |
| nightly | `scenario` | LD1 and the runner side of LD2 together, with one dispatch | 64 | 17 065 647 | 35 |

`scenario` with a second dispatch needs more than 62 million states and passes
the byte cap: the profile keeps one dispatch, and `dock`, `fence` and
`runner-restart` explore the dock and the runner with more ticks. On the Linux
nightly host (release build, one core, one 16 GB scope for each profile) the
four nightly profiles take 18 to 37 seconds and 3.2 to 6.5 GB of resident
memory; `suggested` is the largest. The unit lane (`cargo test -p luau-model`,
about 7 seconds in a debug build) runs the twelve unit profiles and the 78
mutation runs. The four nightly profiles run in `nightly.sh` (one 16 GB scope
for each profile) and, as a test lane, with `LUAU_MODEL_FULL=1 cargo test
--release -p luau-model` (about 100 seconds). `systemd/` holds the user unit
and timer for a Linux nightly host (the install steps are in the unit's
header); nothing installs them.

## Conformance (slice L2b)

The conformance tests of L2b live in `crates/script-luau`. They drive a real
`SlotScript` with a real `LuauSurfaceRun` through traces of this model, with
the replaceable clock of LD1 rule C3, and they fail when the code and the
model differ.

- `Lifecycle::new(bounds, Mutation::None)`, `initial_state`, `enabled` and
  `step_detail` replay a trace: `step_detail` returns the next state and the
  events of the step, or the violation. `describe_trace` prints a trace.
- `goal_traces(bounds, mutation, horizon, limits, cap)` returns, for each
  event of `GOALS` (30 rules of the lifecycle, each at the step where it
  decides), up to `cap` shortest traces. A conformance test drives each one on
  the real slot and compares the real state with the model's after every
  step. The fields to compare: the slot's `RunState`, `want_run`, `AfterStop`,
  generation, Start serial, receipt, card text, `stop_rx` and `ready`; the
  run's latch, pause flag, invocation state, stored completion, `hook_runs`
  and the message on `done`; the dock's call state and first fault.
- The four traces of LD1 6.5 are tests of this crate: `trace_one_*` to
  `trace_four_*`. L2b replays the same four on the real slot.
- A deterministic random walk of `full` or `suggested` (the explorer's
  `Machine` implementation gives the enabled steps) covers the rest. A real
  step that the model does not offer is a divergence.
- Each mutation is a seeded divergence: a conformance driver that puts a
  rejected behaviour back into the code must fail, as in
  `js-teardown-model`.

The model does not replace the bound tests of LD1 3.5 (L2p), which use the real
clock on the reference machine.

## Not modelled

- LD1: the byte bound of I8 a; the content of rows and the seat's client ids,
  stamps and reconnect; the lent-input safety of rule B8; the Q8 fallbacks of
  LD1 11.5 (the control thread during a Luau Stop); the cost of a Stop.
- LD2, outside the run lifecycle: the runner's verdict and evidence table (the
  V-table rows), the scenario's proof, witness and soak rules, the caps and
  logout steps, outstanding send permits and file root closure, the pair
  baseline, the Table Start exception, the multi-role fleet, and the
  mutations of those parts (`ProofBeforeFreeze`, `CleaningWritesPass`,
  `LogoutRequiresLiveGameProof`, `PartialFleetWritesTerminal`,
  `UnknownOutcomePasses`, `CoreGateAfterPass`, `CancelRetractsSentPacket`,
  `ActivePermitMissingFromFrontier`, `StalePollClockAtV21`,
  `SoakPrintsEarlyPass`, `CleanStopCaptureAfterPass`,
  `QuietTicksCloseUnprocessedRoot`, `UnrelatedCheckClosesRoot`,
  `OneQueryClosesDeferredOutput`, `FenceLatencyFailsRun`,
  `QueuedIntentFreezesPairBaseline`, `MissingTailAllowance`,
  `BootstrapRequiresApprovedCap`, `UnapprovedCapAdmitsSoak` and
  `BarrierClaimsLevelGuard`). The model keeps the ledger, the tickets, the
  fence, the dock and the natural self-stop plan, and the mutations that
  touch them. The grace timeout of a natural self-stop (`SCRIPT_STOP_WAIT`)
  has no clock in the runner.
- Progress beyond P1 and P2, and fairness. A wall-clock bound (TA): L2p.

## When the protocol changes

1. Change the transition in the product and the matching one in
   `src/lifecycle/`; the comments name the function.
2. If an invariant changes, change `src/lifecycle/checks.rs`, this README and
   the commit message, and say why.
3. Run the profiles and the conformance tests. A new finding is a product
   defect, a model defect or a known limit; add a `Mutation` for it.
