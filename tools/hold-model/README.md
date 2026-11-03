# hold-model

A checked model of the auto-run hold. It is a developer tool. No product crate
depends on it, except as a dev-dependency of the conformance tests in
`crates/host-play` and `crates/script-js`.

The model follows the merged native and compatibility wait contracts.
It includes F1 (the host holds a renewed wait again), F2 (each journal window
renews its quiet hold) and N1 (the host drops an expired hold's queued verb).
The explorer searches breadth first inside fixed caps.
A violation prints its shortest trace in the product's terms.

## What the model covers

A script can hold the host's auto-run press while it waits for the page of its
own interaction. The model has one slot with one script and at most one wait
lease. The lease kinds are:

| Kind | Owner | Bound |
|---|---|---|
| `Modal` | native `Dialogue` modal wait (`quester/families/dialogue.rs`) | explicit `AwaitingDispatch` before acceptance, without a content deadline. Open, Continue and end gap use observed ticks. Choice and Main acknowledgements use the active clock. |
| `Quiet` | native quest-journal quiet read (`quest_journal/native.rs`) | a window of observed ticks per phase, renewed at each phase. A 10 s wall bound on the hold: from the begin, and from each window the owner arms (its click, its capture, its close). The page is hidden for at most 10 s from the begin (the paint cap). When the host's sample ends the hold by expiry, the click or close that the owner queued and the host has not dispatched is dropped (N1). |
| `Compat` | compatibility (JS) quest-journal read (`script-js/.../quest_journal/isolate.rs`) | a wall bound from the begin, renewed at the click. A separate paint cap. The same drop (N1), in the isolate's drain. |

Each frame has the product's order (`crates/host/src/lib.rs:739,758`):

1. The drain. A PLAYER_INFO is one game tick.
2. The host's session-boundary release (`host/src/lib.rs:1536-1546`).
3. The observe hook: the session reset, the clock sync
   (`host-play/src/script_observe.rs:608`) and the owner's poll (`:815-827`).
   The poll runs only while the slot is `Running`, on a tick edge or an
   evidence wake.
4. The host samples the lease (`script/src/slot.rs:2259-2294`, also while
   paused) and applies its wait policy (`host/src/lib.rs:225-267`). A sample
   that ends a hold by expiry also drops the verb that the owner queued in
   this frame (N1).
5. The press decision (`host/src/lib.rs:1548-1588`).

The inputs are Pause, Resume, Stop, Start, a disconnect, a connect, a relog,
a modal that opens, closes or changes page, a guardian hold, a prayer-cleanup
hold, a death, a run echo, and the passing of time.

### Where each model function comes from

| Model function (`model.rs`) | Product code |
|---|---|
| `frame` | `host/src/lib.rs` `client_frame_observed` (`:739` observe, `:758` press), `script_observe.rs` |
| `observe_wait`, `deferred` | `host::ScriptRunPolicy::observe_script_wait`, `should_defer_auto_run` (`host/src/lib.rs:240-318`); the re-arm at a renewal is in the same-key arm |
| `sample` | `SlotScript::script_interaction_wait` (`script/src/slot.rs:2259-2294`) |
| `sample`, Modal arm | `WaitDeadline::owner_tick` (`script/src/native.rs:922`), `Ledger::modal_wait_tick` |
| `quiet_wall_release`, `quiet_hold_until`, `sample`, Quiet arm | `Ledger::quiet_read`, `quiet_paint_hidden`, `quiet_read_wait` (`script/src/native/ledger.rs`), `ActionContext::renew_quiet_read` (`script/src/native/actions.rs`) |
| `stale_verb` (Quiet) | `Ledger::quiet_read` cancels the owner's queued interaction when it releases the lease by expiry (`cancel_undispatched_interaction`); the host's fence (`HostAction::live`) then drops it |
| `sample`, Compat arm, `poll`, Compat arm | `CompatJournalLease::wait`, `renew_hold` (`script-js/.../quest_journal/isolate.rs`), `JournalRuntime::click` |
| `stale_verb` (Compat) | `SlotScript::drain_host_interacts` (`drop_expired_hold_rows`, `script/src/slot.rs`): every queued row whose `mirror_owner` is a read's machine row that `CompatJournalLease::expired_owners` names (`script-js/.../quest_journal/isolate.rs`) is dropped, in the drained seat batches and in the seat's custody (`GameplaySeat::drop_expired_hold_rows`) |
| `begin`, `send`, `to_gap`, `rearm` | `Dialogue::sync_modal_wait` (`dialogue.rs:361-380`), `ActionContext::begin_modal_wait`, `renew_modal_wait` |
| `poll`, Quiet arm | `JournalMachine::poll`, `arm_window` (`quest_journal/native.rs`); each window renews the lease's hold bound |
| the press in `frame` | `SlotLoop::apply_auto_run` (`host/src/lib.rs:1548-1588`) |
| the boundary in `frame` | `SlotLoop::after_drain` (`host/src/lib.rs:1536-1546`), `SlotScript::session_boundary` |

## Properties

Each property has a plain statement and an invariant in `model.rs`
(`check_frame`). The invariants are frozen. A mutation changes transitions
only.

| Name | In plain words |
|---|---|
| S0 | The host holds the press only while the sample of this frame returned a wait. |
| S1 | No press goes out while a lease is live and its owner's own deadline, in the owner's own clock, has not passed. |
| S2 | After Stop, a session boundary or a death cancel, no older lease or hold stays (bound: see below). |
| S3 | The host ends a hold without the script only as a logged terminal failure, and not before the owner's deadline plus the margin of one tick. |
| S4 | A lease's identity never carries a released state to a new transaction. |
| S5 | The journal page stops being hidden at the wall cap from its begin. |
| S6 | A verb that the host dispatches (Talk, Continue, answer, click, close) gets no press in the same frame. The press would close the page that the verb opens. |
| S7 | No verb is dispatched without a live hold: a verb that reaches the dispatcher in a frame has the host's hold at the end of that frame. A hold that the frame's sample ended by expiry made the verb stale, and the host dropped it (`docs/api/nav.md`: an expired hold makes its queued interaction stale). S6 sees a dispatch after expiry only when a press follows; S7 also when the run is already on. |
| L | Once the owner has a deadline, a live hold cannot outlast that deadline plus the one-tick margin. The deadline uses the owner's clock. `AwaitingDispatch` has no time bound before acceptance, but Stop, session change and death cancellation still release it. |

### The bound of S2, in frames

- Stop. `SlotScript::stop` revokes the native runtime
  (`script/src/slot.rs:1654`) between frames. The runtime generation is no
  longer reported (`host-play/src/script_observe.rs:1317-1321`), so the host
  clears its hold at once (`host/src/lib.rs:203-209`). The bound is **0 frames
  after the next sample**: the first frame after the Stop holds nothing.
- Relog, disconnect, session change. The host releases in the drain
  (`host/src/lib.rs:1536-1546`). The observe hook calls
  `reset_session_work` before the poll (`host-play/src/play_slots.rs:1551`),
  which revokes the native runtime (`script/src/slot.rs:1757`) and the compat
  lease (`script-js/src/runtime/isolate.rs:1274-1279`). The bound is **0
  frames**: the frame that sees the boundary holds nothing from the old
  session.
- Death. The runner cancels its step when it reads the death line
  (`script/src/quester/runner.rs:3010`). Between zero hitpoints and that line it
  does not poll its step (`:3039`). The bound is **the first owner poll after
  the death line**. If no owner poll comes, the hold ends at the owner's
  deadline plus the margin (property L).

## Abstractions

Each abstraction is on purpose. The reason says why it is safe, or what it
cannot show.

1. **Time is three small counters.** Ticks (`tick`), wall units (`wall`) and the
   owner's active clock (`active`). The active clock moves with wall time
   only while the slot is eligible. Every comparison in the product is `<` or
   `>=` between a bound and a clock, so the model keeps the order of the
   events and drops the values. Wall and active units are about 2.5 s:
   Choice/Main acknowledgements use one unit and the 10 s wall bounds use four.
   Open and Continue use the separate `page_ticks` bound.
   The profile `alt` uses other ratios to test different event orderings.
2. **Time between the poll and the sample is one unit, or a stall.** The
   product reads `Instant::now()` after the poll (`script_observe.rs:1141`).
   The model lets a unit pass there (`drift`). A real gap is usually smaller
   than a unit, and the orderings are the same: the sample sees a bound that
   the poll did not. The gap is not bounded, though: a suspended or sleeping
   machine stalls the process for any time (merlot slept for 8 hours on
   2026-11-01). The `stall` event (`Bounds::stall`, on in every profile)
   moves the wall clock past every hold bound between the poll and the sample.
   It is offered only in a frame where a live lease's owner polls or sends, so
   that the state count grows little. A stall is the case of finding N1.
3. **One lease at a time.** The ledger has one foreground owner. A runner
   runs one step, and the compat read belongs to a JS run. Two native leases
   at once, and a compat read next to a native one, are not reachable. The
   lease's owner is also the only sender of verbs: the model cannot put
   another family's rows in the same batch (a compat read's close and a
   `Modals.close()` of the same script, two equal requests). Every verb in
   the model is the owner's, and S7 checks that verb, so the rule needs no
   second invariant; the product finds the read's rows by their owner
   (`mirror_owner`), never by their request, and `script-js` tests that on
   the production queue (see Conformance).
4. **The owner is a small machine.** Its phases are `AwaitingDispatch`, `Open`,
   `Continue`, `ActiveAck` and `Gap`. `ActiveAck` combines Choice and Main.
   `Begin::Modal` collapses queueing and acceptance into one frame.
   `Begin::QueuedModal` separates them. `Accept` arms the content bound at
   the acceptance tick. Before that, the explicit `AwaitingDispatch` ledger
   state samples the current tick, as `WaitDeadline::owner_tick` does.

   No numeric sentinel represents an undispatched owner.
   The model does not prove that a queued Talk will be accepted.
   It also does not model a receipt observed after a pause with an older
   acceptance tick. The accepted tick-bound pause-tail limitation remains.
   A poll advances one phase. The real owner can continue in the same tick.
   The owner may also do nothing at a poll (`Stall`), as with a spent budget.
5. **A new send, a re-armed gap and a new read are new leases.** A change of
   phase that keeps the lease (acknowledgement to gap, each journal phase) is a
   renewal of the same lease and the same key
   (`dialogue.rs:361-380`). The host cannot tell a renewal from a new send
   that kept its key; it sees the same key and a later deadline. After the
   terminal release, such a sample holds again (`HostHold::deadline`,
   `boundary`). A boundary release, and a repeat of the deadline, do not. Keys
   come from counters. The product's tag bits and the compat key's XOR of
   generation and token (`slot.rs:2285-2287`) would collide only at a token
   above 2^32. The quiet lease's hold bound is renewed at each window the
   owner arms (`renew_quiet_read`), from the wall time of that poll, so the
   verb that the window follows cannot meet a hold that ends in the same frame
   (finding F2). The paint cap stays at the begin. The model keeps the hold
   bound at most one unit past its last wall unit: a later value is never
   reached.
6. **The modal is one flag.** The host's press check and the script see the same
   modal state: both read the drain of the frame. A page change is a counter.
   An acknowledgement is a page that differs from the page at the send.
7. **The press is the policy.** Energy and the run echo are one event
   (`RunOff`). A session boundary clears the run latch.
8. **Death, guardian hold and prayer-cleanup hold are three flags.** Zero
   hitpoints stalls the owner until the death line. A guardian hold stops the
   clock and the poll. A prayer-cleanup hold stops the poll only.
9. **Caps.** Ticks at most 4, wall at most 5 units, ids 4, pages 4, epochs 2,
   gaps 1 to 2 ticks (the product allows 30), one re-arm. Each run is bounded by
   a horizon in actions, a state cap, a frontier cap and a byte cap.
10. **The JS isolate thread is one poll.** The product posts a snapshot and the
    isolate steps on its own thread. The model steps it in the host's poll.

The model cannot show a defect that needs two leases, more than one tick per
frame, a gap larger than the caps, or the real owner's phase logic. The real
`Dialogue` and `JournalMachine` are not in the conformance test: see below.

## Run it

Every run is bounded, and it runs on Linux in a memory scope (chardonnay for
the lab; `nightly.sh` below for all the profiles):

```sh
LAB_HOST=chardonnay /Volumes/dev-scratch/274bot-evidence/OVERNIGHT-1027/aubaine.sh run hold-model \
  'cargo build --release -p hold-model && systemd-run --user --scope -p MemoryMax=16G -p MemorySwapMax=0 -- ionice -c2 -n7 \
   target/release/hold-model --profile modal --horizon 14 --all-violations'
```

Options: `--profile NAME|all-profiles`, `--mutation NAME`, `--horizon N`,
`--state-cap N`, `--frontier-cap N`, `--byte-cap N`, `--all-violations`. The
exit code is 0 for a complete clean run, 1 for a violation and 3 when a cap
stopped the run. The profiles are `smoke`, `modal`, `quiet`, `compat`, `all`,
`alt` and `handoff` (an owner may begin a read in the frame that revokes the
old one). `--mutation` takes `none`, the seven `check-*` self-checks, the
earlier designs `m1-hold-two-ticks` to `m6-compat-no-renew`,
`f1-renewal-keeps-release` (the host at `0a3e700f8`, before the fix of F1),
`f2-quiet-click-no-renew` (the ledger at `aa5e8bf2c`, before the fix of F2) and
`n1-expired-click-dispatched` (the product at `c2ca4cf66`, before the fix of
N1). `--no-stall` runs a profile without the stall, as the profiles were
before the fix of N1 (the state counts of the retained mutations of F2 and M6
use it). The mutations `f2-quiet-click-no-renew`, `m6-compat-no-renew` and
`check-compat-click-no-renew` model products that did not drop a stale verb.

## Findings at the head

| | State |
|---|---|
| F1 | Fixed. The host ended the acknowledgement hold, and the end gap that follows is the same lease renewed, so the press went out during the gap. The host now holds a renewal that moves the deadline later (`f1-renewal-keeps-release` is the old host; S1 and S4 find it in 6 steps). |
| F2 | Fixed. The journal click lost its hold in its own frame when the lease's wall bound passed between the owner's poll and the host's sample (S6). Each window the owner arms, the click first, now renews the hold bound from its poll; the paint cap stays at the begin. `f2-quiet-click-no-renew` is the old ledger; S6 finds it in 6 steps in `quiet` and 4 in `alt` (without the stall, `--no-stall`; with the stall it is shorter). |
| N1 | Fixed. Renewal fixes the old-origin crossing, not every expiry before dispatch: a stall of the process (a suspended or sleeping machine) between the owner's poll and the host's sample can pass the whole renewed bound. The sample ended the hold, and the host still dispatched the click that the poll had queued, with no hold. Now an expired hold makes its queued verb stale: the native ledger cancels the owner's undispatched click or close when it releases the lease by expiry, and the host's drain drops every queued row of the compat read's machine row (found by its `mirror_owner`, not by its request); the owner's next poll finds its read expired and fails it (`journal quiet lease expired`; `modal-timeout`), so its own retry applies. `n1-expired-click-dispatched` is the product at `c2ca4cf66`; S7 (and S6 when a press follows) finds it in 2 to 3 steps. The modal lease has no wall bound and cannot reach it. |
| K1 | Open, Continue and end-gap waits use observed ticks. Pause does not stop those deadlines, so the host can release their holds while the owner is paused. Choice/Main keep their active time. |
| K2 | The terminal release is only as good as the owner's polls. |
| K3 | The compat read never gets a terminal release. |

## Expected results of the profiles

Each profile must end clean and complete at its existing horizon.
The profiles include the process-stall event.
State counts, elapsed time and peak RSS belong to the source-bound run receipt.
A cap-reached result is incomplete and fails `nightly.sh`.

| Profile | Horizon | Required result |
|---|---|---|
| `smoke` | 8 | complete, clean |
| `modal` | 14 | complete, clean |
| `quiet` | 14 | complete, clean |
| `compat` | 14 | complete, clean |
| `all` | 11 | complete, clean |
| `alt` | 10 | complete, clean |
| `handoff` | 11 | complete, clean |

The state cap remains 115 million. The byte cap remains 12 GiB.
Every profile runs inside its own 16 GiB, no-swap scope.
Report an incomplete result before changing a cap or a horizon.

## Where the runs go

- The model's tests and the conformance tests of host-play and script-js run
  in the existing lanes of the default workspace: CI `cargo test --workspace`
  and the Windows nightly's `test-workspace` lane build and run `hold-model`,
  the `host-play` library tests (`script_hold_conformance`) and the
  `script-js` library tests (`hold_conformance_tests`). The host-play
  conformance module is behind the `js` feature (`lib_tests.rs`), so
  `cargo test -p host-play --no-default-features --lib` does **not** run it:
  that native-only lane runs no conformance test. Together they take about
  27 s (host-play, of which the F2 trace takes about 10 s of real time) and
  8 s (script-js) after the build.
- The full profiles run separately from those test lanes. They can consume
  most of their 16 GiB scope. `tools/nightly` has no step for them.
  They run nightly on chardonnay (ADR 0017): `nightly.sh` builds the release binary,
  runs every profile one at a time in its own
  `systemd-run --user --scope -p MemoryMax=16G -p MemorySwapMax=0` scope,
  writes one summary line per profile (exit code, states, time, peak
  memory, result) and exits non-zero when any run does not end clean and
  complete. `systemd/274bot-hold-model-nightly.service` and `.timer` start it
  at 02:30 local time as a user unit; the header of the service says how to
  install it. After a change to the hold or to its model, run
  `tools/hold-model/nightly.sh` by hand and retain each source-bound result.

Tests: `cargo test -p hold-model`.

## Conformance

Three product tests drive the real code with traces from the model. Each
trace is a shortest trace that ends a lease of one kind for one cause (its
page, its own bound, a drop, Stop, a session change, a death, the host's
terminal release or the lease's wall bound).

- `crates/host-play/src/script_hold_conformance_tests.rs` runs the native
  kinds on the production frame path: `Host::run_client`,
  `script_observe_cached_with_channels`, the real `host::ScriptRunPolicy`,
  the ledger's lease sampling, the owner's revocation, the active clock and
  `apply_auto_run`. After each frame it compares the host's hold and press
  with the model. The owner is a test double that does what the model says
  through the real lease API (`begin_modal_wait`, `renew_modal_wait`,
  `begin_quiet_read`, `renew_quiet_read`). The real `Dialogue` and
  `JournalMachine` phase logic is not driven. One model unit is 150 ms of
  real time; a trace that brings a quiet lease to its 10 s wall bound runs at
  2.5 s per unit.
- The same file runs the compat traces on the real host. A stub
  compatibility surface reports, at each frame, the lease output that the
  model chose (its key, from the token and the work generation, and its
  wait). The real slot translation (`script_interaction_wait`), the real host
  policy with its session-boundary release and the real press decision must
  decide like the model's host at every frame. No trace needs real time.
- `crates/script-js/src/compat/quest_journal/hold_conformance_tests.rs` runs the
  compat read: the real journal runtime and the real `CompatJournalLease`,
  with `backdate` for wall time. It compares the lease's hold and paint-hide
  state with the model. It cannot reach the host policy: the compat lease
  reports a deadline equal to the current tick (K3), so the host never ends it.

`HOLD_CONFORMANCE_TRACES=N` sets the number of traces per release (the
default is 3 for the native kinds of host-play, 40 for the compat traces of
host-play and 6 for script-js). A native trace that brings a quiet lease to
its 10 s wall bound runs in real seconds (the bound is the product's
constant). The default run drives at most two such traces and prints how many
it skipped; `HOLD_CONFORMANCE_SLOW_TRACES=N` raises the limit. A full run,
`HOLD_CONFORMANCE_TRACES=100000 HOLD_CONFORMANCE_SLOW_TRACES=100000`, drives
every shortest trace of every release: 519 native traces (none skipped) and 805
compat traces, in about 6 minutes (HOLD-MODEL-NITS, chardonnay).

The directed native conformance test
`the_real_modal_lease_matches_each_owner_clock_and_queued_acceptance`
checks all three contracts through the real lease API.
It checks paused Open/Continue tick expiry, paused Choice/Main active time,
and a queued Talk that holds until acceptance arms its tick deadline.
The owner remains a test double. These checks do not replace real Dialogue
or journal phase tests.

Four tests run a counterexample of a retained mutation on the real code. The
real code must decide like the head model at every frame, and each test
fails on code that has the old behaviour:

- `the_renewed_end_gap_is_held_on_the_real_code_where_the_earlier_host_pressed`
  (finding F1, mutation `f1-renewal-keeps-release`, the host at `0a3e700f8`).
- `a_compat_read_renewed_at_its_click_after_the_terminal_release_is_held_again`
  is the same rule on a compat stream, with the real slot translation. A
  generated compat trace cannot reach it: the real compat lease never reports a
  read past its bound (K3), so no generated trace has a terminal release to
  renew. The directed test feeds a read that the surface reports past its bound.
- `the_journal_click_is_held_on_the_real_code_where_the_earlier_ledger_pressed`
  (finding F2, mutation `f2-quiet-click-no-renew`, the ledger at `aa5e8bf2c`).
  It runs in real seconds.
- `an_expired_hold_makes_the_queued_click_stale_on_the_real_code`
  (finding N1, mutation `n1-expired-click-dispatched`, the ledger at
  `c2ca4cf66`). The owner double queues a real click through
  `ActionContext::emit` and the process stalls (the double sleeps 10.5 s) past
  the renewed hold before the host's sample. The real code must decide like
  the head model at every frame, the click's receipt must never come back (the
  host did not dispatch it), and the owner's next poll must find its read
  expired. It runs two model traces (one where a press follows, one where the
  run is already on and only S7 sees the dispatch), about 11 s each.

The compat side of N1 is tested in `script-js` on the production queue: a
slot whose real `JsSurfaceFactory` spawns the isolate with the slot's
execution identity, so the click and close travel in seat batches to the
host's one drain (`SlotScript::drain_host_interacts`).
`production_queue_drops_a_journal_click_whose_hold_expired_before_the_drain`
and `..._whose_read_failed_before_the_drain` (the read's `modal-timeout` step
comes first) drop the click; `production_queue_drops_only_the_journal_close_of_an_expired_hold`
drops the read's close and keeps an equal close of `Modals.close()` from the
same script (identity, not equality). The lease-level
`an_expired_hold_names_the_owner_of_its_queued_verb` and the seat-level
`an_expired_hold_drops_only_its_owners_rows_in_custody` cover the parts. The
native side has three tests in `quest_journal/native_tests.rs` (click, close,
and a dispatched click that keeps its receipt). None of them can reach the
host policy.

The generated compat traces fail on a host that lets a new key inherit the
phase of the old hold, on one that keeps its hold at a session boundary, and
on one with no margin. The direct host tests (`crates/host/src/lib_tests.rs`)
cover the other wrong versions of the F1 fix: a re-arm at any change of the
deadline, and a re-arm after a boundary.

## When the hold changes

1. Change the transition that matches the product code in `model.rs`. The
   comments name the product function.
2. If a property changed, change the invariant and say why in the commit.
3. Run the profiles. A new finding is a product defect, a model defect or a
   known limit. Write each in the findings test.
4. Run the conformance test. It drives the product code with traces from the
   model and compares the press and hold decisions at every frame.
