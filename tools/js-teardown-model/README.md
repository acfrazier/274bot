# js-teardown-model

A checked model of the JS isolate's teardown protocol (ADR 0017). It is a
developer tool. No product crate depends on it, except as a dev-dependency of
the conformance tests in `crates/script-js`.

Two models share one explorer (`explorer.rs`, a copy of the generic breadth
first search in `tools/hold-model`; the Luau lifecycle model copies it the
same way):

| Model | Product code | What it checks |
|---|---|---|
| `teardown` | `crates/script-js/src/runtime/isolate/teardown.rs` (`TeardownState`, `Teardown`, and the functions that take the mutex) | When an eval may enter V8, who may terminate it, when `onStop` runs, what a pending termination request can reach. |
| `retire` | `crates/script-js/src/runtime/isolate/supervisor.rs` (`Registry`, `Pass`, `settle`, `Registry::reap`) | When the supervisor may release a capacity credit. |

## The four invariants

They are the four invariants of the isolate teardown design, each as the checks
named below. A check is a function of the state
after every step.

1. **The eval that armed a terminate request consumes it, or the isolate
   thread cancels it, and no later eval or `onStop` sees it.**
   `a-terminate-outlives-its-eval` (V8 holds a request armed for an eval that
   is no longer active), `a-later-eval-sees-terminate` (an eval enters or runs
   with a request armed for another eval or for `onStop`),
   `a-on-stop-sees-terminate` (the hook starts or runs with a request armed
   for an eval or for Stop), `a-terminate-after-the-hook` (a request is
   pending once the isolate thread has left the hook, apart from Drop's),
   `a-cut-reported-without-delivery` (a cut reported as consumed that V8 never
   delivered).
2. **`onStop` runs at most once and only after no eval is active.**
   `b-on-stop-twice`, `b-on-stop-with-active-eval`. A third check keeps the
   body bounded: `e-hook-unbounded` (the body runs and no hook deadline can
   still terminate it).
3. **The supervisor releases a capacity credit only after the OS thread
   joins.** `c-credit-before-join` (a gameplay credit is released with the
   thread unjoined and the row not in a published quarantine),
   `c-validation-credit-before-join`, `c-join-of-live-thread`,
   `c-live-thread-released` (a row leaves custody with its thread alive),
   `c-census-loses-row`, `c-quarantine-over-cap`, `c-receipt-twice`.
4. **Pause never blocks an eval that already owns execution and always blocks
   the next entry.** `d-cut-before-horizon` (a cut other than the refused
   deadline's before the eval's own horizon), `d-pause-halts-owning-eval`
   (`machines_halted` on Pause intent alone), `d-pause-did-not-block-entry` (a
   tick or recovery eval enters while Pause intent is published). A frame or a
   lifecycle eval does not honour Pause, by design.

## What the teardown model covers

Three actors: the isolate thread, the host (the dispatch watchdog, Pause,
Resume, session reset, Stop, raw Drop; one sequential actor) and the deadline
supervisor. Every action is one critical section of the teardown mutex, or one
step the product takes outside it (a registration with the supervisor, a V8
cancel after the lock, Drop's terminate). The V8 termination request is a flag
with a provenance: the set of requests that armed it.

| Model action | Product function |
|---|---|
| `Enter(kind)` | `begin_interruptible_execution`, `begin_frame_execution`, `begin_lifecycle_hook_execution` |
| `EnterBusy(kind)` | the same three functions, called while an eval is active: the guard in `TeardownState::begin_execution` must refuse. The product's thread never makes this call; the action keeps the guard under conformance |
| `TryHook` | `enter_teardown_hook` while an eval is active: `TeardownState::enter_hook` must refuse (`HookRefusal::ExecutionActive`). Same reason as `EnterBusy` |
| `Poll`, `Pause` (first step), `Look` | `watchdog_look`, then `begin_arm` or `fire_execution_interrupt` (`interrupt_slow_execution_shared`, `isolate.rs`) |
| `PauseSet`, `PauseArm`, `Resume`, `Reset` | `TeardownState::request_pause`, `begin_arm(Pause)`, `clear_pause`, `begin_arm(SessionReset)` |
| `ArmWatchdog` | `begin_arm(Watchdog)` (`arm_execution_watchdog`, `thread.rs`, lifecycle hooks) |
| `Register`, `FireRefused` | the supervisor's `arm_execution`; `finish_arm` on a refusal |
| `Deliver` | V8 delivers the pending request; `note_interrupt_consumed` is the V8-entry fence |
| `Finish` | `finish_interruptible_execution` (the cancel is under the lock) |
| `StopSend`, `StopLock` | `LoadIsolate::retire`: the Stop command, then `begin_stop` |
| `Drop`, `DropSend`, `DropFire` | `LoadIsolate::drop`: `revoke`, the Stop command, the terminate |
| `ScriptStop`, `StopCommand`, `EnterHook`, `HookGuard`, `HookCancel`, `HookRegister`, `HookDone`, `Complete`, `CancelAfter` | `teardown_once`: `enter_teardown_hook`, `hook_deadline`, the V8 cancel, the supervisor's `arm_hook`, the body, `finish_teardown_hook`, the V8 cancel |
| `Fire(i)` | the supervisor's `run_deadline` |

### Abstractions

Each is on purpose.

1. **Time is one counter.** Every comparison in the product is `<=` or `>=`
   between a start and a bound, so the model keeps the order of the events and
   drops the values. One unit is one watchdog horizon (600 ms); the hook budget
   is one unit (the conformance tests set `test_hook_timeout` to match).
2. **The eval's code is one flag.** An eval runs until `Finish`; `Deliver` is
   V8 handing the pending request to the code that runs. A request that is
   pending and not delivered is cancelled at the finish; a cut is reported only
   when delivered. The model has no JS.
3. **One isolate, one eval at a time, at most three evals per run.** The
   isolate thread runs ticks, recovery evals, frame listeners and lifecycle
   callbacks one after the other. Profiles `ticks` and `stop` use two evals,
   `clock` and `full` three.
4. **The host is one sequential actor.** The dispatch watchdog, Pause, Resume,
   reset, Stop and Drop all run on the slot thread. Two of its operations are
   never interleaved; each can interleave with the isolate thread and the
   supervisor between its lock sections.
5. **The supervisor registers deadlines in a list.** `arm_execution` and
   `arm_hook` push. Capacity refusal is an action (`Register { refuse: true }`);
   the registry's own cap on deadlines is not modelled.
6. **Setup is not modelled.** The startup eval before the first tick
   (`SetupMessage::Interrupt`, `fire_before_hook`) shares `terminate_before_hook`
   but has no eval identity. A failed setup is Stop's case.
7. **Only safety.** The model proves nothing about progress: an eval past its
   horizon is cut only if the watchdog or a deadline acts. A Pause deadline
   that finds Pause resumed cuts nothing, and the next watchdog look cuts the
   eval if it is past its horizon (the product's design, `teardown.rs`).

### Mutations

Each earlier design a comment rejects, and each defect the model found, is a
`Mutation` whose invariant must fire (`Mutation::caught_by`, tested in
`tests/model.rs`).

| Mutation | What it is | Caught by |
|---|---|---|
| `r1-stop-terminates-in-hook` | Stop terminates whatever phase it finds ("join cannot terminate after the hook starts") | `a-on-stop-sees-terminate` |
| `r2-pause-checked-outside-lock` | the Pause check of an entry is outside the lock ("Pause sees no active eval immediately before the isolate thread enters a queued one") | `d-pause-did-not-block-entry` |
| `r3-finish-cancels-first` | finish cancels before it clears the eval ("cannot fire after this cancel and leave termination armed") | `a-terminate-outlives-its-eval` |
| `r4-hook-deadline-into-done` | the hook deadline fires in any phase ("the hook's 50 ms one-shot cannot fire into Done") | `a-terminate-after-the-hook` |
| `r5-deadline-without-identity` | an execution deadline cuts whatever eval is active ("deadlines capture the identity") | `d-cut-before-horizon` |
| `r6-hook-enter-twice` | teardown is not exactly-once | `b-on-stop-twice` |
| `r7-stop-enters-hook` | Stop enters the hook from the host while an eval is active | `b-on-stop-with-active-eval` |
| `d1-drop-overwrites-hook` | **found**: `Drop` sets Done even when the hook is running | `e-hook-unbounded` |
| `check-finish-no-cancel`, `check-pause-cuts-active`, `check-halted-on-pause`, `check-fence-marks-always` | builder's checks that an invariant can fail | `a-terminate-outlives-its-eval`, `d-cut-before-horizon`, `d-pause-halts-owning-eval`, `a-cut-reported-without-delivery` |

The retirement model's mutations: `receipts-before-publish`
(`c-credit-before-join`), `validation-credit-at-quarantine`
(`c-validation-credit-before-join`), `join-without-finished` and
`reap-joins-live` (`c-join-of-live-thread`), `processing-uncounted`
(`c-census-loses-row`), `quarantine-ignores-room` (`c-quarantine-over-cap`),
`timeout-releases-live-thread` (`c-live-thread-released`),
`reap-sends-second-receipt` (`c-receipt-twice`).

## Finding D1

At `a3a50d098` `LoadIsolate::drop` set the phase to `Done` from any phase but
`Done`, including `Hook`. The shortest trace, 8 steps (`teardown` profile
`stop`, mutation `d1-drop-overwrites-hook`):

1. the script requests stop; 2. the isolate thread enters the hook; 3.
`arm_hook_deadline` reads the phase (`Hook`); 4. Drop sets `Done`; 5. Drop
queues Stop; 6. Drop terminates V8; 7. the isolate thread cancels the pending
termination (it eats Drop's request); 8. it registers the hook deadline.

The body then runs with phase `Done`. The supervisor fires the hook deadline
only in `Hook` and Drop's request is gone, so no one terminates a body that
loops. LP1 fixes it: `TeardownState::revoke` leaves `Hook` alone. The
conformance test `raw_drop_in_the_hook_window_leaves_the_hook_deadline_armed`
runs this path on the real functions and fails on the earlier `Drop`.

## Run it

Every run is bounded by a horizon, a state cap, a frontier cap and a byte cap.
Every profile is exhausted before its horizon (`deepest < horizon`, asserted
in `tests/model.rs`): the horizon does not bound what the profile covers.

```sh
cargo run --release -p js-teardown-model -- --model teardown --profile all-profiles
cargo run --release -p js-teardown-model -- --model retire --profile all-profiles
cargo run --release -p js-teardown-model -- --model teardown --profile stop --mutation d1-drop-overwrites-hook
```

Options: `--model teardown|retire`, `--profile NAME|all-profiles`, `--mutation
NAME`, `--horizon N`, `--state-cap N`, `--frontier-cap N`, `--byte-cap N`,
`--all-violations`. The exit code is 0 for a complete clean run, 1 for a
violation, 3 when a cap stopped the run and 2 for a bad argument.

| Model | Profile | What it adds | Horizon | States |
|---|---|---|---|---|
| teardown | `smoke` | one eval | 24 | 18 354 |
| teardown | `ticks` | two ticks, Pause, Resume, reset, Stop, Drop, script stop, refusals | 32 | 245 530 |
| teardown | `frames` | `ticks` and frame and lifecycle entries | 32 | 356 022 |
| teardown | `no-refusal` | the supervisor never refuses | 32 | 189 986 |
| teardown | `stop` | the hook protocol alone, a longer clock | 30 | 41 089 |
| teardown | `clock` | three evals, five time units | 40 | 1 300 074 |
| teardown | `full` | `frames`, three evals, four time units | 48 | 4 697 028 |
| retire | `retire-smoke` | two rows | 26 | 2 859 |
| retire | `retire` | three rows (one validation), room one | 32 | 62 921 |
| retire | `retire-cap2` | four rows, room two | 36 | 1 626 026 |

Release build, one core: the largest profile (`full`) takes about 7 s and an
estimated 640 MB. The unit test lane runs every profile but `clock` and
`full`; those two run in the nightly (`nightly.sh`, a 16 GB scope per
profile) and with `JS_TEARDOWN_MODEL_FULL=1 cargo test -p js-teardown-model`.
`systemd/` holds the nightly's user unit and timer for chardonnay (the install
steps are in the unit's header); nothing installs them.

Tests: `cargo test -p js-teardown-model`.

## Conformance

`crates/script-js/src/runtime/isolate/teardown_conformance_tests.rs` and
`supervisor_conformance_tests.rs` drive the real functions through the traces
of the head model and compare the real state with the model's after every step
(see the headers of both files for the fields). They use the shortest trace of
each protocol rule (`TEARDOWN_GOALS`), deterministic random walks of the `full`
and `retire` profiles, and directed paths. `TEARDOWN_CONFORMANCE_TRACES`,
`TEARDOWN_CONFORMANCE_WALKS` and `RETIRE_CONFORMANCE_WALKS` set the counts.

Each file also holds seeded divergences: a behaviour of the code that the
driver puts back (the earlier `Drop`, a finish that leaves V8's request, Stop
terminating in the hook; receipts before the publication, a quarantine without
room, an uncounted pass). The tests require each to fail, so a conformance
test that cannot see a change fails.

The drivers call the functions of `teardown.rs` and `supervisor.rs` in the
order of the model's actions. They do not run the glue that owns a `Runtime`
(`teardown_once`, `arm_hook_deadline`, `call_interruptible`) or the
supervisor's `run` loop; `tests/on_stop.rs`, `tests/load_isolate.rs` and the
live cells cover those.

## When the protocol changes

1. Change the transition in the product and the matching one in `src/teardown.rs`
   or `src/retire.rs`; the comments name the function.
2. If an invariant changes, change it here and say why in the commit.
3. Run the profiles and the conformance tests. A new finding is a product
   defect, a model defect or a known limit; add a `Mutation` for it.
