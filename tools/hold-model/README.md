# hold-model

A checked model of the auto-run hold. It is a developer tool. No product crate
depends on it, except as a dev-dependency of the conformance tests in
`crates/host-play` and `crates/script-js`.

The model follows the product at commit `178a00200` with the fixes of finding F1
(the host holds a renewed wait again) and finding F2 (the journal click renews
the quiet lease's hold bound). The explorer searches it breadth first inside
caps. A violation prints its shortest trace, step by step, in the product's
terms.

## What the model covers

A script can hold the host's auto-run press while it waits for the page of its
own interaction. The model has one slot with one script and at most one wait
lease. The lease kinds are:

| Kind | Owner | Bound |
|---|---|---|
| `Modal` | native `Dialogue` modal wait (`quester/families/dialogue.rs`) | page and acknowledgement waits on the active clock. End gap on observed ticks. |
| `Quiet` | native quest-journal quiet read (`quest_journal/native.rs`) | a window of observed ticks per phase, renewed at each phase. A 10 s wall bound on the hold: from the begin, and from each window the owner arms (its click, its capture, its close). The page is hidden for at most 10 s from the begin (the paint cap). |
| `Compat` | compatibility (JS) quest-journal read (`script-js/.../quest_journal/isolate.rs`) | a wall bound from the begin, renewed at the click. A separate paint cap. |

Each frame has the product's order (`crates/host/src/lib.rs:739,758`):

1. The drain. A PLAYER_INFO is one game tick.
2. The host's session-boundary release (`host/src/lib.rs:1536-1546`).
3. The observe hook: the session reset, the clock sync
   (`host-play/src/script_observe.rs:608`) and the owner's poll (`:815-827`).
   The poll runs only while the slot is `Running`, on a tick edge or an
   evidence wake.
4. The host samples the lease (`script/src/slot.rs:2259-2294`, also while
   paused) and applies its wait policy (`host/src/lib.rs:225-267`).
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
| `quiet_wall_release`, `quiet_hold_until`, `sample`, Quiet arm | `Ledger::quiet_read`, `quiet_paint_hidden`, `quiet_read_wait` (`script/src/native/ledger.rs:367-416`), `ActionContext::renew_quiet_read` (`script/src/native/actions.rs:52-68`) |
| `sample`, Compat arm, `poll`, Compat arm | `CompatJournalLease::wait`, `renew_hold` (`script-js/.../quest_journal/isolate.rs:146-190`), `JournalRuntime::click` |
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
| S6 | An owner that sends a verb (Talk, Continue, answer, click, close) gets no press in the same frame. The press would close the page that the verb opens (`docs/api/nav.md`: the click never goes out without a live hold). |
| L | Bounded liveness: a hold that is live at tick T has an owner deadline that did not pass at tick T minus the margin. The deadline is counted in the owner's clock (active clock, ticks or wall). |

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
   events and drops the values. One unit is about 2.5 s: a Dialogue page wait is 3
   units, an acknowledgement wait 1, the 10 s wall bounds 4.
   The profile `alt` uses other ratios. A bound that is longer in the product
   is longer in the model, so an ordering race between two bounds stays visible.
2. **Time between the poll and the sample is one unit.** The product reads
   `Instant::now()` after the poll (`script_observe.rs:1141`). The model lets
   a unit pass there (`drift`). A real gap is smaller than a unit. The
   orderings are the same: the sample sees a bound that the poll did not.
3. **One lease at a time.** The ledger has one foreground owner. A runner
   runs one step, and the compat read belongs to a JS run. Two native leases
   at once, and a compat read next to a native one, are not reachable.
4. **The owner is a small machine.** The `Dialogue` phases are collapsed to
   `Page`, `Ack` and `Gap`. A poll does one phase. The product continues in
   the same tick after a begin or a completion (`may_continue_this_tick`).
   The model needs a second frame for that. The owner may also do nothing at
   any poll (`Stall`): a snapshot without chat facts, a spent budget. A stall at
   an expired wait is the case that drove finding F1, now fixed.
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
`f1-renewal-keeps-release` (the host at `0a3e700f8`, before the fix of F1) and
`f2-quiet-click-no-renew` (the ledger at `aa5e8bf2c`, before the fix of F2).

## Findings at the head

| | State |
|---|---|
| F1 | Fixed. The host ended the acknowledgement hold, and the end gap that follows is the same lease renewed, so the press went out during the gap. The host now holds a renewal that moves the deadline later (`f1-renewal-keeps-release` is the old host; S1 and S4 find it in 6 steps). |
| F2 | Fixed. The journal click lost its hold in its own frame when the lease's wall bound passed between the owner's poll and the host's sample (S6). Each window the owner arms, the click first, now renews the hold bound from its poll; the paint cap stays at the begin. `f2-quiet-click-no-renew` is the old ledger; S6 finds it in 6 steps in `quiet` and 4 in `alt`. |
| K1 | A wait that the owner bounds in ticks is not on the active clock. A paused owner loses its end-gap hold. |
| K2 | The terminal release is only as good as the owner's polls. |
| K3 | The compat read never gets a terminal release. |

## Expected results of the profiles

Release build on chardonnay, `--all-violations`, no cap reached
(HOLD-MODEL-NITS, 2026-11-02, `nightly.sh`). Every profile ends clean and
complete at its horizon: no violation of S0 to S6 or L. A run that differs
from this table is a finding. The `quiet` run is the longest: 92.4 M of the
100 M states of the default cap.

| Profile | Horizon | States | Time | Peak RSS | Result |
|---|---|---|---|---|---|
| `smoke` | 8 | 1 410 765 | 0.9 s | 0.13 GB | clean |
| `modal` | 14 | 48 187 697 | 71.4 s | 4.00 GB | clean |
| `quiet` | 14 | 92 396 722 | 138.6 s | 7.78 GB | clean |
| `compat` | 14 | 78 047 719 | 133.7 s | 6.90 GB | clean |
| `all` | 11 | 37 430 466 | 51.5 s | 3.41 GB | clean |
| `alt` | 10 | 17 344 503 | 16.9 s | 1.71 GB | clean |
| `handoff` | 11 | 42 427 339 | 48.0 s | 3.70 GB | clean |

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
- The full profiles do not belong in those lanes: the longest needs 8 GB and
  2.5 minutes, and the seven together about 8 minutes. `tools/nightly` has no
  step for them, and the Linux nightly box (`malbec`) has 15 GiB. They run
  nightly on chardonnay (ADR 0017): `nightly.sh` builds the release binary,
  runs every profile one at a time in its own
  `systemd-run --user --scope -p MemoryMax=16G -p MemorySwapMax=0` scope,
  writes one summary line per profile (exit code, states, time, peak
  memory, result) and exits non-zero when any run does not end clean and
  complete. `systemd/274bot-hold-model-nightly.service` and `.timer` start it
  at 02:30 local time as a user unit; the header of the service says how to
  install it. After a change to the hold or to its model, run
  `tools/hold-model/nightly.sh` by hand and compare the lines with the table.

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
every shortest trace of every release.

Three tests run a counterexample of a retained mutation on the real code. The
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
